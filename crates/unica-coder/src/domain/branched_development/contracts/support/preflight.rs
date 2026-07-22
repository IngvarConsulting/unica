#[cfg(test)]
mod tests {
    use super::super::authorization::{
        SupportActionAuthorizationInputs, SupportActionPhaseBinding,
    };
    use super::super::evidence::{
        SupportRecoveryDistributionHandoff, SupportRecoveryDistributionHandoffInputs,
        UserVisibleCfFileName,
    };
    use super::super::model::{
        ManualSupportTargetMode, RepositoryAction, RootReachableSupportLayerSet, SupportCandidate,
        SupportCandidateEvidenceAuthority, SupportHistoryOrderAuthority, SupportTransition,
    };
    use super::*;
    use crate::domain::branched_development::canonical_json::{
        canonical_contract_digest, contract_digest_record_sealed, ContractDigestRecord,
    };
    use crate::domain::branched_development::contracts::repository::lifecycle::SupportGateRelevantBaselineResolver;
    use crate::domain::branched_development::contracts::repository::{
        EvidenceSourceIndex, EvidenceSourceIndexCandidate, EvidenceSourceRegistry,
        RepositoryContractError, RepositoryHistoryEvidenceBytesResolver,
        RepositoryHistoryOrderEvidence, RepositoryHistoryOrderResolver,
        RepositoryHistoryPartitionResolver, RepositoryHistorySourceEvidenceRef,
        SupportGateRelevantBaselineAuthority, UnvalidatedRepositoryHistoryPartition,
        ValidatedRepositoryHistoryPartition,
    };
    use crate::domain::branched_development::contracts::scalars::{
        DisplayPath, RepositoryTargetDisplay, RepositoryUsername,
    };
    use crate::domain::branched_development::contracts::schema::audit_json_schema;
    use crate::domain::branched_development::{
        MetadataObjectId, ProfileArtifactRefId, SupportLayerId,
    };
    use schemars::{schema_for, JsonSchema};
    use serde::de::DeserializeOwned;
    use serde::Serialize;
    use serde_json::{json, Value};
    use std::cmp::Ordering;

    const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const ID_1: &str = "11111111-1111-4111-8111-111111111111";
    const ID_2: &str = "22222222-2222-4222-8222-222222222222";
    const OBJECT: &str = "33333333-3333-4333-8333-333333333333";

    fn digest() -> Sha256Digest {
        Sha256Digest::parse(A).unwrap()
    }

    fn id(value: &str) -> UnicaId {
        UnicaId::parse(value).unwrap()
    }

    fn cursor() -> RepositoryHistoryCursor {
        serde_json::from_value(json!({
            "throughVersion": "v1",
            "historyPrefixDigest": A,
        }))
        .unwrap()
    }

    fn assert_closed<T: JsonSchema>() {
        let schema = serde_json::to_value(schema_for!(T)).unwrap();
        audit_json_schema(&schema).expect("support preflight schema must be recursively closed");
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

    assert_not_deserialize_owned!(SupportPreflightData);
    assert_not_deserialize_owned!(SupportManualPreflightAuthority);

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

    struct StableBaseline;

    impl SupportGateRelevantBaselineResolver for StableBaseline {
        fn recompute_relevant_baseline_digest(
            &self,
            _partition: &ValidatedRepositoryHistoryPartition,
            current_gate_relevant_baseline_digest: &Sha256Digest,
        ) -> Result<Sha256Digest, RepositoryContractError> {
            Ok(current_gate_relevant_baseline_digest.clone())
        }
    }

    struct NoHistoryOrder;

    impl SupportHistoryOrderAuthority for NoHistoryOrder {
        fn compare_versions(
            &self,
            _left: &crate::domain::branched_development::contracts::scalars::RepositoryVersion,
            _right: &crate::domain::branched_development::contracts::scalars::RepositoryVersion,
        ) -> Result<Ordering, SupportContractError> {
            panic!("empty collections must not compare repository versions")
        }

        fn compare_cursors(
            &self,
            _left: &RepositoryHistoryCursor,
            _right: &RepositoryHistoryCursor,
        ) -> Result<Ordering, SupportContractError> {
            panic!("empty collections must not compare repository cursors")
        }
    }

    fn history_evidence() -> SupportGateHistoryEvidence {
        let gate_cursor = cursor();
        let partition_digest = canonical_contract_digest(
            &EmptyPartitionDigestRecord {
                from_exclusive: gate_cursor.clone(),
                through_inclusive: gate_cursor.clone(),
                entries: Vec::new(),
            },
            None,
        )
        .unwrap();
        let wire = serde_json::from_value::<UnvalidatedRepositoryHistoryPartition>(json!({
            "fromExclusive": gate_cursor,
            "throughInclusive": cursor(),
            "entries": [],
            "partitionDigest": partition_digest,
        }))
        .unwrap();
        let registry = EvidenceSourceRegistry::task8().unwrap();
        let partition = RepositoryHistoryPartitionResolver::new(
            &registry,
            &UnexpectedIndex,
            &UnexpectedOrder,
            &UnexpectedBytes,
        )
        .validate(wire)
        .unwrap();
        let authority =
            SupportGateRelevantBaselineAuthority::resolve(&partition, digest(), &StableBaseline)
                .unwrap();
        SupportGateHistoryEvidence::new(partition, &authority).unwrap()
    }

    fn sources(support_graph_digest: Sha256Digest) -> SupportGateSourceEvidence {
        SupportGateSourceEvidence::from_capability_adapter(
            digest(),
            id(ID_1),
            digest(),
            id(ID_2),
            support_graph_digest,
            digest(),
            digest(),
            CapabilityRowId::parse("support-preflight.v1").unwrap(),
            digest(),
            digest(),
        )
    }

    fn empty_inputs() -> SupportPreflightInputs {
        SupportPreflightInputs::new(
            id(ID_1),
            SupportCandidateSet::new(id(ID_2), SupportCandidates::new(Vec::new()).unwrap())
                .unwrap(),
            SupportBlockers::new(Vec::new()).unwrap(),
            SupportEvidenceGaps::new(Vec::new(), &NoHistoryOrder).unwrap(),
            SupportTransitions::new(Vec::new()).unwrap(),
            SupportTransitions::new(Vec::new()).unwrap(),
            SupportRecoveryDistributionSet::new(Vec::new()).unwrap(),
            sources(digest()),
            history_evidence(),
        )
    }

    pub(crate) fn ready_preflight_authority_fixture_test_only() -> ReadySupportPreflightAuthority {
        ReadySupportPreflightAuthority::try_from(
            SupportPreflightData::ready(empty_inputs()).unwrap(),
        )
        .unwrap()
    }

    fn candidate_with_restriction(
        layer_id: &SupportLayerId,
        vendor_restriction: VendorChangeRestriction,
    ) -> SupportCandidate {
        let object_id = MetadataObjectId::parse(OBJECT).unwrap();
        let evidence = SupportCandidateEvidenceAuthority::from_capability_adapter(
            object_id.clone(),
            Some(layer_id.clone()),
            Some(digest()),
            None,
            None,
            None,
            None,
        )
        .unwrap();
        SupportCandidate::from_evidence_authority(
            object_id,
            RepositoryTargetDisplay::parse("Catalog.A").unwrap(),
            Some(layer_id.clone()),
            RepositoryAction::Modify,
            SupportCurrentState::Locked,
            vendor_restriction,
            SupportRequiredState::Editable,
            &evidence,
        )
        .unwrap()
    }

    fn manual_candidate(layer_id: &SupportLayerId) -> SupportCandidate {
        candidate_with_restriction(layer_id, VendorChangeRestriction::ChangesAllowed)
    }

    fn recovery_gap(layer_id: &SupportLayerId) -> SupportEvidenceGap {
        serde_json::from_value(json!({
            "gapKind": "supportLayerRecoveryEvidence",
            "layerId": layer_id,
            "missingEvidenceKind": "recoveryArtifactMissing",
            "diagnostic": "redacted",
        }))
        .unwrap()
    }

    fn manual_parts() -> (
        SupportPreflightInputs,
        SupportRecoveryDistributionCoverageAuthority,
    ) {
        let layer_id = SupportLayerId::parse("layer-a").unwrap();
        let transitions =
            SupportTransitions::new(vec![SupportTransition::enable_configuration_changes(
                RepositoryTargetDisplay::parse("Configuration").unwrap(),
                layer_id.clone(),
            )])
            .unwrap();
        let handoff = SupportRecoveryDistributionHandoff::new(
            ManualSupportTargetMode::ReservedOriginal,
            None,
            SupportRecoveryDistributionHandoffInputs {
                handoff_id: id(ID_1),
                profile_artifact_ref_id: ProfileArtifactRefId::parse("vendor.layer-a").unwrap(),
                profile_artifact_display: DisplayPath::parse("Vendor layer A").unwrap(),
                user_visible_file_name: UserVisibleCfFileName::parse("vendor-layer-a.cf").unwrap(),
                manual_actor_username: RepositoryUsername::parse("reserved-user").unwrap(),
                layer_id: layer_id.clone(),
                distribution_artifact_id: id(ID_2),
                artifact_sha256: digest(),
                readability_probe_receipt_id: id(ID_1),
                manual_readability_capability_row_id: CapabilityRowId::parse(
                    "manual-readability.v1",
                )
                .unwrap(),
                retention_lease_id: id(ID_1),
                retention_receipt_id: id(ID_2),
                retention_capability_row_id: CapabilityRowId::parse("retention-provider.v1")
                    .unwrap(),
            },
        )
        .unwrap();
        let recovery = SupportRecoveryDistributionEvidence::new(
            layer_id.clone(),
            id(ID_2),
            digest(),
            digest(),
            CapabilityRowId::parse("support-recovery.v1").unwrap(),
            handoff,
        )
        .unwrap();
        let recovery_set = SupportRecoveryDistributionSet::new(vec![recovery]).unwrap();
        let reachable =
            RootReachableSupportLayerSet::from_capability_adapter(vec![layer_id.clone()], digest())
                .unwrap();
        let coverage = SupportRecoveryDistributionCoverageAuthority::prove_complete(
            reachable,
            recovery_set.clone(),
            &transitions,
        )
        .unwrap();
        let inputs = SupportPreflightInputs::new(
            id(ID_1),
            SupportCandidateSet::new(
                id(ID_2),
                SupportCandidates::new(vec![manual_candidate(&layer_id)]).unwrap(),
            )
            .unwrap(),
            SupportBlockers::new(Vec::new()).unwrap(),
            SupportEvidenceGaps::new(Vec::new(), &NoHistoryOrder).unwrap(),
            transitions,
            SupportTransitions::new(Vec::new()).unwrap(),
            recovery_set,
            sources(digest()),
            history_evidence(),
        );
        (inputs, coverage)
    }

    fn schema_accepts<T: JsonSchema>(value: &Value) -> bool {
        let schema = serde_json::to_value(schema_for!(T)).unwrap();
        jsonschema::options()
            .with_draft(jsonschema::Draft::Draft202012)
            .build(&schema)
            .unwrap()
            .is_valid(value)
    }

    #[test]
    fn preflight_and_gate_digest_schemas_are_closed() {
        assert_closed::<SupportGateDigestRecord>();
        assert_closed::<SupportPreflightData>();

        let gate_schema = serde_json::to_value(schema_for!(SupportGateDigestRecord)).unwrap();
        let properties = gate_schema["properties"].as_object().unwrap();
        for excluded in [
            "observedHistoryCursor",
            "historyEvidence",
            "supportGateDigest",
            "supportActionId",
            "supportActionDigest",
        ] {
            assert!(!properties.contains_key(excluded));
        }
    }

    #[test]
    fn ready_preflight_has_no_action_projection_and_rejects_splice() {
        let ready = SupportPreflightData::ready(empty_inputs()).unwrap();
        let authority = ReadySupportPreflightAuthority::try_from(ready.clone()).unwrap();
        assert_eq!(authority.support_gate_id(), ready.support_gate_id());
        assert_eq!(authority.support_gate_digest(), ready.support_gate_digest());
        assert_eq!(
            authority.history_evidence_digest(),
            ready.history_evidence().evidence_digest(),
        );
        let encoded = serde_json::to_value(&ready).unwrap();
        assert_eq!(encoded["outcome"], json!("ready"));
        assert!(encoded.get("supportActionId").is_none());
        assert!(schema_accepts::<SupportPreflightData>(&encoded));

        let mut spliced = encoded;
        spliced["supportActionId"] = json!(ID_1);
        spliced["supportActionDigest"] = json!(A);
        assert!(!schema_accepts::<SupportPreflightData>(&spliced));
    }

    #[test]
    fn manual_preflight_binds_exact_action_and_requires_both_projection_fields() {
        let (inputs, coverage) = manual_parts();
        let gate = SupportManualPreflightAuthority::new(inputs, coverage).unwrap();
        let action_inputs = SupportActionAuthorizationInputs::fixture(
            id(ID_2),
            super::super::model::SupportActionPurpose::MainIntegrationPrerequisite,
            gate.support_gate_id().clone(),
            gate.support_gate_digest().clone(),
            gate.candidate_set_digest().clone(),
            gate.observed_history_cursor().clone(),
            gate.relevant_baseline_digest().clone(),
            gate.support_graph_digest().clone(),
            gate.required_transitions().clone(),
            gate.recovery_coverage().clone(),
            RepositoryUsername::parse("reserved-user").unwrap(),
            digest(),
            gate.original_fingerprint().clone(),
            RepositoryUsername::parse("reserved-user").unwrap(),
            None,
            SupportActionPhaseBinding::main_integration(digest()),
        );
        let action = SupportActionAuthorizationAuthority::reserved_original(
            action_inputs,
            CapabilityRowId::parse("reserved-original-lease.v1").unwrap(),
            digest(),
        )
        .unwrap();
        let manual = gate.publish(&action).unwrap();
        assert!(ReadySupportPreflightAuthority::try_from(manual.clone()).is_err());
        let encoded = serde_json::to_value(&manual).unwrap();
        assert_eq!(encoded["outcome"], json!("manualSupportRequired"));
        assert_eq!(encoded["supportActionId"], json!(ID_2));
        assert!(schema_accepts::<SupportPreflightData>(&encoded));

        let mut missing_pair = encoded;
        missing_pair
            .as_object_mut()
            .unwrap()
            .remove("supportActionDigest");
        assert!(!schema_accepts::<SupportPreflightData>(&missing_pair));

        let mut empty_transitions = serde_json::to_value(&manual).unwrap();
        empty_transitions["requiredTransitions"] = json!([]);
        assert!(!schema_accepts::<SupportPreflightData>(&empty_transitions));

        let mut empty_recovery = serde_json::to_value(&manual).unwrap();
        empty_recovery["supportRecoveryDistributions"] = json!([]);
        assert!(!schema_accepts::<SupportPreflightData>(&empty_recovery));
    }

    #[test]
    fn deterministic_outcomes_reject_more_permissive_shape() {
        let mut ready_inputs = empty_inputs();
        ready_inputs.required_transitions =
            SupportTransitions::new(vec![SupportTransition::enable_configuration_changes(
                RepositoryTargetDisplay::parse("Configuration").unwrap(),
                SupportLayerId::parse("layer-a").unwrap(),
            )])
            .unwrap();
        assert!(SupportPreflightData::ready(ready_inputs).is_err());

        assert!(SupportPreflightData::inconclusive(empty_inputs(), None).is_err());
        assert!(SupportPreflightData::vendor_forbids_changes(empty_inputs()).is_err());

        let ready =
            serde_json::to_value(SupportPreflightData::ready(empty_inputs()).unwrap()).unwrap();
        let mut forged_vendor = ready.clone();
        forged_vendor["outcome"] = json!("vendorForbidsChanges");
        assert!(!schema_accepts::<SupportPreflightData>(&forged_vendor));
        let mut forged_inconclusive = ready;
        forged_inconclusive["outcome"] = json!("supportPreflightInconclusive");
        assert!(!schema_accepts::<SupportPreflightData>(
            &forged_inconclusive
        ));
    }

    #[test]
    fn inconclusive_manual_path_requires_exact_successful_subset_and_gap_complement() {
        let (mut inputs, _) = manual_parts();
        let layer_a = SupportLayerId::parse("layer-a").unwrap();
        let layer_b = SupportLayerId::parse("layer-b").unwrap();
        let gaps = SupportEvidenceGaps::new(vec![recovery_gap(&layer_b)], &NoHistoryOrder).unwrap();
        inputs.evidence_gaps = gaps.clone();
        let reachable = RootReachableSupportLayerSet::from_capability_adapter(
            vec![layer_a.clone(), layer_b.clone()],
            digest(),
        )
        .unwrap();
        let authority = SupportInconclusiveRecoveryCoverageAuthority::prove_exact_subset(
            reachable,
            inputs.support_recovery_distributions.clone(),
            inputs.required_transitions.clone(),
            gaps,
        )
        .unwrap();
        let inconclusive =
            SupportPreflightData::inconclusive(inputs.clone(), Some(&authority)).unwrap();
        let encoded = serde_json::to_value(inconclusive).unwrap();
        assert_eq!(encoded["outcome"], json!("supportPreflightInconclusive"));
        assert!(schema_accepts::<SupportPreflightData>(&encoded));

        let wrong_gaps =
            SupportEvidenceGaps::new(vec![recovery_gap(&layer_a)], &NoHistoryOrder).unwrap();
        let reachable =
            RootReachableSupportLayerSet::from_capability_adapter(vec![layer_a, layer_b], digest())
                .unwrap();
        assert!(
            SupportInconclusiveRecoveryCoverageAuthority::prove_exact_subset(
                reachable,
                inputs.support_recovery_distributions,
                inputs.required_transitions,
                wrong_gaps,
            )
            .is_err()
        );
    }

    #[test]
    fn inconclusive_gaps_cannot_mask_ready_vendor_or_classification_precedence() {
        let layer_a = SupportLayerId::parse("layer-a").unwrap();
        let mut ready_inputs = empty_inputs();
        ready_inputs.evidence_gaps =
            SupportEvidenceGaps::new(vec![recovery_gap(&layer_a)], &NoHistoryOrder).unwrap();
        assert!(SupportPreflightData::inconclusive(ready_inputs, None).is_err());

        let (mut vendor_inputs, _) = manual_parts();
        vendor_inputs.candidate_set = SupportCandidateSet::new(
            id(ID_2),
            SupportCandidates::new(vec![candidate_with_restriction(
                &layer_a,
                VendorChangeRestriction::ChangesForbidden,
            )])
            .unwrap(),
        )
        .unwrap();
        let layer_b = SupportLayerId::parse("layer-b").unwrap();
        let gaps = SupportEvidenceGaps::new(vec![recovery_gap(&layer_b)], &NoHistoryOrder).unwrap();
        vendor_inputs.evidence_gaps = gaps.clone();
        let authority = SupportInconclusiveRecoveryCoverageAuthority::prove_exact_subset(
            RootReachableSupportLayerSet::from_capability_adapter(
                vec![layer_a.clone(), layer_b],
                digest(),
            )
            .unwrap(),
            vendor_inputs.support_recovery_distributions.clone(),
            vendor_inputs.required_transitions.clone(),
            gaps,
        )
        .unwrap();
        assert!(SupportPreflightData::inconclusive(vendor_inputs, Some(&authority)).is_err());

        let candidate_gap = serde_json::from_value::<SupportEvidenceGap>(json!({
            "gapKind": "candidateEvidence",
            "objectId": OBJECT,
            "objectDisplay": "Catalog.A",
            "missingEvidenceKind": "candidateClassificationUnavailable",
            "diagnostic": "redacted",
        }))
        .unwrap();
        let mut classification_inputs = empty_inputs();
        classification_inputs.evidence_gaps =
            SupportEvidenceGaps::new(vec![candidate_gap], &NoHistoryOrder).unwrap();
        let inconclusive = SupportPreflightData::inconclusive(classification_inputs, None).unwrap();
        assert_eq!(
            serde_json::to_value(inconclusive).unwrap()["outcome"],
            json!("supportPreflightInconclusive")
        );
    }
}

#[cfg(test)]
pub(crate) use tests::ready_preflight_authority_fixture_test_only;

use super::super::repository::{RepositoryHistoryCursor, SupportGateHistoryEvidence};
use super::super::schema::one_of_schema;
use super::authorization::SupportActionAuthorizationAuthority;
use super::evidence::{
    SupportInconclusiveRecoveryCoverageAuthority, SupportRecoveryDistributionCoverageAuthority,
    SupportRecoveryDistributionEvidence, SupportRecoveryDistributionSet,
};
use super::model::{
    SupportBlocker, SupportBlockerReason, SupportBlockers, SupportCandidateSet, SupportCandidates,
    SupportContractError, SupportCurrentState, SupportEvidenceGap, SupportEvidenceGaps,
    SupportGateInputDigests, SupportPreflightOutcome, SupportRequiredState, SupportTransition,
    SupportTransitions, VendorChangeRestriction,
};
use crate::domain::branched_development::canonical_json::{
    canonical_contract_digest, contract_digest_record_sealed, ContractDigestRecord,
};
use crate::domain::branched_development::{CapabilityRowId, Sha256Digest, UnicaId};
use schemars::{JsonSchema, Schema, SchemaGenerator};
use serde::ser::SerializeMap;
use serde::{Serialize, Serializer};
use serde_json::{Map, Value};
use std::borrow::Cow;

const MAX_RECOVERY_DISTRIBUTIONS: usize = 1_024;
const MAX_GENERAL_ITEMS: usize = 1_024;
const MAX_METADATA_ITEMS: usize = 100_000;

fn contract_digest<T: ContractDigestRecord>(
    record: &T,
    message: &'static str,
) -> Result<Sha256Digest, SupportContractError> {
    canonical_contract_digest(record, None).map_err(|_| SupportContractError(message))
}

/// Typed adapter output for every semantic source named by
/// `SupportGateInputDigests` but not carried as its own rich Task 8 record.
/// The preflight constructor derives `gateInputs`; callers never provide a
/// second, independently selectable digest projection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SupportGateSourceEvidence {
    canonical_delta_digest: Sha256Digest,
    ordinary_result_artifact_id: UnicaId,
    ordinary_result_digest: Sha256Digest,
    comparison_id: UnicaId,
    support_graph_digest: Sha256Digest,
    settings_digest: Sha256Digest,
    sandbox_result_digest: Sha256Digest,
    capability_row_id: CapabilityRowId,
    capability_row_digest: Sha256Digest,
    original_fingerprint: Sha256Digest,
}

impl SupportGateSourceEvidence {
    /// Fixture mint only. Production construction belongs to the task that
    /// binds the named rich canonical-delta/artifact/graph/sandbox/capability
    /// sources; raw digest callers must not mint semantic gate authority.
    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub(crate) const fn from_capability_adapter(
        canonical_delta_digest: Sha256Digest,
        ordinary_result_artifact_id: UnicaId,
        ordinary_result_digest: Sha256Digest,
        comparison_id: UnicaId,
        support_graph_digest: Sha256Digest,
        settings_digest: Sha256Digest,
        sandbox_result_digest: Sha256Digest,
        capability_row_id: CapabilityRowId,
        capability_row_digest: Sha256Digest,
        original_fingerprint: Sha256Digest,
    ) -> Self {
        Self {
            canonical_delta_digest,
            ordinary_result_artifact_id,
            ordinary_result_digest,
            comparison_id,
            support_graph_digest,
            settings_digest,
            sandbox_result_digest,
            capability_row_id,
            capability_row_digest,
            original_fingerprint,
        }
    }
}

/// Authority inputs shared by all four deterministic preflight outcomes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SupportPreflightInputs {
    support_gate_id: UnicaId,
    candidate_set: SupportCandidateSet,
    blockers: SupportBlockers,
    evidence_gaps: SupportEvidenceGaps,
    required_transitions: SupportTransitions,
    surplus_transitions: SupportTransitions,
    support_recovery_distributions: SupportRecoveryDistributionSet,
    sources: SupportGateSourceEvidence,
    history_evidence: SupportGateHistoryEvidence,
}

impl SupportPreflightInputs {
    #[allow(clippy::too_many_arguments)]
    pub(crate) const fn new(
        support_gate_id: UnicaId,
        candidate_set: SupportCandidateSet,
        blockers: SupportBlockers,
        evidence_gaps: SupportEvidenceGaps,
        required_transitions: SupportTransitions,
        surplus_transitions: SupportTransitions,
        support_recovery_distributions: SupportRecoveryDistributionSet,
        sources: SupportGateSourceEvidence,
        history_evidence: SupportGateHistoryEvidence,
    ) -> Self {
        Self {
            support_gate_id,
            candidate_set,
            blockers,
            evidence_gaps,
            required_transitions,
            surplus_transitions,
            support_recovery_distributions,
            sources,
            history_evidence,
        }
    }
}

/// Stable semantic preimage for the support gate.  Replaceable history
/// evidence/cursors, the gate digest itself, and action projection are
/// intentionally absent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SupportGateDigestRecord {
    support_gate_id: UnicaId,
    outcome: SupportPreflightOutcome,
    candidate_set_id: UnicaId,
    gate_inputs: SupportGateInputDigests,
    relevant_baseline_digest: Sha256Digest,
    ordinary_result_artifact_id: UnicaId,
    comparison_id: UnicaId,
    capability_row_id: CapabilityRowId,
    blockers: SupportBlockers,
    evidence_gaps: SupportEvidenceGaps,
    required_transitions: SupportTransitions,
    surplus_transitions: SupportTransitions,
}

impl contract_digest_record_sealed::Sealed for SupportGateDigestRecord {}
impl ContractDigestRecord for SupportGateDigestRecord {}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SupportPreflightRecord {
    support_gate_id: UnicaId,
    outcome: SupportPreflightOutcome,
    candidate_set_id: UnicaId,
    candidate_set_digest: Sha256Digest,
    gate_inputs: SupportGateInputDigests,
    candidates: SupportCandidates,
    blockers: SupportBlockers,
    evidence_gaps: SupportEvidenceGaps,
    support_graph_digest: Sha256Digest,
    required_transitions: SupportTransitions,
    surplus_transitions: SupportTransitions,
    observed_history_cursor: RepositoryHistoryCursor,
    relevant_baseline_digest: Sha256Digest,
    original_fingerprint: Sha256Digest,
    ordinary_result_artifact_id: UnicaId,
    comparison_id: UnicaId,
    settings_digest: Sha256Digest,
    sandbox_result_digest: Sha256Digest,
    support_recovery_distributions: Vec<SupportRecoveryDistributionEvidence>,
    support_recovery_distribution_set_digest: Sha256Digest,
    capability_row_id: CapabilityRowId,
    support_gate_digest: Sha256Digest,
    history_evidence: SupportGateHistoryEvidence,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SupportManualPreflightAuthority {
    record: SupportPreflightRecord,
    recovery_coverage: SupportRecoveryDistributionCoverageAuthority,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SupportPreflightData {
    record: SupportPreflightRecord,
    support_action_id: Option<UnicaId>,
    support_action_digest: Option<Sha256Digest>,
}

/// Consuming proof that a published support preflight is the exact `ready`
/// branch usable by main-integration preparation. It exposes only immutable
/// lineage needed to bind the nested merge session; callers cannot relabel a
/// non-ready preflight or supply those fields independently.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ReadySupportPreflightAuthority(SupportPreflightData);

impl TryFrom<SupportPreflightData> for ReadySupportPreflightAuthority {
    type Error = SupportContractError;

    fn try_from(value: SupportPreflightData) -> Result<Self, Self::Error> {
        if value.record.outcome != SupportPreflightOutcome::Ready
            || value.support_action_id.is_some()
            || value.support_action_digest.is_some()
        {
            return Err(SupportContractError(
                "main-integration preparation requires an action-free ready support preflight",
            ));
        }
        Ok(Self(value))
    }
}

impl ReadySupportPreflightAuthority {
    pub(crate) const fn support_gate_id(&self) -> &UnicaId {
        &self.0.record.support_gate_id
    }

    pub(crate) const fn support_gate_digest(&self) -> &Sha256Digest {
        &self.0.record.support_gate_digest
    }

    pub(crate) fn history_evidence_digest(&self) -> &Sha256Digest {
        self.0.record.history_evidence.evidence_digest()
    }

    pub(crate) const fn history_evidence(&self) -> &SupportGateHistoryEvidence {
        &self.0.record.history_evidence
    }

    pub(crate) const fn ordinary_result_artifact_id(&self) -> &UnicaId {
        &self.0.record.ordinary_result_artifact_id
    }

    pub(crate) const fn comparison_id(&self) -> &UnicaId {
        &self.0.record.comparison_id
    }

    pub(crate) const fn settings_digest(&self) -> &Sha256Digest {
        &self.0.record.settings_digest
    }

    pub(crate) const fn sandbox_result_digest(&self) -> &Sha256Digest {
        &self.0.record.sandbox_result_digest
    }

    pub(crate) const fn support_graph_digest(&self) -> &Sha256Digest {
        &self.0.record.support_graph_digest
    }

    pub(crate) const fn observed_history_cursor(&self) -> &RepositoryHistoryCursor {
        &self.0.record.observed_history_cursor
    }

    pub(crate) const fn relevant_baseline_digest(&self) -> &Sha256Digest {
        &self.0.record.relevant_baseline_digest
    }

    pub(crate) const fn original_fingerprint(&self) -> &Sha256Digest {
        &self.0.record.original_fingerprint
    }

    pub(crate) fn into_data(self) -> SupportPreflightData {
        self.0
    }
}

/// Exact semantic gate candidate submitted to authoritative task state.
///
/// The request is borrowed from a sealed ready authority: callers cannot
/// construct a current-state claim from IDs, digests, or a status DTO.
#[derive(Debug)]
pub(crate) struct CurrentReadySupportGateResolutionRequest<'a> {
    candidate: &'a ReadySupportPreflightAuthority,
}

impl CurrentReadySupportGateResolutionRequest<'_> {
    pub(crate) fn support_gate_id(&self) -> &UnicaId {
        self.candidate.support_gate_id()
    }

    pub(crate) fn support_gate_digest(&self) -> &Sha256Digest {
        self.candidate.support_gate_digest()
    }

    pub(crate) fn candidate_history_evidence(&self) -> &SupportGateHistoryEvidence {
        self.candidate.history_evidence()
    }

    pub(crate) fn ordinary_result_artifact_id(&self) -> &UnicaId {
        self.candidate.ordinary_result_artifact_id()
    }

    pub(crate) fn comparison_id(&self) -> &UnicaId {
        self.candidate.comparison_id()
    }

    pub(crate) fn settings_digest(&self) -> &Sha256Digest {
        self.candidate.settings_digest()
    }

    pub(crate) fn sandbox_result_digest(&self) -> &Sha256Digest {
        self.candidate.sandbox_result_digest()
    }

    pub(crate) fn support_graph_digest(&self) -> &Sha256Digest {
        self.candidate.support_graph_digest()
    }

    pub(crate) fn observed_history_cursor(&self) -> &RepositoryHistoryCursor {
        self.candidate.observed_history_cursor()
    }

    pub(crate) fn relevant_baseline_digest(&self) -> &Sha256Digest {
        self.candidate.relevant_baseline_digest()
    }

    pub(crate) fn original_fingerprint(&self) -> &Sha256Digest {
        self.candidate.original_fingerprint()
    }
}

/// One authoritative task-state read. Implementations must bind the request
/// to the latest non-invalidated preflight whose state is `current` and whose
/// semantic outcome is `ready`.
pub(crate) trait CurrentReadySupportGateStateLease {
    fn binds(&self, request: &CurrentReadySupportGateResolutionRequest<'_>) -> bool;

    fn persisted_history_evidence(&self) -> &SupportGateHistoryEvidence;

    fn current_state_revision(&self) -> &Sha256Digest;
}

/// Production port for resolving the current support gate from authoritative
/// task state. The adapter receives the complete typed semantic request and
/// returns a lease, never a caller-selected status projection.
pub(crate) trait CurrentReadySupportGateStateResolver {
    fn resolve_latest_non_invalidated_current_ready(
        &mut self,
        request: &CurrentReadySupportGateResolutionRequest<'_>,
    ) -> Result<Box<dyn CurrentReadySupportGateStateLease>, SupportContractError>;
}

/// Linear proof that this is the latest non-invalidated ready gate in the
/// authoritative `current` state at `current_state_revision`.
///
/// Deliberately non-wire and non-`Clone`: temporal authority cannot be decoded
/// from a resume handle or replayed by copying a digest projection.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct CurrentReadySupportGateAuthority {
    ready: ReadySupportPreflightAuthority,
    current_state_revision: Sha256Digest,
}

impl CurrentReadySupportGateAuthority {
    pub(crate) fn resolve(
        mut ready: ReadySupportPreflightAuthority,
        resolver: &mut dyn CurrentReadySupportGateStateResolver,
    ) -> Result<Self, SupportContractError> {
        let request = CurrentReadySupportGateResolutionRequest { candidate: &ready };
        let lease = resolver.resolve_latest_non_invalidated_current_ready(&request)?;
        if !lease.binds(&request) {
            return Err(SupportContractError(
                "task state did not resolve the exact latest non-invalidated current ready gate",
            ));
        }
        let persisted_history_evidence = lease.persisted_history_evidence().clone();
        let current_state_revision = lease.current_state_revision().clone();
        ready
            .0
            .replace_history_evidence(persisted_history_evidence)?;
        Ok(Self {
            ready,
            current_state_revision,
        })
    }

    pub(crate) fn support_gate_id(&self) -> &UnicaId {
        self.ready.support_gate_id()
    }

    pub(crate) fn support_gate_digest(&self) -> &Sha256Digest {
        self.ready.support_gate_digest()
    }

    pub(crate) fn history_evidence(&self) -> &SupportGateHistoryEvidence {
        self.ready.history_evidence()
    }

    pub(crate) fn history_evidence_digest(&self) -> &Sha256Digest {
        self.ready.history_evidence_digest()
    }

    pub(crate) fn ordinary_result_artifact_id(&self) -> &UnicaId {
        self.ready.ordinary_result_artifact_id()
    }

    pub(crate) fn comparison_id(&self) -> &UnicaId {
        self.ready.comparison_id()
    }

    pub(crate) fn settings_digest(&self) -> &Sha256Digest {
        self.ready.settings_digest()
    }

    pub(crate) fn sandbox_result_digest(&self) -> &Sha256Digest {
        self.ready.sandbox_result_digest()
    }

    pub(crate) fn support_graph_digest(&self) -> &Sha256Digest {
        self.ready.support_graph_digest()
    }

    pub(crate) fn observed_history_cursor(&self) -> &RepositoryHistoryCursor {
        self.ready.observed_history_cursor()
    }

    pub(crate) fn relevant_baseline_digest(&self) -> &Sha256Digest {
        self.ready.relevant_baseline_digest()
    }

    pub(crate) fn original_fingerprint(&self) -> &Sha256Digest {
        self.ready.original_fingerprint()
    }

    pub(crate) fn current_state_revision(&self) -> &Sha256Digest {
        &self.current_state_revision
    }
}

impl SupportManualPreflightAuthority {
    pub(crate) fn new(
        inputs: SupportPreflightInputs,
        recovery_coverage: SupportRecoveryDistributionCoverageAuthority,
    ) -> Result<Self, SupportContractError> {
        validate_manual_outcome(&inputs, &recovery_coverage)?;
        let record = build_record(SupportPreflightOutcome::ManualSupportRequired, inputs)?;
        Ok(Self {
            record,
            recovery_coverage,
        })
    }

    pub(crate) fn publish(
        self,
        action: &SupportActionAuthorizationAuthority,
    ) -> Result<SupportPreflightData, SupportContractError> {
        if action.support_gate_id() != &self.record.support_gate_id
            || action.support_gate_digest() != &self.record.support_gate_digest
            || action.candidate_set_digest() != &self.record.candidate_set_digest
            || action.expected_before_history_cursor() != &self.record.observed_history_cursor
            || action.expected_relevant_baseline_digest() != &self.record.relevant_baseline_digest
            || action.authorized_transitions() != &self.record.required_transitions
            || action.support_recovery_distribution_set_digest()
                != &self.record.support_recovery_distribution_set_digest
            || action.expected_original_fingerprint() != &self.record.original_fingerprint
        {
            return Err(SupportContractError(
                "support action projection disagrees with its manual support gate",
            ));
        }
        Ok(SupportPreflightData {
            record: self.record,
            support_action_id: Some(action.support_action_id().clone()),
            support_action_digest: Some(action.support_action_digest().clone()),
        })
    }

    pub(crate) const fn support_gate_id(&self) -> &UnicaId {
        &self.record.support_gate_id
    }

    pub(crate) const fn support_gate_digest(&self) -> &Sha256Digest {
        &self.record.support_gate_digest
    }

    pub(crate) const fn candidate_set_digest(&self) -> &Sha256Digest {
        &self.record.candidate_set_digest
    }

    pub(crate) const fn observed_history_cursor(&self) -> &RepositoryHistoryCursor {
        &self.record.observed_history_cursor
    }

    pub(crate) const fn relevant_baseline_digest(&self) -> &Sha256Digest {
        &self.record.relevant_baseline_digest
    }

    pub(crate) const fn support_graph_digest(&self) -> &Sha256Digest {
        &self.record.support_graph_digest
    }

    pub(crate) const fn required_transitions(&self) -> &SupportTransitions {
        &self.record.required_transitions
    }

    pub(crate) const fn original_fingerprint(&self) -> &Sha256Digest {
        &self.record.original_fingerprint
    }

    pub(crate) const fn recovery_coverage(&self) -> &SupportRecoveryDistributionCoverageAuthority {
        &self.recovery_coverage
    }
}

impl SupportPreflightData {
    pub(crate) fn ready(inputs: SupportPreflightInputs) -> Result<Self, SupportContractError> {
        validate_ready_outcome(&inputs)?;
        Self::without_action(SupportPreflightOutcome::Ready, inputs)
    }

    pub(crate) fn vendor_forbids_changes(
        inputs: SupportPreflightInputs,
    ) -> Result<Self, SupportContractError> {
        validate_vendor_outcome(&inputs)?;
        Self::without_action(SupportPreflightOutcome::VendorForbidsChanges, inputs)
    }

    pub(crate) fn inconclusive(
        inputs: SupportPreflightInputs,
        recovery_coverage: Option<&SupportInconclusiveRecoveryCoverageAuthority>,
    ) -> Result<Self, SupportContractError> {
        validate_inconclusive_outcome(&inputs, recovery_coverage)?;
        Self::without_action(
            SupportPreflightOutcome::SupportPreflightInconclusive,
            inputs,
        )
    }

    fn without_action(
        outcome: SupportPreflightOutcome,
        inputs: SupportPreflightInputs,
    ) -> Result<Self, SupportContractError> {
        Ok(Self {
            record: build_record(outcome, inputs)?,
            support_action_id: None,
            support_action_digest: None,
        })
    }

    /// Atomically replace only capability-revalidated all-unrelated history
    /// evidence.  The semantic gate and its digest remain byte-identical.
    pub(crate) fn replace_history_evidence(
        &mut self,
        history_evidence: SupportGateHistoryEvidence,
    ) -> Result<(), SupportContractError> {
        if history_evidence.gate_observed_cursor() != &self.record.observed_history_cursor
            || history_evidence.relevant_baseline_digest() != &self.record.relevant_baseline_digest
        {
            return Err(SupportContractError(
                "replacement history evidence disagrees with the semantic support gate",
            ));
        }
        self.record.history_evidence = history_evidence;
        Ok(())
    }

    pub(crate) const fn outcome(&self) -> SupportPreflightOutcome {
        self.record.outcome
    }

    pub(crate) const fn support_gate_id(&self) -> &UnicaId {
        &self.record.support_gate_id
    }

    pub(crate) const fn support_gate_digest(&self) -> &Sha256Digest {
        &self.record.support_gate_digest
    }

    pub(crate) const fn history_evidence(&self) -> &SupportGateHistoryEvidence {
        &self.record.history_evidence
    }
}

fn build_record(
    outcome: SupportPreflightOutcome,
    inputs: SupportPreflightInputs,
) -> Result<SupportPreflightRecord, SupportContractError> {
    validate_common(&inputs)?;
    let candidate_set_id = inputs.candidate_set.candidate_set_id().clone();
    let candidate_set_digest = inputs.candidate_set.digest().clone();
    let candidates = inputs.candidate_set.candidates().clone();
    let relevant_baseline_digest = inputs.history_evidence.relevant_baseline_digest().clone();
    let observed_history_cursor = inputs.history_evidence.classified_through_cursor().clone();
    let support_recovery_distribution_set_digest =
        inputs.support_recovery_distributions.digest().clone();
    let gate_inputs = SupportGateInputDigests::new(
        candidate_set_digest.clone(),
        inputs.sources.canonical_delta_digest.clone(),
        inputs.sources.ordinary_result_digest.clone(),
        inputs.sources.support_graph_digest.clone(),
        support_recovery_distribution_set_digest.clone(),
        inputs.sources.settings_digest.clone(),
        inputs.sources.sandbox_result_digest.clone(),
        inputs.sources.capability_row_digest.clone(),
        inputs.sources.original_fingerprint.clone(),
    );
    let digest_record = SupportGateDigestRecord {
        support_gate_id: inputs.support_gate_id.clone(),
        outcome,
        candidate_set_id: candidate_set_id.clone(),
        gate_inputs: gate_inputs.clone(),
        relevant_baseline_digest: relevant_baseline_digest.clone(),
        ordinary_result_artifact_id: inputs.sources.ordinary_result_artifact_id.clone(),
        comparison_id: inputs.sources.comparison_id.clone(),
        capability_row_id: inputs.sources.capability_row_id.clone(),
        blockers: inputs.blockers.clone(),
        evidence_gaps: inputs.evidence_gaps.clone(),
        required_transitions: inputs.required_transitions.clone(),
        surplus_transitions: inputs.surplus_transitions.clone(),
    };
    let support_gate_digest = contract_digest(&digest_record, "support-gate digest failed")?;
    Ok(SupportPreflightRecord {
        support_gate_id: inputs.support_gate_id,
        outcome,
        candidate_set_id,
        candidate_set_digest,
        gate_inputs,
        candidates,
        blockers: inputs.blockers,
        evidence_gaps: inputs.evidence_gaps,
        support_graph_digest: inputs.sources.support_graph_digest,
        required_transitions: inputs.required_transitions,
        surplus_transitions: inputs.surplus_transitions,
        observed_history_cursor,
        relevant_baseline_digest,
        original_fingerprint: inputs.sources.original_fingerprint,
        ordinary_result_artifact_id: inputs.sources.ordinary_result_artifact_id,
        comparison_id: inputs.sources.comparison_id,
        settings_digest: inputs.sources.settings_digest,
        sandbox_result_digest: inputs.sources.sandbox_result_digest,
        support_recovery_distributions: inputs.support_recovery_distributions.as_slice().to_vec(),
        support_recovery_distribution_set_digest,
        capability_row_id: inputs.sources.capability_row_id,
        support_gate_digest,
        history_evidence: inputs.history_evidence,
    })
}

fn validate_common(inputs: &SupportPreflightInputs) -> Result<(), SupportContractError> {
    if !inputs.history_evidence.partition_is_empty()
        || inputs.history_evidence.gate_observed_cursor()
            != inputs.history_evidence.classified_through_cursor()
    {
        return Err(SupportContractError(
            "support preflight publication requires equal cursors and an empty history partition",
        ));
    }
    validate_surplus(&inputs.required_transitions, &inputs.surplus_transitions)?;
    for blocker in inputs.blockers.as_slice() {
        let Some(candidate) =
            inputs
                .candidate_set
                .candidates()
                .as_slice()
                .iter()
                .find(|candidate| {
                    candidate.object_id() == blocker.object_id()
                        && candidate.layer_id() == blocker.layer_id()
                        && candidate.object_display() == blocker.object_display()
                })
        else {
            return Err(SupportContractError(
                "support blocker does not repeat an exact candidate identity",
            ));
        };
        let reason_matches = match blocker.reason() {
            SupportBlockerReason::ConfigurationChangesDisabled => candidate.layer_id().is_some(),
            SupportBlockerReason::ObjectLocked => {
                candidate.current_state() == SupportCurrentState::Locked
            }
            SupportBlockerReason::VendorRestriction => matches!(
                candidate.vendor_restriction(),
                VendorChangeRestriction::ChangesNotRecommended
                    | VendorChangeRestriction::ChangesForbidden
            ),
            SupportBlockerReason::OffSupportRequired => {
                candidate.required_state() == SupportRequiredState::OffSupportRequired
            }
            SupportBlockerReason::ClassificationIncomplete => {
                candidate.vendor_restriction() == VendorChangeRestriction::Unknown
            }
            SupportBlockerReason::DiagnosticCoverageIncomplete => true,
        };
        if !reason_matches {
            return Err(SupportContractError(
                "support blocker reason disagrees with its candidate evidence",
            ));
        }
    }
    Ok(())
}

fn validate_surplus(
    required: &SupportTransitions,
    surplus: &SupportTransitions,
) -> Result<(), SupportContractError> {
    if surplus
        .as_slice()
        .iter()
        .any(|transition| !transition.is_restore() || !required.contains(transition))
    {
        return Err(SupportContractError(
            "surplus support transitions must be an exact subset of required restore transitions",
        ));
    }
    Ok(())
}

fn candidates_have_incomplete_classification(inputs: &SupportPreflightInputs) -> bool {
    inputs
        .candidate_set
        .candidates()
        .as_slice()
        .iter()
        .any(|candidate| candidate.vendor_restriction() == VendorChangeRestriction::Unknown)
}

fn candidates_have_vendor_prohibition(inputs: &SupportPreflightInputs) -> bool {
    inputs
        .candidate_set
        .candidates()
        .as_slice()
        .iter()
        .any(|candidate| {
            matches!(
                candidate.vendor_restriction(),
                VendorChangeRestriction::ChangesNotRecommended
                    | VendorChangeRestriction::ChangesForbidden
            ) || candidate.required_state() == SupportRequiredState::OffSupportRequired
        })
}

fn has_incomplete_blocker(inputs: &SupportPreflightInputs) -> bool {
    inputs.blockers.as_slice().iter().any(|blocker| {
        matches!(
            blocker.reason(),
            SupportBlockerReason::ClassificationIncomplete
                | SupportBlockerReason::DiagnosticCoverageIncomplete
        )
    })
}

fn validate_ready_outcome(inputs: &SupportPreflightInputs) -> Result<(), SupportContractError> {
    if !inputs.blockers.is_empty()
        || !inputs.evidence_gaps.is_empty()
        || !inputs.required_transitions.is_empty()
        || !inputs.surplus_transitions.is_empty()
        || !inputs.support_recovery_distributions.is_empty()
        || candidates_have_incomplete_classification(inputs)
        || has_incomplete_blocker(inputs)
        || candidates_have_vendor_prohibition(inputs)
    {
        return Err(SupportContractError(
            "ready support preflight contains blockers, gaps, transitions, recovery evidence, or non-ready candidates",
        ));
    }
    Ok(())
}

fn validate_manual_outcome(
    inputs: &SupportPreflightInputs,
    coverage: &SupportRecoveryDistributionCoverageAuthority,
) -> Result<(), SupportContractError> {
    if !inputs.evidence_gaps.is_empty()
        || inputs.required_transitions.is_empty()
        || candidates_have_incomplete_classification(inputs)
        || has_incomplete_blocker(inputs)
        || candidates_have_vendor_prohibition(inputs)
        || coverage.support_graph_digest() != &inputs.sources.support_graph_digest
        || coverage.distributions() != &inputs.support_recovery_distributions
    {
        return Err(SupportContractError(
            "manual support preflight lacks exact classification, transitions, or recovery coverage",
        ));
    }
    Ok(())
}

fn validate_vendor_outcome(inputs: &SupportPreflightInputs) -> Result<(), SupportContractError> {
    let has_vendor_blocker = inputs.blockers.as_slice().iter().any(|blocker| {
        matches!(
            blocker.reason(),
            SupportBlockerReason::VendorRestriction | SupportBlockerReason::OffSupportRequired
        )
    });
    if !inputs.evidence_gaps.is_empty()
        || inputs.blockers.is_empty()
        || !has_vendor_blocker
        || !candidates_have_vendor_prohibition(inputs)
        || candidates_have_incomplete_classification(inputs)
        || has_incomplete_blocker(inputs)
        || !inputs.required_transitions.is_empty()
        || !inputs.surplus_transitions.is_empty()
        || !inputs.support_recovery_distributions.is_empty()
    {
        return Err(SupportContractError(
            "vendor-forbidden support preflight is not an exact typed prohibition",
        ));
    }
    Ok(())
}

fn validate_inconclusive_outcome(
    inputs: &SupportPreflightInputs,
    recovery_coverage: Option<&SupportInconclusiveRecoveryCoverageAuthority>,
) -> Result<(), SupportContractError> {
    let has_incomplete_blocker = has_incomplete_blocker(inputs);
    if inputs.blockers.is_empty() && inputs.evidence_gaps.is_empty() {
        return Err(SupportContractError(
            "inconclusive support preflight requires typed incomplete blockers or evidence gaps",
        ));
    }

    let has_classification_gap = inputs
        .evidence_gaps
        .as_slice()
        .iter()
        .any(|gap| gap.is_preflight_classification_gap());
    let classification_incomplete = has_incomplete_blocker
        || candidates_have_incomplete_classification(inputs)
        || has_classification_gap;
    if classification_incomplete {
        if recovery_coverage.is_some()
            || !inputs.required_transitions.is_empty()
            || !inputs.surplus_transitions.is_empty()
            || !inputs.support_recovery_distributions.is_empty()
            || !inputs
                .evidence_gaps
                .as_slice()
                .iter()
                .all(|gap| gap.is_preflight_classification_gap())
        {
            return Err(SupportContractError(
                "classification-inconclusive preflight must precede transition and recovery evaluation",
            ));
        }
        return Ok(());
    }

    if candidates_have_vendor_prohibition(inputs) || inputs.required_transitions.is_empty() {
        return Err(SupportContractError(
            "manual-safety gaps cannot mask an exact vendor-forbidden or ready outcome",
        ));
    }
    let Some(recovery_coverage) = recovery_coverage else {
        return Err(SupportContractError(
            "otherwise-manual inconclusive preflight lacks exact recovery subset authority",
        ));
    };
    if recovery_coverage.support_graph_digest() != &inputs.sources.support_graph_digest
        || recovery_coverage.distributions() != &inputs.support_recovery_distributions
        || recovery_coverage.required_transitions() != &inputs.required_transitions
        || recovery_coverage.evidence_gaps() != &inputs.evidence_gaps
    {
        return Err(SupportContractError(
            "inconclusive recovery subset authority disagrees with the preflight projection",
        ));
    }
    Ok(())
}

impl Serialize for SupportPreflightData {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let record = &self.record;
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("supportGateId", &record.support_gate_id)?;
        map.serialize_entry("outcome", &record.outcome)?;
        map.serialize_entry("candidateSetId", &record.candidate_set_id)?;
        map.serialize_entry("candidateSetDigest", &record.candidate_set_digest)?;
        map.serialize_entry("gateInputs", &record.gate_inputs)?;
        map.serialize_entry("candidates", &record.candidates)?;
        map.serialize_entry("blockers", &record.blockers)?;
        map.serialize_entry("evidenceGaps", &record.evidence_gaps)?;
        map.serialize_entry("supportGraphDigest", &record.support_graph_digest)?;
        map.serialize_entry("requiredTransitions", &record.required_transitions)?;
        map.serialize_entry("surplusTransitions", &record.surplus_transitions)?;
        map.serialize_entry("observedHistoryCursor", &record.observed_history_cursor)?;
        map.serialize_entry("relevantBaselineDigest", &record.relevant_baseline_digest)?;
        map.serialize_entry("originalFingerprint", &record.original_fingerprint)?;
        map.serialize_entry(
            "ordinaryResultArtifactId",
            &record.ordinary_result_artifact_id,
        )?;
        map.serialize_entry("comparisonId", &record.comparison_id)?;
        map.serialize_entry("settingsDigest", &record.settings_digest)?;
        map.serialize_entry("sandboxResultDigest", &record.sandbox_result_digest)?;
        map.serialize_entry(
            "supportRecoveryDistributions",
            &record.support_recovery_distributions,
        )?;
        map.serialize_entry(
            "supportRecoveryDistributionSetDigest",
            &record.support_recovery_distribution_set_digest,
        )?;
        map.serialize_entry("capabilityRowId", &record.capability_row_id)?;
        map.serialize_entry("supportGateDigest", &record.support_gate_digest)?;
        map.serialize_entry("historyEvidence", &record.history_evidence)?;
        if let (Some(action_id), Some(action_digest)) =
            (&self.support_action_id, &self.support_action_digest)
        {
            map.serialize_entry("supportActionId", action_id)?;
            map.serialize_entry("supportActionDigest", action_digest)?;
        }
        map.end()
    }
}

fn schema_value<T: JsonSchema>(generator: &mut SchemaGenerator) -> Value {
    serde_json::to_value(generator.subschema_for::<T>())
        .expect("typed support preflight schema is serializable")
}

#[derive(Debug, Clone, Copy)]
struct ArrayBounds {
    min: usize,
    max: usize,
}

impl ArrayBounds {
    const fn any(max: usize) -> Self {
        Self { min: 0, max }
    }

    const fn empty() -> Self {
        Self { min: 0, max: 0 }
    }

    const fn nonempty(max: usize) -> Self {
        Self { min: 1, max }
    }
}

#[derive(Debug, Clone, Copy)]
struct PreflightSchemaBranch {
    outcome: &'static str,
    action_projection: bool,
    blockers: ArrayBounds,
    evidence_gaps: ArrayBounds,
    required_transitions: ArrayBounds,
    surplus_transitions: ArrayBounds,
    recovery_distributions: ArrayBounds,
}

const PREFLIGHT_SCHEMA_BRANCHES: &[PreflightSchemaBranch] = &[
    PreflightSchemaBranch {
        outcome: "ready",
        action_projection: false,
        blockers: ArrayBounds::empty(),
        evidence_gaps: ArrayBounds::empty(),
        required_transitions: ArrayBounds::empty(),
        surplus_transitions: ArrayBounds::empty(),
        recovery_distributions: ArrayBounds::empty(),
    },
    PreflightSchemaBranch {
        outcome: "manualSupportRequired",
        action_projection: true,
        blockers: ArrayBounds::any(MAX_METADATA_ITEMS),
        evidence_gaps: ArrayBounds::empty(),
        required_transitions: ArrayBounds::nonempty(MAX_METADATA_ITEMS),
        surplus_transitions: ArrayBounds::any(MAX_METADATA_ITEMS),
        recovery_distributions: ArrayBounds::nonempty(MAX_RECOVERY_DISTRIBUTIONS),
    },
    PreflightSchemaBranch {
        outcome: "vendorForbidsChanges",
        action_projection: false,
        blockers: ArrayBounds::nonempty(MAX_METADATA_ITEMS),
        evidence_gaps: ArrayBounds::empty(),
        required_transitions: ArrayBounds::empty(),
        surplus_transitions: ArrayBounds::empty(),
        recovery_distributions: ArrayBounds::empty(),
    },
    // The two inconclusive branches are disjoint: the first has no gaps and
    // therefore requires a blocker; the second has at least one gap and may
    // also carry candidate-shaped incomplete blockers.
    PreflightSchemaBranch {
        outcome: "supportPreflightInconclusive",
        action_projection: false,
        blockers: ArrayBounds::nonempty(MAX_METADATA_ITEMS),
        evidence_gaps: ArrayBounds::empty(),
        required_transitions: ArrayBounds::any(MAX_METADATA_ITEMS),
        surplus_transitions: ArrayBounds::any(MAX_METADATA_ITEMS),
        recovery_distributions: ArrayBounds::any(MAX_RECOVERY_DISTRIBUTIONS),
    },
    PreflightSchemaBranch {
        outcome: "supportPreflightInconclusive",
        action_projection: false,
        blockers: ArrayBounds::any(MAX_METADATA_ITEMS),
        evidence_gaps: ArrayBounds::nonempty(MAX_GENERAL_ITEMS),
        required_transitions: ArrayBounds::any(MAX_METADATA_ITEMS),
        surplus_transitions: ArrayBounds::any(MAX_METADATA_ITEMS),
        recovery_distributions: ArrayBounds::any(MAX_RECOVERY_DISTRIBUTIONS),
    },
];

fn bounded_array_schema<T: JsonSchema>(
    generator: &mut SchemaGenerator,
    bounds: ArrayBounds,
) -> Value {
    let mut schema = Map::new();
    schema.insert("type".to_owned(), Value::String("array".to_owned()));
    schema.insert("maxItems".to_owned(), Value::Number(bounds.max.into()));
    if bounds.min > 0 {
        schema.insert("minItems".to_owned(), Value::Number(bounds.min.into()));
    }
    schema.insert("uniqueItems".to_owned(), Value::Bool(true));
    schema.insert("items".to_owned(), schema_value::<T>(generator));
    Value::Object(schema)
}

fn preflight_branch_schema(
    generator: &mut SchemaGenerator,
    branch: PreflightSchemaBranch,
) -> Schema {
    let mut properties = Map::new();
    let mut required = Vec::new();
    macro_rules! property {
        ($wire:literal, $type:ty) => {{
            properties.insert($wire.to_owned(), schema_value::<$type>(generator));
            required.push(Value::String($wire.to_owned()));
        }};
    }
    property!("supportGateId", UnicaId);
    properties.insert(
        "outcome".to_owned(),
        serde_json::json!({ "type": "string", "const": branch.outcome }),
    );
    required.push(Value::String("outcome".to_owned()));
    property!("candidateSetId", UnicaId);
    property!("candidateSetDigest", Sha256Digest);
    property!("gateInputs", SupportGateInputDigests);
    property!("candidates", SupportCandidates);
    properties.insert(
        "blockers".to_owned(),
        bounded_array_schema::<SupportBlocker>(generator, branch.blockers),
    );
    required.push(Value::String("blockers".to_owned()));
    properties.insert(
        "evidenceGaps".to_owned(),
        bounded_array_schema::<SupportEvidenceGap>(generator, branch.evidence_gaps),
    );
    required.push(Value::String("evidenceGaps".to_owned()));
    property!("supportGraphDigest", Sha256Digest);
    properties.insert(
        "requiredTransitions".to_owned(),
        bounded_array_schema::<SupportTransition>(generator, branch.required_transitions),
    );
    required.push(Value::String("requiredTransitions".to_owned()));
    properties.insert(
        "surplusTransitions".to_owned(),
        bounded_array_schema::<SupportTransition>(generator, branch.surplus_transitions),
    );
    required.push(Value::String("surplusTransitions".to_owned()));
    property!("observedHistoryCursor", RepositoryHistoryCursor);
    property!("relevantBaselineDigest", Sha256Digest);
    property!("originalFingerprint", Sha256Digest);
    property!("ordinaryResultArtifactId", UnicaId);
    property!("comparisonId", UnicaId);
    property!("settingsDigest", Sha256Digest);
    property!("sandboxResultDigest", Sha256Digest);
    properties.insert(
        "supportRecoveryDistributions".to_owned(),
        bounded_array_schema::<SupportRecoveryDistributionEvidence>(
            generator,
            branch.recovery_distributions,
        ),
    );
    required.push(Value::String("supportRecoveryDistributions".to_owned()));
    property!("supportRecoveryDistributionSetDigest", Sha256Digest);
    property!("capabilityRowId", CapabilityRowId);
    property!("supportGateDigest", Sha256Digest);
    property!("historyEvidence", SupportGateHistoryEvidence);
    if branch.action_projection {
        property!("supportActionId", UnicaId);
        property!("supportActionDigest", Sha256Digest);
    }
    let mut object = Map::new();
    object.insert("type".to_owned(), Value::String("object".to_owned()));
    object.insert("properties".to_owned(), Value::Object(properties));
    object.insert("required".to_owned(), Value::Array(required));
    object.insert("additionalProperties".to_owned(), Value::Bool(false));
    Schema::from(object)
}

impl JsonSchema for SupportPreflightData {
    fn schema_name() -> Cow<'static, str> {
        "SupportPreflightData".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(
            PREFLIGHT_SCHEMA_BRANCHES
                .iter()
                .copied()
                .map(|branch| preflight_branch_schema(generator, branch))
                .collect(),
        )
    }
}
