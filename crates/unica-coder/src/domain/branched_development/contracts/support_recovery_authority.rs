//! Opaque approval boundary for armed support recovery.

use super::instructions::{
    CleanManualWorkingInfobaseInstruction, CloseReservedOriginalDesignerInstruction,
    InstructionContractError, ReleaseRepositoryLocksInstruction, SupportConflictInstruction,
    SupportContentRestoration, SupportCorrectiveInstruction, SupportCorrectiveInstructionAuthority,
    SupportCorrectiveLockClosureResolver, SupportEvidenceInstruction,
    SupportRecoveryExternalActionRef, SupportRecoveryTransition,
};
use super::recovery::{
    ArmedSupportRecoveryPlanProjection, ManualWorkingInfobaseAvailablePostconditionObservation,
    RecoveryAction, RecoveryPlanStatus, ReservedOriginalAvailablePostconditionObservation,
    SupportRecoveryActionCatalogAuthority, SupportRecoveryEvidencePostconditionObservation,
    SupportRecoveryExternalWaitAuthority, SupportRecoveryHistoryEvidence,
    SupportRecoveryLockReleasePostconditionObservation, SupportRecoveryVersionObservations,
};
use super::repository::{
    DeferredRepositoryAdvance, RepositoryHistoryCursor, RepositoryHistoryPartitionClassification,
    RepositoryOwnerIdentity, RepositoryTargetIdentity, RepositoryTargetStateRef,
    SelectiveRepositoryUpdatePlan, SelectiveRepositoryUpdateProof, SelectiveRepositoryUpdateScope,
    SupportRecoverySelectiveUpdateExecutionObservation,
    SupportRecoverySelectiveUpdatePlanObservation, ValidatedRepositoryHistoryPartition,
    ValidatedSupportRecoveryHistoryEntryRef,
};
use super::scalars::{RepositoryTargetDisplay, RepositoryUsername, RequiredNullable};
use super::support::{
    ActiveSupportActionResumeHandle, ArmedSupportInstructionProjection,
    FrozenArmedSupportRecoveryBinding, FrozenSupportRecoveryAuthorizationProjection,
    ManualActorLockInventoryProof, ManualSupportTargetMode, ReservedOriginalLeaseStopEvidence,
    ReservedOriginalTerminalizationProof, SupportObservationCorrectiveProjection,
    SupportObservationFrozenActionClaim, SupportPrerequisiteVersionObservation,
    SupportRecoveryDisposition, SupportRecoveryDistributionHandoff, SupportRecoveryDistributionSet,
    SupportRecoveryHandoffRevalidation,
};
use super::support_terminalization::{
    CompletedSupportRecoveryAuthorizationOutcome, ManualWorkingInfobaseClosureExecutionAuthority,
    ManualWorkingInfobaseClosurePlan, ManualWorkingInfobaseClosurePlanAuthority,
    ManualWorkingInfobaseClosureProof, ManualWorkingInfobaseStopAuthority,
    ManualWorkingInfobaseStopEvidence, SupportRecoveryAcquiredLockTargets,
    SupportRecoveryAuthorizationTerminalizationReceipt, SupportRecoveryFinalizationPlan,
    SupportRecoveryFinalizationPlanAuthority, SupportRecoveryGuardAuthority,
    SupportRecoveryGuardPlanAuthority, SupportRecoveryGuardProof,
    SupportRecoveryReleasedLockTargets, SupportTerminalizationContractError,
};
use crate::domain::branched_development::canonical_json::{
    canonical_contract_digest, contract_digest_record_sealed, ContractDigestRecord,
};
use crate::domain::branched_development::{CapabilityRowId, Sha256Digest, TaskPhase, UnicaId};
use serde::Serialize;
use std::fmt;
use std::sync::Arc;

/// Unforgeable capability shared only with the Task-9 contract modules.
///
/// The type is crate-visible so constructors can require it, but its field and
/// constructor stay private to this module. Consequently a raw Task-9 value is
/// not a production mint unless an approved support-recovery authority invokes
/// that constructor itself.
#[derive(Debug, Clone)]
struct SupportRecoveryAuthorityLineage {
    identity: Arc<()>,
}

impl SupportRecoveryAuthorityLineage {
    fn new() -> Self {
        Self {
            identity: Arc::new(()),
        }
    }

    fn belongs_to(&self, token: &SupportRecoveryAuthorityToken) -> bool {
        self == &token.lineage
    }
}

impl PartialEq for SupportRecoveryAuthorityLineage {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.identity, &other.identity)
    }
}

impl Eq for SupportRecoveryAuthorityLineage {}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct SupportRecoveryAuthorityToken {
    lineage: SupportRecoveryAuthorityLineage,
}

impl SupportRecoveryAuthorityToken {
    fn new() -> Self {
        Self {
            lineage: SupportRecoveryAuthorityLineage::new(),
        }
    }

    fn lineage(&self) -> SupportRecoveryAuthorityLineage {
        self.lineage.clone()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SupportRecoveryAuthorityError(&'static str);

impl fmt::Display for SupportRecoveryAuthorityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.0)
    }
}

impl std::error::Error for SupportRecoveryAuthorityError {}

impl From<InstructionContractError> for SupportRecoveryAuthorityError {
    fn from(_: InstructionContractError) -> Self {
        Self("approved support corrective instruction is invalid")
    }
}

impl From<SupportTerminalizationContractError> for SupportRecoveryAuthorityError {
    fn from(_: SupportTerminalizationContractError) -> Self {
        Self("approved support recovery terminalization evidence is invalid")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct SupportHistorySemanticBindingRecord {
    repository_version: super::scalars::RepositoryVersion,
    partition_classification: RepositoryHistoryPartitionClassification,
    root_delta_digest: RequiredNullable<Sha256Digest>,
    content_delta_digest: RequiredNullable<Sha256Digest>,
    classification_digest: RequiredNullable<Sha256Digest>,
    external_support_disjointness_digest: RequiredNullable<Sha256Digest>,
    corrective_instruction_digest: RequiredNullable<Sha256Digest>,
    non_conflicting_concurrent_evidence_digest: RequiredNullable<Sha256Digest>,
}

impl contract_digest_record_sealed::Sealed for SupportHistorySemanticBindingRecord {}
impl ContractDigestRecord for SupportHistorySemanticBindingRecord {}

fn nullable<T>(value: Option<T>) -> RequiredNullable<T> {
    value
        .map(RequiredNullable::value)
        .unwrap_or_else(RequiredNullable::null)
}

fn semantic_binding_digest(
    observation: &SupportPrerequisiteVersionObservation,
) -> Result<Sha256Digest, SupportRecoveryAuthorityError> {
    let (
        partition_classification,
        root_delta_digest,
        content_delta_digest,
        classification_digest,
        external_support_disjointness_digest,
        corrective_instruction_digest,
    ) = if let Some(projection) = observation.task8_mapping_projection() {
        (
            projection.partition_classification(),
            projection.root_delta_digest(),
            projection.content_delta_digest(),
            projection.classification_digest(),
            projection.external_support_disjointness_digest(),
            None,
        )
    } else {
        match observation
            .task9_corrective_projection()
            .ok_or(SupportRecoveryAuthorityError(
                "support observation cannot bind a history-partition semantic delta",
            ))? {
            SupportObservationCorrectiveProjection::ActionCorrection {
                root_delta_digest,
                content_delta_digest,
                corrective_instruction_digest,
                ..
            } => (
                RepositoryHistoryPartitionClassification::Corrective,
                Some(root_delta_digest),
                Some(content_delta_digest),
                observation.classification_digest().clone(),
                None,
                Some(corrective_instruction_digest),
            ),
            SupportObservationCorrectiveProjection::ExternalConflictCorrection {
                root_delta_digest,
                content_delta_digest,
                support_conflict_instruction_digest,
                ..
            } => (
                RepositoryHistoryPartitionClassification::Corrective,
                Some(root_delta_digest),
                Some(content_delta_digest),
                observation.classification_digest().clone(),
                None,
                Some(support_conflict_instruction_digest),
            ),
        }
    };
    canonical_contract_digest(
        &SupportHistorySemanticBindingRecord {
            repository_version: observation.repository_version().clone(),
            partition_classification,
            root_delta_digest: nullable(root_delta_digest),
            content_delta_digest: nullable(content_delta_digest),
            classification_digest: RequiredNullable::value(classification_digest),
            external_support_disjointness_digest: nullable(external_support_disjointness_digest),
            corrective_instruction_digest: nullable(corrective_instruction_digest),
            non_conflicting_concurrent_evidence_digest: RequiredNullable::null(),
        },
        None,
    )
    .map_err(|_| SupportRecoveryAuthorityError("support history semantic binding digest failed"))
}

fn partition_has_exact_prefix(
    partition: &ValidatedRepositoryHistoryPartition,
    prefix: &ValidatedRepositoryHistoryPartition,
) -> Result<bool, SupportRecoveryAuthorityError> {
    Ok(partition.has_exact_entry_prefix(prefix))
}

#[allow(clippy::too_many_arguments)]
fn observation_claim_binds_frozen_action(
    observation: &SupportPrerequisiteVersionObservation,
    partition_classification: RepositoryHistoryPartitionClassification,
    authorization: &FrozenSupportRecoveryAuthorizationProjection,
    binding: &FrozenArmedSupportRecoveryBinding,
    corrective_source_action_id: Option<&UnicaId>,
    entry_index: usize,
    arming_prefix_len: usize,
    saw_first_post_arming_claim: &mut bool,
) -> bool {
    let receipt = binding.arming_receipt();
    if entry_index < arming_prefix_len {
        // The exact accepted arming prefix predates this action's manual
        // window. Its rows are retained one-to-one for audit but must never be
        // reinterpreted as claims about the newly frozen action.
        return corrective_source_action_id.is_none();
    }
    let is_root_or_support_entry = matches!(
        partition_classification,
        RepositoryHistoryPartitionClassification::AuthorizedSupport
            | RepositoryHistoryPartitionClassification::ExternalSupport
            | RepositoryHistoryPartitionClassification::PreArmExternal
            | RepositoryHistoryPartitionClassification::Invalid
            | RepositoryHistoryPartitionClassification::Corrective
    );
    match observation.frozen_action_claim() {
        SupportObservationFrozenActionClaim::Unscoped => {
            corrective_source_action_id.is_none()
                && (!is_root_or_support_entry || *saw_first_post_arming_claim)
        }
        SupportObservationFrozenActionClaim::ExactArmedAction {
            support_action_id,
            support_action_digest,
            arming_receipt_id,
            arming_receipt_digest,
            authorized_transitions_digest,
            manual_target_mode,
            working_infobase_identity,
            first_root_support_after_arming,
        } => {
            let exact = corrective_source_action_id.is_none()
                && entry_index >= arming_prefix_len
                && is_root_or_support_entry
                && !*saw_first_post_arming_claim
                && first_root_support_after_arming
                && support_action_id == authorization.support_action_id()
                && support_action_digest == authorization.support_action_digest()
                && arming_receipt_id == receipt.arming_receipt_id()
                && arming_receipt_digest == receipt.receipt_digest()
                && authorized_transitions_digest == binding.authorized_transitions_digest()
                && manual_target_mode == authorization.manual_target_mode()
                && working_infobase_identity == binding.manual_working_infobase_identity();
            if exact {
                *saw_first_post_arming_claim = true;
            }
            exact
        }
        SupportObservationFrozenActionClaim::ExactArmingReceipt {
            arming_receipt_id,
            arming_receipt_digest,
            manual_target_mode,
            working_infobase_identity,
            first_root_support_after_arming,
        } => {
            let expected_first = !*saw_first_post_arming_claim;
            let exact = corrective_source_action_id.is_none()
                && entry_index >= arming_prefix_len
                && is_root_or_support_entry
                && first_root_support_after_arming == expected_first
                && arming_receipt_id == receipt.arming_receipt_id()
                && arming_receipt_digest == receipt.receipt_digest()
                && manual_target_mode == authorization.manual_target_mode()
                && working_infobase_identity == binding.manual_working_infobase_identity();
            if exact && expected_first {
                *saw_first_post_arming_claim = true;
            }
            exact
        }
        SupportObservationFrozenActionClaim::CorrectiveSourceRequired => {
            entry_index >= arming_prefix_len
                && *saw_first_post_arming_claim
                && corrective_source_action_id == Some(authorization.support_action_id())
        }
        SupportObservationFrozenActionClaim::PendingAction => false,
    }
}

fn observations_bind_partition(
    partition: &ValidatedRepositoryHistoryPartition,
    observations: &[SupportRecoveryHistoryEvidence],
    authorization: &FrozenSupportRecoveryAuthorizationProjection,
    binding: &FrozenArmedSupportRecoveryBinding,
) -> Result<bool, SupportRecoveryAuthorityError> {
    if partition.entry_count() != observations.len() {
        return Ok(false);
    }
    let arming_prefix_len = binding.arming_receipt().history_partition().entry_count();
    let mut saw_first_post_arming_claim = false;
    for (entry_index, (entry, evidence)) in partition
        .support_recovery_entries()
        .zip(observations)
        .enumerate()
    {
        match (entry, evidence) {
            (
                ValidatedSupportRecoveryHistoryEntryRef::SupportObservation {
                    repository_version,
                    partition_classification,
                    semantic_delta_digest,
                    source_evidence_digest,
                    corrective_source_action_id,
                    ..
                },
                SupportRecoveryHistoryEvidence::SupportObservation(observation),
            ) => {
                if repository_version != observation.repository_version()
                    || source_evidence_digest != observation.classification_digest()
                    || semantic_delta_digest != &semantic_binding_digest(observation)?
                    || !observation_claim_binds_frozen_action(
                        observation,
                        partition_classification,
                        authorization,
                        binding,
                        corrective_source_action_id,
                        entry_index,
                        arming_prefix_len,
                        &mut saw_first_post_arming_claim,
                    )
                {
                    return Ok(false);
                }
            }
            (
                ValidatedSupportRecoveryHistoryEntryRef::NonConflicting {
                    repository_version,
                    source_evidence_digest,
                    evidence: validated_evidence,
                    ..
                },
                SupportRecoveryHistoryEvidence::NonConflictingConcurrent(evidence),
            ) => {
                if repository_version != evidence.repository_version()
                    || source_evidence_digest != evidence.evidence_digest()
                    || validated_evidence != evidence
                {
                    return Ok(false);
                }
            }
            (ValidatedSupportRecoveryHistoryEntryRef::Unsupported { .. }, _)
            | (ValidatedSupportRecoveryHistoryEntryRef::SupportObservation { .. }, _)
            | (ValidatedSupportRecoveryHistoryEntryRef::NonConflicting { .. }, _) => {
                return Ok(false)
            }
        }
    }
    Ok(partition.entry_count() == arming_prefix_len || saw_first_post_arming_claim)
}

fn desired_targets_bind_materialized_plan(
    finalization_plan: &SupportRecoveryFinalizationPlan,
    selective_plan: &SelectiveRepositoryUpdatePlan,
    history_partition: &ValidatedRepositoryHistoryPartition,
) -> Result<bool, SupportRecoveryAuthorityError> {
    if selective_plan.scope() != SelectiveRepositoryUpdateScope::RecoveryFinalization {
        return Ok(false);
    }
    let desired = finalization_plan.desired_targets().as_slice();
    let planned = selective_plan.planned_targets().as_slice();
    let support_locks = finalization_plan.lock_targets().as_slice();
    let selective_locks = selective_plan.lock_targets().as_slice();
    if desired.len() != planned.len()
        || support_locks.len() != selective_locks.len()
        || !support_locks
            .iter()
            .zip(selective_locks)
            .all(|(expected, actual)| expected.binds_repository_lock_target(actual.as_ref()))
    {
        return Ok(false);
    }

    let version_is_bound = |version: &super::scalars::RepositoryVersion| {
        version == history_partition.start_cursor().through_version()
            || history_partition.contains_repository_version(version)
    };

    for (desired_target, planned_target) in desired.iter().zip(planned.iter()) {
        let planned_target = planned_target.as_ref();
        if !desired_target.binds_repository_target_state(planned_target) {
            return Ok(false);
        }
        let target_version = match planned_target {
            RepositoryTargetStateRef::RootPresent {
                repository_version, ..
            }
            | RepositoryTargetStateRef::ObjectPresent {
                repository_version, ..
            } => repository_version,
            RepositoryTargetStateRef::ObjectAbsent {
                absence_established_at_version,
                ..
            } => absence_established_at_version,
        };
        if !version_is_bound(target_version) {
            return Ok(false);
        }
    }
    Ok(true)
}

fn phase_binding_is_exact(
    binding: &FrozenArmedSupportRecoveryBinding,
    projection: &ArmedSupportRecoveryPlanProjection,
) -> bool {
    match projection.support_recovery_disposition() {
        SupportRecoveryDisposition::RestoreThenReauthorize => {
            projection.planned_result_phase() == binding.cancelled_phase()
                && projection.support_late_relevant_result_phase()
                    == binding.relevant_advance_phase()
        }
        SupportRecoveryDisposition::PreserveExternalAndReauthorize => {
            projection.planned_result_phase() == binding.relevant_advance_phase()
                && projection.support_late_relevant_result_phase()
                    == binding.relevant_advance_phase()
        }
        SupportRecoveryDisposition::RestoreThenAbandon => {
            projection.planned_result_phase() == TaskPhase::AbandonmentReady
                && projection.support_late_relevant_result_phase() == TaskPhase::AbandonmentReady
        }
    }
}

fn closure_plan_binds_frozen_authorization(
    authorization: &FrozenSupportRecoveryAuthorizationProjection,
    binding: &FrozenArmedSupportRecoveryBinding,
    projection: &ArmedSupportRecoveryPlanProjection,
) -> bool {
    match authorization.manual_target_mode() {
        ManualSupportTargetMode::ReservedOriginal => {
            binding.manual_working_infobase_identity().is_none()
                && binding.manual_working_infobase_baseline().is_none()
                && projection.manual_working_infobase_closure_plan().is_none()
        }
        ManualSupportTargetMode::SeparateWorkingInfobase => {
            let (Some(identity), Some(baseline), Some(plan)) = (
                binding.manual_working_infobase_identity(),
                binding.manual_working_infobase_baseline(),
                projection.manual_working_infobase_closure_plan(),
            ) else {
                return false;
            };
            let materialized_cursor_is_bound =
                plan.working_infobase_base_cursor().is_none_or(|cursor| {
                    cursor == baseline.repository_base_cursor()
                        || projection
                            .support_history_partition()
                            .contains_cursor(cursor)
                });
            let baseline_map_is_bound = match (
                plan.working_infobase_base_cursor(),
                plan.recorded_object_version_map_digest(),
            ) {
                (Some(cursor), Some(map)) if cursor == baseline.repository_base_cursor() => {
                    map == baseline.recorded_object_version_map_digest()
                }
                (Some(_), Some(_)) | (None, None) => true,
                (Some(_), None) | (None, Some(_)) => false,
            };
            identity == baseline.working_infobase_identity()
                && plan.working_infobase_identity() == identity
                && plan.authorization_baseline_digest() == baseline.baseline_digest()
                && plan.exclusive_lease_capability_id() == baseline.exclusive_lease_capability_id()
                && plan.desired_support_graph_digest()
                    == projection
                        .support_recovery_finalization_plan()
                        .desired_support_graph_digest()
                && materialized_cursor_is_bound
                && baseline_map_is_bound
        }
    }
}

fn corrective_instruction_binds_plan(
    instruction: &SupportCorrectiveInstruction,
    authorization: &FrozenSupportRecoveryAuthorizationProjection,
    binding: &FrozenArmedSupportRecoveryBinding,
    projection: &ArmedSupportRecoveryPlanProjection,
) -> bool {
    corrective_instruction_binds_inputs(
        instruction,
        authorization,
        binding,
        projection.support_history_through_cursor(),
        projection.support_recovery_finalization_plan(),
    )
}

fn corrective_instruction_binds_inputs(
    instruction: &SupportCorrectiveInstruction,
    authorization: &FrozenSupportRecoveryAuthorizationProjection,
    binding: &FrozenArmedSupportRecoveryBinding,
    support_history_through_cursor: &RepositoryHistoryCursor,
    finalization: &SupportRecoveryFinalizationPlan,
) -> bool {
    let transitions_bind_frozen_action =
        instruction
            .required_root_transitions()
            .iter()
            .all(|transition| {
                if let Some(ordinary) = transition.ordinary_transition() {
                    return binding
                        .authorized_transitions()
                        .as_slice()
                        .iter()
                        .any(|authorized| ordinary.is_exact_inverse_of(authorized));
                }
                let Some((layer_id, artifact_id, handoff_id, capability_row_id)) =
                    transition.recovery_handoff_binding()
                else {
                    return false;
                };
                binding
                    .support_recovery_distributions()
                    .iter()
                    .any(|distribution| {
                        distribution.layer_id() == layer_id
                            && distribution.distribution_artifact_id() == artifact_id
                            && distribution.handoff().handoff_id() == handoff_id
                            && distribution.capability_row_id() == capability_row_id
                    })
            });
    instruction.support_action_id() == authorization.support_action_id()
        && transitions_bind_frozen_action
        && instruction.purpose() == binding.purpose()
        && instruction.manual_target_mode() == authorization.manual_target_mode()
        && instruction.repository_username() == authorization.manual_actor_username()
        && instruction.working_infobase_identity() == binding.manual_working_infobase_identity()
        && instruction.correction_base_cursor() == support_history_through_cursor
        && instruction.finalization_lock_targets() == finalization.lock_targets()
        && instruction.desired_support_graph_digest() == finalization.desired_support_graph_digest()
        && instruction.desired_repository_content_digest()
            == finalization.desired_repository_content_digest()
        && instruction.distribution_handoffs().iter().all(|handoff| {
            binding
                .support_recovery_distributions()
                .iter()
                .any(|distribution| distribution.handoff() == handoff)
        })
}

fn external_action_binds_plan(
    action: &super::instructions::SupportRecoveryExternalAction,
    authorization: &FrozenSupportRecoveryAuthorizationProjection,
    binding: &FrozenArmedSupportRecoveryBinding,
    projection: &ArmedSupportRecoveryPlanProjection,
    token: &SupportRecoveryAuthorityToken,
) -> bool {
    match action.as_ref() {
        SupportRecoveryExternalActionRef::Corrective(instruction) => {
            corrective_instruction_binds_plan(instruction, authorization, binding, projection)
        }
        SupportRecoveryExternalActionRef::ReleaseLocks(instruction) => projection
            .latest_support_recovery_guard_proof()
            .filter(|proof| {
                !proof.is_completed()
                    && proof.finalization_plan_digest()
                        == projection
                            .support_recovery_finalization_plan()
                            .plan_digest()
                    && proof.blocked_target_ref().is_some()
            })
            .and_then(|proof| {
                ReleaseRepositoryLocksInstruction::from_support_recovery_blocked_approved(
                    token, proof,
                )
                .ok()
            })
            .is_some_and(|expected| expected == *instruction),
        SupportRecoveryExternalActionRef::CleanWorkingInfobase(instruction) => {
            let (Some(plan), Some(stop)) = (
                projection.manual_working_infobase_closure_plan(),
                projection
                    .latest_support_recovery_guard_proof()
                    .and_then(SupportRecoveryGuardProof::manual_working_infobase_stop_evidence),
            ) else {
                return false;
            };
            authorization.manual_target_mode() == ManualSupportTargetMode::SeparateWorkingInfobase
                && CleanManualWorkingInfobaseInstruction::from_support_recovery_stop_approved(
                    token, plan, stop,
                )
                .is_ok_and(|expected| expected == *instruction)
        }
        SupportRecoveryExternalActionRef::CloseReservedOriginal(instruction) => {
            let Some(stop) = projection
                .latest_support_recovery_guard_proof()
                .and_then(SupportRecoveryGuardProof::reserved_original_lease_stop_evidence)
            else {
                return false;
            };
            let expected =
                CloseReservedOriginalDesignerInstruction::from_support_recovery_stop_approved(
                    token, stop,
                );
            authorization.manual_target_mode() == ManualSupportTargetMode::ReservedOriginal
                && projection.manual_working_infobase_closure_plan().is_none()
                && authorization
                    .reserved_original_lease_capability_id()
                    .is_some_and(|expected| instruction.exclusive_lease_capability_id() == expected)
                && instruction.reserved_original_identity_digest()
                    == authorization.reserved_original_identity_digest()
                && expected == *instruction
        }
        SupportRecoveryExternalActionRef::Conflict(instruction) => {
            instruction.required_final_baseline_digest()
                == projection
                    .support_recovery_finalization_plan()
                    .desired_support_graph_digest()
        }
        SupportRecoveryExternalActionRef::Evidence(instruction) => {
            // The evidence source capability is consumed before this sealed
            // catalog exists. At approval, the matching await action already
            // binds this exact digest; rehashing the typed instruction guards
            // against a projection that bypassed that catalog.
            projection
                .required_external_action()
                .is_some_and(|expected| {
                    expected == action
                        && instruction.support_evidence_instruction_digest()
                            == match expected.as_ref() {
                                SupportRecoveryExternalActionRef::Evidence(value) => {
                                    value.support_evidence_instruction_digest()
                                }
                                _ => return false,
                            }
                })
        }
    }
}

fn external_action_guard_proof_presence_is_exact(
    action: Option<&super::instructions::SupportRecoveryExternalAction>,
    proof: Option<&SupportRecoveryGuardProof>,
) -> bool {
    match action.map(|value| value.as_ref()) {
        None
        | Some(SupportRecoveryExternalActionRef::Corrective(_))
        | Some(SupportRecoveryExternalActionRef::Conflict(_))
        | Some(SupportRecoveryExternalActionRef::Evidence(_)) => proof.is_none(),
        Some(SupportRecoveryExternalActionRef::ReleaseLocks(_))
        | Some(SupportRecoveryExternalActionRef::CleanWorkingInfobase(_))
        | Some(SupportRecoveryExternalActionRef::CloseReservedOriginal(_)) => proof.is_some(),
    }
}

/// Capability adapter for the under-guard destination recheck. Returning `Ok`
/// means the resolver compared the current support graph, repository content,
/// desired target state, guard receipt, and complete approved history against
/// the exact immutable finalization plan supplied here.
pub(crate) trait SupportRecoveryUnderGuardRecheckResolver {
    fn verify_current_destination(
        &self,
        recovery_digest: &Sha256Digest,
        finalization_plan: &SupportRecoveryFinalizationPlan,
        approved_history: &ValidatedRepositoryHistoryPartition,
        guard_receipt_id: &UnicaId,
    ) -> Result<(), ()>;
}

/// Capability adapter for the complete post-release history scan. The adapter
/// returns the maximal allowed contiguous tail and the exact first forbidden
/// successor or coverage-gap observation, when one exists.
pub(crate) trait SupportRecoveryPostReleaseHistoryScanner {
    fn scan_after_release(
        &self,
        from_cursor: &RepositoryHistoryCursor,
    ) -> Result<
        (
            ValidatedRepositoryHistoryPartition,
            Option<DeferredRepositoryAdvance>,
        ),
        (),
    >;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SupportRecoveryWorkingInfobaseDestinationObservation {
    desired_base_fingerprint: Sha256Digest,
    desired_object_fingerprint_map_digest: Sha256Digest,
}

impl SupportRecoveryWorkingInfobaseDestinationObservation {
    pub(crate) const fn from_capability_adapter(
        desired_base_fingerprint: Sha256Digest,
        desired_object_fingerprint_map_digest: Sha256Digest,
    ) -> Self {
        Self {
            desired_base_fingerprint,
            desired_object_fingerprint_map_digest,
        }
    }
}

/// Capability-derived immutable recovery destination.  Raw callers cannot
/// choose disposition, update/lock targets, or terminal graph/content digests
/// on the bootstrap mint itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SupportRecoveryDestinationObservation {
    disposition: SupportRecoveryDisposition,
    lock_targets: super::support_terminalization::SupportRecoveryLockTargets,
    desired_targets: super::support_terminalization::SupportRecoveryDesiredTargets,
    desired_support_graph_digest: Sha256Digest,
    desired_repository_content_digest: Sha256Digest,
    working_infobase: Option<SupportRecoveryWorkingInfobaseDestinationObservation>,
}

impl SupportRecoveryDestinationObservation {
    #[allow(clippy::too_many_arguments)]
    pub(crate) const fn from_capability_adapter(
        disposition: SupportRecoveryDisposition,
        lock_targets: super::support_terminalization::SupportRecoveryLockTargets,
        desired_targets: super::support_terminalization::SupportRecoveryDesiredTargets,
        desired_support_graph_digest: Sha256Digest,
        desired_repository_content_digest: Sha256Digest,
        working_infobase: Option<SupportRecoveryWorkingInfobaseDestinationObservation>,
    ) -> Self {
        Self {
            disposition,
            lock_targets,
            desired_targets,
            desired_support_graph_digest,
            desired_repository_content_digest,
            working_infobase,
        }
    }
}

/// Trusted current-state resolver for the first frozen recovery plan.  The
/// resolver receives the exact frozen action and typed contiguous history and
/// returns one complete destination observation for both manual modes.
pub(crate) trait SupportRecoveryDestinationCapability {
    fn derive_destination(
        &self,
        prior_operation_id: &crate::domain::branched_development::OperationId,
        frozen_authorization: &FrozenSupportRecoveryAuthorizationProjection,
        support_history: &ValidatedRepositoryHistoryPartition,
    ) -> Result<SupportRecoveryDestinationObservation, ()>;
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct SupportRecoveryDesiredPlans {
    authority_lineage: SupportRecoveryAuthorityLineage,
    prior_operation_id: crate::domain::branched_development::OperationId,
    support_action_id: UnicaId,
    support_action_digest: Sha256Digest,
    history_through_cursor: RepositoryHistoryCursor,
    history_partition_digest: Sha256Digest,
    finalization_plan: SupportRecoveryFinalizationPlan,
    working_infobase_closure_plan: Option<ManualWorkingInfobaseClosurePlan>,
}

impl SupportRecoveryDesiredPlans {
    pub(crate) const fn finalization_plan(&self) -> &SupportRecoveryFinalizationPlan {
        &self.finalization_plan
    }

    fn into_parts(
        self,
        expected_token: &SupportRecoveryAuthorityToken,
        expected_prior_operation_id: &crate::domain::branched_development::OperationId,
        frozen_authorization: &FrozenSupportRecoveryAuthorizationProjection,
        support_history: &ValidatedRepositoryHistoryPartition,
    ) -> Result<
        (
            SupportRecoveryFinalizationPlan,
            Option<ManualWorkingInfobaseClosurePlan>,
        ),
        SupportRecoveryAuthorityError,
    > {
        if !self.authority_lineage.belongs_to(expected_token)
            || self.prior_operation_id != *expected_prior_operation_id
            || self.support_action_id != *frozen_authorization.support_action_id()
            || self.support_action_digest != *frozen_authorization.support_action_digest()
            || self.history_through_cursor != *support_history.through_inclusive()
            || self.history_partition_digest != *support_history.partition_digest()
        {
            return Err(SupportRecoveryAuthorityError(
                "support recovery desired plans belong to another bootstrap or history",
            ));
        }
        Ok((self.finalization_plan, self.working_infobase_closure_plan))
    }
}

/// Capability adapter that derives the exact selective-update plan from the
/// complete approved history and immutable desired finalization state. The
/// returned plan is already opaque repository authority; this boundary only
/// proves that it belongs to this recovery projection.
pub(crate) trait SupportRecoveryFinalizationMaterializationCapability {
    fn materialize_selective_update_plan(
        &self,
        recovery_digest: &Sha256Digest,
        desired_finalization_plan: &SupportRecoveryFinalizationPlan,
        approved_history: &ValidatedRepositoryHistoryPartition,
    ) -> Result<SupportRecoverySelectiveUpdatePlanObservation, ()>;
}

/// Capability adapter for the separate working-IB base cursor/object-version
/// map observation. The enclosing authority binds the observation to the
/// frozen identity, baseline, lease capability, and complete history prefix.
pub(crate) trait ManualWorkingInfobaseClosureMaterializationCapability {
    fn observe_recorded_base(
        &self,
        recovery_digest: &Sha256Digest,
        desired_closure_plan: &ManualWorkingInfobaseClosurePlan,
        approved_history: &ValidatedRepositoryHistoryPartition,
    ) -> Result<(RepositoryHistoryCursor, Sha256Digest), ()>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SupportRecoveryGuardAcquisitionObservation {
    BlockedBeforeRoot {
        guard_receipt_id: UnicaId,
        failed_target: RepositoryTargetIdentity,
        failed_target_display: RepositoryTargetDisplay,
        locked_by: RequiredNullable<RepositoryOwnerIdentity>,
    },
    BlockedAfterPartial {
        guard_receipt_id: UnicaId,
        acquired_in_order: SupportRecoveryAcquiredLockTargets,
        failed_target: RepositoryTargetIdentity,
        failed_target_display: RepositoryTargetDisplay,
        locked_by: RequiredNullable<RepositoryOwnerIdentity>,
        released_in_reverse_order: SupportRecoveryReleasedLockTargets,
    },
    Acquired {
        guard_receipt_id: UnicaId,
        acquired_root_first: SupportRecoveryAcquiredLockTargets,
    },
}

/// Typed capability boundary for one exact root-first guard attempt. Raw
/// blocked/acquired observations cannot mint guard evidence without first
/// passing through the approved recovery authority.
pub(crate) trait SupportRecoveryGuardAcquisitionCapability {
    fn acquire_guard(
        &self,
        recovery_digest: &Sha256Digest,
        finalization_plan: &SupportRecoveryFinalizationPlan,
        materialized_working_infobase_closure_plan: Option<&ManualWorkingInfobaseClosurePlan>,
    ) -> Result<SupportRecoveryGuardAcquisitionObservation, ()>;
}

/// Capability result for releasing the exact complete guard window. Both the
/// acquisition receipt and reverse-order target sequence are echoed so the
/// authority can reject a release from another attempt or a partial unlock.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SupportRecoveryGuardReleaseObservation {
    guard_receipt_id: UnicaId,
    guard_release_receipt_id: UnicaId,
    released_in_reverse_order: SupportRecoveryReleasedLockTargets,
}

impl SupportRecoveryGuardReleaseObservation {
    pub(crate) const fn from_capability_adapter(
        guard_receipt_id: UnicaId,
        guard_release_receipt_id: UnicaId,
        released_in_reverse_order: SupportRecoveryReleasedLockTargets,
    ) -> Self {
        Self {
            guard_receipt_id,
            guard_release_receipt_id,
            released_in_reverse_order,
        }
    }
}

/// Releases a complete guard without performing the selective update. The
/// implementation must release every acquired target in exact reverse order
/// and return the provider's distinct release receipt.
pub(crate) trait SupportRecoveryGuardReleaseCapability {
    fn release_complete_guard(
        &self,
        recovery_digest: &Sha256Digest,
        finalization_plan: &SupportRecoveryFinalizationPlan,
        guard_receipt_id: &UnicaId,
    ) -> Result<SupportRecoveryGuardReleaseObservation, ()>;
}

/// Executes the materialized selective finalization inside the already-held
/// guard and releases that exact window. Returning both opaque repository
/// proof and typed release observation keeps completion reachable without a
/// production caller ever assembling either proof field-by-field.
pub(crate) trait SupportRecoveryFinalizationExecutionCapability {
    fn execute_selective_update(
        &self,
        recovery_digest: &Sha256Digest,
        finalization_plan: &SupportRecoveryFinalizationPlan,
        approved_history: &ValidatedRepositoryHistoryPartition,
        guard_receipt_id: &UnicaId,
    ) -> Result<SupportRecoverySelectiveUpdateExecutionObservation, ()>;

    fn terminalize_authorization(
        &self,
        recovery_digest: &Sha256Digest,
        support_action_id: &UnicaId,
        support_action_digest: &Sha256Digest,
        selective_update_proof: &SelectiveRepositoryUpdateProof,
    ) -> Result<SupportRecoveryAuthorizationTerminalizationObservation, ()>;
}

/// Raw durable terminalization observation returned by the execution
/// capability. It is not a receipt and cannot enter a final proof until the
/// sole authority checks the frozen action and expected disposition outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SupportRecoveryAuthorizationTerminalizationObservation {
    support_action_id: UnicaId,
    support_action_digest: Sha256Digest,
    terminalization_receipt_id: UnicaId,
    authorization_outcome: CompletedSupportRecoveryAuthorizationOutcome,
}

impl SupportRecoveryAuthorizationTerminalizationObservation {
    pub(crate) const fn from_capability_adapter(
        support_action_id: UnicaId,
        support_action_digest: Sha256Digest,
        terminalization_receipt_id: UnicaId,
        authorization_outcome: CompletedSupportRecoveryAuthorizationOutcome,
    ) -> Self {
        Self {
            support_action_id,
            support_action_digest,
            terminalization_receipt_id,
            authorization_outcome,
        }
    }
}

/// Opaque live working-IB lease. It deliberately has no `Clone` and contains
/// no release receipt; the authority must carry it through selective update
/// and durable authorization terminalization before the provider may release
/// it and a closure proof may be minted.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ManualWorkingInfobaseAcquiredLease {
    exclusive_lease_receipt_id: UnicaId,
}

impl ManualWorkingInfobaseAcquiredLease {
    pub(crate) const fn from_capability_adapter(exclusive_lease_receipt_id: UnicaId) -> Self {
        Self {
            exclusive_lease_receipt_id,
        }
    }

    pub(crate) const fn exclusive_lease_receipt_id(&self) -> &UnicaId {
        &self.exclusive_lease_receipt_id
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ManualWorkingInfobaseLiveLeaseWindow {
    exclusive_lease_receipt_id: UnicaId,
    working_infobase_base_cursor: RepositoryHistoryCursor,
    recorded_object_version_map_digest: Sha256Digest,
    final_current_fingerprint: Sha256Digest,
    final_base_fingerprint: Sha256Digest,
    final_object_fingerprint_map_digest: Sha256Digest,
    final_support_graph_digest: Sha256Digest,
}

impl ManualWorkingInfobaseLiveLeaseWindow {
    #[allow(clippy::too_many_arguments)]
    pub(crate) const fn from_capability_adapter(
        exclusive_lease_receipt_id: UnicaId,
        working_infobase_base_cursor: RepositoryHistoryCursor,
        recorded_object_version_map_digest: Sha256Digest,
        final_current_fingerprint: Sha256Digest,
        final_base_fingerprint: Sha256Digest,
        final_object_fingerprint_map_digest: Sha256Digest,
        final_support_graph_digest: Sha256Digest,
    ) -> Self {
        Self {
            exclusive_lease_receipt_id,
            working_infobase_base_cursor,
            recorded_object_version_map_digest,
            final_current_fingerprint,
            final_base_fingerprint,
            final_object_fingerprint_map_digest,
            final_support_graph_digest,
        }
    }

    pub(crate) const fn exclusive_lease_receipt_id(&self) -> &UnicaId {
        &self.exclusive_lease_receipt_id
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SupportRecoveryModeLeaseReleaseObservation {
    exclusive_lease_receipt_id: UnicaId,
    exclusive_lease_release_receipt_id: UnicaId,
    authorization_terminalization_receipt_digest: Sha256Digest,
}

impl SupportRecoveryModeLeaseReleaseObservation {
    pub(crate) const fn from_capability_adapter(
        exclusive_lease_receipt_id: UnicaId,
        exclusive_lease_release_receipt_id: UnicaId,
        authorization_terminalization_receipt_digest: Sha256Digest,
    ) -> Self {
        Self {
            exclusive_lease_receipt_id,
            exclusive_lease_release_receipt_id,
            authorization_terminalization_receipt_digest,
        }
    }
}

pub(crate) trait ManualWorkingInfobaseTerminalLeaseCapability {
    fn acquire(
        &self,
        recovery_digest: &Sha256Digest,
        closure_plan: &ManualWorkingInfobaseClosurePlan,
    ) -> Result<ManualWorkingInfobaseAcquiredLease, ()>;

    fn inspect(
        &self,
        recovery_digest: &Sha256Digest,
        closure_plan: &ManualWorkingInfobaseClosurePlan,
        acquired_lease: ManualWorkingInfobaseAcquiredLease,
    ) -> Result<ManualWorkingInfobaseLiveLeaseWindow, ()>;

    fn release_after_terminalization(
        &self,
        recovery_digest: &Sha256Digest,
        closure_plan: &ManualWorkingInfobaseClosurePlan,
        live_lease: ManualWorkingInfobaseLiveLeaseWindow,
        authorization_terminalization_receipt: &SupportRecoveryAuthorizationTerminalizationReceipt,
    ) -> Result<SupportRecoveryModeLeaseReleaseObservation, ()>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReservedOriginalLeaseBusyObservation {
    manual_actor_username: RepositoryUsername,
    baseline_lock_set_digest: Sha256Digest,
    observed_lock_set_digest: Sha256Digest,
    reserved_original_identity_digest: Sha256Digest,
    exclusive_lease_capability_id: CapabilityRowId,
    lease_owner: RequiredNullable<RepositoryOwnerIdentity>,
}

impl ReservedOriginalLeaseBusyObservation {
    #[allow(clippy::too_many_arguments)]
    pub(crate) const fn from_capability_adapter(
        manual_actor_username: RepositoryUsername,
        baseline_lock_set_digest: Sha256Digest,
        observed_lock_set_digest: Sha256Digest,
        reserved_original_identity_digest: Sha256Digest,
        exclusive_lease_capability_id: CapabilityRowId,
        lease_owner: RequiredNullable<RepositoryOwnerIdentity>,
    ) -> Self {
        Self {
            manual_actor_username,
            baseline_lock_set_digest,
            observed_lock_set_digest,
            reserved_original_identity_digest,
            exclusive_lease_capability_id,
            lease_owner,
        }
    }
}

pub(crate) trait ReservedOriginalLeaseBusyCapability {
    fn observe_lease_busy(
        &self,
        recovery_digest: &Sha256Digest,
        finalization_plan: &SupportRecoveryFinalizationPlan,
    ) -> Result<ReservedOriginalLeaseBusyObservation, ()>;
}

/// Opaque reserved-original lease window held across inspection, selective
/// update, and durable authorization terminalization.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ReservedOriginalAcquiredLease {
    exclusive_lease_receipt_id: UnicaId,
}

impl ReservedOriginalAcquiredLease {
    pub(crate) const fn from_capability_adapter(exclusive_lease_receipt_id: UnicaId) -> Self {
        Self {
            exclusive_lease_receipt_id,
        }
    }

    pub(crate) const fn exclusive_lease_receipt_id(&self) -> &UnicaId {
        &self.exclusive_lease_receipt_id
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ReservedOriginalLiveLeaseWindow {
    manual_actor_username: RepositoryUsername,
    baseline_lock_set_digest: Sha256Digest,
    observed_lock_set_digest: Sha256Digest,
    reserved_original_identity_digest: Sha256Digest,
    exclusive_lease_capability_id: CapabilityRowId,
    exclusive_lease_receipt_id: UnicaId,
    expected_repository_fingerprint: Sha256Digest,
    observed_original_fingerprint: Sha256Digest,
}

impl ReservedOriginalLiveLeaseWindow {
    #[allow(clippy::too_many_arguments)]
    pub(crate) const fn from_capability_adapter(
        manual_actor_username: RepositoryUsername,
        baseline_lock_set_digest: Sha256Digest,
        observed_lock_set_digest: Sha256Digest,
        reserved_original_identity_digest: Sha256Digest,
        exclusive_lease_capability_id: CapabilityRowId,
        exclusive_lease_receipt_id: UnicaId,
        expected_repository_fingerprint: Sha256Digest,
        observed_original_fingerprint: Sha256Digest,
    ) -> Self {
        Self {
            manual_actor_username,
            baseline_lock_set_digest,
            observed_lock_set_digest,
            reserved_original_identity_digest,
            exclusive_lease_capability_id,
            exclusive_lease_receipt_id,
            expected_repository_fingerprint,
            observed_original_fingerprint,
        }
    }

    pub(crate) const fn exclusive_lease_receipt_id(&self) -> &UnicaId {
        &self.exclusive_lease_receipt_id
    }
}

pub(crate) trait ReservedOriginalTerminalLeaseCapability {
    fn acquire(
        &self,
        recovery_digest: &Sha256Digest,
        finalization_plan: &SupportRecoveryFinalizationPlan,
    ) -> Result<ReservedOriginalAcquiredLease, ()>;

    fn inspect(
        &self,
        recovery_digest: &Sha256Digest,
        finalization_plan: &SupportRecoveryFinalizationPlan,
        acquired_lease: ReservedOriginalAcquiredLease,
    ) -> Result<ReservedOriginalLiveLeaseWindow, ()>;

    fn release_after_terminalization(
        &self,
        recovery_digest: &Sha256Digest,
        finalization_plan: &SupportRecoveryFinalizationPlan,
        live_lease: ReservedOriginalLiveLeaseWindow,
        authorization_terminalization_receipt: &SupportRecoveryAuthorizationTerminalizationReceipt,
    ) -> Result<SupportRecoveryModeLeaseReleaseObservation, ()>;
}

pub(crate) trait ManualWorkingInfobaseLeaseBusyCapability {
    fn observe_lease_owner(
        &self,
        recovery_digest: &Sha256Digest,
        closure_plan: &ManualWorkingInfobaseClosurePlan,
    ) -> Result<RequiredNullable<RepositoryOwnerIdentity>, ()>;
}

pub(crate) type WorkingInfobaseDirtyCapabilityObservation =
    (Sha256Digest, Sha256Digest, UnicaId, UnicaId);

/// Exclusive-lease capability adapter for a dirty working-IB stop. The tuple
/// is observed working-IB fingerprint, observed support graph, acquire receipt,
/// and verified release receipt.
pub(crate) trait ManualWorkingInfobaseDirtyCapability {
    fn inspect_dirty_and_release(
        &self,
        recovery_digest: &Sha256Digest,
        closure_plan: &ManualWorkingInfobaseClosurePlan,
    ) -> Result<WorkingInfobaseDirtyCapabilityObservation, ()>;
}

fn post_release_tail_is_allowed(partition: &ValidatedRepositoryHistoryPartition) -> bool {
    partition.classifications().all(|classification| {
        matches!(
            classification,
            RepositoryHistoryPartitionClassification::UnrelatedRoutine
                | RepositoryHistoryPartitionClassification::RelevantRoutine
                | RepositoryHistoryPartitionClassification::ExternalSupport
                | RepositoryHistoryPartitionClassification::NonConflictingConcurrent
        )
    })
}

fn partition_requires_late_phase(partition: &ValidatedRepositoryHistoryPartition) -> bool {
    partition.classifications().any(|classification| {
        matches!(
            classification,
            RepositoryHistoryPartitionClassification::RelevantRoutine
                | RepositoryHistoryPartitionClassification::ExternalSupport
                | RepositoryHistoryPartitionClassification::NonConflictingConcurrent
        )
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ApprovedSupportRecoveryCompletion {
    guard_proof: SupportRecoveryGuardProof,
    result_phase: TaskPhase,
}

impl ApprovedSupportRecoveryCompletion {
    pub(crate) const fn guard_proof(&self) -> &SupportRecoveryGuardProof {
        &self.guard_proof
    }

    pub(crate) const fn result_phase(&self) -> TaskPhase {
        self.result_phase
    }

    pub(crate) fn into_guard_proof(self) -> SupportRecoveryGuardProof {
        self.guard_proof
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ApprovedSupportRecoveryGuardAttempt {
    Blocked(Box<CurrentBlockedSupportRecoveryGuardAttempt>),
    Acquired(Box<ApprovedSupportRecoveryGuardWindow>),
}

/// Linear blocked-guard typestate.  The proof remains inseparable from the
/// exact approved recovery authority whose acquisition attempt minted it, so
/// another authority with a byte-identical finalization plan cannot consume
/// the proof to mint its own external wait.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct CurrentBlockedSupportRecoveryGuardAttempt {
    authority: ApprovedSupportRecoveryAuthority,
    prior_operation_id: crate::domain::branched_development::OperationId,
    support_action_id: UnicaId,
    support_action_digest: Sha256Digest,
    recovery_digest: Sha256Digest,
    guard_proof: SupportRecoveryGuardProof,
}

/// Opaque proof that the exact materialized guard plan was acquired for this
/// approved recovery attempt. Its private fields prevent a raw receipt ID from
/// reaching completion, working-IB execution, or complete-guard stop mints.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ApprovedSupportRecoveryGuardWindow {
    authority: ApprovedSupportRecoveryAuthority,
    guard_receipt_id: UnicaId,
    acquired_root_first: SupportRecoveryAcquiredLockTargets,
}

/// Consuming typestate for a separate-mode stop. The opaque stop reason was
/// observed exactly once from this guard window and cannot be replayed to mint
/// multiple terminal proofs.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct PreparedSupportRecoveryStopWindow {
    window: ApprovedSupportRecoveryGuardWindow,
    stop_evidence: ManualWorkingInfobaseStopEvidence,
}

/// Consuming typestate for separate-mode completion. The live lease window and
/// repository guard move together until durable authorization terminalization.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct PreparedSupportRecoveryCompletionWindow {
    window: ApprovedSupportRecoveryGuardWindow,
    live_lease: ManualWorkingInfobaseLiveLeaseWindow,
}

/// Consuming typestate for reserved-original completion. The manual actor
/// inventory and live exclusive configuration lease are capability-observed
/// once and cannot be supplied as caller-authored final proof values.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct PreparedReservedSupportRecoveryCompletionWindow {
    window: ApprovedSupportRecoveryGuardWindow,
    live_lease: ReservedOriginalLiveLeaseWindow,
}

/// Consuming reserved-original stop typestate. The raw inventory and lease
/// stop stay private until they have minted both the released guard proof and
/// the matching external closure wait.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct PreparedReservedSupportRecoveryStopWindow {
    window: ApprovedSupportRecoveryGuardWindow,
    manual_actor_lock_inventory_proof: ManualActorLockInventoryProof,
    stop_evidence: ReservedOriginalLeaseStopEvidence,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct PreparedSupportRecoveryExternalReplan {
    authority: ApprovedSupportRecoveryAuthority,
    latest_guard_proof: SupportRecoveryGuardProof,
    external_wait: BoundSupportRecoveryExternalWait,
}

/// Linear replan projection bound both to this authority lineage and to the
/// exact previously approved recovery digest from which it was derived.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct PreparedSupportRecoveryReplanProjection {
    authority_lineage: SupportRecoveryAuthorityLineage,
    previous_recovery_digest: Sha256Digest,
    projection: ArmedSupportRecoveryPlanProjection,
}

/// A finalization plan that may enter exactly one replan of the authority
/// version that retained or capability-materialized it.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct BoundSupportRecoveryReplanFinalizationPlan {
    authority_lineage: SupportRecoveryAuthorityLineage,
    previous_recovery_digest: Sha256Digest,
    plan: SupportRecoveryFinalizationPlan,
}

impl BoundSupportRecoveryReplanFinalizationPlan {
    pub(crate) const fn plan(&self) -> &SupportRecoveryFinalizationPlan {
        &self.plan
    }

    fn into_plan(
        self,
        expected_token: &SupportRecoveryAuthorityToken,
        expected_previous_recovery_digest: &Sha256Digest,
    ) -> Result<SupportRecoveryFinalizationPlan, SupportRecoveryAuthorityError> {
        if !self.authority_lineage.belongs_to(expected_token)
            || self.previous_recovery_digest != *expected_previous_recovery_digest
        {
            return Err(SupportRecoveryAuthorityError(
                "support recovery finalization plan belongs to another authority lineage or prior plan",
            ));
        }
        Ok(self.plan)
    }
}

/// Separate-mode closure plan paired with the same exact authority version as
/// its finalization plan. Reserved mode has no value of this type.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct BoundSupportRecoveryReplanWorkingInfobaseClosurePlan {
    authority_lineage: SupportRecoveryAuthorityLineage,
    previous_recovery_digest: Sha256Digest,
    plan: ManualWorkingInfobaseClosurePlan,
}

impl BoundSupportRecoveryReplanWorkingInfobaseClosurePlan {
    pub(crate) const fn plan(&self) -> &ManualWorkingInfobaseClosurePlan {
        &self.plan
    }

    fn into_plan(
        self,
        expected_token: &SupportRecoveryAuthorityToken,
        expected_previous_recovery_digest: &Sha256Digest,
    ) -> Result<ManualWorkingInfobaseClosurePlan, SupportRecoveryAuthorityError> {
        if !self.authority_lineage.belongs_to(expected_token)
            || self.previous_recovery_digest != *expected_previous_recovery_digest
        {
            return Err(SupportRecoveryAuthorityError(
                "support recovery working-infobase closure plan belongs to another authority lineage or prior plan",
            ));
        }
        Ok(self.plan)
    }
}

impl CurrentBlockedSupportRecoveryGuardAttempt {
    fn new(
        authority: ApprovedSupportRecoveryAuthority,
        guard_proof: SupportRecoveryGuardProof,
    ) -> Self {
        Self {
            prior_operation_id: authority.prior_operation_id().clone(),
            support_action_id: authority.support_action_id().clone(),
            support_action_digest: authority.support_action_digest().clone(),
            recovery_digest: authority.recovery_digest().clone(),
            authority,
            guard_proof,
        }
    }

    fn binds_authority(&self, authority: &ApprovedSupportRecoveryAuthority) -> bool {
        self.prior_operation_id == *authority.prior_operation_id()
            && self.support_action_id == *authority.support_action_id()
            && self.support_action_digest == *authority.support_action_digest()
            && self.recovery_digest == *authority.recovery_digest()
            && !self.guard_proof.is_completed()
            && self.guard_proof.blocked_target_ref().is_some()
            && self.guard_proof.manual_target_mode() == authority.authorization.manual_target_mode()
            && self.guard_proof.finalization_plan_digest()
                == authority.finalization_plan().plan_digest()
    }

    pub(crate) fn lock_release_external_replan(
        self,
        action_id: UnicaId,
        expected_unlocked_digest: Sha256Digest,
    ) -> Result<PreparedSupportRecoveryExternalReplan, SupportRecoveryAuthorityError> {
        if !self.binds_authority(&self.authority) {
            return Err(SupportRecoveryAuthorityError(
                "blocked guard attempt no longer binds its exact recovery authority",
            ));
        }
        let Self {
            authority,
            guard_proof,
            ..
        } = self;
        authority.prepare_lock_release_external_replan(
            action_id,
            guard_proof,
            expected_unlocked_digest,
        )
    }
}

impl PreparedSupportRecoveryExternalReplan {
    /// Seals the external wait into the next approved projection without ever
    /// exposing a separable `(authority, proof, wait)` tuple to callers.
    pub(crate) fn approve_replan(
        self,
        history_action_id: UnicaId,
        finalization_action_id: UnicaId,
    ) -> Result<ApprovedSupportRecoveryAuthority, SupportRecoveryAuthorityError> {
        let Self {
            authority,
            latest_guard_proof,
            external_wait,
        } = self;
        let planned_result_phase = authority.projection.planned_result_phase();
        let support_history_from_cursor =
            authority.projection.support_history_from_cursor().clone();
        let support_history_through_cursor = authority
            .projection
            .support_history_through_cursor()
            .clone();
        let support_history_partition = authority.projection.support_history_partition().clone();
        let support_version_observations =
            authority.projection.support_version_observations().clone();
        let support_recovery_disposition = authority.projection.support_recovery_disposition();
        let support_late_relevant_result_phase =
            authority.projection.support_late_relevant_result_phase();
        let support_recovery_finalization_plan = authority.retained_replan_finalization_plan();
        let manual_working_infobase_closure_plan =
            authority.retained_replan_working_infobase_closure_plan();
        let action_catalog = authority.replan_action_catalog_with_external(
            history_action_id,
            external_wait,
            finalization_action_id,
        )?;
        let projection = authority.replan_projection(
            action_catalog,
            planned_result_phase,
            support_history_from_cursor,
            support_history_through_cursor,
            support_history_partition,
            support_version_observations,
            support_recovery_disposition,
            support_late_relevant_result_phase,
            support_recovery_finalization_plan,
            Some(latest_guard_proof),
            manual_working_infobase_closure_plan,
        )?;
        authority.approve_replan(projection)
    }
}

/// Typed provider observation that an armed support operation has an unknown
/// effect and therefore must enter recovery.  It repeats the exact armed
/// identifiers so a capability result from another action cannot bootstrap
/// this authority chain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SupportRecoveryEffectUnknownObservation {
    prior_operation_id: crate::domain::branched_development::OperationId,
    support_action_id: UnicaId,
    support_action_digest: Sha256Digest,
    arming_receipt_id: UnicaId,
    arming_receipt_digest: Sha256Digest,
}

impl SupportRecoveryEffectUnknownObservation {
    pub(crate) const fn from_capability_adapter(
        prior_operation_id: crate::domain::branched_development::OperationId,
        support_action_id: UnicaId,
        support_action_digest: Sha256Digest,
        arming_receipt_id: UnicaId,
        arming_receipt_digest: Sha256Digest,
    ) -> Self {
        Self {
            prior_operation_id,
            support_action_id,
            support_action_digest,
            arming_receipt_id,
            arming_receipt_digest,
        }
    }
}

pub(crate) trait SupportRecoveryEffectUnknownCapability {
    fn observe_unknown_effect(
        &self,
        armed_action: &ArmedSupportInstructionProjection,
    ) -> Result<SupportRecoveryEffectUnknownObservation, ()>;
}

/// Provider boundary that validates an external-support conflict against the
/// exact frozen action, complete recovery history, and desired destination
/// before returning the typed instruction. The integrated authority still
/// checks the destination digest before wrapping it in the recovery catalog.
pub(crate) trait SupportRecoveryConflictSourceCapability {
    fn validated_conflict_instruction(
        &self,
        prior_operation_id: &crate::domain::branched_development::OperationId,
        support_action_id: &UnicaId,
        support_action_digest: &Sha256Digest,
        history: &ValidatedRepositoryHistoryPartition,
        finalization_plan: &SupportRecoveryFinalizationPlan,
    ) -> Result<SupportConflictInstruction, ()>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SupportRecoveryEvidenceSourceObservation {
    instruction: SupportEvidenceInstruction,
    evidence_artifact_id: UnicaId,
    expected_evidence_digest: Sha256Digest,
}

impl SupportRecoveryEvidenceSourceObservation {
    pub(crate) const fn from_capability_adapter(
        instruction: SupportEvidenceInstruction,
        evidence_artifact_id: UnicaId,
        expected_evidence_digest: Sha256Digest,
    ) -> Self {
        Self {
            instruction,
            evidence_artifact_id,
            expected_evidence_digest,
        }
    }
}

/// Provider boundary for a validated evidence gap. It receives the same
/// frozen action/history/destination tuple that will be sealed into the plan,
/// so an evidence wait cannot be sourced from a different recovery attempt.
pub(crate) trait SupportRecoveryEvidenceSourceCapability {
    fn validated_evidence_requirement(
        &self,
        prior_operation_id: &crate::domain::branched_development::OperationId,
        support_action_id: &UnicaId,
        support_action_digest: &Sha256Digest,
        history: &ValidatedRepositoryHistoryPartition,
        finalization_plan: &SupportRecoveryFinalizationPlan,
    ) -> Result<SupportRecoveryEvidenceSourceObservation, ()>;
}

/// Non-cyclic production entry into support recovery.  This authority is the
/// only production mint for the recovery token: it first consumes an already
/// armed action plus capability-proven unknown effect, then freezes that exact
/// action.  Desired plans and the final projection are minted from the same
/// token before `approve` consumes the bootstrap.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct SupportRecoveryBootstrapAuthority {
    token: SupportRecoveryAuthorityToken,
    frozen_authorization: FrozenSupportRecoveryAuthorizationProjection,
    prior_operation_id: crate::domain::branched_development::OperationId,
}

/// Linear bootstrap typestate after the one trusted destination observation.
/// The paired finalization/working-IB plans remain private and inseparable from
/// the exact token, frozen authorization, and prior operation that minted them.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct PreparedSupportRecoveryBootstrap {
    bootstrap: SupportRecoveryBootstrapAuthority,
    desired_plans: SupportRecoveryDesiredPlans,
}

/// Linear initial projection paired with the exact bootstrap lineage that
/// validated its destination, history, observations, and action catalog.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct PreparedSupportRecoveryBootstrapProjection {
    bootstrap: SupportRecoveryBootstrapAuthority,
    projection: ArmedSupportRecoveryPlanProjection,
}

/// Consuming external-wait authority bound to one frozen support action and
/// the exact effect-unknown operation that entered recovery. A wait minted by
/// another bootstrap cannot be inserted into this bootstrap's action catalog.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct BoundSupportRecoveryExternalWait {
    authority_lineage: SupportRecoveryAuthorityLineage,
    prior_operation_id: crate::domain::branched_development::OperationId,
    support_action_id: UnicaId,
    support_action_digest: Sha256Digest,
    candidate_history_through_cursor: Option<RepositoryHistoryCursor>,
    candidate_history_partition_digest: Option<Sha256Digest>,
    candidate_finalization_plan_digest: Option<Sha256Digest>,
    guard_proof_binding: SupportRecoveryGuardProofBinding,
    wait: SupportRecoveryExternalWaitAuthority,
}

#[derive(Debug, PartialEq, Eq)]
enum SupportRecoveryGuardProofBinding {
    Absent,
    Exact(Sha256Digest),
}

impl BoundSupportRecoveryExternalWait {
    fn new(
        token: &SupportRecoveryAuthorityToken,
        prior_operation_id: crate::domain::branched_development::OperationId,
        authorization: &FrozenSupportRecoveryAuthorizationProjection,
        wait: SupportRecoveryExternalWaitAuthority,
    ) -> Self {
        Self {
            authority_lineage: token.lineage(),
            prior_operation_id,
            support_action_id: authorization.support_action_id().clone(),
            support_action_digest: authorization.support_action_digest().clone(),
            candidate_history_through_cursor: None,
            candidate_history_partition_digest: None,
            candidate_finalization_plan_digest: None,
            guard_proof_binding: SupportRecoveryGuardProofBinding::Absent,
            wait,
        }
    }

    fn bind_candidate(
        mut self,
        history: &ValidatedRepositoryHistoryPartition,
        finalization_plan: &SupportRecoveryFinalizationPlan,
    ) -> Self {
        self.candidate_history_through_cursor = Some(history.through_inclusive().clone());
        self.candidate_history_partition_digest = Some(history.partition_digest().clone());
        self.candidate_finalization_plan_digest = Some(finalization_plan.plan_digest().clone());
        self
    }

    fn require_guard_proof(mut self, proof: &SupportRecoveryGuardProof) -> Self {
        self.guard_proof_binding =
            SupportRecoveryGuardProofBinding::Exact(proof.proof_digest().clone());
        self
    }
}

/// Opaque catalog sidecar that preserves the recovery-attempt binding after
/// the external wait has been lowered into the raw recovery action catalog.
/// A catalog minted by one bootstrap/attempt therefore cannot be projected by
/// another authority because both retain the same unique non-wire lineage.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct BoundSupportRecoveryActionCatalog {
    authority_lineage: SupportRecoveryAuthorityLineage,
    prior_operation_id: crate::domain::branched_development::OperationId,
    support_action_id: UnicaId,
    support_action_digest: Sha256Digest,
    candidate_history_through_cursor: Option<RepositoryHistoryCursor>,
    candidate_history_partition_digest: Option<Sha256Digest>,
    candidate_finalization_plan_digest: Option<Sha256Digest>,
    guard_proof_binding: SupportRecoveryGuardProofBinding,
    catalog: SupportRecoveryActionCatalogAuthority,
}

impl BoundSupportRecoveryActionCatalog {
    fn without_external(
        token: &SupportRecoveryAuthorityToken,
        prior_operation_id: crate::domain::branched_development::OperationId,
        authorization: &FrozenSupportRecoveryAuthorizationProjection,
        catalog: SupportRecoveryActionCatalogAuthority,
    ) -> Self {
        Self {
            authority_lineage: token.lineage(),
            prior_operation_id,
            support_action_id: authorization.support_action_id().clone(),
            support_action_digest: authorization.support_action_digest().clone(),
            candidate_history_through_cursor: None,
            candidate_history_partition_digest: None,
            candidate_finalization_plan_digest: None,
            guard_proof_binding: SupportRecoveryGuardProofBinding::Absent,
            catalog,
        }
    }

    fn with_external(
        token: &SupportRecoveryAuthorityToken,
        history_action_id: UnicaId,
        external_wait: BoundSupportRecoveryExternalWait,
        finalization_action_id: UnicaId,
    ) -> Result<Self, SupportRecoveryAuthorityError> {
        if !external_wait.authority_lineage.belongs_to(token) {
            return Err(SupportRecoveryAuthorityError(
                "support recovery external wait belongs to another authority lineage",
            ));
        }
        let BoundSupportRecoveryExternalWait {
            authority_lineage,
            prior_operation_id,
            support_action_id,
            support_action_digest,
            candidate_history_through_cursor,
            candidate_history_partition_digest,
            candidate_finalization_plan_digest,
            guard_proof_binding,
            wait,
        } = external_wait;
        let catalog = SupportRecoveryActionCatalogAuthority::with_external_from_approved(
            token,
            history_action_id,
            wait,
            finalization_action_id,
        )
        .map_err(|_| {
            SupportRecoveryAuthorityError(
                "support recovery action catalog has duplicate or invalid action IDs",
            )
        })?;
        Ok(Self {
            authority_lineage,
            prior_operation_id,
            support_action_id,
            support_action_digest,
            candidate_history_through_cursor,
            candidate_history_partition_digest,
            candidate_finalization_plan_digest,
            guard_proof_binding,
            catalog,
        })
    }

    fn into_catalog_for_projection(
        self,
        expected_token: &SupportRecoveryAuthorityToken,
        expected_prior_operation_id: &crate::domain::branched_development::OperationId,
        authorization: &FrozenSupportRecoveryAuthorizationProjection,
        history: &ValidatedRepositoryHistoryPartition,
        finalization_plan: &SupportRecoveryFinalizationPlan,
        latest_guard_proof: Option<&SupportRecoveryGuardProof>,
    ) -> Result<SupportRecoveryActionCatalogAuthority, SupportRecoveryAuthorityError> {
        let candidate_binds = self
            .candidate_history_through_cursor
            .as_ref()
            .is_none_or(|expected| expected == history.through_inclusive())
            && self
                .candidate_history_partition_digest
                .as_ref()
                .is_none_or(|expected| expected == history.partition_digest())
            && self
                .candidate_finalization_plan_digest
                .as_ref()
                .is_none_or(|expected| expected == finalization_plan.plan_digest())
            && match &self.guard_proof_binding {
                SupportRecoveryGuardProofBinding::Absent => latest_guard_proof.is_none(),
                SupportRecoveryGuardProofBinding::Exact(expected) => {
                    latest_guard_proof.is_some_and(|proof| proof.proof_digest() == expected)
                }
            };
        if !self.authority_lineage.belongs_to(expected_token)
            || self.prior_operation_id != *expected_prior_operation_id
            || self.support_action_id != *authorization.support_action_id()
            || self.support_action_digest != *authorization.support_action_digest()
            || !candidate_binds
        {
            return Err(SupportRecoveryAuthorityError(
                "support recovery action catalog belongs to another bootstrap, candidate, or guard attempt",
            ));
        }
        Ok(self.catalog)
    }
}

impl SupportRecoveryBootstrapAuthority {
    pub(crate) fn from_effect_unknown(
        handle: ActiveSupportActionResumeHandle,
        capability: &dyn SupportRecoveryEffectUnknownCapability,
    ) -> Result<Self, SupportRecoveryAuthorityError> {
        let armed =
            handle
                .armed_support_instruction_projection()
                .ok_or(SupportRecoveryAuthorityError(
                    "support recovery bootstrap requires an armed support action",
                ))?;
        let observation = capability.observe_unknown_effect(&armed).map_err(|()| {
            SupportRecoveryAuthorityError(
                "support recovery effect-unknown capability returned no observation",
            )
        })?;
        if observation.support_action_id != *armed.support_action_id()
            || observation.support_action_digest != *armed.support_action_digest()
            || observation.arming_receipt_id != *armed.arming_receipt_id()
            || observation.arming_receipt_digest != *armed.arming_receipt_digest()
        {
            return Err(SupportRecoveryAuthorityError(
                "support recovery effect-unknown observation belongs to another armed action",
            ));
        }
        let token = SupportRecoveryAuthorityToken::new();
        let frozen = handle
            .freeze_armed_action_from_recovery(&token)
            .map_err(|_| {
                SupportRecoveryAuthorityError(
                    "support recovery bootstrap could not freeze the armed action",
                )
            })?;
        let frozen_authorization =
            frozen
                .frozen_support_recovery_projection()
                .ok_or(SupportRecoveryAuthorityError(
                    "support recovery bootstrap lost its frozen authorization projection",
                ))?;
        Ok(Self {
            token,
            frozen_authorization,
            prior_operation_id: observation.prior_operation_id,
        })
    }

    pub(crate) fn desired_plans(
        self,
        support_history: &ValidatedRepositoryHistoryPartition,
        capability: &dyn SupportRecoveryDestinationCapability,
    ) -> Result<PreparedSupportRecoveryBootstrap, SupportRecoveryAuthorityError> {
        let binding =
            self.frozen_authorization
                .armed_binding()
                .ok_or(SupportRecoveryAuthorityError(
                    "support recovery bootstrap lost its frozen armed binding",
                ))?;
        if support_history.start_cursor() != binding.expected_before_history_cursor()
            || !support_history.has_exact_entry_prefix(binding.arming_receipt().history_partition())
        {
            return Err(SupportRecoveryAuthorityError(
                "support recovery destination resolver received spliced history",
            ));
        }
        let destination = capability
            .derive_destination(
                &self.prior_operation_id,
                &self.frozen_authorization,
                support_history,
            )
            .map_err(|()| {
                SupportRecoveryAuthorityError(
                    "support recovery destination capability returned no exact destination",
                )
            })?;
        let closure_presence_is_exact = match self.frozen_authorization.manual_target_mode() {
            ManualSupportTargetMode::ReservedOriginal => destination.working_infobase.is_none(),
            ManualSupportTargetMode::SeparateWorkingInfobase => {
                destination.working_infobase.is_some()
            }
        };
        if !closure_presence_is_exact {
            return Err(SupportRecoveryAuthorityError(
                "support recovery destination has the wrong manual-mode closure presence",
            ));
        }
        let desired_support_graph_digest = destination.desired_support_graph_digest.clone();
        let finalization_plan = SupportRecoveryFinalizationPlan::new(
            SupportRecoveryFinalizationPlanAuthority::from_approved(
                &self.token,
                destination.disposition,
                destination.lock_targets,
                destination.desired_targets,
                support_history.start_cursor().clone(),
                None,
                destination.desired_support_graph_digest,
                destination.desired_repository_content_digest,
            ),
        )
        .map_err(SupportRecoveryAuthorityError::from)?;
        let working_infobase_closure_plan = destination
            .working_infobase
            .map(|working_infobase| {
                let baseline = binding.manual_working_infobase_baseline().ok_or(
                    SupportRecoveryAuthorityError(
                        "separate-mode recovery lost its frozen working-infobase baseline",
                    ),
                )?;
                ManualWorkingInfobaseClosurePlan::new(
                    ManualWorkingInfobaseClosurePlanAuthority::desired_from_approved(
                        &self.token,
                        baseline.working_infobase_identity().clone(),
                        baseline.baseline_digest().clone(),
                        working_infobase.desired_base_fingerprint,
                        working_infobase.desired_object_fingerprint_map_digest,
                        desired_support_graph_digest,
                        baseline.exclusive_lease_capability_id().clone(),
                    ),
                )
                .map_err(SupportRecoveryAuthorityError::from)
            })
            .transpose()?;
        let desired_plans = SupportRecoveryDesiredPlans {
            authority_lineage: self.token.lineage(),
            prior_operation_id: self.prior_operation_id.clone(),
            support_action_id: self.frozen_authorization.support_action_id().clone(),
            support_action_digest: self.frozen_authorization.support_action_digest().clone(),
            history_through_cursor: support_history.through_inclusive().clone(),
            history_partition_digest: support_history.partition_digest().clone(),
            finalization_plan,
            working_infobase_closure_plan,
        };
        Ok(PreparedSupportRecoveryBootstrap {
            bootstrap: self,
            desired_plans,
        })
    }

    /// Mints the initial corrective wait directly from the frozen action and
    /// the candidate recovery destination. There is intentionally no
    /// already-approved projection to compare against on this first cycle.
    #[allow(clippy::too_many_arguments)]
    fn corrective_external_wait(
        &self,
        action_id: UnicaId,
        support_history: &ValidatedRepositoryHistoryPartition,
        required_root_transitions: Vec<SupportRecoveryTransition>,
        required_content_restorations: Vec<SupportContentRestoration>,
        distribution_handoffs: Vec<SupportRecoveryDistributionHandoff>,
        handoff_revalidations: Vec<SupportRecoveryHandoffRevalidation>,
        finalization_plan: &SupportRecoveryFinalizationPlan,
        lock_closure_resolver: &dyn SupportCorrectiveLockClosureResolver,
    ) -> Result<BoundSupportRecoveryExternalWait, SupportRecoveryAuthorityError> {
        let support_history_through_cursor = support_history.through_inclusive();
        let binding =
            self.frozen_authorization
                .armed_binding()
                .ok_or(SupportRecoveryAuthorityError(
                    "support recovery bootstrap lost its frozen armed binding",
                ))?;
        if distribution_handoffs.iter().any(|handoff| {
            !binding
                .support_recovery_distributions()
                .iter()
                .any(|distribution| distribution.handoff() == handoff)
        }) {
            return Err(SupportRecoveryAuthorityError(
                "initial corrective wait substituted a frozen distribution handoff",
            ));
        }
        let instruction = SupportCorrectiveInstruction::new(
            SupportCorrectiveInstructionAuthority::from_approved(
                &self.token,
                self.frozen_authorization.support_action_id().clone(),
                binding.purpose(),
                self.frozen_authorization.manual_target_mode(),
                self.frozen_authorization.manual_actor_username().clone(),
                binding.manual_working_infobase_identity().cloned(),
                support_history_through_cursor.clone(),
                required_root_transitions,
                required_content_restorations,
                distribution_handoffs,
                handoff_revalidations,
                finalization_plan,
                lock_closure_resolver,
            )?,
        )?;
        if !corrective_instruction_binds_inputs(
            &instruction,
            &self.frozen_authorization,
            binding,
            support_history_through_cursor,
            finalization_plan,
        ) {
            return Err(SupportRecoveryAuthorityError(
                "initial corrective wait differs from the frozen action or recovery destination",
            ));
        }
        Ok(BoundSupportRecoveryExternalWait::new(
            &self.token,
            self.prior_operation_id.clone(),
            &self.frozen_authorization,
            SupportRecoveryExternalWaitAuthority::corrective_from_approved(
                &self.token,
                action_id,
                instruction,
            ),
        )
        .bind_candidate(support_history, finalization_plan))
    }

    fn conflict_external_wait(
        &self,
        action_id: UnicaId,
        history: &ValidatedRepositoryHistoryPartition,
        finalization_plan: &SupportRecoveryFinalizationPlan,
        capability: &dyn SupportRecoveryConflictSourceCapability,
    ) -> Result<BoundSupportRecoveryExternalWait, SupportRecoveryAuthorityError> {
        let instruction = capability
            .validated_conflict_instruction(
                &self.prior_operation_id,
                self.frozen_authorization.support_action_id(),
                self.frozen_authorization.support_action_digest(),
                history,
                finalization_plan,
            )
            .map_err(|()| {
                SupportRecoveryAuthorityError(
                    "support conflict source did not validate the exact recovery inputs",
                )
            })?;
        if instruction.required_final_baseline_digest()
            != finalization_plan.desired_support_graph_digest()
        {
            return Err(SupportRecoveryAuthorityError(
                "support conflict instruction has a substituted final baseline",
            ));
        }
        Ok(BoundSupportRecoveryExternalWait::new(
            &self.token,
            self.prior_operation_id.clone(),
            &self.frozen_authorization,
            SupportRecoveryExternalWaitAuthority::conflict_from_approved(
                &self.token,
                action_id,
                instruction,
            ),
        )
        .bind_candidate(history, finalization_plan))
    }

    fn evidence_external_wait(
        &self,
        action_id: UnicaId,
        history: &ValidatedRepositoryHistoryPartition,
        finalization_plan: &SupportRecoveryFinalizationPlan,
        capability: &dyn SupportRecoveryEvidenceSourceCapability,
    ) -> Result<BoundSupportRecoveryExternalWait, SupportRecoveryAuthorityError> {
        let observation = capability
            .validated_evidence_requirement(
                &self.prior_operation_id,
                self.frozen_authorization.support_action_id(),
                self.frozen_authorization.support_action_digest(),
                history,
                finalization_plan,
            )
            .map_err(|()| {
                SupportRecoveryAuthorityError(
                    "support evidence source did not validate the exact recovery inputs",
                )
            })?;
        let postcondition =
            SupportRecoveryEvidencePostconditionObservation::from_capability_adapter(
                &observation.instruction,
                observation.evidence_artifact_id,
                observation.expected_evidence_digest,
            );
        let wait = SupportRecoveryExternalWaitAuthority::evidence_from_approved(
            &self.token,
            action_id,
            observation.instruction,
            postcondition,
        )
        .map_err(|_| {
            SupportRecoveryAuthorityError(
                "support evidence wait differs from its validated evidence source",
            )
        })?;
        Ok(BoundSupportRecoveryExternalWait::new(
            &self.token,
            self.prior_operation_id.clone(),
            &self.frozen_authorization,
            wait,
        )
        .bind_candidate(history, finalization_plan))
    }

    #[allow(clippy::too_many_arguments)]
    fn build_recovery_plan_projection(
        &self,
        action_catalog: BoundSupportRecoveryActionCatalog,
        planned_result_phase: TaskPhase,
        support_history_partition: ValidatedRepositoryHistoryPartition,
        support_version_observations: SupportRecoveryVersionObservations,
        support_late_relevant_result_phase: TaskPhase,
        desired_plans: SupportRecoveryDesiredPlans,
    ) -> Result<ArmedSupportRecoveryPlanProjection, SupportRecoveryAuthorityError> {
        let support_history_from_cursor = support_history_partition.start_cursor().clone();
        let support_history_through_cursor = support_history_partition.through_inclusive().clone();
        let (support_recovery_finalization_plan, manual_working_infobase_closure_plan) =
            desired_plans.into_parts(
                &self.token,
                &self.prior_operation_id,
                &self.frozen_authorization,
                &support_history_partition,
            )?;
        let support_recovery_disposition = support_recovery_finalization_plan.disposition();
        let action_catalog = action_catalog.into_catalog_for_projection(
            &self.token,
            &self.prior_operation_id,
            &self.frozen_authorization,
            &support_history_partition,
            &support_recovery_finalization_plan,
            None,
        )?;
        ArmedSupportRecoveryPlanProjection::from_approved(
            &self.token,
            self.prior_operation_id.clone(),
            action_catalog,
            self.frozen_authorization.support_action_id().clone(),
            planned_result_phase,
            support_history_from_cursor,
            support_history_through_cursor,
            support_history_partition,
            support_version_observations,
            support_recovery_disposition,
            support_late_relevant_result_phase,
            support_recovery_finalization_plan,
            None,
            manual_working_infobase_closure_plan,
            self.frozen_authorization.manual_target_mode(),
        )
        .map_err(|_| {
            SupportRecoveryAuthorityError(
                "support recovery bootstrap could not construct the approved plan projection",
            )
        })
    }

    fn action_catalog_without_external(
        &self,
        history_action_id: UnicaId,
        finalization_action_id: UnicaId,
    ) -> Result<BoundSupportRecoveryActionCatalog, SupportRecoveryAuthorityError> {
        let catalog = SupportRecoveryActionCatalogAuthority::without_external_from_approved(
            &self.token,
            history_action_id,
            finalization_action_id,
        )
        .map_err(|_| {
            SupportRecoveryAuthorityError(
                "support recovery action catalog has duplicate or invalid action IDs",
            )
        })?;
        Ok(BoundSupportRecoveryActionCatalog::without_external(
            &self.token,
            self.prior_operation_id.clone(),
            &self.frozen_authorization,
            catalog,
        ))
    }

    fn action_catalog_with_external(
        &self,
        history_action_id: UnicaId,
        external_wait: BoundSupportRecoveryExternalWait,
        finalization_action_id: UnicaId,
    ) -> Result<BoundSupportRecoveryActionCatalog, SupportRecoveryAuthorityError> {
        if external_wait.prior_operation_id != self.prior_operation_id
            || external_wait.support_action_id != *self.frozen_authorization.support_action_id()
            || external_wait.support_action_digest
                != *self.frozen_authorization.support_action_digest()
        {
            return Err(SupportRecoveryAuthorityError(
                "support recovery external wait belongs to another bootstrap or frozen action",
            ));
        }
        BoundSupportRecoveryActionCatalog::with_external(
            &self.token,
            history_action_id,
            external_wait,
            finalization_action_id,
        )
    }

    fn approve_projection(
        self,
        projection: ArmedSupportRecoveryPlanProjection,
    ) -> Result<ApprovedSupportRecoveryAuthority, SupportRecoveryAuthorityError> {
        let prior_operation_id = self.prior_operation_id.clone();
        ApprovedSupportRecoveryAuthority::new_with_token(
            self.frozen_authorization,
            projection,
            self.token,
            &prior_operation_id,
        )
    }
}

impl PreparedSupportRecoveryBootstrap {
    pub(crate) const fn finalization_plan(&self) -> &SupportRecoveryFinalizationPlan {
        self.desired_plans.finalization_plan()
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn corrective_external_wait(
        &self,
        action_id: UnicaId,
        support_history: &ValidatedRepositoryHistoryPartition,
        required_root_transitions: Vec<SupportRecoveryTransition>,
        required_content_restorations: Vec<SupportContentRestoration>,
        distribution_handoffs: Vec<SupportRecoveryDistributionHandoff>,
        handoff_revalidations: Vec<SupportRecoveryHandoffRevalidation>,
        lock_closure_resolver: &dyn SupportCorrectiveLockClosureResolver,
    ) -> Result<BoundSupportRecoveryExternalWait, SupportRecoveryAuthorityError> {
        self.bootstrap.corrective_external_wait(
            action_id,
            support_history,
            required_root_transitions,
            required_content_restorations,
            distribution_handoffs,
            handoff_revalidations,
            self.desired_plans.finalization_plan(),
            lock_closure_resolver,
        )
    }

    pub(crate) fn conflict_external_wait(
        &self,
        action_id: UnicaId,
        history: &ValidatedRepositoryHistoryPartition,
        capability: &dyn SupportRecoveryConflictSourceCapability,
    ) -> Result<BoundSupportRecoveryExternalWait, SupportRecoveryAuthorityError> {
        self.bootstrap.conflict_external_wait(
            action_id,
            history,
            self.desired_plans.finalization_plan(),
            capability,
        )
    }

    pub(crate) fn evidence_external_wait(
        &self,
        action_id: UnicaId,
        history: &ValidatedRepositoryHistoryPartition,
        capability: &dyn SupportRecoveryEvidenceSourceCapability,
    ) -> Result<BoundSupportRecoveryExternalWait, SupportRecoveryAuthorityError> {
        self.bootstrap.evidence_external_wait(
            action_id,
            history,
            self.desired_plans.finalization_plan(),
            capability,
        )
    }

    pub(crate) fn action_catalog_without_external(
        &self,
        history_action_id: UnicaId,
        finalization_action_id: UnicaId,
    ) -> Result<BoundSupportRecoveryActionCatalog, SupportRecoveryAuthorityError> {
        self.bootstrap
            .action_catalog_without_external(history_action_id, finalization_action_id)
    }

    pub(crate) fn action_catalog_with_external(
        &self,
        history_action_id: UnicaId,
        external_wait: BoundSupportRecoveryExternalWait,
        finalization_action_id: UnicaId,
    ) -> Result<BoundSupportRecoveryActionCatalog, SupportRecoveryAuthorityError> {
        self.bootstrap.action_catalog_with_external(
            history_action_id,
            external_wait,
            finalization_action_id,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn recovery_plan_projection(
        self,
        action_catalog: BoundSupportRecoveryActionCatalog,
        planned_result_phase: TaskPhase,
        support_history_partition: ValidatedRepositoryHistoryPartition,
        support_version_observations: SupportRecoveryVersionObservations,
        support_late_relevant_result_phase: TaskPhase,
    ) -> Result<PreparedSupportRecoveryBootstrapProjection, SupportRecoveryAuthorityError> {
        let projection = self.bootstrap.build_recovery_plan_projection(
            action_catalog,
            planned_result_phase,
            support_history_partition,
            support_version_observations,
            support_late_relevant_result_phase,
            self.desired_plans,
        )?;
        Ok(PreparedSupportRecoveryBootstrapProjection {
            bootstrap: self.bootstrap,
            projection,
        })
    }
}

impl PreparedSupportRecoveryBootstrapProjection {
    pub(crate) fn approve(
        self,
    ) -> Result<ApprovedSupportRecoveryAuthority, SupportRecoveryAuthorityError> {
        self.bootstrap.approve_projection(self.projection)
    }
}

impl ApprovedSupportRecoveryGuardWindow {
    pub(crate) const fn guard_receipt_id(&self) -> &UnicaId {
        &self.guard_receipt_id
    }
}

/// Sole production capability for a caller-approved armed support-recovery
/// plan. Construction consumes two opaque projections; neither raw frozen
/// fields nor a standalone action plan can mint it.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ApprovedSupportRecoveryAuthority {
    authorization: FrozenSupportRecoveryAuthorizationProjection,
    projection: ArmedSupportRecoveryPlanProjection,
    token: SupportRecoveryAuthorityToken,
}

impl ApprovedSupportRecoveryAuthority {
    #[cfg(test)]
    pub(crate) fn new(
        authorization: FrozenSupportRecoveryAuthorizationProjection,
        projection: ArmedSupportRecoveryPlanProjection,
    ) -> Result<Self, SupportRecoveryAuthorityError> {
        let prior_operation_id = projection.prior_operation_id().clone();
        Self::new_with_token(
            authorization,
            projection,
            SupportRecoveryAuthorityToken::new(),
            &prior_operation_id,
        )
    }

    fn new_with_token(
        authorization: FrozenSupportRecoveryAuthorizationProjection,
        projection: ArmedSupportRecoveryPlanProjection,
        token: SupportRecoveryAuthorityToken,
        expected_prior_operation_id: &crate::domain::branched_development::OperationId,
    ) -> Result<Self, SupportRecoveryAuthorityError> {
        let binding = authorization
            .armed_binding()
            .ok_or(SupportRecoveryAuthorityError(
                "support recovery requires a real frozen armed authorization",
            ))?;
        let receipt = binding.arming_receipt();
        let partition = projection.support_history_partition();
        let finalization = projection.support_recovery_finalization_plan();
        let distribution_set =
            SupportRecoveryDistributionSet::new(binding.support_recovery_distributions().to_vec())
                .map_err(|_| {
                    SupportRecoveryAuthorityError(
                        "frozen support recovery distributions no longer form their exact set",
                    )
                })?;

        let expected_observation_digest = projection
            .support_version_observations()
            .digest()
            .map_err(|_| {
                SupportRecoveryAuthorityError(
                    "support recovery observation aggregate digest could not be recomputed",
                )
            })?;

        if projection.prior_operation_id() != expected_prior_operation_id
            || projection.support_version_observation_digest() != &expected_observation_digest
            || projection.recovery_plan_status().prior_operation_id() != expected_prior_operation_id
            || projection.recovery_plan_status().recovery_digest() != projection.recovery_digest()
            || projection.support_action_id() != authorization.support_action_id()
            || projection.manual_target_mode() != authorization.manual_target_mode()
            || receipt.support_action_id() != authorization.support_action_id()
            || receipt.support_action_digest() != authorization.support_action_digest()
            || receipt.expected_before_history_cursor() != binding.expected_before_history_cursor()
            || receipt.support_graph_digest() != binding.expected_support_graph_digest()
            || receipt.support_recovery_distribution_set_digest() != distribution_set.digest()
            || receipt.original_fingerprint() != authorization.expected_original_fingerprint()
            || receipt.manual_target_mode() != authorization.manual_target_mode()
            || receipt.history_partition().through_inclusive() != receipt.arming_cursor()
        {
            return Err(SupportRecoveryAuthorityError(
                "recovery plan differs from the exact frozen action or accepted arming receipt",
            ));
        }
        if projection.support_history_from_cursor() != binding.expected_before_history_cursor()
            || partition.start_cursor() != projection.support_history_from_cursor()
            || partition.through_inclusive() != projection.support_history_through_cursor()
            || finalization.history_from_cursor() != projection.support_history_from_cursor()
            || finalization.disposition() != projection.support_recovery_disposition()
            || !partition_has_exact_prefix(partition, receipt.history_partition())?
            || !observations_bind_partition(
                partition,
                projection.support_version_observations().as_slice(),
                &authorization,
                binding,
            )?
            || partition.classifications().any(|classification| {
                classification == RepositoryHistoryPartitionClassification::PreArmExternal
            })
        {
            return Err(SupportRecoveryAuthorityError(
                "approved support history, observations, or finalization anchor were spliced",
            ));
        }
        let external_action_precedes_guard =
            projection.required_external_action().is_some_and(|action| {
                matches!(
                    action.as_ref(),
                    SupportRecoveryExternalActionRef::Corrective(_)
                        | SupportRecoveryExternalActionRef::Conflict(_)
                        | SupportRecoveryExternalActionRef::Evidence(_)
                )
            });
        if finalization
            .materialized_selective_update_plan()
            .is_some_and(|selective_plan| {
                !desired_targets_bind_materialized_plan(finalization, selective_plan, partition)
                    .unwrap_or(false)
            })
            || finalization.materialized_selective_update_plan().is_some()
                && external_action_precedes_guard
        {
            return Err(SupportRecoveryAuthorityError(
                "embedded materialized finalization plan is spliced or has a pre-guard external action",
            ));
        }
        if !phase_binding_is_exact(binding, &projection)
            || !closure_plan_binds_frozen_authorization(&authorization, binding, &projection)
        {
            return Err(SupportRecoveryAuthorityError(
                "support recovery mode, phase, or working-infobase closure binding is invalid",
            ));
        }
        if !external_action_guard_proof_presence_is_exact(
            projection.required_external_action(),
            projection.latest_support_recovery_guard_proof(),
        ) || projection
            .latest_support_recovery_guard_proof()
            .is_some_and(|proof| {
                proof.is_completed()
                    || proof.finalization_plan_digest() != finalization.plan_digest()
                    || proof.manual_target_mode() != authorization.manual_target_mode()
            })
        {
            return Err(SupportRecoveryAuthorityError(
                "latest support guard proof has the wrong blocker presence or belongs to another or completed plan",
            ));
        }
        if projection.required_external_action().is_some_and(|action| {
            !external_action_binds_plan(action, &authorization, binding, &projection, &token)
        }) {
            return Err(SupportRecoveryAuthorityError(
                "external recovery action differs from the frozen action, history, blocker, mode, or plan",
            ));
        }

        Ok(Self {
            authorization,
            projection,
            token,
        })
    }

    pub(crate) const fn recovery_digest(&self) -> &Sha256Digest {
        self.projection.recovery_digest()
    }

    pub(crate) const fn prior_operation_id(
        &self,
    ) -> &crate::domain::branched_development::OperationId {
        self.projection.prior_operation_id()
    }

    pub(crate) const fn support_version_observation_digest(&self) -> &Sha256Digest {
        self.projection.support_version_observation_digest()
    }

    pub(crate) fn recovery_plan_status(&self) -> RecoveryPlanStatus {
        self.projection.recovery_plan_status()
    }

    pub(crate) const fn support_action_id(&self) -> &UnicaId {
        self.authorization.support_action_id()
    }

    pub(crate) const fn support_action_digest(&self) -> &Sha256Digest {
        self.authorization.support_action_digest()
    }

    pub(crate) const fn finalization_plan(&self) -> &SupportRecoveryFinalizationPlan {
        self.projection.support_recovery_finalization_plan()
    }

    pub(crate) const fn working_infobase_closure_plan(
        &self,
    ) -> Option<&ManualWorkingInfobaseClosurePlan> {
        self.projection.manual_working_infobase_closure_plan()
    }

    fn bind_replan_finalization_plan(
        &self,
        plan: SupportRecoveryFinalizationPlan,
    ) -> BoundSupportRecoveryReplanFinalizationPlan {
        BoundSupportRecoveryReplanFinalizationPlan {
            authority_lineage: self.token.lineage(),
            previous_recovery_digest: self.recovery_digest().clone(),
            plan,
        }
    }

    fn bind_replan_working_infobase_closure_plan(
        &self,
        plan: ManualWorkingInfobaseClosurePlan,
    ) -> BoundSupportRecoveryReplanWorkingInfobaseClosurePlan {
        BoundSupportRecoveryReplanWorkingInfobaseClosurePlan {
            authority_lineage: self.token.lineage(),
            previous_recovery_digest: self.recovery_digest().clone(),
            plan,
        }
    }

    pub(crate) fn retained_replan_finalization_plan(
        &self,
    ) -> BoundSupportRecoveryReplanFinalizationPlan {
        self.bind_replan_finalization_plan(self.finalization_plan().clone())
    }

    pub(crate) fn retained_replan_working_infobase_closure_plan(
        &self,
    ) -> Option<BoundSupportRecoveryReplanWorkingInfobaseClosurePlan> {
        self.working_infobase_closure_plan()
            .cloned()
            .map(|plan| self.bind_replan_working_infobase_closure_plan(plan))
    }

    fn bind_external_wait(
        &self,
        wait: SupportRecoveryExternalWaitAuthority,
    ) -> BoundSupportRecoveryExternalWait {
        BoundSupportRecoveryExternalWait::new(
            &self.token,
            self.prior_operation_id().clone(),
            &self.authorization,
            wait,
        )
        .bind_candidate(
            self.projection.support_history_partition(),
            self.finalization_plan(),
        )
    }

    fn prepare_lock_release_external_replan(
        self,
        action_id: UnicaId,
        blocked_guard_proof: SupportRecoveryGuardProof,
        expected_unlocked_digest: Sha256Digest,
    ) -> Result<PreparedSupportRecoveryExternalReplan, SupportRecoveryAuthorityError> {
        if blocked_guard_proof.is_completed()
            || blocked_guard_proof.finalization_plan_digest()
                != self.finalization_plan().plan_digest()
            || blocked_guard_proof.blocked_target_ref().is_none()
        {
            return Err(SupportRecoveryAuthorityError(
                "lock-release replan requires this plan's exact blocked guard proof",
            ));
        }
        let postcondition =
            SupportRecoveryLockReleasePostconditionObservation::from_capability_adapter(
                &blocked_guard_proof,
                expected_unlocked_digest,
            )
            .map_err(|_| {
                SupportRecoveryAuthorityError(
                    "lock-release replan could not bind the blocked guard subject",
                )
            })?;
        let wait = SupportRecoveryExternalWaitAuthority::release_locks_from_approved(
            &self.token,
            action_id,
            &blocked_guard_proof,
            postcondition,
        )
        .map_err(|_| {
            SupportRecoveryAuthorityError(
                "lock-release wait differs from its consumed blocked guard proof",
            )
        })?;
        let external_wait = self
            .bind_external_wait(wait)
            .require_guard_proof(&blocked_guard_proof);
        Ok(PreparedSupportRecoveryExternalReplan {
            authority: self,
            latest_guard_proof: blocked_guard_proof,
            external_wait,
        })
    }

    fn clean_working_infobase_wait_from_stop(
        &self,
        action_id: UnicaId,
        stop: &ManualWorkingInfobaseStopEvidence,
        expected_available_lease_digest: Sha256Digest,
    ) -> Result<BoundSupportRecoveryExternalWait, SupportRecoveryAuthorityError> {
        let plan = self
            .working_infobase_closure_plan()
            .ok_or(SupportRecoveryAuthorityError(
                "reserved mode cannot mint a working-infobase cleanup wait",
            ))?;
        let postcondition =
            ManualWorkingInfobaseAvailablePostconditionObservation::from_capability_adapter(
                plan,
                stop,
                expected_available_lease_digest,
            )
            .map_err(|_| {
                SupportRecoveryAuthorityError(
                    "working-infobase cleanup wait belongs to another closure stop",
                )
            })?;
        let wait = SupportRecoveryExternalWaitAuthority::clean_working_infobase_from_approved(
            &self.token,
            action_id,
            plan,
            stop,
            postcondition,
        )
        .map_err(|_| {
            SupportRecoveryAuthorityError(
                "working-infobase cleanup wait differs from its consumed stop evidence",
            )
        })?;
        Ok(self.bind_external_wait(wait))
    }

    fn close_reserved_original_wait_from_stop(
        &self,
        action_id: UnicaId,
        stop: &ReservedOriginalLeaseStopEvidence,
        expected_available_lease_digest: Sha256Digest,
    ) -> Result<BoundSupportRecoveryExternalWait, SupportRecoveryAuthorityError> {
        let expected_capability = self
            .authorization
            .reserved_original_lease_capability_id()
            .ok_or(SupportRecoveryAuthorityError(
                "separate mode cannot mint a reserved-original closure wait",
            ))?;
        if stop.reserved_original_identity_digest()
            != self.authorization.reserved_original_identity_digest()
            || stop.exclusive_lease_capability_id() != expected_capability
        {
            return Err(SupportRecoveryAuthorityError(
                "reserved-original closure wait substituted frozen lease evidence",
            ));
        }
        let postcondition =
            ReservedOriginalAvailablePostconditionObservation::from_capability_adapter(
                stop,
                expected_available_lease_digest,
            );
        let wait = SupportRecoveryExternalWaitAuthority::close_reserved_original_from_approved(
            &self.token,
            action_id,
            stop,
            postcondition,
        )
        .map_err(|_| {
            SupportRecoveryAuthorityError(
                "reserved-original closure wait differs from its consumed stop evidence",
            )
        })?;
        Ok(self.bind_external_wait(wait))
    }

    pub(crate) fn conflict_external_replan_wait(
        &self,
        action_id: UnicaId,
        history: &ValidatedRepositoryHistoryPartition,
        finalization_plan: &SupportRecoveryFinalizationPlan,
        capability: &dyn SupportRecoveryConflictSourceCapability,
    ) -> Result<BoundSupportRecoveryExternalWait, SupportRecoveryAuthorityError> {
        let instruction = capability
            .validated_conflict_instruction(
                self.prior_operation_id(),
                self.support_action_id(),
                self.support_action_digest(),
                history,
                finalization_plan,
            )
            .map_err(|()| {
                SupportRecoveryAuthorityError(
                    "support conflict source did not validate the exact replan inputs",
                )
            })?;
        if instruction.required_final_baseline_digest()
            != finalization_plan.desired_support_graph_digest()
        {
            return Err(SupportRecoveryAuthorityError(
                "support conflict replan has a substituted final baseline",
            ));
        }
        Ok(self
            .bind_external_wait(
                SupportRecoveryExternalWaitAuthority::conflict_from_approved(
                    &self.token,
                    action_id,
                    instruction,
                ),
            )
            .bind_candidate(history, finalization_plan))
    }

    pub(crate) fn evidence_external_replan_wait(
        &self,
        action_id: UnicaId,
        history: &ValidatedRepositoryHistoryPartition,
        finalization_plan: &SupportRecoveryFinalizationPlan,
        capability: &dyn SupportRecoveryEvidenceSourceCapability,
    ) -> Result<BoundSupportRecoveryExternalWait, SupportRecoveryAuthorityError> {
        let observation = capability
            .validated_evidence_requirement(
                self.prior_operation_id(),
                self.support_action_id(),
                self.support_action_digest(),
                history,
                finalization_plan,
            )
            .map_err(|()| {
                SupportRecoveryAuthorityError(
                    "support evidence source did not validate the exact replan inputs",
                )
            })?;
        let postcondition =
            SupportRecoveryEvidencePostconditionObservation::from_capability_adapter(
                &observation.instruction,
                observation.evidence_artifact_id,
                observation.expected_evidence_digest,
            );
        let wait = SupportRecoveryExternalWaitAuthority::evidence_from_approved(
            &self.token,
            action_id,
            observation.instruction,
            postcondition,
        )
        .map_err(|_| {
            SupportRecoveryAuthorityError(
                "support evidence replan wait differs from its validated source",
            )
        })?;
        Ok(self
            .bind_external_wait(wait)
            .bind_candidate(history, finalization_plan))
    }

    pub(crate) fn replan_action_catalog_without_external(
        &self,
        history_action_id: UnicaId,
        finalization_action_id: UnicaId,
    ) -> Result<BoundSupportRecoveryActionCatalog, SupportRecoveryAuthorityError> {
        if self.projection.required_external_action().is_some() {
            return Err(SupportRecoveryAuthorityError(
                "support recovery cannot clear an external action before typed resolution lineage exists",
            ));
        }
        let catalog = SupportRecoveryActionCatalogAuthority::without_external_from_approved(
            &self.token,
            history_action_id,
            finalization_action_id,
        )
        .map_err(|_| {
            SupportRecoveryAuthorityError(
                "support recovery replan action catalog has duplicate action IDs",
            )
        })?;
        Ok(BoundSupportRecoveryActionCatalog::without_external(
            &self.token,
            self.prior_operation_id().clone(),
            &self.authorization,
            catalog,
        ))
    }

    pub(crate) fn replan_action_catalog_with_external(
        &self,
        history_action_id: UnicaId,
        external_wait: BoundSupportRecoveryExternalWait,
        finalization_action_id: UnicaId,
    ) -> Result<BoundSupportRecoveryActionCatalog, SupportRecoveryAuthorityError> {
        if external_wait.prior_operation_id != *self.prior_operation_id()
            || external_wait.support_action_id != *self.support_action_id()
            || external_wait.support_action_digest != *self.support_action_digest()
        {
            return Err(SupportRecoveryAuthorityError(
                "support recovery replan wait belongs to another operation or action",
            ));
        }
        BoundSupportRecoveryActionCatalog::with_external(
            &self.token,
            history_action_id,
            external_wait,
            finalization_action_id,
        )
    }

    fn replan_history_extends_current(
        &self,
        support_history_from_cursor: &RepositoryHistoryCursor,
        support_history_partition: &ValidatedRepositoryHistoryPartition,
        support_version_observations: &SupportRecoveryVersionObservations,
    ) -> bool {
        support_history_from_cursor == self.projection.support_history_from_cursor()
            && support_history_partition
                .has_exact_entry_prefix(self.projection.support_history_partition())
            && support_version_observations
                .as_slice()
                .starts_with(self.projection.support_version_observations().as_slice())
    }

    fn finalization_destination_extends_current(
        &self,
        candidate: &SupportRecoveryFinalizationPlan,
    ) -> bool {
        let current = self.finalization_plan();
        let immutable_destination_is_exact = candidate.disposition() == current.disposition()
            && candidate.lock_targets() == current.lock_targets()
            && candidate.desired_targets() == current.desired_targets()
            && candidate.history_from_cursor() == current.history_from_cursor()
            && candidate.desired_support_graph_digest() == current.desired_support_graph_digest()
            && candidate.desired_repository_content_digest()
                == current.desired_repository_content_digest();
        let materialization_is_monotonic =
            current
                .materialized_selective_update_plan()
                .is_none_or(|materialized| {
                    candidate.materialized_selective_update_plan() == Some(materialized)
                });
        immutable_destination_is_exact && materialization_is_monotonic
    }

    fn working_infobase_destination_extends_current(
        &self,
        candidate: Option<&ManualWorkingInfobaseClosurePlan>,
    ) -> bool {
        let current = self.working_infobase_closure_plan();
        let (Some(current), Some(candidate)) = (current, candidate) else {
            return current.is_none() && candidate.is_none();
        };
        let immutable_destination_is_exact = candidate.working_infobase_identity()
            == current.working_infobase_identity()
            && candidate.authorization_baseline_digest() == current.authorization_baseline_digest()
            && candidate.desired_base_fingerprint() == current.desired_base_fingerprint()
            && candidate.desired_object_fingerprint_map_digest()
                == current.desired_object_fingerprint_map_digest()
            && candidate.desired_support_graph_digest() == current.desired_support_graph_digest()
            && candidate.exclusive_lease_capability_id() == current.exclusive_lease_capability_id();
        let materialization_is_monotonic = current.materialized().is_err() || candidate == current;
        immutable_destination_is_exact && materialization_is_monotonic
    }

    fn replan_destination_extends_current(
        &self,
        disposition: SupportRecoveryDisposition,
        finalization_plan: &SupportRecoveryFinalizationPlan,
        working_infobase_closure_plan: Option<&ManualWorkingInfobaseClosurePlan>,
    ) -> bool {
        disposition == self.projection.support_recovery_disposition()
            && disposition == finalization_plan.disposition()
            && self.finalization_destination_extends_current(finalization_plan)
            && self.working_infobase_destination_extends_current(working_infobase_closure_plan)
    }

    fn external_wait_action(
        projection: &ArmedSupportRecoveryPlanProjection,
    ) -> Option<&RecoveryAction> {
        projection
            .required_external_action()
            .and_then(|_| projection.actions().get(1))
    }

    fn replan_external_action_transition_is_valid(
        &self,
        candidate: &ArmedSupportRecoveryPlanProjection,
    ) -> bool {
        let Some(current_action) = self.projection.required_external_action() else {
            return true;
        };
        candidate.required_external_action() == Some(current_action)
            && Self::external_wait_action(candidate) == Self::external_wait_action(&self.projection)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn replan_projection(
        &self,
        action_catalog: BoundSupportRecoveryActionCatalog,
        planned_result_phase: TaskPhase,
        support_history_from_cursor: RepositoryHistoryCursor,
        support_history_through_cursor: RepositoryHistoryCursor,
        support_history_partition: ValidatedRepositoryHistoryPartition,
        support_version_observations: SupportRecoveryVersionObservations,
        support_recovery_disposition: SupportRecoveryDisposition,
        support_late_relevant_result_phase: TaskPhase,
        support_recovery_finalization_plan: BoundSupportRecoveryReplanFinalizationPlan,
        latest_support_recovery_guard_proof: Option<SupportRecoveryGuardProof>,
        manual_working_infobase_closure_plan: Option<
            BoundSupportRecoveryReplanWorkingInfobaseClosurePlan,
        >,
    ) -> Result<PreparedSupportRecoveryReplanProjection, SupportRecoveryAuthorityError> {
        let support_recovery_finalization_plan =
            support_recovery_finalization_plan.into_plan(&self.token, self.recovery_digest())?;
        let manual_working_infobase_closure_plan = manual_working_infobase_closure_plan
            .map(|plan| plan.into_plan(&self.token, self.recovery_digest()))
            .transpose()?;
        if !self.replan_destination_extends_current(
            support_recovery_disposition,
            &support_recovery_finalization_plan,
            manual_working_infobase_closure_plan.as_ref(),
        ) {
            return Err(SupportRecoveryAuthorityError(
                "support recovery replan substituted or rewound its approved destination",
            ));
        }
        if !self.replan_history_extends_current(
            &support_history_from_cursor,
            &support_history_partition,
            &support_version_observations,
        ) {
            return Err(SupportRecoveryAuthorityError(
                "support recovery replan must retain the exact approved history and observation prefixes",
            ));
        }
        let action_catalog = action_catalog.into_catalog_for_projection(
            &self.token,
            self.prior_operation_id(),
            &self.authorization,
            &support_history_partition,
            &support_recovery_finalization_plan,
            latest_support_recovery_guard_proof.as_ref(),
        )?;
        let projection = ArmedSupportRecoveryPlanProjection::from_approved(
            &self.token,
            self.prior_operation_id().clone(),
            action_catalog,
            self.support_action_id().clone(),
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
            self.authorization.manual_target_mode(),
        )
        .map_err(|_| {
            SupportRecoveryAuthorityError(
                "support recovery authority could not construct its exact replan projection",
            )
        })?;
        if !self.replan_external_action_transition_is_valid(&projection) {
            return Err(SupportRecoveryAuthorityError(
                "support recovery replan substituted or erased its unresolved external action",
            ));
        }
        Ok(PreparedSupportRecoveryReplanProjection {
            authority_lineage: self.token.lineage(),
            previous_recovery_digest: self.recovery_digest().clone(),
            projection,
        })
    }

    pub(crate) fn approve_replan(
        self,
        prepared: PreparedSupportRecoveryReplanProjection,
    ) -> Result<ApprovedSupportRecoveryAuthority, SupportRecoveryAuthorityError> {
        if !prepared.authority_lineage.belongs_to(&self.token)
            || prepared.previous_recovery_digest != *self.recovery_digest()
        {
            return Err(SupportRecoveryAuthorityError(
                "support recovery replan projection belongs to another authority lineage or prior plan",
            ));
        }
        self.approve_replan_projection(prepared.projection)
    }

    fn approve_replan_projection(
        self,
        projection: ArmedSupportRecoveryPlanProjection,
    ) -> Result<ApprovedSupportRecoveryAuthority, SupportRecoveryAuthorityError> {
        if !self.replan_destination_extends_current(
            projection.support_recovery_disposition(),
            projection.support_recovery_finalization_plan(),
            projection.manual_working_infobase_closure_plan(),
        ) {
            return Err(SupportRecoveryAuthorityError(
                "support recovery approval substituted or rewound its approved destination",
            ));
        }
        if !self.replan_history_extends_current(
            projection.support_history_from_cursor(),
            projection.support_history_partition(),
            projection.support_version_observations(),
        ) {
            return Err(SupportRecoveryAuthorityError(
                "support recovery approval cannot rewind or replace approved history",
            ));
        }
        if !self.replan_external_action_transition_is_valid(&projection) {
            return Err(SupportRecoveryAuthorityError(
                "support recovery approval substituted or erased its unresolved external action",
            ));
        }
        let prior_operation_id = self.prior_operation_id().clone();
        ApprovedSupportRecoveryAuthority::new_with_token(
            self.authorization,
            projection,
            self.token,
            &prior_operation_id,
        )
    }

    #[cfg(test)]
    fn approve_reconstructed_replan_for_test(
        self,
        projection: ArmedSupportRecoveryPlanProjection,
    ) -> Result<ApprovedSupportRecoveryAuthority, SupportRecoveryAuthorityError> {
        self.approve_replan_projection(projection)
    }

    pub(crate) fn materialized_finalization_plan(
        &self,
        capability: &dyn SupportRecoveryFinalizationMaterializationCapability,
    ) -> Result<BoundSupportRecoveryReplanFinalizationPlan, SupportRecoveryAuthorityError> {
        let current = self.finalization_plan();
        if current.materialized_selective_update_plan().is_some()
            || self.projection.required_external_action().is_some()
        {
            return Err(SupportRecoveryAuthorityError(
                "finalization materialization requires a desired plan with no pending external action",
            ));
        }
        let selective_observation = capability
            .materialize_selective_update_plan(
                self.recovery_digest(),
                current,
                self.projection.support_history_partition(),
            )
            .map_err(|()| {
                SupportRecoveryAuthorityError(
                    "finalization materialization capability did not prove an exact selective plan",
                )
            })?;
        let selective_plan = SelectiveRepositoryUpdatePlan::recovery_finalization_from_approved(
            &self.token,
            selective_observation,
        )
        .map_err(|_| {
            SupportRecoveryAuthorityError(
                "finalization materialization observation cannot mint an exact selective plan",
            )
        })?;
        if !desired_targets_bind_materialized_plan(
            current,
            &selective_plan,
            self.projection.support_history_partition(),
        )? {
            return Err(SupportRecoveryAuthorityError(
                "materialized selective plan differs from the desired targets, locks, or approved history",
            ));
        }
        let plan = SupportRecoveryFinalizationPlan::new(
            SupportRecoveryFinalizationPlanAuthority::from_approved(
                &self.token,
                current.disposition(),
                current.lock_targets().clone(),
                current.desired_targets().clone(),
                current.history_from_cursor().clone(),
                Some(selective_plan),
                current.desired_support_graph_digest().clone(),
                current.desired_repository_content_digest().clone(),
            ),
        )
        .map_err(SupportRecoveryAuthorityError::from)?;
        Ok(self.bind_replan_finalization_plan(plan))
    }

    pub(crate) fn materialized_working_infobase_closure_plan(
        &self,
        capability: &dyn ManualWorkingInfobaseClosureMaterializationCapability,
    ) -> Result<BoundSupportRecoveryReplanWorkingInfobaseClosurePlan, SupportRecoveryAuthorityError>
    {
        let binding = self
            .authorization
            .armed_binding()
            .ok_or(SupportRecoveryAuthorityError(
                "approved authority lost its frozen armed binding",
            ))?;
        let baseline =
            binding
                .manual_working_infobase_baseline()
                .ok_or(SupportRecoveryAuthorityError(
                    "reserved-mode recovery has no working-infobase closure materialization",
                ))?;
        let desired_plan =
            self.working_infobase_closure_plan()
                .ok_or(SupportRecoveryAuthorityError(
                    "separate-mode recovery lost its desired working-infobase closure plan",
                ))?;
        if desired_plan.materialized().is_ok() {
            return Err(SupportRecoveryAuthorityError(
                "working-infobase closure plan is already materialized",
            ));
        }
        let (working_infobase_base_cursor, recorded_object_version_map_digest) = capability
            .observe_recorded_base(
                self.recovery_digest(),
                desired_plan,
                self.projection.support_history_partition(),
            )
            .map_err(|()| {
                SupportRecoveryAuthorityError(
                    "working-infobase materialization capability did not prove its recorded base",
                )
            })?;
        if working_infobase_base_cursor != *baseline.repository_base_cursor()
            && !self
                .projection
                .support_history_partition()
                .contains_cursor(&working_infobase_base_cursor)
        {
            return Err(SupportRecoveryAuthorityError(
                "working-infobase base cursor is outside its frozen baseline and approved history",
            ));
        }
        if working_infobase_base_cursor == *baseline.repository_base_cursor()
            && recorded_object_version_map_digest != *baseline.recorded_object_version_map_digest()
        {
            return Err(SupportRecoveryAuthorityError(
                "working-infobase baseline cursor was paired with a substituted object-version map",
            ));
        }
        let plan = ManualWorkingInfobaseClosurePlan::new(
            ManualWorkingInfobaseClosurePlanAuthority::materialized_from_approved(
                &self.token,
                desired_plan.working_infobase_identity().clone(),
                desired_plan.authorization_baseline_digest().clone(),
                desired_plan.desired_base_fingerprint().clone(),
                desired_plan.desired_object_fingerprint_map_digest().clone(),
                desired_plan.desired_support_graph_digest().clone(),
                working_infobase_base_cursor,
                recorded_object_version_map_digest,
                desired_plan.exclusive_lease_capability_id().clone(),
            ),
        )
        .map_err(SupportRecoveryAuthorityError::from)?;
        Ok(self.bind_replan_working_infobase_closure_plan(plan))
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn corrective_instruction(
        &self,
        required_root_transitions: Vec<SupportRecoveryTransition>,
        required_content_restorations: Vec<SupportContentRestoration>,
        distribution_handoffs: Vec<SupportRecoveryDistributionHandoff>,
        handoff_revalidations: Vec<SupportRecoveryHandoffRevalidation>,
        lock_closure_resolver: &dyn SupportCorrectiveLockClosureResolver,
    ) -> Result<SupportCorrectiveInstruction, SupportRecoveryAuthorityError> {
        let binding = self
            .authorization
            .armed_binding()
            .ok_or(SupportRecoveryAuthorityError(
                "approved authority lost its frozen armed binding",
            ))?;
        if distribution_handoffs.iter().any(|handoff| {
            !binding
                .support_recovery_distributions()
                .iter()
                .any(|distribution| distribution.handoff() == handoff)
        }) {
            return Err(SupportRecoveryAuthorityError(
                "corrective instruction substituted a frozen distribution handoff",
            ));
        }
        let candidate = SupportCorrectiveInstruction::new(
            SupportCorrectiveInstructionAuthority::from_approved(
                &self.token,
                self.authorization.support_action_id().clone(),
                binding.purpose(),
                self.authorization.manual_target_mode(),
                self.authorization.manual_actor_username().clone(),
                binding.manual_working_infobase_identity().cloned(),
                self.projection.support_history_through_cursor().clone(),
                required_root_transitions,
                required_content_restorations,
                distribution_handoffs,
                handoff_revalidations,
                self.projection.support_recovery_finalization_plan(),
                lock_closure_resolver,
            )?,
        )?;
        let expected = self
            .projection
            .required_external_action()
            .and_then(|action| action.corrective_instruction())
            .ok_or(SupportRecoveryAuthorityError(
                "approved recovery plan does not require a corrective instruction",
            ))?;
        (candidate == *expected)
            .then_some(candidate)
            .ok_or(SupportRecoveryAuthorityError(
                "corrective instruction is not byte-identical to the approved recovery action",
            ))
    }

    fn guard_plan(
        &self,
    ) -> Result<SupportRecoveryGuardPlanAuthority, SupportRecoveryAuthorityError> {
        if self.projection.required_external_action().is_some() {
            return Err(SupportRecoveryAuthorityError(
                "support recovery guard cannot begin while an external action remains required",
            ));
        }
        if self.authorization.manual_target_mode()
            == ManualSupportTargetMode::SeparateWorkingInfobase
            && self
                .working_infobase_closure_plan()
                .is_none_or(|plan| plan.materialized().is_err())
        {
            return Err(SupportRecoveryAuthorityError(
                "separate-mode guard requires its exact materialized working-infobase closure plan",
            ));
        }
        SupportRecoveryGuardPlanAuthority::from_approved(
            &self.token,
            self.authorization.clone(),
            self.projection.support_recovery_finalization_plan(),
        )
        .map_err(Into::into)
    }

    pub(crate) fn acquire_guard(
        self,
        capability: &dyn SupportRecoveryGuardAcquisitionCapability,
    ) -> Result<ApprovedSupportRecoveryGuardAttempt, SupportRecoveryAuthorityError> {
        let plan = self.guard_plan()?;
        let observation = capability
            .acquire_guard(
                self.recovery_digest(),
                self.finalization_plan(),
                self.working_infobase_closure_plan(),
            )
            .map_err(|()| {
                SupportRecoveryAuthorityError(
                    "support recovery guard capability could not prove its acquisition attempt",
                )
            })?;
        match observation {
            SupportRecoveryGuardAcquisitionObservation::BlockedBeforeRoot {
                guard_receipt_id,
                failed_target,
                failed_target_display,
                locked_by,
            } => {
                let authority = SupportRecoveryGuardAuthority::blocked_before_root_from_approved(
                    &self.token,
                    plan,
                    guard_receipt_id,
                    failed_target,
                    failed_target_display,
                    locked_by,
                )?;
                let guard_proof = SupportRecoveryGuardProof::new(authority)?;
                Ok(ApprovedSupportRecoveryGuardAttempt::Blocked(Box::new(
                    CurrentBlockedSupportRecoveryGuardAttempt::new(self, guard_proof),
                )))
            }
            SupportRecoveryGuardAcquisitionObservation::BlockedAfterPartial {
                guard_receipt_id,
                acquired_in_order,
                failed_target,
                failed_target_display,
                locked_by,
                released_in_reverse_order,
            } => {
                let authority = SupportRecoveryGuardAuthority::blocked_after_partial_from_approved(
                    &self.token,
                    plan,
                    guard_receipt_id,
                    acquired_in_order,
                    failed_target,
                    failed_target_display,
                    locked_by,
                    released_in_reverse_order,
                )?;
                let guard_proof = SupportRecoveryGuardProof::new(authority)?;
                Ok(ApprovedSupportRecoveryGuardAttempt::Blocked(Box::new(
                    CurrentBlockedSupportRecoveryGuardAttempt::new(self, guard_proof),
                )))
            }
            SupportRecoveryGuardAcquisitionObservation::Acquired {
                guard_receipt_id,
                acquired_root_first,
            } => {
                if acquired_root_first.as_slice()
                    != self.finalization_plan().lock_targets().as_slice()
                {
                    return Err(SupportRecoveryAuthorityError(
                        "guard acquisition did not prove the exact complete root-first target set",
                    ));
                }
                Ok(ApprovedSupportRecoveryGuardAttempt::Acquired(Box::new(
                    ApprovedSupportRecoveryGuardWindow {
                        authority: self,
                        guard_receipt_id,
                        acquired_root_first,
                    },
                )))
            }
        }
    }

    fn validated_guard_release_receipt(
        &self,
        guard_receipt_id: &UnicaId,
        acquired_root_first: &SupportRecoveryAcquiredLockTargets,
        observation: SupportRecoveryGuardReleaseObservation,
    ) -> Result<UnicaId, SupportRecoveryAuthorityError> {
        let expected_reverse = acquired_root_first.as_slice().iter().rev();
        if observation.guard_receipt_id != *guard_receipt_id
            || observation.guard_release_receipt_id == *guard_receipt_id
            || !observation
                .released_in_reverse_order
                .as_slice()
                .iter()
                .eq(expected_reverse)
        {
            return Err(SupportRecoveryAuthorityError(
                "guard release capability did not prove the exact acquired window",
            ));
        }
        Ok(observation.guard_release_receipt_id)
    }

    #[allow(clippy::too_many_arguments)]
    fn stopped_after_complete_guard_proof(
        &self,
        guard_receipt_id: &UnicaId,
        acquired_root_first: &SupportRecoveryAcquiredLockTargets,
        manual_actor_lock_inventory_proof: Option<ManualActorLockInventoryProof>,
        reserved_original_lease_stop_evidence: Option<ReservedOriginalLeaseStopEvidence>,
        manual_working_infobase_stop_evidence: Option<ManualWorkingInfobaseStopEvidence>,
        release_capability: &dyn SupportRecoveryGuardReleaseCapability,
        recheck_resolver: &dyn SupportRecoveryUnderGuardRecheckResolver,
    ) -> Result<SupportRecoveryGuardProof, SupportRecoveryAuthorityError> {
        if let Some(stop) = manual_working_infobase_stop_evidence.as_ref() {
            let plan =
                self.working_infobase_closure_plan()
                    .ok_or(SupportRecoveryAuthorityError(
                        "reserved-mode recovery cannot carry a working-IB stop reason",
                    ))?;
            if stop.closure_plan_digest() != plan.plan_digest()
                || stop.working_infobase_identity() != plan.working_infobase_identity()
            {
                return Err(SupportRecoveryAuthorityError(
                    "working-IB stop reason belongs to another closure plan",
                ));
            }
        }
        recheck_resolver
            .verify_current_destination(
                self.recovery_digest(),
                self.finalization_plan(),
                self.projection.support_history_partition(),
                guard_receipt_id,
            )
            .map_err(|()| {
                SupportRecoveryAuthorityError(
                    "under-guard destination recheck did not prove the approved state",
                )
            })?;
        let release_observation = release_capability
            .release_complete_guard(
                self.recovery_digest(),
                self.finalization_plan(),
                guard_receipt_id,
            )
            .map_err(|()| {
                SupportRecoveryAuthorityError(
                    "guard release capability did not return a verified release receipt",
                )
            })?;
        let guard_release_receipt_id = self.validated_guard_release_receipt(
            guard_receipt_id,
            acquired_root_first,
            release_observation,
        )?;
        let partition = self.projection.support_history_partition();
        let authority = SupportRecoveryGuardAuthority::stopped_after_complete_guard_from_approved(
            &self.token,
            self.guard_plan()?,
            guard_receipt_id.clone(),
            guard_release_receipt_id,
            manual_actor_lock_inventory_proof,
            reserved_original_lease_stop_evidence,
            manual_working_infobase_stop_evidence,
            partition.start_cursor().clone(),
            partition.through_inclusive().clone(),
            partition.partition_digest().clone(),
        )?;
        SupportRecoveryGuardProof::new(authority).map_err(Into::into)
    }

    fn execute_and_terminalize_under_guard(
        &self,
        guard_receipt_id: &UnicaId,
        execution_capability: &dyn SupportRecoveryFinalizationExecutionCapability,
        recheck_resolver: &dyn SupportRecoveryUnderGuardRecheckResolver,
    ) -> Result<
        (
            SelectiveRepositoryUpdateProof,
            SupportRecoveryAuthorizationTerminalizationReceipt,
        ),
        SupportRecoveryAuthorityError,
    > {
        if self.projection.required_external_action().is_some() {
            return Err(SupportRecoveryAuthorityError(
                "support recovery cannot complete while an external action remains required",
            ));
        }
        recheck_resolver
            .verify_current_destination(
                self.recovery_digest(),
                self.finalization_plan(),
                self.projection.support_history_partition(),
                guard_receipt_id,
            )
            .map_err(|()| {
                SupportRecoveryAuthorityError(
                    "under-guard destination recheck did not prove the approved state",
                )
            })?;
        let selective_update_observation = execution_capability
            .execute_selective_update(
                self.recovery_digest(),
                self.finalization_plan(),
                self.projection.support_history_partition(),
                guard_receipt_id,
            )
            .map_err(|()| {
                SupportRecoveryAuthorityError(
                    "finalization execution capability did not prove its guarded selective update",
                )
            })?;
        let selective_plan = self
            .finalization_plan()
            .materialized_selective_update_plan()
            .ok_or(SupportRecoveryAuthorityError(
                "support recovery completion requires a materialized selective plan",
            ))?;
        let selective_update_proof =
            SelectiveRepositoryUpdateProof::recovery_finalization_from_approved(
                &self.token,
                selective_plan,
                selective_update_observation,
            )
            .map_err(|_| {
                SupportRecoveryAuthorityError(
                    "finalization execution observation cannot mint the approved selective proof",
                )
            })?;
        let terminalization_observation = execution_capability
            .terminalize_authorization(
                self.recovery_digest(),
                self.support_action_id(),
                self.support_action_digest(),
                &selective_update_proof,
            )
            .map_err(|()| {
                SupportRecoveryAuthorityError(
                    "finalization execution capability did not prove durable authorization terminalization",
                )
            })?;
        let expected_outcome = match self.projection.support_recovery_disposition() {
            SupportRecoveryDisposition::RestoreThenReauthorize
            | SupportRecoveryDisposition::PreserveExternalAndReauthorize => {
                CompletedSupportRecoveryAuthorizationOutcome::Cancelled
            }
            SupportRecoveryDisposition::RestoreThenAbandon => {
                CompletedSupportRecoveryAuthorizationOutcome::AbandonmentFinalized
            }
        };
        if terminalization_observation.support_action_id != *self.support_action_id()
            || terminalization_observation.support_action_digest != *self.support_action_digest()
            || terminalization_observation.authorization_outcome != expected_outcome
        {
            return Err(SupportRecoveryAuthorityError(
                "durable authorization terminalization observation was substituted",
            ));
        }
        let authorization_terminalization_receipt =
            SupportRecoveryAuthorizationTerminalizationReceipt::from_approved(
                &self.token,
                terminalization_observation.support_action_id,
                terminalization_observation.support_action_digest,
                terminalization_observation.terminalization_receipt_id,
                terminalization_observation.authorization_outcome,
            )?;
        Ok((
            selective_update_proof,
            authorization_terminalization_receipt,
        ))
    }

    #[allow(clippy::too_many_arguments)]
    fn completed_after_terminalization(
        &self,
        guard_receipt_id: &UnicaId,
        acquired_root_first: &SupportRecoveryAcquiredLockTargets,
        manual_actor_lock_inventory_proof: Option<ManualActorLockInventoryProof>,
        reserved_original_terminalization_proof: Option<ReservedOriginalTerminalizationProof>,
        manual_working_infobase_closure_proof: Option<ManualWorkingInfobaseClosureProof>,
        selective_update_proof: SelectiveRepositoryUpdateProof,
        authorization_terminalization_receipt: SupportRecoveryAuthorizationTerminalizationReceipt,
        guard_release_capability: &dyn SupportRecoveryGuardReleaseCapability,
        post_release_scanner: &dyn SupportRecoveryPostReleaseHistoryScanner,
    ) -> Result<ApprovedSupportRecoveryCompletion, SupportRecoveryAuthorityError> {
        let closure_presence_matches = match self.authorization.manual_target_mode() {
            ManualSupportTargetMode::ReservedOriginal => {
                manual_working_infobase_closure_proof.is_none()
            }
            ManualSupportTargetMode::SeparateWorkingInfobase => {
                let (Some(plan), Some(proof)) = (
                    self.working_infobase_closure_plan(),
                    manual_working_infobase_closure_proof.as_ref(),
                ) else {
                    return Err(SupportRecoveryAuthorityError(
                        "separate-mode completion requires its exact working-IB closure proof",
                    ));
                };
                proof.plan_digest() == plan.plan_digest()
                    && proof.working_infobase_identity() == plan.working_infobase_identity()
            }
        };
        if !closure_presence_matches
            || authorization_terminalization_receipt.support_action_id() != self.support_action_id()
            || authorization_terminalization_receipt.support_action_digest()
                != self.support_action_digest()
        {
            return Err(SupportRecoveryAuthorityError(
                "mode closure proof or authorization terminalization receipt is invalid",
            ));
        }
        let release_observation = guard_release_capability
            .release_complete_guard(
                self.recovery_digest(),
                self.finalization_plan(),
                guard_receipt_id,
            )
            .map_err(|()| {
                SupportRecoveryAuthorityError(
                    "guard release capability did not return a verified release receipt",
                )
            })?;
        let guard_release_receipt_id = self.validated_guard_release_receipt(
            guard_receipt_id,
            acquired_root_first,
            release_observation,
        )?;
        let (post_release_partition, deferred_repository_advance) = post_release_scanner
            .scan_after_release(self.projection.support_history_through_cursor())
            .map_err(|()| {
                SupportRecoveryAuthorityError(
                    "post-release capability scan did not return a complete tail",
                )
            })?;
        if post_release_partition.start_cursor() != self.projection.support_history_through_cursor()
            || !post_release_tail_is_allowed(&post_release_partition)
            || deferred_repository_advance.as_ref().is_some_and(|advance| {
                advance.anchor_cursor() != post_release_partition.through_inclusive()
            })
        {
            return Err(SupportRecoveryAuthorityError(
                "post-release tail has a wrong anchor, forbidden entry, or deferred successor",
            ));
        }
        let result_phase =
            if partition_requires_late_phase(self.projection.support_history_partition())
                || partition_requires_late_phase(&post_release_partition)
                || deferred_repository_advance.is_some()
            {
                self.projection.support_late_relevant_result_phase()
            } else {
                self.projection.planned_result_phase()
            };
        let authority = SupportRecoveryGuardAuthority::completed_from_approved(
            &self.token,
            self.guard_plan()?,
            guard_receipt_id.clone(),
            guard_release_receipt_id,
            manual_actor_lock_inventory_proof,
            reserved_original_terminalization_proof,
            manual_working_infobase_closure_proof,
            self.projection.support_history_partition(),
            selective_update_proof,
            post_release_partition,
            deferred_repository_advance,
            authorization_terminalization_receipt,
        )?;
        Ok(ApprovedSupportRecoveryCompletion {
            guard_proof: SupportRecoveryGuardProof::new(authority)?,
            result_phase,
        })
    }

    fn working_infobase_live_lease(
        &self,
        capability: &dyn ManualWorkingInfobaseTerminalLeaseCapability,
    ) -> Result<ManualWorkingInfobaseLiveLeaseWindow, SupportRecoveryAuthorityError> {
        let plan = self
            .working_infobase_closure_plan()
            .ok_or(SupportRecoveryAuthorityError(
                "reserved-mode recovery has no working-infobase closure plan",
            ))?;
        let acquired_lease = capability
            .acquire(self.recovery_digest(), plan)
            .map_err(|()| {
                SupportRecoveryAuthorityError(
                    "working-IB closure capability could not acquire its live lease",
                )
            })?;
        let expected_receipt_id = acquired_lease.exclusive_lease_receipt_id().clone();
        let live_lease = capability
            .inspect(self.recovery_digest(), plan, acquired_lease)
            .map_err(|()| {
                SupportRecoveryAuthorityError(
                    "working-IB closure capability could not inspect its acquired lease",
                )
            })?;
        plan.materialized()?;
        if live_lease.exclusive_lease_receipt_id != expected_receipt_id
            || Some(&live_lease.working_infobase_base_cursor) != plan.working_infobase_base_cursor()
            || Some(&live_lease.recorded_object_version_map_digest)
                != plan.recorded_object_version_map_digest()
            || live_lease.final_current_fingerprint != *plan.desired_base_fingerprint()
            || live_lease.final_base_fingerprint != *plan.desired_base_fingerprint()
            || live_lease.final_object_fingerprint_map_digest
                != *plan.desired_object_fingerprint_map_digest()
            || live_lease.final_support_graph_digest != *plan.desired_support_graph_digest()
        {
            return Err(SupportRecoveryAuthorityError(
                "working-IB live lease inspection differs from its materialized closure plan",
            ));
        }
        Ok(live_lease)
    }

    fn release_working_infobase_after_terminalization(
        &self,
        live_lease: ManualWorkingInfobaseLiveLeaseWindow,
        terminalization_receipt: &SupportRecoveryAuthorizationTerminalizationReceipt,
        capability: &dyn ManualWorkingInfobaseTerminalLeaseCapability,
    ) -> Result<ManualWorkingInfobaseClosureProof, SupportRecoveryAuthorityError> {
        let plan = self
            .working_infobase_closure_plan()
            .ok_or(SupportRecoveryAuthorityError(
                "reserved-mode recovery has no working-infobase closure plan",
            ))?;
        let exclusive_lease_receipt_id = live_lease.exclusive_lease_receipt_id.clone();
        let working_infobase_base_cursor = live_lease.working_infobase_base_cursor.clone();
        let recorded_object_version_map_digest =
            live_lease.recorded_object_version_map_digest.clone();
        let final_current_fingerprint = live_lease.final_current_fingerprint.clone();
        let final_base_fingerprint = live_lease.final_base_fingerprint.clone();
        let final_object_fingerprint_map_digest =
            live_lease.final_object_fingerprint_map_digest.clone();
        let final_support_graph_digest = live_lease.final_support_graph_digest.clone();
        let release = capability
            .release_after_terminalization(
                self.recovery_digest(),
                plan,
                live_lease,
                terminalization_receipt,
            )
            .map_err(|()| {
                SupportRecoveryAuthorityError(
                    "working-IB lease release was not proven after authorization terminalization",
                )
            })?;
        if release.exclusive_lease_receipt_id != exclusive_lease_receipt_id
            || release.exclusive_lease_release_receipt_id == exclusive_lease_receipt_id
            || release.authorization_terminalization_receipt_digest
                != *terminalization_receipt.receipt_digest()
        {
            return Err(SupportRecoveryAuthorityError(
                "working-IB lease release belongs to another acquisition or terminalization",
            ));
        }
        let authority = ManualWorkingInfobaseClosureExecutionAuthority::from_approved_observation(
            &self.token,
            plan,
            exclusive_lease_receipt_id,
            release.exclusive_lease_release_receipt_id,
            working_infobase_base_cursor,
            recorded_object_version_map_digest,
            final_current_fingerprint,
            final_base_fingerprint,
            final_object_fingerprint_map_digest,
            final_support_graph_digest,
        )?;
        ManualWorkingInfobaseClosureProof::new(plan, authority).map_err(Into::into)
    }

    fn reserved_original_busy_stop_evidence(
        &self,
        capability: &dyn ReservedOriginalLeaseBusyCapability,
    ) -> Result<
        (
            ManualActorLockInventoryProof,
            ReservedOriginalLeaseStopEvidence,
        ),
        SupportRecoveryAuthorityError,
    > {
        let observation = capability
            .observe_lease_busy(self.recovery_digest(), self.finalization_plan())
            .map_err(|()| {
                SupportRecoveryAuthorityError(
                    "reserved-original lease-busy capability returned no observation",
                )
            })?;
        let expected_baseline = self
            .authorization
            .manual_actor_lock_baseline_digest()
            .ok_or(SupportRecoveryAuthorityError(
                "separate mode has no reserved manual-actor lock baseline",
            ))?;
        let expected_capability = self
            .authorization
            .reserved_original_lease_capability_id()
            .ok_or(SupportRecoveryAuthorityError(
                "separate mode has no reserved-original lease capability",
            ))?;
        if observation.manual_actor_username != *self.authorization.manual_actor_username()
            || observation.baseline_lock_set_digest != *expected_baseline
            || observation.observed_lock_set_digest != *expected_baseline
            || observation.reserved_original_identity_digest
                != *self.authorization.reserved_original_identity_digest()
            || observation.exclusive_lease_capability_id != *expected_capability
        {
            return Err(SupportRecoveryAuthorityError(
                "reserved-original busy observation substituted frozen mode evidence",
            ));
        }
        let inventory = ManualActorLockInventoryProof::new(
            observation.manual_actor_username,
            observation.baseline_lock_set_digest,
            observation.observed_lock_set_digest,
        )
        .map_err(|_| SupportRecoveryAuthorityError("reserved lock inventory proof is invalid"))?;
        let stop = ReservedOriginalLeaseStopEvidence::new(
            observation.reserved_original_identity_digest,
            observation.exclusive_lease_capability_id,
            observation.lease_owner,
        )
        .map_err(|_| SupportRecoveryAuthorityError("reserved lease-stop evidence is invalid"))?;
        Ok((inventory, stop))
    }

    fn reserved_original_live_lease(
        &self,
        capability: &dyn ReservedOriginalTerminalLeaseCapability,
    ) -> Result<ReservedOriginalLiveLeaseWindow, SupportRecoveryAuthorityError> {
        let acquired_lease = capability
            .acquire(self.recovery_digest(), self.finalization_plan())
            .map_err(|()| {
                SupportRecoveryAuthorityError(
                    "reserved-original capability could not acquire its live lease",
                )
            })?;
        let expected_receipt_id = acquired_lease.exclusive_lease_receipt_id().clone();
        let live_lease = capability
            .inspect(
                self.recovery_digest(),
                self.finalization_plan(),
                acquired_lease,
            )
            .map_err(|()| {
                SupportRecoveryAuthorityError(
                    "reserved-original capability could not inspect its acquired lease",
                )
            })?;
        let expected_baseline = self
            .authorization
            .manual_actor_lock_baseline_digest()
            .ok_or(SupportRecoveryAuthorityError(
                "separate mode has no reserved manual-actor lock baseline",
            ))?;
        let expected_capability = self
            .authorization
            .reserved_original_lease_capability_id()
            .ok_or(SupportRecoveryAuthorityError(
                "separate mode has no reserved-original lease capability",
            ))?;
        if live_lease.exclusive_lease_receipt_id != expected_receipt_id
            || live_lease.manual_actor_username != *self.authorization.manual_actor_username()
            || live_lease.baseline_lock_set_digest != *expected_baseline
            || live_lease.observed_lock_set_digest != *expected_baseline
            || live_lease.reserved_original_identity_digest
                != *self.authorization.reserved_original_identity_digest()
            || live_lease.exclusive_lease_capability_id != *expected_capability
            || live_lease.expected_repository_fingerprint
                != *self.authorization.expected_original_fingerprint()
            || live_lease.observed_original_fingerprint
                != *self.authorization.expected_original_fingerprint()
        {
            return Err(SupportRecoveryAuthorityError(
                "reserved-original live lease substituted frozen mode evidence",
            ));
        }
        Ok(live_lease)
    }

    fn release_reserved_original_after_terminalization(
        &self,
        live_lease: ReservedOriginalLiveLeaseWindow,
        terminalization_receipt: &SupportRecoveryAuthorizationTerminalizationReceipt,
        capability: &dyn ReservedOriginalTerminalLeaseCapability,
    ) -> Result<
        (
            ManualActorLockInventoryProof,
            ReservedOriginalTerminalizationProof,
        ),
        SupportRecoveryAuthorityError,
    > {
        let manual_actor_username = live_lease.manual_actor_username.clone();
        let baseline_lock_set_digest = live_lease.baseline_lock_set_digest.clone();
        let observed_lock_set_digest = live_lease.observed_lock_set_digest.clone();
        let reserved_original_identity_digest =
            live_lease.reserved_original_identity_digest.clone();
        let exclusive_lease_capability_id = live_lease.exclusive_lease_capability_id.clone();
        let exclusive_lease_receipt_id = live_lease.exclusive_lease_receipt_id.clone();
        let expected_repository_fingerprint = live_lease.expected_repository_fingerprint.clone();
        let observed_original_fingerprint = live_lease.observed_original_fingerprint.clone();
        let release = capability
            .release_after_terminalization(
                self.recovery_digest(),
                self.finalization_plan(),
                live_lease,
                terminalization_receipt,
            )
            .map_err(|()| {
                SupportRecoveryAuthorityError(
                    "reserved-original lease release was not proven after authorization terminalization",
                )
            })?;
        if release.exclusive_lease_receipt_id != exclusive_lease_receipt_id
            || release.exclusive_lease_release_receipt_id == exclusive_lease_receipt_id
            || release.authorization_terminalization_receipt_digest
                != *terminalization_receipt.receipt_digest()
        {
            return Err(SupportRecoveryAuthorityError(
                "reserved-original lease release belongs to another acquisition or terminalization",
            ));
        }
        let inventory = ManualActorLockInventoryProof::new(
            manual_actor_username,
            baseline_lock_set_digest,
            observed_lock_set_digest,
        )
        .map_err(|_| SupportRecoveryAuthorityError("reserved lock inventory proof is invalid"))?;
        let terminalization = ReservedOriginalTerminalizationProof::new(
            reserved_original_identity_digest,
            exclusive_lease_capability_id,
            exclusive_lease_receipt_id,
            release.exclusive_lease_release_receipt_id,
            expected_repository_fingerprint,
            observed_original_fingerprint,
        )
        .map_err(|_| SupportRecoveryAuthorityError("reserved terminalization proof is invalid"))?;
        Ok((inventory, terminalization))
    }

    fn working_infobase_lease_busy_stop(
        &self,
        capability: &dyn ManualWorkingInfobaseLeaseBusyCapability,
    ) -> Result<ManualWorkingInfobaseStopEvidence, SupportRecoveryAuthorityError> {
        let plan = self
            .working_infobase_closure_plan()
            .ok_or(SupportRecoveryAuthorityError(
                "reserved-mode recovery has no working-infobase closure plan",
            ))?;
        let owner = capability
            .observe_lease_owner(self.recovery_digest(), plan)
            .map_err(|()| {
                SupportRecoveryAuthorityError(
                    "working-IB lease-busy capability did not prove its observation",
                )
            })?;
        let authority =
            ManualWorkingInfobaseStopAuthority::lease_busy_from_approved(&self.token, plan, owner)?;
        ManualWorkingInfobaseStopEvidence::new(plan, authority).map_err(Into::into)
    }

    fn working_infobase_dirty_stop(
        &self,
        capability: &dyn ManualWorkingInfobaseDirtyCapability,
    ) -> Result<ManualWorkingInfobaseStopEvidence, SupportRecoveryAuthorityError> {
        let plan = self
            .working_infobase_closure_plan()
            .ok_or(SupportRecoveryAuthorityError(
                "reserved-mode recovery has no working-infobase closure plan",
            ))?;
        let (
            observed_working_infobase_fingerprint,
            observed_support_graph_digest,
            exclusive_lease_receipt_id,
            exclusive_lease_release_receipt_id,
        ) = capability
            .inspect_dirty_and_release(self.recovery_digest(), plan)
            .map_err(|()| {
                SupportRecoveryAuthorityError(
                    "working-IB dirty-stop capability did not prove its lease window",
                )
            })?;
        let authority = ManualWorkingInfobaseStopAuthority::lease_acquired_dirty_from_approved(
            &self.token,
            plan,
            observed_working_infobase_fingerprint,
            observed_support_graph_digest,
            exclusive_lease_receipt_id,
            exclusive_lease_release_receipt_id,
        )?;
        ManualWorkingInfobaseStopEvidence::new(plan, authority).map_err(Into::into)
    }
}

impl ApprovedSupportRecoveryGuardWindow {
    pub(crate) fn prepare_reserved_original_lease_busy_stop(
        self,
        reserved_capability: &dyn ReservedOriginalLeaseBusyCapability,
    ) -> Result<PreparedReservedSupportRecoveryStopWindow, SupportRecoveryAuthorityError> {
        let (manual_actor_lock_inventory_proof, stop_evidence) = self
            .authority
            .reserved_original_busy_stop_evidence(reserved_capability)?;
        Ok(PreparedReservedSupportRecoveryStopWindow {
            window: self,
            manual_actor_lock_inventory_proof,
            stop_evidence,
        })
    }

    #[cfg(test)]
    pub(crate) fn stopped_reserved_after_complete_guard_proof(
        self,
        reserved_capability: &dyn ReservedOriginalLeaseBusyCapability,
        release_capability: &dyn SupportRecoveryGuardReleaseCapability,
        recheck_resolver: &dyn SupportRecoveryUnderGuardRecheckResolver,
    ) -> Result<SupportRecoveryGuardProof, SupportRecoveryAuthorityError> {
        self.prepare_reserved_original_lease_busy_stop(reserved_capability)?
            .stopped_after_complete_guard_proof(release_capability, recheck_resolver)
    }

    pub(crate) fn prepare_reserved_original_completion(
        self,
        capability: &dyn ReservedOriginalTerminalLeaseCapability,
    ) -> Result<PreparedReservedSupportRecoveryCompletionWindow, SupportRecoveryAuthorityError>
    {
        let live_lease = self.authority.reserved_original_live_lease(capability)?;
        Ok(PreparedReservedSupportRecoveryCompletionWindow {
            window: self,
            live_lease,
        })
    }

    pub(crate) fn prepare_working_infobase_completion(
        self,
        capability: &dyn ManualWorkingInfobaseTerminalLeaseCapability,
    ) -> Result<PreparedSupportRecoveryCompletionWindow, SupportRecoveryAuthorityError> {
        let live_lease = self.authority.working_infobase_live_lease(capability)?;
        Ok(PreparedSupportRecoveryCompletionWindow {
            window: self,
            live_lease,
        })
    }

    pub(crate) fn prepare_working_infobase_lease_busy_stop(
        self,
        capability: &dyn ManualWorkingInfobaseLeaseBusyCapability,
    ) -> Result<PreparedSupportRecoveryStopWindow, SupportRecoveryAuthorityError> {
        let stop_evidence = self
            .authority
            .working_infobase_lease_busy_stop(capability)?;
        Ok(PreparedSupportRecoveryStopWindow {
            window: self,
            stop_evidence,
        })
    }

    pub(crate) fn prepare_working_infobase_dirty_stop(
        self,
        capability: &dyn ManualWorkingInfobaseDirtyCapability,
    ) -> Result<PreparedSupportRecoveryStopWindow, SupportRecoveryAuthorityError> {
        let stop_evidence = self.authority.working_infobase_dirty_stop(capability)?;
        Ok(PreparedSupportRecoveryStopWindow {
            window: self,
            stop_evidence,
        })
    }
}

impl PreparedSupportRecoveryStopWindow {
    #[cfg(test)]
    pub(crate) fn stopped_after_complete_guard_proof(
        self,
        release_capability: &dyn SupportRecoveryGuardReleaseCapability,
        recheck_resolver: &dyn SupportRecoveryUnderGuardRecheckResolver,
    ) -> Result<SupportRecoveryGuardProof, SupportRecoveryAuthorityError> {
        self.window.authority.stopped_after_complete_guard_proof(
            &self.window.guard_receipt_id,
            &self.window.acquired_root_first,
            None,
            None,
            Some(self.stop_evidence),
            release_capability,
            recheck_resolver,
        )
    }

    pub(crate) fn stopped_with_cleanup_wait(
        self,
        action_id: UnicaId,
        expected_available_lease_digest: Sha256Digest,
        release_capability: &dyn SupportRecoveryGuardReleaseCapability,
        recheck_resolver: &dyn SupportRecoveryUnderGuardRecheckResolver,
    ) -> Result<PreparedSupportRecoveryExternalReplan, SupportRecoveryAuthorityError> {
        let Self {
            window:
                ApprovedSupportRecoveryGuardWindow {
                    authority,
                    guard_receipt_id,
                    acquired_root_first,
                },
            stop_evidence,
        } = self;
        let external_wait = authority.clean_working_infobase_wait_from_stop(
            action_id,
            &stop_evidence,
            expected_available_lease_digest,
        )?;
        let latest_guard_proof = authority.stopped_after_complete_guard_proof(
            &guard_receipt_id,
            &acquired_root_first,
            None,
            None,
            Some(stop_evidence),
            release_capability,
            recheck_resolver,
        )?;
        let external_wait = external_wait.require_guard_proof(&latest_guard_proof);
        Ok(PreparedSupportRecoveryExternalReplan {
            authority,
            latest_guard_proof,
            external_wait,
        })
    }
}

impl PreparedReservedSupportRecoveryStopWindow {
    #[cfg(test)]
    pub(crate) fn stopped_after_complete_guard_proof(
        self,
        release_capability: &dyn SupportRecoveryGuardReleaseCapability,
        recheck_resolver: &dyn SupportRecoveryUnderGuardRecheckResolver,
    ) -> Result<SupportRecoveryGuardProof, SupportRecoveryAuthorityError> {
        self.window.authority.stopped_after_complete_guard_proof(
            &self.window.guard_receipt_id,
            &self.window.acquired_root_first,
            Some(self.manual_actor_lock_inventory_proof),
            Some(self.stop_evidence),
            None,
            release_capability,
            recheck_resolver,
        )
    }

    pub(crate) fn stopped_with_closure_wait(
        self,
        action_id: UnicaId,
        expected_available_lease_digest: Sha256Digest,
        release_capability: &dyn SupportRecoveryGuardReleaseCapability,
        recheck_resolver: &dyn SupportRecoveryUnderGuardRecheckResolver,
    ) -> Result<PreparedSupportRecoveryExternalReplan, SupportRecoveryAuthorityError> {
        let Self {
            window:
                ApprovedSupportRecoveryGuardWindow {
                    authority,
                    guard_receipt_id,
                    acquired_root_first,
                },
            manual_actor_lock_inventory_proof,
            stop_evidence,
        } = self;
        let external_wait = authority.close_reserved_original_wait_from_stop(
            action_id,
            &stop_evidence,
            expected_available_lease_digest,
        )?;
        let latest_guard_proof = authority.stopped_after_complete_guard_proof(
            &guard_receipt_id,
            &acquired_root_first,
            Some(manual_actor_lock_inventory_proof),
            Some(stop_evidence),
            None,
            release_capability,
            recheck_resolver,
        )?;
        let external_wait = external_wait.require_guard_proof(&latest_guard_proof);
        Ok(PreparedSupportRecoveryExternalReplan {
            authority,
            latest_guard_proof,
            external_wait,
        })
    }
}

impl PreparedSupportRecoveryCompletionWindow {
    pub(crate) fn completed_guard_proof(
        self,
        execution_capability: &dyn SupportRecoveryFinalizationExecutionCapability,
        lease_capability: &dyn ManualWorkingInfobaseTerminalLeaseCapability,
        guard_release_capability: &dyn SupportRecoveryGuardReleaseCapability,
        recheck_resolver: &dyn SupportRecoveryUnderGuardRecheckResolver,
        post_release_scanner: &dyn SupportRecoveryPostReleaseHistoryScanner,
    ) -> Result<ApprovedSupportRecoveryCompletion, SupportRecoveryAuthorityError> {
        let (selective_update_proof, terminalization_receipt) =
            self.window.authority.execute_and_terminalize_under_guard(
                &self.window.guard_receipt_id,
                execution_capability,
                recheck_resolver,
            )?;
        let closure_proof = self
            .window
            .authority
            .release_working_infobase_after_terminalization(
                self.live_lease,
                &terminalization_receipt,
                lease_capability,
            )?;
        self.window.authority.completed_after_terminalization(
            &self.window.guard_receipt_id,
            &self.window.acquired_root_first,
            None,
            None,
            Some(closure_proof),
            selective_update_proof,
            terminalization_receipt,
            guard_release_capability,
            post_release_scanner,
        )
    }
}

impl PreparedReservedSupportRecoveryCompletionWindow {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn completed_guard_proof(
        self,
        execution_capability: &dyn SupportRecoveryFinalizationExecutionCapability,
        lease_capability: &dyn ReservedOriginalTerminalLeaseCapability,
        guard_release_capability: &dyn SupportRecoveryGuardReleaseCapability,
        recheck_resolver: &dyn SupportRecoveryUnderGuardRecheckResolver,
        post_release_scanner: &dyn SupportRecoveryPostReleaseHistoryScanner,
    ) -> Result<ApprovedSupportRecoveryCompletion, SupportRecoveryAuthorityError> {
        let (selective_update_proof, terminalization_receipt) =
            self.window.authority.execute_and_terminalize_under_guard(
                &self.window.guard_receipt_id,
                execution_capability,
                recheck_resolver,
            )?;
        let (inventory, terminalization_proof) = self
            .window
            .authority
            .release_reserved_original_after_terminalization(
                self.live_lease,
                &terminalization_receipt,
                lease_capability,
            )?;
        self.window.authority.completed_after_terminalization(
            &self.window.guard_receipt_id,
            &self.window.acquired_root_first,
            Some(inventory),
            Some(terminalization_proof),
            None,
            selective_update_proof,
            terminalization_receipt,
            guard_release_capability,
            post_release_scanner,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::branched_development::contracts::instructions::{
        SupportCorrectiveLockClosureResolutionError, SupportRecoveryExternalAction,
        VendorRestoredSupportState,
    };
    use crate::domain::branched_development::contracts::recovery::SupportRecoveryVersionObservations;
    use crate::domain::branched_development::contracts::repository::{
        CanonicalEmptyDeltaDigest, EvidenceKind, EvidenceSourceIndex, EvidenceSourceIndexCandidate,
        EvidenceSourceIndexCandidateRow, EvidenceSourceRegistry, NonConflictingConcurrentEvidence,
        RepositoryContractError, RepositoryHistoryEvidenceBytesResolver,
        RepositoryHistoryOrderEvidence, RepositoryHistoryOrderResolver,
        RepositoryHistoryPartitionResolver, RepositoryHistorySourceEvidenceRef,
        RepositoryTargetStates, RepositoryUpdateLockReason, RepositoryUpdateLockTargets,
        SelectiveRepositoryUpdatePlan, SupportRecoverySelectiveUpdateEffectObservation,
        UnvalidatedRepositoryHistoryPartition,
    };
    use crate::domain::branched_development::contracts::scalars::{
        Diagnostic, DisplayPath, RepositoryIdentityComponent, RepositoryTargetDisplay,
        RepositoryUsername, RepositoryVersion,
    };
    use crate::domain::branched_development::contracts::support::{
        ActiveSupportActionResumeHandle, ManualWorkingInfobaseBaseline,
        ManualWorkingInfobaseIdentity, RootReachableSupportLayerSet, SupportActionArmingReceipt,
        SupportActionAuthorizationAuthority, SupportActionAuthorizationInputs,
        SupportActionPhaseBinding, SupportActionPurpose, SupportBlockers, SupportContractError,
        SupportEvidenceGaps, SupportHistoryOrderAuthority,
        SupportRecoveryDistributionCoverageAuthority, SupportRecoveryDistributionEvidence,
        SupportRecoveryDistributionHandoff, SupportRecoveryDistributionHandoffInputs,
        SupportRootLockObservation, SupportTransition, SupportTransitionConflict,
        SupportTransitionConflicts, SupportTransitionOverlapKind, SupportTransitions,
        UserVisibleCfFileName,
    };
    use crate::domain::branched_development::contracts::support_terminalization::{
        ManualWorkingInfobaseClosurePlanAuthority, SupportRecoveryDesiredTarget,
        SupportRecoveryDesiredTargets, SupportRecoveryFinalizationPlanAuthority,
        SupportRecoveryLockTarget, SupportRecoveryLockTargets,
    };
    use crate::domain::branched_development::{
        CapabilityRowId, OperationId, ProfileArtifactRefId, SupportLayerId,
    };
    use serde_json::{json, Value};
    use sha2::{Digest, Sha256};
    use std::cell::RefCell;
    use std::cmp::Ordering;
    use std::collections::BTreeMap;
    use std::rc::Rc;
    use std::str::FromStr;

    const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const C: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
    const ID_1: &str = "11111111-1111-4111-8111-111111111111";
    const ID_2: &str = "22222222-2222-4222-8222-222222222222";
    const ID_3: &str = "33333333-3333-4333-8333-333333333333";
    const ID_4: &str = "44444444-4444-4444-8444-444444444444";

    fn digest(value: &str) -> Sha256Digest {
        Sha256Digest::parse(value).unwrap()
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

    fn owner(username: &str) -> RepositoryOwnerIdentity {
        serde_json::from_value(json!({
            "username": username,
            "computer": null,
            "infobase": null,
            "lockedAt": null,
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

    struct UnexpectedIndex;

    impl EvidenceSourceIndex for UnexpectedIndex {
        fn candidate_for(
            &self,
            _repository_version: &super::super::scalars::RepositoryVersion,
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

    fn raw_digest(value: &Value) -> Sha256Digest {
        Sha256Digest::parse(&format!(
            "{:x}",
            Sha256::digest(serde_json_canonicalizer::to_vec(value).unwrap())
        ))
        .unwrap()
    }

    fn history_cursor(version: &str, prefix_digest: &str) -> RepositoryHistoryCursor {
        serde_json::from_value(json!({
            "throughVersion": version,
            "historyPrefixDigest": prefix_digest,
        }))
        .unwrap()
    }

    fn finalize_history_observation(mut value: Value) -> SupportPrerequisiteVersionObservation {
        let mut record = value.clone();
        record
            .as_object_mut()
            .unwrap()
            .remove("classificationDigest");
        value["classificationDigest"] = json!(raw_digest(&record));
        serde_json::from_value(value).unwrap()
    }

    fn authorized_history_observation(
        version: &str,
        frozen: &FrozenSupportRecoveryAuthorizationProjection,
    ) -> SupportPrerequisiteVersionObservation {
        let binding = frozen.armed_binding().unwrap();
        let receipt = binding.arming_receipt();
        finalize_history_observation(json!({
            "repositoryVersion": version,
            "classification": "authorized",
            "classificationDigest": A,
            "mismatchKinds": [],
            "repositoryActor": {
                "username": "reserved-user",
                "computer": null,
                "infobase": null
            },
            "supportActionId": frozen.support_action_id(),
            "supportActionDigest": frozen.support_action_digest(),
            "armingReceiptId": receipt.arming_receipt_id(),
            "armingReceiptDigest": receipt.receipt_digest(),
            "firstRootSupportAfterArming": true,
            "actionAttributionEvidenceDigest": A,
            "authorizedTransitionsDigest": binding.authorized_transitions_digest(),
            "manualTargetMode": "reservedOriginal",
            "rootDeltaDigest": A,
            "contentDeltaDigest": CanonicalEmptyDeltaDigest::VALUE,
            "observedSupportTransitionsDigest": binding.authorized_transitions_digest(),
            "rootDeltaContainsOnlyAuthorizedSupportTransitions": true
        }))
    }

    fn external_history_observation(version: &str) -> SupportPrerequisiteVersionObservation {
        finalize_history_observation(json!({
            "repositoryVersion": version,
            "classification": "externalSupport",
            "classificationDigest": A,
            "mismatchKinds": [],
            "repositoryActor": {
                "username": "external-user",
                "computer": null,
                "infobase": null
            },
            "rootDeltaDigest": A,
            "contentDeltaDigest": CanonicalEmptyDeltaDigest::VALUE,
            "provenNotThisAction": true,
            "overlapWithAuthorizedTransitions": false,
            "supportOnlyDelta": true,
            "externalSupportDisjointnessDigest": B,
            "externalOwnershipEvidence": {
                "kind": "supportPrerequisiteReceipt",
                "receiptId": ID_2,
                "receiptDigest": A
            }
        }))
    }

    fn routine_history_observation(version: &str) -> SupportPrerequisiteVersionObservation {
        finalize_history_observation(json!({
            "repositoryVersion": version,
            "classification": "routine",
            "classificationDigest": A,
            "mismatchKinds": [],
            "repositoryActor": {
                "username": "routine-user",
                "computer": null,
                "infobase": null
            },
            "relevance": "unrelated",
            "rootDeltaDigest": A,
            "contentDeltaDigest": B,
            "supportTransitionsDigest": CanonicalEmptyDeltaDigest::VALUE,
            "supportGraphUnchanged": true
        }))
    }

    fn invalid_unattributed_history_observation(
        version: &str,
    ) -> SupportPrerequisiteVersionObservation {
        finalize_history_observation(json!({
            "repositoryVersion": version,
            "classification": "invalid",
            "classificationDigest": A,
            "mismatchKinds": ["versionUnattributed"],
            "provenance": "unattributed",
            "repositoryActor": null,
            "rootDeltaDigest": null,
            "contentDeltaDigest": null,
            "missingEvidenceKinds": ["repositoryActorUnavailable"]
        }))
    }

    fn invalid_this_action_history_observation(
        version: &str,
        frozen: &FrozenSupportRecoveryAuthorizationProjection,
        first_root_support_after_arming: bool,
    ) -> SupportPrerequisiteVersionObservation {
        let receipt = frozen.armed_binding().unwrap().arming_receipt();
        let mismatch_kinds = if first_root_support_after_arming {
            json!(["noAuthorizedVersionObserved"])
        } else {
            json!(["armingOrderViolated"])
        };
        finalize_history_observation(json!({
            "repositoryVersion": version,
            "classification": "invalid",
            "classificationDigest": A,
            "mismatchKinds": mismatch_kinds,
            "provenance": "thisAuthorizedAction",
            "repositoryActor": {
                "username": "reserved-user",
                "computer": null,
                "infobase": null
            },
            "manualTargetMode": "reservedOriginal",
            "armingReceiptId": receipt.arming_receipt_id(),
            "armingReceiptDigest": receipt.receipt_digest(),
            "firstRootSupportAfterArming": first_root_support_after_arming,
            "rootDeltaDigest": A,
            "contentDeltaDigest": B,
            "actionAttributionEvidenceDigest": A
        }))
    }

    fn corrective_history_observation(version: &str) -> SupportPrerequisiteVersionObservation {
        finalize_history_observation(json!({
            "repositoryVersion": version,
            "classification": "corrective",
            "classificationDigest": A,
            "mismatchKinds": [],
            "correctionKind": "actionCorrection",
            "repositoryActor": {
                "username": "reserved-user",
                "computer": null,
                "infobase": null
            },
            "manualTargetMode": "reservedOriginal",
            "rootDeltaDigest": A,
            "contentDeltaDigest": B,
            "correctiveInstructionDigest": C
        }))
    }

    #[derive(Clone)]
    struct FixedHistoryIndex(BTreeMap<String, EvidenceSourceIndexCandidate>);

    fn repository_fixture_error() -> RepositoryContractError {
        RepositoryHistorySourceEvidenceRef::new(EvidenceKind::RoutineClassification, "not-a-sha256")
            .unwrap_err()
    }

    impl EvidenceSourceIndex for FixedHistoryIndex {
        fn candidate_for(
            &self,
            repository_version: &RepositoryVersion,
            _registry: &EvidenceSourceRegistry,
        ) -> Result<EvidenceSourceIndexCandidate, RepositoryContractError> {
            self.0
                .get(repository_version.as_str())
                .cloned()
                .ok_or_else(repository_fixture_error)
        }
    }

    #[derive(Clone)]
    struct FixedHistoryOrder(RepositoryHistoryOrderEvidence);

    impl RepositoryHistoryOrderResolver for FixedHistoryOrder {
        fn order_evidence(
            &self,
            _from_exclusive: &RepositoryHistoryCursor,
            _through_inclusive: &RepositoryHistoryCursor,
        ) -> Result<RepositoryHistoryOrderEvidence, RepositoryContractError> {
            Ok(self.0.clone())
        }
    }

    struct FixedHistoryBytes(BTreeMap<(EvidenceKind, String), Vec<u8>>);

    impl RepositoryHistoryEvidenceBytesResolver for FixedHistoryBytes {
        fn load_canonical_evidence_bytes(
            &self,
            reference: &RepositoryHistorySourceEvidenceRef,
        ) -> Result<Vec<u8>, RepositoryContractError> {
            self.0
                .get(&(
                    reference.evidence_kind(),
                    reference.evidence_digest().as_str().to_owned(),
                ))
                .cloned()
                .ok_or_else(repository_fixture_error)
        }
    }

    fn validated_history_partition(
        evidence: &[SupportRecoveryHistoryEvidence],
    ) -> ValidatedRepositoryHistoryPartition {
        assert!(!evidence.is_empty());
        let registry = EvidenceSourceRegistry::task8().unwrap();
        let start_cursor = history_cursor("v1", A);
        let ordered_cursors = evidence
            .iter()
            .enumerate()
            .map(|(index, value)| {
                history_cursor(
                    value.repository_version().as_str(),
                    if index % 2 == 0 { B } else { C },
                )
            })
            .collect::<Vec<_>>();
        let through_cursor = ordered_cursors.last().unwrap().clone();
        let mut entries = Vec::with_capacity(evidence.len());
        let mut candidates = BTreeMap::new();
        let mut evidence_bytes = BTreeMap::new();
        for value in evidence {
            let (entry, selected_ref, rows) = match value {
                SupportRecoveryHistoryEvidence::SupportObservation(observation) => {
                    let projection = observation.task8_mapping_projection().unwrap();
                    let selected_ref = RepositoryHistorySourceEvidenceRef::new(
                        EvidenceKind::SupportPrerequisiteObservation,
                        observation.classification_digest().as_str(),
                    )
                    .unwrap();
                    evidence_bytes.insert(
                        (
                            EvidenceKind::SupportPrerequisiteObservation,
                            selected_ref.evidence_digest().as_str().to_owned(),
                        ),
                        serde_json_canonicalizer::to_vec(observation).unwrap(),
                    );
                    (
                        json!({
                            "repositoryVersion": observation.repository_version(),
                            "classification": projection.partition_classification(),
                            "semanticDeltaDigest": semantic_binding_digest(observation).unwrap(),
                            "sourceEvidenceRef": selected_ref,
                        }),
                        selected_ref.clone(),
                        vec![
                            EvidenceSourceIndexCandidateRow::available(
                                EvidenceKind::RoutineClassification,
                                vec![RepositoryHistorySourceEvidenceRef::new(
                                    EvidenceKind::RoutineClassification,
                                    A,
                                )
                                .unwrap()],
                            ),
                            EvidenceSourceIndexCandidateRow::available(
                                EvidenceKind::SupportPrerequisiteObservation,
                                vec![selected_ref.clone()],
                            ),
                            EvidenceSourceIndexCandidateRow::absent(
                                EvidenceKind::NonConflictingConcurrent,
                            ),
                        ],
                    )
                }
                SupportRecoveryHistoryEvidence::NonConflictingConcurrent(concurrent) => {
                    let selected_ref = RepositoryHistorySourceEvidenceRef::new(
                        EvidenceKind::NonConflictingConcurrent,
                        concurrent.evidence_digest().as_str(),
                    )
                    .unwrap();
                    let semantic_digest = canonical_contract_digest(
                        &SupportHistorySemanticBindingRecord {
                            repository_version: concurrent.repository_version().clone(),
                            partition_classification:
                                RepositoryHistoryPartitionClassification::NonConflictingConcurrent,
                            root_delta_digest: RequiredNullable::null(),
                            content_delta_digest: RequiredNullable::null(),
                            classification_digest: RequiredNullable::null(),
                            external_support_disjointness_digest: RequiredNullable::null(),
                            corrective_instruction_digest: RequiredNullable::null(),
                            non_conflicting_concurrent_evidence_digest: RequiredNullable::value(
                                concurrent.evidence_digest().clone(),
                            ),
                        },
                        None,
                    )
                    .unwrap();
                    evidence_bytes.insert(
                        (
                            EvidenceKind::NonConflictingConcurrent,
                            selected_ref.evidence_digest().as_str().to_owned(),
                        ),
                        serde_json_canonicalizer::to_vec(concurrent).unwrap(),
                    );
                    (
                        json!({
                            "repositoryVersion": concurrent.repository_version(),
                            "classification": "nonConflictingConcurrent",
                            "semanticDeltaDigest": semantic_digest,
                            "sourceEvidenceRef": selected_ref,
                            "nonConflictingConcurrentEvidence": concurrent,
                        }),
                        selected_ref.clone(),
                        vec![
                            EvidenceSourceIndexCandidateRow::available(
                                EvidenceKind::RoutineClassification,
                                vec![RepositoryHistorySourceEvidenceRef::new(
                                    EvidenceKind::RoutineClassification,
                                    A,
                                )
                                .unwrap()],
                            ),
                            EvidenceSourceIndexCandidateRow::absent(
                                EvidenceKind::SupportPrerequisiteObservation,
                            ),
                            EvidenceSourceIndexCandidateRow::available(
                                EvidenceKind::NonConflictingConcurrent,
                                vec![selected_ref.clone()],
                            ),
                        ],
                    )
                }
            };
            candidates.insert(
                value.repository_version().as_str().to_owned(),
                EvidenceSourceIndexCandidate::from_capability_adapter(
                    value.repository_version().as_str(),
                    registry.registry_digest().as_str(),
                    ID_2,
                    rows,
                )
                .unwrap(),
            );
            debug_assert_eq!(
                selected_ref.evidence_kind(),
                match value {
                    SupportRecoveryHistoryEvidence::SupportObservation(_) => {
                        EvidenceKind::SupportPrerequisiteObservation
                    }
                    SupportRecoveryHistoryEvidence::NonConflictingConcurrent(_) => {
                        EvidenceKind::NonConflictingConcurrent
                    }
                }
            );
            entries.push(entry);
        }
        let mut partition = json!({
            "fromExclusive": start_cursor,
            "throughInclusive": through_cursor,
            "entries": entries,
        });
        partition["partitionDigest"] = json!(raw_digest(&partition));
        let order = RepositoryHistoryOrderEvidence::from_capability_adapter(
            "history-order-v1",
            start_cursor,
            through_cursor,
            ordered_cursors,
        )
        .unwrap();
        RepositoryHistoryPartitionResolver::new(
            &registry,
            &FixedHistoryIndex(candidates),
            &FixedHistoryOrder(order),
            &FixedHistoryBytes(evidence_bytes),
        )
        .validate(serde_json::from_value(partition).unwrap())
        .unwrap()
    }

    fn armed_reserved_action() -> ActiveSupportActionResumeHandle {
        let support_action_id = id(ID_1);
        let endpoint = cursor();
        let layer_id = SupportLayerId::parse("layer-a").unwrap();
        let authorized_transitions =
            SupportTransitions::new(vec![SupportTransition::enable_configuration_changes(
                RepositoryTargetDisplay::parse("Configuration").unwrap(),
                layer_id.clone(),
            )])
            .unwrap();
        let manual_actor = RepositoryUsername::parse("reserved-user").unwrap();
        let handoff = SupportRecoveryDistributionHandoff::new(
            ManualSupportTargetMode::ReservedOriginal,
            None,
            SupportRecoveryDistributionHandoffInputs {
                handoff_id: id(ID_2),
                profile_artifact_ref_id: ProfileArtifactRefId::parse("vendor.layer-a").unwrap(),
                profile_artifact_display: DisplayPath::parse("Vendor layer A").unwrap(),
                user_visible_file_name: UserVisibleCfFileName::parse("vendor-layer-a.cf").unwrap(),
                manual_actor_username: manual_actor.clone(),
                layer_id: layer_id.clone(),
                distribution_artifact_id: id(ID_3),
                artifact_sha256: digest(A),
                readability_probe_receipt_id: id(ID_2),
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
        .unwrap();
        let recovery = SupportRecoveryDistributionEvidence::new(
            layer_id.clone(),
            id(ID_3),
            digest(A),
            digest(B),
            CapabilityRowId::parse("support-recovery.v1").unwrap(),
            handoff,
        )
        .unwrap();
        let recovery_set = SupportRecoveryDistributionSet::new(vec![recovery]).unwrap();
        let reachable =
            RootReachableSupportLayerSet::from_capability_adapter(vec![layer_id], digest(A))
                .unwrap();
        let coverage = SupportRecoveryDistributionCoverageAuthority::prove_complete(
            reachable,
            recovery_set,
            &authorized_transitions,
        )
        .unwrap();
        let inputs = SupportActionAuthorizationInputs::fixture(
            support_action_id,
            SupportActionPurpose::MainIntegrationPrerequisite,
            id(ID_2),
            digest(A),
            digest(B),
            endpoint.clone(),
            digest(B),
            digest(A),
            authorized_transitions,
            coverage,
            manual_actor.clone(),
            digest(B),
            digest(C),
            manual_actor.clone(),
            None,
            SupportActionPhaseBinding::main_integration(digest(C)),
        );
        let authorization = SupportActionAuthorizationAuthority::reserved_original(
            inputs,
            CapabilityRowId::parse("reserved-original-lease.v1").unwrap(),
            digest(B),
        )
        .unwrap();
        let receipt = SupportActionArmingReceipt::new(
            id(ID_3),
            authorization.support_action_id().clone(),
            authorization.support_action_digest().clone(),
            endpoint.clone(),
            endpoint.clone(),
            empty_partition(&endpoint),
            authorization.support_gate_digest().clone(),
            authorization.candidate_set_digest().clone(),
            authorization.expected_relevant_baseline_digest().clone(),
            digest(A),
            authorization
                .support_recovery_distribution_set_digest()
                .clone(),
            authorization.expected_original_fingerprint().clone(),
            ManualSupportTargetMode::ReservedOriginal,
            SupportRootLockObservation::new(RequiredNullable::value(owner("reserved-user")))
                .unwrap(),
            &owner("reserved-user"),
        )
        .unwrap();
        ActiveSupportActionResumeHandle::publish(authorization)
            .unwrap()
            .arm(receipt)
            .unwrap()
    }

    fn frozen_reserved_action() -> FrozenSupportRecoveryAuthorizationProjection {
        armed_reserved_action()
            .freeze_armed_action()
            .unwrap()
            .frozen_support_recovery_projection()
            .unwrap()
    }

    fn armed_separate_action() -> ActiveSupportActionResumeHandle {
        let support_action_id = id(ID_1);
        let endpoint = cursor();
        let layer_id = SupportLayerId::parse("layer-a").unwrap();
        let identity = ManualWorkingInfobaseIdentity::new(
            RepositoryIdentityComponent::parse("developer-mac").unwrap(),
            RepositoryIdentityComponent::parse("support-work").unwrap(),
        )
        .unwrap();
        let authorized_transitions =
            SupportTransitions::new(vec![SupportTransition::enable_configuration_changes(
                RepositoryTargetDisplay::parse("Configuration").unwrap(),
                layer_id.clone(),
            )])
            .unwrap();
        let manual_actor = RepositoryUsername::parse("support-user").unwrap();
        let handoff = SupportRecoveryDistributionHandoff::new(
            ManualSupportTargetMode::SeparateWorkingInfobase,
            Some(identity.clone()),
            SupportRecoveryDistributionHandoffInputs {
                handoff_id: id(ID_2),
                profile_artifact_ref_id: ProfileArtifactRefId::parse("vendor.layer-a").unwrap(),
                profile_artifact_display: DisplayPath::parse("Vendor layer A").unwrap(),
                user_visible_file_name: UserVisibleCfFileName::parse("vendor-layer-a.cf").unwrap(),
                manual_actor_username: manual_actor.clone(),
                layer_id: layer_id.clone(),
                distribution_artifact_id: id(ID_3),
                artifact_sha256: digest(A),
                readability_probe_receipt_id: id(ID_2),
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
        .unwrap();
        let recovery = SupportRecoveryDistributionEvidence::new(
            layer_id.clone(),
            id(ID_3),
            digest(A),
            digest(B),
            CapabilityRowId::parse("support-recovery.v1").unwrap(),
            handoff,
        )
        .unwrap();
        let recovery_set = SupportRecoveryDistributionSet::new(vec![recovery]).unwrap();
        let reachable =
            RootReachableSupportLayerSet::from_capability_adapter(vec![layer_id], digest(A))
                .unwrap();
        let coverage = SupportRecoveryDistributionCoverageAuthority::prove_complete(
            reachable,
            recovery_set,
            &authorized_transitions,
        )
        .unwrap();
        let lease_capability = CapabilityRowId::parse("manual-working-ib-lease.v1").unwrap();
        let baseline = ManualWorkingInfobaseBaseline::new(
            identity.clone(),
            endpoint.clone(),
            digest(B),
            digest(C),
            digest(C),
            digest(A),
            id(ID_2),
            lease_capability.clone(),
        )
        .unwrap();
        let inputs = SupportActionAuthorizationInputs::fixture(
            support_action_id,
            SupportActionPurpose::MainIntegrationPrerequisite,
            id(ID_2),
            digest(A),
            digest(B),
            endpoint.clone(),
            digest(B),
            digest(A),
            authorized_transitions,
            coverage,
            RepositoryUsername::parse("reserved-user").unwrap(),
            digest(B),
            digest(C),
            manual_actor.clone(),
            Some(lease_capability),
            SupportActionPhaseBinding::main_integration(digest(C)),
        );
        let authorization = SupportActionAuthorizationAuthority::separate_working_infobase(
            inputs, identity, baseline,
        )
        .unwrap();
        let receipt = SupportActionArmingReceipt::new(
            id(ID_3),
            authorization.support_action_id().clone(),
            authorization.support_action_digest().clone(),
            endpoint.clone(),
            endpoint.clone(),
            empty_partition(&endpoint),
            authorization.support_gate_digest().clone(),
            authorization.candidate_set_digest().clone(),
            authorization.expected_relevant_baseline_digest().clone(),
            digest(A),
            authorization
                .support_recovery_distribution_set_digest()
                .clone(),
            authorization.expected_original_fingerprint().clone(),
            ManualSupportTargetMode::SeparateWorkingInfobase,
            SupportRootLockObservation::new(RequiredNullable::value(owner("support-user")))
                .unwrap(),
            &owner("support-user"),
        )
        .unwrap();
        ActiveSupportActionResumeHandle::publish(authorization)
            .unwrap()
            .arm(receipt)
            .unwrap()
    }

    fn frozen_separate_action() -> FrozenSupportRecoveryAuthorizationProjection {
        armed_separate_action()
            .freeze_armed_action()
            .unwrap()
            .frozen_support_recovery_projection()
            .unwrap()
    }

    fn finalization_plan(endpoint: RepositoryHistoryCursor) -> SupportRecoveryFinalizationPlan {
        let display = RepositoryTargetDisplay::parse("Configuration").unwrap();
        let lock_targets =
            SupportRecoveryLockTargets::new(vec![SupportRecoveryLockTarget::configuration_root(
                display.clone(),
                vec![
                    RepositoryUpdateLockReason::SupportGraphGuard,
                    RepositoryUpdateLockReason::UpdateTarget,
                ],
            )
            .unwrap()])
            .unwrap();
        let desired_targets =
            SupportRecoveryDesiredTargets::new(vec![SupportRecoveryDesiredTarget::root_present(
                display,
                digest(C),
            )])
            .unwrap();
        SupportRecoveryFinalizationPlan::new(
            SupportRecoveryFinalizationPlanAuthority::desired_test_only(
                SupportRecoveryDisposition::RestoreThenReauthorize,
                lock_targets,
                desired_targets,
                endpoint,
                digest(A),
                digest(C),
            ),
        )
        .unwrap()
    }

    fn projection(
        support_action_id: UnicaId,
        planned_result_phase: TaskPhase,
        endpoint: RepositoryHistoryCursor,
    ) -> ArmedSupportRecoveryPlanProjection {
        let plan = finalization_plan(endpoint.clone());
        projection_with_finalization(support_action_id, planned_result_phase, endpoint, plan)
    }

    fn projection_with_finalization(
        support_action_id: UnicaId,
        planned_result_phase: TaskPhase,
        endpoint: RepositoryHistoryCursor,
        finalization_plan: SupportRecoveryFinalizationPlan,
    ) -> ArmedSupportRecoveryPlanProjection {
        projection_with_plans(
            support_action_id,
            planned_result_phase,
            endpoint,
            finalization_plan,
            None,
            ManualSupportTargetMode::ReservedOriginal,
        )
    }

    fn projection_with_plans(
        support_action_id: UnicaId,
        planned_result_phase: TaskPhase,
        endpoint: RepositoryHistoryCursor,
        finalization_plan: SupportRecoveryFinalizationPlan,
        closure_plan: Option<ManualWorkingInfobaseClosurePlan>,
        manual_target_mode: ManualSupportTargetMode,
    ) -> ArmedSupportRecoveryPlanProjection {
        projection_with_plans_and_action(
            support_action_id,
            planned_result_phase,
            endpoint,
            finalization_plan,
            closure_plan,
            manual_target_mode,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn projection_with_plans_and_action(
        support_action_id: UnicaId,
        planned_result_phase: TaskPhase,
        endpoint: RepositoryHistoryCursor,
        finalization_plan: SupportRecoveryFinalizationPlan,
        closure_plan: Option<ManualWorkingInfobaseClosurePlan>,
        manual_target_mode: ManualSupportTargetMode,
        required_external_action: Option<SupportRecoveryExternalAction>,
    ) -> ArmedSupportRecoveryPlanProjection {
        ArmedSupportRecoveryPlanProjection::test_only(
            OperationId::from_str("44444444-4444-4444-8444-444444444444").unwrap(),
            support_action_id,
            planned_result_phase,
            endpoint.clone(),
            endpoint.clone(),
            empty_partition(&endpoint),
            SupportRecoveryVersionObservations::new(Vec::new()).unwrap(),
            SupportRecoveryDisposition::RestoreThenReauthorize,
            TaskPhase::LocalVerified,
            finalization_plan,
            None,
            closure_plan,
            manual_target_mode,
            required_external_action,
        )
        .unwrap()
    }

    fn selective_plan(
        repository_version: &str,
        fingerprint: &str,
        lock_display: &str,
    ) -> SelectiveRepositoryUpdatePlan {
        let planned_targets = serde_json::from_value::<RepositoryTargetStates>(json!([{
            "targetKind": "configurationRoot",
            "state": "present",
            "repositoryVersion": repository_version,
            "targetFingerprint": fingerprint,
        }]))
        .unwrap();
        let lock_targets = serde_json::from_value::<RepositoryUpdateLockTargets>(json!([{
            "targetKind": "configurationRoot",
            "objectDisplay": lock_display,
            "reasons": ["supportGraphGuard", "updateTarget"],
        }]))
        .unwrap();
        SelectiveRepositoryUpdatePlan::recovery_finalization_test_only(
            planned_targets,
            lock_targets,
            CapabilityRowId::parse("selective-update.v1").unwrap(),
            None,
        )
        .unwrap()
    }

    struct FinalizationMaterializer(SelectiveRepositoryUpdatePlan);

    impl SupportRecoveryFinalizationMaterializationCapability for FinalizationMaterializer {
        fn materialize_selective_update_plan(
            &self,
            _recovery_digest: &Sha256Digest,
            _desired_finalization_plan: &SupportRecoveryFinalizationPlan,
            _approved_history: &ValidatedRepositoryHistoryPartition,
        ) -> Result<SupportRecoverySelectiveUpdatePlanObservation, ()> {
            Ok(
                SupportRecoverySelectiveUpdatePlanObservation::from_capability_adapter(
                    self.0.planned_targets().clone(),
                    self.0.lock_targets().clone(),
                    self.0.selective_objects_capability_id().clone(),
                    self.0.structural_capability_row_id().cloned(),
                    Vec::new(),
                ),
            )
        }
    }

    struct UnknownArmedEffect;

    impl SupportRecoveryEffectUnknownCapability for UnknownArmedEffect {
        fn observe_unknown_effect(
            &self,
            armed_action: &ArmedSupportInstructionProjection,
        ) -> Result<SupportRecoveryEffectUnknownObservation, ()> {
            Ok(
                SupportRecoveryEffectUnknownObservation::from_capability_adapter(
                    OperationId::from_str("44444444-4444-4444-8444-444444444444").unwrap(),
                    armed_action.support_action_id().clone(),
                    armed_action.support_action_digest().clone(),
                    armed_action.arming_receipt_id().clone(),
                    armed_action.arming_receipt_digest().clone(),
                ),
            )
        }
    }

    struct UnknownArmedEffectFor(OperationId);

    impl SupportRecoveryEffectUnknownCapability for UnknownArmedEffectFor {
        fn observe_unknown_effect(
            &self,
            armed_action: &ArmedSupportInstructionProjection,
        ) -> Result<SupportRecoveryEffectUnknownObservation, ()> {
            Ok(
                SupportRecoveryEffectUnknownObservation::from_capability_adapter(
                    self.0.clone(),
                    armed_action.support_action_id().clone(),
                    armed_action.support_action_digest().clone(),
                    armed_action.arming_receipt_id().clone(),
                    armed_action.arming_receipt_digest().clone(),
                ),
            )
        }
    }

    struct FixedRecoveryDestination(SupportRecoveryDestinationObservation);

    impl SupportRecoveryDestinationCapability for FixedRecoveryDestination {
        fn derive_destination(
            &self,
            _prior_operation_id: &OperationId,
            _frozen_authorization: &FrozenSupportRecoveryAuthorizationProjection,
            _support_history: &ValidatedRepositoryHistoryPartition,
        ) -> Result<SupportRecoveryDestinationObservation, ()> {
            Ok(self.0.clone())
        }
    }

    struct TrivialHistoryOrder;

    impl SupportHistoryOrderAuthority for TrivialHistoryOrder {
        fn compare_versions(
            &self,
            left: &RepositoryVersion,
            right: &RepositoryVersion,
        ) -> Result<Ordering, SupportContractError> {
            Ok(left.as_str().cmp(right.as_str()))
        }

        fn compare_cursors(
            &self,
            left: &RepositoryHistoryCursor,
            right: &RepositoryHistoryCursor,
        ) -> Result<Ordering, SupportContractError> {
            Ok(left
                .through_version()
                .as_str()
                .cmp(right.through_version().as_str()))
        }
    }

    fn conflict_instruction(
        finalization_plan: &SupportRecoveryFinalizationPlan,
    ) -> SupportConflictInstruction {
        let display = RepositoryTargetDisplay::parse("Configuration").unwrap();
        let layer = SupportLayerId::parse("layer-a").unwrap();
        let transition =
            SupportTransition::enable_configuration_changes(display.clone(), layer.clone());
        let conflict = SupportTransitionConflict::from_capability_adapter(
            RepositoryVersion::parse("v2").unwrap(),
            RequiredNullable::null(),
            None,
            display,
            layer,
            transition,
            digest(B),
            SupportTransitionOverlapKind::SameTarget,
            Diagnostic::parse("validated external support overlap").unwrap(),
        )
        .unwrap();
        SupportConflictInstruction::new(
            id(ID_4),
            SupportTransitionConflicts::new(vec![conflict], &TrivialHistoryOrder).unwrap(),
            finalization_plan.desired_support_graph_digest().clone(),
        )
        .unwrap()
    }

    struct FixedConflictSource(SupportConflictInstruction);

    impl SupportRecoveryConflictSourceCapability for FixedConflictSource {
        fn validated_conflict_instruction(
            &self,
            _prior_operation_id: &OperationId,
            _support_action_id: &UnicaId,
            _support_action_digest: &Sha256Digest,
            _history: &ValidatedRepositoryHistoryPartition,
            _finalization_plan: &SupportRecoveryFinalizationPlan,
        ) -> Result<SupportConflictInstruction, ()> {
            Ok(self.0.clone())
        }
    }

    struct FixedEvidenceSource(SupportRecoveryEvidenceSourceObservation);

    impl SupportRecoveryEvidenceSourceCapability for FixedEvidenceSource {
        fn validated_evidence_requirement(
            &self,
            _prior_operation_id: &OperationId,
            _support_action_id: &UnicaId,
            _support_action_digest: &Sha256Digest,
            _history: &ValidatedRepositoryHistoryPartition,
            _finalization_plan: &SupportRecoveryFinalizationPlan,
        ) -> Result<SupportRecoveryEvidenceSourceObservation, ()> {
            Ok(self.0.clone())
        }
    }

    fn evidence_source() -> FixedEvidenceSource {
        let instruction = SupportEvidenceInstruction::new(
            SupportBlockers::new(Vec::new()).unwrap(),
            SupportEvidenceGaps::new(Vec::new(), &TrivialHistoryOrder).unwrap(),
        )
        .unwrap();
        FixedEvidenceSource(
            SupportRecoveryEvidenceSourceObservation::from_capability_adapter(
                instruction,
                id(ID_4),
                digest(B),
            ),
        )
    }

    struct BlockedRootGuard;

    impl SupportRecoveryGuardAcquisitionCapability for BlockedRootGuard {
        fn acquire_guard(
            &self,
            _recovery_digest: &Sha256Digest,
            _finalization_plan: &SupportRecoveryFinalizationPlan,
            _materialized_working_infobase_closure_plan: Option<&ManualWorkingInfobaseClosurePlan>,
        ) -> Result<SupportRecoveryGuardAcquisitionObservation, ()> {
            Ok(
                SupportRecoveryGuardAcquisitionObservation::BlockedBeforeRoot {
                    guard_receipt_id: id(ID_3),
                    failed_target: serde_json::from_value(json!({
                        "targetKind": "configurationRoot",
                    }))
                    .unwrap(),
                    failed_target_display: RepositoryTargetDisplay::parse("Configuration").unwrap(),
                    locked_by: RequiredNullable::null(),
                },
            )
        }
    }

    struct AcquiredGuard;

    impl SupportRecoveryGuardAcquisitionCapability for AcquiredGuard {
        fn acquire_guard(
            &self,
            _recovery_digest: &Sha256Digest,
            finalization_plan: &SupportRecoveryFinalizationPlan,
            materialized_working_infobase_closure_plan: Option<&ManualWorkingInfobaseClosurePlan>,
        ) -> Result<SupportRecoveryGuardAcquisitionObservation, ()> {
            if materialized_working_infobase_closure_plan
                .is_some_and(|plan| plan.materialized().is_err())
            {
                return Err(());
            }
            Ok(SupportRecoveryGuardAcquisitionObservation::Acquired {
                guard_receipt_id: id(ID_3),
                acquired_root_first: SupportRecoveryAcquiredLockTargets::new(
                    finalization_plan.lock_targets().as_slice().to_vec(),
                )
                .unwrap(),
            })
        }
    }

    struct AcquiredGuardWithReceipt(UnicaId);

    impl SupportRecoveryGuardAcquisitionCapability for AcquiredGuardWithReceipt {
        fn acquire_guard(
            &self,
            _recovery_digest: &Sha256Digest,
            finalization_plan: &SupportRecoveryFinalizationPlan,
            materialized_working_infobase_closure_plan: Option<&ManualWorkingInfobaseClosurePlan>,
        ) -> Result<SupportRecoveryGuardAcquisitionObservation, ()> {
            if materialized_working_infobase_closure_plan
                .is_some_and(|plan| plan.materialized().is_err())
            {
                return Err(());
            }
            Ok(SupportRecoveryGuardAcquisitionObservation::Acquired {
                guard_receipt_id: self.0.clone(),
                acquired_root_first: SupportRecoveryAcquiredLockTargets::new(
                    finalization_plan.lock_targets().as_slice().to_vec(),
                )
                .unwrap(),
            })
        }
    }

    struct SuccessfulGuardRelease;

    impl SupportRecoveryGuardReleaseCapability for SuccessfulGuardRelease {
        fn release_complete_guard(
            &self,
            _recovery_digest: &Sha256Digest,
            finalization_plan: &SupportRecoveryFinalizationPlan,
            guard_receipt_id: &UnicaId,
        ) -> Result<SupportRecoveryGuardReleaseObservation, ()> {
            Ok(
                SupportRecoveryGuardReleaseObservation::from_capability_adapter(
                    guard_receipt_id.clone(),
                    id(ID_1),
                    SupportRecoveryReleasedLockTargets::new(
                        finalization_plan
                            .lock_targets()
                            .as_slice()
                            .iter()
                            .rev()
                            .cloned()
                            .collect(),
                    )
                    .unwrap(),
                ),
            )
        }
    }

    struct FixedFinalizationExecution {
        proof: SelectiveRepositoryUpdateProof,
        support_action_id: UnicaId,
        support_action_digest: Sha256Digest,
    }

    impl FixedFinalizationExecution {
        fn new(
            proof: SelectiveRepositoryUpdateProof,
            authority: &ApprovedSupportRecoveryAuthority,
        ) -> Self {
            Self {
                proof,
                support_action_id: authority.support_action_id().clone(),
                support_action_digest: authority.support_action_digest().clone(),
            }
        }
    }

    impl SupportRecoveryFinalizationExecutionCapability for FixedFinalizationExecution {
        fn execute_selective_update(
            &self,
            _recovery_digest: &Sha256Digest,
            _finalization_plan: &SupportRecoveryFinalizationPlan,
            _approved_history: &ValidatedRepositoryHistoryPartition,
            _guard_receipt_id: &UnicaId,
        ) -> Result<SupportRecoverySelectiveUpdateExecutionObservation, ()> {
            let proof = &self.proof;
            let outcome = if proof.update_performed() {
                SupportRecoverySelectiveUpdateEffectObservation::Performed {
                    update_effect_receipt_id: proof
                        .update_effect_receipt_id()
                        .expect("performed fixture has an effect receipt")
                        .clone(),
                    update_effect_receipt_digest: proof
                        .update_effect_receipt_digest()
                        .expect("performed fixture has an effect digest")
                        .clone(),
                }
            } else {
                SupportRecoverySelectiveUpdateEffectObservation::AlreadyExact
            };
            Ok(
                SupportRecoverySelectiveUpdateExecutionObservation::from_capability_adapter(
                    proof.guard_receipt_id().clone(),
                    proof.planned_targets().clone(),
                    proof.applied_targets().clone(),
                    proof.acquired_root_first().clone(),
                    proof.released_in_reverse_order().clone(),
                    proof
                        .before_original_target_fingerprint_map_digest()
                        .clone(),
                    outcome,
                    proof.verified_original_target_fingerprint_digest().clone(),
                    proof.observed_before_cursor().clone(),
                    proof.observed_after_cursor().clone(),
                ),
            )
        }

        fn terminalize_authorization(
            &self,
            _recovery_digest: &Sha256Digest,
            _support_action_id: &UnicaId,
            _support_action_digest: &Sha256Digest,
            _selective_update_proof: &SelectiveRepositoryUpdateProof,
        ) -> Result<SupportRecoveryAuthorizationTerminalizationObservation, ()> {
            Ok(
                SupportRecoveryAuthorizationTerminalizationObservation::from_capability_adapter(
                    self.support_action_id.clone(),
                    self.support_action_digest.clone(),
                    id(ID_2),
                    CompletedSupportRecoveryAuthorizationOutcome::Cancelled,
                ),
            )
        }
    }

    struct BusyWorkingInfobase;

    impl ManualWorkingInfobaseLeaseBusyCapability for BusyWorkingInfobase {
        fn observe_lease_owner(
            &self,
            _recovery_digest: &Sha256Digest,
            _closure_plan: &ManualWorkingInfobaseClosurePlan,
        ) -> Result<RequiredNullable<RepositoryOwnerIdentity>, ()> {
            Ok(RequiredNullable::null())
        }
    }

    struct SuccessfulRecheck;

    impl SupportRecoveryUnderGuardRecheckResolver for SuccessfulRecheck {
        fn verify_current_destination(
            &self,
            _recovery_digest: &Sha256Digest,
            _finalization_plan: &SupportRecoveryFinalizationPlan,
            _approved_history: &ValidatedRepositoryHistoryPartition,
            _guard_receipt_id: &UnicaId,
        ) -> Result<(), ()> {
            Ok(())
        }
    }

    struct SuccessfulClosure;

    impl ManualWorkingInfobaseTerminalLeaseCapability for SuccessfulClosure {
        fn acquire(
            &self,
            _recovery_digest: &Sha256Digest,
            _closure_plan: &ManualWorkingInfobaseClosurePlan,
        ) -> Result<ManualWorkingInfobaseAcquiredLease, ()> {
            Ok(ManualWorkingInfobaseAcquiredLease::from_capability_adapter(
                id(ID_1),
            ))
        }

        fn inspect(
            &self,
            _recovery_digest: &Sha256Digest,
            _closure_plan: &ManualWorkingInfobaseClosurePlan,
            acquired_lease: ManualWorkingInfobaseAcquiredLease,
        ) -> Result<ManualWorkingInfobaseLiveLeaseWindow, ()> {
            Ok(
                ManualWorkingInfobaseLiveLeaseWindow::from_capability_adapter(
                    acquired_lease.exclusive_lease_receipt_id().clone(),
                    cursor(),
                    digest(B),
                    digest(C),
                    digest(C),
                    digest(B),
                    digest(A),
                ),
            )
        }

        fn release_after_terminalization(
            &self,
            _recovery_digest: &Sha256Digest,
            _closure_plan: &ManualWorkingInfobaseClosurePlan,
            live_lease: ManualWorkingInfobaseLiveLeaseWindow,
            authorization_terminalization_receipt: &SupportRecoveryAuthorizationTerminalizationReceipt,
        ) -> Result<SupportRecoveryModeLeaseReleaseObservation, ()> {
            Ok(
                SupportRecoveryModeLeaseReleaseObservation::from_capability_adapter(
                    live_lease.exclusive_lease_receipt_id().clone(),
                    id(ID_2),
                    authorization_terminalization_receipt
                        .receipt_digest()
                        .clone(),
                ),
            )
        }
    }

    #[derive(Clone)]
    struct TracedClosure {
        trace: Rc<RefCell<Vec<&'static str>>>,
    }

    impl ManualWorkingInfobaseTerminalLeaseCapability for TracedClosure {
        fn acquire(
            &self,
            _recovery_digest: &Sha256Digest,
            _closure_plan: &ManualWorkingInfobaseClosurePlan,
        ) -> Result<ManualWorkingInfobaseAcquiredLease, ()> {
            self.trace.borrow_mut().push("acquire mode lease");
            Ok(ManualWorkingInfobaseAcquiredLease::from_capability_adapter(
                id(ID_1),
            ))
        }

        fn inspect(
            &self,
            _recovery_digest: &Sha256Digest,
            _closure_plan: &ManualWorkingInfobaseClosurePlan,
            acquired_lease: ManualWorkingInfobaseAcquiredLease,
        ) -> Result<ManualWorkingInfobaseLiveLeaseWindow, ()> {
            self.trace.borrow_mut().push("inspect");
            Ok(
                ManualWorkingInfobaseLiveLeaseWindow::from_capability_adapter(
                    acquired_lease.exclusive_lease_receipt_id().clone(),
                    cursor(),
                    digest(B),
                    digest(C),
                    digest(C),
                    digest(B),
                    digest(A),
                ),
            )
        }

        fn release_after_terminalization(
            &self,
            _recovery_digest: &Sha256Digest,
            _closure_plan: &ManualWorkingInfobaseClosurePlan,
            live_lease: ManualWorkingInfobaseLiveLeaseWindow,
            authorization_terminalization_receipt: &SupportRecoveryAuthorizationTerminalizationReceipt,
        ) -> Result<SupportRecoveryModeLeaseReleaseObservation, ()> {
            self.trace.borrow_mut().push("release mode lease");
            Ok(
                SupportRecoveryModeLeaseReleaseObservation::from_capability_adapter(
                    live_lease.exclusive_lease_receipt_id().clone(),
                    id(ID_2),
                    authorization_terminalization_receipt
                        .receipt_digest()
                        .clone(),
                ),
            )
        }
    }

    struct TracedFinalizationExecution {
        inner: FixedFinalizationExecution,
        trace: Rc<RefCell<Vec<&'static str>>>,
    }

    impl SupportRecoveryFinalizationExecutionCapability for TracedFinalizationExecution {
        fn execute_selective_update(
            &self,
            recovery_digest: &Sha256Digest,
            finalization_plan: &SupportRecoveryFinalizationPlan,
            approved_history: &ValidatedRepositoryHistoryPartition,
            guard_receipt_id: &UnicaId,
        ) -> Result<SupportRecoverySelectiveUpdateExecutionObservation, ()> {
            self.trace.borrow_mut().push("guarded selective update");
            self.inner.execute_selective_update(
                recovery_digest,
                finalization_plan,
                approved_history,
                guard_receipt_id,
            )
        }

        fn terminalize_authorization(
            &self,
            recovery_digest: &Sha256Digest,
            support_action_id: &UnicaId,
            support_action_digest: &Sha256Digest,
            selective_update_proof: &SelectiveRepositoryUpdateProof,
        ) -> Result<SupportRecoveryAuthorizationTerminalizationObservation, ()> {
            self.trace
                .borrow_mut()
                .push("durable authorization terminalization");
            self.inner.terminalize_authorization(
                recovery_digest,
                support_action_id,
                support_action_digest,
                selective_update_proof,
            )
        }
    }

    struct TracedGuardRelease {
        trace: Rc<RefCell<Vec<&'static str>>>,
    }

    impl SupportRecoveryGuardReleaseCapability for TracedGuardRelease {
        fn release_complete_guard(
            &self,
            recovery_digest: &Sha256Digest,
            finalization_plan: &SupportRecoveryFinalizationPlan,
            guard_receipt_id: &UnicaId,
        ) -> Result<SupportRecoveryGuardReleaseObservation, ()> {
            self.trace.borrow_mut().push("release repository guard");
            SuccessfulGuardRelease.release_complete_guard(
                recovery_digest,
                finalization_plan,
                guard_receipt_id,
            )
        }
    }

    struct FixedReservedBusy(ReservedOriginalLeaseBusyObservation);

    impl ReservedOriginalLeaseBusyCapability for FixedReservedBusy {
        fn observe_lease_busy(
            &self,
            _recovery_digest: &Sha256Digest,
            _finalization_plan: &SupportRecoveryFinalizationPlan,
        ) -> Result<ReservedOriginalLeaseBusyObservation, ()> {
            Ok(self.0.clone())
        }
    }

    struct FixedReservedLease {
        manual_actor_username: RepositoryUsername,
        baseline_lock_set_digest: Sha256Digest,
        reserved_original_identity_digest: Sha256Digest,
        exclusive_lease_capability_id: CapabilityRowId,
        expected_repository_fingerprint: Sha256Digest,
        observed_original_fingerprint: Sha256Digest,
    }

    impl ReservedOriginalTerminalLeaseCapability for FixedReservedLease {
        fn acquire(
            &self,
            _recovery_digest: &Sha256Digest,
            _finalization_plan: &SupportRecoveryFinalizationPlan,
        ) -> Result<ReservedOriginalAcquiredLease, ()> {
            Ok(ReservedOriginalAcquiredLease::from_capability_adapter(id(
                ID_1,
            )))
        }

        fn inspect(
            &self,
            _recovery_digest: &Sha256Digest,
            _finalization_plan: &SupportRecoveryFinalizationPlan,
            acquired_lease: ReservedOriginalAcquiredLease,
        ) -> Result<ReservedOriginalLiveLeaseWindow, ()> {
            Ok(ReservedOriginalLiveLeaseWindow::from_capability_adapter(
                self.manual_actor_username.clone(),
                self.baseline_lock_set_digest.clone(),
                self.baseline_lock_set_digest.clone(),
                self.reserved_original_identity_digest.clone(),
                self.exclusive_lease_capability_id.clone(),
                acquired_lease.exclusive_lease_receipt_id().clone(),
                self.expected_repository_fingerprint.clone(),
                self.observed_original_fingerprint.clone(),
            ))
        }

        fn release_after_terminalization(
            &self,
            _recovery_digest: &Sha256Digest,
            _finalization_plan: &SupportRecoveryFinalizationPlan,
            live_lease: ReservedOriginalLiveLeaseWindow,
            authorization_terminalization_receipt: &SupportRecoveryAuthorizationTerminalizationReceipt,
        ) -> Result<SupportRecoveryModeLeaseReleaseObservation, ()> {
            Ok(
                SupportRecoveryModeLeaseReleaseObservation::from_capability_adapter(
                    live_lease.exclusive_lease_receipt_id().clone(),
                    id(ID_2),
                    authorization_terminalization_receipt
                        .receipt_digest()
                        .clone(),
                ),
            )
        }
    }

    struct EmptyPostReleaseHistory;

    impl SupportRecoveryPostReleaseHistoryScanner for EmptyPostReleaseHistory {
        fn scan_after_release(
            &self,
            from_cursor: &RepositoryHistoryCursor,
        ) -> Result<
            (
                ValidatedRepositoryHistoryPartition,
                Option<DeferredRepositoryAdvance>,
            ),
            (),
        > {
            Ok((empty_partition(from_cursor), None))
        }
    }

    fn desired_closure_plan(
        frozen: &FrozenSupportRecoveryAuthorizationProjection,
    ) -> ManualWorkingInfobaseClosurePlan {
        let baseline = frozen
            .armed_binding()
            .unwrap()
            .manual_working_infobase_baseline()
            .unwrap();
        ManualWorkingInfobaseClosurePlan::new(
            ManualWorkingInfobaseClosurePlanAuthority::desired_test_only(
                baseline.working_infobase_identity().clone(),
                baseline.baseline_digest().clone(),
                digest(C),
                digest(B),
                digest(A),
                baseline.exclusive_lease_capability_id().clone(),
            ),
        )
        .unwrap()
    }

    fn recovery_destination(
        bootstrap: &SupportRecoveryBootstrapAuthority,
        desired_repository_content_digest: Sha256Digest,
    ) -> FixedRecoveryDestination {
        let display = RepositoryTargetDisplay::parse("Configuration").unwrap();
        let working_infobase = (bootstrap.frozen_authorization.manual_target_mode()
            == ManualSupportTargetMode::SeparateWorkingInfobase)
            .then(|| {
                SupportRecoveryWorkingInfobaseDestinationObservation::from_capability_adapter(
                    digest(C),
                    digest(B),
                )
            });
        FixedRecoveryDestination(
            SupportRecoveryDestinationObservation::from_capability_adapter(
                SupportRecoveryDisposition::RestoreThenReauthorize,
                SupportRecoveryLockTargets::new(vec![
                    SupportRecoveryLockTarget::configuration_root(
                        display.clone(),
                        vec![
                            RepositoryUpdateLockReason::SupportGraphGuard,
                            RepositoryUpdateLockReason::UpdateTarget,
                        ],
                    )
                    .unwrap(),
                ])
                .unwrap(),
                SupportRecoveryDesiredTargets::new(vec![
                    SupportRecoveryDesiredTarget::root_present(display, digest(C)),
                ])
                .unwrap(),
                digest(A),
                desired_repository_content_digest,
                working_infobase,
            ),
        )
    }

    fn prepare_bootstrap(
        bootstrap: SupportRecoveryBootstrapAuthority,
        desired_repository_content_digest: Sha256Digest,
    ) -> PreparedSupportRecoveryBootstrap {
        let history = empty_partition(&cursor());
        let destination = recovery_destination(&bootstrap, desired_repository_content_digest);
        bootstrap.desired_plans(&history, &destination).unwrap()
    }

    fn production_reserved_materialized_authority_for(
        prior_operation_id: OperationId,
    ) -> ApprovedSupportRecoveryAuthority {
        let bootstrap = SupportRecoveryBootstrapAuthority::from_effect_unknown(
            armed_reserved_action(),
            &UnknownArmedEffectFor(prior_operation_id),
        )
        .unwrap();
        let prepared = prepare_bootstrap(bootstrap, digest(C));
        let desired_catalog = prepared
            .action_catalog_without_external(id(ID_2), id(ID_3))
            .unwrap();
        let desired_projection = prepared
            .recovery_plan_projection(
                desired_catalog,
                TaskPhase::Synchronized,
                empty_partition(&cursor()),
                SupportRecoveryVersionObservations::new(Vec::new()).unwrap(),
                TaskPhase::LocalVerified,
            )
            .unwrap();
        let desired = desired_projection.approve().unwrap();
        let materialized = desired
            .materialized_finalization_plan(&FinalizationMaterializer(selective_plan(
                "v1",
                C,
                "Configuration",
            )))
            .unwrap();
        let catalog = desired
            .replan_action_catalog_without_external(id(ID_2), id(ID_3))
            .unwrap();
        let projection = desired
            .replan_projection(
                catalog,
                TaskPhase::Synchronized,
                cursor(),
                cursor(),
                empty_partition(&cursor()),
                SupportRecoveryVersionObservations::new(Vec::new()).unwrap(),
                SupportRecoveryDisposition::RestoreThenReauthorize,
                TaskPhase::LocalVerified,
                materialized,
                None,
                None,
            )
            .unwrap();
        desired.approve_replan(projection).unwrap()
    }

    fn production_reserved_materialized_authority() -> ApprovedSupportRecoveryAuthority {
        production_reserved_materialized_authority_for(OperationId::from_str(ID_4).unwrap())
    }

    fn production_separate_materialized_authority() -> ApprovedSupportRecoveryAuthority {
        let bootstrap = SupportRecoveryBootstrapAuthority::from_effect_unknown(
            // The separate action helper returns a frozen projection, so build
            // the equivalent armed handle through the dedicated fixture below.
            armed_separate_action(),
            &UnknownArmedEffect,
        )
        .unwrap();
        let prepared = prepare_bootstrap(bootstrap, digest(C));
        let desired_catalog = prepared
            .action_catalog_without_external(id(ID_2), id(ID_3))
            .unwrap();
        let desired_projection = prepared
            .recovery_plan_projection(
                desired_catalog,
                TaskPhase::Synchronized,
                empty_partition(&cursor()),
                SupportRecoveryVersionObservations::new(Vec::new()).unwrap(),
                TaskPhase::LocalVerified,
            )
            .unwrap();
        let desired = desired_projection.approve().unwrap();
        let materialized_closure = desired
            .materialized_working_infobase_closure_plan(&ClosureMaterializer {
                base_cursor: cursor(),
                object_version_map_digest: digest(B),
            })
            .unwrap();
        let materialized_finalization = desired
            .materialized_finalization_plan(&FinalizationMaterializer(selective_plan(
                "v1",
                C,
                "Configuration",
            )))
            .unwrap();
        let catalog = desired
            .replan_action_catalog_without_external(id(ID_2), id(ID_3))
            .unwrap();
        let projection = desired
            .replan_projection(
                catalog,
                TaskPhase::Synchronized,
                cursor(),
                cursor(),
                empty_partition(&cursor()),
                SupportRecoveryVersionObservations::new(Vec::new()).unwrap(),
                SupportRecoveryDisposition::RestoreThenReauthorize,
                TaskPhase::LocalVerified,
                materialized_finalization,
                None,
                Some(materialized_closure),
            )
            .unwrap();
        desired.approve_replan(projection).unwrap()
    }

    fn approved_reserved_with_history(
        evidence: Vec<SupportRecoveryHistoryEvidence>,
    ) -> ApprovedSupportRecoveryAuthority {
        let frozen = frozen_reserved_action();
        let partition = validated_history_partition(&evidence);
        let projection = ArmedSupportRecoveryPlanProjection::test_only(
            OperationId::from_str(ID_4).unwrap(),
            frozen.support_action_id().clone(),
            TaskPhase::Synchronized,
            partition.start_cursor().clone(),
            partition.through_inclusive().clone(),
            partition,
            SupportRecoveryVersionObservations::new(evidence).unwrap(),
            SupportRecoveryDisposition::RestoreThenReauthorize,
            TaskPhase::LocalVerified,
            finalization_plan(cursor()),
            None,
            None,
            ManualSupportTargetMode::ReservedOriginal,
            None,
        )
        .unwrap();
        ApprovedSupportRecoveryAuthority::new(frozen, projection).unwrap()
    }

    fn approve_external_replan(
        prepared: PreparedSupportRecoveryExternalReplan,
    ) -> Result<ApprovedSupportRecoveryAuthority, SupportRecoveryAuthorityError> {
        prepared.approve_replan(id(ID_2), id(ID_3))
    }

    fn reconstructed_projection(
        authority: &ApprovedSupportRecoveryAuthority,
        latest_guard_proof: Option<SupportRecoveryGuardProof>,
        required_external_action: Option<SupportRecoveryExternalAction>,
    ) -> ArmedSupportRecoveryPlanProjection {
        ArmedSupportRecoveryPlanProjection::test_only(
            authority.prior_operation_id().clone(),
            authority.support_action_id().clone(),
            authority.projection.planned_result_phase(),
            authority.projection.support_history_from_cursor().clone(),
            authority
                .projection
                .support_history_through_cursor()
                .clone(),
            authority.projection.support_history_partition().clone(),
            authority.projection.support_version_observations().clone(),
            authority.projection.support_recovery_disposition(),
            authority.projection.support_late_relevant_result_phase(),
            authority.finalization_plan().clone(),
            latest_guard_proof,
            authority.working_infobase_closure_plan().cloned(),
            authority.authorization.manual_target_mode(),
            required_external_action,
        )
        .unwrap()
    }

    fn replan_projection_with_history(
        authority: &ApprovedSupportRecoveryAuthority,
        evidence: Vec<SupportRecoveryHistoryEvidence>,
    ) -> Result<PreparedSupportRecoveryReplanProjection, SupportRecoveryAuthorityError> {
        let partition = validated_history_partition(&evidence);
        let catalog = authority.replan_action_catalog_without_external(id(ID_2), id(ID_3))?;
        authority.replan_projection(
            catalog,
            authority.projection.planned_result_phase(),
            partition.start_cursor().clone(),
            partition.through_inclusive().clone(),
            partition,
            SupportRecoveryVersionObservations::new(evidence).unwrap(),
            authority.projection.support_recovery_disposition(),
            authority.projection.support_late_relevant_result_phase(),
            authority.retained_replan_finalization_plan(),
            None,
            authority.retained_replan_working_infobase_closure_plan(),
        )
    }

    fn reconstructed_projection_with_history(
        authority: &ApprovedSupportRecoveryAuthority,
        evidence: Vec<SupportRecoveryHistoryEvidence>,
    ) -> ArmedSupportRecoveryPlanProjection {
        let partition = validated_history_partition(&evidence);
        ArmedSupportRecoveryPlanProjection::test_only(
            authority.prior_operation_id().clone(),
            authority.support_action_id().clone(),
            authority.projection.planned_result_phase(),
            partition.start_cursor().clone(),
            partition.through_inclusive().clone(),
            partition,
            SupportRecoveryVersionObservations::new(evidence).unwrap(),
            authority.projection.support_recovery_disposition(),
            authority.projection.support_late_relevant_result_phase(),
            authority.finalization_plan().clone(),
            None,
            authority.working_infobase_closure_plan().cloned(),
            authority.authorization.manual_target_mode(),
            authority.projection.required_external_action().cloned(),
        )
        .unwrap()
    }

    fn exact_reserved_busy(authority: &ApprovedSupportRecoveryAuthority) -> FixedReservedBusy {
        let baseline_lock_set_digest = authority
            .authorization
            .manual_actor_lock_baseline_digest()
            .unwrap()
            .clone();
        FixedReservedBusy(
            ReservedOriginalLeaseBusyObservation::from_capability_adapter(
                authority.authorization.manual_actor_username().clone(),
                baseline_lock_set_digest.clone(),
                baseline_lock_set_digest,
                authority
                    .authorization
                    .reserved_original_identity_digest()
                    .clone(),
                authority
                    .authorization
                    .reserved_original_lease_capability_id()
                    .unwrap()
                    .clone(),
                RequiredNullable::null(),
            ),
        )
    }

    fn acquired_guard_window(
        authority: ApprovedSupportRecoveryAuthority,
        receipt_id: UnicaId,
    ) -> ApprovedSupportRecoveryGuardWindow {
        let ApprovedSupportRecoveryGuardAttempt::Acquired(window) = authority
            .acquire_guard(&AcquiredGuardWithReceipt(receipt_id))
            .unwrap()
        else {
            panic!("fixture must acquire the complete guard")
        };
        *window
    }

    struct ClosureMaterializer {
        base_cursor: RepositoryHistoryCursor,
        object_version_map_digest: Sha256Digest,
    }

    impl ManualWorkingInfobaseClosureMaterializationCapability for ClosureMaterializer {
        fn observe_recorded_base(
            &self,
            _recovery_digest: &Sha256Digest,
            _desired_closure_plan: &ManualWorkingInfobaseClosurePlan,
            _approved_history: &ValidatedRepositoryHistoryPartition,
        ) -> Result<(RepositoryHistoryCursor, Sha256Digest), ()> {
            Ok((
                self.base_cursor.clone(),
                self.object_version_map_digest.clone(),
            ))
        }
    }

    fn corrective_transition_with_capability(capability_row_id: &str) -> SupportRecoveryTransition {
        SupportRecoveryTransition::restore_vendor_configuration_support_test_only(
            RepositoryTargetDisplay::parse("Configuration").unwrap(),
            SupportLayerId::parse("layer-a").unwrap(),
            VendorRestoredSupportState::Locked,
            id(ID_3),
            id(ID_2),
            CapabilityRowId::parse(capability_row_id).unwrap(),
        )
    }

    fn corrective_transition() -> SupportRecoveryTransition {
        corrective_transition_with_capability("support-recovery.v1")
    }

    fn ordinary_non_inverse_transition() -> SupportRecoveryTransition {
        SupportRecoveryTransition::ordinary(SupportTransition::enable_configuration_changes(
            RepositoryTargetDisplay::parse("Configuration").unwrap(),
            SupportLayerId::parse("layer-a").unwrap(),
        ))
    }

    fn handoff_revalidation(
        handoff: &SupportRecoveryDistributionHandoff,
        receipt_id: &str,
    ) -> SupportRecoveryHandoffRevalidation {
        SupportRecoveryHandoffRevalidation::new(
            handoff,
            digest(A),
            id(receipt_id),
            CapabilityRowId::parse("manual-readability.v1").unwrap(),
            CapabilityRowId::parse("retention-provider.v1").unwrap(),
        )
        .unwrap()
    }

    #[derive(Clone)]
    struct FixedCorrectiveLocks(SupportRecoveryLockTargets);

    impl SupportCorrectiveLockClosureResolver for FixedCorrectiveLocks {
        fn resolve_correction_lock_targets(
            &self,
            _required_root_transitions: &[SupportRecoveryTransition],
            _required_content_restorations: &[SupportContentRestoration],
        ) -> Result<SupportRecoveryLockTargets, SupportCorrectiveLockClosureResolutionError>
        {
            Ok(self.0.clone())
        }
    }

    fn substituted_handoff() -> SupportRecoveryDistributionHandoff {
        SupportRecoveryDistributionHandoff::new(
            ManualSupportTargetMode::ReservedOriginal,
            None,
            SupportRecoveryDistributionHandoffInputs {
                handoff_id: id(ID_1),
                profile_artifact_ref_id: ProfileArtifactRefId::parse("vendor.layer-a").unwrap(),
                profile_artifact_display: DisplayPath::parse("Vendor layer A").unwrap(),
                user_visible_file_name: UserVisibleCfFileName::parse("vendor-layer-a.cf").unwrap(),
                manual_actor_username: RepositoryUsername::parse("reserved-user").unwrap(),
                layer_id: SupportLayerId::parse("layer-a").unwrap(),
                distribution_artifact_id: id(ID_3),
                artifact_sha256: digest(A),
                readability_probe_receipt_id: id(ID_2),
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

    #[test]
    fn recovery_history_binds_every_support_and_ncc_entry_one_to_one() {
        let frozen = frozen_reserved_action();
        let authorization = authorized_history_observation("v2", &frozen);
        let concurrent = NonConflictingConcurrentEvidence::new("v3", ID_2, A, B, A, B, A).unwrap();
        let exact = vec![
            SupportRecoveryHistoryEvidence::from(authorization.clone()),
            SupportRecoveryHistoryEvidence::from(concurrent.clone()),
        ];
        let partition = validated_history_partition(&exact);
        let binding = frozen.armed_binding().unwrap();

        assert!(observations_bind_partition(&partition, &exact, &frozen, binding).unwrap());
        assert!(!observations_bind_partition(&partition, &exact[..1], &frozen, binding).unwrap());
        let swapped = vec![
            SupportRecoveryHistoryEvidence::from(concurrent.clone()),
            SupportRecoveryHistoryEvidence::from(authorization),
        ];
        assert!(!observations_bind_partition(&partition, &swapped, &frozen, binding).unwrap());
        let substituted = NonConflictingConcurrentEvidence::new("v3", ID_2, B, B, A, B, A).unwrap();
        let substituted = vec![
            exact[0].clone(),
            SupportRecoveryHistoryEvidence::from(substituted),
        ];
        assert!(!observations_bind_partition(&partition, &substituted, &frozen, binding).unwrap());
    }

    #[test]
    fn neutral_history_may_precede_the_first_exact_claim_but_actionable_rows_may_not() {
        let frozen = frozen_reserved_action();
        let binding = frozen.armed_binding().unwrap();
        let concurrent = NonConflictingConcurrentEvidence::new("v2", ID_2, A, B, A, B, A).unwrap();
        let authorized = authorized_history_observation("v3", &frozen);
        let ncc_then_authorized = vec![
            SupportRecoveryHistoryEvidence::from(concurrent.clone()),
            SupportRecoveryHistoryEvidence::from(authorized.clone()),
        ];
        let partition = validated_history_partition(&ncc_then_authorized);
        assert!(
            observations_bind_partition(&partition, &ncc_then_authorized, &frozen, binding,)
                .unwrap()
        );

        let routine_then_authorized = vec![
            SupportRecoveryHistoryEvidence::from(routine_history_observation("v2")),
            SupportRecoveryHistoryEvidence::from(authorized),
        ];
        let partition = validated_history_partition(&routine_then_authorized);
        assert!(observations_bind_partition(
            &partition,
            &routine_then_authorized,
            &frozen,
            binding,
        )
        .unwrap());

        let ncc_only = vec![SupportRecoveryHistoryEvidence::from(concurrent)];
        let partition = validated_history_partition(&ncc_only);
        assert!(partition_requires_late_phase(&partition));
        assert!(!observations_bind_partition(&partition, &ncc_only, &frozen, binding).unwrap());

        for first in [
            SupportRecoveryHistoryEvidence::from(external_history_observation("v2")),
            SupportRecoveryHistoryEvidence::from(invalid_unattributed_history_observation("v2")),
        ] {
            let actionable_then_authorized = vec![
                first,
                SupportRecoveryHistoryEvidence::from(authorized_history_observation("v3", &frozen)),
            ];
            let partition = validated_history_partition(&actionable_then_authorized);
            assert!(!observations_bind_partition(
                &partition,
                &actionable_then_authorized,
                &frozen,
                binding,
            )
            .unwrap());
        }
    }

    #[test]
    fn first_claim_flag_and_corrective_source_are_bound_only_after_the_arming_prefix() {
        let frozen = frozen_reserved_action();
        let binding = frozen.armed_binding().unwrap();
        let invalid_false = invalid_this_action_history_observation("v2", &frozen, false);
        let corrective = corrective_history_observation("v3");
        let external = external_history_observation("v1");
        let mut saw_first = false;

        assert!(!observation_claim_binds_frozen_action(
            &invalid_false,
            RepositoryHistoryPartitionClassification::Invalid,
            &frozen,
            binding,
            None,
            0,
            0,
            &mut saw_first,
        ));
        assert!(!saw_first);
        assert!(!observation_claim_binds_frozen_action(
            &corrective,
            RepositoryHistoryPartitionClassification::Corrective,
            &frozen,
            binding,
            Some(frozen.support_action_id()),
            0,
            0,
            &mut saw_first,
        ));
        assert!(observation_claim_binds_frozen_action(
            &external,
            RepositoryHistoryPartitionClassification::ExternalSupport,
            &frozen,
            binding,
            None,
            0,
            1,
            &mut saw_first,
        ));
        assert!(
            !saw_first,
            "an accepted prefix row must not claim the new action"
        );

        let invalid_true = invalid_this_action_history_observation("v2", &frozen, true);
        assert!(observation_claim_binds_frozen_action(
            &invalid_true,
            RepositoryHistoryPartitionClassification::Invalid,
            &frozen,
            binding,
            None,
            0,
            0,
            &mut saw_first,
        ));
        assert!(saw_first);
        assert!(observation_claim_binds_frozen_action(
            &corrective,
            RepositoryHistoryPartitionClassification::Corrective,
            &frozen,
            binding,
            Some(frozen.support_action_id()),
            1,
            0,
            &mut saw_first,
        ));
        assert!(!observation_claim_binds_frozen_action(
            &corrective,
            RepositoryHistoryPartitionClassification::Corrective,
            &frozen,
            binding,
            Some(&id(ID_2)),
            1,
            0,
            &mut saw_first,
        ));
    }

    #[test]
    fn production_bootstrap_reaches_approval_without_a_cyclic_recovery_token() {
        let armed = armed_reserved_action();
        let support_action_id = armed.support_action_id().clone();
        let bootstrap =
            SupportRecoveryBootstrapAuthority::from_effect_unknown(armed, &UnknownArmedEffect)
                .unwrap();
        let endpoint = cursor();
        let history = empty_partition(&endpoint);
        let destination = recovery_destination(&bootstrap, digest(C));
        let prepared = bootstrap.desired_plans(&history, &destination).unwrap();
        let action_catalog = prepared
            .action_catalog_without_external(id(ID_2), id(ID_3))
            .unwrap();
        let projection = prepared
            .recovery_plan_projection(
                action_catalog,
                TaskPhase::Synchronized,
                history,
                SupportRecoveryVersionObservations::new(Vec::new()).unwrap(),
                TaskPhase::LocalVerified,
            )
            .unwrap();
        let approved = projection.approve().unwrap();
        let expected_observation_digest = SupportRecoveryVersionObservations::new(Vec::new())
            .unwrap()
            .digest()
            .unwrap();
        let status = approved.recovery_plan_status();

        assert_eq!(approved.support_action_id(), &support_action_id);
        assert_eq!(
            approved.support_version_observation_digest(),
            &expected_observation_digest,
        );
        assert_eq!(status.prior_operation_id(), approved.prior_operation_id());
        assert_eq!(status.recovery_digest(), approved.recovery_digest());
        assert!(approved
            .finalization_plan()
            .materialized_selective_update_plan()
            .is_none());
    }

    #[test]
    fn bootstrap_destination_is_capability_derived_and_mode_exact() {
        let wrong_mode_bootstrap = SupportRecoveryBootstrapAuthority::from_effect_unknown(
            armed_reserved_action(),
            &UnknownArmedEffect,
        )
        .unwrap();
        let history = empty_partition(&cursor());

        let mut wrong_mode_destination = recovery_destination(&wrong_mode_bootstrap, digest(C)).0;
        wrong_mode_destination.working_infobase = Some(
            SupportRecoveryWorkingInfobaseDestinationObservation::from_capability_adapter(
                digest(C),
                digest(B),
            ),
        );
        assert!(wrong_mode_bootstrap
            .desired_plans(&history, &FixedRecoveryDestination(wrong_mode_destination))
            .is_err());

        let spliced_bootstrap = SupportRecoveryBootstrapAuthority::from_effect_unknown(
            armed_reserved_action(),
            &UnknownArmedEffect,
        )
        .unwrap();
        let spliced_history = empty_partition(&history_cursor("v2", B));
        let destination = recovery_destination(&spliced_bootstrap, digest(C));
        assert!(spliced_bootstrap
            .desired_plans(&spliced_history, &destination)
            .is_err());
    }

    #[test]
    fn bootstrap_lineage_rejects_same_identity_cross_bootstrap_destination() {
        let armed = armed_reserved_action();
        let bootstrap_a = SupportRecoveryBootstrapAuthority::from_effect_unknown(
            armed.clone(),
            &UnknownArmedEffect,
        )
        .unwrap();
        let bootstrap_b =
            SupportRecoveryBootstrapAuthority::from_effect_unknown(armed, &UnknownArmedEffect)
                .unwrap();
        let history = empty_partition(&cursor());
        let destination_a = recovery_destination(&bootstrap_a, digest(C));
        let destination_b = recovery_destination(&bootstrap_b, digest(C));
        let prepared_a = bootstrap_a.desired_plans(&history, &destination_a).unwrap();
        let prepared_b = bootstrap_b.desired_plans(&history, &destination_b).unwrap();
        let catalog_b = prepared_b
            .action_catalog_without_external(id(ID_2), id(ID_3))
            .unwrap();

        assert!(prepared_a
            .recovery_plan_projection(
                catalog_b,
                TaskPhase::Synchronized,
                history,
                SupportRecoveryVersionObservations::new(Vec::new()).unwrap(),
                TaskPhase::LocalVerified,
            )
            .is_err());
    }

    #[test]
    fn bootstrap_destination_claim_is_a_consuming_typestate() {
        let bootstrap = SupportRecoveryBootstrapAuthority::from_effect_unknown(
            armed_reserved_action(),
            &UnknownArmedEffect,
        )
        .unwrap();
        let history = empty_partition(&cursor());
        let destination = recovery_destination(&bootstrap, digest(C));
        let prepared = bootstrap.desired_plans(&history, &destination).unwrap();
        assert_eq!(
            prepared
                .finalization_plan()
                .desired_repository_content_digest(),
            &digest(C),
        );
        assert!(std::mem::size_of::<SupportRecoveryAuthorityToken>() > 0);
    }

    #[test]
    fn replan_rejects_same_identity_destination_substitution() {
        fn desired_authority(
            handle: ActiveSupportActionResumeHandle,
            desired_content: Sha256Digest,
        ) -> ApprovedSupportRecoveryAuthority {
            let bootstrap =
                SupportRecoveryBootstrapAuthority::from_effect_unknown(handle, &UnknownArmedEffect)
                    .unwrap();
            let history = empty_partition(&cursor());
            let destination = recovery_destination(&bootstrap, desired_content);
            let prepared = bootstrap.desired_plans(&history, &destination).unwrap();
            let catalog = prepared
                .action_catalog_without_external(id(ID_2), id(ID_3))
                .unwrap();
            let projection = prepared
                .recovery_plan_projection(
                    catalog,
                    TaskPhase::Synchronized,
                    history,
                    SupportRecoveryVersionObservations::new(Vec::new()).unwrap(),
                    TaskPhase::LocalVerified,
                )
                .unwrap();
            projection.approve().unwrap()
        }

        let armed = armed_reserved_action();
        let authority_a = desired_authority(armed.clone(), digest(C));
        let authority_b = desired_authority(armed, digest(B));
        assert_ne!(
            authority_a
                .finalization_plan()
                .desired_repository_content_digest(),
            authority_b
                .finalization_plan()
                .desired_repository_content_digest(),
        );

        let forged_for_b = BoundSupportRecoveryReplanFinalizationPlan {
            authority_lineage: authority_b.token.lineage(),
            previous_recovery_digest: authority_b.recovery_digest().clone(),
            plan: authority_a.finalization_plan().clone(),
        };
        let catalog_b = authority_b
            .replan_action_catalog_without_external(id(ID_2), id(ID_3))
            .unwrap();
        assert!(authority_b
            .replan_projection(
                catalog_b,
                TaskPhase::Synchronized,
                cursor(),
                cursor(),
                empty_partition(&cursor()),
                SupportRecoveryVersionObservations::new(Vec::new()).unwrap(),
                authority_b.projection.support_recovery_disposition(),
                TaskPhase::LocalVerified,
                forged_for_b,
                None,
                None,
            )
            .is_err());

        let catalog_b = authority_b
            .replan_action_catalog_without_external(id(ID_2), id(ID_3))
            .unwrap();
        let finalization_a = authority_a.retained_replan_finalization_plan();
        let closure_a = authority_a.retained_replan_working_infobase_closure_plan();
        assert!(authority_b
            .replan_projection(
                catalog_b,
                TaskPhase::Synchronized,
                cursor(),
                cursor(),
                empty_partition(&cursor()),
                SupportRecoveryVersionObservations::new(Vec::new()).unwrap(),
                authority_a.projection.support_recovery_disposition(),
                TaskPhase::LocalVerified,
                finalization_a,
                None,
                closure_a,
            )
            .is_err());

        let catalog_a = authority_a
            .replan_action_catalog_without_external(id(ID_2), id(ID_3))
            .unwrap();
        let projection_a = authority_a
            .replan_projection(
                catalog_a,
                TaskPhase::Synchronized,
                cursor(),
                cursor(),
                empty_partition(&cursor()),
                SupportRecoveryVersionObservations::new(Vec::new()).unwrap(),
                authority_a.projection.support_recovery_disposition(),
                TaskPhase::LocalVerified,
                authority_a.retained_replan_finalization_plan(),
                None,
                authority_a.retained_replan_working_infobase_closure_plan(),
            )
            .unwrap();
        assert!(authority_b.approve_replan(projection_a).is_err());

        let authority_c = desired_authority(armed_reserved_action(), digest(B));
        let substituted_projection = ArmedSupportRecoveryPlanProjection::test_only(
            authority_c.prior_operation_id().clone(),
            authority_c.support_action_id().clone(),
            authority_c.projection.planned_result_phase(),
            authority_c.projection.support_history_from_cursor().clone(),
            authority_c
                .projection
                .support_history_through_cursor()
                .clone(),
            authority_c.projection.support_history_partition().clone(),
            authority_c
                .projection
                .support_version_observations()
                .clone(),
            authority_c.projection.support_recovery_disposition(),
            authority_c.projection.support_late_relevant_result_phase(),
            authority_a.finalization_plan().clone(),
            None,
            None,
            authority_c.authorization.manual_target_mode(),
            None,
        )
        .unwrap();
        assert!(authority_c
            .approve_reconstructed_replan_for_test(substituted_projection)
            .is_err());
    }

    #[test]
    fn production_catalog_and_projection_reject_cross_bootstrap_splicing() {
        let armed = armed_reserved_action();
        let bootstrap_a = SupportRecoveryBootstrapAuthority::from_effect_unknown(
            armed.clone(),
            &UnknownArmedEffect,
        )
        .unwrap();
        let bootstrap_b =
            SupportRecoveryBootstrapAuthority::from_effect_unknown(armed, &UnknownArmedEffect)
                .unwrap();
        let history = empty_partition(&cursor());
        let destination_a = recovery_destination(&bootstrap_a, digest(C));
        let destination_b = recovery_destination(&bootstrap_b, digest(C));
        let prepared_a = bootstrap_a.desired_plans(&history, &destination_a).unwrap();
        let prepared_b = bootstrap_b.desired_plans(&history, &destination_b).unwrap();
        let wait_a = prepared_a
            .evidence_external_wait(id(ID_4), &history, &evidence_source())
            .unwrap();
        assert!(prepared_b
            .action_catalog_with_external(id(ID_2), wait_a, id(ID_3))
            .is_err());
    }

    #[test]
    fn external_catalog_rejects_a_different_candidate_finalization() {
        let armed = armed_reserved_action();
        let bootstrap_a = SupportRecoveryBootstrapAuthority::from_effect_unknown(
            armed.clone(),
            &UnknownArmedEffect,
        )
        .unwrap();
        let bootstrap_b =
            SupportRecoveryBootstrapAuthority::from_effect_unknown(armed, &UnknownArmedEffect)
                .unwrap();
        let history = empty_partition(&cursor());
        let destination_a = recovery_destination(&bootstrap_a, digest(C));
        let destination_b = recovery_destination(&bootstrap_b, digest(B));
        let prepared_a = bootstrap_a.desired_plans(&history, &destination_a).unwrap();
        let prepared_b = bootstrap_b.desired_plans(&history, &destination_b).unwrap();
        assert_ne!(
            prepared_a.finalization_plan().plan_digest(),
            prepared_b.finalization_plan().plan_digest(),
        );
        let wait = prepared_a
            .evidence_external_wait(id(ID_4), &history, &evidence_source())
            .unwrap();
        assert!(prepared_b
            .action_catalog_with_external(id(ID_2), wait, id(ID_3))
            .is_err());
    }

    #[test]
    fn production_bootstrap_approves_corrective_conflict_and_evidence_waits() {
        let corrective_bootstrap = SupportRecoveryBootstrapAuthority::from_effect_unknown(
            armed_reserved_action(),
            &UnknownArmedEffect,
        )
        .unwrap();
        let history = empty_partition(&cursor());
        let handoff = corrective_bootstrap
            .frozen_authorization
            .armed_binding()
            .unwrap()
            .support_recovery_distributions()[0]
            .handoff()
            .clone();
        let corrective_prepared = prepare_bootstrap(corrective_bootstrap, digest(C));
        let corrective_lock_targets = corrective_prepared
            .finalization_plan()
            .lock_targets()
            .clone();
        let corrective_wait = corrective_prepared
            .corrective_external_wait(
                id(ID_4),
                &history,
                vec![corrective_transition()],
                Vec::new(),
                vec![handoff.clone()],
                vec![handoff_revalidation(&handoff, ID_4)],
                &FixedCorrectiveLocks(corrective_lock_targets),
            )
            .unwrap();
        let corrective_catalog = corrective_prepared
            .action_catalog_with_external(id(ID_2), corrective_wait, id(ID_3))
            .unwrap();
        let corrective_projection = corrective_prepared
            .recovery_plan_projection(
                corrective_catalog,
                TaskPhase::Synchronized,
                history.clone(),
                SupportRecoveryVersionObservations::new(Vec::new()).unwrap(),
                TaskPhase::LocalVerified,
            )
            .unwrap();
        let corrective = corrective_projection.approve().unwrap();
        assert!(matches!(
            corrective
                .projection
                .required_external_action()
                .unwrap()
                .as_ref(),
            SupportRecoveryExternalActionRef::Corrective(_),
        ));

        let conflict_bootstrap = SupportRecoveryBootstrapAuthority::from_effect_unknown(
            armed_reserved_action(),
            &UnknownArmedEffect,
        )
        .unwrap();
        let conflict_prepared = prepare_bootstrap(conflict_bootstrap, digest(C));
        let conflict_instruction = conflict_instruction(conflict_prepared.finalization_plan());
        let conflict_wait = conflict_prepared
            .conflict_external_wait(
                id(ID_4),
                &history,
                &FixedConflictSource(conflict_instruction),
            )
            .unwrap();
        let conflict_catalog = conflict_prepared
            .action_catalog_with_external(id(ID_2), conflict_wait, id(ID_3))
            .unwrap();
        let conflict_projection = conflict_prepared
            .recovery_plan_projection(
                conflict_catalog,
                TaskPhase::Synchronized,
                history.clone(),
                SupportRecoveryVersionObservations::new(Vec::new()).unwrap(),
                TaskPhase::LocalVerified,
            )
            .unwrap();
        let conflict = conflict_projection.approve().unwrap();
        assert!(matches!(
            conflict
                .projection
                .required_external_action()
                .unwrap()
                .as_ref(),
            SupportRecoveryExternalActionRef::Conflict(_),
        ));

        let evidence_bootstrap = SupportRecoveryBootstrapAuthority::from_effect_unknown(
            armed_reserved_action(),
            &UnknownArmedEffect,
        )
        .unwrap();
        let evidence_prepared = prepare_bootstrap(evidence_bootstrap, digest(C));
        let evidence_wait = evidence_prepared
            .evidence_external_wait(id(ID_4), &history, &evidence_source())
            .unwrap();
        let evidence_catalog = evidence_prepared
            .action_catalog_with_external(id(ID_2), evidence_wait, id(ID_3))
            .unwrap();
        let evidence_projection = evidence_prepared
            .recovery_plan_projection(
                evidence_catalog,
                TaskPhase::Synchronized,
                history,
                SupportRecoveryVersionObservations::new(Vec::new()).unwrap(),
                TaskPhase::LocalVerified,
            )
            .unwrap();
        let evidence = evidence_projection.approve().unwrap();
        assert!(matches!(
            evidence
                .projection
                .required_external_action()
                .unwrap()
                .as_ref(),
            SupportRecoveryExternalActionRef::Evidence(_),
        ));

        let guard_authority = production_reserved_materialized_authority();
        let ApprovedSupportRecoveryGuardAttempt::Blocked(blocked) =
            guard_authority.acquire_guard(&BlockedRootGuard).unwrap()
        else {
            panic!("fixture must return a blocked proof")
        };
        let blocked_guard_proof = blocked.guard_proof.clone();
        for action in [
            corrective.projection.required_external_action().unwrap(),
            conflict.projection.required_external_action().unwrap(),
            evidence.projection.required_external_action().unwrap(),
        ] {
            assert!(!external_action_guard_proof_presence_is_exact(
                Some(action),
                Some(&blocked_guard_proof),
            ));
        }
        assert!(!external_action_guard_proof_presence_is_exact(
            None,
            Some(&blocked_guard_proof),
        ));
        let reconstructed_authority = production_reserved_materialized_authority();
        let reconstructed =
            reconstructed_projection(&reconstructed_authority, Some(blocked_guard_proof), None);
        assert!(reconstructed_authority
            .approve_reconstructed_replan_for_test(reconstructed)
            .is_err());

        assert!(corrective
            .replan_action_catalog_without_external(id(ID_2), id(ID_3))
            .is_err());
        let erased_without_resolution = reconstructed_projection(&evidence, None, None);
        assert!(evidence
            .approve_reconstructed_replan_for_test(erased_without_resolution)
            .is_err());
    }

    #[test]
    fn production_replan_approves_release_clean_and_close_waits() {
        let release_authority = production_reserved_materialized_authority();
        let ApprovedSupportRecoveryGuardAttempt::Blocked(blocked) =
            release_authority.acquire_guard(&BlockedRootGuard).unwrap()
        else {
            panic!("fixture must return a blocked guard proof")
        };
        let release_evidence = blocked
            .lock_release_external_replan(id(ID_4), digest(B))
            .unwrap();
        let release = approve_external_replan(release_evidence).unwrap();
        assert!(!external_action_guard_proof_presence_is_exact(
            release.projection.required_external_action(),
            None,
        ));
        assert!(matches!(
            release
                .projection
                .required_external_action()
                .unwrap()
                .as_ref(),
            SupportRecoveryExternalActionRef::ReleaseLocks(_),
        ));

        let clean_authority = production_separate_materialized_authority();
        let clean_evidence = acquired_guard_window(clean_authority, id(ID_3))
            .prepare_working_infobase_lease_busy_stop(&BusyWorkingInfobase)
            .unwrap()
            .stopped_with_cleanup_wait(
                id(ID_4),
                digest(B),
                &SuccessfulGuardRelease,
                &SuccessfulRecheck,
            )
            .unwrap();
        let clean = approve_external_replan(clean_evidence).unwrap();
        assert!(!external_action_guard_proof_presence_is_exact(
            clean.projection.required_external_action(),
            None,
        ));
        assert!(matches!(
            clean
                .projection
                .required_external_action()
                .unwrap()
                .as_ref(),
            SupportRecoveryExternalActionRef::CleanWorkingInfobase(_),
        ));

        let close_authority = production_reserved_materialized_authority();
        let busy = exact_reserved_busy(&close_authority);
        let close_evidence = acquired_guard_window(close_authority, id(ID_3))
            .prepare_reserved_original_lease_busy_stop(&busy)
            .unwrap()
            .stopped_with_closure_wait(
                id(ID_4),
                digest(B),
                &SuccessfulGuardRelease,
                &SuccessfulRecheck,
            )
            .unwrap();
        let close = approve_external_replan(close_evidence).unwrap();
        assert!(!external_action_guard_proof_presence_is_exact(
            close.projection.required_external_action(),
            None,
        ));
        assert!(matches!(
            close
                .projection
                .required_external_action()
                .unwrap()
                .as_ref(),
            SupportRecoveryExternalActionRef::CloseReservedOriginal(_),
        ));
    }

    #[test]
    fn stop_wait_keeps_guard_proof_and_authority_opaque_across_attempts() {
        let authority_a = production_reserved_materialized_authority();
        let busy_a = exact_reserved_busy(&authority_a);
        let attempt_a = acquired_guard_window(authority_a, id(ID_3))
            .prepare_reserved_original_lease_busy_stop(&busy_a)
            .unwrap()
            .stopped_with_closure_wait(
                id(ID_4),
                digest(B),
                &SuccessfulGuardRelease,
                &SuccessfulRecheck,
            )
            .unwrap();
        let proof_a_digest = attempt_a.latest_guard_proof.proof_digest().clone();

        let authority_b = production_reserved_materialized_authority();
        let busy_b = exact_reserved_busy(&authority_b);
        let proof_b = acquired_guard_window(authority_b, id(ID_4))
            .prepare_reserved_original_lease_busy_stop(&busy_b)
            .unwrap()
            .stopped_after_complete_guard_proof(&SuccessfulGuardRelease, &SuccessfulRecheck)
            .unwrap();
        assert_ne!(&proof_a_digest, proof_b.proof_digest());
        assert!(approve_external_replan(attempt_a).is_ok());
    }

    #[test]
    fn lock_release_rejects_another_authoritys_same_plan_proof_before_wait_mint() {
        let authority_a =
            production_reserved_materialized_authority_for(OperationId::from_str(ID_3).unwrap());
        let authority_b =
            production_reserved_materialized_authority_for(OperationId::from_str(ID_4).unwrap());
        assert_eq!(
            authority_a.finalization_plan().plan_digest(),
            authority_b.finalization_plan().plan_digest(),
        );
        assert_eq!(
            authority_a.authorization.manual_target_mode(),
            authority_b.authorization.manual_target_mode(),
        );
        let ApprovedSupportRecoveryGuardAttempt::Blocked(attempt_a) =
            authority_a.acquire_guard(&BlockedRootGuard).unwrap()
        else {
            panic!("fixture must return a blocked proof")
        };
        let CurrentBlockedSupportRecoveryGuardAttempt {
            authority: authority_a,
            prior_operation_id,
            support_action_id,
            support_action_digest,
            recovery_digest,
            guard_proof,
        } = *attempt_a;
        drop(authority_a);
        let forged_for_b = CurrentBlockedSupportRecoveryGuardAttempt {
            authority: authority_b,
            prior_operation_id,
            support_action_id,
            support_action_digest,
            recovery_digest,
            guard_proof,
        };
        assert!(forged_for_b
            .lock_release_external_replan(id(ID_4), digest(B))
            .is_err());
    }

    #[test]
    fn replan_history_is_monotonic_for_entries_and_typed_observations() {
        let frozen = frozen_reserved_action();
        let authorized =
            SupportRecoveryHistoryEvidence::from(authorized_history_observation("v2", &frozen));
        let routine_v3 = SupportRecoveryHistoryEvidence::from(routine_history_observation("v3"));
        let current = vec![authorized.clone(), routine_v3.clone()];

        let no_change = approved_reserved_with_history(current.clone());
        let projection = replan_projection_with_history(&no_change, current.clone()).unwrap();
        assert!(no_change.approve_replan(projection).is_ok());

        let append = approved_reserved_with_history(current.clone());
        let appended = vec![
            authorized.clone(),
            routine_v3.clone(),
            SupportRecoveryHistoryEvidence::from(routine_history_observation("v4")),
        ];
        let projection = replan_projection_with_history(&append, appended).unwrap();
        assert!(append.approve_replan(projection).is_ok());

        let rewind = approved_reserved_with_history(current.clone());
        assert!(replan_projection_with_history(&rewind, vec![authorized.clone()]).is_err());

        let replacement = approved_reserved_with_history(current.clone());
        let same_through_replacement = vec![
            authorized.clone(),
            SupportRecoveryHistoryEvidence::from(external_history_observation("v3")),
        ];
        assert!(
            replan_projection_with_history(&replacement, same_through_replacement.clone()).is_err()
        );

        let direct_rewind = approved_reserved_with_history(current.clone());
        let direct_rewind_projection =
            reconstructed_projection_with_history(&direct_rewind, vec![authorized]);
        assert!(direct_rewind
            .approve_reconstructed_replan_for_test(direct_rewind_projection)
            .is_err());

        let direct_replacement = approved_reserved_with_history(current);
        let direct_replacement_projection =
            reconstructed_projection_with_history(&direct_replacement, same_through_replacement);
        assert!(direct_replacement
            .approve_reconstructed_replan_for_test(direct_replacement_projection)
            .is_err());
    }

    #[test]
    fn reconstructed_plan_rejects_a_guard_proof_from_another_manual_mode() {
        let separate = production_separate_materialized_authority();
        let ApprovedSupportRecoveryGuardAttempt::Blocked(blocked) =
            separate.acquire_guard(&BlockedRootGuard).unwrap()
        else {
            panic!("fixture must return a blocked proof")
        };
        let release = approve_external_replan(
            blocked
                .lock_release_external_replan(id(ID_4), digest(B))
                .unwrap(),
        )
        .unwrap();
        let wrong_mode_proof = release
            .projection
            .latest_support_recovery_guard_proof()
            .unwrap()
            .clone();
        let release_action = release
            .projection
            .required_external_action()
            .unwrap()
            .clone();

        let reserved = production_reserved_materialized_authority();
        assert_eq!(
            wrong_mode_proof.finalization_plan_digest(),
            reserved.finalization_plan().plan_digest(),
        );
        assert_ne!(
            wrong_mode_proof.manual_target_mode(),
            reserved.authorization.manual_target_mode(),
        );
        let reconstructed =
            reconstructed_projection(&reserved, Some(wrong_mode_proof), Some(release_action));
        assert!(reserved
            .approve_reconstructed_replan_for_test(reconstructed)
            .is_err());
    }

    #[test]
    fn approved_authority_binds_the_real_frozen_action_and_exact_recovery_plan() {
        let frozen = frozen_reserved_action();
        let support_action_id = frozen.support_action_id().clone();
        let support_action_digest = frozen.support_action_digest().clone();
        let sealed_plan = projection(support_action_id.clone(), TaskPhase::Synchronized, cursor());
        let recovery_digest = sealed_plan.recovery_digest().clone();

        let approved = ApprovedSupportRecoveryAuthority::new(frozen, sealed_plan).unwrap();

        assert_eq!(approved.support_action_id(), &support_action_id);
        assert_eq!(approved.support_action_digest(), &support_action_digest);
        assert_eq!(approved.recovery_digest(), &recovery_digest);
        assert!(approved.working_infobase_closure_plan().is_none());
    }

    #[test]
    fn approved_authority_rejects_spliced_action_phase_and_task9_fixtures() {
        let frozen = frozen_reserved_action();
        let endpoint = cursor();

        assert!(ApprovedSupportRecoveryAuthority::new(
            frozen.clone(),
            projection(id(ID_2), TaskPhase::Synchronized, endpoint.clone()),
        )
        .is_err());
        assert!(ApprovedSupportRecoveryAuthority::new(
            frozen.clone(),
            projection(
                frozen.support_action_id().clone(),
                TaskPhase::LocalVerified,
                endpoint,
            ),
        )
        .is_err());

        let task9_fixture = FrozenSupportRecoveryAuthorizationProjection::reserved_test_only(
            frozen.support_action_id().clone(),
            frozen.support_action_digest().clone(),
            frozen.manual_actor_username().clone(),
            frozen.reserved_original_identity_digest().clone(),
            frozen.expected_original_fingerprint().clone(),
            frozen
                .reserved_original_lease_capability_id()
                .unwrap()
                .clone(),
            frozen.manual_actor_lock_baseline_digest().unwrap().clone(),
        );
        assert!(ApprovedSupportRecoveryAuthority::new(
            task9_fixture,
            projection(
                frozen.support_action_id().clone(),
                TaskPhase::Synchronized,
                cursor(),
            ),
        )
        .is_err());
    }

    #[test]
    fn finalization_materialization_is_plan_bound_and_is_the_only_guard_mint() {
        let frozen = frozen_reserved_action();
        let desired_projection = projection(
            frozen.support_action_id().clone(),
            TaskPhase::Synchronized,
            cursor(),
        );
        let desired_authority =
            ApprovedSupportRecoveryAuthority::new(frozen.clone(), desired_projection).unwrap();

        let unmaterialized_guard_authority = ApprovedSupportRecoveryAuthority::new(
            frozen.clone(),
            projection(
                frozen.support_action_id().clone(),
                TaskPhase::Synchronized,
                cursor(),
            ),
        )
        .unwrap();
        assert!(unmaterialized_guard_authority
            .acquire_guard(&BlockedRootGuard)
            .is_err());
        assert!(desired_authority
            .materialized_finalization_plan(&FinalizationMaterializer(selective_plan(
                "v1",
                B,
                "Configuration",
            )))
            .is_err());
        assert!(desired_authority
            .materialized_finalization_plan(&FinalizationMaterializer(selective_plan(
                "v1",
                C,
                "Wrong display",
            )))
            .is_err());
        assert!(desired_authority
            .materialized_finalization_plan(&FinalizationMaterializer(selective_plan(
                "v2",
                C,
                "Configuration",
            )))
            .is_err());

        let materialized = desired_authority
            .materialized_finalization_plan(&FinalizationMaterializer(selective_plan(
                "v1",
                C,
                "Configuration",
            )))
            .unwrap();
        assert!(materialized
            .plan()
            .materialized_selective_update_plan()
            .is_some());
        assert_ne!(
            materialized.plan().plan_digest(),
            desired_authority.finalization_plan().plan_digest(),
        );

        let materialized_projection = projection_with_finalization(
            frozen.support_action_id().clone(),
            TaskPhase::Synchronized,
            cursor(),
            materialized.plan().clone(),
        );
        let approved =
            ApprovedSupportRecoveryAuthority::new(frozen, materialized_projection).unwrap();
        let ApprovedSupportRecoveryGuardAttempt::Blocked(proof) =
            approved.acquire_guard(&BlockedRootGuard).unwrap()
        else {
            panic!("capability returned a blocked guard attempt")
        };
        assert_eq!(
            proof.guard_proof.finalization_plan_digest(),
            materialized.plan().plan_digest()
        );
        assert!(!proof.guard_proof.is_completed());
    }

    #[test]
    fn separate_guard_requires_the_exact_materialized_working_ib_closure() {
        let frozen = frozen_separate_action();
        let desired_finalization = finalization_plan(cursor());
        let desired_closure = desired_closure_plan(&frozen);
        let desired_projection = projection_with_plans(
            frozen.support_action_id().clone(),
            TaskPhase::Synchronized,
            cursor(),
            desired_finalization,
            Some(desired_closure.clone()),
            ManualSupportTargetMode::SeparateWorkingInfobase,
        );
        let desired_authority =
            ApprovedSupportRecoveryAuthority::new(frozen.clone(), desired_projection).unwrap();

        let wrong_cursor = serde_json::from_value(json!({
            "throughVersion": "v2",
            "historyPrefixDigest": B,
        }))
        .unwrap();
        assert!(desired_authority
            .materialized_working_infobase_closure_plan(&ClosureMaterializer {
                base_cursor: wrong_cursor,
                object_version_map_digest: digest(B),
            })
            .is_err());
        assert!(desired_authority
            .materialized_working_infobase_closure_plan(&ClosureMaterializer {
                base_cursor: cursor(),
                object_version_map_digest: digest(A),
            })
            .is_err());

        let materialized_closure = desired_authority
            .materialized_working_infobase_closure_plan(&ClosureMaterializer {
                base_cursor: cursor(),
                object_version_map_digest: digest(B),
            })
            .unwrap();
        assert!(materialized_closure.plan().materialized().is_ok());
        assert_ne!(
            materialized_closure.plan().plan_digest(),
            desired_closure.plan_digest(),
        );
        let materialized_finalization = desired_authority
            .materialized_finalization_plan(&FinalizationMaterializer(selective_plan(
                "v1",
                C,
                "Configuration",
            )))
            .unwrap();

        let still_desired_closure_projection = projection_with_plans(
            frozen.support_action_id().clone(),
            TaskPhase::Synchronized,
            cursor(),
            materialized_finalization.plan().clone(),
            Some(desired_closure),
            ManualSupportTargetMode::SeparateWorkingInfobase,
        );
        let still_desired =
            ApprovedSupportRecoveryAuthority::new(frozen.clone(), still_desired_closure_projection)
                .unwrap();
        assert!(still_desired.acquire_guard(&BlockedRootGuard).is_err());

        let materialized_projection = projection_with_plans(
            frozen.support_action_id().clone(),
            TaskPhase::Synchronized,
            cursor(),
            materialized_finalization.plan().clone(),
            Some(materialized_closure.plan().clone()),
            ManualSupportTargetMode::SeparateWorkingInfobase,
        );
        let approved =
            ApprovedSupportRecoveryAuthority::new(frozen, materialized_projection).unwrap();
        let approved_plan_digest = approved.finalization_plan().plan_digest().clone();
        let ApprovedSupportRecoveryGuardAttempt::Acquired(window) =
            approved.acquire_guard(&AcquiredGuard).unwrap()
        else {
            panic!("capability returned a complete guard window")
        };
        assert_eq!(window.guard_receipt_id(), &id(ID_3));
        let prepared_stop = window
            .prepare_working_infobase_lease_busy_stop(&BusyWorkingInfobase)
            .unwrap();
        let guard_stop = prepared_stop
            .stopped_after_complete_guard_proof(&SuccessfulGuardRelease, &SuccessfulRecheck)
            .unwrap();
        assert!(!guard_stop.is_completed());
        assert_eq!(
            serde_json::to_value(&guard_stop).unwrap()["manualWorkingInfobaseStopEvidence"]
                ["cause"],
            json!("leaseBusy"),
        );
        assert_eq!(
            serde_json::to_value(&guard_stop).unwrap()["guardReleaseReceiptId"],
            json!(ID_1),
        );
        assert_eq!(guard_stop.finalization_plan_digest(), &approved_plan_digest,);
    }

    #[test]
    fn completed_separate_recovery_requires_exact_closure_guard_and_plan_proofs() {
        let frozen = frozen_separate_action();
        let desired_projection = projection_with_plans(
            frozen.support_action_id().clone(),
            TaskPhase::Synchronized,
            cursor(),
            finalization_plan(cursor()),
            Some(desired_closure_plan(&frozen)),
            ManualSupportTargetMode::SeparateWorkingInfobase,
        );
        let desired_authority =
            ApprovedSupportRecoveryAuthority::new(frozen.clone(), desired_projection).unwrap();
        let materialized_closure = desired_authority
            .materialized_working_infobase_closure_plan(&ClosureMaterializer {
                base_cursor: cursor(),
                object_version_map_digest: digest(B),
            })
            .unwrap();
        let materialized_finalization = desired_authority
            .materialized_finalization_plan(&FinalizationMaterializer(selective_plan(
                "v1",
                C,
                "Configuration",
            )))
            .unwrap();
        let exact_selective_plan = materialized_finalization
            .plan()
            .materialized_selective_update_plan()
            .unwrap()
            .clone();
        let approved_projection = projection_with_plans(
            frozen.support_action_id().clone(),
            TaskPhase::Synchronized,
            cursor(),
            materialized_finalization.plan().clone(),
            Some(materialized_closure.plan().clone()),
            ManualSupportTargetMode::SeparateWorkingInfobase,
        );
        let approved_plan_digest = approved_projection
            .support_recovery_finalization_plan()
            .plan_digest()
            .clone();
        let execution_authority =
            ApprovedSupportRecoveryAuthority::new(frozen.clone(), approved_projection.clone())
                .unwrap();
        let acquire_window = || {
            let ApprovedSupportRecoveryGuardAttempt::Acquired(window) =
                ApprovedSupportRecoveryAuthority::new(frozen.clone(), approved_projection.clone())
                    .unwrap()
                    .acquire_guard(&AcquiredGuard)
                    .unwrap()
            else {
                panic!("capability returned a complete guard window")
            };
            window
        };
        let first_window = acquire_window()
            .prepare_working_infobase_completion(&SuccessfulClosure)
            .unwrap();

        let substituted_guard_proof =
            SelectiveRepositoryUpdateProof::recovery_finalization_already_exact_test_only(
                &exact_selective_plan,
                id(ID_1),
                digest(A),
                digest(C),
                cursor(),
                cursor(),
            )
            .unwrap();
        assert!(first_window
            .completed_guard_proof(
                &FixedFinalizationExecution::new(substituted_guard_proof, &execution_authority,),
                &SuccessfulClosure,
                &SuccessfulGuardRelease,
                &SuccessfulRecheck,
                &EmptyPostReleaseHistory,
            )
            .is_err());

        let substituted_plan = selective_plan("v1", B, "Configuration");
        let substituted_plan_proof =
            SelectiveRepositoryUpdateProof::recovery_finalization_already_exact_test_only(
                &substituted_plan,
                id(ID_3),
                digest(A),
                digest(B),
                cursor(),
                cursor(),
            )
            .unwrap();
        assert!(acquire_window()
            .prepare_working_infobase_completion(&SuccessfulClosure)
            .unwrap()
            .completed_guard_proof(
                &FixedFinalizationExecution::new(substituted_plan_proof, &execution_authority,),
                &SuccessfulClosure,
                &SuccessfulGuardRelease,
                &SuccessfulRecheck,
                &EmptyPostReleaseHistory,
            )
            .is_err());

        let exact_update_proof =
            SelectiveRepositoryUpdateProof::recovery_finalization_already_exact_test_only(
                &exact_selective_plan,
                id(ID_3),
                digest(A),
                digest(C),
                cursor(),
                cursor(),
            )
            .unwrap();
        let completion = acquire_window()
            .prepare_working_infobase_completion(&SuccessfulClosure)
            .unwrap()
            .completed_guard_proof(
                &FixedFinalizationExecution::new(exact_update_proof, &execution_authority),
                &SuccessfulClosure,
                &SuccessfulGuardRelease,
                &SuccessfulRecheck,
                &EmptyPostReleaseHistory,
            )
            .unwrap();

        assert_eq!(completion.result_phase(), TaskPhase::Synchronized);
        assert!(completion.guard_proof().is_completed());
        assert_eq!(
            completion.guard_proof().finalization_plan_digest(),
            &approved_plan_digest,
        );
    }

    #[test]
    fn completion_holds_the_mode_lease_through_durable_terminalization_then_releases_guard() {
        let frozen = frozen_separate_action();
        let desired_projection = projection_with_plans(
            frozen.support_action_id().clone(),
            TaskPhase::Synchronized,
            cursor(),
            finalization_plan(cursor()),
            Some(desired_closure_plan(&frozen)),
            ManualSupportTargetMode::SeparateWorkingInfobase,
        );
        let desired_authority =
            ApprovedSupportRecoveryAuthority::new(frozen.clone(), desired_projection).unwrap();
        let materialized_closure = desired_authority
            .materialized_working_infobase_closure_plan(&ClosureMaterializer {
                base_cursor: cursor(),
                object_version_map_digest: digest(B),
            })
            .unwrap();
        let materialized_finalization = desired_authority
            .materialized_finalization_plan(&FinalizationMaterializer(selective_plan(
                "v1",
                C,
                "Configuration",
            )))
            .unwrap();
        let exact_selective_plan = materialized_finalization
            .plan()
            .materialized_selective_update_plan()
            .unwrap()
            .clone();
        let approved = ApprovedSupportRecoveryAuthority::new(
            frozen.clone(),
            projection_with_plans(
                frozen.support_action_id().clone(),
                TaskPhase::Synchronized,
                cursor(),
                materialized_finalization.plan().clone(),
                Some(materialized_closure.plan().clone()),
                ManualSupportTargetMode::SeparateWorkingInfobase,
            ),
        )
        .unwrap();
        let update_proof =
            SelectiveRepositoryUpdateProof::recovery_finalization_already_exact_test_only(
                &exact_selective_plan,
                id(ID_3),
                digest(A),
                digest(C),
                cursor(),
                cursor(),
            )
            .unwrap();
        let trace = Rc::new(RefCell::new(Vec::new()));
        let lease = TracedClosure {
            trace: Rc::clone(&trace),
        };
        let execution = TracedFinalizationExecution {
            inner: FixedFinalizationExecution::new(update_proof, &approved),
            trace: Rc::clone(&trace),
        };
        let guard_release = TracedGuardRelease {
            trace: Rc::clone(&trace),
        };
        let ApprovedSupportRecoveryGuardAttempt::Acquired(window) =
            approved.acquire_guard(&AcquiredGuard).unwrap()
        else {
            panic!("capability returned a complete guard window")
        };

        let completion = window
            .prepare_working_infobase_completion(&lease)
            .unwrap()
            .completed_guard_proof(
                &execution,
                &lease,
                &guard_release,
                &SuccessfulRecheck,
                &EmptyPostReleaseHistory,
            )
            .unwrap();

        assert!(completion.guard_proof().is_completed());
        assert_eq!(
            trace.borrow().as_slice(),
            [
                "acquire mode lease",
                "inspect",
                "guarded selective update",
                "durable authorization terminalization",
                "release mode lease",
                "release repository guard",
            ],
        );
    }

    #[test]
    fn reserved_mode_rejects_substituted_capability_observations_and_completes_with_exact_lease() {
        let frozen = frozen_reserved_action();
        let manual_actor_username = frozen.manual_actor_username().clone();
        let baseline_lock_set_digest = frozen.manual_actor_lock_baseline_digest().unwrap().clone();
        let reserved_original_identity_digest = frozen.reserved_original_identity_digest().clone();
        let exclusive_lease_capability_id = frozen
            .reserved_original_lease_capability_id()
            .unwrap()
            .clone();
        let expected_repository_fingerprint = frozen.expected_original_fingerprint().clone();
        let desired_authority = ApprovedSupportRecoveryAuthority::new(
            frozen.clone(),
            projection(
                frozen.support_action_id().clone(),
                TaskPhase::Synchronized,
                cursor(),
            ),
        )
        .unwrap();
        let materialized_finalization = desired_authority
            .materialized_finalization_plan(&FinalizationMaterializer(selective_plan(
                "v1",
                C,
                "Configuration",
            )))
            .unwrap();
        let exact_selective_plan = materialized_finalization
            .plan()
            .materialized_selective_update_plan()
            .unwrap()
            .clone();
        let approved_projection = projection_with_finalization(
            frozen.support_action_id().clone(),
            TaskPhase::Synchronized,
            cursor(),
            materialized_finalization.plan().clone(),
        );
        let execution_authority =
            ApprovedSupportRecoveryAuthority::new(frozen.clone(), approved_projection.clone())
                .unwrap();
        let acquire_window = || {
            let ApprovedSupportRecoveryGuardAttempt::Acquired(window) =
                ApprovedSupportRecoveryAuthority::new(frozen.clone(), approved_projection.clone())
                    .unwrap()
                    .acquire_guard(&AcquiredGuard)
                    .unwrap()
            else {
                panic!("capability returned a complete guard window")
            };
            window
        };
        let substituted_busy = FixedReservedBusy(
            ReservedOriginalLeaseBusyObservation::from_capability_adapter(
                manual_actor_username.clone(),
                baseline_lock_set_digest.clone(),
                baseline_lock_set_digest.clone(),
                digest(A),
                exclusive_lease_capability_id.clone(),
                RequiredNullable::null(),
            ),
        );
        assert!(acquire_window()
            .stopped_reserved_after_complete_guard_proof(
                &substituted_busy,
                &SuccessfulGuardRelease,
                &SuccessfulRecheck,
            )
            .is_err());

        let substituted_terminal_lease = FixedReservedLease {
            manual_actor_username: manual_actor_username.clone(),
            baseline_lock_set_digest: baseline_lock_set_digest.clone(),
            reserved_original_identity_digest: reserved_original_identity_digest.clone(),
            exclusive_lease_capability_id: CapabilityRowId::parse("substituted-lease.v1").unwrap(),
            expected_repository_fingerprint: expected_repository_fingerprint.clone(),
            observed_original_fingerprint: expected_repository_fingerprint.clone(),
        };
        assert!(acquire_window()
            .prepare_reserved_original_completion(&substituted_terminal_lease)
            .is_err());

        let exact_terminal_lease = FixedReservedLease {
            manual_actor_username,
            baseline_lock_set_digest,
            reserved_original_identity_digest,
            exclusive_lease_capability_id,
            expected_repository_fingerprint: expected_repository_fingerprint.clone(),
            observed_original_fingerprint: expected_repository_fingerprint,
        };
        let update_proof =
            SelectiveRepositoryUpdateProof::recovery_finalization_already_exact_test_only(
                &exact_selective_plan,
                id(ID_3),
                digest(A),
                digest(C),
                cursor(),
                cursor(),
            )
            .unwrap();
        let completion = acquire_window()
            .prepare_reserved_original_completion(&exact_terminal_lease)
            .unwrap()
            .completed_guard_proof(
                &FixedFinalizationExecution::new(update_proof, &execution_authority),
                &exact_terminal_lease,
                &SuccessfulGuardRelease,
                &SuccessfulRecheck,
                &EmptyPostReleaseHistory,
            )
            .unwrap();

        assert!(completion.guard_proof().is_completed());
    }

    #[test]
    fn corrective_mint_replays_only_the_sealed_instruction_and_frozen_handoff() {
        let frozen = frozen_reserved_action();
        let binding = frozen.armed_binding().unwrap();
        let frozen_handoff = binding.support_recovery_distributions()[0]
            .handoff()
            .clone();
        let exact_revalidation = handoff_revalidation(&frozen_handoff, ID_1);
        let plan = finalization_plan(cursor());
        let locks = plan.lock_targets().clone();
        let expected = SupportCorrectiveInstruction::new(
            SupportCorrectiveInstructionAuthority::test_only(
                frozen.support_action_id().clone(),
                binding.purpose(),
                ManualSupportTargetMode::ReservedOriginal,
                frozen.manual_actor_username().clone(),
                None,
                cursor(),
                locks.clone(),
                locks.clone(),
                vec![corrective_transition()],
                Vec::new(),
                vec![frozen_handoff.clone()],
                vec![exact_revalidation.clone()],
                plan.desired_support_graph_digest().clone(),
                plan.desired_repository_content_digest().clone(),
            )
            .unwrap(),
        )
        .unwrap();
        let projection = projection_with_plans_and_action(
            frozen.support_action_id().clone(),
            TaskPhase::Synchronized,
            cursor(),
            plan,
            None,
            ManualSupportTargetMode::ReservedOriginal,
            Some(SupportRecoveryExternalAction::corrective(expected.clone())),
        );
        let approved = ApprovedSupportRecoveryAuthority::new(frozen, projection).unwrap();
        let resolver = FixedCorrectiveLocks(locks);

        assert_eq!(
            approved
                .corrective_instruction(
                    vec![corrective_transition()],
                    Vec::new(),
                    vec![frozen_handoff.clone()],
                    vec![exact_revalidation],
                    &resolver,
                )
                .unwrap(),
            expected,
        );

        let substituted = substituted_handoff();
        assert!(approved
            .corrective_instruction(
                vec![corrective_transition()],
                Vec::new(),
                vec![substituted.clone()],
                vec![handoff_revalidation(&substituted, ID_1)],
                &resolver,
            )
            .is_err());
        assert!(approved
            .corrective_instruction(
                vec![corrective_transition()],
                Vec::new(),
                vec![frozen_handoff.clone()],
                vec![handoff_revalidation(&frozen_handoff, ID_3)],
                &resolver,
            )
            .is_err());
    }

    #[test]
    fn approved_authority_rejects_corrective_distribution_capability_substitution() {
        let frozen = frozen_reserved_action();
        let binding = frozen.armed_binding().unwrap();
        let frozen_handoff = binding.support_recovery_distributions()[0]
            .handoff()
            .clone();
        let plan = finalization_plan(cursor());
        let locks = plan.lock_targets().clone();
        let substituted_transition =
            corrective_transition_with_capability("substituted-recovery.v1");
        let expected = SupportCorrectiveInstruction::new(
            SupportCorrectiveInstructionAuthority::test_only(
                frozen.support_action_id().clone(),
                binding.purpose(),
                ManualSupportTargetMode::ReservedOriginal,
                frozen.manual_actor_username().clone(),
                None,
                cursor(),
                locks.clone(),
                locks,
                vec![substituted_transition],
                Vec::new(),
                vec![frozen_handoff.clone()],
                vec![handoff_revalidation(&frozen_handoff, ID_1)],
                plan.desired_support_graph_digest().clone(),
                plan.desired_repository_content_digest().clone(),
            )
            .unwrap(),
        )
        .unwrap();
        let projection = projection_with_plans_and_action(
            frozen.support_action_id().clone(),
            TaskPhase::Synchronized,
            cursor(),
            plan,
            None,
            ManualSupportTargetMode::ReservedOriginal,
            Some(SupportRecoveryExternalAction::corrective(expected)),
        );

        assert!(ApprovedSupportRecoveryAuthority::new(frozen, projection).is_err());
    }

    #[test]
    fn approved_authority_rejects_corrective_transition_not_inverse_to_frozen_action() {
        let frozen = frozen_reserved_action();
        let binding = frozen.armed_binding().unwrap();
        let plan = finalization_plan(cursor());
        let locks = plan.lock_targets().clone();
        let expected = SupportCorrectiveInstruction::new(
            SupportCorrectiveInstructionAuthority::test_only(
                frozen.support_action_id().clone(),
                binding.purpose(),
                ManualSupportTargetMode::ReservedOriginal,
                frozen.manual_actor_username().clone(),
                None,
                cursor(),
                locks.clone(),
                locks,
                vec![ordinary_non_inverse_transition()],
                Vec::new(),
                Vec::new(),
                Vec::new(),
                plan.desired_support_graph_digest().clone(),
                plan.desired_repository_content_digest().clone(),
            )
            .unwrap(),
        )
        .unwrap();
        let projection = projection_with_plans_and_action(
            frozen.support_action_id().clone(),
            TaskPhase::Synchronized,
            cursor(),
            plan,
            None,
            ManualSupportTargetMode::ReservedOriginal,
            Some(SupportRecoveryExternalAction::corrective(expected)),
        );

        assert!(ApprovedSupportRecoveryAuthority::new(frozen, projection).is_err());
    }

    const _: fn() = || {
        trait AmbiguousIfDeserialize<Marker> {
            fn assert_not_deserialize() {}
        }
        struct ImplementsDeserialize;
        impl<T: ?Sized> AmbiguousIfDeserialize<()> for T {}
        impl<T: serde::de::DeserializeOwned> AmbiguousIfDeserialize<ImplementsDeserialize> for T {}
        let _ =
            <ApprovedSupportRecoveryAuthority as AmbiguousIfDeserialize<_>>::assert_not_deserialize;
    };

    const _: fn() = || {
        trait AmbiguousIfClone<Marker> {
            fn assert_not_clone() {}
        }
        struct ImplementsClone;
        impl<T: ?Sized> AmbiguousIfClone<()> for T {}
        impl<T: Clone> AmbiguousIfClone<ImplementsClone> for T {}

        let _ = <ApprovedSupportRecoveryGuardWindow as AmbiguousIfClone<_>>::assert_not_clone;
        let _ = <PreparedSupportRecoveryStopWindow as AmbiguousIfClone<_>>::assert_not_clone;
        let _ = <PreparedSupportRecoveryCompletionWindow as AmbiguousIfClone<_>>::assert_not_clone;
        let _ =
            <PreparedReservedSupportRecoveryCompletionWindow as AmbiguousIfClone<_>>::assert_not_clone;
        let _ =
            <PreparedReservedSupportRecoveryStopWindow as AmbiguousIfClone<_>>::assert_not_clone;
        let _ = <ApprovedSupportRecoveryAuthority as AmbiguousIfClone<_>>::assert_not_clone;
        let _ = <SupportRecoveryAuthorityToken as AmbiguousIfClone<_>>::assert_not_clone;
        let _ = <SupportRecoveryBootstrapAuthority as AmbiguousIfClone<_>>::assert_not_clone;
        let _ = <PreparedSupportRecoveryBootstrap as AmbiguousIfClone<_>>::assert_not_clone;
        let _ =
            <PreparedSupportRecoveryBootstrapProjection as AmbiguousIfClone<_>>::assert_not_clone;
        let _ = <PreparedSupportRecoveryReplanProjection as AmbiguousIfClone<_>>::assert_not_clone;
        let _ =
            <BoundSupportRecoveryReplanFinalizationPlan as AmbiguousIfClone<_>>::assert_not_clone;
        let _ = <BoundSupportRecoveryReplanWorkingInfobaseClosurePlan as AmbiguousIfClone<_>>::assert_not_clone;
        let _ = <SupportRecoveryDesiredPlans as AmbiguousIfClone<_>>::assert_not_clone;
        let _ =
            <CurrentBlockedSupportRecoveryGuardAttempt as AmbiguousIfClone<_>>::assert_not_clone;
        let _ = <PreparedSupportRecoveryExternalReplan as AmbiguousIfClone<_>>::assert_not_clone;
        let _ = <BoundSupportRecoveryExternalWait as AmbiguousIfClone<_>>::assert_not_clone;
        let _ = <BoundSupportRecoveryActionCatalog as AmbiguousIfClone<_>>::assert_not_clone;
        let _ = <ManualWorkingInfobaseAcquiredLease as AmbiguousIfClone<_>>::assert_not_clone;
        let _ = <ManualWorkingInfobaseLiveLeaseWindow as AmbiguousIfClone<_>>::assert_not_clone;
        let _ = <ReservedOriginalAcquiredLease as AmbiguousIfClone<_>>::assert_not_clone;
        let _ = <ReservedOriginalLiveLeaseWindow as AmbiguousIfClone<_>>::assert_not_clone;
    };
}
