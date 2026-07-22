#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::branched_development::contracts::scalars::RepositoryIdentityComponent;
    use crate::domain::branched_development::contracts::scalars::{Diagnostic, RepositoryVersion};
    use crate::domain::branched_development::contracts::schema::audit_json_schema;
    use crate::domain::branched_development::contracts::support::{
        SupportContractError, SupportEvidenceGap, SupportHistoryOrderAuthority,
        SupportMissingEvidenceKind, SupportTransition, SupportTransitionConflict,
        SupportTransitionOverlapKind,
    };
    use crate::domain::branched_development::SupportLayerId;
    use schemars::{schema_for, JsonSchema};
    use serde::de::DeserializeOwned;
    use serde_json::{json, Value};
    use std::cmp::Ordering;

    const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const ID_1: &str = "11111111-1111-4111-8111-111111111111";
    const ID_2: &str = "22222222-2222-4222-8222-222222222222";

    fn assert_schema_closed<T: JsonSchema>() {
        let schema = serde_json::to_value(schema_for!(T)).unwrap();
        audit_json_schema(&schema).unwrap();
    }

    fn schema_accepts<T: JsonSchema>(value: &Value) -> bool {
        let schema = serde_json::to_value(schema_for!(T)).unwrap();
        jsonschema::options()
            .with_draft(jsonschema::Draft::Draft202012)
            .build(&schema)
            .unwrap()
            .is_valid(value)
    }

    fn assert_missing_and_extra_rejected<T: JsonSchema>(value: &Value) {
        let mut missing = value.clone();
        missing.as_object_mut().unwrap().remove("kind");
        assert!(!schema_accepts::<T>(&missing));

        let mut extra = value.clone();
        extra["unexpected"] = json!(true);
        assert!(!schema_accepts::<T>(&extra));
    }

    fn id(value: &str) -> UnicaId {
        UnicaId::parse(value).unwrap()
    }

    fn digest(value: &str) -> Sha256Digest {
        Sha256Digest::parse(value).unwrap()
    }

    fn cursor() -> RepositoryHistoryCursor {
        serde_json::from_value(json!({
            "throughVersion": "v1",
            "historyPrefixDigest": A,
        }))
        .unwrap()
    }

    fn username() -> RepositoryUsername {
        RepositoryUsername::parse("support-user").unwrap()
    }

    fn working_identity() -> ManualWorkingInfobaseIdentity {
        ManualWorkingInfobaseIdentity::new(
            RepositoryIdentityComponent::parse("HOST").unwrap(),
            RepositoryIdentityComponent::parse("Working IB").unwrap(),
        )
        .unwrap()
    }

    fn transitions() -> SupportTransitions {
        SupportTransitions::new(vec![SupportTransition::enable_configuration_changes(
            RepositoryTargetDisplay::parse("Configuration").unwrap(),
            SupportLayerId::parse("layer-a").unwrap(),
        )])
        .unwrap()
    }

    fn blockers() -> SupportBlockers {
        serde_json::from_value(json!([{
            "objectId": "00000000-0000-0000-0000-000000000001",
            "objectDisplay": "Catalog.A",
            "reason": "vendorRestriction",
            "diagnostic": "redacted",
        }]))
        .unwrap()
    }

    struct TrivialHistoryOrder;

    impl SupportHistoryOrderAuthority for TrivialHistoryOrder {
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

    fn evidence_gaps() -> SupportEvidenceGaps {
        let gap = serde_json::from_value::<SupportEvidenceGap>(json!({
            "gapKind": "candidateEvidence",
            "objectId": "00000000-0000-0000-0000-000000000001",
            "objectDisplay": "Catalog.A",
            "missingEvidenceKind": "candidateClassificationUnavailable",
            "diagnostic": "redacted",
        }))
        .unwrap();
        SupportEvidenceGaps::new(vec![gap], &TrivialHistoryOrder).unwrap()
    }

    fn conflicts() -> SupportTransitionConflicts {
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
        SupportTransitionConflicts::new(vec![conflict], &TrivialHistoryOrder).unwrap()
    }

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

    assert_not_deserialize_owned!(AcquireSupportRootInstruction);
    assert_not_deserialize_owned!(ReleaseRepositoryLocksInstruction);
    assert_not_deserialize_owned!(ManualSupportInstruction);
    assert_not_deserialize_owned!(CleanManualWorkingInfobaseInstruction);
    assert_not_deserialize_owned!(CloseReservedOriginalDesignerInstruction);
    assert_not_deserialize_owned!(SupportConflictInstruction);
    assert_not_deserialize_owned!(SupportEvidenceInstruction);
    assert_not_deserialize_owned!(VendorSupportDecisionInstruction);

    #[test]
    fn all_eight_external_instruction_record_schemas_are_recursively_closed() {
        assert_schema_closed::<AcquireSupportRootInstruction>();
        assert_schema_closed::<ReleaseRepositoryLocksInstruction>();
        assert_schema_closed::<ManualSupportInstruction>();
        assert_schema_closed::<CleanManualWorkingInfobaseInstruction>();
        assert_schema_closed::<CloseReservedOriginalDesignerInstruction>();
        assert_schema_closed::<SupportConflictInstruction>();
        assert_schema_closed::<SupportEvidenceInstruction>();
        assert_schema_closed::<VendorSupportDecisionInstruction>();
        assert_schema_closed::<ReleaseRepositoryLocksInstructionDigestRecord>();
        assert_schema_closed::<SupportConflictInstructionDigestRecord>();
        assert_schema_closed::<SupportEvidenceInstructionDigestRecord>();
    }

    #[test]
    fn acquire_root_instruction_enforces_the_exact_manual_mode_shape() {
        let reserved = AcquireSupportRootInstruction::new(
            id(ID_1),
            SupportActionPurpose::MainIntegrationPrerequisite,
            ManualSupportTargetMode::ReservedOriginal,
            username(),
            None,
        )
        .unwrap();
        let reserved_json = serde_json::to_value(reserved).unwrap();
        assert!(schema_accepts::<AcquireSupportRootInstruction>(
            &reserved_json
        ));
        assert_eq!(reserved_json["kind"], json!("acquireSupportRoot"));
        assert_eq!(reserved_json["lockCandidateObjects"], json!(false));
        assert_eq!(reserved_json["doNotEditBeforeArmed"], json!(true));
        assert_eq!(reserved_json["doNotCommitBeforeArmed"], json!(true));
        assert_missing_and_extra_rejected::<AcquireSupportRootInstruction>(&reserved_json);

        let mut spliced = reserved_json.clone();
        spliced["workingInfobaseIdentity"] = serde_json::to_value(working_identity()).unwrap();
        assert!(!schema_accepts::<AcquireSupportRootInstruction>(&spliced));
        assert!(AcquireSupportRootInstruction::new(
            id(ID_1),
            SupportActionPurpose::MainIntegrationPrerequisite,
            ManualSupportTargetMode::ReservedOriginal,
            username(),
            Some(working_identity()),
        )
        .is_err());

        let separate = AcquireSupportRootInstruction::new(
            id(ID_1),
            SupportActionPurpose::MainIntegrationPrerequisite,
            ManualSupportTargetMode::SeparateWorkingInfobase,
            username(),
            Some(working_identity()),
        )
        .unwrap();
        let mut missing = serde_json::to_value(separate).unwrap();
        missing
            .as_object_mut()
            .unwrap()
            .remove("workingInfobaseIdentity");
        assert!(!schema_accepts::<AcquireSupportRootInstruction>(&missing));
        assert!(AcquireSupportRootInstruction::new(
            id(ID_1),
            SupportActionPurpose::MainIntegrationPrerequisite,
            ManualSupportTargetMode::SeparateWorkingInfobase,
            username(),
            None,
        )
        .is_err());
    }

    #[test]
    fn release_locks_instruction_canonicalizes_no_display_and_hashes_exact_preimage() {
        assert!(ReleaseRepositoryLocksInstruction::new(
            RequiredNullable::null(),
            vec![
                RepositoryTargetDisplay::parse("B").unwrap(),
                RepositoryTargetDisplay::parse("A").unwrap(),
            ],
        )
        .is_err());
        assert!(ReleaseRepositoryLocksInstruction::new(
            RequiredNullable::null(),
            vec![
                RepositoryTargetDisplay::parse("A").unwrap(),
                RepositoryTargetDisplay::parse("A").unwrap(),
            ],
        )
        .is_err());

        let instruction = ReleaseRepositoryLocksInstruction::new(
            RequiredNullable::null(),
            vec![RepositoryTargetDisplay::parse("A").unwrap()],
        )
        .unwrap();
        let encoded = serde_json::to_value(&instruction).unwrap();
        assert_missing_and_extra_rejected::<ReleaseRepositoryLocksInstruction>(&encoded);
        assert_eq!(encoded["owner"], Value::Null);
        assert_eq!(encoded["coordinationRequired"], json!(true));
        assert_eq!(
            encoded["lockInstructionDigest"],
            serde_json::to_value(instruction.lock_instruction_digest()).unwrap()
        );
        assert_eq!(
            instruction.lock_instruction_digest(),
            &canonical_contract_digest(&instruction.digest_record(), None,).unwrap()
        );

        let mut substituted = encoded;
        substituted["lockInstructionDigest"] = json!(A);
        assert!(schema_accepts::<ReleaseRepositoryLocksInstruction>(
            &substituted
        ));
    }

    #[test]
    fn manual_support_instruction_selects_exactly_one_mode_closure_field() {
        let reserved = ManualSupportInstruction::new(
            id(ID_1),
            SupportActionPurpose::AbandonmentCleanup,
            id(ID_2),
            digest(A),
            cursor(),
            ManualSupportTargetMode::ReservedOriginal,
            username(),
            None,
            transitions(),
        )
        .unwrap();
        let mut reserved_json = serde_json::to_value(reserved).unwrap();
        assert_missing_and_extra_rejected::<ManualSupportInstruction>(&reserved_json);
        assert_eq!(
            reserved_json["closeReservedOriginalDesignerSession"],
            json!(true)
        );
        assert!(reserved_json.get("closeWorkingInfobaseSession").is_none());
        reserved_json["closeWorkingInfobaseSession"] = json!(true);
        assert!(!schema_accepts::<ManualSupportInstruction>(&reserved_json));

        let separate = ManualSupportInstruction::new(
            id(ID_1),
            SupportActionPurpose::AbandonmentCleanup,
            id(ID_2),
            digest(A),
            cursor(),
            ManualSupportTargetMode::SeparateWorkingInfobase,
            username(),
            Some(working_identity()),
            transitions(),
        )
        .unwrap();
        let separate_json = serde_json::to_value(separate).unwrap();
        assert_eq!(separate_json["closeWorkingInfobaseSession"], json!(true));
        assert!(separate_json
            .get("closeReservedOriginalDesignerSession")
            .is_none());

        assert!(ManualSupportInstruction::new(
            id(ID_1),
            SupportActionPurpose::AbandonmentCleanup,
            id(ID_2),
            digest(A),
            cursor(),
            ManualSupportTargetMode::ReservedOriginal,
            username(),
            Some(working_identity()),
            transitions(),
        )
        .is_err());
    }

    #[test]
    fn cleanup_and_close_instructions_expose_only_typed_fixed_effects() {
        let cleanup = CleanManualWorkingInfobaseInstruction::new(
            working_identity(),
            digest(A),
            CapabilityRowId::parse("manual-lease.v1").unwrap(),
            digest(A),
            ManualWorkingInfobaseCleanupReason::LeaseBusy,
        );
        let cleanup_json = serde_json::to_value(cleanup).unwrap();
        assert_missing_and_extra_rejected::<CleanManualWorkingInfobaseInstruction>(&cleanup_json);
        assert_eq!(cleanup_json["closeDesignerSession"], json!(true));
        assert_eq!(cleanup_json["resumeWith"], json!("branched.status"));

        let close = CloseReservedOriginalDesignerInstruction::new(
            digest(A),
            CapabilityRowId::parse("reserved-original-lease.v1").unwrap(),
        );
        let mut close_json = serde_json::to_value(close).unwrap();
        assert_missing_and_extra_rejected::<CloseReservedOriginalDesignerInstruction>(&close_json);
        close_json["command"] = json!("close designer");
        assert!(!schema_accepts::<CloseReservedOriginalDesignerInstruction>(
            &close_json
        ));
    }

    #[test]
    fn vendor_decision_instruction_uses_the_only_exact_decision_tuple() {
        let instruction = VendorSupportDecisionInstruction::new(blockers());
        let encoded = serde_json::to_value(instruction).unwrap();
        assert_missing_and_extra_rejected::<VendorSupportDecisionInstruction>(&encoded);
        assert_eq!(
            encoded["allowedDecisions"],
            json!([
                "changeTaskScope",
                "useNewerVendorDelivery",
                "safeAbandonment"
            ])
        );
        let mut reordered = encoded;
        reordered["allowedDecisions"] = json!([
            "safeAbandonment",
            "useNewerVendorDelivery",
            "changeTaskScope"
        ]);
        assert!(!schema_accepts::<VendorSupportDecisionInstruction>(
            &reordered
        ));
    }

    #[test]
    fn support_conflict_instruction_fixes_evidence_tuple_and_hashes_every_member() {
        let instruction =
            SupportConflictInstruction::new(id(ID_1), conflicts(), digest(A)).unwrap();
        let encoded = serde_json::to_value(&instruction).unwrap();
        assert_missing_and_extra_rejected::<SupportConflictInstruction>(&encoded);
        assert_eq!(
            encoded["allowedEvidenceKinds"],
            json!([
                "externalCorrectiveVersion",
                "externalSupportOwnershipReceipt"
            ])
        );
        assert_eq!(encoded["automaticReversalForbidden"], json!(true));
        assert_eq!(
            instruction.support_conflict_instruction_digest(),
            &canonical_contract_digest(&instruction.digest_record(), None).unwrap()
        );

        let mut reordered = encoded.clone();
        reordered["allowedEvidenceKinds"] = json!([
            "externalSupportOwnershipReceipt",
            "externalCorrectiveVersion"
        ]);
        assert!(!schema_accepts::<SupportConflictInstruction>(&reordered));

        let mut substituted = encoded;
        substituted["supportConflictInstructionDigest"] = json!(A);
        assert!(schema_accepts::<SupportConflictInstruction>(&substituted));
    }

    #[test]
    fn support_evidence_instruction_derives_missing_kinds_and_exact_digest() {
        let instruction = SupportEvidenceInstruction::new(blockers(), evidence_gaps()).unwrap();
        let encoded = serde_json::to_value(&instruction).unwrap();
        assert_missing_and_extra_rejected::<SupportEvidenceInstruction>(&encoded);
        assert_eq!(
            encoded["missingEvidenceKinds"],
            json!([SupportMissingEvidenceKind::CandidateClassificationUnavailable])
        );
        assert_eq!(
            instruction.support_evidence_instruction_digest(),
            &canonical_contract_digest(&instruction.digest_record(), None).unwrap()
        );

        let mut substituted = encoded;
        substituted["supportEvidenceInstructionDigest"] = json!(A);
        assert!(schema_accepts::<SupportEvidenceInstruction>(&substituted));
    }

    #[test]
    fn record_kind_discriminators_are_exactly_the_eight_normative_literals() {
        let instructions = [
            serde_json::to_value(
                AcquireSupportRootInstruction::new(
                    id(ID_1),
                    SupportActionPurpose::MainIntegrationPrerequisite,
                    ManualSupportTargetMode::ReservedOriginal,
                    username(),
                    None,
                )
                .unwrap(),
            )
            .unwrap(),
            serde_json::to_value(
                ReleaseRepositoryLocksInstruction::new(RequiredNullable::null(), Vec::new())
                    .unwrap(),
            )
            .unwrap(),
            serde_json::to_value(
                ManualSupportInstruction::new(
                    id(ID_1),
                    SupportActionPurpose::MainIntegrationPrerequisite,
                    id(ID_2),
                    digest(A),
                    cursor(),
                    ManualSupportTargetMode::ReservedOriginal,
                    username(),
                    None,
                    transitions(),
                )
                .unwrap(),
            )
            .unwrap(),
            serde_json::to_value(CleanManualWorkingInfobaseInstruction::new(
                working_identity(),
                digest(A),
                CapabilityRowId::parse("manual-lease.v1").unwrap(),
                digest(A),
                ManualWorkingInfobaseCleanupReason::LocalChanges,
            ))
            .unwrap(),
            serde_json::to_value(CloseReservedOriginalDesignerInstruction::new(
                digest(A),
                CapabilityRowId::parse("reserved-original-lease.v1").unwrap(),
            ))
            .unwrap(),
            serde_json::to_value(
                SupportConflictInstruction::new(id(ID_1), conflicts(), digest(A)).unwrap(),
            )
            .unwrap(),
            serde_json::to_value(
                SupportEvidenceInstruction::new(blockers(), evidence_gaps()).unwrap(),
            )
            .unwrap(),
            serde_json::to_value(VendorSupportDecisionInstruction::new(blockers())).unwrap(),
        ];
        assert_eq!(
            instructions
                .iter()
                .map(|instruction| instruction["kind"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec![
                "acquireSupportRoot",
                "releaseRepositoryLocks",
                "manualSupportAction",
                "cleanManualWorkingInfobase",
                "closeReservedOriginalDesigner",
                "coordinateExternalSupportChange",
                "provideSupportEvidence",
                "vendorSupportDecision",
            ]
        );
    }
}
use super::repository::{RepositoryHistoryCursor, RepositoryOwnerIdentity};
use super::scalars::{RepositoryTargetDisplay, RepositoryUsername, RequiredNullable};
use super::schema::one_of_schema;
use super::support::{
    ArmedSupportInstructionProjection, AwaitingSupportInstructionProjection,
    ManualSupportTargetMode, ManualWorkingInfobaseIdentity, SupportActionPurpose, SupportBlockers,
    SupportEvidenceGaps, SupportMissingEvidenceKinds, SupportTransitionConflicts,
    SupportTransitions, VendorSupportDecisions,
};
use crate::domain::branched_development::canonical_json::{
    canonical_contract_digest, contract_digest_record_sealed, ContractDigestRecord,
};
use crate::domain::branched_development::{CapabilityRowId, Sha256Digest, UnicaId};
use schemars::{json_schema, JsonSchema, Schema, SchemaGenerator};
use serde::Serialize;
use std::borrow::Cow;
use std::fmt;

const MAX_INSTRUCTION_ITEMS: usize = 1024;

macro_rules! wire_literal {
    ($name:ident, $wire:literal) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
        enum $name {
            #[serde(rename = $wire)]
            Value,
        }
    };
}

wire_literal!(AcquireSupportRootKind, "acquireSupportRoot");
wire_literal!(ReleaseRepositoryLocksKind, "releaseRepositoryLocks");
wire_literal!(ManualSupportActionKind, "manualSupportAction");
wire_literal!(CleanManualWorkingInfobaseKind, "cleanManualWorkingInfobase");
wire_literal!(
    CloseReservedOriginalDesignerKind,
    "closeReservedOriginalDesigner"
);
wire_literal!(
    CoordinateExternalSupportChangeKind,
    "coordinateExternalSupportChange"
);
wire_literal!(ProvideSupportEvidenceKind, "provideSupportEvidence");
wire_literal!(VendorSupportDecisionKind, "vendorSupportDecision");
wire_literal!(ReservedOriginalMode, "reservedOriginal");
wire_literal!(SeparateWorkingInfobaseMode, "separateWorkingInfobase");
wire_literal!(ConfigurationRootLockTarget, "configurationRoot");
wire_literal!(RepositoryUpdateResume, "repository.update");
wire_literal!(SupportPrerequisiteArmResumeMode, "supportPrerequisiteArm");
wire_literal!(BranchedStatusResume, "branched.status");
wire_literal!(RetainThroughCommitProcedure, "retainThroughCommit");
wire_literal!(
    ExternalCorrectiveVersionEvidenceKind,
    "externalCorrectiveVersion"
);
wire_literal!(
    ExternalSupportOwnershipReceiptEvidenceKind,
    "externalSupportOwnershipReceipt"
);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct InstructionTrueLiteral;

impl Serialize for InstructionTrueLiteral {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_bool(true)
    }
}

impl JsonSchema for InstructionTrueLiteral {
    fn inline_schema() -> bool {
        true
    }

    fn schema_name() -> Cow<'static, str> {
        "InstructionTrueLiteral".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({ "type": "boolean", "const": true })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct InstructionFalseLiteral;

impl Serialize for InstructionFalseLiteral {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_bool(false)
    }
}

impl JsonSchema for InstructionFalseLiteral {
    fn inline_schema() -> bool {
        true
    }

    fn schema_name() -> Cow<'static, str> {
        "InstructionFalseLiteral".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({ "type": "boolean", "const": false })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReservedOriginalAcquireSupportRootInstruction {
    kind: AcquireSupportRootKind,
    support_action_id: UnicaId,
    purpose: SupportActionPurpose,
    manual_target_mode: ReservedOriginalMode,
    repository_username: RepositoryUsername,
    lock_target: ConfigurationRootLockTarget,
    lock_candidate_objects: InstructionFalseLiteral,
    do_not_edit_before_armed: InstructionTrueLiteral,
    do_not_commit_before_armed: InstructionTrueLiteral,
    resume_with: RepositoryUpdateResume,
    resume_mode: SupportPrerequisiteArmResumeMode,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SeparateWorkingInfobaseAcquireSupportRootInstruction {
    kind: AcquireSupportRootKind,
    support_action_id: UnicaId,
    purpose: SupportActionPurpose,
    manual_target_mode: SeparateWorkingInfobaseMode,
    repository_username: RepositoryUsername,
    working_infobase_identity: ManualWorkingInfobaseIdentity,
    lock_target: ConfigurationRootLockTarget,
    lock_candidate_objects: InstructionFalseLiteral,
    do_not_edit_before_armed: InstructionTrueLiteral,
    do_not_commit_before_armed: InstructionTrueLiteral,
    resume_with: RepositoryUpdateResume,
    resume_mode: SupportPrerequisiteArmResumeMode,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct AcquireSupportRootInstruction(AcquireSupportRootInstructionVariant);

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
enum AcquireSupportRootInstructionVariant {
    ReservedOriginal(ReservedOriginalAcquireSupportRootInstruction),
    SeparateWorkingInfobase(SeparateWorkingInfobaseAcquireSupportRootInstruction),
}

impl AcquireSupportRootInstruction {
    pub(crate) fn from_awaiting_projection(
        projection: &AwaitingSupportInstructionProjection,
    ) -> Result<Self, InstructionContractError> {
        Self::build(
            projection.support_action_id().clone(),
            projection.purpose(),
            projection.manual_target_mode(),
            projection.manual_actor_username().clone(),
            projection.working_infobase_identity().cloned(),
        )
    }

    #[cfg(test)]
    pub(crate) fn new(
        support_action_id: UnicaId,
        purpose: SupportActionPurpose,
        manual_target_mode: ManualSupportTargetMode,
        repository_username: RepositoryUsername,
        working_infobase_identity: Option<ManualWorkingInfobaseIdentity>,
    ) -> Result<Self, InstructionContractError> {
        Self::build(
            support_action_id,
            purpose,
            manual_target_mode,
            repository_username,
            working_infobase_identity,
        )
    }

    fn build(
        support_action_id: UnicaId,
        purpose: SupportActionPurpose,
        manual_target_mode: ManualSupportTargetMode,
        repository_username: RepositoryUsername,
        working_infobase_identity: Option<ManualWorkingInfobaseIdentity>,
    ) -> Result<Self, InstructionContractError> {
        match (manual_target_mode, working_infobase_identity) {
            (ManualSupportTargetMode::ReservedOriginal, None) => Ok(Self(
                AcquireSupportRootInstructionVariant::ReservedOriginal(
                    ReservedOriginalAcquireSupportRootInstruction {
                        kind: AcquireSupportRootKind::Value,
                        support_action_id,
                        purpose,
                        manual_target_mode: ReservedOriginalMode::Value,
                        repository_username,
                        lock_target: ConfigurationRootLockTarget::Value,
                        lock_candidate_objects: InstructionFalseLiteral,
                        do_not_edit_before_armed: InstructionTrueLiteral,
                        do_not_commit_before_armed: InstructionTrueLiteral,
                        resume_with: RepositoryUpdateResume::Value,
                        resume_mode: SupportPrerequisiteArmResumeMode::Value,
                    },
                ),
            )),
            (ManualSupportTargetMode::SeparateWorkingInfobase, Some(identity)) => Ok(Self(
                AcquireSupportRootInstructionVariant::SeparateWorkingInfobase(
                    SeparateWorkingInfobaseAcquireSupportRootInstruction {
                        kind: AcquireSupportRootKind::Value,
                        support_action_id,
                        purpose,
                        manual_target_mode: SeparateWorkingInfobaseMode::Value,
                        repository_username,
                        working_infobase_identity: identity,
                        lock_target: ConfigurationRootLockTarget::Value,
                        lock_candidate_objects: InstructionFalseLiteral,
                        do_not_edit_before_armed: InstructionTrueLiteral,
                        do_not_commit_before_armed: InstructionTrueLiteral,
                        resume_with: RepositoryUpdateResume::Value,
                        resume_mode: SupportPrerequisiteArmResumeMode::Value,
                    },
                ),
            )),
            _ => Err(InstructionContractError(
                "acquire-root working-infobase presence disagrees with manual target mode",
            )),
        }
    }
}

impl JsonSchema for AcquireSupportRootInstruction {
    fn schema_name() -> Cow<'static, str> {
        "AcquireSupportRootInstruction".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<ReservedOriginalAcquireSupportRootInstruction>(),
            generator.subschema_for::<SeparateWorkingInfobaseAcquireSupportRootInstruction>(),
        ])
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
struct RepositoryTargetDisplays(Vec<RepositoryTargetDisplay>);

impl RepositoryTargetDisplays {
    fn new(values: Vec<RepositoryTargetDisplay>) -> Result<Self, InstructionContractError> {
        if values.len() > MAX_INSTRUCTION_ITEMS {
            return Err(InstructionContractError(
                "release-lock object displays exceed the instruction collection bound",
            ));
        }
        if !values.windows(2).all(|pair| pair[0] < pair[1]) {
            return Err(InstructionContractError(
                "release-lock object displays must be Unicode-scalar ordered and duplicate-free",
            ));
        }
        Ok(Self(values))
    }
}

impl JsonSchema for RepositoryTargetDisplays {
    fn schema_name() -> Cow<'static, str> {
        "ReleaseRepositoryLockObjectDisplays".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        let item = generator.subschema_for::<RepositoryTargetDisplay>();
        json_schema!({
            "type": "array",
            "items": item,
            "maxItems": MAX_INSTRUCTION_ITEMS,
            "uniqueItems": true,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct ReleaseRepositoryLocksInstructionDigestRecord {
    kind: ReleaseRepositoryLocksKind,
    owner: RequiredNullable<RepositoryOwnerIdentity>,
    object_displays: RepositoryTargetDisplays,
    coordination_required: InstructionTrueLiteral,
    resume_with: BranchedStatusResume,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ReleaseRepositoryLocksInstruction {
    kind: ReleaseRepositoryLocksKind,
    owner: RequiredNullable<RepositoryOwnerIdentity>,
    object_displays: RepositoryTargetDisplays,
    coordination_required: InstructionTrueLiteral,
    resume_with: BranchedStatusResume,
    lock_instruction_digest: Sha256Digest,
}

impl contract_digest_record_sealed::Sealed for ReleaseRepositoryLocksInstructionDigestRecord {}
impl ContractDigestRecord for ReleaseRepositoryLocksInstructionDigestRecord {}

impl ReleaseRepositoryLocksInstruction {
    pub(crate) fn new(
        owner: RequiredNullable<RepositoryOwnerIdentity>,
        object_displays: Vec<RepositoryTargetDisplay>,
    ) -> Result<Self, InstructionContractError> {
        let record = ReleaseRepositoryLocksInstructionDigestRecord {
            kind: ReleaseRepositoryLocksKind::Value,
            owner,
            object_displays: RepositoryTargetDisplays::new(object_displays)?,
            coordination_required: InstructionTrueLiteral,
            resume_with: BranchedStatusResume::Value,
        };
        let lock_instruction_digest = instruction_digest(
            &record,
            "release-lock instruction digest computation failed",
        )?;
        Ok(Self::from_record(record, lock_instruction_digest))
    }

    fn from_record(
        record: ReleaseRepositoryLocksInstructionDigestRecord,
        lock_instruction_digest: Sha256Digest,
    ) -> Self {
        Self {
            kind: record.kind,
            owner: record.owner,
            object_displays: record.object_displays,
            coordination_required: record.coordination_required,
            resume_with: record.resume_with,
            lock_instruction_digest,
        }
    }

    pub(super) fn digest_record(&self) -> ReleaseRepositoryLocksInstructionDigestRecord {
        ReleaseRepositoryLocksInstructionDigestRecord {
            kind: self.kind,
            owner: self.owner.clone(),
            object_displays: self.object_displays.clone(),
            coordination_required: self.coordination_required,
            resume_with: self.resume_with,
        }
    }

    pub(super) fn lock_instruction_digest(&self) -> &Sha256Digest {
        &self.lock_instruction_digest
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReservedOriginalManualSupportInstruction {
    kind: ManualSupportActionKind,
    support_action_id: UnicaId,
    purpose: SupportActionPurpose,
    arming_receipt_id: UnicaId,
    arming_receipt_digest: Sha256Digest,
    arming_cursor: RepositoryHistoryCursor,
    manual_target_mode: ReservedOriginalMode,
    repository_username: RepositoryUsername,
    root_already_locked: InstructionTrueLiteral,
    requested_root_lock_procedure: RetainThroughCommitProcedure,
    lock_candidate_objects: InstructionFalseLiteral,
    authorized_version_must_be_first_root_support_after_cursor: InstructionTrueLiteral,
    transitions: SupportTransitions,
    commit_as_separate_root_version: InstructionTrueLiteral,
    release_root: InstructionTrueLiteral,
    close_reserved_original_designer_session: InstructionTrueLiteral,
    resume_with: BranchedStatusResume,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SeparateWorkingInfobaseManualSupportInstruction {
    kind: ManualSupportActionKind,
    support_action_id: UnicaId,
    purpose: SupportActionPurpose,
    arming_receipt_id: UnicaId,
    arming_receipt_digest: Sha256Digest,
    arming_cursor: RepositoryHistoryCursor,
    manual_target_mode: SeparateWorkingInfobaseMode,
    repository_username: RepositoryUsername,
    working_infobase_identity: ManualWorkingInfobaseIdentity,
    root_already_locked: InstructionTrueLiteral,
    requested_root_lock_procedure: RetainThroughCommitProcedure,
    lock_candidate_objects: InstructionFalseLiteral,
    authorized_version_must_be_first_root_support_after_cursor: InstructionTrueLiteral,
    transitions: SupportTransitions,
    commit_as_separate_root_version: InstructionTrueLiteral,
    release_root: InstructionTrueLiteral,
    close_working_infobase_session: InstructionTrueLiteral,
    resume_with: BranchedStatusResume,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct ManualSupportInstruction(ManualSupportInstructionVariant);

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
enum ManualSupportInstructionVariant {
    ReservedOriginal(ReservedOriginalManualSupportInstruction),
    SeparateWorkingInfobase(SeparateWorkingInfobaseManualSupportInstruction),
}

impl ManualSupportInstruction {
    pub(crate) fn from_armed_projection(
        projection: &ArmedSupportInstructionProjection,
    ) -> Result<Self, InstructionContractError> {
        Self::build(
            projection.support_action_id().clone(),
            projection.purpose(),
            projection.arming_receipt_id().clone(),
            projection.arming_receipt_digest().clone(),
            projection.arming_cursor().clone(),
            projection.manual_target_mode(),
            projection.manual_actor_username().clone(),
            projection.working_infobase_identity().cloned(),
            projection.authorized_transitions().clone(),
        )
    }

    #[allow(clippy::too_many_arguments)]
    #[cfg(test)]
    pub(crate) fn new(
        support_action_id: UnicaId,
        purpose: SupportActionPurpose,
        arming_receipt_id: UnicaId,
        arming_receipt_digest: Sha256Digest,
        arming_cursor: RepositoryHistoryCursor,
        manual_target_mode: ManualSupportTargetMode,
        repository_username: RepositoryUsername,
        working_infobase_identity: Option<ManualWorkingInfobaseIdentity>,
        transitions: SupportTransitions,
    ) -> Result<Self, InstructionContractError> {
        Self::build(
            support_action_id,
            purpose,
            arming_receipt_id,
            arming_receipt_digest,
            arming_cursor,
            manual_target_mode,
            repository_username,
            working_infobase_identity,
            transitions,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn build(
        support_action_id: UnicaId,
        purpose: SupportActionPurpose,
        arming_receipt_id: UnicaId,
        arming_receipt_digest: Sha256Digest,
        arming_cursor: RepositoryHistoryCursor,
        manual_target_mode: ManualSupportTargetMode,
        repository_username: RepositoryUsername,
        working_infobase_identity: Option<ManualWorkingInfobaseIdentity>,
        transitions: SupportTransitions,
    ) -> Result<Self, InstructionContractError> {
        match (manual_target_mode, working_infobase_identity) {
            (ManualSupportTargetMode::ReservedOriginal, None) => {
                Ok(Self(ManualSupportInstructionVariant::ReservedOriginal(
                    ReservedOriginalManualSupportInstruction {
                        kind: ManualSupportActionKind::Value,
                        support_action_id,
                        purpose,
                        arming_receipt_id,
                        arming_receipt_digest,
                        arming_cursor,
                        manual_target_mode: ReservedOriginalMode::Value,
                        repository_username,
                        root_already_locked: InstructionTrueLiteral,
                        requested_root_lock_procedure: RetainThroughCommitProcedure::Value,
                        lock_candidate_objects: InstructionFalseLiteral,
                        authorized_version_must_be_first_root_support_after_cursor:
                            InstructionTrueLiteral,
                        transitions,
                        commit_as_separate_root_version: InstructionTrueLiteral,
                        release_root: InstructionTrueLiteral,
                        close_reserved_original_designer_session: InstructionTrueLiteral,
                        resume_with: BranchedStatusResume::Value,
                    },
                )))
            }
            (ManualSupportTargetMode::SeparateWorkingInfobase, Some(identity)) => Ok(Self(
                ManualSupportInstructionVariant::SeparateWorkingInfobase(
                    SeparateWorkingInfobaseManualSupportInstruction {
                        kind: ManualSupportActionKind::Value,
                        support_action_id,
                        purpose,
                        arming_receipt_id,
                        arming_receipt_digest,
                        arming_cursor,
                        manual_target_mode: SeparateWorkingInfobaseMode::Value,
                        repository_username,
                        working_infobase_identity: identity,
                        root_already_locked: InstructionTrueLiteral,
                        requested_root_lock_procedure: RetainThroughCommitProcedure::Value,
                        lock_candidate_objects: InstructionFalseLiteral,
                        authorized_version_must_be_first_root_support_after_cursor:
                            InstructionTrueLiteral,
                        transitions,
                        commit_as_separate_root_version: InstructionTrueLiteral,
                        release_root: InstructionTrueLiteral,
                        close_working_infobase_session: InstructionTrueLiteral,
                        resume_with: BranchedStatusResume::Value,
                    },
                ),
            )),
            _ => Err(InstructionContractError(
                "manual-support working-infobase presence disagrees with manual target mode",
            )),
        }
    }
}

impl JsonSchema for ManualSupportInstruction {
    fn schema_name() -> Cow<'static, str> {
        "ManualSupportInstruction".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<ReservedOriginalManualSupportInstruction>(),
            generator.subschema_for::<SeparateWorkingInfobaseManualSupportInstruction>(),
        ])
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) enum ManualWorkingInfobaseCleanupReason {
    LeaseBusy,
    LocalChanges,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CleanManualWorkingInfobaseInstruction {
    kind: CleanManualWorkingInfobaseKind,
    working_infobase_identity: ManualWorkingInfobaseIdentity,
    closure_plan_digest: Sha256Digest,
    exclusive_lease_capability_id: CapabilityRowId,
    expected_repository_fingerprint: Sha256Digest,
    reason: ManualWorkingInfobaseCleanupReason,
    close_designer_session: InstructionTrueLiteral,
    resume_with: BranchedStatusResume,
}

impl CleanManualWorkingInfobaseInstruction {
    #[cfg(test)]
    pub(crate) fn new(
        working_infobase_identity: ManualWorkingInfobaseIdentity,
        closure_plan_digest: Sha256Digest,
        exclusive_lease_capability_id: CapabilityRowId,
        expected_repository_fingerprint: Sha256Digest,
        reason: ManualWorkingInfobaseCleanupReason,
    ) -> Self {
        Self {
            kind: CleanManualWorkingInfobaseKind::Value,
            working_infobase_identity,
            closure_plan_digest,
            exclusive_lease_capability_id,
            expected_repository_fingerprint,
            reason,
            close_designer_session: InstructionTrueLiteral,
            resume_with: BranchedStatusResume::Value,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CloseReservedOriginalDesignerInstruction {
    kind: CloseReservedOriginalDesignerKind,
    reserved_original_identity_digest: Sha256Digest,
    exclusive_lease_capability_id: CapabilityRowId,
    close_designer_session: InstructionTrueLiteral,
    resume_with: BranchedStatusResume,
}

impl CloseReservedOriginalDesignerInstruction {
    #[cfg(test)]
    pub(crate) fn new(
        reserved_original_identity_digest: Sha256Digest,
        exclusive_lease_capability_id: CapabilityRowId,
    ) -> Self {
        Self {
            kind: CloseReservedOriginalDesignerKind::Value,
            reserved_original_identity_digest,
            exclusive_lease_capability_id,
            close_designer_session: InstructionTrueLiteral,
            resume_with: BranchedStatusResume::Value,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
struct SupportConflictAllowedEvidenceKinds([SupportConflictAllowedEvidenceKind; 2]);

impl SupportConflictAllowedEvidenceKinds {
    const fn canonical() -> Self {
        Self([
            SupportConflictAllowedEvidenceKind::ExternalCorrectiveVersion(
                ExternalCorrectiveVersionEvidenceKind::Value,
            ),
            SupportConflictAllowedEvidenceKind::ExternalSupportOwnershipReceipt(
                ExternalSupportOwnershipReceiptEvidenceKind::Value,
            ),
        ])
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(untagged)]
enum SupportConflictAllowedEvidenceKind {
    ExternalCorrectiveVersion(ExternalCorrectiveVersionEvidenceKind),
    ExternalSupportOwnershipReceipt(ExternalSupportOwnershipReceiptEvidenceKind),
}

impl JsonSchema for SupportConflictAllowedEvidenceKinds {
    fn schema_name() -> Cow<'static, str> {
        "SupportConflictAllowedEvidenceKinds".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        let external_corrective_version =
            generator.subschema_for::<ExternalCorrectiveVersionEvidenceKind>();
        let external_support_ownership_receipt =
            generator.subschema_for::<ExternalSupportOwnershipReceiptEvidenceKind>();
        json_schema!({
            "type": "array",
            "prefixItems": [
                external_corrective_version,
                external_support_ownership_receipt,
            ],
            "items": false,
            "minItems": 2,
            "maxItems": 2,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct SupportConflictInstructionDigestRecord {
    kind: CoordinateExternalSupportChangeKind,
    conflict_resolution_id: UnicaId,
    conflicts: SupportTransitionConflicts,
    allowed_evidence_kinds: SupportConflictAllowedEvidenceKinds,
    required_final_baseline_digest: Sha256Digest,
    automatic_reversal_forbidden: InstructionTrueLiteral,
    resume_with: BranchedStatusResume,
}

impl contract_digest_record_sealed::Sealed for SupportConflictInstructionDigestRecord {}
impl ContractDigestRecord for SupportConflictInstructionDigestRecord {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SupportConflictInstruction {
    kind: CoordinateExternalSupportChangeKind,
    conflict_resolution_id: UnicaId,
    conflicts: SupportTransitionConflicts,
    allowed_evidence_kinds: SupportConflictAllowedEvidenceKinds,
    required_final_baseline_digest: Sha256Digest,
    automatic_reversal_forbidden: InstructionTrueLiteral,
    resume_with: BranchedStatusResume,
    support_conflict_instruction_digest: Sha256Digest,
}

impl SupportConflictInstruction {
    pub(crate) fn new(
        conflict_resolution_id: UnicaId,
        conflicts: SupportTransitionConflicts,
        required_final_baseline_digest: Sha256Digest,
    ) -> Result<Self, InstructionContractError> {
        let record = SupportConflictInstructionDigestRecord {
            kind: CoordinateExternalSupportChangeKind::Value,
            conflict_resolution_id,
            conflicts,
            allowed_evidence_kinds: SupportConflictAllowedEvidenceKinds::canonical(),
            required_final_baseline_digest,
            automatic_reversal_forbidden: InstructionTrueLiteral,
            resume_with: BranchedStatusResume::Value,
        };
        let support_conflict_instruction_digest = instruction_digest(
            &record,
            "support-conflict instruction digest computation failed",
        )?;
        Ok(Self::from_record(
            record,
            support_conflict_instruction_digest,
        ))
    }

    fn from_record(
        record: SupportConflictInstructionDigestRecord,
        support_conflict_instruction_digest: Sha256Digest,
    ) -> Self {
        Self {
            kind: record.kind,
            conflict_resolution_id: record.conflict_resolution_id,
            conflicts: record.conflicts,
            allowed_evidence_kinds: record.allowed_evidence_kinds,
            required_final_baseline_digest: record.required_final_baseline_digest,
            automatic_reversal_forbidden: record.automatic_reversal_forbidden,
            resume_with: record.resume_with,
            support_conflict_instruction_digest,
        }
    }

    pub(super) fn digest_record(&self) -> SupportConflictInstructionDigestRecord {
        SupportConflictInstructionDigestRecord {
            kind: self.kind,
            conflict_resolution_id: self.conflict_resolution_id.clone(),
            conflicts: self.conflicts.clone(),
            allowed_evidence_kinds: self.allowed_evidence_kinds,
            required_final_baseline_digest: self.required_final_baseline_digest.clone(),
            automatic_reversal_forbidden: self.automatic_reversal_forbidden,
            resume_with: self.resume_with,
        }
    }

    pub(super) fn support_conflict_instruction_digest(&self) -> &Sha256Digest {
        &self.support_conflict_instruction_digest
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct SupportEvidenceInstructionDigestRecord {
    kind: ProvideSupportEvidenceKind,
    blockers: SupportBlockers,
    evidence_gaps: SupportEvidenceGaps,
    missing_evidence_kinds: SupportMissingEvidenceKinds,
    resume_with: BranchedStatusResume,
}

impl contract_digest_record_sealed::Sealed for SupportEvidenceInstructionDigestRecord {}
impl ContractDigestRecord for SupportEvidenceInstructionDigestRecord {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SupportEvidenceInstruction {
    kind: ProvideSupportEvidenceKind,
    blockers: SupportBlockers,
    evidence_gaps: SupportEvidenceGaps,
    missing_evidence_kinds: SupportMissingEvidenceKinds,
    resume_with: BranchedStatusResume,
    support_evidence_instruction_digest: Sha256Digest,
}

impl SupportEvidenceInstruction {
    pub(crate) fn new(
        blockers: SupportBlockers,
        evidence_gaps: SupportEvidenceGaps,
    ) -> Result<Self, InstructionContractError> {
        let record = SupportEvidenceInstructionDigestRecord {
            kind: ProvideSupportEvidenceKind::Value,
            blockers,
            missing_evidence_kinds: evidence_gaps.missing_evidence_kinds(),
            evidence_gaps,
            resume_with: BranchedStatusResume::Value,
        };
        let support_evidence_instruction_digest = instruction_digest(
            &record,
            "support-evidence instruction digest computation failed",
        )?;
        Ok(Self::from_record(
            record,
            support_evidence_instruction_digest,
        ))
    }

    fn from_record(
        record: SupportEvidenceInstructionDigestRecord,
        support_evidence_instruction_digest: Sha256Digest,
    ) -> Self {
        Self {
            kind: record.kind,
            blockers: record.blockers,
            evidence_gaps: record.evidence_gaps,
            missing_evidence_kinds: record.missing_evidence_kinds,
            resume_with: record.resume_with,
            support_evidence_instruction_digest,
        }
    }

    pub(super) fn digest_record(&self) -> SupportEvidenceInstructionDigestRecord {
        SupportEvidenceInstructionDigestRecord {
            kind: self.kind,
            blockers: self.blockers.clone(),
            evidence_gaps: self.evidence_gaps.clone(),
            missing_evidence_kinds: self.missing_evidence_kinds.clone(),
            resume_with: self.resume_with,
        }
    }

    pub(super) fn support_evidence_instruction_digest(&self) -> &Sha256Digest {
        &self.support_evidence_instruction_digest
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct VendorSupportDecisionInstruction {
    kind: VendorSupportDecisionKind,
    blockers: SupportBlockers,
    allowed_decisions: VendorSupportDecisions,
}

impl VendorSupportDecisionInstruction {
    pub(crate) fn new(blockers: SupportBlockers) -> Self {
        Self {
            kind: VendorSupportDecisionKind::Value,
            blockers,
            allowed_decisions: VendorSupportDecisions::all(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct InstructionContractError(&'static str);

impl fmt::Display for InstructionContractError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.0)
    }
}

impl std::error::Error for InstructionContractError {}

fn instruction_digest<T: ContractDigestRecord>(
    record: &T,
    failure: &'static str,
) -> Result<Sha256Digest, InstructionContractError> {
    canonical_contract_digest(record, None).map_err(|_| InstructionContractError(failure))
}
