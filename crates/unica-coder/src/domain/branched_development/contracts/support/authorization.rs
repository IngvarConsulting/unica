#[cfg(test)]
mod tests {
    use super::super::evidence::{
        SupportRecoveryDistributionHandoff, SupportRecoveryDistributionHandoffInputs,
        SupportRecoveryDistributionSet, UserVisibleCfFileName,
    };
    use super::super::model::{RootReachableSupportLayerSet, SupportTransition};
    use super::*;
    use crate::domain::branched_development::contracts::instructions::AcquireSupportRootInstruction;
    use crate::domain::branched_development::contracts::scalars::{
        DisplayPath, RepositoryIdentityComponent, RepositoryTargetDisplay,
    };
    use crate::domain::branched_development::contracts::schema::audit_json_schema;
    use crate::domain::branched_development::{ProfileArtifactRefId, SupportLayerId};
    use schemars::{schema_for, JsonSchema};
    use serde::de::DeserializeOwned;
    use serde_json::{json, Value};

    const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const ID_1: &str = "11111111-1111-4111-8111-111111111111";
    const ID_2: &str = "22222222-2222-4222-8222-222222222222";

    fn digest() -> Sha256Digest {
        Sha256Digest::parse(A).unwrap()
    }

    fn digest_b() -> Sha256Digest {
        Sha256Digest::parse(B).unwrap()
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
        audit_json_schema(&schema).expect("authorization schema must be recursively closed");
    }

    fn schema_accepts<T: JsonSchema>(value: &Value) -> bool {
        let schema = serde_json::to_value(schema_for!(T)).unwrap();
        jsonschema::options()
            .with_draft(jsonschema::Draft::Draft202012)
            .build(&schema)
            .unwrap()
            .is_valid(value)
    }

    fn inputs(manual_actor: &str) -> SupportActionAuthorizationInputs {
        let layer_id = SupportLayerId::parse("layer-a").unwrap();
        let authorized_transitions =
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
                manual_actor_username: RepositoryUsername::parse(manual_actor).unwrap(),
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
            RootReachableSupportLayerSet::from_capability_adapter(vec![layer_id], digest())
                .unwrap();
        let support_recovery_distribution_coverage =
            SupportRecoveryDistributionCoverageAuthority::prove_complete(
                reachable,
                recovery_set,
                &authorized_transitions,
            )
            .unwrap();
        SupportActionAuthorizationInputs {
            support_action_id: id(ID_1),
            purpose: SupportActionPurpose::MainIntegrationPrerequisite,
            support_gate_id: id(ID_2),
            support_gate_digest: digest(),
            candidate_set_digest: digest(),
            expected_before_history_cursor: cursor(),
            expected_relevant_baseline_digest: digest(),
            expected_support_graph_digest: digest(),
            authorized_transitions,
            support_recovery_distribution_coverage,
            reserved_integration_username: RepositoryUsername::parse("reserved-user").unwrap(),
            reserved_original_identity_digest: digest(),
            expected_original_fingerprint: digest(),
            manual_actor_username: RepositoryUsername::parse(manual_actor).unwrap(),
            expected_manual_working_infobase_lease_capability_id: None,
            phase_binding: SupportActionPhaseBinding::main_integration(digest()),
        }
    }

    fn working_identity(infobase: &str) -> ManualWorkingInfobaseIdentity {
        ManualWorkingInfobaseIdentity::new(
            RepositoryIdentityComponent::parse("HOST").unwrap(),
            RepositoryIdentityComponent::parse(infobase).unwrap(),
        )
        .unwrap()
    }

    fn separate_inputs(
        baseline_graph_digest: Sha256Digest,
        baseline_lease_capability_id: CapabilityRowId,
        expected_lease_capability_id: CapabilityRowId,
    ) -> (
        SupportActionAuthorizationInputs,
        ManualWorkingInfobaseIdentity,
        ManualWorkingInfobaseBaseline,
    ) {
        let identity = working_identity("Working IB");
        let layer_id = SupportLayerId::parse("layer-a").unwrap();
        let authorized_transitions =
            SupportTransitions::new(vec![SupportTransition::enable_configuration_changes(
                RepositoryTargetDisplay::parse("Configuration").unwrap(),
                layer_id.clone(),
            )])
            .unwrap();
        let handoff = SupportRecoveryDistributionHandoff::new(
            ManualSupportTargetMode::SeparateWorkingInfobase,
            Some(identity.clone()),
            SupportRecoveryDistributionHandoffInputs {
                handoff_id: id(ID_1),
                profile_artifact_ref_id: ProfileArtifactRefId::parse("vendor.layer-a").unwrap(),
                profile_artifact_display: DisplayPath::parse("Vendor layer A").unwrap(),
                user_visible_file_name: UserVisibleCfFileName::parse("vendor-layer-a.cf").unwrap(),
                manual_actor_username: RepositoryUsername::parse("support-user").unwrap(),
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
            RootReachableSupportLayerSet::from_capability_adapter(vec![layer_id], digest())
                .unwrap();
        let coverage = SupportRecoveryDistributionCoverageAuthority::prove_complete(
            reachable,
            recovery_set,
            &authorized_transitions,
        )
        .unwrap();
        let baseline = ManualWorkingInfobaseBaseline::new(
            identity.clone(),
            cursor(),
            digest(),
            digest(),
            digest(),
            baseline_graph_digest,
            id(ID_1),
            baseline_lease_capability_id,
        )
        .unwrap();
        let inputs = SupportActionAuthorizationInputs {
            support_action_id: id(ID_1),
            purpose: SupportActionPurpose::MainIntegrationPrerequisite,
            support_gate_id: id(ID_2),
            support_gate_digest: digest(),
            candidate_set_digest: digest(),
            expected_before_history_cursor: cursor(),
            expected_relevant_baseline_digest: digest(),
            expected_support_graph_digest: digest(),
            authorized_transitions,
            support_recovery_distribution_coverage: coverage,
            reserved_integration_username: RepositoryUsername::parse("reserved-user").unwrap(),
            reserved_original_identity_digest: digest(),
            expected_original_fingerprint: digest(),
            manual_actor_username: RepositoryUsername::parse("support-user").unwrap(),
            expected_manual_working_infobase_lease_capability_id: Some(
                expected_lease_capability_id,
            ),
            phase_binding: SupportActionPhaseBinding::main_integration(digest()),
        };
        (inputs, identity, baseline)
    }

    // Authorization records are authority-derived durable outputs, never raw
    // caller-selected state machines.
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

    assert_not_deserialize_owned!(SupportActionAuthorizationData);
    assert_not_deserialize_owned!(ActiveSupportActionResumeHandle);
    assert_not_deserialize_owned!(TerminalSupportActionAuthorization);
    assert_not_deserialize_owned!(AwaitingSupportInstructionProjection);
    assert_not_deserialize_owned!(ArmedSupportInstructionProjection);

    #[test]
    fn action_digest_and_authorization_schemas_are_closed_and_state_exact() {
        assert_closed::<SupportActionDigestRecord>();
        assert_closed::<SupportActionAuthorizationData>();
        assert_closed::<ActiveSupportActionResumeHandle>();
        assert_closed::<TerminalSupportActionAuthorization>();
    }

    #[test]
    fn reserved_mode_rejects_actor_substitution_and_schema_field_splice() {
        assert!(SupportActionAuthorizationAuthority::reserved_original(
            inputs("other-user"),
            CapabilityRowId::parse("reserved-original-lease.v1").unwrap(),
            digest(),
        )
        .is_err());

        let authority = SupportActionAuthorizationAuthority::reserved_original(
            inputs("reserved-user"),
            CapabilityRowId::parse("reserved-original-lease.v1").unwrap(),
            digest(),
        )
        .unwrap();
        let active = ActiveSupportActionResumeHandle::publish(authority).unwrap();
        let projection = active
            .awaiting_support_instruction_projection()
            .expect("published action projects acquire-root instruction inputs");
        assert_eq!(projection.support_action_id(), &id(ID_1));
        assert_eq!(projection.support_gate_id(), &id(ID_2));
        assert_eq!(
            projection.manual_target_mode(),
            ManualSupportTargetMode::ReservedOriginal
        );
        assert_eq!(
            projection.manual_actor_username(),
            &RepositoryUsername::parse("reserved-user").unwrap()
        );
        assert!(projection.working_infobase_identity().is_none());
        let instruction =
            AcquireSupportRootInstruction::from_awaiting_projection(&projection).unwrap();
        let instruction = serde_json::to_value(instruction).unwrap();
        assert_eq!(instruction["supportActionId"], json!(ID_1));
        assert_eq!(instruction["manualTargetMode"], json!("reservedOriginal"));
        assert_eq!(instruction["repositoryUsername"], json!("reserved-user"));
        assert!(active.armed_support_instruction_projection().is_none());
        let encoded = serde_json::to_value(&active).unwrap();
        assert_eq!(encoded["state"], json!("awaitingArm"));
        assert_eq!(encoded["manualTargetMode"], json!("reservedOriginal"));
        assert!(encoded.get("armingReceipt").is_none());
        assert!(encoded.get("freezeKind").is_none());
        assert!(schema_accepts::<ActiveSupportActionResumeHandle>(&encoded));

        let mut spliced = encoded;
        spliced["manualWorkingInfobaseIdentity"] = json!({
            "computer": "HOST",
            "infobase": "IB",
            "digest": A,
        });
        assert!(!schema_accepts::<SupportActionAuthorizationData>(&spliced));
    }

    #[test]
    fn prearm_freeze_stays_active_while_cancellation_is_terminal() {
        let authority = SupportActionAuthorizationAuthority::reserved_original(
            inputs("reserved-user"),
            CapabilityRowId::parse("reserved-original-lease.v1").unwrap(),
            digest(),
        )
        .unwrap();
        let active = ActiveSupportActionResumeHandle::publish(authority).unwrap();
        let frozen = active.clone().freeze_prearm_cancellation_effect().unwrap();
        let frozen_json = serde_json::to_value(&frozen).unwrap();
        assert_eq!(frozen_json["state"], json!("frozenForRecovery"));
        assert_eq!(frozen_json["freezeKind"], json!("preArmCancellationEffect"));
        assert!(frozen_json.get("armingReceipt").is_none());

        let terminal = active.cancel().unwrap();
        let terminal_json = serde_json::to_value(&terminal).unwrap();
        assert_eq!(terminal_json["state"], json!("cancelled"));
        assert!(schema_accepts::<TerminalSupportActionAuthorization>(
            &terminal_json
        ));
        assert!(!schema_accepts::<ActiveSupportActionResumeHandle>(
            &terminal_json
        ));
    }

    #[test]
    fn manual_authorization_rejects_empty_transition_or_recovery_coverage() {
        let layer_id = SupportLayerId::parse("layer-a").unwrap();
        let empty_transitions = SupportTransitions::new(Vec::new()).unwrap();
        let reachable =
            RootReachableSupportLayerSet::from_capability_adapter(vec![layer_id], digest())
                .unwrap();
        let empty_recovery = SupportRecoveryDistributionSet::new(Vec::new()).unwrap();
        assert!(
            SupportRecoveryDistributionCoverageAuthority::prove_complete(
                reachable,
                empty_recovery,
                &empty_transitions,
            )
            .is_err()
        );
    }

    #[test]
    fn arming_rejects_substituted_support_graph_and_graph_is_not_persisted_raw() {
        let mut mismatched_inputs = inputs("reserved-user");
        mismatched_inputs.expected_support_graph_digest = digest_b();
        assert!(SupportActionAuthorizationAuthority::reserved_original(
            mismatched_inputs,
            CapabilityRowId::parse("reserved-original-lease.v1").unwrap(),
            digest(),
        )
        .is_err());

        let authority = SupportActionAuthorizationAuthority::reserved_original(
            inputs("reserved-user"),
            CapabilityRowId::parse("reserved-original-lease.v1").unwrap(),
            digest(),
        )
        .unwrap();
        let active = ActiveSupportActionResumeHandle::publish(authority).unwrap();
        let ActiveSupportActionResumeHandle::AwaitingArm(record) = &active else {
            panic!("fresh action must await arming")
        };
        let substituted_graph = digest_b();
        let substituted = SupportActionArmingBinding {
            support_action_id: &record.immutable.support_action_id,
            support_action_digest: &record.support_action_digest,
            expected_before_history_cursor: &record.immutable.expected_before_history_cursor,
            support_gate_digest: &record.immutable.support_gate_digest,
            candidate_set_digest: &record.immutable.candidate_set_digest,
            expected_relevant_baseline_digest: &record.immutable.expected_relevant_baseline_digest,
            support_graph_digest: &substituted_graph,
            support_recovery_distribution_set_digest: &record
                .immutable
                .support_recovery_distribution_set_digest,
            original_fingerprint: &record.immutable.expected_original_fingerprint,
            manual_target_mode: record.immutable.manual_target_mode,
        };
        assert!(!substituted.matches(record));

        let encoded = serde_json::to_value(active).unwrap();
        assert!(encoded.get("supportGraphDigest").is_none());
    }

    #[test]
    fn separate_mode_binds_gate_graph_profile_lease_and_exact_schema_leaf() {
        let capability = CapabilityRowId::parse("manual-working-ib-lease.v1").unwrap();
        let (valid_inputs, identity, baseline) =
            separate_inputs(digest(), capability.clone(), capability.clone());
        let authority = SupportActionAuthorizationAuthority::separate_working_infobase(
            valid_inputs,
            identity,
            baseline,
        )
        .unwrap();
        let active = ActiveSupportActionResumeHandle::publish(authority).unwrap();
        let encoded = serde_json::to_value(&active).unwrap();
        assert_eq!(
            encoded["manualTargetMode"],
            json!("separateWorkingInfobase")
        );
        assert!(schema_accepts::<ActiveSupportActionResumeHandle>(&encoded));

        let mut cross_mode = encoded;
        cross_mode["reservedOriginalLeaseCapabilityId"] = json!("reserved-original-lease.v1");
        cross_mode["manualActorLockBaselineDigest"] = json!(A);
        assert!(!schema_accepts::<SupportActionAuthorizationData>(
            &cross_mode
        ));

        let (wrong_graph_inputs, identity, wrong_graph_baseline) =
            separate_inputs(digest_b(), capability.clone(), capability.clone());
        assert!(
            SupportActionAuthorizationAuthority::separate_working_infobase(
                wrong_graph_inputs,
                identity,
                wrong_graph_baseline,
            )
            .is_err()
        );

        let wrong_capability = CapabilityRowId::parse("manual-working-ib-lease.v2").unwrap();
        let (wrong_capability_inputs, identity, wrong_capability_baseline) =
            separate_inputs(digest(), wrong_capability, capability);
        assert!(
            SupportActionAuthorizationAuthority::separate_working_infobase(
                wrong_capability_inputs,
                identity,
                wrong_capability_baseline,
            )
            .is_err()
        );
    }
}
use super::super::repository::RepositoryHistoryCursor;
use super::super::scalars::RepositoryUsername;
use super::super::schema::one_of_schema;
use super::evidence::{
    SupportActionArmingReceipt, SupportRecoveryDistributionCoverageAuthority,
    SupportRecoveryDistributionEvidence,
};
use super::model::{
    ManualSupportTargetMode, ManualWorkingInfobaseBaseline, ManualWorkingInfobaseIdentity,
    SupportActionPurpose, SupportContractError, SupportTransitions, TrueLiteral,
};
use crate::domain::branched_development::canonical_json::{
    canonical_contract_digest, contract_digest_record_sealed, ContractDigestRecord,
};
use crate::domain::branched_development::{CapabilityRowId, Sha256Digest, TaskPhase, UnicaId};
use schemars::{json_schema, JsonSchema, Schema, SchemaGenerator};
use serde::ser::SerializeMap;
use serde::{Serialize, Serializer};
use serde_json::{Map, Value};
use std::borrow::Cow;

fn contract_digest<T: ContractDigestRecord>(
    record: &T,
    message: &'static str,
) -> Result<Sha256Digest, SupportContractError> {
    canonical_contract_digest(record, None).map_err(|_| SupportContractError(message))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SupportActionPhaseBinding {
    origin_phase: TaskPhase,
    cancelled_phase: TaskPhase,
    relevant_advance_phase: TaskPhase,
    post_reconcile_phase: TaskPhase,
    phase_evidence_digest: Sha256Digest,
}

impl SupportActionPhaseBinding {
    pub(crate) const fn main_integration(phase_evidence_digest: Sha256Digest) -> Self {
        Self {
            origin_phase: TaskPhase::Synchronized,
            cancelled_phase: TaskPhase::Synchronized,
            relevant_advance_phase: TaskPhase::LocalVerified,
            post_reconcile_phase: TaskPhase::LocalVerified,
            phase_evidence_digest,
        }
    }

    pub(crate) const fn abandonment_from_checkpoint_adapter(
        origin_phase: TaskPhase,
        cancelled_phase: TaskPhase,
        relevant_advance_phase: TaskPhase,
        phase_evidence_digest: Sha256Digest,
    ) -> Self {
        Self {
            origin_phase,
            cancelled_phase,
            relevant_advance_phase,
            post_reconcile_phase: TaskPhase::AbandonmentReady,
            phase_evidence_digest,
        }
    }

    fn valid_for(&self, purpose: SupportActionPurpose) -> bool {
        match purpose {
            SupportActionPurpose::MainIntegrationPrerequisite => {
                self.origin_phase == TaskPhase::Synchronized
                    && self.cancelled_phase == TaskPhase::Synchronized
                    && self.relevant_advance_phase == TaskPhase::LocalVerified
                    && self.post_reconcile_phase == TaskPhase::LocalVerified
            }
            SupportActionPurpose::AbandonmentCleanup => {
                self.post_reconcile_phase == TaskPhase::AbandonmentReady
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SupportActionAuthorizationInputs {
    support_action_id: UnicaId,
    purpose: SupportActionPurpose,
    support_gate_id: UnicaId,
    support_gate_digest: Sha256Digest,
    candidate_set_digest: Sha256Digest,
    expected_before_history_cursor: RepositoryHistoryCursor,
    expected_relevant_baseline_digest: Sha256Digest,
    expected_support_graph_digest: Sha256Digest,
    authorized_transitions: SupportTransitions,
    support_recovery_distribution_coverage: SupportRecoveryDistributionCoverageAuthority,
    reserved_integration_username: RepositoryUsername,
    reserved_original_identity_digest: Sha256Digest,
    expected_original_fingerprint: Sha256Digest,
    manual_actor_username: RepositoryUsername,
    expected_manual_working_infobase_lease_capability_id: Option<CapabilityRowId>,
    phase_binding: SupportActionPhaseBinding,
}

impl SupportActionAuthorizationInputs {
    /// Raw fixture mint only. Production action-input construction stays
    /// sealed until the owning task can consume a typed manual-gate or
    /// abandonment-checkpoint binding rather than caller-selected digests.
    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub(crate) const fn fixture(
        support_action_id: UnicaId,
        purpose: SupportActionPurpose,
        support_gate_id: UnicaId,
        support_gate_digest: Sha256Digest,
        candidate_set_digest: Sha256Digest,
        expected_before_history_cursor: RepositoryHistoryCursor,
        expected_relevant_baseline_digest: Sha256Digest,
        expected_support_graph_digest: Sha256Digest,
        authorized_transitions: SupportTransitions,
        support_recovery_distribution_coverage: SupportRecoveryDistributionCoverageAuthority,
        reserved_integration_username: RepositoryUsername,
        reserved_original_identity_digest: Sha256Digest,
        expected_original_fingerprint: Sha256Digest,
        manual_actor_username: RepositoryUsername,
        expected_manual_working_infobase_lease_capability_id: Option<CapabilityRowId>,
        phase_binding: SupportActionPhaseBinding,
    ) -> Self {
        Self {
            support_action_id,
            purpose,
            support_gate_id,
            support_gate_digest,
            candidate_set_digest,
            expected_before_history_cursor,
            expected_relevant_baseline_digest,
            expected_support_graph_digest,
            authorized_transitions,
            support_recovery_distribution_coverage,
            reserved_integration_username,
            reserved_original_identity_digest,
            expected_original_fingerprint,
            manual_actor_username,
            expected_manual_working_infobase_lease_capability_id,
            phase_binding,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
struct AuthorizedSupportTransitionsDigestRecord(SupportTransitions);

impl contract_digest_record_sealed::Sealed for AuthorizedSupportTransitionsDigestRecord {}
impl ContractDigestRecord for AuthorizedSupportTransitionsDigestRecord {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SupportActionDigestRecord {
    support_action_id: UnicaId,
    purpose: SupportActionPurpose,
    support_gate_id: UnicaId,
    support_gate_digest: Sha256Digest,
    candidate_set_digest: Sha256Digest,
    expected_before_history_cursor: RepositoryHistoryCursor,
    expected_relevant_baseline_digest: Sha256Digest,
    arming_required: TrueLiteral,
    authorized_transitions: SupportTransitions,
    authorized_transitions_digest: Sha256Digest,
    support_recovery_distributions: Vec<SupportRecoveryDistributionEvidence>,
    support_recovery_distribution_set_digest: Sha256Digest,
    manual_target_mode: ManualSupportTargetMode,
    reserved_integration_username: RepositoryUsername,
    reserved_original_identity_digest: Sha256Digest,
    #[serde(skip_serializing_if = "Option::is_none")]
    reserved_original_lease_capability_id: Option<CapabilityRowId>,
    expected_original_fingerprint: Sha256Digest,
    manual_actor_username: RepositoryUsername,
    #[serde(skip_serializing_if = "Option::is_none")]
    manual_actor_lock_baseline_digest: Option<Sha256Digest>,
    #[serde(skip_serializing_if = "Option::is_none")]
    manual_working_infobase_identity: Option<ManualWorkingInfobaseIdentity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    manual_working_infobase_baseline: Option<ManualWorkingInfobaseBaseline>,
    origin_phase: TaskPhase,
    cancelled_phase: TaskPhase,
    relevant_advance_phase: TaskPhase,
    post_reconcile_phase: TaskPhase,
    phase_evidence_digest: Sha256Digest,
}

impl contract_digest_record_sealed::Sealed for SupportActionDigestRecord {}
impl ContractDigestRecord for SupportActionDigestRecord {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SupportActionAuthorizationAuthority {
    digest_record: SupportActionDigestRecord,
    support_action_digest: Sha256Digest,
    expected_support_graph_digest: Sha256Digest,
}

impl SupportActionAuthorizationAuthority {
    pub(crate) fn reserved_original(
        inputs: SupportActionAuthorizationInputs,
        reserved_original_lease_capability_id: CapabilityRowId,
        manual_actor_lock_baseline_digest: Sha256Digest,
    ) -> Result<Self, SupportContractError> {
        if inputs.manual_actor_username != inputs.reserved_integration_username
            || inputs
                .expected_manual_working_infobase_lease_capability_id
                .is_some()
        {
            return Err(SupportContractError(
                "reserved-original actor or working-IB capability binding is invalid",
            ));
        }
        if !inputs
            .support_recovery_distribution_coverage
            .distributions()
            .matches_manual_binding(
                ManualSupportTargetMode::ReservedOriginal,
                &inputs.manual_actor_username,
                None,
            )
        {
            return Err(SupportContractError(
                "recovery-distribution handoff disagrees with reserved-original binding",
            ));
        }
        Self::build(
            inputs,
            ManualSupportTargetMode::ReservedOriginal,
            Some(reserved_original_lease_capability_id),
            Some(manual_actor_lock_baseline_digest),
            None,
            None,
        )
    }

    pub(crate) fn separate_working_infobase(
        inputs: SupportActionAuthorizationInputs,
        manual_working_infobase_identity: ManualWorkingInfobaseIdentity,
        manual_working_infobase_baseline: ManualWorkingInfobaseBaseline,
    ) -> Result<Self, SupportContractError> {
        if inputs.manual_actor_username == inputs.reserved_integration_username
            || manual_working_infobase_baseline.working_infobase_identity()
                != &manual_working_infobase_identity
            || manual_working_infobase_baseline.support_graph_digest()
                != &inputs.expected_support_graph_digest
            || inputs
                .expected_manual_working_infobase_lease_capability_id
                .as_ref()
                != Some(manual_working_infobase_baseline.exclusive_lease_capability_id())
        {
            return Err(SupportContractError(
                "separate-mode actor, graph, identity, or profile lease binding is invalid",
            ));
        }
        if !inputs
            .support_recovery_distribution_coverage
            .distributions()
            .matches_manual_binding(
                ManualSupportTargetMode::SeparateWorkingInfobase,
                &inputs.manual_actor_username,
                Some(&manual_working_infobase_identity),
            )
        {
            return Err(SupportContractError(
                "recovery-distribution handoff disagrees with separate working-IB binding",
            ));
        }
        Self::build(
            inputs,
            ManualSupportTargetMode::SeparateWorkingInfobase,
            None,
            None,
            Some(manual_working_infobase_identity),
            Some(manual_working_infobase_baseline),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn build(
        inputs: SupportActionAuthorizationInputs,
        manual_target_mode: ManualSupportTargetMode,
        reserved_original_lease_capability_id: Option<CapabilityRowId>,
        manual_actor_lock_baseline_digest: Option<Sha256Digest>,
        manual_working_infobase_identity: Option<ManualWorkingInfobaseIdentity>,
        manual_working_infobase_baseline: Option<ManualWorkingInfobaseBaseline>,
    ) -> Result<Self, SupportContractError> {
        if !inputs.phase_binding.valid_for(inputs.purpose) {
            return Err(SupportContractError(
                "support-action phase binding disagrees with its purpose",
            ));
        }
        if inputs.authorized_transitions.is_empty() {
            return Err(SupportContractError(
                "support action authorization requires non-empty transitions",
            ));
        }
        if !inputs
            .support_recovery_distribution_coverage
            .covers_transitions(&inputs.authorized_transitions)
        {
            return Err(SupportContractError(
                "support action transitions escape proven root-reachable recovery coverage",
            ));
        }
        if inputs.expected_support_graph_digest
            != *inputs
                .support_recovery_distribution_coverage
                .support_graph_digest()
        {
            return Err(SupportContractError(
                "support action graph digest disagrees with recovery-coverage authority",
            ));
        }
        let recovery_distributions = inputs
            .support_recovery_distribution_coverage
            .distributions();
        if recovery_distributions.is_empty() {
            return Err(SupportContractError(
                "support action authorization requires complete recovery distributions",
            ));
        }
        let recovery_distribution_values = recovery_distributions.as_slice().to_vec();
        let recovery_distribution_set_digest = recovery_distributions.digest().clone();
        let authorized_transitions_digest = contract_digest(
            &AuthorizedSupportTransitionsDigestRecord(inputs.authorized_transitions.clone()),
            "authorized support-transition digest failed",
        )?;
        let digest_record = SupportActionDigestRecord {
            support_action_id: inputs.support_action_id,
            purpose: inputs.purpose,
            support_gate_id: inputs.support_gate_id,
            support_gate_digest: inputs.support_gate_digest,
            candidate_set_digest: inputs.candidate_set_digest,
            expected_before_history_cursor: inputs.expected_before_history_cursor,
            expected_relevant_baseline_digest: inputs.expected_relevant_baseline_digest,
            arming_required: TrueLiteral,
            authorized_transitions: inputs.authorized_transitions,
            authorized_transitions_digest,
            support_recovery_distributions: recovery_distribution_values,
            support_recovery_distribution_set_digest: recovery_distribution_set_digest,
            manual_target_mode,
            reserved_integration_username: inputs.reserved_integration_username,
            reserved_original_identity_digest: inputs.reserved_original_identity_digest,
            reserved_original_lease_capability_id,
            expected_original_fingerprint: inputs.expected_original_fingerprint,
            manual_actor_username: inputs.manual_actor_username,
            manual_actor_lock_baseline_digest,
            manual_working_infobase_identity,
            manual_working_infobase_baseline,
            origin_phase: inputs.phase_binding.origin_phase,
            cancelled_phase: inputs.phase_binding.cancelled_phase,
            relevant_advance_phase: inputs.phase_binding.relevant_advance_phase,
            post_reconcile_phase: inputs.phase_binding.post_reconcile_phase,
            phase_evidence_digest: inputs.phase_binding.phase_evidence_digest,
        };
        let support_action_digest =
            contract_digest(&digest_record, "immutable support-action digest failed")?;
        Ok(Self {
            digest_record,
            support_action_digest,
            expected_support_graph_digest: inputs.expected_support_graph_digest,
        })
    }

    pub(crate) const fn support_action_id(&self) -> &UnicaId {
        &self.digest_record.support_action_id
    }

    pub(crate) const fn support_action_digest(&self) -> &Sha256Digest {
        &self.support_action_digest
    }

    pub(crate) const fn support_gate_id(&self) -> &UnicaId {
        &self.digest_record.support_gate_id
    }

    pub(crate) const fn support_gate_digest(&self) -> &Sha256Digest {
        &self.digest_record.support_gate_digest
    }

    pub(crate) const fn candidate_set_digest(&self) -> &Sha256Digest {
        &self.digest_record.candidate_set_digest
    }

    pub(crate) const fn expected_before_history_cursor(&self) -> &RepositoryHistoryCursor {
        &self.digest_record.expected_before_history_cursor
    }

    pub(crate) const fn expected_relevant_baseline_digest(&self) -> &Sha256Digest {
        &self.digest_record.expected_relevant_baseline_digest
    }

    pub(crate) const fn authorized_transitions(&self) -> &SupportTransitions {
        &self.digest_record.authorized_transitions
    }

    pub(crate) const fn support_recovery_distribution_set_digest(&self) -> &Sha256Digest {
        &self.digest_record.support_recovery_distribution_set_digest
    }

    pub(crate) const fn expected_original_fingerprint(&self) -> &Sha256Digest {
        &self.digest_record.expected_original_fingerprint
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) enum SupportActionState {
    AwaitingArm,
    Armed,
    Consumed,
    Cancelled,
    FrozenForRecovery,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) enum SupportActionFreezeKind {
    ArmedAction,
    PreArmCancellationEffect,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SupportActionAuthorizationRecord {
    immutable: SupportActionDigestRecord,
    support_action_digest: Sha256Digest,
    expected_support_graph_digest: Sha256Digest,
    arming_receipt: Option<SupportActionArmingReceipt>,
    state: SupportActionState,
    freeze_kind: Option<SupportActionFreezeKind>,
}

/// Opaque, state-checked inputs for the external acquire-root instruction.
///
/// This projection can be minted only from a published action which is still
/// awaiting its arming receipt.  Keeping the fields private prevents an
/// instruction constructor from accepting an arbitrary mode/actor splice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AwaitingSupportInstructionProjection {
    support_action_id: UnicaId,
    purpose: SupportActionPurpose,
    support_action_digest: Sha256Digest,
    support_gate_id: UnicaId,
    support_gate_digest: Sha256Digest,
    expected_before_history_cursor: RepositoryHistoryCursor,
    manual_target_mode: ManualSupportTargetMode,
    manual_actor_username: RepositoryUsername,
    working_infobase_identity: Option<ManualWorkingInfobaseIdentity>,
    authorized_transitions: SupportTransitions,
}

impl AwaitingSupportInstructionProjection {
    pub(crate) const fn support_action_id(&self) -> &UnicaId {
        &self.support_action_id
    }

    pub(crate) const fn purpose(&self) -> SupportActionPurpose {
        self.purpose
    }

    pub(crate) const fn support_action_digest(&self) -> &Sha256Digest {
        &self.support_action_digest
    }

    pub(crate) const fn support_gate_id(&self) -> &UnicaId {
        &self.support_gate_id
    }

    pub(crate) const fn support_gate_digest(&self) -> &Sha256Digest {
        &self.support_gate_digest
    }

    pub(crate) const fn expected_before_history_cursor(&self) -> &RepositoryHistoryCursor {
        &self.expected_before_history_cursor
    }

    pub(crate) const fn manual_target_mode(&self) -> ManualSupportTargetMode {
        self.manual_target_mode
    }

    pub(crate) const fn manual_actor_username(&self) -> &RepositoryUsername {
        &self.manual_actor_username
    }

    pub(crate) const fn working_infobase_identity(&self) -> Option<&ManualWorkingInfobaseIdentity> {
        self.working_infobase_identity.as_ref()
    }

    pub(crate) const fn authorized_transitions(&self) -> &SupportTransitions {
        &self.authorized_transitions
    }
}

/// Opaque, post-arm inputs for the manual support instruction.
///
/// The arming identifiers/cursor are read from the accepted receipt and the
/// remaining members are copied from the immutable action.  Consequently no
/// caller can combine a receipt from one action with another action's manual
/// mode, actor, working infobase, or transition set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ArmedSupportInstructionProjection {
    support_action_id: UnicaId,
    purpose: SupportActionPurpose,
    support_action_digest: Sha256Digest,
    arming_receipt_id: UnicaId,
    arming_receipt_digest: Sha256Digest,
    arming_cursor: RepositoryHistoryCursor,
    manual_target_mode: ManualSupportTargetMode,
    manual_actor_username: RepositoryUsername,
    working_infobase_identity: Option<ManualWorkingInfobaseIdentity>,
    authorized_transitions: SupportTransitions,
}

impl ArmedSupportInstructionProjection {
    pub(crate) const fn support_action_id(&self) -> &UnicaId {
        &self.support_action_id
    }

    pub(crate) const fn purpose(&self) -> SupportActionPurpose {
        self.purpose
    }

    pub(crate) const fn support_action_digest(&self) -> &Sha256Digest {
        &self.support_action_digest
    }

    pub(crate) const fn arming_receipt_id(&self) -> &UnicaId {
        &self.arming_receipt_id
    }

    pub(crate) const fn arming_receipt_digest(&self) -> &Sha256Digest {
        &self.arming_receipt_digest
    }

    pub(crate) const fn arming_cursor(&self) -> &RepositoryHistoryCursor {
        &self.arming_cursor
    }

    pub(crate) const fn manual_target_mode(&self) -> ManualSupportTargetMode {
        self.manual_target_mode
    }

    pub(crate) const fn manual_actor_username(&self) -> &RepositoryUsername {
        &self.manual_actor_username
    }

    pub(crate) const fn working_infobase_identity(&self) -> Option<&ManualWorkingInfobaseIdentity> {
        self.working_infobase_identity.as_ref()
    }

    pub(crate) const fn authorized_transitions(&self) -> &SupportTransitions {
        &self.authorized_transitions
    }
}

struct SupportActionArmingBinding<'a> {
    support_action_id: &'a UnicaId,
    support_action_digest: &'a Sha256Digest,
    expected_before_history_cursor: &'a RepositoryHistoryCursor,
    support_gate_digest: &'a Sha256Digest,
    candidate_set_digest: &'a Sha256Digest,
    expected_relevant_baseline_digest: &'a Sha256Digest,
    support_graph_digest: &'a Sha256Digest,
    support_recovery_distribution_set_digest: &'a Sha256Digest,
    original_fingerprint: &'a Sha256Digest,
    manual_target_mode: ManualSupportTargetMode,
}

impl<'a> SupportActionArmingBinding<'a> {
    fn from_receipt(receipt: &'a SupportActionArmingReceipt) -> Self {
        Self {
            support_action_id: receipt.support_action_id(),
            support_action_digest: receipt.support_action_digest(),
            expected_before_history_cursor: receipt.expected_before_history_cursor(),
            support_gate_digest: receipt.support_gate_digest(),
            candidate_set_digest: receipt.candidate_set_digest(),
            expected_relevant_baseline_digest: receipt.expected_relevant_baseline_digest(),
            support_graph_digest: receipt.support_graph_digest(),
            support_recovery_distribution_set_digest: receipt
                .support_recovery_distribution_set_digest(),
            original_fingerprint: receipt.original_fingerprint(),
            manual_target_mode: receipt.manual_target_mode(),
        }
    }

    fn matches(&self, record: &SupportActionAuthorizationRecord) -> bool {
        self.support_action_id == &record.immutable.support_action_id
            && self.support_action_digest == &record.support_action_digest
            && self.expected_before_history_cursor
                == &record.immutable.expected_before_history_cursor
            && self.support_gate_digest == &record.immutable.support_gate_digest
            && self.candidate_set_digest == &record.immutable.candidate_set_digest
            && self.expected_relevant_baseline_digest
                == &record.immutable.expected_relevant_baseline_digest
            && self.support_graph_digest == &record.expected_support_graph_digest
            && self.support_recovery_distribution_set_digest
                == &record.immutable.support_recovery_distribution_set_digest
            && self.original_fingerprint == &record.immutable.expected_original_fingerprint
            && self.manual_target_mode == record.immutable.manual_target_mode
    }
}

impl Serialize for SupportActionAuthorizationRecord {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let immutable = &self.immutable;
        let optional_count = usize::from(immutable.reserved_original_lease_capability_id.is_some())
            + usize::from(immutable.manual_actor_lock_baseline_digest.is_some())
            + usize::from(immutable.manual_working_infobase_identity.is_some())
            + usize::from(immutable.manual_working_infobase_baseline.is_some())
            + usize::from(self.arming_receipt.is_some())
            + usize::from(self.freeze_kind.is_some());
        let mut map = serializer.serialize_map(Some(27 + optional_count))?;
        map.serialize_entry("supportActionId", &immutable.support_action_id)?;
        map.serialize_entry("purpose", &immutable.purpose)?;
        map.serialize_entry("supportActionDigest", &self.support_action_digest)?;
        map.serialize_entry("supportGateId", &immutable.support_gate_id)?;
        map.serialize_entry("supportGateDigest", &immutable.support_gate_digest)?;
        map.serialize_entry("candidateSetDigest", &immutable.candidate_set_digest)?;
        map.serialize_entry(
            "expectedBeforeHistoryCursor",
            &immutable.expected_before_history_cursor,
        )?;
        map.serialize_entry(
            "expectedRelevantBaselineDigest",
            &immutable.expected_relevant_baseline_digest,
        )?;
        map.serialize_entry("armingRequired", &immutable.arming_required)?;
        map.serialize_entry("authorizedTransitions", &immutable.authorized_transitions)?;
        map.serialize_entry(
            "authorizedTransitionsDigest",
            &immutable.authorized_transitions_digest,
        )?;
        map.serialize_entry(
            "supportRecoveryDistributions",
            &immutable.support_recovery_distributions,
        )?;
        map.serialize_entry(
            "supportRecoveryDistributionSetDigest",
            &immutable.support_recovery_distribution_set_digest,
        )?;
        map.serialize_entry("manualTargetMode", &immutable.manual_target_mode)?;
        map.serialize_entry(
            "reservedIntegrationUsername",
            &immutable.reserved_integration_username,
        )?;
        map.serialize_entry(
            "reservedOriginalIdentityDigest",
            &immutable.reserved_original_identity_digest,
        )?;
        if let Some(value) = &immutable.reserved_original_lease_capability_id {
            map.serialize_entry("reservedOriginalLeaseCapabilityId", value)?;
        }
        map.serialize_entry(
            "expectedOriginalFingerprint",
            &immutable.expected_original_fingerprint,
        )?;
        map.serialize_entry("manualActorUsername", &immutable.manual_actor_username)?;
        if let Some(value) = &immutable.manual_actor_lock_baseline_digest {
            map.serialize_entry("manualActorLockBaselineDigest", value)?;
        }
        if let Some(value) = &immutable.manual_working_infobase_identity {
            map.serialize_entry("manualWorkingInfobaseIdentity", value)?;
        }
        if let Some(value) = &immutable.manual_working_infobase_baseline {
            map.serialize_entry("manualWorkingInfobaseBaseline", value)?;
        }
        if let Some(value) = &self.arming_receipt {
            map.serialize_entry("armingReceipt", value)?;
        }
        map.serialize_entry("originPhase", &immutable.origin_phase)?;
        map.serialize_entry("cancelledPhase", &immutable.cancelled_phase)?;
        map.serialize_entry("relevantAdvancePhase", &immutable.relevant_advance_phase)?;
        map.serialize_entry("postReconcilePhase", &immutable.post_reconcile_phase)?;
        map.serialize_entry("phaseEvidenceDigest", &immutable.phase_evidence_digest)?;
        map.serialize_entry("state", &self.state)?;
        if let Some(value) = &self.freeze_kind {
            map.serialize_entry("freezeKind", value)?;
        }
        map.end()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(private_interfaces)]
pub(crate) enum ActiveSupportActionResumeHandle {
    AwaitingArm(SupportActionAuthorizationRecord),
    Armed(SupportActionAuthorizationRecord),
    FrozenArmedAction(SupportActionAuthorizationRecord),
    FrozenPreArmCancellationEffect(SupportActionAuthorizationRecord),
}

impl ActiveSupportActionResumeHandle {
    pub(crate) fn publish(
        authority: SupportActionAuthorizationAuthority,
    ) -> Result<Self, SupportContractError> {
        Ok(Self::AwaitingArm(SupportActionAuthorizationRecord {
            immutable: authority.digest_record,
            support_action_digest: authority.support_action_digest,
            expected_support_graph_digest: authority.expected_support_graph_digest,
            arming_receipt: None,
            state: SupportActionState::AwaitingArm,
            freeze_kind: None,
        }))
    }

    pub(crate) fn arm(
        self,
        receipt: SupportActionArmingReceipt,
    ) -> Result<Self, SupportContractError> {
        let Self::AwaitingArm(mut record) = self else {
            return Err(SupportContractError(
                "only awaiting support actions can arm",
            ));
        };
        if !SupportActionArmingBinding::from_receipt(&receipt).matches(&record) {
            return Err(SupportContractError(
                "arming receipt disagrees with immutable support authorization",
            ));
        }
        record.arming_receipt = Some(receipt);
        record.state = SupportActionState::Armed;
        Ok(Self::Armed(record))
    }

    pub(crate) fn awaiting_support_instruction_projection(
        &self,
    ) -> Option<AwaitingSupportInstructionProjection> {
        let Self::AwaitingArm(record) = self else {
            return None;
        };
        Some(AwaitingSupportInstructionProjection {
            support_action_id: record.immutable.support_action_id.clone(),
            purpose: record.immutable.purpose,
            support_action_digest: record.support_action_digest.clone(),
            support_gate_id: record.immutable.support_gate_id.clone(),
            support_gate_digest: record.immutable.support_gate_digest.clone(),
            expected_before_history_cursor: record.immutable.expected_before_history_cursor.clone(),
            manual_target_mode: record.immutable.manual_target_mode,
            manual_actor_username: record.immutable.manual_actor_username.clone(),
            working_infobase_identity: record.immutable.manual_working_infobase_identity.clone(),
            authorized_transitions: record.immutable.authorized_transitions.clone(),
        })
    }

    pub(crate) fn armed_support_instruction_projection(
        &self,
    ) -> Option<ArmedSupportInstructionProjection> {
        let Self::Armed(record) = self else {
            return None;
        };
        let receipt = record
            .arming_receipt
            .as_ref()
            .expect("armed support-action state always carries its accepted arming receipt");
        Some(ArmedSupportInstructionProjection {
            support_action_id: record.immutable.support_action_id.clone(),
            purpose: record.immutable.purpose,
            support_action_digest: record.support_action_digest.clone(),
            arming_receipt_id: receipt.arming_receipt_id().clone(),
            arming_receipt_digest: receipt.receipt_digest().clone(),
            arming_cursor: receipt.arming_cursor().clone(),
            manual_target_mode: record.immutable.manual_target_mode,
            manual_actor_username: record.immutable.manual_actor_username.clone(),
            working_infobase_identity: record.immutable.manual_working_infobase_identity.clone(),
            authorized_transitions: record.immutable.authorized_transitions.clone(),
        })
    }

    #[cfg(test)]
    pub(crate) fn freeze_prearm_cancellation_effect(self) -> Result<Self, SupportContractError> {
        let Self::AwaitingArm(mut record) = self else {
            return Err(SupportContractError(
                "pre-arm cancellation freeze requires an awaiting action",
            ));
        };
        record.state = SupportActionState::FrozenForRecovery;
        record.freeze_kind = Some(SupportActionFreezeKind::PreArmCancellationEffect);
        Ok(Self::FrozenPreArmCancellationEffect(record))
    }

    #[cfg(test)]
    pub(crate) fn freeze_armed_action(self) -> Result<Self, SupportContractError> {
        let Self::Armed(mut record) = self else {
            return Err(SupportContractError(
                "armed-action freeze requires an armed action",
            ));
        };
        record.state = SupportActionState::FrozenForRecovery;
        record.freeze_kind = Some(SupportActionFreezeKind::ArmedAction);
        Ok(Self::FrozenArmedAction(record))
    }

    #[cfg(test)]
    pub(crate) fn cancel(self) -> Result<TerminalSupportActionAuthorization, SupportContractError> {
        match self {
            Self::AwaitingArm(mut record) => {
                record.state = SupportActionState::Cancelled;
                Ok(TerminalSupportActionAuthorization::CancelledBeforeArm(
                    record,
                ))
            }
            Self::Armed(mut record) => {
                record.state = SupportActionState::Cancelled;
                Ok(TerminalSupportActionAuthorization::CancelledAfterArm(
                    record,
                ))
            }
            Self::FrozenArmedAction(_) | Self::FrozenPreArmCancellationEffect(_) => Err(
                SupportContractError("frozen support action requires recovery terminalization"),
            ),
        }
    }

    #[cfg(test)]
    pub(crate) fn consume(
        self,
    ) -> Result<TerminalSupportActionAuthorization, SupportContractError> {
        let Self::Armed(mut record) = self else {
            return Err(SupportContractError(
                "only armed support actions can be consumed",
            ));
        };
        record.state = SupportActionState::Consumed;
        Ok(TerminalSupportActionAuthorization::Consumed(record))
    }
}

impl Serialize for ActiveSupportActionResumeHandle {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::AwaitingArm(record)
            | Self::Armed(record)
            | Self::FrozenArmedAction(record)
            | Self::FrozenPreArmCancellationEffect(record) => record.serialize(serializer),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(private_interfaces)]
pub(crate) enum TerminalSupportActionAuthorization {
    Consumed(SupportActionAuthorizationRecord),
    CancelledBeforeArm(SupportActionAuthorizationRecord),
    CancelledAfterArm(SupportActionAuthorizationRecord),
}

impl Serialize for TerminalSupportActionAuthorization {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::Consumed(record)
            | Self::CancelledBeforeArm(record)
            | Self::CancelledAfterArm(record) => record.serialize(serializer),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SupportActionAuthorizationData {
    Active(ActiveSupportActionResumeHandle),
    Terminal(TerminalSupportActionAuthorization),
}

impl From<ActiveSupportActionResumeHandle> for SupportActionAuthorizationData {
    fn from(value: ActiveSupportActionResumeHandle) -> Self {
        Self::Active(value)
    }
}

impl From<TerminalSupportActionAuthorization> for SupportActionAuthorizationData {
    fn from(value: TerminalSupportActionAuthorization) -> Self {
        Self::Terminal(value)
    }
}

impl Serialize for SupportActionAuthorizationData {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::Active(value) => value.serialize(serializer),
            Self::Terminal(value) => value.serialize(serializer),
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum AuthorizationSchemaMode {
    ReservedOriginal,
    SeparateWorkingInfobase,
}

#[derive(Debug, Clone, Copy)]
struct AuthorizationSchemaBranch {
    mode: AuthorizationSchemaMode,
    state: &'static str,
    arming_receipt: bool,
    freeze_kind: Option<&'static str>,
}

const ACTIVE_SCHEMA_BRANCHES: &[AuthorizationSchemaBranch] = &[
    AuthorizationSchemaBranch {
        mode: AuthorizationSchemaMode::ReservedOriginal,
        state: "awaitingArm",
        arming_receipt: false,
        freeze_kind: None,
    },
    AuthorizationSchemaBranch {
        mode: AuthorizationSchemaMode::SeparateWorkingInfobase,
        state: "awaitingArm",
        arming_receipt: false,
        freeze_kind: None,
    },
    AuthorizationSchemaBranch {
        mode: AuthorizationSchemaMode::ReservedOriginal,
        state: "armed",
        arming_receipt: true,
        freeze_kind: None,
    },
    AuthorizationSchemaBranch {
        mode: AuthorizationSchemaMode::SeparateWorkingInfobase,
        state: "armed",
        arming_receipt: true,
        freeze_kind: None,
    },
    AuthorizationSchemaBranch {
        mode: AuthorizationSchemaMode::ReservedOriginal,
        state: "frozenForRecovery",
        arming_receipt: true,
        freeze_kind: Some("armedAction"),
    },
    AuthorizationSchemaBranch {
        mode: AuthorizationSchemaMode::SeparateWorkingInfobase,
        state: "frozenForRecovery",
        arming_receipt: true,
        freeze_kind: Some("armedAction"),
    },
    AuthorizationSchemaBranch {
        mode: AuthorizationSchemaMode::ReservedOriginal,
        state: "frozenForRecovery",
        arming_receipt: false,
        freeze_kind: Some("preArmCancellationEffect"),
    },
    AuthorizationSchemaBranch {
        mode: AuthorizationSchemaMode::SeparateWorkingInfobase,
        state: "frozenForRecovery",
        arming_receipt: false,
        freeze_kind: Some("preArmCancellationEffect"),
    },
];

const TERMINAL_SCHEMA_BRANCHES: &[AuthorizationSchemaBranch] = &[
    AuthorizationSchemaBranch {
        mode: AuthorizationSchemaMode::ReservedOriginal,
        state: "consumed",
        arming_receipt: true,
        freeze_kind: None,
    },
    AuthorizationSchemaBranch {
        mode: AuthorizationSchemaMode::SeparateWorkingInfobase,
        state: "consumed",
        arming_receipt: true,
        freeze_kind: None,
    },
    AuthorizationSchemaBranch {
        mode: AuthorizationSchemaMode::ReservedOriginal,
        state: "cancelled",
        arming_receipt: false,
        freeze_kind: None,
    },
    AuthorizationSchemaBranch {
        mode: AuthorizationSchemaMode::SeparateWorkingInfobase,
        state: "cancelled",
        arming_receipt: false,
        freeze_kind: None,
    },
    AuthorizationSchemaBranch {
        mode: AuthorizationSchemaMode::ReservedOriginal,
        state: "cancelled",
        arming_receipt: true,
        freeze_kind: None,
    },
    AuthorizationSchemaBranch {
        mode: AuthorizationSchemaMode::SeparateWorkingInfobase,
        state: "cancelled",
        arming_receipt: true,
        freeze_kind: None,
    },
];

fn schema_value<T: JsonSchema>(generator: &mut SchemaGenerator) -> Value {
    serde_json::to_value(generator.subschema_for::<T>())
        .expect("typed contract schema is serializable")
}

fn immutable_properties(
    generator: &mut SchemaGenerator,
    mode: AuthorizationSchemaMode,
) -> (Map<String, Value>, Vec<Value>) {
    let mut properties = Map::new();
    let mut required = Vec::new();
    macro_rules! property {
        ($wire:literal, $type:ty) => {{
            properties.insert($wire.to_owned(), schema_value::<$type>(generator));
            required.push(Value::String($wire.to_owned()));
        }};
        ($wire:literal, const $value:expr) => {{
            properties.insert(
                $wire.to_owned(),
                serde_json::json!({ "type": "string", "const": $value }),
            );
            required.push(Value::String($wire.to_owned()));
        }};
    }
    property!("supportActionId", UnicaId);
    property!("purpose", SupportActionPurpose);
    property!("supportGateId", UnicaId);
    property!("supportGateDigest", Sha256Digest);
    property!("candidateSetDigest", Sha256Digest);
    property!("expectedBeforeHistoryCursor", RepositoryHistoryCursor);
    property!("expectedRelevantBaselineDigest", Sha256Digest);
    properties.insert(
        "armingRequired".to_owned(),
        serde_json::json!({ "type": "boolean", "const": true }),
    );
    required.push(Value::String("armingRequired".to_owned()));
    property!("authorizedTransitions", SupportTransitions);
    property!("authorizedTransitionsDigest", Sha256Digest);
    properties.insert(
        "supportRecoveryDistributions".to_owned(),
        serde_json::to_value(json_schema!({
            "type": "array",
            "maxItems": 1024,
            "items": generator.subschema_for::<SupportRecoveryDistributionEvidence>(),
        }))
        .expect("array schema is serializable"),
    );
    required.push(Value::String("supportRecoveryDistributions".to_owned()));
    property!("supportRecoveryDistributionSetDigest", Sha256Digest);
    match mode {
        AuthorizationSchemaMode::ReservedOriginal => {
            property!("manualTargetMode", const "reservedOriginal");
        }
        AuthorizationSchemaMode::SeparateWorkingInfobase => {
            property!("manualTargetMode", const "separateWorkingInfobase");
        }
    }
    property!("reservedIntegrationUsername", RepositoryUsername);
    property!("reservedOriginalIdentityDigest", Sha256Digest);
    property!("expectedOriginalFingerprint", Sha256Digest);
    property!("manualActorUsername", RepositoryUsername);
    match mode {
        AuthorizationSchemaMode::ReservedOriginal => {
            property!("reservedOriginalLeaseCapabilityId", CapabilityRowId);
            property!("manualActorLockBaselineDigest", Sha256Digest);
        }
        AuthorizationSchemaMode::SeparateWorkingInfobase => {
            property!(
                "manualWorkingInfobaseIdentity",
                ManualWorkingInfobaseIdentity
            );
            property!(
                "manualWorkingInfobaseBaseline",
                ManualWorkingInfobaseBaseline
            );
        }
    }
    property!("originPhase", TaskPhase);
    property!("cancelledPhase", TaskPhase);
    property!("relevantAdvancePhase", TaskPhase);
    property!("postReconcilePhase", TaskPhase);
    property!("phaseEvidenceDigest", Sha256Digest);
    (properties, required)
}

fn closed_object_schema(properties: Map<String, Value>, required: Vec<Value>) -> Schema {
    let mut object = Map::new();
    object.insert("type".to_owned(), Value::String("object".to_owned()));
    object.insert("properties".to_owned(), Value::Object(properties));
    object.insert("required".to_owned(), Value::Array(required));
    object.insert("additionalProperties".to_owned(), Value::Bool(false));
    Schema::from(object)
}

fn digest_branch_schema(generator: &mut SchemaGenerator, mode: AuthorizationSchemaMode) -> Schema {
    let (properties, required) = immutable_properties(generator, mode);
    closed_object_schema(properties, required)
}

fn authorization_branch_schema(
    generator: &mut SchemaGenerator,
    branch: AuthorizationSchemaBranch,
) -> Schema {
    let (mut properties, mut required) = immutable_properties(generator, branch.mode);
    properties.insert(
        "supportActionDigest".to_owned(),
        schema_value::<Sha256Digest>(generator),
    );
    required.push(Value::String("supportActionDigest".to_owned()));
    properties.insert(
        "state".to_owned(),
        serde_json::json!({ "type": "string", "const": branch.state }),
    );
    required.push(Value::String("state".to_owned()));
    if branch.arming_receipt {
        properties.insert(
            "armingReceipt".to_owned(),
            schema_value::<SupportActionArmingReceipt>(generator),
        );
        required.push(Value::String("armingReceipt".to_owned()));
    }
    if let Some(freeze_kind) = branch.freeze_kind {
        properties.insert(
            "freezeKind".to_owned(),
            serde_json::json!({ "type": "string", "const": freeze_kind }),
        );
        required.push(Value::String("freezeKind".to_owned()));
    }
    closed_object_schema(properties, required)
}

impl JsonSchema for SupportActionDigestRecord {
    fn schema_name() -> Cow<'static, str> {
        "SupportActionDigestRecord".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            digest_branch_schema(generator, AuthorizationSchemaMode::ReservedOriginal),
            digest_branch_schema(generator, AuthorizationSchemaMode::SeparateWorkingInfobase),
        ])
    }
}

fn branches_schema(
    generator: &mut SchemaGenerator,
    branches: &[AuthorizationSchemaBranch],
) -> Schema {
    one_of_schema(
        branches
            .iter()
            .copied()
            .map(|branch| authorization_branch_schema(generator, branch))
            .collect(),
    )
}

impl JsonSchema for ActiveSupportActionResumeHandle {
    fn schema_name() -> Cow<'static, str> {
        "ActiveSupportActionResumeHandle".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        branches_schema(generator, ACTIVE_SCHEMA_BRANCHES)
    }
}

impl JsonSchema for TerminalSupportActionAuthorization {
    fn schema_name() -> Cow<'static, str> {
        "TerminalSupportActionAuthorization".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        branches_schema(generator, TERMINAL_SCHEMA_BRANCHES)
    }
}

impl JsonSchema for SupportActionAuthorizationData {
    fn schema_name() -> Cow<'static, str> {
        "SupportActionAuthorizationData".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        let mut branches =
            Vec::with_capacity(ACTIVE_SCHEMA_BRANCHES.len() + TERMINAL_SCHEMA_BRANCHES.len());
        branches.extend_from_slice(ACTIVE_SCHEMA_BRANCHES);
        branches.extend_from_slice(TERMINAL_SCHEMA_BRANCHES);
        branches_schema(generator, &branches)
    }
}
