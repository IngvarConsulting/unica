use super::artifacts::OwnedTargetLocator;
use super::prearm_recovery::{
    PreArmCancellationEffectKind, PreArmCancellationEffectReceipt, PreArmCancellationReceiptRef,
    PreArmCancellationReceiptSource,
};
use super::repository::RepositoryHistoryCursor;
use super::schema::one_of_schema;
use super::support::ManualWorkingInfobaseIdentity;
use crate::domain::branched_development::canonical_json::{
    canonical_contract_digest, contract_digest_record_sealed, ContractDigestRecord,
};
use crate::domain::branched_development::{
    CapabilityRowId, MetadataObjectId, OperationId, Sha256Digest, TaskPhase, UnicaId,
};
use schemars::{json_schema, JsonSchema, Schema, SchemaGenerator};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::fmt;

const MAX_RECOVERY_ITEMS: usize = 100_000;

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

    fn canonical_key(&self) -> Result<Vec<u8>, RecoveryContractError> {
        serde_json_canonicalizer::to_vec(self)
            .map_err(|_| RecoveryContractError("recovery subject canonicalization failed"))
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

    fn canonical_key(&self) -> Result<(RecoveryObservationKind, Vec<u8>), RecoveryContractError> {
        Ok((self.observation_kind, self.subject.canonical_key()?))
    }
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

    fn observation_digest(&self) -> &Sha256Digest {
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
struct HandoffRetentionReleaseReceipts(Vec<HandoffRetentionReleaseReceipt>);

impl HandoffRetentionReleaseReceipts {
    fn new(values: Vec<HandoffRetentionReleaseReceipt>) -> Result<Self, RecoveryContractError> {
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
        let mut previous = None;
        for value in &values {
            let key = serde_json_canonicalizer::to_vec(value)
                .map_err(|_| RecoveryContractError("owned target canonicalization failed"))?;
            if previous.as_ref().is_some_and(|previous| previous >= &key) {
                return Err(RecoveryContractError(
                    "recovery owned targets must be canonical and unique",
                ));
            }
            previous = Some(key);
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
    let anchor = recovery_expected_observation_schema(
        generator,
        "repositoryAnchor",
        RecoveryObservationSubjectSchema::Registered,
    );
    let version = recovery_expected_observation_schema(
        generator,
        "repositoryVersion",
        RecoveryObservationSubjectSchema::Registered,
    );
    json_schema!({
        "type": "array",
        "prefixItems": [anchor],
        "items": version,
        "minItems": 1,
        "maxItems": MAX_RECOVERY_ITEMS,
        "uniqueItems": true
    })
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
                allowed(generator, &[("quarantinePresence", Subject::OwnedRole)])
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
            let Some((anchor, versions)) = value.expected_observations.as_slice().split_first()
            else {
                return Err(RecoveryContractError(
                    "support history requires its repository anchor",
                ));
            };
            if anchor.observation_kind != RecoveryObservationKind::RepositoryAnchor
                || !matches!(anchor.subject.0, RecoverySubjectRefKind::Registered(_))
                || versions.iter().any(|observation| {
                    observation.observation_kind != RecoveryObservationKind::RepositoryVersion
                        || !matches!(observation.subject.0, RecoverySubjectRefKind::Registered(_))
                })
            {
                return Err(RecoveryContractError(
                    "support history requires one anchor followed only by versions",
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
            if value.expected_observations.as_slice().len() != value.owned_targets.0.len()
                || value
                    .expected_observations
                    .as_slice()
                    .iter()
                    .zip(&value.owned_targets.0)
                    .any(|(observation, target)| {
                        observation.observation_kind != RecoveryObservationKind::QuarantinePresence
                            || !observation.subject.is_owned_role(target)
                    })
            {
                return Err(RecoveryContractError(
                    "finish cleanup observation/owned-target projection mismatch",
                ));
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
    #[cfg(test)]
    fn test_only(
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::branched_development::contracts::prearm_recovery::PreArmCancellationEffectReceiptAuthority;
    use crate::domain::branched_development::contracts::schema::audit_json_schema;
    use schemars::{schema_for, JsonSchema};
    use serde::de::DeserializeOwned;
    use serde_json::{json, Value};

    const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const ID_1: &str = "11111111-1111-4111-8111-111111111111";

    fn digest(value: &str) -> Sha256Digest {
        Sha256Digest::parse(value).unwrap()
    }

    fn id(value: &str) -> UnicaId {
        UnicaId::parse(value).unwrap()
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

    #[test]
    fn recovery_subject_contract_exists() {
        let _ = std::mem::size_of::<RecoverySubjectRef>();
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
    fn support_history_requires_one_anchor_prefix_followed_only_by_versions() {
        let anchor = RecoveryExpectedObservation::new(
            RecoveryObservationKind::RepositoryAnchor,
            RecoverySubjectRef::registered(id(ID_1)),
            digest(A),
        );
        let version = RecoveryExpectedObservation::new(
            RecoveryObservationKind::RepositoryVersion,
            RecoverySubjectRef::registered(id("22222222-2222-4222-8222-222222222222")),
            digest(B),
        );
        let valid = observe_history_action(vec![anchor.clone(), version.clone()]).unwrap();
        assert!(observe_history_action(vec![anchor.clone()]).is_ok());

        let duplicate_anchor = RecoveryExpectedObservation::new(
            RecoveryObservationKind::RepositoryAnchor,
            RecoverySubjectRef::registered(id("33333333-3333-4333-8333-333333333333")),
            digest(A),
        );
        assert!(
            observe_history_action(vec![anchor.clone(), duplicate_anchor, version.clone(),])
                .is_err()
        );
        assert!(observe_history_action(vec![version.clone()]).is_err());
        assert!(observe_history_action(vec![version.clone(), anchor.clone()]).is_err());
        assert!(
            observe_history_action(vec![RecoveryExpectedObservation::new(
                RecoveryObservationKind::RepositoryAnchor,
                RecoverySubjectRef::configuration_root(),
                digest(A),
            )])
            .is_err()
        );
        assert!(observe_history_action(vec![
            anchor.clone(),
            RecoveryExpectedObservation::new(
                RecoveryObservationKind::RepositoryVersion,
                RecoverySubjectRef::configuration_root(),
                digest(B),
            ),
        ])
        .is_err());

        let action_schema = schema::<RecoveryAction>();
        let validator = jsonschema::options()
            .with_draft(jsonschema::Draft::Draft202012)
            .build(&action_schema)
            .unwrap();
        let valid = serde_json::to_value(valid).unwrap();
        assert!(validator.is_valid(&valid));

        let mut duplicate = valid.clone();
        duplicate["expectedObservations"]
            .as_array_mut()
            .unwrap()
            .insert(1, serde_json::to_value(anchor.clone()).unwrap());
        assert!(!validator.is_valid(&duplicate));

        let mut missing = valid.clone();
        missing["expectedObservations"]
            .as_array_mut()
            .unwrap()
            .remove(0);
        assert!(!validator.is_valid(&missing));

        let mut anchor_after_version = valid;
        anchor_after_version["expectedObservations"]
            .as_array_mut()
            .unwrap()
            .swap(0, 1);
        assert!(!validator.is_valid(&anchor_after_version));
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
    fn archive_grammar_binds_release_action_and_receipt_lineage_to_finish_archive() {
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
}
