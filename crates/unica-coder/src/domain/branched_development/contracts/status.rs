use super::artifacts::{ArtifactKind, ArtifactRole, OwnedTargetLocator};
use super::change_receipts::{BranchedAffectedTarget, ChangeReceiptSequence};
use super::errors::{ConflictResolution, StableErrorCode};
use super::prearm_recovery::{
    PreArmCancellationEffectObservation, PreArmCancellationFinalizationAttemptProgress,
    PreArmCancellationFinalizationAttemptState, PreArmCancellationFinalizationPlan,
    PreArmCancellationFinalizationRecheckEvidence,
};
use super::recovery::{
    ArchivedCleanupRecoveryPlanStatusSchema, FinishCleanupAbsenceObservation,
    FinishCleanupAbsenceObservations, PreWorkspaceRecoveryPlanStatusSchema, RecoveryPlanStatus,
    RecoveryTarget, WorkspaceRecoveryPlanStatusSchema,
};
use super::repository::{
    DeferredRepositoryAdvance, DeferredRepositoryAdvanceConsumptionReceipt,
    RepositoryActorIdentity, RepositoryHistoryCursor, RepositoryOwnerIdentity,
    RepositoryTargetIdentity, SupportGateHistoryEvidence,
};
use super::results::task::ArchiveStatusProjectionAuthority;
use super::scalars::{NormalizedUtcInstant, PositiveGeneration, Reason, RepositoryVersion};
use super::schema::one_of_schema;
use super::selectors::TaskOperationSelector;
use super::support::{
    ActiveSupportActionResumeHandle, SupportActionPurpose, SupportPreflightOutcome,
};
use crate::domain::branched_development::canonical_json::{
    canonical_contract_digest, contract_digest_record_sealed, ContractDigestRecord,
};
use crate::domain::branched_development::{
    DurableExecutionPolicy, OperationId, Sha256Digest, TaskPhase, UnicaId,
};
use schemars::{json_schema, JsonSchema, Schema, SchemaGenerator};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::fmt;
use time::{format_description::well_known::Rfc3339, OffsetDateTime};

const MAX_STATUS_ITEMS: usize = 1_024;
const MAX_I_JSON_INTEGER: u64 = 9_007_199_254_740_991;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct StatusContractError(&'static str);

impl fmt::Display for StatusContractError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.0)
    }
}

impl std::error::Error for StatusContractError {}

fn status_digest<T: ContractDigestRecord>(
    value: &T,
    context: &'static str,
) -> Result<Sha256Digest, StatusContractError> {
    canonical_contract_digest(value, None).map_err(|_| StatusContractError(context))
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
                S: serde::Serializer,
            {
                serializer.serialize_bool($value)
            }
        }

        impl JsonSchema for $name {
            fn schema_name() -> Cow<'static, str> {
                stringify!($name).into()
            }

            fn json_schema(_: &mut SchemaGenerator) -> Schema {
                json_schema!({"type": "boolean", "const": $value})
            }
        }
    };
}

wire_literal!(RegisteredOperationState, "registered");
wire_literal!(IntentWrittenOperationState, "intentWritten");
wire_literal!(EffectUnknownOperationState, "effectUnknown");
wire_literal!(OrphanedOwnerState, "orphaned");

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct OperationHeartbeatDigestRecord {
    owner_instance_id: UnicaId,
    generation: PositiveGeneration,
    heartbeat_at: NormalizedUtcInstant,
    expires_at: NormalizedUtcInstant,
}

impl contract_digest_record_sealed::Sealed for OperationHeartbeatDigestRecord {}
impl ContractDigestRecord for OperationHeartbeatDigestRecord {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct OperationLeaseDigestRecord {
    owner_instance_id: UnicaId,
    generation: PositiveGeneration,
    acquired_at: NormalizedUtcInstant,
    heartbeat_at: NormalizedUtcInstant,
    expires_at: NormalizedUtcInstant,
    heartbeat_digest: Sha256Digest,
}

impl contract_digest_record_sealed::Sealed for OperationLeaseDigestRecord {}
impl ContractDigestRecord for OperationLeaseDigestRecord {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OperationLeaseAuthority {
    owner_instance_id: UnicaId,
    generation: PositiveGeneration,
    acquired_at: NormalizedUtcInstant,
    heartbeat_at: NormalizedUtcInstant,
    expires_at: NormalizedUtcInstant,
}

impl OperationLeaseAuthority {
    pub(crate) fn new(
        owner_instance_id: UnicaId,
        generation: PositiveGeneration,
        acquired_at: NormalizedUtcInstant,
        heartbeat_at: NormalizedUtcInstant,
        expires_at: NormalizedUtcInstant,
    ) -> Result<Self, StatusContractError> {
        let acquired_ordering = OffsetDateTime::parse(acquired_at.as_str(), &Rfc3339)
            .map_err(|_| StatusContractError("operation lease acquiredAt cannot be ordered"))?;
        let heartbeat_ordering = OffsetDateTime::parse(heartbeat_at.as_str(), &Rfc3339)
            .map_err(|_| StatusContractError("operation lease heartbeatAt cannot be ordered"))?;
        let expires_ordering = OffsetDateTime::parse(expires_at.as_str(), &Rfc3339)
            .map_err(|_| StatusContractError("operation lease expiresAt cannot be ordered"))?;
        if acquired_ordering > heartbeat_ordering || heartbeat_ordering >= expires_ordering {
            return Err(StatusContractError(
                "operation lease timestamps are not strictly live-ordered",
            ));
        }
        Ok(Self {
            owner_instance_id,
            generation,
            acquired_at,
            heartbeat_at,
            expires_at,
        })
    }
}

/// Digest-validated lease evidence. Deliberately not `Deserialize`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct OperationLease {
    owner_instance_id: UnicaId,
    generation: PositiveGeneration,
    acquired_at: NormalizedUtcInstant,
    heartbeat_at: NormalizedUtcInstant,
    expires_at: NormalizedUtcInstant,
    heartbeat_digest: Sha256Digest,
    lease_digest: Sha256Digest,
}

impl OperationLease {
    pub(crate) fn new(authority: OperationLeaseAuthority) -> Result<Self, StatusContractError> {
        let heartbeat_digest = status_digest(
            &OperationHeartbeatDigestRecord {
                owner_instance_id: authority.owner_instance_id.clone(),
                generation: authority.generation,
                heartbeat_at: authority.heartbeat_at.clone(),
                expires_at: authority.expires_at.clone(),
            },
            "operation heartbeat digest failed",
        )?;
        let record = OperationLeaseDigestRecord {
            owner_instance_id: authority.owner_instance_id,
            generation: authority.generation,
            acquired_at: authority.acquired_at,
            heartbeat_at: authority.heartbeat_at,
            expires_at: authority.expires_at,
            heartbeat_digest,
        };
        let lease_digest = status_digest(&record, "operation lease digest failed")?;
        Ok(Self {
            owner_instance_id: record.owner_instance_id,
            generation: record.generation,
            acquired_at: record.acquired_at,
            heartbeat_at: record.heartbeat_at,
            expires_at: record.expires_at,
            heartbeat_digest: record.heartbeat_digest,
            lease_digest,
        })
    }

    #[cfg(test)]
    fn test_only(
        owner_instance_id: UnicaId,
        generation: PositiveGeneration,
        acquired_at: NormalizedUtcInstant,
        heartbeat_at: NormalizedUtcInstant,
        expires_at: NormalizedUtcInstant,
    ) -> Result<Self, StatusContractError> {
        Self::new(OperationLeaseAuthority::new(
            owner_instance_id,
            generation,
            acquired_at,
            heartbeat_at,
            expires_at,
        )?)
    }

    #[cfg(test)]
    pub(crate) fn load_test_json(value: &serde_json::Value) -> Result<Self, StatusContractError> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct UncheckedOperationLease {
            owner_instance_id: UnicaId,
            generation: PositiveGeneration,
            acquired_at: NormalizedUtcInstant,
            heartbeat_at: NormalizedUtcInstant,
            expires_at: NormalizedUtcInstant,
            heartbeat_digest: Sha256Digest,
            lease_digest: Sha256Digest,
        }

        let unchecked = serde_json::from_value::<UncheckedOperationLease>(value.clone())
            .map_err(|_| StatusContractError("operation lease JSON shape is invalid"))?;
        let validated = Self::test_only(
            unchecked.owner_instance_id,
            unchecked.generation,
            unchecked.acquired_at,
            unchecked.heartbeat_at,
            unchecked.expires_at,
        )?;
        if validated.heartbeat_digest != unchecked.heartbeat_digest
            || validated.lease_digest != unchecked.lease_digest
        {
            return Err(StatusContractError(
                "operation lease heartbeat or lease digest mismatch",
            ));
        }
        Ok(validated)
    }

    #[cfg(test)]
    fn validates_json(value: &serde_json::Value) -> bool {
        Self::load_test_json(value).is_ok()
    }

    pub(crate) const fn lease_digest(&self) -> &Sha256Digest {
        &self.lease_digest
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) enum ActiveOperationOwnerState {
    Live,
    Orphaned,
}

macro_rules! leased_active_operation_leaf {
    ($record:ident, $state:ty) => {
        #[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct $record {
            operation_id: OperationId,
            operation: TaskOperationSelector,
            policy: DurableExecutionPolicy,
            state: $state,
            canonical_input_digest: Sha256Digest,
            registered_at: NormalizedUtcInstant,
            operation_lease: OperationLease,
            owner_state: ActiveOperationOwnerState,
        }
    };
}

leased_active_operation_leaf!(RegisteredActiveOperationStatus, RegisteredOperationState);
leased_active_operation_leaf!(
    IntentWrittenActiveOperationStatus,
    IntentWrittenOperationState
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct EffectUnknownActiveOperationStatus {
    operation_id: OperationId,
    operation: TaskOperationSelector,
    policy: DurableExecutionPolicy,
    state: EffectUnknownOperationState,
    canonical_input_digest: Sha256Digest,
    registered_at: NormalizedUtcInstant,
    owner_state: OrphanedOwnerState,
    recovery_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
enum ActiveOperationStatusKind {
    Registered(RegisteredActiveOperationStatus),
    IntentWritten(IntentWrittenActiveOperationStatus),
    EffectUnknown(EffectUnknownActiveOperationStatus),
}

/// Exact non-terminal projection of a durable operation. Deliberately not
/// `Deserialize` and contains no terminal or observed pseudo-state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct ActiveOperationStatus(ActiveOperationStatusKind);

impl JsonSchema for ActiveOperationStatus {
    fn schema_name() -> Cow<'static, str> {
        "ActiveOperationStatus".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<RegisteredActiveOperationStatus>(),
            generator.subschema_for::<IntentWrittenActiveOperationStatus>(),
            generator.subschema_for::<EffectUnknownActiveOperationStatus>(),
        ])
    }
}

impl ActiveOperationStatus {
    #[cfg(test)]
    fn registered_test_only(
        operation_id: OperationId,
        operation: TaskOperationSelector,
        policy: DurableExecutionPolicy,
        canonical_input_digest: Sha256Digest,
        registered_at: NormalizedUtcInstant,
        operation_lease: OperationLease,
        owner_state: ActiveOperationOwnerState,
    ) -> Result<Self, StatusContractError> {
        Ok(Self(ActiveOperationStatusKind::Registered(
            RegisteredActiveOperationStatus {
                operation_id,
                operation,
                policy,
                state: RegisteredOperationState::Value,
                canonical_input_digest,
                registered_at,
                operation_lease,
                owner_state,
            },
        )))
    }

    #[cfg(test)]
    fn effect_unknown_test_only(
        operation_id: OperationId,
        operation: TaskOperationSelector,
        policy: DurableExecutionPolicy,
        canonical_input_digest: Sha256Digest,
        registered_at: NormalizedUtcInstant,
        recovery_digest: Sha256Digest,
    ) -> Result<Self, StatusContractError> {
        Ok(Self(ActiveOperationStatusKind::EffectUnknown(
            EffectUnknownActiveOperationStatus {
                operation_id,
                operation,
                policy,
                state: EffectUnknownOperationState::Value,
                canonical_input_digest,
                registered_at,
                owner_state: OrphanedOwnerState::Value,
                recovery_digest,
            },
        )))
    }

    fn effect_unknown_recovery_binding(&self) -> Option<(&OperationId, &Sha256Digest)> {
        match &self.0 {
            ActiveOperationStatusKind::EffectUnknown(status) => {
                Some((&status.operation_id, &status.recovery_digest))
            }
            ActiveOperationStatusKind::Registered(_)
            | ActiveOperationStatusKind::IntentWritten(_) => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
struct CanonicalStatusIds(Vec<UnicaId>);

impl CanonicalStatusIds {
    fn new(values: Vec<UnicaId>) -> Result<Self, StatusContractError> {
        if values.len() > MAX_STATUS_ITEMS || values.windows(2).any(|pair| pair[0] >= pair[1]) {
            return Err(StatusContractError(
                "status IDs must be canonical and duplicate-free",
            ));
        }
        Ok(Self(values))
    }
}

impl JsonSchema for CanonicalStatusIds {
    fn schema_name() -> Cow<'static, str> {
        "CanonicalStatusIds".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "array",
            "items": generator.subschema_for::<UnicaId>(),
            "minItems": 0,
            "maxItems": MAX_STATUS_ITEMS,
            "uniqueItems": true
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
struct EmptyStatusIds(Vec<UnicaId>);

impl EmptyStatusIds {
    const fn new() -> Self {
        Self(Vec::new())
    }
}

impl JsonSchema for EmptyStatusIds {
    fn schema_name() -> Cow<'static, str> {
        "EmptyStatusIds".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "array",
            "items": generator.subschema_for::<UnicaId>(),
            "minItems": 0,
            "maxItems": 0
        })
    }
}

wire_literal!(MergeConflictDecisionKind, "mergeConflict");
wire_literal!(AdaptationDecisionKind, "adaptation");

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(transparent)]
struct StatusCount(u64);

impl StatusCount {
    fn new(value: u64) -> Result<Self, StatusContractError> {
        (value <= MAX_I_JSON_INTEGER)
            .then_some(Self(value))
            .ok_or(StatusContractError(
                "status count must be a non-negative I-JSON safe integer",
            ))
    }
}

impl JsonSchema for StatusCount {
    fn schema_name() -> Cow<'static, str> {
        "StatusCount".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "integer",
            "minimum": 0,
            "maximum": MAX_I_JSON_INTEGER
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MergeConflictPendingDecisionStatus {
    decision_kind: MergeConflictDecisionKind,
    producer_id: UnicaId,
    decision_ids: CanonicalStatusIds,
    replacement_pending_decision_ids: CanonicalStatusIds,
    remaining_count: StatusCount,
    decision_set_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AdaptationPendingDecisionStatus {
    decision_kind: AdaptationDecisionKind,
    producer_id: UnicaId,
    decision_ids: CanonicalStatusIds,
    replacement_pending_decision_ids: EmptyStatusIds,
    remaining_count: StatusCount,
    decision_set_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
enum PendingDecisionStatusKind {
    MergeConflict(MergeConflictPendingDecisionStatus),
    Adaptation(AdaptationPendingDecisionStatus),
}

/// Current decision heads grouped by producer. Deliberately not `Deserialize`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct PendingDecisionStatus(PendingDecisionStatusKind);

impl JsonSchema for PendingDecisionStatus {
    fn schema_name() -> Cow<'static, str> {
        "PendingDecisionStatus".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<MergeConflictPendingDecisionStatus>(),
            generator.subschema_for::<AdaptationPendingDecisionStatus>(),
        ])
    }
}

impl PendingDecisionStatus {
    pub(crate) fn merge_conflict(
        producer_id: UnicaId,
        decision_ids: Vec<UnicaId>,
        replacement_pending_decision_ids: Vec<UnicaId>,
        remaining_count: u64,
        decision_set_digest: Sha256Digest,
    ) -> Result<Self, StatusContractError> {
        Ok(Self(PendingDecisionStatusKind::MergeConflict(
            MergeConflictPendingDecisionStatus {
                decision_kind: MergeConflictDecisionKind::Value,
                producer_id,
                decision_ids: CanonicalStatusIds::new(decision_ids)?,
                replacement_pending_decision_ids: CanonicalStatusIds::new(
                    replacement_pending_decision_ids,
                )?,
                remaining_count: StatusCount::new(remaining_count)?,
                decision_set_digest,
            },
        )))
    }

    pub(crate) fn adaptation(
        producer_id: UnicaId,
        decision_ids: Vec<UnicaId>,
        remaining_count: u64,
        decision_set_digest: Sha256Digest,
    ) -> Result<Self, StatusContractError> {
        Ok(Self(PendingDecisionStatusKind::Adaptation(
            AdaptationPendingDecisionStatus {
                decision_kind: AdaptationDecisionKind::Value,
                producer_id,
                decision_ids: CanonicalStatusIds::new(decision_ids)?,
                replacement_pending_decision_ids: EmptyStatusIds::new(),
                remaining_count: StatusCount::new(remaining_count)?,
                decision_set_digest,
            },
        )))
    }
}

wire_literal!(RepositoryCursorAnchorKind, "repositoryCursor");
wire_literal!(TaskFingerprintAnchorKind, "taskFingerprint");
wire_literal!(OriginalFingerprintAnchorKind, "originalFingerprint");
wire_literal!(VendorFingerprintAnchorKind, "vendorFingerprint");

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RepositoryCursorAnchorStatus {
    anchor_kind: RepositoryCursorAnchorKind,
    cursor: RepositoryHistoryCursor,
    anchor_digest: Sha256Digest,
}

macro_rules! fingerprint_anchor_leaf {
    ($name:ident, $kind:ty) => {
        #[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct $name {
            anchor_kind: $kind,
            fingerprint: Sha256Digest,
            anchor_digest: Sha256Digest,
        }
    };
}

fingerprint_anchor_leaf!(TaskFingerprintAnchorStatus, TaskFingerprintAnchorKind);
fingerprint_anchor_leaf!(
    OriginalFingerprintAnchorStatus,
    OriginalFingerprintAnchorKind
);
fingerprint_anchor_leaf!(VendorFingerprintAnchorStatus, VendorFingerprintAnchorKind);

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
enum TaskAnchorStatusKind {
    RepositoryCursor(RepositoryCursorAnchorStatus),
    TaskFingerprint(TaskFingerprintAnchorStatus),
    OriginalFingerprint(OriginalFingerprintAnchorStatus),
    VendorFingerprint(VendorFingerprintAnchorStatus),
}

/// Typed task anchor projection. Deliberately not `Deserialize`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct TaskAnchorStatus(TaskAnchorStatusKind);

impl JsonSchema for TaskAnchorStatus {
    fn schema_name() -> Cow<'static, str> {
        "TaskAnchorStatus".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<RepositoryCursorAnchorStatus>(),
            generator.subschema_for::<TaskFingerprintAnchorStatus>(),
            generator.subschema_for::<OriginalFingerprintAnchorStatus>(),
            generator.subschema_for::<VendorFingerprintAnchorStatus>(),
        ])
    }
}

impl TaskAnchorStatus {
    pub(crate) fn repository_cursor(
        cursor: RepositoryHistoryCursor,
        anchor_digest: Sha256Digest,
    ) -> Self {
        Self(TaskAnchorStatusKind::RepositoryCursor(
            RepositoryCursorAnchorStatus {
                anchor_kind: RepositoryCursorAnchorKind::Value,
                cursor,
                anchor_digest,
            },
        ))
    }

    pub(crate) fn task_fingerprint(fingerprint: Sha256Digest, anchor_digest: Sha256Digest) -> Self {
        Self(TaskAnchorStatusKind::TaskFingerprint(
            TaskFingerprintAnchorStatus {
                anchor_kind: TaskFingerprintAnchorKind::Value,
                fingerprint,
                anchor_digest,
            },
        ))
    }

    pub(crate) fn original_fingerprint(
        fingerprint: Sha256Digest,
        anchor_digest: Sha256Digest,
    ) -> Self {
        Self(TaskAnchorStatusKind::OriginalFingerprint(
            OriginalFingerprintAnchorStatus {
                anchor_kind: OriginalFingerprintAnchorKind::Value,
                fingerprint,
                anchor_digest,
            },
        ))
    }

    pub(crate) fn vendor_fingerprint(
        fingerprint: Sha256Digest,
        anchor_digest: Sha256Digest,
    ) -> Self {
        Self(TaskAnchorStatusKind::VendorFingerprint(
            VendorFingerprintAnchorStatus {
                anchor_kind: VendorFingerprintAnchorKind::Value,
                fingerprint,
                anchor_digest,
            },
        ))
    }
}

/// Current repository lock projection. Deliberately not `Deserialize`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct OwnedLockStatus {
    target: RepositoryTargetIdentity,
    owner: RepositoryOwnerIdentity,
    acquisition_receipt_id: UnicaId,
    lock_digest: Sha256Digest,
}

impl OwnedLockStatus {
    pub(crate) fn new(
        target: RepositoryTargetIdentity,
        owner: RepositoryOwnerIdentity,
        acquisition_receipt_id: UnicaId,
        lock_digest: Sha256Digest,
    ) -> Self {
        Self {
            target,
            owner,
            acquisition_receipt_id,
            lock_digest,
        }
    }

    fn identity(&self) -> &RepositoryTargetIdentity {
        &self.target
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) enum ValidationGateKind {
    Checkpoint,
    Support,
    MainMerge,
    IntegrationSet,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) enum ValidationGateState {
    Current,
    Consumed,
}

/// Validation gate lifecycle projection. Deliberately not `Deserialize`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ValidationGateStatus {
    gate_kind: ValidationGateKind,
    gate_id: UnicaId,
    gate_digest: Sha256Digest,
    state: ValidationGateState,
}

impl ValidationGateStatus {
    pub(crate) fn new(
        gate_kind: ValidationGateKind,
        gate_id: UnicaId,
        gate_digest: Sha256Digest,
        state: ValidationGateState,
    ) -> Self {
        Self {
            gate_kind,
            gate_id,
            gate_digest,
            state,
        }
    }
}

/// Artifact content-address projection. Deliberately not `Deserialize`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ArtifactHashStatus {
    artifact_id: UnicaId,
    role: ArtifactRole,
    kind: ArtifactKind,
    sha256: Sha256Digest,
}

impl ArtifactHashStatus {
    pub(crate) fn new(
        artifact_id: UnicaId,
        role: ArtifactRole,
        kind: ArtifactKind,
        sha256: Sha256Digest,
    ) -> Self {
        Self {
            artifact_id,
            role,
            kind,
            sha256,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) enum TaskArchiveOutcome {
    Success,
    Abandoned,
}

/// Immutable archive projection retained through cleanup. Deliberately not
/// `Deserialize`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct TaskArchiveStatus {
    archive_id: UnicaId,
    outcome: TaskArchiveOutcome,
    sha256: Sha256Digest,
    retained_lineage_digest: Sha256Digest,
}

impl TaskArchiveStatus {
    pub(crate) fn from_publication(authority: ArchiveStatusProjectionAuthority) -> Self {
        let (archive_id, outcome, sha256, retained_lineage_digest) = authority.into_parts();
        Self {
            archive_id,
            outcome,
            sha256,
            retained_lineage_digest,
        }
    }

    #[cfg(test)]
    fn new(
        archive_id: UnicaId,
        outcome: TaskArchiveOutcome,
        sha256: Sha256Digest,
        retained_lineage_digest: Sha256Digest,
    ) -> Self {
        Self {
            archive_id,
            outcome,
            sha256,
            retained_lineage_digest,
        }
    }

    pub(crate) const fn archive_id(&self) -> &UnicaId {
        &self.archive_id
    }

    pub(crate) const fn outcome(&self) -> TaskArchiveOutcome {
        self.outcome
    }

    pub(crate) const fn sha256(&self) -> &Sha256Digest {
        &self.sha256
    }

    pub(crate) const fn retained_lineage_digest(&self) -> &Sha256Digest {
        &self.retained_lineage_digest
    }
}

impl PendingDecisionStatus {
    fn identity(&self) -> (u8, &UnicaId) {
        match &self.0 {
            PendingDecisionStatusKind::MergeConflict(value) => (0, &value.producer_id),
            PendingDecisionStatusKind::Adaptation(value) => (1, &value.producer_id),
        }
    }
}

impl TaskAnchorStatus {
    fn identity(&self) -> u8 {
        match &self.0 {
            TaskAnchorStatusKind::RepositoryCursor(_) => 0,
            TaskAnchorStatusKind::TaskFingerprint(_) => 1,
            TaskAnchorStatusKind::OriginalFingerprint(_) => 2,
            TaskAnchorStatusKind::VendorFingerprint(_) => 3,
        }
    }
}

macro_rules! canonical_status_array_schema {
    ($name:ident, $item:ty) => {
        impl JsonSchema for $name {
            fn schema_name() -> Cow<'static, str> {
                stringify!($name).into()
            }

            fn json_schema(generator: &mut SchemaGenerator) -> Schema {
                json_schema!({
                    "type": "array",
                    "items": generator.subschema_for::<$item>(),
                    "minItems": 0,
                    "maxItems": MAX_STATUS_ITEMS,
                    "uniqueItems": true
                })
            }
        }
    };
}

/// Canonical repository locks ordered by typed target identity. Deliberately
/// not `Deserialize`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct OwnedLockStatuses(Vec<OwnedLockStatus>);

impl OwnedLockStatuses {
    pub(crate) fn new(values: Vec<OwnedLockStatus>) -> Result<Self, StatusContractError> {
        if values.len() > MAX_STATUS_ITEMS
            || values
                .windows(2)
                .any(|pair| pair[0].identity() >= pair[1].identity())
        {
            return Err(StatusContractError(
                "owned lock statuses must be canonical and unique by target identity",
            ));
        }
        Ok(Self(values))
    }

    pub(crate) const fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

canonical_status_array_schema!(OwnedLockStatuses, OwnedLockStatus);

/// Canonical decision groups. Deliberately not `Deserialize`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct PendingDecisionStatuses(Vec<PendingDecisionStatus>);

impl PendingDecisionStatuses {
    pub(crate) fn new(values: Vec<PendingDecisionStatus>) -> Result<Self, StatusContractError> {
        if values.len() > MAX_STATUS_ITEMS
            || values
                .windows(2)
                .any(|pair| pair[0].identity() >= pair[1].identity())
        {
            return Err(StatusContractError(
                "pending decision statuses must be canonical and unique by kind and producer",
            ));
        }
        Ok(Self(values))
    }

    pub(crate) const fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

canonical_status_array_schema!(PendingDecisionStatuses, PendingDecisionStatus);

/// Canonical anchor kinds. Deliberately not `Deserialize`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct TaskAnchorStatuses(Vec<TaskAnchorStatus>);

impl TaskAnchorStatuses {
    pub(crate) fn new(values: Vec<TaskAnchorStatus>) -> Result<Self, StatusContractError> {
        if values.len() > MAX_STATUS_ITEMS
            || values
                .windows(2)
                .any(|pair| pair[0].identity() >= pair[1].identity())
        {
            return Err(StatusContractError(
                "task anchor statuses must be canonical and unique by anchor kind",
            ));
        }
        Ok(Self(values))
    }
}

canonical_status_array_schema!(TaskAnchorStatuses, TaskAnchorStatus);

/// Canonical validation gates. Deliberately not `Deserialize`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct ValidationGateStatuses(Vec<ValidationGateStatus>);

impl ValidationGateStatuses {
    pub(crate) fn new(values: Vec<ValidationGateStatus>) -> Result<Self, StatusContractError> {
        if values.len() > MAX_STATUS_ITEMS
            || values
                .windows(2)
                .any(|pair| pair[0].gate_id >= pair[1].gate_id)
        {
            return Err(StatusContractError(
                "validation gate statuses must be canonical and unique by gate ID",
            ));
        }
        Ok(Self(values))
    }
}

canonical_status_array_schema!(ValidationGateStatuses, ValidationGateStatus);

/// Canonical artifact hashes. Deliberately not `Deserialize`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct ArtifactHashStatuses(Vec<ArtifactHashStatus>);

impl ArtifactHashStatuses {
    pub(crate) fn new(values: Vec<ArtifactHashStatus>) -> Result<Self, StatusContractError> {
        if values.len() > MAX_STATUS_ITEMS
            || values
                .windows(2)
                .any(|pair| pair[0].artifact_id >= pair[1].artifact_id)
        {
            return Err(StatusContractError(
                "artifact hash statuses must be canonical and unique by artifact ID",
            ));
        }
        Ok(Self(values))
    }
}

canonical_status_array_schema!(ArtifactHashStatuses, ArtifactHashStatus);

fn stable_code_ordinal(value: &StableErrorCode) -> usize {
    StableErrorCode::ALL
        .iter()
        .position(|candidate| candidate == value)
        .expect("closed stable error code has an ordinal")
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
struct NonEmptyCanonicalStableCodes(Vec<StableErrorCode>);

impl NonEmptyCanonicalStableCodes {
    fn new(values: Vec<StableErrorCode>) -> Result<Self, StatusContractError> {
        if values.is_empty()
            || values.len() > MAX_STATUS_ITEMS
            || values
                .windows(2)
                .any(|pair| stable_code_ordinal(&pair[0]) >= stable_code_ordinal(&pair[1]))
        {
            return Err(StatusContractError(
                "cleanup blocker codes must be non-empty, canonical, and unique",
            ));
        }
        Ok(Self(values))
    }
}

impl JsonSchema for NonEmptyCanonicalStableCodes {
    fn schema_name() -> Cow<'static, str> {
        "NonEmptyCanonicalCleanupBlockerCodes".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "array",
            "items": generator.subschema_for::<StableErrorCode>(),
            "minItems": 1,
            "maxItems": MAX_STATUS_ITEMS,
            "uniqueItems": true
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
struct EmptyStableCodes(Vec<StableErrorCode>);

impl EmptyStableCodes {
    const fn new() -> Self {
        Self(Vec::new())
    }
}

impl JsonSchema for EmptyStableCodes {
    fn schema_name() -> Cow<'static, str> {
        "EmptyCleanupBlockerCodes".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "array",
            "items": generator.subschema_for::<StableErrorCode>(),
            "minItems": 0,
            "maxItems": 0
        })
    }
}

bool_literal!(EligibleLiteral, true);
bool_literal!(IneligibleLiteral, false);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct EligibleCleanupDigestRecord {
    eligible: EligibleLiteral,
    archive_id: UnicaId,
    blocker_codes: EmptyStableCodes,
}

impl contract_digest_record_sealed::Sealed for EligibleCleanupDigestRecord {}
impl ContractDigestRecord for EligibleCleanupDigestRecord {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ArchivedIneligibleCleanupDigestRecord {
    eligible: IneligibleLiteral,
    archive_id: UnicaId,
    blocker_codes: NonEmptyCanonicalStableCodes,
}

impl contract_digest_record_sealed::Sealed for ArchivedIneligibleCleanupDigestRecord {}
impl ContractDigestRecord for ArchivedIneligibleCleanupDigestRecord {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UnarchivedIneligibleCleanupDigestRecord {
    eligible: IneligibleLiteral,
    blocker_codes: NonEmptyCanonicalStableCodes,
}

impl contract_digest_record_sealed::Sealed for UnarchivedIneligibleCleanupDigestRecord {}
impl ContractDigestRecord for UnarchivedIneligibleCleanupDigestRecord {}

macro_rules! cleanup_eligibility_leaf {
    ($name:ident, $record:ty) => {
        #[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct $name {
            #[serde(flatten)]
            record: $record,
            eligibility_digest: Sha256Digest,
        }
    };
}

cleanup_eligibility_leaf!(EligibleCleanupStatus, EligibleCleanupDigestRecord);
cleanup_eligibility_leaf!(
    ArchivedIneligibleCleanupStatus,
    ArchivedIneligibleCleanupDigestRecord
);
cleanup_eligibility_leaf!(
    UnarchivedIneligibleCleanupStatus,
    UnarchivedIneligibleCleanupDigestRecord
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
enum CleanupEligibilityStatusKind {
    Eligible(EligibleCleanupStatus),
    ArchivedIneligible(ArchivedIneligibleCleanupStatus),
    UnarchivedIneligible(UnarchivedIneligibleCleanupStatus),
}

/// Exact cleanup eligibility matrix. Deliberately not `Deserialize`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct CleanupEligibilityStatus(CleanupEligibilityStatusKind);

impl JsonSchema for CleanupEligibilityStatus {
    fn schema_name() -> Cow<'static, str> {
        "CleanupEligibilityStatus".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<EligibleCleanupStatus>(),
            generator.subschema_for::<ArchivedIneligibleCleanupStatus>(),
            generator.subschema_for::<UnarchivedIneligibleCleanupStatus>(),
        ])
    }
}

impl CleanupEligibilityStatus {
    pub(crate) fn eligible(archive_id: UnicaId) -> Result<Self, StatusContractError> {
        let record = EligibleCleanupDigestRecord {
            eligible: EligibleLiteral,
            archive_id,
            blocker_codes: EmptyStableCodes::new(),
        };
        let eligibility_digest = status_digest(&record, "cleanup eligibility digest failed")?;
        Ok(Self(CleanupEligibilityStatusKind::Eligible(
            EligibleCleanupStatus {
                record,
                eligibility_digest,
            },
        )))
    }

    pub(crate) fn ineligible_with_archive(
        archive_id: UnicaId,
        blocker_codes: Vec<StableErrorCode>,
    ) -> Result<Self, StatusContractError> {
        let record = ArchivedIneligibleCleanupDigestRecord {
            eligible: IneligibleLiteral,
            archive_id,
            blocker_codes: NonEmptyCanonicalStableCodes::new(blocker_codes)?,
        };
        let eligibility_digest = status_digest(&record, "cleanup eligibility digest failed")?;
        Ok(Self(CleanupEligibilityStatusKind::ArchivedIneligible(
            ArchivedIneligibleCleanupStatus {
                record,
                eligibility_digest,
            },
        )))
    }

    pub(crate) fn ineligible_without_archive(
        blocker_codes: Vec<StableErrorCode>,
    ) -> Result<Self, StatusContractError> {
        let record = UnarchivedIneligibleCleanupDigestRecord {
            eligible: IneligibleLiteral,
            blocker_codes: NonEmptyCanonicalStableCodes::new(blocker_codes)?,
        };
        let eligibility_digest = status_digest(&record, "cleanup eligibility digest failed")?;
        Ok(Self(CleanupEligibilityStatusKind::UnarchivedIneligible(
            UnarchivedIneligibleCleanupStatus {
                record,
                eligibility_digest,
            },
        )))
    }

    #[cfg(test)]
    fn validates_json(value: &serde_json::Value) -> bool {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct UncheckedCleanupEligibilityStatus {
            eligible: bool,
            archive_id: Option<UnicaId>,
            blocker_codes: Vec<StableErrorCode>,
            eligibility_digest: Sha256Digest,
        }

        let Ok(unchecked) =
            serde_json::from_value::<UncheckedCleanupEligibilityStatus>(value.clone())
        else {
            return false;
        };
        let expected = match (unchecked.eligible, unchecked.archive_id) {
            (true, Some(archive_id)) if unchecked.blocker_codes.is_empty() => {
                Self::eligible(archive_id)
            }
            (false, Some(archive_id)) => {
                Self::ineligible_with_archive(archive_id, unchecked.blocker_codes)
            }
            (false, None) => Self::ineligible_without_archive(unchecked.blocker_codes),
            _ => return false,
        };
        let Ok(expected) = expected else {
            return false;
        };
        match expected.0 {
            CleanupEligibilityStatusKind::Eligible(status) => {
                status.eligibility_digest == unchecked.eligibility_digest
            }
            CleanupEligibilityStatusKind::ArchivedIneligible(status) => {
                status.eligibility_digest == unchecked.eligibility_digest
            }
            CleanupEligibilityStatusKind::UnarchivedIneligible(status) => {
                status.eligibility_digest == unchecked.eligibility_digest
            }
        }
    }

    pub(crate) const fn archive_id(&self) -> Option<&UnicaId> {
        match &self.0 {
            CleanupEligibilityStatusKind::Eligible(status) => Some(&status.record.archive_id),
            CleanupEligibilityStatusKind::ArchivedIneligible(status) => {
                Some(&status.record.archive_id)
            }
            CleanupEligibilityStatusKind::UnarchivedIneligible(_) => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) enum RecentTerminalKind {
    Completed,
    Stopped,
    Rejected,
}

/// Bounded terminal projection; the durable terminal envelope remains the
/// replay source. Deliberately not `Deserialize`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RecentOperationStatus {
    operation_id: OperationId,
    operation: TaskOperationSelector,
    terminal_kind: RecentTerminalKind,
    result_digest: Sha256Digest,
}

impl RecentOperationStatus {
    #[cfg(test)]
    fn new_test_only(
        operation_id: OperationId,
        operation: TaskOperationSelector,
        terminal_kind: RecentTerminalKind,
        result_digest: Sha256Digest,
    ) -> Self {
        Self {
            operation_id,
            operation,
            terminal_kind,
            result_digest,
        }
    }
}

/// Canonical recent terminal projection ordered by operation ID. Deliberately
/// not `Deserialize`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct RecentOperations(Vec<RecentOperationStatus>);

impl RecentOperations {
    pub(crate) fn new(values: Vec<RecentOperationStatus>) -> Result<Self, StatusContractError> {
        if values.len() > MAX_STATUS_ITEMS
            || values
                .windows(2)
                .any(|pair| pair[0].operation_id >= pair[1].operation_id)
        {
            return Err(StatusContractError(
                "recent operations must be bounded, canonical, and unique by operation ID",
            ));
        }
        Ok(Self(values))
    }
}

impl JsonSchema for RecentOperations {
    fn schema_name() -> Cow<'static, str> {
        "RecentOperations".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "array",
            "items": generator.subschema_for::<RecentOperationStatus>(),
            "minItems": 0,
            "maxItems": MAX_STATUS_ITEMS,
            "uniqueItems": true
        })
    }
}

wire_literal!(ArtifactHandleKind, "artifact");
wire_literal!(WorkspaceHandleKind, "workspace");
wire_literal!(
    MergeResolutionWorkspaceHandleKind,
    "mergeResolutionWorkspace"
);
wire_literal!(CheckpointHandleKind, "checkpoint");
wire_literal!(ComparisonHandleKind, "comparison");
wire_literal!(
    DeferredRepositoryAdvanceHandleKind,
    "deferredRepositoryAdvance"
);
wire_literal!(RecoveryHandleKind, "recovery");
wire_literal!(ArchiveHandleKind, "archive");

/// Current verified artifact handle. Deliberately not `Deserialize`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ArtifactResumeHandle {
    handle_kind: ArtifactHandleKind,
    artifact_id: UnicaId,
    role: ArtifactRole,
    kind: ArtifactKind,
    sha256: Sha256Digest,
    #[serde(skip_serializing_if = "Option::is_none")]
    verification_id: Option<UnicaId>,
}

impl ArtifactResumeHandle {
    pub(crate) fn new(
        artifact_id: UnicaId,
        role: ArtifactRole,
        kind: ArtifactKind,
        sha256: Sha256Digest,
        verification_id: Option<UnicaId>,
    ) -> Self {
        Self {
            handle_kind: ArtifactHandleKind::Value,
            artifact_id,
            role,
            kind,
            sha256,
            verification_id,
        }
    }
}

/// Opaque task-workspace handle. Deliberately not `Deserialize`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct WorkspaceResumeHandle {
    handle_kind: WorkspaceHandleKind,
    task_workspace_id: UnicaId,
}

impl WorkspaceResumeHandle {
    pub(crate) const fn new(task_workspace_id: UnicaId) -> Self {
        Self {
            handle_kind: WorkspaceHandleKind::Value,
            task_workspace_id,
        }
    }

    pub(crate) const fn task_workspace_id(&self) -> &UnicaId {
        &self.task_workspace_id
    }
}

/// Opaque resolution workspace generation handle. Deliberately not
/// `Deserialize`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct MergeResolutionWorkspaceResumeHandle {
    handle_kind: MergeResolutionWorkspaceHandleKind,
    session_id: UnicaId,
    workspace_id: UnicaId,
    base_session_digest: Sha256Digest,
}

impl MergeResolutionWorkspaceResumeHandle {
    pub(crate) const fn new(
        session_id: UnicaId,
        workspace_id: UnicaId,
        base_session_digest: Sha256Digest,
    ) -> Self {
        Self {
            handle_kind: MergeResolutionWorkspaceHandleKind::Value,
            session_id,
            workspace_id,
            base_session_digest,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) enum CheckpointScope {
    Local,
    Synchronized,
}

/// Immutable checkpoint handle. Deliberately not `Deserialize`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CheckpointResumeHandle {
    handle_kind: CheckpointHandleKind,
    checkpoint_id: UnicaId,
    scope: CheckpointScope,
    source_fingerprint: Sha256Digest,
}

impl CheckpointResumeHandle {
    pub(crate) const fn new(
        checkpoint_id: UnicaId,
        scope: CheckpointScope,
        source_fingerprint: Sha256Digest,
    ) -> Self {
        Self {
            handle_kind: CheckpointHandleKind::Value,
            checkpoint_id,
            scope,
            source_fingerprint,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) enum ComparisonScope {
    ProjectDelta,
    MainIntegration,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ComparisonArtifactAnchor {
    artifact_id: UnicaId,
}

/// Path-free comparison anchor. Deliberately not `Deserialize`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ComparisonStatusAnchor {
    OriginalCurrent,
    Repository,
    TaskCurrent,
    TaskVendor,
    Artifact(UnicaId),
}

impl Serialize for ComparisonStatusAnchor {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            Self::OriginalCurrent => serializer.serialize_str("originalCurrent"),
            Self::Repository => serializer.serialize_str("repository"),
            Self::TaskCurrent => serializer.serialize_str("taskCurrent"),
            Self::TaskVendor => serializer.serialize_str("taskVendor"),
            Self::Artifact(artifact_id) => ComparisonArtifactAnchor {
                artifact_id: artifact_id.clone(),
            }
            .serialize(serializer),
        }
    }
}

impl JsonSchema for ComparisonStatusAnchor {
    fn schema_name() -> Cow<'static, str> {
        "ComparisonStatusAnchor".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            json_schema!({
                "type": "string",
                "enum": ["originalCurrent", "repository", "taskCurrent", "taskVendor"]
            }),
            generator.subschema_for::<ComparisonArtifactAnchor>(),
        ])
    }
}

/// Immutable comparison handle. Deliberately not `Deserialize`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ComparisonResumeHandle {
    handle_kind: ComparisonHandleKind,
    comparison_id: UnicaId,
    scope: ComparisonScope,
    left_anchor: ComparisonStatusAnchor,
    right_anchor: ComparisonStatusAnchor,
    delta_digest: Sha256Digest,
}

impl ComparisonResumeHandle {
    pub(crate) const fn new(
        comparison_id: UnicaId,
        scope: ComparisonScope,
        left_anchor: ComparisonStatusAnchor,
        right_anchor: ComparisonStatusAnchor,
        delta_digest: Sha256Digest,
    ) -> Self {
        Self {
            handle_kind: ComparisonHandleKind::Value,
            comparison_id,
            scope,
            left_anchor,
            right_anchor,
            delta_digest,
        }
    }
}

/// Current unconsumed deferred repository advance. Deliberately not
/// `Deserialize`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct DeferredRepositoryAdvanceResumeHandle {
    handle_kind: DeferredRepositoryAdvanceHandleKind,
    advance: DeferredRepositoryAdvance,
}

impl DeferredRepositoryAdvanceResumeHandle {
    pub(crate) const fn new(advance: DeferredRepositoryAdvance) -> Self {
        Self {
            handle_kind: DeferredRepositoryAdvanceHandleKind::Value,
            advance,
        }
    }
}

/// Minimal current recovery handle. Deliberately not `Deserialize`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RecoveryResumeHandle {
    handle_kind: RecoveryHandleKind,
    prior_operation_id: OperationId,
    recovery_digest: Sha256Digest,
}

impl RecoveryResumeHandle {
    pub(crate) const fn new(
        prior_operation_id: OperationId,
        recovery_digest: Sha256Digest,
    ) -> Self {
        Self {
            handle_kind: RecoveryHandleKind::Value,
            prior_operation_id,
            recovery_digest,
        }
    }

    pub(crate) const fn prior_operation_id(&self) -> &OperationId {
        &self.prior_operation_id
    }

    pub(crate) const fn recovery_digest(&self) -> &Sha256Digest {
        &self.recovery_digest
    }
}

/// Minimal retained archive handle. Deliberately not `Deserialize`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ArchiveResumeHandle {
    handle_kind: ArchiveHandleKind,
    archive_id: UnicaId,
    sha256: Sha256Digest,
    outcome: TaskArchiveOutcome,
    retained_lineage_digest: Sha256Digest,
}

impl ArchiveResumeHandle {
    pub(crate) fn from_archive_status(archive: &TaskArchiveStatus) -> Self {
        Self {
            handle_kind: ArchiveHandleKind::Value,
            archive_id: archive.archive_id.clone(),
            sha256: archive.sha256.clone(),
            outcome: archive.outcome,
            retained_lineage_digest: archive.retained_lineage_digest.clone(),
        }
    }

    #[cfg(test)]
    fn new(
        archive_id: UnicaId,
        sha256: Sha256Digest,
        outcome: TaskArchiveOutcome,
        retained_lineage_digest: Sha256Digest,
    ) -> Self {
        Self {
            handle_kind: ArchiveHandleKind::Value,
            archive_id,
            sha256,
            outcome,
            retained_lineage_digest,
        }
    }

    pub(crate) const fn archive_id(&self) -> &UnicaId {
        &self.archive_id
    }

    pub(crate) const fn sha256(&self) -> &Sha256Digest {
        &self.sha256
    }

    pub(crate) const fn outcome(&self) -> TaskArchiveOutcome {
        self.outcome
    }

    pub(crate) const fn retained_lineage_digest(&self) -> &Sha256Digest {
        &self.retained_lineage_digest
    }
}

wire_literal!(MergeSessionHandleKind, "mergeSession");
wire_literal!(SupportedUpdateSessionMode, "supportedUpdate");
wire_literal!(MainIntegrationSessionMode, "mainIntegration");

macro_rules! merge_session_leaf {
    ($name:ident, $mode:ty, {$($field:ident: $field_type:ty),* $(,)?}) => {
        #[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct $name {
            handle_kind: MergeSessionHandleKind,
            session_id: UnicaId,
            mode: $mode,
            checkpoint_id: UnicaId,
            $($field: $field_type,)*
            comparison_id: UnicaId,
            base_session_digest: Sha256Digest,
            decision_set_digest: Sha256Digest,
            #[serde(skip_serializing_if = "Option::is_none")]
            resolved_session_digest: Option<Sha256Digest>,
            conflict_count: StatusCount,
        }
    };
}

merge_session_leaf!(SupportedUpdateMergeSessionResumeHandle, SupportedUpdateSessionMode, {
    incoming_distribution_id: UnicaId
});
merge_session_leaf!(MainIntegrationMergeSessionResumeHandle, MainIntegrationSessionMode, {
    support_gate_id: UnicaId,
    support_gate_digest: Sha256Digest,
    support_gate_history_evidence_digest: Sha256Digest
});

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
enum MergeSessionResumeHandleKind {
    SupportedUpdate(SupportedUpdateMergeSessionResumeHandle),
    MainIntegration(MainIntegrationMergeSessionResumeHandle),
}

/// Current merge session with an exact mode-specific field matrix.
/// Deliberately not `Deserialize`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct MergeSessionResumeHandle(MergeSessionResumeHandleKind);

impl JsonSchema for MergeSessionResumeHandle {
    fn schema_name() -> Cow<'static, str> {
        "MergeSessionResumeHandle".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<SupportedUpdateMergeSessionResumeHandle>(),
            generator.subschema_for::<MainIntegrationMergeSessionResumeHandle>(),
        ])
    }
}

wire_literal!(DecisionHandleKind, "decision");
wire_literal!(MergeConflictResumeDecisionKind, "mergeConflict");
wire_literal!(AdaptationResumeDecisionKind, "adaptation");
bool_literal!(CurrentDecisionLiteral, true);
bool_literal!(HistoricalDecisionLiteral, false);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MergeConflictDecisionResumeBody {
    handle_kind: DecisionHandleKind,
    decision_id: UnicaId,
    decision_kind: MergeConflictResumeDecisionKind,
    session_id: UnicaId,
    base_session_digest: Sha256Digest,
    conflict_id: UnicaId,
    resolution: ConflictResolution,
    rationale_digest: Sha256Digest,
    #[serde(skip_serializing_if = "Option::is_none")]
    change_receipt_digest: Option<Sha256Digest>,
    #[serde(skip_serializing_if = "Option::is_none")]
    replaces_decision_id: Option<UnicaId>,
    decision_digest: Sha256Digest,
    revised_decision_set_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CurrentMergeConflictDecisionResumeHandle {
    #[serde(flatten)]
    body: MergeConflictDecisionResumeBody,
    current: CurrentDecisionLiteral,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SupersededMergeConflictDecisionResumeHandle {
    #[serde(flatten)]
    body: MergeConflictDecisionResumeBody,
    current: HistoricalDecisionLiteral,
    superseded_by_change_receipt_id: UnicaId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReplacedMergeConflictDecisionResumeHandle {
    #[serde(flatten)]
    body: MergeConflictDecisionResumeBody,
    current: HistoricalDecisionLiteral,
    replaced_by_decision_id: UnicaId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SupersededAndReplacedMergeConflictDecisionResumeHandle {
    #[serde(flatten)]
    body: MergeConflictDecisionResumeBody,
    current: HistoricalDecisionLiteral,
    superseded_by_change_receipt_id: UnicaId,
    replaced_by_decision_id: UnicaId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AdaptationDecisionResumeHandle {
    handle_kind: DecisionHandleKind,
    decision_id: UnicaId,
    decision_kind: AdaptationResumeDecisionKind,
    verification_id: UnicaId,
    adaptation_decision_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
enum DecisionResumeHandleKind {
    CurrentMergeConflict(CurrentMergeConflictDecisionResumeHandle),
    SupersededMergeConflict(SupersededMergeConflictDecisionResumeHandle),
    ReplacedMergeConflict(ReplacedMergeConflictDecisionResumeHandle),
    SupersededAndReplacedMergeConflict(SupersededAndReplacedMergeConflictDecisionResumeHandle),
    Adaptation(AdaptationDecisionResumeHandle),
}

/// Current or immutable historical decision handle. Deliberately not
/// `Deserialize`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct DecisionResumeHandle(DecisionResumeHandleKind);

impl JsonSchema for DecisionResumeHandle {
    fn schema_name() -> Cow<'static, str> {
        "DecisionResumeHandle".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<CurrentMergeConflictDecisionResumeHandle>(),
            generator.subschema_for::<SupersededMergeConflictDecisionResumeHandle>(),
            generator.subschema_for::<ReplacedMergeConflictDecisionResumeHandle>(),
            generator.subschema_for::<SupersededAndReplacedMergeConflictDecisionResumeHandle>(),
            generator.subschema_for::<AdaptationDecisionResumeHandle>(),
        ])
    }
}

wire_literal!(ResolutionChangeReceiptHandleKind, "resolutionChangeReceipt");
wire_literal!(
    SynchronizationConflictsStatusPhase,
    "synchronizationConflicts"
);
bool_literal!(ConsumedReceiptLiteral, true);
bool_literal!(UnconsumedReceiptLiteral, false);
bool_literal!(SelectableReceiptLiteral, true);
bool_literal!(UnselectableReceiptLiteral, false);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct MergeResolutionStatusPhaseTransition {
    phase_before: SynchronizationConflictsStatusPhase,
    resulting_phase: SynchronizationConflictsStatusPhase,
}

impl MergeResolutionStatusPhaseTransition {
    const VALUE: Self = Self {
        phase_before: SynchronizationConflictsStatusPhase::Value,
        resulting_phase: SynchronizationConflictsStatusPhase::Value,
    };
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ResolutionChangeReceiptResumeBody {
    handle_kind: ResolutionChangeReceiptHandleKind,
    change_receipt_id: UnicaId,
    affected_target: BranchedAffectedTarget,
    after_sha256: Sha256Digest,
    change_receipt_digest: Sha256Digest,
    superseded_change_receipt_ids: CanonicalStatusIds,
    superseded_decision_ids: CanonicalStatusIds,
    #[serde(skip_serializing_if = "Option::is_none")]
    pending_replacement_decision_id: Option<UnicaId>,
    decision_set_digest_before: Sha256Digest,
    revised_decision_set_digest: Sha256Digest,
    phase_transition: MergeResolutionStatusPhaseTransition,
    base_session_digest: Sha256Digest,
    workspace_generation_id: UnicaId,
    receipt_sequence: ChangeReceiptSequence,
}

macro_rules! resolution_receipt_leaf {
    ($name:ident, $consumed:ty, $selectable:ty, {$($field:ident: $field_type:ty),* $(,)?}) => {
        #[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct $name {
            #[serde(flatten)]
            body: ResolutionChangeReceiptResumeBody,
            consumed: $consumed,
            selectable: $selectable,
            $($field: $field_type,)*
        }
    };
}

resolution_receipt_leaf!(
    SelectableResolutionChangeReceiptResumeHandle,
    UnconsumedReceiptLiteral,
    SelectableReceiptLiteral,
    {}
);
resolution_receipt_leaf!(
    InvalidatedResolutionChangeReceiptResumeHandle,
    UnconsumedReceiptLiteral,
    UnselectableReceiptLiteral,
    {}
);
resolution_receipt_leaf!(SupersededResolutionChangeReceiptResumeHandle, UnconsumedReceiptLiteral, UnselectableReceiptLiteral, {
    superseded_by_receipt_id: UnicaId
});
resolution_receipt_leaf!(
    ConsumedResolutionChangeReceiptResumeHandle,
    ConsumedReceiptLiteral,
    UnselectableReceiptLiteral,
    {}
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
enum ResolutionChangeReceiptResumeHandleKind {
    Selectable(SelectableResolutionChangeReceiptResumeHandle),
    Invalidated(InvalidatedResolutionChangeReceiptResumeHandle),
    Superseded(SupersededResolutionChangeReceiptResumeHandle),
    Consumed(ConsumedResolutionChangeReceiptResumeHandle),
}

/// Exact changed-receipt lifecycle projection. Deliberately not `Deserialize`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct ResolutionChangeReceiptResumeHandle(ResolutionChangeReceiptResumeHandleKind);

impl JsonSchema for ResolutionChangeReceiptResumeHandle {
    fn schema_name() -> Cow<'static, str> {
        "ResolutionChangeReceiptResumeHandle".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<SelectableResolutionChangeReceiptResumeHandle>(),
            generator.subschema_for::<InvalidatedResolutionChangeReceiptResumeHandle>(),
            generator.subschema_for::<SupersededResolutionChangeReceiptResumeHandle>(),
            generator.subschema_for::<ConsumedResolutionChangeReceiptResumeHandle>(),
        ])
    }
}

wire_literal!(VerificationHandleKind, "verification");
wire_literal!(LocalCheckpointVerificationScope, "localCheckpoint");
wire_literal!(SynchronizedTaskVerificationScope, "synchronizedTask");
wire_literal!(MainSandboxVerificationScope, "mainSandbox");
wire_literal!(MainIntegrationVerificationScope, "mainIntegration");
wire_literal!(ValidVerificationOutcome, "valid");
wire_literal!(InvalidVerificationOutcome, "invalid");
wire_literal!(EquivalentVerificationOutcome, "equivalent");
wire_literal!(AdaptedVerificationOutcome, "adapted");
wire_literal!(UnexpectedVerificationOutcome, "unexpected");

macro_rules! verification_resume_leaf {
    ($name:ident, $scope:ty, $outcome:ty, {$($field:ident: $field_type:ty),* $(,)?}) => {
        #[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct $name {
            handle_kind: VerificationHandleKind,
            verification_id: UnicaId,
            scope: $scope,
            $($field: $field_type,)*
            outcome: $outcome,
            verification_digest: Sha256Digest,
            canonical_delta_digest: Sha256Digest,
        }
    };
}

verification_resume_leaf!(LocalCheckpointValidVerificationResumeHandle, LocalCheckpointVerificationScope, ValidVerificationOutcome, {
    checkpoint_id: UnicaId
});
verification_resume_leaf!(
    LocalCheckpointInvalidVerificationResumeHandle,
    LocalCheckpointVerificationScope,
    InvalidVerificationOutcome,
    {}
);
verification_resume_leaf!(SynchronizedEquivalentVerificationResumeHandle, SynchronizedTaskVerificationScope, EquivalentVerificationOutcome, {
    session_id: UnicaId,
    checkpoint_id: UnicaId
});
verification_resume_leaf!(SynchronizedAdaptedVerificationResumeHandle, SynchronizedTaskVerificationScope, AdaptedVerificationOutcome, {
    session_id: UnicaId,
    checkpoint_id: UnicaId,
    difference_manifest_id: UnicaId,
    difference_digest: Sha256Digest,
    adaptation_decision_id: UnicaId
});
verification_resume_leaf!(SynchronizedUnexpectedVerificationResumeHandle, SynchronizedTaskVerificationScope, UnexpectedVerificationOutcome, {
    session_id: UnicaId,
    difference_manifest_id: UnicaId,
    difference_digest: Sha256Digest
});
verification_resume_leaf!(SynchronizedInvalidVerificationResumeHandle, SynchronizedTaskVerificationScope, InvalidVerificationOutcome, {
    session_id: UnicaId
});
verification_resume_leaf!(MainSandboxValidVerificationResumeHandle, MainSandboxVerificationScope, ValidVerificationOutcome, {
    session_id: UnicaId,
    support_gate_history_evidence_digest: Sha256Digest
});
verification_resume_leaf!(MainSandboxInvalidVerificationResumeHandle, MainSandboxVerificationScope, InvalidVerificationOutcome, {
    session_id: UnicaId,
    support_gate_history_evidence_digest: Sha256Digest
});
verification_resume_leaf!(MainIntegrationValidVerificationResumeHandle, MainIntegrationVerificationScope, ValidVerificationOutcome, {
    session_id: UnicaId,
    merge_receipt_id: UnicaId,
    integration_set_digest: Sha256Digest,
    support_gate_history_evidence_digest: Sha256Digest
});
verification_resume_leaf!(MainIntegrationInvalidVerificationResumeHandle, MainIntegrationVerificationScope, InvalidVerificationOutcome, {
    session_id: UnicaId,
    merge_receipt_id: UnicaId,
    integration_set_digest: Sha256Digest,
    support_gate_history_evidence_digest: Sha256Digest
});

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
enum VerificationResumeHandleKind {
    LocalCheckpointValid(LocalCheckpointValidVerificationResumeHandle),
    LocalCheckpointInvalid(LocalCheckpointInvalidVerificationResumeHandle),
    SynchronizedEquivalent(SynchronizedEquivalentVerificationResumeHandle),
    SynchronizedAdapted(SynchronizedAdaptedVerificationResumeHandle),
    SynchronizedUnexpected(SynchronizedUnexpectedVerificationResumeHandle),
    SynchronizedInvalid(SynchronizedInvalidVerificationResumeHandle),
    MainSandboxValid(MainSandboxValidVerificationResumeHandle),
    MainSandboxInvalid(MainSandboxInvalidVerificationResumeHandle),
    MainIntegrationValid(MainIntegrationValidVerificationResumeHandle),
    MainIntegrationInvalid(MainIntegrationInvalidVerificationResumeHandle),
}

/// Exact verification scope/outcome projection. Deliberately not `Deserialize`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct VerificationResumeHandle(VerificationResumeHandleKind);

impl JsonSchema for VerificationResumeHandle {
    fn schema_name() -> Cow<'static, str> {
        "VerificationResumeHandle".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<LocalCheckpointValidVerificationResumeHandle>(),
            generator.subschema_for::<LocalCheckpointInvalidVerificationResumeHandle>(),
            generator.subschema_for::<SynchronizedEquivalentVerificationResumeHandle>(),
            generator.subschema_for::<SynchronizedAdaptedVerificationResumeHandle>(),
            generator.subschema_for::<SynchronizedUnexpectedVerificationResumeHandle>(),
            generator.subschema_for::<SynchronizedInvalidVerificationResumeHandle>(),
            generator.subschema_for::<MainSandboxValidVerificationResumeHandle>(),
            generator.subschema_for::<MainSandboxInvalidVerificationResumeHandle>(),
            generator.subschema_for::<MainIntegrationValidVerificationResumeHandle>(),
            generator.subschema_for::<MainIntegrationInvalidVerificationResumeHandle>(),
        ])
    }
}

wire_literal!(MergeApplyHandleKind, "mergeApply");
wire_literal!(TaskMergeApplyTarget, "task");
wire_literal!(OriginalMergeApplyTarget, "original");

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TaskMergeApplyResumeHandle {
    handle_kind: MergeApplyHandleKind,
    merge_receipt_id: UnicaId,
    target: TaskMergeApplyTarget,
    session_id: UnicaId,
    resolved_session_digest: Sha256Digest,
    result_fingerprint: Sha256Digest,
    source_publication_id: UnicaId,
    source_fingerprint: Sha256Digest,
    task_infobase_fingerprint: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct OriginalMergeApplyResumeHandle {
    handle_kind: MergeApplyHandleKind,
    merge_receipt_id: UnicaId,
    target: OriginalMergeApplyTarget,
    session_id: UnicaId,
    resolved_session_digest: Sha256Digest,
    result_fingerprint: Sha256Digest,
    repository_history_cursor: RepositoryHistoryCursor,
    rollback_checkpoint_id: UnicaId,
    integration_set_id: UnicaId,
    integration_set_digest: Sha256Digest,
    support_gate_history_evidence_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
enum MergeApplyResumeHandleKind {
    Task(TaskMergeApplyResumeHandle),
    Original(OriginalMergeApplyResumeHandle),
}

/// Exact task/original apply projection. Deliberately not `Deserialize`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct MergeApplyResumeHandle(MergeApplyResumeHandleKind);

impl JsonSchema for MergeApplyResumeHandle {
    fn schema_name() -> Cow<'static, str> {
        "MergeApplyResumeHandle".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<TaskMergeApplyResumeHandle>(),
            generator.subschema_for::<OriginalMergeApplyResumeHandle>(),
        ])
    }
}

wire_literal!(LockPlanHandleKind, "lockPlan");
wire_literal!(LockSetHandleKind, "lockSet");

/// Immutable lock-plan handle. Deliberately not `Deserialize`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct LockPlanResumeHandle {
    handle_kind: LockPlanHandleKind,
    plan_id: UnicaId,
    plan_digest: Sha256Digest,
    merge_session_id: UnicaId,
    resolved_session_digest: Sha256Digest,
    support_gate_id: UnicaId,
    support_gate_digest: Sha256Digest,
    support_gate_history_evidence_digest: Sha256Digest,
    verification_id: UnicaId,
    verification_digest: Sha256Digest,
    integration_set_id: UnicaId,
    integration_set_digest: Sha256Digest,
}

/// Immutable acquired lock-set handle. Deliberately not `Deserialize`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct LockSetResumeHandle {
    handle_kind: LockSetHandleKind,
    lock_set_id: UnicaId,
    lock_set_digest: Sha256Digest,
    plan_id: UnicaId,
    plan_digest: Sha256Digest,
    integration_set_id: UnicaId,
    integration_set_digest: Sha256Digest,
    support_gate_history_evidence_digest: Sha256Digest,
}

wire_literal!(PreviewHandleKind, "preview");
wire_literal!(BranchedArchivePreviewToolName, "unica.branched.archive");
wire_literal!(BranchedCleanupPreviewToolName, "unica.branched.cleanup");
wire_literal!(DeliveryCreatePreviewToolName, "unica.delivery.create");
wire_literal!(DeliveryDeployPreviewToolName, "unica.delivery.deploy");
wire_literal!(RepositoryUpdatePreviewToolName, "unica.repository.update");
wire_literal!(RepositoryCommitPreviewToolName, "unica.repository.commit");
wire_literal!(ArchiveSuccessPreviewOutcome, "success");
wire_literal!(ArchiveAbandonedPreviewOutcome, "abandoned");
wire_literal!(BaselineDistributionPreviewRole, "baselineDistribution");
wire_literal!(RefreshDistributionPreviewRole, "refreshDistribution");
wire_literal!(RoutineUpdatePreviewMode, "routine");
wire_literal!(SupportPrerequisitePreviewMode, "supportPrerequisite");
wire_literal!(
    SupportPrerequisiteCancellationPreviewMode,
    "supportPrerequisiteCancellation"
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ArchiveSuccessPreviewRequest {
    outcome: ArchiveSuccessPreviewOutcome,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ArchiveAbandonedPreviewRequest {
    outcome: ArchiveAbandonedPreviewOutcome,
    reason: Reason,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
enum ArchivePreviewRequestKind {
    Success(ArchiveSuccessPreviewRequest),
    Abandoned(ArchiveAbandonedPreviewRequest),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct ArchivePreviewRequest(ArchivePreviewRequestKind);

impl JsonSchema for ArchivePreviewRequest {
    fn schema_name() -> Cow<'static, str> {
        "ArchivePreviewRequest".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<ArchiveSuccessPreviewRequest>(),
            generator.subschema_for::<ArchiveAbandonedPreviewRequest>(),
        ])
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CleanupPreviewRequest {
    archive_id: UnicaId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BaselineDeliveryCreatePreviewRequest {
    role: BaselineDistributionPreviewRole,
    inspection_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RefreshDeliveryCreatePreviewRequest {
    role: RefreshDistributionPreviewRole,
    inspection_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
enum DeliveryCreatePreviewRequestKind {
    Baseline(BaselineDeliveryCreatePreviewRequest),
    Refresh(RefreshDeliveryCreatePreviewRequest),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct DeliveryCreatePreviewRequest(DeliveryCreatePreviewRequestKind);

impl JsonSchema for DeliveryCreatePreviewRequest {
    fn schema_name() -> Cow<'static, str> {
        "DeliveryCreatePreviewRequest".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<BaselineDeliveryCreatePreviewRequest>(),
            generator.subschema_for::<RefreshDeliveryCreatePreviewRequest>(),
        ])
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct DeliveryDeployPreviewRequest {
    distribution_id: UnicaId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RoutineRepositoryUpdatePreviewRequest {
    mode: RoutineUpdatePreviewMode,
    expected_status_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SupportPrerequisiteRepositoryUpdatePreviewRequest {
    mode: SupportPrerequisitePreviewMode,
    expected_status_digest: Sha256Digest,
    support_action_id: UnicaId,
    expected_support_action_digest: Sha256Digest,
    expected_arming_receipt_id: UnicaId,
    expected_arming_receipt_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RepositoryCommitPreviewRequest {
    integration_set_id: UnicaId,
    expected_integration_set_digest: Sha256Digest,
    lock_set_id: UnicaId,
    expected_lock_set_digest: Sha256Digest,
    verification_id: UnicaId,
    expected_verification_digest: Sha256Digest,
    merge_receipt_id: UnicaId,
    support_gate_id: UnicaId,
    expected_support_gate_digest: Sha256Digest,
    expected_support_gate_history_evidence_digest: Sha256Digest,
    expected_authorized_post_merge_fingerprint: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AwaitingArmSupportCancellationPreviewRequest {
    mode: SupportPrerequisiteCancellationPreviewMode,
    expected_status_digest: Sha256Digest,
    support_action_id: UnicaId,
    expected_support_action_digest: Sha256Digest,
    reason: Reason,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ArmedSupportCancellationPreviewRequest {
    mode: SupportPrerequisiteCancellationPreviewMode,
    expected_status_digest: Sha256Digest,
    support_action_id: UnicaId,
    expected_support_action_digest: Sha256Digest,
    expected_arming_receipt_id: UnicaId,
    expected_arming_receipt_digest: Sha256Digest,
    reason: Reason,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
enum SupportCancellationPreviewRequestKind {
    AwaitingArm(AwaitingArmSupportCancellationPreviewRequest),
    Armed(ArmedSupportCancellationPreviewRequest),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct SupportCancellationPreviewRequest(SupportCancellationPreviewRequestKind);

impl JsonSchema for SupportCancellationPreviewRequest {
    fn schema_name() -> Cow<'static, str> {
        "SupportCancellationPreviewRequest".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<AwaitingArmSupportCancellationPreviewRequest>(),
            generator.subschema_for::<ArmedSupportCancellationPreviewRequest>(),
        ])
    }
}

macro_rules! preview_resume_leaf {
    ($name:ident, $tool_name:ty, $request:ty) => {
        #[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct $name {
            handle_kind: PreviewHandleKind,
            tool_name: $tool_name,
            preview_operation_id: OperationId,
            preview_digest: Sha256Digest,
            request: $request,
        }
    };
}

preview_resume_leaf!(
    ArchivePreviewResumeHandle,
    BranchedArchivePreviewToolName,
    ArchivePreviewRequest
);
preview_resume_leaf!(
    CleanupPreviewResumeHandle,
    BranchedCleanupPreviewToolName,
    CleanupPreviewRequest
);
preview_resume_leaf!(
    DeliveryCreatePreviewResumeHandle,
    DeliveryCreatePreviewToolName,
    DeliveryCreatePreviewRequest
);
preview_resume_leaf!(
    DeliveryDeployPreviewResumeHandle,
    DeliveryDeployPreviewToolName,
    DeliveryDeployPreviewRequest
);
preview_resume_leaf!(
    RoutineRepositoryUpdatePreviewResumeHandle,
    RepositoryUpdatePreviewToolName,
    RoutineRepositoryUpdatePreviewRequest
);
preview_resume_leaf!(
    SupportPrerequisiteRepositoryUpdatePreviewResumeHandle,
    RepositoryUpdatePreviewToolName,
    SupportPrerequisiteRepositoryUpdatePreviewRequest
);
preview_resume_leaf!(
    RepositoryCommitPreviewResumeHandle,
    RepositoryCommitPreviewToolName,
    RepositoryCommitPreviewRequest
);
preview_resume_leaf!(
    SupportCancellationPreviewResumeHandle,
    RepositoryUpdatePreviewToolName,
    SupportCancellationPreviewRequest
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
enum PreviewResumeHandleKind {
    Archive(ArchivePreviewResumeHandle),
    Cleanup(CleanupPreviewResumeHandle),
    DeliveryCreate(DeliveryCreatePreviewResumeHandle),
    DeliveryDeploy(DeliveryDeployPreviewResumeHandle),
    RoutineRepositoryUpdate(RoutineRepositoryUpdatePreviewResumeHandle),
    SupportPrerequisiteRepositoryUpdate(SupportPrerequisiteRepositoryUpdatePreviewResumeHandle),
    RepositoryCommit(RepositoryCommitPreviewResumeHandle),
    SupportCancellation(SupportCancellationPreviewResumeHandle),
}

/// Durable normalized preview request, bound to its exact tool name.
/// Deliberately not `Deserialize`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct PreviewResumeHandle(PreviewResumeHandleKind);

impl JsonSchema for PreviewResumeHandle {
    fn schema_name() -> Cow<'static, str> {
        "PreviewResumeHandle".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<ArchivePreviewResumeHandle>(),
            generator.subschema_for::<CleanupPreviewResumeHandle>(),
            generator.subschema_for::<DeliveryCreatePreviewResumeHandle>(),
            generator.subschema_for::<DeliveryDeployPreviewResumeHandle>(),
            generator.subschema_for::<RoutineRepositoryUpdatePreviewResumeHandle>(),
            generator.subschema_for::<SupportPrerequisiteRepositoryUpdatePreviewResumeHandle>(),
            generator.subschema_for::<RepositoryCommitPreviewResumeHandle>(),
            generator.subschema_for::<SupportCancellationPreviewResumeHandle>(),
        ])
    }
}

wire_literal!(SupportPreflightHandleKind, "supportPreflight");
wire_literal!(CurrentSupportPreflightState, "current");
wire_literal!(ConsumedSupportPreflightState, "consumedByOriginalMerge");
wire_literal!(ReadySupportPreflightOutcome, "ready");

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SupportPreflightResumeBody {
    handle_kind: SupportPreflightHandleKind,
    support_gate_id: UnicaId,
    candidate_set_id: UnicaId,
    candidate_set_digest: Sha256Digest,
    support_graph_digest: Sha256Digest,
    observed_history_cursor: RepositoryHistoryCursor,
    relevant_baseline_digest: Sha256Digest,
    ordinary_result_artifact_id: UnicaId,
    comparison_id: UnicaId,
    support_gate_digest: Sha256Digest,
    support_recovery_distribution_set_digest: Sha256Digest,
    history_evidence: SupportGateHistoryEvidence,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CurrentSupportPreflightResumeHandle {
    #[serde(flatten)]
    body: SupportPreflightResumeBody,
    outcome: SupportPreflightOutcome,
    state: CurrentSupportPreflightState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ConsumedSupportPreflightResumeHandle {
    #[serde(flatten)]
    body: SupportPreflightResumeBody,
    outcome: ReadySupportPreflightOutcome,
    state: ConsumedSupportPreflightState,
    consumed_by_merge_receipt_id: UnicaId,
    authorized_post_merge_fingerprint: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
enum SupportPreflightResumeHandleKind {
    Current(CurrentSupportPreflightResumeHandle),
    Consumed(ConsumedSupportPreflightResumeHandle),
}

/// Current or consumed support preflight gate. Deliberately not `Deserialize`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct SupportPreflightResumeHandle(SupportPreflightResumeHandleKind);

impl JsonSchema for SupportPreflightResumeHandle {
    fn schema_name() -> Cow<'static, str> {
        "SupportPreflightResumeHandle".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<CurrentSupportPreflightResumeHandle>(),
            generator.subschema_for::<ConsumedSupportPreflightResumeHandle>(),
        ])
    }
}

wire_literal!(
    SupportActionAuthorizationHandleKind,
    "supportActionAuthorization"
);

/// Adds the resume discriminator to the already exact active/frozen support
/// authorization projection. Deliberately not `Deserialize`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SupportActionAuthorizationResumeHandle {
    handle_kind: SupportActionAuthorizationHandleKind,
    #[serde(flatten)]
    authorization: ActiveSupportActionResumeHandle,
}

impl SupportActionAuthorizationResumeHandle {
    pub(crate) const fn new(authorization: ActiveSupportActionResumeHandle) -> Self {
        Self {
            handle_kind: SupportActionAuthorizationHandleKind::Value,
            authorization,
        }
    }
}

impl JsonSchema for SupportActionAuthorizationResumeHandle {
    fn schema_name() -> Cow<'static, str> {
        "SupportActionAuthorizationResumeHandle".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        let mut schema = ActiveSupportActionResumeHandle::json_schema(generator);
        let branches = schema
            .as_object_mut()
            .and_then(|object| object.get_mut("oneOf"))
            .and_then(serde_json::Value::as_array_mut)
            .expect("active authorization schema is a oneOf");
        for branch in branches {
            let object = branch
                .as_object_mut()
                .expect("active authorization branch is an object schema");
            object
                .get_mut("properties")
                .and_then(serde_json::Value::as_object_mut)
                .expect("active authorization branch has properties")
                .insert(
                    "handleKind".to_owned(),
                    json_schema!({"type": "string", "const": "supportActionAuthorization"})
                        .to_value(),
                );
            object
                .get_mut("required")
                .and_then(serde_json::Value::as_array_mut)
                .expect("active authorization branch has required fields")
                .push(serde_json::Value::String("handleKind".to_owned()));
        }
        schema
    }
}

wire_literal!(SupportPrerequisiteHandleKind, "supportPrerequisite");
wire_literal!(ReservedOriginalStatusMode, "reservedOriginal");
wire_literal!(SeparateWorkingInfobaseStatusMode, "separateWorkingInfobase");

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SupportPrerequisiteResumeBody {
    handle_kind: SupportPrerequisiteHandleKind,
    receipt_id: UnicaId,
    prior_support_action_id: UnicaId,
    prior_support_gate_id: UnicaId,
    purpose: SupportActionPurpose,
    arming_receipt_id: UnicaId,
    arming_receipt_digest: Sha256Digest,
    repository_version: RepositoryVersion,
    authorized_transitions_digest: Sha256Digest,
    root_delta_digest: Sha256Digest,
    root_lock_proof_digest: Sha256Digest,
    history_from_cursor: RepositoryHistoryCursor,
    history_through_cursor: RepositoryHistoryCursor,
    history_partition_digest: Sha256Digest,
    selective_update_proof_digest: Sha256Digest,
    post_release_observed_history_cursor: RepositoryHistoryCursor,
    post_apply_history_partition_digest: Sha256Digest,
    #[serde(skip_serializing_if = "Option::is_none")]
    deferred_repository_advance_digest: Option<Sha256Digest>,
    resulting_phase: TaskPhase,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReservedOriginalSupportPrerequisiteResumeHandle {
    #[serde(flatten)]
    body: SupportPrerequisiteResumeBody,
    repository_actor: RepositoryActorIdentity,
    manual_target_mode: ReservedOriginalStatusMode,
    manual_actor_lock_inventory_proof_digest: Sha256Digest,
    reserved_original_terminalization_proof_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SeparateWorkingInfobaseSupportPrerequisiteResumeHandle {
    #[serde(flatten)]
    body: SupportPrerequisiteResumeBody,
    manual_target_mode: SeparateWorkingInfobaseStatusMode,
    manual_working_infobase_closure_proof_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
enum SupportPrerequisiteResumeHandleKind {
    ReservedOriginal(ReservedOriginalSupportPrerequisiteResumeHandle),
    SeparateWorkingInfobase(SeparateWorkingInfobaseSupportPrerequisiteResumeHandle),
}

/// Immutable prerequisite terminal receipt projection. Deliberately not
/// `Deserialize`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct SupportPrerequisiteResumeHandle(SupportPrerequisiteResumeHandleKind);

impl JsonSchema for SupportPrerequisiteResumeHandle {
    fn schema_name() -> Cow<'static, str> {
        "SupportPrerequisiteResumeHandle".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<ReservedOriginalSupportPrerequisiteResumeHandle>(),
            generator.subschema_for::<SeparateWorkingInfobaseSupportPrerequisiteResumeHandle>(),
        ])
    }
}

wire_literal!(SupportRecoveryHandleKind, "supportRecovery");
wire_literal!(RestoreThenReauthorizeDisposition, "restoreThenReauthorize");
wire_literal!(
    PreserveExternalAndReauthorizeDisposition,
    "preserveExternalAndReauthorize"
);
wire_literal!(RestoreThenAbandonDisposition, "restoreThenAbandon");
bool_literal!(SuccessfulIntegrationForbiddenLiteral, true);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SupportRecoveryResumeBody {
    handle_kind: SupportRecoveryHandleKind,
    receipt_id: UnicaId,
    prior_support_action_id: UnicaId,
    arming_receipt_id: UnicaId,
    arming_receipt_digest: Sha256Digest,
    history_from_cursor: RepositoryHistoryCursor,
    history_through_cursor: RepositoryHistoryCursor,
    post_release_observed_history_cursor: RepositoryHistoryCursor,
    post_release_history_partition_digest: Sha256Digest,
    support_version_observation_digest: Sha256Digest,
    support_recovery_finalization_plan_digest: Sha256Digest,
    support_recovery_guard_proof_digest: Sha256Digest,
    #[serde(skip_serializing_if = "Option::is_none")]
    deferred_repository_advance_digest: Option<Sha256Digest>,
    resulting_phase: TaskPhase,
}

macro_rules! support_recovery_leaf {
    ($name:ident, $disposition:ty, $mode:ty, {$($field:ident: $field_type:ty),* $(,)?}) => {
        #[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct $name {
            #[serde(flatten)]
            body: SupportRecoveryResumeBody,
            disposition: $disposition,
            manual_target_mode: $mode,
            $($field: $field_type,)*
        }
    };
}

support_recovery_leaf!(ReservedRestoreThenReauthorizeSupportRecoveryResumeHandle, RestoreThenReauthorizeDisposition, ReservedOriginalStatusMode, {
    reserved_original_terminalization_proof_digest: Sha256Digest
});
support_recovery_leaf!(SeparateRestoreThenReauthorizeSupportRecoveryResumeHandle, RestoreThenReauthorizeDisposition, SeparateWorkingInfobaseStatusMode, {
    manual_working_infobase_closure_proof_digest: Sha256Digest
});
support_recovery_leaf!(ReservedPreserveExternalSupportRecoveryResumeHandle, PreserveExternalAndReauthorizeDisposition, ReservedOriginalStatusMode, {
    reserved_original_terminalization_proof_digest: Sha256Digest
});
support_recovery_leaf!(SeparatePreserveExternalSupportRecoveryResumeHandle, PreserveExternalAndReauthorizeDisposition, SeparateWorkingInfobaseStatusMode, {
    manual_working_infobase_closure_proof_digest: Sha256Digest
});
support_recovery_leaf!(ReservedRestoreThenAbandonSupportRecoveryResumeHandle, RestoreThenAbandonDisposition, ReservedOriginalStatusMode, {
    successful_integration_forbidden: SuccessfulIntegrationForbiddenLiteral,
    reserved_original_terminalization_proof_digest: Sha256Digest
});
support_recovery_leaf!(SeparateRestoreThenAbandonSupportRecoveryResumeHandle, RestoreThenAbandonDisposition, SeparateWorkingInfobaseStatusMode, {
    successful_integration_forbidden: SuccessfulIntegrationForbiddenLiteral,
    manual_working_infobase_closure_proof_digest: Sha256Digest
});

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
enum SupportRecoveryResumeHandleKind {
    ReservedRestoreThenReauthorize(ReservedRestoreThenReauthorizeSupportRecoveryResumeHandle),
    SeparateRestoreThenReauthorize(SeparateRestoreThenReauthorizeSupportRecoveryResumeHandle),
    ReservedPreserveExternal(ReservedPreserveExternalSupportRecoveryResumeHandle),
    SeparatePreserveExternal(SeparatePreserveExternalSupportRecoveryResumeHandle),
    ReservedRestoreThenAbandon(ReservedRestoreThenAbandonSupportRecoveryResumeHandle),
    SeparateRestoreThenAbandon(SeparateRestoreThenAbandonSupportRecoveryResumeHandle),
}

/// Immutable support-recovery terminal receipt projection. Deliberately not
/// `Deserialize`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct SupportRecoveryResumeHandle(SupportRecoveryResumeHandleKind);

impl JsonSchema for SupportRecoveryResumeHandle {
    fn schema_name() -> Cow<'static, str> {
        "SupportRecoveryResumeHandle".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<ReservedRestoreThenReauthorizeSupportRecoveryResumeHandle>(),
            generator.subschema_for::<SeparateRestoreThenReauthorizeSupportRecoveryResumeHandle>(),
            generator.subschema_for::<ReservedPreserveExternalSupportRecoveryResumeHandle>(),
            generator.subschema_for::<SeparatePreserveExternalSupportRecoveryResumeHandle>(),
            generator.subschema_for::<ReservedRestoreThenAbandonSupportRecoveryResumeHandle>(),
            generator.subschema_for::<SeparateRestoreThenAbandonSupportRecoveryResumeHandle>(),
        ])
    }
}

/// Completed-only projection of the broader Task 10 attempt-progress state
/// machine. Deliberately not `Deserialize`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct CompletedPreArmCancellationProgress(
    PreArmCancellationFinalizationAttemptProgress,
);

impl CompletedPreArmCancellationProgress {
    pub(crate) fn new(
        progress: PreArmCancellationFinalizationAttemptProgress,
    ) -> Result<Self, StatusContractError> {
        (progress.state() == PreArmCancellationFinalizationAttemptState::Completed)
            .then_some(Self(progress))
            .ok_or(StatusContractError(
                "support cancellation status requires completed pre-arm progress",
            ))
    }
}

impl JsonSchema for CompletedPreArmCancellationProgress {
    fn schema_name() -> Cow<'static, str> {
        "CompletedPreArmCancellationProgress".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        let schema = PreArmCancellationFinalizationAttemptProgress::json_schema(generator);
        let completed = schema
            .as_object()
            .and_then(|object| object.get("oneOf"))
            .and_then(serde_json::Value::as_array)
            .and_then(|branches| {
                branches.iter().find(|branch| {
                    branch
                        .pointer("/properties/attemptState/const")
                        .and_then(serde_json::Value::as_str)
                        == Some("completed")
                })
            })
            .cloned()
            .expect("Task 10 progress schema contains a completed branch");
        serde_json::from_value(completed)
            .expect("Task 10 completed progress branch remains a valid JSON Schema")
    }
}

wire_literal!(SupportCancellationHandleKind, "supportCancellation");

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SupportCancellationResumeBody {
    handle_kind: SupportCancellationHandleKind,
    receipt_id: UnicaId,
    receipt_digest: Sha256Digest,
    prior_support_action_id: UnicaId,
    purpose: SupportActionPurpose,
    reason: Reason,
    root_lock_proof_digest: Sha256Digest,
    history_from_cursor: RepositoryHistoryCursor,
    history_through_cursor: RepositoryHistoryCursor,
    history_partition_digest: Sha256Digest,
    preserved_external_support_digest: Sha256Digest,
    selective_update_proof_digest: Sha256Digest,
    post_release_observed_history_cursor: RepositoryHistoryCursor,
    post_apply_history_partition_digest: Sha256Digest,
    #[serde(skip_serializing_if = "Option::is_none")]
    deferred_repository_advance_digest: Option<Sha256Digest>,
    resulting_phase: TaskPhase,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReservedCancellationModeProof {
    manual_target_mode: ReservedOriginalStatusMode,
    manual_actor_lock_inventory_proof_digest: Sha256Digest,
    reserved_original_terminalization_proof_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SeparateCancellationModeProof {
    manual_target_mode: SeparateWorkingInfobaseStatusMode,
    manual_working_infobase_closure_proof_digest: Sha256Digest,
}

macro_rules! support_cancellation_leaf {
    ($name:ident, $mode_proof:ty, {$($field:ident: $field_type:ty),* $(,)?}) => {
        #[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct $name {
            #[serde(flatten)]
            body: SupportCancellationResumeBody,
            #[serde(flatten)]
            mode_proof: $mode_proof,
            $($field: $field_type,)*
        }
    };
}

support_cancellation_leaf!(
    ReservedUnarmedSupportCancellationResumeHandle,
    ReservedCancellationModeProof,
    {}
);
support_cancellation_leaf!(
    SeparateUnarmedSupportCancellationResumeHandle,
    SeparateCancellationModeProof,
    {}
);
support_cancellation_leaf!(ReservedArmedSupportCancellationResumeHandle, ReservedCancellationModeProof, {
    arming_receipt_id: UnicaId,
    arming_receipt_digest: Sha256Digest
});
support_cancellation_leaf!(SeparateArmedSupportCancellationResumeHandle, SeparateCancellationModeProof, {
    arming_receipt_id: UnicaId,
    arming_receipt_digest: Sha256Digest
});
support_cancellation_leaf!(ReservedPreArmRecoverySupportCancellationResumeHandle, ReservedCancellationModeProof, {
    pre_arm_cancellation_effect_observation: PreArmCancellationEffectObservation,
    pre_arm_cancellation_finalization_plan: PreArmCancellationFinalizationPlan,
    pre_arm_cancellation_finalization_plan_digest: Sha256Digest,
    pre_arm_cancellation_receipt_plan_digest: Sha256Digest,
    pre_arm_cancellation_finalization_recheck_evidence: PreArmCancellationFinalizationRecheckEvidence,
    pre_arm_cancellation_completed_progress: CompletedPreArmCancellationProgress,
    pre_arm_cancellation_finalization_attempt_audit_digest: Sha256Digest,
    pre_arm_recovery_receipt_id: UnicaId,
    pre_arm_recovery_receipt_digest: Sha256Digest,
    recovery_receipt_digest: Sha256Digest
});
support_cancellation_leaf!(SeparatePreArmRecoverySupportCancellationResumeHandle, SeparateCancellationModeProof, {
    pre_arm_cancellation_effect_observation: PreArmCancellationEffectObservation,
    pre_arm_cancellation_finalization_plan: PreArmCancellationFinalizationPlan,
    pre_arm_cancellation_finalization_plan_digest: Sha256Digest,
    pre_arm_cancellation_receipt_plan_digest: Sha256Digest,
    pre_arm_cancellation_finalization_recheck_evidence: PreArmCancellationFinalizationRecheckEvidence,
    pre_arm_cancellation_completed_progress: CompletedPreArmCancellationProgress,
    pre_arm_cancellation_finalization_attempt_audit_digest: Sha256Digest,
    pre_arm_recovery_receipt_id: UnicaId,
    pre_arm_recovery_receipt_digest: Sha256Digest,
    recovery_receipt_digest: Sha256Digest
});

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
enum SupportCancellationResumeHandleKind {
    ReservedUnarmed(ReservedUnarmedSupportCancellationResumeHandle),
    SeparateUnarmed(SeparateUnarmedSupportCancellationResumeHandle),
    ReservedArmed(ReservedArmedSupportCancellationResumeHandle),
    SeparateArmed(SeparateArmedSupportCancellationResumeHandle),
    ReservedPreArmRecovery(ReservedPreArmRecoverySupportCancellationResumeHandle),
    SeparatePreArmRecovery(SeparatePreArmRecoverySupportCancellationResumeHandle),
}

/// Immutable cancellation terminal receipt projection. Deliberately not
/// `Deserialize`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct SupportCancellationResumeHandle(SupportCancellationResumeHandleKind);

impl JsonSchema for SupportCancellationResumeHandle {
    fn schema_name() -> Cow<'static, str> {
        "SupportCancellationResumeHandle".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<ReservedUnarmedSupportCancellationResumeHandle>(),
            generator.subschema_for::<SeparateUnarmedSupportCancellationResumeHandle>(),
            generator.subschema_for::<ReservedArmedSupportCancellationResumeHandle>(),
            generator.subschema_for::<SeparateArmedSupportCancellationResumeHandle>(),
            generator.subschema_for::<ReservedPreArmRecoverySupportCancellationResumeHandle>(),
            generator.subschema_for::<SeparatePreArmRecoverySupportCancellationResumeHandle>(),
        ])
    }
}

/// Complete tagged resume union. Every leaf carries its own exact
/// `handleKind`; the enum itself is deliberately not `Deserialize`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub(crate) enum ResumeHandle {
    Artifact(ArtifactResumeHandle),
    Workspace(WorkspaceResumeHandle),
    MergeResolutionWorkspace(MergeResolutionWorkspaceResumeHandle),
    Checkpoint(CheckpointResumeHandle),
    Comparison(ComparisonResumeHandle),
    SupportPreflight(Box<SupportPreflightResumeHandle>),
    SupportActionAuthorization(Box<SupportActionAuthorizationResumeHandle>),
    SupportPrerequisite(SupportPrerequisiteResumeHandle),
    SupportCancellation(Box<SupportCancellationResumeHandle>),
    SupportRecovery(SupportRecoveryResumeHandle),
    DeferredRepositoryAdvance(DeferredRepositoryAdvanceResumeHandle),
    MergeSession(MergeSessionResumeHandle),
    Decision(DecisionResumeHandle),
    ResolutionChangeReceipt(ResolutionChangeReceiptResumeHandle),
    Verification(VerificationResumeHandle),
    MergeApply(MergeApplyResumeHandle),
    LockPlan(LockPlanResumeHandle),
    LockSet(LockSetResumeHandle),
    Preview(PreviewResumeHandle),
    Recovery(RecoveryResumeHandle),
    Archive(ArchiveResumeHandle),
}

impl JsonSchema for ResumeHandle {
    fn schema_name() -> Cow<'static, str> {
        "ResumeHandle".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<ArtifactResumeHandle>(),
            generator.subschema_for::<WorkspaceResumeHandle>(),
            generator.subschema_for::<MergeResolutionWorkspaceResumeHandle>(),
            generator.subschema_for::<CheckpointResumeHandle>(),
            generator.subschema_for::<ComparisonResumeHandle>(),
            generator.subschema_for::<SupportPreflightResumeHandle>(),
            generator.subschema_for::<SupportActionAuthorizationResumeHandle>(),
            generator.subschema_for::<SupportPrerequisiteResumeHandle>(),
            generator.subschema_for::<SupportCancellationResumeHandle>(),
            generator.subschema_for::<SupportRecoveryResumeHandle>(),
            generator.subschema_for::<DeferredRepositoryAdvanceResumeHandle>(),
            generator.subschema_for::<MergeSessionResumeHandle>(),
            generator.subschema_for::<DecisionResumeHandle>(),
            generator.subschema_for::<ResolutionChangeReceiptResumeHandle>(),
            generator.subschema_for::<VerificationResumeHandle>(),
            generator.subschema_for::<MergeApplyResumeHandle>(),
            generator.subschema_for::<LockPlanResumeHandle>(),
            generator.subschema_for::<LockSetResumeHandle>(),
            generator.subschema_for::<PreviewResumeHandle>(),
            generator.subschema_for::<RecoveryResumeHandle>(),
            generator.subschema_for::<ArchiveResumeHandle>(),
        ])
    }
}

impl MergeSessionResumeHandle {
    fn identity(&self) -> &UnicaId {
        match &self.0 {
            MergeSessionResumeHandleKind::SupportedUpdate(value) => &value.session_id,
            MergeSessionResumeHandleKind::MainIntegration(value) => &value.session_id,
        }
    }
}

impl DecisionResumeHandle {
    fn identity(&self) -> &UnicaId {
        match &self.0 {
            DecisionResumeHandleKind::CurrentMergeConflict(value) => &value.body.decision_id,
            DecisionResumeHandleKind::SupersededMergeConflict(value) => &value.body.decision_id,
            DecisionResumeHandleKind::ReplacedMergeConflict(value) => &value.body.decision_id,
            DecisionResumeHandleKind::SupersededAndReplacedMergeConflict(value) => {
                &value.body.decision_id
            }
            DecisionResumeHandleKind::Adaptation(value) => &value.decision_id,
        }
    }

    fn is_terminal_audit(&self) -> bool {
        !matches!(self.0, DecisionResumeHandleKind::CurrentMergeConflict(_))
    }
}

impl ResolutionChangeReceiptResumeHandle {
    fn identity(&self) -> &UnicaId {
        match &self.0 {
            ResolutionChangeReceiptResumeHandleKind::Selectable(value) => {
                &value.body.change_receipt_id
            }
            ResolutionChangeReceiptResumeHandleKind::Invalidated(value) => {
                &value.body.change_receipt_id
            }
            ResolutionChangeReceiptResumeHandleKind::Superseded(value) => {
                &value.body.change_receipt_id
            }
            ResolutionChangeReceiptResumeHandleKind::Consumed(value) => {
                &value.body.change_receipt_id
            }
        }
    }

    fn is_terminal_audit(&self) -> bool {
        matches!(
            self.0,
            ResolutionChangeReceiptResumeHandleKind::Superseded(_)
                | ResolutionChangeReceiptResumeHandleKind::Consumed(_)
        )
    }
}

impl VerificationResumeHandle {
    fn identity(&self) -> &UnicaId {
        match &self.0 {
            VerificationResumeHandleKind::LocalCheckpointValid(value) => &value.verification_id,
            VerificationResumeHandleKind::LocalCheckpointInvalid(value) => &value.verification_id,
            VerificationResumeHandleKind::SynchronizedEquivalent(value) => &value.verification_id,
            VerificationResumeHandleKind::SynchronizedAdapted(value) => &value.verification_id,
            VerificationResumeHandleKind::SynchronizedUnexpected(value) => &value.verification_id,
            VerificationResumeHandleKind::SynchronizedInvalid(value) => &value.verification_id,
            VerificationResumeHandleKind::MainSandboxValid(value) => &value.verification_id,
            VerificationResumeHandleKind::MainSandboxInvalid(value) => &value.verification_id,
            VerificationResumeHandleKind::MainIntegrationValid(value) => &value.verification_id,
            VerificationResumeHandleKind::MainIntegrationInvalid(value) => &value.verification_id,
        }
    }
}

impl MergeApplyResumeHandle {
    fn identity(&self) -> &UnicaId {
        match &self.0 {
            MergeApplyResumeHandleKind::Task(value) => &value.merge_receipt_id,
            MergeApplyResumeHandleKind::Original(value) => &value.merge_receipt_id,
        }
    }
}

impl PreviewResumeHandle {
    fn identity(&self) -> &OperationId {
        match &self.0 {
            PreviewResumeHandleKind::Archive(value) => &value.preview_operation_id,
            PreviewResumeHandleKind::Cleanup(value) => &value.preview_operation_id,
            PreviewResumeHandleKind::DeliveryCreate(value) => &value.preview_operation_id,
            PreviewResumeHandleKind::DeliveryDeploy(value) => &value.preview_operation_id,
            PreviewResumeHandleKind::RoutineRepositoryUpdate(value) => &value.preview_operation_id,
            PreviewResumeHandleKind::SupportPrerequisiteRepositoryUpdate(value) => {
                &value.preview_operation_id
            }
            PreviewResumeHandleKind::RepositoryCommit(value) => &value.preview_operation_id,
            PreviewResumeHandleKind::SupportCancellation(value) => &value.preview_operation_id,
        }
    }
}

impl SupportPreflightResumeHandle {
    fn identity(&self) -> &UnicaId {
        match &self.0 {
            SupportPreflightResumeHandleKind::Current(value) => &value.body.support_gate_id,
            SupportPreflightResumeHandleKind::Consumed(value) => &value.body.support_gate_id,
        }
    }

    fn is_terminal_audit(&self) -> bool {
        matches!(self.0, SupportPreflightResumeHandleKind::Consumed(_))
    }
}

impl SupportPrerequisiteResumeHandle {
    fn identity(&self) -> &UnicaId {
        match &self.0 {
            SupportPrerequisiteResumeHandleKind::ReservedOriginal(value) => &value.body.receipt_id,
            SupportPrerequisiteResumeHandleKind::SeparateWorkingInfobase(value) => {
                &value.body.receipt_id
            }
        }
    }

    fn terminal_binding(&self) -> TerminalSupportReceiptBinding<'_> {
        let body = match &self.0 {
            SupportPrerequisiteResumeHandleKind::ReservedOriginal(value) => &value.body,
            SupportPrerequisiteResumeHandleKind::SeparateWorkingInfobase(value) => &value.body,
        };
        TerminalSupportReceiptBinding {
            receipt_id: &body.receipt_id,
            deferred_repository_advance_digest: body.deferred_repository_advance_digest.as_ref(),
        }
    }
}

impl SupportCancellationResumeHandle {
    fn identity(&self) -> &UnicaId {
        match &self.0 {
            SupportCancellationResumeHandleKind::ReservedUnarmed(value) => &value.body.receipt_id,
            SupportCancellationResumeHandleKind::SeparateUnarmed(value) => &value.body.receipt_id,
            SupportCancellationResumeHandleKind::ReservedArmed(value) => &value.body.receipt_id,
            SupportCancellationResumeHandleKind::SeparateArmed(value) => &value.body.receipt_id,
            SupportCancellationResumeHandleKind::ReservedPreArmRecovery(value) => {
                &value.body.receipt_id
            }
            SupportCancellationResumeHandleKind::SeparatePreArmRecovery(value) => {
                &value.body.receipt_id
            }
        }
    }

    fn terminal_binding(&self) -> TerminalSupportReceiptBinding<'_> {
        let body = match &self.0 {
            SupportCancellationResumeHandleKind::ReservedUnarmed(value) => &value.body,
            SupportCancellationResumeHandleKind::SeparateUnarmed(value) => &value.body,
            SupportCancellationResumeHandleKind::ReservedArmed(value) => &value.body,
            SupportCancellationResumeHandleKind::SeparateArmed(value) => &value.body,
            SupportCancellationResumeHandleKind::ReservedPreArmRecovery(value) => &value.body,
            SupportCancellationResumeHandleKind::SeparatePreArmRecovery(value) => &value.body,
        };
        TerminalSupportReceiptBinding {
            receipt_id: &body.receipt_id,
            deferred_repository_advance_digest: body.deferred_repository_advance_digest.as_ref(),
        }
    }
}

impl SupportRecoveryResumeHandle {
    fn identity(&self) -> &UnicaId {
        match &self.0 {
            SupportRecoveryResumeHandleKind::ReservedRestoreThenReauthorize(value) => {
                &value.body.receipt_id
            }
            SupportRecoveryResumeHandleKind::SeparateRestoreThenReauthorize(value) => {
                &value.body.receipt_id
            }
            SupportRecoveryResumeHandleKind::ReservedPreserveExternal(value) => {
                &value.body.receipt_id
            }
            SupportRecoveryResumeHandleKind::SeparatePreserveExternal(value) => {
                &value.body.receipt_id
            }
            SupportRecoveryResumeHandleKind::ReservedRestoreThenAbandon(value) => {
                &value.body.receipt_id
            }
            SupportRecoveryResumeHandleKind::SeparateRestoreThenAbandon(value) => {
                &value.body.receipt_id
            }
        }
    }

    fn terminal_binding(&self) -> TerminalSupportReceiptBinding<'_> {
        let body = match &self.0 {
            SupportRecoveryResumeHandleKind::ReservedRestoreThenReauthorize(value) => &value.body,
            SupportRecoveryResumeHandleKind::SeparateRestoreThenReauthorize(value) => &value.body,
            SupportRecoveryResumeHandleKind::ReservedPreserveExternal(value) => &value.body,
            SupportRecoveryResumeHandleKind::SeparatePreserveExternal(value) => &value.body,
            SupportRecoveryResumeHandleKind::ReservedRestoreThenAbandon(value) => &value.body,
            SupportRecoveryResumeHandleKind::SeparateRestoreThenAbandon(value) => &value.body,
        };
        TerminalSupportReceiptBinding {
            receipt_id: &body.receipt_id,
            deferred_repository_advance_digest: body.deferred_repository_advance_digest.as_ref(),
        }
    }
}

#[derive(Clone, Copy)]
struct TerminalSupportReceiptBinding<'a> {
    receipt_id: &'a UnicaId,
    deferred_repository_advance_digest: Option<&'a Sha256Digest>,
}

/// Non-wire proof that a later terminal producer selected one exact support
/// terminal as the latest one.  The authority deliberately has no production
/// raw-field constructor: Tasks 15-16 mint it together with their typed
/// terminal receipt/handle, while status only consumes and revalidates it.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct LatestTerminalSupportAuthority {
    receipt_id: UnicaId,
    deferred_repository_advance_digest: Option<Sha256Digest>,
}

impl LatestTerminalSupportAuthority {
    fn from_binding(binding: TerminalSupportReceiptBinding<'_>) -> Self {
        Self {
            receipt_id: binding.receipt_id.clone(),
            deferred_repository_advance_digest: binding.deferred_repository_advance_digest.cloned(),
        }
    }

    #[cfg(test)]
    fn from_handle_test_only(handle: &ResumeHandle) -> Result<Self, StatusContractError> {
        let binding = match handle {
            ResumeHandle::SupportPrerequisite(handle) => handle.terminal_binding(),
            ResumeHandle::SupportCancellation(handle) => handle.terminal_binding(),
            ResumeHandle::SupportRecovery(handle) => handle.terminal_binding(),
            _ => {
                return Err(StatusContractError(
                    "latest support terminal authority requires a terminal support handle",
                ));
            }
        };
        Ok(Self::from_binding(binding))
    }
}

impl ResumeHandle {
    fn identity(&self) -> (u8, &str) {
        match self {
            Self::Artifact(value) => (0, value.artifact_id.as_str()),
            Self::Workspace(value) => (1, value.task_workspace_id.as_str()),
            Self::MergeResolutionWorkspace(value) => (2, value.workspace_id.as_str()),
            Self::Checkpoint(value) => (3, value.checkpoint_id.as_str()),
            Self::Comparison(value) => (4, value.comparison_id.as_str()),
            Self::SupportPreflight(value) => (5, value.identity().as_str()),
            Self::SupportActionAuthorization(value) => {
                (6, value.authorization.support_action_id().as_str())
            }
            Self::SupportPrerequisite(value) => (7, value.identity().as_str()),
            Self::SupportCancellation(value) => (8, value.identity().as_str()),
            Self::SupportRecovery(value) => (9, value.identity().as_str()),
            Self::DeferredRepositoryAdvance(value) => {
                (10, value.advance.observation_digest().as_str())
            }
            Self::MergeSession(value) => (11, value.identity().as_str()),
            Self::Decision(value) => (12, value.identity().as_str()),
            Self::ResolutionChangeReceipt(value) => (13, value.identity().as_str()),
            Self::Verification(value) => (14, value.identity().as_str()),
            Self::MergeApply(value) => (15, value.identity().as_str()),
            Self::LockPlan(value) => (16, value.plan_id.as_str()),
            Self::LockSet(value) => (17, value.lock_set_id.as_str()),
            Self::Preview(value) => (18, value.identity().as_str()),
            Self::Recovery(value) => (19, value.prior_operation_id.as_str()),
            Self::Archive(value) => (20, value.archive_id.as_str()),
        }
    }
}

/// Canonical, duplicate-free status handle projection. Deliberately not
/// `Deserialize`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct ResumeHandles(Vec<ResumeHandle>);

impl ResumeHandles {
    pub(crate) fn new(values: Vec<ResumeHandle>) -> Result<Self, StatusContractError> {
        if values.len() > MAX_STATUS_ITEMS
            || values
                .windows(2)
                .any(|pair| pair[0].identity() >= pair[1].identity())
        {
            return Err(StatusContractError(
                "resume handles must be bounded, canonical, and unique by typed identity",
            ));
        }
        Ok(Self(values))
    }

    fn workspace_id(&self) -> Result<Option<&UnicaId>, StatusContractError> {
        let mut found = None;
        for handle in &self.0 {
            if let ResumeHandle::Workspace(handle) = handle {
                if found.replace(handle.task_workspace_id()).is_some() {
                    return Err(StatusContractError(
                        "status contains more than one task-workspace handle",
                    ));
                }
            }
        }
        Ok(found)
    }

    fn recovery_binding(
        &self,
    ) -> Result<Option<(&OperationId, &Sha256Digest)>, StatusContractError> {
        let mut found = None;
        for handle in &self.0 {
            if let ResumeHandle::Recovery(handle) = handle {
                if found
                    .replace((handle.prior_operation_id(), handle.recovery_digest()))
                    .is_some()
                {
                    return Err(StatusContractError(
                        "status contains more than one current recovery handle",
                    ));
                }
            }
        }
        Ok(found)
    }

    fn archive_binding(
        &self,
    ) -> Result<
        Option<(&UnicaId, &Sha256Digest, TaskArchiveOutcome, &Sha256Digest)>,
        StatusContractError,
    > {
        let mut found = None;
        for handle in &self.0 {
            if let ResumeHandle::Archive(handle) = handle {
                if found
                    .replace((
                        handle.archive_id(),
                        handle.sha256(),
                        handle.outcome(),
                        handle.retained_lineage_digest(),
                    ))
                    .is_some()
                {
                    return Err(StatusContractError(
                        "status contains more than one retained archive handle",
                    ));
                }
            }
        }
        Ok(found)
    }

    fn deferred_advance_digest(&self) -> Result<Option<&Sha256Digest>, StatusContractError> {
        let mut found = None;
        for handle in &self.0 {
            if let ResumeHandle::DeferredRepositoryAdvance(handle) = handle {
                if found.replace(handle.advance.observation_digest()).is_some() {
                    return Err(StatusContractError(
                        "status contains more than one current deferred repository advance",
                    ));
                }
            }
        }
        Ok(found)
    }

    fn terminal_support_binding(
        &self,
        receipt_id: &UnicaId,
    ) -> Result<TerminalSupportReceiptBinding<'_>, StatusContractError> {
        let mut found = None;
        for handle in &self.0 {
            let candidate = match handle {
                ResumeHandle::SupportPrerequisite(handle) => Some(handle.terminal_binding()),
                ResumeHandle::SupportCancellation(handle) => Some(handle.terminal_binding()),
                ResumeHandle::SupportRecovery(handle) => Some(handle.terminal_binding()),
                _ => None,
            };
            if let Some(candidate) = candidate.filter(|value| value.receipt_id == receipt_id) {
                if found.replace(candidate).is_some() {
                    return Err(StatusContractError(
                        "latest terminal support receipt ID is ambiguous across handles",
                    ));
                }
            }
        }
        found.ok_or(StatusContractError(
            "latest terminal support receipt has no retained handle",
        ))
    }

    fn has_terminal_support(&self) -> bool {
        self.0.iter().any(|handle| {
            matches!(
                handle,
                ResumeHandle::SupportPrerequisite(_)
                    | ResumeHandle::SupportCancellation(_)
                    | ResumeHandle::SupportRecovery(_)
            )
        })
    }

    fn has_terminal_mutable_handle(
        &self,
        workspace_is_retained: bool,
        recovery_is_current: bool,
    ) -> bool {
        self.0.iter().any(|handle| match handle {
            ResumeHandle::SupportPrerequisite(_)
            | ResumeHandle::SupportCancellation(_)
            | ResumeHandle::SupportRecovery(_)
            | ResumeHandle::Archive(_) => false,
            ResumeHandle::SupportPreflight(handle) => !handle.is_terminal_audit(),
            ResumeHandle::Decision(handle) => !handle.is_terminal_audit(),
            ResumeHandle::ResolutionChangeReceipt(handle) => !handle.is_terminal_audit(),
            ResumeHandle::Workspace(_) => !workspace_is_retained,
            ResumeHandle::Recovery(_) => !recovery_is_current,
            _ => true,
        })
    }
}

impl JsonSchema for ResumeHandles {
    fn schema_name() -> Cow<'static, str> {
        "ResumeHandles".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "array",
            "items": generator.subschema_for::<ResumeHandle>(),
            "minItems": 0,
            "maxItems": MAX_STATUS_ITEMS,
            "uniqueItems": true
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) enum CleanupResultPhase {
    CleanedSuccess,
    CleanedAbandoned,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
struct CanonicalOwnedTargets(Vec<OwnedTargetLocator>);

impl CanonicalOwnedTargets {
    fn new(values: Vec<OwnedTargetLocator>) -> Result<Self, StatusContractError> {
        if values.len() > MAX_STATUS_ITEMS || values.windows(2).any(|pair| pair[0] >= pair[1]) {
            return Err(StatusContractError(
                "cleanup owned targets must be bounded, canonical, and unique",
            ));
        }
        Ok(Self(values))
    }

    fn from_absences(values: &CompletedCleanupAbsences) -> Self {
        Self(
            values
                .0
                .iter()
                .map(|value| value.owned_target.clone())
                .collect(),
        )
    }

    fn as_slice(&self) -> &[OwnedTargetLocator] {
        &self.0
    }
}

impl JsonSchema for CanonicalOwnedTargets {
    fn schema_name() -> Cow<'static, str> {
        "CleanupOwnedTargets".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "array",
            "items": generator.subschema_for::<OwnedTargetLocator>(),
            "minItems": 0,
            "maxItems": MAX_STATUS_ITEMS,
            "uniqueItems": true
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
struct CanonicalAbsenceObservationDigests(Vec<Sha256Digest>);

impl CanonicalAbsenceObservationDigests {
    fn from_absences(values: &CompletedCleanupAbsences) -> Self {
        Self(
            values
                .0
                .iter()
                .map(|value| value.absence_observation_digest.clone())
                .collect(),
        )
    }

    fn as_slice(&self) -> &[Sha256Digest] {
        &self.0
    }
}

impl JsonSchema for CanonicalAbsenceObservationDigests {
    fn schema_name() -> Cow<'static, str> {
        "CleanupAbsentObservationDigests".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "array",
            "items": generator.subschema_for::<Sha256Digest>(),
            "minItems": 0,
            "maxItems": MAX_STATUS_ITEMS,
            "uniqueItems": true
        })
    }
}

/// One capability-validated target/absence pair. The wire receipt projects the
/// two arrays from this paired authority; callers can never sort or splice the
/// observation digests independently from their targets.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct CompletedCleanupAbsence {
    owned_target: OwnedTargetLocator,
    absence_observation_digest: Sha256Digest,
}

impl CompletedCleanupAbsence {
    pub(crate) fn from_finish_cleanup_observation(
        observation: FinishCleanupAbsenceObservation,
    ) -> Self {
        Self {
            owned_target: observation.owned_target().clone(),
            absence_observation_digest: observation.observation_digest().clone(),
        }
    }

    #[cfg(test)]
    fn test_only(
        owned_target: OwnedTargetLocator,
        absence_observation_digest: Sha256Digest,
    ) -> Self {
        Self {
            owned_target,
            absence_observation_digest,
        }
    }

    pub(crate) const fn owned_target(&self) -> &OwnedTargetLocator {
        &self.owned_target
    }

    pub(crate) const fn absence_observation_digest(&self) -> &Sha256Digest {
        &self.absence_observation_digest
    }
}

/// Canonically target-ordered cleanup completion evidence. Empty is a valid
/// direct-completion authority when the owned target set was already empty.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct CompletedCleanupAbsences(Vec<CompletedCleanupAbsence>);

impl CompletedCleanupAbsences {
    pub(crate) fn new(values: Vec<CompletedCleanupAbsence>) -> Result<Self, StatusContractError> {
        if values.len() > MAX_STATUS_ITEMS {
            return Err(StatusContractError(
                "cleanup absence pairs exceed the general collection bound",
            ));
        }
        let mut previous_target: Option<&OwnedTargetLocator> = None;
        let mut observation_digests = std::collections::BTreeSet::new();
        for value in &values {
            if previous_target.is_some_and(|previous| previous >= &value.owned_target)
                || !observation_digests.insert(value.absence_observation_digest.as_str())
            {
                return Err(StatusContractError(
                    "cleanup absence pairs must be target-ordered and observation-unique",
                ));
            }
            previous_target = Some(&value.owned_target);
        }
        Ok(Self(values))
    }

    pub(crate) fn as_slice(&self) -> &[CompletedCleanupAbsence] {
        &self.0
    }
}

#[derive(Debug, PartialEq, Eq)]
enum ApprovedCleanupAttemptLineage {
    DirectEmpty,
    Recovery {
        recovery_digest: Sha256Digest,
        finish_action_id: UnicaId,
        finish_action_digest: Sha256Digest,
    },
}

/// Linear approval for one cleanup execution attempt. It binds the approved
/// preview and marker to one archive/outcome, operation, quarantine, exact
/// target set, and (for recovery) the final action lineage.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ApprovedCleanupAttempt {
    operation_id: OperationId,
    archive_id: UnicaId,
    outcome: TaskArchiveOutcome,
    approved_preview_digest: Sha256Digest,
    marker_digest: Sha256Digest,
    quarantine_id: UnicaId,
    owned_targets: CanonicalOwnedTargets,
    lineage: ApprovedCleanupAttemptLineage,
}

impl ApprovedCleanupAttempt {
    #[cfg(test)]
    pub(crate) fn from_recovery_test_only(
        operation_id: OperationId,
        archive: &TaskArchiveStatus,
        approved_preview_digest: Sha256Digest,
        marker_digest: Sha256Digest,
        quarantine_id: UnicaId,
        owned_targets: Vec<OwnedTargetLocator>,
        recovery: RecoveryPlanStatus,
    ) -> Result<Self, StatusContractError> {
        let owned_targets = CanonicalOwnedTargets::new(owned_targets)?;
        if owned_targets.as_slice().is_empty() {
            return Err(StatusContractError(
                "cleanup recovery attempt requires a non-empty target set",
            ));
        }
        let binding = recovery
            .cleanup_binding()
            .map_err(|_| StatusContractError("cleanup attempt requires a cleanup recovery plan"))?;
        let expected_phase = match archive.outcome() {
            TaskArchiveOutcome::Success => TaskPhase::CleanedSuccess,
            TaskArchiveOutcome::Abandoned => TaskPhase::CleanedAbandoned,
        };
        if binding.prior_operation_id() != &operation_id
            || binding.archive_id() != archive.archive_id()
            || binding.planned_result_phase() != expected_phase
            || binding.quarantine_id() != &quarantine_id
            || binding.owned_targets() != owned_targets.as_slice()
        {
            return Err(StatusContractError(
                "cleanup attempt does not match its operation/archive/recovery/target lineage",
            ));
        }
        let recovery_digest = binding.recovery_digest().clone();
        let finish_action_id = binding.finish_action_id().clone();
        let finish_action_digest = binding.finish_action_digest().clone();
        drop(recovery);
        Ok(Self {
            operation_id,
            archive_id: archive.archive_id().clone(),
            outcome: archive.outcome(),
            approved_preview_digest,
            marker_digest,
            quarantine_id,
            owned_targets,
            lineage: ApprovedCleanupAttemptLineage::Recovery {
                recovery_digest,
                finish_action_id,
                finish_action_digest,
            },
        })
    }

    #[cfg(test)]
    pub(crate) fn direct_empty_test_only(
        operation_id: OperationId,
        archive: &TaskArchiveStatus,
        approved_preview_digest: Sha256Digest,
        marker_digest: Sha256Digest,
        quarantine_id: UnicaId,
        owned_targets: Vec<OwnedTargetLocator>,
    ) -> Result<Self, StatusContractError> {
        let owned_targets = CanonicalOwnedTargets::new(owned_targets)?;
        if !owned_targets.as_slice().is_empty() {
            return Err(StatusContractError(
                "direct cleanup completion requires an empty target set",
            ));
        }
        Ok(Self {
            operation_id,
            archive_id: archive.archive_id().clone(),
            outcome: archive.outcome(),
            approved_preview_digest,
            marker_digest,
            quarantine_id,
            owned_targets,
            lineage: ApprovedCleanupAttemptLineage::DirectEmpty,
        })
    }

    pub(crate) fn observe_absences(
        self,
        observations: FinishCleanupAbsenceObservations,
    ) -> Result<CompletedCleanupAttempt, StatusContractError> {
        let ApprovedCleanupAttemptLineage::Recovery {
            recovery_digest,
            finish_action_id,
            finish_action_digest,
        } = &self.lineage
        else {
            return Err(StatusContractError(
                "direct-empty cleanup cannot consume recovery observations",
            ));
        };
        if observations.prior_operation_id() != &self.operation_id
            || observations.archive_id() != &self.archive_id
            || observations.recovery_digest() != recovery_digest
            || observations.finish_action_id() != finish_action_id
            || observations.finish_action_digest() != finish_action_digest
            || observations.as_slice().len() != self.owned_targets.as_slice().len()
            || observations
                .as_slice()
                .iter()
                .zip(self.owned_targets.as_slice())
                .any(|(observation, target)| observation.owned_target() != target)
        {
            return Err(StatusContractError(
                "cleanup absence observations belong to another attempt or target set",
            ));
        }
        let completed_absences = CompletedCleanupAbsences::new(
            observations
                .into_observations()
                .into_iter()
                .map(CompletedCleanupAbsence::from_finish_cleanup_observation)
                .collect(),
        )?;
        Ok(CompletedCleanupAttempt::from_approved(
            self,
            completed_absences,
        ))
    }

    pub(crate) fn complete_direct_empty(
        self,
    ) -> Result<CompletedCleanupAttempt, StatusContractError> {
        if !matches!(self.lineage, ApprovedCleanupAttemptLineage::DirectEmpty)
            || !self.owned_targets.as_slice().is_empty()
        {
            return Err(StatusContractError(
                "only a paired-empty direct cleanup attempt completes without observations",
            ));
        }
        Ok(CompletedCleanupAttempt::from_approved(
            self,
            CompletedCleanupAbsences::new(Vec::new())?,
        ))
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct CompletedCleanupAttempt {
    operation_id: OperationId,
    archive_id: UnicaId,
    outcome: TaskArchiveOutcome,
    approved_preview_digest: Sha256Digest,
    marker_digest: Sha256Digest,
    quarantine_id: UnicaId,
    completed_absences: CompletedCleanupAbsences,
    recovery_digest: Option<Sha256Digest>,
    finish_action_id: Option<UnicaId>,
    finish_action_digest: Option<Sha256Digest>,
}

impl CompletedCleanupAttempt {
    fn from_approved(
        approved: ApprovedCleanupAttempt,
        completed_absences: CompletedCleanupAbsences,
    ) -> Self {
        let (recovery_digest, finish_action_id, finish_action_digest) = match approved.lineage {
            ApprovedCleanupAttemptLineage::DirectEmpty => (None, None, None),
            ApprovedCleanupAttemptLineage::Recovery {
                recovery_digest,
                finish_action_id,
                finish_action_digest,
            } => (
                Some(recovery_digest),
                Some(finish_action_id),
                Some(finish_action_digest),
            ),
        };
        Self {
            operation_id: approved.operation_id,
            archive_id: approved.archive_id,
            outcome: approved.outcome,
            approved_preview_digest: approved.approved_preview_digest,
            marker_digest: approved.marker_digest,
            quarantine_id: approved.quarantine_id,
            completed_absences,
            recovery_digest,
            finish_action_id,
            finish_action_digest,
        }
    }

    pub(crate) const fn operation_id(&self) -> &OperationId {
        &self.operation_id
    }

    pub(crate) const fn archive_id(&self) -> &UnicaId {
        &self.archive_id
    }

    pub(crate) const fn outcome(&self) -> TaskArchiveOutcome {
        self.outcome
    }

    pub(crate) const fn approved_preview_digest(&self) -> &Sha256Digest {
        &self.approved_preview_digest
    }

    pub(crate) const fn marker_digest(&self) -> &Sha256Digest {
        &self.marker_digest
    }

    pub(crate) const fn quarantine_id(&self) -> &UnicaId {
        &self.quarantine_id
    }

    pub(crate) fn owned_targets(&self) -> Vec<OwnedTargetLocator> {
        self.completed_absences
            .as_slice()
            .iter()
            .map(|value| value.owned_target().clone())
            .collect()
    }

    pub(crate) fn absent_observation_digests(&self) -> Vec<Sha256Digest> {
        self.completed_absences
            .as_slice()
            .iter()
            .map(|value| value.absence_observation_digest().clone())
            .collect()
    }

    pub(crate) const fn recovery_digest(&self) -> Option<&Sha256Digest> {
        self.recovery_digest.as_ref()
    }

    pub(crate) const fn finish_action_id(&self) -> Option<&UnicaId> {
        self.finish_action_id.as_ref()
    }

    pub(crate) const fn finish_action_digest(&self) -> Option<&Sha256Digest> {
        self.finish_action_digest.as_ref()
    }

    pub(crate) fn authorize_receipt(
        self,
        cleanup_receipt_id: UnicaId,
    ) -> Result<CleanupReceiptAuthority, StatusContractError> {
        let owned_targets = CanonicalOwnedTargets::from_absences(&self.completed_absences);
        let absent_observation_digests =
            CanonicalAbsenceObservationDigests::from_absences(&self.completed_absences);
        let common = CleanupReceiptAuthorityCommon {
            marker_digest: self.marker_digest,
            outcome: self.outcome,
            recovery_digest: self.recovery_digest,
            finish_action_id: self.finish_action_id,
            finish_action_digest: self.finish_action_digest,
        };
        let kind = match self.outcome {
            TaskArchiveOutcome::Success => {
                CleanupReceiptAuthorityKind::Success(SuccessCleanupReceiptDigestRecord {
                    cleanup_receipt_id,
                    operation_id: self.operation_id,
                    archive_id: self.archive_id,
                    approved_preview_digest: self.approved_preview_digest,
                    owned_targets,
                    quarantine_id: self.quarantine_id,
                    absent_observation_digests,
                    resulting_phase: CleanedSuccessTaskPhase::Value,
                })
            }
            TaskArchiveOutcome::Abandoned => {
                CleanupReceiptAuthorityKind::Abandoned(AbandonedCleanupReceiptDigestRecord {
                    cleanup_receipt_id,
                    operation_id: self.operation_id,
                    archive_id: self.archive_id,
                    approved_preview_digest: self.approved_preview_digest,
                    owned_targets,
                    quarantine_id: self.quarantine_id,
                    absent_observation_digests,
                    resulting_phase: CleanedAbandonedTaskPhase::Value,
                })
            }
        };
        Ok(CleanupReceiptAuthority { kind, common })
    }
}

macro_rules! cleanup_receipt_digest_record {
    ($name:ident, $phase:ty) => {
        #[derive(Debug, PartialEq, Eq, Serialize, JsonSchema)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        pub(crate) struct $name {
            cleanup_receipt_id: UnicaId,
            operation_id: OperationId,
            archive_id: UnicaId,
            approved_preview_digest: Sha256Digest,
            owned_targets: CanonicalOwnedTargets,
            quarantine_id: UnicaId,
            absent_observation_digests: CanonicalAbsenceObservationDigests,
            resulting_phase: $phase,
        }

        impl contract_digest_record_sealed::Sealed for $name {}
        impl ContractDigestRecord for $name {}
    };
}

cleanup_receipt_digest_record!(SuccessCleanupReceiptDigestRecord, CleanedSuccessTaskPhase);
cleanup_receipt_digest_record!(
    AbandonedCleanupReceiptDigestRecord,
    CleanedAbandonedTaskPhase
);

#[derive(Debug, PartialEq, Eq)]
enum CleanupReceiptAuthorityKind {
    Success(SuccessCleanupReceiptDigestRecord),
    Abandoned(AbandonedCleanupReceiptDigestRecord),
}

#[derive(Debug, PartialEq, Eq)]
struct CleanupReceiptAuthorityCommon {
    marker_digest: Sha256Digest,
    outcome: TaskArchiveOutcome,
    recovery_digest: Option<Sha256Digest>,
    finish_action_id: Option<UnicaId>,
    finish_action_digest: Option<Sha256Digest>,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct CleanupReceiptAuthority {
    kind: CleanupReceiptAuthorityKind,
    common: CleanupReceiptAuthorityCommon,
}

impl CleanupReceiptAuthority {
    pub(crate) const fn marker_digest(&self) -> &Sha256Digest {
        &self.common.marker_digest
    }

    pub(crate) const fn outcome(&self) -> TaskArchiveOutcome {
        self.common.outcome
    }

    pub(crate) const fn recovery_digest(&self) -> Option<&Sha256Digest> {
        self.common.recovery_digest.as_ref()
    }

    pub(crate) const fn finish_action_id(&self) -> Option<&UnicaId> {
        self.common.finish_action_id.as_ref()
    }

    pub(crate) const fn finish_action_digest(&self) -> Option<&Sha256Digest> {
        self.common.finish_action_digest.as_ref()
    }

    pub(crate) fn issue_success(self) -> Result<SuccessCleanupReceipt, StatusContractError> {
        let CleanupReceiptAuthorityKind::Success(record) = self.kind else {
            return Err(StatusContractError(
                "abandoned cleanup authority cannot issue a success receipt",
            ));
        };
        let receipt_digest = status_digest(&record, "cleanup receipt digest failed")?;
        Ok(SuccessCleanupReceipt {
            cleanup_receipt_id: record.cleanup_receipt_id,
            operation_id: record.operation_id,
            archive_id: record.archive_id,
            approved_preview_digest: record.approved_preview_digest,
            owned_targets: record.owned_targets,
            quarantine_id: record.quarantine_id,
            absent_observation_digests: record.absent_observation_digests,
            resulting_phase: record.resulting_phase,
            receipt_digest,
        })
    }

    pub(crate) fn issue_abandoned(self) -> Result<AbandonedCleanupReceipt, StatusContractError> {
        let CleanupReceiptAuthorityKind::Abandoned(record) = self.kind else {
            return Err(StatusContractError(
                "success cleanup authority cannot issue an abandoned receipt",
            ));
        };
        let receipt_digest = status_digest(&record, "cleanup receipt digest failed")?;
        Ok(AbandonedCleanupReceipt {
            cleanup_receipt_id: record.cleanup_receipt_id,
            operation_id: record.operation_id,
            archive_id: record.archive_id,
            approved_preview_digest: record.approved_preview_digest,
            owned_targets: record.owned_targets,
            quarantine_id: record.quarantine_id,
            absent_observation_digests: record.absent_observation_digests,
            resulting_phase: record.resulting_phase,
            receipt_digest,
        })
    }
}

macro_rules! cleanup_receipt_leaf {
    ($name:ident, $phase:ty, $result_phase:expr) => {
        #[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        pub(crate) struct $name {
            cleanup_receipt_id: UnicaId,
            operation_id: OperationId,
            archive_id: UnicaId,
            approved_preview_digest: Sha256Digest,
            owned_targets: CanonicalOwnedTargets,
            quarantine_id: UnicaId,
            absent_observation_digests: CanonicalAbsenceObservationDigests,
            resulting_phase: $phase,
            receipt_digest: Sha256Digest,
        }

        impl $name {
            pub(crate) const fn cleanup_receipt_id(&self) -> &UnicaId {
                &self.cleanup_receipt_id
            }

            pub(crate) const fn operation_id(&self) -> &OperationId {
                &self.operation_id
            }

            pub(crate) const fn archive_id(&self) -> &UnicaId {
                &self.archive_id
            }

            pub(crate) const fn approved_preview_digest(&self) -> &Sha256Digest {
                &self.approved_preview_digest
            }

            pub(crate) fn owned_targets(&self) -> &[OwnedTargetLocator] {
                self.owned_targets.as_slice()
            }

            pub(crate) const fn quarantine_id(&self) -> &UnicaId {
                &self.quarantine_id
            }

            pub(crate) fn absent_observation_digests(&self) -> &[Sha256Digest] {
                self.absent_observation_digests.as_slice()
            }

            pub(crate) const fn resulting_phase(&self) -> CleanupResultPhase {
                $result_phase
            }

            pub(crate) const fn receipt_digest(&self) -> &Sha256Digest {
                &self.receipt_digest
            }
        }
    };
}

cleanup_receipt_leaf!(
    SuccessCleanupReceipt,
    CleanedSuccessTaskPhase,
    CleanupResultPhase::CleanedSuccess
);
cleanup_receipt_leaf!(
    AbandonedCleanupReceipt,
    CleanedAbandonedTaskPhase,
    CleanupResultPhase::CleanedAbandoned
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
enum CleanupReceiptKind {
    Success(SuccessCleanupReceipt),
    Abandoned(AbandonedCleanupReceipt),
}

/// Immutable physical success-or-abandoned cleanup receipt. Deliberately not
/// `Deserialize`; only a consumed attempt authority can mint it in production.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct CleanupReceipt(CleanupReceiptKind);

impl JsonSchema for CleanupReceipt {
    fn schema_name() -> Cow<'static, str> {
        "CleanupReceipt".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<SuccessCleanupReceipt>(),
            generator.subschema_for::<AbandonedCleanupReceipt>(),
        ])
    }
}

#[cfg(test)]
struct CleanupReceiptTestParts {
    cleanup_receipt_id: UnicaId,
    operation_id: OperationId,
    archive_id: UnicaId,
    approved_preview_digest: Sha256Digest,
    owned_targets: Vec<OwnedTargetLocator>,
    quarantine_id: UnicaId,
    absent_observation_digests: Vec<Sha256Digest>,
    resulting_phase: CleanupResultPhase,
}

impl CleanupReceipt {
    pub(crate) fn new(authority: CleanupReceiptAuthority) -> Result<Self, StatusContractError> {
        Ok(Self(match authority.kind {
            CleanupReceiptAuthorityKind::Success(record) => {
                let receipt_digest = status_digest(&record, "cleanup receipt digest failed")?;
                CleanupReceiptKind::Success(SuccessCleanupReceipt {
                    cleanup_receipt_id: record.cleanup_receipt_id,
                    operation_id: record.operation_id,
                    archive_id: record.archive_id,
                    approved_preview_digest: record.approved_preview_digest,
                    owned_targets: record.owned_targets,
                    quarantine_id: record.quarantine_id,
                    absent_observation_digests: record.absent_observation_digests,
                    resulting_phase: record.resulting_phase,
                    receipt_digest,
                })
            }
            CleanupReceiptAuthorityKind::Abandoned(record) => {
                let receipt_digest = status_digest(&record, "cleanup receipt digest failed")?;
                CleanupReceiptKind::Abandoned(AbandonedCleanupReceipt {
                    cleanup_receipt_id: record.cleanup_receipt_id,
                    operation_id: record.operation_id,
                    archive_id: record.archive_id,
                    approved_preview_digest: record.approved_preview_digest,
                    owned_targets: record.owned_targets,
                    quarantine_id: record.quarantine_id,
                    absent_observation_digests: record.absent_observation_digests,
                    resulting_phase: record.resulting_phase,
                    receipt_digest,
                })
            }
        }))
    }

    #[cfg(test)]
    fn test_only(parts: CleanupReceiptTestParts) -> Result<Self, StatusContractError> {
        let CleanupReceiptTestParts {
            cleanup_receipt_id,
            operation_id,
            archive_id,
            approved_preview_digest,
            owned_targets,
            quarantine_id,
            absent_observation_digests,
            resulting_phase,
        } = parts;
        if owned_targets.len() != absent_observation_digests.len() {
            return Err(StatusContractError(
                "cleanup receipt requires one absence observation per owned target",
            ));
        }
        let completed_absences = CompletedCleanupAbsences::new(
            owned_targets
                .into_iter()
                .zip(absent_observation_digests)
                .map(|(target, digest)| CompletedCleanupAbsence::test_only(target, digest))
                .collect(),
        )?;
        let owned_targets = CanonicalOwnedTargets::from_absences(&completed_absences);
        let absent_observation_digests =
            CanonicalAbsenceObservationDigests::from_absences(&completed_absences);
        let common = CleanupReceiptAuthorityCommon {
            marker_digest: approved_preview_digest.clone(),
            outcome: match resulting_phase {
                CleanupResultPhase::CleanedSuccess => TaskArchiveOutcome::Success,
                CleanupResultPhase::CleanedAbandoned => TaskArchiveOutcome::Abandoned,
            },
            recovery_digest: None,
            finish_action_id: None,
            finish_action_digest: None,
        };
        let kind = match resulting_phase {
            CleanupResultPhase::CleanedSuccess => {
                CleanupReceiptAuthorityKind::Success(SuccessCleanupReceiptDigestRecord {
                    cleanup_receipt_id,
                    operation_id,
                    archive_id,
                    approved_preview_digest,
                    owned_targets,
                    quarantine_id,
                    absent_observation_digests,
                    resulting_phase: CleanedSuccessTaskPhase::Value,
                })
            }
            CleanupResultPhase::CleanedAbandoned => {
                CleanupReceiptAuthorityKind::Abandoned(AbandonedCleanupReceiptDigestRecord {
                    cleanup_receipt_id,
                    operation_id,
                    archive_id,
                    approved_preview_digest,
                    owned_targets,
                    quarantine_id,
                    absent_observation_digests,
                    resulting_phase: CleanedAbandonedTaskPhase::Value,
                })
            }
        };
        Self::new(CleanupReceiptAuthority { kind, common })
    }

    #[cfg(test)]
    fn validates_json(value: &serde_json::Value) -> bool {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct UncheckedCleanupReceipt {
            cleanup_receipt_id: UnicaId,
            operation_id: OperationId,
            archive_id: UnicaId,
            approved_preview_digest: Sha256Digest,
            owned_targets: Vec<OwnedTargetLocator>,
            quarantine_id: UnicaId,
            absent_observation_digests: Vec<Sha256Digest>,
            resulting_phase: CleanupResultPhase,
            receipt_digest: Sha256Digest,
        }

        let Ok(unchecked) = serde_json::from_value::<UncheckedCleanupReceipt>(value.clone()) else {
            return false;
        };
        let Ok(validated) = Self::test_only(CleanupReceiptTestParts {
            cleanup_receipt_id: unchecked.cleanup_receipt_id,
            operation_id: unchecked.operation_id,
            archive_id: unchecked.archive_id,
            approved_preview_digest: unchecked.approved_preview_digest,
            owned_targets: unchecked.owned_targets,
            quarantine_id: unchecked.quarantine_id,
            absent_observation_digests: unchecked.absent_observation_digests,
            resulting_phase: unchecked.resulting_phase,
        }) else {
            return false;
        };
        validated.receipt_digest() == &unchecked.receipt_digest
    }

    pub(crate) const fn cleanup_receipt_id(&self) -> &UnicaId {
        match &self.0 {
            CleanupReceiptKind::Success(value) => &value.cleanup_receipt_id,
            CleanupReceiptKind::Abandoned(value) => &value.cleanup_receipt_id,
        }
    }

    pub(crate) const fn operation_id(&self) -> &OperationId {
        match &self.0 {
            CleanupReceiptKind::Success(value) => &value.operation_id,
            CleanupReceiptKind::Abandoned(value) => &value.operation_id,
        }
    }

    pub(crate) const fn archive_id(&self) -> &UnicaId {
        match &self.0 {
            CleanupReceiptKind::Success(value) => &value.archive_id,
            CleanupReceiptKind::Abandoned(value) => &value.archive_id,
        }
    }

    pub(crate) const fn approved_preview_digest(&self) -> &Sha256Digest {
        match &self.0 {
            CleanupReceiptKind::Success(value) => &value.approved_preview_digest,
            CleanupReceiptKind::Abandoned(value) => &value.approved_preview_digest,
        }
    }

    pub(crate) fn owned_targets(&self) -> &[OwnedTargetLocator] {
        match &self.0 {
            CleanupReceiptKind::Success(value) => value.owned_targets.as_slice(),
            CleanupReceiptKind::Abandoned(value) => value.owned_targets.as_slice(),
        }
    }

    pub(crate) fn absent_observation_digests(&self) -> &[Sha256Digest] {
        match &self.0 {
            CleanupReceiptKind::Success(value) => value.absent_observation_digests.as_slice(),
            CleanupReceiptKind::Abandoned(value) => value.absent_observation_digests.as_slice(),
        }
    }

    pub(crate) const fn quarantine_id(&self) -> &UnicaId {
        match &self.0 {
            CleanupReceiptKind::Success(value) => &value.quarantine_id,
            CleanupReceiptKind::Abandoned(value) => &value.quarantine_id,
        }
    }

    pub(crate) const fn resulting_phase(&self) -> CleanupResultPhase {
        match &self.0 {
            CleanupReceiptKind::Success(_) => CleanupResultPhase::CleanedSuccess,
            CleanupReceiptKind::Abandoned(_) => CleanupResultPhase::CleanedAbandoned,
        }
    }

    pub(crate) const fn receipt_digest(&self) -> &Sha256Digest {
        match &self.0 {
            CleanupReceiptKind::Success(value) => &value.receipt_digest,
            CleanupReceiptKind::Abandoned(value) => &value.receipt_digest,
        }
    }
}

bool_literal!(ExistingTaskLiteral, true);
wire_literal!(ArchivedSuccessTaskPhase, "archivedSuccess");
wire_literal!(ArchivedAbandonedTaskPhase, "archivedAbandoned");
wire_literal!(CleanedSuccessTaskPhase, "cleanedSuccess");
wire_literal!(CleanedAbandonedTaskPhase, "cleanedAbandoned");
wire_literal!(SuccessArchiveOutcome, "success");
wire_literal!(AbandonedArchiveOutcome, "abandoned");

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) enum PreWorkspaceTaskStatusPhase {
    Created,
    PreflightPassed,
}

impl TryFrom<TaskPhase> for PreWorkspaceTaskStatusPhase {
    type Error = StatusContractError;

    fn try_from(value: TaskPhase) -> Result<Self, Self::Error> {
        match value {
            TaskPhase::Created => Ok(Self::Created),
            TaskPhase::PreflightPassed => Ok(Self::PreflightPassed),
            _ => Err(StatusContractError(
                "pre-workspace status requires created or preflightPassed phase",
            )),
        }
    }
}

impl From<PreWorkspaceTaskStatusPhase> for TaskPhase {
    fn from(value: PreWorkspaceTaskStatusPhase) -> Self {
        match value {
            PreWorkspaceTaskStatusPhase::Created => Self::Created,
            PreWorkspaceTaskStatusPhase::PreflightPassed => Self::PreflightPassed,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) enum WorkspaceTaskStatusPhase {
    BaselineReady,
    Developing,
    LocalVerified,
    SynchronizationPrepared,
    SynchronizationConflicts,
    Synchronized,
    IntegrationPlanned,
    AcquiringLocks,
    Locked,
    MainMerged,
    MainValidated,
    Committing,
    CommittedAndUnlocked,
    BlockedByForeignLock,
    StaleRelevantBaseline,
    LockPlanExpansionRequired,
    StaleSupportPreflight,
    UnexpectedDelta,
    ValidationFailed,
    AbandonmentReady,
}

impl TryFrom<TaskPhase> for WorkspaceTaskStatusPhase {
    type Error = StatusContractError;

    fn try_from(value: TaskPhase) -> Result<Self, Self::Error> {
        match value {
            TaskPhase::BaselineReady => Ok(Self::BaselineReady),
            TaskPhase::Developing => Ok(Self::Developing),
            TaskPhase::LocalVerified => Ok(Self::LocalVerified),
            TaskPhase::SynchronizationPrepared => Ok(Self::SynchronizationPrepared),
            TaskPhase::SynchronizationConflicts => Ok(Self::SynchronizationConflicts),
            TaskPhase::Synchronized => Ok(Self::Synchronized),
            TaskPhase::IntegrationPlanned => Ok(Self::IntegrationPlanned),
            TaskPhase::AcquiringLocks => Ok(Self::AcquiringLocks),
            TaskPhase::Locked => Ok(Self::Locked),
            TaskPhase::MainMerged => Ok(Self::MainMerged),
            TaskPhase::MainValidated => Ok(Self::MainValidated),
            TaskPhase::Committing => Ok(Self::Committing),
            TaskPhase::CommittedAndUnlocked => Ok(Self::CommittedAndUnlocked),
            TaskPhase::BlockedByForeignLock => Ok(Self::BlockedByForeignLock),
            TaskPhase::StaleRelevantBaseline => Ok(Self::StaleRelevantBaseline),
            TaskPhase::LockPlanExpansionRequired => Ok(Self::LockPlanExpansionRequired),
            TaskPhase::StaleSupportPreflight => Ok(Self::StaleSupportPreflight),
            TaskPhase::UnexpectedDelta => Ok(Self::UnexpectedDelta),
            TaskPhase::ValidationFailed => Ok(Self::ValidationFailed),
            TaskPhase::AbandonmentReady => Ok(Self::AbandonmentReady),
            _ => Err(StatusContractError(
                "workspace status phase has a different physical presence branch",
            )),
        }
    }
}

impl From<WorkspaceTaskStatusPhase> for TaskPhase {
    fn from(value: WorkspaceTaskStatusPhase) -> Self {
        match value {
            WorkspaceTaskStatusPhase::BaselineReady => Self::BaselineReady,
            WorkspaceTaskStatusPhase::Developing => Self::Developing,
            WorkspaceTaskStatusPhase::LocalVerified => Self::LocalVerified,
            WorkspaceTaskStatusPhase::SynchronizationPrepared => Self::SynchronizationPrepared,
            WorkspaceTaskStatusPhase::SynchronizationConflicts => Self::SynchronizationConflicts,
            WorkspaceTaskStatusPhase::Synchronized => Self::Synchronized,
            WorkspaceTaskStatusPhase::IntegrationPlanned => Self::IntegrationPlanned,
            WorkspaceTaskStatusPhase::AcquiringLocks => Self::AcquiringLocks,
            WorkspaceTaskStatusPhase::Locked => Self::Locked,
            WorkspaceTaskStatusPhase::MainMerged => Self::MainMerged,
            WorkspaceTaskStatusPhase::MainValidated => Self::MainValidated,
            WorkspaceTaskStatusPhase::Committing => Self::Committing,
            WorkspaceTaskStatusPhase::CommittedAndUnlocked => Self::CommittedAndUnlocked,
            WorkspaceTaskStatusPhase::BlockedByForeignLock => Self::BlockedByForeignLock,
            WorkspaceTaskStatusPhase::StaleRelevantBaseline => Self::StaleRelevantBaseline,
            WorkspaceTaskStatusPhase::LockPlanExpansionRequired => Self::LockPlanExpansionRequired,
            WorkspaceTaskStatusPhase::StaleSupportPreflight => Self::StaleSupportPreflight,
            WorkspaceTaskStatusPhase::UnexpectedDelta => Self::UnexpectedDelta,
            WorkspaceTaskStatusPhase::ValidationFailed => Self::ValidationFailed,
            WorkspaceTaskStatusPhase::AbandonmentReady => Self::AbandonmentReady,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) enum RecoveryTaskStatusPhase {
    CommitBlocked,
    RecoveryRequired,
    CommittedUnverified,
}

impl TryFrom<TaskPhase> for RecoveryTaskStatusPhase {
    type Error = StatusContractError;

    fn try_from(value: TaskPhase) -> Result<Self, Self::Error> {
        match value {
            TaskPhase::CommitBlocked => Ok(Self::CommitBlocked),
            TaskPhase::RecoveryRequired => Ok(Self::RecoveryRequired),
            TaskPhase::CommittedUnverified => Ok(Self::CommittedUnverified),
            _ => Err(StatusContractError(
                "recovery status requires a recovery-bearing phase",
            )),
        }
    }
}

impl From<RecoveryTaskStatusPhase> for TaskPhase {
    fn from(value: RecoveryTaskStatusPhase) -> Self {
        match value {
            RecoveryTaskStatusPhase::CommitBlocked => Self::CommitBlocked,
            RecoveryTaskStatusPhase::RecoveryRequired => Self::RecoveryRequired,
            RecoveryTaskStatusPhase::CommittedUnverified => Self::CommittedUnverified,
        }
    }
}

/// Fields whose own values are already validated by their closed projection
/// types. Phase-dependent presence is added only by `ExistingTaskStatusData`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct ExistingTaskStatusCommon {
    exists: ExistingTaskLiteral,
    instance_id: UnicaId,
    #[serde(skip_serializing_if = "Option::is_none")]
    active_operation: Option<ActiveOperationStatus>,
    pending_decisions: PendingDecisionStatuses,
    anchors: TaskAnchorStatuses,
    owned_locks: OwnedLockStatuses,
    validation_gates: ValidationGateStatuses,
    artifact_hashes: ArtifactHashStatuses,
    resume_handles: ResumeHandles,
    recent_operations: RecentOperations,
    #[serde(skip_serializing_if = "Option::is_none")]
    latest_deferred_advance_consumption: Option<DeferredRepositoryAdvanceConsumptionReceipt>,
    cleanup_eligibility: CleanupEligibilityStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExistingTaskStatusCollections {
    pending_decisions: PendingDecisionStatuses,
    anchors: TaskAnchorStatuses,
    owned_locks: OwnedLockStatuses,
    validation_gates: ValidationGateStatuses,
    artifact_hashes: ArtifactHashStatuses,
    resume_handles: ResumeHandles,
    recent_operations: RecentOperations,
}

impl ExistingTaskStatusCollections {
    pub(crate) const fn new(
        pending_decisions: PendingDecisionStatuses,
        anchors: TaskAnchorStatuses,
        owned_locks: OwnedLockStatuses,
        validation_gates: ValidationGateStatuses,
        artifact_hashes: ArtifactHashStatuses,
        resume_handles: ResumeHandles,
        recent_operations: RecentOperations,
    ) -> Self {
        Self {
            pending_decisions,
            anchors,
            owned_locks,
            validation_gates,
            artifact_hashes,
            resume_handles,
            recent_operations,
        }
    }
}

/// Non-wire pointer to the authoritative latest support terminal. It lets the
/// aggregate distinguish retained historical receipts from the one that owns
/// the current deferred-advance XOR state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExistingTaskDeferredState {
    latest_terminal_support_receipt_id: Option<UnicaId>,
    latest_terminal_deferred_repository_advance_digest: Option<Sha256Digest>,
    latest_consumption: Option<DeferredRepositoryAdvanceConsumptionReceipt>,
}

impl ExistingTaskDeferredState {
    /// Production-safe empty branch.  It is derived from the exact handle set,
    /// so a caller cannot hide a terminal receipt or deferred advance behind a
    /// scalar "none" claim.
    pub(crate) fn without_terminal_support(
        handles: &ResumeHandles,
    ) -> Result<Self, StatusContractError> {
        let state = Self {
            latest_terminal_support_receipt_id: None,
            latest_terminal_deferred_repository_advance_digest: None,
            latest_consumption: None,
        };
        state.validate(handles)?;
        Ok(state)
    }

    /// Current-handle branch for an authority minted together with the exact
    /// latest terminal receipt.  A deferred advance, when named by that
    /// terminal, must still be present in `handles`.
    pub(crate) fn latest_terminal_current(
        handles: &ResumeHandles,
        latest: LatestTerminalSupportAuthority,
    ) -> Result<Self, StatusContractError> {
        let state = Self {
            latest_terminal_support_receipt_id: Some(latest.receipt_id),
            latest_terminal_deferred_repository_advance_digest: latest
                .deferred_repository_advance_digest,
            latest_consumption: None,
        };
        state.validate(handles)?;
        Ok(state)
    }

    /// Consumed branch for the same sealed latest-terminal authority.  The
    /// typed consumption receipt must replace, rather than accompany, the
    /// terminal's exact deferred-advance handle.
    pub(crate) fn latest_terminal_consumed(
        handles: &ResumeHandles,
        latest: LatestTerminalSupportAuthority,
        consumption: DeferredRepositoryAdvanceConsumptionReceipt,
    ) -> Result<Self, StatusContractError> {
        let state = Self {
            latest_terminal_support_receipt_id: Some(latest.receipt_id),
            latest_terminal_deferred_repository_advance_digest: latest
                .deferred_repository_advance_digest,
            latest_consumption: Some(consumption),
        };
        state.validate(handles)?;
        Ok(state)
    }

    #[cfg(test)]
    pub(crate) const fn no_terminal_support() -> Self {
        Self {
            latest_terminal_support_receipt_id: None,
            latest_terminal_deferred_repository_advance_digest: None,
            latest_consumption: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn latest_terminal(
        receipt_id: UnicaId,
        latest_consumption: Option<DeferredRepositoryAdvanceConsumptionReceipt>,
    ) -> Self {
        let latest_terminal_deferred_repository_advance_digest = latest_consumption
            .as_ref()
            .map(|receipt| receipt.advance_observation_digest().clone());
        Self {
            latest_terminal_support_receipt_id: Some(receipt_id),
            latest_terminal_deferred_repository_advance_digest,
            latest_consumption,
        }
    }

    fn validate(&self, handles: &ResumeHandles) -> Result<(), StatusContractError> {
        let current_advance = handles.deferred_advance_digest()?;
        let Some(latest_receipt_id) = &self.latest_terminal_support_receipt_id else {
            if self
                .latest_terminal_deferred_repository_advance_digest
                .is_some()
                || handles.has_terminal_support()
                || current_advance.is_some()
                || self.latest_consumption.is_some()
            {
                return Err(StatusContractError(
                    "support terminal/deferred state requires its authoritative latest receipt",
                ));
            }
            return Ok(());
        };
        let terminal = handles.terminal_support_binding(latest_receipt_id)?;
        if terminal.deferred_repository_advance_digest
            != self
                .latest_terminal_deferred_repository_advance_digest
                .as_ref()
        {
            return Err(StatusContractError(
                "latest support terminal authority differs from its retained handle",
            ));
        }
        match self
            .latest_terminal_deferred_repository_advance_digest
            .as_ref()
        {
            None => {
                if current_advance.is_some() || self.latest_consumption.is_some() {
                    return Err(StatusContractError(
                        "latest support terminal has no deferred repository advance",
                    ));
                }
            }
            Some(expected_digest) => match (current_advance, &self.latest_consumption) {
                (Some(current_digest), None) if current_digest == expected_digest => {}
                (None, Some(consumption))
                    if consumption.terminal_receipt_id() == terminal.receipt_id
                        && consumption.advance_observation_digest() == expected_digest => {}
                _ => {
                    return Err(StatusContractError(
                        "deferred repository advance must be exactly current or exactly consumed",
                    ));
                }
            },
        }
        Ok(())
    }
}

/// Sealed aggregate projection used to mint exactly one physical status leaf.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExistingTaskStatusAuthority {
    common: ExistingTaskStatusCommon,
}

impl ExistingTaskStatusAuthority {
    pub(crate) fn new(
        instance_id: UnicaId,
        active_operation: Option<ActiveOperationStatus>,
        collections: ExistingTaskStatusCollections,
        deferred_state: ExistingTaskDeferredState,
        cleanup_eligibility: CleanupEligibilityStatus,
    ) -> Result<Self, StatusContractError> {
        deferred_state.validate(&collections.resume_handles)?;
        Ok(Self {
            common: ExistingTaskStatusCommon {
                exists: ExistingTaskLiteral,
                instance_id,
                active_operation,
                pending_decisions: collections.pending_decisions,
                anchors: collections.anchors,
                owned_locks: collections.owned_locks,
                validation_gates: collections.validation_gates,
                artifact_hashes: collections.artifact_hashes,
                resume_handles: collections.resume_handles,
                recent_operations: collections.recent_operations,
                latest_deferred_advance_consumption: deferred_state.latest_consumption,
                cleanup_eligibility,
            },
        })
    }

    fn require_no_archive(&self) -> Result<(), StatusContractError> {
        if self.common.cleanup_eligibility.archive_id().is_some()
            || self.common.resume_handles.archive_binding()?.is_some()
        {
            return Err(StatusContractError(
                "pre-archive status cannot carry archived cleanup eligibility",
            ));
        }
        Ok(())
    }

    fn require_archive(&self, archive_id: &UnicaId) -> Result<(), StatusContractError> {
        if self.common.cleanup_eligibility.archive_id() != Some(archive_id) {
            return Err(StatusContractError(
                "archived status cleanup eligibility must bind the same archive",
            ));
        }
        let Some((handle_archive_id, handle_sha256, handle_outcome, handle_lineage_digest)) =
            self.common.resume_handles.archive_binding()?
        else {
            return Err(StatusContractError(
                "archived status requires its retained archive handle",
            ));
        };
        if handle_archive_id != archive_id {
            return Err(StatusContractError(
                "archive handle ID disagrees with archived status",
            ));
        }
        let _ = (handle_sha256, handle_outcome, handle_lineage_digest);
        Ok(())
    }

    fn require_exact_archive(
        &self,
        archive: &TaskArchiveStatus,
    ) -> Result<(), StatusContractError> {
        self.require_archive(archive.archive_id())?;
        let Some((_, handle_sha256, handle_outcome, handle_lineage_digest)) =
            self.common.resume_handles.archive_binding()?
        else {
            unreachable!("require_archive already proved an archive handle");
        };
        if handle_sha256 != archive.sha256()
            || handle_outcome != archive.outcome()
            || handle_lineage_digest != archive.retained_lineage_digest()
        {
            return Err(StatusContractError(
                "archive handle digest, outcome, or lineage disagrees with the retained archive",
            ));
        }
        Ok(())
    }

    fn require_no_workspace(&self) -> Result<(), StatusContractError> {
        if self.common.resume_handles.workspace_id()?.is_some() {
            return Err(StatusContractError(
                "status branch forbids a task-workspace handle",
            ));
        }
        Ok(())
    }

    fn require_workspace(&self, task_workspace_id: &UnicaId) -> Result<(), StatusContractError> {
        if self.common.resume_handles.workspace_id()? != Some(task_workspace_id) {
            return Err(StatusContractError(
                "taskWorkspaceId must equal the sole current workspace handle",
            ));
        }
        Ok(())
    }

    fn require_no_recovery(&self) -> Result<(), StatusContractError> {
        if self.common.resume_handles.recovery_binding()?.is_some()
            || self
                .common
                .active_operation
                .as_ref()
                .and_then(ActiveOperationStatus::effect_unknown_recovery_binding)
                .is_some()
        {
            return Err(StatusContractError(
                "non-recovery status cannot retain current recovery authority",
            ));
        }
        Ok(())
    }

    fn require_recovery(&self, recovery: &RecoveryPlanStatus) -> Result<(), StatusContractError> {
        let expected = (recovery.prior_operation_id(), recovery.recovery_digest());
        if self.common.resume_handles.recovery_binding()? != Some(expected) {
            return Err(StatusContractError(
                "recovery status requires an exact prior-operation/digest resume handle",
            ));
        }
        if let Some(active_binding) = self
            .common
            .active_operation
            .as_ref()
            .and_then(ActiveOperationStatus::effect_unknown_recovery_binding)
        {
            if active_binding != expected {
                return Err(StatusContractError(
                    "effect-unknown active operation must bind the current recovery plan",
                ));
            }
        }
        Ok(())
    }

    fn require_terminal_quiescence(
        &self,
        workspace_is_retained: bool,
        recovery_is_current: bool,
    ) -> Result<(), StatusContractError> {
        if (!recovery_is_current && self.common.active_operation.is_some())
            || !self.common.pending_decisions.is_empty()
            || !self.common.owned_locks.is_empty()
            || self
                .common
                .resume_handles
                .has_terminal_mutable_handle(workspace_is_retained, recovery_is_current)
        {
            return Err(StatusContractError(
                "terminal archive state cannot retain active operations, decisions, locks, or mutable handles",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct ExistingTaskWithoutArchiveCommonSchema {
    exists: ExistingTaskLiteral,
    instance_id: UnicaId,
    #[serde(skip_serializing_if = "Option::is_none")]
    active_operation: Option<ActiveOperationStatus>,
    pending_decisions: PendingDecisionStatuses,
    anchors: TaskAnchorStatuses,
    owned_locks: OwnedLockStatuses,
    validation_gates: ValidationGateStatuses,
    artifact_hashes: ArtifactHashStatuses,
    resume_handles: ResumeHandles,
    recent_operations: RecentOperations,
    #[serde(skip_serializing_if = "Option::is_none")]
    latest_deferred_advance_consumption: Option<DeferredRepositoryAdvanceConsumptionReceipt>,
    cleanup_eligibility: UnarchivedIneligibleCleanupStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ArchivedCleanupEligibilitySchema {
    Eligible(EligibleCleanupStatus),
    Ineligible(ArchivedIneligibleCleanupStatus),
}

impl JsonSchema for ArchivedCleanupEligibilitySchema {
    fn schema_name() -> Cow<'static, str> {
        "ArchivedCleanupEligibilityStatus".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<EligibleCleanupStatus>(),
            generator.subschema_for::<ArchivedIneligibleCleanupStatus>(),
        ])
    }
}

#[derive(Debug, Clone, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct ExistingTaskWithArchiveCommonSchema {
    exists: ExistingTaskLiteral,
    instance_id: UnicaId,
    #[serde(skip_serializing_if = "Option::is_none")]
    active_operation: Option<ActiveOperationStatus>,
    pending_decisions: PendingDecisionStatuses,
    anchors: TaskAnchorStatuses,
    owned_locks: OwnedLockStatuses,
    validation_gates: ValidationGateStatuses,
    artifact_hashes: ArtifactHashStatuses,
    resume_handles: ResumeHandles,
    recent_operations: RecentOperations,
    #[serde(skip_serializing_if = "Option::is_none")]
    latest_deferred_advance_consumption: Option<DeferredRepositoryAdvanceConsumptionReceipt>,
    cleanup_eligibility: ArchivedCleanupEligibilitySchema,
}

#[derive(Debug, Clone, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SuccessTaskArchiveStatusSchema {
    archive_id: UnicaId,
    outcome: SuccessArchiveOutcome,
    sha256: Sha256Digest,
    retained_lineage_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AbandonedTaskArchiveStatusSchema {
    archive_id: UnicaId,
    outcome: AbandonedArchiveOutcome,
    sha256: Sha256Digest,
    retained_lineage_digest: Sha256Digest,
}

macro_rules! cleanup_receipt_status_schema {
    ($name:ident, $phase:ty) => {
        #[derive(Debug, Clone, PartialEq, Eq, JsonSchema)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct $name {
            cleanup_receipt_id: UnicaId,
            operation_id: OperationId,
            archive_id: UnicaId,
            approved_preview_digest: Sha256Digest,
            owned_targets: CanonicalOwnedTargets,
            quarantine_id: UnicaId,
            absent_observation_digests: CanonicalAbsenceObservationDigests,
            resulting_phase: $phase,
            receipt_digest: Sha256Digest,
        }
    };
}

cleanup_receipt_status_schema!(CleanedSuccessCleanupReceiptSchema, CleanedSuccessTaskPhase);
cleanup_receipt_status_schema!(
    CleanedAbandonedCleanupReceiptSchema,
    CleanedAbandonedTaskPhase
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PreWorkspaceExistingTaskStatus {
    #[serde(flatten)]
    common: ExistingTaskStatusCommon,
    phase: PreWorkspaceTaskStatusPhase,
}

#[derive(Debug, Clone, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PreWorkspaceExistingTaskStatusSchema {
    #[serde(flatten)]
    common: ExistingTaskWithoutArchiveCommonSchema,
    phase: PreWorkspaceTaskStatusPhase,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WorkspaceExistingTaskStatus {
    #[serde(flatten)]
    common: ExistingTaskStatusCommon,
    phase: WorkspaceTaskStatusPhase,
    task_workspace_id: UnicaId,
}

#[derive(Debug, Clone, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WorkspaceExistingTaskStatusSchema {
    #[serde(flatten)]
    common: ExistingTaskWithoutArchiveCommonSchema,
    phase: WorkspaceTaskStatusPhase,
    task_workspace_id: UnicaId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PreWorkspaceRecoveryExistingTaskStatus {
    #[serde(flatten)]
    common: ExistingTaskStatusCommon,
    phase: RecoveryTaskStatusPhase,
    recovery: RecoveryPlanStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PreWorkspaceRecoveryExistingTaskStatusSchema {
    #[serde(flatten)]
    common: ExistingTaskWithoutArchiveCommonSchema,
    phase: RecoveryTaskStatusPhase,
    recovery: PreWorkspaceRecoveryPlanStatusSchema,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WorkspaceRecoveryExistingTaskStatus {
    #[serde(flatten)]
    common: ExistingTaskStatusCommon,
    phase: RecoveryTaskStatusPhase,
    task_workspace_id: UnicaId,
    recovery: RecoveryPlanStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WorkspaceRecoveryExistingTaskStatusSchema {
    #[serde(flatten)]
    common: ExistingTaskWithoutArchiveCommonSchema,
    phase: RecoveryTaskStatusPhase,
    task_workspace_id: UnicaId,
    recovery: WorkspaceRecoveryPlanStatusSchema,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ArchivedCleanupRecoveryExistingTaskStatus {
    #[serde(flatten)]
    common: ExistingTaskStatusCommon,
    phase: RecoveryTaskStatusPhase,
    recovery: RecoveryPlanStatus,
    archive: TaskArchiveStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ArchivedCleanupRecoveryExistingTaskStatusSchema {
    #[serde(flatten)]
    common: ExistingTaskWithArchiveCommonSchema,
    phase: RecoveryTaskStatusPhase,
    recovery: ArchivedCleanupRecoveryPlanStatusSchema,
    archive: TaskArchiveStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ArchivedSuccessExistingTaskStatus {
    #[serde(flatten)]
    common: ExistingTaskStatusCommon,
    phase: ArchivedSuccessTaskPhase,
    task_workspace_id: UnicaId,
    archive: TaskArchiveStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ArchivedSuccessExistingTaskStatusSchema {
    #[serde(flatten)]
    common: ExistingTaskWithArchiveCommonSchema,
    phase: ArchivedSuccessTaskPhase,
    task_workspace_id: UnicaId,
    archive: SuccessTaskArchiveStatusSchema,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ArchivedAbandonedExistingTaskStatus {
    #[serde(flatten)]
    common: ExistingTaskStatusCommon,
    phase: ArchivedAbandonedTaskPhase,
    task_workspace_id: UnicaId,
    archive: TaskArchiveStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ArchivedAbandonedExistingTaskStatusSchema {
    #[serde(flatten)]
    common: ExistingTaskWithArchiveCommonSchema,
    phase: ArchivedAbandonedTaskPhase,
    task_workspace_id: UnicaId,
    archive: AbandonedTaskArchiveStatusSchema,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CleanedSuccessExistingTaskStatus {
    #[serde(flatten)]
    common: ExistingTaskStatusCommon,
    phase: CleanedSuccessTaskPhase,
    archive: TaskArchiveStatus,
    cleanup_receipt: CleanupReceipt,
}

#[derive(Debug, Clone, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CleanedSuccessExistingTaskStatusSchema {
    #[serde(flatten)]
    common: ExistingTaskWithArchiveCommonSchema,
    phase: CleanedSuccessTaskPhase,
    archive: SuccessTaskArchiveStatusSchema,
    cleanup_receipt: CleanedSuccessCleanupReceiptSchema,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CleanedAbandonedExistingTaskStatus {
    #[serde(flatten)]
    common: ExistingTaskStatusCommon,
    phase: CleanedAbandonedTaskPhase,
    archive: TaskArchiveStatus,
    cleanup_receipt: CleanupReceipt,
}

#[derive(Debug, Clone, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CleanedAbandonedExistingTaskStatusSchema {
    #[serde(flatten)]
    common: ExistingTaskWithArchiveCommonSchema,
    phase: CleanedAbandonedTaskPhase,
    archive: AbandonedTaskArchiveStatusSchema,
    cleanup_receipt: CleanedAbandonedCleanupReceiptSchema,
}

macro_rules! exact_leaf_schema {
    ($runtime:ty, $schema:ty, $name:literal) => {
        impl JsonSchema for $runtime {
            fn schema_name() -> Cow<'static, str> {
                $name.into()
            }

            fn json_schema(generator: &mut SchemaGenerator) -> Schema {
                <$schema>::json_schema(generator)
            }
        }
    };
}

exact_leaf_schema!(
    PreWorkspaceExistingTaskStatus,
    PreWorkspaceExistingTaskStatusSchema,
    "PreWorkspaceExistingTaskStatus"
);
exact_leaf_schema!(
    WorkspaceExistingTaskStatus,
    WorkspaceExistingTaskStatusSchema,
    "WorkspaceExistingTaskStatus"
);
exact_leaf_schema!(
    PreWorkspaceRecoveryExistingTaskStatus,
    PreWorkspaceRecoveryExistingTaskStatusSchema,
    "PreWorkspaceRecoveryExistingTaskStatus"
);
exact_leaf_schema!(
    WorkspaceRecoveryExistingTaskStatus,
    WorkspaceRecoveryExistingTaskStatusSchema,
    "WorkspaceRecoveryExistingTaskStatus"
);
exact_leaf_schema!(
    ArchivedCleanupRecoveryExistingTaskStatus,
    ArchivedCleanupRecoveryExistingTaskStatusSchema,
    "ArchivedCleanupRecoveryExistingTaskStatus"
);
exact_leaf_schema!(
    ArchivedSuccessExistingTaskStatus,
    ArchivedSuccessExistingTaskStatusSchema,
    "ArchivedSuccessExistingTaskStatus"
);
exact_leaf_schema!(
    ArchivedAbandonedExistingTaskStatus,
    ArchivedAbandonedExistingTaskStatusSchema,
    "ArchivedAbandonedExistingTaskStatus"
);
exact_leaf_schema!(
    CleanedSuccessExistingTaskStatus,
    CleanedSuccessExistingTaskStatusSchema,
    "CleanedSuccessExistingTaskStatus"
);
exact_leaf_schema!(
    CleanedAbandonedExistingTaskStatus,
    CleanedAbandonedExistingTaskStatusSchema,
    "CleanedAbandonedExistingTaskStatus"
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
enum ExistingTaskStatusDataKind {
    PreWorkspace(PreWorkspaceExistingTaskStatus),
    Workspace(WorkspaceExistingTaskStatus),
    PreWorkspaceRecovery(Box<PreWorkspaceRecoveryExistingTaskStatus>),
    WorkspaceRecovery(Box<WorkspaceRecoveryExistingTaskStatus>),
    ArchivedCleanupRecovery(Box<ArchivedCleanupRecoveryExistingTaskStatus>),
    ArchivedSuccess(ArchivedSuccessExistingTaskStatus),
    ArchivedAbandoned(ArchivedAbandonedExistingTaskStatus),
    CleanedSuccess(CleanedSuccessExistingTaskStatus),
    CleanedAbandoned(CleanedAbandonedExistingTaskStatus),
}

/// Existing-task status with phase-dependent fields represented as a physical
/// nine-leaf union. Deliberately not `Deserialize`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct ExistingTaskStatusData(ExistingTaskStatusDataKind);

impl JsonSchema for ExistingTaskStatusData {
    fn schema_name() -> Cow<'static, str> {
        "TaskStatusData".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<PreWorkspaceExistingTaskStatus>(),
            generator.subschema_for::<WorkspaceExistingTaskStatus>(),
            generator.subschema_for::<PreWorkspaceRecoveryExistingTaskStatus>(),
            generator.subschema_for::<WorkspaceRecoveryExistingTaskStatus>(),
            generator.subschema_for::<ArchivedCleanupRecoveryExistingTaskStatus>(),
            generator.subschema_for::<ArchivedSuccessExistingTaskStatus>(),
            generator.subschema_for::<ArchivedAbandonedExistingTaskStatus>(),
            generator.subschema_for::<CleanedSuccessExistingTaskStatus>(),
            generator.subschema_for::<CleanedAbandonedExistingTaskStatus>(),
        ])
    }
}

impl ExistingTaskStatusData {
    pub(crate) fn pre_workspace(
        phase: TaskPhase,
        authority: ExistingTaskStatusAuthority,
    ) -> Result<Self, StatusContractError> {
        authority.require_no_archive()?;
        authority.require_no_workspace()?;
        authority.require_no_recovery()?;
        Ok(Self(ExistingTaskStatusDataKind::PreWorkspace(
            PreWorkspaceExistingTaskStatus {
                common: authority.common,
                phase: phase.try_into()?,
            },
        )))
    }

    pub(crate) fn workspace(
        phase: TaskPhase,
        task_workspace_id: UnicaId,
        authority: ExistingTaskStatusAuthority,
    ) -> Result<Self, StatusContractError> {
        authority.require_no_archive()?;
        authority.require_workspace(&task_workspace_id)?;
        authority.require_no_recovery()?;
        Ok(Self(ExistingTaskStatusDataKind::Workspace(
            WorkspaceExistingTaskStatus {
                common: authority.common,
                phase: phase.try_into()?,
                task_workspace_id,
            },
        )))
    }

    pub(crate) fn pre_workspace_recovery(
        phase: TaskPhase,
        recovery: RecoveryPlanStatus,
        authority: ExistingTaskStatusAuthority,
    ) -> Result<Self, StatusContractError> {
        authority.require_no_archive()?;
        authority.require_no_workspace()?;
        authority.require_recovery(&recovery)?;
        if !matches!(
            recovery.target(),
            RecoveryTarget::TaskConfiguration | RecoveryTarget::Artifact
        ) {
            return Err(StatusContractError(
                "recovery target is not admissible before a durable task workspace",
            ));
        }
        Ok(Self(ExistingTaskStatusDataKind::PreWorkspaceRecovery(
            Box::new(PreWorkspaceRecoveryExistingTaskStatus {
                common: authority.common,
                phase: phase.try_into()?,
                recovery,
            }),
        )))
    }

    pub(crate) fn workspace_recovery(
        phase: TaskPhase,
        task_workspace_id: UnicaId,
        recovery: RecoveryPlanStatus,
        authority: ExistingTaskStatusAuthority,
    ) -> Result<Self, StatusContractError> {
        authority.require_no_archive()?;
        authority.require_workspace(&task_workspace_id)?;
        authority.require_recovery(&recovery)?;
        if recovery.target() == RecoveryTarget::Cleanup {
            return Err(StatusContractError(
                "cleanup recovery requires the archived-cleanup status branch",
            ));
        }
        Ok(Self(ExistingTaskStatusDataKind::WorkspaceRecovery(
            Box::new(WorkspaceRecoveryExistingTaskStatus {
                common: authority.common,
                phase: phase.try_into()?,
                task_workspace_id,
                recovery,
            }),
        )))
    }

    pub(crate) fn archived_cleanup_recovery(
        phase: TaskPhase,
        recovery: RecoveryPlanStatus,
        archive: TaskArchiveStatus,
        authority: ExistingTaskStatusAuthority,
    ) -> Result<Self, StatusContractError> {
        authority.require_no_workspace()?;
        authority.require_exact_archive(&archive)?;
        authority.require_recovery(&recovery)?;
        authority.require_terminal_quiescence(false, true)?;
        let cleanup = recovery.cleanup_binding().map_err(|_| {
            StatusContractError("archived-cleanup recovery accepts only a valid cleanup plan")
        })?;
        if cleanup.archive_id() != archive.archive_id() {
            return Err(StatusContractError(
                "cleanup finalization archive ID disagrees with retained archive",
            ));
        }
        let expected_phase = match archive.outcome() {
            TaskArchiveOutcome::Success => TaskPhase::CleanedSuccess,
            TaskArchiveOutcome::Abandoned => TaskPhase::CleanedAbandoned,
        };
        if cleanup.planned_result_phase() != expected_phase {
            return Err(StatusContractError(
                "cleanup planned result phase disagrees with retained archive outcome",
            ));
        }
        Ok(Self(ExistingTaskStatusDataKind::ArchivedCleanupRecovery(
            Box::new(ArchivedCleanupRecoveryExistingTaskStatus {
                common: authority.common,
                phase: phase.try_into()?,
                recovery,
                archive,
            }),
        )))
    }

    pub(crate) fn archived(
        task_workspace_id: UnicaId,
        archive: TaskArchiveStatus,
        authority: ExistingTaskStatusAuthority,
    ) -> Result<Self, StatusContractError> {
        authority.require_workspace(&task_workspace_id)?;
        authority.require_no_recovery()?;
        authority.require_exact_archive(&archive)?;
        authority.require_terminal_quiescence(true, false)?;
        Ok(match archive.outcome() {
            TaskArchiveOutcome::Success => Self(ExistingTaskStatusDataKind::ArchivedSuccess(
                ArchivedSuccessExistingTaskStatus {
                    common: authority.common,
                    phase: ArchivedSuccessTaskPhase::Value,
                    task_workspace_id,
                    archive,
                },
            )),
            TaskArchiveOutcome::Abandoned => Self(ExistingTaskStatusDataKind::ArchivedAbandoned(
                ArchivedAbandonedExistingTaskStatus {
                    common: authority.common,
                    phase: ArchivedAbandonedTaskPhase::Value,
                    task_workspace_id,
                    archive,
                },
            )),
        })
    }

    pub(crate) fn cleaned(
        archive: TaskArchiveStatus,
        cleanup_receipt: CleanupReceipt,
        authority: ExistingTaskStatusAuthority,
    ) -> Result<Self, StatusContractError> {
        authority.require_no_workspace()?;
        authority.require_no_recovery()?;
        authority.require_exact_archive(&archive)?;
        authority.require_terminal_quiescence(false, false)?;
        if archive.archive_id() != cleanup_receipt.archive_id() {
            return Err(StatusContractError(
                "cleanup receipt must bind the retained archive",
            ));
        }
        Ok(
            match (archive.outcome(), cleanup_receipt.resulting_phase()) {
                (TaskArchiveOutcome::Success, CleanupResultPhase::CleanedSuccess) => Self(
                    ExistingTaskStatusDataKind::CleanedSuccess(CleanedSuccessExistingTaskStatus {
                        common: authority.common,
                        phase: CleanedSuccessTaskPhase::Value,
                        archive,
                        cleanup_receipt,
                    }),
                ),
                (TaskArchiveOutcome::Abandoned, CleanupResultPhase::CleanedAbandoned) => {
                    Self(ExistingTaskStatusDataKind::CleanedAbandoned(
                        CleanedAbandonedExistingTaskStatus {
                            common: authority.common,
                            phase: CleanedAbandonedTaskPhase::Value,
                            archive,
                            cleanup_receipt,
                        },
                    ))
                }
                _ => {
                    return Err(StatusContractError(
                        "cleanup result phase must match the retained archive outcome",
                    ));
                }
            },
        )
    }

    pub(crate) fn phase(&self) -> TaskPhase {
        match &self.0 {
            ExistingTaskStatusDataKind::PreWorkspace(status) => status.phase.into(),
            ExistingTaskStatusDataKind::Workspace(status) => status.phase.into(),
            ExistingTaskStatusDataKind::PreWorkspaceRecovery(status) => status.phase.into(),
            ExistingTaskStatusDataKind::WorkspaceRecovery(status) => status.phase.into(),
            ExistingTaskStatusDataKind::ArchivedCleanupRecovery(status) => status.phase.into(),
            ExistingTaskStatusDataKind::ArchivedSuccess(_) => TaskPhase::ArchivedSuccess,
            ExistingTaskStatusDataKind::ArchivedAbandoned(_) => TaskPhase::ArchivedAbandoned,
            ExistingTaskStatusDataKind::CleanedSuccess(_) => TaskPhase::CleanedSuccess,
            ExistingTaskStatusDataKind::CleanedAbandoned(_) => TaskPhase::CleanedAbandoned,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::branched_development::contracts::scalars::{
        NormalizedUtcInstant, PositiveGeneration,
    };
    use crate::domain::branched_development::contracts::schema::audit_json_schema;
    use crate::domain::branched_development::contracts::selectors::{
        BranchedStartSelector, TaskOperationSelector,
    };
    use crate::domain::branched_development::{
        DurableExecutionPolicy, OperationId, Sha256Digest, UnicaId,
    };
    use schemars::{schema_for, JsonSchema};
    use serde::de::DeserializeOwned;
    use serde_json::{json, Value};

    const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const ID_1: &str = "11111111-1111-4111-8111-111111111111";
    const ID_2: &str = "22222222-2222-4222-8222-222222222222";

    fn digest(value: &str) -> Sha256Digest {
        Sha256Digest::parse(value).unwrap()
    }

    fn id(value: &str) -> UnicaId {
        UnicaId::parse(value).unwrap()
    }

    fn operation_id(value: &str) -> OperationId {
        OperationId::parse(value).unwrap()
    }

    fn instant(value: &str) -> NormalizedUtcInstant {
        serde_json::from_value(json!(value)).unwrap()
    }

    fn generation(value: u64) -> PositiveGeneration {
        serde_json::from_value(json!(value)).unwrap()
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

    assert_not_deserialize_owned!(OperationLease);
    assert_not_deserialize_owned!(ActiveOperationStatus);
    assert_not_deserialize_owned!(CleanupReceipt);
    assert_not_deserialize_owned!(CompletedCleanupAbsence);
    assert_not_deserialize_owned!(CompletedCleanupAbsences);
    assert_not_deserialize_owned!(ApprovedCleanupAttempt);
    assert_not_deserialize_owned!(CompletedCleanupAttempt);
    assert_not_deserialize_owned!(CleanupReceiptAuthority);
    assert_not_deserialize_owned!(SuccessCleanupReceiptDigestRecord);
    assert_not_deserialize_owned!(AbandonedCleanupReceiptDigestRecord);
    assert_not_deserialize_owned!(SuccessCleanupReceipt);
    assert_not_deserialize_owned!(AbandonedCleanupReceipt);
    assert_not_deserialize_owned!(PendingDecisionStatus);
    assert_not_deserialize_owned!(TaskAnchorStatus);
    assert_not_deserialize_owned!(OwnedLockStatus);
    assert_not_deserialize_owned!(ValidationGateStatus);
    assert_not_deserialize_owned!(ArtifactHashStatus);
    assert_not_deserialize_owned!(TaskArchiveStatus);
    assert_not_deserialize_owned!(CleanupEligibilityStatus);
    assert_not_deserialize_owned!(RecentOperationStatus);
    assert_not_deserialize_owned!(RecentOperations);
    assert_not_deserialize_owned!(ArtifactResumeHandle);
    assert_not_deserialize_owned!(WorkspaceResumeHandle);
    assert_not_deserialize_owned!(MergeResolutionWorkspaceResumeHandle);
    assert_not_deserialize_owned!(CheckpointResumeHandle);
    assert_not_deserialize_owned!(ComparisonResumeHandle);
    assert_not_deserialize_owned!(DeferredRepositoryAdvanceResumeHandle);
    assert_not_deserialize_owned!(RecoveryResumeHandle);
    assert_not_deserialize_owned!(ArchiveResumeHandle);
    assert_not_deserialize_owned!(MergeSessionResumeHandle);
    assert_not_deserialize_owned!(DecisionResumeHandle);
    assert_not_deserialize_owned!(ResolutionChangeReceiptResumeHandle);
    assert_not_deserialize_owned!(VerificationResumeHandle);
    assert_not_deserialize_owned!(MergeApplyResumeHandle);
    assert_not_deserialize_owned!(LockPlanResumeHandle);
    assert_not_deserialize_owned!(LockSetResumeHandle);
    assert_not_deserialize_owned!(PreviewResumeHandle);
    assert_not_deserialize_owned!(SupportPreflightResumeHandle);
    assert_not_deserialize_owned!(SupportActionAuthorizationResumeHandle);
    assert_not_deserialize_owned!(SupportPrerequisiteResumeHandle);
    assert_not_deserialize_owned!(SupportRecoveryResumeHandle);
    assert_not_deserialize_owned!(CompletedPreArmCancellationProgress);
    assert_not_deserialize_owned!(SupportCancellationResumeHandle);
    assert_not_deserialize_owned!(ResumeHandle);
    assert_not_deserialize_owned!(ResumeHandles);
    assert_not_deserialize_owned!(OwnedLockStatuses);
    assert_not_deserialize_owned!(PendingDecisionStatuses);
    assert_not_deserialize_owned!(TaskAnchorStatuses);
    assert_not_deserialize_owned!(ValidationGateStatuses);
    assert_not_deserialize_owned!(ArtifactHashStatuses);
    assert_not_deserialize_owned!(ExistingTaskStatusData);

    fn lease() -> OperationLease {
        OperationLease::test_only(
            id(ID_1),
            generation(1),
            instant("2026-07-22T00:00:00Z"),
            instant("2026-07-22T00:00:01Z"),
            instant("2026-07-22T00:01:01Z"),
        )
        .unwrap()
    }

    fn empty_status_authority(
        cleanup_eligibility: CleanupEligibilityStatus,
    ) -> ExistingTaskStatusAuthority {
        status_authority(
            None,
            Vec::new(),
            ExistingTaskDeferredState::no_terminal_support(),
            cleanup_eligibility,
        )
        .unwrap()
    }

    fn status_authority(
        active_operation: Option<ActiveOperationStatus>,
        resume_handles: Vec<ResumeHandle>,
        deferred_state: ExistingTaskDeferredState,
        cleanup_eligibility: CleanupEligibilityStatus,
    ) -> Result<ExistingTaskStatusAuthority, StatusContractError> {
        status_authority_with_work(
            active_operation,
            Vec::new(),
            Vec::new(),
            resume_handles,
            deferred_state,
            cleanup_eligibility,
        )
    }

    fn status_authority_with_work(
        active_operation: Option<ActiveOperationStatus>,
        pending_decisions: Vec<PendingDecisionStatus>,
        owned_locks: Vec<OwnedLockStatus>,
        resume_handles: Vec<ResumeHandle>,
        deferred_state: ExistingTaskDeferredState,
        cleanup_eligibility: CleanupEligibilityStatus,
    ) -> Result<ExistingTaskStatusAuthority, StatusContractError> {
        ExistingTaskStatusAuthority::new(
            id(ID_1),
            active_operation,
            ExistingTaskStatusCollections::new(
                PendingDecisionStatuses::new(pending_decisions).unwrap(),
                TaskAnchorStatuses::new(Vec::new()).unwrap(),
                OwnedLockStatuses::new(owned_locks).unwrap(),
                ValidationGateStatuses::new(Vec::new()).unwrap(),
                ArtifactHashStatuses::new(Vec::new()).unwrap(),
                ResumeHandles::new(resume_handles).unwrap(),
                RecentOperations::new(Vec::new()).unwrap(),
            ),
            deferred_state,
            cleanup_eligibility,
        )
    }

    fn cursor(version: &str, digest_value: &str) -> RepositoryHistoryCursor {
        serde_json::from_value(json!({
            "throughVersion": version,
            "historyPrefixDigest": digest_value,
        }))
        .unwrap()
    }

    fn terminal_support_handle(
        receipt_id: UnicaId,
        deferred_repository_advance_digest: Option<Sha256Digest>,
    ) -> ResumeHandle {
        let body = SupportPrerequisiteResumeBody {
            handle_kind: SupportPrerequisiteHandleKind::Value,
            receipt_id,
            prior_support_action_id: id("33333333-3333-4333-8333-333333333333"),
            prior_support_gate_id: id("44444444-4444-4444-8444-444444444444"),
            purpose: SupportActionPurpose::MainIntegrationPrerequisite,
            arming_receipt_id: id("55555555-5555-4555-8555-555555555555"),
            arming_receipt_digest: digest(A),
            repository_version: serde_json::from_value(json!("version-10")).unwrap(),
            authorized_transitions_digest: digest(A),
            root_delta_digest: digest(B),
            root_lock_proof_digest: digest(A),
            history_from_cursor: cursor("version-9", A),
            history_through_cursor: cursor("version-10", B),
            history_partition_digest: digest(A),
            selective_update_proof_digest: digest(B),
            post_release_observed_history_cursor: cursor("version-10", B),
            post_apply_history_partition_digest: digest(A),
            deferred_repository_advance_digest,
            resulting_phase: TaskPhase::Synchronized,
        };
        ResumeHandle::SupportPrerequisite(SupportPrerequisiteResumeHandle(
            SupportPrerequisiteResumeHandleKind::SeparateWorkingInfobase(
                SeparateWorkingInfobaseSupportPrerequisiteResumeHandle {
                    body,
                    manual_target_mode: SeparateWorkingInfobaseStatusMode::Value,
                    manual_working_infobase_closure_proof_digest: digest(B),
                },
            ),
        ))
    }

    fn merge_decision_body() -> MergeConflictDecisionResumeBody {
        MergeConflictDecisionResumeBody {
            handle_kind: DecisionHandleKind::Value,
            decision_id: id("33333333-3333-4333-8333-333333333333"),
            decision_kind: MergeConflictResumeDecisionKind::Value,
            session_id: id("44444444-4444-4444-8444-444444444444"),
            base_session_digest: digest(A),
            conflict_id: id("55555555-5555-4555-8555-555555555555"),
            resolution: ConflictResolution::TakeOurs,
            rationale_digest: digest(B),
            change_receipt_digest: None,
            replaces_decision_id: None,
            decision_digest: digest(A),
            revised_decision_set_digest: digest(B),
        }
    }

    fn historical_decision_handle() -> ResumeHandle {
        ResumeHandle::Decision(DecisionResumeHandle(
            DecisionResumeHandleKind::SupersededMergeConflict(
                SupersededMergeConflictDecisionResumeHandle {
                    body: merge_decision_body(),
                    current: HistoricalDecisionLiteral,
                    superseded_by_change_receipt_id: id("66666666-6666-4666-8666-666666666666"),
                },
            ),
        ))
    }

    fn current_decision_handle() -> ResumeHandle {
        ResumeHandle::Decision(DecisionResumeHandle(
            DecisionResumeHandleKind::CurrentMergeConflict(
                CurrentMergeConflictDecisionResumeHandle {
                    body: merge_decision_body(),
                    current: CurrentDecisionLiteral,
                },
            ),
        ))
    }

    fn selector() -> TaskOperationSelector {
        TaskOperationSelector::BranchedStart(BranchedStartSelector::new())
    }

    #[test]
    fn operation_lease_hashes_heartbeat_and_complete_lease_records() {
        let lease = lease();
        let encoded = serde_json::to_value(&lease).unwrap();
        assert_ne!(encoded["heartbeatDigest"], encoded["leaseDigest"]);

        audit_json_schema(&schema::<OperationHeartbeatDigestRecord>()).unwrap();
        audit_json_schema(&schema::<OperationLeaseDigestRecord>()).unwrap();
        audit_json_schema(&schema::<OperationLease>()).unwrap();

        let mut substituted = encoded;
        substituted["heartbeatAt"] = json!("2026-07-22T00:00:02Z");
        assert!(schema_accepts::<OperationLease>(&substituted));
        assert!(!OperationLease::validates_json(&substituted));
    }

    #[test]
    fn operation_lease_orders_instants_chronologically_not_lexically() {
        OperationLease::test_only(
            id(ID_1),
            generation(1),
            instant("2026-07-22T00:00:00Z"),
            instant("2026-07-22T00:00:00.1Z"),
            instant("2026-07-22T00:00:01Z"),
        )
        .expect("a fractional heartbeat after the whole-second acquisition is ordered later");
    }

    #[test]
    fn general_status_collections_use_the_normative_1024_item_bound() {
        for contract in [
            schema::<RecentOperations>(),
            schema::<ResumeHandles>(),
            schema::<OwnedLockStatuses>(),
            schema::<PendingDecisionStatuses>(),
            schema::<TaskAnchorStatuses>(),
            schema::<ValidationGateStatuses>(),
            schema::<ArtifactHashStatuses>(),
        ] {
            assert_eq!(contract["maxItems"], 1024);
        }
    }

    #[test]
    fn active_operation_status_is_an_exact_three_state_projection() {
        let registered = ActiveOperationStatus::registered_test_only(
            operation_id(ID_2),
            selector(),
            DurableExecutionPolicy::LocalJournaled,
            digest(A),
            instant("2026-07-22T00:00:00Z"),
            lease(),
            ActiveOperationOwnerState::Live,
        )
        .unwrap();
        let encoded = serde_json::to_value(&registered).unwrap();
        assert_eq!(encoded["state"], "registered");
        assert!(encoded.get("operationLease").is_some());
        assert!(encoded.get("recoveryDigest").is_none());

        let unknown = ActiveOperationStatus::effect_unknown_test_only(
            operation_id(ID_2),
            selector(),
            DurableExecutionPolicy::JournaledEffect,
            digest(A),
            instant("2026-07-22T00:00:00Z"),
            digest(B),
        )
        .unwrap();
        let unknown = serde_json::to_value(unknown).unwrap();
        assert!(unknown.get("operationLease").is_none());
        assert_eq!(unknown["recoveryDigest"], B);

        let contract = schema::<ActiveOperationStatus>();
        audit_json_schema(&contract).unwrap();
        assert_eq!(contract["oneOf"].as_array().map(Vec::len), Some(3));
        let mut read_only = encoded.clone();
        read_only["policy"] = json!("readOnly");
        assert!(!schema_accepts::<ActiveOperationStatus>(&read_only));
        let mut missing_lease = encoded;
        missing_lease
            .as_object_mut()
            .unwrap()
            .remove("operationLease");
        assert!(!schema_accepts::<ActiveOperationStatus>(&missing_lease));
        let mut spliced_lease = unknown;
        spliced_lease["operationLease"] = serde_json::to_value(lease()).unwrap();
        assert!(!schema_accepts::<ActiveOperationStatus>(&spliced_lease));
    }

    #[test]
    fn cleanup_receipt_is_digest_bound_and_has_a_closed_terminal_phase() {
        let owned_target = serde_json::from_value(json!({
            "projectId": ID_1,
            "instanceId": ID_2,
            "role": "artifact"
        }))
        .unwrap();
        let receipt = CleanupReceipt::test_only(CleanupReceiptTestParts {
            cleanup_receipt_id: id(ID_1),
            operation_id: operation_id(ID_2),
            archive_id: id("33333333-3333-4333-8333-333333333333"),
            approved_preview_digest: digest(A),
            owned_targets: vec![owned_target],
            quarantine_id: id("44444444-4444-4444-8444-444444444444"),
            absent_observation_digests: vec![digest(B)],
            resulting_phase: CleanupResultPhase::CleanedSuccess,
        })
        .unwrap();
        let encoded = serde_json::to_value(&receipt).unwrap();
        assert_eq!(encoded["resultingPhase"], "cleanedSuccess");
        assert!(encoded.get("receiptDigest").is_some());

        audit_json_schema(&schema::<SuccessCleanupReceiptDigestRecord>()).unwrap();
        audit_json_schema(&schema::<AbandonedCleanupReceiptDigestRecord>()).unwrap();
        audit_json_schema(&schema::<CleanupReceipt>()).unwrap();
        let mut substituted = encoded;
        substituted["quarantineId"] = json!("55555555-5555-4555-8555-555555555555");
        assert!(schema_accepts::<CleanupReceipt>(&substituted));
        assert!(!CleanupReceipt::validates_json(&substituted));
    }

    #[test]
    fn cleanup_receipt_preserves_paired_target_order_and_allows_direct_empty_completion() {
        let target_1 = serde_json::from_value(json!({
            "projectId": ID_1,
            "instanceId": ID_1,
            "role": "artifact"
        }))
        .unwrap();
        let target_2 = serde_json::from_value(json!({
            "projectId": ID_1,
            "instanceId": ID_2,
            "role": "artifact"
        }))
        .unwrap();

        let receipt = CleanupReceipt::test_only(CleanupReceiptTestParts {
            cleanup_receipt_id: id(ID_1),
            operation_id: operation_id(ID_2),
            archive_id: id("33333333-3333-4333-8333-333333333333"),
            approved_preview_digest: digest(A),
            owned_targets: vec![target_1, target_2],
            quarantine_id: id("44444444-4444-4444-8444-444444444444"),
            absent_observation_digests: vec![digest(B), digest(A)],
            resulting_phase: CleanupResultPhase::CleanedSuccess,
        })
        .expect("observation digests follow target order and are never sorted independently");
        let encoded = serde_json::to_value(receipt).unwrap();
        assert_eq!(encoded["absentObservationDigests"], json!([B, A]));

        let empty = CleanupReceipt::test_only(CleanupReceiptTestParts {
            cleanup_receipt_id: id(ID_1),
            operation_id: operation_id(ID_2),
            archive_id: id("33333333-3333-4333-8333-333333333333"),
            approved_preview_digest: digest(A),
            owned_targets: Vec::new(),
            quarantine_id: id("44444444-4444-4444-8444-444444444444"),
            absent_observation_digests: Vec::new(),
            resulting_phase: CleanupResultPhase::CleanedSuccess,
        })
        .expect("an already-empty owned set completes directly");
        let encoded = serde_json::to_value(empty).unwrap();
        assert_eq!(encoded["ownedTargets"], json!([]));
        assert_eq!(encoded["absentObservationDigests"], json!([]));
    }

    #[test]
    fn task12_cleanup_attempt_consumes_exact_full_recovery_observation_set() {
        let operation_id = operation_id(ID_2);
        let archive = TaskArchiveStatus::new(
            id("33333333-3333-4333-8333-333333333333"),
            TaskArchiveOutcome::Success,
            digest(A),
            digest(B),
        );
        let owned_target: OwnedTargetLocator = serde_json::from_value(json!({
            "projectId": ID_1,
            "instanceId": ID_1,
            "role": "artifact"
        }))
        .unwrap();
        let recovery = RecoveryPlanStatus::cleanup_fixture_test_only(
            operation_id.clone(),
            archive.archive_id().clone(),
            owned_target.clone(),
            TaskPhase::CleanedSuccess,
        )
        .unwrap();
        let observations = recovery
            .cleanup_matching_absence_observations_test_only()
            .unwrap();
        let attempt = ApprovedCleanupAttempt::from_recovery_test_only(
            operation_id.clone(),
            &archive,
            digest(A),
            digest(B),
            id("cccccccc-cccc-4ccc-8ccc-cccccccccccc"),
            vec![owned_target.clone()],
            recovery,
        )
        .unwrap();
        let completed = attempt.observe_absences(observations).unwrap();
        assert_eq!(completed.operation_id(), &operation_id);
        assert_eq!(
            completed.owned_targets(),
            std::slice::from_ref(&owned_target)
        );
        assert_eq!(completed.outcome(), TaskArchiveOutcome::Success);
        assert!(completed.recovery_digest().is_some());
        assert!(completed.finish_action_id().is_some());

        let authority = completed.authorize_receipt(id(ID_1)).unwrap();
        assert_eq!(authority.marker_digest(), &digest(B));
        assert!(authority.recovery_digest().is_some());
        let receipt = CleanupReceipt::new(authority).unwrap();
        assert_eq!(
            receipt.resulting_phase(),
            CleanupResultPhase::CleanedSuccess
        );
        assert_eq!(receipt.owned_targets(), &[owned_target]);
        assert_eq!(receipt.absent_observation_digests().len(), 1);
    }

    #[test]
    fn task12_cleanup_attempt_rejects_cross_attempt_observation_splice() {
        let target: OwnedTargetLocator = serde_json::from_value(json!({
            "projectId": ID_1,
            "instanceId": ID_1,
            "role": "artifact"
        }))
        .unwrap();
        let archive = TaskArchiveStatus::new(
            id("33333333-3333-4333-8333-333333333333"),
            TaskArchiveOutcome::Success,
            digest(A),
            digest(B),
        );
        let own_recovery = RecoveryPlanStatus::cleanup_fixture_test_only(
            operation_id(ID_1),
            archive.archive_id().clone(),
            target.clone(),
            TaskPhase::CleanedSuccess,
        )
        .unwrap();
        let foreign_recovery = RecoveryPlanStatus::cleanup_fixture_test_only(
            operation_id(ID_2),
            archive.archive_id().clone(),
            target.clone(),
            TaskPhase::CleanedSuccess,
        )
        .unwrap();
        let foreign_observations = foreign_recovery
            .cleanup_matching_absence_observations_test_only()
            .unwrap();
        let attempt = ApprovedCleanupAttempt::from_recovery_test_only(
            operation_id(ID_1),
            &archive,
            digest(A),
            digest(B),
            id("cccccccc-cccc-4ccc-8ccc-cccccccccccc"),
            vec![target],
            own_recovery,
        )
        .unwrap();

        assert!(attempt.observe_absences(foreign_observations).is_err());
    }

    #[test]
    fn task12_cleanup_direct_completion_requires_a_paired_empty_set() {
        let archive = TaskArchiveStatus::new(
            id("33333333-3333-4333-8333-333333333333"),
            TaskArchiveOutcome::Abandoned,
            digest(A),
            digest(B),
        );
        let attempt = ApprovedCleanupAttempt::direct_empty_test_only(
            operation_id(ID_2),
            &archive,
            digest(A),
            digest(B),
            id("44444444-4444-4444-8444-444444444444"),
            Vec::new(),
        )
        .unwrap();
        let completed = attempt.complete_direct_empty().unwrap();
        assert!(completed.owned_targets().is_empty());
        assert!(completed.absent_observation_digests().is_empty());
        assert!(completed.recovery_digest().is_none());

        let receipt = CleanupReceipt::new(completed.authorize_receipt(id(ID_1)).unwrap()).unwrap();
        let encoded = serde_json::to_value(&receipt).unwrap();
        assert_eq!(encoded["resultingPhase"], "cleanedAbandoned");
        assert_eq!(encoded["ownedTargets"], json!([]));
        assert_eq!(encoded["absentObservationDigests"], json!([]));
        assert_eq!(
            schema::<CleanupReceipt>()["oneOf"].as_array().map(Vec::len),
            Some(2)
        );
    }

    #[test]
    fn task12_cleanup_attempt_typestates_are_non_clone() {
        const _: fn() = || {
            trait AmbiguousIfClone<Marker> {
                fn assert_not_clone() {}
            }
            struct ImplementsClone;
            impl<T: ?Sized> AmbiguousIfClone<()> for T {}
            impl<T: Clone> AmbiguousIfClone<ImplementsClone> for T {}
            let _ = <ApprovedCleanupAttempt as AmbiguousIfClone<_>>::assert_not_clone;
            let _ = <CompletedCleanupAttempt as AmbiguousIfClone<_>>::assert_not_clone;
            let _ = <CleanupReceiptAuthority as AmbiguousIfClone<_>>::assert_not_clone;
        };
    }

    #[test]
    fn task12_cleanup_authority_issues_only_its_exact_physical_receipt_leaf() {
        fn abandoned_authority() -> CleanupReceiptAuthority {
            let archive = TaskArchiveStatus::new(
                id("33333333-3333-4333-8333-333333333333"),
                TaskArchiveOutcome::Abandoned,
                digest(A),
                digest(B),
            );
            ApprovedCleanupAttempt::direct_empty_test_only(
                operation_id(ID_2),
                &archive,
                digest(A),
                digest(B),
                id("44444444-4444-4444-8444-444444444444"),
                Vec::new(),
            )
            .unwrap()
            .complete_direct_empty()
            .unwrap()
            .authorize_receipt(id(ID_1))
            .unwrap()
        }

        assert!(abandoned_authority().issue_success().is_err());
        let receipt: AbandonedCleanupReceipt = abandoned_authority().issue_abandoned().unwrap();
        assert_eq!(
            receipt.archive_id(),
            &id("33333333-3333-4333-8333-333333333333")
        );
        assert_eq!(receipt.approved_preview_digest(), &digest(A));
        assert_eq!(
            receipt.resulting_phase(),
            CleanupResultPhase::CleanedAbandoned
        );
        assert!(receipt.owned_targets().is_empty());
        assert!(schema::<AbandonedCleanupReceipt>().get("oneOf").is_none());
    }

    #[test]
    fn pending_decisions_and_anchors_encode_closed_presence_matrices() {
        let adaptation =
            PendingDecisionStatus::adaptation(id(ID_1), vec![id(ID_2)], 0, digest(A)).unwrap();
        let adaptation = serde_json::to_value(adaptation).unwrap();
        assert_eq!(adaptation["decisionKind"], "adaptation");
        assert_eq!(adaptation["replacementPendingDecisionIds"], json!([]));

        let pending_schema = schema::<PendingDecisionStatus>();
        audit_json_schema(&pending_schema).unwrap();
        assert_eq!(pending_schema["oneOf"].as_array().map(Vec::len), Some(2));
        let mut illegal_replacement = adaptation;
        illegal_replacement["replacementPendingDecisionIds"] = json!([ID_1]);
        assert!(!schema_accepts::<PendingDecisionStatus>(
            &illegal_replacement
        ));
        assert!(PendingDecisionStatus::merge_conflict(
            id(ID_1),
            vec![id(ID_2), id(ID_1)],
            Vec::new(),
            0,
            digest(A),
        )
        .is_err());

        let cursor: RepositoryHistoryCursor = serde_json::from_value(json!({
            "throughVersion": "version-10",
            "historyPrefixDigest": A
        }))
        .unwrap();
        let repository = TaskAnchorStatus::repository_cursor(cursor, digest(B));
        let repository = serde_json::to_value(repository).unwrap();
        assert!(repository.get("cursor").is_some());
        assert!(repository.get("fingerprint").is_none());
        let anchor_schema = schema::<TaskAnchorStatus>();
        audit_json_schema(&anchor_schema).unwrap();
        assert_eq!(anchor_schema["oneOf"].as_array().map(Vec::len), Some(4));
        let mut spliced = repository;
        spliced["fingerprint"] = json!(A);
        assert!(!schema_accepts::<TaskAnchorStatus>(&spliced));
    }

    #[test]
    fn simple_named_status_records_are_closed_output_only_projections() {
        let target: RepositoryTargetIdentity =
            serde_json::from_value(json!({"targetKind": "configurationRoot"})).unwrap();
        let owner: RepositoryOwnerIdentity = serde_json::from_value(json!({
            "username": "repo-user",
            "computer": null,
            "infobase": null,
            "lockedAt": null
        }))
        .unwrap();
        let lock = OwnedLockStatus::new(target, owner, id(ID_1), digest(A));
        let gate = ValidationGateStatus::new(
            ValidationGateKind::Support,
            id(ID_1),
            digest(A),
            ValidationGateState::Current,
        );
        let artifact = ArtifactHashStatus::new(
            id(ID_1),
            ArtifactRole::OrdinaryResult,
            ArtifactKind::ConfigurationDistribution,
            digest(A),
        );
        let archive =
            TaskArchiveStatus::new(id(ID_1), TaskArchiveOutcome::Success, digest(A), digest(B));

        for value in [
            serde_json::to_value(lock).unwrap(),
            serde_json::to_value(gate).unwrap(),
            serde_json::to_value(artifact).unwrap(),
            serde_json::to_value(archive).unwrap(),
        ] {
            assert!(value.as_object().is_some());
        }
        audit_json_schema(&schema::<OwnedLockStatus>()).unwrap();
        audit_json_schema(&schema::<ValidationGateStatus>()).unwrap();
        audit_json_schema(&schema::<ArtifactHashStatus>()).unwrap();
        audit_json_schema(&schema::<TaskArchiveStatus>()).unwrap();
    }

    #[test]
    fn cleanup_eligibility_has_three_exact_digest_bound_states() {
        let eligible = CleanupEligibilityStatus::eligible(id(ID_1)).unwrap();
        let eligible = serde_json::to_value(eligible).unwrap();
        assert_eq!(eligible["eligible"], true);
        assert_eq!(eligible["archiveId"], ID_1);
        assert_eq!(eligible["blockerCodes"], json!([]));

        let contract = schema::<CleanupEligibilityStatus>();
        audit_json_schema(&contract).unwrap();
        assert_eq!(contract["oneOf"].as_array().map(Vec::len), Some(3));
        let mut illegal_blocker = eligible.clone();
        illegal_blocker["blockerCodes"] = json!(["cleanupNotAllowed"]);
        assert!(!schema_accepts::<CleanupEligibilityStatus>(
            &illegal_blocker
        ));
        let mut substituted_digest = eligible;
        substituted_digest["eligibilityDigest"] = json!(A);
        assert!(schema_accepts::<CleanupEligibilityStatus>(
            &substituted_digest
        ));
        assert!(!CleanupEligibilityStatus::validates_json(
            &substituted_digest
        ));

        CleanupEligibilityStatus::ineligible_without_archive(vec![
            StableErrorCode::CleanupNotAllowed,
            StableErrorCode::OperationInProgress,
        ])
        .unwrap();
        assert!(CleanupEligibilityStatus::ineligible_without_archive(vec![
            StableErrorCode::OperationInProgress,
            StableErrorCode::CleanupNotAllowed,
        ])
        .is_err());
        assert!(CleanupEligibilityStatus::ineligible_with_archive(id(ID_1), Vec::new()).is_err());
    }

    #[test]
    fn recent_operations_are_bounded_canonical_and_have_closed_terminal_kinds() {
        let completed = RecentOperationStatus::new_test_only(
            operation_id(ID_1),
            selector(),
            RecentTerminalKind::Completed,
            digest(A),
        );
        let stopped = RecentOperationStatus::new_test_only(
            operation_id(ID_2),
            selector(),
            RecentTerminalKind::Stopped,
            digest(B),
        );
        let recent = RecentOperations::new(vec![completed.clone(), stopped.clone()]).unwrap();
        let encoded = serde_json::to_value(recent).unwrap();
        assert_eq!(encoded[0]["terminalKind"], "completed");
        assert_eq!(encoded[1]["terminalKind"], "stopped");

        audit_json_schema(&schema::<RecentOperationStatus>()).unwrap();
        audit_json_schema(&schema::<RecentOperations>()).unwrap();
        let mut unknown = serde_json::to_value(completed).unwrap();
        unknown["terminalKind"] = json!("terminal");
        assert!(!schema_accepts::<RecentOperationStatus>(&unknown));
        assert!(RecentOperations::new(vec![stopped.clone(), stopped]).is_err());
    }

    #[test]
    fn basic_resume_handles_are_closed_and_path_free() {
        let artifact = ArtifactResumeHandle::new(
            id(ID_1),
            ArtifactRole::OrdinaryResult,
            ArtifactKind::OrdinaryConfiguration,
            digest(A),
            Some(id(ID_2)),
        );
        let workspace = WorkspaceResumeHandle::new(id(ID_1));
        let resolution_workspace =
            MergeResolutionWorkspaceResumeHandle::new(id(ID_1), id(ID_2), digest(A));
        let checkpoint =
            CheckpointResumeHandle::new(id(ID_1), CheckpointScope::Synchronized, digest(A));
        let comparison = ComparisonResumeHandle::new(
            id(ID_1),
            ComparisonScope::ProjectDelta,
            ComparisonStatusAnchor::TaskCurrent,
            ComparisonStatusAnchor::Artifact(id(ID_2)),
            digest(A),
        );
        let recovery = RecoveryResumeHandle::new(operation_id(ID_1), digest(A));
        let archive = ArchiveResumeHandle::new(
            id(ID_1),
            digest(A),
            TaskArchiveOutcome::Abandoned,
            digest(B),
        );

        for value in [
            serde_json::to_value(artifact).unwrap(),
            serde_json::to_value(workspace).unwrap(),
            serde_json::to_value(resolution_workspace).unwrap(),
            serde_json::to_value(checkpoint).unwrap(),
            serde_json::to_value(comparison).unwrap(),
            serde_json::to_value(recovery).unwrap(),
            serde_json::to_value(archive).unwrap(),
        ] {
            assert!(value.get("path").is_none());
            assert!(value.get("cwd").is_none());
        }

        audit_json_schema(&schema::<ArtifactResumeHandle>()).unwrap();
        audit_json_schema(&schema::<WorkspaceResumeHandle>()).unwrap();
        audit_json_schema(&schema::<MergeResolutionWorkspaceResumeHandle>()).unwrap();
        audit_json_schema(&schema::<CheckpointResumeHandle>()).unwrap();
        audit_json_schema(&schema::<ComparisonResumeHandle>()).unwrap();
        audit_json_schema(&schema::<RecoveryResumeHandle>()).unwrap();
        audit_json_schema(&schema::<ArchiveResumeHandle>()).unwrap();

        let mut invalid_scope = serde_json::to_value(CheckpointResumeHandle::new(
            id(ID_1),
            CheckpointScope::Local,
            digest(A),
        ))
        .unwrap();
        invalid_scope["scope"] = json!("main");
        assert!(!schema_accepts::<CheckpointResumeHandle>(&invalid_scope));
    }

    #[test]
    fn task12_archive_resume_handle_retains_final_hash_and_lineage_digest() {
        let archive =
            TaskArchiveStatus::new(id(ID_1), TaskArchiveOutcome::Success, digest(A), digest(B));
        let handle = ArchiveResumeHandle::from_archive_status(&archive);
        let encoded = serde_json::to_value(&handle).unwrap();
        assert_eq!(encoded["sha256"], A);
        assert_eq!(encoded["retainedLineageDigest"], B);
        assert_eq!(handle.retained_lineage_digest(), &digest(B));

        assert_eq!(
            archive.retained_lineage_digest(),
            handle.retained_lineage_digest()
        );
    }

    #[test]
    fn merge_resume_handles_encode_exact_cross_field_matrices() {
        let main_session = json!({
            "handleKind": "mergeSession",
            "sessionId": ID_1,
            "mode": "mainIntegration",
            "checkpointId": ID_2,
            "comparisonId": "33333333-3333-4333-8333-333333333333",
            "supportGateId": "44444444-4444-4444-8444-444444444444",
            "supportGateDigest": A,
            "baseSessionDigest": B,
            "supportGateHistoryEvidenceDigest": A,
            "decisionSetDigest": B,
            "conflictCount": 0
        });
        assert!(schema_accepts::<MergeSessionResumeHandle>(&main_session));
        let mut session_splice = main_session;
        session_splice["incomingDistributionId"] = json!(ID_1);
        assert!(!schema_accepts::<MergeSessionResumeHandle>(&session_splice));

        let current_decision = json!({
            "handleKind": "decision",
            "decisionId": ID_1,
            "decisionKind": "mergeConflict",
            "sessionId": ID_2,
            "baseSessionDigest": A,
            "conflictId": "33333333-3333-4333-8333-333333333333",
            "resolution": "manual",
            "rationaleDigest": B,
            "decisionDigest": A,
            "revisedDecisionSetDigest": B,
            "current": true
        });
        assert!(schema_accepts::<DecisionResumeHandle>(&current_decision));
        let mut invalid_current = current_decision;
        invalid_current["replacedByDecisionId"] = json!(ID_1);
        assert!(!schema_accepts::<DecisionResumeHandle>(&invalid_current));
        let mut invalid_adaptation = json!({
            "handleKind": "decision",
            "decisionId": ID_1,
            "decisionKind": "adaptation",
            "verificationId": ID_2,
            "adaptationDecisionDigest": A
        });
        assert!(schema_accepts::<DecisionResumeHandle>(&invalid_adaptation));
        invalid_adaptation["sessionId"] = json!(ID_1);
        assert!(!schema_accepts::<DecisionResumeHandle>(&invalid_adaptation));

        let selectable_receipt = json!({
            "handleKind": "resolutionChangeReceipt",
            "changeReceiptId": ID_1,
            "affectedTarget": {
                "targetKind": "metadataProperty",
                "objectId": ID_2,
                "propertyPath": "Attributes.Name"
            },
            "afterSha256": A,
            "changeReceiptDigest": B,
            "supersededChangeReceiptIds": [],
            "supersededDecisionIds": [],
            "decisionSetDigestBefore": A,
            "revisedDecisionSetDigest": B,
            "phaseTransition": {
                "phaseBefore": "synchronizationConflicts",
                "resultingPhase": "synchronizationConflicts"
            },
            "baseSessionDigest": A,
            "workspaceGenerationId": ID_2,
            "receiptSequence": 1,
            "consumed": false,
            "selectable": true
        });
        assert!(schema_accepts::<ResolutionChangeReceiptResumeHandle>(
            &selectable_receipt
        ));
        let mut invalid_superseded = selectable_receipt;
        invalid_superseded["supersededByReceiptId"] = json!(ID_2);
        assert!(!schema_accepts::<ResolutionChangeReceiptResumeHandle>(
            &invalid_superseded
        ));

        let unexpected = json!({
            "handleKind": "verification",
            "verificationId": ID_1,
            "scope": "synchronizedTask",
            "sessionId": ID_2,
            "outcome": "unexpected",
            "verificationDigest": A,
            "canonicalDeltaDigest": B,
            "differenceManifestId": "33333333-3333-4333-8333-333333333333",
            "differenceDigest": A
        });
        assert!(schema_accepts::<VerificationResumeHandle>(&unexpected));
        let mut missing_difference = unexpected;
        missing_difference
            .as_object_mut()
            .unwrap()
            .remove("differenceDigest");
        assert!(!schema_accepts::<VerificationResumeHandle>(
            &missing_difference
        ));

        let task_apply = json!({
            "handleKind": "mergeApply",
            "mergeReceiptId": ID_1,
            "target": "task",
            "sessionId": ID_2,
            "resolvedSessionDigest": A,
            "resultFingerprint": B,
            "sourcePublicationId": "33333333-3333-4333-8333-333333333333",
            "sourceFingerprint": A,
            "taskInfobaseFingerprint": B
        });
        assert!(schema_accepts::<MergeApplyResumeHandle>(&task_apply));
        let mut apply_splice = task_apply;
        apply_splice["rollbackCheckpointId"] = json!(ID_1);
        assert!(!schema_accepts::<MergeApplyResumeHandle>(&apply_splice));

        for contract in [
            schema::<MergeSessionResumeHandle>(),
            schema::<DecisionResumeHandle>(),
            schema::<ResolutionChangeReceiptResumeHandle>(),
            schema::<VerificationResumeHandle>(),
            schema::<MergeApplyResumeHandle>(),
            schema::<LockPlanResumeHandle>(),
            schema::<LockSetResumeHandle>(),
        ] {
            audit_json_schema(&contract).unwrap();
        }
    }

    #[test]
    fn preview_resume_handle_binds_tool_to_an_exact_normalized_request() {
        let armed_cancellation = json!({
            "handleKind": "preview",
            "toolName": "unica.repository.update",
            "previewOperationId": ID_1,
            "previewDigest": A,
            "request": {
                "mode": "supportPrerequisiteCancellation",
                "expectedStatusDigest": B,
                "supportActionId": ID_2,
                "expectedSupportActionDigest": A,
                "expectedArmingReceiptId": "33333333-3333-4333-8333-333333333333",
                "expectedArmingReceiptDigest": B,
                "reason": "operator cancelled the prerequisite"
            }
        });
        assert!(schema_accepts::<PreviewResumeHandle>(&armed_cancellation));
        let mut partial_arming = armed_cancellation;
        partial_arming["request"]
            .as_object_mut()
            .unwrap()
            .remove("expectedArmingReceiptDigest");
        assert!(!schema_accepts::<PreviewResumeHandle>(&partial_arming));

        let archive_success = json!({
            "handleKind": "preview",
            "toolName": "unica.branched.archive",
            "previewOperationId": ID_1,
            "previewDigest": A,
            "request": { "outcome": "success" }
        });
        assert!(schema_accepts::<PreviewResumeHandle>(&archive_success));
        let mut archive_splice = archive_success;
        archive_splice["request"]["reason"] = json!("not legal for success");
        assert!(!schema_accepts::<PreviewResumeHandle>(&archive_splice));

        let mut wrong_tool = json!({
            "handleKind": "preview",
            "toolName": "unica.branched.cleanup",
            "previewOperationId": ID_1,
            "previewDigest": A,
            "request": { "archiveId": ID_2 }
        });
        assert!(schema_accepts::<PreviewResumeHandle>(&wrong_tool));
        wrong_tool["toolName"] = json!("unica.delivery.deploy");
        assert!(!schema_accepts::<PreviewResumeHandle>(&wrong_tool));

        let contract = schema::<PreviewResumeHandle>();
        audit_json_schema(&contract).unwrap();
        assert_eq!(contract["oneOf"].as_array().map(Vec::len), Some(8));
    }

    #[test]
    fn support_gate_and_authorization_resume_schemas_are_exact_output_projections() {
        let preflight = schema::<SupportPreflightResumeHandle>();
        audit_json_schema(&preflight).unwrap();
        assert_eq!(preflight["oneOf"].as_array().map(Vec::len), Some(2));

        let authorization = schema::<SupportActionAuthorizationResumeHandle>();
        audit_json_schema(&authorization).unwrap();
        let encoded = serde_json::to_string(&authorization).unwrap();
        assert!(encoded.contains("supportActionAuthorization"));
        assert!(!encoded.contains("\"const\":\"consumed\""));
        assert!(!encoded.contains("\"const\":\"cancelled\""));
    }

    #[test]
    fn support_terminal_resume_handles_enforce_mode_and_disposition_presence() {
        let separate_prerequisite = json!({
            "handleKind": "supportPrerequisite",
            "receiptId": ID_1,
            "priorSupportActionId": ID_2,
            "priorSupportGateId": "33333333-3333-4333-8333-333333333333",
            "purpose": "mainIntegrationPrerequisite",
            "armingReceiptId": "44444444-4444-4444-8444-444444444444",
            "armingReceiptDigest": A,
            "repositoryVersion": "version-10",
            "manualTargetMode": "separateWorkingInfobase",
            "authorizedTransitionsDigest": B,
            "rootDeltaDigest": A,
            "rootLockProofDigest": B,
            "historyFromCursor": {"throughVersion":"version-9","historyPrefixDigest":A},
            "historyThroughCursor": {"throughVersion":"version-10","historyPrefixDigest":B},
            "historyPartitionDigest": A,
            "selectiveUpdateProofDigest": B,
            "postReleaseObservedHistoryCursor": {"throughVersion":"version-10","historyPrefixDigest":B},
            "postApplyHistoryPartitionDigest": A,
            "manualWorkingInfobaseClosureProofDigest": B,
            "resultingPhase": "synchronized"
        });
        assert!(schema_accepts::<SupportPrerequisiteResumeHandle>(
            &separate_prerequisite
        ));
        let mut wrong_mode_proof = separate_prerequisite;
        wrong_mode_proof["reservedOriginalTerminalizationProofDigest"] = json!(A);
        assert!(!schema_accepts::<SupportPrerequisiteResumeHandle>(
            &wrong_mode_proof
        ));

        let abandon_recovery = json!({
            "handleKind": "supportRecovery",
            "receiptId": ID_1,
            "priorSupportActionId": ID_2,
            "armingReceiptId": "33333333-3333-4333-8333-333333333333",
            "armingReceiptDigest": A,
            "disposition": "restoreThenAbandon",
            "manualTargetMode": "reservedOriginal",
            "successfulIntegrationForbidden": true,
            "historyFromCursor": {"throughVersion":"version-9","historyPrefixDigest":A},
            "historyThroughCursor": {"throughVersion":"version-10","historyPrefixDigest":B},
            "postReleaseObservedHistoryCursor": {"throughVersion":"version-10","historyPrefixDigest":B},
            "postReleaseHistoryPartitionDigest": A,
            "supportVersionObservationDigest": B,
            "supportRecoveryFinalizationPlanDigest": A,
            "supportRecoveryGuardProofDigest": B,
            "reservedOriginalTerminalizationProofDigest": A,
            "resultingPhase": "abandonmentReady"
        });
        assert!(schema_accepts::<SupportRecoveryResumeHandle>(
            &abandon_recovery
        ));
        let mut missing_forbidden = abandon_recovery;
        missing_forbidden
            .as_object_mut()
            .unwrap()
            .remove("successfulIntegrationForbidden");
        assert!(!schema_accepts::<SupportRecoveryResumeHandle>(
            &missing_forbidden
        ));

        let prerequisite_schema = schema::<SupportPrerequisiteResumeHandle>();
        let recovery_schema = schema::<SupportRecoveryResumeHandle>();
        audit_json_schema(&prerequisite_schema).unwrap();
        audit_json_schema(&recovery_schema).unwrap();
        assert_eq!(
            prerequisite_schema["oneOf"].as_array().map(Vec::len),
            Some(2)
        );
        assert_eq!(recovery_schema["oneOf"].as_array().map(Vec::len), Some(6));
    }

    #[test]
    fn support_cancellation_resume_handle_closes_arming_mode_and_prearm_recovery() {
        let separate_unarmed = json!({
            "handleKind": "supportCancellation",
            "receiptId": ID_1,
            "receiptDigest": A,
            "priorSupportActionId": ID_2,
            "purpose": "mainIntegrationPrerequisite",
            "reason": "cancel before arm",
            "manualTargetMode": "separateWorkingInfobase",
            "rootLockProofDigest": B,
            "historyFromCursor": {"throughVersion":"version-9","historyPrefixDigest":A},
            "historyThroughCursor": {"throughVersion":"version-10","historyPrefixDigest":B},
            "historyPartitionDigest": A,
            "preservedExternalSupportDigest": B,
            "selectiveUpdateProofDigest": A,
            "postReleaseObservedHistoryCursor": {"throughVersion":"version-10","historyPrefixDigest":B},
            "postApplyHistoryPartitionDigest": A,
            "manualWorkingInfobaseClosureProofDigest": B,
            "resultingPhase": "synchronized"
        });
        assert!(schema_accepts::<SupportCancellationResumeHandle>(
            &separate_unarmed
        ));
        let mut partial_arming = separate_unarmed.clone();
        partial_arming["armingReceiptId"] = json!(ID_1);
        assert!(!schema_accepts::<SupportCancellationResumeHandle>(
            &partial_arming
        ));
        let mut wrong_mode = separate_unarmed;
        wrong_mode["reservedOriginalTerminalizationProofDigest"] = json!(A);
        assert!(!schema_accepts::<SupportCancellationResumeHandle>(
            &wrong_mode
        ));

        let contract = schema::<SupportCancellationResumeHandle>();
        audit_json_schema(&contract).unwrap();
        assert_eq!(contract["oneOf"].as_array().map(Vec::len), Some(6));
        let encoded = serde_json::to_string(&contract).unwrap();
        assert!(encoded.contains("preArmCancellationCompletedProgress"));
        assert!(encoded.contains("\"const\":\"completed\""));
        assert!(CompletedPreArmCancellationProgress::new(
            PreArmCancellationFinalizationAttemptProgress::not_started_test_only(id(ID_1))
        )
        .is_err());
        audit_json_schema(&schema::<CompletedPreArmCancellationProgress>()).unwrap();
    }

    #[test]
    fn resume_handle_is_an_exact_twenty_one_leaf_tagged_union() {
        let artifact = serde_json::to_value(ArtifactResumeHandle::new(
            id(ID_1),
            ArtifactRole::OrdinaryResult,
            ArtifactKind::OrdinaryConfiguration,
            digest(A),
            None,
        ))
        .unwrap();
        assert!(schema_accepts::<ResumeHandle>(&artifact));

        let mut cross_leaf = artifact.clone();
        cross_leaf["handleKind"] = json!("archive");
        assert!(!schema_accepts::<ResumeHandle>(&cross_leaf));
        let mut unknown = artifact;
        unknown["handleKind"] = json!("artifactStatus");
        assert!(!schema_accepts::<ResumeHandle>(&unknown));

        let contract = schema::<ResumeHandle>();
        audit_json_schema(&contract).unwrap();
        assert_eq!(contract["oneOf"].as_array().map(Vec::len), Some(21));
    }

    #[test]
    fn resume_handles_are_canonical_and_duplicate_free_by_typed_identity() {
        let artifact_a = ResumeHandle::Artifact(ArtifactResumeHandle::new(
            id(ID_1),
            ArtifactRole::OrdinaryResult,
            ArtifactKind::OrdinaryConfiguration,
            digest(A),
            None,
        ));
        let artifact_same_id = ResumeHandle::Artifact(ArtifactResumeHandle::new(
            id(ID_1),
            ArtifactRole::OrdinaryResult,
            ArtifactKind::OrdinaryConfiguration,
            digest(B),
            Some(id(ID_2)),
        ));
        assert!(ResumeHandles::new(vec![artifact_a.clone(), artifact_same_id]).is_err());

        let workspace = ResumeHandle::Workspace(WorkspaceResumeHandle::new(id(ID_2)));
        ResumeHandles::new(vec![artifact_a.clone(), workspace.clone()]).unwrap();
        assert!(ResumeHandles::new(vec![workspace, artifact_a]).is_err());
        audit_json_schema(&schema::<ResumeHandles>()).unwrap();
    }

    #[test]
    fn named_status_arrays_reject_duplicate_semantic_identities() {
        let root_target: RepositoryTargetIdentity =
            serde_json::from_value(json!({"targetKind": "configurationRoot"})).unwrap();
        let object_target: RepositoryTargetIdentity = serde_json::from_value(json!({
            "targetKind": "developmentObject",
            "objectId": "00000000-0000-0000-0000-000000000001"
        }))
        .unwrap();
        let lock_owner: RepositoryOwnerIdentity = serde_json::from_value(json!({
            "username": "repo-user",
            "computer": null,
            "infobase": null,
            "lockedAt": null
        }))
        .unwrap();
        let root_lock =
            OwnedLockStatus::new(root_target.clone(), lock_owner.clone(), id(ID_1), digest(A));
        let object_lock =
            OwnedLockStatus::new(object_target, lock_owner.clone(), id(ID_2), digest(B));
        OwnedLockStatuses::new(vec![root_lock.clone(), object_lock]).unwrap();
        assert!(OwnedLockStatuses::new(vec![root_lock.clone(), root_lock.clone()]).is_err());
        assert!(OwnedLockStatuses::new(vec![
            OwnedLockStatus::new(
                serde_json::from_value(json!({
                    "targetKind": "developmentObject",
                    "objectId": "00000000-0000-0000-0000-000000000001"
                }))
                .unwrap(),
                lock_owner,
                id(ID_2),
                digest(B),
            ),
            root_lock,
        ])
        .is_err());

        let pending =
            PendingDecisionStatus::adaptation(id(ID_1), Vec::new(), 0, digest(A)).unwrap();
        let duplicate_pending =
            PendingDecisionStatus::adaptation(id(ID_1), vec![id(ID_2)], 0, digest(B)).unwrap();
        assert!(PendingDecisionStatuses::new(vec![pending, duplicate_pending]).is_err());

        let task_anchor = TaskAnchorStatus::task_fingerprint(digest(A), digest(B));
        let duplicate_anchor = TaskAnchorStatus::task_fingerprint(digest(B), digest(A));
        assert!(TaskAnchorStatuses::new(vec![task_anchor, duplicate_anchor]).is_err());

        let gate_2 = ValidationGateStatus::new(
            ValidationGateKind::Support,
            id(ID_2),
            digest(A),
            ValidationGateState::Current,
        );
        let gate_1 = ValidationGateStatus::new(
            ValidationGateKind::Checkpoint,
            id(ID_1),
            digest(B),
            ValidationGateState::Consumed,
        );
        assert!(ValidationGateStatuses::new(vec![gate_2, gate_1]).is_err());

        let artifact = ArtifactHashStatus::new(
            id(ID_1),
            ArtifactRole::OrdinaryResult,
            ArtifactKind::OrdinaryConfiguration,
            digest(A),
        );
        let duplicate_artifact = ArtifactHashStatus::new(
            id(ID_1),
            ArtifactRole::OrdinaryResult,
            ArtifactKind::OrdinaryConfiguration,
            digest(B),
        );
        assert!(ArtifactHashStatuses::new(vec![artifact, duplicate_artifact]).is_err());

        for contract in [
            schema::<OwnedLockStatuses>(),
            schema::<PendingDecisionStatuses>(),
            schema::<TaskAnchorStatuses>(),
            schema::<ValidationGateStatuses>(),
            schema::<ArtifactHashStatuses>(),
        ] {
            audit_json_schema(&contract).unwrap();
        }
    }

    #[test]
    fn existing_task_status_is_the_exact_phase_presence_union() {
        fn common(phase: &str, eligibility: &CleanupEligibilityStatus) -> Value {
            json!({
                "exists": true,
                "instanceId": ID_1,
                "phase": phase,
                "pendingDecisions": [],
                "anchors": [],
                "ownedLocks": [],
                "validationGates": [],
                "artifactHashes": [],
                "resumeHandles": [],
                "recentOperations": [],
                "cleanupEligibility": eligibility,
            })
        }

        let no_archive = CleanupEligibilityStatus::ineligible_without_archive(vec![
            StableErrorCode::CleanupNotAllowed,
        ])
        .unwrap();
        let with_archive = CleanupEligibilityStatus::eligible(id(ID_2)).unwrap();

        let created = common("created", &no_archive);
        assert!(schema_accepts::<ExistingTaskStatusData>(&created));
        let mut created_with_workspace = created;
        created_with_workspace["taskWorkspaceId"] = json!(ID_2);
        assert!(!schema_accepts::<ExistingTaskStatusData>(
            &created_with_workspace
        ));

        let mut workspace = common("baselineReady", &no_archive);
        assert!(!schema_accepts::<ExistingTaskStatusData>(&workspace));
        workspace["taskWorkspaceId"] = json!(ID_2);
        assert!(schema_accepts::<ExistingTaskStatusData>(&workspace));

        let repository_recovery = json!({
            "priorOperationId": ID_2,
            "target": "repositoryCommit",
            "effectClass": "reconcileOnly",
            "repositoryCommitStage": "committed",
            "plannedResultPhase": "committedAndUnlocked",
            "observations": [],
            "remainingUnknowns": [],
            "actions": [],
            "recoveryDigest": A,
        });
        let mut recovery = common("recoveryRequired", &no_archive);
        recovery["recovery"] = repository_recovery.clone();
        assert!(!schema_accepts::<ExistingTaskStatusData>(&recovery));
        recovery["taskWorkspaceId"] = json!(ID_2);
        assert!(schema_accepts::<ExistingTaskStatusData>(&recovery));

        let pre_workspace_plan =
            RecoveryPlanStatus::task_configuration_fixture_test_only(operation_id(ID_2)).unwrap();
        let mut pre_workspace_recovery = common("recoveryRequired", &no_archive);
        pre_workspace_recovery["recovery"] = serde_json::to_value(pre_workspace_plan).unwrap();
        assert!(schema_accepts::<ExistingTaskStatusData>(
            &pre_workspace_recovery
        ));
        let mut pre_workspace_with_archive = pre_workspace_recovery;
        pre_workspace_with_archive["archive"] = json!({
            "archiveId": ID_2,
            "outcome": "success",
            "sha256": A,
            "retainedLineageDigest": B,
        });
        assert!(!schema_accepts::<ExistingTaskStatusData>(
            &pre_workspace_with_archive
        ));

        let archive =
            TaskArchiveStatus::new(id(ID_2), TaskArchiveOutcome::Success, digest(A), digest(B));
        let archive_json = serde_json::to_value(archive.clone()).unwrap();
        let mut archived = common("archivedSuccess", &with_archive);
        archived["taskWorkspaceId"] = json!(ID_2);
        archived["archive"] = archive_json.clone();
        assert!(schema_accepts::<ExistingTaskStatusData>(&archived));
        let mut archived_without_workspace = archived.clone();
        archived_without_workspace
            .as_object_mut()
            .unwrap()
            .remove("taskWorkspaceId");
        assert!(!schema_accepts::<ExistingTaskStatusData>(
            &archived_without_workspace
        ));

        let cleanup_target = serde_json::from_value(json!({
            "projectId": ID_1,
            "instanceId": ID_1,
            "role": "quarantine"
        }))
        .unwrap();
        let cleanup_plan = RecoveryPlanStatus::cleanup_fixture_test_only(
            operation_id(ID_2),
            id(ID_2),
            cleanup_target,
            TaskPhase::CleanedSuccess,
        )
        .unwrap();
        let mut archived_cleanup_recovery = common("recoveryRequired", &with_archive);
        archived_cleanup_recovery["archive"] = archive_json;
        archived_cleanup_recovery["recovery"] = serde_json::to_value(cleanup_plan).unwrap();
        assert!(schema_accepts::<ExistingTaskStatusData>(
            &archived_cleanup_recovery
        ));
        let mut cleanup_with_workspace = archived_cleanup_recovery.clone();
        cleanup_with_workspace["taskWorkspaceId"] = json!(ID_2);
        assert!(!schema_accepts::<ExistingTaskStatusData>(
            &cleanup_with_workspace
        ));
        let mut archived_non_cleanup_recovery = archived_cleanup_recovery;
        archived_non_cleanup_recovery["recovery"] = repository_recovery;
        assert!(!schema_accepts::<ExistingTaskStatusData>(
            &archived_non_cleanup_recovery
        ));

        let cleanup_receipt = CleanupReceipt::test_only(CleanupReceiptTestParts {
            cleanup_receipt_id: id("33333333-3333-4333-8333-333333333333"),
            operation_id: operation_id("44444444-4444-4444-8444-444444444444"),
            archive_id: id(ID_2),
            approved_preview_digest: digest(A),
            owned_targets: Vec::new(),
            quarantine_id: id("55555555-5555-4555-8555-555555555555"),
            absent_observation_digests: Vec::new(),
            resulting_phase: CleanupResultPhase::CleanedSuccess,
        })
        .unwrap();
        let mut cleaned = common("cleanedSuccess", &with_archive);
        cleaned["archive"] = serde_json::to_value(archive).unwrap();
        cleaned["cleanupReceipt"] = serde_json::to_value(cleanup_receipt).unwrap();
        assert!(schema_accepts::<ExistingTaskStatusData>(&cleaned));
        let mut cleaned_with_workspace = cleaned.clone();
        cleaned_with_workspace["taskWorkspaceId"] = json!(ID_2);
        assert!(!schema_accepts::<ExistingTaskStatusData>(
            &cleaned_with_workspace
        ));
        let mut mismatched_phase = cleaned;
        mismatched_phase["phase"] = json!("cleanedAbandoned");
        assert!(!schema_accepts::<ExistingTaskStatusData>(&mismatched_phase));

        let contract = schema::<ExistingTaskStatusData>();
        audit_json_schema(&contract).unwrap();
        assert_eq!(ExistingTaskStatusData::schema_name(), "TaskStatusData");
        assert_eq!(contract["oneOf"].as_array().map(Vec::len), Some(9));
    }

    #[test]
    fn existing_task_status_constructors_reject_cross_phase_splices() {
        let no_archive = || {
            CleanupEligibilityStatus::ineligible_without_archive(vec![
                StableErrorCode::CleanupNotAllowed,
            ])
            .unwrap()
        };
        let created = ExistingTaskStatusData::pre_workspace(
            TaskPhase::Created,
            empty_status_authority(no_archive()),
        )
        .unwrap();
        assert_eq!(created.phase(), TaskPhase::Created);
        assert!(ExistingTaskStatusData::pre_workspace(
            TaskPhase::BaselineReady,
            empty_status_authority(no_archive()),
        )
        .is_err());
        assert!(ExistingTaskStatusData::workspace(
            TaskPhase::BaselineReady,
            id(ID_2),
            empty_status_authority(no_archive()),
        )
        .is_err());
        assert!(ExistingTaskStatusData::workspace(
            TaskPhase::BaselineReady,
            id(ID_2),
            status_authority(
                None,
                vec![ResumeHandle::Workspace(WorkspaceResumeHandle::new(id(
                    ID_2
                )))],
                ExistingTaskDeferredState::no_terminal_support(),
                no_archive(),
            )
            .unwrap(),
        )
        .is_ok());

        let archive =
            TaskArchiveStatus::new(id(ID_2), TaskArchiveOutcome::Success, digest(A), digest(B));
        assert!(ExistingTaskStatusData::archived(
            id(ID_1),
            archive.clone(),
            empty_status_authority(no_archive()),
        )
        .is_err());

        let success_receipt = CleanupReceipt::test_only(CleanupReceiptTestParts {
            cleanup_receipt_id: id("33333333-3333-4333-8333-333333333333"),
            operation_id: operation_id("44444444-4444-4444-8444-444444444444"),
            archive_id: id(ID_2),
            approved_preview_digest: digest(A),
            owned_targets: Vec::new(),
            quarantine_id: id("55555555-5555-4555-8555-555555555555"),
            absent_observation_digests: Vec::new(),
            resulting_phase: CleanupResultPhase::CleanedSuccess,
        })
        .unwrap();
        let cleaned = ExistingTaskStatusData::cleaned(
            archive.clone(),
            success_receipt,
            status_authority(
                None,
                vec![ResumeHandle::Archive(ArchiveResumeHandle::new(
                    id(ID_2),
                    digest(A),
                    TaskArchiveOutcome::Success,
                    digest(B),
                ))],
                ExistingTaskDeferredState::no_terminal_support(),
                CleanupEligibilityStatus::eligible(id(ID_2)).unwrap(),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(cleaned.phase(), TaskPhase::CleanedSuccess);

        let abandoned_receipt = CleanupReceipt::test_only(CleanupReceiptTestParts {
            cleanup_receipt_id: id("33333333-3333-4333-8333-333333333333"),
            operation_id: operation_id("44444444-4444-4444-8444-444444444444"),
            archive_id: id(ID_2),
            approved_preview_digest: digest(A),
            owned_targets: Vec::new(),
            quarantine_id: id("55555555-5555-4555-8555-555555555555"),
            absent_observation_digests: Vec::new(),
            resulting_phase: CleanupResultPhase::CleanedAbandoned,
        })
        .unwrap();
        assert!(ExistingTaskStatusData::cleaned(
            archive,
            abandoned_receipt,
            status_authority(
                None,
                vec![ResumeHandle::Archive(ArchiveResumeHandle::new(
                    id(ID_2),
                    digest(A),
                    TaskArchiveOutcome::Success,
                    digest(B),
                ))],
                ExistingTaskDeferredState::no_terminal_support(),
                CleanupEligibilityStatus::eligible(id(ID_2)).unwrap(),
            )
            .unwrap(),
        )
        .is_err());
    }

    #[test]
    fn existing_task_recovery_binds_context_operation_and_resume_handle() {
        let no_archive = || {
            CleanupEligibilityStatus::ineligible_without_archive(vec![
                StableErrorCode::CleanupNotAllowed,
            ])
            .unwrap()
        };
        let prior_operation_id = operation_id(ID_2);
        let plan =
            RecoveryPlanStatus::task_configuration_fixture_test_only(prior_operation_id.clone())
                .unwrap();
        let recovery_handle = || {
            ResumeHandle::Recovery(RecoveryResumeHandle::new(
                prior_operation_id.clone(),
                plan.recovery_digest().clone(),
            ))
        };
        let authority = status_authority(
            None,
            vec![recovery_handle()],
            ExistingTaskDeferredState::no_terminal_support(),
            no_archive(),
        )
        .unwrap();
        assert!(ExistingTaskStatusData::pre_workspace_recovery(
            TaskPhase::RecoveryRequired,
            plan.clone(),
            authority,
        )
        .is_ok());

        assert!(ExistingTaskStatusData::pre_workspace_recovery(
            TaskPhase::RecoveryRequired,
            plan.clone(),
            empty_status_authority(no_archive()),
        )
        .is_err());
        let wrong_prior = status_authority(
            None,
            vec![ResumeHandle::Recovery(RecoveryResumeHandle::new(
                operation_id("33333333-3333-4333-8333-333333333333"),
                plan.recovery_digest().clone(),
            ))],
            ExistingTaskDeferredState::no_terminal_support(),
            no_archive(),
        )
        .unwrap();
        assert!(ExistingTaskStatusData::pre_workspace_recovery(
            TaskPhase::RecoveryRequired,
            plan.clone(),
            wrong_prior,
        )
        .is_err());
        let wrong_digest = status_authority(
            None,
            vec![ResumeHandle::Recovery(RecoveryResumeHandle::new(
                prior_operation_id.clone(),
                digest(B),
            ))],
            ExistingTaskDeferredState::no_terminal_support(),
            no_archive(),
        )
        .unwrap();
        assert!(ExistingTaskStatusData::pre_workspace_recovery(
            TaskPhase::RecoveryRequired,
            plan.clone(),
            wrong_digest,
        )
        .is_err());

        let exact_effect_unknown = ActiveOperationStatus::effect_unknown_test_only(
            prior_operation_id.clone(),
            selector(),
            DurableExecutionPolicy::JournaledEffect,
            digest(A),
            instant("2026-07-22T00:00:00Z"),
            plan.recovery_digest().clone(),
        )
        .unwrap();
        let authority = status_authority(
            Some(exact_effect_unknown),
            vec![recovery_handle()],
            ExistingTaskDeferredState::no_terminal_support(),
            no_archive(),
        )
        .unwrap();
        assert!(ExistingTaskStatusData::pre_workspace_recovery(
            TaskPhase::RecoveryRequired,
            plan.clone(),
            authority,
        )
        .is_ok());
        let substituted_effect_unknown = ActiveOperationStatus::effect_unknown_test_only(
            prior_operation_id.clone(),
            selector(),
            DurableExecutionPolicy::JournaledEffect,
            digest(A),
            instant("2026-07-22T00:00:00Z"),
            digest(B),
        )
        .unwrap();
        let authority = status_authority(
            Some(substituted_effect_unknown),
            vec![recovery_handle()],
            ExistingTaskDeferredState::no_terminal_support(),
            no_archive(),
        )
        .unwrap();
        assert!(ExistingTaskStatusData::pre_workspace_recovery(
            TaskPhase::RecoveryRequired,
            plan.clone(),
            authority,
        )
        .is_err());

        let registered = ActiveOperationStatus::registered_test_only(
            operation_id("33333333-3333-4333-8333-333333333333"),
            selector(),
            DurableExecutionPolicy::JournaledEffect,
            digest(A),
            instant("2026-07-22T00:00:00Z"),
            lease(),
            ActiveOperationOwnerState::Live,
        )
        .unwrap();
        let authority = status_authority(
            Some(registered),
            vec![recovery_handle()],
            ExistingTaskDeferredState::no_terminal_support(),
            no_archive(),
        )
        .unwrap();
        assert!(ExistingTaskStatusData::pre_workspace_recovery(
            TaskPhase::RecoveryRequired,
            plan,
            authority,
        )
        .is_ok());
    }

    #[test]
    fn existing_task_recovery_context_closes_workspace_and_archived_cleanup_targets() {
        let no_archive = || {
            CleanupEligibilityStatus::ineligible_without_archive(vec![
                StableErrorCode::CleanupNotAllowed,
            ])
            .unwrap()
        };
        let prior_operation_id = operation_id(ID_2);
        let commit_plan = RecoveryPlanStatus::repository_commit_committed_test_only(
            prior_operation_id.clone(),
            Vec::new(),
            Vec::new(),
        )
        .unwrap();
        let commit_recovery_handle = || {
            ResumeHandle::Recovery(RecoveryResumeHandle::new(
                prior_operation_id.clone(),
                commit_plan.recovery_digest().clone(),
            ))
        };
        let pre_workspace_authority = status_authority(
            None,
            vec![commit_recovery_handle()],
            ExistingTaskDeferredState::no_terminal_support(),
            no_archive(),
        )
        .unwrap();
        assert!(ExistingTaskStatusData::pre_workspace_recovery(
            TaskPhase::RecoveryRequired,
            commit_plan.clone(),
            pre_workspace_authority,
        )
        .is_err());
        let workspace_authority = status_authority(
            None,
            vec![
                ResumeHandle::Workspace(WorkspaceResumeHandle::new(id(ID_1))),
                commit_recovery_handle(),
            ],
            ExistingTaskDeferredState::no_terminal_support(),
            no_archive(),
        )
        .unwrap();
        assert!(ExistingTaskStatusData::workspace_recovery(
            TaskPhase::CommittedUnverified,
            id(ID_1),
            commit_plan,
            workspace_authority,
        )
        .is_ok());

        let archive =
            TaskArchiveStatus::new(id(ID_2), TaskArchiveOutcome::Success, digest(A), digest(B));
        let owned_target = serde_json::from_value(json!({
            "projectId": ID_1,
            "instanceId": ID_1,
            "role": "quarantine"
        }))
        .unwrap();
        let cleanup_plan = RecoveryPlanStatus::cleanup_fixture_test_only(
            prior_operation_id.clone(),
            id(ID_2),
            owned_target,
            TaskPhase::CleanedSuccess,
        )
        .unwrap();
        let archived_authority = status_authority(
            None,
            vec![
                ResumeHandle::Recovery(RecoveryResumeHandle::new(
                    prior_operation_id,
                    cleanup_plan.recovery_digest().clone(),
                )),
                ResumeHandle::Archive(ArchiveResumeHandle::new(
                    id(ID_2),
                    digest(A),
                    TaskArchiveOutcome::Success,
                    digest(B),
                )),
            ],
            ExistingTaskDeferredState::no_terminal_support(),
            CleanupEligibilityStatus::eligible(id(ID_2)).unwrap(),
        )
        .unwrap();
        assert!(ExistingTaskStatusData::archived_cleanup_recovery(
            TaskPhase::RecoveryRequired,
            cleanup_plan,
            archive,
            archived_authority,
        )
        .is_ok());

        let assert_cleanup_binding_rejected =
            |plan_archive_id: UnicaId, planned_result_phase: TaskPhase| {
                let plan = RecoveryPlanStatus::cleanup_fixture_test_only(
                    operation_id(ID_2),
                    plan_archive_id,
                    serde_json::from_value(json!({
                        "projectId": ID_1,
                        "instanceId": ID_1,
                        "role": "quarantine"
                    }))
                    .unwrap(),
                    planned_result_phase,
                )
                .unwrap();
                let authority = status_authority(
                    None,
                    vec![
                        ResumeHandle::Recovery(RecoveryResumeHandle::new(
                            operation_id(ID_2),
                            plan.recovery_digest().clone(),
                        )),
                        ResumeHandle::Archive(ArchiveResumeHandle::new(
                            id(ID_2),
                            digest(A),
                            TaskArchiveOutcome::Success,
                            digest(B),
                        )),
                    ],
                    ExistingTaskDeferredState::no_terminal_support(),
                    CleanupEligibilityStatus::eligible(id(ID_2)).unwrap(),
                )
                .unwrap();
                assert!(ExistingTaskStatusData::archived_cleanup_recovery(
                    TaskPhase::RecoveryRequired,
                    plan,
                    TaskArchiveStatus::new(
                        id(ID_2),
                        TaskArchiveOutcome::Success,
                        digest(A),
                        digest(B),
                    ),
                    authority,
                )
                .is_err());
            };
        assert_cleanup_binding_rejected(
            id("77777777-7777-4777-8777-777777777777"),
            TaskPhase::CleanedSuccess,
        );
        assert_cleanup_binding_rejected(id(ID_2), TaskPhase::CleanedAbandoned);
    }

    #[test]
    fn existing_task_deferred_state_binds_latest_terminal_producer_and_xor_consumption() {
        let receipt_id = id(ID_2);
        let no_archive = || {
            CleanupEligibilityStatus::ineligible_without_archive(vec![
                StableErrorCode::CleanupNotAllowed,
            ])
            .unwrap()
        };
        let consumption = DeferredRepositoryAdvanceConsumptionReceipt::new(
            id("66666666-6666-4666-8666-666666666666"),
            receipt_id.clone(),
            digest(B),
            id("77777777-7777-4777-8777-777777777777"),
            digest(A),
            TaskPhase::Synchronized,
        )
        .unwrap();
        assert!(status_authority(
            None,
            vec![terminal_support_handle(receipt_id.clone(), Some(digest(B)),)],
            ExistingTaskDeferredState::latest_terminal(receipt_id.clone(), Some(consumption),),
            no_archive(),
        )
        .is_ok());

        let wrong_producer = DeferredRepositoryAdvanceConsumptionReceipt::new(
            id("66666666-6666-4666-8666-666666666666"),
            id("88888888-8888-4888-8888-888888888888"),
            digest(B),
            id("77777777-7777-4777-8777-777777777777"),
            digest(A),
            TaskPhase::Synchronized,
        )
        .unwrap();
        assert!(status_authority(
            None,
            vec![terminal_support_handle(receipt_id.clone(), Some(digest(B)),)],
            ExistingTaskDeferredState::latest_terminal(receipt_id.clone(), Some(wrong_producer),),
            no_archive(),
        )
        .is_err());
        assert!(status_authority(
            None,
            vec![terminal_support_handle(receipt_id.clone(), Some(digest(B)),)],
            ExistingTaskDeferredState::latest_terminal(receipt_id.clone(), None),
            no_archive(),
        )
        .is_err());
        assert!(status_authority(
            None,
            vec![terminal_support_handle(receipt_id, None)],
            ExistingTaskDeferredState::no_terminal_support(),
            no_archive(),
        )
        .is_err());
    }

    #[test]
    fn production_deferred_state_authority_uses_exact_typed_handles_and_receipts() {
        let empty_handles = ResumeHandles::new(Vec::new()).unwrap();
        let no_terminal =
            ExistingTaskDeferredState::without_terminal_support(&empty_handles).unwrap();
        let status_authority = ExistingTaskStatusAuthority::new(
            id(ID_1),
            None,
            ExistingTaskStatusCollections::new(
                PendingDecisionStatuses::new(Vec::new()).unwrap(),
                TaskAnchorStatuses::new(Vec::new()).unwrap(),
                OwnedLockStatuses::new(Vec::new()).unwrap(),
                ValidationGateStatuses::new(Vec::new()).unwrap(),
                ArtifactHashStatuses::new(Vec::new()).unwrap(),
                empty_handles,
                RecentOperations::new(Vec::new()).unwrap(),
            ),
            no_terminal,
            CleanupEligibilityStatus::ineligible_without_archive(vec![
                StableErrorCode::CleanupNotAllowed,
            ])
            .unwrap(),
        )
        .unwrap();
        assert!(
            ExistingTaskStatusData::pre_workspace(TaskPhase::Created, status_authority).is_ok()
        );

        assert!(
            LatestTerminalSupportAuthority::from_handle_test_only(&ResumeHandle::Workspace(
                WorkspaceResumeHandle::new(id(ID_1))
            ),)
            .is_err()
        );

        let terminal_without_advance = terminal_support_handle(id(ID_2), None);
        let latest_without_advance =
            LatestTerminalSupportAuthority::from_handle_test_only(&terminal_without_advance)
                .unwrap();
        let terminal_without_advance_handles =
            ResumeHandles::new(vec![terminal_without_advance]).unwrap();
        assert!(ExistingTaskDeferredState::latest_terminal_current(
            &terminal_without_advance_handles,
            latest_without_advance,
        )
        .is_ok());
        assert!(ExistingTaskDeferredState::without_terminal_support(
            &terminal_without_advance_handles,
        )
        .is_err());

        let terminal_with_advance = terminal_support_handle(id(ID_2), Some(digest(B)));
        let latest_with_advance =
            LatestTerminalSupportAuthority::from_handle_test_only(&terminal_with_advance).unwrap();
        let terminal_with_advance_handles =
            ResumeHandles::new(vec![terminal_with_advance]).unwrap();
        let consumption = DeferredRepositoryAdvanceConsumptionReceipt::new(
            id("66666666-6666-4666-8666-666666666666"),
            id(ID_2),
            digest(B),
            id("77777777-7777-4777-8777-777777777777"),
            digest(A),
            TaskPhase::Synchronized,
        )
        .unwrap();
        assert!(ExistingTaskDeferredState::latest_terminal_consumed(
            &terminal_with_advance_handles,
            latest_with_advance,
            consumption,
        )
        .is_ok());

        let missing_current_authority = LatestTerminalSupportAuthority::from_handle_test_only(
            &terminal_support_handle(id(ID_2), Some(digest(B))),
        )
        .unwrap();
        assert!(ExistingTaskDeferredState::latest_terminal_current(
            &terminal_with_advance_handles,
            missing_current_authority,
        )
        .is_err());

        let wrong_consumption_authority = LatestTerminalSupportAuthority::from_handle_test_only(
            &terminal_support_handle(id(ID_2), Some(digest(B))),
        )
        .unwrap();
        assert!(ExistingTaskDeferredState::latest_terminal_consumed(
            &terminal_with_advance_handles,
            wrong_consumption_authority,
            DeferredRepositoryAdvanceConsumptionReceipt::new(
                id("66666666-6666-4666-8666-666666666666"),
                id(ID_2),
                digest(A),
                id("77777777-7777-4777-8777-777777777777"),
                digest(A),
                TaskPhase::Synchronized,
            )
            .unwrap(),
        )
        .is_err());

        let authority_source = terminal_support_handle(id(ID_2), Some(digest(A)));
        let substituted_authority =
            LatestTerminalSupportAuthority::from_handle_test_only(&authority_source).unwrap();
        assert!(ExistingTaskDeferredState::latest_terminal_consumed(
            &terminal_with_advance_handles,
            substituted_authority,
            DeferredRepositoryAdvanceConsumptionReceipt::new(
                id("66666666-6666-4666-8666-666666666666"),
                id(ID_2),
                digest(B),
                id("77777777-7777-4777-8777-777777777777"),
                digest(A),
                TaskPhase::Synchronized,
            )
            .unwrap(),
        )
        .is_err());

        let foreign_source =
            terminal_support_handle(id("88888888-8888-4888-8888-888888888888"), Some(digest(B)));
        let foreign_authority =
            LatestTerminalSupportAuthority::from_handle_test_only(&foreign_source).unwrap();
        assert!(ExistingTaskDeferredState::latest_terminal_consumed(
            &terminal_with_advance_handles,
            foreign_authority,
            DeferredRepositoryAdvanceConsumptionReceipt::new(
                id("66666666-6666-4666-8666-666666666666"),
                id(ID_2),
                digest(B),
                id("77777777-7777-4777-8777-777777777777"),
                digest(A),
                TaskPhase::Synchronized,
            )
            .unwrap(),
        )
        .is_err());
    }

    #[test]
    fn archived_status_rejects_live_work_and_mutable_handles() {
        let archive =
            TaskArchiveStatus::new(id(ID_2), TaskArchiveOutcome::Success, digest(A), digest(B));
        let eligibility = || CleanupEligibilityStatus::eligible(id(ID_2)).unwrap();
        let archive_handles = || {
            vec![
                ResumeHandle::Workspace(WorkspaceResumeHandle::new(id(ID_1))),
                ResumeHandle::Archive(ArchiveResumeHandle::new(
                    id(ID_2),
                    digest(A),
                    TaskArchiveOutcome::Success,
                    digest(B),
                )),
            ]
        };
        let pending =
            PendingDecisionStatus::adaptation(id(ID_1), Vec::new(), 0, digest(A)).unwrap();
        let authority = status_authority_with_work(
            None,
            vec![pending],
            Vec::new(),
            archive_handles(),
            ExistingTaskDeferredState::no_terminal_support(),
            eligibility(),
        )
        .unwrap();
        assert!(ExistingTaskStatusData::archived(id(ID_1), archive.clone(), authority).is_err());

        let target: RepositoryTargetIdentity =
            serde_json::from_value(json!({"targetKind": "configurationRoot"})).unwrap();
        let owner: RepositoryOwnerIdentity = serde_json::from_value(json!({
            "username": "repo-user",
            "computer": null,
            "infobase": null,
            "lockedAt": null
        }))
        .unwrap();
        let authority = status_authority_with_work(
            None,
            Vec::new(),
            vec![OwnedLockStatus::new(target, owner, id(ID_1), digest(A))],
            archive_handles(),
            ExistingTaskDeferredState::no_terminal_support(),
            eligibility(),
        )
        .unwrap();
        assert!(ExistingTaskStatusData::archived(id(ID_1), archive.clone(), authority).is_err());

        let registered = ActiveOperationStatus::registered_test_only(
            operation_id("33333333-3333-4333-8333-333333333333"),
            selector(),
            DurableExecutionPolicy::JournaledEffect,
            digest(A),
            instant("2026-07-22T00:00:00Z"),
            lease(),
            ActiveOperationOwnerState::Live,
        )
        .unwrap();
        let authority = status_authority(
            Some(registered),
            archive_handles(),
            ExistingTaskDeferredState::no_terminal_support(),
            eligibility(),
        )
        .unwrap();
        assert!(ExistingTaskStatusData::archived(id(ID_1), archive.clone(), authority).is_err());

        let authority = status_authority(
            None,
            vec![
                ResumeHandle::Artifact(ArtifactResumeHandle::new(
                    id("33333333-3333-4333-8333-333333333333"),
                    ArtifactRole::OrdinaryResult,
                    ArtifactKind::OrdinaryConfiguration,
                    digest(A),
                    None,
                )),
                ResumeHandle::Workspace(WorkspaceResumeHandle::new(id(ID_1))),
                ResumeHandle::Archive(ArchiveResumeHandle::new(
                    id(ID_2),
                    digest(A),
                    TaskArchiveOutcome::Success,
                    digest(B),
                )),
            ],
            ExistingTaskDeferredState::no_terminal_support(),
            eligibility(),
        )
        .unwrap();
        assert!(ExistingTaskStatusData::archived(id(ID_1), archive, authority).is_err());
    }

    #[test]
    fn archived_status_retains_historical_audit_handles_but_rejects_current_handles() {
        let archive =
            || TaskArchiveStatus::new(id(ID_2), TaskArchiveOutcome::Success, digest(A), digest(B));
        let eligibility = || CleanupEligibilityStatus::eligible(id(ID_2)).unwrap();
        let handles = |decision| {
            vec![
                ResumeHandle::Workspace(WorkspaceResumeHandle::new(id(ID_1))),
                decision,
                ResumeHandle::Archive(ArchiveResumeHandle::new(
                    id(ID_2),
                    digest(A),
                    TaskArchiveOutcome::Success,
                    digest(B),
                )),
            ]
        };

        let historical = status_authority(
            None,
            handles(historical_decision_handle()),
            ExistingTaskDeferredState::no_terminal_support(),
            eligibility(),
        )
        .unwrap();
        assert!(ExistingTaskStatusData::archived(id(ID_1), archive(), historical).is_ok());

        let current = status_authority(
            None,
            handles(current_decision_handle()),
            ExistingTaskDeferredState::no_terminal_support(),
            eligibility(),
        )
        .unwrap();
        assert!(ExistingTaskStatusData::archived(id(ID_1), archive(), current).is_err());
    }
}
