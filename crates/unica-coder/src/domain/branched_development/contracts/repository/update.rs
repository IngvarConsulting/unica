use super::{
    AcquiredRepositoryUpdateLockTargets, ConfigurationRootKind, FalseLiteral, HasTargetKey,
    PresentState, ReleasedRepositoryUpdateLockTargets, RepositoryAnchor, RepositoryContractError,
    RepositoryHistoryCursor, RepositoryTargetState, RepositoryTargetStates,
    RepositoryUpdateLockTargets, RootPresentTargetState, TargetKey, TrueLiteral,
    ValidatedSupportPrerequisiteHistoryProjection,
};
use crate::domain::branched_development::canonical_json::{
    canonical_contract_digest, contract_digest_record_sealed, ContractDigestRecord,
};
use crate::domain::branched_development::contracts::schema::one_of_schema;
use crate::domain::branched_development::contracts::support_recovery_authority::SupportRecoveryAuthorityToken;
use crate::domain::branched_development::{CapabilityRowId, Sha256Digest, UnicaId};
use schemars::{JsonSchema, Schema, SchemaGenerator};
use serde::Serialize;
use std::borrow::Cow;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) enum SelectiveRepositoryUpdateScope {
    RoutinePlannedObjects,
    SupportRoot,
    RecoveryFinalization,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
enum StructuralSelectiveRepositoryUpdateScope {
    RoutinePlannedObjects,
    RecoveryFinalization,
}

impl TryFrom<SelectiveRepositoryUpdateScope> for StructuralSelectiveRepositoryUpdateScope {
    type Error = RepositoryContractError;

    fn try_from(scope: SelectiveRepositoryUpdateScope) -> Result<Self, Self::Error> {
        match scope {
            SelectiveRepositoryUpdateScope::RoutinePlannedObjects => {
                Ok(Self::RoutinePlannedObjects)
            }
            SelectiveRepositoryUpdateScope::RecoveryFinalization => Ok(Self::RecoveryFinalization),
            SelectiveRepositoryUpdateScope::SupportRoot => Err(RepositoryContractError(
                "support-root update cannot require structural confirmation",
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(transparent)]
struct RepositoryTargetRevisionMapDigestRecord(RepositoryTargetStates);

impl contract_digest_record_sealed::Sealed for RepositoryTargetRevisionMapDigestRecord {}
impl ContractDigestRecord for RepositoryTargetRevisionMapDigestRecord {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SelectiveRepositoryUpdatePlanDigestRecord {
    scope: SelectiveRepositoryUpdateScope,
    planned_targets: RepositoryTargetStates,
    lock_targets: RepositoryUpdateLockTargets,
    expected_target_revision_map_digest: Sha256Digest,
    selective_objects_capability_id: CapabilityRowId,
    structural_confirmation_required: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    structural_capability_row_id: Option<CapabilityRowId>,
}

impl contract_digest_record_sealed::Sealed for SelectiveRepositoryUpdatePlanDigestRecord {}
impl ContractDigestRecord for SelectiveRepositoryUpdatePlanDigestRecord {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct NonStructuralSelectiveRepositoryUpdatePlanDigestSchema {
    scope: SelectiveRepositoryUpdateScope,
    planned_targets: RepositoryTargetStates,
    lock_targets: RepositoryUpdateLockTargets,
    expected_target_revision_map_digest: Sha256Digest,
    selective_objects_capability_id: CapabilityRowId,
    structural_confirmation_required: FalseLiteral,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StructuralSelectiveRepositoryUpdatePlanDigestSchema {
    scope: StructuralSelectiveRepositoryUpdateScope,
    planned_targets: RepositoryTargetStates,
    lock_targets: RepositoryUpdateLockTargets,
    expected_target_revision_map_digest: Sha256Digest,
    selective_objects_capability_id: CapabilityRowId,
    structural_confirmation_required: TrueLiteral,
    structural_capability_row_id: CapabilityRowId,
}

impl JsonSchema for SelectiveRepositoryUpdatePlanDigestRecord {
    fn schema_name() -> Cow<'static, str> {
        "SelectiveRepositoryUpdatePlanDigestRecord".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<NonStructuralSelectiveRepositoryUpdatePlanDigestSchema>(),
            generator.subschema_for::<StructuralSelectiveRepositoryUpdatePlanDigestSchema>(),
        ])
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct NonStructuralSelectiveRepositoryUpdatePlanSchema {
    scope: SelectiveRepositoryUpdateScope,
    planned_targets: RepositoryTargetStates,
    lock_targets: RepositoryUpdateLockTargets,
    expected_target_revision_map_digest: Sha256Digest,
    selective_objects_capability_id: CapabilityRowId,
    structural_confirmation_required: FalseLiteral,
    plan_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StructuralSelectiveRepositoryUpdatePlanSchema {
    scope: StructuralSelectiveRepositoryUpdateScope,
    planned_targets: RepositoryTargetStates,
    lock_targets: RepositoryUpdateLockTargets,
    expected_target_revision_map_digest: Sha256Digest,
    selective_objects_capability_id: CapabilityRowId,
    structural_confirmation_required: TrueLiteral,
    structural_capability_row_id: CapabilityRowId,
    plan_digest: Sha256Digest,
}

/// Capability-derived authority for one exact selective-update plan.
///
/// This type deliberately has neither `Deserialize` nor a crate-visible raw
/// constructor. The repository adapter must prove the exact planned/lock-set
/// coverage and structural decision before issuing it inside this module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SelectiveRepositoryUpdatePlanAuthority {
    scope: SelectiveRepositoryUpdateScope,
    planned_targets: RepositoryTargetStates,
    lock_targets: RepositoryUpdateLockTargets,
    selective_objects_capability_id: CapabilityRowId,
    structural_capability_row_id: Option<CapabilityRowId>,
}

/// Raw capability observation for a recovery-finalization selective plan.
/// It is intentionally not a plan authority: only the recovery token can turn
/// this typed observation into the opaque, digest-bound plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SupportRecoverySelectiveUpdatePlanObservation {
    planned_targets: RepositoryTargetStates,
    lock_targets: RepositoryUpdateLockTargets,
    selective_objects_capability_id: CapabilityRowId,
    structural_capability_row_id: Option<CapabilityRowId>,
    structural_closure_covered_targets: Vec<super::RepositoryTargetIdentity>,
}

/// One sealed adapter capability for a routine selective-update plan.
///
/// The token owns the exact target map together with the lock/capability
/// observation that covers it. It is deliberately non-`Clone` and non-wire;
/// a routine resolver may return the resulting authority but cannot return a
/// caller-assembled plan.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct RoutineSelectiveRepositoryUpdateCapabilityToken {
    plan_authority: SelectiveRepositoryUpdatePlanAuthority,
    structural_targets: Vec<super::RepositoryTargetIdentity>,
}

/// Sealed capability observation for one support-root plan. The target map is
/// derived here from the exact before anchor, authorized observation version,
/// and capability-observed after-root fingerprint; callers never provide a
/// `SelectiveRepositoryUpdatePlan` or target list.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct SupportRootSelectiveRepositoryUpdateCapabilityToken {
    plan_authority: SelectiveRepositoryUpdatePlanAuthority,
    before_anchor_digest: Sha256Digest,
    history_partition_digest: Sha256Digest,
    authorized_repository_version: super::super::scalars::RepositoryVersion,
    authorized_root_delta_digest: Sha256Digest,
    final_root_repository_version: super::super::scalars::RepositoryVersion,
    observed_after_root_fingerprint: Sha256Digest,
    update_required: bool,
}

impl SupportRootSelectiveRepositoryUpdateCapabilityToken {
    pub(crate) fn from_capability_adapter(
        before_anchor: &RepositoryAnchor,
        history: &ValidatedSupportPrerequisiteHistoryProjection,
        observed_before_root_fingerprint: Sha256Digest,
        observed_after_root_fingerprint: Sha256Digest,
        lock_targets: RepositoryUpdateLockTargets,
        selective_objects_capability_id: CapabilityRowId,
    ) -> Result<Self, RepositoryContractError> {
        let authorized_observation = history.authorized_observation();
        let authorized = authorized_observation
            .authorized_support_projection()
            .ok_or(RepositoryContractError(
                "support-root plan requires an authorized support observation",
            ))?;
        if &observed_before_root_fingerprint != before_anchor.configuration_fingerprint() {
            return Err(RepositoryContractError(
                "support-root capability observed another before-root fingerprint",
            ));
        }
        if lock_targets.as_slice().len() != 1
            || lock_targets.as_slice()[0].target_key() != TargetKey::Root
        {
            return Err(RepositoryContractError(
                "support-root capability must bind the exact root-only lock closure",
            ));
        }
        let update_required = observed_before_root_fingerprint != observed_after_root_fingerprint;
        let planned_targets = if update_required {
            RepositoryTargetStates(vec![RepositoryTargetState::RootPresent(
                RootPresentTargetState {
                    target_kind: ConfigurationRootKind::Value,
                    state: PresentState::Value,
                    repository_version: history.final_root_repository_version().clone(),
                    target_fingerprint: observed_after_root_fingerprint.clone(),
                },
            )])
        } else {
            RepositoryTargetStates(Vec::new())
        };
        let plan_authority = SelectiveRepositoryUpdatePlanAuthority::from_capability_adapter(
            SelectiveRepositoryUpdateScope::SupportRoot,
            planned_targets,
            lock_targets,
            selective_objects_capability_id,
            None,
            Vec::new(),
        )?;
        Ok(Self {
            plan_authority,
            before_anchor_digest: before_anchor.anchor_digest().clone(),
            history_partition_digest: history.partition().partition_digest().clone(),
            authorized_repository_version: authorized_observation.repository_version().clone(),
            authorized_root_delta_digest: authorized.root_delta_digest().clone(),
            final_root_repository_version: history.final_root_repository_version().clone(),
            observed_after_root_fingerprint,
            update_required,
        })
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct SupportRootSelectiveRepositoryUpdatePlanAuthority {
    token: SupportRootSelectiveRepositoryUpdateCapabilityToken,
}

impl SupportRootSelectiveRepositoryUpdatePlanAuthority {
    pub(crate) fn from_capability_token(
        token: SupportRootSelectiveRepositoryUpdateCapabilityToken,
    ) -> Result<Self, RepositoryContractError> {
        if token.plan_authority.scope != SelectiveRepositoryUpdateScope::SupportRoot
            || token.plan_authority.structural_capability_row_id.is_some()
        {
            return Err(RepositoryContractError(
                "support-root capability token has a foreign or structural plan",
            ));
        }
        Ok(Self { token })
    }
}

impl RoutineSelectiveRepositoryUpdateCapabilityToken {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_capability_adapter(
        planned_targets: RepositoryTargetStates,
        structural_targets: Vec<super::RepositoryTargetIdentity>,
        lock_targets: RepositoryUpdateLockTargets,
        selective_objects_capability_id: CapabilityRowId,
        structural_capability_row_id: Option<CapabilityRowId>,
        structural_closure_covered_targets: Vec<super::RepositoryTargetIdentity>,
    ) -> Result<Self, RepositoryContractError> {
        let has_structural_targets = !structural_targets.is_empty();
        if structural_targets.windows(2).any(|pair| pair[0] >= pair[1])
            || structural_targets.iter().any(|target| {
                !planned_targets
                    .as_slice()
                    .iter()
                    .any(|planned| planned.target_key() == target.target_key())
            })
            || structural_capability_row_id.is_some() != has_structural_targets
        {
            return Err(RepositoryContractError(
                "routine structural targets and capability evidence are not exact",
            ));
        }
        let structural_closure_covered_targets = structural_closure_covered_targets
            .iter()
            .map(HasTargetKey::target_key)
            .collect();
        let plan_authority = SelectiveRepositoryUpdatePlanAuthority::from_capability_adapter(
            SelectiveRepositoryUpdateScope::RoutinePlannedObjects,
            planned_targets,
            lock_targets,
            selective_objects_capability_id,
            structural_capability_row_id,
            structural_closure_covered_targets,
        )?;
        Ok(Self {
            plan_authority,
            structural_targets,
        })
    }
}

/// Resolver-returnable routine plan authority. The final factory rechecks its
/// target-state and structural bindings against the independently folded
/// routine history before producing a wire plan.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct RoutineSelectiveRepositoryUpdatePlanAuthority {
    plan_authority: SelectiveRepositoryUpdatePlanAuthority,
    structural_targets: Vec<super::RepositoryTargetIdentity>,
}

impl RoutineSelectiveRepositoryUpdatePlanAuthority {
    pub(crate) fn from_capability_token(
        token: RoutineSelectiveRepositoryUpdateCapabilityToken,
    ) -> Result<Self, RepositoryContractError> {
        if token.plan_authority.scope != SelectiveRepositoryUpdateScope::RoutinePlannedObjects {
            return Err(RepositoryContractError(
                "routine capability token has a foreign selective-update scope",
            ));
        }
        Ok(Self {
            plan_authority: token.plan_authority,
            structural_targets: token.structural_targets,
        })
    }
}

impl SupportRecoverySelectiveUpdatePlanObservation {
    pub(crate) fn from_capability_adapter(
        planned_targets: RepositoryTargetStates,
        lock_targets: RepositoryUpdateLockTargets,
        selective_objects_capability_id: CapabilityRowId,
        structural_capability_row_id: Option<CapabilityRowId>,
        structural_closure_covered_targets: Vec<super::RepositoryTargetIdentity>,
    ) -> Self {
        Self {
            planned_targets,
            lock_targets,
            selective_objects_capability_id,
            structural_capability_row_id,
            structural_closure_covered_targets,
        }
    }
}

impl SelectiveRepositoryUpdatePlanAuthority {
    fn from_capability_adapter(
        scope: SelectiveRepositoryUpdateScope,
        planned_targets: RepositoryTargetStates,
        lock_targets: RepositoryUpdateLockTargets,
        selective_objects_capability_id: CapabilityRowId,
        structural_capability_row_id: Option<CapabilityRowId>,
        structural_closure_covered_targets: Vec<TargetKey>,
    ) -> Result<Self, RepositoryContractError> {
        if scope == SelectiveRepositoryUpdateScope::SupportRoot
            && structural_capability_row_id.is_some()
        {
            return Err(RepositoryContractError(
                "support-root update cannot require structural confirmation",
            ));
        }

        let lock_keys: Vec<_> = lock_targets
            .0
            .iter()
            .map(HasTargetKey::target_key)
            .collect();
        let planned_keys: Vec<_> = planned_targets
            .0
            .iter()
            .map(HasTargetKey::target_key)
            .collect();
        let mut lock_index = 0;
        let mut missing_direct_locks = Vec::new();
        for target in planned_keys {
            while lock_keys
                .get(lock_index)
                .is_some_and(|locked| locked < &target)
            {
                lock_index += 1;
            }
            if lock_keys.get(lock_index) == Some(&target) {
                lock_index += 1;
            } else {
                missing_direct_locks.push(target);
            }
        }
        if missing_direct_locks != structural_closure_covered_targets
            || !missing_direct_locks.is_empty() && structural_capability_row_id.is_none()
        {
            return Err(RepositoryContractError(
                "planned targets lack exact target or structural-closure lock coverage",
            ));
        }

        Ok(Self {
            scope,
            planned_targets,
            lock_targets,
            selective_objects_capability_id,
            structural_capability_row_id,
        })
    }
}

/// A validated selective repository update plan.
///
/// The fields are private and the type deliberately has no `Deserialize`
/// implementation: callers can only obtain a value through `new`, which
/// derives both digests and the structural-confirmation branch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SelectiveRepositoryUpdatePlan {
    scope: SelectiveRepositoryUpdateScope,
    planned_targets: RepositoryTargetStates,
    lock_targets: RepositoryUpdateLockTargets,
    expected_target_revision_map_digest: Sha256Digest,
    selective_objects_capability_id: CapabilityRowId,
    structural_confirmation_required: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    structural_capability_row_id: Option<CapabilityRowId>,
    plan_digest: Sha256Digest,
}

impl JsonSchema for SelectiveRepositoryUpdatePlan {
    fn schema_name() -> Cow<'static, str> {
        "SelectiveRepositoryUpdatePlan".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<NonStructuralSelectiveRepositoryUpdatePlanSchema>(),
            generator.subschema_for::<StructuralSelectiveRepositoryUpdatePlanSchema>(),
        ])
    }
}

impl SelectiveRepositoryUpdatePlan {
    pub(crate) fn routine_from_authority(
        authority: RoutineSelectiveRepositoryUpdatePlanAuthority,
        expected_planned_targets: &RepositoryTargetStates,
        expected_structural_targets: &[super::RepositoryTargetIdentity],
    ) -> Result<Self, RepositoryContractError> {
        if &authority.plan_authority.planned_targets != expected_planned_targets
            || authority.structural_targets.as_slice() != expected_structural_targets
        {
            return Err(RepositoryContractError(
                "routine plan authority disagrees with the independently folded target map",
            ));
        }
        Self::new(authority.plan_authority)
    }

    pub(crate) fn support_root_from_authority(
        authority: SupportRootSelectiveRepositoryUpdatePlanAuthority,
        before_anchor: &RepositoryAnchor,
        history: &ValidatedSupportPrerequisiteHistoryProjection,
    ) -> Result<(Self, bool), RepositoryContractError> {
        let token = authority.token;
        let authorized_observation = history.authorized_observation();
        let authorized = authorized_observation
            .authorized_support_projection()
            .ok_or(RepositoryContractError(
                "support-root plan requires its authorized observation",
            ))?;
        let expected_targets = if token.update_required {
            RepositoryTargetStates(vec![RepositoryTargetState::RootPresent(
                RootPresentTargetState {
                    target_kind: ConfigurationRootKind::Value,
                    state: PresentState::Value,
                    repository_version: history.final_root_repository_version().clone(),
                    target_fingerprint: token.observed_after_root_fingerprint.clone(),
                },
            )])
        } else {
            RepositoryTargetStates(Vec::new())
        };
        if token.before_anchor_digest != *before_anchor.anchor_digest()
            || token.history_partition_digest != *history.partition().partition_digest()
            || token.authorized_repository_version != *authorized_observation.repository_version()
            || token.authorized_root_delta_digest != *authorized.root_delta_digest()
            || token.final_root_repository_version != *history.final_root_repository_version()
            || token.plan_authority.planned_targets != expected_targets
        {
            return Err(RepositoryContractError(
                "support-root plan capability differs from the authorized root observation",
            ));
        }
        Self::new(token.plan_authority).map(|plan| (plan, token.update_required))
    }

    pub(crate) fn recovery_finalization_from_approved(
        _token: &SupportRecoveryAuthorityToken,
        observation: SupportRecoverySelectiveUpdatePlanObservation,
    ) -> Result<Self, RepositoryContractError> {
        let SupportRecoverySelectiveUpdatePlanObservation {
            planned_targets,
            lock_targets,
            selective_objects_capability_id,
            structural_capability_row_id,
            structural_closure_covered_targets,
        } = observation;
        let covered_targets = structural_closure_covered_targets
            .iter()
            .map(HasTargetKey::target_key)
            .collect();
        SelectiveRepositoryUpdatePlanAuthority::from_capability_adapter(
            SelectiveRepositoryUpdateScope::RecoveryFinalization,
            planned_targets,
            lock_targets,
            selective_objects_capability_id,
            structural_capability_row_id,
            covered_targets,
        )
        .and_then(Self::new)
    }

    #[cfg(test)]
    pub(crate) fn recovery_finalization_test_only(
        planned_targets: RepositoryTargetStates,
        lock_targets: RepositoryUpdateLockTargets,
        selective_objects_capability_id: CapabilityRowId,
        structural_capability_row_id: Option<CapabilityRowId>,
    ) -> Result<Self, RepositoryContractError> {
        SelectiveRepositoryUpdatePlanAuthority::from_capability_adapter(
            SelectiveRepositoryUpdateScope::RecoveryFinalization,
            planned_targets,
            lock_targets,
            selective_objects_capability_id,
            structural_capability_row_id,
            Vec::new(),
        )
        .and_then(Self::new)
    }

    pub(crate) fn new(
        authority: SelectiveRepositoryUpdatePlanAuthority,
    ) -> Result<Self, RepositoryContractError> {
        let SelectiveRepositoryUpdatePlanAuthority {
            scope,
            planned_targets,
            lock_targets,
            selective_objects_capability_id,
            structural_capability_row_id,
        } = authority;
        let expected_target_revision_map_digest = canonical_contract_digest(
            &RepositoryTargetRevisionMapDigestRecord(planned_targets.clone()),
            None,
        )
        .map_err(|_| RepositoryContractError("target revision map digest failed"))?;
        let record = SelectiveRepositoryUpdatePlanDigestRecord {
            scope,
            planned_targets,
            lock_targets,
            expected_target_revision_map_digest,
            selective_objects_capability_id,
            structural_confirmation_required: structural_capability_row_id.is_some(),
            structural_capability_row_id,
        };
        let plan_digest = canonical_contract_digest(&record, None)
            .map_err(|_| RepositoryContractError("selective update plan digest failed"))?;
        Ok(Self::from_digest_record(record, plan_digest))
    }

    fn from_digest_record(
        record: SelectiveRepositoryUpdatePlanDigestRecord,
        plan_digest: Sha256Digest,
    ) -> Self {
        Self {
            scope: record.scope,
            planned_targets: record.planned_targets,
            lock_targets: record.lock_targets,
            expected_target_revision_map_digest: record.expected_target_revision_map_digest,
            selective_objects_capability_id: record.selective_objects_capability_id,
            structural_confirmation_required: record.structural_confirmation_required,
            structural_capability_row_id: record.structural_capability_row_id,
            plan_digest,
        }
    }

    pub(crate) fn scope(&self) -> SelectiveRepositoryUpdateScope {
        self.scope
    }

    pub(crate) fn planned_targets(&self) -> &RepositoryTargetStates {
        &self.planned_targets
    }

    pub(crate) fn lock_targets(&self) -> &RepositoryUpdateLockTargets {
        &self.lock_targets
    }

    pub(crate) fn expected_target_revision_map_digest(&self) -> &Sha256Digest {
        &self.expected_target_revision_map_digest
    }

    pub(crate) fn selective_objects_capability_id(&self) -> &CapabilityRowId {
        &self.selective_objects_capability_id
    }

    pub(crate) fn structural_confirmation_required(&self) -> bool {
        self.structural_confirmation_required
    }

    pub(crate) fn structural_capability_row_id(&self) -> Option<&CapabilityRowId> {
        self.structural_capability_row_id.as_ref()
    }

    pub(crate) fn plan_digest(&self) -> &Sha256Digest {
        &self.plan_digest
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AlreadyExactAuthorityBasis {
    GuardedCurrentObservation,
    RecoveryFinalizationPriorOperation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum SelectiveRepositoryUpdateEffectOutcome {
    AlreadyExact {
        basis: AlreadyExactAuthorityBasis,
    },
    Performed {
        update_effect_receipt_id: UnicaId,
        update_effect_receipt_digest: Sha256Digest,
    },
}

/// Capability-observed effect branch for one recovery finalization.  The
/// branch is raw evidence only and cannot mint a repository proof by itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SupportRecoverySelectiveUpdateEffectObservation {
    AlreadyExact,
    Performed {
        update_effect_receipt_id: UnicaId,
        update_effect_receipt_digest: Sha256Digest,
    },
}

/// Typed observations retained from the selective-update window.  Construction
/// is available to an adapter, while proof construction remains token-gated in
/// this module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SupportRecoverySelectiveUpdateExecutionObservation {
    guard_receipt_id: UnicaId,
    observed_before_targets: RepositoryTargetStates,
    applied_targets: RepositoryTargetStates,
    acquired_root_first: AcquiredRepositoryUpdateLockTargets,
    released_in_reverse_order: ReleasedRepositoryUpdateLockTargets,
    before_original_target_fingerprint_map_digest: Sha256Digest,
    outcome: SupportRecoverySelectiveUpdateEffectObservation,
    verified_original_target_fingerprint_digest: Sha256Digest,
    observed_before_cursor: RepositoryHistoryCursor,
    observed_after_cursor: RepositoryHistoryCursor,
}

impl SupportRecoverySelectiveUpdateExecutionObservation {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_capability_adapter(
        guard_receipt_id: UnicaId,
        observed_before_targets: RepositoryTargetStates,
        applied_targets: RepositoryTargetStates,
        acquired_root_first: AcquiredRepositoryUpdateLockTargets,
        released_in_reverse_order: ReleasedRepositoryUpdateLockTargets,
        before_original_target_fingerprint_map_digest: Sha256Digest,
        outcome: SupportRecoverySelectiveUpdateEffectObservation,
        verified_original_target_fingerprint_digest: Sha256Digest,
        observed_before_cursor: RepositoryHistoryCursor,
        observed_after_cursor: RepositoryHistoryCursor,
    ) -> Self {
        Self {
            guard_receipt_id,
            observed_before_targets,
            applied_targets,
            acquired_root_first,
            released_in_reverse_order,
            before_original_target_fingerprint_map_digest,
            outcome,
            verified_original_target_fingerprint_digest,
            observed_before_cursor,
            observed_after_cursor,
        }
    }
}

/// Capability-derived observation of one complete selective-update lock window.
///
/// There is intentionally no production raw constructor in Task 7. A later
/// adapter factory must bind every retained observation to `plan_digest` before
/// this value can reach `SelectiveRepositoryUpdateProof::new`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SelectiveRepositoryUpdateExecutionAuthority {
    plan_digest: Sha256Digest,
    guard_receipt_id: UnicaId,
    observed_before_targets: RepositoryTargetStates,
    applied_targets: RepositoryTargetStates,
    acquired_root_first: AcquiredRepositoryUpdateLockTargets,
    released_in_reverse_order: ReleasedRepositoryUpdateLockTargets,
    before_original_target_fingerprint_map_digest: Sha256Digest,
    outcome: SelectiveRepositoryUpdateEffectOutcome,
    verified_original_target_fingerprint_digest: Sha256Digest,
    observed_before_cursor: RepositoryHistoryCursor,
    observed_after_cursor: RepositoryHistoryCursor,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SelectiveRepositoryUpdateProofDigestRecord {
    plan_digest: Sha256Digest,
    guard_receipt_id: UnicaId,
    planned_targets: RepositoryTargetStates,
    applied_targets: RepositoryTargetStates,
    expected_target_revision_map_digest: Sha256Digest,
    applied_target_revision_map_digest: Sha256Digest,
    lock_targets: RepositoryUpdateLockTargets,
    acquired_root_first: AcquiredRepositoryUpdateLockTargets,
    released_in_reverse_order: ReleasedRepositoryUpdateLockTargets,
    release_verified: TrueLiteral,
    before_original_target_fingerprint_map_digest: Sha256Digest,
    update_performed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    update_effect_receipt_id: Option<UnicaId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    update_effect_receipt_digest: Option<Sha256Digest>,
    structural_confirmation_used: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    structural_capability_row_id: Option<CapabilityRowId>,
    verified_original_target_fingerprint_digest: Sha256Digest,
    observed_before_cursor: RepositoryHistoryCursor,
    observed_after_cursor: RepositoryHistoryCursor,
    selective_objects_capability_id: CapabilityRowId,
}

impl contract_digest_record_sealed::Sealed for SelectiveRepositoryUpdateProofDigestRecord {}
impl ContractDigestRecord for SelectiveRepositoryUpdateProofDigestRecord {}

macro_rules! define_selective_update_proof_schema_pair {
    ($proof:ident, $digest:ident, { $($conditional:tt)* }) => {
        #[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct $proof {
            plan_digest: Sha256Digest,
            guard_receipt_id: UnicaId,
            planned_targets: RepositoryTargetStates,
            applied_targets: RepositoryTargetStates,
            expected_target_revision_map_digest: Sha256Digest,
            applied_target_revision_map_digest: Sha256Digest,
            lock_targets: RepositoryUpdateLockTargets,
            acquired_root_first: AcquiredRepositoryUpdateLockTargets,
            released_in_reverse_order: ReleasedRepositoryUpdateLockTargets,
            release_verified: TrueLiteral,
            before_original_target_fingerprint_map_digest: Sha256Digest,
            $($conditional)*
            verified_original_target_fingerprint_digest: Sha256Digest,
            observed_before_cursor: RepositoryHistoryCursor,
            observed_after_cursor: RepositoryHistoryCursor,
            selective_objects_capability_id: CapabilityRowId,
            proof_digest: Sha256Digest,
        }

        #[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct $digest {
            plan_digest: Sha256Digest,
            guard_receipt_id: UnicaId,
            planned_targets: RepositoryTargetStates,
            applied_targets: RepositoryTargetStates,
            expected_target_revision_map_digest: Sha256Digest,
            applied_target_revision_map_digest: Sha256Digest,
            lock_targets: RepositoryUpdateLockTargets,
            acquired_root_first: AcquiredRepositoryUpdateLockTargets,
            released_in_reverse_order: ReleasedRepositoryUpdateLockTargets,
            release_verified: TrueLiteral,
            before_original_target_fingerprint_map_digest: Sha256Digest,
            $($conditional)*
            verified_original_target_fingerprint_digest: Sha256Digest,
            observed_before_cursor: RepositoryHistoryCursor,
            observed_after_cursor: RepositoryHistoryCursor,
            selective_objects_capability_id: CapabilityRowId,
        }
    };
}

define_selective_update_proof_schema_pair!(
    AlreadyExactNonStructuralSelectiveRepositoryUpdateProofSchema,
    AlreadyExactNonStructuralSelectiveRepositoryUpdateProofDigestSchema,
    {
        update_performed: FalseLiteral,
        structural_confirmation_used: FalseLiteral,
    }
);
define_selective_update_proof_schema_pair!(
    AlreadyExactStructuralSelectiveRepositoryUpdateProofSchema,
    AlreadyExactStructuralSelectiveRepositoryUpdateProofDigestSchema,
    {
        update_performed: FalseLiteral,
        structural_confirmation_used: FalseLiteral,
        structural_capability_row_id: CapabilityRowId,
    }
);
define_selective_update_proof_schema_pair!(
    PerformedNonStructuralSelectiveRepositoryUpdateProofSchema,
    PerformedNonStructuralSelectiveRepositoryUpdateProofDigestSchema,
    {
        update_performed: TrueLiteral,
        update_effect_receipt_id: UnicaId,
        update_effect_receipt_digest: Sha256Digest,
        structural_confirmation_used: FalseLiteral,
    }
);
define_selective_update_proof_schema_pair!(
    PerformedStructuralSelectiveRepositoryUpdateProofSchema,
    PerformedStructuralSelectiveRepositoryUpdateProofDigestSchema,
    {
        update_performed: TrueLiteral,
        update_effect_receipt_id: UnicaId,
        update_effect_receipt_digest: Sha256Digest,
        structural_confirmation_used: TrueLiteral,
        structural_capability_row_id: CapabilityRowId,
    }
);

impl JsonSchema for SelectiveRepositoryUpdateProofDigestRecord {
    fn schema_name() -> Cow<'static, str> {
        "SelectiveRepositoryUpdateProofDigestRecord".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<
                AlreadyExactNonStructuralSelectiveRepositoryUpdateProofDigestSchema,
            >(),
            generator.subschema_for::<
                AlreadyExactStructuralSelectiveRepositoryUpdateProofDigestSchema,
            >(),
            generator.subschema_for::<
                PerformedNonStructuralSelectiveRepositoryUpdateProofDigestSchema,
            >(),
            generator.subschema_for::<PerformedStructuralSelectiveRepositoryUpdateProofDigestSchema>(),
        ])
    }
}

/// Completed proof of one selective repository update lock window.
///
/// This type intentionally cannot be deserialized or assembled field-by-field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SelectiveRepositoryUpdateProof {
    plan_digest: Sha256Digest,
    guard_receipt_id: UnicaId,
    planned_targets: RepositoryTargetStates,
    applied_targets: RepositoryTargetStates,
    expected_target_revision_map_digest: Sha256Digest,
    applied_target_revision_map_digest: Sha256Digest,
    lock_targets: RepositoryUpdateLockTargets,
    acquired_root_first: AcquiredRepositoryUpdateLockTargets,
    released_in_reverse_order: ReleasedRepositoryUpdateLockTargets,
    release_verified: TrueLiteral,
    before_original_target_fingerprint_map_digest: Sha256Digest,
    update_performed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    update_effect_receipt_id: Option<UnicaId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    update_effect_receipt_digest: Option<Sha256Digest>,
    structural_confirmation_used: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    structural_capability_row_id: Option<CapabilityRowId>,
    verified_original_target_fingerprint_digest: Sha256Digest,
    observed_before_cursor: RepositoryHistoryCursor,
    observed_after_cursor: RepositoryHistoryCursor,
    selective_objects_capability_id: CapabilityRowId,
    proof_digest: Sha256Digest,
}

impl JsonSchema for SelectiveRepositoryUpdateProof {
    fn schema_name() -> Cow<'static, str> {
        "SelectiveRepositoryUpdateProof".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator
                .subschema_for::<AlreadyExactNonStructuralSelectiveRepositoryUpdateProofSchema>(),
            generator.subschema_for::<AlreadyExactStructuralSelectiveRepositoryUpdateProofSchema>(),
            generator.subschema_for::<PerformedNonStructuralSelectiveRepositoryUpdateProofSchema>(),
            generator.subschema_for::<PerformedStructuralSelectiveRepositoryUpdateProofSchema>(),
        ])
    }
}

impl SelectiveRepositoryUpdateProof {
    pub(crate) fn recovery_finalization_from_approved(
        _token: &SupportRecoveryAuthorityToken,
        plan: &SelectiveRepositoryUpdatePlan,
        observation: SupportRecoverySelectiveUpdateExecutionObservation,
    ) -> Result<Self, RepositoryContractError> {
        let SupportRecoverySelectiveUpdateExecutionObservation {
            guard_receipt_id,
            observed_before_targets,
            applied_targets,
            acquired_root_first,
            released_in_reverse_order,
            before_original_target_fingerprint_map_digest,
            outcome,
            verified_original_target_fingerprint_digest,
            observed_before_cursor,
            observed_after_cursor,
        } = observation;
        let outcome = match outcome {
            SupportRecoverySelectiveUpdateEffectObservation::AlreadyExact => {
                SelectiveRepositoryUpdateEffectOutcome::AlreadyExact {
                    basis: AlreadyExactAuthorityBasis::GuardedCurrentObservation,
                }
            }
            SupportRecoverySelectiveUpdateEffectObservation::Performed {
                update_effect_receipt_id,
                update_effect_receipt_digest,
            } => SelectiveRepositoryUpdateEffectOutcome::Performed {
                update_effect_receipt_id,
                update_effect_receipt_digest,
            },
        };
        Self::new(
            plan,
            SelectiveRepositoryUpdateExecutionAuthority {
                plan_digest: plan.plan_digest.clone(),
                guard_receipt_id,
                observed_before_targets,
                applied_targets,
                acquired_root_first,
                released_in_reverse_order,
                before_original_target_fingerprint_map_digest,
                outcome,
                verified_original_target_fingerprint_digest,
                observed_before_cursor,
                observed_after_cursor,
            },
        )
    }

    /// Cross-contract fixture for Task 11 completion tests. The caller may
    /// select only receipt/cursor/fingerprint observations; targets, lock
    /// acquisition/release order, plan digest, and capabilities are copied
    /// from the already validated plan and still pass through `Self::new`.
    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub(in crate::domain::branched_development) fn recovery_finalization_already_exact_test_only(
        plan: &SelectiveRepositoryUpdatePlan,
        guard_receipt_id: UnicaId,
        before_original_target_fingerprint_map_digest: Sha256Digest,
        verified_original_target_fingerprint_digest: Sha256Digest,
        observed_before_cursor: RepositoryHistoryCursor,
        observed_after_cursor: RepositoryHistoryCursor,
    ) -> Result<Self, RepositoryContractError> {
        let authority = SelectiveRepositoryUpdateExecutionAuthority {
            plan_digest: plan.plan_digest.clone(),
            guard_receipt_id,
            observed_before_targets: plan.planned_targets.clone(),
            applied_targets: plan.planned_targets.clone(),
            acquired_root_first: AcquiredRepositoryUpdateLockTargets(plan.lock_targets.0.clone()),
            released_in_reverse_order: ReleasedRepositoryUpdateLockTargets(
                plan.lock_targets.0.iter().rev().cloned().collect(),
            ),
            before_original_target_fingerprint_map_digest,
            outcome: SelectiveRepositoryUpdateEffectOutcome::AlreadyExact {
                basis: AlreadyExactAuthorityBasis::GuardedCurrentObservation,
            },
            verified_original_target_fingerprint_digest,
            observed_before_cursor,
            observed_after_cursor,
        };
        Self::new(plan, authority)
    }

    pub(crate) fn new(
        plan: &SelectiveRepositoryUpdatePlan,
        authority: SelectiveRepositoryUpdateExecutionAuthority,
    ) -> Result<Self, RepositoryContractError> {
        let SelectiveRepositoryUpdateExecutionAuthority {
            plan_digest,
            guard_receipt_id,
            observed_before_targets,
            applied_targets,
            acquired_root_first,
            released_in_reverse_order,
            before_original_target_fingerprint_map_digest,
            outcome,
            verified_original_target_fingerprint_digest,
            observed_before_cursor,
            observed_after_cursor,
        } = authority;
        if plan_digest != plan.plan_digest {
            return Err(RepositoryContractError(
                "execution authority belongs to another selective update plan",
            ));
        }
        if applied_targets != plan.planned_targets {
            return Err(RepositoryContractError(
                "applied targets differ from the approved plan",
            ));
        }
        if acquired_root_first.0 != plan.lock_targets.0 {
            return Err(RepositoryContractError(
                "acquired lock targets differ from the approved plan",
            ));
        }
        if !released_in_reverse_order
            .0
            .iter()
            .eq(plan.lock_targets.0.iter().rev())
        {
            return Err(RepositoryContractError(
                "released lock targets are not the exact reverse of the approved plan",
            ));
        }

        let applied_target_revision_map_digest = canonical_contract_digest(
            &RepositoryTargetRevisionMapDigestRecord(applied_targets.clone()),
            None,
        )
        .map_err(|_| RepositoryContractError("applied target revision map digest failed"))?;
        let (update_performed, update_effect_receipt_id, update_effect_receipt_digest) =
            match outcome {
                SelectiveRepositoryUpdateEffectOutcome::AlreadyExact { basis: _ } => {
                    if observed_before_targets != plan.planned_targets {
                        return Err(RepositoryContractError(
                            "already-exact authority did not observe the approved target map",
                        ));
                    }
                    // Task 7's generic recovery scope cannot distinguish pre-arm
                    // cancellation finalization. Its later enclosing constructor
                    // must require the retained prior-operation basis for that
                    // specific flow; the generic proof only enforces map equality.
                    (false, None, None)
                }
                SelectiveRepositoryUpdateEffectOutcome::Performed {
                    update_effect_receipt_id,
                    update_effect_receipt_digest,
                } => (
                    true,
                    Some(update_effect_receipt_id),
                    Some(update_effect_receipt_digest),
                ),
            };
        let record = SelectiveRepositoryUpdateProofDigestRecord {
            plan_digest: plan.plan_digest.clone(),
            guard_receipt_id,
            planned_targets: plan.planned_targets.clone(),
            applied_targets,
            expected_target_revision_map_digest: plan.expected_target_revision_map_digest.clone(),
            applied_target_revision_map_digest,
            lock_targets: plan.lock_targets.clone(),
            acquired_root_first,
            released_in_reverse_order,
            release_verified: TrueLiteral,
            before_original_target_fingerprint_map_digest,
            update_performed,
            update_effect_receipt_id,
            update_effect_receipt_digest,
            structural_confirmation_used: update_performed && plan.structural_confirmation_required,
            structural_capability_row_id: plan.structural_capability_row_id.clone(),
            verified_original_target_fingerprint_digest,
            observed_before_cursor,
            observed_after_cursor,
            selective_objects_capability_id: plan.selective_objects_capability_id.clone(),
        };
        let proof_digest = canonical_contract_digest(&record, None)
            .map_err(|_| RepositoryContractError("selective update proof digest failed"))?;
        Ok(Self::from_digest_record(record, proof_digest))
    }

    fn from_digest_record(
        record: SelectiveRepositoryUpdateProofDigestRecord,
        proof_digest: Sha256Digest,
    ) -> Self {
        Self {
            plan_digest: record.plan_digest,
            guard_receipt_id: record.guard_receipt_id,
            planned_targets: record.planned_targets,
            applied_targets: record.applied_targets,
            expected_target_revision_map_digest: record.expected_target_revision_map_digest,
            applied_target_revision_map_digest: record.applied_target_revision_map_digest,
            lock_targets: record.lock_targets,
            acquired_root_first: record.acquired_root_first,
            released_in_reverse_order: record.released_in_reverse_order,
            release_verified: record.release_verified,
            before_original_target_fingerprint_map_digest: record
                .before_original_target_fingerprint_map_digest,
            update_performed: record.update_performed,
            update_effect_receipt_id: record.update_effect_receipt_id,
            update_effect_receipt_digest: record.update_effect_receipt_digest,
            structural_confirmation_used: record.structural_confirmation_used,
            structural_capability_row_id: record.structural_capability_row_id,
            verified_original_target_fingerprint_digest: record
                .verified_original_target_fingerprint_digest,
            observed_before_cursor: record.observed_before_cursor,
            observed_after_cursor: record.observed_after_cursor,
            selective_objects_capability_id: record.selective_objects_capability_id,
            proof_digest,
        }
    }

    pub(crate) fn plan_digest(&self) -> &Sha256Digest {
        &self.plan_digest
    }

    pub(crate) fn guard_receipt_id(&self) -> &UnicaId {
        &self.guard_receipt_id
    }

    pub(crate) fn planned_targets(&self) -> &RepositoryTargetStates {
        &self.planned_targets
    }

    pub(crate) fn applied_targets(&self) -> &RepositoryTargetStates {
        &self.applied_targets
    }

    pub(crate) fn expected_target_revision_map_digest(&self) -> &Sha256Digest {
        &self.expected_target_revision_map_digest
    }

    pub(crate) fn applied_target_revision_map_digest(&self) -> &Sha256Digest {
        &self.applied_target_revision_map_digest
    }

    pub(crate) fn lock_targets(&self) -> &RepositoryUpdateLockTargets {
        &self.lock_targets
    }

    pub(crate) fn acquired_root_first(&self) -> &AcquiredRepositoryUpdateLockTargets {
        &self.acquired_root_first
    }

    pub(crate) fn released_in_reverse_order(&self) -> &ReleasedRepositoryUpdateLockTargets {
        &self.released_in_reverse_order
    }

    pub(crate) fn before_original_target_fingerprint_map_digest(&self) -> &Sha256Digest {
        &self.before_original_target_fingerprint_map_digest
    }

    pub(crate) fn update_performed(&self) -> bool {
        self.update_performed
    }

    pub(crate) fn update_effect_receipt_id(&self) -> Option<&UnicaId> {
        self.update_effect_receipt_id.as_ref()
    }

    pub(crate) fn update_effect_receipt_digest(&self) -> Option<&Sha256Digest> {
        self.update_effect_receipt_digest.as_ref()
    }

    pub(crate) fn structural_confirmation_used(&self) -> bool {
        self.structural_confirmation_used
    }

    pub(crate) fn structural_capability_row_id(&self) -> Option<&CapabilityRowId> {
        self.structural_capability_row_id.as_ref()
    }

    pub(crate) fn verified_original_target_fingerprint_digest(&self) -> &Sha256Digest {
        &self.verified_original_target_fingerprint_digest
    }

    pub(crate) fn observed_before_cursor(&self) -> &RepositoryHistoryCursor {
        &self.observed_before_cursor
    }

    pub(crate) fn observed_after_cursor(&self) -> &RepositoryHistoryCursor {
        &self.observed_after_cursor
    }

    pub(crate) fn selective_objects_capability_id(&self) -> &CapabilityRowId {
        &self.selective_objects_capability_id
    }

    pub(crate) fn proof_digest(&self) -> &Sha256Digest {
        &self.proof_digest
    }
}

/// Cross-contract fixture for support completion tests. It still derives the
/// plan and execution proof through the real constructors; only the external
/// root-lock/cursor observations are supplied by the test adapter.
#[cfg(test)]
#[allow(clippy::too_many_arguments)]
pub(in crate::domain::branched_development) fn support_root_already_exact_fixture_test_only(
    lock_targets: RepositoryUpdateLockTargets,
    selective_objects_capability_id: CapabilityRowId,
    guard_receipt_id: UnicaId,
    before_original_target_fingerprint_map_digest: Sha256Digest,
    verified_original_target_fingerprint_digest: Sha256Digest,
    observed_before_cursor: RepositoryHistoryCursor,
    observed_after_cursor: RepositoryHistoryCursor,
) -> Result<
    (
        SelectiveRepositoryUpdatePlan,
        SelectiveRepositoryUpdateProof,
    ),
    RepositoryContractError,
> {
    let plan = SelectiveRepositoryUpdatePlan::new(
        SelectiveRepositoryUpdatePlanAuthority::from_capability_adapter(
            SelectiveRepositoryUpdateScope::SupportRoot,
            RepositoryTargetStates(Vec::new()),
            lock_targets,
            selective_objects_capability_id,
            None,
            Vec::new(),
        )?,
    )?;
    let proof = SelectiveRepositoryUpdateProof::recovery_finalization_already_exact_test_only(
        &plan,
        guard_receipt_id,
        before_original_target_fingerprint_map_digest,
        verified_original_target_fingerprint_digest,
        observed_before_cursor,
        observed_after_cursor,
    )?;
    Ok((plan, proof))
}

#[cfg(test)]
mod tests {
    use super::{
        AlreadyExactAuthorityBasis, RepositoryTargetRevisionMapDigestRecord,
        RoutineSelectiveRepositoryUpdateCapabilityToken,
        RoutineSelectiveRepositoryUpdatePlanAuthority, SelectiveRepositoryUpdateEffectOutcome,
        SelectiveRepositoryUpdateExecutionAuthority, SelectiveRepositoryUpdatePlan,
        SelectiveRepositoryUpdatePlanAuthority, SelectiveRepositoryUpdatePlanDigestRecord,
        SelectiveRepositoryUpdateProof, SelectiveRepositoryUpdateProofDigestRecord,
        SelectiveRepositoryUpdateScope,
    };
    use crate::domain::branched_development::contracts::repository::{
        AcquiredRepositoryUpdateLockTargets, ReleasedRepositoryUpdateLockTargets,
        RepositoryHistoryCursor, RepositoryTargetIdentity, RepositoryTargetStates,
        RepositoryUpdateLockTargets,
    };
    use crate::domain::branched_development::contracts::schema::audit_json_schema;
    use crate::domain::branched_development::{CapabilityRowId, Sha256Digest, UnicaId};
    use schemars::{schema_for, JsonSchema};
    use serde_json::{json, Value};
    use sha2::{Digest, Sha256};

    struct ImplementsDeserialize;

    trait AmbiguousIfDeserialize<Marker> {
        fn marker() {}
    }

    impl<T: ?Sized> AmbiguousIfDeserialize<()> for T {}
    impl<T: for<'de> serde::Deserialize<'de>> AmbiguousIfDeserialize<ImplementsDeserialize> for T {}

    const SHA_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const SHA_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const SHA_C: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
    const OBJECT_A: &str = "00000000-0000-0000-0000-000000000001";
    const GUARD_ID: &str = "123e4567-e89b-12d3-a456-426614174000";
    const EFFECT_ID: &str = "123e4567-e89b-12d3-a456-426614174001";

    fn root_state() -> Value {
        json!({
            "targetKind": "configurationRoot",
            "state": "present",
            "repositoryVersion": "root-v1",
            "targetFingerprint": SHA_A
        })
    }

    fn object_state(version: &str) -> Value {
        json!({
            "targetKind": "developmentObject",
            "state": "present",
            "objectId": OBJECT_A,
            "repositoryVersion": version,
            "targetFingerprint": SHA_B
        })
    }

    fn root_lock(display: &str) -> Value {
        json!({
            "targetKind": "configurationRoot",
            "objectDisplay": display,
            "reasons": ["supportGraphGuard"]
        })
    }

    fn object_lock(display: &str) -> Value {
        json!({
            "targetKind": "developmentObject",
            "objectId": OBJECT_A,
            "objectDisplay": display,
            "reasons": ["updateTarget", "referenceClosure"]
        })
    }

    fn targets() -> RepositoryTargetStates {
        serde_json::from_value(json!([root_state(), object_state("object-v1")])).unwrap()
    }

    fn root_only_targets() -> RepositoryTargetStates {
        serde_json::from_value(json!([root_state()])).unwrap()
    }

    fn object_identity() -> RepositoryTargetIdentity {
        serde_json::from_value(json!({
            "targetKind": "developmentObject",
            "objectId": OBJECT_A,
        }))
        .unwrap()
    }

    fn lock_targets() -> RepositoryUpdateLockTargets {
        serde_json::from_value(json!([root_lock("Configuration"), object_lock("Catalog")])).unwrap()
    }

    fn root_only_lock_targets() -> RepositoryUpdateLockTargets {
        serde_json::from_value(json!([root_lock("Configuration")])).unwrap()
    }

    fn acquired_targets() -> AcquiredRepositoryUpdateLockTargets {
        serde_json::from_value(json!([root_lock("Configuration"), object_lock("Catalog")])).unwrap()
    }

    fn released_targets() -> ReleasedRepositoryUpdateLockTargets {
        serde_json::from_value(json!([object_lock("Catalog"), root_lock("Configuration")])).unwrap()
    }

    fn cursor(version: &str, digest: &str) -> RepositoryHistoryCursor {
        serde_json::from_value(json!({
            "throughVersion": version,
            "historyPrefixDigest": digest
        }))
        .unwrap()
    }

    fn capability(value: &str) -> CapabilityRowId {
        CapabilityRowId::parse(value).unwrap()
    }

    fn digest(value: &str) -> Sha256Digest {
        Sha256Digest::parse(value).unwrap()
    }

    fn plan(
        scope: SelectiveRepositoryUpdateScope,
        structural: bool,
    ) -> SelectiveRepositoryUpdatePlan {
        let authority = SelectiveRepositoryUpdatePlanAuthority::from_capability_adapter(
            scope,
            targets(),
            lock_targets(),
            capability("selective.objects.v1"),
            structural.then(|| capability("selective.structural.v1")),
            Vec::new(),
        )
        .unwrap();
        SelectiveRepositoryUpdatePlan::new(authority).unwrap()
    }

    fn execution_authority(
        plan: &SelectiveRepositoryUpdatePlan,
        performed: bool,
    ) -> SelectiveRepositoryUpdateExecutionAuthority {
        let outcome = if performed {
            SelectiveRepositoryUpdateEffectOutcome::Performed {
                update_effect_receipt_id: UnicaId::parse(EFFECT_ID).unwrap(),
                update_effect_receipt_digest: digest(SHA_C),
            }
        } else {
            SelectiveRepositoryUpdateEffectOutcome::AlreadyExact {
                basis: if plan.scope() == SelectiveRepositoryUpdateScope::RecoveryFinalization {
                    AlreadyExactAuthorityBasis::RecoveryFinalizationPriorOperation
                } else {
                    AlreadyExactAuthorityBasis::GuardedCurrentObservation
                },
            }
        };
        SelectiveRepositoryUpdateExecutionAuthority {
            plan_digest: plan.plan_digest().clone(),
            guard_receipt_id: UnicaId::parse(GUARD_ID).unwrap(),
            observed_before_targets: targets(),
            applied_targets: targets(),
            acquired_root_first: acquired_targets(),
            released_in_reverse_order: released_targets(),
            before_original_target_fingerprint_map_digest: digest(SHA_A),
            outcome,
            verified_original_target_fingerprint_digest: digest(SHA_B),
            observed_before_cursor: cursor("v-before", SHA_A),
            observed_after_cursor: cursor("v-after", SHA_B),
        }
    }

    fn proof(
        plan: &SelectiveRepositoryUpdatePlan,
        performed: bool,
    ) -> SelectiveRepositoryUpdateProof {
        SelectiveRepositoryUpdateProof::new(plan, execution_authority(plan, performed)).unwrap()
    }

    fn schema<T: JsonSchema>() -> Value {
        serde_json::to_value(schema_for!(T)).unwrap()
    }

    fn validator<T: JsonSchema>() -> jsonschema::Validator {
        jsonschema::options()
            .with_draft(jsonschema::Draft::Draft202012)
            .build(&schema::<T>())
            .expect("generated contract schema must compile")
    }

    fn assert_closed<T: JsonSchema>() {
        audit_json_schema(&schema::<T>()).expect("wire schema must be recursively closed");
    }

    fn jcs_digest(value: &Value) -> String {
        format!(
            "{:x}",
            Sha256::digest(serde_json_canonicalizer::to_vec(value).unwrap())
        )
    }

    #[test]
    fn plans_derive_exact_projection_and_named_record_digests() {
        for (scope, structural) in [
            (SelectiveRepositoryUpdateScope::RoutinePlannedObjects, false),
            (SelectiveRepositoryUpdateScope::SupportRoot, false),
            (SelectiveRepositoryUpdateScope::RecoveryFinalization, true),
        ] {
            let plan = plan(scope, structural);
            let value = serde_json::to_value(&plan).unwrap();
            assert!(validator::<SelectiveRepositoryUpdatePlan>().is_valid(&value));
            assert_eq!(value["scope"], serde_json::to_value(scope).unwrap());
            assert_eq!(value["structuralConfirmationRequired"], json!(structural));
            assert_eq!(value.get("structuralCapabilityRowId").is_some(), structural);
            assert_eq!(
                value["expectedTargetRevisionMapDigest"],
                json!(jcs_digest(&value["plannedTargets"]))
            );
            let mut projection = value.as_object().unwrap().clone();
            let observed = projection.remove("planDigest").unwrap();
            assert_eq!(observed, json!(jcs_digest(&Value::Object(projection))));
        }

        assert!(
            SelectiveRepositoryUpdatePlanAuthority::from_capability_adapter(
                SelectiveRepositoryUpdateScope::SupportRoot,
                targets(),
                lock_targets(),
                capability("selective.objects.v1"),
                Some(capability("selective.structural.v1")),
                Vec::new(),
            )
            .is_err()
        );

        assert!(
            SelectiveRepositoryUpdatePlanAuthority::from_capability_adapter(
                SelectiveRepositoryUpdateScope::RoutinePlannedObjects,
                targets(),
                root_only_lock_targets(),
                capability("selective.objects.v1"),
                None,
                Vec::new(),
            )
            .is_err()
        );

        let structural_targets = serde_json::from_value::<RepositoryTargetStates>(json!([
            root_state(),
            {
                "targetKind": "developmentObject",
                "state": "absent",
                "objectId": OBJECT_A,
                "absenceEstablishedAtVersion": "object-v2",
                "expectedAbsent": true
            }
        ]))
        .unwrap();
        let structural_authority = SelectiveRepositoryUpdatePlanAuthority::from_capability_adapter(
            SelectiveRepositoryUpdateScope::RoutinePlannedObjects,
            structural_targets,
            root_only_lock_targets(),
            capability("selective.objects.v1"),
            Some(capability("selective.structural.v1")),
            vec![super::TargetKey::Object(OBJECT_A.to_owned())],
        )
        .unwrap();
        assert!(SelectiveRepositoryUpdatePlan::new(structural_authority).is_ok());
    }

    #[test]
    fn routine_plan_authority_binds_exact_target_map_and_capability_evidence() {
        let expected_targets = targets();
        let token = RoutineSelectiveRepositoryUpdateCapabilityToken::from_capability_adapter(
            expected_targets.clone(),
            Vec::new(),
            lock_targets(),
            capability("selective.objects.v1"),
            None,
            Vec::new(),
        )
        .unwrap();
        let authority =
            RoutineSelectiveRepositoryUpdatePlanAuthority::from_capability_token(token).unwrap();
        let plan = SelectiveRepositoryUpdatePlan::routine_from_authority(
            authority,
            &expected_targets,
            &[],
        )
        .unwrap();
        assert_eq!(
            plan.scope(),
            SelectiveRepositoryUpdateScope::RoutinePlannedObjects
        );
        assert_eq!(plan.planned_targets(), &expected_targets);
        assert!(!plan.structural_confirmation_required());

        let wrong_map_token =
            RoutineSelectiveRepositoryUpdateCapabilityToken::from_capability_adapter(
                root_only_targets(),
                Vec::new(),
                root_only_lock_targets(),
                capability("selective.objects.v1"),
                None,
                Vec::new(),
            )
            .unwrap();
        let wrong_map_authority =
            RoutineSelectiveRepositoryUpdatePlanAuthority::from_capability_token(wrong_map_token)
                .unwrap();
        assert!(SelectiveRepositoryUpdatePlan::routine_from_authority(
            wrong_map_authority,
            &expected_targets,
            &[],
        )
        .is_err());
    }

    #[test]
    fn routine_plan_authority_rejects_foreign_structural_capability_binding() {
        let structural_target = object_identity();
        assert!(
            RoutineSelectiveRepositoryUpdateCapabilityToken::from_capability_adapter(
                targets(),
                vec![structural_target.clone()],
                lock_targets(),
                capability("selective.objects.v1"),
                None,
                Vec::new(),
            )
            .is_err()
        );
        assert!(
            RoutineSelectiveRepositoryUpdateCapabilityToken::from_capability_adapter(
                targets(),
                Vec::new(),
                lock_targets(),
                capability("selective.objects.v1"),
                Some(capability("selective.structural.v1")),
                Vec::new(),
            )
            .is_err()
        );

        let token = RoutineSelectiveRepositoryUpdateCapabilityToken::from_capability_adapter(
            targets(),
            vec![structural_target.clone()],
            lock_targets(),
            capability("selective.objects.v1"),
            Some(capability("selective.structural.v1")),
            Vec::new(),
        )
        .unwrap();
        let authority =
            RoutineSelectiveRepositoryUpdatePlanAuthority::from_capability_token(token).unwrap();
        assert!(
            SelectiveRepositoryUpdatePlan::routine_from_authority(authority, &targets(), &[],)
                .is_err()
        );
    }

    #[test]
    fn plan_schema_closes_conditional_presence_without_claiming_digest_relations() {
        let schema = validator::<SelectiveRepositoryUpdatePlan>();
        let non_structural = serde_json::to_value(plan(
            SelectiveRepositoryUpdateScope::RoutinePlannedObjects,
            false,
        ))
        .unwrap();
        let structural = serde_json::to_value(plan(
            SelectiveRepositoryUpdateScope::RecoveryFinalization,
            true,
        ))
        .unwrap();

        for field in [
            "plannedTargets",
            "lockTargets",
            "expectedTargetRevisionMapDigest",
            "selectiveObjectsCapabilityId",
            "structuralConfirmationRequired",
            "planDigest",
        ] {
            let mut omitted = non_structural.as_object().unwrap().clone();
            omitted.remove(field);
            assert!(
                !schema.is_valid(&Value::Object(omitted)),
                "accepted omission {field}"
            );
        }
        let mut unknown = non_structural.as_object().unwrap().clone();
        unknown.insert("unknown".into(), json!(true));
        assert!(!schema.is_valid(&Value::Object(unknown)));

        let mut null_capability = structural.as_object().unwrap().clone();
        null_capability.insert("structuralCapabilityRowId".into(), Value::Null);
        assert!(!schema.is_valid(&Value::Object(null_capability)));

        let mut forbidden_capability = non_structural.as_object().unwrap().clone();
        forbidden_capability.insert(
            "structuralCapabilityRowId".into(),
            json!("selective.structural.v1"),
        );
        assert!(!schema.is_valid(&Value::Object(forbidden_capability)));

        let mut support_structural = structural.as_object().unwrap().clone();
        support_structural.insert("scope".into(), json!("supportRoot"));
        assert!(!schema.is_valid(&Value::Object(support_structural)));

        // JSON Schema is intentionally a structural superset: it cannot compare
        // siblings or recompute either JCS digest. The constructor is the gate.
        let mut substituted = non_structural.as_object().unwrap().clone();
        substituted.insert("expectedTargetRevisionMapDigest".into(), json!(SHA_C));
        substituted.insert("planDigest".into(), json!(SHA_C));
        assert!(schema.is_valid(&Value::Object(substituted)));
    }

    #[test]
    fn proof_covers_all_four_exact_or_performed_and_structural_branches() {
        for (structural, performed) in [(false, false), (true, false), (false, true), (true, true)]
        {
            let plan = plan(
                SelectiveRepositoryUpdateScope::RoutinePlannedObjects,
                structural,
            );
            let proof = proof(&plan, performed);
            let value = serde_json::to_value(&proof).unwrap();
            assert!(validator::<SelectiveRepositoryUpdateProof>().is_valid(&value));
            assert_eq!(value["updatePerformed"], json!(performed));
            assert_eq!(value.get("updateEffectReceiptId").is_some(), performed);
            assert_eq!(value.get("updateEffectReceiptDigest").is_some(), performed);
            assert_eq!(
                value["structuralConfirmationUsed"],
                json!(performed && structural)
            );
            assert_eq!(value.get("structuralCapabilityRowId").is_some(), structural);
            assert_eq!(value["releaseVerified"], json!(true));
            assert_eq!(value["planDigest"], json!(plan.plan_digest().as_str()));
            assert_eq!(
                value["appliedTargetRevisionMapDigest"],
                json!(jcs_digest(&value["appliedTargets"]))
            );
            let mut projection = value.as_object().unwrap().clone();
            let observed = projection.remove("proofDigest").unwrap();
            assert_eq!(observed, json!(jcs_digest(&Value::Object(projection))));
        }
    }

    #[test]
    fn proof_constructor_rejects_every_within_proof_substitution() {
        let routine_plan = plan(SelectiveRepositoryUpdateScope::RoutinePlannedObjects, false);
        let mut wrong_plan = execution_authority(&routine_plan, false);
        wrong_plan.plan_digest = digest(SHA_C);
        assert!(SelectiveRepositoryUpdateProof::new(&routine_plan, wrong_plan).is_err());

        let mut substituted_applied = execution_authority(&routine_plan, false);
        substituted_applied.applied_targets = root_only_targets();
        assert!(SelectiveRepositoryUpdateProof::new(&routine_plan, substituted_applied).is_err());

        let mut false_already_exact = execution_authority(&routine_plan, false);
        false_already_exact.observed_before_targets = root_only_targets();
        assert!(SelectiveRepositoryUpdateProof::new(&routine_plan, false_already_exact).is_err());

        let substituted_acquisition: AcquiredRepositoryUpdateLockTargets =
            serde_json::from_value(json!([
                root_lock("Different configuration display"),
                object_lock("Catalog")
            ]))
            .unwrap();
        let mut authority = execution_authority(&routine_plan, false);
        authority.acquired_root_first = substituted_acquisition;
        assert!(SelectiveRepositoryUpdateProof::new(&routine_plan, authority).is_err());

        let substituted_release: ReleasedRepositoryUpdateLockTargets =
            serde_json::from_value(json!([
                object_lock("Different object display"),
                root_lock("Configuration")
            ]))
            .unwrap();
        let mut authority = execution_authority(&routine_plan, false);
        authority.released_in_reverse_order = substituted_release;
        assert!(SelectiveRepositoryUpdateProof::new(&routine_plan, authority).is_err());

        // The typed collection rejects non-reverse identity order before the
        // proof constructor; the constructor then compares every full record.
        assert!(
            serde_json::from_value::<ReleasedRepositoryUpdateLockTargets>(json!([
                root_lock("Configuration"),
                object_lock("Catalog")
            ]))
            .is_err()
        );

        let recovery_plan = plan(SelectiveRepositoryUpdateScope::RecoveryFinalization, false);
        let mut generic_recovery = execution_authority(&recovery_plan, false);
        generic_recovery.outcome = SelectiveRepositoryUpdateEffectOutcome::AlreadyExact {
            basis: AlreadyExactAuthorityBasis::GuardedCurrentObservation,
        };
        assert!(SelectiveRepositoryUpdateProof::new(&recovery_plan, generic_recovery).is_ok());
    }

    #[test]
    fn proof_schema_is_an_explicit_relational_superset_but_closes_wire_shape() {
        let plan = plan(SelectiveRepositoryUpdateScope::RoutinePlannedObjects, true);
        let performed = serde_json::to_value(proof(&plan, true)).unwrap();
        let already_exact = serde_json::to_value(proof(&plan, false)).unwrap();
        let schema = validator::<SelectiveRepositoryUpdateProof>();

        for field in ["updateEffectReceiptId", "updateEffectReceiptDigest"] {
            let mut omitted = performed.as_object().unwrap().clone();
            omitted.remove(field);
            assert!(!schema.is_valid(&Value::Object(omitted)));
            let mut null = performed.as_object().unwrap().clone();
            null.insert(field.into(), Value::Null);
            assert!(!schema.is_valid(&Value::Object(null)));
        }
        let mut missing_capability = performed.as_object().unwrap().clone();
        missing_capability.remove("structuralCapabilityRowId");
        assert!(!schema.is_valid(&Value::Object(missing_capability)));
        let mut null_capability = performed.as_object().unwrap().clone();
        null_capability.insert("structuralCapabilityRowId".into(), Value::Null);
        assert!(!schema.is_valid(&Value::Object(null_capability)));
        let mut receipt_on_exact = already_exact.as_object().unwrap().clone();
        receipt_on_exact.insert("updateEffectReceiptId".into(), json!(EFFECT_ID));
        assert!(!schema.is_valid(&Value::Object(receipt_on_exact)));
        let mut unknown = performed.as_object().unwrap().clone();
        unknown.insert("unknown".into(), json!(true));
        assert!(!schema.is_valid(&Value::Object(unknown)));

        // These substitutions violate constructor-only byte equality, reverse
        // equality, sibling equality, and digest relations. They remain schema
        // valid by design because Draft 2020-12 cannot express those relations.
        let mut substituted = performed.as_object().unwrap().clone();
        substituted.insert("plannedTargets".into(), json!([root_state()]));
        substituted.insert("expectedTargetRevisionMapDigest".into(), json!(SHA_C));
        substituted.insert("planDigest".into(), json!(SHA_C));
        substituted.insert("proofDigest".into(), json!(SHA_C));
        substituted.insert(
            "releasedInReverseOrder".into(),
            json!([
                object_lock("Substituted but structurally valid"),
                root_lock("Configuration")
            ]),
        );
        assert!(schema.is_valid(&Value::Object(substituted)));
    }

    #[test]
    fn selective_update_schemas_are_recursively_closed() {
        assert_closed::<RepositoryTargetRevisionMapDigestRecord>();
        assert_closed::<SelectiveRepositoryUpdatePlanDigestRecord>();
        assert_closed::<SelectiveRepositoryUpdatePlan>();
        assert_closed::<SelectiveRepositoryUpdateProofDigestRecord>();
        assert_closed::<SelectiveRepositoryUpdateProof>();
    }

    #[test]
    fn validated_plan_and_proof_have_a_compile_time_no_deserialize_guard() {
        let _ = <SelectiveRepositoryUpdatePlanAuthority as AmbiguousIfDeserialize<_>>::marker;
        let _ = <SelectiveRepositoryUpdateExecutionAuthority as AmbiguousIfDeserialize<_>>::marker;
        let _ = <SelectiveRepositoryUpdatePlan as AmbiguousIfDeserialize<_>>::marker;
        let _ = <SelectiveRepositoryUpdateProof as AmbiguousIfDeserialize<_>>::marker;
    }
}
