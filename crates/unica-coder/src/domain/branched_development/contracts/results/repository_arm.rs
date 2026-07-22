use crate::domain::branched_development::canonical_json::{
    canonical_contract_digest, contract_digest_record_sealed, ContractDigestRecord,
};
use crate::domain::branched_development::contracts::instructions::ManualSupportInstruction;
use crate::domain::branched_development::contracts::repository::{
    RepositoryHistoryCursor, RepositoryOwnerIdentity, ValidatedRepositoryHistoryPartition,
};
use crate::domain::branched_development::contracts::requests::repository::{
    ValidatedArmApplyRequest, ValidatedArmPreviewRequest,
};
use crate::domain::branched_development::contracts::scalars::{
    OriginalProjectCwd, RepositoryUsername,
};
use crate::domain::branched_development::contracts::support::{
    ActiveSupportActionResumeHandle, ManualSupportTargetMode, ManualWorkingInfobaseIdentity,
    SupportActionArmingReceipt, SupportActionPurpose, SupportRecoveryDistributionCoverageAuthority,
    SupportRootLockObservation, SupportUpdateAuthorizationProjection,
};
use crate::domain::branched_development::{OperationId, Sha256Digest, TaskId, TaskPhase, UnicaId};
use schemars::JsonSchema;
use serde::Serialize;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SupportPrerequisiteArmContractError(&'static str);

impl SupportPrerequisiteArmContractError {
    pub(crate) const fn adapter_failure(message: &'static str) -> Self {
        Self(message)
    }
}

impl fmt::Display for SupportPrerequisiteArmContractError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.0)
    }
}

impl std::error::Error for SupportPrerequisiteArmContractError {}

macro_rules! wire_literal {
    ($name:ident, $wire:literal) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
        enum $name {
            #[serde(rename = $wire)]
            Value,
        }
    };
}

wire_literal!(SupportPrerequisiteArmMode, "supportPrerequisiteArm");
wire_literal!(PreviewStage, "preview");
wire_literal!(ApplyStage, "apply");

/// Immutable scope handed to the one atomic arm-observation adapter call.
/// Every reference originates in the live support authorization; the adapter
/// cannot select a different action, target mode, or expected lineage.
#[derive(Debug, Clone, Copy)]
pub(crate) struct SupportArmObservationScope<'a> {
    support_action_id: &'a UnicaId,
    support_action_digest: &'a Sha256Digest,
    support_gate_id: &'a UnicaId,
    support_gate_digest: &'a Sha256Digest,
    candidate_set_digest: &'a Sha256Digest,
    expected_before_history_cursor: &'a RepositoryHistoryCursor,
    expected_relevant_baseline_digest: &'a Sha256Digest,
    expected_support_graph_digest: &'a Sha256Digest,
    expected_recovery_distribution_set_digest: &'a Sha256Digest,
    expected_original_fingerprint: &'a Sha256Digest,
    manual_target_mode: ManualSupportTargetMode,
    manual_actor_username: &'a RepositoryUsername,
    reserved_original_identity_digest: &'a Sha256Digest,
    manual_working_infobase_identity: Option<&'a ManualWorkingInfobaseIdentity>,
}

impl SupportArmObservationScope<'_> {
    pub(crate) const fn support_action_id(&self) -> &UnicaId {
        self.support_action_id
    }

    pub(crate) const fn support_action_digest(&self) -> &Sha256Digest {
        self.support_action_digest
    }

    pub(crate) const fn support_gate_id(&self) -> &UnicaId {
        self.support_gate_id
    }

    pub(crate) const fn support_gate_digest(&self) -> &Sha256Digest {
        self.support_gate_digest
    }

    pub(crate) const fn candidate_set_digest(&self) -> &Sha256Digest {
        self.candidate_set_digest
    }

    pub(crate) const fn expected_before_history_cursor(&self) -> &RepositoryHistoryCursor {
        self.expected_before_history_cursor
    }

    pub(crate) const fn expected_relevant_baseline_digest(&self) -> &Sha256Digest {
        self.expected_relevant_baseline_digest
    }

    pub(crate) const fn expected_support_graph_digest(&self) -> &Sha256Digest {
        self.expected_support_graph_digest
    }

    pub(crate) const fn expected_recovery_distribution_set_digest(&self) -> &Sha256Digest {
        self.expected_recovery_distribution_set_digest
    }

    pub(crate) const fn expected_original_fingerprint(&self) -> &Sha256Digest {
        self.expected_original_fingerprint
    }

    pub(crate) const fn manual_target_mode(&self) -> ManualSupportTargetMode {
        self.manual_target_mode
    }

    pub(crate) const fn manual_actor_username(&self) -> &RepositoryUsername {
        self.manual_actor_username
    }

    pub(crate) const fn reserved_original_identity_digest(&self) -> &Sha256Digest {
        self.reserved_original_identity_digest
    }

    pub(crate) const fn manual_working_infobase_identity(
        &self,
    ) -> Option<&ManualWorkingInfobaseIdentity> {
        self.manual_working_infobase_identity
    }
}

/// One inseparable result of the platform's current arm-capability probe.
/// It deliberately excludes `armingDigest`: the result contract derives that
/// digest only after validating this snapshot against the live authorization.
pub(crate) struct SupportArmCapabilitySnapshot {
    observed_support_gate_digest: Sha256Digest,
    observed_candidate_set_digest: Sha256Digest,
    observed_relevant_baseline_digest: Sha256Digest,
    observed_recovery_coverage: SupportRecoveryDistributionCoverageAuthority,
    observed_original_fingerprint: Sha256Digest,
    bound_root_lock: Box<dyn SupportArmBoundRootLockCapabilityAuthority>,
}

impl fmt::Debug for SupportArmCapabilitySnapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SupportArmCapabilitySnapshot")
            .field(
                "observed_support_gate_digest",
                &self.observed_support_gate_digest,
            )
            .field(
                "observed_candidate_set_digest",
                &self.observed_candidate_set_digest,
            )
            .finish_non_exhaustive()
    }
}

/// Mode-specific root-lock capability. For `reservedOriginal`, this authority
/// is the only component allowed to bind the opaque reserved identity digest
/// to the concrete owner observation; no result constructor accepts those two
/// values independently. For a separate working IB, the final factory also
/// checks the concrete computer/infobase components structurally.
pub(crate) trait SupportArmBoundRootLockCapabilityAuthority {
    fn manual_target_mode(&self) -> ManualSupportTargetMode;

    fn target_identity_digest(&self) -> &Sha256Digest;

    fn root_lock_observation(&self) -> &SupportRootLockObservation;

    fn proves_exact_scope_binding(&self, scope: &SupportArmObservationScope<'_>) -> bool;

    fn into_root_lock_observation(self: Box<Self>) -> SupportRootLockObservation;
}

impl SupportArmCapabilitySnapshot {
    pub(crate) fn from_capability_adapter(
        observed_support_gate_digest: Sha256Digest,
        observed_candidate_set_digest: Sha256Digest,
        observed_relevant_baseline_digest: Sha256Digest,
        observed_recovery_coverage: SupportRecoveryDistributionCoverageAuthority,
        observed_original_fingerprint: Sha256Digest,
        bound_root_lock: Box<dyn SupportArmBoundRootLockCapabilityAuthority>,
    ) -> Self {
        Self {
            observed_support_gate_digest,
            observed_candidate_set_digest,
            observed_relevant_baseline_digest,
            observed_recovery_coverage,
            observed_original_fingerprint,
            bound_root_lock,
        }
    }
}

/// Atomic platform boundary for all current facts needed by arming. The
/// implementation must read one coherent snapshot for the supplied scope and
/// complete capability-validated history partition.
pub(crate) trait SupportArmCapabilityResolver {
    fn observe(
        &mut self,
        scope: &SupportArmObservationScope<'_>,
        history_partition: &ValidatedRepositoryHistoryPartition,
    ) -> Result<SupportArmCapabilitySnapshot, SupportPrerequisiteArmContractError>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SupportPrerequisiteArmPreviewDigestRecord {
    mode: SupportPrerequisiteArmMode,
    stage: PreviewStage,
    purpose: SupportActionPurpose,
    origin_phase: TaskPhase,
    support_action_id: UnicaId,
    support_action_digest: Sha256Digest,
    support_gate_id: UnicaId,
    support_gate_digest: Sha256Digest,
    candidate_set_digest: Sha256Digest,
    expected_before_history_cursor: RepositoryHistoryCursor,
    observed_history_cursor: RepositoryHistoryCursor,
    history_partition: ValidatedRepositoryHistoryPartition,
    expected_relevant_baseline_digest: Sha256Digest,
    observed_relevant_baseline_digest: Sha256Digest,
    expected_support_graph_digest: Sha256Digest,
    observed_support_graph_digest: Sha256Digest,
    expected_recovery_distribution_set_digest: Sha256Digest,
    observed_recovery_distribution_set_digest: Sha256Digest,
    expected_original_fingerprint: Sha256Digest,
    observed_original_fingerprint: Sha256Digest,
    manual_target_mode: ManualSupportTargetMode,
    expected_manual_actor_username: RepositoryUsername,
    root_lock_observation: SupportRootLockObservation,
}

impl contract_digest_record_sealed::Sealed for SupportPrerequisiteArmPreviewDigestRecord {}
impl ContractDigestRecord for SupportPrerequisiteArmPreviewDigestRecord {}

fn authorization_scope(
    authorization: &SupportUpdateAuthorizationProjection,
) -> SupportArmObservationScope<'_> {
    SupportArmObservationScope {
        support_action_id: authorization.support_action_id(),
        support_action_digest: authorization.support_action_digest(),
        support_gate_id: authorization.support_gate_id(),
        support_gate_digest: authorization.support_gate_digest(),
        candidate_set_digest: authorization.candidate_set_digest(),
        expected_before_history_cursor: authorization.expected_before_history_cursor(),
        expected_relevant_baseline_digest: authorization.expected_relevant_baseline_digest(),
        expected_support_graph_digest: authorization.expected_support_graph_digest(),
        expected_recovery_distribution_set_digest: authorization
            .support_recovery_distribution_set_digest(),
        expected_original_fingerprint: authorization.expected_original_fingerprint(),
        manual_target_mode: authorization.manual_target_mode(),
        manual_actor_username: authorization.manual_actor_username(),
        reserved_original_identity_digest: authorization.reserved_original_identity_digest(),
        manual_working_infobase_identity: authorization.manual_working_infobase_identity(),
    }
}

fn owner_matches_manual_binding(
    owner: &RepositoryOwnerIdentity,
    scope: &SupportArmObservationScope<'_>,
) -> bool {
    if owner.username() != scope.manual_actor_username {
        return false;
    }
    match (
        scope.manual_target_mode,
        scope.manual_working_infobase_identity,
    ) {
        (ManualSupportTargetMode::ReservedOriginal, None) => true,
        (ManualSupportTargetMode::SeparateWorkingInfobase, Some(identity)) => {
            owner.computer() == Some(identity.computer())
                && owner.infobase() == Some(identity.infobase())
        }
        _ => false,
    }
}

fn record_matches_authorization(
    record: &SupportPrerequisiteArmPreviewDigestRecord,
    authorization: &SupportUpdateAuthorizationProjection,
) -> bool {
    authorization.arming_receipt().is_none()
        && record.purpose == authorization.purpose()
        && record.origin_phase == authorization.origin_phase()
        && &record.support_action_id == authorization.support_action_id()
        && &record.support_action_digest == authorization.support_action_digest()
        && &record.support_gate_id == authorization.support_gate_id()
        && &record.support_gate_digest == authorization.support_gate_digest()
        && &record.candidate_set_digest == authorization.candidate_set_digest()
        && &record.expected_before_history_cursor == authorization.expected_before_history_cursor()
        && &record.expected_relevant_baseline_digest
            == authorization.expected_relevant_baseline_digest()
        && &record.expected_support_graph_digest == authorization.expected_support_graph_digest()
        && &record.expected_recovery_distribution_set_digest
            == authorization.support_recovery_distribution_set_digest()
        && &record.expected_original_fingerprint == authorization.expected_original_fingerprint()
        && record.manual_target_mode == authorization.manual_target_mode()
        && &record.expected_manual_actor_username == authorization.manual_actor_username()
        && record.history_partition.start_cursor() == authorization.expected_before_history_cursor()
        && record.history_partition.through_inclusive() == &record.observed_history_cursor
        && record.observed_relevant_baseline_digest == record.expected_relevant_baseline_digest
        && record.observed_support_graph_digest == record.expected_support_graph_digest
        && record.observed_recovery_distribution_set_digest
            == record.expected_recovery_distribution_set_digest
        && record.observed_original_fingerprint == record.expected_original_fingerprint
}

/// Capability-validated, all-stable observation for one awaiting action.
/// Non-`Clone` and non-wire so a generic partition or independent digest set
/// cannot be promoted into a preview/apply authority.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ValidatedArmObservationAuthority {
    record: SupportPrerequisiteArmPreviewDigestRecord,
}

impl ValidatedArmObservationAuthority {
    pub(crate) fn from_capability_resolver(
        authorization: &SupportUpdateAuthorizationProjection,
        history_partition: ValidatedRepositoryHistoryPartition,
        resolver: &mut dyn SupportArmCapabilityResolver,
    ) -> Result<Self, SupportPrerequisiteArmContractError> {
        if authorization.arming_receipt().is_some()
            || history_partition.start_cursor()
                != authorization.expected_before_history_cursor()
            || !history_partition.classifications().all(|classification| {
                classification
                    == crate::domain::branched_development::contracts::repository::RepositoryHistoryPartitionClassification::UnrelatedRoutine
            })
        {
            return Err(SupportPrerequisiteArmContractError(
                "arm observation requires the awaiting action's exact all-unrelated history prefix",
            ));
        }

        let scope = authorization_scope(authorization);
        let snapshot = resolver.observe(&scope, &history_partition)?;
        let distributions = snapshot.observed_recovery_coverage.distributions();
        let expected_target_identity_digest = match (
            scope.manual_target_mode,
            scope.manual_working_infobase_identity,
        ) {
            (ManualSupportTargetMode::ReservedOriginal, None) => {
                scope.reserved_original_identity_digest
            }
            (ManualSupportTargetMode::SeparateWorkingInfobase, Some(identity)) => identity.digest(),
            _ => {
                return Err(SupportPrerequisiteArmContractError(
                    "manual target mode and working-infobase identity disagree",
                ));
            }
        };
        let inspected_root_lock_observation =
            snapshot.bound_root_lock.root_lock_observation().clone();
        let Some(owner) = inspected_root_lock_observation.owner().as_ref() else {
            return Err(SupportPrerequisiteArmContractError(
                "arm root-lock observation does not name an owner",
            ));
        };
        if snapshot.observed_support_gate_digest != *scope.support_gate_digest
            || snapshot.observed_candidate_set_digest != *scope.candidate_set_digest
            || snapshot.observed_relevant_baseline_digest
                != *scope.expected_relevant_baseline_digest
            || snapshot.observed_recovery_coverage.support_graph_digest()
                != scope.expected_support_graph_digest
            || distributions.digest() != scope.expected_recovery_distribution_set_digest
            || !snapshot
                .observed_recovery_coverage
                .covers_transitions(authorization.authorized_transitions())
            || !distributions.matches_manual_binding(
                scope.manual_target_mode,
                scope.manual_actor_username,
                scope.manual_working_infobase_identity,
            )
            || snapshot.observed_original_fingerprint != *scope.expected_original_fingerprint
            || snapshot.bound_root_lock.manual_target_mode() != scope.manual_target_mode
            || snapshot.bound_root_lock.target_identity_digest() != expected_target_identity_digest
            || !snapshot.bound_root_lock.proves_exact_scope_binding(&scope)
            || !owner_matches_manual_binding(owner, &scope)
        {
            return Err(SupportPrerequisiteArmContractError(
                "current arm capability snapshot differs from its support authorization",
            ));
        }

        let root_lock_observation = snapshot.bound_root_lock.into_root_lock_observation();
        if root_lock_observation != inspected_root_lock_observation {
            return Err(SupportPrerequisiteArmContractError(
                "arm root-lock capability changed its observation while being consumed",
            ));
        }
        let record = SupportPrerequisiteArmPreviewDigestRecord {
            mode: SupportPrerequisiteArmMode::Value,
            stage: PreviewStage::Value,
            purpose: authorization.purpose(),
            origin_phase: authorization.origin_phase(),
            support_action_id: authorization.support_action_id().clone(),
            support_action_digest: authorization.support_action_digest().clone(),
            support_gate_id: authorization.support_gate_id().clone(),
            support_gate_digest: authorization.support_gate_digest().clone(),
            candidate_set_digest: authorization.candidate_set_digest().clone(),
            expected_before_history_cursor: authorization.expected_before_history_cursor().clone(),
            observed_history_cursor: history_partition.through_inclusive().clone(),
            history_partition,
            expected_relevant_baseline_digest: authorization
                .expected_relevant_baseline_digest()
                .clone(),
            observed_relevant_baseline_digest: snapshot.observed_relevant_baseline_digest,
            expected_support_graph_digest: authorization.expected_support_graph_digest().clone(),
            observed_support_graph_digest: snapshot
                .observed_recovery_coverage
                .support_graph_digest()
                .clone(),
            expected_recovery_distribution_set_digest: authorization
                .support_recovery_distribution_set_digest()
                .clone(),
            observed_recovery_distribution_set_digest: distributions.digest().clone(),
            expected_original_fingerprint: authorization.expected_original_fingerprint().clone(),
            observed_original_fingerprint: snapshot.observed_original_fingerprint,
            manual_target_mode: authorization.manual_target_mode(),
            expected_manual_actor_username: authorization.manual_actor_username().clone(),
            root_lock_observation,
        };
        Ok(Self { record })
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct SupportPrerequisiteArmPreviewAuthority {
    cwd: OriginalProjectCwd,
    task_id: TaskId,
    expected_status_digest: Sha256Digest,
    record: SupportPrerequisiteArmPreviewDigestRecord,
    arming_digest: Sha256Digest,
}

impl SupportPrerequisiteArmPreviewAuthority {
    pub(crate) fn from_authorities(
        request: ValidatedArmPreviewRequest<'_>,
        authorization: SupportUpdateAuthorizationProjection,
        observation: ValidatedArmObservationAuthority,
    ) -> Result<Self, SupportPrerequisiteArmContractError> {
        if request.support_action_id() != authorization.support_action_id()
            || request.expected_support_action_digest() != authorization.support_action_digest()
            || !record_matches_authorization(&observation.record, &authorization)
        {
            return Err(SupportPrerequisiteArmContractError(
                "arm preview request, authorization, and observation lineage disagree",
            ));
        }
        let arming_digest = canonical_contract_digest(&observation.record, None).map_err(|_| {
            SupportPrerequisiteArmContractError("arm preview digest canonicalization failed")
        })?;
        Ok(Self {
            cwd: request.cwd().clone(),
            task_id: request.task_id().clone(),
            expected_status_digest: request.expected_status_digest().clone(),
            record: observation.record,
            arming_digest,
        })
    }

    pub(crate) const fn cwd(&self) -> &OriginalProjectCwd {
        &self.cwd
    }

    pub(crate) const fn task_id(&self) -> &TaskId {
        &self.task_id
    }

    pub(crate) const fn expected_status_digest(&self) -> &Sha256Digest {
        &self.expected_status_digest
    }

    pub(crate) const fn arming_digest(&self) -> &Sha256Digest {
        &self.arming_digest
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SupportPrerequisiteArmPreviewData {
    mode: SupportPrerequisiteArmMode,
    stage: PreviewStage,
    purpose: SupportActionPurpose,
    origin_phase: TaskPhase,
    support_action_id: UnicaId,
    support_action_digest: Sha256Digest,
    support_gate_id: UnicaId,
    support_gate_digest: Sha256Digest,
    candidate_set_digest: Sha256Digest,
    expected_before_history_cursor: RepositoryHistoryCursor,
    observed_history_cursor: RepositoryHistoryCursor,
    history_partition: ValidatedRepositoryHistoryPartition,
    expected_relevant_baseline_digest: Sha256Digest,
    observed_relevant_baseline_digest: Sha256Digest,
    expected_support_graph_digest: Sha256Digest,
    observed_support_graph_digest: Sha256Digest,
    expected_recovery_distribution_set_digest: Sha256Digest,
    observed_recovery_distribution_set_digest: Sha256Digest,
    expected_original_fingerprint: Sha256Digest,
    observed_original_fingerprint: Sha256Digest,
    manual_target_mode: ManualSupportTargetMode,
    expected_manual_actor_username: RepositoryUsername,
    root_lock_observation: SupportRootLockObservation,
    arming_digest: Sha256Digest,
}

impl SupportPrerequisiteArmPreviewData {
    pub(crate) fn from_authority(authority: SupportPrerequisiteArmPreviewAuthority) -> Self {
        let SupportPrerequisiteArmPreviewDigestRecord {
            mode,
            stage,
            purpose,
            origin_phase,
            support_action_id,
            support_action_digest,
            support_gate_id,
            support_gate_digest,
            candidate_set_digest,
            expected_before_history_cursor,
            observed_history_cursor,
            history_partition,
            expected_relevant_baseline_digest,
            observed_relevant_baseline_digest,
            expected_support_graph_digest,
            observed_support_graph_digest,
            expected_recovery_distribution_set_digest,
            observed_recovery_distribution_set_digest,
            expected_original_fingerprint,
            observed_original_fingerprint,
            manual_target_mode,
            expected_manual_actor_username,
            root_lock_observation,
        } = authority.record;
        Self {
            mode,
            stage,
            purpose,
            origin_phase,
            support_action_id,
            support_action_digest,
            support_gate_id,
            support_gate_digest,
            candidate_set_digest,
            expected_before_history_cursor,
            observed_history_cursor,
            history_partition,
            expected_relevant_baseline_digest,
            observed_relevant_baseline_digest,
            expected_support_graph_digest,
            observed_support_graph_digest,
            expected_recovery_distribution_set_digest,
            observed_recovery_distribution_set_digest,
            expected_original_fingerprint,
            observed_original_fingerprint,
            manual_target_mode,
            expected_manual_actor_username,
            root_lock_observation,
            arming_digest: authority.arming_digest,
        }
    }

    pub(crate) const fn arming_digest(&self) -> &Sha256Digest {
        &self.arming_digest
    }
}

/// Exclusive status-store lease for one exact arming mutation. `Box<dyn ...>`
/// has no clone path; the authoritative resolver must refuse a second lease
/// for the same current status lineage until this one is committed/released.
pub(crate) trait SupportArmStatusCasLease {
    fn binds(&self, request: &ValidatedArmApplyRequest<'_>) -> bool;

    fn commit_armed(
        self: Box<Self>,
        armed: &ActiveSupportActionResumeHandle,
    ) -> Result<(), SupportPrerequisiteArmContractError>;
}

pub(crate) trait SupportArmStatusCasResolver {
    fn acquire(
        &mut self,
        request: &ValidatedArmApplyRequest<'_>,
    ) -> Result<Box<dyn SupportArmStatusCasLease>, SupportPrerequisiteArmContractError>;
}

pub(crate) struct ValidatedArmStatusCasAuthority {
    lease: Box<dyn SupportArmStatusCasLease>,
}

impl fmt::Debug for ValidatedArmStatusCasAuthority {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ValidatedArmStatusCasAuthority")
            .finish_non_exhaustive()
    }
}

impl ValidatedArmStatusCasAuthority {
    fn acquire(
        request: &ValidatedArmApplyRequest<'_>,
        resolver: &mut dyn SupportArmStatusCasResolver,
    ) -> Result<Self, SupportPrerequisiteArmContractError> {
        let lease = resolver.acquire(request)?;
        if !lease.binds(request) {
            return Err(SupportPrerequisiteArmContractError(
                "status CAS lease is bound to another arm request lineage",
            ));
        }
        Ok(Self { lease })
    }

    fn commit_armed(
        self,
        armed: &ActiveSupportActionResumeHandle,
    ) -> Result<(), SupportPrerequisiteArmContractError> {
        self.lease.commit_armed(armed)
    }
}

pub(crate) trait SupportArmReceiptIdIssuer {
    fn issue(
        &mut self,
        support_action_id: &UnicaId,
        operation_id: &OperationId,
    ) -> Result<UnicaId, SupportPrerequisiteArmContractError>;
}

pub(crate) struct ValidatedRepeatedArmPreviewAuthority {
    cwd: OriginalProjectCwd,
    task_id: TaskId,
    operation_id: OperationId,
    expected_status_digest: Sha256Digest,
    support_action_id: UnicaId,
    support_action_digest: Sha256Digest,
    record: SupportPrerequisiteArmPreviewDigestRecord,
    arming_digest: Sha256Digest,
    arming_receipt_id: UnicaId,
    status_cas: ValidatedArmStatusCasAuthority,
}

impl fmt::Debug for ValidatedRepeatedArmPreviewAuthority {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ValidatedRepeatedArmPreviewAuthority")
            .field("support_action_id", &self.support_action_id)
            .field("arming_digest", &self.arming_digest)
            .finish_non_exhaustive()
    }
}

impl ValidatedRepeatedArmPreviewAuthority {
    pub(crate) fn from_authorities(
        request: &ValidatedArmApplyRequest<'_>,
        authorization: SupportUpdateAuthorizationProjection,
        observation: ValidatedArmObservationAuthority,
        status_cas_resolver: &mut dyn SupportArmStatusCasResolver,
        receipt_id_issuer: &mut dyn SupportArmReceiptIdIssuer,
    ) -> Result<Self, SupportPrerequisiteArmContractError> {
        if request.support_action_id() != authorization.support_action_id()
            || request.expected_support_action_digest() != authorization.support_action_digest()
            || !record_matches_authorization(&observation.record, &authorization)
        {
            return Err(SupportPrerequisiteArmContractError(
                "repeated arm preview differs from the apply request or live authorization",
            ));
        }
        let arming_digest = canonical_contract_digest(&observation.record, None).map_err(|_| {
            SupportPrerequisiteArmContractError("repeated arm preview digest failed")
        })?;
        if request.approved_arming_digest() != &arming_digest {
            return Err(SupportPrerequisiteArmContractError(
                "apply approval differs from the freshly repeated arm preview",
            ));
        }
        let arming_receipt_id =
            receipt_id_issuer.issue(request.support_action_id(), request.operation_id())?;
        let status_cas = ValidatedArmStatusCasAuthority::acquire(request, status_cas_resolver)?;
        Ok(Self {
            cwd: request.cwd().clone(),
            task_id: request.task_id().clone(),
            operation_id: request.operation_id().clone(),
            expected_status_digest: request.expected_status_digest().clone(),
            support_action_id: request.support_action_id().clone(),
            support_action_digest: request.expected_support_action_digest().clone(),
            record: observation.record,
            arming_digest,
            arming_receipt_id,
            status_cas,
        })
    }
}

fn repeated_context_matches_request(
    repeated: &ValidatedRepeatedArmPreviewAuthority,
    request: &ValidatedArmApplyRequest<'_>,
) -> bool {
    &repeated.cwd == request.cwd()
        && &repeated.task_id == request.task_id()
        && &repeated.operation_id == request.operation_id()
        && &repeated.expected_status_digest == request.expected_status_digest()
        && &repeated.support_action_id == request.support_action_id()
        && &repeated.support_action_digest == request.expected_support_action_digest()
        && &repeated.arming_digest == request.approved_arming_digest()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SupportPrerequisiteArmData {
    mode: SupportPrerequisiteArmMode,
    stage: ApplyStage,
    support_action_id: UnicaId,
    support_action_digest: Sha256Digest,
    arming_receipt: SupportActionArmingReceipt,
    required_external_action: ManualSupportInstruction,
    arming_digest: Sha256Digest,
}

/// Atomic apply product. The armed authorization is retained beside the wire
/// result so callers cannot publish the data while silently discarding the
/// state transition.
pub(crate) struct SupportPrerequisiteArmCommitAuthority {
    data: SupportPrerequisiteArmData,
    armed: ActiveSupportActionResumeHandle,
}

impl fmt::Debug for SupportPrerequisiteArmCommitAuthority {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SupportPrerequisiteArmCommitAuthority")
            .field("support_action_id", &self.data.support_action_id)
            .field("arming_digest", &self.data.arming_digest)
            .finish_non_exhaustive()
    }
}

impl SupportPrerequisiteArmCommitAuthority {
    pub(crate) fn apply(
        request: ValidatedArmApplyRequest<'_>,
        repeated: ValidatedRepeatedArmPreviewAuthority,
        active: ActiveSupportActionResumeHandle,
    ) -> Result<Self, SupportPrerequisiteArmContractError> {
        if !repeated_context_matches_request(&repeated, &request) {
            return Err(SupportPrerequisiteArmContractError(
                "arm apply request differs from its repeated preview authority",
            ));
        }
        let Some(live_authorization) = active.support_update_authorization_projection() else {
            return Err(SupportPrerequisiteArmContractError(
                "frozen support authorization cannot be armed",
            ));
        };
        if !record_matches_authorization(&repeated.record, &live_authorization) {
            return Err(SupportPrerequisiteArmContractError(
                "arm apply handle is not the exact awaiting authorization",
            ));
        }
        let expected_owner = repeated
            .record
            .root_lock_observation
            .owner()
            .as_ref()
            .ok_or(SupportPrerequisiteArmContractError(
                "arm apply lost its manual root owner",
            ))?;
        let receipt = SupportActionArmingReceipt::new(
            repeated.arming_receipt_id,
            repeated.record.support_action_id.clone(),
            repeated.record.support_action_digest.clone(),
            repeated.record.expected_before_history_cursor.clone(),
            repeated.record.observed_history_cursor.clone(),
            repeated.record.history_partition.clone(),
            repeated.record.support_gate_digest.clone(),
            repeated.record.candidate_set_digest.clone(),
            repeated.record.expected_relevant_baseline_digest.clone(),
            repeated.record.expected_support_graph_digest.clone(),
            repeated
                .record
                .expected_recovery_distribution_set_digest
                .clone(),
            repeated.record.expected_original_fingerprint.clone(),
            repeated.record.manual_target_mode,
            repeated.record.root_lock_observation.clone(),
            expected_owner,
        )
        .map_err(|_| {
            SupportPrerequisiteArmContractError(
                "fresh arm preview could not produce its exact arming receipt",
            )
        })?;
        let armed = active.arm(receipt.clone()).map_err(|_| {
            SupportPrerequisiteArmContractError(
                "support authorization rejected its exact arming receipt",
            )
        })?;
        let instruction_projection = armed.armed_support_instruction_projection().ok_or(
            SupportPrerequisiteArmContractError(
                "armed authorization did not expose its instruction projection",
            ),
        )?;
        let required_external_action = ManualSupportInstruction::from_armed_projection(
            &instruction_projection,
        )
        .map_err(|_| {
            SupportPrerequisiteArmContractError(
                "armed authorization could not reconstruct its manual instruction",
            )
        })?;
        repeated.status_cas.commit_armed(&armed)?;
        Ok(Self {
            data: SupportPrerequisiteArmData {
                mode: SupportPrerequisiteArmMode::Value,
                stage: ApplyStage::Value,
                support_action_id: repeated.record.support_action_id,
                support_action_digest: repeated.record.support_action_digest,
                arming_receipt: receipt,
                required_external_action,
                arming_digest: repeated.arming_digest,
            },
            armed,
        })
    }

    pub(crate) fn into_parts(
        self,
    ) -> (SupportPrerequisiteArmData, ActiveSupportActionResumeHandle) {
        (self.data, self.armed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::branched_development::canonical_json::{
        canonical_contract_digest, contract_digest_record_sealed, ContractDigestRecord,
    };
    use crate::domain::branched_development::contracts::repository::{
        EvidenceSourceIndex, EvidenceSourceIndexCandidate, EvidenceSourceRegistry,
        RepositoryContractError, RepositoryHistoryCursor, RepositoryHistoryEvidenceBytesResolver,
        RepositoryHistoryOrderEvidence, RepositoryHistoryOrderResolver,
        RepositoryHistoryPartitionResolver, RepositoryHistorySourceEvidenceRef,
        RepositoryOwnerIdentity, UnvalidatedRepositoryHistoryPartition,
        ValidatedRepositoryHistoryPartition,
    };
    use crate::domain::branched_development::contracts::requests::repository::RepositoryUpdateRequest;
    use crate::domain::branched_development::contracts::scalars::{
        OriginalProjectCwd, RepositoryUsername, RequiredNullable,
    };
    use crate::domain::branched_development::contracts::schema::audit_json_schema;
    use crate::domain::branched_development::contracts::support::{
        active_support_action_resume_handle_fixture_test_only,
        support_update_authorization_projection_fixture_test_only, ManualSupportTargetMode,
        RootReachableSupportLayerSet, SupportRecoveryDistributionCoverageAuthority,
        SupportRecoveryDistributionSet, SupportRootLockObservation,
        SupportUpdateAuthorizationProjection,
    };
    use crate::domain::branched_development::{OperationId, Sha256Digest, UnicaId};
    use schemars::{schema_for, JsonSchema};
    use serde::de::DeserializeOwned;
    use serde::Serialize;
    use serde_json::{json, Value};

    const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const C: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
    const RECEIPT_ID: &str = "33333333-3333-4333-8333-333333333333";
    const OPERATION_ID: &str = "44444444-4444-4444-8444-444444444444";

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
            panic!("empty partition must not query the source index")
        }
    }

    struct UnexpectedOrder;

    impl RepositoryHistoryOrderResolver for UnexpectedOrder {
        fn order_evidence(
            &self,
            _from_exclusive: &RepositoryHistoryCursor,
            _through_inclusive: &RepositoryHistoryCursor,
        ) -> Result<RepositoryHistoryOrderEvidence, RepositoryContractError> {
            panic!("empty partition must not query history order")
        }
    }

    struct UnexpectedBytes;

    impl RepositoryHistoryEvidenceBytesResolver for UnexpectedBytes {
        fn load_canonical_evidence_bytes(
            &self,
            _reference: &RepositoryHistorySourceEvidenceRef,
        ) -> Result<Vec<u8>, RepositoryContractError> {
            panic!("empty partition must not load evidence bytes")
        }
    }

    fn digest(value: &str) -> Sha256Digest {
        Sha256Digest::parse(value).unwrap()
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

    fn foreign_cursor() -> RepositoryHistoryCursor {
        serde_json::from_value(json!({
            "throughVersion": "foreign",
            "historyPrefixDigest": B,
        }))
        .unwrap()
    }

    fn coverage_for(
        authority: &SupportUpdateAuthorizationProjection,
        graph_digest: Sha256Digest,
    ) -> SupportRecoveryDistributionCoverageAuthority {
        let distributions = SupportRecoveryDistributionSet::new(
            authority.support_recovery_distributions().to_vec(),
        )
        .unwrap();
        let layer_ids = distributions
            .as_slice()
            .iter()
            .map(|distribution| distribution.layer_id().clone())
            .collect();
        let reachable =
            RootReachableSupportLayerSet::from_capability_adapter(layer_ids, graph_digest).unwrap();
        SupportRecoveryDistributionCoverageAuthority::prove_complete(
            reachable,
            distributions,
            authority.authorized_transitions(),
        )
        .unwrap()
    }

    fn owner_for(
        authority: &SupportUpdateAuthorizationProjection,
        username: &RepositoryUsername,
    ) -> RepositoryOwnerIdentity {
        match authority.manual_working_infobase_identity() {
            Some(identity) => serde_json::from_value(json!({
                "username": username,
                "computer": identity.computer(),
                "infobase": identity.infobase(),
                "lockedAt": null,
            }))
            .unwrap(),
            None => serde_json::from_value(json!({
                "username": username,
                "computer": null,
                "infobase": null,
                "lockedAt": null,
            }))
            .unwrap(),
        }
    }

    fn target_identity_digest(authority: &SupportUpdateAuthorizationProjection) -> Sha256Digest {
        authority
            .manual_working_infobase_identity()
            .map(|identity| identity.digest().clone())
            .unwrap_or_else(|| authority.reserved_original_identity_digest().clone())
    }

    struct StaticBoundRootLockAuthority {
        manual_target_mode: ManualSupportTargetMode,
        target_identity_digest: Sha256Digest,
        root_lock_observation: SupportRootLockObservation,
        exact_scope_binding_proven: bool,
    }

    impl SupportArmBoundRootLockCapabilityAuthority for StaticBoundRootLockAuthority {
        fn manual_target_mode(&self) -> ManualSupportTargetMode {
            self.manual_target_mode
        }

        fn target_identity_digest(&self) -> &Sha256Digest {
            &self.target_identity_digest
        }

        fn root_lock_observation(&self) -> &SupportRootLockObservation {
            &self.root_lock_observation
        }

        fn proves_exact_scope_binding(&self, _scope: &SupportArmObservationScope<'_>) -> bool {
            self.exact_scope_binding_proven
        }

        fn into_root_lock_observation(self: Box<Self>) -> SupportRootLockObservation {
            self.root_lock_observation
        }
    }

    struct SplitBoundRootLockAuthority {
        manual_target_mode: ManualSupportTargetMode,
        target_identity_digest: Sha256Digest,
        inspected_root_lock_observation: SupportRootLockObservation,
        consumed_root_lock_observation: SupportRootLockObservation,
    }

    impl SupportArmBoundRootLockCapabilityAuthority for SplitBoundRootLockAuthority {
        fn manual_target_mode(&self) -> ManualSupportTargetMode {
            self.manual_target_mode
        }

        fn target_identity_digest(&self) -> &Sha256Digest {
            &self.target_identity_digest
        }

        fn root_lock_observation(&self) -> &SupportRootLockObservation {
            &self.inspected_root_lock_observation
        }

        fn proves_exact_scope_binding(&self, _scope: &SupportArmObservationScope<'_>) -> bool {
            true
        }

        fn into_root_lock_observation(self: Box<Self>) -> SupportRootLockObservation {
            self.consumed_root_lock_observation
        }
    }

    fn bound_root_lock(
        authority: &SupportUpdateAuthorizationProjection,
        root_lock_observation: SupportRootLockObservation,
        target_identity_override: Option<Sha256Digest>,
        exact_scope_binding_proven: bool,
    ) -> Box<dyn SupportArmBoundRootLockCapabilityAuthority> {
        Box::new(StaticBoundRootLockAuthority {
            manual_target_mode: authority.manual_target_mode(),
            target_identity_digest: target_identity_override
                .unwrap_or_else(|| target_identity_digest(authority)),
            root_lock_observation,
            exact_scope_binding_proven,
        })
    }

    fn snapshot_for(
        authority: &SupportUpdateAuthorizationProjection,
    ) -> SupportArmCapabilitySnapshot {
        let root_lock_observation = SupportRootLockObservation::new(RequiredNullable::value(
            owner_for(authority, authority.manual_actor_username()),
        ))
        .unwrap();
        SupportArmCapabilitySnapshot::from_capability_adapter(
            authority.support_gate_digest().clone(),
            authority.candidate_set_digest().clone(),
            authority.expected_relevant_baseline_digest().clone(),
            coverage_for(authority, authority.expected_support_graph_digest().clone()),
            authority.expected_original_fingerprint().clone(),
            bound_root_lock(authority, root_lock_observation, None, true),
        )
    }

    struct StaticResolver(Option<SupportArmCapabilitySnapshot>);

    impl SupportArmCapabilityResolver for StaticResolver {
        fn observe(
            &mut self,
            _scope: &SupportArmObservationScope<'_>,
            _history_partition: &ValidatedRepositoryHistoryPartition,
        ) -> Result<SupportArmCapabilitySnapshot, SupportPrerequisiteArmContractError> {
            Ok(self.0.take().expect("resolver is single-use"))
        }
    }

    struct StaticReceiptIssuer(Option<UnicaId>);

    impl SupportArmReceiptIdIssuer for StaticReceiptIssuer {
        fn issue(
            &mut self,
            _support_action_id: &UnicaId,
            _operation_id: &OperationId,
        ) -> Result<UnicaId, SupportPrerequisiteArmContractError> {
            Ok(self.0.take().expect("receipt issuer is single-use"))
        }
    }

    struct StaticStatusCasResolver {
        acquired: bool,
    }

    struct StaticStatusCasLease {
        cwd: OriginalProjectCwd,
        task_id: crate::domain::branched_development::TaskId,
        operation_id: OperationId,
        expected_status_digest: Sha256Digest,
        support_action_id: UnicaId,
        support_action_digest: Sha256Digest,
    }

    impl SupportArmStatusCasResolver for StaticStatusCasResolver {
        fn acquire(
            &mut self,
            request: &crate::domain::branched_development::contracts::requests::repository::ValidatedArmApplyRequest<'_>,
        ) -> Result<Box<dyn SupportArmStatusCasLease>, SupportPrerequisiteArmContractError>
        {
            if self.acquired {
                return Err(SupportPrerequisiteArmContractError::adapter_failure(
                    "status lineage already has an active CAS lease",
                ));
            }
            self.acquired = true;
            Ok(Box::new(StaticStatusCasLease {
                cwd: request.cwd().clone(),
                task_id: request.task_id().clone(),
                operation_id: request.operation_id().clone(),
                expected_status_digest: request.expected_status_digest().clone(),
                support_action_id: request.support_action_id().clone(),
                support_action_digest: request.expected_support_action_digest().clone(),
            }))
        }
    }

    impl SupportArmStatusCasLease for StaticStatusCasLease {
        fn binds(
            &self,
            request: &crate::domain::branched_development::contracts::requests::repository::ValidatedArmApplyRequest<'_>,
        ) -> bool {
            &self.cwd == request.cwd()
                && &self.task_id == request.task_id()
                && &self.operation_id == request.operation_id()
                && &self.expected_status_digest == request.expected_status_digest()
                && &self.support_action_id == request.support_action_id()
                && &self.support_action_digest == request.expected_support_action_digest()
        }

        fn commit_armed(
            self: Box<Self>,
            armed: &crate::domain::branched_development::contracts::support::ActiveSupportActionResumeHandle,
        ) -> Result<(), SupportPrerequisiteArmContractError> {
            if armed.armed_support_instruction_projection().is_none() {
                return Err(SupportPrerequisiteArmContractError::adapter_failure(
                    "CAS commit did not receive an armed authorization",
                ));
            }
            Ok(())
        }
    }

    fn validated_observation(
        authority: &SupportUpdateAuthorizationProjection,
    ) -> ValidatedArmObservationAuthority {
        let partition = empty_partition(authority.expected_before_history_cursor());
        let mut resolver = StaticResolver(Some(snapshot_for(authority)));
        ValidatedArmObservationAuthority::from_capability_resolver(
            authority,
            partition,
            &mut resolver,
        )
        .unwrap()
    }

    fn preview_request(
        authority: &SupportUpdateAuthorizationProjection,
    ) -> RepositoryUpdateRequest {
        serde_json::from_value(json!({
            "cwd": "/original/project",
            "taskId": "TASK-173",
            "mode": "supportPrerequisiteArm",
            "stage": "preview",
            "expectedStatusDigest": C,
            "supportActionId": authority.support_action_id(),
            "expectedSupportActionDigest": authority.support_action_digest(),
        }))
        .unwrap()
    }

    fn apply_request(
        authority: &SupportUpdateAuthorizationProjection,
        approved_arming_digest: &Sha256Digest,
    ) -> RepositoryUpdateRequest {
        serde_json::from_value(json!({
            "cwd": "/original/project",
            "taskId": "TASK-173",
            "operationId": OPERATION_ID,
            "mode": "supportPrerequisiteArm",
            "stage": "apply",
            "expectedStatusDigest": C,
            "supportActionId": authority.support_action_id(),
            "expectedSupportActionDigest": authority.support_action_digest(),
            "approvedArmingDigest": approved_arming_digest,
        }))
        .unwrap()
    }

    fn preview_digest(mode: ManualSupportTargetMode) -> Sha256Digest {
        let authority = support_update_authorization_projection_fixture_test_only(false, mode);
        let observation = validated_observation(&authority);
        let request = preview_request(&authority);
        let request = request.validate_arm_preview_context().unwrap();
        SupportPrerequisiteArmPreviewAuthority::from_authorities(request, authority, observation)
            .unwrap()
            .arming_digest()
            .clone()
    }

    fn schema_accepts<T: JsonSchema>(value: &Value) -> bool {
        let schema = serde_json::to_value(schema_for!(T)).unwrap();
        jsonschema::options()
            .with_draft(jsonschema::Draft::Draft202012)
            .build(&schema)
            .unwrap()
            .is_valid(value)
    }

    macro_rules! assert_not_deserialize_owned {
        ($type:ty) => {
            const _: fn() = || {
                trait AmbiguousIfDeserialize<Marker> {
                    fn assert_not_deserialize() {}
                }
                struct ImplementsDeserialize;
                impl<T: ?Sized> AmbiguousIfDeserialize<()> for T {}
                impl<T: ?Sized + DeserializeOwned> AmbiguousIfDeserialize<ImplementsDeserialize>
                    for T
                {
                }
                let _ = <$type as AmbiguousIfDeserialize<_>>::assert_not_deserialize;
            };
        };
    }

    macro_rules! assert_not_clone {
        ($type:ty) => {
            const _: fn() = || {
                trait AmbiguousIfClone<Marker> {
                    fn assert_not_clone() {}
                }
                struct ImplementsClone;
                impl<T: ?Sized> AmbiguousIfClone<()> for T {}
                impl<T: ?Sized + Clone> AmbiguousIfClone<ImplementsClone> for T {}
                let _ = <$type as AmbiguousIfClone<_>>::assert_not_clone;
            };
        };
    }

    assert_not_deserialize_owned!(SupportPrerequisiteArmPreviewData);
    assert_not_deserialize_owned!(SupportPrerequisiteArmData);
    assert_not_deserialize_owned!(SupportArmCapabilitySnapshot);
    assert_not_deserialize_owned!(ValidatedArmObservationAuthority);
    assert_not_deserialize_owned!(SupportPrerequisiteArmPreviewAuthority);
    assert_not_deserialize_owned!(ValidatedRepeatedArmPreviewAuthority);
    assert_not_deserialize_owned!(SupportPrerequisiteArmCommitAuthority);
    assert_not_deserialize_owned!(ValidatedArmStatusCasAuthority);
    assert_not_clone!(ValidatedArmObservationAuthority);
    assert_not_clone!(SupportArmCapabilitySnapshot);
    assert_not_clone!(SupportPrerequisiteArmPreviewAuthority);
    assert_not_clone!(ValidatedRepeatedArmPreviewAuthority);
    assert_not_clone!(SupportPrerequisiteArmCommitAuthority);
    assert_not_clone!(ValidatedArmStatusCasAuthority);
    const _: for<'a> fn(
        crate::domain::branched_development::contracts::requests::repository::ValidatedArmApplyRequest<'a>,
        ValidatedRepeatedArmPreviewAuthority,
        crate::domain::branched_development::contracts::support::ActiveSupportActionResumeHandle,
    ) -> Result<SupportPrerequisiteArmCommitAuthority, SupportPrerequisiteArmContractError> =
        SupportPrerequisiteArmCommitAuthority::apply;

    #[test]
    fn arm_preview_is_closed_physical_schema_and_digest_covers_the_whole_record() {
        let authorization = support_update_authorization_projection_fixture_test_only(
            false,
            ManualSupportTargetMode::ReservedOriginal,
        );
        let observation = validated_observation(&authorization);
        let request = preview_request(&authorization);
        let request = request.validate_arm_preview_context().unwrap();
        let preview = SupportPrerequisiteArmPreviewAuthority::from_authorities(
            request,
            authorization,
            observation,
        )
        .unwrap();
        assert_eq!(
            preview.arming_digest(),
            &canonical_contract_digest(&preview.record, None).unwrap()
        );
        let data = SupportPrerequisiteArmPreviewData::from_authority(preview);
        let value = serde_json::to_value(&data).unwrap();
        assert_eq!(value["mode"], json!("supportPrerequisiteArm"));
        assert_eq!(value["stage"], json!("preview"));
        assert!(schema_accepts::<SupportPrerequisiteArmPreviewData>(&value));
        audit_json_schema(
            &serde_json::to_value(schema_for!(SupportPrerequisiteArmPreviewData)).unwrap(),
        )
        .unwrap();

        let mut wrong_stage = value.clone();
        wrong_stage["stage"] = json!("apply");
        assert!(!schema_accepts::<SupportPrerequisiteArmPreviewData>(
            &wrong_stage
        ));
        let mut extra = value;
        extra["requiredExternalAction"] = json!({});
        assert!(!schema_accepts::<SupportPrerequisiteArmPreviewData>(&extra));
    }

    #[test]
    fn arm_observation_rejects_history_gate_baseline_original_and_target_splices() {
        fn rejects_with(
            authorization: &SupportUpdateAuthorizationProjection,
            partition: ValidatedRepositoryHistoryPartition,
            snapshot: SupportArmCapabilitySnapshot,
        ) {
            let mut resolver = StaticResolver(Some(snapshot));
            assert!(ValidatedArmObservationAuthority::from_capability_resolver(
                authorization,
                partition,
                &mut resolver,
            )
            .is_err());
        }

        let authorization = support_update_authorization_projection_fixture_test_only(
            false,
            ManualSupportTargetMode::ReservedOriginal,
        );
        rejects_with(
            &authorization,
            empty_partition(&foreign_cursor()),
            snapshot_for(&authorization),
        );

        let root = SupportRootLockObservation::new(RequiredNullable::value(owner_for(
            &authorization,
            authorization.manual_actor_username(),
        )))
        .unwrap();
        let stable_partition = empty_partition(authorization.expected_before_history_cursor());
        for snapshot in [
            SupportArmCapabilitySnapshot::from_capability_adapter(
                digest(B),
                authorization.candidate_set_digest().clone(),
                authorization.expected_relevant_baseline_digest().clone(),
                coverage_for(
                    &authorization,
                    authorization.expected_support_graph_digest().clone(),
                ),
                authorization.expected_original_fingerprint().clone(),
                bound_root_lock(&authorization, root.clone(), None, true),
            ),
            SupportArmCapabilitySnapshot::from_capability_adapter(
                authorization.support_gate_digest().clone(),
                digest(B),
                authorization.expected_relevant_baseline_digest().clone(),
                coverage_for(
                    &authorization,
                    authorization.expected_support_graph_digest().clone(),
                ),
                authorization.expected_original_fingerprint().clone(),
                bound_root_lock(&authorization, root.clone(), None, true),
            ),
            SupportArmCapabilitySnapshot::from_capability_adapter(
                authorization.support_gate_digest().clone(),
                authorization.candidate_set_digest().clone(),
                digest(B),
                coverage_for(
                    &authorization,
                    authorization.expected_support_graph_digest().clone(),
                ),
                authorization.expected_original_fingerprint().clone(),
                bound_root_lock(&authorization, root.clone(), None, true),
            ),
            SupportArmCapabilitySnapshot::from_capability_adapter(
                authorization.support_gate_digest().clone(),
                authorization.candidate_set_digest().clone(),
                authorization.expected_relevant_baseline_digest().clone(),
                coverage_for(
                    &authorization,
                    authorization.expected_support_graph_digest().clone(),
                ),
                digest(B),
                bound_root_lock(&authorization, root.clone(), None, true),
            ),
            SupportArmCapabilitySnapshot::from_capability_adapter(
                authorization.support_gate_digest().clone(),
                authorization.candidate_set_digest().clone(),
                authorization.expected_relevant_baseline_digest().clone(),
                coverage_for(
                    &authorization,
                    authorization.expected_support_graph_digest().clone(),
                ),
                authorization.expected_original_fingerprint().clone(),
                bound_root_lock(&authorization, root, Some(digest(B)), true),
            ),
        ] {
            rejects_with(&authorization, stable_partition.clone(), snapshot);
        }
    }

    #[test]
    fn arm_observation_rejects_graph_distribution_mode_and_root_owner_splices() {
        let reserved = support_update_authorization_projection_fixture_test_only(
            false,
            ManualSupportTargetMode::ReservedOriginal,
        );
        let wrong_graph = SupportArmCapabilitySnapshot::from_capability_adapter(
            reserved.support_gate_digest().clone(),
            reserved.candidate_set_digest().clone(),
            reserved.expected_relevant_baseline_digest().clone(),
            coverage_for(&reserved, digest(B)),
            reserved.expected_original_fingerprint().clone(),
            bound_root_lock(
                &reserved,
                SupportRootLockObservation::new(RequiredNullable::value(owner_for(
                    &reserved,
                    reserved.manual_actor_username(),
                )))
                .unwrap(),
                None,
                true,
            ),
        );
        let mut resolver = StaticResolver(Some(wrong_graph));
        assert!(ValidatedArmObservationAuthority::from_capability_resolver(
            &reserved,
            empty_partition(reserved.expected_before_history_cursor()),
            &mut resolver,
        )
        .is_err());

        let foreign_username = RepositoryUsername::parse("foreign-user").unwrap();
        let wrong_owner = SupportArmCapabilitySnapshot::from_capability_adapter(
            reserved.support_gate_digest().clone(),
            reserved.candidate_set_digest().clone(),
            reserved.expected_relevant_baseline_digest().clone(),
            coverage_for(&reserved, reserved.expected_support_graph_digest().clone()),
            reserved.expected_original_fingerprint().clone(),
            bound_root_lock(
                &reserved,
                SupportRootLockObservation::new(RequiredNullable::value(owner_for(
                    &reserved,
                    &foreign_username,
                )))
                .unwrap(),
                None,
                true,
            ),
        );
        let mut resolver = StaticResolver(Some(wrong_owner));
        assert!(ValidatedArmObservationAuthority::from_capability_resolver(
            &reserved,
            empty_partition(reserved.expected_before_history_cursor()),
            &mut resolver,
        )
        .is_err());

        let foreign_reserved_target_owner: RepositoryOwnerIdentity =
            serde_json::from_value(json!({
                "username": reserved.manual_actor_username(),
                "computer": "FOREIGN-HOST",
                "infobase": "Foreign reserved IB",
                "lockedAt": null,
            }))
            .unwrap();
        let foreign_reserved_target = SupportArmCapabilitySnapshot::from_capability_adapter(
            reserved.support_gate_digest().clone(),
            reserved.candidate_set_digest().clone(),
            reserved.expected_relevant_baseline_digest().clone(),
            coverage_for(&reserved, reserved.expected_support_graph_digest().clone()),
            reserved.expected_original_fingerprint().clone(),
            bound_root_lock(
                &reserved,
                SupportRootLockObservation::new(RequiredNullable::value(
                    foreign_reserved_target_owner,
                ))
                .unwrap(),
                None,
                false,
            ),
        );
        let mut resolver = StaticResolver(Some(foreign_reserved_target));
        assert!(ValidatedArmObservationAuthority::from_capability_resolver(
            &reserved,
            empty_partition(reserved.expected_before_history_cursor()),
            &mut resolver,
        )
        .is_err());

        let separate = support_update_authorization_projection_fixture_test_only(
            false,
            ManualSupportTargetMode::SeparateWorkingInfobase,
        );
        let reserved_distributions =
            coverage_for(&reserved, separate.expected_support_graph_digest().clone());
        let cross_mode = SupportArmCapabilitySnapshot::from_capability_adapter(
            separate.support_gate_digest().clone(),
            separate.candidate_set_digest().clone(),
            separate.expected_relevant_baseline_digest().clone(),
            reserved_distributions,
            separate.expected_original_fingerprint().clone(),
            bound_root_lock(
                &separate,
                SupportRootLockObservation::new(RequiredNullable::value(owner_for(
                    &separate,
                    separate.manual_actor_username(),
                )))
                .unwrap(),
                None,
                true,
            ),
        );
        let mut resolver = StaticResolver(Some(cross_mode));
        assert!(ValidatedArmObservationAuthority::from_capability_resolver(
            &separate,
            empty_partition(separate.expected_before_history_cursor()),
            &mut resolver,
        )
        .is_err());
    }

    #[test]
    fn arm_observation_rejects_root_owner_changed_during_capability_consumption() {
        let authorization = support_update_authorization_projection_fixture_test_only(
            false,
            ManualSupportTargetMode::ReservedOriginal,
        );
        let expected = SupportRootLockObservation::new(RequiredNullable::value(owner_for(
            &authorization,
            authorization.manual_actor_username(),
        )))
        .unwrap();
        let foreign_username = RepositoryUsername::parse("foreign-user").unwrap();
        let substituted = SupportRootLockObservation::new(RequiredNullable::value(owner_for(
            &authorization,
            &foreign_username,
        )))
        .unwrap();
        let snapshot = SupportArmCapabilitySnapshot::from_capability_adapter(
            authorization.support_gate_digest().clone(),
            authorization.candidate_set_digest().clone(),
            authorization.expected_relevant_baseline_digest().clone(),
            coverage_for(
                &authorization,
                authorization.expected_support_graph_digest().clone(),
            ),
            authorization.expected_original_fingerprint().clone(),
            Box::new(SplitBoundRootLockAuthority {
                manual_target_mode: authorization.manual_target_mode(),
                target_identity_digest: target_identity_digest(&authorization),
                inspected_root_lock_observation: expected,
                consumed_root_lock_observation: substituted,
            }),
        );
        let mut resolver = StaticResolver(Some(snapshot));

        assert!(ValidatedArmObservationAuthority::from_capability_resolver(
            &authorization,
            empty_partition(authorization.expected_before_history_cursor()),
            &mut resolver,
        )
        .is_err());
    }

    #[test]
    fn arm_preview_rejects_cross_action_request_and_already_armed_authorization() {
        let authorization = support_update_authorization_projection_fixture_test_only(
            false,
            ManualSupportTargetMode::ReservedOriginal,
        );
        let observation = validated_observation(&authorization);
        let mut request = serde_json::to_value(preview_request(&authorization)).unwrap();
        request["supportActionId"] = json!(RECEIPT_ID);
        let request: RepositoryUpdateRequest = serde_json::from_value(request).unwrap();
        assert!(SupportPrerequisiteArmPreviewAuthority::from_authorities(
            request.validate_arm_preview_context().unwrap(),
            authorization,
            observation,
        )
        .is_err());

        let armed = support_update_authorization_projection_fixture_test_only(
            true,
            ManualSupportTargetMode::ReservedOriginal,
        );
        let partition = empty_partition(armed.expected_before_history_cursor());
        let mut resolver = StaticResolver(Some(snapshot_for(&armed)));
        assert!(ValidatedArmObservationAuthority::from_capability_resolver(
            &armed,
            partition,
            &mut resolver,
        )
        .is_err());
    }

    #[test]
    fn arm_apply_consumes_exact_approval_and_derives_receipt_instruction_and_armed_handle() {
        let approved_digest = preview_digest(ManualSupportTargetMode::ReservedOriginal);
        let active = active_support_action_resume_handle_fixture_test_only(
            ManualSupportTargetMode::ReservedOriginal,
        );
        let authorization = active.support_update_authorization_projection().unwrap();
        let observation = validated_observation(&authorization);
        let request = apply_request(&authorization, &approved_digest);
        let validated = request.validate_arm_approval(&approved_digest).unwrap();
        let mut issuer = StaticReceiptIssuer(Some(UnicaId::parse(RECEIPT_ID).unwrap()));
        let mut status_cas = StaticStatusCasResolver { acquired: false };
        let repeated = ValidatedRepeatedArmPreviewAuthority::from_authorities(
            &validated,
            authorization,
            observation,
            &mut status_cas,
            &mut issuer,
        )
        .unwrap();
        let committed =
            SupportPrerequisiteArmCommitAuthority::apply(validated, repeated, active).unwrap();
        let (data, armed) = committed.into_parts();
        let value = serde_json::to_value(&data).unwrap();
        assert_eq!(value["mode"], json!("supportPrerequisiteArm"));
        assert_eq!(value["stage"], json!("apply"));
        assert_eq!(value["armingDigest"], json!(approved_digest));
        assert_eq!(value["armingReceipt"]["armingReceiptId"], json!(RECEIPT_ID));
        assert!(schema_accepts::<SupportPrerequisiteArmData>(&value));
        audit_json_schema(&serde_json::to_value(schema_for!(SupportPrerequisiteArmData)).unwrap())
            .unwrap();

        let armed_projection = armed.armed_support_instruction_projection().unwrap();
        let expected_instruction =
            crate::domain::branched_development::contracts::instructions::ManualSupportInstruction::from_armed_projection(
                &armed_projection,
            )
            .unwrap();
        assert_eq!(
            value["requiredExternalAction"],
            serde_json::to_value(expected_instruction).unwrap()
        );

        let mut wrong_stage = value.clone();
        wrong_stage["stage"] = json!("preview");
        assert!(!schema_accepts::<SupportPrerequisiteArmData>(&wrong_stage));
        let mut splice = value;
        splice["requiredExternalAction"]["repositoryUsername"] = json!("foreign-user");
        assert_ne!(
            splice["requiredExternalAction"],
            serde_json::to_value(
                crate::domain::branched_development::contracts::instructions::ManualSupportInstruction::from_armed_projection(
                    &armed_projection,
                )
                .unwrap()
            )
            .unwrap()
        );
    }

    #[test]
    fn separate_working_infobase_apply_reconstructs_its_exact_target_instruction() {
        let approved_digest = preview_digest(ManualSupportTargetMode::SeparateWorkingInfobase);
        let active = active_support_action_resume_handle_fixture_test_only(
            ManualSupportTargetMode::SeparateWorkingInfobase,
        );
        let authorization = active.support_update_authorization_projection().unwrap();
        let expected_identity = authorization
            .manual_working_infobase_identity()
            .expect("separate mode must retain its working-IB identity")
            .clone();
        let observation = validated_observation(&authorization);
        let request = apply_request(&authorization, &approved_digest);
        let validated = request.validate_arm_approval(&approved_digest).unwrap();
        let mut issuer = StaticReceiptIssuer(Some(UnicaId::parse(RECEIPT_ID).unwrap()));
        let mut status_cas = StaticStatusCasResolver { acquired: false };
        let repeated = ValidatedRepeatedArmPreviewAuthority::from_authorities(
            &validated,
            authorization,
            observation,
            &mut status_cas,
            &mut issuer,
        )
        .unwrap();
        let committed =
            SupportPrerequisiteArmCommitAuthority::apply(validated, repeated, active).unwrap();
        let (data, armed) = committed.into_parts();
        let value = serde_json::to_value(data).unwrap();
        assert_eq!(
            value["requiredExternalAction"]["manualTargetMode"],
            json!("separateWorkingInfobase")
        );
        assert_eq!(
            value["requiredExternalAction"]["workingInfobaseIdentity"],
            serde_json::to_value(expected_identity).unwrap()
        );
        assert!(armed.armed_support_instruction_projection().is_some());
    }

    #[test]
    fn arm_apply_rejects_wrong_approval_cross_action_and_armed_or_frozen_input() {
        let approved_digest = preview_digest(ManualSupportTargetMode::ReservedOriginal);
        let active = active_support_action_resume_handle_fixture_test_only(
            ManualSupportTargetMode::ReservedOriginal,
        );
        let authorization = active.support_update_authorization_projection().unwrap();
        let wrong_approval = apply_request(&authorization, &digest(B));
        assert!(wrong_approval
            .validate_arm_approval(&approved_digest)
            .is_err());

        let mut wrong_action_json =
            serde_json::to_value(apply_request(&authorization, &approved_digest)).unwrap();
        wrong_action_json["supportActionId"] = json!(RECEIPT_ID);
        let wrong_action: RepositoryUpdateRequest =
            serde_json::from_value(wrong_action_json).unwrap();
        let wrong_action = wrong_action
            .validate_arm_approval(&approved_digest)
            .unwrap();
        let observation = validated_observation(&authorization);
        let mut issuer = StaticReceiptIssuer(Some(UnicaId::parse(RECEIPT_ID).unwrap()));
        let mut status_cas = StaticStatusCasResolver { acquired: false };
        assert!(ValidatedRepeatedArmPreviewAuthority::from_authorities(
            &wrong_action,
            authorization,
            observation,
            &mut status_cas,
            &mut issuer,
        )
        .is_err());

        let active = active_support_action_resume_handle_fixture_test_only(
            ManualSupportTargetMode::ReservedOriginal,
        );
        let authorization = active.support_update_authorization_projection().unwrap();
        let observation = validated_observation(&authorization);
        let request = apply_request(&authorization, &approved_digest);
        let validated = request.validate_arm_approval(&approved_digest).unwrap();
        let mut issuer = StaticReceiptIssuer(Some(UnicaId::parse(RECEIPT_ID).unwrap()));
        let mut status_cas = StaticStatusCasResolver { acquired: false };
        let repeated = ValidatedRepeatedArmPreviewAuthority::from_authorities(
            &validated,
            authorization,
            observation,
            &mut status_cas,
            &mut issuer,
        )
        .unwrap();
        let first =
            SupportPrerequisiteArmCommitAuthority::apply(validated, repeated, active).unwrap();
        let (_, armed) = first.into_parts();

        for invalid in [armed.clone(), armed.freeze_armed_action().unwrap()] {
            let fresh = active_support_action_resume_handle_fixture_test_only(
                ManualSupportTargetMode::ReservedOriginal,
            );
            let authorization = fresh.support_update_authorization_projection().unwrap();
            let observation = validated_observation(&authorization);
            let request = apply_request(&authorization, &approved_digest);
            let validated = request.validate_arm_approval(&approved_digest).unwrap();
            let mut issuer = StaticReceiptIssuer(Some(UnicaId::parse(RECEIPT_ID).unwrap()));
            let mut status_cas = StaticStatusCasResolver { acquired: false };
            let repeated = ValidatedRepeatedArmPreviewAuthority::from_authorities(
                &validated,
                authorization,
                observation,
                &mut status_cas,
                &mut issuer,
            )
            .unwrap();
            assert!(
                SupportPrerequisiteArmCommitAuthority::apply(validated, repeated, invalid).is_err()
            );
        }
    }

    #[test]
    fn one_status_lineage_cannot_mint_two_apply_cas_authorities() {
        let approved_digest = preview_digest(ManualSupportTargetMode::ReservedOriginal);
        let request_owner = active_support_action_resume_handle_fixture_test_only(
            ManualSupportTargetMode::ReservedOriginal,
        );
        let request_authorization = request_owner
            .support_update_authorization_projection()
            .unwrap();
        let request = apply_request(&request_authorization, &approved_digest);
        let validated = request.validate_arm_approval(&approved_digest).unwrap();
        let mut status_cas = StaticStatusCasResolver { acquired: false };

        let observation = validated_observation(&request_authorization);
        let mut first_issuer = StaticReceiptIssuer(Some(UnicaId::parse(RECEIPT_ID).unwrap()));
        let _first = ValidatedRepeatedArmPreviewAuthority::from_authorities(
            &validated,
            request_authorization,
            observation,
            &mut status_cas,
            &mut first_issuer,
        )
        .unwrap();

        let duplicate_owner = active_support_action_resume_handle_fixture_test_only(
            ManualSupportTargetMode::ReservedOriginal,
        );
        let duplicate_authorization = duplicate_owner
            .support_update_authorization_projection()
            .unwrap();
        let duplicate_observation = validated_observation(&duplicate_authorization);
        let mut duplicate_issuer = StaticReceiptIssuer(Some(UnicaId::parse(RECEIPT_ID).unwrap()));
        assert!(ValidatedRepeatedArmPreviewAuthority::from_authorities(
            &validated,
            duplicate_authorization,
            duplicate_observation,
            &mut status_cas,
            &mut duplicate_issuer,
        )
        .is_err());
    }

    #[test]
    fn both_manual_target_modes_produce_stable_arming_digest_without_instruction_in_preview() {
        for mode in [
            ManualSupportTargetMode::ReservedOriginal,
            ManualSupportTargetMode::SeparateWorkingInfobase,
        ] {
            let authorization =
                support_update_authorization_projection_fixture_test_only(false, mode);
            let expected_actor = authorization.manual_actor_username().clone();
            let observation = validated_observation(&authorization);
            let request = preview_request(&authorization);
            let preview = SupportPrerequisiteArmPreviewAuthority::from_authorities(
                request.validate_arm_preview_context().unwrap(),
                authorization,
                observation,
            )
            .unwrap();
            let first_digest = preview.arming_digest().clone();
            let value =
                serde_json::to_value(SupportPrerequisiteArmPreviewData::from_authority(preview))
                    .unwrap();
            assert_eq!(value["expectedManualActorUsername"], json!(expected_actor));
            assert_eq!(value["armingDigest"], json!(first_digest));
            assert!(value.get("requiredExternalAction").is_none());
            assert!(value.get("armingReceipt").is_none());
        }
    }

    #[test]
    fn preview_context_is_retained_inside_the_sealed_authority() {
        let authorization = support_update_authorization_projection_fixture_test_only(
            false,
            ManualSupportTargetMode::ReservedOriginal,
        );
        let observation = validated_observation(&authorization);
        let request = preview_request(&authorization);
        let preview = SupportPrerequisiteArmPreviewAuthority::from_authorities(
            request.validate_arm_preview_context().unwrap(),
            authorization,
            observation,
        )
        .unwrap();
        assert_eq!(
            preview.cwd(),
            &OriginalProjectCwd::parse("/original/project").unwrap()
        );
        assert_eq!(preview.expected_status_digest(), &digest(C));
    }
}
