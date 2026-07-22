use crate::domain::branched_development::canonical_json::{
    canonical_contract_digest, contract_digest_record_sealed, ContractDigestRecord,
};
use crate::domain::branched_development::contracts::artifacts::{
    ArchiveEntryName, ArchiveSchemaVersion, OriginalInfobaseKind, OwnedTargetLocator,
    OwnedTargetRole, RepositoryTransport, SafeResultCount,
};
use crate::domain::branched_development::contracts::errors::{
    OperationInProgressContext, ProjectDigestProfileStateContext,
    RepositoryAccountReservationBusyContext, TargetReservationBusyContext,
};
#[cfg(test)]
use crate::domain::branched_development::contracts::prearm_recovery::PreArmCancellationArchiveLineageProjection;
use crate::domain::branched_development::contracts::prearm_recovery::{
    PreArmCancellationEffectObservation, PreArmCancellationFinalizationAttemptProgress,
    PreArmCancellationFinalizationPlan, PreArmCancellationFinalizationRecheckEvidence,
};
#[cfg(test)]
use crate::domain::branched_development::contracts::recovery::ValidatedPreArmArchiveReceiptOutcomeWitness;
use crate::domain::branched_development::contracts::recovery::{
    ArchivePublicationByteObservation, ArchiveStagingReceipt, HandoffRetentionReleaseReceipts,
    PublishedArchiveSha256,
};
use crate::domain::branched_development::contracts::repository::{
    DeferredRepositoryAdvance, RepositoryHistoryCursor, SelectiveRepositoryUpdateProof,
    ValidatedRepositoryHistoryPartition,
};
use crate::domain::branched_development::contracts::scalars::{
    Comment, LocalProfileName, RepositoryUsername,
};
use crate::domain::branched_development::contracts::schema::one_of_schema;
use crate::domain::branched_development::contracts::status::{
    AbandonedCleanupReceipt, CleanupReceiptAuthority, ExistingTaskStatusData,
    SuccessCleanupReceipt, TaskArchiveOutcome, TaskArchiveStatus,
};
use crate::domain::branched_development::contracts::support::ManualWorkingInfobaseIdentity;
use crate::domain::branched_development::{
    CapabilityRowId, ProjectId, Sha256Digest, TaskPhase, UnicaId,
};
use schemars::{json_schema, JsonSchema, Schema, SchemaGenerator};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::borrow::Cow;
use std::collections::BTreeSet;
use std::fmt;

const MAX_RESULT_ITEMS: usize = 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TaskResultContractError(&'static str);

impl fmt::Display for TaskResultContractError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.0)
    }
}

impl std::error::Error for TaskResultContractError {}

fn task_digest<T: ContractDigestRecord>(
    record: &T,
    message: &'static str,
) -> Result<Sha256Digest, TaskResultContractError> {
    canonical_contract_digest(record, None).map_err(|_| TaskResultContractError(message))
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
            fn inline_schema() -> bool {
                true
            }

            fn schema_name() -> Cow<'static, str> {
                stringify!($name).into()
            }

            fn json_schema(_: &mut SchemaGenerator) -> Schema {
                json_schema!({"type": "boolean", "const": $value})
            }
        }
    };
}

wire_literal!(OperationInProgressCode, "operationInProgress");
wire_literal!(TargetReservationBusyCode, "targetReservationBusy");
wire_literal!(
    RepositoryAccountReservationBusyCode,
    "repositoryAccountReservationBusy"
);
wire_literal!(ProjectIdentityCollisionCode, "projectIdentityCollision");
wire_literal!(
    StateRootRelocationRequiredCode,
    "stateRootRelocationRequired"
);
wire_literal!(ReservedOriginalMode, "reservedOriginal");
wire_literal!(SeparateWorkingInfobaseMode, "separateWorkingInfobase");
wire_literal!(SuccessOutcome, "success");
wire_literal!(AbandonedOutcome, "abandoned");
bool_literal!(FalseLiteral, false);
bool_literal!(TrueLiteral, true);

macro_rules! blocker_leaf {
    ($name:ident, $code:ty, $context:ty) => {
        #[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        pub(crate) struct $name {
            code: $code,
            context: $context,
        }

        impl $name {
            const fn new(context: $context) -> Self {
                Self {
                    code: <$code>::Value,
                    context,
                }
            }
        }
    };
}

blocker_leaf!(
    OperationInProgressBlocker,
    OperationInProgressCode,
    OperationInProgressContext
);
blocker_leaf!(
    TargetReservationBusyBlocker,
    TargetReservationBusyCode,
    TargetReservationBusyContext
);
blocker_leaf!(
    RepositoryAccountReservationBusyBlocker,
    RepositoryAccountReservationBusyCode,
    RepositoryAccountReservationBusyContext
);
blocker_leaf!(
    ProjectIdentityCollisionBlocker,
    ProjectIdentityCollisionCode,
    ProjectDigestProfileStateContext
);
blocker_leaf!(
    StateRootRelocationRequiredBlocker,
    StateRootRelocationRequiredCode,
    ProjectDigestProfileStateContext
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub(crate) enum NotCreatedBlocker {
    OperationInProgress(OperationInProgressBlocker),
    TargetReservationBusy(TargetReservationBusyBlocker),
    RepositoryAccountReservationBusy(RepositoryAccountReservationBusyBlocker),
    ProjectIdentityCollision(ProjectIdentityCollisionBlocker),
    StateRootRelocationRequired(StateRootRelocationRequiredBlocker),
}

impl NotCreatedBlocker {
    pub(crate) const fn operation_in_progress(context: OperationInProgressContext) -> Self {
        Self::OperationInProgress(OperationInProgressBlocker::new(context))
    }

    pub(crate) const fn target_reservation_busy(context: TargetReservationBusyContext) -> Self {
        Self::TargetReservationBusy(TargetReservationBusyBlocker::new(context))
    }

    pub(crate) const fn repository_account_reservation_busy(
        context: RepositoryAccountReservationBusyContext,
    ) -> Self {
        Self::RepositoryAccountReservationBusy(RepositoryAccountReservationBusyBlocker::new(
            context,
        ))
    }

    pub(crate) const fn project_identity_collision(
        context: ProjectDigestProfileStateContext,
    ) -> Self {
        Self::ProjectIdentityCollision(ProjectIdentityCollisionBlocker::new(context))
    }

    pub(crate) const fn state_root_relocation_required(
        context: ProjectDigestProfileStateContext,
    ) -> Self {
        Self::StateRootRelocationRequired(StateRootRelocationRequiredBlocker::new(context))
    }

    const fn rank(&self) -> u8 {
        match self {
            Self::OperationInProgress(_) => 0,
            Self::TargetReservationBusy(_) => 1,
            Self::RepositoryAccountReservationBusy(_) => 2,
            Self::ProjectIdentityCollision(_) => 3,
            Self::StateRootRelocationRequired(_) => 4,
        }
    }
}

impl JsonSchema for NotCreatedBlocker {
    fn schema_name() -> Cow<'static, str> {
        "NotCreatedBlocker".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<OperationInProgressBlocker>(),
            generator.subschema_for::<TargetReservationBusyBlocker>(),
            generator.subschema_for::<RepositoryAccountReservationBusyBlocker>(),
            generator.subschema_for::<ProjectIdentityCollisionBlocker>(),
            generator.subschema_for::<StateRootRelocationRequiredBlocker>(),
        ])
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct NotCreatedBlockers(Vec<NotCreatedBlocker>);

impl NotCreatedBlockers {
    pub(crate) fn new(values: Vec<NotCreatedBlocker>) -> Result<Self, TaskResultContractError> {
        if values.len() > 5
            || values
                .windows(2)
                .any(|pair| pair[0].rank() >= pair[1].rank())
        {
            return Err(TaskResultContractError(
                "not-created blockers must be unique and in contract order",
            ));
        }
        Ok(Self(values))
    }

    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl JsonSchema for NotCreatedBlockers {
    fn schema_name() -> Cow<'static, str> {
        "NotCreatedBlockers".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "array",
            "items": generator.subschema_for::<NotCreatedBlocker>(),
            "minItems": 0,
            "maxItems": 5,
            "uniqueItems": true
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
struct NonEmptyNotCreatedBlockers(NotCreatedBlockers);

impl JsonSchema for NonEmptyNotCreatedBlockers {
    fn schema_name() -> Cow<'static, str> {
        "NonEmptyNotCreatedBlockers".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "array",
            "items": generator.subschema_for::<NotCreatedBlocker>(),
            "minItems": 1,
            "maxItems": 5,
            "uniqueItems": true
        })
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct NotCreatedAuthority(NotCreatedBlockers);

impl NotCreatedAuthority {
    pub(crate) const fn from_coordinator(blockers: NotCreatedBlockers) -> Self {
        Self(blockers)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AllowedNotCreatedData {
    exists: FalseLiteral,
    start_allowed: TrueLiteral,
    blockers: EmptyNotCreatedBlockers,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct EmptyNotCreatedBlockers;

impl Serialize for EmptyNotCreatedBlockers {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let empty: [NotCreatedBlocker; 0] = [];
        empty.serialize(serializer)
    }
}

impl JsonSchema for EmptyNotCreatedBlockers {
    fn schema_name() -> Cow<'static, str> {
        "EmptyNotCreatedBlockers".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "array",
            "items": generator.subschema_for::<NotCreatedBlocker>(),
            "minItems": 0,
            "maxItems": 0,
            "uniqueItems": true
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BlockedNotCreatedData {
    exists: FalseLiteral,
    start_allowed: FalseLiteral,
    blockers: NonEmptyNotCreatedBlockers,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
enum NotCreatedDataKind {
    Allowed(AllowedNotCreatedData),
    Blocked(BlockedNotCreatedData),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct NotCreatedData(NotCreatedDataKind);

impl NotCreatedData {
    pub(crate) fn from_authority(authority: NotCreatedAuthority) -> Self {
        if authority.0.is_empty() {
            Self(NotCreatedDataKind::Allowed(AllowedNotCreatedData {
                exists: FalseLiteral,
                start_allowed: TrueLiteral,
                blockers: EmptyNotCreatedBlockers,
            }))
        } else {
            Self(NotCreatedDataKind::Blocked(BlockedNotCreatedData {
                exists: FalseLiteral,
                start_allowed: FalseLiteral,
                blockers: NonEmptyNotCreatedBlockers(authority.0),
            }))
        }
    }
}

impl JsonSchema for NotCreatedData {
    fn schema_name() -> Cow<'static, str> {
        "NotCreatedData".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<AllowedNotCreatedData>(),
            generator.subschema_for::<BlockedNotCreatedData>(),
        ])
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
struct CanonicalCapabilityRowIds(Vec<CapabilityRowId>);

impl CanonicalCapabilityRowIds {
    fn new(values: Vec<CapabilityRowId>) -> Result<Self, TaskResultContractError> {
        if values.len() > MAX_RESULT_ITEMS || values.windows(2).any(|pair| pair[0] >= pair[1]) {
            return Err(TaskResultContractError(
                "capability row IDs must be bounded, canonical, and unique",
            ));
        }
        Ok(Self(values))
    }
}

impl JsonSchema for CanonicalCapabilityRowIds {
    fn schema_name() -> Cow<'static, str> {
        "CanonicalCapabilityRowIds".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "array",
            "items": generator.subschema_for::<CapabilityRowId>(),
            "minItems": 0,
            "maxItems": MAX_RESULT_ITEMS,
            "uniqueItems": true
        })
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ReservedOriginalStartAuthority {
    instance_id: UnicaId,
    project_id: ProjectId,
    profile: LocalProfileName,
    original_infobase_kind: OriginalInfobaseKind,
    repository_transport: RepositoryTransport,
    capability_row_id: CapabilityRowId,
    pre_arm_cancellation_guard_capability_id: CapabilityRowId,
    retention_provider_capability_row_ids: CanonicalCapabilityRowIds,
    manual_actor_username: RepositoryUsername,
    work_root_locator: OwnedTargetLocator,
    commit_comment_preview: Comment,
    reserved_original_lease_capability_id: CapabilityRowId,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct SeparateWorkingInfobaseStartAuthority {
    instance_id: UnicaId,
    project_id: ProjectId,
    profile: LocalProfileName,
    original_infobase_kind: OriginalInfobaseKind,
    repository_transport: RepositoryTransport,
    capability_row_id: CapabilityRowId,
    pre_arm_cancellation_guard_capability_id: CapabilityRowId,
    retention_provider_capability_row_ids: CanonicalCapabilityRowIds,
    manual_actor_username: RepositoryUsername,
    work_root_locator: OwnedTargetLocator,
    commit_comment_preview: Comment,
    manual_working_infobase_identity: ManualWorkingInfobaseIdentity,
    manual_working_infobase_inspection_capability_id: CapabilityRowId,
}

#[cfg(test)]
pub(crate) struct ReservedOriginalStartAuthorityTestParts {
    pub(crate) instance_id: UnicaId,
    pub(crate) project_id: ProjectId,
    pub(crate) profile: LocalProfileName,
    pub(crate) original_infobase_kind: OriginalInfobaseKind,
    pub(crate) repository_transport: RepositoryTransport,
    pub(crate) capability_row_id: CapabilityRowId,
    pub(crate) pre_arm_cancellation_guard_capability_id: CapabilityRowId,
    pub(crate) retention_provider_capability_row_ids: Vec<CapabilityRowId>,
    pub(crate) profile_declares_recovery_distribution_sources: bool,
    pub(crate) manual_actor_username: RepositoryUsername,
    pub(crate) reserved_integration_username: RepositoryUsername,
    pub(crate) work_root_locator: OwnedTargetLocator,
    pub(crate) commit_comment_preview: Comment,
    pub(crate) reserved_original_lease_capability_id: CapabilityRowId,
}

#[cfg(test)]
pub(crate) struct SeparateWorkingInfobaseStartAuthorityTestParts {
    pub(crate) instance_id: UnicaId,
    pub(crate) project_id: ProjectId,
    pub(crate) profile: LocalProfileName,
    pub(crate) original_infobase_kind: OriginalInfobaseKind,
    pub(crate) repository_transport: RepositoryTransport,
    pub(crate) capability_row_id: CapabilityRowId,
    pub(crate) pre_arm_cancellation_guard_capability_id: CapabilityRowId,
    pub(crate) retention_provider_capability_row_ids: Vec<CapabilityRowId>,
    pub(crate) profile_declares_recovery_distribution_sources: bool,
    pub(crate) manual_actor_username: RepositoryUsername,
    pub(crate) work_root_locator: OwnedTargetLocator,
    pub(crate) commit_comment_preview: Comment,
    pub(crate) manual_working_infobase_identity: ManualWorkingInfobaseIdentity,
    pub(crate) manual_working_infobase_inspection_capability_id: CapabilityRowId,
}

fn validate_retention_provider_presence(
    values: &CanonicalCapabilityRowIds,
    profile_declares_sources: bool,
) -> Result<(), TaskResultContractError> {
    if values.0.is_empty() == profile_declares_sources {
        return Err(TaskResultContractError(
            "retention-provider capability rows disagree with profile recovery sources",
        ));
    }
    Ok(())
}

impl ReservedOriginalStartAuthority {
    #[cfg(test)]
    pub(crate) fn test_only(
        parts: ReservedOriginalStartAuthorityTestParts,
    ) -> Result<Self, TaskResultContractError> {
        let retention_provider_capability_row_ids =
            CanonicalCapabilityRowIds::new(parts.retention_provider_capability_row_ids)?;
        validate_retention_provider_presence(
            &retention_provider_capability_row_ids,
            parts.profile_declares_recovery_distribution_sources,
        )?;
        if parts.manual_actor_username != parts.reserved_integration_username {
            return Err(TaskResultContractError(
                "reserved-original manual actor must equal the reserved integration username",
            ));
        }
        Ok(Self {
            instance_id: parts.instance_id,
            project_id: parts.project_id,
            profile: parts.profile,
            original_infobase_kind: parts.original_infobase_kind,
            repository_transport: parts.repository_transport,
            capability_row_id: parts.capability_row_id,
            pre_arm_cancellation_guard_capability_id: parts
                .pre_arm_cancellation_guard_capability_id,
            retention_provider_capability_row_ids,
            manual_actor_username: parts.manual_actor_username,
            work_root_locator: parts.work_root_locator,
            commit_comment_preview: parts.commit_comment_preview,
            reserved_original_lease_capability_id: parts.reserved_original_lease_capability_id,
        })
    }
}

impl SeparateWorkingInfobaseStartAuthority {
    #[cfg(test)]
    pub(crate) fn test_only(
        parts: SeparateWorkingInfobaseStartAuthorityTestParts,
    ) -> Result<Self, TaskResultContractError> {
        let retention_provider_capability_row_ids =
            CanonicalCapabilityRowIds::new(parts.retention_provider_capability_row_ids)?;
        validate_retention_provider_presence(
            &retention_provider_capability_row_ids,
            parts.profile_declares_recovery_distribution_sources,
        )?;
        Ok(Self {
            instance_id: parts.instance_id,
            project_id: parts.project_id,
            profile: parts.profile,
            original_infobase_kind: parts.original_infobase_kind,
            repository_transport: parts.repository_transport,
            capability_row_id: parts.capability_row_id,
            pre_arm_cancellation_guard_capability_id: parts
                .pre_arm_cancellation_guard_capability_id,
            retention_provider_capability_row_ids,
            manual_actor_username: parts.manual_actor_username,
            work_root_locator: parts.work_root_locator,
            commit_comment_preview: parts.commit_comment_preview,
            manual_working_infobase_identity: parts.manual_working_infobase_identity,
            manual_working_infobase_inspection_capability_id: parts
                .manual_working_infobase_inspection_capability_id,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ReservedOriginalStartData {
    instance_id: UnicaId,
    project_id: ProjectId,
    profile: LocalProfileName,
    original_infobase_kind: OriginalInfobaseKind,
    repository_transport: RepositoryTransport,
    capability_row_id: CapabilityRowId,
    pre_arm_cancellation_guard_capability_id: CapabilityRowId,
    retention_provider_capability_row_ids: CanonicalCapabilityRowIds,
    manual_actor_username: RepositoryUsername,
    work_root_locator: OwnedTargetLocator,
    commit_comment_preview: Comment,
    manual_target_mode: ReservedOriginalMode,
    reserved_original_lease_capability_id: CapabilityRowId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SeparateWorkingInfobaseStartData {
    instance_id: UnicaId,
    project_id: ProjectId,
    profile: LocalProfileName,
    original_infobase_kind: OriginalInfobaseKind,
    repository_transport: RepositoryTransport,
    capability_row_id: CapabilityRowId,
    pre_arm_cancellation_guard_capability_id: CapabilityRowId,
    retention_provider_capability_row_ids: CanonicalCapabilityRowIds,
    manual_actor_username: RepositoryUsername,
    work_root_locator: OwnedTargetLocator,
    commit_comment_preview: Comment,
    manual_target_mode: SeparateWorkingInfobaseMode,
    manual_working_infobase_identity: ManualWorkingInfobaseIdentity,
    manual_working_infobase_inspection_capability_id: CapabilityRowId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub(crate) enum StartData {
    ReservedOriginal(ReservedOriginalStartData),
    SeparateWorkingInfobase(SeparateWorkingInfobaseStartData),
}

fn validate_start_root(
    project_id: &ProjectId,
    instance_id: &UnicaId,
    work_root_locator: &OwnedTargetLocator,
) -> Result<(), TaskResultContractError> {
    if work_root_locator.project_id() != project_id
        || work_root_locator.instance_id() != instance_id
        || work_root_locator.role() != OwnedTargetRole::InstanceRoot
    {
        return Err(TaskResultContractError(
            "start work-root locator must be the enclosing instanceRoot",
        ));
    }
    Ok(())
}

impl StartData {
    pub(crate) fn reserved_original(
        authority: ReservedOriginalStartAuthority,
    ) -> Result<Self, TaskResultContractError> {
        validate_start_root(
            &authority.project_id,
            &authority.instance_id,
            &authority.work_root_locator,
        )?;
        Ok(Self::ReservedOriginal(ReservedOriginalStartData {
            instance_id: authority.instance_id,
            project_id: authority.project_id,
            profile: authority.profile,
            original_infobase_kind: authority.original_infobase_kind,
            repository_transport: authority.repository_transport,
            capability_row_id: authority.capability_row_id,
            pre_arm_cancellation_guard_capability_id: authority
                .pre_arm_cancellation_guard_capability_id,
            retention_provider_capability_row_ids: authority.retention_provider_capability_row_ids,
            manual_actor_username: authority.manual_actor_username,
            work_root_locator: authority.work_root_locator,
            commit_comment_preview: authority.commit_comment_preview,
            manual_target_mode: ReservedOriginalMode::Value,
            reserved_original_lease_capability_id: authority.reserved_original_lease_capability_id,
        }))
    }

    pub(crate) fn separate_working_infobase(
        authority: SeparateWorkingInfobaseStartAuthority,
    ) -> Result<Self, TaskResultContractError> {
        validate_start_root(
            &authority.project_id,
            &authority.instance_id,
            &authority.work_root_locator,
        )?;
        Ok(Self::SeparateWorkingInfobase(
            SeparateWorkingInfobaseStartData {
                instance_id: authority.instance_id,
                project_id: authority.project_id,
                profile: authority.profile,
                original_infobase_kind: authority.original_infobase_kind,
                repository_transport: authority.repository_transport,
                capability_row_id: authority.capability_row_id,
                pre_arm_cancellation_guard_capability_id: authority
                    .pre_arm_cancellation_guard_capability_id,
                retention_provider_capability_row_ids: authority
                    .retention_provider_capability_row_ids,
                manual_actor_username: authority.manual_actor_username,
                work_root_locator: authority.work_root_locator,
                commit_comment_preview: authority.commit_comment_preview,
                manual_target_mode: SeparateWorkingInfobaseMode::Value,
                manual_working_infobase_identity: authority.manual_working_infobase_identity,
                manual_working_infobase_inspection_capability_id: authority
                    .manual_working_infobase_inspection_capability_id,
            },
        ))
    }
}

impl JsonSchema for StartData {
    fn schema_name() -> Cow<'static, str> {
        "StartData".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<ReservedOriginalStartData>(),
            generator.subschema_for::<SeparateWorkingInfobaseStartData>(),
        ])
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub(crate) enum BranchedStatusData {
    NotCreated(NotCreatedData),
    Existing(Box<ExistingTaskStatusData>),
}

impl JsonSchema for BranchedStatusData {
    fn schema_name() -> Cow<'static, str> {
        "BranchedStatusData".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<NotCreatedData>(),
            generator.subschema_for::<ExistingTaskStatusData>(),
        ])
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct CanonicalArchiveEntryNames(Vec<ArchiveEntryName>);

impl CanonicalArchiveEntryNames {
    fn new(values: Vec<ArchiveEntryName>) -> Result<Self, TaskResultContractError> {
        if values.len() > MAX_RESULT_ITEMS {
            return Err(TaskResultContractError(
                "archive entry-name list is oversized",
            ));
        }
        let mut previous: Option<&str> = None;
        let mut folded = BTreeSet::new();
        for value in &values {
            let name = value.as_str();
            let lower = name.to_ascii_lowercase();
            if previous.is_some_and(|previous| previous >= name)
                || !folded.insert(lower.clone())
                || lower == "archive-manifest.json"
                || lower.starts_with("handoff-releases/")
            {
                return Err(TaskResultContractError(
                    "archive entry names must be ASCII-ordered, case-fold unique, and outside reserved namespaces",
                ));
            }
            previous = Some(name);
        }
        Ok(Self(values))
    }

    pub(crate) fn as_slice(&self) -> &[ArchiveEntryName] {
        &self.0
    }
}

impl JsonSchema for CanonicalArchiveEntryNames {
    fn schema_name() -> Cow<'static, str> {
        "ArchiveEntryNames".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "array",
            "items": generator.subschema_for::<ArchiveEntryName>(),
            "minItems": 0,
            "maxItems": MAX_RESULT_ITEMS,
            "uniqueItems": true
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExcludedArchiveRoles;

impl ExcludedArchiveRoles {
    const ROLES: [OwnedTargetRole; 7] = [
        OwnedTargetRole::InstanceRoot,
        OwnedTargetRole::TaskInfobase,
        OwnedTargetRole::TaskWorkspace,
        OwnedTargetRole::Probe,
        OwnedTargetRole::Sandbox,
        OwnedTargetRole::Artifact,
        OwnedTargetRole::Quarantine,
    ];

    pub(crate) const fn as_slice(&self) -> &[OwnedTargetRole; 7] {
        &Self::ROLES
    }
}

impl Serialize for ExcludedArchiveRoles {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        Self::ROLES.serialize(serializer)
    }
}

impl JsonSchema for ExcludedArchiveRoles {
    fn schema_name() -> Cow<'static, str> {
        "ExcludedArchiveRoles".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "array",
            "minItems": 7,
            "maxItems": 7,
            "items": false,
            "prefixItems": [
                {"type": "string", "const": "instanceRoot"},
                {"type": "string", "const": "taskInfobase"},
                {"type": "string", "const": "taskWorkspace"},
                {"type": "string", "const": "probe"},
                {"type": "string", "const": "sandbox"},
                {"type": "string", "const": "artifact"},
                {"type": "string", "const": "quarantine"}
            ]
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ArchiveEligibilityDigestRecord {
    outcome: TaskArchiveOutcome,
    pre_preview_status: ExistingTaskStatusData,
    retained_entry_names: CanonicalArchiveEntryNames,
    excluded_roles: ExcludedArchiveRoles,
}

impl contract_digest_record_sealed::Sealed for ArchiveEligibilityDigestRecord {}
impl ContractDigestRecord for ArchiveEligibilityDigestRecord {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ArchivePreviewDigestRecord {
    outcome: TaskArchiveOutcome,
    retained_entry_names: CanonicalArchiveEntryNames,
    excluded_roles: ExcludedArchiveRoles,
    eligibility_digest: Sha256Digest,
}

impl contract_digest_record_sealed::Sealed for ArchivePreviewDigestRecord {}
impl ContractDigestRecord for ArchivePreviewDigestRecord {}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ArchivePreviewAuthority {
    outcome: TaskArchiveOutcome,
    pre_preview_status: ExistingTaskStatusData,
    retained_entry_names: CanonicalArchiveEntryNames,
}

impl ArchivePreviewAuthority {
    #[cfg(test)]
    fn test_only(
        outcome: TaskArchiveOutcome,
        pre_preview_status: ExistingTaskStatusData,
        retained_entry_names: Vec<ArchiveEntryName>,
    ) -> Result<Self, TaskResultContractError> {
        if outcome == TaskArchiveOutcome::Success
            && pre_preview_status.phase() != TaskPhase::CommittedAndUnlocked
        {
            return Err(TaskResultContractError(
                "success archive preview requires committedAndUnlocked status",
            ));
        }
        Ok(Self {
            outcome,
            pre_preview_status,
            retained_entry_names: CanonicalArchiveEntryNames::new(retained_entry_names)?,
        })
    }
}

macro_rules! archive_preview_leaf {
    ($name:ident, $outcome:ty) => {
        #[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        pub(crate) struct $name {
            outcome: $outcome,
            retained_entry_names: CanonicalArchiveEntryNames,
            excluded_roles: ExcludedArchiveRoles,
            eligibility_digest: Sha256Digest,
            preview_digest: Sha256Digest,
        }
    };
}

archive_preview_leaf!(SuccessArchivePreviewData, SuccessOutcome);
archive_preview_leaf!(AbandonedArchivePreviewData, AbandonedOutcome);

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub(crate) enum ArchivePreviewData {
    Success(SuccessArchivePreviewData),
    Abandoned(AbandonedArchivePreviewData),
}

impl ArchivePreviewData {
    pub(crate) fn from_authority(
        authority: ArchivePreviewAuthority,
    ) -> Result<Self, TaskResultContractError> {
        let eligibility_record = ArchiveEligibilityDigestRecord {
            outcome: authority.outcome,
            pre_preview_status: authority.pre_preview_status,
            retained_entry_names: authority.retained_entry_names.clone(),
            excluded_roles: ExcludedArchiveRoles,
        };
        let eligibility_digest = task_digest(
            &eligibility_record,
            "archive eligibility digest computation failed",
        )?;
        let preview_record = ArchivePreviewDigestRecord {
            outcome: authority.outcome,
            retained_entry_names: authority.retained_entry_names.clone(),
            excluded_roles: ExcludedArchiveRoles,
            eligibility_digest: eligibility_digest.clone(),
        };
        let preview_digest = task_digest(&preview_record, "archive preview digest failed")?;
        Ok(match authority.outcome {
            TaskArchiveOutcome::Success => Self::Success(SuccessArchivePreviewData {
                outcome: SuccessOutcome::Value,
                retained_entry_names: authority.retained_entry_names,
                excluded_roles: ExcludedArchiveRoles,
                eligibility_digest,
                preview_digest,
            }),
            TaskArchiveOutcome::Abandoned => Self::Abandoned(AbandonedArchivePreviewData {
                outcome: AbandonedOutcome::Value,
                retained_entry_names: authority.retained_entry_names,
                excluded_roles: ExcludedArchiveRoles,
                eligibility_digest,
                preview_digest,
            }),
        })
    }
}

impl JsonSchema for ArchivePreviewData {
    fn schema_name() -> Cow<'static, str> {
        "ArchivePreviewData".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<SuccessArchivePreviewData>(),
            generator.subschema_for::<AbandonedArchivePreviewData>(),
        ])
    }
}

/// Consuming preview approval. The production CAS/operation-journal mint is
/// added with the Task 16 terminal producer; Task 12 exposes no raw digest or
/// outcome constructor.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ApprovedArchivePreviewAuthority {
    outcome: TaskArchiveOutcome,
    retained_entry_names: CanonicalArchiveEntryNames,
    preview_digest: Sha256Digest,
}

impl ApprovedArchivePreviewAuthority {
    #[cfg(test)]
    pub(crate) fn approve_test_only(preview: ArchivePreviewData) -> Self {
        match preview {
            ArchivePreviewData::Success(value) => Self {
                outcome: TaskArchiveOutcome::Success,
                retained_entry_names: value.retained_entry_names,
                preview_digest: value.preview_digest,
            },
            ArchivePreviewData::Abandoned(value) => Self {
                outcome: TaskArchiveOutcome::Abandoned,
                retained_entry_names: value.retained_entry_names,
                preview_digest: value.preview_digest,
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct CanonicalOwnedTargets(Vec<OwnedTargetLocator>);

impl CanonicalOwnedTargets {
    fn new(values: Vec<OwnedTargetLocator>) -> Result<Self, TaskResultContractError> {
        if values.len() > MAX_RESULT_ITEMS || values.windows(2).any(|pair| pair[0] >= pair[1]) {
            return Err(TaskResultContractError(
                "owned targets must be bounded, canonical, and unique",
            ));
        }
        if let Some(first) = values.first() {
            if values.iter().any(|value| {
                value.project_id() != first.project_id()
                    || value.instance_id() != first.instance_id()
            }) {
                return Err(TaskResultContractError(
                    "cleanup owned targets must belong to one project instance",
                ));
            }
        }
        Ok(Self(values))
    }

    fn as_slice(&self) -> &[OwnedTargetLocator] {
        &self.0
    }

    fn roles(&self) -> Result<CanonicalOwnedTargetRoles, TaskResultContractError> {
        CanonicalOwnedTargetRoles::new(self.0.iter().map(OwnedTargetLocator::role).collect())
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
            "maxItems": MAX_RESULT_ITEMS,
            "uniqueItems": true
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct CanonicalOwnedTargetRoles(Vec<OwnedTargetRole>);

impl CanonicalOwnedTargetRoles {
    fn new(values: Vec<OwnedTargetRole>) -> Result<Self, TaskResultContractError> {
        if values.len() > 7 || values.windows(2).any(|pair| pair[0] >= pair[1]) {
            return Err(TaskResultContractError(
                "owned target roles must be canonical and unique",
            ));
        }
        Ok(Self(values))
    }
}

impl JsonSchema for CanonicalOwnedTargetRoles {
    fn schema_name() -> Cow<'static, str> {
        "CanonicalOwnedTargetRoles".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "array",
            "items": generator.subschema_for::<OwnedTargetRole>(),
            "minItems": 0,
            "maxItems": 7,
            "uniqueItems": true
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CleanupMarkerDigestRecord {
    archive_id: UnicaId,
    owned_targets: CanonicalOwnedTargets,
}

impl contract_digest_record_sealed::Sealed for CleanupMarkerDigestRecord {}
impl ContractDigestRecord for CleanupMarkerDigestRecord {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CleanupPreviewDigestRecord {
    archive_id: UnicaId,
    outcome: TaskArchiveOutcome,
    removable_roles: CanonicalOwnedTargetRoles,
    owned_targets: CanonicalOwnedTargets,
    marker_digest: Sha256Digest,
}

impl contract_digest_record_sealed::Sealed for CleanupPreviewDigestRecord {}
impl ContractDigestRecord for CleanupPreviewDigestRecord {}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct CleanupPreviewAuthority {
    archive_id: UnicaId,
    outcome: TaskArchiveOutcome,
    owned_targets: CanonicalOwnedTargets,
}

impl CleanupPreviewAuthority {
    #[cfg(test)]
    fn test_only(
        archive: &TaskArchiveStatus,
        owned_targets: Vec<OwnedTargetLocator>,
    ) -> Result<Self, TaskResultContractError> {
        Ok(Self {
            archive_id: archive.archive_id().clone(),
            outcome: archive.outcome(),
            owned_targets: CanonicalOwnedTargets::new(owned_targets)?,
        })
    }
}

macro_rules! cleanup_preview_leaf {
    ($name:ident, $outcome:ty) => {
        #[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        pub(crate) struct $name {
            archive_id: UnicaId,
            outcome: $outcome,
            removable_roles: CanonicalOwnedTargetRoles,
            owned_targets: CanonicalOwnedTargets,
            marker_digest: Sha256Digest,
            preview_digest: Sha256Digest,
        }
    };
}

cleanup_preview_leaf!(SuccessCleanupPreviewData, SuccessOutcome);
cleanup_preview_leaf!(AbandonedCleanupPreviewData, AbandonedOutcome);

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub(crate) enum CleanupPreviewData {
    Success(SuccessCleanupPreviewData),
    Abandoned(AbandonedCleanupPreviewData),
}

impl CleanupPreviewData {
    pub(crate) fn from_authority(
        authority: CleanupPreviewAuthority,
    ) -> Result<Self, TaskResultContractError> {
        let roles = authority.owned_targets.roles()?;
        let marker_digest = task_digest(
            &CleanupMarkerDigestRecord {
                archive_id: authority.archive_id.clone(),
                owned_targets: authority.owned_targets.clone(),
            },
            "cleanup marker digest failed",
        )?;
        let preview_digest = task_digest(
            &CleanupPreviewDigestRecord {
                archive_id: authority.archive_id.clone(),
                outcome: authority.outcome,
                removable_roles: roles.clone(),
                owned_targets: authority.owned_targets.clone(),
                marker_digest: marker_digest.clone(),
            },
            "cleanup preview digest failed",
        )?;
        Ok(match authority.outcome {
            TaskArchiveOutcome::Success => Self::Success(SuccessCleanupPreviewData {
                archive_id: authority.archive_id,
                outcome: SuccessOutcome::Value,
                removable_roles: roles,
                owned_targets: authority.owned_targets,
                marker_digest,
                preview_digest,
            }),
            TaskArchiveOutcome::Abandoned => Self::Abandoned(AbandonedCleanupPreviewData {
                archive_id: authority.archive_id,
                outcome: AbandonedOutcome::Value,
                removable_roles: roles,
                owned_targets: authority.owned_targets,
                marker_digest,
                preview_digest,
            }),
        })
    }
}

impl JsonSchema for CleanupPreviewData {
    fn schema_name() -> Cow<'static, str> {
        "CleanupPreviewData".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<SuccessCleanupPreviewData>(),
            generator.subschema_for::<AbandonedCleanupPreviewData>(),
        ])
    }
}

macro_rules! cleanup_data_leaf {
    ($name:ident, $outcome:ty, $receipt:ty) => {
        #[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        pub(crate) struct $name {
            quarantine_id: UnicaId,
            outcome: $outcome,
            removed_roles: CanonicalOwnedTargetRoles,
            retained_archive_id: UnicaId,
            marker_digest: Sha256Digest,
            absent_observation_digests: CanonicalDigests,
            cleanup_receipt: $receipt,
            preview_digest: Sha256Digest,
        }
    };
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct CanonicalDigests(Vec<Sha256Digest>);

impl CanonicalDigests {
    fn from_aligned(values: &[Sha256Digest]) -> Result<Self, TaskResultContractError> {
        if values.len() > MAX_RESULT_ITEMS {
            return Err(TaskResultContractError("digest projection is oversized"));
        }
        let mut seen = BTreeSet::new();
        if values.iter().any(|value| !seen.insert(value.as_str())) {
            return Err(TaskResultContractError(
                "aligned observation digests must be unique",
            ));
        }
        Ok(Self(values.to_vec()))
    }

    fn canonical(values: Vec<Sha256Digest>) -> Result<Self, TaskResultContractError> {
        if values.len() > MAX_RESULT_ITEMS || values.windows(2).any(|pair| pair[0] >= pair[1]) {
            return Err(TaskResultContractError(
                "digests must be bounded, canonical, and unique",
            ));
        }
        Ok(Self(values))
    }
}

impl JsonSchema for CanonicalDigests {
    fn schema_name() -> Cow<'static, str> {
        "CanonicalDigests".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "array",
            "items": generator.subschema_for::<Sha256Digest>(),
            "minItems": 0,
            "maxItems": MAX_RESULT_ITEMS,
            "uniqueItems": true
        })
    }
}

cleanup_data_leaf!(SuccessCleanupData, SuccessOutcome, SuccessCleanupReceipt);
cleanup_data_leaf!(
    AbandonedCleanupData,
    AbandonedOutcome,
    AbandonedCleanupReceipt
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub(crate) enum CleanupData {
    Success(SuccessCleanupData),
    Abandoned(AbandonedCleanupData),
}

impl CleanupData {
    pub(crate) fn from_receipt_authority(
        authority: CleanupReceiptAuthority,
        archive: &TaskArchiveStatus,
    ) -> Result<Self, TaskResultContractError> {
        if authority.outcome() != archive.outcome() {
            return Err(TaskResultContractError(
                "cleanup receipt outcome differs from retained archive",
            ));
        }
        let marker_digest = authority.marker_digest().clone();
        match authority.outcome() {
            TaskArchiveOutcome::Success => {
                let receipt = authority
                    .issue_success()
                    .map_err(|_| TaskResultContractError("success cleanup receipt failed"))?;
                validate_cleanup_receipt(&receipt, archive, &marker_digest)?;
                Ok(Self::Success(SuccessCleanupData {
                    quarantine_id: receipt.quarantine_id().clone(),
                    outcome: SuccessOutcome::Value,
                    removed_roles: roles_from_targets(receipt.owned_targets())?,
                    retained_archive_id: receipt.archive_id().clone(),
                    marker_digest,
                    absent_observation_digests: CanonicalDigests::from_aligned(
                        receipt.absent_observation_digests(),
                    )?,
                    preview_digest: receipt.approved_preview_digest().clone(),
                    cleanup_receipt: receipt,
                }))
            }
            TaskArchiveOutcome::Abandoned => {
                let receipt = authority
                    .issue_abandoned()
                    .map_err(|_| TaskResultContractError("abandoned cleanup receipt failed"))?;
                validate_cleanup_receipt(&receipt, archive, &marker_digest)?;
                Ok(Self::Abandoned(AbandonedCleanupData {
                    quarantine_id: receipt.quarantine_id().clone(),
                    outcome: AbandonedOutcome::Value,
                    removed_roles: roles_from_targets(receipt.owned_targets())?,
                    retained_archive_id: receipt.archive_id().clone(),
                    marker_digest,
                    absent_observation_digests: CanonicalDigests::from_aligned(
                        receipt.absent_observation_digests(),
                    )?,
                    preview_digest: receipt.approved_preview_digest().clone(),
                    cleanup_receipt: receipt,
                }))
            }
        }
    }
}

trait CleanupReceiptView {
    fn archive_id(&self) -> &UnicaId;
    fn owned_targets(&self) -> &[OwnedTargetLocator];
    fn quarantine_id(&self) -> &UnicaId;
    fn absent_observation_digests(&self) -> &[Sha256Digest];
    fn approved_preview_digest(&self) -> &Sha256Digest;
}

macro_rules! cleanup_receipt_view {
    ($type:ty) => {
        impl CleanupReceiptView for $type {
            fn archive_id(&self) -> &UnicaId {
                self.archive_id()
            }
            fn owned_targets(&self) -> &[OwnedTargetLocator] {
                self.owned_targets()
            }
            fn quarantine_id(&self) -> &UnicaId {
                self.quarantine_id()
            }
            fn absent_observation_digests(&self) -> &[Sha256Digest] {
                self.absent_observation_digests()
            }
            fn approved_preview_digest(&self) -> &Sha256Digest {
                self.approved_preview_digest()
            }
        }
    };
}

cleanup_receipt_view!(SuccessCleanupReceipt);
cleanup_receipt_view!(AbandonedCleanupReceipt);

fn validate_cleanup_receipt(
    receipt: &impl CleanupReceiptView,
    archive: &TaskArchiveStatus,
    marker_digest: &Sha256Digest,
) -> Result<(), TaskResultContractError> {
    let owned_targets = CanonicalOwnedTargets::new(receipt.owned_targets().to_vec())?;
    let removable_roles = owned_targets.roles()?;
    let expected_marker_digest = task_digest(
        &CleanupMarkerDigestRecord {
            archive_id: receipt.archive_id().clone(),
            owned_targets: owned_targets.clone(),
        },
        "cleanup marker digest failed",
    )?;
    let expected_preview_digest = task_digest(
        &CleanupPreviewDigestRecord {
            archive_id: receipt.archive_id().clone(),
            outcome: archive.outcome(),
            removable_roles,
            owned_targets,
            marker_digest: marker_digest.clone(),
        },
        "cleanup preview digest failed",
    )?;
    if receipt.archive_id() != archive.archive_id()
        || receipt.owned_targets().len() != receipt.absent_observation_digests().len()
        || expected_marker_digest != *marker_digest
        || expected_preview_digest != *receipt.approved_preview_digest()
    {
        return Err(TaskResultContractError(
            "cleanup receipt does not match its archive/target/marker/preview lineage",
        ));
    }
    Ok(())
}

fn roles_from_targets(
    targets: &[OwnedTargetLocator],
) -> Result<CanonicalOwnedTargetRoles, TaskResultContractError> {
    CanonicalOwnedTargets::new(targets.to_vec())?.roles()
}

impl JsonSchema for CleanupData {
    fn schema_name() -> Cow<'static, str> {
        "CleanupData".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<SuccessCleanupData>(),
            generator.subschema_for::<AbandonedCleanupData>(),
        ])
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct PreArmCancellationArchiveEntryDigestRecord {
    support_action_id: UnicaId,
    effect_observation: PreArmCancellationEffectObservation,
    finalization_plan: PreArmCancellationFinalizationPlan,
    finalization_plan_digest: Sha256Digest,
    receipt_plan_digest: Sha256Digest,
    finalization_recheck_evidence: PreArmCancellationFinalizationRecheckEvidence,
    completed_finalization_progress: PreArmCancellationFinalizationAttemptProgress,
    finalization_attempt_audit_digest: Sha256Digest,
    support_cancellation_receipt_id: UnicaId,
    support_cancellation_receipt_digest: Sha256Digest,
    pre_arm_recovery_receipt_id: UnicaId,
    pre_arm_recovery_receipt_digest: Sha256Digest,
    recovery_receipt_digest: Sha256Digest,
    selective_update_proof: SelectiveRepositoryUpdateProof,
    post_release_observed_history_cursor: RepositoryHistoryCursor,
    post_apply_history_partition: ValidatedRepositoryHistoryPartition,
    #[serde(skip_serializing_if = "Option::is_none")]
    deferred_repository_advance: Option<DeferredRepositoryAdvance>,
    resulting_phase: TaskPhase,
}

impl contract_digest_record_sealed::Sealed for PreArmCancellationArchiveEntryDigestRecord {}
impl ContractDigestRecord for PreArmCancellationArchiveEntryDigestRecord {}

/// Non-wire archive-lineage authority whose constructor consumes and validates
/// a non-`Clone` action/outcome receipt witness. It deliberately does not claim
/// full terminal authority: Task 13 seals the remaining terminal proofs and
/// Task 16 adds the production projection. It has no production raw-field
/// constructor.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct PreArmCancellationArchiveAuthority {
    record: PreArmCancellationArchiveEntryDigestRecord,
}

#[cfg(test)]
pub(crate) struct PreArmCancellationArchiveAuthorityTestParts {
    pub(crate) receipt_outcome_witness: ValidatedPreArmArchiveReceiptOutcomeWitness,
    pub(crate) effect_observation: PreArmCancellationEffectObservation,
    pub(crate) finalization_plan: PreArmCancellationFinalizationPlan,
    pub(crate) finalization_recheck_evidence: PreArmCancellationFinalizationRecheckEvidence,
    pub(crate) completed_finalization_progress: PreArmCancellationFinalizationAttemptProgress,
    pub(crate) support_cancellation_receipt_id: UnicaId,
    pub(crate) support_cancellation_receipt_digest: Sha256Digest,
    pub(crate) pre_arm_recovery_receipt_id: UnicaId,
    pub(crate) pre_arm_recovery_receipt_digest: Sha256Digest,
    pub(crate) recovery_receipt_digest: Sha256Digest,
    pub(crate) selective_update_proof: SelectiveRepositoryUpdateProof,
    pub(crate) post_release_observed_history_cursor: RepositoryHistoryCursor,
    pub(crate) post_apply_history_partition: ValidatedRepositoryHistoryPartition,
    pub(crate) deferred_repository_advance: Option<DeferredRepositoryAdvance>,
    pub(crate) resulting_phase: TaskPhase,
    pub(crate) terminal_prior_support_action_id: UnicaId,
    pub(crate) terminal_effect_observation: PreArmCancellationEffectObservation,
    pub(crate) terminal_finalization_plan: PreArmCancellationFinalizationPlan,
    pub(crate) terminal_finalization_recheck_evidence:
        PreArmCancellationFinalizationRecheckEvidence,
    pub(crate) terminal_completed_finalization_progress:
        PreArmCancellationFinalizationAttemptProgress,
    pub(crate) terminal_finalization_plan_digest: Sha256Digest,
    pub(crate) terminal_receipt_plan_digest: Sha256Digest,
    pub(crate) terminal_recheck_evidence_digest: Sha256Digest,
    pub(crate) terminal_finalization_attempt_audit_digest: Sha256Digest,
    pub(crate) terminal_support_cancellation_receipt_id: UnicaId,
    pub(crate) terminal_support_cancellation_receipt_digest: Sha256Digest,
    pub(crate) terminal_pre_arm_recovery_receipt_id: UnicaId,
    pub(crate) terminal_pre_arm_recovery_receipt_digest: Sha256Digest,
    pub(crate) terminal_recovery_receipt_digest: Sha256Digest,
    pub(crate) terminal_selective_update_proof: SelectiveRepositoryUpdateProof,
    pub(crate) terminal_selective_update_proof_digest: Sha256Digest,
    pub(crate) terminal_post_release_observed_history_cursor: RepositoryHistoryCursor,
    pub(crate) terminal_post_apply_history_partition: ValidatedRepositoryHistoryPartition,
    pub(crate) terminal_post_apply_history_partition_digest: Sha256Digest,
    pub(crate) terminal_deferred_repository_advance: Option<DeferredRepositoryAdvance>,
    pub(crate) terminal_deferred_repository_advance_digest: Option<Sha256Digest>,
    pub(crate) terminal_resulting_phase: TaskPhase,
}

impl PreArmCancellationArchiveAuthority {
    #[cfg(test)]
    pub(crate) fn test_only(
        parts: PreArmCancellationArchiveAuthorityTestParts,
    ) -> Result<Self, TaskResultContractError> {
        if !parts.receipt_outcome_witness.binds_archive_lineage(
            &parts.effect_observation,
            &parts.finalization_plan,
            &parts.completed_finalization_progress,
        ) {
            return Err(TaskResultContractError(
                "pre-arm archive lacks its exact action-outcome receipt witness",
            ));
        }
        let finalization_plan_digest = parts.finalization_plan.finalization_plan_digest().clone();
        let receipt_plan_digest = parts
            .finalization_plan
            .receipt_plan()
            .receipt_plan_digest()
            .clone();
        let finalization_attempt_audit_digest = parts
            .completed_finalization_progress
            .attempt_audit_digest()
            .ok_or(TaskResultContractError(
                "pre-arm archive progress lacks a terminal audit digest",
            ))?
            .clone();
        let deferred_digest = parts
            .deferred_repository_advance
            .as_ref()
            .map(|value| value.observation_digest().clone());
        parts
            .finalization_plan
            .validate_archive_lineage_projection(PreArmCancellationArchiveLineageProjection {
                effect_observation: &parts.effect_observation,
                finalization_recheck_evidence: &parts.finalization_recheck_evidence,
                completed_finalization_progress: &parts.completed_finalization_progress,
                support_cancellation_receipt_id: &parts.support_cancellation_receipt_id,
                support_cancellation_receipt_digest: &parts.support_cancellation_receipt_digest,
                pre_arm_recovery_receipt_id: &parts.pre_arm_recovery_receipt_id,
                pre_arm_recovery_receipt_digest: &parts.pre_arm_recovery_receipt_digest,
                selective_update_proof: &parts.selective_update_proof,
                post_release_observed_history_cursor: &parts.post_release_observed_history_cursor,
                post_apply_history_partition: &parts.post_apply_history_partition,
                deferred_repository_advance: parts.deferred_repository_advance.as_ref(),
                resulting_phase: parts.resulting_phase,
            })
            .map_err(|_| {
                TaskResultContractError(
                    "pre-arm archive-lineage projection violates its finalization plan",
                )
            })?;
        if parts.terminal_prior_support_action_id != *parts.effect_observation.support_action_id()
            || parts.terminal_effect_observation != parts.effect_observation
            || parts.terminal_finalization_plan != parts.finalization_plan
            || parts.terminal_finalization_recheck_evidence != parts.finalization_recheck_evidence
            || parts.terminal_completed_finalization_progress
                != parts.completed_finalization_progress
            || parts.terminal_finalization_plan_digest != finalization_plan_digest
            || parts.terminal_receipt_plan_digest != receipt_plan_digest
            || parts.terminal_recheck_evidence_digest
                != *parts.finalization_recheck_evidence.evidence_digest()
            || parts.terminal_finalization_attempt_audit_digest != finalization_attempt_audit_digest
            || parts.terminal_support_cancellation_receipt_id
                != parts.support_cancellation_receipt_id
            || parts.terminal_support_cancellation_receipt_digest
                != parts.support_cancellation_receipt_digest
            || parts.terminal_pre_arm_recovery_receipt_id != parts.pre_arm_recovery_receipt_id
            || parts.terminal_pre_arm_recovery_receipt_digest
                != parts.pre_arm_recovery_receipt_digest
            || parts.terminal_recovery_receipt_digest != parts.recovery_receipt_digest
            || parts.terminal_selective_update_proof != parts.selective_update_proof
            || parts.terminal_selective_update_proof_digest
                != *parts.selective_update_proof.proof_digest()
            || parts.terminal_post_release_observed_history_cursor
                != parts.post_release_observed_history_cursor
            || parts.terminal_post_apply_history_partition_digest
                != *parts.post_apply_history_partition.partition_digest()
            || parts.terminal_post_apply_history_partition != parts.post_apply_history_partition
            || parts.terminal_deferred_repository_advance != parts.deferred_repository_advance
            || parts.terminal_deferred_repository_advance_digest != deferred_digest
            || parts.terminal_resulting_phase != parts.resulting_phase
        {
            return Err(TaskResultContractError(
                "pre-arm archive authority disagrees with its single archive-lineage projection",
            ));
        }
        Ok(Self {
            record: PreArmCancellationArchiveEntryDigestRecord {
                support_action_id: parts.terminal_prior_support_action_id,
                effect_observation: parts.effect_observation,
                finalization_plan: parts.finalization_plan,
                finalization_plan_digest,
                receipt_plan_digest,
                finalization_recheck_evidence: parts.finalization_recheck_evidence,
                completed_finalization_progress: parts.completed_finalization_progress,
                finalization_attempt_audit_digest,
                support_cancellation_receipt_id: parts.support_cancellation_receipt_id,
                support_cancellation_receipt_digest: parts.support_cancellation_receipt_digest,
                pre_arm_recovery_receipt_id: parts.pre_arm_recovery_receipt_id,
                pre_arm_recovery_receipt_digest: parts.pre_arm_recovery_receipt_digest,
                recovery_receipt_digest: parts.recovery_receipt_digest,
                selective_update_proof: parts.selective_update_proof,
                post_release_observed_history_cursor: parts.post_release_observed_history_cursor,
                post_apply_history_partition: parts.post_apply_history_partition,
                deferred_repository_advance: parts.deferred_repository_advance,
                resulting_phase: parts.resulting_phase,
            },
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct PreArmCancellationArchiveEntry {
    support_action_id: UnicaId,
    effect_observation: PreArmCancellationEffectObservation,
    finalization_plan: PreArmCancellationFinalizationPlan,
    finalization_plan_digest: Sha256Digest,
    receipt_plan_digest: Sha256Digest,
    finalization_recheck_evidence: PreArmCancellationFinalizationRecheckEvidence,
    completed_finalization_progress: PreArmCancellationFinalizationAttemptProgress,
    finalization_attempt_audit_digest: Sha256Digest,
    support_cancellation_receipt_id: UnicaId,
    support_cancellation_receipt_digest: Sha256Digest,
    pre_arm_recovery_receipt_id: UnicaId,
    pre_arm_recovery_receipt_digest: Sha256Digest,
    recovery_receipt_digest: Sha256Digest,
    selective_update_proof: SelectiveRepositoryUpdateProof,
    post_release_observed_history_cursor: RepositoryHistoryCursor,
    post_apply_history_partition: ValidatedRepositoryHistoryPartition,
    #[serde(skip_serializing_if = "Option::is_none")]
    deferred_repository_advance: Option<DeferredRepositoryAdvance>,
    resulting_phase: TaskPhase,
    entry_digest: Sha256Digest,
}

impl PreArmCancellationArchiveEntry {
    pub(crate) fn from_archive_lineage(
        authority: PreArmCancellationArchiveAuthority,
    ) -> Result<Self, TaskResultContractError> {
        let record = authority.record;
        let entry_digest = task_digest(&record, "pre-arm archive entry digest failed")?;
        Ok(Self {
            support_action_id: record.support_action_id,
            effect_observation: record.effect_observation,
            finalization_plan: record.finalization_plan,
            finalization_plan_digest: record.finalization_plan_digest,
            receipt_plan_digest: record.receipt_plan_digest,
            finalization_recheck_evidence: record.finalization_recheck_evidence,
            completed_finalization_progress: record.completed_finalization_progress,
            finalization_attempt_audit_digest: record.finalization_attempt_audit_digest,
            support_cancellation_receipt_id: record.support_cancellation_receipt_id,
            support_cancellation_receipt_digest: record.support_cancellation_receipt_digest,
            pre_arm_recovery_receipt_id: record.pre_arm_recovery_receipt_id,
            pre_arm_recovery_receipt_digest: record.pre_arm_recovery_receipt_digest,
            recovery_receipt_digest: record.recovery_receipt_digest,
            selective_update_proof: record.selective_update_proof,
            post_release_observed_history_cursor: record.post_release_observed_history_cursor,
            post_apply_history_partition: record.post_apply_history_partition,
            deferred_repository_advance: record.deferred_repository_advance,
            resulting_phase: record.resulting_phase,
            entry_digest,
        })
    }

    pub(crate) const fn support_action_id(&self) -> &UnicaId {
        &self.support_action_id
    }

    pub(crate) const fn support_cancellation_receipt_id(&self) -> &UnicaId {
        &self.support_cancellation_receipt_id
    }

    pub(crate) const fn pre_arm_recovery_receipt_id(&self) -> &UnicaId {
        &self.pre_arm_recovery_receipt_id
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct PreArmCancellationArchiveEntries(Vec<PreArmCancellationArchiveEntry>);

impl PreArmCancellationArchiveEntries {
    fn new(values: Vec<PreArmCancellationArchiveEntry>) -> Result<Self, TaskResultContractError> {
        if values.len() > MAX_RESULT_ITEMS
            || values
                .windows(2)
                .any(|pair| pair[0].support_action_id() >= pair[1].support_action_id())
            || {
                let mut cancellation_ids = BTreeSet::new();
                let mut recovery_ids = BTreeSet::new();
                values.iter().any(|entry| {
                    !cancellation_ids.insert(entry.support_cancellation_receipt_id().as_str())
                        || !recovery_ids.insert(entry.pre_arm_recovery_receipt_id().as_str())
                })
            }
        {
            return Err(TaskResultContractError(
                "pre-arm archive entries must be canonical and unique by support action",
            ));
        }
        Ok(Self(values))
    }

    fn as_slice(&self) -> &[PreArmCancellationArchiveEntry] {
        &self.0
    }
}

impl JsonSchema for PreArmCancellationArchiveEntries {
    fn schema_name() -> Cow<'static, str> {
        "PreArmCancellationArchiveEntries".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "array",
            "items": generator.subschema_for::<PreArmCancellationArchiveEntry>(),
            "minItems": 0,
            "maxItems": MAX_RESULT_ITEMS,
            "uniqueItems": true
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ArchivePublicationEntry {
    entry_name: ArchiveEntryName,
    sha256: Sha256Digest,
}

impl ArchivePublicationEntry {
    #[cfg(test)]
    pub(crate) const fn test_only(entry_name: ArchiveEntryName, sha256: Sha256Digest) -> Self {
        Self { entry_name, sha256 }
    }

    pub(crate) const fn entry_name(&self) -> &ArchiveEntryName {
        &self.entry_name
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct ArchivePublicationEntries(Vec<ArchivePublicationEntry>);

impl ArchivePublicationEntries {
    fn new(values: Vec<ArchivePublicationEntry>) -> Result<Self, TaskResultContractError> {
        if values.len() > MAX_RESULT_ITEMS {
            return Err(TaskResultContractError("archive member list is oversized"));
        }
        let mut previous: Option<&str> = None;
        let mut folded = BTreeSet::new();
        for value in &values {
            let name = value.entry_name.as_str();
            let lower = name.to_ascii_lowercase();
            if previous.is_some_and(|previous| previous >= name)
                || !folded.insert(lower.clone())
                || lower == "archive-manifest.json"
            {
                return Err(TaskResultContractError(
                    "archive members must be ASCII-ordered, case-fold unique, and exclude the final manifest",
                ));
            }
            previous = Some(name);
        }
        Ok(Self(values))
    }

    fn staged(values: Vec<ArchivePublicationEntry>) -> Result<Self, TaskResultContractError> {
        let values = Self::new(values)?;
        if values.0.iter().any(|entry| {
            entry
                .entry_name
                .as_str()
                .to_ascii_lowercase()
                .starts_with("handoff-releases/")
        }) {
            return Err(TaskResultContractError(
                "staged archive members cannot use the handoff-release namespace",
            ));
        }
        Ok(values)
    }

    fn as_slice(&self) -> &[ArchivePublicationEntry] {
        &self.0
    }
}

impl JsonSchema for ArchivePublicationEntries {
    fn schema_name() -> Cow<'static, str> {
        "ArchivePublicationEntries".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "array",
            "items": generator.subschema_for::<ArchivePublicationEntry>(),
            "minItems": 0,
            "maxItems": MAX_RESULT_ITEMS,
            "uniqueItems": true
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct StagedArchiveEntryManifestDigestRecord {
    entries: ArchivePublicationEntries,
}

impl contract_digest_record_sealed::Sealed for StagedArchiveEntryManifestDigestRecord {}
impl ContractDigestRecord for StagedArchiveEntryManifestDigestRecord {}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct StagedArchiveEntryManifestAuthority(ArchivePublicationEntries);

impl StagedArchiveEntryManifestAuthority {
    #[cfg(test)]
    pub(crate) fn test_only(
        entries: Vec<ArchivePublicationEntry>,
    ) -> Result<Self, TaskResultContractError> {
        Ok(Self(ArchivePublicationEntries::staged(entries)?))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct StagedArchiveEntryManifest {
    entries: ArchivePublicationEntries,
    staged_entry_manifest_digest: Sha256Digest,
}

impl StagedArchiveEntryManifest {
    pub(crate) fn from_parser(
        authority: StagedArchiveEntryManifestAuthority,
    ) -> Result<Self, TaskResultContractError> {
        let record = StagedArchiveEntryManifestDigestRecord {
            entries: authority.0,
        };
        let staged_entry_manifest_digest =
            task_digest(&record, "staged archive entry-manifest digest failed")?;
        Ok(Self {
            entries: record.entries,
            staged_entry_manifest_digest,
        })
    }

    pub(crate) fn entries(&self) -> &[ArchivePublicationEntry] {
        self.entries.as_slice()
    }

    pub(crate) const fn staged_entry_manifest_digest(&self) -> &Sha256Digest {
        &self.staged_entry_manifest_digest
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct CanonicalUnicaIds(Vec<UnicaId>);

impl CanonicalUnicaIds {
    fn new(values: Vec<UnicaId>) -> Result<Self, TaskResultContractError> {
        if values.len() > MAX_RESULT_ITEMS || values.windows(2).any(|pair| pair[0] >= pair[1]) {
            return Err(TaskResultContractError(
                "receipt IDs must be bounded, canonical, and unique",
            ));
        }
        Ok(Self(values))
    }

    fn as_slice(&self) -> &[UnicaId] {
        &self.0
    }
}

impl JsonSchema for CanonicalUnicaIds {
    fn schema_name() -> Cow<'static, str> {
        "CanonicalUnicaIds".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "array",
            "items": generator.subschema_for::<UnicaId>(),
            "minItems": 0,
            "maxItems": MAX_RESULT_ITEMS,
            "uniqueItems": true
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ArchiveReceiptIdGroups {
    support_arming: CanonicalUnicaIds,
    support_prerequisite: CanonicalUnicaIds,
    support_cancellation: CanonicalUnicaIds,
    support_recovery: CanonicalUnicaIds,
    pre_arm_recovery: CanonicalUnicaIds,
}

impl ArchiveReceiptIdGroups {
    fn new(
        support_arming: Vec<UnicaId>,
        support_prerequisite: Vec<UnicaId>,
        support_cancellation: Vec<UnicaId>,
        support_recovery: Vec<UnicaId>,
        pre_arm_recovery: Vec<UnicaId>,
    ) -> Result<Self, TaskResultContractError> {
        let groups = Self {
            support_arming: CanonicalUnicaIds::new(support_arming)?,
            support_prerequisite: CanonicalUnicaIds::new(support_prerequisite)?,
            support_cancellation: CanonicalUnicaIds::new(support_cancellation)?,
            support_recovery: CanonicalUnicaIds::new(support_recovery)?,
            pre_arm_recovery: CanonicalUnicaIds::new(pre_arm_recovery)?,
        };
        let mut seen = BTreeSet::new();
        if [
            groups.support_arming.as_slice(),
            groups.support_prerequisite.as_slice(),
            groups.support_cancellation.as_slice(),
            groups.support_recovery.as_slice(),
            groups.pre_arm_recovery.as_slice(),
        ]
        .into_iter()
        .flatten()
        .any(|id| !seen.insert(id.as_str()))
        {
            return Err(TaskResultContractError(
                "archive receipt-ID groups must be pairwise disjoint",
            ));
        }
        Ok(groups)
    }

    fn validate_prearm_entries(
        &self,
        entries: &PreArmCancellationArchiveEntries,
    ) -> Result<(), TaskResultContractError> {
        let mut recovery_ids: Vec<_> = entries
            .as_slice()
            .iter()
            .map(|entry| entry.pre_arm_recovery_receipt_id().clone())
            .collect();
        recovery_ids.sort();
        if recovery_ids != self.pre_arm_recovery.0
            || entries.as_slice().iter().any(|entry| {
                self.support_cancellation
                    .0
                    .binary_search(entry.support_cancellation_receipt_id())
                    .is_err()
            })
        {
            return Err(TaskResultContractError(
                "pre-arm archive entries disagree with cancellation/recovery receipt ID projections",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct HandoffLineageDigestRecord {
    archive_id: UnicaId,
    outcome: TaskArchiveOutcome,
    schema_version: ArchiveSchemaVersion,
    archive_container_capability_row_id: CapabilityRowId,
    staged_entry_manifest: StagedArchiveEntryManifest,
    retained_entry_names: CanonicalArchiveEntryNames,
    support_arming_receipt_ids: CanonicalUnicaIds,
    support_prerequisite_receipt_ids: CanonicalUnicaIds,
    support_cancellation_receipt_ids: CanonicalUnicaIds,
    support_recovery_receipt_ids: CanonicalUnicaIds,
    pre_arm_recovery_receipt_ids: CanonicalUnicaIds,
    pre_arm_cancellation_recoveries: PreArmCancellationArchiveEntries,
    deferred_advance_consumption_receipt_ids: CanonicalUnicaIds,
    preview_digest: Sha256Digest,
}

impl contract_digest_record_sealed::Sealed for HandoffLineageDigestRecord {}
impl ContractDigestRecord for HandoffLineageDigestRecord {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct FrozenProviderBoundaryDigestRecord {
    provider_boundary_digests: CanonicalDigests,
}

impl contract_digest_record_sealed::Sealed for FrozenProviderBoundaryDigestRecord {}
impl ContractDigestRecord for FrozenProviderBoundaryDigestRecord {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RetainedArchiveLineageDigestRecord {
    archive_id: UnicaId,
    outcome: TaskArchiveOutcome,
    schema_version: ArchiveSchemaVersion,
    archive_container_capability_row_id: CapabilityRowId,
    staged_entry_manifest: StagedArchiveEntryManifest,
    retained_entry_names: CanonicalArchiveEntryNames,
    support_arming_receipt_ids: CanonicalUnicaIds,
    support_prerequisite_receipt_ids: CanonicalUnicaIds,
    support_cancellation_receipt_ids: CanonicalUnicaIds,
    support_recovery_receipt_ids: CanonicalUnicaIds,
    pre_arm_recovery_receipt_ids: CanonicalUnicaIds,
    pre_arm_cancellation_recoveries: PreArmCancellationArchiveEntries,
    deferred_advance_consumption_receipt_ids: CanonicalUnicaIds,
    preview_digest: Sha256Digest,
    archive_staging_receipt_digest: Sha256Digest,
    handoff_retention_releases: HandoffRetentionReleaseReceipts,
}

impl contract_digest_record_sealed::Sealed for RetainedArchiveLineageDigestRecord {}
impl ContractDigestRecord for RetainedArchiveLineageDigestRecord {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ArchivePublicationManifestDigestRecord {
    archive_id: UnicaId,
    outcome: TaskArchiveOutcome,
    schema_version: ArchiveSchemaVersion,
    archive_container_capability_row_id: CapabilityRowId,
    staged_archive_sha256: Sha256Digest,
    staged_entry_manifest_digest: Sha256Digest,
    archive_staging_receipt_digest: Sha256Digest,
    retained_lineage_digest: Sha256Digest,
    handoff_release_receipt_digests: CanonicalDigests,
    entries: ArchivePublicationEntries,
}

impl contract_digest_record_sealed::Sealed for ArchivePublicationManifestDigestRecord {}
impl ContractDigestRecord for ArchivePublicationManifestDigestRecord {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ArchivePublicationManifest {
    archive_id: UnicaId,
    outcome: TaskArchiveOutcome,
    schema_version: ArchiveSchemaVersion,
    archive_container_capability_row_id: CapabilityRowId,
    staged_archive_sha256: Sha256Digest,
    staged_entry_manifest_digest: Sha256Digest,
    archive_staging_receipt_digest: Sha256Digest,
    retained_lineage_digest: Sha256Digest,
    handoff_release_receipt_digests: CanonicalDigests,
    entries: ArchivePublicationEntries,
    manifest_digest: Sha256Digest,
}

impl ArchivePublicationManifest {
    fn from_record(
        record: ArchivePublicationManifestDigestRecord,
    ) -> Result<Self, TaskResultContractError> {
        let manifest_digest = task_digest(&record, "archive publication manifest digest failed")?;
        Ok(Self {
            archive_id: record.archive_id,
            outcome: record.outcome,
            schema_version: record.schema_version,
            archive_container_capability_row_id: record.archive_container_capability_row_id,
            staged_archive_sha256: record.staged_archive_sha256,
            staged_entry_manifest_digest: record.staged_entry_manifest_digest,
            archive_staging_receipt_digest: record.archive_staging_receipt_digest,
            retained_lineage_digest: record.retained_lineage_digest,
            handoff_release_receipt_digests: record.handoff_release_receipt_digests,
            entries: record.entries,
            manifest_digest,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ArchiveParsedEntrySetDigestRecord {
    entries: ArchivePublicationEntries,
}

impl contract_digest_record_sealed::Sealed for ArchiveParsedEntrySetDigestRecord {}
impl ContractDigestRecord for ArchiveParsedEntrySetDigestRecord {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ArchivePublicationObservationDigestRecord {
    publication_observation_id: UnicaId,
    archive_id: UnicaId,
    archive_container_capability_row_id: CapabilityRowId,
    final_byte_observation_id: UnicaId,
    final_archive_size: SafeResultCount,
    final_archive_sha256: Sha256Digest,
    archive_staging_receipt_digest: Sha256Digest,
    staged_entry_manifest_digest: Sha256Digest,
    handoff_release_receipt_digests: CanonicalDigests,
    publication_manifest_digest: Sha256Digest,
    parsed_entry_set_digest: Sha256Digest,
    file_synced: TrueLiteral,
    parent_directory_synced: TrueLiteral,
    durable_write_receipt_id: UnicaId,
}

impl contract_digest_record_sealed::Sealed for ArchivePublicationObservationDigestRecord {}
impl ContractDigestRecord for ArchivePublicationObservationDigestRecord {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ArchivePublicationObservation {
    publication_observation_id: UnicaId,
    archive_id: UnicaId,
    archive_container_capability_row_id: CapabilityRowId,
    final_byte_observation_id: UnicaId,
    final_archive_size: SafeResultCount,
    final_archive_sha256: Sha256Digest,
    archive_staging_receipt_digest: Sha256Digest,
    staged_entry_manifest_digest: Sha256Digest,
    handoff_release_receipt_digests: CanonicalDigests,
    publication_manifest_digest: Sha256Digest,
    parsed_entry_set_digest: Sha256Digest,
    file_synced: TrueLiteral,
    parent_directory_synced: TrueLiteral,
    durable_write_receipt_id: UnicaId,
    observation_digest: Sha256Digest,
}

fn hash_member_bytes(bytes: &[u8]) -> Result<Sha256Digest, TaskResultContractError> {
    Sha256Digest::parse(&format!("{:x}", Sha256::digest(bytes)))
        .map_err(|_| TaskResultContractError("archive member hashing failed"))
}

fn validate_publication_entries(
    staged: &StagedArchiveEntryManifest,
    releases: &HandoffRetentionReleaseReceipts,
    final_entries: &ArchivePublicationEntries,
) -> Result<(), TaskResultContractError> {
    let mut expected = staged.entries().to_vec();
    for release in releases.as_slice() {
        let entry_name = ArchiveEntryName::parse(&format!(
            "handoff-releases/{}.json",
            release.retention_lease_id().as_str()
        ))
        .map_err(|_| TaskResultContractError("handoff release entry name is invalid"))?;
        let bytes = serde_json_canonicalizer::to_vec(release)
            .map_err(|_| TaskResultContractError("handoff release serialization failed"))?;
        expected.push(ArchivePublicationEntry {
            entry_name,
            sha256: hash_member_bytes(&bytes)?,
        });
    }
    expected.sort_by(|left, right| left.entry_name.as_str().cmp(right.entry_name.as_str()));
    if expected != final_entries.0 {
        return Err(TaskResultContractError(
            "final archive members are not the exact staged-plus-release set",
        ));
    }
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ArchiveDataAuthority {
    archive_id: UnicaId,
    approved_preview: ApprovedArchivePreviewAuthority,
    archive_container_capability_row_id: CapabilityRowId,
    archive_staging_receipt: ArchiveStagingReceipt,
    staged_entry_manifest: StagedArchiveEntryManifest,
    publication_entries: ArchivePublicationEntries,
    publication_observation_id: UnicaId,
    publication_byte_observation: ArchivePublicationByteObservation,
    receipt_ids: ArchiveReceiptIdGroups,
    pre_arm_cancellation_recoveries: PreArmCancellationArchiveEntries,
    handoff_retention_releases: HandoffRetentionReleaseReceipts,
    deferred_advance_consumption_receipt_ids: CanonicalUnicaIds,
    provider_boundary_digests: CanonicalDigests,
}

#[cfg(test)]
pub(crate) struct ArchiveDataAuthorityTestParts {
    pub(crate) archive_id: UnicaId,
    pub(crate) approved_preview: ApprovedArchivePreviewAuthority,
    pub(crate) archive_container_capability_row_id: CapabilityRowId,
    pub(crate) archive_staging_receipt: ArchiveStagingReceipt,
    pub(crate) staged_entry_manifest: StagedArchiveEntryManifest,
    pub(crate) publication_entries: Vec<ArchivePublicationEntry>,
    pub(crate) publication_observation_id: UnicaId,
    pub(crate) publication_byte_observation: ArchivePublicationByteObservation,
    pub(crate) support_arming_receipt_ids: Vec<UnicaId>,
    pub(crate) support_prerequisite_receipt_ids: Vec<UnicaId>,
    pub(crate) support_cancellation_receipt_ids: Vec<UnicaId>,
    pub(crate) support_recovery_receipt_ids: Vec<UnicaId>,
    pub(crate) pre_arm_recovery_receipt_ids: Vec<UnicaId>,
    pub(crate) pre_arm_cancellation_recoveries: Vec<PreArmCancellationArchiveEntry>,
    pub(crate) handoff_retention_releases: HandoffRetentionReleaseReceipts,
    pub(crate) deferred_advance_consumption_receipt_ids: Vec<UnicaId>,
    pub(crate) provider_boundary_digests: Vec<Sha256Digest>,
}

impl ArchiveDataAuthority {
    #[cfg(test)]
    pub(crate) fn test_only(
        parts: ArchiveDataAuthorityTestParts,
    ) -> Result<Self, TaskResultContractError> {
        let receipt_ids = ArchiveReceiptIdGroups::new(
            parts.support_arming_receipt_ids,
            parts.support_prerequisite_receipt_ids,
            parts.support_cancellation_receipt_ids,
            parts.support_recovery_receipt_ids,
            parts.pre_arm_recovery_receipt_ids,
        )?;
        let pre_arm_cancellation_recoveries =
            PreArmCancellationArchiveEntries::new(parts.pre_arm_cancellation_recoveries)?;
        receipt_ids.validate_prearm_entries(&pre_arm_cancellation_recoveries)?;
        Ok(Self {
            archive_id: parts.archive_id,
            approved_preview: parts.approved_preview,
            archive_container_capability_row_id: parts.archive_container_capability_row_id,
            archive_staging_receipt: parts.archive_staging_receipt,
            staged_entry_manifest: parts.staged_entry_manifest,
            publication_entries: ArchivePublicationEntries::new(parts.publication_entries)?,
            publication_observation_id: parts.publication_observation_id,
            publication_byte_observation: parts.publication_byte_observation,
            receipt_ids,
            pre_arm_cancellation_recoveries,
            handoff_retention_releases: parts.handoff_retention_releases,
            deferred_advance_consumption_receipt_ids: CanonicalUnicaIds::new(
                parts.deferred_advance_consumption_receipt_ids,
            )?,
            provider_boundary_digests: CanonicalDigests::canonical(
                parts.provider_boundary_digests,
            )?,
        })
    }
}

macro_rules! archive_data_leaf {
    ($name:ident, $outcome:ty) => {
        #[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        pub(crate) struct $name {
            archive_id: UnicaId,
            outcome: $outcome,
            schema_version: ArchiveSchemaVersion,
            sha256: Sha256Digest,
            archive_staging_receipt: ArchiveStagingReceipt,
            archive_container_capability_row_id: CapabilityRowId,
            staged_entry_manifest: StagedArchiveEntryManifest,
            publication_manifest: ArchivePublicationManifest,
            publication_observation: ArchivePublicationObservation,
            retained_entry_names: CanonicalArchiveEntryNames,
            support_arming_receipt_ids: CanonicalUnicaIds,
            support_prerequisite_receipt_ids: CanonicalUnicaIds,
            support_cancellation_receipt_ids: CanonicalUnicaIds,
            support_recovery_receipt_ids: CanonicalUnicaIds,
            pre_arm_recovery_receipt_ids: CanonicalUnicaIds,
            pre_arm_cancellation_recoveries: PreArmCancellationArchiveEntries,
            handoff_retention_releases: HandoffRetentionReleaseReceipts,
            deferred_advance_consumption_receipt_ids: CanonicalUnicaIds,
            retained_lineage_digest: Sha256Digest,
            preview_digest: Sha256Digest,
        }
    };
}

archive_data_leaf!(SuccessArchiveData, SuccessOutcome);
archive_data_leaf!(AbandonedArchiveData, AbandonedOutcome);

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub(crate) enum ArchiveData {
    Success(SuccessArchiveData),
    Abandoned(AbandonedArchiveData),
}

impl JsonSchema for ArchiveData {
    fn schema_name() -> Cow<'static, str> {
        "ArchiveData".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<SuccessArchiveData>(),
            generator.subschema_for::<AbandonedArchiveData>(),
        ])
    }
}

/// Linear projection from one fully validated final archive publication into
/// the immutable status row.  The final byte hash stays wrapped in the
/// publication-only capability until this value is consumed by `status.rs`;
/// no caller can pair an observed hash with a different archive identity,
/// outcome, or retained-lineage digest.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ArchiveStatusProjectionAuthority {
    archive_id: UnicaId,
    outcome: TaskArchiveOutcome,
    sha256: PublishedArchiveSha256,
    retained_lineage_digest: Sha256Digest,
}

impl ArchiveStatusProjectionAuthority {
    pub(crate) fn into_parts(self) -> (UnicaId, TaskArchiveOutcome, Sha256Digest, Sha256Digest) {
        (
            self.archive_id,
            self.outcome,
            self.sha256.as_digest().clone(),
            self.retained_lineage_digest,
        )
    }
}

/// One completed final publication owns both wire result and retained status.
/// It is non-Clone so the same sealed final-byte evidence cannot authorize a
/// second independent status/result pair.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct CompletedArchivePublication {
    data: ArchiveData,
    status: TaskArchiveStatus,
}

impl CompletedArchivePublication {
    pub(crate) fn from_authority(
        authority: ArchiveDataAuthority,
    ) -> Result<Self, TaskResultContractError> {
        let ArchiveDataAuthority {
            archive_id,
            approved_preview,
            archive_container_capability_row_id,
            archive_staging_receipt,
            staged_entry_manifest,
            publication_entries,
            publication_observation_id,
            publication_byte_observation,
            receipt_ids,
            pre_arm_cancellation_recoveries,
            handoff_retention_releases,
            deferred_advance_consumption_receipt_ids,
            provider_boundary_digests,
        } = authority;
        let ApprovedArchivePreviewAuthority {
            outcome,
            retained_entry_names,
            preview_digest,
        } = approved_preview;
        let staged_names: Vec<_> = staged_entry_manifest
            .entries()
            .iter()
            .map(|entry| entry.entry_name().clone())
            .collect();
        if retained_entry_names.0 != staged_names
            || archive_staging_receipt.archive_id() != &archive_id
        {
            return Err(TaskResultContractError(
                "archive staging receipt or retained names disagree with the staged core",
            ));
        }
        let schema_version = ArchiveSchemaVersion::current();
        let handoff_record = HandoffLineageDigestRecord {
            archive_id: archive_id.clone(),
            outcome,
            schema_version,
            archive_container_capability_row_id: archive_container_capability_row_id.clone(),
            staged_entry_manifest: staged_entry_manifest.clone(),
            retained_entry_names: retained_entry_names.clone(),
            support_arming_receipt_ids: receipt_ids.support_arming.clone(),
            support_prerequisite_receipt_ids: receipt_ids.support_prerequisite.clone(),
            support_cancellation_receipt_ids: receipt_ids.support_cancellation.clone(),
            support_recovery_receipt_ids: receipt_ids.support_recovery.clone(),
            pre_arm_recovery_receipt_ids: receipt_ids.pre_arm_recovery.clone(),
            pre_arm_cancellation_recoveries: pre_arm_cancellation_recoveries.clone(),
            deferred_advance_consumption_receipt_ids: deferred_advance_consumption_receipt_ids
                .clone(),
            preview_digest: preview_digest.clone(),
        };
        let handoff_lineage_digest =
            task_digest(&handoff_record, "archive handoff lineage digest failed")?;
        let frozen_provider_boundary_digest = task_digest(
            &FrozenProviderBoundaryDigestRecord {
                provider_boundary_digests,
            },
            "frozen provider boundary digest failed",
        )?;
        if archive_staging_receipt.handoff_lineage_digest() != &handoff_lineage_digest
            || archive_staging_receipt.frozen_provider_boundary_digest()
                != &frozen_provider_boundary_digest
        {
            return Err(TaskResultContractError(
                "archive staging receipt does not bind the exact handoff/provider records",
            ));
        }
        let retained_record = RetainedArchiveLineageDigestRecord {
            archive_id: archive_id.clone(),
            outcome,
            schema_version,
            archive_container_capability_row_id: archive_container_capability_row_id.clone(),
            staged_entry_manifest: staged_entry_manifest.clone(),
            retained_entry_names: retained_entry_names.clone(),
            support_arming_receipt_ids: receipt_ids.support_arming.clone(),
            support_prerequisite_receipt_ids: receipt_ids.support_prerequisite.clone(),
            support_cancellation_receipt_ids: receipt_ids.support_cancellation.clone(),
            support_recovery_receipt_ids: receipt_ids.support_recovery.clone(),
            pre_arm_recovery_receipt_ids: receipt_ids.pre_arm_recovery.clone(),
            pre_arm_cancellation_recoveries: pre_arm_cancellation_recoveries.clone(),
            deferred_advance_consumption_receipt_ids: deferred_advance_consumption_receipt_ids
                .clone(),
            preview_digest: preview_digest.clone(),
            archive_staging_receipt_digest: archive_staging_receipt.receipt_digest().clone(),
            handoff_retention_releases: handoff_retention_releases.clone(),
        };
        let retained_lineage_digest =
            task_digest(&retained_record, "retained archive lineage digest failed")?;
        validate_publication_entries(
            &staged_entry_manifest,
            &handoff_retention_releases,
            &publication_entries,
        )?;
        let release_digests =
            CanonicalDigests::from_aligned(&handoff_retention_releases.release_receipt_digests())?;
        let publication_manifest =
            ArchivePublicationManifest::from_record(ArchivePublicationManifestDigestRecord {
                archive_id: archive_id.clone(),
                outcome,
                schema_version,
                archive_container_capability_row_id: archive_container_capability_row_id.clone(),
                staged_archive_sha256: archive_staging_receipt.staged_archive_sha256().clone(),
                staged_entry_manifest_digest: staged_entry_manifest
                    .staged_entry_manifest_digest()
                    .clone(),
                archive_staging_receipt_digest: archive_staging_receipt.receipt_digest().clone(),
                retained_lineage_digest: retained_lineage_digest.clone(),
                handoff_release_receipt_digests: release_digests.clone(),
                entries: publication_entries.clone(),
            })?;
        let parsed_entry_set_digest = task_digest(
            &ArchiveParsedEntrySetDigestRecord {
                entries: publication_entries,
            },
            "archive parsed entry-set digest failed",
        )?;
        if publication_byte_observation.archive_id() != &archive_id
            || publication_byte_observation.archive_container_capability_row_id()
                != &archive_container_capability_row_id
            || publication_byte_observation.publication_manifest_digest()
                != &publication_manifest.manifest_digest
            || publication_byte_observation.parsed_entry_set_digest() != &parsed_entry_set_digest
        {
            return Err(TaskResultContractError(
                "final byte observation differs from archive manifest or parsed member set",
            ));
        }
        let final_archive_size =
            SafeResultCount::new(publication_byte_observation.final_archive_size())
                .map_err(|_| TaskResultContractError("final archive size is not I-JSON safe"))?;
        let final_sha_digest = publication_byte_observation
            .final_archive_sha256()
            .as_digest()
            .clone();
        let observation_record = ArchivePublicationObservationDigestRecord {
            publication_observation_id: publication_observation_id.clone(),
            archive_id: archive_id.clone(),
            archive_container_capability_row_id: archive_container_capability_row_id.clone(),
            final_byte_observation_id: publication_byte_observation
                .final_byte_observation_id()
                .clone(),
            final_archive_size,
            final_archive_sha256: final_sha_digest.clone(),
            archive_staging_receipt_digest: archive_staging_receipt.receipt_digest().clone(),
            staged_entry_manifest_digest: staged_entry_manifest
                .staged_entry_manifest_digest()
                .clone(),
            handoff_release_receipt_digests: release_digests,
            publication_manifest_digest: publication_manifest.manifest_digest.clone(),
            parsed_entry_set_digest,
            file_synced: TrueLiteral,
            parent_directory_synced: TrueLiteral,
            durable_write_receipt_id: publication_byte_observation
                .durable_write_receipt_id()
                .clone(),
        };
        let observation_digest = task_digest(
            &observation_record,
            "archive publication observation digest failed",
        )?;
        let publication_observation = ArchivePublicationObservation {
            publication_observation_id: observation_record.publication_observation_id,
            archive_id: observation_record.archive_id,
            archive_container_capability_row_id: observation_record
                .archive_container_capability_row_id,
            final_byte_observation_id: observation_record.final_byte_observation_id,
            final_archive_size: observation_record.final_archive_size,
            final_archive_sha256: observation_record.final_archive_sha256,
            archive_staging_receipt_digest: observation_record.archive_staging_receipt_digest,
            staged_entry_manifest_digest: observation_record.staged_entry_manifest_digest,
            handoff_release_receipt_digests: observation_record.handoff_release_receipt_digests,
            publication_manifest_digest: observation_record.publication_manifest_digest,
            parsed_entry_set_digest: observation_record.parsed_entry_set_digest,
            file_synced: observation_record.file_synced,
            parent_directory_synced: observation_record.parent_directory_synced,
            durable_write_receipt_id: observation_record.durable_write_receipt_id,
            observation_digest,
        };
        let final_sha: PublishedArchiveSha256 =
            publication_byte_observation.into_final_archive_sha256();
        let status = TaskArchiveStatus::from_publication(ArchiveStatusProjectionAuthority {
            archive_id: archive_id.clone(),
            outcome,
            sha256: final_sha,
            retained_lineage_digest: retained_lineage_digest.clone(),
        });
        let data = match outcome {
            TaskArchiveOutcome::Success => ArchiveData::Success(SuccessArchiveData {
                archive_id,
                outcome: SuccessOutcome::Value,
                schema_version,
                sha256: final_sha_digest,
                archive_staging_receipt,
                archive_container_capability_row_id,
                staged_entry_manifest,
                publication_manifest,
                publication_observation,
                retained_entry_names,
                support_arming_receipt_ids: receipt_ids.support_arming,
                support_prerequisite_receipt_ids: receipt_ids.support_prerequisite,
                support_cancellation_receipt_ids: receipt_ids.support_cancellation,
                support_recovery_receipt_ids: receipt_ids.support_recovery,
                pre_arm_recovery_receipt_ids: receipt_ids.pre_arm_recovery,
                pre_arm_cancellation_recoveries,
                handoff_retention_releases,
                deferred_advance_consumption_receipt_ids,
                retained_lineage_digest,
                preview_digest,
            }),
            TaskArchiveOutcome::Abandoned => ArchiveData::Abandoned(AbandonedArchiveData {
                archive_id,
                outcome: AbandonedOutcome::Value,
                schema_version,
                sha256: final_sha_digest,
                archive_staging_receipt,
                archive_container_capability_row_id,
                staged_entry_manifest,
                publication_manifest,
                publication_observation,
                retained_entry_names,
                support_arming_receipt_ids: receipt_ids.support_arming,
                support_prerequisite_receipt_ids: receipt_ids.support_prerequisite,
                support_cancellation_receipt_ids: receipt_ids.support_cancellation,
                support_recovery_receipt_ids: receipt_ids.support_recovery,
                pre_arm_recovery_receipt_ids: receipt_ids.pre_arm_recovery,
                pre_arm_cancellation_recoveries,
                handoff_retention_releases,
                deferred_advance_consumption_receipt_ids,
                retained_lineage_digest,
                preview_digest,
            }),
        };
        Ok(Self { data, status })
    }

    pub(crate) const fn data(&self) -> &ArchiveData {
        &self.data
    }

    pub(crate) const fn status(&self) -> &TaskArchiveStatus {
        &self.status
    }

    pub(crate) fn into_parts(self) -> (ArchiveData, TaskArchiveStatus) {
        (self.data, self.status)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::branched_development::contracts::errors::{
        ReservationOwnerRef, StableErrorCode,
    };
    use crate::domain::branched_development::contracts::prearm_recovery::archive_selective_update_proof_test_only;
    use crate::domain::branched_development::contracts::recovery::{
        prearm_archive_receipt_outcome_witness_fixture_test_only,
        ArchivePublicationByteObservationTestParts, HandoffRetentionReleaseReceipt,
        PublishedArchiveSha256, RecoveryPlanStatus,
    };
    use crate::domain::branched_development::contracts::scalars::RepositoryIdentityComponent;
    use crate::domain::branched_development::contracts::schema::audit_json_schema;
    use crate::domain::branched_development::contracts::status::{
        ApprovedCleanupAttempt, ArtifactHashStatuses, CleanupEligibilityStatus,
        ExistingTaskDeferredState, ExistingTaskStatusAuthority, ExistingTaskStatusCollections,
        OwnedLockStatuses, PendingDecisionStatuses, RecentOperations, ResumeHandle, ResumeHandles,
        TaskAnchorStatuses, ValidationGateStatuses, WorkspaceResumeHandle,
    };
    use crate::domain::branched_development::{OperationId, TaskId};
    use schemars::{schema_for, JsonSchema};
    use serde::de::DeserializeOwned;
    use serde_json::{json, Value};

    const ID_1: &str = "11111111-1111-4111-8111-111111111111";
    const ID_2: &str = "22222222-2222-4222-8222-222222222222";
    const ID_3: &str = "33333333-3333-4333-8333-333333333333";
    const ID_4: &str = "44444444-4444-4444-8444-444444444444";
    const ID_5: &str = "55555555-5555-4555-8555-555555555555";
    const ID_6: &str = "66666666-6666-4666-8666-666666666666";
    const ID_7: &str = "77777777-7777-4777-8777-777777777777";
    const ID_8: &str = "88888888-8888-4888-8888-888888888888";
    const ID_9: &str = "99999999-9999-4999-8999-999999999999";

    fn id(value: &str) -> UnicaId {
        UnicaId::parse(value).unwrap()
    }

    fn project(value: &str) -> ProjectId {
        ProjectId::parse(value).unwrap()
    }

    fn operation(value: &str) -> OperationId {
        OperationId::parse(value).unwrap()
    }

    fn digest(character: char) -> Sha256Digest {
        Sha256Digest::parse(&character.to_string().repeat(64)).unwrap()
    }

    fn capability(value: &str) -> CapabilityRowId {
        CapabilityRowId::parse(value).unwrap()
    }

    fn exact_prearm_archive_authority_parts() -> PreArmCancellationArchiveAuthorityTestParts {
        let fixture = prearm_archive_receipt_outcome_witness_fixture_test_only();
        let selective_update_proof = archive_selective_update_proof_test_only(&fixture.plan);
        let receipts = fixture.progress.completed_realized_receipts().unwrap();
        let cancellation = receipts
            .iter()
            .find(|receipt| {
                receipt.effect_kind()
                    == crate::domain::branched_development::contracts::prearm_recovery::PreArmCancellationEffectKind::AuthorizationCancellation
            })
            .unwrap();
        let recovery = receipts
            .iter()
            .find(|receipt| {
                receipt.effect_kind()
                    == crate::domain::branched_development::contracts::prearm_recovery::PreArmCancellationEffectKind::RecoveryFinalization
            })
            .unwrap();
        let support_cancellation_receipt_id = cancellation.receipt_id().clone();
        let support_cancellation_receipt_digest = cancellation.receipt_digest().clone();
        let pre_arm_recovery_receipt_id = recovery.receipt_id().clone();
        let pre_arm_recovery_receipt_digest = recovery.receipt_digest().clone();
        let finalization_plan_digest = fixture.plan.finalization_plan_digest().clone();
        let receipt_plan_digest = fixture.plan.receipt_plan().receipt_plan_digest().clone();
        let finalization_attempt_audit_digest =
            fixture.progress.attempt_audit_digest().unwrap().clone();
        let post_apply_history_partition = fixture.observation.history_partition().clone();
        let post_release_observed_history_cursor =
            post_apply_history_partition.through_inclusive().clone();
        let resulting_phase = fixture.plan.planned_result_phase();
        let recovery_receipt_digest = digest('d');
        PreArmCancellationArchiveAuthorityTestParts {
            receipt_outcome_witness: fixture.witness,
            effect_observation: fixture.observation.clone(),
            finalization_plan: fixture.plan.clone(),
            finalization_recheck_evidence: fixture.recheck_evidence.clone(),
            completed_finalization_progress: fixture.progress.clone(),
            support_cancellation_receipt_id: support_cancellation_receipt_id.clone(),
            support_cancellation_receipt_digest: support_cancellation_receipt_digest.clone(),
            pre_arm_recovery_receipt_id: pre_arm_recovery_receipt_id.clone(),
            pre_arm_recovery_receipt_digest: pre_arm_recovery_receipt_digest.clone(),
            recovery_receipt_digest: recovery_receipt_digest.clone(),
            selective_update_proof: selective_update_proof.clone(),
            post_release_observed_history_cursor: post_release_observed_history_cursor.clone(),
            post_apply_history_partition: post_apply_history_partition.clone(),
            deferred_repository_advance: None,
            resulting_phase,
            terminal_prior_support_action_id: fixture.observation.support_action_id().clone(),
            terminal_effect_observation: fixture.observation,
            terminal_finalization_plan: fixture.plan,
            terminal_finalization_recheck_evidence: fixture.recheck_evidence.clone(),
            terminal_completed_finalization_progress: fixture.progress,
            terminal_finalization_plan_digest: finalization_plan_digest,
            terminal_receipt_plan_digest: receipt_plan_digest,
            terminal_recheck_evidence_digest: fixture.recheck_evidence.evidence_digest().clone(),
            terminal_finalization_attempt_audit_digest: finalization_attempt_audit_digest,
            terminal_support_cancellation_receipt_id: support_cancellation_receipt_id,
            terminal_support_cancellation_receipt_digest: support_cancellation_receipt_digest,
            terminal_pre_arm_recovery_receipt_id: pre_arm_recovery_receipt_id,
            terminal_pre_arm_recovery_receipt_digest: pre_arm_recovery_receipt_digest,
            terminal_recovery_receipt_digest: recovery_receipt_digest,
            terminal_selective_update_proof_digest: selective_update_proof.proof_digest().clone(),
            terminal_selective_update_proof: selective_update_proof,
            terminal_post_release_observed_history_cursor: post_release_observed_history_cursor,
            terminal_post_apply_history_partition_digest: post_apply_history_partition
                .partition_digest()
                .clone(),
            terminal_post_apply_history_partition: post_apply_history_partition,
            terminal_deferred_repository_advance: None,
            terminal_deferred_repository_advance_digest: None,
            terminal_resulting_phase: resulting_phase,
        }
    }

    fn exact_prearm_archive_entry() -> PreArmCancellationArchiveEntry {
        PreArmCancellationArchiveEntry::from_archive_lineage(
            PreArmCancellationArchiveAuthority::test_only(exact_prearm_archive_authority_parts())
                .unwrap(),
        )
        .unwrap()
    }

    fn handoff_release() -> HandoffRetentionReleaseReceipt {
        HandoffRetentionReleaseReceipt::test_only(
            id(ID_6),
            id(ID_7),
            digest('6'),
            id(ID_8),
            digest('7'),
        )
    }

    fn handoff_release_member(release: &HandoffRetentionReleaseReceipt) -> ArchivePublicationEntry {
        let entry_name = ArchiveEntryName::parse(&format!(
            "handoff-releases/{}.json",
            release.retention_lease_id().as_str()
        ))
        .unwrap();
        let bytes = serde_json_canonicalizer::to_vec(release).unwrap();
        ArchivePublicationEntry::test_only(entry_name, hash_member_bytes(&bytes).unwrap())
    }

    fn publication_byte_observation_with(
        observed: &ArchivePublicationByteObservation,
        archive_container_capability_row_id: CapabilityRowId,
        parsed_entry_set_digest: Sha256Digest,
    ) -> ArchivePublicationByteObservation {
        ArchivePublicationByteObservation::test_only(ArchivePublicationByteObservationTestParts {
            archive_container_capability_row_id,
            generation_id: observed.generation_id().clone(),
            final_byte_observation_id: observed.final_byte_observation_id().clone(),
            archive_id: observed.archive_id().clone(),
            publication_manifest_digest: observed.publication_manifest_digest().clone(),
            parsed_entry_set_digest,
            final_archive_size: observed.final_archive_size(),
            final_archive_sha256: PublishedArchiveSha256::test_only(
                observed.final_archive_sha256().as_digest().clone(),
            ),
            durable_write_receipt_id: observed.durable_write_receipt_id().clone(),
        })
        .unwrap()
    }

    fn committed_status() -> ExistingTaskStatusData {
        let workspace_id = id(ID_2);
        let handles = ResumeHandles::new(vec![ResumeHandle::Workspace(
            WorkspaceResumeHandle::new(workspace_id.clone()),
        )])
        .unwrap();
        let deferred = ExistingTaskDeferredState::without_terminal_support(&handles).unwrap();
        let authority = ExistingTaskStatusAuthority::new(
            id(ID_1),
            None,
            ExistingTaskStatusCollections::new(
                PendingDecisionStatuses::new(Vec::new()).unwrap(),
                TaskAnchorStatuses::new(Vec::new()).unwrap(),
                OwnedLockStatuses::new(Vec::new()).unwrap(),
                ValidationGateStatuses::new(Vec::new()).unwrap(),
                ArtifactHashStatuses::new(Vec::new()).unwrap(),
                handles,
                RecentOperations::new(Vec::new()).unwrap(),
            ),
            deferred,
            CleanupEligibilityStatus::ineligible_without_archive(vec![
                StableErrorCode::CleanupNotAllowed,
            ])
            .unwrap(),
        )
        .unwrap();
        ExistingTaskStatusData::workspace(TaskPhase::CommittedAndUnlocked, workspace_id, authority)
            .unwrap()
    }

    fn success_archive_preview(names: &[&str]) -> ArchivePreviewData {
        archive_preview(TaskArchiveOutcome::Success, names)
    }

    fn archive_preview(outcome: TaskArchiveOutcome, names: &[&str]) -> ArchivePreviewData {
        ArchivePreviewData::from_authority(
            ArchivePreviewAuthority::test_only(
                outcome,
                committed_status(),
                names
                    .iter()
                    .map(|name| ArchiveEntryName::parse(name).unwrap())
                    .collect(),
            )
            .unwrap(),
        )
        .unwrap()
    }

    fn blocker_set() -> Vec<NotCreatedBlocker> {
        let task_id = TaskId::parse("TASK-12").unwrap();
        vec![
            NotCreatedBlocker::operation_in_progress(OperationInProgressContext::new_test_only(
                operation(ID_1),
                digest('a'),
            )),
            NotCreatedBlocker::target_reservation_busy(
                TargetReservationBusyContext::new_test_only(
                    digest('a'),
                    digest('b'),
                    digest('c'),
                    ReservationOwnerRef::start_attempt_test_only(
                        project(ID_1),
                        task_id.clone(),
                        operation(ID_2),
                    ),
                ),
            ),
            NotCreatedBlocker::repository_account_reservation_busy(
                RepositoryAccountReservationBusyContext::new_test_only(
                    digest('a'),
                    digest('b'),
                    digest('d'),
                    ReservationOwnerRef::unresolved_task_test_only(
                        project(ID_1),
                        task_id,
                        id(ID_2),
                        TaskPhase::Created,
                    ),
                ),
            ),
            NotCreatedBlocker::project_identity_collision(
                ProjectDigestProfileStateContext::new_test_only(
                    project(ID_1),
                    digest('a'),
                    digest('b'),
                )
                .unwrap(),
            ),
            NotCreatedBlocker::state_root_relocation_required(
                ProjectDigestProfileStateContext::new_test_only(
                    project(ID_1),
                    digest('b'),
                    digest('c'),
                )
                .unwrap(),
            ),
        ]
    }

    fn reserved_start_authority() -> ReservedOriginalStartAuthority {
        let project_id = project(ID_1);
        let instance_id = id(ID_2);
        ReservedOriginalStartAuthority::test_only(ReservedOriginalStartAuthorityTestParts {
            instance_id: instance_id.clone(),
            project_id: project_id.clone(),
            profile: LocalProfileName::parse("safe-reserved").unwrap(),
            original_infobase_kind: OriginalInfobaseKind::File,
            repository_transport: RepositoryTransport::Server,
            capability_row_id: capability(ID_3),
            pre_arm_cancellation_guard_capability_id: capability(ID_4),
            retention_provider_capability_row_ids: vec![capability(ID_5)],
            profile_declares_recovery_distribution_sources: true,
            manual_actor_username: RepositoryUsername::parse("integration").unwrap(),
            reserved_integration_username: RepositoryUsername::parse("integration").unwrap(),
            work_root_locator: OwnedTargetLocator::new(
                project_id,
                instance_id,
                OwnedTargetRole::InstanceRoot,
            ),
            commit_comment_preview: Comment::parse("BD TASK-12: frozen comment").unwrap(),
            reserved_original_lease_capability_id: capability(ID_5),
        })
        .unwrap()
    }

    fn reserved_start() -> StartData {
        StartData::reserved_original(reserved_start_authority()).unwrap()
    }

    fn separate_start_authority() -> SeparateWorkingInfobaseStartAuthority {
        let project_id = project(ID_1);
        let instance_id = id(ID_2);
        SeparateWorkingInfobaseStartAuthority::test_only(
            SeparateWorkingInfobaseStartAuthorityTestParts {
                instance_id: instance_id.clone(),
                project_id: project_id.clone(),
                profile: LocalProfileName::parse("safe-separate").unwrap(),
                original_infobase_kind: OriginalInfobaseKind::ClientServer,
                repository_transport: RepositoryTransport::File,
                capability_row_id: capability(ID_3),
                pre_arm_cancellation_guard_capability_id: capability(ID_4),
                retention_provider_capability_row_ids: Vec::new(),
                profile_declares_recovery_distribution_sources: false,
                manual_actor_username: RepositoryUsername::parse("developer").unwrap(),
                work_root_locator: OwnedTargetLocator::new(
                    project_id,
                    instance_id,
                    OwnedTargetRole::InstanceRoot,
                ),
                commit_comment_preview: Comment::parse("BD TASK-12: frozen comment").unwrap(),
                manual_working_infobase_identity: ManualWorkingInfobaseIdentity::new(
                    RepositoryIdentityComponent::parse("DEV-PC").unwrap(),
                    RepositoryIdentityComponent::parse("TaskIB").unwrap(),
                )
                .unwrap(),
                manual_working_infobase_inspection_capability_id: capability(ID_5),
            },
        )
        .unwrap()
    }

    fn separate_start() -> StartData {
        StartData::separate_working_infobase(separate_start_authority()).unwrap()
    }

    fn preview_lineage(
        preview: &ArchivePreviewData,
    ) -> (TaskArchiveOutcome, CanonicalArchiveEntryNames, Sha256Digest) {
        match preview {
            ArchivePreviewData::Success(value) => (
                TaskArchiveOutcome::Success,
                value.retained_entry_names.clone(),
                value.preview_digest.clone(),
            ),
            ArchivePreviewData::Abandoned(value) => (
                TaskArchiveOutcome::Abandoned,
                value.retained_entry_names.clone(),
                value.preview_digest.clone(),
            ),
        }
    }

    fn archive_authority(
        archive_id: UnicaId,
        outcome: TaskArchiveOutcome,
        staged_hash_character: char,
        final_hash_character: char,
    ) -> ArchiveDataAuthority {
        let entry = ArchivePublicationEntry::test_only(
            ArchiveEntryName::parse("evidence/task.json").unwrap(),
            digest('9'),
        );
        let staged_entry_manifest = StagedArchiveEntryManifest::from_parser(
            StagedArchiveEntryManifestAuthority::test_only(vec![entry.clone()]).unwrap(),
        )
        .unwrap();
        let preview = archive_preview(outcome, &["evidence/task.json"]);
        let (preview_outcome, retained_entry_names, preview_digest) = preview_lineage(&preview);
        let approved_preview = ApprovedArchivePreviewAuthority::approve_test_only(preview);
        let archive_container_capability_row_id = capability(ID_4);
        let pre_arm_cancellation_recovery = exact_prearm_archive_entry();
        let support_cancellation_receipt_ids = vec![pre_arm_cancellation_recovery
            .support_cancellation_receipt_id()
            .clone()];
        let pre_arm_recovery_receipt_ids = vec![pre_arm_cancellation_recovery
            .pre_arm_recovery_receipt_id()
            .clone()];
        let receipt_ids = ArchiveReceiptIdGroups::new(
            Vec::new(),
            Vec::new(),
            support_cancellation_receipt_ids.clone(),
            Vec::new(),
            pre_arm_recovery_receipt_ids.clone(),
        )
        .unwrap();
        let pre_arm_cancellation_recoveries =
            PreArmCancellationArchiveEntries::new(vec![pre_arm_cancellation_recovery]).unwrap();
        let deferred_advance_consumption_ids = vec![id(ID_9)];
        let deferred_advance_consumption_receipt_ids =
            CanonicalUnicaIds::new(deferred_advance_consumption_ids.clone()).unwrap();
        let handoff_lineage_digest = task_digest(
            &HandoffLineageDigestRecord {
                archive_id: archive_id.clone(),
                outcome: preview_outcome,
                schema_version: ArchiveSchemaVersion::current(),
                archive_container_capability_row_id: archive_container_capability_row_id.clone(),
                staged_entry_manifest: staged_entry_manifest.clone(),
                retained_entry_names: retained_entry_names.clone(),
                support_arming_receipt_ids: receipt_ids.support_arming.clone(),
                support_prerequisite_receipt_ids: receipt_ids.support_prerequisite.clone(),
                support_cancellation_receipt_ids: receipt_ids.support_cancellation.clone(),
                support_recovery_receipt_ids: receipt_ids.support_recovery.clone(),
                pre_arm_recovery_receipt_ids: receipt_ids.pre_arm_recovery.clone(),
                pre_arm_cancellation_recoveries: pre_arm_cancellation_recoveries.clone(),
                deferred_advance_consumption_receipt_ids: deferred_advance_consumption_receipt_ids
                    .clone(),
                preview_digest: preview_digest.clone(),
            },
            "test handoff lineage digest failed",
        )
        .unwrap();
        let provider_boundary_digest_values = vec![digest('e')];
        let provider_boundary_digests =
            CanonicalDigests::canonical(provider_boundary_digest_values.clone()).unwrap();
        let frozen_provider_boundary_digest = task_digest(
            &FrozenProviderBoundaryDigestRecord {
                provider_boundary_digests: provider_boundary_digests.clone(),
            },
            "test provider-boundary digest failed",
        )
        .unwrap();
        let archive_staging_receipt = ArchiveStagingReceipt::test_only(
            id(ID_1),
            archive_id.clone(),
            handoff_lineage_digest,
            frozen_provider_boundary_digest,
            digest(staged_hash_character),
            id(ID_2),
        )
        .unwrap();
        let handoff_release = handoff_release();
        let handoff_release_member = handoff_release_member(&handoff_release);
        let handoff_retention_releases =
            HandoffRetentionReleaseReceipts::new(vec![handoff_release]).unwrap();
        let retained_lineage_digest = task_digest(
            &RetainedArchiveLineageDigestRecord {
                archive_id: archive_id.clone(),
                outcome: preview_outcome,
                schema_version: ArchiveSchemaVersion::current(),
                archive_container_capability_row_id: archive_container_capability_row_id.clone(),
                staged_entry_manifest: staged_entry_manifest.clone(),
                retained_entry_names: retained_entry_names.clone(),
                support_arming_receipt_ids: receipt_ids.support_arming,
                support_prerequisite_receipt_ids: receipt_ids.support_prerequisite,
                support_cancellation_receipt_ids: receipt_ids.support_cancellation,
                support_recovery_receipt_ids: receipt_ids.support_recovery,
                pre_arm_recovery_receipt_ids: receipt_ids.pre_arm_recovery,
                pre_arm_cancellation_recoveries: pre_arm_cancellation_recoveries.clone(),
                deferred_advance_consumption_receipt_ids: deferred_advance_consumption_receipt_ids
                    .clone(),
                preview_digest,
                archive_staging_receipt_digest: archive_staging_receipt.receipt_digest().clone(),
                handoff_retention_releases: handoff_retention_releases.clone(),
            },
            "test retained lineage digest failed",
        )
        .unwrap();
        let publication_entries =
            ArchivePublicationEntries::new(vec![entry.clone(), handoff_release_member]).unwrap();
        let release_digests =
            CanonicalDigests::from_aligned(&handoff_retention_releases.release_receipt_digests())
                .unwrap();
        let publication_manifest =
            ArchivePublicationManifest::from_record(ArchivePublicationManifestDigestRecord {
                archive_id: archive_id.clone(),
                outcome: preview_outcome,
                schema_version: ArchiveSchemaVersion::current(),
                archive_container_capability_row_id: archive_container_capability_row_id.clone(),
                staged_archive_sha256: digest(staged_hash_character),
                staged_entry_manifest_digest: staged_entry_manifest
                    .staged_entry_manifest_digest()
                    .clone(),
                archive_staging_receipt_digest: archive_staging_receipt.receipt_digest().clone(),
                retained_lineage_digest,
                handoff_release_receipt_digests: release_digests,
                entries: publication_entries.clone(),
            })
            .unwrap();
        let parsed_entry_set_digest = task_digest(
            &ArchiveParsedEntrySetDigestRecord {
                entries: publication_entries.clone(),
            },
            "test parsed entry-set digest failed",
        )
        .unwrap();
        let publication_byte_observation = ArchivePublicationByteObservation::test_only(
            ArchivePublicationByteObservationTestParts {
                archive_container_capability_row_id: archive_container_capability_row_id.clone(),
                generation_id: id(ID_3),
                final_byte_observation_id: id(ID_4),
                archive_id: archive_id.clone(),
                publication_manifest_digest: publication_manifest.manifest_digest,
                parsed_entry_set_digest,
                final_archive_size: 4096,
                final_archive_sha256: PublishedArchiveSha256::test_only(digest(
                    final_hash_character,
                )),
                durable_write_receipt_id: id(ID_5),
            },
        )
        .unwrap();
        ArchiveDataAuthority::test_only(ArchiveDataAuthorityTestParts {
            archive_id,
            approved_preview,
            archive_container_capability_row_id,
            archive_staging_receipt,
            staged_entry_manifest,
            publication_entries: publication_entries.0,
            publication_observation_id: id(ID_5),
            publication_byte_observation,
            support_arming_receipt_ids: Vec::new(),
            support_prerequisite_receipt_ids: Vec::new(),
            support_cancellation_receipt_ids,
            support_recovery_receipt_ids: Vec::new(),
            pre_arm_recovery_receipt_ids,
            pre_arm_cancellation_recoveries: pre_arm_cancellation_recoveries.0,
            handoff_retention_releases,
            deferred_advance_consumption_receipt_ids: deferred_advance_consumption_ids,
            provider_boundary_digests: provider_boundary_digest_values,
        })
        .unwrap()
    }

    fn completed_archive(
        archive_id: UnicaId,
        outcome: TaskArchiveOutcome,
    ) -> CompletedArchivePublication {
        CompletedArchivePublication::from_authority(archive_authority(
            archive_id, outcome, 'a', 'f',
        ))
        .unwrap()
    }

    fn cleanup_preview(
        archive: &TaskArchiveStatus,
        owned_targets: Vec<OwnedTargetLocator>,
    ) -> CleanupPreviewData {
        CleanupPreviewData::from_authority(
            CleanupPreviewAuthority::test_only(archive, owned_targets).unwrap(),
        )
        .unwrap()
    }

    fn cleanup_preview_lineage(preview: &CleanupPreviewData) -> (Sha256Digest, Sha256Digest) {
        match preview {
            CleanupPreviewData::Success(value) => {
                (value.marker_digest.clone(), value.preview_digest.clone())
            }
            CleanupPreviewData::Abandoned(value) => {
                (value.marker_digest.clone(), value.preview_digest.clone())
            }
        }
    }

    fn direct_empty_cleanup(archive: &TaskArchiveStatus) -> (CleanupPreviewData, CleanupData) {
        let preview = cleanup_preview(archive, Vec::new());
        let (marker_digest, preview_digest) = cleanup_preview_lineage(&preview);
        let receipt_authority = ApprovedCleanupAttempt::direct_empty_test_only(
            operation(ID_1),
            archive,
            preview_digest,
            marker_digest,
            id(ID_2),
            Vec::new(),
        )
        .unwrap()
        .complete_direct_empty()
        .unwrap()
        .authorize_receipt(id(ID_3))
        .unwrap();
        let data = CleanupData::from_receipt_authority(receipt_authority, archive).unwrap();
        (preview, data)
    }

    fn recovered_cleanup(
        archive: &TaskArchiveStatus,
        owned_targets: Vec<OwnedTargetLocator>,
    ) -> (CleanupPreviewData, CleanupData) {
        let preview = cleanup_preview(archive, owned_targets.clone());
        let (marker_digest, preview_digest) = cleanup_preview_lineage(&preview);
        let resulting_phase = match archive.outcome() {
            TaskArchiveOutcome::Success => TaskPhase::CleanedSuccess,
            TaskArchiveOutcome::Abandoned => TaskPhase::CleanedAbandoned,
        };
        let recovery = RecoveryPlanStatus::cleanup_targets_fixture_test_only(
            operation(ID_1),
            archive.archive_id().clone(),
            owned_targets.clone(),
            resulting_phase,
        )
        .unwrap();
        let observations = recovery
            .cleanup_matching_absence_observations_test_only()
            .unwrap();
        let receipt_authority = ApprovedCleanupAttempt::from_recovery_test_only(
            operation(ID_1),
            archive,
            preview_digest,
            marker_digest,
            id("cccccccc-cccc-4ccc-8ccc-cccccccccccc"),
            owned_targets,
            recovery,
        )
        .unwrap()
        .observe_absences(observations)
        .unwrap()
        .authorize_receipt(id(ID_3))
        .unwrap();
        let data = CleanupData::from_receipt_authority(receipt_authority, archive).unwrap();
        (preview, data)
    }

    fn schema<T: JsonSchema>() -> Value {
        serde_json::to_value(schema_for!(T)).unwrap()
    }

    fn one_of_len<T: JsonSchema>() -> usize {
        schema::<T>()["oneOf"].as_array().unwrap().len()
    }

    fn schema_accepts<T: JsonSchema>(value: &Value) -> bool {
        jsonschema::options()
            .with_draft(jsonschema::Draft::Draft202012)
            .build(&schema::<T>())
            .unwrap()
            .is_valid(value)
    }

    fn collect_object_pointers(value: &Value, pointer: String, output: &mut Vec<String>) {
        match value {
            Value::Object(object) => {
                output.push(pointer.clone());
                for (key, nested) in object {
                    let escaped = key.replace('~', "~0").replace('/', "~1");
                    collect_object_pointers(nested, format!("{pointer}/{escaped}"), output);
                }
            }
            Value::Array(values) => {
                for (index, nested) in values.iter().enumerate() {
                    collect_object_pointers(nested, format!("{pointer}/{index}"), output);
                }
            }
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
        }
    }

    fn assert_recursively_rejects_forbidden_fields<T: JsonSchema>(valid: Value) {
        let contract = schema::<T>();
        let validator = jsonschema::options()
            .with_draft(jsonschema::Draft::Draft202012)
            .build(&contract)
            .unwrap();
        assert!(
            validator.is_valid(&valid),
            "invalid positive fixture: {valid}"
        );
        let mut pointers = Vec::new();
        collect_object_pointers(&valid, String::new(), &mut pointers);
        for pointer in pointers {
            for forbidden in [
                "cwd",
                "localPath",
                "processHandle",
                "pid",
                "credentialRef",
                "password",
                "secret",
                "rawConnectionString",
            ] {
                let mut poisoned = valid.clone();
                poisoned
                    .pointer_mut(&pointer)
                    .unwrap()
                    .as_object_mut()
                    .unwrap()
                    .insert(forbidden.to_owned(), json!("forbidden"));
                assert!(
                    !validator.is_valid(&poisoned),
                    "{} accepted forbidden {forbidden} at {pointer}",
                    T::schema_name()
                );
            }
        }
    }

    fn assert_schema_declares_no_forbidden_properties<T: JsonSchema>() {
        const FORBIDDEN: &[&str] = &[
            "cwd",
            "path",
            "localPath",
            "processId",
            "processHandle",
            "pid",
            "credential",
            "credentialRef",
            "password",
            "secret",
            "token",
            "rawConnection",
            "rawConnectionString",
            "connectionString",
        ];
        fn visit(value: &Value, type_name: &str) {
            match value {
                Value::Object(object) => {
                    if let Some(properties) = object.get("properties").and_then(Value::as_object) {
                        for forbidden in FORBIDDEN {
                            assert!(
                                !properties.contains_key(*forbidden),
                                "{type_name} declares forbidden wire property {forbidden}"
                            );
                        }
                    }
                    for nested in object.values() {
                        visit(nested, type_name);
                    }
                }
                Value::Array(values) => {
                    for nested in values {
                        visit(nested, type_name);
                    }
                }
                Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
            }
        }
        let contract = schema::<T>();
        audit_json_schema(&contract).unwrap();
        visit(&contract, T::schema_name().as_ref());
    }

    fn required_fields<T: JsonSchema>() -> BTreeSet<String> {
        schema::<T>()["required"]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value.as_str().unwrap().to_owned())
            .collect()
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

    assert_not_deserialize_owned!(StartData);
    assert_not_deserialize_owned!(NotCreatedData);
    assert_not_deserialize_owned!(NotCreatedBlocker);
    assert_not_deserialize_owned!(BranchedStatusData);
    assert_not_deserialize_owned!(ArchivePreviewData);
    assert_not_deserialize_owned!(ArchiveEligibilityDigestRecord);
    assert_not_deserialize_owned!(ArchivePreviewDigestRecord);
    assert_not_deserialize_owned!(ArchiveData);
    assert_not_deserialize_owned!(ArchiveStagingReceipt);
    assert_not_deserialize_owned!(ArchivePublicationEntry);
    assert_not_deserialize_owned!(ArchivePublicationEntries);
    assert_not_deserialize_owned!(CanonicalArchiveEntryNames);
    assert_not_deserialize_owned!(CanonicalUnicaIds);
    assert_not_deserialize_owned!(ArchiveReceiptIdGroups);
    assert_not_deserialize_owned!(StagedArchiveEntryManifest);
    assert_not_deserialize_owned!(StagedArchiveEntryManifestDigestRecord);
    assert_not_deserialize_owned!(ArchivePublicationManifest);
    assert_not_deserialize_owned!(ArchivePublicationManifestDigestRecord);
    assert_not_deserialize_owned!(ArchivePublicationObservation);
    assert_not_deserialize_owned!(ArchivePublicationObservationDigestRecord);
    assert_not_deserialize_owned!(ArchiveParsedEntrySetDigestRecord);
    assert_not_deserialize_owned!(HandoffLineageDigestRecord);
    assert_not_deserialize_owned!(FrozenProviderBoundaryDigestRecord);
    assert_not_deserialize_owned!(RetainedArchiveLineageDigestRecord);
    assert_not_deserialize_owned!(PreArmCancellationArchiveEntry);
    assert_not_deserialize_owned!(PreArmCancellationArchiveEntryDigestRecord);
    assert_not_deserialize_owned!(CleanupPreviewData);
    assert_not_deserialize_owned!(CleanupMarkerDigestRecord);
    assert_not_deserialize_owned!(CleanupPreviewDigestRecord);
    assert_not_deserialize_owned!(CleanupData);
    assert_not_deserialize_owned!(ArchiveStatusProjectionAuthority);
    assert_not_deserialize_owned!(ReservedOriginalStartAuthority);
    assert_not_deserialize_owned!(SeparateWorkingInfobaseStartAuthority);
    assert_not_deserialize_owned!(NotCreatedAuthority);
    assert_not_deserialize_owned!(ArchivePreviewAuthority);
    assert_not_deserialize_owned!(ApprovedArchivePreviewAuthority);
    assert_not_deserialize_owned!(CleanupPreviewAuthority);
    assert_not_deserialize_owned!(StagedArchiveEntryManifestAuthority);
    assert_not_deserialize_owned!(ArchiveDataAuthority);
    assert_not_deserialize_owned!(CompletedArchivePublication);
    assert_not_deserialize_owned!(PreArmCancellationArchiveAuthority);
    assert_not_deserialize_owned!(ValidatedPreArmArchiveReceiptOutcomeWitness);

    assert_not_clone!(ReservedOriginalStartAuthority);
    assert_not_clone!(SeparateWorkingInfobaseStartAuthority);
    assert_not_clone!(NotCreatedAuthority);
    assert_not_clone!(ArchivePreviewAuthority);
    assert_not_clone!(ApprovedArchivePreviewAuthority);
    assert_not_clone!(CleanupPreviewAuthority);
    assert_not_clone!(StagedArchiveEntryManifestAuthority);
    assert_not_clone!(ArchiveDataAuthority);
    assert_not_clone!(ArchivePublicationByteObservation);
    assert_not_clone!(PublishedArchiveSha256);
    assert_not_clone!(ArchiveStatusProjectionAuthority);
    assert_not_clone!(CompletedArchivePublication);
    assert_not_clone!(PreArmCancellationArchiveAuthority);
    assert_not_clone!(ValidatedPreArmArchiveReceiptOutcomeWitness);

    #[test]
    fn all_lifecycle_result_and_digest_schemas_are_recursively_closed() {
        for value in [
            schema::<StartData>(),
            schema::<NotCreatedData>(),
            schema::<BranchedStatusData>(),
            schema::<ArchivePreviewData>(),
            schema::<ArchiveEligibilityDigestRecord>(),
            schema::<ArchivePreviewDigestRecord>(),
            schema::<PreArmCancellationArchiveEntry>(),
            schema::<PreArmCancellationArchiveEntryDigestRecord>(),
            schema::<StagedArchiveEntryManifestDigestRecord>(),
            schema::<StagedArchiveEntryManifest>(),
            schema::<ArchivePublicationManifestDigestRecord>(),
            schema::<ArchivePublicationManifest>(),
            schema::<ArchiveParsedEntrySetDigestRecord>(),
            schema::<ArchivePublicationObservationDigestRecord>(),
            schema::<ArchivePublicationObservation>(),
            schema::<HandoffLineageDigestRecord>(),
            schema::<FrozenProviderBoundaryDigestRecord>(),
            schema::<RetainedArchiveLineageDigestRecord>(),
            schema::<ArchiveData>(),
            schema::<CleanupPreviewData>(),
            schema::<CleanupMarkerDigestRecord>(),
            schema::<CleanupPreviewDigestRecord>(),
            schema::<CleanupData>(),
        ] {
            audit_json_schema(&value).unwrap();
        }
    }

    #[test]
    fn lifecycle_results_use_exact_physical_unions() {
        assert_eq!(one_of_len::<StartData>(), 2);
        assert_eq!(one_of_len::<NotCreatedData>(), 2);
        assert_eq!(one_of_len::<NotCreatedBlocker>(), 5);
        assert_eq!(one_of_len::<BranchedStatusData>(), 2);
        assert_eq!(one_of_len::<ArchivePreviewData>(), 2);
        assert_eq!(one_of_len::<ArchiveData>(), 2);
        assert_eq!(one_of_len::<CleanupPreviewData>(), 2);
        assert_eq!(one_of_len::<CleanupData>(), 2);
    }

    #[test]
    fn existing_status_keeps_its_task_status_data_schema_name() {
        let value = schema::<BranchedStatusData>();
        let serialized = serde_json::to_string(&value).unwrap();
        assert!(serialized.contains("TaskStatusData"));
        assert!(!serialized.contains("ExistingTaskStatusData\""));
    }

    #[test]
    fn not_created_schema_correlates_start_allowed_with_blocker_cardinality() {
        assert!(schema_accepts::<NotCreatedData>(&serde_json::json!({
            "exists": false,
            "startAllowed": true,
            "blockers": []
        })));
        assert!(!schema_accepts::<NotCreatedData>(&serde_json::json!({
            "exists": false,
            "startAllowed": false,
            "blockers": []
        })));
    }

    #[test]
    fn not_created_authority_enforces_blocker_order_uniqueness_and_correlation() {
        let allowed = NotCreatedData::from_authority(NotCreatedAuthority::from_coordinator(
            NotCreatedBlockers::new(Vec::new()).unwrap(),
        ));
        assert_eq!(
            serde_json::to_value(&allowed).unwrap(),
            json!({"exists": false, "startAllowed": true, "blockers": []})
        );

        let blockers = blocker_set();
        let blocked = NotCreatedData::from_authority(NotCreatedAuthority::from_coordinator(
            NotCreatedBlockers::new(blockers.clone()).unwrap(),
        ));
        let value = serde_json::to_value(&blocked).unwrap();
        assert_eq!(value["exists"], false);
        assert_eq!(value["startAllowed"], false);
        assert_eq!(
            value["blockers"]
                .as_array()
                .unwrap()
                .iter()
                .map(|blocker| blocker["code"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec![
                "operationInProgress",
                "targetReservationBusy",
                "repositoryAccountReservationBusy",
                "projectIdentityCollision",
                "stateRootRelocationRequired",
            ]
        );
        assert!(schema_accepts::<NotCreatedData>(&value));

        let mut reversed = blockers.clone();
        reversed.swap(0, 1);
        assert!(NotCreatedBlockers::new(reversed).is_err());
        assert!(NotCreatedBlockers::new(vec![blockers[0].clone(), blockers[0].clone()]).is_err());

        let mut wrong_code = value;
        wrong_code["blockers"][0]["code"] = json!("targetReservationBusy");
        assert!(!schema_accepts::<NotCreatedData>(&wrong_code));
    }

    #[test]
    fn start_results_are_exact_mode_specific_leaves_with_frozen_lineage() {
        let reserved = serde_json::to_value(reserved_start()).unwrap();
        assert_eq!(reserved["manualTargetMode"], "reservedOriginal");
        assert_eq!(reserved["projectId"], ID_1);
        assert_eq!(reserved["instanceId"], ID_2);
        assert_eq!(reserved["workRootLocator"]["role"], "instanceRoot");
        assert_eq!(
            reserved["commitCommentPreview"],
            "BD TASK-12: frozen comment"
        );
        assert!(reserved.get("reservedOriginalLeaseCapabilityId").is_some());
        assert!(reserved.get("manualWorkingInfobaseIdentity").is_none());
        assert!(schema_accepts::<StartData>(&reserved));

        let separate = serde_json::to_value(separate_start()).unwrap();
        assert_eq!(separate["manualTargetMode"], "separateWorkingInfobase");
        assert!(separate.get("manualWorkingInfobaseIdentity").is_some());
        assert!(separate
            .get("manualWorkingInfobaseInspectionCapabilityId")
            .is_some());
        assert!(separate.get("reservedOriginalLeaseCapabilityId").is_none());
        assert!(schema_accepts::<StartData>(&separate));

        let mut reserved_splice = reserved.clone();
        reserved_splice["manualWorkingInfobaseIdentity"] =
            separate["manualWorkingInfobaseIdentity"].clone();
        assert!(!schema_accepts::<StartData>(&reserved_splice));
        let mut separate_splice = separate.clone();
        separate_splice["reservedOriginalLeaseCapabilityId"] = json!(ID_5);
        assert!(!schema_accepts::<StartData>(&separate_splice));
        let mut weakened_comment = reserved;
        weakened_comment["commitCommentPreview"] = json!({"summary": "not a Comment"});
        assert!(!schema_accepts::<StartData>(&weakened_comment));
    }

    #[test]
    fn start_authorities_reject_wrong_root_and_noncanonical_provider_rows() {
        let mut wrong_root = reserved_start_authority();
        wrong_root.work_root_locator =
            OwnedTargetLocator::new(project(ID_1), id(ID_2), OwnedTargetRole::Artifact);
        assert!(StartData::reserved_original(wrong_root).is_err());

        assert!(CanonicalCapabilityRowIds::new(vec![capability(ID_4), capability(ID_3)]).is_err());
        assert!(CanonicalCapabilityRowIds::new(vec![capability(ID_3), capability(ID_3)]).is_err());
        let none = CanonicalCapabilityRowIds::new(Vec::new()).unwrap();
        let one = CanonicalCapabilityRowIds::new(vec![capability(ID_3)]).unwrap();
        assert!(validate_retention_provider_presence(&none, true).is_err());
        assert!(validate_retention_provider_presence(&one, false).is_err());
        assert!(validate_retention_provider_presence(&none, false).is_ok());
        assert!(validate_retention_provider_presence(&one, true).is_ok());
    }

    #[test]
    fn branched_status_outer_union_rejects_not_created_existing_field_splices() {
        let not_created = serde_json::to_value(NotCreatedData::from_authority(
            NotCreatedAuthority::from_coordinator(NotCreatedBlockers::new(Vec::new()).unwrap()),
        ))
        .unwrap();
        let existing = serde_json::to_value(committed_status()).unwrap();
        assert!(schema_accepts::<BranchedStatusData>(&not_created));
        assert!(schema_accepts::<BranchedStatusData>(&existing));

        let mut splice = not_created;
        splice["instanceId"] = existing["instanceId"].clone();
        assert!(!schema_accepts::<BranchedStatusData>(&splice));
        let mut reverse_splice = existing;
        reverse_splice["startAllowed"] = json!(true);
        reverse_splice["blockers"] = json!([]);
        assert!(!schema_accepts::<BranchedStatusData>(&reverse_splice));
    }

    #[test]
    fn archive_preview_is_content_bound_and_contains_no_post_effect_fields() {
        let first = success_archive_preview(&["evidence/a.json"]);
        let second = success_archive_preview(&["evidence/b.json"]);
        let abandoned = archive_preview(TaskArchiveOutcome::Abandoned, &["evidence/a.json"]);
        let first_value = serde_json::to_value(&first).unwrap();
        assert_eq!(
            first_value["excludedRoles"],
            json!([
                "instanceRoot",
                "taskInfobase",
                "taskWorkspace",
                "probe",
                "sandbox",
                "artifact",
                "quarantine"
            ])
        );
        assert_ne!(preview_lineage(&first).2, preview_lineage(&second).2);
        assert_ne!(preview_lineage(&first).2, preview_lineage(&abandoned).2);
        for forbidden in [
            "archiveId",
            "sha256",
            "stagedArchiveSha256",
            "receiptId",
            "createdAt",
            "fingerprint",
        ] {
            let mut poisoned = first_value.clone();
            poisoned
                .as_object_mut()
                .unwrap()
                .insert(forbidden.to_owned(), json!(ID_1));
            assert!(
                !schema_accepts::<ArchivePreviewData>(&poisoned),
                "archive preview accepted post-effect field {forbidden}"
            );
        }
    }

    #[test]
    fn archive_names_and_members_reject_noncanonical_reserved_and_case_fold_collisions() {
        for invalid in ["../escape", "/absolute", "данные.json", "CON", "a\\b"] {
            assert!(
                ArchiveEntryName::parse(invalid).is_err(),
                "accepted {invalid}"
            );
        }
        let names = |values: &[&str]| {
            values
                .iter()
                .map(|value| ArchiveEntryName::parse(value).unwrap())
                .collect()
        };
        assert!(CanonicalArchiveEntryNames::new(names(&["b.json", "a.json"])).is_err());
        assert!(CanonicalArchiveEntryNames::new(names(&["A.json", "a.json"])).is_err());
        assert!(CanonicalArchiveEntryNames::new(names(&["archive-manifest.json"])).is_err());
        assert!(CanonicalArchiveEntryNames::new(names(&["handoff-releases/x.json"])).is_err());

        let entry = |name: &str| {
            ArchivePublicationEntry::test_only(ArchiveEntryName::parse(name).unwrap(), digest('a'))
        };
        assert!(StagedArchiveEntryManifestAuthority::test_only(vec![
            entry("z.json"),
            entry("a.json"),
        ])
        .is_err());
        assert!(StagedArchiveEntryManifestAuthority::test_only(vec![
            entry("A.json"),
            entry("a.json"),
        ])
        .is_err());
        assert!(StagedArchiveEntryManifestAuthority::test_only(vec![entry(
            "handoff-releases/11111111-1111-4111-8111-111111111111.json",
        )])
        .is_err());
    }

    #[test]
    fn completed_archive_binds_staged_and_final_layers_and_status_to_one_publication() {
        for outcome in [TaskArchiveOutcome::Success, TaskArchiveOutcome::Abandoned] {
            let completed = completed_archive(id(ID_3), outcome);
            let data = serde_json::to_value(completed.data()).unwrap();
            let status = serde_json::to_value(completed.status()).unwrap();
            assert_eq!(data["archiveId"], ID_3);
            assert_eq!(data["schemaVersion"], "branchedArchiveV1");
            assert_eq!(
                data["archiveStagingReceipt"]["stagedArchiveSha256"],
                digest('a').as_str()
            );
            assert_eq!(
                data["publicationManifest"]["stagedArchiveSha256"],
                digest('a').as_str()
            );
            assert_eq!(data["sha256"], digest('f').as_str());
            assert_eq!(
                data["publicationObservation"]["finalArchiveSha256"],
                digest('f').as_str()
            );
            assert_eq!(
                data["preArmCancellationRecoveries"]
                    .as_array()
                    .unwrap()
                    .len(),
                1
            );
            assert_eq!(
                data["handoffRetentionReleases"].as_array().unwrap().len(),
                1
            );
            assert_eq!(
                data["handoffRetentionReleases"][0]["releaseActionDigest"],
                digest('6').as_str()
            );
            assert_eq!(
                data["handoffRetentionReleases"][0]["releaseReceiptDigest"],
                digest('7').as_str()
            );
            assert_eq!(
                data["supportCancellationReceiptIds"],
                json!([
                    data["preArmCancellationRecoveries"][0]["supportCancellationReceiptId"].clone()
                ])
            );
            assert_eq!(
                data["preArmRecoveryReceiptIds"],
                json!([data["preArmCancellationRecoveries"][0]["preArmRecoveryReceiptId"].clone()])
            );
            assert_eq!(data["deferredAdvanceConsumptionReceiptIds"], json!([ID_9]));
            assert_eq!(
                data["publicationManifest"]["handoffReleaseReceiptDigests"],
                json!([digest('7')])
            );
            assert_eq!(
                data["publicationObservation"]["handoffReleaseReceiptDigests"],
                data["publicationManifest"]["handoffReleaseReceiptDigests"]
            );
            assert_eq!(
                data["publicationManifest"]["entries"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|entry| entry["entryName"].as_str().unwrap())
                    .collect::<Vec<_>>(),
                vec![
                    "evidence/task.json",
                    "handoff-releases/66666666-6666-4666-8666-666666666666.json",
                ]
            );
            let expected_release_member_sha =
                hash_member_bytes(&serde_json_canonicalizer::to_vec(&handoff_release()).unwrap())
                    .unwrap();
            assert_eq!(
                data["publicationManifest"]["entries"][1]["sha256"],
                expected_release_member_sha.as_str()
            );
            let expected_provider_boundary_digest = task_digest(
                &FrozenProviderBoundaryDigestRecord {
                    provider_boundary_digests: CanonicalDigests::canonical(vec![digest('e')])
                        .unwrap(),
                },
                "test provider-boundary digest failed",
            )
            .unwrap();
            assert_eq!(
                data["archiveStagingReceipt"]["frozenProviderBoundaryDigest"],
                expected_provider_boundary_digest.as_str()
            );
            assert_ne!(
                data["sha256"],
                data["archiveStagingReceipt"]["stagedArchiveSha256"]
            );
            assert_eq!(status["archiveId"], data["archiveId"]);
            assert_eq!(status["outcome"], data["outcome"]);
            assert_eq!(status["sha256"], data["sha256"]);
            assert_eq!(
                status["retainedLineageDigest"],
                data["retainedLineageDigest"]
            );
            assert!(data.get("generationId").is_none());
            assert!(schema_accepts::<ArchiveData>(&data));
        }
    }

    #[test]
    fn archive_publication_rejects_identity_member_manifest_and_digest_substitution() {
        let mut wrong_identity = archive_authority(id(ID_3), TaskArchiveOutcome::Success, 'a', 'f');
        wrong_identity.archive_id = id(ID_4);
        assert!(CompletedArchivePublication::from_authority(wrong_identity).is_err());

        let mut wrong_staging_lineage =
            archive_authority(id(ID_3), TaskArchiveOutcome::Success, 'a', 'f');
        let staging = &wrong_staging_lineage.archive_staging_receipt;
        wrong_staging_lineage.archive_staging_receipt = ArchiveStagingReceipt::test_only(
            staging.staging_receipt_id().clone(),
            staging.archive_id().clone(),
            digest('0'),
            staging.frozen_provider_boundary_digest().clone(),
            staging.staged_archive_sha256().clone(),
            staging.durable_write_receipt_id().clone(),
        )
        .unwrap();
        assert!(CompletedArchivePublication::from_authority(wrong_staging_lineage).is_err());

        let mut wrong_member = archive_authority(id(ID_3), TaskArchiveOutcome::Success, 'a', 'f');
        wrong_member.publication_entries =
            ArchivePublicationEntries::new(vec![ArchivePublicationEntry::test_only(
                ArchiveEntryName::parse("evidence/task.json").unwrap(),
                digest('8'),
            )])
            .unwrap();
        assert!(CompletedArchivePublication::from_authority(wrong_member).is_err());

        let mut wrong_observation =
            archive_authority(id(ID_3), TaskArchiveOutcome::Success, 'a', 'f');
        let observed = &wrong_observation.publication_byte_observation;
        wrong_observation.publication_byte_observation =
            ArchivePublicationByteObservation::test_only(
                ArchivePublicationByteObservationTestParts {
                    archive_container_capability_row_id: observed
                        .archive_container_capability_row_id()
                        .clone(),
                    generation_id: observed.generation_id().clone(),
                    final_byte_observation_id: observed.final_byte_observation_id().clone(),
                    archive_id: observed.archive_id().clone(),
                    publication_manifest_digest: digest('0'),
                    parsed_entry_set_digest: observed.parsed_entry_set_digest().clone(),
                    final_archive_size: observed.final_archive_size(),
                    final_archive_sha256: PublishedArchiveSha256::test_only(digest('f')),
                    durable_write_receipt_id: observed.durable_write_receipt_id().clone(),
                },
            )
            .unwrap();
        assert!(CompletedArchivePublication::from_authority(wrong_observation).is_err());
    }

    #[test]
    fn archive_publication_rejects_handoff_member_and_release_receipt_substitutions() {
        let mut missing_release_member =
            archive_authority(id(ID_3), TaskArchiveOutcome::Success, 'a', 'f');
        missing_release_member.publication_entries = ArchivePublicationEntries::new(
            missing_release_member
                .staged_entry_manifest
                .entries()
                .to_vec(),
        )
        .unwrap();
        assert!(CompletedArchivePublication::from_authority(missing_release_member).is_err());

        let mut extra_release_member =
            archive_authority(id(ID_3), TaskArchiveOutcome::Success, 'a', 'f');
        let mut entries = extra_release_member.publication_entries.as_slice().to_vec();
        entries.push(ArchivePublicationEntry::test_only(
            ArchiveEntryName::parse("unexpected.json").unwrap(),
            digest('0'),
        ));
        extra_release_member.publication_entries = ArchivePublicationEntries::new(entries).unwrap();
        assert!(CompletedArchivePublication::from_authority(extra_release_member).is_err());

        let mut wrong_release_member =
            archive_authority(id(ID_3), TaskArchiveOutcome::Success, 'a', 'f');
        let mut entries = wrong_release_member
            .staged_entry_manifest
            .entries()
            .to_vec();
        entries.push(ArchivePublicationEntry::test_only(
            ArchiveEntryName::parse(&format!("handoff-releases/{ID_6}.json")).unwrap(),
            digest('0'),
        ));
        wrong_release_member.publication_entries = ArchivePublicationEntries::new(entries).unwrap();
        assert!(CompletedArchivePublication::from_authority(wrong_release_member).is_err());

        let mut wrong_action_digest =
            archive_authority(id(ID_3), TaskArchiveOutcome::Success, 'a', 'f');
        let release = &wrong_action_digest.handoff_retention_releases.as_slice()[0];
        wrong_action_digest.handoff_retention_releases =
            HandoffRetentionReleaseReceipts::new(vec![HandoffRetentionReleaseReceipt::test_only(
                release.retention_lease_id().clone(),
                release.release_action_id().clone(),
                digest('0'),
                release.release_receipt_id().clone(),
                release.release_receipt_digest().clone(),
            )])
            .unwrap();
        assert!(CompletedArchivePublication::from_authority(wrong_action_digest).is_err());

        let mut wrong_receipt_digest =
            archive_authority(id(ID_3), TaskArchiveOutcome::Success, 'a', 'f');
        let release = &wrong_receipt_digest.handoff_retention_releases.as_slice()[0];
        wrong_receipt_digest.handoff_retention_releases =
            HandoffRetentionReleaseReceipts::new(vec![HandoffRetentionReleaseReceipt::test_only(
                release.retention_lease_id().clone(),
                release.release_action_id().clone(),
                release.release_action_digest().clone(),
                release.release_receipt_id().clone(),
                digest('0'),
            )])
            .unwrap();
        assert!(CompletedArchivePublication::from_authority(wrong_receipt_digest).is_err());
    }

    #[test]
    fn archive_publication_rejects_final_byte_capability_and_parsed_set_substitutions() {
        let mut wrong_parsed_entry_set =
            archive_authority(id(ID_3), TaskArchiveOutcome::Success, 'a', 'f');
        wrong_parsed_entry_set.publication_byte_observation = publication_byte_observation_with(
            &wrong_parsed_entry_set.publication_byte_observation,
            wrong_parsed_entry_set
                .archive_container_capability_row_id
                .clone(),
            digest('0'),
        );
        assert!(CompletedArchivePublication::from_authority(wrong_parsed_entry_set).is_err());

        let mut wrong_capability =
            archive_authority(id(ID_3), TaskArchiveOutcome::Success, 'a', 'f');
        wrong_capability.publication_byte_observation = publication_byte_observation_with(
            &wrong_capability.publication_byte_observation,
            capability(ID_5),
            wrong_capability
                .publication_byte_observation
                .parsed_entry_set_digest()
                .clone(),
        );
        assert!(CompletedArchivePublication::from_authority(wrong_capability).is_err());
    }

    #[test]
    fn archive_publication_rejects_provider_and_deferred_lineage_substitutions() {
        let mut wrong_provider = archive_authority(id(ID_3), TaskArchiveOutcome::Success, 'a', 'f');
        wrong_provider.provider_boundary_digests =
            CanonicalDigests::canonical(vec![digest('0')]).unwrap();
        assert!(CompletedArchivePublication::from_authority(wrong_provider).is_err());

        let mut wrong_deferred = archive_authority(id(ID_3), TaskArchiveOutcome::Success, 'a', 'f');
        wrong_deferred.deferred_advance_consumption_receipt_ids =
            CanonicalUnicaIds::new(vec![id(ID_8)]).unwrap();
        assert!(CompletedArchivePublication::from_authority(wrong_deferred).is_err());
    }

    #[test]
    fn archive_receipt_groups_are_canonical_disjoint_and_exactly_cover_prearm_ids() {
        assert!(ArchiveReceiptIdGroups::new(
            vec![id(ID_2), id(ID_1)],
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        )
        .is_err());
        assert!(ArchiveReceiptIdGroups::new(
            vec![id(ID_1)],
            vec![id(ID_1)],
            Vec::new(),
            Vec::new(),
            Vec::new(),
        )
        .is_err());

        let empty_entries = PreArmCancellationArchiveEntries::new(Vec::new()).unwrap();
        let exact =
            ArchiveReceiptIdGroups::new(Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new())
                .unwrap();
        assert!(exact.validate_prearm_entries(&empty_entries).is_ok());
        let spurious_recovery = ArchiveReceiptIdGroups::new(
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            vec![id(ID_1)],
        )
        .unwrap();
        assert!(spurious_recovery
            .validate_prearm_entries(&empty_entries)
            .is_err());
    }

    #[test]
    fn prearm_archive_entries_reject_duplicate_cancellation_receipt_ids() {
        let mut first = exact_prearm_archive_entry();
        first.support_action_id = id(ID_6);
        let mut duplicate_cancellation = first.clone();
        duplicate_cancellation.support_action_id = id(ID_7);
        duplicate_cancellation.pre_arm_recovery_receipt_id = id(ID_9);
        assert!(
            PreArmCancellationArchiveEntries::new(vec![first, duplicate_cancellation]).is_err()
        );
    }

    #[test]
    fn prearm_archive_authority_requires_its_exact_action_outcome_witness() {
        let authority =
            PreArmCancellationArchiveAuthority::test_only(exact_prearm_archive_authority_parts())
                .unwrap();
        let entry = PreArmCancellationArchiveEntry::from_archive_lineage(authority).unwrap();
        assert_eq!(entry.resulting_phase, TaskPhase::Synchronized);

        let mut foreign_progress = exact_prearm_archive_authority_parts();
        foreign_progress.completed_finalization_progress =
            PreArmCancellationFinalizationAttemptProgress::not_started_test_only(id(ID_5));
        assert!(PreArmCancellationArchiveAuthority::test_only(foreign_progress).is_err());
    }

    #[test]
    fn prearm_archive_schema_requires_the_full_archive_lineage_projection() {
        let expected_record: BTreeSet<_> = [
            "supportActionId",
            "effectObservation",
            "finalizationPlan",
            "finalizationPlanDigest",
            "receiptPlanDigest",
            "finalizationRecheckEvidence",
            "completedFinalizationProgress",
            "finalizationAttemptAuditDigest",
            "supportCancellationReceiptId",
            "supportCancellationReceiptDigest",
            "preArmRecoveryReceiptId",
            "preArmRecoveryReceiptDigest",
            "recoveryReceiptDigest",
            "selectiveUpdateProof",
            "postReleaseObservedHistoryCursor",
            "postApplyHistoryPartition",
            "resultingPhase",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();
        assert_eq!(
            required_fields::<PreArmCancellationArchiveEntryDigestRecord>(),
            expected_record
        );
        let mut expected_entry = expected_record;
        expected_entry.insert("entryDigest".to_owned());
        assert_eq!(
            required_fields::<PreArmCancellationArchiveEntry>(),
            expected_entry
        );
        let wire = serde_json::to_string(&schema::<PreArmCancellationArchiveEntry>()).unwrap();
        for nested in [
            "PreArmCancellationEffectObservation",
            "PreArmCancellationFinalizationPlan",
            "PreArmCancellationFinalizationRecheckEvidence",
            "PreArmCancellationFinalizationAttemptProgress",
            "SelectiveRepositoryUpdateProof",
            "RepositoryHistoryPartition",
        ] {
            assert!(
                wire.contains(nested),
                "missing nested terminal contract {nested}"
            );
        }
    }

    #[test]
    fn cleanup_preview_and_applied_results_preserve_full_target_and_phase_lineage() {
        for outcome in [TaskArchiveOutcome::Success, TaskArchiveOutcome::Abandoned] {
            let archive_publication = completed_archive(id(ID_3), outcome);
            let archive = archive_publication.status().clone();
            let owned_targets = [
                OwnedTargetRole::InstanceRoot,
                OwnedTargetRole::TaskInfobase,
                OwnedTargetRole::TaskWorkspace,
                OwnedTargetRole::Probe,
                OwnedTargetRole::Sandbox,
                OwnedTargetRole::Artifact,
                OwnedTargetRole::Quarantine,
            ]
            .into_iter()
            .map(|role| OwnedTargetLocator::new(project(ID_1), id(ID_2), role))
            .collect::<Vec<_>>();
            let expected_roles = json!([
                "instanceRoot",
                "taskInfobase",
                "taskWorkspace",
                "probe",
                "sandbox",
                "artifact",
                "quarantine"
            ]);
            let (preview, data) = recovered_cleanup(&archive, owned_targets.clone());
            let preview_value = serde_json::to_value(&preview).unwrap();
            let data_value = serde_json::to_value(&data).unwrap();
            assert_eq!(preview_value["ownedTargets"], json!(owned_targets));
            assert_eq!(preview_value["removableRoles"], expected_roles);
            assert_eq!(data_value["removedRoles"], expected_roles);
            assert_eq!(data_value["retainedArchiveId"], ID_3);
            assert_eq!(data_value["previewDigest"], preview_value["previewDigest"]);
            assert_eq!(data_value["markerDigest"], preview_value["markerDigest"]);
            let expected_phase = match outcome {
                TaskArchiveOutcome::Success => "cleanedSuccess",
                TaskArchiveOutcome::Abandoned => "cleanedAbandoned",
            };
            assert_eq!(
                data_value["cleanupReceipt"]["resultingPhase"],
                expected_phase
            );
            assert_eq!(
                data_value["cleanupReceipt"]["ownedTargets"],
                preview_value["ownedTargets"]
            );
            assert_eq!(
                data_value["absentObservationDigests"],
                data_value["cleanupReceipt"]["absentObservationDigests"]
            );
            assert_eq!(
                data_value["absentObservationDigests"]
                    .as_array()
                    .unwrap()
                    .len(),
                7
            );
            assert!(schema_accepts::<CleanupPreviewData>(&preview_value));
            assert!(schema_accepts::<CleanupData>(&data_value));
        }
    }

    #[test]
    fn cleanup_paired_empty_is_zero_effect_but_keeps_durable_attempt_identity() {
        let archive_publication = completed_archive(id(ID_3), TaskArchiveOutcome::Abandoned);
        let archive = archive_publication.status().clone();
        let (preview, data) = direct_empty_cleanup(&archive);
        let preview = serde_json::to_value(preview).unwrap();
        let data = serde_json::to_value(data).unwrap();
        assert_eq!(preview["ownedTargets"], json!([]));
        assert_eq!(preview["removableRoles"], json!([]));
        assert_eq!(data["cleanupReceipt"]["ownedTargets"], json!([]));
        assert_eq!(data["absentObservationDigests"], json!([]));
        assert_eq!(
            data["cleanupReceipt"]["absentObservationDigests"],
            json!([])
        );
        assert_eq!(data["quarantineId"], ID_2);
        assert_eq!(data["cleanupReceipt"]["quarantineId"], ID_2);
        assert!(data.get("quarantinePath").is_none());
    }

    #[test]
    fn cleanup_rejects_noncanonical_targets_and_cross_archive_identity_splices() {
        let archive_publication = completed_archive(id(ID_3), TaskArchiveOutcome::Success);
        let archive = archive_publication.status().clone();
        let a = OwnedTargetLocator::new(project(ID_1), id(ID_2), OwnedTargetRole::Artifact);
        let b = OwnedTargetLocator::new(project(ID_1), id(ID_2), OwnedTargetRole::Quarantine);
        assert!(CleanupPreviewAuthority::test_only(&archive, vec![b.clone(), a.clone()]).is_err());
        assert!(CleanupPreviewAuthority::test_only(&archive, vec![a.clone(), a.clone()]).is_err());
        let foreign = OwnedTargetLocator::new(project(ID_4), id(ID_2), OwnedTargetRole::Quarantine);
        assert!(CleanupPreviewAuthority::test_only(&archive, vec![a, foreign]).is_err());

        let preview = cleanup_preview(&archive, Vec::new());
        let (marker_digest, preview_digest) = cleanup_preview_lineage(&preview);
        let authority = ApprovedCleanupAttempt::direct_empty_test_only(
            operation(ID_1),
            &archive,
            preview_digest,
            marker_digest,
            id(ID_2),
            Vec::new(),
        )
        .unwrap()
        .complete_direct_empty()
        .unwrap()
        .authorize_receipt(id(ID_3))
        .unwrap();
        let foreign_archive = completed_archive(id(ID_4), TaskArchiveOutcome::Success);
        assert!(CleanupData::from_receipt_authority(authority, foreign_archive.status()).is_err());

        let valid_preview = cleanup_preview(&archive, Vec::new());
        let (valid_marker, valid_preview_digest) = cleanup_preview_lineage(&valid_preview);
        let invalid_marker_authority = ApprovedCleanupAttempt::direct_empty_test_only(
            operation(ID_1),
            &archive,
            valid_preview_digest,
            digest('0'),
            id(ID_2),
            Vec::new(),
        )
        .unwrap()
        .complete_direct_empty()
        .unwrap()
        .authorize_receipt(id(ID_3))
        .unwrap();
        assert!(CleanupData::from_receipt_authority(invalid_marker_authority, &archive).is_err());

        let invalid_preview_authority = ApprovedCleanupAttempt::direct_empty_test_only(
            operation(ID_1),
            &archive,
            digest('0'),
            valid_marker,
            id(ID_2),
            Vec::new(),
        )
        .unwrap()
        .complete_direct_empty()
        .unwrap()
        .authorize_receipt(id(ID_3))
        .unwrap();
        assert!(CleanupData::from_receipt_authority(invalid_preview_authority, &archive).is_err());
    }

    #[test]
    fn applied_schemas_reject_cross_contract_and_cross_outcome_field_splices() {
        let success_publication = completed_archive(id(ID_3), TaskArchiveOutcome::Success);
        let abandoned_publication = completed_archive(id(ID_4), TaskArchiveOutcome::Abandoned);
        let (_, success_cleanup) = direct_empty_cleanup(success_publication.status());
        let (_, abandoned_cleanup) = direct_empty_cleanup(abandoned_publication.status());
        let mut success = serde_json::to_value(success_cleanup).unwrap();
        let abandoned = serde_json::to_value(abandoned_cleanup).unwrap();

        success["cleanupReceipt"] = abandoned["cleanupReceipt"].clone();
        assert!(!schema_accepts::<CleanupData>(&success));

        let mut wrong_outcome =
            serde_json::to_value(direct_empty_cleanup(success_publication.status()).1).unwrap();
        wrong_outcome["outcome"] = json!("abandoned");
        assert!(!schema_accepts::<CleanupData>(&wrong_outcome));

        let mut archive = serde_json::to_value(success_publication.data()).unwrap();
        archive["cleanupReceipt"] = abandoned["cleanupReceipt"].clone();
        assert!(!schema_accepts::<ArchiveData>(&archive));
        let mut cleanup = abandoned;
        cleanup["archiveStagingReceipt"] = serde_json::to_value(success_publication.data())
            .unwrap()["archiveStagingReceipt"]
            .clone();
        assert!(!schema_accepts::<CleanupData>(&cleanup));
    }

    #[test]
    fn preview_schemas_reject_all_post_effect_splices() {
        let archive_publication = completed_archive(id(ID_3), TaskArchiveOutcome::Success);
        let archive = archive_publication.status();
        for mut preview in [
            serde_json::to_value(success_archive_preview(&["evidence/task.json"])).unwrap(),
            serde_json::to_value(cleanup_preview(archive, Vec::new())).unwrap(),
        ] {
            for forbidden in [
                "operationId",
                "quarantineId",
                "sha256",
                "createdAt",
                "receiptDigest",
                "fingerprint",
            ] {
                let mut poisoned = preview.clone();
                poisoned
                    .as_object_mut()
                    .unwrap()
                    .insert(forbidden.to_owned(), json!(ID_1));
                assert!(
                    !schema_accepts::<ArchivePreviewData>(&poisoned)
                        && !schema_accepts::<CleanupPreviewData>(&poisoned),
                    "a preview schema accepted post-effect field {forbidden}"
                );
            }
            preview
                .as_object_mut()
                .unwrap()
                .insert("rawConnectionString".to_owned(), json!("Server=secret"));
            assert!(!schema_accepts::<ArchivePreviewData>(&preview));
            assert!(!schema_accepts::<CleanupPreviewData>(&preview));
        }
    }

    #[test]
    fn concrete_result_fixtures_reject_forbidden_fields_at_every_nested_object() {
        let not_created = NotCreatedData::from_authority(NotCreatedAuthority::from_coordinator(
            NotCreatedBlockers::new(blocker_set()).unwrap(),
        ));
        let archive_publication = completed_archive(id(ID_3), TaskArchiveOutcome::Success);
        let archive = archive_publication.status().clone();
        let (cleanup_preview, cleanup_data) = direct_empty_cleanup(&archive);
        assert_recursively_rejects_forbidden_fields::<StartData>(
            serde_json::to_value(reserved_start()).unwrap(),
        );
        assert_recursively_rejects_forbidden_fields::<StartData>(
            serde_json::to_value(separate_start()).unwrap(),
        );
        assert_recursively_rejects_forbidden_fields::<NotCreatedData>(
            serde_json::to_value(not_created).unwrap(),
        );
        assert_recursively_rejects_forbidden_fields::<ArchivePreviewData>(
            serde_json::to_value(success_archive_preview(&["evidence/task.json"])).unwrap(),
        );
        assert_recursively_rejects_forbidden_fields::<ArchiveData>(
            serde_json::to_value(archive_publication.data()).unwrap(),
        );
        assert_recursively_rejects_forbidden_fields::<CleanupPreviewData>(
            serde_json::to_value(cleanup_preview).unwrap(),
        );
        assert_recursively_rejects_forbidden_fields::<CleanupData>(
            serde_json::to_value(cleanup_data).unwrap(),
        );
    }

    #[test]
    fn every_result_and_digest_schema_declares_no_forbidden_wire_property() {
        assert_schema_declares_no_forbidden_properties::<StartData>();
        assert_schema_declares_no_forbidden_properties::<NotCreatedData>();
        assert_schema_declares_no_forbidden_properties::<BranchedStatusData>();
        assert_schema_declares_no_forbidden_properties::<ArchiveEligibilityDigestRecord>();
        assert_schema_declares_no_forbidden_properties::<ArchivePreviewDigestRecord>();
        assert_schema_declares_no_forbidden_properties::<ArchivePreviewData>();
        assert_schema_declares_no_forbidden_properties::<PreArmCancellationArchiveEntryDigestRecord>(
        );
        assert_schema_declares_no_forbidden_properties::<PreArmCancellationArchiveEntry>();
        assert_schema_declares_no_forbidden_properties::<StagedArchiveEntryManifestDigestRecord>();
        assert_schema_declares_no_forbidden_properties::<StagedArchiveEntryManifest>();
        assert_schema_declares_no_forbidden_properties::<HandoffLineageDigestRecord>();
        assert_schema_declares_no_forbidden_properties::<FrozenProviderBoundaryDigestRecord>();
        assert_schema_declares_no_forbidden_properties::<RetainedArchiveLineageDigestRecord>();
        assert_schema_declares_no_forbidden_properties::<ArchivePublicationManifestDigestRecord>();
        assert_schema_declares_no_forbidden_properties::<ArchivePublicationManifest>();
        assert_schema_declares_no_forbidden_properties::<ArchiveParsedEntrySetDigestRecord>();
        assert_schema_declares_no_forbidden_properties::<ArchivePublicationObservationDigestRecord>(
        );
        assert_schema_declares_no_forbidden_properties::<ArchivePublicationObservation>();
        assert_schema_declares_no_forbidden_properties::<ArchiveData>();
        assert_schema_declares_no_forbidden_properties::<CleanupMarkerDigestRecord>();
        assert_schema_declares_no_forbidden_properties::<CleanupPreviewDigestRecord>();
        assert_schema_declares_no_forbidden_properties::<CleanupPreviewData>();
        assert_schema_declares_no_forbidden_properties::<CleanupData>();
    }
}
