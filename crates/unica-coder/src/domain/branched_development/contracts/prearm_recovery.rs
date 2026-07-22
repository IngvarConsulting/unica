use super::instructions::{
    CleanManualWorkingInfobaseInstruction, CloseReservedOriginalDesignerInstruction,
    ReleaseRepositoryLocksInstruction,
};
use super::repository::{
    DeferredRepositoryAdvance, RepositoryHistoryCursor, RepositoryHistoryPartitionClassification,
    RepositoryOwnerIdentity, RepositoryTargetIdentity, RepositoryTargetStates,
    SelectiveRepositoryUpdatePlan, SelectiveRepositoryUpdateProof, SelectiveRepositoryUpdateScope,
    ValidatedRepositoryHistoryPartition,
};
use super::scalars::{RepositoryTargetDisplay, RequiredNullable};
use super::support::{ManualSupportTargetMode, ReservedOriginalLeaseStopEvidence};
use super::support_terminalization::ManualWorkingInfobaseStopEvidence;
use crate::domain::branched_development::canonical_json::{
    canonical_contract_digest, contract_digest_record_sealed, ContractDigestRecord,
};
use crate::domain::branched_development::{
    CapabilityRowId, OperationId, Sha256Digest, TaskPhase, UnicaId,
};
use schemars::{json_schema, JsonSchema, Schema, SchemaGenerator};
use serde::{Serialize, Serializer};
use std::borrow::Cow;
use std::collections::BTreeSet;
use std::fmt;

const MAX_PREARM_ITEMS: usize = 1_024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PreArmRecoveryContractError(&'static str);

impl fmt::Display for PreArmRecoveryContractError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.0)
    }
}

impl std::error::Error for PreArmRecoveryContractError {}

fn prearm_digest<T: ContractDigestRecord>(
    record: &T,
    message: &'static str,
) -> Result<Sha256Digest, PreArmRecoveryContractError> {
    canonical_contract_digest(record, None).map_err(|_| PreArmRecoveryContractError(message))
}

macro_rules! wire_literal {
    ($name:ident, $wire:literal) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
        enum $name {
            #[serde(rename = $wire)]
            Value,
        }
    };
}

macro_rules! bool_literal {
    ($name:ident, $value:literal) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        struct $name;

        impl Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: Serializer,
            {
                serializer.serialize_bool($value)
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

wire_literal!(
    PreArmCancellationEffectReceiptKind,
    "preArmCancellationEffect"
);
wire_literal!(ReservedOriginalMode, "reservedOriginal");
wire_literal!(SeparateWorkingInfobaseMode, "separateWorkingInfobase");
bool_literal!(PreArmTrueLiteral, true);
bool_literal!(PreArmFalseLiteral, false);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) enum PreArmCancellationEffectKind {
    RootGuardAcquire,
    ModeLeaseAcquire,
    SelectiveOriginalUpdate,
    AuthorizationCancellation,
    ModeLeaseRelease,
    RootGuardRelease,
    RecoveryFinalization,
}

/// Authority-only expected postconditions indexed by the exact effect slot.
///
/// Keeping the slots named is intentional: an aggregate digest or a digest set
/// cannot prove which postcondition belongs to which effect-intent formula.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct PreArmCancellationExpectedPostconditionDigests {
    root_guard_acquisition: Option<Sha256Digest>,
    mode_lease_acquisition: Option<Sha256Digest>,
    selective_original_update: Option<Sha256Digest>,
    authorization_cancellation: Option<Sha256Digest>,
    mode_lease_release: Option<Sha256Digest>,
    root_guard_release: Option<Sha256Digest>,
    recovery_finalization: Option<Sha256Digest>,
}

impl PreArmCancellationExpectedPostconditionDigests {
    fn get(&self, kind: PreArmCancellationEffectKind) -> Option<&Sha256Digest> {
        match kind {
            PreArmCancellationEffectKind::RootGuardAcquire => self.root_guard_acquisition.as_ref(),
            PreArmCancellationEffectKind::ModeLeaseAcquire => self.mode_lease_acquisition.as_ref(),
            PreArmCancellationEffectKind::SelectiveOriginalUpdate => {
                self.selective_original_update.as_ref()
            }
            PreArmCancellationEffectKind::AuthorizationCancellation => {
                self.authorization_cancellation.as_ref()
            }
            PreArmCancellationEffectKind::ModeLeaseRelease => self.mode_lease_release.as_ref(),
            PreArmCancellationEffectKind::RootGuardRelease => self.root_guard_release.as_ref(),
            PreArmCancellationEffectKind::RecoveryFinalization => {
                self.recovery_finalization.as_ref()
            }
        }
    }

    fn validate_exact_effects<'a>(
        &self,
        effects: impl IntoIterator<
            Item = (
                &'a PreArmCancellationReceiptRef,
                PreArmCancellationEffectKind,
            ),
        >,
    ) -> Result<(), PreArmRecoveryContractError> {
        let mut expected_kinds = BTreeSet::new();
        for (receipt, kind) in effects {
            if receipt.effect_kind() != kind
                || self.get(kind).is_none()
                || !expected_kinds.insert(kind)
            {
                return Err(PreArmRecoveryContractError(
                    "expected postconditions do not match the exact effect slots",
                ));
            }
        }
        let all_kinds = [
            PreArmCancellationEffectKind::RootGuardAcquire,
            PreArmCancellationEffectKind::ModeLeaseAcquire,
            PreArmCancellationEffectKind::SelectiveOriginalUpdate,
            PreArmCancellationEffectKind::AuthorizationCancellation,
            PreArmCancellationEffectKind::ModeLeaseRelease,
            PreArmCancellationEffectKind::RootGuardRelease,
            PreArmCancellationEffectKind::RecoveryFinalization,
        ];
        if all_kinds
            .into_iter()
            .any(|kind| self.get(kind).is_some() != expected_kinds.contains(&kind))
        {
            return Err(PreArmRecoveryContractError(
                "expected postconditions contain a missing or extraneous effect slot",
            ));
        }
        Ok(())
    }

    #[cfg(test)]
    fn from_pairs_test_only(
        values: impl IntoIterator<Item = (PreArmCancellationEffectKind, Sha256Digest)>,
    ) -> Result<Self, PreArmRecoveryContractError> {
        let mut result = Self::default();
        for (kind, digest) in values {
            let slot = match kind {
                PreArmCancellationEffectKind::RootGuardAcquire => {
                    &mut result.root_guard_acquisition
                }
                PreArmCancellationEffectKind::ModeLeaseAcquire => {
                    &mut result.mode_lease_acquisition
                }
                PreArmCancellationEffectKind::SelectiveOriginalUpdate => {
                    &mut result.selective_original_update
                }
                PreArmCancellationEffectKind::AuthorizationCancellation => {
                    &mut result.authorization_cancellation
                }
                PreArmCancellationEffectKind::ModeLeaseRelease => &mut result.mode_lease_release,
                PreArmCancellationEffectKind::RootGuardRelease => &mut result.root_guard_release,
                PreArmCancellationEffectKind::RecoveryFinalization => {
                    &mut result.recovery_finalization
                }
            };
            if slot.replace(digest).is_some() {
                return Err(PreArmRecoveryContractError(
                    "expected postcondition effect slot is duplicated",
                ));
            }
        }
        Ok(result)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
struct NonEmptyOrderedDigests(Vec<Sha256Digest>);

impl NonEmptyOrderedDigests {
    fn new(values: Vec<Sha256Digest>) -> Result<Self, PreArmRecoveryContractError> {
        if values.is_empty()
            || values.len() > MAX_PREARM_ITEMS
            || values.iter().collect::<BTreeSet<_>>().len() != values.len()
        {
            return Err(PreArmRecoveryContractError(
                "terminal observation digests must be non-empty, bounded, and unique in expected-observation order",
            ));
        }
        Ok(Self(values))
    }

    fn as_slice(&self) -> &[Sha256Digest] {
        &self.0
    }
}

impl JsonSchema for NonEmptyOrderedDigests {
    fn schema_name() -> Cow<'static, str> {
        "PreArmCancellationTerminalObservationDigests".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "array",
            "minItems": 1,
            "maxItems": MAX_PREARM_ITEMS,
            "uniqueItems": true,
            "items": generator.subschema_for::<Sha256Digest>(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct PriorOperationEffectIntentDigestRecord {
    effect_kind: PreArmCancellationEffectKind,
    prior_operation_id: OperationId,
    support_action_id: UnicaId,
    expected_support_action_digest: Sha256Digest,
    approved_cancellation_digest: Sha256Digest,
    manual_target_mode: ManualSupportTargetMode,
    selective_update_plan_digest: Sha256Digest,
    expected_postcondition_digest: Sha256Digest,
}

impl contract_digest_record_sealed::Sealed for PriorOperationEffectIntentDigestRecord {}
impl ContractDigestRecord for PriorOperationEffectIntentDigestRecord {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct FinalizationPlanEffectIntentDigestRecord {
    effect_kind: PreArmCancellationEffectKind,
    finalization_attempt_id: UnicaId,
    support_action_id: UnicaId,
    expected_support_action_digest: Sha256Digest,
    approved_cancellation_digest: Sha256Digest,
    effect_observation_digest: Sha256Digest,
    manual_target_mode: ManualSupportTargetMode,
    selective_update_plan_digest: Sha256Digest,
    expected_postcondition_digest: Sha256Digest,
}

impl contract_digest_record_sealed::Sealed for FinalizationPlanEffectIntentDigestRecord {}
impl ContractDigestRecord for FinalizationPlanEffectIntentDigestRecord {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct PreArmCancellationEffectReceiptDigestRecord {
    receipt_kind: PreArmCancellationEffectReceiptKind,
    receipt_id: UnicaId,
    effect_kind: PreArmCancellationEffectKind,
    effect_intent_digest: Sha256Digest,
    producer_action_id: UnicaId,
    producer_action_digest: Sha256Digest,
    terminal_observation_digests: NonEmptyOrderedDigests,
}

impl contract_digest_record_sealed::Sealed for PreArmCancellationEffectReceiptDigestRecord {}
impl ContractDigestRecord for PreArmCancellationEffectReceiptDigestRecord {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PreArmCancellationEffectReceiptAuthority {
    record: PreArmCancellationEffectReceiptDigestRecord,
}

impl PreArmCancellationEffectReceiptAuthority {
    #[cfg(test)]
    pub(crate) fn test_only(
        receipt_id: UnicaId,
        effect_kind: PreArmCancellationEffectKind,
        effect_intent_digest: Sha256Digest,
        producer_action_id: UnicaId,
        producer_action_digest: Sha256Digest,
        terminal_observation_digests: Vec<Sha256Digest>,
    ) -> Result<Self, PreArmRecoveryContractError> {
        Ok(Self {
            record: PreArmCancellationEffectReceiptDigestRecord {
                receipt_kind: PreArmCancellationEffectReceiptKind::Value,
                receipt_id,
                effect_kind,
                effect_intent_digest,
                producer_action_id,
                producer_action_digest,
                terminal_observation_digests: NonEmptyOrderedDigests::new(
                    terminal_observation_digests,
                )?,
            },
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct PreArmCancellationEffectReceipt {
    receipt_kind: PreArmCancellationEffectReceiptKind,
    receipt_id: UnicaId,
    effect_kind: PreArmCancellationEffectKind,
    effect_intent_digest: Sha256Digest,
    producer_action_id: UnicaId,
    producer_action_digest: Sha256Digest,
    terminal_observation_digests: NonEmptyOrderedDigests,
    receipt_digest: Sha256Digest,
}

impl PreArmCancellationEffectReceipt {
    pub(crate) fn new(
        authority: PreArmCancellationEffectReceiptAuthority,
    ) -> Result<Self, PreArmRecoveryContractError> {
        let receipt_digest = prearm_digest(
            &authority.record,
            "pre-arm effect receipt digest computation failed",
        )?;
        Ok(Self {
            receipt_kind: authority.record.receipt_kind,
            receipt_id: authority.record.receipt_id,
            effect_kind: authority.record.effect_kind,
            effect_intent_digest: authority.record.effect_intent_digest,
            producer_action_id: authority.record.producer_action_id,
            producer_action_digest: authority.record.producer_action_digest,
            terminal_observation_digests: authority.record.terminal_observation_digests,
            receipt_digest,
        })
    }

    pub(crate) const fn receipt_id(&self) -> &UnicaId {
        &self.receipt_id
    }

    pub(crate) const fn effect_kind(&self) -> PreArmCancellationEffectKind {
        self.effect_kind
    }

    pub(crate) const fn effect_intent_digest(&self) -> &Sha256Digest {
        &self.effect_intent_digest
    }

    pub(crate) const fn receipt_digest(&self) -> &Sha256Digest {
        &self.receipt_digest
    }

    pub(crate) const fn producer_action_id(&self) -> &UnicaId {
        &self.producer_action_id
    }

    pub(crate) const fn producer_action_digest(&self) -> &Sha256Digest {
        &self.producer_action_digest
    }

    pub(crate) fn terminal_observation_digests(&self) -> &[Sha256Digest] {
        self.terminal_observation_digests.as_slice()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PreArmCancellationReceiptSource {
    PriorOperation,
    FinalizationPlan,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(tag = "source", rename_all = "camelCase", deny_unknown_fields)]
enum PreArmCancellationReceiptRefRecord {
    PriorOperation {
        receipt: PreArmCancellationEffectReceipt,
    },
    FinalizationPlan {
        receipt_id: UnicaId,
        effect_kind: PreArmCancellationEffectKind,
        effect_intent_digest: Sha256Digest,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct PreArmCancellationReceiptRef(PreArmCancellationReceiptRefRecord);

impl JsonSchema for PreArmCancellationReceiptRef {
    fn schema_name() -> Cow<'static, str> {
        "PreArmCancellationReceiptRef".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        PreArmCancellationReceiptRefRecord::json_schema(generator)
    }
}

impl PreArmCancellationReceiptRef {
    pub(crate) fn prior_operation(receipt: PreArmCancellationEffectReceipt) -> Self {
        Self(PreArmCancellationReceiptRefRecord::PriorOperation { receipt })
    }

    #[cfg(test)]
    pub(crate) fn finalization_plan(
        receipt_id: UnicaId,
        effect_kind: PreArmCancellationEffectKind,
        effect_intent_digest: Sha256Digest,
    ) -> Self {
        Self(PreArmCancellationReceiptRefRecord::FinalizationPlan {
            receipt_id,
            effect_kind,
            effect_intent_digest,
        })
    }

    pub(crate) const fn source(&self) -> PreArmCancellationReceiptSource {
        match &self.0 {
            PreArmCancellationReceiptRefRecord::PriorOperation { .. } => {
                PreArmCancellationReceiptSource::PriorOperation
            }
            PreArmCancellationReceiptRefRecord::FinalizationPlan { .. } => {
                PreArmCancellationReceiptSource::FinalizationPlan
            }
        }
    }

    pub(crate) const fn effect_kind(&self) -> PreArmCancellationEffectKind {
        match &self.0 {
            PreArmCancellationReceiptRefRecord::PriorOperation { receipt } => receipt.effect_kind,
            PreArmCancellationReceiptRefRecord::FinalizationPlan { effect_kind, .. } => {
                *effect_kind
            }
        }
    }

    pub(crate) const fn receipt_id(&self) -> &UnicaId {
        match &self.0 {
            PreArmCancellationReceiptRefRecord::PriorOperation { receipt } => &receipt.receipt_id,
            PreArmCancellationReceiptRefRecord::FinalizationPlan { receipt_id, .. } => receipt_id,
        }
    }

    pub(crate) const fn effect_intent_digest(&self) -> &Sha256Digest {
        match &self.0 {
            PreArmCancellationReceiptRefRecord::PriorOperation { receipt } => {
                &receipt.effect_intent_digest
            }
            PreArmCancellationReceiptRefRecord::FinalizationPlan {
                effect_intent_digest,
                ..
            } => effect_intent_digest,
        }
    }

    pub(crate) const fn prior_receipt(&self) -> Option<&PreArmCancellationEffectReceipt> {
        match &self.0 {
            PreArmCancellationReceiptRefRecord::PriorOperation { receipt } => Some(receipt),
            PreArmCancellationReceiptRefRecord::FinalizationPlan { .. } => None,
        }
    }

    fn is_source(&self, expected: PreArmCancellationReceiptSource) -> bool {
        self.source() == expected
    }

    fn has_kind(&self, expected: PreArmCancellationEffectKind) -> bool {
        self.effect_kind() == expected
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
struct PreArmCancellationTargetRevisionMapDigestRecord(RepositoryTargetStates);

impl contract_digest_record_sealed::Sealed for PreArmCancellationTargetRevisionMapDigestRecord {}
impl ContractDigestRecord for PreArmCancellationTargetRevisionMapDigestRecord {}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ApprovedSelectiveUpdatePlanProjection {
    planned_targets: RepositoryTargetStates,
    expected_target_revision_map_digest: Sha256Digest,
    selective_objects_capability_id: CapabilityRowId,
    structural_confirmation_required: bool,
    structural_capability_row_id: Option<CapabilityRowId>,
    plan_digest: Sha256Digest,
}

impl ApprovedSelectiveUpdatePlanProjection {
    fn from_plan(
        plan: &SelectiveRepositoryUpdatePlan,
    ) -> Result<Self, PreArmRecoveryContractError> {
        if plan.scope() != SelectiveRepositoryUpdateScope::RecoveryFinalization {
            return Err(PreArmRecoveryContractError(
                "pre-arm recovery requires a recovery-finalization selective plan",
            ));
        }
        Ok(Self {
            planned_targets: plan.planned_targets().clone(),
            expected_target_revision_map_digest: plan.expected_target_revision_map_digest().clone(),
            selective_objects_capability_id: plan.selective_objects_capability_id().clone(),
            structural_confirmation_required: plan.structural_confirmation_required(),
            structural_capability_row_id: plan.structural_capability_row_id().cloned(),
            plan_digest: plan.plan_digest().clone(),
        })
    }

    #[cfg(test)]
    fn test_only(
        planned_targets: RepositoryTargetStates,
        selective_objects_capability_id: CapabilityRowId,
        structural_capability_row_id: Option<CapabilityRowId>,
    ) -> Result<Self, PreArmRecoveryContractError> {
        let expected_target_revision_map_digest = prearm_digest(
            &PreArmCancellationTargetRevisionMapDigestRecord(planned_targets.clone()),
            "test selective target map digest failed",
        )?;
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct TestPlanDigestRecord<'a> {
            planned_targets: &'a RepositoryTargetStates,
            expected_target_revision_map_digest: &'a Sha256Digest,
            selective_objects_capability_id: &'a CapabilityRowId,
            structural_capability_row_id: &'a Option<CapabilityRowId>,
        }
        impl contract_digest_record_sealed::Sealed for TestPlanDigestRecord<'_> {}
        impl ContractDigestRecord for TestPlanDigestRecord<'_> {}
        let plan_digest = prearm_digest(
            &TestPlanDigestRecord {
                planned_targets: &planned_targets,
                expected_target_revision_map_digest: &expected_target_revision_map_digest,
                selective_objects_capability_id: &selective_objects_capability_id,
                structural_capability_row_id: &structural_capability_row_id,
            },
            "test selective plan digest failed",
        )?;
        Ok(Self {
            planned_targets,
            expected_target_revision_map_digest,
            selective_objects_capability_id,
            structural_confirmation_required: structural_capability_row_id.is_some(),
            structural_capability_row_id,
            plan_digest,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct PreArmCancellationSelectiveUpdateEffectDigestRecord {
    update_effect_receipt: PreArmCancellationReceiptRef,
    selective_update_plan_digest: Sha256Digest,
    root_guard_acquisition_receipt: PreArmCancellationReceiptRef,
    mode_lease_acquisition_receipt: PreArmCancellationReceiptRef,
    planned_targets: RepositoryTargetStates,
    applied_targets: RepositoryTargetStates,
    applied_target_revision_map_digest: Sha256Digest,
    before_original_target_fingerprint_digest: Sha256Digest,
    verified_original_target_fingerprint_digest: Sha256Digest,
    observed_before_cursor: RepositoryHistoryCursor,
    observed_effect_cursor: RepositoryHistoryCursor,
    selective_objects_capability_id: CapabilityRowId,
    structural_confirmation_required: bool,
    structural_confirmation_used: bool,
}

impl contract_digest_record_sealed::Sealed for PreArmCancellationSelectiveUpdateEffectDigestRecord {}
impl ContractDigestRecord for PreArmCancellationSelectiveUpdateEffectDigestRecord {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PreArmCancellationSelectiveUpdateEffectAuthority {
    update_effect_receipt: PreArmCancellationReceiptRef,
    root_guard_acquisition_receipt: PreArmCancellationReceiptRef,
    mode_lease_acquisition_receipt: PreArmCancellationReceiptRef,
    applied_targets: RepositoryTargetStates,
    before_original_target_fingerprint_digest: Sha256Digest,
    verified_original_target_fingerprint_digest: Sha256Digest,
    observed_before_cursor: RepositoryHistoryCursor,
    observed_effect_cursor: RepositoryHistoryCursor,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct PreArmCancellationSelectiveUpdateEffect {
    update_effect_receipt: PreArmCancellationReceiptRef,
    selective_update_plan_digest: Sha256Digest,
    root_guard_acquisition_receipt: PreArmCancellationReceiptRef,
    mode_lease_acquisition_receipt: PreArmCancellationReceiptRef,
    planned_targets: RepositoryTargetStates,
    applied_targets: RepositoryTargetStates,
    applied_target_revision_map_digest: Sha256Digest,
    before_original_target_fingerprint_digest: Sha256Digest,
    verified_original_target_fingerprint_digest: Sha256Digest,
    observed_before_cursor: RepositoryHistoryCursor,
    observed_effect_cursor: RepositoryHistoryCursor,
    selective_objects_capability_id: CapabilityRowId,
    structural_confirmation_required: bool,
    structural_confirmation_used: bool,
    effect_digest: Sha256Digest,
}

impl PreArmCancellationSelectiveUpdateEffect {
    fn new(
        plan: &ApprovedSelectiveUpdatePlanProjection,
        authority: PreArmCancellationSelectiveUpdateEffectAuthority,
    ) -> Result<Self, PreArmRecoveryContractError> {
        if plan.planned_targets.is_empty_for_prearm()
            || authority.applied_targets != plan.planned_targets
            || !authority
                .update_effect_receipt
                .has_kind(PreArmCancellationEffectKind::SelectiveOriginalUpdate)
            || !authority
                .root_guard_acquisition_receipt
                .has_kind(PreArmCancellationEffectKind::RootGuardAcquire)
            || !authority
                .mode_lease_acquisition_receipt
                .has_kind(PreArmCancellationEffectKind::ModeLeaseAcquire)
            || !authority
                .root_guard_acquisition_receipt
                .is_source(PreArmCancellationReceiptSource::PriorOperation)
            || !authority
                .mode_lease_acquisition_receipt
                .is_source(PreArmCancellationReceiptSource::PriorOperation)
            || !authority
                .update_effect_receipt
                .is_source(PreArmCancellationReceiptSource::PriorOperation)
        {
            return Err(PreArmRecoveryContractError(
                "selective update effect does not match the approved guarded plan",
            ));
        }
        let applied_target_revision_map_digest = prearm_digest(
            &PreArmCancellationTargetRevisionMapDigestRecord(authority.applied_targets.clone()),
            "applied target revision map digest failed",
        )?;
        if applied_target_revision_map_digest != plan.expected_target_revision_map_digest {
            return Err(PreArmRecoveryContractError(
                "applied target revision map differs from the approved plan",
            ));
        }
        let record = PreArmCancellationSelectiveUpdateEffectDigestRecord {
            update_effect_receipt: authority.update_effect_receipt,
            selective_update_plan_digest: plan.plan_digest.clone(),
            root_guard_acquisition_receipt: authority.root_guard_acquisition_receipt,
            mode_lease_acquisition_receipt: authority.mode_lease_acquisition_receipt,
            planned_targets: plan.planned_targets.clone(),
            applied_targets: authority.applied_targets,
            applied_target_revision_map_digest,
            before_original_target_fingerprint_digest: authority
                .before_original_target_fingerprint_digest,
            verified_original_target_fingerprint_digest: authority
                .verified_original_target_fingerprint_digest,
            observed_before_cursor: authority.observed_before_cursor,
            observed_effect_cursor: authority.observed_effect_cursor,
            selective_objects_capability_id: plan.selective_objects_capability_id.clone(),
            structural_confirmation_required: plan.structural_confirmation_required,
            structural_confirmation_used: plan.structural_confirmation_required,
        };
        let effect_digest = prearm_digest(&record, "selective update effect digest failed")?;
        Ok(Self {
            update_effect_receipt: record.update_effect_receipt,
            selective_update_plan_digest: record.selective_update_plan_digest,
            root_guard_acquisition_receipt: record.root_guard_acquisition_receipt,
            mode_lease_acquisition_receipt: record.mode_lease_acquisition_receipt,
            planned_targets: record.planned_targets,
            applied_targets: record.applied_targets,
            applied_target_revision_map_digest: record.applied_target_revision_map_digest,
            before_original_target_fingerprint_digest: record
                .before_original_target_fingerprint_digest,
            verified_original_target_fingerprint_digest: record
                .verified_original_target_fingerprint_digest,
            observed_before_cursor: record.observed_before_cursor,
            observed_effect_cursor: record.observed_effect_cursor,
            selective_objects_capability_id: record.selective_objects_capability_id,
            structural_confirmation_required: record.structural_confirmation_required,
            structural_confirmation_used: record.structural_confirmation_used,
            effect_digest,
        })
    }
}

trait RepositoryTargetStatesPreArmExt {
    fn is_empty_for_prearm(&self) -> bool;
}

impl RepositoryTargetStatesPreArmExt for RepositoryTargetStates {
    fn is_empty_for_prearm(&self) -> bool {
        serde_json::to_value(self)
            .ok()
            .and_then(|value| value.as_array().map(Vec::is_empty))
            .unwrap_or(false)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct PreArmCancellationSelectiveUpdateAlreadyExactEvidenceDigestRecord {
    selective_update_plan_digest: Sha256Digest,
    root_guard_acquisition_receipt: PreArmCancellationReceiptRef,
    mode_lease_acquisition_receipt: PreArmCancellationReceiptRef,
    planned_targets: RepositoryTargetStates,
    expected_target_revision_map_digest: Sha256Digest,
    before_original_target_fingerprint_map_digest: Sha256Digest,
    verified_original_target_fingerprint_digest: Sha256Digest,
    observed_before_cursor: RepositoryHistoryCursor,
    selective_objects_capability_id: CapabilityRowId,
    structural_confirmation_required: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    structural_capability_row_id: Option<CapabilityRowId>,
    structural_confirmation_used: PreArmFalseLiteral,
}

impl contract_digest_record_sealed::Sealed
    for PreArmCancellationSelectiveUpdateAlreadyExactEvidenceDigestRecord
{
}
impl ContractDigestRecord for PreArmCancellationSelectiveUpdateAlreadyExactEvidenceDigestRecord {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct PreArmCancellationSelectiveUpdateAlreadyExactEvidence {
    selective_update_plan_digest: Sha256Digest,
    root_guard_acquisition_receipt: PreArmCancellationReceiptRef,
    mode_lease_acquisition_receipt: PreArmCancellationReceiptRef,
    planned_targets: RepositoryTargetStates,
    expected_target_revision_map_digest: Sha256Digest,
    before_original_target_fingerprint_map_digest: Sha256Digest,
    verified_original_target_fingerprint_digest: Sha256Digest,
    observed_before_cursor: RepositoryHistoryCursor,
    selective_objects_capability_id: CapabilityRowId,
    structural_confirmation_required: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    structural_capability_row_id: Option<CapabilityRowId>,
    structural_confirmation_used: PreArmFalseLiteral,
    evidence_digest: Sha256Digest,
}

impl PreArmCancellationSelectiveUpdateAlreadyExactEvidence {
    fn new(
        plan: &ApprovedSelectiveUpdatePlanProjection,
        root_guard_acquisition_receipt: PreArmCancellationReceiptRef,
        mode_lease_acquisition_receipt: PreArmCancellationReceiptRef,
        before_original_target_fingerprint_map_digest: Sha256Digest,
        verified_original_target_fingerprint_digest: Sha256Digest,
        observed_before_cursor: RepositoryHistoryCursor,
    ) -> Result<Self, PreArmRecoveryContractError> {
        if plan.planned_targets.is_empty_for_prearm()
            || !root_guard_acquisition_receipt
                .has_kind(PreArmCancellationEffectKind::RootGuardAcquire)
            || !mode_lease_acquisition_receipt
                .has_kind(PreArmCancellationEffectKind::ModeLeaseAcquire)
            || !root_guard_acquisition_receipt
                .is_source(PreArmCancellationReceiptSource::PriorOperation)
            || !mode_lease_acquisition_receipt
                .is_source(PreArmCancellationReceiptSource::PriorOperation)
        {
            return Err(PreArmRecoveryContractError(
                "already-exact evidence requires non-empty plan and prior guarded receipts",
            ));
        }
        let record = PreArmCancellationSelectiveUpdateAlreadyExactEvidenceDigestRecord {
            selective_update_plan_digest: plan.plan_digest.clone(),
            root_guard_acquisition_receipt,
            mode_lease_acquisition_receipt,
            planned_targets: plan.planned_targets.clone(),
            expected_target_revision_map_digest: plan.expected_target_revision_map_digest.clone(),
            before_original_target_fingerprint_map_digest,
            verified_original_target_fingerprint_digest,
            observed_before_cursor,
            selective_objects_capability_id: plan.selective_objects_capability_id.clone(),
            structural_confirmation_required: plan.structural_confirmation_required,
            structural_capability_row_id: plan.structural_capability_row_id.clone(),
            structural_confirmation_used: PreArmFalseLiteral,
        };
        let evidence_digest = prearm_digest(&record, "already-exact evidence digest failed")?;
        Ok(Self {
            selective_update_plan_digest: record.selective_update_plan_digest,
            root_guard_acquisition_receipt: record.root_guard_acquisition_receipt,
            mode_lease_acquisition_receipt: record.mode_lease_acquisition_receipt,
            planned_targets: record.planned_targets,
            expected_target_revision_map_digest: record.expected_target_revision_map_digest,
            before_original_target_fingerprint_map_digest: record
                .before_original_target_fingerprint_map_digest,
            verified_original_target_fingerprint_digest: record
                .verified_original_target_fingerprint_digest,
            observed_before_cursor: record.observed_before_cursor,
            selective_objects_capability_id: record.selective_objects_capability_id,
            structural_confirmation_required: record.structural_confirmation_required,
            structural_capability_row_id: record.structural_capability_row_id,
            structural_confirmation_used: record.structural_confirmation_used,
            evidence_digest,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) enum PreArmCancellationUpdateState {
    NotRequired,
    Applied,
    AlreadyExact,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(tag = "updateState", rename_all = "camelCase", deny_unknown_fields)]
enum PreArmCancellationUpdateProgressRecord {
    NotRequired,
    Applied {
        selective_update_effect: PreArmCancellationSelectiveUpdateEffect,
    },
    AlreadyExact {
        already_exact_evidence: PreArmCancellationSelectiveUpdateAlreadyExactEvidence,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct PreArmCancellationUpdateProgress(PreArmCancellationUpdateProgressRecord);

impl JsonSchema for PreArmCancellationUpdateProgress {
    fn schema_name() -> Cow<'static, str> {
        "PreArmCancellationUpdateProgress".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        PreArmCancellationUpdateProgressRecord::json_schema(generator)
    }
}

impl PreArmCancellationUpdateProgress {
    fn not_required(
        plan: &ApprovedSelectiveUpdatePlanProjection,
    ) -> Result<Self, PreArmRecoveryContractError> {
        if !plan.planned_targets.is_empty_for_prearm() {
            return Err(PreArmRecoveryContractError(
                "notRequired update progress requires an empty selective target set",
            ));
        }
        Ok(Self(PreArmCancellationUpdateProgressRecord::NotRequired))
    }

    fn applied(
        plan: &ApprovedSelectiveUpdatePlanProjection,
        authority: PreArmCancellationSelectiveUpdateEffectAuthority,
    ) -> Result<Self, PreArmRecoveryContractError> {
        Ok(Self(PreArmCancellationUpdateProgressRecord::Applied {
            selective_update_effect: PreArmCancellationSelectiveUpdateEffect::new(plan, authority)?,
        }))
    }

    fn already_exact(evidence: PreArmCancellationSelectiveUpdateAlreadyExactEvidence) -> Self {
        Self(PreArmCancellationUpdateProgressRecord::AlreadyExact {
            already_exact_evidence: evidence,
        })
    }

    pub(crate) const fn state(&self) -> PreArmCancellationUpdateState {
        match &self.0 {
            PreArmCancellationUpdateProgressRecord::NotRequired => {
                PreArmCancellationUpdateState::NotRequired
            }
            PreArmCancellationUpdateProgressRecord::Applied { .. } => {
                PreArmCancellationUpdateState::Applied
            }
            PreArmCancellationUpdateProgressRecord::AlreadyExact { .. } => {
                PreArmCancellationUpdateState::AlreadyExact
            }
        }
    }

    fn update_receipt(&self) -> Option<&PreArmCancellationReceiptRef> {
        match &self.0 {
            PreArmCancellationUpdateProgressRecord::Applied {
                selective_update_effect,
            } => Some(&selective_update_effect.update_effect_receipt),
            PreArmCancellationUpdateProgressRecord::NotRequired
            | PreArmCancellationUpdateProgressRecord::AlreadyExact { .. } => None,
        }
    }

    #[cfg(test)]
    fn selective_update_plan_digest(&self) -> Option<&Sha256Digest> {
        match &self.0 {
            PreArmCancellationUpdateProgressRecord::NotRequired => None,
            PreArmCancellationUpdateProgressRecord::Applied {
                selective_update_effect,
            } => Some(&selective_update_effect.selective_update_plan_digest),
            PreArmCancellationUpdateProgressRecord::AlreadyExact {
                already_exact_evidence,
            } => Some(&already_exact_evidence.selective_update_plan_digest),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) enum PreArmCancellationEffectProgressStage {
    NoGuard,
    RootHeldBeforeLease,
    RootReleasedBeforeLease,
    GuardsHeldBeforeUpdate,
    ModeReleasedBeforeUpdateRootHeld,
    GuardsReleasedBeforeUpdate,
    UpdateReadyGuardsHeld,
    CancellationPersistedGuardsHeld,
    CancellationPersistedModeReleased,
    CancellationPersistedReleased,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(tag = "stage", rename_all = "camelCase", deny_unknown_fields)]
enum PreArmCancellationEffectProgressRecord {
    NoGuard,
    RootHeldBeforeLease {
        root_guard_acquisition_receipt: PreArmCancellationReceiptRef,
    },
    RootReleasedBeforeLease {
        root_guard_acquisition_receipt: PreArmCancellationReceiptRef,
        root_guard_release_receipt: PreArmCancellationReceiptRef,
    },
    GuardsHeldBeforeUpdate {
        root_guard_acquisition_receipt: PreArmCancellationReceiptRef,
        mode_lease_acquisition_receipt: PreArmCancellationReceiptRef,
    },
    ModeReleasedBeforeUpdateRootHeld {
        root_guard_acquisition_receipt: PreArmCancellationReceiptRef,
        mode_lease_acquisition_receipt: PreArmCancellationReceiptRef,
        mode_lease_release_receipt: PreArmCancellationReceiptRef,
    },
    GuardsReleasedBeforeUpdate {
        root_guard_acquisition_receipt: PreArmCancellationReceiptRef,
        mode_lease_acquisition_receipt: PreArmCancellationReceiptRef,
        mode_lease_release_receipt: PreArmCancellationReceiptRef,
        root_guard_release_receipt: PreArmCancellationReceiptRef,
    },
    UpdateReadyGuardsHeld {
        root_guard_acquisition_receipt: PreArmCancellationReceiptRef,
        mode_lease_acquisition_receipt: PreArmCancellationReceiptRef,
        update_progress: PreArmCancellationUpdateProgress,
    },
    CancellationPersistedGuardsHeld {
        root_guard_acquisition_receipt: PreArmCancellationReceiptRef,
        mode_lease_acquisition_receipt: PreArmCancellationReceiptRef,
        update_progress: PreArmCancellationUpdateProgress,
        cancellation_persistence_receipt: PreArmCancellationReceiptRef,
    },
    CancellationPersistedModeReleased {
        root_guard_acquisition_receipt: PreArmCancellationReceiptRef,
        mode_lease_acquisition_receipt: PreArmCancellationReceiptRef,
        update_progress: PreArmCancellationUpdateProgress,
        cancellation_persistence_receipt: PreArmCancellationReceiptRef,
        mode_lease_release_receipt: PreArmCancellationReceiptRef,
    },
    CancellationPersistedReleased {
        root_guard_acquisition_receipt: PreArmCancellationReceiptRef,
        mode_lease_acquisition_receipt: PreArmCancellationReceiptRef,
        update_progress: PreArmCancellationUpdateProgress,
        cancellation_persistence_receipt: PreArmCancellationReceiptRef,
        mode_lease_release_receipt: PreArmCancellationReceiptRef,
        root_guard_release_receipt: PreArmCancellationReceiptRef,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct PreArmCancellationEffectProgress(PreArmCancellationEffectProgressRecord);

impl JsonSchema for PreArmCancellationEffectProgress {
    fn schema_name() -> Cow<'static, str> {
        "PreArmCancellationEffectProgress".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        PreArmCancellationEffectProgressRecord::json_schema(generator)
    }
}

impl PreArmCancellationEffectProgress {
    fn validate_prior_ref(
        receipt: &PreArmCancellationReceiptRef,
        kind: PreArmCancellationEffectKind,
    ) -> Result<(), PreArmRecoveryContractError> {
        if receipt.source() != PreArmCancellationReceiptSource::PriorOperation
            || receipt.effect_kind() != kind
        {
            return Err(PreArmRecoveryContractError(
                "original effect progress requires an exact prior-operation receipt kind",
            ));
        }
        Ok(())
    }

    fn validate_record(
        record: &PreArmCancellationEffectProgressRecord,
    ) -> Result<(), PreArmRecoveryContractError> {
        let progress = Self(record.clone());
        for (receipt, kind) in progress.ordered_refs_with_kinds() {
            Self::validate_prior_ref(receipt, kind)?;
        }
        let mut ids = BTreeSet::new();
        if !progress
            .ordered_refs_with_kinds()
            .into_iter()
            .all(|(receipt, _)| ids.insert(receipt.receipt_id().as_str().to_owned()))
        {
            return Err(PreArmRecoveryContractError(
                "effect progress repeats a receipt identifier",
            ));
        }
        if let Some(update_progress) = progress.update_progress() {
            match update_progress.state() {
                PreArmCancellationUpdateState::Applied => {
                    let effect_receipt =
                        update_progress
                            .update_receipt()
                            .ok_or(PreArmRecoveryContractError(
                                "applied update lacks its effect receipt",
                            ))?;
                    if progress.root_guard_acquisition_receipt()
                        != progress_receipt_from_update(update_progress, true)
                        || progress.mode_lease_acquisition_receipt()
                            != progress_receipt_from_update(update_progress, false)
                        || effect_receipt.effect_kind()
                            != PreArmCancellationEffectKind::SelectiveOriginalUpdate
                    {
                        return Err(PreArmRecoveryContractError(
                            "applied update does not reproduce its guarded receipt prefix",
                        ));
                    }
                }
                PreArmCancellationUpdateState::AlreadyExact => {
                    let (root, mode) = already_exact_guard_receipts(update_progress).ok_or(
                        PreArmRecoveryContractError(
                            "already-exact progress lacks complete guarded evidence",
                        ),
                    )?;
                    if progress.root_guard_acquisition_receipt() != Some(root)
                        || progress.mode_lease_acquisition_receipt() != Some(mode)
                    {
                        return Err(PreArmRecoveryContractError(
                            "already-exact evidence does not reproduce its guarded prefix",
                        ));
                    }
                }
                PreArmCancellationUpdateState::NotRequired => {}
            }
        }
        Ok(())
    }

    fn new(
        record: PreArmCancellationEffectProgressRecord,
    ) -> Result<Self, PreArmRecoveryContractError> {
        Self::validate_record(&record)?;
        Ok(Self(record))
    }

    #[cfg(test)]
    fn no_guard_test_only() -> Self {
        Self(PreArmCancellationEffectProgressRecord::NoGuard)
    }

    #[cfg(test)]
    fn root_held_before_lease_test_only(
        root_guard_acquisition_receipt: PreArmCancellationReceiptRef,
    ) -> Result<Self, PreArmRecoveryContractError> {
        Self::new(
            PreArmCancellationEffectProgressRecord::RootHeldBeforeLease {
                root_guard_acquisition_receipt,
            },
        )
    }

    #[cfg(test)]
    fn guards_held_before_update_test_only(
        root_guard_acquisition_receipt: PreArmCancellationReceiptRef,
        mode_lease_acquisition_receipt: PreArmCancellationReceiptRef,
    ) -> Result<Self, PreArmRecoveryContractError> {
        Self::new(
            PreArmCancellationEffectProgressRecord::GuardsHeldBeforeUpdate {
                root_guard_acquisition_receipt,
                mode_lease_acquisition_receipt,
            },
        )
    }

    #[cfg(test)]
    fn update_ready_test_only(
        root_guard_acquisition_receipt: PreArmCancellationReceiptRef,
        mode_lease_acquisition_receipt: PreArmCancellationReceiptRef,
        update_progress: PreArmCancellationUpdateProgress,
    ) -> Result<Self, PreArmRecoveryContractError> {
        Self::new(
            PreArmCancellationEffectProgressRecord::UpdateReadyGuardsHeld {
                root_guard_acquisition_receipt,
                mode_lease_acquisition_receipt,
                update_progress,
            },
        )
    }

    #[cfg(test)]
    fn cancellation_persisted_released_test_only(
        root_guard_acquisition_receipt: PreArmCancellationReceiptRef,
        mode_lease_acquisition_receipt: PreArmCancellationReceiptRef,
        update_progress: PreArmCancellationUpdateProgress,
        cancellation_persistence_receipt: PreArmCancellationReceiptRef,
        mode_lease_release_receipt: PreArmCancellationReceiptRef,
        root_guard_release_receipt: PreArmCancellationReceiptRef,
    ) -> Result<Self, PreArmRecoveryContractError> {
        Self::new(
            PreArmCancellationEffectProgressRecord::CancellationPersistedReleased {
                root_guard_acquisition_receipt,
                mode_lease_acquisition_receipt,
                update_progress,
                cancellation_persistence_receipt,
                mode_lease_release_receipt,
                root_guard_release_receipt,
            },
        )
    }

    pub(crate) const fn stage(&self) -> PreArmCancellationEffectProgressStage {
        match &self.0 {
            PreArmCancellationEffectProgressRecord::NoGuard => {
                PreArmCancellationEffectProgressStage::NoGuard
            }
            PreArmCancellationEffectProgressRecord::RootHeldBeforeLease { .. } => {
                PreArmCancellationEffectProgressStage::RootHeldBeforeLease
            }
            PreArmCancellationEffectProgressRecord::RootReleasedBeforeLease { .. } => {
                PreArmCancellationEffectProgressStage::RootReleasedBeforeLease
            }
            PreArmCancellationEffectProgressRecord::GuardsHeldBeforeUpdate { .. } => {
                PreArmCancellationEffectProgressStage::GuardsHeldBeforeUpdate
            }
            PreArmCancellationEffectProgressRecord::ModeReleasedBeforeUpdateRootHeld { .. } => {
                PreArmCancellationEffectProgressStage::ModeReleasedBeforeUpdateRootHeld
            }
            PreArmCancellationEffectProgressRecord::GuardsReleasedBeforeUpdate { .. } => {
                PreArmCancellationEffectProgressStage::GuardsReleasedBeforeUpdate
            }
            PreArmCancellationEffectProgressRecord::UpdateReadyGuardsHeld { .. } => {
                PreArmCancellationEffectProgressStage::UpdateReadyGuardsHeld
            }
            PreArmCancellationEffectProgressRecord::CancellationPersistedGuardsHeld { .. } => {
                PreArmCancellationEffectProgressStage::CancellationPersistedGuardsHeld
            }
            PreArmCancellationEffectProgressRecord::CancellationPersistedModeReleased {
                ..
            } => PreArmCancellationEffectProgressStage::CancellationPersistedModeReleased,
            PreArmCancellationEffectProgressRecord::CancellationPersistedReleased { .. } => {
                PreArmCancellationEffectProgressStage::CancellationPersistedReleased
            }
        }
    }

    pub(crate) fn root_guard_acquisition_receipt(&self) -> Option<&PreArmCancellationReceiptRef> {
        match &self.0 {
            PreArmCancellationEffectProgressRecord::NoGuard => None,
            PreArmCancellationEffectProgressRecord::RootHeldBeforeLease {
                root_guard_acquisition_receipt,
            }
            | PreArmCancellationEffectProgressRecord::RootReleasedBeforeLease {
                root_guard_acquisition_receipt,
                ..
            }
            | PreArmCancellationEffectProgressRecord::GuardsHeldBeforeUpdate {
                root_guard_acquisition_receipt,
                ..
            }
            | PreArmCancellationEffectProgressRecord::ModeReleasedBeforeUpdateRootHeld {
                root_guard_acquisition_receipt,
                ..
            }
            | PreArmCancellationEffectProgressRecord::GuardsReleasedBeforeUpdate {
                root_guard_acquisition_receipt,
                ..
            }
            | PreArmCancellationEffectProgressRecord::UpdateReadyGuardsHeld {
                root_guard_acquisition_receipt,
                ..
            }
            | PreArmCancellationEffectProgressRecord::CancellationPersistedGuardsHeld {
                root_guard_acquisition_receipt,
                ..
            }
            | PreArmCancellationEffectProgressRecord::CancellationPersistedModeReleased {
                root_guard_acquisition_receipt,
                ..
            }
            | PreArmCancellationEffectProgressRecord::CancellationPersistedReleased {
                root_guard_acquisition_receipt,
                ..
            } => Some(root_guard_acquisition_receipt),
        }
    }

    pub(crate) fn mode_lease_acquisition_receipt(&self) -> Option<&PreArmCancellationReceiptRef> {
        match &self.0 {
            PreArmCancellationEffectProgressRecord::NoGuard
            | PreArmCancellationEffectProgressRecord::RootHeldBeforeLease { .. }
            | PreArmCancellationEffectProgressRecord::RootReleasedBeforeLease { .. } => None,
            PreArmCancellationEffectProgressRecord::GuardsHeldBeforeUpdate {
                mode_lease_acquisition_receipt,
                ..
            }
            | PreArmCancellationEffectProgressRecord::ModeReleasedBeforeUpdateRootHeld {
                mode_lease_acquisition_receipt,
                ..
            }
            | PreArmCancellationEffectProgressRecord::GuardsReleasedBeforeUpdate {
                mode_lease_acquisition_receipt,
                ..
            }
            | PreArmCancellationEffectProgressRecord::UpdateReadyGuardsHeld {
                mode_lease_acquisition_receipt,
                ..
            }
            | PreArmCancellationEffectProgressRecord::CancellationPersistedGuardsHeld {
                mode_lease_acquisition_receipt,
                ..
            }
            | PreArmCancellationEffectProgressRecord::CancellationPersistedModeReleased {
                mode_lease_acquisition_receipt,
                ..
            }
            | PreArmCancellationEffectProgressRecord::CancellationPersistedReleased {
                mode_lease_acquisition_receipt,
                ..
            } => Some(mode_lease_acquisition_receipt),
        }
    }

    pub(crate) fn update_progress(&self) -> Option<&PreArmCancellationUpdateProgress> {
        match &self.0 {
            PreArmCancellationEffectProgressRecord::UpdateReadyGuardsHeld {
                update_progress,
                ..
            }
            | PreArmCancellationEffectProgressRecord::CancellationPersistedGuardsHeld {
                update_progress,
                ..
            }
            | PreArmCancellationEffectProgressRecord::CancellationPersistedModeReleased {
                update_progress,
                ..
            }
            | PreArmCancellationEffectProgressRecord::CancellationPersistedReleased {
                update_progress,
                ..
            } => Some(update_progress),
            _ => None,
        }
    }

    pub(crate) fn cancellation_persistence_receipt(&self) -> Option<&PreArmCancellationReceiptRef> {
        match &self.0 {
            PreArmCancellationEffectProgressRecord::CancellationPersistedGuardsHeld {
                cancellation_persistence_receipt,
                ..
            }
            | PreArmCancellationEffectProgressRecord::CancellationPersistedModeReleased {
                cancellation_persistence_receipt,
                ..
            }
            | PreArmCancellationEffectProgressRecord::CancellationPersistedReleased {
                cancellation_persistence_receipt,
                ..
            } => Some(cancellation_persistence_receipt),
            _ => None,
        }
    }

    pub(crate) fn mode_lease_release_receipt(&self) -> Option<&PreArmCancellationReceiptRef> {
        match &self.0 {
            PreArmCancellationEffectProgressRecord::ModeReleasedBeforeUpdateRootHeld {
                mode_lease_release_receipt,
                ..
            }
            | PreArmCancellationEffectProgressRecord::GuardsReleasedBeforeUpdate {
                mode_lease_release_receipt,
                ..
            }
            | PreArmCancellationEffectProgressRecord::CancellationPersistedModeReleased {
                mode_lease_release_receipt,
                ..
            }
            | PreArmCancellationEffectProgressRecord::CancellationPersistedReleased {
                mode_lease_release_receipt,
                ..
            } => Some(mode_lease_release_receipt),
            _ => None,
        }
    }

    pub(crate) fn root_guard_release_receipt(&self) -> Option<&PreArmCancellationReceiptRef> {
        match &self.0 {
            PreArmCancellationEffectProgressRecord::RootReleasedBeforeLease {
                root_guard_release_receipt,
                ..
            }
            | PreArmCancellationEffectProgressRecord::GuardsReleasedBeforeUpdate {
                root_guard_release_receipt,
                ..
            }
            | PreArmCancellationEffectProgressRecord::CancellationPersistedReleased {
                root_guard_release_receipt,
                ..
            } => Some(root_guard_release_receipt),
            _ => None,
        }
    }

    fn ordered_refs_with_kinds(
        &self,
    ) -> Vec<(&PreArmCancellationReceiptRef, PreArmCancellationEffectKind)> {
        let mut refs = Vec::new();
        if let Some(value) = self.root_guard_acquisition_receipt() {
            refs.push((value, PreArmCancellationEffectKind::RootGuardAcquire));
        }
        if let Some(value) = self.mode_lease_acquisition_receipt() {
            refs.push((value, PreArmCancellationEffectKind::ModeLeaseAcquire));
        }
        if let Some(value) = self
            .update_progress()
            .and_then(PreArmCancellationUpdateProgress::update_receipt)
        {
            refs.push((value, PreArmCancellationEffectKind::SelectiveOriginalUpdate));
        }
        if let Some(value) = self.cancellation_persistence_receipt() {
            refs.push((
                value,
                PreArmCancellationEffectKind::AuthorizationCancellation,
            ));
        }
        if let Some(value) = self.mode_lease_release_receipt() {
            refs.push((value, PreArmCancellationEffectKind::ModeLeaseRelease));
        }
        if let Some(value) = self.root_guard_release_receipt() {
            refs.push((value, PreArmCancellationEffectKind::RootGuardRelease));
        }
        refs
    }
}

fn progress_receipt_from_update(
    update: &PreArmCancellationUpdateProgress,
    root: bool,
) -> Option<&PreArmCancellationReceiptRef> {
    match &update.0 {
        PreArmCancellationUpdateProgressRecord::Applied {
            selective_update_effect,
        } => Some(if root {
            &selective_update_effect.root_guard_acquisition_receipt
        } else {
            &selective_update_effect.mode_lease_acquisition_receipt
        }),
        _ => None,
    }
}

fn already_exact_guard_receipts(
    update: &PreArmCancellationUpdateProgress,
) -> Option<(&PreArmCancellationReceiptRef, &PreArmCancellationReceiptRef)> {
    match &update.0 {
        PreArmCancellationUpdateProgressRecord::AlreadyExact {
            already_exact_evidence,
        } => Some((
            &already_exact_evidence.root_guard_acquisition_receipt,
            &already_exact_evidence.mode_lease_acquisition_receipt,
        )),
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct PreArmCancellationEffectObservationDigestRecord {
    observation_id: UnicaId,
    prior_operation_id: OperationId,
    support_action_id: UnicaId,
    expected_support_action_digest: Sha256Digest,
    approved_cancellation_digest: Sha256Digest,
    arming_receipt_absent: PreArmTrueLiteral,
    manual_target_mode: ManualSupportTargetMode,
    effect_progress: PreArmCancellationEffectProgress,
    history_partition: ValidatedRepositoryHistoryPartition,
    observed_original_fingerprint: Sha256Digest,
    observed_support_graph_digest: Sha256Digest,
}

impl contract_digest_record_sealed::Sealed for PreArmCancellationEffectObservationDigestRecord {}
impl ContractDigestRecord for PreArmCancellationEffectObservationDigestRecord {}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg(test)]
pub(crate) struct PreArmCancellationEffectObservationAuthority {
    observation_id: UnicaId,
    prior_operation_id: OperationId,
    support_action_id: UnicaId,
    expected_support_action_digest: Sha256Digest,
    approved_cancellation_digest: Sha256Digest,
    bound_cancelled_phase: TaskPhase,
    bound_relevant_advance_phase: TaskPhase,
    manual_target_mode: ManualSupportTargetMode,
    effect_progress: PreArmCancellationEffectProgress,
    history_partition: ValidatedRepositoryHistoryPartition,
    observed_original_fingerprint: Sha256Digest,
    observed_support_graph_digest: Sha256Digest,
    bound_selective_update_plan_digest: Sha256Digest,
    expected_postcondition_digests: PreArmCancellationExpectedPostconditionDigests,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct PreArmCancellationEffectObservation {
    observation_id: UnicaId,
    prior_operation_id: OperationId,
    support_action_id: UnicaId,
    expected_support_action_digest: Sha256Digest,
    approved_cancellation_digest: Sha256Digest,
    arming_receipt_absent: PreArmTrueLiteral,
    manual_target_mode: ManualSupportTargetMode,
    effect_progress: PreArmCancellationEffectProgress,
    history_partition: ValidatedRepositoryHistoryPartition,
    observed_original_fingerprint: Sha256Digest,
    observed_support_graph_digest: Sha256Digest,
    observation_digest: Sha256Digest,
    #[serde(skip)]
    #[schemars(skip)]
    bound_selective_update_plan_digest: Sha256Digest,
    #[serde(skip)]
    #[schemars(skip)]
    bound_cancelled_phase: TaskPhase,
    #[serde(skip)]
    #[schemars(skip)]
    bound_relevant_advance_phase: TaskPhase,
}

impl PreArmCancellationEffectObservation {
    #[cfg(test)]
    pub(crate) fn new_test_only(
        authority: PreArmCancellationEffectObservationAuthority,
    ) -> Result<Self, PreArmRecoveryContractError> {
        PreArmCancellationEffectProgress::validate_record(&authority.effect_progress.0)?;
        if authority
            .effect_progress
            .update_progress()
            .and_then(PreArmCancellationUpdateProgress::selective_update_plan_digest)
            .is_some_and(|digest| digest != &authority.bound_selective_update_plan_digest)
        {
            return Err(PreArmRecoveryContractError(
                "effect observation update progress belongs to another selective plan",
            ));
        }
        const ALLOWED: &[RepositoryHistoryPartitionClassification] = &[
            RepositoryHistoryPartitionClassification::UnrelatedRoutine,
            RepositoryHistoryPartitionClassification::RelevantRoutine,
            RepositoryHistoryPartitionClassification::ExternalSupport,
            RepositoryHistoryPartitionClassification::PreArmExternal,
        ];
        if !authority.history_partition.all_entries_are_one_of(ALLOWED) {
            return Err(PreArmRecoveryContractError(
                "pre-arm effect observation contains history illegal for awaitingArm",
            ));
        }
        let observed_effects = authority.effect_progress.ordered_refs_with_kinds();
        authority
            .expected_postcondition_digests
            .validate_exact_effects(observed_effects.iter().copied())?;
        for (receipt, effect_kind) in observed_effects {
            let expected_postcondition_digest = authority
                .expected_postcondition_digests
                .get(effect_kind)
                .ok_or(PreArmRecoveryContractError(
                    "observed effect has no expected postcondition binding",
                ))?;
            let expected_intent = prearm_digest(
                &PriorOperationEffectIntentDigestRecord {
                    effect_kind,
                    prior_operation_id: authority.prior_operation_id.clone(),
                    support_action_id: authority.support_action_id.clone(),
                    expected_support_action_digest: authority
                        .expected_support_action_digest
                        .clone(),
                    approved_cancellation_digest: authority.approved_cancellation_digest.clone(),
                    manual_target_mode: authority.manual_target_mode,
                    selective_update_plan_digest: authority
                        .bound_selective_update_plan_digest
                        .clone(),
                    expected_postcondition_digest: expected_postcondition_digest.clone(),
                },
                "prior-operation effect intent digest failed",
            )?;
            if receipt.effect_intent_digest() != &expected_intent {
                return Err(PreArmRecoveryContractError(
                    "effect observation receipt intent disagrees with the interrupted operation",
                ));
            }
        }
        let record = PreArmCancellationEffectObservationDigestRecord {
            observation_id: authority.observation_id,
            prior_operation_id: authority.prior_operation_id,
            support_action_id: authority.support_action_id,
            expected_support_action_digest: authority.expected_support_action_digest,
            approved_cancellation_digest: authority.approved_cancellation_digest,
            arming_receipt_absent: PreArmTrueLiteral,
            manual_target_mode: authority.manual_target_mode,
            effect_progress: authority.effect_progress,
            history_partition: authority.history_partition,
            observed_original_fingerprint: authority.observed_original_fingerprint,
            observed_support_graph_digest: authority.observed_support_graph_digest,
        };
        let observation_digest = prearm_digest(&record, "effect observation digest failed")?;
        Ok(Self {
            observation_id: record.observation_id,
            prior_operation_id: record.prior_operation_id,
            support_action_id: record.support_action_id,
            expected_support_action_digest: record.expected_support_action_digest,
            approved_cancellation_digest: record.approved_cancellation_digest,
            arming_receipt_absent: record.arming_receipt_absent,
            manual_target_mode: record.manual_target_mode,
            effect_progress: record.effect_progress,
            history_partition: record.history_partition,
            observed_original_fingerprint: record.observed_original_fingerprint,
            observed_support_graph_digest: record.observed_support_graph_digest,
            observation_digest,
            bound_selective_update_plan_digest: authority.bound_selective_update_plan_digest,
            bound_cancelled_phase: authority.bound_cancelled_phase,
            bound_relevant_advance_phase: authority.bound_relevant_advance_phase,
        })
    }

    pub(crate) const fn prior_operation_id(&self) -> &OperationId {
        &self.prior_operation_id
    }

    pub(crate) const fn support_action_id(&self) -> &UnicaId {
        &self.support_action_id
    }

    pub(crate) const fn expected_support_action_digest(&self) -> &Sha256Digest {
        &self.expected_support_action_digest
    }

    pub(crate) const fn approved_cancellation_digest(&self) -> &Sha256Digest {
        &self.approved_cancellation_digest
    }

    pub(crate) const fn cancelled_phase(&self) -> TaskPhase {
        self.bound_cancelled_phase
    }

    pub(crate) const fn relevant_advance_phase(&self) -> TaskPhase {
        self.bound_relevant_advance_phase
    }

    pub(crate) const fn manual_target_mode(&self) -> ManualSupportTargetMode {
        self.manual_target_mode
    }

    pub(crate) const fn effect_progress(&self) -> &PreArmCancellationEffectProgress {
        &self.effect_progress
    }

    pub(crate) const fn history_partition(&self) -> &ValidatedRepositoryHistoryPartition {
        &self.history_partition
    }

    pub(crate) const fn observation_digest(&self) -> &Sha256Digest {
        &self.observation_digest
    }

    fn selective_update_plan_digest(&self) -> &Sha256Digest {
        &self.bound_selective_update_plan_digest
    }

    const fn observed_original_fingerprint(&self) -> &Sha256Digest {
        &self.observed_original_fingerprint
    }

    fn completed_update_final_original_fingerprint(&self) -> Option<&Sha256Digest> {
        match self
            .effect_progress
            .update_progress()
            .map(|progress| &progress.0)
        {
            Some(PreArmCancellationUpdateProgressRecord::Applied {
                selective_update_effect,
            }) => Some(&selective_update_effect.verified_original_target_fingerprint_digest),
            Some(PreArmCancellationUpdateProgressRecord::AlreadyExact {
                already_exact_evidence,
            }) => Some(&already_exact_evidence.verified_original_target_fingerprint_digest),
            Some(PreArmCancellationUpdateProgressRecord::NotRequired) | None => None,
        }
    }

    fn current_original_fingerprint(&self) -> &Sha256Digest {
        self.completed_update_final_original_fingerprint()
            .unwrap_or(&self.observed_original_fingerprint)
    }

    const fn expected_final_support_graph_digest(&self) -> &Sha256Digest {
        &self.observed_support_graph_digest
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) enum PreArmCancellationSelectiveUpdateDisposition {
    NotRequired,
    AlreadyExact,
    AlreadyApplied,
    Perform,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct PreArmCancellationReceiptPlanDigestRecord {
    root_guard_acquisition_receipt: PreArmCancellationReceiptRef,
    mode_lease_acquisition_receipt: PreArmCancellationReceiptRef,
    selective_update_disposition: PreArmCancellationSelectiveUpdateDisposition,
    #[serde(skip_serializing_if = "Option::is_none")]
    selective_update_effect_receipt: Option<PreArmCancellationReceiptRef>,
    cancellation_persistence_receipt: PreArmCancellationReceiptRef,
    mode_lease_release_receipt: PreArmCancellationReceiptRef,
    root_guard_release_receipt: PreArmCancellationReceiptRef,
    recovery_finalization_receipt: PreArmCancellationReceiptRef,
}

impl contract_digest_record_sealed::Sealed for PreArmCancellationReceiptPlanDigestRecord {}
impl ContractDigestRecord for PreArmCancellationReceiptPlanDigestRecord {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PreArmCancellationReceiptPlanAuthority {
    root_guard_acquisition_receipt: PreArmCancellationReceiptRef,
    mode_lease_acquisition_receipt: PreArmCancellationReceiptRef,
    selective_update_effect_receipt: Option<PreArmCancellationReceiptRef>,
    cancellation_persistence_receipt: PreArmCancellationReceiptRef,
    mode_lease_release_receipt: PreArmCancellationReceiptRef,
    root_guard_release_receipt: PreArmCancellationReceiptRef,
    recovery_finalization_receipt: PreArmCancellationReceiptRef,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct PreArmCancellationReceiptPlan {
    root_guard_acquisition_receipt: PreArmCancellationReceiptRef,
    mode_lease_acquisition_receipt: PreArmCancellationReceiptRef,
    selective_update_disposition: PreArmCancellationSelectiveUpdateDisposition,
    #[serde(skip_serializing_if = "Option::is_none")]
    selective_update_effect_receipt: Option<PreArmCancellationReceiptRef>,
    cancellation_persistence_receipt: PreArmCancellationReceiptRef,
    mode_lease_release_receipt: PreArmCancellationReceiptRef,
    root_guard_release_receipt: PreArmCancellationReceiptRef,
    recovery_finalization_receipt: PreArmCancellationReceiptRef,
    receipt_plan_digest: Sha256Digest,
    #[serde(skip)]
    #[schemars(skip)]
    bound_selective_update_plan_digest: Sha256Digest,
    #[serde(skip)]
    #[schemars(skip)]
    bound_acquire_root_guard: bool,
    #[serde(skip)]
    #[schemars(skip)]
    bound_acquire_mode_lease: bool,
}

impl PreArmCancellationReceiptPlan {
    fn new(
        plan: &ApprovedSelectiveUpdatePlanProjection,
        observation: &PreArmCancellationEffectObservation,
        acquire_root_guard: bool,
        acquire_mode_lease: bool,
        authority: PreArmCancellationReceiptPlanAuthority,
    ) -> Result<Self, PreArmRecoveryContractError> {
        if &plan.plan_digest != observation.selective_update_plan_digest() {
            return Err(PreArmRecoveryContractError(
                "receipt plan selective update differs from the observed operation",
            ));
        }
        let progress = observation.effect_progress();
        let (selective_update_disposition, expected_update_receipt) =
            if plan.planned_targets.is_empty_for_prearm() {
                if progress.update_progress().is_some_and(|value| {
                    value.state() != PreArmCancellationUpdateState::NotRequired
                }) {
                    return Err(PreArmRecoveryContractError(
                        "empty selective plan has non-empty update progress",
                    ));
                }
                (
                    PreArmCancellationSelectiveUpdateDisposition::NotRequired,
                    None,
                )
            } else {
                match progress
                    .update_progress()
                    .map(PreArmCancellationUpdateProgress::state)
                {
                    Some(PreArmCancellationUpdateState::Applied) => (
                        PreArmCancellationSelectiveUpdateDisposition::AlreadyApplied,
                        progress
                            .update_progress()
                            .and_then(PreArmCancellationUpdateProgress::update_receipt),
                    ),
                    Some(PreArmCancellationUpdateState::AlreadyExact) => (
                        PreArmCancellationSelectiveUpdateDisposition::AlreadyExact,
                        None,
                    ),
                    Some(PreArmCancellationUpdateState::NotRequired) => {
                        return Err(PreArmRecoveryContractError(
                            "non-empty selective plan cannot be notRequired",
                        ));
                    }
                    None => (PreArmCancellationSelectiveUpdateDisposition::Perform, None),
                }
            };

        validate_receipt_ref_slot(
            &authority.root_guard_acquisition_receipt,
            PreArmCancellationEffectKind::RootGuardAcquire,
            (!acquire_root_guard)
                .then(|| progress.root_guard_acquisition_receipt())
                .flatten(),
        )?;
        validate_receipt_ref_slot(
            &authority.mode_lease_acquisition_receipt,
            PreArmCancellationEffectKind::ModeLeaseAcquire,
            (!acquire_mode_lease)
                .then(|| progress.mode_lease_acquisition_receipt())
                .flatten(),
        )?;
        validate_receipt_ref_slot(
            &authority.cancellation_persistence_receipt,
            PreArmCancellationEffectKind::AuthorizationCancellation,
            progress.cancellation_persistence_receipt(),
        )?;
        validate_receipt_ref_slot(
            &authority.mode_lease_release_receipt,
            PreArmCancellationEffectKind::ModeLeaseRelease,
            (!acquire_mode_lease)
                .then(|| progress.mode_lease_release_receipt())
                .flatten(),
        )?;
        validate_receipt_ref_slot(
            &authority.root_guard_release_receipt,
            PreArmCancellationEffectKind::RootGuardRelease,
            (!acquire_root_guard)
                .then(|| progress.root_guard_release_receipt())
                .flatten(),
        )?;
        if authority.recovery_finalization_receipt.effect_kind()
            != PreArmCancellationEffectKind::RecoveryFinalization
            || authority.recovery_finalization_receipt.source()
                != PreArmCancellationReceiptSource::FinalizationPlan
        {
            return Err(PreArmRecoveryContractError(
                "recovery-finalization receipt must be a future finalization-plan ref",
            ));
        }

        match selective_update_disposition {
            PreArmCancellationSelectiveUpdateDisposition::NotRequired
            | PreArmCancellationSelectiveUpdateDisposition::AlreadyExact => {
                if authority.selective_update_effect_receipt.is_some() {
                    return Err(PreArmRecoveryContractError(
                        "no-effect selective disposition cannot allocate an update receipt",
                    ));
                }
            }
            PreArmCancellationSelectiveUpdateDisposition::AlreadyApplied => {
                if authority.selective_update_effect_receipt.as_ref() != expected_update_receipt
                    || authority
                        .selective_update_effect_receipt
                        .as_ref()
                        .is_none_or(|value| {
                            value.source() != PreArmCancellationReceiptSource::PriorOperation
                        })
                {
                    return Err(PreArmRecoveryContractError(
                        "already-applied disposition must copy the immutable update receipt",
                    ));
                }
            }
            PreArmCancellationSelectiveUpdateDisposition::Perform => {
                if authority
                    .selective_update_effect_receipt
                    .as_ref()
                    .is_none_or(|value| {
                        value.effect_kind() != PreArmCancellationEffectKind::SelectiveOriginalUpdate
                            || value.source() != PreArmCancellationReceiptSource::FinalizationPlan
                    })
                {
                    return Err(PreArmRecoveryContractError(
                        "perform disposition requires one future selective-update receipt",
                    ));
                }
            }
        }

        let refs = [
            Some(&authority.root_guard_acquisition_receipt),
            Some(&authority.mode_lease_acquisition_receipt),
            authority.selective_update_effect_receipt.as_ref(),
            Some(&authority.cancellation_persistence_receipt),
            Some(&authority.mode_lease_release_receipt),
            Some(&authority.root_guard_release_receipt),
            Some(&authority.recovery_finalization_receipt),
        ];
        let mut receipt_ids = BTreeSet::new();
        if refs
            .into_iter()
            .flatten()
            .any(|value| !receipt_ids.insert(value.receipt_id().as_str().to_owned()))
        {
            return Err(PreArmRecoveryContractError(
                "receipt plan repeats one receipt ID across effect kinds",
            ));
        }

        let record = PreArmCancellationReceiptPlanDigestRecord {
            root_guard_acquisition_receipt: authority.root_guard_acquisition_receipt,
            mode_lease_acquisition_receipt: authority.mode_lease_acquisition_receipt,
            selective_update_disposition,
            selective_update_effect_receipt: authority.selective_update_effect_receipt,
            cancellation_persistence_receipt: authority.cancellation_persistence_receipt,
            mode_lease_release_receipt: authority.mode_lease_release_receipt,
            root_guard_release_receipt: authority.root_guard_release_receipt,
            recovery_finalization_receipt: authority.recovery_finalization_receipt,
        };
        let receipt_plan_digest = prearm_digest(&record, "receipt plan digest failed")?;
        Ok(Self {
            root_guard_acquisition_receipt: record.root_guard_acquisition_receipt,
            mode_lease_acquisition_receipt: record.mode_lease_acquisition_receipt,
            selective_update_disposition: record.selective_update_disposition,
            selective_update_effect_receipt: record.selective_update_effect_receipt,
            cancellation_persistence_receipt: record.cancellation_persistence_receipt,
            mode_lease_release_receipt: record.mode_lease_release_receipt,
            root_guard_release_receipt: record.root_guard_release_receipt,
            recovery_finalization_receipt: record.recovery_finalization_receipt,
            receipt_plan_digest,
            bound_selective_update_plan_digest: plan.plan_digest.clone(),
            bound_acquire_root_guard: acquire_root_guard,
            bound_acquire_mode_lease: acquire_mode_lease,
        })
    }

    pub(crate) const fn selective_update_disposition(
        &self,
    ) -> PreArmCancellationSelectiveUpdateDisposition {
        self.selective_update_disposition
    }

    pub(crate) const fn root_guard_acquisition_receipt(&self) -> &PreArmCancellationReceiptRef {
        &self.root_guard_acquisition_receipt
    }

    pub(crate) const fn mode_lease_acquisition_receipt(&self) -> &PreArmCancellationReceiptRef {
        &self.mode_lease_acquisition_receipt
    }

    pub(crate) const fn selective_update_effect_receipt(
        &self,
    ) -> Option<&PreArmCancellationReceiptRef> {
        self.selective_update_effect_receipt.as_ref()
    }

    pub(crate) const fn cancellation_persistence_receipt(&self) -> &PreArmCancellationReceiptRef {
        &self.cancellation_persistence_receipt
    }

    pub(crate) const fn mode_lease_release_receipt(&self) -> &PreArmCancellationReceiptRef {
        &self.mode_lease_release_receipt
    }

    pub(crate) const fn root_guard_release_receipt(&self) -> &PreArmCancellationReceiptRef {
        &self.root_guard_release_receipt
    }

    pub(crate) const fn recovery_finalization_receipt(&self) -> &PreArmCancellationReceiptRef {
        &self.recovery_finalization_receipt
    }

    pub(crate) const fn receipt_plan_digest(&self) -> &Sha256Digest {
        &self.receipt_plan_digest
    }

    fn selective_update_plan_digest(&self) -> &Sha256Digest {
        &self.bound_selective_update_plan_digest
    }

    fn bound_guard_acquisition_flags(&self) -> (bool, bool) {
        (self.bound_acquire_root_guard, self.bound_acquire_mode_lease)
    }

    fn ordered_refs_with_kinds(
        &self,
    ) -> Vec<(&PreArmCancellationReceiptRef, PreArmCancellationEffectKind)> {
        let mut values = Vec::with_capacity(7);
        values.push((
            &self.root_guard_acquisition_receipt,
            PreArmCancellationEffectKind::RootGuardAcquire,
        ));
        values.push((
            &self.mode_lease_acquisition_receipt,
            PreArmCancellationEffectKind::ModeLeaseAcquire,
        ));
        if let Some(receipt) = &self.selective_update_effect_receipt {
            values.push((
                receipt,
                PreArmCancellationEffectKind::SelectiveOriginalUpdate,
            ));
        }
        values.extend([
            (
                &self.cancellation_persistence_receipt,
                PreArmCancellationEffectKind::AuthorizationCancellation,
            ),
            (
                &self.mode_lease_release_receipt,
                PreArmCancellationEffectKind::ModeLeaseRelease,
            ),
            (
                &self.root_guard_release_receipt,
                PreArmCancellationEffectKind::RootGuardRelease,
            ),
            (
                &self.recovery_finalization_receipt,
                PreArmCancellationEffectKind::RecoveryFinalization,
            ),
        ]);
        values
    }
}

fn validate_receipt_ref_slot(
    actual: &PreArmCancellationReceiptRef,
    expected_kind: PreArmCancellationEffectKind,
    prior: Option<&PreArmCancellationReceiptRef>,
) -> Result<(), PreArmRecoveryContractError> {
    if actual.effect_kind() != expected_kind {
        return Err(PreArmRecoveryContractError(
            "receipt plan ref has the wrong effect kind",
        ));
    }
    match prior {
        Some(expected) if actual != expected => Err(PreArmRecoveryContractError(
            "receipt plan failed to copy an immutable prior-operation ref",
        )),
        Some(_) => Ok(()),
        None if actual.source() != PreArmCancellationReceiptSource::FinalizationPlan => {
            Err(PreArmRecoveryContractError(
                "missing prior effect requires a future finalization-plan ref",
            ))
        }
        None => Ok(()),
    }
}

fn validate_finalization_receipt_intents(
    receipt_plan: &PreArmCancellationReceiptPlan,
    finalization_attempt_id: &UnicaId,
    observation: &PreArmCancellationEffectObservation,
    expected_postconditions: &PreArmCancellationExpectedPostconditionDigests,
) -> Result<(), PreArmRecoveryContractError> {
    let refs = receipt_plan.ordered_refs_with_kinds();
    let finalization_refs: Vec<_> = refs
        .iter()
        .copied()
        .filter(|(receipt, _)| {
            receipt.source() == PreArmCancellationReceiptSource::FinalizationPlan
        })
        .collect();
    expected_postconditions.validate_exact_effects(finalization_refs.iter().copied())?;
    for (receipt, effect_kind) in finalization_refs {
        let expected_postcondition_digest =
            expected_postconditions
                .get(effect_kind)
                .ok_or(PreArmRecoveryContractError(
                    "finalization effect has no expected postcondition binding",
                ))?;
        let expected_intent = prearm_digest(
            &FinalizationPlanEffectIntentDigestRecord {
                effect_kind,
                finalization_attempt_id: finalization_attempt_id.clone(),
                support_action_id: observation.support_action_id().clone(),
                expected_support_action_digest: observation
                    .expected_support_action_digest()
                    .clone(),
                approved_cancellation_digest: observation.approved_cancellation_digest().clone(),
                effect_observation_digest: observation.observation_digest().clone(),
                manual_target_mode: observation.manual_target_mode(),
                selective_update_plan_digest: observation.selective_update_plan_digest().clone(),
                expected_postcondition_digest: expected_postcondition_digest.clone(),
            },
            "finalization-plan effect intent digest failed",
        )?;
        if receipt.effect_intent_digest() != &expected_intent {
            return Err(PreArmRecoveryContractError(
                "finalization receipt intent disagrees with its exact effect postcondition",
            ));
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) enum PreArmCancellationFinalizationRecheckMode {
    ReplannableBeforeUpdate,
    ProtectedUpdateReady,
    ReleaseOnlyAfterPersistence,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
enum ReplannableSourceProgressStage {
    NoGuard,
    RootHeldBeforeLease,
    RootReleasedBeforeLease,
    GuardsHeldBeforeUpdate,
    ModeReleasedBeforeUpdateRootHeld,
    GuardsReleasedBeforeUpdate,
}

impl TryFrom<PreArmCancellationEffectProgressStage> for ReplannableSourceProgressStage {
    type Error = PreArmRecoveryContractError;

    fn try_from(value: PreArmCancellationEffectProgressStage) -> Result<Self, Self::Error> {
        match value {
            PreArmCancellationEffectProgressStage::NoGuard => Ok(Self::NoGuard),
            PreArmCancellationEffectProgressStage::RootHeldBeforeLease => {
                Ok(Self::RootHeldBeforeLease)
            }
            PreArmCancellationEffectProgressStage::RootReleasedBeforeLease => {
                Ok(Self::RootReleasedBeforeLease)
            }
            PreArmCancellationEffectProgressStage::GuardsHeldBeforeUpdate => {
                Ok(Self::GuardsHeldBeforeUpdate)
            }
            PreArmCancellationEffectProgressStage::ModeReleasedBeforeUpdateRootHeld => {
                Ok(Self::ModeReleasedBeforeUpdateRootHeld)
            }
            PreArmCancellationEffectProgressStage::GuardsReleasedBeforeUpdate => {
                Ok(Self::GuardsReleasedBeforeUpdate)
            }
            _ => Err(PreArmRecoveryContractError(
                "replannable policy has an update-ready or persisted source stage",
            )),
        }
    }
}

impl From<ReplannableSourceProgressStage> for PreArmCancellationEffectProgressStage {
    fn from(value: ReplannableSourceProgressStage) -> Self {
        match value {
            ReplannableSourceProgressStage::NoGuard => Self::NoGuard,
            ReplannableSourceProgressStage::RootHeldBeforeLease => Self::RootHeldBeforeLease,
            ReplannableSourceProgressStage::RootReleasedBeforeLease => {
                Self::RootReleasedBeforeLease
            }
            ReplannableSourceProgressStage::GuardsHeldBeforeUpdate => Self::GuardsHeldBeforeUpdate,
            ReplannableSourceProgressStage::ModeReleasedBeforeUpdateRootHeld => {
                Self::ModeReleasedBeforeUpdateRootHeld
            }
            ReplannableSourceProgressStage::GuardsReleasedBeforeUpdate => {
                Self::GuardsReleasedBeforeUpdate
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
enum ReleaseOnlySourceProgressStage {
    #[serde(rename = "cancellationPersistedGuardsHeld")]
    GuardsHeld,
    #[serde(rename = "cancellationPersistedModeReleased")]
    ModeReleased,
    #[serde(rename = "cancellationPersistedReleased")]
    Released,
}

impl From<ReleaseOnlySourceProgressStage> for PreArmCancellationEffectProgressStage {
    fn from(value: ReleaseOnlySourceProgressStage) -> Self {
        match value {
            ReleaseOnlySourceProgressStage::GuardsHeld => Self::CancellationPersistedGuardsHeld,
            ReleaseOnlySourceProgressStage::ModeReleased => Self::CancellationPersistedModeReleased,
            ReleaseOnlySourceProgressStage::Released => Self::CancellationPersistedReleased,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AllowedNonRootTailClassifications;

impl Serialize for AllowedNonRootTailClassifications {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        [
            RepositoryHistoryPartitionClassification::UnrelatedRoutine,
            RepositoryHistoryPartitionClassification::RelevantRoutine,
        ]
        .serialize(serializer)
    }
}

impl JsonSchema for AllowedNonRootTailClassifications {
    fn inline_schema() -> bool {
        true
    }

    fn schema_name() -> Cow<'static, str> {
        "PreArmAllowedNonRootTailClassifications".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "array",
            "prefixItems": [
                { "type": "string", "const": "unrelatedRoutine" },
                { "type": "string", "const": "relevantRoutine" }
            ],
            "items": false,
            "minItems": 2,
            "maxItems": 2,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AllowedReleasedTailClassifications;

impl Serialize for AllowedReleasedTailClassifications {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        [
            RepositoryHistoryPartitionClassification::UnrelatedRoutine,
            RepositoryHistoryPartitionClassification::RelevantRoutine,
            RepositoryHistoryPartitionClassification::ExternalSupport,
            RepositoryHistoryPartitionClassification::PreArmExternal,
        ]
        .serialize(serializer)
    }
}

impl JsonSchema for AllowedReleasedTailClassifications {
    fn inline_schema() -> bool {
        true
    }

    fn schema_name() -> Cow<'static, str> {
        "PreArmAllowedReleasedTailClassifications".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "array",
            "prefixItems": [
                { "type": "string", "const": "unrelatedRoutine" },
                { "type": "string", "const": "relevantRoutine" },
                { "type": "string", "const": "externalSupport" },
                { "type": "string", "const": "preArmExternal" }
            ],
            "items": false,
            "minItems": 4,
            "maxItems": 4,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ReplannableBeforeUpdatePolicyDigestRecord {
    mode: ReplannableBeforeUpdateMode,
    source_progress_stage: ReplannableSourceProgressStage,
    continuously_held_root: bool,
    continuously_held_mode_lease: bool,
    expected_history_through_cursor: RepositoryHistoryCursor,
    expected_history_partition_digest: Sha256Digest,
    expected_original_fingerprint: Sha256Digest,
    expected_support_graph_digest: Sha256Digest,
    pre_arm_freeze_digest: Sha256Digest,
}

wire_literal!(ReplannableBeforeUpdateMode, "replannableBeforeUpdate");

impl contract_digest_record_sealed::Sealed for ReplannableBeforeUpdatePolicyDigestRecord {}
impl ContractDigestRecord for ReplannableBeforeUpdatePolicyDigestRecord {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReplannableBeforeUpdatePolicy {
    mode: ReplannableBeforeUpdateMode,
    source_progress_stage: ReplannableSourceProgressStage,
    continuously_held_root: bool,
    continuously_held_mode_lease: bool,
    expected_history_through_cursor: RepositoryHistoryCursor,
    expected_history_partition_digest: Sha256Digest,
    expected_original_fingerprint: Sha256Digest,
    expected_support_graph_digest: Sha256Digest,
    pre_arm_freeze_digest: Sha256Digest,
    policy_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ProtectedUpdateReadyPolicyDigestRecord {
    mode: ProtectedUpdateReadyMode,
    source_progress_stage: UpdateReadyGuardsHeldStage,
    expected_history_through_cursor: RepositoryHistoryCursor,
    expected_history_partition_digest: Sha256Digest,
    expected_original_fingerprint: Sha256Digest,
    expected_support_graph_digest: Sha256Digest,
    pre_arm_freeze_digest: Sha256Digest,
    allowed_non_root_tail_classifications: AllowedNonRootTailClassifications,
    always_select_relevant_advance_phase: PreArmTrueLiteral,
}

wire_literal!(ProtectedUpdateReadyMode, "protectedUpdateReady");
wire_literal!(UpdateReadyGuardsHeldStage, "updateReadyGuardsHeld");

impl contract_digest_record_sealed::Sealed for ProtectedUpdateReadyPolicyDigestRecord {}
impl ContractDigestRecord for ProtectedUpdateReadyPolicyDigestRecord {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ProtectedUpdateReadyPolicy {
    mode: ProtectedUpdateReadyMode,
    source_progress_stage: UpdateReadyGuardsHeldStage,
    expected_history_through_cursor: RepositoryHistoryCursor,
    expected_history_partition_digest: Sha256Digest,
    expected_original_fingerprint: Sha256Digest,
    expected_support_graph_digest: Sha256Digest,
    pre_arm_freeze_digest: Sha256Digest,
    allowed_non_root_tail_classifications: AllowedNonRootTailClassifications,
    always_select_relevant_advance_phase: PreArmTrueLiteral,
    policy_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ReleaseOnlyAfterPersistencePolicyDigestRecord {
    mode: ReleaseOnlyAfterPersistenceMode,
    source_progress_stage: ReleaseOnlySourceProgressStage,
    persisted_history_through_cursor: RepositoryHistoryCursor,
    persisted_history_partition_digest: Sha256Digest,
    pre_arm_freeze_digest: Sha256Digest,
    allowed_tail_classifications: ReleaseTailClassifications,
    always_select_relevant_advance_phase: PreArmTrueLiteral,
}

wire_literal!(
    ReleaseOnlyAfterPersistenceMode,
    "releaseOnlyAfterPersistence"
);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(untagged)]
enum ReleaseTailClassifications {
    RootHeld(AllowedNonRootTailClassifications),
    FullyReleased(AllowedReleasedTailClassifications),
}

impl JsonSchema for ReleaseTailClassifications {
    fn schema_name() -> Cow<'static, str> {
        "PreArmReleaseTailClassifications".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        super::schema::one_of_schema(vec![
            generator.subschema_for::<AllowedNonRootTailClassifications>(),
            generator.subschema_for::<AllowedReleasedTailClassifications>(),
        ])
    }
}

impl contract_digest_record_sealed::Sealed for ReleaseOnlyAfterPersistencePolicyDigestRecord {}
impl ContractDigestRecord for ReleaseOnlyAfterPersistencePolicyDigestRecord {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReleaseOnlyAfterPersistencePolicy {
    mode: ReleaseOnlyAfterPersistenceMode,
    source_progress_stage: ReleaseOnlySourceProgressStage,
    persisted_history_through_cursor: RepositoryHistoryCursor,
    persisted_history_partition_digest: Sha256Digest,
    pre_arm_freeze_digest: Sha256Digest,
    allowed_tail_classifications: ReleaseTailClassifications,
    always_select_relevant_advance_phase: PreArmTrueLiteral,
    policy_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
enum PreArmCancellationFinalizationRecheckPolicyRecord {
    ReplannableBeforeUpdate(ReplannableBeforeUpdatePolicy),
    ProtectedUpdateReady(ProtectedUpdateReadyPolicy),
    ReleaseOnlyAfterPersistence(ReleaseOnlyAfterPersistencePolicy),
}

impl JsonSchema for PreArmCancellationFinalizationRecheckPolicyRecord {
    fn schema_name() -> Cow<'static, str> {
        "PreArmCancellationFinalizationRecheckPolicyRecord".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        super::schema::one_of_schema(vec![
            generator.subschema_for::<ReplannableBeforeUpdatePolicy>(),
            generator.subschema_for::<ProtectedUpdateReadyPolicy>(),
            generator.subschema_for::<ReleaseOnlyAfterPersistencePolicy>(),
        ])
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PreArmCancellationFinalizationRecheckPolicyAuthorityKind {
    ReplannableBeforeUpdate {
        source_progress_stage: PreArmCancellationEffectProgressStage,
        continuously_held_root: bool,
        continuously_held_mode_lease: bool,
        history_partition: ValidatedRepositoryHistoryPartition,
        expected_original_fingerprint: Sha256Digest,
        expected_support_graph_digest: Sha256Digest,
        pre_arm_freeze_digest: Sha256Digest,
    },
    ProtectedUpdateReady {
        history_partition: ValidatedRepositoryHistoryPartition,
        expected_original_fingerprint: Sha256Digest,
        expected_support_graph_digest: Sha256Digest,
        pre_arm_freeze_digest: Sha256Digest,
    },
    ReleaseOnlyAfterPersistence {
        source_progress_stage: PreArmCancellationEffectProgressStage,
        history_partition: ValidatedRepositoryHistoryPartition,
        pre_arm_freeze_digest: Sha256Digest,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PreArmCancellationFinalizationRecheckPolicyAuthority {
    kind: PreArmCancellationFinalizationRecheckPolicyAuthorityKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct PreArmCancellationFinalizationRecheckPolicy(
    PreArmCancellationFinalizationRecheckPolicyRecord,
);

impl JsonSchema for PreArmCancellationFinalizationRecheckPolicy {
    fn schema_name() -> Cow<'static, str> {
        "PreArmCancellationFinalizationRecheckPolicy".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        PreArmCancellationFinalizationRecheckPolicyRecord::json_schema(generator)
    }
}

impl PreArmCancellationFinalizationRecheckPolicy {
    pub(crate) fn new(
        authority: PreArmCancellationFinalizationRecheckPolicyAuthority,
    ) -> Result<Self, PreArmRecoveryContractError> {
        match authority.kind {
            PreArmCancellationFinalizationRecheckPolicyAuthorityKind::ReplannableBeforeUpdate {
                source_progress_stage,
                continuously_held_root,
                continuously_held_mode_lease,
                history_partition,
                expected_original_fingerprint,
                expected_support_graph_digest,
                pre_arm_freeze_digest,
            } => {
                let source_progress_stage = source_progress_stage.try_into()?;
                let record = ReplannableBeforeUpdatePolicyDigestRecord {
                    mode: ReplannableBeforeUpdateMode::Value,
                    source_progress_stage,
                    continuously_held_root,
                    continuously_held_mode_lease,
                    expected_history_through_cursor: history_partition.through_inclusive().clone(),
                    expected_history_partition_digest: history_partition.partition_digest().clone(),
                    expected_original_fingerprint,
                    expected_support_graph_digest,
                    pre_arm_freeze_digest,
                };
                let policy_digest = prearm_digest(&record, "replannable policy digest failed")?;
                Ok(Self(
                    PreArmCancellationFinalizationRecheckPolicyRecord::ReplannableBeforeUpdate(
                        ReplannableBeforeUpdatePolicy {
                            mode: record.mode,
                            source_progress_stage: record.source_progress_stage,
                            continuously_held_root: record.continuously_held_root,
                            continuously_held_mode_lease: record.continuously_held_mode_lease,
                            expected_history_through_cursor: record.expected_history_through_cursor,
                            expected_history_partition_digest: record
                                .expected_history_partition_digest,
                            expected_original_fingerprint: record.expected_original_fingerprint,
                            expected_support_graph_digest: record.expected_support_graph_digest,
                            pre_arm_freeze_digest: record.pre_arm_freeze_digest,
                            policy_digest,
                        },
                    ),
                ))
            }
            PreArmCancellationFinalizationRecheckPolicyAuthorityKind::ProtectedUpdateReady {
                history_partition,
                expected_original_fingerprint,
                expected_support_graph_digest,
                pre_arm_freeze_digest,
            } => {
                let record = ProtectedUpdateReadyPolicyDigestRecord {
                    mode: ProtectedUpdateReadyMode::Value,
                    source_progress_stage: UpdateReadyGuardsHeldStage::Value,
                    expected_history_through_cursor: history_partition.through_inclusive().clone(),
                    expected_history_partition_digest: history_partition.partition_digest().clone(),
                    expected_original_fingerprint,
                    expected_support_graph_digest,
                    pre_arm_freeze_digest,
                    allowed_non_root_tail_classifications: AllowedNonRootTailClassifications,
                    always_select_relevant_advance_phase: PreArmTrueLiteral,
                };
                let policy_digest =
                    prearm_digest(&record, "protected update-ready policy digest failed")?;
                Ok(Self(
                    PreArmCancellationFinalizationRecheckPolicyRecord::ProtectedUpdateReady(
                        ProtectedUpdateReadyPolicy {
                            mode: record.mode,
                            source_progress_stage: record.source_progress_stage,
                            expected_history_through_cursor: record.expected_history_through_cursor,
                            expected_history_partition_digest: record
                                .expected_history_partition_digest,
                            expected_original_fingerprint: record.expected_original_fingerprint,
                            expected_support_graph_digest: record.expected_support_graph_digest,
                            pre_arm_freeze_digest: record.pre_arm_freeze_digest,
                            allowed_non_root_tail_classifications: record
                                .allowed_non_root_tail_classifications,
                            always_select_relevant_advance_phase: record
                                .always_select_relevant_advance_phase,
                            policy_digest,
                        },
                    ),
                ))
            }
            PreArmCancellationFinalizationRecheckPolicyAuthorityKind::ReleaseOnlyAfterPersistence {
                source_progress_stage,
                history_partition,
                pre_arm_freeze_digest,
            } => {
                let (source_progress_stage, allowed_tail_classifications) =
                    match source_progress_stage {
                    PreArmCancellationEffectProgressStage::CancellationPersistedGuardsHeld => (
                        ReleaseOnlySourceProgressStage::GuardsHeld,
                        ReleaseTailClassifications::RootHeld(
                            AllowedNonRootTailClassifications,
                        ),
                    ),
                    PreArmCancellationEffectProgressStage::CancellationPersistedModeReleased => (
                        ReleaseOnlySourceProgressStage::ModeReleased,
                        ReleaseTailClassifications::RootHeld(
                            AllowedNonRootTailClassifications,
                        ),
                    ),
                    PreArmCancellationEffectProgressStage::CancellationPersistedReleased => {
                        (ReleaseOnlySourceProgressStage::Released, ReleaseTailClassifications::FullyReleased(
                            AllowedReleasedTailClassifications,
                        ))
                    }
                    _ => {
                        return Err(PreArmRecoveryContractError(
                            "release-only policy requires a persisted cancellation stage",
                        ));
                    }
                };
                let record = ReleaseOnlyAfterPersistencePolicyDigestRecord {
                    mode: ReleaseOnlyAfterPersistenceMode::Value,
                    source_progress_stage,
                    persisted_history_through_cursor: history_partition.through_inclusive().clone(),
                    persisted_history_partition_digest: history_partition
                        .partition_digest()
                        .clone(),
                    pre_arm_freeze_digest,
                    allowed_tail_classifications,
                    always_select_relevant_advance_phase: PreArmTrueLiteral,
                };
                let policy_digest = prearm_digest(&record, "release-only policy digest failed")?;
                Ok(Self(
                    PreArmCancellationFinalizationRecheckPolicyRecord::ReleaseOnlyAfterPersistence(
                        ReleaseOnlyAfterPersistencePolicy {
                            mode: record.mode,
                            source_progress_stage: record.source_progress_stage,
                            persisted_history_through_cursor: record
                                .persisted_history_through_cursor,
                            persisted_history_partition_digest: record
                                .persisted_history_partition_digest,
                            pre_arm_freeze_digest: record.pre_arm_freeze_digest,
                            allowed_tail_classifications: record.allowed_tail_classifications,
                            always_select_relevant_advance_phase: record
                                .always_select_relevant_advance_phase,
                            policy_digest,
                        },
                    ),
                ))
            }
        }
    }

    pub(crate) const fn mode(&self) -> PreArmCancellationFinalizationRecheckMode {
        match &self.0 {
            PreArmCancellationFinalizationRecheckPolicyRecord::ReplannableBeforeUpdate(_) => {
                PreArmCancellationFinalizationRecheckMode::ReplannableBeforeUpdate
            }
            PreArmCancellationFinalizationRecheckPolicyRecord::ProtectedUpdateReady(_) => {
                PreArmCancellationFinalizationRecheckMode::ProtectedUpdateReady
            }
            PreArmCancellationFinalizationRecheckPolicyRecord::ReleaseOnlyAfterPersistence(_) => {
                PreArmCancellationFinalizationRecheckMode::ReleaseOnlyAfterPersistence
            }
        }
    }

    pub(crate) fn source_progress_stage(&self) -> PreArmCancellationEffectProgressStage {
        match &self.0 {
            PreArmCancellationFinalizationRecheckPolicyRecord::ReplannableBeforeUpdate(value) => {
                value.source_progress_stage.into()
            }
            PreArmCancellationFinalizationRecheckPolicyRecord::ProtectedUpdateReady(_) => {
                PreArmCancellationEffectProgressStage::UpdateReadyGuardsHeld
            }
            PreArmCancellationFinalizationRecheckPolicyRecord::ReleaseOnlyAfterPersistence(
                value,
            ) => value.source_progress_stage.into(),
        }
    }

    pub(crate) const fn policy_digest(&self) -> &Sha256Digest {
        match &self.0 {
            PreArmCancellationFinalizationRecheckPolicyRecord::ReplannableBeforeUpdate(value) => {
                &value.policy_digest
            }
            PreArmCancellationFinalizationRecheckPolicyRecord::ProtectedUpdateReady(value) => {
                &value.policy_digest
            }
            PreArmCancellationFinalizationRecheckPolicyRecord::ReleaseOnlyAfterPersistence(
                value,
            ) => &value.policy_digest,
        }
    }

    fn expected_partition_binding(&self) -> (&RepositoryHistoryCursor, &Sha256Digest) {
        match &self.0 {
            PreArmCancellationFinalizationRecheckPolicyRecord::ReplannableBeforeUpdate(value) => (
                &value.expected_history_through_cursor,
                &value.expected_history_partition_digest,
            ),
            PreArmCancellationFinalizationRecheckPolicyRecord::ProtectedUpdateReady(value) => (
                &value.expected_history_through_cursor,
                &value.expected_history_partition_digest,
            ),
            PreArmCancellationFinalizationRecheckPolicyRecord::ReleaseOnlyAfterPersistence(
                value,
            ) => (
                &value.persisted_history_through_cursor,
                &value.persisted_history_partition_digest,
            ),
        }
    }

    fn continuity(&self) -> Option<(bool, bool)> {
        match &self.0 {
            PreArmCancellationFinalizationRecheckPolicyRecord::ReplannableBeforeUpdate(value) => {
                Some((
                    value.continuously_held_root,
                    value.continuously_held_mode_lease,
                ))
            }
            _ => None,
        }
    }

    fn expected_recheck_state_binding(&self) -> Option<(&Sha256Digest, &Sha256Digest)> {
        match &self.0 {
            PreArmCancellationFinalizationRecheckPolicyRecord::ReplannableBeforeUpdate(value) => {
                Some((
                    &value.expected_original_fingerprint,
                    &value.expected_support_graph_digest,
                ))
            }
            PreArmCancellationFinalizationRecheckPolicyRecord::ProtectedUpdateReady(value) => {
                Some((
                    &value.expected_original_fingerprint,
                    &value.expected_support_graph_digest,
                ))
            }
            PreArmCancellationFinalizationRecheckPolicyRecord::ReleaseOnlyAfterPersistence(_) => {
                None
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) enum PreArmCancellationFinalizationReplanMismatchKind {
    NonRootRoutineTailAdvanced,
    RootOrSupportVersionChanged,
    OriginalTargetChanged,
    SupportGraphChanged,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) enum PreArmCancellationFinalizationCapabilityBreachKind {
    HistoryGap,
    RootOrSupportVersionChanged,
    OriginalTargetChanged,
    SupportGraphChanged,
    RootGuardLost,
    ModeLeaseLost,
    ReceiptOwnerMismatch,
}

macro_rules! canonical_non_empty_enum_set {
    ($name:ident, $item:ty, $schema_name:literal) => {
        #[derive(Debug, Clone, PartialEq, Eq, Serialize)]
        #[serde(transparent)]
        struct $name(Vec<$item>);

        impl $name {
            fn new(values: Vec<$item>) -> Result<Self, PreArmRecoveryContractError> {
                if values.is_empty()
                    || values.len() > MAX_PREARM_ITEMS
                    || values.windows(2).any(|pair| pair[0] >= pair[1])
                {
                    return Err(PreArmRecoveryContractError(
                        "mismatch kinds must be non-empty, unique, and canonical",
                    ));
                }
                Ok(Self(values))
            }

            fn as_slice(&self) -> &[$item] {
                &self.0
            }
        }

        impl JsonSchema for $name {
            fn schema_name() -> Cow<'static, str> {
                $schema_name.into()
            }

            fn json_schema(generator: &mut SchemaGenerator) -> Schema {
                json_schema!({
                    "type": "array",
                    "minItems": 1,
                    "maxItems": MAX_PREARM_ITEMS,
                    "uniqueItems": true,
                    "items": generator.subschema_for::<$item>(),
                })
            }
        }
    };
}

canonical_non_empty_enum_set!(
    CanonicalReplanMismatchKinds,
    PreArmCancellationFinalizationReplanMismatchKind,
    "PreArmCancellationFinalizationReplanMismatchKinds"
);
canonical_non_empty_enum_set!(
    CanonicalCapabilityBreachKinds,
    PreArmCancellationFinalizationCapabilityBreachKind,
    "PreArmCancellationFinalizationCapabilityBreachKinds"
);

wire_literal!(MatchedOutcome, "matched");
wire_literal!(SafeTailExtendedOutcome, "safeTailExtended");
wire_literal!(ReleaseTailObservedOutcome, "releaseTailObserved");
wire_literal!(ReplanRequiredOutcome, "replanRequired");
wire_literal!(CapabilityBreachOutcome, "capabilityBreach");

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct MatchedRecheckEvidenceDigestRecord {
    outcome: MatchedOutcome,
    observed_history_partition: ValidatedRepositoryHistoryPartition,
    observed_original_fingerprint: Sha256Digest,
    observed_support_graph_digest: Sha256Digest,
}

impl contract_digest_record_sealed::Sealed for MatchedRecheckEvidenceDigestRecord {}
impl ContractDigestRecord for MatchedRecheckEvidenceDigestRecord {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MatchedRecheckEvidence {
    outcome: MatchedOutcome,
    observed_history_partition: ValidatedRepositoryHistoryPartition,
    observed_original_fingerprint: Sha256Digest,
    observed_support_graph_digest: Sha256Digest,
    evidence_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SafeTailExtendedRecheckEvidenceDigestRecord {
    outcome: SafeTailExtendedOutcome,
    base_history_partition: ValidatedRepositoryHistoryPartition,
    appended_non_root_history_partition: ValidatedRepositoryHistoryPartition,
    combined_history_partition: ValidatedRepositoryHistoryPartition,
    observed_original_fingerprint: Sha256Digest,
    observed_support_graph_digest: Sha256Digest,
    relevant_advance_selected: PreArmTrueLiteral,
}

impl contract_digest_record_sealed::Sealed for SafeTailExtendedRecheckEvidenceDigestRecord {}
impl ContractDigestRecord for SafeTailExtendedRecheckEvidenceDigestRecord {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SafeTailExtendedRecheckEvidence {
    outcome: SafeTailExtendedOutcome,
    base_history_partition: ValidatedRepositoryHistoryPartition,
    appended_non_root_history_partition: ValidatedRepositoryHistoryPartition,
    combined_history_partition: ValidatedRepositoryHistoryPartition,
    observed_original_fingerprint: Sha256Digest,
    observed_support_graph_digest: Sha256Digest,
    relevant_advance_selected: PreArmTrueLiteral,
    evidence_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ReleaseTailObservedRecheckEvidenceDigestRecord {
    outcome: ReleaseTailObservedOutcome,
    persisted_history_partition: ValidatedRepositoryHistoryPartition,
    appended_history_partition: ValidatedRepositoryHistoryPartition,
    observed_original_fingerprint: Sha256Digest,
    observed_support_graph_digest: Sha256Digest,
    relevant_advance_selected: PreArmTrueLiteral,
}

impl contract_digest_record_sealed::Sealed for ReleaseTailObservedRecheckEvidenceDigestRecord {}
impl ContractDigestRecord for ReleaseTailObservedRecheckEvidenceDigestRecord {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReleaseTailObservedRecheckEvidence {
    outcome: ReleaseTailObservedOutcome,
    persisted_history_partition: ValidatedRepositoryHistoryPartition,
    appended_history_partition: ValidatedRepositoryHistoryPartition,
    observed_original_fingerprint: Sha256Digest,
    observed_support_graph_digest: Sha256Digest,
    relevant_advance_selected: PreArmTrueLiteral,
    evidence_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ReplanRequiredRecheckEvidenceDigestRecord {
    outcome: ReplanRequiredOutcome,
    mismatch_kinds: CanonicalReplanMismatchKinds,
    refreshed_history_partition: ValidatedRepositoryHistoryPartition,
    observed_original_fingerprint: Sha256Digest,
    observed_support_graph_digest: Sha256Digest,
}

impl contract_digest_record_sealed::Sealed for ReplanRequiredRecheckEvidenceDigestRecord {}
impl ContractDigestRecord for ReplanRequiredRecheckEvidenceDigestRecord {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReplanRequiredRecheckEvidence {
    outcome: ReplanRequiredOutcome,
    mismatch_kinds: CanonicalReplanMismatchKinds,
    refreshed_history_partition: ValidatedRepositoryHistoryPartition,
    observed_original_fingerprint: Sha256Digest,
    observed_support_graph_digest: Sha256Digest,
    evidence_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CapabilityBreachRecheckEvidenceDigestRecord {
    outcome: CapabilityBreachOutcome,
    mismatch_kinds: CanonicalCapabilityBreachKinds,
    #[serde(skip_serializing_if = "Option::is_none")]
    observed_history_partition: Option<ValidatedRepositoryHistoryPartition>,
    #[serde(skip_serializing_if = "Option::is_none")]
    observed_original_fingerprint: Option<Sha256Digest>,
    #[serde(skip_serializing_if = "Option::is_none")]
    observed_support_graph_digest: Option<Sha256Digest>,
}

impl contract_digest_record_sealed::Sealed for CapabilityBreachRecheckEvidenceDigestRecord {}
impl ContractDigestRecord for CapabilityBreachRecheckEvidenceDigestRecord {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CapabilityBreachRecheckEvidence {
    outcome: CapabilityBreachOutcome,
    mismatch_kinds: CanonicalCapabilityBreachKinds,
    #[serde(skip_serializing_if = "Option::is_none")]
    observed_history_partition: Option<ValidatedRepositoryHistoryPartition>,
    #[serde(skip_serializing_if = "Option::is_none")]
    observed_original_fingerprint: Option<Sha256Digest>,
    #[serde(skip_serializing_if = "Option::is_none")]
    observed_support_graph_digest: Option<Sha256Digest>,
    evidence_digest: Sha256Digest,
}

// Keep every closed oneOf leaf inline so wire and in-memory authority projections stay identical;
// heap indirection or a custom serializer would weaken that invariant.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
enum PreArmCancellationFinalizationRecheckEvidenceRecord {
    Matched(MatchedRecheckEvidence),
    SafeTailExtended(SafeTailExtendedRecheckEvidence),
    ReleaseTailObserved(ReleaseTailObservedRecheckEvidence),
    ReplanRequired(ReplanRequiredRecheckEvidence),
    CapabilityBreach(CapabilityBreachRecheckEvidence),
}

impl JsonSchema for PreArmCancellationFinalizationRecheckEvidenceRecord {
    fn schema_name() -> Cow<'static, str> {
        "PreArmCancellationFinalizationRecheckEvidenceRecord".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        super::schema::one_of_schema(vec![
            generator.subschema_for::<MatchedRecheckEvidence>(),
            generator.subschema_for::<SafeTailExtendedRecheckEvidence>(),
            generator.subschema_for::<ReleaseTailObservedRecheckEvidence>(),
            generator.subschema_for::<ReplanRequiredRecheckEvidence>(),
            generator.subschema_for::<CapabilityBreachRecheckEvidence>(),
        ])
    }
}

// Authority mirrors the exact wire oneOf; keeping the same inline payloads avoids a second shape.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Eq)]
enum PreArmCancellationFinalizationRecheckEvidenceAuthorityKind {
    Matched {
        observed_history_partition: ValidatedRepositoryHistoryPartition,
        observed_original_fingerprint: Sha256Digest,
        observed_support_graph_digest: Sha256Digest,
    },
    SafeTailExtended {
        base_history_partition: ValidatedRepositoryHistoryPartition,
        appended_non_root_history_partition: ValidatedRepositoryHistoryPartition,
        combined_history_partition: ValidatedRepositoryHistoryPartition,
        observed_original_fingerprint: Sha256Digest,
        observed_support_graph_digest: Sha256Digest,
    },
    ReleaseTailObserved {
        persisted_history_partition: ValidatedRepositoryHistoryPartition,
        appended_history_partition: ValidatedRepositoryHistoryPartition,
        observed_original_fingerprint: Sha256Digest,
        observed_support_graph_digest: Sha256Digest,
    },
    ReplanRequired {
        mismatch_kinds: CanonicalReplanMismatchKinds,
        refreshed_history_partition: ValidatedRepositoryHistoryPartition,
        observed_original_fingerprint: Sha256Digest,
        observed_support_graph_digest: Sha256Digest,
    },
    CapabilityBreach {
        mismatch_kinds: CanonicalCapabilityBreachKinds,
        observed_history_partition: Option<ValidatedRepositoryHistoryPartition>,
        observed_original_fingerprint: Option<Sha256Digest>,
        observed_support_graph_digest: Option<Sha256Digest>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PreArmCancellationFinalizationRecheckEvidenceAuthority {
    kind: PreArmCancellationFinalizationRecheckEvidenceAuthorityKind,
}

impl PreArmCancellationFinalizationRecheckEvidenceAuthority {
    #[cfg(test)]
    fn matched_test_only(
        observed_history_partition: ValidatedRepositoryHistoryPartition,
        observed_original_fingerprint: Sha256Digest,
        observed_support_graph_digest: Sha256Digest,
    ) -> Self {
        Self {
            kind: PreArmCancellationFinalizationRecheckEvidenceAuthorityKind::Matched {
                observed_history_partition,
                observed_original_fingerprint,
                observed_support_graph_digest,
            },
        }
    }

    #[cfg(test)]
    fn replan_required_test_only(
        mismatch_kinds: Vec<PreArmCancellationFinalizationReplanMismatchKind>,
        refreshed_history_partition: ValidatedRepositoryHistoryPartition,
        observed_original_fingerprint: Sha256Digest,
        observed_support_graph_digest: Sha256Digest,
    ) -> Result<Self, PreArmRecoveryContractError> {
        Ok(Self {
            kind: PreArmCancellationFinalizationRecheckEvidenceAuthorityKind::ReplanRequired {
                mismatch_kinds: CanonicalReplanMismatchKinds::new(mismatch_kinds)?,
                refreshed_history_partition,
                observed_original_fingerprint,
                observed_support_graph_digest,
            },
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PreArmCancellationFinalizationRecheckOutcome {
    Matched,
    SafeTailExtended,
    ReleaseTailObserved,
    ReplanRequired,
    CapabilityBreach,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct PreArmCancellationFinalizationRecheckEvidence(
    PreArmCancellationFinalizationRecheckEvidenceRecord,
);

impl JsonSchema for PreArmCancellationFinalizationRecheckEvidence {
    fn schema_name() -> Cow<'static, str> {
        "PreArmCancellationFinalizationRecheckEvidence".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        PreArmCancellationFinalizationRecheckEvidenceRecord::json_schema(generator)
    }
}

impl PreArmCancellationFinalizationRecheckEvidence {
    pub(crate) fn new(
        authority: PreArmCancellationFinalizationRecheckEvidenceAuthority,
    ) -> Result<Self, PreArmRecoveryContractError> {
        match authority.kind {
            PreArmCancellationFinalizationRecheckEvidenceAuthorityKind::Matched {
                observed_history_partition,
                observed_original_fingerprint,
                observed_support_graph_digest,
            } => {
                let record = MatchedRecheckEvidenceDigestRecord {
                    outcome: MatchedOutcome::Value,
                    observed_history_partition,
                    observed_original_fingerprint,
                    observed_support_graph_digest,
                };
                let evidence_digest = prearm_digest(&record, "matched evidence digest failed")?;
                Ok(Self(
                    PreArmCancellationFinalizationRecheckEvidenceRecord::Matched(
                        MatchedRecheckEvidence {
                            outcome: record.outcome,
                            observed_history_partition: record.observed_history_partition,
                            observed_original_fingerprint: record.observed_original_fingerprint,
                            observed_support_graph_digest: record.observed_support_graph_digest,
                            evidence_digest,
                        },
                    ),
                ))
            }
            PreArmCancellationFinalizationRecheckEvidenceAuthorityKind::SafeTailExtended {
                base_history_partition,
                appended_non_root_history_partition,
                combined_history_partition,
                observed_original_fingerprint,
                observed_support_graph_digest,
            } => {
                if appended_non_root_history_partition.start_cursor()
                    != base_history_partition.through_inclusive()
                    || combined_history_partition.start_cursor()
                        != base_history_partition.start_cursor()
                    || combined_history_partition.through_inclusive()
                        != appended_non_root_history_partition.through_inclusive()
                    || appended_non_root_history_partition.start_cursor()
                        == appended_non_root_history_partition.through_inclusive()
                    || !appended_non_root_history_partition.all_entries_are_one_of(&[
                        RepositoryHistoryPartitionClassification::UnrelatedRoutine,
                        RepositoryHistoryPartitionClassification::RelevantRoutine,
                    ])
                    || !is_exact_partition_concatenation(
                        &base_history_partition,
                        &appended_non_root_history_partition,
                        &combined_history_partition,
                    )?
                {
                    return Err(PreArmRecoveryContractError(
                        "safe-tail evidence lacks an exact non-empty contiguous routine append",
                    ));
                }
                let record = SafeTailExtendedRecheckEvidenceDigestRecord {
                    outcome: SafeTailExtendedOutcome::Value,
                    base_history_partition,
                    appended_non_root_history_partition,
                    combined_history_partition,
                    observed_original_fingerprint,
                    observed_support_graph_digest,
                    relevant_advance_selected: PreArmTrueLiteral,
                };
                let evidence_digest = prearm_digest(&record, "safe-tail evidence digest failed")?;
                Ok(Self(
                    PreArmCancellationFinalizationRecheckEvidenceRecord::SafeTailExtended(
                        SafeTailExtendedRecheckEvidence {
                            outcome: record.outcome,
                            base_history_partition: record.base_history_partition,
                            appended_non_root_history_partition: record
                                .appended_non_root_history_partition,
                            combined_history_partition: record.combined_history_partition,
                            observed_original_fingerprint: record.observed_original_fingerprint,
                            observed_support_graph_digest: record.observed_support_graph_digest,
                            relevant_advance_selected: record.relevant_advance_selected,
                            evidence_digest,
                        },
                    ),
                ))
            }
            PreArmCancellationFinalizationRecheckEvidenceAuthorityKind::ReleaseTailObserved {
                persisted_history_partition,
                appended_history_partition,
                observed_original_fingerprint,
                observed_support_graph_digest,
            } => {
                if appended_history_partition.start_cursor()
                    != persisted_history_partition.through_inclusive()
                    || !appended_history_partition.all_entries_are_one_of(&[
                        RepositoryHistoryPartitionClassification::UnrelatedRoutine,
                        RepositoryHistoryPartitionClassification::RelevantRoutine,
                        RepositoryHistoryPartitionClassification::ExternalSupport,
                        RepositoryHistoryPartitionClassification::PreArmExternal,
                    ])
                {
                    return Err(PreArmRecoveryContractError(
                        "release-tail evidence is not a contiguous allowed append",
                    ));
                }
                let record = ReleaseTailObservedRecheckEvidenceDigestRecord {
                    outcome: ReleaseTailObservedOutcome::Value,
                    persisted_history_partition,
                    appended_history_partition,
                    observed_original_fingerprint,
                    observed_support_graph_digest,
                    relevant_advance_selected: PreArmTrueLiteral,
                };
                let evidence_digest =
                    prearm_digest(&record, "release-tail evidence digest failed")?;
                Ok(Self(
                    PreArmCancellationFinalizationRecheckEvidenceRecord::ReleaseTailObserved(
                        ReleaseTailObservedRecheckEvidence {
                            outcome: record.outcome,
                            persisted_history_partition: record.persisted_history_partition,
                            appended_history_partition: record.appended_history_partition,
                            observed_original_fingerprint: record.observed_original_fingerprint,
                            observed_support_graph_digest: record.observed_support_graph_digest,
                            relevant_advance_selected: record.relevant_advance_selected,
                            evidence_digest,
                        },
                    ),
                ))
            }
            PreArmCancellationFinalizationRecheckEvidenceAuthorityKind::ReplanRequired {
                mismatch_kinds,
                refreshed_history_partition,
                observed_original_fingerprint,
                observed_support_graph_digest,
            } => {
                let record = ReplanRequiredRecheckEvidenceDigestRecord {
                    outcome: ReplanRequiredOutcome::Value,
                    mismatch_kinds,
                    refreshed_history_partition,
                    observed_original_fingerprint,
                    observed_support_graph_digest,
                };
                let evidence_digest =
                    prearm_digest(&record, "replan-required evidence digest failed")?;
                Ok(Self(
                    PreArmCancellationFinalizationRecheckEvidenceRecord::ReplanRequired(
                        ReplanRequiredRecheckEvidence {
                            outcome: record.outcome,
                            mismatch_kinds: record.mismatch_kinds,
                            refreshed_history_partition: record.refreshed_history_partition,
                            observed_original_fingerprint: record.observed_original_fingerprint,
                            observed_support_graph_digest: record.observed_support_graph_digest,
                            evidence_digest,
                        },
                    ),
                ))
            }
            PreArmCancellationFinalizationRecheckEvidenceAuthorityKind::CapabilityBreach {
                mismatch_kinds,
                observed_history_partition,
                observed_original_fingerprint,
                observed_support_graph_digest,
            } => {
                let record = CapabilityBreachRecheckEvidenceDigestRecord {
                    outcome: CapabilityBreachOutcome::Value,
                    mismatch_kinds,
                    observed_history_partition,
                    observed_original_fingerprint,
                    observed_support_graph_digest,
                };
                let evidence_digest =
                    prearm_digest(&record, "capability-breach evidence digest failed")?;
                Ok(Self(
                    PreArmCancellationFinalizationRecheckEvidenceRecord::CapabilityBreach(
                        CapabilityBreachRecheckEvidence {
                            outcome: record.outcome,
                            mismatch_kinds: record.mismatch_kinds,
                            observed_history_partition: record.observed_history_partition,
                            observed_original_fingerprint: record.observed_original_fingerprint,
                            observed_support_graph_digest: record.observed_support_graph_digest,
                            evidence_digest,
                        },
                    ),
                ))
            }
        }
    }

    pub(crate) const fn outcome(&self) -> PreArmCancellationFinalizationRecheckOutcome {
        match &self.0 {
            PreArmCancellationFinalizationRecheckEvidenceRecord::Matched(_) => {
                PreArmCancellationFinalizationRecheckOutcome::Matched
            }
            PreArmCancellationFinalizationRecheckEvidenceRecord::SafeTailExtended(_) => {
                PreArmCancellationFinalizationRecheckOutcome::SafeTailExtended
            }
            PreArmCancellationFinalizationRecheckEvidenceRecord::ReleaseTailObserved(_) => {
                PreArmCancellationFinalizationRecheckOutcome::ReleaseTailObserved
            }
            PreArmCancellationFinalizationRecheckEvidenceRecord::ReplanRequired(_) => {
                PreArmCancellationFinalizationRecheckOutcome::ReplanRequired
            }
            PreArmCancellationFinalizationRecheckEvidenceRecord::CapabilityBreach(_) => {
                PreArmCancellationFinalizationRecheckOutcome::CapabilityBreach
            }
        }
    }

    pub(crate) const fn evidence_digest(&self) -> &Sha256Digest {
        match &self.0 {
            PreArmCancellationFinalizationRecheckEvidenceRecord::Matched(value) => {
                &value.evidence_digest
            }
            PreArmCancellationFinalizationRecheckEvidenceRecord::SafeTailExtended(value) => {
                &value.evidence_digest
            }
            PreArmCancellationFinalizationRecheckEvidenceRecord::ReleaseTailObserved(value) => {
                &value.evidence_digest
            }
            PreArmCancellationFinalizationRecheckEvidenceRecord::ReplanRequired(value) => {
                &value.evidence_digest
            }
            PreArmCancellationFinalizationRecheckEvidenceRecord::CapabilityBreach(value) => {
                &value.evidence_digest
            }
        }
    }

    fn replan_mismatch_kinds(&self) -> Option<&[PreArmCancellationFinalizationReplanMismatchKind]> {
        match &self.0 {
            PreArmCancellationFinalizationRecheckEvidenceRecord::ReplanRequired(value) => {
                Some(value.mismatch_kinds.as_slice())
            }
            _ => None,
        }
    }

    fn refreshed_history_partition(&self) -> Option<&ValidatedRepositoryHistoryPartition> {
        match &self.0 {
            PreArmCancellationFinalizationRecheckEvidenceRecord::ReplanRequired(value) => {
                Some(&value.refreshed_history_partition)
            }
            _ => None,
        }
    }

    fn observed_state_binding(&self) -> Option<(&Sha256Digest, &Sha256Digest)> {
        match &self.0 {
            PreArmCancellationFinalizationRecheckEvidenceRecord::Matched(value) => Some((
                &value.observed_original_fingerprint,
                &value.observed_support_graph_digest,
            )),
            PreArmCancellationFinalizationRecheckEvidenceRecord::SafeTailExtended(value) => Some((
                &value.observed_original_fingerprint,
                &value.observed_support_graph_digest,
            )),
            PreArmCancellationFinalizationRecheckEvidenceRecord::ReleaseTailObserved(value) => {
                Some((
                    &value.observed_original_fingerprint,
                    &value.observed_support_graph_digest,
                ))
            }
            PreArmCancellationFinalizationRecheckEvidenceRecord::ReplanRequired(value) => Some((
                &value.observed_original_fingerprint,
                &value.observed_support_graph_digest,
            )),
            PreArmCancellationFinalizationRecheckEvidenceRecord::CapabilityBreach(value) => value
                .observed_original_fingerprint
                .as_ref()
                .zip(value.observed_support_graph_digest.as_ref()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) enum PreArmCancellationFinalizationExecutionPathKind {
    Success,
    CapabilityBreachStop,
    RootGuardConflictCompensation,
    ModeLeaseUnavailableBeforeAcquisitionCompensation,
    ModeLeaseUnavailableAfterAcquisitionCompensation,
    RecheckReplanCompensation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
struct UniqueActionIds(Vec<UnicaId>);

impl UniqueActionIds {
    fn new(action_ids: Vec<UnicaId>) -> Result<Self, PreArmRecoveryContractError> {
        let mut seen = BTreeSet::new();
        if action_ids.is_empty()
            || action_ids.len() > MAX_PREARM_ITEMS
            || action_ids
                .iter()
                .any(|value| !seen.insert(value.as_str().to_owned()))
        {
            return Err(PreArmRecoveryContractError(
                "execution-path action IDs must be non-empty and duplicate-free",
            ));
        }
        Ok(Self(action_ids))
    }

    fn as_slice(&self) -> &[UnicaId] {
        &self.0
    }
}

impl JsonSchema for UniqueActionIds {
    fn schema_name() -> Cow<'static, str> {
        "PreArmCancellationFinalizationActionIds".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "array",
            "minItems": 1,
            "maxItems": MAX_PREARM_ITEMS,
            "uniqueItems": true,
            "items": generator.subschema_for::<UnicaId>(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(tag = "pathKind", rename_all = "camelCase", deny_unknown_fields)]
enum PreArmCancellationFinalizationExecutionPathRecord {
    Success { action_ids: UniqueActionIds },
    CapabilityBreachStop { action_ids: UniqueActionIds },
    RootGuardConflictCompensation { action_ids: UniqueActionIds },
    ModeLeaseUnavailableBeforeAcquisitionCompensation { action_ids: UniqueActionIds },
    ModeLeaseUnavailableAfterAcquisitionCompensation { action_ids: UniqueActionIds },
    RecheckReplanCompensation { action_ids: UniqueActionIds },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct PreArmCancellationFinalizationExecutionPath(
    PreArmCancellationFinalizationExecutionPathRecord,
);

impl JsonSchema for PreArmCancellationFinalizationExecutionPath {
    fn schema_name() -> Cow<'static, str> {
        "PreArmCancellationFinalizationExecutionPath".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        PreArmCancellationFinalizationExecutionPathRecord::json_schema(generator)
    }
}

impl PreArmCancellationFinalizationExecutionPath {
    pub(crate) fn new(
        path_kind: PreArmCancellationFinalizationExecutionPathKind,
        action_ids: Vec<UnicaId>,
    ) -> Result<Self, PreArmRecoveryContractError> {
        let action_ids = UniqueActionIds::new(action_ids)?;
        let record = match path_kind {
            PreArmCancellationFinalizationExecutionPathKind::Success => {
                PreArmCancellationFinalizationExecutionPathRecord::Success { action_ids }
            }
            PreArmCancellationFinalizationExecutionPathKind::CapabilityBreachStop => {
                PreArmCancellationFinalizationExecutionPathRecord::CapabilityBreachStop {
                    action_ids,
                }
            }
            PreArmCancellationFinalizationExecutionPathKind::RootGuardConflictCompensation => {
                PreArmCancellationFinalizationExecutionPathRecord::RootGuardConflictCompensation {
                    action_ids,
                }
            }
            PreArmCancellationFinalizationExecutionPathKind::ModeLeaseUnavailableBeforeAcquisitionCompensation => {
                PreArmCancellationFinalizationExecutionPathRecord::ModeLeaseUnavailableBeforeAcquisitionCompensation { action_ids }
            }
            PreArmCancellationFinalizationExecutionPathKind::ModeLeaseUnavailableAfterAcquisitionCompensation => {
                PreArmCancellationFinalizationExecutionPathRecord::ModeLeaseUnavailableAfterAcquisitionCompensation { action_ids }
            }
            PreArmCancellationFinalizationExecutionPathKind::RecheckReplanCompensation => {
                PreArmCancellationFinalizationExecutionPathRecord::RecheckReplanCompensation {
                    action_ids,
                }
            }
        };
        Ok(Self(record))
    }

    pub(crate) const fn path_kind(&self) -> PreArmCancellationFinalizationExecutionPathKind {
        match &self.0 {
            PreArmCancellationFinalizationExecutionPathRecord::Success { .. } => {
                PreArmCancellationFinalizationExecutionPathKind::Success
            }
            PreArmCancellationFinalizationExecutionPathRecord::CapabilityBreachStop { .. } => {
                PreArmCancellationFinalizationExecutionPathKind::CapabilityBreachStop
            }
            PreArmCancellationFinalizationExecutionPathRecord::RootGuardConflictCompensation {
                ..
            } => PreArmCancellationFinalizationExecutionPathKind::RootGuardConflictCompensation,
            PreArmCancellationFinalizationExecutionPathRecord::ModeLeaseUnavailableBeforeAcquisitionCompensation { .. } => PreArmCancellationFinalizationExecutionPathKind::ModeLeaseUnavailableBeforeAcquisitionCompensation,
            PreArmCancellationFinalizationExecutionPathRecord::ModeLeaseUnavailableAfterAcquisitionCompensation { .. } => PreArmCancellationFinalizationExecutionPathKind::ModeLeaseUnavailableAfterAcquisitionCompensation,
            PreArmCancellationFinalizationExecutionPathRecord::RecheckReplanCompensation { .. } => {
                PreArmCancellationFinalizationExecutionPathKind::RecheckReplanCompensation
            }
        }
    }

    pub(crate) fn action_ids(&self) -> &[UnicaId] {
        match &self.0 {
            PreArmCancellationFinalizationExecutionPathRecord::Success { action_ids }
            | PreArmCancellationFinalizationExecutionPathRecord::CapabilityBreachStop {
                action_ids,
            }
            | PreArmCancellationFinalizationExecutionPathRecord::RootGuardConflictCompensation {
                action_ids,
            }
            | PreArmCancellationFinalizationExecutionPathRecord::ModeLeaseUnavailableBeforeAcquisitionCompensation { action_ids }
            | PreArmCancellationFinalizationExecutionPathRecord::ModeLeaseUnavailableAfterAcquisitionCompensation { action_ids }
            | PreArmCancellationFinalizationExecutionPathRecord::RecheckReplanCompensation {
                action_ids,
            } => action_ids.as_slice(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
struct FinalizationExecutionPaths(Vec<PreArmCancellationFinalizationExecutionPath>);

impl FinalizationExecutionPaths {
    fn new(
        paths: Vec<PreArmCancellationFinalizationExecutionPath>,
    ) -> Result<Self, PreArmRecoveryContractError> {
        if paths.len() < 2
            || paths.len() > 6
            || paths
                .windows(2)
                .any(|pair| pair[0].path_kind() >= pair[1].path_kind())
            || !paths.iter().any(|path| {
                path.path_kind() == PreArmCancellationFinalizationExecutionPathKind::Success
            })
            || !paths.iter().any(|path| {
                path.path_kind()
                    == PreArmCancellationFinalizationExecutionPathKind::CapabilityBreachStop
            })
        {
            return Err(PreArmRecoveryContractError(
                "execution paths must be canonical and include success plus capability-breach",
            ));
        }
        Ok(Self(paths))
    }

    fn as_slice(&self) -> &[PreArmCancellationFinalizationExecutionPath] {
        &self.0
    }
}

impl JsonSchema for FinalizationExecutionPaths {
    fn schema_name() -> Cow<'static, str> {
        "PreArmCancellationFinalizationExecutionPaths".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "array",
            "minItems": 2,
            "maxItems": 6,
            "uniqueItems": true,
            "items": generator.subschema_for::<PreArmCancellationFinalizationExecutionPath>(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct PreArmCancellationFinalizationExecutionPathPlanDigestRecord {
    paths: FinalizationExecutionPaths,
}

impl contract_digest_record_sealed::Sealed
    for PreArmCancellationFinalizationExecutionPathPlanDigestRecord
{
}
impl ContractDigestRecord for PreArmCancellationFinalizationExecutionPathPlanDigestRecord {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct PreArmCancellationFinalizationExecutionPathPlan {
    paths: FinalizationExecutionPaths,
    execution_path_plan_digest: Sha256Digest,
}

impl PreArmCancellationFinalizationExecutionPathPlan {
    pub(crate) fn new(
        paths: Vec<PreArmCancellationFinalizationExecutionPath>,
    ) -> Result<Self, PreArmRecoveryContractError> {
        let record = PreArmCancellationFinalizationExecutionPathPlanDigestRecord {
            paths: FinalizationExecutionPaths::new(paths)?,
        };
        let execution_path_plan_digest =
            prearm_digest(&record, "execution path plan digest failed")?;
        Ok(Self {
            paths: record.paths,
            execution_path_plan_digest,
        })
    }

    pub(crate) fn paths(&self) -> &[PreArmCancellationFinalizationExecutionPath] {
        self.paths.as_slice()
    }

    pub(crate) const fn execution_path_plan_digest(&self) -> &Sha256Digest {
        &self.execution_path_plan_digest
    }
}

fn effect_kind_order(kind: PreArmCancellationEffectKind) -> u8 {
    match kind {
        PreArmCancellationEffectKind::RootGuardAcquire => 0,
        PreArmCancellationEffectKind::ModeLeaseAcquire => 1,
        PreArmCancellationEffectKind::SelectiveOriginalUpdate => 2,
        PreArmCancellationEffectKind::AuthorizationCancellation => 3,
        PreArmCancellationEffectKind::ModeLeaseRelease => 4,
        PreArmCancellationEffectKind::RootGuardRelease => 5,
        PreArmCancellationEffectKind::RecoveryFinalization => 6,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
struct EffectReceiptList(Vec<PreArmCancellationEffectReceipt>);

impl EffectReceiptList {
    fn new(
        values: Vec<PreArmCancellationEffectReceipt>,
    ) -> Result<Self, PreArmRecoveryContractError> {
        if values.len() > MAX_PREARM_ITEMS {
            return Err(PreArmRecoveryContractError(
                "pre-arm receipt list exceeds its contract bound",
            ));
        }
        let mut ids = BTreeSet::new();
        if values
            .iter()
            .any(|value| !ids.insert(value.receipt_id().as_str().to_owned()))
        {
            return Err(PreArmRecoveryContractError(
                "pre-arm receipt list repeats a receipt ID",
            ));
        }
        Ok(Self(values))
    }

    fn as_slice(&self) -> &[PreArmCancellationEffectReceipt] {
        &self.0
    }

    fn kinds(&self) -> Vec<PreArmCancellationEffectKind> {
        self.0.iter().map(|value| value.effect_kind()).collect()
    }

    fn is_forward_ordered(&self) -> bool {
        self.0.windows(2).all(|pair| {
            effect_kind_order(pair[0].effect_kind()) < effect_kind_order(pair[1].effect_kind())
        })
    }
}

impl JsonSchema for EffectReceiptList {
    fn schema_name() -> Cow<'static, str> {
        "PreArmCancellationEffectReceipts".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "array",
            "maxItems": MAX_PREARM_ITEMS,
            "uniqueItems": true,
            "items": generator.subschema_for::<PreArmCancellationEffectReceipt>(),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) enum PreArmCancellationModeLeaseAcquisitionState {
    NotAcquired,
    Acquired,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(
    tag = "modeLeaseAcquisitionState",
    rename_all = "camelCase",
    deny_unknown_fields
)]
enum PreArmCancellationModeLeaseCompensationRecord {
    NotAcquired {
        selected_execution_path_kind: ModeLeaseUnavailableBeforeAcquisitionPath,
        realized_forward_receipts: EffectReceiptList,
        compensation_release_receipts: EffectReceiptList,
    },
    Acquired {
        selected_execution_path_kind: ModeLeaseUnavailableAfterAcquisitionPath,
        realized_forward_receipts: EffectReceiptList,
        compensation_release_receipts: EffectReceiptList,
    },
}

wire_literal!(
    ModeLeaseUnavailableBeforeAcquisitionPath,
    "modeLeaseUnavailableBeforeAcquisitionCompensation"
);
wire_literal!(
    ModeLeaseUnavailableAfterAcquisitionPath,
    "modeLeaseUnavailableAfterAcquisitionCompensation"
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct PreArmCancellationModeLeaseCompensation(
    PreArmCancellationModeLeaseCompensationRecord,
);

impl JsonSchema for PreArmCancellationModeLeaseCompensation {
    fn schema_name() -> Cow<'static, str> {
        "PreArmCancellationModeLeaseCompensation".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        PreArmCancellationModeLeaseCompensationRecord::json_schema(generator)
    }
}

impl PreArmCancellationModeLeaseCompensation {
    fn new(
        state: PreArmCancellationModeLeaseAcquisitionState,
        realized_forward_receipts: Vec<PreArmCancellationEffectReceipt>,
        compensation_release_receipts: Vec<PreArmCancellationEffectReceipt>,
        compensation_complete: bool,
    ) -> Result<Self, PreArmRecoveryContractError> {
        let forward = EffectReceiptList::new(realized_forward_receipts)?;
        let compensation = EffectReceiptList::new(compensation_release_receipts)?;
        if !forward.is_forward_ordered() {
            return Err(PreArmRecoveryContractError(
                "mode-stop forward receipts violate effect order",
            ));
        }
        let expected_forward_suffix = match state {
            PreArmCancellationModeLeaseAcquisitionState::NotAcquired => {
                if forward
                    .kinds()
                    .iter()
                    .any(|kind| *kind != PreArmCancellationEffectKind::RootGuardAcquire)
                {
                    return Err(PreArmRecoveryContractError(
                        "mode-not-acquired forward prefix contains a non-root effect",
                    ));
                }
                vec![PreArmCancellationEffectKind::RootGuardRelease]
            }
            PreArmCancellationModeLeaseAcquisitionState::Acquired => {
                if forward.as_slice().is_empty()
                    || forward.as_slice().last().map(|value| value.effect_kind())
                        != Some(PreArmCancellationEffectKind::ModeLeaseAcquire)
                    || forward.kinds().iter().any(|kind| {
                        !matches!(
                            kind,
                            PreArmCancellationEffectKind::RootGuardAcquire
                                | PreArmCancellationEffectKind::ModeLeaseAcquire
                        )
                    })
                {
                    return Err(PreArmRecoveryContractError(
                        "mode-acquired forward prefix is not the acquisition sequence",
                    ));
                }
                vec![
                    PreArmCancellationEffectKind::ModeLeaseRelease,
                    PreArmCancellationEffectKind::RootGuardRelease,
                ]
            }
        };
        let actual_compensation = compensation.kinds();
        if actual_compensation.len() > expected_forward_suffix.len()
            || actual_compensation != expected_forward_suffix[..actual_compensation.len()]
            || compensation_complete && actual_compensation != expected_forward_suffix
            || !compensation_complete && actual_compensation.len() == expected_forward_suffix.len()
        {
            return Err(PreArmRecoveryContractError(
                "mode-stop compensation is not the exact reverse-release prefix",
            ));
        }
        let record = match state {
            PreArmCancellationModeLeaseAcquisitionState::NotAcquired => {
                PreArmCancellationModeLeaseCompensationRecord::NotAcquired {
                    selected_execution_path_kind: ModeLeaseUnavailableBeforeAcquisitionPath::Value,
                    realized_forward_receipts: forward,
                    compensation_release_receipts: compensation,
                }
            }
            PreArmCancellationModeLeaseAcquisitionState::Acquired => {
                PreArmCancellationModeLeaseCompensationRecord::Acquired {
                    selected_execution_path_kind: ModeLeaseUnavailableAfterAcquisitionPath::Value,
                    realized_forward_receipts: forward,
                    compensation_release_receipts: compensation,
                }
            }
        };
        Ok(Self(record))
    }

    pub(crate) const fn acquisition_state(&self) -> PreArmCancellationModeLeaseAcquisitionState {
        match &self.0 {
            PreArmCancellationModeLeaseCompensationRecord::NotAcquired { .. } => {
                PreArmCancellationModeLeaseAcquisitionState::NotAcquired
            }
            PreArmCancellationModeLeaseCompensationRecord::Acquired { .. } => {
                PreArmCancellationModeLeaseAcquisitionState::Acquired
            }
        }
    }

    fn realized_forward_receipts(&self) -> &[PreArmCancellationEffectReceipt] {
        match &self.0 {
            PreArmCancellationModeLeaseCompensationRecord::NotAcquired {
                realized_forward_receipts,
                ..
            }
            | PreArmCancellationModeLeaseCompensationRecord::Acquired {
                realized_forward_receipts,
                ..
            } => realized_forward_receipts.as_slice(),
        }
    }

    fn compensation_release_receipts(&self) -> &[PreArmCancellationEffectReceipt] {
        match &self.0 {
            PreArmCancellationModeLeaseCompensationRecord::NotAcquired {
                compensation_release_receipts,
                ..
            }
            | PreArmCancellationModeLeaseCompensationRecord::Acquired {
                compensation_release_receipts,
                ..
            } => compensation_release_receipts.as_slice(),
        }
    }
}

wire_literal!(RecheckReplanCompensationPath, "recheckReplanCompensation");
wire_literal!(
    RootGuardConflictCompensationPath,
    "rootGuardConflictCompensation"
);

// This is the exact closed progress oneOf; inline payloads deliberately match its wire leaves.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(tag = "stopCause", rename_all = "camelCase", deny_unknown_fields)]
enum PreArmCancellationFinalizationCompensatingCauseRecord {
    ModeLeaseUnavailable {
        mode_lease: PreArmCancellationModeLeaseCompensation,
    },
    RecheckReplanRequired {
        selected_execution_path_kind: RecheckReplanCompensationPath,
        realized_forward_receipts: EffectReceiptList,
        compensation_release_receipts: EffectReceiptList,
        recheck_evidence: PreArmCancellationFinalizationRecheckEvidence,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct PreArmCancellationFinalizationCompensatingCause(
    PreArmCancellationFinalizationCompensatingCauseRecord,
);

impl JsonSchema for PreArmCancellationFinalizationCompensatingCause {
    fn schema_name() -> Cow<'static, str> {
        "PreArmCancellationFinalizationCompensatingCause".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        PreArmCancellationFinalizationCompensatingCauseRecord::json_schema(generator)
    }
}

impl PreArmCancellationFinalizationCompensatingCause {
    fn mode_lease_unavailable(mode_lease: PreArmCancellationModeLeaseCompensation) -> Self {
        Self(
            PreArmCancellationFinalizationCompensatingCauseRecord::ModeLeaseUnavailable {
                mode_lease,
            },
        )
    }

    fn recheck_replan_required(
        realized_forward_receipts: Vec<PreArmCancellationEffectReceipt>,
        compensation_release_receipts: Vec<PreArmCancellationEffectReceipt>,
        recheck_evidence: PreArmCancellationFinalizationRecheckEvidence,
    ) -> Result<Self, PreArmRecoveryContractError> {
        if recheck_evidence.outcome()
            != PreArmCancellationFinalizationRecheckOutcome::ReplanRequired
        {
            return Err(PreArmRecoveryContractError(
                "recheck-replan compensation requires replanRequired evidence",
            ));
        }
        let forward = EffectReceiptList::new(realized_forward_receipts)?;
        let compensation = EffectReceiptList::new(compensation_release_receipts)?;
        if !forward.is_forward_ordered()
            || forward.kinds().iter().any(|kind| {
                !matches!(
                    kind,
                    PreArmCancellationEffectKind::RootGuardAcquire
                        | PreArmCancellationEffectKind::ModeLeaseAcquire
                )
            })
            || !matches!(
                compensation.kinds().as_slice(),
                [] | [PreArmCancellationEffectKind::ModeLeaseRelease]
            )
        {
            return Err(PreArmRecoveryContractError(
                "recheck-replan compensating progress has an invalid receipt prefix",
            ));
        }
        Ok(Self(
            PreArmCancellationFinalizationCompensatingCauseRecord::RecheckReplanRequired {
                selected_execution_path_kind: RecheckReplanCompensationPath::Value,
                realized_forward_receipts: forward,
                compensation_release_receipts: compensation,
                recheck_evidence,
            },
        ))
    }
}

// This is the exact closed progress oneOf; inline payloads deliberately match its wire leaves.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(tag = "stopCause", rename_all = "camelCase", deny_unknown_fields)]
enum PreArmCancellationFinalizationCompensatedCauseRecord {
    RootGuardConflict {
        selected_execution_path_kind: RootGuardConflictCompensationPath,
        realized_forward_receipts: EffectReceiptList,
        compensation_release_receipts: EffectReceiptList,
    },
    ModeLeaseUnavailable {
        mode_lease: PreArmCancellationModeLeaseCompensation,
    },
    RecheckReplanRequired {
        selected_execution_path_kind: RecheckReplanCompensationPath,
        realized_forward_receipts: EffectReceiptList,
        compensation_release_receipts: EffectReceiptList,
        recheck_evidence: PreArmCancellationFinalizationRecheckEvidence,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct PreArmCancellationFinalizationCompensatedCause(
    PreArmCancellationFinalizationCompensatedCauseRecord,
);

impl JsonSchema for PreArmCancellationFinalizationCompensatedCause {
    fn schema_name() -> Cow<'static, str> {
        "PreArmCancellationFinalizationCompensatedCause".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        PreArmCancellationFinalizationCompensatedCauseRecord::json_schema(generator)
    }
}

impl PreArmCancellationFinalizationCompensatedCause {
    fn root_guard_conflict() -> Result<Self, PreArmRecoveryContractError> {
        Ok(Self(
            PreArmCancellationFinalizationCompensatedCauseRecord::RootGuardConflict {
                selected_execution_path_kind: RootGuardConflictCompensationPath::Value,
                realized_forward_receipts: EffectReceiptList::new(Vec::new())?,
                compensation_release_receipts: EffectReceiptList::new(Vec::new())?,
            },
        ))
    }

    fn mode_lease_unavailable(
        mode_lease: PreArmCancellationModeLeaseCompensation,
    ) -> Result<Self, PreArmRecoveryContractError> {
        let expected = match mode_lease.acquisition_state() {
            PreArmCancellationModeLeaseAcquisitionState::NotAcquired => {
                vec![PreArmCancellationEffectKind::RootGuardRelease]
            }
            PreArmCancellationModeLeaseAcquisitionState::Acquired => vec![
                PreArmCancellationEffectKind::ModeLeaseRelease,
                PreArmCancellationEffectKind::RootGuardRelease,
            ],
        };
        let actual: Vec<_> = mode_lease
            .compensation_release_receipts()
            .iter()
            .map(|receipt| receipt.effect_kind())
            .collect();
        if actual != expected {
            return Err(PreArmRecoveryContractError(
                "compensated mode stop lacks the full reverse-release list",
            ));
        }
        Ok(Self(
            PreArmCancellationFinalizationCompensatedCauseRecord::ModeLeaseUnavailable {
                mode_lease,
            },
        ))
    }

    fn recheck_replan_required(
        realized_forward_receipts: Vec<PreArmCancellationEffectReceipt>,
        compensation_release_receipts: Vec<PreArmCancellationEffectReceipt>,
        recheck_evidence: PreArmCancellationFinalizationRecheckEvidence,
    ) -> Result<Self, PreArmRecoveryContractError> {
        let forward = EffectReceiptList::new(realized_forward_receipts)?;
        let compensation = EffectReceiptList::new(compensation_release_receipts)?;
        if recheck_evidence.outcome()
            != PreArmCancellationFinalizationRecheckOutcome::ReplanRequired
            || !forward.is_forward_ordered()
            || forward.kinds().iter().any(|kind| {
                !matches!(
                    kind,
                    PreArmCancellationEffectKind::RootGuardAcquire
                        | PreArmCancellationEffectKind::ModeLeaseAcquire
                )
            })
            || compensation.kinds()
                != vec![
                    PreArmCancellationEffectKind::ModeLeaseRelease,
                    PreArmCancellationEffectKind::RootGuardRelease,
                ]
        {
            return Err(PreArmRecoveryContractError(
                "compensated replan must retain full reverse release and replan evidence",
            ));
        }
        Ok(Self(
            PreArmCancellationFinalizationCompensatedCauseRecord::RecheckReplanRequired {
                selected_execution_path_kind: RecheckReplanCompensationPath::Value,
                realized_forward_receipts: forward,
                compensation_release_receipts: compensation,
                recheck_evidence,
            },
        ))
    }

    fn stop_kind(&self) -> PreArmCancellationCompensatedStopKind {
        match &self.0 {
            PreArmCancellationFinalizationCompensatedCauseRecord::RootGuardConflict { .. } => {
                PreArmCancellationCompensatedStopKind::RootGuardConflict
            }
            PreArmCancellationFinalizationCompensatedCauseRecord::ModeLeaseUnavailable {
                ..
            } => PreArmCancellationCompensatedStopKind::ModeLeaseUnavailable,
            PreArmCancellationFinalizationCompensatedCauseRecord::RecheckReplanRequired {
                ..
            } => PreArmCancellationCompensatedStopKind::RecheckReplanRequired,
        }
    }

    fn replan_evidence(&self) -> Option<&PreArmCancellationFinalizationRecheckEvidence> {
        match &self.0 {
            PreArmCancellationFinalizationCompensatedCauseRecord::RecheckReplanRequired {
                recheck_evidence,
                ..
            } => Some(recheck_evidence),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PreArmCancellationCompensatedStopKind {
    RootGuardConflict,
    ModeLeaseUnavailable,
    RecheckReplanRequired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) enum PreArmCancellationFinalizationAttemptState {
    NotStarted,
    InProgress,
    Compensating,
    Compensated,
    Completed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) enum PreArmCancellationForwardExecutionPathKind {
    Success,
    CapabilityBreachStop,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CompensatedAttemptProgressDigestRecord {
    attempt_state: CompensatedAttemptState,
    finalization_attempt_id: UnicaId,
    compensation: PreArmCancellationFinalizationCompensatedCause,
    all_attempt_guards_released: PreArmTrueLiteral,
}

wire_literal!(CompensatedAttemptState, "compensated");

impl contract_digest_record_sealed::Sealed for CompensatedAttemptProgressDigestRecord {}
impl ContractDigestRecord for CompensatedAttemptProgressDigestRecord {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CompletedAttemptProgressDigestRecord {
    attempt_state: CompletedAttemptState,
    finalization_attempt_id: UnicaId,
    selected_execution_path_kind: SuccessExecutionPath,
    realized_receipts: EffectReceiptList,
    recheck_evidence: PreArmCancellationFinalizationRecheckEvidence,
    all_attempt_guards_released: PreArmTrueLiteral,
}

wire_literal!(CompletedAttemptState, "completed");
wire_literal!(SuccessExecutionPath, "success");

impl contract_digest_record_sealed::Sealed for CompletedAttemptProgressDigestRecord {}
impl ContractDigestRecord for CompletedAttemptProgressDigestRecord {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(tag = "attemptState", rename_all = "camelCase", deny_unknown_fields)]
enum PreArmCancellationFinalizationAttemptProgressRecord {
    NotStarted {
        finalization_attempt_id: UnicaId,
    },
    InProgress {
        finalization_attempt_id: UnicaId,
        selected_execution_path_kind: PreArmCancellationForwardExecutionPathKind,
        realized_forward_receipts: EffectReceiptList,
        #[serde(skip_serializing_if = "Option::is_none")]
        recheck_evidence: Option<PreArmCancellationFinalizationRecheckEvidence>,
    },
    Compensating {
        finalization_attempt_id: UnicaId,
        compensation: PreArmCancellationFinalizationCompensatingCause,
    },
    Compensated {
        finalization_attempt_id: UnicaId,
        compensation: PreArmCancellationFinalizationCompensatedCause,
        all_attempt_guards_released: PreArmTrueLiteral,
        attempt_audit_digest: Sha256Digest,
    },
    Completed {
        finalization_attempt_id: UnicaId,
        selected_execution_path_kind: SuccessExecutionPath,
        realized_receipts: EffectReceiptList,
        recheck_evidence: PreArmCancellationFinalizationRecheckEvidence,
        all_attempt_guards_released: PreArmTrueLiteral,
        attempt_audit_digest: Sha256Digest,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct PreArmCancellationFinalizationAttemptProgress(
    PreArmCancellationFinalizationAttemptProgressRecord,
);

impl JsonSchema for PreArmCancellationFinalizationAttemptProgress {
    fn schema_name() -> Cow<'static, str> {
        "PreArmCancellationFinalizationAttemptProgress".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        PreArmCancellationFinalizationAttemptProgressRecord::json_schema(generator)
    }
}

impl PreArmCancellationFinalizationAttemptProgress {
    #[cfg(test)]
    pub(crate) fn not_started_test_only(finalization_attempt_id: UnicaId) -> Self {
        Self(
            PreArmCancellationFinalizationAttemptProgressRecord::NotStarted {
                finalization_attempt_id,
            },
        )
    }

    #[cfg(test)]
    pub(crate) fn in_progress_test_only(
        finalization_attempt_id: UnicaId,
        selected_execution_path_kind: PreArmCancellationForwardExecutionPathKind,
        realized_forward_receipts: Vec<PreArmCancellationEffectReceipt>,
        recheck_evidence: Option<PreArmCancellationFinalizationRecheckEvidence>,
    ) -> Result<Self, PreArmRecoveryContractError> {
        let receipts = EffectReceiptList::new(realized_forward_receipts)?;
        if !receipts.is_forward_ordered()
            || selected_execution_path_kind
                == PreArmCancellationForwardExecutionPathKind::CapabilityBreachStop
                && recheck_evidence.as_ref().is_none_or(|evidence| {
                    evidence.outcome()
                        != PreArmCancellationFinalizationRecheckOutcome::CapabilityBreach
                })
            || selected_execution_path_kind == PreArmCancellationForwardExecutionPathKind::Success
                && recheck_evidence.as_ref().is_some_and(|evidence| {
                    matches!(
                        evidence.outcome(),
                        PreArmCancellationFinalizationRecheckOutcome::CapabilityBreach
                            | PreArmCancellationFinalizationRecheckOutcome::ReplanRequired
                    )
                })
        {
            return Err(PreArmRecoveryContractError(
                "in-progress finalization has an invalid path/evidence/receipt prefix",
            ));
        }
        Ok(Self(
            PreArmCancellationFinalizationAttemptProgressRecord::InProgress {
                finalization_attempt_id,
                selected_execution_path_kind,
                realized_forward_receipts: receipts,
                recheck_evidence,
            },
        ))
    }

    #[cfg(test)]
    pub(crate) fn compensating_test_only(
        finalization_attempt_id: UnicaId,
        compensation: PreArmCancellationFinalizationCompensatingCause,
    ) -> Self {
        Self(
            PreArmCancellationFinalizationAttemptProgressRecord::Compensating {
                finalization_attempt_id,
                compensation,
            },
        )
    }

    #[cfg(test)]
    pub(crate) fn compensated_test_only(
        finalization_attempt_id: UnicaId,
        compensation: PreArmCancellationFinalizationCompensatedCause,
    ) -> Result<Self, PreArmRecoveryContractError> {
        let record = CompensatedAttemptProgressDigestRecord {
            attempt_state: CompensatedAttemptState::Value,
            finalization_attempt_id: finalization_attempt_id.clone(),
            compensation: compensation.clone(),
            all_attempt_guards_released: PreArmTrueLiteral,
        };
        let attempt_audit_digest =
            prearm_digest(&record, "compensated attempt progress digest failed")?;
        Ok(Self(
            PreArmCancellationFinalizationAttemptProgressRecord::Compensated {
                finalization_attempt_id,
                compensation,
                all_attempt_guards_released: PreArmTrueLiteral,
                attempt_audit_digest,
            },
        ))
    }

    #[cfg(test)]
    pub(crate) fn completed_test_only(
        finalization_attempt_id: UnicaId,
        realized_receipts: Vec<PreArmCancellationEffectReceipt>,
        recheck_evidence: PreArmCancellationFinalizationRecheckEvidence,
    ) -> Result<Self, PreArmRecoveryContractError> {
        let realized_receipts = EffectReceiptList::new(realized_receipts)?;
        if !realized_receipts.is_forward_ordered()
            || realized_receipts
                .as_slice()
                .last()
                .map(|value| value.effect_kind())
                != Some(PreArmCancellationEffectKind::RecoveryFinalization)
            || !matches!(
                recheck_evidence.outcome(),
                PreArmCancellationFinalizationRecheckOutcome::Matched
                    | PreArmCancellationFinalizationRecheckOutcome::SafeTailExtended
                    | PreArmCancellationFinalizationRecheckOutcome::ReleaseTailObserved
            )
        {
            return Err(PreArmRecoveryContractError(
                "completed finalization lacks ordered receipts, final receipt, or success evidence",
            ));
        }
        let record = CompletedAttemptProgressDigestRecord {
            attempt_state: CompletedAttemptState::Value,
            finalization_attempt_id: finalization_attempt_id.clone(),
            selected_execution_path_kind: SuccessExecutionPath::Value,
            realized_receipts: realized_receipts.clone(),
            recheck_evidence: recheck_evidence.clone(),
            all_attempt_guards_released: PreArmTrueLiteral,
        };
        let attempt_audit_digest =
            prearm_digest(&record, "completed attempt progress digest failed")?;
        Ok(Self(
            PreArmCancellationFinalizationAttemptProgressRecord::Completed {
                finalization_attempt_id,
                selected_execution_path_kind: SuccessExecutionPath::Value,
                realized_receipts,
                recheck_evidence,
                all_attempt_guards_released: PreArmTrueLiteral,
                attempt_audit_digest,
            },
        ))
    }

    pub(crate) const fn state(&self) -> PreArmCancellationFinalizationAttemptState {
        match &self.0 {
            PreArmCancellationFinalizationAttemptProgressRecord::NotStarted { .. } => {
                PreArmCancellationFinalizationAttemptState::NotStarted
            }
            PreArmCancellationFinalizationAttemptProgressRecord::InProgress { .. } => {
                PreArmCancellationFinalizationAttemptState::InProgress
            }
            PreArmCancellationFinalizationAttemptProgressRecord::Compensating { .. } => {
                PreArmCancellationFinalizationAttemptState::Compensating
            }
            PreArmCancellationFinalizationAttemptProgressRecord::Compensated { .. } => {
                PreArmCancellationFinalizationAttemptState::Compensated
            }
            PreArmCancellationFinalizationAttemptProgressRecord::Completed { .. } => {
                PreArmCancellationFinalizationAttemptState::Completed
            }
        }
    }

    pub(crate) const fn finalization_attempt_id(&self) -> &UnicaId {
        match &self.0 {
            PreArmCancellationFinalizationAttemptProgressRecord::NotStarted {
                finalization_attempt_id,
            }
            | PreArmCancellationFinalizationAttemptProgressRecord::InProgress {
                finalization_attempt_id,
                ..
            }
            | PreArmCancellationFinalizationAttemptProgressRecord::Compensating {
                finalization_attempt_id,
                ..
            }
            | PreArmCancellationFinalizationAttemptProgressRecord::Compensated {
                finalization_attempt_id,
                ..
            }
            | PreArmCancellationFinalizationAttemptProgressRecord::Completed {
                finalization_attempt_id,
                ..
            } => finalization_attempt_id,
        }
    }

    pub(crate) const fn attempt_audit_digest(&self) -> Option<&Sha256Digest> {
        match &self.0 {
            PreArmCancellationFinalizationAttemptProgressRecord::Compensated {
                attempt_audit_digest,
                ..
            }
            | PreArmCancellationFinalizationAttemptProgressRecord::Completed {
                attempt_audit_digest,
                ..
            } => Some(attempt_audit_digest),
            _ => None,
        }
    }

    /// Recheck evidence carried by the completed terminal branch.
    ///
    /// Archive projection uses this typed view to prove that the separately
    /// retained terminal evidence is byte-identical to the evidence embedded
    /// in the completed progress, without serializing and probing the record.
    pub(crate) const fn completed_recheck_evidence(
        &self,
    ) -> Option<&PreArmCancellationFinalizationRecheckEvidence> {
        match &self.0 {
            PreArmCancellationFinalizationAttemptProgressRecord::Completed {
                recheck_evidence,
                ..
            } => Some(recheck_evidence),
            _ => None,
        }
    }

    /// Exact finalization-plan receipts retained by the completed attempt.
    ///
    /// This deliberately exposes only the immutable completed slice. Recovery
    /// uses it to prove that its ordered action outcomes produced precisely the
    /// receipts persisted in progress, rather than trusting receipt IDs alone.
    pub(crate) fn completed_realized_receipts(&self) -> Option<&[PreArmCancellationEffectReceipt]> {
        match &self.0 {
            PreArmCancellationFinalizationAttemptProgressRecord::Completed {
                realized_receipts,
                ..
            } => Some(realized_receipts.as_slice()),
            _ => None,
        }
    }

    fn compensated_stop_kind(&self) -> Option<PreArmCancellationCompensatedStopKind> {
        match &self.0 {
            PreArmCancellationFinalizationAttemptProgressRecord::Compensated {
                compensation,
                ..
            } => Some(compensation.stop_kind()),
            _ => None,
        }
    }

    fn compensated_replan_evidence(
        &self,
    ) -> Option<&PreArmCancellationFinalizationRecheckEvidence> {
        match &self.0 {
            PreArmCancellationFinalizationAttemptProgressRecord::Compensated {
                compensation,
                ..
            } => compensation.replan_evidence(),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
struct CompensatedFinalizationAttemptProgress(PreArmCancellationFinalizationAttemptProgress);

impl CompensatedFinalizationAttemptProgress {
    #[cfg(test)]
    fn new(
        progress: PreArmCancellationFinalizationAttemptProgress,
    ) -> Result<Self, PreArmRecoveryContractError> {
        if progress.state() != PreArmCancellationFinalizationAttemptState::Compensated {
            return Err(PreArmRecoveryContractError(
                "attempt audit may preserve only a fully compensated attempt",
            ));
        }
        Ok(Self(progress))
    }

    fn progress(&self) -> &PreArmCancellationFinalizationAttemptProgress {
        &self.0
    }
}

impl JsonSchema for CompensatedFinalizationAttemptProgress {
    fn schema_name() -> Cow<'static, str> {
        "PreArmCancellationCompensatedFinalizationAttemptProgress".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "object",
            "properties": {
                "attemptState": { "type": "string", "const": "compensated" },
                "finalizationAttemptId": generator.subschema_for::<UnicaId>(),
                "compensation": generator.subschema_for::<PreArmCancellationFinalizationCompensatedCause>(),
                "allAttemptGuardsReleased": { "type": "boolean", "const": true },
                "attemptAuditDigest": generator.subschema_for::<Sha256Digest>(),
            },
            "required": [
                "attemptState",
                "finalizationAttemptId",
                "compensation",
                "allAttemptGuardsReleased",
                "attemptAuditDigest"
            ],
            "additionalProperties": false,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct PreArmCancellationFinalizationAttemptAuditDigestRecord {
    finalization_attempt_id: UnicaId,
    finalization_plan_digest: Sha256Digest,
    compensated_progress: CompensatedFinalizationAttemptProgress,
}

impl contract_digest_record_sealed::Sealed
    for PreArmCancellationFinalizationAttemptAuditDigestRecord
{
}
impl ContractDigestRecord for PreArmCancellationFinalizationAttemptAuditDigestRecord {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct PreArmCancellationFinalizationAttemptAudit {
    finalization_attempt_id: UnicaId,
    finalization_plan_digest: Sha256Digest,
    compensated_progress: CompensatedFinalizationAttemptProgress,
    audit_digest: Sha256Digest,
}

impl PreArmCancellationFinalizationAttemptAudit {
    #[cfg(test)]
    pub(crate) fn new_test_only(
        finalization_plan_digest: Sha256Digest,
        compensated_progress: PreArmCancellationFinalizationAttemptProgress,
    ) -> Result<Self, PreArmRecoveryContractError> {
        let compensated_progress =
            CompensatedFinalizationAttemptProgress::new(compensated_progress)?;
        let record = PreArmCancellationFinalizationAttemptAuditDigestRecord {
            finalization_attempt_id: compensated_progress
                .progress()
                .finalization_attempt_id()
                .clone(),
            finalization_plan_digest,
            compensated_progress,
        };
        let audit_digest = prearm_digest(&record, "attempt audit digest failed")?;
        Ok(Self {
            finalization_attempt_id: record.finalization_attempt_id,
            finalization_plan_digest: record.finalization_plan_digest,
            compensated_progress: record.compensated_progress,
            audit_digest,
        })
    }

    pub(crate) const fn finalization_attempt_id(&self) -> &UnicaId {
        &self.finalization_attempt_id
    }

    pub(crate) const fn audit_digest(&self) -> &Sha256Digest {
        &self.audit_digest
    }

    fn stop_kind(&self) -> PreArmCancellationCompensatedStopKind {
        self.compensated_progress
            .progress()
            .compensated_stop_kind()
            .expect("attempt audit constructor enforces compensated progress")
    }

    fn replan_evidence(&self) -> Option<&PreArmCancellationFinalizationRecheckEvidence> {
        self.compensated_progress
            .progress()
            .compensated_replan_evidence()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PreArmCancellationKnownBlockerKind {
    RootGuardConflict,
    ModeLeaseUnavailable,
}

wire_literal!(RootGuardConflictBlocker, "rootGuardConflict");
wire_literal!(ModeLeaseUnavailableBlocker, "modeLeaseUnavailable");

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RootGuardConflictBlockerDigestRecord {
    blocker_kind: RootGuardConflictBlocker,
    previous_recovery_digest: Sha256Digest,
    compensated_attempt_audit_digest: Sha256Digest,
    failed_target: RepositoryTargetIdentity,
    failed_target_display: RepositoryTargetDisplay,
    #[serde(deserialize_with = "RequiredNullable::deserialize_required")]
    locked_by: RequiredNullable<RepositoryOwnerIdentity>,
    required_external_action: ReleaseRepositoryLocksInstruction,
}

impl contract_digest_record_sealed::Sealed for RootGuardConflictBlockerDigestRecord {}
impl ContractDigestRecord for RootGuardConflictBlockerDigestRecord {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RootGuardConflictBlockerValue {
    blocker_kind: RootGuardConflictBlocker,
    previous_recovery_digest: Sha256Digest,
    compensated_attempt_audit_digest: Sha256Digest,
    failed_target: RepositoryTargetIdentity,
    failed_target_display: RepositoryTargetDisplay,
    #[serde(deserialize_with = "RequiredNullable::deserialize_required")]
    locked_by: RequiredNullable<RepositoryOwnerIdentity>,
    required_external_action: ReleaseRepositoryLocksInstruction,
    blocker_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SeparateModeLeaseUnavailableBlockerDigestRecord {
    blocker_kind: ModeLeaseUnavailableBlocker,
    previous_recovery_digest: Sha256Digest,
    compensated_attempt_audit_digest: Sha256Digest,
    manual_target_mode: SeparateWorkingInfobaseMode,
    working_infobase_stop: ManualWorkingInfobaseStopEvidence,
    required_external_action: CleanManualWorkingInfobaseInstruction,
}

impl contract_digest_record_sealed::Sealed for SeparateModeLeaseUnavailableBlockerDigestRecord {}
impl ContractDigestRecord for SeparateModeLeaseUnavailableBlockerDigestRecord {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SeparateModeLeaseUnavailableBlockerValue {
    blocker_kind: ModeLeaseUnavailableBlocker,
    previous_recovery_digest: Sha256Digest,
    compensated_attempt_audit_digest: Sha256Digest,
    manual_target_mode: SeparateWorkingInfobaseMode,
    working_infobase_stop: ManualWorkingInfobaseStopEvidence,
    required_external_action: CleanManualWorkingInfobaseInstruction,
    blocker_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ReservedModeLeaseUnavailableBlockerDigestRecord {
    blocker_kind: ModeLeaseUnavailableBlocker,
    previous_recovery_digest: Sha256Digest,
    compensated_attempt_audit_digest: Sha256Digest,
    manual_target_mode: ReservedOriginalMode,
    reserved_original_lease_stop: ReservedOriginalLeaseStopEvidence,
    required_external_action: CloseReservedOriginalDesignerInstruction,
}

impl contract_digest_record_sealed::Sealed for ReservedModeLeaseUnavailableBlockerDigestRecord {}
impl ContractDigestRecord for ReservedModeLeaseUnavailableBlockerDigestRecord {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReservedModeLeaseUnavailableBlockerValue {
    blocker_kind: ModeLeaseUnavailableBlocker,
    previous_recovery_digest: Sha256Digest,
    compensated_attempt_audit_digest: Sha256Digest,
    manual_target_mode: ReservedOriginalMode,
    reserved_original_lease_stop: ReservedOriginalLeaseStopEvidence,
    required_external_action: CloseReservedOriginalDesignerInstruction,
    blocker_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
enum PreArmCancellationKnownBlockerRecord {
    RootGuardConflict(RootGuardConflictBlockerValue),
    SeparateModeLeaseUnavailable(SeparateModeLeaseUnavailableBlockerValue),
    ReservedModeLeaseUnavailable(ReservedModeLeaseUnavailableBlockerValue),
}

impl JsonSchema for PreArmCancellationKnownBlockerRecord {
    fn schema_name() -> Cow<'static, str> {
        "PreArmCancellationKnownBlockerRecord".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        super::schema::one_of_schema(vec![
            generator.subschema_for::<RootGuardConflictBlockerValue>(),
            generator.subschema_for::<SeparateModeLeaseUnavailableBlockerValue>(),
            generator.subschema_for::<ReservedModeLeaseUnavailableBlockerValue>(),
        ])
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg(test)]
enum PreArmCancellationKnownBlockerAuthorityKind {
    RootGuardConflict {
        failed_target: RepositoryTargetIdentity,
        failed_target_display: RepositoryTargetDisplay,
        locked_by: RequiredNullable<RepositoryOwnerIdentity>,
        required_external_action: ReleaseRepositoryLocksInstruction,
    },
    SeparateModeLeaseUnavailable {
        working_infobase_stop: ManualWorkingInfobaseStopEvidence,
        required_external_action: CleanManualWorkingInfobaseInstruction,
    },
    ReservedModeLeaseUnavailable {
        reserved_original_lease_stop: ReservedOriginalLeaseStopEvidence,
        required_external_action: CloseReservedOriginalDesignerInstruction,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg(test)]
pub(crate) struct PreArmCancellationKnownBlockerAuthority {
    previous_recovery_digest: Sha256Digest,
    compensated_attempt: PreArmCancellationFinalizationAttemptAudit,
    kind: PreArmCancellationKnownBlockerAuthorityKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct PreArmCancellationKnownBlocker(PreArmCancellationKnownBlockerRecord);

impl JsonSchema for PreArmCancellationKnownBlocker {
    fn schema_name() -> Cow<'static, str> {
        "PreArmCancellationKnownBlocker".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        PreArmCancellationKnownBlockerRecord::json_schema(generator)
    }
}

impl PreArmCancellationKnownBlocker {
    /// Fixture-only until the stopped-data producer can bind the full
    /// plan/attempt/mode-specific stop evidence as one authority.
    #[cfg(test)]
    pub(crate) fn new_test_only(
        authority: PreArmCancellationKnownBlockerAuthority,
    ) -> Result<Self, PreArmRecoveryContractError> {
        let compensated_attempt_audit_digest = authority.compensated_attempt.audit_digest().clone();
        match authority.kind {
            PreArmCancellationKnownBlockerAuthorityKind::RootGuardConflict {
                failed_target,
                failed_target_display,
                locked_by,
                required_external_action,
            } => {
                if authority.compensated_attempt.stop_kind()
                    != PreArmCancellationCompensatedStopKind::RootGuardConflict
                {
                    return Err(PreArmRecoveryContractError(
                        "root blocker does not match its compensated attempt",
                    ));
                }
                let record = RootGuardConflictBlockerDigestRecord {
                    blocker_kind: RootGuardConflictBlocker::Value,
                    previous_recovery_digest: authority.previous_recovery_digest,
                    compensated_attempt_audit_digest,
                    failed_target,
                    failed_target_display,
                    locked_by,
                    required_external_action,
                };
                let blocker_digest = prearm_digest(&record, "root blocker digest failed")?;
                Ok(Self(
                    PreArmCancellationKnownBlockerRecord::RootGuardConflict(
                        RootGuardConflictBlockerValue {
                            blocker_kind: record.blocker_kind,
                            previous_recovery_digest: record.previous_recovery_digest,
                            compensated_attempt_audit_digest: record
                                .compensated_attempt_audit_digest,
                            failed_target: record.failed_target,
                            failed_target_display: record.failed_target_display,
                            locked_by: record.locked_by,
                            required_external_action: record.required_external_action,
                            blocker_digest,
                        },
                    ),
                ))
            }
            PreArmCancellationKnownBlockerAuthorityKind::SeparateModeLeaseUnavailable {
                working_infobase_stop,
                required_external_action,
            } => {
                if authority.compensated_attempt.stop_kind()
                    != PreArmCancellationCompensatedStopKind::ModeLeaseUnavailable
                {
                    return Err(PreArmRecoveryContractError(
                        "separate-mode blocker does not match its compensated attempt",
                    ));
                }
                let record = SeparateModeLeaseUnavailableBlockerDigestRecord {
                    blocker_kind: ModeLeaseUnavailableBlocker::Value,
                    previous_recovery_digest: authority.previous_recovery_digest,
                    compensated_attempt_audit_digest,
                    manual_target_mode: SeparateWorkingInfobaseMode::Value,
                    working_infobase_stop,
                    required_external_action,
                };
                let blocker_digest = prearm_digest(&record, "separate-mode blocker digest failed")?;
                Ok(Self(
                    PreArmCancellationKnownBlockerRecord::SeparateModeLeaseUnavailable(
                        SeparateModeLeaseUnavailableBlockerValue {
                            blocker_kind: record.blocker_kind,
                            previous_recovery_digest: record.previous_recovery_digest,
                            compensated_attempt_audit_digest: record
                                .compensated_attempt_audit_digest,
                            manual_target_mode: record.manual_target_mode,
                            working_infobase_stop: record.working_infobase_stop,
                            required_external_action: record.required_external_action,
                            blocker_digest,
                        },
                    ),
                ))
            }
            PreArmCancellationKnownBlockerAuthorityKind::ReservedModeLeaseUnavailable {
                reserved_original_lease_stop,
                required_external_action,
            } => {
                if authority.compensated_attempt.stop_kind()
                    != PreArmCancellationCompensatedStopKind::ModeLeaseUnavailable
                {
                    return Err(PreArmRecoveryContractError(
                        "reserved-mode blocker does not match its compensated attempt",
                    ));
                }
                let record = ReservedModeLeaseUnavailableBlockerDigestRecord {
                    blocker_kind: ModeLeaseUnavailableBlocker::Value,
                    previous_recovery_digest: authority.previous_recovery_digest,
                    compensated_attempt_audit_digest,
                    manual_target_mode: ReservedOriginalMode::Value,
                    reserved_original_lease_stop,
                    required_external_action,
                };
                let blocker_digest = prearm_digest(&record, "reserved-mode blocker digest failed")?;
                Ok(Self(
                    PreArmCancellationKnownBlockerRecord::ReservedModeLeaseUnavailable(
                        ReservedModeLeaseUnavailableBlockerValue {
                            blocker_kind: record.blocker_kind,
                            previous_recovery_digest: record.previous_recovery_digest,
                            compensated_attempt_audit_digest: record
                                .compensated_attempt_audit_digest,
                            manual_target_mode: record.manual_target_mode,
                            reserved_original_lease_stop: record.reserved_original_lease_stop,
                            required_external_action: record.required_external_action,
                            blocker_digest,
                        },
                    ),
                ))
            }
        }
    }

    pub(crate) const fn blocker_kind(&self) -> PreArmCancellationKnownBlockerKind {
        match &self.0 {
            PreArmCancellationKnownBlockerRecord::RootGuardConflict(_) => {
                PreArmCancellationKnownBlockerKind::RootGuardConflict
            }
            PreArmCancellationKnownBlockerRecord::SeparateModeLeaseUnavailable(_)
            | PreArmCancellationKnownBlockerRecord::ReservedModeLeaseUnavailable(_) => {
                PreArmCancellationKnownBlockerKind::ModeLeaseUnavailable
            }
        }
    }

    pub(crate) const fn blocker_digest(&self) -> &Sha256Digest {
        match &self.0 {
            PreArmCancellationKnownBlockerRecord::RootGuardConflict(value) => &value.blocker_digest,
            PreArmCancellationKnownBlockerRecord::SeparateModeLeaseUnavailable(value) => {
                &value.blocker_digest
            }
            PreArmCancellationKnownBlockerRecord::ReservedModeLeaseUnavailable(value) => {
                &value.blocker_digest
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) enum PreArmCancellationFinalizationCompletionMode {
    VerifyCancelledAndRelease,
    FinishCancellationAndRelease,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) enum PreArmCancellationStartingGuardState {
    HeldFromPriorOperation,
    Released,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
struct PriorAttemptAudits(Vec<PreArmCancellationFinalizationAttemptAudit>);

impl PriorAttemptAudits {
    fn new(
        values: Vec<PreArmCancellationFinalizationAttemptAudit>,
    ) -> Result<Self, PreArmRecoveryContractError> {
        let mut ids = BTreeSet::new();
        if values.len() > MAX_PREARM_ITEMS
            || values
                .iter()
                .any(|value| !ids.insert(value.finalization_attempt_id().as_str().to_owned()))
        {
            return Err(PreArmRecoveryContractError(
                "prior attempt audits must be bounded and unique by attempt ID",
            ));
        }
        Ok(Self(values))
    }

    fn as_slice(&self) -> &[PreArmCancellationFinalizationAttemptAudit] {
        &self.0
    }
}

impl JsonSchema for PriorAttemptAudits {
    fn schema_name() -> Cow<'static, str> {
        "PreArmCancellationPriorAttemptAudits".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "array",
            "maxItems": MAX_PREARM_ITEMS,
            "uniqueItems": true,
            "items": generator.subschema_for::<PreArmCancellationFinalizationAttemptAudit>(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PreArmCancellationAttemptAuditLineageAuthority {
    audits: PriorAttemptAudits,
    previous_planned_result_phase: Option<TaskPhase>,
    previous_history_partition: Option<ValidatedRepositoryHistoryPartition>,
    previous_replannable_continuity: Option<Option<(bool, bool)>>,
}

impl PreArmCancellationAttemptAuditLineageAuthority {
    fn initial() -> Self {
        Self {
            audits: PriorAttemptAudits(Vec::new()),
            previous_planned_result_phase: None,
            previous_history_partition: None,
            previous_replannable_continuity: None,
        }
    }

    pub(crate) fn append_compensated_attempt(
        previous_plan: &PreArmCancellationFinalizationPlan,
        audit: PreArmCancellationFinalizationAttemptAudit,
    ) -> Result<Self, PreArmRecoveryContractError> {
        if audit.finalization_plan_digest != previous_plan.finalization_plan_digest
            || audit.finalization_attempt_id != previous_plan.finalization_attempt_id
        {
            return Err(PreArmRecoveryContractError(
                "appended attempt audit does not bind the immediately preceding plan",
            ));
        }
        let mut audits = previous_plan.prior_attempt_audits.0.clone();
        audits.push(audit);
        Ok(Self {
            audits: PriorAttemptAudits::new(audits)?,
            previous_planned_result_phase: Some(previous_plan.planned_result_phase),
            previous_history_partition: Some(previous_plan.finalization_history_partition.clone()),
            previous_replannable_continuity: Some(previous_plan.recheck_policy.continuity()),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct PreArmCancellationFinalizationPlanDigestRecord {
    finalization_attempt_id: UnicaId,
    prior_operation_id: OperationId,
    support_action_id: UnicaId,
    expected_support_action_digest: Sha256Digest,
    approved_cancellation_digest: Sha256Digest,
    effect_observation_digest: Sha256Digest,
    completion_mode: PreArmCancellationFinalizationCompletionMode,
    manual_target_mode: ManualSupportTargetMode,
    starting_root_guard_state: PreArmCancellationStartingGuardState,
    starting_mode_lease_state: PreArmCancellationStartingGuardState,
    acquire_root_guard: bool,
    acquire_mode_lease: bool,
    receipt_plan: PreArmCancellationReceiptPlan,
    recheck_policy: PreArmCancellationFinalizationRecheckPolicy,
    execution_path_plan: PreArmCancellationFinalizationExecutionPathPlan,
    prior_attempt_audits: PriorAttemptAudits,
    finalization_history_partition: ValidatedRepositoryHistoryPartition,
    selective_update_plan: SelectiveRepositoryUpdatePlan,
    expected_final_original_fingerprint: Sha256Digest,
    expected_final_support_graph_digest: Sha256Digest,
    planned_result_phase: TaskPhase,
    relevant_advance_phase: TaskPhase,
}

impl contract_digest_record_sealed::Sealed for PreArmCancellationFinalizationPlanDigestRecord {}
impl ContractDigestRecord for PreArmCancellationFinalizationPlanDigestRecord {}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg(test)]
pub(crate) struct PreArmCancellationFinalizationPlanAuthority {
    finalization_attempt_id: UnicaId,
    effect_observation: PreArmCancellationEffectObservation,
    completion_mode: PreArmCancellationFinalizationCompletionMode,
    starting_root_guard_state: PreArmCancellationStartingGuardState,
    starting_mode_lease_state: PreArmCancellationStartingGuardState,
    acquire_root_guard: bool,
    acquire_mode_lease: bool,
    receipt_plan: PreArmCancellationReceiptPlan,
    expected_postcondition_digests: PreArmCancellationExpectedPostconditionDigests,
    recheck_policy: PreArmCancellationFinalizationRecheckPolicy,
    execution_path_plan: PreArmCancellationFinalizationExecutionPathPlan,
    prior_attempt_lineage: PreArmCancellationAttemptAuditLineageAuthority,
    finalization_history_partition: ValidatedRepositoryHistoryPartition,
    selective_update_plan: SelectiveRepositoryUpdatePlan,
    expected_final_original_fingerprint: Sha256Digest,
    expected_final_support_graph_digest: Sha256Digest,
    planned_result_phase: TaskPhase,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct PreArmCancellationFinalizationPlan {
    finalization_attempt_id: UnicaId,
    prior_operation_id: OperationId,
    support_action_id: UnicaId,
    expected_support_action_digest: Sha256Digest,
    approved_cancellation_digest: Sha256Digest,
    effect_observation_digest: Sha256Digest,
    completion_mode: PreArmCancellationFinalizationCompletionMode,
    manual_target_mode: ManualSupportTargetMode,
    starting_root_guard_state: PreArmCancellationStartingGuardState,
    starting_mode_lease_state: PreArmCancellationStartingGuardState,
    acquire_root_guard: bool,
    acquire_mode_lease: bool,
    receipt_plan: PreArmCancellationReceiptPlan,
    recheck_policy: PreArmCancellationFinalizationRecheckPolicy,
    execution_path_plan: PreArmCancellationFinalizationExecutionPathPlan,
    prior_attempt_audits: PriorAttemptAudits,
    finalization_history_partition: ValidatedRepositoryHistoryPartition,
    selective_update_plan: SelectiveRepositoryUpdatePlan,
    expected_final_original_fingerprint: Sha256Digest,
    expected_final_support_graph_digest: Sha256Digest,
    planned_result_phase: TaskPhase,
    relevant_advance_phase: TaskPhase,
    finalization_plan_digest: Sha256Digest,
    #[serde(skip)]
    #[schemars(skip)]
    bound_cancelled_phase: TaskPhase,
}

/// Borrowed, non-wire projection of the pre-arm lineage retained by an
/// archive entry. Keeping these observations together prevents an archive
/// producer from independently selecting receipts, history, or phase.
///
/// This is intentionally not the complete terminal-recovery authority. The
/// archive constructor separately requires the exact action/outcome receipt
/// witness; Task 13/16 must additionally bind root/mode proofs, exact recheck
/// rows, approved-cancellation producer authority, and the enclosing recovery
/// receipt before a production archive constructor exists.
#[derive(Debug)]
pub(crate) struct PreArmCancellationArchiveLineageProjection<'a> {
    pub(crate) effect_observation: &'a PreArmCancellationEffectObservation,
    pub(crate) finalization_recheck_evidence: &'a PreArmCancellationFinalizationRecheckEvidence,
    pub(crate) completed_finalization_progress: &'a PreArmCancellationFinalizationAttemptProgress,
    pub(crate) support_cancellation_receipt_id: &'a UnicaId,
    pub(crate) support_cancellation_receipt_digest: &'a Sha256Digest,
    pub(crate) pre_arm_recovery_receipt_id: &'a UnicaId,
    pub(crate) pre_arm_recovery_receipt_digest: &'a Sha256Digest,
    pub(crate) selective_update_proof: &'a SelectiveRepositoryUpdateProof,
    pub(crate) post_release_observed_history_cursor: &'a RepositoryHistoryCursor,
    pub(crate) post_apply_history_partition: &'a ValidatedRepositoryHistoryPartition,
    pub(crate) deferred_repository_advance: Option<&'a DeferredRepositoryAdvance>,
    pub(crate) resulting_phase: TaskPhase,
}

fn partition_requires_relevant_advance(partition: &ValidatedRepositoryHistoryPartition) -> bool {
    partition.classifications().any(|classification| {
        matches!(
            classification,
            RepositoryHistoryPartitionClassification::RelevantRoutine
                | RepositoryHistoryPartitionClassification::ExternalSupport
                | RepositoryHistoryPartitionClassification::PreArmExternal
        )
    })
}

fn serialized_partition_entries(
    partition: &ValidatedRepositoryHistoryPartition,
) -> Result<Vec<serde_json::Value>, PreArmRecoveryContractError> {
    serde_json::to_value(partition)
        .map_err(|_| PreArmRecoveryContractError("repository history serialization failed"))?
        .get("entries")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .ok_or(PreArmRecoveryContractError(
            "repository history lacks canonical entries",
        ))
}

fn is_exact_partition_concatenation(
    base: &ValidatedRepositoryHistoryPartition,
    appended: &ValidatedRepositoryHistoryPartition,
    combined: &ValidatedRepositoryHistoryPartition,
) -> Result<bool, PreArmRecoveryContractError> {
    if combined.start_cursor() != base.start_cursor()
        || appended.start_cursor() != base.through_inclusive()
        || combined.through_inclusive() != appended.through_inclusive()
        || !combined.contains_cursor(base.through_inclusive())
    {
        return Ok(false);
    }
    let mut expected = serialized_partition_entries(base)?;
    expected.extend(serialized_partition_entries(appended)?);
    Ok(serialized_partition_entries(combined)? == expected)
}

fn exact_replan_suffix_classifications(
    base: &ValidatedRepositoryHistoryPartition,
    refreshed: &ValidatedRepositoryHistoryPartition,
) -> Result<Vec<RepositoryHistoryPartitionClassification>, PreArmRecoveryContractError> {
    if base.start_cursor() != refreshed.start_cursor()
        || base.through_inclusive() == refreshed.through_inclusive()
        || !refreshed.contains_cursor(base.through_inclusive())
    {
        return Err(PreArmRecoveryContractError(
            "replan history is not a non-empty append from the prior plan",
        ));
    }
    let base_entries = serialized_partition_entries(base)?;
    let refreshed_entries = serialized_partition_entries(refreshed)?;
    if refreshed_entries.len() <= base_entries.len()
        || &refreshed_entries[..base_entries.len()] != base_entries.as_slice()
    {
        return Err(PreArmRecoveryContractError(
            "replan history does not preserve the prior plan as an exact prefix",
        ));
    }
    let suffix: Vec<_> = refreshed
        .classifications()
        .skip(base_entries.len())
        .collect();
    if suffix.is_empty()
        || suffix.iter().any(|classification| {
            !matches!(
                classification,
                RepositoryHistoryPartitionClassification::UnrelatedRoutine
                    | RepositoryHistoryPartitionClassification::RelevantRoutine
            )
        })
    {
        return Err(PreArmRecoveryContractError(
            "replan history suffix is not a non-empty non-root routine tail",
        ));
    }
    Ok(suffix)
}

impl PreArmCancellationFinalizationPlan {
    /// Fixture-only until a result/status producer binds progress and blockers
    /// to this exact plan, receipt plan, and execution path.
    #[cfg(test)]
    pub(crate) fn new_test_only(
        authority: PreArmCancellationFinalizationPlanAuthority,
    ) -> Result<Self, PreArmRecoveryContractError> {
        let progress = authority.effect_observation.effect_progress();
        let source_stage = progress.stage();
        let has_prior_attempts = !authority.prior_attempt_lineage.audits.0.is_empty();
        if source_stage == PreArmCancellationEffectProgressStage::UpdateReadyGuardsHeld
            && has_prior_attempts
        {
            return Err(PreArmRecoveryContractError(
                "update-ready source cannot carry prior compensated attempt audits",
            ));
        }
        if authority
            .prior_attempt_lineage
            .audits
            .as_slice()
            .iter()
            .any(|audit| audit.finalization_attempt_id() == &authority.finalization_attempt_id)
        {
            return Err(PreArmRecoveryContractError(
                "fresh finalization attempt ID reuses an archived attempt",
            ));
        }
        if authority.recheck_policy.source_progress_stage() != source_stage
            || authority.finalization_history_partition.start_cursor()
                != authority
                    .effect_observation
                    .history_partition()
                    .start_cursor()
            || authority.recheck_policy.expected_partition_binding()
                != (
                    authority.finalization_history_partition.through_inclusive(),
                    authority.finalization_history_partition.partition_digest(),
                )
            || authority.starting_mode_lease_state
                == PreArmCancellationStartingGuardState::HeldFromPriorOperation
                && authority.starting_root_guard_state
                    != PreArmCancellationStartingGuardState::HeldFromPriorOperation
        {
            return Err(PreArmRecoveryContractError(
                "finalization plan violates its source stage, history binding, or guard hierarchy",
            ));
        }
        let disposition = authority.receipt_plan.selective_update_disposition();
        let (source_current_original_fingerprint, source_current_support_graph_digest) = authority
            .prior_attempt_lineage
            .audits
            .as_slice()
            .iter()
            .rev()
            .find_map(|audit| audit.replan_evidence())
            .and_then(PreArmCancellationFinalizationRecheckEvidence::observed_state_binding)
            .unwrap_or((
                authority.effect_observation.current_original_fingerprint(),
                authority
                    .effect_observation
                    .expected_final_support_graph_digest(),
            ));
        let expected_final_original_fingerprint = (disposition
            != PreArmCancellationSelectiveUpdateDisposition::Perform)
            .then_some(source_current_original_fingerprint);
        if expected_final_original_fingerprint
            .is_some_and(|expected| &authority.expected_final_original_fingerprint != expected)
            || &authority.expected_final_support_graph_digest != source_current_support_graph_digest
            || authority
                .recheck_policy
                .expected_recheck_state_binding()
                .is_some_and(|(policy_original, policy_support)| {
                    policy_original != source_current_original_fingerprint
                        || policy_support != source_current_support_graph_digest
                })
        {
            return Err(PreArmRecoveryContractError(
                "finalization plan state fingerprints differ from current source/replan evidence",
            ));
        }

        if has_prior_attempts
            != (authority
                .prior_attempt_lineage
                .previous_planned_result_phase
                .is_some()
                && authority
                    .prior_attempt_lineage
                    .previous_history_partition
                    .is_some()
                && authority
                    .prior_attempt_lineage
                    .previous_replannable_continuity
                    .is_some())
        {
            return Err(PreArmRecoveryContractError(
                "attempt lineage lacks the immediately preceding phase or history partition",
            ));
        }
        let mut expected_lineage_partition = authority.effect_observation.history_partition();
        for audit in authority.prior_attempt_lineage.audits.as_slice() {
            if let Some(evidence) = audit.replan_evidence() {
                let refreshed =
                    evidence
                        .refreshed_history_partition()
                        .ok_or(PreArmRecoveryContractError(
                            "replan attempt audit lacks its refreshed history partition",
                        ))?;
                if refreshed.start_cursor() != expected_lineage_partition.start_cursor() {
                    return Err(PreArmRecoveryContractError(
                        "replan attempt changed the authorization history anchor",
                    ));
                }
                expected_lineage_partition = refreshed;
            }
        }
        if &authority.finalization_history_partition != expected_lineage_partition {
            return Err(PreArmRecoveryContractError(
                "finalization history partition does not follow immutable attempt lineage",
            ));
        }

        let cancelled_phase = authority.effect_observation.cancelled_phase();
        let relevant_advance_phase = authority.effect_observation.relevant_advance_phase();
        if authority.planned_result_phase != cancelled_phase
            && authority.planned_result_phase != relevant_advance_phase
        {
            return Err(PreArmRecoveryContractError(
                "planned result phase is outside the approved cancellation phase pair",
            ));
        }
        let source_forces_relevant = matches!(
            source_stage,
            PreArmCancellationEffectProgressStage::UpdateReadyGuardsHeld
                | PreArmCancellationEffectProgressStage::CancellationPersistedGuardsHeld
                | PreArmCancellationEffectProgressStage::CancellationPersistedModeReleased
                | PreArmCancellationEffectProgressStage::CancellationPersistedReleased
        );
        let expected_result_phase = if source_forces_relevant {
            relevant_advance_phase
        } else if has_prior_attempts {
            let previous_phase = authority
                .prior_attempt_lineage
                .previous_planned_result_phase
                .expect("presence checked above");
            let previous_partition = authority
                .prior_attempt_lineage
                .previous_history_partition
                .as_ref()
                .expect("presence checked above");
            let latest_audit = authority
                .prior_attempt_lineage
                .audits
                .as_slice()
                .last()
                .expect("non-empty lineage checked above");
            match latest_audit.replan_evidence() {
                Some(evidence) => {
                    let mismatch_kinds =
                        evidence
                            .replan_mismatch_kinds()
                            .ok_or(PreArmRecoveryContractError(
                                "replan evidence lacks mismatch kinds",
                            ))?;
                    let refreshed = evidence.refreshed_history_partition().ok_or(
                        PreArmRecoveryContractError(
                            "replan attempt audit lacks its refreshed history partition",
                        ),
                    )?;
                    if mismatch_kinds
                        == [PreArmCancellationFinalizationReplanMismatchKind::NonRootRoutineTailAdvanced]
                    {
                        let suffix =
                            exact_replan_suffix_classifications(previous_partition, refreshed)?;
                        if suffix
                            .contains(&RepositoryHistoryPartitionClassification::RelevantRoutine)
                        {
                            relevant_advance_phase
                        } else {
                            previous_phase
                        }
                    } else {
                        if mismatch_kinds.contains(
                            &PreArmCancellationFinalizationReplanMismatchKind::NonRootRoutineTailAdvanced,
                        ) {
                            return Err(PreArmRecoveryContractError(
                                "non-root routine-tail advance must be the sole replan mismatch",
                            ));
                        }
                        let (continuous_root, continuous_mode) = authority
                            .prior_attempt_lineage
                            .previous_replannable_continuity
                            .expect("presence checked above")
                            .ok_or(PreArmRecoveryContractError(
                                "replan evidence does not follow a replannable plan",
                            ))?;
                        if mismatch_kinds.iter().any(|kind| {
                            matches!(
                                kind,
                                PreArmCancellationFinalizationReplanMismatchKind::RootOrSupportVersionChanged
                                    | PreArmCancellationFinalizationReplanMismatchKind::SupportGraphChanged
                            ) && continuous_root
                                || *kind
                                    == PreArmCancellationFinalizationReplanMismatchKind::OriginalTargetChanged
                                    && continuous_mode
                        }) {
                            return Err(PreArmRecoveryContractError(
                                "mutable-state replan mismatch was observed under its continuous guard",
                            ));
                        }
                        if partition_requires_relevant_advance(refreshed) {
                            relevant_advance_phase
                        } else {
                            cancelled_phase
                        }
                    }
                }
                None => {
                    if previous_partition != &authority.finalization_history_partition {
                        return Err(PreArmRecoveryContractError(
                            "non-replan attempt changed the finalization history partition",
                        ));
                    }
                    previous_phase
                }
            }
        } else if partition_requires_relevant_advance(&authority.finalization_history_partition) {
            relevant_advance_phase
        } else {
            cancelled_phase
        };
        if authority.planned_result_phase != expected_result_phase
            || partition_requires_relevant_advance(&authority.finalization_history_partition)
                && authority.planned_result_phase != relevant_advance_phase
        {
            return Err(PreArmRecoveryContractError(
                "planned result phase disagrees with approved history and attempt lineage",
            ));
        }
        let expected_projection = if has_prior_attempts {
            (
                PreArmCancellationStartingGuardState::Released,
                PreArmCancellationStartingGuardState::Released,
                true,
                true,
            )
        } else {
            initial_guard_projection(source_stage)
        };
        if (
            authority.starting_root_guard_state,
            authority.starting_mode_lease_state,
            authority.acquire_root_guard,
            authority.acquire_mode_lease,
        ) != expected_projection
            || authority.receipt_plan.bound_guard_acquisition_flags()
                != (authority.acquire_root_guard, authority.acquire_mode_lease)
        {
            return Err(PreArmRecoveryContractError(
                "starting guard state does not follow the source/compensation projection",
            ));
        }

        let persisted = matches!(
            source_stage,
            PreArmCancellationEffectProgressStage::CancellationPersistedGuardsHeld
                | PreArmCancellationEffectProgressStage::CancellationPersistedModeReleased
                | PreArmCancellationEffectProgressStage::CancellationPersistedReleased
        );
        if persisted
            != (authority.completion_mode
                == PreArmCancellationFinalizationCompletionMode::VerifyCancelledAndRelease)
            || persisted && has_prior_attempts
            || persisted
                && (authority.acquire_root_guard
                    || authority.acquire_mode_lease
                    || authority.recheck_policy.mode()
                        != PreArmCancellationFinalizationRecheckMode::ReleaseOnlyAfterPersistence
                    || authority.planned_result_phase != relevant_advance_phase)
            || !persisted
                && authority.recheck_policy.mode()
                    == PreArmCancellationFinalizationRecheckMode::ReleaseOnlyAfterPersistence
            || source_stage == PreArmCancellationEffectProgressStage::UpdateReadyGuardsHeld
                && (authority.recheck_policy.mode()
                    != PreArmCancellationFinalizationRecheckMode::ProtectedUpdateReady
                    || authority.planned_result_phase != relevant_advance_phase)
            || !persisted
                && source_stage != PreArmCancellationEffectProgressStage::UpdateReadyGuardsHeld
                && authority.recheck_policy.mode()
                    != PreArmCancellationFinalizationRecheckMode::ReplannableBeforeUpdate
        {
            return Err(PreArmRecoveryContractError(
                "completion mode, recheck policy, and result phase disagree with source stage",
            ));
        }

        let expected_continuity = (
            authority.starting_root_guard_state
                == PreArmCancellationStartingGuardState::HeldFromPriorOperation
                && !authority.acquire_root_guard,
            authority.starting_mode_lease_state
                == PreArmCancellationStartingGuardState::HeldFromPriorOperation
                && !authority.acquire_mode_lease,
        );
        if authority
            .recheck_policy
            .continuity()
            .is_some_and(|value| value != expected_continuity)
        {
            return Err(PreArmRecoveryContractError(
                "replannable continuity literals do not follow actual starting guards",
            ));
        }

        validate_receipt_plan_starting_guards(
            &authority.receipt_plan,
            authority.starting_root_guard_state,
            authority.starting_mode_lease_state,
            authority.acquire_root_guard,
            authority.acquire_mode_lease,
        )?;
        validate_execution_paths(
            &authority.execution_path_plan,
            &authority.receipt_plan,
            authority.effect_observation.manual_target_mode(),
            authority.acquire_root_guard,
            authority.acquire_mode_lease,
            authority.recheck_policy.mode(),
        )?;

        let projection =
            ApprovedSelectiveUpdatePlanProjection::from_plan(&authority.selective_update_plan)?;
        if &projection.plan_digest != authority.receipt_plan.selective_update_plan_digest()
            || &projection.plan_digest
                != authority.effect_observation.selective_update_plan_digest()
        {
            return Err(PreArmRecoveryContractError(
                "observation and receipt plan do not bind the exact selective update plan",
            ));
        }
        validate_finalization_receipt_intents(
            &authority.receipt_plan,
            &authority.finalization_attempt_id,
            &authority.effect_observation,
            &authority.expected_postcondition_digests,
        )?;

        let record = PreArmCancellationFinalizationPlanDigestRecord {
            finalization_attempt_id: authority.finalization_attempt_id,
            prior_operation_id: authority.effect_observation.prior_operation_id().clone(),
            support_action_id: authority.effect_observation.support_action_id().clone(),
            expected_support_action_digest: authority
                .effect_observation
                .expected_support_action_digest()
                .clone(),
            approved_cancellation_digest: authority
                .effect_observation
                .approved_cancellation_digest()
                .clone(),
            effect_observation_digest: authority.effect_observation.observation_digest().clone(),
            completion_mode: authority.completion_mode,
            manual_target_mode: authority.effect_observation.manual_target_mode(),
            starting_root_guard_state: authority.starting_root_guard_state,
            starting_mode_lease_state: authority.starting_mode_lease_state,
            acquire_root_guard: authority.acquire_root_guard,
            acquire_mode_lease: authority.acquire_mode_lease,
            receipt_plan: authority.receipt_plan,
            recheck_policy: authority.recheck_policy,
            execution_path_plan: authority.execution_path_plan,
            prior_attempt_audits: authority.prior_attempt_lineage.audits,
            finalization_history_partition: authority.finalization_history_partition,
            selective_update_plan: authority.selective_update_plan,
            expected_final_original_fingerprint: authority.expected_final_original_fingerprint,
            expected_final_support_graph_digest: authority.expected_final_support_graph_digest,
            planned_result_phase: authority.planned_result_phase,
            relevant_advance_phase,
        };
        let finalization_plan_digest = prearm_digest(&record, "finalization plan digest failed")?;
        Ok(Self {
            finalization_attempt_id: record.finalization_attempt_id,
            prior_operation_id: record.prior_operation_id,
            support_action_id: record.support_action_id,
            expected_support_action_digest: record.expected_support_action_digest,
            approved_cancellation_digest: record.approved_cancellation_digest,
            effect_observation_digest: record.effect_observation_digest,
            completion_mode: record.completion_mode,
            manual_target_mode: record.manual_target_mode,
            starting_root_guard_state: record.starting_root_guard_state,
            starting_mode_lease_state: record.starting_mode_lease_state,
            acquire_root_guard: record.acquire_root_guard,
            acquire_mode_lease: record.acquire_mode_lease,
            receipt_plan: record.receipt_plan,
            recheck_policy: record.recheck_policy,
            execution_path_plan: record.execution_path_plan,
            prior_attempt_audits: record.prior_attempt_audits,
            finalization_history_partition: record.finalization_history_partition,
            selective_update_plan: record.selective_update_plan,
            expected_final_original_fingerprint: record.expected_final_original_fingerprint,
            expected_final_support_graph_digest: record.expected_final_support_graph_digest,
            planned_result_phase: record.planned_result_phase,
            relevant_advance_phase: record.relevant_advance_phase,
            finalization_plan_digest,
            bound_cancelled_phase: cancelled_phase,
        })
    }

    pub(crate) const fn finalization_attempt_id(&self) -> &UnicaId {
        &self.finalization_attempt_id
    }

    /// One canonical cross-contract identity check for every consumer of a
    /// pre-arm finalization plan.  Matching only the support action is not
    /// sufficient: two observations for the same action may belong to
    /// different operations, cancellation approvals, target modes, or
    /// observation generations.
    pub(crate) fn binds_effect_observation(
        &self,
        observation: &PreArmCancellationEffectObservation,
    ) -> bool {
        observation.prior_operation_id() == &self.prior_operation_id
            && observation.support_action_id() == &self.support_action_id
            && observation.expected_support_action_digest() == &self.expected_support_action_digest
            && observation.approved_cancellation_digest() == &self.approved_cancellation_digest
            && observation.manual_target_mode() == self.manual_target_mode
            && observation.observation_digest() == &self.effect_observation_digest
            && observation.cancelled_phase() == self.bound_cancelled_phase
            && observation.relevant_advance_phase() == self.relevant_advance_phase
    }

    /// Validate the complete archive-retained projection against this exact
    /// finalization plan.
    ///
    /// This is deliberately one archive-lineage predicate: validating the
    /// retained plan, receipts, recheck evidence, update proof, cursor range,
    /// and phase separately permits values from different attempts to be
    /// spliced into one archive entry. Full root/mode proof, terminal scanner,
    /// and enclosing recovery-receipt authority remain Task 13/16 gates.
    pub(crate) fn validate_archive_lineage_projection(
        &self,
        projection: PreArmCancellationArchiveLineageProjection<'_>,
    ) -> Result<(), PreArmRecoveryContractError> {
        if !self.binds_effect_observation(projection.effect_observation) {
            return Err(PreArmRecoveryContractError(
                "terminal effect observation belongs to another finalization plan",
            ));
        }

        let (completed_attempt_id, realized_receipts, embedded_recheck_evidence) =
            match &projection.completed_finalization_progress.0 {
                PreArmCancellationFinalizationAttemptProgressRecord::Completed {
                    finalization_attempt_id,
                    realized_receipts,
                    recheck_evidence,
                    ..
                } => (finalization_attempt_id, realized_receipts, recheck_evidence),
                _ => {
                    return Err(PreArmRecoveryContractError(
                    "archive-lineage projection does not contain completed finalization progress",
                ));
                }
            };
        if completed_attempt_id != &self.finalization_attempt_id
            || embedded_recheck_evidence != projection.finalization_recheck_evidence
        {
            return Err(PreArmRecoveryContractError(
                "completed finalization progress belongs to another attempt or recheck",
            ));
        }

        let mut realized = realized_receipts.as_slice().iter();
        let mut cancellation_pair_matched = false;
        let mut recovery_pair_matched = false;
        let mut resolved_root_guard_receipt_id = None;
        let mut resolved_update_receipt = None;
        for (receipt_ref, expected_kind) in self.receipt_plan.ordered_refs_with_kinds() {
            let resolved = match receipt_ref.source() {
                PreArmCancellationReceiptSource::PriorOperation => receipt_ref
                    .prior_receipt()
                    .ok_or(PreArmRecoveryContractError(
                        "prior-operation receipt ref lacks its immutable receipt",
                    ))?,
                PreArmCancellationReceiptSource::FinalizationPlan => {
                    realized.next().ok_or(PreArmRecoveryContractError(
                        "completed finalization omits a planned effect receipt",
                    ))?
                }
            };
            if receipt_ref.effect_kind() != expected_kind
                || resolved.effect_kind() != expected_kind
                || resolved.receipt_id() != receipt_ref.receipt_id()
                || resolved.effect_intent_digest() != receipt_ref.effect_intent_digest()
            {
                return Err(PreArmRecoveryContractError(
                    "completed finalization receipt does not resolve its exact plan ref",
                ));
            }
            match expected_kind {
                PreArmCancellationEffectKind::RootGuardAcquire => {
                    resolved_root_guard_receipt_id = Some(resolved.receipt_id());
                }
                PreArmCancellationEffectKind::SelectiveOriginalUpdate => {
                    resolved_update_receipt = Some(resolved);
                }
                PreArmCancellationEffectKind::AuthorizationCancellation => {
                    cancellation_pair_matched = resolved.receipt_id()
                        == projection.support_cancellation_receipt_id
                        && resolved.receipt_digest()
                            == projection.support_cancellation_receipt_digest;
                }
                PreArmCancellationEffectKind::RecoveryFinalization => {
                    recovery_pair_matched = resolved.receipt_id()
                        == projection.pre_arm_recovery_receipt_id
                        && resolved.receipt_digest() == projection.pre_arm_recovery_receipt_digest;
                }
                _ => {}
            }
        }
        if realized.next().is_some() || !cancellation_pair_matched || !recovery_pair_matched {
            return Err(PreArmRecoveryContractError(
                "completed finalization receipts or terminal receipt pairs differ from the plan",
            ));
        }

        if projection.selective_update_proof.plan_digest()
            != self.selective_update_plan.plan_digest()
            || resolved_root_guard_receipt_id
                != Some(projection.selective_update_proof.guard_receipt_id())
            || projection
                .selective_update_proof
                .verified_original_target_fingerprint_digest()
                != &self.expected_final_original_fingerprint
        {
            return Err(PreArmRecoveryContractError(
                "terminal selective update proof belongs to another plan or fingerprint",
            ));
        }
        let proof_effect_pair = projection
            .selective_update_proof
            .update_effect_receipt_id()
            .zip(
                projection
                    .selective_update_proof
                    .update_effect_receipt_digest(),
            );
        let resolved_effect_pair =
            resolved_update_receipt.map(|receipt| (receipt.receipt_id(), receipt.receipt_digest()));
        let update_disposition_matches = match self.receipt_plan.selective_update_disposition {
            PreArmCancellationSelectiveUpdateDisposition::NotRequired => {
                self.selective_update_plan
                    .planned_targets()
                    .is_empty_for_prearm()
                    && resolved_update_receipt.is_none()
                    && !projection.selective_update_proof.update_performed()
                    && proof_effect_pair.is_none()
            }
            PreArmCancellationSelectiveUpdateDisposition::AlreadyExact => {
                !self
                    .selective_update_plan
                    .planned_targets()
                    .is_empty_for_prearm()
                    && resolved_update_receipt.is_none()
                    && !projection.selective_update_proof.update_performed()
                    && proof_effect_pair.is_none()
                    && projection
                        .effect_observation
                        .effect_progress()
                        .update_progress()
                        .is_some_and(|progress| match &progress.0 {
                            PreArmCancellationUpdateProgressRecord::AlreadyExact {
                                already_exact_evidence,
                            } => {
                                projection
                                    .selective_update_proof
                                    .before_original_target_fingerprint_map_digest()
                                    == &already_exact_evidence
                                        .before_original_target_fingerprint_map_digest
                                    && projection
                                        .selective_update_proof
                                        .verified_original_target_fingerprint_digest()
                                        == &already_exact_evidence
                                            .verified_original_target_fingerprint_digest
                            }
                            _ => false,
                        })
            }
            PreArmCancellationSelectiveUpdateDisposition::AlreadyApplied => {
                !self
                    .selective_update_plan
                    .planned_targets()
                    .is_empty_for_prearm()
                    && projection.selective_update_proof.update_performed()
                    && proof_effect_pair == resolved_effect_pair
                    && projection
                        .effect_observation
                        .effect_progress()
                        .update_progress()
                        .is_some_and(|progress| match &progress.0 {
                            PreArmCancellationUpdateProgressRecord::Applied {
                                selective_update_effect,
                            } => {
                                projection
                                    .selective_update_proof
                                    .before_original_target_fingerprint_map_digest()
                                    == &selective_update_effect
                                        .before_original_target_fingerprint_digest
                                    && projection
                                        .selective_update_proof
                                        .verified_original_target_fingerprint_digest()
                                        == &selective_update_effect
                                            .verified_original_target_fingerprint_digest
                            }
                            _ => false,
                        })
            }
            PreArmCancellationSelectiveUpdateDisposition::Perform => {
                !self
                    .selective_update_plan
                    .planned_targets()
                    .is_empty_for_prearm()
                    && projection.selective_update_proof.update_performed()
                    && proof_effect_pair == resolved_effect_pair
            }
        };
        if !update_disposition_matches {
            return Err(PreArmRecoveryContractError(
                "terminal selective update proof contradicts the receipt-plan disposition",
            ));
        }

        let (reconciled_partition, recheck_forces_relevant) = self
            .validate_terminal_recheck_evidence(
                projection.finalization_recheck_evidence,
                projection.post_apply_history_partition,
            )?;
        if reconciled_partition.through_inclusive()
            != projection.selective_update_proof.observed_before_cursor()
            || projection.post_apply_history_partition.start_cursor()
                != projection.selective_update_proof.observed_before_cursor()
            || !projection
                .post_apply_history_partition
                .contains_cursor(projection.selective_update_proof.observed_after_cursor())
            || projection.post_apply_history_partition.through_inclusive()
                != projection.post_release_observed_history_cursor
            || !projection
                .post_apply_history_partition
                .all_entries_are_one_of(&[
                    RepositoryHistoryPartitionClassification::UnrelatedRoutine,
                    RepositoryHistoryPartitionClassification::RelevantRoutine,
                    RepositoryHistoryPartitionClassification::ExternalSupport,
                    RepositoryHistoryPartitionClassification::PreArmExternal,
                ])
            || projection
                .deferred_repository_advance
                .is_some_and(|advance| {
                    advance.anchor_cursor()
                        != projection.post_apply_history_partition.through_inclusive()
                })
        {
            return Err(PreArmRecoveryContractError(
                "terminal history partitions, proof cursors, and deferred advance are not contiguous",
            ));
        }

        let expected_phase = if recheck_forces_relevant
            || self.planned_result_phase == self.relevant_advance_phase
            || partition_requires_relevant_advance(projection.post_apply_history_partition)
            || projection.deferred_repository_advance.is_some()
        {
            self.relevant_advance_phase
        } else {
            self.bound_cancelled_phase
        };
        if projection.resulting_phase != expected_phase {
            return Err(PreArmRecoveryContractError(
                "terminal result phase disagrees with recheck and post-release history",
            ));
        }
        Ok(())
    }

    fn validate_terminal_recheck_evidence<'a>(
        &self,
        evidence: &'a PreArmCancellationFinalizationRecheckEvidence,
        post_apply_history_partition: &ValidatedRepositoryHistoryPartition,
    ) -> Result<(&'a ValidatedRepositoryHistoryPartition, bool), PreArmRecoveryContractError> {
        match &evidence.0 {
            PreArmCancellationFinalizationRecheckEvidenceRecord::Matched(value) => {
                let expected_state = self.recheck_policy.expected_recheck_state_binding();
                if !matches!(
                    self.recheck_policy.mode(),
                    PreArmCancellationFinalizationRecheckMode::ReplannableBeforeUpdate
                        | PreArmCancellationFinalizationRecheckMode::ProtectedUpdateReady
                ) || value.observed_history_partition != self.finalization_history_partition
                    || expected_state
                        != Some((
                            &value.observed_original_fingerprint,
                            &value.observed_support_graph_digest,
                        ))
                {
                    return Err(PreArmRecoveryContractError(
                        "matched recheck evidence differs from the finalization plan",
                    ));
                }
                Ok((&value.observed_history_partition, false))
            }
            PreArmCancellationFinalizationRecheckEvidenceRecord::SafeTailExtended(value) => {
                let expected_state = self.recheck_policy.expected_recheck_state_binding();
                if self.recheck_policy.mode()
                    != PreArmCancellationFinalizationRecheckMode::ProtectedUpdateReady
                    || value.base_history_partition != self.finalization_history_partition
                    || expected_state
                        != Some((
                            &value.observed_original_fingerprint,
                            &value.observed_support_graph_digest,
                        ))
                {
                    return Err(PreArmRecoveryContractError(
                        "safe-tail recheck evidence differs from the finalization plan",
                    ));
                }
                Ok((&value.combined_history_partition, true))
            }
            PreArmCancellationFinalizationRecheckEvidenceRecord::ReleaseTailObserved(value) => {
                let tail_allowed = match &self.recheck_policy.0 {
                    PreArmCancellationFinalizationRecheckPolicyRecord::ReleaseOnlyAfterPersistence(
                        policy,
                    ) => match policy.allowed_tail_classifications {
                        ReleaseTailClassifications::RootHeld(_) => value
                            .appended_history_partition
                            .all_entries_are_one_of(&[
                                RepositoryHistoryPartitionClassification::UnrelatedRoutine,
                                RepositoryHistoryPartitionClassification::RelevantRoutine,
                            ]),
                        ReleaseTailClassifications::FullyReleased(_) => value
                            .appended_history_partition
                            .all_entries_are_one_of(&[
                                RepositoryHistoryPartitionClassification::UnrelatedRoutine,
                                RepositoryHistoryPartitionClassification::RelevantRoutine,
                                RepositoryHistoryPartitionClassification::ExternalSupport,
                                RepositoryHistoryPartitionClassification::PreArmExternal,
                            ]),
                    },
                    _ => false,
                };
                if !tail_allowed
                    || value.persisted_history_partition != self.finalization_history_partition
                    || value.observed_original_fingerprint
                        != self.expected_final_original_fingerprint
                    || value.observed_support_graph_digest
                        != self.expected_final_support_graph_digest
                    || !post_apply_history_partition
                        .has_exact_entry_prefix(&value.appended_history_partition)
                {
                    return Err(PreArmRecoveryContractError(
                        "release-tail recheck is not the exact leading post-apply segment",
                    ));
                }
                Ok((&value.persisted_history_partition, true))
            }
            PreArmCancellationFinalizationRecheckEvidenceRecord::ReplanRequired(_)
            | PreArmCancellationFinalizationRecheckEvidenceRecord::CapabilityBreach(_) => {
                Err(PreArmRecoveryContractError(
                    "non-success recheck evidence cannot complete finalization",
                ))
            }
        }
    }

    pub(crate) const fn prior_operation_id(&self) -> &OperationId {
        &self.prior_operation_id
    }

    pub(crate) const fn support_action_id(&self) -> &UnicaId {
        &self.support_action_id
    }

    pub(crate) const fn expected_support_action_digest(&self) -> &Sha256Digest {
        &self.expected_support_action_digest
    }

    pub(crate) const fn approved_cancellation_digest(&self) -> &Sha256Digest {
        &self.approved_cancellation_digest
    }

    pub(crate) const fn effect_observation_digest(&self) -> &Sha256Digest {
        &self.effect_observation_digest
    }

    pub(crate) const fn manual_target_mode(&self) -> ManualSupportTargetMode {
        self.manual_target_mode
    }

    pub(crate) fn selective_update_plan_digest(&self) -> &Sha256Digest {
        self.selective_update_plan.plan_digest()
    }

    pub(crate) fn expected_target_revision_map_digest(&self) -> &Sha256Digest {
        self.selective_update_plan
            .expected_target_revision_map_digest()
    }

    pub(crate) const fn planned_result_phase(&self) -> TaskPhase {
        self.planned_result_phase
    }

    pub(crate) const fn receipt_plan(&self) -> &PreArmCancellationReceiptPlan {
        &self.receipt_plan
    }

    pub(crate) const fn recheck_policy(&self) -> &PreArmCancellationFinalizationRecheckPolicy {
        &self.recheck_policy
    }

    pub(crate) const fn execution_path_plan(
        &self,
    ) -> &PreArmCancellationFinalizationExecutionPathPlan {
        &self.execution_path_plan
    }

    pub(crate) fn prior_attempt_audits(&self) -> &[PreArmCancellationFinalizationAttemptAudit] {
        self.prior_attempt_audits.as_slice()
    }

    pub(crate) const fn finalization_plan_digest(&self) -> &Sha256Digest {
        &self.finalization_plan_digest
    }

    /// Recompute a future receipt intent from the sealed plan identity and the
    /// action's exact expected postcondition. Comparing a receipt ref alone is
    /// insufficient because an action could otherwise retain the right ref
    /// while carrying a substituted postcondition.
    pub(crate) fn validates_finalization_action_receipt_intent(
        &self,
        receipt_ref: &PreArmCancellationReceiptRef,
        effect_kind: PreArmCancellationEffectKind,
        expected_postcondition_digest: &Sha256Digest,
    ) -> Result<bool, PreArmRecoveryContractError> {
        if receipt_ref.source() != PreArmCancellationReceiptSource::FinalizationPlan
            || receipt_ref.effect_kind() != effect_kind
        {
            return Ok(false);
        }
        let expected_intent = prearm_digest(
            &FinalizationPlanEffectIntentDigestRecord {
                effect_kind,
                finalization_attempt_id: self.finalization_attempt_id.clone(),
                support_action_id: self.support_action_id.clone(),
                expected_support_action_digest: self.expected_support_action_digest.clone(),
                approved_cancellation_digest: self.approved_cancellation_digest.clone(),
                effect_observation_digest: self.effect_observation_digest.clone(),
                manual_target_mode: self.manual_target_mode,
                selective_update_plan_digest: self.selective_update_plan.plan_digest().clone(),
                expected_postcondition_digest: expected_postcondition_digest.clone(),
            },
            "finalization action receipt intent digest failed",
        )?;
        Ok(receipt_ref.effect_intent_digest() == &expected_intent)
    }
}

fn initial_guard_projection(
    stage: PreArmCancellationEffectProgressStage,
) -> (
    PreArmCancellationStartingGuardState,
    PreArmCancellationStartingGuardState,
    bool,
    bool,
) {
    match stage {
        PreArmCancellationEffectProgressStage::NoGuard
        | PreArmCancellationEffectProgressStage::RootReleasedBeforeLease
        | PreArmCancellationEffectProgressStage::GuardsReleasedBeforeUpdate => (
            PreArmCancellationStartingGuardState::Released,
            PreArmCancellationStartingGuardState::Released,
            true,
            true,
        ),
        PreArmCancellationEffectProgressStage::RootHeldBeforeLease
        | PreArmCancellationEffectProgressStage::ModeReleasedBeforeUpdateRootHeld => (
            PreArmCancellationStartingGuardState::HeldFromPriorOperation,
            PreArmCancellationStartingGuardState::Released,
            false,
            true,
        ),
        PreArmCancellationEffectProgressStage::GuardsHeldBeforeUpdate
        | PreArmCancellationEffectProgressStage::UpdateReadyGuardsHeld
        | PreArmCancellationEffectProgressStage::CancellationPersistedGuardsHeld => (
            PreArmCancellationStartingGuardState::HeldFromPriorOperation,
            PreArmCancellationStartingGuardState::HeldFromPriorOperation,
            false,
            false,
        ),
        PreArmCancellationEffectProgressStage::CancellationPersistedModeReleased => (
            PreArmCancellationStartingGuardState::HeldFromPriorOperation,
            PreArmCancellationStartingGuardState::Released,
            false,
            false,
        ),
        PreArmCancellationEffectProgressStage::CancellationPersistedReleased => (
            PreArmCancellationStartingGuardState::Released,
            PreArmCancellationStartingGuardState::Released,
            false,
            false,
        ),
    }
}

fn validate_receipt_plan_starting_guards(
    receipt_plan: &PreArmCancellationReceiptPlan,
    root_state: PreArmCancellationStartingGuardState,
    mode_state: PreArmCancellationStartingGuardState,
    acquire_root: bool,
    acquire_mode: bool,
) -> Result<(), PreArmRecoveryContractError> {
    let root_source = receipt_plan.root_guard_acquisition_receipt().source();
    let mode_source = receipt_plan.mode_lease_acquisition_receipt().source();
    if (root_state == PreArmCancellationStartingGuardState::HeldFromPriorOperation
        && root_source != PreArmCancellationReceiptSource::PriorOperation)
        || (root_state == PreArmCancellationStartingGuardState::Released
            && acquire_root
            && root_source != PreArmCancellationReceiptSource::FinalizationPlan)
        || (mode_state == PreArmCancellationStartingGuardState::HeldFromPriorOperation
            && mode_source != PreArmCancellationReceiptSource::PriorOperation)
        || (mode_state == PreArmCancellationStartingGuardState::Released
            && acquire_mode
            && mode_source != PreArmCancellationReceiptSource::FinalizationPlan)
    {
        return Err(PreArmRecoveryContractError(
            "guard acquisition refs do not match the actual starting state",
        ));
    }
    Ok(())
}

fn validate_execution_paths(
    plan: &PreArmCancellationFinalizationExecutionPathPlan,
    receipt_plan: &PreArmCancellationReceiptPlan,
    mode: ManualSupportTargetMode,
    acquire_root: bool,
    acquire_mode: bool,
    recheck_mode: PreArmCancellationFinalizationRecheckMode,
) -> Result<(), PreArmRecoveryContractError> {
    let actual: BTreeSet<_> = plan.paths().iter().map(|path| path.path_kind()).collect();
    let mut expected = BTreeSet::from([
        PreArmCancellationFinalizationExecutionPathKind::Success,
        PreArmCancellationFinalizationExecutionPathKind::CapabilityBreachStop,
    ]);
    if acquire_root {
        expected
            .insert(PreArmCancellationFinalizationExecutionPathKind::RootGuardConflictCompensation);
    }
    if acquire_mode {
        expected.insert(
            PreArmCancellationFinalizationExecutionPathKind::ModeLeaseUnavailableBeforeAcquisitionCompensation,
        );
        if mode == ManualSupportTargetMode::SeparateWorkingInfobase {
            expected.insert(
                PreArmCancellationFinalizationExecutionPathKind::ModeLeaseUnavailableAfterAcquisitionCompensation,
            );
        }
    }
    if recheck_mode == PreArmCancellationFinalizationRecheckMode::ReplannableBeforeUpdate {
        expected.insert(PreArmCancellationFinalizationExecutionPathKind::RecheckReplanCompensation);
    }
    if actual != expected {
        return Err(PreArmRecoveryContractError(
            "execution path kinds do not match applicable failure branches",
        ));
    }

    let success_ids = plan
        .paths()
        .iter()
        .find(|path| path.path_kind() == PreArmCancellationFinalizationExecutionPathKind::Success)
        .expect("execution path plan constructor requires success")
        .action_ids();
    let has_update_action = receipt_plan
        .selective_update_effect_receipt()
        .is_some_and(|receipt| {
            receipt.source() == PreArmCancellationReceiptSource::FinalizationPlan
        });
    let has_cancellation_action = receipt_plan.cancellation_persistence_receipt().source()
        == PreArmCancellationReceiptSource::FinalizationPlan;
    let has_mode_release_action = receipt_plan.mode_lease_release_receipt().source()
        == PreArmCancellationReceiptSource::FinalizationPlan;
    let has_root_release_action = receipt_plan.root_guard_release_receipt().source()
        == PreArmCancellationReceiptSource::FinalizationPlan;
    if acquire_mode && !has_root_release_action {
        return Err(PreArmRecoveryContractError(
            "mode acquisition requires a finalization-plan root-release action",
        ));
    }
    let expected_success_len = usize::from(acquire_root)
        + usize::from(acquire_mode)
        + 1
        + usize::from(has_update_action)
        + usize::from(has_cancellation_action)
        + usize::from(has_mode_release_action)
        + usize::from(has_root_release_action)
        + 1;
    if success_ids.len() != expected_success_len {
        return Err(PreArmRecoveryContractError(
            "success path action count disagrees with the exact receipt projection",
        ));
    }

    let mut position = 0;
    let root_acquire_action = acquire_root.then(|| {
        let value = success_ids[position].clone();
        position += 1;
        value
    });
    let mode_acquire_action = acquire_mode.then(|| {
        let value = success_ids[position].clone();
        position += 1;
        value
    });
    let recheck_action = success_ids[position].clone();
    position += 1;
    if has_update_action {
        position += 1;
    }
    if has_cancellation_action {
        position += 1;
    }
    let mode_release_action = has_mode_release_action.then(|| {
        let value = success_ids[position].clone();
        position += 1;
        value
    });
    let root_release_action = has_root_release_action.then(|| {
        let value = success_ids[position].clone();
        position += 1;
        value
    });
    position += 1; // recovery finalization
    debug_assert_eq!(position, success_ids.len());

    let mut acquisitions = Vec::new();
    acquisitions.extend(root_acquire_action.iter().cloned());
    acquisitions.extend(mode_acquire_action.iter().cloned());
    for path in plan.paths() {
        let expected_ids = match path.path_kind() {
            PreArmCancellationFinalizationExecutionPathKind::Success => continue,
            PreArmCancellationFinalizationExecutionPathKind::CapabilityBreachStop => {
                let mut values = acquisitions.clone();
                values.push(recheck_action.clone());
                values
            }
            PreArmCancellationFinalizationExecutionPathKind::RootGuardConflictCompensation => {
                vec![root_acquire_action.clone().ok_or(PreArmRecoveryContractError(
                    "root-conflict path exists without a root acquisition",
                ))?]
            }
            PreArmCancellationFinalizationExecutionPathKind::ModeLeaseUnavailableBeforeAcquisitionCompensation => {
                let mut values = acquisitions.clone();
                values.push(root_release_action.clone().ok_or(PreArmRecoveryContractError(
                    "mode compensation lacks a root-release action",
                ))?);
                values
            }
            PreArmCancellationFinalizationExecutionPathKind::ModeLeaseUnavailableAfterAcquisitionCompensation => {
                let mut values = acquisitions.clone();
                values.push(mode_release_action.clone().ok_or(PreArmRecoveryContractError(
                    "mode compensation lacks a mode-release action",
                ))?);
                values.push(root_release_action.clone().ok_or(PreArmRecoveryContractError(
                    "mode compensation lacks a root-release action",
                ))?);
                values
            }
            PreArmCancellationFinalizationExecutionPathKind::RecheckReplanCompensation => {
                let mut values = acquisitions.clone();
                values.push(recheck_action.clone());
                values.push(mode_release_action.clone().ok_or(PreArmRecoveryContractError(
                    "replan compensation lacks a mode-release action",
                ))?);
                values.push(root_release_action.clone().ok_or(PreArmRecoveryContractError(
                    "replan compensation lacks a root-release action",
                ))?);
                values
            }
        };
        if path.action_ids() != expected_ids {
            return Err(PreArmRecoveryContractError(
                "execution path action sequence disagrees with the success/receipt projection",
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
pub(crate) fn archive_outcome_fixture_test_only(
    include_selective_update: bool,
    expected_postconditions: Vec<(PreArmCancellationEffectKind, Sha256Digest)>,
) -> (
    PreArmCancellationEffectObservation,
    PreArmCancellationFinalizationPlan,
    PreArmCancellationFinalizationRecheckEvidence,
) {
    tests::archive_outcome_fixture(include_selective_update, expected_postconditions)
}

#[cfg(test)]
pub(crate) fn archive_selective_update_proof_test_only(
    plan: &PreArmCancellationFinalizationPlan,
) -> SelectiveRepositoryUpdateProof {
    assert_eq!(
        plan.receipt_plan.selective_update_disposition(),
        PreArmCancellationSelectiveUpdateDisposition::NotRequired,
        "test helper currently covers the no-update archive branch",
    );
    let endpoint = plan
        .finalization_history_partition
        .through_inclusive()
        .clone();
    SelectiveRepositoryUpdateProof::recovery_finalization_already_exact_test_only(
        &plan.selective_update_plan,
        plan.receipt_plan
            .root_guard_acquisition_receipt()
            .receipt_id()
            .clone(),
        plan.expected_final_original_fingerprint.clone(),
        plan.expected_final_original_fingerprint.clone(),
        endpoint.clone(),
        endpoint,
    )
    .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::branched_development::contracts::repository::{
        CanonicalEmptyDeltaDigest, EvidenceKind, EvidenceSourceIndex, EvidenceSourceIndexCandidate,
        EvidenceSourceIndexCandidateRow, EvidenceSourceRegistry, RepositoryContractError,
        RepositoryHistoryEvidenceBytesResolver, RepositoryHistoryOrderEvidence,
        RepositoryHistoryOrderResolver, RepositoryHistoryPartitionResolver,
        RepositoryHistorySourceEvidenceRef, RepositoryUpdateLockTargets,
        RoutineRepositoryVersionClassificationEvidence, UnvalidatedRepositoryHistoryPartition,
    };
    use crate::domain::branched_development::contracts::scalars::RepositoryVersion;
    use crate::domain::branched_development::contracts::schema::audit_json_schema;
    use schemars::{schema_for, JsonSchema};
    use serde::de::DeserializeOwned;
    use serde_json::{json, Value};
    use sha2::{Digest as _, Sha256};
    use std::collections::BTreeMap;

    const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const C: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
    const ID_1: &str = "11111111-1111-4111-8111-111111111111";
    const ID_2: &str = "22222222-2222-4222-8222-222222222222";
    const ID_3: &str = "33333333-3333-4333-8333-333333333333";
    const ID_4: &str = "44444444-4444-4444-8444-444444444444";
    const ID_5: &str = "55555555-5555-4555-8555-555555555555";
    const ID_6: &str = "66666666-6666-4666-8666-666666666666";
    const ID_7: &str = "77777777-7777-4777-8777-777777777777";
    const ID_8: &str = "88888888-8888-4888-8888-888888888888";
    const ID_9: &str = "99999999-9999-4999-8999-999999999999";

    fn digest(value: &str) -> Sha256Digest {
        Sha256Digest::parse(value).unwrap()
    }

    fn id(value: &str) -> UnicaId {
        UnicaId::parse(value).unwrap()
    }

    fn operation_id(value: &str) -> OperationId {
        OperationId::parse(value).unwrap()
    }

    fn capability(value: &str) -> CapabilityRowId {
        CapabilityRowId::parse(value).unwrap()
    }

    fn cursor(version: &str, prefix_digest: &str) -> RepositoryHistoryCursor {
        serde_json::from_value(json!({
            "throughVersion": version,
            "historyPrefixDigest": prefix_digest,
        }))
        .unwrap()
    }

    fn empty_targets() -> RepositoryTargetStates {
        serde_json::from_value(json!([])).unwrap()
    }

    fn root_target() -> RepositoryTargetStates {
        serde_json::from_value(json!([{
            "targetKind": "configurationRoot",
            "state": "present",
            "repositoryVersion": "17",
            "targetFingerprint": A,
        }]))
        .unwrap()
    }

    fn receipt(
        receipt_id: &str,
        kind: PreArmCancellationEffectKind,
        intent: &str,
    ) -> PreArmCancellationEffectReceipt {
        PreArmCancellationEffectReceipt::new(
            PreArmCancellationEffectReceiptAuthority::test_only(
                id(receipt_id),
                kind,
                digest(intent),
                id(ID_8),
                digest(B),
                vec![digest(A), digest(B)],
            )
            .unwrap(),
        )
        .unwrap()
    }

    fn prior_ref(
        receipt_id: &str,
        kind: PreArmCancellationEffectKind,
        intent: &str,
    ) -> PreArmCancellationReceiptRef {
        PreArmCancellationReceiptRef::prior_operation(receipt(receipt_id, kind, intent))
    }

    fn prior_ref_with_digest(
        receipt_id: &str,
        kind: PreArmCancellationEffectKind,
        intent: Sha256Digest,
    ) -> PreArmCancellationReceiptRef {
        PreArmCancellationReceiptRef::prior_operation(
            PreArmCancellationEffectReceipt::new(
                PreArmCancellationEffectReceiptAuthority::test_only(
                    id(receipt_id),
                    kind,
                    intent,
                    id(ID_8),
                    digest(B),
                    vec![digest(A), digest(B)],
                )
                .unwrap(),
            )
            .unwrap(),
        )
    }

    fn prior_operation_intent(
        kind: PreArmCancellationEffectKind,
        selective_update_plan_digest: Sha256Digest,
        expected_postcondition_digest: Sha256Digest,
    ) -> Sha256Digest {
        prearm_digest(
            &PriorOperationEffectIntentDigestRecord {
                effect_kind: kind,
                prior_operation_id: operation_id(ID_2),
                support_action_id: id(ID_1),
                expected_support_action_digest: digest(A),
                approved_cancellation_digest: digest(B),
                manual_target_mode: ManualSupportTargetMode::ReservedOriginal,
                selective_update_plan_digest,
                expected_postcondition_digest,
            },
            "test prior-operation intent failed",
        )
        .unwrap()
    }

    fn future_ref(
        receipt_id: &str,
        kind: PreArmCancellationEffectKind,
        intent: &str,
    ) -> PreArmCancellationReceiptRef {
        PreArmCancellationReceiptRef::finalization_plan(id(receipt_id), kind, digest(intent))
    }

    fn future_ref_with_digest(
        receipt_id: &str,
        kind: PreArmCancellationEffectKind,
        intent: Sha256Digest,
    ) -> PreArmCancellationReceiptRef {
        PreArmCancellationReceiptRef::finalization_plan(id(receipt_id), kind, intent)
    }

    fn finalization_intent(
        kind: PreArmCancellationEffectKind,
        attempt_id: &UnicaId,
        observation: &PreArmCancellationEffectObservation,
        expected_postcondition_digest: Sha256Digest,
    ) -> Sha256Digest {
        prearm_digest(
            &FinalizationPlanEffectIntentDigestRecord {
                effect_kind: kind,
                finalization_attempt_id: attempt_id.clone(),
                support_action_id: observation.support_action_id().clone(),
                expected_support_action_digest: observation
                    .expected_support_action_digest()
                    .clone(),
                approved_cancellation_digest: observation.approved_cancellation_digest().clone(),
                effect_observation_digest: observation.observation_digest().clone(),
                manual_target_mode: observation.manual_target_mode(),
                selective_update_plan_digest: observation.selective_update_plan_digest().clone(),
                expected_postcondition_digest,
            },
            "test finalization intent failed",
        )
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
            _repository_version: &RepositoryVersion,
            _registry: &EvidenceSourceRegistry,
        ) -> Result<EvidenceSourceIndexCandidate, RepositoryContractError> {
            panic!("empty partition must not consult source index")
        }
    }

    struct UnexpectedOrder;

    impl RepositoryHistoryOrderResolver for UnexpectedOrder {
        fn order_evidence(
            &self,
            _from_exclusive: &RepositoryHistoryCursor,
            _through_inclusive: &RepositoryHistoryCursor,
        ) -> Result<RepositoryHistoryOrderEvidence, RepositoryContractError> {
            panic!("empty partition must not consult history order")
        }
    }

    struct UnexpectedBytes;

    impl RepositoryHistoryEvidenceBytesResolver for UnexpectedBytes {
        fn load_canonical_evidence_bytes(
            &self,
            _reference: &crate::domain::branched_development::contracts::repository::RepositoryHistorySourceEvidenceRef,
        ) -> Result<Vec<u8>, RepositoryContractError> {
            panic!("empty partition must not load evidence bytes")
        }
    }

    #[derive(Clone)]
    struct FixtureIndex {
        candidates: BTreeMap<String, EvidenceSourceIndexCandidate>,
    }

    impl EvidenceSourceIndex for FixtureIndex {
        fn candidate_for(
            &self,
            repository_version: &RepositoryVersion,
            _registry: &EvidenceSourceRegistry,
        ) -> Result<EvidenceSourceIndexCandidate, RepositoryContractError> {
            Ok(self
                .candidates
                .get(repository_version.as_str())
                .expect("routine fixture candidate")
                .clone())
        }
    }

    #[derive(Clone)]
    struct FixtureOrder(RepositoryHistoryOrderEvidence);

    impl RepositoryHistoryOrderResolver for FixtureOrder {
        fn order_evidence(
            &self,
            _from_exclusive: &RepositoryHistoryCursor,
            _through_inclusive: &RepositoryHistoryCursor,
        ) -> Result<RepositoryHistoryOrderEvidence, RepositoryContractError> {
            Ok(self.0.clone())
        }
    }

    struct FixtureBytes {
        values: BTreeMap<(EvidenceKind, String), Vec<u8>>,
    }

    impl RepositoryHistoryEvidenceBytesResolver for FixtureBytes {
        fn load_canonical_evidence_bytes(
            &self,
            reference: &RepositoryHistorySourceEvidenceRef,
        ) -> Result<Vec<u8>, RepositoryContractError> {
            Ok(self
                .values
                .get(&(
                    reference.evidence_kind(),
                    reference.evidence_digest().as_str().to_owned(),
                ))
                .expect("routine fixture evidence bytes")
                .clone())
        }
    }

    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct RoutineSemanticDigestRecord {
        repository_version: RepositoryVersion,
        partition_classification: RepositoryHistoryPartitionClassification,
        root_delta_digest: RequiredNullable<Sha256Digest>,
        content_delta_digest: RequiredNullable<Sha256Digest>,
        classification_digest: RequiredNullable<Sha256Digest>,
        external_support_disjointness_digest: RequiredNullable<Sha256Digest>,
        corrective_instruction_digest: RequiredNullable<Sha256Digest>,
        non_conflicting_concurrent_evidence_digest: RequiredNullable<Sha256Digest>,
    }

    impl contract_digest_record_sealed::Sealed for RoutineSemanticDigestRecord {}
    impl ContractDigestRecord for RoutineSemanticDigestRecord {}

    fn routine_partition(
        from_version: &str,
        from_prefix: &str,
        entries: &[(&str, &str, bool)],
    ) -> ValidatedRepositoryHistoryPartition {
        assert!(!entries.is_empty());
        let registry = EvidenceSourceRegistry::task8().unwrap();
        let mut candidates = BTreeMap::new();
        let mut bytes = BTreeMap::new();
        let mut wire_entries = Vec::new();
        let mut ordered_cursors = Vec::new();
        for (index, (version, prefix, relevant)) in entries.iter().enumerate() {
            let relevance = if *relevant { "relevant" } else { "unrelated" };
            let classification = if *relevant {
                RepositoryHistoryPartitionClassification::RelevantRoutine
            } else {
                RepositoryHistoryPartitionClassification::UnrelatedRoutine
            };
            let evidence =
                RoutineRepositoryVersionClassificationEvidence::new(version, relevance, None, A, B)
                    .unwrap();
            let evidence_value = serde_json::to_value(&evidence).unwrap();
            let classification_digest =
                digest(evidence_value["classificationDigest"].as_str().unwrap());
            let source_ref = RepositoryHistorySourceEvidenceRef::new(
                EvidenceKind::RoutineClassification,
                classification_digest.as_str(),
            )
            .unwrap();
            let semantic_delta_digest = prearm_digest(
                &RoutineSemanticDigestRecord {
                    repository_version: RepositoryVersion::parse(version).unwrap(),
                    partition_classification: classification,
                    root_delta_digest: RequiredNullable::value(digest(A)),
                    content_delta_digest: RequiredNullable::value(digest(B)),
                    classification_digest: RequiredNullable::value(classification_digest),
                    external_support_disjointness_digest: RequiredNullable::null(),
                    corrective_instruction_digest: RequiredNullable::null(),
                    non_conflicting_concurrent_evidence_digest: RequiredNullable::null(),
                },
                "routine semantic fixture digest failed",
            )
            .unwrap();
            wire_entries.push(json!({
                "repositoryVersion": version,
                "classification": if *relevant { "relevantRoutine" } else { "unrelatedRoutine" },
                "semanticDeltaDigest": semantic_delta_digest,
                "sourceEvidenceRef": source_ref,
            }));
            candidates.insert(
                (*version).to_owned(),
                EvidenceSourceIndexCandidate::from_capability_adapter(
                    version,
                    registry.registry_digest().as_str(),
                    [ID_1, ID_2, ID_3, ID_4][index],
                    vec![
                        EvidenceSourceIndexCandidateRow::available(
                            EvidenceKind::RoutineClassification,
                            vec![source_ref.clone()],
                        ),
                        EvidenceSourceIndexCandidateRow::absent(
                            EvidenceKind::SupportPrerequisiteObservation,
                        ),
                        EvidenceSourceIndexCandidateRow::absent(
                            EvidenceKind::NonConflictingConcurrent,
                        ),
                    ],
                )
                .unwrap(),
            );
            bytes.insert(
                (
                    source_ref.evidence_kind(),
                    source_ref.evidence_digest().as_str().to_owned(),
                ),
                serde_json_canonicalizer::to_vec(&evidence).unwrap(),
            );
            ordered_cursors.push(cursor(version, prefix));
        }
        let from = cursor(from_version, from_prefix);
        let through = ordered_cursors.last().unwrap().clone();
        let partition_digest = canonical_contract_digest(
            &EmptyPartitionDigestRecord {
                from_exclusive: from.clone(),
                through_inclusive: through.clone(),
                entries: wire_entries.clone(),
            },
            None,
        )
        .unwrap();
        let wire = serde_json::from_value::<UnvalidatedRepositoryHistoryPartition>(json!({
            "fromExclusive": from,
            "throughInclusive": through,
            "entries": wire_entries,
            "partitionDigest": partition_digest,
        }))
        .unwrap();
        let order = FixtureOrder(
            RepositoryHistoryOrderEvidence::from_capability_adapter(
                "prearm-routine-order-v1",
                cursor(from_version, from_prefix),
                ordered_cursors.last().unwrap().clone(),
                ordered_cursors,
            )
            .unwrap(),
        );
        RepositoryHistoryPartitionResolver::new(
            &registry,
            &FixtureIndex { candidates },
            &order,
            &FixtureBytes { values: bytes },
        )
        .validate(wire)
        .unwrap()
    }

    fn support_partition(
        classification: RepositoryHistoryPartitionClassification,
    ) -> ValidatedRepositoryHistoryPartition {
        let (classification_wire, mut observation, content_delta, external_disjointness) =
            match classification {
                RepositoryHistoryPartitionClassification::ExternalSupport => (
                    "externalSupport",
                    json!({
                        "repositoryVersion": "17",
                        "classification": "externalSupport",
                        "classificationDigest": A,
                        "mismatchKinds": [],
                        "repositoryActor": {
                            "username": "repository-user",
                            "computer": null,
                            "infobase": null,
                        },
                        "rootDeltaDigest": A,
                        "contentDeltaDigest": CanonicalEmptyDeltaDigest::VALUE,
                        "provenNotThisAction": true,
                        "overlapWithAuthorizedTransitions": false,
                        "supportOnlyDelta": true,
                        "externalSupportDisjointnessDigest": B,
                        "externalOwnershipEvidence": {
                            "kind": "supportPrerequisiteReceipt",
                            "receiptId": ID_1,
                            "receiptDigest": A,
                        },
                    }),
                    CanonicalEmptyDeltaDigest::VALUE,
                    Some(digest(B)),
                ),
                RepositoryHistoryPartitionClassification::PreArmExternal => (
                    "preArmExternal",
                    json!({
                        "repositoryVersion": "17",
                        "classification": "preArmExternal",
                        "classificationDigest": A,
                        "mismatchKinds": ["armingOrderViolated"],
                        "pendingSupportActionId": ID_1,
                        "pendingSupportActionDigest": A,
                        "authorizationState": "awaitingArm",
                        "armingReceiptAbsent": true,
                        "repositoryActor": {
                            "username": "repository-user",
                            "computer": null,
                            "infobase": null,
                        },
                        "rootDeltaDigest": A,
                        "contentDeltaDigest": B,
                        "supportTransitionsDigest": A,
                        "preserveAsExternalBaseline": true,
                    }),
                    B,
                    None,
                ),
                _ => panic!("support fixture requires an external classification"),
            };
        let mut digest_record = observation.clone();
        digest_record
            .as_object_mut()
            .unwrap()
            .remove("classificationDigest");
        let classification_digest = digest(&format!(
            "{:x}",
            Sha256::digest(serde_json_canonicalizer::to_vec(&digest_record).unwrap())
        ));
        observation["classificationDigest"] = json!(classification_digest);

        let source_ref = RepositoryHistorySourceEvidenceRef::new(
            EvidenceKind::SupportPrerequisiteObservation,
            classification_digest.as_str(),
        )
        .unwrap();
        let semantic_delta_digest = prearm_digest(
            &RoutineSemanticDigestRecord {
                repository_version: RepositoryVersion::parse("17").unwrap(),
                partition_classification: classification,
                root_delta_digest: RequiredNullable::value(digest(A)),
                content_delta_digest: RequiredNullable::value(digest(content_delta)),
                classification_digest: RequiredNullable::value(classification_digest),
                external_support_disjointness_digest: external_disjointness
                    .map_or_else(RequiredNullable::null, RequiredNullable::value),
                corrective_instruction_digest: RequiredNullable::null(),
                non_conflicting_concurrent_evidence_digest: RequiredNullable::null(),
            },
            "support semantic fixture digest failed",
        )
        .unwrap();
        let from = cursor("16", A);
        let through = cursor("17", B);
        let wire_entries = vec![json!({
            "repositoryVersion": "17",
            "classification": classification_wire,
            "semanticDeltaDigest": semantic_delta_digest,
            "sourceEvidenceRef": source_ref,
        })];
        let partition_digest = canonical_contract_digest(
            &EmptyPartitionDigestRecord {
                from_exclusive: from.clone(),
                through_inclusive: through.clone(),
                entries: wire_entries.clone(),
            },
            None,
        )
        .unwrap();
        let wire = serde_json::from_value::<UnvalidatedRepositoryHistoryPartition>(json!({
            "fromExclusive": from.clone(),
            "throughInclusive": through.clone(),
            "entries": wire_entries,
            "partitionDigest": partition_digest,
        }))
        .unwrap();

        let registry = EvidenceSourceRegistry::task8().unwrap();
        let candidate = EvidenceSourceIndexCandidate::from_capability_adapter(
            "17",
            registry.registry_digest().as_str(),
            ID_1,
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
                    vec![source_ref.clone()],
                ),
                EvidenceSourceIndexCandidateRow::available(
                    EvidenceKind::NonConflictingConcurrent,
                    vec![RepositoryHistorySourceEvidenceRef::new(
                        EvidenceKind::NonConflictingConcurrent,
                        B,
                    )
                    .unwrap()],
                ),
            ],
        )
        .unwrap();
        RepositoryHistoryPartitionResolver::new(
            &registry,
            &FixtureIndex {
                candidates: BTreeMap::from([("17".to_owned(), candidate)]),
            },
            &FixtureOrder(
                RepositoryHistoryOrderEvidence::from_capability_adapter(
                    "prearm-support-order-v1",
                    from,
                    through.clone(),
                    vec![through],
                )
                .unwrap(),
            ),
            &FixtureBytes {
                values: BTreeMap::from([(
                    (
                        EvidenceKind::SupportPrerequisiteObservation,
                        source_ref.evidence_digest().as_str().to_owned(),
                    ),
                    serde_json_canonicalizer::to_vec(&observation).unwrap(),
                )]),
            },
        )
        .validate(wire)
        .unwrap()
    }

    fn empty_partition() -> ValidatedRepositoryHistoryPartition {
        let endpoint = cursor("16", A);
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

    fn schema<T: JsonSchema>() -> Value {
        serde_json::to_value(schema_for!(T)).unwrap()
    }

    fn contains_wire_literal(value: &Value, expected: &str) -> bool {
        match value {
            Value::Object(object) => object.iter().any(|(key, value)| {
                (key == "const" && value == expected)
                    || (key == "enum"
                        && value
                            .as_array()
                            .is_some_and(|values| values.iter().any(|value| value == expected)))
                    || contains_wire_literal(value, expected)
            }),
            Value::Array(values) => values
                .iter()
                .any(|value| contains_wire_literal(value, expected)),
            _ => false,
        }
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

    assert_not_deserialize_owned!(PreArmCancellationEffectReceipt);
    assert_not_deserialize_owned!(PreArmCancellationEffectReceiptAuthority);
    assert_not_deserialize_owned!(PreArmCancellationReceiptRef);
    assert_not_deserialize_owned!(PreArmCancellationEffectObservation);
    assert_not_deserialize_owned!(PreArmCancellationEffectObservationAuthority);
    assert_not_deserialize_owned!(PreArmCancellationFinalizationRecheckPolicy);
    assert_not_deserialize_owned!(PreArmCancellationFinalizationRecheckPolicyAuthority);
    assert_not_deserialize_owned!(PreArmCancellationFinalizationRecheckEvidence);
    assert_not_deserialize_owned!(PreArmCancellationFinalizationRecheckEvidenceAuthority);
    assert_not_deserialize_owned!(PreArmCancellationFinalizationPlan);
    assert_not_deserialize_owned!(PreArmCancellationFinalizationPlanAuthority);

    #[test]
    fn prearm_contract_surface_is_closed_and_has_no_observed_operation_state() {
        let schemas = [
            schema::<PreArmCancellationEffectReceipt>(),
            schema::<PreArmCancellationReceiptRef>(),
            schema::<PreArmCancellationUpdateProgress>(),
            schema::<PreArmCancellationEffectProgress>(),
            schema::<PreArmCancellationEffectObservation>(),
            schema::<PreArmCancellationReceiptPlan>(),
            schema::<PreArmCancellationFinalizationRecheckPolicy>(),
            schema::<PreArmCancellationFinalizationRecheckEvidence>(),
            schema::<PreArmCancellationFinalizationExecutionPathPlan>(),
            schema::<PreArmCancellationFinalizationAttemptProgress>(),
            schema::<PreArmCancellationFinalizationAttemptAudit>(),
            schema::<PreArmCancellationKnownBlocker>(),
            schema::<PreArmCancellationFinalizationPlan>(),
        ];

        for contract in schemas {
            audit_json_schema(&contract).unwrap();
            assert!(!contains_wire_literal(&contract, "observed"));
        }

        assert_eq!(
            serde_json::to_value([
                PreArmCancellationFinalizationAttemptState::NotStarted,
                PreArmCancellationFinalizationAttemptState::InProgress,
                PreArmCancellationFinalizationAttemptState::Compensating,
                PreArmCancellationFinalizationAttemptState::Compensated,
                PreArmCancellationFinalizationAttemptState::Completed,
            ])
            .unwrap(),
            json!([
                "notStarted",
                "inProgress",
                "compensating",
                "compensated",
                "completed"
            ])
        );
    }

    #[test]
    fn effect_receipt_and_ref_hash_exact_records_without_future_cycles() {
        assert!(PreArmCancellationEffectReceiptAuthority::test_only(
            id(ID_1),
            PreArmCancellationEffectKind::RootGuardAcquire,
            digest(A),
            id(ID_2),
            digest(B),
            Vec::new(),
        )
        .is_err());
        assert!(PreArmCancellationEffectReceiptAuthority::test_only(
            id(ID_1),
            PreArmCancellationEffectKind::RootGuardAcquire,
            digest(A),
            id(ID_2),
            digest(B),
            vec![digest(A), digest(A)],
        )
        .is_err());

        let receipt = PreArmCancellationEffectReceipt::new(
            PreArmCancellationEffectReceiptAuthority::test_only(
                id(ID_1),
                PreArmCancellationEffectKind::RootGuardAcquire,
                digest(C),
                id(ID_8),
                digest(B),
                vec![digest(B), digest(A)],
            )
            .unwrap(),
        )
        .unwrap();
        let expected = canonical_contract_digest(
            &PreArmCancellationEffectReceiptDigestRecord {
                receipt_kind: PreArmCancellationEffectReceiptKind::Value,
                receipt_id: id(ID_1),
                effect_kind: PreArmCancellationEffectKind::RootGuardAcquire,
                effect_intent_digest: digest(C),
                producer_action_id: id(ID_8),
                producer_action_digest: digest(B),
                terminal_observation_digests: NonEmptyOrderedDigests::new(vec![
                    digest(B),
                    digest(A),
                ])
                .unwrap(),
            },
            None,
        )
        .unwrap();
        assert_eq!(receipt.receipt_digest(), &expected);
        assert_eq!(
            receipt.terminal_observation_digests(),
            [digest(B), digest(A)]
        );
        assert_eq!(
            serde_json::to_value(&receipt).unwrap()["terminalObservationDigests"],
            json!([B, A])
        );

        let prior = PreArmCancellationReceiptRef::prior_operation(receipt);
        let prior_json = serde_json::to_value(&prior).unwrap();
        assert_eq!(prior_json["source"], json!("priorOperation"));
        assert!(prior_json["receipt"].get("receiptDigest").is_some());

        let future = future_ref(ID_2, PreArmCancellationEffectKind::ModeLeaseAcquire, A);
        let future_json = serde_json::to_value(&future).unwrap();
        assert_eq!(future_json["source"], json!("finalizationPlan"));
        assert!(future_json.get("receipt").is_none());
        assert!(future_json.get("receiptDigest").is_none());
        assert!(future_json.get("producerActionId").is_none());
    }

    #[test]
    fn original_effect_progress_rejects_future_or_wrong_kind_receipts() {
        assert!(
            PreArmCancellationEffectProgress::root_held_before_lease_test_only(future_ref(
                ID_1,
                PreArmCancellationEffectKind::RootGuardAcquire,
                A,
            ))
            .is_err()
        );
        assert!(
            PreArmCancellationEffectProgress::root_held_before_lease_test_only(prior_ref(
                ID_1,
                PreArmCancellationEffectKind::ModeLeaseAcquire,
                A,
            ))
            .is_err()
        );

        let progress = PreArmCancellationEffectProgress::guards_held_before_update_test_only(
            prior_ref(ID_1, PreArmCancellationEffectKind::RootGuardAcquire, A),
            prior_ref(ID_2, PreArmCancellationEffectKind::ModeLeaseAcquire, B),
        )
        .unwrap();
        assert_eq!(
            progress.stage(),
            PreArmCancellationEffectProgressStage::GuardsHeldBeforeUpdate
        );
        assert_eq!(
            serde_json::to_value(progress).unwrap()["stage"],
            json!("guardsHeldBeforeUpdate")
        );
    }

    #[test]
    fn selective_progress_derives_empty_and_already_exact_presence() {
        let empty = ApprovedSelectiveUpdatePlanProjection::test_only(
            empty_targets(),
            capability("selective-recovery-v1"),
            None,
        )
        .unwrap();
        assert_eq!(
            PreArmCancellationUpdateProgress::not_required(&empty)
                .unwrap()
                .state(),
            PreArmCancellationUpdateState::NotRequired
        );

        let non_empty = ApprovedSelectiveUpdatePlanProjection::test_only(
            root_target(),
            capability("selective-recovery-v1"),
            Some(capability("structural-recovery-v1")),
        )
        .unwrap();
        assert!(PreArmCancellationUpdateProgress::not_required(&non_empty).is_err());
        let evidence = PreArmCancellationSelectiveUpdateAlreadyExactEvidence::new(
            &non_empty,
            prior_ref(ID_1, PreArmCancellationEffectKind::RootGuardAcquire, A),
            prior_ref(ID_2, PreArmCancellationEffectKind::ModeLeaseAcquire, B),
            digest(A),
            digest(B),
            cursor("17", C),
        )
        .unwrap();
        let encoded = serde_json::to_value(&evidence).unwrap();
        assert_eq!(encoded["structuralConfirmationRequired"], json!(true));
        assert_eq!(encoded["structuralConfirmationUsed"], json!(false));
        assert_eq!(
            encoded["structuralCapabilityRowId"],
            json!("structural-recovery-v1")
        );
        assert_eq!(
            PreArmCancellationUpdateProgress::already_exact(evidence).state(),
            PreArmCancellationUpdateState::AlreadyExact
        );
    }

    #[test]
    fn effect_observation_rejects_update_progress_from_another_selective_plan() {
        let embedded_plan = ApprovedSelectiveUpdatePlanProjection::test_only(
            root_target(),
            capability("selective-recovery-v1"),
            Some(capability("structural-recovery-v1")),
        )
        .unwrap();
        let bound_plan = ApprovedSelectiveUpdatePlanProjection::test_only(
            root_target(),
            capability("selective-recovery-v2"),
            Some(capability("structural-recovery-v1")),
        )
        .unwrap();
        assert_ne!(embedded_plan.plan_digest, bound_plan.plan_digest);

        assert!(
            update_ready_observation(&embedded_plan, bound_plan.plan_digest.clone(), false,)
                .is_err()
        );
        assert!(update_ready_observation(&embedded_plan, bound_plan.plan_digest, true,).is_err());
    }

    fn no_guard_observation_with_history(
        selective_update_plan_digest: Sha256Digest,
        history_partition: ValidatedRepositoryHistoryPartition,
    ) -> PreArmCancellationEffectObservation {
        PreArmCancellationEffectObservation::new_test_only(
            PreArmCancellationEffectObservationAuthority {
                observation_id: id(ID_1),
                prior_operation_id: operation_id(ID_2),
                support_action_id: id(ID_1),
                expected_support_action_digest: digest(A),
                approved_cancellation_digest: digest(B),
                bound_cancelled_phase: TaskPhase::Synchronized,
                bound_relevant_advance_phase: TaskPhase::LocalVerified,
                manual_target_mode: ManualSupportTargetMode::ReservedOriginal,
                effect_progress: PreArmCancellationEffectProgress::no_guard_test_only(),
                history_partition,
                observed_original_fingerprint: digest(C),
                observed_support_graph_digest: digest(A),
                bound_selective_update_plan_digest: selective_update_plan_digest,
                expected_postcondition_digests:
                    PreArmCancellationExpectedPostconditionDigests::default(),
            },
        )
        .unwrap()
    }

    fn no_guard_observation(
        selective_update_plan_digest: Sha256Digest,
    ) -> PreArmCancellationEffectObservation {
        no_guard_observation_with_history(selective_update_plan_digest, empty_partition())
    }

    fn update_ready_observation(
        embedded_plan: &ApprovedSelectiveUpdatePlanProjection,
        bound_plan_digest: Sha256Digest,
        applied: bool,
    ) -> Result<PreArmCancellationEffectObservation, PreArmRecoveryContractError> {
        let root_postcondition = digest(A);
        let mode_postcondition = digest(B);
        let update_postcondition = digest(C);
        let root = prior_ref_with_digest(
            ID_4,
            PreArmCancellationEffectKind::RootGuardAcquire,
            prior_operation_intent(
                PreArmCancellationEffectKind::RootGuardAcquire,
                bound_plan_digest.clone(),
                root_postcondition.clone(),
            ),
        );
        let mode = prior_ref_with_digest(
            ID_5,
            PreArmCancellationEffectKind::ModeLeaseAcquire,
            prior_operation_intent(
                PreArmCancellationEffectKind::ModeLeaseAcquire,
                bound_plan_digest.clone(),
                mode_postcondition.clone(),
            ),
        );
        let update_progress = if applied {
            let update = prior_ref_with_digest(
                ID_6,
                PreArmCancellationEffectKind::SelectiveOriginalUpdate,
                prior_operation_intent(
                    PreArmCancellationEffectKind::SelectiveOriginalUpdate,
                    bound_plan_digest.clone(),
                    update_postcondition.clone(),
                ),
            );
            PreArmCancellationUpdateProgress::applied(
                embedded_plan,
                PreArmCancellationSelectiveUpdateEffectAuthority {
                    update_effect_receipt: update,
                    root_guard_acquisition_receipt: root.clone(),
                    mode_lease_acquisition_receipt: mode.clone(),
                    applied_targets: embedded_plan.planned_targets.clone(),
                    before_original_target_fingerprint_digest: digest(A),
                    verified_original_target_fingerprint_digest: digest(B),
                    observed_before_cursor: cursor("16", A),
                    observed_effect_cursor: cursor("17", B),
                },
            )?
        } else {
            PreArmCancellationUpdateProgress::already_exact(
                PreArmCancellationSelectiveUpdateAlreadyExactEvidence::new(
                    embedded_plan,
                    root.clone(),
                    mode.clone(),
                    digest(A),
                    digest(B),
                    cursor("17", C),
                )?,
            )
        };
        let effect_progress =
            PreArmCancellationEffectProgress::update_ready_test_only(root, mode, update_progress)?;
        let mut postconditions = vec![
            (
                PreArmCancellationEffectKind::RootGuardAcquire,
                root_postcondition,
            ),
            (
                PreArmCancellationEffectKind::ModeLeaseAcquire,
                mode_postcondition,
            ),
        ];
        if applied {
            postconditions.push((
                PreArmCancellationEffectKind::SelectiveOriginalUpdate,
                update_postcondition,
            ));
        }
        PreArmCancellationEffectObservation::new_test_only(
            PreArmCancellationEffectObservationAuthority {
                observation_id: id(ID_9),
                prior_operation_id: operation_id(ID_2),
                support_action_id: id(ID_1),
                expected_support_action_digest: digest(A),
                approved_cancellation_digest: digest(B),
                bound_cancelled_phase: TaskPhase::Synchronized,
                bound_relevant_advance_phase: TaskPhase::LocalVerified,
                manual_target_mode: ManualSupportTargetMode::ReservedOriginal,
                effect_progress,
                history_partition: empty_partition(),
                observed_original_fingerprint: digest(C),
                observed_support_graph_digest: digest(A),
                bound_selective_update_plan_digest: bound_plan_digest,
                expected_postcondition_digests:
                    PreArmCancellationExpectedPostconditionDigests::from_pairs_test_only(
                        postconditions,
                    )?,
            },
        )
    }

    fn empty_recovery_update_plan() -> SelectiveRepositoryUpdatePlan {
        SelectiveRepositoryUpdatePlan::recovery_finalization_test_only(
            empty_targets(),
            serde_json::from_value::<RepositoryUpdateLockTargets>(json!([{
                "targetKind": "configurationRoot",
                "objectDisplay": "Configuration",
                "reasons": ["supportGraphGuard"],
            }]))
            .unwrap(),
            capability("selective-recovery-v1"),
            None,
        )
        .unwrap()
    }

    fn finalization_paths() -> PreArmCancellationFinalizationExecutionPathPlan {
        PreArmCancellationFinalizationExecutionPathPlan::new(vec![
            PreArmCancellationFinalizationExecutionPath::new(
                PreArmCancellationFinalizationExecutionPathKind::Success,
                [ID_1, ID_2, ID_3, ID_4, ID_5, ID_6, ID_7]
                    .into_iter()
                    .map(id)
                    .collect(),
            )
            .unwrap(),
            PreArmCancellationFinalizationExecutionPath::new(
                PreArmCancellationFinalizationExecutionPathKind::CapabilityBreachStop,
                [ID_1, ID_2, ID_3].into_iter().map(id).collect(),
            )
            .unwrap(),
            PreArmCancellationFinalizationExecutionPath::new(
                PreArmCancellationFinalizationExecutionPathKind::RootGuardConflictCompensation,
                vec![id(ID_1)],
            )
            .unwrap(),
            PreArmCancellationFinalizationExecutionPath::new(
                PreArmCancellationFinalizationExecutionPathKind::ModeLeaseUnavailableBeforeAcquisitionCompensation,
                [ID_1, ID_2, ID_6].into_iter().map(id).collect(),
            )
            .unwrap(),
            PreArmCancellationFinalizationExecutionPath::new(
                PreArmCancellationFinalizationExecutionPathKind::RecheckReplanCompensation,
                [ID_1, ID_2, ID_3, ID_5, ID_6]
                    .into_iter()
                    .map(id)
                    .collect(),
            )
            .unwrap(),
        ])
        .unwrap()
    }

    fn finalization_authority(
        attempt_id: UnicaId,
        observation: PreArmCancellationEffectObservation,
        selective_update_plan: SelectiveRepositoryUpdatePlan,
        finalization_history_partition: ValidatedRepositoryHistoryPartition,
        prior_attempt_lineage: PreArmCancellationAttemptAuditLineageAuthority,
        planned_result_phase: TaskPhase,
    ) -> PreArmCancellationFinalizationPlanAuthority {
        let slots = [
            (PreArmCancellationEffectKind::RootGuardAcquire, digest(A)),
            (PreArmCancellationEffectKind::ModeLeaseAcquire, digest(B)),
            (
                PreArmCancellationEffectKind::AuthorizationCancellation,
                digest(C),
            ),
            (PreArmCancellationEffectKind::ModeLeaseRelease, digest(A)),
            (PreArmCancellationEffectKind::RootGuardRelease, digest(B)),
            (
                PreArmCancellationEffectKind::RecoveryFinalization,
                digest(C),
            ),
        ];
        let expected_postcondition_digests =
            PreArmCancellationExpectedPostconditionDigests::from_pairs_test_only(slots.clone())
                .unwrap();
        let receipt = |receipt_id, slot: (PreArmCancellationEffectKind, Sha256Digest)| {
            future_ref_with_digest(
                receipt_id,
                slot.0,
                finalization_intent(slot.0, &attempt_id, &observation, slot.1),
            )
        };
        let projection =
            ApprovedSelectiveUpdatePlanProjection::from_plan(&selective_update_plan).unwrap();
        let receipt_plan = PreArmCancellationReceiptPlan::new(
            &projection,
            &observation,
            true,
            true,
            PreArmCancellationReceiptPlanAuthority {
                root_guard_acquisition_receipt: receipt(ID_1, slots[0].clone()),
                mode_lease_acquisition_receipt: receipt(ID_2, slots[1].clone()),
                selective_update_effect_receipt: None,
                cancellation_persistence_receipt: receipt(ID_4, slots[2].clone()),
                mode_lease_release_receipt: receipt(ID_5, slots[3].clone()),
                root_guard_release_receipt: receipt(ID_6, slots[4].clone()),
                recovery_finalization_receipt: receipt(ID_7, slots[5].clone()),
            },
        )
        .unwrap();
        let recheck_policy = PreArmCancellationFinalizationRecheckPolicy::new(
            PreArmCancellationFinalizationRecheckPolicyAuthority {
                kind: PreArmCancellationFinalizationRecheckPolicyAuthorityKind::ReplannableBeforeUpdate {
                    source_progress_stage: PreArmCancellationEffectProgressStage::NoGuard,
                    continuously_held_root: false,
                    continuously_held_mode_lease: false,
                    history_partition: finalization_history_partition.clone(),
                    expected_original_fingerprint: digest(C),
                    expected_support_graph_digest: digest(A),
                    pre_arm_freeze_digest: digest(B),
                },
            },
        )
        .unwrap();
        PreArmCancellationFinalizationPlanAuthority {
            finalization_attempt_id: attempt_id,
            effect_observation: observation,
            completion_mode:
                PreArmCancellationFinalizationCompletionMode::FinishCancellationAndRelease,
            starting_root_guard_state: PreArmCancellationStartingGuardState::Released,
            starting_mode_lease_state: PreArmCancellationStartingGuardState::Released,
            acquire_root_guard: true,
            acquire_mode_lease: true,
            receipt_plan,
            expected_postcondition_digests,
            recheck_policy,
            execution_path_plan: finalization_paths(),
            prior_attempt_lineage,
            finalization_history_partition,
            selective_update_plan,
            expected_final_original_fingerprint: digest(C),
            expected_final_support_graph_digest: digest(A),
            planned_result_phase,
        }
    }

    fn realized_finalization_receipts(
        plan: &PreArmCancellationFinalizationPlan,
    ) -> Vec<PreArmCancellationEffectReceipt> {
        plan.receipt_plan
            .ordered_refs_with_kinds()
            .into_iter()
            .filter(|(receipt_ref, _)| {
                receipt_ref.source() == PreArmCancellationReceiptSource::FinalizationPlan
            })
            .map(|(receipt_ref, effect_kind)| {
                PreArmCancellationEffectReceipt::new(
                    PreArmCancellationEffectReceiptAuthority::test_only(
                        receipt_ref.receipt_id().clone(),
                        effect_kind,
                        receipt_ref.effect_intent_digest().clone(),
                        id(ID_9),
                        digest(B),
                        vec![digest(A)],
                    )
                    .unwrap(),
                )
                .unwrap()
            })
            .collect()
    }

    #[derive(Clone)]
    struct MatchedTerminalFixture {
        observation: PreArmCancellationEffectObservation,
        plan: PreArmCancellationFinalizationPlan,
        evidence: PreArmCancellationFinalizationRecheckEvidence,
        progress: PreArmCancellationFinalizationAttemptProgress,
        cancellation_receipt_id: UnicaId,
        cancellation_receipt_digest: Sha256Digest,
        recovery_receipt_id: UnicaId,
        recovery_receipt_digest: Sha256Digest,
        selective_update_proof: SelectiveRepositoryUpdateProof,
        post_release_cursor: RepositoryHistoryCursor,
        post_apply_partition: ValidatedRepositoryHistoryPartition,
    }

    impl MatchedTerminalFixture {
        fn projection(
            &self,
            resulting_phase: TaskPhase,
        ) -> PreArmCancellationArchiveLineageProjection<'_> {
            PreArmCancellationArchiveLineageProjection {
                effect_observation: &self.observation,
                finalization_recheck_evidence: &self.evidence,
                completed_finalization_progress: &self.progress,
                support_cancellation_receipt_id: &self.cancellation_receipt_id,
                support_cancellation_receipt_digest: &self.cancellation_receipt_digest,
                pre_arm_recovery_receipt_id: &self.recovery_receipt_id,
                pre_arm_recovery_receipt_digest: &self.recovery_receipt_digest,
                selective_update_proof: &self.selective_update_proof,
                post_release_observed_history_cursor: &self.post_release_cursor,
                post_apply_history_partition: &self.post_apply_partition,
                deferred_repository_advance: None,
                resulting_phase,
            }
        }

        fn validate(&self, resulting_phase: TaskPhase) -> Result<(), PreArmRecoveryContractError> {
            self.plan
                .validate_archive_lineage_projection(self.projection(resulting_phase))
        }
    }

    fn matched_terminal_fixture() -> MatchedTerminalFixture {
        let selective_update_plan = empty_recovery_update_plan();
        let history = empty_partition();
        let observation = no_guard_observation_with_history(
            selective_update_plan.plan_digest().clone(),
            history.clone(),
        );
        let plan = PreArmCancellationFinalizationPlan::new_test_only(finalization_authority(
            id(ID_8),
            observation.clone(),
            selective_update_plan,
            history.clone(),
            PreArmCancellationAttemptAuditLineageAuthority::initial(),
            TaskPhase::Synchronized,
        ))
        .unwrap();
        let evidence = PreArmCancellationFinalizationRecheckEvidence::new(
            PreArmCancellationFinalizationRecheckEvidenceAuthority::matched_test_only(
                history.clone(),
                digest(C),
                digest(A),
            ),
        )
        .unwrap();
        let receipts = realized_finalization_receipts(&plan);
        let cancellation = receipts
            .iter()
            .find(|receipt| {
                receipt.effect_kind() == PreArmCancellationEffectKind::AuthorizationCancellation
            })
            .unwrap();
        let cancellation_receipt_id = cancellation.receipt_id().clone();
        let cancellation_receipt_digest = cancellation.receipt_digest().clone();
        let recovery = receipts
            .iter()
            .find(|receipt| {
                receipt.effect_kind() == PreArmCancellationEffectKind::RecoveryFinalization
            })
            .unwrap();
        let recovery_receipt_id = recovery.receipt_id().clone();
        let recovery_receipt_digest = recovery.receipt_digest().clone();
        let progress = PreArmCancellationFinalizationAttemptProgress::completed_test_only(
            plan.finalization_attempt_id().clone(),
            receipts,
            evidence.clone(),
        )
        .unwrap();
        let endpoint = history.through_inclusive().clone();
        let selective_update_proof =
            SelectiveRepositoryUpdateProof::recovery_finalization_already_exact_test_only(
                &plan.selective_update_plan,
                plan.receipt_plan
                    .root_guard_acquisition_receipt()
                    .receipt_id()
                    .clone(),
                digest(A),
                digest(C),
                endpoint.clone(),
                endpoint.clone(),
            )
            .unwrap();
        MatchedTerminalFixture {
            observation,
            plan,
            evidence,
            progress,
            cancellation_receipt_id,
            cancellation_receipt_digest,
            recovery_receipt_id,
            recovery_receipt_digest,
            selective_update_proof,
            post_release_cursor: endpoint,
            post_apply_partition: history,
        }
    }

    pub(super) fn archive_outcome_fixture(
        include_selective_update: bool,
        expected_postconditions: Vec<(PreArmCancellationEffectKind, Sha256Digest)>,
    ) -> (
        PreArmCancellationEffectObservation,
        PreArmCancellationFinalizationPlan,
        PreArmCancellationFinalizationRecheckEvidence,
    ) {
        let selective_update_plan = SelectiveRepositoryUpdatePlan::recovery_finalization_test_only(
            if include_selective_update {
                root_target()
            } else {
                empty_targets()
            },
            serde_json::from_value::<RepositoryUpdateLockTargets>(json!([{
                "targetKind": "configurationRoot",
                "objectDisplay": "Configuration",
                "reasons": ["supportGraphGuard"],
            }]))
            .unwrap(),
            capability("selective-recovery-v1"),
            None,
        )
        .unwrap();
        let history = empty_partition();
        let observation = no_guard_observation_with_history(
            selective_update_plan.plan_digest().clone(),
            history.clone(),
        );
        let attempt_id = id(ID_9);
        let postcondition = |kind| {
            expected_postconditions
                .iter()
                .find_map(|(candidate, digest)| (*candidate == kind).then(|| digest.clone()))
                .unwrap()
        };
        let receipt = |receipt_id, kind| {
            future_ref_with_digest(
                receipt_id,
                kind,
                finalization_intent(kind, &attempt_id, &observation, postcondition(kind)),
            )
        };
        let projection =
            ApprovedSelectiveUpdatePlanProjection::from_plan(&selective_update_plan).unwrap();
        let receipt_plan = PreArmCancellationReceiptPlan::new(
            &projection,
            &observation,
            true,
            true,
            PreArmCancellationReceiptPlanAuthority {
                root_guard_acquisition_receipt: receipt(
                    ID_1,
                    PreArmCancellationEffectKind::RootGuardAcquire,
                ),
                mode_lease_acquisition_receipt: receipt(
                    ID_2,
                    PreArmCancellationEffectKind::ModeLeaseAcquire,
                ),
                selective_update_effect_receipt: include_selective_update
                    .then(|| receipt(ID_3, PreArmCancellationEffectKind::SelectiveOriginalUpdate)),
                cancellation_persistence_receipt: receipt(
                    ID_4,
                    PreArmCancellationEffectKind::AuthorizationCancellation,
                ),
                mode_lease_release_receipt: receipt(
                    ID_5,
                    PreArmCancellationEffectKind::ModeLeaseRelease,
                ),
                root_guard_release_receipt: receipt(
                    ID_6,
                    PreArmCancellationEffectKind::RootGuardRelease,
                ),
                recovery_finalization_receipt: receipt(
                    ID_7,
                    PreArmCancellationEffectKind::RecoveryFinalization,
                ),
            },
        )
        .unwrap();
        let recheck_policy = PreArmCancellationFinalizationRecheckPolicy::new(
            PreArmCancellationFinalizationRecheckPolicyAuthority {
                kind: PreArmCancellationFinalizationRecheckPolicyAuthorityKind::ReplannableBeforeUpdate {
                    source_progress_stage: PreArmCancellationEffectProgressStage::NoGuard,
                    continuously_held_root: false,
                    continuously_held_mode_lease: false,
                    history_partition: history.clone(),
                    expected_original_fingerprint: digest(C),
                    expected_support_graph_digest: digest(A),
                    pre_arm_freeze_digest: digest(B),
                },
            },
        )
        .unwrap();
        let execution_path_plan = if include_selective_update {
            PreArmCancellationFinalizationExecutionPathPlan::new(vec![
                PreArmCancellationFinalizationExecutionPath::new(
                    PreArmCancellationFinalizationExecutionPathKind::Success,
                    [ID_1, ID_2, ID_3, ID_4, ID_5, ID_6, ID_7, ID_8]
                        .into_iter()
                        .map(id)
                        .collect(),
                )
                .unwrap(),
                PreArmCancellationFinalizationExecutionPath::new(
                    PreArmCancellationFinalizationExecutionPathKind::CapabilityBreachStop,
                    [ID_1, ID_2, ID_3].into_iter().map(id).collect(),
                )
                .unwrap(),
                PreArmCancellationFinalizationExecutionPath::new(
                    PreArmCancellationFinalizationExecutionPathKind::RootGuardConflictCompensation,
                    vec![id(ID_1)],
                )
                .unwrap(),
                PreArmCancellationFinalizationExecutionPath::new(
                    PreArmCancellationFinalizationExecutionPathKind::ModeLeaseUnavailableBeforeAcquisitionCompensation,
                    [ID_1, ID_2, ID_7].into_iter().map(id).collect(),
                )
                .unwrap(),
                PreArmCancellationFinalizationExecutionPath::new(
                    PreArmCancellationFinalizationExecutionPathKind::RecheckReplanCompensation,
                    [ID_1, ID_2, ID_3, ID_6, ID_7]
                        .into_iter()
                        .map(id)
                        .collect(),
                )
                .unwrap(),
            ])
            .unwrap()
        } else {
            finalization_paths()
        };
        let plan = PreArmCancellationFinalizationPlan::new_test_only(
            PreArmCancellationFinalizationPlanAuthority {
                finalization_attempt_id: attempt_id,
                effect_observation: observation.clone(),
                completion_mode:
                    PreArmCancellationFinalizationCompletionMode::FinishCancellationAndRelease,
                starting_root_guard_state: PreArmCancellationStartingGuardState::Released,
                starting_mode_lease_state: PreArmCancellationStartingGuardState::Released,
                acquire_root_guard: true,
                acquire_mode_lease: true,
                receipt_plan,
                expected_postcondition_digests:
                    PreArmCancellationExpectedPostconditionDigests::from_pairs_test_only(
                        expected_postconditions,
                    )
                    .unwrap(),
                recheck_policy,
                execution_path_plan,
                prior_attempt_lineage: PreArmCancellationAttemptAuditLineageAuthority::initial(),
                finalization_history_partition: history.clone(),
                selective_update_plan,
                expected_final_original_fingerprint: if include_selective_update {
                    digest(B)
                } else {
                    digest(C)
                },
                expected_final_support_graph_digest: digest(A),
                planned_result_phase: TaskPhase::Synchronized,
            },
        )
        .unwrap();
        let evidence = PreArmCancellationFinalizationRecheckEvidence::new(
            PreArmCancellationFinalizationRecheckEvidenceAuthority::matched_test_only(
                history,
                digest(C),
                digest(A),
            ),
        )
        .unwrap();
        (observation, plan, evidence)
    }

    fn replan_lineage(
        previous_plan: &PreArmCancellationFinalizationPlan,
        refreshed_history_partition: ValidatedRepositoryHistoryPartition,
        mismatch_kinds: Vec<PreArmCancellationFinalizationReplanMismatchKind>,
    ) -> PreArmCancellationAttemptAuditLineageAuthority {
        let evidence = PreArmCancellationFinalizationRecheckEvidence::new(
            PreArmCancellationFinalizationRecheckEvidenceAuthority::replan_required_test_only(
                mismatch_kinds,
                refreshed_history_partition,
                digest(C),
                digest(A),
            )
            .unwrap(),
        )
        .unwrap();
        let cause = PreArmCancellationFinalizationCompensatedCause::recheck_replan_required(
            vec![
                receipt(ID_1, PreArmCancellationEffectKind::RootGuardAcquire, A),
                receipt(ID_2, PreArmCancellationEffectKind::ModeLeaseAcquire, B),
            ],
            vec![
                receipt(ID_5, PreArmCancellationEffectKind::ModeLeaseRelease, A),
                receipt(ID_6, PreArmCancellationEffectKind::RootGuardRelease, B),
            ],
            evidence,
        )
        .unwrap();
        let progress = PreArmCancellationFinalizationAttemptProgress::compensated_test_only(
            previous_plan.finalization_attempt_id().clone(),
            cause,
        )
        .unwrap();
        let audit = PreArmCancellationFinalizationAttemptAudit::new_test_only(
            previous_plan.finalization_plan_digest().clone(),
            progress,
        )
        .unwrap();
        PreArmCancellationAttemptAuditLineageAuthority::append_compensated_attempt(
            previous_plan,
            audit,
        )
        .unwrap()
    }

    #[test]
    fn observation_is_no_arming_and_receipt_plan_allocates_only_missing_effects() {
        let plan = ApprovedSelectiveUpdatePlanProjection::test_only(
            empty_targets(),
            capability("selective-recovery-v1"),
            None,
        )
        .unwrap();
        let observation = no_guard_observation(plan.plan_digest.clone());
        let observation_json = serde_json::to_value(&observation).unwrap();
        assert_eq!(observation_json["armingReceiptAbsent"], json!(true));
        assert!(observation_json.get("armingReceipt").is_none());
        assert!(observation_json.get("disposition").is_none());

        let receipt_plan = PreArmCancellationReceiptPlan::new(
            &plan,
            &observation,
            true,
            true,
            PreArmCancellationReceiptPlanAuthority {
                root_guard_acquisition_receipt: future_ref(
                    ID_1,
                    PreArmCancellationEffectKind::RootGuardAcquire,
                    A,
                ),
                mode_lease_acquisition_receipt: future_ref(
                    ID_2,
                    PreArmCancellationEffectKind::ModeLeaseAcquire,
                    B,
                ),
                selective_update_effect_receipt: None,
                cancellation_persistence_receipt: future_ref(
                    ID_3,
                    PreArmCancellationEffectKind::AuthorizationCancellation,
                    C,
                ),
                mode_lease_release_receipt: future_ref(
                    ID_4,
                    PreArmCancellationEffectKind::ModeLeaseRelease,
                    A,
                ),
                root_guard_release_receipt: future_ref(
                    ID_5,
                    PreArmCancellationEffectKind::RootGuardRelease,
                    B,
                ),
                recovery_finalization_receipt: future_ref(
                    ID_6,
                    PreArmCancellationEffectKind::RecoveryFinalization,
                    C,
                ),
            },
        )
        .unwrap();
        let encoded = serde_json::to_value(receipt_plan).unwrap();
        assert_eq!(encoded["selectiveUpdateDisposition"], json!("notRequired"));
        assert!(encoded.get("selectiveUpdateEffectReceipt").is_none());
        assert!(encoded.get("boundSelectiveUpdatePlanDigest").is_none());
    }

    #[test]
    fn effect_intents_reject_per_effect_postcondition_substitution() {
        let plan = ApprovedSelectiveUpdatePlanProjection::test_only(
            empty_targets(),
            capability("selective-recovery-v1"),
            None,
        )
        .unwrap();
        let observation = no_guard_observation(plan.plan_digest.clone());
        let attempt_id = id(ID_8);
        let slots = [
            (PreArmCancellationEffectKind::RootGuardAcquire, digest(A)),
            (PreArmCancellationEffectKind::ModeLeaseAcquire, digest(B)),
            (
                PreArmCancellationEffectKind::AuthorizationCancellation,
                digest(C),
            ),
            (PreArmCancellationEffectKind::ModeLeaseRelease, digest(A)),
            (PreArmCancellationEffectKind::RootGuardRelease, digest(B)),
            (
                PreArmCancellationEffectKind::RecoveryFinalization,
                digest(C),
            ),
        ];
        let postconditions =
            PreArmCancellationExpectedPostconditionDigests::from_pairs_test_only(slots.clone())
                .unwrap();
        let make_ref = |receipt_id, (kind, postcondition)| {
            future_ref_with_digest(
                receipt_id,
                kind,
                finalization_intent(kind, &attempt_id, &observation, postcondition),
            )
        };
        let receipt_plan = PreArmCancellationReceiptPlan::new(
            &plan,
            &observation,
            true,
            true,
            PreArmCancellationReceiptPlanAuthority {
                root_guard_acquisition_receipt: make_ref(ID_1, slots[0].clone()),
                mode_lease_acquisition_receipt: make_ref(ID_2, slots[1].clone()),
                selective_update_effect_receipt: None,
                cancellation_persistence_receipt: make_ref(ID_3, slots[2].clone()),
                mode_lease_release_receipt: make_ref(ID_4, slots[3].clone()),
                root_guard_release_receipt: make_ref(ID_5, slots[4].clone()),
                recovery_finalization_receipt: make_ref(ID_6, slots[5].clone()),
            },
        )
        .unwrap();
        assert!(validate_finalization_receipt_intents(
            &receipt_plan,
            &attempt_id,
            &observation,
            &postconditions,
        )
        .is_ok());

        let substituted = PreArmCancellationExpectedPostconditionDigests::from_pairs_test_only([
            (PreArmCancellationEffectKind::RootGuardAcquire, digest(B)),
            (PreArmCancellationEffectKind::ModeLeaseAcquire, digest(A)),
            slots[2].clone(),
            slots[3].clone(),
            slots[4].clone(),
            slots[5].clone(),
        ])
        .unwrap();
        assert!(validate_finalization_receipt_intents(
            &receipt_plan,
            &attempt_id,
            &observation,
            &substituted,
        )
        .is_err());
    }

    #[test]
    fn policy_and_recheck_evidence_bind_full_partitions_and_named_digests() {
        let partition = empty_partition();
        let policy = PreArmCancellationFinalizationRecheckPolicy::new(
            PreArmCancellationFinalizationRecheckPolicyAuthority {
                kind: PreArmCancellationFinalizationRecheckPolicyAuthorityKind::ReplannableBeforeUpdate {
                    source_progress_stage: PreArmCancellationEffectProgressStage::NoGuard,
                    continuously_held_root: false,
                    continuously_held_mode_lease: false,
                    history_partition: partition.clone(),
                    expected_original_fingerprint: digest(A),
                    expected_support_graph_digest: digest(B),
                    pre_arm_freeze_digest: digest(C),
                },
            },
        )
        .unwrap();
        assert_eq!(
            policy.mode(),
            PreArmCancellationFinalizationRecheckMode::ReplannableBeforeUpdate
        );

        assert!(PreArmCancellationFinalizationRecheckPolicy::new(
            PreArmCancellationFinalizationRecheckPolicyAuthority {
                kind: PreArmCancellationFinalizationRecheckPolicyAuthorityKind::ReplannableBeforeUpdate {
                    source_progress_stage: PreArmCancellationEffectProgressStage::UpdateReadyGuardsHeld,
                    continuously_held_root: true,
                    continuously_held_mode_lease: true,
                    history_partition: partition.clone(),
                    expected_original_fingerprint: digest(A),
                    expected_support_graph_digest: digest(B),
                    pre_arm_freeze_digest: digest(C),
                },
            },
        )
        .is_err());

        let evidence = PreArmCancellationFinalizationRecheckEvidence::new(
            PreArmCancellationFinalizationRecheckEvidenceAuthority::matched_test_only(
                partition,
                digest(A),
                digest(B),
            ),
        )
        .unwrap();
        assert_eq!(
            evidence.outcome(),
            PreArmCancellationFinalizationRecheckOutcome::Matched
        );
        assert!(serde_json::to_value(evidence)
            .unwrap()
            .get("evidenceDigest")
            .is_some());

        assert!(CanonicalReplanMismatchKinds::new(vec![
            PreArmCancellationFinalizationReplanMismatchKind::OriginalTargetChanged,
            PreArmCancellationFinalizationReplanMismatchKind::NonRootRoutineTailAdvanced,
        ])
        .is_err());

        let base = routine_partition("16", A, &[("17", B, false)]);
        let appended = routine_partition("17", B, &[("18", C, true)]);
        let exact_combined = routine_partition("16", A, &[("17", B, false), ("18", C, true)]);
        assert!(PreArmCancellationFinalizationRecheckEvidence::new(
            PreArmCancellationFinalizationRecheckEvidenceAuthority {
                kind:
                    PreArmCancellationFinalizationRecheckEvidenceAuthorityKind::SafeTailExtended {
                        base_history_partition: base.clone(),
                        appended_non_root_history_partition: appended.clone(),
                        combined_history_partition: exact_combined,
                        observed_original_fingerprint: digest(A),
                        observed_support_graph_digest: digest(B),
                    },
            },
        )
        .is_ok());
        let substituted_combined = routine_partition("16", A, &[("17", B, true), ("18", C, true)]);
        assert!(PreArmCancellationFinalizationRecheckEvidence::new(
            PreArmCancellationFinalizationRecheckEvidenceAuthority {
                kind:
                    PreArmCancellationFinalizationRecheckEvidenceAuthorityKind::SafeTailExtended {
                        base_history_partition: base,
                        appended_non_root_history_partition: appended,
                        combined_history_partition: substituted_combined,
                        observed_original_fingerprint: digest(A),
                        observed_support_graph_digest: digest(B),
                    },
            },
        )
        .is_err());

        let base = routine_partition("16", A, &[("17", B, false)]);
        let appended = routine_partition("17", B, &[("18", C, true)]);
        let cursor_spliced_combined =
            routine_partition("16", A, &[("17", A, false), ("18", C, true)]);
        assert_eq!(
            serialized_partition_entries(&base).unwrap(),
            serialized_partition_entries(&cursor_spliced_combined).unwrap()[..1]
        );
        assert!(!cursor_spliced_combined.contains_cursor(base.through_inclusive()));
        assert!(PreArmCancellationFinalizationRecheckEvidence::new(
            PreArmCancellationFinalizationRecheckEvidenceAuthority {
                kind:
                    PreArmCancellationFinalizationRecheckEvidenceAuthorityKind::SafeTailExtended {
                        base_history_partition: base,
                        appended_non_root_history_partition: appended,
                        combined_history_partition: cursor_spliced_combined,
                        observed_original_fingerprint: digest(A),
                        observed_support_graph_digest: digest(B),
                    },
            },
        )
        .is_err());
    }

    #[test]
    fn execution_paths_and_attempt_audits_are_canonical_and_append_only() {
        assert!(PreArmCancellationFinalizationExecutionPath::new(
            PreArmCancellationFinalizationExecutionPathKind::Success,
            vec![id(ID_1), id(ID_1)],
        )
        .is_err());
        let success = PreArmCancellationFinalizationExecutionPath::new(
            PreArmCancellationFinalizationExecutionPathKind::Success,
            vec![id(ID_1)],
        )
        .unwrap();
        let breach = PreArmCancellationFinalizationExecutionPath::new(
            PreArmCancellationFinalizationExecutionPathKind::CapabilityBreachStop,
            vec![id(ID_2)],
        )
        .unwrap();
        assert!(PreArmCancellationFinalizationExecutionPathPlan::new(vec![
            breach.clone(),
            success.clone(),
        ])
        .is_err());
        assert!(PreArmCancellationFinalizationExecutionPathPlan::new(vec![
            success.clone(),
            success,
        ])
        .is_err());
        assert!(
            PreArmCancellationFinalizationExecutionPathPlan::new(vec![successless(breach.clone())])
                .is_err()
        );
        let plan = PreArmCancellationFinalizationExecutionPathPlan::new(vec![
            PreArmCancellationFinalizationExecutionPath::new(
                PreArmCancellationFinalizationExecutionPathKind::Success,
                vec![id(ID_1)],
            )
            .unwrap(),
            breach,
        ])
        .unwrap();
        assert_eq!(plan.paths().len(), 2);

        let root_conflict =
            PreArmCancellationFinalizationCompensatedCause::root_guard_conflict().unwrap();
        let compensated = PreArmCancellationFinalizationAttemptProgress::compensated_test_only(
            id(ID_3),
            root_conflict,
        )
        .unwrap();
        let audit =
            PreArmCancellationFinalizationAttemptAudit::new_test_only(digest(A), compensated)
                .unwrap();
        assert_eq!(audit.finalization_attempt_id(), &id(ID_3));
        assert!(PriorAttemptAudits::new(vec![audit.clone(), audit]).is_err());
        assert!(PreArmCancellationFinalizationAttemptAudit::new_test_only(
            digest(B),
            PreArmCancellationFinalizationAttemptProgress::not_started_test_only(id(ID_4)),
        )
        .is_err());

        let replan_evidence = PreArmCancellationFinalizationRecheckEvidence::new(
            PreArmCancellationFinalizationRecheckEvidenceAuthority::replan_required_test_only(
                vec![PreArmCancellationFinalizationReplanMismatchKind::OriginalTargetChanged],
                routine_partition("16", A, &[("17", B, false)]),
                digest(C),
                digest(A),
            )
            .unwrap(),
        )
        .unwrap();
        for forbidden_forward in [
            vec![
                receipt(ID_1, PreArmCancellationEffectKind::RootGuardAcquire, A),
                receipt(
                    ID_2,
                    PreArmCancellationEffectKind::SelectiveOriginalUpdate,
                    B,
                ),
            ],
            vec![
                receipt(ID_1, PreArmCancellationEffectKind::RootGuardAcquire, A),
                receipt(ID_2, PreArmCancellationEffectKind::ModeLeaseAcquire, B),
                receipt(
                    ID_3,
                    PreArmCancellationEffectKind::AuthorizationCancellation,
                    C,
                ),
            ],
        ] {
            assert!(
                PreArmCancellationFinalizationCompensatedCause::recheck_replan_required(
                    forbidden_forward,
                    vec![
                        receipt(ID_5, PreArmCancellationEffectKind::ModeLeaseRelease, A),
                        receipt(ID_6, PreArmCancellationEffectKind::RootGuardRelease, B),
                    ],
                    replan_evidence.clone(),
                )
                .is_err()
            );
        }
    }

    #[test]
    fn execution_path_and_attempt_audit_schemas_enforce_constructor_invariants() {
        let path_plan = PreArmCancellationFinalizationExecutionPathPlan::new(vec![
            PreArmCancellationFinalizationExecutionPath::new(
                PreArmCancellationFinalizationExecutionPathKind::Success,
                vec![id(ID_1)],
            )
            .unwrap(),
            PreArmCancellationFinalizationExecutionPath::new(
                PreArmCancellationFinalizationExecutionPathKind::CapabilityBreachStop,
                vec![id(ID_2)],
            )
            .unwrap(),
        ])
        .unwrap();
        let path_plan_schema = schema::<PreArmCancellationFinalizationExecutionPathPlan>();
        let path_plan_validator = jsonschema::validator_for(&path_plan_schema).unwrap();
        let valid_path_plan = serde_json::to_value(&path_plan).unwrap();
        assert!(path_plan_validator.is_valid(&valid_path_plan));

        let mut empty_paths = valid_path_plan.clone();
        empty_paths["paths"] = json!([]);
        assert!(!path_plan_validator.is_valid(&empty_paths));

        let mut too_many_paths = valid_path_plan.clone();
        let repeated = valid_path_plan["paths"][0].clone();
        too_many_paths["paths"] = Value::Array(vec![repeated; 7]);
        assert!(!path_plan_validator.is_valid(&too_many_paths));

        let mut duplicate_paths = valid_path_plan;
        duplicate_paths["paths"][1] = duplicate_paths["paths"][0].clone();
        assert!(!path_plan_validator.is_valid(&duplicate_paths));

        let compensated = PreArmCancellationFinalizationAttemptProgress::compensated_test_only(
            id(ID_3),
            PreArmCancellationFinalizationCompensatedCause::root_guard_conflict().unwrap(),
        )
        .unwrap();
        let audit =
            PreArmCancellationFinalizationAttemptAudit::new_test_only(digest(A), compensated)
                .unwrap();
        let audit_schema = schema::<PreArmCancellationFinalizationAttemptAudit>();
        let audit_validator = jsonschema::validator_for(&audit_schema).unwrap();
        let mut invalid_audit = serde_json::to_value(audit).unwrap();
        invalid_audit["compensatedProgress"] = serde_json::to_value(
            PreArmCancellationFinalizationAttemptProgress::not_started_test_only(id(ID_3)),
        )
        .unwrap();
        assert!(!audit_validator.is_valid(&invalid_audit));
    }

    #[test]
    fn finalization_plan_binds_phases_paths_receipts_and_fresh_attempt_id() {
        let selective_update_plan = empty_recovery_update_plan();
        let history = empty_partition();
        let observation = no_guard_observation_with_history(
            selective_update_plan.plan_digest().clone(),
            history.clone(),
        );
        let authority = finalization_authority(
            id(ID_8),
            observation.clone(),
            selective_update_plan.clone(),
            history.clone(),
            PreArmCancellationAttemptAuditLineageAuthority::initial(),
            TaskPhase::Synchronized,
        );
        let plan = PreArmCancellationFinalizationPlan::new_test_only(authority.clone()).unwrap();
        assert!(plan.binds_effect_observation(&observation));
        let mut wrong_binding = plan.clone();
        wrong_binding.prior_operation_id = operation_id(ID_3);
        assert!(!wrong_binding.binds_effect_observation(&observation));
        let mut wrong_binding = plan.clone();
        wrong_binding.support_action_id = id(ID_3);
        assert!(!wrong_binding.binds_effect_observation(&observation));
        let mut wrong_binding = plan.clone();
        wrong_binding.expected_support_action_digest = digest(B);
        assert!(!wrong_binding.binds_effect_observation(&observation));
        let mut wrong_binding = plan.clone();
        wrong_binding.approved_cancellation_digest = digest(C);
        assert!(!wrong_binding.binds_effect_observation(&observation));
        let mut wrong_binding = plan.clone();
        wrong_binding.manual_target_mode = ManualSupportTargetMode::SeparateWorkingInfobase;
        assert!(!wrong_binding.binds_effect_observation(&observation));
        let mut wrong_binding = plan.clone();
        wrong_binding.effect_observation_digest = digest(C);
        assert!(!wrong_binding.binds_effect_observation(&observation));
        let mut wrong_binding = plan.clone();
        wrong_binding.bound_cancelled_phase = TaskPhase::Developing;
        assert!(!wrong_binding.binds_effect_observation(&observation));
        let mut wrong_binding = plan.clone();
        wrong_binding.relevant_advance_phase = TaskPhase::Developing;
        assert!(!wrong_binding.binds_effect_observation(&observation));
        let observation_wire = serde_json::to_value(&observation).unwrap();
        assert!(observation_wire.get("cancelledPhase").is_none());
        assert!(observation_wire.get("relevantAdvancePhase").is_none());

        let mut wrong_paths = authority.clone();
        wrong_paths.execution_path_plan =
            PreArmCancellationFinalizationExecutionPathPlan::new(vec![
                PreArmCancellationFinalizationExecutionPath::new(
                    PreArmCancellationFinalizationExecutionPathKind::Success,
                    [ID_1, ID_2, ID_3, ID_4, ID_5, ID_6, ID_7]
                        .into_iter()
                        .map(id)
                        .collect(),
                )
                .unwrap(),
                PreArmCancellationFinalizationExecutionPath::new(
                    PreArmCancellationFinalizationExecutionPathKind::CapabilityBreachStop,
                    [ID_1, ID_2, ID_3].into_iter().map(id).collect(),
                )
                .unwrap(),
                PreArmCancellationFinalizationExecutionPath::new(
                    PreArmCancellationFinalizationExecutionPathKind::RootGuardConflictCompensation,
                    vec![id(ID_1)],
                )
                .unwrap(),
                PreArmCancellationFinalizationExecutionPath::new(
                    PreArmCancellationFinalizationExecutionPathKind::ModeLeaseUnavailableBeforeAcquisitionCompensation,
                    [ID_1, ID_2].into_iter().map(id).collect(),
                )
                .unwrap(),
                PreArmCancellationFinalizationExecutionPath::new(
                    PreArmCancellationFinalizationExecutionPathKind::RecheckReplanCompensation,
                    [ID_1, ID_2, ID_3, ID_5, ID_6]
                        .into_iter()
                        .map(id)
                        .collect(),
                )
                .unwrap(),
            ])
            .unwrap();
        assert!(PreArmCancellationFinalizationPlan::new_test_only(wrong_paths).is_err());

        let mut wrong_receipt = authority.clone();
        wrong_receipt.receipt_plan.root_guard_release_receipt =
            future_ref(ID_6, PreArmCancellationEffectKind::RootGuardRelease, A);
        assert!(PreArmCancellationFinalizationPlan::new_test_only(wrong_receipt).is_err());

        let mut outside_pair = authority.clone();
        outside_pair.planned_result_phase = TaskPhase::Developing;
        assert!(PreArmCancellationFinalizationPlan::new_test_only(outside_pair).is_err());

        let relevant_history = routine_partition("16", A, &[("17", B, true)]);
        let relevant_observation = no_guard_observation_with_history(
            selective_update_plan.plan_digest().clone(),
            relevant_history.clone(),
        );
        let mut relevant_with_cancelled = finalization_authority(
            id(ID_9),
            relevant_observation.clone(),
            selective_update_plan.clone(),
            relevant_history.clone(),
            PreArmCancellationAttemptAuditLineageAuthority::initial(),
            TaskPhase::Synchronized,
        );
        assert!(
            PreArmCancellationFinalizationPlan::new_test_only(relevant_with_cancelled.clone())
                .is_err()
        );
        relevant_with_cancelled.planned_result_phase = TaskPhase::LocalVerified;
        assert!(PreArmCancellationFinalizationPlan::new_test_only(relevant_with_cancelled).is_ok());

        let root_conflict =
            PreArmCancellationFinalizationCompensatedCause::root_guard_conflict().unwrap();
        let compensated = PreArmCancellationFinalizationAttemptProgress::compensated_test_only(
            plan.finalization_attempt_id().clone(),
            root_conflict,
        )
        .unwrap();
        let audit = PreArmCancellationFinalizationAttemptAudit::new_test_only(
            plan.finalization_plan_digest().clone(),
            compensated,
        )
        .unwrap();
        let lineage = PreArmCancellationAttemptAuditLineageAuthority::append_compensated_attempt(
            &plan, audit,
        )
        .unwrap();

        let embedded_plan = ApprovedSelectiveUpdatePlanProjection::test_only(
            root_target(),
            capability("selective-recovery-v1"),
            Some(capability("structural-recovery-v1")),
        )
        .unwrap();
        let mut held_source_with_lineage = finalization_authority(
            id(ID_9),
            observation.clone(),
            selective_update_plan.clone(),
            history.clone(),
            lineage.clone(),
            TaskPhase::Synchronized,
        );
        held_source_with_lineage.effect_observation =
            update_ready_observation(&embedded_plan, embedded_plan.plan_digest.clone(), false)
                .unwrap();
        assert_eq!(
            PreArmCancellationFinalizationPlan::new_test_only(held_source_with_lineage)
                .unwrap_err()
                .to_string(),
            "update-ready source cannot carry prior compensated attempt audits"
        );

        let reused_attempt = finalization_authority(
            plan.finalization_attempt_id().clone(),
            observation,
            selective_update_plan,
            history,
            lineage,
            TaskPhase::Synchronized,
        );
        assert!(PreArmCancellationFinalizationPlan::new_test_only(reused_attempt).is_err());
    }

    #[test]
    fn external_initial_history_requires_relevant_advance_phase() {
        let selective_update_plan = empty_recovery_update_plan();
        for classification in [
            RepositoryHistoryPartitionClassification::ExternalSupport,
            RepositoryHistoryPartitionClassification::PreArmExternal,
        ] {
            let history = support_partition(classification);
            let observation = no_guard_observation_with_history(
                selective_update_plan.plan_digest().clone(),
                history.clone(),
            );
            assert_eq!(observation.support_action_id(), &id(ID_1));
            let mut authority = finalization_authority(
                id(ID_8),
                observation,
                selective_update_plan.clone(),
                history,
                PreArmCancellationAttemptAuditLineageAuthority::initial(),
                TaskPhase::Synchronized,
            );
            assert!(PreArmCancellationFinalizationPlan::new_test_only(authority.clone()).is_err());
            authority.planned_result_phase = TaskPhase::LocalVerified;
            assert!(PreArmCancellationFinalizationPlan::new_test_only(authority).is_ok());
        }
    }

    #[test]
    fn archive_lineage_projection_rejects_receipt_proof_history_and_phase_splices() {
        let fixture = matched_terminal_fixture();
        assert!(fixture.validate(TaskPhase::Synchronized).is_ok());
        assert!(fixture.validate(TaskPhase::LocalVerified).is_err());

        let wrong_id = id(ID_9);
        let mut projection = fixture.projection(TaskPhase::Synchronized);
        projection.support_cancellation_receipt_id = &wrong_id;
        assert!(fixture
            .plan
            .validate_archive_lineage_projection(projection)
            .is_err());
        let wrong_digest = digest(B);
        let mut projection = fixture.projection(TaskPhase::Synchronized);
        projection.pre_arm_recovery_receipt_digest = &wrong_digest;
        assert!(fixture
            .plan
            .validate_archive_lineage_projection(projection)
            .is_err());

        let mut wrong_progress = fixture.clone();
        let mut receipts = realized_finalization_receipts(&wrong_progress.plan);
        let root_ref = wrong_progress
            .plan
            .receipt_plan
            .root_guard_acquisition_receipt();
        receipts[0] = PreArmCancellationEffectReceipt::new(
            PreArmCancellationEffectReceiptAuthority::test_only(
                id(ID_9),
                PreArmCancellationEffectKind::RootGuardAcquire,
                root_ref.effect_intent_digest().clone(),
                id(ID_9),
                digest(B),
                vec![digest(A)],
            )
            .unwrap(),
        )
        .unwrap();
        wrong_progress.progress =
            PreArmCancellationFinalizationAttemptProgress::completed_test_only(
                wrong_progress.plan.finalization_attempt_id().clone(),
                receipts,
                wrong_progress.evidence.clone(),
            )
            .unwrap();
        assert!(wrong_progress.validate(TaskPhase::Synchronized).is_err());

        let mut wrong_evidence = fixture.clone();
        wrong_evidence.evidence = PreArmCancellationFinalizationRecheckEvidence::new(
            PreArmCancellationFinalizationRecheckEvidenceAuthority::matched_test_only(
                wrong_evidence.post_apply_partition.clone(),
                digest(B),
                digest(A),
            ),
        )
        .unwrap();
        wrong_evidence.progress =
            PreArmCancellationFinalizationAttemptProgress::completed_test_only(
                wrong_evidence.plan.finalization_attempt_id().clone(),
                realized_finalization_receipts(&wrong_evidence.plan),
                wrong_evidence.evidence.clone(),
            )
            .unwrap();
        assert!(wrong_evidence.validate(TaskPhase::Synchronized).is_err());

        let mut wrong_proof = fixture.clone();
        let other_plan = SelectiveRepositoryUpdatePlan::recovery_finalization_test_only(
            empty_targets(),
            wrong_proof
                .plan
                .selective_update_plan
                .lock_targets()
                .clone(),
            capability("selective-recovery-v2"),
            None,
        )
        .unwrap();
        wrong_proof.selective_update_proof =
            SelectiveRepositoryUpdateProof::recovery_finalization_already_exact_test_only(
                &other_plan,
                wrong_proof
                    .plan
                    .receipt_plan
                    .root_guard_acquisition_receipt()
                    .receipt_id()
                    .clone(),
                digest(A),
                digest(C),
                wrong_proof.post_apply_partition.start_cursor().clone(),
                wrong_proof.post_apply_partition.start_cursor().clone(),
            )
            .unwrap();
        assert!(wrong_proof.validate(TaskPhase::Synchronized).is_err());

        let mut outside_after_cursor = fixture.clone();
        outside_after_cursor.selective_update_proof =
            SelectiveRepositoryUpdateProof::recovery_finalization_already_exact_test_only(
                &outside_after_cursor.plan.selective_update_plan,
                outside_after_cursor
                    .plan
                    .receipt_plan
                    .root_guard_acquisition_receipt()
                    .receipt_id()
                    .clone(),
                digest(A),
                digest(C),
                outside_after_cursor
                    .post_apply_partition
                    .start_cursor()
                    .clone(),
                cursor("17", B),
            )
            .unwrap();
        assert!(outside_after_cursor
            .validate(TaskPhase::Synchronized)
            .is_err());

        let mut relevant_tail = fixture;
        relevant_tail.post_apply_partition = routine_partition("16", A, &[("17", B, true)]);
        relevant_tail.post_release_cursor = relevant_tail
            .post_apply_partition
            .through_inclusive()
            .clone();
        assert!(relevant_tail.validate(TaskPhase::LocalVerified).is_ok());
        assert!(relevant_tail.validate(TaskPhase::Synchronized).is_err());
    }

    #[test]
    fn replan_phase_preserves_unrelated_and_switches_for_relevant_suffix() {
        let selective_update_plan = empty_recovery_update_plan();
        let initial_history = empty_partition();
        let observation = no_guard_observation_with_history(
            selective_update_plan.plan_digest().clone(),
            initial_history.clone(),
        );
        let previous = PreArmCancellationFinalizationPlan::new_test_only(finalization_authority(
            id(ID_8),
            observation.clone(),
            selective_update_plan.clone(),
            initial_history,
            PreArmCancellationAttemptAuditLineageAuthority::initial(),
            TaskPhase::Synchronized,
        ))
        .unwrap();

        let unrelated = routine_partition("16", A, &[("17", B, false)]);
        let unrelated_lineage = replan_lineage(
            &previous,
            unrelated.clone(),
            vec![PreArmCancellationFinalizationReplanMismatchKind::NonRootRoutineTailAdvanced],
        );
        assert!(
            PreArmCancellationFinalizationPlan::new_test_only(finalization_authority(
                id(ID_9),
                observation.clone(),
                selective_update_plan.clone(),
                unrelated.clone(),
                unrelated_lineage.clone(),
                TaskPhase::Synchronized,
            ))
            .is_ok()
        );
        assert!(
            PreArmCancellationFinalizationPlan::new_test_only(finalization_authority(
                id(ID_9),
                observation.clone(),
                selective_update_plan.clone(),
                unrelated,
                unrelated_lineage,
                TaskPhase::LocalVerified,
            ))
            .is_err()
        );

        let relevant = routine_partition("16", A, &[("17", B, true)]);
        let relevant_lineage = replan_lineage(
            &previous,
            relevant.clone(),
            vec![PreArmCancellationFinalizationReplanMismatchKind::NonRootRoutineTailAdvanced],
        );
        assert!(
            PreArmCancellationFinalizationPlan::new_test_only(finalization_authority(
                id(ID_9),
                observation.clone(),
                selective_update_plan.clone(),
                relevant.clone(),
                relevant_lineage.clone(),
                TaskPhase::LocalVerified,
            ))
            .is_ok()
        );
        assert!(
            PreArmCancellationFinalizationPlan::new_test_only(finalization_authority(
                id(ID_9),
                observation.clone(),
                selective_update_plan.clone(),
                relevant.clone(),
                relevant_lineage,
                TaskPhase::Synchronized,
            ))
            .is_err()
        );

        let mutable_lineage = replan_lineage(
            &previous,
            relevant.clone(),
            vec![PreArmCancellationFinalizationReplanMismatchKind::RootOrSupportVersionChanged],
        );
        assert!(
            PreArmCancellationFinalizationPlan::new_test_only(finalization_authority(
                id(ID_9),
                observation,
                selective_update_plan,
                relevant,
                mutable_lineage,
                TaskPhase::LocalVerified,
            ))
            .is_ok()
        );
    }

    #[test]
    fn replan_rejects_same_entries_with_a_spliced_intermediate_prefix_digest() {
        let selective_update_plan = empty_recovery_update_plan();
        let base_history = routine_partition("16", A, &[("17", B, false)]);
        let observation = no_guard_observation_with_history(
            selective_update_plan.plan_digest().clone(),
            base_history.clone(),
        );
        let previous = PreArmCancellationFinalizationPlan::new_test_only(finalization_authority(
            id(ID_8),
            observation.clone(),
            selective_update_plan.clone(),
            base_history.clone(),
            PreArmCancellationAttemptAuditLineageAuthority::initial(),
            TaskPhase::Synchronized,
        ))
        .unwrap();
        let spliced_history = routine_partition("16", A, &[("17", A, false), ("18", C, false)]);
        assert_eq!(
            serialized_partition_entries(&base_history).unwrap(),
            serialized_partition_entries(&spliced_history).unwrap()[..1]
        );
        assert!(!spliced_history.contains_cursor(base_history.through_inclusive()));
        let lineage = replan_lineage(
            &previous,
            spliced_history.clone(),
            vec![PreArmCancellationFinalizationReplanMismatchKind::NonRootRoutineTailAdvanced],
        );
        assert!(
            PreArmCancellationFinalizationPlan::new_test_only(finalization_authority(
                id(ID_9),
                observation,
                selective_update_plan,
                spliced_history,
                lineage,
                TaskPhase::Synchronized,
            ))
            .is_err()
        );
    }

    fn successless(
        path: PreArmCancellationFinalizationExecutionPath,
    ) -> PreArmCancellationFinalizationExecutionPath {
        path
    }
}
