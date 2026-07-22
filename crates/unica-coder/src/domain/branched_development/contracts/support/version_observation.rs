use super::{
    ExternalSupportOwnershipEvidence, ManualSupportTargetMode, ManualWorkingInfobaseIdentity,
    SupportMissingEvidenceKind, SupportPrerequisiteMismatchKind,
};
use crate::domain::branched_development::canonical_json::{
    canonical_contract_digest, contract_digest_record_sealed, ContractDigestRecord,
};
use crate::domain::branched_development::contracts::repository::{
    CanonicalEmptyDeltaDigest, RepositoryActorIdentity, RepositoryHistoryPartitionClassification,
    RepositoryRelevance,
};
use crate::domain::branched_development::contracts::scalars::{
    RepositoryVersion, RequiredNullable,
};
use crate::domain::branched_development::contracts::schema::one_of_schema;
use crate::domain::branched_development::{Sha256Digest, UnicaId};
use schemars::{json_schema, JsonSchema, Schema, SchemaGenerator};
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};
use std::borrow::Cow;

macro_rules! bool_literal {
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

macro_rules! string_literal {
    ($name:ident, $wire:literal) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
        enum $name {
            #[serde(rename = $wire)]
            Value,
        }
    };
}

bool_literal!(TrueLiteral, true);
string_literal!(RoutineClassification, "routine");
string_literal!(AuthorizedClassification, "authorized");
string_literal!(ExternalSupportClassification, "externalSupport");
string_literal!(PreArmExternalClassification, "preArmExternal");
string_literal!(CorrectiveClassification, "corrective");
string_literal!(InvalidClassification, "invalid");
string_literal!(ReservedOriginalMode, "reservedOriginal");
string_literal!(SeparateWorkingInfobaseMode, "separateWorkingInfobase");
string_literal!(AwaitingArmState, "awaitingArm");
string_literal!(FrozenForRecoveryState, "frozenForRecovery");
string_literal!(
    PreArmCancellationEffectFreezeKind,
    "preArmCancellationEffect"
);
string_literal!(ActionCorrectionKind, "actionCorrection");
string_literal!(ExternalConflictCorrectionKind, "externalConflictCorrection");
string_literal!(ThisAuthorizedActionProvenance, "thisAuthorizedAction");
string_literal!(ExternalActorProvenance, "externalActor");
string_literal!(UnattributedProvenance, "unattributed");
string_literal!(ArmingOrderViolatedMismatch, "armingOrderViolated");

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(transparent)]
struct EmptyMismatchKinds([(); 0]);

impl<'de> Deserialize<'de> for EmptyMismatchKinds {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let values = Vec::<serde::de::IgnoredAny>::deserialize(deserializer)?;
        values
            .is_empty()
            .then_some(Self([]))
            .ok_or_else(|| D::Error::custom("positive observation mismatchKinds must be empty"))
    }
}

impl JsonSchema for EmptyMismatchKinds {
    fn schema_name() -> Cow<'static, str> {
        "EmptySupportPrerequisiteMismatchKinds".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "array",
            "items": { "type": "string" },
            "minItems": 0,
            "maxItems": 0,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
struct ArmingOrderViolationMismatchKinds([ArmingOrderViolatedMismatch; 1]);

impl JsonSchema for ArmingOrderViolationMismatchKinds {
    fn schema_name() -> Cow<'static, str> {
        "ArmingOrderViolationMismatchKinds".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "array",
            "prefixItems": [{ "type": "string", "const": "armingOrderViolated" }],
            "items": false,
            "minItems": 1,
            "maxItems": 1,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
struct InvalidMismatchKinds(Vec<SupportPrerequisiteMismatchKind>);

impl InvalidMismatchKinds {
    fn contains(&self, expected: SupportPrerequisiteMismatchKind) -> bool {
        self.0.contains(&expected)
    }
}

impl<'de> Deserialize<'de> for InvalidMismatchKinds {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let values = Vec::<SupportPrerequisiteMismatchKind>::deserialize(deserializer)?;
        if values.is_empty() || values.len() > 15 {
            return Err(D::Error::custom(
                "invalid observation mismatchKinds must be non-empty and bounded",
            ));
        }
        if values.windows(2).any(|pair| pair[0] >= pair[1]) {
            return Err(D::Error::custom(
                "mismatchKinds must be unique and in declaration order",
            ));
        }
        Ok(Self(values))
    }
}

impl JsonSchema for InvalidMismatchKinds {
    fn schema_name() -> Cow<'static, str> {
        "InvalidSupportPrerequisiteMismatchKinds".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "array",
            "items": generator.subschema_for::<SupportPrerequisiteMismatchKind>(),
            "minItems": 1,
            "maxItems": 15,
            "uniqueItems": true,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
struct UnattributedMissingEvidenceKinds(Vec<SupportMissingEvidenceKind>);

impl<'de> Deserialize<'de> for UnattributedMissingEvidenceKinds {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let values = Vec::<SupportMissingEvidenceKind>::deserialize(deserializer)?;
        if values.is_empty() || values.len() > SupportMissingEvidenceKind::ALL.len() {
            return Err(D::Error::custom(
                "unattributed missingEvidenceKinds must be non-empty and bounded",
            ));
        }
        let allowed = |value: SupportMissingEvidenceKind| {
            matches!(
                value,
                SupportMissingEvidenceKind::RepositoryActorUnavailable
                    | SupportMissingEvidenceKind::ManualTargetModeUnavailable
                    | SupportMissingEvidenceKind::WorkingInfobaseIdentityUnavailable
                    | SupportMissingEvidenceKind::RootDeltaUnavailable
                    | SupportMissingEvidenceKind::ContentDeltaUnavailable
                    | SupportMissingEvidenceKind::OwnershipEvidenceUnavailable
                    | SupportMissingEvidenceKind::SupportLayerIdentityUnavailable
                    | SupportMissingEvidenceKind::RepositoryHistoryCoverageIncomplete
            )
        };
        if values.iter().copied().any(|value| !allowed(value)) {
            return Err(D::Error::custom(
                "unattributed observation contains an inapplicable evidence kind",
            ));
        }
        let positions: Vec<_> = values
            .iter()
            .map(|value| {
                SupportMissingEvidenceKind::ALL
                    .iter()
                    .position(|candidate| candidate == value)
                    .expect("closed evidence kind is registered")
            })
            .collect();
        if positions.windows(2).any(|pair| pair[0] >= pair[1]) {
            return Err(D::Error::custom(
                "missingEvidenceKinds must be unique and in declaration order",
            ));
        }
        Ok(Self(values))
    }
}

impl JsonSchema for UnattributedMissingEvidenceKinds {
    fn schema_name() -> Cow<'static, str> {
        "UnattributedSupportMissingEvidenceKinds".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "array",
            "items": {
                "type": "string",
                "enum": [
                    "repositoryActorUnavailable",
                    "manualTargetModeUnavailable",
                    "workingInfobaseIdentityUnavailable",
                    "rootDeltaUnavailable",
                    "contentDeltaUnavailable",
                    "ownershipEvidenceUnavailable",
                    "supportLayerIdentityUnavailable",
                    "repositoryHistoryCoverageIncomplete",
                ],
            },
            "minItems": 1,
            "maxItems": 8,
            "uniqueItems": true,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
struct RequiredNullRepositoryActor(RequiredNullable<RepositoryActorIdentity>);

impl RequiredNullRepositoryActor {
    fn deserialize_required<'de, D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value =
            RequiredNullable::<RepositoryActorIdentity>::deserialize_required(deserializer)?;
        value
            .as_ref()
            .is_none()
            .then_some(Self(value))
            .ok_or_else(|| D::Error::custom("unattributed repositoryActor must be explicit null"))
    }
}

impl<'de> Deserialize<'de> for RequiredNullRepositoryActor {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::deserialize_required(deserializer)
    }
}

impl JsonSchema for RequiredNullRepositoryActor {
    fn schema_name() -> Cow<'static, str> {
        "RequiredNullRepositoryActor".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({ "type": "null" })
    }
}

macro_rules! observation_leaf {
    (
        $record:ident, $leaf:ident,
        $classification:ty, $mismatch:ty,
        { $( $(#[$attribute:meta])* $field:ident : $field_type:ty ),* $(,)? }
    ) => {
        #[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct $record {
            repository_version: RepositoryVersion,
            classification: $classification,
            mismatch_kinds: $mismatch,
            $( $(#[$attribute])* $field: $field_type, )*
        }

        #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct $leaf {
            repository_version: RepositoryVersion,
            classification: $classification,
            classification_digest: Sha256Digest,
            mismatch_kinds: $mismatch,
            $( $(#[$attribute])* $field: $field_type, )*
        }

        impl $leaf {
            fn digest_record(&self) -> $record {
                $record {
                    repository_version: self.repository_version.clone(),
                    classification: self.classification,
                    mismatch_kinds: self.mismatch_kinds.clone(),
                    $( $field: self.$field.clone(), )*
                }
            }
        }
    };
}

observation_leaf!(
    RoutineObservationDigestRecord,
    RoutineObservation,
    RoutineClassification,
    EmptyMismatchKinds,
    {
        repository_actor: RepositoryActorIdentity,
        relevance: RepositoryRelevance,
        root_delta_digest: Sha256Digest,
        content_delta_digest: Sha256Digest,
        support_transitions_digest: CanonicalEmptyDeltaDigest,
        support_graph_unchanged: TrueLiteral,
    }
);

observation_leaf!(
    AuthorizedReservedObservationDigestRecord,
    AuthorizedReservedObservation,
    AuthorizedClassification,
    EmptyMismatchKinds,
    {
        repository_actor: RepositoryActorIdentity,
        support_action_id: UnicaId,
        support_action_digest: Sha256Digest,
        arming_receipt_id: UnicaId,
        arming_receipt_digest: Sha256Digest,
        first_root_support_after_arming: TrueLiteral,
        action_attribution_evidence_digest: Sha256Digest,
        authorized_transitions_digest: Sha256Digest,
        manual_target_mode: ReservedOriginalMode,
        root_delta_digest: Sha256Digest,
        content_delta_digest: CanonicalEmptyDeltaDigest,
        observed_support_transitions_digest: Sha256Digest,
        root_delta_contains_only_authorized_support_transitions: TrueLiteral,
    }
);

observation_leaf!(
    AuthorizedSeparateObservationDigestRecord,
    AuthorizedSeparateObservation,
    AuthorizedClassification,
    EmptyMismatchKinds,
    {
        repository_actor: RepositoryActorIdentity,
        support_action_id: UnicaId,
        support_action_digest: Sha256Digest,
        arming_receipt_id: UnicaId,
        arming_receipt_digest: Sha256Digest,
        first_root_support_after_arming: TrueLiteral,
        action_attribution_evidence_digest: Sha256Digest,
        authorized_transitions_digest: Sha256Digest,
        manual_target_mode: SeparateWorkingInfobaseMode,
        working_infobase_identity: ManualWorkingInfobaseIdentity,
        root_delta_digest: Sha256Digest,
        content_delta_digest: CanonicalEmptyDeltaDigest,
        observed_support_transitions_digest: Sha256Digest,
        root_delta_contains_only_authorized_support_transitions: TrueLiteral,
    }
);

observation_leaf!(
    ExternalSupportObservationDigestRecord,
    ExternalSupportObservation,
    ExternalSupportClassification,
    EmptyMismatchKinds,
    {
        repository_actor: RepositoryActorIdentity,
        root_delta_digest: Sha256Digest,
        content_delta_digest: CanonicalEmptyDeltaDigest,
        proven_not_this_action: TrueLiteral,
        overlap_with_authorized_transitions: FalseLiteral,
        support_only_delta: TrueLiteral,
        external_support_disjointness_digest: Sha256Digest,
        external_ownership_evidence: ExternalSupportOwnershipEvidence,
    }
);

bool_literal!(FalseLiteral, false);

observation_leaf!(
    PreArmAwaitingObservationDigestRecord,
    PreArmAwaitingObservation,
    PreArmExternalClassification,
    ArmingOrderViolationMismatchKinds,
    {
        pending_support_action_id: UnicaId,
        pending_support_action_digest: Sha256Digest,
        authorization_state: AwaitingArmState,
        arming_receipt_absent: TrueLiteral,
        repository_actor: RepositoryActorIdentity,
        root_delta_digest: Sha256Digest,
        content_delta_digest: Sha256Digest,
        support_transitions_digest: Sha256Digest,
        preserve_as_external_baseline: TrueLiteral,
    }
);

observation_leaf!(
    PreArmFrozenObservationDigestRecord,
    PreArmFrozenObservation,
    PreArmExternalClassification,
    ArmingOrderViolationMismatchKinds,
    {
        pending_support_action_id: UnicaId,
        pending_support_action_digest: Sha256Digest,
        authorization_state: FrozenForRecoveryState,
        freeze_kind: PreArmCancellationEffectFreezeKind,
        pre_arm_freeze_digest: Sha256Digest,
        arming_receipt_absent: TrueLiteral,
        repository_actor: RepositoryActorIdentity,
        root_delta_digest: Sha256Digest,
        content_delta_digest: Sha256Digest,
        support_transitions_digest: Sha256Digest,
        preserve_as_external_baseline: TrueLiteral,
    }
);

observation_leaf!(
    ActionCorrectionReservedObservationDigestRecord,
    ActionCorrectionReservedObservation,
    CorrectiveClassification,
    EmptyMismatchKinds,
    {
        correction_kind: ActionCorrectionKind,
        repository_actor: RepositoryActorIdentity,
        manual_target_mode: ReservedOriginalMode,
        root_delta_digest: Sha256Digest,
        content_delta_digest: Sha256Digest,
        corrective_instruction_digest: Sha256Digest,
    }
);

observation_leaf!(
    ActionCorrectionSeparateObservationDigestRecord,
    ActionCorrectionSeparateObservation,
    CorrectiveClassification,
    EmptyMismatchKinds,
    {
        correction_kind: ActionCorrectionKind,
        repository_actor: RepositoryActorIdentity,
        manual_target_mode: SeparateWorkingInfobaseMode,
        working_infobase_identity: ManualWorkingInfobaseIdentity,
        root_delta_digest: Sha256Digest,
        content_delta_digest: Sha256Digest,
        corrective_instruction_digest: Sha256Digest,
    }
);

observation_leaf!(
    ExternalConflictCorrectionObservationDigestRecord,
    ExternalConflictCorrectionObservation,
    CorrectiveClassification,
    EmptyMismatchKinds,
    {
        correction_kind: ExternalConflictCorrectionKind,
        repository_actor: RepositoryActorIdentity,
        root_delta_digest: Sha256Digest,
        content_delta_digest: Sha256Digest,
        conflict_resolution_id: UnicaId,
        support_conflict_instruction_digest: Sha256Digest,
        final_baseline_digest: Sha256Digest,
        external_ownership_evidence: ExternalSupportOwnershipEvidence,
    }
);

observation_leaf!(
    InvalidThisActionReservedObservationDigestRecord,
    InvalidThisActionReservedObservation,
    InvalidClassification,
    InvalidMismatchKinds,
    {
        provenance: ThisAuthorizedActionProvenance,
        repository_actor: RepositoryActorIdentity,
        manual_target_mode: ReservedOriginalMode,
        arming_receipt_id: UnicaId,
        arming_receipt_digest: Sha256Digest,
        first_root_support_after_arming: bool,
        root_delta_digest: Sha256Digest,
        content_delta_digest: Sha256Digest,
        action_attribution_evidence_digest: Sha256Digest,
    }
);

observation_leaf!(
    InvalidThisActionSeparateObservationDigestRecord,
    InvalidThisActionSeparateObservation,
    InvalidClassification,
    InvalidMismatchKinds,
    {
        provenance: ThisAuthorizedActionProvenance,
        repository_actor: RepositoryActorIdentity,
        manual_target_mode: SeparateWorkingInfobaseMode,
        working_infobase_identity: ManualWorkingInfobaseIdentity,
        arming_receipt_id: UnicaId,
        arming_receipt_digest: Sha256Digest,
        first_root_support_after_arming: bool,
        root_delta_digest: Sha256Digest,
        content_delta_digest: Sha256Digest,
        action_attribution_evidence_digest: Sha256Digest,
    }
);

observation_leaf!(
    InvalidExternalActorObservationDigestRecord,
    InvalidExternalActorObservation,
    InvalidClassification,
    InvalidMismatchKinds,
    {
        provenance: ExternalActorProvenance,
        repository_actor: RepositoryActorIdentity,
        #[serde(deserialize_with = "RequiredNullable::deserialize_required")]
        observed_working_infobase_identity: RequiredNullable<ManualWorkingInfobaseIdentity>,
        root_delta_digest: Sha256Digest,
        content_delta_digest: Sha256Digest,
        proven_not_this_action: TrueLiteral,
        external_ownership_evidence: ExternalSupportOwnershipEvidence,
    }
);

observation_leaf!(
    InvalidUnattributedObservationDigestRecord,
    InvalidUnattributedObservation,
    InvalidClassification,
    InvalidMismatchKinds,
    {
        provenance: UnattributedProvenance,
        #[serde(deserialize_with = "RequiredNullRepositoryActor::deserialize_required")]
        repository_actor: RequiredNullRepositoryActor,
        #[serde(deserialize_with = "RequiredNullable::deserialize_required")]
        root_delta_digest: RequiredNullable<Sha256Digest>,
        #[serde(deserialize_with = "RequiredNullable::deserialize_required")]
        content_delta_digest: RequiredNullable<Sha256Digest>,
        missing_evidence_kinds: UnattributedMissingEvidenceKinds,
    }
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
enum ObservationWire {
    Routine(RoutineObservation),
    AuthorizedReserved(AuthorizedReservedObservation),
    AuthorizedSeparate(AuthorizedSeparateObservation),
    ExternalSupport(ExternalSupportObservation),
    PreArmAwaiting(PreArmAwaitingObservation),
    PreArmFrozen(PreArmFrozenObservation),
    ActionCorrectionReserved(ActionCorrectionReservedObservation),
    ActionCorrectionSeparate(ActionCorrectionSeparateObservation),
    ExternalConflictCorrection(ExternalConflictCorrectionObservation),
    InvalidThisActionReserved(InvalidThisActionReservedObservation),
    InvalidThisActionSeparate(InvalidThisActionSeparateObservation),
    InvalidExternalActor(InvalidExternalActorObservation),
    InvalidUnattributed(InvalidUnattributedObservation),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(untagged)]
enum ObservationDigestRecordWire {
    Routine(RoutineObservationDigestRecord),
    AuthorizedReserved(AuthorizedReservedObservationDigestRecord),
    AuthorizedSeparate(AuthorizedSeparateObservationDigestRecord),
    ExternalSupport(ExternalSupportObservationDigestRecord),
    PreArmAwaiting(PreArmAwaitingObservationDigestRecord),
    PreArmFrozen(PreArmFrozenObservationDigestRecord),
    ActionCorrectionReserved(ActionCorrectionReservedObservationDigestRecord),
    ActionCorrectionSeparate(ActionCorrectionSeparateObservationDigestRecord),
    ExternalConflictCorrection(ExternalConflictCorrectionObservationDigestRecord),
    InvalidThisActionReserved(InvalidThisActionReservedObservationDigestRecord),
    InvalidThisActionSeparate(InvalidThisActionSeparateObservationDigestRecord),
    InvalidExternalActor(InvalidExternalActorObservationDigestRecord),
    InvalidUnattributed(InvalidUnattributedObservationDigestRecord),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct SupportPrerequisiteVersionObservationDigestRecord(ObservationDigestRecordWire);

impl JsonSchema for SupportPrerequisiteVersionObservationDigestRecord {
    fn schema_name() -> Cow<'static, str> {
        "SupportPrerequisiteVersionObservationDigestRecord".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<RoutineObservationDigestRecord>(),
            generator.subschema_for::<AuthorizedReservedObservationDigestRecord>(),
            generator.subschema_for::<AuthorizedSeparateObservationDigestRecord>(),
            generator.subschema_for::<ExternalSupportObservationDigestRecord>(),
            generator.subschema_for::<PreArmAwaitingObservationDigestRecord>(),
            generator.subschema_for::<PreArmFrozenObservationDigestRecord>(),
            generator.subschema_for::<ActionCorrectionReservedObservationDigestRecord>(),
            generator.subschema_for::<ActionCorrectionSeparateObservationDigestRecord>(),
            generator.subschema_for::<ExternalConflictCorrectionObservationDigestRecord>(),
            generator.subschema_for::<InvalidThisActionReservedObservationDigestRecord>(),
            generator.subschema_for::<InvalidThisActionSeparateObservationDigestRecord>(),
            generator.subschema_for::<InvalidExternalActorObservationDigestRecord>(),
            generator.subschema_for::<InvalidUnattributedObservationDigestRecord>(),
        ])
    }
}

impl contract_digest_record_sealed::Sealed for SupportPrerequisiteVersionObservationDigestRecord {}
impl ContractDigestRecord for SupportPrerequisiteVersionObservationDigestRecord {}

impl ObservationWire {
    fn digest_record(&self) -> SupportPrerequisiteVersionObservationDigestRecord {
        let record = match self {
            Self::Routine(value) => ObservationDigestRecordWire::Routine(value.digest_record()),
            Self::AuthorizedReserved(value) => {
                ObservationDigestRecordWire::AuthorizedReserved(value.digest_record())
            }
            Self::AuthorizedSeparate(value) => {
                ObservationDigestRecordWire::AuthorizedSeparate(value.digest_record())
            }
            Self::ExternalSupport(value) => {
                ObservationDigestRecordWire::ExternalSupport(value.digest_record())
            }
            Self::PreArmAwaiting(value) => {
                ObservationDigestRecordWire::PreArmAwaiting(value.digest_record())
            }
            Self::PreArmFrozen(value) => {
                ObservationDigestRecordWire::PreArmFrozen(value.digest_record())
            }
            Self::ActionCorrectionReserved(value) => {
                ObservationDigestRecordWire::ActionCorrectionReserved(value.digest_record())
            }
            Self::ActionCorrectionSeparate(value) => {
                ObservationDigestRecordWire::ActionCorrectionSeparate(value.digest_record())
            }
            Self::ExternalConflictCorrection(value) => {
                ObservationDigestRecordWire::ExternalConflictCorrection(value.digest_record())
            }
            Self::InvalidThisActionReserved(value) => {
                ObservationDigestRecordWire::InvalidThisActionReserved(value.digest_record())
            }
            Self::InvalidThisActionSeparate(value) => {
                ObservationDigestRecordWire::InvalidThisActionSeparate(value.digest_record())
            }
            Self::InvalidExternalActor(value) => {
                ObservationDigestRecordWire::InvalidExternalActor(value.digest_record())
            }
            Self::InvalidUnattributed(value) => {
                ObservationDigestRecordWire::InvalidUnattributed(value.digest_record())
            }
        };
        SupportPrerequisiteVersionObservationDigestRecord(record)
    }

    fn repository_version(&self) -> &RepositoryVersion {
        match self {
            Self::Routine(value) => &value.repository_version,
            Self::AuthorizedReserved(value) => &value.repository_version,
            Self::AuthorizedSeparate(value) => &value.repository_version,
            Self::ExternalSupport(value) => &value.repository_version,
            Self::PreArmAwaiting(value) => &value.repository_version,
            Self::PreArmFrozen(value) => &value.repository_version,
            Self::ActionCorrectionReserved(value) => &value.repository_version,
            Self::ActionCorrectionSeparate(value) => &value.repository_version,
            Self::ExternalConflictCorrection(value) => &value.repository_version,
            Self::InvalidThisActionReserved(value) => &value.repository_version,
            Self::InvalidThisActionSeparate(value) => &value.repository_version,
            Self::InvalidExternalActor(value) => &value.repository_version,
            Self::InvalidUnattributed(value) => &value.repository_version,
        }
    }

    fn classification_digest(&self) -> &Sha256Digest {
        match self {
            Self::Routine(value) => &value.classification_digest,
            Self::AuthorizedReserved(value) => &value.classification_digest,
            Self::AuthorizedSeparate(value) => &value.classification_digest,
            Self::ExternalSupport(value) => &value.classification_digest,
            Self::PreArmAwaiting(value) => &value.classification_digest,
            Self::PreArmFrozen(value) => &value.classification_digest,
            Self::ActionCorrectionReserved(value) => &value.classification_digest,
            Self::ActionCorrectionSeparate(value) => &value.classification_digest,
            Self::ExternalConflictCorrection(value) => &value.classification_digest,
            Self::InvalidThisActionReserved(value) => &value.classification_digest,
            Self::InvalidThisActionSeparate(value) => &value.classification_digest,
            Self::InvalidExternalActor(value) => &value.classification_digest,
            Self::InvalidUnattributed(value) => &value.classification_digest,
        }
    }

    fn validate_relations(&self) -> Result<(), &'static str> {
        match self {
            Self::AuthorizedReserved(value) => (value.authorized_transitions_digest
                == value.observed_support_transitions_digest)
                .then_some(())
                .ok_or("authorized observation transition digests disagree"),
            Self::AuthorizedSeparate(value) => (value.authorized_transitions_digest
                == value.observed_support_transitions_digest)
                .then_some(())
                .ok_or("authorized observation transition digests disagree"),
            Self::InvalidThisActionReserved(value) => {
                if !value.first_root_support_after_arming
                    && !value
                        .mismatch_kinds
                        .contains(SupportPrerequisiteMismatchKind::ArmingOrderViolated)
                {
                    return Err("non-first authorized-action observation lacks arming mismatch");
                }
                Ok(())
            }
            Self::InvalidThisActionSeparate(value) => {
                if !value.first_root_support_after_arming
                    && !value
                        .mismatch_kinds
                        .contains(SupportPrerequisiteMismatchKind::ArmingOrderViolated)
                {
                    return Err("non-first authorized-action observation lacks arming mismatch");
                }
                Ok(())
            }
            Self::InvalidUnattributed(value) => {
                if value.repository_actor.0.as_ref().is_some()
                    || !value
                        .mismatch_kinds
                        .contains(SupportPrerequisiteMismatchKind::VersionUnattributed)
                {
                    return Err("unattributed observation provenance disagrees with evidence");
                }
                Ok(())
            }
            Self::Routine(_)
            | Self::ExternalSupport(_)
            | Self::PreArmAwaiting(_)
            | Self::PreArmFrozen(_)
            | Self::ActionCorrectionReserved(_)
            | Self::ActionCorrectionSeparate(_)
            | Self::ExternalConflictCorrection(_)
            | Self::InvalidExternalActor(_) => Ok(()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Intrinsic Task 8 mapper projection from a digest-validated observation.
///
/// This is deliberately not a production authority: it does not prove the
/// active registry/index row, content-addressed ref, partition entry, or
/// semantic-delta match. Only the repository resolver may combine it with
/// those proofs and construct downstream control-flow authority.
pub(crate) struct SupportObservationTask8Projection {
    partition_classification: RepositoryHistoryPartitionClassification,
    root_delta_digest: Option<Sha256Digest>,
    content_delta_digest: Option<Sha256Digest>,
    classification_digest: Sha256Digest,
    external_support_disjointness_digest: Option<Sha256Digest>,
}

/// Intrinsic Task 9 projection for the two instruction-bound corrective leaves.
///
/// Like the Task 8 projection, this is data only and is not an authority. The
/// repository resolver must still load and rehash the exact historical
/// instruction, validate external ownership when applicable, and bind the
/// active source-index proof before a corrective partition entry can exist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SupportObservationCorrectiveProjection {
    ActionCorrection {
        repository_actor: RepositoryActorIdentity,
        manual_target_mode: ManualSupportTargetMode,
        working_infobase_identity: Option<ManualWorkingInfobaseIdentity>,
        root_delta_digest: Sha256Digest,
        content_delta_digest: Sha256Digest,
        corrective_instruction_digest: Sha256Digest,
    },
    ExternalConflictCorrection {
        repository_actor: RepositoryActorIdentity,
        root_delta_digest: Sha256Digest,
        content_delta_digest: Sha256Digest,
        conflict_resolution_id: UnicaId,
        support_conflict_instruction_digest: Sha256Digest,
        final_baseline_digest: Sha256Digest,
        external_ownership_evidence: ExternalSupportOwnershipEvidence,
    },
}

impl SupportObservationTask8Projection {
    pub(crate) const fn partition_classification(
        &self,
    ) -> RepositoryHistoryPartitionClassification {
        self.partition_classification
    }

    pub(crate) fn root_delta_digest(&self) -> Option<Sha256Digest> {
        self.root_delta_digest.clone()
    }

    pub(crate) fn content_delta_digest(&self) -> Option<Sha256Digest> {
        self.content_delta_digest.clone()
    }

    pub(crate) fn classification_digest(&self) -> Sha256Digest {
        self.classification_digest.clone()
    }

    pub(crate) fn external_support_disjointness_digest(&self) -> Option<Sha256Digest> {
        self.external_support_disjointness_digest.clone()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct SupportPrerequisiteVersionObservation(ObservationWire);

impl SupportPrerequisiteVersionObservation {
    pub(crate) fn repository_version(&self) -> &RepositoryVersion {
        self.0.repository_version()
    }

    pub(crate) fn classification_digest(&self) -> &Sha256Digest {
        self.0.classification_digest()
    }

    pub(crate) fn task8_mapping_projection(&self) -> Option<SupportObservationTask8Projection> {
        let empty_digest = || {
            Sha256Digest::parse(CanonicalEmptyDeltaDigest::VALUE)
                .expect("canonical empty delta digest is a valid SHA-256")
        };
        let projection = match &self.0 {
            ObservationWire::Routine(value) => SupportObservationTask8Projection {
                partition_classification: match value.relevance {
                    RepositoryRelevance::Unrelated => {
                        RepositoryHistoryPartitionClassification::UnrelatedRoutine
                    }
                    RepositoryRelevance::Relevant => {
                        RepositoryHistoryPartitionClassification::RelevantRoutine
                    }
                },
                root_delta_digest: Some(value.root_delta_digest.clone()),
                content_delta_digest: Some(value.content_delta_digest.clone()),
                classification_digest: value.classification_digest.clone(),
                external_support_disjointness_digest: None,
            },
            ObservationWire::AuthorizedReserved(value) => SupportObservationTask8Projection {
                partition_classification:
                    RepositoryHistoryPartitionClassification::AuthorizedSupport,
                root_delta_digest: Some(value.root_delta_digest.clone()),
                content_delta_digest: Some(empty_digest()),
                classification_digest: value.classification_digest.clone(),
                external_support_disjointness_digest: None,
            },
            ObservationWire::AuthorizedSeparate(value) => SupportObservationTask8Projection {
                partition_classification:
                    RepositoryHistoryPartitionClassification::AuthorizedSupport,
                root_delta_digest: Some(value.root_delta_digest.clone()),
                content_delta_digest: Some(empty_digest()),
                classification_digest: value.classification_digest.clone(),
                external_support_disjointness_digest: None,
            },
            ObservationWire::ExternalSupport(value) => SupportObservationTask8Projection {
                partition_classification: RepositoryHistoryPartitionClassification::ExternalSupport,
                root_delta_digest: Some(value.root_delta_digest.clone()),
                content_delta_digest: Some(empty_digest()),
                classification_digest: value.classification_digest.clone(),
                external_support_disjointness_digest: Some(
                    value.external_support_disjointness_digest.clone(),
                ),
            },
            ObservationWire::PreArmAwaiting(value) => SupportObservationTask8Projection {
                partition_classification: RepositoryHistoryPartitionClassification::PreArmExternal,
                root_delta_digest: Some(value.root_delta_digest.clone()),
                content_delta_digest: Some(value.content_delta_digest.clone()),
                classification_digest: value.classification_digest.clone(),
                external_support_disjointness_digest: None,
            },
            ObservationWire::PreArmFrozen(value) => SupportObservationTask8Projection {
                partition_classification: RepositoryHistoryPartitionClassification::PreArmExternal,
                root_delta_digest: Some(value.root_delta_digest.clone()),
                content_delta_digest: Some(value.content_delta_digest.clone()),
                classification_digest: value.classification_digest.clone(),
                external_support_disjointness_digest: None,
            },
            ObservationWire::InvalidThisActionReserved(value) => {
                SupportObservationTask8Projection {
                    partition_classification: RepositoryHistoryPartitionClassification::Invalid,
                    root_delta_digest: Some(value.root_delta_digest.clone()),
                    content_delta_digest: Some(value.content_delta_digest.clone()),
                    classification_digest: value.classification_digest.clone(),
                    external_support_disjointness_digest: None,
                }
            }
            ObservationWire::InvalidThisActionSeparate(value) => {
                SupportObservationTask8Projection {
                    partition_classification: RepositoryHistoryPartitionClassification::Invalid,
                    root_delta_digest: Some(value.root_delta_digest.clone()),
                    content_delta_digest: Some(value.content_delta_digest.clone()),
                    classification_digest: value.classification_digest.clone(),
                    external_support_disjointness_digest: None,
                }
            }
            ObservationWire::InvalidExternalActor(value) => SupportObservationTask8Projection {
                partition_classification: RepositoryHistoryPartitionClassification::Invalid,
                root_delta_digest: Some(value.root_delta_digest.clone()),
                content_delta_digest: Some(value.content_delta_digest.clone()),
                classification_digest: value.classification_digest.clone(),
                external_support_disjointness_digest: None,
            },
            ObservationWire::InvalidUnattributed(value) => SupportObservationTask8Projection {
                partition_classification: RepositoryHistoryPartitionClassification::Invalid,
                root_delta_digest: value.root_delta_digest.as_ref().cloned(),
                content_delta_digest: value.content_delta_digest.as_ref().cloned(),
                classification_digest: value.classification_digest.clone(),
                external_support_disjointness_digest: None,
            },
            ObservationWire::ActionCorrectionReserved(_)
            | ObservationWire::ActionCorrectionSeparate(_)
            | ObservationWire::ExternalConflictCorrection(_) => return None,
        };
        Some(projection)
    }

    pub(crate) fn task9_corrective_projection(
        &self,
    ) -> Option<SupportObservationCorrectiveProjection> {
        match &self.0 {
            ObservationWire::ActionCorrectionReserved(value) => {
                Some(SupportObservationCorrectiveProjection::ActionCorrection {
                    repository_actor: value.repository_actor.clone(),
                    manual_target_mode: ManualSupportTargetMode::ReservedOriginal,
                    working_infobase_identity: None,
                    root_delta_digest: value.root_delta_digest.clone(),
                    content_delta_digest: value.content_delta_digest.clone(),
                    corrective_instruction_digest: value.corrective_instruction_digest.clone(),
                })
            }
            ObservationWire::ActionCorrectionSeparate(value) => {
                Some(SupportObservationCorrectiveProjection::ActionCorrection {
                    repository_actor: value.repository_actor.clone(),
                    manual_target_mode: ManualSupportTargetMode::SeparateWorkingInfobase,
                    working_infobase_identity: Some(value.working_infobase_identity.clone()),
                    root_delta_digest: value.root_delta_digest.clone(),
                    content_delta_digest: value.content_delta_digest.clone(),
                    corrective_instruction_digest: value.corrective_instruction_digest.clone(),
                })
            }
            ObservationWire::ExternalConflictCorrection(value) => Some(
                SupportObservationCorrectiveProjection::ExternalConflictCorrection {
                    repository_actor: value.repository_actor.clone(),
                    root_delta_digest: value.root_delta_digest.clone(),
                    content_delta_digest: value.content_delta_digest.clone(),
                    conflict_resolution_id: value.conflict_resolution_id.clone(),
                    support_conflict_instruction_digest: value
                        .support_conflict_instruction_digest
                        .clone(),
                    final_baseline_digest: value.final_baseline_digest.clone(),
                    external_ownership_evidence: value.external_ownership_evidence.clone(),
                },
            ),
            ObservationWire::Routine(_)
            | ObservationWire::AuthorizedReserved(_)
            | ObservationWire::AuthorizedSeparate(_)
            | ObservationWire::ExternalSupport(_)
            | ObservationWire::PreArmAwaiting(_)
            | ObservationWire::PreArmFrozen(_)
            | ObservationWire::InvalidThisActionReserved(_)
            | ObservationWire::InvalidThisActionSeparate(_)
            | ObservationWire::InvalidExternalActor(_)
            | ObservationWire::InvalidUnattributed(_) => None,
        }
    }
}

impl<'de> Deserialize<'de> for SupportPrerequisiteVersionObservation {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = ObservationWire::deserialize(deserializer)?;
        wire.validate_relations().map_err(D::Error::custom)?;
        let expected = canonical_contract_digest(&wire.digest_record(), None)
            .map_err(|_| D::Error::custom("support observation digest failed"))?;
        if &expected != wire.classification_digest() {
            return Err(D::Error::custom("support observation digest mismatch"));
        }
        Ok(Self(wire))
    }
}

impl JsonSchema for SupportPrerequisiteVersionObservation {
    fn schema_name() -> Cow<'static, str> {
        "SupportPrerequisiteVersionObservation".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<RoutineObservation>(),
            generator.subschema_for::<AuthorizedReservedObservation>(),
            generator.subschema_for::<AuthorizedSeparateObservation>(),
            generator.subschema_for::<ExternalSupportObservation>(),
            generator.subschema_for::<PreArmAwaitingObservation>(),
            generator.subschema_for::<PreArmFrozenObservation>(),
            generator.subschema_for::<ActionCorrectionReservedObservation>(),
            generator.subschema_for::<ActionCorrectionSeparateObservation>(),
            generator.subschema_for::<ExternalConflictCorrectionObservation>(),
            generator.subschema_for::<InvalidThisActionReservedObservation>(),
            generator.subschema_for::<InvalidThisActionSeparateObservation>(),
            generator.subschema_for::<InvalidExternalActorObservation>(),
            generator.subschema_for::<InvalidUnattributedObservation>(),
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::{
        SupportPrerequisiteVersionObservation, SupportPrerequisiteVersionObservationDigestRecord,
    };
    use crate::domain::branched_development::contracts::repository::CanonicalEmptyDeltaDigest;
    use crate::domain::branched_development::contracts::schema::audit_json_schema;
    use schemars::schema_for;
    use serde_json::{json, Value};
    use sha2::{Digest, Sha256};

    const SHA_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const UUID_A: &str = "123e4567-e89b-12d3-a456-426614174000";

    fn classification_digest(value: &Value) -> String {
        let mut record = value.clone();
        record
            .as_object_mut()
            .expect("observation fixture is an object")
            .remove("classificationDigest");
        format!(
            "{:x}",
            Sha256::digest(serde_json_canonicalizer::to_vec(&record).unwrap())
        )
    }

    fn actor() -> Value {
        json!({
            "username": "repository-user",
            "computer": null,
            "infobase": null
        })
    }

    fn working_identity() -> Value {
        let record = json!({ "computer": "HOST", "infobase": "Working IB" });
        json!({
            "computer": "HOST",
            "infobase": "Working IB",
            "digest": format!(
                "{:x}",
                Sha256::digest(serde_json_canonicalizer::to_vec(&record).unwrap())
            )
        })
    }

    fn external_ownership() -> Value {
        json!({
            "kind": "supportPrerequisiteReceipt",
            "receiptId": UUID_A,
            "receiptDigest": SHA_A
        })
    }

    fn finalize(mut value: Value) -> Value {
        value["classificationDigest"] = json!(classification_digest(&value));
        value
    }

    fn routine_observation() -> Value {
        finalize(json!({
            "repositoryVersion": "opaque-v1",
            "classification": "routine",
            "classificationDigest": SHA_A,
            "mismatchKinds": [],
            "repositoryActor": actor(),
            "relevance": "unrelated",
            "rootDeltaDigest": SHA_A,
            "contentDeltaDigest": SHA_A,
            "supportTransitionsDigest": CanonicalEmptyDeltaDigest::VALUE,
            "supportGraphUnchanged": true
        }))
    }

    fn accepts(value: Value) -> SupportPrerequisiteVersionObservation {
        serde_json::from_value(value.clone())
            .unwrap_or_else(|error| panic!("observation rejected {value}: {error}"))
    }

    #[test]
    fn routine_observation_is_closed_and_digest_bound() {
        let value = routine_observation();
        assert_eq!(serde_json::to_value(accepts(value.clone())).unwrap(), value);

        let mut wrong_digest = value.clone();
        wrong_digest["classificationDigest"] = json!(SHA_A);
        assert!(
            serde_json::from_value::<SupportPrerequisiteVersionObservation>(wrong_digest).is_err()
        );

        let mut extra = value;
        extra["manualTargetMode"] = json!("reservedOriginal");
        assert!(serde_json::from_value::<SupportPrerequisiteVersionObservation>(extra).is_err());

        for schema in [
            serde_json::to_value(schema_for!(SupportPrerequisiteVersionObservation)).unwrap(),
            serde_json::to_value(schema_for!(
                SupportPrerequisiteVersionObservationDigestRecord
            ))
            .unwrap(),
        ] {
            audit_json_schema(&schema).unwrap();
        }
    }

    #[test]
    fn positive_and_pre_arm_observation_leaves_enforce_exact_mode_presence() {
        let authorized_reserved = finalize(json!({
            "repositoryVersion":"opaque-v2",
            "classification":"authorized",
            "classificationDigest":SHA_A,
            "mismatchKinds":[],
            "repositoryActor":actor(),
            "supportActionId":UUID_A,
            "supportActionDigest":SHA_A,
            "armingReceiptId":UUID_A,
            "armingReceiptDigest":SHA_A,
            "firstRootSupportAfterArming":true,
            "actionAttributionEvidenceDigest":SHA_A,
            "authorizedTransitionsDigest":SHA_A,
            "manualTargetMode":"reservedOriginal",
            "rootDeltaDigest":SHA_A,
            "contentDeltaDigest":CanonicalEmptyDeltaDigest::VALUE,
            "observedSupportTransitionsDigest":SHA_A,
            "rootDeltaContainsOnlyAuthorizedSupportTransitions":true
        }));
        accepts(authorized_reserved.clone());
        let mut cross_mode = authorized_reserved;
        cross_mode["workingInfobaseIdentity"] = json!({
            "computer":"HOST",
            "infobase":"IB",
            "digest":SHA_A
        });
        cross_mode["classificationDigest"] = json!(classification_digest(&cross_mode));
        assert!(
            serde_json::from_value::<SupportPrerequisiteVersionObservation>(cross_mode).is_err()
        );

        let awaiting = finalize(json!({
            "repositoryVersion":"opaque-v3",
            "classification":"preArmExternal",
            "classificationDigest":SHA_A,
            "mismatchKinds":["armingOrderViolated"],
            "pendingSupportActionId":UUID_A,
            "pendingSupportActionDigest":SHA_A,
            "authorizationState":"awaitingArm",
            "armingReceiptAbsent":true,
            "repositoryActor":actor(),
            "rootDeltaDigest":SHA_A,
            "contentDeltaDigest":SHA_A,
            "supportTransitionsDigest":SHA_A,
            "preserveAsExternalBaseline":true
        }));
        accepts(awaiting.clone());
        let mut illegal_freeze = awaiting;
        illegal_freeze["freezeKind"] = json!("preArmCancellationEffect");
        illegal_freeze["preArmFreezeDigest"] = json!(SHA_A);
        illegal_freeze["classificationDigest"] = json!(classification_digest(&illegal_freeze));
        assert!(
            serde_json::from_value::<SupportPrerequisiteVersionObservation>(illegal_freeze)
                .is_err()
        );
    }

    #[test]
    fn corrective_is_structural_but_has_no_task8_semantic_mapping() {
        let value = finalize(json!({
            "repositoryVersion":"opaque-v4",
            "classification":"corrective",
            "classificationDigest":SHA_A,
            "mismatchKinds":[],
            "correctionKind":"actionCorrection",
            "repositoryActor":actor(),
            "manualTargetMode":"reservedOriginal",
            "rootDeltaDigest":SHA_A,
            "contentDeltaDigest":SHA_A,
            "correctiveInstructionDigest":SHA_A
        }));
        let observation = accepts(value.clone());
        assert!(observation.task8_mapping_projection().is_none());

        let schema =
            serde_json::to_value(schema_for!(SupportPrerequisiteVersionObservation)).unwrap();
        assert!(jsonschema::validator_for(&schema).unwrap().is_valid(&value));
    }

    #[test]
    fn task9_corrective_projection_preserves_the_exact_instruction_binding() {
        let reserved = accepts(finalize(json!({
            "repositoryVersion":"opaque-v4",
            "classification":"corrective",
            "classificationDigest":SHA_A,
            "mismatchKinds":[],
            "correctionKind":"actionCorrection",
            "repositoryActor":actor(),
            "manualTargetMode":"reservedOriginal",
            "rootDeltaDigest":SHA_A,
            "contentDeltaDigest":SHA_A,
            "correctiveInstructionDigest":SHA_A
        })));
        match reserved.task9_corrective_projection().unwrap() {
            super::SupportObservationCorrectiveProjection::ActionCorrection {
                manual_target_mode,
                working_infobase_identity,
                corrective_instruction_digest,
                ..
            } => {
                assert_eq!(
                    manual_target_mode,
                    super::ManualSupportTargetMode::ReservedOriginal
                );
                assert!(working_infobase_identity.is_none());
                assert_eq!(corrective_instruction_digest.as_str(), SHA_A);
            }
            other => panic!("unexpected corrective projection: {other:?}"),
        }

        let external = accepts(finalize(json!({
            "repositoryVersion":"opaque-v6",
            "classification":"corrective",
            "classificationDigest":SHA_A,
            "mismatchKinds":[],
            "correctionKind":"externalConflictCorrection",
            "repositoryActor":actor(),
            "rootDeltaDigest":SHA_A,
            "contentDeltaDigest":SHA_A,
            "conflictResolutionId":UUID_A,
            "supportConflictInstructionDigest":SHA_A,
            "finalBaselineDigest":SHA_A,
            "externalOwnershipEvidence":external_ownership()
        })));
        match external.task9_corrective_projection().unwrap() {
            super::SupportObservationCorrectiveProjection::ExternalConflictCorrection {
                conflict_resolution_id,
                support_conflict_instruction_digest,
                ..
            } => {
                assert_eq!(conflict_resolution_id.as_str(), UUID_A);
                assert_eq!(support_conflict_instruction_digest.as_str(), SHA_A);
            }
            other => panic!("unexpected corrective projection: {other:?}"),
        }
    }

    #[test]
    fn all_thirteen_physical_leaves_round_trip_and_rehash() {
        let mut leaves = vec![routine_observation()];

        let authorized_reserved = finalize(json!({
            "repositoryVersion":"opaque-v2",
            "classification":"authorized",
            "classificationDigest":SHA_A,
            "mismatchKinds":[],
            "repositoryActor":actor(),
            "supportActionId":UUID_A,
            "supportActionDigest":SHA_A,
            "armingReceiptId":UUID_A,
            "armingReceiptDigest":SHA_A,
            "firstRootSupportAfterArming":true,
            "actionAttributionEvidenceDigest":SHA_A,
            "authorizedTransitionsDigest":SHA_A,
            "manualTargetMode":"reservedOriginal",
            "rootDeltaDigest":SHA_A,
            "contentDeltaDigest":CanonicalEmptyDeltaDigest::VALUE,
            "observedSupportTransitionsDigest":SHA_A,
            "rootDeltaContainsOnlyAuthorizedSupportTransitions":true
        }));
        leaves.push(authorized_reserved.clone());
        let mut authorized_separate = authorized_reserved;
        authorized_separate["manualTargetMode"] = json!("separateWorkingInfobase");
        authorized_separate["workingInfobaseIdentity"] = working_identity();
        authorized_separate["classificationDigest"] =
            json!(classification_digest(&authorized_separate));
        leaves.push(authorized_separate);

        leaves.push(finalize(json!({
            "repositoryVersion":"opaque-v3",
            "classification":"externalSupport",
            "classificationDigest":SHA_A,
            "mismatchKinds":[],
            "repositoryActor":actor(),
            "rootDeltaDigest":SHA_A,
            "contentDeltaDigest":CanonicalEmptyDeltaDigest::VALUE,
            "provenNotThisAction":true,
            "overlapWithAuthorizedTransitions":false,
            "supportOnlyDelta":true,
            "externalSupportDisjointnessDigest":SHA_A,
            "externalOwnershipEvidence":external_ownership()
        })));

        let pre_arm_awaiting = finalize(json!({
            "repositoryVersion":"opaque-v4",
            "classification":"preArmExternal",
            "classificationDigest":SHA_A,
            "mismatchKinds":["armingOrderViolated"],
            "pendingSupportActionId":UUID_A,
            "pendingSupportActionDigest":SHA_A,
            "authorizationState":"awaitingArm",
            "armingReceiptAbsent":true,
            "repositoryActor":actor(),
            "rootDeltaDigest":SHA_A,
            "contentDeltaDigest":SHA_A,
            "supportTransitionsDigest":SHA_A,
            "preserveAsExternalBaseline":true
        }));
        leaves.push(pre_arm_awaiting.clone());
        let mut pre_arm_frozen = pre_arm_awaiting;
        pre_arm_frozen["authorizationState"] = json!("frozenForRecovery");
        pre_arm_frozen["freezeKind"] = json!("preArmCancellationEffect");
        pre_arm_frozen["preArmFreezeDigest"] = json!(SHA_A);
        pre_arm_frozen["classificationDigest"] = json!(classification_digest(&pre_arm_frozen));
        leaves.push(pre_arm_frozen);

        let correction_reserved = finalize(json!({
            "repositoryVersion":"opaque-v5",
            "classification":"corrective",
            "classificationDigest":SHA_A,
            "mismatchKinds":[],
            "correctionKind":"actionCorrection",
            "repositoryActor":actor(),
            "manualTargetMode":"reservedOriginal",
            "rootDeltaDigest":SHA_A,
            "contentDeltaDigest":SHA_A,
            "correctiveInstructionDigest":SHA_A
        }));
        leaves.push(correction_reserved.clone());
        let mut correction_separate = correction_reserved;
        correction_separate["manualTargetMode"] = json!("separateWorkingInfobase");
        correction_separate["workingInfobaseIdentity"] = working_identity();
        correction_separate["classificationDigest"] =
            json!(classification_digest(&correction_separate));
        leaves.push(correction_separate);
        leaves.push(finalize(json!({
            "repositoryVersion":"opaque-v6",
            "classification":"corrective",
            "classificationDigest":SHA_A,
            "mismatchKinds":[],
            "correctionKind":"externalConflictCorrection",
            "repositoryActor":actor(),
            "rootDeltaDigest":SHA_A,
            "contentDeltaDigest":SHA_A,
            "conflictResolutionId":UUID_A,
            "supportConflictInstructionDigest":SHA_A,
            "finalBaselineDigest":SHA_A,
            "externalOwnershipEvidence":external_ownership()
        })));

        let invalid_this_reserved = finalize(json!({
            "repositoryVersion":"opaque-v7",
            "classification":"invalid",
            "classificationDigest":SHA_A,
            "mismatchKinds":["noAuthorizedVersionObserved"],
            "provenance":"thisAuthorizedAction",
            "repositoryActor":actor(),
            "manualTargetMode":"reservedOriginal",
            "armingReceiptId":UUID_A,
            "armingReceiptDigest":SHA_A,
            "firstRootSupportAfterArming":true,
            "rootDeltaDigest":SHA_A,
            "contentDeltaDigest":SHA_A,
            "actionAttributionEvidenceDigest":SHA_A
        }));
        leaves.push(invalid_this_reserved.clone());
        let mut invalid_this_separate = invalid_this_reserved;
        invalid_this_separate["manualTargetMode"] = json!("separateWorkingInfobase");
        invalid_this_separate["workingInfobaseIdentity"] = working_identity();
        invalid_this_separate["classificationDigest"] =
            json!(classification_digest(&invalid_this_separate));
        leaves.push(invalid_this_separate);
        leaves.push(finalize(json!({
            "repositoryVersion":"opaque-v8",
            "classification":"invalid",
            "classificationDigest":SHA_A,
            "mismatchKinds":["targetModeMismatch"],
            "provenance":"externalActor",
            "repositoryActor":actor(),
            "observedWorkingInfobaseIdentity":null,
            "rootDeltaDigest":SHA_A,
            "contentDeltaDigest":SHA_A,
            "provenNotThisAction":true,
            "externalOwnershipEvidence":external_ownership()
        })));
        leaves.push(finalize(json!({
            "repositoryVersion":"opaque-v9",
            "classification":"invalid",
            "classificationDigest":SHA_A,
            "mismatchKinds":["versionUnattributed"],
            "provenance":"unattributed",
            "repositoryActor":null,
            "rootDeltaDigest":null,
            "contentDeltaDigest":null,
            "missingEvidenceKinds":["repositoryActorUnavailable"]
        })));

        assert_eq!(leaves.len(), 13);
        for value in leaves {
            let observation = accepts(value.clone());
            assert_eq!(serde_json::to_value(&observation).unwrap(), value);
            let corrective = value["classification"] == json!("corrective");
            assert_eq!(observation.task8_mapping_projection().is_none(), corrective);

            let mut substituted = value;
            substituted["classificationDigest"] = json!(SHA_A);
            assert!(
                serde_json::from_value::<SupportPrerequisiteVersionObservation>(substituted)
                    .is_err()
            );
        }

        let schema =
            serde_json::to_value(schema_for!(SupportPrerequisiteVersionObservation)).unwrap();
        assert_eq!(schema["oneOf"].as_array().map(Vec::len), Some(13));
        assert!(schema.get("anyOf").is_none());
    }

    #[test]
    fn invalid_observations_require_nullable_keys_and_semantic_order() {
        let external = finalize(json!({
            "repositoryVersion":"opaque-v8",
            "classification":"invalid",
            "classificationDigest":SHA_A,
            "mismatchKinds":["targetModeMismatch"],
            "provenance":"externalActor",
            "repositoryActor":actor(),
            "observedWorkingInfobaseIdentity":null,
            "rootDeltaDigest":SHA_A,
            "contentDeltaDigest":SHA_A,
            "provenNotThisAction":true,
            "externalOwnershipEvidence":external_ownership()
        }));
        let mut omitted = external;
        omitted
            .as_object_mut()
            .unwrap()
            .remove("observedWorkingInfobaseIdentity");
        omitted["classificationDigest"] = json!(classification_digest(&omitted));
        assert!(serde_json::from_value::<SupportPrerequisiteVersionObservation>(omitted).is_err());

        let unattributed = finalize(json!({
            "repositoryVersion":"opaque-v9",
            "classification":"invalid",
            "classificationDigest":SHA_A,
            "mismatchKinds":["versionUnattributed", "targetModeMismatch"],
            "provenance":"unattributed",
            "repositoryActor":null,
            "rootDeltaDigest":null,
            "contentDeltaDigest":null,
            "missingEvidenceKinds":[
                "repositoryActorUnavailable",
                "rootDeltaUnavailable"
            ]
        }));
        accepts(unattributed.clone());
        let schema =
            serde_json::to_value(schema_for!(SupportPrerequisiteVersionObservation)).unwrap();
        let validator = jsonschema::validator_for(&schema).unwrap();
        assert!(validator.is_valid(&unattributed));
        let mut actor_substitution = unattributed.clone();
        actor_substitution["repositoryActor"] = actor();
        actor_substitution["classificationDigest"] =
            json!(classification_digest(&actor_substitution));
        assert!(!validator.is_valid(&actor_substitution));
        assert!(
            serde_json::from_value::<SupportPrerequisiteVersionObservation>(actor_substitution)
                .is_err()
        );
        for required in ["repositoryActor", "rootDeltaDigest", "contentDeltaDigest"] {
            let mut missing = unattributed.clone();
            missing.as_object_mut().unwrap().remove(required);
            missing["classificationDigest"] = json!(classification_digest(&missing));
            assert!(
                serde_json::from_value::<SupportPrerequisiteVersionObservation>(missing).is_err()
            );
        }
        for (field, invalid) in [
            (
                "mismatchKinds",
                json!(["targetModeMismatch", "versionUnattributed"]),
            ),
            (
                "missingEvidenceKinds",
                json!(["rootDeltaUnavailable", "repositoryActorUnavailable"]),
            ),
        ] {
            let mut reordered = unattributed.clone();
            reordered[field] = invalid;
            reordered["classificationDigest"] = json!(classification_digest(&reordered));
            assert!(
                serde_json::from_value::<SupportPrerequisiteVersionObservation>(reordered).is_err()
            );
        }
    }
}
