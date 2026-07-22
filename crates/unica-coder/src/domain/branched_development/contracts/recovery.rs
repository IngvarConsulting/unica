use super::artifacts::OwnedTargetLocator;
use super::instructions::{
    CleanManualWorkingInfobaseInstruction, CloseReservedOriginalDesignerInstruction,
    ReleaseRepositoryLocksInstruction, ReservedOriginalSupportCorrectiveInstructionSchema,
    SeparateWorkingInfobaseSupportCorrectiveInstructionSchema, SupportConflictInstruction,
    SupportCorrectiveInstruction, SupportEvidenceInstruction, SupportRecoveryExternalAction,
    SupportRecoveryExternalActionRef,
};
#[cfg(test)]
use super::prearm_recovery::PreArmCancellationFinalizationRecheckEvidence;
use super::prearm_recovery::{
    PreArmCancellationEffectKind, PreArmCancellationEffectObservation,
    PreArmCancellationEffectReceipt, PreArmCancellationFinalizationAttemptProgress,
    PreArmCancellationFinalizationExecutionPathKind, PreArmCancellationFinalizationPlan,
    PreArmCancellationKnownBlocker, PreArmCancellationReceiptRef, PreArmCancellationReceiptSource,
};
use super::repository::{
    NonConflictingConcurrentEvidence, RepositoryHistoryCursor, ValidatedRepositoryHistoryPartition,
};
use super::schema::one_of_schema;
use super::support::{
    ManualSupportTargetMode, ManualWorkingInfobaseIdentity, ReservedOriginalLeaseStopEvidence,
    SupportPrerequisiteVersionObservation, SupportRecoveryDisposition,
};
use super::support_recovery_authority::SupportRecoveryAuthorityToken;
use super::support_terminalization::{
    BlockedSupportRecoveryTargetRef, ManualWorkingInfobaseClosurePlan,
    ManualWorkingInfobaseStopEvidence, ReservedBlockedAfterPartialGuardProofSchema,
    ReservedBlockedBeforeRootGuardProofSchema, ReservedStoppedAfterCompleteGuardProofSchema,
    SeparateBlockedAfterPartialGuardProofSchema, SeparateBlockedBeforeRootGuardProofSchema,
    SeparateStoppedAfterCompleteGuardProofSchema, SupportRecoveryFinalizationPlan,
    SupportRecoveryGuardProof,
};
use crate::domain::branched_development::canonical_json::{
    canonical_contract_digest, contract_digest_record_sealed, ContractDigestRecord,
};
use crate::domain::branched_development::{
    CapabilityRowId, MetadataObjectId, OperationId, Sha256Digest, TaskPhase, UnicaId,
};
use schemars::{json_schema, JsonSchema, Schema, SchemaGenerator};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::borrow::Cow;
use std::fmt;

const MAX_RECOVERY_ITEMS: usize = 1_024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RecoveryContractError(&'static str);

impl fmt::Display for RecoveryContractError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.0)
    }
}

impl std::error::Error for RecoveryContractError {}

fn contract_digest<T: ContractDigestRecord>(
    value: &T,
    context: &'static str,
) -> Result<Sha256Digest, RecoveryContractError> {
    canonical_contract_digest(value, None).map_err(|_| RecoveryContractError(context))
}

macro_rules! wire_literal {
    ($name:ident, $wire:literal) => {
        #[derive(
            Debug,
            Clone,
            Copy,
            PartialEq,
            Eq,
            PartialOrd,
            Ord,
            Hash,
            Serialize,
            Deserialize,
            JsonSchema,
        )]
        enum $name {
            #[serde(rename = $wire)]
            Value,
        }
    };
}

wire_literal!(RegisteredSubjectKind, "registered");
wire_literal!(MetadataObjectSubjectKind, "metadataObject");
wire_literal!(ConfigurationRootSubjectKind, "configurationRoot");
wire_literal!(OwnedRoleSubjectKind, "ownedRole");
wire_literal!(
    ExternalWorkingInfobaseSubjectKind,
    "externalWorkingInfobase"
);
wire_literal!(
    ReservedOriginalInfobaseSubjectKind,
    "reservedOriginalInfobase"
);
wire_literal!(RetentionLeaseSubjectKind, "retentionLease");
wire_literal!(MatchesOutcome, "matches");
wire_literal!(DiffersOutcome, "differs");
wire_literal!(UnknownOutcome, "unknown");
wire_literal!(QuarantinedOwnedTargetState, "quarantined");
wire_literal!(AbsentOwnedTargetState, "absent");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct NullLiteral;

impl Serialize for NullLiteral {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_none()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TrueLiteral;

impl Serialize for TrueLiteral {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_bool(true)
    }
}

impl JsonSchema for TrueLiteral {
    fn inline_schema() -> bool {
        true
    }

    fn schema_name() -> Cow<'static, str> {
        "RecoveryTrueLiteral".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({ "type": "boolean", "const": true })
    }
}

impl JsonSchema for NullLiteral {
    fn inline_schema() -> bool {
        true
    }

    fn schema_name() -> Cow<'static, str> {
        "RecoveryNullLiteral".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({ "type": "null" })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RegisteredRecoverySubject {
    subject_kind: RegisteredSubjectKind,
    subject_id: UnicaId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MetadataObjectRecoverySubject {
    subject_kind: MetadataObjectSubjectKind,
    object_id: MetadataObjectId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ConfigurationRootRecoverySubject {
    subject_kind: ConfigurationRootSubjectKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct OwnedRoleRecoverySubject {
    subject_kind: OwnedRoleSubjectKind,
    locator: OwnedTargetLocator,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ExternalWorkingInfobaseRecoverySubject {
    subject_kind: ExternalWorkingInfobaseSubjectKind,
    identity: ManualWorkingInfobaseIdentity,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReservedOriginalInfobaseRecoverySubject {
    subject_kind: ReservedOriginalInfobaseSubjectKind,
    original_identity_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RetentionLeaseRecoverySubject {
    subject_kind: RetentionLeaseSubjectKind,
    retention_lease_id: UnicaId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
enum RecoverySubjectRefKind {
    Registered(RegisteredRecoverySubject),
    MetadataObject(MetadataObjectRecoverySubject),
    ConfigurationRoot(ConfigurationRootRecoverySubject),
    OwnedRole(OwnedRoleRecoverySubject),
    ExternalWorkingInfobase(ExternalWorkingInfobaseRecoverySubject),
    ReservedOriginalInfobase(ReservedOriginalInfobaseRecoverySubject),
    RetentionLease(RetentionLeaseRecoverySubject),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub(crate) struct RecoverySubjectRef(RecoverySubjectRefKind);

impl RecoverySubjectRef {
    pub(crate) fn registered(subject_id: UnicaId) -> Self {
        Self(RecoverySubjectRefKind::Registered(
            RegisteredRecoverySubject {
                subject_kind: RegisteredSubjectKind::Value,
                subject_id,
            },
        ))
    }

    pub(crate) fn metadata_object(object_id: MetadataObjectId) -> Self {
        Self(RecoverySubjectRefKind::MetadataObject(
            MetadataObjectRecoverySubject {
                subject_kind: MetadataObjectSubjectKind::Value,
                object_id,
            },
        ))
    }

    pub(crate) fn configuration_root() -> Self {
        Self(RecoverySubjectRefKind::ConfigurationRoot(
            ConfigurationRootRecoverySubject {
                subject_kind: ConfigurationRootSubjectKind::Value,
            },
        ))
    }

    pub(crate) fn owned_role(locator: OwnedTargetLocator) -> Self {
        Self(RecoverySubjectRefKind::OwnedRole(
            OwnedRoleRecoverySubject {
                subject_kind: OwnedRoleSubjectKind::Value,
                locator,
            },
        ))
    }

    pub(crate) fn external_working_infobase(identity: ManualWorkingInfobaseIdentity) -> Self {
        Self(RecoverySubjectRefKind::ExternalWorkingInfobase(
            ExternalWorkingInfobaseRecoverySubject {
                subject_kind: ExternalWorkingInfobaseSubjectKind::Value,
                identity,
            },
        ))
    }

    pub(crate) fn reserved_original_infobase(original_identity_digest: Sha256Digest) -> Self {
        Self(RecoverySubjectRefKind::ReservedOriginalInfobase(
            ReservedOriginalInfobaseRecoverySubject {
                subject_kind: ReservedOriginalInfobaseSubjectKind::Value,
                original_identity_digest,
            },
        ))
    }

    pub(crate) fn retention_lease(retention_lease_id: UnicaId) -> Self {
        Self(RecoverySubjectRefKind::RetentionLease(
            RetentionLeaseRecoverySubject {
                subject_kind: RetentionLeaseSubjectKind::Value,
                retention_lease_id,
            },
        ))
    }

    fn canonical_key(&self) -> Result<RecoverySubjectOrderKey, RecoveryContractError> {
        let canonical_json = || {
            serde_json_canonicalizer::to_vec(self)
                .map_err(|_| RecoveryContractError("recovery subject canonicalization failed"))
        };
        match &self.0 {
            RecoverySubjectRefKind::ExternalWorkingInfobase(_) => Ok(
                RecoverySubjectOrderKey::ExternalWorkingInfobase(canonical_json()?),
            ),
            RecoverySubjectRefKind::OwnedRole(value) => {
                Ok(RecoverySubjectOrderKey::OwnedRole(value.locator.clone()))
            }
            RecoverySubjectRefKind::MetadataObject(_) => {
                Ok(RecoverySubjectOrderKey::MetadataObject(canonical_json()?))
            }
            RecoverySubjectRefKind::ReservedOriginalInfobase(_) => Ok(
                RecoverySubjectOrderKey::ReservedOriginalInfobase(canonical_json()?),
            ),
            RecoverySubjectRefKind::RetentionLease(_) => {
                Ok(RecoverySubjectOrderKey::RetentionLease(canonical_json()?))
            }
            RecoverySubjectRefKind::Registered(_) => {
                Ok(RecoverySubjectOrderKey::Registered(canonical_json()?))
            }
            RecoverySubjectRefKind::ConfigurationRoot(_) => {
                Ok(RecoverySubjectOrderKey::ConfigurationRoot(canonical_json()?))
            }
        }
    }

    fn is_registered(&self, expected: &UnicaId) -> bool {
        matches!(
            &self.0,
            RecoverySubjectRefKind::Registered(value) if &value.subject_id == expected
        )
    }

    fn is_configuration_root(&self) -> bool {
        matches!(self.0, RecoverySubjectRefKind::ConfigurationRoot(_))
    }

    fn is_owned_role(&self, expected: &OwnedTargetLocator) -> bool {
        matches!(
            &self.0,
            RecoverySubjectRefKind::OwnedRole(value) if &value.locator == expected
        )
    }

    fn is_external_working_infobase(&self, expected: &ManualWorkingInfobaseIdentity) -> bool {
        matches!(
            &self.0,
            RecoverySubjectRefKind::ExternalWorkingInfobase(value) if &value.identity == expected
        )
    }

    fn is_reserved_original(&self, expected: Option<&Sha256Digest>) -> bool {
        matches!(
            &self.0,
            RecoverySubjectRefKind::ReservedOriginalInfobase(value)
                if expected.is_none_or(|expected| &value.original_identity_digest == expected)
        )
    }

    fn is_retention_lease(&self, expected: &UnicaId) -> bool {
        matches!(
            &self.0,
            RecoverySubjectRefKind::RetentionLease(value) if &value.retention_lease_id == expected
        )
    }
}

/// Stable subject ordering used by every recovery collection. Variant order
/// preserves the previous canonical-JSON cross-kind order (`identity`,
/// `locator`, `objectId`, `originalIdentityDigest`, `retentionLeaseId`,
/// `subjectId`, `subjectKind`). Owned-role subjects alone use the normative
/// typed locator order instead of JSON member/spelling order.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
enum RecoverySubjectOrderKey {
    ExternalWorkingInfobase(Vec<u8>),
    OwnedRole(OwnedTargetLocator),
    MetadataObject(Vec<u8>),
    ReservedOriginalInfobase(Vec<u8>),
    RetentionLease(Vec<u8>),
    Registered(Vec<u8>),
    ConfigurationRoot(Vec<u8>),
}

impl JsonSchema for RecoverySubjectRef {
    fn schema_name() -> Cow<'static, str> {
        "RecoverySubjectRef".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<RegisteredRecoverySubject>(),
            generator.subschema_for::<MetadataObjectRecoverySubject>(),
            generator.subschema_for::<ConfigurationRootRecoverySubject>(),
            generator.subschema_for::<OwnedRoleRecoverySubject>(),
            generator.subschema_for::<ExternalWorkingInfobaseRecoverySubject>(),
            generator.subschema_for::<ReservedOriginalInfobaseRecoverySubject>(),
            generator.subschema_for::<RetentionLeaseRecoverySubject>(),
        ])
    }
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub(crate) enum RecoveryObservationKind {
    RepositoryAnchor,
    RepositoryVersion,
    SupportGraph,
    SupportActionAuthorization,
    ObjectFingerprint,
    TaskFingerprint,
    LockOwnership,
    WorkingInfobaseLease,
    ReservedOriginalLease,
    RetentionLease,
    FinalizationPolicy,
    ArtifactPresence,
    ArchiveStagingPresence,
    ArchivePresence,
    QuarantinePresence,
    OwnedTargetAbsence,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub(crate) enum RecoveryUnknownReason {
    ObservationUnavailable,
    CapabilityUnproven,
    EffectOutcomeUnavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RecoveryExpectedObservation {
    observation_kind: RecoveryObservationKind,
    subject: RecoverySubjectRef,
    expected_digest: Sha256Digest,
}

impl RecoveryExpectedObservation {
    pub(crate) fn new(
        observation_kind: RecoveryObservationKind,
        subject: RecoverySubjectRef,
        expected_digest: Sha256Digest,
    ) -> Self {
        Self {
            observation_kind,
            subject,
            expected_digest,
        }
    }

    fn canonical_key(
        &self,
    ) -> Result<(RecoveryObservationKind, RecoverySubjectOrderKey), RecoveryContractError> {
        Ok((self.observation_kind, self.subject.canonical_key()?))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct QuarantinedOwnedTargetDigestRecord {
    state: QuarantinedOwnedTargetState,
    owned_target: OwnedTargetLocator,
    quarantine_id: UnicaId,
}

impl contract_digest_record_sealed::Sealed for QuarantinedOwnedTargetDigestRecord {}
impl ContractDigestRecord for QuarantinedOwnedTargetDigestRecord {}

fn expected_quarantined_owned_target_digest(
    owned_target: &OwnedTargetLocator,
    quarantine_id: &UnicaId,
) -> Result<Sha256Digest, RecoveryContractError> {
    contract_digest(
        &QuarantinedOwnedTargetDigestRecord {
            state: QuarantinedOwnedTargetState::Value,
            owned_target: owned_target.clone(),
            quarantine_id: quarantine_id.clone(),
        },
        "quarantined owned-target digest failed",
    )
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AbsentOwnedTargetDigestRecord {
    state: AbsentOwnedTargetState,
    archive_id: UnicaId,
    finish_action_id: UnicaId,
    owned_target: OwnedTargetLocator,
}

impl contract_digest_record_sealed::Sealed for AbsentOwnedTargetDigestRecord {}
impl ContractDigestRecord for AbsentOwnedTargetDigestRecord {}

fn expected_absent_owned_target_digest(
    archive_id: &UnicaId,
    finish_action_id: &UnicaId,
    owned_target: &OwnedTargetLocator,
) -> Result<Sha256Digest, RecoveryContractError> {
    contract_digest(
        &AbsentOwnedTargetDigestRecord {
            state: AbsentOwnedTargetState::Value,
            archive_id: archive_id.clone(),
            finish_action_id: finish_action_id.clone(),
            owned_target: owned_target.clone(),
        },
        "absent owned-target digest failed",
    )
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MatchedRecoveryObservationDigestRecord {
    outcome: MatchesOutcome,
    observation_kind: RecoveryObservationKind,
    subject: RecoverySubjectRef,
    expected_digest: Sha256Digest,
    observed_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DifferedRecoveryObservationDigestRecord {
    outcome: DiffersOutcome,
    observation_kind: RecoveryObservationKind,
    subject: RecoverySubjectRef,
    expected_digest: Sha256Digest,
    observed_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UnknownRecoveryObservationDigestRecord {
    outcome: UnknownOutcome,
    observation_kind: RecoveryObservationKind,
    subject: RecoverySubjectRef,
    expected_digest: Sha256Digest,
    observed_digest: NullLiteral,
    unknown_reason: RecoveryUnknownReason,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
enum RecoveryObservationDigestRecordKind {
    Matched(MatchedRecoveryObservationDigestRecord),
    Differed(DifferedRecoveryObservationDigestRecord),
    Unknown(UnknownRecoveryObservationDigestRecord),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct RecoveryObservationDigestRecord(RecoveryObservationDigestRecordKind);

impl contract_digest_record_sealed::Sealed for RecoveryObservationDigestRecord {}
impl ContractDigestRecord for RecoveryObservationDigestRecord {}

impl JsonSchema for RecoveryObservationDigestRecord {
    fn schema_name() -> Cow<'static, str> {
        "RecoveryObservationDigestRecord".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<MatchedRecoveryObservationDigestRecord>(),
            generator.subschema_for::<DifferedRecoveryObservationDigestRecord>(),
            generator.subschema_for::<UnknownRecoveryObservationDigestRecord>(),
        ])
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MatchedRecoveryObservation {
    outcome: MatchesOutcome,
    observation_kind: RecoveryObservationKind,
    subject: RecoverySubjectRef,
    expected_digest: Sha256Digest,
    observed_digest: Sha256Digest,
    observation_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DifferedRecoveryObservation {
    outcome: DiffersOutcome,
    observation_kind: RecoveryObservationKind,
    subject: RecoverySubjectRef,
    expected_digest: Sha256Digest,
    observed_digest: Sha256Digest,
    observation_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UnknownRecoveryObservation {
    outcome: UnknownOutcome,
    observation_kind: RecoveryObservationKind,
    subject: RecoverySubjectRef,
    expected_digest: Sha256Digest,
    observed_digest: NullLiteral,
    unknown_reason: RecoveryUnknownReason,
    observation_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
enum RecoveryObservationKindWire {
    Matched(MatchedRecoveryObservation),
    Differed(DifferedRecoveryObservation),
    Unknown(UnknownRecoveryObservation),
}

/// A capability-validated observation. Deliberately not `Deserialize`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct RecoveryObservation(RecoveryObservationKindWire);

impl JsonSchema for RecoveryObservation {
    fn schema_name() -> Cow<'static, str> {
        "RecoveryObservation".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<MatchedRecoveryObservation>(),
            generator.subschema_for::<DifferedRecoveryObservation>(),
            generator.subschema_for::<UnknownRecoveryObservation>(),
        ])
    }
}

impl RecoveryObservation {
    #[cfg(test)]
    fn matched_test_only(
        observation_kind: RecoveryObservationKind,
        subject: RecoverySubjectRef,
        expected_digest: Sha256Digest,
        observed_digest: Sha256Digest,
    ) -> Result<Self, RecoveryContractError> {
        if expected_digest != observed_digest {
            return Err(RecoveryContractError(
                "matched recovery observation digests differ",
            ));
        }
        let record = RecoveryObservationDigestRecord(RecoveryObservationDigestRecordKind::Matched(
            MatchedRecoveryObservationDigestRecord {
                outcome: MatchesOutcome::Value,
                observation_kind,
                subject: subject.clone(),
                expected_digest: expected_digest.clone(),
                observed_digest: observed_digest.clone(),
            },
        ));
        let observation_digest = contract_digest(&record, "recovery observation digest failed")?;
        Ok(Self(RecoveryObservationKindWire::Matched(
            MatchedRecoveryObservation {
                outcome: MatchesOutcome::Value,
                observation_kind,
                subject,
                expected_digest,
                observed_digest,
                observation_digest,
            },
        )))
    }

    #[cfg(test)]
    fn differed_test_only(
        observation_kind: RecoveryObservationKind,
        subject: RecoverySubjectRef,
        expected_digest: Sha256Digest,
        observed_digest: Sha256Digest,
    ) -> Result<Self, RecoveryContractError> {
        if expected_digest == observed_digest {
            return Err(RecoveryContractError(
                "differed recovery observation digests are equal",
            ));
        }
        let record =
            RecoveryObservationDigestRecord(RecoveryObservationDigestRecordKind::Differed(
                DifferedRecoveryObservationDigestRecord {
                    outcome: DiffersOutcome::Value,
                    observation_kind,
                    subject: subject.clone(),
                    expected_digest: expected_digest.clone(),
                    observed_digest: observed_digest.clone(),
                },
            ));
        let observation_digest = contract_digest(&record, "recovery observation digest failed")?;
        Ok(Self(RecoveryObservationKindWire::Differed(
            DifferedRecoveryObservation {
                outcome: DiffersOutcome::Value,
                observation_kind,
                subject,
                expected_digest,
                observed_digest,
                observation_digest,
            },
        )))
    }

    #[cfg(test)]
    fn unknown_test_only(
        observation_kind: RecoveryObservationKind,
        subject: RecoverySubjectRef,
        expected_digest: Sha256Digest,
        unknown_reason: RecoveryUnknownReason,
    ) -> Result<Self, RecoveryContractError> {
        let record = RecoveryObservationDigestRecord(RecoveryObservationDigestRecordKind::Unknown(
            UnknownRecoveryObservationDigestRecord {
                outcome: UnknownOutcome::Value,
                observation_kind,
                subject: subject.clone(),
                expected_digest: expected_digest.clone(),
                observed_digest: NullLiteral,
                unknown_reason,
            },
        ));
        let observation_digest = contract_digest(&record, "recovery observation digest failed")?;
        Ok(Self(RecoveryObservationKindWire::Unknown(
            UnknownRecoveryObservation {
                outcome: UnknownOutcome::Value,
                observation_kind,
                subject,
                expected_digest,
                observed_digest: NullLiteral,
                unknown_reason,
                observation_digest,
            },
        )))
    }

    fn expected_projection(&self) -> RecoveryExpectedObservation {
        match &self.0 {
            RecoveryObservationKindWire::Matched(value) => RecoveryExpectedObservation::new(
                value.observation_kind,
                value.subject.clone(),
                value.expected_digest.clone(),
            ),
            RecoveryObservationKindWire::Differed(value) => RecoveryExpectedObservation::new(
                value.observation_kind,
                value.subject.clone(),
                value.expected_digest.clone(),
            ),
            RecoveryObservationKindWire::Unknown(value) => RecoveryExpectedObservation::new(
                value.observation_kind,
                value.subject.clone(),
                value.expected_digest.clone(),
            ),
        }
    }

    pub(crate) fn observation_digest(&self) -> &Sha256Digest {
        match &self.0 {
            RecoveryObservationKindWire::Matched(value) => &value.observation_digest,
            RecoveryObservationKindWire::Differed(value) => &value.observation_digest,
            RecoveryObservationKindWire::Unknown(value) => &value.observation_digest,
        }
    }

    fn is_match(&self) -> bool {
        matches!(self.0, RecoveryObservationKindWire::Matched(_))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RecoveryUnknown {
    observation_kind: RecoveryObservationKind,
    subject: RecoverySubjectRef,
    expected_digest: Sha256Digest,
}

impl RecoveryUnknown {
    pub(crate) fn from_observation(
        observation: &RecoveryObservation,
    ) -> Result<Self, RecoveryContractError> {
        match &observation.0 {
            RecoveryObservationKindWire::Unknown(value) => Ok(Self {
                observation_kind: value.observation_kind,
                subject: value.subject.clone(),
                expected_digest: value.expected_digest.clone(),
            }),
            RecoveryObservationKindWire::Matched(_) | RecoveryObservationKindWire::Differed(_) => {
                Err(RecoveryContractError(
                    "recovery unknown must project an unknown observation",
                ))
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
struct RecoveryExpectedObservations(Vec<RecoveryExpectedObservation>);

impl RecoveryExpectedObservations {
    fn new(values: Vec<RecoveryExpectedObservation>) -> Result<Self, RecoveryContractError> {
        if values.is_empty() || values.len() > MAX_RECOVERY_ITEMS {
            return Err(RecoveryContractError(
                "recovery expected observations must be non-empty and bounded",
            ));
        }
        let mut previous = None;
        for value in &values {
            let key = value.canonical_key()?;
            if previous.as_ref().is_some_and(|previous| previous >= &key) {
                return Err(RecoveryContractError(
                    "recovery expected observations must be canonical and unique",
                ));
            }
            previous = Some(key);
        }
        Ok(Self(values))
    }

    fn as_slice(&self) -> &[RecoveryExpectedObservation] {
        &self.0
    }
}

impl JsonSchema for RecoveryExpectedObservations {
    fn schema_name() -> Cow<'static, str> {
        "RecoveryExpectedObservations".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        let item = generator.subschema_for::<RecoveryExpectedObservation>();
        json_schema!({
            "type": "array",
            "items": item,
            "minItems": 1,
            "maxItems": MAX_RECOVERY_ITEMS,
            "uniqueItems": true
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(transparent)]
pub(crate) struct RecoveryExpectedPostconditionDigestRecord(RecoveryExpectedObservations);

impl contract_digest_record_sealed::Sealed for RecoveryExpectedPostconditionDigestRecord {}
impl ContractDigestRecord for RecoveryExpectedPostconditionDigestRecord {}

fn expected_postcondition(
    values: Vec<RecoveryExpectedObservation>,
) -> Result<(RecoveryExpectedObservations, Sha256Digest), RecoveryContractError> {
    let values = RecoveryExpectedObservations::new(values)?;
    let digest = contract_digest(
        &RecoveryExpectedPostconditionDigestRecord(values.clone()),
        "recovery expected postcondition digest failed",
    )?;
    Ok((values, digest))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
struct RecoverySubjects(Vec<RecoverySubjectRef>);

impl RecoverySubjects {
    fn new(values: Vec<RecoverySubjectRef>) -> Result<Self, RecoveryContractError> {
        if values.is_empty() || values.len() > MAX_RECOVERY_ITEMS {
            return Err(RecoveryContractError(
                "recovery subjects must be non-empty and bounded",
            ));
        }
        let mut previous = None;
        for value in &values {
            let key = value.canonical_key()?;
            if previous.as_ref().is_some_and(|previous| previous >= &key) {
                return Err(RecoveryContractError(
                    "recovery subjects must be canonical and unique",
                ));
            }
            previous = Some(key);
        }
        Ok(Self(values))
    }

    fn as_slice(&self) -> &[RecoverySubjectRef] {
        &self.0
    }
}

impl JsonSchema for RecoverySubjects {
    fn schema_name() -> Cow<'static, str> {
        "RecoverySubjects".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        let item = generator.subschema_for::<RecoverySubjectRef>();
        json_schema!({
            "type": "array",
            "items": item,
            "minItems": 1,
            "maxItems": MAX_RECOVERY_ITEMS,
            "uniqueItems": true
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct HandoffRetentionReleaseReceipt {
    retention_lease_id: UnicaId,
    release_action_id: UnicaId,
    release_action_digest: Sha256Digest,
    release_receipt_id: UnicaId,
    release_receipt_digest: Sha256Digest,
}

impl HandoffRetentionReleaseReceipt {
    /// Project the archive handoff receipt only from a validated performed or
    /// recovered effect outcome for the exact release action.
    pub(crate) fn from_release_outcome(
        action: &RecoveryAction,
        outcome: &RecoveryActionOutcome,
    ) -> Result<Self, RecoveryContractError> {
        let RecoveryActionKindWire::ReleaseRetentionLease(release) = &action.0 else {
            return Err(RecoveryContractError(
                "handoff receipt requires a releaseRetentionLease action",
            ));
        };
        let receipt = match &outcome.0 {
            RecoveryActionOutcomeKind::Performed(value) => &value.receipt,
            RecoveryActionOutcomeKind::RecoveredReceipt(value) => &value.receipt,
            RecoveryActionOutcomeKind::AlreadySatisfied(_) => {
                return Err(RecoveryContractError(
                    "handoff release requires an effect receipt",
                ));
            }
        };
        if outcome.action_binding() != (&release.action_id, &release.action_digest) {
            return Err(RecoveryContractError(
                "handoff outcome belongs to another release action",
            ));
        }
        let EffectReceiptKind::RecoveryAction(receipt) = &receipt.0 else {
            return Err(RecoveryContractError(
                "handoff release requires a recovery-action receipt",
            ));
        };
        let expected_observation_digests =
            expected_matched_terminal_observation_digests(&release.expected_observations)?;
        if receipt.receipt_id != release.expected_release_receipt_id
            || !receipt.validates(action, &expected_observation_digests)
        {
            return Err(RecoveryContractError(
                "handoff release receipt does not match its action and observations",
            ));
        }
        Ok(Self {
            retention_lease_id: release.retention_lease_id.clone(),
            release_action_id: release.action_id.clone(),
            release_action_digest: release.action_digest.clone(),
            release_receipt_id: receipt.receipt_id.clone(),
            release_receipt_digest: receipt.receipt_digest.clone(),
        })
    }

    pub(crate) const fn retention_lease_id(&self) -> &UnicaId {
        &self.retention_lease_id
    }

    pub(crate) const fn release_action_id(&self) -> &UnicaId {
        &self.release_action_id
    }

    pub(crate) const fn release_action_digest(&self) -> &Sha256Digest {
        &self.release_action_digest
    }

    pub(crate) const fn release_receipt_id(&self) -> &UnicaId {
        &self.release_receipt_id
    }

    pub(crate) const fn release_receipt_digest(&self) -> &Sha256Digest {
        &self.release_receipt_digest
    }

    #[cfg(test)]
    pub(crate) fn test_only(
        retention_lease_id: UnicaId,
        release_action_id: UnicaId,
        release_action_digest: Sha256Digest,
        release_receipt_id: UnicaId,
        release_receipt_digest: Sha256Digest,
    ) -> Self {
        Self {
            retention_lease_id,
            release_action_id,
            release_action_digest,
            release_receipt_id,
            release_receipt_digest,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct HandoffRetentionReleaseReceipts(Vec<HandoffRetentionReleaseReceipt>);

impl HandoffRetentionReleaseReceipts {
    pub(crate) fn new(
        values: Vec<HandoffRetentionReleaseReceipt>,
    ) -> Result<Self, RecoveryContractError> {
        if values.len() > MAX_RECOVERY_ITEMS {
            return Err(RecoveryContractError(
                "handoff retention release receipts are oversized",
            ));
        }
        let mut previous: Option<&UnicaId> = None;
        for value in &values {
            if previous.is_some_and(|previous| previous >= &value.retention_lease_id) {
                return Err(RecoveryContractError(
                    "handoff retention release receipts must be canonical and unique",
                ));
            }
            previous = Some(&value.retention_lease_id);
        }
        Ok(Self(values))
    }

    pub(crate) fn as_slice(&self) -> &[HandoffRetentionReleaseReceipt] {
        &self.0
    }

    pub(crate) fn release_receipt_digests(&self) -> Vec<Sha256Digest> {
        self.0
            .iter()
            .map(|receipt| receipt.release_receipt_digest.clone())
            .collect()
    }
}

impl JsonSchema for HandoffRetentionReleaseReceipts {
    fn schema_name() -> Cow<'static, str> {
        "HandoffRetentionReleaseReceipts".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        let item = generator.subschema_for::<HandoffRetentionReleaseReceipt>();
        json_schema!({
            "type": "array",
            "items": item,
            "minItems": 0,
            "maxItems": MAX_RECOVERY_ITEMS,
            "uniqueItems": true
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(transparent)]
pub(crate) struct HandoffRetentionReleaseSetDigestRecord(HandoffRetentionReleaseReceipts);

impl contract_digest_record_sealed::Sealed for HandoffRetentionReleaseSetDigestRecord {}
impl ContractDigestRecord for HandoffRetentionReleaseSetDigestRecord {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
struct RecoveryUnicaIds(Vec<UnicaId>);

impl RecoveryUnicaIds {
    fn new(values: Vec<UnicaId>) -> Result<Self, RecoveryContractError> {
        if values.len() > MAX_RECOVERY_ITEMS || values.windows(2).any(|pair| pair[0] >= pair[1]) {
            return Err(RecoveryContractError(
                "recovery identifier set must be bounded, canonical, and unique",
            ));
        }
        Ok(Self(values))
    }
}

impl JsonSchema for RecoveryUnicaIds {
    fn schema_name() -> Cow<'static, str> {
        "RecoveryUnicaIds".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        let item = generator.subschema_for::<UnicaId>();
        json_schema!({
            "type": "array",
            "items": item,
            "minItems": 0,
            "maxItems": MAX_RECOVERY_ITEMS,
            "uniqueItems": true
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
struct RecoveryOwnedTargets(Vec<OwnedTargetLocator>);

impl RecoveryOwnedTargets {
    fn new(values: Vec<OwnedTargetLocator>) -> Result<Self, RecoveryContractError> {
        if values.is_empty() || values.len() > MAX_RECOVERY_ITEMS {
            return Err(RecoveryContractError(
                "recovery owned targets must be non-empty and bounded",
            ));
        }
        if values.windows(2).any(|pair| pair[0] >= pair[1]) {
            return Err(RecoveryContractError(
                "recovery owned targets must be canonical and unique",
            ));
        }
        Ok(Self(values))
    }
}

impl JsonSchema for RecoveryOwnedTargets {
    fn schema_name() -> Cow<'static, str> {
        "RecoveryOwnedTargets".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        let item = generator.subschema_for::<OwnedTargetLocator>();
        json_schema!({
            "type": "array",
            "items": item,
            "minItems": 1,
            "maxItems": MAX_RECOVERY_ITEMS,
            "uniqueItems": true
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
enum ExternalLeaseExpectedState {
    Available,
    ExclusivelyHeld,
    Released,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
enum RetentionLeaseExpectedState {
    Held,
    Released,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
enum FinalizeSupportAuthorizationOutcome {
    Cancelled,
    AbandonmentFinalized,
}

macro_rules! action_leaf {
    (
        $literal:ident, $wire:literal,
        $record:ident, $full:ident,
        { $($field:ident : $field_ty:ty),+ $(,)? }
    ) => {
        wire_literal!($literal, $wire);

        #[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct $record {
            action_kind: $literal,
            action_id: UnicaId,
            $($field: $field_ty,)+
            expected_observations: RecoveryExpectedObservations,
            expected_postcondition_digest: Sha256Digest,
        }

        #[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct $full {
            action_kind: $literal,
            action_id: UnicaId,
            $($field: $field_ty,)+
            expected_observations: RecoveryExpectedObservations,
            expected_postcondition_digest: Sha256Digest,
            action_digest: Sha256Digest,
        }

        impl $record {
            fn with_digest(self, action_digest: Sha256Digest) -> $full {
                $full {
                    action_kind: self.action_kind,
                    action_id: self.action_id,
                    $($field: self.$field,)+
                    expected_observations: self.expected_observations,
                    expected_postcondition_digest: self.expected_postcondition_digest,
                    action_digest,
                }
            }
        }

        impl $full {
            fn common(&self) -> (&UnicaId, &RecoveryExpectedObservations, &Sha256Digest, &Sha256Digest) {
                (
                    &self.action_id,
                    &self.expected_observations,
                    &self.expected_postcondition_digest,
                    &self.action_digest,
                )
            }
        }
    };
}

action_leaf!(
    ReleaseOwnedLocksActionKind,
    "releaseOwnedLocks",
    ReleaseOwnedLocksActionDigestRecord,
    ReleaseOwnedLocksAction,
    {
        subjects: RecoverySubjects,
        expected_owned_lock_set_digest: Sha256Digest
    }
);
action_leaf!(
    RestoreOriginalActionKind,
    "restoreOriginal",
    RestoreOriginalActionDigestRecord,
    RestoreOriginalAction,
    {
        checkpoint_id: UnicaId,
        expected_original_fingerprint: Sha256Digest
    }
);
action_leaf!(
    RestoreTaskCheckpointActionKind,
    "restoreTaskCheckpoint",
    RestoreTaskCheckpointActionDigestRecord,
    RestoreTaskCheckpointAction,
    {
        checkpoint_id: UnicaId,
        expected_task_fingerprint: Sha256Digest
    }
);
action_leaf!(
    RecreateTaskInfobaseActionKind,
    "recreateTaskInfobase",
    RecreateTaskInfobaseActionDigestRecord,
    RecreateTaskInfobaseAction,
    {
        source_checkpoint_id: UnicaId,
        expected_task_fingerprint: Sha256Digest
    }
);
action_leaf!(
    VerifyTaskFingerprintActionKind,
    "verifyTaskFingerprint",
    VerifyTaskFingerprintActionDigestRecord,
    VerifyTaskFingerprintAction,
    { expected_task_fingerprint: Sha256Digest }
);
action_leaf!(
    ObserveCommitActionKind,
    "observeCommit",
    ObserveCommitActionDigestRecord,
    ObserveCommitAction,
    {
        operation_id: OperationId,
        integration_set_id: UnicaId,
        expected_integration_set_digest: Sha256Digest
    }
);
action_leaf!(
    ObservePreArmCancellationOutcomeActionKind,
    "observePreArmCancellationOutcome",
    ObservePreArmCancellationOutcomeActionDigestRecord,
    ObservePreArmCancellationOutcomeAction,
    {
        prior_operation_id: OperationId,
        support_action_id: UnicaId,
        expected_support_action_digest: Sha256Digest,
        approved_cancellation_digest: Sha256Digest
    }
);
action_leaf!(
    AcquirePreArmRootGuardActionKind,
    "acquirePreArmRootGuard",
    AcquirePreArmRootGuardActionDigestRecord,
    AcquirePreArmRootGuardAction,
    {
        finalization_attempt_id: UnicaId,
        finalization_plan_digest: Sha256Digest,
        support_action_id: UnicaId,
        receipt_ref: PreArmCancellationReceiptRef
    }
);
action_leaf!(
    RecheckPreArmCancellationFinalizationActionKind,
    "recheckPreArmCancellationFinalization",
    RecheckPreArmCancellationFinalizationActionDigestRecord,
    RecheckPreArmCancellationFinalizationAction,
    {
        finalization_attempt_id: UnicaId,
        finalization_plan_digest: Sha256Digest,
        effect_observation_digest: Sha256Digest,
        recheck_policy_digest: Sha256Digest
    }
);
action_leaf!(
    ApplyPreArmCancellationSelectiveUpdateActionKind,
    "applyPreArmCancellationSelectiveUpdate",
    ApplyPreArmCancellationSelectiveUpdateActionDigestRecord,
    ApplyPreArmCancellationSelectiveUpdateAction,
    {
        finalization_attempt_id: UnicaId,
        finalization_plan_digest: Sha256Digest,
        selective_update_plan_digest: Sha256Digest,
        expected_target_revision_map_digest: Sha256Digest,
        receipt_ref: PreArmCancellationReceiptRef
    }
);
action_leaf!(
    PersistPreArmSupportCancellationActionKind,
    "persistPreArmSupportCancellation",
    PersistPreArmSupportCancellationActionDigestRecord,
    PersistPreArmSupportCancellationAction,
    {
        finalization_attempt_id: UnicaId,
        support_action_id: UnicaId,
        expected_support_action_digest: Sha256Digest,
        approved_cancellation_digest: Sha256Digest,
        effect_observation_digest: Sha256Digest,
        finalization_plan_digest: Sha256Digest,
        receipt_ref: PreArmCancellationReceiptRef
    }
);
action_leaf!(
    ReleasePreArmRootGuardActionKind,
    "releasePreArmRootGuard",
    ReleasePreArmRootGuardActionDigestRecord,
    ReleasePreArmRootGuardAction,
    {
        finalization_attempt_id: UnicaId,
        finalization_plan_digest: Sha256Digest,
        support_action_id: UnicaId,
        receipt_ref: PreArmCancellationReceiptRef
    }
);
action_leaf!(
    FinishPreArmCancellationRecoveryActionKind,
    "finishPreArmCancellationRecovery",
    FinishPreArmCancellationRecoveryActionDigestRecord,
    FinishPreArmCancellationRecoveryAction,
    {
        finalization_attempt_id: UnicaId,
        support_action_id: UnicaId,
        expected_support_action_digest: Sha256Digest,
        approved_cancellation_digest: Sha256Digest,
        effect_observation_digest: Sha256Digest,
        finalization_plan_digest: Sha256Digest,
        receipt_plan_digest: Sha256Digest,
        expected_result_phase: TaskPhase,
        receipt_ref: PreArmCancellationReceiptRef
    }
);
action_leaf!(
    QuarantineArtifactActionKind,
    "quarantineArtifact",
    QuarantineArtifactActionDigestRecord,
    QuarantineArtifactAction,
    {
        artifact_id: UnicaId,
        expected_artifact_sha256: Sha256Digest,
        quarantine_id: UnicaId
    }
);
action_leaf!(
    ObserveSupportPrerequisiteHistoryActionKind,
    "observeSupportPrerequisiteHistory",
    ObserveSupportPrerequisiteHistoryActionDigestRecord,
    ObserveSupportPrerequisiteHistoryAction,
    {
        support_action_id: UnicaId,
        from_cursor: RepositoryHistoryCursor,
        through_cursor: RepositoryHistoryCursor,
        expected_partition_digest: Sha256Digest
    }
);
action_leaf!(
    UpdateOriginalSelectedTargetsActionKind,
    "updateOriginalSelectedTargets",
    UpdateOriginalSelectedTargetsActionDigestRecord,
    UpdateOriginalSelectedTargetsAction,
    {
        finalization_plan_digest: Sha256Digest,
        selective_update_plan_digest: Sha256Digest,
        expected_target_revision_map_digest: Sha256Digest
    }
);
action_leaf!(
    ObserveWorkingInfobaseLeaseActionKind,
    "observeWorkingInfobaseLease",
    ObserveWorkingInfobaseLeaseActionDigestRecord,
    ObserveWorkingInfobaseLeaseAction,
    {
        working_infobase_identity: ManualWorkingInfobaseIdentity,
        exclusive_lease_capability_id: CapabilityRowId,
        expected_lease_state: ExternalLeaseExpectedState
    }
);
action_leaf!(
    ReleaseWorkingInfobaseLeaseActionKind,
    "releaseWorkingInfobaseLease",
    ReleaseWorkingInfobaseLeaseActionDigestRecord,
    ReleaseWorkingInfobaseLeaseAction,
    {
        working_infobase_identity: ManualWorkingInfobaseIdentity,
        exclusive_lease_capability_id: CapabilityRowId,
        exclusive_lease_receipt_id: UnicaId,
        expected_release_receipt_id: UnicaId
    }
);
action_leaf!(
    ObserveReservedOriginalLeaseActionKind,
    "observeReservedOriginalLease",
    ObserveReservedOriginalLeaseActionDigestRecord,
    ObserveReservedOriginalLeaseAction,
    {
        reserved_original_identity_digest: Sha256Digest,
        exclusive_lease_capability_id: CapabilityRowId,
        expected_lease_state: ExternalLeaseExpectedState
    }
);
action_leaf!(
    ReleaseReservedOriginalLeaseActionKind,
    "releaseReservedOriginalLease",
    ReleaseReservedOriginalLeaseActionDigestRecord,
    ReleaseReservedOriginalLeaseAction,
    {
        reserved_original_identity_digest: Sha256Digest,
        exclusive_lease_capability_id: CapabilityRowId,
        exclusive_lease_receipt_id: UnicaId,
        expected_release_receipt_id: UnicaId
    }
);
action_leaf!(
    ObserveRetentionLeaseActionKind,
    "observeRetentionLease",
    ObserveRetentionLeaseActionDigestRecord,
    ObserveRetentionLeaseAction,
    {
        retention_lease_id: UnicaId,
        retention_capability_row_id: CapabilityRowId,
        expected_lease_state: RetentionLeaseExpectedState
    }
);
action_leaf!(
    ObserveArchiveStagingActionKind,
    "observeArchiveStaging",
    ObserveArchiveStagingActionDigestRecord,
    ObserveArchiveStagingAction,
    {
        archive_staging_receipt_id: UnicaId,
        expected_archive_staging_receipt_digest: Sha256Digest,
        handoff_lineage_digest: Sha256Digest
    }
);
action_leaf!(
    ReleaseRetentionLeaseActionKind,
    "releaseRetentionLease",
    ReleaseRetentionLeaseActionDigestRecord,
    ReleaseRetentionLeaseAction,
    {
        retention_lease_id: UnicaId,
        retention_acquire_receipt_id: UnicaId,
        retention_capability_row_id: CapabilityRowId,
        archive_staging_receipt_id: UnicaId,
        expected_archive_staging_receipt_digest: Sha256Digest,
        expected_release_receipt_id: UnicaId,
        expected_released: TrueLiteral
    }
);
action_leaf!(
    AwaitExternalSupportCorrectionActionKind,
    "awaitExternalSupportCorrection",
    AwaitExternalSupportCorrectionActionDigestRecord,
    AwaitExternalSupportCorrectionAction,
    {
        support_action_id: UnicaId,
        corrective_instruction_digest: Sha256Digest
    }
);
action_leaf!(
    AwaitExternalLockReleaseActionKind,
    "awaitExternalLockRelease",
    AwaitExternalLockReleaseActionDigestRecord,
    AwaitExternalLockReleaseAction,
    {
        lock_instruction_digest: Sha256Digest,
        subjects: RecoverySubjects
    }
);
action_leaf!(
    AwaitManualWorkingInfobaseClosureActionKind,
    "awaitManualWorkingInfobaseClosure",
    AwaitManualWorkingInfobaseClosureActionDigestRecord,
    AwaitManualWorkingInfobaseClosureAction,
    {
        working_infobase_identity: ManualWorkingInfobaseIdentity,
        closure_plan_digest: Sha256Digest,
        exclusive_lease_capability_id: CapabilityRowId
    }
);
action_leaf!(
    AwaitReservedOriginalClosureActionKind,
    "awaitReservedOriginalClosure",
    AwaitReservedOriginalClosureActionDigestRecord,
    AwaitReservedOriginalClosureAction,
    {
        reserved_original_identity_digest: Sha256Digest,
        exclusive_lease_capability_id: CapabilityRowId
    }
);
action_leaf!(
    AwaitExternalSupportConflictResolutionActionKind,
    "awaitExternalSupportConflictResolution",
    AwaitExternalSupportConflictResolutionActionDigestRecord,
    AwaitExternalSupportConflictResolutionAction,
    {
        support_action_id: UnicaId,
        support_conflict_instruction_digest: Sha256Digest
    }
);
action_leaf!(
    AwaitSupportRecoveryEvidenceActionKind,
    "awaitSupportRecoveryEvidence",
    AwaitSupportRecoveryEvidenceActionDigestRecord,
    AwaitSupportRecoveryEvidenceAction,
    {
        support_action_id: UnicaId,
        support_evidence_instruction_digest: Sha256Digest
    }
);
action_leaf!(
    FinalizeSupportPrerequisiteRecoveryActionKind,
    "finalizeSupportPrerequisiteRecovery",
    FinalizeSupportPrerequisiteRecoveryActionDigestRecord,
    FinalizeSupportPrerequisiteRecoveryAction,
    {
        support_action_id: UnicaId,
        finalization_plan_digest: Sha256Digest,
        authorization_outcome: FinalizeSupportAuthorizationOutcome
    }
);
action_leaf!(
    ResumeQuarantineActionKind,
    "resumeQuarantine",
    ResumeQuarantineActionDigestRecord,
    ResumeQuarantineAction,
    {
        artifact_id: UnicaId,
        quarantine_id: UnicaId
    }
);
action_leaf!(
    ResumeOwnedTargetQuarantineActionKind,
    "resumeOwnedTargetQuarantine",
    ResumeOwnedTargetQuarantineActionDigestRecord,
    ResumeOwnedTargetQuarantineAction,
    {
        owned_target: OwnedTargetLocator,
        quarantine_id: UnicaId,
        expected_quarantined_digest: Sha256Digest
    }
);
action_leaf!(
    FinishArchiveActionKind,
    "finishArchive",
    FinishArchiveActionDigestRecord,
    FinishArchiveAction,
    {
        archive_id: UnicaId,
        archive_staging_receipt_id: UnicaId,
        expected_archive_staging_receipt_digest: Sha256Digest,
        handoff_lineage_digest: Sha256Digest,
        retention_lease_ids: RecoveryUnicaIds,
        expected_releases: HandoffRetentionReleaseReceipts,
        expected_release_set_digest: Sha256Digest
    }
);
action_leaf!(
    FinishCleanupActionKind,
    "finishCleanup",
    FinishCleanupActionDigestRecord,
    FinishCleanupAction,
    {
        archive_id: UnicaId,
        owned_targets: RecoveryOwnedTargets,
        expected_all_absent: TrueLiteral
    }
);

wire_literal!(AcquirePreArmModeLeaseActionKind, "acquirePreArmModeLease");
wire_literal!(ReleasePreArmModeLeaseActionKind, "releasePreArmModeLease");
wire_literal!(ReservedOriginalModeLiteral, "reservedOriginal");
wire_literal!(
    SeparateWorkingInfobaseModeLiteral,
    "separateWorkingInfobase"
);

macro_rules! prearm_mode_action_leaf {
    (
        $record:ident, $full:ident, $kind:ty,
        $mode:ty, { $identity:ident : $identity_ty:ty },
        { $($binding:ident : $binding_ty:ty),* $(,)? }
    ) => {
        #[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct $record {
            action_kind: $kind,
            action_id: UnicaId,
            finalization_attempt_id: UnicaId,
            finalization_plan_digest: Sha256Digest,
            $($binding: $binding_ty,)*
            manual_target_mode: $mode,
            $identity: $identity_ty,
            exclusive_lease_capability_id: CapabilityRowId,
            receipt_ref: PreArmCancellationReceiptRef,
            expected_observations: RecoveryExpectedObservations,
            expected_postcondition_digest: Sha256Digest,
        }

        #[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct $full {
            action_kind: $kind,
            action_id: UnicaId,
            finalization_attempt_id: UnicaId,
            finalization_plan_digest: Sha256Digest,
            $($binding: $binding_ty,)*
            manual_target_mode: $mode,
            $identity: $identity_ty,
            exclusive_lease_capability_id: CapabilityRowId,
            receipt_ref: PreArmCancellationReceiptRef,
            expected_observations: RecoveryExpectedObservations,
            expected_postcondition_digest: Sha256Digest,
            action_digest: Sha256Digest,
        }

        impl $record {
            fn with_digest(self, action_digest: Sha256Digest) -> $full {
                $full {
                    action_kind: self.action_kind,
                    action_id: self.action_id,
                    finalization_attempt_id: self.finalization_attempt_id,
                    finalization_plan_digest: self.finalization_plan_digest,
                    $($binding: self.$binding,)*
                    manual_target_mode: self.manual_target_mode,
                    $identity: self.$identity,
                    exclusive_lease_capability_id: self.exclusive_lease_capability_id,
                    receipt_ref: self.receipt_ref,
                    expected_observations: self.expected_observations,
                    expected_postcondition_digest: self.expected_postcondition_digest,
                    action_digest,
                }
            }
        }

        impl $full {
            fn common(
                &self,
            ) -> (
                &UnicaId,
                &RecoveryExpectedObservations,
                &Sha256Digest,
                &Sha256Digest,
            ) {
                (
                    &self.action_id,
                    &self.expected_observations,
                    &self.expected_postcondition_digest,
                    &self.action_digest,
                )
            }
        }
    };
}

prearm_mode_action_leaf!(
    AcquirePreArmReservedOriginalModeLeaseActionDigestRecord,
    AcquirePreArmReservedOriginalModeLeaseAction,
    AcquirePreArmModeLeaseActionKind,
    ReservedOriginalModeLiteral,
    { reserved_original_identity_digest: Sha256Digest },
    { support_action_id: UnicaId }
);
prearm_mode_action_leaf!(
    AcquirePreArmWorkingInfobaseModeLeaseActionDigestRecord,
    AcquirePreArmWorkingInfobaseModeLeaseAction,
    AcquirePreArmModeLeaseActionKind,
    SeparateWorkingInfobaseModeLiteral,
    { working_infobase_identity: ManualWorkingInfobaseIdentity },
    { support_action_id: UnicaId }
);
prearm_mode_action_leaf!(
    ReleasePreArmReservedOriginalModeLeaseActionDigestRecord,
    ReleasePreArmReservedOriginalModeLeaseAction,
    ReleasePreArmModeLeaseActionKind,
    ReservedOriginalModeLiteral,
    { reserved_original_identity_digest: Sha256Digest },
    {}
);
prearm_mode_action_leaf!(
    ReleasePreArmWorkingInfobaseModeLeaseActionDigestRecord,
    ReleasePreArmWorkingInfobaseModeLeaseAction,
    ReleasePreArmModeLeaseActionKind,
    SeparateWorkingInfobaseModeLiteral,
    { working_infobase_identity: ManualWorkingInfobaseIdentity },
    {}
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
enum AcquirePreArmModeLeaseActionDigestRecordKind {
    ReservedOriginal(AcquirePreArmReservedOriginalModeLeaseActionDigestRecord),
    SeparateWorkingInfobase(AcquirePreArmWorkingInfobaseModeLeaseActionDigestRecord),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
struct AcquirePreArmModeLeaseActionDigestRecord(AcquirePreArmModeLeaseActionDigestRecordKind);

impl JsonSchema for AcquirePreArmModeLeaseActionDigestRecord {
    fn schema_name() -> Cow<'static, str> {
        "AcquirePreArmModeLeaseActionDigestRecord".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<AcquirePreArmReservedOriginalModeLeaseActionDigestRecord>(),
            generator.subschema_for::<AcquirePreArmWorkingInfobaseModeLeaseActionDigestRecord>(),
        ])
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
enum AcquirePreArmModeLeaseActionKindWire {
    ReservedOriginal(AcquirePreArmReservedOriginalModeLeaseAction),
    SeparateWorkingInfobase(AcquirePreArmWorkingInfobaseModeLeaseAction),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
struct AcquirePreArmModeLeaseAction(AcquirePreArmModeLeaseActionKindWire);

impl AcquirePreArmModeLeaseActionDigestRecord {
    fn with_digest(self, action_digest: Sha256Digest) -> AcquirePreArmModeLeaseAction {
        AcquirePreArmModeLeaseAction(match self.0 {
            AcquirePreArmModeLeaseActionDigestRecordKind::ReservedOriginal(value) => {
                AcquirePreArmModeLeaseActionKindWire::ReservedOriginal(
                    value.with_digest(action_digest),
                )
            }
            AcquirePreArmModeLeaseActionDigestRecordKind::SeparateWorkingInfobase(value) => {
                AcquirePreArmModeLeaseActionKindWire::SeparateWorkingInfobase(
                    value.with_digest(action_digest),
                )
            }
        })
    }
}

impl AcquirePreArmModeLeaseAction {
    fn common(
        &self,
    ) -> (
        &UnicaId,
        &RecoveryExpectedObservations,
        &Sha256Digest,
        &Sha256Digest,
    ) {
        match &self.0 {
            AcquirePreArmModeLeaseActionKindWire::ReservedOriginal(value) => value.common(),
            AcquirePreArmModeLeaseActionKindWire::SeparateWorkingInfobase(value) => value.common(),
        }
    }
}

impl JsonSchema for AcquirePreArmModeLeaseAction {
    fn schema_name() -> Cow<'static, str> {
        "AcquirePreArmModeLeaseAction".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<AcquirePreArmReservedOriginalModeLeaseAction>(),
            generator.subschema_for::<AcquirePreArmWorkingInfobaseModeLeaseAction>(),
        ])
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
enum ReleasePreArmModeLeaseActionDigestRecordKind {
    ReservedOriginal(ReleasePreArmReservedOriginalModeLeaseActionDigestRecord),
    SeparateWorkingInfobase(ReleasePreArmWorkingInfobaseModeLeaseActionDigestRecord),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
struct ReleasePreArmModeLeaseActionDigestRecord(ReleasePreArmModeLeaseActionDigestRecordKind);

impl JsonSchema for ReleasePreArmModeLeaseActionDigestRecord {
    fn schema_name() -> Cow<'static, str> {
        "ReleasePreArmModeLeaseActionDigestRecord".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<ReleasePreArmReservedOriginalModeLeaseActionDigestRecord>(),
            generator.subschema_for::<ReleasePreArmWorkingInfobaseModeLeaseActionDigestRecord>(),
        ])
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
enum ReleasePreArmModeLeaseActionKindWire {
    ReservedOriginal(ReleasePreArmReservedOriginalModeLeaseAction),
    SeparateWorkingInfobase(ReleasePreArmWorkingInfobaseModeLeaseAction),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
struct ReleasePreArmModeLeaseAction(ReleasePreArmModeLeaseActionKindWire);

impl ReleasePreArmModeLeaseActionDigestRecord {
    fn with_digest(self, action_digest: Sha256Digest) -> ReleasePreArmModeLeaseAction {
        ReleasePreArmModeLeaseAction(match self.0 {
            ReleasePreArmModeLeaseActionDigestRecordKind::ReservedOriginal(value) => {
                ReleasePreArmModeLeaseActionKindWire::ReservedOriginal(
                    value.with_digest(action_digest),
                )
            }
            ReleasePreArmModeLeaseActionDigestRecordKind::SeparateWorkingInfobase(value) => {
                ReleasePreArmModeLeaseActionKindWire::SeparateWorkingInfobase(
                    value.with_digest(action_digest),
                )
            }
        })
    }
}

impl ReleasePreArmModeLeaseAction {
    fn common(
        &self,
    ) -> (
        &UnicaId,
        &RecoveryExpectedObservations,
        &Sha256Digest,
        &Sha256Digest,
    ) {
        match &self.0 {
            ReleasePreArmModeLeaseActionKindWire::ReservedOriginal(value) => value.common(),
            ReleasePreArmModeLeaseActionKindWire::SeparateWorkingInfobase(value) => value.common(),
        }
    }
}

impl JsonSchema for ReleasePreArmModeLeaseAction {
    fn schema_name() -> Cow<'static, str> {
        "ReleasePreArmModeLeaseAction".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<ReleasePreArmReservedOriginalModeLeaseAction>(),
            generator.subschema_for::<ReleasePreArmWorkingInfobaseModeLeaseAction>(),
        ])
    }
}

macro_rules! recovery_action_union {
    ($($variant:ident : $record:ty => $full:ty),+ $(,)?) => {
        #[derive(Debug, Clone, PartialEq, Eq, Serialize)]
        #[serde(untagged)]
        enum RecoveryActionDigestRecordKind {
            $($variant($record),)+
        }

        #[derive(Debug, Clone, PartialEq, Eq, Serialize)]
        #[serde(transparent)]
        pub(crate) struct RecoveryActionDigestRecord(RecoveryActionDigestRecordKind);

        impl contract_digest_record_sealed::Sealed for RecoveryActionDigestRecord {}
        impl ContractDigestRecord for RecoveryActionDigestRecord {}

        impl JsonSchema for RecoveryActionDigestRecord {
            fn schema_name() -> Cow<'static, str> {
                "RecoveryActionDigestRecord".into()
            }

            fn json_schema(generator: &mut SchemaGenerator) -> Schema {
                one_of_schema(vec![$(generator.subschema_for::<$record>(),)+])
            }
        }

        #[derive(Debug, Clone, PartialEq, Eq, Serialize)]
        #[serde(untagged)]
        enum RecoveryActionKindWire {
            $($variant($full),)+
        }

        /// A plan-authority-backed action. Deliberately not `Deserialize`.
        #[derive(Debug, Clone, PartialEq, Eq, Serialize)]
        #[serde(transparent)]
        pub(crate) struct RecoveryAction(RecoveryActionKindWire);

        impl RecoveryAction {
            fn from_record(
                record: RecoveryActionDigestRecordKind,
            ) -> Result<Self, RecoveryContractError> {
                validate_recovery_action_record(&record)?;
                let digest = contract_digest(
                    &RecoveryActionDigestRecord(record.clone()),
                    "recovery action digest failed",
                )?;
                Ok(Self(match record {
                    $(RecoveryActionDigestRecordKind::$variant(value) =>
                        RecoveryActionKindWire::$variant(value.with_digest(digest)),)+
                }))
            }

            fn common(
                &self,
            ) -> (&UnicaId, &RecoveryExpectedObservations, &Sha256Digest, &Sha256Digest) {
                match &self.0 {
                    $(RecoveryActionKindWire::$variant(value) => value.common(),)+
                }
            }

            pub(crate) fn action_id(&self) -> &UnicaId {
                self.common().0
            }

            pub(crate) fn expected_postcondition_digest(&self) -> &Sha256Digest {
                self.common().2
            }

            pub(crate) fn action_digest(&self) -> &Sha256Digest {
                self.common().3
            }
        }
    };
}

recovery_action_union!(
    ReleaseOwnedLocks: ReleaseOwnedLocksActionDigestRecord => ReleaseOwnedLocksAction,
    RestoreOriginal: RestoreOriginalActionDigestRecord => RestoreOriginalAction,
    RestoreTaskCheckpoint: RestoreTaskCheckpointActionDigestRecord => RestoreTaskCheckpointAction,
    RecreateTaskInfobase: RecreateTaskInfobaseActionDigestRecord => RecreateTaskInfobaseAction,
    VerifyTaskFingerprint: VerifyTaskFingerprintActionDigestRecord => VerifyTaskFingerprintAction,
    ObserveCommit: ObserveCommitActionDigestRecord => ObserveCommitAction,
    ObservePreArmCancellationOutcome: ObservePreArmCancellationOutcomeActionDigestRecord => ObservePreArmCancellationOutcomeAction,
    AcquirePreArmRootGuard: AcquirePreArmRootGuardActionDigestRecord => AcquirePreArmRootGuardAction,
    AcquirePreArmModeLease: AcquirePreArmModeLeaseActionDigestRecord => AcquirePreArmModeLeaseAction,
    RecheckPreArmCancellationFinalization: RecheckPreArmCancellationFinalizationActionDigestRecord => RecheckPreArmCancellationFinalizationAction,
    ApplyPreArmCancellationSelectiveUpdate: ApplyPreArmCancellationSelectiveUpdateActionDigestRecord => ApplyPreArmCancellationSelectiveUpdateAction,
    PersistPreArmSupportCancellation: PersistPreArmSupportCancellationActionDigestRecord => PersistPreArmSupportCancellationAction,
    ReleasePreArmModeLease: ReleasePreArmModeLeaseActionDigestRecord => ReleasePreArmModeLeaseAction,
    ReleasePreArmRootGuard: ReleasePreArmRootGuardActionDigestRecord => ReleasePreArmRootGuardAction,
    FinishPreArmCancellationRecovery: FinishPreArmCancellationRecoveryActionDigestRecord => FinishPreArmCancellationRecoveryAction,
    QuarantineArtifact: QuarantineArtifactActionDigestRecord => QuarantineArtifactAction,
    ObserveSupportPrerequisiteHistory: ObserveSupportPrerequisiteHistoryActionDigestRecord => ObserveSupportPrerequisiteHistoryAction,
    UpdateOriginalSelectedTargets: UpdateOriginalSelectedTargetsActionDigestRecord => UpdateOriginalSelectedTargetsAction,
    ObserveWorkingInfobaseLease: ObserveWorkingInfobaseLeaseActionDigestRecord => ObserveWorkingInfobaseLeaseAction,
    ReleaseWorkingInfobaseLease: ReleaseWorkingInfobaseLeaseActionDigestRecord => ReleaseWorkingInfobaseLeaseAction,
    ObserveReservedOriginalLease: ObserveReservedOriginalLeaseActionDigestRecord => ObserveReservedOriginalLeaseAction,
    ReleaseReservedOriginalLease: ReleaseReservedOriginalLeaseActionDigestRecord => ReleaseReservedOriginalLeaseAction,
    ObserveRetentionLease: ObserveRetentionLeaseActionDigestRecord => ObserveRetentionLeaseAction,
    ObserveArchiveStaging: ObserveArchiveStagingActionDigestRecord => ObserveArchiveStagingAction,
    ReleaseRetentionLease: ReleaseRetentionLeaseActionDigestRecord => ReleaseRetentionLeaseAction,
    AwaitExternalSupportCorrection: AwaitExternalSupportCorrectionActionDigestRecord => AwaitExternalSupportCorrectionAction,
    AwaitExternalLockRelease: AwaitExternalLockReleaseActionDigestRecord => AwaitExternalLockReleaseAction,
    AwaitManualWorkingInfobaseClosure: AwaitManualWorkingInfobaseClosureActionDigestRecord => AwaitManualWorkingInfobaseClosureAction,
    AwaitReservedOriginalClosure: AwaitReservedOriginalClosureActionDigestRecord => AwaitReservedOriginalClosureAction,
    AwaitExternalSupportConflictResolution: AwaitExternalSupportConflictResolutionActionDigestRecord => AwaitExternalSupportConflictResolutionAction,
    AwaitSupportRecoveryEvidence: AwaitSupportRecoveryEvidenceActionDigestRecord => AwaitSupportRecoveryEvidenceAction,
    FinalizeSupportPrerequisiteRecovery: FinalizeSupportPrerequisiteRecoveryActionDigestRecord => FinalizeSupportPrerequisiteRecoveryAction,
    ResumeQuarantine: ResumeQuarantineActionDigestRecord => ResumeQuarantineAction,
    ResumeOwnedTargetQuarantine: ResumeOwnedTargetQuarantineActionDigestRecord => ResumeOwnedTargetQuarantineAction,
    FinishArchive: FinishArchiveActionDigestRecord => FinishArchiveAction,
    FinishCleanup: FinishCleanupActionDigestRecord => FinishCleanupAction,
);

#[derive(Debug, Clone, PartialEq, Eq)]
struct AwaitExternalSupportCorrectionPostconditionAuthority {
    expected_support_graph_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SupportRecoveryLockReleasePostconditionObservation {
    blocked_guard_proof_digest: Sha256Digest,
    subject: RecoverySubjectRef,
    expected_unlocked_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ManualWorkingInfobaseAvailablePostconditionObservation {
    closure_plan_digest: Sha256Digest,
    working_infobase_identity: ManualWorkingInfobaseIdentity,
    expected_available_lease_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReservedOriginalAvailablePostconditionObservation {
    reserved_original_identity_digest: Sha256Digest,
    exclusive_lease_capability_id: CapabilityRowId,
    expected_available_lease_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AwaitExternalSupportConflictPostconditionAuthority {
    required_final_baseline_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SupportRecoveryEvidencePostconditionObservation {
    support_evidence_instruction_digest: Sha256Digest,
    evidence_artifact_id: UnicaId,
    expected_evidence_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum SupportRecoveryExternalWaitAuthorityKind {
    Corrective {
        action_id: UnicaId,
        external_action: SupportRecoveryExternalAction,
        postcondition: AwaitExternalSupportCorrectionPostconditionAuthority,
    },
    ReleaseLocks {
        action_id: UnicaId,
        external_action: SupportRecoveryExternalAction,
        postcondition: SupportRecoveryLockReleasePostconditionObservation,
    },
    CleanWorkingInfobase {
        action_id: UnicaId,
        external_action: SupportRecoveryExternalAction,
        postcondition: ManualWorkingInfobaseAvailablePostconditionObservation,
    },
    CloseReservedOriginal {
        action_id: UnicaId,
        external_action: SupportRecoveryExternalAction,
        postcondition: ReservedOriginalAvailablePostconditionObservation,
    },
    Conflict {
        action_id: UnicaId,
        external_action: SupportRecoveryExternalAction,
        postcondition: AwaitExternalSupportConflictPostconditionAuthority,
    },
    Evidence {
        action_id: UnicaId,
        external_action: SupportRecoveryExternalAction,
        postcondition: SupportRecoveryEvidencePostconditionObservation,
    },
}

/// One authority-minted external blocker and its exact recovery wait action.
/// The private leaf payloads prevent callers from pairing an instruction with
/// another wait kind or from supplying generic recovery observations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SupportRecoveryExternalWaitAuthority(SupportRecoveryExternalWaitAuthorityKind);

fn blocked_recovery_subject(
    proof: &SupportRecoveryGuardProof,
) -> Result<RecoverySubjectRef, RecoveryContractError> {
    match proof.blocked_target_ref() {
        Some(BlockedSupportRecoveryTargetRef::ConfigurationRoot) => {
            Ok(RecoverySubjectRef::configuration_root())
        }
        Some(BlockedSupportRecoveryTargetRef::DevelopmentObject(object_id)) => {
            Ok(RecoverySubjectRef::metadata_object(object_id.clone()))
        }
        None => Err(RecoveryContractError(
            "lock-release wait requires a blocked support recovery guard proof",
        )),
    }
}

impl SupportRecoveryLockReleasePostconditionObservation {
    pub(crate) fn from_capability_adapter(
        blocked_guard_proof: &SupportRecoveryGuardProof,
        expected_unlocked_digest: Sha256Digest,
    ) -> Result<Self, RecoveryContractError> {
        Ok(Self {
            blocked_guard_proof_digest: blocked_guard_proof.proof_digest().clone(),
            subject: blocked_recovery_subject(blocked_guard_proof)?,
            expected_unlocked_digest,
        })
    }
}

impl ManualWorkingInfobaseAvailablePostconditionObservation {
    pub(crate) fn from_capability_adapter(
        closure_plan: &ManualWorkingInfobaseClosurePlan,
        stop: &ManualWorkingInfobaseStopEvidence,
        expected_available_lease_digest: Sha256Digest,
    ) -> Result<Self, RecoveryContractError> {
        if stop.working_infobase_identity() != closure_plan.working_infobase_identity()
            || stop.closure_plan_digest() != closure_plan.plan_digest()
        {
            return Err(RecoveryContractError(
                "working-infobase availability observation belongs to another closure stop",
            ));
        }
        Ok(Self {
            closure_plan_digest: closure_plan.plan_digest().clone(),
            working_infobase_identity: closure_plan.working_infobase_identity().clone(),
            expected_available_lease_digest,
        })
    }
}

impl ReservedOriginalAvailablePostconditionObservation {
    pub(crate) fn from_capability_adapter(
        stop: &ReservedOriginalLeaseStopEvidence,
        expected_available_lease_digest: Sha256Digest,
    ) -> Self {
        Self {
            reserved_original_identity_digest: stop.reserved_original_identity_digest().clone(),
            exclusive_lease_capability_id: stop.exclusive_lease_capability_id().clone(),
            expected_available_lease_digest,
        }
    }
}

impl SupportRecoveryEvidencePostconditionObservation {
    pub(crate) fn from_capability_adapter(
        instruction: &SupportEvidenceInstruction,
        evidence_artifact_id: UnicaId,
        expected_evidence_digest: Sha256Digest,
    ) -> Self {
        Self {
            support_evidence_instruction_digest: instruction
                .support_evidence_instruction_digest()
                .clone(),
            evidence_artifact_id,
            expected_evidence_digest,
        }
    }
}

impl SupportRecoveryExternalWaitAuthority {
    pub(crate) fn corrective_from_approved(
        token: &SupportRecoveryAuthorityToken,
        action_id: UnicaId,
        instruction: SupportCorrectiveInstruction,
    ) -> Self {
        let postcondition = AwaitExternalSupportCorrectionPostconditionAuthority {
            expected_support_graph_digest: instruction.desired_support_graph_digest().clone(),
        };
        let external_action =
            SupportRecoveryExternalAction::corrective_from_approved(token, instruction);
        Self(SupportRecoveryExternalWaitAuthorityKind::Corrective {
            action_id,
            external_action,
            postcondition,
        })
    }

    pub(crate) fn release_locks_from_approved(
        token: &SupportRecoveryAuthorityToken,
        action_id: UnicaId,
        blocked_guard_proof: &SupportRecoveryGuardProof,
        postcondition: SupportRecoveryLockReleasePostconditionObservation,
    ) -> Result<Self, RecoveryContractError> {
        let subject = blocked_recovery_subject(blocked_guard_proof)?;
        if postcondition.blocked_guard_proof_digest != *blocked_guard_proof.proof_digest()
            || postcondition.subject != subject
        {
            return Err(RecoveryContractError(
                "lock-release postcondition belongs to another blocked guard proof",
            ));
        }
        let instruction =
            ReleaseRepositoryLocksInstruction::from_support_recovery_blocked_approved(
                token,
                blocked_guard_proof,
            )
            .map_err(|_| {
                RecoveryContractError(
                    "blocked support recovery proof could not mint its lock-release instruction",
                )
            })?;
        Ok(Self(
            SupportRecoveryExternalWaitAuthorityKind::ReleaseLocks {
                action_id,
                external_action: SupportRecoveryExternalAction::release_locks_from_approved(
                    token,
                    instruction,
                ),
                postcondition,
            },
        ))
    }

    pub(crate) fn clean_working_infobase_from_approved(
        token: &SupportRecoveryAuthorityToken,
        action_id: UnicaId,
        closure_plan: &ManualWorkingInfobaseClosurePlan,
        stop: &ManualWorkingInfobaseStopEvidence,
        postcondition: ManualWorkingInfobaseAvailablePostconditionObservation,
    ) -> Result<Self, RecoveryContractError> {
        let instruction =
            CleanManualWorkingInfobaseInstruction::from_support_recovery_stop_approved(
                token,
                closure_plan,
                stop,
            )
            .map_err(|_| {
                RecoveryContractError(
                    "working-infobase stop could not mint its cleanup instruction",
                )
            })?;
        if postcondition.closure_plan_digest != *closure_plan.plan_digest()
            || postcondition.working_infobase_identity != *instruction.working_infobase_identity()
        {
            return Err(RecoveryContractError(
                "working-infobase postcondition belongs to another closure plan",
            ));
        }
        Ok(Self(
            SupportRecoveryExternalWaitAuthorityKind::CleanWorkingInfobase {
                action_id,
                external_action:
                    SupportRecoveryExternalAction::clean_working_infobase_from_approved(
                        token,
                        instruction,
                    ),
                postcondition,
            },
        ))
    }

    pub(crate) fn close_reserved_original_from_approved(
        token: &SupportRecoveryAuthorityToken,
        action_id: UnicaId,
        stop: &ReservedOriginalLeaseStopEvidence,
        postcondition: ReservedOriginalAvailablePostconditionObservation,
    ) -> Result<Self, RecoveryContractError> {
        let instruction =
            CloseReservedOriginalDesignerInstruction::from_support_recovery_stop_approved(
                token, stop,
            );
        if postcondition.reserved_original_identity_digest
            != *instruction.reserved_original_identity_digest()
            || postcondition.exclusive_lease_capability_id
                != *instruction.exclusive_lease_capability_id()
        {
            return Err(RecoveryContractError(
                "reserved-original postcondition belongs to another lease stop",
            ));
        }
        Ok(Self(
            SupportRecoveryExternalWaitAuthorityKind::CloseReservedOriginal {
                action_id,
                external_action:
                    SupportRecoveryExternalAction::close_reserved_original_from_approved(
                        token,
                        instruction,
                    ),
                postcondition,
            },
        ))
    }

    pub(crate) fn conflict_from_approved(
        token: &SupportRecoveryAuthorityToken,
        action_id: UnicaId,
        instruction: SupportConflictInstruction,
    ) -> Self {
        let postcondition = AwaitExternalSupportConflictPostconditionAuthority {
            required_final_baseline_digest: instruction.required_final_baseline_digest().clone(),
        };
        let external_action =
            SupportRecoveryExternalAction::conflict_from_approved(token, instruction);
        Self(SupportRecoveryExternalWaitAuthorityKind::Conflict {
            action_id,
            external_action,
            postcondition,
        })
    }

    pub(crate) fn evidence_from_approved(
        token: &SupportRecoveryAuthorityToken,
        action_id: UnicaId,
        instruction: SupportEvidenceInstruction,
        postcondition: SupportRecoveryEvidencePostconditionObservation,
    ) -> Result<Self, RecoveryContractError> {
        if postcondition.support_evidence_instruction_digest
            != *instruction.support_evidence_instruction_digest()
        {
            return Err(RecoveryContractError(
                "support-evidence postcondition belongs to another instruction",
            ));
        }
        let external_action =
            SupportRecoveryExternalAction::evidence_from_approved(token, instruction);
        Ok(Self(SupportRecoveryExternalWaitAuthorityKind::Evidence {
            action_id,
            external_action,
            postcondition,
        }))
    }

    fn action_id(&self) -> &UnicaId {
        match &self.0 {
            SupportRecoveryExternalWaitAuthorityKind::Corrective { action_id, .. }
            | SupportRecoveryExternalWaitAuthorityKind::ReleaseLocks { action_id, .. }
            | SupportRecoveryExternalWaitAuthorityKind::CleanWorkingInfobase {
                action_id, ..
            }
            | SupportRecoveryExternalWaitAuthorityKind::CloseReservedOriginal {
                action_id, ..
            }
            | SupportRecoveryExternalWaitAuthorityKind::Conflict { action_id, .. }
            | SupportRecoveryExternalWaitAuthorityKind::Evidence { action_id, .. } => action_id,
        }
    }

    fn validate_plan_binding(
        &self,
        support_action_id: &UnicaId,
        manual_target_mode: ManualSupportTargetMode,
        closure_plan: Option<&ManualWorkingInfobaseClosurePlan>,
    ) -> Result<(), RecoveryContractError> {
        let external_action = match &self.0 {
            SupportRecoveryExternalWaitAuthorityKind::Corrective {
                external_action, ..
            }
            | SupportRecoveryExternalWaitAuthorityKind::ReleaseLocks {
                external_action, ..
            }
            | SupportRecoveryExternalWaitAuthorityKind::CleanWorkingInfobase {
                external_action,
                ..
            }
            | SupportRecoveryExternalWaitAuthorityKind::CloseReservedOriginal {
                external_action,
                ..
            }
            | SupportRecoveryExternalWaitAuthorityKind::Conflict {
                external_action, ..
            }
            | SupportRecoveryExternalWaitAuthorityKind::Evidence {
                external_action, ..
            } => external_action,
        };
        let valid = match external_action.as_ref() {
            SupportRecoveryExternalActionRef::Corrective(instruction) => {
                instruction.support_action_id() == support_action_id
                    && instruction.manual_target_mode() == manual_target_mode
            }
            SupportRecoveryExternalActionRef::ReleaseLocks(_) => true,
            SupportRecoveryExternalActionRef::CleanWorkingInfobase(instruction) => {
                manual_target_mode == ManualSupportTargetMode::SeparateWorkingInfobase
                    && closure_plan.is_some_and(|plan| {
                        instruction.working_infobase_identity() == plan.working_infobase_identity()
                            && instruction.closure_plan_digest() == plan.plan_digest()
                            && instruction.exclusive_lease_capability_id()
                                == plan.exclusive_lease_capability_id()
                    })
            }
            SupportRecoveryExternalActionRef::CloseReservedOriginal(_) => {
                manual_target_mode == ManualSupportTargetMode::ReservedOriginal
                    && closure_plan.is_none()
            }
            SupportRecoveryExternalActionRef::Conflict(_)
            | SupportRecoveryExternalActionRef::Evidence(_) => true,
        };
        valid.then_some(()).ok_or(RecoveryContractError(
            "support recovery external wait differs from its action, mode, or closure plan",
        ))
    }

    fn into_parts(
        self,
        support_action_id: &UnicaId,
    ) -> Result<(RecoveryAction, SupportRecoveryExternalAction), RecoveryContractError> {
        match self.0 {
            SupportRecoveryExternalWaitAuthorityKind::Corrective {
                action_id,
                external_action,
                postcondition,
            } => {
                let SupportRecoveryExternalActionRef::Corrective(instruction) =
                    external_action.as_ref()
                else {
                    return Err(RecoveryContractError(
                        "corrective wait lost its corrective instruction",
                    ));
                };
                if instruction.support_action_id() != support_action_id
                    || instruction.desired_support_graph_digest()
                        != &postcondition.expected_support_graph_digest
                {
                    return Err(RecoveryContractError(
                        "corrective wait differs from its support action or desired graph",
                    ));
                }
                let (expected_observations, expected_postcondition_digest) =
                    expected_postcondition(vec![RecoveryExpectedObservation::new(
                        RecoveryObservationKind::SupportGraph,
                        RecoverySubjectRef::configuration_root(),
                        postcondition.expected_support_graph_digest,
                    )])?;
                let action = RecoveryAction::from_record(
                    RecoveryActionDigestRecordKind::AwaitExternalSupportCorrection(
                        AwaitExternalSupportCorrectionActionDigestRecord {
                            action_kind: AwaitExternalSupportCorrectionActionKind::Value,
                            action_id,
                            support_action_id: support_action_id.clone(),
                            corrective_instruction_digest: instruction
                                .corrective_instruction_digest()
                                .clone(),
                            expected_observations,
                            expected_postcondition_digest,
                        },
                    ),
                )?;
                Ok((action, external_action))
            }
            SupportRecoveryExternalWaitAuthorityKind::ReleaseLocks {
                action_id,
                external_action,
                postcondition,
            } => {
                let SupportRecoveryExternalActionRef::ReleaseLocks(instruction) =
                    external_action.as_ref()
                else {
                    return Err(RecoveryContractError(
                        "lock-release wait lost its release instruction",
                    ));
                };
                let (expected_observations, expected_postcondition_digest) =
                    expected_postcondition(vec![RecoveryExpectedObservation::new(
                        RecoveryObservationKind::LockOwnership,
                        postcondition.subject.clone(),
                        postcondition.expected_unlocked_digest,
                    )])?;
                let action = RecoveryAction::from_record(
                    RecoveryActionDigestRecordKind::AwaitExternalLockRelease(
                        AwaitExternalLockReleaseActionDigestRecord {
                            action_kind: AwaitExternalLockReleaseActionKind::Value,
                            action_id,
                            lock_instruction_digest: instruction.lock_instruction_digest().clone(),
                            subjects: RecoverySubjects::new(vec![postcondition.subject])?,
                            expected_observations,
                            expected_postcondition_digest,
                        },
                    ),
                )?;
                Ok((action, external_action))
            }
            SupportRecoveryExternalWaitAuthorityKind::CleanWorkingInfobase {
                action_id,
                external_action,
                postcondition,
            } => {
                let SupportRecoveryExternalActionRef::CleanWorkingInfobase(instruction) =
                    external_action.as_ref()
                else {
                    return Err(RecoveryContractError(
                        "working-infobase wait lost its cleanup instruction",
                    ));
                };
                if instruction.working_infobase_identity()
                    != &postcondition.working_infobase_identity
                {
                    return Err(RecoveryContractError(
                        "working-infobase wait identity differs from its instruction",
                    ));
                }
                let (expected_observations, expected_postcondition_digest) =
                    expected_postcondition(vec![RecoveryExpectedObservation::new(
                        RecoveryObservationKind::WorkingInfobaseLease,
                        RecoverySubjectRef::external_working_infobase(
                            postcondition.working_infobase_identity.clone(),
                        ),
                        postcondition.expected_available_lease_digest,
                    )])?;
                let action = RecoveryAction::from_record(
                    RecoveryActionDigestRecordKind::AwaitManualWorkingInfobaseClosure(
                        AwaitManualWorkingInfobaseClosureActionDigestRecord {
                            action_kind: AwaitManualWorkingInfobaseClosureActionKind::Value,
                            action_id,
                            working_infobase_identity: instruction
                                .working_infobase_identity()
                                .clone(),
                            closure_plan_digest: instruction.closure_plan_digest().clone(),
                            exclusive_lease_capability_id: instruction
                                .exclusive_lease_capability_id()
                                .clone(),
                            expected_observations,
                            expected_postcondition_digest,
                        },
                    ),
                )?;
                Ok((action, external_action))
            }
            SupportRecoveryExternalWaitAuthorityKind::CloseReservedOriginal {
                action_id,
                external_action,
                postcondition,
            } => {
                let SupportRecoveryExternalActionRef::CloseReservedOriginal(instruction) =
                    external_action.as_ref()
                else {
                    return Err(RecoveryContractError(
                        "reserved-original wait lost its closure instruction",
                    ));
                };
                if instruction.reserved_original_identity_digest()
                    != &postcondition.reserved_original_identity_digest
                {
                    return Err(RecoveryContractError(
                        "reserved-original wait identity differs from its instruction",
                    ));
                }
                let (expected_observations, expected_postcondition_digest) =
                    expected_postcondition(vec![RecoveryExpectedObservation::new(
                        RecoveryObservationKind::ReservedOriginalLease,
                        RecoverySubjectRef::reserved_original_infobase(
                            postcondition.reserved_original_identity_digest.clone(),
                        ),
                        postcondition.expected_available_lease_digest,
                    )])?;
                let action = RecoveryAction::from_record(
                    RecoveryActionDigestRecordKind::AwaitReservedOriginalClosure(
                        AwaitReservedOriginalClosureActionDigestRecord {
                            action_kind: AwaitReservedOriginalClosureActionKind::Value,
                            action_id,
                            reserved_original_identity_digest: instruction
                                .reserved_original_identity_digest()
                                .clone(),
                            exclusive_lease_capability_id: instruction
                                .exclusive_lease_capability_id()
                                .clone(),
                            expected_observations,
                            expected_postcondition_digest,
                        },
                    ),
                )?;
                Ok((action, external_action))
            }
            SupportRecoveryExternalWaitAuthorityKind::Conflict {
                action_id,
                external_action,
                postcondition,
            } => {
                let SupportRecoveryExternalActionRef::Conflict(instruction) =
                    external_action.as_ref()
                else {
                    return Err(RecoveryContractError(
                        "support-conflict wait lost its conflict instruction",
                    ));
                };
                if instruction.required_final_baseline_digest()
                    != &postcondition.required_final_baseline_digest
                {
                    return Err(RecoveryContractError(
                        "support-conflict wait differs from its required final baseline",
                    ));
                }
                let (expected_observations, expected_postcondition_digest) =
                    expected_postcondition(vec![RecoveryExpectedObservation::new(
                        RecoveryObservationKind::SupportGraph,
                        RecoverySubjectRef::configuration_root(),
                        postcondition.required_final_baseline_digest,
                    )])?;
                let action = RecoveryAction::from_record(
                    RecoveryActionDigestRecordKind::AwaitExternalSupportConflictResolution(
                        AwaitExternalSupportConflictResolutionActionDigestRecord {
                            action_kind: AwaitExternalSupportConflictResolutionActionKind::Value,
                            action_id,
                            support_action_id: support_action_id.clone(),
                            support_conflict_instruction_digest: instruction
                                .support_conflict_instruction_digest()
                                .clone(),
                            expected_observations,
                            expected_postcondition_digest,
                        },
                    ),
                )?;
                Ok((action, external_action))
            }
            SupportRecoveryExternalWaitAuthorityKind::Evidence {
                action_id,
                external_action,
                postcondition,
            } => {
                let SupportRecoveryExternalActionRef::Evidence(instruction) =
                    external_action.as_ref()
                else {
                    return Err(RecoveryContractError(
                        "support-evidence wait lost its evidence instruction",
                    ));
                };
                let (expected_observations, expected_postcondition_digest) =
                    expected_postcondition(vec![RecoveryExpectedObservation::new(
                        RecoveryObservationKind::ArtifactPresence,
                        RecoverySubjectRef::registered(postcondition.evidence_artifact_id),
                        postcondition.expected_evidence_digest,
                    )])?;
                let action = RecoveryAction::from_record(
                    RecoveryActionDigestRecordKind::AwaitSupportRecoveryEvidence(
                        AwaitSupportRecoveryEvidenceActionDigestRecord {
                            action_kind: AwaitSupportRecoveryEvidenceActionKind::Value,
                            action_id,
                            support_action_id: support_action_id.clone(),
                            support_evidence_instruction_digest: instruction
                                .support_evidence_instruction_digest()
                                .clone(),
                            expected_observations,
                            expected_postcondition_digest,
                        },
                    ),
                )?;
                Ok((action, external_action))
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn from_external_action_test_only(
        action_id: UnicaId,
        external_action: SupportRecoveryExternalAction,
        expected_digest: Sha256Digest,
    ) -> Result<Self, RecoveryContractError> {
        Ok(match external_action.as_ref() {
            SupportRecoveryExternalActionRef::Corrective(instruction) => {
                Self(SupportRecoveryExternalWaitAuthorityKind::Corrective {
                    action_id,
                    postcondition: AwaitExternalSupportCorrectionPostconditionAuthority {
                        expected_support_graph_digest: instruction
                            .desired_support_graph_digest()
                            .clone(),
                    },
                    external_action,
                })
            }
            SupportRecoveryExternalActionRef::ReleaseLocks(_) => {
                Self(SupportRecoveryExternalWaitAuthorityKind::ReleaseLocks {
                    action_id,
                    postcondition: SupportRecoveryLockReleasePostconditionObservation {
                        blocked_guard_proof_digest: expected_digest.clone(),
                        subject: RecoverySubjectRef::configuration_root(),
                        expected_unlocked_digest: expected_digest,
                    },
                    external_action,
                })
            }
            SupportRecoveryExternalActionRef::CleanWorkingInfobase(instruction) => Self(
                SupportRecoveryExternalWaitAuthorityKind::CleanWorkingInfobase {
                    action_id,
                    postcondition: ManualWorkingInfobaseAvailablePostconditionObservation {
                        closure_plan_digest: instruction.closure_plan_digest().clone(),
                        working_infobase_identity: instruction.working_infobase_identity().clone(),
                        expected_available_lease_digest: expected_digest,
                    },
                    external_action,
                },
            ),
            SupportRecoveryExternalActionRef::CloseReservedOriginal(instruction) => Self(
                SupportRecoveryExternalWaitAuthorityKind::CloseReservedOriginal {
                    action_id,
                    postcondition: ReservedOriginalAvailablePostconditionObservation {
                        reserved_original_identity_digest: instruction
                            .reserved_original_identity_digest()
                            .clone(),
                        exclusive_lease_capability_id: instruction
                            .exclusive_lease_capability_id()
                            .clone(),
                        expected_available_lease_digest: expected_digest,
                    },
                    external_action,
                },
            ),
            SupportRecoveryExternalActionRef::Conflict(instruction) => {
                Self(SupportRecoveryExternalWaitAuthorityKind::Conflict {
                    action_id,
                    postcondition: AwaitExternalSupportConflictPostconditionAuthority {
                        required_final_baseline_digest: instruction
                            .required_final_baseline_digest()
                            .clone(),
                    },
                    external_action,
                })
            }
            SupportRecoveryExternalActionRef::Evidence(instruction) => {
                Self(SupportRecoveryExternalWaitAuthorityKind::Evidence {
                    action_id,
                    postcondition: SupportRecoveryEvidencePostconditionObservation {
                        support_evidence_instruction_digest: instruction
                            .support_evidence_instruction_digest()
                            .clone(),
                        evidence_artifact_id: UnicaId::parse(
                            "ffffffff-ffff-4fff-8fff-ffffffffffff",
                        )
                        .expect("test evidence artifact ID is valid"),
                        expected_evidence_digest: expected_digest,
                    },
                    external_action,
                })
            }
        })
    }

    #[cfg(test)]
    pub(crate) fn into_recovery_action_test_only(
        self,
        support_action_id: UnicaId,
    ) -> Result<RecoveryAction, RecoveryContractError> {
        self.into_parts(&support_action_id)
            .map(|(action, _)| action)
    }
}

/// Exact ID catalog for the fixed history/[external wait]/finalization support
/// recovery sequence. Construction enforces the optional wait and uniqueness
/// before any digest-bearing action is minted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SupportRecoveryActionCatalogAuthority {
    history_action_id: UnicaId,
    external_wait: Option<SupportRecoveryExternalWaitAuthority>,
    finalization_action_id: UnicaId,
}

impl SupportRecoveryActionCatalogAuthority {
    fn new(
        history_action_id: UnicaId,
        external_wait: Option<SupportRecoveryExternalWaitAuthority>,
        finalization_action_id: UnicaId,
    ) -> Result<Self, RecoveryContractError> {
        if history_action_id == finalization_action_id
            || external_wait.as_ref().is_some_and(|wait| {
                wait.action_id() == &history_action_id
                    || wait.action_id() == &finalization_action_id
            })
        {
            return Err(RecoveryContractError(
                "support recovery action IDs must be pairwise distinct",
            ));
        }
        Ok(Self {
            history_action_id,
            external_wait,
            finalization_action_id,
        })
    }

    pub(crate) fn without_external_from_approved(
        _token: &SupportRecoveryAuthorityToken,
        history_action_id: UnicaId,
        finalization_action_id: UnicaId,
    ) -> Result<Self, RecoveryContractError> {
        Self::new(history_action_id, None, finalization_action_id)
    }

    pub(crate) fn with_external_from_approved(
        _token: &SupportRecoveryAuthorityToken,
        history_action_id: UnicaId,
        external_wait: SupportRecoveryExternalWaitAuthority,
        finalization_action_id: UnicaId,
    ) -> Result<Self, RecoveryContractError> {
        Self::new(
            history_action_id,
            Some(external_wait),
            finalization_action_id,
        )
    }

    fn into_parts(
        self,
    ) -> (
        UnicaId,
        Option<SupportRecoveryExternalWaitAuthority>,
        UnicaId,
    ) {
        (
            self.history_action_id,
            self.external_wait,
            self.finalization_action_id,
        )
    }

    #[cfg(test)]
    fn test_only(
        history_action_id: UnicaId,
        external_wait: Option<SupportRecoveryExternalWaitAuthority>,
        finalization_action_id: UnicaId,
    ) -> Result<Self, RecoveryContractError> {
        Self::new(history_action_id, external_wait, finalization_action_id)
    }
}

/// A matched absence observation that has been checked against one exact
/// `finishCleanup` action. This is the only production authority from which a
/// cleanup receipt may project an owned-target absence.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct FinishCleanupAbsenceObservation {
    owned_target: OwnedTargetLocator,
    observation_digest: Sha256Digest,
}

impl FinishCleanupAbsenceObservation {
    pub(crate) const fn owned_target(&self) -> &OwnedTargetLocator {
        &self.owned_target
    }

    pub(crate) const fn observation_digest(&self) -> &Sha256Digest {
        &self.observation_digest
    }
}

impl RecoveryAction {
    fn match_finish_cleanup_absence(
        &self,
        owned_target: &OwnedTargetLocator,
        observation: &RecoveryObservation,
    ) -> Result<FinishCleanupAbsenceObservation, RecoveryContractError> {
        let RecoveryActionKindWire::FinishCleanup(finish) = &self.0 else {
            return Err(RecoveryContractError(
                "cleanup absence authority requires a finishCleanup action",
            ));
        };
        let Some(index) = finish
            .owned_targets
            .0
            .iter()
            .position(|candidate| candidate == owned_target)
        else {
            return Err(RecoveryContractError(
                "cleanup absence target is outside the finish action",
            ));
        };
        let expected = &finish.expected_observations.as_slice()[index];
        let expected_absent_digest = expected_absent_owned_target_digest(
            &finish.archive_id,
            &finish.action_id,
            owned_target,
        )?;
        let RecoveryObservationKindWire::Matched(matched) = &observation.0 else {
            return Err(RecoveryContractError(
                "cleanup absence authority requires a matched observation",
            ));
        };
        if expected.observation_kind != RecoveryObservationKind::OwnedTargetAbsence
            || !expected.subject.is_owned_role(owned_target)
            || expected.expected_digest != expected_absent_digest
            || matched.observation_kind != expected.observation_kind
            || matched.subject != expected.subject
            || matched.expected_digest != expected.expected_digest
            || matched.observed_digest != expected.expected_digest
        {
            return Err(RecoveryContractError(
                "cleanup absence observation is not the exact fresh finish projection",
            ));
        }
        Ok(FinishCleanupAbsenceObservation {
            owned_target: owned_target.clone(),
            observation_digest: matched.observation_digest.clone(),
        })
    }
}

/// Exact full-set absence authority minted by one cleanup recovery plan. The
/// set is non-`Clone` and retains the recovery/action lineage that the wire
/// observations do not carry, closing cross-attempt receipt splicing.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct FinishCleanupAbsenceObservations {
    prior_operation_id: OperationId,
    recovery_digest: Sha256Digest,
    archive_id: UnicaId,
    finish_action_id: UnicaId,
    finish_action_digest: Sha256Digest,
    observations: Vec<FinishCleanupAbsenceObservation>,
}

impl FinishCleanupAbsenceObservations {
    pub(crate) const fn prior_operation_id(&self) -> &OperationId {
        &self.prior_operation_id
    }

    pub(crate) const fn recovery_digest(&self) -> &Sha256Digest {
        &self.recovery_digest
    }

    pub(crate) const fn archive_id(&self) -> &UnicaId {
        &self.archive_id
    }

    pub(crate) const fn finish_action_id(&self) -> &UnicaId {
        &self.finish_action_id
    }

    pub(crate) const fn finish_action_digest(&self) -> &Sha256Digest {
        &self.finish_action_digest
    }

    pub(crate) fn as_slice(&self) -> &[FinishCleanupAbsenceObservation] {
        &self.observations
    }

    pub(crate) fn into_observations(self) -> Vec<FinishCleanupAbsenceObservation> {
        self.observations
    }
}

#[derive(Debug, Clone, Copy)]
enum RecoveryObservationSubjectSchema {
    Any,
    Registered,
    ConfigurationRoot,
    OwnedRole,
    ExternalWorkingInfobase,
    ReservedOriginalInfobase,
    RetentionLease,
    RootOrMetadataObject,
    OriginalTarget,
}

fn recovery_observation_subject_schema(
    generator: &mut SchemaGenerator,
    shape: RecoveryObservationSubjectSchema,
) -> Schema {
    match shape {
        RecoveryObservationSubjectSchema::Any => generator.subschema_for::<RecoverySubjectRef>(),
        RecoveryObservationSubjectSchema::Registered => {
            generator.subschema_for::<RegisteredRecoverySubject>()
        }
        RecoveryObservationSubjectSchema::ConfigurationRoot => {
            generator.subschema_for::<ConfigurationRootRecoverySubject>()
        }
        RecoveryObservationSubjectSchema::OwnedRole => {
            generator.subschema_for::<OwnedRoleRecoverySubject>()
        }
        RecoveryObservationSubjectSchema::ExternalWorkingInfobase => {
            generator.subschema_for::<ExternalWorkingInfobaseRecoverySubject>()
        }
        RecoveryObservationSubjectSchema::ReservedOriginalInfobase => {
            generator.subschema_for::<ReservedOriginalInfobaseRecoverySubject>()
        }
        RecoveryObservationSubjectSchema::RetentionLease => {
            generator.subschema_for::<RetentionLeaseRecoverySubject>()
        }
        RecoveryObservationSubjectSchema::RootOrMetadataObject => one_of_schema(vec![
            generator.subschema_for::<ConfigurationRootRecoverySubject>(),
            generator.subschema_for::<MetadataObjectRecoverySubject>(),
        ]),
        RecoveryObservationSubjectSchema::OriginalTarget => one_of_schema(vec![
            generator.subschema_for::<RegisteredRecoverySubject>(),
            generator.subschema_for::<MetadataObjectRecoverySubject>(),
            generator.subschema_for::<ReservedOriginalInfobaseRecoverySubject>(),
        ]),
    }
}

fn recovery_expected_observation_schema(
    generator: &mut SchemaGenerator,
    kind: &'static str,
    subject: RecoveryObservationSubjectSchema,
) -> Schema {
    let mut schema = serde_json::to_value(RecoveryExpectedObservation::json_schema(generator))
        .expect("recovery expected-observation schema must serialize");
    let properties = schema
        .get_mut("properties")
        .and_then(serde_json::Value::as_object_mut)
        .expect("derived recovery expected-observation schema must expose properties");
    properties.insert(
        "observationKind".to_owned(),
        serde_json::to_value(json_schema!({ "type": "string", "const": kind }))
            .expect("literal observation-kind schema must serialize"),
    );
    properties.insert(
        "subject".to_owned(),
        serde_json::to_value(recovery_observation_subject_schema(generator, subject))
            .expect("recovery observation subject schema must serialize"),
    );
    Schema::try_from(schema).expect("refined recovery expected-observation schema must be valid")
}

fn exact_recovery_observation_sequence_schema(
    generator: &mut SchemaGenerator,
    sequence: &[(&'static str, RecoveryObservationSubjectSchema)],
) -> Schema {
    let items = sequence
        .iter()
        .map(|(kind, subject)| recovery_expected_observation_schema(generator, kind, *subject))
        .collect::<Vec<_>>();
    let length = items.len();
    json_schema!({
        "type": "array",
        "prefixItems": items,
        "items": false,
        "minItems": length,
        "maxItems": length,
        "uniqueItems": true
    })
}

fn allowed_recovery_observations_schema(
    generator: &mut SchemaGenerator,
    allowed: &[(&'static str, RecoveryObservationSubjectSchema)],
    min_items: usize,
) -> Schema {
    let variants = allowed
        .iter()
        .map(|(kind, subject)| recovery_expected_observation_schema(generator, kind, *subject))
        .collect::<Vec<_>>();
    let item = if variants.len() == 1 {
        variants.into_iter().next().unwrap()
    } else {
        one_of_schema(variants)
    };
    json_schema!({
        "type": "array",
        "items": item,
        "minItems": min_items,
        "maxItems": MAX_RECOVERY_ITEMS,
        "uniqueItems": true
    })
}

fn support_history_observations_schema(generator: &mut SchemaGenerator) -> Schema {
    exact_recovery_observation_sequence_schema(
        generator,
        &[(
            "repositoryAnchor",
            RecoveryObservationSubjectSchema::Registered,
        )],
    )
}

fn recovery_action_leaf_schema<T: JsonSchema>(
    generator: &mut SchemaGenerator,
    expected_observations: impl FnOnce(&mut SchemaGenerator) -> Schema,
) -> Schema {
    let expected_observations = expected_observations(generator);
    let mut schema = serde_json::to_value(T::json_schema(generator))
        .expect("recovery action leaf schema must serialize");
    schema
        .get_mut("properties")
        .and_then(serde_json::Value::as_object_mut)
        .expect("derived recovery action leaf schema must expose properties")
        .insert(
            "expectedObservations".to_owned(),
            serde_json::to_value(expected_observations)
                .expect("expected-observations schema must serialize"),
        );
    Schema::try_from(schema).expect("refined recovery action leaf schema must be valid")
}

impl JsonSchema for RecoveryAction {
    fn schema_name() -> Cow<'static, str> {
        "RecoveryAction".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        use RecoveryObservationSubjectSchema as Subject;

        let exact =
            |generator: &mut SchemaGenerator,
             sequence: &[(&'static str, RecoveryObservationSubjectSchema)]| {
                exact_recovery_observation_sequence_schema(generator, sequence)
            };
        let allowed =
            |generator: &mut SchemaGenerator,
             observations: &[(&'static str, RecoveryObservationSubjectSchema)]| {
                allowed_recovery_observations_schema(generator, observations, 1)
            };
        let prearm_terminal =
            |generator: &mut SchemaGenerator,
             mode_kind: &'static str,
             mode_subject: RecoveryObservationSubjectSchema| {
                exact(
                    generator,
                    &[
                        ("supportGraph", Subject::Any),
                        ("supportActionAuthorization", Subject::Registered),
                        ("objectFingerprint", Subject::OriginalTarget),
                        ("lockOwnership", Subject::ConfigurationRoot),
                        (mode_kind, mode_subject),
                    ],
                )
            };
        let recheck = |generator: &mut SchemaGenerator,
                       mode_kind: &'static str,
                       mode_subject: RecoveryObservationSubjectSchema| {
            exact(
                generator,
                &[
                    ("supportGraph", Subject::Any),
                    ("supportActionAuthorization", Subject::Registered),
                    ("objectFingerprint", Subject::OriginalTarget),
                    ("lockOwnership", Subject::ConfigurationRoot),
                    (mode_kind, mode_subject),
                    ("finalizationPolicy", Subject::Registered),
                ],
            )
        };

        one_of_schema(vec![
            recovery_action_leaf_schema::<ReleaseOwnedLocksAction>(generator, |generator| {
                allowed(generator, &[("lockOwnership", Subject::Any)])
            }),
            recovery_action_leaf_schema::<RestoreOriginalAction>(generator, |generator| {
                exact(
                    generator,
                    &[("objectFingerprint", Subject::ReservedOriginalInfobase)],
                )
            }),
            recovery_action_leaf_schema::<RestoreTaskCheckpointAction>(generator, |generator| {
                exact(generator, &[("taskFingerprint", Subject::Registered)])
            }),
            recovery_action_leaf_schema::<RecreateTaskInfobaseAction>(generator, |generator| {
                exact(generator, &[("taskFingerprint", Subject::Registered)])
            }),
            recovery_action_leaf_schema::<VerifyTaskFingerprintAction>(generator, |generator| {
                exact(generator, &[("taskFingerprint", Subject::Registered)])
            }),
            recovery_action_leaf_schema::<ObserveCommitAction>(generator, |generator| {
                exact(generator, &[("repositoryVersion", Subject::Registered)])
            }),
            recovery_action_leaf_schema::<ObservePreArmCancellationOutcomeAction>(
                generator,
                |generator| {
                    one_of_schema(vec![
                        prearm_terminal(
                            generator,
                            "workingInfobaseLease",
                            Subject::ExternalWorkingInfobase,
                        ),
                        prearm_terminal(
                            generator,
                            "reservedOriginalLease",
                            Subject::ReservedOriginalInfobase,
                        ),
                    ])
                },
            ),
            recovery_action_leaf_schema::<AcquirePreArmRootGuardAction>(generator, |generator| {
                exact(generator, &[("lockOwnership", Subject::ConfigurationRoot)])
            }),
            one_of_schema(vec![
                recovery_action_leaf_schema::<AcquirePreArmReservedOriginalModeLeaseAction>(
                    generator,
                    |generator| {
                        exact(
                            generator,
                            &[("reservedOriginalLease", Subject::ReservedOriginalInfobase)],
                        )
                    },
                ),
                recovery_action_leaf_schema::<AcquirePreArmWorkingInfobaseModeLeaseAction>(
                    generator,
                    |generator| {
                        exact(
                            generator,
                            &[("workingInfobaseLease", Subject::ExternalWorkingInfobase)],
                        )
                    },
                ),
            ]),
            recovery_action_leaf_schema::<RecheckPreArmCancellationFinalizationAction>(
                generator,
                |generator| {
                    one_of_schema(vec![
                        recheck(
                            generator,
                            "workingInfobaseLease",
                            Subject::ExternalWorkingInfobase,
                        ),
                        recheck(
                            generator,
                            "reservedOriginalLease",
                            Subject::ReservedOriginalInfobase,
                        ),
                    ])
                },
            ),
            recovery_action_leaf_schema::<ApplyPreArmCancellationSelectiveUpdateAction>(
                generator,
                |generator| {
                    allowed(
                        generator,
                        &[("objectFingerprint", Subject::RootOrMetadataObject)],
                    )
                },
            ),
            recovery_action_leaf_schema::<PersistPreArmSupportCancellationAction>(
                generator,
                |generator| {
                    exact(
                        generator,
                        &[("supportActionAuthorization", Subject::Registered)],
                    )
                },
            ),
            one_of_schema(vec![
                recovery_action_leaf_schema::<ReleasePreArmReservedOriginalModeLeaseAction>(
                    generator,
                    |generator| {
                        exact(
                            generator,
                            &[("reservedOriginalLease", Subject::ReservedOriginalInfobase)],
                        )
                    },
                ),
                recovery_action_leaf_schema::<ReleasePreArmWorkingInfobaseModeLeaseAction>(
                    generator,
                    |generator| {
                        exact(
                            generator,
                            &[("workingInfobaseLease", Subject::ExternalWorkingInfobase)],
                        )
                    },
                ),
            ]),
            recovery_action_leaf_schema::<ReleasePreArmRootGuardAction>(generator, |generator| {
                exact(generator, &[("lockOwnership", Subject::ConfigurationRoot)])
            }),
            recovery_action_leaf_schema::<FinishPreArmCancellationRecoveryAction>(
                generator,
                |generator| {
                    one_of_schema(vec![
                        prearm_terminal(
                            generator,
                            "workingInfobaseLease",
                            Subject::ExternalWorkingInfobase,
                        ),
                        prearm_terminal(
                            generator,
                            "reservedOriginalLease",
                            Subject::ReservedOriginalInfobase,
                        ),
                    ])
                },
            ),
            recovery_action_leaf_schema::<QuarantineArtifactAction>(generator, |generator| {
                exact(
                    generator,
                    &[
                        ("artifactPresence", Subject::Registered),
                        ("quarantinePresence", Subject::Registered),
                    ],
                )
            }),
            recovery_action_leaf_schema::<ObserveSupportPrerequisiteHistoryAction>(
                generator,
                support_history_observations_schema,
            ),
            recovery_action_leaf_schema::<UpdateOriginalSelectedTargetsAction>(
                generator,
                |generator| {
                    allowed(
                        generator,
                        &[("objectFingerprint", Subject::RootOrMetadataObject)],
                    )
                },
            ),
            recovery_action_leaf_schema::<ObserveWorkingInfobaseLeaseAction>(
                generator,
                |generator| {
                    exact(
                        generator,
                        &[("workingInfobaseLease", Subject::ExternalWorkingInfobase)],
                    )
                },
            ),
            recovery_action_leaf_schema::<ReleaseWorkingInfobaseLeaseAction>(
                generator,
                |generator| {
                    exact(
                        generator,
                        &[("workingInfobaseLease", Subject::ExternalWorkingInfobase)],
                    )
                },
            ),
            recovery_action_leaf_schema::<ObserveReservedOriginalLeaseAction>(
                generator,
                |generator| {
                    exact(
                        generator,
                        &[("reservedOriginalLease", Subject::ReservedOriginalInfobase)],
                    )
                },
            ),
            recovery_action_leaf_schema::<ReleaseReservedOriginalLeaseAction>(
                generator,
                |generator| {
                    exact(
                        generator,
                        &[("reservedOriginalLease", Subject::ReservedOriginalInfobase)],
                    )
                },
            ),
            recovery_action_leaf_schema::<ObserveRetentionLeaseAction>(generator, |generator| {
                exact(generator, &[("retentionLease", Subject::RetentionLease)])
            }),
            recovery_action_leaf_schema::<ObserveArchiveStagingAction>(generator, |generator| {
                exact(
                    generator,
                    &[("archiveStagingPresence", Subject::Registered)],
                )
            }),
            recovery_action_leaf_schema::<ReleaseRetentionLeaseAction>(generator, |generator| {
                exact(generator, &[("retentionLease", Subject::RetentionLease)])
            }),
            recovery_action_leaf_schema::<AwaitExternalSupportCorrectionAction>(
                generator,
                |generator| {
                    allowed(
                        generator,
                        &[
                            ("repositoryVersion", Subject::Registered),
                            ("supportGraph", Subject::Any),
                            ("objectFingerprint", Subject::OriginalTarget),
                        ],
                    )
                },
            ),
            recovery_action_leaf_schema::<AwaitExternalLockReleaseAction>(generator, |generator| {
                allowed(generator, &[("lockOwnership", Subject::Any)])
            }),
            recovery_action_leaf_schema::<AwaitManualWorkingInfobaseClosureAction>(
                generator,
                |generator| {
                    exact(
                        generator,
                        &[("workingInfobaseLease", Subject::ExternalWorkingInfobase)],
                    )
                },
            ),
            recovery_action_leaf_schema::<AwaitReservedOriginalClosureAction>(
                generator,
                |generator| {
                    exact(
                        generator,
                        &[("reservedOriginalLease", Subject::ReservedOriginalInfobase)],
                    )
                },
            ),
            recovery_action_leaf_schema::<AwaitExternalSupportConflictResolutionAction>(
                generator,
                |generator| {
                    allowed(
                        generator,
                        &[
                            ("repositoryVersion", Subject::Registered),
                            ("supportGraph", Subject::Any),
                        ],
                    )
                },
            ),
            recovery_action_leaf_schema::<AwaitSupportRecoveryEvidenceAction>(
                generator,
                |generator| {
                    allowed(
                        generator,
                        &[
                            ("repositoryAnchor", Subject::Registered),
                            ("repositoryVersion", Subject::Registered),
                            ("supportGraph", Subject::Any),
                            ("lockOwnership", Subject::Any),
                            ("workingInfobaseLease", Subject::ExternalWorkingInfobase),
                            ("reservedOriginalLease", Subject::ReservedOriginalInfobase),
                            ("artifactPresence", Subject::Registered),
                        ],
                    )
                },
            ),
            recovery_action_leaf_schema::<FinalizeSupportPrerequisiteRecoveryAction>(
                generator,
                |generator| {
                    exact(
                        generator,
                        &[
                            ("supportGraph", Subject::Any),
                            ("supportActionAuthorization", Subject::Registered),
                        ],
                    )
                },
            ),
            recovery_action_leaf_schema::<ResumeQuarantineAction>(generator, |generator| {
                exact(
                    generator,
                    &[
                        ("artifactPresence", Subject::Registered),
                        ("quarantinePresence", Subject::Registered),
                    ],
                )
            }),
            recovery_action_leaf_schema::<ResumeOwnedTargetQuarantineAction>(
                generator,
                |generator| exact(generator, &[("quarantinePresence", Subject::OwnedRole)]),
            ),
            recovery_action_leaf_schema::<FinishArchiveAction>(generator, |generator| {
                exact(generator, &[("archivePresence", Subject::Registered)])
            }),
            recovery_action_leaf_schema::<FinishCleanupAction>(generator, |generator| {
                allowed(generator, &[("ownedTargetAbsence", Subject::OwnedRole)])
            }),
        ])
    }
}

impl RecoveryAction {
    fn expected_release_receipt_id(&self) -> Option<&UnicaId> {
        match &self.0 {
            RecoveryActionKindWire::ReleaseWorkingInfobaseLease(value) => {
                Some(&value.expected_release_receipt_id)
            }
            RecoveryActionKindWire::ReleaseReservedOriginalLease(value) => {
                Some(&value.expected_release_receipt_id)
            }
            RecoveryActionKindWire::ReleaseRetentionLease(value) => {
                Some(&value.expected_release_receipt_id)
            }
            _ => None,
        }
    }
}

fn validate_common_expected_observations(
    observations: &RecoveryExpectedObservations,
    expected_postcondition_digest: &Sha256Digest,
) -> Result<(), RecoveryContractError> {
    let recomputed = contract_digest(
        &RecoveryExpectedPostconditionDigestRecord(observations.clone()),
        "recovery expected postcondition digest failed",
    )?;
    if &recomputed != expected_postcondition_digest {
        return Err(RecoveryContractError(
            "recovery expected postcondition digest mismatch",
        ));
    }
    Ok(())
}

fn require_exact_single_observation(
    observations: &RecoveryExpectedObservations,
    kind: RecoveryObservationKind,
    subject_matches: impl FnOnce(&RecoverySubjectRef) -> bool,
    digest: Option<&Sha256Digest>,
) -> Result<(), RecoveryContractError> {
    let [observation] = observations.as_slice() else {
        return Err(RecoveryContractError(
            "recovery action requires one exact expected observation",
        ));
    };
    if observation.observation_kind != kind
        || !subject_matches(&observation.subject)
        || digest.is_some_and(|digest| &observation.expected_digest != digest)
    {
        return Err(RecoveryContractError(
            "recovery action expected-observation projection mismatch",
        ));
    }
    Ok(())
}

fn require_only_observation_kinds(
    observations: &RecoveryExpectedObservations,
    allowed: &[RecoveryObservationKind],
) -> Result<(), RecoveryContractError> {
    if observations
        .as_slice()
        .iter()
        .any(|value| !allowed.contains(&value.observation_kind))
    {
        return Err(RecoveryContractError(
            "recovery action contains a foreign expected-observation kind",
        ));
    }
    Ok(())
}

fn require_observation_kind(
    observations: &RecoveryExpectedObservations,
    kind: RecoveryObservationKind,
) -> Result<(), RecoveryContractError> {
    observations
        .as_slice()
        .iter()
        .any(|value| value.observation_kind == kind)
        .then_some(())
        .ok_or(RecoveryContractError(
            "recovery action is missing a required expected-observation kind",
        ))
}

fn require_object_fingerprint_target_projection(
    observations: &RecoveryExpectedObservations,
) -> Result<(), RecoveryContractError> {
    if observations.as_slice().iter().any(|observation| {
        observation.observation_kind != RecoveryObservationKind::ObjectFingerprint
            || !matches!(
                observation.subject.0,
                RecoverySubjectRefKind::ConfigurationRoot(_)
                    | RecoverySubjectRefKind::MetadataObject(_)
            )
    }) {
        return Err(RecoveryContractError(
            "selective update requires only root/metadata object fingerprints",
        ));
    }
    Ok(())
}

fn require_finalization_receipt_ref(
    receipt_ref: &PreArmCancellationReceiptRef,
    expected_kind: PreArmCancellationEffectKind,
) -> Result<(), RecoveryContractError> {
    if receipt_ref.source() != PreArmCancellationReceiptSource::FinalizationPlan
        || receipt_ref.effect_kind() != expected_kind
    {
        return Err(RecoveryContractError(
            "pre-arm effect action requires its exact finalization-plan receipt ref",
        ));
    }
    Ok(())
}

fn validate_lock_subject_projection(
    observations: &RecoveryExpectedObservations,
    subjects: &RecoverySubjects,
    expected_digest: Option<&Sha256Digest>,
) -> Result<(), RecoveryContractError> {
    if observations.as_slice().len() != subjects.as_slice().len() {
        return Err(RecoveryContractError(
            "recovery lock observation count does not match subjects",
        ));
    }
    for (observation, subject) in observations.as_slice().iter().zip(subjects.as_slice()) {
        if observation.observation_kind != RecoveryObservationKind::LockOwnership
            || &observation.subject != subject
            || expected_digest.is_some_and(|expected| &observation.expected_digest != expected)
        {
            return Err(RecoveryContractError(
                "recovery lock observation does not match its subject",
            ));
        }
    }
    Ok(())
}

fn validate_prearm_outcome_projection(
    observations: &RecoveryExpectedObservations,
    support_action_id: &UnicaId,
) -> Result<(), RecoveryContractError> {
    let [support_graph, authorization, original_target, root_lock, mode_lease] =
        observations.as_slice()
    else {
        return Err(RecoveryContractError(
            "pre-arm outcome requires five exact observations",
        ));
    };
    let mode_is_exact = matches!(
        (mode_lease.observation_kind, &mode_lease.subject.0),
        (
            RecoveryObservationKind::WorkingInfobaseLease,
            RecoverySubjectRefKind::ExternalWorkingInfobase(_),
        ) | (
            RecoveryObservationKind::ReservedOriginalLease,
            RecoverySubjectRefKind::ReservedOriginalInfobase(_),
        )
    );
    if support_graph.observation_kind != RecoveryObservationKind::SupportGraph
        || authorization.observation_kind != RecoveryObservationKind::SupportActionAuthorization
        || !authorization.subject.is_registered(support_action_id)
        || original_target.observation_kind != RecoveryObservationKind::ObjectFingerprint
        || !matches!(
            &original_target.subject.0,
            RecoverySubjectRefKind::Registered(_)
                | RecoverySubjectRefKind::MetadataObject(_)
                | RecoverySubjectRefKind::ReservedOriginalInfobase(_)
        )
        || root_lock.observation_kind != RecoveryObservationKind::LockOwnership
        || !root_lock.subject.is_configuration_root()
        || !mode_is_exact
    {
        return Err(RecoveryContractError(
            "pre-arm outcome observation projection mismatch",
        ));
    }
    Ok(())
}

fn validate_recheck_projection(
    observations: &RecoveryExpectedObservations,
    finalization_attempt_id: &UnicaId,
    recheck_policy_digest: &Sha256Digest,
) -> Result<(), RecoveryContractError> {
    let values = observations.as_slice();
    if values.len() != 6 {
        return Err(RecoveryContractError(
            "pre-arm recheck requires six exact observations",
        ));
    }
    let exactly_one = |kind| {
        let mut matching = values.iter().filter(|value| value.observation_kind == kind);
        matching.next().filter(|_| matching.next().is_none())
    };
    let Some(support_graph) = exactly_one(RecoveryObservationKind::SupportGraph) else {
        return Err(RecoveryContractError(
            "pre-arm recheck requires one support-graph observation",
        ));
    };
    let Some(authorization) = exactly_one(RecoveryObservationKind::SupportActionAuthorization)
    else {
        return Err(RecoveryContractError(
            "pre-arm recheck requires one authorization observation",
        ));
    };
    let Some(original_target) = exactly_one(RecoveryObservationKind::ObjectFingerprint) else {
        return Err(RecoveryContractError(
            "pre-arm recheck requires one original-target observation",
        ));
    };
    let Some(root_lock) = exactly_one(RecoveryObservationKind::LockOwnership) else {
        return Err(RecoveryContractError(
            "pre-arm recheck requires one root-lock observation",
        ));
    };
    let working_mode = exactly_one(RecoveryObservationKind::WorkingInfobaseLease);
    let reserved_mode = exactly_one(RecoveryObservationKind::ReservedOriginalLease);
    let mode_is_exact = match (working_mode, reserved_mode) {
        (Some(value), None) => matches!(
            value.subject.0,
            RecoverySubjectRefKind::ExternalWorkingInfobase(_)
        ),
        (None, Some(value)) => matches!(
            value.subject.0,
            RecoverySubjectRefKind::ReservedOriginalInfobase(_)
        ),
        (Some(_), Some(_)) | (None, None) => false,
    };
    let Some(policy) = exactly_one(RecoveryObservationKind::FinalizationPolicy) else {
        return Err(RecoveryContractError(
            "pre-arm recheck requires one finalization-policy observation",
        ));
    };
    if !mode_is_exact
        || !root_lock.subject.is_configuration_root()
        || !matches!(
            authorization.subject.0,
            RecoverySubjectRefKind::Registered(_)
        )
        || !matches!(
            original_target.subject.0,
            RecoverySubjectRefKind::Registered(_)
                | RecoverySubjectRefKind::MetadataObject(_)
                | RecoverySubjectRefKind::ReservedOriginalInfobase(_)
        )
        || !matches!(
            support_graph.subject.0,
            RecoverySubjectRefKind::Registered(_) | RecoverySubjectRefKind::ConfigurationRoot(_)
        )
        || !policy.subject.is_registered(finalization_attempt_id)
        || &policy.expected_digest != recheck_policy_digest
    {
        return Err(RecoveryContractError(
            "pre-arm recheck observation projection mismatch",
        ));
    }
    Ok(())
}

fn validate_finish_prearm_projection(
    observations: &RecoveryExpectedObservations,
    support_action_id: &UnicaId,
) -> Result<(), RecoveryContractError> {
    validate_prearm_outcome_projection(observations, support_action_id)
}

fn validate_recovery_action_record(
    record: &RecoveryActionDigestRecordKind,
) -> Result<(), RecoveryContractError> {
    macro_rules! common {
        ($value:expr) => {
            validate_common_expected_observations(
                &$value.expected_observations,
                &$value.expected_postcondition_digest,
            )?
        };
    }

    match record {
        RecoveryActionDigestRecordKind::ReleaseOwnedLocks(value) => {
            common!(value);
            validate_lock_subject_projection(
                &value.expected_observations,
                &value.subjects,
                Some(&value.expected_owned_lock_set_digest),
            )
        }
        RecoveryActionDigestRecordKind::RestoreOriginal(value) => {
            common!(value);
            require_exact_single_observation(
                &value.expected_observations,
                RecoveryObservationKind::ObjectFingerprint,
                |subject| subject.is_reserved_original(None),
                Some(&value.expected_original_fingerprint),
            )
        }
        RecoveryActionDigestRecordKind::RestoreTaskCheckpoint(value) => {
            common!(value);
            require_exact_single_observation(
                &value.expected_observations,
                RecoveryObservationKind::TaskFingerprint,
                |subject| matches!(subject.0, RecoverySubjectRefKind::Registered(_)),
                Some(&value.expected_task_fingerprint),
            )
        }
        RecoveryActionDigestRecordKind::RecreateTaskInfobase(value) => {
            common!(value);
            require_exact_single_observation(
                &value.expected_observations,
                RecoveryObservationKind::TaskFingerprint,
                |subject| matches!(subject.0, RecoverySubjectRefKind::Registered(_)),
                Some(&value.expected_task_fingerprint),
            )
        }
        RecoveryActionDigestRecordKind::VerifyTaskFingerprint(value) => {
            common!(value);
            require_exact_single_observation(
                &value.expected_observations,
                RecoveryObservationKind::TaskFingerprint,
                |subject| matches!(subject.0, RecoverySubjectRefKind::Registered(_)),
                Some(&value.expected_task_fingerprint),
            )
        }
        RecoveryActionDigestRecordKind::ObserveCommit(value) => {
            common!(value);
            require_exact_single_observation(
                &value.expected_observations,
                RecoveryObservationKind::RepositoryVersion,
                |subject| subject.is_registered(&value.integration_set_id),
                Some(&value.expected_integration_set_digest),
            )
        }
        RecoveryActionDigestRecordKind::ObservePreArmCancellationOutcome(value) => {
            common!(value);
            validate_prearm_outcome_projection(
                &value.expected_observations,
                &value.support_action_id,
            )
        }
        RecoveryActionDigestRecordKind::AcquirePreArmRootGuard(value) => {
            common!(value);
            require_finalization_receipt_ref(
                &value.receipt_ref,
                PreArmCancellationEffectKind::RootGuardAcquire,
            )?;
            require_exact_single_observation(
                &value.expected_observations,
                RecoveryObservationKind::LockOwnership,
                RecoverySubjectRef::is_configuration_root,
                None,
            )
        }
        RecoveryActionDigestRecordKind::AcquirePreArmModeLease(value) => match &value.0 {
            AcquirePreArmModeLeaseActionDigestRecordKind::ReservedOriginal(value) => {
                common!(value);
                require_finalization_receipt_ref(
                    &value.receipt_ref,
                    PreArmCancellationEffectKind::ModeLeaseAcquire,
                )?;
                require_exact_single_observation(
                    &value.expected_observations,
                    RecoveryObservationKind::ReservedOriginalLease,
                    |subject| {
                        subject.is_reserved_original(Some(&value.reserved_original_identity_digest))
                    },
                    None,
                )
            }
            AcquirePreArmModeLeaseActionDigestRecordKind::SeparateWorkingInfobase(value) => {
                common!(value);
                require_finalization_receipt_ref(
                    &value.receipt_ref,
                    PreArmCancellationEffectKind::ModeLeaseAcquire,
                )?;
                require_exact_single_observation(
                    &value.expected_observations,
                    RecoveryObservationKind::WorkingInfobaseLease,
                    |subject| {
                        subject.is_external_working_infobase(&value.working_infobase_identity)
                    },
                    None,
                )
            }
        },
        RecoveryActionDigestRecordKind::RecheckPreArmCancellationFinalization(value) => {
            common!(value);
            validate_recheck_projection(
                &value.expected_observations,
                &value.finalization_attempt_id,
                &value.recheck_policy_digest,
            )
        }
        RecoveryActionDigestRecordKind::ApplyPreArmCancellationSelectiveUpdate(value) => {
            common!(value);
            require_finalization_receipt_ref(
                &value.receipt_ref,
                PreArmCancellationEffectKind::SelectiveOriginalUpdate,
            )?;
            require_object_fingerprint_target_projection(&value.expected_observations)
        }
        RecoveryActionDigestRecordKind::PersistPreArmSupportCancellation(value) => {
            common!(value);
            require_finalization_receipt_ref(
                &value.receipt_ref,
                PreArmCancellationEffectKind::AuthorizationCancellation,
            )?;
            require_exact_single_observation(
                &value.expected_observations,
                RecoveryObservationKind::SupportActionAuthorization,
                |subject| subject.is_registered(&value.support_action_id),
                None,
            )
        }
        RecoveryActionDigestRecordKind::ReleasePreArmModeLease(value) => match &value.0 {
            ReleasePreArmModeLeaseActionDigestRecordKind::ReservedOriginal(value) => {
                common!(value);
                require_finalization_receipt_ref(
                    &value.receipt_ref,
                    PreArmCancellationEffectKind::ModeLeaseRelease,
                )?;
                require_exact_single_observation(
                    &value.expected_observations,
                    RecoveryObservationKind::ReservedOriginalLease,
                    |subject| {
                        subject.is_reserved_original(Some(&value.reserved_original_identity_digest))
                    },
                    None,
                )
            }
            ReleasePreArmModeLeaseActionDigestRecordKind::SeparateWorkingInfobase(value) => {
                common!(value);
                require_finalization_receipt_ref(
                    &value.receipt_ref,
                    PreArmCancellationEffectKind::ModeLeaseRelease,
                )?;
                require_exact_single_observation(
                    &value.expected_observations,
                    RecoveryObservationKind::WorkingInfobaseLease,
                    |subject| {
                        subject.is_external_working_infobase(&value.working_infobase_identity)
                    },
                    None,
                )
            }
        },
        RecoveryActionDigestRecordKind::ReleasePreArmRootGuard(value) => {
            common!(value);
            require_finalization_receipt_ref(
                &value.receipt_ref,
                PreArmCancellationEffectKind::RootGuardRelease,
            )?;
            require_exact_single_observation(
                &value.expected_observations,
                RecoveryObservationKind::LockOwnership,
                RecoverySubjectRef::is_configuration_root,
                None,
            )
        }
        RecoveryActionDigestRecordKind::FinishPreArmCancellationRecovery(value) => {
            common!(value);
            require_finalization_receipt_ref(
                &value.receipt_ref,
                PreArmCancellationEffectKind::RecoveryFinalization,
            )?;
            validate_finish_prearm_projection(
                &value.expected_observations,
                &value.support_action_id,
            )
        }
        RecoveryActionDigestRecordKind::QuarantineArtifact(value) => {
            common!(value);
            let [artifact, quarantine] = value.expected_observations.as_slice() else {
                return Err(RecoveryContractError(
                    "artifact quarantine requires exact presence observations",
                ));
            };
            if artifact.observation_kind != RecoveryObservationKind::ArtifactPresence
                || !artifact.subject.is_registered(&value.artifact_id)
                || quarantine.observation_kind != RecoveryObservationKind::QuarantinePresence
                || !quarantine.subject.is_registered(&value.quarantine_id)
            {
                return Err(RecoveryContractError(
                    "artifact quarantine observation projection mismatch",
                ));
            }
            Ok(())
        }
        RecoveryActionDigestRecordKind::ObserveSupportPrerequisiteHistory(value) => {
            common!(value);
            let [anchor] = value.expected_observations.as_slice() else {
                return Err(RecoveryContractError(
                    "support history requires exactly its repository anchor",
                ));
            };
            if anchor.observation_kind != RecoveryObservationKind::RepositoryAnchor
                || !anchor.subject.is_registered(&value.support_action_id)
                || anchor.expected_digest != value.expected_partition_digest
            {
                return Err(RecoveryContractError(
                    "support history anchor differs from its action or partition",
                ));
            }
            Ok(())
        }
        RecoveryActionDigestRecordKind::UpdateOriginalSelectedTargets(value) => {
            common!(value);
            require_object_fingerprint_target_projection(&value.expected_observations)
        }
        RecoveryActionDigestRecordKind::ObserveWorkingInfobaseLease(value) => {
            common!(value);
            require_exact_single_observation(
                &value.expected_observations,
                RecoveryObservationKind::WorkingInfobaseLease,
                |subject| subject.is_external_working_infobase(&value.working_infobase_identity),
                None,
            )
        }
        RecoveryActionDigestRecordKind::ReleaseWorkingInfobaseLease(value) => {
            common!(value);
            require_exact_single_observation(
                &value.expected_observations,
                RecoveryObservationKind::WorkingInfobaseLease,
                |subject| subject.is_external_working_infobase(&value.working_infobase_identity),
                None,
            )
        }
        RecoveryActionDigestRecordKind::ObserveReservedOriginalLease(value) => {
            common!(value);
            require_exact_single_observation(
                &value.expected_observations,
                RecoveryObservationKind::ReservedOriginalLease,
                |subject| {
                    subject.is_reserved_original(Some(&value.reserved_original_identity_digest))
                },
                None,
            )
        }
        RecoveryActionDigestRecordKind::ReleaseReservedOriginalLease(value) => {
            common!(value);
            require_exact_single_observation(
                &value.expected_observations,
                RecoveryObservationKind::ReservedOriginalLease,
                |subject| {
                    subject.is_reserved_original(Some(&value.reserved_original_identity_digest))
                },
                None,
            )
        }
        RecoveryActionDigestRecordKind::ObserveRetentionLease(value) => {
            common!(value);
            require_exact_single_observation(
                &value.expected_observations,
                RecoveryObservationKind::RetentionLease,
                |subject| subject.is_retention_lease(&value.retention_lease_id),
                None,
            )
        }
        RecoveryActionDigestRecordKind::ObserveArchiveStaging(value) => {
            common!(value);
            require_exact_single_observation(
                &value.expected_observations,
                RecoveryObservationKind::ArchiveStagingPresence,
                |subject| subject.is_registered(&value.archive_staging_receipt_id),
                Some(&value.expected_archive_staging_receipt_digest),
            )
        }
        RecoveryActionDigestRecordKind::ReleaseRetentionLease(value) => {
            common!(value);
            require_exact_single_observation(
                &value.expected_observations,
                RecoveryObservationKind::RetentionLease,
                |subject| subject.is_retention_lease(&value.retention_lease_id),
                None,
            )
        }
        RecoveryActionDigestRecordKind::AwaitExternalSupportCorrection(value) => {
            common!(value);
            require_only_observation_kinds(
                &value.expected_observations,
                &[
                    RecoveryObservationKind::RepositoryVersion,
                    RecoveryObservationKind::SupportGraph,
                    RecoveryObservationKind::ObjectFingerprint,
                ],
            )
        }
        RecoveryActionDigestRecordKind::AwaitExternalLockRelease(value) => {
            common!(value);
            validate_lock_subject_projection(&value.expected_observations, &value.subjects, None)
        }
        RecoveryActionDigestRecordKind::AwaitManualWorkingInfobaseClosure(value) => {
            common!(value);
            require_exact_single_observation(
                &value.expected_observations,
                RecoveryObservationKind::WorkingInfobaseLease,
                |subject| subject.is_external_working_infobase(&value.working_infobase_identity),
                None,
            )
        }
        RecoveryActionDigestRecordKind::AwaitReservedOriginalClosure(value) => {
            common!(value);
            require_exact_single_observation(
                &value.expected_observations,
                RecoveryObservationKind::ReservedOriginalLease,
                |subject| {
                    subject.is_reserved_original(Some(&value.reserved_original_identity_digest))
                },
                None,
            )
        }
        RecoveryActionDigestRecordKind::AwaitExternalSupportConflictResolution(value) => {
            common!(value);
            require_only_observation_kinds(
                &value.expected_observations,
                &[
                    RecoveryObservationKind::RepositoryVersion,
                    RecoveryObservationKind::SupportGraph,
                ],
            )
        }
        RecoveryActionDigestRecordKind::AwaitSupportRecoveryEvidence(value) => {
            common!(value);
            require_only_observation_kinds(
                &value.expected_observations,
                &[
                    RecoveryObservationKind::RepositoryAnchor,
                    RecoveryObservationKind::RepositoryVersion,
                    RecoveryObservationKind::SupportGraph,
                    RecoveryObservationKind::LockOwnership,
                    RecoveryObservationKind::WorkingInfobaseLease,
                    RecoveryObservationKind::ReservedOriginalLease,
                    RecoveryObservationKind::ArtifactPresence,
                ],
            )
        }
        RecoveryActionDigestRecordKind::FinalizeSupportPrerequisiteRecovery(value) => {
            common!(value);
            let observations = value.expected_observations.as_slice();
            if observations.len() != 2 {
                return Err(RecoveryContractError(
                    "support finalization requires authorization and graph observations",
                ));
            }
            require_observation_kind(
                &value.expected_observations,
                RecoveryObservationKind::SupportActionAuthorization,
            )?;
            require_observation_kind(
                &value.expected_observations,
                RecoveryObservationKind::SupportGraph,
            )?;
            if !observations.iter().any(|observation| {
                observation.observation_kind == RecoveryObservationKind::SupportActionAuthorization
                    && observation.subject.is_registered(&value.support_action_id)
            }) {
                return Err(RecoveryContractError(
                    "support finalization authorization subject mismatch",
                ));
            }
            Ok(())
        }
        RecoveryActionDigestRecordKind::ResumeQuarantine(value) => {
            common!(value);
            let [artifact, quarantine] = value.expected_observations.as_slice() else {
                return Err(RecoveryContractError(
                    "resume quarantine requires exact presence observations",
                ));
            };
            if artifact.observation_kind != RecoveryObservationKind::ArtifactPresence
                || !artifact.subject.is_registered(&value.artifact_id)
                || quarantine.observation_kind != RecoveryObservationKind::QuarantinePresence
                || !quarantine.subject.is_registered(&value.quarantine_id)
            {
                return Err(RecoveryContractError(
                    "resume quarantine observation projection mismatch",
                ));
            }
            Ok(())
        }
        RecoveryActionDigestRecordKind::ResumeOwnedTargetQuarantine(value) => {
            common!(value);
            let expected_quarantined_digest = expected_quarantined_owned_target_digest(
                &value.owned_target,
                &value.quarantine_id,
            )?;
            if value.expected_quarantined_digest != expected_quarantined_digest {
                return Err(RecoveryContractError(
                    "resume owned-target quarantine digest does not describe its named quarantine",
                ));
            }
            require_exact_single_observation(
                &value.expected_observations,
                RecoveryObservationKind::QuarantinePresence,
                |subject| subject.is_owned_role(&value.owned_target),
                Some(&value.expected_quarantined_digest),
            )
        }
        RecoveryActionDigestRecordKind::FinishArchive(value) => {
            common!(value);
            let expected_release_set_digest = contract_digest(
                &HandoffRetentionReleaseSetDigestRecord(value.expected_releases.clone()),
                "handoff retention release-set digest failed",
            )?;
            if expected_release_set_digest != value.expected_release_set_digest {
                return Err(RecoveryContractError(
                    "finish archive release-set digest mismatch",
                ));
            }
            if value.retention_lease_ids.0.len() != value.expected_releases.0.len()
                || value
                    .retention_lease_ids
                    .0
                    .iter()
                    .zip(&value.expected_releases.0)
                    .any(|(lease_id, release)| lease_id != &release.retention_lease_id)
            {
                return Err(RecoveryContractError(
                    "finish archive lease/release projection mismatch",
                ));
            }
            require_exact_single_observation(
                &value.expected_observations,
                RecoveryObservationKind::ArchivePresence,
                |subject| subject.is_registered(&value.archive_id),
                None,
            )
        }
        RecoveryActionDigestRecordKind::FinishCleanup(value) => {
            common!(value);
            if value.expected_observations.as_slice().len() != value.owned_targets.0.len() {
                return Err(RecoveryContractError(
                    "finish cleanup observation/owned-target projection mismatch",
                ));
            }
            for (observation, target) in value
                .expected_observations
                .as_slice()
                .iter()
                .zip(&value.owned_targets.0)
            {
                let expected_absent_digest = expected_absent_owned_target_digest(
                    &value.archive_id,
                    &value.action_id,
                    target,
                )?;
                if observation.observation_kind != RecoveryObservationKind::OwnedTargetAbsence
                    || !observation.subject.is_owned_role(target)
                    || observation.expected_digest != expected_absent_digest
                {
                    return Err(RecoveryContractError(
                        "finish cleanup observation does not prove fresh absence for its target",
                    ));
                }
            }
            Ok(())
        }
    }
}

/// The closed recovery target vocabulary. The enclosing status contract supplies
/// target-specific branch evidence; this type is only the action-grammar key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) enum RecoveryTarget {
    TaskConfiguration,
    RepositoryLocks,
    OriginalConfiguration,
    RepositoryCommit,
    SupportPrerequisite,
    PreArmSupportCancellation,
    ManualWorkingInfobaseLease,
    Artifact,
    Archive,
    Cleanup,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) enum RecoveryEffectClass {
    Compensate,
    Rollback,
    ReconcileOnly,
    Quarantine,
    Cleanup,
}

/// A target/effect/action authority value. It is deliberately not
/// `Deserialize`: only a status authority that has validated the corresponding
/// branch evidence may construct it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RecoveryActionPlan {
    target: RecoveryTarget,
    effect_class: RecoveryEffectClass,
    actions: Vec<RecoveryAction>,
}

impl RecoveryActionPlan {
    #[cfg(test)]
    fn test_only(
        target: RecoveryTarget,
        effect_class: RecoveryEffectClass,
        actions: Vec<RecoveryAction>,
    ) -> Result<Self, RecoveryContractError> {
        validate_target_effect_action_grammar(target, effect_class, &actions)?;
        Ok(Self {
            target,
            effect_class,
            actions,
        })
    }

    pub(crate) fn validate_completed_outcomes(
        &self,
        outcomes: &[RecoveryActionOutcome],
    ) -> Result<(), RecoveryContractError> {
        if outcomes.len() != self.actions.len() {
            return Err(RecoveryContractError(
                "completed recovery outcomes do not cover every action",
            ));
        }
        for (action, outcome) in self.actions.iter().zip(outcomes) {
            let (outcome_action_id, outcome_action_digest) = outcome.action_binding();
            if outcome_action_id != action.action_id()
                || outcome_action_digest != action.action_digest()
            {
                return Err(RecoveryContractError(
                    "completed recovery outcome action binding mismatch",
                ));
            }
            let class_matches = match action.class() {
                RecoveryActionClass::ObservationOnly => {
                    outcome.outcome_class() == RecoveryActionOutcomeClass::AlreadySatisfied
                }
                RecoveryActionClass::OrdinaryMutating | RecoveryActionClass::PreArmEffect => {
                    matches!(
                        outcome.outcome_class(),
                        RecoveryActionOutcomeClass::Performed
                            | RecoveryActionOutcomeClass::RecoveredReceipt
                    )
                }
            };
            if !class_matches {
                return Err(RecoveryContractError(
                    "completed recovery outcome class mismatch",
                ));
            }
        }

        if self.target == RecoveryTarget::Archive
            && self.effect_class == RecoveryEffectClass::Cleanup
        {
            for (index, action) in self.actions.iter().enumerate() {
                if !matches!(action.0, RecoveryActionKindWire::ReleaseRetentionLease(_)) {
                    continue;
                }
                let Some(RecoveryAction(RecoveryActionKindWire::ObserveRetentionLease(observe))) =
                    index
                        .checked_sub(1)
                        .and_then(|previous| self.actions.get(previous))
                else {
                    return Err(RecoveryContractError(
                        "archive release outcome lacks its lease observation",
                    ));
                };
                let expected = match observe.expected_lease_state {
                    RetentionLeaseExpectedState::Held => RecoveryActionOutcomeClass::Performed,
                    RetentionLeaseExpectedState::Released => {
                        RecoveryActionOutcomeClass::RecoveredReceipt
                    }
                };
                if outcomes[index].outcome_class() != expected {
                    return Err(RecoveryContractError(
                        "archive release outcome does not match observed lease state",
                    ));
                }
            }
        }
        Ok(())
    }
}

fn validate_target_effect_action_grammar(
    target: RecoveryTarget,
    effect_class: RecoveryEffectClass,
    actions: &[RecoveryAction],
) -> Result<(), RecoveryContractError> {
    if actions.len() > MAX_RECOVERY_ITEMS {
        return Err(RecoveryContractError("recovery action plan is oversized"));
    }
    for (index, action) in actions.iter().enumerate() {
        let action_id = action.common().0;
        if actions[..index]
            .iter()
            .any(|prior| prior.common().0 == action_id)
        {
            return Err(RecoveryContractError(
                "recovery action plan contains a duplicate action id",
            ));
        }
    }

    let valid = match (target, effect_class) {
        (RecoveryTarget::TaskConfiguration, RecoveryEffectClass::Rollback) => {
            matches!(
                actions,
                [RecoveryAction(
                    RecoveryActionKindWire::RestoreTaskCheckpoint(_)
                )] | [RecoveryAction(
                    RecoveryActionKindWire::RecreateTaskInfobase(_)
                )] | [RecoveryAction(
                    RecoveryActionKindWire::VerifyTaskFingerprint(_)
                )]
            )
        }
        (RecoveryTarget::RepositoryLocks, RecoveryEffectClass::Compensate) => matches!(
            actions,
            [RecoveryAction(RecoveryActionKindWire::ReleaseOwnedLocks(_))]
        ),
        (RecoveryTarget::OriginalConfiguration, RecoveryEffectClass::Rollback) => matches!(
            actions,
            [RecoveryAction(RecoveryActionKindWire::RestoreOriginal(_))]
                | [
                    RecoveryAction(RecoveryActionKindWire::RestoreOriginal(_)),
                    RecoveryAction(RecoveryActionKindWire::ReleaseOwnedLocks(_)),
                ]
        ),
        (RecoveryTarget::RepositoryCommit, RecoveryEffectClass::ReconcileOnly) => matches!(
            actions,
            [] | [RecoveryAction(RecoveryActionKindWire::ObserveCommit(_))]
                | [RecoveryAction(RecoveryActionKindWire::ReleaseOwnedLocks(_))]
        ),
        (RecoveryTarget::RepositoryCommit, RecoveryEffectClass::Rollback) => matches!(
            actions,
            [
                RecoveryAction(RecoveryActionKindWire::RestoreOriginal(_)),
                RecoveryAction(RecoveryActionKindWire::ReleaseOwnedLocks(_)),
            ]
        ),
        (RecoveryTarget::ManualWorkingInfobaseLease, RecoveryEffectClass::ReconcileOnly) => {
            validate_manual_working_infobase_lease_actions(actions).is_ok()
        }
        (RecoveryTarget::Artifact, RecoveryEffectClass::Quarantine) => matches!(
            actions,
            [RecoveryAction(RecoveryActionKindWire::QuarantineArtifact(
                _
            ))] | [RecoveryAction(RecoveryActionKindWire::ResumeQuarantine(_))]
        ),
        (RecoveryTarget::Archive, RecoveryEffectClass::Cleanup) => {
            validate_archive_action_grammar(actions).is_ok()
        }
        (RecoveryTarget::Cleanup, RecoveryEffectClass::Cleanup) => {
            validate_cleanup_action_grammar(actions).is_ok()
        }
        // These rows need their closed branch evidence before an action catalog
        // can be authoritative. Their dedicated status constructors validate
        // that evidence together with the action grammar.
        (RecoveryTarget::SupportPrerequisite, RecoveryEffectClass::ReconcileOnly)
        | (RecoveryTarget::PreArmSupportCancellation, RecoveryEffectClass::ReconcileOnly) => {
            return Err(RecoveryContractError(
                "support recovery actions require their dedicated branch authority",
            ));
        }
        _ => false,
    };

    valid.then_some(()).ok_or(RecoveryContractError(
        "unsupported recovery target/effect/action combination",
    ))
}

fn validate_manual_working_infobase_lease_actions(
    actions: &[RecoveryAction],
) -> Result<(), RecoveryContractError> {
    match actions {
        [RecoveryAction(RecoveryActionKindWire::ObserveWorkingInfobaseLease(observe))] => {
            if observe.expected_lease_state == ExternalLeaseExpectedState::ExclusivelyHeld {
                return Err(RecoveryContractError(
                    "a held working-infobase lease requires its release action",
                ));
            }
            Ok(())
        }
        [RecoveryAction(RecoveryActionKindWire::ObserveWorkingInfobaseLease(observe)), RecoveryAction(RecoveryActionKindWire::ReleaseWorkingInfobaseLease(release))]
            if observe.expected_lease_state == ExternalLeaseExpectedState::ExclusivelyHeld
                && observe.working_infobase_identity == release.working_infobase_identity
                && observe.exclusive_lease_capability_id
                    == release.exclusive_lease_capability_id =>
        {
            Ok(())
        }
        _ => Err(RecoveryContractError(
            "invalid manual working-infobase lease action grammar",
        )),
    }
}

fn expected_matched_terminal_observation_digests(
    observations: &RecoveryExpectedObservations,
) -> Result<TerminalObservationDigests, RecoveryContractError> {
    let digests = observations
        .as_slice()
        .iter()
        .map(|observation| {
            contract_digest(
                &RecoveryObservationDigestRecord(RecoveryObservationDigestRecordKind::Matched(
                    MatchedRecoveryObservationDigestRecord {
                        outcome: MatchesOutcome::Value,
                        observation_kind: observation.observation_kind,
                        subject: observation.subject.clone(),
                        expected_digest: observation.expected_digest.clone(),
                        observed_digest: observation.expected_digest.clone(),
                    },
                )),
                "expected matched recovery observation digest failed",
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    TerminalObservationDigests::new(digests)
}

fn expected_retention_release_receipt_digest(
    release: &ReleaseRetentionLeaseAction,
) -> Result<Sha256Digest, RecoveryContractError> {
    contract_digest(
        &RecoveryActionEffectReceiptDigestRecord {
            receipt_kind: RecoveryActionReceiptKind::Value,
            receipt_id: release.expected_release_receipt_id.clone(),
            producer_action_id: release.action_id.clone(),
            producer_action_digest: release.action_digest.clone(),
            terminal_observation_digests: expected_matched_terminal_observation_digests(
                &release.expected_observations,
            )?,
        },
        "expected retention release receipt digest failed",
    )
}

fn validate_archive_action_grammar(
    actions: &[RecoveryAction],
) -> Result<(), RecoveryContractError> {
    let Some((first, remainder)) = actions.split_first() else {
        return Err(RecoveryContractError("archive recovery actions are empty"));
    };
    let Some((finish, middle)) = remainder.split_last() else {
        return Err(RecoveryContractError(
            "archive recovery requires staging observation and finalization",
        ));
    };
    let RecoveryAction(RecoveryActionKindWire::ObserveArchiveStaging(staging)) = first else {
        return Err(RecoveryContractError(
            "archive recovery must observe staging first",
        ));
    };
    let RecoveryAction(RecoveryActionKindWire::FinishArchive(finish)) = finish else {
        return Err(RecoveryContractError(
            "archive recovery must finish exactly once and last",
        ));
    };
    if staging.archive_staging_receipt_id != finish.archive_staging_receipt_id
        || staging.expected_archive_staging_receipt_digest
            != finish.expected_archive_staging_receipt_digest
        || staging.handoff_lineage_digest != finish.handoff_lineage_digest
    {
        return Err(RecoveryContractError(
            "archive staging evidence changed before finalization",
        ));
    }

    let mut observed_lease_ids = Vec::new();
    let mut index = 0;
    while index < middle.len() {
        let RecoveryAction(RecoveryActionKindWire::ObserveRetentionLease(observe)) = &middle[index]
        else {
            return Err(RecoveryContractError(
                "archive recovery lease actions must start with an observation",
            ));
        };
        let expected_release = finish
            .expected_releases
            .0
            .get(observed_lease_ids.len())
            .ok_or(RecoveryContractError(
                "archive release lineage is missing its lease entry",
            ))?;
        observed_lease_ids.push(observe.retention_lease_id.clone());
        index += 1;

        let Some(RecoveryAction(RecoveryActionKindWire::ReleaseRetentionLease(release))) =
            middle.get(index)
        else {
            return Err(RecoveryContractError(
                "every retention lease requires its exact release action",
            ));
        };
        if observe.retention_lease_id != release.retention_lease_id
            || observe.retention_capability_row_id != release.retention_capability_row_id
            || release.archive_staging_receipt_id != staging.archive_staging_receipt_id
            || release.expected_archive_staging_receipt_digest
                != staging.expected_archive_staging_receipt_digest
            || expected_release.retention_lease_id != release.retention_lease_id
            || expected_release.release_action_id != release.action_id
            || expected_release.release_action_digest != release.action_digest
            || expected_release.release_receipt_id != release.expected_release_receipt_id
            || expected_release.release_receipt_digest
                != expected_retention_release_receipt_digest(release)?
        {
            return Err(RecoveryContractError(
                "retention release does not bind its observation, staging, and finish lineage",
            ));
        }
        index += 1;
    }

    if observed_lease_ids != finish.retention_lease_ids.0 {
        return Err(RecoveryContractError(
            "archive lease observations do not equal the final lease set",
        ));
    }
    Ok(())
}

fn validate_cleanup_action_grammar(
    actions: &[RecoveryAction],
) -> Result<(), RecoveryContractError> {
    let Some((finish, resumed)) = actions.split_last() else {
        return Err(RecoveryContractError("cleanup recovery actions are empty"));
    };
    let RecoveryAction(RecoveryActionKindWire::FinishCleanup(finish)) = finish else {
        return Err(RecoveryContractError(
            "cleanup recovery must finish exactly once and last",
        ));
    };
    if resumed.len() != finish.owned_targets.0.len()
        || resumed
            .iter()
            .zip(&finish.owned_targets.0)
            .any(|(action, target)| {
                !matches!(
                    action,
                    RecoveryAction(RecoveryActionKindWire::ResumeOwnedTargetQuarantine(resume))
                        if &resume.owned_target == target
                )
            })
    {
        return Err(RecoveryContractError(
            "cleanup resume actions do not equal the canonical owned-target set",
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
struct RecoveryPlanObservations(Vec<RecoveryObservation>);

impl JsonSchema for RecoveryPlanObservations {
    fn schema_name() -> Cow<'static, str> {
        "RecoveryPlanObservations".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "array",
            "items": generator.subschema_for::<RecoveryObservation>(),
            "minItems": 0,
            "maxItems": MAX_RECOVERY_ITEMS,
            "uniqueItems": true
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
struct RecoveryPlanUnknowns(Vec<RecoveryUnknown>);

impl JsonSchema for RecoveryPlanUnknowns {
    fn schema_name() -> Cow<'static, str> {
        "RecoveryPlanRemainingUnknowns".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "array",
            "items": generator.subschema_for::<RecoveryUnknown>(),
            "minItems": 0,
            "maxItems": MAX_RECOVERY_ITEMS,
            "uniqueItems": true
        })
    }
}

fn validated_plan_observations(
    values: Vec<RecoveryObservation>,
) -> Result<(RecoveryPlanObservations, RecoveryPlanUnknowns), RecoveryContractError> {
    if values.len() > MAX_RECOVERY_ITEMS {
        return Err(RecoveryContractError(
            "recovery plan observations exceed the general collection bound",
        ));
    }
    let mut previous = None;
    let mut unknowns = Vec::new();
    for value in &values {
        let key = value.expected_projection().canonical_key()?;
        if previous.as_ref().is_some_and(|previous| previous >= &key) {
            return Err(RecoveryContractError(
                "recovery plan observations must be canonical and unique",
            ));
        }
        previous = Some(key);
        if matches!(value.0, RecoveryObservationKindWire::Unknown(_)) {
            unknowns.push(RecoveryUnknown::from_observation(value)?);
        }
    }
    Ok((
        RecoveryPlanObservations(values),
        RecoveryPlanUnknowns(unknowns),
    ))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
struct RecoveryPlanActions(Vec<RecoveryAction>);

impl RecoveryPlanActions {
    fn new(values: Vec<RecoveryAction>) -> Result<Self, RecoveryContractError> {
        if values.len() > MAX_RECOVERY_ITEMS {
            return Err(RecoveryContractError(
                "recovery actions exceed the general collection bound",
            ));
        }
        for (index, action) in values.iter().enumerate() {
            if values[..index]
                .iter()
                .any(|prior| prior.action_id() == action.action_id())
            {
                return Err(RecoveryContractError(
                    "recovery actions repeat an action ID",
                ));
            }
        }
        Ok(Self(values))
    }

    fn as_slice(&self) -> &[RecoveryAction] {
        &self.0
    }
}

fn exact_action_array_schema(items: Vec<Schema>) -> Schema {
    let length = items.len();
    if items.is_empty() {
        return json_schema!({
            "type": "array",
            "items": { "type": "string", "const": "__no_recovery_action__" },
            "minItems": 0,
            "maxItems": 0,
            "uniqueItems": true
        });
    }
    json_schema!({
        "type": "array",
        "prefixItems": items,
        "items": false,
        "minItems": length,
        "maxItems": length,
        "uniqueItems": true
    })
}

fn ordered_terminal_action_array_schema(
    first: Schema,
    middle: Vec<Schema>,
    terminal: Schema,
    min_items: usize,
) -> Schema {
    let mut tail = middle;
    tail.push(terminal.clone());
    json_schema!({
        "type": "array",
        "prefixItems": [first],
        "items": one_of_schema(tail),
        "minItems": min_items,
        "maxItems": MAX_RECOVERY_ITEMS,
        "uniqueItems": true,
        "contains": terminal,
        "minContains": 1,
        "maxContains": 1
    })
}

fn prearm_finalize_action_array_schema(generator: &mut SchemaGenerator) -> Schema {
    let acquire_root = generator.subschema_for::<AcquirePreArmRootGuardAction>();
    let acquire_mode = generator.subschema_for::<AcquirePreArmModeLeaseAction>();
    let recheck = generator.subschema_for::<RecheckPreArmCancellationFinalizationAction>();
    let apply = generator.subschema_for::<ApplyPreArmCancellationSelectiveUpdateAction>();
    let persist = generator.subschema_for::<PersistPreArmSupportCancellationAction>();
    let release_mode = generator.subschema_for::<ReleasePreArmModeLeaseAction>();
    let release_root = generator.subschema_for::<ReleasePreArmRootGuardAction>();
    let finish = generator.subschema_for::<FinishPreArmCancellationRecoveryAction>();

    let mut variants = Vec::with_capacity(64);
    for mask in 0_u8..64 {
        let mut items = Vec::with_capacity(8);
        if mask & 1 != 0 {
            items.push(acquire_root.clone());
        }
        if mask & 2 != 0 {
            items.push(acquire_mode.clone());
        }
        items.push(recheck.clone());
        if mask & 4 != 0 {
            items.push(apply.clone());
        }
        if mask & 8 != 0 {
            items.push(persist.clone());
        }
        if mask & 16 != 0 {
            items.push(release_mode.clone());
        }
        if mask & 32 != 0 {
            items.push(release_root.clone());
        }
        items.push(finish.clone());
        variants.push(exact_action_array_schema(items));
    }
    one_of_schema(variants)
}

macro_rules! recovery_plan_action_wrapper {
    ($name:ident, |$actions:ident| $validate:block, |$generator:ident| $schema:block) => {
        #[derive(Debug, Clone, PartialEq, Eq, Serialize)]
        #[serde(transparent)]
        struct $name(RecoveryPlanActions);

        impl $name {
            fn from_actions(actions: Vec<RecoveryAction>) -> Result<Self, RecoveryContractError> {
                let actions = RecoveryPlanActions::new(actions)?;
                let $actions = actions.as_slice();
                ($validate)?;
                Ok(Self(actions))
            }

            fn as_slice(&self) -> &[RecoveryAction] {
                self.0.as_slice()
            }
        }

        impl JsonSchema for $name {
            fn schema_name() -> Cow<'static, str> {
                stringify!($name).into()
            }

            fn json_schema($generator: &mut SchemaGenerator) -> Schema $schema
        }
    };
}

recovery_plan_action_wrapper!(
    TaskConfigurationRecoveryActions,
    |actions| {
        validate_target_effect_action_grammar(
            RecoveryTarget::TaskConfiguration,
            RecoveryEffectClass::Rollback,
            actions,
        )
    },
    |generator| {
        one_of_schema(vec![
            exact_action_array_schema(vec![
                generator.subschema_for::<RestoreTaskCheckpointAction>()
            ]),
            exact_action_array_schema(
                vec![generator.subschema_for::<RecreateTaskInfobaseAction>()],
            ),
            exact_action_array_schema(vec![
                generator.subschema_for::<VerifyTaskFingerprintAction>()
            ]),
        ])
    }
);
recovery_plan_action_wrapper!(
    RepositoryLocksRecoveryActions,
    |actions| {
        validate_target_effect_action_grammar(
            RecoveryTarget::RepositoryLocks,
            RecoveryEffectClass::Compensate,
            actions,
        )
    },
    |generator| {
        exact_action_array_schema(vec![generator.subschema_for::<ReleaseOwnedLocksAction>()])
    }
);
recovery_plan_action_wrapper!(
    OriginalConfigurationRecoveryActions,
    |actions| {
        validate_target_effect_action_grammar(
            RecoveryTarget::OriginalConfiguration,
            RecoveryEffectClass::Rollback,
            actions,
        )
    },
    |generator| {
        one_of_schema(vec![
            exact_action_array_schema(vec![generator.subschema_for::<RestoreOriginalAction>()]),
            exact_action_array_schema(vec![
                generator.subschema_for::<RestoreOriginalAction>(),
                generator.subschema_for::<ReleaseOwnedLocksAction>(),
            ]),
        ])
    }
);
recovery_plan_action_wrapper!(
    RepositoryCommitObserveRecoveryActions,
    |actions| {
        matches!(
            actions,
            [RecoveryAction(RecoveryActionKindWire::ObserveCommit(_))]
        )
        .then_some(())
        .ok_or(RecoveryContractError(
            "repository commit observation wrapper requires observeCommit",
        ))
    },
    |generator| {
        exact_action_array_schema(vec![generator.subschema_for::<ObserveCommitAction>()])
    }
);
recovery_plan_action_wrapper!(
    RepositoryCommitCommittedRecoveryActions,
    |actions| {
        validate_target_effect_action_grammar(
            RecoveryTarget::RepositoryCommit,
            RecoveryEffectClass::ReconcileOnly,
            actions,
        )
    },
    |generator| {
        one_of_schema(vec![
            exact_action_array_schema(Vec::new()),
            exact_action_array_schema(vec![generator.subschema_for::<ReleaseOwnedLocksAction>()]),
        ])
    }
);
recovery_plan_action_wrapper!(
    RepositoryCommitNotCommittedRecoveryActions,
    |actions| {
        validate_target_effect_action_grammar(
            RecoveryTarget::RepositoryCommit,
            RecoveryEffectClass::Rollback,
            actions,
        )
    },
    |generator| {
        exact_action_array_schema(vec![
            generator.subschema_for::<RestoreOriginalAction>(),
            generator.subschema_for::<ReleaseOwnedLocksAction>(),
        ])
    }
);
recovery_plan_action_wrapper!(
    SupportPrerequisiteRecoveryActions,
    |actions| { validate_support_recovery_action_shape(actions) },
    |generator| {
        ordered_terminal_action_array_schema(
            generator.subschema_for::<ObserveSupportPrerequisiteHistoryAction>(),
            vec![
                generator.subschema_for::<AwaitExternalSupportCorrectionAction>(),
                generator.subschema_for::<AwaitExternalLockReleaseAction>(),
                generator.subschema_for::<AwaitManualWorkingInfobaseClosureAction>(),
                generator.subschema_for::<AwaitReservedOriginalClosureAction>(),
                generator.subschema_for::<AwaitExternalSupportConflictResolutionAction>(),
                generator.subschema_for::<AwaitSupportRecoveryEvidenceAction>(),
                generator.subschema_for::<ObserveWorkingInfobaseLeaseAction>(),
                generator.subschema_for::<ReleaseWorkingInfobaseLeaseAction>(),
                generator.subschema_for::<ObserveReservedOriginalLeaseAction>(),
                generator.subschema_for::<ReleaseReservedOriginalLeaseAction>(),
                generator.subschema_for::<UpdateOriginalSelectedTargetsAction>(),
            ],
            generator.subschema_for::<FinalizeSupportPrerequisiteRecoveryAction>(),
            2,
        )
    }
);
recovery_plan_action_wrapper!(
    PreArmObserveRecoveryActions,
    |actions| {
        matches!(
            actions,
            [RecoveryAction(
                RecoveryActionKindWire::ObservePreArmCancellationOutcome(_)
            )]
        )
        .then_some(())
        .ok_or(RecoveryContractError(
            "pre-arm observation wrapper requires its single observation action",
        ))
    },
    |generator| {
        exact_action_array_schema(vec![
            generator.subschema_for::<ObservePreArmCancellationOutcomeAction>()
        ])
    }
);
recovery_plan_action_wrapper!(
    PreArmFinalizeRecoveryActions,
    |actions| { validate_prearm_finalize_action_shape(actions) },
    |generator| { prearm_finalize_action_array_schema(generator) }
);
recovery_plan_action_wrapper!(
    ManualWorkingInfobaseLeaseRecoveryActions,
    |actions| {
        validate_target_effect_action_grammar(
            RecoveryTarget::ManualWorkingInfobaseLease,
            RecoveryEffectClass::ReconcileOnly,
            actions,
        )
    },
    |generator| {
        one_of_schema(vec![
            exact_action_array_schema(vec![
                generator.subschema_for::<ObserveWorkingInfobaseLeaseAction>()
            ]),
            exact_action_array_schema(vec![
                generator.subschema_for::<ObserveWorkingInfobaseLeaseAction>(),
                generator.subschema_for::<ReleaseWorkingInfobaseLeaseAction>(),
            ]),
        ])
    }
);
recovery_plan_action_wrapper!(
    ArtifactRecoveryActions,
    |actions| {
        validate_target_effect_action_grammar(
            RecoveryTarget::Artifact,
            RecoveryEffectClass::Quarantine,
            actions,
        )
    },
    |generator| {
        one_of_schema(vec![
            exact_action_array_schema(vec![generator.subschema_for::<QuarantineArtifactAction>()]),
            exact_action_array_schema(vec![generator.subschema_for::<ResumeQuarantineAction>()]),
        ])
    }
);
recovery_plan_action_wrapper!(
    ArchiveRecoveryActions,
    |actions| { validate_archive_action_grammar(actions) },
    |generator| {
        ordered_terminal_action_array_schema(
            generator.subschema_for::<ObserveArchiveStagingAction>(),
            vec![
                generator.subschema_for::<ObserveRetentionLeaseAction>(),
                generator.subschema_for::<ReleaseRetentionLeaseAction>(),
            ],
            generator.subschema_for::<FinishArchiveAction>(),
            2,
        )
    }
);
recovery_plan_action_wrapper!(
    CleanupRecoveryActions,
    |actions| { validate_cleanup_action_grammar(actions) },
    |generator| {
        ordered_terminal_action_array_schema(
            generator.subschema_for::<ResumeOwnedTargetQuarantineAction>(),
            vec![generator.subschema_for::<ResumeOwnedTargetQuarantineAction>()],
            generator.subschema_for::<FinishCleanupAction>(),
            2,
        )
    }
);

wire_literal!(TaskConfigurationRecoveryTarget, "taskConfiguration");
wire_literal!(RepositoryLocksRecoveryTarget, "repositoryLocks");
wire_literal!(OriginalConfigurationRecoveryTarget, "originalConfiguration");
wire_literal!(RepositoryCommitRecoveryTarget, "repositoryCommit");
wire_literal!(SupportPrerequisiteRecoveryTarget, "supportPrerequisite");
wire_literal!(
    PreArmSupportCancellationRecoveryTarget,
    "preArmSupportCancellation"
);
wire_literal!(
    ManualWorkingInfobaseLeaseRecoveryTarget,
    "manualWorkingInfobaseLease"
);
wire_literal!(ArtifactRecoveryTarget, "artifact");
wire_literal!(ArchiveRecoveryTarget, "archive");
wire_literal!(CleanupRecoveryTarget, "cleanup");
wire_literal!(CompensateRecoveryEffectClass, "compensate");
wire_literal!(RollbackRecoveryEffectClass, "rollback");
wire_literal!(ReconcileOnlyRecoveryEffectClass, "reconcileOnly");
wire_literal!(QuarantineRecoveryEffectClass, "quarantine");
wire_literal!(CleanupRecoveryEffectClass, "cleanup");
wire_literal!(ObserveOutcomeCommitRecoveryStage, "observeOutcome");
wire_literal!(CommittedCommitRecoveryStage, "committed");
wire_literal!(NotCommittedCommitRecoveryStage, "notCommitted");
wire_literal!(ObserveOutcomePreArmRecoveryStage, "observeOutcome");
wire_literal!(FinalizePreArmRecoveryStage, "finalize");

/// One exact source-evidence value for one recovery-history partition entry.
/// NCC is a first-class typed branch because it is legal in the recovery
/// range but cannot masquerade as a support-prerequisite observation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub(crate) enum SupportRecoveryHistoryEvidence {
    SupportObservation(SupportPrerequisiteVersionObservation),
    NonConflictingConcurrent(NonConflictingConcurrentEvidence),
}

impl SupportRecoveryHistoryEvidence {
    pub(crate) fn repository_version(&self) -> &super::scalars::RepositoryVersion {
        match self {
            Self::SupportObservation(value) => value.repository_version(),
            Self::NonConflictingConcurrent(value) => value.repository_version(),
        }
    }

    pub(crate) const fn support_observation(
        &self,
    ) -> Option<&SupportPrerequisiteVersionObservation> {
        match self {
            Self::SupportObservation(value) => Some(value),
            Self::NonConflictingConcurrent(_) => None,
        }
    }

    pub(crate) const fn non_conflicting_concurrent(
        &self,
    ) -> Option<&NonConflictingConcurrentEvidence> {
        match self {
            Self::SupportObservation(_) => None,
            Self::NonConflictingConcurrent(value) => Some(value),
        }
    }
}

impl From<SupportPrerequisiteVersionObservation> for SupportRecoveryHistoryEvidence {
    fn from(value: SupportPrerequisiteVersionObservation) -> Self {
        Self::SupportObservation(value)
    }
}

impl From<NonConflictingConcurrentEvidence> for SupportRecoveryHistoryEvidence {
    fn from(value: NonConflictingConcurrentEvidence) -> Self {
        Self::NonConflictingConcurrent(value)
    }
}

impl JsonSchema for SupportRecoveryHistoryEvidence {
    fn schema_name() -> Cow<'static, str> {
        "SupportRecoveryHistoryEvidence".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<SupportPrerequisiteVersionObservation>(),
            generator.subschema_for::<NonConflictingConcurrentEvidence>(),
        ])
    }
}

/// Ordered, bounded version evidence retained by an armed support-recovery
/// plan. Every value corresponds one-to-one with the partition entry in the
/// same position; duplicate repository versions are rejected across branches.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct SupportRecoveryVersionObservations(Vec<SupportRecoveryHistoryEvidence>);

impl SupportRecoveryVersionObservations {
    pub(crate) fn new(
        values: Vec<SupportRecoveryHistoryEvidence>,
    ) -> Result<Self, RecoveryContractError> {
        if values.len() > MAX_RECOVERY_ITEMS {
            return Err(RecoveryContractError(
                "support recovery version observations exceed the general collection bound",
            ));
        }
        let mut versions = std::collections::BTreeSet::new();
        if values
            .iter()
            .any(|value| !versions.insert(value.repository_version().as_str()))
        {
            return Err(RecoveryContractError(
                "support recovery version observations repeat a repository version",
            ));
        }
        Ok(Self(values))
    }

    pub(crate) fn as_slice(&self) -> &[SupportRecoveryHistoryEvidence] {
        &self.0
    }

    pub(crate) fn digest(&self) -> Result<Sha256Digest, RecoveryContractError> {
        contract_digest(
            &SupportRecoveryVersionObservationDigestRecord(self.clone()),
            "support recovery version-observation digest failed",
        )
    }
}

impl JsonSchema for SupportRecoveryVersionObservations {
    fn schema_name() -> Cow<'static, str> {
        "SupportRecoveryVersionObservations".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "array",
            "items": generator.subschema_for::<SupportRecoveryHistoryEvidence>(),
            "minItems": 0,
            "maxItems": MAX_RECOVERY_ITEMS,
            "uniqueItems": true
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(transparent)]
pub(crate) struct SupportRecoveryVersionObservationDigestRecord(SupportRecoveryVersionObservations);

impl contract_digest_record_sealed::Sealed for SupportRecoveryVersionObservationDigestRecord {}
impl ContractDigestRecord for SupportRecoveryVersionObservationDigestRecord {}

macro_rules! recovery_plan_digest_record {
    ($name:ty) => {
        impl contract_digest_record_sealed::Sealed for $name {}
        impl ContractDigestRecord for $name {}
    };
}

macro_rules! recovery_plan_leaf {
    ($name:ident, $record:ty) => {
        #[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct $name {
            #[serde(flatten)]
            record: $record,
            recovery_digest: Sha256Digest,
        }

        impl $name {
            fn new(record: $record) -> Result<Self, RecoveryContractError> {
                let recovery_digest = contract_digest(&record, "recovery plan digest failed")?;
                Ok(Self {
                    record,
                    recovery_digest,
                })
            }
        }
    };
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TaskConfigurationRecoveryPlanDigestRecord {
    prior_operation_id: OperationId,
    target: TaskConfigurationRecoveryTarget,
    effect_class: RollbackRecoveryEffectClass,
    planned_result_phase: TaskPhase,
    observations: RecoveryPlanObservations,
    actions: TaskConfigurationRecoveryActions,
    remaining_unknowns: RecoveryPlanUnknowns,
}
recovery_plan_digest_record!(TaskConfigurationRecoveryPlanDigestRecord);
recovery_plan_leaf!(
    TaskConfigurationRecoveryPlanStatus,
    TaskConfigurationRecoveryPlanDigestRecord
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RepositoryLocksRecoveryPlanDigestRecord {
    prior_operation_id: OperationId,
    target: RepositoryLocksRecoveryTarget,
    effect_class: CompensateRecoveryEffectClass,
    planned_result_phase: TaskPhase,
    observations: RecoveryPlanObservations,
    actions: RepositoryLocksRecoveryActions,
    remaining_unknowns: RecoveryPlanUnknowns,
}
recovery_plan_digest_record!(RepositoryLocksRecoveryPlanDigestRecord);
recovery_plan_leaf!(
    RepositoryLocksRecoveryPlanStatus,
    RepositoryLocksRecoveryPlanDigestRecord
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct OriginalConfigurationRecoveryPlanDigestRecord {
    prior_operation_id: OperationId,
    target: OriginalConfigurationRecoveryTarget,
    effect_class: RollbackRecoveryEffectClass,
    planned_result_phase: TaskPhase,
    observations: RecoveryPlanObservations,
    actions: OriginalConfigurationRecoveryActions,
    remaining_unknowns: RecoveryPlanUnknowns,
}
recovery_plan_digest_record!(OriginalConfigurationRecoveryPlanDigestRecord);
recovery_plan_leaf!(
    OriginalConfigurationRecoveryPlanStatus,
    OriginalConfigurationRecoveryPlanDigestRecord
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RepositoryCommitObserveRecoveryPlanDigestRecord {
    prior_operation_id: OperationId,
    target: RepositoryCommitRecoveryTarget,
    effect_class: ReconcileOnlyRecoveryEffectClass,
    repository_commit_stage: ObserveOutcomeCommitRecoveryStage,
    planned_result_phase: TaskPhase,
    observations: RecoveryPlanObservations,
    actions: RepositoryCommitObserveRecoveryActions,
    remaining_unknowns: RecoveryPlanUnknowns,
}
recovery_plan_digest_record!(RepositoryCommitObserveRecoveryPlanDigestRecord);
recovery_plan_leaf!(
    RepositoryCommitObserveRecoveryPlanStatus,
    RepositoryCommitObserveRecoveryPlanDigestRecord
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RepositoryCommitCommittedRecoveryPlanDigestRecord {
    prior_operation_id: OperationId,
    target: RepositoryCommitRecoveryTarget,
    effect_class: ReconcileOnlyRecoveryEffectClass,
    repository_commit_stage: CommittedCommitRecoveryStage,
    planned_result_phase: TaskPhase,
    observations: RecoveryPlanObservations,
    actions: RepositoryCommitCommittedRecoveryActions,
    remaining_unknowns: RecoveryPlanUnknowns,
}
recovery_plan_digest_record!(RepositoryCommitCommittedRecoveryPlanDigestRecord);
recovery_plan_leaf!(
    RepositoryCommitCommittedRecoveryPlanStatus,
    RepositoryCommitCommittedRecoveryPlanDigestRecord
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RepositoryCommitNotCommittedRecoveryPlanDigestRecord {
    prior_operation_id: OperationId,
    target: RepositoryCommitRecoveryTarget,
    effect_class: RollbackRecoveryEffectClass,
    repository_commit_stage: NotCommittedCommitRecoveryStage,
    planned_result_phase: TaskPhase,
    observations: RecoveryPlanObservations,
    actions: RepositoryCommitNotCommittedRecoveryActions,
    remaining_unknowns: RecoveryPlanUnknowns,
}
recovery_plan_digest_record!(RepositoryCommitNotCommittedRecoveryPlanDigestRecord);
recovery_plan_leaf!(
    RepositoryCommitNotCommittedRecoveryPlanStatus,
    RepositoryCommitNotCommittedRecoveryPlanDigestRecord
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ArmedSupportRecoveryPlanDigestRecord {
    prior_operation_id: OperationId,
    target: SupportPrerequisiteRecoveryTarget,
    effect_class: ReconcileOnlyRecoveryEffectClass,
    planned_result_phase: TaskPhase,
    observations: RecoveryPlanObservations,
    actions: SupportPrerequisiteRecoveryActions,
    support_version_observations: SupportRecoveryVersionObservations,
    support_version_observation_digest: Sha256Digest,
    support_history_from_cursor: RepositoryHistoryCursor,
    support_history_through_cursor: RepositoryHistoryCursor,
    support_history_partition: ValidatedRepositoryHistoryPartition,
    support_recovery_disposition: SupportRecoveryDisposition,
    support_late_relevant_result_phase: TaskPhase,
    #[serde(skip_serializing_if = "Option::is_none")]
    successful_integration_forbidden: Option<TrueLiteral>,
    support_recovery_finalization_plan: SupportRecoveryFinalizationPlan,
    #[serde(skip_serializing_if = "Option::is_none")]
    latest_support_recovery_guard_proof: Option<SupportRecoveryGuardProof>,
    #[serde(skip_serializing_if = "Option::is_none")]
    manual_working_infobase_closure_plan: Option<ManualWorkingInfobaseClosurePlan>,
    manual_target_mode: ManualSupportTargetMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    required_external_action: Option<SupportRecoveryExternalAction>,
    remaining_unknowns: RecoveryPlanUnknowns,
}
recovery_plan_digest_record!(ArmedSupportRecoveryPlanDigestRecord);

macro_rules! armed_support_exact_actions_schema {
    ($name:ident $(, $wait:ty)?) => {
        #[allow(dead_code)]
        struct $name;

        impl JsonSchema for $name {
            fn schema_name() -> Cow<'static, str> {
                stringify!($name).into()
            }

            fn json_schema(generator: &mut SchemaGenerator) -> Schema {
                let mut items = vec![
                    generator.subschema_for::<ObserveSupportPrerequisiteHistoryAction>(),
                ];
                $(items.push(generator.subschema_for::<$wait>());)?
                items.push(
                    generator.subschema_for::<FinalizeSupportPrerequisiteRecoveryAction>(),
                );
                exact_action_array_schema(items)
            }
        }
    };
}

armed_support_exact_actions_schema!(ArmedSupportNoExternalWaitActionsSchema);
armed_support_exact_actions_schema!(
    ArmedSupportCorrectionWaitActionsSchema,
    AwaitExternalSupportCorrectionAction
);
armed_support_exact_actions_schema!(
    ArmedSupportLockReleaseWaitActionsSchema,
    AwaitExternalLockReleaseAction
);
armed_support_exact_actions_schema!(
    ArmedSupportWorkingInfobaseClosureWaitActionsSchema,
    AwaitManualWorkingInfobaseClosureAction
);
armed_support_exact_actions_schema!(
    ArmedSupportReservedOriginalClosureWaitActionsSchema,
    AwaitReservedOriginalClosureAction
);
armed_support_exact_actions_schema!(
    ArmedSupportConflictWaitActionsSchema,
    AwaitExternalSupportConflictResolutionAction
);
armed_support_exact_actions_schema!(
    ArmedSupportEvidenceWaitActionsSchema,
    AwaitSupportRecoveryEvidenceAction
);

#[allow(dead_code)]
struct ReservedBlockedSupportRecoveryGuardProofSchema;

impl JsonSchema for ReservedBlockedSupportRecoveryGuardProofSchema {
    fn schema_name() -> Cow<'static, str> {
        "ReservedBlockedSupportRecoveryGuardProofSchema".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<ReservedBlockedBeforeRootGuardProofSchema>(),
            generator.subschema_for::<ReservedBlockedAfterPartialGuardProofSchema>(),
        ])
    }
}

#[allow(dead_code)]
struct SeparateBlockedSupportRecoveryGuardProofSchema;

impl JsonSchema for SeparateBlockedSupportRecoveryGuardProofSchema {
    fn schema_name() -> Cow<'static, str> {
        "SeparateBlockedSupportRecoveryGuardProofSchema".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<SeparateBlockedBeforeRootGuardProofSchema>(),
            generator.subschema_for::<SeparateBlockedAfterPartialGuardProofSchema>(),
        ])
    }
}

macro_rules! reserved_armed_support_schema_branch {
    ($name:ident, $actions:ty $(, external = $external:ty)? $(, proof = $proof:ty)?) => {
        #[allow(dead_code)]
        #[derive(JsonSchema)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct $name {
            prior_operation_id: OperationId,
            target: SupportPrerequisiteRecoveryTarget,
            effect_class: ReconcileOnlyRecoveryEffectClass,
            planned_result_phase: TaskPhase,
            observations: RecoveryPlanObservations,
            actions: $actions,
            support_version_observations: SupportRecoveryVersionObservations,
            support_version_observation_digest: Sha256Digest,
            support_history_from_cursor: RepositoryHistoryCursor,
            support_history_through_cursor: RepositoryHistoryCursor,
            support_history_partition: ValidatedRepositoryHistoryPartition,
            support_recovery_disposition: SupportRecoveryDisposition,
            support_late_relevant_result_phase: TaskPhase,
            #[serde(skip_serializing_if = "Option::is_none")]
            successful_integration_forbidden: Option<TrueLiteral>,
            support_recovery_finalization_plan: SupportRecoveryFinalizationPlan,
            $(latest_support_recovery_guard_proof: $proof,)?
            manual_target_mode: ReservedOriginalModeLiteral,
            $(required_external_action: $external,)?
            remaining_unknowns: RecoveryPlanUnknowns,
            recovery_digest: Sha256Digest,
        }
    };
}

macro_rules! separate_armed_support_schema_branch {
    ($name:ident, $actions:ty $(, external = $external:ty)? $(, proof = $proof:ty)?) => {
        #[allow(dead_code)]
        #[derive(JsonSchema)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct $name {
            prior_operation_id: OperationId,
            target: SupportPrerequisiteRecoveryTarget,
            effect_class: ReconcileOnlyRecoveryEffectClass,
            planned_result_phase: TaskPhase,
            observations: RecoveryPlanObservations,
            actions: $actions,
            support_version_observations: SupportRecoveryVersionObservations,
            support_version_observation_digest: Sha256Digest,
            support_history_from_cursor: RepositoryHistoryCursor,
            support_history_through_cursor: RepositoryHistoryCursor,
            support_history_partition: ValidatedRepositoryHistoryPartition,
            support_recovery_disposition: SupportRecoveryDisposition,
            support_late_relevant_result_phase: TaskPhase,
            #[serde(skip_serializing_if = "Option::is_none")]
            successful_integration_forbidden: Option<TrueLiteral>,
            support_recovery_finalization_plan: SupportRecoveryFinalizationPlan,
            $(latest_support_recovery_guard_proof: $proof,)?
            manual_working_infobase_closure_plan: ManualWorkingInfobaseClosurePlan,
            manual_target_mode: SeparateWorkingInfobaseModeLiteral,
            $(required_external_action: $external,)?
            remaining_unknowns: RecoveryPlanUnknowns,
            recovery_digest: Sha256Digest,
        }
    };
}

reserved_armed_support_schema_branch!(
    ReservedArmedSupportWithoutExternalWaitStatusSchema,
    ArmedSupportNoExternalWaitActionsSchema
);
reserved_armed_support_schema_branch!(
    ReservedArmedSupportCorrectionWaitStatusSchema,
    ArmedSupportCorrectionWaitActionsSchema,
    external = ReservedOriginalSupportCorrectiveInstructionSchema
);
reserved_armed_support_schema_branch!(
    ReservedArmedSupportLockReleaseWaitStatusSchema,
    ArmedSupportLockReleaseWaitActionsSchema,
    external = ReleaseRepositoryLocksInstruction,
    proof = ReservedBlockedSupportRecoveryGuardProofSchema
);
reserved_armed_support_schema_branch!(
    ReservedArmedSupportClosureWaitStatusSchema,
    ArmedSupportReservedOriginalClosureWaitActionsSchema,
    external = CloseReservedOriginalDesignerInstruction,
    proof = ReservedStoppedAfterCompleteGuardProofSchema
);
reserved_armed_support_schema_branch!(
    ReservedArmedSupportConflictWaitStatusSchema,
    ArmedSupportConflictWaitActionsSchema,
    external = SupportConflictInstruction
);
reserved_armed_support_schema_branch!(
    ReservedArmedSupportEvidenceWaitStatusSchema,
    ArmedSupportEvidenceWaitActionsSchema,
    external = SupportEvidenceInstruction
);

separate_armed_support_schema_branch!(
    SeparateArmedSupportWithoutExternalWaitStatusSchema,
    ArmedSupportNoExternalWaitActionsSchema
);
separate_armed_support_schema_branch!(
    SeparateArmedSupportCorrectionWaitStatusSchema,
    ArmedSupportCorrectionWaitActionsSchema,
    external = SeparateWorkingInfobaseSupportCorrectiveInstructionSchema
);
separate_armed_support_schema_branch!(
    SeparateArmedSupportLockReleaseWaitStatusSchema,
    ArmedSupportLockReleaseWaitActionsSchema,
    external = ReleaseRepositoryLocksInstruction,
    proof = SeparateBlockedSupportRecoveryGuardProofSchema
);
separate_armed_support_schema_branch!(
    SeparateArmedSupportClosureWaitStatusSchema,
    ArmedSupportWorkingInfobaseClosureWaitActionsSchema,
    external = CleanManualWorkingInfobaseInstruction,
    proof = SeparateStoppedAfterCompleteGuardProofSchema
);
separate_armed_support_schema_branch!(
    SeparateArmedSupportConflictWaitStatusSchema,
    ArmedSupportConflictWaitActionsSchema,
    external = SupportConflictInstruction
);
separate_armed_support_schema_branch!(
    SeparateArmedSupportEvidenceWaitStatusSchema,
    ArmedSupportEvidenceWaitActionsSchema,
    external = SupportEvidenceInstruction
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ArmedSupportRecoveryPlanStatus {
    #[serde(flatten)]
    record: ArmedSupportRecoveryPlanDigestRecord,
    recovery_digest: Sha256Digest,
}

impl ArmedSupportRecoveryPlanStatus {
    fn new(record: ArmedSupportRecoveryPlanDigestRecord) -> Result<Self, RecoveryContractError> {
        let recovery_digest = contract_digest(&record, "recovery plan digest failed")?;
        Ok(Self {
            record,
            recovery_digest,
        })
    }
}

impl JsonSchema for ArmedSupportRecoveryPlanStatus {
    fn schema_name() -> Cow<'static, str> {
        "ArmedSupportRecoveryPlanStatus".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<ReservedArmedSupportWithoutExternalWaitStatusSchema>(),
            generator.subschema_for::<ReservedArmedSupportCorrectionWaitStatusSchema>(),
            generator.subschema_for::<ReservedArmedSupportLockReleaseWaitStatusSchema>(),
            generator.subschema_for::<ReservedArmedSupportClosureWaitStatusSchema>(),
            generator.subschema_for::<ReservedArmedSupportConflictWaitStatusSchema>(),
            generator.subschema_for::<ReservedArmedSupportEvidenceWaitStatusSchema>(),
            generator.subschema_for::<SeparateArmedSupportWithoutExternalWaitStatusSchema>(),
            generator.subschema_for::<SeparateArmedSupportCorrectionWaitStatusSchema>(),
            generator.subschema_for::<SeparateArmedSupportLockReleaseWaitStatusSchema>(),
            generator.subschema_for::<SeparateArmedSupportClosureWaitStatusSchema>(),
            generator.subschema_for::<SeparateArmedSupportConflictWaitStatusSchema>(),
            generator.subschema_for::<SeparateArmedSupportEvidenceWaitStatusSchema>(),
        ])
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PreArmObserveRecoveryPlanDigestRecord {
    prior_operation_id: OperationId,
    target: PreArmSupportCancellationRecoveryTarget,
    effect_class: ReconcileOnlyRecoveryEffectClass,
    pre_arm_cancellation_stage: ObserveOutcomePreArmRecoveryStage,
    planned_result_phase: TaskPhase,
    observations: RecoveryPlanObservations,
    actions: PreArmObserveRecoveryActions,
    remaining_unknowns: RecoveryPlanUnknowns,
}
recovery_plan_digest_record!(PreArmObserveRecoveryPlanDigestRecord);
recovery_plan_leaf!(
    PreArmObserveRecoveryPlanStatus,
    PreArmObserveRecoveryPlanDigestRecord
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PreArmFinalizeRecoveryPlanDigestRecord {
    prior_operation_id: OperationId,
    target: PreArmSupportCancellationRecoveryTarget,
    effect_class: ReconcileOnlyRecoveryEffectClass,
    pre_arm_cancellation_stage: FinalizePreArmRecoveryStage,
    planned_result_phase: TaskPhase,
    observations: RecoveryPlanObservations,
    actions: PreArmFinalizeRecoveryActions,
    pre_arm_cancellation_effect_observation: PreArmCancellationEffectObservation,
    pre_arm_cancellation_finalization_plan: PreArmCancellationFinalizationPlan,
    #[serde(skip_serializing_if = "Option::is_none")]
    pre_arm_cancellation_known_blocker: Option<PreArmCancellationKnownBlocker>,
    remaining_unknowns: RecoveryPlanUnknowns,
}
recovery_plan_digest_record!(PreArmFinalizeRecoveryPlanDigestRecord);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PreArmFinalizeRecoveryPlanStatus {
    #[serde(flatten)]
    record: PreArmFinalizeRecoveryPlanDigestRecord,
    pre_arm_cancellation_finalization_progress: PreArmCancellationFinalizationAttemptProgress,
    recovery_digest: Sha256Digest,
}

impl PreArmFinalizeRecoveryPlanStatus {
    fn new(
        record: PreArmFinalizeRecoveryPlanDigestRecord,
        pre_arm_cancellation_finalization_progress: PreArmCancellationFinalizationAttemptProgress,
    ) -> Result<Self, RecoveryContractError> {
        let recovery_digest = contract_digest(&record, "pre-arm recovery plan digest failed")?;
        Ok(Self {
            record,
            pre_arm_cancellation_finalization_progress,
            recovery_digest,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ManualWorkingInfobaseLeaseRecoveryPlanDigestRecord {
    prior_operation_id: OperationId,
    target: ManualWorkingInfobaseLeaseRecoveryTarget,
    effect_class: ReconcileOnlyRecoveryEffectClass,
    planned_result_phase: TaskPhase,
    observations: RecoveryPlanObservations,
    actions: ManualWorkingInfobaseLeaseRecoveryActions,
    remaining_unknowns: RecoveryPlanUnknowns,
}
recovery_plan_digest_record!(ManualWorkingInfobaseLeaseRecoveryPlanDigestRecord);
recovery_plan_leaf!(
    ManualWorkingInfobaseLeaseRecoveryPlanStatus,
    ManualWorkingInfobaseLeaseRecoveryPlanDigestRecord
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ArtifactRecoveryPlanDigestRecord {
    prior_operation_id: OperationId,
    target: ArtifactRecoveryTarget,
    effect_class: QuarantineRecoveryEffectClass,
    planned_result_phase: TaskPhase,
    observations: RecoveryPlanObservations,
    actions: ArtifactRecoveryActions,
    remaining_unknowns: RecoveryPlanUnknowns,
}
recovery_plan_digest_record!(ArtifactRecoveryPlanDigestRecord);
recovery_plan_leaf!(ArtifactRecoveryPlanStatus, ArtifactRecoveryPlanDigestRecord);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ArchiveRecoveryPlanDigestRecord {
    prior_operation_id: OperationId,
    target: ArchiveRecoveryTarget,
    effect_class: CleanupRecoveryEffectClass,
    planned_result_phase: TaskPhase,
    observations: RecoveryPlanObservations,
    actions: ArchiveRecoveryActions,
    remaining_unknowns: RecoveryPlanUnknowns,
}
recovery_plan_digest_record!(ArchiveRecoveryPlanDigestRecord);
recovery_plan_leaf!(ArchiveRecoveryPlanStatus, ArchiveRecoveryPlanDigestRecord);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CleanupRecoveryPlanDigestRecord {
    prior_operation_id: OperationId,
    target: CleanupRecoveryTarget,
    effect_class: CleanupRecoveryEffectClass,
    planned_result_phase: TaskPhase,
    observations: RecoveryPlanObservations,
    actions: CleanupRecoveryActions,
    remaining_unknowns: RecoveryPlanUnknowns,
}
recovery_plan_digest_record!(CleanupRecoveryPlanDigestRecord);
recovery_plan_leaf!(CleanupRecoveryPlanStatus, CleanupRecoveryPlanDigestRecord);

fn validate_support_recovery_action_shape(
    actions: &[RecoveryAction],
) -> Result<(), RecoveryContractError> {
    let Some((first, remainder)) = actions.split_first() else {
        return Err(RecoveryContractError("support recovery actions are empty"));
    };
    let Some((last, middle)) = remainder.split_last() else {
        return Err(RecoveryContractError(
            "support recovery requires history observation and finalization",
        ));
    };
    if !matches!(
        first,
        RecoveryAction(RecoveryActionKindWire::ObserveSupportPrerequisiteHistory(_))
    ) || !matches!(
        last,
        RecoveryAction(RecoveryActionKindWire::FinalizeSupportPrerequisiteRecovery(
            _
        ))
    ) {
        return Err(RecoveryContractError(
            "support recovery history and finalization are not in their terminal positions",
        ));
    }

    let mut seen = [false; 11];
    let mut previous_rank = 0;
    let mut external_waits = 0;
    let mut has_working_infobase_action = false;
    let mut has_reserved_original_action = false;
    for action in middle {
        let (rank, key, is_external_wait, is_working, is_reserved) = match &action.0 {
            RecoveryActionKindWire::AwaitExternalSupportCorrection(_) => (0, 0, true, false, false),
            RecoveryActionKindWire::AwaitExternalLockRelease(_) => (0, 1, true, false, false),
            RecoveryActionKindWire::AwaitExternalSupportConflictResolution(_) => {
                (0, 2, true, false, false)
            }
            RecoveryActionKindWire::AwaitSupportRecoveryEvidence(_) => (0, 3, true, false, false),
            RecoveryActionKindWire::ObserveWorkingInfobaseLease(_) => (1, 4, false, true, false),
            RecoveryActionKindWire::ObserveReservedOriginalLease(_) => (1, 5, false, false, true),
            RecoveryActionKindWire::AwaitManualWorkingInfobaseClosure(_) => {
                (2, 6, true, true, false)
            }
            RecoveryActionKindWire::AwaitReservedOriginalClosure(_) => (2, 7, true, false, true),
            RecoveryActionKindWire::ReleaseWorkingInfobaseLease(_) => (3, 8, false, true, false),
            RecoveryActionKindWire::ReleaseReservedOriginalLease(_) => (3, 9, false, false, true),
            RecoveryActionKindWire::UpdateOriginalSelectedTargets(_) => {
                (4, 10, false, false, false)
            }
            _ => {
                return Err(RecoveryContractError(
                    "support recovery contains an action from another target row",
                ));
            }
        };
        if rank < previous_rank || seen[key] {
            return Err(RecoveryContractError(
                "support recovery actions are duplicated or out of canonical order",
            ));
        }
        previous_rank = rank;
        seen[key] = true;
        external_waits += usize::from(is_external_wait);
        has_working_infobase_action |= is_working;
        has_reserved_original_action |= is_reserved;
    }
    if external_waits > 1 {
        return Err(RecoveryContractError(
            "support recovery has more than one required external wait",
        ));
    }
    if has_working_infobase_action && has_reserved_original_action {
        return Err(RecoveryContractError(
            "support recovery mixes manual target modes",
        ));
    }
    Ok(())
}

fn validate_prearm_finalize_action_shape(
    actions: &[RecoveryAction],
) -> Result<(), RecoveryContractError> {
    let mut seen = [false; 8];
    let mut previous_rank = 0;
    for action in actions {
        let rank = match &action.0 {
            RecoveryActionKindWire::AcquirePreArmRootGuard(_) => 0,
            RecoveryActionKindWire::AcquirePreArmModeLease(_) => 1,
            RecoveryActionKindWire::RecheckPreArmCancellationFinalization(_) => 2,
            RecoveryActionKindWire::ApplyPreArmCancellationSelectiveUpdate(_) => 3,
            RecoveryActionKindWire::PersistPreArmSupportCancellation(_) => 4,
            RecoveryActionKindWire::ReleasePreArmModeLease(_) => 5,
            RecoveryActionKindWire::ReleasePreArmRootGuard(_) => 6,
            RecoveryActionKindWire::FinishPreArmCancellationRecovery(_) => 7,
            _ => {
                return Err(RecoveryContractError(
                    "pre-arm finalization contains an action from another target row",
                ));
            }
        };
        if rank < previous_rank || seen[rank] {
            return Err(RecoveryContractError(
                "pre-arm finalization actions are duplicated or out of canonical order",
            ));
        }
        previous_rank = rank;
        seen[rank] = true;
    }
    if !seen[2]
        || !seen[7]
        || !matches!(
            actions.last(),
            Some(RecoveryAction(
                RecoveryActionKindWire::FinishPreArmCancellationRecovery(_)
            ))
        )
    {
        return Err(RecoveryContractError(
            "pre-arm finalization requires one recheck and one final action last",
        ));
    }
    Ok(())
}

fn validate_support_recovery_action_grammar(
    actions: &[RecoveryAction],
    mode: ManualSupportTargetMode,
    disposition: SupportRecoveryDisposition,
    finalization_plan_digest: &Sha256Digest,
) -> Result<(), RecoveryContractError> {
    validate_support_recovery_action_shape(actions)?;
    let Some((first, remainder)) = actions.split_first() else {
        return Err(RecoveryContractError("support recovery actions are empty"));
    };
    let Some((last, middle)) = remainder.split_last() else {
        return Err(RecoveryContractError(
            "support recovery requires history observation and finalization",
        ));
    };
    let RecoveryAction(RecoveryActionKindWire::ObserveSupportPrerequisiteHistory(history)) = first
    else {
        return Err(RecoveryContractError(
            "support recovery must observe its history first",
        ));
    };
    let RecoveryAction(RecoveryActionKindWire::FinalizeSupportPrerequisiteRecovery(finalize)) =
        last
    else {
        return Err(RecoveryContractError(
            "support recovery must finalize exactly once and last",
        ));
    };
    let expected_outcome = match disposition {
        SupportRecoveryDisposition::RestoreThenReauthorize
        | SupportRecoveryDisposition::PreserveExternalAndReauthorize => {
            FinalizeSupportAuthorizationOutcome::Cancelled
        }
        SupportRecoveryDisposition::RestoreThenAbandon => {
            FinalizeSupportAuthorizationOutcome::AbandonmentFinalized
        }
    };
    if history.support_action_id != finalize.support_action_id
        || &finalize.finalization_plan_digest != finalization_plan_digest
        || finalize.authorization_outcome != expected_outcome
    {
        return Err(RecoveryContractError(
            "support recovery finalization does not bind its history, plan, or disposition",
        ));
    }

    for action in middle {
        let legal = matches!(
            (&action.0, mode),
            (
                RecoveryActionKindWire::ObserveWorkingInfobaseLease(_)
                    | RecoveryActionKindWire::ReleaseWorkingInfobaseLease(_)
                    | RecoveryActionKindWire::AwaitManualWorkingInfobaseClosure(_),
                ManualSupportTargetMode::SeparateWorkingInfobase,
            ) | (
                RecoveryActionKindWire::ObserveReservedOriginalLease(_)
                    | RecoveryActionKindWire::ReleaseReservedOriginalLease(_)
                    | RecoveryActionKindWire::AwaitReservedOriginalClosure(_),
                ManualSupportTargetMode::ReservedOriginal,
            ) | (
                RecoveryActionKindWire::AwaitExternalSupportCorrection(_)
                    | RecoveryActionKindWire::AwaitExternalLockRelease(_)
                    | RecoveryActionKindWire::AwaitExternalSupportConflictResolution(_)
                    | RecoveryActionKindWire::AwaitSupportRecoveryEvidence(_)
                    | RecoveryActionKindWire::UpdateOriginalSelectedTargets(_),
                _,
            )
        );
        if !legal {
            return Err(RecoveryContractError(
                "support recovery action is illegal for its manual target mode",
            ));
        }
    }

    Ok(())
}

fn validate_prearm_observe_action_grammar(
    prior_operation_id: &OperationId,
    actions: &[RecoveryAction],
) -> Result<(), RecoveryContractError> {
    let [RecoveryAction(RecoveryActionKindWire::ObservePreArmCancellationOutcome(action))] =
        actions
    else {
        return Err(RecoveryContractError(
            "pre-arm observation stage requires exactly its outcome observation action",
        ));
    };
    if &action.prior_operation_id != prior_operation_id {
        return Err(RecoveryContractError(
            "pre-arm outcome observation belongs to another operation",
        ));
    }
    Ok(())
}

fn validate_prearm_finalize_action_grammar(
    observation: &PreArmCancellationEffectObservation,
    plan: &PreArmCancellationFinalizationPlan,
    progress: &PreArmCancellationFinalizationAttemptProgress,
    actions: &[RecoveryAction],
) -> Result<(), RecoveryContractError> {
    validate_prearm_finalize_action_shape(actions)?;
    if !plan.binds_effect_observation(observation)
        || progress.finalization_attempt_id() != plan.finalization_attempt_id()
    {
        return Err(RecoveryContractError(
            "pre-arm finalization observation, plan, and progress bindings disagree",
        ));
    }

    let success_path = plan
        .execution_path_plan()
        .paths()
        .iter()
        .find(|path| path.path_kind() == PreArmCancellationFinalizationExecutionPathKind::Success)
        .ok_or(RecoveryContractError(
            "pre-arm finalization plan lacks its success path",
        ))?;

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum ExpectedActionKind {
        AcquireRoot,
        AcquireMode,
        Recheck,
        Apply,
        Persist,
        ReleaseMode,
        ReleaseRoot,
        Finish,
    }

    let receipt_plan = plan.receipt_plan();
    let mut expected_kinds = Vec::with_capacity(8);
    if receipt_plan.root_guard_acquisition_receipt().source()
        == PreArmCancellationReceiptSource::FinalizationPlan
    {
        expected_kinds.push(ExpectedActionKind::AcquireRoot);
    }
    if receipt_plan.mode_lease_acquisition_receipt().source()
        == PreArmCancellationReceiptSource::FinalizationPlan
    {
        expected_kinds.push(ExpectedActionKind::AcquireMode);
    }
    expected_kinds.push(ExpectedActionKind::Recheck);
    if receipt_plan
        .selective_update_effect_receipt()
        .is_some_and(|receipt| {
            receipt.source() == PreArmCancellationReceiptSource::FinalizationPlan
        })
    {
        expected_kinds.push(ExpectedActionKind::Apply);
    }
    if receipt_plan.cancellation_persistence_receipt().source()
        == PreArmCancellationReceiptSource::FinalizationPlan
    {
        expected_kinds.push(ExpectedActionKind::Persist);
    }
    if receipt_plan.mode_lease_release_receipt().source()
        == PreArmCancellationReceiptSource::FinalizationPlan
    {
        expected_kinds.push(ExpectedActionKind::ReleaseMode);
    }
    if receipt_plan.root_guard_release_receipt().source()
        == PreArmCancellationReceiptSource::FinalizationPlan
    {
        expected_kinds.push(ExpectedActionKind::ReleaseRoot);
    }
    expected_kinds.push(ExpectedActionKind::Finish);

    let action_kind = |action: &RecoveryAction| match &action.0 {
        RecoveryActionKindWire::AcquirePreArmRootGuard(_) => ExpectedActionKind::AcquireRoot,
        RecoveryActionKindWire::AcquirePreArmModeLease(_) => ExpectedActionKind::AcquireMode,
        RecoveryActionKindWire::RecheckPreArmCancellationFinalization(_) => {
            ExpectedActionKind::Recheck
        }
        RecoveryActionKindWire::ApplyPreArmCancellationSelectiveUpdate(_) => {
            ExpectedActionKind::Apply
        }
        RecoveryActionKindWire::PersistPreArmSupportCancellation(_) => ExpectedActionKind::Persist,
        RecoveryActionKindWire::ReleasePreArmModeLease(_) => ExpectedActionKind::ReleaseMode,
        RecoveryActionKindWire::ReleasePreArmRootGuard(_) => ExpectedActionKind::ReleaseRoot,
        RecoveryActionKindWire::FinishPreArmCancellationRecovery(_) => ExpectedActionKind::Finish,
        _ => unreachable!("shape validation excludes non-pre-arm actions"),
    };
    if success_path.action_ids().len() != actions.len()
        || expected_kinds.len() != actions.len()
        || success_path
            .action_ids()
            .iter()
            .zip(expected_kinds.iter())
            .zip(actions)
            .any(|((expected_id, expected_kind), action)| {
                expected_id != action.action_id() || *expected_kind != action_kind(action)
            })
    {
        return Err(RecoveryContractError(
            "pre-arm recovery actions do not equal the finalization success catalog",
        ));
    }

    let receipt_action_matches = |action: &RecoveryAction,
                                  expected_ref: &PreArmCancellationReceiptRef,
                                  expected_kind: PreArmCancellationEffectKind|
     -> Result<bool, RecoveryContractError> {
        let Some((actual_ref, actual_kind)) = action.prearm_receipt_binding() else {
            return Ok(false);
        };
        if actual_ref != expected_ref || actual_kind != expected_kind {
            return Ok(false);
        }
        plan.validates_finalization_action_receipt_intent(
            actual_ref,
            expected_kind,
            action.expected_postcondition_digest(),
        )
        .map_err(|_| RecoveryContractError("pre-arm action receipt intent validation failed"))
    };

    for action in actions {
        let matches = match &action.0 {
            RecoveryActionKindWire::AcquirePreArmRootGuard(value) => {
                &value.finalization_attempt_id == plan.finalization_attempt_id()
                    && &value.finalization_plan_digest == plan.finalization_plan_digest()
                    && &value.support_action_id == plan.support_action_id()
                    && receipt_action_matches(
                        action,
                        receipt_plan.root_guard_acquisition_receipt(),
                        PreArmCancellationEffectKind::RootGuardAcquire,
                    )?
            }
            RecoveryActionKindWire::AcquirePreArmModeLease(value) => match &value.0 {
                AcquirePreArmModeLeaseActionKindWire::ReservedOriginal(value) => {
                    plan.manual_target_mode() == ManualSupportTargetMode::ReservedOriginal
                        && &value.finalization_attempt_id == plan.finalization_attempt_id()
                        && &value.finalization_plan_digest == plan.finalization_plan_digest()
                        && &value.support_action_id == plan.support_action_id()
                        && receipt_action_matches(
                            action,
                            receipt_plan.mode_lease_acquisition_receipt(),
                            PreArmCancellationEffectKind::ModeLeaseAcquire,
                        )?
                }
                AcquirePreArmModeLeaseActionKindWire::SeparateWorkingInfobase(value) => {
                    plan.manual_target_mode() == ManualSupportTargetMode::SeparateWorkingInfobase
                        && &value.finalization_attempt_id == plan.finalization_attempt_id()
                        && &value.finalization_plan_digest == plan.finalization_plan_digest()
                        && &value.support_action_id == plan.support_action_id()
                        && receipt_action_matches(
                            action,
                            receipt_plan.mode_lease_acquisition_receipt(),
                            PreArmCancellationEffectKind::ModeLeaseAcquire,
                        )?
                }
            },
            RecoveryActionKindWire::RecheckPreArmCancellationFinalization(value) => {
                &value.finalization_attempt_id == plan.finalization_attempt_id()
                    && &value.finalization_plan_digest == plan.finalization_plan_digest()
                    && &value.effect_observation_digest == plan.effect_observation_digest()
                    && &value.recheck_policy_digest == plan.recheck_policy().policy_digest()
            }
            RecoveryActionKindWire::ApplyPreArmCancellationSelectiveUpdate(value) => {
                let common_matches = &value.finalization_attempt_id
                    == plan.finalization_attempt_id()
                    && &value.finalization_plan_digest == plan.finalization_plan_digest()
                    && &value.selective_update_plan_digest == plan.selective_update_plan_digest()
                    && &value.expected_target_revision_map_digest
                        == plan.expected_target_revision_map_digest();
                common_matches
                    && match receipt_plan.selective_update_effect_receipt() {
                        Some(expected_ref) => receipt_action_matches(
                            action,
                            expected_ref,
                            PreArmCancellationEffectKind::SelectiveOriginalUpdate,
                        )?,
                        None => false,
                    }
            }
            RecoveryActionKindWire::PersistPreArmSupportCancellation(value) => {
                &value.finalization_attempt_id == plan.finalization_attempt_id()
                    && &value.support_action_id == plan.support_action_id()
                    && &value.expected_support_action_digest
                        == plan.expected_support_action_digest()
                    && &value.approved_cancellation_digest == plan.approved_cancellation_digest()
                    && &value.effect_observation_digest == plan.effect_observation_digest()
                    && &value.finalization_plan_digest == plan.finalization_plan_digest()
                    && receipt_action_matches(
                        action,
                        receipt_plan.cancellation_persistence_receipt(),
                        PreArmCancellationEffectKind::AuthorizationCancellation,
                    )?
            }
            RecoveryActionKindWire::ReleasePreArmModeLease(value) => match &value.0 {
                ReleasePreArmModeLeaseActionKindWire::ReservedOriginal(value) => {
                    plan.manual_target_mode() == ManualSupportTargetMode::ReservedOriginal
                        && &value.finalization_attempt_id == plan.finalization_attempt_id()
                        && &value.finalization_plan_digest == plan.finalization_plan_digest()
                        && receipt_action_matches(
                            action,
                            receipt_plan.mode_lease_release_receipt(),
                            PreArmCancellationEffectKind::ModeLeaseRelease,
                        )?
                }
                ReleasePreArmModeLeaseActionKindWire::SeparateWorkingInfobase(value) => {
                    plan.manual_target_mode() == ManualSupportTargetMode::SeparateWorkingInfobase
                        && &value.finalization_attempt_id == plan.finalization_attempt_id()
                        && &value.finalization_plan_digest == plan.finalization_plan_digest()
                        && receipt_action_matches(
                            action,
                            receipt_plan.mode_lease_release_receipt(),
                            PreArmCancellationEffectKind::ModeLeaseRelease,
                        )?
                }
            },
            RecoveryActionKindWire::ReleasePreArmRootGuard(value) => {
                &value.finalization_attempt_id == plan.finalization_attempt_id()
                    && &value.finalization_plan_digest == plan.finalization_plan_digest()
                    && &value.support_action_id == plan.support_action_id()
                    && receipt_action_matches(
                        action,
                        receipt_plan.root_guard_release_receipt(),
                        PreArmCancellationEffectKind::RootGuardRelease,
                    )?
            }
            RecoveryActionKindWire::FinishPreArmCancellationRecovery(value) => {
                &value.finalization_attempt_id == plan.finalization_attempt_id()
                    && &value.support_action_id == plan.support_action_id()
                    && &value.expected_support_action_digest
                        == plan.expected_support_action_digest()
                    && &value.approved_cancellation_digest == plan.approved_cancellation_digest()
                    && &value.effect_observation_digest == plan.effect_observation_digest()
                    && &value.finalization_plan_digest == plan.finalization_plan_digest()
                    && &value.receipt_plan_digest == plan.receipt_plan().receipt_plan_digest()
                    && value.expected_result_phase == plan.planned_result_phase()
                    && receipt_action_matches(
                        action,
                        receipt_plan.recovery_finalization_receipt(),
                        PreArmCancellationEffectKind::RecoveryFinalization,
                    )?
            }
            _ => false,
        };
        if !matches {
            return Err(RecoveryContractError(
                "pre-arm recovery action fields do not equal the finalization plan",
            ));
        }
    }

    let acquired_mode = actions.iter().find_map(|action| match &action.0 {
        RecoveryActionKindWire::AcquirePreArmModeLease(value) => Some(value),
        _ => None,
    });
    let released_mode = actions.iter().find_map(|action| match &action.0 {
        RecoveryActionKindWire::ReleasePreArmModeLease(value) => Some(value),
        _ => None,
    });
    if let (Some(acquired), Some(released)) = (acquired_mode, released_mode) {
        let same_lease_window = match (&acquired.0, &released.0) {
            (
                AcquirePreArmModeLeaseActionKindWire::ReservedOriginal(acquired),
                ReleasePreArmModeLeaseActionKindWire::ReservedOriginal(released),
            ) => {
                acquired.reserved_original_identity_digest
                    == released.reserved_original_identity_digest
                    && acquired.exclusive_lease_capability_id
                        == released.exclusive_lease_capability_id
            }
            (
                AcquirePreArmModeLeaseActionKindWire::SeparateWorkingInfobase(acquired),
                ReleasePreArmModeLeaseActionKindWire::SeparateWorkingInfobase(released),
            ) => {
                acquired.working_infobase_identity == released.working_infobase_identity
                    && acquired.exclusive_lease_capability_id
                        == released.exclusive_lease_capability_id
            }
            _ => false,
        };
        if !same_lease_window {
            return Err(RecoveryContractError(
                "pre-arm acquire/release actions describe different mode-lease windows",
            ));
        }
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
enum RecoveryPlanStatusKind {
    TaskConfiguration(TaskConfigurationRecoveryPlanStatus),
    RepositoryLocks(RepositoryLocksRecoveryPlanStatus),
    OriginalConfiguration(OriginalConfigurationRecoveryPlanStatus),
    RepositoryCommitObserve(RepositoryCommitObserveRecoveryPlanStatus),
    RepositoryCommitCommitted(RepositoryCommitCommittedRecoveryPlanStatus),
    RepositoryCommitNotCommitted(RepositoryCommitNotCommittedRecoveryPlanStatus),
    SupportPrerequisite(Box<ArmedSupportRecoveryPlanStatus>),
    PreArmObserve(PreArmObserveRecoveryPlanStatus),
    PreArmFinalize(Box<PreArmFinalizeRecoveryPlanStatus>),
    ManualWorkingInfobaseLease(ManualWorkingInfobaseLeaseRecoveryPlanStatus),
    Artifact(ArtifactRecoveryPlanStatus),
    Archive(ArchiveRecoveryPlanStatus),
    Cleanup(CleanupRecoveryPlanStatus),
}

/// Complete current recovery authority. It is a physical target/effect/stage
/// union and deliberately has no `Deserialize` or raw-field constructor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct RecoveryPlanStatus(RecoveryPlanStatusKind);

/// Exact cleanup finalization lineage retained by an archived-cleanup status.
/// Callers cannot manufacture it independently of a validated cleanup plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CleanupRecoveryBinding<'a> {
    prior_operation_id: &'a OperationId,
    recovery_digest: &'a Sha256Digest,
    archive_id: &'a UnicaId,
    finish_action_id: &'a UnicaId,
    finish_action_digest: &'a Sha256Digest,
    owned_targets: &'a [OwnedTargetLocator],
    quarantine_id: &'a UnicaId,
    planned_result_phase: TaskPhase,
}

impl CleanupRecoveryBinding<'_> {
    pub(crate) const fn prior_operation_id(&self) -> &OperationId {
        self.prior_operation_id
    }

    pub(crate) const fn recovery_digest(&self) -> &Sha256Digest {
        self.recovery_digest
    }

    pub(crate) const fn archive_id(&self) -> &UnicaId {
        self.archive_id
    }

    pub(crate) const fn finish_action_id(&self) -> &UnicaId {
        self.finish_action_id
    }

    pub(crate) const fn finish_action_digest(&self) -> &Sha256Digest {
        self.finish_action_digest
    }

    pub(crate) const fn owned_targets(&self) -> &[OwnedTargetLocator] {
        self.owned_targets
    }

    pub(crate) const fn quarantine_id(&self) -> &UnicaId {
        self.quarantine_id
    }

    pub(crate) const fn planned_result_phase(&self) -> TaskPhase {
        self.planned_result_phase
    }
}

impl JsonSchema for RecoveryPlanStatus {
    fn schema_name() -> Cow<'static, str> {
        "RecoveryPlanStatus".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<TaskConfigurationRecoveryPlanStatus>(),
            generator.subschema_for::<RepositoryLocksRecoveryPlanStatus>(),
            generator.subschema_for::<OriginalConfigurationRecoveryPlanStatus>(),
            generator.subschema_for::<RepositoryCommitObserveRecoveryPlanStatus>(),
            generator.subschema_for::<RepositoryCommitCommittedRecoveryPlanStatus>(),
            generator.subschema_for::<RepositoryCommitNotCommittedRecoveryPlanStatus>(),
            generator.subschema_for::<ArmedSupportRecoveryPlanStatus>(),
            generator.subschema_for::<PreArmObserveRecoveryPlanStatus>(),
            generator.subschema_for::<PreArmFinalizeRecoveryPlanStatus>(),
            generator.subschema_for::<ManualWorkingInfobaseLeaseRecoveryPlanStatus>(),
            generator.subschema_for::<ArtifactRecoveryPlanStatus>(),
            generator.subschema_for::<ArchiveRecoveryPlanStatus>(),
            generator.subschema_for::<CleanupRecoveryPlanStatus>(),
        ])
    }
}

/// Schema-only recovery context for an effect that happened before a task
/// workspace became durable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PreWorkspaceRecoveryPlanStatusSchema;

impl JsonSchema for PreWorkspaceRecoveryPlanStatusSchema {
    fn schema_name() -> Cow<'static, str> {
        "PreWorkspaceRecoveryPlanStatus".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<TaskConfigurationRecoveryPlanStatus>(),
            generator.subschema_for::<ArtifactRecoveryPlanStatus>(),
        ])
    }
}

/// Schema-only recovery context while the task workspace still exists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WorkspaceRecoveryPlanStatusSchema;

impl JsonSchema for WorkspaceRecoveryPlanStatusSchema {
    fn schema_name() -> Cow<'static, str> {
        "WorkspaceRecoveryPlanStatus".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<TaskConfigurationRecoveryPlanStatus>(),
            generator.subschema_for::<RepositoryLocksRecoveryPlanStatus>(),
            generator.subschema_for::<OriginalConfigurationRecoveryPlanStatus>(),
            generator.subschema_for::<RepositoryCommitObserveRecoveryPlanStatus>(),
            generator.subschema_for::<RepositoryCommitCommittedRecoveryPlanStatus>(),
            generator.subschema_for::<RepositoryCommitNotCommittedRecoveryPlanStatus>(),
            generator.subschema_for::<ArmedSupportRecoveryPlanStatus>(),
            generator.subschema_for::<PreArmObserveRecoveryPlanStatus>(),
            generator.subschema_for::<PreArmFinalizeRecoveryPlanStatus>(),
            generator.subschema_for::<ManualWorkingInfobaseLeaseRecoveryPlanStatus>(),
            generator.subschema_for::<ArtifactRecoveryPlanStatus>(),
            generator.subschema_for::<ArchiveRecoveryPlanStatus>(),
        ])
    }
}

/// Schema-only recovery context after archive completion and before cleanup
/// completion. No target other than cleanup is legal here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ArchivedCleanupRecoveryPlanStatusSchema;

impl JsonSchema for ArchivedCleanupRecoveryPlanStatusSchema {
    fn schema_name() -> Cow<'static, str> {
        "ArchivedCleanupRecoveryPlanStatus".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        generator.subschema_for::<CleanupRecoveryPlanStatus>()
    }
}

impl RecoveryPlanStatus {
    #[cfg(test)]
    pub(crate) fn task_configuration_fixture_test_only(
        prior_operation_id: OperationId,
    ) -> Result<Self, RecoveryContractError> {
        let fingerprint =
            Sha256Digest::parse("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
                .map_err(|_| RecoveryContractError("test fingerprint is invalid"))?;
        let action_id = UnicaId::parse("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa")
            .map_err(|_| RecoveryContractError("test action ID is invalid"))?;
        let (expected_observations, expected_postcondition_digest) =
            expected_postcondition(vec![RecoveryExpectedObservation::new(
                RecoveryObservationKind::TaskFingerprint,
                RecoverySubjectRef::registered(action_id.clone()),
                fingerprint.clone(),
            )])?;
        let action =
            RecoveryAction::from_record(RecoveryActionDigestRecordKind::VerifyTaskFingerprint(
                VerifyTaskFingerprintActionDigestRecord {
                    action_kind: VerifyTaskFingerprintActionKind::Value,
                    action_id,
                    expected_task_fingerprint: fingerprint,
                    expected_observations,
                    expected_postcondition_digest,
                },
            ))?;
        Self::task_configuration_test_only(
            prior_operation_id,
            TaskPhase::LocalVerified,
            Vec::new(),
            vec![action],
        )
    }

    #[cfg(test)]
    pub(crate) fn cleanup_fixture_test_only(
        prior_operation_id: OperationId,
        archive_id: UnicaId,
        owned_target: OwnedTargetLocator,
        planned_result_phase: TaskPhase,
    ) -> Result<Self, RecoveryContractError> {
        Self::cleanup_targets_fixture_test_only(
            prior_operation_id,
            archive_id,
            vec![owned_target],
            planned_result_phase,
        )
    }

    #[cfg(test)]
    pub(crate) fn cleanup_targets_fixture_test_only(
        prior_operation_id: OperationId,
        archive_id: UnicaId,
        owned_targets: Vec<OwnedTargetLocator>,
        planned_result_phase: TaskPhase,
    ) -> Result<Self, RecoveryContractError> {
        let quarantine_id = UnicaId::parse("cccccccc-cccc-4ccc-8ccc-cccccccccccc")
            .map_err(|_| RecoveryContractError("test quarantine ID is invalid"))?;
        let mut actions = owned_targets
            .iter()
            .enumerate()
            .map(|(index, owned_target)| {
                let expected_quarantined_digest =
                    expected_quarantined_owned_target_digest(owned_target, &quarantine_id)?;
                let resume_observation = RecoveryExpectedObservation::new(
                    RecoveryObservationKind::QuarantinePresence,
                    RecoverySubjectRef::owned_role(owned_target.clone()),
                    expected_quarantined_digest.clone(),
                );
                let (resume_observations, resume_postcondition) =
                    expected_postcondition(vec![resume_observation])?;
                RecoveryAction::from_record(
                    RecoveryActionDigestRecordKind::ResumeOwnedTargetQuarantine(
                        ResumeOwnedTargetQuarantineActionDigestRecord {
                            action_kind: ResumeOwnedTargetQuarantineActionKind::Value,
                            action_id: UnicaId::parse(&format!(
                                "aaaaaaaa-aaaa-4aaa-8aaa-{index:012}"
                            ))
                            .map_err(|_| RecoveryContractError("test action ID is invalid"))?,
                            owned_target: owned_target.clone(),
                            quarantine_id: quarantine_id.clone(),
                            expected_quarantined_digest,
                            expected_observations: resume_observations,
                            expected_postcondition_digest: resume_postcondition,
                        },
                    ),
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        let finish_action_id = UnicaId::parse("bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb")
            .map_err(|_| RecoveryContractError("test action ID is invalid"))?;
        let finish_observations = owned_targets
            .iter()
            .map(|owned_target| {
                Ok(RecoveryExpectedObservation::new(
                    RecoveryObservationKind::OwnedTargetAbsence,
                    RecoverySubjectRef::owned_role(owned_target.clone()),
                    expected_absent_owned_target_digest(
                        &archive_id,
                        &finish_action_id,
                        owned_target,
                    )?,
                ))
            })
            .collect::<Result<Vec<_>, RecoveryContractError>>()?;
        let (finish_observations, finish_postcondition) =
            expected_postcondition(finish_observations)?;
        let finish = RecoveryAction::from_record(RecoveryActionDigestRecordKind::FinishCleanup(
            FinishCleanupActionDigestRecord {
                action_kind: FinishCleanupActionKind::Value,
                action_id: finish_action_id,
                archive_id,
                owned_targets: RecoveryOwnedTargets::new(owned_targets)?,
                expected_all_absent: TrueLiteral,
                expected_observations: finish_observations,
                expected_postcondition_digest: finish_postcondition,
            },
        ))?;
        actions.push(finish);
        let (observations, remaining_unknowns) = validated_plan_observations(Vec::new())?;
        let actions = CleanupRecoveryActions::from_actions(actions)?;
        Ok(Self(RecoveryPlanStatusKind::Cleanup(
            CleanupRecoveryPlanStatus::new(CleanupRecoveryPlanDigestRecord {
                prior_operation_id,
                target: CleanupRecoveryTarget::Value,
                effect_class: CleanupRecoveryEffectClass::Value,
                planned_result_phase,
                observations,
                actions,
                remaining_unknowns,
            })?,
        )))
    }

    #[cfg(test)]
    fn task_configuration_test_only(
        prior_operation_id: OperationId,
        planned_result_phase: TaskPhase,
        observations: Vec<RecoveryObservation>,
        actions: Vec<RecoveryAction>,
    ) -> Result<Self, RecoveryContractError> {
        validate_target_effect_action_grammar(
            RecoveryTarget::TaskConfiguration,
            RecoveryEffectClass::Rollback,
            &actions,
        )?;
        let (observations, remaining_unknowns) = validated_plan_observations(observations)?;
        let actions = TaskConfigurationRecoveryActions::from_actions(actions)?;
        Ok(Self(RecoveryPlanStatusKind::TaskConfiguration(
            TaskConfigurationRecoveryPlanStatus::new(TaskConfigurationRecoveryPlanDigestRecord {
                prior_operation_id,
                target: TaskConfigurationRecoveryTarget::Value,
                effect_class: RollbackRecoveryEffectClass::Value,
                planned_result_phase,
                observations,
                actions,
                remaining_unknowns,
            })?,
        )))
    }

    #[cfg(test)]
    fn repository_commit_observe_test_only(
        prior_operation_id: OperationId,
        planned_result_phase: TaskPhase,
        observations: Vec<RecoveryObservation>,
        actions: Vec<RecoveryAction>,
    ) -> Result<Self, RecoveryContractError> {
        if !matches!(
            actions.as_slice(),
            [RecoveryAction(RecoveryActionKindWire::ObserveCommit(_))]
        ) {
            return Err(RecoveryContractError(
                "repository commit observe stage requires exactly observeCommit",
            ));
        }
        let (observations, remaining_unknowns) = validated_plan_observations(observations)?;
        let actions = RepositoryCommitObserveRecoveryActions::from_actions(actions)?;
        Ok(Self(RecoveryPlanStatusKind::RepositoryCommitObserve(
            RepositoryCommitObserveRecoveryPlanStatus::new(
                RepositoryCommitObserveRecoveryPlanDigestRecord {
                    prior_operation_id,
                    target: RepositoryCommitRecoveryTarget::Value,
                    effect_class: ReconcileOnlyRecoveryEffectClass::Value,
                    repository_commit_stage: ObserveOutcomeCommitRecoveryStage::Value,
                    planned_result_phase,
                    observations,
                    actions,
                    remaining_unknowns,
                },
            )?,
        )))
    }

    #[cfg(test)]
    pub(crate) fn repository_commit_committed_test_only(
        prior_operation_id: OperationId,
        observations: Vec<RecoveryObservation>,
        actions: Vec<RecoveryAction>,
    ) -> Result<Self, RecoveryContractError> {
        validate_target_effect_action_grammar(
            RecoveryTarget::RepositoryCommit,
            RecoveryEffectClass::ReconcileOnly,
            &actions,
        )?;
        if !matches!(
            actions.as_slice(),
            [] | [RecoveryAction(RecoveryActionKindWire::ReleaseOwnedLocks(_))]
        ) {
            return Err(RecoveryContractError(
                "committed recovery branch is release-only",
            ));
        }
        let (observations, remaining_unknowns) = validated_plan_observations(observations)?;
        let actions = RepositoryCommitCommittedRecoveryActions::from_actions(actions)?;
        Ok(Self(RecoveryPlanStatusKind::RepositoryCommitCommitted(
            RepositoryCommitCommittedRecoveryPlanStatus::new(
                RepositoryCommitCommittedRecoveryPlanDigestRecord {
                    prior_operation_id,
                    target: RepositoryCommitRecoveryTarget::Value,
                    effect_class: ReconcileOnlyRecoveryEffectClass::Value,
                    repository_commit_stage: CommittedCommitRecoveryStage::Value,
                    planned_result_phase: TaskPhase::CommittedAndUnlocked,
                    observations,
                    actions,
                    remaining_unknowns,
                },
            )?,
        )))
    }

    #[cfg(test)]
    fn repository_commit_not_committed_test_only(
        prior_operation_id: OperationId,
        observations: Vec<RecoveryObservation>,
        actions: Vec<RecoveryAction>,
    ) -> Result<Self, RecoveryContractError> {
        validate_target_effect_action_grammar(
            RecoveryTarget::RepositoryCommit,
            RecoveryEffectClass::Rollback,
            &actions,
        )?;
        let (observations, remaining_unknowns) = validated_plan_observations(observations)?;
        let actions = RepositoryCommitNotCommittedRecoveryActions::from_actions(actions)?;
        Ok(Self(RecoveryPlanStatusKind::RepositoryCommitNotCommitted(
            RepositoryCommitNotCommittedRecoveryPlanStatus::new(
                RepositoryCommitNotCommittedRecoveryPlanDigestRecord {
                    prior_operation_id,
                    target: RepositoryCommitRecoveryTarget::Value,
                    effect_class: RollbackRecoveryEffectClass::Value,
                    repository_commit_stage: NotCommittedCommitRecoveryStage::Value,
                    planned_result_phase: TaskPhase::Synchronized,
                    observations,
                    actions,
                    remaining_unknowns,
                },
            )?,
        )))
    }

    #[cfg(test)]
    fn prearm_observe_test_only(
        prior_operation_id: OperationId,
        planned_result_phase: TaskPhase,
        observations: Vec<RecoveryObservation>,
        actions: Vec<RecoveryAction>,
    ) -> Result<Self, RecoveryContractError> {
        validate_prearm_observe_action_grammar(&prior_operation_id, &actions)?;
        let (observations, remaining_unknowns) = validated_plan_observations(observations)?;
        let actions = PreArmObserveRecoveryActions::from_actions(actions)?;
        Ok(Self(RecoveryPlanStatusKind::PreArmObserve(
            PreArmObserveRecoveryPlanStatus::new(PreArmObserveRecoveryPlanDigestRecord {
                prior_operation_id,
                target: PreArmSupportCancellationRecoveryTarget::Value,
                effect_class: ReconcileOnlyRecoveryEffectClass::Value,
                pre_arm_cancellation_stage: ObserveOutcomePreArmRecoveryStage::Value,
                planned_result_phase,
                observations,
                actions,
                remaining_unknowns,
            })?,
        )))
    }

    #[cfg(test)]
    fn prearm_finalize_test_only(
        observation: PreArmCancellationEffectObservation,
        plan: PreArmCancellationFinalizationPlan,
        progress: PreArmCancellationFinalizationAttemptProgress,
        known_blocker: Option<PreArmCancellationKnownBlocker>,
        observations: Vec<RecoveryObservation>,
        actions: Vec<RecoveryAction>,
    ) -> Result<Self, RecoveryContractError> {
        validate_prearm_finalize_action_grammar(&observation, &plan, &progress, &actions)?;
        let (observations, remaining_unknowns) = validated_plan_observations(observations)?;
        let actions = PreArmFinalizeRecoveryActions::from_actions(actions)?;
        let planned_result_phase = plan.planned_result_phase();
        let prior_operation_id = observation.prior_operation_id().clone();
        Ok(Self(RecoveryPlanStatusKind::PreArmFinalize(Box::new(
            PreArmFinalizeRecoveryPlanStatus::new(
                PreArmFinalizeRecoveryPlanDigestRecord {
                    prior_operation_id,
                    target: PreArmSupportCancellationRecoveryTarget::Value,
                    effect_class: ReconcileOnlyRecoveryEffectClass::Value,
                    pre_arm_cancellation_stage: FinalizePreArmRecoveryStage::Value,
                    planned_result_phase,
                    observations,
                    actions,
                    pre_arm_cancellation_effect_observation: observation,
                    pre_arm_cancellation_finalization_plan: plan,
                    pre_arm_cancellation_known_blocker: known_blocker,
                    remaining_unknowns,
                },
                progress,
            )?,
        ))))
    }

    pub(crate) const fn recovery_digest(&self) -> &Sha256Digest {
        match &self.0 {
            RecoveryPlanStatusKind::TaskConfiguration(value) => &value.recovery_digest,
            RecoveryPlanStatusKind::RepositoryLocks(value) => &value.recovery_digest,
            RecoveryPlanStatusKind::OriginalConfiguration(value) => &value.recovery_digest,
            RecoveryPlanStatusKind::RepositoryCommitObserve(value) => &value.recovery_digest,
            RecoveryPlanStatusKind::RepositoryCommitCommitted(value) => &value.recovery_digest,
            RecoveryPlanStatusKind::RepositoryCommitNotCommitted(value) => &value.recovery_digest,
            RecoveryPlanStatusKind::SupportPrerequisite(value) => &value.recovery_digest,
            RecoveryPlanStatusKind::PreArmObserve(value) => &value.recovery_digest,
            RecoveryPlanStatusKind::PreArmFinalize(value) => &value.recovery_digest,
            RecoveryPlanStatusKind::ManualWorkingInfobaseLease(value) => &value.recovery_digest,
            RecoveryPlanStatusKind::Artifact(value) => &value.recovery_digest,
            RecoveryPlanStatusKind::Archive(value) => &value.recovery_digest,
            RecoveryPlanStatusKind::Cleanup(value) => &value.recovery_digest,
        }
    }

    pub(crate) const fn prior_operation_id(&self) -> &OperationId {
        match &self.0 {
            RecoveryPlanStatusKind::TaskConfiguration(value) => &value.record.prior_operation_id,
            RecoveryPlanStatusKind::RepositoryLocks(value) => &value.record.prior_operation_id,
            RecoveryPlanStatusKind::OriginalConfiguration(value) => {
                &value.record.prior_operation_id
            }
            RecoveryPlanStatusKind::RepositoryCommitObserve(value) => {
                &value.record.prior_operation_id
            }
            RecoveryPlanStatusKind::RepositoryCommitCommitted(value) => {
                &value.record.prior_operation_id
            }
            RecoveryPlanStatusKind::RepositoryCommitNotCommitted(value) => {
                &value.record.prior_operation_id
            }
            RecoveryPlanStatusKind::SupportPrerequisite(value) => &value.record.prior_operation_id,
            RecoveryPlanStatusKind::PreArmObserve(value) => &value.record.prior_operation_id,
            RecoveryPlanStatusKind::PreArmFinalize(value) => &value.record.prior_operation_id,
            RecoveryPlanStatusKind::ManualWorkingInfobaseLease(value) => {
                &value.record.prior_operation_id
            }
            RecoveryPlanStatusKind::Artifact(value) => &value.record.prior_operation_id,
            RecoveryPlanStatusKind::Archive(value) => &value.record.prior_operation_id,
            RecoveryPlanStatusKind::Cleanup(value) => &value.record.prior_operation_id,
        }
    }

    pub(crate) const fn target(&self) -> RecoveryTarget {
        match &self.0 {
            RecoveryPlanStatusKind::TaskConfiguration(_) => RecoveryTarget::TaskConfiguration,
            RecoveryPlanStatusKind::RepositoryLocks(_) => RecoveryTarget::RepositoryLocks,
            RecoveryPlanStatusKind::OriginalConfiguration(_) => {
                RecoveryTarget::OriginalConfiguration
            }
            RecoveryPlanStatusKind::RepositoryCommitObserve(_)
            | RecoveryPlanStatusKind::RepositoryCommitCommitted(_)
            | RecoveryPlanStatusKind::RepositoryCommitNotCommitted(_) => {
                RecoveryTarget::RepositoryCommit
            }
            RecoveryPlanStatusKind::SupportPrerequisite(_) => RecoveryTarget::SupportPrerequisite,
            RecoveryPlanStatusKind::PreArmObserve(_)
            | RecoveryPlanStatusKind::PreArmFinalize(_) => {
                RecoveryTarget::PreArmSupportCancellation
            }
            RecoveryPlanStatusKind::ManualWorkingInfobaseLease(_) => {
                RecoveryTarget::ManualWorkingInfobaseLease
            }
            RecoveryPlanStatusKind::Artifact(_) => RecoveryTarget::Artifact,
            RecoveryPlanStatusKind::Archive(_) => RecoveryTarget::Archive,
            RecoveryPlanStatusKind::Cleanup(_) => RecoveryTarget::Cleanup,
        }
    }

    pub(crate) const fn planned_result_phase(&self) -> TaskPhase {
        match &self.0 {
            RecoveryPlanStatusKind::TaskConfiguration(value) => value.record.planned_result_phase,
            RecoveryPlanStatusKind::RepositoryLocks(value) => value.record.planned_result_phase,
            RecoveryPlanStatusKind::OriginalConfiguration(value) => {
                value.record.planned_result_phase
            }
            RecoveryPlanStatusKind::RepositoryCommitObserve(value) => {
                value.record.planned_result_phase
            }
            RecoveryPlanStatusKind::RepositoryCommitCommitted(value) => {
                value.record.planned_result_phase
            }
            RecoveryPlanStatusKind::RepositoryCommitNotCommitted(value) => {
                value.record.planned_result_phase
            }
            RecoveryPlanStatusKind::SupportPrerequisite(value) => value.record.planned_result_phase,
            RecoveryPlanStatusKind::PreArmObserve(value) => value.record.planned_result_phase,
            RecoveryPlanStatusKind::PreArmFinalize(value) => value.record.planned_result_phase,
            RecoveryPlanStatusKind::ManualWorkingInfobaseLease(value) => {
                value.record.planned_result_phase
            }
            RecoveryPlanStatusKind::Artifact(value) => value.record.planned_result_phase,
            RecoveryPlanStatusKind::Archive(value) => value.record.planned_result_phase,
            RecoveryPlanStatusKind::Cleanup(value) => value.record.planned_result_phase,
        }
    }

    pub(crate) fn cleanup_binding(
        &self,
    ) -> Result<CleanupRecoveryBinding<'_>, RecoveryContractError> {
        let RecoveryPlanStatusKind::Cleanup(value) = &self.0 else {
            return Err(RecoveryContractError("recovery plan is not a cleanup plan"));
        };
        let actions = value.record.actions.as_slice();
        validate_cleanup_action_grammar(actions)?;
        let Some(RecoveryAction(RecoveryActionKindWire::FinishCleanup(finish))) = actions.last()
        else {
            return Err(RecoveryContractError(
                "validated cleanup plan has no final cleanup action",
            ));
        };
        let Some(RecoveryAction(RecoveryActionKindWire::ResumeOwnedTargetQuarantine(first))) =
            actions.first()
        else {
            return Err(RecoveryContractError(
                "validated cleanup recovery has no quarantine action",
            ));
        };
        if actions[..actions.len() - 1].iter().any(|action| {
            !matches!(
                action,
                RecoveryAction(RecoveryActionKindWire::ResumeOwnedTargetQuarantine(resume))
                    if resume.quarantine_id == first.quarantine_id
            )
        }) {
            return Err(RecoveryContractError(
                "cleanup recovery actions disagree on quarantine identity",
            ));
        }
        Ok(CleanupRecoveryBinding {
            prior_operation_id: &value.record.prior_operation_id,
            recovery_digest: &value.recovery_digest,
            archive_id: &finish.archive_id,
            finish_action_id: &finish.action_id,
            finish_action_digest: &finish.action_digest,
            owned_targets: &finish.owned_targets.0,
            quarantine_id: &first.quarantine_id,
            planned_result_phase: value.record.planned_result_phase,
        })
    }

    /// Match one exact observation for every cleanup target, in the plan's
    /// canonical target order, and retain the recovery lineage around the
    /// otherwise value-only observation digests.
    pub(crate) fn match_cleanup_absences(
        &self,
        observations: Vec<RecoveryObservation>,
    ) -> Result<FinishCleanupAbsenceObservations, RecoveryContractError> {
        let RecoveryPlanStatusKind::Cleanup(value) = &self.0 else {
            return Err(RecoveryContractError("recovery plan is not a cleanup plan"));
        };
        let actions = value.record.actions.as_slice();
        validate_cleanup_action_grammar(actions)?;
        let Some(RecoveryAction(RecoveryActionKindWire::FinishCleanup(finish))) = actions.last()
        else {
            return Err(RecoveryContractError(
                "validated cleanup plan has no final cleanup action",
            ));
        };
        if observations.len() != finish.owned_targets.0.len() {
            return Err(RecoveryContractError(
                "cleanup requires one fresh absence observation per owned target",
            ));
        }
        let observations = finish
            .owned_targets
            .0
            .iter()
            .zip(&observations)
            .map(|(target, observation)| {
                RecoveryAction(RecoveryActionKindWire::FinishCleanup(finish.clone()))
                    .match_finish_cleanup_absence(target, observation)
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(FinishCleanupAbsenceObservations {
            prior_operation_id: value.record.prior_operation_id.clone(),
            recovery_digest: value.recovery_digest.clone(),
            archive_id: finish.archive_id.clone(),
            finish_action_id: finish.action_id.clone(),
            finish_action_digest: finish.action_digest.clone(),
            observations,
        })
    }

    #[cfg(test)]
    pub(crate) fn cleanup_matching_absence_observations_test_only(
        &self,
    ) -> Result<FinishCleanupAbsenceObservations, RecoveryContractError> {
        let RecoveryPlanStatusKind::Cleanup(value) = &self.0 else {
            return Err(RecoveryContractError("recovery plan is not a cleanup plan"));
        };
        let Some(RecoveryAction(RecoveryActionKindWire::FinishCleanup(finish))) =
            value.record.actions.as_slice().last()
        else {
            return Err(RecoveryContractError(
                "validated cleanup plan has no final cleanup action",
            ));
        };
        let observations = finish
            .expected_observations
            .as_slice()
            .iter()
            .map(|expected| {
                RecoveryObservation::matched_test_only(
                    expected.observation_kind,
                    expected.subject.clone(),
                    expected.expected_digest.clone(),
                    expected.expected_digest.clone(),
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        self.match_cleanup_absences(observations)
    }

    pub(crate) fn armed_support_recovery_projection(
        &self,
    ) -> Result<ArmedSupportRecoveryPlanProjection, RecoveryContractError> {
        let RecoveryPlanStatusKind::SupportPrerequisite(value) = &self.0 else {
            return Err(RecoveryContractError(
                "recovery plan is not an armed support-prerequisite plan",
            ));
        };
        Ok(ArmedSupportRecoveryPlanProjection::from_status(
            value.as_ref().clone(),
        ))
    }
}

/// Opaque owned projection consumed by the sole support-recovery approval and
/// execution authority. Its fields remain private so no caller can splice a
/// different history, plan, guard proof, or ordered action catalog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ArmedSupportRecoveryPlanProjection {
    sealed_status: ArmedSupportRecoveryPlanStatus,
    recovery_digest: Sha256Digest,
    prior_operation_id: OperationId,
    support_action_id: UnicaId,
    manual_target_mode: ManualSupportTargetMode,
    planned_result_phase: TaskPhase,
    support_history_from_cursor: RepositoryHistoryCursor,
    support_history_through_cursor: RepositoryHistoryCursor,
    support_history_partition: ValidatedRepositoryHistoryPartition,
    support_version_observations: SupportRecoveryVersionObservations,
    support_version_observation_digest: Sha256Digest,
    support_recovery_disposition: SupportRecoveryDisposition,
    support_late_relevant_result_phase: TaskPhase,
    support_recovery_finalization_plan: SupportRecoveryFinalizationPlan,
    latest_support_recovery_guard_proof: Option<SupportRecoveryGuardProof>,
    manual_working_infobase_closure_plan: Option<ManualWorkingInfobaseClosurePlan>,
    required_external_action: Option<SupportRecoveryExternalAction>,
    actions: Vec<RecoveryAction>,
}

impl ArmedSupportRecoveryPlanProjection {
    fn from_status(value: ArmedSupportRecoveryPlanStatus) -> Self {
        Self {
            sealed_status: value.clone(),
            recovery_digest: value.recovery_digest.clone(),
            prior_operation_id: value.record.prior_operation_id.clone(),
            support_action_id: match &value.record.actions.as_slice()[0].0 {
                RecoveryActionKindWire::ObserveSupportPrerequisiteHistory(action) => {
                    action.support_action_id.clone()
                }
                _ => unreachable!("armed support status constructor validates its first action"),
            },
            manual_target_mode: value.record.manual_target_mode,
            planned_result_phase: value.record.planned_result_phase,
            support_history_from_cursor: value.record.support_history_from_cursor.clone(),
            support_history_through_cursor: value.record.support_history_through_cursor.clone(),
            support_history_partition: value.record.support_history_partition.clone(),
            support_version_observations: value.record.support_version_observations.clone(),
            support_version_observation_digest: value
                .record
                .support_version_observation_digest
                .clone(),
            support_recovery_disposition: value.record.support_recovery_disposition,
            support_late_relevant_result_phase: value.record.support_late_relevant_result_phase,
            support_recovery_finalization_plan: value
                .record
                .support_recovery_finalization_plan
                .clone(),
            latest_support_recovery_guard_proof: value
                .record
                .latest_support_recovery_guard_proof
                .clone(),
            manual_working_infobase_closure_plan: value
                .record
                .manual_working_infobase_closure_plan
                .clone(),
            required_external_action: value.record.required_external_action.clone(),
            actions: value.record.actions.as_slice().to_vec(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn from_parts(
        prior_operation_id: OperationId,
        action_catalog: SupportRecoveryActionCatalogAuthority,
        support_action_id: UnicaId,
        planned_result_phase: TaskPhase,
        support_history_from_cursor: RepositoryHistoryCursor,
        support_history_through_cursor: RepositoryHistoryCursor,
        support_history_partition: ValidatedRepositoryHistoryPartition,
        support_version_observations: SupportRecoveryVersionObservations,
        support_recovery_disposition: SupportRecoveryDisposition,
        support_late_relevant_result_phase: TaskPhase,
        support_recovery_finalization_plan: SupportRecoveryFinalizationPlan,
        latest_support_recovery_guard_proof: Option<SupportRecoveryGuardProof>,
        manual_working_infobase_closure_plan: Option<ManualWorkingInfobaseClosurePlan>,
        manual_target_mode: ManualSupportTargetMode,
    ) -> Result<Self, RecoveryContractError> {
        let (history_action_id, external_wait, finalization_action_id) =
            action_catalog.into_parts();
        if support_history_partition.start_cursor() != &support_history_from_cursor
            || support_history_partition.through_inclusive() != &support_history_through_cursor
            || support_history_partition.classifications().count()
                != support_version_observations.as_slice().len()
            || (manual_target_mode == ManualSupportTargetMode::SeparateWorkingInfobase)
                != manual_working_infobase_closure_plan.is_some()
        {
            return Err(RecoveryContractError(
                "armed support recovery history or manual-mode presence mismatch",
            ));
        }
        if let Some(wait) = &external_wait {
            wait.validate_plan_binding(
                &support_action_id,
                manual_target_mode,
                manual_working_infobase_closure_plan.as_ref(),
            )?;
        }
        if support_recovery_disposition
            == SupportRecoveryDisposition::PreserveExternalAndReauthorize
            && planned_result_phase != support_late_relevant_result_phase
            || support_recovery_disposition == SupportRecoveryDisposition::RestoreThenAbandon
                && (planned_result_phase != TaskPhase::AbandonmentReady
                    || support_late_relevant_result_phase != TaskPhase::AbandonmentReady)
        {
            return Err(RecoveryContractError(
                "armed support recovery phases disagree with its disposition",
            ));
        }

        let (history_expected_observations, history_expected_postcondition_digest) =
            expected_postcondition(vec![RecoveryExpectedObservation::new(
                RecoveryObservationKind::RepositoryAnchor,
                RecoverySubjectRef::registered(support_action_id.clone()),
                support_history_partition.partition_digest().clone(),
            )])?;
        let history_action = RecoveryAction::from_record(
            RecoveryActionDigestRecordKind::ObserveSupportPrerequisiteHistory(
                ObserveSupportPrerequisiteHistoryActionDigestRecord {
                    action_kind: ObserveSupportPrerequisiteHistoryActionKind::Value,
                    action_id: history_action_id,
                    support_action_id: support_action_id.clone(),
                    from_cursor: support_history_from_cursor.clone(),
                    through_cursor: support_history_through_cursor.clone(),
                    expected_partition_digest: support_history_partition.partition_digest().clone(),
                    expected_observations: history_expected_observations,
                    expected_postcondition_digest: history_expected_postcondition_digest,
                },
            ),
        )?;
        let (finish_expected_observations, finish_expected_postcondition_digest) =
            expected_postcondition(vec![
                RecoveryExpectedObservation::new(
                    RecoveryObservationKind::SupportGraph,
                    RecoverySubjectRef::configuration_root(),
                    support_recovery_finalization_plan
                        .desired_support_graph_digest()
                        .clone(),
                ),
                RecoveryExpectedObservation::new(
                    RecoveryObservationKind::SupportActionAuthorization,
                    RecoverySubjectRef::registered(support_action_id.clone()),
                    support_recovery_finalization_plan.plan_digest().clone(),
                ),
            ])?;
        let authorization_outcome = match support_recovery_disposition {
            SupportRecoveryDisposition::RestoreThenReauthorize
            | SupportRecoveryDisposition::PreserveExternalAndReauthorize => {
                FinalizeSupportAuthorizationOutcome::Cancelled
            }
            SupportRecoveryDisposition::RestoreThenAbandon => {
                FinalizeSupportAuthorizationOutcome::AbandonmentFinalized
            }
        };
        let finish_action = RecoveryAction::from_record(
            RecoveryActionDigestRecordKind::FinalizeSupportPrerequisiteRecovery(
                FinalizeSupportPrerequisiteRecoveryActionDigestRecord {
                    action_kind: FinalizeSupportPrerequisiteRecoveryActionKind::Value,
                    action_id: finalization_action_id,
                    support_action_id: support_action_id.clone(),
                    finalization_plan_digest: support_recovery_finalization_plan
                        .plan_digest()
                        .clone(),
                    authorization_outcome,
                    expected_observations: finish_expected_observations,
                    expected_postcondition_digest: finish_expected_postcondition_digest,
                },
            ),
        )?;
        let (external_wait_action, required_external_action) = match external_wait {
            Some(wait) => {
                let (action, external_action) = wait.into_parts(&support_action_id)?;
                (Some(action), Some(external_action))
            }
            None => (None, None),
        };
        let mut actions = Vec::with_capacity(2 + usize::from(external_wait_action.is_some()));
        actions.push(history_action);
        actions.extend(external_wait_action);
        actions.push(finish_action);
        validate_support_recovery_action_grammar(
            &actions,
            manual_target_mode,
            support_recovery_disposition,
            support_recovery_finalization_plan.plan_digest(),
        )?;
        let actions = SupportPrerequisiteRecoveryActions::from_actions(actions)?;
        let (observations, remaining_unknowns) = validated_plan_observations(Vec::new())?;
        let successful_integration_forbidden = (support_recovery_disposition
            == SupportRecoveryDisposition::RestoreThenAbandon)
            .then_some(TrueLiteral);
        let support_version_observation_digest = support_version_observations.digest()?;
        let status = ArmedSupportRecoveryPlanStatus::new(ArmedSupportRecoveryPlanDigestRecord {
            prior_operation_id,
            target: SupportPrerequisiteRecoveryTarget::Value,
            effect_class: ReconcileOnlyRecoveryEffectClass::Value,
            planned_result_phase,
            observations,
            actions,
            support_version_observations,
            support_version_observation_digest,
            support_history_from_cursor,
            support_history_through_cursor,
            support_history_partition,
            support_recovery_disposition,
            support_late_relevant_result_phase,
            successful_integration_forbidden,
            support_recovery_finalization_plan,
            latest_support_recovery_guard_proof,
            manual_working_infobase_closure_plan,
            manual_target_mode,
            required_external_action,
            remaining_unknowns,
        })?;
        Ok(Self::from_status(status))
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_approved(
        _token: &SupportRecoveryAuthorityToken,
        prior_operation_id: OperationId,
        action_catalog: SupportRecoveryActionCatalogAuthority,
        support_action_id: UnicaId,
        planned_result_phase: TaskPhase,
        support_history_from_cursor: RepositoryHistoryCursor,
        support_history_through_cursor: RepositoryHistoryCursor,
        support_history_partition: ValidatedRepositoryHistoryPartition,
        support_version_observations: SupportRecoveryVersionObservations,
        support_recovery_disposition: SupportRecoveryDisposition,
        support_late_relevant_result_phase: TaskPhase,
        support_recovery_finalization_plan: SupportRecoveryFinalizationPlan,
        latest_support_recovery_guard_proof: Option<SupportRecoveryGuardProof>,
        manual_working_infobase_closure_plan: Option<ManualWorkingInfobaseClosurePlan>,
        manual_target_mode: ManualSupportTargetMode,
    ) -> Result<Self, RecoveryContractError> {
        Self::from_parts(
            prior_operation_id,
            action_catalog,
            support_action_id,
            planned_result_phase,
            support_history_from_cursor,
            support_history_through_cursor,
            support_history_partition,
            support_version_observations,
            support_recovery_disposition,
            support_late_relevant_result_phase,
            support_recovery_finalization_plan,
            latest_support_recovery_guard_proof,
            manual_working_infobase_closure_plan,
            manual_target_mode,
        )
    }

    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn test_only(
        prior_operation_id: OperationId,
        support_action_id: UnicaId,
        planned_result_phase: TaskPhase,
        support_history_from_cursor: RepositoryHistoryCursor,
        support_history_through_cursor: RepositoryHistoryCursor,
        support_history_partition: ValidatedRepositoryHistoryPartition,
        support_version_observations: SupportRecoveryVersionObservations,
        support_recovery_disposition: SupportRecoveryDisposition,
        support_late_relevant_result_phase: TaskPhase,
        support_recovery_finalization_plan: SupportRecoveryFinalizationPlan,
        latest_support_recovery_guard_proof: Option<SupportRecoveryGuardProof>,
        manual_working_infobase_closure_plan: Option<ManualWorkingInfobaseClosurePlan>,
        manual_target_mode: ManualSupportTargetMode,
        required_external_action: Option<SupportRecoveryExternalAction>,
    ) -> Result<Self, RecoveryContractError> {
        let external_wait = required_external_action
            .map(|action| {
                SupportRecoveryExternalWaitAuthority::from_external_action_test_only(
                    UnicaId::parse("cccccccc-cccc-4ccc-8ccc-cccccccccccc")
                        .expect("test external wait action ID is valid"),
                    action,
                    Sha256Digest::parse(
                        "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
                    )
                    .expect("test external wait postcondition digest is valid"),
                )
            })
            .transpose()?;
        let action_catalog = SupportRecoveryActionCatalogAuthority::test_only(
            UnicaId::parse("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa")
                .expect("test history action ID is valid"),
            external_wait,
            UnicaId::parse("bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb")
                .expect("test finalization action ID is valid"),
        )?;
        Self::from_parts(
            prior_operation_id,
            action_catalog,
            support_action_id,
            planned_result_phase,
            support_history_from_cursor,
            support_history_through_cursor,
            support_history_partition,
            support_version_observations,
            support_recovery_disposition,
            support_late_relevant_result_phase,
            support_recovery_finalization_plan,
            latest_support_recovery_guard_proof,
            manual_working_infobase_closure_plan,
            manual_target_mode,
        )
    }

    pub(crate) const fn recovery_digest(&self) -> &Sha256Digest {
        &self.recovery_digest
    }

    pub(crate) const fn prior_operation_id(&self) -> &OperationId {
        &self.prior_operation_id
    }

    pub(crate) fn recovery_plan_status(&self) -> RecoveryPlanStatus {
        RecoveryPlanStatus(RecoveryPlanStatusKind::SupportPrerequisite(Box::new(
            self.sealed_status.clone(),
        )))
    }

    pub(crate) fn into_recovery_plan_status(self) -> RecoveryPlanStatus {
        RecoveryPlanStatus(RecoveryPlanStatusKind::SupportPrerequisite(Box::new(
            self.sealed_status,
        )))
    }

    pub(crate) const fn support_action_id(&self) -> &UnicaId {
        &self.support_action_id
    }

    pub(crate) const fn manual_target_mode(&self) -> ManualSupportTargetMode {
        self.manual_target_mode
    }

    pub(crate) const fn planned_result_phase(&self) -> TaskPhase {
        self.planned_result_phase
    }

    pub(crate) const fn support_history_from_cursor(&self) -> &RepositoryHistoryCursor {
        &self.support_history_from_cursor
    }

    pub(crate) const fn support_history_through_cursor(&self) -> &RepositoryHistoryCursor {
        &self.support_history_through_cursor
    }

    pub(crate) const fn support_history_partition(&self) -> &ValidatedRepositoryHistoryPartition {
        &self.support_history_partition
    }

    pub(crate) const fn support_version_observations(&self) -> &SupportRecoveryVersionObservations {
        &self.support_version_observations
    }

    pub(crate) const fn support_version_observation_digest(&self) -> &Sha256Digest {
        &self.support_version_observation_digest
    }

    pub(crate) const fn support_recovery_disposition(&self) -> SupportRecoveryDisposition {
        self.support_recovery_disposition
    }

    pub(crate) const fn support_late_relevant_result_phase(&self) -> TaskPhase {
        self.support_late_relevant_result_phase
    }

    pub(crate) const fn support_recovery_finalization_plan(
        &self,
    ) -> &SupportRecoveryFinalizationPlan {
        &self.support_recovery_finalization_plan
    }

    pub(crate) const fn latest_support_recovery_guard_proof(
        &self,
    ) -> Option<&SupportRecoveryGuardProof> {
        self.latest_support_recovery_guard_proof.as_ref()
    }

    pub(crate) const fn manual_working_infobase_closure_plan(
        &self,
    ) -> Option<&ManualWorkingInfobaseClosurePlan> {
        self.manual_working_infobase_closure_plan.as_ref()
    }

    pub(crate) const fn required_external_action(&self) -> Option<&SupportRecoveryExternalAction> {
        self.required_external_action.as_ref()
    }

    pub(crate) fn actions(&self) -> &[RecoveryAction] {
        &self.actions
    }
}

wire_literal!(RecoveryActionReceiptKind, "recoveryAction");
wire_literal!(PerformedOutcome, "performed");
wire_literal!(RecoveredReceiptOutcome, "recoveredReceipt");
wire_literal!(AlreadySatisfiedOutcome, "alreadySatisfied");

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
struct TerminalObservationDigests(Vec<Sha256Digest>);

impl TerminalObservationDigests {
    fn new(values: Vec<Sha256Digest>) -> Result<Self, RecoveryContractError> {
        if values.is_empty() || values.len() > MAX_RECOVERY_ITEMS {
            return Err(RecoveryContractError(
                "terminal observation digests must be non-empty and bounded",
            ));
        }
        Ok(Self(values))
    }

    fn as_slice(&self) -> &[Sha256Digest] {
        &self.0
    }
}

impl JsonSchema for TerminalObservationDigests {
    fn schema_name() -> Cow<'static, str> {
        "TerminalObservationDigests".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        let item = generator.subschema_for::<Sha256Digest>();
        json_schema!({
            "type": "array",
            "items": item,
            "minItems": 1,
            "maxItems": MAX_RECOVERY_ITEMS
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RecoveryActionEffectReceiptDigestRecord {
    receipt_kind: RecoveryActionReceiptKind,
    receipt_id: UnicaId,
    producer_action_id: UnicaId,
    producer_action_digest: Sha256Digest,
    terminal_observation_digests: TerminalObservationDigests,
}

impl contract_digest_record_sealed::Sealed for RecoveryActionEffectReceiptDigestRecord {}
impl ContractDigestRecord for RecoveryActionEffectReceiptDigestRecord {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RecoveryActionEffectReceipt {
    receipt_kind: RecoveryActionReceiptKind,
    receipt_id: UnicaId,
    producer_action_id: UnicaId,
    producer_action_digest: Sha256Digest,
    terminal_observation_digests: TerminalObservationDigests,
    receipt_digest: Sha256Digest,
}

impl RecoveryActionEffectReceipt {
    #[cfg(test)]
    fn test_only(
        receipt_id: UnicaId,
        action: &RecoveryAction,
        terminal_observation_digests: TerminalObservationDigests,
    ) -> Result<Self, RecoveryContractError> {
        let (action_id, _, _, action_digest) = action.common();
        let record = RecoveryActionEffectReceiptDigestRecord {
            receipt_kind: RecoveryActionReceiptKind::Value,
            receipt_id,
            producer_action_id: action_id.clone(),
            producer_action_digest: action_digest.clone(),
            terminal_observation_digests,
        };
        let receipt_digest = contract_digest(&record, "recovery effect receipt digest failed")?;
        Ok(Self {
            receipt_kind: record.receipt_kind,
            receipt_id: record.receipt_id,
            producer_action_id: record.producer_action_id,
            producer_action_digest: record.producer_action_digest,
            terminal_observation_digests: record.terminal_observation_digests,
            receipt_digest,
        })
    }

    fn validates(
        &self,
        action: &RecoveryAction,
        expected_observation_digests: &TerminalObservationDigests,
    ) -> bool {
        let (action_id, _, _, action_digest) = action.common();
        &self.producer_action_id == action_id
            && &self.producer_action_digest == action_digest
            && self.terminal_observation_digests == *expected_observation_digests
            && contract_digest(
                &RecoveryActionEffectReceiptDigestRecord {
                    receipt_kind: self.receipt_kind,
                    receipt_id: self.receipt_id.clone(),
                    producer_action_id: self.producer_action_id.clone(),
                    producer_action_digest: self.producer_action_digest.clone(),
                    terminal_observation_digests: self.terminal_observation_digests.clone(),
                },
                "recovery effect receipt digest failed",
            )
            .is_ok_and(|digest| digest == self.receipt_digest)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
enum EffectReceiptKind {
    RecoveryAction(RecoveryActionEffectReceipt),
    PreArmCancellationEffect(super::prearm_recovery::PreArmCancellationEffectReceipt),
}

/// An immutable effect receipt. Deliberately not `Deserialize`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct EffectReceipt(EffectReceiptKind);

impl JsonSchema for EffectReceipt {
    fn schema_name() -> Cow<'static, str> {
        "EffectReceipt".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<RecoveryActionEffectReceipt>(),
            generator.subschema_for::<super::prearm_recovery::PreArmCancellationEffectReceipt>(),
        ])
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RecoveryActionClass {
    ObservationOnly,
    OrdinaryMutating,
    PreArmEffect,
}

impl RecoveryAction {
    fn class(&self) -> RecoveryActionClass {
        match self.0 {
            RecoveryActionKindWire::VerifyTaskFingerprint(_)
            | RecoveryActionKindWire::ObserveCommit(_)
            | RecoveryActionKindWire::ObservePreArmCancellationOutcome(_)
            | RecoveryActionKindWire::RecheckPreArmCancellationFinalization(_)
            | RecoveryActionKindWire::ObserveSupportPrerequisiteHistory(_)
            | RecoveryActionKindWire::ObserveWorkingInfobaseLease(_)
            | RecoveryActionKindWire::ObserveReservedOriginalLease(_)
            | RecoveryActionKindWire::ObserveRetentionLease(_)
            | RecoveryActionKindWire::ObserveArchiveStaging(_) => {
                RecoveryActionClass::ObservationOnly
            }
            RecoveryActionKindWire::AcquirePreArmRootGuard(_)
            | RecoveryActionKindWire::AcquirePreArmModeLease(_)
            | RecoveryActionKindWire::ApplyPreArmCancellationSelectiveUpdate(_)
            | RecoveryActionKindWire::PersistPreArmSupportCancellation(_)
            | RecoveryActionKindWire::ReleasePreArmModeLease(_)
            | RecoveryActionKindWire::ReleasePreArmRootGuard(_)
            | RecoveryActionKindWire::FinishPreArmCancellationRecovery(_) => {
                RecoveryActionClass::PreArmEffect
            }
            RecoveryActionKindWire::ReleaseOwnedLocks(_)
            | RecoveryActionKindWire::RestoreOriginal(_)
            | RecoveryActionKindWire::RestoreTaskCheckpoint(_)
            | RecoveryActionKindWire::RecreateTaskInfobase(_)
            | RecoveryActionKindWire::QuarantineArtifact(_)
            | RecoveryActionKindWire::UpdateOriginalSelectedTargets(_)
            | RecoveryActionKindWire::ReleaseWorkingInfobaseLease(_)
            | RecoveryActionKindWire::ReleaseReservedOriginalLease(_)
            | RecoveryActionKindWire::ReleaseRetentionLease(_)
            | RecoveryActionKindWire::AwaitExternalSupportCorrection(_)
            | RecoveryActionKindWire::AwaitExternalLockRelease(_)
            | RecoveryActionKindWire::AwaitManualWorkingInfobaseClosure(_)
            | RecoveryActionKindWire::AwaitReservedOriginalClosure(_)
            | RecoveryActionKindWire::AwaitExternalSupportConflictResolution(_)
            | RecoveryActionKindWire::AwaitSupportRecoveryEvidence(_)
            | RecoveryActionKindWire::FinalizeSupportPrerequisiteRecovery(_)
            | RecoveryActionKindWire::ResumeQuarantine(_)
            | RecoveryActionKindWire::ResumeOwnedTargetQuarantine(_)
            | RecoveryActionKindWire::FinishArchive(_)
            | RecoveryActionKindWire::FinishCleanup(_) => RecoveryActionClass::OrdinaryMutating,
        }
    }

    fn prearm_receipt_binding(
        &self,
    ) -> Option<(&PreArmCancellationReceiptRef, PreArmCancellationEffectKind)> {
        match &self.0 {
            RecoveryActionKindWire::AcquirePreArmRootGuard(value) => Some((
                &value.receipt_ref,
                PreArmCancellationEffectKind::RootGuardAcquire,
            )),
            RecoveryActionKindWire::AcquirePreArmModeLease(value) => match &value.0 {
                AcquirePreArmModeLeaseActionKindWire::ReservedOriginal(value) => Some((
                    &value.receipt_ref,
                    PreArmCancellationEffectKind::ModeLeaseAcquire,
                )),
                AcquirePreArmModeLeaseActionKindWire::SeparateWorkingInfobase(value) => Some((
                    &value.receipt_ref,
                    PreArmCancellationEffectKind::ModeLeaseAcquire,
                )),
            },
            RecoveryActionKindWire::ApplyPreArmCancellationSelectiveUpdate(value) => Some((
                &value.receipt_ref,
                PreArmCancellationEffectKind::SelectiveOriginalUpdate,
            )),
            RecoveryActionKindWire::PersistPreArmSupportCancellation(value) => Some((
                &value.receipt_ref,
                PreArmCancellationEffectKind::AuthorizationCancellation,
            )),
            RecoveryActionKindWire::ReleasePreArmModeLease(value) => match &value.0 {
                ReleasePreArmModeLeaseActionKindWire::ReservedOriginal(value) => Some((
                    &value.receipt_ref,
                    PreArmCancellationEffectKind::ModeLeaseRelease,
                )),
                ReleasePreArmModeLeaseActionKindWire::SeparateWorkingInfobase(value) => Some((
                    &value.receipt_ref,
                    PreArmCancellationEffectKind::ModeLeaseRelease,
                )),
            },
            RecoveryActionKindWire::ReleasePreArmRootGuard(value) => Some((
                &value.receipt_ref,
                PreArmCancellationEffectKind::RootGuardRelease,
            )),
            RecoveryActionKindWire::FinishPreArmCancellationRecovery(value) => Some((
                &value.receipt_ref,
                PreArmCancellationEffectKind::RecoveryFinalization,
            )),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PerformedRecoveryActionOutcomeDigestRecord {
    outcome: PerformedOutcome,
    action_id: UnicaId,
    action_digest: Sha256Digest,
    expected_postcondition_digest: Sha256Digest,
    observed_postcondition_digest: Sha256Digest,
    receipt: EffectReceipt,
    terminal_observation_digests: TerminalObservationDigests,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RecoveredReceiptActionOutcomeDigestRecord {
    outcome: RecoveredReceiptOutcome,
    action_id: UnicaId,
    action_digest: Sha256Digest,
    expected_postcondition_digest: Sha256Digest,
    observed_postcondition_digest: Sha256Digest,
    receipt: EffectReceipt,
    terminal_observation_digests: TerminalObservationDigests,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AlreadySatisfiedActionOutcomeDigestRecord {
    outcome: AlreadySatisfiedOutcome,
    action_id: UnicaId,
    action_digest: Sha256Digest,
    expected_postcondition_digest: Sha256Digest,
    observed_postcondition_digest: Sha256Digest,
    terminal_observation_digests: TerminalObservationDigests,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
enum RecoveryActionOutcomeDigestRecordKind {
    Performed(PerformedRecoveryActionOutcomeDigestRecord),
    RecoveredReceipt(RecoveredReceiptActionOutcomeDigestRecord),
    AlreadySatisfied(AlreadySatisfiedActionOutcomeDigestRecord),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct RecoveryActionOutcomeDigestRecord(RecoveryActionOutcomeDigestRecordKind);

impl contract_digest_record_sealed::Sealed for RecoveryActionOutcomeDigestRecord {}
impl ContractDigestRecord for RecoveryActionOutcomeDigestRecord {}

impl JsonSchema for RecoveryActionOutcomeDigestRecord {
    fn schema_name() -> Cow<'static, str> {
        "RecoveryActionOutcomeDigestRecord".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<PerformedRecoveryActionOutcomeDigestRecord>(),
            generator.subschema_for::<RecoveredReceiptActionOutcomeDigestRecord>(),
            generator.subschema_for::<AlreadySatisfiedActionOutcomeDigestRecord>(),
        ])
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PerformedRecoveryActionOutcome {
    outcome: PerformedOutcome,
    action_id: UnicaId,
    action_digest: Sha256Digest,
    expected_postcondition_digest: Sha256Digest,
    observed_postcondition_digest: Sha256Digest,
    receipt: EffectReceipt,
    terminal_observation_digests: TerminalObservationDigests,
    outcome_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RecoveredReceiptActionOutcome {
    outcome: RecoveredReceiptOutcome,
    action_id: UnicaId,
    action_digest: Sha256Digest,
    expected_postcondition_digest: Sha256Digest,
    observed_postcondition_digest: Sha256Digest,
    receipt: EffectReceipt,
    terminal_observation_digests: TerminalObservationDigests,
    outcome_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AlreadySatisfiedActionOutcome {
    outcome: AlreadySatisfiedOutcome,
    action_id: UnicaId,
    action_digest: Sha256Digest,
    expected_postcondition_digest: Sha256Digest,
    observed_postcondition_digest: Sha256Digest,
    terminal_observation_digests: TerminalObservationDigests,
    outcome_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
enum RecoveryActionOutcomeKind {
    Performed(PerformedRecoveryActionOutcome),
    RecoveredReceipt(RecoveredReceiptActionOutcome),
    AlreadySatisfied(AlreadySatisfiedActionOutcome),
}

/// A receipt/observation-validated outcome. Deliberately not `Deserialize`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct RecoveryActionOutcome(RecoveryActionOutcomeKind);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RecoveryActionOutcomeClass {
    Performed,
    RecoveredReceipt,
    AlreadySatisfied,
}

impl RecoveryActionOutcome {
    fn action_binding(&self) -> (&UnicaId, &Sha256Digest) {
        match &self.0 {
            RecoveryActionOutcomeKind::Performed(value) => (&value.action_id, &value.action_digest),
            RecoveryActionOutcomeKind::RecoveredReceipt(value) => {
                (&value.action_id, &value.action_digest)
            }
            RecoveryActionOutcomeKind::AlreadySatisfied(value) => {
                (&value.action_id, &value.action_digest)
            }
        }
    }

    fn outcome_class(&self) -> RecoveryActionOutcomeClass {
        match self.0 {
            RecoveryActionOutcomeKind::Performed(_) => RecoveryActionOutcomeClass::Performed,
            RecoveryActionOutcomeKind::RecoveredReceipt(_) => {
                RecoveryActionOutcomeClass::RecoveredReceipt
            }
            RecoveryActionOutcomeKind::AlreadySatisfied(_) => {
                RecoveryActionOutcomeClass::AlreadySatisfied
            }
        }
    }

    fn outcome_digest(&self) -> &Sha256Digest {
        match &self.0 {
            RecoveryActionOutcomeKind::Performed(value) => &value.outcome_digest,
            RecoveryActionOutcomeKind::RecoveredReceipt(value) => &value.outcome_digest,
            RecoveryActionOutcomeKind::AlreadySatisfied(value) => &value.outcome_digest,
        }
    }

    fn prearm_effect_receipt(&self) -> Option<&PreArmCancellationEffectReceipt> {
        let receipt = match &self.0 {
            RecoveryActionOutcomeKind::Performed(value) => &value.receipt,
            RecoveryActionOutcomeKind::RecoveredReceipt(value) => &value.receipt,
            RecoveryActionOutcomeKind::AlreadySatisfied(_) => return None,
        };
        match &receipt.0 {
            EffectReceiptKind::PreArmCancellationEffect(receipt) => Some(receipt),
            EffectReceiptKind::RecoveryAction(_) => None,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
struct PreArmArchiveActionOutcomeBinding {
    action_id: UnicaId,
    action_digest: Sha256Digest,
    outcome_digest: Sha256Digest,
}

/// Opaque validated witness that one exact finalization success catalog produced
/// the exact finalization-plan receipt sequence retained by completed progress.
///
/// This is intentionally narrower than terminal recovery authority: root/mode
/// capability proofs, the exact recheck observation rows, and the enclosing
/// recovery-receipt digest remain later gates. The witness is non-`Clone`, but
/// repeatable validation over the same immutable evidence may mint another one.
/// It has no raw constructor, `Serialize`, or `Deserialize`.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ValidatedPreArmArchiveReceiptOutcomeWitness {
    effect_observation_digest: Sha256Digest,
    finalization_attempt_id: UnicaId,
    finalization_plan_digest: Sha256Digest,
    attempt_audit_digest: Sha256Digest,
    ordered_action_outcome_bindings: Vec<PreArmArchiveActionOutcomeBinding>,
}

impl ValidatedPreArmArchiveReceiptOutcomeWitness {
    pub(crate) fn from_completed_outcomes(
        observation: &PreArmCancellationEffectObservation,
        plan: &PreArmCancellationFinalizationPlan,
        progress: &PreArmCancellationFinalizationAttemptProgress,
        actions: &[RecoveryAction],
        outcomes: &[RecoveryActionOutcome],
    ) -> Result<Self, RecoveryContractError> {
        validate_prearm_finalize_action_grammar(observation, plan, progress, actions)?;
        let realized_receipts =
            progress
                .completed_realized_receipts()
                .ok_or(RecoveryContractError(
                    "pre-arm archive outcomes require completed progress",
                ))?;
        let attempt_audit_digest = progress
            .attempt_audit_digest()
            .ok_or(RecoveryContractError(
                "completed pre-arm progress lacks its attempt audit digest",
            ))?;
        if progress.completed_recheck_evidence().is_none() || actions.len() != outcomes.len() {
            return Err(RecoveryContractError(
                "pre-arm archive outcomes do not cover the exact completed success catalog",
            ));
        }

        let mut projected_receipts = Vec::with_capacity(realized_receipts.len());
        let mut bindings = Vec::with_capacity(actions.len());
        for (action, outcome) in actions.iter().zip(outcomes) {
            let (outcome_action_id, outcome_action_digest) = outcome.action_binding();
            if outcome_action_id != action.action_id()
                || outcome_action_digest != action.action_digest()
            {
                return Err(RecoveryContractError(
                    "pre-arm outcome belongs to another action or action digest",
                ));
            }

            let is_recheck = matches!(
                action.0,
                RecoveryActionKindWire::RecheckPreArmCancellationFinalization(_)
            );
            if is_recheck {
                if outcome.outcome_class() != RecoveryActionOutcomeClass::AlreadySatisfied
                    || outcome.prearm_effect_receipt().is_some()
                {
                    return Err(RecoveryContractError(
                        "pre-arm recheck requires exactly its observation-only outcome",
                    ));
                }
            } else {
                if action.class() != RecoveryActionClass::PreArmEffect
                    || !matches!(
                        outcome.outcome_class(),
                        RecoveryActionOutcomeClass::Performed
                            | RecoveryActionOutcomeClass::RecoveredReceipt
                    )
                {
                    return Err(RecoveryContractError(
                        "pre-arm effect action lacks a performed or recovered receipt outcome",
                    ));
                }
                let receipt = outcome
                    .prearm_effect_receipt()
                    .ok_or(RecoveryContractError(
                        "pre-arm effect outcome lacks its typed pre-arm receipt",
                    ))?;
                projected_receipts.push(receipt.clone());
            }

            bindings.push(PreArmArchiveActionOutcomeBinding {
                action_id: action.action_id().clone(),
                action_digest: action.action_digest().clone(),
                outcome_digest: outcome.outcome_digest().clone(),
            });
        }

        if projected_receipts.as_slice() != realized_receipts {
            return Err(RecoveryContractError(
                "pre-arm outcomes do not equal the completed finalization receipt sequence",
            ));
        }

        Ok(Self {
            effect_observation_digest: observation.observation_digest().clone(),
            finalization_attempt_id: plan.finalization_attempt_id().clone(),
            finalization_plan_digest: plan.finalization_plan_digest().clone(),
            attempt_audit_digest: attempt_audit_digest.clone(),
            ordered_action_outcome_bindings: bindings,
        })
    }

    pub(crate) fn binds_archive_lineage(
        &self,
        observation: &PreArmCancellationEffectObservation,
        plan: &PreArmCancellationFinalizationPlan,
        progress: &PreArmCancellationFinalizationAttemptProgress,
    ) -> bool {
        !self.ordered_action_outcome_bindings.is_empty()
            && plan.binds_effect_observation(observation)
            && &self.effect_observation_digest == observation.observation_digest()
            && &self.finalization_attempt_id == plan.finalization_attempt_id()
            && &self.finalization_plan_digest == plan.finalization_plan_digest()
            && progress.finalization_attempt_id() == &self.finalization_attempt_id
            && progress.attempt_audit_digest() == Some(&self.attempt_audit_digest)
            && progress.completed_realized_receipts().is_some()
    }
}

#[cfg(test)]
pub(crate) struct PreArmArchiveReceiptOutcomeWitnessTestFixture {
    pub(crate) witness: ValidatedPreArmArchiveReceiptOutcomeWitness,
    pub(crate) observation: PreArmCancellationEffectObservation,
    pub(crate) plan: PreArmCancellationFinalizationPlan,
    pub(crate) recheck_evidence: PreArmCancellationFinalizationRecheckEvidence,
    pub(crate) progress: PreArmCancellationFinalizationAttemptProgress,
}

#[cfg(test)]
pub(crate) fn prearm_archive_receipt_outcome_witness_fixture_test_only(
) -> PreArmArchiveReceiptOutcomeWitnessTestFixture {
    tests::exact_prearm_archive_fixture_for_task()
}

impl JsonSchema for RecoveryActionOutcome {
    fn schema_name() -> Cow<'static, str> {
        "RecoveryActionOutcome".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<PerformedRecoveryActionOutcome>(),
            generator.subschema_for::<RecoveredReceiptActionOutcome>(),
            generator.subschema_for::<AlreadySatisfiedActionOutcome>(),
        ])
    }
}

struct ValidatedOutcomeProjection {
    action_id: UnicaId,
    action_digest: Sha256Digest,
    expected_postcondition_digest: Sha256Digest,
    observed_postcondition_digest: Sha256Digest,
    terminal_observation_digests: TerminalObservationDigests,
}

fn validate_outcome_projection(
    action: &RecoveryAction,
    observations: Vec<RecoveryObservation>,
) -> Result<ValidatedOutcomeProjection, RecoveryContractError> {
    let (action_id, expected, expected_postcondition_digest, action_digest) = action.common();
    if observations.len() != expected.as_slice().len() {
        return Err(RecoveryContractError(
            "recovery action outcome observation count mismatch",
        ));
    }
    let mut projected = Vec::with_capacity(observations.len());
    let mut digests = Vec::with_capacity(observations.len());
    for (observation, expected) in observations.iter().zip(expected.as_slice()) {
        if !observation.is_match() || &observation.expected_projection() != expected {
            return Err(RecoveryContractError(
                "recovery action outcome contains a non-matching observation projection",
            ));
        }
        projected.push(observation.expected_projection());
        digests.push(observation.observation_digest().clone());
    }
    let projected = RecoveryExpectedObservations::new(projected)?;
    let observed_postcondition_digest = contract_digest(
        &RecoveryExpectedPostconditionDigestRecord(projected),
        "observed recovery postcondition digest failed",
    )?;
    if &observed_postcondition_digest != expected_postcondition_digest {
        return Err(RecoveryContractError(
            "observed recovery postcondition does not equal the expected postcondition",
        ));
    }
    Ok(ValidatedOutcomeProjection {
        action_id: action_id.clone(),
        action_digest: action_digest.clone(),
        expected_postcondition_digest: expected_postcondition_digest.clone(),
        observed_postcondition_digest,
        terminal_observation_digests: TerminalObservationDigests::new(digests)?,
    })
}

impl RecoveryActionOutcome {
    #[cfg(test)]
    fn already_satisfied_test_only(
        action: &RecoveryAction,
        observations: Vec<RecoveryObservation>,
    ) -> Result<Self, RecoveryContractError> {
        if action.class() != RecoveryActionClass::ObservationOnly {
            return Err(RecoveryContractError(
                "alreadySatisfied is legal only for observation actions",
            ));
        }
        let projection = validate_outcome_projection(action, observations)?;
        let record = RecoveryActionOutcomeDigestRecord(
            RecoveryActionOutcomeDigestRecordKind::AlreadySatisfied(
                AlreadySatisfiedActionOutcomeDigestRecord {
                    outcome: AlreadySatisfiedOutcome::Value,
                    action_id: projection.action_id.clone(),
                    action_digest: projection.action_digest.clone(),
                    expected_postcondition_digest: projection.expected_postcondition_digest.clone(),
                    observed_postcondition_digest: projection.observed_postcondition_digest.clone(),
                    terminal_observation_digests: projection.terminal_observation_digests.clone(),
                },
            ),
        );
        let outcome_digest = contract_digest(&record, "recovery outcome digest failed")?;
        Ok(Self(RecoveryActionOutcomeKind::AlreadySatisfied(
            AlreadySatisfiedActionOutcome {
                outcome: AlreadySatisfiedOutcome::Value,
                action_id: projection.action_id,
                action_digest: projection.action_digest,
                expected_postcondition_digest: projection.expected_postcondition_digest,
                observed_postcondition_digest: projection.observed_postcondition_digest,
                terminal_observation_digests: projection.terminal_observation_digests,
                outcome_digest,
            },
        )))
    }

    #[cfg(test)]
    fn performed_test_only(
        action: &RecoveryAction,
        observations: Vec<RecoveryObservation>,
        receipt_id: UnicaId,
    ) -> Result<Self, RecoveryContractError> {
        Self::ordinary_effect_test_only(action, observations, receipt_id, false)
    }

    #[cfg(test)]
    fn recovered_receipt_test_only(
        action: &RecoveryAction,
        observations: Vec<RecoveryObservation>,
        receipt_id: UnicaId,
    ) -> Result<Self, RecoveryContractError> {
        Self::ordinary_effect_test_only(action, observations, receipt_id, true)
    }

    #[cfg(test)]
    fn ordinary_effect_test_only(
        action: &RecoveryAction,
        observations: Vec<RecoveryObservation>,
        receipt_id: UnicaId,
        recovered: bool,
    ) -> Result<Self, RecoveryContractError> {
        if action.class() != RecoveryActionClass::OrdinaryMutating {
            return Err(RecoveryContractError(
                "ordinary recovery receipt is legal only for ordinary mutating actions",
            ));
        }
        if action
            .expected_release_receipt_id()
            .is_some_and(|expected| expected != &receipt_id)
        {
            return Err(RecoveryContractError(
                "lease-release outcome receipt id does not match its planned action",
            ));
        }
        let projection = validate_outcome_projection(action, observations)?;
        let receipt = RecoveryActionEffectReceipt::test_only(
            receipt_id,
            action,
            projection.terminal_observation_digests.clone(),
        )?;
        if !receipt.validates(action, &projection.terminal_observation_digests) {
            return Err(RecoveryContractError(
                "ordinary recovery receipt does not bind its action outcome",
            ));
        }
        Self::effect_outcome(
            projection,
            EffectReceipt(EffectReceiptKind::RecoveryAction(receipt)),
            recovered,
        )
    }

    #[cfg(test)]
    pub(crate) fn performed_from_prearm_receipt_test_only(
        action: &RecoveryAction,
        observations: Vec<RecoveryObservation>,
        receipt: PreArmCancellationEffectReceipt,
    ) -> Result<Self, RecoveryContractError> {
        Self::prearm_effect_outcome(action, observations, receipt, false)
    }

    #[cfg(test)]
    pub(crate) fn recovered_from_prearm_receipt_test_only(
        action: &RecoveryAction,
        observations: Vec<RecoveryObservation>,
        receipt: PreArmCancellationEffectReceipt,
    ) -> Result<Self, RecoveryContractError> {
        Self::prearm_effect_outcome(action, observations, receipt, true)
    }

    fn prearm_effect_outcome(
        action: &RecoveryAction,
        observations: Vec<RecoveryObservation>,
        receipt: PreArmCancellationEffectReceipt,
        recovered: bool,
    ) -> Result<Self, RecoveryContractError> {
        if action.class() != RecoveryActionClass::PreArmEffect {
            return Err(RecoveryContractError(
                "pre-arm receipt is legal only for a pre-arm effect action",
            ));
        }
        let projection = validate_outcome_projection(action, observations)?;
        let (receipt_ref, expected_effect_kind) =
            action
                .prearm_receipt_binding()
                .ok_or(RecoveryContractError(
                    "pre-arm action lacks its effect receipt binding",
                ))?;
        let (action_id, _, _, action_digest) = action.common();
        if receipt_ref.source() != PreArmCancellationReceiptSource::FinalizationPlan
            || receipt_ref.effect_kind() != expected_effect_kind
            || receipt_ref.receipt_id() != receipt.receipt_id()
            || receipt_ref.effect_intent_digest() != receipt.effect_intent_digest()
            || receipt.effect_kind() != expected_effect_kind
            || receipt.producer_action_id() != action_id
            || receipt.producer_action_digest() != action_digest
            || receipt.terminal_observation_digests()
                != projection.terminal_observation_digests.as_slice()
        {
            return Err(RecoveryContractError(
                "pre-arm effect receipt does not bind its planned action outcome",
            ));
        }
        Self::effect_outcome(
            projection,
            EffectReceipt(EffectReceiptKind::PreArmCancellationEffect(receipt)),
            recovered,
        )
    }

    fn effect_outcome(
        projection: ValidatedOutcomeProjection,
        receipt: EffectReceipt,
        recovered: bool,
    ) -> Result<Self, RecoveryContractError> {
        if recovered {
            let record = RecoveryActionOutcomeDigestRecord(
                RecoveryActionOutcomeDigestRecordKind::RecoveredReceipt(
                    RecoveredReceiptActionOutcomeDigestRecord {
                        outcome: RecoveredReceiptOutcome::Value,
                        action_id: projection.action_id.clone(),
                        action_digest: projection.action_digest.clone(),
                        expected_postcondition_digest: projection
                            .expected_postcondition_digest
                            .clone(),
                        observed_postcondition_digest: projection
                            .observed_postcondition_digest
                            .clone(),
                        receipt: receipt.clone(),
                        terminal_observation_digests: projection
                            .terminal_observation_digests
                            .clone(),
                    },
                ),
            );
            let outcome_digest = contract_digest(&record, "recovery outcome digest failed")?;
            Ok(Self(RecoveryActionOutcomeKind::RecoveredReceipt(
                RecoveredReceiptActionOutcome {
                    outcome: RecoveredReceiptOutcome::Value,
                    action_id: projection.action_id,
                    action_digest: projection.action_digest,
                    expected_postcondition_digest: projection.expected_postcondition_digest,
                    observed_postcondition_digest: projection.observed_postcondition_digest,
                    receipt,
                    terminal_observation_digests: projection.terminal_observation_digests,
                    outcome_digest,
                },
            )))
        } else {
            let record = RecoveryActionOutcomeDigestRecord(
                RecoveryActionOutcomeDigestRecordKind::Performed(
                    PerformedRecoveryActionOutcomeDigestRecord {
                        outcome: PerformedOutcome::Value,
                        action_id: projection.action_id.clone(),
                        action_digest: projection.action_digest.clone(),
                        expected_postcondition_digest: projection
                            .expected_postcondition_digest
                            .clone(),
                        observed_postcondition_digest: projection
                            .observed_postcondition_digest
                            .clone(),
                        receipt: receipt.clone(),
                        terminal_observation_digests: projection
                            .terminal_observation_digests
                            .clone(),
                    },
                ),
            );
            let outcome_digest = contract_digest(&record, "recovery outcome digest failed")?;
            Ok(Self(RecoveryActionOutcomeKind::Performed(
                PerformedRecoveryActionOutcome {
                    outcome: PerformedOutcome::Value,
                    action_id: projection.action_id,
                    action_digest: projection.action_digest,
                    expected_postcondition_digest: projection.expected_postcondition_digest,
                    observed_postcondition_digest: projection.observed_postcondition_digest,
                    receipt,
                    terminal_observation_digests: projection.terminal_observation_digests,
                    outcome_digest,
                },
            )))
        }
    }
}

/// Hash of the durable staged-core container bytes. This internal type is
/// intentionally distinct from [`PublishedArchiveSha256`], so a staged hash
/// cannot accidentally satisfy a final-publication API.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StagedArchiveSha256(Sha256Digest);

impl StagedArchiveSha256 {
    pub(crate) const fn as_digest(&self) -> &Sha256Digest {
        &self.0
    }

    #[cfg(test)]
    fn test_only(value: Sha256Digest) -> Self {
        Self(value)
    }
}

/// Hash of the exact immutable final published container bytes. This is never
/// derived from a member list or publication-manifest digest.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct PublishedArchiveSha256(Sha256Digest);

impl PublishedArchiveSha256 {
    pub(crate) const fn as_digest(&self) -> &Sha256Digest {
        &self.0
    }

    #[cfg(test)]
    pub(crate) fn test_only(value: Sha256Digest) -> Self {
        Self(value)
    }
}

#[derive(Debug, PartialEq, Eq)]
enum ImmutableArchiveByteLayer {
    Staged {
        archive_id: UnicaId,
        handoff_lineage_digest: Sha256Digest,
        frozen_provider_boundary_digest: Sha256Digest,
    },
    Published {
        archive_id: UnicaId,
        publication_manifest_digest: Sha256Digest,
    },
}

/// One writer-produced, immutable byte generation. It is deliberately
/// non-`Clone`: hashing and strict parsing happen together in the consuming
/// observation methods below, so neither observer can reopen a path or inspect
/// another generation.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ImmutableArchiveByteGeneration {
    archive_container_capability_row_id: CapabilityRowId,
    generation_id: UnicaId,
    byte_observation_id: UnicaId,
    layer: ImmutableArchiveByteLayer,
    bytes: Box<[u8]>,
    durable_write_receipt_id: UnicaId,
}

/// Cohesive staged-writer lineage consumed together with the exact staged
/// bytes. Keeping the lineage in one authority also prevents field-by-field
/// staging calls from becoming an accidental splice surface.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ArchiveStagingWriterLineage {
    archive_container_capability_row_id: CapabilityRowId,
    generation_id: UnicaId,
    byte_observation_id: UnicaId,
    archive_id: UnicaId,
    handoff_lineage_digest: Sha256Digest,
    frozen_provider_boundary_digest: Sha256Digest,
    durable_write_receipt_id: UnicaId,
}

impl ArchiveStagingWriterLineage {
    #[cfg(test)]
    fn test_only(
        archive_container_capability_row_id: CapabilityRowId,
        generation_id: UnicaId,
        byte_observation_id: UnicaId,
        archive_id: UnicaId,
        handoff_lineage_digest: Sha256Digest,
        frozen_provider_boundary_digest: Sha256Digest,
        durable_write_receipt_id: UnicaId,
    ) -> Self {
        Self {
            archive_container_capability_row_id,
            generation_id,
            byte_observation_id,
            archive_id,
            handoff_lineage_digest,
            frozen_provider_boundary_digest,
            durable_write_receipt_id,
        }
    }
}

impl ImmutableArchiveByteGeneration {
    /// Test-only raw mint. The production writer/fsync adapter that creates
    /// this sealed generation belongs to the handler integration task.
    #[cfg(test)]
    fn seal_staged_from_writer_test_only(
        lineage: ArchiveStagingWriterLineage,
        bytes: Vec<u8>,
    ) -> Result<Self, RecoveryContractError> {
        Self::seal(
            lineage.archive_container_capability_row_id,
            lineage.generation_id,
            lineage.byte_observation_id,
            ImmutableArchiveByteLayer::Staged {
                archive_id: lineage.archive_id,
                handoff_lineage_digest: lineage.handoff_lineage_digest,
                frozen_provider_boundary_digest: lineage.frozen_provider_boundary_digest,
            },
            bytes,
            lineage.durable_write_receipt_id,
        )
    }

    /// Test-only raw mint for final bytes. Production minting is reserved for
    /// the paired writer/durable-publication adapter.
    #[cfg(test)]
    fn seal_published_from_writer_test_only(
        archive_container_capability_row_id: CapabilityRowId,
        generation_id: UnicaId,
        final_byte_observation_id: UnicaId,
        archive_id: UnicaId,
        publication_manifest_digest: Sha256Digest,
        bytes: Vec<u8>,
        durable_write_receipt_id: UnicaId,
    ) -> Result<Self, RecoveryContractError> {
        Self::seal(
            archive_container_capability_row_id,
            generation_id,
            final_byte_observation_id,
            ImmutableArchiveByteLayer::Published {
                archive_id,
                publication_manifest_digest,
            },
            bytes,
            durable_write_receipt_id,
        )
    }

    fn seal(
        archive_container_capability_row_id: CapabilityRowId,
        generation_id: UnicaId,
        byte_observation_id: UnicaId,
        layer: ImmutableArchiveByteLayer,
        bytes: Vec<u8>,
        durable_write_receipt_id: UnicaId,
    ) -> Result<Self, RecoveryContractError> {
        if bytes.is_empty() {
            return Err(RecoveryContractError(
                "immutable archive byte generation cannot be empty",
            ));
        }
        Ok(Self {
            archive_container_capability_row_id,
            generation_id,
            byte_observation_id,
            layer,
            bytes: bytes.into_boxed_slice(),
            durable_write_receipt_id,
        })
    }

    pub(crate) fn observe_staging(
        self,
        parser: &dyn ArchiveStagingStrictParser,
    ) -> Result<ArchiveStagingObservation, RecoveryContractError> {
        let ImmutableArchiveByteLayer::Staged {
            archive_id,
            handoff_lineage_digest,
            frozen_provider_boundary_digest,
        } = self.layer
        else {
            return Err(RecoveryContractError(
                "final archive bytes cannot produce a staging observation",
            ));
        };
        let staged_entry_manifest_digest =
            parser.parse_staged_generation(&self.generation_id, &self.bytes)?;
        let staged_archive_sha256 = StagedArchiveSha256(hash_archive_bytes(&self.bytes)?);
        Ok(ArchiveStagingObservation {
            archive_container_capability_row_id: self.archive_container_capability_row_id,
            generation_id: self.generation_id,
            byte_observation_id: self.byte_observation_id,
            archive_id,
            handoff_lineage_digest,
            frozen_provider_boundary_digest,
            staged_archive_sha256,
            staged_entry_manifest_digest,
            durable_write_receipt_id: self.durable_write_receipt_id,
        })
    }

    pub(crate) fn observe_publication(
        self,
        parser: &dyn ArchivePublicationStrictParser,
    ) -> Result<ArchivePublicationByteObservation, RecoveryContractError> {
        let ImmutableArchiveByteLayer::Published {
            archive_id,
            publication_manifest_digest,
        } = self.layer
        else {
            return Err(RecoveryContractError(
                "staged archive bytes cannot produce a publication observation",
            ));
        };
        let parsed = parser.parse_published_generation(&self.generation_id, &self.bytes)?;
        if parsed.embedded_manifest_digest != publication_manifest_digest {
            return Err(RecoveryContractError(
                "strict parser embedded manifest differs from the writer-bound manifest",
            ));
        }
        let final_archive_size = u64::try_from(self.bytes.len()).map_err(|_| {
            RecoveryContractError("final archive byte count exceeds the supported range")
        })?;
        let final_archive_sha256 = PublishedArchiveSha256(hash_archive_bytes(&self.bytes)?);
        Ok(ArchivePublicationByteObservation {
            archive_container_capability_row_id: self.archive_container_capability_row_id,
            generation_id: self.generation_id,
            final_byte_observation_id: self.byte_observation_id,
            archive_id,
            publication_manifest_digest,
            parsed_entry_set_digest: parsed.parsed_entry_set_digest,
            final_archive_size,
            final_archive_sha256,
            durable_write_receipt_id: self.durable_write_receipt_id,
        })
    }
}

fn hash_archive_bytes(bytes: &[u8]) -> Result<Sha256Digest, RecoveryContractError> {
    Sha256Digest::parse(&format!("{:x}", Sha256::digest(bytes)))
        .map_err(|_| RecoveryContractError("archive byte hashing produced an invalid digest"))
}

mod archive_parser_sealed {
    pub trait Sealed {}
}

/// Strict staged-core parser backed by the same concrete container capability
/// as the writer. The sealed adapter receives bytes, never a reopenable path.
pub(crate) trait ArchiveStagingStrictParser: archive_parser_sealed::Sealed {
    fn parse_staged_generation(
        &self,
        generation_id: &UnicaId,
        bytes: &[u8],
    ) -> Result<Sha256Digest, RecoveryContractError>;
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ArchiveStagingObservation {
    archive_container_capability_row_id: CapabilityRowId,
    generation_id: UnicaId,
    byte_observation_id: UnicaId,
    archive_id: UnicaId,
    handoff_lineage_digest: Sha256Digest,
    frozen_provider_boundary_digest: Sha256Digest,
    staged_archive_sha256: StagedArchiveSha256,
    staged_entry_manifest_digest: Sha256Digest,
    durable_write_receipt_id: UnicaId,
}

impl ArchiveStagingObservation {
    pub(crate) const fn archive_container_capability_row_id(&self) -> &CapabilityRowId {
        &self.archive_container_capability_row_id
    }

    pub(crate) const fn generation_id(&self) -> &UnicaId {
        &self.generation_id
    }

    pub(crate) const fn byte_observation_id(&self) -> &UnicaId {
        &self.byte_observation_id
    }

    pub(crate) const fn archive_id(&self) -> &UnicaId {
        &self.archive_id
    }

    pub(crate) const fn handoff_lineage_digest(&self) -> &Sha256Digest {
        &self.handoff_lineage_digest
    }

    pub(crate) const fn frozen_provider_boundary_digest(&self) -> &Sha256Digest {
        &self.frozen_provider_boundary_digest
    }

    pub(crate) const fn staged_archive_sha256(&self) -> &StagedArchiveSha256 {
        &self.staged_archive_sha256
    }

    pub(crate) const fn staged_entry_manifest_digest(&self) -> &Sha256Digest {
        &self.staged_entry_manifest_digest
    }

    pub(crate) const fn durable_write_receipt_id(&self) -> &UnicaId {
        &self.durable_write_receipt_id
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ArchivePublicationParsedDigests {
    parsed_entry_set_digest: Sha256Digest,
    embedded_manifest_digest: Sha256Digest,
}

impl ArchivePublicationParsedDigests {
    fn new(parsed_entry_set_digest: Sha256Digest, embedded_manifest_digest: Sha256Digest) -> Self {
        Self {
            parsed_entry_set_digest,
            embedded_manifest_digest,
        }
    }
}

pub(crate) trait ArchivePublicationStrictParser: archive_parser_sealed::Sealed {
    fn parse_published_generation(
        &self,
        generation_id: &UnicaId,
        bytes: &[u8],
    ) -> Result<ArchivePublicationParsedDigests, RecoveryContractError>;
}

/// Sealed final-byte evidence consumed by the result-owned publication
/// observation. It is deliberately not serializable and cannot be cloned.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ArchivePublicationByteObservation {
    archive_container_capability_row_id: CapabilityRowId,
    generation_id: UnicaId,
    final_byte_observation_id: UnicaId,
    archive_id: UnicaId,
    publication_manifest_digest: Sha256Digest,
    parsed_entry_set_digest: Sha256Digest,
    final_archive_size: u64,
    final_archive_sha256: PublishedArchiveSha256,
    durable_write_receipt_id: UnicaId,
}

#[cfg(test)]
pub(crate) struct ArchivePublicationByteObservationTestParts {
    pub(crate) archive_container_capability_row_id: CapabilityRowId,
    pub(crate) generation_id: UnicaId,
    pub(crate) final_byte_observation_id: UnicaId,
    pub(crate) archive_id: UnicaId,
    pub(crate) publication_manifest_digest: Sha256Digest,
    pub(crate) parsed_entry_set_digest: Sha256Digest,
    pub(crate) final_archive_size: u64,
    pub(crate) final_archive_sha256: PublishedArchiveSha256,
    pub(crate) durable_write_receipt_id: UnicaId,
}

impl ArchivePublicationByteObservation {
    pub(crate) const fn archive_container_capability_row_id(&self) -> &CapabilityRowId {
        &self.archive_container_capability_row_id
    }

    pub(crate) const fn generation_id(&self) -> &UnicaId {
        &self.generation_id
    }

    pub(crate) const fn final_byte_observation_id(&self) -> &UnicaId {
        &self.final_byte_observation_id
    }

    pub(crate) const fn archive_id(&self) -> &UnicaId {
        &self.archive_id
    }

    pub(crate) const fn publication_manifest_digest(&self) -> &Sha256Digest {
        &self.publication_manifest_digest
    }

    pub(crate) const fn parsed_entry_set_digest(&self) -> &Sha256Digest {
        &self.parsed_entry_set_digest
    }

    pub(crate) const fn final_archive_size(&self) -> u64 {
        self.final_archive_size
    }

    pub(crate) const fn final_archive_sha256(&self) -> &PublishedArchiveSha256 {
        &self.final_archive_sha256
    }

    pub(crate) const fn durable_write_receipt_id(&self) -> &UnicaId {
        &self.durable_write_receipt_id
    }

    pub(crate) fn into_final_archive_sha256(self) -> PublishedArchiveSha256 {
        self.final_archive_sha256
    }

    #[cfg(test)]
    pub(crate) fn test_only(
        parts: ArchivePublicationByteObservationTestParts,
    ) -> Result<Self, RecoveryContractError> {
        if parts.final_archive_size == 0 {
            return Err(RecoveryContractError(
                "test publication observation requires non-empty final bytes",
            ));
        }
        Ok(Self {
            archive_container_capability_row_id: parts.archive_container_capability_row_id,
            generation_id: parts.generation_id,
            final_byte_observation_id: parts.final_byte_observation_id,
            archive_id: parts.archive_id,
            publication_manifest_digest: parts.publication_manifest_digest,
            parsed_entry_set_digest: parts.parsed_entry_set_digest,
            final_archive_size: parts.final_archive_size,
            final_archive_sha256: parts.final_archive_sha256,
            durable_write_receipt_id: parts.durable_write_receipt_id,
        })
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ArchiveStagingReceiptAuthority {
    record: ArchiveStagingReceiptDigestRecord,
}

impl ArchiveStagingReceiptAuthority {
    pub(crate) fn from_observation(
        staging_receipt_id: UnicaId,
        expected_staged_entry_manifest_digest: Sha256Digest,
        observation: ArchiveStagingObservation,
    ) -> Result<Self, RecoveryContractError> {
        if observation.staged_entry_manifest_digest != expected_staged_entry_manifest_digest {
            return Err(RecoveryContractError(
                "staged archive parser manifest differs from the approved staged manifest",
            ));
        }
        Ok(Self {
            record: ArchiveStagingReceiptDigestRecord {
                staging_receipt_id,
                archive_id: observation.archive_id,
                handoff_lineage_digest: observation.handoff_lineage_digest,
                frozen_provider_boundary_digest: observation.frozen_provider_boundary_digest,
                staged_archive_sha256: observation.staged_archive_sha256.0,
                file_synced: TrueLiteral,
                parent_directory_synced: TrueLiteral,
                durable_write_receipt_id: observation.durable_write_receipt_id,
            },
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ArchiveStagingReceiptDigestRecord {
    staging_receipt_id: UnicaId,
    archive_id: UnicaId,
    handoff_lineage_digest: Sha256Digest,
    frozen_provider_boundary_digest: Sha256Digest,
    staged_archive_sha256: Sha256Digest,
    file_synced: TrueLiteral,
    parent_directory_synced: TrueLiteral,
    durable_write_receipt_id: UnicaId,
}

impl contract_digest_record_sealed::Sealed for ArchiveStagingReceiptDigestRecord {}
impl ContractDigestRecord for ArchiveStagingReceiptDigestRecord {}

/// Durable-before-release archive handoff evidence. Deliberately not `Deserialize`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ArchiveStagingReceipt {
    staging_receipt_id: UnicaId,
    archive_id: UnicaId,
    handoff_lineage_digest: Sha256Digest,
    frozen_provider_boundary_digest: Sha256Digest,
    staged_archive_sha256: Sha256Digest,
    file_synced: TrueLiteral,
    parent_directory_synced: TrueLiteral,
    durable_write_receipt_id: UnicaId,
    receipt_digest: Sha256Digest,
}

impl ArchiveStagingReceipt {
    pub(crate) fn new(
        authority: ArchiveStagingReceiptAuthority,
    ) -> Result<Self, RecoveryContractError> {
        let record = authority.record;
        let receipt_digest = contract_digest(&record, "archive staging receipt digest failed")?;
        Ok(Self {
            staging_receipt_id: record.staging_receipt_id,
            archive_id: record.archive_id,
            handoff_lineage_digest: record.handoff_lineage_digest,
            frozen_provider_boundary_digest: record.frozen_provider_boundary_digest,
            staged_archive_sha256: record.staged_archive_sha256,
            file_synced: record.file_synced,
            parent_directory_synced: record.parent_directory_synced,
            durable_write_receipt_id: record.durable_write_receipt_id,
            receipt_digest,
        })
    }

    #[cfg(test)]
    pub(crate) fn test_only(
        staging_receipt_id: UnicaId,
        archive_id: UnicaId,
        handoff_lineage_digest: Sha256Digest,
        frozen_provider_boundary_digest: Sha256Digest,
        staged_archive_sha256: Sha256Digest,
        durable_write_receipt_id: UnicaId,
    ) -> Result<Self, RecoveryContractError> {
        let record = ArchiveStagingReceiptDigestRecord {
            staging_receipt_id,
            archive_id,
            handoff_lineage_digest,
            frozen_provider_boundary_digest,
            staged_archive_sha256,
            file_synced: TrueLiteral,
            parent_directory_synced: TrueLiteral,
            durable_write_receipt_id,
        };
        Self::new(ArchiveStagingReceiptAuthority {
            record: ArchiveStagingReceiptDigestRecord {
                staging_receipt_id: record.staging_receipt_id,
                archive_id: record.archive_id,
                handoff_lineage_digest: record.handoff_lineage_digest,
                frozen_provider_boundary_digest: record.frozen_provider_boundary_digest,
                staged_archive_sha256: record.staged_archive_sha256,
                file_synced: record.file_synced,
                parent_directory_synced: record.parent_directory_synced,
                durable_write_receipt_id: record.durable_write_receipt_id,
            },
        })
    }

    pub(crate) const fn staging_receipt_id(&self) -> &UnicaId {
        &self.staging_receipt_id
    }

    pub(crate) const fn archive_id(&self) -> &UnicaId {
        &self.archive_id
    }

    pub(crate) const fn handoff_lineage_digest(&self) -> &Sha256Digest {
        &self.handoff_lineage_digest
    }

    pub(crate) const fn frozen_provider_boundary_digest(&self) -> &Sha256Digest {
        &self.frozen_provider_boundary_digest
    }

    pub(crate) const fn staged_archive_sha256(&self) -> &Sha256Digest {
        &self.staged_archive_sha256
    }

    pub(crate) const fn durable_write_receipt_id(&self) -> &UnicaId {
        &self.durable_write_receipt_id
    }

    pub(crate) const fn receipt_digest(&self) -> &Sha256Digest {
        &self.receipt_digest
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::branched_development::contracts::artifacts::OwnedTargetRole;
    use crate::domain::branched_development::contracts::instructions::{
        ManualWorkingInfobaseCleanupReason, SupportCorrectiveInstructionAuthority,
        SupportRecoveryTransition,
    };
    use crate::domain::branched_development::contracts::prearm_recovery::{
        archive_outcome_fixture_test_only, PreArmCancellationEffectReceiptAuthority,
    };
    use crate::domain::branched_development::contracts::repository::{
        EvidenceSourceIndex, EvidenceSourceIndexCandidate, EvidenceSourceRegistry,
        RepositoryContractError, RepositoryHistoryEvidenceBytesResolver,
        RepositoryHistoryOrderEvidence, RepositoryHistoryOrderResolver,
        RepositoryHistoryPartitionResolver, RepositoryHistorySourceEvidenceRef,
        RepositoryUpdateLockReason, UnvalidatedRepositoryHistoryPartition,
    };
    use crate::domain::branched_development::contracts::scalars::{
        Diagnostic, RepositoryTargetDisplay, RepositoryUsername, RepositoryVersion,
        RequiredNullable,
    };
    use crate::domain::branched_development::contracts::schema::audit_json_schema;
    use crate::domain::branched_development::contracts::support::{
        ManualActorLockInventoryProof, SupportActionPurpose, SupportBlockers, SupportContractError,
        SupportEvidenceGaps, SupportHistoryOrderAuthority, SupportTransition,
        SupportTransitionConflict, SupportTransitionConflicts, SupportTransitionOverlapKind,
    };
    use crate::domain::branched_development::contracts::support_terminalization::{
        ManualWorkingInfobaseClosurePlanAuthority, ManualWorkingInfobaseStopAuthority,
        SupportRecoveryDesiredTarget, SupportRecoveryDesiredTargets,
        SupportRecoveryFinalizationPlanAuthority, SupportRecoveryLockTarget,
        SupportRecoveryLockTargets,
    };
    use crate::domain::branched_development::{ProjectId, SupportLayerId};
    use schemars::{schema_for, JsonSchema};
    use serde::de::DeserializeOwned;
    use serde::Serialize;
    use serde_json::{json, Value};
    use std::cmp::Ordering;

    const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const ID_1: &str = "11111111-1111-4111-8111-111111111111";
    const ID_2: &str = "22222222-2222-4222-8222-222222222222";

    fn digest(value: &str) -> Sha256Digest {
        Sha256Digest::parse(value).unwrap()
    }

    fn id(value: &str) -> UnicaId {
        UnicaId::parse(value).unwrap()
    }

    fn typed_cleanup_targets() -> Vec<OwnedTargetLocator> {
        let first_project = ProjectId::parse("10000000-0000-4000-8000-000000000000").unwrap();
        let second_project = ProjectId::parse("20000000-0000-4000-8000-000000000000").unwrap();
        let late_instance = id("f0000000-0000-4000-8000-000000000000");
        let early_instance = id("00000000-0000-4000-8000-000000000000");
        let mut targets = [
            OwnedTargetRole::InstanceRoot,
            OwnedTargetRole::TaskInfobase,
            OwnedTargetRole::TaskWorkspace,
            OwnedTargetRole::Probe,
            OwnedTargetRole::Sandbox,
            OwnedTargetRole::Artifact,
            OwnedTargetRole::Quarantine,
        ]
        .into_iter()
        .map(|role| OwnedTargetLocator::new(first_project.clone(), late_instance.clone(), role))
        .collect::<Vec<_>>();
        // Typed locator order is project first. Deliberately give the later
        // project an earlier instance so JSON object-key order cannot pass.
        targets.push(OwnedTargetLocator::new(
            second_project,
            early_instance,
            OwnedTargetRole::InstanceRoot,
        ));
        targets
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

    fn runtime_action(
        record: RecoveryActionDigestRecordKind,
    ) -> Result<RecoveryAction, RecoveryContractError> {
        let action = RecoveryAction::from_record(record);
        if let Ok(action) = &action {
            let encoded = serde_json::to_value(action).unwrap();
            assert!(
                schema_accepts::<RecoveryAction>(&encoded),
                "runtime constructor emitted an action rejected by its schema: {encoded}"
            );
        }
        action
    }

    fn verify_task_action(
        observation_kind: RecoveryObservationKind,
        observation_digest: Sha256Digest,
        expected_task_fingerprint: Sha256Digest,
    ) -> Result<RecoveryAction, RecoveryContractError> {
        let (expected_observations, expected_postcondition_digest) =
            expected_postcondition(vec![RecoveryExpectedObservation::new(
                observation_kind,
                RecoverySubjectRef::registered(id(ID_1)),
                observation_digest,
            )])?;
        runtime_action(RecoveryActionDigestRecordKind::VerifyTaskFingerprint(
            VerifyTaskFingerprintActionDigestRecord {
                action_kind: VerifyTaskFingerprintActionKind::Value,
                action_id: id(ID_1),
                expected_task_fingerprint,
                expected_observations,
                expected_postcondition_digest,
            },
        ))
    }

    fn release_root_lock_action() -> Result<RecoveryAction, RecoveryContractError> {
        let subject = RecoverySubjectRef::configuration_root();
        let (expected_observations, expected_postcondition_digest) =
            expected_postcondition(vec![RecoveryExpectedObservation::new(
                RecoveryObservationKind::LockOwnership,
                subject.clone(),
                digest(A),
            )])?;
        runtime_action(RecoveryActionDigestRecordKind::ReleaseOwnedLocks(
            ReleaseOwnedLocksActionDigestRecord {
                action_kind: ReleaseOwnedLocksActionKind::Value,
                action_id: id(ID_1),
                subjects: RecoverySubjects::new(vec![subject])?,
                expected_owned_lock_set_digest: digest(A),
                expected_observations,
                expected_postcondition_digest,
            },
        ))
    }

    fn observe_commit_action() -> Result<RecoveryAction, RecoveryContractError> {
        let integration_set_id = id("22222222-2222-4222-8222-222222222222");
        let (expected_observations, expected_postcondition_digest) =
            expected_postcondition(vec![RecoveryExpectedObservation::new(
                RecoveryObservationKind::RepositoryVersion,
                RecoverySubjectRef::registered(integration_set_id.clone()),
                digest(A),
            )])?;
        runtime_action(RecoveryActionDigestRecordKind::ObserveCommit(
            ObserveCommitActionDigestRecord {
                action_kind: ObserveCommitActionKind::Value,
                action_id: id(ID_1),
                operation_id: OperationId::parse("33333333-3333-4333-8333-333333333333").unwrap(),
                integration_set_id,
                expected_integration_set_digest: digest(A),
                expected_observations,
                expected_postcondition_digest,
            },
        ))
    }

    fn restore_original_action() -> Result<RecoveryAction, RecoveryContractError> {
        let (expected_observations, expected_postcondition_digest) =
            expected_postcondition(vec![RecoveryExpectedObservation::new(
                RecoveryObservationKind::ObjectFingerprint,
                RecoverySubjectRef::reserved_original_infobase(digest(B)),
                digest(A),
            )])?;
        runtime_action(RecoveryActionDigestRecordKind::RestoreOriginal(
            RestoreOriginalActionDigestRecord {
                action_kind: RestoreOriginalActionKind::Value,
                action_id: id(ID_1),
                checkpoint_id: id("33333333-3333-4333-8333-333333333333"),
                expected_original_fingerprint: digest(A),
                expected_observations,
                expected_postcondition_digest,
            },
        ))
    }

    fn release_root_lock_action_two() -> Result<RecoveryAction, RecoveryContractError> {
        let subject = RecoverySubjectRef::configuration_root();
        let (expected_observations, expected_postcondition_digest) =
            expected_postcondition(vec![RecoveryExpectedObservation::new(
                RecoveryObservationKind::LockOwnership,
                subject.clone(),
                digest(A),
            )])?;
        runtime_action(RecoveryActionDigestRecordKind::ReleaseOwnedLocks(
            ReleaseOwnedLocksActionDigestRecord {
                action_kind: ReleaseOwnedLocksActionKind::Value,
                action_id: id("22222222-2222-4222-8222-222222222222"),
                subjects: RecoverySubjects::new(vec![subject])?,
                expected_owned_lock_set_digest: digest(A),
                expected_observations,
                expected_postcondition_digest,
            },
        ))
    }

    fn acquire_prearm_root_action(
        receipt_ref: PreArmCancellationReceiptRef,
    ) -> Result<RecoveryAction, RecoveryContractError> {
        let (expected_observations, expected_postcondition_digest) =
            expected_postcondition(vec![RecoveryExpectedObservation::new(
                RecoveryObservationKind::LockOwnership,
                RecoverySubjectRef::configuration_root(),
                digest(A),
            )])?;
        runtime_action(RecoveryActionDigestRecordKind::AcquirePreArmRootGuard(
            AcquirePreArmRootGuardActionDigestRecord {
                action_kind: AcquirePreArmRootGuardActionKind::Value,
                action_id: id(ID_1),
                finalization_attempt_id: id("22222222-2222-4222-8222-222222222222"),
                finalization_plan_digest: digest(A),
                support_action_id: id("33333333-3333-4333-8333-333333333333"),
                receipt_ref,
                expected_observations,
                expected_postcondition_digest,
            },
        ))
    }

    fn selective_prearm_action(
        receipt_ref: PreArmCancellationReceiptRef,
    ) -> Result<RecoveryAction, RecoveryContractError> {
        let (expected_observations, expected_postcondition_digest) = expected_postcondition(vec![
            RecoveryExpectedObservation::new(
                RecoveryObservationKind::ObjectFingerprint,
                RecoverySubjectRef::metadata_object(MetadataObjectId::parse(ID_1).unwrap()),
                digest(A),
            ),
            RecoveryExpectedObservation::new(
                RecoveryObservationKind::ObjectFingerprint,
                RecoverySubjectRef::configuration_root(),
                digest(A),
            ),
        ])?;
        runtime_action(
            RecoveryActionDigestRecordKind::ApplyPreArmCancellationSelectiveUpdate(
                ApplyPreArmCancellationSelectiveUpdateActionDigestRecord {
                    action_kind: ApplyPreArmCancellationSelectiveUpdateActionKind::Value,
                    action_id: id("22222222-2222-4222-8222-222222222222"),
                    finalization_attempt_id: id("44444444-4444-4444-8444-444444444444"),
                    finalization_plan_digest: digest(A),
                    selective_update_plan_digest: digest(B),
                    expected_target_revision_map_digest: digest(A),
                    receipt_ref,
                    expected_observations,
                    expected_postcondition_digest,
                },
            ),
        )
    }

    fn working_identity() -> ManualWorkingInfobaseIdentity {
        use crate::domain::branched_development::contracts::scalars::RepositoryIdentityComponent;

        ManualWorkingInfobaseIdentity::new(
            RepositoryIdentityComponent::parse("HOST").unwrap(),
            RepositoryIdentityComponent::parse("Working IB").unwrap(),
        )
        .unwrap()
    }

    fn recheck_prearm_action(
        expected_observations: Vec<RecoveryExpectedObservation>,
        recheck_policy_digest: Sha256Digest,
    ) -> Result<RecoveryAction, RecoveryContractError> {
        let (expected_observations, expected_postcondition_digest) =
            expected_postcondition(expected_observations)?;
        runtime_action(
            RecoveryActionDigestRecordKind::RecheckPreArmCancellationFinalization(
                RecheckPreArmCancellationFinalizationActionDigestRecord {
                    action_kind: RecheckPreArmCancellationFinalizationActionKind::Value,
                    action_id: id("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"),
                    finalization_attempt_id: id("44444444-4444-4444-8444-444444444444"),
                    finalization_plan_digest: digest(A),
                    effect_observation_digest: digest(B),
                    recheck_policy_digest,
                    expected_observations,
                    expected_postcondition_digest,
                },
            ),
        )
    }

    fn prearm_terminal_observations(
        mode_observation: RecoveryExpectedObservation,
    ) -> Vec<RecoveryExpectedObservation> {
        vec![
            RecoveryExpectedObservation::new(
                RecoveryObservationKind::SupportGraph,
                RecoverySubjectRef::configuration_root(),
                digest(A),
            ),
            RecoveryExpectedObservation::new(
                RecoveryObservationKind::SupportActionAuthorization,
                RecoverySubjectRef::registered(id("22222222-2222-4222-8222-222222222222")),
                digest(A),
            ),
            RecoveryExpectedObservation::new(
                RecoveryObservationKind::ObjectFingerprint,
                RecoverySubjectRef::registered(id("33333333-3333-4333-8333-333333333333")),
                digest(A),
            ),
            RecoveryExpectedObservation::new(
                RecoveryObservationKind::LockOwnership,
                RecoverySubjectRef::configuration_root(),
                digest(A),
            ),
            mode_observation,
        ]
    }

    struct ExactPreArmOutcomeFixture {
        observation: PreArmCancellationEffectObservation,
        plan: PreArmCancellationFinalizationPlan,
        progress: PreArmCancellationFinalizationAttemptProgress,
        actions: Vec<RecoveryAction>,
        outcomes: Vec<RecoveryActionOutcome>,
    }

    fn matched_action_observations(action: &RecoveryAction) -> Vec<RecoveryObservation> {
        action
            .common()
            .1
            .as_slice()
            .iter()
            .map(|expected| {
                RecoveryObservation::matched_test_only(
                    expected.observation_kind,
                    expected.subject.clone(),
                    expected.expected_digest.clone(),
                    expected.expected_digest.clone(),
                )
                .unwrap()
            })
            .collect()
    }

    fn exact_prearm_outcome_fixture(include_selective_update: bool) -> ExactPreArmOutcomeFixture {
        let reserved_identity = digest(A);
        let mode_capability = CapabilityRowId::parse("reserved-original-lease.v1").unwrap();
        let (root_observations, root_postcondition) =
            expected_postcondition(vec![RecoveryExpectedObservation::new(
                RecoveryObservationKind::LockOwnership,
                RecoverySubjectRef::configuration_root(),
                digest(A),
            )])
            .unwrap();
        let (mode_acquire_observations, mode_acquire_postcondition) =
            expected_postcondition(vec![RecoveryExpectedObservation::new(
                RecoveryObservationKind::ReservedOriginalLease,
                RecoverySubjectRef::reserved_original_infobase(reserved_identity.clone()),
                digest(A),
            )])
            .unwrap();
        let (update_observations, update_postcondition) =
            expected_postcondition(vec![RecoveryExpectedObservation::new(
                RecoveryObservationKind::ObjectFingerprint,
                RecoverySubjectRef::configuration_root(),
                digest(B),
            )])
            .unwrap();
        let (persist_observations, persist_postcondition) =
            expected_postcondition(vec![RecoveryExpectedObservation::new(
                RecoveryObservationKind::SupportActionAuthorization,
                RecoverySubjectRef::registered(id(ID_1)),
                digest(B),
            )])
            .unwrap();
        let (mode_release_observations, mode_release_postcondition) =
            expected_postcondition(vec![RecoveryExpectedObservation::new(
                RecoveryObservationKind::ReservedOriginalLease,
                RecoverySubjectRef::reserved_original_infobase(reserved_identity.clone()),
                digest(B),
            )])
            .unwrap();
        let (root_release_observations, root_release_postcondition) =
            expected_postcondition(vec![RecoveryExpectedObservation::new(
                RecoveryObservationKind::LockOwnership,
                RecoverySubjectRef::configuration_root(),
                digest(B),
            )])
            .unwrap();
        let (finish_observations, finish_postcondition) = expected_postcondition(vec![
            RecoveryExpectedObservation::new(
                RecoveryObservationKind::SupportGraph,
                RecoverySubjectRef::configuration_root(),
                digest(A),
            ),
            RecoveryExpectedObservation::new(
                RecoveryObservationKind::SupportActionAuthorization,
                RecoverySubjectRef::registered(id(ID_1)),
                digest(B),
            ),
            RecoveryExpectedObservation::new(
                RecoveryObservationKind::ObjectFingerprint,
                RecoverySubjectRef::registered(id(ID_2)),
                digest(B),
            ),
            RecoveryExpectedObservation::new(
                RecoveryObservationKind::LockOwnership,
                RecoverySubjectRef::configuration_root(),
                digest(B),
            ),
            RecoveryExpectedObservation::new(
                RecoveryObservationKind::ReservedOriginalLease,
                RecoverySubjectRef::reserved_original_infobase(reserved_identity.clone()),
                digest(B),
            ),
        ])
        .unwrap();

        let mut postconditions = vec![
            (
                PreArmCancellationEffectKind::RootGuardAcquire,
                root_postcondition.clone(),
            ),
            (
                PreArmCancellationEffectKind::ModeLeaseAcquire,
                mode_acquire_postcondition.clone(),
            ),
        ];
        if include_selective_update {
            postconditions.push((
                PreArmCancellationEffectKind::SelectiveOriginalUpdate,
                update_postcondition.clone(),
            ));
        }
        postconditions.extend([
            (
                PreArmCancellationEffectKind::AuthorizationCancellation,
                persist_postcondition.clone(),
            ),
            (
                PreArmCancellationEffectKind::ModeLeaseRelease,
                mode_release_postcondition.clone(),
            ),
            (
                PreArmCancellationEffectKind::RootGuardRelease,
                root_release_postcondition.clone(),
            ),
            (
                PreArmCancellationEffectKind::RecoveryFinalization,
                finish_postcondition.clone(),
            ),
        ]);
        let (observation, plan, evidence) =
            archive_outcome_fixture_test_only(include_selective_update, postconditions);
        let receipt_plan = plan.receipt_plan();

        let root = runtime_action(RecoveryActionDigestRecordKind::AcquirePreArmRootGuard(
            AcquirePreArmRootGuardActionDigestRecord {
                action_kind: AcquirePreArmRootGuardActionKind::Value,
                action_id: id(ID_1),
                finalization_attempt_id: plan.finalization_attempt_id().clone(),
                finalization_plan_digest: plan.finalization_plan_digest().clone(),
                support_action_id: plan.support_action_id().clone(),
                receipt_ref: receipt_plan.root_guard_acquisition_receipt().clone(),
                expected_observations: root_observations,
                expected_postcondition_digest: root_postcondition,
            },
        ))
        .unwrap();
        let mode_acquire = runtime_action(RecoveryActionDigestRecordKind::AcquirePreArmModeLease(
            AcquirePreArmModeLeaseActionDigestRecord(
                AcquirePreArmModeLeaseActionDigestRecordKind::ReservedOriginal(
                    AcquirePreArmReservedOriginalModeLeaseActionDigestRecord {
                        action_kind: AcquirePreArmModeLeaseActionKind::Value,
                        action_id: id(ID_2),
                        finalization_attempt_id: plan.finalization_attempt_id().clone(),
                        finalization_plan_digest: plan.finalization_plan_digest().clone(),
                        support_action_id: plan.support_action_id().clone(),
                        manual_target_mode: ReservedOriginalModeLiteral::Value,
                        reserved_original_identity_digest: reserved_identity.clone(),
                        exclusive_lease_capability_id: mode_capability.clone(),
                        receipt_ref: receipt_plan.mode_lease_acquisition_receipt().clone(),
                        expected_observations: mode_acquire_observations,
                        expected_postcondition_digest: mode_acquire_postcondition,
                    },
                ),
            ),
        ))
        .unwrap();
        let recheck_expected = vec![
            RecoveryExpectedObservation::new(
                RecoveryObservationKind::SupportGraph,
                RecoverySubjectRef::configuration_root(),
                digest(A),
            ),
            RecoveryExpectedObservation::new(
                RecoveryObservationKind::SupportActionAuthorization,
                RecoverySubjectRef::registered(plan.support_action_id().clone()),
                digest(A),
            ),
            RecoveryExpectedObservation::new(
                RecoveryObservationKind::ObjectFingerprint,
                RecoverySubjectRef::registered(id(ID_2)),
                digest(B),
            ),
            RecoveryExpectedObservation::new(
                RecoveryObservationKind::LockOwnership,
                RecoverySubjectRef::configuration_root(),
                digest(A),
            ),
            RecoveryExpectedObservation::new(
                RecoveryObservationKind::ReservedOriginalLease,
                RecoverySubjectRef::reserved_original_infobase(reserved_identity.clone()),
                digest(A),
            ),
            RecoveryExpectedObservation::new(
                RecoveryObservationKind::FinalizationPolicy,
                RecoverySubjectRef::registered(plan.finalization_attempt_id().clone()),
                plan.recheck_policy().policy_digest().clone(),
            ),
        ];
        let (recheck_observations, recheck_postcondition) =
            expected_postcondition(recheck_expected).unwrap();
        let recheck = runtime_action(
            RecoveryActionDigestRecordKind::RecheckPreArmCancellationFinalization(
                RecheckPreArmCancellationFinalizationActionDigestRecord {
                    action_kind: RecheckPreArmCancellationFinalizationActionKind::Value,
                    action_id: id("33333333-3333-4333-8333-333333333333"),
                    finalization_attempt_id: plan.finalization_attempt_id().clone(),
                    finalization_plan_digest: plan.finalization_plan_digest().clone(),
                    effect_observation_digest: plan.effect_observation_digest().clone(),
                    recheck_policy_digest: plan.recheck_policy().policy_digest().clone(),
                    expected_observations: recheck_observations,
                    expected_postcondition_digest: recheck_postcondition,
                },
            ),
        )
        .unwrap();

        let mut actions = vec![root, mode_acquire, recheck];
        if include_selective_update {
            actions.push(
                runtime_action(
                    RecoveryActionDigestRecordKind::ApplyPreArmCancellationSelectiveUpdate(
                        ApplyPreArmCancellationSelectiveUpdateActionDigestRecord {
                            action_kind: ApplyPreArmCancellationSelectiveUpdateActionKind::Value,
                            action_id: id("44444444-4444-4444-8444-444444444444"),
                            finalization_attempt_id: plan.finalization_attempt_id().clone(),
                            finalization_plan_digest: plan.finalization_plan_digest().clone(),
                            selective_update_plan_digest: plan
                                .selective_update_plan_digest()
                                .clone(),
                            expected_target_revision_map_digest: plan
                                .expected_target_revision_map_digest()
                                .clone(),
                            receipt_ref: receipt_plan
                                .selective_update_effect_receipt()
                                .unwrap()
                                .clone(),
                            expected_observations: update_observations,
                            expected_postcondition_digest: update_postcondition,
                        },
                    ),
                )
                .unwrap(),
            );
        }
        let persist_action_id = if include_selective_update {
            "55555555-5555-4555-8555-555555555555"
        } else {
            "44444444-4444-4444-8444-444444444444"
        };
        actions.push(
            runtime_action(
                RecoveryActionDigestRecordKind::PersistPreArmSupportCancellation(
                    PersistPreArmSupportCancellationActionDigestRecord {
                        action_kind: PersistPreArmSupportCancellationActionKind::Value,
                        action_id: id(persist_action_id),
                        finalization_attempt_id: plan.finalization_attempt_id().clone(),
                        support_action_id: plan.support_action_id().clone(),
                        expected_support_action_digest: plan
                            .expected_support_action_digest()
                            .clone(),
                        approved_cancellation_digest: plan.approved_cancellation_digest().clone(),
                        effect_observation_digest: plan.effect_observation_digest().clone(),
                        finalization_plan_digest: plan.finalization_plan_digest().clone(),
                        receipt_ref: receipt_plan.cancellation_persistence_receipt().clone(),
                        expected_observations: persist_observations,
                        expected_postcondition_digest: persist_postcondition,
                    },
                ),
            )
            .unwrap(),
        );
        let mode_release_action_id = if include_selective_update {
            "66666666-6666-4666-8666-666666666666"
        } else {
            "55555555-5555-4555-8555-555555555555"
        };
        actions.push(
            runtime_action(RecoveryActionDigestRecordKind::ReleasePreArmModeLease(
                ReleasePreArmModeLeaseActionDigestRecord(
                    ReleasePreArmModeLeaseActionDigestRecordKind::ReservedOriginal(
                        ReleasePreArmReservedOriginalModeLeaseActionDigestRecord {
                            action_kind: ReleasePreArmModeLeaseActionKind::Value,
                            action_id: id(mode_release_action_id),
                            finalization_attempt_id: plan.finalization_attempt_id().clone(),
                            finalization_plan_digest: plan.finalization_plan_digest().clone(),
                            manual_target_mode: ReservedOriginalModeLiteral::Value,
                            reserved_original_identity_digest: reserved_identity,
                            exclusive_lease_capability_id: mode_capability,
                            receipt_ref: receipt_plan.mode_lease_release_receipt().clone(),
                            expected_observations: mode_release_observations,
                            expected_postcondition_digest: mode_release_postcondition,
                        },
                    ),
                ),
            ))
            .unwrap(),
        );
        let root_release_action_id = if include_selective_update {
            "77777777-7777-4777-8777-777777777777"
        } else {
            "66666666-6666-4666-8666-666666666666"
        };
        actions.push(
            runtime_action(RecoveryActionDigestRecordKind::ReleasePreArmRootGuard(
                ReleasePreArmRootGuardActionDigestRecord {
                    action_kind: ReleasePreArmRootGuardActionKind::Value,
                    action_id: id(root_release_action_id),
                    finalization_attempt_id: plan.finalization_attempt_id().clone(),
                    finalization_plan_digest: plan.finalization_plan_digest().clone(),
                    support_action_id: plan.support_action_id().clone(),
                    receipt_ref: receipt_plan.root_guard_release_receipt().clone(),
                    expected_observations: root_release_observations,
                    expected_postcondition_digest: root_release_postcondition,
                },
            ))
            .unwrap(),
        );
        let finish_action_id = if include_selective_update {
            "88888888-8888-4888-8888-888888888888"
        } else {
            "77777777-7777-4777-8777-777777777777"
        };
        actions.push(
            runtime_action(
                RecoveryActionDigestRecordKind::FinishPreArmCancellationRecovery(
                    FinishPreArmCancellationRecoveryActionDigestRecord {
                        action_kind: FinishPreArmCancellationRecoveryActionKind::Value,
                        action_id: id(finish_action_id),
                        finalization_attempt_id: plan.finalization_attempt_id().clone(),
                        support_action_id: plan.support_action_id().clone(),
                        expected_support_action_digest: plan
                            .expected_support_action_digest()
                            .clone(),
                        approved_cancellation_digest: plan.approved_cancellation_digest().clone(),
                        effect_observation_digest: plan.effect_observation_digest().clone(),
                        finalization_plan_digest: plan.finalization_plan_digest().clone(),
                        receipt_plan_digest: plan.receipt_plan().receipt_plan_digest().clone(),
                        expected_result_phase: plan.planned_result_phase(),
                        receipt_ref: receipt_plan.recovery_finalization_receipt().clone(),
                        expected_observations: finish_observations,
                        expected_postcondition_digest: finish_postcondition,
                    },
                ),
            )
            .unwrap(),
        );

        validate_prearm_finalize_action_grammar(
            &observation,
            &plan,
            &PreArmCancellationFinalizationAttemptProgress::not_started_test_only(
                plan.finalization_attempt_id().clone(),
            ),
            &actions,
        )
        .unwrap();

        let mut outcomes = Vec::with_capacity(actions.len());
        let mut receipts = Vec::with_capacity(actions.len() - 1);
        for action in &actions {
            let matched = matched_action_observations(action);
            if matches!(
                action.0,
                RecoveryActionKindWire::RecheckPreArmCancellationFinalization(_)
            ) {
                outcomes.push(
                    RecoveryActionOutcome::already_satisfied_test_only(action, matched).unwrap(),
                );
                continue;
            }
            let (receipt_ref, effect_kind) = action.prearm_receipt_binding().unwrap();
            let terminal_observation_digests = matched
                .iter()
                .map(|value| value.observation_digest().clone())
                .collect();
            let receipt = PreArmCancellationEffectReceipt::new(
                PreArmCancellationEffectReceiptAuthority::test_only(
                    receipt_ref.receipt_id().clone(),
                    effect_kind,
                    receipt_ref.effect_intent_digest().clone(),
                    action.action_id().clone(),
                    action.action_digest().clone(),
                    terminal_observation_digests,
                )
                .unwrap(),
            )
            .unwrap();
            outcomes.push(
                RecoveryActionOutcome::performed_from_prearm_receipt_test_only(
                    action,
                    matched,
                    receipt.clone(),
                )
                .unwrap(),
            );
            receipts.push(receipt);
        }
        let progress = PreArmCancellationFinalizationAttemptProgress::completed_test_only(
            plan.finalization_attempt_id().clone(),
            receipts,
            evidence,
        )
        .unwrap();
        ExactPreArmOutcomeFixture {
            observation,
            plan,
            progress,
            actions,
            outcomes,
        }
    }

    pub(super) fn exact_prearm_archive_fixture_for_task(
    ) -> PreArmArchiveReceiptOutcomeWitnessTestFixture {
        let fixture = exact_prearm_outcome_fixture(false);
        let recheck_evidence = fixture
            .progress
            .completed_recheck_evidence()
            .unwrap()
            .clone();
        let witness = ValidatedPreArmArchiveReceiptOutcomeWitness::from_completed_outcomes(
            &fixture.observation,
            &fixture.plan,
            &fixture.progress,
            &fixture.actions,
            &fixture.outcomes,
        )
        .unwrap();
        PreArmArchiveReceiptOutcomeWitnessTestFixture {
            witness,
            observation: fixture.observation,
            plan: fixture.plan,
            recheck_evidence,
            progress: fixture.progress,
        }
    }

    fn observe_prearm_outcome_action(
        expected_observations: Vec<RecoveryExpectedObservation>,
    ) -> Result<RecoveryAction, RecoveryContractError> {
        let (expected_observations, expected_postcondition_digest) =
            expected_postcondition(expected_observations)?;
        runtime_action(
            RecoveryActionDigestRecordKind::ObservePreArmCancellationOutcome(
                ObservePreArmCancellationOutcomeActionDigestRecord {
                    action_kind: ObservePreArmCancellationOutcomeActionKind::Value,
                    action_id: id("44444444-4444-4444-8444-444444444444"),
                    prior_operation_id: OperationId::parse("55555555-5555-4555-8555-555555555555")
                        .unwrap(),
                    support_action_id: id("22222222-2222-4222-8222-222222222222"),
                    expected_support_action_digest: digest(A),
                    approved_cancellation_digest: digest(B),
                    expected_observations,
                    expected_postcondition_digest,
                },
            ),
        )
    }

    fn finish_prearm_action(
        expected_observations: Vec<RecoveryExpectedObservation>,
    ) -> Result<RecoveryAction, RecoveryContractError> {
        let (expected_observations, expected_postcondition_digest) =
            expected_postcondition(expected_observations)?;
        runtime_action(
            RecoveryActionDigestRecordKind::FinishPreArmCancellationRecovery(
                FinishPreArmCancellationRecoveryActionDigestRecord {
                    action_kind: FinishPreArmCancellationRecoveryActionKind::Value,
                    action_id: id("66666666-6666-4666-8666-666666666666"),
                    finalization_attempt_id: id("77777777-7777-4777-8777-777777777777"),
                    support_action_id: id("22222222-2222-4222-8222-222222222222"),
                    expected_support_action_digest: digest(A),
                    approved_cancellation_digest: digest(B),
                    effect_observation_digest: digest(A),
                    finalization_plan_digest: digest(B),
                    receipt_plan_digest: digest(A),
                    expected_result_phase: TaskPhase::RecoveryRequired,
                    receipt_ref: PreArmCancellationReceiptRef::finalization_plan(
                        id("88888888-8888-4888-8888-888888888888"),
                        PreArmCancellationEffectKind::RecoveryFinalization,
                        digest(B),
                    ),
                    expected_observations,
                    expected_postcondition_digest,
                },
            ),
        )
    }

    fn history_cursor(version: &str, prefix_digest: &str) -> RepositoryHistoryCursor {
        serde_json::from_value(json!({
            "throughVersion": version,
            "historyPrefixDigest": prefix_digest,
        }))
        .unwrap()
    }

    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct EmptyPartitionDigestRecord {
        from_exclusive: RepositoryHistoryCursor,
        through_inclusive: RepositoryHistoryCursor,
        entries: Vec<Value>,
    }

    impl contract_digest_record_sealed::Sealed for EmptyPartitionDigestRecord {}
    impl ContractDigestRecord for EmptyPartitionDigestRecord {}

    struct UnexpectedEvidenceIndex;

    impl EvidenceSourceIndex for UnexpectedEvidenceIndex {
        fn candidate_for(
            &self,
            _repository_version: &RepositoryVersion,
            _registry: &EvidenceSourceRegistry,
        ) -> Result<EvidenceSourceIndexCandidate, RepositoryContractError> {
            panic!("empty history partition must not consult the evidence index")
        }
    }

    struct UnexpectedHistoryOrder;

    impl RepositoryHistoryOrderResolver for UnexpectedHistoryOrder {
        fn order_evidence(
            &self,
            _from_exclusive: &RepositoryHistoryCursor,
            _through_inclusive: &RepositoryHistoryCursor,
        ) -> Result<RepositoryHistoryOrderEvidence, RepositoryContractError> {
            panic!("empty history partition must not consult repository history")
        }
    }

    struct UnexpectedEvidenceBytes;

    impl RepositoryHistoryEvidenceBytesResolver for UnexpectedEvidenceBytes {
        fn load_canonical_evidence_bytes(
            &self,
            _reference: &RepositoryHistorySourceEvidenceRef,
        ) -> Result<Vec<u8>, RepositoryContractError> {
            panic!("empty history partition must not load evidence bytes")
        }
    }

    fn empty_history_partition(
        endpoint: &RepositoryHistoryCursor,
    ) -> ValidatedRepositoryHistoryPartition {
        let partition_digest = contract_digest(
            &EmptyPartitionDigestRecord {
                from_exclusive: endpoint.clone(),
                through_inclusive: endpoint.clone(),
                entries: Vec::new(),
            },
            "empty test partition digest failed",
        )
        .unwrap();
        let wire = serde_json::from_value::<UnvalidatedRepositoryHistoryPartition>(json!({
            "fromExclusive": endpoint,
            "throughInclusive": endpoint,
            "entries": [],
            "partitionDigest": partition_digest,
        }))
        .unwrap();
        let registry = EvidenceSourceRegistry::task8().unwrap();
        RepositoryHistoryPartitionResolver::new(
            &registry,
            &UnexpectedEvidenceIndex,
            &UnexpectedHistoryOrder,
            &UnexpectedEvidenceBytes,
        )
        .validate(wire)
        .unwrap()
    }

    fn support_finalization_plan(
        endpoint: RepositoryHistoryCursor,
    ) -> SupportRecoveryFinalizationPlan {
        let display = RepositoryTargetDisplay::parse("Configuration").unwrap();
        let lock_targets = support_lock_targets();
        let desired_targets =
            SupportRecoveryDesiredTargets::new(vec![SupportRecoveryDesiredTarget::root_present(
                display,
                digest(B),
            )])
            .unwrap();
        SupportRecoveryFinalizationPlan::new(
            SupportRecoveryFinalizationPlanAuthority::desired_test_only(
                SupportRecoveryDisposition::RestoreThenReauthorize,
                lock_targets,
                desired_targets,
                endpoint,
                digest(A),
                digest(B),
            ),
        )
        .unwrap()
    }

    fn support_lock_targets() -> SupportRecoveryLockTargets {
        SupportRecoveryLockTargets::new(vec![SupportRecoveryLockTarget::configuration_root(
            RepositoryTargetDisplay::parse("Configuration").unwrap(),
            vec![
                RepositoryUpdateLockReason::SupportGraphGuard,
                RepositoryUpdateLockReason::UpdateTarget,
            ],
        )
        .unwrap()])
        .unwrap()
    }

    struct TrivialSupportHistoryOrder;

    impl SupportHistoryOrderAuthority for TrivialSupportHistoryOrder {
        fn compare_versions(
            &self,
            _left: &RepositoryVersion,
            _right: &RepositoryVersion,
        ) -> Result<Ordering, SupportContractError> {
            Ok(Ordering::Equal)
        }

        fn compare_cursors(
            &self,
            _left: &RepositoryHistoryCursor,
            _right: &RepositoryHistoryCursor,
        ) -> Result<Ordering, SupportContractError> {
            Ok(Ordering::Equal)
        }
    }

    fn corrective_external_action(
        mode: ManualSupportTargetMode,
        cursor: RepositoryHistoryCursor,
    ) -> SupportRecoveryExternalAction {
        let transition =
            SupportRecoveryTransition::ordinary(SupportTransition::enable_configuration_changes(
                RepositoryTargetDisplay::parse("Configuration").unwrap(),
                SupportLayerId::parse("layer-a").unwrap(),
            ));
        let authority = SupportCorrectiveInstructionAuthority::test_only(
            id("22222222-2222-4222-8222-222222222222"),
            SupportActionPurpose::MainIntegrationPrerequisite,
            mode,
            RepositoryUsername::parse("support-user").unwrap(),
            (mode == ManualSupportTargetMode::SeparateWorkingInfobase).then(working_identity),
            cursor,
            support_lock_targets(),
            support_lock_targets(),
            vec![transition],
            Vec::new(),
            Vec::new(),
            Vec::new(),
            digest(A),
            digest(B),
        )
        .unwrap();
        SupportRecoveryExternalAction::corrective(
            SupportCorrectiveInstruction::new(authority).unwrap(),
        )
    }

    fn conflict_external_action() -> SupportRecoveryExternalAction {
        let transition = SupportTransition::enable_configuration_changes(
            RepositoryTargetDisplay::parse("Configuration").unwrap(),
            SupportLayerId::parse("layer-a").unwrap(),
        );
        let conflict = SupportTransitionConflict::from_capability_adapter(
            RepositoryVersion::parse("v2").unwrap(),
            RequiredNullable::null(),
            None,
            RepositoryTargetDisplay::parse("Configuration").unwrap(),
            SupportLayerId::parse("layer-a").unwrap(),
            transition,
            digest(A),
            SupportTransitionOverlapKind::SameTarget,
            Diagnostic::parse("redacted").unwrap(),
        )
        .unwrap();
        let conflicts =
            SupportTransitionConflicts::new(vec![conflict], &TrivialSupportHistoryOrder).unwrap();
        SupportRecoveryExternalAction::conflict(
            SupportConflictInstruction::new(
                id("55555555-5555-4555-8555-555555555555"),
                conflicts,
                digest(B),
            )
            .unwrap(),
        )
    }

    fn evidence_external_action() -> SupportRecoveryExternalAction {
        SupportRecoveryExternalAction::evidence(
            SupportEvidenceInstruction::new(
                SupportBlockers::new(Vec::new()).unwrap(),
                SupportEvidenceGaps::new(Vec::new(), &TrivialSupportHistoryOrder).unwrap(),
            )
            .unwrap(),
        )
    }

    #[derive(Clone, Copy)]
    enum ArmedSupportBlockerFixture {
        Absent,
        Corrective,
        ReleaseLocks,
        CleanWorkingInfobase,
        CloseReservedOriginal,
        Conflict,
        Evidence,
    }

    fn armed_support_closure_plan(
        endpoint: &RepositoryHistoryCursor,
    ) -> ManualWorkingInfobaseClosurePlan {
        ManualWorkingInfobaseClosurePlan::new(
            ManualWorkingInfobaseClosurePlanAuthority::materialized_test_only(
                working_identity(),
                digest(A),
                digest(B),
                digest(A),
                digest(B),
                endpoint.clone(),
                digest(A),
                CapabilityRowId::parse("manual-working-infobase-lease.v1").unwrap(),
            ),
        )
        .unwrap()
    }

    fn blocked_guard_proof_fixture(mode: ManualSupportTargetMode, status: &Value) -> Value {
        json!({
            "outcome": "blockedBeforeRoot",
            "guardReceiptId": ID_1,
            "manualTargetMode": match mode {
                ManualSupportTargetMode::ReservedOriginal => "reservedOriginal",
                ManualSupportTargetMode::SeparateWorkingInfobase => "separateWorkingInfobase",
            },
            "finalizationPlanDigest": status["supportRecoveryFinalizationPlan"]["planDigest"],
            "plannedLockTargets": status["supportRecoveryFinalizationPlan"]["lockTargets"],
            "acquiredInOrder": [],
            "failedTarget": { "targetKind": "configurationRoot" },
            "failedTargetDisplay": "Configuration",
            "lockedBy": null,
            "authorizationOutcome": "unchanged",
            "releasedInReverseOrder": [],
            "releaseVerified": true,
            "proofDigest": B,
        })
    }

    fn stopped_guard_proof_fixture(mode: ManualSupportTargetMode, status: &Value) -> Value {
        let mut proof = json!({
            "outcome": "stoppedAfterCompleteGuard",
            "guardReceiptId": ID_1,
            "guardReleaseReceiptId": ID_2,
            "manualTargetMode": match mode {
                ManualSupportTargetMode::ReservedOriginal => "reservedOriginal",
                ManualSupportTargetMode::SeparateWorkingInfobase => "separateWorkingInfobase",
            },
            "finalizationPlanDigest": status["supportRecoveryFinalizationPlan"]["planDigest"],
            "plannedLockTargets": status["supportRecoveryFinalizationPlan"]["lockTargets"],
            "acquiredInOrder": status["supportRecoveryFinalizationPlan"]["lockTargets"],
            "historyFromCursor": status["supportHistoryFromCursor"],
            "historyThroughCursor": status["supportHistoryThroughCursor"],
            "historyPartitionDigest": status["supportHistoryPartition"]["partitionDigest"],
            "supportGraphRecheckedUnderGuard": true,
            "correctiveBeforeStateBindingVerified": true,
            "contentRecheckedUnderGuard": true,
            "originalRecheckedUnderGuard": true,
            "selectiveUpdatePerformed": false,
            "authorizationOutcome": "unchanged",
            "releasedInReverseOrder": status["supportRecoveryFinalizationPlan"]["lockTargets"],
            "releaseVerified": true,
            "proofDigest": B,
        });
        match mode {
            ManualSupportTargetMode::ReservedOriginal => {
                proof["manualActorLockInventoryProof"] = serde_json::to_value(
                    ManualActorLockInventoryProof::new(
                        RepositoryUsername::parse("support-user").unwrap(),
                        digest(A),
                        digest(A),
                    )
                    .unwrap(),
                )
                .unwrap();
                proof["reservedOriginalLeaseStopEvidence"] = serde_json::to_value(
                    ReservedOriginalLeaseStopEvidence::new(
                        digest(A),
                        CapabilityRowId::parse("reserved-original-lease.v1").unwrap(),
                        RequiredNullable::null(),
                    )
                    .unwrap(),
                )
                .unwrap();
            }
            ManualSupportTargetMode::SeparateWorkingInfobase => {
                let plan = armed_support_closure_plan(&history_cursor("v1", A));
                proof["manualWorkingInfobaseStopEvidence"] = serde_json::to_value(
                    ManualWorkingInfobaseStopEvidence::new(
                        &plan,
                        ManualWorkingInfobaseStopAuthority::lease_busy_test_only(
                            &plan,
                            RequiredNullable::null(),
                        )
                        .unwrap(),
                    )
                    .unwrap(),
                )
                .unwrap();
            }
        }
        proof
    }

    fn armed_support_status_fixture(
        mode: ManualSupportTargetMode,
        blocker: ArmedSupportBlockerFixture,
    ) -> Value {
        let mut status = serde_json::to_value(
            armed_support_projection_fixture(mode, blocker).into_recovery_plan_status(),
        )
        .unwrap();
        let proof = match blocker {
            ArmedSupportBlockerFixture::ReleaseLocks => {
                Some(blocked_guard_proof_fixture(mode, &status))
            }
            ArmedSupportBlockerFixture::CleanWorkingInfobase
            | ArmedSupportBlockerFixture::CloseReservedOriginal => {
                Some(stopped_guard_proof_fixture(mode, &status))
            }
            ArmedSupportBlockerFixture::Absent
            | ArmedSupportBlockerFixture::Corrective
            | ArmedSupportBlockerFixture::Conflict
            | ArmedSupportBlockerFixture::Evidence => None,
        };
        if let Some(proof) = proof {
            status["latestSupportRecoveryGuardProof"] = proof;
        }
        status
    }

    fn armed_support_projection_fixture(
        mode: ManualSupportTargetMode,
        blocker: ArmedSupportBlockerFixture,
    ) -> ArmedSupportRecoveryPlanProjection {
        let endpoint = history_cursor("v1", A);
        let closure_plan = (mode == ManualSupportTargetMode::SeparateWorkingInfobase)
            .then(|| armed_support_closure_plan(&endpoint));
        let required_external_action = match blocker {
            ArmedSupportBlockerFixture::Absent => None,
            ArmedSupportBlockerFixture::Corrective => {
                Some(corrective_external_action(mode, endpoint.clone()))
            }
            ArmedSupportBlockerFixture::ReleaseLocks => {
                Some(SupportRecoveryExternalAction::release_locks(
                    ReleaseRepositoryLocksInstruction::new(
                        RequiredNullable::null(),
                        vec![RepositoryTargetDisplay::parse("Configuration").unwrap()],
                    )
                    .unwrap(),
                ))
            }
            ArmedSupportBlockerFixture::CleanWorkingInfobase => {
                let plan = closure_plan
                    .as_ref()
                    .expect("clean fixture requires separate-working-infobase mode");
                Some(SupportRecoveryExternalAction::clean_working_infobase(
                    CleanManualWorkingInfobaseInstruction::new(
                        plan.working_infobase_identity().clone(),
                        plan.plan_digest().clone(),
                        plan.exclusive_lease_capability_id().clone(),
                        plan.desired_base_fingerprint().clone(),
                        ManualWorkingInfobaseCleanupReason::LeaseBusy,
                    ),
                ))
            }
            ArmedSupportBlockerFixture::CloseReservedOriginal => {
                Some(SupportRecoveryExternalAction::close_reserved_original(
                    CloseReservedOriginalDesignerInstruction::new(
                        digest(A),
                        CapabilityRowId::parse("reserved-original-lease.v1").unwrap(),
                    ),
                ))
            }
            ArmedSupportBlockerFixture::Conflict => Some(conflict_external_action()),
            ArmedSupportBlockerFixture::Evidence => Some(evidence_external_action()),
        };
        ArmedSupportRecoveryPlanProjection::test_only(
            OperationId::parse("44444444-4444-4444-8444-444444444444").unwrap(),
            id("22222222-2222-4222-8222-222222222222"),
            TaskPhase::LocalVerified,
            endpoint.clone(),
            endpoint.clone(),
            empty_history_partition(&endpoint),
            SupportRecoveryVersionObservations::new(Vec::new()).unwrap(),
            SupportRecoveryDisposition::RestoreThenReauthorize,
            TaskPhase::LocalVerified,
            support_finalization_plan(endpoint),
            None,
            closure_plan,
            mode,
            required_external_action,
        )
        .unwrap()
    }

    fn observe_history_action(
        observations: Vec<RecoveryExpectedObservation>,
    ) -> Result<RecoveryAction, RecoveryContractError> {
        let expected_observations = RecoveryExpectedObservations(observations);
        let expected_postcondition_digest = contract_digest(
            &RecoveryExpectedPostconditionDigestRecord(expected_observations.clone()),
            "test recovery expected postcondition digest failed",
        )?;
        runtime_action(
            RecoveryActionDigestRecordKind::ObserveSupportPrerequisiteHistory(
                ObserveSupportPrerequisiteHistoryActionDigestRecord {
                    action_kind: ObserveSupportPrerequisiteHistoryActionKind::Value,
                    action_id: id("99999999-9999-4999-8999-999999999999"),
                    support_action_id: id("22222222-2222-4222-8222-222222222222"),
                    from_cursor: history_cursor("v1", A),
                    through_cursor: history_cursor("v2", B),
                    expected_partition_digest: digest(A),
                    expected_observations,
                    expected_postcondition_digest,
                },
            ),
        )
    }

    fn valid_recheck_observations(
        mode_observation: RecoveryExpectedObservation,
    ) -> Vec<RecoveryExpectedObservation> {
        vec![
            RecoveryExpectedObservation::new(
                RecoveryObservationKind::SupportGraph,
                RecoverySubjectRef::configuration_root(),
                digest(A),
            ),
            RecoveryExpectedObservation::new(
                RecoveryObservationKind::SupportActionAuthorization,
                RecoverySubjectRef::registered(id("22222222-2222-4222-8222-222222222222")),
                digest(A),
            ),
            RecoveryExpectedObservation::new(
                RecoveryObservationKind::ObjectFingerprint,
                RecoverySubjectRef::registered(id("33333333-3333-4333-8333-333333333333")),
                digest(A),
            ),
            RecoveryExpectedObservation::new(
                RecoveryObservationKind::LockOwnership,
                RecoverySubjectRef::configuration_root(),
                digest(A),
            ),
            mode_observation,
            RecoveryExpectedObservation::new(
                RecoveryObservationKind::FinalizationPolicy,
                RecoverySubjectRef::registered(id("44444444-4444-4444-8444-444444444444")),
                digest(B),
            ),
        ]
    }

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

    assert_not_deserialize_owned!(RecoveryObservation);
    assert_not_deserialize_owned!(RecoveryObservationDigestRecord);
    assert_not_deserialize_owned!(RecoveryUnknown);
    assert_not_deserialize_owned!(RecoveryAction);
    assert_not_deserialize_owned!(RecoveryActionDigestRecord);
    assert_not_deserialize_owned!(EffectReceipt);
    assert_not_deserialize_owned!(RecoveryActionOutcome);
    assert_not_deserialize_owned!(RecoveryActionOutcomeDigestRecord);
    assert_not_deserialize_owned!(ArchiveStagingReceipt);
    assert_not_deserialize_owned!(HandoffRetentionReleaseReceipt);
    assert_not_deserialize_owned!(HandoffRetentionReleaseSetDigestRecord);
    assert_not_deserialize_owned!(RecoveryActionPlan);
    assert_not_deserialize_owned!(FinishCleanupAbsenceObservation);
    assert_not_deserialize_owned!(FinishCleanupAbsenceObservations);
    assert_not_deserialize_owned!(StagedArchiveSha256);
    assert_not_deserialize_owned!(PublishedArchiveSha256);
    assert_not_deserialize_owned!(ImmutableArchiveByteGeneration);
    assert_not_deserialize_owned!(ArchiveStagingWriterLineage);
    assert_not_deserialize_owned!(ArchiveStagingObservation);
    assert_not_deserialize_owned!(ArchivePublicationParsedDigests);
    assert_not_deserialize_owned!(ArchivePublicationByteObservation);
    assert_not_deserialize_owned!(ArchiveStagingReceiptAuthority);
    assert_not_deserialize_owned!(ValidatedPreArmArchiveReceiptOutcomeWitness);

    #[test]
    fn prearm_archive_outcome_witness_accepts_exact_empty_and_perform_catalogs() {
        for include_selective_update in [false, true] {
            let fixture = exact_prearm_outcome_fixture(include_selective_update);
            let witness = ValidatedPreArmArchiveReceiptOutcomeWitness::from_completed_outcomes(
                &fixture.observation,
                &fixture.plan,
                &fixture.progress,
                &fixture.actions,
                &fixture.outcomes,
            )
            .unwrap();
            assert!(witness.binds_archive_lineage(
                &fixture.observation,
                &fixture.plan,
                &fixture.progress,
            ));
        }
    }

    #[test]
    fn prearm_archive_outcome_witness_rejects_order_class_and_legacy_producers() {
        let fixture = exact_prearm_outcome_fixture(false);

        let mut reordered = fixture.outcomes.clone();
        reordered.swap(0, 1);
        assert!(
            ValidatedPreArmArchiveReceiptOutcomeWitness::from_completed_outcomes(
                &fixture.observation,
                &fixture.plan,
                &fixture.progress,
                &fixture.actions,
                &reordered,
            )
            .is_err()
        );

        let mut missing = fixture.outcomes.clone();
        missing.pop();
        assert!(
            ValidatedPreArmArchiveReceiptOutcomeWitness::from_completed_outcomes(
                &fixture.observation,
                &fixture.plan,
                &fixture.progress,
                &fixture.actions,
                &missing,
            )
            .is_err()
        );

        let mut wrong_recheck_class = fixture.outcomes.clone();
        let recheck_id = fixture.actions[2].action_id().clone();
        let recheck_digest = fixture.actions[2].action_digest().clone();
        let RecoveryActionOutcomeKind::Performed(mut performed) = fixture.outcomes[0].0.clone()
        else {
            panic!("fixture root outcome must be performed");
        };
        performed.action_id = recheck_id;
        performed.action_digest = recheck_digest;
        wrong_recheck_class[2] =
            RecoveryActionOutcome(RecoveryActionOutcomeKind::Performed(performed));
        assert!(
            ValidatedPreArmArchiveReceiptOutcomeWitness::from_completed_outcomes(
                &fixture.observation,
                &fixture.plan,
                &fixture.progress,
                &fixture.actions,
                &wrong_recheck_class,
            )
            .is_err()
        );

        let legacy_receipts = fixture
            .progress
            .completed_realized_receipts()
            .unwrap()
            .iter()
            .map(|receipt| {
                PreArmCancellationEffectReceipt::new(
                    PreArmCancellationEffectReceiptAuthority::test_only(
                        receipt.receipt_id().clone(),
                        receipt.effect_kind(),
                        receipt.effect_intent_digest().clone(),
                        id("99999999-9999-4999-8999-999999999999"),
                        digest(B),
                        vec![digest(A)],
                    )
                    .unwrap(),
                )
                .unwrap()
            })
            .collect();
        let legacy_progress = PreArmCancellationFinalizationAttemptProgress::completed_test_only(
            fixture.plan.finalization_attempt_id().clone(),
            legacy_receipts,
            fixture
                .progress
                .completed_recheck_evidence()
                .unwrap()
                .clone(),
        )
        .unwrap();
        assert!(
            ValidatedPreArmArchiveReceiptOutcomeWitness::from_completed_outcomes(
                &fixture.observation,
                &fixture.plan,
                &legacy_progress,
                &fixture.actions,
                &fixture.outcomes,
            )
            .is_err()
        );
    }

    #[test]
    fn prearm_exact_action_grammar_rejects_kind_ref_payload_and_mode_splices() {
        let fixture = exact_prearm_outcome_fixture(false);
        let rejects = |actions: &[RecoveryAction]| {
            validate_prearm_finalize_action_grammar(
                &fixture.observation,
                &fixture.plan,
                &fixture.progress,
                actions,
            )
            .is_err()
        };

        // Same action IDs and length, but an optional update kind substituted
        // for the required cancellation persistence kind.
        let mut wrong_kind_mask = fixture.actions.clone();
        let (apply_observations, apply_postcondition) =
            expected_postcondition(vec![RecoveryExpectedObservation::new(
                RecoveryObservationKind::ObjectFingerprint,
                RecoverySubjectRef::configuration_root(),
                digest(A),
            )])
            .unwrap();
        wrong_kind_mask[3] = runtime_action(
            RecoveryActionDigestRecordKind::ApplyPreArmCancellationSelectiveUpdate(
                ApplyPreArmCancellationSelectiveUpdateActionDigestRecord {
                    action_kind: ApplyPreArmCancellationSelectiveUpdateActionKind::Value,
                    action_id: fixture.actions[3].action_id().clone(),
                    finalization_attempt_id: fixture.plan.finalization_attempt_id().clone(),
                    finalization_plan_digest: fixture.plan.finalization_plan_digest().clone(),
                    selective_update_plan_digest: fixture
                        .plan
                        .selective_update_plan_digest()
                        .clone(),
                    expected_target_revision_map_digest: fixture
                        .plan
                        .expected_target_revision_map_digest()
                        .clone(),
                    receipt_ref: PreArmCancellationReceiptRef::finalization_plan(
                        id("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"),
                        PreArmCancellationEffectKind::SelectiveOriginalUpdate,
                        digest(A),
                    ),
                    expected_observations: apply_observations,
                    expected_postcondition_digest: apply_postcondition,
                },
            ),
        )
        .unwrap();
        assert!(rejects(&wrong_kind_mask));

        let mut foreign_same_kind_ref = fixture.actions.clone();
        let RecoveryActionKindWire::AcquirePreArmRootGuard(root) = &mut foreign_same_kind_ref[0].0
        else {
            panic!("fixture must acquire root first");
        };
        root.receipt_ref = PreArmCancellationReceiptRef::finalization_plan(
            id("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"),
            PreArmCancellationEffectKind::RootGuardAcquire,
            root.receipt_ref.effect_intent_digest().clone(),
        );
        assert!(rejects(&foreign_same_kind_ref));

        let mut foreign_support = fixture.actions.clone();
        let RecoveryActionKindWire::AcquirePreArmRootGuard(root) = &mut foreign_support[0].0 else {
            panic!("fixture must acquire root first");
        };
        root.support_action_id = id(ID_2);
        assert!(rejects(&foreign_support));

        let mut foreign_cancellation = fixture.actions.clone();
        let RecoveryActionKindWire::PersistPreArmSupportCancellation(persist) =
            &mut foreign_cancellation[3].0
        else {
            panic!("empty-plan fixture must persist at index 3");
        };
        persist.approved_cancellation_digest = digest(A);
        assert!(rejects(&foreign_cancellation));

        let mut foreign_observation = fixture.actions.clone();
        let RecoveryActionKindWire::PersistPreArmSupportCancellation(persist) =
            &mut foreign_observation[3].0
        else {
            panic!("empty-plan fixture must persist at index 3");
        };
        persist.effect_observation_digest = digest(A);
        assert!(rejects(&foreign_observation));

        let mut foreign_support_digest = fixture.actions.clone();
        let RecoveryActionKindWire::PersistPreArmSupportCancellation(persist) =
            &mut foreign_support_digest[3].0
        else {
            panic!("empty-plan fixture must persist at index 3");
        };
        persist.expected_support_action_digest = digest(B);
        assert!(rejects(&foreign_support_digest));

        let mut substituted_postcondition = fixture.actions.clone();
        let RecoveryActionKindWire::AcquirePreArmRootGuard(root) =
            &mut substituted_postcondition[0].0
        else {
            panic!("fixture must acquire root first");
        };
        let (observations, postcondition) =
            expected_postcondition(vec![RecoveryExpectedObservation::new(
                RecoveryObservationKind::LockOwnership,
                RecoverySubjectRef::configuration_root(),
                digest(B),
            )])
            .unwrap();
        root.expected_observations = observations;
        root.expected_postcondition_digest = postcondition;
        assert!(rejects(&substituted_postcondition));

        for mutate_capability in [false, true] {
            let mut foreign_mode_window = fixture.actions.clone();
            let release = foreign_mode_window
                .iter_mut()
                .find_map(|action| match &mut action.0 {
                    RecoveryActionKindWire::ReleasePreArmModeLease(value) => Some(value),
                    _ => None,
                })
                .unwrap();
            let ReleasePreArmModeLeaseActionKindWire::ReservedOriginal(release) = &mut release.0
            else {
                panic!("fixture must use reserved-original mode");
            };
            if mutate_capability {
                release.exclusive_lease_capability_id =
                    CapabilityRowId::parse("foreign-reserved-lease.v1").unwrap();
            } else {
                release.reserved_original_identity_digest = digest(B);
            }
            assert!(rejects(&foreign_mode_window));
        }

        let mut foreign_receipt_plan = fixture.actions.clone();
        let RecoveryActionKindWire::FinishPreArmCancellationRecovery(finish) =
            &mut foreign_receipt_plan.last_mut().unwrap().0
        else {
            panic!("fixture must finish last");
        };
        finish.receipt_plan_digest = digest(A);
        assert!(rejects(&foreign_receipt_plan));

        let mut foreign_phase = fixture.actions.clone();
        let RecoveryActionKindWire::FinishPreArmCancellationRecovery(finish) =
            &mut foreign_phase.last_mut().unwrap().0
        else {
            panic!("fixture must finish last");
        };
        finish.expected_result_phase = TaskPhase::RecoveryRequired;
        assert!(rejects(&foreign_phase));
    }

    #[test]
    fn prearm_perform_action_rejects_foreign_selective_plan_and_revision_map() {
        let fixture = exact_prearm_outcome_fixture(true);
        let rejects = |actions: &[RecoveryAction]| {
            validate_prearm_finalize_action_grammar(
                &fixture.observation,
                &fixture.plan,
                &fixture.progress,
                actions,
            )
            .is_err()
        };

        let mut foreign_plan = fixture.actions.clone();
        let RecoveryActionKindWire::ApplyPreArmCancellationSelectiveUpdate(apply) =
            &mut foreign_plan[3].0
        else {
            panic!("perform fixture must apply at index 3");
        };
        apply.selective_update_plan_digest = digest(A);
        assert!(rejects(&foreign_plan));

        let mut foreign_map = fixture.actions.clone();
        let RecoveryActionKindWire::ApplyPreArmCancellationSelectiveUpdate(apply) =
            &mut foreign_map[3].0
        else {
            panic!("perform fixture must apply at index 3");
        };
        apply.expected_target_revision_map_digest = digest(B);
        assert!(rejects(&foreign_map));
    }

    #[test]
    fn recovery_subject_contract_exists() {
        let _ = std::mem::size_of::<RecoverySubjectRef>();
    }

    #[test]
    fn recovery_plan_status_is_a_closed_physical_target_effect_stage_union() {
        let contract = schema::<RecoveryPlanStatus>();
        audit_json_schema(&contract).unwrap();
        assert_eq!(contract["oneOf"].as_array().map(Vec::len), Some(13));
        let encoded = serde_json::to_string(&contract).unwrap();
        for literal in [
            "taskConfiguration",
            "repositoryLocks",
            "originalConfiguration",
            "repositoryCommit",
            "supportPrerequisite",
            "preArmSupportCancellation",
            "manualWorkingInfobaseLease",
            "artifact",
            "archive",
            "cleanup",
            "observeOutcome",
            "committed",
            "notCommitted",
            "finalize",
        ] {
            assert!(encoded.contains(literal), "missing plan branch {literal}");
        }
        for collection in [
            schema::<RecoveryPlanObservations>(),
            schema::<RecoveryPlanUnknowns>(),
            schema::<SupportRecoveryVersionObservations>(),
            schema::<RecoveryExpectedObservations>(),
        ] {
            assert_eq!(collection["maxItems"], 1024);
        }
        assert_eq!(
            schema::<SupportRecoveryHistoryEvidence>()["oneOf"]
                .as_array()
                .map(Vec::len),
            Some(2),
        );
    }

    #[test]
    fn armed_support_schema_binds_mode_external_blocker_and_exact_wait_tuple() {
        let reserved_without_blocker = armed_support_status_fixture(
            ManualSupportTargetMode::ReservedOriginal,
            ArmedSupportBlockerFixture::Absent,
        );
        let separate_without_blocker = armed_support_status_fixture(
            ManualSupportTargetMode::SeparateWorkingInfobase,
            ArmedSupportBlockerFixture::Absent,
        );
        let reserved_blockers = [
            ArmedSupportBlockerFixture::Corrective,
            ArmedSupportBlockerFixture::ReleaseLocks,
            ArmedSupportBlockerFixture::CloseReservedOriginal,
            ArmedSupportBlockerFixture::Conflict,
            ArmedSupportBlockerFixture::Evidence,
        ]
        .map(|blocker| {
            armed_support_status_fixture(ManualSupportTargetMode::ReservedOriginal, blocker)
        });
        let separate_blockers = [
            ArmedSupportBlockerFixture::Corrective,
            ArmedSupportBlockerFixture::ReleaseLocks,
            ArmedSupportBlockerFixture::CleanWorkingInfobase,
            ArmedSupportBlockerFixture::Conflict,
            ArmedSupportBlockerFixture::Evidence,
        ]
        .map(|blocker| {
            armed_support_status_fixture(ManualSupportTargetMode::SeparateWorkingInfobase, blocker)
        });

        for valid in std::iter::once(&reserved_without_blocker)
            .chain(reserved_blockers.iter())
            .chain(std::iter::once(&separate_without_blocker))
            .chain(separate_blockers.iter())
        {
            assert!(schema_accepts::<ArmedSupportRecoveryPlanStatus>(valid));
            assert!(schema_accepts::<RecoveryPlanStatus>(valid));
        }

        for statuses in [&reserved_blockers, &separate_blockers] {
            for index in [1, 2] {
                let mut missing_proof = statuses[index].clone();
                missing_proof
                    .as_object_mut()
                    .unwrap()
                    .remove("latestSupportRecoveryGuardProof");
                assert!(
                    !schema_accepts::<ArmedSupportRecoveryPlanStatus>(&missing_proof),
                    "guard-bound blocker {index} accepted a missing latest proof",
                );
            }
            for index in [0, 3, 4] {
                let mut extra_proof = statuses[index].clone();
                extra_proof["latestSupportRecoveryGuardProof"] =
                    statuses[1]["latestSupportRecoveryGuardProof"].clone();
                assert!(
                    !schema_accepts::<ArmedSupportRecoveryPlanStatus>(&extra_proof),
                    "pre-guard blocker {index} accepted a latest proof",
                );
            }

            let mut lock_with_stopped = statuses[1].clone();
            lock_with_stopped["latestSupportRecoveryGuardProof"] =
                statuses[2]["latestSupportRecoveryGuardProof"].clone();
            assert!(!schema_accepts::<ArmedSupportRecoveryPlanStatus>(
                &lock_with_stopped,
            ));

            let mut closure_with_blocked = statuses[2].clone();
            closure_with_blocked["latestSupportRecoveryGuardProof"] =
                statuses[1]["latestSupportRecoveryGuardProof"].clone();
            assert!(!schema_accepts::<ArmedSupportRecoveryPlanStatus>(
                &closure_with_blocked,
            ));

            let mut completed_outcome = statuses[1].clone();
            completed_outcome["latestSupportRecoveryGuardProof"]["outcome"] = json!("completed");
            assert!(!schema_accepts::<ArmedSupportRecoveryPlanStatus>(
                &completed_outcome,
            ));
        }

        let mut no_blocker_with_proof = reserved_without_blocker.clone();
        no_blocker_with_proof["latestSupportRecoveryGuardProof"] =
            reserved_blockers[1]["latestSupportRecoveryGuardProof"].clone();
        assert!(!schema_accepts::<ArmedSupportRecoveryPlanStatus>(
            &no_blocker_with_proof,
        ));

        for index in [1, 2] {
            let mut reserved_with_separate_proof = reserved_blockers[index].clone();
            reserved_with_separate_proof["latestSupportRecoveryGuardProof"] =
                separate_blockers[index]["latestSupportRecoveryGuardProof"].clone();
            assert!(!schema_accepts::<ArmedSupportRecoveryPlanStatus>(
                &reserved_with_separate_proof,
            ));

            let mut separate_with_reserved_proof = separate_blockers[index].clone();
            separate_with_reserved_proof["latestSupportRecoveryGuardProof"] =
                reserved_blockers[index]["latestSupportRecoveryGuardProof"].clone();
            assert!(!schema_accepts::<ArmedSupportRecoveryPlanStatus>(
                &separate_with_reserved_proof,
            ));
        }

        let mut action_without_wait = reserved_without_blocker.clone();
        action_without_wait["requiredExternalAction"] =
            reserved_blockers[2]["requiredExternalAction"].clone();
        assert!(!schema_accepts::<ArmedSupportRecoveryPlanStatus>(
            &action_without_wait
        ));

        let mut wait_without_action = reserved_without_blocker.clone();
        wait_without_action["actions"]
            .as_array_mut()
            .unwrap()
            .insert(1, reserved_blockers[2]["actions"][1].clone());
        assert!(!schema_accepts::<ArmedSupportRecoveryPlanStatus>(
            &wait_without_action
        ));

        for statuses in [&reserved_blockers, &separate_blockers] {
            for index in 0..statuses.len() {
                let other = (index + 1) % statuses.len();
                let mut wrong_wait = statuses[index].clone();
                wrong_wait["actions"][1] = statuses[other]["actions"][1].clone();
                assert!(
                    !schema_accepts::<ArmedSupportRecoveryPlanStatus>(&wrong_wait),
                    "external blocker {index} accepted wait {other}",
                );

                let mut wrong_instruction = statuses[index].clone();
                wrong_instruction["requiredExternalAction"] =
                    statuses[other]["requiredExternalAction"].clone();
                assert!(
                    !schema_accepts::<ArmedSupportRecoveryPlanStatus>(&wrong_instruction),
                    "wait {index} accepted external blocker {other}",
                );
            }
        }

        let mut reserved_with_working_infobase_blocker = reserved_without_blocker.clone();
        reserved_with_working_infobase_blocker["requiredExternalAction"] =
            separate_blockers[2]["requiredExternalAction"].clone();
        reserved_with_working_infobase_blocker["actions"] = separate_blockers[2]["actions"].clone();
        assert!(!schema_accepts::<ArmedSupportRecoveryPlanStatus>(
            &reserved_with_working_infobase_blocker
        ));

        let mut separate_with_reserved_original_blocker = separate_without_blocker.clone();
        separate_with_reserved_original_blocker["requiredExternalAction"] =
            reserved_blockers[2]["requiredExternalAction"].clone();
        separate_with_reserved_original_blocker["actions"] =
            reserved_blockers[2]["actions"].clone();
        assert!(!schema_accepts::<ArmedSupportRecoveryPlanStatus>(
            &separate_with_reserved_original_blocker
        ));

        let mut reserved_with_separate_corrective_payload = reserved_blockers[0].clone();
        reserved_with_separate_corrective_payload["requiredExternalAction"] =
            separate_blockers[0]["requiredExternalAction"].clone();
        assert!(!schema_accepts::<ArmedSupportRecoveryPlanStatus>(
            &reserved_with_separate_corrective_payload
        ));

        let mut separate_with_reserved_corrective_payload = separate_blockers[0].clone();
        separate_with_reserved_corrective_payload["requiredExternalAction"] =
            reserved_blockers[0]["requiredExternalAction"].clone();
        assert!(!schema_accepts::<ArmedSupportRecoveryPlanStatus>(
            &separate_with_reserved_corrective_payload
        ));

        let mut reserved_with_closure_plan = reserved_without_blocker.clone();
        reserved_with_closure_plan["manualWorkingInfobaseClosurePlan"] =
            separate_without_blocker["manualWorkingInfobaseClosurePlan"].clone();
        assert!(!schema_accepts::<ArmedSupportRecoveryPlanStatus>(
            &reserved_with_closure_plan
        ));

        let mut separate_without_closure_plan = separate_without_blocker.clone();
        separate_without_closure_plan
            .as_object_mut()
            .unwrap()
            .remove("manualWorkingInfobaseClosurePlan");
        assert!(!schema_accepts::<ArmedSupportRecoveryPlanStatus>(
            &separate_without_closure_plan
        ));

        assert_eq!(
            schema::<ArmedSupportRecoveryPlanStatus>()["oneOf"]
                .as_array()
                .map(Vec::len),
            Some(12),
        );
        audit_json_schema(&schema::<ArmedSupportRecoveryPlanStatus>()).unwrap();
    }

    #[test]
    fn armed_support_action_catalog_rejects_every_duplicate_id_position() {
        let first = id("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa");
        let second = id("bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb");
        let third = id("cccccccc-cccc-4ccc-8ccc-cccccccccccc");
        assert!(SupportRecoveryActionCatalogAuthority::test_only(
            first.clone(),
            None,
            first.clone(),
        )
        .is_err());

        let external_action = SupportRecoveryExternalAction::close_reserved_original(
            CloseReservedOriginalDesignerInstruction::new(
                digest(A),
                CapabilityRowId::parse("reserved-original-lease.v1").unwrap(),
            ),
        );
        let wait = SupportRecoveryExternalWaitAuthority::from_external_action_test_only(
            first.clone(),
            external_action,
            digest(B),
        )
        .unwrap();
        assert!(SupportRecoveryActionCatalogAuthority::test_only(
            first.clone(),
            Some(wait.clone()),
            second.clone(),
        )
        .is_err());
        assert!(SupportRecoveryActionCatalogAuthority::test_only(
            second.clone(),
            Some(wait),
            first.clone(),
        )
        .is_err());

        let distinct_wait = SupportRecoveryExternalWaitAuthority::from_external_action_test_only(
            second.clone(),
            SupportRecoveryExternalAction::close_reserved_original(
                CloseReservedOriginalDesignerInstruction::new(
                    digest(A),
                    CapabilityRowId::parse("reserved-original-lease.v1").unwrap(),
                ),
            ),
            digest(B),
        )
        .unwrap();
        assert!(SupportRecoveryActionCatalogAuthority::test_only(
            first,
            Some(distinct_wait),
            third,
        )
        .is_ok());
    }

    #[test]
    fn armed_support_projection_preserves_sealed_status_lineage_and_observation_digest() {
        let projection = armed_support_projection_fixture(
            ManualSupportTargetMode::ReservedOriginal,
            ArmedSupportBlockerFixture::Absent,
        );
        let expected_operation_id =
            OperationId::parse("44444444-4444-4444-8444-444444444444").unwrap();
        let expected_observation_digest = SupportRecoveryVersionObservations::new(Vec::new())
            .unwrap()
            .digest()
            .unwrap();

        assert_eq!(projection.prior_operation_id(), &expected_operation_id);
        assert_eq!(
            projection.support_version_observation_digest(),
            &expected_observation_digest
        );

        let cloned_status = projection.recovery_plan_status();
        let cloned_wire = serde_json::to_value(&cloned_status).unwrap();
        let owned_wire =
            serde_json::to_value(projection.clone().into_recovery_plan_status()).unwrap();
        assert_eq!(cloned_wire, owned_wire);
        assert_eq!(
            cloned_wire["priorOperationId"],
            expected_operation_id.as_str()
        );
        assert_eq!(
            cloned_wire["supportVersionObservationDigest"],
            expected_observation_digest.as_str()
        );

        let reprojected = cloned_status.armed_support_recovery_projection().unwrap();
        assert_eq!(reprojected.recovery_digest(), projection.recovery_digest());
        assert_eq!(
            reprojected.prior_operation_id(),
            projection.prior_operation_id()
        );
        assert_eq!(
            reprojected.support_version_observation_digest(),
            projection.support_version_observation_digest()
        );
    }

    #[test]
    fn dynamic_recovery_action_schemas_and_wrappers_freeze_the_structural_boundary() {
        let unrelated_actions = vec![
            verify_task_action(
                RecoveryObservationKind::TaskFingerprint,
                digest(A),
                digest(A),
            )
            .unwrap(),
            release_root_lock_action_two().unwrap(),
        ];
        let unrelated = serde_json::to_value(&unrelated_actions).unwrap();
        assert!(!schema_accepts::<SupportPrerequisiteRecoveryActions>(
            &unrelated
        ));
        assert!(!schema_accepts::<PreArmFinalizeRecoveryActions>(&unrelated));
        assert!(
            SupportPrerequisiteRecoveryActions::from_actions(unrelated_actions.clone()).is_err()
        );
        assert!(PreArmFinalizeRecoveryActions::from_actions(unrelated_actions).is_err());

        let plan = RecoveryPlanStatus::cleanup_fixture_test_only(
            OperationId::parse("22222222-2222-4222-8222-222222222222").unwrap(),
            id("33333333-3333-4333-8333-333333333333"),
            serde_json::from_value(json!({
                "projectId": ID_1,
                "instanceId": ID_1,
                "role": "quarantine"
            }))
            .unwrap(),
            TaskPhase::CleanedSuccess,
        )
        .unwrap();
        let RecoveryPlanStatusKind::Cleanup(cleanup) = &plan.0 else {
            panic!("fixture must be cleanup");
        };
        let [resume, finish] = cleanup.record.actions.as_slice() else {
            panic!("fixture must contain resume and finish actions");
        };
        let mut runtime_actions = cleanup.record.actions.as_slice().to_vec();
        runtime_actions.reverse();
        assert!(CleanupRecoveryActions::from_actions(runtime_actions).is_err());
        let mut actions = serde_json::to_value(&plan).unwrap()["actions"]
            .as_array()
            .unwrap()
            .clone();
        actions.reverse();
        assert!(!schema_accepts::<CleanupRecoveryActions>(&json!(actions)));

        let second_plan = RecoveryPlanStatus::cleanup_fixture_test_only(
            OperationId::parse("44444444-4444-4444-8444-444444444444").unwrap(),
            id("33333333-3333-4333-8333-333333333333"),
            serde_json::from_value(json!({
                "projectId": ID_1,
                "instanceId": ID_1,
                "role": "artifact"
            }))
            .unwrap(),
            TaskPhase::CleanedSuccess,
        )
        .unwrap();
        let RecoveryPlanStatusKind::Cleanup(second_cleanup) = &second_plan.0 else {
            panic!("fixture must be cleanup");
        };
        let [second_resume, second_finish] = second_cleanup.record.actions.as_slice() else {
            panic!("fixture must contain resume and finish actions");
        };

        let missing_terminal = vec![resume.clone(), second_resume.clone()];
        assert!(!schema_accepts::<CleanupRecoveryActions>(
            &serde_json::to_value(&missing_terminal).unwrap()
        ));
        let duplicate_terminal = vec![resume.clone(), finish.clone(), second_finish.clone()];
        assert!(!schema_accepts::<CleanupRecoveryActions>(
            &serde_json::to_value(&duplicate_terminal).unwrap()
        ));

        // Draft 2020-12 cannot express a variable-length "last item" relation
        // without enumerating every length. The wire schema intentionally
        // admits this structural permutation; only the sealed constructor can
        // reject it using the complete ordered catalog.
        let terminal_before_last = vec![resume.clone(), finish.clone(), second_resume.clone()];
        assert!(schema_accepts::<CleanupRecoveryActions>(
            &serde_json::to_value(&terminal_before_last).unwrap()
        ));
        assert!(CleanupRecoveryActions::from_actions(terminal_before_last).is_err());
    }

    #[test]
    fn task_recovery_plan_derives_exact_unknowns_actions_and_digest() {
        let unknown = RecoveryObservation::unknown_test_only(
            RecoveryObservationKind::TaskFingerprint,
            RecoverySubjectRef::registered(id(ID_1)),
            digest(A),
            RecoveryUnknownReason::EffectOutcomeUnavailable,
        )
        .unwrap();
        let action = verify_task_action(
            RecoveryObservationKind::TaskFingerprint,
            digest(A),
            digest(A),
        )
        .unwrap();
        let plan = RecoveryPlanStatus::task_configuration_test_only(
            OperationId::parse("22222222-2222-4222-8222-222222222222").unwrap(),
            TaskPhase::LocalVerified,
            vec![unknown],
            vec![action],
        )
        .unwrap();
        let encoded = serde_json::to_value(&plan).unwrap();
        assert_eq!(encoded["target"], "taskConfiguration");
        assert_eq!(encoded["effectClass"], "rollback");
        assert_eq!(encoded["remainingUnknowns"].as_array().unwrap().len(), 1);
        assert!(encoded.get("recoveryDigest").is_some());
        assert!(schema_accepts::<RecoveryPlanStatus>(&encoded));

        let wrong_action = release_root_lock_action().unwrap();
        assert!(RecoveryPlanStatus::task_configuration_test_only(
            OperationId::parse("22222222-2222-4222-8222-222222222222").unwrap(),
            TaskPhase::LocalVerified,
            Vec::new(),
            vec![wrong_action],
        )
        .is_err());
    }

    #[test]
    fn repository_commit_recovery_stages_have_disjoint_action_grammars() {
        let prior_operation_id =
            OperationId::parse("44444444-4444-4444-8444-444444444444").unwrap();
        let observe = RecoveryPlanStatus::repository_commit_observe_test_only(
            prior_operation_id.clone(),
            TaskPhase::CommittedUnverified,
            Vec::new(),
            vec![observe_commit_action().unwrap()],
        )
        .unwrap();
        let observe_json = serde_json::to_value(observe).unwrap();
        assert_eq!(observe_json["repositoryCommitStage"], "observeOutcome");
        assert!(schema_accepts::<RecoveryPlanStatus>(&observe_json));

        let committed = RecoveryPlanStatus::repository_commit_committed_test_only(
            prior_operation_id.clone(),
            Vec::new(),
            Vec::new(),
        )
        .unwrap();
        let committed_json = serde_json::to_value(committed).unwrap();
        assert_eq!(committed_json["repositoryCommitStage"], "committed");
        assert_eq!(committed_json["plannedResultPhase"], "committedAndUnlocked");
        assert!(schema_accepts::<RecoveryPlanStatus>(&committed_json));

        let restore = restore_original_action().unwrap();
        let release = release_root_lock_action_two().unwrap();
        let not_committed = RecoveryPlanStatus::repository_commit_not_committed_test_only(
            prior_operation_id.clone(),
            Vec::new(),
            vec![restore.clone(), release.clone()],
        )
        .unwrap();
        let not_committed_json = serde_json::to_value(not_committed).unwrap();
        assert_eq!(not_committed_json["repositoryCommitStage"], "notCommitted");
        assert_eq!(not_committed_json["plannedResultPhase"], "synchronized");
        assert!(schema_accepts::<RecoveryPlanStatus>(&not_committed_json));

        assert!(
            RecoveryPlanStatus::repository_commit_not_committed_test_only(
                prior_operation_id.clone(),
                Vec::new(),
                vec![release, restore],
            )
            .is_err()
        );
        assert!(RecoveryPlanStatus::repository_commit_committed_test_only(
            prior_operation_id,
            Vec::new(),
            vec![observe_commit_action().unwrap()],
        )
        .is_err());
    }

    #[test]
    fn prearm_recovery_observe_and_finalize_stages_are_physically_disjoint() {
        let mode_observation = RecoveryExpectedObservation::new(
            RecoveryObservationKind::ReservedOriginalLease,
            RecoverySubjectRef::reserved_original_infobase(digest(A)),
            digest(A),
        );
        let action =
            observe_prearm_outcome_action(prearm_terminal_observations(mode_observation)).unwrap();
        let prior_operation_id =
            OperationId::parse("55555555-5555-4555-8555-555555555555").unwrap();
        let plan = RecoveryPlanStatus::prearm_observe_test_only(
            prior_operation_id.clone(),
            TaskPhase::RecoveryRequired,
            Vec::new(),
            vec![action.clone()],
        )
        .unwrap();
        let encoded = serde_json::to_value(plan).unwrap();
        assert_eq!(encoded["preArmCancellationStage"], "observeOutcome");
        assert!(encoded.get("preArmCancellationEffectObservation").is_none());
        assert!(encoded
            .get("preArmCancellationFinalizationProgress")
            .is_none());
        assert!(schema_accepts::<RecoveryPlanStatus>(&encoded));

        assert!(RecoveryPlanStatus::prearm_observe_test_only(
            OperationId::parse("66666666-6666-4666-8666-666666666666").unwrap(),
            TaskPhase::RecoveryRequired,
            Vec::new(),
            vec![action],
        )
        .is_err());

        let digest_schema = schema::<PreArmFinalizeRecoveryPlanDigestRecord>();
        let status_schema = schema::<PreArmFinalizeRecoveryPlanStatus>();
        let digest_schema = serde_json::to_string(&digest_schema).unwrap();
        let status_schema = serde_json::to_string(&status_schema).unwrap();
        assert!(!digest_schema.contains("preArmCancellationFinalizationProgress"));
        assert!(status_schema.contains("preArmCancellationFinalizationProgress"));
    }

    #[test]
    fn recovery_subject_is_the_exact_closed_seven_leaf_union() {
        let contract = schema::<RecoverySubjectRef>();
        audit_json_schema(&contract).unwrap();
        assert_eq!(contract["oneOf"].as_array().map(Vec::len), Some(7));

        let valid = [
            json!({"subjectKind": "registered", "subjectId": ID_1}),
            json!({"subjectKind": "metadataObject", "objectId": ID_1}),
            json!({"subjectKind": "configurationRoot"}),
            json!({"subjectKind": "ownedRole", "locator": {
                "projectId": ID_1, "instanceId": ID_1, "role": "artifact"
            }}),
            json!({"subjectKind": "externalWorkingInfobase", "identity": {
                "computer": "host", "infobase": "manual", "digest": A
            }}),
            json!({"subjectKind": "reservedOriginalInfobase", "originalIdentityDigest": A}),
            json!({"subjectKind": "retentionLease", "retentionLeaseId": ID_1}),
        ];
        for value in valid {
            assert!(
                schema_accepts::<RecoverySubjectRef>(&value),
                "rejected {value}"
            );
        }
        for value in [
            json!({"subjectKind": "configurationRoot", "subjectId": ID_1}),
            json!({"subjectKind": "path", "path": "/tmp/unsafe"}),
            json!({"subjectKind": "registered"}),
        ] {
            assert!(
                !schema_accepts::<RecoverySubjectRef>(&value),
                "accepted {value}"
            );
        }
    }

    #[test]
    fn recovery_observation_outcomes_are_capability_values_with_exact_digests() {
        let subject = RecoverySubjectRef::registered(id(ID_1));
        let matched = RecoveryObservation::matched_test_only(
            RecoveryObservationKind::RepositoryVersion,
            subject.clone(),
            digest(A),
            digest(A),
        )
        .unwrap();
        assert!(RecoveryObservation::matched_test_only(
            RecoveryObservationKind::RepositoryVersion,
            subject.clone(),
            digest(A),
            digest(B),
        )
        .is_err());
        let differed = RecoveryObservation::differed_test_only(
            RecoveryObservationKind::RepositoryVersion,
            subject.clone(),
            digest(A),
            digest(B),
        )
        .unwrap();
        assert!(RecoveryObservation::differed_test_only(
            RecoveryObservationKind::RepositoryVersion,
            subject.clone(),
            digest(A),
            digest(A),
        )
        .is_err());
        let unknown = RecoveryObservation::unknown_test_only(
            RecoveryObservationKind::RepositoryVersion,
            subject,
            digest(A),
            RecoveryUnknownReason::EffectOutcomeUnavailable,
        )
        .unwrap();

        let matched_json = serde_json::to_value(&matched).unwrap();
        let differed_json = serde_json::to_value(&differed).unwrap();
        let unknown_json = serde_json::to_value(&unknown).unwrap();
        assert_eq!(matched_json["outcome"], "matches");
        assert_eq!(differed_json["outcome"], "differs");
        assert_eq!(unknown_json["outcome"], "unknown");
        assert_eq!(unknown_json["observedDigest"], Value::Null);
        assert_ne!(matched.observation_digest(), differed.observation_digest());
        assert_ne!(matched.observation_digest(), unknown.observation_digest());

        assert!(RecoveryUnknown::from_observation(&matched).is_err());
        assert!(RecoveryUnknown::from_observation(&differed).is_err());
        assert_eq!(
            serde_json::to_value(RecoveryUnknown::from_observation(&unknown).unwrap()).unwrap(),
            json!({
                "observationKind": "repositoryVersion",
                "subject": {"subjectKind": "registered", "subjectId": ID_1},
                "expectedDigest": A,
            })
        );

        audit_json_schema(&schema::<RecoveryObservationDigestRecord>()).unwrap();
        audit_json_schema(&schema::<RecoveryObservation>()).unwrap();
        audit_json_schema(&schema::<RecoveryUnknown>()).unwrap();
    }

    #[test]
    fn unknown_observation_schema_requires_a_literal_null_and_known_reason() {
        let valid = serde_json::to_value(
            RecoveryObservation::unknown_test_only(
                RecoveryObservationKind::LockOwnership,
                RecoverySubjectRef::configuration_root(),
                digest(A),
                RecoveryUnknownReason::CapabilityUnproven,
            )
            .unwrap(),
        )
        .unwrap();
        assert!(schema_accepts::<RecoveryObservation>(&valid));

        let mut non_null = valid.clone();
        non_null["observedDigest"] = json!(B);
        assert!(!schema_accepts::<RecoveryObservation>(&non_null));

        let mut unknown_reason = valid;
        unknown_reason["unknownReason"] = json!("operatorGuess");
        assert!(!schema_accepts::<RecoveryObservation>(&unknown_reason));
    }

    #[test]
    fn recovery_action_union_names_every_exact_closed_leaf() {
        let contract = schema::<RecoveryAction>();
        audit_json_schema(&contract).unwrap();
        assert_eq!(contract["oneOf"].as_array().map(Vec::len), Some(36));
        let encoded = serde_json::to_string(&contract).unwrap();
        for action_kind in [
            "releaseOwnedLocks",
            "restoreOriginal",
            "restoreTaskCheckpoint",
            "recreateTaskInfobase",
            "verifyTaskFingerprint",
            "observeCommit",
            "observePreArmCancellationOutcome",
            "acquirePreArmRootGuard",
            "acquirePreArmModeLease",
            "recheckPreArmCancellationFinalization",
            "applyPreArmCancellationSelectiveUpdate",
            "persistPreArmSupportCancellation",
            "releasePreArmModeLease",
            "releasePreArmRootGuard",
            "finishPreArmCancellationRecovery",
            "quarantineArtifact",
            "observeSupportPrerequisiteHistory",
            "updateOriginalSelectedTargets",
            "observeWorkingInfobaseLease",
            "releaseWorkingInfobaseLease",
            "observeReservedOriginalLease",
            "releaseReservedOriginalLease",
            "observeRetentionLease",
            "observeArchiveStaging",
            "releaseRetentionLease",
            "awaitExternalSupportCorrection",
            "awaitExternalLockRelease",
            "awaitManualWorkingInfobaseClosure",
            "awaitReservedOriginalClosure",
            "awaitExternalSupportConflictResolution",
            "awaitSupportRecoveryEvidence",
            "finalizeSupportPrerequisiteRecovery",
            "resumeQuarantine",
            "resumeOwnedTargetQuarantine",
            "finishArchive",
            "finishCleanup",
        ] {
            assert!(encoded.contains(action_kind), "missing {action_kind}");
        }
        audit_json_schema(&schema::<RecoveryActionDigestRecord>()).unwrap();
    }

    #[test]
    fn recovery_action_authority_rejects_wrong_observation_projection() {
        let valid = verify_task_action(
            RecoveryObservationKind::TaskFingerprint,
            digest(A),
            digest(A),
        )
        .unwrap();
        let encoded = serde_json::to_value(valid).unwrap();
        assert_eq!(encoded["actionKind"], "verifyTaskFingerprint");
        assert!(encoded.get("actionDigest").is_some());

        assert!(verify_task_action(
            RecoveryObservationKind::ObjectFingerprint,
            digest(A),
            digest(A),
        )
        .is_err());
        assert!(verify_task_action(
            RecoveryObservationKind::TaskFingerprint,
            digest(B),
            digest(A),
        )
        .is_err());
    }

    #[test]
    fn action_outcome_requires_exact_ordered_matched_observations() {
        let action = verify_task_action(
            RecoveryObservationKind::TaskFingerprint,
            digest(A),
            digest(A),
        )
        .unwrap();
        let matched = RecoveryObservation::matched_test_only(
            RecoveryObservationKind::TaskFingerprint,
            RecoverySubjectRef::registered(id(ID_1)),
            digest(A),
            digest(A),
        )
        .unwrap();
        let outcome =
            RecoveryActionOutcome::already_satisfied_test_only(&action, vec![matched.clone()])
                .unwrap();
        let encoded = serde_json::to_value(outcome).unwrap();
        assert_eq!(encoded["outcome"], "alreadySatisfied");
        assert_eq!(
            encoded["terminalObservationDigests"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert!(encoded.get("receipt").is_none());

        let differed = RecoveryObservation::differed_test_only(
            RecoveryObservationKind::TaskFingerprint,
            RecoverySubjectRef::registered(id(ID_1)),
            digest(A),
            digest(B),
        )
        .unwrap();
        assert!(
            RecoveryActionOutcome::already_satisfied_test_only(&action, vec![differed]).is_err()
        );
        assert!(
            RecoveryActionOutcome::performed_test_only(&action, vec![matched], id(ID_1),).is_err()
        );

        audit_json_schema(&schema::<EffectReceipt>()).unwrap();
        audit_json_schema(&schema::<RecoveryActionOutcomeDigestRecord>()).unwrap();
        audit_json_schema(&schema::<RecoveryActionOutcome>()).unwrap();
    }

    #[test]
    fn prearm_outcome_binds_the_exact_planned_receipt_and_producer() {
        let receipt_id = id("44444444-4444-4444-8444-444444444444");
        let effect_intent_digest = digest(B);
        let action = acquire_prearm_root_action(PreArmCancellationReceiptRef::finalization_plan(
            receipt_id.clone(),
            PreArmCancellationEffectKind::RootGuardAcquire,
            effect_intent_digest.clone(),
        ))
        .unwrap();
        let observation = RecoveryObservation::matched_test_only(
            RecoveryObservationKind::LockOwnership,
            RecoverySubjectRef::configuration_root(),
            digest(A),
            digest(A),
        )
        .unwrap();
        let (action_id, _, _, action_digest) = action.common();
        let receipt = PreArmCancellationEffectReceipt::new(
            PreArmCancellationEffectReceiptAuthority::test_only(
                receipt_id,
                PreArmCancellationEffectKind::RootGuardAcquire,
                effect_intent_digest,
                action_id.clone(),
                action_digest.clone(),
                vec![observation.observation_digest().clone()],
            )
            .unwrap(),
        )
        .unwrap();

        let performed = RecoveryActionOutcome::performed_from_prearm_receipt_test_only(
            &action,
            vec![observation.clone()],
            receipt.clone(),
        )
        .unwrap();
        assert_eq!(
            serde_json::to_value(performed).unwrap()["outcome"],
            "performed"
        );
        assert!(
            RecoveryActionOutcome::recovered_from_prearm_receipt_test_only(
                &action,
                vec![observation.clone()],
                receipt,
            )
            .is_ok()
        );

        let substituted = PreArmCancellationEffectReceipt::new(
            PreArmCancellationEffectReceiptAuthority::test_only(
                id("55555555-5555-4555-8555-555555555555"),
                PreArmCancellationEffectKind::ModeLeaseAcquire,
                digest(B),
                action_id.clone(),
                action_digest.clone(),
                vec![observation.observation_digest().clone()],
            )
            .unwrap(),
        )
        .unwrap();
        assert!(
            RecoveryActionOutcome::performed_from_prearm_receipt_test_only(
                &action,
                vec![observation],
                substituted,
            )
            .is_err()
        );

        let wrong_kind =
            acquire_prearm_root_action(PreArmCancellationReceiptRef::finalization_plan(
                id("66666666-6666-4666-8666-666666666666"),
                PreArmCancellationEffectKind::RootGuardRelease,
                digest(B),
            ));
        assert!(wrong_kind.is_err());
    }

    #[test]
    fn prearm_outcome_preserves_expected_observation_order_without_digest_sorting() {
        let receipt_id = id("55555555-5555-4555-8555-555555555555");
        let effect_intent_digest = digest(B);
        let action = selective_prearm_action(PreArmCancellationReceiptRef::finalization_plan(
            receipt_id.clone(),
            PreArmCancellationEffectKind::SelectiveOriginalUpdate,
            effect_intent_digest.clone(),
        ))
        .unwrap();
        let observations = vec![
            RecoveryObservation::matched_test_only(
                RecoveryObservationKind::ObjectFingerprint,
                RecoverySubjectRef::metadata_object(MetadataObjectId::parse(ID_1).unwrap()),
                digest(A),
                digest(A),
            )
            .unwrap(),
            RecoveryObservation::matched_test_only(
                RecoveryObservationKind::ObjectFingerprint,
                RecoverySubjectRef::configuration_root(),
                digest(A),
                digest(A),
            )
            .unwrap(),
        ];
        let exact_digests = observations
            .iter()
            .map(|observation| observation.observation_digest().clone())
            .collect::<Vec<_>>();
        assert!(
            exact_digests[0] > exact_digests[1],
            "fixture must differ from lexicographic digest order"
        );
        let (action_id, _, _, action_digest) = action.common();
        let exact_receipt = PreArmCancellationEffectReceipt::new(
            PreArmCancellationEffectReceiptAuthority::test_only(
                receipt_id.clone(),
                PreArmCancellationEffectKind::SelectiveOriginalUpdate,
                effect_intent_digest.clone(),
                action_id.clone(),
                action_digest.clone(),
                exact_digests.clone(),
            )
            .unwrap(),
        )
        .unwrap();
        assert!(
            RecoveryActionOutcome::performed_from_prearm_receipt_test_only(
                &action,
                observations.clone(),
                exact_receipt,
            )
            .is_ok()
        );

        let reordered_receipt = PreArmCancellationEffectReceipt::new(
            PreArmCancellationEffectReceiptAuthority::test_only(
                receipt_id.clone(),
                PreArmCancellationEffectKind::SelectiveOriginalUpdate,
                effect_intent_digest.clone(),
                action_id.clone(),
                action_digest.clone(),
                exact_digests.iter().cloned().rev().collect(),
            )
            .unwrap(),
        )
        .unwrap();
        assert!(
            RecoveryActionOutcome::performed_from_prearm_receipt_test_only(
                &action,
                observations.clone(),
                reordered_receipt,
            )
            .is_err()
        );

        let substituted_receipt = PreArmCancellationEffectReceipt::new(
            PreArmCancellationEffectReceiptAuthority::test_only(
                receipt_id,
                PreArmCancellationEffectKind::SelectiveOriginalUpdate,
                effect_intent_digest,
                action_id.clone(),
                action_digest.clone(),
                vec![exact_digests[0].clone(), digest(B)],
            )
            .unwrap(),
        )
        .unwrap();
        assert!(
            RecoveryActionOutcome::performed_from_prearm_receipt_test_only(
                &action,
                observations,
                substituted_receipt,
            )
            .is_err()
        );
    }

    #[test]
    fn prearm_recheck_requires_exact_mode_and_policy_observation_projection() {
        let working_mode = RecoveryExpectedObservation::new(
            RecoveryObservationKind::WorkingInfobaseLease,
            RecoverySubjectRef::external_working_infobase(working_identity()),
            digest(A),
        );
        let valid_observations = valid_recheck_observations(working_mode.clone());
        let valid = recheck_prearm_action(valid_observations.clone(), digest(B)).unwrap();

        let matched = valid_observations
            .iter()
            .map(|expected| {
                RecoveryObservation::matched_test_only(
                    expected.observation_kind,
                    expected.subject.clone(),
                    expected.expected_digest.clone(),
                    expected.expected_digest.clone(),
                )
                .unwrap()
            })
            .collect();
        assert!(RecoveryActionOutcome::already_satisfied_test_only(&valid, matched).is_ok());

        assert!(recheck_prearm_action(valid_observations.clone(), digest(A)).is_err());

        let mut both_modes = valid_observations.clone();
        both_modes.insert(
            5,
            RecoveryExpectedObservation::new(
                RecoveryObservationKind::ReservedOriginalLease,
                RecoverySubjectRef::reserved_original_infobase(digest(A)),
                digest(A),
            ),
        );
        assert!(recheck_prearm_action(both_modes, digest(B)).is_err());

        let mut wrong_mode_subject = valid_observations.clone();
        wrong_mode_subject[4] = RecoveryExpectedObservation::new(
            RecoveryObservationKind::WorkingInfobaseLease,
            RecoverySubjectRef::reserved_original_infobase(digest(A)),
            digest(A),
        );
        assert!(recheck_prearm_action(wrong_mode_subject, digest(B)).is_err());

        let mut extra_object = valid_observations;
        extra_object.insert(
            2,
            RecoveryExpectedObservation::new(
                RecoveryObservationKind::ObjectFingerprint,
                RecoverySubjectRef::registered(id(ID_1)),
                digest(A),
            ),
        );
        assert!(recheck_prearm_action(extra_object, digest(B)).is_err());
    }

    #[test]
    fn prearm_terminal_actions_require_the_exact_five_observation_projection() {
        let working_mode = RecoveryExpectedObservation::new(
            RecoveryObservationKind::WorkingInfobaseLease,
            RecoverySubjectRef::external_working_infobase(working_identity()),
            digest(A),
        );
        let valid_working = prearm_terminal_observations(working_mode.clone());
        assert!(observe_prearm_outcome_action(valid_working.clone()).is_ok());
        assert!(finish_prearm_action(valid_working.clone()).is_ok());

        let reserved_mode = RecoveryExpectedObservation::new(
            RecoveryObservationKind::ReservedOriginalLease,
            RecoverySubjectRef::reserved_original_infobase(digest(A)),
            digest(A),
        );
        let valid_reserved = prearm_terminal_observations(reserved_mode.clone());
        assert!(observe_prearm_outcome_action(valid_reserved.clone()).is_ok());
        assert!(finish_prearm_action(valid_reserved).is_ok());

        let mut extra_object = valid_working.clone();
        extra_object.insert(
            3,
            RecoveryExpectedObservation::new(
                RecoveryObservationKind::ObjectFingerprint,
                RecoverySubjectRef::registered(id("55555555-5555-4555-8555-555555555555")),
                digest(A),
            ),
        );

        let mut both_modes = valid_working.clone();
        both_modes.push(reserved_mode);

        let mut wrong_authorization = valid_working.clone();
        wrong_authorization[1] = RecoveryExpectedObservation::new(
            RecoveryObservationKind::SupportActionAuthorization,
            RecoverySubjectRef::registered(id("55555555-5555-4555-8555-555555555555")),
            digest(A),
        );

        let mut wrong_object = valid_working.clone();
        wrong_object[2] = RecoveryExpectedObservation::new(
            RecoveryObservationKind::ObjectFingerprint,
            RecoverySubjectRef::external_working_infobase(working_identity()),
            digest(A),
        );

        let mut wrong_root = valid_working.clone();
        wrong_root[3] = RecoveryExpectedObservation::new(
            RecoveryObservationKind::LockOwnership,
            RecoverySubjectRef::registered(id("55555555-5555-4555-8555-555555555555")),
            digest(A),
        );

        let mut wrong_mode = valid_working;
        wrong_mode[4] = RecoveryExpectedObservation::new(
            RecoveryObservationKind::WorkingInfobaseLease,
            RecoverySubjectRef::reserved_original_infobase(digest(A)),
            digest(A),
        );

        for invalid in [
            extra_object,
            both_modes,
            wrong_authorization,
            wrong_object,
            wrong_root,
            wrong_mode,
        ] {
            assert!(observe_prearm_outcome_action(invalid.clone()).is_err());
            assert!(finish_prearm_action(invalid).is_err());
        }
    }

    #[test]
    fn support_history_requires_exact_bound_anchor_only() {
        let anchor = RecoveryExpectedObservation::new(
            RecoveryObservationKind::RepositoryAnchor,
            RecoverySubjectRef::registered(id("22222222-2222-4222-8222-222222222222")),
            digest(A),
        );
        let version = RecoveryExpectedObservation::new(
            RecoveryObservationKind::RepositoryVersion,
            RecoverySubjectRef::registered(id(ID_1)),
            digest(B),
        );
        let valid = observe_history_action(vec![anchor.clone()]).unwrap();
        assert!(observe_history_action(vec![anchor.clone(), version.clone()]).is_err());
        assert!(observe_history_action(vec![version.clone()]).is_err());
        for wrong_anchor in [
            RecoveryExpectedObservation::new(
                RecoveryObservationKind::RepositoryAnchor,
                RecoverySubjectRef::configuration_root(),
                digest(A),
            ),
            RecoveryExpectedObservation::new(
                RecoveryObservationKind::RepositoryAnchor,
                RecoverySubjectRef::registered(id(ID_1)),
                digest(A),
            ),
            RecoveryExpectedObservation::new(
                RecoveryObservationKind::RepositoryAnchor,
                RecoverySubjectRef::registered(id("22222222-2222-4222-8222-222222222222")),
                digest(B),
            ),
        ] {
            assert!(observe_history_action(vec![wrong_anchor]).is_err());
        }

        let action_schema = schema::<RecoveryAction>();
        let validator = jsonschema::options()
            .with_draft(jsonschema::Draft::Draft202012)
            .build(&action_schema)
            .unwrap();
        let valid = serde_json::to_value(valid).unwrap();
        assert!(validator.is_valid(&valid));

        let mut extra_version = valid.clone();
        extra_version["expectedObservations"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::to_value(version).unwrap());
        assert!(!validator.is_valid(&extra_version));

        let mut missing = valid.clone();
        missing["expectedObservations"]
            .as_array_mut()
            .unwrap()
            .remove(0);
        assert!(!validator.is_valid(&missing));

        let mut wrong_kind = valid;
        wrong_kind["expectedObservations"][0]["observationKind"] = json!("repositoryVersion");
        assert!(!validator.is_valid(&wrong_kind));
    }

    #[test]
    fn recovery_action_schema_rejects_cross_kind_and_extra_observation_bags() {
        let action_schema = schema::<RecoveryAction>();
        let validator = jsonschema::options()
            .with_draft(jsonschema::Draft::Draft202012)
            .build(&action_schema)
            .unwrap();
        let valid = serde_json::to_value(
            verify_task_action(
                RecoveryObservationKind::TaskFingerprint,
                digest(A),
                digest(A),
            )
            .unwrap(),
        )
        .unwrap();
        assert!(validator.is_valid(&valid));

        let mut cross_kind = valid.clone();
        cross_kind["expectedObservations"][0]["observationKind"] = json!("objectFingerprint");
        assert!(!validator.is_valid(&cross_kind));

        let mut extra = valid;
        let mut additional = extra["expectedObservations"][0].clone();
        additional["subject"]["subjectId"] = json!("33333333-3333-4333-8333-333333333333");
        extra["expectedObservations"]
            .as_array_mut()
            .unwrap()
            .push(additional);
        assert!(!validator.is_valid(&extra));

        let recheck = serde_json::to_value(
            recheck_prearm_action(
                valid_recheck_observations(RecoveryExpectedObservation::new(
                    RecoveryObservationKind::WorkingInfobaseLease,
                    RecoverySubjectRef::external_working_infobase(working_identity()),
                    digest(A),
                )),
                digest(B),
            )
            .unwrap(),
        )
        .unwrap();
        assert!(validator.is_valid(&recheck));

        let mut both_mode_identities = recheck.clone();
        both_mode_identities["expectedObservations"]
            .as_array_mut()
            .unwrap()
            .insert(
                5,
                serde_json::to_value(RecoveryExpectedObservation::new(
                    RecoveryObservationKind::ReservedOriginalLease,
                    RecoverySubjectRef::reserved_original_infobase(digest(A)),
                    digest(A),
                ))
                .unwrap(),
            );
        assert!(!validator.is_valid(&both_mode_identities));

        let mut wrong_mode_identity = recheck;
        wrong_mode_identity["expectedObservations"][4]["subject"] =
            serde_json::to_value(RecoverySubjectRef::reserved_original_infobase(digest(A)))
                .unwrap();
        assert!(!validator.is_valid(&wrong_mode_identity));
    }

    #[test]
    fn release_prearm_mode_lease_forbids_acquire_only_support_action_id() {
        let receipt_ref = serde_json::to_value(PreArmCancellationReceiptRef::finalization_plan(
            id("88888888-8888-4888-8888-888888888888"),
            PreArmCancellationEffectKind::ModeLeaseRelease,
            digest(B),
        ))
        .unwrap();
        let schema_working_identity = working_identity();
        let observation = RecoveryExpectedObservation::new(
            RecoveryObservationKind::WorkingInfobaseLease,
            RecoverySubjectRef::external_working_infobase(schema_working_identity.clone()),
            digest(A),
        );
        let release_record = json!({
            "actionKind": "releasePreArmModeLease",
            "actionId": ID_1,
            "finalizationAttemptId": "22222222-2222-4222-8222-222222222222",
            "finalizationPlanDigest": A,
            "manualTargetMode": "separateWorkingInfobase",
            "workingInfobaseIdentity": schema_working_identity,
            "exclusiveLeaseCapabilityId": "prearm-mode-lease.v1",
            "receiptRef": receipt_ref,
            "expectedObservations": [observation],
            "expectedPostconditionDigest": A,
        });
        assert!(schema_accepts::<ReleasePreArmModeLeaseActionDigestRecord>(
            &release_record
        ));
        assert!(schema_accepts::<RecoveryActionDigestRecord>(
            &release_record
        ));

        let mut release_action = release_record.clone();
        release_action["actionDigest"] = json!(B);
        assert!(schema_accepts::<ReleasePreArmModeLeaseAction>(
            &release_action
        ));
        assert!(schema_accepts::<RecoveryAction>(&release_action));

        let mut substituted_release_record = release_record;
        substituted_release_record["supportActionId"] =
            json!("33333333-3333-4333-8333-333333333333");
        assert!(!schema_accepts::<ReleasePreArmModeLeaseActionDigestRecord>(
            &substituted_release_record
        ));
        let mut substituted_release_action = substituted_release_record;
        substituted_release_action["actionDigest"] = json!(B);
        assert!(!schema_accepts::<ReleasePreArmModeLeaseAction>(
            &substituted_release_action
        ));

        let mut acquire_record = release_action;
        acquire_record
            .as_object_mut()
            .unwrap()
            .remove("actionDigest");
        acquire_record["actionKind"] = json!("acquirePreArmModeLease");
        acquire_record["supportActionId"] = json!("33333333-3333-4333-8333-333333333333");
        acquire_record["receiptRef"] =
            serde_json::to_value(PreArmCancellationReceiptRef::finalization_plan(
                id("88888888-8888-4888-8888-888888888888"),
                PreArmCancellationEffectKind::ModeLeaseAcquire,
                digest(B),
            ))
            .unwrap();
        assert!(schema_accepts::<AcquirePreArmModeLeaseActionDigestRecord>(
            &acquire_record
        ));
        acquire_record
            .as_object_mut()
            .unwrap()
            .remove("supportActionId");
        assert!(!schema_accepts::<AcquirePreArmModeLeaseActionDigestRecord>(
            &acquire_record
        ));

        let working_identity = working_identity();
        let (expected_observations, expected_postcondition_digest) =
            expected_postcondition(vec![RecoveryExpectedObservation::new(
                RecoveryObservationKind::WorkingInfobaseLease,
                RecoverySubjectRef::external_working_infobase(working_identity.clone()),
                digest(A),
            )])
            .unwrap();
        let release = runtime_action(RecoveryActionDigestRecordKind::ReleasePreArmModeLease(
            ReleasePreArmModeLeaseActionDigestRecord(
                ReleasePreArmModeLeaseActionDigestRecordKind::SeparateWorkingInfobase(
                    ReleasePreArmWorkingInfobaseModeLeaseActionDigestRecord {
                        action_kind: ReleasePreArmModeLeaseActionKind::Value,
                        action_id: id(ID_1),
                        finalization_attempt_id: id("22222222-2222-4222-8222-222222222222"),
                        finalization_plan_digest: digest(A),
                        manual_target_mode: SeparateWorkingInfobaseModeLiteral::Value,
                        working_infobase_identity: working_identity.clone(),
                        exclusive_lease_capability_id: CapabilityRowId::parse(
                            "prearm-mode-lease.v1",
                        )
                        .unwrap(),
                        receipt_ref: PreArmCancellationReceiptRef::finalization_plan(
                            id("88888888-8888-4888-8888-888888888888"),
                            PreArmCancellationEffectKind::ModeLeaseRelease,
                            digest(B),
                        ),
                        expected_observations,
                        expected_postcondition_digest,
                    },
                ),
            ),
        ))
        .unwrap();
        assert!(serde_json::to_value(&release)
            .unwrap()
            .get("supportActionId")
            .is_none());

        let observation = RecoveryObservation::matched_test_only(
            RecoveryObservationKind::WorkingInfobaseLease,
            RecoverySubjectRef::external_working_infobase(working_identity),
            digest(A),
            digest(A),
        )
        .unwrap();
        let (action_id, _, _, action_digest) = release.common();
        let receipt = PreArmCancellationEffectReceipt::new(
            PreArmCancellationEffectReceiptAuthority::test_only(
                id("88888888-8888-4888-8888-888888888888"),
                PreArmCancellationEffectKind::ModeLeaseRelease,
                digest(B),
                action_id.clone(),
                action_digest.clone(),
                vec![observation.observation_digest().clone()],
            )
            .unwrap(),
        )
        .unwrap();
        assert!(
            RecoveryActionOutcome::performed_from_prearm_receipt_test_only(
                &release,
                vec![observation],
                receipt,
            )
            .is_ok()
        );
    }

    #[test]
    fn archive_staging_receipt_binds_the_durable_handoff_barrier() {
        let receipt = ArchiveStagingReceipt::test_only(
            id(ID_1),
            id("22222222-2222-4222-8222-222222222222"),
            digest(A),
            digest(B),
            digest(A),
            id("33333333-3333-4333-8333-333333333333"),
        )
        .unwrap();
        let encoded = serde_json::to_value(receipt).unwrap();
        assert_eq!(encoded["fileSynced"], true);
        assert_eq!(encoded["parentDirectorySynced"], true);
        assert!(encoded.get("receiptDigest").is_some());

        audit_json_schema(&schema::<ArchiveStagingReceiptDigestRecord>()).unwrap();
        audit_json_schema(&schema::<ArchiveStagingReceipt>()).unwrap();
        let mut substitution = encoded;
        substitution["fileSynced"] = json!(false);
        assert!(!schema_accepts::<ArchiveStagingReceipt>(&substitution));
    }

    struct Task12StagingParser {
        expected_generation_id: UnicaId,
        expected_bytes: Vec<u8>,
        staged_entry_manifest_digest: Sha256Digest,
    }

    impl archive_parser_sealed::Sealed for Task12StagingParser {}

    impl ArchiveStagingStrictParser for Task12StagingParser {
        fn parse_staged_generation(
            &self,
            generation_id: &UnicaId,
            bytes: &[u8],
        ) -> Result<Sha256Digest, RecoveryContractError> {
            if generation_id != &self.expected_generation_id || bytes != self.expected_bytes {
                return Err(RecoveryContractError(
                    "strict staging parser received another byte generation",
                ));
            }
            Ok(self.staged_entry_manifest_digest.clone())
        }
    }

    #[test]
    fn task12_archive_staging_hash_parse_and_fsync_consume_one_byte_generation() {
        use sha2::{Digest, Sha256};

        let generation_id = id("33333333-3333-4333-8333-333333333333");
        let archive_id = id(ID_2);
        let bytes = b"immutable staged archive bytes".to_vec();
        let expected_sha256 = Sha256Digest::parse(&format!("{:x}", Sha256::digest(&bytes)))
            .expect("SHA-256 output is canonical");
        let expected_manifest_digest = digest(B);
        let generation = ImmutableArchiveByteGeneration::seal_staged_from_writer_test_only(
            ArchiveStagingWriterLineage::test_only(
                CapabilityRowId::parse("archive-container.v1").unwrap(),
                generation_id.clone(),
                id("44444444-4444-4444-8444-444444444444"),
                archive_id.clone(),
                digest(A),
                digest(B),
                id("55555555-5555-4555-8555-555555555555"),
            ),
            bytes.clone(),
        )
        .unwrap();
        let observation = generation
            .observe_staging(&Task12StagingParser {
                expected_generation_id: generation_id,
                expected_bytes: bytes,
                staged_entry_manifest_digest: expected_manifest_digest.clone(),
            })
            .unwrap();

        assert_eq!(
            observation.staged_archive_sha256().as_digest(),
            &expected_sha256
        );
        assert_eq!(
            observation.staged_entry_manifest_digest(),
            &expected_manifest_digest
        );
        assert_eq!(observation.archive_id(), &archive_id);

        let authority = ArchiveStagingReceiptAuthority::from_observation(
            id(ID_1),
            expected_manifest_digest,
            observation,
        )
        .unwrap();
        let receipt = ArchiveStagingReceipt::new(authority).unwrap();
        assert_eq!(receipt.archive_id(), &archive_id);
        assert_eq!(receipt.staged_archive_sha256(), &expected_sha256);
        assert_eq!(receipt.handoff_lineage_digest(), &digest(A));
        assert_eq!(receipt.frozen_provider_boundary_digest(), &digest(B));
    }

    #[test]
    fn task12_archive_staging_rejects_a_manifest_from_another_generation() {
        let generation = ImmutableArchiveByteGeneration::seal_staged_from_writer_test_only(
            ArchiveStagingWriterLineage::test_only(
                CapabilityRowId::parse("archive-container.v1").unwrap(),
                id("33333333-3333-4333-8333-333333333333"),
                id("44444444-4444-4444-8444-444444444444"),
                id(ID_2),
                digest(A),
                digest(B),
                id("55555555-5555-4555-8555-555555555555"),
            ),
            b"generation-a".to_vec(),
        )
        .unwrap();

        assert!(generation
            .observe_staging(&Task12StagingParser {
                expected_generation_id: id("66666666-6666-4666-8666-666666666666"),
                expected_bytes: b"generation-b".to_vec(),
                staged_entry_manifest_digest: digest(A),
            })
            .is_err());
    }

    struct Task12PublicationParser {
        expected_generation_id: UnicaId,
        expected_bytes: Vec<u8>,
        parsed_entry_set_digest: Sha256Digest,
        embedded_manifest_digest: Sha256Digest,
    }

    impl archive_parser_sealed::Sealed for Task12PublicationParser {}

    impl ArchivePublicationStrictParser for Task12PublicationParser {
        fn parse_published_generation(
            &self,
            generation_id: &UnicaId,
            bytes: &[u8],
        ) -> Result<ArchivePublicationParsedDigests, RecoveryContractError> {
            if generation_id != &self.expected_generation_id || bytes != self.expected_bytes {
                return Err(RecoveryContractError(
                    "strict publication parser received another byte generation",
                ));
            }
            Ok(ArchivePublicationParsedDigests::new(
                self.parsed_entry_set_digest.clone(),
                self.embedded_manifest_digest.clone(),
            ))
        }
    }

    #[test]
    fn task12_archive_publication_hash_and_parse_consume_one_byte_generation() {
        use sha2::{Digest, Sha256};

        let generation_id = id("33333333-3333-4333-8333-333333333333");
        let bytes = b"immutable final archive bytes".to_vec();
        let expected_sha256 = Sha256Digest::parse(&format!("{:x}", Sha256::digest(&bytes)))
            .expect("SHA-256 output is canonical");
        let manifest_digest = digest(B);
        let generation = ImmutableArchiveByteGeneration::seal_published_from_writer_test_only(
            CapabilityRowId::parse("archive-container.v1").unwrap(),
            generation_id.clone(),
            id("44444444-4444-4444-8444-444444444444"),
            id(ID_2),
            manifest_digest.clone(),
            bytes.clone(),
            id("55555555-5555-4555-8555-555555555555"),
        )
        .unwrap();
        let observation = generation
            .observe_publication(&Task12PublicationParser {
                expected_generation_id: generation_id,
                expected_bytes: bytes,
                parsed_entry_set_digest: digest(A),
                embedded_manifest_digest: manifest_digest.clone(),
            })
            .unwrap();

        assert_eq!(
            observation.final_archive_sha256().as_digest(),
            &expected_sha256
        );
        assert_eq!(observation.publication_manifest_digest(), &manifest_digest);
        assert_eq!(observation.parsed_entry_set_digest(), &digest(A));
        assert_eq!(observation.final_archive_size(), 29);
    }

    #[test]
    fn task12_archive_publication_rejects_an_embedded_manifest_substitution() {
        let generation = ImmutableArchiveByteGeneration::seal_published_from_writer_test_only(
            CapabilityRowId::parse("archive-container.v1").unwrap(),
            id("33333333-3333-4333-8333-333333333333"),
            id("44444444-4444-4444-8444-444444444444"),
            id(ID_2),
            digest(A),
            b"immutable final archive bytes".to_vec(),
            id("55555555-5555-4555-8555-555555555555"),
        )
        .unwrap();

        assert!(generation
            .observe_publication(&Task12PublicationParser {
                expected_generation_id: id("33333333-3333-4333-8333-333333333333"),
                expected_bytes: b"immutable final archive bytes".to_vec(),
                parsed_entry_set_digest: digest(B),
                embedded_manifest_digest: digest(B),
            })
            .is_err());
    }

    #[test]
    fn task12_archive_byte_generation_is_linear_and_hash_layers_are_distinct() {
        fn accepts_staged(_: &StagedArchiveSha256) {}
        fn accepts_published(_: &PublishedArchiveSha256) {}

        let staged = StagedArchiveSha256::test_only(digest(A));
        let published = PublishedArchiveSha256::test_only(digest(A));
        accepts_staged(&staged);
        accepts_published(&published);

        const _: fn() = || {
            trait AmbiguousIfClone<Marker> {
                fn assert_not_clone() {}
            }
            struct ImplementsClone;
            impl<T: ?Sized> AmbiguousIfClone<()> for T {}
            impl<T: Clone> AmbiguousIfClone<ImplementsClone> for T {}
            let _ = <ImmutableArchiveByteGeneration as AmbiguousIfClone<_>>::assert_not_clone;
            let _ = <PublishedArchiveSha256 as AmbiguousIfClone<_>>::assert_not_clone;
            let _ = <ArchivePublicationByteObservation as AmbiguousIfClone<_>>::assert_not_clone;
        };
    }

    #[test]
    fn target_effect_action_matrix_has_no_cross_row_authority_value() {
        let verify = verify_task_action(
            RecoveryObservationKind::TaskFingerprint,
            digest(A),
            digest(A),
        )
        .unwrap();
        let task = RecoveryActionPlan::test_only(
            RecoveryTarget::TaskConfiguration,
            RecoveryEffectClass::Rollback,
            vec![verify.clone()],
        )
        .unwrap();
        assert_eq!(
            serde_json::to_value(task).unwrap()["target"],
            "taskConfiguration"
        );

        assert!(RecoveryActionPlan::test_only(
            RecoveryTarget::RepositoryLocks,
            RecoveryEffectClass::Compensate,
            vec![verify],
        )
        .is_err());
        assert!(RecoveryActionPlan::test_only(
            RecoveryTarget::TaskConfiguration,
            RecoveryEffectClass::Compensate,
            vec![release_root_lock_action().unwrap()],
        )
        .is_err());
        assert!(RecoveryActionPlan::test_only(
            RecoveryTarget::RepositoryLocks,
            RecoveryEffectClass::Compensate,
            vec![release_root_lock_action().unwrap()],
        )
        .is_ok());
        let prearm = acquire_prearm_root_action(PreArmCancellationReceiptRef::finalization_plan(
            id("77777777-7777-4777-8777-777777777777"),
            PreArmCancellationEffectKind::RootGuardAcquire,
            digest(B),
        ))
        .unwrap();
        assert!(RecoveryActionPlan::test_only(
            RecoveryTarget::PreArmSupportCancellation,
            RecoveryEffectClass::ReconcileOnly,
            vec![prearm],
        )
        .is_err());
    }

    #[test]
    fn finish_archive_rejects_a_substituted_release_set_digest() {
        let archive_id = id(ID_1);
        let retention_lease_id = id("22222222-2222-4222-8222-222222222222");
        let expected_releases =
            HandoffRetentionReleaseReceipts::new(vec![HandoffRetentionReleaseReceipt {
                retention_lease_id: retention_lease_id.clone(),
                release_action_id: id("33333333-3333-4333-8333-333333333333"),
                release_action_digest: digest(A),
                release_receipt_id: id("44444444-4444-4444-8444-444444444444"),
                release_receipt_digest: digest(B),
            }])
            .unwrap();
        let (expected_observations, expected_postcondition_digest) =
            expected_postcondition(vec![RecoveryExpectedObservation::new(
                RecoveryObservationKind::ArchivePresence,
                RecoverySubjectRef::registered(archive_id.clone()),
                digest(A),
            )])
            .unwrap();
        let expected_release_set_digest = contract_digest(
            &HandoffRetentionReleaseSetDigestRecord(expected_releases.clone()),
            "test release-set digest failed",
        )
        .unwrap();
        let valid = FinishArchiveActionDigestRecord {
            action_kind: FinishArchiveActionKind::Value,
            action_id: id("55555555-5555-4555-8555-555555555555"),
            archive_id,
            archive_staging_receipt_id: id("66666666-6666-4666-8666-666666666666"),
            expected_archive_staging_receipt_digest: digest(A),
            handoff_lineage_digest: digest(B),
            retention_lease_ids: RecoveryUnicaIds::new(vec![retention_lease_id]).unwrap(),
            expected_releases,
            expected_release_set_digest: expected_release_set_digest.clone(),
            expected_observations,
            expected_postcondition_digest,
        };
        assert!(
            runtime_action(RecoveryActionDigestRecordKind::FinishArchive(valid.clone(),)).is_ok()
        );

        let mut substituted = valid;
        substituted.expected_release_set_digest = if expected_release_set_digest == digest(A) {
            digest(B)
        } else {
            digest(A)
        };
        assert!(
            runtime_action(RecoveryActionDigestRecordKind::FinishArchive(substituted,)).is_err()
        );
    }

    #[test]
    fn task12_archive_grammar_and_handoff_receipt_bind_release_lineage() {
        let archive_id = id(ID_1);
        let staging_receipt_id = id("22222222-2222-4222-8222-222222222222");
        let retention_lease_id = id("33333333-3333-4333-8333-333333333333");
        let release_receipt_id = id("77777777-7777-4777-8777-777777777777");

        let (staging_observations, staging_postcondition) =
            expected_postcondition(vec![RecoveryExpectedObservation::new(
                RecoveryObservationKind::ArchiveStagingPresence,
                RecoverySubjectRef::registered(staging_receipt_id.clone()),
                digest(A),
            )])
            .unwrap();
        let observe_staging =
            runtime_action(RecoveryActionDigestRecordKind::ObserveArchiveStaging(
                ObserveArchiveStagingActionDigestRecord {
                    action_kind: ObserveArchiveStagingActionKind::Value,
                    action_id: id("44444444-4444-4444-8444-444444444444"),
                    archive_staging_receipt_id: staging_receipt_id.clone(),
                    expected_archive_staging_receipt_digest: digest(A),
                    handoff_lineage_digest: digest(B),
                    expected_observations: staging_observations,
                    expected_postcondition_digest: staging_postcondition,
                },
            ))
            .unwrap();

        let retention_subject = RecoverySubjectRef::retention_lease(retention_lease_id.clone());
        let (observe_lease_observations, observe_lease_postcondition) =
            expected_postcondition(vec![RecoveryExpectedObservation::new(
                RecoveryObservationKind::RetentionLease,
                retention_subject.clone(),
                digest(A),
            )])
            .unwrap();
        let observe_lease = runtime_action(RecoveryActionDigestRecordKind::ObserveRetentionLease(
            ObserveRetentionLeaseActionDigestRecord {
                action_kind: ObserveRetentionLeaseActionKind::Value,
                action_id: id("55555555-5555-4555-8555-555555555555"),
                retention_lease_id: retention_lease_id.clone(),
                retention_capability_row_id: CapabilityRowId::parse("retention.v1").unwrap(),
                expected_lease_state: RetentionLeaseExpectedState::Held,
                expected_observations: observe_lease_observations,
                expected_postcondition_digest: observe_lease_postcondition,
            },
        ))
        .unwrap();

        let (release_observations, release_postcondition) =
            expected_postcondition(vec![RecoveryExpectedObservation::new(
                RecoveryObservationKind::RetentionLease,
                retention_subject.clone(),
                digest(B),
            )])
            .unwrap();
        let release = runtime_action(RecoveryActionDigestRecordKind::ReleaseRetentionLease(
            ReleaseRetentionLeaseActionDigestRecord {
                action_kind: ReleaseRetentionLeaseActionKind::Value,
                action_id: id("66666666-6666-4666-8666-666666666666"),
                retention_lease_id: retention_lease_id.clone(),
                retention_acquire_receipt_id: id("88888888-8888-4888-8888-888888888888"),
                retention_capability_row_id: CapabilityRowId::parse("retention.v1").unwrap(),
                archive_staging_receipt_id: staging_receipt_id.clone(),
                expected_archive_staging_receipt_digest: digest(A),
                expected_release_receipt_id: release_receipt_id.clone(),
                expected_released: TrueLiteral,
                expected_observations: release_observations,
                expected_postcondition_digest: release_postcondition,
            },
        ))
        .unwrap();
        let release_observation = RecoveryObservation::matched_test_only(
            RecoveryObservationKind::RetentionLease,
            retention_subject,
            digest(B),
            digest(B),
        )
        .unwrap();
        let (release_action_id, release_action_digest) = {
            let (action_id, _, _, action_digest) = release.common();
            (action_id.clone(), action_digest.clone())
        };
        let release_receipt_digest = contract_digest(
            &RecoveryActionEffectReceiptDigestRecord {
                receipt_kind: RecoveryActionReceiptKind::Value,
                receipt_id: release_receipt_id.clone(),
                producer_action_id: release_action_id.clone(),
                producer_action_digest: release_action_digest.clone(),
                terminal_observation_digests: TerminalObservationDigests::new(vec![
                    release_observation.observation_digest().clone(),
                ])
                .unwrap(),
            },
            "test release receipt digest failed",
        )
        .unwrap();
        let exact_release = HandoffRetentionReleaseReceipt {
            retention_lease_id: retention_lease_id.clone(),
            release_action_id: release_action_id.clone(),
            release_action_digest: release_action_digest.clone(),
            release_receipt_id: release_receipt_id.clone(),
            release_receipt_digest: release_receipt_digest.clone(),
        };

        let finish_action = |expected_release: HandoffRetentionReleaseReceipt| {
            let expected_releases =
                HandoffRetentionReleaseReceipts::new(vec![expected_release]).unwrap();
            let expected_release_set_digest = contract_digest(
                &HandoffRetentionReleaseSetDigestRecord(expected_releases.clone()),
                "test release-set digest failed",
            )
            .unwrap();
            let (expected_observations, expected_postcondition_digest) =
                expected_postcondition(vec![RecoveryExpectedObservation::new(
                    RecoveryObservationKind::ArchivePresence,
                    RecoverySubjectRef::registered(archive_id.clone()),
                    digest(A),
                )])
                .unwrap();
            runtime_action(RecoveryActionDigestRecordKind::FinishArchive(
                FinishArchiveActionDigestRecord {
                    action_kind: FinishArchiveActionKind::Value,
                    action_id: id("99999999-9999-4999-8999-999999999999"),
                    archive_id: archive_id.clone(),
                    archive_staging_receipt_id: staging_receipt_id.clone(),
                    expected_archive_staging_receipt_digest: digest(A),
                    handoff_lineage_digest: digest(B),
                    retention_lease_ids: RecoveryUnicaIds::new(vec![retention_lease_id.clone()])
                        .unwrap(),
                    expected_releases,
                    expected_release_set_digest,
                    expected_observations,
                    expected_postcondition_digest,
                },
            ))
            .unwrap()
        };

        let prefix = vec![
            observe_staging.clone(),
            observe_lease.clone(),
            release.clone(),
        ];
        let exact_finish = finish_action(exact_release.clone());
        let mut exact_actions = prefix.clone();
        exact_actions.push(exact_finish.clone());
        let exact_action_json = serde_json::to_value(
            ArchiveRecoveryActions::from_actions(exact_actions.clone()).unwrap(),
        )
        .unwrap();
        assert!(schema_accepts::<ArchiveRecoveryActions>(&exact_action_json));
        let mut permuted_action_json = exact_action_json.as_array().unwrap().clone();
        permuted_action_json.swap(0, 3);
        assert!(!schema_accepts::<ArchiveRecoveryActions>(&json!(
            permuted_action_json
        )));
        let mut pair_permuted = exact_actions.clone();
        pair_permuted.swap(1, 2);
        // Pair adjacency is the second documented variable-length schema
        // superset; the sealed constructor remains the authority boundary.
        assert!(schema_accepts::<ArchiveRecoveryActions>(
            &serde_json::to_value(&pair_permuted).unwrap()
        ));
        assert!(ArchiveRecoveryActions::from_actions(pair_permuted).is_err());
        let held_plan = RecoveryActionPlan::test_only(
            RecoveryTarget::Archive,
            RecoveryEffectClass::Cleanup,
            exact_actions,
        )
        .unwrap();

        assert!(RecoveryActionOutcome::performed_test_only(
            &release,
            vec![release_observation.clone()],
            id("cccccccc-cccc-4ccc-8ccc-cccccccccccc"),
        )
        .is_err());
        let held_release_outcome = RecoveryActionOutcome::performed_test_only(
            &release,
            vec![release_observation.clone()],
            release_receipt_id.clone(),
        )
        .unwrap();
        let recovered_release_outcome = RecoveryActionOutcome::recovered_receipt_test_only(
            &release,
            vec![release_observation.clone()],
            release_receipt_id.clone(),
        )
        .unwrap();
        let projected_release =
            HandoffRetentionReleaseReceipt::from_release_outcome(&release, &held_release_outcome)
                .unwrap();
        assert_eq!(projected_release.retention_lease_id(), &retention_lease_id);
        assert_eq!(projected_release.release_action_id(), &release_action_id);
        assert_eq!(projected_release.release_receipt_id(), &release_receipt_id);
        assert_eq!(
            projected_release.release_receipt_digest(),
            &release_receipt_digest
        );
        let releases = HandoffRetentionReleaseReceipts::new(vec![projected_release]).unwrap();
        assert_eq!(releases.as_slice().len(), 1);
        assert_eq!(
            releases.release_receipt_digests(),
            vec![release_receipt_digest.clone()]
        );
        let staging_outcome = RecoveryActionOutcome::already_satisfied_test_only(
            &observe_staging,
            vec![RecoveryObservation::matched_test_only(
                RecoveryObservationKind::ArchiveStagingPresence,
                RecoverySubjectRef::registered(staging_receipt_id.clone()),
                digest(A),
                digest(A),
            )
            .unwrap()],
        )
        .unwrap();
        let lease_outcome = RecoveryActionOutcome::already_satisfied_test_only(
            &observe_lease,
            vec![RecoveryObservation::matched_test_only(
                RecoveryObservationKind::RetentionLease,
                RecoverySubjectRef::retention_lease(retention_lease_id.clone()),
                digest(A),
                digest(A),
            )
            .unwrap()],
        )
        .unwrap();
        let finish_outcome = RecoveryActionOutcome::performed_test_only(
            &exact_finish,
            vec![RecoveryObservation::matched_test_only(
                RecoveryObservationKind::ArchivePresence,
                RecoverySubjectRef::registered(archive_id.clone()),
                digest(A),
                digest(A),
            )
            .unwrap()],
            id("dddddddd-dddd-4ddd-8ddd-dddddddddddd"),
        )
        .unwrap();
        assert!(held_plan
            .validate_completed_outcomes(&[
                staging_outcome.clone(),
                lease_outcome.clone(),
                held_release_outcome.clone(),
                finish_outcome.clone(),
            ])
            .is_ok());
        assert!(held_plan
            .validate_completed_outcomes(&[
                staging_outcome.clone(),
                lease_outcome.clone(),
                recovered_release_outcome.clone(),
                finish_outcome.clone(),
            ])
            .is_err());

        let (released_observations, released_postcondition) =
            expected_postcondition(vec![RecoveryExpectedObservation::new(
                RecoveryObservationKind::RetentionLease,
                RecoverySubjectRef::retention_lease(retention_lease_id.clone()),
                digest(A),
            )])
            .unwrap();
        let observe_released =
            runtime_action(RecoveryActionDigestRecordKind::ObserveRetentionLease(
                ObserveRetentionLeaseActionDigestRecord {
                    action_kind: ObserveRetentionLeaseActionKind::Value,
                    action_id: id("55555555-5555-4555-8555-555555555555"),
                    retention_lease_id: retention_lease_id.clone(),
                    retention_capability_row_id: CapabilityRowId::parse("retention.v1").unwrap(),
                    expected_lease_state: RetentionLeaseExpectedState::Released,
                    expected_observations: released_observations,
                    expected_postcondition_digest: released_postcondition,
                },
            ))
            .unwrap();
        assert!(RecoveryActionPlan::test_only(
            RecoveryTarget::Archive,
            RecoveryEffectClass::Cleanup,
            vec![
                observe_staging.clone(),
                observe_released.clone(),
                exact_finish.clone(),
            ],
        )
        .is_err());
        let released_plan = RecoveryActionPlan::test_only(
            RecoveryTarget::Archive,
            RecoveryEffectClass::Cleanup,
            vec![
                observe_staging,
                observe_released.clone(),
                release,
                exact_finish,
            ],
        )
        .unwrap();
        let released_observe_outcome = RecoveryActionOutcome::already_satisfied_test_only(
            &observe_released,
            vec![RecoveryObservation::matched_test_only(
                RecoveryObservationKind::RetentionLease,
                RecoverySubjectRef::retention_lease(retention_lease_id.clone()),
                digest(A),
                digest(A),
            )
            .unwrap()],
        )
        .unwrap();
        assert!(released_plan
            .validate_completed_outcomes(&[
                staging_outcome.clone(),
                released_observe_outcome.clone(),
                recovered_release_outcome,
                finish_outcome.clone(),
            ])
            .is_ok());
        assert!(released_plan
            .validate_completed_outcomes(&[
                staging_outcome,
                released_observe_outcome,
                held_release_outcome,
                finish_outcome,
            ])
            .is_err());

        let mut wrong_action_id = exact_release.clone();
        wrong_action_id.release_action_id = id("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa");
        let mut wrong_action_digest = exact_release.clone();
        wrong_action_digest.release_action_digest = if release_action_digest == digest(A) {
            digest(B)
        } else {
            digest(A)
        };
        let mut wrong_receipt_id = exact_release.clone();
        wrong_receipt_id.release_receipt_id = id("bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb");
        let mut wrong_receipt_digest = exact_release;
        wrong_receipt_digest.release_receipt_digest = if release_receipt_digest == digest(A) {
            digest(B)
        } else {
            digest(A)
        };
        for substituted in [
            wrong_action_id,
            wrong_action_digest,
            wrong_receipt_id,
            wrong_receipt_digest,
        ] {
            let mut actions = prefix.clone();
            actions.push(finish_action(substituted));
            assert!(RecoveryActionPlan::test_only(
                RecoveryTarget::Archive,
                RecoveryEffectClass::Cleanup,
                actions,
            )
            .is_err());
        }
    }

    #[test]
    fn finish_cleanup_cannot_encode_an_empty_owned_target_recovery() {
        assert!(RecoveryOwnedTargets::new(Vec::new()).is_err());
        assert!(!schema_accepts::<RecoveryOwnedTargets>(&json!([])));
        assert!(RecoveryActionPlan::test_only(
            RecoveryTarget::Cleanup,
            RecoveryEffectClass::Cleanup,
            Vec::new(),
        )
        .is_err());
    }

    #[test]
    fn recovery_owned_targets_use_typed_locator_order_and_reject_reorder_or_duplicate() {
        let project = ProjectId::parse("10000000-0000-4000-8000-000000000000").unwrap();
        let instance = id("f0000000-0000-4000-8000-000000000000");
        let root = OwnedTargetLocator::new(
            project.clone(),
            instance.clone(),
            OwnedTargetRole::InstanceRoot,
        );
        let artifact = OwnedTargetLocator::new(project, instance, OwnedTargetRole::Artifact);

        assert!(RecoveryOwnedTargets::new(vec![root.clone(), artifact.clone()]).is_ok());
        assert!(RecoveryOwnedTargets::new(vec![artifact.clone(), root.clone()]).is_err());
        assert!(RecoveryOwnedTargets::new(vec![root.clone(), root.clone()]).is_err());

        let observations = vec![
            RecoveryObservation::matched_test_only(
                RecoveryObservationKind::OwnedTargetAbsence,
                RecoverySubjectRef::owned_role(root),
                digest(A),
                digest(A),
            )
            .unwrap(),
            RecoveryObservation::matched_test_only(
                RecoveryObservationKind::OwnedTargetAbsence,
                RecoverySubjectRef::owned_role(artifact),
                digest(B),
                digest(B),
            )
            .unwrap(),
        ];
        assert!(validated_plan_observations(observations.clone()).is_ok());
        let mut reordered = observations.clone();
        reordered.reverse();
        assert!(validated_plan_observations(reordered).is_err());
        assert!(validated_plan_observations(vec![
            observations[0].clone(),
            observations[0].clone(),
        ])
        .is_err());
    }

    #[test]
    fn all_typed_owned_targets_retain_order_through_cleanup_grammar_and_absences() {
        let targets = typed_cleanup_targets();
        let plan = RecoveryPlanStatus::cleanup_targets_fixture_test_only(
            OperationId::parse("22222222-2222-4222-8222-222222222222").unwrap(),
            id("33333333-3333-4333-8333-333333333333"),
            targets.clone(),
            TaskPhase::CleanedSuccess,
        )
        .unwrap();

        let binding = plan.cleanup_binding().unwrap();
        assert_eq!(binding.owned_targets(), targets.as_slice());

        let absences = plan
            .cleanup_matching_absence_observations_test_only()
            .unwrap();
        assert_eq!(absences.as_slice().len(), targets.len());
        assert!(absences
            .as_slice()
            .iter()
            .zip(&targets)
            .all(|(observation, target)| observation.owned_target() == target));

        let RecoveryPlanStatusKind::Cleanup(cleanup) = &plan.0 else {
            panic!("fixture must be cleanup");
        };
        let (finish, resumes) = cleanup.record.actions.as_slice().split_last().unwrap();
        assert!(resumes
            .iter()
            .zip(&targets)
            .all(|(action, target)| matches!(
                &action.0,
                RecoveryActionKindWire::ResumeOwnedTargetQuarantine(resume)
                    if &resume.owned_target == target
            )));
        let RecoveryActionKindWire::FinishCleanup(finish) = &finish.0 else {
            panic!("fixture must finish cleanup last");
        };
        assert_eq!(finish.owned_targets.0, targets);
        assert!(schema_accepts::<RecoveryPlanStatus>(
            &serde_json::to_value(plan).unwrap()
        ));
    }

    #[test]
    fn a_quarantined_observation_cannot_masquerade_as_cleanup_absence() {
        let target: OwnedTargetLocator = serde_json::from_value(json!({
            "projectId": ID_1,
            "instanceId": ID_1,
            "role": "quarantine"
        }))
        .unwrap();
        let plan = RecoveryPlanStatus::cleanup_fixture_test_only(
            OperationId::parse("22222222-2222-4222-8222-222222222222").unwrap(),
            id("33333333-3333-4333-8333-333333333333"),
            target.clone(),
            TaskPhase::CleanedSuccess,
        )
        .unwrap();
        let RecoveryPlanStatusKind::Cleanup(plan) = &plan.0 else {
            panic!("fixture must be cleanup");
        };
        let [resume, finish] = plan.record.actions.as_slice() else {
            panic!("fixture must retain resume and finish actions");
        };
        let RecoveryActionKindWire::ResumeOwnedTargetQuarantine(resume_body) = &resume.0 else {
            panic!("fixture must resume quarantine first");
        };
        let quarantined_digest = resume_body.expected_quarantined_digest.clone();
        let quarantined = RecoveryObservation::matched_test_only(
            RecoveryObservationKind::QuarantinePresence,
            RecoverySubjectRef::owned_role(target.clone()),
            quarantined_digest.clone(),
            quarantined_digest,
        )
        .unwrap();
        assert!(finish
            .match_finish_cleanup_absence(&target, &quarantined)
            .is_err());

        let RecoveryActionKindWire::FinishCleanup(finish_body) = &finish.0 else {
            panic!("fixture must finish cleanup last");
        };
        let absent_digest = finish_body.expected_observations.as_slice()[0]
            .expected_digest
            .clone();
        let wrong_kind = RecoveryObservation::matched_test_only(
            RecoveryObservationKind::QuarantinePresence,
            RecoverySubjectRef::owned_role(target.clone()),
            absent_digest.clone(),
            absent_digest.clone(),
        )
        .unwrap();
        assert!(finish
            .match_finish_cleanup_absence(&target, &wrong_kind)
            .is_err());
        let absent = RecoveryObservation::matched_test_only(
            RecoveryObservationKind::OwnedTargetAbsence,
            RecoverySubjectRef::owned_role(target.clone()),
            absent_digest.clone(),
            absent_digest,
        )
        .unwrap();
        let authority = finish
            .match_finish_cleanup_absence(&target, &absent)
            .unwrap();
        assert_eq!(authority.owned_target(), &target);
        assert_eq!(authority.observation_digest(), absent.observation_digest());
    }
}
