#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::branched_development::contracts::repository::{
        RepositoryHistoryCursor, RepositoryOwnerIdentity, RepositoryTargetIdentity,
        RepositoryUpdateLockReason,
    };
    use crate::domain::branched_development::contracts::scalars::{
        RepositoryIdentityComponent, RepositoryTargetDisplay, RepositoryUsername, RequiredNullable,
    };
    use crate::domain::branched_development::contracts::schema::audit_json_schema;
    use crate::domain::branched_development::contracts::support::{
        ManualActorLockInventoryProof, ManualWorkingInfobaseIdentity,
        ReservedOriginalLeaseStopEvidence, SupportRecoveryDisposition,
    };
    use crate::domain::branched_development::{
        CapabilityRowId, MetadataObjectId, Sha256Digest, UnicaId,
    };
    use schemars::{schema_for, JsonSchema};
    use serde::de::DeserializeOwned;
    use serde_json::{json, Value};

    const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const C: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
    const ID_1: &str = "11111111-1111-4111-8111-111111111111";
    const ID_2: &str = "22222222-2222-4222-8222-222222222222";
    const ID_3: &str = "33333333-3333-4333-8333-333333333333";
    const OBJECT_A: &str = "00000000-0000-0000-0000-000000000001";
    const OBJECT_B: &str = "00000000-0000-0000-0000-000000000002";

    fn digest(value: &str) -> Sha256Digest {
        Sha256Digest::parse(value).unwrap()
    }

    fn id(value: &str) -> UnicaId {
        UnicaId::parse(value).unwrap()
    }

    fn object(value: &str) -> MetadataObjectId {
        MetadataObjectId::parse(value).unwrap()
    }

    fn capability(value: &str) -> CapabilityRowId {
        CapabilityRowId::parse(value).unwrap()
    }

    fn display(value: &str) -> RepositoryTargetDisplay {
        RepositoryTargetDisplay::parse(value).unwrap()
    }

    fn cursor(version: &str, digest_value: &str) -> RepositoryHistoryCursor {
        serde_json::from_value(json!({
            "throughVersion": version,
            "historyPrefixDigest": digest_value,
        }))
        .unwrap()
    }

    fn root_identity() -> RepositoryTargetIdentity {
        serde_json::from_value(json!({ "targetKind": "configurationRoot" })).unwrap()
    }

    fn object_identity(value: &str) -> RepositoryTargetIdentity {
        serde_json::from_value(json!({
            "targetKind": "developmentObject",
            "objectId": value,
        }))
        .unwrap()
    }

    fn working_identity() -> ManualWorkingInfobaseIdentity {
        ManualWorkingInfobaseIdentity::new(
            RepositoryIdentityComponent::parse("developer-mac").unwrap(),
            RepositoryIdentityComponent::parse("support-work").unwrap(),
        )
        .unwrap()
    }

    fn separate_guard_authorization() -> FrozenSupportRecoveryAuthorizationProjection {
        FrozenSupportRecoveryAuthorizationProjection::separate_test_only(
            id(ID_1),
            digest(A),
            RepositoryUsername::parse("manual-user").unwrap(),
            digest(B),
            digest(B),
        )
    }

    fn reserved_guard_authorization() -> FrozenSupportRecoveryAuthorizationProjection {
        FrozenSupportRecoveryAuthorizationProjection::reserved_test_only(
            id(ID_1),
            digest(A),
            RepositoryUsername::parse("reserved").unwrap(),
            digest(A),
            digest(B),
            capability("reserved-original-exclusive-lease"),
            digest(B),
        )
    }

    fn root_lock() -> SupportRecoveryLockTarget {
        SupportRecoveryLockTarget::configuration_root(
            display("Configuration"),
            vec![
                RepositoryUpdateLockReason::SupportGraphGuard,
                RepositoryUpdateLockReason::UpdateTarget,
            ],
        )
        .unwrap()
    }

    fn object_lock(value: &str, name: &str) -> SupportRecoveryLockTarget {
        SupportRecoveryLockTarget::development_object(
            object(value),
            display(name),
            vec![RepositoryUpdateLockReason::UpdateTarget],
        )
        .unwrap()
    }

    fn locks() -> SupportRecoveryLockTargets {
        SupportRecoveryLockTargets::new(vec![
            root_lock(),
            object_lock(OBJECT_A, "Catalog.A"),
            object_lock(OBJECT_B, "Catalog.B"),
        ])
        .unwrap()
    }

    fn desired_targets() -> SupportRecoveryDesiredTargets {
        SupportRecoveryDesiredTargets::new(vec![
            SupportRecoveryDesiredTarget::root_present(display("Configuration"), digest(A)),
            SupportRecoveryDesiredTarget::object_present(
                object(OBJECT_A),
                display("Catalog.A"),
                digest(B),
            ),
            SupportRecoveryDesiredTarget::object_absent(object(OBJECT_B), display("Catalog.B")),
        ])
        .unwrap()
    }

    fn desired_finalization_plan() -> SupportRecoveryFinalizationPlan {
        SupportRecoveryFinalizationPlan::new(
            SupportRecoveryFinalizationPlanAuthority::desired_test_only(
                SupportRecoveryDisposition::RestoreThenReauthorize,
                locks(),
                desired_targets(),
                cursor("17", A),
                digest(B),
                digest(C),
            ),
        )
        .unwrap()
    }

    fn desired_closure_plan() -> ManualWorkingInfobaseClosurePlan {
        ManualWorkingInfobaseClosurePlan::new(
            ManualWorkingInfobaseClosurePlanAuthority::desired_test_only(
                working_identity(),
                digest(A),
                digest(B),
                digest(C),
                digest(A),
                capability("working-ib-exclusive-lease"),
            ),
        )
        .unwrap()
    }

    fn separate_completed_guard_json() -> Value {
        let root_lock = serde_json::to_value(root_lock()).unwrap();
        let root_state = json!({
            "targetKind": "configurationRoot",
            "state": "present",
            "repositoryVersion": "19",
            "targetFingerprint": A,
        });
        let before = serde_json::to_value(cursor("19", B)).unwrap();
        let after = before.clone();
        let closure_proof = working_closure_proof();
        let terminalization_receipt =
            SupportRecoveryAuthorizationTerminalizationReceipt::from_fields(
                id(ID_1),
                digest(A),
                id(ID_3),
                CompletedSupportRecoveryAuthorizationOutcome::Cancelled,
            )
            .unwrap();
        json!({
            "outcome": "completed",
            "guardReceiptId": ID_1,
            "guardReleaseReceiptId": ID_2,
            "manualTargetMode": "separateWorkingInfobase",
            "manualWorkingInfobaseClosureProof": closure_proof,
            "finalizationPlanDigest": A,
            "plannedLockTargets": [root_lock.clone()],
            "acquiredInOrder": [root_lock.clone()],
            "historyFromCursor": cursor("17", A),
            "historyThroughCursor": before.clone(),
            "historyPartitionDigest": C,
            "supportGraphRecheckedUnderGuard": true,
            "correctiveBeforeStateBindingVerified": true,
            "contentRecheckedUnderGuard": true,
            "originalRecheckedUnderGuard": true,
            "selectiveUpdateProof": {
                "planDigest": B,
                "guardReceiptId": ID_1,
                "plannedTargets": [root_state.clone()],
                "appliedTargets": [root_state],
                "expectedTargetRevisionMapDigest": A,
                "appliedTargetRevisionMapDigest": A,
                "lockTargets": [root_lock.clone()],
                "acquiredRootFirst": [root_lock.clone()],
                "releasedInReverseOrder": [root_lock.clone()],
                "releaseVerified": true,
                "beforeOriginalTargetFingerprintMapDigest": A,
                "updatePerformed": false,
                "structuralConfirmationUsed": false,
                "verifiedOriginalTargetFingerprintDigest": B,
                "observedBeforeCursor": before.clone(),
                "observedAfterCursor": after.clone(),
                "selectiveObjectsCapabilityId": "selective-objects",
                "proofDigest": C,
            },
            "postReleaseObservedHistoryCursor": after.clone(),
            "postReleaseHistoryPartition": {
                "fromExclusive": before,
                "throughInclusive": after,
                "entries": [],
                "partitionDigest": A,
            },
            "authorizationTerminalizationReceipt": terminalization_receipt,
            "authorizationOutcome": "cancelled",
            "releasedInReverseOrder": [root_lock],
            "releaseVerified": true,
            "proofDigest": C,
        })
    }

    fn materialized_closure_plan() -> ManualWorkingInfobaseClosurePlan {
        ManualWorkingInfobaseClosurePlan::new(
            ManualWorkingInfobaseClosurePlanAuthority::materialized_test_only(
                working_identity(),
                digest(A),
                digest(B),
                digest(C),
                digest(A),
                cursor("19", B),
                digest(C),
                capability("working-ib-exclusive-lease"),
            ),
        )
        .unwrap()
    }

    fn working_stop_evidence() -> ManualWorkingInfobaseStopEvidence {
        let plan = materialized_closure_plan();
        ManualWorkingInfobaseStopEvidence::new(
            &plan,
            ManualWorkingInfobaseStopAuthority::lease_busy_test_only(
                &plan,
                RequiredNullable::null(),
            )
            .unwrap(),
        )
        .unwrap()
    }

    fn working_closure_proof() -> ManualWorkingInfobaseClosureProof {
        let plan = materialized_closure_plan();
        let authority = ManualWorkingInfobaseClosureExecutionAuthority::matching_test_only(
            &plan,
            id(ID_1),
            id(ID_2),
        )
        .unwrap();
        ManualWorkingInfobaseClosureProof::new(&plan, authority).unwrap()
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

    assert_not_deserialize_owned!(SupportRecoveryFinalizationPlan);
    assert_not_deserialize_owned!(SupportRecoveryFinalizationPlanAuthority);
    assert_not_deserialize_owned!(ManualWorkingInfobaseClosurePlan);
    assert_not_deserialize_owned!(ManualWorkingInfobaseClosurePlanAuthority);
    assert_not_deserialize_owned!(ManualWorkingInfobaseClosureProof);
    assert_not_deserialize_owned!(ManualWorkingInfobaseClosureExecutionAuthority);
    assert_not_deserialize_owned!(ManualWorkingInfobaseStopEvidence);
    assert_not_deserialize_owned!(ManualWorkingInfobaseStopAuthority);
    assert_not_deserialize_owned!(SupportRecoveryGuardProof);
    assert_not_deserialize_owned!(SupportRecoveryGuardAuthority);

    #[test]
    fn finalization_plan_is_root_first_canonical_and_hashes_explicit_null() {
        let plan = desired_finalization_plan();
        let encoded = serde_json::to_value(&plan).unwrap();
        assert_eq!(encoded["lockTargets"][0]["targetKind"], "configurationRoot");
        assert_eq!(
            encoded["desiredTargets"][0]["targetKind"],
            "configurationRoot"
        );
        assert_eq!(encoded["materializedSelectiveUpdatePlan"], Value::Null);
        assert!(encoded.get("planDigest").is_some());

        assert!(SupportRecoveryLockTargets::new(vec![
            object_lock(OBJECT_A, "Catalog.A"),
            root_lock(),
        ])
        .is_err());
        assert!(SupportRecoveryDesiredTargets::new(vec![
            SupportRecoveryDesiredTarget::root_present(display("Configuration"), digest(A)),
            SupportRecoveryDesiredTarget::object_present(
                object(OBJECT_A),
                display("Catalog.A"),
                digest(B),
            ),
            SupportRecoveryDesiredTarget::object_absent(object(OBJECT_A), display("Catalog.A"),),
        ])
        .is_err());

        assert_closed::<SupportRecoveryDesiredTarget>();
        assert_closed::<SupportRecoveryFinalizationPlanDigestRecord>();
        assert_closed::<SupportRecoveryFinalizationPlan>();
    }

    #[test]
    fn completed_guard_authorization_outcome_is_derived_from_disposition() {
        assert_eq!(
            completed_authorization_outcome(SupportRecoveryDisposition::RestoreThenReauthorize),
            CompletedSupportRecoveryAuthorizationOutcome::Cancelled,
        );
        assert_eq!(
            completed_authorization_outcome(
                SupportRecoveryDisposition::PreserveExternalAndReauthorize,
            ),
            CompletedSupportRecoveryAuthorizationOutcome::Cancelled,
        );
        assert_eq!(
            completed_authorization_outcome(SupportRecoveryDisposition::RestoreThenAbandon),
            CompletedSupportRecoveryAuthorizationOutcome::AbandonmentFinalized,
        );
    }

    #[test]
    fn lock_reason_contract_is_contextual_for_root_and_object_targets() {
        let valid = serde_json::to_value(root_lock()).unwrap();
        assert!(schema_accepts::<SupportRecoveryLockTarget>(&valid));

        let mut empty = valid.clone();
        empty["reasons"] = json!([]);
        assert!(!schema_accepts::<SupportRecoveryLockTarget>(&empty));

        let mut duplicate = valid.clone();
        duplicate["reasons"] = json!(["supportGraphGuard", "supportGraphGuard"]);
        assert!(!schema_accepts::<SupportRecoveryLockTarget>(&duplicate));

        for forbidden in ["parentClosure", "referenceClosure"] {
            let mut wrong_root_role = valid.clone();
            wrong_root_role["reasons"] = json!(["supportGraphGuard", forbidden]);
            assert!(!schema_accepts::<SupportRecoveryLockTarget>(
                &wrong_root_role
            ));
        }
        assert!(SupportRecoveryLockTarget::configuration_root(
            display("Configuration"),
            vec![
                RepositoryUpdateLockReason::SupportGraphGuard,
                RepositoryUpdateLockReason::ParentClosure,
            ],
        )
        .is_err());
        assert!(SupportRecoveryLockTarget::configuration_root(
            display("Configuration"),
            vec![
                RepositoryUpdateLockReason::SupportGraphGuard,
                RepositoryUpdateLockReason::ReferenceClosure,
            ],
        )
        .is_err());

        let mut object_with_guard =
            serde_json::to_value(object_lock(OBJECT_A, "Catalog.A")).unwrap();
        object_with_guard["reasons"] = json!(["supportGraphGuard", "updateTarget"]);
        assert!(!schema_accepts::<SupportRecoveryLockTarget>(
            &object_with_guard
        ));
        assert!(SupportRecoveryLockTarget::development_object(
            object(OBJECT_A),
            display("Catalog.A"),
            vec![RepositoryUpdateLockReason::SupportGraphGuard],
        )
        .is_err());

        let exact_root_roles = SupportRecoveryLockTarget::configuration_root(
            display("Configuration"),
            vec![
                RepositoryUpdateLockReason::SupportGraphGuard,
                RepositoryUpdateLockReason::UpdateTarget,
                RepositoryUpdateLockReason::StructuralClosure,
            ],
        )
        .unwrap();
        assert!(schema_accepts::<SupportRecoveryLockTarget>(
            &serde_json::to_value(exact_root_roles).unwrap()
        ));
        let exact_object_roles = SupportRecoveryLockTarget::development_object(
            object(OBJECT_A),
            display("Catalog.A"),
            vec![
                RepositoryUpdateLockReason::UpdateTarget,
                RepositoryUpdateLockReason::ParentClosure,
                RepositoryUpdateLockReason::ReferenceClosure,
                RepositoryUpdateLockReason::StructuralClosure,
            ],
        )
        .unwrap();
        assert!(schema_accepts::<SupportRecoveryLockTarget>(
            &serde_json::to_value(exact_object_roles).unwrap()
        ));
    }

    #[test]
    fn closure_plan_has_exact_desired_and_materialized_leaves() {
        let desired = serde_json::to_value(desired_closure_plan()).unwrap();
        assert_eq!(desired["state"], "desired");
        assert!(desired.get("workingInfobaseBaseCursor").is_none());
        assert!(desired.get("recordedObjectVersionMapDigest").is_none());

        let materialized_plan = materialized_closure_plan();
        let materialized = serde_json::to_value(&materialized_plan).unwrap();
        assert_eq!(materialized["state"], "materialized");
        assert!(materialized.get("workingInfobaseBaseCursor").is_some());
        assert!(materialized.get("recordedObjectVersionMapDigest").is_some());

        let mut splice = desired;
        splice["workingInfobaseBaseCursor"] = serde_json::to_value(cursor("19", B)).unwrap();
        assert!(!schema_accepts::<ManualWorkingInfobaseClosurePlan>(&splice));

        assert_closed::<ManualWorkingInfobaseClosurePlanDigestRecord>();
        assert_closed::<ManualWorkingInfobaseClosurePlan>();
    }

    #[test]
    fn closure_proof_requires_the_exact_materialized_plan_and_lease_window() {
        let desired = desired_closure_plan();
        assert!(
            ManualWorkingInfobaseClosureExecutionAuthority::matching_test_only(
                &desired,
                id(ID_1),
                id(ID_2),
            )
            .is_err()
        );

        let materialized = materialized_closure_plan();
        let authority = ManualWorkingInfobaseClosureExecutionAuthority::matching_test_only(
            &materialized,
            id(ID_1),
            id(ID_2),
        )
        .unwrap();
        let proof = ManualWorkingInfobaseClosureProof::new(&materialized, authority).unwrap();
        let encoded = serde_json::to_value(&proof).unwrap();
        assert_eq!(encoded["exclusiveLeaseReceiptId"], ID_1);
        assert_eq!(encoded["exclusiveLeaseReleaseReceiptId"], ID_2);
        assert_eq!(encoded["currentEqualsRecordedBase"], true);
        assert_eq!(encoded["noLocalSupportDelta"], true);
        assert_eq!(encoded["noUncommittedConfigurationDelta"], true);

        let another = ManualWorkingInfobaseClosurePlan::new(
            ManualWorkingInfobaseClosurePlanAuthority::materialized_test_only(
                working_identity(),
                digest(A),
                digest(C),
                digest(C),
                digest(A),
                cursor("19", B),
                digest(C),
                capability("working-ib-exclusive-lease"),
            ),
        )
        .unwrap();
        let substituted = ManualWorkingInfobaseClosureExecutionAuthority::matching_test_only(
            &another,
            id(ID_1),
            id(ID_2),
        )
        .unwrap();
        assert!(ManualWorkingInfobaseClosureProof::new(&materialized, substituted).is_err());

        assert_closed::<ManualWorkingInfobaseClosureProofDigestRecord>();
        assert_closed::<ManualWorkingInfobaseClosureProof>();
    }

    #[test]
    fn both_mode_stop_evidence_branches_are_closed_and_non_terminal() {
        let plan = materialized_closure_plan();
        let busy = ManualWorkingInfobaseStopEvidence::new(
            &plan,
            ManualWorkingInfobaseStopAuthority::lease_busy_test_only(
                &plan,
                RequiredNullable::null(),
            )
            .unwrap(),
        )
        .unwrap();
        let busy_json = serde_json::to_value(&busy).unwrap();
        assert_eq!(busy_json["cause"], "leaseBusy");
        assert_eq!(busy_json["exclusiveLeaseAcquired"], false);
        assert!(busy_json.get("exclusiveLeaseReceiptId").is_none());

        let dirty = ManualWorkingInfobaseStopEvidence::new(
            &plan,
            ManualWorkingInfobaseStopAuthority::lease_acquired_dirty_test_only(
                &plan,
                digest(C),
                digest(B),
                id(ID_1),
                id(ID_2),
            )
            .unwrap(),
        )
        .unwrap();
        let dirty_json = serde_json::to_value(&dirty).unwrap();
        assert_eq!(dirty_json["cause"], "leaseAcquiredDirty");
        assert_eq!(dirty_json["workingInfobaseLeaseReleased"], true);
        assert_eq!(dirty_json["workingInfobaseLeaseReleaseVerified"], true);
        assert!(dirty_json.get("leaseOwner").is_none());

        let reserved = ReservedOriginalLeaseStopEvidence::new(
            digest(A),
            capability("reserved-original-exclusive-lease"),
            RequiredNullable::null(),
        )
        .unwrap();
        assert_eq!(
            serde_json::to_value(reserved).unwrap()["cause"],
            "designerSessionOpenOrLeaseBusy"
        );

        assert_closed::<ManualWorkingInfobaseLeaseBusyEvidenceDigestRecord>();
        assert_closed::<ManualWorkingInfobaseDirtyStopEvidenceDigestRecord>();
        assert_closed::<ManualWorkingInfobaseStopEvidence>();
    }

    #[test]
    fn guard_blocked_variants_bind_exact_prefix_and_reverse_compensation() {
        let plan = SupportRecoveryGuardPlanAuthority::test_only(
            separate_guard_authorization(),
            digest(A),
            locks(),
            cursor("17", A),
        );
        let before_root = SupportRecoveryGuardProof::new(
            SupportRecoveryGuardAuthority::blocked_before_root_test_only(
                plan.clone(),
                id(ID_1),
                root_identity(),
                display("Configuration"),
                RequiredNullable::null(),
            )
            .unwrap(),
        )
        .unwrap();
        assert!(matches!(
            before_root.blocked_target_ref(),
            Some(BlockedSupportRecoveryTargetRef::ConfigurationRoot)
        ));
        assert_eq!(
            before_root.blocked_target_display(),
            Some(&display("Configuration"))
        );
        assert!(before_root
            .blocked_owner()
            .is_some_and(|owner| owner.as_ref().is_none()));
        let before_json = serde_json::to_value(&before_root).unwrap();
        assert_eq!(before_json["outcome"], "blockedBeforeRoot");
        assert_eq!(before_json["acquiredInOrder"], json!([]));
        assert_eq!(before_json["releasedInReverseOrder"], json!([]));

        let wrong_root = SupportRecoveryGuardPlanAuthority::test_only(
            separate_guard_authorization(),
            digest(A),
            locks(),
            cursor("17", A),
        );
        assert!(
            SupportRecoveryGuardAuthority::blocked_before_root_test_only(
                wrong_root,
                id(ID_1),
                object_identity(OBJECT_A),
                display("Catalog.A"),
                RequiredNullable::null(),
            )
            .is_err()
        );

        let acquired = SupportRecoveryAcquiredLockTargets::new(vec![root_lock()]).unwrap();
        let released = SupportRecoveryReleasedLockTargets::new(vec![root_lock()]).unwrap();
        let partial = SupportRecoveryGuardProof::new(
            SupportRecoveryGuardAuthority::blocked_after_partial_test_only(
                plan.clone(),
                id(ID_2),
                acquired,
                object_identity(OBJECT_A),
                display("Catalog.A"),
                RequiredNullable::null(),
                released,
            )
            .unwrap(),
        )
        .unwrap();
        assert!(matches!(
            partial.blocked_target_ref(),
            Some(BlockedSupportRecoveryTargetRef::DevelopmentObject(object_id))
                if object_id == &object(OBJECT_A)
        ));
        assert_eq!(
            partial.blocked_target_display(),
            Some(&display("Catalog.A"))
        );
        let partial_json = serde_json::to_value(&partial).unwrap();
        assert_eq!(partial_json["outcome"], "blockedAfterPartial");
        assert_eq!(partial_json["acquiredInOrder"].as_array().unwrap().len(), 1);

        let bad_release = SupportRecoveryReleasedLockTargets::new(vec![
            root_lock(),
            object_lock(OBJECT_A, "Catalog.A"),
        ]);
        assert!(bad_release.is_err());
        assert!(
            SupportRecoveryGuardAuthority::blocked_after_partial_test_only(
                plan,
                id(ID_3),
                SupportRecoveryAcquiredLockTargets::new(vec![root_lock()]).unwrap(),
                object_identity(OBJECT_A),
                display("Catalog.A"),
                RequiredNullable::null(),
                SupportRecoveryReleasedLockTargets::new(vec![root_lock()]).unwrap(),
            )
            .is_ok()
        );

        let wrong_next = SupportRecoveryGuardPlanAuthority::test_only(
            separate_guard_authorization(),
            digest(A),
            locks(),
            cursor("17", A),
        );
        assert!(
            SupportRecoveryGuardAuthority::blocked_after_partial_test_only(
                wrong_next,
                id(ID_3),
                SupportRecoveryAcquiredLockTargets::new(vec![root_lock()]).unwrap(),
                object_identity(OBJECT_B),
                display("Catalog.B"),
                RequiredNullable::null(),
                SupportRecoveryReleasedLockTargets::new(vec![root_lock()]).unwrap(),
            )
            .is_err()
        );

        assert_closed::<SupportRecoveryGuardProofDigestRecord>();
        assert_closed::<SupportRecoveryGuardProof>();
    }

    #[test]
    fn complete_guard_stop_enforces_manual_mode_presence_rules() {
        let separate = SupportRecoveryGuardPlanAuthority::test_only(
            separate_guard_authorization(),
            digest(A),
            locks(),
            cursor("17", A),
        );
        let stopped = SupportRecoveryGuardProof::new(
            SupportRecoveryGuardAuthority::stopped_after_complete_guard_test_only(
                separate,
                id(ID_1),
                id(ID_2),
                None,
                None,
                Some(working_stop_evidence()),
                cursor("17", A),
                cursor("19", B),
                digest(C),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            stopped.proof_digest,
            terminalization_digest(&stopped.record, "guard proof regression digest").unwrap()
        );
        let stopped_json = serde_json::to_value(&stopped).unwrap();
        assert_eq!(stopped_json["manualTargetMode"], "separateWorkingInfobase");
        assert!(stopped_json.get("manualActorLockInventoryProof").is_none());
        assert!(stopped_json
            .get("reservedOriginalLeaseStopEvidence")
            .is_none());
        assert!(stopped_json
            .get("manualWorkingInfobaseStopEvidence")
            .is_some());

        let inventory = ManualActorLockInventoryProof::new(
            RepositoryUsername::parse("reserved").unwrap(),
            digest(A),
            digest(A),
        )
        .unwrap();
        let reserved_stop = ReservedOriginalLeaseStopEvidence::new(
            digest(B),
            capability("reserved-original-exclusive-lease"),
            RequiredNullable::<RepositoryOwnerIdentity>::null(),
        )
        .unwrap();
        let mut separate_splice = stopped_json.clone();
        separate_splice["manualActorLockInventoryProof"] =
            serde_json::to_value(&inventory).unwrap();
        separate_splice["reservedOriginalLeaseStopEvidence"] =
            serde_json::to_value(&reserved_stop).unwrap();
        assert!(!schema_accepts::<SupportRecoveryGuardProof>(
            &separate_splice
        ));

        let reserved = SupportRecoveryGuardPlanAuthority::test_only(
            reserved_guard_authorization(),
            digest(A),
            locks(),
            cursor("17", A),
        );
        let stopped = SupportRecoveryGuardProof::new(
            SupportRecoveryGuardAuthority::stopped_after_complete_guard_test_only(
                reserved,
                id(ID_2),
                id(ID_3),
                Some(inventory.clone()),
                Some(reserved_stop),
                None,
                cursor("17", A),
                cursor("19", B),
                digest(C),
            )
            .unwrap(),
        )
        .unwrap();
        let stopped_json = serde_json::to_value(stopped).unwrap();
        assert_eq!(stopped_json["manualTargetMode"], "reservedOriginal");
        assert!(stopped_json.get("manualActorLockInventoryProof").is_some());
        assert!(stopped_json
            .get("reservedOriginalLeaseStopEvidence")
            .is_some());
        let mut reserved_splice = stopped_json.clone();
        reserved_splice
            .as_object_mut()
            .unwrap()
            .remove("reservedOriginalLeaseStopEvidence");
        assert!(!schema_accepts::<SupportRecoveryGuardProof>(
            &reserved_splice
        ));

        let wrong_mode = SupportRecoveryGuardPlanAuthority::test_only(
            separate_guard_authorization(),
            digest(A),
            locks(),
            cursor("17", A),
        );
        assert!(
            SupportRecoveryGuardAuthority::stopped_after_complete_guard_test_only(
                wrong_mode,
                id(ID_3),
                id(ID_1),
                Some(
                    ManualActorLockInventoryProof::new(
                        RepositoryUsername::parse("reserved").unwrap(),
                        digest(A),
                        digest(A),
                    )
                    .unwrap(),
                ),
                Some(
                    ReservedOriginalLeaseStopEvidence::new(
                        digest(B),
                        capability("reserved-original-exclusive-lease"),
                        RequiredNullable::null(),
                    )
                    .unwrap(),
                ),
                None,
                cursor("17", A),
                cursor("19", B),
                digest(C),
            )
            .is_err()
        );

        let proof_schema = schema::<SupportRecoveryGuardProof>();
        let proof_schema_text = serde_json::to_string(&proof_schema).unwrap();
        let separate_completed = separate_completed_guard_json();
        assert!(schema_accepts::<SupportRecoveryGuardProof>(
            &separate_completed
        ));

        let terminalization_proof = ReservedOriginalTerminalizationProof::new(
            digest(A),
            capability("reserved-original-exclusive-lease"),
            id(ID_1),
            id(ID_2),
            digest(B),
            digest(B),
        )
        .unwrap();
        let mut reserved_completed = separate_completed.clone();
        reserved_completed["manualTargetMode"] = json!("reservedOriginal");
        reserved_completed["manualActorLockInventoryProof"] =
            serde_json::to_value(&inventory).unwrap();
        reserved_completed["reservedOriginalTerminalizationProof"] =
            serde_json::to_value(&terminalization_proof).unwrap();
        reserved_completed
            .as_object_mut()
            .unwrap()
            .remove("manualWorkingInfobaseClosureProof");
        assert!(schema_accepts::<SupportRecoveryGuardProof>(
            &reserved_completed
        ));

        let mut cross_mode_completed = separate_completed;
        cross_mode_completed["manualActorLockInventoryProof"] =
            serde_json::to_value(&inventory).unwrap();
        cross_mode_completed["reservedOriginalTerminalizationProof"] =
            serde_json::to_value(&terminalization_proof).unwrap();
        assert!(!schema_accepts::<SupportRecoveryGuardProof>(
            &cross_mode_completed
        ));

        reserved_completed
            .as_object_mut()
            .unwrap()
            .remove("reservedOriginalTerminalizationProof");
        assert!(!schema_accepts::<SupportRecoveryGuardProof>(
            &reserved_completed
        ));
        assert!(proof_schema_text.contains("deferredRepositoryAdvance"));
        assert_closed::<SupportRecoveryGuardProofDigestRecord>();
        assert_closed::<SupportRecoveryGuardProof>();
    }

    #[test]
    fn reserved_guard_rejects_evidence_from_another_frozen_authorization() {
        let authorization = FrozenSupportRecoveryAuthorizationProjection::reserved_test_only(
            id(ID_1),
            digest(A),
            RepositoryUsername::parse("reserved").unwrap(),
            digest(A),
            digest(B),
            capability("reserved-original-exclusive-lease"),
            digest(C),
        );
        let plan = SupportRecoveryGuardPlanAuthority::test_only(
            authorization,
            digest(A),
            locks(),
            cursor("17", A),
        );
        let foreign_inventory = ManualActorLockInventoryProof::new(
            RepositoryUsername::parse("other-actor").unwrap(),
            digest(A),
            digest(A),
        )
        .unwrap();
        let foreign_stop = ReservedOriginalLeaseStopEvidence::new(
            digest(A),
            capability("other-exclusive-lease"),
            RequiredNullable::null(),
        )
        .unwrap();
        assert!(
            SupportRecoveryGuardAuthority::stopped_after_complete_guard_test_only(
                plan.clone(),
                id(ID_2),
                id(ID_3),
                Some(foreign_inventory),
                Some(foreign_stop),
                None,
                cursor("17", A),
                cursor("19", B),
                digest(C),
            )
            .is_err()
        );

        let matching_inventory = ManualActorLockInventoryProof::new(
            RepositoryUsername::parse("reserved").unwrap(),
            digest(A),
            digest(A),
        )
        .unwrap();
        let wrong_baseline_inventory = ManualActorLockInventoryProof::new(
            RepositoryUsername::parse("reserved").unwrap(),
            digest(C),
            digest(C),
        )
        .unwrap();
        let matching_stop = ReservedOriginalLeaseStopEvidence::new(
            digest(B),
            capability("reserved-original-exclusive-lease"),
            RequiredNullable::null(),
        )
        .unwrap();
        let wrong_identity_stop = ReservedOriginalLeaseStopEvidence::new(
            digest(A),
            capability("reserved-original-exclusive-lease"),
            RequiredNullable::null(),
        )
        .unwrap();
        let wrong_capability_stop = ReservedOriginalLeaseStopEvidence::new(
            digest(B),
            capability("other-exclusive-lease"),
            RequiredNullable::null(),
        )
        .unwrap();
        for (inventory, stop) in [
            (wrong_baseline_inventory, matching_stop),
            (matching_inventory.clone(), wrong_identity_stop),
            (matching_inventory, wrong_capability_stop),
        ] {
            assert!(
                SupportRecoveryGuardAuthority::stopped_after_complete_guard_test_only(
                    plan.clone(),
                    id(ID_2),
                    id(ID_3),
                    Some(inventory),
                    Some(stop),
                    None,
                    cursor("17", A),
                    cursor("19", B),
                    digest(C),
                )
                .is_err()
            );
        }
    }

    #[test]
    fn completed_reserved_guard_binds_inventory_identity_capability_and_fingerprint() {
        let plan = SupportRecoveryGuardPlanAuthority::test_only(
            reserved_guard_authorization(),
            digest(A),
            locks(),
            cursor("17", A),
        );
        let inventory = ManualActorLockInventoryProof::new(
            RepositoryUsername::parse("reserved").unwrap(),
            digest(A),
            digest(A),
        )
        .unwrap();
        let terminalization = ReservedOriginalTerminalizationProof::new(
            digest(B),
            capability("reserved-original-exclusive-lease"),
            id(ID_1),
            id(ID_2),
            digest(B),
            digest(B),
        )
        .unwrap();
        assert!(completed_mode_evidence_matches_authorization(
            &plan,
            Some(&inventory),
            Some(&terminalization),
            None,
        ));
        let foreign_inventory = ManualActorLockInventoryProof::new(
            RepositoryUsername::parse("reserved").unwrap(),
            digest(C),
            digest(C),
        )
        .unwrap();
        assert!(!completed_mode_evidence_matches_authorization(
            &plan,
            Some(&foreign_inventory),
            Some(&terminalization),
            None,
        ));
        let foreign_actor_inventory = ManualActorLockInventoryProof::new(
            RepositoryUsername::parse("other-actor").unwrap(),
            digest(A),
            digest(A),
        )
        .unwrap();
        assert!(!completed_mode_evidence_matches_authorization(
            &plan,
            Some(&foreign_actor_inventory),
            Some(&terminalization),
            None,
        ));

        let foreign_identity = ReservedOriginalTerminalizationProof::new(
            digest(A),
            capability("reserved-original-exclusive-lease"),
            id(ID_1),
            id(ID_2),
            digest(B),
            digest(B),
        )
        .unwrap();
        let foreign_capability = ReservedOriginalTerminalizationProof::new(
            digest(B),
            capability("other-exclusive-lease"),
            id(ID_1),
            id(ID_2),
            digest(B),
            digest(B),
        )
        .unwrap();
        let foreign_fingerprint = ReservedOriginalTerminalizationProof::new(
            digest(B),
            capability("reserved-original-exclusive-lease"),
            id(ID_1),
            id(ID_2),
            digest(C),
            digest(C),
        )
        .unwrap();
        for foreign in [foreign_identity, foreign_capability, foreign_fingerprint] {
            assert!(!completed_mode_evidence_matches_authorization(
                &plan,
                Some(&inventory),
                Some(&foreign),
                None,
            ));
        }

        let separate = SupportRecoveryGuardPlanAuthority::test_only(
            separate_guard_authorization(),
            digest(A),
            locks(),
            cursor("17", A),
        );
        let lease_stop = ReservedOriginalLeaseStopEvidence::new(
            digest(B),
            capability("reserved-original-exclusive-lease"),
            RequiredNullable::null(),
        )
        .unwrap();
        assert!(!stopped_mode_evidence_matches_authorization(
            &separate,
            Some(&inventory),
            None,
            None,
        ));
        assert!(!stopped_mode_evidence_matches_authorization(
            &separate,
            None,
            Some(&lease_stop),
            None,
        ));
        assert!(!completed_mode_evidence_matches_authorization(
            &separate,
            Some(&inventory),
            None,
            None,
        ));
        assert!(!completed_mode_evidence_matches_authorization(
            &separate,
            None,
            Some(&terminalization),
            None,
        ));
        assert!(stopped_mode_evidence_matches_authorization(
            &separate,
            None,
            None,
            Some(&working_stop_evidence()),
        ));
        assert!(completed_mode_evidence_matches_authorization(
            &separate,
            None,
            None,
            Some(&working_closure_proof()),
        ));
    }
}
use super::repository::{
    DeferredRepositoryAdvance, RepositoryHistoryCursor, RepositoryOwnerIdentity,
    RepositoryTargetIdentity, RepositoryUpdateLockReason, SelectiveRepositoryUpdatePlan,
    SelectiveRepositoryUpdateProof, ValidatedRepositoryHistoryPartition,
};
use super::scalars::{RepositoryTargetDisplay, RequiredNullable};
use super::schema::one_of_schema;
use super::support::{
    FrozenSupportRecoveryAuthorizationProjection, ManualActorLockInventoryProof,
    ManualSupportTargetMode, ManualWorkingInfobaseIdentity, ReservedOriginalLeaseStopEvidence,
    ReservedOriginalTerminalizationProof, SupportRecoveryDisposition,
};
use super::support_recovery_authority::SupportRecoveryAuthorityToken;
use crate::domain::branched_development::canonical_json::{
    canonical_contract_digest, contract_digest_record_sealed, ContractDigestRecord,
};
use crate::domain::branched_development::{
    CapabilityRowId, MetadataObjectId, Sha256Digest, UnicaId,
};
use schemars::{json_schema, JsonSchema, Schema, SchemaGenerator};
use serde::Serialize;
use std::borrow::Cow;
use std::fmt;

const MAX_TERMINALIZATION_TARGETS: usize = 100_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SupportTerminalizationContractError(&'static str);

impl fmt::Display for SupportTerminalizationContractError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.0)
    }
}

impl std::error::Error for SupportTerminalizationContractError {}

macro_rules! wire_literal {
    ($name:ident, $wire:literal) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, JsonSchema)]
        enum $name {
            #[serde(rename = $wire)]
            Value,
        }
    };
}

wire_literal!(ConfigurationRootKind, "configurationRoot");
wire_literal!(DevelopmentObjectKind, "developmentObject");
wire_literal!(PresentState, "present");
wire_literal!(AbsentState, "absent");
wire_literal!(DesiredClosureState, "desired");
wire_literal!(MaterializedClosureState, "materialized");
wire_literal!(LeaseBusyCause, "leaseBusy");
wire_literal!(LeaseAcquiredDirtyCause, "leaseAcquiredDirty");
wire_literal!(BlockedBeforeRootOutcome, "blockedBeforeRoot");
wire_literal!(BlockedAfterPartialOutcome, "blockedAfterPartial");
wire_literal!(
    StoppedAfterCompleteGuardOutcome,
    "stoppedAfterCompleteGuard"
);
wire_literal!(UnchangedAuthorizationOutcome, "unchanged");
wire_literal!(CompletedGuardOutcome, "completed");
wire_literal!(ReservedOriginalModeLiteral, "reservedOriginal");
wire_literal!(
    SeparateWorkingInfobaseModeLiteral,
    "separateWorkingInfobase"
);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) enum CompletedSupportRecoveryAuthorizationOutcome {
    Cancelled,
    AbandonmentFinalized,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SupportRecoveryAuthorizationTerminalizationReceiptDigestRecord {
    support_action_id: UnicaId,
    support_action_digest: Sha256Digest,
    terminalization_receipt_id: UnicaId,
    authorization_outcome: CompletedSupportRecoveryAuthorizationOutcome,
}

impl contract_digest_record_sealed::Sealed
    for SupportRecoveryAuthorizationTerminalizationReceiptDigestRecord
{
}
impl ContractDigestRecord for SupportRecoveryAuthorizationTerminalizationReceiptDigestRecord {}

/// Capability-backed durable authorization terminalization.  The receipt is
/// minted only behind the support-recovery token and is retained in the final
/// guard proof, so `authorizationOutcome` is no longer a bare derived literal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SupportRecoveryAuthorizationTerminalizationReceipt {
    support_action_id: UnicaId,
    support_action_digest: Sha256Digest,
    terminalization_receipt_id: UnicaId,
    authorization_outcome: CompletedSupportRecoveryAuthorizationOutcome,
    receipt_digest: Sha256Digest,
}

impl SupportRecoveryAuthorizationTerminalizationReceipt {
    pub(crate) fn from_approved(
        _token: &SupportRecoveryAuthorityToken,
        support_action_id: UnicaId,
        support_action_digest: Sha256Digest,
        terminalization_receipt_id: UnicaId,
        authorization_outcome: CompletedSupportRecoveryAuthorizationOutcome,
    ) -> Result<Self, SupportTerminalizationContractError> {
        Self::from_fields(
            support_action_id,
            support_action_digest,
            terminalization_receipt_id,
            authorization_outcome,
        )
    }

    fn from_fields(
        support_action_id: UnicaId,
        support_action_digest: Sha256Digest,
        terminalization_receipt_id: UnicaId,
        authorization_outcome: CompletedSupportRecoveryAuthorizationOutcome,
    ) -> Result<Self, SupportTerminalizationContractError> {
        let record = SupportRecoveryAuthorizationTerminalizationReceiptDigestRecord {
            support_action_id,
            support_action_digest,
            terminalization_receipt_id,
            authorization_outcome,
        };
        let receipt_digest = terminalization_digest(
            &record,
            "support recovery authorization terminalization receipt digest failed",
        )?;
        Ok(Self {
            support_action_id: record.support_action_id,
            support_action_digest: record.support_action_digest,
            terminalization_receipt_id: record.terminalization_receipt_id,
            authorization_outcome: record.authorization_outcome,
            receipt_digest,
        })
    }

    pub(crate) const fn support_action_id(&self) -> &UnicaId {
        &self.support_action_id
    }

    pub(crate) const fn support_action_digest(&self) -> &Sha256Digest {
        &self.support_action_digest
    }

    pub(crate) const fn terminalization_receipt_id(&self) -> &UnicaId {
        &self.terminalization_receipt_id
    }

    pub(crate) const fn authorization_outcome(
        &self,
    ) -> CompletedSupportRecoveryAuthorizationOutcome {
        self.authorization_outcome
    }

    pub(crate) const fn receipt_digest(&self) -> &Sha256Digest {
        &self.receipt_digest
    }
}

const fn completed_authorization_outcome(
    disposition: SupportRecoveryDisposition,
) -> CompletedSupportRecoveryAuthorizationOutcome {
    match disposition {
        SupportRecoveryDisposition::RestoreThenReauthorize
        | SupportRecoveryDisposition::PreserveExternalAndReauthorize => {
            CompletedSupportRecoveryAuthorizationOutcome::Cancelled
        }
        SupportRecoveryDisposition::RestoreThenAbandon => {
            CompletedSupportRecoveryAuthorizationOutcome::AbandonmentFinalized
        }
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
        "TerminalizationTrueLiteral".into()
    }
    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({ "type": "boolean", "const": true })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FalseLiteral;

impl Serialize for FalseLiteral {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_bool(false)
    }
}

impl JsonSchema for FalseLiteral {
    fn inline_schema() -> bool {
        true
    }
    fn schema_name() -> Cow<'static, str> {
        "TerminalizationFalseLiteral".into()
    }
    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({ "type": "boolean", "const": false })
    }
}

fn exact_lock_reason_sequence_schema(values: &[&str]) -> Schema {
    let prefix_items = values
        .iter()
        .map(|value| json_schema!({ "type": "string", "const": value }))
        .collect::<Vec<_>>();
    json_schema!({
        "type": "array",
        "prefixItems": prefix_items,
        "items": false,
        "minItems": values.len(),
        "maxItems": values.len(),
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
struct RootSupportRecoveryLockReasons(Vec<RepositoryUpdateLockReason>);

impl RootSupportRecoveryLockReasons {
    fn new(
        reasons: Vec<RepositoryUpdateLockReason>,
    ) -> Result<Self, SupportTerminalizationContractError> {
        let allowed = [
            RepositoryUpdateLockReason::SupportGraphGuard,
            RepositoryUpdateLockReason::UpdateTarget,
            RepositoryUpdateLockReason::StructuralClosure,
        ];
        if reasons.is_empty()
            || reasons.len() > allowed.len()
            || reasons.first() != Some(&RepositoryUpdateLockReason::SupportGraphGuard)
            || reasons.windows(2).any(|pair| pair[0] >= pair[1])
            || reasons.iter().any(|reason| !allowed.contains(reason))
        {
            return Err(SupportTerminalizationContractError(
                "root support recovery reasons require the guard and exact update/structural roles",
            ));
        }
        Ok(Self(reasons))
    }
}

impl JsonSchema for RootSupportRecoveryLockReasons {
    fn schema_name() -> Cow<'static, str> {
        "RootSupportRecoveryLockReasons".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            exact_lock_reason_sequence_schema(&["supportGraphGuard"]),
            exact_lock_reason_sequence_schema(&["supportGraphGuard", "updateTarget"]),
            exact_lock_reason_sequence_schema(&["supportGraphGuard", "structuralClosure"]),
            exact_lock_reason_sequence_schema(&[
                "supportGraphGuard",
                "updateTarget",
                "structuralClosure",
            ]),
        ])
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
struct ObjectSupportRecoveryLockReasons(Vec<RepositoryUpdateLockReason>);

impl ObjectSupportRecoveryLockReasons {
    fn new(
        reasons: Vec<RepositoryUpdateLockReason>,
    ) -> Result<Self, SupportTerminalizationContractError> {
        if reasons.is_empty()
            || reasons.len() > 4
            || reasons.windows(2).any(|pair| pair[0] >= pair[1])
            || reasons.contains(&RepositoryUpdateLockReason::SupportGraphGuard)
        {
            return Err(SupportTerminalizationContractError(
                "object support recovery reasons require exact update/closure roles",
            ));
        }
        Ok(Self(reasons))
    }
}

impl JsonSchema for ObjectSupportRecoveryLockReasons {
    fn schema_name() -> Cow<'static, str> {
        "ObjectSupportRecoveryLockReasons".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        let ordered = [
            "updateTarget",
            "parentClosure",
            "referenceClosure",
            "structuralClosure",
        ];
        let mut variants = Vec::with_capacity(15);
        for mask in 1_u8..16 {
            let values = ordered
                .iter()
                .enumerate()
                .filter_map(|(index, value)| (mask & (1 << index) != 0).then_some(*value))
                .collect::<Vec<_>>();
            variants.push(exact_lock_reason_sequence_schema(&values));
        }
        one_of_schema(variants)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RootSupportRecoveryLockTarget {
    target_kind: ConfigurationRootKind,
    object_display: RepositoryTargetDisplay,
    reasons: RootSupportRecoveryLockReasons,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ObjectSupportRecoveryLockTarget {
    target_kind: DevelopmentObjectKind,
    object_id: MetadataObjectId,
    object_display: RepositoryTargetDisplay,
    reasons: ObjectSupportRecoveryLockReasons,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
enum SupportRecoveryLockTargetKind {
    ConfigurationRoot(RootSupportRecoveryLockTarget),
    DevelopmentObject(ObjectSupportRecoveryLockTarget),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BlockedSupportRecoveryTargetRef<'a> {
    ConfigurationRoot,
    DevelopmentObject(&'a MetadataObjectId),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct SupportRecoveryLockTarget(SupportRecoveryLockTargetKind);

impl JsonSchema for SupportRecoveryLockTarget {
    fn schema_name() -> Cow<'static, str> {
        "SupportRecoveryLockTarget".into()
    }
    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<RootSupportRecoveryLockTarget>(),
            generator.subschema_for::<ObjectSupportRecoveryLockTarget>(),
        ])
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum SupportRecoveryTargetKey {
    Root,
    Object(String),
}

impl SupportRecoveryLockTarget {
    pub(crate) fn configuration_root(
        object_display: RepositoryTargetDisplay,
        reasons: Vec<RepositoryUpdateLockReason>,
    ) -> Result<Self, SupportTerminalizationContractError> {
        Ok(Self(SupportRecoveryLockTargetKind::ConfigurationRoot(
            RootSupportRecoveryLockTarget {
                target_kind: ConfigurationRootKind::Value,
                object_display,
                reasons: RootSupportRecoveryLockReasons::new(reasons)?,
            },
        )))
    }

    pub(crate) fn development_object(
        object_id: MetadataObjectId,
        object_display: RepositoryTargetDisplay,
        reasons: Vec<RepositoryUpdateLockReason>,
    ) -> Result<Self, SupportTerminalizationContractError> {
        Ok(Self(SupportRecoveryLockTargetKind::DevelopmentObject(
            ObjectSupportRecoveryLockTarget {
                target_kind: DevelopmentObjectKind::Value,
                object_id,
                object_display,
                reasons: ObjectSupportRecoveryLockReasons::new(reasons)?,
            },
        )))
    }

    fn key(&self) -> SupportRecoveryTargetKey {
        match &self.0 {
            SupportRecoveryLockTargetKind::ConfigurationRoot(_) => SupportRecoveryTargetKey::Root,
            SupportRecoveryLockTargetKind::DevelopmentObject(value) => {
                SupportRecoveryTargetKey::Object(value.object_id.as_str().to_owned())
            }
        }
    }

    fn blocked_target_ref(&self) -> BlockedSupportRecoveryTargetRef<'_> {
        match &self.0 {
            SupportRecoveryLockTargetKind::ConfigurationRoot(_) => {
                BlockedSupportRecoveryTargetRef::ConfigurationRoot
            }
            SupportRecoveryLockTargetKind::DevelopmentObject(value) => {
                BlockedSupportRecoveryTargetRef::DevelopmentObject(&value.object_id)
            }
        }
    }

    pub(crate) fn binds_repository_lock_target(
        &self,
        target: super::repository::RepositoryUpdateLockTargetRef<'_>,
    ) -> bool {
        match (&self.0, target) {
            (
                SupportRecoveryLockTargetKind::ConfigurationRoot(expected),
                super::repository::RepositoryUpdateLockTargetRef::ConfigurationRoot {
                    object_display,
                    reasons,
                },
            ) => &expected.object_display == object_display && expected.reasons.0 == reasons,
            (
                SupportRecoveryLockTargetKind::DevelopmentObject(expected),
                super::repository::RepositoryUpdateLockTargetRef::DevelopmentObject {
                    object_id,
                    object_display,
                    reasons,
                },
            ) => {
                &expected.object_id == object_id
                    && &expected.object_display == object_display
                    && expected.reasons.0 == reasons
            }
            _ => false,
        }
    }

    fn matches_identity_and_display(
        &self,
        identity: &RepositoryTargetIdentity,
        display: &RepositoryTargetDisplay,
    ) -> bool {
        let expected_identity = match &self.0 {
            SupportRecoveryLockTargetKind::ConfigurationRoot(value) => {
                if &value.object_display != display {
                    return false;
                }
                serde_json::json!({ "targetKind": "configurationRoot" })
            }
            SupportRecoveryLockTargetKind::DevelopmentObject(value) => {
                if &value.object_display != display {
                    return false;
                }
                serde_json::json!({
                    "targetKind": "developmentObject",
                    "objectId": value.object_id.as_str(),
                })
            }
        };
        serde_json::to_value(identity).is_ok_and(|observed| observed == expected_identity)
    }
}

fn validate_forward_targets(values: &[SupportRecoveryLockTarget]) -> bool {
    !values.is_empty()
        && values.len() <= MAX_TERMINALIZATION_TARGETS
        && values.first().map(SupportRecoveryLockTarget::key)
            == Some(SupportRecoveryTargetKey::Root)
        && values.windows(2).all(|pair| pair[0].key() < pair[1].key())
}

fn validate_reverse_targets(values: &[SupportRecoveryLockTarget]) -> bool {
    !values.is_empty()
        && values.len() <= MAX_TERMINALIZATION_TARGETS
        && values.last().map(SupportRecoveryLockTarget::key) == Some(SupportRecoveryTargetKey::Root)
        && values.windows(2).all(|pair| pair[0].key() > pair[1].key())
}

macro_rules! target_collection {
    ($name:ident, $validator:ident, $schema_min:literal) => {
        #[derive(Debug, Clone, PartialEq, Eq, Serialize)]
        #[serde(transparent)]
        pub(crate) struct $name(Vec<SupportRecoveryLockTarget>);

        impl $name {
            pub(crate) fn new(
                values: Vec<SupportRecoveryLockTarget>,
            ) -> Result<Self, SupportTerminalizationContractError> {
                $validator(&values)
                    .then_some(Self(values))
                    .ok_or(SupportTerminalizationContractError(
                        "support recovery lock targets violate canonical order",
                    ))
            }

            pub(crate) fn as_slice(&self) -> &[SupportRecoveryLockTarget] {
                &self.0
            }
        }

        impl JsonSchema for $name {
            fn schema_name() -> Cow<'static, str> {
                stringify!($name).into()
            }
            fn json_schema(generator: &mut SchemaGenerator) -> Schema {
                json_schema!({
                    "type": "array",
                    "items": generator.subschema_for::<SupportRecoveryLockTarget>(),
                    "minItems": $schema_min,
                    "maxItems": MAX_TERMINALIZATION_TARGETS,
                    "uniqueItems": true,
                })
            }
        }
    };
}

target_collection!(SupportRecoveryLockTargets, validate_forward_targets, 1);
target_collection!(
    SupportRecoveryAcquiredLockTargets,
    validate_forward_targets,
    1
);
target_collection!(
    SupportRecoveryReleasedLockTargets,
    validate_reverse_targets,
    1
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
#[serde(transparent)]
struct EmptySupportRecoveryLockTargets([SupportRecoveryLockTarget; 0]);

impl JsonSchema for EmptySupportRecoveryLockTargets {
    fn schema_name() -> Cow<'static, str> {
        "EmptySupportRecoveryLockTargets".into()
    }
    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "array",
            "items": generator.subschema_for::<SupportRecoveryLockTarget>(),
            "minItems": 0,
            "maxItems": 0,
            "uniqueItems": true,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RootPresentDesiredTarget {
    target_kind: ConfigurationRootKind,
    state: PresentState,
    object_display: RepositoryTargetDisplay,
    desired_fingerprint: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ObjectPresentDesiredTarget {
    target_kind: DevelopmentObjectKind,
    state: PresentState,
    object_id: MetadataObjectId,
    object_display: RepositoryTargetDisplay,
    desired_fingerprint: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ObjectAbsentDesiredTarget {
    target_kind: DevelopmentObjectKind,
    state: AbsentState,
    object_id: MetadataObjectId,
    object_display: RepositoryTargetDisplay,
    expected_absent: TrueLiteral,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub(crate) enum SupportRecoveryDesiredTarget {
    RootPresent(RootPresentDesiredTarget),
    ObjectPresent(ObjectPresentDesiredTarget),
    ObjectAbsent(ObjectAbsentDesiredTarget),
}

impl JsonSchema for SupportRecoveryDesiredTarget {
    fn schema_name() -> Cow<'static, str> {
        "SupportRecoveryDesiredTarget".into()
    }
    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<RootPresentDesiredTarget>(),
            generator.subschema_for::<ObjectPresentDesiredTarget>(),
            generator.subschema_for::<ObjectAbsentDesiredTarget>(),
        ])
    }
}

impl SupportRecoveryDesiredTarget {
    pub(crate) fn root_present(
        object_display: RepositoryTargetDisplay,
        desired_fingerprint: Sha256Digest,
    ) -> Self {
        Self::RootPresent(RootPresentDesiredTarget {
            target_kind: ConfigurationRootKind::Value,
            state: PresentState::Value,
            object_display,
            desired_fingerprint,
        })
    }

    pub(crate) fn object_present(
        object_id: MetadataObjectId,
        object_display: RepositoryTargetDisplay,
        desired_fingerprint: Sha256Digest,
    ) -> Self {
        Self::ObjectPresent(ObjectPresentDesiredTarget {
            target_kind: DevelopmentObjectKind::Value,
            state: PresentState::Value,
            object_id,
            object_display,
            desired_fingerprint,
        })
    }

    pub(crate) fn object_absent(
        object_id: MetadataObjectId,
        object_display: RepositoryTargetDisplay,
    ) -> Self {
        Self::ObjectAbsent(ObjectAbsentDesiredTarget {
            target_kind: DevelopmentObjectKind::Value,
            state: AbsentState::Value,
            object_id,
            object_display,
            expected_absent: TrueLiteral,
        })
    }

    fn key(&self) -> SupportRecoveryTargetKey {
        match self {
            Self::RootPresent(_) => SupportRecoveryTargetKey::Root,
            Self::ObjectPresent(value) => {
                SupportRecoveryTargetKey::Object(value.object_id.as_str().to_owned())
            }
            Self::ObjectAbsent(value) => {
                SupportRecoveryTargetKey::Object(value.object_id.as_str().to_owned())
            }
        }
    }

    pub(crate) fn binds_repository_target_state(
        &self,
        target: super::repository::RepositoryTargetStateRef<'_>,
    ) -> bool {
        match (self, target) {
            (
                Self::RootPresent(expected),
                super::repository::RepositoryTargetStateRef::RootPresent {
                    target_fingerprint, ..
                },
            ) => &expected.desired_fingerprint == target_fingerprint,
            (
                Self::ObjectPresent(expected),
                super::repository::RepositoryTargetStateRef::ObjectPresent {
                    object_id,
                    target_fingerprint,
                    ..
                },
            ) => {
                &expected.object_id == object_id
                    && &expected.desired_fingerprint == target_fingerprint
            }
            (
                Self::ObjectAbsent(expected),
                super::repository::RepositoryTargetStateRef::ObjectAbsent { object_id, .. },
            ) => &expected.object_id == object_id,
            _ => false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct SupportRecoveryDesiredTargets(Vec<SupportRecoveryDesiredTarget>);

impl SupportRecoveryDesiredTargets {
    pub(crate) fn new(
        values: Vec<SupportRecoveryDesiredTarget>,
    ) -> Result<Self, SupportTerminalizationContractError> {
        let valid = !values.is_empty()
            && values.len() <= MAX_TERMINALIZATION_TARGETS
            && values.first().map(SupportRecoveryDesiredTarget::key)
                == Some(SupportRecoveryTargetKey::Root)
            && values.windows(2).all(|pair| pair[0].key() < pair[1].key());
        valid
            .then_some(Self(values))
            .ok_or(SupportTerminalizationContractError(
                "desired support recovery targets must be root-first, canonical, and unique",
            ))
    }

    pub(crate) fn as_slice(&self) -> &[SupportRecoveryDesiredTarget] {
        &self.0
    }
}

impl JsonSchema for SupportRecoveryDesiredTargets {
    fn schema_name() -> Cow<'static, str> {
        "SupportRecoveryDesiredTargets".into()
    }
    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "array",
            "items": generator.subschema_for::<SupportRecoveryDesiredTarget>(),
            "minItems": 1,
            "maxItems": MAX_TERMINALIZATION_TARGETS,
            "uniqueItems": true,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SupportRecoveryFinalizationPlanAuthority {
    disposition: SupportRecoveryDisposition,
    lock_targets: SupportRecoveryLockTargets,
    desired_targets: SupportRecoveryDesiredTargets,
    history_from_cursor: RepositoryHistoryCursor,
    materialized_selective_update_plan: Option<SelectiveRepositoryUpdatePlan>,
    desired_support_graph_digest: Sha256Digest,
    desired_repository_content_digest: Sha256Digest,
}

impl SupportRecoveryFinalizationPlanAuthority {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_approved(
        _token: &SupportRecoveryAuthorityToken,
        disposition: SupportRecoveryDisposition,
        lock_targets: SupportRecoveryLockTargets,
        desired_targets: SupportRecoveryDesiredTargets,
        history_from_cursor: RepositoryHistoryCursor,
        materialized_selective_update_plan: Option<SelectiveRepositoryUpdatePlan>,
        desired_support_graph_digest: Sha256Digest,
        desired_repository_content_digest: Sha256Digest,
    ) -> Self {
        Self {
            disposition,
            lock_targets,
            desired_targets,
            history_from_cursor,
            materialized_selective_update_plan,
            desired_support_graph_digest,
            desired_repository_content_digest,
        }
    }

    #[cfg(test)]
    pub(crate) fn desired_test_only(
        disposition: SupportRecoveryDisposition,
        lock_targets: SupportRecoveryLockTargets,
        desired_targets: SupportRecoveryDesiredTargets,
        history_from_cursor: RepositoryHistoryCursor,
        desired_support_graph_digest: Sha256Digest,
        desired_repository_content_digest: Sha256Digest,
    ) -> Self {
        Self {
            disposition,
            lock_targets,
            desired_targets,
            history_from_cursor,
            materialized_selective_update_plan: None,
            desired_support_graph_digest,
            desired_repository_content_digest,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SupportRecoveryFinalizationPlanDigestRecord {
    disposition: SupportRecoveryDisposition,
    lock_targets: SupportRecoveryLockTargets,
    desired_targets: SupportRecoveryDesiredTargets,
    history_from_cursor: RepositoryHistoryCursor,
    materialized_selective_update_plan: RequiredNullable<SelectiveRepositoryUpdatePlan>,
    desired_support_graph_digest: Sha256Digest,
    desired_repository_content_digest: Sha256Digest,
}

impl contract_digest_record_sealed::Sealed for SupportRecoveryFinalizationPlanDigestRecord {}
impl ContractDigestRecord for SupportRecoveryFinalizationPlanDigestRecord {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SupportRecoveryFinalizationPlan {
    disposition: SupportRecoveryDisposition,
    lock_targets: SupportRecoveryLockTargets,
    desired_targets: SupportRecoveryDesiredTargets,
    history_from_cursor: RepositoryHistoryCursor,
    materialized_selective_update_plan: RequiredNullable<SelectiveRepositoryUpdatePlan>,
    desired_support_graph_digest: Sha256Digest,
    desired_repository_content_digest: Sha256Digest,
    plan_digest: Sha256Digest,
}

impl SupportRecoveryFinalizationPlan {
    pub(crate) fn new(
        authority: SupportRecoveryFinalizationPlanAuthority,
    ) -> Result<Self, SupportTerminalizationContractError> {
        let materialized_selective_update_plan = match authority.materialized_selective_update_plan
        {
            Some(plan) => RequiredNullable::value(plan),
            None => RequiredNullable::null(),
        };
        let record = SupportRecoveryFinalizationPlanDigestRecord {
            disposition: authority.disposition,
            lock_targets: authority.lock_targets,
            desired_targets: authority.desired_targets,
            history_from_cursor: authority.history_from_cursor,
            materialized_selective_update_plan,
            desired_support_graph_digest: authority.desired_support_graph_digest,
            desired_repository_content_digest: authority.desired_repository_content_digest,
        };
        let plan_digest = terminalization_digest(
            &record,
            "support recovery finalization plan digest computation failed",
        )?;
        Ok(Self {
            disposition: record.disposition,
            lock_targets: record.lock_targets,
            desired_targets: record.desired_targets,
            history_from_cursor: record.history_from_cursor,
            materialized_selective_update_plan: record.materialized_selective_update_plan,
            desired_support_graph_digest: record.desired_support_graph_digest,
            desired_repository_content_digest: record.desired_repository_content_digest,
            plan_digest,
        })
    }

    pub(crate) fn plan_digest(&self) -> &Sha256Digest {
        &self.plan_digest
    }

    pub(crate) const fn disposition(&self) -> SupportRecoveryDisposition {
        self.disposition
    }

    pub(crate) const fn lock_targets(&self) -> &SupportRecoveryLockTargets {
        &self.lock_targets
    }

    pub(crate) const fn desired_targets(&self) -> &SupportRecoveryDesiredTargets {
        &self.desired_targets
    }

    pub(crate) const fn history_from_cursor(&self) -> &RepositoryHistoryCursor {
        &self.history_from_cursor
    }

    pub(crate) const fn materialized_selective_update_plan(
        &self,
    ) -> Option<&SelectiveRepositoryUpdatePlan> {
        self.materialized_selective_update_plan.as_ref()
    }

    pub(crate) const fn desired_support_graph_digest(&self) -> &Sha256Digest {
        &self.desired_support_graph_digest
    }

    pub(crate) const fn desired_repository_content_digest(&self) -> &Sha256Digest {
        &self.desired_repository_content_digest
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ClosurePlanAuthorityKind {
    Desired,
    Materialized {
        working_infobase_base_cursor: RepositoryHistoryCursor,
        recorded_object_version_map_digest: Sha256Digest,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ManualWorkingInfobaseClosurePlanAuthority {
    working_infobase_identity: ManualWorkingInfobaseIdentity,
    authorization_baseline_digest: Sha256Digest,
    desired_base_fingerprint: Sha256Digest,
    desired_object_fingerprint_map_digest: Sha256Digest,
    desired_support_graph_digest: Sha256Digest,
    exclusive_lease_capability_id: CapabilityRowId,
    kind: ClosurePlanAuthorityKind,
}

impl ManualWorkingInfobaseClosurePlanAuthority {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn desired_from_approved(
        _token: &SupportRecoveryAuthorityToken,
        working_infobase_identity: ManualWorkingInfobaseIdentity,
        authorization_baseline_digest: Sha256Digest,
        desired_base_fingerprint: Sha256Digest,
        desired_object_fingerprint_map_digest: Sha256Digest,
        desired_support_graph_digest: Sha256Digest,
        exclusive_lease_capability_id: CapabilityRowId,
    ) -> Self {
        Self {
            working_infobase_identity,
            authorization_baseline_digest,
            desired_base_fingerprint,
            desired_object_fingerprint_map_digest,
            desired_support_graph_digest,
            exclusive_lease_capability_id,
            kind: ClosurePlanAuthorityKind::Desired,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn materialized_from_approved(
        _token: &SupportRecoveryAuthorityToken,
        working_infobase_identity: ManualWorkingInfobaseIdentity,
        authorization_baseline_digest: Sha256Digest,
        desired_base_fingerprint: Sha256Digest,
        desired_object_fingerprint_map_digest: Sha256Digest,
        desired_support_graph_digest: Sha256Digest,
        working_infobase_base_cursor: RepositoryHistoryCursor,
        recorded_object_version_map_digest: Sha256Digest,
        exclusive_lease_capability_id: CapabilityRowId,
    ) -> Self {
        Self {
            working_infobase_identity,
            authorization_baseline_digest,
            desired_base_fingerprint,
            desired_object_fingerprint_map_digest,
            desired_support_graph_digest,
            exclusive_lease_capability_id,
            kind: ClosurePlanAuthorityKind::Materialized {
                working_infobase_base_cursor,
                recorded_object_version_map_digest,
            },
        }
    }

    #[cfg(test)]
    pub(crate) fn desired_test_only(
        working_infobase_identity: ManualWorkingInfobaseIdentity,
        authorization_baseline_digest: Sha256Digest,
        desired_base_fingerprint: Sha256Digest,
        desired_object_fingerprint_map_digest: Sha256Digest,
        desired_support_graph_digest: Sha256Digest,
        exclusive_lease_capability_id: CapabilityRowId,
    ) -> Self {
        Self {
            working_infobase_identity,
            authorization_baseline_digest,
            desired_base_fingerprint,
            desired_object_fingerprint_map_digest,
            desired_support_graph_digest,
            exclusive_lease_capability_id,
            kind: ClosurePlanAuthorityKind::Desired,
        }
    }

    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn materialized_test_only(
        working_infobase_identity: ManualWorkingInfobaseIdentity,
        authorization_baseline_digest: Sha256Digest,
        desired_base_fingerprint: Sha256Digest,
        desired_object_fingerprint_map_digest: Sha256Digest,
        desired_support_graph_digest: Sha256Digest,
        working_infobase_base_cursor: RepositoryHistoryCursor,
        recorded_object_version_map_digest: Sha256Digest,
        exclusive_lease_capability_id: CapabilityRowId,
    ) -> Self {
        Self {
            working_infobase_identity,
            authorization_baseline_digest,
            desired_base_fingerprint,
            desired_object_fingerprint_map_digest,
            desired_support_graph_digest,
            exclusive_lease_capability_id,
            kind: ClosurePlanAuthorityKind::Materialized {
                working_infobase_base_cursor,
                recorded_object_version_map_digest,
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DesiredManualWorkingInfobaseClosurePlanDigestRecord {
    state: DesiredClosureState,
    working_infobase_identity: ManualWorkingInfobaseIdentity,
    authorization_baseline_digest: Sha256Digest,
    desired_base_fingerprint: Sha256Digest,
    desired_object_fingerprint_map_digest: Sha256Digest,
    desired_support_graph_digest: Sha256Digest,
    exclusive_lease_capability_id: CapabilityRowId,
    clean_state_must_be_reproduced: TrueLiteral,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MaterializedManualWorkingInfobaseClosurePlanDigestRecord {
    state: MaterializedClosureState,
    working_infobase_identity: ManualWorkingInfobaseIdentity,
    authorization_baseline_digest: Sha256Digest,
    desired_base_fingerprint: Sha256Digest,
    desired_object_fingerprint_map_digest: Sha256Digest,
    desired_support_graph_digest: Sha256Digest,
    working_infobase_base_cursor: RepositoryHistoryCursor,
    recorded_object_version_map_digest: Sha256Digest,
    exclusive_lease_capability_id: CapabilityRowId,
    clean_state_must_be_reproduced: TrueLiteral,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
enum ClosurePlanDigestRecordKind {
    Desired(DesiredManualWorkingInfobaseClosurePlanDigestRecord),
    Materialized(MaterializedManualWorkingInfobaseClosurePlanDigestRecord),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct ManualWorkingInfobaseClosurePlanDigestRecord(ClosurePlanDigestRecordKind);

impl contract_digest_record_sealed::Sealed for ManualWorkingInfobaseClosurePlanDigestRecord {}
impl ContractDigestRecord for ManualWorkingInfobaseClosurePlanDigestRecord {}

impl JsonSchema for ManualWorkingInfobaseClosurePlanDigestRecord {
    fn schema_name() -> Cow<'static, str> {
        "ManualWorkingInfobaseClosurePlanDigestRecord".into()
    }
    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<DesiredManualWorkingInfobaseClosurePlanDigestRecord>(),
            generator.subschema_for::<MaterializedManualWorkingInfobaseClosurePlanDigestRecord>(),
        ])
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct DesiredManualWorkingInfobaseClosurePlan {
    state: DesiredClosureState,
    working_infobase_identity: ManualWorkingInfobaseIdentity,
    authorization_baseline_digest: Sha256Digest,
    desired_base_fingerprint: Sha256Digest,
    desired_object_fingerprint_map_digest: Sha256Digest,
    desired_support_graph_digest: Sha256Digest,
    exclusive_lease_capability_id: CapabilityRowId,
    clean_state_must_be_reproduced: TrueLiteral,
    plan_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct MaterializedManualWorkingInfobaseClosurePlan {
    state: MaterializedClosureState,
    working_infobase_identity: ManualWorkingInfobaseIdentity,
    authorization_baseline_digest: Sha256Digest,
    desired_base_fingerprint: Sha256Digest,
    desired_object_fingerprint_map_digest: Sha256Digest,
    desired_support_graph_digest: Sha256Digest,
    working_infobase_base_cursor: RepositoryHistoryCursor,
    recorded_object_version_map_digest: Sha256Digest,
    exclusive_lease_capability_id: CapabilityRowId,
    clean_state_must_be_reproduced: TrueLiteral,
    plan_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub(crate) enum ManualWorkingInfobaseClosurePlan {
    Desired(DesiredManualWorkingInfobaseClosurePlan),
    Materialized(MaterializedManualWorkingInfobaseClosurePlan),
}

impl JsonSchema for ManualWorkingInfobaseClosurePlan {
    fn schema_name() -> Cow<'static, str> {
        "ManualWorkingInfobaseClosurePlan".into()
    }
    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<DesiredManualWorkingInfobaseClosurePlan>(),
            generator.subschema_for::<MaterializedManualWorkingInfobaseClosurePlan>(),
        ])
    }
}

impl ManualWorkingInfobaseClosurePlan {
    pub(crate) fn new(
        authority: ManualWorkingInfobaseClosurePlanAuthority,
    ) -> Result<Self, SupportTerminalizationContractError> {
        match authority.kind {
            ClosurePlanAuthorityKind::Desired => {
                let record = DesiredManualWorkingInfobaseClosurePlanDigestRecord {
                    state: DesiredClosureState::Value,
                    working_infobase_identity: authority.working_infobase_identity,
                    authorization_baseline_digest: authority.authorization_baseline_digest,
                    desired_base_fingerprint: authority.desired_base_fingerprint,
                    desired_object_fingerprint_map_digest: authority
                        .desired_object_fingerprint_map_digest,
                    desired_support_graph_digest: authority.desired_support_graph_digest,
                    exclusive_lease_capability_id: authority.exclusive_lease_capability_id,
                    clean_state_must_be_reproduced: TrueLiteral,
                };
                let plan_digest = terminalization_digest(
                    &ManualWorkingInfobaseClosurePlanDigestRecord(
                        ClosurePlanDigestRecordKind::Desired(record.clone()),
                    ),
                    "desired working-infobase closure plan digest failed",
                )?;
                Ok(Self::Desired(DesiredManualWorkingInfobaseClosurePlan {
                    state: record.state,
                    working_infobase_identity: record.working_infobase_identity,
                    authorization_baseline_digest: record.authorization_baseline_digest,
                    desired_base_fingerprint: record.desired_base_fingerprint,
                    desired_object_fingerprint_map_digest: record
                        .desired_object_fingerprint_map_digest,
                    desired_support_graph_digest: record.desired_support_graph_digest,
                    exclusive_lease_capability_id: record.exclusive_lease_capability_id,
                    clean_state_must_be_reproduced: record.clean_state_must_be_reproduced,
                    plan_digest,
                }))
            }
            ClosurePlanAuthorityKind::Materialized {
                working_infobase_base_cursor,
                recorded_object_version_map_digest,
            } => {
                let record = MaterializedManualWorkingInfobaseClosurePlanDigestRecord {
                    state: MaterializedClosureState::Value,
                    working_infobase_identity: authority.working_infobase_identity,
                    authorization_baseline_digest: authority.authorization_baseline_digest,
                    desired_base_fingerprint: authority.desired_base_fingerprint,
                    desired_object_fingerprint_map_digest: authority
                        .desired_object_fingerprint_map_digest,
                    desired_support_graph_digest: authority.desired_support_graph_digest,
                    working_infobase_base_cursor,
                    recorded_object_version_map_digest,
                    exclusive_lease_capability_id: authority.exclusive_lease_capability_id,
                    clean_state_must_be_reproduced: TrueLiteral,
                };
                let plan_digest = terminalization_digest(
                    &ManualWorkingInfobaseClosurePlanDigestRecord(
                        ClosurePlanDigestRecordKind::Materialized(record.clone()),
                    ),
                    "materialized working-infobase closure plan digest failed",
                )?;
                Ok(Self::Materialized(
                    MaterializedManualWorkingInfobaseClosurePlan {
                        state: record.state,
                        working_infobase_identity: record.working_infobase_identity,
                        authorization_baseline_digest: record.authorization_baseline_digest,
                        desired_base_fingerprint: record.desired_base_fingerprint,
                        desired_object_fingerprint_map_digest: record
                            .desired_object_fingerprint_map_digest,
                        desired_support_graph_digest: record.desired_support_graph_digest,
                        working_infobase_base_cursor: record.working_infobase_base_cursor,
                        recorded_object_version_map_digest: record
                            .recorded_object_version_map_digest,
                        exclusive_lease_capability_id: record.exclusive_lease_capability_id,
                        clean_state_must_be_reproduced: record.clean_state_must_be_reproduced,
                        plan_digest,
                    },
                ))
            }
        }
    }

    pub(crate) fn materialized(
        &self,
    ) -> Result<&MaterializedManualWorkingInfobaseClosurePlan, SupportTerminalizationContractError>
    {
        match self {
            Self::Materialized(value) => Ok(value),
            Self::Desired(_) => Err(SupportTerminalizationContractError(
                "working-infobase closure plan is not materialized",
            )),
        }
    }

    pub(crate) fn plan_digest(&self) -> &Sha256Digest {
        match self {
            Self::Desired(value) => &value.plan_digest,
            Self::Materialized(value) => &value.plan_digest,
        }
    }

    pub(crate) fn working_infobase_identity(&self) -> &ManualWorkingInfobaseIdentity {
        match self {
            Self::Desired(value) => &value.working_infobase_identity,
            Self::Materialized(value) => &value.working_infobase_identity,
        }
    }

    pub(crate) fn authorization_baseline_digest(&self) -> &Sha256Digest {
        match self {
            Self::Desired(value) => &value.authorization_baseline_digest,
            Self::Materialized(value) => &value.authorization_baseline_digest,
        }
    }

    pub(crate) fn desired_base_fingerprint(&self) -> &Sha256Digest {
        match self {
            Self::Desired(value) => &value.desired_base_fingerprint,
            Self::Materialized(value) => &value.desired_base_fingerprint,
        }
    }

    pub(crate) fn desired_object_fingerprint_map_digest(&self) -> &Sha256Digest {
        match self {
            Self::Desired(value) => &value.desired_object_fingerprint_map_digest,
            Self::Materialized(value) => &value.desired_object_fingerprint_map_digest,
        }
    }

    pub(crate) fn desired_support_graph_digest(&self) -> &Sha256Digest {
        match self {
            Self::Desired(value) => &value.desired_support_graph_digest,
            Self::Materialized(value) => &value.desired_support_graph_digest,
        }
    }

    pub(crate) fn exclusive_lease_capability_id(&self) -> &CapabilityRowId {
        match self {
            Self::Desired(value) => &value.exclusive_lease_capability_id,
            Self::Materialized(value) => &value.exclusive_lease_capability_id,
        }
    }

    pub(crate) fn working_infobase_base_cursor(&self) -> Option<&RepositoryHistoryCursor> {
        match self {
            Self::Desired(_) => None,
            Self::Materialized(value) => Some(&value.working_infobase_base_cursor),
        }
    }

    pub(crate) fn recorded_object_version_map_digest(&self) -> Option<&Sha256Digest> {
        match self {
            Self::Desired(_) => None,
            Self::Materialized(value) => Some(&value.recorded_object_version_map_digest),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ManualWorkingInfobaseClosureExecutionAuthority {
    plan_digest: Sha256Digest,
    working_infobase_identity: ManualWorkingInfobaseIdentity,
    authorization_baseline_digest: Sha256Digest,
    exclusive_lease_capability_id: CapabilityRowId,
    exclusive_lease_receipt_id: UnicaId,
    exclusive_lease_release_receipt_id: UnicaId,
    working_infobase_base_cursor: RepositoryHistoryCursor,
    recorded_object_version_map_digest: Sha256Digest,
    final_current_fingerprint: Sha256Digest,
    final_base_fingerprint: Sha256Digest,
    final_object_fingerprint_map_digest: Sha256Digest,
    final_support_graph_digest: Sha256Digest,
}

impl ManualWorkingInfobaseClosureExecutionAuthority {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_approved_observation(
        _token: &SupportRecoveryAuthorityToken,
        plan: &ManualWorkingInfobaseClosurePlan,
        exclusive_lease_receipt_id: UnicaId,
        exclusive_lease_release_receipt_id: UnicaId,
        working_infobase_base_cursor: RepositoryHistoryCursor,
        recorded_object_version_map_digest: Sha256Digest,
        final_current_fingerprint: Sha256Digest,
        final_base_fingerprint: Sha256Digest,
        final_object_fingerprint_map_digest: Sha256Digest,
        final_support_graph_digest: Sha256Digest,
    ) -> Result<Self, SupportTerminalizationContractError> {
        let materialized = plan.materialized()?;
        Ok(Self {
            plan_digest: materialized.plan_digest.clone(),
            working_infobase_identity: materialized.working_infobase_identity.clone(),
            authorization_baseline_digest: materialized.authorization_baseline_digest.clone(),
            exclusive_lease_capability_id: materialized.exclusive_lease_capability_id.clone(),
            exclusive_lease_receipt_id,
            exclusive_lease_release_receipt_id,
            working_infobase_base_cursor,
            recorded_object_version_map_digest,
            final_current_fingerprint,
            final_base_fingerprint,
            final_object_fingerprint_map_digest,
            final_support_graph_digest,
        })
    }

    #[cfg(test)]
    pub(crate) fn matching_test_only(
        plan: &ManualWorkingInfobaseClosurePlan,
        exclusive_lease_receipt_id: UnicaId,
        exclusive_lease_release_receipt_id: UnicaId,
    ) -> Result<Self, SupportTerminalizationContractError> {
        let plan = plan.materialized()?;
        Ok(Self {
            plan_digest: plan.plan_digest.clone(),
            working_infobase_identity: plan.working_infobase_identity.clone(),
            authorization_baseline_digest: plan.authorization_baseline_digest.clone(),
            exclusive_lease_capability_id: plan.exclusive_lease_capability_id.clone(),
            exclusive_lease_receipt_id,
            exclusive_lease_release_receipt_id,
            working_infobase_base_cursor: plan.working_infobase_base_cursor.clone(),
            recorded_object_version_map_digest: plan.recorded_object_version_map_digest.clone(),
            final_current_fingerprint: plan.desired_base_fingerprint.clone(),
            final_base_fingerprint: plan.desired_base_fingerprint.clone(),
            final_object_fingerprint_map_digest: plan.desired_object_fingerprint_map_digest.clone(),
            final_support_graph_digest: plan.desired_support_graph_digest.clone(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ManualWorkingInfobaseClosureProofDigestRecord {
    working_infobase_identity: ManualWorkingInfobaseIdentity,
    authorization_baseline_digest: Sha256Digest,
    plan_digest: Sha256Digest,
    exclusive_lease_capability_id: CapabilityRowId,
    exclusive_lease_receipt_id: UnicaId,
    exclusive_lease_release_receipt_id: UnicaId,
    lease_held_through_inspection_and_terminalization: TrueLiteral,
    working_infobase_base_cursor: RepositoryHistoryCursor,
    final_current_fingerprint: Sha256Digest,
    recorded_object_version_map_digest: Sha256Digest,
    final_base_fingerprint: Sha256Digest,
    final_object_fingerprint_map_digest: Sha256Digest,
    current_equals_recorded_base: TrueLiteral,
    final_support_graph_digest: Sha256Digest,
    no_local_support_delta: TrueLiteral,
    no_uncommitted_configuration_delta: TrueLiteral,
    lease_released: TrueLiteral,
    lease_release_verified: TrueLiteral,
}

impl contract_digest_record_sealed::Sealed for ManualWorkingInfobaseClosureProofDigestRecord {}
impl ContractDigestRecord for ManualWorkingInfobaseClosureProofDigestRecord {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ManualWorkingInfobaseClosureProof {
    working_infobase_identity: ManualWorkingInfobaseIdentity,
    authorization_baseline_digest: Sha256Digest,
    plan_digest: Sha256Digest,
    exclusive_lease_capability_id: CapabilityRowId,
    exclusive_lease_receipt_id: UnicaId,
    exclusive_lease_release_receipt_id: UnicaId,
    lease_held_through_inspection_and_terminalization: TrueLiteral,
    working_infobase_base_cursor: RepositoryHistoryCursor,
    final_current_fingerprint: Sha256Digest,
    recorded_object_version_map_digest: Sha256Digest,
    final_base_fingerprint: Sha256Digest,
    final_object_fingerprint_map_digest: Sha256Digest,
    current_equals_recorded_base: TrueLiteral,
    final_support_graph_digest: Sha256Digest,
    no_local_support_delta: TrueLiteral,
    no_uncommitted_configuration_delta: TrueLiteral,
    lease_released: TrueLiteral,
    lease_release_verified: TrueLiteral,
    proof_digest: Sha256Digest,
}

impl ManualWorkingInfobaseClosureProof {
    pub(crate) fn new(
        plan: &ManualWorkingInfobaseClosurePlan,
        authority: ManualWorkingInfobaseClosureExecutionAuthority,
    ) -> Result<Self, SupportTerminalizationContractError> {
        let plan = plan.materialized()?;
        if authority.plan_digest != plan.plan_digest
            || authority.working_infobase_identity != plan.working_infobase_identity
            || authority.authorization_baseline_digest != plan.authorization_baseline_digest
            || authority.exclusive_lease_capability_id != plan.exclusive_lease_capability_id
            || authority.working_infobase_base_cursor != plan.working_infobase_base_cursor
            || authority.recorded_object_version_map_digest
                != plan.recorded_object_version_map_digest
            || authority.final_current_fingerprint != plan.desired_base_fingerprint
            || authority.final_base_fingerprint != plan.desired_base_fingerprint
            || authority.final_object_fingerprint_map_digest
                != plan.desired_object_fingerprint_map_digest
            || authority.final_support_graph_digest != plan.desired_support_graph_digest
            || authority.exclusive_lease_receipt_id == authority.exclusive_lease_release_receipt_id
        {
            return Err(SupportTerminalizationContractError(
                "working-infobase closure proof does not match its materialized plan",
            ));
        }
        let record = ManualWorkingInfobaseClosureProofDigestRecord {
            working_infobase_identity: authority.working_infobase_identity,
            authorization_baseline_digest: authority.authorization_baseline_digest,
            plan_digest: authority.plan_digest,
            exclusive_lease_capability_id: authority.exclusive_lease_capability_id,
            exclusive_lease_receipt_id: authority.exclusive_lease_receipt_id,
            exclusive_lease_release_receipt_id: authority.exclusive_lease_release_receipt_id,
            lease_held_through_inspection_and_terminalization: TrueLiteral,
            working_infobase_base_cursor: authority.working_infobase_base_cursor,
            final_current_fingerprint: authority.final_current_fingerprint,
            recorded_object_version_map_digest: authority.recorded_object_version_map_digest,
            final_base_fingerprint: authority.final_base_fingerprint,
            final_object_fingerprint_map_digest: authority.final_object_fingerprint_map_digest,
            current_equals_recorded_base: TrueLiteral,
            final_support_graph_digest: authority.final_support_graph_digest,
            no_local_support_delta: TrueLiteral,
            no_uncommitted_configuration_delta: TrueLiteral,
            lease_released: TrueLiteral,
            lease_release_verified: TrueLiteral,
        };
        let proof_digest = terminalization_digest(
            &record,
            "working-infobase closure proof digest computation failed",
        )?;
        Ok(Self {
            working_infobase_identity: record.working_infobase_identity,
            authorization_baseline_digest: record.authorization_baseline_digest,
            plan_digest: record.plan_digest,
            exclusive_lease_capability_id: record.exclusive_lease_capability_id,
            exclusive_lease_receipt_id: record.exclusive_lease_receipt_id,
            exclusive_lease_release_receipt_id: record.exclusive_lease_release_receipt_id,
            lease_held_through_inspection_and_terminalization: record
                .lease_held_through_inspection_and_terminalization,
            working_infobase_base_cursor: record.working_infobase_base_cursor,
            final_current_fingerprint: record.final_current_fingerprint,
            recorded_object_version_map_digest: record.recorded_object_version_map_digest,
            final_base_fingerprint: record.final_base_fingerprint,
            final_object_fingerprint_map_digest: record.final_object_fingerprint_map_digest,
            current_equals_recorded_base: record.current_equals_recorded_base,
            final_support_graph_digest: record.final_support_graph_digest,
            no_local_support_delta: record.no_local_support_delta,
            no_uncommitted_configuration_delta: record.no_uncommitted_configuration_delta,
            lease_released: record.lease_released,
            lease_release_verified: record.lease_release_verified,
            proof_digest,
        })
    }

    pub(crate) const fn working_infobase_identity(&self) -> &ManualWorkingInfobaseIdentity {
        &self.working_infobase_identity
    }

    pub(crate) const fn plan_digest(&self) -> &Sha256Digest {
        &self.plan_digest
    }

    pub(crate) const fn exclusive_lease_receipt_id(&self) -> &UnicaId {
        &self.exclusive_lease_receipt_id
    }

    pub(crate) const fn exclusive_lease_release_receipt_id(&self) -> &UnicaId {
        &self.exclusive_lease_release_receipt_id
    }

    pub(crate) const fn proof_digest(&self) -> &Sha256Digest {
        &self.proof_digest
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ManualWorkingInfobaseStopAuthorityKind {
    LeaseBusy {
        lease_owner: RequiredNullable<RepositoryOwnerIdentity>,
    },
    LeaseAcquiredDirty {
        observed_working_infobase_fingerprint: Sha256Digest,
        observed_support_graph_digest: Sha256Digest,
        exclusive_lease_receipt_id: UnicaId,
        exclusive_lease_release_receipt_id: UnicaId,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ManualWorkingInfobaseStopAuthority {
    plan_digest: Sha256Digest,
    working_infobase_identity: ManualWorkingInfobaseIdentity,
    exclusive_lease_capability_id: CapabilityRowId,
    expected_repository_fingerprint: Sha256Digest,
    kind: ManualWorkingInfobaseStopAuthorityKind,
}

impl ManualWorkingInfobaseStopAuthority {
    pub(crate) fn lease_busy_from_approved(
        _token: &SupportRecoveryAuthorityToken,
        plan: &ManualWorkingInfobaseClosurePlan,
        lease_owner: RequiredNullable<RepositoryOwnerIdentity>,
    ) -> Result<Self, SupportTerminalizationContractError> {
        Self::lease_busy(plan, lease_owner)
    }

    #[cfg(test)]
    pub(crate) fn lease_busy_test_only(
        plan: &ManualWorkingInfobaseClosurePlan,
        lease_owner: RequiredNullable<RepositoryOwnerIdentity>,
    ) -> Result<Self, SupportTerminalizationContractError> {
        Self::lease_busy(plan, lease_owner)
    }

    fn lease_busy(
        plan: &ManualWorkingInfobaseClosurePlan,
        lease_owner: RequiredNullable<RepositoryOwnerIdentity>,
    ) -> Result<Self, SupportTerminalizationContractError> {
        let plan = plan.materialized()?;
        Ok(Self {
            plan_digest: plan.plan_digest.clone(),
            working_infobase_identity: plan.working_infobase_identity.clone(),
            exclusive_lease_capability_id: plan.exclusive_lease_capability_id.clone(),
            expected_repository_fingerprint: plan.desired_base_fingerprint.clone(),
            kind: ManualWorkingInfobaseStopAuthorityKind::LeaseBusy { lease_owner },
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn lease_acquired_dirty_from_approved(
        _token: &SupportRecoveryAuthorityToken,
        plan: &ManualWorkingInfobaseClosurePlan,
        observed_working_infobase_fingerprint: Sha256Digest,
        observed_support_graph_digest: Sha256Digest,
        exclusive_lease_receipt_id: UnicaId,
        exclusive_lease_release_receipt_id: UnicaId,
    ) -> Result<Self, SupportTerminalizationContractError> {
        Self::lease_acquired_dirty(
            plan,
            observed_working_infobase_fingerprint,
            observed_support_graph_digest,
            exclusive_lease_receipt_id,
            exclusive_lease_release_receipt_id,
        )
    }

    #[cfg(test)]
    pub(crate) fn lease_acquired_dirty_test_only(
        plan: &ManualWorkingInfobaseClosurePlan,
        observed_working_infobase_fingerprint: Sha256Digest,
        observed_support_graph_digest: Sha256Digest,
        exclusive_lease_receipt_id: UnicaId,
        exclusive_lease_release_receipt_id: UnicaId,
    ) -> Result<Self, SupportTerminalizationContractError> {
        Self::lease_acquired_dirty(
            plan,
            observed_working_infobase_fingerprint,
            observed_support_graph_digest,
            exclusive_lease_receipt_id,
            exclusive_lease_release_receipt_id,
        )
    }

    fn lease_acquired_dirty(
        plan: &ManualWorkingInfobaseClosurePlan,
        observed_working_infobase_fingerprint: Sha256Digest,
        observed_support_graph_digest: Sha256Digest,
        exclusive_lease_receipt_id: UnicaId,
        exclusive_lease_release_receipt_id: UnicaId,
    ) -> Result<Self, SupportTerminalizationContractError> {
        let plan = plan.materialized()?;
        if observed_working_infobase_fingerprint == plan.desired_base_fingerprint
            || exclusive_lease_receipt_id == exclusive_lease_release_receipt_id
        {
            return Err(SupportTerminalizationContractError(
                "dirty stop requires a real delta and distinct lease receipts",
            ));
        }
        Ok(Self {
            plan_digest: plan.plan_digest.clone(),
            working_infobase_identity: plan.working_infobase_identity.clone(),
            exclusive_lease_capability_id: plan.exclusive_lease_capability_id.clone(),
            expected_repository_fingerprint: plan.desired_base_fingerprint.clone(),
            kind: ManualWorkingInfobaseStopAuthorityKind::LeaseAcquiredDirty {
                observed_working_infobase_fingerprint,
                observed_support_graph_digest,
                exclusive_lease_receipt_id,
                exclusive_lease_release_receipt_id,
            },
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ManualWorkingInfobaseLeaseBusyEvidenceDigestRecord {
    cause: LeaseBusyCause,
    working_infobase_identity: ManualWorkingInfobaseIdentity,
    #[serde(deserialize_with = "RequiredNullable::deserialize_required")]
    lease_owner: RequiredNullable<RepositoryOwnerIdentity>,
    closure_plan_digest: Sha256Digest,
    exclusive_lease_capability_id: CapabilityRowId,
    exclusive_lease_acquired: FalseLiteral,
}

impl contract_digest_record_sealed::Sealed for ManualWorkingInfobaseLeaseBusyEvidenceDigestRecord {}
impl ContractDigestRecord for ManualWorkingInfobaseLeaseBusyEvidenceDigestRecord {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ManualWorkingInfobaseLeaseBusyEvidence {
    cause: LeaseBusyCause,
    working_infobase_identity: ManualWorkingInfobaseIdentity,
    #[serde(deserialize_with = "RequiredNullable::deserialize_required")]
    lease_owner: RequiredNullable<RepositoryOwnerIdentity>,
    closure_plan_digest: Sha256Digest,
    exclusive_lease_capability_id: CapabilityRowId,
    exclusive_lease_acquired: FalseLiteral,
    lease_busy_evidence_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ManualWorkingInfobaseDirtyStopEvidenceDigestRecord {
    cause: LeaseAcquiredDirtyCause,
    working_infobase_identity: ManualWorkingInfobaseIdentity,
    closure_plan_digest: Sha256Digest,
    exclusive_lease_capability_id: CapabilityRowId,
    expected_repository_fingerprint: Sha256Digest,
    observed_working_infobase_fingerprint: Sha256Digest,
    observed_support_graph_digest: Sha256Digest,
    exclusive_lease_receipt_id: UnicaId,
    exclusive_lease_release_receipt_id: UnicaId,
    working_infobase_lease_released: TrueLiteral,
    working_infobase_lease_release_verified: TrueLiteral,
}

impl contract_digest_record_sealed::Sealed for ManualWorkingInfobaseDirtyStopEvidenceDigestRecord {}
impl ContractDigestRecord for ManualWorkingInfobaseDirtyStopEvidenceDigestRecord {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ManualWorkingInfobaseDirtyStopEvidence {
    cause: LeaseAcquiredDirtyCause,
    working_infobase_identity: ManualWorkingInfobaseIdentity,
    closure_plan_digest: Sha256Digest,
    exclusive_lease_capability_id: CapabilityRowId,
    expected_repository_fingerprint: Sha256Digest,
    observed_working_infobase_fingerprint: Sha256Digest,
    observed_support_graph_digest: Sha256Digest,
    exclusive_lease_receipt_id: UnicaId,
    exclusive_lease_release_receipt_id: UnicaId,
    working_infobase_lease_released: TrueLiteral,
    working_infobase_lease_release_verified: TrueLiteral,
    stop_evidence_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub(crate) enum ManualWorkingInfobaseStopEvidence {
    LeaseBusy(ManualWorkingInfobaseLeaseBusyEvidence),
    LeaseAcquiredDirty(ManualWorkingInfobaseDirtyStopEvidence),
}

impl JsonSchema for ManualWorkingInfobaseStopEvidence {
    fn schema_name() -> Cow<'static, str> {
        "ManualWorkingInfobaseStopEvidence".into()
    }
    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<ManualWorkingInfobaseLeaseBusyEvidence>(),
            generator.subschema_for::<ManualWorkingInfobaseDirtyStopEvidence>(),
        ])
    }
}

impl ManualWorkingInfobaseStopEvidence {
    pub(crate) fn new(
        plan: &ManualWorkingInfobaseClosurePlan,
        authority: ManualWorkingInfobaseStopAuthority,
    ) -> Result<Self, SupportTerminalizationContractError> {
        let plan = plan.materialized()?;
        if authority.plan_digest != plan.plan_digest
            || authority.working_infobase_identity != plan.working_infobase_identity
            || authority.exclusive_lease_capability_id != plan.exclusive_lease_capability_id
            || authority.expected_repository_fingerprint != plan.desired_base_fingerprint
        {
            return Err(SupportTerminalizationContractError(
                "working-infobase stop evidence belongs to another closure plan",
            ));
        }
        match authority.kind {
            ManualWorkingInfobaseStopAuthorityKind::LeaseBusy { lease_owner } => {
                let record = ManualWorkingInfobaseLeaseBusyEvidenceDigestRecord {
                    cause: LeaseBusyCause::Value,
                    working_infobase_identity: authority.working_infobase_identity,
                    lease_owner,
                    closure_plan_digest: authority.plan_digest,
                    exclusive_lease_capability_id: authority.exclusive_lease_capability_id,
                    exclusive_lease_acquired: FalseLiteral,
                };
                let lease_busy_evidence_digest = terminalization_digest(
                    &record,
                    "working-infobase lease-busy evidence digest failed",
                )?;
                Ok(Self::LeaseBusy(ManualWorkingInfobaseLeaseBusyEvidence {
                    cause: record.cause,
                    working_infobase_identity: record.working_infobase_identity,
                    lease_owner: record.lease_owner,
                    closure_plan_digest: record.closure_plan_digest,
                    exclusive_lease_capability_id: record.exclusive_lease_capability_id,
                    exclusive_lease_acquired: record.exclusive_lease_acquired,
                    lease_busy_evidence_digest,
                }))
            }
            ManualWorkingInfobaseStopAuthorityKind::LeaseAcquiredDirty {
                observed_working_infobase_fingerprint,
                observed_support_graph_digest,
                exclusive_lease_receipt_id,
                exclusive_lease_release_receipt_id,
            } => {
                if observed_working_infobase_fingerprint
                    == authority.expected_repository_fingerprint
                    || exclusive_lease_receipt_id == exclusive_lease_release_receipt_id
                {
                    return Err(SupportTerminalizationContractError(
                        "dirty stop lacks a dirty delta or a distinct lease window",
                    ));
                }
                let record = ManualWorkingInfobaseDirtyStopEvidenceDigestRecord {
                    cause: LeaseAcquiredDirtyCause::Value,
                    working_infobase_identity: authority.working_infobase_identity,
                    closure_plan_digest: authority.plan_digest,
                    exclusive_lease_capability_id: authority.exclusive_lease_capability_id,
                    expected_repository_fingerprint: authority.expected_repository_fingerprint,
                    observed_working_infobase_fingerprint,
                    observed_support_graph_digest,
                    exclusive_lease_receipt_id,
                    exclusive_lease_release_receipt_id,
                    working_infobase_lease_released: TrueLiteral,
                    working_infobase_lease_release_verified: TrueLiteral,
                };
                let stop_evidence_digest = terminalization_digest(
                    &record,
                    "working-infobase dirty-stop evidence digest failed",
                )?;
                Ok(Self::LeaseAcquiredDirty(
                    ManualWorkingInfobaseDirtyStopEvidence {
                        cause: record.cause,
                        working_infobase_identity: record.working_infobase_identity,
                        closure_plan_digest: record.closure_plan_digest,
                        exclusive_lease_capability_id: record.exclusive_lease_capability_id,
                        expected_repository_fingerprint: record.expected_repository_fingerprint,
                        observed_working_infobase_fingerprint: record
                            .observed_working_infobase_fingerprint,
                        observed_support_graph_digest: record.observed_support_graph_digest,
                        exclusive_lease_receipt_id: record.exclusive_lease_receipt_id,
                        exclusive_lease_release_receipt_id: record
                            .exclusive_lease_release_receipt_id,
                        working_infobase_lease_released: record.working_infobase_lease_released,
                        working_infobase_lease_release_verified: record
                            .working_infobase_lease_release_verified,
                        stop_evidence_digest,
                    },
                ))
            }
        }
    }

    pub(crate) fn working_infobase_identity(&self) -> &ManualWorkingInfobaseIdentity {
        match self {
            Self::LeaseBusy(value) => &value.working_infobase_identity,
            Self::LeaseAcquiredDirty(value) => &value.working_infobase_identity,
        }
    }

    pub(crate) fn closure_plan_digest(&self) -> &Sha256Digest {
        match self {
            Self::LeaseBusy(value) => &value.closure_plan_digest,
            Self::LeaseAcquiredDirty(value) => &value.closure_plan_digest,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SupportRecoveryGuardPlanAuthority {
    authorization: FrozenSupportRecoveryAuthorizationProjection,
    disposition: SupportRecoveryDisposition,
    finalization_plan_digest: Sha256Digest,
    materialized_selective_update_plan_digest: Sha256Digest,
    planned_lock_targets: SupportRecoveryLockTargets,
    history_from_cursor: RepositoryHistoryCursor,
}

impl SupportRecoveryGuardPlanAuthority {
    pub(crate) fn from_approved(
        _token: &SupportRecoveryAuthorityToken,
        authorization: FrozenSupportRecoveryAuthorizationProjection,
        finalization_plan: &SupportRecoveryFinalizationPlan,
    ) -> Result<Self, SupportTerminalizationContractError> {
        Self::from_materialized_finalization_plan(authorization, finalization_plan)
    }

    /// Task 9 owns only the closed guard wire/digest contract. Production
    /// materialization stays unavailable until Task 11 can bind the exact
    /// frozen action, approved history partition, materialized plan, and
    /// capability-backed under-guard recheck as one opaque authority.
    #[cfg(test)]
    pub(crate) fn from_materialized_finalization_plan_test_only(
        authorization: FrozenSupportRecoveryAuthorizationProjection,
        finalization_plan: &SupportRecoveryFinalizationPlan,
    ) -> Result<Self, SupportTerminalizationContractError> {
        Self::from_materialized_finalization_plan(authorization, finalization_plan)
    }

    fn from_materialized_finalization_plan(
        authorization: FrozenSupportRecoveryAuthorizationProjection,
        finalization_plan: &SupportRecoveryFinalizationPlan,
    ) -> Result<Self, SupportTerminalizationContractError> {
        let materialized_plan = finalization_plan
            .materialized_selective_update_plan
            .as_ref()
            .ok_or(SupportTerminalizationContractError(
                "support recovery guard requires a materialized selective update plan",
            ))?;
        Ok(Self {
            authorization,
            disposition: finalization_plan.disposition,
            finalization_plan_digest: finalization_plan.plan_digest.clone(),
            materialized_selective_update_plan_digest: materialized_plan.plan_digest().clone(),
            planned_lock_targets: finalization_plan.lock_targets.clone(),
            history_from_cursor: finalization_plan.history_from_cursor.clone(),
        })
    }

    #[cfg(test)]
    pub(crate) fn test_only(
        authorization: FrozenSupportRecoveryAuthorizationProjection,
        finalization_plan_digest: Sha256Digest,
        planned_lock_targets: SupportRecoveryLockTargets,
        history_from_cursor: RepositoryHistoryCursor,
    ) -> Self {
        Self {
            authorization,
            disposition: SupportRecoveryDisposition::RestoreThenReauthorize,
            materialized_selective_update_plan_digest: finalization_plan_digest.clone(),
            finalization_plan_digest,
            planned_lock_targets,
            history_from_cursor,
        }
    }

    fn manual_target_mode(&self) -> ManualSupportTargetMode {
        self.authorization.manual_target_mode()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct BlockedBeforeRootGuardAuthority {
    failed_target: RepositoryTargetIdentity,
    failed_target_display: RepositoryTargetDisplay,
    locked_by: RequiredNullable<RepositoryOwnerIdentity>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct BlockedAfterPartialGuardAuthority {
    acquired_in_order: SupportRecoveryAcquiredLockTargets,
    failed_target: RepositoryTargetIdentity,
    failed_target_display: RepositoryTargetDisplay,
    locked_by: RequiredNullable<RepositoryOwnerIdentity>,
    released_in_reverse_order: SupportRecoveryReleasedLockTargets,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StoppedAfterCompleteGuardAuthority {
    manual_actor_lock_inventory_proof: Option<ManualActorLockInventoryProof>,
    reserved_original_lease_stop_evidence: Option<ReservedOriginalLeaseStopEvidence>,
    manual_working_infobase_stop_evidence: Option<ManualWorkingInfobaseStopEvidence>,
    guard_release_receipt_id: UnicaId,
    history_from_cursor: RepositoryHistoryCursor,
    history_through_cursor: RepositoryHistoryCursor,
    history_partition_digest: Sha256Digest,
    released_in_reverse_order: SupportRecoveryReleasedLockTargets,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CompletedGuardAuthority {
    manual_actor_lock_inventory_proof: Option<ManualActorLockInventoryProof>,
    reserved_original_terminalization_proof: Option<ReservedOriginalTerminalizationProof>,
    manual_working_infobase_closure_proof: Option<ManualWorkingInfobaseClosureProof>,
    guard_release_receipt_id: UnicaId,
    history_from_cursor: RepositoryHistoryCursor,
    history_through_cursor: RepositoryHistoryCursor,
    history_partition_digest: Sha256Digest,
    selective_update_proof: SelectiveRepositoryUpdateProof,
    post_release_observed_history_cursor: RepositoryHistoryCursor,
    post_release_history_partition: ValidatedRepositoryHistoryPartition,
    deferred_repository_advance: Option<DeferredRepositoryAdvance>,
    authorization_terminalization_receipt: SupportRecoveryAuthorizationTerminalizationReceipt,
    authorization_outcome: CompletedSupportRecoveryAuthorizationOutcome,
    released_in_reverse_order: SupportRecoveryReleasedLockTargets,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SupportRecoveryGuardAuthority {
    plan: SupportRecoveryGuardPlanAuthority,
    guard_receipt_id: UnicaId,
    blocked_before_root: Option<BlockedBeforeRootGuardAuthority>,
    blocked_after_partial: Option<BlockedAfterPartialGuardAuthority>,
    stopped_after_complete_guard: Option<StoppedAfterCompleteGuardAuthority>,
    completed: Option<CompletedGuardAuthority>,
}

fn reserved_inventory_matches_authorization(
    authorization: &FrozenSupportRecoveryAuthorizationProjection,
    inventory: &ManualActorLockInventoryProof,
) -> bool {
    let Some(expected_baseline) = authorization.manual_actor_lock_baseline_digest() else {
        return false;
    };
    inventory.username() == authorization.manual_actor_username()
        && inventory.baseline_lock_set_digest() == expected_baseline
        && inventory.observed_lock_set_digest() == expected_baseline
}

fn stopped_mode_evidence_matches_authorization(
    plan: &SupportRecoveryGuardPlanAuthority,
    inventory: Option<&ManualActorLockInventoryProof>,
    lease_stop: Option<&ReservedOriginalLeaseStopEvidence>,
    working_infobase_stop: Option<&ManualWorkingInfobaseStopEvidence>,
) -> bool {
    match plan.manual_target_mode() {
        ManualSupportTargetMode::ReservedOriginal => {
            let (Some(inventory), Some(lease_stop), Some(expected_capability)) = (
                inventory,
                lease_stop,
                plan.authorization.reserved_original_lease_capability_id(),
            ) else {
                return false;
            };
            reserved_inventory_matches_authorization(&plan.authorization, inventory)
                && lease_stop.reserved_original_identity_digest()
                    == plan.authorization.reserved_original_identity_digest()
                && lease_stop.exclusive_lease_capability_id() == expected_capability
                && working_infobase_stop.is_none()
        }
        ManualSupportTargetMode::SeparateWorkingInfobase => {
            inventory.is_none() && lease_stop.is_none() && working_infobase_stop.is_some()
        }
    }
}

fn completed_mode_evidence_matches_authorization(
    plan: &SupportRecoveryGuardPlanAuthority,
    inventory: Option<&ManualActorLockInventoryProof>,
    terminalization: Option<&ReservedOriginalTerminalizationProof>,
    working_infobase_closure: Option<&ManualWorkingInfobaseClosureProof>,
) -> bool {
    match plan.manual_target_mode() {
        ManualSupportTargetMode::ReservedOriginal => {
            let (Some(inventory), Some(terminalization), Some(expected_capability)) = (
                inventory,
                terminalization,
                plan.authorization.reserved_original_lease_capability_id(),
            ) else {
                return false;
            };
            reserved_inventory_matches_authorization(&plan.authorization, inventory)
                && terminalization.reserved_original_identity_digest()
                    == plan.authorization.reserved_original_identity_digest()
                && terminalization.exclusive_lease_capability_id() == expected_capability
                && terminalization.expected_repository_fingerprint()
                    == plan.authorization.expected_original_fingerprint()
                && working_infobase_closure.is_none()
        }
        ManualSupportTargetMode::SeparateWorkingInfobase => {
            inventory.is_none() && terminalization.is_none() && working_infobase_closure.is_some()
        }
    }
}

impl SupportRecoveryGuardAuthority {
    pub(crate) fn blocked_before_root_from_approved(
        _token: &SupportRecoveryAuthorityToken,
        plan: SupportRecoveryGuardPlanAuthority,
        guard_receipt_id: UnicaId,
        failed_target: RepositoryTargetIdentity,
        failed_target_display: RepositoryTargetDisplay,
        locked_by: RequiredNullable<RepositoryOwnerIdentity>,
    ) -> Result<Self, SupportTerminalizationContractError> {
        Self::blocked_before_root(
            plan,
            guard_receipt_id,
            failed_target,
            failed_target_display,
            locked_by,
        )
    }

    #[cfg(test)]
    pub(crate) fn blocked_before_root_test_only(
        plan: SupportRecoveryGuardPlanAuthority,
        guard_receipt_id: UnicaId,
        failed_target: RepositoryTargetIdentity,
        failed_target_display: RepositoryTargetDisplay,
        locked_by: RequiredNullable<RepositoryOwnerIdentity>,
    ) -> Result<Self, SupportTerminalizationContractError> {
        Self::blocked_before_root(
            plan,
            guard_receipt_id,
            failed_target,
            failed_target_display,
            locked_by,
        )
    }

    fn blocked_before_root(
        plan: SupportRecoveryGuardPlanAuthority,
        guard_receipt_id: UnicaId,
        failed_target: RepositoryTargetIdentity,
        failed_target_display: RepositoryTargetDisplay,
        locked_by: RequiredNullable<RepositoryOwnerIdentity>,
    ) -> Result<Self, SupportTerminalizationContractError> {
        if !plan.planned_lock_targets.as_slice()[0]
            .matches_identity_and_display(&failed_target, &failed_target_display)
        {
            return Err(SupportTerminalizationContractError(
                "blocked-before-root target differs from the first planned root target",
            ));
        }
        Ok(Self {
            plan,
            guard_receipt_id,
            blocked_before_root: Some(BlockedBeforeRootGuardAuthority {
                failed_target,
                failed_target_display,
                locked_by,
            }),
            blocked_after_partial: None,
            stopped_after_complete_guard: None,
            completed: None,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn blocked_after_partial_from_approved(
        _token: &SupportRecoveryAuthorityToken,
        plan: SupportRecoveryGuardPlanAuthority,
        guard_receipt_id: UnicaId,
        acquired_in_order: SupportRecoveryAcquiredLockTargets,
        failed_target: RepositoryTargetIdentity,
        failed_target_display: RepositoryTargetDisplay,
        locked_by: RequiredNullable<RepositoryOwnerIdentity>,
        released_in_reverse_order: SupportRecoveryReleasedLockTargets,
    ) -> Result<Self, SupportTerminalizationContractError> {
        Self::blocked_after_partial(
            plan,
            guard_receipt_id,
            acquired_in_order,
            failed_target,
            failed_target_display,
            locked_by,
            released_in_reverse_order,
        )
    }

    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn blocked_after_partial_test_only(
        plan: SupportRecoveryGuardPlanAuthority,
        guard_receipt_id: UnicaId,
        acquired_in_order: SupportRecoveryAcquiredLockTargets,
        failed_target: RepositoryTargetIdentity,
        failed_target_display: RepositoryTargetDisplay,
        locked_by: RequiredNullable<RepositoryOwnerIdentity>,
        released_in_reverse_order: SupportRecoveryReleasedLockTargets,
    ) -> Result<Self, SupportTerminalizationContractError> {
        Self::blocked_after_partial(
            plan,
            guard_receipt_id,
            acquired_in_order,
            failed_target,
            failed_target_display,
            locked_by,
            released_in_reverse_order,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn blocked_after_partial(
        plan: SupportRecoveryGuardPlanAuthority,
        guard_receipt_id: UnicaId,
        acquired_in_order: SupportRecoveryAcquiredLockTargets,
        failed_target: RepositoryTargetIdentity,
        failed_target_display: RepositoryTargetDisplay,
        locked_by: RequiredNullable<RepositoryOwnerIdentity>,
        released_in_reverse_order: SupportRecoveryReleasedLockTargets,
    ) -> Result<Self, SupportTerminalizationContractError> {
        let acquired = acquired_in_order.as_slice();
        if acquired.len() >= plan.planned_lock_targets.as_slice().len()
            || acquired != &plan.planned_lock_targets.as_slice()[..acquired.len()]
            || !plan.planned_lock_targets.as_slice()[acquired.len()]
                .matches_identity_and_display(&failed_target, &failed_target_display)
            || !released_in_reverse_order
                .as_slice()
                .iter()
                .eq(acquired.iter().rev())
        {
            return Err(SupportTerminalizationContractError(
                "partial guard does not bind the exact acquired prefix and reverse compensation",
            ));
        }
        Ok(Self {
            plan,
            guard_receipt_id,
            blocked_before_root: None,
            blocked_after_partial: Some(BlockedAfterPartialGuardAuthority {
                acquired_in_order,
                failed_target,
                failed_target_display,
                locked_by,
                released_in_reverse_order,
            }),
            stopped_after_complete_guard: None,
            completed: None,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn stopped_after_complete_guard_from_approved(
        _token: &SupportRecoveryAuthorityToken,
        plan: SupportRecoveryGuardPlanAuthority,
        guard_receipt_id: UnicaId,
        guard_release_receipt_id: UnicaId,
        manual_actor_lock_inventory_proof: Option<ManualActorLockInventoryProof>,
        reserved_original_lease_stop_evidence: Option<ReservedOriginalLeaseStopEvidence>,
        manual_working_infobase_stop_evidence: Option<ManualWorkingInfobaseStopEvidence>,
        history_from_cursor: RepositoryHistoryCursor,
        history_through_cursor: RepositoryHistoryCursor,
        history_partition_digest: Sha256Digest,
    ) -> Result<Self, SupportTerminalizationContractError> {
        Self::stopped_after_complete_guard(
            plan,
            guard_receipt_id,
            guard_release_receipt_id,
            manual_actor_lock_inventory_proof,
            reserved_original_lease_stop_evidence,
            manual_working_infobase_stop_evidence,
            history_from_cursor,
            history_through_cursor,
            history_partition_digest,
        )
    }

    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn stopped_after_complete_guard_test_only(
        plan: SupportRecoveryGuardPlanAuthority,
        guard_receipt_id: UnicaId,
        guard_release_receipt_id: UnicaId,
        manual_actor_lock_inventory_proof: Option<ManualActorLockInventoryProof>,
        reserved_original_lease_stop_evidence: Option<ReservedOriginalLeaseStopEvidence>,
        manual_working_infobase_stop_evidence: Option<ManualWorkingInfobaseStopEvidence>,
        history_from_cursor: RepositoryHistoryCursor,
        history_through_cursor: RepositoryHistoryCursor,
        history_partition_digest: Sha256Digest,
    ) -> Result<Self, SupportTerminalizationContractError> {
        Self::stopped_after_complete_guard(
            plan,
            guard_receipt_id,
            guard_release_receipt_id,
            manual_actor_lock_inventory_proof,
            reserved_original_lease_stop_evidence,
            manual_working_infobase_stop_evidence,
            history_from_cursor,
            history_through_cursor,
            history_partition_digest,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn stopped_after_complete_guard(
        plan: SupportRecoveryGuardPlanAuthority,
        guard_receipt_id: UnicaId,
        guard_release_receipt_id: UnicaId,
        manual_actor_lock_inventory_proof: Option<ManualActorLockInventoryProof>,
        reserved_original_lease_stop_evidence: Option<ReservedOriginalLeaseStopEvidence>,
        manual_working_infobase_stop_evidence: Option<ManualWorkingInfobaseStopEvidence>,
        history_from_cursor: RepositoryHistoryCursor,
        history_through_cursor: RepositoryHistoryCursor,
        history_partition_digest: Sha256Digest,
    ) -> Result<Self, SupportTerminalizationContractError> {
        let mode_presence_is_valid = stopped_mode_evidence_matches_authorization(
            &plan,
            manual_actor_lock_inventory_proof.as_ref(),
            reserved_original_lease_stop_evidence.as_ref(),
            manual_working_infobase_stop_evidence.as_ref(),
        );
        if !mode_presence_is_valid
            || guard_receipt_id == guard_release_receipt_id
            || history_from_cursor != plan.history_from_cursor
        {
            return Err(SupportTerminalizationContractError(
                "complete stopped guard violates mode presence or history anchor",
            ));
        }
        let released_in_reverse_order = SupportRecoveryReleasedLockTargets::new(
            plan.planned_lock_targets
                .as_slice()
                .iter()
                .rev()
                .cloned()
                .collect(),
        )?;
        Ok(Self {
            plan,
            guard_receipt_id,
            blocked_before_root: None,
            blocked_after_partial: None,
            stopped_after_complete_guard: Some(StoppedAfterCompleteGuardAuthority {
                manual_actor_lock_inventory_proof,
                reserved_original_lease_stop_evidence,
                manual_working_infobase_stop_evidence,
                guard_release_receipt_id,
                history_from_cursor,
                history_through_cursor,
                history_partition_digest,
                released_in_reverse_order,
            }),
            completed: None,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn completed_from_approved(
        _token: &SupportRecoveryAuthorityToken,
        plan: SupportRecoveryGuardPlanAuthority,
        guard_receipt_id: UnicaId,
        guard_release_receipt_id: UnicaId,
        manual_actor_lock_inventory_proof: Option<ManualActorLockInventoryProof>,
        reserved_original_terminalization_proof: Option<ReservedOriginalTerminalizationProof>,
        manual_working_infobase_closure_proof: Option<ManualWorkingInfobaseClosureProof>,
        current_history_partition: &ValidatedRepositoryHistoryPartition,
        selective_update_proof: SelectiveRepositoryUpdateProof,
        post_release_history_partition: ValidatedRepositoryHistoryPartition,
        deferred_repository_advance: Option<DeferredRepositoryAdvance>,
        authorization_terminalization_receipt: SupportRecoveryAuthorizationTerminalizationReceipt,
    ) -> Result<Self, SupportTerminalizationContractError> {
        Self::completed(
            plan,
            guard_receipt_id,
            guard_release_receipt_id,
            manual_actor_lock_inventory_proof,
            reserved_original_terminalization_proof,
            manual_working_infobase_closure_proof,
            current_history_partition,
            selective_update_proof,
            post_release_history_partition,
            deferred_repository_advance,
            authorization_terminalization_receipt,
        )
    }

    /// Schema/constructor fixture only. The production completion mint is
    /// deliberately deferred to the Task 11 recovery-status authority so a
    /// caller cannot assert recheck literals or post-release scan boundaries.
    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn completed_test_only(
        plan: SupportRecoveryGuardPlanAuthority,
        guard_receipt_id: UnicaId,
        guard_release_receipt_id: UnicaId,
        manual_actor_lock_inventory_proof: Option<ManualActorLockInventoryProof>,
        reserved_original_terminalization_proof: Option<ReservedOriginalTerminalizationProof>,
        manual_working_infobase_closure_proof: Option<ManualWorkingInfobaseClosureProof>,
        current_history_partition: &ValidatedRepositoryHistoryPartition,
        selective_update_proof: SelectiveRepositoryUpdateProof,
        post_release_history_partition: ValidatedRepositoryHistoryPartition,
        deferred_repository_advance: Option<DeferredRepositoryAdvance>,
    ) -> Result<Self, SupportTerminalizationContractError> {
        let authorization_terminalization_receipt =
            SupportRecoveryAuthorizationTerminalizationReceipt::from_fields(
                plan.authorization.support_action_id().clone(),
                plan.authorization.support_action_digest().clone(),
                UnicaId::parse("cccccccc-cccc-4ccc-8ccc-cccccccccccc")
                    .expect("test terminalization receipt id is valid"),
                completed_authorization_outcome(plan.disposition),
            )?;
        Self::completed(
            plan,
            guard_receipt_id,
            guard_release_receipt_id,
            manual_actor_lock_inventory_proof,
            reserved_original_terminalization_proof,
            manual_working_infobase_closure_proof,
            current_history_partition,
            selective_update_proof,
            post_release_history_partition,
            deferred_repository_advance,
            authorization_terminalization_receipt,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn completed(
        plan: SupportRecoveryGuardPlanAuthority,
        guard_receipt_id: UnicaId,
        guard_release_receipt_id: UnicaId,
        manual_actor_lock_inventory_proof: Option<ManualActorLockInventoryProof>,
        reserved_original_terminalization_proof: Option<ReservedOriginalTerminalizationProof>,
        manual_working_infobase_closure_proof: Option<ManualWorkingInfobaseClosureProof>,
        current_history_partition: &ValidatedRepositoryHistoryPartition,
        selective_update_proof: SelectiveRepositoryUpdateProof,
        post_release_history_partition: ValidatedRepositoryHistoryPartition,
        deferred_repository_advance: Option<DeferredRepositoryAdvance>,
        authorization_terminalization_receipt: SupportRecoveryAuthorizationTerminalizationReceipt,
    ) -> Result<Self, SupportTerminalizationContractError> {
        let mode_presence_is_valid = completed_mode_evidence_matches_authorization(
            &plan,
            manual_actor_lock_inventory_proof.as_ref(),
            reserved_original_terminalization_proof.as_ref(),
            manual_working_infobase_closure_proof.as_ref(),
        );
        let history_from_cursor = current_history_partition.start_cursor().clone();
        let history_through_cursor = current_history_partition.through_inclusive().clone();
        let post_release_observed_history_cursor =
            post_release_history_partition.through_inclusive().clone();
        let deferred_anchor_is_valid = deferred_repository_advance
            .as_ref()
            .is_none_or(|advance| advance.anchor_cursor() == &post_release_observed_history_cursor);
        let authorization_outcome = completed_authorization_outcome(plan.disposition);
        if !mode_presence_is_valid
            || guard_receipt_id == guard_release_receipt_id
            || history_from_cursor != plan.history_from_cursor
            || selective_update_proof.plan_digest()
                != &plan.materialized_selective_update_plan_digest
            || selective_update_proof.guard_receipt_id() != &guard_receipt_id
            || selective_update_proof.observed_before_cursor() != &history_through_cursor
            || post_release_history_partition.start_cursor() != &history_through_cursor
            || !post_release_history_partition
                .contains_cursor(selective_update_proof.observed_after_cursor())
            || !deferred_anchor_is_valid
            || authorization_terminalization_receipt.support_action_id()
                != plan.authorization.support_action_id()
            || authorization_terminalization_receipt.support_action_digest()
                != plan.authorization.support_action_digest()
            || authorization_terminalization_receipt.authorization_outcome()
                != authorization_outcome
        {
            return Err(SupportTerminalizationContractError(
                "completed guard evidence does not bind the materialized plan and history windows",
            ));
        }
        let released_in_reverse_order = SupportRecoveryReleasedLockTargets::new(
            plan.planned_lock_targets
                .as_slice()
                .iter()
                .rev()
                .cloned()
                .collect(),
        )?;
        Ok(Self {
            plan,
            guard_receipt_id,
            blocked_before_root: None,
            blocked_after_partial: None,
            stopped_after_complete_guard: None,
            completed: Some(CompletedGuardAuthority {
                manual_actor_lock_inventory_proof,
                reserved_original_terminalization_proof,
                manual_working_infobase_closure_proof,
                guard_release_receipt_id,
                history_from_cursor,
                history_through_cursor,
                history_partition_digest: current_history_partition.partition_digest().clone(),
                selective_update_proof,
                post_release_observed_history_cursor,
                post_release_history_partition,
                deferred_repository_advance,
                authorization_terminalization_receipt,
                authorization_outcome,
                released_in_reverse_order,
            }),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BlockedBeforeRootGuardProofDigestRecord {
    outcome: BlockedBeforeRootOutcome,
    guard_receipt_id: UnicaId,
    manual_target_mode: ManualSupportTargetMode,
    finalization_plan_digest: Sha256Digest,
    planned_lock_targets: SupportRecoveryLockTargets,
    acquired_in_order: EmptySupportRecoveryLockTargets,
    failed_target: RepositoryTargetIdentity,
    failed_target_display: RepositoryTargetDisplay,
    #[serde(deserialize_with = "RequiredNullable::deserialize_required")]
    locked_by: RequiredNullable<RepositoryOwnerIdentity>,
    authorization_outcome: UnchangedAuthorizationOutcome,
    released_in_reverse_order: EmptySupportRecoveryLockTargets,
    release_verified: TrueLiteral,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BlockedAfterPartialGuardProofDigestRecord {
    outcome: BlockedAfterPartialOutcome,
    guard_receipt_id: UnicaId,
    manual_target_mode: ManualSupportTargetMode,
    finalization_plan_digest: Sha256Digest,
    planned_lock_targets: SupportRecoveryLockTargets,
    acquired_in_order: SupportRecoveryAcquiredLockTargets,
    failed_target: RepositoryTargetIdentity,
    failed_target_display: RepositoryTargetDisplay,
    #[serde(deserialize_with = "RequiredNullable::deserialize_required")]
    locked_by: RequiredNullable<RepositoryOwnerIdentity>,
    authorization_outcome: UnchangedAuthorizationOutcome,
    released_in_reverse_order: SupportRecoveryReleasedLockTargets,
    release_verified: TrueLiteral,
}

macro_rules! define_blocked_before_root_guard_schema {
    ($name:ident, $mode:ty) => {
        #[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        pub(crate) struct $name {
            outcome: BlockedBeforeRootOutcome,
            guard_receipt_id: UnicaId,
            manual_target_mode: $mode,
            finalization_plan_digest: Sha256Digest,
            planned_lock_targets: SupportRecoveryLockTargets,
            acquired_in_order: EmptySupportRecoveryLockTargets,
            failed_target: RepositoryTargetIdentity,
            failed_target_display: RepositoryTargetDisplay,
            #[serde(deserialize_with = "RequiredNullable::deserialize_required")]
            locked_by: RequiredNullable<RepositoryOwnerIdentity>,
            authorization_outcome: UnchangedAuthorizationOutcome,
            released_in_reverse_order: EmptySupportRecoveryLockTargets,
            release_verified: TrueLiteral,
            proof_digest: Sha256Digest,
        }
    };
}

macro_rules! define_blocked_after_partial_guard_schema {
    ($name:ident, $mode:ty) => {
        #[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        pub(crate) struct $name {
            outcome: BlockedAfterPartialOutcome,
            guard_receipt_id: UnicaId,
            manual_target_mode: $mode,
            finalization_plan_digest: Sha256Digest,
            planned_lock_targets: SupportRecoveryLockTargets,
            acquired_in_order: SupportRecoveryAcquiredLockTargets,
            failed_target: RepositoryTargetIdentity,
            failed_target_display: RepositoryTargetDisplay,
            #[serde(deserialize_with = "RequiredNullable::deserialize_required")]
            locked_by: RequiredNullable<RepositoryOwnerIdentity>,
            authorization_outcome: UnchangedAuthorizationOutcome,
            released_in_reverse_order: SupportRecoveryReleasedLockTargets,
            release_verified: TrueLiteral,
            proof_digest: Sha256Digest,
        }
    };
}

define_blocked_before_root_guard_schema!(
    ReservedBlockedBeforeRootGuardProofSchema,
    ReservedOriginalModeLiteral
);
define_blocked_after_partial_guard_schema!(
    ReservedBlockedAfterPartialGuardProofSchema,
    ReservedOriginalModeLiteral
);
define_blocked_before_root_guard_schema!(
    SeparateBlockedBeforeRootGuardProofSchema,
    SeparateWorkingInfobaseModeLiteral
);
define_blocked_after_partial_guard_schema!(
    SeparateBlockedAfterPartialGuardProofSchema,
    SeparateWorkingInfobaseModeLiteral
);

macro_rules! define_stopped_guard_schema {
    ($name:ident, $mode:ty, { $($mode_fields:tt)* }, { $($suffix:tt)* }) => {
        #[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        pub(crate) struct $name {
            outcome: StoppedAfterCompleteGuardOutcome,
            guard_receipt_id: UnicaId,
            guard_release_receipt_id: UnicaId,
            manual_target_mode: $mode,
            $($mode_fields)*
            finalization_plan_digest: Sha256Digest,
            planned_lock_targets: SupportRecoveryLockTargets,
            acquired_in_order: SupportRecoveryLockTargets,
            history_from_cursor: RepositoryHistoryCursor,
            history_through_cursor: RepositoryHistoryCursor,
            history_partition_digest: Sha256Digest,
            support_graph_rechecked_under_guard: TrueLiteral,
            corrective_before_state_binding_verified: TrueLiteral,
            content_rechecked_under_guard: TrueLiteral,
            original_rechecked_under_guard: TrueLiteral,
            selective_update_performed: FalseLiteral,
            authorization_outcome: UnchangedAuthorizationOutcome,
            released_in_reverse_order: SupportRecoveryReleasedLockTargets,
            release_verified: TrueLiteral,
            $($suffix)*
        }
    };
}

define_stopped_guard_schema!(
    SeparateStoppedAfterCompleteGuardProofDigestSchema,
    SeparateWorkingInfobaseModeLiteral,
    {
        manual_working_infobase_stop_evidence: ManualWorkingInfobaseStopEvidence,
    },
    {}
);
define_stopped_guard_schema!(
    ReservedStoppedAfterCompleteGuardProofDigestSchema,
    ReservedOriginalModeLiteral,
    {
        manual_actor_lock_inventory_proof: ManualActorLockInventoryProof,
        reserved_original_lease_stop_evidence: ReservedOriginalLeaseStopEvidence,
    },
    {}
);
define_stopped_guard_schema!(
    SeparateStoppedAfterCompleteGuardProofSchema,
    SeparateWorkingInfobaseModeLiteral,
    {
        manual_working_infobase_stop_evidence: ManualWorkingInfobaseStopEvidence,
    },
    { proof_digest: Sha256Digest, }
);
define_stopped_guard_schema!(
    ReservedStoppedAfterCompleteGuardProofSchema,
    ReservedOriginalModeLiteral,
    {
        manual_actor_lock_inventory_proof: ManualActorLockInventoryProof,
        reserved_original_lease_stop_evidence: ReservedOriginalLeaseStopEvidence,
    },
    { proof_digest: Sha256Digest, }
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StoppedAfterCompleteGuardProofDigestRecord {
    outcome: StoppedAfterCompleteGuardOutcome,
    guard_receipt_id: UnicaId,
    guard_release_receipt_id: UnicaId,
    manual_target_mode: ManualSupportTargetMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    manual_actor_lock_inventory_proof: Option<ManualActorLockInventoryProof>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reserved_original_lease_stop_evidence: Option<ReservedOriginalLeaseStopEvidence>,
    #[serde(skip_serializing_if = "Option::is_none")]
    manual_working_infobase_stop_evidence: Option<ManualWorkingInfobaseStopEvidence>,
    finalization_plan_digest: Sha256Digest,
    planned_lock_targets: SupportRecoveryLockTargets,
    acquired_in_order: SupportRecoveryLockTargets,
    history_from_cursor: RepositoryHistoryCursor,
    history_through_cursor: RepositoryHistoryCursor,
    history_partition_digest: Sha256Digest,
    support_graph_rechecked_under_guard: TrueLiteral,
    corrective_before_state_binding_verified: TrueLiteral,
    content_rechecked_under_guard: TrueLiteral,
    original_rechecked_under_guard: TrueLiteral,
    selective_update_performed: FalseLiteral,
    authorization_outcome: UnchangedAuthorizationOutcome,
    released_in_reverse_order: SupportRecoveryReleasedLockTargets,
    release_verified: TrueLiteral,
}

impl JsonSchema for StoppedAfterCompleteGuardProofDigestRecord {
    fn schema_name() -> Cow<'static, str> {
        "StoppedAfterCompleteGuardProofDigestRecord".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<SeparateStoppedAfterCompleteGuardProofDigestSchema>(),
            generator.subschema_for::<ReservedStoppedAfterCompleteGuardProofDigestSchema>(),
        ])
    }
}

macro_rules! define_completed_guard_schema {
    ($name:ident, $mode:ty, { $($mode_fields:tt)* }, { $($suffix:tt)* }) => {
        #[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct $name {
            outcome: CompletedGuardOutcome,
            guard_receipt_id: UnicaId,
            guard_release_receipt_id: UnicaId,
            manual_target_mode: $mode,
            $($mode_fields)*
            finalization_plan_digest: Sha256Digest,
            planned_lock_targets: SupportRecoveryLockTargets,
            acquired_in_order: SupportRecoveryLockTargets,
            history_from_cursor: RepositoryHistoryCursor,
            history_through_cursor: RepositoryHistoryCursor,
            history_partition_digest: Sha256Digest,
            support_graph_rechecked_under_guard: TrueLiteral,
            corrective_before_state_binding_verified: TrueLiteral,
            content_rechecked_under_guard: TrueLiteral,
            original_rechecked_under_guard: TrueLiteral,
            selective_update_proof: SelectiveRepositoryUpdateProof,
            post_release_observed_history_cursor: RepositoryHistoryCursor,
            post_release_history_partition: ValidatedRepositoryHistoryPartition,
            #[serde(skip_serializing_if = "Option::is_none")]
            deferred_repository_advance: Option<DeferredRepositoryAdvance>,
            authorization_terminalization_receipt: SupportRecoveryAuthorizationTerminalizationReceipt,
            authorization_outcome: CompletedSupportRecoveryAuthorizationOutcome,
            released_in_reverse_order: SupportRecoveryReleasedLockTargets,
            release_verified: TrueLiteral,
            $($suffix)*
        }
    };
}

define_completed_guard_schema!(
    SeparateCompletedGuardProofDigestSchema,
    SeparateWorkingInfobaseModeLiteral,
    {
        manual_working_infobase_closure_proof: ManualWorkingInfobaseClosureProof,
    },
    {}
);
define_completed_guard_schema!(
    ReservedCompletedGuardProofDigestSchema,
    ReservedOriginalModeLiteral,
    {
        manual_actor_lock_inventory_proof: ManualActorLockInventoryProof,
        reserved_original_terminalization_proof: ReservedOriginalTerminalizationProof,
    },
    {}
);
define_completed_guard_schema!(
    SeparateCompletedGuardProofSchema,
    SeparateWorkingInfobaseModeLiteral,
    {
        manual_working_infobase_closure_proof: ManualWorkingInfobaseClosureProof,
    },
    { proof_digest: Sha256Digest, }
);
define_completed_guard_schema!(
    ReservedCompletedGuardProofSchema,
    ReservedOriginalModeLiteral,
    {
        manual_actor_lock_inventory_proof: ManualActorLockInventoryProof,
        reserved_original_terminalization_proof: ReservedOriginalTerminalizationProof,
    },
    { proof_digest: Sha256Digest, }
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CompletedGuardProofDigestRecord {
    outcome: CompletedGuardOutcome,
    guard_receipt_id: UnicaId,
    guard_release_receipt_id: UnicaId,
    manual_target_mode: ManualSupportTargetMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    manual_actor_lock_inventory_proof: Option<ManualActorLockInventoryProof>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reserved_original_terminalization_proof: Option<ReservedOriginalTerminalizationProof>,
    #[serde(skip_serializing_if = "Option::is_none")]
    manual_working_infobase_closure_proof: Option<ManualWorkingInfobaseClosureProof>,
    finalization_plan_digest: Sha256Digest,
    planned_lock_targets: SupportRecoveryLockTargets,
    acquired_in_order: SupportRecoveryLockTargets,
    history_from_cursor: RepositoryHistoryCursor,
    history_through_cursor: RepositoryHistoryCursor,
    history_partition_digest: Sha256Digest,
    support_graph_rechecked_under_guard: TrueLiteral,
    corrective_before_state_binding_verified: TrueLiteral,
    content_rechecked_under_guard: TrueLiteral,
    original_rechecked_under_guard: TrueLiteral,
    selective_update_proof: SelectiveRepositoryUpdateProof,
    post_release_observed_history_cursor: RepositoryHistoryCursor,
    post_release_history_partition: ValidatedRepositoryHistoryPartition,
    #[serde(skip_serializing_if = "Option::is_none")]
    deferred_repository_advance: Option<DeferredRepositoryAdvance>,
    authorization_terminalization_receipt: SupportRecoveryAuthorizationTerminalizationReceipt,
    authorization_outcome: CompletedSupportRecoveryAuthorizationOutcome,
    released_in_reverse_order: SupportRecoveryReleasedLockTargets,
    release_verified: TrueLiteral,
}

impl JsonSchema for CompletedGuardProofDigestRecord {
    fn schema_name() -> Cow<'static, str> {
        "CompletedGuardProofDigestRecord".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<SeparateCompletedGuardProofDigestSchema>(),
            generator.subschema_for::<ReservedCompletedGuardProofDigestSchema>(),
        ])
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SupportRecoveryGuardProofDigestRecord {
    blocked_before_root: Option<BlockedBeforeRootGuardProofDigestRecord>,
    blocked_after_partial: Option<BlockedAfterPartialGuardProofDigestRecord>,
    stopped_after_complete_guard: Option<StoppedAfterCompleteGuardProofDigestRecord>,
    completed: Option<CompletedGuardProofDigestRecord>,
}

impl SupportRecoveryGuardProofDigestRecord {
    fn blocked_before_root(record: BlockedBeforeRootGuardProofDigestRecord) -> Self {
        Self {
            blocked_before_root: Some(record),
            blocked_after_partial: None,
            stopped_after_complete_guard: None,
            completed: None,
        }
    }

    fn blocked_after_partial(record: BlockedAfterPartialGuardProofDigestRecord) -> Self {
        Self {
            blocked_before_root: None,
            blocked_after_partial: Some(record),
            stopped_after_complete_guard: None,
            completed: None,
        }
    }

    fn stopped_after_complete_guard(record: StoppedAfterCompleteGuardProofDigestRecord) -> Self {
        Self {
            blocked_before_root: None,
            blocked_after_partial: None,
            stopped_after_complete_guard: Some(record),
            completed: None,
        }
    }

    fn completed(record: CompletedGuardProofDigestRecord) -> Self {
        Self {
            blocked_before_root: None,
            blocked_after_partial: None,
            stopped_after_complete_guard: None,
            completed: Some(record),
        }
    }
}

impl Serialize for SupportRecoveryGuardProofDigestRecord {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match (
            &self.blocked_before_root,
            &self.blocked_after_partial,
            &self.stopped_after_complete_guard,
            &self.completed,
        ) {
            (Some(record), None, None, None) => record.serialize(serializer),
            (None, Some(record), None, None) => record.serialize(serializer),
            (None, None, Some(record), None) => record.serialize(serializer),
            (None, None, None, Some(record)) => record.serialize(serializer),
            _ => Err(serde::ser::Error::custom(
                "guard proof digest record must contain exactly one outcome",
            )),
        }
    }
}

impl contract_digest_record_sealed::Sealed for SupportRecoveryGuardProofDigestRecord {}
impl ContractDigestRecord for SupportRecoveryGuardProofDigestRecord {}

impl JsonSchema for SupportRecoveryGuardProofDigestRecord {
    fn schema_name() -> Cow<'static, str> {
        "SupportRecoveryGuardProofDigestRecord".into()
    }
    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<BlockedBeforeRootGuardProofDigestRecord>(),
            generator.subschema_for::<BlockedAfterPartialGuardProofDigestRecord>(),
            generator.subschema_for::<StoppedAfterCompleteGuardProofDigestRecord>(),
            generator.subschema_for::<CompletedGuardProofDigestRecord>(),
        ])
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct BlockedBeforeRootGuardProof {
    #[serde(flatten)]
    record: BlockedBeforeRootGuardProofDigestRecord,
    proof_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct BlockedAfterPartialGuardProof {
    #[serde(flatten)]
    record: BlockedAfterPartialGuardProofDigestRecord,
    proof_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct StoppedAfterCompleteGuardProof {
    #[serde(flatten)]
    record: StoppedAfterCompleteGuardProofDigestRecord,
    proof_digest: Sha256Digest,
}

impl JsonSchema for StoppedAfterCompleteGuardProof {
    fn schema_name() -> Cow<'static, str> {
        "StoppedAfterCompleteGuardProof".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<SeparateStoppedAfterCompleteGuardProofSchema>(),
            generator.subschema_for::<ReservedStoppedAfterCompleteGuardProofSchema>(),
        ])
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CompletedGuardProof {
    #[serde(flatten)]
    record: CompletedGuardProofDigestRecord,
    proof_digest: Sha256Digest,
}

impl JsonSchema for CompletedGuardProof {
    fn schema_name() -> Cow<'static, str> {
        "CompletedGuardProof".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<SeparateCompletedGuardProofSchema>(),
            generator.subschema_for::<ReservedCompletedGuardProofSchema>(),
        ])
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SupportRecoveryGuardProof {
    #[serde(flatten)]
    record: SupportRecoveryGuardProofDigestRecord,
    proof_digest: Sha256Digest,
}

impl JsonSchema for SupportRecoveryGuardProof {
    fn schema_name() -> Cow<'static, str> {
        "SupportRecoveryGuardProof".into()
    }
    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<BlockedBeforeRootGuardProof>(),
            generator.subschema_for::<BlockedAfterPartialGuardProof>(),
            generator.subschema_for::<StoppedAfterCompleteGuardProof>(),
            generator.subschema_for::<CompletedGuardProof>(),
        ])
    }
}

impl SupportRecoveryGuardProof {
    pub(crate) fn new(
        authority: SupportRecoveryGuardAuthority,
    ) -> Result<Self, SupportTerminalizationContractError> {
        let SupportRecoveryGuardAuthority {
            plan,
            guard_receipt_id,
            blocked_before_root,
            blocked_after_partial,
            stopped_after_complete_guard,
            completed,
        } = authority;
        match (
            blocked_before_root,
            blocked_after_partial,
            stopped_after_complete_guard,
            completed,
        ) {
            (
                Some(BlockedBeforeRootGuardAuthority {
                    failed_target,
                    failed_target_display,
                    locked_by,
                }),
                None,
                None,
                None,
            ) => {
                let record = BlockedBeforeRootGuardProofDigestRecord {
                    outcome: BlockedBeforeRootOutcome::Value,
                    guard_receipt_id,
                    manual_target_mode: plan.manual_target_mode(),
                    finalization_plan_digest: plan.finalization_plan_digest,
                    planned_lock_targets: plan.planned_lock_targets,
                    acquired_in_order: EmptySupportRecoveryLockTargets::default(),
                    failed_target,
                    failed_target_display,
                    locked_by,
                    authorization_outcome: UnchangedAuthorizationOutcome::Value,
                    released_in_reverse_order: EmptySupportRecoveryLockTargets::default(),
                    release_verified: TrueLiteral,
                };
                let digest_record =
                    SupportRecoveryGuardProofDigestRecord::blocked_before_root(record);
                let proof_digest = terminalization_digest(
                    &digest_record,
                    "blocked-before-root guard proof digest failed",
                )?;
                Ok(Self {
                    record: digest_record,
                    proof_digest,
                })
            }
            (
                None,
                Some(BlockedAfterPartialGuardAuthority {
                    acquired_in_order,
                    failed_target,
                    failed_target_display,
                    locked_by,
                    released_in_reverse_order,
                }),
                None,
                None,
            ) => {
                let record = BlockedAfterPartialGuardProofDigestRecord {
                    outcome: BlockedAfterPartialOutcome::Value,
                    guard_receipt_id,
                    manual_target_mode: plan.manual_target_mode(),
                    finalization_plan_digest: plan.finalization_plan_digest,
                    planned_lock_targets: plan.planned_lock_targets,
                    acquired_in_order,
                    failed_target,
                    failed_target_display,
                    locked_by,
                    authorization_outcome: UnchangedAuthorizationOutcome::Value,
                    released_in_reverse_order,
                    release_verified: TrueLiteral,
                };
                let digest_record =
                    SupportRecoveryGuardProofDigestRecord::blocked_after_partial(record);
                let proof_digest = terminalization_digest(
                    &digest_record,
                    "blocked-after-partial guard proof digest failed",
                )?;
                Ok(Self {
                    record: digest_record,
                    proof_digest,
                })
            }
            (
                None,
                None,
                Some(StoppedAfterCompleteGuardAuthority {
                    manual_actor_lock_inventory_proof,
                    reserved_original_lease_stop_evidence,
                    manual_working_infobase_stop_evidence,
                    guard_release_receipt_id,
                    history_from_cursor,
                    history_through_cursor,
                    history_partition_digest,
                    released_in_reverse_order,
                }),
                None,
            ) => {
                let record = StoppedAfterCompleteGuardProofDigestRecord {
                    outcome: StoppedAfterCompleteGuardOutcome::Value,
                    guard_receipt_id,
                    guard_release_receipt_id,
                    manual_target_mode: plan.manual_target_mode(),
                    manual_actor_lock_inventory_proof,
                    reserved_original_lease_stop_evidence,
                    manual_working_infobase_stop_evidence,
                    finalization_plan_digest: plan.finalization_plan_digest,
                    acquired_in_order: plan.planned_lock_targets.clone(),
                    planned_lock_targets: plan.planned_lock_targets,
                    history_from_cursor,
                    history_through_cursor,
                    history_partition_digest,
                    support_graph_rechecked_under_guard: TrueLiteral,
                    corrective_before_state_binding_verified: TrueLiteral,
                    content_rechecked_under_guard: TrueLiteral,
                    original_rechecked_under_guard: TrueLiteral,
                    selective_update_performed: FalseLiteral,
                    authorization_outcome: UnchangedAuthorizationOutcome::Value,
                    released_in_reverse_order,
                    release_verified: TrueLiteral,
                };
                let digest_record =
                    SupportRecoveryGuardProofDigestRecord::stopped_after_complete_guard(record);
                let proof_digest = terminalization_digest(
                    &digest_record,
                    "stopped complete-guard proof digest failed",
                )?;
                Ok(Self {
                    record: digest_record,
                    proof_digest,
                })
            }
            (
                None,
                None,
                None,
                Some(CompletedGuardAuthority {
                    manual_actor_lock_inventory_proof,
                    reserved_original_terminalization_proof,
                    manual_working_infobase_closure_proof,
                    guard_release_receipt_id,
                    history_from_cursor,
                    history_through_cursor,
                    history_partition_digest,
                    selective_update_proof,
                    post_release_observed_history_cursor,
                    post_release_history_partition,
                    deferred_repository_advance,
                    authorization_terminalization_receipt,
                    authorization_outcome,
                    released_in_reverse_order,
                }),
            ) => {
                let record = CompletedGuardProofDigestRecord {
                    outcome: CompletedGuardOutcome::Value,
                    guard_receipt_id,
                    guard_release_receipt_id,
                    manual_target_mode: plan.manual_target_mode(),
                    manual_actor_lock_inventory_proof,
                    reserved_original_terminalization_proof,
                    manual_working_infobase_closure_proof,
                    finalization_plan_digest: plan.finalization_plan_digest,
                    acquired_in_order: plan.planned_lock_targets.clone(),
                    planned_lock_targets: plan.planned_lock_targets,
                    history_from_cursor,
                    history_through_cursor,
                    history_partition_digest,
                    support_graph_rechecked_under_guard: TrueLiteral,
                    corrective_before_state_binding_verified: TrueLiteral,
                    content_rechecked_under_guard: TrueLiteral,
                    original_rechecked_under_guard: TrueLiteral,
                    selective_update_proof,
                    post_release_observed_history_cursor,
                    post_release_history_partition,
                    deferred_repository_advance,
                    authorization_terminalization_receipt,
                    authorization_outcome,
                    released_in_reverse_order,
                    release_verified: TrueLiteral,
                };
                let digest_record = SupportRecoveryGuardProofDigestRecord::completed(record);
                let proof_digest =
                    terminalization_digest(&digest_record, "completed guard proof digest failed")?;
                Ok(Self {
                    record: digest_record,
                    proof_digest,
                })
            }
            _ => Err(SupportTerminalizationContractError(
                "support recovery guard authority must contain exactly one outcome",
            )),
        }
    }

    pub(crate) fn finalization_plan_digest(&self) -> &Sha256Digest {
        match (
            &self.record.blocked_before_root,
            &self.record.blocked_after_partial,
            &self.record.stopped_after_complete_guard,
            &self.record.completed,
        ) {
            (Some(value), None, None, None) => &value.finalization_plan_digest,
            (None, Some(value), None, None) => &value.finalization_plan_digest,
            (None, None, Some(value), None) => &value.finalization_plan_digest,
            (None, None, None, Some(value)) => &value.finalization_plan_digest,
            _ => unreachable!("guard proof constructor preserves exactly one outcome"),
        }
    }

    pub(crate) fn manual_target_mode(&self) -> ManualSupportTargetMode {
        match (
            &self.record.blocked_before_root,
            &self.record.blocked_after_partial,
            &self.record.stopped_after_complete_guard,
            &self.record.completed,
        ) {
            (Some(value), None, None, None) => value.manual_target_mode,
            (None, Some(value), None, None) => value.manual_target_mode,
            (None, None, Some(value), None) => value.manual_target_mode,
            (None, None, None, Some(value)) => value.manual_target_mode,
            _ => unreachable!("guard proof constructor preserves exactly one outcome"),
        }
    }

    pub(crate) fn blocked_target_ref(&self) -> Option<BlockedSupportRecoveryTargetRef<'_>> {
        match (
            &self.record.blocked_before_root,
            &self.record.blocked_after_partial,
            &self.record.stopped_after_complete_guard,
            &self.record.completed,
        ) {
            (Some(value), None, None, None) => value
                .planned_lock_targets
                .as_slice()
                .first()
                .map(SupportRecoveryLockTarget::blocked_target_ref),
            (None, Some(value), None, None) => value
                .planned_lock_targets
                .as_slice()
                .get(value.acquired_in_order.as_slice().len())
                .map(SupportRecoveryLockTarget::blocked_target_ref),
            (None, None, Some(_), None) | (None, None, None, Some(_)) => None,
            _ => unreachable!("guard proof constructor preserves exactly one outcome"),
        }
    }

    pub(crate) fn blocked_target_display(&self) -> Option<&RepositoryTargetDisplay> {
        match (
            &self.record.blocked_before_root,
            &self.record.blocked_after_partial,
            &self.record.stopped_after_complete_guard,
            &self.record.completed,
        ) {
            (Some(value), None, None, None) => Some(&value.failed_target_display),
            (None, Some(value), None, None) => Some(&value.failed_target_display),
            (None, None, Some(_), None) | (None, None, None, Some(_)) => None,
            _ => unreachable!("guard proof constructor preserves exactly one outcome"),
        }
    }

    pub(crate) fn blocked_owner(&self) -> Option<&RequiredNullable<RepositoryOwnerIdentity>> {
        match (
            &self.record.blocked_before_root,
            &self.record.blocked_after_partial,
            &self.record.stopped_after_complete_guard,
            &self.record.completed,
        ) {
            (Some(value), None, None, None) => Some(&value.locked_by),
            (None, Some(value), None, None) => Some(&value.locked_by),
            (None, None, Some(_), None) | (None, None, None, Some(_)) => None,
            _ => unreachable!("guard proof constructor preserves exactly one outcome"),
        }
    }

    pub(crate) const fn is_completed(&self) -> bool {
        self.record.completed.is_some()
    }

    pub(crate) fn manual_working_infobase_stop_evidence(
        &self,
    ) -> Option<&ManualWorkingInfobaseStopEvidence> {
        self.record
            .stopped_after_complete_guard
            .as_ref()
            .and_then(|record| record.manual_working_infobase_stop_evidence.as_ref())
    }

    pub(crate) fn reserved_original_lease_stop_evidence(
        &self,
    ) -> Option<&ReservedOriginalLeaseStopEvidence> {
        self.record
            .stopped_after_complete_guard
            .as_ref()
            .and_then(|record| record.reserved_original_lease_stop_evidence.as_ref())
    }

    pub(crate) const fn proof_digest(&self) -> &Sha256Digest {
        &self.proof_digest
    }
}

fn terminalization_digest<T: ContractDigestRecord>(
    record: &T,
    failure: &'static str,
) -> Result<Sha256Digest, SupportTerminalizationContractError> {
    canonical_contract_digest(record, None)
        .map_err(|_| SupportTerminalizationContractError(failure))
}
