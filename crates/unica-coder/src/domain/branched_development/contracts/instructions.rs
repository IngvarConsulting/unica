#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::branched_development::contracts::repository::RepositoryUpdateLockReason;
    use crate::domain::branched_development::contracts::scalars::{
        Diagnostic, DisplayPath, RepositoryIdentityComponent, RepositoryVersion,
    };
    use crate::domain::branched_development::contracts::schema::audit_json_schema;
    use crate::domain::branched_development::contracts::support::{
        SupportContractError, SupportEvidenceGap, SupportHistoryOrderAuthority,
        SupportMissingEvidenceKind, SupportRecoveryDisposition, SupportRecoveryDistributionHandoff,
        SupportRecoveryDistributionHandoffInputs, SupportRecoveryHandoffRevalidation,
        SupportTransition, SupportTransitionConflict, SupportTransitionOverlapKind,
        UserVisibleCfFileName,
    };
    use crate::domain::branched_development::contracts::support_terminalization::{
        SupportRecoveryDesiredTarget, SupportRecoveryDesiredTargets,
        SupportRecoveryFinalizationPlanAuthority,
    };
    use crate::domain::branched_development::{
        MetadataObjectId, ProfileArtifactRefId, SupportLayerId,
    };
    use schemars::{schema_for, JsonSchema};
    use serde::de::DeserializeOwned;
    use serde_json::{json, Value};
    use sha2::{Digest, Sha256};
    use std::cmp::Ordering;

    const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const C: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
    const ID_1: &str = "11111111-1111-4111-8111-111111111111";
    const ID_2: &str = "22222222-2222-4222-8222-222222222222";
    const ID_3: &str = "33333333-3333-4333-8333-333333333333";
    const OBJECT_A: &str = "00000000-0000-0000-0000-000000000001";

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

    fn recovery_root_lock() -> SupportRecoveryLockTarget {
        SupportRecoveryLockTarget::configuration_root(
            RepositoryTargetDisplay::parse("Configuration").unwrap(),
            vec![
                RepositoryUpdateLockReason::SupportGraphGuard,
                RepositoryUpdateLockReason::UpdateTarget,
            ],
        )
        .unwrap()
    }

    fn recovery_object_lock() -> SupportRecoveryLockTarget {
        SupportRecoveryLockTarget::development_object(
            MetadataObjectId::parse(OBJECT_A).unwrap(),
            RepositoryTargetDisplay::parse("Catalog.A").unwrap(),
            vec![RepositoryUpdateLockReason::UpdateTarget],
        )
        .unwrap()
    }

    fn recovery_locks(include_object: bool) -> SupportRecoveryLockTargets {
        let mut values = vec![recovery_root_lock()];
        if include_object {
            values.push(recovery_object_lock());
        }
        SupportRecoveryLockTargets::new(values).unwrap()
    }

    fn finalization_plan(
        lock_targets: SupportRecoveryLockTargets,
        root_display: &str,
        object_display: Option<&str>,
        desired_support_graph_digest: Sha256Digest,
        desired_repository_content_digest: Sha256Digest,
    ) -> SupportRecoveryFinalizationPlan {
        let mut desired_targets = vec![SupportRecoveryDesiredTarget::root_present(
            RepositoryTargetDisplay::parse(root_display).unwrap(),
            digest(A),
        )];
        if let Some(object_display) = object_display {
            desired_targets.push(SupportRecoveryDesiredTarget::object_present(
                MetadataObjectId::parse(OBJECT_A).unwrap(),
                RepositoryTargetDisplay::parse(object_display).unwrap(),
                digest(A),
            ));
        }
        SupportRecoveryFinalizationPlan::new(
            SupportRecoveryFinalizationPlanAuthority::desired_test_only(
                SupportRecoveryDisposition::RestoreThenReauthorize,
                lock_targets,
                SupportRecoveryDesiredTargets::new(desired_targets).unwrap(),
                cursor(),
                desired_support_graph_digest,
                desired_repository_content_digest,
            ),
        )
        .unwrap()
    }

    #[derive(Clone)]
    struct FixedCorrectiveLockClosureResolver {
        expected_transition_count: usize,
        expected_restoration_count: usize,
        result: Result<SupportRecoveryLockTargets, SupportCorrectiveLockClosureResolutionError>,
    }

    impl SupportCorrectiveLockClosureResolver for FixedCorrectiveLockClosureResolver {
        fn resolve_correction_lock_targets(
            &self,
            required_root_transitions: &[SupportRecoveryTransition],
            required_content_restorations: &[SupportContentRestoration],
        ) -> Result<SupportRecoveryLockTargets, SupportCorrectiveLockClosureResolutionError>
        {
            if required_root_transitions.len() != self.expected_transition_count
                || required_content_restorations.len() != self.expected_restoration_count
            {
                return Err(SupportCorrectiveLockClosureResolutionError::Unavailable);
            }
            self.result.clone()
        }
    }

    fn object_restoration() -> SupportContentRestoration {
        let object_id = MetadataObjectId::parse(OBJECT_A).unwrap();
        SupportContentRestoration::restore_existing_object_test_only(
            object_id.clone(),
            RepositoryTargetDisplay::parse("Catalog.A").unwrap(),
            cursor(),
            digest(A),
            digest(A),
            vec![object_id.clone()],
            vec![object_id],
            digest(A),
        )
    }

    fn recovery_handoff(mode: ManualSupportTargetMode) -> SupportRecoveryDistributionHandoff {
        SupportRecoveryDistributionHandoff::new(
            mode,
            (mode == ManualSupportTargetMode::SeparateWorkingInfobase).then(working_identity),
            SupportRecoveryDistributionHandoffInputs {
                handoff_id: id(ID_1),
                profile_artifact_ref_id: ProfileArtifactRefId::parse("vendor.layer-a").unwrap(),
                profile_artifact_display: DisplayPath::parse("Vendor layer A").unwrap(),
                user_visible_file_name: UserVisibleCfFileName::parse("vendor-layer-a.cf").unwrap(),
                manual_actor_username: username(),
                layer_id: SupportLayerId::parse("layer-a").unwrap(),
                distribution_artifact_id: id(ID_2),
                artifact_sha256: digest(A),
                readability_probe_receipt_id: id(ID_3),
                manual_readability_capability_row_id: CapabilityRowId::parse(
                    "manual-readability.v1",
                )
                .unwrap(),
                retention_lease_id: id(ID_2),
                retention_receipt_id: id(ID_3),
                retention_capability_row_id: CapabilityRowId::parse("retention-provider.v1")
                    .unwrap(),
            },
        )
        .unwrap()
    }

    fn handoff_revalidation(
        handoff: &SupportRecoveryDistributionHandoff,
    ) -> SupportRecoveryHandoffRevalidation {
        SupportRecoveryHandoffRevalidation::new(
            handoff,
            digest(A),
            id(ID_3),
            CapabilityRowId::parse("manual-readability.v1").unwrap(),
            CapabilityRowId::parse("retention-provider.v1").unwrap(),
        )
        .unwrap()
    }

    fn corrective_instruction(
        mode: ManualSupportTargetMode,
        working_infobase_identity: Option<ManualWorkingInfobaseIdentity>,
    ) -> SupportCorrectiveInstruction {
        SupportCorrectiveInstruction::new(
            SupportCorrectiveInstructionAuthority::test_only(
                id(ID_1),
                SupportActionPurpose::AbandonmentCleanup,
                mode,
                username(),
                working_infobase_identity,
                cursor(),
                recovery_locks(false),
                recovery_locks(false),
                vec![SupportRecoveryTransition::ordinary(
                    SupportTransition::enable_configuration_changes(
                        RepositoryTargetDisplay::parse("Configuration").unwrap(),
                        SupportLayerId::parse("layer-a").unwrap(),
                    ),
                )],
                Vec::new(),
                Vec::new(),
                Vec::new(),
                digest(A),
                digest(A),
            )
            .unwrap(),
        )
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

    struct OpaqueZBeforeOpaqueAHistoryOrder;

    impl SupportHistoryOrderAuthority for OpaqueZBeforeOpaqueAHistoryOrder {
        fn compare_versions(
            &self,
            left: &RepositoryVersion,
            right: &RepositoryVersion,
        ) -> Result<Ordering, SupportContractError> {
            match (left.as_str(), right.as_str()) {
                ("opaque-z", "opaque-a") => Ok(Ordering::Less),
                ("opaque-a", "opaque-z") => Ok(Ordering::Greater),
                (left, right) if left == right => Ok(Ordering::Equal),
                _ => panic!("unexpected version in test history authority"),
            }
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

    fn conflicts_in_proven_history_order() -> SupportTransitionConflicts {
        let transition = SupportTransition::enable_configuration_changes(
            RepositoryTargetDisplay::parse("Configuration").unwrap(),
            SupportLayerId::parse("layer-a").unwrap(),
        );
        let first = SupportTransitionConflict::from_capability_adapter(
            RepositoryVersion::parse("opaque-z").unwrap(),
            RequiredNullable::null(),
            None,
            RepositoryTargetDisplay::parse("Configuration").unwrap(),
            SupportLayerId::parse("layer-a").unwrap(),
            transition.clone(),
            digest(A),
            SupportTransitionOverlapKind::SameTarget,
            Diagnostic::parse("first").unwrap(),
        )
        .unwrap();
        let second = SupportTransitionConflict::from_capability_adapter(
            RepositoryVersion::parse("opaque-a").unwrap(),
            RequiredNullable::null(),
            None,
            RepositoryTargetDisplay::parse("Configuration").unwrap(),
            SupportLayerId::parse("layer-a").unwrap(),
            transition,
            digest(B),
            SupportTransitionOverlapKind::SameTarget,
            Diagnostic::parse("second").unwrap(),
        )
        .unwrap();
        SupportTransitionConflicts::new(vec![first, second], &OpaqueZBeforeOpaqueAHistoryOrder)
            .unwrap()
    }

    fn recompute_support_conflict_instruction_digest(value: &mut Value) {
        value
            .as_object_mut()
            .unwrap()
            .remove("supportConflictInstructionDigest");
        let digest = format!(
            "{:x}",
            Sha256::digest(serde_json_canonicalizer::to_vec(&*value).unwrap())
        );
        value["supportConflictInstructionDigest"] = json!(digest);
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
    assert_not_deserialize_owned!(SupportCorrectiveInstructionAuthority);
    assert_not_deserialize_owned!(FixedCorrectiveLockClosureResolver);
    assert_not_deserialize_owned!(SupportCorrectiveLockClosureResolutionError);
    assert_not_deserialize_owned!(SupportRecoveryExternalAction);

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

        let mut substituted = encoded.clone();
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

        let mut substituted = encoded.clone();
        substituted["supportConflictInstructionDigest"] = json!(A);
        assert!(schema_accepts::<SupportConflictInstruction>(&substituted));
        assert!(
            decode_historical_support_conflict_instruction(substituted, &TrivialHistoryOrder,)
                .is_err()
        );

        let round_trip = serde_json::to_value(
            decode_historical_support_conflict_instruction(encoded.clone(), &TrivialHistoryOrder)
                .unwrap(),
        )
        .unwrap();
        assert_eq!(round_trip, encoded);
        let decoded =
            decode_historical_support_conflict_instruction(encoded, &TrivialHistoryOrder).unwrap();
        assert_eq!(decoded.conflict_resolution_id(), &id(ID_1));
        assert_eq!(decoded.required_final_baseline_digest(), &digest(A));
        assert_eq!(
            decoded.support_conflict_instruction_digest(),
            instruction.support_conflict_instruction_digest()
        );
    }

    #[test]
    fn historical_conflict_instruction_requires_independently_proven_history_order() {
        let instruction = SupportConflictInstruction::new(
            id(ID_1),
            conflicts_in_proven_history_order(),
            digest(C),
        )
        .unwrap();
        let encoded = serde_json::to_value(&instruction).unwrap();
        assert!(decode_historical_support_conflict_instruction(
            encoded.clone(),
            &OpaqueZBeforeOpaqueAHistoryOrder,
        )
        .is_ok());

        let mut reversed = encoded;
        reversed["conflicts"].as_array_mut().unwrap().reverse();
        recompute_support_conflict_instruction_digest(&mut reversed);
        assert!(schema_accepts::<SupportConflictInstruction>(&reversed));
        assert!(decode_historical_support_conflict_instruction(
            reversed,
            &OpaqueZBeforeOpaqueAHistoryOrder,
        )
        .is_err());
    }

    #[test]
    fn corrective_instruction_is_exact_mode_bound_digest_checked_and_canonical() {
        let instruction = corrective_instruction(ManualSupportTargetMode::ReservedOriginal, None);
        let encoded = serde_json::to_value(&instruction).unwrap();
        assert_eq!(encoded["kind"], "correctSupportPrerequisite");
        assert_eq!(encoded["manualTargetMode"], "reservedOriginal");
        assert!(encoded.get("workingInfobaseIdentity").is_none());
        assert_eq!(encoded["offSupportForbidden"], true);
        assert_eq!(encoded["commitAsSeparateRecoveryVersion"], true);
        assert_eq!(encoded["releaseAllLocks"], true);
        assert_eq!(encoded["resumeWith"], "branched.status");
        let encoded_root_transitions = serde_json::from_value::<SupportRecoveryTransitions>(
            encoded["requiredRootTransitions"].clone(),
        )
        .unwrap();
        let encoded_content_restorations = serde_json::from_value::<SupportContentRestorations>(
            encoded["requiredContentRestorations"].clone(),
        )
        .unwrap();
        assert_eq!(
            instruction.required_root_delta_digest(),
            &canonical_contract_digest(
                &SupportRequiredRootDeltaDigestRecord {
                    required_root_transitions: encoded_root_transitions,
                },
                None,
            )
            .unwrap()
        );
        assert_eq!(
            instruction.required_content_delta_digest(),
            &canonical_contract_digest(
                &SupportRequiredContentDeltaDigestRecord {
                    required_content_restorations: encoded_content_restorations,
                },
                None,
            )
            .unwrap()
        );
        assert_eq!(
            instruction.corrective_instruction_digest(),
            &canonical_contract_digest(&instruction.digest_record(), None).unwrap()
        );
        assert_eq!(
            serde_json::to_value(
                serde_json::from_value::<SupportCorrectiveInstruction>(encoded.clone()).unwrap()
            )
            .unwrap(),
            encoded
        );

        let mut tampered = encoded.clone();
        tampered["correctiveInstructionDigest"] = json!(A);
        assert!(schema_accepts::<SupportCorrectiveInstruction>(&tampered));
        assert!(serde_json::from_value::<SupportCorrectiveInstruction>(tampered).is_err());

        for field in ["requiredRootDeltaDigest", "requiredContentDeltaDigest"] {
            let mut substituted = encoded.clone();
            substituted[field] = json!(A);
            assert!(schema_accepts::<SupportCorrectiveInstruction>(&substituted));
            assert!(serde_json::from_value::<SupportCorrectiveInstruction>(substituted).is_err());
        }

        let content_only = SupportCorrectiveInstruction::new(
            SupportCorrectiveInstructionAuthority::test_only(
                id(ID_1),
                SupportActionPurpose::AbandonmentCleanup,
                ManualSupportTargetMode::ReservedOriginal,
                username(),
                None,
                cursor(),
                recovery_locks(false),
                recovery_locks(false),
                Vec::new(),
                vec![SupportContentRestoration::restore_existing_root_test_only(
                    RepositoryTargetDisplay::parse("Configuration").unwrap(),
                    cursor(),
                    digest(A),
                    digest(A),
                )],
                Vec::new(),
                Vec::new(),
                digest(A),
                digest(A),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            content_only.required_root_delta_digest(),
            &required_root_delta_digest(&SupportRecoveryTransitions::new(Vec::new()).unwrap())
                .unwrap()
        );
        let content_only_json = serde_json::to_value(&content_only).unwrap();
        let content_only_restorations = serde_json::from_value::<SupportContentRestorations>(
            content_only_json["requiredContentRestorations"].clone(),
        )
        .unwrap();
        assert_eq!(
            content_only.required_content_delta_digest(),
            &required_content_delta_digest(&content_only_restorations).unwrap()
        );

        let mut mode_splice = encoded.clone();
        mode_splice["workingInfobaseIdentity"] = serde_json::to_value(working_identity()).unwrap();
        assert!(!schema_accepts::<SupportCorrectiveInstruction>(
            &mode_splice
        ));
        assert!(serde_json::from_value::<SupportCorrectiveInstruction>(mode_splice).is_err());

        assert!(SupportCorrectiveInstructionAuthority::test_only(
            id(ID_1),
            SupportActionPurpose::AbandonmentCleanup,
            ManualSupportTargetMode::SeparateWorkingInfobase,
            username(),
            None,
            cursor(),
            recovery_locks(false),
            recovery_locks(false),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            digest(A),
            digest(A),
        )
        .is_err());

        assert_schema_closed::<SupportRequiredRootDeltaDigestRecord>();
        assert_schema_closed::<SupportRequiredContentDeltaDigestRecord>();
        assert_schema_closed::<SupportCorrectiveInstructionDigestRecord>();
        assert_schema_closed::<SupportCorrectiveInstruction>();
    }

    #[test]
    fn corrective_fixture_binds_resolved_correction_and_exact_finalization_plan() {
        let resolver = FixedCorrectiveLockClosureResolver {
            expected_transition_count: 1,
            expected_restoration_count: 1,
            result: Ok(recovery_locks(true)),
        };
        let plan = finalization_plan(
            recovery_locks(true),
            "Configuration",
            Some("Catalog.A"),
            digest(B),
            digest(C),
        );
        let authority =
            SupportCorrectiveInstructionAuthority::from_lock_closure_resolver_test_only(
                id(ID_1),
                SupportActionPurpose::AbandonmentCleanup,
                ManualSupportTargetMode::ReservedOriginal,
                username(),
                None,
                cursor(),
                vec![SupportRecoveryTransition::ordinary(
                    SupportTransition::enable_configuration_changes(
                        RepositoryTargetDisplay::parse("Configuration").unwrap(),
                        SupportLayerId::parse("layer-a").unwrap(),
                    ),
                )],
                vec![object_restoration()],
                Vec::new(),
                Vec::new(),
                &plan,
                &resolver,
            )
            .unwrap();
        let instruction = SupportCorrectiveInstruction::new(authority).unwrap();
        let encoded_instruction = serde_json::to_value(&instruction).unwrap();
        let encoded_plan = serde_json::to_value(&plan).unwrap();

        assert_eq!(
            encoded_instruction["correctionLockTargets"],
            serde_json::to_value(recovery_locks(true)).unwrap(),
        );
        assert_eq!(
            serde_json::to_vec(&encoded_instruction["finalizationLockTargets"]).unwrap(),
            serde_json::to_vec(&encoded_plan["lockTargets"]).unwrap(),
        );
        assert_eq!(
            encoded_instruction["desiredSupportGraphDigest"],
            encoded_plan["desiredSupportGraphDigest"],
        );
        assert_eq!(
            encoded_instruction["desiredRepositoryContentDigest"],
            encoded_plan["desiredRepositoryContentDigest"],
        );

        let altered_same_id_locks = SupportRecoveryLockTargets::new(vec![
            recovery_root_lock(),
            SupportRecoveryLockTarget::development_object(
                MetadataObjectId::parse(OBJECT_A).unwrap(),
                RepositoryTargetDisplay::parse("Catalog.A from another frozen plan").unwrap(),
                vec![RepositoryUpdateLockReason::StructuralClosure],
            )
            .unwrap(),
        ])
        .unwrap();
        let altered_plan = finalization_plan(
            altered_same_id_locks,
            "Configuration",
            Some("Catalog.A from another frozen plan"),
            digest(B),
            digest(C),
        );
        let altered_instruction = SupportCorrectiveInstruction::new(
            SupportCorrectiveInstructionAuthority::from_lock_closure_resolver_test_only(
                id(ID_1),
                SupportActionPurpose::AbandonmentCleanup,
                ManualSupportTargetMode::ReservedOriginal,
                username(),
                None,
                cursor(),
                vec![SupportRecoveryTransition::ordinary(
                    SupportTransition::enable_configuration_changes(
                        RepositoryTargetDisplay::parse("Configuration").unwrap(),
                        SupportLayerId::parse("layer-a").unwrap(),
                    ),
                )],
                vec![object_restoration()],
                Vec::new(),
                Vec::new(),
                &altered_plan,
                &resolver,
            )
            .unwrap(),
        )
        .unwrap();
        let encoded_altered_instruction = serde_json::to_value(&altered_instruction).unwrap();
        let encoded_altered_plan = serde_json::to_value(&altered_plan).unwrap();
        assert_ne!(
            encoded_instruction["finalizationLockTargets"],
            encoded_altered_instruction["finalizationLockTargets"],
        );
        assert_ne!(
            instruction.corrective_instruction_digest(),
            altered_instruction.corrective_instruction_digest(),
        );
        assert_eq!(
            serde_json::to_vec(&encoded_altered_instruction["finalizationLockTargets"]).unwrap(),
            serde_json::to_vec(&encoded_altered_plan["lockTargets"]).unwrap(),
        );

        let unavailable = FixedCorrectiveLockClosureResolver {
            expected_transition_count: 1,
            expected_restoration_count: 1,
            result: Err(SupportCorrectiveLockClosureResolutionError::Unavailable),
        };
        assert!(
            SupportCorrectiveInstructionAuthority::from_lock_closure_resolver_test_only(
                id(ID_1),
                SupportActionPurpose::AbandonmentCleanup,
                ManualSupportTargetMode::ReservedOriginal,
                username(),
                None,
                cursor(),
                vec![SupportRecoveryTransition::ordinary(
                    SupportTransition::enable_configuration_changes(
                        RepositoryTargetDisplay::parse("Configuration").unwrap(),
                        SupportLayerId::parse("layer-a").unwrap(),
                    ),
                )],
                vec![object_restoration()],
                Vec::new(),
                Vec::new(),
                &plan,
                &unavailable,
            )
            .is_err()
        );

        let wrong_closure_ids = FixedCorrectiveLockClosureResolver {
            expected_transition_count: 1,
            expected_restoration_count: 1,
            result: Ok(recovery_locks(false)),
        };
        assert!(
            SupportCorrectiveInstructionAuthority::from_lock_closure_resolver_test_only(
                id(ID_1),
                SupportActionPurpose::AbandonmentCleanup,
                ManualSupportTargetMode::ReservedOriginal,
                username(),
                None,
                cursor(),
                vec![SupportRecoveryTransition::ordinary(
                    SupportTransition::enable_configuration_changes(
                        RepositoryTargetDisplay::parse("Configuration").unwrap(),
                        SupportLayerId::parse("layer-a").unwrap(),
                    ),
                )],
                vec![object_restoration()],
                Vec::new(),
                Vec::new(),
                &plan,
                &wrong_closure_ids,
            )
            .is_err()
        );
    }

    #[test]
    fn corrective_authority_rejects_wrong_closure_cursor_and_handoff_lineage() {
        assert!(SupportCorrectiveInstructionAuthority::test_only(
            id(ID_1),
            SupportActionPurpose::AbandonmentCleanup,
            ManualSupportTargetMode::ReservedOriginal,
            username(),
            None,
            cursor(),
            recovery_locks(true),
            recovery_locks(true),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            digest(A),
            digest(A),
        )
        .is_err());

        let restoration = SupportContentRestoration::restore_existing_root_test_only(
            RepositoryTargetDisplay::parse("Configuration").unwrap(),
            serde_json::from_value(json!({
                "throughVersion": "v2",
                "historyPrefixDigest": A,
            }))
            .unwrap(),
            digest(A),
            digest(A),
        );
        assert!(SupportCorrectiveInstructionAuthority::test_only(
            id(ID_1),
            SupportActionPurpose::AbandonmentCleanup,
            ManualSupportTargetMode::ReservedOriginal,
            username(),
            None,
            cursor(),
            recovery_locks(false),
            recovery_locks(false),
            Vec::new(),
            vec![restoration],
            Vec::new(),
            Vec::new(),
            digest(A),
            digest(A),
        )
        .is_err());

        let handoff = recovery_handoff(ManualSupportTargetMode::ReservedOriginal);
        let revalidation = handoff_revalidation(&handoff);
        let recovery_transition =
            SupportRecoveryTransition::restore_vendor_configuration_support_test_only(
                RepositoryTargetDisplay::parse("Configuration").unwrap(),
                SupportLayerId::parse("layer-a").unwrap(),
                VendorRestoredSupportState::Locked,
                id(ID_2),
                id(ID_1),
                CapabilityRowId::parse("support-recovery.v1").unwrap(),
            );
        assert!(SupportCorrectiveInstructionAuthority::test_only(
            id(ID_1),
            SupportActionPurpose::AbandonmentCleanup,
            ManualSupportTargetMode::ReservedOriginal,
            username(),
            None,
            cursor(),
            recovery_locks(false),
            recovery_locks(false),
            vec![recovery_transition.clone()],
            Vec::new(),
            vec![handoff.clone()],
            vec![revalidation],
            digest(A),
            digest(A),
        )
        .is_ok());
        assert!(SupportCorrectiveInstructionAuthority::test_only(
            id(ID_1),
            SupportActionPurpose::AbandonmentCleanup,
            ManualSupportTargetMode::ReservedOriginal,
            username(),
            None,
            cursor(),
            recovery_locks(false),
            recovery_locks(false),
            vec![recovery_transition],
            Vec::new(),
            vec![handoff],
            Vec::new(),
            digest(A),
            digest(A),
        )
        .is_err());
    }

    #[test]
    fn recovery_external_action_is_the_exact_six_leaf_union() {
        let leaves = vec![
            SupportRecoveryExternalAction::corrective(corrective_instruction(
                ManualSupportTargetMode::ReservedOriginal,
                None,
            )),
            SupportRecoveryExternalAction::release_locks(
                ReleaseRepositoryLocksInstruction::new(RequiredNullable::null(), Vec::new())
                    .unwrap(),
            ),
            SupportRecoveryExternalAction::clean_working_infobase(
                CleanManualWorkingInfobaseInstruction::new(
                    working_identity(),
                    digest(A),
                    CapabilityRowId::parse("manual-lease.v1").unwrap(),
                    digest(A),
                    ManualWorkingInfobaseCleanupReason::LeaseBusy,
                ),
            ),
            SupportRecoveryExternalAction::close_reserved_original(
                CloseReservedOriginalDesignerInstruction::new(
                    digest(A),
                    CapabilityRowId::parse("reserved-original-lease.v1").unwrap(),
                ),
            ),
            SupportRecoveryExternalAction::conflict(
                SupportConflictInstruction::new(id(ID_1), conflicts(), digest(A)).unwrap(),
            ),
            SupportRecoveryExternalAction::evidence(
                SupportEvidenceInstruction::new(blockers(), evidence_gaps()).unwrap(),
            ),
        ];
        assert_eq!(
            leaves
                .iter()
                .map(|leaf| serde_json::to_value(leaf).unwrap()["kind"]
                    .as_str()
                    .unwrap()
                    .to_owned())
                .collect::<Vec<_>>(),
            vec![
                "correctSupportPrerequisite",
                "releaseRepositoryLocks",
                "cleanManualWorkingInfobase",
                "closeReservedOriginalDesigner",
                "coordinateExternalSupportChange",
                "provideSupportEvidence",
            ]
        );
        for leaf in &leaves {
            let encoded = serde_json::to_value(leaf).unwrap();
            assert!(schema_accepts::<SupportRecoveryExternalAction>(&encoded));
            let mut cross_leaf = encoded;
            cross_leaf["unexpectedCrossLeafField"] = json!(true);
            assert!(!schema_accepts::<SupportRecoveryExternalAction>(
                &cross_leaf
            ));
        }
        assert_schema_closed::<SupportRecoveryExternalAction>();
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
use super::repository::{
    RepositoryActorIdentity, RepositoryHistoryCursor, RepositoryOwnerIdentity,
    RepositoryUpdateLockReason,
};
use super::scalars::{
    Diagnostic, RepositoryTargetDisplay, RepositoryUsername, RepositoryVersion, RequiredNullable,
};
use super::schema::one_of_schema;
use super::support::{
    ArmedSupportInstructionProjection, AwaitingSupportInstructionProjection,
    ManualSupportTargetMode, ManualWorkingInfobaseIdentity, SupportActionPurpose, SupportBlockers,
    SupportEvidenceGaps, SupportHistoryOrderAuthority, SupportMissingEvidenceKinds,
    SupportRecoveryDistributionHandoff, SupportRecoveryHandoffRevalidation, SupportTransition,
    SupportTransitionConflict, SupportTransitionConflicts, SupportTransitionOverlapKind,
    SupportTransitions, VendorSupportDecisions,
};
#[cfg(test)]
use super::support_terminalization::SupportRecoveryFinalizationPlan;
use super::support_terminalization::{SupportRecoveryLockTarget, SupportRecoveryLockTargets};
use crate::domain::branched_development::canonical_json::{
    canonical_contract_digest, contract_digest_record_sealed, ContractDigestRecord,
};
use crate::domain::branched_development::{
    CapabilityRowId, MetadataObjectId, Sha256Digest, SupportLayerId, UnicaId,
};
use schemars::{json_schema, JsonSchema, Schema, SchemaGenerator};
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};
use std::borrow::Cow;
use std::collections::BTreeSet;
use std::fmt;

const MAX_INSTRUCTION_ITEMS: usize = 1024;

macro_rules! wire_literal {
    ($name:ident, $wire:literal) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
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
wire_literal!(CorrectSupportPrerequisiteKind, "correctSupportPrerequisite");
wire_literal!(ReservedOriginalMode, "reservedOriginal");
wire_literal!(SeparateWorkingInfobaseMode, "separateWorkingInfobase");
wire_literal!(ConfigurationRootLockTarget, "configurationRoot");
wire_literal!(RepositoryUpdateResume, "repository.update");
wire_literal!(SupportPrerequisiteArmResumeMode, "supportPrerequisiteArm");
wire_literal!(BranchedStatusResume, "branched.status");
wire_literal!(RetainThroughCommitProcedure, "retainThroughCommit");
wire_literal!(ConfigurationRootTargetKind, "configurationRoot");
wire_literal!(DevelopmentObjectTargetKind, "developmentObject");
wire_literal!(RestoreExistingAction, "restoreExisting");
wire_literal!(
    RemoveUnauthorizedAdditionAction,
    "removeUnauthorizedAddition"
);
wire_literal!(
    RecreateUnauthorizedDeletionAction,
    "recreateUnauthorizedDeletion"
);
wire_literal!(
    RestoreVendorConfigurationSupportKind,
    "restoreVendorConfigurationSupport"
);
wire_literal!(RestoreVendorObjectSupportKind, "restoreVendorObjectSupport");
wire_literal!(OffSupportState, "offSupport");
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

impl<'de> Deserialize<'de> for InstructionTrueLiteral {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        bool::deserialize(deserializer)?
            .then_some(Self)
            .ok_or_else(|| D::Error::custom("expected literal true"))
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

impl<'de> Deserialize<'de> for InstructionFalseLiteral {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        (!bool::deserialize(deserializer)?)
            .then_some(Self)
            .ok_or_else(|| D::Error::custom("expected literal false"))
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

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub(crate) enum VendorRestoredSupportState {
    Locked,
    Editable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
struct MetadataObjectIds(Vec<MetadataObjectId>);

impl MetadataObjectIds {
    fn new(values: Vec<MetadataObjectId>) -> Result<Self, InstructionContractError> {
        if values.len() > MAX_INSTRUCTION_ITEMS || values.windows(2).any(|pair| pair[0] >= pair[1])
        {
            return Err(InstructionContractError(
                "restoration lock IDs must be canonical, unique, and bounded",
            ));
        }
        Ok(Self(values))
    }

    fn as_slice(&self) -> &[MetadataObjectId] {
        &self.0
    }
}

impl<'de> Deserialize<'de> for MetadataObjectIds {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::new(Vec::<MetadataObjectId>::deserialize(deserializer)?).map_err(D::Error::custom)
    }
}

impl JsonSchema for MetadataObjectIds {
    fn schema_name() -> Cow<'static, str> {
        "SupportRestorationLockObjectIds".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "array",
            "items": generator.subschema_for::<MetadataObjectId>(),
            "maxItems": MAX_INSTRUCTION_ITEMS,
            "uniqueItems": true,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(transparent)]
struct EmptyMetadataObjectIds([MetadataObjectId; 0]);

impl JsonSchema for EmptyMetadataObjectIds {
    fn schema_name() -> Cow<'static, str> {
        "EmptySupportRestorationLockObjectIds".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "array",
            "items": generator.subschema_for::<MetadataObjectId>(),
            "minItems": 0,
            "maxItems": 0,
            "uniqueItems": true,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RootRestoreExistingContentRestoration {
    action: RestoreExistingAction,
    target_kind: ConfigurationRootTargetKind,
    object_display: RepositoryTargetDisplay,
    correction_base_cursor: RepositoryHistoryCursor,
    expected_current_fingerprint: Sha256Digest,
    expected_repository_fingerprint: Sha256Digest,
    correction_lock_object_ids: EmptyMetadataObjectIds,
    finalization_lock_object_ids: EmptyMetadataObjectIds,
    structural_confirmation_required: InstructionFalseLiteral,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ObjectRestoreExistingContentRestoration {
    action: RestoreExistingAction,
    target_kind: DevelopmentObjectTargetKind,
    object_id: MetadataObjectId,
    object_display: RepositoryTargetDisplay,
    correction_base_cursor: RepositoryHistoryCursor,
    expected_current_fingerprint: Sha256Digest,
    expected_repository_fingerprint: Sha256Digest,
    correction_lock_object_ids: MetadataObjectIds,
    finalization_lock_object_ids: MetadataObjectIds,
    reference_closure_digest: Sha256Digest,
    structural_confirmation_required: InstructionFalseLiteral,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RemoveUnauthorizedAdditionContentRestoration {
    action: RemoveUnauthorizedAdditionAction,
    target_kind: DevelopmentObjectTargetKind,
    object_id: MetadataObjectId,
    object_display: RepositoryTargetDisplay,
    correction_base_cursor: RepositoryHistoryCursor,
    expected_current_fingerprint: Sha256Digest,
    expected_absent: InstructionTrueLiteral,
    correction_lock_object_ids: MetadataObjectIds,
    finalization_lock_object_ids: MetadataObjectIds,
    reference_closure_digest: Sha256Digest,
    structural_confirmation_required: InstructionTrueLiteral,
    structural_capability_row_id: CapabilityRowId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RecreateUnauthorizedDeletionContentRestoration {
    action: RecreateUnauthorizedDeletionAction,
    target_kind: DevelopmentObjectTargetKind,
    object_id: MetadataObjectId,
    object_display: RepositoryTargetDisplay,
    correction_base_cursor: RepositoryHistoryCursor,
    expected_current_absent: InstructionTrueLiteral,
    expected_repository_fingerprint: Sha256Digest,
    source_checkpoint_id: UnicaId,
    correction_lock_object_ids: MetadataObjectIds,
    finalization_lock_object_ids: MetadataObjectIds,
    reference_closure_digest: Sha256Digest,
    structural_confirmation_required: InstructionTrueLiteral,
    structural_capability_row_id: CapabilityRowId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
enum SupportContentRestorationKind {
    RootRestoreExisting(RootRestoreExistingContentRestoration),
    ObjectRestoreExisting(ObjectRestoreExistingContentRestoration),
    RemoveUnauthorizedAddition(RemoveUnauthorizedAdditionContentRestoration),
    RecreateUnauthorizedDeletion(RecreateUnauthorizedDeletionContentRestoration),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct SupportContentRestoration(SupportContentRestorationKind);

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum RestorationTargetKey {
    Root,
    Object(String),
}

impl SupportContentRestoration {
    fn validate_kind(
        kind: SupportContentRestorationKind,
    ) -> Result<Self, InstructionContractError> {
        let valid = match &kind {
            SupportContentRestorationKind::RemoveUnauthorizedAddition(value) => !value
                .finalization_lock_object_ids
                .as_slice()
                .contains(&value.object_id),
            SupportContentRestorationKind::RecreateUnauthorizedDeletion(value) => !value
                .correction_lock_object_ids
                .as_slice()
                .contains(&value.object_id),
            SupportContentRestorationKind::RootRestoreExisting(_)
            | SupportContentRestorationKind::ObjectRestoreExisting(_) => true,
        };
        valid.then_some(Self(kind)).ok_or(InstructionContractError(
            "restoration target appears in the wrong existence-state lock set",
        ))
    }

    #[cfg(test)]
    pub(crate) fn restore_existing_root_test_only(
        object_display: RepositoryTargetDisplay,
        correction_base_cursor: RepositoryHistoryCursor,
        expected_current_fingerprint: Sha256Digest,
        expected_repository_fingerprint: Sha256Digest,
    ) -> Self {
        Self(SupportContentRestorationKind::RootRestoreExisting(
            RootRestoreExistingContentRestoration {
                action: RestoreExistingAction::Value,
                target_kind: ConfigurationRootTargetKind::Value,
                object_display,
                correction_base_cursor,
                expected_current_fingerprint,
                expected_repository_fingerprint,
                correction_lock_object_ids: EmptyMetadataObjectIds::default(),
                finalization_lock_object_ids: EmptyMetadataObjectIds::default(),
                structural_confirmation_required: InstructionFalseLiteral,
            },
        ))
    }

    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn restore_existing_object_test_only(
        object_id: MetadataObjectId,
        object_display: RepositoryTargetDisplay,
        correction_base_cursor: RepositoryHistoryCursor,
        expected_current_fingerprint: Sha256Digest,
        expected_repository_fingerprint: Sha256Digest,
        correction_lock_object_ids: Vec<MetadataObjectId>,
        finalization_lock_object_ids: Vec<MetadataObjectId>,
        reference_closure_digest: Sha256Digest,
    ) -> Self {
        Self::validate_kind(SupportContentRestorationKind::ObjectRestoreExisting(
            ObjectRestoreExistingContentRestoration {
                action: RestoreExistingAction::Value,
                target_kind: DevelopmentObjectTargetKind::Value,
                object_id,
                object_display,
                correction_base_cursor,
                expected_current_fingerprint,
                expected_repository_fingerprint,
                correction_lock_object_ids: MetadataObjectIds::new(correction_lock_object_ids)
                    .expect("test correction lock IDs must be canonical"),
                finalization_lock_object_ids: MetadataObjectIds::new(finalization_lock_object_ids)
                    .expect("test finalization lock IDs must be canonical"),
                reference_closure_digest,
                structural_confirmation_required: InstructionFalseLiteral,
            },
        ))
        .expect("test restoration must preserve existence-state lock semantics")
    }

    fn key(&self) -> RestorationTargetKey {
        match &self.0 {
            SupportContentRestorationKind::RootRestoreExisting(_) => RestorationTargetKey::Root,
            SupportContentRestorationKind::ObjectRestoreExisting(value) => {
                RestorationTargetKey::Object(value.object_id.as_str().to_owned())
            }
            SupportContentRestorationKind::RemoveUnauthorizedAddition(value) => {
                RestorationTargetKey::Object(value.object_id.as_str().to_owned())
            }
            SupportContentRestorationKind::RecreateUnauthorizedDeletion(value) => {
                RestorationTargetKey::Object(value.object_id.as_str().to_owned())
            }
        }
    }

    fn correction_base_cursor(&self) -> &RepositoryHistoryCursor {
        match &self.0 {
            SupportContentRestorationKind::RootRestoreExisting(value) => {
                &value.correction_base_cursor
            }
            SupportContentRestorationKind::ObjectRestoreExisting(value) => {
                &value.correction_base_cursor
            }
            SupportContentRestorationKind::RemoveUnauthorizedAddition(value) => {
                &value.correction_base_cursor
            }
            SupportContentRestorationKind::RecreateUnauthorizedDeletion(value) => {
                &value.correction_base_cursor
            }
        }
    }

    fn correction_lock_object_ids(&self) -> &[MetadataObjectId] {
        match &self.0 {
            SupportContentRestorationKind::RootRestoreExisting(_) => &[],
            SupportContentRestorationKind::ObjectRestoreExisting(value) => {
                value.correction_lock_object_ids.as_slice()
            }
            SupportContentRestorationKind::RemoveUnauthorizedAddition(value) => {
                value.correction_lock_object_ids.as_slice()
            }
            SupportContentRestorationKind::RecreateUnauthorizedDeletion(value) => {
                value.correction_lock_object_ids.as_slice()
            }
        }
    }

    fn finalization_lock_object_ids(&self) -> &[MetadataObjectId] {
        match &self.0 {
            SupportContentRestorationKind::RootRestoreExisting(_) => &[],
            SupportContentRestorationKind::ObjectRestoreExisting(value) => {
                value.finalization_lock_object_ids.as_slice()
            }
            SupportContentRestorationKind::RemoveUnauthorizedAddition(value) => {
                value.finalization_lock_object_ids.as_slice()
            }
            SupportContentRestorationKind::RecreateUnauthorizedDeletion(value) => {
                value.finalization_lock_object_ids.as_slice()
            }
        }
    }
}

impl<'de> Deserialize<'de> for SupportContentRestoration {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::validate_kind(SupportContentRestorationKind::deserialize(deserializer)?)
            .map_err(D::Error::custom)
    }
}

impl JsonSchema for SupportContentRestoration {
    fn schema_name() -> Cow<'static, str> {
        "SupportContentRestoration".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<RootRestoreExistingContentRestoration>(),
            generator.subschema_for::<ObjectRestoreExistingContentRestoration>(),
            generator.subschema_for::<RemoveUnauthorizedAdditionContentRestoration>(),
            generator.subschema_for::<RecreateUnauthorizedDeletionContentRestoration>(),
        ])
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
struct SupportContentRestorations(Vec<SupportContentRestoration>);

impl SupportContentRestorations {
    fn new(values: Vec<SupportContentRestoration>) -> Result<Self, InstructionContractError> {
        if values.len() > MAX_INSTRUCTION_ITEMS
            || values.windows(2).any(|pair| pair[0].key() >= pair[1].key())
        {
            return Err(InstructionContractError(
                "content restorations must be canonical, target-unique, and bounded",
            ));
        }
        Ok(Self(values))
    }

    fn as_slice(&self) -> &[SupportContentRestoration] {
        &self.0
    }

    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl<'de> Deserialize<'de> for SupportContentRestorations {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::new(Vec::<SupportContentRestoration>::deserialize(deserializer)?)
            .map_err(D::Error::custom)
    }
}

impl JsonSchema for SupportContentRestorations {
    fn schema_name() -> Cow<'static, str> {
        "SupportContentRestorations".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "array",
            "items": generator.subschema_for::<SupportContentRestoration>(),
            "maxItems": MAX_INSTRUCTION_ITEMS,
            "uniqueItems": true,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RestoreVendorConfigurationSupportTransition {
    transition_kind: RestoreVendorConfigurationSupportKind,
    target_kind: ConfigurationRootTargetKind,
    configuration_display: RepositoryTargetDisplay,
    layer_id: SupportLayerId,
    from_state: OffSupportState,
    to_state: VendorRestoredSupportState,
    vendor_distribution_artifact_id: UnicaId,
    recovery_distribution_handoff_id: UnicaId,
    capability_row_id: CapabilityRowId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RestoreVendorObjectSupportTransition {
    transition_kind: RestoreVendorObjectSupportKind,
    target_kind: DevelopmentObjectTargetKind,
    object_id: MetadataObjectId,
    object_display: RepositoryTargetDisplay,
    layer_id: SupportLayerId,
    from_state: OffSupportState,
    to_state: VendorRestoredSupportState,
    vendor_distribution_artifact_id: UnicaId,
    recovery_distribution_handoff_id: UnicaId,
    capability_row_id: CapabilityRowId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
enum SupportRecoveryTransitionKind {
    Ordinary(SupportTransition),
    RestoreVendorConfigurationSupport(RestoreVendorConfigurationSupportTransition),
    RestoreVendorObjectSupport(RestoreVendorObjectSupportTransition),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct SupportRecoveryTransition(SupportRecoveryTransitionKind);

impl SupportRecoveryTransition {
    pub(crate) const fn ordinary(transition: SupportTransition) -> Self {
        Self(SupportRecoveryTransitionKind::Ordinary(transition))
    }

    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn restore_vendor_configuration_support_test_only(
        configuration_display: RepositoryTargetDisplay,
        layer_id: SupportLayerId,
        to_state: VendorRestoredSupportState,
        vendor_distribution_artifact_id: UnicaId,
        recovery_distribution_handoff_id: UnicaId,
        capability_row_id: CapabilityRowId,
    ) -> Self {
        Self(
            SupportRecoveryTransitionKind::RestoreVendorConfigurationSupport(
                RestoreVendorConfigurationSupportTransition {
                    transition_kind: RestoreVendorConfigurationSupportKind::Value,
                    target_kind: ConfigurationRootTargetKind::Value,
                    configuration_display,
                    layer_id,
                    from_state: OffSupportState::Value,
                    to_state,
                    vendor_distribution_artifact_id,
                    recovery_distribution_handoff_id,
                    capability_row_id,
                },
            ),
        )
    }

    fn canonical_bytes(&self) -> Result<Vec<u8>, InstructionContractError> {
        serde_json_canonicalizer::to_vec(self).map_err(|_| {
            InstructionContractError("support recovery transition canonicalization failed")
        })
    }

    fn recovery_handoff_binding(&self) -> Option<(&SupportLayerId, &UnicaId, &UnicaId)> {
        match &self.0 {
            SupportRecoveryTransitionKind::Ordinary(_) => None,
            SupportRecoveryTransitionKind::RestoreVendorConfigurationSupport(value) => Some((
                &value.layer_id,
                &value.vendor_distribution_artifact_id,
                &value.recovery_distribution_handoff_id,
            )),
            SupportRecoveryTransitionKind::RestoreVendorObjectSupport(value) => Some((
                &value.layer_id,
                &value.vendor_distribution_artifact_id,
                &value.recovery_distribution_handoff_id,
            )),
        }
    }
}

impl<'de> Deserialize<'de> for SupportRecoveryTransition {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        SupportRecoveryTransitionKind::deserialize(deserializer).map(Self)
    }
}

impl JsonSchema for SupportRecoveryTransition {
    fn schema_name() -> Cow<'static, str> {
        "SupportRecoveryTransition".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<SupportTransition>(),
            generator.subschema_for::<RestoreVendorConfigurationSupportTransition>(),
            generator.subschema_for::<RestoreVendorObjectSupportTransition>(),
        ])
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
struct SupportRecoveryTransitions(Vec<SupportRecoveryTransition>);

impl SupportRecoveryTransitions {
    fn new(values: Vec<SupportRecoveryTransition>) -> Result<Self, InstructionContractError> {
        if values.len() > MAX_INSTRUCTION_ITEMS {
            return Err(InstructionContractError(
                "support recovery transitions exceed the instruction collection bound",
            ));
        }
        let keys = values
            .iter()
            .map(SupportRecoveryTransition::canonical_bytes)
            .collect::<Result<Vec<_>, _>>()?;
        if keys.windows(2).any(|pair| pair[0] >= pair[1]) {
            return Err(InstructionContractError(
                "support recovery transitions must be canonical and duplicate-free",
            ));
        }
        Ok(Self(values))
    }

    fn as_slice(&self) -> &[SupportRecoveryTransition] {
        &self.0
    }

    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl<'de> Deserialize<'de> for SupportRecoveryTransitions {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::new(Vec::<SupportRecoveryTransition>::deserialize(deserializer)?)
            .map_err(D::Error::custom)
    }
}

impl JsonSchema for SupportRecoveryTransitions {
    fn schema_name() -> Cow<'static, str> {
        "SupportRecoveryTransitions".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "array",
            "items": generator.subschema_for::<SupportRecoveryTransition>(),
            "maxItems": MAX_INSTRUCTION_ITEMS,
            "uniqueItems": true,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
struct SupportRecoveryDistributionHandoffs(Vec<SupportRecoveryDistributionHandoff>);

impl SupportRecoveryDistributionHandoffs {
    fn new(
        values: Vec<SupportRecoveryDistributionHandoff>,
    ) -> Result<Self, InstructionContractError> {
        let mut layer_ids = BTreeSet::new();
        let mut handoff_ids = BTreeSet::new();
        if values.len() > MAX_INSTRUCTION_ITEMS
            || values.windows(2).any(|pair| {
                (pair[0].layer_id(), pair[0].handoff_id())
                    >= (pair[1].layer_id(), pair[1].handoff_id())
            })
            || values.iter().any(|value| {
                !layer_ids.insert(value.layer_id().clone())
                    || !handoff_ids.insert(value.handoff_id().clone())
            })
        {
            return Err(InstructionContractError(
                "support recovery handoffs must be canonical, unique, and bounded",
            ));
        }
        Ok(Self(values))
    }

    fn as_slice(&self) -> &[SupportRecoveryDistributionHandoff] {
        &self.0
    }
}

impl JsonSchema for SupportRecoveryDistributionHandoffs {
    fn schema_name() -> Cow<'static, str> {
        "SupportRecoveryDistributionHandoffs".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "array",
            "items": generator.subschema_for::<SupportRecoveryDistributionHandoff>(),
            "maxItems": MAX_INSTRUCTION_ITEMS,
            "uniqueItems": true,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SupportRecoveryHandoffRevalidationWire {
    handoff_id: UnicaId,
    retention_lease_id: UnicaId,
    expected_artifact_sha256: Sha256Digest,
    observed_artifact_sha256: Sha256Digest,
    retention_lease_still_held: InstructionTrueLiteral,
    readable_by_manual_actor: InstructionTrueLiteral,
    revalidation_receipt_id: UnicaId,
    manual_readability_capability_row_id: CapabilityRowId,
    retention_capability_row_id: CapabilityRowId,
    revalidation_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
struct SupportRecoveryHandoffRevalidations(Vec<SupportRecoveryHandoffRevalidation>);

impl SupportRecoveryHandoffRevalidations {
    fn new(
        handoffs: &SupportRecoveryDistributionHandoffs,
        values: Vec<SupportRecoveryHandoffRevalidation>,
    ) -> Result<Self, InstructionContractError> {
        if handoffs.as_slice().len() != values.len() {
            return Err(InstructionContractError(
                "handoff revalidations must map one-to-one to frozen handoffs",
            ));
        }
        for (handoff, revalidation) in handoffs.as_slice().iter().zip(&values) {
            let encoded = serde_json::to_value(revalidation).map_err(|_| {
                InstructionContractError("handoff revalidation serialization failed")
            })?;
            if encoded.get("handoffId").and_then(serde_json::Value::as_str)
                != Some(handoff.handoff_id().as_str())
            {
                return Err(InstructionContractError(
                    "handoff revalidation order or identity differs from the handoff set",
                ));
            }
        }
        Ok(Self(values))
    }
}

impl JsonSchema for SupportRecoveryHandoffRevalidations {
    fn schema_name() -> Cow<'static, str> {
        "SupportRecoveryHandoffRevalidations".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "array",
            "items": generator.subschema_for::<SupportRecoveryHandoffRevalidation>(),
            "maxItems": MAX_INSTRUCTION_ITEMS,
            "uniqueItems": true,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SupportRequiredRootDeltaDigestRecord {
    required_root_transitions: SupportRecoveryTransitions,
}

impl contract_digest_record_sealed::Sealed for SupportRequiredRootDeltaDigestRecord {}
impl ContractDigestRecord for SupportRequiredRootDeltaDigestRecord {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SupportRequiredContentDeltaDigestRecord {
    required_content_restorations: SupportContentRestorations,
}

impl contract_digest_record_sealed::Sealed for SupportRequiredContentDeltaDigestRecord {}
impl ContractDigestRecord for SupportRequiredContentDeltaDigestRecord {}

fn required_root_delta_digest(
    required_root_transitions: &SupportRecoveryTransitions,
) -> Result<Sha256Digest, InstructionContractError> {
    instruction_digest(
        &SupportRequiredRootDeltaDigestRecord {
            required_root_transitions: required_root_transitions.clone(),
        },
        "required root delta digest computation failed",
    )
}

fn required_content_delta_digest(
    required_content_restorations: &SupportContentRestorations,
) -> Result<Sha256Digest, InstructionContractError> {
    instruction_digest(
        &SupportRequiredContentDeltaDigestRecord {
            required_content_restorations: required_content_restorations.clone(),
        },
        "required content delta digest computation failed",
    )
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RootSupportRecoveryLockTargetWire {
    target_kind: ConfigurationRootTargetKind,
    object_display: RepositoryTargetDisplay,
    reasons: Vec<RepositoryUpdateLockReason>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ObjectSupportRecoveryLockTargetWire {
    target_kind: DevelopmentObjectTargetKind,
    object_id: MetadataObjectId,
    object_display: RepositoryTargetDisplay,
    reasons: Vec<RepositoryUpdateLockReason>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
enum SupportRecoveryLockTargetWire {
    Root(RootSupportRecoveryLockTargetWire),
    Object(ObjectSupportRecoveryLockTargetWire),
}

fn decode_support_recovery_lock_targets(
    values: Vec<SupportRecoveryLockTargetWire>,
) -> Result<SupportRecoveryLockTargets, InstructionContractError> {
    let targets = values
        .into_iter()
        .map(|value| match value {
            SupportRecoveryLockTargetWire::Root(value) => {
                SupportRecoveryLockTarget::configuration_root(value.object_display, value.reasons)
                    .map_err(|_| InstructionContractError("invalid corrective root lock target"))
            }
            SupportRecoveryLockTargetWire::Object(value) => {
                SupportRecoveryLockTarget::development_object(
                    value.object_id,
                    value.object_display,
                    value.reasons,
                )
                .map_err(|_| InstructionContractError("invalid corrective object lock target"))
            }
        })
        .collect::<Result<Vec<_>, _>>()?;
    SupportRecoveryLockTargets::new(targets)
        .map_err(|_| InstructionContractError("invalid corrective lock target set"))
}

fn serialized_lock_object_ids(
    targets: &SupportRecoveryLockTargets,
) -> Result<Vec<String>, InstructionContractError> {
    let encoded = serde_json::to_value(targets)
        .map_err(|_| InstructionContractError("corrective lock target serialization failed"))?;
    let values = encoded.as_array().ok_or(InstructionContractError(
        "corrective lock targets are not an array",
    ))?;
    values
        .iter()
        .skip(1)
        .map(|value| {
            value
                .get("objectId")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
                .ok_or(InstructionContractError(
                    "corrective object lock target lacks an object ID",
                ))
        })
        .collect()
}

fn decode_handoff_revalidations(
    handoffs: &SupportRecoveryDistributionHandoffs,
    wires: Vec<SupportRecoveryHandoffRevalidationWire>,
) -> Result<SupportRecoveryHandoffRevalidations, InstructionContractError> {
    if handoffs.as_slice().len() != wires.len() {
        return Err(InstructionContractError(
            "handoff revalidation wire set differs from the handoff set",
        ));
    }
    let mut values = Vec::with_capacity(wires.len());
    for (handoff, wire) in handoffs.as_slice().iter().zip(wires) {
        let expected_wire = serde_json::to_value(&wire).map_err(|_| {
            InstructionContractError("handoff revalidation wire serialization failed")
        })?;
        let value = SupportRecoveryHandoffRevalidation::new(
            handoff,
            wire.observed_artifact_sha256,
            wire.revalidation_receipt_id,
            wire.manual_readability_capability_row_id,
            wire.retention_capability_row_id,
        )
        .map_err(|_| InstructionContractError("handoff revalidation reconstruction failed"))?;
        let observed_wire = serde_json::to_value(&value)
            .map_err(|_| InstructionContractError("handoff revalidation serialization failed"))?;
        if observed_wire != expected_wire {
            return Err(InstructionContractError(
                "handoff revalidation fields or digest do not match reconstructed evidence",
            ));
        }
        values.push(value);
    }
    SupportRecoveryHandoffRevalidations::new(handoffs, values)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SupportCorrectiveInstructionAuthority {
    support_action_id: UnicaId,
    purpose: SupportActionPurpose,
    manual_target_mode: ManualSupportTargetMode,
    repository_username: RepositoryUsername,
    working_infobase_identity: Option<ManualWorkingInfobaseIdentity>,
    correction_base_cursor: RepositoryHistoryCursor,
    correction_lock_targets: SupportRecoveryLockTargets,
    finalization_lock_targets: SupportRecoveryLockTargets,
    required_root_transitions: SupportRecoveryTransitions,
    required_content_restorations: SupportContentRestorations,
    distribution_handoffs: SupportRecoveryDistributionHandoffs,
    handoff_revalidations: SupportRecoveryHandoffRevalidations,
    desired_support_graph_digest: Sha256Digest,
    desired_repository_content_digest: Sha256Digest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg(test)]
pub(crate) enum SupportCorrectiveLockClosureResolutionError {
    Unavailable,
}

/// Test fixture for the future capability boundary that resolves a correction
/// lock closure from an action-bound recovery projection.
#[cfg(test)]
pub(crate) trait SupportCorrectiveLockClosureResolver {
    fn resolve_correction_lock_targets(
        &self,
        required_root_transitions: &[SupportRecoveryTransition],
        required_content_restorations: &[SupportContentRestoration],
    ) -> Result<SupportRecoveryLockTargets, SupportCorrectiveLockClosureResolutionError>;
}

impl SupportCorrectiveInstructionAuthority {
    /// Task 9 fixture for the corrective wire/digest contract. Production mint
    /// stays unavailable until Task 11 can supply one opaque action-bound
    /// recovery projection containing the approved delta, handoffs, history,
    /// and finalization plan.
    #[allow(clippy::too_many_arguments)]
    #[cfg(test)]
    pub(crate) fn from_lock_closure_resolver_test_only(
        support_action_id: UnicaId,
        purpose: SupportActionPurpose,
        manual_target_mode: ManualSupportTargetMode,
        repository_username: RepositoryUsername,
        working_infobase_identity: Option<ManualWorkingInfobaseIdentity>,
        correction_base_cursor: RepositoryHistoryCursor,
        required_root_transitions: Vec<SupportRecoveryTransition>,
        required_content_restorations: Vec<SupportContentRestoration>,
        distribution_handoffs: Vec<SupportRecoveryDistributionHandoff>,
        handoff_revalidations: Vec<SupportRecoveryHandoffRevalidation>,
        finalization_plan: &SupportRecoveryFinalizationPlan,
        lock_closure_resolver: &dyn SupportCorrectiveLockClosureResolver,
    ) -> Result<Self, InstructionContractError> {
        let required_root_transitions = SupportRecoveryTransitions::new(required_root_transitions)?;
        let required_content_restorations =
            SupportContentRestorations::new(required_content_restorations)?;
        let correction_lock_targets = lock_closure_resolver
            .resolve_correction_lock_targets(
                required_root_transitions.as_slice(),
                required_content_restorations.as_slice(),
            )
            .map_err(|_| {
                InstructionContractError("corrective lock closure capability resolution failed")
            })?;
        let distribution_handoffs =
            SupportRecoveryDistributionHandoffs::new(distribution_handoffs)?;
        let handoff_revalidations = SupportRecoveryHandoffRevalidations::new(
            &distribution_handoffs,
            handoff_revalidations,
        )?;

        Self::validated(
            support_action_id,
            purpose,
            manual_target_mode,
            repository_username,
            working_infobase_identity,
            correction_base_cursor,
            correction_lock_targets,
            finalization_plan.lock_targets().clone(),
            required_root_transitions,
            required_content_restorations,
            distribution_handoffs,
            handoff_revalidations,
            finalization_plan.desired_support_graph_digest().clone(),
            finalization_plan
                .desired_repository_content_digest()
                .clone(),
        )
    }

    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn test_only(
        support_action_id: UnicaId,
        purpose: SupportActionPurpose,
        manual_target_mode: ManualSupportTargetMode,
        repository_username: RepositoryUsername,
        working_infobase_identity: Option<ManualWorkingInfobaseIdentity>,
        correction_base_cursor: RepositoryHistoryCursor,
        correction_lock_targets: SupportRecoveryLockTargets,
        finalization_lock_targets: SupportRecoveryLockTargets,
        required_root_transitions: Vec<SupportRecoveryTransition>,
        required_content_restorations: Vec<SupportContentRestoration>,
        distribution_handoffs: Vec<SupportRecoveryDistributionHandoff>,
        handoff_revalidations: Vec<SupportRecoveryHandoffRevalidation>,
        desired_support_graph_digest: Sha256Digest,
        desired_repository_content_digest: Sha256Digest,
    ) -> Result<Self, InstructionContractError> {
        let distribution_handoffs =
            SupportRecoveryDistributionHandoffs::new(distribution_handoffs)?;
        let handoff_revalidations = SupportRecoveryHandoffRevalidations::new(
            &distribution_handoffs,
            handoff_revalidations,
        )?;
        Self::validated(
            support_action_id,
            purpose,
            manual_target_mode,
            repository_username,
            working_infobase_identity,
            correction_base_cursor,
            correction_lock_targets,
            finalization_lock_targets,
            SupportRecoveryTransitions::new(required_root_transitions)?,
            SupportContentRestorations::new(required_content_restorations)?,
            distribution_handoffs,
            handoff_revalidations,
            desired_support_graph_digest,
            desired_repository_content_digest,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn validated(
        support_action_id: UnicaId,
        purpose: SupportActionPurpose,
        manual_target_mode: ManualSupportTargetMode,
        repository_username: RepositoryUsername,
        working_infobase_identity: Option<ManualWorkingInfobaseIdentity>,
        correction_base_cursor: RepositoryHistoryCursor,
        correction_lock_targets: SupportRecoveryLockTargets,
        finalization_lock_targets: SupportRecoveryLockTargets,
        required_root_transitions: SupportRecoveryTransitions,
        required_content_restorations: SupportContentRestorations,
        distribution_handoffs: SupportRecoveryDistributionHandoffs,
        handoff_revalidations: SupportRecoveryHandoffRevalidations,
        desired_support_graph_digest: Sha256Digest,
        desired_repository_content_digest: Sha256Digest,
    ) -> Result<Self, InstructionContractError> {
        let mode_presence_is_valid = match manual_target_mode {
            ManualSupportTargetMode::ReservedOriginal => working_infobase_identity.is_none(),
            ManualSupportTargetMode::SeparateWorkingInfobase => working_infobase_identity.is_some(),
        };
        if !mode_presence_is_valid {
            return Err(InstructionContractError(
                "corrective instruction working-IB presence disagrees with manual target mode",
            ));
        }
        if required_root_transitions.is_empty() && required_content_restorations.is_empty() {
            return Err(InstructionContractError(
                "corrective instruction contains no corrective transition or restoration",
            ));
        }
        if required_content_restorations
            .as_slice()
            .iter()
            .any(|value| value.correction_base_cursor() != &correction_base_cursor)
        {
            return Err(InstructionContractError(
                "content restoration belongs to another correction cursor",
            ));
        }

        let expected_correction_ids = required_content_restorations
            .as_slice()
            .iter()
            .flat_map(SupportContentRestoration::correction_lock_object_ids)
            .map(|value| value.as_str().to_owned())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let expected_finalization_ids = required_content_restorations
            .as_slice()
            .iter()
            .flat_map(SupportContentRestoration::finalization_lock_object_ids)
            .map(|value| value.as_str().to_owned())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        if serialized_lock_object_ids(&correction_lock_targets)? != expected_correction_ids
            || serialized_lock_object_ids(&finalization_lock_targets)? != expected_finalization_ids
        {
            return Err(InstructionContractError(
                "corrective lock targets differ from the exact restoration closure unions",
            ));
        }

        let mut referenced_handoffs = BTreeSet::new();
        for transition in required_root_transitions.as_slice() {
            let Some((layer_id, artifact_id, handoff_id)) = transition.recovery_handoff_binding()
            else {
                continue;
            };
            let Some(handoff) = distribution_handoffs
                .as_slice()
                .iter()
                .find(|candidate| candidate.handoff_id() == handoff_id)
            else {
                return Err(InstructionContractError(
                    "recovery transition references a missing distribution handoff",
                ));
            };
            if handoff.layer_id() != layer_id || handoff.distribution_artifact_id() != artifact_id {
                return Err(InstructionContractError(
                    "recovery transition disagrees with its frozen distribution handoff",
                ));
            }
            referenced_handoffs.insert(handoff_id.as_str().to_owned());
        }
        let available_handoffs = distribution_handoffs
            .as_slice()
            .iter()
            .map(|value| value.handoff_id().as_str().to_owned())
            .collect::<BTreeSet<_>>();
        if referenced_handoffs != available_handoffs {
            return Err(InstructionContractError(
                "distribution handoff set is not the exact referenced recovery subset",
            ));
        }
        if distribution_handoffs.as_slice().iter().any(|handoff| {
            handoff.manual_target_mode() != manual_target_mode
                || handoff.manual_actor_username() != &repository_username
                || handoff.working_infobase_identity() != working_infobase_identity.as_ref()
        }) {
            return Err(InstructionContractError(
                "distribution handoff actor or manual target differs from the instruction",
            ));
        }

        Ok(Self {
            support_action_id,
            purpose,
            manual_target_mode,
            repository_username,
            working_infobase_identity,
            correction_base_cursor,
            correction_lock_targets,
            finalization_lock_targets,
            required_root_transitions,
            required_content_restorations,
            distribution_handoffs,
            handoff_revalidations,
            desired_support_graph_digest,
            desired_repository_content_digest,
        })
    }
}

macro_rules! define_corrective_instruction_pair {
    ($digest:ident, $full:ident, $mode:ty, { $($mode_fields:tt)* }) => {
        #[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct $digest {
            kind: CorrectSupportPrerequisiteKind,
            support_action_id: UnicaId,
            purpose: SupportActionPurpose,
            manual_target_mode: $mode,
            repository_username: RepositoryUsername,
            $($mode_fields)*
            correction_base_cursor: RepositoryHistoryCursor,
            correction_lock_targets: SupportRecoveryLockTargets,
            finalization_lock_targets: SupportRecoveryLockTargets,
            required_root_transitions: SupportRecoveryTransitions,
            required_root_delta_digest: Sha256Digest,
            required_content_restorations: SupportContentRestorations,
            required_content_delta_digest: Sha256Digest,
            distribution_handoffs: SupportRecoveryDistributionHandoffs,
            handoff_revalidations: SupportRecoveryHandoffRevalidations,
            desired_support_graph_digest: Sha256Digest,
            desired_repository_content_digest: Sha256Digest,
            off_support_forbidden: InstructionTrueLiteral,
            commit_as_separate_recovery_version: InstructionTrueLiteral,
            release_all_locks: InstructionTrueLiteral,
            resume_with: BranchedStatusResume,
        }

        #[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct $full {
            kind: CorrectSupportPrerequisiteKind,
            support_action_id: UnicaId,
            purpose: SupportActionPurpose,
            manual_target_mode: $mode,
            repository_username: RepositoryUsername,
            $($mode_fields)*
            correction_base_cursor: RepositoryHistoryCursor,
            correction_lock_targets: SupportRecoveryLockTargets,
            finalization_lock_targets: SupportRecoveryLockTargets,
            required_root_transitions: SupportRecoveryTransitions,
            required_root_delta_digest: Sha256Digest,
            required_content_restorations: SupportContentRestorations,
            required_content_delta_digest: Sha256Digest,
            distribution_handoffs: SupportRecoveryDistributionHandoffs,
            handoff_revalidations: SupportRecoveryHandoffRevalidations,
            desired_support_graph_digest: Sha256Digest,
            desired_repository_content_digest: Sha256Digest,
            off_support_forbidden: InstructionTrueLiteral,
            commit_as_separate_recovery_version: InstructionTrueLiteral,
            release_all_locks: InstructionTrueLiteral,
            resume_with: BranchedStatusResume,
            corrective_instruction_digest: Sha256Digest,
        }
    };
}

define_corrective_instruction_pair!(
    ReservedOriginalSupportCorrectiveInstructionDigestRecord,
    ReservedOriginalSupportCorrectiveInstruction,
    ReservedOriginalMode,
    {}
);
define_corrective_instruction_pair!(
    SeparateWorkingInfobaseSupportCorrectiveInstructionDigestRecord,
    SeparateWorkingInfobaseSupportCorrectiveInstruction,
    SeparateWorkingInfobaseMode,
    { working_infobase_identity: ManualWorkingInfobaseIdentity, }
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
enum SupportCorrectiveInstructionDigestRecordKind {
    ReservedOriginal(ReservedOriginalSupportCorrectiveInstructionDigestRecord),
    SeparateWorkingInfobase(SeparateWorkingInfobaseSupportCorrectiveInstructionDigestRecord),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct SupportCorrectiveInstructionDigestRecord(
    SupportCorrectiveInstructionDigestRecordKind,
);

impl contract_digest_record_sealed::Sealed for SupportCorrectiveInstructionDigestRecord {}
impl ContractDigestRecord for SupportCorrectiveInstructionDigestRecord {}

impl JsonSchema for SupportCorrectiveInstructionDigestRecord {
    fn schema_name() -> Cow<'static, str> {
        "SupportCorrectiveInstructionDigestRecord".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<ReservedOriginalSupportCorrectiveInstructionDigestRecord>(),
            generator
                .subschema_for::<SeparateWorkingInfobaseSupportCorrectiveInstructionDigestRecord>(),
        ])
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
enum SupportCorrectiveInstructionKind {
    ReservedOriginal(ReservedOriginalSupportCorrectiveInstruction),
    SeparateWorkingInfobase(SeparateWorkingInfobaseSupportCorrectiveInstruction),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct SupportCorrectiveInstruction(SupportCorrectiveInstructionKind);

impl JsonSchema for SupportCorrectiveInstruction {
    fn schema_name() -> Cow<'static, str> {
        "SupportCorrectiveInstruction".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<ReservedOriginalSupportCorrectiveInstruction>(),
            generator.subschema_for::<SeparateWorkingInfobaseSupportCorrectiveInstruction>(),
        ])
    }
}

impl SupportCorrectiveInstruction {
    pub(crate) fn new(
        authority: SupportCorrectiveInstructionAuthority,
    ) -> Result<Self, InstructionContractError> {
        let SupportCorrectiveInstructionAuthority {
            support_action_id,
            purpose,
            manual_target_mode,
            repository_username,
            working_infobase_identity,
            correction_base_cursor,
            correction_lock_targets,
            finalization_lock_targets,
            required_root_transitions,
            required_content_restorations,
            distribution_handoffs,
            handoff_revalidations,
            desired_support_graph_digest,
            desired_repository_content_digest,
        } = authority;
        let required_root_delta_digest = required_root_delta_digest(&required_root_transitions)?;
        let required_content_delta_digest =
            required_content_delta_digest(&required_content_restorations)?;
        match (manual_target_mode, working_infobase_identity) {
            (ManualSupportTargetMode::ReservedOriginal, None) => {
                let record = ReservedOriginalSupportCorrectiveInstructionDigestRecord {
                    kind: CorrectSupportPrerequisiteKind::Value,
                    support_action_id,
                    purpose,
                    manual_target_mode: ReservedOriginalMode::Value,
                    repository_username,
                    correction_base_cursor,
                    correction_lock_targets,
                    finalization_lock_targets,
                    required_root_transitions,
                    required_root_delta_digest,
                    required_content_restorations,
                    required_content_delta_digest,
                    distribution_handoffs,
                    handoff_revalidations,
                    desired_support_graph_digest,
                    desired_repository_content_digest,
                    off_support_forbidden: InstructionTrueLiteral,
                    commit_as_separate_recovery_version: InstructionTrueLiteral,
                    release_all_locks: InstructionTrueLiteral,
                    resume_with: BranchedStatusResume::Value,
                };
                let digest_record = SupportCorrectiveInstructionDigestRecord(
                    SupportCorrectiveInstructionDigestRecordKind::ReservedOriginal(record.clone()),
                );
                let corrective_instruction_digest = instruction_digest(
                    &digest_record,
                    "support corrective instruction digest computation failed",
                )?;
                Ok(Self(SupportCorrectiveInstructionKind::ReservedOriginal(
                    ReservedOriginalSupportCorrectiveInstruction {
                        kind: record.kind,
                        support_action_id: record.support_action_id,
                        purpose: record.purpose,
                        manual_target_mode: record.manual_target_mode,
                        repository_username: record.repository_username,
                        correction_base_cursor: record.correction_base_cursor,
                        correction_lock_targets: record.correction_lock_targets,
                        finalization_lock_targets: record.finalization_lock_targets,
                        required_root_transitions: record.required_root_transitions,
                        required_root_delta_digest: record.required_root_delta_digest,
                        required_content_restorations: record.required_content_restorations,
                        required_content_delta_digest: record.required_content_delta_digest,
                        distribution_handoffs: record.distribution_handoffs,
                        handoff_revalidations: record.handoff_revalidations,
                        desired_support_graph_digest: record.desired_support_graph_digest,
                        desired_repository_content_digest: record.desired_repository_content_digest,
                        off_support_forbidden: record.off_support_forbidden,
                        commit_as_separate_recovery_version: record
                            .commit_as_separate_recovery_version,
                        release_all_locks: record.release_all_locks,
                        resume_with: record.resume_with,
                        corrective_instruction_digest,
                    },
                )))
            }
            (ManualSupportTargetMode::SeparateWorkingInfobase, Some(identity)) => {
                let record = SeparateWorkingInfobaseSupportCorrectiveInstructionDigestRecord {
                    kind: CorrectSupportPrerequisiteKind::Value,
                    support_action_id,
                    purpose,
                    manual_target_mode: SeparateWorkingInfobaseMode::Value,
                    repository_username,
                    working_infobase_identity: identity,
                    correction_base_cursor,
                    correction_lock_targets,
                    finalization_lock_targets,
                    required_root_transitions,
                    required_root_delta_digest,
                    required_content_restorations,
                    required_content_delta_digest,
                    distribution_handoffs,
                    handoff_revalidations,
                    desired_support_graph_digest,
                    desired_repository_content_digest,
                    off_support_forbidden: InstructionTrueLiteral,
                    commit_as_separate_recovery_version: InstructionTrueLiteral,
                    release_all_locks: InstructionTrueLiteral,
                    resume_with: BranchedStatusResume::Value,
                };
                let digest_record = SupportCorrectiveInstructionDigestRecord(
                    SupportCorrectiveInstructionDigestRecordKind::SeparateWorkingInfobase(
                        record.clone(),
                    ),
                );
                let corrective_instruction_digest = instruction_digest(
                    &digest_record,
                    "support corrective instruction digest computation failed",
                )?;
                Ok(Self(
                    SupportCorrectiveInstructionKind::SeparateWorkingInfobase(
                        SeparateWorkingInfobaseSupportCorrectiveInstruction {
                            kind: record.kind,
                            support_action_id: record.support_action_id,
                            purpose: record.purpose,
                            manual_target_mode: record.manual_target_mode,
                            repository_username: record.repository_username,
                            working_infobase_identity: record.working_infobase_identity,
                            correction_base_cursor: record.correction_base_cursor,
                            correction_lock_targets: record.correction_lock_targets,
                            finalization_lock_targets: record.finalization_lock_targets,
                            required_root_transitions: record.required_root_transitions,
                            required_root_delta_digest: record.required_root_delta_digest,
                            required_content_restorations: record.required_content_restorations,
                            required_content_delta_digest: record.required_content_delta_digest,
                            distribution_handoffs: record.distribution_handoffs,
                            handoff_revalidations: record.handoff_revalidations,
                            desired_support_graph_digest: record.desired_support_graph_digest,
                            desired_repository_content_digest: record
                                .desired_repository_content_digest,
                            off_support_forbidden: record.off_support_forbidden,
                            commit_as_separate_recovery_version: record
                                .commit_as_separate_recovery_version,
                            release_all_locks: record.release_all_locks,
                            resume_with: record.resume_with,
                            corrective_instruction_digest,
                        },
                    ),
                ))
            }
            _ => Err(InstructionContractError(
                "corrective authority violates manual-target presence",
            )),
        }
    }

    pub(super) fn digest_record(&self) -> SupportCorrectiveInstructionDigestRecord {
        match &self.0 {
            SupportCorrectiveInstructionKind::ReservedOriginal(value) => {
                SupportCorrectiveInstructionDigestRecord(
                    SupportCorrectiveInstructionDigestRecordKind::ReservedOriginal(
                        ReservedOriginalSupportCorrectiveInstructionDigestRecord {
                            kind: value.kind,
                            support_action_id: value.support_action_id.clone(),
                            purpose: value.purpose,
                            manual_target_mode: value.manual_target_mode,
                            repository_username: value.repository_username.clone(),
                            correction_base_cursor: value.correction_base_cursor.clone(),
                            correction_lock_targets: value.correction_lock_targets.clone(),
                            finalization_lock_targets: value.finalization_lock_targets.clone(),
                            required_root_transitions: value.required_root_transitions.clone(),
                            required_root_delta_digest: value.required_root_delta_digest.clone(),
                            required_content_restorations: value
                                .required_content_restorations
                                .clone(),
                            required_content_delta_digest: value
                                .required_content_delta_digest
                                .clone(),
                            distribution_handoffs: value.distribution_handoffs.clone(),
                            handoff_revalidations: value.handoff_revalidations.clone(),
                            desired_support_graph_digest: value
                                .desired_support_graph_digest
                                .clone(),
                            desired_repository_content_digest: value
                                .desired_repository_content_digest
                                .clone(),
                            off_support_forbidden: value.off_support_forbidden,
                            commit_as_separate_recovery_version: value
                                .commit_as_separate_recovery_version,
                            release_all_locks: value.release_all_locks,
                            resume_with: value.resume_with,
                        },
                    ),
                )
            }
            SupportCorrectiveInstructionKind::SeparateWorkingInfobase(value) => {
                SupportCorrectiveInstructionDigestRecord(
                    SupportCorrectiveInstructionDigestRecordKind::SeparateWorkingInfobase(
                        SeparateWorkingInfobaseSupportCorrectiveInstructionDigestRecord {
                            kind: value.kind,
                            support_action_id: value.support_action_id.clone(),
                            purpose: value.purpose,
                            manual_target_mode: value.manual_target_mode,
                            repository_username: value.repository_username.clone(),
                            working_infobase_identity: value.working_infobase_identity.clone(),
                            correction_base_cursor: value.correction_base_cursor.clone(),
                            correction_lock_targets: value.correction_lock_targets.clone(),
                            finalization_lock_targets: value.finalization_lock_targets.clone(),
                            required_root_transitions: value.required_root_transitions.clone(),
                            required_root_delta_digest: value.required_root_delta_digest.clone(),
                            required_content_restorations: value
                                .required_content_restorations
                                .clone(),
                            required_content_delta_digest: value
                                .required_content_delta_digest
                                .clone(),
                            distribution_handoffs: value.distribution_handoffs.clone(),
                            handoff_revalidations: value.handoff_revalidations.clone(),
                            desired_support_graph_digest: value
                                .desired_support_graph_digest
                                .clone(),
                            desired_repository_content_digest: value
                                .desired_repository_content_digest
                                .clone(),
                            off_support_forbidden: value.off_support_forbidden,
                            commit_as_separate_recovery_version: value
                                .commit_as_separate_recovery_version,
                            release_all_locks: value.release_all_locks,
                            resume_with: value.resume_with,
                        },
                    ),
                )
            }
        }
    }

    pub(crate) fn support_action_id(&self) -> &UnicaId {
        match &self.0 {
            SupportCorrectiveInstructionKind::ReservedOriginal(value) => &value.support_action_id,
            SupportCorrectiveInstructionKind::SeparateWorkingInfobase(value) => {
                &value.support_action_id
            }
        }
    }

    pub(crate) const fn purpose(&self) -> SupportActionPurpose {
        match &self.0 {
            SupportCorrectiveInstructionKind::ReservedOriginal(value) => value.purpose,
            SupportCorrectiveInstructionKind::SeparateWorkingInfobase(value) => value.purpose,
        }
    }

    pub(crate) const fn manual_target_mode(&self) -> ManualSupportTargetMode {
        match &self.0 {
            SupportCorrectiveInstructionKind::ReservedOriginal(_) => {
                ManualSupportTargetMode::ReservedOriginal
            }
            SupportCorrectiveInstructionKind::SeparateWorkingInfobase(_) => {
                ManualSupportTargetMode::SeparateWorkingInfobase
            }
        }
    }

    pub(crate) fn repository_username(&self) -> &RepositoryUsername {
        match &self.0 {
            SupportCorrectiveInstructionKind::ReservedOriginal(value) => &value.repository_username,
            SupportCorrectiveInstructionKind::SeparateWorkingInfobase(value) => {
                &value.repository_username
            }
        }
    }

    pub(crate) fn working_infobase_identity(&self) -> Option<&ManualWorkingInfobaseIdentity> {
        match &self.0 {
            SupportCorrectiveInstructionKind::ReservedOriginal(_) => None,
            SupportCorrectiveInstructionKind::SeparateWorkingInfobase(value) => {
                Some(&value.working_infobase_identity)
            }
        }
    }

    pub(crate) fn correction_base_cursor(&self) -> &RepositoryHistoryCursor {
        match &self.0 {
            SupportCorrectiveInstructionKind::ReservedOriginal(value) => {
                &value.correction_base_cursor
            }
            SupportCorrectiveInstructionKind::SeparateWorkingInfobase(value) => {
                &value.correction_base_cursor
            }
        }
    }

    pub(crate) fn required_root_delta_digest(&self) -> &Sha256Digest {
        match &self.0 {
            SupportCorrectiveInstructionKind::ReservedOriginal(value) => {
                &value.required_root_delta_digest
            }
            SupportCorrectiveInstructionKind::SeparateWorkingInfobase(value) => {
                &value.required_root_delta_digest
            }
        }
    }

    pub(crate) fn required_content_delta_digest(&self) -> &Sha256Digest {
        match &self.0 {
            SupportCorrectiveInstructionKind::ReservedOriginal(value) => {
                &value.required_content_delta_digest
            }
            SupportCorrectiveInstructionKind::SeparateWorkingInfobase(value) => {
                &value.required_content_delta_digest
            }
        }
    }

    pub(crate) fn desired_support_graph_digest(&self) -> &Sha256Digest {
        match &self.0 {
            SupportCorrectiveInstructionKind::ReservedOriginal(value) => {
                &value.desired_support_graph_digest
            }
            SupportCorrectiveInstructionKind::SeparateWorkingInfobase(value) => {
                &value.desired_support_graph_digest
            }
        }
    }

    pub(crate) fn desired_repository_content_digest(&self) -> &Sha256Digest {
        match &self.0 {
            SupportCorrectiveInstructionKind::ReservedOriginal(value) => {
                &value.desired_repository_content_digest
            }
            SupportCorrectiveInstructionKind::SeparateWorkingInfobase(value) => {
                &value.desired_repository_content_digest
            }
        }
    }

    pub(crate) fn corrective_instruction_digest(&self) -> &Sha256Digest {
        match &self.0 {
            SupportCorrectiveInstructionKind::ReservedOriginal(value) => {
                &value.corrective_instruction_digest
            }
            SupportCorrectiveInstructionKind::SeparateWorkingInfobase(value) => {
                &value.corrective_instruction_digest
            }
        }
    }
}

macro_rules! define_unvalidated_corrective_instruction {
    ($name:ident, $mode:ty, { $($mode_fields:tt)* }) => {
        #[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct $name {
            kind: CorrectSupportPrerequisiteKind,
            support_action_id: UnicaId,
            purpose: SupportActionPurpose,
            manual_target_mode: $mode,
            repository_username: RepositoryUsername,
            $($mode_fields)*
            correction_base_cursor: RepositoryHistoryCursor,
            correction_lock_targets: Vec<SupportRecoveryLockTargetWire>,
            finalization_lock_targets: Vec<SupportRecoveryLockTargetWire>,
            required_root_transitions: SupportRecoveryTransitions,
            required_root_delta_digest: Sha256Digest,
            required_content_restorations: SupportContentRestorations,
            required_content_delta_digest: Sha256Digest,
            distribution_handoffs: Vec<SupportRecoveryDistributionHandoff>,
            handoff_revalidations: Vec<SupportRecoveryHandoffRevalidationWire>,
            desired_support_graph_digest: Sha256Digest,
            desired_repository_content_digest: Sha256Digest,
            off_support_forbidden: InstructionTrueLiteral,
            commit_as_separate_recovery_version: InstructionTrueLiteral,
            release_all_locks: InstructionTrueLiteral,
            resume_with: BranchedStatusResume,
            corrective_instruction_digest: Sha256Digest,
        }
    };
}

define_unvalidated_corrective_instruction!(
    UnvalidatedReservedOriginalSupportCorrectiveInstruction,
    ReservedOriginalMode,
    {}
);
define_unvalidated_corrective_instruction!(
    UnvalidatedSeparateWorkingInfobaseSupportCorrectiveInstruction,
    SeparateWorkingInfobaseMode,
    { working_infobase_identity: ManualWorkingInfobaseIdentity, }
);

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
enum UnvalidatedSupportCorrectiveInstruction {
    ReservedOriginal(UnvalidatedReservedOriginalSupportCorrectiveInstruction),
    SeparateWorkingInfobase(UnvalidatedSeparateWorkingInfobaseSupportCorrectiveInstruction),
}

impl<'de> Deserialize<'de> for SupportCorrectiveInstruction {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = UnvalidatedSupportCorrectiveInstruction::deserialize(deserializer)?;
        let (
            authority,
            observed_root_delta_digest,
            observed_content_delta_digest,
            observed_instruction_digest,
        ) = match wire {
            UnvalidatedSupportCorrectiveInstruction::ReservedOriginal(wire) => {
                let handoffs = SupportRecoveryDistributionHandoffs::new(wire.distribution_handoffs)
                    .map_err(D::Error::custom)?;
                let revalidations =
                    decode_handoff_revalidations(&handoffs, wire.handoff_revalidations)
                        .map_err(D::Error::custom)?;
                (
                    SupportCorrectiveInstructionAuthority::validated(
                        wire.support_action_id,
                        wire.purpose,
                        ManualSupportTargetMode::ReservedOriginal,
                        wire.repository_username,
                        None,
                        wire.correction_base_cursor,
                        decode_support_recovery_lock_targets(wire.correction_lock_targets)
                            .map_err(D::Error::custom)?,
                        decode_support_recovery_lock_targets(wire.finalization_lock_targets)
                            .map_err(D::Error::custom)?,
                        wire.required_root_transitions,
                        wire.required_content_restorations,
                        handoffs,
                        revalidations,
                        wire.desired_support_graph_digest,
                        wire.desired_repository_content_digest,
                    )
                    .map_err(D::Error::custom)?,
                    wire.required_root_delta_digest,
                    wire.required_content_delta_digest,
                    wire.corrective_instruction_digest,
                )
            }
            UnvalidatedSupportCorrectiveInstruction::SeparateWorkingInfobase(wire) => {
                let handoffs = SupportRecoveryDistributionHandoffs::new(wire.distribution_handoffs)
                    .map_err(D::Error::custom)?;
                let revalidations =
                    decode_handoff_revalidations(&handoffs, wire.handoff_revalidations)
                        .map_err(D::Error::custom)?;
                (
                    SupportCorrectiveInstructionAuthority::validated(
                        wire.support_action_id,
                        wire.purpose,
                        ManualSupportTargetMode::SeparateWorkingInfobase,
                        wire.repository_username,
                        Some(wire.working_infobase_identity),
                        wire.correction_base_cursor,
                        decode_support_recovery_lock_targets(wire.correction_lock_targets)
                            .map_err(D::Error::custom)?,
                        decode_support_recovery_lock_targets(wire.finalization_lock_targets)
                            .map_err(D::Error::custom)?,
                        wire.required_root_transitions,
                        wire.required_content_restorations,
                        handoffs,
                        revalidations,
                        wire.desired_support_graph_digest,
                        wire.desired_repository_content_digest,
                    )
                    .map_err(D::Error::custom)?,
                    wire.required_root_delta_digest,
                    wire.required_content_delta_digest,
                    wire.corrective_instruction_digest,
                )
            }
        };
        let value = Self::new(authority).map_err(D::Error::custom)?;
        (value.required_root_delta_digest() == &observed_root_delta_digest
            && value.required_content_delta_digest() == &observed_content_delta_digest
            && value.corrective_instruction_digest() == &observed_instruction_digest)
            .then_some(value)
            .ok_or_else(|| {
                D::Error::custom("support corrective instruction derived or record digest mismatch")
            })
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

impl<'de> Deserialize<'de> for SupportConflictAllowedEvidenceKinds {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let values = <[SupportConflictAllowedEvidenceKind; 2]>::deserialize(deserializer)?;
        (values == Self::canonical().0)
            .then_some(Self(values))
            .ok_or_else(|| D::Error::custom("support conflict evidence kinds are not canonical"))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SupportTransitionConflictWire {
    repository_version: RepositoryVersion,
    #[serde(deserialize_with = "RequiredNullable::deserialize_required")]
    repository_actor: RequiredNullable<RepositoryActorIdentity>,
    object_id: Option<MetadataObjectId>,
    object_display: RepositoryTargetDisplay,
    layer_id: SupportLayerId,
    authorized_transition: SupportTransition,
    external_transition_digest: Sha256Digest,
    overlap_kind: SupportTransitionOverlapKind,
    diagnostic: Diagnostic,
}

fn decode_support_transition_conflicts(
    wires: Vec<SupportTransitionConflictWire>,
    history_order: &dyn SupportHistoryOrderAuthority,
) -> Result<SupportTransitionConflicts, InstructionContractError> {
    let values = wires
        .into_iter()
        .map(|wire| {
            SupportTransitionConflict::from_capability_adapter(
                wire.repository_version,
                wire.repository_actor,
                wire.object_id,
                wire.object_display,
                wire.layer_id,
                wire.authorized_transition,
                wire.external_transition_digest,
                wire.overlap_kind,
                wire.diagnostic,
            )
            .map_err(|_| InstructionContractError("invalid historical support conflict"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    SupportTransitionConflicts::new(values, history_order)
        .map_err(|_| InstructionContractError("invalid historical support conflict ordering"))
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UnvalidatedSupportConflictInstruction {
    kind: CoordinateExternalSupportChangeKind,
    conflict_resolution_id: UnicaId,
    conflicts: Vec<SupportTransitionConflictWire>,
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

    pub(crate) fn conflict_resolution_id(&self) -> &UnicaId {
        &self.conflict_resolution_id
    }

    pub(crate) fn required_final_baseline_digest(&self) -> &Sha256Digest {
        &self.required_final_baseline_digest
    }

    pub(crate) fn support_conflict_instruction_digest(&self) -> &Sha256Digest {
        &self.support_conflict_instruction_digest
    }
}

pub(crate) fn decode_historical_support_conflict_instruction(
    observed: serde_json::Value,
    history_order: &dyn SupportHistoryOrderAuthority,
) -> Result<SupportConflictInstruction, InstructionContractError> {
    let wire = serde_json::from_value::<UnvalidatedSupportConflictInstruction>(observed.clone())
        .map_err(|_| InstructionContractError("support-conflict instruction shape is invalid"))?;
    let record = SupportConflictInstructionDigestRecord {
        kind: wire.kind,
        conflict_resolution_id: wire.conflict_resolution_id,
        conflicts: decode_support_transition_conflicts(wire.conflicts, history_order)?,
        allowed_evidence_kinds: wire.allowed_evidence_kinds,
        required_final_baseline_digest: wire.required_final_baseline_digest,
        automatic_reversal_forbidden: wire.automatic_reversal_forbidden,
        resume_with: wire.resume_with,
    };
    let expected = instruction_digest(
        &record,
        "support-conflict instruction digest computation failed",
    )?;
    let value = SupportConflictInstruction::from_record(record, expected.clone());
    let reconstructed = serde_json::to_value(&value)
        .map_err(|_| InstructionContractError("support-conflict reconstruction failed"))?;
    (expected == wire.support_conflict_instruction_digest && reconstructed == observed)
        .then_some(value)
        .ok_or(InstructionContractError(
            "support-conflict instruction digest mismatch",
        ))
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
// This is the normative conceptual wire union. Keeping its leaves inline preserves
// the type boundary; the bounded size cost is preferable to hidden indirection.
#[allow(clippy::large_enum_variant)]
enum SupportRecoveryExternalActionKind {
    Corrective(SupportCorrectiveInstruction),
    ReleaseLocks(ReleaseRepositoryLocksInstruction),
    CleanWorkingInfobase(CleanManualWorkingInfobaseInstruction),
    CloseReservedOriginal(CloseReservedOriginalDesignerInstruction),
    Conflict(SupportConflictInstruction),
    Evidence(SupportEvidenceInstruction),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct SupportRecoveryExternalAction(SupportRecoveryExternalActionKind);

impl SupportRecoveryExternalAction {
    pub(crate) fn corrective(value: SupportCorrectiveInstruction) -> Self {
        Self(SupportRecoveryExternalActionKind::Corrective(value))
    }

    pub(crate) fn release_locks(value: ReleaseRepositoryLocksInstruction) -> Self {
        Self(SupportRecoveryExternalActionKind::ReleaseLocks(value))
    }

    pub(crate) fn clean_working_infobase(value: CleanManualWorkingInfobaseInstruction) -> Self {
        Self(SupportRecoveryExternalActionKind::CleanWorkingInfobase(
            value,
        ))
    }

    pub(crate) fn close_reserved_original(value: CloseReservedOriginalDesignerInstruction) -> Self {
        Self(SupportRecoveryExternalActionKind::CloseReservedOriginal(
            value,
        ))
    }

    pub(crate) fn conflict(value: SupportConflictInstruction) -> Self {
        Self(SupportRecoveryExternalActionKind::Conflict(value))
    }

    pub(crate) fn evidence(value: SupportEvidenceInstruction) -> Self {
        Self(SupportRecoveryExternalActionKind::Evidence(value))
    }
}

impl JsonSchema for SupportRecoveryExternalAction {
    fn schema_name() -> Cow<'static, str> {
        "SupportRecoveryExternalAction".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<SupportCorrectiveInstruction>(),
            generator.subschema_for::<ReleaseRepositoryLocksInstruction>(),
            generator.subschema_for::<CleanManualWorkingInfobaseInstruction>(),
            generator.subschema_for::<CloseReservedOriginalDesignerInstruction>(),
            generator.subschema_for::<SupportConflictInstruction>(),
            generator.subschema_for::<SupportEvidenceInstruction>(),
        ])
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
