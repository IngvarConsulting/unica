#[cfg(test)]
mod tests {
    use super::super::authorization::{
        ActiveSupportActionResumeHandle, SupportActionAuthorizationAuthority,
        SupportActionAuthorizationInputs, SupportActionPhaseBinding,
    };
    use super::super::model::{
        RootReachableSupportLayerSet, SupportActionPurpose, SupportTransition,
    };
    use super::*;
    use crate::domain::branched_development::canonical_json::{
        canonical_contract_digest, contract_digest_record_sealed, ContractDigestRecord,
    };
    use crate::domain::branched_development::contracts::instructions::ManualSupportInstruction;
    use crate::domain::branched_development::contracts::repository::{
        EvidenceSourceIndex, EvidenceSourceIndexCandidate, EvidenceSourceRegistry,
        RepositoryContractError, RepositoryHistoryEvidenceBytesResolver,
        RepositoryHistoryOrderEvidence, RepositoryHistoryOrderResolver,
        RepositoryHistoryPartitionResolver, RepositoryHistorySourceEvidenceRef,
        UnvalidatedRepositoryHistoryPartition,
    };
    use crate::domain::branched_development::contracts::scalars::RepositoryIdentityComponent;
    use crate::domain::branched_development::contracts::schema::audit_json_schema;
    use crate::domain::branched_development::contracts::support::ManualSupportTargetMode;
    use crate::domain::branched_development::{CapabilityRowId, ProfileArtifactRefId};
    use schemars::{schema_for, JsonSchema};
    use serde::{de::DeserializeOwned, Serialize};
    use serde_json::{json, Value};

    const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const ID_1: &str = "11111111-1111-4111-8111-111111111111";
    const ID_2: &str = "22222222-2222-4222-8222-222222222222";
    const ID_3: &str = "33333333-3333-4333-8333-333333333333";

    fn digest(value: &str) -> Sha256Digest {
        Sha256Digest::parse(value).unwrap()
    }

    fn id(value: &str) -> UnicaId {
        UnicaId::parse(value).unwrap()
    }

    fn capability(value: &str) -> CapabilityRowId {
        CapabilityRowId::parse(value).unwrap()
    }

    fn accepts<T: DeserializeOwned>(value: Value) -> T {
        serde_json::from_value(value.clone())
            .unwrap_or_else(|error| panic!("contract rejected {value}: {error}"))
    }

    fn rejects<T: DeserializeOwned>(value: Value) {
        assert!(
            serde_json::from_value::<T>(value.clone()).is_err(),
            "contract accepted {value}"
        );
    }

    fn assert_closed<T: JsonSchema>() {
        let schema = serde_json::to_value(schema_for!(T)).unwrap();
        audit_json_schema(&schema).expect("support evidence schema must be recursively closed");
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

    struct UnexpectedIndex;

    impl EvidenceSourceIndex for UnexpectedIndex {
        fn candidate_for(
            &self,
            _repository_version: &crate::domain::branched_development::contracts::scalars::RepositoryVersion,
            _registry: &EvidenceSourceRegistry,
        ) -> Result<EvidenceSourceIndexCandidate, RepositoryContractError> {
            panic!("empty history partition must not consult the source index")
        }
    }

    struct UnexpectedOrder;

    impl RepositoryHistoryOrderResolver for UnexpectedOrder {
        fn order_evidence(
            &self,
            _from_exclusive: &RepositoryHistoryCursor,
            _through_inclusive: &RepositoryHistoryCursor,
        ) -> Result<RepositoryHistoryOrderEvidence, RepositoryContractError> {
            panic!("empty history partition must not consult history order")
        }
    }

    struct UnexpectedBytes;

    impl RepositoryHistoryEvidenceBytesResolver for UnexpectedBytes {
        fn load_canonical_evidence_bytes(
            &self,
            _reference: &RepositoryHistorySourceEvidenceRef,
        ) -> Result<Vec<u8>, RepositoryContractError> {
            panic!("empty history partition must not load evidence")
        }
    }

    fn cursor(version: &str, prefix_digest: &str) -> RepositoryHistoryCursor {
        serde_json::from_value(json!({
            "throughVersion": version,
            "historyPrefixDigest": prefix_digest,
        }))
        .unwrap()
    }

    fn empty_partition(endpoint: &RepositoryHistoryCursor) -> ValidatedRepositoryHistoryPartition {
        let partition_digest = canonical_contract_digest(
            &EmptyPartitionDigestRecord {
                from_exclusive: endpoint.clone(),
                through_inclusive: endpoint.clone(),
                entries: Vec::new(),
            },
            None,
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
            &UnexpectedIndex,
            &UnexpectedOrder,
            &UnexpectedBytes,
        )
        .validate(wire)
        .unwrap()
    }

    fn owner(username: &str) -> RepositoryOwnerIdentity {
        serde_json::from_value(json!({
            "username": username,
            "computer": null,
            "infobase": null,
            "lockedAt": null,
        }))
        .unwrap()
    }

    #[test]
    fn root_lock_observation_preserves_explicit_null_and_rehashes_on_load() {
        let observation = SupportRootLockObservation::new(RequiredNullable::null()).unwrap();
        let encoded = serde_json::to_value(&observation).unwrap();
        assert_eq!(encoded["mode"], json!("readOnlySnapshot"));
        assert_eq!(encoded["completeness"], json!("readOnlySnapshotProven"));
        assert_eq!(encoded["owner"], Value::Null);
        accepts::<SupportRootLockObservation>(encoded.clone());

        let mut omitted = encoded.clone();
        omitted.as_object_mut().unwrap().remove("owner");
        rejects::<SupportRootLockObservation>(omitted);

        let mut substituted = encoded;
        substituted["observationDigest"] = json!(A);
        rejects::<SupportRootLockObservation>(substituted);
        assert_closed::<SupportRootLockObservation>();
    }

    #[test]
    fn root_guard_proof_forbids_terminalization_digest_for_unchanged_outcome() {
        assert!(SupportRootLockProof::new(
            id(ID_1),
            id(ID_2),
            SupportAuthorizationOutcome::Unchanged,
            Some(digest(A)),
        )
        .is_err());
        let proof = SupportRootLockProof::new(
            id(ID_1),
            id(ID_2),
            SupportAuthorizationOutcome::Unchanged,
            None,
        )
        .unwrap();
        let encoded = serde_json::to_value(&proof).unwrap();
        assert!(encoded
            .get("reservedOriginalTerminalizationProofDigest")
            .is_none());
        accepts::<SupportRootLockProof>(encoded);
        assert_closed::<SupportRootLockProofDigestRecord>();
    }

    #[test]
    fn manual_lock_inventory_and_reserved_lease_stop_bind_exact_observations() {
        assert!(ManualActorLockInventoryProof::new(
            RepositoryUsername::parse("reserved-user").unwrap(),
            digest(A),
            digest(B),
        )
        .is_err());
        let inventory = ManualActorLockInventoryProof::new(
            RepositoryUsername::parse("reserved-user").unwrap(),
            digest(A),
            digest(A),
        )
        .unwrap();
        accepts::<ManualActorLockInventoryProof>(serde_json::to_value(inventory).unwrap());

        let stop = ReservedOriginalLeaseStopEvidence::new(
            digest(A),
            capability("reserved-original-lease.v1"),
            RequiredNullable::null(),
        )
        .unwrap();
        let encoded = serde_json::to_value(stop).unwrap();
        assert_eq!(encoded["leaseOwner"], Value::Null);
        assert_eq!(encoded["exclusiveLeaseAcquired"], json!(false));
        accepts::<ReservedOriginalLeaseStopEvidence>(encoded);
    }

    #[test]
    fn reserved_original_terminalization_proof_rejects_fingerprint_substitution() {
        assert!(ReservedOriginalTerminalizationProof::new(
            digest(A),
            capability("reserved-original-lease.v1"),
            id(ID_1),
            id(ID_2),
            digest(A),
            digest(B),
        )
        .is_err());
        let proof = ReservedOriginalTerminalizationProof::new(
            digest(A),
            capability("reserved-original-lease.v1"),
            id(ID_1),
            id(ID_2),
            digest(B),
            digest(B),
        )
        .unwrap();
        accepts::<ReservedOriginalTerminalizationProof>(serde_json::to_value(proof).unwrap());
        assert_closed::<ReservedOriginalTerminalizationProofDigestRecord>();
    }

    #[test]
    fn recovery_handoff_enforces_mode_fields_and_safe_cf_leaf() {
        let common = SupportRecoveryDistributionHandoffInputs {
            handoff_id: id(ID_1),
            profile_artifact_ref_id: ProfileArtifactRefId::parse("vendor.layer-a").unwrap(),
            profile_artifact_display: DisplayPath::parse("Vendor layer A").unwrap(),
            user_visible_file_name: UserVisibleCfFileName::parse("vendor-layer-a.cf").unwrap(),
            manual_actor_username: RepositoryUsername::parse("support-user").unwrap(),
            layer_id: SupportLayerId::parse("layer-a").unwrap(),
            distribution_artifact_id: id(ID_2),
            artifact_sha256: digest(A),
            readability_probe_receipt_id: id(ID_3),
            manual_readability_capability_row_id: capability("manual-readability.v1"),
            retention_lease_id: id(ID_1),
            retention_receipt_id: id(ID_2),
            retention_capability_row_id: capability("retention-provider.v1"),
        };
        assert!(SupportRecoveryDistributionHandoff::new(
            ManualSupportTargetMode::ReservedOriginal,
            Some(working_identity()),
            common.clone(),
        )
        .is_err());
        assert!(SupportRecoveryDistributionHandoff::new(
            ManualSupportTargetMode::SeparateWorkingInfobase,
            None,
            common.clone(),
        )
        .is_err());
        let handoff = SupportRecoveryDistributionHandoff::new(
            ManualSupportTargetMode::ReservedOriginal,
            None,
            common,
        )
        .unwrap();
        let encoded = serde_json::to_value(handoff).unwrap();
        assert!(encoded.get("workingInfobaseIdentity").is_none());
        assert_eq!(encoded["retentionLeaseHeld"], json!(true));

        for unsafe_name in [
            "../vendor.cf",
            "vendor/cf.cf",
            "vendor.CF",
            ".",
            "vendor.txt",
        ] {
            assert!(
                UserVisibleCfFileName::parse(unsafe_name).is_err(),
                "accepted {unsafe_name}"
            );
        }
        assert_closed::<SupportRecoveryDistributionHandoff>();
    }

    #[test]
    fn recovery_distribution_binds_the_exact_handoff_identity() {
        let handoff = handoff();
        let evidence = SupportRecoveryDistributionEvidence::new(
            SupportLayerId::parse("layer-a").unwrap(),
            id(ID_2),
            digest(A),
            digest(B),
            capability("support-recovery.v1"),
            handoff.clone(),
        )
        .unwrap();
        accepts::<UnvalidatedSupportRecoveryDistributionEvidence>(
            serde_json::to_value(evidence.clone()).unwrap(),
        );
        assert!(SupportRecoveryDistributionEvidence::new(
            SupportLayerId::parse("other-layer").unwrap(),
            id(ID_2),
            digest(A),
            digest(B),
            capability("support-recovery.v1"),
            handoff,
        )
        .is_err());
        assert_closed::<SupportRecoveryDistributionEvidenceDigestRecord>();
    }

    #[test]
    fn manual_recovery_coverage_is_exact_and_transition_bound() {
        let layer_a = SupportLayerId::parse("layer-a").unwrap();
        let evidence = SupportRecoveryDistributionEvidence::new(
            layer_a.clone(),
            id(ID_2),
            digest(A),
            digest(B),
            capability("support-recovery.v1"),
            handoff(),
        )
        .unwrap();
        let distributions = SupportRecoveryDistributionSet::new(vec![evidence]).unwrap();
        let transitions = SupportTransitions::new(vec![
            super::super::model::SupportTransition::enable_configuration_changes(
                crate::domain::branched_development::contracts::scalars::RepositoryTargetDisplay::parse(
                    "Configuration",
                )
                .unwrap(),
                layer_a.clone(),
            ),
        ])
        .unwrap();
        let reachable =
            RootReachableSupportLayerSet::from_capability_adapter(vec![layer_a], digest(A))
                .unwrap();
        SupportRecoveryDistributionCoverageAuthority::prove_complete(
            reachable,
            distributions,
            &transitions,
        )
        .unwrap();

        let wrong_reachable = RootReachableSupportLayerSet::from_capability_adapter(
            vec![SupportLayerId::parse("layer-b").unwrap()],
            digest(A),
        )
        .unwrap();
        let empty = SupportRecoveryDistributionSet::new(Vec::new()).unwrap();
        assert!(
            SupportRecoveryDistributionCoverageAuthority::prove_complete(
                wrong_reachable,
                empty,
                &transitions,
            )
            .is_err()
        );
    }

    // Stable negative-impl assertion: a validated history partition must never
    // be reconstructed by plain Serde beneath an arming receipt.
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

    assert_not_deserialize_owned!(SupportActionArmingReceipt);
    assert_not_deserialize_owned!(SupportArmStaleEvidence);
    assert_not_deserialize_owned!(SupportRecoveryDistributionEvidence);

    #[test]
    fn arming_and_stale_digest_records_are_closed_and_acyclic() {
        assert_closed::<SupportActionArmingReceipt>();
        assert_closed::<SupportActionArmingReceiptDigestRecord>();
        assert_closed::<SupportArmStaleEvidence>();
        assert_closed::<SupportArmStaleEvidenceDigestRecord>();
    }

    #[test]
    fn arming_receipt_binds_endpoints_owner_and_every_action_digest() {
        let endpoint = cursor("v1", A);
        let expected_owner = owner("support-user");
        let root_lock =
            SupportRootLockObservation::new(RequiredNullable::value(expected_owner.clone()))
                .unwrap();
        let receipt = SupportActionArmingReceipt::new(
            id(ID_1),
            id(ID_2),
            digest(A),
            endpoint.clone(),
            endpoint.clone(),
            empty_partition(&endpoint),
            digest(A),
            digest(A),
            digest(A),
            digest(A),
            digest(A),
            digest(A),
            ManualSupportTargetMode::ReservedOriginal,
            root_lock.clone(),
            &expected_owner,
        )
        .unwrap();
        let baseline_digest = receipt.receipt_digest().clone();

        let graph_substitution = SupportActionArmingReceipt::new(
            id(ID_1),
            id(ID_2),
            digest(A),
            endpoint.clone(),
            endpoint.clone(),
            empty_partition(&endpoint),
            digest(A),
            digest(A),
            digest(A),
            digest(B),
            digest(A),
            digest(A),
            ManualSupportTargetMode::ReservedOriginal,
            root_lock.clone(),
            &expected_owner,
        )
        .unwrap();
        assert_ne!(graph_substitution.receipt_digest(), &baseline_digest);

        let recovery_substitution = SupportActionArmingReceipt::new(
            id(ID_1),
            id(ID_2),
            digest(A),
            endpoint.clone(),
            endpoint.clone(),
            empty_partition(&endpoint),
            digest(A),
            digest(A),
            digest(A),
            digest(A),
            digest(B),
            digest(A),
            ManualSupportTargetMode::ReservedOriginal,
            root_lock.clone(),
            &expected_owner,
        )
        .unwrap();
        assert_ne!(recovery_substitution.receipt_digest(), &baseline_digest);

        assert!(SupportActionArmingReceipt::new(
            id(ID_1),
            id(ID_2),
            digest(A),
            cursor("v0", B),
            endpoint.clone(),
            empty_partition(&endpoint),
            digest(A),
            digest(A),
            digest(A),
            digest(A),
            digest(A),
            digest(A),
            ManualSupportTargetMode::ReservedOriginal,
            root_lock.clone(),
            &expected_owner,
        )
        .is_err());
        assert!(SupportActionArmingReceipt::new(
            id(ID_1),
            id(ID_2),
            digest(A),
            endpoint.clone(),
            endpoint.clone(),
            empty_partition(&endpoint),
            digest(A),
            digest(A),
            digest(A),
            digest(A),
            digest(A),
            digest(A),
            ManualSupportTargetMode::ReservedOriginal,
            root_lock,
            &owner("other-user"),
        )
        .is_err());
    }

    #[test]
    fn armed_projection_reconstructs_manual_instruction_from_accepted_receipt() {
        let endpoint = cursor("v1", A);
        let layer_id = SupportLayerId::parse("layer-a").unwrap();
        let transitions = SupportTransitions::new(vec![
            SupportTransition::enable_configuration_changes(
                crate::domain::branched_development::contracts::scalars::RepositoryTargetDisplay::parse(
                    "Configuration",
                )
                .unwrap(),
                layer_id.clone(),
            ),
        ])
        .unwrap();
        let recovery = SupportRecoveryDistributionEvidence::new(
            layer_id.clone(),
            id(ID_2),
            digest(A),
            digest(B),
            capability("support-recovery.v1"),
            handoff(),
        )
        .unwrap();
        let recovery_set = SupportRecoveryDistributionSet::new(vec![recovery]).unwrap();
        let coverage = SupportRecoveryDistributionCoverageAuthority::prove_complete(
            RootReachableSupportLayerSet::from_capability_adapter(vec![layer_id], digest(A))
                .unwrap(),
            recovery_set,
            &transitions,
        )
        .unwrap();
        let action = SupportActionAuthorizationAuthority::reserved_original(
            SupportActionAuthorizationInputs::fixture(
                id(ID_2),
                SupportActionPurpose::MainIntegrationPrerequisite,
                id(ID_3),
                digest(A),
                digest(A),
                endpoint.clone(),
                digest(A),
                digest(A),
                transitions,
                coverage,
                RepositoryUsername::parse("support-user").unwrap(),
                digest(A),
                digest(A),
                RepositoryUsername::parse("support-user").unwrap(),
                None,
                SupportActionPhaseBinding::main_integration(digest(A)),
            ),
            capability("reserved-original-lease.v1"),
            digest(A),
        )
        .unwrap();
        let expected_owner = owner("support-user");
        let receipt = SupportActionArmingReceipt::new(
            id(ID_1),
            action.support_action_id().clone(),
            action.support_action_digest().clone(),
            endpoint.clone(),
            endpoint.clone(),
            empty_partition(&endpoint),
            action.support_gate_digest().clone(),
            action.candidate_set_digest().clone(),
            action.expected_relevant_baseline_digest().clone(),
            digest(A),
            action.support_recovery_distribution_set_digest().clone(),
            action.expected_original_fingerprint().clone(),
            ManualSupportTargetMode::ReservedOriginal,
            SupportRootLockObservation::new(RequiredNullable::value(expected_owner.clone()))
                .unwrap(),
            &expected_owner,
        )
        .unwrap();
        let armed = ActiveSupportActionResumeHandle::publish(action)
            .unwrap()
            .arm(receipt)
            .unwrap();
        assert!(armed.awaiting_support_instruction_projection().is_none());
        let projection = armed
            .armed_support_instruction_projection()
            .expect("accepted arming receipt yields manual instruction inputs");
        assert_eq!(projection.arming_receipt_id(), &id(ID_1));
        assert_eq!(projection.arming_cursor(), &endpoint);
        let instruction = ManualSupportInstruction::from_armed_projection(&projection).unwrap();
        let instruction = serde_json::to_value(instruction).unwrap();
        assert_eq!(instruction["supportActionId"], json!(ID_2));
        assert_eq!(instruction["armingReceiptId"], json!(ID_1));
        assert_eq!(instruction["repositoryUsername"], json!("support-user"));
        assert_eq!(instruction["transitions"].as_array().map(Vec::len), Some(1));
    }

    #[test]
    fn stale_arm_evidence_derives_exact_mismatch_union_and_binds_root_observation() {
        let endpoint = cursor("v1", A);
        let no_owner = SupportRootLockObservation::new(RequiredNullable::null()).unwrap();
        assert!(SupportArmStaleEvidence::new(
            endpoint.clone(),
            endpoint.clone(),
            empty_partition(&endpoint),
            digest(A),
            digest(A),
            digest(A),
            digest(A),
            digest(A),
            digest(A),
            digest(A),
            digest(A),
            digest(A),
            digest(A),
            no_owner.clone(),
        )
        .is_err());

        let stale = SupportArmStaleEvidence::new(
            endpoint.clone(),
            endpoint.clone(),
            empty_partition(&endpoint),
            digest(A),
            digest(B),
            digest(A),
            digest(B),
            digest(A),
            digest(B),
            digest(A),
            digest(B),
            digest(A),
            digest(B),
            no_owner.clone(),
        )
        .unwrap();
        assert_eq!(
            stale.mismatch_kinds.as_slice(),
            &[
                SupportArmStaleKind::SupportGateChanged,
                SupportArmStaleKind::RelevantBaselineChanged,
                SupportArmStaleKind::SupportGraphChanged,
                SupportArmStaleKind::RecoveryDistributionSetChanged,
                SupportArmStaleKind::OriginalFingerprintChanged,
            ]
        );
        let no_owner_digest = stale.evidence_digest.clone();

        let observed_owner = owner("foreign-user");
        let owned_root =
            SupportRootLockObservation::new(RequiredNullable::value(observed_owner)).unwrap();
        let stale_with_owner = SupportArmStaleEvidence::new(
            endpoint.clone(),
            endpoint.clone(),
            empty_partition(&endpoint),
            digest(A),
            digest(B),
            digest(A),
            digest(B),
            digest(A),
            digest(B),
            digest(A),
            digest(B),
            digest(A),
            digest(B),
            owned_root,
        )
        .unwrap();
        assert_ne!(stale_with_owner.evidence_digest, no_owner_digest);

        assert!(SupportArmStaleEvidence::new(
            cursor("v0", B),
            endpoint.clone(),
            empty_partition(&endpoint),
            digest(A),
            digest(B),
            digest(A),
            digest(A),
            digest(A),
            digest(A),
            digest(A),
            digest(A),
            digest(A),
            digest(A),
            no_owner,
        )
        .is_err());
    }

    #[test]
    fn stale_kind_collection_is_nonempty_unique_and_in_declaration_order() {
        accepts::<SupportArmStaleKinds>(json!([
            "historyChanged",
            "supportGateChanged",
            "originalFingerprintChanged"
        ]));
        rejects::<SupportArmStaleKinds>(json!([]));
        rejects::<SupportArmStaleKinds>(json!(["supportGateChanged", "historyChanged"]));
        rejects::<SupportArmStaleKinds>(json!(["historyChanged", "historyChanged"]));
    }

    fn working_identity() -> ManualWorkingInfobaseIdentity {
        ManualWorkingInfobaseIdentity::new(
            RepositoryIdentityComponent::parse("HOST").unwrap(),
            RepositoryIdentityComponent::parse("Working IB").unwrap(),
        )
        .unwrap()
    }

    fn handoff() -> SupportRecoveryDistributionHandoff {
        SupportRecoveryDistributionHandoff::new(
            ManualSupportTargetMode::ReservedOriginal,
            None,
            SupportRecoveryDistributionHandoffInputs {
                handoff_id: id(ID_1),
                profile_artifact_ref_id: ProfileArtifactRefId::parse("vendor.layer-a").unwrap(),
                profile_artifact_display: DisplayPath::parse("Vendor layer A").unwrap(),
                user_visible_file_name: UserVisibleCfFileName::parse("vendor-layer-a.cf").unwrap(),
                manual_actor_username: RepositoryUsername::parse("support-user").unwrap(),
                layer_id: SupportLayerId::parse("layer-a").unwrap(),
                distribution_artifact_id: id(ID_2),
                artifact_sha256: digest(A),
                readability_probe_receipt_id: id(ID_3),
                manual_readability_capability_row_id: capability("manual-readability.v1"),
                retention_lease_id: id(ID_1),
                retention_receipt_id: id(ID_2),
                retention_capability_row_id: capability("retention-provider.v1"),
            },
        )
        .unwrap()
    }
}
use super::super::repository::{
    RepositoryHistoryCursor, RepositoryHistoryPartitionClassification, RepositoryOwnerIdentity,
    ValidatedRepositoryHistoryPartition,
};
use super::super::scalars::{DisplayPath, RepositoryUsername, RequiredNullable};
use super::super::schema::{
    is_i_json_single_line_text, one_of_schema, string_schema, I_JSON_SINGLE_LINE_TEXT_FORMAT,
};
use super::model::{
    FalseLiteral, ManualSupportTargetMode, ManualWorkingInfobaseIdentity,
    RootReachableSupportLayerSet, SupportArmStaleKind, SupportContractError, SupportEvidenceGaps,
    SupportTransitions, TrueLiteral,
};
use crate::domain::branched_development::canonical_json::{
    canonical_contract_digest, contract_digest_record_sealed, ContractDigestRecord,
};
use crate::domain::branched_development::{
    CapabilityRowId, ProfileArtifactRefId, Sha256Digest, SupportLayerId, UnicaId,
};
use schemars::{json_schema, JsonSchema, Schema, SchemaGenerator};
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};
use std::borrow::Cow;
use std::fmt;

const MAX_RECOVERY_DISTRIBUTIONS: usize = 1_024;

fn contract_digest<T: ContractDigestRecord>(
    record: &T,
    message: &'static str,
) -> Result<Sha256Digest, SupportContractError> {
    canonical_contract_digest(record, None).map_err(|_| SupportContractError(message))
}

macro_rules! wire_literal {
    ($name:ident, $wire:literal) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
        enum $name {
            #[serde(rename = $wire)]
            Value,
        }
    };
}

wire_literal!(ReadOnlySnapshotMode, "readOnlySnapshot");
wire_literal!(ReadOnlySnapshotProvenCompleteness, "readOnlySnapshotProven");
wire_literal!(AcquireRecheckReleaseGuardMode, "acquireRecheckReleaseGuard");
wire_literal!(
    DesignerSessionOpenOrLeaseBusyCause,
    "designerSessionOpenOrLeaseBusy"
);
wire_literal!(ReservedOriginalTargetMode, "reservedOriginal");
wire_literal!(SeparateWorkingInfobaseTargetMode, "separateWorkingInfobase");
wire_literal!(ExternalProfileRetentionOwner, "externalProfile");
wire_literal!(
    ProfileManagedUntilArchivePolicy,
    "profileManagedAtLeastUntilTaskArchive"
);
wire_literal!(
    SupportRecoveryDistributionRole,
    "supportRecoveryDistribution"
);
wire_literal!(ConfigurationDistributionKind, "configurationDistribution");

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SupportRootLockObservationDigestRecord {
    mode: ReadOnlySnapshotMode,
    completeness: ReadOnlySnapshotProvenCompleteness,
    #[serde(deserialize_with = "RequiredNullable::deserialize_required")]
    owner: RequiredNullable<RepositoryOwnerIdentity>,
}

impl contract_digest_record_sealed::Sealed for SupportRootLockObservationDigestRecord {}
impl ContractDigestRecord for SupportRootLockObservationDigestRecord {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SupportRootLockObservation {
    mode: ReadOnlySnapshotMode,
    completeness: ReadOnlySnapshotProvenCompleteness,
    #[serde(deserialize_with = "RequiredNullable::deserialize_required")]
    owner: RequiredNullable<RepositoryOwnerIdentity>,
    observation_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UnvalidatedSupportRootLockObservation {
    mode: ReadOnlySnapshotMode,
    completeness: ReadOnlySnapshotProvenCompleteness,
    #[serde(deserialize_with = "RequiredNullable::deserialize_required")]
    owner: RequiredNullable<RepositoryOwnerIdentity>,
    observation_digest: Sha256Digest,
}

impl SupportRootLockObservation {
    pub(crate) fn new(
        owner: RequiredNullable<RepositoryOwnerIdentity>,
    ) -> Result<Self, SupportContractError> {
        let record = SupportRootLockObservationDigestRecord {
            mode: ReadOnlySnapshotMode::Value,
            completeness: ReadOnlySnapshotProvenCompleteness::Value,
            owner,
        };
        let observation_digest = contract_digest(&record, "root-lock observation digest failed")?;
        Ok(Self {
            mode: record.mode,
            completeness: record.completeness,
            owner: record.owner,
            observation_digest,
        })
    }

    pub(crate) const fn owner(&self) -> &RequiredNullable<RepositoryOwnerIdentity> {
        &self.owner
    }

    pub(crate) const fn observation_digest(&self) -> &Sha256Digest {
        &self.observation_digest
    }
}

impl<'de> Deserialize<'de> for SupportRootLockObservation {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = UnvalidatedSupportRootLockObservation::deserialize(deserializer)?;
        let value = Self::new(wire.owner).map_err(D::Error::custom)?;
        (value.observation_digest == wire.observation_digest)
            .then_some(value)
            .ok_or_else(|| D::Error::custom("root-lock observation digest mismatch"))
    }
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub(crate) enum SupportAuthorizationOutcome {
    Consumed,
    Cancelled,
    Unchanged,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SupportRootLockProofDigestRecord {
    mode: AcquireRecheckReleaseGuardMode,
    guard_receipt_id: UnicaId,
    root_guard_release_receipt_id: UnicaId,
    acquired_by_reserved_account: TrueLiteral,
    history_rechecked_under_guard: TrueLiteral,
    support_graph_rechecked_under_guard: TrueLiteral,
    original_rechecked_under_guard: TrueLiteral,
    release_verified: TrueLiteral,
    authorization_outcome: SupportAuthorizationOutcome,
    #[serde(skip_serializing_if = "Option::is_none")]
    reserved_original_terminalization_proof_digest: Option<Sha256Digest>,
}

impl contract_digest_record_sealed::Sealed for SupportRootLockProofDigestRecord {}
impl ContractDigestRecord for SupportRootLockProofDigestRecord {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SupportRootLockProof {
    mode: AcquireRecheckReleaseGuardMode,
    guard_receipt_id: UnicaId,
    root_guard_release_receipt_id: UnicaId,
    acquired_by_reserved_account: TrueLiteral,
    history_rechecked_under_guard: TrueLiteral,
    support_graph_rechecked_under_guard: TrueLiteral,
    original_rechecked_under_guard: TrueLiteral,
    release_verified: TrueLiteral,
    authorization_outcome: SupportAuthorizationOutcome,
    #[serde(skip_serializing_if = "Option::is_none")]
    reserved_original_terminalization_proof_digest: Option<Sha256Digest>,
    observation_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UnvalidatedSupportRootLockProof {
    mode: AcquireRecheckReleaseGuardMode,
    guard_receipt_id: UnicaId,
    root_guard_release_receipt_id: UnicaId,
    acquired_by_reserved_account: TrueLiteral,
    history_rechecked_under_guard: TrueLiteral,
    support_graph_rechecked_under_guard: TrueLiteral,
    original_rechecked_under_guard: TrueLiteral,
    release_verified: TrueLiteral,
    authorization_outcome: SupportAuthorizationOutcome,
    reserved_original_terminalization_proof_digest: Option<Sha256Digest>,
    observation_digest: Sha256Digest,
}

impl SupportRootLockProof {
    pub(crate) fn new(
        guard_receipt_id: UnicaId,
        root_guard_release_receipt_id: UnicaId,
        authorization_outcome: SupportAuthorizationOutcome,
        reserved_original_terminalization_proof_digest: Option<Sha256Digest>,
    ) -> Result<Self, SupportContractError> {
        if authorization_outcome == SupportAuthorizationOutcome::Unchanged
            && reserved_original_terminalization_proof_digest.is_some()
        {
            return Err(SupportContractError(
                "unchanged root guard cannot carry terminalization proof",
            ));
        }
        let record = SupportRootLockProofDigestRecord {
            mode: AcquireRecheckReleaseGuardMode::Value,
            guard_receipt_id,
            root_guard_release_receipt_id,
            acquired_by_reserved_account: TrueLiteral,
            history_rechecked_under_guard: TrueLiteral,
            support_graph_rechecked_under_guard: TrueLiteral,
            original_rechecked_under_guard: TrueLiteral,
            release_verified: TrueLiteral,
            authorization_outcome,
            reserved_original_terminalization_proof_digest,
        };
        let observation_digest = contract_digest(&record, "root-lock proof digest failed")?;
        Ok(Self {
            mode: record.mode,
            guard_receipt_id: record.guard_receipt_id,
            root_guard_release_receipt_id: record.root_guard_release_receipt_id,
            acquired_by_reserved_account: record.acquired_by_reserved_account,
            history_rechecked_under_guard: record.history_rechecked_under_guard,
            support_graph_rechecked_under_guard: record.support_graph_rechecked_under_guard,
            original_rechecked_under_guard: record.original_rechecked_under_guard,
            release_verified: record.release_verified,
            authorization_outcome: record.authorization_outcome,
            reserved_original_terminalization_proof_digest: record
                .reserved_original_terminalization_proof_digest,
            observation_digest,
        })
    }
}

impl<'de> Deserialize<'de> for SupportRootLockProof {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = UnvalidatedSupportRootLockProof::deserialize(deserializer)?;
        let value = Self::new(
            wire.guard_receipt_id,
            wire.root_guard_release_receipt_id,
            wire.authorization_outcome,
            wire.reserved_original_terminalization_proof_digest,
        )
        .map_err(D::Error::custom)?;
        (value.observation_digest == wire.observation_digest)
            .then_some(value)
            .ok_or_else(|| D::Error::custom("root-lock proof digest mismatch"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ManualActorLockInventoryProofDigestRecord {
    username: RepositoryUsername,
    completeness: ReadOnlySnapshotProvenCompleteness,
    baseline_lock_set_digest: Sha256Digest,
    observed_lock_set_digest: Sha256Digest,
    unchanged_from_baseline: TrueLiteral,
    root_absent: TrueLiteral,
    baseline_was_empty: TrueLiteral,
}

impl contract_digest_record_sealed::Sealed for ManualActorLockInventoryProofDigestRecord {}
impl ContractDigestRecord for ManualActorLockInventoryProofDigestRecord {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ManualActorLockInventoryProof {
    username: RepositoryUsername,
    completeness: ReadOnlySnapshotProvenCompleteness,
    baseline_lock_set_digest: Sha256Digest,
    observed_lock_set_digest: Sha256Digest,
    unchanged_from_baseline: TrueLiteral,
    root_absent: TrueLiteral,
    baseline_was_empty: TrueLiteral,
    observation_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UnvalidatedManualActorLockInventoryProof {
    username: RepositoryUsername,
    completeness: ReadOnlySnapshotProvenCompleteness,
    baseline_lock_set_digest: Sha256Digest,
    observed_lock_set_digest: Sha256Digest,
    unchanged_from_baseline: TrueLiteral,
    root_absent: TrueLiteral,
    baseline_was_empty: TrueLiteral,
    observation_digest: Sha256Digest,
}

impl ManualActorLockInventoryProof {
    pub(crate) fn new(
        username: RepositoryUsername,
        baseline_lock_set_digest: Sha256Digest,
        observed_lock_set_digest: Sha256Digest,
    ) -> Result<Self, SupportContractError> {
        if baseline_lock_set_digest != observed_lock_set_digest {
            return Err(SupportContractError(
                "manual actor lock inventory differs from its empty baseline",
            ));
        }
        let record = ManualActorLockInventoryProofDigestRecord {
            username,
            completeness: ReadOnlySnapshotProvenCompleteness::Value,
            baseline_lock_set_digest,
            observed_lock_set_digest,
            unchanged_from_baseline: TrueLiteral,
            root_absent: TrueLiteral,
            baseline_was_empty: TrueLiteral,
        };
        let observation_digest = contract_digest(&record, "lock-inventory proof digest failed")?;
        Ok(Self {
            username: record.username,
            completeness: record.completeness,
            baseline_lock_set_digest: record.baseline_lock_set_digest,
            observed_lock_set_digest: record.observed_lock_set_digest,
            unchanged_from_baseline: record.unchanged_from_baseline,
            root_absent: record.root_absent,
            baseline_was_empty: record.baseline_was_empty,
            observation_digest,
        })
    }
}

impl<'de> Deserialize<'de> for ManualActorLockInventoryProof {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = UnvalidatedManualActorLockInventoryProof::deserialize(deserializer)?;
        let value = Self::new(
            wire.username,
            wire.baseline_lock_set_digest,
            wire.observed_lock_set_digest,
        )
        .map_err(D::Error::custom)?;
        (value.observation_digest == wire.observation_digest)
            .then_some(value)
            .ok_or_else(|| D::Error::custom("lock-inventory proof digest mismatch"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ReservedOriginalLeaseStopEvidenceDigestRecord {
    cause: DesignerSessionOpenOrLeaseBusyCause,
    reserved_original_identity_digest: Sha256Digest,
    exclusive_lease_capability_id: CapabilityRowId,
    #[serde(deserialize_with = "RequiredNullable::deserialize_required")]
    lease_owner: RequiredNullable<RepositoryOwnerIdentity>,
    exclusive_lease_acquired: FalseLiteral,
}

impl contract_digest_record_sealed::Sealed for ReservedOriginalLeaseStopEvidenceDigestRecord {}
impl ContractDigestRecord for ReservedOriginalLeaseStopEvidenceDigestRecord {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ReservedOriginalLeaseStopEvidence {
    cause: DesignerSessionOpenOrLeaseBusyCause,
    reserved_original_identity_digest: Sha256Digest,
    exclusive_lease_capability_id: CapabilityRowId,
    #[serde(deserialize_with = "RequiredNullable::deserialize_required")]
    lease_owner: RequiredNullable<RepositoryOwnerIdentity>,
    exclusive_lease_acquired: FalseLiteral,
    evidence_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UnvalidatedReservedOriginalLeaseStopEvidence {
    cause: DesignerSessionOpenOrLeaseBusyCause,
    reserved_original_identity_digest: Sha256Digest,
    exclusive_lease_capability_id: CapabilityRowId,
    #[serde(deserialize_with = "RequiredNullable::deserialize_required")]
    lease_owner: RequiredNullable<RepositoryOwnerIdentity>,
    exclusive_lease_acquired: FalseLiteral,
    evidence_digest: Sha256Digest,
}

impl ReservedOriginalLeaseStopEvidence {
    pub(crate) fn new(
        reserved_original_identity_digest: Sha256Digest,
        exclusive_lease_capability_id: CapabilityRowId,
        lease_owner: RequiredNullable<RepositoryOwnerIdentity>,
    ) -> Result<Self, SupportContractError> {
        let record = ReservedOriginalLeaseStopEvidenceDigestRecord {
            cause: DesignerSessionOpenOrLeaseBusyCause::Value,
            reserved_original_identity_digest,
            exclusive_lease_capability_id,
            lease_owner,
            exclusive_lease_acquired: FalseLiteral,
        };
        let evidence_digest = contract_digest(&record, "reserved lease-stop digest failed")?;
        Ok(Self {
            cause: record.cause,
            reserved_original_identity_digest: record.reserved_original_identity_digest,
            exclusive_lease_capability_id: record.exclusive_lease_capability_id,
            lease_owner: record.lease_owner,
            exclusive_lease_acquired: record.exclusive_lease_acquired,
            evidence_digest,
        })
    }
}

impl<'de> Deserialize<'de> for ReservedOriginalLeaseStopEvidence {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = UnvalidatedReservedOriginalLeaseStopEvidence::deserialize(deserializer)?;
        let value = Self::new(
            wire.reserved_original_identity_digest,
            wire.exclusive_lease_capability_id,
            wire.lease_owner,
        )
        .map_err(D::Error::custom)?;
        (value.evidence_digest == wire.evidence_digest)
            .then_some(value)
            .ok_or_else(|| D::Error::custom("reserved lease-stop evidence digest mismatch"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ReservedOriginalTerminalizationProofDigestRecord {
    reserved_original_identity_digest: Sha256Digest,
    exclusive_lease_capability_id: CapabilityRowId,
    exclusive_lease_receipt_id: UnicaId,
    exclusive_lease_release_receipt_id: UnicaId,
    designer_session_closed_before_acquisition: TrueLiteral,
    exclusive_configuration_lease_acquired: TrueLiteral,
    lease_held_through_inspection_and_terminalization: TrueLiteral,
    expected_repository_fingerprint: Sha256Digest,
    observed_original_fingerprint: Sha256Digest,
    original_equals_classified_repository_state: TrueLiteral,
    no_uncommitted_configuration_delta: TrueLiteral,
    lease_released: TrueLiteral,
    lease_release_verified: TrueLiteral,
}

impl contract_digest_record_sealed::Sealed for ReservedOriginalTerminalizationProofDigestRecord {}
impl ContractDigestRecord for ReservedOriginalTerminalizationProofDigestRecord {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ReservedOriginalTerminalizationProof {
    reserved_original_identity_digest: Sha256Digest,
    exclusive_lease_capability_id: CapabilityRowId,
    exclusive_lease_receipt_id: UnicaId,
    exclusive_lease_release_receipt_id: UnicaId,
    designer_session_closed_before_acquisition: TrueLiteral,
    exclusive_configuration_lease_acquired: TrueLiteral,
    lease_held_through_inspection_and_terminalization: TrueLiteral,
    expected_repository_fingerprint: Sha256Digest,
    observed_original_fingerprint: Sha256Digest,
    original_equals_classified_repository_state: TrueLiteral,
    no_uncommitted_configuration_delta: TrueLiteral,
    lease_released: TrueLiteral,
    lease_release_verified: TrueLiteral,
    proof_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UnvalidatedReservedOriginalTerminalizationProof {
    reserved_original_identity_digest: Sha256Digest,
    exclusive_lease_capability_id: CapabilityRowId,
    exclusive_lease_receipt_id: UnicaId,
    exclusive_lease_release_receipt_id: UnicaId,
    designer_session_closed_before_acquisition: TrueLiteral,
    exclusive_configuration_lease_acquired: TrueLiteral,
    lease_held_through_inspection_and_terminalization: TrueLiteral,
    expected_repository_fingerprint: Sha256Digest,
    observed_original_fingerprint: Sha256Digest,
    original_equals_classified_repository_state: TrueLiteral,
    no_uncommitted_configuration_delta: TrueLiteral,
    lease_released: TrueLiteral,
    lease_release_verified: TrueLiteral,
    proof_digest: Sha256Digest,
}

impl ReservedOriginalTerminalizationProof {
    pub(crate) fn new(
        reserved_original_identity_digest: Sha256Digest,
        exclusive_lease_capability_id: CapabilityRowId,
        exclusive_lease_receipt_id: UnicaId,
        exclusive_lease_release_receipt_id: UnicaId,
        expected_repository_fingerprint: Sha256Digest,
        observed_original_fingerprint: Sha256Digest,
    ) -> Result<Self, SupportContractError> {
        if expected_repository_fingerprint != observed_original_fingerprint {
            return Err(SupportContractError(
                "reserved original differs from the classified repository fingerprint",
            ));
        }
        let record = ReservedOriginalTerminalizationProofDigestRecord {
            reserved_original_identity_digest,
            exclusive_lease_capability_id,
            exclusive_lease_receipt_id,
            exclusive_lease_release_receipt_id,
            designer_session_closed_before_acquisition: TrueLiteral,
            exclusive_configuration_lease_acquired: TrueLiteral,
            lease_held_through_inspection_and_terminalization: TrueLiteral,
            expected_repository_fingerprint,
            observed_original_fingerprint,
            original_equals_classified_repository_state: TrueLiteral,
            no_uncommitted_configuration_delta: TrueLiteral,
            lease_released: TrueLiteral,
            lease_release_verified: TrueLiteral,
        };
        let proof_digest = contract_digest(&record, "reserved terminalization digest failed")?;
        Ok(Self {
            reserved_original_identity_digest: record.reserved_original_identity_digest,
            exclusive_lease_capability_id: record.exclusive_lease_capability_id,
            exclusive_lease_receipt_id: record.exclusive_lease_receipt_id,
            exclusive_lease_release_receipt_id: record.exclusive_lease_release_receipt_id,
            designer_session_closed_before_acquisition: record
                .designer_session_closed_before_acquisition,
            exclusive_configuration_lease_acquired: record.exclusive_configuration_lease_acquired,
            lease_held_through_inspection_and_terminalization: record
                .lease_held_through_inspection_and_terminalization,
            expected_repository_fingerprint: record.expected_repository_fingerprint,
            observed_original_fingerprint: record.observed_original_fingerprint,
            original_equals_classified_repository_state: record
                .original_equals_classified_repository_state,
            no_uncommitted_configuration_delta: record.no_uncommitted_configuration_delta,
            lease_released: record.lease_released,
            lease_release_verified: record.lease_release_verified,
            proof_digest,
        })
    }

    pub(crate) const fn proof_digest(&self) -> &Sha256Digest {
        &self.proof_digest
    }
}

impl<'de> Deserialize<'de> for ReservedOriginalTerminalizationProof {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = UnvalidatedReservedOriginalTerminalizationProof::deserialize(deserializer)?;
        let value = Self::new(
            wire.reserved_original_identity_digest,
            wire.exclusive_lease_capability_id,
            wire.exclusive_lease_receipt_id,
            wire.exclusive_lease_release_receipt_id,
            wire.expected_repository_fingerprint,
            wire.observed_original_fingerprint,
        )
        .map_err(D::Error::custom)?;
        (value.proof_digest == wire.proof_digest)
            .then_some(value)
            .ok_or_else(|| D::Error::custom("reserved terminalization proof digest mismatch"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub(crate) struct UserVisibleCfFileName(String);

impl UserVisibleCfFileName {
    pub(crate) fn parse(value: &str) -> Result<Self, SupportContractError> {
        let valid = (1..=255).contains(&value.chars().count())
            && is_i_json_single_line_text(value)
            && value.ends_with(".cf")
            && !value.contains(['/', '\\', ':'])
            && !matches!(value, "." | "..");
        valid
            .then(|| Self(value.to_owned()))
            .ok_or(SupportContractError("invalid user-visible CF file name"))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for UserVisibleCfFileName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for UserVisibleCfFileName {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::parse(&String::deserialize(deserializer)?).map_err(D::Error::custom)
    }
}

impl JsonSchema for UserVisibleCfFileName {
    fn inline_schema() -> bool {
        true
    }

    fn schema_name() -> Cow<'static, str> {
        "UserVisibleCfFileName".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        string_schema(
            1,
            255,
            Some(r"^[^/\\:]+\.cf$"),
            Some(I_JSON_SINGLE_LINE_TEXT_FORMAT),
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SupportRecoveryDistributionHandoffInputs {
    pub(crate) handoff_id: UnicaId,
    pub(crate) profile_artifact_ref_id: ProfileArtifactRefId,
    pub(crate) profile_artifact_display: DisplayPath,
    pub(crate) user_visible_file_name: UserVisibleCfFileName,
    pub(crate) manual_actor_username: RepositoryUsername,
    pub(crate) layer_id: SupportLayerId,
    pub(crate) distribution_artifact_id: UnicaId,
    pub(crate) artifact_sha256: Sha256Digest,
    pub(crate) readability_probe_receipt_id: UnicaId,
    pub(crate) manual_readability_capability_row_id: CapabilityRowId,
    pub(crate) retention_lease_id: UnicaId,
    pub(crate) retention_receipt_id: UnicaId,
    pub(crate) retention_capability_row_id: CapabilityRowId,
}

// Rust macros cannot expand directly to struct fields on stable, so the two
// mode leaves remain explicit. Their duplication makes the wire presence rule
// visible to both Serde and JSON Schema.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReservedOriginalRecoveryHandoff {
    handoff_id: UnicaId,
    profile_artifact_ref_id: ProfileArtifactRefId,
    profile_artifact_display: DisplayPath,
    user_visible_file_name: UserVisibleCfFileName,
    manual_target_mode: ReservedOriginalTargetMode,
    manual_actor_username: RepositoryUsername,
    layer_id: SupportLayerId,
    distribution_artifact_id: UnicaId,
    artifact_sha256: Sha256Digest,
    readability_probe_receipt_id: UnicaId,
    manual_readability_capability_row_id: CapabilityRowId,
    retention_lease_id: UnicaId,
    retention_receipt_id: UnicaId,
    retention_capability_row_id: CapabilityRowId,
    retention_owner: ExternalProfileRetentionOwner,
    retention_policy: ProfileManagedUntilArchivePolicy,
    retention_lease_held: TrueLiteral,
    content_mutation_rejected_while_held: TrueLiteral,
    available_to_manual_actor: TrueLiteral,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SeparateWorkingInfobaseRecoveryHandoff {
    handoff_id: UnicaId,
    profile_artifact_ref_id: ProfileArtifactRefId,
    profile_artifact_display: DisplayPath,
    user_visible_file_name: UserVisibleCfFileName,
    manual_target_mode: SeparateWorkingInfobaseTargetMode,
    manual_actor_username: RepositoryUsername,
    working_infobase_identity: ManualWorkingInfobaseIdentity,
    layer_id: SupportLayerId,
    distribution_artifact_id: UnicaId,
    artifact_sha256: Sha256Digest,
    readability_probe_receipt_id: UnicaId,
    manual_readability_capability_row_id: CapabilityRowId,
    retention_lease_id: UnicaId,
    retention_receipt_id: UnicaId,
    retention_capability_row_id: CapabilityRowId,
    retention_owner: ExternalProfileRetentionOwner,
    retention_policy: ProfileManagedUntilArchivePolicy,
    retention_lease_held: TrueLiteral,
    content_mutation_rejected_while_held: TrueLiteral,
    available_to_manual_actor: TrueLiteral,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[allow(private_interfaces)]
#[serde(untagged)]
pub(crate) enum SupportRecoveryDistributionHandoff {
    ReservedOriginal(ReservedOriginalRecoveryHandoff),
    SeparateWorkingInfobase(SeparateWorkingInfobaseRecoveryHandoff),
}

impl JsonSchema for SupportRecoveryDistributionHandoff {
    fn schema_name() -> Cow<'static, str> {
        "SupportRecoveryDistributionHandoff".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<ReservedOriginalRecoveryHandoff>(),
            generator.subschema_for::<SeparateWorkingInfobaseRecoveryHandoff>(),
        ])
    }
}

impl SupportRecoveryDistributionHandoff {
    pub(crate) fn new(
        manual_target_mode: ManualSupportTargetMode,
        working_infobase_identity: Option<ManualWorkingInfobaseIdentity>,
        inputs: SupportRecoveryDistributionHandoffInputs,
    ) -> Result<Self, SupportContractError> {
        match (manual_target_mode, working_infobase_identity) {
            (ManualSupportTargetMode::ReservedOriginal, None) => {
                Ok(Self::ReservedOriginal(ReservedOriginalRecoveryHandoff {
                    handoff_id: inputs.handoff_id,
                    profile_artifact_ref_id: inputs.profile_artifact_ref_id,
                    profile_artifact_display: inputs.profile_artifact_display,
                    user_visible_file_name: inputs.user_visible_file_name,
                    manual_target_mode: ReservedOriginalTargetMode::Value,
                    manual_actor_username: inputs.manual_actor_username,
                    layer_id: inputs.layer_id,
                    distribution_artifact_id: inputs.distribution_artifact_id,
                    artifact_sha256: inputs.artifact_sha256,
                    readability_probe_receipt_id: inputs.readability_probe_receipt_id,
                    manual_readability_capability_row_id: inputs
                        .manual_readability_capability_row_id,
                    retention_lease_id: inputs.retention_lease_id,
                    retention_receipt_id: inputs.retention_receipt_id,
                    retention_capability_row_id: inputs.retention_capability_row_id,
                    retention_owner: ExternalProfileRetentionOwner::Value,
                    retention_policy: ProfileManagedUntilArchivePolicy::Value,
                    retention_lease_held: TrueLiteral,
                    content_mutation_rejected_while_held: TrueLiteral,
                    available_to_manual_actor: TrueLiteral,
                }))
            }
            (ManualSupportTargetMode::SeparateWorkingInfobase, Some(identity)) => Ok(
                Self::SeparateWorkingInfobase(SeparateWorkingInfobaseRecoveryHandoff {
                    handoff_id: inputs.handoff_id,
                    profile_artifact_ref_id: inputs.profile_artifact_ref_id,
                    profile_artifact_display: inputs.profile_artifact_display,
                    user_visible_file_name: inputs.user_visible_file_name,
                    manual_target_mode: SeparateWorkingInfobaseTargetMode::Value,
                    manual_actor_username: inputs.manual_actor_username,
                    working_infobase_identity: identity,
                    layer_id: inputs.layer_id,
                    distribution_artifact_id: inputs.distribution_artifact_id,
                    artifact_sha256: inputs.artifact_sha256,
                    readability_probe_receipt_id: inputs.readability_probe_receipt_id,
                    manual_readability_capability_row_id: inputs
                        .manual_readability_capability_row_id,
                    retention_lease_id: inputs.retention_lease_id,
                    retention_receipt_id: inputs.retention_receipt_id,
                    retention_capability_row_id: inputs.retention_capability_row_id,
                    retention_owner: ExternalProfileRetentionOwner::Value,
                    retention_policy: ProfileManagedUntilArchivePolicy::Value,
                    retention_lease_held: TrueLiteral,
                    content_mutation_rejected_while_held: TrueLiteral,
                    available_to_manual_actor: TrueLiteral,
                }),
            ),
            _ => Err(SupportContractError(
                "support recovery handoff working-IB presence disagrees with target mode",
            )),
        }
    }

    pub(crate) fn layer_id(&self) -> &SupportLayerId {
        match self {
            Self::ReservedOriginal(value) => &value.layer_id,
            Self::SeparateWorkingInfobase(value) => &value.layer_id,
        }
    }

    pub(crate) fn distribution_artifact_id(&self) -> &UnicaId {
        match self {
            Self::ReservedOriginal(value) => &value.distribution_artifact_id,
            Self::SeparateWorkingInfobase(value) => &value.distribution_artifact_id,
        }
    }

    pub(crate) fn artifact_sha256(&self) -> &Sha256Digest {
        match self {
            Self::ReservedOriginal(value) => &value.artifact_sha256,
            Self::SeparateWorkingInfobase(value) => &value.artifact_sha256,
        }
    }

    pub(crate) fn handoff_id(&self) -> &UnicaId {
        match self {
            Self::ReservedOriginal(value) => &value.handoff_id,
            Self::SeparateWorkingInfobase(value) => &value.handoff_id,
        }
    }

    pub(crate) fn retention_lease_id(&self) -> &UnicaId {
        match self {
            Self::ReservedOriginal(value) => &value.retention_lease_id,
            Self::SeparateWorkingInfobase(value) => &value.retention_lease_id,
        }
    }

    pub(crate) fn manual_readability_capability_row_id(&self) -> &CapabilityRowId {
        match self {
            Self::ReservedOriginal(value) => &value.manual_readability_capability_row_id,
            Self::SeparateWorkingInfobase(value) => &value.manual_readability_capability_row_id,
        }
    }

    pub(crate) fn retention_capability_row_id(&self) -> &CapabilityRowId {
        match self {
            Self::ReservedOriginal(value) => &value.retention_capability_row_id,
            Self::SeparateWorkingInfobase(value) => &value.retention_capability_row_id,
        }
    }

    pub(crate) const fn manual_target_mode(&self) -> ManualSupportTargetMode {
        match self {
            Self::ReservedOriginal(_) => ManualSupportTargetMode::ReservedOriginal,
            Self::SeparateWorkingInfobase(_) => ManualSupportTargetMode::SeparateWorkingInfobase,
        }
    }

    pub(crate) fn manual_actor_username(&self) -> &RepositoryUsername {
        match self {
            Self::ReservedOriginal(value) => &value.manual_actor_username,
            Self::SeparateWorkingInfobase(value) => &value.manual_actor_username,
        }
    }

    pub(crate) fn working_infobase_identity(&self) -> Option<&ManualWorkingInfobaseIdentity> {
        match self {
            Self::ReservedOriginal(_) => None,
            Self::SeparateWorkingInfobase(value) => Some(&value.working_infobase_identity),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SupportRecoveryDistributionEvidenceDigestRecord {
    layer_id: SupportLayerId,
    distribution_artifact_id: UnicaId,
    role: SupportRecoveryDistributionRole,
    verified_kind: ConfigurationDistributionKind,
    artifact_sha256: Sha256Digest,
    vendor_layer_identity_digest: Sha256Digest,
    capability_row_id: CapabilityRowId,
    handoff: SupportRecoveryDistributionHandoff,
}

impl contract_digest_record_sealed::Sealed for SupportRecoveryDistributionEvidenceDigestRecord {}
impl ContractDigestRecord for SupportRecoveryDistributionEvidenceDigestRecord {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SupportRecoveryDistributionEvidence {
    layer_id: SupportLayerId,
    distribution_artifact_id: UnicaId,
    role: SupportRecoveryDistributionRole,
    verified_kind: ConfigurationDistributionKind,
    artifact_sha256: Sha256Digest,
    vendor_layer_identity_digest: Sha256Digest,
    capability_row_id: CapabilityRowId,
    handoff: SupportRecoveryDistributionHandoff,
    evidence_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct UnvalidatedSupportRecoveryDistributionEvidence {
    layer_id: SupportLayerId,
    distribution_artifact_id: UnicaId,
    role: SupportRecoveryDistributionRole,
    verified_kind: ConfigurationDistributionKind,
    artifact_sha256: Sha256Digest,
    vendor_layer_identity_digest: Sha256Digest,
    capability_row_id: CapabilityRowId,
    handoff: SupportRecoveryDistributionHandoff,
    evidence_digest: Sha256Digest,
}

impl SupportRecoveryDistributionEvidence {
    /// Fixture mint only. Production evidence must be promoted by the
    /// contained distribution/handoff resolver after capability, role, layer,
    /// artifact, retention, and readability checks.
    #[cfg(test)]
    pub(crate) fn new(
        layer_id: SupportLayerId,
        distribution_artifact_id: UnicaId,
        artifact_sha256: Sha256Digest,
        vendor_layer_identity_digest: Sha256Digest,
        capability_row_id: CapabilityRowId,
        handoff: SupportRecoveryDistributionHandoff,
    ) -> Result<Self, SupportContractError> {
        if &layer_id != handoff.layer_id()
            || &distribution_artifact_id != handoff.distribution_artifact_id()
            || &artifact_sha256 != handoff.artifact_sha256()
        {
            return Err(SupportContractError(
                "support recovery evidence disagrees with its handoff identity",
            ));
        }
        let record = SupportRecoveryDistributionEvidenceDigestRecord {
            layer_id,
            distribution_artifact_id,
            role: SupportRecoveryDistributionRole::Value,
            verified_kind: ConfigurationDistributionKind::Value,
            artifact_sha256,
            vendor_layer_identity_digest,
            capability_row_id,
            handoff,
        };
        let evidence_digest = contract_digest(&record, "recovery-distribution digest failed")?;
        Ok(Self {
            layer_id: record.layer_id,
            distribution_artifact_id: record.distribution_artifact_id,
            role: record.role,
            verified_kind: record.verified_kind,
            artifact_sha256: record.artifact_sha256,
            vendor_layer_identity_digest: record.vendor_layer_identity_digest,
            capability_row_id: record.capability_row_id,
            handoff: record.handoff,
            evidence_digest,
        })
    }

    pub(crate) const fn layer_id(&self) -> &SupportLayerId {
        &self.layer_id
    }

    pub(crate) const fn evidence_digest(&self) -> &Sha256Digest {
        &self.evidence_digest
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SupportRecoveryDistributionSetDigestEntry {
    layer_id: SupportLayerId,
    evidence_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(transparent)]
pub(crate) struct SupportRecoveryDistributionSetDigestRecord(
    Vec<SupportRecoveryDistributionSetDigestEntry>,
);

impl contract_digest_record_sealed::Sealed for SupportRecoveryDistributionSetDigestRecord {}
impl ContractDigestRecord for SupportRecoveryDistributionSetDigestRecord {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SupportRecoveryDistributionSet {
    distributions: Vec<SupportRecoveryDistributionEvidence>,
    support_recovery_distribution_set_digest: Sha256Digest,
}

impl SupportRecoveryDistributionSet {
    pub(crate) fn new(
        distributions: Vec<SupportRecoveryDistributionEvidence>,
    ) -> Result<Self, SupportContractError> {
        if distributions.len() > MAX_RECOVERY_DISTRIBUTIONS
            || distributions
                .windows(2)
                .any(|pair| pair[0].layer_id() >= pair[1].layer_id())
        {
            return Err(SupportContractError(
                "recovery distributions must be unique, bounded, and layer-sorted",
            ));
        }
        let record = SupportRecoveryDistributionSetDigestRecord(
            distributions
                .iter()
                .map(|value| SupportRecoveryDistributionSetDigestEntry {
                    layer_id: value.layer_id().clone(),
                    evidence_digest: value.evidence_digest().clone(),
                })
                .collect(),
        );
        let support_recovery_distribution_set_digest =
            contract_digest(&record, "recovery-distribution set digest failed")?;
        Ok(Self {
            distributions,
            support_recovery_distribution_set_digest,
        })
    }

    pub(crate) fn as_slice(&self) -> &[SupportRecoveryDistributionEvidence] {
        &self.distributions
    }

    pub(crate) const fn digest(&self) -> &Sha256Digest {
        &self.support_recovery_distribution_set_digest
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.distributions.is_empty()
    }

    pub(crate) fn matches_manual_binding(
        &self,
        mode: ManualSupportTargetMode,
        manual_actor_username: &RepositoryUsername,
        working_infobase_identity: Option<&ManualWorkingInfobaseIdentity>,
    ) -> bool {
        self.distributions.iter().all(|distribution| {
            distribution.handoff.manual_target_mode() == mode
                && distribution.handoff.manual_actor_username() == manual_actor_username
                && distribution.handoff.working_infobase_identity() == working_infobase_identity
        })
    }
}

/// Exact recovery coverage proven against the capability-derived set of every
/// support layer reachable from the configuration root.
///
/// The private fields make this token impossible to fabricate by struct
/// literal.  Manual authorization accepts this authority instead of accepting
/// an arbitrary distribution set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SupportRecoveryDistributionCoverageAuthority {
    root_reachable_layers: RootReachableSupportLayerSet,
    distributions: SupportRecoveryDistributionSet,
}

/// Exact successful-subset/missing-gap proof for an otherwise-manual
/// inconclusive preflight. This authority prevents an arbitrary recovery gap
/// from masking an exact ready, vendor-forbidden, or fully safe manual result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SupportInconclusiveRecoveryCoverageAuthority {
    root_reachable_layers: RootReachableSupportLayerSet,
    distributions: SupportRecoveryDistributionSet,
    required_transitions: SupportTransitions,
    evidence_gaps: SupportEvidenceGaps,
}

impl SupportInconclusiveRecoveryCoverageAuthority {
    pub(crate) fn prove_exact_subset(
        root_reachable_layers: RootReachableSupportLayerSet,
        distributions: SupportRecoveryDistributionSet,
        required_transitions: SupportTransitions,
        evidence_gaps: SupportEvidenceGaps,
    ) -> Result<Self, SupportContractError> {
        if required_transitions.is_empty()
            || required_transitions.as_slice().iter().any(|transition| {
                root_reachable_layers
                    .as_slice()
                    .binary_search(transition.layer_id())
                    .is_err()
            })
            || distributions.as_slice().iter().any(|distribution| {
                root_reachable_layers
                    .as_slice()
                    .binary_search(distribution.layer_id())
                    .is_err()
            })
        {
            return Err(SupportContractError(
                "inconclusive recovery subset escapes the root-reachable support graph",
            ));
        }

        let successful_layers: Vec<_> = distributions
            .as_slice()
            .iter()
            .map(SupportRecoveryDistributionEvidence::layer_id)
            .collect();
        let missing_layers: Vec<_> = root_reachable_layers
            .as_slice()
            .iter()
            .filter(|layer_id| successful_layers.binary_search(layer_id).is_err())
            .collect();
        let mut gap_layers: Vec<_> = evidence_gaps
            .as_slice()
            .iter()
            .filter_map(|gap| gap.recovery_layer_id())
            .collect();
        gap_layers.sort_unstable();
        gap_layers.dedup();
        let gaps_are_manual_safety_only = evidence_gaps
            .as_slice()
            .iter()
            .all(|gap| gap.recovery_layer_id().is_some() || gap.is_manual_working_infobase_gap());
        let has_manual_ib_gap = evidence_gaps
            .as_slice()
            .iter()
            .any(|gap| gap.is_manual_working_infobase_gap());
        if !gaps_are_manual_safety_only
            || missing_layers != gap_layers
            || missing_layers.is_empty() && !has_manual_ib_gap
        {
            return Err(SupportContractError(
                "inconclusive recovery evidence must exactly name every unproven reachable layer",
            ));
        }

        Ok(Self {
            root_reachable_layers,
            distributions,
            required_transitions,
            evidence_gaps,
        })
    }

    pub(crate) const fn support_graph_digest(&self) -> &Sha256Digest {
        self.root_reachable_layers.support_graph_digest()
    }

    pub(crate) const fn distributions(&self) -> &SupportRecoveryDistributionSet {
        &self.distributions
    }

    pub(crate) const fn required_transitions(&self) -> &SupportTransitions {
        &self.required_transitions
    }

    pub(crate) const fn evidence_gaps(&self) -> &SupportEvidenceGaps {
        &self.evidence_gaps
    }
}

impl SupportRecoveryDistributionCoverageAuthority {
    pub(crate) fn prove_complete(
        root_reachable_layers: RootReachableSupportLayerSet,
        distributions: SupportRecoveryDistributionSet,
        required_transitions: &SupportTransitions,
    ) -> Result<Self, SupportContractError> {
        if distributions.is_empty()
            || root_reachable_layers.as_slice().len() != distributions.as_slice().len()
            || root_reachable_layers
                .as_slice()
                .iter()
                .zip(distributions.as_slice())
                .any(|(layer_id, distribution)| layer_id != distribution.layer_id())
        {
            return Err(SupportContractError(
                "manual recovery distributions do not exactly cover root-reachable support layers",
            ));
        }
        if required_transitions.is_empty()
            || required_transitions.as_slice().iter().any(|transition| {
                root_reachable_layers
                    .as_slice()
                    .binary_search(transition.layer_id())
                    .is_err()
            })
        {
            return Err(SupportContractError(
                "manual transitions must be non-empty and confined to root-reachable support layers",
            ));
        }
        Ok(Self {
            root_reachable_layers,
            distributions,
        })
    }

    pub(crate) const fn support_graph_digest(&self) -> &Sha256Digest {
        self.root_reachable_layers.support_graph_digest()
    }

    pub(crate) const fn distributions(&self) -> &SupportRecoveryDistributionSet {
        &self.distributions
    }

    pub(crate) fn covers_transitions(&self, transitions: &SupportTransitions) -> bool {
        !transitions.is_empty()
            && transitions.as_slice().iter().all(|transition| {
                self.root_reachable_layers
                    .as_slice()
                    .binary_search(transition.layer_id())
                    .is_ok()
            })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SupportRecoveryHandoffRevalidationDigestRecord {
    handoff_id: UnicaId,
    retention_lease_id: UnicaId,
    expected_artifact_sha256: Sha256Digest,
    observed_artifact_sha256: Sha256Digest,
    retention_lease_still_held: TrueLiteral,
    readable_by_manual_actor: TrueLiteral,
    revalidation_receipt_id: UnicaId,
    manual_readability_capability_row_id: CapabilityRowId,
    retention_capability_row_id: CapabilityRowId,
}

impl contract_digest_record_sealed::Sealed for SupportRecoveryHandoffRevalidationDigestRecord {}
impl ContractDigestRecord for SupportRecoveryHandoffRevalidationDigestRecord {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SupportRecoveryHandoffRevalidation {
    handoff_id: UnicaId,
    retention_lease_id: UnicaId,
    expected_artifact_sha256: Sha256Digest,
    observed_artifact_sha256: Sha256Digest,
    retention_lease_still_held: TrueLiteral,
    readable_by_manual_actor: TrueLiteral,
    revalidation_receipt_id: UnicaId,
    manual_readability_capability_row_id: CapabilityRowId,
    retention_capability_row_id: CapabilityRowId,
    revalidation_digest: Sha256Digest,
}

impl SupportRecoveryHandoffRevalidation {
    pub(crate) fn new(
        handoff: &SupportRecoveryDistributionHandoff,
        observed_artifact_sha256: Sha256Digest,
        revalidation_receipt_id: UnicaId,
        manual_readability_capability_row_id: CapabilityRowId,
        retention_capability_row_id: CapabilityRowId,
    ) -> Result<Self, SupportContractError> {
        if &observed_artifact_sha256 != handoff.artifact_sha256()
            || &manual_readability_capability_row_id
                != handoff.manual_readability_capability_row_id()
            || &retention_capability_row_id != handoff.retention_capability_row_id()
        {
            return Err(SupportContractError(
                "handoff revalidation disagrees with the frozen handoff",
            ));
        }
        let record = SupportRecoveryHandoffRevalidationDigestRecord {
            handoff_id: handoff.handoff_id().clone(),
            retention_lease_id: handoff.retention_lease_id().clone(),
            expected_artifact_sha256: handoff.artifact_sha256().clone(),
            observed_artifact_sha256,
            retention_lease_still_held: TrueLiteral,
            readable_by_manual_actor: TrueLiteral,
            revalidation_receipt_id,
            manual_readability_capability_row_id,
            retention_capability_row_id,
        };
        let revalidation_digest = contract_digest(&record, "handoff revalidation digest failed")?;
        Ok(Self {
            handoff_id: record.handoff_id,
            retention_lease_id: record.retention_lease_id,
            expected_artifact_sha256: record.expected_artifact_sha256,
            observed_artifact_sha256: record.observed_artifact_sha256,
            retention_lease_still_held: record.retention_lease_still_held,
            readable_by_manual_actor: record.readable_by_manual_actor,
            revalidation_receipt_id: record.revalidation_receipt_id,
            manual_readability_capability_row_id: record.manual_readability_capability_row_id,
            retention_capability_row_id: record.retention_capability_row_id,
            revalidation_digest,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SupportActionArmingReceiptDigestRecord {
    arming_receipt_id: UnicaId,
    support_action_id: UnicaId,
    support_action_digest: Sha256Digest,
    expected_before_history_cursor: RepositoryHistoryCursor,
    arming_cursor: RepositoryHistoryCursor,
    history_partition: ValidatedRepositoryHistoryPartition,
    support_gate_digest: Sha256Digest,
    candidate_set_digest: Sha256Digest,
    expected_relevant_baseline_digest: Sha256Digest,
    support_graph_digest: Sha256Digest,
    support_recovery_distribution_set_digest: Sha256Digest,
    original_fingerprint: Sha256Digest,
    manual_target_mode: ManualSupportTargetMode,
    root_lock_observation: SupportRootLockObservation,
    root_held_by_manual_actor: TrueLiteral,
    authorized_version_must_be_first_root_support_after_cursor: TrueLiteral,
}

impl contract_digest_record_sealed::Sealed for SupportActionArmingReceiptDigestRecord {}
impl ContractDigestRecord for SupportActionArmingReceiptDigestRecord {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SupportActionArmingReceipt {
    arming_receipt_id: UnicaId,
    support_action_id: UnicaId,
    support_action_digest: Sha256Digest,
    expected_before_history_cursor: RepositoryHistoryCursor,
    arming_cursor: RepositoryHistoryCursor,
    history_partition: ValidatedRepositoryHistoryPartition,
    support_gate_digest: Sha256Digest,
    candidate_set_digest: Sha256Digest,
    expected_relevant_baseline_digest: Sha256Digest,
    support_graph_digest: Sha256Digest,
    support_recovery_distribution_set_digest: Sha256Digest,
    original_fingerprint: Sha256Digest,
    manual_target_mode: ManualSupportTargetMode,
    root_lock_observation: SupportRootLockObservation,
    root_held_by_manual_actor: TrueLiteral,
    authorized_version_must_be_first_root_support_after_cursor: TrueLiteral,
    receipt_digest: Sha256Digest,
}

impl SupportActionArmingReceipt {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        arming_receipt_id: UnicaId,
        support_action_id: UnicaId,
        support_action_digest: Sha256Digest,
        expected_before_history_cursor: RepositoryHistoryCursor,
        arming_cursor: RepositoryHistoryCursor,
        history_partition: ValidatedRepositoryHistoryPartition,
        support_gate_digest: Sha256Digest,
        candidate_set_digest: Sha256Digest,
        expected_relevant_baseline_digest: Sha256Digest,
        support_graph_digest: Sha256Digest,
        support_recovery_distribution_set_digest: Sha256Digest,
        original_fingerprint: Sha256Digest,
        manual_target_mode: ManualSupportTargetMode,
        root_lock_observation: SupportRootLockObservation,
        expected_manual_actor_owner: &RepositoryOwnerIdentity,
    ) -> Result<Self, SupportContractError> {
        if history_partition.start_cursor() != &expected_before_history_cursor
            || history_partition.through_inclusive() != &arming_cursor
            || !history_partition.all_entries_are_one_of(&[
                RepositoryHistoryPartitionClassification::UnrelatedRoutine,
            ])
        {
            return Err(SupportContractError(
                "arming receipt history must be the exact all-unrelated prefix",
            ));
        }
        if root_lock_observation.owner().as_ref() != Some(expected_manual_actor_owner) {
            return Err(SupportContractError(
                "arming root observation does not name the bound manual actor",
            ));
        }
        let record = SupportActionArmingReceiptDigestRecord {
            arming_receipt_id,
            support_action_id,
            support_action_digest,
            expected_before_history_cursor,
            arming_cursor,
            history_partition,
            support_gate_digest,
            candidate_set_digest,
            expected_relevant_baseline_digest,
            support_graph_digest,
            support_recovery_distribution_set_digest,
            original_fingerprint,
            manual_target_mode,
            root_lock_observation,
            root_held_by_manual_actor: TrueLiteral,
            authorized_version_must_be_first_root_support_after_cursor: TrueLiteral,
        };
        let receipt_digest = contract_digest(&record, "support arming-receipt digest failed")?;
        Ok(Self {
            arming_receipt_id: record.arming_receipt_id,
            support_action_id: record.support_action_id,
            support_action_digest: record.support_action_digest,
            expected_before_history_cursor: record.expected_before_history_cursor,
            arming_cursor: record.arming_cursor,
            history_partition: record.history_partition,
            support_gate_digest: record.support_gate_digest,
            candidate_set_digest: record.candidate_set_digest,
            expected_relevant_baseline_digest: record.expected_relevant_baseline_digest,
            support_graph_digest: record.support_graph_digest,
            support_recovery_distribution_set_digest: record
                .support_recovery_distribution_set_digest,
            original_fingerprint: record.original_fingerprint,
            manual_target_mode: record.manual_target_mode,
            root_lock_observation: record.root_lock_observation,
            root_held_by_manual_actor: record.root_held_by_manual_actor,
            authorized_version_must_be_first_root_support_after_cursor: record
                .authorized_version_must_be_first_root_support_after_cursor,
            receipt_digest,
        })
    }

    pub(crate) const fn arming_receipt_id(&self) -> &UnicaId {
        &self.arming_receipt_id
    }

    pub(crate) const fn support_action_id(&self) -> &UnicaId {
        &self.support_action_id
    }

    pub(crate) const fn support_action_digest(&self) -> &Sha256Digest {
        &self.support_action_digest
    }

    pub(crate) const fn arming_cursor(&self) -> &RepositoryHistoryCursor {
        &self.arming_cursor
    }

    pub(crate) const fn receipt_digest(&self) -> &Sha256Digest {
        &self.receipt_digest
    }

    pub(crate) const fn expected_before_history_cursor(&self) -> &RepositoryHistoryCursor {
        &self.expected_before_history_cursor
    }

    pub(crate) const fn support_gate_digest(&self) -> &Sha256Digest {
        &self.support_gate_digest
    }

    pub(crate) const fn candidate_set_digest(&self) -> &Sha256Digest {
        &self.candidate_set_digest
    }

    pub(crate) const fn expected_relevant_baseline_digest(&self) -> &Sha256Digest {
        &self.expected_relevant_baseline_digest
    }

    pub(crate) const fn support_graph_digest(&self) -> &Sha256Digest {
        &self.support_graph_digest
    }

    pub(crate) const fn support_recovery_distribution_set_digest(&self) -> &Sha256Digest {
        &self.support_recovery_distribution_set_digest
    }

    pub(crate) const fn original_fingerprint(&self) -> &Sha256Digest {
        &self.original_fingerprint
    }

    pub(crate) const fn manual_target_mode(&self) -> ManualSupportTargetMode {
        self.manual_target_mode
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct SupportArmStaleKinds(Vec<SupportArmStaleKind>);

impl SupportArmStaleKinds {
    pub(crate) fn new(values: Vec<SupportArmStaleKind>) -> Result<Self, SupportContractError> {
        if values.is_empty() || values.len() > SupportArmStaleKind::ALL.len() {
            return Err(SupportContractError(
                "support-arm stale kinds must be non-empty and bounded",
            ));
        }
        if values.windows(2).any(|pair| pair[0] >= pair[1]) {
            return Err(SupportContractError(
                "support-arm stale kinds must be unique and in declaration order",
            ));
        }
        Ok(Self(values))
    }

    pub(crate) fn as_slice(&self) -> &[SupportArmStaleKind] {
        &self.0
    }
}

impl<'de> Deserialize<'de> for SupportArmStaleKinds {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::new(Vec::<SupportArmStaleKind>::deserialize(deserializer)?).map_err(D::Error::custom)
    }
}

impl JsonSchema for SupportArmStaleKinds {
    fn schema_name() -> Cow<'static, str> {
        "SupportArmStaleKinds".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "array",
            "minItems": 1,
            "maxItems": SupportArmStaleKind::ALL.len(),
            "uniqueItems": true,
            "items": generator.subschema_for::<SupportArmStaleKind>(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SupportArmStaleEvidenceDigestRecord {
    expected_before_history_cursor: RepositoryHistoryCursor,
    observed_history_cursor: RepositoryHistoryCursor,
    history_partition: ValidatedRepositoryHistoryPartition,
    mismatch_kinds: SupportArmStaleKinds,
    expected_support_gate_digest: Sha256Digest,
    observed_support_gate_digest: Sha256Digest,
    expected_relevant_baseline_digest: Sha256Digest,
    observed_relevant_baseline_digest: Sha256Digest,
    expected_support_graph_digest: Sha256Digest,
    observed_support_graph_digest: Sha256Digest,
    expected_recovery_distribution_set_digest: Sha256Digest,
    observed_recovery_distribution_set_digest: Sha256Digest,
    expected_original_fingerprint: Sha256Digest,
    observed_original_fingerprint: Sha256Digest,
    observed_root_lock: SupportRootLockObservation,
}

impl contract_digest_record_sealed::Sealed for SupportArmStaleEvidenceDigestRecord {}
impl ContractDigestRecord for SupportArmStaleEvidenceDigestRecord {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SupportArmStaleEvidence {
    expected_before_history_cursor: RepositoryHistoryCursor,
    observed_history_cursor: RepositoryHistoryCursor,
    history_partition: ValidatedRepositoryHistoryPartition,
    mismatch_kinds: SupportArmStaleKinds,
    expected_support_gate_digest: Sha256Digest,
    observed_support_gate_digest: Sha256Digest,
    expected_relevant_baseline_digest: Sha256Digest,
    observed_relevant_baseline_digest: Sha256Digest,
    expected_support_graph_digest: Sha256Digest,
    observed_support_graph_digest: Sha256Digest,
    expected_recovery_distribution_set_digest: Sha256Digest,
    observed_recovery_distribution_set_digest: Sha256Digest,
    expected_original_fingerprint: Sha256Digest,
    observed_original_fingerprint: Sha256Digest,
    observed_root_lock: SupportRootLockObservation,
    evidence_digest: Sha256Digest,
}

impl SupportArmStaleEvidence {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        expected_before_history_cursor: RepositoryHistoryCursor,
        observed_history_cursor: RepositoryHistoryCursor,
        history_partition: ValidatedRepositoryHistoryPartition,
        expected_support_gate_digest: Sha256Digest,
        observed_support_gate_digest: Sha256Digest,
        expected_relevant_baseline_digest: Sha256Digest,
        observed_relevant_baseline_digest: Sha256Digest,
        expected_support_graph_digest: Sha256Digest,
        observed_support_graph_digest: Sha256Digest,
        expected_recovery_distribution_set_digest: Sha256Digest,
        observed_recovery_distribution_set_digest: Sha256Digest,
        expected_original_fingerprint: Sha256Digest,
        observed_original_fingerprint: Sha256Digest,
        observed_root_lock: SupportRootLockObservation,
    ) -> Result<Self, SupportContractError> {
        if history_partition.start_cursor() != &expected_before_history_cursor
            || history_partition.through_inclusive() != &observed_history_cursor
        {
            return Err(SupportContractError(
                "stale-arm history partition endpoints do not match its cursors",
            ));
        }
        let classifications: Vec<_> = history_partition.classifications().collect();
        let history_changed = classifications.iter().any(|classification| {
            *classification != RepositoryHistoryPartitionClassification::UnrelatedRoutine
        });
        let relevant_history_changed = classifications.iter().any(|classification| {
            matches!(
                classification,
                RepositoryHistoryPartitionClassification::RelevantRoutine
                    | RepositoryHistoryPartitionClassification::AuthorizedSupport
                    | RepositoryHistoryPartitionClassification::ExternalSupport
                    | RepositoryHistoryPartitionClassification::PreArmExternal
                    | RepositoryHistoryPartitionClassification::Invalid
                    | RepositoryHistoryPartitionClassification::Corrective
            )
        });
        let mut kinds = Vec::new();
        if history_changed {
            kinds.push(SupportArmStaleKind::HistoryChanged);
        }
        if expected_support_gate_digest != observed_support_gate_digest {
            kinds.push(SupportArmStaleKind::SupportGateChanged);
        }
        if expected_relevant_baseline_digest != observed_relevant_baseline_digest
            || relevant_history_changed
        {
            kinds.push(SupportArmStaleKind::RelevantBaselineChanged);
        }
        if expected_support_graph_digest != observed_support_graph_digest {
            kinds.push(SupportArmStaleKind::SupportGraphChanged);
        }
        if expected_recovery_distribution_set_digest != observed_recovery_distribution_set_digest {
            kinds.push(SupportArmStaleKind::RecoveryDistributionSetChanged);
        }
        if expected_original_fingerprint != observed_original_fingerprint {
            kinds.push(SupportArmStaleKind::OriginalFingerprintChanged);
        }
        let mismatch_kinds = SupportArmStaleKinds::new(kinds)?;
        let record = SupportArmStaleEvidenceDigestRecord {
            expected_before_history_cursor,
            observed_history_cursor,
            history_partition,
            mismatch_kinds,
            expected_support_gate_digest,
            observed_support_gate_digest,
            expected_relevant_baseline_digest,
            observed_relevant_baseline_digest,
            expected_support_graph_digest,
            observed_support_graph_digest,
            expected_recovery_distribution_set_digest,
            observed_recovery_distribution_set_digest,
            expected_original_fingerprint,
            observed_original_fingerprint,
            observed_root_lock,
        };
        let evidence_digest = contract_digest(&record, "support-arm stale digest failed")?;
        Ok(Self {
            expected_before_history_cursor: record.expected_before_history_cursor,
            observed_history_cursor: record.observed_history_cursor,
            history_partition: record.history_partition,
            mismatch_kinds: record.mismatch_kinds,
            expected_support_gate_digest: record.expected_support_gate_digest,
            observed_support_gate_digest: record.observed_support_gate_digest,
            expected_relevant_baseline_digest: record.expected_relevant_baseline_digest,
            observed_relevant_baseline_digest: record.observed_relevant_baseline_digest,
            expected_support_graph_digest: record.expected_support_graph_digest,
            observed_support_graph_digest: record.observed_support_graph_digest,
            expected_recovery_distribution_set_digest: record
                .expected_recovery_distribution_set_digest,
            observed_recovery_distribution_set_digest: record
                .observed_recovery_distribution_set_digest,
            expected_original_fingerprint: record.expected_original_fingerprint,
            observed_original_fingerprint: record.observed_original_fingerprint,
            observed_root_lock: record.observed_root_lock,
            evidence_digest,
        })
    }
}
