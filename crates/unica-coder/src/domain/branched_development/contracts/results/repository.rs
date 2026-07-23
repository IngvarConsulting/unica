use crate::domain::branched_development::canonical_json::{
    canonical_contract_digest, canonical_contract_encoding, contract_digest_record_sealed,
    operation_input_digest, ContractDigestRecord,
};
use crate::domain::branched_development::contracts::artifacts::{
    CompatibilityMode, ConfigurationIdentity, OriginalInfobaseKind, RepositoryTransport,
};
use crate::domain::branched_development::contracts::prearm_recovery::{
    PreArmCancellationEffectObservation, PreArmCancellationFinalizationAttemptProgress,
    PreArmCancellationFinalizationPlan, PreArmCancellationFinalizationRecheckEvidence,
};
use crate::domain::branched_development::contracts::recovery::{
    RecoveryAction, RecoveryActionOutcome, RecoveryEffectClass, RecoveryObservation,
    RecoveryPlanStatus, RecoveryTarget, RecoveryUnknown, ValidatedCompletedPreArmTerminalEvidence,
};
use crate::domain::branched_development::contracts::repository::{
    DeferredRepositoryAdvance, DeferredRepositoryAdvanceConsumptionReceipt, ObjectTargetIdentity,
    PostMergeHistoryGuardAuthority, PostMergeHistoryGuardEvidence, RepositoryActorIdentity,
    RepositoryAnchor, RepositoryHistoryCursor, RepositoryHistoryPartitionClassification,
    RepositoryHistoryPartitionResolver, RepositoryPlannedChanges, RepositoryTargetIdentity,
    RepositoryTargetState, RepositoryTargetStateRef, RepositoryTargetStates,
    RepositoryUpdateLockReason, RepositoryUpdateLockTargetRef, RepositoryUpdateLockTargets,
    RootTargetIdentity, SelectiveRepositoryUpdatePlan, SelectiveRepositoryUpdateProof,
    SelectiveRepositoryUpdateScope, SupportGateHistoryEvidence,
    SupportRootSelectiveRepositoryUpdatePlanAuthority, UnvalidatedRepositoryHistoryPartition,
    ValidatedRepositoryHistoryPartition, ValidatedRoutineUpdateProjection,
    ValidatedSupportPrerequisiteHistoryProjection, ValidatedSupportRecoveryHistoryEntryRef,
    ValidatedTaskCommitHistoryPartition,
};
use crate::domain::branched_development::contracts::requests::repository::{
    RepositoryCommitRequest, SupportCancellationReason, ValidatedCancellationApplyRequest,
    ValidatedCancellationArming, ValidatedCancellationPreviewRequest,
    ValidatedPrerequisiteUpdateApplyRequest, ValidatedPrerequisiteUpdatePreviewRequest,
    ValidatedRepositoryCommitApplyRequest, ValidatedRepositoryCommitPreviewRequest,
    ValidatedRoutineUpdateApplyRequest, ValidatedRoutineUpdatePreviewRequest,
};
#[cfg(test)]
use crate::domain::branched_development::contracts::results::merge::ValidatedMainIntegrationVerificationAuthority;
use crate::domain::branched_development::contracts::results::merge::{
    ResolvedCommitLineageConsumedSupportGateAuthority, ValidatedMainSandboxVerificationAuthority,
};
use crate::domain::branched_development::contracts::scalars::{
    Comment, NormalizedUtcInstant, OriginalProjectCwd, PositiveGeneration,
    RepositoryIdentityComponent, RepositoryTargetDisplay, RepositoryUsername, RepositoryVersion,
    RequiredNullable, TaskSummary,
};
use crate::domain::branched_development::contracts::schema::one_of_schema;
use crate::domain::branched_development::contracts::selectors::{
    RepositoryCommitSelectorVariant, TaskOperationSelector,
};
use crate::domain::branched_development::contracts::status::{
    ActiveOperationStatus, CleanupReceipt,
};
use crate::domain::branched_development::contracts::storage::OperationScope;
use crate::domain::branched_development::contracts::support::{
    ActiveSupportActionResumeHandle, CurrentReadySupportGateAuthority,
    ManualActorLockInventoryProof, ManualSupportTargetMode, ManualWorkingInfobaseIdentity,
    ReservedOriginalTerminalizationProof, SupportActionArmingReceipt, SupportActionPurpose,
    SupportActionTerminalOutcome, SupportActionTerminalStatusCasBinding,
    SupportActionTerminalStatusCasResolver, SupportAuthorizationOutcome,
    SupportPrerequisiteVersionObservation, SupportRootLockObservation, SupportRootLockProof,
    SupportTransitions, SupportUpdateAuthorizationProjection, TerminalSupportActionAuthorization,
    ValidatedSupportActionTerminalStatusCasAuthority,
};
use crate::domain::branched_development::contracts::support_terminalization::{
    ManualWorkingInfobaseClosurePlan, ManualWorkingInfobaseClosureProof, SupportRecoveryGuardProof,
};
use crate::domain::branched_development::{
    BranchedLifecycleToolName, CapabilityRowId, DurableExecutionPolicy, MetadataObjectId,
    OperationId, ProjectId, Sha256Digest, TaskId, TaskPhase, UnicaId,
};
use schemars::{json_schema, JsonSchema, Schema, SchemaGenerator};
use serde::Serialize;
use serde_json::{Map as JsonMap, Value as JsonValue};
use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::Arc;

const MAX_RESULT_ITEMS: usize = 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RepositoryResultContractError(&'static str);

impl fmt::Display for RepositoryResultContractError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.0)
    }
}

impl std::error::Error for RepositoryResultContractError {}

fn result_digest<T: ContractDigestRecord>(
    record: &T,
    failure: &'static str,
) -> Result<Sha256Digest, RepositoryResultContractError> {
    canonical_contract_digest(record, None).map_err(|_| RepositoryResultContractError(failure))
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

wire_literal!(ModifyAction, "modify");
wire_literal!(AddAction, "add");
wire_literal!(DeleteAction, "delete");
wire_literal!(ConfigurationRootKind, "configurationRoot");
wire_literal!(DevelopmentObjectKind, "developmentObject");
bool_literal!(TrueLiteral, true);

/// Builds one physical closed object branch for a serde-flattened result.
/// Schemars otherwise emits `unevaluatedProperties`, which is intentionally
/// outside Unica's auditable schema subset. Every supplied component is a
/// normal closed object; this helper merges its properties/required set into
/// one branch and closes it with `additionalProperties: false`.
fn closed_flattened_object_schema(
    parts: Vec<Schema>,
    extra_required_properties: Vec<(&'static str, Schema)>,
) -> Schema {
    let mut properties = JsonMap::new();
    let mut required = BTreeSet::new();
    for part in parts {
        let value = part.to_value();
        let object = value
            .as_object()
            .expect("flattened schema component must be an object schema");
        if let Some(component_properties) = object.get("properties").and_then(JsonValue::as_object)
        {
            for (name, property) in component_properties {
                assert!(
                    properties.insert(name.clone(), property.clone()).is_none(),
                    "flattened schema components must not repeat a property"
                );
            }
        }
        for name in object
            .get("required")
            .and_then(JsonValue::as_array)
            .into_iter()
            .flatten()
        {
            required.insert(
                name.as_str()
                    .expect("required property names must be strings")
                    .to_owned(),
            );
        }
    }
    for (name, schema) in extra_required_properties {
        assert!(
            properties
                .insert(name.to_owned(), schema.to_value())
                .is_none(),
            "flattened schema extras must not repeat a property"
        );
        required.insert(name.to_owned());
    }
    let mut object = JsonMap::new();
    object.insert("type".to_owned(), JsonValue::String("object".to_owned()));
    object.insert("properties".to_owned(), JsonValue::Object(properties));
    object.insert(
        "required".to_owned(),
        JsonValue::Array(required.into_iter().map(JsonValue::String).collect()),
    );
    object.insert("additionalProperties".to_owned(), JsonValue::Bool(false));
    Schema::from(object)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) enum RepositoryIntegrationReason {
    CanonicalDelta,
    OwnershipClosure,
    ReferenceClosure,
    AddDeleteSemantics,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct RepositoryIntegrationReasons(Vec<RepositoryIntegrationReason>);

impl RepositoryIntegrationReasons {
    pub(crate) fn new(
        values: Vec<RepositoryIntegrationReason>,
    ) -> Result<Self, RepositoryResultContractError> {
        if values.is_empty() || values.len() > 4 || values.windows(2).any(|pair| pair[0] >= pair[1])
        {
            return Err(RepositoryResultContractError(
                "integration reasons must be non-empty, unique, and in declaration order",
            ));
        }
        Ok(Self(values))
    }

    pub(crate) fn as_slice(&self) -> &[RepositoryIntegrationReason] {
        &self.0
    }
}

impl JsonSchema for RepositoryIntegrationReasons {
    fn schema_name() -> Cow<'static, str> {
        "RepositoryIntegrationReasons".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "array",
            "minItems": 1,
            "maxItems": 4,
            "uniqueItems": true,
            "items": generator.subschema_for::<RepositoryIntegrationReason>(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct CanonicalRepositoryTargets(Vec<RepositoryTargetIdentity>);

impl CanonicalRepositoryTargets {
    pub(crate) fn new(
        values: Vec<RepositoryTargetIdentity>,
    ) -> Result<Self, RepositoryResultContractError> {
        if values.len() > MAX_RESULT_ITEMS || values.windows(2).any(|pair| pair[0] >= pair[1]) {
            return Err(RepositoryResultContractError(
                "repository targets must be canonical and duplicate-free",
            ));
        }
        Ok(Self(values))
    }

    pub(crate) fn as_slice(&self) -> &[RepositoryTargetIdentity] {
        &self.0
    }
}

impl JsonSchema for CanonicalRepositoryTargets {
    fn schema_name() -> Cow<'static, str> {
        "CanonicalRepositoryTargets".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "array",
            "maxItems": MAX_RESULT_ITEMS,
            "uniqueItems": true,
            "items": generator.subschema_for::<RepositoryTargetIdentity>(),
        })
    }
}

fn project_lock_targets(
    values: &RepositoryUpdateLockTargets,
) -> Result<CanonicalRepositoryTargets, RepositoryResultContractError> {
    CanonicalRepositoryTargets::new(
        values
            .as_slice()
            .iter()
            .map(|value| match value.as_ref() {
                RepositoryUpdateLockTargetRef::ConfigurationRoot { .. } => {
                    RepositoryTargetIdentity::configuration_root()
                }
                RepositoryUpdateLockTargetRef::DevelopmentObject { object_id, .. } => {
                    RepositoryTargetIdentity::development_object(object_id.clone())
                }
            })
            .collect(),
    )
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RootModifyIntegrationEntry {
    target: RootTargetIdentity,
    object_display: RepositoryTargetDisplay,
    action: ModifyAction,
    reasons: RepositoryIntegrationReasons,
    required_lock_targets: CanonicalRepositoryTargets,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ObjectAddIntegrationEntry {
    target: ObjectTargetIdentity,
    object_display: RepositoryTargetDisplay,
    action: AddAction,
    reasons: RepositoryIntegrationReasons,
    required_lock_targets: CanonicalRepositoryTargets,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ObjectModifyIntegrationEntry {
    target: ObjectTargetIdentity,
    object_display: RepositoryTargetDisplay,
    action: ModifyAction,
    reasons: RepositoryIntegrationReasons,
    required_lock_targets: CanonicalRepositoryTargets,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ObjectDeleteIntegrationEntry {
    target: ObjectTargetIdentity,
    object_display: RepositoryTargetDisplay,
    action: DeleteAction,
    reasons: RepositoryIntegrationReasons,
    required_lock_targets: CanonicalRepositoryTargets,
}

/// Exact integration-set leaf. Presentation and lock closure are deliberately
/// retained here, while commit identity/action projections below cannot encode
/// either of them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub(crate) enum RepositoryIntegrationEntry {
    RootModify(RootModifyIntegrationEntry),
    ObjectAdd(ObjectAddIntegrationEntry),
    ObjectModify(ObjectModifyIntegrationEntry),
    ObjectDelete(ObjectDeleteIntegrationEntry),
}

impl RepositoryIntegrationEntry {
    #[cfg(test)]
    pub(crate) fn root_modify(
        target: RootTargetIdentity,
        object_display: RepositoryTargetDisplay,
        reasons: RepositoryIntegrationReasons,
        required_lock_targets: CanonicalRepositoryTargets,
    ) -> Self {
        Self::RootModify(RootModifyIntegrationEntry {
            target,
            object_display,
            action: ModifyAction::Value,
            reasons,
            required_lock_targets,
        })
    }

    #[cfg(test)]
    pub(crate) fn object_add(
        target: ObjectTargetIdentity,
        object_display: RepositoryTargetDisplay,
        reasons: RepositoryIntegrationReasons,
        required_lock_targets: CanonicalRepositoryTargets,
    ) -> Self {
        Self::ObjectAdd(ObjectAddIntegrationEntry {
            target,
            object_display,
            action: AddAction::Value,
            reasons,
            required_lock_targets,
        })
    }

    #[cfg(test)]
    pub(crate) fn object_modify(
        target: ObjectTargetIdentity,
        object_display: RepositoryTargetDisplay,
        reasons: RepositoryIntegrationReasons,
        required_lock_targets: CanonicalRepositoryTargets,
    ) -> Self {
        Self::ObjectModify(ObjectModifyIntegrationEntry {
            target,
            object_display,
            action: ModifyAction::Value,
            reasons,
            required_lock_targets,
        })
    }

    #[cfg(test)]
    pub(crate) fn object_delete(
        target: ObjectTargetIdentity,
        object_display: RepositoryTargetDisplay,
        reasons: RepositoryIntegrationReasons,
        required_lock_targets: CanonicalRepositoryTargets,
    ) -> Self {
        Self::ObjectDelete(ObjectDeleteIntegrationEntry {
            target,
            object_display,
            action: DeleteAction::Value,
            reasons,
            required_lock_targets,
        })
    }

    pub(crate) fn target_identity(&self) -> RepositoryTargetIdentity {
        match self {
            Self::RootModify(value) => {
                RepositoryTargetIdentity::ConfigurationRoot(value.target.clone())
            }
            Self::ObjectAdd(value) => {
                RepositoryTargetIdentity::DevelopmentObject(value.target.clone())
            }
            Self::ObjectModify(value) => {
                RepositoryTargetIdentity::DevelopmentObject(value.target.clone())
            }
            Self::ObjectDelete(value) => {
                RepositoryTargetIdentity::DevelopmentObject(value.target.clone())
            }
        }
    }

    pub(crate) fn required_lock_targets(&self) -> &CanonicalRepositoryTargets {
        match self {
            Self::RootModify(value) => &value.required_lock_targets,
            Self::ObjectAdd(value) => &value.required_lock_targets,
            Self::ObjectModify(value) => &value.required_lock_targets,
            Self::ObjectDelete(value) => &value.required_lock_targets,
        }
    }

    fn exact_projection(&self) -> CommitExactObject {
        match self {
            Self::RootModify(value) => CommitExactObject::RootModify(RootModifyExactObject {
                target: value.target.clone(),
                action: ModifyAction::Value,
            }),
            Self::ObjectAdd(value) => CommitExactObject::ObjectAdd(ObjectAddExactObject {
                target: value.target.clone(),
                action: AddAction::Value,
            }),
            Self::ObjectModify(value) => CommitExactObject::ObjectModify(ObjectModifyExactObject {
                target: value.target.clone(),
                action: ModifyAction::Value,
            }),
            Self::ObjectDelete(value) => CommitExactObject::ObjectDelete(ObjectDeleteExactObject {
                target: value.target.clone(),
                action: DeleteAction::Value,
            }),
        }
    }
}

impl JsonSchema for RepositoryIntegrationEntry {
    fn schema_name() -> Cow<'static, str> {
        "RepositoryIntegrationEntry".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<RootModifyIntegrationEntry>(),
            generator.subschema_for::<ObjectAddIntegrationEntry>(),
            generator.subschema_for::<ObjectModifyIntegrationEntry>(),
            generator.subschema_for::<ObjectDeleteIntegrationEntry>(),
        ])
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct RepositoryIntegrationEntries(Vec<RepositoryIntegrationEntry>);

impl RepositoryIntegrationEntries {
    pub(crate) fn new(
        values: Vec<RepositoryIntegrationEntry>,
    ) -> Result<Self, RepositoryResultContractError> {
        if values.is_empty() || values.len() > MAX_RESULT_ITEMS {
            return Err(RepositoryResultContractError(
                "integration entries must be non-empty and bounded",
            ));
        }
        if values
            .windows(2)
            .any(|pair| pair[0].target_identity() >= pair[1].target_identity())
        {
            return Err(RepositoryResultContractError(
                "integration entries must be canonical and unique by target",
            ));
        }
        Ok(Self(values))
    }

    pub(crate) fn as_slice(&self) -> &[RepositoryIntegrationEntry] {
        &self.0
    }

    fn exact_objects(&self) -> CommitExactObjects {
        CommitExactObjects(
            self.0
                .iter()
                .map(RepositoryIntegrationEntry::exact_projection)
                .collect(),
        )
    }
}

impl JsonSchema for RepositoryIntegrationEntries {
    fn schema_name() -> Cow<'static, str> {
        "RepositoryIntegrationEntries".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "array",
            "minItems": 1,
            "maxItems": MAX_RESULT_ITEMS,
            "uniqueItems": true,
            "items": generator.subschema_for::<RepositoryIntegrationEntry>(),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DeleteSelfLockabilityObservation {
    Absent,
    ExistingNotSeparatelyLockable,
    ExistingSeparatelyLockable,
}

/// Complete semantic topology observed by one repository lock-planner
/// invocation. These values are not wire data: the core projects the public
/// integration entry, its exact required-lock closure, and its reason list.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum RepositoryIntegrationTopologyObservation {
    RootModify {
        object_display: RepositoryTargetDisplay,
    },
    TopLevelAdd {
        target: ObjectTargetIdentity,
        object_display: RepositoryTargetDisplay,
    },
    SubordinateAdd {
        target: ObjectTargetIdentity,
        object_display: RepositoryTargetDisplay,
        parent: ObjectTargetIdentity,
    },
    ObjectModify {
        target: ObjectTargetIdentity,
        object_display: RepositoryTargetDisplay,
        changed_referrers: Vec<RepositoryTargetIdentity>,
    },
    OwnedChildModify {
        target: ObjectTargetIdentity,
        object_display: RepositoryTargetDisplay,
        changed_referrers: Vec<RepositoryTargetIdentity>,
    },
    ObjectDelete {
        target: ObjectTargetIdentity,
        object_display: RepositoryTargetDisplay,
        parent: RepositoryTargetIdentity,
        existing_subordinate_development_objects: Vec<RepositoryTargetIdentity>,
        changed_referrers: Vec<RepositoryTargetIdentity>,
        self_lockability: DeleteSelfLockabilityObservation,
    },
}

impl RepositoryIntegrationTopologyObservation {
    pub(crate) fn root_modify(object_display: RepositoryTargetDisplay) -> Self {
        Self::RootModify { object_display }
    }

    pub(crate) fn top_level_add(
        target: ObjectTargetIdentity,
        object_display: RepositoryTargetDisplay,
    ) -> Self {
        Self::TopLevelAdd {
            target,
            object_display,
        }
    }

    pub(crate) fn subordinate_add(
        target: ObjectTargetIdentity,
        object_display: RepositoryTargetDisplay,
        parent: ObjectTargetIdentity,
    ) -> Self {
        Self::SubordinateAdd {
            target,
            object_display,
            parent,
        }
    }

    pub(crate) fn object_modify(
        target: ObjectTargetIdentity,
        object_display: RepositoryTargetDisplay,
        changed_referrers: Vec<RepositoryTargetIdentity>,
    ) -> Self {
        Self::ObjectModify {
            target,
            object_display,
            changed_referrers,
        }
    }

    pub(crate) fn owned_child_modify(
        target: ObjectTargetIdentity,
        object_display: RepositoryTargetDisplay,
        changed_referrers: Vec<RepositoryTargetIdentity>,
    ) -> Self {
        Self::OwnedChildModify {
            target,
            object_display,
            changed_referrers,
        }
    }

    pub(crate) fn object_delete(
        target: ObjectTargetIdentity,
        object_display: RepositoryTargetDisplay,
        parent: RepositoryTargetIdentity,
        existing_subordinate_development_objects: Vec<RepositoryTargetIdentity>,
        changed_referrers: Vec<RepositoryTargetIdentity>,
        self_lockability: DeleteSelfLockabilityObservation,
    ) -> Self {
        Self::ObjectDelete {
            target,
            object_display,
            parent,
            existing_subordinate_development_objects,
            changed_referrers,
            self_lockability,
        }
    }

    fn target_identity(&self) -> RepositoryTargetIdentity {
        match self {
            Self::RootModify { .. } => RepositoryTargetIdentity::configuration_root(),
            Self::TopLevelAdd { target, .. }
            | Self::SubordinateAdd { target, .. }
            | Self::ObjectModify { target, .. }
            | Self::OwnedChildModify { target, .. }
            | Self::ObjectDelete { target, .. } => {
                RepositoryTargetIdentity::DevelopmentObject(target.clone())
            }
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
struct RepositoryIntegrationTopologyBatchAuthority {
    integration_entries: RepositoryIntegrationEntries,
    expected_lock_reasons: BTreeMap<RepositoryTargetIdentity, Vec<RepositoryUpdateLockReason>>,
}

impl RepositoryIntegrationTopologyBatchAuthority {
    fn derive(
        observations: Vec<RepositoryIntegrationTopologyObservation>,
    ) -> Result<Self, RepositoryResultContractError> {
        if observations.is_empty() || observations.len() > MAX_RESULT_ITEMS {
            return Err(RepositoryResultContractError(
                "integration topology observations must be non-empty and bounded",
            ));
        }
        if observations
            .windows(2)
            .any(|pair| pair[0].target_identity() >= pair[1].target_identity())
        {
            return Err(RepositoryResultContractError(
                "integration topology observations must be canonical and unique by target",
            ));
        }

        let mut expected_lock_reasons =
            BTreeMap::<RepositoryTargetIdentity, BTreeSet<RepositoryUpdateLockReason>>::new();
        expected_lock_reasons
            .entry(RepositoryTargetIdentity::configuration_root())
            .or_default()
            .insert(RepositoryUpdateLockReason::SupportGraphGuard);
        let mut entries = Vec::with_capacity(observations.len());

        for observation in observations {
            let owns_non_development_child = matches!(
                &observation,
                RepositoryIntegrationTopologyObservation::OwnedChildModify { .. }
            );
            let mut required_targets = BTreeSet::new();
            let mut integration_reasons = BTreeSet::new();
            integration_reasons.insert(RepositoryIntegrationReason::CanonicalDelta);

            let entry = match observation {
                RepositoryIntegrationTopologyObservation::RootModify { object_display } => {
                    let root = RepositoryTargetIdentity::configuration_root();
                    required_targets.insert(root.clone());
                    expected_lock_reasons
                        .entry(root)
                        .or_default()
                        .insert(RepositoryUpdateLockReason::UpdateTarget);
                    RepositoryIntegrationEntry::RootModify(RootModifyIntegrationEntry {
                        target: RootTargetIdentity::new(),
                        object_display,
                        action: ModifyAction::Value,
                        reasons: RepositoryIntegrationReasons::new(
                            integration_reasons.into_iter().collect(),
                        )?,
                        required_lock_targets: CanonicalRepositoryTargets::new(
                            required_targets.into_iter().collect(),
                        )?,
                    })
                }
                RepositoryIntegrationTopologyObservation::TopLevelAdd {
                    target,
                    object_display,
                } => {
                    let root = RepositoryTargetIdentity::configuration_root();
                    required_targets.insert(root.clone());
                    integration_reasons.insert(RepositoryIntegrationReason::OwnershipClosure);
                    integration_reasons.insert(RepositoryIntegrationReason::AddDeleteSemantics);
                    expected_lock_reasons
                        .entry(root)
                        .or_default()
                        .insert(RepositoryUpdateLockReason::ParentClosure);
                    RepositoryIntegrationEntry::ObjectAdd(ObjectAddIntegrationEntry {
                        target,
                        object_display,
                        action: AddAction::Value,
                        reasons: RepositoryIntegrationReasons::new(
                            integration_reasons.into_iter().collect(),
                        )?,
                        required_lock_targets: CanonicalRepositoryTargets::new(
                            required_targets.into_iter().collect(),
                        )?,
                    })
                }
                RepositoryIntegrationTopologyObservation::SubordinateAdd {
                    target,
                    object_display,
                    parent,
                } => {
                    if target == parent {
                        return Err(RepositoryResultContractError(
                            "subordinate addition cannot own itself",
                        ));
                    }
                    let parent = RepositoryTargetIdentity::DevelopmentObject(parent);
                    required_targets.insert(parent.clone());
                    integration_reasons.insert(RepositoryIntegrationReason::OwnershipClosure);
                    integration_reasons.insert(RepositoryIntegrationReason::AddDeleteSemantics);
                    expected_lock_reasons
                        .entry(parent)
                        .or_default()
                        .insert(RepositoryUpdateLockReason::ParentClosure);
                    RepositoryIntegrationEntry::ObjectAdd(ObjectAddIntegrationEntry {
                        target,
                        object_display,
                        action: AddAction::Value,
                        reasons: RepositoryIntegrationReasons::new(
                            integration_reasons.into_iter().collect(),
                        )?,
                        required_lock_targets: CanonicalRepositoryTargets::new(
                            required_targets.into_iter().collect(),
                        )?,
                    })
                }
                RepositoryIntegrationTopologyObservation::ObjectModify {
                    target,
                    object_display,
                    changed_referrers,
                }
                | RepositoryIntegrationTopologyObservation::OwnedChildModify {
                    target,
                    object_display,
                    changed_referrers,
                } => {
                    let target_identity =
                        RepositoryTargetIdentity::DevelopmentObject(target.clone());
                    required_targets.insert(target_identity.clone());
                    expected_lock_reasons
                        .entry(target_identity)
                        .or_default()
                        .insert(RepositoryUpdateLockReason::UpdateTarget);
                    if owns_non_development_child {
                        integration_reasons.insert(RepositoryIntegrationReason::OwnershipClosure);
                    }
                    add_changed_referrers(
                        &mut required_targets,
                        &mut expected_lock_reasons,
                        &mut integration_reasons,
                        changed_referrers,
                    )?;
                    RepositoryIntegrationEntry::ObjectModify(ObjectModifyIntegrationEntry {
                        target,
                        object_display,
                        action: ModifyAction::Value,
                        reasons: RepositoryIntegrationReasons::new(
                            integration_reasons.into_iter().collect(),
                        )?,
                        required_lock_targets: CanonicalRepositoryTargets::new(
                            required_targets.into_iter().collect(),
                        )?,
                    })
                }
                RepositoryIntegrationTopologyObservation::ObjectDelete {
                    target,
                    object_display,
                    parent,
                    existing_subordinate_development_objects,
                    changed_referrers,
                    self_lockability,
                } => {
                    let target_identity =
                        RepositoryTargetIdentity::DevelopmentObject(target.clone());
                    if parent == target_identity {
                        return Err(RepositoryResultContractError(
                            "deleted object cannot be its own parent",
                        ));
                    }
                    required_targets.insert(parent.clone());
                    expected_lock_reasons
                        .entry(parent.clone())
                        .or_default()
                        .insert(RepositoryUpdateLockReason::ParentClosure);
                    integration_reasons.insert(RepositoryIntegrationReason::OwnershipClosure);
                    integration_reasons.insert(RepositoryIntegrationReason::AddDeleteSemantics);

                    let subordinates =
                        CanonicalRepositoryTargets::new(existing_subordinate_development_objects)?;
                    for subordinate in subordinates.as_slice() {
                        if subordinate == &target_identity
                            || subordinate == &parent
                            || !matches!(
                                subordinate,
                                RepositoryTargetIdentity::DevelopmentObject(_)
                            )
                        {
                            return Err(RepositoryResultContractError(
                                "delete subordinate closure contains a non-subordinate target",
                            ));
                        }
                        required_targets.insert(subordinate.clone());
                        expected_lock_reasons
                            .entry(subordinate.clone())
                            .or_default()
                            .insert(RepositoryUpdateLockReason::StructuralClosure);
                    }
                    if changed_referrers
                        .iter()
                        .any(|referrer| referrer == &target_identity)
                    {
                        return Err(RepositoryResultContractError(
                            "deleted object cannot be its own changed referrer",
                        ));
                    }
                    add_changed_referrers(
                        &mut required_targets,
                        &mut expected_lock_reasons,
                        &mut integration_reasons,
                        changed_referrers,
                    )?;
                    if matches!(
                        self_lockability,
                        DeleteSelfLockabilityObservation::ExistingSeparatelyLockable
                    ) {
                        required_targets.insert(target_identity.clone());
                        expected_lock_reasons
                            .entry(target_identity)
                            .or_default()
                            .insert(RepositoryUpdateLockReason::UpdateTarget);
                    }
                    RepositoryIntegrationEntry::ObjectDelete(ObjectDeleteIntegrationEntry {
                        target,
                        object_display,
                        action: DeleteAction::Value,
                        reasons: RepositoryIntegrationReasons::new(
                            integration_reasons.into_iter().collect(),
                        )?,
                        required_lock_targets: CanonicalRepositoryTargets::new(
                            required_targets.into_iter().collect(),
                        )?,
                    })
                }
            };
            entries.push(entry);
        }

        let integration_entries = RepositoryIntegrationEntries::new(entries)?;
        for entry in integration_entries.as_slice() {
            let target = entry.target_identity();
            match entry {
                RepositoryIntegrationEntry::ObjectAdd(_) => {
                    if expected_lock_reasons.contains_key(&target) {
                        return Err(RepositoryResultContractError(
                            "new integration target cannot appear in the physical lock closure",
                        ));
                    }
                }
                RepositoryIntegrationEntry::ObjectDelete(_) => {
                    let self_lock_is_required = entry
                        .required_lock_targets()
                        .as_slice()
                        .binary_search(&target)
                        .is_ok();
                    if expected_lock_reasons.contains_key(&target) != self_lock_is_required {
                        return Err(RepositoryResultContractError(
                            "delete target lock presence disagrees with its self-lockability observation",
                        ));
                    }
                }
                RepositoryIntegrationEntry::RootModify(_)
                | RepositoryIntegrationEntry::ObjectModify(_) => {}
            }
        }
        Ok(Self {
            integration_entries,
            expected_lock_reasons: expected_lock_reasons
                .into_iter()
                .map(|(target, reasons)| (target, reasons.into_iter().collect()))
                .collect(),
        })
    }

    fn validate_lock_entries(
        &self,
        lock_entries: &RepositoryUpdateLockTargets,
    ) -> Result<(), RepositoryResultContractError> {
        if self.expected_lock_reasons.len() != lock_entries.as_slice().len() {
            return Err(RepositoryResultContractError(
                "observed lock targets differ from the exact topology closure",
            ));
        }
        for ((expected_target, expected_reasons), observed) in self
            .expected_lock_reasons
            .iter()
            .zip(lock_entries.as_slice())
        {
            let (observed_target, observed_reasons) = match observed.as_ref() {
                RepositoryUpdateLockTargetRef::ConfigurationRoot { reasons, .. } => {
                    (RepositoryTargetIdentity::configuration_root(), reasons)
                }
                RepositoryUpdateLockTargetRef::DevelopmentObject {
                    object_id, reasons, ..
                } => (
                    RepositoryTargetIdentity::development_object(object_id.clone()),
                    reasons,
                ),
            };
            if expected_target != &observed_target
                || expected_reasons.as_slice() != observed_reasons
            {
                return Err(RepositoryResultContractError(
                    "observed lock target or reasons differ from the exact topology closure",
                ));
            }
        }
        Ok(())
    }

    fn integration_entries(&self) -> &RepositoryIntegrationEntries {
        &self.integration_entries
    }

    fn into_integration_entries(self) -> RepositoryIntegrationEntries {
        self.integration_entries
    }
}

fn add_changed_referrers(
    required_targets: &mut BTreeSet<RepositoryTargetIdentity>,
    expected_lock_reasons: &mut BTreeMap<
        RepositoryTargetIdentity,
        BTreeSet<RepositoryUpdateLockReason>,
    >,
    integration_reasons: &mut BTreeSet<RepositoryIntegrationReason>,
    changed_referrers: Vec<RepositoryTargetIdentity>,
) -> Result<(), RepositoryResultContractError> {
    let changed_referrers = CanonicalRepositoryTargets::new(changed_referrers)?;
    if !changed_referrers.as_slice().is_empty() {
        integration_reasons.insert(RepositoryIntegrationReason::ReferenceClosure);
    }
    for referrer in changed_referrers.as_slice() {
        if !matches!(referrer, RepositoryTargetIdentity::DevelopmentObject(_)) {
            return Err(RepositoryResultContractError(
                "changed referrer must be a development object",
            ));
        }
        required_targets.insert(referrer.clone());
        expected_lock_reasons
            .entry(referrer.clone())
            .or_default()
            .insert(RepositoryUpdateLockReason::ReferenceClosure);
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RootModifyExactObject {
    target: RootTargetIdentity,
    action: ModifyAction,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ObjectAddExactObject {
    target: ObjectTargetIdentity,
    action: AddAction,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ObjectModifyExactObject {
    target: ObjectTargetIdentity,
    action: ModifyAction,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ObjectDeleteExactObject {
    target: ObjectTargetIdentity,
    action: DeleteAction,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub(crate) enum CommitExactObject {
    RootModify(RootModifyExactObject),
    ObjectAdd(ObjectAddExactObject),
    ObjectModify(ObjectModifyExactObject),
    ObjectDelete(ObjectDeleteExactObject),
}

/// Borrowed command projection for a repository adapter.  It exposes the
/// approved target/action tuple without opening constructors or requiring a
/// JSON round trip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CommitExactObjectRef<'a> {
    RootModify,
    ObjectAdd { object_id: &'a MetadataObjectId },
    ObjectModify { object_id: &'a MetadataObjectId },
    ObjectDelete { object_id: &'a MetadataObjectId },
}

impl CommitExactObject {
    fn target_identity(&self) -> RepositoryTargetIdentity {
        match self {
            Self::RootModify(value) => {
                RepositoryTargetIdentity::ConfigurationRoot(value.target.clone())
            }
            Self::ObjectAdd(value) => {
                RepositoryTargetIdentity::DevelopmentObject(value.target.clone())
            }
            Self::ObjectModify(value) => {
                RepositoryTargetIdentity::DevelopmentObject(value.target.clone())
            }
            Self::ObjectDelete(value) => {
                RepositoryTargetIdentity::DevelopmentObject(value.target.clone())
            }
        }
    }

    pub(crate) const fn as_ref(&self) -> CommitExactObjectRef<'_> {
        match self {
            Self::RootModify(_) => CommitExactObjectRef::RootModify,
            Self::ObjectAdd(value) => CommitExactObjectRef::ObjectAdd {
                object_id: value.target.object_id(),
            },
            Self::ObjectModify(value) => CommitExactObjectRef::ObjectModify {
                object_id: value.target.object_id(),
            },
            Self::ObjectDelete(value) => CommitExactObjectRef::ObjectDelete {
                object_id: value.target.object_id(),
            },
        }
    }
}

impl JsonSchema for CommitExactObject {
    fn schema_name() -> Cow<'static, str> {
        "CommitExactObject".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<RootModifyExactObject>(),
            generator.subschema_for::<ObjectAddExactObject>(),
            generator.subschema_for::<ObjectModifyExactObject>(),
            generator.subschema_for::<ObjectDeleteExactObject>(),
        ])
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct CommitExactObjects(Vec<CommitExactObject>);

impl CommitExactObjects {
    fn new(values: Vec<CommitExactObject>) -> Result<Self, RepositoryResultContractError> {
        if values.is_empty()
            || values.len() > MAX_RESULT_ITEMS
            || values
                .windows(2)
                .any(|pair| pair[0].target_identity() >= pair[1].target_identity())
        {
            return Err(RepositoryResultContractError(
                "exact commit objects must be non-empty, canonical, and unique by target",
            ));
        }
        Ok(Self(values))
    }

    pub(crate) fn as_slice(&self) -> &[CommitExactObject] {
        &self.0
    }

    pub(crate) fn iter(&self) -> impl ExactSizeIterator<Item = CommitExactObjectRef<'_>> {
        self.0.iter().map(CommitExactObject::as_ref)
    }
}

impl JsonSchema for CommitExactObjects {
    fn schema_name() -> Cow<'static, str> {
        "CommitExactObjects".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "array",
            "minItems": 1,
            "maxItems": MAX_RESULT_ITEMS,
            "uniqueItems": true,
            "items": generator.subschema_for::<CommitExactObject>(),
        })
    }
}

fn target_states_cover_exact_commit_objects(
    exact_objects: &CommitExactObjects,
    target_states: &RepositoryTargetStates,
) -> bool {
    exact_objects.as_slice().len() == target_states.as_slice().len()
        && exact_objects
            .iter()
            .zip(
                target_states
                    .as_slice()
                    .iter()
                    .map(RepositoryTargetState::as_ref),
            )
            .all(|(exact, state)| match (exact, state) {
                (
                    CommitExactObjectRef::RootModify,
                    RepositoryTargetStateRef::RootPresent { .. },
                ) => true,
                (
                    CommitExactObjectRef::ObjectAdd {
                        object_id: expected,
                    },
                    RepositoryTargetStateRef::ObjectAbsent { object_id, .. },
                )
                | (
                    CommitExactObjectRef::ObjectModify {
                        object_id: expected,
                    },
                    RepositoryTargetStateRef::ObjectPresent { object_id, .. },
                )
                | (
                    CommitExactObjectRef::ObjectDelete {
                        object_id: expected,
                    },
                    RepositoryTargetStateRef::ObjectPresent { object_id, .. },
                ) => expected == object_id,
                _ => false,
            })
}

fn exact_zero_effect_target_transition(
    exact_objects: &CommitExactObjects,
    before: &RepositoryTargetStates,
    terminal: &RepositoryTargetStates,
) -> bool {
    target_states_cover_exact_commit_objects(exact_objects, before)
        && target_states_cover_exact_commit_objects(exact_objects, terminal)
        // RepositoryTargetState versions are the versions that established
        // the state, not observation cursors.  Exact equality intentionally
        // rejects ABA/change-then-revert as a zero-effect certificate.
        && before == terminal
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct CommitTargetStateSnapshotRecord {
    digest_kind: &'static str,
    repository_anchor: RepositoryAnchor,
    target_states: RepositoryTargetStates,
    observation_capability_id: CapabilityRowId,
    atomic_commit_safety_capability_id: CapabilityRowId,
}

impl contract_digest_record_sealed::Sealed for CommitTargetStateSnapshotRecord {}
impl ContractDigestRecord for CommitTargetStateSnapshotRecord {}

#[derive(Debug, PartialEq, Eq)]
struct CommitPreCommandTargetSnapshotAuthority {
    record: CommitTargetStateSnapshotRecord,
    snapshot_digest: Sha256Digest,
}

impl CommitPreCommandTargetSnapshotAuthority {
    fn from_immediate_observation(
        exact_objects: &CommitExactObjects,
        repository_anchor: RepositoryAnchor,
        target_states: RepositoryTargetStates,
        observation_capability_id: CapabilityRowId,
        atomic_commit_safety_capability_id: CapabilityRowId,
    ) -> Result<Self, RepositoryResultContractError> {
        if !target_states_cover_exact_commit_objects(exact_objects, &target_states) {
            return Err(RepositoryResultContractError(
                "pre-command target snapshot does not cover the exact approved actions",
            ));
        }
        let record = CommitTargetStateSnapshotRecord {
            digest_kind: "unica.repository.commit.target-state-snapshot.v1",
            repository_anchor,
            target_states,
            observation_capability_id,
            atomic_commit_safety_capability_id,
        };
        let snapshot_digest =
            result_digest(&record, "pre-command target-state snapshot digest failed")?;
        Ok(Self {
            record,
            snapshot_digest,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RootModifyCommittedObject {
    target_kind: ConfigurationRootKind,
    action: ModifyAction,
    repository_version: RepositoryVersion,
    target_fingerprint: Sha256Digest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) enum PresentObjectAction {
    Add,
    Modify,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ObjectPresentCommittedObject {
    target_kind: DevelopmentObjectKind,
    object_id: MetadataObjectId,
    action: PresentObjectAction,
    repository_version: RepositoryVersion,
    target_fingerprint: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ObjectAbsentCommittedObject {
    target_kind: DevelopmentObjectKind,
    object_id: MetadataObjectId,
    action: DeleteAction,
    absence_established_at_version: RepositoryVersion,
    expected_absent: TrueLiteral,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub(crate) enum CommittedRepositoryObject {
    RootModify(RootModifyCommittedObject),
    ObjectPresent(ObjectPresentCommittedObject),
    ObjectAbsent(ObjectAbsentCommittedObject),
}

impl CommittedRepositoryObject {
    pub(crate) fn root_modify(
        repository_version: RepositoryVersion,
        target_fingerprint: Sha256Digest,
    ) -> Self {
        Self::RootModify(RootModifyCommittedObject {
            target_kind: ConfigurationRootKind::Value,
            action: ModifyAction::Value,
            repository_version,
            target_fingerprint,
        })
    }

    pub(crate) fn object_present(
        object_id: MetadataObjectId,
        action: PresentObjectAction,
        repository_version: RepositoryVersion,
        target_fingerprint: Sha256Digest,
    ) -> Self {
        Self::ObjectPresent(ObjectPresentCommittedObject {
            target_kind: DevelopmentObjectKind::Value,
            object_id,
            action,
            repository_version,
            target_fingerprint,
        })
    }

    pub(crate) fn object_absent(
        object_id: MetadataObjectId,
        absence_established_at_version: RepositoryVersion,
    ) -> Self {
        Self::ObjectAbsent(ObjectAbsentCommittedObject {
            target_kind: DevelopmentObjectKind::Value,
            object_id,
            action: DeleteAction::Value,
            absence_established_at_version,
            expected_absent: TrueLiteral,
        })
    }

    fn target_identity(&self) -> RepositoryTargetIdentity {
        match self {
            Self::RootModify(_) => RepositoryTargetIdentity::configuration_root(),
            Self::ObjectPresent(value) => {
                RepositoryTargetIdentity::development_object(value.object_id.clone())
            }
            Self::ObjectAbsent(value) => {
                RepositoryTargetIdentity::development_object(value.object_id.clone())
            }
        }
    }

    fn exact_projection(&self) -> CommitExactObject {
        match self {
            Self::RootModify(_) => match self.target_identity() {
                RepositoryTargetIdentity::ConfigurationRoot(target) => {
                    CommitExactObject::RootModify(RootModifyExactObject {
                        target,
                        action: ModifyAction::Value,
                    })
                }
                RepositoryTargetIdentity::DevelopmentObject(_) => unreachable!(),
            },
            Self::ObjectPresent(value) => match self.target_identity() {
                RepositoryTargetIdentity::DevelopmentObject(target) => match value.action {
                    PresentObjectAction::Add => {
                        CommitExactObject::ObjectAdd(ObjectAddExactObject {
                            target,
                            action: AddAction::Value,
                        })
                    }
                    PresentObjectAction::Modify => {
                        CommitExactObject::ObjectModify(ObjectModifyExactObject {
                            target,
                            action: ModifyAction::Value,
                        })
                    }
                },
                RepositoryTargetIdentity::ConfigurationRoot(_) => unreachable!(),
            },
            Self::ObjectAbsent(_) => match self.target_identity() {
                RepositoryTargetIdentity::DevelopmentObject(target) => {
                    CommitExactObject::ObjectDelete(ObjectDeleteExactObject {
                        target,
                        action: DeleteAction::Value,
                    })
                }
                RepositoryTargetIdentity::ConfigurationRoot(_) => unreachable!(),
            },
        }
    }

    fn version_matches(&self, version: &RepositoryVersion) -> bool {
        match self {
            Self::RootModify(value) => &value.repository_version == version,
            Self::ObjectPresent(value) => &value.repository_version == version,
            Self::ObjectAbsent(value) => &value.absence_established_at_version == version,
        }
    }
}

impl JsonSchema for CommittedRepositoryObject {
    fn schema_name() -> Cow<'static, str> {
        "CommittedRepositoryObject".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<RootModifyCommittedObject>(),
            generator.subschema_for::<ObjectPresentCommittedObject>(),
            generator.subschema_for::<ObjectAbsentCommittedObject>(),
        ])
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct CommittedRepositoryObjects(Vec<CommittedRepositoryObject>);

impl CommittedRepositoryObjects {
    pub(crate) fn new(
        values: Vec<CommittedRepositoryObject>,
    ) -> Result<Self, RepositoryResultContractError> {
        if values.is_empty()
            || values.len() > MAX_RESULT_ITEMS
            || values
                .windows(2)
                .any(|pair| pair[0].target_identity() >= pair[1].target_identity())
        {
            return Err(RepositoryResultContractError(
                "committed objects must be non-empty, canonical, and unique by target",
            ));
        }
        Ok(Self(values))
    }

    pub(crate) fn as_slice(&self) -> &[CommittedRepositoryObject] {
        &self.0
    }

    fn exact_objects(&self) -> CommitExactObjects {
        CommitExactObjects(
            self.0
                .iter()
                .map(CommittedRepositoryObject::exact_projection)
                .collect(),
        )
    }

    fn all_versions_match(&self, repository_version: &RepositoryVersion) -> bool {
        self.0
            .iter()
            .all(|value| value.version_matches(repository_version))
    }
}

impl JsonSchema for CommittedRepositoryObjects {
    fn schema_name() -> Cow<'static, str> {
        "CommittedRepositoryObjects".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "array",
            "minItems": 1,
            "maxItems": MAX_RESULT_ITEMS,
            "uniqueItems": true,
            "items": generator.subschema_for::<CommittedRepositoryObject>(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CommittedObjectsDigestRecord {
    integration_set_digest: Sha256Digest,
    committed_objects: CommittedRepositoryObjects,
}

impl contract_digest_record_sealed::Sealed for CommittedObjectsDigestRecord {}
impl ContractDigestRecord for CommittedObjectsDigestRecord {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CommitCommentPolicyDigestRecord {
    template: Comment,
    task_id: TaskId,
    task_summary: TaskSummary,
    project_id: ProjectId,
    rendered_comment: Comment,
    non_empty: TrueLiteral,
    task_bound: TrueLiteral,
}

impl CommitCommentPolicyDigestRecord {
    pub(crate) fn new(
        template: Comment,
        task_id: TaskId,
        task_summary: TaskSummary,
        project_id: ProjectId,
        rendered_comment: Comment,
    ) -> Self {
        Self {
            template,
            task_id,
            task_summary,
            project_id,
            rendered_comment,
            non_empty: TrueLiteral,
            task_bound: TrueLiteral,
        }
    }

    pub(crate) fn digest(&self) -> Result<Sha256Digest, RepositoryResultContractError> {
        result_digest(self, "commit comment policy digest failed")
    }
}

impl contract_digest_record_sealed::Sealed for CommitCommentPolicyDigestRecord {}
impl ContractDigestRecord for CommitCommentPolicyDigestRecord {}

/// Immutable task-start rendering policy. The renderer capability seals the
/// exact template and task metadata before repository integration begins.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct FrozenCommitCommentPolicyAuthority {
    record: CommitCommentPolicyDigestRecord,
    policy_digest: Sha256Digest,
    renderer_capability_id: CapabilityRowId,
}

impl FrozenCommitCommentPolicyAuthority {
    pub(crate) fn from_task_start_renderer_adapter(
        template: Comment,
        task_id: TaskId,
        task_summary: TaskSummary,
        project_id: ProjectId,
        rendered_comment: Comment,
        renderer_capability_id: CapabilityRowId,
    ) -> Result<Self, RepositoryResultContractError> {
        if !template.as_str().contains("{taskId}")
            || !rendered_comment.as_str().contains(task_id.as_str())
        {
            return Err(RepositoryResultContractError(
                "commit comment policy is not bound to its task identifier",
            ));
        }
        let record = CommitCommentPolicyDigestRecord::new(
            template,
            task_id,
            task_summary,
            project_id,
            rendered_comment,
        );
        let policy_digest = record.digest()?;
        Ok(Self {
            record,
            policy_digest,
            renderer_capability_id,
        })
    }
}

/// Commit-time revalidation of the frozen comment policy. This token is
/// linear and cannot be reconstructed from a rendered string alone.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ValidatedCommitCommentPolicyAuthority {
    record: CommitCommentPolicyDigestRecord,
    policy_digest: Sha256Digest,
    renderer_capability_id: CapabilityRowId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CommitCommentPolicyRevalidationFailureEvidence {
    TaskBindingMismatch,
    DigestError(RepositoryResultContractError),
    FrozenPolicyMismatch,
}

/// Failed revalidation retains the frozen task-start policy and the exact
/// candidate record/capability. A caller can recover or retry without
/// reconstructing the linear frozen authority from wire scalars.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct CommitCommentPolicyRevalidationBlockedAuthority {
    frozen: FrozenCommitCommentPolicyAuthority,
    candidate_record: CommitCommentPolicyDigestRecord,
    candidate_renderer_capability_id: CapabilityRowId,
    failure: CommitCommentPolicyRevalidationFailureEvidence,
}

struct CommitCommentPolicyRevalidationCandidate {
    record: CommitCommentPolicyDigestRecord,
    renderer_capability_id: CapabilityRowId,
    task_bound: bool,
}

impl CommitCommentPolicyRevalidationBlockedAuthority {
    fn new(
        frozen: FrozenCommitCommentPolicyAuthority,
        candidate_record: CommitCommentPolicyDigestRecord,
        candidate_renderer_capability_id: CapabilityRowId,
        failure: CommitCommentPolicyRevalidationFailureEvidence,
    ) -> Box<Self> {
        Box::new(Self {
            frozen,
            candidate_record,
            candidate_renderer_capability_id,
            failure,
        })
    }

    pub(crate) fn failure(&self) -> &CommitCommentPolicyRevalidationFailureEvidence {
        &self.failure
    }

    pub(crate) fn into_recovery_parts(
        self: Box<Self>,
    ) -> (
        FrozenCommitCommentPolicyAuthority,
        CommitCommentPolicyDigestRecord,
        CapabilityRowId,
        CommitCommentPolicyRevalidationFailureEvidence,
    ) {
        let Self {
            frozen,
            candidate_record,
            candidate_renderer_capability_id,
            failure,
        } = *self;
        (
            frozen,
            candidate_record,
            candidate_renderer_capability_id,
            failure,
        )
    }
}

impl ValidatedCommitCommentPolicyAuthority {
    pub(crate) fn revalidate(
        frozen: FrozenCommitCommentPolicyAuthority,
        template: Comment,
        task_id: TaskId,
        task_summary: TaskSummary,
        project_id: ProjectId,
        rendered_comment: Comment,
        renderer_capability_id: CapabilityRowId,
    ) -> Result<Self, Box<CommitCommentPolicyRevalidationBlockedAuthority>> {
        let task_bound = template.as_str().contains("{taskId}")
            && rendered_comment.as_str().contains(task_id.as_str());
        let candidate = CommitCommentPolicyRevalidationCandidate {
            record: CommitCommentPolicyDigestRecord::new(
                template,
                task_id,
                task_summary,
                project_id,
                rendered_comment,
            ),
            renderer_capability_id,
            task_bound,
        };
        Self::revalidate_candidate_using_digest(
            frozen,
            candidate,
            CommitCommentPolicyDigestRecord::digest,
        )
    }

    fn revalidate_candidate_using_digest<F>(
        frozen: FrozenCommitCommentPolicyAuthority,
        candidate: CommitCommentPolicyRevalidationCandidate,
        digest: F,
    ) -> Result<Self, Box<CommitCommentPolicyRevalidationBlockedAuthority>>
    where
        F: FnOnce(
            &CommitCommentPolicyDigestRecord,
        ) -> Result<Sha256Digest, RepositoryResultContractError>,
    {
        let CommitCommentPolicyRevalidationCandidate {
            record,
            renderer_capability_id,
            task_bound,
        } = candidate;
        if !task_bound {
            return Err(CommitCommentPolicyRevalidationBlockedAuthority::new(
                frozen,
                record,
                renderer_capability_id,
                CommitCommentPolicyRevalidationFailureEvidence::TaskBindingMismatch,
            ));
        }
        let policy_digest = match digest(&record) {
            Ok(value) => value,
            Err(error) => {
                return Err(CommitCommentPolicyRevalidationBlockedAuthority::new(
                    frozen,
                    record,
                    renderer_capability_id,
                    CommitCommentPolicyRevalidationFailureEvidence::DigestError(error),
                ));
            }
        };
        if record != frozen.record
            || policy_digest != frozen.policy_digest
            || renderer_capability_id != frozen.renderer_capability_id
        {
            return Err(CommitCommentPolicyRevalidationBlockedAuthority::new(
                frozen,
                record,
                renderer_capability_id,
                CommitCommentPolicyRevalidationFailureEvidence::FrozenPolicyMismatch,
            ));
        }
        Ok(Self {
            record,
            policy_digest,
            renderer_capability_id,
        })
    }

    pub(crate) fn rendered_comment(&self) -> &Comment {
        &self.record.rendered_comment
    }

    pub(crate) fn policy_digest(&self) -> &Sha256Digest {
        &self.policy_digest
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct IntegrationSetLineageDigestRecord {
    plan_set_digest: Sha256Digest,
    merge_set_digest: Sha256Digest,
    verification_set_digest: Sha256Digest,
    commit_set_digest: Sha256Digest,
    lock_set_digest: Sha256Digest,
}

impl IntegrationSetLineageDigestRecord {
    pub(crate) fn new(
        plan_set_digest: Sha256Digest,
        merge_set_digest: Sha256Digest,
        verification_set_digest: Sha256Digest,
        commit_set_digest: Sha256Digest,
        lock_set_digest: Sha256Digest,
    ) -> Self {
        Self {
            plan_set_digest,
            merge_set_digest,
            verification_set_digest,
            commit_set_digest,
            lock_set_digest,
        }
    }

    pub(crate) fn digest(&self) -> Result<Sha256Digest, RepositoryResultContractError> {
        result_digest(self, "integration-set lineage digest failed")
    }
}

impl contract_digest_record_sealed::Sealed for IntegrationSetLineageDigestRecord {}
impl ContractDigestRecord for IntegrationSetLineageDigestRecord {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RepositoryRelevantAnchor {
    target: RepositoryTargetIdentity,
    anchor: RepositoryAnchor,
}

impl RepositoryRelevantAnchor {
    pub(crate) fn new(target: RepositoryTargetIdentity, anchor: RepositoryAnchor) -> Self {
        Self { target, anchor }
    }

    pub(crate) fn target(&self) -> &RepositoryTargetIdentity {
        &self.target
    }

    pub(crate) fn anchor(&self) -> &RepositoryAnchor {
        &self.anchor
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct RepositoryRelevantAnchors(Vec<RepositoryRelevantAnchor>);

impl RepositoryRelevantAnchors {
    pub(crate) fn new(
        values: Vec<RepositoryRelevantAnchor>,
    ) -> Result<Self, RepositoryResultContractError> {
        if values.is_empty()
            || values.len() > MAX_RESULT_ITEMS
            || values
                .windows(2)
                .any(|pair| pair[0].target >= pair[1].target)
        {
            return Err(RepositoryResultContractError(
                "relevant anchors must be non-empty, canonical, and unique by target",
            ));
        }
        Ok(Self(values))
    }

    pub(crate) fn as_slice(&self) -> &[RepositoryRelevantAnchor] {
        &self.0
    }
}

impl JsonSchema for RepositoryRelevantAnchors {
    fn schema_name() -> Cow<'static, str> {
        "RepositoryRelevantAnchors".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "array",
            "minItems": 1,
            "maxItems": MAX_RESULT_ITEMS,
            "uniqueItems": true,
            "items": generator.subschema_for::<RepositoryRelevantAnchor>(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct IntegrationSetDigestRecord {
    merge_session_id: UnicaId,
    resolved_session_digest: Sha256Digest,
    support_gate_id: UnicaId,
    support_gate_digest: Sha256Digest,
    support_gate_history_evidence_digest: Sha256Digest,
    verification_id: UnicaId,
    verification_digest: Sha256Digest,
    integration_entries: RepositoryIntegrationEntries,
    compatibility_mode: CompatibilityMode,
    reference_closure_digest: Sha256Digest,
    settings_digest: Sha256Digest,
    prevalidation_diagnostics_digest: Sha256Digest,
}

impl contract_digest_record_sealed::Sealed for IntegrationSetDigestRecord {}
impl ContractDigestRecord for IntegrationSetDigestRecord {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LockPlanDigestRecord {
    plan_id: UnicaId,
    merge_session_id: UnicaId,
    resolved_session_digest: Sha256Digest,
    support_gate_id: UnicaId,
    support_gate_digest: Sha256Digest,
    support_gate_history_evidence: SupportGateHistoryEvidence,
    verification_id: UnicaId,
    verification_digest: Sha256Digest,
    integration_set_id: UnicaId,
    integration_entries: RepositoryIntegrationEntries,
    integration_set_digest: Sha256Digest,
    lock_entries: RepositoryUpdateLockTargets,
    relevant_anchors: RepositoryRelevantAnchors,
    compatibility_mode: CompatibilityMode,
    reference_closure_digest: Sha256Digest,
    settings_digest: Sha256Digest,
    prevalidation_diagnostics_digest: Sha256Digest,
}

impl contract_digest_record_sealed::Sealed for LockPlanDigestRecord {}
impl ContractDigestRecord for LockPlanDigestRecord {}

/// Producer-side evidence for one completed main-integration lock plan.
///
/// Production construction consumes the exact successful main-sandbox
/// verification together with one atomically scoped planner capability result.
/// Tests can exercise the result contract through the cfg-only fixture
/// constructor below without opening a runtime bypass.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct LockPlanAuthority {
    record: LockPlanDigestRecord,
    gate_session_lineage: LockPlanGateSessionLineage,
}

/// Non-wire session values that the semantic support-gate digest already
/// commits to, retained so the immediate pre-effect check can compare the
/// authoritative current gate field-for-field rather than trusting digest
/// equality alone.
#[derive(Debug, Clone, PartialEq, Eq)]
struct LockPlanGateSessionLineage {
    comparison_id: UnicaId,
    ordinary_result_artifact_id: UnicaId,
    result_digest: Sha256Digest,
    planner_capability_id: CapabilityRowId,
}

#[cfg(test)]
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct DeleteSelfLockCapabilityEvidence {
    target: RepositoryTargetIdentity,
    exists_and_separately_lockable: bool,
    capability_row_id: CapabilityRowId,
}

#[cfg(test)]
impl DeleteSelfLockCapabilityEvidence {
    pub(crate) fn from_capability_adapter(
        target: RepositoryTargetIdentity,
        exists_and_separately_lockable: bool,
        capability_row_id: CapabilityRowId,
    ) -> Result<Self, RepositoryResultContractError> {
        if !matches!(target, RepositoryTargetIdentity::DevelopmentObject(_)) {
            return Err(RepositoryResultContractError(
                "delete self-lock capability must name a development object",
            ));
        }
        Ok(Self {
            target,
            exists_and_separately_lockable,
            capability_row_id,
        })
    }
}

/// Identifiers observed by one current repository-planner invocation.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct RepositoryLockPlanObservedIds {
    plan_id: UnicaId,
    integration_set_id: UnicaId,
}

impl RepositoryLockPlanObservedIds {
    pub(crate) fn new(plan_id: UnicaId, integration_set_id: UnicaId) -> Self {
        Self {
            plan_id,
            integration_set_id,
        }
    }
}

/// Non-topology evidence returned by the same planner invocation as its
/// identifiers, authoritative topology and lock rows.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct RepositoryLockPlanObservedEvidence {
    relevant_anchors: RepositoryRelevantAnchors,
    compatibility_mode: CompatibilityMode,
    reference_closure_digest: Sha256Digest,
    prevalidation_diagnostics_digest: Sha256Digest,
    planner_capability_id: CapabilityRowId,
}

impl RepositoryLockPlanObservedEvidence {
    pub(crate) fn from_planner_adapter(
        relevant_anchors: RepositoryRelevantAnchors,
        compatibility_mode: CompatibilityMode,
        reference_closure_digest: Sha256Digest,
        prevalidation_diagnostics_digest: Sha256Digest,
        planner_capability_id: CapabilityRowId,
    ) -> Self {
        Self {
            relevant_anchors,
            compatibility_mode,
            reference_closure_digest,
            prevalidation_diagnostics_digest,
            planner_capability_id,
        }
    }
}

/// Complete output of one current repository-planner invocation. It carries
/// typed authoritative topology rather than caller-built integration entries
/// or independent delete-self evidence.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct RepositoryLockPlanObservationInput {
    ids: RepositoryLockPlanObservedIds,
    topology: Vec<RepositoryIntegrationTopologyObservation>,
    lock_entries: RepositoryUpdateLockTargets,
    evidence: RepositoryLockPlanObservedEvidence,
}

impl RepositoryLockPlanObservationInput {
    pub(crate) fn from_planner_adapter(
        ids: RepositoryLockPlanObservedIds,
        topology: Vec<RepositoryIntegrationTopologyObservation>,
        lock_entries: RepositoryUpdateLockTargets,
        evidence: RepositoryLockPlanObservedEvidence,
    ) -> Self {
        Self {
            ids,
            topology,
            lock_entries,
            evidence,
        }
    }
}

#[derive(Debug)]
struct RepositoryLockPlanObservationInvocationMarker;

#[derive(Debug)]
struct RepositoryLockPlanObservationInvocationCapability(
    Arc<RepositoryLockPlanObservationInvocationMarker>,
);

#[derive(Debug)]
struct RepositoryLockPlanObservationCompletionCapability(
    Arc<RepositoryLockPlanObservationInvocationMarker>,
);

impl RepositoryLockPlanObservationInvocationCapability {
    fn mint() -> Self {
        Self(Arc::new(RepositoryLockPlanObservationInvocationMarker))
    }

    fn completion(&self) -> RepositoryLockPlanObservationCompletionCapability {
        RepositoryLockPlanObservationCompletionCapability(Arc::clone(&self.0))
    }

    fn owns_completion(
        &self,
        completion: &RepositoryLockPlanObservationCompletionCapability,
    ) -> bool {
        Arc::ptr_eq(&self.0, &completion.0)
    }
}

/// Exact verified main-sandbox scope made visible to one planner invocation.
#[derive(Debug)]
pub(crate) struct RepositoryLockPlanObservationRequest<'a> {
    verified_scope: &'a ValidatedMainSandboxVerificationAuthority,
    invocation: &'a RepositoryLockPlanObservationInvocationCapability,
}

impl RepositoryLockPlanObservationRequest<'_> {
    pub(crate) fn verification_id(&self) -> &UnicaId {
        self.verified_scope.verification_id()
    }

    pub(crate) fn verification_digest(&self) -> &Sha256Digest {
        self.verified_scope.verification_digest()
    }

    pub(crate) fn merge_session_id(&self) -> &UnicaId {
        self.verified_scope.merge_session_id()
    }

    pub(crate) fn resolved_session_digest(&self) -> &Sha256Digest {
        self.verified_scope.resolved_session_digest()
    }

    pub(crate) fn comparison_id(&self) -> &UnicaId {
        self.verified_scope.comparison_id()
    }

    pub(crate) fn support_gate_id(&self) -> &UnicaId {
        self.verified_scope.support_gate_id()
    }

    pub(crate) fn support_gate_digest(&self) -> &Sha256Digest {
        self.verified_scope.support_gate_digest()
    }

    pub(crate) fn support_gate_history_evidence(&self) -> &SupportGateHistoryEvidence {
        self.verified_scope.support_gate_history_evidence()
    }

    pub(crate) fn settings_digest(&self) -> &Sha256Digest {
        self.verified_scope.settings_digest()
    }

    pub(crate) fn ordinary_result_artifact_id(&self) -> &UnicaId {
        self.verified_scope.ordinary_result_artifact_id()
    }

    pub(crate) fn result_digest(&self) -> &Sha256Digest {
        self.verified_scope.result_digest()
    }

    pub(crate) fn applied_decision_ids(&self) -> &[UnicaId] {
        self.verified_scope.applied_decision_ids()
    }
}

/// Request-bound completed planner batch. The topology has already been
/// projected by the core and checked against the observed physical lock rows.
#[derive(Debug)]
pub(crate) struct RepositoryLockPlanObservationLease {
    completion: RepositoryLockPlanObservationCompletionCapability,
    ids: RepositoryLockPlanObservedIds,
    topology_batch: RepositoryIntegrationTopologyBatchAuthority,
    lock_entries: RepositoryUpdateLockTargets,
    evidence: RepositoryLockPlanObservedEvidence,
}

impl RepositoryLockPlanObservationLease {
    pub(crate) fn complete_from_planner_adapter(
        request: &RepositoryLockPlanObservationRequest<'_>,
        input: RepositoryLockPlanObservationInput,
    ) -> Result<Self, RepositoryResultContractError> {
        let RepositoryLockPlanObservationInput {
            ids,
            topology,
            lock_entries,
            evidence,
        } = input;
        let topology_batch = RepositoryIntegrationTopologyBatchAuthority::derive(topology)?;
        topology_batch.validate_lock_entries(&lock_entries)?;
        Ok(Self {
            completion: request.invocation.completion(),
            ids,
            topology_batch,
            lock_entries,
            evidence,
        })
    }
}

pub(crate) trait RepositoryLockPlanObservationPort {
    fn observe_lock_plan(
        &mut self,
        request: RepositoryLockPlanObservationRequest<'_>,
    ) -> Result<RepositoryLockPlanObservationLease, RepositoryResultContractError>;
}

/// One atomic planner result. All plan identifiers, integration entries,
/// closure evidence and lock rows come from the same capability invocation.
/// It is deliberately non-Clone so consumers cannot split and recombine rows.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct AtomicRepositoryLockPlanCapabilityAuthority {
    plan_id: UnicaId,
    integration_set_id: UnicaId,
    merge_session_id: UnicaId,
    resolved_session_digest: Sha256Digest,
    support_gate_id: UnicaId,
    support_gate_digest: Sha256Digest,
    support_gate_history_evidence: SupportGateHistoryEvidence,
    verification_id: UnicaId,
    verification_digest: Sha256Digest,
    settings_digest: Sha256Digest,
    comparison_id: UnicaId,
    ordinary_result_artifact_id: UnicaId,
    result_digest: Sha256Digest,
    topology_batch: RepositoryIntegrationTopologyBatchAuthority,
    lock_entries: RepositoryUpdateLockTargets,
    relevant_anchors: RepositoryRelevantAnchors,
    compatibility_mode: CompatibilityMode,
    reference_closure_digest: Sha256Digest,
    prevalidation_diagnostics_digest: Sha256Digest,
    planner_capability_id: CapabilityRowId,
}

impl AtomicRepositoryLockPlanCapabilityAuthority {
    pub(crate) fn from_observation_port(
        verified_scope: &ValidatedMainSandboxVerificationAuthority,
        port: &mut dyn RepositoryLockPlanObservationPort,
    ) -> Result<Self, RepositoryResultContractError> {
        let invocation = RepositoryLockPlanObservationInvocationCapability::mint();
        let request = RepositoryLockPlanObservationRequest {
            verified_scope,
            invocation: &invocation,
        };
        let lease = port.observe_lock_plan(request)?;
        if !invocation.owns_completion(&lease.completion) {
            return Err(RepositoryResultContractError(
                "repository lock-plan completion belongs to another planner invocation",
            ));
        }
        let RepositoryLockPlanObservationLease {
            completion: _,
            ids,
            topology_batch,
            lock_entries,
            evidence,
        } = lease;
        let RepositoryLockPlanObservedIds {
            plan_id,
            integration_set_id,
        } = ids;
        let RepositoryLockPlanObservedEvidence {
            relevant_anchors,
            compatibility_mode,
            reference_closure_digest,
            prevalidation_diagnostics_digest,
            planner_capability_id,
        } = evidence;
        Ok(Self {
            plan_id,
            integration_set_id,
            merge_session_id: verified_scope.merge_session_id().clone(),
            resolved_session_digest: verified_scope.resolved_session_digest().clone(),
            support_gate_id: verified_scope.support_gate_id().clone(),
            support_gate_digest: verified_scope.support_gate_digest().clone(),
            support_gate_history_evidence: verified_scope.support_gate_history_evidence().clone(),
            verification_id: verified_scope.verification_id().clone(),
            verification_digest: verified_scope.verification_digest().clone(),
            settings_digest: verified_scope.settings_digest().clone(),
            comparison_id: verified_scope.comparison_id().clone(),
            ordinary_result_artifact_id: verified_scope.ordinary_result_artifact_id().clone(),
            result_digest: verified_scope.result_digest().clone(),
            topology_batch,
            lock_entries,
            relevant_anchors,
            compatibility_mode,
            reference_closure_digest,
            prevalidation_diagnostics_digest,
            planner_capability_id,
        })
    }
}

#[cfg(test)]
pub(crate) struct LockPlanAuthorityTestParts {
    pub(crate) plan_id: UnicaId,
    pub(crate) merge_session_id: UnicaId,
    pub(crate) resolved_session_digest: Sha256Digest,
    pub(crate) support_gate_id: UnicaId,
    pub(crate) support_gate_digest: Sha256Digest,
    pub(crate) support_gate_history_evidence: SupportGateHistoryEvidence,
    pub(crate) verification_id: UnicaId,
    pub(crate) verification_digest: Sha256Digest,
    pub(crate) integration_set_id: UnicaId,
    pub(crate) integration_entries: RepositoryIntegrationEntries,
    pub(crate) delete_self_lock_evidence: Vec<DeleteSelfLockCapabilityEvidence>,
    pub(crate) lock_entries: RepositoryUpdateLockTargets,
    pub(crate) relevant_anchors: RepositoryRelevantAnchors,
    pub(crate) compatibility_mode: CompatibilityMode,
    pub(crate) reference_closure_digest: Sha256Digest,
    pub(crate) settings_digest: Sha256Digest,
    pub(crate) prevalidation_diagnostics_digest: Sha256Digest,
    pub(crate) gate_comparison_id: UnicaId,
    pub(crate) gate_ordinary_result_artifact_id: UnicaId,
    pub(crate) gate_result_digest: Sha256Digest,
    pub(crate) planner_capability_id: CapabilityRowId,
}

impl LockPlanAuthority {
    pub(crate) fn from_verified_main_sandbox(
        verified: ValidatedMainSandboxVerificationAuthority,
        planner: AtomicRepositoryLockPlanCapabilityAuthority,
    ) -> Result<Self, RepositoryResultContractError> {
        if &planner.merge_session_id != verified.merge_session_id()
            || &planner.resolved_session_digest != verified.resolved_session_digest()
            || &planner.support_gate_id != verified.support_gate_id()
            || &planner.support_gate_digest != verified.support_gate_digest()
            || &planner.support_gate_history_evidence != verified.support_gate_history_evidence()
            || &planner.verification_id != verified.verification_id()
            || &planner.verification_digest != verified.verification_digest()
            || &planner.settings_digest != verified.settings_digest()
            || &planner.comparison_id != verified.comparison_id()
            || &planner.ordinary_result_artifact_id != verified.ordinary_result_artifact_id()
            || &planner.result_digest != verified.result_digest()
        {
            return Err(RepositoryResultContractError(
                "atomic lock plan does not consume the exact sandbox verification scope",
            ));
        }
        planner
            .topology_batch
            .validate_lock_entries(&planner.lock_entries)?;

        let merge_session_id = verified.merge_session_id().clone();
        let resolved_session_digest = verified.resolved_session_digest().clone();
        let support_gate_id = verified.support_gate_id().clone();
        let support_gate_digest = verified.support_gate_digest().clone();
        let support_gate_history_evidence = verified.support_gate_history_evidence().clone();
        let verification_id = verified.verification_id().clone();
        let verification_digest = verified.verification_digest().clone();
        let settings_digest = verified.settings_digest().clone();
        let gate_session_lineage = LockPlanGateSessionLineage {
            comparison_id: verified.comparison_id().clone(),
            ordinary_result_artifact_id: verified.ordinary_result_artifact_id().clone(),
            result_digest: verified.result_digest().clone(),
            planner_capability_id: planner.planner_capability_id.clone(),
        };
        let _consumed_planning = verified.into_planning();
        let integration_set_digest = result_digest(
            &IntegrationSetDigestRecord {
                merge_session_id: merge_session_id.clone(),
                resolved_session_digest: resolved_session_digest.clone(),
                support_gate_id: support_gate_id.clone(),
                support_gate_digest: support_gate_digest.clone(),
                support_gate_history_evidence_digest: support_gate_history_evidence
                    .evidence_digest()
                    .clone(),
                verification_id: verification_id.clone(),
                verification_digest: verification_digest.clone(),
                integration_entries: planner.topology_batch.integration_entries().clone(),
                compatibility_mode: planner.compatibility_mode.clone(),
                reference_closure_digest: planner.reference_closure_digest.clone(),
                settings_digest: settings_digest.clone(),
                prevalidation_diagnostics_digest: planner.prevalidation_diagnostics_digest.clone(),
            },
            "integration-set digest failed",
        )?;
        Ok(Self {
            gate_session_lineage,
            record: LockPlanDigestRecord {
                plan_id: planner.plan_id,
                merge_session_id,
                resolved_session_digest,
                support_gate_id,
                support_gate_digest,
                support_gate_history_evidence,
                verification_id,
                verification_digest,
                integration_set_id: planner.integration_set_id,
                integration_entries: planner.topology_batch.into_integration_entries(),
                integration_set_digest,
                lock_entries: planner.lock_entries,
                relevant_anchors: planner.relevant_anchors,
                compatibility_mode: planner.compatibility_mode,
                reference_closure_digest: planner.reference_closure_digest,
                settings_digest,
                prevalidation_diagnostics_digest: planner.prevalidation_diagnostics_digest,
            },
        })
    }

    #[cfg(test)]
    pub(crate) fn test_only(
        parts: LockPlanAuthorityTestParts,
    ) -> Result<Self, RepositoryResultContractError> {
        validate_integration_lock_closure(
            &parts.integration_entries,
            &parts.lock_entries,
            &parts.delete_self_lock_evidence,
        )?;
        let integration_set_digest = result_digest(
            &IntegrationSetDigestRecord {
                merge_session_id: parts.merge_session_id.clone(),
                resolved_session_digest: parts.resolved_session_digest.clone(),
                support_gate_id: parts.support_gate_id.clone(),
                support_gate_digest: parts.support_gate_digest.clone(),
                support_gate_history_evidence_digest: parts
                    .support_gate_history_evidence
                    .evidence_digest()
                    .clone(),
                verification_id: parts.verification_id.clone(),
                verification_digest: parts.verification_digest.clone(),
                integration_entries: parts.integration_entries.clone(),
                compatibility_mode: parts.compatibility_mode.clone(),
                reference_closure_digest: parts.reference_closure_digest.clone(),
                settings_digest: parts.settings_digest.clone(),
                prevalidation_diagnostics_digest: parts.prevalidation_diagnostics_digest.clone(),
            },
            "integration-set digest failed",
        )?;
        Ok(Self {
            gate_session_lineage: LockPlanGateSessionLineage {
                comparison_id: parts.gate_comparison_id,
                ordinary_result_artifact_id: parts.gate_ordinary_result_artifact_id,
                result_digest: parts.gate_result_digest,
                planner_capability_id: parts.planner_capability_id,
            },
            record: LockPlanDigestRecord {
                plan_id: parts.plan_id,
                merge_session_id: parts.merge_session_id,
                resolved_session_digest: parts.resolved_session_digest,
                support_gate_id: parts.support_gate_id,
                support_gate_digest: parts.support_gate_digest,
                support_gate_history_evidence: parts.support_gate_history_evidence,
                verification_id: parts.verification_id,
                verification_digest: parts.verification_digest,
                integration_set_id: parts.integration_set_id,
                integration_entries: parts.integration_entries,
                integration_set_digest,
                lock_entries: parts.lock_entries,
                relevant_anchors: parts.relevant_anchors,
                compatibility_mode: parts.compatibility_mode,
                reference_closure_digest: parts.reference_closure_digest,
                settings_digest: parts.settings_digest,
                prevalidation_diagnostics_digest: parts.prevalidation_diagnostics_digest,
            },
        })
    }
}

#[cfg(test)]
fn validate_integration_lock_closure(
    entries: &RepositoryIntegrationEntries,
    lock_entries: &RepositoryUpdateLockTargets,
    delete_evidence: &[DeleteSelfLockCapabilityEvidence],
) -> Result<(), RepositoryResultContractError> {
    if delete_evidence
        .windows(2)
        .any(|pair| pair[0].target >= pair[1].target)
    {
        return Err(RepositoryResultContractError(
            "delete self-lock evidence must be canonical and unique",
        ));
    }
    let content_targets = entries
        .as_slice()
        .iter()
        .map(RepositoryIntegrationEntry::target_identity)
        .collect::<BTreeSet<_>>();
    for lock in lock_entries.as_slice() {
        match lock.as_ref() {
            RepositoryUpdateLockTargetRef::ConfigurationRoot { reasons, .. } => {
                if reasons.first() != Some(&RepositoryUpdateLockReason::SupportGraphGuard)
                    || content_targets.contains(&RepositoryTargetIdentity::configuration_root())
                        && !reasons.contains(&RepositoryUpdateLockReason::UpdateTarget)
                {
                    return Err(RepositoryResultContractError(
                        "root lock requires supportGraphGuard first and updateTarget for root content",
                    ));
                }
            }
            RepositoryUpdateLockTargetRef::DevelopmentObject {
                object_id, reasons, ..
            } => {
                let target = RepositoryTargetIdentity::development_object(object_id.clone());
                let is_content = content_targets.contains(&target);
                let is_update_target = reasons.contains(&RepositoryUpdateLockReason::UpdateTarget);
                let has_closure_role = reasons.iter().any(|reason| {
                    matches!(
                        reason,
                        RepositoryUpdateLockReason::ParentClosure
                            | RepositoryUpdateLockReason::ReferenceClosure
                            | RepositoryUpdateLockReason::StructuralClosure
                    )
                });
                if reasons.contains(&RepositoryUpdateLockReason::SupportGraphGuard)
                    || is_content != is_update_target
                    || !is_content && !has_closure_role
                {
                    return Err(RepositoryResultContractError(
                        "object lock reasons do not explain its exact content or closure role",
                    ));
                }
            }
        }
    }
    let mut required = BTreeSet::new();
    required.insert(RepositoryTargetIdentity::configuration_root());
    let mut delete_index = 0;
    for entry in entries.as_slice() {
        let target = entry.target_identity();
        let required_targets = entry.required_lock_targets().as_slice();
        required.extend(required_targets.iter().cloned());
        match entry {
            RepositoryIntegrationEntry::RootModify(value) => {
                if required_targets.binary_search(&target).is_err()
                    || !value
                        .reasons
                        .as_slice()
                        .contains(&RepositoryIntegrationReason::CanonicalDelta)
                {
                    return Err(RepositoryResultContractError(
                        "root modification lacks its exact target lock or canonical reason",
                    ));
                }
            }
            RepositoryIntegrationEntry::ObjectAdd(value) => {
                if required_targets.binary_search(&target).is_ok()
                    || !value
                        .reasons
                        .as_slice()
                        .contains(&RepositoryIntegrationReason::AddDeleteSemantics)
                {
                    return Err(RepositoryResultContractError(
                        "object addition illegally locks itself or lacks add/delete semantics",
                    ));
                }
            }
            RepositoryIntegrationEntry::ObjectModify(value) => {
                if required_targets.binary_search(&target).is_err()
                    || !value
                        .reasons
                        .as_slice()
                        .contains(&RepositoryIntegrationReason::CanonicalDelta)
                {
                    return Err(RepositoryResultContractError(
                        "object modification lacks its exact target lock or canonical reason",
                    ));
                }
            }
            RepositoryIntegrationEntry::ObjectDelete(value) => {
                if !value
                    .reasons
                    .as_slice()
                    .contains(&RepositoryIntegrationReason::AddDeleteSemantics)
                {
                    return Err(RepositoryResultContractError(
                        "object deletion lacks add/delete semantics",
                    ));
                }
                let Some(evidence) = delete_evidence.get(delete_index) else {
                    return Err(RepositoryResultContractError(
                        "object deletion lacks self-lock capability evidence",
                    ));
                };
                let _capability_row_id = &evidence.capability_row_id;
                if evidence.target != target
                    || (required_targets.binary_search(&target).is_ok()
                        != evidence.exists_and_separately_lockable)
                {
                    return Err(RepositoryResultContractError(
                        "delete self-lock presence disagrees with capability evidence",
                    ));
                }
                delete_index += 1;
            }
        }
    }
    if delete_index != delete_evidence.len() {
        return Err(RepositoryResultContractError(
            "delete self-lock evidence has an extra or cross-target row",
        ));
    }
    let observed = project_lock_targets(lock_entries)?;
    if required.into_iter().collect::<Vec<_>>() != observed.0 {
        return Err(RepositoryResultContractError(
            "lock plan is broader or narrower than the exact integration closure",
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct LockPlanData {
    plan_id: UnicaId,
    merge_session_id: UnicaId,
    resolved_session_digest: Sha256Digest,
    support_gate_id: UnicaId,
    support_gate_digest: Sha256Digest,
    support_gate_history_evidence: SupportGateHistoryEvidence,
    verification_id: UnicaId,
    verification_digest: Sha256Digest,
    integration_set_id: UnicaId,
    integration_entries: RepositoryIntegrationEntries,
    integration_set_digest: Sha256Digest,
    lock_entries: RepositoryUpdateLockTargets,
    relevant_anchors: RepositoryRelevantAnchors,
    compatibility_mode: CompatibilityMode,
    reference_closure_digest: Sha256Digest,
    settings_digest: Sha256Digest,
    prevalidation_diagnostics_digest: Sha256Digest,
    plan_digest: Sha256Digest,
    #[serde(skip)]
    #[schemars(skip)]
    gate_session_lineage: LockPlanGateSessionLineage,
}

impl LockPlanData {
    pub(crate) fn from_authority(
        authority: LockPlanAuthority,
    ) -> Result<Self, RepositoryResultContractError> {
        let plan_digest = result_digest(&authority.record, "lock-plan digest failed")?;
        let LockPlanAuthority {
            record,
            gate_session_lineage,
        } = authority;
        Ok(Self {
            plan_id: record.plan_id,
            merge_session_id: record.merge_session_id,
            resolved_session_digest: record.resolved_session_digest,
            support_gate_id: record.support_gate_id,
            support_gate_digest: record.support_gate_digest,
            support_gate_history_evidence: record.support_gate_history_evidence,
            verification_id: record.verification_id,
            verification_digest: record.verification_digest,
            integration_set_id: record.integration_set_id,
            integration_entries: record.integration_entries,
            integration_set_digest: record.integration_set_digest,
            lock_entries: record.lock_entries,
            relevant_anchors: record.relevant_anchors,
            compatibility_mode: record.compatibility_mode,
            reference_closure_digest: record.reference_closure_digest,
            settings_digest: record.settings_digest,
            prevalidation_diagnostics_digest: record.prevalidation_diagnostics_digest,
            plan_digest,
            gate_session_lineage,
        })
    }

    pub(crate) fn integration_entries(&self) -> &RepositoryIntegrationEntries {
        &self.integration_entries
    }

    pub(crate) fn merge_session_id(&self) -> &UnicaId {
        &self.merge_session_id
    }

    pub(crate) fn resolved_session_digest(&self) -> &Sha256Digest {
        &self.resolved_session_digest
    }

    pub(crate) fn support_gate_id(&self) -> &UnicaId {
        &self.support_gate_id
    }

    pub(crate) fn support_gate_digest(&self) -> &Sha256Digest {
        &self.support_gate_digest
    }

    pub(crate) fn support_gate_history_evidence(&self) -> &SupportGateHistoryEvidence {
        &self.support_gate_history_evidence
    }

    pub(crate) fn verification_id(&self) -> &UnicaId {
        &self.verification_id
    }

    pub(crate) fn verification_digest(&self) -> &Sha256Digest {
        &self.verification_digest
    }

    pub(crate) fn integration_set_id(&self) -> &UnicaId {
        &self.integration_set_id
    }

    pub(crate) fn integration_set_digest(&self) -> &Sha256Digest {
        &self.integration_set_digest
    }

    pub(crate) fn plan_digest(&self) -> &Sha256Digest {
        &self.plan_digest
    }

    pub(crate) fn exact_objects(&self) -> CommitExactObjects {
        self.integration_entries.exact_objects()
    }

    pub(crate) fn lock_entries(&self) -> &RepositoryUpdateLockTargets {
        &self.lock_entries
    }

    pub(crate) fn relevant_anchors(&self) -> &RepositoryRelevantAnchors {
        &self.relevant_anchors
    }

    pub(crate) fn reference_closure_digest(&self) -> &Sha256Digest {
        &self.reference_closure_digest
    }

    pub(crate) fn settings_digest(&self) -> &Sha256Digest {
        &self.settings_digest
    }

    pub(crate) fn prevalidation_diagnostics_digest(&self) -> &Sha256Digest {
        &self.prevalidation_diagnostics_digest
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LockSetDigestRecord {
    plan_digest: Sha256Digest,
    integration_set_id: UnicaId,
    integration_set_digest: Sha256Digest,
    acquired: RepositoryUpdateLockTargets,
}

impl contract_digest_record_sealed::Sealed for LockSetDigestRecord {}
impl ContractDigestRecord for LockSetDigestRecord {}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct LockAcquisitionObservationAuthority {
    lock_set_id: UnicaId,
    acquired: RepositoryUpdateLockTargets,
}

impl LockAcquisitionObservationAuthority {
    pub(crate) fn from_repository_adapter(
        lock_set_id: UnicaId,
        acquired: RepositoryUpdateLockTargets,
    ) -> Self {
        Self {
            lock_set_id,
            acquired,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ValidatedLockSetAuthority {
    plan: LockPlanData,
    lock_set_id: UnicaId,
    lock_set_digest: Sha256Digest,
    gate_proof: ValidatedLockGateProof,
}

#[derive(Debug, PartialEq, Eq)]
enum ValidatedLockGateProof {
    Production {
        current_gate: Box<CurrentReadySupportGateAuthority>,
        root_lock_receipt: JournaledRepositoryLock,
        journaled_lock_receipts: Vec<JournaledRepositoryLock>,
        root_reread_capability_id: CapabilityRowId,
    },
    #[cfg(test)]
    Fixture,
}

fn planned_lock_target_identity(
    target: &crate::domain::branched_development::contracts::repository::RepositoryUpdateLockTarget,
) -> RepositoryTargetIdentity {
    match target.as_ref() {
        RepositoryUpdateLockTargetRef::ConfigurationRoot { .. } => {
            RepositoryTargetIdentity::configuration_root()
        }
        RepositoryUpdateLockTargetRef::DevelopmentObject { object_id, .. } => {
            RepositoryTargetIdentity::DevelopmentObject(ObjectTargetIdentity::new(
                object_id.clone(),
            ))
        }
    }
}

fn planned_lock_target_identities(plan: &LockPlanData) -> Vec<RepositoryTargetIdentity> {
    plan.lock_entries
        .as_slice()
        .iter()
        .map(planned_lock_target_identity)
        .collect()
}

/// Presentation is deliberately excluded: only stable target identity,
/// lock-set identity, and the journal observation instant identify a receipt.
fn same_journaled_lock_identity(
    left: &JournaledRepositoryLock,
    right: &JournaledRepositoryLock,
) -> bool {
    left.target == right.target
        && left.lock_set_id == right.lock_set_id
        && left.observed_at == right.observed_at
}

fn receipt_binds_target_and_lock_set(
    receipt: &JournaledRepositoryLock,
    target: &RepositoryTargetIdentity,
    lock_set_id: &UnicaId,
) -> bool {
    &receipt.target == target && &receipt.lock_set_id == lock_set_id
}

fn stable_identity_deduped_receipts(
    root_receipt: Option<&JournaledRepositoryLock>,
    receipt_groups: &[&[JournaledRepositoryLock]],
) -> Vec<JournaledRepositoryLock> {
    let mut retained = root_receipt.into_iter().cloned().collect::<Vec<_>>();
    for group in receipt_groups {
        for receipt in *group {
            if !retained
                .iter()
                .any(|known| same_journaled_lock_identity(known, receipt))
            {
                retained.push(receipt.clone());
            }
        }
    }
    retained
}

/// Consuming pre-effect proof. Construction compares the complete plan-bound
/// gate/session lineage with a freshly resolved authoritative current gate and
/// therefore must complete before the root-lock adapter is called.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct PreLockCurrentGateAuthority {
    plan: LockPlanData,
    current_gate: CurrentReadySupportGateAuthority,
}

impl PreLockCurrentGateAuthority {
    pub(crate) fn recheck(
        plan: LockPlanData,
        current_gate: CurrentReadySupportGateAuthority,
    ) -> Result<Self, RepositoryResultContractError> {
        let root_anchor = plan
            .relevant_anchors
            .as_slice()
            .iter()
            .find(|value| value.target() == &RepositoryTargetIdentity::configuration_root())
            .ok_or(RepositoryResultContractError(
                "current-gate recheck requires the configuration-root anchor",
            ))?;
        let root_target =
            plan.lock_entries
                .as_slice()
                .first()
                .ok_or(RepositoryResultContractError(
                    "lock plan lacks its configuration-root guard",
                ))?;
        let root_is_support_guard = matches!(
            root_target.as_ref(),
            RepositoryUpdateLockTargetRef::ConfigurationRoot { reasons, .. }
                if reasons.first() == Some(&RepositoryUpdateLockReason::SupportGraphGuard)
        );
        if !root_is_support_guard
            || plan.support_gate_id != *current_gate.support_gate_id()
            || plan.support_gate_digest != *current_gate.support_gate_digest()
            || plan.support_gate_history_evidence != *current_gate.history_evidence()
            || plan.settings_digest != *current_gate.settings_digest()
            || plan.gate_session_lineage.comparison_id != *current_gate.comparison_id()
            || plan.gate_session_lineage.ordinary_result_artifact_id
                != *current_gate.ordinary_result_artifact_id()
            || plan.gate_session_lineage.result_digest != *current_gate.sandbox_result_digest()
            || plan.support_gate_history_evidence.gate_observed_cursor()
                != current_gate.observed_history_cursor()
            || plan
                .support_gate_history_evidence
                .relevant_baseline_digest()
                != current_gate.relevant_baseline_digest()
            || root_anchor.anchor().history_cursor()
                != current_gate.history_evidence().classified_through_cursor()
            || root_anchor.anchor().configuration_fingerprint()
                != current_gate.original_fingerprint()
        {
            return Err(RepositoryResultContractError(
                "lock plan does not bind the exact latest current ready support gate",
            ));
        }
        Ok(Self { plan, current_gate })
    }

    pub(crate) fn plan(&self) -> &LockPlanData {
        &self.plan
    }

    pub(crate) fn current_gate(&self) -> &CurrentReadySupportGateAuthority {
        &self.current_gate
    }

    pub(crate) fn root_target_display(&self) -> &RepositoryTargetDisplay {
        match self
            .plan
            .lock_entries
            .as_slice()
            .first()
            .expect("prelock validated the root target")
            .as_ref()
        {
            RepositoryUpdateLockTargetRef::ConfigurationRoot { object_display, .. } => {
                object_display
            }
            RepositoryUpdateLockTargetRef::DevelopmentObject { .. } => {
                unreachable!("prelock validated the root target")
            }
        }
    }

    /// One root-first operation: the adapter journals the exact root receipt,
    /// then rereads the gate and complete support graph under that receipt.
    pub(crate) fn acquire_root_and_reread_current_gate(
        self,
        port: &mut dyn RootGuardAcquisitionPort,
    ) -> Result<RootGuardedCurrentGateAuthority, RootGuardAcquisitionBlockedAuthority> {
        let outcome = port.acquire_root_and_reread_current_gate(&self);
        let lease = match outcome {
            RootGuardAcquisitionPortOutcome::Held(lease) => lease,
            RootGuardAcquisitionPortOutcome::FirstRootConflict(observation) => {
                return Err(
                    RootGuardAcquisitionBlockedAuthority::from_first_root_conflict(
                        self.plan,
                        self.current_gate,
                        observation,
                    ),
                );
            }
            RootGuardAcquisitionPortOutcome::Recovery(observation) => {
                return Err(RootGuardAcquisitionBlockedAuthority::from_root_recovery(
                    self.plan,
                    self.current_gate,
                    observation,
                ));
            }
        };
        let receipt = lease.root_lock_receipt().clone();
        let pre_reread_lock_observation = lease.pre_reread_lock_observation().clone();
        let expected_root = RepositoryTargetIdentity::configuration_root();
        let protocol_valid = receipt.target() == &expected_root
            && pre_reread_lock_observation.lock_set_id == *receipt.lock_set_id()
            && pre_reread_lock_observation.attempted_targets == [expected_root]
            && pre_reread_lock_observation.journaled_acquired.len() == 1
            && same_journaled_lock_identity(
                &pre_reread_lock_observation.journaled_acquired[0],
                &receipt,
            )
            && pre_reread_lock_observation.released.is_empty()
            && pre_reread_lock_observation.retained.len() == 1
            && same_journaled_lock_identity(&pre_reread_lock_observation.retained[0], &receipt)
            && lease.complete_support_graph_was_reread();
        let semantics_unchanged = lease.reread_support_gate_id()
            == self.current_gate.support_gate_id()
            && lease.reread_support_gate_digest() == self.current_gate.support_gate_digest()
            && lease.reread_history_evidence() == self.current_gate.history_evidence()
            && lease.reread_support_graph_digest() == self.current_gate.support_graph_digest()
            && lease.reread_relevant_baseline_digest()
                == self.current_gate.relevant_baseline_digest()
            && lease.reread_original_fingerprint() == self.current_gate.original_fingerprint()
            && lease.reread_state_revision() == self.current_gate.current_state_revision();
        if !protocol_valid || !semantics_unchanged {
            let release = lease.release_root();
            return Err(RootGuardAcquisitionBlockedAuthority::from_held_failure(
                self.plan,
                self.current_gate,
                receipt,
                pre_reread_lock_observation,
                protocol_valid,
                semantics_unchanged,
                release,
            ));
        }
        let root_reread_capability_id = lease.root_reread_capability_id().clone();
        Ok(RootGuardedCurrentGateAuthority {
            plan: self.plan,
            current_gate: self.current_gate,
            root_lock_receipt: receipt,
            root_reread_capability_id,
        })
    }
}

/// Complete adapter observation immediately before the support-gate reread.
/// A bare target list is not effect evidence: every reported acquisition,
/// release, and retained lock is carried as its exact journal receipt together
/// with the adapter-observed lock-set identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RootGuardPreRereadLockObservation {
    lock_set_id: UnicaId,
    attempted_targets: Vec<RepositoryTargetIdentity>,
    journaled_acquired: Vec<JournaledRepositoryLock>,
    released: Vec<JournaledRepositoryLock>,
    retained: Vec<JournaledRepositoryLock>,
}

impl RootGuardPreRereadLockObservation {
    pub(crate) fn from_repository_adapter(
        lock_set_id: UnicaId,
        attempted_targets: Vec<RepositoryTargetIdentity>,
        journaled_acquired: Vec<JournaledRepositoryLock>,
        released: Vec<JournaledRepositoryLock>,
        retained: Vec<JournaledRepositoryLock>,
    ) -> Self {
        Self {
            lock_set_id,
            attempted_targets,
            journaled_acquired,
            released,
            retained,
        }
    }
}

/// Root acquisition/reread output supplied by the repository adapter. The
/// core independently compares every temporal observation before minting the
/// guarded authority.
pub(crate) trait RootGuardAcquisitionLease {
    fn root_lock_receipt(&self) -> &JournaledRepositoryLock;
    fn pre_reread_lock_observation(&self) -> &RootGuardPreRereadLockObservation;
    fn reread_support_gate_id(&self) -> &UnicaId;
    fn reread_support_gate_digest(&self) -> &Sha256Digest;
    fn reread_history_evidence(&self) -> &SupportGateHistoryEvidence;
    fn complete_support_graph_was_reread(&self) -> bool;
    fn reread_support_graph_digest(&self) -> &Sha256Digest;
    fn reread_relevant_baseline_digest(&self) -> &Sha256Digest;
    fn reread_original_fingerprint(&self) -> &Sha256Digest;
    fn reread_state_revision(&self) -> &Sha256Digest;
    fn root_reread_capability_id(&self) -> &CapabilityRowId;
    fn release_root(self: Box<Self>) -> RootGuardReleaseObservation;
}

pub(crate) enum RootGuardAcquisitionPortOutcome {
    Held(Box<dyn RootGuardAcquisitionLease>),
    FirstRootConflict(RootFirstLockConflictObservation),
    Recovery(RepositoryLockRecoveryObservation),
}

pub(crate) trait RootGuardAcquisitionPort {
    fn acquire_root_and_reread_current_gate(
        &mut self,
        request: &PreLockCurrentGateAuthority,
    ) -> RootGuardAcquisitionPortOutcome;
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct RootFirstLockConflictObservation {
    failed_target: RepositoryTargetIdentity,
    attempted_targets: Vec<RepositoryTargetIdentity>,
    acquired: Vec<JournaledRepositoryLock>,
    conflict_capability_id: CapabilityRowId,
}

impl RootFirstLockConflictObservation {
    pub(crate) fn from_repository_adapter(
        failed_target: RepositoryTargetIdentity,
        attempted_targets: Vec<RepositoryTargetIdentity>,
        acquired: Vec<JournaledRepositoryLock>,
        conflict_capability_id: CapabilityRowId,
    ) -> Self {
        Self {
            failed_target,
            attempted_targets,
            acquired,
            conflict_capability_id,
        }
    }
}

/// Complete raw evidence retained whenever the adapter cannot prove a clean
/// success or a fully compensated conflict. It is observation only, never a
/// lock or stop authority by itself.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct RepositoryLockRecoveryObservation {
    lock_set_id: Option<UnicaId>,
    attempted_targets: Vec<RepositoryTargetIdentity>,
    acquired: Vec<JournaledRepositoryLock>,
    failed_target: Option<RepositoryTargetIdentity>,
    released: Vec<JournaledRepositoryLock>,
    retained: Vec<JournaledRepositoryLock>,
    recovery_capability_id: CapabilityRowId,
}

impl RepositoryLockRecoveryObservation {
    pub(crate) fn from_repository_adapter(
        lock_set_id: Option<UnicaId>,
        attempted_targets: Vec<RepositoryTargetIdentity>,
        acquired: Vec<JournaledRepositoryLock>,
        failed_target: Option<RepositoryTargetIdentity>,
        released: Vec<JournaledRepositoryLock>,
        retained: Vec<JournaledRepositoryLock>,
        recovery_capability_id: CapabilityRowId,
    ) -> Self {
        Self {
            lock_set_id,
            attempted_targets,
            acquired,
            failed_target,
            released,
            retained,
            recovery_capability_id,
        }
    }
}

/// Adapter observation only. It cannot authorize a stop or recovery path until
/// the core binds it to the exact receipt owned by the failed root operation.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum RootGuardReleaseObservation {
    Verified {
        released: Vec<JournaledRepositoryLock>,
        retained: Vec<JournaledRepositoryLock>,
        release_capability_id: CapabilityRowId,
    },
    Unverified {
        released: Vec<JournaledRepositoryLock>,
        retained: Vec<JournaledRepositoryLock>,
        recovery_capability_id: CapabilityRowId,
    },
}

impl RootGuardReleaseObservation {
    pub(crate) fn verified_from_repository_adapter(
        released: Vec<JournaledRepositoryLock>,
        retained: Vec<JournaledRepositoryLock>,
        release_capability_id: CapabilityRowId,
    ) -> Self {
        Self::Verified {
            released,
            retained,
            release_capability_id,
        }
    }

    pub(crate) fn unverified_from_repository_adapter(
        released: Vec<JournaledRepositoryLock>,
        retained: Vec<JournaledRepositoryLock>,
        recovery_capability_id: CapabilityRowId,
    ) -> Self {
        Self::Unverified {
            released,
            retained,
            recovery_capability_id,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
struct RootLockAttemptContext {
    plan: LockPlanData,
    current_gate: CurrentReadySupportGateAuthority,
}

#[derive(Debug, PartialEq, Eq)]
struct RepositoryLockAttemptEvidence {
    lock_set_id: Option<UnicaId>,
    attempted_targets: Vec<RepositoryTargetIdentity>,
    acquired: Vec<JournaledRepositoryLock>,
    failed_target: Option<RepositoryTargetIdentity>,
    released: Vec<JournaledRepositoryLock>,
    reported_retained: Vec<JournaledRepositoryLock>,
    /// Conservative stable-identity union used by recovery. This is empty only
    /// after an exact verified compensation/release proof.
    retained: Vec<JournaledRepositoryLock>,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct VerifiedRootGateDriftStopAuthority {
    context: Box<RootLockAttemptContext>,
    evidence: RepositoryLockAttemptEvidence,
    release_capability_id: CapabilityRowId,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct RootLockConflictStopAuthority {
    context: Box<RootLockAttemptContext>,
    evidence: RepositoryLockAttemptEvidence,
    conflict_capability_id: CapabilityRowId,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct RootLockRecoveryAuthority {
    context: Box<RootLockAttemptContext>,
    evidence: RepositoryLockAttemptEvidence,
    recovery_capability_id: CapabilityRowId,
}

#[derive(Debug, PartialEq, Eq)]
enum RootGuardAcquisitionBlockedKind {
    VerifiedStale(VerifiedRootGateDriftStopAuthority),
    FirstRootConflict(RootLockConflictStopAuthority),
    Recovery(RootLockRecoveryAuthority),
}

/// Sealed root-operation failure. Only its typed consuming projections can
/// reach a stale stop, a first-root conflict stop, or recovery.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct RootGuardAcquisitionBlockedAuthority {
    kind: Box<RootGuardAcquisitionBlockedKind>,
}

impl RootGuardAcquisitionBlockedAuthority {
    fn context(
        plan: LockPlanData,
        current_gate: CurrentReadySupportGateAuthority,
    ) -> Box<RootLockAttemptContext> {
        Box::new(RootLockAttemptContext { plan, current_gate })
    }

    fn from_held_failure(
        plan: LockPlanData,
        current_gate: CurrentReadySupportGateAuthority,
        receipt: JournaledRepositoryLock,
        pre_reread: RootGuardPreRereadLockObservation,
        protocol_valid: bool,
        semantics_unchanged: bool,
        release: RootGuardReleaseObservation,
    ) -> Self {
        let (post_release_released, post_release_retained, release_verified, capability_id) =
            match release {
                RootGuardReleaseObservation::Verified {
                    released,
                    retained,
                    release_capability_id,
                } => (released, retained, true, release_capability_id),
                RootGuardReleaseObservation::Unverified {
                    released,
                    retained,
                    recovery_capability_id,
                } => (released, retained, false, recovery_capability_id),
            };
        let exact_release = release_verified
            && post_release_retained.is_empty()
            && post_release_released.len() == 1
            && same_journaled_lock_identity(&post_release_released[0], &receipt);
        let RootGuardPreRereadLockObservation {
            lock_set_id,
            attempted_targets,
            journaled_acquired: acquired,
            released: mut pre_reread_released,
            retained: mut reported_retained,
        } = pre_reread;
        pre_reread_released.extend(post_release_released);
        reported_retained.extend(post_release_retained);
        let verified_stale = protocol_valid && !semantics_unchanged && exact_release;
        let possibly_retained = if verified_stale {
            Vec::new()
        } else {
            stable_identity_deduped_receipts(
                Some(&receipt),
                &[
                    acquired.as_slice(),
                    pre_reread_released.as_slice(),
                    reported_retained.as_slice(),
                ],
            )
        };
        let evidence = RepositoryLockAttemptEvidence {
            lock_set_id: Some(lock_set_id),
            attempted_targets,
            acquired,
            failed_target: None,
            released: pre_reread_released,
            reported_retained,
            retained: possibly_retained,
        };
        let context = Self::context(plan, current_gate);
        let kind = if verified_stale {
            RootGuardAcquisitionBlockedKind::VerifiedStale(VerifiedRootGateDriftStopAuthority {
                context,
                evidence,
                release_capability_id: capability_id,
            })
        } else {
            RootGuardAcquisitionBlockedKind::Recovery(RootLockRecoveryAuthority {
                context,
                evidence,
                recovery_capability_id: capability_id,
            })
        };
        Self {
            kind: Box::new(kind),
        }
    }

    fn from_first_root_conflict(
        plan: LockPlanData,
        current_gate: CurrentReadySupportGateAuthority,
        observation: RootFirstLockConflictObservation,
    ) -> Self {
        let exact_root = RepositoryTargetIdentity::configuration_root();
        let structurally_valid = observation.failed_target == exact_root
            && observation.attempted_targets == [exact_root]
            && observation.acquired.is_empty();
        let lock_set_id = observation
            .acquired
            .first()
            .map(|receipt| receipt.lock_set_id.clone());
        let possibly_retained =
            stable_identity_deduped_receipts(None, &[observation.acquired.as_slice()]);
        let evidence = RepositoryLockAttemptEvidence {
            lock_set_id,
            attempted_targets: observation.attempted_targets,
            acquired: observation.acquired,
            failed_target: Some(observation.failed_target),
            released: Vec::new(),
            reported_retained: Vec::new(),
            retained: possibly_retained,
        };
        let context = Self::context(plan, current_gate);
        let kind = if structurally_valid {
            RootGuardAcquisitionBlockedKind::FirstRootConflict(RootLockConflictStopAuthority {
                context,
                evidence,
                conflict_capability_id: observation.conflict_capability_id,
            })
        } else {
            RootGuardAcquisitionBlockedKind::Recovery(RootLockRecoveryAuthority {
                context,
                evidence,
                recovery_capability_id: observation.conflict_capability_id,
            })
        };
        Self {
            kind: Box::new(kind),
        }
    }

    fn from_root_recovery(
        plan: LockPlanData,
        current_gate: CurrentReadySupportGateAuthority,
        observation: RepositoryLockRecoveryObservation,
    ) -> Self {
        let RepositoryLockRecoveryObservation {
            lock_set_id,
            attempted_targets,
            acquired,
            failed_target,
            released,
            retained: reported_retained,
            recovery_capability_id,
        } = observation;
        let possibly_retained = stable_identity_deduped_receipts(
            None,
            &[
                acquired.as_slice(),
                released.as_slice(),
                reported_retained.as_slice(),
            ],
        );
        Self {
            kind: Box::new(RootGuardAcquisitionBlockedKind::Recovery(
                RootLockRecoveryAuthority {
                    context: Self::context(plan, current_gate),
                    evidence: RepositoryLockAttemptEvidence {
                        lock_set_id,
                        attempted_targets,
                        acquired,
                        failed_target,
                        released,
                        reported_retained,
                        retained: possibly_retained,
                    },
                    recovery_capability_id,
                },
            )),
        }
    }

    pub(crate) fn release_verified(&self) -> bool {
        matches!(
            self.kind.as_ref(),
            RootGuardAcquisitionBlockedKind::VerifiedStale(_)
        )
    }

    pub(crate) fn is_recovery_only(&self) -> bool {
        matches!(
            self.kind.as_ref(),
            RootGuardAcquisitionBlockedKind::Recovery(_)
        )
    }

    pub(crate) fn into_verified_stale_stop(
        self,
    ) -> Result<VerifiedRootGateDriftStopAuthority, Self> {
        match *self.kind {
            RootGuardAcquisitionBlockedKind::VerifiedStale(value) => Ok(value),
            kind => Err(Self {
                kind: Box::new(kind),
            }),
        }
    }

    pub(crate) fn into_root_conflict_stop(self) -> Result<RootLockConflictStopAuthority, Self> {
        match *self.kind {
            RootGuardAcquisitionBlockedKind::FirstRootConflict(value) => Ok(value),
            kind => Err(Self {
                kind: Box::new(kind),
            }),
        }
    }

    pub(crate) fn into_recovery(self) -> Result<RootLockRecoveryAuthority, Self> {
        match *self.kind {
            RootGuardAcquisitionBlockedKind::Recovery(value) => Ok(value),
            kind => Err(Self {
                kind: Box::new(kind),
            }),
        }
    }
}

impl RootLockConflictStopAuthority {
    pub(crate) fn failed_target(&self) -> &RepositoryTargetIdentity {
        self.evidence
            .failed_target
            .as_ref()
            .expect("root conflict always carries the failed target")
    }

    pub(crate) fn acquired_lock_count(&self) -> usize {
        self.evidence.acquired.len()
    }
}

impl RootLockRecoveryAuthority {
    pub(crate) fn attempted_target_count(&self) -> usize {
        self.evidence.attempted_targets.len()
    }

    pub(crate) fn acquired_lock_count(&self) -> usize {
        self.evidence.acquired.len()
    }

    pub(crate) fn retained_lock_count(&self) -> usize {
        self.evidence.retained.len()
    }
}

/// Successful under-root reread. This linear authority is the only production
/// input accepted by remaining-lock acquisition.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct RootGuardedCurrentGateAuthority {
    plan: LockPlanData,
    current_gate: CurrentReadySupportGateAuthority,
    root_lock_receipt: JournaledRepositoryLock,
    root_reread_capability_id: CapabilityRowId,
}

impl RootGuardedCurrentGateAuthority {
    pub(crate) fn plan(&self) -> &LockPlanData {
        &self.plan
    }

    pub(crate) fn current_gate(&self) -> &CurrentReadySupportGateAuthority {
        &self.current_gate
    }

    pub(crate) fn root_lock_receipt(&self) -> &JournaledRepositoryLock {
        &self.root_lock_receipt
    }
}

/// Raw adapter evidence for a completed root-first acquisition. The core still
/// verifies exact order, target identity, lock-set identity, and the original
/// root receipt before this can become a lock-set authority. `Journaled` is a
/// trusted-port observation type, not a claim accepted from an untrusted wire
/// caller; its exact per-lock sequence replaces a redundant aggregate boolean.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct RemainingLockSuccessObservation {
    lock_set_id: UnicaId,
    attempted_targets: Vec<RepositoryTargetIdentity>,
    journaled_receipts: Vec<JournaledRepositoryLock>,
}

impl RemainingLockSuccessObservation {
    pub(crate) fn from_repository_adapter(
        lock_set_id: UnicaId,
        attempted_targets: Vec<RepositoryTargetIdentity>,
        journaled_receipts: Vec<JournaledRepositoryLock>,
    ) -> Self {
        Self {
            lock_set_id,
            attempted_targets,
            journaled_receipts,
        }
    }
}

/// Named adapter input keeps the conflict observation auditable without a
/// positional constructor whose arguments can be silently transposed.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct RemainingLockConflictObservationInput {
    pub(crate) lock_set_id: UnicaId,
    pub(crate) attempted_targets: Vec<RepositoryTargetIdentity>,
    pub(crate) acquired: Vec<JournaledRepositoryLock>,
    pub(crate) failed_target: RepositoryTargetIdentity,
    pub(crate) released: Vec<JournaledRepositoryLock>,
    pub(crate) retained: Vec<JournaledRepositoryLock>,
    pub(crate) compensation_verified: bool,
    pub(crate) conflict_capability_id: CapabilityRowId,
    pub(crate) compensation_capability_id: CapabilityRowId,
}

/// Raw adapter evidence for a conflict after the root lock was already held.
/// It is not a stop authority until the core proves exact-prefix acquisition
/// and exact reverse-order compensation.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct RemainingLockConflictObservation {
    lock_set_id: UnicaId,
    attempted_targets: Vec<RepositoryTargetIdentity>,
    acquired: Vec<JournaledRepositoryLock>,
    failed_target: RepositoryTargetIdentity,
    released: Vec<JournaledRepositoryLock>,
    retained: Vec<JournaledRepositoryLock>,
    compensation_verified: bool,
    conflict_capability_id: CapabilityRowId,
    compensation_capability_id: CapabilityRowId,
}

impl RemainingLockConflictObservation {
    pub(crate) fn from_repository_adapter(input: RemainingLockConflictObservationInput) -> Self {
        Self {
            lock_set_id: input.lock_set_id,
            attempted_targets: input.attempted_targets,
            acquired: input.acquired,
            failed_target: input.failed_target,
            released: input.released,
            retained: input.retained,
            compensation_verified: input.compensation_verified,
            conflict_capability_id: input.conflict_capability_id,
            compensation_capability_id: input.compensation_capability_id,
        }
    }
}

/// Every adapter result is sealed and consumed once. The caller cannot turn a
/// malformed success or an unknown effect into a lock authority.
pub(crate) enum RemainingLockAcquisitionPortOutcome {
    Success(RemainingLockSuccessObservation),
    Conflict(Box<RemainingLockConflictObservation>),
    Recovery(Box<RepositoryLockRecoveryObservation>),
}

pub(crate) trait RemainingLockAcquisitionPort {
    fn acquire_remaining_locks(
        &mut self,
        root_guarded: &RootGuardedCurrentGateAuthority,
    ) -> RemainingLockAcquisitionPortOutcome;
}

#[derive(Debug, PartialEq, Eq)]
struct RemainingLockAttemptContext {
    root_guarded: RootGuardedCurrentGateAuthority,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct RemainingLockConflictStopAuthority {
    context: Box<RemainingLockAttemptContext>,
    evidence: RepositoryLockAttemptEvidence,
    conflict_capability_id: CapabilityRowId,
    compensation_capability_id: CapabilityRowId,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct RemainingLockRecoveryAuthority {
    context: Box<RemainingLockAttemptContext>,
    evidence: RepositoryLockAttemptEvidence,
    evidence_capability_ids: Vec<CapabilityRowId>,
}

#[derive(Debug, PartialEq, Eq)]
enum RemainingLockAcquisitionBlockedKind {
    Conflict(RemainingLockConflictStopAuthority),
    Recovery(RemainingLockRecoveryAuthority),
}

/// Typed consuming failure for the remaining-lock operation. It retains the
/// root-held context, so no malformed adapter response can drop an acquired
/// root receipt through a scalar contract error.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct RemainingLockAcquisitionBlockedAuthority {
    kind: Box<RemainingLockAcquisitionBlockedKind>,
}

impl RemainingLockAcquisitionBlockedAuthority {
    fn recovery(
        root_guarded: RootGuardedCurrentGateAuthority,
        evidence: RepositoryLockAttemptEvidence,
        evidence_capability_ids: Vec<CapabilityRowId>,
    ) -> Self {
        Self {
            kind: Box::new(RemainingLockAcquisitionBlockedKind::Recovery(
                RemainingLockRecoveryAuthority {
                    context: Box::new(RemainingLockAttemptContext { root_guarded }),
                    evidence,
                    evidence_capability_ids,
                },
            )),
        }
    }

    fn from_success_protocol_breach(
        root_guarded: RootGuardedCurrentGateAuthority,
        observation: RemainingLockSuccessObservation,
    ) -> Self {
        let evidence_capability_id = root_guarded.root_reread_capability_id.clone();
        let retained = stable_identity_deduped_receipts(
            Some(&root_guarded.root_lock_receipt),
            &[observation.journaled_receipts.as_slice()],
        );
        let evidence = RepositoryLockAttemptEvidence {
            lock_set_id: Some(observation.lock_set_id),
            attempted_targets: observation.attempted_targets,
            acquired: observation.journaled_receipts,
            failed_target: None,
            released: Vec::new(),
            reported_retained: Vec::new(),
            retained,
        };
        Self::recovery(root_guarded, evidence, vec![evidence_capability_id])
    }

    fn from_conflict(
        root_guarded: RootGuardedCurrentGateAuthority,
        observation: RemainingLockConflictObservation,
        conflict_verified: bool,
    ) -> Self {
        let RemainingLockConflictObservation {
            lock_set_id,
            attempted_targets,
            acquired,
            failed_target,
            released,
            retained: reported_retained,
            compensation_verified: _,
            conflict_capability_id,
            compensation_capability_id,
        } = observation;
        let retained = if conflict_verified {
            Vec::new()
        } else {
            stable_identity_deduped_receipts(
                Some(&root_guarded.root_lock_receipt),
                &[
                    acquired.as_slice(),
                    released.as_slice(),
                    reported_retained.as_slice(),
                ],
            )
        };
        let evidence = RepositoryLockAttemptEvidence {
            lock_set_id: Some(lock_set_id),
            attempted_targets,
            acquired,
            failed_target: Some(failed_target),
            released,
            reported_retained,
            retained,
        };
        if conflict_verified {
            Self {
                kind: Box::new(RemainingLockAcquisitionBlockedKind::Conflict(
                    RemainingLockConflictStopAuthority {
                        context: Box::new(RemainingLockAttemptContext { root_guarded }),
                        evidence,
                        conflict_capability_id,
                        compensation_capability_id,
                    },
                )),
            }
        } else {
            Self::recovery(
                root_guarded,
                evidence,
                vec![conflict_capability_id, compensation_capability_id],
            )
        }
    }

    fn from_recovery_observation(
        root_guarded: RootGuardedCurrentGateAuthority,
        observation: RepositoryLockRecoveryObservation,
    ) -> Self {
        let RepositoryLockRecoveryObservation {
            lock_set_id,
            attempted_targets,
            acquired,
            failed_target,
            released,
            retained: reported_retained,
            recovery_capability_id,
        } = observation;
        let retained = stable_identity_deduped_receipts(
            Some(&root_guarded.root_lock_receipt),
            &[
                acquired.as_slice(),
                released.as_slice(),
                reported_retained.as_slice(),
            ],
        );
        let evidence = RepositoryLockAttemptEvidence {
            lock_set_id,
            attempted_targets,
            acquired,
            failed_target,
            released,
            reported_retained,
            retained,
        };
        Self::recovery(root_guarded, evidence, vec![recovery_capability_id])
    }

    pub(crate) fn is_recovery_only(&self) -> bool {
        matches!(
            self.kind.as_ref(),
            RemainingLockAcquisitionBlockedKind::Recovery(_)
        )
    }

    pub(crate) fn into_verified_conflict_stop(
        self,
    ) -> Result<RemainingLockConflictStopAuthority, Self> {
        match *self.kind {
            RemainingLockAcquisitionBlockedKind::Conflict(value) => Ok(value),
            kind => Err(Self {
                kind: Box::new(kind),
            }),
        }
    }

    pub(crate) fn into_recovery(self) -> Result<RemainingLockRecoveryAuthority, Self> {
        match *self.kind {
            RemainingLockAcquisitionBlockedKind::Recovery(value) => Ok(value),
            kind => Err(Self {
                kind: Box::new(kind),
            }),
        }
    }
}

impl RemainingLockConflictStopAuthority {
    pub(crate) fn failed_target(&self) -> &RepositoryTargetIdentity {
        self.evidence
            .failed_target
            .as_ref()
            .expect("verified conflict always carries the failed target")
    }

    pub(crate) fn acquired_lock_count(&self) -> usize {
        self.evidence.acquired.len()
    }

    pub(crate) fn released_lock_count(&self) -> usize {
        self.evidence.released.len()
    }

    pub(crate) fn retained_lock_count(&self) -> usize {
        self.evidence.retained.len()
    }
}

impl RemainingLockRecoveryAuthority {
    pub(crate) fn acquired_lock_count(&self) -> usize {
        self.evidence.acquired.len()
    }

    pub(crate) fn released_lock_count(&self) -> usize {
        self.evidence.released.len()
    }

    pub(crate) fn retained_lock_count(&self) -> usize {
        self.evidence.retained.len()
    }
}

/// Linear lock/plan lineage consumed by the original-configuration merge
/// executor.  It intentionally carries no wire representation and cannot be
/// cloned, so merge apply cannot be assembled from caller-selected digests.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ValidatedOriginalMergeLockProjection {
    plan: LockPlanData,
    lock_set_id: UnicaId,
    lock_set_digest: Sha256Digest,
    gate_proof: ValidatedLockGateProof,
}

impl ValidatedOriginalMergeLockProjection {
    #[cfg(test)]
    pub(crate) fn test_only(
        plan: LockPlanData,
        lock_set_id: UnicaId,
        lock_set_digest: Sha256Digest,
    ) -> Self {
        Self {
            plan,
            lock_set_id,
            lock_set_digest,
            gate_proof: ValidatedLockGateProof::Fixture,
        }
    }

    pub(crate) fn plan(&self) -> &LockPlanData {
        &self.plan
    }

    pub(crate) const fn plan_id(&self) -> &UnicaId {
        &self.plan.plan_id
    }

    pub(crate) const fn plan_digest(&self) -> &Sha256Digest {
        &self.plan.plan_digest
    }

    pub(crate) const fn merge_session_id(&self) -> &UnicaId {
        &self.plan.merge_session_id
    }

    pub(crate) const fn resolved_session_digest(&self) -> &Sha256Digest {
        &self.plan.resolved_session_digest
    }

    pub(crate) const fn support_gate_id(&self) -> &UnicaId {
        &self.plan.support_gate_id
    }

    pub(crate) const fn support_gate_digest(&self) -> &Sha256Digest {
        &self.plan.support_gate_digest
    }

    pub(crate) const fn support_gate_history_evidence(&self) -> &SupportGateHistoryEvidence {
        &self.plan.support_gate_history_evidence
    }

    pub(crate) const fn integration_set_id(&self) -> &UnicaId {
        &self.plan.integration_set_id
    }

    pub(crate) const fn integration_set_digest(&self) -> &Sha256Digest {
        &self.plan.integration_set_digest
    }

    pub(crate) const fn lock_set_id(&self) -> &UnicaId {
        &self.lock_set_id
    }

    pub(crate) const fn lock_set_digest(&self) -> &Sha256Digest {
        &self.lock_set_digest
    }

    #[cfg(test)]
    pub(crate) fn into_parts(self) -> (LockPlanData, UnicaId, Sha256Digest) {
        (self.plan, self.lock_set_id, self.lock_set_digest)
    }

    pub(crate) fn current_gate_authority(&self) -> Option<&CurrentReadySupportGateAuthority> {
        match &self.gate_proof {
            ValidatedLockGateProof::Production { current_gate, .. } => Some(current_gate),
            #[cfg(test)]
            ValidatedLockGateProof::Fixture => None,
        }
    }

    pub(crate) fn root_lock_receipt(&self) -> Option<&JournaledRepositoryLock> {
        match &self.gate_proof {
            ValidatedLockGateProof::Production {
                root_lock_receipt, ..
            } => Some(root_lock_receipt),
            #[cfg(test)]
            ValidatedLockGateProof::Fixture => None,
        }
    }

    pub(crate) fn journaled_lock_receipts(&self) -> Option<&[JournaledRepositoryLock]> {
        match &self.gate_proof {
            ValidatedLockGateProof::Production {
                journaled_lock_receipts,
                ..
            } => Some(journaled_lock_receipts),
            #[cfg(test)]
            ValidatedLockGateProof::Fixture => None,
        }
    }

    pub(crate) fn root_reread_capability_id(&self) -> Option<&CapabilityRowId> {
        match &self.gate_proof {
            ValidatedLockGateProof::Production {
                root_reread_capability_id,
                ..
            } => Some(root_reread_capability_id),
            #[cfg(test)]
            ValidatedLockGateProof::Fixture => None,
        }
    }
}

fn remaining_success_matches_exact_plan(
    root_guarded: &RootGuardedCurrentGateAuthority,
    observation: &RemainingLockSuccessObservation,
) -> bool {
    let expected = planned_lock_target_identities(&root_guarded.plan);
    observation.lock_set_id == root_guarded.root_lock_receipt.lock_set_id
        && observation.attempted_targets == expected
        && observation.journaled_receipts.len() == expected.len()
        && observation
            .journaled_receipts
            .iter()
            .zip(expected.iter())
            .all(|(receipt, target)| {
                receipt_binds_target_and_lock_set(receipt, target, &observation.lock_set_id)
            })
        && observation
            .journaled_receipts
            .first()
            .is_some_and(|receipt| {
                same_journaled_lock_identity(receipt, &root_guarded.root_lock_receipt)
            })
}

fn remaining_conflict_is_exact_and_compensated(
    root_guarded: &RootGuardedCurrentGateAuthority,
    observation: &RemainingLockConflictObservation,
) -> bool {
    let expected = planned_lock_target_identities(&root_guarded.plan);
    let acquired_len = observation.acquired.len();
    if acquired_len == 0 || acquired_len >= expected.len() {
        return false;
    }
    observation.lock_set_id == root_guarded.root_lock_receipt.lock_set_id
        && observation.attempted_targets == expected[..=acquired_len]
        && observation.failed_target == expected[acquired_len]
        && observation
            .acquired
            .iter()
            .zip(expected[..acquired_len].iter())
            .all(|(receipt, target)| {
                receipt_binds_target_and_lock_set(receipt, target, &observation.lock_set_id)
            })
        && observation.acquired.first().is_some_and(|receipt| {
            same_journaled_lock_identity(receipt, &root_guarded.root_lock_receipt)
        })
        && observation.compensation_verified
        && observation.retained.is_empty()
        && observation.released.len() == acquired_len
        && observation
            .released
            .iter()
            .zip(observation.acquired.iter().rev())
            .all(|(released, acquired)| same_journaled_lock_identity(released, acquired))
}

impl ValidatedLockSetAuthority {
    #[cfg(test)]
    pub(crate) fn from_plan(
        plan: LockPlanData,
        observation: LockAcquisitionObservationAuthority,
    ) -> Result<Self, RepositoryResultContractError> {
        if observation.acquired != plan.lock_entries {
            return Err(RepositoryResultContractError(
                "acquired lock set is not the complete approved plan",
            ));
        }
        let lock_set_digest = result_digest(
            &LockSetDigestRecord {
                plan_digest: plan.plan_digest.clone(),
                integration_set_id: plan.integration_set_id.clone(),
                integration_set_digest: plan.integration_set_digest.clone(),
                acquired: observation.acquired,
            },
            "lock-set digest failed",
        )?;
        Ok(Self {
            plan,
            lock_set_id: observation.lock_set_id,
            lock_set_digest,
            gate_proof: ValidatedLockGateProof::Fixture,
        })
    }

    pub(crate) fn from_root_guarded_acquisition(
        root_guarded: RootGuardedCurrentGateAuthority,
        port: &mut dyn RemainingLockAcquisitionPort,
    ) -> Result<Self, RemainingLockAcquisitionBlockedAuthority> {
        match port.acquire_remaining_locks(&root_guarded) {
            RemainingLockAcquisitionPortOutcome::Success(observation) => {
                if !remaining_success_matches_exact_plan(&root_guarded, &observation) {
                    return Err(
                        RemainingLockAcquisitionBlockedAuthority::from_success_protocol_breach(
                            root_guarded,
                            observation,
                        ),
                    );
                }
                let lock_set_digest = match result_digest(
                    &LockSetDigestRecord {
                        plan_digest: root_guarded.plan.plan_digest.clone(),
                        integration_set_id: root_guarded.plan.integration_set_id.clone(),
                        integration_set_digest: root_guarded.plan.integration_set_digest.clone(),
                        acquired: root_guarded.plan.lock_entries.clone(),
                    },
                    "lock-set digest failed",
                ) {
                    Ok(value) => value,
                    Err(_) => {
                        return Err(
                            RemainingLockAcquisitionBlockedAuthority::from_success_protocol_breach(
                                root_guarded,
                                observation,
                            ),
                        );
                    }
                };
                Ok(Self {
                    plan: root_guarded.plan,
                    lock_set_id: observation.lock_set_id,
                    lock_set_digest,
                    gate_proof: ValidatedLockGateProof::Production {
                        current_gate: Box::new(root_guarded.current_gate),
                        root_lock_receipt: root_guarded.root_lock_receipt,
                        journaled_lock_receipts: observation.journaled_receipts,
                        root_reread_capability_id: root_guarded.root_reread_capability_id,
                    },
                })
            }
            RemainingLockAcquisitionPortOutcome::Conflict(observation) => {
                let conflict_verified =
                    remaining_conflict_is_exact_and_compensated(&root_guarded, &observation);
                Err(RemainingLockAcquisitionBlockedAuthority::from_conflict(
                    root_guarded,
                    *observation,
                    conflict_verified,
                ))
            }
            RemainingLockAcquisitionPortOutcome::Recovery(observation) => Err(
                RemainingLockAcquisitionBlockedAuthority::from_recovery_observation(
                    root_guarded,
                    *observation,
                ),
            ),
        }
    }

    pub(crate) fn data(&self) -> LockResultData {
        LockResultData {
            plan_id: self.plan.plan_id.clone(),
            plan_digest: self.plan.plan_digest.clone(),
            integration_set_id: self.plan.integration_set_id.clone(),
            integration_set_digest: self.plan.integration_set_digest.clone(),
            lock_set_id: self.lock_set_id.clone(),
            acquired: self.plan.lock_entries.clone(),
            support_gate_id: self.plan.support_gate_id.clone(),
            support_gate_digest: self.plan.support_gate_digest.clone(),
            support_gate_history_evidence: self.plan.support_gate_history_evidence.clone(),
            relevant_anchors: self.plan.relevant_anchors.clone(),
            lock_set_digest: self.lock_set_digest.clone(),
        }
    }

    pub(crate) fn plan(&self) -> &LockPlanData {
        &self.plan
    }

    pub(crate) fn lock_set_digest(&self) -> &Sha256Digest {
        &self.lock_set_digest
    }

    pub(crate) fn into_original_merge_projection(self) -> ValidatedOriginalMergeLockProjection {
        ValidatedOriginalMergeLockProjection {
            plan: self.plan,
            lock_set_id: self.lock_set_id,
            lock_set_digest: self.lock_set_digest,
            gate_proof: self.gate_proof,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct LockResultData {
    plan_id: UnicaId,
    plan_digest: Sha256Digest,
    integration_set_id: UnicaId,
    integration_set_digest: Sha256Digest,
    lock_set_id: UnicaId,
    acquired: RepositoryUpdateLockTargets,
    support_gate_id: UnicaId,
    support_gate_digest: Sha256Digest,
    support_gate_history_evidence: SupportGateHistoryEvidence,
    relevant_anchors: RepositoryRelevantAnchors,
    lock_set_digest: Sha256Digest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) enum UnlockReason {
    Compensation,
    Rollback,
    Abandonment,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct UnlockCompletionObservationAuthority {
    released: CanonicalRepositoryTargets,
    retained: CanonicalRepositoryTargets,
    original_restored: bool,
    unlock_receipt_id: UnicaId,
}

impl UnlockCompletionObservationAuthority {
    pub(crate) fn from_repository_adapter(
        released: CanonicalRepositoryTargets,
        retained: CanonicalRepositoryTargets,
        original_restored: bool,
        unlock_receipt_id: UnicaId,
    ) -> Self {
        Self {
            released,
            retained,
            original_restored,
            unlock_receipt_id,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct UnlockData {
    released: CanonicalRepositoryTargets,
    retained: CanonicalRepositoryTargets,
    release_verified: TrueLiteral,
    original_restored: bool,
    unlock_receipt_id: UnicaId,
}

impl UnlockData {
    pub(crate) fn from_owned_lock_set(
        lock_set: ValidatedLockSetAuthority,
        _reason: UnlockReason,
        observation: UnlockCompletionObservationAuthority,
    ) -> Result<Self, RepositoryResultContractError> {
        let expected = project_lock_targets(&lock_set.plan.lock_entries)?;
        if observation.released != expected || !observation.retained.0.is_empty() {
            return Err(RepositoryResultContractError(
                "completed unlock did not release the exact complete owned lock set",
            ));
        }
        Ok(Self {
            released: observation.released,
            retained: observation.retained,
            release_verified: TrueLiteral,
            original_restored: observation.original_restored,
            unlock_receipt_id: observation.unlock_receipt_id,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
struct CommitExactObjectsDigestRecord(CommitExactObjects);

impl contract_digest_record_sealed::Sealed for CommitExactObjectsDigestRecord {}
impl ContractDigestRecord for CommitExactObjectsDigestRecord {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CommitPreviewDigestRecord {
    exact_objects: CommitExactObjects,
    guard_locks: CanonicalRepositoryTargets,
    comment: Comment,
    integration_set_digest: Sha256Digest,
    exact_objects_digest: Sha256Digest,
    verification_digest: Sha256Digest,
    lock_set_digest: Sha256Digest,
    merge_receipt_id: UnicaId,
    support_gate_id: UnicaId,
    consumed_support_gate_digest: Sha256Digest,
    support_gate_history_evidence_digest: Sha256Digest,
    authorized_post_merge_fingerprint: Sha256Digest,
    observed_original_fingerprint: Sha256Digest,
    history_guard_evidence: PostMergeHistoryGuardEvidence,
}

impl contract_digest_record_sealed::Sealed for CommitPreviewDigestRecord {}
impl ContractDigestRecord for CommitPreviewDigestRecord {}

#[derive(Debug)]
struct CommitSafetyLineageMarker;

/// Opaque, owning proof of one exact commit-safety lineage marker. Adapters can
/// only obtain it from a scoped request and return it through the completed
/// lease; it exposes no scalar identity or control operation.
#[derive(Debug)]
pub(crate) struct CommitSafetyLineageWitness(Arc<CommitSafetyLineageMarker>);

/// Private pointer-identity binding which nests the consumed-gate lineage under
/// every later commit authority. The source itself is never cloned or Arc-wrapped.
#[derive(Debug)]
pub(crate) struct CommitSafetyLineageBinding {
    marker: Arc<CommitSafetyLineageMarker>,
    source: ResolvedCommitLineageConsumedSupportGateAuthority,
}

impl CommitSafetyLineageBinding {
    fn new(source: ResolvedCommitLineageConsumedSupportGateAuthority) -> Self {
        Self {
            marker: Arc::new(CommitSafetyLineageMarker),
            source,
        }
    }

    fn into_source(self) -> ResolvedCommitLineageConsumedSupportGateAuthority {
        self.source
    }

    fn witness(&self) -> CommitSafetyLineageWitness {
        CommitSafetyLineageWitness(Arc::clone(&self.marker))
    }

    fn owns_witness(&self, witness: &CommitSafetyLineageWitness) -> bool {
        Arc::ptr_eq(&self.marker, &witness.0)
    }
}

impl std::ops::Deref for CommitSafetyLineageBinding {
    type Target = ResolvedCommitLineageConsumedSupportGateAuthority;

    fn deref(&self) -> &Self::Target {
        &self.source
    }
}

impl PartialEq for CommitSafetyLineageBinding {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.marker, &other.marker) && self.source == other.source
    }
}

impl Eq for CommitSafetyLineageBinding {}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct PostMergeCommitGuardObservedFailureEvidence {
    history_guard_evidence: PostMergeHistoryGuardEvidence,
    observed_original_fingerprint: Sha256Digest,
    observed_repository_anchor: RepositoryAnchor,
    original_fingerprint_capability_id: CapabilityRowId,
}

impl PostMergeCommitGuardObservedFailureEvidence {
    fn from_lease(lease: &dyn PostMergeCommitGuardLease) -> Self {
        Self {
            history_guard_evidence: lease.history_guard_evidence().clone(),
            observed_original_fingerprint: lease.observed_original_fingerprint().clone(),
            observed_repository_anchor: lease.observed_repository_anchor().clone(),
            original_fingerprint_capability_id: lease.original_fingerprint_capability_id().clone(),
        }
    }
}

#[derive(Debug)]
pub(crate) enum PostMergeCommitGuardFailureEvidence {
    PortError(RepositoryResultContractError),
    CompletionAttemptMismatch {
        completion: PostMergeCommitGuardCompletion,
    },
    CapabilityBindingMismatch {
        completion: PostMergeCommitGuardCompletion,
    },
    PostMergeDrift {
        completion: PostMergeCommitGuardCompletion,
        evidence: Box<PostMergeCommitGuardObservedFailureEvidence>,
    },
    UnscopedNonConflictingConcurrent {
        completion: PostMergeCommitGuardCompletion,
        evidence: Box<PostMergeCommitGuardObservedFailureEvidence>,
    },
}

/// Recovery-only result for every post-merge guard failure. Neither the
/// verified consumed lineage nor a completed live lease is lost.
#[derive(Debug)]
pub(crate) struct PostMergeCommitGuardBlockedAuthority {
    source: CommitSafetyLineageBinding,
    failure: PostMergeCommitGuardFailureEvidence,
}

impl PostMergeCommitGuardBlockedAuthority {
    fn new(
        source: CommitSafetyLineageBinding,
        failure: PostMergeCommitGuardFailureEvidence,
    ) -> Box<Self> {
        Box::new(Self { source, failure })
    }

    pub(crate) fn failure(&self) -> &PostMergeCommitGuardFailureEvidence {
        &self.failure
    }

    pub(crate) fn into_recovery_parts(
        self: Box<Self>,
    ) -> (
        ResolvedCommitLineageConsumedSupportGateAuthority,
        PostMergeCommitGuardFailureEvidence,
    ) {
        let Self { source, failure } = *self;
        (source.into_source(), failure)
    }
}

/// Atomic post-merge observation retained until commit preview construction.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct PostMergeCommitGuardAuthority {
    source: CommitSafetyLineageBinding,
    history_guard_evidence: PostMergeHistoryGuardEvidence,
    observed_original_fingerprint: Sha256Digest,
    post_merge_repository_anchor: RepositoryAnchor,
    original_fingerprint_capability_id: CapabilityRowId,
}

#[cfg(test)]
#[derive(Debug, PartialEq, Eq)]
struct PostMergeCommitGuardFixtureAuthority {
    history_guard_evidence: PostMergeHistoryGuardEvidence,
    observed_original_fingerprint: Sha256Digest,
    original_fingerprint_capability_id: CapabilityRowId,
}

#[cfg(test)]
impl PostMergeCommitGuardFixtureAuthority {
    fn from_capability_adapter(
        history_guard_evidence: PostMergeHistoryGuardEvidence,
        observed_original_fingerprint: Sha256Digest,
        original_fingerprint_capability_id: CapabilityRowId,
    ) -> Self {
        Self {
            history_guard_evidence,
            observed_original_fingerprint,
            original_fingerprint_capability_id,
        }
    }
}

impl PostMergeCommitGuardAuthority {
    pub(crate) fn from_authoritative_consumed_lineage(
        source: ResolvedCommitLineageConsumedSupportGateAuthority,
        port: &mut dyn PostMergeCommitGuardPort,
    ) -> Result<Self, Box<PostMergeCommitGuardBlockedAuthority>> {
        let source = CommitSafetyLineageBinding::new(source);
        let invocation = PostMergeCommitGuardInvocationCapability::mint();
        let request = PostMergeCommitGuardRequest {
            source: &source,
            invocation: &invocation,
        };
        let completion = match port.observe_post_merge_commit_guard(request) {
            Ok(completion) => completion,
            Err(error) => {
                return Err(PostMergeCommitGuardBlockedAuthority::new(
                    source,
                    PostMergeCommitGuardFailureEvidence::PortError(error),
                ));
            }
        };
        if !invocation.owns_completion(&completion.completion) {
            return Err(PostMergeCommitGuardBlockedAuthority::new(
                source,
                PostMergeCommitGuardFailureEvidence::CompletionAttemptMismatch { completion },
            ));
        }
        if !source.owns_witness(completion.lease.commit_safety_lineage_witness()) {
            return Err(PostMergeCommitGuardBlockedAuthority::new(
                source,
                PostMergeCommitGuardFailureEvidence::CapabilityBindingMismatch { completion },
            ));
        }
        let request = PostMergeCommitGuardRequest {
            source: &source,
            invocation: &invocation,
        };
        if !completion.lease.binds(&request) {
            return Err(PostMergeCommitGuardBlockedAuthority::new(
                source,
                PostMergeCommitGuardFailureEvidence::CapabilityBindingMismatch { completion },
            ));
        }
        let observed_evidence =
            PostMergeCommitGuardObservedFailureEvidence::from_lease(&*completion.lease);
        let expected_root = source
            .lineage()
            .lock_plan()
            .relevant_anchors()
            .as_slice()
            .iter()
            .find(|anchor| anchor.target() == &RepositoryTargetIdentity::configuration_root())
            .map(RepositoryRelevantAnchor::anchor)
            .expect("commit lineage always retains the exact B1 root anchor");
        if observed_evidence
            .history_guard_evidence
            .merge_receipt_cursor()
            != source.lineage().merge_receipt_cursor()
            || observed_evidence
                .history_guard_evidence
                .recomputed_reference_closure_digest()
                != source.lineage().lock_plan().reference_closure_digest()
            || observed_evidence.observed_original_fingerprint
                != *source.lineage().result_fingerprint()
            || observed_evidence
                .observed_repository_anchor
                .repository_identity()
                != expected_root.repository_identity()
            || observed_evidence
                .observed_repository_anchor
                .configuration_identity()
                != expected_root.configuration_identity()
            || observed_evidence
                .observed_repository_anchor
                .history_cursor()
                != observed_evidence
                    .history_guard_evidence
                    .classified_through_cursor()
            || observed_evidence
                .observed_repository_anchor
                .configuration_fingerprint()
                != source.lineage().result_fingerprint()
        {
            return Err(PostMergeCommitGuardBlockedAuthority::new(
                source,
                PostMergeCommitGuardFailureEvidence::PostMergeDrift {
                    completion,
                    evidence: Box::new(observed_evidence),
                },
            ));
        }
        if observed_evidence
            .history_guard_evidence
            .partition()
            .classifications()
            .any(|classification| {
                classification == RepositoryHistoryPartitionClassification::NonConflictingConcurrent
            })
        {
            return Err(PostMergeCommitGuardBlockedAuthority::new(
                source,
                PostMergeCommitGuardFailureEvidence::UnscopedNonConflictingConcurrent {
                    completion,
                    evidence: Box::new(observed_evidence),
                },
            ));
        }
        let PostMergeCommitGuardObservedFailureEvidence {
            history_guard_evidence,
            observed_original_fingerprint,
            observed_repository_anchor,
            original_fingerprint_capability_id,
        } = observed_evidence;
        drop(completion);
        Ok(Self {
            source,
            history_guard_evidence,
            observed_original_fingerprint,
            post_merge_repository_anchor: observed_repository_anchor,
            original_fingerprint_capability_id,
        })
    }
}

/// Exact read-only scope given to the post-merge history/fingerprint adapter.
#[derive(Debug)]
pub(crate) struct PostMergeCommitGuardRequest<'a> {
    source: &'a CommitSafetyLineageBinding,
    invocation: &'a PostMergeCommitGuardInvocationCapability,
}

#[derive(Debug)]
struct PostMergeCommitGuardInvocationMarker;

#[derive(Debug)]
struct PostMergeCommitGuardInvocationCapability(Arc<PostMergeCommitGuardInvocationMarker>);

#[derive(Debug)]
struct PostMergeCommitGuardCompletionCapability(Arc<PostMergeCommitGuardInvocationMarker>);

impl PostMergeCommitGuardInvocationCapability {
    fn mint() -> Self {
        Self(Arc::new(PostMergeCommitGuardInvocationMarker))
    }

    fn completion(&self) -> PostMergeCommitGuardCompletionCapability {
        PostMergeCommitGuardCompletionCapability(Arc::clone(&self.0))
    }

    fn owns_completion(&self, completion: &PostMergeCommitGuardCompletionCapability) -> bool {
        Arc::ptr_eq(&self.0, &completion.0)
    }
}

impl PostMergeCommitGuardRequest<'_> {
    pub(crate) fn commit_safety_lineage_witness(&self) -> CommitSafetyLineageWitness {
        self.source.witness()
    }

    pub(crate) fn verification_id(&self) -> &UnicaId {
        self.source.lineage().verification_id()
    }

    pub(crate) fn verification_digest(&self) -> &Sha256Digest {
        self.source.lineage().verification_digest()
    }

    pub(crate) fn merge_receipt_id(&self) -> &UnicaId {
        self.source.lineage().merge_receipt_id()
    }

    pub(crate) fn session_id(&self) -> &UnicaId {
        self.source.lineage().session_id()
    }

    pub(crate) fn resolved_session_digest(&self) -> &Sha256Digest {
        self.source.lineage().resolved_session_digest()
    }

    pub(crate) fn merge_receipt_cursor(&self) -> &RepositoryHistoryCursor {
        self.source.lineage().merge_receipt_cursor()
    }

    pub(crate) fn authorized_result_fingerprint(&self) -> &Sha256Digest {
        self.source.lineage().result_fingerprint()
    }

    pub(crate) fn support_gate_id(&self) -> &UnicaId {
        self.source.lineage().support_gate_id()
    }

    pub(crate) fn support_gate_digest(&self) -> &Sha256Digest {
        self.source.lineage().support_gate_digest()
    }

    pub(crate) fn support_gate_history_evidence(&self) -> &SupportGateHistoryEvidence {
        self.source.lineage().support_gate_history_evidence()
    }

    pub(crate) fn plan_id(&self) -> &UnicaId {
        self.source.lineage().plan_id()
    }

    pub(crate) fn plan_digest(&self) -> &Sha256Digest {
        self.source.lineage().plan_digest()
    }

    pub(crate) fn integration_set_id(&self) -> &UnicaId {
        self.source.lineage().integration_set_id()
    }

    pub(crate) fn integration_set_digest(&self) -> &Sha256Digest {
        self.source.lineage().integration_set_digest()
    }

    pub(crate) fn lock_set_id(&self) -> &UnicaId {
        self.source.lineage().lock_set_id()
    }

    pub(crate) fn lock_set_digest(&self) -> &Sha256Digest {
        self.source.lineage().lock_set_digest()
    }

    pub(crate) fn reference_closure_digest(&self) -> &Sha256Digest {
        self.source.lineage().lock_plan().reference_closure_digest()
    }

    pub(crate) fn consumed_state_revision(&self) -> &Sha256Digest {
        self.source
            .consumed_gate_observation()
            .consumed_state_revision()
    }

    pub(crate) fn consumed_state_observation_capability_id(&self) -> &CapabilityRowId {
        self.source
            .consumed_gate_observation()
            .observation_capability_id()
    }

    pub(crate) fn root_reread_capability_id(&self) -> &CapabilityRowId {
        self.source.lineage().root_reread_capability_id()
    }

    pub(crate) fn lock_plan(&self) -> &LockPlanData {
        self.source.lineage().lock_plan()
    }

    pub(crate) fn rollback_checkpoint_id(&self) -> &UnicaId {
        self.source.lineage().rollback_checkpoint_id()
    }

    pub(crate) fn journaled_lock_receipts(&self) -> &[JournaledRepositoryLock] {
        self.source.lineage().journaled_lock_receipts()
    }

    pub(crate) fn expected_plan_root_anchor(&self) -> &RepositoryAnchor {
        self.source
            .lineage()
            .lock_plan()
            .relevant_anchors()
            .as_slice()
            .iter()
            .find(|anchor| anchor.target() == &RepositoryTargetIdentity::configuration_root())
            .map(RepositoryRelevantAnchor::anchor)
            .expect("commit lineage always retains the exact B1 root anchor")
    }

    pub(crate) fn observe_repository_anchor(
        &self,
        history_guard_evidence: &PostMergeHistoryGuardEvidence,
        repository_identity: Sha256Digest,
        configuration_identity: ConfigurationIdentity,
        configuration_fingerprint: Sha256Digest,
    ) -> Result<RepositoryAnchor, RepositoryResultContractError> {
        RepositoryAnchor::from_guarded_observation(
            repository_identity,
            history_guard_evidence.classified_through_cursor().clone(),
            configuration_identity,
            configuration_fingerprint,
        )
        .map_err(|_| RepositoryResultContractError("post-merge repository anchor digest failed"))
    }

    pub(crate) fn complete(
        self,
        lease: Box<dyn PostMergeCommitGuardLease>,
    ) -> PostMergeCommitGuardCompletion {
        PostMergeCommitGuardCompletion {
            completion: self.invocation.completion(),
            lease,
        }
    }
}

pub(crate) struct PostMergeCommitGuardCompletion {
    completion: PostMergeCommitGuardCompletionCapability,
    lease: Box<dyn PostMergeCommitGuardLease>,
}

impl fmt::Debug for PostMergeCommitGuardCompletion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PostMergeCommitGuardCompletion")
            .field("completion", &self.completion)
            .field("lease", &"<live post-merge guard lease>")
            .finish()
    }
}

pub(crate) trait PostMergeCommitGuardLease {
    fn commit_safety_lineage_witness(&self) -> &CommitSafetyLineageWitness;
    fn binds(&self, request: &PostMergeCommitGuardRequest<'_>) -> bool;
    fn history_guard_evidence(&self) -> &PostMergeHistoryGuardEvidence;
    fn observed_original_fingerprint(&self) -> &Sha256Digest;
    fn observed_repository_anchor(&self) -> &RepositoryAnchor;
    fn original_fingerprint_capability_id(&self) -> &CapabilityRowId;
}

pub(crate) trait PostMergeCommitGuardPort {
    fn observe_post_merge_commit_guard(
        &mut self,
        request: PostMergeCommitGuardRequest<'_>,
    ) -> Result<PostMergeCommitGuardCompletion, RepositoryResultContractError>;
}

fn derive_commit_guard_locks(
    plan: &LockPlanData,
) -> Result<CanonicalRepositoryTargets, RepositoryResultContractError> {
    let content_targets = plan
        .integration_entries
        .as_slice()
        .iter()
        .map(RepositoryIntegrationEntry::target_identity)
        .collect::<BTreeSet<_>>();
    let acquired = project_lock_targets(&plan.lock_entries)?;
    CanonicalRepositoryTargets::new(
        acquired
            .0
            .into_iter()
            .filter(|target| !content_targets.contains(target))
            .collect(),
    )
}

/// Typed preview producer retaining the exact lock plan whose identity/action
/// projection is approved. It is intentionally non-Clone and non-Deserialize.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct CommitPreviewAuthority {
    plan: LockPlanData,
    record: CommitPreviewDigestRecord,
    commit_digest: Sha256Digest,
    _validated_lineage: Option<CommitPreviewValidatedLineage>,
}

#[derive(Debug, PartialEq, Eq)]
struct CommitPreviewValidatedLineage {
    preview_request: ValidatedRepositoryCommitPreviewRequest,
    verification_id: UnicaId,
    lock_set_id: UnicaId,
    comment_policy: ValidatedCommitCommentPolicyAuthority,
    original_fingerprint_capability_id: CapabilityRowId,
    post_merge_guard: Box<PostMergeCommitGuardAuthority>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CommitPreviewFailureEvidence {
    RequestLineageMismatch,
    InvalidExactObjects(RepositoryResultContractError),
    ExactObjectsDigest(RepositoryResultContractError),
    GuardLocks(RepositoryResultContractError),
    CommitDigest(RepositoryResultContractError),
}

/// Recovery-only result for preview derivation failures. The exact post-merge
/// guard and validated comment policy remain linear and owned instead of being
/// erased behind a scalar digest/projection error.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct CommitPreviewBlockedAuthority {
    request: ValidatedRepositoryCommitPreviewRequest,
    guard: PostMergeCommitGuardAuthority,
    comment_policy: ValidatedCommitCommentPolicyAuthority,
    failure: CommitPreviewFailureEvidence,
}

impl CommitPreviewBlockedAuthority {
    fn new(
        request: ValidatedRepositoryCommitPreviewRequest,
        guard: PostMergeCommitGuardAuthority,
        comment_policy: ValidatedCommitCommentPolicyAuthority,
        failure: CommitPreviewFailureEvidence,
    ) -> Box<Self> {
        Box::new(Self {
            request,
            guard,
            comment_policy,
            failure,
        })
    }

    pub(crate) fn failure(&self) -> &CommitPreviewFailureEvidence {
        &self.failure
    }

    pub(crate) fn into_recovery_parts(
        self: Box<Self>,
    ) -> (
        ValidatedRepositoryCommitPreviewRequest,
        PostMergeCommitGuardAuthority,
        ValidatedCommitCommentPolicyAuthority,
        CommitPreviewFailureEvidence,
    ) {
        let Self {
            request,
            guard,
            comment_policy,
            failure,
        } = *self;
        (request, guard, comment_policy, failure)
    }
}

#[cfg(test)]
pub(crate) struct CommitPreviewAuthorityTestParts {
    pub(crate) plan: LockPlanData,
    pub(crate) guard_locks: CanonicalRepositoryTargets,
    pub(crate) comment: Comment,
    pub(crate) verification_digest: Sha256Digest,
    pub(crate) lock_set_digest: Sha256Digest,
    pub(crate) merge_receipt_id: UnicaId,
    pub(crate) merge_receipt_cursor: RepositoryHistoryCursor,
    pub(crate) consumed_support_gate_history_evidence: SupportGateHistoryEvidence,
    pub(crate) authorized_post_merge_fingerprint: Sha256Digest,
    pub(crate) observed_original_fingerprint: Sha256Digest,
    pub(crate) history_guard_evidence: PostMergeHistoryGuardEvidence,
}

impl CommitPreviewAuthority {
    #[cfg(test)]
    fn from_verified_main_integration(
        verified: ValidatedMainIntegrationVerificationAuthority,
        guard: PostMergeCommitGuardFixtureAuthority,
        comment_policy: ValidatedCommitCommentPolicyAuthority,
    ) -> Result<Self, RepositoryResultContractError> {
        let verification_id = verified.verification_id().clone();
        let verification_digest = verified.verification_digest().clone();
        let merge_receipt_id = verified.merge_receipt_id().clone();
        let merge_receipt_cursor = verified.repository_history_cursor().clone();
        let authorized_post_merge_fingerprint = verified.result_fingerprint().clone();
        let expected_plan_id = verified.plan_id().clone();
        let expected_plan_digest = verified.plan_digest().clone();
        let expected_session_id = verified.session_id().clone();
        let expected_resolved_session_digest = verified.resolved_session_digest().clone();
        let expected_gate_id = verified.support_gate_id().clone();
        let expected_gate_digest = verified.support_gate_digest().clone();
        let expected_gate_history = verified.support_gate_history_evidence().clone();
        let expected_integration_set_id = verified.integration_set_id().clone();
        let expected_integration_set_digest = verified.integration_set_digest().clone();
        let expected_lock_set_id = verified.lock_set_id().clone();
        let expected_lock_set_digest = verified.lock_set_digest().clone();
        let (plan, lock_set_id, lock_set_digest) = verified.into_lock_projection().into_parts();

        if plan.plan_id != expected_plan_id
            || plan.plan_digest != expected_plan_digest
            || plan.merge_session_id != expected_session_id
            || plan.resolved_session_digest != expected_resolved_session_digest
            || plan.support_gate_id != expected_gate_id
            || plan.support_gate_digest != expected_gate_digest
            || plan.support_gate_history_evidence != expected_gate_history
            || plan.integration_set_id != expected_integration_set_id
            || plan.integration_set_digest != expected_integration_set_digest
            || lock_set_id != expected_lock_set_id
            || lock_set_digest != expected_lock_set_digest
        {
            return Err(RepositoryResultContractError(
                "commit preview received a cross-plan merge verification lineage",
            ));
        }
        if guard.history_guard_evidence.merge_receipt_cursor() != &merge_receipt_cursor
            || plan
                .support_gate_history_evidence
                .classified_through_cursor()
                != &merge_receipt_cursor
            || guard
                .history_guard_evidence
                .recomputed_reference_closure_digest()
                != &plan.reference_closure_digest
        {
            return Err(RepositoryResultContractError(
                "post-merge guard is not rooted at this receipt and plan closure",
            ));
        }
        if guard.observed_original_fingerprint != authorized_post_merge_fingerprint {
            return Err(RepositoryResultContractError(
                "commit preview observed an unauthorized original fingerprint",
            ));
        }

        let exact_objects = CommitExactObjects::new(plan.exact_objects().0)?;
        let exact_objects_digest = result_digest(
            &CommitExactObjectsDigestRecord(exact_objects.clone()),
            "exact commit-object digest failed",
        )?;
        let guard_locks = derive_commit_guard_locks(&plan)?;
        let record = CommitPreviewDigestRecord {
            exact_objects,
            guard_locks,
            comment: comment_policy.rendered_comment().clone(),
            integration_set_digest: plan.integration_set_digest.clone(),
            exact_objects_digest,
            verification_digest,
            lock_set_digest,
            merge_receipt_id,
            support_gate_id: plan.support_gate_id.clone(),
            consumed_support_gate_digest: plan.support_gate_digest.clone(),
            support_gate_history_evidence_digest: plan
                .support_gate_history_evidence
                .evidence_digest()
                .clone(),
            authorized_post_merge_fingerprint,
            observed_original_fingerprint: guard.observed_original_fingerprint,
            history_guard_evidence: guard.history_guard_evidence,
        };
        let commit_digest = result_digest(&record, "commit preview digest failed")?;
        let _fixture_lineage = (
            verification_id,
            lock_set_id,
            comment_policy.policy_digest,
            comment_policy.renderer_capability_id,
            guard.original_fingerprint_capability_id,
        );
        Ok(Self {
            plan,
            record,
            commit_digest,
            _validated_lineage: None,
        })
    }

    pub(crate) fn from_validated_post_merge_guard(
        request: ValidatedRepositoryCommitPreviewRequest,
        guard: PostMergeCommitGuardAuthority,
        comment_policy: ValidatedCommitCommentPolicyAuthority,
    ) -> Result<Self, Box<CommitPreviewBlockedAuthority>> {
        Self::from_validated_post_merge_guard_using_digests(
            request,
            guard,
            comment_policy,
            |record| result_digest(record, "exact commit-object digest failed"),
            |record| result_digest(record, "commit preview digest failed"),
        )
    }

    fn from_validated_post_merge_guard_using_digests<ExactDigest, PreviewDigest>(
        request: ValidatedRepositoryCommitPreviewRequest,
        guard: PostMergeCommitGuardAuthority,
        comment_policy: ValidatedCommitCommentPolicyAuthority,
        exact_objects_digest: ExactDigest,
        preview_digest: PreviewDigest,
    ) -> Result<Self, Box<CommitPreviewBlockedAuthority>>
    where
        ExactDigest: FnOnce(
            &CommitExactObjectsDigestRecord,
        ) -> Result<Sha256Digest, RepositoryResultContractError>,
        PreviewDigest: FnOnce(
            &CommitPreviewDigestRecord,
        ) -> Result<Sha256Digest, RepositoryResultContractError>,
    {
        let lineage = guard.source.lineage();
        if request.task_id() != &comment_policy.record.task_id
            || request.integration_set_id() != lineage.integration_set_id()
            || request.expected_integration_set_digest() != lineage.integration_set_digest()
            || request.lock_set_id() != lineage.lock_set_id()
            || request.expected_lock_set_digest() != lineage.lock_set_digest()
            || request.verification_id() != lineage.verification_id()
            || request.expected_verification_digest() != lineage.verification_digest()
            || request.merge_receipt_id() != lineage.merge_receipt_id()
            || request.support_gate_id() != lineage.support_gate_id()
            || request.expected_support_gate_digest() != lineage.support_gate_digest()
            || request.expected_support_gate_digest()
                != lineage.consumed_gate().support_gate_digest()
            || request.expected_support_gate_history_evidence_digest()
                != lineage.support_gate_history_evidence().evidence_digest()
            || request.expected_authorized_post_merge_fingerprint() != lineage.result_fingerprint()
        {
            return Err(CommitPreviewBlockedAuthority::new(
                request,
                guard,
                comment_policy,
                CommitPreviewFailureEvidence::RequestLineageMismatch,
            ));
        }
        let plan = lineage.lock_plan().clone();
        let exact_objects = match CommitExactObjects::new(plan.exact_objects().0) {
            Ok(value) => value,
            Err(error) => {
                return Err(CommitPreviewBlockedAuthority::new(
                    request,
                    guard,
                    comment_policy,
                    CommitPreviewFailureEvidence::InvalidExactObjects(error),
                ));
            }
        };
        let exact_objects_digest =
            match exact_objects_digest(&CommitExactObjectsDigestRecord(exact_objects.clone())) {
                Ok(value) => value,
                Err(error) => {
                    return Err(CommitPreviewBlockedAuthority::new(
                        request,
                        guard,
                        comment_policy,
                        CommitPreviewFailureEvidence::ExactObjectsDigest(error),
                    ));
                }
            };
        let guard_locks = match derive_commit_guard_locks(&plan) {
            Ok(value) => value,
            Err(error) => {
                return Err(CommitPreviewBlockedAuthority::new(
                    request,
                    guard,
                    comment_policy,
                    CommitPreviewFailureEvidence::GuardLocks(error),
                ));
            }
        };
        let record = CommitPreviewDigestRecord {
            exact_objects,
            guard_locks,
            comment: comment_policy.rendered_comment().clone(),
            integration_set_digest: lineage.integration_set_digest().clone(),
            exact_objects_digest,
            verification_digest: lineage.verification_digest().clone(),
            lock_set_digest: lineage.lock_set_digest().clone(),
            merge_receipt_id: lineage.merge_receipt_id().clone(),
            support_gate_id: lineage.support_gate_id().clone(),
            consumed_support_gate_digest: lineage.consumed_gate().support_gate_digest().clone(),
            support_gate_history_evidence_digest: lineage
                .support_gate_history_evidence()
                .evidence_digest()
                .clone(),
            authorized_post_merge_fingerprint: lineage.result_fingerprint().clone(),
            observed_original_fingerprint: guard.observed_original_fingerprint.clone(),
            history_guard_evidence: guard.history_guard_evidence.clone(),
        };
        let commit_digest = match preview_digest(&record) {
            Ok(value) => value,
            Err(error) => {
                return Err(CommitPreviewBlockedAuthority::new(
                    request,
                    guard,
                    comment_policy,
                    CommitPreviewFailureEvidence::CommitDigest(error),
                ));
            }
        };
        let validated_lineage = CommitPreviewValidatedLineage {
            preview_request: request,
            verification_id: lineage.verification_id().clone(),
            lock_set_id: lineage.lock_set_id().clone(),
            comment_policy,
            original_fingerprint_capability_id: guard.original_fingerprint_capability_id.clone(),
            post_merge_guard: Box::new(guard),
        };
        Ok(Self {
            plan,
            record,
            commit_digest,
            _validated_lineage: Some(validated_lineage),
        })
    }

    #[cfg(test)]
    pub(crate) fn test_only(
        parts: CommitPreviewAuthorityTestParts,
    ) -> Result<Self, RepositoryResultContractError> {
        if parts.consumed_support_gate_history_evidence != parts.plan.support_gate_history_evidence
        {
            return Err(RepositoryResultContractError(
                "commit preview consumed another support-gate history evidence",
            ));
        }
        if parts.history_guard_evidence.merge_receipt_cursor() != &parts.merge_receipt_cursor
            || parts
                .consumed_support_gate_history_evidence
                .classified_through_cursor()
                != &parts.merge_receipt_cursor
        {
            return Err(RepositoryResultContractError(
                "commit history guard, merge receipt, and consumed gate cursors disagree",
            ));
        }
        if parts.authorized_post_merge_fingerprint != parts.observed_original_fingerprint {
            return Err(RepositoryResultContractError(
                "commit preview observed an unauthorized original fingerprint",
            ));
        }
        if parts.guard_locks != derive_commit_guard_locks(&parts.plan)?
            || parts
                .history_guard_evidence
                .recomputed_reference_closure_digest()
                != &parts.plan.reference_closure_digest
        {
            return Err(RepositoryResultContractError(
                "commit preview guard set or reference closure differs from the exact plan",
            ));
        }
        let exact_objects = CommitExactObjects::new(parts.plan.exact_objects().0)?;
        let exact_objects_digest = result_digest(
            &CommitExactObjectsDigestRecord(exact_objects.clone()),
            "exact commit-object digest failed",
        )?;
        let record = CommitPreviewDigestRecord {
            exact_objects,
            guard_locks: parts.guard_locks,
            comment: parts.comment,
            integration_set_digest: parts.plan.integration_set_digest.clone(),
            exact_objects_digest,
            verification_digest: parts.verification_digest,
            lock_set_digest: parts.lock_set_digest,
            merge_receipt_id: parts.merge_receipt_id,
            support_gate_id: parts.plan.support_gate_id.clone(),
            consumed_support_gate_digest: parts.plan.support_gate_digest.clone(),
            support_gate_history_evidence_digest: parts
                .consumed_support_gate_history_evidence
                .evidence_digest()
                .clone(),
            authorized_post_merge_fingerprint: parts.authorized_post_merge_fingerprint,
            observed_original_fingerprint: parts.observed_original_fingerprint,
            history_guard_evidence: parts.history_guard_evidence,
        };
        let commit_digest = result_digest(&record, "commit preview digest failed")?;
        Ok(Self {
            plan: parts.plan,
            record,
            commit_digest,
            _validated_lineage: None,
        })
    }

    pub(crate) fn data(&self) -> CommitPreviewData {
        CommitPreviewData {
            exact_objects: self.record.exact_objects.clone(),
            guard_locks: self.record.guard_locks.clone(),
            comment: self.record.comment.clone(),
            integration_set_digest: self.record.integration_set_digest.clone(),
            exact_objects_digest: self.record.exact_objects_digest.clone(),
            verification_digest: self.record.verification_digest.clone(),
            lock_set_digest: self.record.lock_set_digest.clone(),
            merge_receipt_id: self.record.merge_receipt_id.clone(),
            support_gate_id: self.record.support_gate_id.clone(),
            consumed_support_gate_digest: self.record.consumed_support_gate_digest.clone(),
            support_gate_history_evidence_digest: self
                .record
                .support_gate_history_evidence_digest
                .clone(),
            authorized_post_merge_fingerprint: self
                .record
                .authorized_post_merge_fingerprint
                .clone(),
            observed_original_fingerprint: self.record.observed_original_fingerprint.clone(),
            history_guard_evidence: self.record.history_guard_evidence.clone(),
            commit_digest: self.commit_digest.clone(),
        }
    }

    pub(crate) fn commit_digest(&self) -> &Sha256Digest {
        &self.commit_digest
    }

    pub(crate) fn validated_preview_request(&self) -> &ValidatedRepositoryCommitPreviewRequest {
        &self
            ._validated_lineage
            .as_ref()
            .expect("production preview owns its validated request")
            .preview_request
    }

    pub(crate) fn validated_comment_policy(&self) -> &ValidatedCommitCommentPolicyAuthority {
        &self
            ._validated_lineage
            .as_ref()
            .expect("production preview owns its validated comment policy")
            .comment_policy
    }

    pub(crate) fn validate_apply(
        self,
        request: RepositoryCommitRequest,
    ) -> Result<ValidatedCommitApplyApprovalAuthority, Box<CommitApplyApprovalBlockedAuthority>>
    {
        let apply_request = match request.into_validated_apply() {
            Ok(request) => request,
            Err(failure) => {
                return Err(CommitApplyApprovalBlockedAuthority::new(
                    self,
                    failure.into_request(),
                    CommitApplyApprovalFailureEvidence::NotApply,
                ));
            }
        };
        let preview_request = self.validated_preview_request();
        if apply_request.operation_id() == preview_request.operation_id() {
            return Err(CommitApplyApprovalBlockedAuthority::new(
                self,
                apply_request.into_request(),
                CommitApplyApprovalFailureEvidence::SameOperationId,
            ));
        }
        if apply_request.cwd() != preview_request.cwd()
            || apply_request.task_id() != preview_request.task_id()
            || apply_request.integration_set_id() != preview_request.integration_set_id()
            || apply_request.expected_integration_set_digest()
                != preview_request.expected_integration_set_digest()
            || apply_request.lock_set_id() != preview_request.lock_set_id()
            || apply_request.expected_lock_set_digest()
                != preview_request.expected_lock_set_digest()
            || apply_request.verification_id() != preview_request.verification_id()
            || apply_request.expected_verification_digest()
                != preview_request.expected_verification_digest()
            || apply_request.merge_receipt_id() != preview_request.merge_receipt_id()
            || apply_request.support_gate_id() != preview_request.support_gate_id()
            || apply_request.expected_support_gate_digest()
                != preview_request.expected_support_gate_digest()
            || apply_request.expected_support_gate_history_evidence_digest()
                != preview_request.expected_support_gate_history_evidence_digest()
            || apply_request.expected_authorized_post_merge_fingerprint()
                != preview_request.expected_authorized_post_merge_fingerprint()
        {
            return Err(CommitApplyApprovalBlockedAuthority::new(
                self,
                apply_request.into_request(),
                CommitApplyApprovalFailureEvidence::RequestLineageMismatch,
            ));
        }
        if apply_request.approved_commit_digest() != self.commit_digest() {
            return Err(CommitApplyApprovalBlockedAuthority::new(
                self,
                apply_request.into_request(),
                CommitApplyApprovalFailureEvidence::ApprovedCommitDigestMismatch,
            ));
        }
        Ok(ValidatedCommitApplyApprovalAuthority {
            preview: self,
            apply_request,
        })
    }

    pub(crate) fn has_persisted_consumed_gate_lineage(&self) -> bool {
        self._validated_lineage.is_some()
    }

    pub(crate) fn consumed_gate_observation_revision(&self) -> &Sha256Digest {
        self._validated_lineage
            .as_ref()
            .map(|lineage| lineage.post_merge_guard.source.consumed_gate_observation())
            .expect("production preview owns its consumed-gate observation")
            .consumed_state_revision()
    }

    pub(crate) fn validated_consumed_gate_digest(&self) -> &Sha256Digest {
        self._validated_lineage
            .as_ref()
            .map(|lineage| lineage.post_merge_guard.source.lineage())
            .expect("production preview owns its consumed-gate lineage")
            .consumed_gate()
            .support_gate_digest()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CommitPreviewData {
    exact_objects: CommitExactObjects,
    guard_locks: CanonicalRepositoryTargets,
    comment: Comment,
    integration_set_digest: Sha256Digest,
    exact_objects_digest: Sha256Digest,
    verification_digest: Sha256Digest,
    lock_set_digest: Sha256Digest,
    merge_receipt_id: UnicaId,
    support_gate_id: UnicaId,
    consumed_support_gate_digest: Sha256Digest,
    support_gate_history_evidence_digest: Sha256Digest,
    authorized_post_merge_fingerprint: Sha256Digest,
    observed_original_fingerprint: Sha256Digest,
    history_guard_evidence: PostMergeHistoryGuardEvidence,
    commit_digest: Sha256Digest,
}

impl CommitPreviewData {
    pub(crate) fn exact_objects(&self) -> &CommitExactObjects {
        &self.exact_objects
    }

    pub(crate) fn exact_objects_digest(&self) -> &Sha256Digest {
        &self.exact_objects_digest
    }

    pub(crate) fn commit_digest(&self) -> &Sha256Digest {
        &self.commit_digest
    }
}

/// One-shot production proof that an apply request was checked against and now
/// owns one exact preview authority. No independent preview/request pair can be
/// supplied to the approval mint after this boundary.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ValidatedCommitApplyApprovalAuthority {
    preview: CommitPreviewAuthority,
    apply_request: ValidatedRepositoryCommitApplyRequest,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum CommitApplyApprovalFailureEvidence {
    NotApply,
    SameOperationId,
    RequestLineageMismatch,
    ApprovedCommitDigestMismatch,
}

/// Owning retry result for apply validation. It returns both the exact preview
/// and the consumed wire request on every failure.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct CommitApplyApprovalBlockedAuthority {
    preview: CommitPreviewAuthority,
    request: RepositoryCommitRequest,
    failure: CommitApplyApprovalFailureEvidence,
}

impl CommitApplyApprovalBlockedAuthority {
    fn new(
        preview: CommitPreviewAuthority,
        request: RepositoryCommitRequest,
        failure: CommitApplyApprovalFailureEvidence,
    ) -> Box<Self> {
        Box::new(Self {
            preview,
            request,
            failure,
        })
    }

    pub(crate) const fn failure(&self) -> &CommitApplyApprovalFailureEvidence {
        &self.failure
    }

    pub(crate) fn into_recovery_parts(
        self: Box<Self>,
    ) -> (
        CommitPreviewAuthority,
        RepositoryCommitRequest,
        CommitApplyApprovalFailureEvidence,
    ) {
        let Self {
            preview,
            request,
            failure,
        } = *self;
        (preview, request, failure)
    }
}

/// Preview plus a digest approval already validated against the immutable
/// preview record. The production request coordinator must mint this token;
/// tests use the cfg-only helper.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ApprovedCommitPreviewAuthority(
    CommitPreviewAuthority,
    Option<ValidatedRepositoryCommitApplyRequest>,
);

impl ApprovedCommitPreviewAuthority {
    pub(crate) fn from_validated_request(validated: ValidatedCommitApplyApprovalAuthority) -> Self {
        Self(validated.preview, Some(validated.apply_request))
    }

    #[cfg(test)]
    pub(crate) fn approve_test_only(
        authority: CommitPreviewAuthority,
        approved_commit_digest: &Sha256Digest,
    ) -> Result<Self, RepositoryResultContractError> {
        if authority.commit_digest() != approved_commit_digest {
            return Err(RepositoryResultContractError(
                "approved commit digest does not match the preview",
            ));
        }
        Ok(Self(authority, None))
    }

    pub(crate) fn preview(&self) -> CommitPreviewData {
        self.0.data()
    }

    pub(crate) fn validated_apply_request(&self) -> &ValidatedRepositoryCommitApplyRequest {
        self.1
            .as_ref()
            .expect("production approval owns its validated apply request")
    }

    /// Consumes the approved preview into the one immediate, request-bound
    /// recheck that is allowed to authorize a physical commit intent.
    pub(crate) fn recheck_before_commit_intent(
        self,
        port: &mut dyn CommitImmediateRecheckPort,
    ) -> CommitImmediateRecheckOutcome {
        let invocation = CommitImmediateRecheckInvocationCapability::mint();
        let request = CommitImmediateRecheckRequest {
            approved: &self,
            invocation: &invocation,
        };
        let completion = match port.recheck_before_commit_intent(request) {
            Ok(completion) => completion,
            Err(error) => {
                return CommitImmediateRecheckOutcome::RecoveryRequired(
                    CommitImmediateRecoveryRequiredAuthority {
                        approved: self,
                        failure: CommitImmediateRecheckFailureEvidence::PortError(error),
                    },
                );
            }
        };

        if !invocation.owns_completion(&completion.completion) {
            return CommitImmediateRecheckOutcome::RecoveryRequired(
                CommitImmediateRecoveryRequiredAuthority {
                    approved: self,
                    failure: CommitImmediateRecheckFailureEvidence::CompletionAttemptMismatch {
                        completion,
                    },
                },
            );
        }
        if !self
            .0
            ._validated_lineage
            .as_ref()
            .expect("immediate commit recheck requires the production approval lineage")
            .post_merge_guard
            .source
            .owns_witness(completion.lease.commit_safety_lineage_witness())
        {
            return CommitImmediateRecheckOutcome::RecoveryRequired(
                CommitImmediateRecoveryRequiredAuthority {
                    approved: self,
                    failure: CommitImmediateRecheckFailureEvidence::CapabilityBindingMismatch {
                        completion,
                    },
                },
            );
        }
        let request = CommitImmediateRecheckRequest {
            approved: &self,
            invocation: &invocation,
        };
        if !completion.lease.binds(&request) {
            return CommitImmediateRecheckOutcome::RecoveryRequired(
                CommitImmediateRecoveryRequiredAuthority {
                    approved: self,
                    failure: CommitImmediateRecheckFailureEvidence::CapabilityBindingMismatch {
                        completion,
                    },
                },
            );
        }

        let observed = CommitImmediateRecheckObservedEvidence::from_lease(&*completion.lease);
        let lineage = self
            .0
            ._validated_lineage
            .as_ref()
            .expect("immediate commit recheck requires the production approval lineage");
        let guard = &lineage.post_merge_guard;
        let prior_history = &self.0.record.history_guard_evidence;
        let retained_anchor = &guard.post_merge_repository_anchor;

        if observed.consumed_state_revision
            != *guard
                .source
                .consumed_gate_observation()
                .consumed_state_revision()
            || observed.consumed_state_observation_capability_id
                != *guard
                    .source
                    .consumed_gate_observation()
                    .observation_capability_id()
            || observed.original_fingerprint_capability_id
                != guard.original_fingerprint_capability_id
            || observed.root_reread_capability_id
                != *guard.source.lineage().root_reread_capability_id()
            || observed.atomic_commit_safety_capability_id
                != *prior_history.atomic_commit_safety_capability_id()
        {
            return CommitImmediateRecheckOutcome::RecoveryRequired(
                CommitImmediateRecoveryRequiredAuthority {
                    approved: self,
                    failure:
                        CommitImmediateRecheckFailureEvidence::ConsumedGateOrCapabilityChanged {
                            completion,
                            evidence: Box::new(observed),
                        },
                },
            );
        }

        if observed.recomputed_reference_closure_digest
            != *guard
                .source
                .lineage()
                .lock_plan()
                .reference_closure_digest()
            || observed.observed_original_fingerprint
                != self.0.record.authorized_post_merge_fingerprint
            || observed.observed_repository_anchor.repository_identity()
                != retained_anchor.repository_identity()
            || observed.observed_repository_anchor.configuration_identity()
                != retained_anchor.configuration_identity()
            || observed
                .observed_repository_anchor
                .configuration_fingerprint()
                != retained_anchor.configuration_fingerprint()
            || observed.observed_repository_anchor.history_cursor()
                != observed.history_partition.through_inclusive()
        {
            return CommitImmediateRecheckOutcome::RecoveryRequired(
                CommitImmediateRecoveryRequiredAuthority {
                    approved: self,
                    failure: CommitImmediateRecheckFailureEvidence::PostMergeStateChanged {
                        completion,
                        evidence: Box::new(observed),
                    },
                },
            );
        }

        if observed
            .history_partition
            .classifications()
            .any(|classification| {
                classification == RepositoryHistoryPartitionClassification::NonConflictingConcurrent
            })
        {
            return CommitImmediateRecheckOutcome::RecoveryRequired(
                CommitImmediateRecoveryRequiredAuthority {
                    approved: self,
                    failure:
                        CommitImmediateRecheckFailureEvidence::UnscopedNonConflictingConcurrent {
                            completion,
                            evidence: Box::new(observed),
                        },
                },
            );
        }

        if observed
            .history_partition
            .is_semantically_exact(prior_history.partition())
        {
            if &observed.observed_repository_anchor != retained_anchor {
                return CommitImmediateRecheckOutcome::RecoveryRequired(
                    CommitImmediateRecoveryRequiredAuthority {
                        approved: self,
                        failure: CommitImmediateRecheckFailureEvidence::PostMergeStateChanged {
                            completion,
                            evidence: Box::new(observed),
                        },
                    },
                );
            }
            let refreshed_history =
                match rebuild_immediate_history_evidence(prior_history, &observed) {
                    Ok(evidence)
                        if evidence.evidence_digest() == prior_history.evidence_digest() =>
                    {
                        evidence
                    }
                    Ok(_) | Err(_) => {
                        return CommitImmediateRecheckOutcome::RecoveryRequired(
                            CommitImmediateRecoveryRequiredAuthority {
                                approved: self,
                                failure:
                                    CommitImmediateRecheckFailureEvidence::HistoryLineageChanged {
                                        completion,
                                        evidence: Box::new(observed),
                                    },
                            },
                        );
                    }
                };
            let before_repository_cursor = observed.history_partition.through_inclusive().clone();
            let post_merge_repository_anchor = observed.observed_repository_anchor.clone();
            let pre_command_target_snapshot =
                match CommitPreCommandTargetSnapshotAuthority::from_immediate_observation(
                    &self.0.record.exact_objects,
                    observed.observed_repository_anchor.clone(),
                    observed.pre_command_target_states.clone(),
                    observed
                        .pre_command_target_snapshot_observation_capability_id
                        .clone(),
                    observed.atomic_commit_safety_capability_id.clone(),
                ) {
                    Ok(snapshot) => snapshot,
                    Err(_) => {
                        return CommitImmediateRecheckOutcome::RecoveryRequired(
                            CommitImmediateRecoveryRequiredAuthority {
                                approved: self,
                                failure:
                                    CommitImmediateRecheckFailureEvidence::PostMergeStateChanged {
                                        completion,
                                        evidence: Box::new(observed),
                                    },
                            },
                        );
                    }
                };
            return CommitImmediateRecheckOutcome::Ready(CommitScopedAtomicSafetyAuthority {
                approved: self,
                completion,
                immediate_history_guard_evidence: refreshed_history,
                post_merge_repository_anchor,
                before_repository_cursor,
                pre_command_target_snapshot: Box::new(pre_command_target_snapshot),
            });
        }

        if observed.history_partition.entry_count() > prior_history.partition().entry_count()
            && observed
                .history_partition
                .has_exact_entry_prefix(prior_history.partition())
        {
            if !observed
                .history_partition
                .is_strict_unrelated_extension_of(prior_history.partition())
            {
                return CommitImmediateRecheckOutcome::RecoveryRequired(
                    CommitImmediateRecoveryRequiredAuthority {
                        approved: self,
                        failure: CommitImmediateRecheckFailureEvidence::UnsafeHistoryAdvance {
                            completion,
                            evidence: Box::new(observed),
                        },
                    },
                );
            }
            let fresh_history_guard_evidence =
                match rebuild_immediate_history_evidence(prior_history, &observed) {
                    Ok(evidence) => evidence,
                    Err(_) => {
                        return CommitImmediateRecheckOutcome::RecoveryRequired(
                            CommitImmediateRecoveryRequiredAuthority {
                                approved: self,
                                failure:
                                    CommitImmediateRecheckFailureEvidence::HistoryLineageChanged {
                                        completion,
                                        evidence: Box::new(observed),
                                    },
                            },
                        );
                    }
                };
            let fresh_repository_anchor = observed.observed_repository_anchor.clone();
            return CommitImmediateRecheckOutcome::FreshPreviewRequired(
                CommitFreshPreviewRequiredAuthority {
                    approved: self,
                    completion,
                    fresh_history_guard_evidence,
                    fresh_repository_anchor,
                },
            );
        }

        CommitImmediateRecheckOutcome::RecoveryRequired(CommitImmediateRecoveryRequiredAuthority {
            approved: self,
            failure: CommitImmediateRecheckFailureEvidence::HistoryLineageChanged {
                completion,
                evidence: Box::new(observed),
            },
        })
    }
}

#[derive(Debug)]
struct CommitImmediateRecheckInvocationMarker;

#[derive(Debug)]
struct CommitImmediateRecheckInvocationCapability(Arc<CommitImmediateRecheckInvocationMarker>);

#[derive(Debug)]
struct CommitImmediateRecheckCompletionCapability(Arc<CommitImmediateRecheckInvocationMarker>);

impl CommitImmediateRecheckInvocationCapability {
    fn mint() -> Self {
        Self(Arc::new(CommitImmediateRecheckInvocationMarker))
    }

    fn completion(&self) -> CommitImmediateRecheckCompletionCapability {
        CommitImmediateRecheckCompletionCapability(Arc::clone(&self.0))
    }

    fn owns_completion(&self, completion: &CommitImmediateRecheckCompletionCapability) -> bool {
        Arc::ptr_eq(&self.0, &completion.0)
    }
}

/// Read-only, request-bound view of the complete approved commit lineage.
#[derive(Debug)]
pub(crate) struct CommitImmediateRecheckRequest<'a> {
    approved: &'a ApprovedCommitPreviewAuthority,
    invocation: &'a CommitImmediateRecheckInvocationCapability,
}

impl CommitImmediateRecheckRequest<'_> {
    fn preview_lineage(&self) -> &CommitPreviewValidatedLineage {
        self.approved
            .0
            ._validated_lineage
            .as_ref()
            .expect("immediate commit recheck requires a production preview lineage")
    }

    fn guard(&self) -> &PostMergeCommitGuardAuthority {
        &self.preview_lineage().post_merge_guard
    }

    pub(crate) fn commit_safety_lineage_witness(&self) -> CommitSafetyLineageWitness {
        self.guard().source.witness()
    }

    pub(crate) fn cwd(&self) -> &OriginalProjectCwd {
        self.approved.validated_apply_request().cwd()
    }

    pub(crate) fn task_id(&self) -> &TaskId {
        self.approved.validated_apply_request().task_id()
    }

    pub(crate) fn preview_operation_id(&self) -> &OperationId {
        self.approved.0.validated_preview_request().operation_id()
    }

    pub(crate) fn apply_operation_id(&self) -> &OperationId {
        self.approved.validated_apply_request().operation_id()
    }

    pub(crate) fn approved_commit_digest(&self) -> &Sha256Digest {
        self.approved.0.commit_digest()
    }

    pub(crate) fn session_id(&self) -> &UnicaId {
        self.guard().source.lineage().session_id()
    }

    pub(crate) fn resolved_session_digest(&self) -> &Sha256Digest {
        self.guard().source.lineage().resolved_session_digest()
    }

    pub(crate) fn verification_id(&self) -> &UnicaId {
        self.guard().source.lineage().verification_id()
    }

    pub(crate) fn verification_digest(&self) -> &Sha256Digest {
        self.guard().source.lineage().verification_digest()
    }

    pub(crate) fn merge_receipt_id(&self) -> &UnicaId {
        self.guard().source.lineage().merge_receipt_id()
    }

    pub(crate) fn merge_receipt_cursor(&self) -> &RepositoryHistoryCursor {
        self.guard().source.lineage().merge_receipt_cursor()
    }

    pub(crate) fn plan_id(&self) -> &UnicaId {
        self.guard().source.lineage().plan_id()
    }

    pub(crate) fn plan_digest(&self) -> &Sha256Digest {
        self.guard().source.lineage().plan_digest()
    }

    pub(crate) fn integration_set_id(&self) -> &UnicaId {
        self.guard().source.lineage().integration_set_id()
    }

    pub(crate) fn integration_set_digest(&self) -> &Sha256Digest {
        self.guard().source.lineage().integration_set_digest()
    }

    pub(crate) fn lock_set_id(&self) -> &UnicaId {
        self.guard().source.lineage().lock_set_id()
    }

    pub(crate) fn lock_set_digest(&self) -> &Sha256Digest {
        self.guard().source.lineage().lock_set_digest()
    }

    pub(crate) fn support_gate_id(&self) -> &UnicaId {
        self.guard().source.lineage().support_gate_id()
    }

    pub(crate) fn support_gate_digest(&self) -> &Sha256Digest {
        self.guard().source.lineage().support_gate_digest()
    }

    pub(crate) fn support_gate_history_evidence(&self) -> &SupportGateHistoryEvidence {
        self.guard()
            .source
            .lineage()
            .support_gate_history_evidence()
    }

    pub(crate) fn expected_support_gate_history_evidence_digest(&self) -> &Sha256Digest {
        self.approved
            .validated_apply_request()
            .expected_support_gate_history_evidence_digest()
    }

    pub(crate) fn lock_plan(&self) -> &LockPlanData {
        self.guard().source.lineage().lock_plan()
    }

    pub(crate) fn integration_entries(&self) -> &RepositoryIntegrationEntries {
        self.lock_plan().integration_entries()
    }

    pub(crate) fn planned_locks(&self) -> &RepositoryUpdateLockTargets {
        self.lock_plan().lock_entries()
    }

    pub(crate) fn journaled_lock_receipts(&self) -> &[JournaledRepositoryLock] {
        self.guard().source.lineage().journaled_lock_receipts()
    }

    pub(crate) fn rollback_checkpoint_id(&self) -> &UnicaId {
        self.guard().source.lineage().rollback_checkpoint_id()
    }

    pub(crate) fn exact_objects(&self) -> &CommitExactObjects {
        &self.approved.0.record.exact_objects
    }

    pub(crate) fn guard_locks(&self) -> &CanonicalRepositoryTargets {
        &self.approved.0.record.guard_locks
    }

    pub(crate) fn validated_preview_request(&self) -> &ValidatedRepositoryCommitPreviewRequest {
        self.approved.0.validated_preview_request()
    }

    pub(crate) fn validated_apply_request(&self) -> &ValidatedRepositoryCommitApplyRequest {
        self.approved.validated_apply_request()
    }

    pub(crate) fn consumed_state_revision(&self) -> &Sha256Digest {
        self.guard()
            .source
            .consumed_gate_observation()
            .consumed_state_revision()
    }

    pub(crate) fn consumed_state_observation_capability_id(&self) -> &CapabilityRowId {
        self.guard()
            .source
            .consumed_gate_observation()
            .observation_capability_id()
    }

    pub(crate) fn authorized_post_merge_fingerprint(&self) -> &Sha256Digest {
        &self.approved.0.record.authorized_post_merge_fingerprint
    }

    pub(crate) fn expected_reference_closure_digest(&self) -> &Sha256Digest {
        self.guard()
            .source
            .lineage()
            .lock_plan()
            .reference_closure_digest()
    }

    pub(crate) fn retained_post_merge_repository_anchor(&self) -> &RepositoryAnchor {
        &self.guard().post_merge_repository_anchor
    }

    pub(crate) fn original_fingerprint_capability_id(&self) -> &CapabilityRowId {
        &self.guard().original_fingerprint_capability_id
    }

    pub(crate) fn root_reread_capability_id(&self) -> &CapabilityRowId {
        self.guard().source.lineage().root_reread_capability_id()
    }

    pub(crate) fn atomic_commit_safety_capability_id(&self) -> &CapabilityRowId {
        self.approved
            .0
            .record
            .history_guard_evidence
            .atomic_commit_safety_capability_id()
    }

    pub(crate) fn observe_repository_anchor(
        &self,
        history_partition: &ValidatedRepositoryHistoryPartition,
        repository_identity: Sha256Digest,
        configuration_identity: ConfigurationIdentity,
        configuration_fingerprint: Sha256Digest,
    ) -> Result<RepositoryAnchor, RepositoryResultContractError> {
        RepositoryAnchor::from_guarded_observation(
            repository_identity,
            history_partition.through_inclusive().clone(),
            configuration_identity,
            configuration_fingerprint,
        )
        .map_err(|_| RepositoryResultContractError("immediate repository anchor digest failed"))
    }

    pub(crate) fn complete(
        self,
        lease: Box<dyn CommitImmediateRecheckLease>,
    ) -> CommitImmediateRecheckCompletion {
        CommitImmediateRecheckCompletion {
            completion: self.invocation.completion(),
            lease,
        }
    }
}

pub(crate) struct CommitImmediateRecheckCompletion {
    completion: CommitImmediateRecheckCompletionCapability,
    lease: Box<dyn CommitImmediateRecheckLease>,
}

impl fmt::Debug for CommitImmediateRecheckCompletion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CommitImmediateRecheckCompletion")
            .field("completion", &self.completion)
            .field("lease", &"<live immediate commit recheck lease>")
            .finish()
    }
}

pub(crate) trait CommitImmediateRecheckLease {
    fn commit_safety_lineage_witness(&self) -> &CommitSafetyLineageWitness;
    fn binds(&self, request: &CommitImmediateRecheckRequest<'_>) -> bool;
    fn history_partition(&self) -> &ValidatedRepositoryHistoryPartition;
    fn recomputed_reference_closure_digest(&self) -> &Sha256Digest;
    fn observed_original_fingerprint(&self) -> &Sha256Digest;
    fn observed_repository_anchor(&self) -> &RepositoryAnchor;
    fn consumed_state_revision(&self) -> &Sha256Digest;
    fn consumed_state_observation_capability_id(&self) -> &CapabilityRowId;
    fn original_fingerprint_capability_id(&self) -> &CapabilityRowId;
    fn root_reread_capability_id(&self) -> &CapabilityRowId;
    fn atomic_commit_safety_capability_id(&self) -> &CapabilityRowId;
    fn pre_command_target_states(&self) -> &RepositoryTargetStates;
    fn pre_command_target_snapshot_observation_capability_id(&self) -> &CapabilityRowId;
    fn commit_exact_once(
        self: Box<Self>,
        request: CommitAtomicCommitRequest<'_>,
    ) -> Result<CommitAtomicCommitCompletion, RepositoryResultContractError>;
}

pub(crate) trait CommitImmediateRecheckPort {
    fn recheck_before_commit_intent(
        &mut self,
        request: CommitImmediateRecheckRequest<'_>,
    ) -> Result<CommitImmediateRecheckCompletion, RepositoryResultContractError>;
}

#[derive(Debug)]
pub(crate) struct CommitImmediateRecheckObservedEvidence {
    history_partition: ValidatedRepositoryHistoryPartition,
    recomputed_reference_closure_digest: Sha256Digest,
    observed_original_fingerprint: Sha256Digest,
    observed_repository_anchor: RepositoryAnchor,
    consumed_state_revision: Sha256Digest,
    consumed_state_observation_capability_id: CapabilityRowId,
    original_fingerprint_capability_id: CapabilityRowId,
    root_reread_capability_id: CapabilityRowId,
    atomic_commit_safety_capability_id: CapabilityRowId,
    pre_command_target_states: RepositoryTargetStates,
    pre_command_target_snapshot_observation_capability_id: CapabilityRowId,
}

impl CommitImmediateRecheckObservedEvidence {
    fn from_lease(lease: &dyn CommitImmediateRecheckLease) -> Self {
        Self {
            history_partition: lease.history_partition().clone(),
            recomputed_reference_closure_digest: lease
                .recomputed_reference_closure_digest()
                .clone(),
            observed_original_fingerprint: lease.observed_original_fingerprint().clone(),
            observed_repository_anchor: lease.observed_repository_anchor().clone(),
            consumed_state_revision: lease.consumed_state_revision().clone(),
            consumed_state_observation_capability_id: lease
                .consumed_state_observation_capability_id()
                .clone(),
            original_fingerprint_capability_id: lease.original_fingerprint_capability_id().clone(),
            root_reread_capability_id: lease.root_reread_capability_id().clone(),
            atomic_commit_safety_capability_id: lease.atomic_commit_safety_capability_id().clone(),
            pre_command_target_states: lease.pre_command_target_states().clone(),
            pre_command_target_snapshot_observation_capability_id: lease
                .pre_command_target_snapshot_observation_capability_id()
                .clone(),
        }
    }
}

fn rebuild_immediate_history_evidence(
    prior: &PostMergeHistoryGuardEvidence,
    observed: &CommitImmediateRecheckObservedEvidence,
) -> Result<PostMergeHistoryGuardEvidence, RepositoryResultContractError> {
    let authority = PostMergeHistoryGuardAuthority::from_capability_adapter(
        &observed.history_partition,
        prior.merge_receipt_cursor().clone(),
        observed.recomputed_reference_closure_digest.clone(),
        observed.atomic_commit_safety_capability_id.clone(),
    )
    .map_err(|_| RepositoryResultContractError("immediate history authority validation failed"))?;
    PostMergeHistoryGuardEvidence::new(observed.history_partition.clone(), &authority)
        .map_err(|_| RepositoryResultContractError("immediate history evidence validation failed"))
}

#[derive(Debug)]
pub(crate) enum CommitImmediateRecheckFailureEvidence {
    PortError(RepositoryResultContractError),
    CompletionAttemptMismatch {
        completion: CommitImmediateRecheckCompletion,
    },
    CapabilityBindingMismatch {
        completion: CommitImmediateRecheckCompletion,
    },
    ConsumedGateOrCapabilityChanged {
        completion: CommitImmediateRecheckCompletion,
        evidence: Box<CommitImmediateRecheckObservedEvidence>,
    },
    PostMergeStateChanged {
        completion: CommitImmediateRecheckCompletion,
        evidence: Box<CommitImmediateRecheckObservedEvidence>,
    },
    HistoryLineageChanged {
        completion: CommitImmediateRecheckCompletion,
        evidence: Box<CommitImmediateRecheckObservedEvidence>,
    },
    UnsafeHistoryAdvance {
        completion: CommitImmediateRecheckCompletion,
        evidence: Box<CommitImmediateRecheckObservedEvidence>,
    },
    UnscopedNonConflictingConcurrent {
        completion: CommitImmediateRecheckCompletion,
        evidence: Box<CommitImmediateRecheckObservedEvidence>,
    },
}

#[derive(Debug)]
pub(crate) struct CommitImmediateRecoveryRequiredAuthority {
    approved: ApprovedCommitPreviewAuthority,
    failure: CommitImmediateRecheckFailureEvidence,
}

impl CommitImmediateRecoveryRequiredAuthority {
    pub(crate) fn failure(&self) -> &CommitImmediateRecheckFailureEvidence {
        &self.failure
    }

    pub(crate) fn approved_commit_digest(&self) -> &Sha256Digest {
        self.approved.0.commit_digest()
    }
}

#[derive(Debug)]
pub(crate) struct CommitScopedAtomicSafetyAuthority {
    approved: ApprovedCommitPreviewAuthority,
    completion: CommitImmediateRecheckCompletion,
    immediate_history_guard_evidence: PostMergeHistoryGuardEvidence,
    post_merge_repository_anchor: RepositoryAnchor,
    before_repository_cursor: RepositoryHistoryCursor,
    pre_command_target_snapshot: Box<CommitPreCommandTargetSnapshotAuthority>,
}

impl CommitScopedAtomicSafetyAuthority {
    pub(crate) fn before_repository_cursor(&self) -> &RepositoryHistoryCursor {
        &self.before_repository_cursor
    }

    pub(crate) fn post_merge_repository_anchor(&self) -> &RepositoryAnchor {
        &self.post_merge_repository_anchor
    }

    fn pre_command_target_snapshot(&self) -> &CommitPreCommandTargetSnapshotAuthority {
        &self.pre_command_target_snapshot
    }

    fn lineage_binding(&self) -> &CommitSafetyLineageBinding {
        &self
            .approved
            .0
            ._validated_lineage
            .as_ref()
            .expect("atomic commit requires the production preview lineage")
            .post_merge_guard
            .source
    }

    fn commit_safety_lineage_witness(&self) -> CommitSafetyLineageWitness {
        self.lineage_binding().witness()
    }

    fn owns_commit_safety_lineage_witness(&self, witness: &CommitSafetyLineageWitness) -> bool {
        self.lineage_binding().owns_witness(witness)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct RegisteredCommitStorageEvidence {
    record_revision: PositiveGeneration,
    record_digest: Sha256Digest,
    lease_generation: PositiveGeneration,
    lease_digest: Sha256Digest,
}

impl RegisteredCommitStorageEvidence {
    pub(crate) const fn new(
        record_revision: PositiveGeneration,
        record_digest: Sha256Digest,
        lease_generation: PositiveGeneration,
        lease_digest: Sha256Digest,
    ) -> Self {
        Self {
            record_revision,
            record_digest,
            lease_generation,
            lease_digest,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RegisteredCommitOperationIdentity {
    operation_id: OperationId,
    scope: OperationScope,
    operation: TaskOperationSelector,
    policy: DurableExecutionPolicy,
    canonical_input_digest: Sha256Digest,
}

/// Opaque current-record observation returned only through the invocation-bound
/// registered-operation lease. Its digests retain Task 16 storage semantics;
/// this slice neither reconstructs nor aliases those preimages.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct RegisteredCommitOperationObservation {
    identity: RegisteredCommitOperationIdentity,
    evidence: RegisteredCommitStorageEvidence,
}

impl RegisteredCommitOperationObservation {
    pub(crate) fn from_current_record(
        operation_id: OperationId,
        scope: OperationScope,
        operation: TaskOperationSelector,
        policy: DurableExecutionPolicy,
        canonical_input_digest: Sha256Digest,
        evidence: RegisteredCommitStorageEvidence,
    ) -> Self {
        Self {
            identity: RegisteredCommitOperationIdentity {
                operation_id,
                scope,
                operation,
                policy,
                canonical_input_digest,
            },
            evidence,
        }
    }
}

#[derive(Debug)]
struct CommitRegisteredOperationInvocationMarker;

#[derive(Debug)]
struct CommitRegisteredOperationInvocationCapability(
    Arc<CommitRegisteredOperationInvocationMarker>,
);

#[derive(Debug)]
struct CommitRegisteredOperationCompletionCapability(
    Arc<CommitRegisteredOperationInvocationMarker>,
);

impl CommitRegisteredOperationInvocationCapability {
    fn mint() -> Self {
        Self(Arc::new(CommitRegisteredOperationInvocationMarker))
    }

    fn completion(&self) -> CommitRegisteredOperationCompletionCapability {
        CommitRegisteredOperationCompletionCapability(Arc::clone(&self.0))
    }

    fn owns_completion(&self, completion: &CommitRegisteredOperationCompletionCapability) -> bool {
        Arc::ptr_eq(&self.0, &completion.0)
    }
}

/// Invocation-bound storage read. The real current-operation record and lease
/// remain owned by Task 16; this request asks its adapter to prove an exact
/// registered apply without redefining either digest preimage here.
#[derive(Debug)]
pub(crate) struct CommitRegisteredOperationRequest<'a> {
    scope: &'a CommitScopedAtomicSafetyAuthority,
    operation_scope: &'a OperationScope,
    canonical_input_digest: &'a Sha256Digest,
    invocation: &'a CommitRegisteredOperationInvocationCapability,
}

impl CommitRegisteredOperationRequest<'_> {
    pub(crate) fn apply_operation_id(&self) -> &OperationId {
        self.scope.approved.validated_apply_request().operation_id()
    }

    pub(crate) fn operation_scope(&self) -> &OperationScope {
        self.operation_scope
    }

    pub(crate) const fn operation(&self) -> TaskOperationSelector {
        TaskOperationSelector::RepositoryCommit(
            crate::domain::branched_development::contracts::selectors::RepositoryCommitSelector::new(
                RepositoryCommitSelectorVariant::Apply,
            ),
        )
    }

    pub(crate) const fn policy(&self) -> DurableExecutionPolicy {
        DurableExecutionPolicy::PreviewedJournaledEffect
    }

    pub(crate) fn canonical_input_digest(&self) -> &Sha256Digest {
        self.canonical_input_digest
    }

    pub(crate) fn commit_safety_lineage_witness(&self) -> CommitSafetyLineageWitness {
        self.scope.commit_safety_lineage_witness()
    }

    pub(crate) fn complete(
        self,
        lease: Box<dyn CommitRegisteredOperationLease>,
    ) -> CommitRegisteredOperationCompletion {
        CommitRegisteredOperationCompletion {
            completion: self.invocation.completion(),
            lease,
        }
    }
}

pub(crate) trait CommitRegisteredOperationLease {
    fn commit_safety_lineage_witness(&self) -> &CommitSafetyLineageWitness;
    fn binds(&self, request: &CommitRegisteredOperationRequest<'_>) -> bool;
    fn into_current_operation(self: Box<Self>) -> RegisteredCommitOperationObservation;
}

pub(crate) struct CommitRegisteredOperationCompletion {
    completion: CommitRegisteredOperationCompletionCapability,
    lease: Box<dyn CommitRegisteredOperationLease>,
}

impl fmt::Debug for CommitRegisteredOperationCompletion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CommitRegisteredOperationCompletion")
            .field("completion", &self.completion)
            .field("lease", &"<sealed registered-operation lease>")
            .finish()
    }
}

pub(crate) trait CommitRegisteredOperationPort {
    fn load_registered_commit_operation(
        &mut self,
        request: CommitRegisteredOperationRequest<'_>,
    ) -> Result<CommitRegisteredOperationCompletion, RepositoryResultContractError>;
}

/// Exact registered-operation authority produced only after the current
/// invocation, stable lineage witness and full binding have all succeeded.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct RegisteredCommitOperationAuthority {
    operation_id: OperationId,
    task_scope: RegisteredCommitTaskScopeAuthority,
    operation: TaskOperationSelector,
    policy: DurableExecutionPolicy,
    canonical_input_digest: Sha256Digest,
    evidence: RegisteredCommitStorageEvidence,
}

/// Opaque owner of the task container read from the invocation-bound current
/// durable-operation record. The caller-supplied scope is only a storage
/// locator hint and never becomes commit authority. Project/task identity is
/// closed here against retained preview lineage; the instance-to-terminal
/// envelope binding remains part of the Task 16 durable-terminal contract.
#[derive(Debug, PartialEq, Eq)]
struct RegisteredCommitTaskScopeAuthority(OperationScope);

impl RegisteredCommitTaskScopeAuthority {
    fn from_current_record(
        scope: OperationScope,
        expected_project_id: &ProjectId,
        expected_task_id: &TaskId,
    ) -> Option<Self> {
        match &scope {
            OperationScope::Task {
                project_id,
                task_id,
                ..
            } if project_id == expected_project_id && task_id == expected_task_id => {
                Some(Self(scope))
            }
            OperationScope::StartAttempt { .. } | OperationScope::Task { .. } => None,
        }
    }

    fn scope(&self) -> &OperationScope {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CommitEffectIntentDurableRecord {
    digest_kind: &'static str,
    operation_id: OperationId,
    operation_scope: OperationScope,
    operation: TaskOperationSelector,
    policy: DurableExecutionPolicy,
    canonical_input_digest: Sha256Digest,
    registered_record_revision: PositiveGeneration,
    registered_record_digest: Sha256Digest,
    registered_lease_generation: PositiveGeneration,
    registered_lease_digest: Sha256Digest,
    preallocated_commit_receipt_id: UnicaId,
    approved_commit_digest: Sha256Digest,
    immediate_history_guard_evidence_digest: Sha256Digest,
    before_repository_cursor: RepositoryHistoryCursor,
    atomic_commit_safety_capability_id: CapabilityRowId,
    pre_command_target_snapshot: CommitTargetStateSnapshotRecord,
    pre_command_target_snapshot_digest: Sha256Digest,
}

impl contract_digest_record_sealed::Sealed for CommitEffectIntentDurableRecord {}
impl ContractDigestRecord for CommitEffectIntentDurableRecord {}

#[derive(Debug, PartialEq, Eq)]
struct CommitEffectIntentRecord {
    durable_record: CommitEffectIntentDurableRecord,
    canonical_record_bytes: Vec<u8>,
    intent_record_digest: Sha256Digest,
}

impl CommitEffectIntentRecord {
    fn new(
        scope: &CommitScopedAtomicSafetyAuthority,
        registered: &RegisteredCommitOperationAuthority,
        preallocated_commit_receipt_id: UnicaId,
    ) -> Result<Self, RepositoryResultContractError> {
        let evidence = &registered.evidence;
        let approved_commit_digest = scope.approved.0.commit_digest().clone();
        let immediate_history_guard_evidence_digest = scope
            .immediate_history_guard_evidence
            .evidence_digest()
            .clone();
        let atomic_commit_safety_capability_id = scope
            .immediate_history_guard_evidence
            .atomic_commit_safety_capability_id()
            .clone();
        let pre_command_target_snapshot = scope.pre_command_target_snapshot();
        let durable_record = CommitEffectIntentDurableRecord {
            digest_kind: "unica.repository.commit.effect-intent.v1",
            operation_id: registered.operation_id.clone(),
            operation_scope: registered.task_scope.scope().clone(),
            operation: registered.operation.clone(),
            policy: registered.policy,
            canonical_input_digest: registered.canonical_input_digest.clone(),
            registered_record_revision: evidence.record_revision,
            registered_record_digest: evidence.record_digest.clone(),
            registered_lease_generation: evidence.lease_generation,
            registered_lease_digest: evidence.lease_digest.clone(),
            preallocated_commit_receipt_id,
            approved_commit_digest,
            immediate_history_guard_evidence_digest,
            before_repository_cursor: scope.before_repository_cursor.clone(),
            atomic_commit_safety_capability_id,
            pre_command_target_snapshot: pre_command_target_snapshot.record.clone(),
            pre_command_target_snapshot_digest: pre_command_target_snapshot.snapshot_digest.clone(),
        };
        let canonical_record_bytes = canonical_contract_encoding(&durable_record)
            .map_err(|_| RepositoryResultContractError("commit intent encoding failed"))?;
        let intent_record_digest =
            canonical_contract_digest(&durable_record, Some(&canonical_record_bytes))
                .map_err(|_| RepositoryResultContractError("commit intent digest failed"))?;
        Ok(Self {
            durable_record,
            canonical_record_bytes,
            intent_record_digest,
        })
    }
}

#[derive(Debug)]
struct CommitEffectIntentInvocationMarker;

#[derive(Debug)]
struct CommitEffectIntentInvocationCapability(Arc<CommitEffectIntentInvocationMarker>);

#[derive(Debug)]
struct CommitEffectIntentCompletionCapability(Arc<CommitEffectIntentInvocationMarker>);

impl CommitEffectIntentInvocationCapability {
    fn mint() -> Self {
        Self(Arc::new(CommitEffectIntentInvocationMarker))
    }

    fn completion(&self) -> CommitEffectIntentCompletionCapability {
        CommitEffectIntentCompletionCapability(Arc::clone(&self.0))
    }

    fn owns_completion(&self, completion: &CommitEffectIntentCompletionCapability) -> bool {
        Arc::ptr_eq(&self.0, &completion.0)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct CommitEffectIntentFsyncEvidence {
    intent_record_digest: Sha256Digest,
    fsync_receipt_id: UnicaId,
    fsync_capability_id: CapabilityRowId,
    effect_intent_digest: Sha256Digest,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CommitEffectIntentFsyncDigestRecord<'a> {
    digest_kind: &'static str,
    intent_record_digest: &'a Sha256Digest,
    fsync_receipt_id: &'a UnicaId,
    fsync_capability_id: &'a CapabilityRowId,
}

impl contract_digest_record_sealed::Sealed for CommitEffectIntentFsyncDigestRecord<'_> {}
impl ContractDigestRecord for CommitEffectIntentFsyncDigestRecord<'_> {}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct CommitEffectIntentNotWrittenCertificate {
    intent_record_digest: Sha256Digest,
    certificate_id: UnicaId,
    certificate_capability_id: CapabilityRowId,
    certificate_digest: Sha256Digest,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CommitEffectIntentNotWrittenDigestRecord<'a> {
    digest_kind: &'static str,
    intent_record_digest: &'a Sha256Digest,
    certificate_id: &'a UnicaId,
    certificate_capability_id: &'a CapabilityRowId,
}

impl contract_digest_record_sealed::Sealed for CommitEffectIntentNotWrittenDigestRecord<'_> {}
impl ContractDigestRecord for CommitEffectIntentNotWrittenDigestRecord<'_> {}

/// Invocation-bound view passed to the intent journal. It is the only mint for
/// fsync and proven-not-written evidence bound to this exact Ready authority.
#[derive(Debug)]
pub(crate) struct CommitEffectIntentRequest<'a> {
    scope: &'a CommitScopedAtomicSafetyAuthority,
    registered: &'a RegisteredCommitOperationAuthority,
    record: &'a CommitEffectIntentRecord,
    invocation: &'a CommitEffectIntentInvocationCapability,
}

impl CommitEffectIntentRequest<'_> {
    pub(crate) fn apply_operation_id(&self) -> &OperationId {
        &self.record.durable_record.operation_id
    }

    pub(crate) fn operation_scope(&self) -> &OperationScope {
        &self.record.durable_record.operation_scope
    }

    pub(crate) const fn registered_record_revision(&self) -> PositiveGeneration {
        self.record.durable_record.registered_record_revision
    }

    pub(crate) const fn registered_lease_generation(&self) -> PositiveGeneration {
        self.record.durable_record.registered_lease_generation
    }

    pub(crate) fn intent_record_digest(&self) -> &Sha256Digest {
        &self.record.intent_record_digest
    }

    pub(crate) fn registered_record_digest(&self) -> &Sha256Digest {
        &self.record.durable_record.registered_record_digest
    }

    pub(crate) fn registered_lease_digest(&self) -> &Sha256Digest {
        &self.record.durable_record.registered_lease_digest
    }

    pub(crate) fn canonical_input_digest(&self) -> &Sha256Digest {
        &self.record.durable_record.canonical_input_digest
    }

    pub(crate) fn approved_commit_digest(&self) -> &Sha256Digest {
        &self.record.durable_record.approved_commit_digest
    }

    pub(crate) fn preallocated_commit_receipt_id(&self) -> &UnicaId {
        &self.record.durable_record.preallocated_commit_receipt_id
    }

    pub(crate) fn before_repository_cursor(&self) -> &RepositoryHistoryCursor {
        &self.record.durable_record.before_repository_cursor
    }

    pub(crate) fn atomic_commit_safety_capability_id(&self) -> &CapabilityRowId {
        &self
            .record
            .durable_record
            .atomic_commit_safety_capability_id
    }

    pub(crate) fn durable_record(&self) -> &CommitEffectIntentDurableRecord {
        &self.record.durable_record
    }

    pub(crate) fn canonical_record_bytes(&self) -> &[u8] {
        &self.record.canonical_record_bytes
    }

    pub(crate) fn commit_safety_lineage_witness(&self) -> CommitSafetyLineageWitness {
        self.scope.commit_safety_lineage_witness()
    }

    pub(crate) fn observe_fsync(
        &self,
        fsync_receipt_id: UnicaId,
        fsync_capability_id: CapabilityRowId,
    ) -> Result<CommitEffectIntentFsyncEvidence, RepositoryResultContractError> {
        let effect_intent_digest = result_digest(
            &CommitEffectIntentFsyncDigestRecord {
                digest_kind: "unica.repository.commit.effect-intent-fsync.v1",
                intent_record_digest: &self.record.intent_record_digest,
                fsync_receipt_id: &fsync_receipt_id,
                fsync_capability_id: &fsync_capability_id,
            },
            "commit effect-intent fsync digest failed",
        )?;
        Ok(CommitEffectIntentFsyncEvidence {
            intent_record_digest: self.record.intent_record_digest.clone(),
            fsync_receipt_id,
            fsync_capability_id,
            effect_intent_digest,
        })
    }

    pub(crate) fn observe_proven_not_written(
        &self,
        certificate_id: UnicaId,
        certificate_capability_id: CapabilityRowId,
    ) -> Result<CommitEffectIntentNotWrittenCertificate, RepositoryResultContractError> {
        let certificate_digest = result_digest(
            &CommitEffectIntentNotWrittenDigestRecord {
                digest_kind: "unica.repository.commit.effect-intent-not-written.v1",
                intent_record_digest: &self.record.intent_record_digest,
                certificate_id: &certificate_id,
                certificate_capability_id: &certificate_capability_id,
            },
            "commit effect-intent not-written digest failed",
        )?;
        Ok(CommitEffectIntentNotWrittenCertificate {
            intent_record_digest: self.record.intent_record_digest.clone(),
            certificate_id,
            certificate_capability_id,
            certificate_digest,
        })
    }

    pub(crate) fn complete_written(
        self,
        lease: Box<dyn CommitEffectIntentWrittenLease>,
    ) -> CommitEffectIntentCompletion {
        CommitEffectIntentCompletion {
            completion: self.invocation.completion(),
            disposition: CommitEffectIntentCompletionDisposition::Written(lease),
        }
    }

    pub(crate) fn complete_proven_not_written(
        self,
        lease: Box<dyn CommitEffectIntentProvenNotWrittenLease>,
    ) -> CommitEffectIntentCompletion {
        CommitEffectIntentCompletion {
            completion: self.invocation.completion(),
            disposition: CommitEffectIntentCompletionDisposition::ProvenNotWritten(lease),
        }
    }
}

pub(crate) trait CommitEffectIntentWrittenLease {
    fn commit_safety_lineage_witness(&self) -> &CommitSafetyLineageWitness;
    fn binds(&self, request: &CommitEffectIntentRequest<'_>) -> bool;
    fn into_fsync_evidence(self: Box<Self>) -> CommitEffectIntentFsyncEvidence;
}

pub(crate) trait CommitEffectIntentProvenNotWrittenLease {
    fn commit_safety_lineage_witness(&self) -> &CommitSafetyLineageWitness;
    fn binds(&self, request: &CommitEffectIntentRequest<'_>) -> bool;
    fn into_certificate(self: Box<Self>) -> CommitEffectIntentNotWrittenCertificate;
}

enum CommitEffectIntentCompletionDisposition {
    Written(Box<dyn CommitEffectIntentWrittenLease>),
    ProvenNotWritten(Box<dyn CommitEffectIntentProvenNotWrittenLease>),
}

pub(crate) struct CommitEffectIntentCompletion {
    completion: CommitEffectIntentCompletionCapability,
    disposition: CommitEffectIntentCompletionDisposition,
}

impl fmt::Debug for CommitEffectIntentCompletion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CommitEffectIntentCompletion")
            .field("completion", &self.completion)
            .field("disposition", &"<sealed intent completion>")
            .finish()
    }
}

pub(crate) trait CommitEffectIntentPort {
    fn write_and_fsync_commit_intent(
        &mut self,
        request: CommitEffectIntentRequest<'_>,
    ) -> Result<CommitEffectIntentCompletion, RepositoryResultContractError>;
}

#[derive(Debug)]
struct CommitWrittenEffectIntentAuthority {
    registered: RegisteredCommitOperationAuthority,
    record: CommitEffectIntentRecord,
    fsync: CommitEffectIntentFsyncEvidence,
}

#[derive(Debug)]
pub(crate) struct CommitPreIntentBlockedAuthority {
    scope: Box<CommitScopedAtomicSafetyAuthority>,
    registered: Option<Box<RegisteredCommitOperationAuthority>>,
    record: Option<Box<CommitEffectIntentRecord>>,
    certificate: Option<Box<CommitEffectIntentNotWrittenCertificate>>,
}

#[derive(Debug)]
pub(crate) struct CommitPreIntentFreshRecheckAuthority {
    recheck: CommitImmediateRecheckOutcome,
    _registered: Option<Box<RegisteredCommitOperationAuthority>>,
    _record: Option<Box<CommitEffectIntentRecord>>,
    _certificate: Option<Box<CommitEffectIntentNotWrittenCertificate>>,
}

impl CommitPreIntentBlockedAuthority {
    fn new(
        scope: CommitScopedAtomicSafetyAuthority,
        registered: Option<RegisteredCommitOperationAuthority>,
        record: Option<CommitEffectIntentRecord>,
        certificate: Option<CommitEffectIntentNotWrittenCertificate>,
    ) -> Self {
        Self {
            scope: Box::new(scope),
            registered: registered.map(Box::new),
            record: record.map(Box::new),
            certificate: certificate.map(Box::new),
        }
    }

    pub(crate) fn approved_commit_digest(&self) -> &Sha256Digest {
        self.scope.approved.0.commit_digest()
    }

    /// Consumes the stale Ready scope and obtains a new immediate observation.
    /// The old lease cannot be reused; attempted registration/intent evidence
    /// remains owned beside the fresh recheck outcome.
    pub(crate) fn recheck_with_fresh_observation(
        self,
        port: &mut dyn CommitImmediateRecheckPort,
    ) -> CommitPreIntentFreshRecheckAuthority {
        let Self {
            scope,
            registered,
            record,
            certificate,
        } = self;
        let CommitScopedAtomicSafetyAuthority { approved, .. } = *scope;
        CommitPreIntentFreshRecheckAuthority {
            recheck: approved.recheck_before_commit_intent(port),
            _registered: registered,
            _record: record,
            _certificate: certificate,
        }
    }
}

impl CommitPreIntentFreshRecheckAuthority {
    pub(crate) fn into_recheck(self) -> CommitImmediateRecheckOutcome {
        self.recheck
    }
}

#[derive(Debug)]
enum CommitRejectedIntentEvidence {
    Fsync(CommitEffectIntentFsyncEvidence),
    ProvenNotWritten(CommitEffectIntentNotWrittenCertificate),
}

#[derive(Debug)]
enum CommitAmbiguousSource {
    IntentPort {
        scope: Box<CommitScopedAtomicSafetyAuthority>,
        registered: RegisteredCommitOperationAuthority,
        record: CommitEffectIntentRecord,
        invocation: CommitEffectIntentInvocationCapability,
        error: RepositoryResultContractError,
    },
    IntentCompletion {
        scope: Box<CommitScopedAtomicSafetyAuthority>,
        registered: RegisteredCommitOperationAuthority,
        record: CommitEffectIntentRecord,
        invocation: CommitEffectIntentInvocationCapability,
        completion: CommitEffectIntentCompletion,
    },
    IntentEvidence {
        scope: Box<CommitScopedAtomicSafetyAuthority>,
        registered: RegisteredCommitOperationAuthority,
        record: CommitEffectIntentRecord,
        invocation: CommitEffectIntentInvocationCapability,
        evidence: CommitRejectedIntentEvidence,
    },
    AtomicPort {
        source: Box<CommitAtomicCommitSource>,
        error: RepositoryResultContractError,
    },
    AtomicCompletion {
        source: Box<CommitAtomicCommitSource>,
        completion: CommitAtomicCommitCompletion,
    },
    AtomicObservation {
        source: Box<CommitAtomicCommitSource>,
        observation: Box<CommitAtomicCommitObservation>,
    },
}

impl CommitAmbiguousSource {
    fn intent_port(
        scope: CommitScopedAtomicSafetyAuthority,
        registered: RegisteredCommitOperationAuthority,
        record: CommitEffectIntentRecord,
        invocation: CommitEffectIntentInvocationCapability,
        error: RepositoryResultContractError,
    ) -> Box<Self> {
        Box::new(Self::IntentPort {
            scope: Box::new(scope),
            registered,
            record,
            invocation,
            error,
        })
    }

    fn intent_completion(
        scope: CommitScopedAtomicSafetyAuthority,
        registered: RegisteredCommitOperationAuthority,
        record: CommitEffectIntentRecord,
        invocation: CommitEffectIntentInvocationCapability,
        completion: CommitEffectIntentCompletion,
    ) -> Box<Self> {
        Box::new(Self::IntentCompletion {
            scope: Box::new(scope),
            registered,
            record,
            invocation,
            completion,
        })
    }

    fn intent_evidence(
        scope: CommitScopedAtomicSafetyAuthority,
        registered: RegisteredCommitOperationAuthority,
        record: CommitEffectIntentRecord,
        invocation: CommitEffectIntentInvocationCapability,
        evidence: CommitRejectedIntentEvidence,
    ) -> Box<Self> {
        Box::new(Self::IntentEvidence {
            scope: Box::new(scope),
            registered,
            record,
            invocation,
            evidence,
        })
    }

    fn atomic_port(
        source: CommitAtomicCommitSource,
        error: RepositoryResultContractError,
    ) -> Box<Self> {
        Box::new(Self::AtomicPort {
            source: Box::new(source),
            error,
        })
    }

    fn atomic_completion(
        source: CommitAtomicCommitSource,
        completion: CommitAtomicCommitCompletion,
    ) -> Box<Self> {
        Box::new(Self::AtomicCompletion {
            source: Box::new(source),
            completion,
        })
    }

    fn atomic_observation(
        source: CommitAtomicCommitSource,
        observation: CommitAtomicCommitObservation,
    ) -> Box<Self> {
        Box::new(Self::AtomicObservation {
            source: Box::new(source),
            observation: Box::new(observation),
        })
    }
}

#[derive(Debug)]
pub(crate) struct CommitAmbiguousAuthority {
    source: Box<CommitAmbiguousSource>,
}

/// Opaque consuming hand-off for Task 16's observe-only repositoryCommit
/// recovery builder. It owns the complete pre/post-intent source enum rather
/// than reconstructing authority from published digests.
#[derive(Debug)]
pub(crate) struct CommitAmbiguousRecoverySourceAuthority(Box<CommitAmbiguousSource>);

impl CommitAmbiguousAuthority {
    pub(crate) fn approved_commit_digest(&self) -> &Sha256Digest {
        match self.source.as_ref() {
            CommitAmbiguousSource::IntentPort { scope, .. }
            | CommitAmbiguousSource::IntentCompletion { scope, .. }
            | CommitAmbiguousSource::IntentEvidence { scope, .. } => {
                scope.approved.0.commit_digest()
            }
            CommitAmbiguousSource::AtomicPort { source, .. }
            | CommitAmbiguousSource::AtomicCompletion { source, .. }
            | CommitAmbiguousSource::AtomicObservation { source, .. } => {
                source.approved.0.commit_digest()
            }
        }
    }

    pub(crate) fn into_recovery_source(self) -> CommitAmbiguousRecoverySourceAuthority {
        CommitAmbiguousRecoverySourceAuthority(self.source)
    }
}

impl CommitAmbiguousRecoverySourceAuthority {
    pub(crate) fn approved_commit_digest(&self) -> &Sha256Digest {
        match self.0.as_ref() {
            CommitAmbiguousSource::IntentPort { scope, .. }
            | CommitAmbiguousSource::IntentCompletion { scope, .. }
            | CommitAmbiguousSource::IntentEvidence { scope, .. } => {
                scope.approved.0.commit_digest()
            }
            CommitAmbiguousSource::AtomicPort { source, .. }
            | CommitAmbiguousSource::AtomicCompletion { source, .. }
            | CommitAmbiguousSource::AtomicObservation { source, .. } => {
                source.approved.0.commit_digest()
            }
        }
    }

    pub(crate) fn effect_intent_record_digest(&self) -> &Sha256Digest {
        match self.0.as_ref() {
            CommitAmbiguousSource::IntentPort { record, .. }
            | CommitAmbiguousSource::IntentCompletion { record, .. }
            | CommitAmbiguousSource::IntentEvidence { record, .. } => &record.intent_record_digest,
            CommitAmbiguousSource::AtomicPort { source, .. }
            | CommitAmbiguousSource::AtomicCompletion { source, .. }
            | CommitAmbiguousSource::AtomicObservation { source, .. } => {
                &source.written_intent.record.intent_record_digest
            }
        }
    }
}

#[derive(Debug)]
pub(crate) enum CommitEffectIntentOutcome {
    PreIntentBlocked(CommitPreIntentBlockedAuthority),
    PostIntent(CommitExactOnceOutcome),
}

#[derive(Debug)]
struct CommitAtomicCommitSource {
    approved: ApprovedCommitPreviewAuthority,
    immediate_history_guard_evidence: PostMergeHistoryGuardEvidence,
    post_merge_repository_anchor: RepositoryAnchor,
    before_repository_cursor: RepositoryHistoryCursor,
    pre_command_target_snapshot: Box<CommitPreCommandTargetSnapshotAuthority>,
    written_intent: CommitWrittenEffectIntentAuthority,
}

impl CommitAtomicCommitSource {
    fn lineage_binding(&self) -> &CommitSafetyLineageBinding {
        &self
            .approved
            .0
            ._validated_lineage
            .as_ref()
            .expect("atomic commit requires the production preview lineage")
            .post_merge_guard
            .source
    }

    fn commit_safety_lineage_witness(&self) -> CommitSafetyLineageWitness {
        self.lineage_binding().witness()
    }

    fn owns_commit_safety_lineage_witness(&self, witness: &CommitSafetyLineageWitness) -> bool {
        self.lineage_binding().owns_witness(witness)
    }
}

fn retained_commit_lock_lineage_record(
    source: &CommitAtomicCommitSource,
) -> CommitRetainedLockLineageRecord {
    CommitRetainedLockLineageRecord {
        digest_kind: "unica.repository.commit.retained-lock-lineage.v1",
        lock_set_id: source.lineage_binding().lineage().lock_set_id().clone(),
        lock_set_digest: source.lineage_binding().lineage().lock_set_digest().clone(),
        journaled_lock_receipts: source
            .lineage_binding()
            .lineage()
            .journaled_lock_receipts()
            .to_vec(),
    }
}

#[derive(Debug)]
struct CommitAtomicCommitInvocationMarker;

#[derive(Debug)]
struct CommitAtomicCommitInvocationCapability(Arc<CommitAtomicCommitInvocationMarker>);

#[derive(Debug)]
struct CommitAtomicCommitCompletionCapability(Arc<CommitAtomicCommitInvocationMarker>);

impl CommitAtomicCommitInvocationCapability {
    fn mint() -> Self {
        Self(Arc::new(CommitAtomicCommitInvocationMarker))
    }

    fn completion(&self) -> CommitAtomicCommitCompletionCapability {
        CommitAtomicCommitCompletionCapability(Arc::clone(&self.0))
    }

    fn owns_completion(&self, completion: &CommitAtomicCommitCompletionCapability) -> bool {
        Arc::ptr_eq(&self.0, &completion.0)
    }

    fn object_history_binding_witness(&self) -> CommitObjectHistoryBindingWitness {
        CommitObjectHistoryBindingWitness(Arc::clone(&self.0))
    }

    fn owns_object_history_binding_witness(
        &self,
        witness: &CommitObjectHistoryBindingWitness,
    ) -> bool {
        Arc::ptr_eq(&self.0, &witness.0)
    }

    fn child_witness(&self) -> CommitAtomicInvocationChildWitness {
        CommitAtomicInvocationChildWitness(Arc::clone(&self.0))
    }

    fn owns_child_witness(&self, witness: &CommitAtomicInvocationChildWitness) -> bool {
        Arc::ptr_eq(&self.0, &witness.0)
    }
}

/// Opaque child witness of one exact atomic invocation. Equality is pointer
/// identity, never equality of replayable version/digest/capability scalars.
#[derive(Debug, Clone)]
pub(crate) struct CommitObjectHistoryBindingWitness(Arc<CommitAtomicCommitInvocationMarker>);

impl CommitObjectHistoryBindingWitness {
    pub(crate) fn same_invocation(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl PartialEq for CommitObjectHistoryBindingWitness {
    fn eq(&self, other: &Self) -> bool {
        self.same_invocation(other)
    }
}

impl Eq for CommitObjectHistoryBindingWitness {}

/// Opaque child identity for a terminal zero-effect observation minted by one
/// exact atomic invocation.  Equal replayable snapshot scalars cannot replace
/// this pointer-bound proof.
#[derive(Debug, Clone)]
struct CommitAtomicInvocationChildWitness(Arc<CommitAtomicCommitInvocationMarker>);

impl PartialEq for CommitAtomicInvocationChildWitness {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for CommitAtomicInvocationChildWitness {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct CommitRetainedLockLineageRecord {
    digest_kind: &'static str,
    lock_set_id: UnicaId,
    lock_set_digest: Sha256Digest,
    journaled_lock_receipts: Vec<JournaledRepositoryLock>,
}

impl contract_digest_record_sealed::Sealed for CommitRetainedLockLineageRecord {}
impl ContractDigestRecord for CommitRetainedLockLineageRecord {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
enum CommitPostCommandLockStatusRecord {
    Held {
        current_full_lock_inventory: CanonicalRepositoryTargets,
    },
    VerifiedReleased {
        current_full_lock_inventory: CanonicalRepositoryTargets,
        release_capability_id: CapabilityRowId,
        /// Exact acquisition-order receipt projection identifying the locks
        /// whose release was observed. This is not a release-step sequence;
        /// commit has one atomic release invariant and canonical target lists.
        released_acquisition_receipts: Vec<JournaledRepositoryLock>,
        released_objects: CanonicalRepositoryTargets,
        released_guard_locks: CanonicalRepositoryTargets,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct CommitPostCommandLockEvidenceRecord {
    digest_kind: &'static str,
    effect_intent_digest: Sha256Digest,
    preallocated_commit_receipt_id: UnicaId,
    retained_lock_lineage: CommitRetainedLockLineageRecord,
    retained_lock_lineage_digest: Sha256Digest,
    status: CommitPostCommandLockStatusRecord,
}

impl contract_digest_record_sealed::Sealed for CommitPostCommandLockEvidenceRecord {}
impl ContractDigestRecord for CommitPostCommandLockEvidenceRecord {}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct CommitPostCommandLockAuthority {
    invocation_witness: CommitAtomicInvocationChildWitness,
    record: CommitPostCommandLockEvidenceRecord,
    evidence_digest: Sha256Digest,
}

impl CommitPostCommandLockAuthority {
    fn released_projection(
        &self,
    ) -> Option<(&CanonicalRepositoryTargets, &CanonicalRepositoryTargets)> {
        match &self.record.status {
            CommitPostCommandLockStatusRecord::VerifiedReleased {
                released_objects,
                released_guard_locks,
                ..
            } => Some((released_objects, released_guard_locks)),
            CommitPostCommandLockStatusRecord::Held { .. } => None,
        }
    }

    fn into_released_projection(
        self,
    ) -> Option<(CanonicalRepositoryTargets, CanonicalRepositoryTargets)> {
        match self.record.status {
            CommitPostCommandLockStatusRecord::VerifiedReleased {
                released_objects,
                released_guard_locks,
                ..
            } => Some((released_objects, released_guard_locks)),
            CommitPostCommandLockStatusRecord::Held { .. } => None,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct CommitHeldLocksObservationInput {
    lock_set_id: UnicaId,
    journaled_lock_receipts: Vec<JournaledRepositoryLock>,
    current_full_lock_inventory: CanonicalRepositoryTargets,
}

impl CommitHeldLocksObservationInput {
    pub(crate) const fn from_repository_adapter(
        lock_set_id: UnicaId,
        journaled_lock_receipts: Vec<JournaledRepositoryLock>,
        current_full_lock_inventory: CanonicalRepositoryTargets,
    ) -> Self {
        Self {
            lock_set_id,
            journaled_lock_receipts,
            current_full_lock_inventory,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct CommitReleasedLocksObservationInput {
    lock_set_id: UnicaId,
    released_acquisition_receipts: Vec<JournaledRepositoryLock>,
    current_full_lock_inventory: CanonicalRepositoryTargets,
    release_capability_id: CapabilityRowId,
}

impl CommitReleasedLocksObservationInput {
    pub(crate) const fn from_repository_adapter(
        lock_set_id: UnicaId,
        released_acquisition_receipts: Vec<JournaledRepositoryLock>,
        current_full_lock_inventory: CanonicalRepositoryTargets,
        release_capability_id: CapabilityRowId,
    ) -> Self {
        Self {
            lock_set_id,
            released_acquisition_receipts,
            current_full_lock_inventory,
            release_capability_id,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum CommitLockReleaseObservation {
    Verified(Box<CommitPostCommandLockAuthority>),
    Unknown,
}

/// Minimal sealed-by-construction binding consumed by the taskCommit-specific
/// history resolver. Production implementations can only be minted from the
/// validated pre-commit lineage or from this invocation's validated atomic
/// object observation.
mod commit_object_history_binding_sealed {
    pub trait Sealed {}
}

pub(crate) trait CommitObjectHistoryBinding:
    commit_object_history_binding_sealed::Sealed
{
    fn object_history_binding_witness(&self) -> &CommitObjectHistoryBindingWitness;
    fn repository_version(&self) -> &RepositoryVersion;
    fn committed_objects_digest(&self) -> &Sha256Digest;
    fn atomic_commit_safety_capability_id(&self) -> &CapabilityRowId;
}

impl commit_object_history_binding_sealed::Sealed for CommitCommittedCoreObservation {}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct CommitCommittedCoreObservationInput {
    commit_receipt_id: UnicaId,
    repository_version: RepositoryVersion,
    committed_objects: CommittedRepositoryObjects,
    committed_objects_digest: Sha256Digest,
    atomic_commit_safety_capability_id: CapabilityRowId,
}

impl CommitCommittedCoreObservationInput {
    pub(crate) const fn from_atomic_adapter(
        commit_receipt_id: UnicaId,
        repository_version: RepositoryVersion,
        committed_objects: CommittedRepositoryObjects,
        committed_objects_digest: Sha256Digest,
        atomic_commit_safety_capability_id: CapabilityRowId,
    ) -> Self {
        Self {
            commit_receipt_id,
            repository_version,
            committed_objects,
            committed_objects_digest,
            atomic_commit_safety_capability_id,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct CommitCommittedCoreObservation {
    object_history_binding_witness: CommitObjectHistoryBindingWitness,
    commit_receipt_id: UnicaId,
    repository_version: RepositoryVersion,
    committed_objects: CommittedRepositoryObjects,
    committed_objects_digest: Sha256Digest,
    atomic_commit_safety_capability_id: CapabilityRowId,
}

impl CommitCommittedCoreObservation {}

impl CommitObjectHistoryBinding for CommitCommittedCoreObservation {
    fn object_history_binding_witness(&self) -> &CommitObjectHistoryBindingWitness {
        &self.object_history_binding_witness
    }

    fn repository_version(&self) -> &RepositoryVersion {
        &self.repository_version
    }

    fn committed_objects_digest(&self) -> &Sha256Digest {
        &self.committed_objects_digest
    }

    fn atomic_commit_safety_capability_id(&self) -> &CapabilityRowId {
        &self.atomic_commit_safety_capability_id
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct CommitCommittedHistoryObservation {
    post_commit_history_partition: ValidatedTaskCommitHistoryPartition,
    task_version_anchor: RepositoryAnchor,
    terminal_repository_anchor: RepositoryAnchor,
}

impl CommitCommittedHistoryObservation {
    #[cfg(test)]
    pub(crate) const fn new(
        post_commit_history_partition: ValidatedTaskCommitHistoryPartition,
        task_version_anchor: RepositoryAnchor,
        terminal_repository_anchor: RepositoryAnchor,
    ) -> Self {
        Self {
            post_commit_history_partition,
            task_version_anchor,
            terminal_repository_anchor,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct CommitCommittedObservationInput {
    core: CommitCommittedCoreObservation,
    history: CommitCommittedHistoryObservation,
    release: CommitLockReleaseObservation,
}

impl CommitCommittedObservationInput {
    pub(crate) const fn from_validated_atomic_observation(
        core: CommitCommittedCoreObservation,
        history: CommitCommittedHistoryObservation,
        release: CommitLockReleaseObservation,
    ) -> Self {
        Self {
            core,
            history,
            release,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct CommitTerminalTargetSnapshotAuthority {
    invocation_witness: CommitAtomicInvocationChildWitness,
    record: CommitTargetStateSnapshotRecord,
    snapshot_digest: Sha256Digest,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum CommitZeroEffectLockState {
    Held(CommitPostCommandLockAuthority),
    VerifiedReleased(CommitPostCommandLockAuthority),
    Unknown,
}

#[derive(Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
enum CommitZeroEffectLockStateDigestRecord<'a> {
    Held {
        lock_evidence_digest: &'a Sha256Digest,
    },
    VerifiedReleased {
        lock_evidence_digest: &'a Sha256Digest,
    },
    Unknown,
}

impl contract_digest_record_sealed::Sealed for CommitZeroEffectLockStateDigestRecord<'_> {}
impl ContractDigestRecord for CommitZeroEffectLockStateDigestRecord<'_> {}

fn zero_effect_lock_state_digest(
    state: &CommitZeroEffectLockState,
) -> Result<Sha256Digest, RepositoryResultContractError> {
    let record = match state {
        CommitZeroEffectLockState::Held(authority) => CommitZeroEffectLockStateDigestRecord::Held {
            lock_evidence_digest: &authority.evidence_digest,
        },
        CommitZeroEffectLockState::VerifiedReleased(authority) => {
            CommitZeroEffectLockStateDigestRecord::VerifiedReleased {
                lock_evidence_digest: &authority.evidence_digest,
            }
        }
        CommitZeroEffectLockState::Unknown => CommitZeroEffectLockStateDigestRecord::Unknown,
    };
    result_digest(&record, "commit zero-effect lock-state digest failed")
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct CommitZeroEffectCertificate {
    effect_intent_digest: Sha256Digest,
    pre_command_target_snapshot_digest: Sha256Digest,
    terminal_target_snapshot_digest: Sha256Digest,
    post_command_history_partition_digest: Sha256Digest,
    terminal_repository_anchor_digest: Sha256Digest,
    lock_state_digest: Sha256Digest,
    certificate_id: UnicaId,
    certificate_capability_id: CapabilityRowId,
    certificate_digest: Sha256Digest,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CommitZeroEffectCertificateDigestRecord<'a> {
    digest_kind: &'static str,
    effect_intent_digest: &'a Sha256Digest,
    pre_command_target_snapshot_digest: &'a Sha256Digest,
    terminal_target_snapshot_digest: &'a Sha256Digest,
    post_command_history_partition_digest: &'a Sha256Digest,
    terminal_repository_anchor_digest: &'a Sha256Digest,
    lock_state_digest: &'a Sha256Digest,
    certificate_id: &'a UnicaId,
    certificate_capability_id: &'a CapabilityRowId,
}

impl contract_digest_record_sealed::Sealed for CommitZeroEffectCertificateDigestRecord<'_> {}
impl ContractDigestRecord for CommitZeroEffectCertificateDigestRecord<'_> {}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct CommitProvenZeroEffectObservationInput {
    certificate: CommitZeroEffectCertificate,
    post_command_history_partition: ValidatedRepositoryHistoryPartition,
    terminal_target_snapshot: CommitTerminalTargetSnapshotAuthority,
    lock_state: CommitZeroEffectLockState,
}

impl CommitProvenZeroEffectObservationInput {
    pub(crate) const fn from_validated_atomic_observation(
        certificate: CommitZeroEffectCertificate,
        post_command_history_partition: ValidatedRepositoryHistoryPartition,
        terminal_target_snapshot: CommitTerminalTargetSnapshotAuthority,
        lock_state: CommitZeroEffectLockState,
    ) -> Self {
        Self {
            certificate,
            post_command_history_partition,
            terminal_target_snapshot,
            lock_state,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum CommitAtomicCommitObservation {
    Committed(Box<CommitCommittedObservationInput>),
    ProvenZeroEffect(Box<CommitProvenZeroEffectObservationInput>),
    Ambiguous,
}

/// Exact request consumed by the same immediate-recheck lease that supplied
/// Ready. There is deliberately no separate physical commit port.
#[derive(Debug)]
pub(crate) struct CommitAtomicCommitRequest<'a> {
    source: &'a CommitAtomicCommitSource,
    invocation: &'a CommitAtomicCommitInvocationCapability,
}

impl CommitAtomicCommitRequest<'_> {
    pub(crate) fn commit_safety_lineage_witness(&self) -> CommitSafetyLineageWitness {
        self.source.commit_safety_lineage_witness()
    }

    pub(crate) fn approved_commit_digest(&self) -> &Sha256Digest {
        self.source.approved.0.commit_digest()
    }

    pub(crate) fn apply_operation_id(&self) -> &OperationId {
        &self
            .source
            .written_intent
            .record
            .durable_record
            .operation_id
    }

    pub(crate) fn operation_scope(&self) -> &OperationScope {
        &self
            .source
            .written_intent
            .record
            .durable_record
            .operation_scope
    }

    pub(crate) fn effect_intent_digest(&self) -> &Sha256Digest {
        &self.source.written_intent.fsync.effect_intent_digest
    }

    pub(crate) fn preallocated_commit_receipt_id(&self) -> &UnicaId {
        &self
            .source
            .written_intent
            .record
            .durable_record
            .preallocated_commit_receipt_id
    }

    pub(crate) fn before_repository_cursor(&self) -> &RepositoryHistoryCursor {
        &self.source.before_repository_cursor
    }

    pub(crate) fn post_merge_repository_anchor(&self) -> &RepositoryAnchor {
        &self.source.post_merge_repository_anchor
    }

    pub(crate) fn atomic_commit_safety_capability_id(&self) -> &CapabilityRowId {
        &self
            .source
            .written_intent
            .record
            .durable_record
            .atomic_commit_safety_capability_id
    }

    pub(crate) fn exact_objects(&self) -> &CommitExactObjects {
        &self.source.approved.0.record.exact_objects
    }

    /// Frozen rendered comment that was included in the approved preview and
    /// its digest.  The adapter cannot supply or re-render a comment here.
    pub(crate) fn rendered_comment(&self) -> &Comment {
        &self.source.approved.0.record.comment
    }

    pub(crate) fn exact_object_refs(
        &self,
    ) -> impl ExactSizeIterator<Item = CommitExactObjectRef<'_>> {
        self.exact_objects().iter()
    }

    pub(crate) fn pre_command_target_snapshot_digest(&self) -> &Sha256Digest {
        &self.source.pre_command_target_snapshot.snapshot_digest
    }

    pub(crate) fn lock_set_id(&self) -> &UnicaId {
        self.source.lineage_binding().lineage().lock_set_id()
    }

    pub(crate) fn lock_set_digest(&self) -> &Sha256Digest {
        self.source.lineage_binding().lineage().lock_set_digest()
    }

    pub(crate) fn journaled_lock_receipts(&self) -> &[JournaledRepositoryLock] {
        self.source
            .lineage_binding()
            .lineage()
            .journaled_lock_receipts()
    }

    fn retained_lock_lineage_record(&self) -> CommitRetainedLockLineageRecord {
        retained_commit_lock_lineage_record(self.source)
    }

    fn mint_post_command_lock_authority(
        &self,
        status: CommitPostCommandLockStatusRecord,
    ) -> Result<CommitPostCommandLockAuthority, RepositoryResultContractError> {
        let retained_lock_lineage = self.retained_lock_lineage_record();
        let retained_lock_lineage_digest = result_digest(
            &retained_lock_lineage,
            "commit retained-lock lineage digest failed",
        )?;
        let record = CommitPostCommandLockEvidenceRecord {
            digest_kind: "unica.repository.commit.post-command-lock-evidence.v1",
            effect_intent_digest: self.effect_intent_digest().clone(),
            preallocated_commit_receipt_id: self.preallocated_commit_receipt_id().clone(),
            retained_lock_lineage,
            retained_lock_lineage_digest,
            status,
        };
        let evidence_digest =
            result_digest(&record, "commit post-command lock evidence digest failed")?;
        Ok(CommitPostCommandLockAuthority {
            invocation_witness: self.invocation.child_witness(),
            record,
            evidence_digest,
        })
    }

    pub(crate) fn observe_locks_held(
        &self,
        observation: CommitHeldLocksObservationInput,
    ) -> Result<CommitPostCommandLockAuthority, RepositoryResultContractError> {
        let expected_inventory = self.full_lock_inventory()?;
        if observation.lock_set_id != *self.lock_set_id()
            || observation.journaled_lock_receipts != self.journaled_lock_receipts()
            || observation.current_full_lock_inventory != expected_inventory
        {
            return Err(RepositoryResultContractError(
                "held-lock observation differs from retained acquisition lineage",
            ));
        }
        self.mint_post_command_lock_authority(CommitPostCommandLockStatusRecord::Held {
            current_full_lock_inventory: observation.current_full_lock_inventory,
        })
    }

    pub(crate) fn observe_locks_released(
        &self,
        observation: CommitReleasedLocksObservationInput,
    ) -> Result<CommitPostCommandLockAuthority, RepositoryResultContractError> {
        let (released_objects, released_guard_locks) = self.exact_release_projection()?;
        if observation.lock_set_id != *self.lock_set_id()
            || observation.released_acquisition_receipts != self.journaled_lock_receipts()
            || !observation
                .current_full_lock_inventory
                .as_slice()
                .is_empty()
            || observation.release_capability_id != *self.atomic_commit_safety_capability_id()
        {
            return Err(RepositoryResultContractError(
                "release observation differs from retained acquisition lineage",
            ));
        }
        self.mint_post_command_lock_authority(CommitPostCommandLockStatusRecord::VerifiedReleased {
            current_full_lock_inventory: observation.current_full_lock_inventory,
            release_capability_id: observation.release_capability_id,
            released_acquisition_receipts: observation.released_acquisition_receipts,
            released_objects,
            released_guard_locks,
        })
    }

    pub(crate) fn committed_objects_digest(
        &self,
        objects: &CommittedRepositoryObjects,
    ) -> Result<Sha256Digest, RepositoryResultContractError> {
        result_digest(
            &CommittedObjectsDigestRecord {
                integration_set_digest: self
                    .source
                    .approved
                    .0
                    .record
                    .integration_set_digest
                    .clone(),
                committed_objects: objects.clone(),
            },
            "atomic committed-object digest failed",
        )
    }

    /// Mints the only production object binding accepted by the specialized
    /// taskCommit history resolver. Raw adapter scalars are checked against
    /// the current invocation before they can authorize history validation.
    pub(crate) fn observe_committed_core(
        &self,
        observation: CommitCommittedCoreObservationInput,
    ) -> Result<CommitCommittedCoreObservation, RepositoryResultContractError> {
        let CommitCommittedCoreObservationInput {
            commit_receipt_id,
            repository_version,
            committed_objects,
            committed_objects_digest,
            atomic_commit_safety_capability_id,
        } = observation;
        let expected_digest = self.committed_objects_digest(&committed_objects)?;
        if &commit_receipt_id != self.preallocated_commit_receipt_id()
            || committed_objects.exact_objects() != *self.exact_objects()
            || !committed_objects.all_versions_match(&repository_version)
            || committed_objects_digest != expected_digest
            || &atomic_commit_safety_capability_id != self.atomic_commit_safety_capability_id()
        {
            return Err(RepositoryResultContractError(
                "atomic committed-object observation differs from its invocation",
            ));
        }
        Ok(CommitCommittedCoreObservation {
            object_history_binding_witness: self.invocation.object_history_binding_witness(),
            commit_receipt_id,
            repository_version,
            committed_objects,
            committed_objects_digest,
            atomic_commit_safety_capability_id,
        })
    }

    #[cfg(test)]
    fn observe_committed_core_unchecked_test_only(
        &self,
        observation: CommitCommittedCoreObservationInput,
        foreign_equal_scalar_invocation: bool,
    ) -> CommitCommittedCoreObservation {
        let CommitCommittedCoreObservationInput {
            commit_receipt_id,
            repository_version,
            committed_objects,
            committed_objects_digest,
            atomic_commit_safety_capability_id,
        } = observation;
        let object_history_binding_witness = if foreign_equal_scalar_invocation {
            CommitAtomicCommitInvocationCapability::mint().object_history_binding_witness()
        } else {
            self.invocation.object_history_binding_witness()
        };
        CommitCommittedCoreObservation {
            object_history_binding_witness,
            commit_receipt_id,
            repository_version,
            committed_objects,
            committed_objects_digest,
            atomic_commit_safety_capability_id,
        }
    }

    /// Production bridge from the owning raw post-command partition to the
    /// sole taskCommit-specific resolver. A generic validated partition can
    /// never enter the committed-success path.
    pub(crate) fn resolve_task_commit_history(
        &self,
        core: &CommitCommittedCoreObservation,
        raw_partition: UnvalidatedRepositoryHistoryPartition,
        resolver: &RepositoryHistoryPartitionResolver<'_>,
        task_version_anchor: RepositoryAnchor,
        terminal_repository_anchor: RepositoryAnchor,
    ) -> Result<CommitCommittedHistoryObservation, RepositoryResultContractError> {
        if !self
            .invocation
            .owns_object_history_binding_witness(core.object_history_binding_witness())
        {
            return Err(RepositoryResultContractError(
                "taskCommit object binding belongs to another atomic invocation",
            ));
        }
        let partition = resolver
            .validate_task_commit_partition(raw_partition, core)
            .map_err(|_| {
                RepositoryResultContractError("taskCommit history partition validation failed")
            })?;
        if partition.partition().start_cursor() != self.before_repository_cursor()
            || terminal_repository_anchor.history_cursor()
                != partition.partition().through_inclusive()
            || task_version_anchor.history_cursor().through_version() != core.repository_version()
            || !partition
                .partition()
                .contains_cursor(task_version_anchor.history_cursor())
            || task_version_anchor.repository_identity()
                != self.post_merge_repository_anchor().repository_identity()
            || terminal_repository_anchor.repository_identity()
                != self.post_merge_repository_anchor().repository_identity()
            || task_version_anchor.configuration_identity()
                != self.post_merge_repository_anchor().configuration_identity()
            || terminal_repository_anchor.configuration_identity()
                != self.post_merge_repository_anchor().configuration_identity()
        {
            return Err(RepositoryResultContractError(
                "taskCommit history anchors differ from the atomic invocation",
            ));
        }
        Ok(CommitCommittedHistoryObservation {
            post_commit_history_partition: partition,
            task_version_anchor,
            terminal_repository_anchor,
        })
    }

    pub(crate) fn observe_repository_anchor(
        &self,
        history_cursor: RepositoryHistoryCursor,
        repository_identity: Sha256Digest,
        configuration_identity: ConfigurationIdentity,
        configuration_fingerprint: Sha256Digest,
    ) -> Result<RepositoryAnchor, RepositoryResultContractError> {
        RepositoryAnchor::from_guarded_observation(
            repository_identity,
            history_cursor,
            configuration_identity,
            configuration_fingerprint,
        )
        .map_err(|_| RepositoryResultContractError("atomic repository anchor digest failed"))
    }

    pub(crate) fn exact_release_projection(
        &self,
    ) -> Result<
        (CanonicalRepositoryTargets, CanonicalRepositoryTargets),
        RepositoryResultContractError,
    > {
        let all = project_lock_targets(&self.source.approved.0.plan.lock_entries)?;
        let guards = self.source.approved.0.record.guard_locks.clone();
        let objects = CanonicalRepositoryTargets::new(
            all.as_slice()
                .iter()
                .filter(|target| guards.as_slice().binary_search(target).is_err())
                .cloned()
                .collect(),
        )?;
        Ok((objects, guards))
    }

    pub(crate) fn full_lock_inventory(
        &self,
    ) -> Result<CanonicalRepositoryTargets, RepositoryResultContractError> {
        project_lock_targets(&self.source.approved.0.plan.lock_entries)
    }

    pub(crate) fn observe_terminal_target_snapshot(
        &self,
        repository_anchor: RepositoryAnchor,
        target_states: RepositoryTargetStates,
        observation_capability_id: CapabilityRowId,
        atomic_commit_safety_capability_id: CapabilityRowId,
    ) -> Result<CommitTerminalTargetSnapshotAuthority, RepositoryResultContractError> {
        if atomic_commit_safety_capability_id != *self.atomic_commit_safety_capability_id()
            || repository_anchor.repository_identity()
                != self.post_merge_repository_anchor().repository_identity()
            || repository_anchor.configuration_identity()
                != self.post_merge_repository_anchor().configuration_identity()
            || !target_states_cover_exact_commit_objects(self.exact_objects(), &target_states)
        {
            return Err(RepositoryResultContractError(
                "terminal target snapshot differs from the atomic invocation",
            ));
        }
        let record = CommitTargetStateSnapshotRecord {
            digest_kind: "unica.repository.commit.target-state-snapshot.v1",
            repository_anchor,
            target_states,
            observation_capability_id,
            atomic_commit_safety_capability_id,
        };
        let snapshot_digest =
            result_digest(&record, "terminal target-state snapshot digest failed")?;
        Ok(CommitTerminalTargetSnapshotAuthority {
            invocation_witness: self.invocation.child_witness(),
            record,
            snapshot_digest,
        })
    }

    #[cfg(test)]
    fn replace_zero_snapshot_witness_with_foreign_invocation_test_only(
        &self,
        snapshot: &mut CommitTerminalTargetSnapshotAuthority,
    ) {
        snapshot.invocation_witness =
            CommitAtomicCommitInvocationCapability::mint().child_witness();
    }

    #[cfg(test)]
    fn replace_lock_witness_with_foreign_invocation_test_only(
        &self,
        authority: &mut CommitPostCommandLockAuthority,
    ) {
        authority.invocation_witness =
            CommitAtomicCommitInvocationCapability::mint().child_witness();
    }

    pub(crate) fn observe_zero_effect_certificate(
        &self,
        terminal_target_snapshot: &CommitTerminalTargetSnapshotAuthority,
        post_command_history_partition: &ValidatedRepositoryHistoryPartition,
        lock_state: &CommitZeroEffectLockState,
        certificate_id: UnicaId,
        certificate_capability_id: CapabilityRowId,
    ) -> Result<CommitZeroEffectCertificate, RepositoryResultContractError> {
        // Pointer ownership is checked before any replayable snapshot scalar.
        if !self
            .invocation
            .owns_child_witness(&terminal_target_snapshot.invocation_witness)
        {
            return Err(RepositoryResultContractError(
                "terminal target snapshot belongs to another atomic invocation",
            ));
        }
        let terminal_anchor = &terminal_target_snapshot.record.repository_anchor;
        if certificate_capability_id != *self.atomic_commit_safety_capability_id()
            || post_command_history_partition.start_cursor() != self.before_repository_cursor()
            || terminal_anchor.history_cursor()
                != post_command_history_partition.through_inclusive()
            || !post_command_history_partition.all_entries_are_one_of(&[
                RepositoryHistoryPartitionClassification::UnrelatedRoutine,
            ])
            || !exact_zero_effect_target_transition(
                self.exact_objects(),
                &self.source.pre_command_target_snapshot.record.target_states,
                &terminal_target_snapshot.record.target_states,
            )
            || !valid_zero_effect_lock_state(self.source, self.invocation, lock_state)
        {
            return Err(RepositoryResultContractError(
                "zero-effect evidence is incomplete or differs from the atomic invocation",
            ));
        }
        let lock_state_digest = zero_effect_lock_state_digest(lock_state)?;
        let certificate_digest = result_digest(
            &CommitZeroEffectCertificateDigestRecord {
                digest_kind: "unica.repository.commit.zero-effect-certificate.v1",
                effect_intent_digest: self.effect_intent_digest(),
                pre_command_target_snapshot_digest: self.pre_command_target_snapshot_digest(),
                terminal_target_snapshot_digest: &terminal_target_snapshot.snapshot_digest,
                post_command_history_partition_digest: post_command_history_partition
                    .partition_digest(),
                terminal_repository_anchor_digest: terminal_anchor.anchor_digest(),
                lock_state_digest: &lock_state_digest,
                certificate_id: &certificate_id,
                certificate_capability_id: &certificate_capability_id,
            },
            "commit zero-effect certificate digest failed",
        )?;
        Ok(CommitZeroEffectCertificate {
            effect_intent_digest: self.effect_intent_digest().clone(),
            pre_command_target_snapshot_digest: self.pre_command_target_snapshot_digest().clone(),
            terminal_target_snapshot_digest: terminal_target_snapshot.snapshot_digest.clone(),
            post_command_history_partition_digest: post_command_history_partition
                .partition_digest()
                .clone(),
            terminal_repository_anchor_digest: terminal_anchor.anchor_digest().clone(),
            lock_state_digest,
            certificate_id,
            certificate_capability_id,
            certificate_digest,
        })
    }

    pub(crate) fn observe_committed(
        &self,
        observation: CommitCommittedObservationInput,
    ) -> CommitAtomicCommitObservation {
        CommitAtomicCommitObservation::Committed(Box::new(observation))
    }

    pub(crate) fn observe_proven_zero_effect(
        &self,
        observation: CommitProvenZeroEffectObservationInput,
    ) -> CommitAtomicCommitObservation {
        CommitAtomicCommitObservation::ProvenZeroEffect(Box::new(observation))
    }

    pub(crate) const fn observe_ambiguous(&self) -> CommitAtomicCommitObservation {
        CommitAtomicCommitObservation::Ambiguous
    }

    pub(crate) fn complete(
        self,
        payload: Box<dyn CommitAtomicCommitPayload>,
    ) -> CommitAtomicCommitCompletion {
        CommitAtomicCommitCompletion {
            completion: self.invocation.completion(),
            payload,
        }
    }
}

pub(crate) trait CommitAtomicCommitPayload {
    fn commit_safety_lineage_witness(&self) -> &CommitSafetyLineageWitness;
    fn binds(&self, request: &CommitAtomicCommitRequest<'_>) -> bool;
    fn into_observation(self: Box<Self>) -> CommitAtomicCommitObservation;
}

pub(crate) struct CommitAtomicCommitCompletion {
    completion: CommitAtomicCommitCompletionCapability,
    payload: Box<dyn CommitAtomicCommitPayload>,
}

impl fmt::Debug for CommitAtomicCommitCompletion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CommitAtomicCommitCompletion")
            .field("completion", &self.completion)
            .field("payload", &"<sealed atomic commit payload>")
            .finish()
    }
}

#[derive(Debug)]
pub(crate) struct CommitCommittedAuthority {
    source: Box<CommitAtomicCommitSource>,
    atomic_invocation: CommitAtomicCommitInvocationCapability,
    observation: Box<CommitCommittedObservationInput>,
}

impl CommitCommittedAuthority {
    pub(crate) fn task_version_anchor(&self) -> &RepositoryAnchor {
        &self.observation.history.task_version_anchor
    }

    pub(crate) fn terminal_repository_anchor(&self) -> &RepositoryAnchor {
        &self.observation.history.terminal_repository_anchor
    }
}

#[derive(Debug)]
pub(crate) struct CommitProvenZeroEffectAuthority {
    source: Box<CommitAtomicCommitSource>,
    atomic_invocation: CommitAtomicCommitInvocationCapability,
    observation: Box<CommitProvenZeroEffectObservationInput>,
}

/// A zero-effect observation with locks still held is explicitly nonterminal:
/// the whole source is preserved for Task 16 cleanup/recovery.  A verified
/// release has a separate owning hand-off and is never reconstructed from a
/// certificate digest.
#[derive(Debug)]
pub(crate) enum CommitProvenZeroEffectDisposition {
    CleanupRequired(CommitZeroEffectCleanupRequiredAuthority),
    VerifiedReleased(CommitZeroEffectReleasedAuthority),
}

#[derive(Debug)]
pub(crate) struct CommitZeroEffectCleanupRequiredAuthority {
    source: Box<CommitAtomicCommitSource>,
    atomic_invocation: CommitAtomicCommitInvocationCapability,
    observation: Box<CommitProvenZeroEffectObservationInput>,
}

#[derive(Debug)]
pub(crate) struct CommitZeroEffectReleasedAuthority {
    source: Box<CommitAtomicCommitSource>,
    atomic_invocation: CommitAtomicCommitInvocationCapability,
    observation: Box<CommitProvenZeroEffectObservationInput>,
}

impl CommitProvenZeroEffectAuthority {
    pub(crate) fn into_disposition(self) -> CommitProvenZeroEffectDisposition {
        let Self {
            source,
            atomic_invocation,
            observation,
        } = self;
        if matches!(&observation.lock_state, CommitZeroEffectLockState::Held(_)) {
            CommitProvenZeroEffectDisposition::CleanupRequired(
                CommitZeroEffectCleanupRequiredAuthority {
                    source,
                    atomic_invocation,
                    observation,
                },
            )
        } else {
            debug_assert!(matches!(
                &observation.lock_state,
                CommitZeroEffectLockState::VerifiedReleased(_)
            ));
            CommitProvenZeroEffectDisposition::VerifiedReleased(CommitZeroEffectReleasedAuthority {
                source,
                atomic_invocation,
                observation,
            })
        }
    }
}

impl CommitZeroEffectCleanupRequiredAuthority {
    pub(crate) fn effect_intent_digest(&self) -> &Sha256Digest {
        &self.source.written_intent.fsync.effect_intent_digest
    }

    pub(crate) fn certificate_digest(&self) -> &Sha256Digest {
        &self.observation.certificate.certificate_digest
    }

    pub(crate) fn owns_current_lock_evidence(&self) -> bool {
        match &self.observation.lock_state {
            CommitZeroEffectLockState::Held(authority) => self
                .atomic_invocation
                .owns_child_witness(&authority.invocation_witness),
            CommitZeroEffectLockState::VerifiedReleased(_) | CommitZeroEffectLockState::Unknown => {
                false
            }
        }
    }
}

impl CommitZeroEffectReleasedAuthority {
    pub(crate) fn effect_intent_digest(&self) -> &Sha256Digest {
        &self.source.written_intent.fsync.effect_intent_digest
    }

    pub(crate) fn certificate_digest(&self) -> &Sha256Digest {
        &self.observation.certificate.certificate_digest
    }

    pub(crate) fn owns_current_lock_evidence(&self) -> bool {
        match &self.observation.lock_state {
            CommitZeroEffectLockState::VerifiedReleased(authority) => self
                .atomic_invocation
                .owns_child_witness(&authority.invocation_witness),
            CommitZeroEffectLockState::Held(_) | CommitZeroEffectLockState::Unknown => false,
        }
    }
}

#[derive(Debug)]
pub(crate) enum CommitExactOnceOutcome {
    Committed(CommitCommittedAuthority),
    ProvenZeroEffect(CommitProvenZeroEffectAuthority),
    Ambiguous(CommitAmbiguousAuthority),
}

fn valid_effect_intent_fsync(
    record: &CommitEffectIntentRecord,
    evidence: &CommitEffectIntentFsyncEvidence,
) -> bool {
    if evidence.intent_record_digest != record.intent_record_digest {
        return false;
    }
    result_digest(
        &CommitEffectIntentFsyncDigestRecord {
            digest_kind: "unica.repository.commit.effect-intent-fsync.v1",
            intent_record_digest: &evidence.intent_record_digest,
            fsync_receipt_id: &evidence.fsync_receipt_id,
            fsync_capability_id: &evidence.fsync_capability_id,
        },
        "commit effect-intent fsync digest failed",
    )
    .is_ok_and(|digest| digest == evidence.effect_intent_digest)
}

fn valid_effect_intent_not_written(
    record: &CommitEffectIntentRecord,
    certificate: &CommitEffectIntentNotWrittenCertificate,
) -> bool {
    if certificate.intent_record_digest != record.intent_record_digest {
        return false;
    }
    result_digest(
        &CommitEffectIntentNotWrittenDigestRecord {
            digest_kind: "unica.repository.commit.effect-intent-not-written.v1",
            intent_record_digest: &certificate.intent_record_digest,
            certificate_id: &certificate.certificate_id,
            certificate_capability_id: &certificate.certificate_capability_id,
        },
        "commit effect-intent not-written digest failed",
    )
    .is_ok_and(|digest| digest == certificate.certificate_digest)
}

#[derive(Clone, Copy)]
enum ExpectedPostCommandLockState {
    Held,
    VerifiedReleased,
}

fn valid_post_command_lock_authority(
    source: &CommitAtomicCommitSource,
    invocation: &CommitAtomicCommitInvocationCapability,
    authority: &CommitPostCommandLockAuthority,
    expected_status: ExpectedPostCommandLockState,
) -> bool {
    // Reject a replayed equal-scalar release/held observation before reading
    // its retained lock-set or receipt projections.
    if !invocation.owns_child_witness(&authority.invocation_witness) {
        return false;
    }
    let expected_lineage = retained_commit_lock_lineage_record(source);
    let expected_lineage_digest = match result_digest(
        &expected_lineage,
        "commit retained-lock lineage digest failed",
    ) {
        Ok(value) => value,
        Err(_) => return false,
    };
    if authority.record.effect_intent_digest != source.written_intent.fsync.effect_intent_digest
        || authority.record.preallocated_commit_receipt_id
            != source
                .written_intent
                .record
                .durable_record
                .preallocated_commit_receipt_id
        || authority.record.retained_lock_lineage != expected_lineage
        || authority.record.retained_lock_lineage_digest != expected_lineage_digest
        || !result_digest(
            &authority.record,
            "commit post-command lock evidence digest failed",
        )
        .is_ok_and(|digest| digest == authority.evidence_digest)
    {
        return false;
    }
    match (&authority.record.status, expected_status) {
        (
            CommitPostCommandLockStatusRecord::Held {
                current_full_lock_inventory,
            },
            ExpectedPostCommandLockState::Held,
        ) => project_lock_targets(&source.approved.0.plan.lock_entries)
            .as_ref()
            .is_ok_and(|expected| expected == current_full_lock_inventory),
        (
            CommitPostCommandLockStatusRecord::VerifiedReleased {
                current_full_lock_inventory,
                release_capability_id,
                released_acquisition_receipts,
                released_objects,
                released_guard_locks,
                ..
            },
            ExpectedPostCommandLockState::VerifiedReleased,
        ) => {
            current_full_lock_inventory.as_slice().is_empty()
                && release_capability_id
                    == &source
                        .written_intent
                        .record
                        .durable_record
                        .atomic_commit_safety_capability_id
                && released_acquisition_receipts == &expected_lineage.journaled_lock_receipts
                && validate_commit_release_projection(
                    &source.approved.0.plan,
                    &source.approved.0.record.guard_locks,
                    released_objects,
                    released_guard_locks,
                )
                .is_ok()
        }
        _ => false,
    }
}

fn valid_committed_observation(
    source: &CommitAtomicCommitSource,
    invocation: &CommitAtomicCommitInvocationCapability,
    observation: &CommitCommittedObservationInput,
) -> bool {
    let core = &observation.core;
    let history = &observation.history;
    let sealed_partition = &history.post_commit_history_partition;
    if !invocation.owns_object_history_binding_witness(core.object_history_binding_witness())
        || !sealed_partition.binds(core)
    {
        return false;
    }
    let partition = sealed_partition.partition();
    let task_entries = partition
        .entries()
        .filter(|entry| {
            entry.classification() == RepositoryHistoryPartitionClassification::TaskCommit
        })
        .collect::<Vec<_>>();
    let digest_matches = result_digest(
        &CommittedObjectsDigestRecord {
            integration_set_digest: source.approved.0.record.integration_set_digest.clone(),
            committed_objects: core.committed_objects.clone(),
        },
        "committed-object digest failed",
    )
    .is_ok_and(|digest| digest == core.committed_objects_digest);
    let release_matches = match &observation.release {
        CommitLockReleaseObservation::Verified(authority) => valid_post_command_lock_authority(
            source,
            invocation,
            authority,
            ExpectedPostCommandLockState::VerifiedReleased,
        ),
        CommitLockReleaseObservation::Unknown => false,
    };
    let pre_anchor = &source.post_merge_repository_anchor;
    let task_anchor = &history.task_version_anchor;
    let terminal_anchor = &history.terminal_repository_anchor;
    core.commit_receipt_id
        == source
            .written_intent
            .record
            .durable_record
            .preallocated_commit_receipt_id
        && core.committed_objects.exact_objects() == source.approved.0.record.exact_objects
        && core
            .committed_objects
            .all_versions_match(&core.repository_version)
        && digest_matches
        && core.atomic_commit_safety_capability_id
            == source
                .written_intent
                .record
                .durable_record
                .atomic_commit_safety_capability_id
        && partition.start_cursor() == &source.before_repository_cursor
        && terminal_anchor.history_cursor() == partition.through_inclusive()
        && task_entries.len() == 1
        && task_entries[0].repository_version() == &core.repository_version
        && task_entries[0].semantic_delta_digest() == &core.committed_objects_digest
        && task_anchor.history_cursor().through_version() == &core.repository_version
        && partition.contains_cursor(task_anchor.history_cursor())
        && partition.all_entries_are_one_of(&[
            RepositoryHistoryPartitionClassification::TaskCommit,
            RepositoryHistoryPartitionClassification::UnrelatedRoutine,
        ])
        && pre_anchor.repository_identity() == task_anchor.repository_identity()
        && pre_anchor.repository_identity() == terminal_anchor.repository_identity()
        && pre_anchor.configuration_identity() == task_anchor.configuration_identity()
        && pre_anchor.configuration_identity() == terminal_anchor.configuration_identity()
        && release_matches
}

fn valid_zero_effect_lock_state(
    source: &CommitAtomicCommitSource,
    invocation: &CommitAtomicCommitInvocationCapability,
    lock_state: &CommitZeroEffectLockState,
) -> bool {
    match lock_state {
        CommitZeroEffectLockState::Held(authority) => valid_post_command_lock_authority(
            source,
            invocation,
            authority,
            ExpectedPostCommandLockState::Held,
        ),
        CommitZeroEffectLockState::VerifiedReleased(authority) => {
            valid_post_command_lock_authority(
                source,
                invocation,
                authority,
                ExpectedPostCommandLockState::VerifiedReleased,
            )
        }
        CommitZeroEffectLockState::Unknown => false,
    }
}

fn valid_zero_effect_observation(
    source: &CommitAtomicCommitSource,
    invocation: &CommitAtomicCommitInvocationCapability,
    observation: &CommitProvenZeroEffectObservationInput,
) -> bool {
    let terminal_snapshot = &observation.terminal_target_snapshot;
    // Pointer ownership precedes all getters and digest/scalar comparisons.
    if !invocation.owns_child_witness(&terminal_snapshot.invocation_witness) {
        return false;
    }
    let baseline_snapshot = &source.pre_command_target_snapshot;
    let durable = &source.written_intent.record.durable_record;
    let certificate = &observation.certificate;
    let partition = &observation.post_command_history_partition;
    let terminal_anchor = &terminal_snapshot.record.repository_anchor;
    let pre_anchor = &source.post_merge_repository_anchor;
    let baseline_digest_matches = result_digest(
        &baseline_snapshot.record,
        "pre-command target-state snapshot digest failed",
    )
    .is_ok_and(|digest| digest == baseline_snapshot.snapshot_digest);
    let terminal_digest_matches = result_digest(
        &terminal_snapshot.record,
        "terminal target-state snapshot digest failed",
    )
    .is_ok_and(|digest| digest == terminal_snapshot.snapshot_digest);
    let lock_state_digest = match zero_effect_lock_state_digest(&observation.lock_state) {
        Ok(value) => value,
        Err(_) => return false,
    };
    let certificate_matches = certificate.effect_intent_digest
        == source.written_intent.fsync.effect_intent_digest
        && certificate.pre_command_target_snapshot_digest == baseline_snapshot.snapshot_digest
        && certificate.terminal_target_snapshot_digest == terminal_snapshot.snapshot_digest
        && certificate.post_command_history_partition_digest == *partition.partition_digest()
        && certificate.terminal_repository_anchor_digest == *terminal_anchor.anchor_digest()
        && certificate.lock_state_digest == lock_state_digest
        && certificate.certificate_capability_id == durable.atomic_commit_safety_capability_id
        && result_digest(
            &CommitZeroEffectCertificateDigestRecord {
                digest_kind: "unica.repository.commit.zero-effect-certificate.v1",
                effect_intent_digest: &certificate.effect_intent_digest,
                pre_command_target_snapshot_digest: &certificate.pre_command_target_snapshot_digest,
                terminal_target_snapshot_digest: &certificate.terminal_target_snapshot_digest,
                post_command_history_partition_digest: &certificate
                    .post_command_history_partition_digest,
                terminal_repository_anchor_digest: &certificate.terminal_repository_anchor_digest,
                lock_state_digest: &certificate.lock_state_digest,
                certificate_id: &certificate.certificate_id,
                certificate_capability_id: &certificate.certificate_capability_id,
            },
            "commit zero-effect certificate digest failed",
        )
        .is_ok_and(|digest| digest == certificate.certificate_digest);
    baseline_digest_matches
        && terminal_digest_matches
        && durable.pre_command_target_snapshot == baseline_snapshot.record
        && durable.pre_command_target_snapshot_digest == baseline_snapshot.snapshot_digest
        && baseline_snapshot.record.repository_anchor == *pre_anchor
        && baseline_snapshot.record.atomic_commit_safety_capability_id
            == durable.atomic_commit_safety_capability_id
        && terminal_snapshot.record.atomic_commit_safety_capability_id
            == durable.atomic_commit_safety_capability_id
        && exact_zero_effect_target_transition(
            &source.approved.0.record.exact_objects,
            &baseline_snapshot.record.target_states,
            &terminal_snapshot.record.target_states,
        )
        && certificate_matches
        && valid_zero_effect_lock_state(source, invocation, &observation.lock_state)
        && partition.start_cursor() == &source.before_repository_cursor
        && terminal_anchor.history_cursor() == partition.through_inclusive()
        && partition
            .all_entries_are_one_of(&[RepositoryHistoryPartitionClassification::UnrelatedRoutine])
        && pre_anchor.repository_identity() == terminal_anchor.repository_identity()
        && pre_anchor.configuration_identity() == terminal_anchor.configuration_identity()
}

impl CommitScopedAtomicSafetyAuthority {
    pub(crate) fn commit_exact_once(
        self,
        operation_scope: OperationScope,
        preallocated_commit_receipt_id: UnicaId,
        registered_port: &mut dyn CommitRegisteredOperationPort,
        intent_port: &mut dyn CommitEffectIntentPort,
    ) -> CommitEffectIntentOutcome {
        let apply_task_id = self.approved.validated_apply_request().task_id();
        let expected_project_id = &self.approved.0.validated_comment_policy().record.project_id;
        if !matches!(
            &operation_scope,
            OperationScope::Task {
                project_id,
                task_id,
                ..
            } if project_id == expected_project_id && task_id == apply_task_id
        ) {
            return CommitEffectIntentOutcome::PreIntentBlocked(
                CommitPreIntentBlockedAuthority::new(self, None, None, None),
            );
        }
        let request_value =
            match serde_json::to_value(self.approved.validated_apply_request().request()) {
                Ok(value) => value,
                Err(_) => {
                    return CommitEffectIntentOutcome::PreIntentBlocked(
                        CommitPreIntentBlockedAuthority::new(self, None, None, None),
                    );
                }
            };
        let canonical_input_digest = match operation_input_digest(
            BranchedLifecycleToolName::RepositoryCommit,
            DurableExecutionPolicy::PreviewedJournaledEffect,
            &request_value,
        ) {
            Ok(digest) => digest,
            Err(_) => {
                return CommitEffectIntentOutcome::PreIntentBlocked(
                    CommitPreIntentBlockedAuthority::new(self, None, None, None),
                );
            }
        };
        let invocation = CommitRegisteredOperationInvocationCapability::mint();
        let request = CommitRegisteredOperationRequest {
            scope: &self,
            operation_scope: &operation_scope,
            canonical_input_digest: &canonical_input_digest,
            invocation: &invocation,
        };
        let completion = match registered_port.load_registered_commit_operation(request) {
            Ok(completion) => completion,
            Err(_) => {
                return CommitEffectIntentOutcome::PreIntentBlocked(
                    CommitPreIntentBlockedAuthority::new(self, None, None, None),
                );
            }
        };
        let request = CommitRegisteredOperationRequest {
            scope: &self,
            operation_scope: &operation_scope,
            canonical_input_digest: &canonical_input_digest,
            invocation: &invocation,
        };
        if !invocation.owns_completion(&completion.completion)
            || !self.owns_commit_safety_lineage_witness(
                completion.lease.commit_safety_lineage_witness(),
            )
            || !completion.lease.binds(&request)
        {
            return CommitEffectIntentOutcome::PreIntentBlocked(
                CommitPreIntentBlockedAuthority::new(self, None, None, None),
            );
        }
        let observation = completion.lease.into_current_operation();
        let expected_operation = request.operation();
        let expected_policy = request.policy();
        let identity = &observation.identity;
        if identity.operation_id != *self.approved.validated_apply_request().operation_id()
            || identity.scope != operation_scope
            || identity.operation != expected_operation
            || identity.policy != expected_policy
            || identity.canonical_input_digest != canonical_input_digest
        {
            return CommitEffectIntentOutcome::PreIntentBlocked(
                CommitPreIntentBlockedAuthority::new(self, None, None, None),
            );
        }
        let RegisteredCommitOperationIdentity {
            operation_id,
            scope: authoritative_scope,
            operation,
            policy,
            canonical_input_digest,
        } = observation.identity;
        let Some(task_scope) = RegisteredCommitTaskScopeAuthority::from_current_record(
            authoritative_scope,
            expected_project_id,
            apply_task_id,
        ) else {
            return CommitEffectIntentOutcome::PreIntentBlocked(
                CommitPreIntentBlockedAuthority::new(self, None, None, None),
            );
        };
        let registered = RegisteredCommitOperationAuthority {
            operation_id,
            task_scope,
            operation,
            policy,
            canonical_input_digest,
            evidence: observation.evidence,
        };
        self.write_intent_and_commit(registered, preallocated_commit_receipt_id, intent_port)
    }

    fn write_intent_and_commit(
        self,
        registered: RegisteredCommitOperationAuthority,
        preallocated_commit_receipt_id: UnicaId,
        intent_port: &mut dyn CommitEffectIntentPort,
    ) -> CommitEffectIntentOutcome {
        let record =
            match CommitEffectIntentRecord::new(&self, &registered, preallocated_commit_receipt_id)
            {
                Ok(record) => record,
                Err(_) => {
                    return CommitEffectIntentOutcome::PreIntentBlocked(
                        CommitPreIntentBlockedAuthority::new(self, Some(registered), None, None),
                    );
                }
            };
        let invocation = CommitEffectIntentInvocationCapability::mint();
        let request = CommitEffectIntentRequest {
            scope: &self,
            registered: &registered,
            record: &record,
            invocation: &invocation,
        };
        let completion = match intent_port.write_and_fsync_commit_intent(request) {
            Ok(completion) => completion,
            Err(error) => {
                return CommitEffectIntentOutcome::PostIntent(CommitExactOnceOutcome::Ambiguous(
                    CommitAmbiguousAuthority {
                        source: CommitAmbiguousSource::intent_port(
                            self, registered, record, invocation, error,
                        ),
                    },
                ));
            }
        };
        let request = CommitEffectIntentRequest {
            scope: &self,
            registered: &registered,
            record: &record,
            invocation: &invocation,
        };
        if !invocation.owns_completion(&completion.completion) {
            return CommitEffectIntentOutcome::PostIntent(CommitExactOnceOutcome::Ambiguous(
                CommitAmbiguousAuthority {
                    source: CommitAmbiguousSource::intent_completion(
                        self, registered, record, invocation, completion,
                    ),
                },
            ));
        }
        let lineage_matches = match &completion.disposition {
            CommitEffectIntentCompletionDisposition::Written(lease) => {
                self.owns_commit_safety_lineage_witness(lease.commit_safety_lineage_witness())
            }
            CommitEffectIntentCompletionDisposition::ProvenNotWritten(lease) => {
                self.owns_commit_safety_lineage_witness(lease.commit_safety_lineage_witness())
            }
        };
        let binding_matches = lineage_matches
            && match &completion.disposition {
                CommitEffectIntentCompletionDisposition::Written(lease) => lease.binds(&request),
                CommitEffectIntentCompletionDisposition::ProvenNotWritten(lease) => {
                    lease.binds(&request)
                }
            };
        if !binding_matches {
            return CommitEffectIntentOutcome::PostIntent(CommitExactOnceOutcome::Ambiguous(
                CommitAmbiguousAuthority {
                    source: CommitAmbiguousSource::intent_completion(
                        self, registered, record, invocation, completion,
                    ),
                },
            ));
        }
        match completion.disposition {
            CommitEffectIntentCompletionDisposition::ProvenNotWritten(lease) => {
                let certificate = lease.into_certificate();
                if valid_effect_intent_not_written(&record, &certificate) {
                    CommitEffectIntentOutcome::PreIntentBlocked(
                        CommitPreIntentBlockedAuthority::new(
                            self,
                            Some(registered),
                            Some(record),
                            Some(certificate),
                        ),
                    )
                } else {
                    CommitEffectIntentOutcome::PostIntent(CommitExactOnceOutcome::Ambiguous(
                        CommitAmbiguousAuthority {
                            source: CommitAmbiguousSource::intent_evidence(
                                self,
                                registered,
                                record,
                                invocation,
                                CommitRejectedIntentEvidence::ProvenNotWritten(certificate),
                            ),
                        },
                    ))
                }
            }
            CommitEffectIntentCompletionDisposition::Written(lease) => {
                let fsync = lease.into_fsync_evidence();
                if !valid_effect_intent_fsync(&record, &fsync) {
                    return CommitEffectIntentOutcome::PostIntent(
                        CommitExactOnceOutcome::Ambiguous(CommitAmbiguousAuthority {
                            source: CommitAmbiguousSource::intent_evidence(
                                self,
                                registered,
                                record,
                                invocation,
                                CommitRejectedIntentEvidence::Fsync(fsync),
                            ),
                        }),
                    );
                }
                let written_intent = CommitWrittenEffectIntentAuthority {
                    registered,
                    record,
                    fsync,
                };
                CommitEffectIntentOutcome::PostIntent(self.invoke_atomic_commit(written_intent))
            }
        }
    }

    fn invoke_atomic_commit(
        self,
        written_intent: CommitWrittenEffectIntentAuthority,
    ) -> CommitExactOnceOutcome {
        let Self {
            approved,
            completion,
            immediate_history_guard_evidence,
            post_merge_repository_anchor,
            before_repository_cursor,
            pre_command_target_snapshot,
        } = self;
        let source = CommitAtomicCommitSource {
            approved,
            immediate_history_guard_evidence,
            post_merge_repository_anchor,
            before_repository_cursor,
            pre_command_target_snapshot,
            written_intent,
        };
        let invocation = CommitAtomicCommitInvocationCapability::mint();
        let request = CommitAtomicCommitRequest {
            source: &source,
            invocation: &invocation,
        };
        let completion = match completion.lease.commit_exact_once(request) {
            Ok(completion) => completion,
            Err(error) => {
                return CommitExactOnceOutcome::Ambiguous(CommitAmbiguousAuthority {
                    source: CommitAmbiguousSource::atomic_port(source, error),
                });
            }
        };
        let request = CommitAtomicCommitRequest {
            source: &source,
            invocation: &invocation,
        };
        if !invocation.owns_completion(&completion.completion)
            || !source.owns_commit_safety_lineage_witness(
                completion.payload.commit_safety_lineage_witness(),
            )
            || !completion.payload.binds(&request)
        {
            return CommitExactOnceOutcome::Ambiguous(CommitAmbiguousAuthority {
                source: CommitAmbiguousSource::atomic_completion(source, completion),
            });
        }
        let observation = completion.payload.into_observation();
        match observation {
            CommitAtomicCommitObservation::Committed(observation)
                if valid_committed_observation(&source, &invocation, &observation) =>
            {
                CommitExactOnceOutcome::Committed(CommitCommittedAuthority {
                    source: Box::new(source),
                    atomic_invocation: invocation,
                    observation,
                })
            }
            CommitAtomicCommitObservation::ProvenZeroEffect(observation)
                if valid_zero_effect_observation(&source, &invocation, &observation) =>
            {
                CommitExactOnceOutcome::ProvenZeroEffect(CommitProvenZeroEffectAuthority {
                    source: Box::new(source),
                    atomic_invocation: invocation,
                    observation,
                })
            }
            observation => CommitExactOnceOutcome::Ambiguous(CommitAmbiguousAuthority {
                source: CommitAmbiguousSource::atomic_observation(source, observation),
            }),
        }
    }
}

#[derive(Debug)]
pub(crate) struct CommitFreshPreviewRequiredAuthority {
    approved: ApprovedCommitPreviewAuthority,
    completion: CommitImmediateRecheckCompletion,
    fresh_history_guard_evidence: PostMergeHistoryGuardEvidence,
    fresh_repository_anchor: RepositoryAnchor,
}

impl CommitFreshPreviewRequiredAuthority {
    pub(crate) fn fresh_history_guard_evidence(&self) -> &PostMergeHistoryGuardEvidence {
        &self.fresh_history_guard_evidence
    }

    pub(crate) fn fresh_repository_anchor(&self) -> &RepositoryAnchor {
        &self.fresh_repository_anchor
    }

    pub(crate) fn validate_refresh_preview_request(
        self,
        request: RepositoryCommitRequest,
    ) -> Result<ValidatedFreshCommitPreviewRequestAuthority, Box<CommitFreshPreviewBlockedAuthority>>
    {
        let request = match request.into_validated_preview() {
            Ok(request) => request,
            Err(failure) => {
                return Err(Box::new(CommitFreshPreviewBlockedAuthority {
                    fresh: self,
                    request: failure.into_request(),
                    failure: CommitFreshPreviewRequestFailure::NotPreview,
                }));
            }
        };
        let old_preview = self.approved.0.validated_preview_request();
        let Some(old_apply) = self.approved.1.as_ref() else {
            return Err(Box::new(CommitFreshPreviewBlockedAuthority {
                fresh: self,
                request: request.into_request(),
                failure: CommitFreshPreviewRequestFailure::LineageMismatch,
            }));
        };
        if request.operation_id() == old_preview.operation_id()
            || request.operation_id() == old_apply.operation_id()
        {
            return Err(Box::new(CommitFreshPreviewBlockedAuthority {
                fresh: self,
                request: request.into_request(),
                failure: CommitFreshPreviewRequestFailure::ReusedOperationId,
            }));
        }
        if !commit_preview_lineage_matches(&request, old_preview) {
            return Err(Box::new(CommitFreshPreviewBlockedAuthority {
                fresh: self,
                request: request.into_request(),
                failure: CommitFreshPreviewRequestFailure::LineageMismatch,
            }));
        }
        Ok(ValidatedFreshCommitPreviewRequestAuthority {
            fresh: self,
            request,
        })
    }
}

fn commit_preview_lineage_matches(
    candidate: &ValidatedRepositoryCommitPreviewRequest,
    approved: &ValidatedRepositoryCommitPreviewRequest,
) -> bool {
    candidate.cwd() == approved.cwd()
        && candidate.task_id() == approved.task_id()
        && candidate.integration_set_id() == approved.integration_set_id()
        && candidate.expected_integration_set_digest() == approved.expected_integration_set_digest()
        && candidate.lock_set_id() == approved.lock_set_id()
        && candidate.expected_lock_set_digest() == approved.expected_lock_set_digest()
        && candidate.verification_id() == approved.verification_id()
        && candidate.expected_verification_digest() == approved.expected_verification_digest()
        && candidate.merge_receipt_id() == approved.merge_receipt_id()
        && candidate.support_gate_id() == approved.support_gate_id()
        && candidate.expected_support_gate_digest() == approved.expected_support_gate_digest()
        && candidate.expected_support_gate_history_evidence_digest()
            == approved.expected_support_gate_history_evidence_digest()
        && candidate.expected_authorized_post_merge_fingerprint()
            == approved.expected_authorized_post_merge_fingerprint()
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum CommitFreshPreviewRequestFailure {
    NotPreview,
    ReusedOperationId,
    LineageMismatch,
}

#[derive(Debug)]
pub(crate) struct CommitFreshPreviewBlockedAuthority {
    fresh: CommitFreshPreviewRequiredAuthority,
    request: RepositoryCommitRequest,
    failure: CommitFreshPreviewRequestFailure,
}

impl CommitFreshPreviewBlockedAuthority {
    pub(crate) fn failure(&self) -> &CommitFreshPreviewRequestFailure {
        &self.failure
    }

    pub(crate) fn into_recovery_parts(
        self: Box<Self>,
    ) -> (
        CommitFreshPreviewRequiredAuthority,
        RepositoryCommitRequest,
        CommitFreshPreviewRequestFailure,
    ) {
        let Self {
            fresh,
            request,
            failure,
        } = *self;
        (fresh, request, failure)
    }
}

#[derive(Debug)]
pub(crate) struct ValidatedFreshCommitPreviewRequestAuthority {
    fresh: CommitFreshPreviewRequiredAuthority,
    request: ValidatedRepositoryCommitPreviewRequest,
}

impl ValidatedFreshCommitPreviewRequestAuthority {
    pub(crate) fn validated_preview_request(&self) -> &ValidatedRepositoryCommitPreviewRequest {
        &self.request
    }

    pub(crate) fn fresh_requirement(&self) -> &CommitFreshPreviewRequiredAuthority {
        &self.fresh
    }

    pub(crate) fn into_parts(
        self,
    ) -> (
        CommitFreshPreviewRequiredAuthority,
        ValidatedRepositoryCommitPreviewRequest,
    ) {
        (self.fresh, self.request)
    }
}

#[derive(Debug)]
pub(crate) enum CommitImmediateRecheckOutcome {
    Ready(CommitScopedAtomicSafetyAuthority),
    FreshPreviewRequired(CommitFreshPreviewRequiredAuthority),
    RecoveryRequired(CommitImmediateRecoveryRequiredAuthority),
}

/// Atomic adapter observation of the exact post-commit object set. It is
/// non-Clone/non-Deserialize so callers cannot independently splice version,
/// fingerprints, absence and capability evidence.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct CommitObjectPostStateObservationAuthority {
    repository_version: RepositoryVersion,
    committed_objects: CommittedRepositoryObjects,
    atomic_commit_safety_capability_id: CapabilityRowId,
}

impl CommitObjectPostStateObservationAuthority {
    #[cfg(test)]
    pub(crate) fn from_atomic_adapter(
        repository_version: RepositoryVersion,
        committed_objects: CommittedRepositoryObjects,
        atomic_commit_safety_capability_id: CapabilityRowId,
    ) -> Self {
        Self {
            repository_version,
            committed_objects,
            atomic_commit_safety_capability_id,
        }
    }
}

/// The concrete commit-object authority consumed by the only taskCommit
/// partition constructor. It retains the approved plan/preview equality chain,
/// verified post-state, full integration-set digest and atomic capability.
#[derive(Debug)]
pub(crate) struct ValidatedCommitObjectAuthority {
    #[cfg(test)]
    object_history_binding_witness: CommitObjectHistoryBindingWitness,
    approved_preview: ApprovedCommitPreviewAuthority,
    _commit_recheck_completion: Option<CommitImmediateRecheckCompletion>,
    _immediate_history_guard_evidence: Option<PostMergeHistoryGuardEvidence>,
    _post_merge_repository_anchor: Option<RepositoryAnchor>,
    _before_repository_cursor: Option<RepositoryHistoryCursor>,
    repository_version: RepositoryVersion,
    committed_objects: CommittedRepositoryObjects,
    committed_objects_digest: Sha256Digest,
    atomic_commit_safety_capability_id: CapabilityRowId,
}

impl ValidatedCommitObjectAuthority {
    #[cfg(test)]
    pub(crate) fn from_commit_scope(
        scope: CommitScopedAtomicSafetyAuthority,
        observation: CommitObjectPostStateObservationAuthority,
    ) -> Result<Self, RepositoryResultContractError> {
        let CommitScopedAtomicSafetyAuthority {
            approved,
            completion,
            immediate_history_guard_evidence,
            post_merge_repository_anchor,
            before_repository_cursor,
            ..
        } = scope;
        Self::from_retained_lineage(
            approved,
            observation,
            Some((
                completion,
                immediate_history_guard_evidence,
                post_merge_repository_anchor,
                before_repository_cursor,
            )),
        )
    }

    #[cfg(test)]
    fn from_approved_lineage_test_only(
        approved_preview: ApprovedCommitPreviewAuthority,
        observation: CommitObjectPostStateObservationAuthority,
    ) -> Result<Self, RepositoryResultContractError> {
        Self::from_retained_lineage(approved_preview, observation, None)
    }

    #[cfg(test)]
    fn from_retained_lineage(
        approved_preview: ApprovedCommitPreviewAuthority,
        observation: CommitObjectPostStateObservationAuthority,
        retained_scope: Option<(
            CommitImmediateRecheckCompletion,
            PostMergeHistoryGuardEvidence,
            RepositoryAnchor,
            RepositoryHistoryCursor,
        )>,
    ) -> Result<Self, RepositoryResultContractError> {
        let planned = &approved_preview.0.record.exact_objects;
        let observed = observation.committed_objects.exact_objects();
        if planned != &observed {
            return Err(RepositoryResultContractError(
                "committed-object target/action projection differs from the approved plan",
            ));
        }
        if !observation
            .committed_objects
            .all_versions_match(&observation.repository_version)
        {
            return Err(RepositoryResultContractError(
                "committed-object version does not equal the task commit version",
            ));
        }
        if approved_preview
            .0
            .record
            .history_guard_evidence
            .atomic_commit_safety_capability_id()
            != &observation.atomic_commit_safety_capability_id
        {
            return Err(RepositoryResultContractError(
                "post-commit observation uses another atomic-safety capability",
            ));
        }
        let committed_objects_digest = result_digest(
            &CommittedObjectsDigestRecord {
                integration_set_digest: approved_preview.0.record.integration_set_digest.clone(),
                committed_objects: observation.committed_objects.clone(),
            },
            "committed-object digest failed",
        )?;
        let (
            commit_recheck_completion,
            immediate_history_guard_evidence,
            post_merge_repository_anchor,
            before_repository_cursor,
        ) = retained_scope.map_or((None, None, None, None), |retained| {
            (
                Some(retained.0),
                Some(retained.1),
                Some(retained.2),
                Some(retained.3),
            )
        });
        Ok(Self {
            #[cfg(test)]
            object_history_binding_witness: CommitAtomicCommitInvocationCapability::mint()
                .object_history_binding_witness(),
            approved_preview,
            _commit_recheck_completion: commit_recheck_completion,
            _immediate_history_guard_evidence: immediate_history_guard_evidence,
            _post_merge_repository_anchor: post_merge_repository_anchor,
            _before_repository_cursor: before_repository_cursor,
            repository_version: observation.repository_version,
            committed_objects: observation.committed_objects,
            committed_objects_digest,
            atomic_commit_safety_capability_id: observation.atomic_commit_safety_capability_id,
        })
    }

    pub(crate) fn repository_version(&self) -> &RepositoryVersion {
        &self.repository_version
    }

    pub(crate) fn committed_objects(&self) -> &CommittedRepositoryObjects {
        &self.committed_objects
    }

    pub(crate) fn committed_objects_digest(&self) -> &Sha256Digest {
        &self.committed_objects_digest
    }

    pub(crate) fn atomic_commit_safety_capability_id(&self) -> &CapabilityRowId {
        &self.atomic_commit_safety_capability_id
    }

    pub(crate) fn integration_set_digest(&self) -> &Sha256Digest {
        &self.approved_preview.0.record.integration_set_digest
    }

    pub(crate) fn exact_objects_digest(&self) -> &Sha256Digest {
        &self.approved_preview.0.record.exact_objects_digest
    }
}

#[cfg(test)]
impl commit_object_history_binding_sealed::Sealed for ValidatedCommitObjectAuthority {}

#[cfg(test)]
impl CommitObjectHistoryBinding for ValidatedCommitObjectAuthority {
    fn object_history_binding_witness(&self) -> &CommitObjectHistoryBindingWitness {
        &self.object_history_binding_witness
    }

    fn repository_version(&self) -> &RepositoryVersion {
        self.repository_version()
    }

    fn committed_objects_digest(&self) -> &Sha256Digest {
        self.committed_objects_digest()
    }

    fn atomic_commit_safety_capability_id(&self) -> &CapabilityRowId {
        self.atomic_commit_safety_capability_id()
    }
}

/// Cross-module fixture for the taskCommit history resolver. It traverses the
/// real plan, preview approval, exact projection, version and digest checks;
/// only the external adapter observations are cfg-only fixtures.
#[cfg(test)]
pub(crate) fn validated_commit_object_authority_fixture_test_only(
    repository_version: RepositoryVersion,
    atomic_commit_safety_capability_id: CapabilityRowId,
) -> ValidatedCommitObjectAuthority {
    use crate::domain::branched_development::contracts::artifacts::ConfigurationIdentity;
    use crate::domain::branched_development::contracts::repository::{
        empty_commit_history_evidence_fixture_test_only, RepositoryAnchorObservationAuthority,
    };
    use crate::domain::branched_development::contracts::scalars::{EmptyOrName, Name};

    fn digest(character: char) -> Sha256Digest {
        Sha256Digest::parse(&character.to_string().repeat(64)).unwrap()
    }

    fn id(value: &str) -> UnicaId {
        UnicaId::parse(value).unwrap()
    }

    let cursor = RepositoryHistoryCursor::new(repository_version.clone(), digest('a'));
    let (gate_history, history_guard) = empty_commit_history_evidence_fixture_test_only(
        cursor.clone(),
        digest('b'),
        digest('3'),
        atomic_commit_safety_capability_id.clone(),
    )
    .unwrap();
    let configuration_identity = ConfigurationIdentity::new(
        MetadataObjectId::parse("123e4567-e89b-12d3-a456-426614174000").unwrap(),
        Name::parse("Fixture configuration").unwrap(),
        EmptyOrName::parse("").unwrap(),
        EmptyOrName::parse("").unwrap(),
    );
    let anchor = RepositoryAnchorObservationAuthority::test_only(
        digest('d'),
        cursor.clone(),
        configuration_identity,
        digest('e'),
    )
    .into_anchor()
    .unwrap();
    let root = RepositoryTargetIdentity::configuration_root();
    let root_leaf = match root.clone() {
        RepositoryTargetIdentity::ConfigurationRoot(value) => value,
        RepositoryTargetIdentity::DevelopmentObject(_) => unreachable!(),
    };
    let integration_entries =
        RepositoryIntegrationEntries::new(vec![RepositoryIntegrationEntry::root_modify(
            root_leaf,
            RepositoryTargetDisplay::parse("Configuration root").unwrap(),
            RepositoryIntegrationReasons::new(vec![RepositoryIntegrationReason::CanonicalDelta])
                .unwrap(),
            CanonicalRepositoryTargets::new(vec![root.clone()]).unwrap(),
        )])
        .unwrap();
    let lock_entries = serde_json::from_value(serde_json::json!([{
        "targetKind": "configurationRoot",
        "objectDisplay": "Configuration root",
        "reasons": ["supportGraphGuard", "updateTarget"]
    }]))
    .unwrap();
    let plan = LockPlanData::from_authority(
        LockPlanAuthority::test_only(LockPlanAuthorityTestParts {
            plan_id: id("11111111-1111-4111-8111-111111111111"),
            merge_session_id: id("22222222-2222-4222-8222-222222222222"),
            resolved_session_digest: digest('f'),
            support_gate_id: id("33333333-3333-4333-8333-333333333333"),
            support_gate_digest: digest('1'),
            support_gate_history_evidence: gate_history.clone(),
            verification_id: id("44444444-4444-4444-8444-444444444444"),
            verification_digest: digest('2'),
            integration_set_id: id("55555555-5555-4555-8555-555555555555"),
            integration_entries,
            delete_self_lock_evidence: Vec::new(),
            lock_entries,
            relevant_anchors: RepositoryRelevantAnchors::new(vec![RepositoryRelevantAnchor::new(
                root.clone(),
                anchor,
            )])
            .unwrap(),
            compatibility_mode: CompatibilityMode::parse("Version8_3_24").unwrap(),
            reference_closure_digest: digest('3'),
            settings_digest: digest('4'),
            prevalidation_diagnostics_digest: digest('5'),
            gate_comparison_id: id("88888888-8888-4888-8888-888888888888"),
            gate_ordinary_result_artifact_id: id("99999999-9999-4999-8999-999999999999"),
            gate_result_digest: digest('0'),
            planner_capability_id: CapabilityRowId::parse("repository.lock-plan.fixture").unwrap(),
        })
        .unwrap(),
    )
    .unwrap();
    let preview_verification_digest = plan.verification_digest.clone();
    let preview = CommitPreviewAuthority::test_only(CommitPreviewAuthorityTestParts {
        plan,
        guard_locks: CanonicalRepositoryTargets::new(Vec::new()).unwrap(),
        comment: Comment::parse("Task fixture commit").unwrap(),
        verification_digest: preview_verification_digest,
        lock_set_digest: digest('6'),
        merge_receipt_id: id("66666666-6666-4666-8666-666666666666"),
        merge_receipt_cursor: cursor,
        consumed_support_gate_history_evidence: gate_history,
        authorized_post_merge_fingerprint: digest('7'),
        observed_original_fingerprint: digest('7'),
        history_guard_evidence: history_guard,
    })
    .unwrap();
    let approved_digest = preview.commit_digest().clone();
    let approved =
        ApprovedCommitPreviewAuthority::approve_test_only(preview, &approved_digest).unwrap();
    let committed_objects =
        CommittedRepositoryObjects::new(vec![CommittedRepositoryObject::root_modify(
            repository_version.clone(),
            digest('8'),
        )])
        .unwrap();
    ValidatedCommitObjectAuthority::from_approved_lineage_test_only(
        approved,
        CommitObjectPostStateObservationAuthority::from_atomic_adapter(
            repository_version,
            committed_objects,
            atomic_commit_safety_capability_id,
        ),
    )
    .unwrap()
}

/// Cross-module fixture for merge-result lifecycle tests. It starts from the
/// same complete canonical plan used by commit tests, replaces only the
/// caller-observed merge/gate lineage, and recomputes every dependent digest.
#[cfg(test)]
pub(crate) fn original_merge_lock_projection_fixture_test_only(
    merge_session_id: UnicaId,
    resolved_session_digest: Sha256Digest,
    support_gate_id: UnicaId,
    support_gate_digest: Sha256Digest,
    support_gate_history_evidence: SupportGateHistoryEvidence,
    settings_digest: Sha256Digest,
) -> ValidatedOriginalMergeLockProjection {
    let committed = validated_commit_object_authority_fixture_test_only(
        RepositoryVersion::parse("101").unwrap(),
        CapabilityRowId::parse("repository.atomic-commit.merge-fixture").unwrap(),
    );
    let mut plan = committed.approved_preview.0.plan;
    plan.merge_session_id = merge_session_id;
    plan.resolved_session_digest = resolved_session_digest;
    plan.support_gate_id = support_gate_id;
    plan.support_gate_digest = support_gate_digest;
    plan.support_gate_history_evidence = support_gate_history_evidence;
    plan.settings_digest = settings_digest;
    plan.integration_set_digest = result_digest(
        &IntegrationSetDigestRecord {
            merge_session_id: plan.merge_session_id.clone(),
            resolved_session_digest: plan.resolved_session_digest.clone(),
            support_gate_id: plan.support_gate_id.clone(),
            support_gate_digest: plan.support_gate_digest.clone(),
            support_gate_history_evidence_digest: plan
                .support_gate_history_evidence
                .evidence_digest()
                .clone(),
            verification_id: plan.verification_id.clone(),
            verification_digest: plan.verification_digest.clone(),
            integration_entries: plan.integration_entries.clone(),
            compatibility_mode: plan.compatibility_mode.clone(),
            reference_closure_digest: plan.reference_closure_digest.clone(),
            settings_digest: plan.settings_digest.clone(),
            prevalidation_diagnostics_digest: plan.prevalidation_diagnostics_digest.clone(),
        },
        "merge fixture integration-set digest failed",
    )
    .unwrap();
    plan.plan_digest = result_digest(
        &LockPlanDigestRecord {
            plan_id: plan.plan_id.clone(),
            merge_session_id: plan.merge_session_id.clone(),
            resolved_session_digest: plan.resolved_session_digest.clone(),
            support_gate_id: plan.support_gate_id.clone(),
            support_gate_digest: plan.support_gate_digest.clone(),
            support_gate_history_evidence: plan.support_gate_history_evidence.clone(),
            verification_id: plan.verification_id.clone(),
            verification_digest: plan.verification_digest.clone(),
            integration_set_id: plan.integration_set_id.clone(),
            integration_entries: plan.integration_entries.clone(),
            integration_set_digest: plan.integration_set_digest.clone(),
            lock_entries: plan.lock_entries.clone(),
            relevant_anchors: plan.relevant_anchors.clone(),
            compatibility_mode: plan.compatibility_mode.clone(),
            reference_closure_digest: plan.reference_closure_digest.clone(),
            settings_digest: plan.settings_digest.clone(),
            prevalidation_diagnostics_digest: plan.prevalidation_diagnostics_digest.clone(),
        },
        "merge fixture lock-plan digest failed",
    )
    .unwrap();
    let lock_set_id = UnicaId::parse("77777777-7777-4777-8777-777777777777").unwrap();
    let lock_set_digest = result_digest(
        &LockSetDigestRecord {
            plan_digest: plan.plan_digest.clone(),
            integration_set_id: plan.integration_set_id.clone(),
            integration_set_digest: plan.integration_set_digest.clone(),
            acquired: plan.lock_entries.clone(),
        },
        "merge fixture lock-set digest failed",
    )
    .unwrap();
    ValidatedOriginalMergeLockProjection::test_only(plan, lock_set_id, lock_set_digest)
}

/// B2 tests need a real production-shaped B1 projection: the fixture variant
/// must never satisfy the immediate pre-intent check. This helper constructs
/// the production enum arm with an authority resolved through the same typed
/// current-state lease used by B1; no production constructor is opened.
#[cfg(test)]
pub(crate) fn original_merge_production_lock_projection_fixture_test_only(
    merge_session_id: UnicaId,
    resolved_session_digest: Sha256Digest,
    ready: crate::domain::branched_development::contracts::support::ReadySupportPreflightAuthority,
    settings_digest: Sha256Digest,
) -> ValidatedOriginalMergeLockProjection {
    original_merge_production_lock_projection_with_revision_fixture_test_only(
        merge_session_id,
        resolved_session_digest,
        ready,
        settings_digest,
        Sha256Digest::parse(&"b".repeat(64)).unwrap(),
    )
}

#[cfg(test)]
pub(crate) fn original_merge_production_lock_projection_with_revision_fixture_test_only(
    merge_session_id: UnicaId,
    resolved_session_digest: Sha256Digest,
    ready: crate::domain::branched_development::contracts::support::ReadySupportPreflightAuthority,
    settings_digest: Sha256Digest,
    current_state_revision: Sha256Digest,
) -> ValidatedOriginalMergeLockProjection {
    use crate::domain::branched_development::contracts::support::{
        CurrentReadySupportGateResolutionRequest, CurrentReadySupportGateStateLease,
        CurrentReadySupportGateStateResolver, SupportContractError,
    };

    struct FixtureCurrentLease {
        history: SupportGateHistoryEvidence,
        revision: Sha256Digest,
    }

    impl CurrentReadySupportGateStateLease for FixtureCurrentLease {
        fn binds(&self, _request: &CurrentReadySupportGateResolutionRequest<'_>) -> bool {
            true
        }

        fn persisted_history_evidence(&self) -> &SupportGateHistoryEvidence {
            &self.history
        }

        fn current_state_revision(&self) -> &Sha256Digest {
            &self.revision
        }
    }

    struct FixtureCurrentResolver {
        history: SupportGateHistoryEvidence,
        revision: Sha256Digest,
    }

    impl CurrentReadySupportGateStateResolver for FixtureCurrentResolver {
        fn resolve_latest_non_invalidated_current_ready(
            &mut self,
            _request: &CurrentReadySupportGateResolutionRequest<'_>,
        ) -> Result<Box<dyn CurrentReadySupportGateStateLease>, SupportContractError> {
            Ok(Box::new(FixtureCurrentLease {
                history: self.history.clone(),
                revision: self.revision.clone(),
            }))
        }
    }

    let support_gate_id = ready.support_gate_id().clone();
    let support_gate_digest = ready.support_gate_digest().clone();
    let history = ready.history_evidence().clone();
    let comparison_id = ready.comparison_id().clone();
    let ordinary_result_artifact_id = ready.ordinary_result_artifact_id().clone();
    let gate_result_digest = ready.sandbox_result_digest().clone();
    let original_fingerprint = ready.original_fingerprint().clone();
    let projection = original_merge_lock_projection_fixture_test_only(
        merge_session_id,
        resolved_session_digest,
        support_gate_id,
        support_gate_digest,
        history.clone(),
        settings_digest,
    );
    let mut plan = projection.plan;
    let planner_capability_id = plan.gate_session_lineage.planner_capability_id.clone();
    plan.gate_session_lineage = LockPlanGateSessionLineage {
        comparison_id,
        ordinary_result_artifact_id,
        result_digest: gate_result_digest,
        planner_capability_id,
    };
    let previous_root = plan
        .relevant_anchors
        .as_slice()
        .iter()
        .find(|value| value.target() == &RepositoryTargetIdentity::configuration_root())
        .unwrap();
    let refreshed_root = crate::domain::branched_development::contracts::repository::RepositoryAnchorObservationAuthority::test_only(
        previous_root.anchor().repository_identity().clone(),
        history.classified_through_cursor().clone(),
        previous_root.anchor().configuration_identity().clone(),
        original_fingerprint,
    )
    .into_anchor()
    .unwrap();
    plan.relevant_anchors = RepositoryRelevantAnchors::new(vec![RepositoryRelevantAnchor::new(
        RepositoryTargetIdentity::configuration_root(),
        refreshed_root,
    )])
    .unwrap();
    plan.integration_set_digest = result_digest(
        &IntegrationSetDigestRecord {
            merge_session_id: plan.merge_session_id.clone(),
            resolved_session_digest: plan.resolved_session_digest.clone(),
            support_gate_id: plan.support_gate_id.clone(),
            support_gate_digest: plan.support_gate_digest.clone(),
            support_gate_history_evidence_digest: plan
                .support_gate_history_evidence
                .evidence_digest()
                .clone(),
            verification_id: plan.verification_id.clone(),
            verification_digest: plan.verification_digest.clone(),
            integration_entries: plan.integration_entries.clone(),
            compatibility_mode: plan.compatibility_mode.clone(),
            reference_closure_digest: plan.reference_closure_digest.clone(),
            settings_digest: plan.settings_digest.clone(),
            prevalidation_diagnostics_digest: plan.prevalidation_diagnostics_digest.clone(),
        },
        "production merge fixture integration-set digest failed",
    )
    .unwrap();
    plan.plan_digest = result_digest(
        &LockPlanDigestRecord {
            plan_id: plan.plan_id.clone(),
            merge_session_id: plan.merge_session_id.clone(),
            resolved_session_digest: plan.resolved_session_digest.clone(),
            support_gate_id: plan.support_gate_id.clone(),
            support_gate_digest: plan.support_gate_digest.clone(),
            support_gate_history_evidence: plan.support_gate_history_evidence.clone(),
            verification_id: plan.verification_id.clone(),
            verification_digest: plan.verification_digest.clone(),
            integration_set_id: plan.integration_set_id.clone(),
            integration_entries: plan.integration_entries.clone(),
            integration_set_digest: plan.integration_set_digest.clone(),
            lock_entries: plan.lock_entries.clone(),
            relevant_anchors: plan.relevant_anchors.clone(),
            compatibility_mode: plan.compatibility_mode.clone(),
            reference_closure_digest: plan.reference_closure_digest.clone(),
            settings_digest: plan.settings_digest.clone(),
            prevalidation_diagnostics_digest: plan.prevalidation_diagnostics_digest.clone(),
        },
        "production merge fixture lock-plan digest failed",
    )
    .unwrap();
    let lock_set_id = projection.lock_set_id;
    let lock_set_digest = result_digest(
        &LockSetDigestRecord {
            plan_digest: plan.plan_digest.clone(),
            integration_set_id: plan.integration_set_id.clone(),
            integration_set_digest: plan.integration_set_digest.clone(),
            acquired: plan.lock_entries.clone(),
        },
        "production merge fixture lock-set digest failed",
    )
    .unwrap();
    let journaled_lock_receipts = plan
        .lock_entries
        .as_slice()
        .iter()
        .map(|target| {
            let target_display = match target.as_ref() {
                RepositoryUpdateLockTargetRef::ConfigurationRoot { object_display, .. }
                | RepositoryUpdateLockTargetRef::DevelopmentObject { object_display, .. } => {
                    object_display.clone()
                }
            };
            JournaledRepositoryLock::new(
                planned_lock_target_identity(target),
                target_display,
                lock_set_id.clone(),
                NormalizedUtcInstant::parse("2026-07-23T01:00:00Z").unwrap(),
            )
        })
        .collect::<Vec<_>>();
    let root_lock_receipt = journaled_lock_receipts.first().unwrap().clone();
    let current_gate = CurrentReadySupportGateAuthority::resolve(
        ready,
        &mut FixtureCurrentResolver {
            history,
            revision: current_state_revision,
        },
    )
    .unwrap();
    ValidatedOriginalMergeLockProjection {
        plan,
        lock_set_id,
        lock_set_digest,
        gate_proof: ValidatedLockGateProof::Production {
            current_gate: Box::new(current_gate),
            root_lock_receipt,
            journaled_lock_receipts,
            root_reread_capability_id: CapabilityRowId::parse(
                "repository.root-reread.merge-fixture",
            )
            .unwrap(),
        },
    }
}

/// Produces a second internally consistent production-shaped acquisition
/// lineage for cross-invocation splice tests. The override is cfg-only; all
/// production constructors remain closed.
#[cfg(test)]
pub(crate) fn original_merge_production_lock_projection_with_identity_fixture_test_only(
    merge_session_id: UnicaId,
    resolved_session_digest: Sha256Digest,
    ready: crate::domain::branched_development::contracts::support::ReadySupportPreflightAuthority,
    settings_digest: Sha256Digest,
    lock_set_id: UnicaId,
    observed_at: NormalizedUtcInstant,
) -> ValidatedOriginalMergeLockProjection {
    let mut projection = original_merge_production_lock_projection_with_revision_fixture_test_only(
        merge_session_id,
        resolved_session_digest,
        ready,
        settings_digest,
        Sha256Digest::parse(&"b".repeat(64)).unwrap(),
    );
    projection.lock_set_id = lock_set_id.clone();
    let ValidatedLockGateProof::Production {
        root_lock_receipt,
        journaled_lock_receipts,
        ..
    } = &mut projection.gate_proof
    else {
        unreachable!("production-shaped lock projection must retain production gate proof")
    };
    for receipt in journaled_lock_receipts.iter_mut() {
        receipt.lock_set_id = lock_set_id.clone();
        receipt.observed_at = observed_at.clone();
    }
    *root_lock_receipt = journaled_lock_receipts
        .first()
        .expect("production lock fixture always contains the root receipt")
        .clone();
    projection
}

#[cfg(test)]
mod gate_b1_tests {
    use super::*;
    use crate::domain::branched_development::contracts::artifacts::ConfigurationIdentity;
    use crate::domain::branched_development::contracts::repository::RepositoryAnchorObservationAuthority;
    use crate::domain::branched_development::contracts::scalars::{EmptyOrName, Name};
    use crate::domain::branched_development::contracts::support::{
        ready_preflight_authority_fixture_test_only, CurrentReadySupportGateAuthority,
        CurrentReadySupportGateResolutionRequest, CurrentReadySupportGateStateLease,
        CurrentReadySupportGateStateResolver, SupportContractError,
    };
    use serde_json::json;
    use std::cell::RefCell;
    use std::rc::Rc;

    macro_rules! assert_not_clone {
        ($type:ty) => {
            const _: fn() = || {
                trait AmbiguousIfClone<Marker> {
                    fn assert_not_clone() {}
                }
                struct ImplementsClone;
                impl<T: ?Sized> AmbiguousIfClone<()> for T {}
                impl<T: Clone> AmbiguousIfClone<ImplementsClone> for T {}
                let _ = <$type as AmbiguousIfClone<_>>::assert_not_clone;
            };
        };
    }

    macro_rules! assert_not_serialize {
        ($type:ty) => {
            const _: fn() = || {
                trait AmbiguousIfSerialize<Marker> {
                    fn assert_not_serialize() {}
                }
                struct ImplementsSerialize;
                impl<T: ?Sized> AmbiguousIfSerialize<()> for T {}
                impl<T: ?Sized + serde::Serialize>
                    AmbiguousIfSerialize<ImplementsSerialize> for T
                {
                }
                let _ = <$type as AmbiguousIfSerialize<_>>::assert_not_serialize;
            };
        };
    }

    macro_rules! assert_not_deserialize_owned {
        ($type:ty) => {
            const _: fn() = || {
                trait AmbiguousIfDeserialize<Marker> {
                    fn assert_not_deserialize() {}
                }
                struct ImplementsDeserialize;
                impl<T: ?Sized> AmbiguousIfDeserialize<()> for T {}
                impl<T: serde::de::DeserializeOwned> AmbiguousIfDeserialize<ImplementsDeserialize>
                    for T
                {
                }
                let _ = <$type as AmbiguousIfDeserialize<_>>::assert_not_deserialize;
            };
        };
    }

    assert_not_clone!(CurrentReadySupportGateAuthority);
    assert_not_clone!(PreLockCurrentGateAuthority);
    assert_not_clone!(RootGuardedCurrentGateAuthority);
    assert_not_clone!(RootGuardAcquisitionBlockedAuthority);
    assert_not_clone!(VerifiedRootGateDriftStopAuthority);
    assert_not_clone!(RootLockConflictStopAuthority);
    assert_not_clone!(RootLockRecoveryAuthority);
    assert_not_clone!(RemainingLockAcquisitionBlockedAuthority);
    assert_not_clone!(RemainingLockConflictStopAuthority);
    assert_not_clone!(RemainingLockRecoveryAuthority);
    assert_not_serialize!(CurrentReadySupportGateAuthority);
    assert_not_serialize!(PreLockCurrentGateAuthority);
    assert_not_serialize!(RootGuardedCurrentGateAuthority);
    assert_not_serialize!(RootGuardAcquisitionBlockedAuthority);
    assert_not_serialize!(VerifiedRootGateDriftStopAuthority);
    assert_not_serialize!(RootLockConflictStopAuthority);
    assert_not_serialize!(RootLockRecoveryAuthority);
    assert_not_serialize!(RemainingLockAcquisitionBlockedAuthority);
    assert_not_serialize!(RemainingLockConflictStopAuthority);
    assert_not_serialize!(RemainingLockRecoveryAuthority);
    assert_not_deserialize_owned!(CurrentReadySupportGateAuthority);
    assert_not_deserialize_owned!(PreLockCurrentGateAuthority);
    assert_not_deserialize_owned!(RootGuardedCurrentGateAuthority);
    assert_not_deserialize_owned!(RootGuardAcquisitionBlockedAuthority);
    assert_not_deserialize_owned!(VerifiedRootGateDriftStopAuthority);
    assert_not_deserialize_owned!(RootLockConflictStopAuthority);
    assert_not_deserialize_owned!(RootLockRecoveryAuthority);
    assert_not_deserialize_owned!(RemainingLockAcquisitionBlockedAuthority);
    assert_not_deserialize_owned!(RemainingLockConflictStopAuthority);
    assert_not_deserialize_owned!(RemainingLockRecoveryAuthority);

    const OBJECT: &str = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
    const OBJECT_B: &str = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";

    fn digest(character: char) -> Sha256Digest {
        Sha256Digest::parse(&character.to_string().repeat(64)).unwrap()
    }

    fn id(value: &str) -> UnicaId {
        UnicaId::parse(value).unwrap()
    }

    fn object() -> MetadataObjectId {
        MetadataObjectId::parse(OBJECT).unwrap()
    }

    fn object_b() -> MetadataObjectId {
        MetadataObjectId::parse(OBJECT_B).unwrap()
    }

    fn display(value: &str) -> RepositoryTargetDisplay {
        RepositoryTargetDisplay::parse(value).unwrap()
    }

    #[derive(Default)]
    struct GateEvents(Rc<RefCell<Vec<String>>>);

    impl GateEvents {
        fn push(&self, value: impl Into<String>) {
            self.0.borrow_mut().push(value.into());
        }

        fn values(&self) -> Vec<String> {
            self.0.borrow().clone()
        }
    }

    struct TestCurrentGateLease {
        history: SupportGateHistoryEvidence,
        state_revision: Sha256Digest,
        binds: bool,
    }

    impl CurrentReadySupportGateStateLease for TestCurrentGateLease {
        fn binds(&self, _request: &CurrentReadySupportGateResolutionRequest<'_>) -> bool {
            self.binds
        }

        fn persisted_history_evidence(&self) -> &SupportGateHistoryEvidence {
            &self.history
        }

        fn current_state_revision(&self) -> &Sha256Digest {
            &self.state_revision
        }
    }

    struct TestCurrentGateResolver {
        events: GateEvents,
        stale: bool,
    }

    impl CurrentReadySupportGateStateResolver for TestCurrentGateResolver {
        fn resolve_latest_non_invalidated_current_ready(
            &mut self,
            request: &CurrentReadySupportGateResolutionRequest<'_>,
        ) -> Result<Box<dyn CurrentReadySupportGateStateLease>, SupportContractError> {
            self.events.push("current-gate-recheck");
            Ok(Box::new(TestCurrentGateLease {
                history: request.candidate_history_evidence().clone(),
                state_revision: digest('9'),
                binds: !self.stale,
            }))
        }
    }

    fn current_gate(
        events: GateEvents,
    ) -> Result<CurrentReadySupportGateAuthority, SupportContractError> {
        CurrentReadySupportGateAuthority::resolve(
            ready_preflight_authority_fixture_test_only(),
            &mut TestCurrentGateResolver {
                events,
                stale: false,
            },
        )
    }

    fn plan_for(current: &CurrentReadySupportGateAuthority) -> LockPlanData {
        plan_for_objects(current, &[(object(), "Catalog.B1")])
    }

    fn plan_for_objects(
        current: &CurrentReadySupportGateAuthority,
        objects: &[(MetadataObjectId, &str)],
    ) -> LockPlanData {
        let root = RepositoryTargetIdentity::configuration_root();
        let mut entries = vec![RepositoryIntegrationEntry::root_modify(
            RootTargetIdentity::new(),
            display("Configuration root"),
            RepositoryIntegrationReasons::new(vec![RepositoryIntegrationReason::CanonicalDelta])
                .unwrap(),
            CanonicalRepositoryTargets::new(vec![root.clone()]).unwrap(),
        )];
        let mut lock_entries = vec![json!({
            "targetKind": "configurationRoot",
            "objectDisplay": "Configuration root",
            "reasons": ["supportGraphGuard", "updateTarget"]
        })];
        for (object_id, object_display) in objects {
            let target = RepositoryTargetIdentity::DevelopmentObject(ObjectTargetIdentity::new(
                object_id.clone(),
            ));
            entries.push(RepositoryIntegrationEntry::object_modify(
                ObjectTargetIdentity::new(object_id.clone()),
                display(object_display),
                RepositoryIntegrationReasons::new(vec![
                    RepositoryIntegrationReason::CanonicalDelta,
                ])
                .unwrap(),
                CanonicalRepositoryTargets::new(vec![target]).unwrap(),
            ));
            lock_entries.push(json!({
                "targetKind": "developmentObject",
                "objectId": object_id.as_str(),
                "objectDisplay": object_display,
                "reasons": ["updateTarget"]
            }));
        }
        let entries = RepositoryIntegrationEntries::new(entries).unwrap();
        let lock_entries = serde_json::from_value(serde_json::Value::Array(lock_entries)).unwrap();
        let anchor = RepositoryAnchorObservationAuthority::test_only(
            digest('8'),
            current
                .history_evidence()
                .classified_through_cursor()
                .clone(),
            ConfigurationIdentity::new(
                MetadataObjectId::parse("123e4567-e89b-12d3-a456-426614174000").unwrap(),
                Name::parse("B1 configuration").unwrap(),
                EmptyOrName::parse("").unwrap(),
                EmptyOrName::parse("").unwrap(),
            ),
            current.original_fingerprint().clone(),
        )
        .into_anchor()
        .unwrap();
        LockPlanData::from_authority(
            LockPlanAuthority::test_only(LockPlanAuthorityTestParts {
                plan_id: id("11111111-1111-4111-8111-111111111111"),
                merge_session_id: id("22222222-2222-4222-8222-222222222222"),
                resolved_session_digest: digest('7'),
                support_gate_id: current.support_gate_id().clone(),
                support_gate_digest: current.support_gate_digest().clone(),
                support_gate_history_evidence: current.history_evidence().clone(),
                verification_id: id("33333333-3333-4333-8333-333333333333"),
                verification_digest: digest('6'),
                integration_set_id: id("44444444-4444-4444-8444-444444444444"),
                integration_entries: entries,
                delete_self_lock_evidence: Vec::new(),
                lock_entries,
                relevant_anchors: RepositoryRelevantAnchors::new(vec![
                    RepositoryRelevantAnchor::new(root, anchor),
                ])
                .unwrap(),
                compatibility_mode: CompatibilityMode::parse("Version8_3_24").unwrap(),
                reference_closure_digest: digest('5'),
                settings_digest: current.settings_digest().clone(),
                prevalidation_diagnostics_digest: digest('4'),
                gate_comparison_id: current.comparison_id().clone(),
                gate_ordinary_result_artifact_id: current.ordinary_result_artifact_id().clone(),
                gate_result_digest: current.sandbox_result_digest().clone(),
                planner_capability_id: CapabilityRowId::parse("repository.lock-plan.fixture")
                    .unwrap(),
            })
            .unwrap(),
        )
        .unwrap()
    }

    struct TestRootLease {
        events: GateEvents,
        receipt: JournaledRepositoryLock,
        gate_id: UnicaId,
        gate_digest: Sha256Digest,
        history: SupportGateHistoryEvidence,
        support_graph_digest: Sha256Digest,
        relevant_baseline_digest: Sha256Digest,
        original_fingerprint: Sha256Digest,
        state_revision: Sha256Digest,
        reread_capability_id: CapabilityRowId,
        release_verified: bool,
        wrong_release_identity: bool,
        pre_reread_lock_observation: RootGuardPreRereadLockObservation,
        additional_retained_on_release: Vec<JournaledRepositoryLock>,
        complete_support_graph_reread: bool,
    }

    impl RootGuardAcquisitionLease for TestRootLease {
        fn root_lock_receipt(&self) -> &JournaledRepositoryLock {
            &self.receipt
        }

        fn pre_reread_lock_observation(&self) -> &RootGuardPreRereadLockObservation {
            &self.pre_reread_lock_observation
        }

        fn reread_support_gate_id(&self) -> &UnicaId {
            &self.gate_id
        }

        fn reread_support_gate_digest(&self) -> &Sha256Digest {
            &self.gate_digest
        }

        fn reread_history_evidence(&self) -> &SupportGateHistoryEvidence {
            &self.history
        }

        fn complete_support_graph_was_reread(&self) -> bool {
            self.complete_support_graph_reread
        }

        fn reread_support_graph_digest(&self) -> &Sha256Digest {
            &self.support_graph_digest
        }

        fn reread_relevant_baseline_digest(&self) -> &Sha256Digest {
            &self.relevant_baseline_digest
        }

        fn reread_original_fingerprint(&self) -> &Sha256Digest {
            &self.original_fingerprint
        }

        fn reread_state_revision(&self) -> &Sha256Digest {
            &self.state_revision
        }

        fn root_reread_capability_id(&self) -> &CapabilityRowId {
            &self.reread_capability_id
        }

        fn release_root(self: Box<Self>) -> RootGuardReleaseObservation {
            self.events.push("release-root");
            if self.release_verified {
                let released = if self.wrong_release_identity {
                    JournaledRepositoryLock::new(
                        self.receipt.target.clone(),
                        self.receipt.target_display.clone(),
                        self.receipt.lock_set_id.clone(),
                        NormalizedUtcInstant::parse("2026-07-22T00:00:09Z").unwrap(),
                    )
                } else {
                    self.receipt.clone()
                };
                RootGuardReleaseObservation::verified_from_repository_adapter(
                    vec![released],
                    self.additional_retained_on_release.clone(),
                    CapabilityRowId::parse("repository.root-release.verified").unwrap(),
                )
            } else {
                RootGuardReleaseObservation::unverified_from_repository_adapter(
                    Vec::new(),
                    std::iter::once(self.receipt.clone())
                        .chain(self.additional_retained_on_release.iter().cloned())
                        .collect(),
                    CapabilityRowId::parse("repository.root-release.recovery").unwrap(),
                )
            }
        }
    }

    #[derive(Clone, Copy)]
    enum TestRootPortMode {
        Held,
        FirstRootConflict,
        MalformedFirstRootConflict,
        UnknownFailure,
    }

    #[derive(Clone, Copy)]
    enum TestSemanticDrift {
        None,
        GateId,
        GateDigest,
        History,
        SupportGraph,
        RelevantBaseline,
        Fingerprint,
        StateRevision,
    }

    struct TestRootPort {
        events: GateEvents,
        mode: TestRootPortMode,
        semantic_drift: TestSemanticDrift,
        release_verified: bool,
        wrong_release_identity: bool,
        wrong_receipt_identity: bool,
        display_only_change: bool,
        journaled_before_reread: bool,
        prior_object_attempt: bool,
        complete_support_graph_reread: bool,
        calls: usize,
    }

    impl RootGuardAcquisitionPort for TestRootPort {
        fn acquire_root_and_reread_current_gate(
            &mut self,
            request: &PreLockCurrentGateAuthority,
        ) -> RootGuardAcquisitionPortOutcome {
            self.calls += 1;
            self.events.push("acquire-root");
            if matches!(self.mode, TestRootPortMode::FirstRootConflict) {
                return RootGuardAcquisitionPortOutcome::FirstRootConflict(
                    RootFirstLockConflictObservation::from_repository_adapter(
                        RepositoryTargetIdentity::configuration_root(),
                        vec![RepositoryTargetIdentity::configuration_root()],
                        Vec::new(),
                        CapabilityRowId::parse("repository.root-conflict.b1").unwrap(),
                    ),
                );
            }
            if matches!(self.mode, TestRootPortMode::MalformedFirstRootConflict) {
                let acquired = JournaledRepositoryLock::new(
                    RepositoryTargetIdentity::configuration_root(),
                    request.root_target_display().clone(),
                    id("55555555-5555-4555-8555-555555555555"),
                    NormalizedUtcInstant::parse("2026-07-22T00:00:00Z").unwrap(),
                );
                return RootGuardAcquisitionPortOutcome::FirstRootConflict(
                    RootFirstLockConflictObservation::from_repository_adapter(
                        RepositoryTargetIdentity::configuration_root(),
                        vec![RepositoryTargetIdentity::configuration_root()],
                        vec![acquired],
                        CapabilityRowId::parse("repository.root-malformed-conflict.b1").unwrap(),
                    ),
                );
            }
            if matches!(self.mode, TestRootPortMode::UnknownFailure) {
                return RootGuardAcquisitionPortOutcome::Recovery(
                    RepositoryLockRecoveryObservation::from_repository_adapter(
                        None,
                        vec![RepositoryTargetIdentity::configuration_root()],
                        Vec::new(),
                        Some(RepositoryTargetIdentity::configuration_root()),
                        Vec::new(),
                        Vec::new(),
                        CapabilityRowId::parse("repository.root-unknown.b1").unwrap(),
                    ),
                );
            }
            let receipt = JournaledRepositoryLock::new(
                if self.wrong_receipt_identity {
                    RepositoryTargetIdentity::DevelopmentObject(ObjectTargetIdentity::new(object()))
                } else {
                    RepositoryTargetIdentity::configuration_root()
                },
                if self.display_only_change {
                    display("Presentation changed after planning")
                } else {
                    request.root_target_display().clone()
                },
                id("55555555-5555-4555-8555-555555555555"),
                NormalizedUtcInstant::parse("2026-07-22T00:00:00Z").unwrap(),
            );
            self.events.push("journal-root");
            self.events
                .push(format!("reread-under:{}", receipt.lock_set_id().as_str()));
            let mut history = request.current_gate().history_evidence().clone();
            if matches!(self.semantic_drift, TestSemanticDrift::History) {
                history = validated_commit_object_authority_fixture_test_only(
                    RepositoryVersion::parse("b1-history-drift").unwrap(),
                    CapabilityRowId::parse("repository.atomic-commit.b1-history-drift").unwrap(),
                )
                .approved_preview
                .0
                .plan
                .support_gate_history_evidence;
            }
            let additional_retained_on_release = if self.prior_object_attempt {
                vec![JournaledRepositoryLock::new(
                    RepositoryTargetIdentity::DevelopmentObject(
                        ObjectTargetIdentity::new(object()),
                    ),
                    display("Catalog.B1"),
                    receipt.lock_set_id().clone(),
                    NormalizedUtcInstant::parse("2026-07-22T00:00:01Z").unwrap(),
                )]
            } else {
                Vec::new()
            };
            let attempted_targets = std::iter::once(RepositoryTargetIdentity::configuration_root())
                .chain(
                    additional_retained_on_release
                        .iter()
                        .map(|lock| lock.target().clone()),
                )
                .collect();
            let journaled_acquired = if self.journaled_before_reread {
                std::iter::once(receipt.clone())
                    .chain(additional_retained_on_release.iter().cloned())
                    .collect()
            } else {
                Vec::new()
            };
            let reported_retained = std::iter::once(receipt.clone())
                .chain(additional_retained_on_release.iter().cloned())
                .collect();
            let pre_reread_lock_observation =
                RootGuardPreRereadLockObservation::from_repository_adapter(
                    receipt.lock_set_id().clone(),
                    attempted_targets,
                    journaled_acquired,
                    Vec::new(),
                    reported_retained,
                );
            RootGuardAcquisitionPortOutcome::Held(Box::new(TestRootLease {
                events: GateEvents(self.events.0.clone()),
                receipt,
                gate_id: if matches!(self.semantic_drift, TestSemanticDrift::GateId) {
                    id("aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee")
                } else {
                    request.current_gate().support_gate_id().clone()
                },
                gate_digest: if matches!(self.semantic_drift, TestSemanticDrift::GateDigest) {
                    digest('2')
                } else {
                    request.current_gate().support_gate_digest().clone()
                },
                history,
                support_graph_digest: if matches!(
                    self.semantic_drift,
                    TestSemanticDrift::SupportGraph
                ) {
                    digest('2')
                } else {
                    request.current_gate().support_graph_digest().clone()
                },
                relevant_baseline_digest: if matches!(
                    self.semantic_drift,
                    TestSemanticDrift::RelevantBaseline
                ) {
                    digest('3')
                } else {
                    request.current_gate().relevant_baseline_digest().clone()
                },
                original_fingerprint: if matches!(
                    self.semantic_drift,
                    TestSemanticDrift::Fingerprint
                ) {
                    digest('4')
                } else {
                    request.current_gate().original_fingerprint().clone()
                },
                state_revision: if matches!(self.semantic_drift, TestSemanticDrift::StateRevision) {
                    digest('5')
                } else {
                    request.current_gate().current_state_revision().clone()
                },
                reread_capability_id: CapabilityRowId::parse("repository.root-reread.b1").unwrap(),
                release_verified: self.release_verified,
                wrong_release_identity: self.wrong_release_identity,
                pre_reread_lock_observation,
                additional_retained_on_release,
                complete_support_graph_reread: self.complete_support_graph_reread,
            }))
        }
    }

    #[derive(Clone, Copy)]
    enum TestRemainingMode {
        Success,
        PartialConflict,
        FailedCompensation,
        ReorderedReceipts,
        MissingReceipt,
        DuplicateReceipt,
        CrossLockSetReceipt,
        DisplayOnlyChange,
        MalformedConflict,
        DuplicateMalformedConflictReports,
        UnknownEffect,
    }

    struct TestRemainingPort {
        events: GateEvents,
        mode: TestRemainingMode,
        calls: usize,
    }

    struct MultiPrefixConflictPort {
        reverse_compensation: bool,
    }

    impl RemainingLockAcquisitionPort for MultiPrefixConflictPort {
        fn acquire_remaining_locks(
            &mut self,
            root_guarded: &RootGuardedCurrentGateAuthority,
        ) -> RemainingLockAcquisitionPortOutcome {
            let targets = planned_lock_target_identities(root_guarded.plan());
            assert_eq!(targets.len(), 3);
            let lock_set_id = root_guarded.root_lock_receipt().lock_set_id().clone();
            let root_receipt = root_guarded.root_lock_receipt().clone();
            let first_object_receipt = JournaledRepositoryLock::new(
                targets[1].clone(),
                display("Catalog.B1"),
                lock_set_id.clone(),
                NormalizedUtcInstant::parse("2026-07-22T00:00:01Z").unwrap(),
            );
            let acquired = vec![root_receipt.clone(), first_object_receipt.clone()];
            let released = if self.reverse_compensation {
                vec![first_object_receipt, root_receipt]
            } else {
                vec![root_receipt, first_object_receipt]
            };
            RemainingLockAcquisitionPortOutcome::Conflict(Box::new(
                RemainingLockConflictObservation::from_repository_adapter(
                    RemainingLockConflictObservationInput {
                        lock_set_id,
                        attempted_targets: targets[..=2].to_vec(),
                        acquired,
                        failed_target: targets[2].clone(),
                        released,
                        retained: Vec::new(),
                        compensation_verified: true,
                        conflict_capability_id: CapabilityRowId::parse(
                            "repository.remaining-prefix-conflict.b1",
                        )
                        .unwrap(),
                        compensation_capability_id: CapabilityRowId::parse(
                            "repository.remaining-prefix-release.b1",
                        )
                        .unwrap(),
                    },
                ),
            ))
        }
    }

    impl RemainingLockAcquisitionPort for TestRemainingPort {
        fn acquire_remaining_locks(
            &mut self,
            root_guarded: &RootGuardedCurrentGateAuthority,
        ) -> RemainingLockAcquisitionPortOutcome {
            self.calls += 1;
            for _ in root_guarded.plan().lock_entries().as_slice().iter().skip(1) {
                self.events.push("acquire-object");
            }
            let lock_set_id = root_guarded.root_lock_receipt().lock_set_id().clone();
            let root_receipt = root_guarded.root_lock_receipt().clone();
            let object_target =
                RepositoryTargetIdentity::DevelopmentObject(ObjectTargetIdentity::new(object()));
            let object_receipt = JournaledRepositoryLock::new(
                object_target.clone(),
                display("Catalog.B1"),
                lock_set_id.clone(),
                NormalizedUtcInstant::parse("2026-07-22T00:00:01Z").unwrap(),
            );
            let attempted = vec![
                RepositoryTargetIdentity::configuration_root(),
                object_target.clone(),
            ];
            match self.mode {
                TestRemainingMode::Success => RemainingLockAcquisitionPortOutcome::Success(
                    RemainingLockSuccessObservation::from_repository_adapter(
                        lock_set_id,
                        attempted,
                        vec![root_receipt, object_receipt],
                    ),
                ),
                TestRemainingMode::PartialConflict => {
                    RemainingLockAcquisitionPortOutcome::Conflict(Box::new(
                        RemainingLockConflictObservation::from_repository_adapter(
                            RemainingLockConflictObservationInput {
                                lock_set_id,
                                attempted_targets: attempted,
                                acquired: vec![root_receipt.clone()],
                                failed_target: object_target,
                                released: vec![root_receipt],
                                retained: Vec::new(),
                                compensation_verified: true,
                                conflict_capability_id: CapabilityRowId::parse(
                                    "repository.remaining-conflict.b1",
                                )
                                .unwrap(),
                                compensation_capability_id: CapabilityRowId::parse(
                                    "repository.remaining-release.b1",
                                )
                                .unwrap(),
                            },
                        ),
                    ))
                }
                TestRemainingMode::FailedCompensation => {
                    RemainingLockAcquisitionPortOutcome::Conflict(Box::new(
                        RemainingLockConflictObservation::from_repository_adapter(
                            RemainingLockConflictObservationInput {
                                lock_set_id,
                                attempted_targets: attempted,
                                acquired: vec![root_receipt.clone()],
                                failed_target: object_target,
                                released: Vec::new(),
                                retained: vec![root_receipt],
                                compensation_verified: false,
                                conflict_capability_id: CapabilityRowId::parse(
                                    "repository.remaining-conflict.b1",
                                )
                                .unwrap(),
                                compensation_capability_id: CapabilityRowId::parse(
                                    "repository.remaining-release-unknown.b1",
                                )
                                .unwrap(),
                            },
                        ),
                    ))
                }
                TestRemainingMode::ReorderedReceipts => {
                    RemainingLockAcquisitionPortOutcome::Success(
                        RemainingLockSuccessObservation::from_repository_adapter(
                            lock_set_id,
                            attempted,
                            vec![object_receipt, root_receipt],
                        ),
                    )
                }
                TestRemainingMode::MissingReceipt => RemainingLockAcquisitionPortOutcome::Success(
                    RemainingLockSuccessObservation::from_repository_adapter(
                        lock_set_id,
                        attempted,
                        vec![root_receipt],
                    ),
                ),
                TestRemainingMode::DuplicateReceipt => {
                    RemainingLockAcquisitionPortOutcome::Success(
                        RemainingLockSuccessObservation::from_repository_adapter(
                            lock_set_id,
                            attempted,
                            vec![root_receipt.clone(), root_receipt, object_receipt],
                        ),
                    )
                }
                TestRemainingMode::CrossLockSetReceipt => {
                    let cross = JournaledRepositoryLock::new(
                        object_target,
                        display("Catalog.B1"),
                        id("99999999-9999-4999-8999-999999999999"),
                        NormalizedUtcInstant::parse("2026-07-22T00:00:01Z").unwrap(),
                    );
                    RemainingLockAcquisitionPortOutcome::Success(
                        RemainingLockSuccessObservation::from_repository_adapter(
                            lock_set_id,
                            attempted,
                            vec![root_receipt, cross],
                        ),
                    )
                }
                TestRemainingMode::DisplayOnlyChange => {
                    let renamed = JournaledRepositoryLock::new(
                        object_target,
                        display("Presentation-only renamed object"),
                        lock_set_id.clone(),
                        NormalizedUtcInstant::parse("2026-07-22T00:00:01Z").unwrap(),
                    );
                    RemainingLockAcquisitionPortOutcome::Success(
                        RemainingLockSuccessObservation::from_repository_adapter(
                            lock_set_id,
                            attempted,
                            vec![root_receipt, renamed],
                        ),
                    )
                }
                TestRemainingMode::MalformedConflict => {
                    RemainingLockAcquisitionPortOutcome::Conflict(Box::new(
                        RemainingLockConflictObservation::from_repository_adapter(
                            RemainingLockConflictObservationInput {
                                lock_set_id,
                                attempted_targets: attempted,
                                acquired: vec![root_receipt.clone()],
                                failed_target: RepositoryTargetIdentity::configuration_root(),
                                released: vec![root_receipt],
                                retained: Vec::new(),
                                compensation_verified: true,
                                conflict_capability_id: CapabilityRowId::parse(
                                    "repository.remaining-malformed-conflict.b1",
                                )
                                .unwrap(),
                                compensation_capability_id: CapabilityRowId::parse(
                                    "repository.remaining-malformed-release.b1",
                                )
                                .unwrap(),
                            },
                        ),
                    ))
                }
                TestRemainingMode::DuplicateMalformedConflictReports => {
                    RemainingLockAcquisitionPortOutcome::Conflict(Box::new(
                        RemainingLockConflictObservation::from_repository_adapter(
                            RemainingLockConflictObservationInput {
                                lock_set_id,
                                attempted_targets: attempted,
                                acquired: vec![root_receipt.clone(), root_receipt.clone()],
                                failed_target: object_target,
                                released: vec![root_receipt.clone(), root_receipt.clone()],
                                retained: vec![root_receipt.clone(), root_receipt],
                                compensation_verified: false,
                                conflict_capability_id: CapabilityRowId::parse(
                                    "repository.remaining-duplicate-conflict.b1",
                                )
                                .unwrap(),
                                compensation_capability_id: CapabilityRowId::parse(
                                    "repository.remaining-duplicate-release.b1",
                                )
                                .unwrap(),
                            },
                        ),
                    ))
                }
                TestRemainingMode::UnknownEffect => RemainingLockAcquisitionPortOutcome::Recovery(
                    Box::new(RepositoryLockRecoveryObservation::from_repository_adapter(
                        Some(lock_set_id),
                        attempted,
                        vec![root_receipt.clone()],
                        Some(object_target),
                        vec![root_receipt],
                        Vec::new(),
                        CapabilityRowId::parse("repository.remaining-unknown.b1").unwrap(),
                    )),
                ),
            }
        }
    }

    fn remaining_port(events: GateEvents, mode: TestRemainingMode) -> TestRemainingPort {
        TestRemainingPort {
            events,
            mode,
            calls: 0,
        }
    }

    fn recovery_retains_original_root(recovery: &RemainingLockRecoveryAuthority) -> bool {
        recovery.evidence.retained.iter().any(|receipt| {
            same_journaled_lock_identity(receipt, &recovery.context.root_guarded.root_lock_receipt)
        })
    }

    fn root_port(events: GateEvents) -> TestRootPort {
        TestRootPort {
            events,
            mode: TestRootPortMode::Held,
            semantic_drift: TestSemanticDrift::None,
            release_verified: true,
            wrong_release_identity: false,
            wrong_receipt_identity: false,
            display_only_change: false,
            journaled_before_reread: true,
            prior_object_attempt: false,
            complete_support_graph_reread: true,
            calls: 0,
        }
    }

    fn resolve_plan_and_gate(
        events: GateEvents,
    ) -> (LockPlanData, CurrentReadySupportGateAuthority) {
        let current = current_gate(events).unwrap();
        let plan = plan_for(&current);
        (plan, current)
    }

    #[test]
    fn lock_rechecks_latest_current_ready_gate_before_first_effect() {
        let events = GateEvents::default();
        let (plan, current) = resolve_plan_and_gate(GateEvents(events.0.clone()));
        let prelock = PreLockCurrentGateAuthority::recheck(plan, current).unwrap();
        let mut root = root_port(GateEvents(events.0.clone()));
        let _guarded = prelock
            .acquire_root_and_reread_current_gate(&mut root)
            .unwrap();

        assert_eq!(events.values()[0], "current-gate-recheck");
        assert_eq!(events.values()[1], "acquire-root");
    }

    #[test]
    fn lock_rejects_stale_gate_without_calling_root_acquire_adapter() {
        let events = GateEvents::default();
        let good = current_gate(GateEvents(events.0.clone())).unwrap();
        let plan = plan_for(&good);
        let stale = CurrentReadySupportGateAuthority::resolve(
            ready_preflight_authority_fixture_test_only(),
            &mut TestCurrentGateResolver {
                events: GateEvents(events.0.clone()),
                stale: true,
            },
        );
        let mut root = root_port(GateEvents(events.0.clone()));
        let rejected = match stale {
            Ok(stale) => {
                let prelock = PreLockCurrentGateAuthority::recheck(plan, stale).unwrap();
                let _ = prelock.acquire_root_and_reread_current_gate(&mut root);
                false
            }
            Err(_) => true,
        };

        assert!(rejected);
        assert_eq!(root.calls, 0);
    }

    #[test]
    fn lock_acquires_support_graph_guard_before_every_object_lock() {
        let events = GateEvents::default();
        let (plan, current) = resolve_plan_and_gate(GateEvents(events.0.clone()));
        let mut root = root_port(GateEvents(events.0.clone()));
        let guarded = PreLockCurrentGateAuthority::recheck(plan, current)
            .unwrap()
            .acquire_root_and_reread_current_gate(&mut root)
            .unwrap();
        let mut remaining =
            remaining_port(GateEvents(events.0.clone()), TestRemainingMode::Success);
        let _locks =
            ValidatedLockSetAuthority::from_root_guarded_acquisition(guarded, &mut remaining)
                .unwrap();

        let values = events.values();
        let journal = values
            .iter()
            .position(|value| value == "journal-root")
            .unwrap();
        let object = values
            .iter()
            .position(|value| value == "acquire-object")
            .unwrap();
        assert!(journal < object);
    }

    #[test]
    fn lock_rereads_gate_and_support_graph_while_exact_root_guard_is_held() {
        let events = GateEvents::default();
        let (plan, current) = resolve_plan_and_gate(GateEvents(events.0.clone()));
        let expected_revision = current.current_state_revision().clone();
        let mut root = root_port(GateEvents(events.0.clone()));
        let guarded = PreLockCurrentGateAuthority::recheck(plan, current)
            .unwrap()
            .acquire_root_and_reread_current_gate(&mut root)
            .unwrap();

        assert_eq!(
            guarded.current_gate().current_state_revision(),
            &expected_revision
        );
        assert!(events
            .values()
            .iter()
            .any(|value| value == "reread-under:55555555-5555-4555-8555-555555555555"));
    }

    #[test]
    fn root_reread_drift_releases_root_and_attempts_no_object_lock() {
        let events = GateEvents::default();
        let (plan, current) = resolve_plan_and_gate(GateEvents(events.0.clone()));
        let mut root = root_port(GateEvents(events.0.clone()));
        root.semantic_drift = TestSemanticDrift::SupportGraph;
        let stop = PreLockCurrentGateAuthority::recheck(plan, current)
            .unwrap()
            .acquire_root_and_reread_current_gate(&mut root)
            .unwrap_err();

        assert!(stop.release_verified());
        assert!(!events
            .values()
            .iter()
            .any(|value| value == "acquire-object"));
        assert!(events.values().iter().any(|value| value == "release-root"));
        assert!(stop.into_verified_stale_stop().is_ok());
    }

    #[test]
    fn root_reread_drift_with_unverified_release_cannot_mint_lock_set() {
        let events = GateEvents::default();
        let (plan, current) = resolve_plan_and_gate(GateEvents(events.0.clone()));
        let mut root = root_port(GateEvents(events.0.clone()));
        root.semantic_drift = TestSemanticDrift::SupportGraph;
        root.release_verified = false;
        let stop = PreLockCurrentGateAuthority::recheck(plan, current)
            .unwrap()
            .acquire_root_and_reread_current_gate(&mut root)
            .unwrap_err();

        assert!(!stop.release_verified());
        assert!(stop.is_recovery_only());
        assert!(stop.into_verified_stale_stop().is_err());
    }

    #[test]
    fn root_reread_drift_with_wrong_verified_release_receipt_is_recovery_only() {
        let events = GateEvents::default();
        let (plan, current) = resolve_plan_and_gate(GateEvents(events.0.clone()));
        let mut root = root_port(events);
        root.semantic_drift = TestSemanticDrift::SupportGraph;
        root.wrong_release_identity = true;
        let blocked = PreLockCurrentGateAuthority::recheck(plan, current)
            .unwrap()
            .acquire_root_and_reread_current_gate(&mut root)
            .unwrap_err();

        assert!(blocked.is_recovery_only());
        assert!(blocked.into_verified_stale_stop().is_err());
    }

    #[test]
    fn root_receipt_presentation_change_does_not_change_lock_identity() {
        let events = GateEvents::default();
        let (plan, current) = resolve_plan_and_gate(GateEvents(events.0.clone()));
        let mut root = root_port(events);
        root.display_only_change = true;

        assert!(PreLockCurrentGateAuthority::recheck(plan, current)
            .unwrap()
            .acquire_root_and_reread_current_gate(&mut root)
            .is_ok());
    }

    #[test]
    fn wrong_root_receipt_identity_is_recovery_only_even_after_exact_reported_release() {
        let events = GateEvents::default();
        let (plan, current) = resolve_plan_and_gate(GateEvents(events.0.clone()));
        let mut root = root_port(events);
        root.wrong_receipt_identity = true;
        root.semantic_drift = TestSemanticDrift::SupportGraph;
        let blocked = PreLockCurrentGateAuthority::recheck(plan, current)
            .unwrap()
            .acquire_root_and_reread_current_gate(&mut root)
            .unwrap_err();

        assert!(blocked.is_recovery_only());
        assert!(blocked.into_verified_stale_stop().is_err());
    }

    #[test]
    fn unjournaled_root_receipt_is_recovery_only_not_a_stale_stop() {
        let events = GateEvents::default();
        let (plan, current) = resolve_plan_and_gate(GateEvents(events.0.clone()));
        let mut root = root_port(events);
        root.journaled_before_reread = false;
        root.semantic_drift = TestSemanticDrift::SupportGraph;
        let blocked = PreLockCurrentGateAuthority::recheck(plan, current)
            .unwrap()
            .acquire_root_and_reread_current_gate(&mut root)
            .unwrap_err();

        assert!(blocked.is_recovery_only());
        assert!(blocked.into_verified_stale_stop().is_err());
    }

    #[test]
    fn prior_object_attempt_preserves_exact_journal_receipt_and_root_for_recovery() {
        let events = GateEvents::default();
        let (plan, current) = resolve_plan_and_gate(GateEvents(events.0.clone()));
        let mut root = root_port(events);
        root.prior_object_attempt = true;
        root.semantic_drift = TestSemanticDrift::SupportGraph;
        let blocked = PreLockCurrentGateAuthority::recheck(plan, current)
            .unwrap()
            .acquire_root_and_reread_current_gate(&mut root)
            .unwrap_err();

        let recovery = blocked.into_recovery().unwrap();
        let expected_root = JournaledRepositoryLock::new(
            RepositoryTargetIdentity::configuration_root(),
            display("Configuration root"),
            id("55555555-5555-4555-8555-555555555555"),
            NormalizedUtcInstant::parse("2026-07-22T00:00:00Z").unwrap(),
        );
        let expected_prior_object = JournaledRepositoryLock::new(
            RepositoryTargetIdentity::DevelopmentObject(ObjectTargetIdentity::new(object())),
            display("Catalog.B1"),
            id("55555555-5555-4555-8555-555555555555"),
            NormalizedUtcInstant::parse("2026-07-22T00:00:01Z").unwrap(),
        );
        assert_eq!(recovery.attempted_target_count(), 2);
        assert_eq!(recovery.acquired_lock_count(), 2);
        assert_eq!(recovery.retained_lock_count(), 2);
        assert!(recovery
            .evidence
            .acquired
            .iter()
            .any(|receipt| same_journaled_lock_identity(receipt, &expected_prior_object)));
        assert!(recovery
            .evidence
            .retained
            .iter()
            .any(|receipt| same_journaled_lock_identity(receipt, &expected_root)));
        assert!(recovery
            .evidence
            .retained
            .iter()
            .any(|receipt| same_journaled_lock_identity(receipt, &expected_prior_object)));
    }

    #[test]
    fn incomplete_under_root_reread_is_recovery_only_not_a_stale_stop() {
        let events = GateEvents::default();
        let (plan, current) = resolve_plan_and_gate(GateEvents(events.0.clone()));
        let mut root = root_port(events);
        root.complete_support_graph_reread = false;
        root.semantic_drift = TestSemanticDrift::SupportGraph;
        let blocked = PreLockCurrentGateAuthority::recheck(plan, current)
            .unwrap()
            .acquire_root_and_reread_current_gate(&mut root)
            .unwrap_err();

        assert!(blocked.is_recovery_only());
        assert!(blocked.into_verified_stale_stop().is_err());
    }

    #[test]
    fn every_material_semantic_reread_drift_can_only_mint_verified_stale_stop() {
        for drift in [
            TestSemanticDrift::GateId,
            TestSemanticDrift::GateDigest,
            TestSemanticDrift::History,
            TestSemanticDrift::SupportGraph,
            TestSemanticDrift::RelevantBaseline,
            TestSemanticDrift::Fingerprint,
            TestSemanticDrift::StateRevision,
        ] {
            let events = GateEvents::default();
            let (plan, current) = resolve_plan_and_gate(GateEvents(events.0.clone()));
            let mut root = root_port(events);
            root.semantic_drift = drift;
            let blocked = PreLockCurrentGateAuthority::recheck(plan, current)
                .unwrap()
                .acquire_root_and_reread_current_gate(&mut root)
                .unwrap_err();

            assert!(blocked.into_verified_stale_stop().is_ok());
        }
    }

    #[test]
    fn first_root_conflict_with_no_acquired_lock_is_a_typed_conflict_stop() {
        let events = GateEvents::default();
        let (plan, current) = resolve_plan_and_gate(GateEvents(events.0.clone()));
        let mut root = root_port(events);
        root.mode = TestRootPortMode::FirstRootConflict;
        let blocked = PreLockCurrentGateAuthority::recheck(plan, current)
            .unwrap()
            .acquire_root_and_reread_current_gate(&mut root)
            .unwrap_err();

        let conflict = blocked.into_root_conflict_stop().unwrap();
        assert_eq!(
            conflict.failed_target(),
            &RepositoryTargetIdentity::configuration_root()
        );
        assert_eq!(conflict.acquired_lock_count(), 0);
    }

    #[test]
    fn malformed_first_root_conflict_preserves_possible_retained_lock_for_recovery() {
        let events = GateEvents::default();
        let (plan, current) = resolve_plan_and_gate(GateEvents(events.0.clone()));
        let mut root = root_port(events);
        root.mode = TestRootPortMode::MalformedFirstRootConflict;
        let blocked = PreLockCurrentGateAuthority::recheck(plan, current)
            .unwrap()
            .acquire_root_and_reread_current_gate(&mut root)
            .unwrap_err();

        let recovery = blocked.into_recovery().unwrap();
        assert_eq!(recovery.attempted_target_count(), 1);
        assert_eq!(recovery.acquired_lock_count(), 1);
        assert_eq!(recovery.retained_lock_count(), 1);
    }

    #[test]
    fn unknown_first_root_failure_preserves_attempt_evidence_for_recovery() {
        let events = GateEvents::default();
        let (plan, current) = resolve_plan_and_gate(GateEvents(events.0.clone()));
        let mut root = root_port(events);
        root.mode = TestRootPortMode::UnknownFailure;
        let blocked = PreLockCurrentGateAuthority::recheck(plan, current)
            .unwrap()
            .acquire_root_and_reread_current_gate(&mut root)
            .unwrap_err();

        let recovery = blocked.into_recovery().unwrap();
        assert_eq!(recovery.attempted_target_count(), 1);
        assert_eq!(recovery.acquired_lock_count(), 0);
    }

    #[test]
    fn lock_rejects_cross_plan_current_gate_authority() {
        let events = GateEvents::default();
        let (mut plan, current) = resolve_plan_and_gate(events);
        plan.gate_session_lineage.comparison_id = id("99999999-9999-4999-8999-999999999999");

        assert!(PreLockCurrentGateAuthority::recheck(plan, current).is_err());
    }

    #[test]
    fn root_guard_authority_is_consumed_once_and_is_statically_non_clone() {
        let events = GateEvents::default();
        let (plan, current) = resolve_plan_and_gate(GateEvents(events.0.clone()));
        let mut root = root_port(GateEvents(events.0.clone()));
        let guarded = PreLockCurrentGateAuthority::recheck(plan, current)
            .unwrap()
            .acquire_root_and_reread_current_gate(&mut root)
            .unwrap();
        let mut remaining = remaining_port(events, TestRemainingMode::Success);
        let locks =
            ValidatedLockSetAuthority::from_root_guarded_acquisition(guarded, &mut remaining)
                .unwrap();
        let projection = locks.into_original_merge_projection();

        assert_eq!(remaining.calls, 1);
        assert!(projection.current_gate_authority().is_some());
    }

    #[test]
    fn remaining_partial_conflict_preserves_exact_prefix_and_verified_reverse_compensation() {
        let events = GateEvents::default();
        let (plan, current) = resolve_plan_and_gate(GateEvents(events.0.clone()));
        let mut root = root_port(GateEvents(events.0.clone()));
        let guarded = PreLockCurrentGateAuthority::recheck(plan, current)
            .unwrap()
            .acquire_root_and_reread_current_gate(&mut root)
            .unwrap();
        let mut remaining = remaining_port(events, TestRemainingMode::PartialConflict);
        let blocked =
            ValidatedLockSetAuthority::from_root_guarded_acquisition(guarded, &mut remaining)
                .unwrap_err();

        let conflict = blocked.into_verified_conflict_stop().unwrap();
        assert_eq!(
            conflict.failed_target(),
            &RepositoryTargetIdentity::DevelopmentObject(ObjectTargetIdentity::new(object()))
        );
        assert_eq!(conflict.acquired_lock_count(), 1);
        assert_eq!(conflict.released_lock_count(), 1);
        assert_eq!(conflict.retained_lock_count(), 0);
    }

    #[test]
    fn remaining_multi_lock_prefix_requires_reverse_compensation_order() {
        for reverse_compensation in [true, false] {
            let events = GateEvents::default();
            let current = current_gate(GateEvents(events.0.clone())).unwrap();
            let plan = plan_for_objects(
                &current,
                &[(object(), "Catalog.B1"), (object_b(), "Catalog.B2")],
            );
            let mut root = root_port(events);
            let guarded = PreLockCurrentGateAuthority::recheck(plan, current)
                .unwrap()
                .acquire_root_and_reread_current_gate(&mut root)
                .unwrap();
            let mut remaining = MultiPrefixConflictPort {
                reverse_compensation,
            };
            let blocked =
                ValidatedLockSetAuthority::from_root_guarded_acquisition(guarded, &mut remaining)
                    .unwrap_err();

            if reverse_compensation {
                let conflict = blocked.into_verified_conflict_stop().unwrap();
                assert_eq!(conflict.acquired_lock_count(), 2);
                assert_eq!(conflict.released_lock_count(), 2);
                assert_eq!(conflict.retained_lock_count(), 0);
            } else {
                let recovery = blocked.into_recovery().unwrap();
                assert_eq!(recovery.retained_lock_count(), 2);
            }
        }
    }

    #[test]
    fn remaining_failed_compensation_preserves_retained_locks_for_recovery_only() {
        let events = GateEvents::default();
        let (plan, current) = resolve_plan_and_gate(GateEvents(events.0.clone()));
        let mut root = root_port(GateEvents(events.0.clone()));
        let guarded = PreLockCurrentGateAuthority::recheck(plan, current)
            .unwrap()
            .acquire_root_and_reread_current_gate(&mut root)
            .unwrap();
        let mut remaining = remaining_port(events, TestRemainingMode::FailedCompensation);
        let blocked =
            ValidatedLockSetAuthority::from_root_guarded_acquisition(guarded, &mut remaining)
                .unwrap_err();

        let recovery = blocked.into_recovery().unwrap();
        assert_eq!(recovery.acquired_lock_count(), 1);
        assert_eq!(recovery.released_lock_count(), 0);
        assert_eq!(recovery.retained_lock_count(), 1);
    }

    #[test]
    fn remaining_malformed_conflict_and_unknown_effect_are_recovery_only() {
        for mode in [
            TestRemainingMode::MalformedConflict,
            TestRemainingMode::UnknownEffect,
        ] {
            let events = GateEvents::default();
            let (plan, current) = resolve_plan_and_gate(GateEvents(events.0.clone()));
            let mut root = root_port(GateEvents(events.0.clone()));
            let guarded = PreLockCurrentGateAuthority::recheck(plan, current)
                .unwrap()
                .acquire_root_and_reread_current_gate(&mut root)
                .unwrap();
            let mut remaining = remaining_port(events, mode);
            let blocked =
                ValidatedLockSetAuthority::from_root_guarded_acquisition(guarded, &mut remaining)
                    .unwrap_err();

            let recovery = blocked.into_recovery().unwrap();
            assert_eq!(recovery.acquired_lock_count(), 1);
            assert_eq!(recovery.released_lock_count(), 1);
            assert_eq!(recovery.retained_lock_count(), 1);
        }
    }

    #[test]
    fn malformed_remaining_duplicate_reports_dedupe_possible_retention() {
        let events = GateEvents::default();
        let (plan, current) = resolve_plan_and_gate(GateEvents(events.0.clone()));
        let mut root = root_port(GateEvents(events.0.clone()));
        let guarded = PreLockCurrentGateAuthority::recheck(plan, current)
            .unwrap()
            .acquire_root_and_reread_current_gate(&mut root)
            .unwrap();
        let mut remaining =
            remaining_port(events, TestRemainingMode::DuplicateMalformedConflictReports);
        let blocked =
            ValidatedLockSetAuthority::from_root_guarded_acquisition(guarded, &mut remaining)
                .unwrap_err();

        let recovery = blocked.into_recovery().unwrap();
        assert_eq!(recovery.acquired_lock_count(), 2);
        assert_eq!(recovery.released_lock_count(), 2);
        assert_eq!(recovery.retained_lock_count(), 1);
    }

    #[test]
    fn remaining_success_rejects_reordered_missing_and_duplicate_journal_receipts() {
        for (mode, acquired_count, retained_count) in [
            (TestRemainingMode::ReorderedReceipts, 2, 2),
            (TestRemainingMode::MissingReceipt, 1, 1),
            (TestRemainingMode::DuplicateReceipt, 3, 2),
        ] {
            let events = GateEvents::default();
            let (plan, current) = resolve_plan_and_gate(GateEvents(events.0.clone()));
            let mut root = root_port(GateEvents(events.0.clone()));
            let guarded = PreLockCurrentGateAuthority::recheck(plan, current)
                .unwrap()
                .acquire_root_and_reread_current_gate(&mut root)
                .unwrap();
            let mut remaining = remaining_port(events, mode);
            let blocked =
                ValidatedLockSetAuthority::from_root_guarded_acquisition(guarded, &mut remaining)
                    .unwrap_err();

            let recovery = blocked.into_recovery().unwrap();
            assert_eq!(recovery.acquired_lock_count(), acquired_count);
            assert_eq!(recovery.retained_lock_count(), retained_count);
            assert!(recovery_retains_original_root(&recovery));
        }
    }

    #[test]
    fn remaining_success_rejects_cross_lock_set_journal_receipt() {
        let events = GateEvents::default();
        let (plan, current) = resolve_plan_and_gate(GateEvents(events.0.clone()));
        let mut root = root_port(GateEvents(events.0.clone()));
        let guarded = PreLockCurrentGateAuthority::recheck(plan, current)
            .unwrap()
            .acquire_root_and_reread_current_gate(&mut root)
            .unwrap();
        let mut remaining = remaining_port(events, TestRemainingMode::CrossLockSetReceipt);
        let blocked =
            ValidatedLockSetAuthority::from_root_guarded_acquisition(guarded, &mut remaining)
                .unwrap_err();

        let recovery = blocked.into_recovery().unwrap();
        assert_eq!(recovery.acquired_lock_count(), 2);
        assert_eq!(recovery.retained_lock_count(), 2);
        assert!(recovery_retains_original_root(&recovery));
    }

    #[test]
    fn remaining_receipt_presentation_change_does_not_change_lock_identity() {
        let events = GateEvents::default();
        let (plan, current) = resolve_plan_and_gate(GateEvents(events.0.clone()));
        let mut root = root_port(GateEvents(events.0.clone()));
        let guarded = PreLockCurrentGateAuthority::recheck(plan, current)
            .unwrap()
            .acquire_root_and_reread_current_gate(&mut root)
            .unwrap();
        let mut remaining = remaining_port(events, TestRemainingMode::DisplayOnlyChange);

        assert!(
            ValidatedLockSetAuthority::from_root_guarded_acquisition(guarded, &mut remaining,)
                .is_ok()
        );
    }
}

#[cfg(test)]
mod merge_consumer_tests {
    use super::*;
    use crate::domain::branched_development::contracts::repository::empty_commit_history_evidence_fixture_test_only;
    use crate::domain::branched_development::contracts::results::merge::{
        validated_main_integration_verification_fixture_test_only,
        validated_main_sandbox_verification_fixture_test_only,
    };

    #[derive(Serialize)]
    #[serde(transparent)]
    struct PreviewWithoutCommitDigest(JsonValue);

    impl contract_digest_record_sealed::Sealed for PreviewWithoutCommitDigest {}
    impl ContractDigestRecord for PreviewWithoutCommitDigest {}

    fn digest(character: char) -> Sha256Digest {
        Sha256Digest::parse(&character.to_string().repeat(64)).unwrap()
    }

    fn id(value: &str) -> UnicaId {
        UnicaId::parse(value).unwrap()
    }

    fn template_plan() -> LockPlanData {
        validated_commit_object_authority_fixture_test_only(
            RepositoryVersion::parse("201").unwrap(),
            CapabilityRowId::parse("repository.atomic-commit.consumer-template").unwrap(),
        )
        .approved_preview
        .0
        .plan
    }

    fn root_observation_input() -> RepositoryLockPlanObservationInput {
        let template = template_plan();
        RepositoryLockPlanObservationInput::from_planner_adapter(
            RepositoryLockPlanObservedIds::new(
                id("91000000-0000-4000-8000-000000000001"),
                id("91000000-0000-4000-8000-000000000002"),
            ),
            vec![RepositoryIntegrationTopologyObservation::root_modify(
                RepositoryTargetDisplay::parse("Configuration root").unwrap(),
            )],
            template.lock_entries,
            RepositoryLockPlanObservedEvidence::from_planner_adapter(
                template.relevant_anchors,
                template.compatibility_mode,
                template.reference_closure_digest,
                template.prevalidation_diagnostics_digest,
                CapabilityRowId::parse("repository.lock-plan.consumer").unwrap(),
            ),
        )
    }

    struct TestRepositoryLockPlanObservationPort {
        input: Option<RepositoryLockPlanObservationInput>,
        observed_verification_id: Option<UnicaId>,
    }

    impl RepositoryLockPlanObservationPort for TestRepositoryLockPlanObservationPort {
        fn observe_lock_plan(
            &mut self,
            request: RepositoryLockPlanObservationRequest<'_>,
        ) -> Result<RepositoryLockPlanObservationLease, RepositoryResultContractError> {
            self.observed_verification_id = Some(request.verification_id().clone());
            RepositoryLockPlanObservationLease::complete_from_planner_adapter(
                &request,
                self.input
                    .take()
                    .expect("test planner is called exactly once"),
            )
        }
    }

    #[test]
    fn lock_plan_observation_port_binds_verified_scope_and_derives_root_entry() {
        let verified = validated_main_sandbox_verification_fixture_test_only();
        let expected_verification_id = verified.verification_id().clone();
        let mut port = TestRepositoryLockPlanObservationPort {
            input: Some(root_observation_input()),
            observed_verification_id: None,
        };

        let planner = AtomicRepositoryLockPlanCapabilityAuthority::from_observation_port(
            &verified, &mut port,
        )
        .unwrap();
        let plan = LockPlanData::from_authority(
            LockPlanAuthority::from_verified_main_sandbox(verified, planner).unwrap(),
        )
        .unwrap();

        assert_eq!(
            port.observed_verification_id,
            Some(expected_verification_id)
        );
        assert_eq!(
            serde_json::to_value(plan.integration_entries()).unwrap(),
            serde_json::json!([{
                "target": {"targetKind": "configurationRoot"},
                "objectDisplay": "Configuration root",
                "action": "modify",
                "reasons": ["canonicalDelta"],
                "requiredLockTargets": [{"targetKind": "configurationRoot"}]
            }])
        );
    }

    struct ForeignRepositoryLockPlanLeasePort {
        lease: Option<RepositoryLockPlanObservationLease>,
    }

    impl RepositoryLockPlanObservationPort for ForeignRepositoryLockPlanLeasePort {
        fn observe_lock_plan(
            &mut self,
            _request: RepositoryLockPlanObservationRequest<'_>,
        ) -> Result<RepositoryLockPlanObservationLease, RepositoryResultContractError> {
            Ok(self
                .lease
                .take()
                .expect("foreign planner lease is returned exactly once"))
        }
    }

    #[test]
    fn lock_plan_observation_rejects_equal_scalar_cross_invocation_lease() {
        let verified_a = validated_main_sandbox_verification_fixture_test_only();
        let verified_b = validated_main_sandbox_verification_fixture_test_only();
        assert_eq!(verified_a.verification_id(), verified_b.verification_id());
        assert_eq!(
            verified_a.verification_digest(),
            verified_b.verification_digest()
        );

        let foreign_invocation = RepositoryLockPlanObservationInvocationCapability::mint();
        let foreign_request = RepositoryLockPlanObservationRequest {
            verified_scope: &verified_a,
            invocation: &foreign_invocation,
        };
        let foreign_lease = RepositoryLockPlanObservationLease::complete_from_planner_adapter(
            &foreign_request,
            root_observation_input(),
        )
        .unwrap();
        let mut port = ForeignRepositoryLockPlanLeasePort {
            lease: Some(foreign_lease),
        };

        assert!(
            AtomicRepositoryLockPlanCapabilityAuthority::from_observation_port(
                &verified_b,
                &mut port,
            )
            .is_err()
        );
    }

    fn planner_for(
        verified: &ValidatedMainSandboxVerificationAuthority,
    ) -> AtomicRepositoryLockPlanCapabilityAuthority {
        let mut port = TestRepositoryLockPlanObservationPort {
            input: Some(root_observation_input()),
            observed_verification_id: None,
        };
        AtomicRepositoryLockPlanCapabilityAuthority::from_observation_port(verified, &mut port)
            .unwrap()
    }

    fn validated_comment_policy() -> ValidatedCommitCommentPolicyAuthority {
        let task_id = TaskId::parse("PR-173").unwrap();
        let frozen = FrozenCommitCommentPolicyAuthority::from_task_start_renderer_adapter(
            Comment::parse("{taskId}: {summary}").unwrap(),
            task_id.clone(),
            TaskSummary::parse("Branched development").unwrap(),
            ProjectId::parse("92000000-0000-4000-8000-000000000001").unwrap(),
            Comment::parse("PR-173: Branched development").unwrap(),
            CapabilityRowId::parse("profile.commit-comment.renderer").unwrap(),
        )
        .unwrap();
        ValidatedCommitCommentPolicyAuthority::revalidate(
            frozen,
            Comment::parse("{taskId}: {summary}").unwrap(),
            task_id,
            TaskSummary::parse("Branched development").unwrap(),
            ProjectId::parse("92000000-0000-4000-8000-000000000001").unwrap(),
            Comment::parse("PR-173: Branched development").unwrap(),
            CapabilityRowId::parse("profile.commit-comment.renderer").unwrap(),
        )
        .unwrap()
    }

    fn commit_guard(
        verified: &ValidatedMainIntegrationVerificationAuthority,
        observed_fingerprint: Sha256Digest,
        cursor: RepositoryHistoryCursor,
    ) -> PostMergeCommitGuardFixtureAuthority {
        commit_guard_with_closure(
            observed_fingerprint,
            cursor,
            verified
                .lock_projection()
                .plan()
                .reference_closure_digest
                .clone(),
        )
    }

    fn commit_guard_with_closure(
        observed_fingerprint: Sha256Digest,
        cursor: RepositoryHistoryCursor,
        reference_closure_digest: Sha256Digest,
    ) -> PostMergeCommitGuardFixtureAuthority {
        let (_, history_guard) = empty_commit_history_evidence_fixture_test_only(
            cursor,
            digest('d'),
            reference_closure_digest,
            CapabilityRowId::parse("repository.atomic-commit.consumer").unwrap(),
        )
        .unwrap();
        PostMergeCommitGuardFixtureAuthority::from_capability_adapter(
            history_guard,
            observed_fingerprint,
            CapabilityRowId::parse("repository.original-fingerprint.consumer").unwrap(),
        )
    }

    #[test]
    fn lock_plan_consumes_exact_main_sandbox_and_atomic_planner_lineage() {
        let verified = validated_main_sandbox_verification_fixture_test_only();
        let expected_session = verified.merge_session_id().clone();
        let expected_verification = verified.verification_digest().clone();
        let planner = planner_for(&verified);
        let plan = LockPlanData::from_authority(
            LockPlanAuthority::from_verified_main_sandbox(verified, planner).unwrap(),
        )
        .unwrap();
        assert_eq!(plan.merge_session_id, expected_session);
        assert_eq!(plan.verification_digest, expected_verification);

        let verified = validated_main_sandbox_verification_fixture_test_only();
        let wrong_comparison = id("91000000-0000-4000-8000-000000000099");
        let mut planner = planner_for(&verified);
        planner.comparison_id = wrong_comparison;
        assert!(LockPlanAuthority::from_verified_main_sandbox(verified, planner).is_err());

        let verified = validated_main_sandbox_verification_fixture_test_only();
        let mut planner = planner_for(&verified);
        planner.result_digest = digest('f');
        assert!(LockPlanAuthority::from_verified_main_sandbox(verified, planner).is_err());

        let verified = validated_main_sandbox_verification_fixture_test_only();
        let mut planner = planner_for(&verified);
        planner.verification_id = id("91000000-0000-4000-8000-000000000098");
        planner.verification_digest = digest('e');
        assert!(LockPlanAuthority::from_verified_main_sandbox(verified, planner).is_err());

        let verified = validated_main_sandbox_verification_fixture_test_only();
        let mut planner = planner_for(&verified);
        planner.lock_entries = serde_json::from_value(serde_json::json!([{
            "targetKind": "configurationRoot",
            "objectDisplay": "Configuration root",
            "reasons": ["updateTarget"]
        }]))
        .unwrap();
        assert!(LockPlanAuthority::from_verified_main_sandbox(verified, planner).is_err());
    }

    #[test]
    fn commit_preview_consumes_post_merge_verification_plan_guard_and_comment_policy() {
        let verified = validated_main_integration_verification_fixture_test_only();
        let expected_verification = verified.verification_digest().clone();
        let stale_plan_verification = verified
            .lock_projection()
            .plan()
            .verification_digest
            .clone();
        let cursor = verified.repository_history_cursor().clone();
        let fingerprint = verified.result_fingerprint().clone();
        let guard = commit_guard(&verified, fingerprint, cursor);
        let preview = CommitPreviewAuthority::from_verified_main_integration(
            verified,
            guard,
            validated_comment_policy(),
        )
        .unwrap();
        assert_eq!(preview.record.verification_digest, expected_verification);
        assert_ne!(preview.record.verification_digest, stale_plan_verification);
        assert_eq!(
            preview.record.guard_locks,
            derive_commit_guard_locks(&preview.plan).unwrap()
        );
        let data = preview.data();
        let mut without_commit_digest = serde_json::to_value(&data).unwrap();
        without_commit_digest
            .as_object_mut()
            .unwrap()
            .remove("commitDigest");
        assert_eq!(
            canonical_contract_digest(&PreviewWithoutCommitDigest(without_commit_digest), None,)
                .unwrap(),
            *data.commit_digest()
        );

        let verified = validated_main_integration_verification_fixture_test_only();
        let cursor = verified.repository_history_cursor().clone();
        let guard = commit_guard(&verified, digest('f'), cursor);
        assert!(CommitPreviewAuthority::from_verified_main_integration(
            verified,
            guard,
            validated_comment_policy(),
        )
        .is_err());

        let verified = validated_main_integration_verification_fixture_test_only();
        let wrong_cursor =
            RepositoryHistoryCursor::new(RepositoryVersion::parse("999").unwrap(), digest('e'));
        let fingerprint = verified.result_fingerprint().clone();
        let guard = commit_guard(&verified, fingerprint, wrong_cursor);
        assert!(CommitPreviewAuthority::from_verified_main_integration(
            verified,
            guard,
            validated_comment_policy(),
        )
        .is_err());

        let verified = validated_main_integration_verification_fixture_test_only();
        let cursor = verified.repository_history_cursor().clone();
        let fingerprint = verified.result_fingerprint().clone();
        let guard = commit_guard_with_closure(fingerprint, cursor, digest('e'));
        assert!(CommitPreviewAuthority::from_verified_main_integration(
            verified,
            guard,
            validated_comment_policy(),
        )
        .is_err());
    }

    #[test]
    fn commit_preview_rejects_broader_guard_set_and_cross_plan_gate_history() {
        let fixture = validated_commit_object_authority_fixture_test_only(
            RepositoryVersion::parse("301").unwrap(),
            CapabilityRowId::parse("repository.atomic-commit.guard-negative").unwrap(),
        );
        let preview = &fixture.approved_preview.0;
        let wrong_guard_locks =
            CanonicalRepositoryTargets::new(vec![RepositoryTargetIdentity::configuration_root()])
                .unwrap();
        assert!(
            CommitPreviewAuthority::test_only(CommitPreviewAuthorityTestParts {
                plan: preview.plan.clone(),
                guard_locks: wrong_guard_locks,
                comment: preview.record.comment.clone(),
                verification_digest: preview.record.verification_digest.clone(),
                lock_set_digest: preview.record.lock_set_digest.clone(),
                merge_receipt_id: preview.record.merge_receipt_id.clone(),
                merge_receipt_cursor: preview
                    .record
                    .history_guard_evidence
                    .merge_receipt_cursor()
                    .clone(),
                consumed_support_gate_history_evidence: preview
                    .plan
                    .support_gate_history_evidence
                    .clone(),
                authorized_post_merge_fingerprint: preview
                    .record
                    .authorized_post_merge_fingerprint
                    .clone(),
                observed_original_fingerprint: preview.record.observed_original_fingerprint.clone(),
                history_guard_evidence: preview.record.history_guard_evidence.clone(),
            })
            .is_err()
        );

        let other = validated_commit_object_authority_fixture_test_only(
            RepositoryVersion::parse("302").unwrap(),
            CapabilityRowId::parse("repository.atomic-commit.cross-plan").unwrap(),
        );
        let other_preview = &other.approved_preview.0;
        assert!(
            CommitPreviewAuthority::test_only(CommitPreviewAuthorityTestParts {
                plan: preview.plan.clone(),
                guard_locks: preview.record.guard_locks.clone(),
                comment: preview.record.comment.clone(),
                verification_digest: preview.record.verification_digest.clone(),
                lock_set_digest: preview.record.lock_set_digest.clone(),
                merge_receipt_id: preview.record.merge_receipt_id.clone(),
                merge_receipt_cursor: other_preview
                    .record
                    .history_guard_evidence
                    .merge_receipt_cursor()
                    .clone(),
                consumed_support_gate_history_evidence: other_preview
                    .plan
                    .support_gate_history_evidence
                    .clone(),
                authorized_post_merge_fingerprint: preview
                    .record
                    .authorized_post_merge_fingerprint
                    .clone(),
                observed_original_fingerprint: preview.record.observed_original_fingerprint.clone(),
                history_guard_evidence: other_preview.record.history_guard_evidence.clone(),
            })
            .is_err()
        );
    }

    #[test]
    fn commit_comment_policy_rejects_metadata_or_task_binding_drift() {
        assert!(
            FrozenCommitCommentPolicyAuthority::from_task_start_renderer_adapter(
                Comment::parse("{summary}").unwrap(),
                TaskId::parse("PR-173").unwrap(),
                TaskSummary::parse("Branched development").unwrap(),
                ProjectId::parse("92000000-0000-4000-8000-000000000001").unwrap(),
                Comment::parse("Branched development").unwrap(),
                CapabilityRowId::parse("profile.commit-comment.renderer").unwrap(),
            )
            .is_err()
        );

        let frozen = FrozenCommitCommentPolicyAuthority::from_task_start_renderer_adapter(
            Comment::parse("{taskId}: {summary}").unwrap(),
            TaskId::parse("PR-173").unwrap(),
            TaskSummary::parse("Branched development").unwrap(),
            ProjectId::parse("92000000-0000-4000-8000-000000000001").unwrap(),
            Comment::parse("PR-173: Branched development").unwrap(),
            CapabilityRowId::parse("profile.commit-comment.renderer").unwrap(),
        )
        .unwrap();
        assert!(ValidatedCommitCommentPolicyAuthority::revalidate(
            frozen,
            Comment::parse("{taskId}: {summary}").unwrap(),
            TaskId::parse("PR-173").unwrap(),
            TaskSummary::parse("Changed summary").unwrap(),
            ProjectId::parse("92000000-0000-4000-8000-000000000001").unwrap(),
            Comment::parse("PR-173: Changed summary").unwrap(),
            CapabilityRowId::parse("profile.commit-comment.renderer").unwrap(),
        )
        .is_err());

        let frozen = FrozenCommitCommentPolicyAuthority::from_task_start_renderer_adapter(
            Comment::parse("{taskId}: {summary}").unwrap(),
            TaskId::parse("PR-173").unwrap(),
            TaskSummary::parse("Branched development").unwrap(),
            ProjectId::parse("92000000-0000-4000-8000-000000000001").unwrap(),
            Comment::parse("PR-173: Branched development").unwrap(),
            CapabilityRowId::parse("profile.commit-comment.renderer").unwrap(),
        )
        .unwrap();
        assert!(ValidatedCommitCommentPolicyAuthority::revalidate(
            frozen,
            Comment::parse("{taskId}: {summary}").unwrap(),
            TaskId::parse("PR-173").unwrap(),
            TaskSummary::parse("Branched development").unwrap(),
            ProjectId::parse("92000000-0000-4000-8000-000000000001").unwrap(),
            Comment::parse("PR-173: Branched development").unwrap(),
            CapabilityRowId::parse("profile.commit-comment.another-renderer").unwrap(),
        )
        .is_err());
    }
}

/// Atomic completion observation after the repository command and the complete
/// post-command history scan. The nested partition must already have been
/// produced by the taskCommit-specific resolver using the same commit-object
/// authority later consumed by `CommitData::from_authority`.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct CommitCompletionObservationAuthority {
    commit_receipt_id: UnicaId,
    before_repository_cursor: RepositoryHistoryCursor,
    after_repository_cursor: RepositoryHistoryCursor,
    post_commit_history_partition: ValidatedTaskCommitHistoryPartition,
    released_objects: CanonicalRepositoryTargets,
    released_guard_locks: CanonicalRepositoryTargets,
    repository_anchor: RepositoryAnchor,
}

impl CommitCompletionObservationAuthority {
    #[allow(clippy::too_many_arguments)]
    #[cfg(test)]
    pub(crate) fn from_atomic_adapter(
        commit_receipt_id: UnicaId,
        before_repository_cursor: RepositoryHistoryCursor,
        after_repository_cursor: RepositoryHistoryCursor,
        post_commit_history_partition: ValidatedTaskCommitHistoryPartition,
        released_objects: CanonicalRepositoryTargets,
        released_guard_locks: CanonicalRepositoryTargets,
        repository_anchor: RepositoryAnchor,
    ) -> Result<Self, RepositoryResultContractError> {
        if post_commit_history_partition.partition().start_cursor() != &before_repository_cursor
            || post_commit_history_partition
                .partition()
                .through_inclusive()
                != &after_repository_cursor
        {
            return Err(RepositoryResultContractError(
                "post-commit history partition endpoints disagree with the completion cursors",
            ));
        }
        if repository_anchor.history_cursor() != &after_repository_cursor {
            return Err(RepositoryResultContractError(
                "post-commit repository anchor does not end at the observed cursor",
            ));
        }
        if post_commit_history_partition
            .partition()
            .classifications()
            .filter(|classification| {
                *classification
                    == crate::domain::branched_development::contracts::repository::RepositoryHistoryPartitionClassification::TaskCommit
            })
            .count()
            != 1
        {
            return Err(RepositoryResultContractError(
                "completed commit history must contain exactly one taskCommit entry",
            ));
        }
        if released_objects.as_slice().iter().any(|target| {
            released_guard_locks
                .as_slice()
                .binary_search(target)
                .is_ok()
        }) {
            return Err(RepositoryResultContractError(
                "released task objects and guard-only locks must be disjoint",
            ));
        }
        Ok(Self {
            commit_receipt_id,
            before_repository_cursor,
            after_repository_cursor,
            post_commit_history_partition,
            released_objects,
            released_guard_locks,
            repository_anchor,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CommitData {
    commit_receipt_id: UnicaId,
    repository_version: RepositoryVersion,
    before_repository_cursor: RepositoryHistoryCursor,
    after_repository_cursor: RepositoryHistoryCursor,
    post_merge_history_guard_evidence_digest: Sha256Digest,
    post_commit_history_partition: ValidatedRepositoryHistoryPartition,
    atomic_commit_safety_capability_id: CapabilityRowId,
    committed_objects: CommittedRepositoryObjects,
    committed_objects_digest: Sha256Digest,
    content_verified: TrueLiteral,
    released_objects: CanonicalRepositoryTargets,
    released_guard_locks: CanonicalRepositoryTargets,
    unlock_verified: TrueLiteral,
    repository_anchor: RepositoryAnchor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) enum ConflictObservationCompleteness {
    JournalOnly,
    ReadOnlySnapshotProven,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ObservedRepositoryConflict {
    target: RepositoryTargetIdentity,
    target_display: RepositoryTargetDisplay,
    locked_by: RequiredNullable<RepositoryUsername>,
    computer: RequiredNullable<RepositoryIdentityComponent>,
    infobase: RequiredNullable<RepositoryIdentityComponent>,
    locked_at: RequiredNullable<NormalizedUtcInstant>,
}

impl ObservedRepositoryConflict {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        target: RepositoryTargetIdentity,
        target_display: RepositoryTargetDisplay,
        locked_by: RequiredNullable<RepositoryUsername>,
        computer: RequiredNullable<RepositoryIdentityComponent>,
        infobase: RequiredNullable<RepositoryIdentityComponent>,
        locked_at: RequiredNullable<NormalizedUtcInstant>,
    ) -> Self {
        Self {
            target,
            target_display,
            locked_by,
            computer,
            infobase,
            locked_at,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct JournaledRepositoryLock {
    target: RepositoryTargetIdentity,
    target_display: RepositoryTargetDisplay,
    lock_set_id: UnicaId,
    observed_at: NormalizedUtcInstant,
}

impl JournaledRepositoryLock {
    pub(crate) fn new(
        target: RepositoryTargetIdentity,
        target_display: RepositoryTargetDisplay,
        lock_set_id: UnicaId,
        observed_at: NormalizedUtcInstant,
    ) -> Self {
        Self {
            target,
            target_display,
            lock_set_id,
            observed_at,
        }
    }

    pub(crate) fn target(&self) -> &RepositoryTargetIdentity {
        &self.target
    }

    pub(crate) fn target_display(&self) -> &RepositoryTargetDisplay {
        &self.target_display
    }

    pub(crate) fn lock_set_id(&self) -> &UnicaId {
        &self.lock_set_id
    }
}

macro_rules! canonical_status_targets {
    ($name:ident, $item:ty) => {
        #[derive(Debug, Clone, PartialEq, Eq, Serialize)]
        #[serde(transparent)]
        pub(crate) struct $name(Vec<$item>);

        impl $name {
            pub(crate) fn new(values: Vec<$item>) -> Result<Self, RepositoryResultContractError> {
                if values.len() > MAX_RESULT_ITEMS
                    || values.windows(2).any(|pair| pair[0].target >= pair[1].target)
                {
                    return Err(RepositoryResultContractError(
                        "status target list must be canonical and duplicate-free",
                    ));
                }
                Ok(Self(values))
            }
        }

        impl JsonSchema for $name {
            fn schema_name() -> Cow<'static, str> {
                stringify!($name).into()
            }

            fn json_schema(generator: &mut SchemaGenerator) -> Schema {
                json_schema!({
                    "type": "array",
                    "maxItems": MAX_RESULT_ITEMS,
                    "uniqueItems": true,
                    "items": generator.subschema_for::<$item>(),
                })
            }
        }
    };
}

canonical_status_targets!(JournaledRepositoryLocks, JournaledRepositoryLock);
canonical_status_targets!(ObservedRepositoryConflicts, ObservedRepositoryConflict);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RepositoryStatusDigestRecord {
    binding_identity: Sha256Digest,
    #[serde(skip_serializing_if = "Option::is_none")]
    repository_version: Option<RepositoryVersion>,
    original_infobase_kind: OriginalInfobaseKind,
    repository_transport: RepositoryTransport,
    main_equals_repository: bool,
    main_equals_database_configuration: bool,
    journaled_locks: JournaledRepositoryLocks,
    last_observed_conflicts: ObservedRepositoryConflicts,
    conflict_observation_completeness: ConflictObservationCompleteness,
    #[serde(skip_serializing_if = "Option::is_none")]
    conflicts_observed_at: Option<NormalizedUtcInstant>,
    #[serde(skip_serializing_if = "Option::is_none")]
    active_operation: Option<ActiveOperationStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    recovery: Option<RecoveryPlanStatus>,
}

impl contract_digest_record_sealed::Sealed for RepositoryStatusDigestRecord {}
impl ContractDigestRecord for RepositoryStatusDigestRecord {}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct RepositoryStatusAuthority(RepositoryStatusDigestRecord);

impl RepositoryStatusAuthority {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_repository_adapter(
        binding_identity: Sha256Digest,
        repository_version: Option<RepositoryVersion>,
        original_infobase_kind: OriginalInfobaseKind,
        repository_transport: RepositoryTransport,
        main_equals_repository: bool,
        main_equals_database_configuration: bool,
        journaled_locks: JournaledRepositoryLocks,
        last_observed_conflicts: ObservedRepositoryConflicts,
        conflict_observation_completeness: ConflictObservationCompleteness,
        conflicts_observed_at: Option<NormalizedUtcInstant>,
        active_operation: Option<ActiveOperationStatus>,
        recovery: Option<RecoveryPlanStatus>,
    ) -> Result<Self, RepositoryResultContractError> {
        if last_observed_conflicts.0.is_empty() != conflicts_observed_at.is_none() {
            return Err(RepositoryResultContractError(
                "conflict observation time must be present exactly for a non-empty conflict set",
            ));
        }
        if recovery.is_some() && active_operation.is_some() {
            return Err(RepositoryResultContractError(
                "repository status cannot publish an active operation beside current recovery",
            ));
        }
        Ok(Self(RepositoryStatusDigestRecord {
            binding_identity,
            repository_version,
            original_infobase_kind,
            repository_transport,
            main_equals_repository,
            main_equals_database_configuration,
            journaled_locks,
            last_observed_conflicts,
            conflict_observation_completeness,
            conflicts_observed_at,
            active_operation,
            recovery,
        }))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RepositoryStatusData {
    binding_identity: Sha256Digest,
    #[serde(skip_serializing_if = "Option::is_none")]
    repository_version: Option<RepositoryVersion>,
    original_infobase_kind: OriginalInfobaseKind,
    repository_transport: RepositoryTransport,
    main_equals_repository: bool,
    main_equals_database_configuration: bool,
    journaled_locks: JournaledRepositoryLocks,
    last_observed_conflicts: ObservedRepositoryConflicts,
    conflict_observation_completeness: ConflictObservationCompleteness,
    #[serde(skip_serializing_if = "Option::is_none")]
    conflicts_observed_at: Option<NormalizedUtcInstant>,
    #[serde(skip_serializing_if = "Option::is_none")]
    active_operation: Option<ActiveOperationStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    recovery: Option<RecoveryPlanStatus>,
    status_digest: Sha256Digest,
}

impl RepositoryStatusData {
    pub(crate) fn from_authority(
        authority: RepositoryStatusAuthority,
    ) -> Result<Self, RepositoryResultContractError> {
        let status_digest = result_digest(&authority.0, "repository-status digest failed")?;
        let record = authority.0;
        Ok(Self {
            binding_identity: record.binding_identity,
            repository_version: record.repository_version,
            original_infobase_kind: record.original_infobase_kind,
            repository_transport: record.repository_transport,
            main_equals_repository: record.main_equals_repository,
            main_equals_database_configuration: record.main_equals_database_configuration,
            journaled_locks: record.journaled_locks,
            last_observed_conflicts: record.last_observed_conflicts,
            conflict_observation_completeness: record.conflict_observation_completeness,
            conflicts_observed_at: record.conflicts_observed_at,
            active_operation: record.active_operation,
            recovery: record.recovery,
            status_digest,
        })
    }
}

fn validate_commit_release_projection(
    plan: &LockPlanData,
    approved_guard_locks: &CanonicalRepositoryTargets,
    released_objects: &CanonicalRepositoryTargets,
    released_guard_locks: &CanonicalRepositoryTargets,
) -> Result<(), RepositoryResultContractError> {
    if released_guard_locks != approved_guard_locks {
        return Err(RepositoryResultContractError(
            "released guard locks differ from the approved commit preview",
        ));
    }
    let expected = project_lock_targets(&plan.lock_entries)?;
    let observed: BTreeSet<_> = released_objects
        .as_slice()
        .iter()
        .chain(released_guard_locks.as_slice())
        .cloned()
        .collect();
    if observed.len() != released_objects.as_slice().len() + released_guard_locks.as_slice().len()
        || observed.into_iter().collect::<Vec<_>>() != expected.as_slice()
    {
        return Err(RepositoryResultContractError(
            "released task objects and guard locks are not the exact approved lock set",
        ));
    }
    Ok(())
}

impl CommitData {
    /// The only production committed-success projection. Both object state and
    /// taskCommit history arrive through one already validated, invocation-
    /// bound authority; independent completion/object authorities cannot be
    /// spliced at this boundary.
    pub(crate) fn from_committed_outcome(authority: CommitCommittedAuthority) -> Self {
        let CommitCommittedAuthority {
            source,
            atomic_invocation: _,
            observation,
        } = authority;
        let CommitCommittedObservationInput {
            core,
            history,
            release,
        } = *observation;
        let (released_objects, released_guard_locks) = match release {
            CommitLockReleaseObservation::Verified(authority) => (*authority)
                .into_released_projection()
                .expect("validated committed release must be a released lock authority"),
            CommitLockReleaseObservation::Unknown => unreachable!(
                "CommitCommittedAuthority is minted only after exact release validation"
            ),
        };
        let CommitAtomicCommitSource {
            immediate_history_guard_evidence,
            before_repository_cursor,
            ..
        } = *source;
        let after_repository_cursor = history
            .post_commit_history_partition
            .partition()
            .through_inclusive()
            .clone();
        let post_merge_history_guard_evidence_digest =
            immediate_history_guard_evidence.evidence_digest().clone();
        let post_commit_history_partition = history.post_commit_history_partition.into_partition();
        Self {
            commit_receipt_id: core.commit_receipt_id,
            repository_version: core.repository_version,
            before_repository_cursor,
            after_repository_cursor,
            post_merge_history_guard_evidence_digest,
            post_commit_history_partition,
            atomic_commit_safety_capability_id: core.atomic_commit_safety_capability_id,
            committed_objects: core.committed_objects,
            committed_objects_digest: core.committed_objects_digest,
            content_verified: TrueLiteral,
            released_objects,
            released_guard_locks,
            unlock_verified: TrueLiteral,
            repository_anchor: history.terminal_repository_anchor,
        }
    }

    #[cfg(test)]
    pub(crate) fn from_authority(
        authority: ValidatedCommitObjectAuthority,
        completion: CommitCompletionObservationAuthority,
    ) -> Result<Self, RepositoryResultContractError> {
        let preview = &authority.approved_preview.0.record;
        if preview.history_guard_evidence.classified_through_cursor()
            != &completion.before_repository_cursor
        {
            return Err(RepositoryResultContractError(
                "commit completion did not start at the approved history-guard cursor",
            ));
        }
        if preview
            .history_guard_evidence
            .atomic_commit_safety_capability_id()
            != &authority.atomic_commit_safety_capability_id
        {
            return Err(RepositoryResultContractError(
                "commit completion capability differs from the approved history guard",
            ));
        }
        if !completion.post_commit_history_partition.binds(&authority) {
            return Err(RepositoryResultContractError(
                "taskCommit history partition belongs to another commit-object authority",
            ));
        }
        validate_commit_release_projection(
            &authority.approved_preview.0.plan,
            &preview.guard_locks,
            &completion.released_objects,
            &completion.released_guard_locks,
        )?;
        let post_merge_history_guard_evidence_digest =
            preview.history_guard_evidence.evidence_digest().clone();
        let post_commit_history_partition =
            completion.post_commit_history_partition.into_partition();
        Ok(Self {
            commit_receipt_id: completion.commit_receipt_id,
            repository_version: authority.repository_version,
            before_repository_cursor: completion.before_repository_cursor,
            after_repository_cursor: completion.after_repository_cursor,
            post_merge_history_guard_evidence_digest,
            post_commit_history_partition,
            atomic_commit_safety_capability_id: authority.atomic_commit_safety_capability_id,
            committed_objects: authority.committed_objects,
            committed_objects_digest: authority.committed_objects_digest,
            content_verified: TrueLiteral,
            released_objects: completion.released_objects,
            released_guard_locks: completion.released_guard_locks,
            unlock_verified: TrueLiteral,
            repository_anchor: completion.repository_anchor,
        })
    }
}

wire_literal!(RoutineUpdateMode, "routine");

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct CanonicalSupportVersionObservations(Vec<SupportPrerequisiteVersionObservation>);

impl CanonicalSupportVersionObservations {
    fn from_validated_history_order(
        values: Vec<SupportPrerequisiteVersionObservation>,
    ) -> Result<Self, RepositoryResultContractError> {
        if values.len() > MAX_RESULT_ITEMS {
            return Err(RepositoryResultContractError(
                "support-version observations exceed the bounded history projection",
            ));
        }
        Ok(Self(values))
    }

    pub(crate) fn as_slice(&self) -> &[SupportPrerequisiteVersionObservation] {
        &self.0
    }
}

impl JsonSchema for CanonicalSupportVersionObservations {
    fn schema_name() -> Cow<'static, str> {
        "CanonicalSupportVersionObservations".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "array",
            "maxItems": MAX_RESULT_ITEMS,
            "uniqueItems": true,
            "items": generator.subschema_for::<SupportPrerequisiteVersionObservation>(),
        })
    }
}

fn observation_has_classification(
    observation: &SupportPrerequisiteVersionObservation,
    expected: RepositoryHistoryPartitionClassification,
) -> bool {
    observation
        .task8_mapping_projection()
        .is_some_and(|projection| projection.partition_classification() == expected)
}

fn exact_partition_observation_projection(
    partition: &ValidatedRepositoryHistoryPartition,
    groups: &[&CanonicalSupportVersionObservations],
) -> bool {
    let entries: Vec<_> = partition.support_recovery_entries().collect();
    let observations: Vec<_> = groups.iter().flat_map(|group| group.as_slice()).collect();
    if entries.len() != observations.len() {
        return false;
    }

    let mut covered = vec![false; entries.len()];
    for group in groups {
        let mut prior_partition_index = None;
        for observation in group.as_slice() {
            let Some(projection) = observation.task8_mapping_projection() else {
                return false;
            };
            let Some((partition_index, entry)) =
                entries.iter().enumerate().find(|(index, entry)| {
                    !covered[*index]
                        && matches!(
                            entry,
                            ValidatedSupportRecoveryHistoryEntryRef::SupportObservation {
                                repository_version,
                                ..
                            } if *repository_version == observation.repository_version()
                        )
                })
            else {
                return false;
            };
            let ValidatedSupportRecoveryHistoryEntryRef::SupportObservation {
                partition_classification,
                source_evidence_digest,
                ..
            } = entry
            else {
                return false;
            };
            if *partition_classification != projection.partition_classification()
                || *source_evidence_digest != observation.classification_digest()
                || prior_partition_index.is_some_and(|prior| prior >= partition_index)
            {
                return false;
            }
            covered[partition_index] = true;
            prior_partition_index = Some(partition_index);
        }
    }
    covered.into_iter().all(std::convert::identity)
}

/// Sealed projection of one complete routine-update preview.  The repository
/// adapter supplies typed history/change/plan values; this constructor closes
/// the exact cursor chain and all derived change subsets before a digest can be
/// published.
#[derive(Debug, PartialEq, Eq)]
struct RoutineUpdateRequestLineage {
    cwd: OriginalProjectCwd,
    task_id: TaskId,
    preview_operation_id: OperationId,
    expected_status_digest: Sha256Digest,
}

impl RoutineUpdateRequestLineage {
    fn from_preview(request: &ValidatedRoutineUpdatePreviewRequest<'_>) -> Self {
        Self {
            cwd: request.cwd().clone(),
            task_id: request.task_id().clone(),
            preview_operation_id: request.operation_id().clone(),
            expected_status_digest: request.expected_status_digest().clone(),
        }
    }
}

fn routine_update_apply_context_matches(
    preview: &RoutineUpdateRequestLineage,
    update_digest: &Sha256Digest,
    apply: &ValidatedRoutineUpdateApplyRequest<'_>,
) -> bool {
    apply.cwd() == &preview.cwd
        && apply.task_id() == &preview.task_id
        && apply.expected_status_digest() == &preview.expected_status_digest
        && apply.operation_id() != &preview.preview_operation_id
        && apply.approved_update_digest() == update_digest
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct RepositoryUpdatePreviewAuthority {
    request_lineage: RoutineUpdateRequestLineage,
    deferred_terminal_receipt_id: Option<UnicaId>,
    record: RepositoryUpdatePreviewDigestRecord,
    update_digest: Sha256Digest,
}

impl RepositoryUpdatePreviewAuthority {
    pub(crate) fn from_projection(
        request: ValidatedRoutineUpdatePreviewRequest<'_>,
        projection: ValidatedRoutineUpdateProjection,
    ) -> Result<Self, RepositoryResultContractError> {
        let planned_relevant_objects =
            CanonicalRepositoryTargets::new(projection.planned_relevant_objects().to_vec())?;
        let planned_unrelated_objects =
            CanonicalRepositoryTargets::new(projection.planned_unrelated_objects().to_vec())?;
        let record = RepositoryUpdatePreviewDigestRecord {
            mode: RoutineUpdateMode::Value,
            before_anchor: projection.before_anchor().clone(),
            expected_history_cursor: projection.expected_history_cursor().clone(),
            observed_history_cursor: projection.observed_history_cursor().clone(),
            deferred_repository_advance: projection.deferred_repository_advance().cloned(),
            deferred_advance_resolution_digest: projection
                .deferred_advance_resolution_digest()
                .cloned(),
            planned_changes: projection.planned_changes().clone(),
            planned_relevant_objects,
            planned_unrelated_objects,
            structural_changes: projection.structural_changes().clone(),
            structural_confirmation_required: projection.structural_confirmation_required(),
            history_partition: projection.history_partition().clone(),
            selective_update_plan: projection.selective_update_plan().clone(),
            resulting_phase: projection.resulting_phase(),
        };
        let update_digest = result_digest(&record, "repository-update preview digest failed")?;
        Ok(Self {
            request_lineage: RoutineUpdateRequestLineage::from_preview(&request),
            deferred_terminal_receipt_id: projection.deferred_terminal_receipt_id().cloned(),
            record,
            update_digest,
        })
    }

    pub(crate) fn approve(
        self,
        request: ValidatedRoutineUpdateApplyRequest<'_>,
    ) -> Result<ApprovedRoutineUpdatePreviewAuthority, RepositoryResultContractError> {
        let lineage_matches = routine_update_apply_context_matches(
            &self.request_lineage,
            &self.update_digest,
            &request,
        );
        if !lineage_matches {
            return Err(RepositoryResultContractError(
                "routine apply request belongs to another preview lineage",
            ));
        }
        Ok(ApprovedRoutineUpdatePreviewAuthority {
            apply_operation_id: request.operation_id().clone(),
            preview: self,
        })
    }

    pub(crate) const fn update_digest(&self) -> &Sha256Digest {
        &self.update_digest
    }

    /// Raw-field construction is deliberately fixture-only.  Production must
    /// consume the capability-backed routine fold/plan/phase authority; until
    /// that producer exists, publishing a preview fails closed instead of
    /// trusting independently supplied values.
    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn fixture_test_only(
        request: ValidatedRoutineUpdatePreviewRequest<'_>,
        before_anchor: RepositoryAnchor,
        expected_history_cursor: RepositoryHistoryCursor,
        observed_history_cursor: RepositoryHistoryCursor,
        deferred_repository_advance: Option<DeferredRepositoryAdvance>,
        deferred_terminal_receipt_id: Option<UnicaId>,
        deferred_advance_resolution_digest: Option<Sha256Digest>,
        planned_changes: RepositoryPlannedChanges,
        planned_relevant_objects: CanonicalRepositoryTargets,
        planned_unrelated_objects: CanonicalRepositoryTargets,
        structural_changes: RepositoryPlannedChanges,
        structural_confirmation_required: bool,
        history_partition: ValidatedRepositoryHistoryPartition,
        selective_update_plan: SelectiveRepositoryUpdatePlan,
        resulting_phase: TaskPhase,
    ) -> Result<Self, RepositoryResultContractError> {
        if before_anchor.history_cursor() != &expected_history_cursor
            || history_partition.start_cursor() != &expected_history_cursor
            || history_partition.through_inclusive() != &observed_history_cursor
        {
            return Err(RepositoryResultContractError(
                "routine-update preview cursor chain mismatch",
            ));
        }
        match (
            deferred_repository_advance.as_ref(),
            deferred_terminal_receipt_id.as_ref(),
            deferred_advance_resolution_digest.as_ref(),
        ) {
            (None, None, None) => {}
            (Some(deferred), Some(_), Some(_))
                if deferred.anchor_cursor() == &expected_history_cursor => {}
            _ => {
                return Err(RepositoryResultContractError(
                    "deferred repository advance and its resolution must be present together at the expected cursor",
                ));
            }
        }

        let expected_relevant: Vec<_> = planned_changes
            .as_slice()
            .iter()
            .filter(|change| {
                change.relevance()
                    == crate::domain::branched_development::contracts::repository::RepositoryRelevance::Relevant
            })
            .map(|change| change.target_identity())
            .collect();
        let expected_unrelated: Vec<_> = planned_changes
            .as_slice()
            .iter()
            .filter(|change| {
                change.relevance()
                    == crate::domain::branched_development::contracts::repository::RepositoryRelevance::Unrelated
            })
            .map(|change| change.target_identity())
            .collect();
        if planned_relevant_objects.as_slice() != expected_relevant
            || planned_unrelated_objects.as_slice() != expected_unrelated
        {
            return Err(RepositoryResultContractError(
                "routine-update relevant/unrelated projections are not exact",
            ));
        }
        let expected_structural: Vec<_> = planned_changes
            .as_slice()
            .iter()
            .filter(|change| change.is_structural())
            .collect();
        let has_expected_structural = !expected_structural.is_empty();
        if structural_changes.as_slice().iter().collect::<Vec<_>>() != expected_structural
            || structural_confirmation_required != has_expected_structural
            || selective_update_plan.scope()
                != SelectiveRepositoryUpdateScope::RoutinePlannedObjects
            || selective_update_plan.structural_confirmation_required()
                != structural_confirmation_required
        {
            return Err(RepositoryResultContractError(
                "routine-update structural projection or selective plan mismatch",
            ));
        }

        let record = RepositoryUpdatePreviewDigestRecord {
            mode: RoutineUpdateMode::Value,
            before_anchor,
            expected_history_cursor,
            observed_history_cursor,
            deferred_repository_advance,
            deferred_advance_resolution_digest,
            planned_changes,
            planned_relevant_objects,
            planned_unrelated_objects,
            structural_changes,
            structural_confirmation_required,
            history_partition,
            selective_update_plan,
            resulting_phase,
        };
        let update_digest = result_digest(&record, "repository-update preview digest failed")?;
        Ok(Self {
            request_lineage: RoutineUpdateRequestLineage::from_preview(&request),
            deferred_terminal_receipt_id,
            record,
            update_digest,
        })
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ApprovedRoutineUpdatePreviewAuthority {
    apply_operation_id: OperationId,
    preview: RepositoryUpdatePreviewAuthority,
}

impl ApprovedRoutineUpdatePreviewAuthority {
    pub(crate) const fn apply_operation_id(&self) -> &OperationId {
        &self.apply_operation_id
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RepositoryUpdatePreviewDigestRecord {
    mode: RoutineUpdateMode,
    before_anchor: RepositoryAnchor,
    expected_history_cursor: RepositoryHistoryCursor,
    observed_history_cursor: RepositoryHistoryCursor,
    #[serde(skip_serializing_if = "Option::is_none")]
    deferred_repository_advance: Option<DeferredRepositoryAdvance>,
    #[serde(skip_serializing_if = "Option::is_none")]
    deferred_advance_resolution_digest: Option<Sha256Digest>,
    planned_changes: RepositoryPlannedChanges,
    planned_relevant_objects: CanonicalRepositoryTargets,
    planned_unrelated_objects: CanonicalRepositoryTargets,
    structural_changes: RepositoryPlannedChanges,
    structural_confirmation_required: bool,
    history_partition: ValidatedRepositoryHistoryPartition,
    selective_update_plan: SelectiveRepositoryUpdatePlan,
    resulting_phase: TaskPhase,
}

impl contract_digest_record_sealed::Sealed for RepositoryUpdatePreviewDigestRecord {}
impl ContractDigestRecord for RepositoryUpdatePreviewDigestRecord {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RepositoryUpdatePreviewData {
    mode: RoutineUpdateMode,
    before_anchor: RepositoryAnchor,
    expected_history_cursor: RepositoryHistoryCursor,
    observed_history_cursor: RepositoryHistoryCursor,
    #[serde(skip_serializing_if = "Option::is_none")]
    deferred_repository_advance: Option<DeferredRepositoryAdvance>,
    #[serde(skip_serializing_if = "Option::is_none")]
    deferred_advance_resolution_digest: Option<Sha256Digest>,
    planned_changes: RepositoryPlannedChanges,
    planned_relevant_objects: CanonicalRepositoryTargets,
    planned_unrelated_objects: CanonicalRepositoryTargets,
    structural_changes: RepositoryPlannedChanges,
    structural_confirmation_required: bool,
    history_partition: ValidatedRepositoryHistoryPartition,
    selective_update_plan: SelectiveRepositoryUpdatePlan,
    resulting_phase: TaskPhase,
    update_digest: Sha256Digest,
}

impl RepositoryUpdatePreviewData {
    pub(crate) fn from_authority(
        authority: &RepositoryUpdatePreviewAuthority,
    ) -> Result<Self, RepositoryResultContractError> {
        let record = authority.record.clone();
        let update_digest = authority.update_digest.clone();
        Ok(Self {
            mode: record.mode,
            before_anchor: record.before_anchor,
            expected_history_cursor: record.expected_history_cursor,
            observed_history_cursor: record.observed_history_cursor,
            deferred_repository_advance: record.deferred_repository_advance,
            deferred_advance_resolution_digest: record.deferred_advance_resolution_digest,
            planned_changes: record.planned_changes,
            planned_relevant_objects: record.planned_relevant_objects,
            planned_unrelated_objects: record.planned_unrelated_objects,
            structural_changes: record.structural_changes,
            structural_confirmation_required: record.structural_confirmation_required,
            history_partition: record.history_partition,
            selective_update_plan: record.selective_update_plan,
            resulting_phase: record.resulting_phase,
            update_digest,
        })
    }
}

wire_literal!(SupportPrerequisiteModeLiteral, "supportPrerequisite");
wire_literal!(
    SupportCancellationModeLiteral,
    "supportPrerequisiteCancellation"
);
wire_literal!(ReservedOriginalModeLiteral, "reservedOriginal");
wire_literal!(
    SeparateWorkingInfobaseModeLiteral,
    "separateWorkingInfobase"
);
wire_literal!(ReadOnlySnapshotModeLiteral, "readOnlySnapshot");
wire_literal!(ApplyGuardOnlyModeLiteral, "applyGuardOnly");

fn support_root_plan_shape_is_exact(
    plan: &SelectiveRepositoryUpdatePlan,
    update_required: bool,
) -> bool {
    if plan.scope() != SelectiveRepositoryUpdateScope::SupportRoot
        || plan.structural_confirmation_required()
        || plan.structural_capability_row_id().is_some()
    {
        return false;
    }
    match plan.planned_targets().as_slice() {
        [] => !update_required,
        [target] => {
            update_required
                && matches!(
                    target.as_ref(),
                    RepositoryTargetStateRef::RootPresent { .. }
                )
        }
        _ => false,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReservedPrerequisitePreviewBinding {
    manual_target_mode: ReservedOriginalModeLiteral,
    reserved_original_lease_capability_id: CapabilityRowId,
    manual_actor_lock_inventory_proof: ManualActorLockInventoryProof,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SeparatePrerequisitePreviewBinding {
    manual_target_mode: SeparateWorkingInfobaseModeLiteral,
    observed_working_infobase_identity: ManualWorkingInfobaseIdentity,
    manual_working_infobase_closure_plan: ManualWorkingInfobaseClosurePlan,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
enum PrerequisitePreviewModeBinding {
    Reserved(ReservedPrerequisitePreviewBinding),
    Separate(Box<SeparatePrerequisitePreviewBinding>),
}

impl JsonSchema for PrerequisitePreviewModeBinding {
    fn schema_name() -> Cow<'static, str> {
        "PrerequisitePreviewModeBinding".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<ReservedPrerequisitePreviewBinding>(),
            generator.subschema_for::<SeparatePrerequisitePreviewBinding>(),
        ])
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReadOnlyRootLockPreviewBinding {
    root_lock_observation_mode: ReadOnlySnapshotModeLiteral,
    preview_root_lock_observation: SupportRootLockObservation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ApplyGuardRootLockPreviewBinding {
    root_lock_observation_mode: ApplyGuardOnlyModeLiteral,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
enum RootLockPreviewBinding {
    ReadOnly(ReadOnlyRootLockPreviewBinding),
    ApplyGuardOnly(ApplyGuardRootLockPreviewBinding),
}

impl JsonSchema for RootLockPreviewBinding {
    fn schema_name() -> Cow<'static, str> {
        "RootLockPreviewBinding".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<ReadOnlyRootLockPreviewBinding>(),
            generator.subschema_for::<ApplyGuardRootLockPreviewBinding>(),
        ])
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SupportPrerequisitePreviewCommon {
    mode: SupportPrerequisiteModeLiteral,
    purpose: SupportActionPurpose,
    origin_phase: TaskPhase,
    post_reconcile_phase: TaskPhase,
    support_gate_id: UnicaId,
    support_action_id: UnicaId,
    support_action_digest: Sha256Digest,
    arming_receipt_id: UnicaId,
    expected_arming_receipt_digest: Sha256Digest,
    arming_cursor: RepositoryHistoryCursor,
    support_gate_digest: Sha256Digest,
    repository_version: RepositoryVersion,
    repository_actor: RepositoryActorIdentity,
    authorized_transitions: SupportTransitions,
    expected_original_fingerprint: Sha256Digest,
    observed_original_fingerprint: Sha256Digest,
    observed_root_delta_digest: Sha256Digest,
    history_partition: ValidatedRepositoryHistoryPartition,
    selective_update_plan: SelectiveRepositoryUpdatePlan,
    concurrent_routine_changes: CanonicalSupportVersionObservations,
    disjoint_external_support_changes: CanonicalSupportVersionObservations,
    lock_guard_digest: Sha256Digest,
    update_required: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SupportPrerequisitePreviewDigestRecord {
    #[serde(flatten)]
    common: SupportPrerequisitePreviewCommon,
    #[serde(flatten)]
    mode_binding: PrerequisitePreviewModeBinding,
    #[serde(flatten)]
    root_lock_binding: RootLockPreviewBinding,
}

impl contract_digest_record_sealed::Sealed for SupportPrerequisitePreviewDigestRecord {}
impl ContractDigestRecord for SupportPrerequisitePreviewDigestRecord {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SupportPrerequisiteLockGuardDigestRecord {
    mandatory_guard_capability_id: CapabilityRowId,
    purpose: SupportActionPurpose,
    origin_phase: TaskPhase,
    post_reconcile_phase: TaskPhase,
    support_gate_id: UnicaId,
    support_gate_digest: Sha256Digest,
    support_action_id: UnicaId,
    support_action_digest: Sha256Digest,
    arming_receipt_id: UnicaId,
    arming_receipt_digest: Sha256Digest,
    arming_cursor: RepositoryHistoryCursor,
    candidate_set_digest: Sha256Digest,
    expected_relevant_baseline_digest: Sha256Digest,
    expected_support_graph_digest: Sha256Digest,
    observed_support_graph_digest: Sha256Digest,
    support_recovery_distribution_set_digest: Sha256Digest,
    authorized_transitions_digest: Sha256Digest,
    phase_evidence_digest: Sha256Digest,
    before_anchor: RepositoryAnchor,
    repository_version: RepositoryVersion,
    repository_actor: RepositoryActorIdentity,
    authorized_observation_digest: Sha256Digest,
    observed_root_delta_digest: Sha256Digest,
    history_partition: ValidatedRepositoryHistoryPartition,
    selective_update_plan: SelectiveRepositoryUpdatePlan,
    concurrent_routine_changes: CanonicalSupportVersionObservations,
    disjoint_external_support_changes: CanonicalSupportVersionObservations,
    expected_original_fingerprint: Sha256Digest,
    observed_original_fingerprint: Sha256Digest,
    manual_target_mode: ManualSupportTargetMode,
    manual_actor_username: RepositoryUsername,
    #[serde(skip_serializing_if = "Option::is_none")]
    manual_actor_lock_baseline_digest: Option<Sha256Digest>,
    reserved_original_identity_digest: Sha256Digest,
    #[serde(skip_serializing_if = "Option::is_none")]
    working_infobase_identity: Option<ManualWorkingInfobaseIdentity>,
    mode_binding: PrerequisitePreviewModeBinding,
    root_lock_binding: RootLockPreviewBinding,
    update_required: bool,
}

impl contract_digest_record_sealed::Sealed for SupportPrerequisiteLockGuardDigestRecord {}
impl ContractDigestRecord for SupportPrerequisiteLockGuardDigestRecord {}

/// Capability-selected preview root-lock mode. The mandatory apply guard row
/// is retained in the digest even when a read-only snapshot is available.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum SupportRootLockPreviewCapabilityAuthority {
    ReadOnlySnapshot {
        mandatory_guard_capability_id: CapabilityRowId,
        observation: SupportRootLockObservation,
    },
    ApplyGuardOnly {
        mandatory_guard_capability_id: CapabilityRowId,
    },
}

impl SupportRootLockPreviewCapabilityAuthority {
    pub(crate) fn read_only_snapshot_from_capability_adapter(
        mandatory_guard_capability_id: CapabilityRowId,
        observation: SupportRootLockObservation,
    ) -> Self {
        Self::ReadOnlySnapshot {
            mandatory_guard_capability_id,
            observation,
        }
    }

    pub(crate) fn apply_guard_only_from_capability_adapter(
        mandatory_guard_capability_id: CapabilityRowId,
    ) -> Self {
        Self::ApplyGuardOnly {
            mandatory_guard_capability_id,
        }
    }

    fn into_parts(self) -> (CapabilityRowId, RootLockPreviewBinding) {
        match self {
            Self::ReadOnlySnapshot {
                mandatory_guard_capability_id,
                observation,
            } => (
                mandatory_guard_capability_id,
                RootLockPreviewBinding::ReadOnly(ReadOnlyRootLockPreviewBinding {
                    root_lock_observation_mode: ReadOnlySnapshotModeLiteral::Value,
                    preview_root_lock_observation: observation,
                }),
            ),
            Self::ApplyGuardOnly {
                mandatory_guard_capability_id,
            } => (
                mandatory_guard_capability_id,
                RootLockPreviewBinding::ApplyGuardOnly(ApplyGuardRootLockPreviewBinding {
                    root_lock_observation_mode: ApplyGuardOnlyModeLiteral::Value,
                }),
            ),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum SupportPrerequisitePreviewModeObservationAuthority {
    ReservedOriginal {
        manual_actor_lock_inventory_proof: ManualActorLockInventoryProof,
    },
    SeparateWorkingInfobase {
        observed_working_infobase_identity: ManualWorkingInfobaseIdentity,
        manual_working_infobase_closure_plan: Box<ManualWorkingInfobaseClosurePlan>,
    },
}

/// Complete typed repository-side evidence for one prerequisite preview.
/// Authorization/request identity and all wire duplicates are derived later.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ValidatedSupportPrerequisitePreviewObservationAuthority {
    before_anchor: RepositoryAnchor,
    observed_original_fingerprint: Sha256Digest,
    observed_support_graph_digest: Sha256Digest,
    history: ValidatedSupportPrerequisiteHistoryProjection,
    support_root_plan_authority: SupportRootSelectiveRepositoryUpdatePlanAuthority,
    mode: SupportPrerequisitePreviewModeObservationAuthority,
    root_lock: SupportRootLockPreviewCapabilityAuthority,
}

impl ValidatedSupportPrerequisitePreviewObservationAuthority {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_repository_adapter(
        before_anchor: RepositoryAnchor,
        observed_original_fingerprint: Sha256Digest,
        observed_support_graph_digest: Sha256Digest,
        history: ValidatedSupportPrerequisiteHistoryProjection,
        support_root_plan_authority: SupportRootSelectiveRepositoryUpdatePlanAuthority,
        mode: SupportPrerequisitePreviewModeObservationAuthority,
        root_lock: SupportRootLockPreviewCapabilityAuthority,
    ) -> Self {
        Self {
            before_anchor,
            observed_original_fingerprint,
            observed_support_graph_digest,
            history,
            support_root_plan_authority,
            mode,
            root_lock,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
struct PrerequisiteUpdateRequestLineage {
    cwd: OriginalProjectCwd,
    task_id: TaskId,
    preview_operation_id: OperationId,
    expected_status_digest: Sha256Digest,
}

impl PrerequisiteUpdateRequestLineage {
    fn from_preview(request: &ValidatedPrerequisiteUpdatePreviewRequest<'_>) -> Self {
        Self {
            cwd: request.cwd().clone(),
            task_id: request.task_id().clone(),
            preview_operation_id: request.operation_id().clone(),
            expected_status_digest: request.expected_status_digest().clone(),
        }
    }
}

fn prerequisite_preview_request_authorization_matches(
    request: &ValidatedPrerequisiteUpdatePreviewRequest<'_>,
    authorization: &SupportUpdateAuthorizationProjection,
) -> bool {
    let Some(receipt) = authorization.arming_receipt() else {
        return false;
    };
    request.support_action_id() == authorization.support_action_id()
        && request.expected_support_action_digest() == authorization.support_action_digest()
        && request.expected_arming_receipt_id() == receipt.arming_receipt_id()
        && request.expected_arming_receipt_digest() == receipt.receipt_digest()
}

fn prerequisite_update_apply_context_matches(
    lineage: &PrerequisiteUpdateRequestLineage,
    authorization: &SupportUpdateAuthorizationProjection,
    update_digest: &Sha256Digest,
    request: &ValidatedPrerequisiteUpdateApplyRequest<'_>,
) -> bool {
    let Some(receipt) = authorization.arming_receipt() else {
        return false;
    };
    request.cwd() == &lineage.cwd
        && request.task_id() == &lineage.task_id
        && request.operation_id() != &lineage.preview_operation_id
        && request.expected_status_digest() == &lineage.expected_status_digest
        && request.support_action_id() == authorization.support_action_id()
        && request.expected_support_action_digest() == authorization.support_action_digest()
        && request.expected_arming_receipt_id() == receipt.arming_receipt_id()
        && request.expected_arming_receipt_digest() == receipt.receipt_digest()
        && request.approved_update_digest() == update_digest
}

/// Non-wire preview authority retaining its exact request and live armed
/// authorization. Approval cannot be reconstructed from the wire digest alone.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct SupportPrerequisitePreviewAuthority {
    request_lineage: PrerequisiteUpdateRequestLineage,
    authorization: SupportUpdateAuthorizationProjection,
    before_anchor: RepositoryAnchor,
    record: SupportPrerequisitePreviewDigestRecord,
    update_digest: Sha256Digest,
}

impl SupportPrerequisitePreviewAuthority {
    pub(crate) fn from_authorities(
        request: ValidatedPrerequisiteUpdatePreviewRequest<'_>,
        authorization: SupportUpdateAuthorizationProjection,
        observation: ValidatedSupportPrerequisitePreviewObservationAuthority,
    ) -> Result<Self, RepositoryResultContractError> {
        if !prerequisite_preview_request_authorization_matches(&request, &authorization) {
            return Err(RepositoryResultContractError(
                "prerequisite preview request differs from its live armed authorization",
            ));
        }
        let receipt =
            authorization
                .arming_receipt()
                .cloned()
                .ok_or(RepositoryResultContractError(
                    "prerequisite preview requires a live armed authorization",
                ))?;
        let ValidatedSupportPrerequisitePreviewObservationAuthority {
            before_anchor,
            observed_original_fingerprint,
            observed_support_graph_digest,
            history,
            support_root_plan_authority,
            mode,
            root_lock,
        } = observation;
        if before_anchor.history_cursor() != authorization.expected_before_history_cursor()
            || history.partition().start_cursor() != authorization.expected_before_history_cursor()
            || !history
                .partition()
                .has_exact_entry_prefix(receipt.history_partition())
            || observed_original_fingerprint != *authorization.expected_original_fingerprint()
            || observed_support_graph_digest != *authorization.expected_support_graph_digest()
        {
            return Err(RepositoryResultContractError(
                "prerequisite preview changed an immutable anchor, arming prefix, original, or support graph",
            ));
        }
        let authorized_observation = history.authorized_observation();
        let authorized = authorized_observation
            .authorized_support_projection()
            .ok_or(RepositoryResultContractError(
                "prerequisite preview lacks its authorized support observation",
            ))?;
        if authorized.support_action_id() != authorization.support_action_id()
            || authorized.support_action_digest() != authorization.support_action_digest()
            || authorized.arming_receipt_id() != receipt.arming_receipt_id()
            || authorized.arming_receipt_digest() != receipt.receipt_digest()
            || authorized.authorized_transitions_digest()
                != authorization.authorized_transitions_digest()
            || authorized.manual_target_mode() != authorization.manual_target_mode()
            || authorized.repository_actor().username() != authorization.manual_actor_username()
        {
            return Err(RepositoryResultContractError(
                "authorized support observation belongs to another action, actor, mode, or transition set",
            ));
        }
        let (selective_update_plan, update_required) =
            SelectiveRepositoryUpdatePlan::support_root_from_authority(
                support_root_plan_authority,
                &before_anchor,
                &history,
            )
            .map_err(|_| {
                RepositoryResultContractError(
                    "support-root plan authority differs from the authorized observation",
                )
            })?;
        let prefix_len = receipt.history_partition().entry_versions().count();
        let classifications: Vec<_> = history.partition().classifications().collect();
        let first_root_or_support = history
            .partition()
            .entry_versions()
            .zip(history.partition().classifications())
            .skip(prefix_len)
            .find(|(_, classification)| {
                !matches!(
                    classification,
                    RepositoryHistoryPartitionClassification::UnrelatedRoutine
                        | RepositoryHistoryPartitionClassification::RelevantRoutine
                )
            });
        if !matches!(
            first_root_or_support,
            Some((version, RepositoryHistoryPartitionClassification::AuthorizedSupport))
                if version == authorized_observation.repository_version()
        ) || classifications
            .iter()
            .filter(|classification| {
                **classification == RepositoryHistoryPartitionClassification::AuthorizedSupport
            })
            .count()
            != 1
        {
            return Err(RepositoryResultContractError(
                "authorized support version is not the exact first root/support entry after arming",
            ));
        }
        let mode_binding = match (authorization.manual_target_mode(), mode) {
            (
                ManualSupportTargetMode::ReservedOriginal,
                SupportPrerequisitePreviewModeObservationAuthority::ReservedOriginal {
                    manual_actor_lock_inventory_proof,
                },
            ) if authorization
                .reserved_original_lease_capability_id()
                .is_some()
                && authorization.manual_actor_lock_baseline_digest()
                    == Some(manual_actor_lock_inventory_proof.baseline_lock_set_digest())
                && authorization.manual_actor_username()
                    == manual_actor_lock_inventory_proof.username()
                && authorization.manual_working_infobase_identity().is_none()
                && authorization.manual_working_infobase_baseline().is_none() =>
            {
                PrerequisitePreviewModeBinding::Reserved(ReservedPrerequisitePreviewBinding {
                    manual_target_mode: ReservedOriginalModeLiteral::Value,
                    reserved_original_lease_capability_id: authorization
                        .reserved_original_lease_capability_id()
                        .expect("reserved branch checked its lease")
                        .clone(),
                    manual_actor_lock_inventory_proof,
                })
            }
            (
                ManualSupportTargetMode::SeparateWorkingInfobase,
                SupportPrerequisitePreviewModeObservationAuthority::SeparateWorkingInfobase {
                    observed_working_infobase_identity,
                    manual_working_infobase_closure_plan,
                },
            ) if authorization
                .reserved_original_lease_capability_id()
                .is_none()
                && authorization.manual_actor_lock_baseline_digest().is_none()
                && authorization.manual_working_infobase_identity()
                    == Some(&observed_working_infobase_identity)
                && authorized.working_infobase_identity()
                    == Some(&observed_working_infobase_identity)
                && manual_working_infobase_closure_plan.materialized().is_ok()
                && manual_working_infobase_closure_plan.working_infobase_identity()
                    == &observed_working_infobase_identity
                && authorization
                    .manual_working_infobase_baseline()
                    .is_some_and(|baseline| {
                        manual_working_infobase_closure_plan.authorization_baseline_digest()
                            == baseline.baseline_digest()
                            && manual_working_infobase_closure_plan.desired_base_fingerprint()
                                == baseline.current_fingerprint()
                            && manual_working_infobase_closure_plan
                                .desired_object_fingerprint_map_digest()
                                == baseline.recorded_object_version_map_digest()
                            && manual_working_infobase_closure_plan.desired_support_graph_digest()
                                == baseline.support_graph_digest()
                            && manual_working_infobase_closure_plan.working_infobase_base_cursor()
                                == Some(baseline.repository_base_cursor())
                            && manual_working_infobase_closure_plan
                                .recorded_object_version_map_digest()
                                == Some(baseline.recorded_object_version_map_digest())
                            && manual_working_infobase_closure_plan.exclusive_lease_capability_id()
                                == baseline.exclusive_lease_capability_id()
                    }) =>
            {
                let actor = authorized.repository_actor();
                if actor.computer() != Some(observed_working_infobase_identity.computer())
                    || actor.infobase() != Some(observed_working_infobase_identity.infobase())
                {
                    return Err(RepositoryResultContractError(
                        "authorized support actor does not name the bound working infobase",
                    ));
                }
                PrerequisitePreviewModeBinding::Separate(Box::new(
                    SeparatePrerequisitePreviewBinding {
                        manual_target_mode: SeparateWorkingInfobaseModeLiteral::Value,
                        observed_working_infobase_identity,
                        manual_working_infobase_closure_plan: *manual_working_infobase_closure_plan,
                    },
                ))
            }
            _ => {
                return Err(RepositoryResultContractError(
                    "prerequisite preview mode evidence differs from its immutable authorization",
                ));
            }
        };
        let authorized_repository_version = authorized_observation.repository_version().clone();
        let authorized_observation_digest = authorized_observation.classification_digest().clone();
        let authorized_repository_actor = authorized.repository_actor().clone();
        let authorized_root_delta_digest = authorized.root_delta_digest().clone();
        let (history_partition, observations, authorized_index) = history.into_parts();
        let mut concurrent = Vec::new();
        let mut external = Vec::new();
        for (index, observation) in observations.into_iter().enumerate() {
            if index == authorized_index {
                continue;
            }
            match observation
                .task8_mapping_projection()
                .expect("validated prerequisite history contains only Task8 observations")
                .partition_classification()
            {
                RepositoryHistoryPartitionClassification::UnrelatedRoutine
                | RepositoryHistoryPartitionClassification::RelevantRoutine => {
                    concurrent.push(observation)
                }
                RepositoryHistoryPartitionClassification::ExternalSupport => {
                    external.push(observation)
                }
                _ => unreachable!(
                    "validated prerequisite history has one separately retained authorized entry"
                ),
            }
        }
        let concurrent_routine_changes =
            CanonicalSupportVersionObservations::from_validated_history_order(concurrent)?;
        let disjoint_external_support_changes =
            CanonicalSupportVersionObservations::from_validated_history_order(external)?;
        if !support_root_plan_shape_is_exact(&selective_update_plan, update_required) {
            return Err(RepositoryResultContractError(
                "prerequisite selective plan is not the exact support-root projection",
            ));
        }
        let (mandatory_guard_capability_id, root_lock_binding) = root_lock.into_parts();
        if matches!(
            &root_lock_binding,
            RootLockPreviewBinding::ReadOnly(ReadOnlyRootLockPreviewBinding {
                preview_root_lock_observation,
                ..
            }) if preview_root_lock_observation.owner().as_ref().is_some()
        ) {
            return Err(RepositoryResultContractError(
                "prerequisite read-only root observation still has an owner",
            ));
        }
        let lock_guard_digest = result_digest(
            &SupportPrerequisiteLockGuardDigestRecord {
                mandatory_guard_capability_id,
                purpose: authorization.purpose(),
                origin_phase: authorization.origin_phase(),
                post_reconcile_phase: authorization.post_reconcile_phase(),
                support_gate_id: authorization.support_gate_id().clone(),
                support_gate_digest: authorization.support_gate_digest().clone(),
                support_action_id: authorization.support_action_id().clone(),
                support_action_digest: authorization.support_action_digest().clone(),
                arming_receipt_id: receipt.arming_receipt_id().clone(),
                arming_receipt_digest: receipt.receipt_digest().clone(),
                arming_cursor: receipt.arming_cursor().clone(),
                candidate_set_digest: authorization.candidate_set_digest().clone(),
                expected_relevant_baseline_digest: authorization
                    .expected_relevant_baseline_digest()
                    .clone(),
                expected_support_graph_digest: authorization
                    .expected_support_graph_digest()
                    .clone(),
                observed_support_graph_digest: observed_support_graph_digest.clone(),
                support_recovery_distribution_set_digest: authorization
                    .support_recovery_distribution_set_digest()
                    .clone(),
                authorized_transitions_digest: authorization
                    .authorized_transitions_digest()
                    .clone(),
                phase_evidence_digest: authorization.phase_evidence_digest().clone(),
                before_anchor: before_anchor.clone(),
                repository_version: authorized_repository_version.clone(),
                repository_actor: authorized_repository_actor.clone(),
                authorized_observation_digest: authorized_observation_digest.clone(),
                observed_root_delta_digest: authorized_root_delta_digest.clone(),
                history_partition: history_partition.clone(),
                selective_update_plan: selective_update_plan.clone(),
                concurrent_routine_changes: concurrent_routine_changes.clone(),
                disjoint_external_support_changes: disjoint_external_support_changes.clone(),
                expected_original_fingerprint: authorization
                    .expected_original_fingerprint()
                    .clone(),
                observed_original_fingerprint: observed_original_fingerprint.clone(),
                manual_target_mode: authorization.manual_target_mode(),
                manual_actor_username: authorization.manual_actor_username().clone(),
                manual_actor_lock_baseline_digest: authorization
                    .manual_actor_lock_baseline_digest()
                    .cloned(),
                reserved_original_identity_digest: authorization
                    .reserved_original_identity_digest()
                    .clone(),
                working_infobase_identity: authorization
                    .manual_working_infobase_identity()
                    .cloned(),
                mode_binding: mode_binding.clone(),
                root_lock_binding: root_lock_binding.clone(),
                update_required,
            },
            "support-prerequisite lock-guard digest failed",
        )?;
        let common = SupportPrerequisitePreviewCommon {
            mode: SupportPrerequisiteModeLiteral::Value,
            purpose: authorization.purpose(),
            origin_phase: authorization.origin_phase(),
            post_reconcile_phase: authorization.post_reconcile_phase(),
            support_gate_id: authorization.support_gate_id().clone(),
            support_action_id: authorization.support_action_id().clone(),
            support_action_digest: authorization.support_action_digest().clone(),
            arming_receipt_id: receipt.arming_receipt_id().clone(),
            expected_arming_receipt_digest: receipt.receipt_digest().clone(),
            arming_cursor: receipt.arming_cursor().clone(),
            support_gate_digest: authorization.support_gate_digest().clone(),
            repository_version: authorized_repository_version,
            repository_actor: authorized_repository_actor,
            authorized_transitions: authorization.authorized_transitions().clone(),
            expected_original_fingerprint: authorization.expected_original_fingerprint().clone(),
            observed_original_fingerprint,
            observed_root_delta_digest: authorized_root_delta_digest,
            history_partition,
            selective_update_plan,
            concurrent_routine_changes,
            disjoint_external_support_changes,
            lock_guard_digest,
            update_required,
        };
        let record = SupportPrerequisitePreviewDigestRecord {
            common,
            mode_binding,
            root_lock_binding,
        };
        let update_digest = result_digest(&record, "support-prerequisite preview digest failed")?;
        Ok(Self {
            request_lineage: PrerequisiteUpdateRequestLineage::from_preview(&request),
            authorization,
            before_anchor,
            record,
            update_digest,
        })
    }

    pub(crate) fn approve(
        self,
        request: ValidatedPrerequisiteUpdateApplyRequest<'_>,
    ) -> Result<ApprovedSupportPrerequisitePreviewAuthority, RepositoryResultContractError> {
        if !prerequisite_update_apply_context_matches(
            &self.request_lineage,
            &self.authorization,
            &self.update_digest,
            &request,
        ) {
            return Err(RepositoryResultContractError(
                "prerequisite apply request belongs to another preview lineage",
            ));
        }
        Ok(ApprovedSupportPrerequisitePreviewAuthority {
            apply_operation_id: request.operation_id().clone(),
            preview: self,
        })
    }

    pub(crate) const fn update_digest(&self) -> &Sha256Digest {
        &self.update_digest
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ApprovedSupportPrerequisitePreviewAuthority {
    apply_operation_id: OperationId,
    preview: SupportPrerequisitePreviewAuthority,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SupportPrerequisitePreviewData {
    #[serde(flatten)]
    common: SupportPrerequisitePreviewCommon,
    #[serde(flatten)]
    mode_binding: PrerequisitePreviewModeBinding,
    #[serde(flatten)]
    root_lock_binding: RootLockPreviewBinding,
    update_digest: Sha256Digest,
}

impl JsonSchema for SupportPrerequisitePreviewData {
    fn schema_name() -> Cow<'static, str> {
        "SupportPrerequisitePreviewData".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        macro_rules! branch {
            ($mode:ty, $root:ty) => {
                closed_flattened_object_schema(
                    vec![
                        SupportPrerequisitePreviewCommon::json_schema(generator),
                        <$mode>::json_schema(generator),
                        <$root>::json_schema(generator),
                    ],
                    vec![("updateDigest", generator.subschema_for::<Sha256Digest>())],
                )
            };
        }
        one_of_schema(vec![
            branch!(
                ReservedPrerequisitePreviewBinding,
                ReadOnlyRootLockPreviewBinding
            ),
            branch!(
                ReservedPrerequisitePreviewBinding,
                ApplyGuardRootLockPreviewBinding
            ),
            branch!(
                SeparatePrerequisitePreviewBinding,
                ReadOnlyRootLockPreviewBinding
            ),
            branch!(
                SeparatePrerequisitePreviewBinding,
                ApplyGuardRootLockPreviewBinding
            ),
        ])
    }
}

impl SupportPrerequisitePreviewData {
    pub(crate) fn from_authority(authority: &SupportPrerequisitePreviewAuthority) -> Self {
        Self {
            common: authority.record.common.clone(),
            mode_binding: authority.record.mode_binding.clone(),
            root_lock_binding: authority.record.root_lock_binding.clone(),
            update_digest: authority.update_digest.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct CanonicalRepositoryVersions(Vec<RepositoryVersion>);

impl CanonicalRepositoryVersions {
    pub(crate) fn from_validated_partition(
        partition: &ValidatedRepositoryHistoryPartition,
    ) -> Result<Self, RepositoryResultContractError> {
        let values: Vec<_> = partition.entry_versions().cloned().collect();
        if values.len() > MAX_RESULT_ITEMS {
            return Err(RepositoryResultContractError(
                "repository versions exceed the bounded validated history partition",
            ));
        }
        Ok(Self(values))
    }

    pub(crate) fn as_slice(&self) -> &[RepositoryVersion] {
        &self.0
    }
}

impl JsonSchema for CanonicalRepositoryVersions {
    fn schema_name() -> Cow<'static, str> {
        "CanonicalRepositoryVersions".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "array",
            "maxItems": MAX_RESULT_ITEMS,
            "uniqueItems": true,
            "items": generator.subschema_for::<RepositoryVersion>(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AwaitingCancellationPreviewBinding {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ArmedCancellationPreviewBinding {
    arming_receipt_id: UnicaId,
    expected_arming_receipt_digest: Sha256Digest,
    arming_cursor: RepositoryHistoryCursor,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
enum CancellationArmingBinding {
    Awaiting(AwaitingCancellationPreviewBinding),
    Armed(ArmedCancellationPreviewBinding),
}

impl JsonSchema for CancellationArmingBinding {
    fn schema_name() -> Cow<'static, str> {
        "CancellationArmingBinding".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<AwaitingCancellationPreviewBinding>(),
            generator.subschema_for::<ArmedCancellationPreviewBinding>(),
        ])
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReservedCancellationPreviewBinding {
    reserved_original_lease_capability_id: CapabilityRowId,
    manual_actor_lock_inventory_proof: ManualActorLockInventoryProof,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SeparateCancellationPreviewBinding {
    manual_working_infobase_closure_plan: ManualWorkingInfobaseClosurePlan,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
enum CancellationModeBinding {
    Reserved(ReservedCancellationPreviewBinding),
    Separate(SeparateCancellationPreviewBinding),
}

impl JsonSchema for CancellationModeBinding {
    fn schema_name() -> Cow<'static, str> {
        "CancellationModeBinding".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<ReservedCancellationPreviewBinding>(),
            generator.subschema_for::<SeparateCancellationPreviewBinding>(),
        ])
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SupportPrerequisiteCancellationPreviewCommon {
    mode: SupportCancellationModeLiteral,
    purpose: SupportActionPurpose,
    origin_phase: TaskPhase,
    cancelled_phase: TaskPhase,
    relevant_advance_phase: TaskPhase,
    support_action_id: UnicaId,
    support_action_digest: Sha256Digest,
    prior_support_gate_id: UnicaId,
    reason: SupportCancellationReason,
    before_anchor: RepositoryAnchor,
    observed_repository_versions: CanonicalRepositoryVersions,
    history_partition: ValidatedRepositoryHistoryPartition,
    selective_update_plan: SelectiveRepositoryUpdatePlan,
    partitioned_routine_changes: CanonicalSupportVersionObservations,
    relevant_routine_changes: CanonicalSupportVersionObservations,
    disjoint_external_support_changes: CanonicalSupportVersionObservations,
    pre_arm_external_changes: CanonicalSupportVersionObservations,
    expected_original_fingerprint: Sha256Digest,
    observed_original_fingerprint: Sha256Digest,
    expected_support_graph_digest: Sha256Digest,
    observed_support_graph_digest: Sha256Digest,
    lock_guard_digest: Sha256Digest,
    update_required: bool,
    planned_result_phase: TaskPhase,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SupportPrerequisiteCancellationPreviewDigestRecord {
    #[serde(flatten)]
    common: SupportPrerequisiteCancellationPreviewCommon,
    #[serde(flatten)]
    arming_binding: CancellationArmingBinding,
    #[serde(flatten)]
    mode_binding: CancellationModeBinding,
    #[serde(flatten)]
    root_lock_binding: RootLockPreviewBinding,
}

impl contract_digest_record_sealed::Sealed for SupportPrerequisiteCancellationPreviewDigestRecord {}
impl ContractDigestRecord for SupportPrerequisiteCancellationPreviewDigestRecord {}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct SupportPrerequisiteCancellationPreviewAuthority {
    cwd: OriginalProjectCwd,
    task_id: TaskId,
    preview_operation_id: OperationId,
    expected_status_digest: Sha256Digest,
    authorization: SupportUpdateAuthorizationProjection,
    reserved_original_identity_digest: Sha256Digest,
    record: SupportPrerequisiteCancellationPreviewDigestRecord,
    cancellation_digest: Sha256Digest,
}

/// Complete capability-backed runtime projection for one cancellation
/// preview.  The repository/history/root-lock resolver must mint this opaque
/// authority; this result module deliberately exposes no production raw-field
/// constructor.  The final preview additionally consumes the live immutable
/// support authorization and the typed request token.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ValidatedCancellationPreviewObservationAuthority {
    reserved_original_identity_digest: Sha256Digest,
    common: SupportPrerequisiteCancellationPreviewCommon,
    arming_binding: CancellationArmingBinding,
    mode_binding: CancellationModeBinding,
    root_lock_binding: RootLockPreviewBinding,
    arming_receipt: Option<SupportActionArmingReceipt>,
}

impl ValidatedCancellationPreviewObservationAuthority {
    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    fn fixture_test_only(
        reserved_original_identity_digest: Sha256Digest,
        common: SupportPrerequisiteCancellationPreviewCommon,
        arming_binding: CancellationArmingBinding,
        mode_binding: CancellationModeBinding,
        root_lock_binding: RootLockPreviewBinding,
        arming_receipt: Option<SupportActionArmingReceipt>,
    ) -> Self {
        Self {
            reserved_original_identity_digest,
            common,
            arming_binding,
            mode_binding,
            root_lock_binding,
            arming_receipt,
        }
    }
}

fn cancellation_apply_context_matches(
    cwd: &OriginalProjectCwd,
    task_id: &TaskId,
    expected_status_digest: &Sha256Digest,
    request: &ValidatedCancellationApplyRequest<'_>,
) -> bool {
    request.cwd() == cwd
        && request.task_id() == task_id
        && request.expected_status_digest() == expected_status_digest
}

fn cancellation_preview_request_authorization_matches(
    request: &ValidatedCancellationPreviewRequest<'_>,
    authorization: &SupportUpdateAuthorizationProjection,
) -> bool {
    request.support_action_id() == authorization.support_action_id()
        && request.expected_support_action_digest() == authorization.support_action_digest()
        && match (request.arming(), authorization.arming_receipt()) {
            (ValidatedCancellationArming::Awaiting, None) => true,
            (
                ValidatedCancellationArming::Armed {
                    expected_arming_receipt_id,
                    expected_arming_receipt_digest,
                },
                Some(receipt),
            ) => {
                *expected_arming_receipt_id == receipt.arming_receipt_id()
                    && *expected_arming_receipt_digest == receipt.receipt_digest()
            }
            _ => false,
        }
}

impl SupportPrerequisiteCancellationPreviewAuthority {
    pub(crate) fn from_authorities(
        request: ValidatedCancellationPreviewRequest<'_>,
        authorization: SupportUpdateAuthorizationProjection,
        observation: ValidatedCancellationPreviewObservationAuthority,
    ) -> Result<Self, RepositoryResultContractError> {
        let ValidatedCancellationPreviewObservationAuthority {
            reserved_original_identity_digest,
            common,
            arming_binding,
            mode_binding,
            root_lock_binding,
            arming_receipt,
        } = observation;
        let request_arming_matches = match (request.arming(), &arming_binding) {
            (ValidatedCancellationArming::Awaiting, CancellationArmingBinding::Awaiting(_)) => true,
            (
                ValidatedCancellationArming::Armed {
                    expected_arming_receipt_id,
                    expected_arming_receipt_digest,
                },
                CancellationArmingBinding::Armed(binding),
            ) => {
                *expected_arming_receipt_id == &binding.arming_receipt_id
                    && *expected_arming_receipt_digest == &binding.expected_arming_receipt_digest
            }
            _ => false,
        };
        if request.support_action_id() != &common.support_action_id
            || request.expected_support_action_digest() != &common.support_action_digest
            || request.reason() != common.reason
            || !request_arming_matches
            || !cancellation_preview_request_authorization_matches(&request, &authorization)
        {
            return Err(RepositoryResultContractError(
                "cancellation preview request selectors do not match its projection",
            ));
        }
        if common.purpose != authorization.purpose()
            || common.origin_phase != authorization.origin_phase()
            || common.cancelled_phase != authorization.cancelled_phase()
            || common.relevant_advance_phase != authorization.relevant_advance_phase()
            || common.prior_support_gate_id != *authorization.support_gate_id()
            || common.before_anchor.history_cursor()
                != authorization.expected_before_history_cursor()
            || common.before_anchor.history_cursor() != common.history_partition.start_cursor()
            || common.expected_original_fingerprint
                != *authorization.expected_original_fingerprint()
            || common.observed_original_fingerprint != common.expected_original_fingerprint
            || common.expected_support_graph_digest
                != *authorization.expected_support_graph_digest()
            || common.observed_support_graph_digest != common.expected_support_graph_digest
            || reserved_original_identity_digest
                != *authorization.reserved_original_identity_digest()
        {
            return Err(RepositoryResultContractError(
                "cancellation preview differs from the immutable support authorization",
            ));
        }
        let actual_versions: Vec<_> = common.history_partition.entry_versions().collect();
        if actual_versions.len() != common.observed_repository_versions.as_slice().len()
            || actual_versions
                .iter()
                .zip(common.observed_repository_versions.as_slice())
                .any(|(actual, observed)| *actual != observed)
        {
            return Err(RepositoryResultContractError(
                "cancellation observed-version list is not the exact history partition",
            ));
        }
        if !common
            .partitioned_routine_changes
            .as_slice()
            .iter()
            .all(|value| {
                observation_has_classification(
                    value,
                    RepositoryHistoryPartitionClassification::UnrelatedRoutine,
                )
            })
            || !common
                .relevant_routine_changes
                .as_slice()
                .iter()
                .all(|value| {
                    observation_has_classification(
                        value,
                        RepositoryHistoryPartitionClassification::RelevantRoutine,
                    )
                })
            || !common
                .disjoint_external_support_changes
                .as_slice()
                .iter()
                .all(|value| {
                    observation_has_classification(
                        value,
                        RepositoryHistoryPartitionClassification::ExternalSupport,
                    )
                })
            || !common
                .pre_arm_external_changes
                .as_slice()
                .iter()
                .all(|value| {
                    observation_has_classification(
                        value,
                        RepositoryHistoryPartitionClassification::PreArmExternal,
                    )
                })
            || !exact_partition_observation_projection(
                &common.history_partition,
                &[
                    &common.partitioned_routine_changes,
                    &common.relevant_routine_changes,
                    &common.disjoint_external_support_changes,
                    &common.pre_arm_external_changes,
                ],
            )
        {
            return Err(RepositoryResultContractError(
                "cancellation classified lists are not the exact history partition",
            ));
        }
        match (&arming_binding, arming_receipt) {
            (CancellationArmingBinding::Awaiting(_), None) => {}
            (CancellationArmingBinding::Armed(binding), Some(receipt))
                if common.pre_arm_external_changes.as_slice().is_empty()
                    && &binding.arming_receipt_id == receipt.arming_receipt_id()
                    && &binding.expected_arming_receipt_digest == receipt.receipt_digest()
                    && &binding.arming_cursor == receipt.arming_cursor()
                    && common
                        .history_partition
                        .has_exact_entry_prefix(receipt.history_partition()) => {}
            _ => {
                return Err(RepositoryResultContractError(
                    "cancellation arming fields or immutable prefix mismatch",
                ));
            }
        }
        let mode_matches_authorization = match (&mode_binding, authorization.manual_target_mode()) {
            (
                CancellationModeBinding::Reserved(binding),
                ManualSupportTargetMode::ReservedOriginal,
            ) => {
                authorization.reserved_original_lease_capability_id()
                    == Some(&binding.reserved_original_lease_capability_id)
                    && authorization.manual_actor_lock_baseline_digest()
                        == Some(
                            binding
                                .manual_actor_lock_inventory_proof
                                .baseline_lock_set_digest(),
                        )
                    && authorization.manual_actor_username()
                        == binding.manual_actor_lock_inventory_proof.username()
                    && authorization.manual_working_infobase_identity().is_none()
                    && authorization.manual_working_infobase_baseline().is_none()
            }
            (
                CancellationModeBinding::Separate(binding),
                ManualSupportTargetMode::SeparateWorkingInfobase,
            ) => {
                let plan = &binding.manual_working_infobase_closure_plan;
                match (
                    authorization.manual_working_infobase_identity(),
                    authorization.manual_working_infobase_baseline(),
                    plan.materialized(),
                ) {
                    (Some(identity), Some(baseline), Ok(_)) => {
                        authorization
                            .reserved_original_lease_capability_id()
                            .is_none()
                            && authorization.manual_actor_lock_baseline_digest().is_none()
                            && plan.working_infobase_identity() == identity
                            && plan.working_infobase_identity()
                                == baseline.working_infobase_identity()
                            && plan.authorization_baseline_digest() == baseline.baseline_digest()
                            && plan.desired_base_fingerprint() == baseline.current_fingerprint()
                            && plan.desired_object_fingerprint_map_digest()
                                == baseline.recorded_object_version_map_digest()
                            && plan.desired_support_graph_digest()
                                == baseline.support_graph_digest()
                            && plan.working_infobase_base_cursor()
                                == Some(baseline.repository_base_cursor())
                            && plan.recorded_object_version_map_digest()
                                == Some(baseline.recorded_object_version_map_digest())
                            && plan.exclusive_lease_capability_id()
                                == baseline.exclusive_lease_capability_id()
                    }
                    _ => false,
                }
            }
            _ => false,
        };
        if !mode_matches_authorization {
            return Err(RepositoryResultContractError(
                "cancellation mode evidence differs from the immutable authorization",
            ));
        }
        let preserves_support = !common
            .disjoint_external_support_changes
            .as_slice()
            .is_empty()
            || !common.pre_arm_external_changes.as_slice().is_empty();
        let requires_relevant_phase =
            preserves_support || !common.relevant_routine_changes.as_slice().is_empty();
        if common.update_required != preserves_support
            || !support_root_plan_shape_is_exact(
                &common.selective_update_plan,
                common.update_required,
            )
            || common.planned_result_phase
                != if requires_relevant_phase {
                    common.relevant_advance_phase
                } else {
                    common.cancelled_phase
                }
        {
            return Err(RepositoryResultContractError(
                "cancellation selective plan or result phase mismatch",
            ));
        }
        let record = SupportPrerequisiteCancellationPreviewDigestRecord {
            common,
            arming_binding,
            mode_binding,
            root_lock_binding,
        };
        let cancellation_digest = result_digest(&record, "cancellation preview digest failed")?;
        Ok(Self {
            cwd: request.cwd().clone(),
            task_id: request.task_id().clone(),
            preview_operation_id: request.operation_id().clone(),
            expected_status_digest: request.expected_status_digest().clone(),
            authorization,
            reserved_original_identity_digest,
            record,
            cancellation_digest,
        })
    }

    pub(crate) fn approve(
        self,
        request: ValidatedCancellationApplyRequest<'_>,
    ) -> Result<
        ApprovedSupportPrerequisiteCancellationPreviewAuthority,
        RepositoryResultContractError,
    > {
        let common = &self.record.common;
        if request.support_action_id() != &common.support_action_id
            || request.expected_support_action_digest() != &common.support_action_digest
            || request.reason() != common.reason
            || request.approved_cancellation_digest() != &self.cancellation_digest
            || !cancellation_apply_context_matches(
                &self.cwd,
                &self.task_id,
                &self.expected_status_digest,
                &request,
            )
        {
            return Err(RepositoryResultContractError(
                "cancellation apply request belongs to another preview lineage",
            ));
        }
        let arming_matches = match (request.arming(), &self.record.arming_binding) {
            (ValidatedCancellationArming::Awaiting, CancellationArmingBinding::Awaiting(_)) => true,
            (
                ValidatedCancellationArming::Armed {
                    expected_arming_receipt_id,
                    expected_arming_receipt_digest,
                },
                CancellationArmingBinding::Armed(binding),
            ) => {
                *expected_arming_receipt_id == &binding.arming_receipt_id
                    && *expected_arming_receipt_digest == &binding.expected_arming_receipt_digest
            }
            _ => false,
        };
        if !arming_matches {
            return Err(RepositoryResultContractError(
                "cancellation apply request changes the approved arming branch",
            ));
        }
        Ok(ApprovedSupportPrerequisiteCancellationPreviewAuthority {
            apply_operation_id: request.operation_id().clone(),
            preview: self,
        })
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ApprovedSupportPrerequisiteCancellationPreviewAuthority {
    apply_operation_id: OperationId,
    preview: SupportPrerequisiteCancellationPreviewAuthority,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ApprovedCancellationPreviewModeProjection<'a> {
    ReservedOriginal {
        reserved_original_identity_digest: &'a Sha256Digest,
        exclusive_lease_capability_id: &'a CapabilityRowId,
    },
    SeparateWorkingInfobase {
        working_infobase_identity: &'a ManualWorkingInfobaseIdentity,
        exclusive_lease_capability_id: &'a CapabilityRowId,
        closure_plan_digest: &'a Sha256Digest,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ApprovedCancellationPreviewProjection<'a> {
    prior_operation_id: &'a OperationId,
    support_action_id: &'a UnicaId,
    expected_support_action_digest: &'a Sha256Digest,
    approved_cancellation_digest: &'a Sha256Digest,
    mode: ApprovedCancellationPreviewModeProjection<'a>,
}

impl ApprovedCancellationPreviewProjection<'_> {
    pub(crate) const fn prior_operation_id(&self) -> &OperationId {
        self.prior_operation_id
    }

    pub(crate) const fn support_action_id(&self) -> &UnicaId {
        self.support_action_id
    }

    pub(crate) const fn expected_support_action_digest(&self) -> &Sha256Digest {
        self.expected_support_action_digest
    }

    pub(crate) const fn approved_cancellation_digest(&self) -> &Sha256Digest {
        self.approved_cancellation_digest
    }

    pub(crate) const fn mode(&self) -> ApprovedCancellationPreviewModeProjection<'_> {
        self.mode
    }
}

impl ApprovedSupportPrerequisiteCancellationPreviewAuthority {
    pub(crate) fn projection(&self) -> ApprovedCancellationPreviewProjection<'_> {
        let preview = &self.preview;
        let mode = match &preview.record.mode_binding {
            CancellationModeBinding::Reserved(binding) => {
                ApprovedCancellationPreviewModeProjection::ReservedOriginal {
                    reserved_original_identity_digest: &preview.reserved_original_identity_digest,
                    exclusive_lease_capability_id: &binding.reserved_original_lease_capability_id,
                }
            }
            CancellationModeBinding::Separate(binding) => {
                ApprovedCancellationPreviewModeProjection::SeparateWorkingInfobase {
                    working_infobase_identity: binding
                        .manual_working_infobase_closure_plan
                        .working_infobase_identity(),
                    exclusive_lease_capability_id: binding
                        .manual_working_infobase_closure_plan
                        .exclusive_lease_capability_id(),
                    closure_plan_digest: binding.manual_working_infobase_closure_plan.plan_digest(),
                }
            }
        };
        ApprovedCancellationPreviewProjection {
            prior_operation_id: &self.apply_operation_id,
            support_action_id: &preview.record.common.support_action_id,
            expected_support_action_digest: &preview.record.common.support_action_digest,
            approved_cancellation_digest: &preview.cancellation_digest,
            mode,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SupportPrerequisiteCancellationPreviewData {
    #[serde(flatten)]
    common: SupportPrerequisiteCancellationPreviewCommon,
    #[serde(flatten)]
    arming_binding: CancellationArmingBinding,
    #[serde(flatten)]
    mode_binding: CancellationModeBinding,
    #[serde(flatten)]
    root_lock_binding: RootLockPreviewBinding,
    cancellation_digest: Sha256Digest,
}

impl JsonSchema for SupportPrerequisiteCancellationPreviewData {
    fn schema_name() -> Cow<'static, str> {
        "SupportPrerequisiteCancellationPreviewData".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        macro_rules! branch {
            ($arming:ty, $mode:ty, $root:ty) => {
                closed_flattened_object_schema(
                    vec![
                        SupportPrerequisiteCancellationPreviewCommon::json_schema(generator),
                        <$arming>::json_schema(generator),
                        <$mode>::json_schema(generator),
                        <$root>::json_schema(generator),
                    ],
                    vec![(
                        "cancellationDigest",
                        generator.subschema_for::<Sha256Digest>(),
                    )],
                )
            };
        }
        one_of_schema(vec![
            branch!(
                AwaitingCancellationPreviewBinding,
                ReservedCancellationPreviewBinding,
                ReadOnlyRootLockPreviewBinding
            ),
            branch!(
                AwaitingCancellationPreviewBinding,
                ReservedCancellationPreviewBinding,
                ApplyGuardRootLockPreviewBinding
            ),
            branch!(
                AwaitingCancellationPreviewBinding,
                SeparateCancellationPreviewBinding,
                ReadOnlyRootLockPreviewBinding
            ),
            branch!(
                AwaitingCancellationPreviewBinding,
                SeparateCancellationPreviewBinding,
                ApplyGuardRootLockPreviewBinding
            ),
            branch!(
                ArmedCancellationPreviewBinding,
                ReservedCancellationPreviewBinding,
                ReadOnlyRootLockPreviewBinding
            ),
            branch!(
                ArmedCancellationPreviewBinding,
                ReservedCancellationPreviewBinding,
                ApplyGuardRootLockPreviewBinding
            ),
            branch!(
                ArmedCancellationPreviewBinding,
                SeparateCancellationPreviewBinding,
                ReadOnlyRootLockPreviewBinding
            ),
            branch!(
                ArmedCancellationPreviewBinding,
                SeparateCancellationPreviewBinding,
                ApplyGuardRootLockPreviewBinding
            ),
        ])
    }
}

impl SupportPrerequisiteCancellationPreviewData {
    pub(crate) fn from_authority(
        authority: SupportPrerequisiteCancellationPreviewAuthority,
    ) -> Self {
        Self {
            common: authority.record.common,
            arming_binding: authority.record.arming_binding,
            mode_binding: authority.record.mode_binding,
            root_lock_binding: authority.record.root_lock_binding,
            cancellation_digest: authority.cancellation_digest,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ArmedCancellationResultBinding {
    arming_receipt_id: UnicaId,
    arming_receipt_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AwaitingCancellationResultBinding {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
enum CancellationResultArmingBinding {
    Awaiting(AwaitingCancellationResultBinding),
    Armed(ArmedCancellationResultBinding),
}

impl JsonSchema for CancellationResultArmingBinding {
    fn schema_name() -> Cow<'static, str> {
        "CancellationResultArmingBinding".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<AwaitingCancellationResultBinding>(),
            generator.subschema_for::<ArmedCancellationResultBinding>(),
        ])
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReservedCancellationResultBinding {
    manual_target_mode: ReservedOriginalModeLiteral,
    manual_actor_lock_inventory_proof: ManualActorLockInventoryProof,
    reserved_original_terminalization_proof: ReservedOriginalTerminalizationProof,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SeparateCancellationResultBinding {
    manual_target_mode: SeparateWorkingInfobaseModeLiteral,
    manual_working_infobase_closure_proof: ManualWorkingInfobaseClosureProof,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
enum CancellationResultModeBinding {
    Reserved(ReservedCancellationResultBinding),
    Separate(SeparateCancellationResultBinding),
}

impl JsonSchema for CancellationResultModeBinding {
    fn schema_name() -> Cow<'static, str> {
        "CancellationResultModeBinding".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<ReservedCancellationResultBinding>(),
            generator.subschema_for::<SeparateCancellationResultBinding>(),
        ])
    }
}

/// Durable repository result receipt bound by the adapter to the exact apply
/// operation and selective-update effect proof.  It is deliberately non-wire
/// and non-`Clone`: result constructors must consume it and compare the hidden
/// operation lineage retained by the approved preview.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct RepositoryApplyOperationReceiptAuthority {
    operation_id: OperationId,
    receipt_id: UnicaId,
    selective_update_proof_digest: Sha256Digest,
}

impl RepositoryApplyOperationReceiptAuthority {
    pub(crate) fn from_capability_adapter(
        operation_id: OperationId,
        receipt_id: UnicaId,
        selective_update_proof: &SelectiveRepositoryUpdateProof,
    ) -> Self {
        Self {
            operation_id,
            receipt_id,
            selective_update_proof_digest: selective_update_proof.proof_digest().clone(),
        }
    }

    fn binds(
        &self,
        expected_operation_id: &OperationId,
        selective_update_proof: &SelectiveRepositoryUpdateProof,
    ) -> bool {
        self.binds_operation_and_proof_digest(
            expected_operation_id,
            selective_update_proof.proof_digest(),
        )
    }

    fn binds_operation_and_proof_digest(
        &self,
        expected_operation_id: &OperationId,
        selective_update_proof_digest: &Sha256Digest,
    ) -> bool {
        &self.operation_id == expected_operation_id
            && &self.selective_update_proof_digest == selective_update_proof_digest
    }

    fn into_receipt_id(self) -> UnicaId {
        self.receipt_id
    }

    #[cfg(test)]
    fn fixture_test_only(
        operation_id: OperationId,
        receipt_id: UnicaId,
        selective_update_proof_digest: Sha256Digest,
    ) -> Self {
        Self {
            operation_id,
            receipt_id,
            selective_update_proof_digest,
        }
    }
}

/// Immutable lineage for the one root-guard window that encloses selective
/// update and support terminalization. The adapter lease must bind this entire
/// record before terminal status is committed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SupportRootGuardCompletionBinding {
    cwd: OriginalProjectCwd,
    task_id: TaskId,
    operation_id: OperationId,
    expected_status_digest: Sha256Digest,
    support_action_id: UnicaId,
    support_action_digest: Sha256Digest,
    plan_digest: Sha256Digest,
    history_partition_digest: Sha256Digest,
    lock_guard_digest: Sha256Digest,
    selective_update_proof_digest: Sha256Digest,
    guard_receipt_id: UnicaId,
    authorization_outcome: SupportAuthorizationOutcome,
    terminalization_proof_digest: Option<Sha256Digest>,
}

impl SupportRootGuardCompletionBinding {
    #[allow(clippy::too_many_arguments)]
    fn new(
        cwd: OriginalProjectCwd,
        task_id: TaskId,
        operation_id: OperationId,
        expected_status_digest: Sha256Digest,
        support_action_id: UnicaId,
        support_action_digest: Sha256Digest,
        plan_digest: Sha256Digest,
        history_partition_digest: Sha256Digest,
        lock_guard_digest: Sha256Digest,
        selective_update_proof_digest: Sha256Digest,
        guard_receipt_id: UnicaId,
        authorization_outcome: SupportAuthorizationOutcome,
        terminalization_proof_digest: Option<Sha256Digest>,
    ) -> Self {
        Self {
            cwd,
            task_id,
            operation_id,
            expected_status_digest,
            support_action_id,
            support_action_digest,
            plan_digest,
            history_partition_digest,
            lock_guard_digest,
            selective_update_proof_digest,
            guard_receipt_id,
            authorization_outcome,
            terminalization_proof_digest,
        }
    }

    pub(crate) const fn cwd(&self) -> &OriginalProjectCwd {
        &self.cwd
    }

    pub(crate) const fn task_id(&self) -> &TaskId {
        &self.task_id
    }

    pub(crate) const fn operation_id(&self) -> &OperationId {
        &self.operation_id
    }

    pub(crate) const fn expected_status_digest(&self) -> &Sha256Digest {
        &self.expected_status_digest
    }

    pub(crate) const fn support_action_id(&self) -> &UnicaId {
        &self.support_action_id
    }

    pub(crate) const fn support_action_digest(&self) -> &Sha256Digest {
        &self.support_action_digest
    }

    pub(crate) const fn plan_digest(&self) -> &Sha256Digest {
        &self.plan_digest
    }

    pub(crate) const fn history_partition_digest(&self) -> &Sha256Digest {
        &self.history_partition_digest
    }

    pub(crate) const fn lock_guard_digest(&self) -> &Sha256Digest {
        &self.lock_guard_digest
    }

    pub(crate) const fn selective_update_proof_digest(&self) -> &Sha256Digest {
        &self.selective_update_proof_digest
    }

    pub(crate) const fn guard_receipt_id(&self) -> &UnicaId {
        &self.guard_receipt_id
    }

    pub(crate) const fn authorization_outcome(&self) -> SupportAuthorizationOutcome {
        self.authorization_outcome
    }

    pub(crate) const fn terminalization_proof_digest(&self) -> Option<&Sha256Digest> {
        self.terminalization_proof_digest.as_ref()
    }
}

pub(crate) trait SupportRootGuardCompletionLease {
    fn binds(&self, binding: &SupportRootGuardCompletionBinding) -> bool;

    fn root_guard_release_receipt_id(&self) -> &UnicaId;

    fn commit_terminal_and_release(
        self: Box<Self>,
        terminal: &TerminalSupportActionAuthorization,
    ) -> Result<SupportRootLockProof, RepositoryResultContractError>;
}

pub(crate) trait SupportRootGuardCompletionResolver {
    fn resume(
        &mut self,
        binding: &SupportRootGuardCompletionBinding,
    ) -> Result<Box<dyn SupportRootGuardCompletionLease>, RepositoryResultContractError>;
}

/// One-shot continuation of the exact root guard acquired by this apply.
/// Production completion cannot accept a deserialized `SupportRootLockProof`;
/// the proof is produced only after terminal CAS and release this lease.
pub(crate) struct ValidatedSupportRootGuardCompletionAuthority {
    binding: SupportRootGuardCompletionBinding,
    expected_release_receipt_id: UnicaId,
    lease: Box<dyn SupportRootGuardCompletionLease>,
}

impl fmt::Debug for ValidatedSupportRootGuardCompletionAuthority {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ValidatedSupportRootGuardCompletionAuthority")
            .field("binding", &self.binding)
            .field(
                "expected_release_receipt_id",
                &self.expected_release_receipt_id,
            )
            .finish_non_exhaustive()
    }
}

impl ValidatedSupportRootGuardCompletionAuthority {
    fn resume(
        binding: SupportRootGuardCompletionBinding,
        resolver: &mut dyn SupportRootGuardCompletionResolver,
    ) -> Result<Self, RepositoryResultContractError> {
        let lease = resolver.resume(&binding)?;
        if !lease.binds(&binding)
            || lease.root_guard_release_receipt_id() == binding.guard_receipt_id()
        {
            return Err(RepositoryResultContractError(
                "root-guard completion lease belongs to another apply window",
            ));
        }
        let expected_release_receipt_id = lease.root_guard_release_receipt_id().clone();
        Ok(Self {
            binding,
            expected_release_receipt_id,
            lease,
        })
    }

    fn commit_terminal_and_release(
        self,
        terminal: &TerminalSupportActionAuthorization,
    ) -> Result<SupportRootLockProof, RepositoryResultContractError> {
        let proof = self.lease.commit_terminal_and_release(terminal)?;
        if proof.guard_receipt_id() != self.binding.guard_receipt_id()
            || proof.root_guard_release_receipt_id() != &self.expected_release_receipt_id
            || proof.authorization_outcome() != self.binding.authorization_outcome()
            || proof.reserved_original_terminalization_proof_digest()
                != self.binding.terminalization_proof_digest()
        {
            return Err(RepositoryResultContractError(
                "root-guard completion proof differs from its acquired operation window",
            ));
        }
        Ok(proof)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum CancellationCompletionModeProof {
    ReservedOriginal {
        manual_actor_lock_inventory_proof: ManualActorLockInventoryProof,
        terminalization_proof: ReservedOriginalTerminalizationProof,
    },
    SeparateWorkingInfobase {
        closure_proof: ManualWorkingInfobaseClosureProof,
    },
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct SupportActionCancellationCompletionObservation {
    cancellation_receipt: RepositoryApplyOperationReceiptAuthority,
    mode_proof: CancellationCompletionModeProof,
    before_anchor: RepositoryAnchor,
    after_anchor: RepositoryAnchor,
    changed_relevant_objects: CanonicalRepositoryTargets,
    changed_unrelated_objects: CanonicalRepositoryTargets,
    applied_structural_changes: RepositoryPlannedChanges,
    reconciled_history_partition: ValidatedRepositoryHistoryPartition,
    selective_update_proof: SelectiveRepositoryUpdateProof,
    post_release_observed_history_cursor: RepositoryHistoryCursor,
    post_apply_history_partition: ValidatedRepositoryHistoryPartition,
    deferred_repository_advance: Option<DeferredRepositoryAdvance>,
    resulting_phase: TaskPhase,
}

impl SupportActionCancellationCompletionObservation {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_repository_adapter(
        cancellation_receipt: RepositoryApplyOperationReceiptAuthority,
        mode_proof: CancellationCompletionModeProof,
        before_anchor: RepositoryAnchor,
        after_anchor: RepositoryAnchor,
        changed_relevant_objects: CanonicalRepositoryTargets,
        changed_unrelated_objects: CanonicalRepositoryTargets,
        applied_structural_changes: RepositoryPlannedChanges,
        reconciled_history_partition: ValidatedRepositoryHistoryPartition,
        selective_update_proof: SelectiveRepositoryUpdateProof,
        post_release_observed_history_cursor: RepositoryHistoryCursor,
        post_apply_history_partition: ValidatedRepositoryHistoryPartition,
        deferred_repository_advance: Option<DeferredRepositoryAdvance>,
        resulting_phase: TaskPhase,
    ) -> Self {
        Self {
            cancellation_receipt,
            mode_proof,
            before_anchor,
            after_anchor,
            changed_relevant_objects,
            changed_unrelated_objects,
            applied_structural_changes,
            reconciled_history_partition,
            selective_update_proof,
            post_release_observed_history_cursor,
            post_apply_history_partition,
            deferred_repository_advance,
            resulting_phase,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SupportActionCancellationCommon {
    support_action_id: UnicaId,
    purpose: SupportActionPurpose,
    prior_support_gate_id: UnicaId,
    support_root_lock_proof: SupportRootLockProof,
    cancellation_receipt_id: UnicaId,
    before_anchor: RepositoryAnchor,
    after_anchor: RepositoryAnchor,
    changed_relevant_objects: CanonicalRepositoryTargets,
    changed_unrelated_objects: CanonicalRepositoryTargets,
    applied_structural_changes: RepositoryPlannedChanges,
    reconciled_history_partition: ValidatedRepositoryHistoryPartition,
    selective_update_proof: SelectiveRepositoryUpdateProof,
    post_release_observed_history_cursor: RepositoryHistoryCursor,
    post_apply_history_partition: ValidatedRepositoryHistoryPartition,
    #[serde(skip_serializing_if = "Option::is_none")]
    deferred_repository_advance: Option<DeferredRepositoryAdvance>,
    resulting_phase: TaskPhase,
    reason: SupportCancellationReason,
    cancellation_digest: Sha256Digest,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ValidatedSupportActionCancellationAuthority {
    apply_operation_id: OperationId,
    common: SupportActionCancellationCommon,
    arming_binding: CancellationResultArmingBinding,
    mode_binding: CancellationResultModeBinding,
}

fn cancellation_tail_classification_is_admissible(
    classification: RepositoryHistoryPartitionClassification,
    awaiting: bool,
) -> bool {
    matches!(
        classification,
        RepositoryHistoryPartitionClassification::UnrelatedRoutine
            | RepositoryHistoryPartitionClassification::RelevantRoutine
            | RepositoryHistoryPartitionClassification::ExternalSupport
    ) || (awaiting && classification == RepositoryHistoryPartitionClassification::PreArmExternal)
}

impl ValidatedSupportActionCancellationAuthority {
    pub(crate) fn validate(
        approved: ApprovedSupportPrerequisiteCancellationPreviewAuthority,
        live_authorization: ActiveSupportActionResumeHandle,
        completion: SupportActionCancellationCompletionObservation,
        status_cas_resolver: &mut dyn SupportActionTerminalStatusCasResolver,
        root_guard_resolver: &mut dyn SupportRootGuardCompletionResolver,
    ) -> Result<Self, RepositoryResultContractError> {
        let ApprovedSupportPrerequisiteCancellationPreviewAuthority {
            apply_operation_id,
            preview,
        } = approved;
        let preview_common = &preview.record.common;
        if live_authorization
            .support_update_authorization_projection()
            .as_ref()
            != Some(&preview.authorization)
        {
            return Err(RepositoryResultContractError(
                "cancellation completion received another live support authorization",
            ));
        }
        if completion.before_anchor != preview_common.before_anchor
            || completion.reconciled_history_partition != preview_common.history_partition
            || completion.selective_update_proof.plan_digest()
                != preview_common.selective_update_plan.plan_digest()
            || completion.selective_update_proof.update_performed()
                != preview_common.update_required
            || completion.selective_update_proof.observed_before_cursor()
                != completion.reconciled_history_partition.through_inclusive()
            || completion.post_apply_history_partition.start_cursor()
                != completion.selective_update_proof.observed_before_cursor()
            || completion.post_apply_history_partition.through_inclusive()
                != &completion.post_release_observed_history_cursor
            || completion.after_anchor.history_cursor()
                != completion.selective_update_proof.observed_after_cursor()
            || !completion
                .cancellation_receipt
                .binds(&apply_operation_id, &completion.selective_update_proof)
            || !completion.applied_structural_changes.as_slice().is_empty()
        {
            return Err(RepositoryResultContractError(
                "cancellation completion does not reproduce its approved plan and cursor chain",
            ));
        }
        let expected_changed_relevant = CanonicalRepositoryTargets::new(
            preview_common
                .update_required
                .then(RepositoryTargetIdentity::configuration_root)
                .into_iter()
                .collect(),
        )?;
        if completion.changed_relevant_objects != expected_changed_relevant
            || !completion.changed_unrelated_objects.as_slice().is_empty()
        {
            return Err(RepositoryResultContractError(
                "cancellation changed-object projections differ from the exact support-root effect",
            ));
        }
        let awaiting = matches!(
            &preview.record.arming_binding,
            CancellationArmingBinding::Awaiting(_)
        );
        if completion
            .post_apply_history_partition
            .classifications()
            .any(|classification| {
                !cancellation_tail_classification_is_admissible(classification, awaiting)
            })
        {
            return Err(RepositoryResultContractError(
                "cancellation post-apply tail contains a disallowed successor instead of deferring before it",
            ));
        }
        match completion.deferred_repository_advance.as_ref() {
            Some(deferred)
                if deferred.anchor_cursor() == &completion.post_release_observed_history_cursor
                    && deferred.first_observed_version().is_none_or(|successor| {
                        completion
                            .post_apply_history_partition
                            .entry_versions()
                            .all(|included| included != successor)
                    }) => {}
            None => {}
            Some(_) => {
                return Err(RepositoryResultContractError(
                    "cancellation deferred advance is not anchored at the terminal scan cursor",
                ));
            }
        }
        let tail_relevant = completion
            .post_apply_history_partition
            .classifications()
            .any(|classification| {
                matches!(
                    classification,
                    RepositoryHistoryPartitionClassification::RelevantRoutine
                        | RepositoryHistoryPartitionClassification::ExternalSupport
                        | RepositoryHistoryPartitionClassification::PreArmExternal
                )
            });
        let expected_phase = if completion.deferred_repository_advance.is_some() || tail_relevant {
            preview_common.relevant_advance_phase
        } else {
            preview_common.planned_result_phase
        };
        if completion.resulting_phase != expected_phase {
            return Err(RepositoryResultContractError(
                "cancellation completion phase does not follow the approved/tail evidence",
            ));
        }

        let arming_binding = match preview.record.arming_binding {
            CancellationArmingBinding::Awaiting(_) => {
                CancellationResultArmingBinding::Awaiting(AwaitingCancellationResultBinding {})
            }
            CancellationArmingBinding::Armed(binding) => {
                CancellationResultArmingBinding::Armed(ArmedCancellationResultBinding {
                    arming_receipt_id: binding.arming_receipt_id,
                    arming_receipt_digest: binding.expected_arming_receipt_digest,
                })
            }
        };
        let expected_terminalization_proof_digest = match &completion.mode_proof {
            CancellationCompletionModeProof::ReservedOriginal {
                terminalization_proof,
                ..
            } => Some(terminalization_proof.proof_digest().clone()),
            CancellationCompletionModeProof::SeparateWorkingInfobase { .. } => None,
        };
        let mode_binding = match (preview.record.mode_binding, completion.mode_proof) {
            (
                CancellationModeBinding::Reserved(preview_mode),
                CancellationCompletionModeProof::ReservedOriginal {
                    manual_actor_lock_inventory_proof,
                    terminalization_proof,
                },
            ) if terminalization_proof.reserved_original_identity_digest()
                == &preview.reserved_original_identity_digest
                && terminalization_proof.exclusive_lease_capability_id()
                    == &preview_mode.reserved_original_lease_capability_id
                && terminalization_proof.expected_repository_fingerprint()
                    == completion.after_anchor.configuration_fingerprint()
                && manual_actor_lock_inventory_proof.username()
                    == preview_mode.manual_actor_lock_inventory_proof.username()
                && manual_actor_lock_inventory_proof.baseline_lock_set_digest()
                    == preview_mode
                        .manual_actor_lock_inventory_proof
                        .baseline_lock_set_digest() =>
            {
                CancellationResultModeBinding::Reserved(ReservedCancellationResultBinding {
                    manual_target_mode: ReservedOriginalModeLiteral::Value,
                    manual_actor_lock_inventory_proof,
                    reserved_original_terminalization_proof: terminalization_proof,
                })
            }
            (
                CancellationModeBinding::Separate(preview_mode),
                CancellationCompletionModeProof::SeparateWorkingInfobase { closure_proof },
            ) if closure_proof.working_infobase_identity()
                == preview_mode
                    .manual_working_infobase_closure_plan
                    .working_infobase_identity()
                && closure_proof.plan_digest()
                    == preview_mode
                        .manual_working_infobase_closure_plan
                        .plan_digest()
                && closure_proof.exclusive_lease_capability_id()
                    == preview_mode
                        .manual_working_infobase_closure_plan
                        .exclusive_lease_capability_id() =>
            {
                CancellationResultModeBinding::Separate(SeparateCancellationResultBinding {
                    manual_target_mode: SeparateWorkingInfobaseModeLiteral::Value,
                    manual_working_infobase_closure_proof: closure_proof,
                })
            }
            _ => {
                return Err(RepositoryResultContractError(
                    "cancellation completion mode proof belongs to another authorization",
                ));
            }
        };

        let cancellation_receipt_id = completion.cancellation_receipt.into_receipt_id();
        let root_guard = ValidatedSupportRootGuardCompletionAuthority::resume(
            SupportRootGuardCompletionBinding::new(
                preview.cwd.clone(),
                preview.task_id.clone(),
                apply_operation_id.clone(),
                preview.expected_status_digest.clone(),
                preview_common.support_action_id.clone(),
                preview_common.support_action_digest.clone(),
                preview_common.selective_update_plan.plan_digest().clone(),
                preview_common.history_partition.partition_digest().clone(),
                preview_common.lock_guard_digest.clone(),
                completion.selective_update_proof.proof_digest().clone(),
                completion.selective_update_proof.guard_receipt_id().clone(),
                SupportAuthorizationOutcome::Cancelled,
                expected_terminalization_proof_digest,
            ),
            root_guard_resolver,
        )?;
        let status_cas = ValidatedSupportActionTerminalStatusCasAuthority::acquire(
            SupportActionTerminalStatusCasBinding::new(
                preview.cwd.clone(),
                preview.task_id.clone(),
                apply_operation_id.clone(),
                preview.expected_status_digest.clone(),
                preview_common.support_action_id.clone(),
                preview_common.support_action_digest.clone(),
                SupportActionTerminalOutcome::Cancelled,
                cancellation_receipt_id.clone(),
                completion.selective_update_proof.proof_digest().clone(),
                completion.resulting_phase,
                completion
                    .deferred_repository_advance
                    .as_ref()
                    .map(|advance| advance.observation_digest().clone()),
            ),
            status_cas_resolver,
        )
        .map_err(|_| {
            RepositoryResultContractError(
                "cancellation status CAS could not bind the approved apply operation",
            )
        })?;
        let terminal_authorization = live_authorization
            .terminalize_with_status_cas(status_cas)
            .map_err(|_| {
                RepositoryResultContractError(
                    "cancellation could not atomically terminalize the live authorization",
                )
            })?;
        let support_root_lock_proof =
            root_guard.commit_terminal_and_release(&terminal_authorization)?;
        Ok(Self {
            apply_operation_id,
            common: SupportActionCancellationCommon {
                support_action_id: preview_common.support_action_id.clone(),
                purpose: preview_common.purpose,
                prior_support_gate_id: preview_common.prior_support_gate_id.clone(),
                support_root_lock_proof,
                cancellation_receipt_id,
                before_anchor: completion.before_anchor,
                after_anchor: completion.after_anchor,
                changed_relevant_objects: completion.changed_relevant_objects,
                changed_unrelated_objects: completion.changed_unrelated_objects,
                applied_structural_changes: completion.applied_structural_changes,
                reconciled_history_partition: completion.reconciled_history_partition,
                selective_update_proof: completion.selective_update_proof,
                post_release_observed_history_cursor: completion
                    .post_release_observed_history_cursor,
                post_apply_history_partition: completion.post_apply_history_partition,
                deferred_repository_advance: completion.deferred_repository_advance,
                resulting_phase: completion.resulting_phase,
                reason: preview_common.reason,
                cancellation_digest: preview.cancellation_digest,
            },
            arming_binding,
            mode_binding,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SupportActionCancellationData {
    #[serde(flatten)]
    common: SupportActionCancellationCommon,
    #[serde(flatten)]
    arming_binding: CancellationResultArmingBinding,
    #[serde(flatten)]
    mode_binding: CancellationResultModeBinding,
}

impl JsonSchema for SupportActionCancellationData {
    fn schema_name() -> Cow<'static, str> {
        "SupportActionCancellationData".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        macro_rules! branch {
            ($arming:ty, $mode:ty) => {
                closed_flattened_object_schema(
                    vec![
                        SupportActionCancellationCommon::json_schema(generator),
                        <$arming>::json_schema(generator),
                        <$mode>::json_schema(generator),
                    ],
                    Vec::new(),
                )
            };
        }
        one_of_schema(vec![
            branch!(
                AwaitingCancellationResultBinding,
                ReservedCancellationResultBinding
            ),
            branch!(
                AwaitingCancellationResultBinding,
                SeparateCancellationResultBinding
            ),
            branch!(
                ArmedCancellationResultBinding,
                ReservedCancellationResultBinding
            ),
            branch!(
                ArmedCancellationResultBinding,
                SeparateCancellationResultBinding
            ),
        ])
    }
}

impl SupportActionCancellationData {
    pub(crate) fn from_authority(authority: ValidatedSupportActionCancellationAuthority) -> Self {
        let _apply_operation_id = authority.apply_operation_id;
        Self {
            common: authority.common,
            arming_binding: authority.arming_binding,
            mode_binding: authority.mode_binding,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RepositoryUpdateCompletionMode {
    Routine,
    SupportPrerequisiteReservedOriginal,
    SupportPrerequisiteSeparateWorkingInfobase,
}

/// Typed adapter observation for completing one approved routine update.  It
/// deliberately omits the approved plan/partition/before anchor: those are
/// consumed from `ApprovedRoutineUpdatePreviewAuthority` and compared here.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct RoutineUpdateCompletionObservationAuthority {
    after_anchor: RepositoryAnchor,
    changed_relevant_objects: CanonicalRepositoryTargets,
    changed_unrelated_objects: CanonicalRepositoryTargets,
    applied_structural_changes: RepositoryPlannedChanges,
    original_fingerprint: Sha256Digest,
    update_receipt: RepositoryApplyOperationReceiptAuthority,
    reconciled_history_partition: ValidatedRepositoryHistoryPartition,
    selective_update_proof: SelectiveRepositoryUpdateProof,
    post_release_observed_history_cursor: RepositoryHistoryCursor,
    post_apply_history_partition: ValidatedRepositoryHistoryPartition,
    deferred_advance_consumption_receipt: Option<DeferredRepositoryAdvanceConsumptionReceipt>,
    resulting_phase: TaskPhase,
}

impl RoutineUpdateCompletionObservationAuthority {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_repository_adapter(
        after_anchor: RepositoryAnchor,
        changed_relevant_objects: CanonicalRepositoryTargets,
        changed_unrelated_objects: CanonicalRepositoryTargets,
        applied_structural_changes: RepositoryPlannedChanges,
        original_fingerprint: Sha256Digest,
        update_receipt: RepositoryApplyOperationReceiptAuthority,
        reconciled_history_partition: ValidatedRepositoryHistoryPartition,
        selective_update_proof: SelectiveRepositoryUpdateProof,
        post_release_observed_history_cursor: RepositoryHistoryCursor,
        post_apply_history_partition: ValidatedRepositoryHistoryPartition,
        deferred_advance_consumption_receipt: Option<DeferredRepositoryAdvanceConsumptionReceipt>,
        resulting_phase: TaskPhase,
    ) -> Self {
        Self {
            after_anchor,
            changed_relevant_objects,
            changed_unrelated_objects,
            applied_structural_changes,
            original_fingerprint,
            update_receipt,
            reconciled_history_partition,
            selective_update_proof,
            post_release_observed_history_cursor,
            post_apply_history_partition,
            deferred_advance_consumption_receipt,
            resulting_phase,
        }
    }
}

/// Durable support-reconciliation receipt projected from the adapter journal.
/// Every semantic member that the public receipt is required to attest is
/// retained here and compared with the consumed approved preview.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct SupportPrerequisiteReceiptAuthority {
    operation_id: OperationId,
    receipt_id: UnicaId,
    support_action_id: UnicaId,
    support_action_digest: Sha256Digest,
    arming_receipt_id: UnicaId,
    arming_receipt_digest: Sha256Digest,
    repository_version: RepositoryVersion,
    repository_actor: RepositoryActorIdentity,
    authorized_transitions: SupportTransitions,
    authorized_root_delta_digest: Sha256Digest,
    preserved_external_support_changes: CanonicalSupportVersionObservations,
    selective_update_proof_digest: Sha256Digest,
}

impl SupportPrerequisiteReceiptAuthority {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_capability_adapter(
        operation_id: OperationId,
        receipt_id: UnicaId,
        support_action_id: UnicaId,
        support_action_digest: Sha256Digest,
        arming_receipt_id: UnicaId,
        arming_receipt_digest: Sha256Digest,
        repository_version: RepositoryVersion,
        repository_actor: RepositoryActorIdentity,
        authorized_transitions: SupportTransitions,
        authorized_root_delta_digest: Sha256Digest,
        preserved_external_support_changes: Vec<SupportPrerequisiteVersionObservation>,
        selective_update_proof: &SelectiveRepositoryUpdateProof,
    ) -> Result<Self, RepositoryResultContractError> {
        Ok(Self {
            operation_id,
            receipt_id,
            support_action_id,
            support_action_digest,
            arming_receipt_id,
            arming_receipt_digest,
            repository_version,
            repository_actor,
            authorized_transitions,
            authorized_root_delta_digest,
            preserved_external_support_changes:
                CanonicalSupportVersionObservations::from_validated_history_order(
                    preserved_external_support_changes,
                )?,
            selective_update_proof_digest: selective_update_proof.proof_digest().clone(),
        })
    }

    fn matches(
        &self,
        operation_id: &OperationId,
        preview: &SupportPrerequisitePreviewCommon,
        selective_update_proof: &SelectiveRepositoryUpdateProof,
    ) -> bool {
        &self.operation_id == operation_id
            && self.support_action_id == preview.support_action_id
            && self.support_action_digest == preview.support_action_digest
            && self.arming_receipt_id == preview.arming_receipt_id
            && self.arming_receipt_digest == preview.expected_arming_receipt_digest
            && self.repository_version == preview.repository_version
            && self.repository_actor == preview.repository_actor
            && self.authorized_transitions == preview.authorized_transitions
            && self.authorized_root_delta_digest == preview.observed_root_delta_digest
            && self.preserved_external_support_changes == preview.disjoint_external_support_changes
            && &self.selective_update_proof_digest == selective_update_proof.proof_digest()
    }

    fn into_receipt_id(self) -> UnicaId {
        self.receipt_id
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum SupportPrerequisiteCompletionModeProof {
    ReservedOriginal {
        manual_actor_lock_inventory_proof: ManualActorLockInventoryProof,
        terminalization_proof: ReservedOriginalTerminalizationProof,
    },
    SeparateWorkingInfobase {
        closure_proof: ManualWorkingInfobaseClosureProof,
    },
}

/// Typed repository/status observation for one approved support prerequisite
/// completion. The approved before state and live authorization are supplied
/// separately and consumed by `support_from_approved`.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct SupportPrerequisiteCompletionObservationAuthority {
    after_anchor: RepositoryAnchor,
    changed_relevant_objects: CanonicalRepositoryTargets,
    changed_unrelated_objects: CanonicalRepositoryTargets,
    applied_structural_changes: RepositoryPlannedChanges,
    original_fingerprint: Sha256Digest,
    update_receipt: RepositoryApplyOperationReceiptAuthority,
    support_prerequisite_receipt: SupportPrerequisiteReceiptAuthority,
    mode_proof: SupportPrerequisiteCompletionModeProof,
    reconciled_history_partition: ValidatedRepositoryHistoryPartition,
    selective_update_proof: SelectiveRepositoryUpdateProof,
    post_release_observed_history_cursor: RepositoryHistoryCursor,
    post_apply_history_partition: ValidatedRepositoryHistoryPartition,
    deferred_repository_advance: Option<DeferredRepositoryAdvance>,
    resulting_phase: TaskPhase,
}

impl SupportPrerequisiteCompletionObservationAuthority {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_repository_adapter(
        after_anchor: RepositoryAnchor,
        changed_relevant_objects: CanonicalRepositoryTargets,
        changed_unrelated_objects: CanonicalRepositoryTargets,
        applied_structural_changes: RepositoryPlannedChanges,
        original_fingerprint: Sha256Digest,
        update_receipt: RepositoryApplyOperationReceiptAuthority,
        support_prerequisite_receipt: SupportPrerequisiteReceiptAuthority,
        mode_proof: SupportPrerequisiteCompletionModeProof,
        reconciled_history_partition: ValidatedRepositoryHistoryPartition,
        selective_update_proof: SelectiveRepositoryUpdateProof,
        post_release_observed_history_cursor: RepositoryHistoryCursor,
        post_apply_history_partition: ValidatedRepositoryHistoryPartition,
        deferred_repository_advance: Option<DeferredRepositoryAdvance>,
        resulting_phase: TaskPhase,
    ) -> Self {
        Self {
            after_anchor,
            changed_relevant_objects,
            changed_unrelated_objects,
            applied_structural_changes,
            original_fingerprint,
            update_receipt,
            support_prerequisite_receipt,
            mode_proof,
            reconciled_history_partition,
            selective_update_proof,
            post_release_observed_history_cursor,
            post_apply_history_partition,
            deferred_repository_advance,
            resulting_phase,
        }
    }
}

fn routine_deferred_consumption_matches(
    terminal_receipt_id: &UnicaId,
    advance_observation_digest: &Sha256Digest,
    update_receipt_id: &UnicaId,
    resolved_history_partition_digest: &Sha256Digest,
    resulting_phase: TaskPhase,
    receipt: &DeferredRepositoryAdvanceConsumptionReceipt,
) -> bool {
    receipt.terminal_receipt_id() == terminal_receipt_id
        && receipt.advance_observation_digest() == advance_observation_digest
        && receipt.routine_update_receipt_id() == update_receipt_id
        && receipt.resolved_history_partition_digest() == resolved_history_partition_digest
        && receipt.resulting_phase() == resulting_phase
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct RepositoryUpdateCompletionAuthority {
    apply_operation_id: OperationId,
    mode: RepositoryUpdateCompletionMode,
    before_anchor: RepositoryAnchor,
    after_anchor: RepositoryAnchor,
    changed_relevant_objects: CanonicalRepositoryTargets,
    changed_unrelated_objects: CanonicalRepositoryTargets,
    applied_structural_changes: RepositoryPlannedChanges,
    original_fingerprint: Sha256Digest,
    update_receipt_id: UnicaId,
    resulting_phase: TaskPhase,
    support_prerequisite_receipt_id: Option<UnicaId>,
    support_root_lock_proof: Option<SupportRootLockProof>,
    manual_actor_lock_inventory_proof: Option<ManualActorLockInventoryProof>,
    reconciled_history_partition: ValidatedRepositoryHistoryPartition,
    selective_update_proof: SelectiveRepositoryUpdateProof,
    post_release_observed_history_cursor: RepositoryHistoryCursor,
    post_apply_history_partition: ValidatedRepositoryHistoryPartition,
    reserved_original_terminalization_proof: Option<ReservedOriginalTerminalizationProof>,
    manual_working_infobase_closure_proof: Option<ManualWorkingInfobaseClosureProof>,
    deferred_repository_advance: Option<DeferredRepositoryAdvance>,
    deferred_advance_consumption_receipt: Option<DeferredRepositoryAdvanceConsumptionReceipt>,
}

impl RepositoryUpdateCompletionAuthority {
    pub(crate) fn routine_from_approved(
        approved: ApprovedRoutineUpdatePreviewAuthority,
        completion: RoutineUpdateCompletionObservationAuthority,
    ) -> Result<Self, RepositoryResultContractError> {
        let ApprovedRoutineUpdatePreviewAuthority {
            apply_operation_id,
            preview,
        } = approved;
        let plan = &preview.record.selective_update_plan;
        let proof = &completion.selective_update_proof;
        if completion.reconciled_history_partition != preview.record.history_partition
            || proof.plan_digest() != plan.plan_digest()
            || proof.planned_targets() != plan.planned_targets()
            || proof.applied_targets() != plan.planned_targets()
            || proof.expected_target_revision_map_digest()
                != plan.expected_target_revision_map_digest()
            || proof.lock_targets() != plan.lock_targets()
            || proof.structural_confirmation_used()
                != (proof.update_performed() && plan.structural_confirmation_required())
            || proof.structural_capability_row_id() != plan.structural_capability_row_id()
            || proof.observed_before_cursor()
                != completion.reconciled_history_partition.through_inclusive()
            || completion.after_anchor.history_cursor() != proof.observed_after_cursor()
            || completion.post_apply_history_partition.start_cursor()
                != proof.observed_before_cursor()
            || completion.post_apply_history_partition.through_inclusive()
                != &completion.post_release_observed_history_cursor
            || completion.original_fingerprint
                != *completion.after_anchor.configuration_fingerprint()
            || !completion
                .update_receipt
                .binds(&apply_operation_id, &completion.selective_update_proof)
        {
            return Err(RepositoryResultContractError(
                "routine completion does not reproduce the approved plan, target map, and cursor chain",
            ));
        }
        if completion.changed_relevant_objects != preview.record.planned_relevant_objects
            || completion.changed_unrelated_objects != preview.record.planned_unrelated_objects
            || completion.applied_structural_changes != preview.record.structural_changes
        {
            return Err(RepositoryResultContractError(
                "routine completion changed-object projections differ from the approved preview",
            ));
        }
        if completion
            .post_apply_history_partition
            .classifications()
            .any(|classification| {
                !matches!(
                    classification,
                    RepositoryHistoryPartitionClassification::UnrelatedRoutine
                        | RepositoryHistoryPartitionClassification::RelevantRoutine
                        | RepositoryHistoryPartitionClassification::ExternalSupport
                )
            })
        {
            return Err(RepositoryResultContractError(
                "routine completion tail contains an inadmissible support classification",
            ));
        }
        let tail_relevant = completion
            .post_apply_history_partition
            .classifications()
            .any(|classification| {
                matches!(
                    classification,
                    RepositoryHistoryPartitionClassification::RelevantRoutine
                        | RepositoryHistoryPartitionClassification::ExternalSupport
                )
            });
        let expected_phase = if preview.record.resulting_phase == TaskPhase::AbandonmentReady {
            TaskPhase::AbandonmentReady
        } else if tail_relevant {
            TaskPhase::LocalVerified
        } else {
            preview.record.resulting_phase
        };
        if completion.resulting_phase != expected_phase {
            return Err(RepositoryResultContractError(
                "routine completion phase does not follow the approved preview and tail evidence",
            ));
        }
        let update_receipt_id = completion.update_receipt.into_receipt_id();
        match (
            preview.deferred_terminal_receipt_id.as_ref(),
            preview.record.deferred_repository_advance.as_ref(),
            completion.deferred_advance_consumption_receipt.as_ref(),
        ) {
            (None, None, None) => {}
            (Some(terminal_receipt_id), Some(deferred), Some(receipt))
                if routine_deferred_consumption_matches(
                    terminal_receipt_id,
                    deferred.observation_digest(),
                    &update_receipt_id,
                    completion.reconciled_history_partition.partition_digest(),
                    completion.resulting_phase,
                    receipt,
                ) => {}
            _ => {
                return Err(RepositoryResultContractError(
                    "routine deferred advance must be consumed exactly when the approved preview is current",
                ));
            }
        }
        Ok(Self {
            apply_operation_id,
            mode: RepositoryUpdateCompletionMode::Routine,
            before_anchor: preview.record.before_anchor,
            after_anchor: completion.after_anchor,
            changed_relevant_objects: completion.changed_relevant_objects,
            changed_unrelated_objects: completion.changed_unrelated_objects,
            applied_structural_changes: completion.applied_structural_changes,
            original_fingerprint: completion.original_fingerprint,
            update_receipt_id,
            resulting_phase: completion.resulting_phase,
            support_prerequisite_receipt_id: None,
            support_root_lock_proof: None,
            manual_actor_lock_inventory_proof: None,
            reconciled_history_partition: completion.reconciled_history_partition,
            selective_update_proof: completion.selective_update_proof,
            post_release_observed_history_cursor: completion.post_release_observed_history_cursor,
            post_apply_history_partition: completion.post_apply_history_partition,
            reserved_original_terminalization_proof: None,
            manual_working_infobase_closure_proof: None,
            deferred_repository_advance: None,
            deferred_advance_consumption_receipt: completion.deferred_advance_consumption_receipt,
        })
    }

    pub(crate) fn support_from_approved(
        approved: ApprovedSupportPrerequisitePreviewAuthority,
        live_authorization: ActiveSupportActionResumeHandle,
        completion: SupportPrerequisiteCompletionObservationAuthority,
        status_cas_resolver: &mut dyn SupportActionTerminalStatusCasResolver,
        root_guard_resolver: &mut dyn SupportRootGuardCompletionResolver,
    ) -> Result<Self, RepositoryResultContractError> {
        let ApprovedSupportPrerequisitePreviewAuthority {
            apply_operation_id,
            preview,
        } = approved;
        if live_authorization
            .support_update_authorization_projection()
            .as_ref()
            != Some(&preview.authorization)
        {
            return Err(RepositoryResultContractError(
                "support completion received another live armed authorization",
            ));
        }
        let common = &preview.record.common;
        let plan = &common.selective_update_plan;
        let proof = &completion.selective_update_proof;
        if completion.reconciled_history_partition != common.history_partition
            || proof.plan_digest() != plan.plan_digest()
            || proof.planned_targets() != plan.planned_targets()
            || proof.applied_targets() != plan.planned_targets()
            || proof.expected_target_revision_map_digest()
                != plan.expected_target_revision_map_digest()
            || proof.lock_targets() != plan.lock_targets()
            || proof.update_performed() != common.update_required
            || proof.structural_confirmation_used()
            || proof.structural_capability_row_id().is_some()
            || proof.observed_before_cursor()
                != completion.reconciled_history_partition.through_inclusive()
            || completion.after_anchor.history_cursor() != proof.observed_after_cursor()
            || completion.post_apply_history_partition.start_cursor()
                != proof.observed_before_cursor()
            || completion.post_apply_history_partition.through_inclusive()
                != &completion.post_release_observed_history_cursor
            || completion.original_fingerprint
                != *completion.after_anchor.configuration_fingerprint()
            || !completion
                .update_receipt
                .binds(&apply_operation_id, &completion.selective_update_proof)
            || !completion.support_prerequisite_receipt.matches(
                &apply_operation_id,
                common,
                &completion.selective_update_proof,
            )
            || !completion.applied_structural_changes.as_slice().is_empty()
        {
            return Err(RepositoryResultContractError(
                "support completion does not reproduce its approved plan, receipt, and cursor chain",
            ));
        }
        let expected_changed_relevant = CanonicalRepositoryTargets::new(
            common
                .update_required
                .then(RepositoryTargetIdentity::configuration_root)
                .into_iter()
                .collect(),
        )?;
        if completion.changed_relevant_objects != expected_changed_relevant
            || !completion.changed_unrelated_objects.as_slice().is_empty()
        {
            return Err(RepositoryResultContractError(
                "support completion changed objects outside the exact approved root effect",
            ));
        }
        if completion
            .post_apply_history_partition
            .classifications()
            .any(|classification| {
                !cancellation_tail_classification_is_admissible(classification, false)
            })
        {
            return Err(RepositoryResultContractError(
                "support completion tail contains a disallowed successor instead of deferring before it",
            ));
        }
        match completion.deferred_repository_advance.as_ref() {
            Some(deferred)
                if deferred.anchor_cursor() == &completion.post_release_observed_history_cursor
                    && deferred.first_observed_version().is_none_or(|successor| {
                        completion
                            .post_apply_history_partition
                            .entry_versions()
                            .all(|included| included != successor)
                    }) => {}
            None => {}
            Some(_) => {
                return Err(RepositoryResultContractError(
                    "support deferred advance does not start after the exact allowed terminal tail",
                ));
            }
        }
        let reconciled_relevant =
            common
                .history_partition
                .classifications()
                .any(|classification| {
                    matches!(
                        classification,
                        RepositoryHistoryPartitionClassification::RelevantRoutine
                            | RepositoryHistoryPartitionClassification::ExternalSupport
                    )
                });
        let tail_relevant = completion
            .post_apply_history_partition
            .classifications()
            .any(|classification| {
                matches!(
                    classification,
                    RepositoryHistoryPartitionClassification::RelevantRoutine
                        | RepositoryHistoryPartitionClassification::ExternalSupport
                )
            });
        let expected_phase = if reconciled_relevant
            || tail_relevant
            || completion.deferred_repository_advance.is_some()
        {
            preview.authorization.relevant_advance_phase()
        } else {
            common.post_reconcile_phase
        };
        if completion.resulting_phase != expected_phase {
            return Err(RepositoryResultContractError(
                "support completion phase differs from its approved and terminal history evidence",
            ));
        }

        let expected_terminalization_proof_digest = match &completion.mode_proof {
            SupportPrerequisiteCompletionModeProof::ReservedOriginal {
                terminalization_proof,
                ..
            } => Some(terminalization_proof.proof_digest().clone()),
            SupportPrerequisiteCompletionModeProof::SeparateWorkingInfobase { .. } => None,
        };
        let (
            mode,
            manual_actor_lock_inventory_proof,
            reserved_original_terminalization_proof,
            manual_working_infobase_closure_proof,
        ) = match (preview.record.mode_binding, completion.mode_proof) {
            (
                PrerequisitePreviewModeBinding::Reserved(preview_mode),
                SupportPrerequisiteCompletionModeProof::ReservedOriginal {
                    manual_actor_lock_inventory_proof,
                    terminalization_proof,
                },
            ) if manual_actor_lock_inventory_proof.username()
                == preview_mode.manual_actor_lock_inventory_proof.username()
                && manual_actor_lock_inventory_proof.baseline_lock_set_digest()
                    == preview_mode
                        .manual_actor_lock_inventory_proof
                        .baseline_lock_set_digest()
                && terminalization_proof.reserved_original_identity_digest()
                    == preview.authorization.reserved_original_identity_digest()
                && preview
                    .authorization
                    .reserved_original_lease_capability_id()
                    == Some(terminalization_proof.exclusive_lease_capability_id())
                && terminalization_proof.expected_repository_fingerprint()
                    == &completion.original_fingerprint =>
            {
                (
                    RepositoryUpdateCompletionMode::SupportPrerequisiteReservedOriginal,
                    Some(manual_actor_lock_inventory_proof),
                    Some(terminalization_proof),
                    None,
                )
            }
            (
                PrerequisitePreviewModeBinding::Separate(preview_mode),
                SupportPrerequisiteCompletionModeProof::SeparateWorkingInfobase { closure_proof },
            ) if closure_proof.working_infobase_identity()
                == &preview_mode.observed_working_infobase_identity
                && closure_proof.plan_digest()
                    == preview_mode
                        .manual_working_infobase_closure_plan
                        .plan_digest()
                && closure_proof.exclusive_lease_capability_id()
                    == preview_mode
                        .manual_working_infobase_closure_plan
                        .exclusive_lease_capability_id() =>
            {
                (
                    RepositoryUpdateCompletionMode::SupportPrerequisiteSeparateWorkingInfobase,
                    None,
                    None,
                    Some(closure_proof),
                )
            }
            _ => {
                return Err(RepositoryResultContractError(
                    "support completion mode proof belongs to another approved authorization",
                ));
            }
        };

        let update_receipt_id = completion.update_receipt.into_receipt_id();
        let support_prerequisite_receipt_id =
            completion.support_prerequisite_receipt.into_receipt_id();
        if update_receipt_id == support_prerequisite_receipt_id {
            return Err(RepositoryResultContractError(
                "support operation receipt and authorization receipt must be distinct",
            ));
        }
        let root_guard = ValidatedSupportRootGuardCompletionAuthority::resume(
            SupportRootGuardCompletionBinding::new(
                preview.request_lineage.cwd.clone(),
                preview.request_lineage.task_id.clone(),
                apply_operation_id.clone(),
                preview.request_lineage.expected_status_digest.clone(),
                common.support_action_id.clone(),
                common.support_action_digest.clone(),
                plan.plan_digest().clone(),
                common.history_partition.partition_digest().clone(),
                common.lock_guard_digest.clone(),
                completion.selective_update_proof.proof_digest().clone(),
                completion.selective_update_proof.guard_receipt_id().clone(),
                SupportAuthorizationOutcome::Consumed,
                expected_terminalization_proof_digest,
            ),
            root_guard_resolver,
        )?;
        let status_cas = ValidatedSupportActionTerminalStatusCasAuthority::acquire(
            SupportActionTerminalStatusCasBinding::new(
                preview.request_lineage.cwd.clone(),
                preview.request_lineage.task_id.clone(),
                apply_operation_id.clone(),
                preview.request_lineage.expected_status_digest.clone(),
                common.support_action_id.clone(),
                common.support_action_digest.clone(),
                SupportActionTerminalOutcome::Consumed,
                support_prerequisite_receipt_id.clone(),
                completion.selective_update_proof.proof_digest().clone(),
                completion.resulting_phase,
                completion
                    .deferred_repository_advance
                    .as_ref()
                    .map(|advance| advance.observation_digest().clone()),
            ),
            status_cas_resolver,
        )
        .map_err(|_| {
            RepositoryResultContractError(
                "support status CAS could not bind the approved apply operation",
            )
        })?;
        let terminal_authorization = live_authorization
            .terminalize_with_status_cas(status_cas)
            .map_err(|_| {
                RepositoryResultContractError(
                    "support completion could not atomically consume the live authorization",
                )
            })?;
        let support_root_lock_proof =
            root_guard.commit_terminal_and_release(&terminal_authorization)?;

        Ok(Self {
            apply_operation_id,
            mode,
            before_anchor: preview.before_anchor,
            after_anchor: completion.after_anchor,
            changed_relevant_objects: completion.changed_relevant_objects,
            changed_unrelated_objects: completion.changed_unrelated_objects,
            applied_structural_changes: completion.applied_structural_changes,
            original_fingerprint: completion.original_fingerprint,
            update_receipt_id,
            resulting_phase: completion.resulting_phase,
            support_prerequisite_receipt_id: Some(support_prerequisite_receipt_id),
            support_root_lock_proof: Some(support_root_lock_proof),
            manual_actor_lock_inventory_proof,
            reconciled_history_partition: completion.reconciled_history_partition,
            selective_update_proof: completion.selective_update_proof,
            post_release_observed_history_cursor: completion.post_release_observed_history_cursor,
            post_apply_history_partition: completion.post_apply_history_partition,
            reserved_original_terminalization_proof,
            manual_working_infobase_closure_proof,
            deferred_repository_advance: completion.deferred_repository_advance,
            deferred_advance_consumption_receipt: None,
        })
    }

    /// Raw completion observations cannot establish equality with an approved
    /// preview.  Keep this fixture-only until the typed effect authority binds
    /// the approved preview, selective proof, receipt and terminal phase.
    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn fixture_test_only(
        apply_operation_id: OperationId,
        mode: RepositoryUpdateCompletionMode,
        before_anchor: RepositoryAnchor,
        after_anchor: RepositoryAnchor,
        changed_relevant_objects: CanonicalRepositoryTargets,
        changed_unrelated_objects: CanonicalRepositoryTargets,
        applied_structural_changes: RepositoryPlannedChanges,
        original_fingerprint: Sha256Digest,
        update_receipt_id: UnicaId,
        resulting_phase: TaskPhase,
        support_prerequisite_receipt_id: Option<UnicaId>,
        support_root_lock_proof: Option<SupportRootLockProof>,
        manual_actor_lock_inventory_proof: Option<ManualActorLockInventoryProof>,
        reconciled_history_partition: ValidatedRepositoryHistoryPartition,
        selective_update_proof: SelectiveRepositoryUpdateProof,
        post_release_observed_history_cursor: RepositoryHistoryCursor,
        post_apply_history_partition: ValidatedRepositoryHistoryPartition,
        reserved_original_terminalization_proof: Option<ReservedOriginalTerminalizationProof>,
        manual_working_infobase_closure_proof: Option<ManualWorkingInfobaseClosureProof>,
        deferred_repository_advance: Option<DeferredRepositoryAdvance>,
        deferred_advance_consumption_receipt: Option<DeferredRepositoryAdvanceConsumptionReceipt>,
    ) -> Result<Self, RepositoryResultContractError> {
        if changed_relevant_objects.as_slice().iter().any(|target| {
            changed_unrelated_objects
                .as_slice()
                .binary_search(target)
                .is_ok()
        }) {
            return Err(RepositoryResultContractError(
                "changed relevant and unrelated object sets must be disjoint",
            ));
        }
        if reconciled_history_partition.through_inclusive()
            != selective_update_proof.observed_before_cursor()
            || post_apply_history_partition.start_cursor()
                != selective_update_proof.observed_before_cursor()
            || post_apply_history_partition.through_inclusive()
                != &post_release_observed_history_cursor
        {
            return Err(RepositoryResultContractError(
                "repository-update partitions do not form the exact selective-update chain",
            ));
        }
        if let Some(deferred) = deferred_repository_advance.as_ref() {
            if deferred.anchor_cursor() != &post_release_observed_history_cursor {
                return Err(RepositoryResultContractError(
                    "deferred repository advance is not anchored at the post-release cursor",
                ));
            }
        }
        match mode {
            RepositoryUpdateCompletionMode::Routine => {
                if support_prerequisite_receipt_id.is_some()
                    || support_root_lock_proof.is_some()
                    || manual_actor_lock_inventory_proof.is_some()
                    || reserved_original_terminalization_proof.is_some()
                    || manual_working_infobase_closure_proof.is_some()
                    || deferred_repository_advance.is_some()
                {
                    return Err(RepositoryResultContractError(
                        "routine update contains support-only or terminal deferred fields",
                    ));
                }
                if let Some(receipt) = deferred_advance_consumption_receipt.as_ref() {
                    if receipt.resulting_phase() != resulting_phase {
                        return Err(RepositoryResultContractError(
                            "deferred consumption receipt has another result phase",
                        ));
                    }
                }
            }
            RepositoryUpdateCompletionMode::SupportPrerequisiteReservedOriginal => {
                if support_prerequisite_receipt_id.is_none()
                    || support_root_lock_proof.is_none()
                    || manual_actor_lock_inventory_proof.is_none()
                    || reserved_original_terminalization_proof.is_none()
                    || manual_working_infobase_closure_proof.is_some()
                    || deferred_advance_consumption_receipt.is_some()
                {
                    return Err(RepositoryResultContractError(
                        "reserved-original support update violates proof presence rules",
                    ));
                }
            }
            RepositoryUpdateCompletionMode::SupportPrerequisiteSeparateWorkingInfobase => {
                if support_prerequisite_receipt_id.is_none()
                    || support_root_lock_proof.is_none()
                    || manual_actor_lock_inventory_proof.is_some()
                    || reserved_original_terminalization_proof.is_some()
                    || manual_working_infobase_closure_proof.is_none()
                    || deferred_advance_consumption_receipt.is_some()
                {
                    return Err(RepositoryResultContractError(
                        "separate-working-infobase support update violates proof presence rules",
                    ));
                }
            }
        }
        Ok(Self {
            apply_operation_id,
            mode,
            before_anchor,
            after_anchor,
            changed_relevant_objects,
            changed_unrelated_objects,
            applied_structural_changes,
            original_fingerprint,
            update_receipt_id,
            resulting_phase,
            support_prerequisite_receipt_id,
            support_root_lock_proof,
            manual_actor_lock_inventory_proof,
            reconciled_history_partition,
            selective_update_proof,
            post_release_observed_history_cursor,
            post_apply_history_partition,
            reserved_original_terminalization_proof,
            manual_working_infobase_closure_proof,
            deferred_repository_advance,
            deferred_advance_consumption_receipt,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RepositoryUpdateDigestRecord {
    before_anchor: RepositoryAnchor,
    after_anchor: RepositoryAnchor,
    changed_relevant_objects: CanonicalRepositoryTargets,
    changed_unrelated_objects: CanonicalRepositoryTargets,
    applied_structural_changes: RepositoryPlannedChanges,
    original_fingerprint: Sha256Digest,
    update_receipt_id: UnicaId,
    resulting_phase: TaskPhase,
    #[serde(skip_serializing_if = "Option::is_none")]
    support_prerequisite_receipt_id: Option<UnicaId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    support_root_lock_proof: Option<SupportRootLockProof>,
    #[serde(skip_serializing_if = "Option::is_none")]
    manual_actor_lock_inventory_proof: Option<ManualActorLockInventoryProof>,
    reconciled_history_partition: ValidatedRepositoryHistoryPartition,
    selective_update_proof: SelectiveRepositoryUpdateProof,
    post_release_observed_history_cursor: RepositoryHistoryCursor,
    post_apply_history_partition: ValidatedRepositoryHistoryPartition,
    #[serde(skip_serializing_if = "Option::is_none")]
    reserved_original_terminalization_proof: Option<ReservedOriginalTerminalizationProof>,
    #[serde(skip_serializing_if = "Option::is_none")]
    manual_working_infobase_closure_proof: Option<ManualWorkingInfobaseClosureProof>,
    #[serde(skip_serializing_if = "Option::is_none")]
    deferred_repository_advance: Option<DeferredRepositoryAdvance>,
    #[serde(skip_serializing_if = "Option::is_none")]
    deferred_advance_consumption_receipt: Option<DeferredRepositoryAdvanceConsumptionReceipt>,
}

impl contract_digest_record_sealed::Sealed for RepositoryUpdateDigestRecord {}
impl ContractDigestRecord for RepositoryUpdateDigestRecord {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RepositoryUpdateData {
    before_anchor: RepositoryAnchor,
    after_anchor: RepositoryAnchor,
    changed_relevant_objects: CanonicalRepositoryTargets,
    changed_unrelated_objects: CanonicalRepositoryTargets,
    applied_structural_changes: RepositoryPlannedChanges,
    original_fingerprint: Sha256Digest,
    update_receipt_id: UnicaId,
    resulting_phase: TaskPhase,
    #[serde(skip_serializing_if = "Option::is_none")]
    support_prerequisite_receipt_id: Option<UnicaId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    support_root_lock_proof: Option<SupportRootLockProof>,
    #[serde(skip_serializing_if = "Option::is_none")]
    manual_actor_lock_inventory_proof: Option<ManualActorLockInventoryProof>,
    reconciled_history_partition: ValidatedRepositoryHistoryPartition,
    selective_update_proof: SelectiveRepositoryUpdateProof,
    post_release_observed_history_cursor: RepositoryHistoryCursor,
    post_apply_history_partition: ValidatedRepositoryHistoryPartition,
    #[serde(skip_serializing_if = "Option::is_none")]
    reserved_original_terminalization_proof: Option<ReservedOriginalTerminalizationProof>,
    #[serde(skip_serializing_if = "Option::is_none")]
    manual_working_infobase_closure_proof: Option<ManualWorkingInfobaseClosureProof>,
    #[serde(skip_serializing_if = "Option::is_none")]
    deferred_repository_advance: Option<DeferredRepositoryAdvance>,
    #[serde(skip_serializing_if = "Option::is_none")]
    deferred_advance_consumption_receipt: Option<DeferredRepositoryAdvanceConsumptionReceipt>,
    update_digest: Sha256Digest,
}

impl RepositoryUpdateData {
    pub(crate) fn from_authority(
        authority: RepositoryUpdateCompletionAuthority,
    ) -> Result<Self, RepositoryResultContractError> {
        let _apply_operation_id = authority.apply_operation_id;
        let _mode = authority.mode;
        let record = RepositoryUpdateDigestRecord {
            before_anchor: authority.before_anchor,
            after_anchor: authority.after_anchor,
            changed_relevant_objects: authority.changed_relevant_objects,
            changed_unrelated_objects: authority.changed_unrelated_objects,
            applied_structural_changes: authority.applied_structural_changes,
            original_fingerprint: authority.original_fingerprint,
            update_receipt_id: authority.update_receipt_id,
            resulting_phase: authority.resulting_phase,
            support_prerequisite_receipt_id: authority.support_prerequisite_receipt_id,
            support_root_lock_proof: authority.support_root_lock_proof,
            manual_actor_lock_inventory_proof: authority.manual_actor_lock_inventory_proof,
            reconciled_history_partition: authority.reconciled_history_partition,
            selective_update_proof: authority.selective_update_proof,
            post_release_observed_history_cursor: authority.post_release_observed_history_cursor,
            post_apply_history_partition: authority.post_apply_history_partition,
            reserved_original_terminalization_proof: authority
                .reserved_original_terminalization_proof,
            manual_working_infobase_closure_proof: authority.manual_working_infobase_closure_proof,
            deferred_repository_advance: authority.deferred_repository_advance,
            deferred_advance_consumption_receipt: authority.deferred_advance_consumption_receipt,
        };
        let update_digest = result_digest(&record, "repository-update digest failed")?;
        Ok(Self {
            before_anchor: record.before_anchor,
            after_anchor: record.after_anchor,
            changed_relevant_objects: record.changed_relevant_objects,
            changed_unrelated_objects: record.changed_unrelated_objects,
            applied_structural_changes: record.applied_structural_changes,
            original_fingerprint: record.original_fingerprint,
            update_receipt_id: record.update_receipt_id,
            resulting_phase: record.resulting_phase,
            support_prerequisite_receipt_id: record.support_prerequisite_receipt_id,
            support_root_lock_proof: record.support_root_lock_proof,
            manual_actor_lock_inventory_proof: record.manual_actor_lock_inventory_proof,
            reconciled_history_partition: record.reconciled_history_partition,
            selective_update_proof: record.selective_update_proof,
            post_release_observed_history_cursor: record.post_release_observed_history_cursor,
            post_apply_history_partition: record.post_apply_history_partition,
            reserved_original_terminalization_proof: record.reserved_original_terminalization_proof,
            manual_working_infobase_closure_proof: record.manual_working_infobase_closure_proof,
            deferred_repository_advance: record.deferred_repository_advance,
            deferred_advance_consumption_receipt: record.deferred_advance_consumption_receipt,
            update_digest,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct RecoveryTerminalObservations(Vec<RecoveryObservation>);

impl RecoveryTerminalObservations {
    pub(crate) fn new(
        values: Vec<RecoveryObservation>,
    ) -> Result<Self, RepositoryResultContractError> {
        let mut digests = BTreeSet::new();
        if values.is_empty()
            || values.len() > MAX_RESULT_ITEMS
            || !values
                .iter()
                .all(|value| digests.insert(value.observation_digest().clone()))
        {
            return Err(RepositoryResultContractError(
                "terminal recovery observations must be non-empty, bounded, and digest-unique",
            ));
        }
        Ok(Self(values))
    }
}

impl JsonSchema for RecoveryTerminalObservations {
    fn schema_name() -> Cow<'static, str> {
        "RecoveryTerminalObservations".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "array",
            "minItems": 1,
            "maxItems": MAX_RESULT_ITEMS,
            "uniqueItems": true,
            "items": generator.subschema_for::<RecoveryObservation>(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct CompletedRecoveryActions(Vec<RecoveryAction>);

impl CompletedRecoveryActions {
    fn from_plan(values: Vec<RecoveryAction>) -> Result<Self, RepositoryResultContractError> {
        if values.is_empty() || values.len() > MAX_RESULT_ITEMS {
            return Err(RepositoryResultContractError(
                "completed recovery action list must be non-empty and bounded",
            ));
        }
        let mut ids = BTreeSet::new();
        if !values
            .iter()
            .all(|action| ids.insert(action.action_id().clone()))
        {
            return Err(RepositoryResultContractError(
                "completed recovery action list contains a duplicate action ID",
            ));
        }
        Ok(Self(values))
    }
}

impl JsonSchema for CompletedRecoveryActions {
    fn schema_name() -> Cow<'static, str> {
        "CompletedRecoveryActions".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "array",
            "minItems": 1,
            "maxItems": MAX_RESULT_ITEMS,
            "uniqueItems": true,
            "items": generator.subschema_for::<RecoveryAction>(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct CompletedRecoveryActionOutcomes(Vec<RecoveryActionOutcome>);

impl CompletedRecoveryActionOutcomes {
    fn from_validated(
        values: Vec<RecoveryActionOutcome>,
        action_count: usize,
    ) -> Result<Self, RepositoryResultContractError> {
        if values.len() != action_count || values.len() > MAX_RESULT_ITEMS {
            return Err(RepositoryResultContractError(
                "completed recovery outcomes are not one-to-one with the action catalog",
            ));
        }
        Ok(Self(values))
    }
}

impl JsonSchema for CompletedRecoveryActionOutcomes {
    fn schema_name() -> Cow<'static, str> {
        "CompletedRecoveryActionOutcomes".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "array",
            "minItems": 1,
            "maxItems": MAX_RESULT_ITEMS,
            "items": generator.subschema_for::<RecoveryActionOutcome>(),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct EmptyRecoveryUnknowns;

impl Serialize for EmptyRecoveryUnknowns {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.collect_seq(std::iter::empty::<&RecoveryUnknown>())
    }
}

impl JsonSchema for EmptyRecoveryUnknowns {
    fn schema_name() -> Cow<'static, str> {
        "EmptyRecoveryUnknowns".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "array",
            "minItems": 0,
            "maxItems": 0,
            "items": generator.subschema_for::<RecoveryUnknown>(),
        })
    }
}

wire_literal!(
    PreArmCancellationRecoveryTarget,
    "preArmSupportCancellation"
);
wire_literal!(ReconcileOnlyRecoveryEffect, "reconcileOnly");

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReservedPreArmRecoveryModeBinding {
    manual_target_mode: ReservedOriginalModeLiteral,
    reserved_original_terminalization_proof: ReservedOriginalTerminalizationProof,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SeparatePreArmRecoveryModeBinding {
    manual_target_mode: SeparateWorkingInfobaseModeLiteral,
    manual_working_infobase_closure_proof: ManualWorkingInfobaseClosureProof,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
enum PreArmRecoveryModeBinding {
    Reserved(ReservedPreArmRecoveryModeBinding),
    Separate(Box<SeparatePreArmRecoveryModeBinding>),
}

impl JsonSchema for PreArmRecoveryModeBinding {
    fn schema_name() -> Cow<'static, str> {
        "PreArmRecoveryModeBinding".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<ReservedPreArmRecoveryModeBinding>(),
            generator.subschema_for::<SeparatePreArmRecoveryModeBinding>(),
        ])
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PreArmCancellationRecoveryCommon {
    target: PreArmCancellationRecoveryTarget,
    effect_class: ReconcileOnlyRecoveryEffect,
    prior_operation_id: OperationId,
    terminal_observations: RecoveryTerminalObservations,
    actions: CompletedRecoveryActions,
    action_outcomes: CompletedRecoveryActionOutcomes,
    resulting_phase: TaskPhase,
    remaining_unknowns: EmptyRecoveryUnknowns,
    approved_recovery_digest: Sha256Digest,
    support_action_id: UnicaId,
    expected_support_action_digest: Sha256Digest,
    approved_cancellation_digest: Sha256Digest,
    arming_receipt_absent: TrueLiteral,
    effect_observation: PreArmCancellationEffectObservation,
    pre_arm_cancellation_finalization_plan: PreArmCancellationFinalizationPlan,
    pre_arm_cancellation_finalization_plan_digest: Sha256Digest,
    pre_arm_cancellation_receipt_plan_digest: Sha256Digest,
    finalization_recheck_evidence: PreArmCancellationFinalizationRecheckEvidence,
    pre_arm_cancellation_completed_progress: PreArmCancellationFinalizationAttemptProgress,
    finalization_attempt_audit_digest: Sha256Digest,
    support_root_lock_proof: SupportRootLockProof,
    selective_update_proof: SelectiveRepositoryUpdateProof,
    support_cancellation_receipt_id: UnicaId,
    support_cancellation_receipt_digest: Sha256Digest,
    pre_arm_recovery_receipt_id: UnicaId,
    pre_arm_recovery_receipt_digest: Sha256Digest,
    reconciled_history_partition: ValidatedRepositoryHistoryPartition,
    post_release_observed_history_cursor: RepositoryHistoryCursor,
    post_apply_history_partition: ValidatedRepositoryHistoryPartition,
    #[serde(skip_serializing_if = "Option::is_none")]
    deferred_repository_advance: Option<DeferredRepositoryAdvance>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PreArmCancellationRecoveryDigestRecord {
    #[serde(flatten)]
    common: PreArmCancellationRecoveryCommon,
    #[serde(flatten)]
    mode_binding: PreArmRecoveryModeBinding,
}

impl contract_digest_record_sealed::Sealed for PreArmCancellationRecoveryDigestRecord {}
impl ContractDigestRecord for PreArmCancellationRecoveryDigestRecord {}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ValidatedPreArmCancellationRecoveryAuthority {
    record: PreArmCancellationRecoveryDigestRecord,
    recovery_receipt_digest: Sha256Digest,
}

impl ValidatedPreArmCancellationRecoveryAuthority {
    pub(crate) fn from_terminal_evidence(
        evidence: ValidatedCompletedPreArmTerminalEvidence,
    ) -> Result<Self, RepositoryResultContractError> {
        let projection = evidence.completed_projection();
        if projection.target != RecoveryTarget::PreArmSupportCancellation
            || projection.effect_class != RecoveryEffectClass::ReconcileOnly
            || !projection.remaining_unknowns.is_empty()
        {
            return Err(RepositoryResultContractError(
                "completed pre-arm evidence has a non-terminal target/effect/unknown branch",
            ));
        }
        let terminal_observations =
            RecoveryTerminalObservations::new(projection.terminal_observations.to_vec())?;
        let actions = CompletedRecoveryActions::from_plan(projection.actions.to_vec())?;
        let action_outcomes = CompletedRecoveryActionOutcomes::from_validated(
            projection.action_outcomes.to_vec(),
            actions.0.len(),
        )?;
        let mode_binding = match (
            projection.manual_target_mode,
            projection.reserved_original_terminalization_proof,
            projection.manual_working_infobase_closure_proof,
        ) {
            (ManualSupportTargetMode::ReservedOriginal, Some(proof), None) => {
                PreArmRecoveryModeBinding::Reserved(ReservedPreArmRecoveryModeBinding {
                    manual_target_mode: ReservedOriginalModeLiteral::Value,
                    reserved_original_terminalization_proof: proof.clone(),
                })
            }
            (ManualSupportTargetMode::SeparateWorkingInfobase, None, Some(proof)) => {
                PreArmRecoveryModeBinding::Separate(Box::new(SeparatePreArmRecoveryModeBinding {
                    manual_target_mode: SeparateWorkingInfobaseModeLiteral::Value,
                    manual_working_infobase_closure_proof: proof.clone(),
                }))
            }
            _ => {
                return Err(RepositoryResultContractError(
                    "completed pre-arm evidence violates mode-proof presence",
                ));
            }
        };
        let record = PreArmCancellationRecoveryDigestRecord {
            common: PreArmCancellationRecoveryCommon {
                target: PreArmCancellationRecoveryTarget::Value,
                effect_class: ReconcileOnlyRecoveryEffect::Value,
                prior_operation_id: projection.prior_operation_id.clone(),
                terminal_observations,
                actions,
                action_outcomes,
                resulting_phase: projection.resulting_phase,
                remaining_unknowns: EmptyRecoveryUnknowns,
                approved_recovery_digest: projection.approved_recovery_digest.clone(),
                support_action_id: projection.support_action_id.clone(),
                expected_support_action_digest: projection.expected_support_action_digest.clone(),
                approved_cancellation_digest: projection.approved_cancellation_digest.clone(),
                arming_receipt_absent: TrueLiteral,
                effect_observation: projection.effect_observation.clone(),
                pre_arm_cancellation_finalization_plan: projection.finalization_plan.clone(),
                pre_arm_cancellation_finalization_plan_digest: projection
                    .finalization_plan_digest
                    .clone(),
                pre_arm_cancellation_receipt_plan_digest: projection.receipt_plan_digest.clone(),
                finalization_recheck_evidence: projection.finalization_recheck_evidence.clone(),
                pre_arm_cancellation_completed_progress: projection.completed_progress.clone(),
                finalization_attempt_audit_digest: projection
                    .finalization_attempt_audit_digest
                    .clone(),
                support_root_lock_proof: projection.support_root_lock_proof.clone(),
                selective_update_proof: projection.selective_update_proof.clone(),
                support_cancellation_receipt_id: projection.support_cancellation_receipt_id.clone(),
                support_cancellation_receipt_digest: projection
                    .support_cancellation_receipt_digest
                    .clone(),
                pre_arm_recovery_receipt_id: projection.pre_arm_recovery_receipt_id.clone(),
                pre_arm_recovery_receipt_digest: projection.pre_arm_recovery_receipt_digest.clone(),
                reconciled_history_partition: projection.reconciled_history_partition.clone(),
                post_release_observed_history_cursor: projection
                    .post_release_observed_history_cursor
                    .clone(),
                post_apply_history_partition: projection.post_apply_history_partition.clone(),
                deferred_repository_advance: projection.deferred_repository_advance.cloned(),
            },
            mode_binding,
        };
        let recovery_receipt_digest =
            result_digest(&record, "pre-arm recovery receipt digest failed")?;
        Ok(Self {
            record,
            recovery_receipt_digest,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct PreArmCancellationRecoveryData {
    #[serde(flatten)]
    common: PreArmCancellationRecoveryCommon,
    #[serde(flatten)]
    mode_binding: PreArmRecoveryModeBinding,
    recovery_receipt_digest: Sha256Digest,
}

impl JsonSchema for PreArmCancellationRecoveryData {
    fn schema_name() -> Cow<'static, str> {
        "PreArmCancellationRecoveryData".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            closed_flattened_object_schema(
                vec![
                    PreArmCancellationRecoveryCommon::json_schema(generator),
                    ReservedPreArmRecoveryModeBinding::json_schema(generator),
                ],
                vec![(
                    "recoveryReceiptDigest",
                    generator.subschema_for::<Sha256Digest>(),
                )],
            ),
            closed_flattened_object_schema(
                vec![
                    PreArmCancellationRecoveryCommon::json_schema(generator),
                    SeparatePreArmRecoveryModeBinding::json_schema(generator),
                ],
                vec![(
                    "recoveryReceiptDigest",
                    generator.subschema_for::<Sha256Digest>(),
                )],
            ),
        ])
    }
}

impl PreArmCancellationRecoveryData {
    pub(crate) fn from_authority(authority: ValidatedPreArmCancellationRecoveryAuthority) -> Self {
        let record = authority.record;
        Self {
            common: record.common,
            mode_binding: record.mode_binding,
            recovery_receipt_digest: authority.recovery_receipt_digest,
        }
    }
}

wire_literal!(SupportPrerequisiteRecoveryTarget, "supportPrerequisite");
wire_literal!(RestoreThenReauthorizeDisposition, "restoreThenReauthorize");
wire_literal!(
    PreserveExternalAndReauthorizeDisposition,
    "preserveExternalAndReauthorize"
);
wire_literal!(RestoreThenAbandonDisposition, "restoreThenAbandon");

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReauthorizeSupportRecoveryDispositionBinding {
    support_recovery_disposition: RestoreThenReauthorizeDisposition,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PreserveSupportRecoveryDispositionBinding {
    support_recovery_disposition: PreserveExternalAndReauthorizeDisposition,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AbandonSupportRecoveryDispositionBinding {
    support_recovery_disposition: RestoreThenAbandonDisposition,
    successful_integration_forbidden: TrueLiteral,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
enum SupportRecoveryDispositionBinding {
    Reauthorize(ReauthorizeSupportRecoveryDispositionBinding),
    Preserve(PreserveSupportRecoveryDispositionBinding),
    Abandon(AbandonSupportRecoveryDispositionBinding),
}

impl JsonSchema for SupportRecoveryDispositionBinding {
    fn schema_name() -> Cow<'static, str> {
        "SupportRecoveryDispositionBinding".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<ReauthorizeSupportRecoveryDispositionBinding>(),
            generator.subschema_for::<PreserveSupportRecoveryDispositionBinding>(),
            generator.subschema_for::<AbandonSupportRecoveryDispositionBinding>(),
        ])
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReservedSupportRecoveryModeBinding {
    manual_target_mode: ReservedOriginalModeLiteral,
    reserved_original_terminalization_proof: ReservedOriginalTerminalizationProof,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SeparateSupportRecoveryModeBinding {
    manual_target_mode: SeparateWorkingInfobaseModeLiteral,
    manual_working_infobase_closure_proof: ManualWorkingInfobaseClosureProof,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
enum SupportRecoveryModeBinding {
    Reserved(ReservedSupportRecoveryModeBinding),
    Separate(Box<SeparateSupportRecoveryModeBinding>),
}

impl JsonSchema for SupportRecoveryModeBinding {
    fn schema_name() -> Cow<'static, str> {
        "SupportRecoveryModeBinding".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<ReservedSupportRecoveryModeBinding>(),
            generator.subschema_for::<SeparateSupportRecoveryModeBinding>(),
        ])
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SupportRecoveryCommon {
    target: SupportPrerequisiteRecoveryTarget,
    effect_class: ReconcileOnlyRecoveryEffect,
    prior_operation_id: OperationId,
    terminal_observations: RecoveryTerminalObservations,
    actions: CompletedRecoveryActions,
    action_outcomes: CompletedRecoveryActionOutcomes,
    resulting_phase: TaskPhase,
    remaining_unknowns: EmptyRecoveryUnknowns,
    approved_recovery_digest: Sha256Digest,
    support_recovery_guard_proof: SupportRecoveryGuardProof,
    #[serde(skip_serializing_if = "Option::is_none")]
    deferred_repository_advance: Option<DeferredRepositoryAdvance>,
    support_recovery_receipt_id: UnicaId,
    support_version_observation_digest: Sha256Digest,
    support_recovery_finalization_plan_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SupportRecoveryDigestRecord {
    #[serde(flatten)]
    common: SupportRecoveryCommon,
    #[serde(flatten)]
    disposition_binding: SupportRecoveryDispositionBinding,
    #[serde(flatten)]
    mode_binding: SupportRecoveryModeBinding,
}

impl contract_digest_record_sealed::Sealed for SupportRecoveryDigestRecord {}
impl ContractDigestRecord for SupportRecoveryDigestRecord {}

/// Terminal support recovery remains deliberately fail-closed in production
/// until the linear support-recovery executor retains the approved plan and
/// terminal action/outcome projection beside its completed guard proof.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ValidatedSupportRecoveryAuthority {
    record: SupportRecoveryDigestRecord,
    recovery_receipt_digest: Sha256Digest,
}

impl ValidatedSupportRecoveryAuthority {
    #[cfg(test)]
    fn fixture_test_only(
        common: SupportRecoveryCommon,
        disposition: crate::domain::branched_development::contracts::support::SupportRecoveryDisposition,
        mode_binding: SupportRecoveryModeBinding,
    ) -> Result<Self, RepositoryResultContractError> {
        if !common.support_recovery_guard_proof.is_completed()
            || common
                .support_recovery_guard_proof
                .finalization_plan_digest()
                != &common.support_recovery_finalization_plan_digest
            || common.support_recovery_guard_proof.manual_target_mode()
                != match mode_binding {
                    SupportRecoveryModeBinding::Reserved(_) => {
                        ManualSupportTargetMode::ReservedOriginal
                    }
                    SupportRecoveryModeBinding::Separate(_) => {
                        ManualSupportTargetMode::SeparateWorkingInfobase
                    }
                }
        {
            return Err(RepositoryResultContractError(
                "support recovery guard/mode/finalization projection mismatch",
            ));
        }
        let disposition_binding = match disposition {
            crate::domain::branched_development::contracts::support::SupportRecoveryDisposition::RestoreThenReauthorize => {
                SupportRecoveryDispositionBinding::Reauthorize(
                    ReauthorizeSupportRecoveryDispositionBinding {
                        support_recovery_disposition: RestoreThenReauthorizeDisposition::Value,
                    },
                )
            }
            crate::domain::branched_development::contracts::support::SupportRecoveryDisposition::PreserveExternalAndReauthorize => {
                SupportRecoveryDispositionBinding::Preserve(
                    PreserveSupportRecoveryDispositionBinding {
                        support_recovery_disposition:
                            PreserveExternalAndReauthorizeDisposition::Value,
                    },
                )
            }
            crate::domain::branched_development::contracts::support::SupportRecoveryDisposition::RestoreThenAbandon => {
                SupportRecoveryDispositionBinding::Abandon(
                    AbandonSupportRecoveryDispositionBinding {
                        support_recovery_disposition: RestoreThenAbandonDisposition::Value,
                        successful_integration_forbidden: TrueLiteral,
                    },
                )
            }
        };
        let record = SupportRecoveryDigestRecord {
            common,
            disposition_binding,
            mode_binding,
        };
        let recovery_receipt_digest =
            result_digest(&record, "support recovery receipt digest failed")?;
        Ok(Self {
            record,
            recovery_receipt_digest,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SupportRecoveryData {
    #[serde(flatten)]
    common: SupportRecoveryCommon,
    #[serde(flatten)]
    disposition_binding: SupportRecoveryDispositionBinding,
    #[serde(flatten)]
    mode_binding: SupportRecoveryModeBinding,
    recovery_receipt_digest: Sha256Digest,
}

impl JsonSchema for SupportRecoveryData {
    fn schema_name() -> Cow<'static, str> {
        "SupportRecoveryData".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        macro_rules! branch {
            ($disposition:ty, $mode:ty) => {
                closed_flattened_object_schema(
                    vec![
                        SupportRecoveryCommon::json_schema(generator),
                        <$disposition>::json_schema(generator),
                        <$mode>::json_schema(generator),
                    ],
                    vec![(
                        "recoveryReceiptDigest",
                        generator.subschema_for::<Sha256Digest>(),
                    )],
                )
            };
        }
        one_of_schema(vec![
            branch!(
                ReauthorizeSupportRecoveryDispositionBinding,
                ReservedSupportRecoveryModeBinding
            ),
            branch!(
                ReauthorizeSupportRecoveryDispositionBinding,
                SeparateSupportRecoveryModeBinding
            ),
            branch!(
                PreserveSupportRecoveryDispositionBinding,
                ReservedSupportRecoveryModeBinding
            ),
            branch!(
                PreserveSupportRecoveryDispositionBinding,
                SeparateSupportRecoveryModeBinding
            ),
            branch!(
                AbandonSupportRecoveryDispositionBinding,
                ReservedSupportRecoveryModeBinding
            ),
            branch!(
                AbandonSupportRecoveryDispositionBinding,
                SeparateSupportRecoveryModeBinding
            ),
        ])
    }
}

impl SupportRecoveryData {
    pub(crate) fn from_authority(authority: ValidatedSupportRecoveryAuthority) -> Self {
        Self {
            common: authority.record.common,
            disposition_binding: authority.record.disposition_binding,
            mode_binding: authority.record.mode_binding,
            recovery_receipt_digest: authority.recovery_receipt_digest,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) enum OrdinaryGeneralRecoveryTarget {
    TaskConfiguration,
    RepositoryLocks,
    OriginalConfiguration,
    RepositoryCommit,
    ManualWorkingInfobaseLease,
    Artifact,
    Archive,
}

wire_literal!(CleanupRecoveryTarget, "cleanup");

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct OrdinaryGeneralRecoveryDigestRecord {
    target: OrdinaryGeneralRecoveryTarget,
    effect_class: RecoveryEffectClass,
    prior_operation_id: OperationId,
    terminal_observations: RecoveryTerminalObservations,
    actions: CompletedRecoveryActions,
    action_outcomes: CompletedRecoveryActionOutcomes,
    resulting_phase: TaskPhase,
    remaining_unknowns: EmptyRecoveryUnknowns,
    approved_recovery_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CleanupGeneralRecoveryDigestRecord {
    target: CleanupRecoveryTarget,
    effect_class: RecoveryEffectClass,
    cleanup_receipt: CleanupReceipt,
    prior_operation_id: OperationId,
    terminal_observations: RecoveryTerminalObservations,
    actions: CompletedRecoveryActions,
    action_outcomes: CompletedRecoveryActionOutcomes,
    resulting_phase: TaskPhase,
    remaining_unknowns: EmptyRecoveryUnknowns,
    approved_recovery_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
enum GeneralRecoveryDigestRecord {
    Ordinary(OrdinaryGeneralRecoveryDigestRecord),
    Cleanup(CleanupGeneralRecoveryDigestRecord),
}

impl contract_digest_record_sealed::Sealed for GeneralRecoveryDigestRecord {}
impl ContractDigestRecord for GeneralRecoveryDigestRecord {}

impl JsonSchema for GeneralRecoveryDigestRecord {
    fn schema_name() -> Cow<'static, str> {
        "GeneralRecoveryDigestRecord".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<OrdinaryGeneralRecoveryDigestRecord>(),
            generator.subschema_for::<CleanupGeneralRecoveryDigestRecord>(),
        ])
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ValidatedGeneralRecoveryAuthority {
    record: GeneralRecoveryDigestRecord,
    recovery_receipt_digest: Sha256Digest,
}

impl ValidatedGeneralRecoveryAuthority {
    /// The core terminal-completion producer must replace this fixture path in
    /// production because only recovery.rs can validate the private matched-
    /// observation/outcome digest union.
    #[cfg(test)]
    fn fixture_test_only(
        status: RecoveryPlanStatus,
        approved_recovery_digest: &Sha256Digest,
        terminal_observations: Vec<RecoveryObservation>,
        action_outcomes: Vec<RecoveryActionOutcome>,
        cleanup_receipt: Option<CleanupReceipt>,
    ) -> Result<Self, RepositoryResultContractError> {
        if status.recovery_digest() != approved_recovery_digest
            || !status.remaining_unknowns().is_empty()
            || matches!(
                status.target(),
                RecoveryTarget::SupportPrerequisite
                    | RecoveryTarget::PreArmSupportCancellation
                    | RecoveryTarget::Cleanup if cleanup_receipt.is_none()
            )
            || status.target() != RecoveryTarget::Cleanup && cleanup_receipt.is_some()
            || status
                .validate_completed_outcomes(&action_outcomes)
                .is_err()
        {
            return Err(RepositoryResultContractError(
                "general recovery completion does not reproduce the approved terminal plan",
            ));
        }
        let observations = RecoveryTerminalObservations::new(terminal_observations)?;
        let actions = CompletedRecoveryActions::from_plan(status.actions().to_vec())?;
        let outcomes =
            CompletedRecoveryActionOutcomes::from_validated(action_outcomes, actions.0.len())?;
        let common = (
            status.effect_class(),
            status.prior_operation_id().clone(),
            observations,
            actions,
            outcomes,
            status.planned_result_phase(),
            status.recovery_digest().clone(),
        );
        let record = match (status.target(), cleanup_receipt) {
            (RecoveryTarget::Cleanup, Some(cleanup_receipt)) => {
                GeneralRecoveryDigestRecord::Cleanup(CleanupGeneralRecoveryDigestRecord {
                    target: CleanupRecoveryTarget::Value,
                    effect_class: common.0,
                    cleanup_receipt,
                    prior_operation_id: common.1,
                    terminal_observations: common.2,
                    actions: common.3,
                    action_outcomes: common.4,
                    resulting_phase: common.5,
                    remaining_unknowns: EmptyRecoveryUnknowns,
                    approved_recovery_digest: common.6,
                })
            }
            (target, None) => {
                let target = match target {
                    RecoveryTarget::TaskConfiguration => {
                        OrdinaryGeneralRecoveryTarget::TaskConfiguration
                    }
                    RecoveryTarget::RepositoryLocks => {
                        OrdinaryGeneralRecoveryTarget::RepositoryLocks
                    }
                    RecoveryTarget::OriginalConfiguration => {
                        OrdinaryGeneralRecoveryTarget::OriginalConfiguration
                    }
                    RecoveryTarget::RepositoryCommit => {
                        OrdinaryGeneralRecoveryTarget::RepositoryCommit
                    }
                    RecoveryTarget::ManualWorkingInfobaseLease => {
                        OrdinaryGeneralRecoveryTarget::ManualWorkingInfobaseLease
                    }
                    RecoveryTarget::Artifact => OrdinaryGeneralRecoveryTarget::Artifact,
                    RecoveryTarget::Archive => OrdinaryGeneralRecoveryTarget::Archive,
                    RecoveryTarget::SupportPrerequisite
                    | RecoveryTarget::PreArmSupportCancellation
                    | RecoveryTarget::Cleanup => {
                        return Err(RepositoryResultContractError(
                            "support/pre-arm/cleanup target escaped the general recovery branch",
                        ));
                    }
                };
                GeneralRecoveryDigestRecord::Ordinary(OrdinaryGeneralRecoveryDigestRecord {
                    target,
                    effect_class: common.0,
                    prior_operation_id: common.1,
                    terminal_observations: common.2,
                    actions: common.3,
                    action_outcomes: common.4,
                    resulting_phase: common.5,
                    remaining_unknowns: EmptyRecoveryUnknowns,
                    approved_recovery_digest: common.6,
                })
            }
            (_, Some(_)) => {
                return Err(RepositoryResultContractError(
                    "cleanup receipt is forbidden outside cleanup recovery",
                ));
            }
        };
        let recovery_receipt_digest =
            result_digest(&record, "general recovery receipt digest failed")?;
        Ok(Self {
            record,
            recovery_receipt_digest,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct OrdinaryGeneralRecoveryData {
    target: OrdinaryGeneralRecoveryTarget,
    effect_class: RecoveryEffectClass,
    prior_operation_id: OperationId,
    terminal_observations: RecoveryTerminalObservations,
    actions: CompletedRecoveryActions,
    action_outcomes: CompletedRecoveryActionOutcomes,
    resulting_phase: TaskPhase,
    remaining_unknowns: EmptyRecoveryUnknowns,
    approved_recovery_digest: Sha256Digest,
    recovery_receipt_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CleanupGeneralRecoveryData {
    target: CleanupRecoveryTarget,
    effect_class: RecoveryEffectClass,
    cleanup_receipt: CleanupReceipt,
    prior_operation_id: OperationId,
    terminal_observations: RecoveryTerminalObservations,
    actions: CompletedRecoveryActions,
    action_outcomes: CompletedRecoveryActionOutcomes,
    resulting_phase: TaskPhase,
    remaining_unknowns: EmptyRecoveryUnknowns,
    approved_recovery_digest: Sha256Digest,
    recovery_receipt_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub(crate) enum GeneralRecoveryData {
    Ordinary(OrdinaryGeneralRecoveryData),
    Cleanup(CleanupGeneralRecoveryData),
}

impl JsonSchema for GeneralRecoveryData {
    fn schema_name() -> Cow<'static, str> {
        "GeneralRecoveryData".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<OrdinaryGeneralRecoveryData>(),
            generator.subschema_for::<CleanupGeneralRecoveryData>(),
        ])
    }
}

impl GeneralRecoveryData {
    pub(crate) fn from_authority(authority: ValidatedGeneralRecoveryAuthority) -> Self {
        match authority.record {
            GeneralRecoveryDigestRecord::Ordinary(record) => {
                Self::Ordinary(OrdinaryGeneralRecoveryData {
                    target: record.target,
                    effect_class: record.effect_class,
                    prior_operation_id: record.prior_operation_id,
                    terminal_observations: record.terminal_observations,
                    actions: record.actions,
                    action_outcomes: record.action_outcomes,
                    resulting_phase: record.resulting_phase,
                    remaining_unknowns: record.remaining_unknowns,
                    approved_recovery_digest: record.approved_recovery_digest,
                    recovery_receipt_digest: authority.recovery_receipt_digest,
                })
            }
            GeneralRecoveryDigestRecord::Cleanup(record) => {
                Self::Cleanup(CleanupGeneralRecoveryData {
                    target: record.target,
                    effect_class: record.effect_class,
                    cleanup_receipt: record.cleanup_receipt,
                    prior_operation_id: record.prior_operation_id,
                    terminal_observations: record.terminal_observations,
                    actions: record.actions,
                    action_outcomes: record.action_outcomes,
                    resulting_phase: record.resulting_phase,
                    remaining_unknowns: record.remaining_unknowns,
                    approved_recovery_digest: record.approved_recovery_digest,
                    recovery_receipt_digest: authority.recovery_receipt_digest,
                })
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub(crate) enum RepositoryRecoveryData {
    General(Box<GeneralRecoveryData>),
    Support(Box<SupportRecoveryData>),
    PreArmCancellation(Box<PreArmCancellationRecoveryData>),
}

impl JsonSchema for RepositoryRecoveryData {
    fn schema_name() -> Cow<'static, str> {
        "RepositoryRecoveryData".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<GeneralRecoveryData>(),
            generator.subschema_for::<SupportRecoveryData>(),
            generator.subschema_for::<PreArmCancellationRecoveryData>(),
        ])
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct RecoveryData(RepositoryRecoveryData);

impl RecoveryData {
    pub(crate) const fn new(data: RepositoryRecoveryData) -> Self {
        Self(data)
    }
}

impl JsonSchema for RecoveryData {
    fn schema_name() -> Cow<'static, str> {
        "RecoveryData".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        RepositoryRecoveryData::json_schema(generator)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RecoveryPlanCancellationDigestRecord {
    approved_recovery_digest: Sha256Digest,
    resulting_phase: TaskPhase,
}

impl contract_digest_record_sealed::Sealed for RecoveryPlanCancellationDigestRecord {}
impl ContractDigestRecord for RecoveryPlanCancellationDigestRecord {}

/// Sealed no-effect pending-plan cancellation proof. There is intentionally no
/// production raw constructor; the recovery core must mint it only for the
/// cancellable abandonment-preview state.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct PendingRecoveryPlanCancellationAuthority {
    approved_recovery_digest: Sha256Digest,
    resulting_phase: TaskPhase,
}

impl PendingRecoveryPlanCancellationAuthority {
    #[cfg(test)]
    pub(crate) fn test_only(
        approved_recovery_digest: Sha256Digest,
        resulting_phase: TaskPhase,
    ) -> Self {
        Self {
            approved_recovery_digest,
            resulting_phase,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RecoveryPlanCancellationData {
    approved_recovery_digest: Sha256Digest,
    cancellation_receipt_digest: Sha256Digest,
    resulting_phase: TaskPhase,
}

impl RecoveryPlanCancellationData {
    pub(crate) fn from_authority(
        authority: PendingRecoveryPlanCancellationAuthority,
    ) -> Result<Self, RepositoryResultContractError> {
        let record = RecoveryPlanCancellationDigestRecord {
            approved_recovery_digest: authority.approved_recovery_digest,
            resulting_phase: authority.resulting_phase,
        };
        let cancellation_receipt_digest =
            result_digest(&record, "recovery-plan cancellation receipt digest failed")?;
        Ok(Self {
            approved_recovery_digest: record.approved_recovery_digest,
            cancellation_receipt_digest,
            resulting_phase: record.resulting_phase,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::branched_development::contracts::schema::audit_json_schema;
    use schemars::schema_for;
    use serde::de::DeserializeOwned;
    use serde_json::{json, Value};

    const OBJECT_A: &str = "00000000-0000-0000-0000-000000000001";
    const OBJECT_B: &str = "00000000-0000-0000-0000-000000000002";

    fn digest(character: char) -> Sha256Digest {
        Sha256Digest::parse(&character.to_string().repeat(64)).unwrap()
    }

    fn object(value: &str) -> MetadataObjectId {
        MetadataObjectId::parse(value).unwrap()
    }

    fn root_leaf() -> RootTargetIdentity {
        RootTargetIdentity::new()
    }

    fn object_leaf(value: &str) -> ObjectTargetIdentity {
        ObjectTargetIdentity::new(object(value))
    }

    fn display(value: &str) -> RepositoryTargetDisplay {
        RepositoryTargetDisplay::parse(value).unwrap()
    }

    fn reasons(values: Vec<RepositoryIntegrationReason>) -> RepositoryIntegrationReasons {
        RepositoryIntegrationReasons::new(values).unwrap()
    }

    fn targets(values: Vec<RepositoryTargetIdentity>) -> CanonicalRepositoryTargets {
        CanonicalRepositoryTargets::new(values).unwrap()
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

    fn id(value: &str) -> UnicaId {
        UnicaId::parse(value).unwrap()
    }

    fn operation_id(value: &str) -> OperationId {
        OperationId::parse(value).unwrap()
    }

    fn support_completion_anchor(
        cursor: RepositoryHistoryCursor,
        fingerprint: Sha256Digest,
    ) -> RepositoryAnchor {
        use crate::domain::branched_development::contracts::artifacts::ConfigurationIdentity;
        use crate::domain::branched_development::contracts::repository::RepositoryAnchorObservationAuthority;
        use crate::domain::branched_development::contracts::scalars::{EmptyOrName, Name};

        RepositoryAnchorObservationAuthority::test_only(
            digest('1'),
            cursor,
            ConfigurationIdentity::new(
                object("123e4567-e89b-12d3-a456-426614174000"),
                Name::parse("Support completion fixture").unwrap(),
                EmptyOrName::parse("").unwrap(),
                EmptyOrName::parse("").unwrap(),
            ),
            fingerprint,
        )
        .into_anchor()
        .unwrap()
    }

    #[derive(Default)]
    struct OneShotStatusCasResolver {
        acquired: bool,
    }

    struct TestStatusCasLease {
        binding: SupportActionTerminalStatusCasBinding,
        accepted: bool,
    }

    impl
        crate::domain::branched_development::contracts::support::SupportActionTerminalStatusCasLease
        for TestStatusCasLease
    {
        fn binds(&self, binding: &SupportActionTerminalStatusCasBinding) -> bool {
            self.accepted && &self.binding == binding
        }

        fn commit_terminal(
            self: Box<Self>,
            _terminal: &TerminalSupportActionAuthorization,
        ) -> Result<(), crate::domain::branched_development::contracts::support::SupportContractError>
        {
            Ok(())
        }
    }

    impl SupportActionTerminalStatusCasResolver for OneShotStatusCasResolver {
        fn acquire(
            &mut self,
            binding: &SupportActionTerminalStatusCasBinding,
        ) -> Result<
            Box<
                dyn crate::domain::branched_development::contracts::support::SupportActionTerminalStatusCasLease,
            >,
            crate::domain::branched_development::contracts::support::SupportContractError,
        >{
            let accepted = !self.acquired;
            self.acquired = true;
            Ok(Box::new(TestStatusCasLease {
                binding: binding.clone(),
                accepted,
            }))
        }
    }

    #[derive(Default)]
    struct TestRootGuardResolver {
        foreign_release_proof: bool,
    }

    struct TestRootGuardLease {
        binding: SupportRootGuardCompletionBinding,
        declared_release_receipt_id: UnicaId,
        proof_release_receipt_id: UnicaId,
    }

    impl SupportRootGuardCompletionLease for TestRootGuardLease {
        fn binds(&self, binding: &SupportRootGuardCompletionBinding) -> bool {
            &self.binding == binding
        }

        fn root_guard_release_receipt_id(&self) -> &UnicaId {
            &self.declared_release_receipt_id
        }

        fn commit_terminal_and_release(
            self: Box<Self>,
            _terminal: &TerminalSupportActionAuthorization,
        ) -> Result<SupportRootLockProof, RepositoryResultContractError> {
            SupportRootLockProof::new(
                self.binding.guard_receipt_id().clone(),
                self.proof_release_receipt_id,
                self.binding.authorization_outcome(),
                self.binding.terminalization_proof_digest().cloned(),
            )
            .map_err(|_| RepositoryResultContractError("test root-guard proof failed"))
        }
    }

    impl SupportRootGuardCompletionResolver for TestRootGuardResolver {
        fn resume(
            &mut self,
            binding: &SupportRootGuardCompletionBinding,
        ) -> Result<Box<dyn SupportRootGuardCompletionLease>, RepositoryResultContractError>
        {
            let declared_release_receipt_id = id("66666666-6666-4666-8666-666666666666");
            let proof_release_receipt_id = if self.foreign_release_proof {
                id("77777777-7777-4777-8777-777777777777")
            } else {
                declared_release_receipt_id.clone()
            };
            Ok(Box::new(TestRootGuardLease {
                binding: binding.clone(),
                declared_release_receipt_id,
                proof_release_receipt_id,
            }))
        }
    }

    struct SupportCompletionFixture {
        approved: ApprovedSupportPrerequisitePreviewAuthority,
        live_authorization: ActiveSupportActionResumeHandle,
        completion: SupportPrerequisiteCompletionObservationAuthority,
    }

    fn support_completion_fixture(mode: ManualSupportTargetMode) -> SupportCompletionFixture {
        use crate::domain::branched_development::contracts::repository::update::support_root_already_exact_fixture_test_only;
        use crate::domain::branched_development::contracts::repository::RepositoryAnchorObservationAuthority;
        use crate::domain::branched_development::contracts::support::armed_support_action_resume_handle_fixture_test_only;
        use crate::domain::branched_development::contracts::support_terminalization::{
            ManualWorkingInfobaseClosureExecutionAuthority,
            ManualWorkingInfobaseClosurePlanAuthority,
        };

        let live_authorization = armed_support_action_resume_handle_fixture_test_only(mode);
        let authorization = live_authorization
            .support_update_authorization_projection()
            .expect("armed fixture projects support-update authorization");
        let arming_receipt = authorization.arming_receipt().unwrap().clone();
        let endpoint = arming_receipt.arming_cursor().clone();
        let fingerprint = authorization.expected_original_fingerprint().clone();
        let before_anchor = support_completion_anchor(endpoint.clone(), fingerprint.clone());
        let lock_targets: RepositoryUpdateLockTargets = serde_json::from_value(json!([{
            "targetKind": "configurationRoot",
            "objectDisplay": "Configuration",
            "reasons": ["supportGraphGuard"]
        }]))
        .unwrap();
        let (selective_update_plan, selective_update_proof) =
            support_root_already_exact_fixture_test_only(
                lock_targets,
                CapabilityRowId::parse("support-root-selective-update.v1").unwrap(),
                id("55555555-5555-4555-8555-555555555555"),
                fingerprint.clone(),
                fingerprint.clone(),
                endpoint.clone(),
                endpoint.clone(),
            )
            .unwrap();
        let repository_actor: RepositoryActorIdentity = serde_json::from_value(json!({
            "username": authorization.manual_actor_username(),
            "computer": authorization
                .manual_working_infobase_identity()
                .map(ManualWorkingInfobaseIdentity::computer),
            "infobase": authorization
                .manual_working_infobase_identity()
                .map(ManualWorkingInfobaseIdentity::infobase),
        }))
        .unwrap();
        let history_partition = arming_receipt.history_partition().clone();
        let empty_observations =
            CanonicalSupportVersionObservations::from_validated_history_order(Vec::new()).unwrap();
        let common = SupportPrerequisitePreviewCommon {
            mode: SupportPrerequisiteModeLiteral::Value,
            purpose: authorization.purpose(),
            origin_phase: authorization.origin_phase(),
            post_reconcile_phase: authorization.post_reconcile_phase(),
            support_gate_id: authorization.support_gate_id().clone(),
            support_action_id: authorization.support_action_id().clone(),
            support_action_digest: authorization.support_action_digest().clone(),
            arming_receipt_id: arming_receipt.arming_receipt_id().clone(),
            expected_arming_receipt_digest: arming_receipt.receipt_digest().clone(),
            arming_cursor: endpoint.clone(),
            support_gate_digest: authorization.support_gate_digest().clone(),
            repository_version: RepositoryVersion::parse("support-v1").unwrap(),
            repository_actor: repository_actor.clone(),
            authorized_transitions: authorization.authorized_transitions().clone(),
            expected_original_fingerprint: fingerprint.clone(),
            observed_original_fingerprint: fingerprint.clone(),
            observed_root_delta_digest: digest('2'),
            history_partition: history_partition.clone(),
            selective_update_plan,
            concurrent_routine_changes: empty_observations.clone(),
            disjoint_external_support_changes: empty_observations,
            lock_guard_digest: digest('3'),
            update_required: false,
        };

        let (mode_binding, completion_mode_proof) = match mode {
            ManualSupportTargetMode::ReservedOriginal => {
                let inventory = ManualActorLockInventoryProof::new(
                    authorization.manual_actor_username().clone(),
                    authorization
                        .manual_actor_lock_baseline_digest()
                        .unwrap()
                        .clone(),
                    authorization
                        .manual_actor_lock_baseline_digest()
                        .unwrap()
                        .clone(),
                )
                .unwrap();
                let terminalization = ReservedOriginalTerminalizationProof::new(
                    authorization.reserved_original_identity_digest().clone(),
                    authorization
                        .reserved_original_lease_capability_id()
                        .unwrap()
                        .clone(),
                    id("88888888-8888-4888-8888-888888888888"),
                    id("99999999-9999-4999-8999-999999999999"),
                    fingerprint.clone(),
                    fingerprint.clone(),
                )
                .unwrap();
                (
                    PrerequisitePreviewModeBinding::Reserved(ReservedPrerequisitePreviewBinding {
                        manual_target_mode: ReservedOriginalModeLiteral::Value,
                        reserved_original_lease_capability_id: authorization
                            .reserved_original_lease_capability_id()
                            .unwrap()
                            .clone(),
                        manual_actor_lock_inventory_proof: inventory.clone(),
                    }),
                    SupportPrerequisiteCompletionModeProof::ReservedOriginal {
                        manual_actor_lock_inventory_proof: inventory,
                        terminalization_proof: terminalization,
                    },
                )
            }
            ManualSupportTargetMode::SeparateWorkingInfobase => {
                let identity = authorization
                    .manual_working_infobase_identity()
                    .unwrap()
                    .clone();
                let baseline = authorization.manual_working_infobase_baseline().unwrap();
                let closure_plan = ManualWorkingInfobaseClosurePlan::new(
                    ManualWorkingInfobaseClosurePlanAuthority::materialized_test_only(
                        identity.clone(),
                        baseline.baseline_digest().clone(),
                        baseline.current_fingerprint().clone(),
                        baseline.recorded_object_version_map_digest().clone(),
                        baseline.support_graph_digest().clone(),
                        baseline.repository_base_cursor().clone(),
                        baseline.recorded_object_version_map_digest().clone(),
                        baseline.exclusive_lease_capability_id().clone(),
                    ),
                )
                .unwrap();
                let closure_proof = ManualWorkingInfobaseClosureProof::new(
                    &closure_plan,
                    ManualWorkingInfobaseClosureExecutionAuthority::matching_test_only(
                        &closure_plan,
                        id("88888888-8888-4888-8888-888888888888"),
                        id("99999999-9999-4999-8999-999999999999"),
                    )
                    .unwrap(),
                )
                .unwrap();
                (
                    PrerequisitePreviewModeBinding::Separate(Box::new(
                        SeparatePrerequisitePreviewBinding {
                            manual_target_mode: SeparateWorkingInfobaseModeLiteral::Value,
                            observed_working_infobase_identity: identity,
                            manual_working_infobase_closure_plan: closure_plan,
                        },
                    )),
                    SupportPrerequisiteCompletionModeProof::SeparateWorkingInfobase {
                        closure_proof,
                    },
                )
            }
        };
        let record = SupportPrerequisitePreviewDigestRecord {
            common: common.clone(),
            mode_binding,
            root_lock_binding: RootLockPreviewBinding::ApplyGuardOnly(
                ApplyGuardRootLockPreviewBinding {
                    root_lock_observation_mode: ApplyGuardOnlyModeLiteral::Value,
                },
            ),
        };
        let update_digest =
            result_digest(&record, "support completion fixture preview digest failed").unwrap();
        let apply_operation_id = operation_id("22222222-2222-4222-8222-222222222222");
        let approved = ApprovedSupportPrerequisitePreviewAuthority {
            apply_operation_id: apply_operation_id.clone(),
            preview: SupportPrerequisitePreviewAuthority {
                request_lineage: PrerequisiteUpdateRequestLineage {
                    cwd: serde_json::from_value(json!("/original/project")).unwrap(),
                    task_id: serde_json::from_value(json!("TASK-173")).unwrap(),
                    preview_operation_id: operation_id("11111111-1111-4111-8111-111111111111"),
                    expected_status_digest: digest('4'),
                },
                authorization,
                before_anchor: before_anchor.clone(),
                record,
                update_digest,
            },
        };
        let update_receipt = RepositoryApplyOperationReceiptAuthority::from_capability_adapter(
            apply_operation_id.clone(),
            id("33333333-3333-4333-8333-333333333333"),
            &selective_update_proof,
        );
        let support_prerequisite_receipt =
            SupportPrerequisiteReceiptAuthority::from_capability_adapter(
                apply_operation_id,
                id("44444444-4444-4444-8444-444444444444"),
                common.support_action_id.clone(),
                common.support_action_digest.clone(),
                common.arming_receipt_id.clone(),
                common.expected_arming_receipt_digest.clone(),
                common.repository_version.clone(),
                common.repository_actor.clone(),
                common.authorized_transitions.clone(),
                common.observed_root_delta_digest.clone(),
                Vec::new(),
                &selective_update_proof,
            )
            .unwrap();
        let after_anchor = RepositoryAnchorObservationAuthority::test_only(
            before_anchor.repository_identity().clone(),
            endpoint.clone(),
            before_anchor.configuration_identity().clone(),
            fingerprint.clone(),
        )
        .into_anchor()
        .unwrap();
        let completion = SupportPrerequisiteCompletionObservationAuthority::from_repository_adapter(
            after_anchor,
            CanonicalRepositoryTargets::new(Vec::new()).unwrap(),
            CanonicalRepositoryTargets::new(Vec::new()).unwrap(),
            serde_json::from_value(json!([])).unwrap(),
            fingerprint,
            update_receipt,
            support_prerequisite_receipt,
            completion_mode_proof,
            history_partition.clone(),
            selective_update_proof,
            endpoint,
            history_partition,
            None,
            common.post_reconcile_phase,
        );
        SupportCompletionFixture {
            approved,
            live_authorization,
            completion,
        }
    }

    struct CancellationCompletionFixture {
        approved: ApprovedSupportPrerequisiteCancellationPreviewAuthority,
        live_authorization: ActiveSupportActionResumeHandle,
        completion: SupportActionCancellationCompletionObservation,
    }

    fn cancellation_completion_fixture(
        mode: ManualSupportTargetMode,
    ) -> CancellationCompletionFixture {
        let SupportCompletionFixture {
            approved,
            live_authorization,
            completion,
        } = support_completion_fixture(mode);
        let ApprovedSupportPrerequisitePreviewAuthority {
            apply_operation_id,
            preview,
        } = approved;
        let SupportPrerequisitePreviewAuthority {
            request_lineage,
            authorization,
            before_anchor,
            record: support_record,
            update_digest: _,
        } = preview;
        let SupportPrerequisiteCompletionObservationAuthority {
            after_anchor,
            changed_relevant_objects,
            changed_unrelated_objects,
            applied_structural_changes,
            original_fingerprint: _,
            update_receipt,
            support_prerequisite_receipt: _,
            mode_proof,
            reconciled_history_partition,
            selective_update_proof,
            post_release_observed_history_cursor,
            post_apply_history_partition,
            deferred_repository_advance,
            resulting_phase: _,
        } = completion;
        let cancellation_mode_binding = match support_record.mode_binding {
            PrerequisitePreviewModeBinding::Reserved(binding) => {
                CancellationModeBinding::Reserved(ReservedCancellationPreviewBinding {
                    reserved_original_lease_capability_id: binding
                        .reserved_original_lease_capability_id,
                    manual_actor_lock_inventory_proof: binding.manual_actor_lock_inventory_proof,
                })
            }
            PrerequisitePreviewModeBinding::Separate(binding) => {
                let binding = *binding;
                CancellationModeBinding::Separate(SeparateCancellationPreviewBinding {
                    manual_working_infobase_closure_plan: binding
                        .manual_working_infobase_closure_plan,
                })
            }
        };
        let cancellation_mode_proof = match mode_proof {
            SupportPrerequisiteCompletionModeProof::ReservedOriginal {
                manual_actor_lock_inventory_proof,
                terminalization_proof,
            } => CancellationCompletionModeProof::ReservedOriginal {
                manual_actor_lock_inventory_proof,
                terminalization_proof,
            },
            SupportPrerequisiteCompletionModeProof::SeparateWorkingInfobase { closure_proof } => {
                CancellationCompletionModeProof::SeparateWorkingInfobase { closure_proof }
            }
        };
        let arming_receipt = authorization.arming_receipt().unwrap().clone();
        let empty =
            CanonicalSupportVersionObservations::from_validated_history_order(Vec::new()).unwrap();
        let cancellation_common = SupportPrerequisiteCancellationPreviewCommon {
            mode: SupportCancellationModeLiteral::Value,
            purpose: authorization.purpose(),
            origin_phase: authorization.origin_phase(),
            cancelled_phase: authorization.cancelled_phase(),
            relevant_advance_phase: authorization.relevant_advance_phase(),
            support_action_id: authorization.support_action_id().clone(),
            support_action_digest: authorization.support_action_digest().clone(),
            prior_support_gate_id: authorization.support_gate_id().clone(),
            reason: SupportCancellationReason::OperatorCancelled,
            before_anchor: before_anchor.clone(),
            observed_repository_versions: CanonicalRepositoryVersions::from_validated_partition(
                &support_record.common.history_partition,
            )
            .unwrap(),
            history_partition: support_record.common.history_partition,
            selective_update_plan: support_record.common.selective_update_plan,
            partitioned_routine_changes: empty.clone(),
            relevant_routine_changes: empty.clone(),
            disjoint_external_support_changes: empty.clone(),
            pre_arm_external_changes: empty,
            expected_original_fingerprint: authorization.expected_original_fingerprint().clone(),
            observed_original_fingerprint: authorization.expected_original_fingerprint().clone(),
            expected_support_graph_digest: authorization.expected_support_graph_digest().clone(),
            observed_support_graph_digest: authorization.expected_support_graph_digest().clone(),
            lock_guard_digest: support_record.common.lock_guard_digest,
            update_required: false,
            planned_result_phase: authorization.cancelled_phase(),
        };
        let cancellation_record = SupportPrerequisiteCancellationPreviewDigestRecord {
            common: cancellation_common.clone(),
            arming_binding: CancellationArmingBinding::Armed(ArmedCancellationPreviewBinding {
                arming_receipt_id: arming_receipt.arming_receipt_id().clone(),
                expected_arming_receipt_digest: arming_receipt.receipt_digest().clone(),
                arming_cursor: arming_receipt.arming_cursor().clone(),
            }),
            mode_binding: cancellation_mode_binding,
            root_lock_binding: support_record.root_lock_binding,
        };
        let cancellation_digest = result_digest(
            &cancellation_record,
            "cancellation completion fixture digest failed",
        )
        .unwrap();
        let approved = ApprovedSupportPrerequisiteCancellationPreviewAuthority {
            apply_operation_id,
            preview: SupportPrerequisiteCancellationPreviewAuthority {
                cwd: request_lineage.cwd,
                task_id: request_lineage.task_id,
                preview_operation_id: request_lineage.preview_operation_id,
                expected_status_digest: request_lineage.expected_status_digest,
                reserved_original_identity_digest: authorization
                    .reserved_original_identity_digest()
                    .clone(),
                authorization,
                record: cancellation_record,
                cancellation_digest,
            },
        };
        let completion = SupportActionCancellationCompletionObservation::from_repository_adapter(
            update_receipt,
            cancellation_mode_proof,
            before_anchor,
            after_anchor,
            changed_relevant_objects,
            changed_unrelated_objects,
            applied_structural_changes,
            reconciled_history_partition,
            selective_update_proof,
            post_release_observed_history_cursor,
            post_apply_history_partition,
            deferred_repository_advance,
            cancellation_common.planned_result_phase,
        );
        CancellationCompletionFixture {
            approved,
            live_authorization,
            completion,
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
                impl<T: DeserializeOwned> AmbiguousIfDeserialize<ImplementsDeserialize> for T {}
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
                impl<T: Clone> AmbiguousIfClone<ImplementsClone> for T {}
                let _ = <$type as AmbiguousIfClone<_>>::assert_not_clone;
            };
        };
    }

    assert_not_deserialize_owned!(RepositoryIntegrationEntry);
    assert_not_deserialize_owned!(CommitExactObject);
    assert_not_deserialize_owned!(CommittedRepositoryObject);
    assert_not_deserialize_owned!(LockPlanData);
    assert_not_deserialize_owned!(LockResultData);
    assert_not_deserialize_owned!(ValidatedOriginalMergeLockProjection);
    assert_not_deserialize_owned!(CommitPreviewData);
    assert_not_deserialize_owned!(CommitData);
    assert_not_deserialize_owned!(RepositoryStatusData);
    assert_not_deserialize_owned!(RepositoryUpdatePreviewData);
    assert_not_deserialize_owned!(RepositoryUpdateData);
    assert_not_deserialize_owned!(SupportPrerequisitePreviewData);
    assert_not_deserialize_owned!(SupportPrerequisiteCancellationPreviewData);
    assert_not_deserialize_owned!(SupportActionCancellationData);
    assert_not_deserialize_owned!(CancellationCompletionModeProof);
    assert_not_deserialize_owned!(SupportActionCancellationCompletionObservation);
    assert_not_deserialize_owned!(ValidatedCancellationPreviewObservationAuthority);
    assert_not_deserialize_owned!(RepositoryApplyOperationReceiptAuthority);
    assert_not_deserialize_owned!(SupportPrerequisiteReceiptAuthority);
    assert_not_deserialize_owned!(SupportPrerequisiteCompletionObservationAuthority);
    assert_not_deserialize_owned!(ValidatedSupportRootGuardCompletionAuthority);
    assert_not_deserialize_owned!(ValidatedSupportActionTerminalStatusCasAuthority);
    assert_not_deserialize_owned!(PreArmCancellationRecoveryData);
    assert_not_deserialize_owned!(SupportRecoveryData);
    assert_not_deserialize_owned!(GeneralRecoveryData);
    assert_not_deserialize_owned!(RecoveryData);
    assert_not_deserialize_owned!(UnlockData);
    assert_not_clone!(LockPlanAuthority);
    assert_not_clone!(AtomicRepositoryLockPlanCapabilityAuthority);
    assert_not_clone!(FrozenCommitCommentPolicyAuthority);
    assert_not_clone!(ValidatedCommitCommentPolicyAuthority);
    assert_not_clone!(CommitCommentPolicyRevalidationBlockedAuthority);
    assert_not_clone!(PostMergeCommitGuardAuthority);
    assert_not_clone!(PostMergeCommitGuardBlockedAuthority);
    assert_not_clone!(CommitPreviewBlockedAuthority);
    assert_not_clone!(ValidatedMainSandboxVerificationAuthority);
    assert_not_clone!(ValidatedMainIntegrationVerificationAuthority);
    assert_not_clone!(ValidatedOriginalMergeLockProjection);
    assert_not_clone!(RepositoryUpdatePreviewAuthority);
    assert_not_clone!(ApprovedRoutineUpdatePreviewAuthority);
    assert_not_clone!(ValidatedSupportPrerequisitePreviewObservationAuthority);
    assert_not_clone!(SupportPrerequisitePreviewAuthority);
    assert_not_clone!(ApprovedSupportPrerequisitePreviewAuthority);
    assert_not_clone!(CommitPreviewAuthority);
    assert_not_clone!(ApprovedCommitPreviewAuthority);
    assert_not_clone!(ValidatedCommitObjectAuthority);
    assert_not_clone!(SupportPrerequisiteCancellationPreviewAuthority);
    assert_not_clone!(ValidatedCancellationPreviewObservationAuthority);
    assert_not_clone!(ApprovedSupportPrerequisiteCancellationPreviewAuthority);
    assert_not_clone!(ValidatedSupportActionCancellationAuthority);
    assert_not_clone!(CancellationCompletionModeProof);
    assert_not_clone!(SupportActionCancellationCompletionObservation);
    assert_not_clone!(RepositoryApplyOperationReceiptAuthority);
    assert_not_clone!(SupportPrerequisiteReceiptAuthority);
    assert_not_clone!(SupportPrerequisiteCompletionObservationAuthority);
    assert_not_clone!(ValidatedSupportRootGuardCompletionAuthority);
    assert_not_clone!(ValidatedSupportActionTerminalStatusCasAuthority);
    assert_not_clone!(ValidatedPreArmCancellationRecoveryAuthority);
    const _: for<'a> fn(
        ValidatedCancellationPreviewRequest<'a>,
        SupportUpdateAuthorizationProjection,
        ValidatedCancellationPreviewObservationAuthority,
    ) -> Result<
        SupportPrerequisiteCancellationPreviewAuthority,
        RepositoryResultContractError,
    > = SupportPrerequisiteCancellationPreviewAuthority::from_authorities;

    #[test]
    fn routine_update_apply_context_requires_exact_hidden_lineage_and_distinct_operation() {
        use crate::domain::branched_development::contracts::requests::repository::RepositoryUpdateRequest;

        let preview_request: RepositoryUpdateRequest = serde_json::from_value(json!({
            "cwd": "/original/project",
            "taskId": "TASK-173",
            "operationId": "123e4567-e89b-12d3-a456-426614174000",
            "mode": "routine",
            "expectedStatusDigest": digest('a'),
        }))
        .unwrap();
        let preview = preview_request
            .validate_routine_update_preview_context()
            .unwrap();
        let preview = RoutineUpdateRequestLineage::from_preview(&preview);
        let exact_apply: RepositoryUpdateRequest = serde_json::from_value(json!({
            "cwd": "/original/project",
            "taskId": "TASK-173",
            "operationId": "223e4567-e89b-12d3-a456-426614174000",
            "mode": "routine",
            "expectedStatusDigest": digest('a'),
            "dryRun": false,
            "approvedUpdateDigest": digest('b'),
        }))
        .unwrap();
        let exact = exact_apply
            .validate_routine_update_approval(&digest('b'))
            .unwrap();
        assert!(routine_update_apply_context_matches(
            &preview,
            &digest('b'),
            &exact,
        ));

        let same_operation: RepositoryUpdateRequest = serde_json::from_value(json!({
            "cwd": "/original/project",
            "taskId": "TASK-173",
            "operationId": "123e4567-e89b-12d3-a456-426614174000",
            "mode": "routine",
            "expectedStatusDigest": digest('a'),
            "dryRun": false,
            "approvedUpdateDigest": digest('b'),
        }))
        .unwrap();
        let same_operation = same_operation
            .validate_routine_update_approval(&digest('b'))
            .unwrap();
        assert!(!routine_update_apply_context_matches(
            &preview,
            &digest('b'),
            &same_operation,
        ));

        let wrong_status: RepositoryUpdateRequest = serde_json::from_value(json!({
            "cwd": "/original/project",
            "taskId": "TASK-173",
            "operationId": "323e4567-e89b-12d3-a456-426614174000",
            "mode": "routine",
            "expectedStatusDigest": digest('c'),
            "dryRun": false,
            "approvedUpdateDigest": digest('b'),
        }))
        .unwrap();
        let wrong_status = wrong_status
            .validate_routine_update_approval(&digest('b'))
            .unwrap();
        assert!(!routine_update_apply_context_matches(
            &preview,
            &digest('b'),
            &wrong_status,
        ));
    }

    #[test]
    fn prerequisite_preview_request_requires_the_exact_live_armed_authorization() {
        use crate::domain::branched_development::contracts::requests::repository::RepositoryUpdateRequest;
        use crate::domain::branched_development::contracts::support::support_update_authorization_projection_fixture_test_only;

        let armed = support_update_authorization_projection_fixture_test_only(
            true,
            ManualSupportTargetMode::ReservedOriginal,
        );
        let receipt = armed.arming_receipt().unwrap();
        let request: RepositoryUpdateRequest = serde_json::from_value(json!({
            "cwd": "/original/project",
            "taskId": "TASK-173",
            "operationId": "123e4567-e89b-12d3-a456-426614174000",
            "mode": "supportPrerequisite",
            "expectedStatusDigest": digest('a'),
            "supportActionId": armed.support_action_id(),
            "expectedSupportActionDigest": armed.support_action_digest(),
            "expectedArmingReceiptId": receipt.arming_receipt_id(),
            "expectedArmingReceiptDigest": receipt.receipt_digest(),
        }))
        .unwrap();
        let request = request
            .validate_prerequisite_update_preview_context()
            .unwrap();
        assert!(prerequisite_preview_request_authorization_matches(
            &request, &armed,
        ));

        let awaiting = support_update_authorization_projection_fixture_test_only(
            false,
            ManualSupportTargetMode::ReservedOriginal,
        );
        assert!(!prerequisite_preview_request_authorization_matches(
            &request, &awaiting,
        ));
    }

    #[test]
    fn routine_deferred_consumption_rejects_a_foreign_terminal_receipt() {
        let expected_terminal = UnicaId::parse("11111111-1111-4111-8111-111111111111").unwrap();
        let foreign_terminal = UnicaId::parse("22222222-2222-4222-8222-222222222222").unwrap();
        let update_receipt = UnicaId::parse("33333333-3333-4333-8333-333333333333").unwrap();
        let receipt = DeferredRepositoryAdvanceConsumptionReceipt::new(
            UnicaId::parse("44444444-4444-4444-8444-444444444444").unwrap(),
            foreign_terminal,
            digest('a'),
            update_receipt.clone(),
            digest('b'),
            TaskPhase::Synchronized,
        )
        .unwrap();
        assert!(!routine_deferred_consumption_matches(
            &expected_terminal,
            &digest('a'),
            &update_receipt,
            &digest('b'),
            TaskPhase::Synchronized,
            &receipt,
        ));
    }

    #[test]
    fn repository_apply_receipt_rejects_a_foreign_apply_operation() {
        let expected_operation =
            OperationId::parse("11111111-1111-4111-8111-111111111111").unwrap();
        let foreign_operation = OperationId::parse("22222222-2222-4222-8222-222222222222").unwrap();
        let receipt = RepositoryApplyOperationReceiptAuthority::fixture_test_only(
            foreign_operation,
            UnicaId::parse("33333333-3333-4333-8333-333333333333").unwrap(),
            digest('a'),
        );

        assert!(!receipt.binds_operation_and_proof_digest(&expected_operation, &digest('a'),));
    }

    #[test]
    fn support_completion_consumes_live_authorization_in_both_manual_modes() {
        for (manual_mode, completion_mode) in [
            (
                ManualSupportTargetMode::ReservedOriginal,
                RepositoryUpdateCompletionMode::SupportPrerequisiteReservedOriginal,
            ),
            (
                ManualSupportTargetMode::SeparateWorkingInfobase,
                RepositoryUpdateCompletionMode::SupportPrerequisiteSeparateWorkingInfobase,
            ),
        ] {
            let fixture = support_completion_fixture(manual_mode);
            let mut status_cas = OneShotStatusCasResolver::default();
            let mut root_guard = TestRootGuardResolver::default();
            let completed = RepositoryUpdateCompletionAuthority::support_from_approved(
                fixture.approved,
                fixture.live_authorization,
                fixture.completion,
                &mut status_cas,
                &mut root_guard,
            )
            .unwrap();
            assert_eq!(completed.mode, completion_mode);
            assert_eq!(
                completed
                    .support_root_lock_proof
                    .as_ref()
                    .unwrap()
                    .authorization_outcome(),
                SupportAuthorizationOutcome::Consumed,
            );
            assert!(status_cas.acquired);
        }
    }

    #[test]
    fn support_completion_rejects_cross_authority_operation_receipt_and_proof_splices() {
        let approved = support_completion_fixture(ManualSupportTargetMode::ReservedOriginal);
        let foreign = support_completion_fixture(ManualSupportTargetMode::SeparateWorkingInfobase);
        let mut status_cas = OneShotStatusCasResolver::default();
        let mut root_guard = TestRootGuardResolver::default();
        assert!(RepositoryUpdateCompletionAuthority::support_from_approved(
            approved.approved,
            foreign.live_authorization,
            approved.completion,
            &mut status_cas,
            &mut root_guard,
        )
        .is_err());
        assert!(!status_cas.acquired);

        let mut foreign_operation =
            support_completion_fixture(ManualSupportTargetMode::ReservedOriginal);
        foreign_operation.completion.update_receipt.operation_id =
            operation_id("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa");
        let mut status_cas = OneShotStatusCasResolver::default();
        let mut root_guard = TestRootGuardResolver::default();
        assert!(RepositoryUpdateCompletionAuthority::support_from_approved(
            foreign_operation.approved,
            foreign_operation.live_authorization,
            foreign_operation.completion,
            &mut status_cas,
            &mut root_guard,
        )
        .is_err());
        assert!(!status_cas.acquired);

        let mut duplicate_receipt =
            support_completion_fixture(ManualSupportTargetMode::ReservedOriginal);
        duplicate_receipt
            .completion
            .support_prerequisite_receipt
            .receipt_id = id("33333333-3333-4333-8333-333333333333");
        let mut status_cas = OneShotStatusCasResolver::default();
        let mut root_guard = TestRootGuardResolver::default();
        assert!(RepositoryUpdateCompletionAuthority::support_from_approved(
            duplicate_receipt.approved,
            duplicate_receipt.live_authorization,
            duplicate_receipt.completion,
            &mut status_cas,
            &mut root_guard,
        )
        .is_err());
        assert!(!status_cas.acquired);

        let mut foreign_proof =
            support_completion_fixture(ManualSupportTargetMode::ReservedOriginal);
        let observed_proof_digest = foreign_proof
            .completion
            .selective_update_proof
            .proof_digest()
            .clone();
        foreign_proof
            .completion
            .support_prerequisite_receipt
            .selective_update_proof_digest = if observed_proof_digest == digest('f') {
            digest('e')
        } else {
            digest('f')
        };
        let mut status_cas = OneShotStatusCasResolver::default();
        let mut root_guard = TestRootGuardResolver::default();
        assert!(RepositoryUpdateCompletionAuthority::support_from_approved(
            foreign_proof.approved,
            foreign_proof.live_authorization,
            foreign_proof.completion,
            &mut status_cas,
            &mut root_guard,
        )
        .is_err());
        assert!(!status_cas.acquired);
    }

    #[test]
    fn support_completion_rejects_cross_window_root_release_proof_and_cas_replay() {
        let fixture = support_completion_fixture(ManualSupportTargetMode::ReservedOriginal);
        let mut status_cas = OneShotStatusCasResolver::default();
        let mut root_guard = TestRootGuardResolver {
            foreign_release_proof: true,
        };
        assert!(RepositoryUpdateCompletionAuthority::support_from_approved(
            fixture.approved,
            fixture.live_authorization,
            fixture.completion,
            &mut status_cas,
            &mut root_guard,
        )
        .is_err());
        assert!(status_cas.acquired);

        let first = support_completion_fixture(ManualSupportTargetMode::ReservedOriginal);
        let mut status_cas = OneShotStatusCasResolver::default();
        let mut root_guard = TestRootGuardResolver::default();
        RepositoryUpdateCompletionAuthority::support_from_approved(
            first.approved,
            first.live_authorization,
            first.completion,
            &mut status_cas,
            &mut root_guard,
        )
        .unwrap();

        let replay = support_completion_fixture(ManualSupportTargetMode::ReservedOriginal);
        let mut replay_root_guard = TestRootGuardResolver::default();
        assert!(RepositoryUpdateCompletionAuthority::support_from_approved(
            replay.approved,
            replay.live_authorization,
            replay.completion,
            &mut status_cas,
            &mut replay_root_guard,
        )
        .is_err());
    }

    #[test]
    fn cancellation_completion_terminalizes_the_live_handle_in_both_manual_modes() {
        for manual_mode in [
            ManualSupportTargetMode::ReservedOriginal,
            ManualSupportTargetMode::SeparateWorkingInfobase,
        ] {
            let fixture = cancellation_completion_fixture(manual_mode);
            let mut status_cas = OneShotStatusCasResolver::default();
            let mut root_guard = TestRootGuardResolver::default();
            let completed = ValidatedSupportActionCancellationAuthority::validate(
                fixture.approved,
                fixture.live_authorization,
                fixture.completion,
                &mut status_cas,
                &mut root_guard,
            )
            .unwrap();
            assert_eq!(
                completed
                    .common
                    .support_root_lock_proof
                    .authorization_outcome(),
                SupportAuthorizationOutcome::Cancelled,
            );
            assert!(matches!(
                (&completed.mode_binding, manual_mode),
                (
                    CancellationResultModeBinding::Reserved(_),
                    ManualSupportTargetMode::ReservedOriginal,
                ) | (
                    CancellationResultModeBinding::Separate(_),
                    ManualSupportTargetMode::SeparateWorkingInfobase,
                )
            ));
            assert!(status_cas.acquired);
        }
    }

    #[test]
    fn cancellation_completion_rejects_foreign_live_authority_and_cas_replay() {
        let approved = cancellation_completion_fixture(ManualSupportTargetMode::ReservedOriginal);
        let foreign =
            cancellation_completion_fixture(ManualSupportTargetMode::SeparateWorkingInfobase);
        let mut status_cas = OneShotStatusCasResolver::default();
        let mut root_guard = TestRootGuardResolver::default();
        assert!(ValidatedSupportActionCancellationAuthority::validate(
            approved.approved,
            foreign.live_authorization,
            approved.completion,
            &mut status_cas,
            &mut root_guard,
        )
        .is_err());
        assert!(!status_cas.acquired);

        let mut foreign_terminalization =
            cancellation_completion_fixture(ManualSupportTargetMode::ReservedOriginal);
        let observed_fingerprint = foreign_terminalization
            .completion
            .after_anchor
            .configuration_fingerprint()
            .clone();
        let foreign_fingerprint = if observed_fingerprint == digest('e') {
            digest('f')
        } else {
            digest('e')
        };
        let (reserved_identity, lease_capability) =
            match &foreign_terminalization.completion.mode_proof {
                CancellationCompletionModeProof::ReservedOriginal {
                    terminalization_proof,
                    ..
                } => (
                    terminalization_proof
                        .reserved_original_identity_digest()
                        .clone(),
                    terminalization_proof
                        .exclusive_lease_capability_id()
                        .clone(),
                ),
                CancellationCompletionModeProof::SeparateWorkingInfobase { .. } => {
                    unreachable!()
                }
            };
        let CancellationCompletionModeProof::ReservedOriginal {
            terminalization_proof,
            ..
        } = &mut foreign_terminalization.completion.mode_proof
        else {
            unreachable!()
        };
        *terminalization_proof = ReservedOriginalTerminalizationProof::new(
            reserved_identity,
            lease_capability,
            id("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"),
            id("bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb"),
            foreign_fingerprint.clone(),
            foreign_fingerprint,
        )
        .unwrap();
        let mut status_cas = OneShotStatusCasResolver::default();
        let mut root_guard = TestRootGuardResolver::default();
        assert!(ValidatedSupportActionCancellationAuthority::validate(
            foreign_terminalization.approved,
            foreign_terminalization.live_authorization,
            foreign_terminalization.completion,
            &mut status_cas,
            &mut root_guard,
        )
        .is_err());
        assert!(!status_cas.acquired);

        let first = cancellation_completion_fixture(ManualSupportTargetMode::ReservedOriginal);
        let mut status_cas = OneShotStatusCasResolver::default();
        let mut root_guard = TestRootGuardResolver::default();
        ValidatedSupportActionCancellationAuthority::validate(
            first.approved,
            first.live_authorization,
            first.completion,
            &mut status_cas,
            &mut root_guard,
        )
        .unwrap();

        let replay = cancellation_completion_fixture(ManualSupportTargetMode::ReservedOriginal);
        let mut replay_root_guard = TestRootGuardResolver::default();
        assert!(ValidatedSupportActionCancellationAuthority::validate(
            replay.approved,
            replay.live_authorization,
            replay.completion,
            &mut status_cas,
            &mut replay_root_guard,
        )
        .is_err());
    }

    #[test]
    fn cancellation_tail_requires_deferred_stop_before_disallowed_successors() {
        for classification in [
            RepositoryHistoryPartitionClassification::AuthorizedSupport,
            RepositoryHistoryPartitionClassification::Invalid,
            RepositoryHistoryPartitionClassification::Corrective,
            RepositoryHistoryPartitionClassification::NonConflictingConcurrent,
            RepositoryHistoryPartitionClassification::TaskCommit,
        ] {
            assert!(!cancellation_tail_classification_is_admissible(
                classification,
                true,
            ));
            assert!(!cancellation_tail_classification_is_admissible(
                classification,
                false,
            ));
        }
        assert!(cancellation_tail_classification_is_admissible(
            RepositoryHistoryPartitionClassification::PreArmExternal,
            true,
        ));
        assert!(!cancellation_tail_classification_is_admissible(
            RepositoryHistoryPartitionClassification::PreArmExternal,
            false,
        ));
    }

    #[test]
    fn original_merge_lock_projection_retains_the_complete_owned_lock_plan() {
        let committed = validated_commit_object_authority_fixture_test_only(
            RepositoryVersion::parse("101").unwrap(),
            CapabilityRowId::parse("repository.atomic-commit.projection").unwrap(),
        );
        let plan = committed.approved_preview.0.plan;
        let expected_integration_set_id = plan.integration_set_id.clone();
        let expected_entries = plan.integration_entries.clone();
        let projection = ValidatedOriginalMergeLockProjection::test_only(
            plan,
            UnicaId::parse("77777777-7777-4777-8777-777777777777").unwrap(),
            digest('9'),
        );

        assert_eq!(
            projection.integration_set_id(),
            &expected_integration_set_id
        );
        assert_eq!(projection.plan().integration_entries(), &expected_entries);
        let (owned_plan, _, _) = projection.into_parts();
        assert_eq!(owned_plan.integration_entries(), &expected_entries);
    }

    #[test]
    fn repository_result_named_leaf_schemas_are_closed_and_cross_leaf_splices_fail() {
        for contract in [
            schema::<RepositoryIntegrationEntry>(),
            schema::<CommitExactObject>(),
            schema::<CommittedRepositoryObject>(),
            schema::<CommitCommentPolicyDigestRecord>(),
            schema::<IntegrationSetLineageDigestRecord>(),
        ] {
            audit_json_schema(&contract).unwrap();
        }

        let integration_values = [
            json!({
                "target": {"targetKind": "configurationRoot"},
                "objectDisplay": "Configuration root",
                "action": "modify",
                "reasons": ["canonicalDelta"],
                "requiredLockTargets": [{"targetKind": "configurationRoot"}],
            }),
            json!({
                "target": {"targetKind": "developmentObject", "objectId": OBJECT_A},
                "objectDisplay": "Catalog.Products",
                "action": "add",
                "reasons": ["addDeleteSemantics"],
                "requiredLockTargets": [{"targetKind": "configurationRoot"}],
            }),
            json!({
                "target": {"targetKind": "developmentObject", "objectId": OBJECT_A},
                "objectDisplay": "Catalog.Products",
                "action": "modify",
                "reasons": ["canonicalDelta", "referenceClosure"],
                "requiredLockTargets": [
                    {"targetKind": "configurationRoot"},
                    {"targetKind": "developmentObject", "objectId": OBJECT_A}
                ],
            }),
            json!({
                "target": {"targetKind": "developmentObject", "objectId": OBJECT_A},
                "objectDisplay": "Catalog.Products",
                "action": "delete",
                "reasons": ["addDeleteSemantics"],
                "requiredLockTargets": [{"targetKind": "configurationRoot"}],
            }),
        ];
        for value in &integration_values {
            assert!(schema_accepts::<RepositoryIntegrationEntry>(value));
        }
        let mut cross_target_action = integration_values[0].clone();
        cross_target_action["action"] = json!("add");
        assert!(!schema_accepts::<RepositoryIntegrationEntry>(
            &cross_target_action
        ));

        let exact_values = [
            json!({"target": {"targetKind": "configurationRoot"}, "action": "modify"}),
            json!({"target": {"targetKind": "developmentObject", "objectId": OBJECT_A}, "action": "add"}),
            json!({"target": {"targetKind": "developmentObject", "objectId": OBJECT_A}, "action": "modify"}),
            json!({"target": {"targetKind": "developmentObject", "objectId": OBJECT_A}, "action": "delete"}),
        ];
        for value in exact_values {
            assert!(schema_accepts::<CommitExactObject>(&value));
            for forbidden in ["objectDisplay", "reasons", "requiredLockTargets"] {
                let mut leaked = value.clone();
                leaked[forbidden] = json!([]);
                assert!(!schema_accepts::<CommitExactObject>(&leaked));
            }
        }

        let committed_values = [
            json!({
                "targetKind": "configurationRoot",
                "action": "modify",
                "repositoryVersion": "101",
                "targetFingerprint": digest('a'),
            }),
            json!({
                "targetKind": "developmentObject",
                "objectId": OBJECT_A,
                "action": "add",
                "repositoryVersion": "101",
                "targetFingerprint": digest('b'),
            }),
            json!({
                "targetKind": "developmentObject",
                "objectId": OBJECT_A,
                "action": "delete",
                "absenceEstablishedAtVersion": "101",
                "expectedAbsent": true,
            }),
        ];
        for value in &committed_values {
            assert!(schema_accepts::<CommittedRepositoryObject>(value));
        }
        let mut absent_with_fingerprint = committed_values[2].clone();
        absent_with_fingerprint["targetFingerprint"] = json!(digest('c'));
        assert!(!schema_accepts::<CommittedRepositoryObject>(
            &absent_with_fingerprint
        ));
        let mut root_with_object = committed_values[0].clone();
        root_with_object["objectId"] = json!(OBJECT_A);
        assert!(!schema_accepts::<CommittedRepositoryObject>(
            &root_with_object
        ));
    }

    #[test]
    fn repository_result_canonical_collections_reject_empty_duplicate_and_reordered_values() {
        assert!(RepositoryIntegrationReasons::new(Vec::new()).is_err());
        assert!(RepositoryIntegrationReasons::new(vec![
            RepositoryIntegrationReason::CanonicalDelta,
            RepositoryIntegrationReason::CanonicalDelta,
        ])
        .is_err());
        assert!(RepositoryIntegrationReasons::new(vec![
            RepositoryIntegrationReason::ReferenceClosure,
            RepositoryIntegrationReason::OwnershipClosure,
        ])
        .is_err());

        let root = RepositoryIntegrationEntry::root_modify(
            root_leaf(),
            display("Root"),
            reasons(vec![RepositoryIntegrationReason::CanonicalDelta]),
            targets(vec![RepositoryTargetIdentity::configuration_root()]),
        );
        let object_entry = RepositoryIntegrationEntry::object_modify(
            object_leaf(OBJECT_A),
            display("Object A"),
            reasons(vec![RepositoryIntegrationReason::CanonicalDelta]),
            targets(vec![
                RepositoryTargetIdentity::configuration_root(),
                RepositoryTargetIdentity::development_object(object(OBJECT_A)),
            ]),
        );
        assert!(RepositoryIntegrationEntries::new(Vec::new()).is_err());
        assert!(
            RepositoryIntegrationEntries::new(vec![object_entry.clone(), root.clone()]).is_err()
        );
        assert!(
            RepositoryIntegrationEntries::new(vec![object_entry.clone(), object_entry]).is_err()
        );
        RepositoryIntegrationEntries::new(vec![root]).unwrap();

        assert!(CommittedRepositoryObjects::new(Vec::new()).is_err());
        let first = CommittedRepositoryObject::object_present(
            object(OBJECT_A),
            PresentObjectAction::Modify,
            RepositoryVersion::parse("101").unwrap(),
            digest('a'),
        );
        let second = CommittedRepositoryObject::object_absent(
            object(OBJECT_B),
            RepositoryVersion::parse("101").unwrap(),
        );
        CommittedRepositoryObjects::new(vec![first.clone(), second.clone()]).unwrap();
        assert!(CommittedRepositoryObjects::new(vec![second, first.clone()]).is_err());
        assert!(CommittedRepositoryObjects::new(vec![first.clone(), first]).is_err());
    }

    #[test]
    fn repository_result_exact_commit_projection_cannot_absorb_display_reason_or_lock_fields() {
        let target = object_leaf(OBJECT_A);
        let first = RepositoryIntegrationEntry::object_modify(
            target.clone(),
            display("First presentation"),
            reasons(vec![RepositoryIntegrationReason::CanonicalDelta]),
            targets(vec![
                RepositoryTargetIdentity::configuration_root(),
                RepositoryTargetIdentity::development_object(object(OBJECT_A)),
            ]),
        );
        let second = RepositoryIntegrationEntry::object_modify(
            target,
            display("Changed presentation"),
            reasons(vec![
                RepositoryIntegrationReason::CanonicalDelta,
                RepositoryIntegrationReason::ReferenceClosure,
            ]),
            targets(vec![RepositoryTargetIdentity::development_object(object(
                OBJECT_A,
            ))]),
        );
        let first_exact = CommitExactObjects::new(vec![first.exact_projection()]).unwrap();
        let second_exact = CommitExactObjects::new(vec![second.exact_projection()]).unwrap();
        assert_eq!(first_exact, second_exact);
        assert_eq!(
            result_digest(
                &CommitExactObjectsDigestRecord(first_exact),
                "test exact digest failed"
            )
            .unwrap(),
            result_digest(
                &CommitExactObjectsDigestRecord(second_exact),
                "test exact digest failed"
            )
            .unwrap()
        );
    }

    #[test]
    fn repository_result_delete_self_lock_follows_capability_without_removing_commit_entry() {
        let target = RepositoryTargetIdentity::development_object(object(OBJECT_A));
        let delete_without_self =
            RepositoryIntegrationEntries::new(vec![RepositoryIntegrationEntry::object_delete(
                object_leaf(OBJECT_A),
                display("Deleted object"),
                reasons(vec![RepositoryIntegrationReason::AddDeleteSemantics]),
                targets(vec![RepositoryTargetIdentity::configuration_root()]),
            )])
            .unwrap();
        let root_locks = serde_json::from_value(json!([{
            "targetKind": "configurationRoot",
            "objectDisplay": "Root",
            "reasons": ["supportGraphGuard"]
        }]))
        .unwrap();
        let not_lockable = DeleteSelfLockCapabilityEvidence::from_capability_adapter(
            target.clone(),
            false,
            CapabilityRowId::parse("repository.delete-self.v1").unwrap(),
        )
        .unwrap();
        validate_integration_lock_closure(&delete_without_self, &root_locks, &[not_lockable])
            .unwrap();
        assert!(matches!(
            delete_without_self.as_slice(),
            [RepositoryIntegrationEntry::ObjectDelete(_)]
        ));

        let delete_with_self =
            RepositoryIntegrationEntries::new(vec![RepositoryIntegrationEntry::object_delete(
                object_leaf(OBJECT_A),
                display("Deleted object"),
                reasons(vec![RepositoryIntegrationReason::AddDeleteSemantics]),
                targets(vec![
                    RepositoryTargetIdentity::configuration_root(),
                    target.clone(),
                ]),
            )])
            .unwrap();
        let root_and_self = serde_json::from_value(json!([
            {
                "targetKind": "configurationRoot",
                "objectDisplay": "Root",
                "reasons": ["supportGraphGuard"]
            },
            {
                "targetKind": "developmentObject",
                "objectId": OBJECT_A,
                "objectDisplay": "Deleted object",
                "reasons": ["updateTarget"]
            }
        ]))
        .unwrap();
        let lockable = DeleteSelfLockCapabilityEvidence::from_capability_adapter(
            target.clone(),
            true,
            CapabilityRowId::parse("repository.delete-self.v1").unwrap(),
        )
        .unwrap();
        validate_integration_lock_closure(&delete_with_self, &root_and_self, &[lockable]).unwrap();

        let mismatched = DeleteSelfLockCapabilityEvidence::from_capability_adapter(
            target,
            false,
            CapabilityRowId::parse("repository.delete-self.v1").unwrap(),
        )
        .unwrap();
        assert!(validate_integration_lock_closure(
            &delete_with_self,
            &root_and_self,
            &[mismatched]
        )
        .is_err());
    }

    #[test]
    fn repository_lock_plan_rejects_symmetric_extra_closure_target() {
        let topology = RepositoryIntegrationTopologyBatchAuthority::derive(vec![
            RepositoryIntegrationTopologyObservation::object_modify(
                object_leaf(OBJECT_A),
                display("Modified object"),
                vec![],
            ),
        ])
        .unwrap();
        let locks = serde_json::from_value(json!([
            {
                "targetKind": "configurationRoot",
                "objectDisplay": "Root",
                "reasons": ["supportGraphGuard"]
            },
            {
                "targetKind": "developmentObject",
                "objectId": OBJECT_A,
                "objectDisplay": "Modified object",
                "reasons": ["updateTarget"]
            },
            {
                "targetKind": "developmentObject",
                "objectId": OBJECT_B,
                "objectDisplay": "Unrelated object",
                "reasons": ["referenceClosure"]
            }
        ]))
        .unwrap();

        assert!(topology.validate_lock_entries(&locks).is_err());
    }

    #[test]
    fn repository_lock_plan_rejects_configuration_root_as_changed_referrer() {
        assert!(RepositoryIntegrationTopologyBatchAuthority::derive(vec![
            RepositoryIntegrationTopologyObservation::object_modify(
                object_leaf(OBJECT_A),
                display("Modified object"),
                vec![RepositoryTargetIdentity::configuration_root()],
            ),
        ])
        .is_err());
    }

    #[test]
    fn repository_lock_plan_derives_all_topology_leaves_and_ignores_display() {
        const OBJECT_C: &str = "00000000-0000-0000-0000-000000000003";
        const OBJECT_D: &str = "00000000-0000-0000-0000-000000000004";
        const OBJECT_E: &str = "00000000-0000-0000-0000-000000000005";
        const OBJECT_F: &str = "00000000-0000-0000-0000-000000000006";
        const OBJECT_G: &str = "00000000-0000-0000-0000-000000000007";
        const OBJECT_H: &str = "00000000-0000-0000-0000-000000000008";

        let root = RepositoryTargetIdentity::configuration_root();
        let target_a = RepositoryTargetIdentity::development_object(object(OBJECT_A));
        let target_b = RepositoryTargetIdentity::development_object(object(OBJECT_B));
        let target_c = RepositoryTargetIdentity::development_object(object(OBJECT_C));
        let target_d = RepositoryTargetIdentity::development_object(object(OBJECT_D));
        let target_e = RepositoryTargetIdentity::development_object(object(OBJECT_E));
        let target_f = RepositoryTargetIdentity::development_object(object(OBJECT_F));
        let target_g = RepositoryTargetIdentity::development_object(object(OBJECT_G));
        let target_h = RepositoryTargetIdentity::development_object(object(OBJECT_H));
        let topology = RepositoryIntegrationTopologyBatchAuthority::derive(vec![
            RepositoryIntegrationTopologyObservation::root_modify(display("Root")),
            RepositoryIntegrationTopologyObservation::top_level_add(
                object_leaf(OBJECT_A),
                display("Top-level add"),
            ),
            RepositoryIntegrationTopologyObservation::subordinate_add(
                object_leaf(OBJECT_B),
                display("Subordinate add"),
                object_leaf(OBJECT_D),
            ),
            RepositoryIntegrationTopologyObservation::object_modify(
                object_leaf(OBJECT_C),
                display("Modify"),
                vec![target_d.clone()],
            ),
            RepositoryIntegrationTopologyObservation::owned_child_modify(
                object_leaf(OBJECT_E),
                display("Owned child modify"),
                vec![],
            ),
            RepositoryIntegrationTopologyObservation::object_delete(
                object_leaf(OBJECT_F),
                display("Delete"),
                target_d.clone(),
                vec![target_g.clone()],
                vec![target_h.clone()],
                DeleteSelfLockabilityObservation::ExistingSeparatelyLockable,
            ),
        ])
        .unwrap();

        assert_eq!(
            serde_json::to_value(topology.integration_entries()).unwrap(),
            json!([
                {
                    "target": {"targetKind": "configurationRoot"},
                    "objectDisplay": "Root",
                    "action": "modify",
                    "reasons": ["canonicalDelta"],
                    "requiredLockTargets": [{"targetKind": "configurationRoot"}]
                },
                {
                    "target": {"targetKind": "developmentObject", "objectId": OBJECT_A},
                    "objectDisplay": "Top-level add",
                    "action": "add",
                    "reasons": ["canonicalDelta", "ownershipClosure", "addDeleteSemantics"],
                    "requiredLockTargets": [{"targetKind": "configurationRoot"}]
                },
                {
                    "target": {"targetKind": "developmentObject", "objectId": OBJECT_B},
                    "objectDisplay": "Subordinate add",
                    "action": "add",
                    "reasons": ["canonicalDelta", "ownershipClosure", "addDeleteSemantics"],
                    "requiredLockTargets": [
                        {"targetKind": "developmentObject", "objectId": OBJECT_D}
                    ]
                },
                {
                    "target": {"targetKind": "developmentObject", "objectId": OBJECT_C},
                    "objectDisplay": "Modify",
                    "action": "modify",
                    "reasons": ["canonicalDelta", "referenceClosure"],
                    "requiredLockTargets": [
                        {"targetKind": "developmentObject", "objectId": OBJECT_C},
                        {"targetKind": "developmentObject", "objectId": OBJECT_D}
                    ]
                },
                {
                    "target": {"targetKind": "developmentObject", "objectId": OBJECT_E},
                    "objectDisplay": "Owned child modify",
                    "action": "modify",
                    "reasons": ["canonicalDelta", "ownershipClosure"],
                    "requiredLockTargets": [
                        {"targetKind": "developmentObject", "objectId": OBJECT_E}
                    ]
                },
                {
                    "target": {"targetKind": "developmentObject", "objectId": OBJECT_F},
                    "objectDisplay": "Delete",
                    "action": "delete",
                    "reasons": [
                        "canonicalDelta",
                        "ownershipClosure",
                        "referenceClosure",
                        "addDeleteSemantics"
                    ],
                    "requiredLockTargets": [
                        {"targetKind": "developmentObject", "objectId": OBJECT_D},
                        {"targetKind": "developmentObject", "objectId": OBJECT_F},
                        {"targetKind": "developmentObject", "objectId": OBJECT_G},
                        {"targetKind": "developmentObject", "objectId": OBJECT_H}
                    ]
                }
            ])
        );
        assert_eq!(
            topology.expected_lock_reasons.get(&root).unwrap(),
            &vec![
                RepositoryUpdateLockReason::SupportGraphGuard,
                RepositoryUpdateLockReason::UpdateTarget,
                RepositoryUpdateLockReason::ParentClosure,
            ]
        );
        assert!(!topology.expected_lock_reasons.contains_key(&target_a));
        assert!(!topology.expected_lock_reasons.contains_key(&target_b));
        assert_eq!(
            topology.expected_lock_reasons.get(&target_c).unwrap(),
            &vec![RepositoryUpdateLockReason::UpdateTarget]
        );
        assert_eq!(
            topology.expected_lock_reasons.get(&target_d).unwrap(),
            &vec![
                RepositoryUpdateLockReason::ParentClosure,
                RepositoryUpdateLockReason::ReferenceClosure,
            ]
        );
        assert_eq!(
            topology.expected_lock_reasons.get(&target_e).unwrap(),
            &vec![RepositoryUpdateLockReason::UpdateTarget]
        );
        assert_eq!(
            topology.expected_lock_reasons.get(&target_f).unwrap(),
            &vec![RepositoryUpdateLockReason::UpdateTarget]
        );
        assert_eq!(
            topology.expected_lock_reasons.get(&target_g).unwrap(),
            &vec![RepositoryUpdateLockReason::StructuralClosure]
        );
        assert_eq!(
            topology.expected_lock_reasons.get(&target_h).unwrap(),
            &vec![RepositoryUpdateLockReason::ReferenceClosure]
        );

        let locks = serde_json::from_value(json!([
            {
                "targetKind": "configurationRoot",
                "objectDisplay": "Different root presentation",
                "reasons": ["supportGraphGuard", "updateTarget", "parentClosure"]
            },
            {
                "targetKind": "developmentObject",
                "objectId": OBJECT_C,
                "objectDisplay": "Different C presentation",
                "reasons": ["updateTarget"]
            },
            {
                "targetKind": "developmentObject",
                "objectId": OBJECT_D,
                "objectDisplay": "Different D presentation",
                "reasons": ["parentClosure", "referenceClosure"]
            },
            {
                "targetKind": "developmentObject",
                "objectId": OBJECT_E,
                "objectDisplay": "Different E presentation",
                "reasons": ["updateTarget"]
            },
            {
                "targetKind": "developmentObject",
                "objectId": OBJECT_F,
                "objectDisplay": "Different F presentation",
                "reasons": ["updateTarget"]
            },
            {
                "targetKind": "developmentObject",
                "objectId": OBJECT_G,
                "objectDisplay": "Different G presentation",
                "reasons": ["structuralClosure"]
            },
            {
                "targetKind": "developmentObject",
                "objectId": OBJECT_H,
                "objectDisplay": "Different H presentation",
                "reasons": ["referenceClosure"]
            }
        ]))
        .unwrap();
        topology.validate_lock_entries(&locks).unwrap();
    }

    #[test]
    fn repository_lock_plan_delete_retains_entry_for_all_self_lockability_branches() {
        let root = RepositoryTargetIdentity::configuration_root();
        let deleted = RepositoryTargetIdentity::development_object(object(OBJECT_A));
        for self_lockability in [
            DeleteSelfLockabilityObservation::Absent,
            DeleteSelfLockabilityObservation::ExistingNotSeparatelyLockable,
        ] {
            let topology = RepositoryIntegrationTopologyBatchAuthority::derive(vec![
                RepositoryIntegrationTopologyObservation::object_delete(
                    object_leaf(OBJECT_A),
                    display("Delete"),
                    root.clone(),
                    vec![],
                    vec![],
                    self_lockability,
                ),
            ])
            .unwrap();
            assert!(matches!(
                topology.integration_entries.as_slice(),
                [RepositoryIntegrationEntry::ObjectDelete(_)]
            ));
            assert_eq!(
                topology.integration_entries.as_slice()[0]
                    .required_lock_targets()
                    .as_slice(),
                std::slice::from_ref(&root)
            );
            assert!(!topology.expected_lock_reasons.contains_key(&deleted));
        }

        let topology = RepositoryIntegrationTopologyBatchAuthority::derive(vec![
            RepositoryIntegrationTopologyObservation::object_delete(
                object_leaf(OBJECT_A),
                display("Delete"),
                root.clone(),
                vec![],
                vec![],
                DeleteSelfLockabilityObservation::ExistingSeparatelyLockable,
            ),
        ])
        .unwrap();
        assert_eq!(
            topology.integration_entries.as_slice()[0]
                .required_lock_targets()
                .as_slice(),
            &[root, deleted.clone()]
        );
        assert_eq!(
            topology.expected_lock_reasons.get(&deleted).unwrap(),
            &vec![RepositoryUpdateLockReason::UpdateTarget]
        );
    }

    #[test]
    fn repository_lock_plan_keeps_global_root_guard_out_of_unrelated_entry_closure() {
        let root = RepositoryTargetIdentity::configuration_root();
        let modified = RepositoryTargetIdentity::development_object(object(OBJECT_A));
        let topology = RepositoryIntegrationTopologyBatchAuthority::derive(vec![
            RepositoryIntegrationTopologyObservation::object_modify(
                object_leaf(OBJECT_A),
                display("Modify"),
                vec![],
            ),
        ])
        .unwrap();
        assert_eq!(
            topology.integration_entries.as_slice()[0]
                .required_lock_targets()
                .as_slice(),
            std::slice::from_ref(&modified)
        );
        assert_eq!(
            topology.expected_lock_reasons.get(&root).unwrap(),
            &vec![RepositoryUpdateLockReason::SupportGraphGuard]
        );
        assert_eq!(
            topology.expected_lock_reasons.get(&modified).unwrap(),
            &vec![RepositoryUpdateLockReason::UpdateTarget]
        );
    }

    #[test]
    fn repository_lock_plan_never_locks_a_new_add_target() {
        let root = RepositoryTargetIdentity::configuration_root();
        let target_a = RepositoryTargetIdentity::development_object(object(OBJECT_A));
        let target_b = RepositoryTargetIdentity::development_object(object(OBJECT_B));
        let top_level = RepositoryIntegrationTopologyBatchAuthority::derive(vec![
            RepositoryIntegrationTopologyObservation::top_level_add(
                object_leaf(OBJECT_A),
                display("Top-level add"),
            ),
        ])
        .unwrap();
        assert_eq!(
            top_level.integration_entries.as_slice()[0]
                .required_lock_targets()
                .as_slice(),
            &[root]
        );
        assert!(!top_level.expected_lock_reasons.contains_key(&target_a));

        let subordinate = RepositoryIntegrationTopologyBatchAuthority::derive(vec![
            RepositoryIntegrationTopologyObservation::subordinate_add(
                object_leaf(OBJECT_B),
                display("Subordinate add"),
                object_leaf(OBJECT_A),
            ),
        ])
        .unwrap();
        assert_eq!(
            subordinate.integration_entries.as_slice()[0]
                .required_lock_targets()
                .as_slice(),
            std::slice::from_ref(&target_a)
        );
        assert!(!subordinate.expected_lock_reasons.contains_key(&target_b));

        assert!(RepositoryIntegrationTopologyBatchAuthority::derive(vec![
            RepositoryIntegrationTopologyObservation::top_level_add(
                object_leaf(OBJECT_A),
                display("New target"),
            ),
            RepositoryIntegrationTopologyObservation::object_modify(
                object_leaf(OBJECT_B),
                display("Existing referrer"),
                vec![target_a],
            ),
        ])
        .is_err());
    }

    #[test]
    fn repository_lock_plan_unions_overlapping_roles_and_rejects_noncanonical_inputs() {
        let target_a = RepositoryTargetIdentity::development_object(object(OBJECT_A));
        let target_b = RepositoryTargetIdentity::development_object(object(OBJECT_B));
        let modify = RepositoryIntegrationTopologyBatchAuthority::derive(vec![
            RepositoryIntegrationTopologyObservation::object_modify(
                object_leaf(OBJECT_A),
                display("Self referring modify"),
                vec![target_a.clone()],
            ),
        ])
        .unwrap();
        assert_eq!(
            modify.expected_lock_reasons.get(&target_a).unwrap(),
            &vec![
                RepositoryUpdateLockReason::UpdateTarget,
                RepositoryUpdateLockReason::ReferenceClosure,
            ]
        );
        assert_eq!(
            modify.integration_entries.as_slice()[0]
                .required_lock_targets()
                .as_slice(),
            std::slice::from_ref(&target_a)
        );

        const OBJECT_C: &str = "00000000-0000-0000-0000-000000000003";
        let target_c = RepositoryTargetIdentity::development_object(object(OBJECT_C));
        let delete = RepositoryIntegrationTopologyBatchAuthority::derive(vec![
            RepositoryIntegrationTopologyObservation::object_delete(
                object_leaf(OBJECT_B),
                display("Delete"),
                target_a.clone(),
                vec![target_c.clone()],
                vec![target_a.clone(), target_c.clone()],
                DeleteSelfLockabilityObservation::ExistingSeparatelyLockable,
            ),
        ])
        .unwrap();
        assert_eq!(
            delete.expected_lock_reasons.get(&target_a).unwrap(),
            &vec![
                RepositoryUpdateLockReason::ParentClosure,
                RepositoryUpdateLockReason::ReferenceClosure,
            ]
        );
        assert_eq!(
            delete.expected_lock_reasons.get(&target_c).unwrap(),
            &vec![
                RepositoryUpdateLockReason::ReferenceClosure,
                RepositoryUpdateLockReason::StructuralClosure,
            ]
        );

        assert!(RepositoryIntegrationTopologyBatchAuthority::derive(vec![
            RepositoryIntegrationTopologyObservation::object_modify(
                object_leaf(OBJECT_B),
                display("B"),
                vec![],
            ),
            RepositoryIntegrationTopologyObservation::object_modify(
                object_leaf(OBJECT_A),
                display("A"),
                vec![],
            ),
        ])
        .is_err());
        assert!(RepositoryIntegrationTopologyBatchAuthority::derive(vec![
            RepositoryIntegrationTopologyObservation::object_modify(
                object_leaf(OBJECT_A),
                display("A first"),
                vec![],
            ),
            RepositoryIntegrationTopologyObservation::object_modify(
                object_leaf(OBJECT_A),
                display("A duplicate"),
                vec![],
            ),
        ])
        .is_err());
        for changed_referrers in [
            vec![target_b.clone(), target_a.clone()],
            vec![target_a.clone(), target_a.clone()],
        ] {
            assert!(RepositoryIntegrationTopologyBatchAuthority::derive(vec![
                RepositoryIntegrationTopologyObservation::object_modify(
                    object_leaf(OBJECT_C),
                    display("Noncanonical referrers"),
                    changed_referrers,
                ),
            ])
            .is_err());
        }
    }

    #[test]
    fn repository_lock_plan_rejects_contradictory_delete_topology() {
        let root = RepositoryTargetIdentity::configuration_root();
        let target_a = RepositoryTargetIdentity::development_object(object(OBJECT_A));
        assert!(RepositoryIntegrationTopologyBatchAuthority::derive(vec![
            RepositoryIntegrationTopologyObservation::object_delete(
                object_leaf(OBJECT_B),
                display("Parent also claimed as subordinate"),
                target_a.clone(),
                vec![target_a],
                vec![],
                DeleteSelfLockabilityObservation::ExistingSeparatelyLockable,
            ),
        ])
        .is_err());
        assert!(RepositoryIntegrationTopologyBatchAuthority::derive(vec![
            RepositoryIntegrationTopologyObservation::object_delete(
                object_leaf(OBJECT_A),
                display("Deleted target also claimed as changed referrer"),
                root,
                vec![],
                vec![RepositoryTargetIdentity::development_object(object(
                    OBJECT_A
                ))],
                DeleteSelfLockabilityObservation::Absent,
            ),
        ])
        .is_err());
    }

    #[test]
    fn repository_result_validated_commit_object_fixture_binds_version_digest_and_atomic_capability(
    ) {
        let version = RepositoryVersion::parse("101").unwrap();
        let capability = CapabilityRowId::parse("repository.atomic-commit.v1").unwrap();
        let authority = validated_commit_object_authority_fixture_test_only(
            version.clone(),
            capability.clone(),
        );
        let released_objects = targets(vec![RepositoryTargetIdentity::configuration_root()]);
        let empty = targets(Vec::new());
        assert!(validate_commit_release_projection(
            &authority.approved_preview.0.plan,
            &authority.approved_preview.0.record.guard_locks,
            &released_objects,
            &empty,
        )
        .is_ok());
        assert!(validate_commit_release_projection(
            &authority.approved_preview.0.plan,
            &authority.approved_preview.0.record.guard_locks,
            &empty,
            &empty,
        )
        .is_err());
        assert!(validate_commit_release_projection(
            &authority.approved_preview.0.plan,
            &authority.approved_preview.0.record.guard_locks,
            &empty,
            &released_objects,
        )
        .is_err());
        assert_eq!(authority.repository_version(), &version);
        assert_eq!(authority.atomic_commit_safety_capability_id(), &capability);
        assert_eq!(authority.committed_objects().as_slice().len(), 1);
        assert_ne!(
            authority.committed_objects_digest(),
            authority.exact_objects_digest()
        );
    }

    #[test]
    fn repository_result_task_commit_partition_rejects_another_commit_authority() {
        let first = validated_commit_object_authority_fixture_test_only(
            RepositoryVersion::parse("101").unwrap(),
            CapabilityRowId::parse("repository.atomic-commit.first").unwrap(),
        );
        let second = validated_commit_object_authority_fixture_test_only(
            RepositoryVersion::parse("102").unwrap(),
            CapabilityRowId::parse("repository.atomic-commit.second").unwrap(),
        );
        let second_partition = crate::domain::branched_development::contracts::repository::validated_task_commit_partition_fixture_test_only(&second);
        assert!(second_partition.binds(&second));
        assert!(!second_partition.binds(&first));
    }

    #[test]
    fn repository_result_cancellation_apply_context_rejects_task_status_and_cwd_splices() {
        use crate::domain::branched_development::contracts::requests::repository::RepositoryUpdateRequest;

        let approved = digest('a');
        let request: RepositoryUpdateRequest = serde_json::from_value(json!({
            "cwd": "/original/project",
            "taskId": "TASK-173",
            "operationId": "123e4567-e89b-12d3-a456-426614174000",
            "mode": "supportPrerequisiteCancellation",
            "expectedStatusDigest": approved,
            "supportActionId": "223e4567-e89b-12d3-a456-426614174000",
            "expectedSupportActionDigest": digest('b'),
            "reason": "operatorCancelled",
            "dryRun": false,
            "approvedCancellationDigest": digest('a'),
        }))
        .unwrap();
        let token = request
            .validate_cancellation_approval(&digest('a'))
            .unwrap();
        let cwd: OriginalProjectCwd = serde_json::from_value(json!("/original/project")).unwrap();
        let other_cwd: OriginalProjectCwd =
            serde_json::from_value(json!("/other/project")).unwrap();
        let task_id: TaskId = serde_json::from_value(json!("TASK-173")).unwrap();
        let other_task: TaskId = serde_json::from_value(json!("TASK-174")).unwrap();
        assert!(cancellation_apply_context_matches(
            &cwd,
            &task_id,
            &digest('a'),
            &token,
        ));
        assert!(!cancellation_apply_context_matches(
            &other_cwd,
            &task_id,
            &digest('a'),
            &token,
        ));
        assert!(!cancellation_apply_context_matches(
            &cwd,
            &other_task,
            &digest('a'),
            &token,
        ));
        assert!(!cancellation_apply_context_matches(
            &cwd,
            &task_id,
            &digest('c'),
            &token,
        ));
    }

    #[test]
    fn repository_result_prearm_terminal_evidence_hashes_the_whole_recovery_result() {
        let evidence = crate::domain::branched_development::contracts::recovery::validated_completed_prearm_terminal_evidence_fixture_test_only();
        let authority =
            ValidatedPreArmCancellationRecoveryAuthority::from_terminal_evidence(evidence).unwrap();
        let expected_receipt_digest = authority.recovery_receipt_digest.clone();
        let data = PreArmCancellationRecoveryData::from_authority(authority);
        let encoded = serde_json::to_value(data).unwrap();
        assert_eq!(encoded["target"], json!("preArmSupportCancellation"));
        assert_eq!(encoded["effectClass"], json!("reconcileOnly"));
        assert_eq!(encoded["armingReceiptAbsent"], json!(true));
        assert_eq!(
            encoded["recoveryReceiptDigest"],
            serde_json::to_value(expected_receipt_digest).unwrap()
        );
        assert!(encoded["remainingUnknowns"].as_array().unwrap().is_empty());
        assert!(!encoded["terminalObservations"]
            .as_array()
            .unwrap()
            .is_empty());
        assert_eq!(
            encoded["actions"].as_array().unwrap().len(),
            encoded["actionOutcomes"].as_array().unwrap().len()
        );
    }

    #[test]
    fn empty_recovery_unknowns_schema_is_an_exact_typed_empty_array() {
        let contract = schema::<EmptyRecoveryUnknowns>();
        audit_json_schema(&contract).unwrap();
        assert_eq!(contract["type"], json!("array"));
        assert_eq!(contract["minItems"], json!(0));
        assert_eq!(contract["maxItems"], json!(0));
        assert_eq!(
            contract["items"],
            json!({"$ref": "#/$defs/RecoveryUnknown"})
        );
        assert!(contract["$defs"]["RecoveryUnknown"].is_object());
    }

    #[test]
    fn repository_result_update_and_recovery_schemas_are_recursively_closed() {
        for contract in [
            schema::<RepositoryUpdatePreviewData>(),
            schema::<SupportPrerequisitePreviewData>(),
            schema::<SupportPrerequisiteCancellationPreviewData>(),
            schema::<SupportActionCancellationData>(),
            schema::<PreArmCancellationRecoveryData>(),
            schema::<SupportRecoveryData>(),
            schema::<GeneralRecoveryData>(),
            schema::<RecoveryData>(),
        ] {
            audit_json_schema(&contract).unwrap_or_else(|error| panic!("{error}: {contract}"));
        }
    }
}

#[cfg(test)]
mod gate_b2_preview_tests {
    use super::*;
    use crate::domain::branched_development::contracts::artifacts::ConfigurationIdentity;
    use crate::domain::branched_development::contracts::repository::{
        empty_commit_history_evidence_fixture_test_only,
        repository_history_partition_fixture_test_only,
        task_commit_history_partition_fixture_test_only, EvidenceKind, EvidenceSourceIndex,
        EvidenceSourceIndexCandidate, EvidenceSourceIndexCandidateRow, EvidenceSourceRegistry,
        RepositoryContractError, RepositoryHistoryEvidenceBytesResolver,
        RepositoryHistoryOrderEvidence, RepositoryHistoryOrderResolver,
        RepositoryHistorySourceEvidenceRef, RoutineRepositoryVersionClassificationEvidence,
    };
    use crate::domain::branched_development::contracts::requests::repository::{
        RepositoryCommitRequest, RepositoryCommitRequestValidationFailure,
        ValidatedRepositoryCommitApplyRequest, ValidatedRepositoryCommitPreviewRequest,
    };
    use crate::domain::branched_development::contracts::results::merge::{
        validated_main_integration_commit_context_fixture_test_only,
        validated_main_integration_commit_context_with_lock_identity_fixture_test_only,
        ResolvedCommitLineageConsumedSupportGateAuthority,
    };
    use crate::domain::branched_development::contracts::scalars::PositiveGeneration;
    use crate::domain::branched_development::contracts::storage::OperationScope;
    use serde_json::{json, Value};
    use sha2::{Digest, Sha256};
    use std::cell::{Cell, RefCell};
    use std::collections::BTreeMap;
    use std::rc::Rc;

    macro_rules! assert_not_clone {
        ($type:ty) => {
            const _: fn() = || {
                trait AmbiguousIfClone<Marker> {
                    fn assert_not_clone() {}
                }
                struct ImplementsClone;
                impl<T: ?Sized> AmbiguousIfClone<()> for T {}
                impl<T: Clone> AmbiguousIfClone<ImplementsClone> for T {}
                let _ = <$type as AmbiguousIfClone<_>>::assert_not_clone;
            };
        };
    }

    macro_rules! assert_not_serialize {
        ($type:ty) => {
            const _: fn() = || {
                trait AmbiguousIfSerialize<Marker> {
                    fn assert_not_serialize() {}
                }
                struct ImplementsSerialize;
                impl<T: ?Sized> AmbiguousIfSerialize<()> for T {}
                impl<T: ?Sized + serde::Serialize> AmbiguousIfSerialize<ImplementsSerialize> for T {}
                let _ = <$type as AmbiguousIfSerialize<_>>::assert_not_serialize;
            };
        };
    }

    macro_rules! assert_not_deserialize_owned {
        ($type:ty) => {
            const _: fn() = || {
                trait AmbiguousIfDeserialize<Marker> {
                    fn assert_not_deserialize() {}
                }
                struct ImplementsDeserialize;
                impl<T: ?Sized> AmbiguousIfDeserialize<()> for T {}
                impl<T: serde::de::DeserializeOwned> AmbiguousIfDeserialize<ImplementsDeserialize>
                    for T
                {
                }
                let _ = <$type as AmbiguousIfDeserialize<_>>::assert_not_deserialize;
            };
        };
    }

    assert_not_clone!(RepositoryIntegrationTopologyObservation);
    assert_not_clone!(RepositoryIntegrationTopologyBatchAuthority);
    assert_not_clone!(RepositoryLockPlanObservedIds);
    assert_not_clone!(RepositoryLockPlanObservedEvidence);
    assert_not_clone!(RepositoryLockPlanObservationInput);
    assert_not_clone!(RepositoryLockPlanObservationRequest<'static>);
    assert_not_clone!(RepositoryLockPlanObservationInvocationCapability);
    assert_not_clone!(RepositoryLockPlanObservationCompletionCapability);
    assert_not_clone!(RepositoryLockPlanObservationLease);
    assert_not_serialize!(RepositoryIntegrationTopologyObservation);
    assert_not_serialize!(RepositoryIntegrationTopologyBatchAuthority);
    assert_not_serialize!(RepositoryLockPlanObservedIds);
    assert_not_serialize!(RepositoryLockPlanObservedEvidence);
    assert_not_serialize!(RepositoryLockPlanObservationInput);
    assert_not_serialize!(RepositoryLockPlanObservationRequest<'static>);
    assert_not_serialize!(RepositoryLockPlanObservationInvocationCapability);
    assert_not_serialize!(RepositoryLockPlanObservationCompletionCapability);
    assert_not_serialize!(RepositoryLockPlanObservationLease);
    assert_not_deserialize_owned!(RepositoryIntegrationTopologyObservation);
    assert_not_deserialize_owned!(RepositoryIntegrationTopologyBatchAuthority);
    assert_not_deserialize_owned!(RepositoryLockPlanObservedIds);
    assert_not_deserialize_owned!(RepositoryLockPlanObservedEvidence);
    assert_not_deserialize_owned!(RepositoryLockPlanObservationInput);
    assert_not_deserialize_owned!(RepositoryLockPlanObservationRequest<'static>);
    assert_not_deserialize_owned!(RepositoryLockPlanObservationInvocationCapability);
    assert_not_deserialize_owned!(RepositoryLockPlanObservationCompletionCapability);
    assert_not_deserialize_owned!(RepositoryLockPlanObservationLease);
    assert_not_clone!(PostMergeCommitGuardRequest<'static>);
    assert_not_clone!(PostMergeCommitGuardInvocationCapability);
    assert_not_clone!(PostMergeCommitGuardCompletionCapability);
    assert_not_clone!(PostMergeCommitGuardCompletion);
    assert_not_serialize!(PostMergeCommitGuardRequest<'static>);
    assert_not_serialize!(PostMergeCommitGuardInvocationCapability);
    assert_not_serialize!(PostMergeCommitGuardCompletionCapability);
    assert_not_serialize!(PostMergeCommitGuardCompletion);
    assert_not_deserialize_owned!(PostMergeCommitGuardRequest<'static>);
    assert_not_deserialize_owned!(PostMergeCommitGuardInvocationCapability);
    assert_not_deserialize_owned!(PostMergeCommitGuardCompletionCapability);
    assert_not_deserialize_owned!(PostMergeCommitGuardCompletion);
    assert_not_clone!(ValidatedRepositoryCommitPreviewRequest);
    assert_not_clone!(ValidatedRepositoryCommitApplyRequest);
    assert_not_clone!(RepositoryCommitRequestValidationFailure);
    assert_not_clone!(ValidatedCommitApplyApprovalAuthority);
    assert_not_clone!(CommitApplyApprovalFailureEvidence);
    assert_not_clone!(CommitApplyApprovalBlockedAuthority);
    assert_not_serialize!(ValidatedRepositoryCommitPreviewRequest);
    assert_not_serialize!(ValidatedRepositoryCommitApplyRequest);
    assert_not_serialize!(RepositoryCommitRequestValidationFailure);
    assert_not_serialize!(ValidatedCommitApplyApprovalAuthority);
    assert_not_serialize!(CommitApplyApprovalFailureEvidence);
    assert_not_serialize!(CommitApplyApprovalBlockedAuthority);
    assert_not_deserialize_owned!(ValidatedRepositoryCommitPreviewRequest);
    assert_not_deserialize_owned!(ValidatedRepositoryCommitApplyRequest);
    assert_not_deserialize_owned!(RepositoryCommitRequestValidationFailure);
    assert_not_deserialize_owned!(ValidatedCommitApplyApprovalAuthority);
    assert_not_deserialize_owned!(CommitApplyApprovalFailureEvidence);
    assert_not_deserialize_owned!(CommitApplyApprovalBlockedAuthority);
    assert_not_clone!(CommitSafetyLineageBinding);
    assert_not_clone!(CommitSafetyLineageWitness);
    assert_not_clone!(CommitImmediateRecheckRequest<'static>);
    assert_not_clone!(CommitImmediateRecheckCompletion);
    assert_not_clone!(CommitScopedAtomicSafetyAuthority);
    assert_not_clone!(CommitFreshPreviewRequiredAuthority);
    assert_not_clone!(CommitImmediateRecoveryRequiredAuthority);
    assert_not_serialize!(CommitSafetyLineageBinding);
    assert_not_serialize!(CommitSafetyLineageWitness);
    assert_not_serialize!(CommitImmediateRecheckRequest<'static>);
    assert_not_serialize!(CommitImmediateRecheckCompletion);
    assert_not_serialize!(CommitScopedAtomicSafetyAuthority);
    assert_not_serialize!(CommitFreshPreviewRequiredAuthority);
    assert_not_serialize!(CommitImmediateRecoveryRequiredAuthority);
    assert_not_deserialize_owned!(CommitSafetyLineageBinding);
    assert_not_deserialize_owned!(CommitSafetyLineageWitness);
    assert_not_deserialize_owned!(CommitImmediateRecheckRequest<'static>);
    assert_not_deserialize_owned!(CommitImmediateRecheckCompletion);
    assert_not_deserialize_owned!(CommitScopedAtomicSafetyAuthority);
    assert_not_deserialize_owned!(CommitFreshPreviewRequiredAuthority);
    assert_not_deserialize_owned!(CommitImmediateRecoveryRequiredAuthority);

    const _: fn(ValidatedCommitApplyApprovalAuthority) -> ApprovedCommitPreviewAuthority =
        ApprovedCommitPreviewAuthority::from_validated_request;
    const _: fn(
        CommitPreviewAuthority,
        RepositoryCommitRequest,
    ) -> Result<
        ValidatedCommitApplyApprovalAuthority,
        Box<CommitApplyApprovalBlockedAuthority>,
    > = CommitPreviewAuthority::validate_apply;
    const _: fn(
        CommitScopedAtomicSafetyAuthority,
        CommitObjectPostStateObservationAuthority,
    ) -> Result<ValidatedCommitObjectAuthority, RepositoryResultContractError> =
        ValidatedCommitObjectAuthority::from_commit_scope;

    fn digest(character: char) -> Sha256Digest {
        Sha256Digest::parse(&character.to_string().repeat(64)).unwrap()
    }

    fn id(value: &str) -> UnicaId {
        UnicaId::parse(value).unwrap()
    }

    fn validated_comment_policy() -> ValidatedCommitCommentPolicyAuthority {
        let task_id = TaskId::parse("PR-137").unwrap();
        let frozen = FrozenCommitCommentPolicyAuthority::from_task_start_renderer_adapter(
            Comment::parse("{taskId}: {summary}").unwrap(),
            task_id.clone(),
            TaskSummary::parse("Consumed gate B2").unwrap(),
            ProjectId::parse("b2100000-0000-4000-8000-000000000001").unwrap(),
            Comment::parse("PR-137: Consumed gate B2").unwrap(),
            CapabilityRowId::parse("profile.commit-comment.gate-b2").unwrap(),
        )
        .unwrap();
        ValidatedCommitCommentPolicyAuthority::revalidate(
            frozen,
            Comment::parse("{taskId}: {summary}").unwrap(),
            task_id,
            TaskSummary::parse("Consumed gate B2").unwrap(),
            ProjectId::parse("b2100000-0000-4000-8000-000000000001").unwrap(),
            Comment::parse("PR-137: Consumed gate B2").unwrap(),
            CapabilityRowId::parse("profile.commit-comment.gate-b2").unwrap(),
        )
        .unwrap()
    }

    #[derive(Clone, Copy)]
    enum GuardMismatch {
        None,
        Receipt,
        Plan,
        Lock,
        Integration,
        ConsumedCapability,
        RootRereadCapability,
        PortError,
        History,
        Closure,
        Fingerprint,
    }

    struct TestPostMergeGuardLease {
        lineage_witness: CommitSafetyLineageWitness,
        history: PostMergeHistoryGuardEvidence,
        observed_fingerprint: Sha256Digest,
        observed_repository_anchor: RepositoryAnchor,
        capability_id: CapabilityRowId,
        binds: bool,
    }

    impl PostMergeCommitGuardLease for TestPostMergeGuardLease {
        fn commit_safety_lineage_witness(&self) -> &CommitSafetyLineageWitness {
            &self.lineage_witness
        }

        fn binds(&self, request: &PostMergeCommitGuardRequest<'_>) -> bool {
            let _complete_initial_scope = (
                request.consumed_state_observation_capability_id(),
                request.root_reread_capability_id(),
                request.lock_plan(),
                request.rollback_checkpoint_id(),
                request.journaled_lock_receipts(),
            );
            self.binds
        }

        fn history_guard_evidence(&self) -> &PostMergeHistoryGuardEvidence {
            &self.history
        }

        fn observed_original_fingerprint(&self) -> &Sha256Digest {
            &self.observed_fingerprint
        }

        fn observed_repository_anchor(&self) -> &RepositoryAnchor {
            &self.observed_repository_anchor
        }

        fn original_fingerprint_capability_id(&self) -> &CapabilityRowId {
            &self.capability_id
        }
    }

    struct TestPostMergeGuardPort {
        mismatch: GuardMismatch,
    }

    impl PostMergeCommitGuardPort for TestPostMergeGuardPort {
        fn observe_post_merge_commit_guard(
            &mut self,
            request: PostMergeCommitGuardRequest<'_>,
        ) -> Result<PostMergeCommitGuardCompletion, RepositoryResultContractError> {
            if matches!(self.mismatch, GuardMismatch::PortError) {
                return Err(RepositoryResultContractError(
                    "post-merge guard adapter failed",
                ));
            }
            let cursor = if matches!(self.mismatch, GuardMismatch::History) {
                RepositoryHistoryCursor::new(RepositoryVersion::parse("999").unwrap(), digest('e'))
            } else {
                request.merge_receipt_cursor().clone()
            };
            let closure = if matches!(self.mismatch, GuardMismatch::Closure) {
                digest('e')
            } else {
                request.reference_closure_digest().clone()
            };
            let (_, history) = empty_commit_history_evidence_fixture_test_only(
                cursor,
                digest('d'),
                closure,
                CapabilityRowId::parse("repository.atomic-commit.gate-b2").unwrap(),
            )
            .map_err(|_| RepositoryResultContractError("history fixture failed"))?;
            let binds = match self.mismatch {
                GuardMismatch::None
                | GuardMismatch::PortError
                | GuardMismatch::History
                | GuardMismatch::Closure
                | GuardMismatch::Fingerprint => true,
                GuardMismatch::Receipt => {
                    request.merge_receipt_id() == &id("b2100000-0000-4000-8000-000000000099")
                }
                GuardMismatch::Plan => {
                    request.plan_id() == &id("b2100000-0000-4000-8000-000000000098")
                }
                GuardMismatch::Lock => {
                    request.lock_set_id() == &id("b2100000-0000-4000-8000-000000000097")
                }
                GuardMismatch::Integration => {
                    request.integration_set_id() == &id("b2100000-0000-4000-8000-000000000096")
                }
                GuardMismatch::ConsumedCapability => {
                    request.consumed_state_observation_capability_id()
                        == &CapabilityRowId::parse("repository.consumed-gate.foreign").unwrap()
                }
                GuardMismatch::RootRereadCapability => {
                    request.root_reread_capability_id()
                        == &CapabilityRowId::parse("repository.root-reread.foreign").unwrap()
                }
            };
            let observed_fingerprint = if matches!(self.mismatch, GuardMismatch::Fingerprint) {
                digest('f')
            } else {
                request.authorized_result_fingerprint().clone()
            };
            let root = request.expected_plan_root_anchor();
            let observed_repository_anchor = request.observe_repository_anchor(
                &history,
                root.repository_identity().clone(),
                root.configuration_identity().clone(),
                observed_fingerprint.clone(),
            )?;
            let lineage_witness = request.commit_safety_lineage_witness();
            Ok(request.complete(Box::new(TestPostMergeGuardLease {
                lineage_witness,
                history,
                observed_fingerprint,
                observed_repository_anchor,
                capability_id: CapabilityRowId::parse("repository.original-fingerprint.gate-b2")
                    .unwrap(),
                binds,
            })))
        }
    }

    struct ObserveThenReplayPostMergeGuardPort {
        stored_completion: Option<PostMergeCommitGuardCompletion>,
    }

    impl PostMergeCommitGuardPort for ObserveThenReplayPostMergeGuardPort {
        fn observe_post_merge_commit_guard(
            &mut self,
            request: PostMergeCommitGuardRequest<'_>,
        ) -> Result<PostMergeCommitGuardCompletion, RepositoryResultContractError> {
            if self.stored_completion.is_none() {
                let completion = TestPostMergeGuardPort {
                    mismatch: GuardMismatch::None,
                }
                .observe_post_merge_commit_guard(request)?;
                self.stored_completion = Some(completion);
                return Err(RepositoryResultContractError(
                    "post-merge guard response was lost after lease creation",
                ));
            }
            self.stored_completion
                .take()
                .ok_or(RepositoryResultContractError(
                    "stored post-merge guard lease already replayed",
                ))
        }
    }

    fn context(
        receipt_id: UnicaId,
        fingerprint: Sha256Digest,
    ) -> ResolvedCommitLineageConsumedSupportGateAuthority {
        validated_main_integration_commit_context_fixture_test_only(receipt_id, fingerprint)
    }

    fn validated_guard(
        receipt_id: UnicaId,
        fingerprint: Sha256Digest,
    ) -> PostMergeCommitGuardAuthority {
        let source = context(receipt_id, fingerprint);
        PostMergeCommitGuardAuthority::from_authoritative_consumed_lineage(
            source,
            &mut TestPostMergeGuardPort {
                mismatch: GuardMismatch::None,
            },
        )
        .unwrap()
    }

    const PREVIEW_OPERATION_ID: &str = "b3100000-0000-4000-8000-000000000001";
    const APPLY_OPERATION_ID: &str = "b3100000-0000-4000-8000-000000000002";
    const OTHER_LINEAGE_ID: &str = "b3100000-0000-4000-8000-000000000099";

    fn commit_preview_request_value(
        guard: &PostMergeCommitGuardAuthority,
        cwd: &str,
        task_id: &str,
        operation_id: &str,
    ) -> Value {
        let lineage = guard.source.lineage();
        json!({
            "cwd": cwd,
            "taskId": task_id,
            "operationId": operation_id,
            "integrationSetId": lineage.integration_set_id(),
            "expectedIntegrationSetDigest": lineage.integration_set_digest(),
            "lockSetId": lineage.lock_set_id(),
            "expectedLockSetDigest": lineage.lock_set_digest(),
            "verificationId": lineage.verification_id(),
            "expectedVerificationDigest": lineage.verification_digest(),
            "mergeReceiptId": lineage.merge_receipt_id(),
            "supportGateId": lineage.support_gate_id(),
            "expectedSupportGateDigest": lineage.support_gate_digest(),
            "expectedSupportGateHistoryEvidenceDigest": lineage
                .support_gate_history_evidence()
                .evidence_digest(),
            "expectedAuthorizedPostMergeFingerprint": lineage.result_fingerprint(),
        })
    }

    fn validated_commit_preview_request(value: Value) -> ValidatedRepositoryCommitPreviewRequest {
        serde_json::from_value::<RepositoryCommitRequest>(value)
            .unwrap()
            .into_validated_preview()
            .unwrap()
    }

    fn commit_apply_request_value(
        mut preview: Value,
        operation_id: &str,
        approved_commit_digest: &Sha256Digest,
    ) -> Value {
        let object = preview
            .as_object_mut()
            .expect("commit request fixture must be an object");
        object.insert("operationId".to_owned(), json!(operation_id));
        object.insert("dryRun".to_owned(), json!(false));
        object.insert(
            "approvedCommitDigest".to_owned(),
            serde_json::to_value(approved_commit_digest).unwrap(),
        );
        preview
    }

    fn validated_preview_authority() -> (CommitPreviewAuthority, Value) {
        let guard = validated_guard(id("b3100000-0000-4000-8000-000000000014"), digest('9'));
        let request_value = commit_preview_request_value(
            &guard,
            "/original/project",
            "PR-137",
            PREVIEW_OPERATION_ID,
        );
        let request = validated_commit_preview_request(request_value.clone());
        let preview = CommitPreviewAuthority::from_validated_post_merge_guard(
            request,
            guard,
            validated_comment_policy(),
        )
        .unwrap();
        (preview, request_value)
    }

    const ATOMIC_COMMIT_CAPABILITY_ID: &str = "repository.atomic-commit.gate-b3";
    const REFRESH_OPERATION_ID: &str = "b3100000-0000-4000-8000-000000000003";

    fn history_partition(
        start: RepositoryHistoryCursor,
        classifications: &[RepositoryHistoryPartitionClassification],
        order_capability_id: &str,
    ) -> ValidatedRepositoryHistoryPartition {
        repository_history_partition_fixture_test_only(
            start,
            classifications
                .iter()
                .enumerate()
                .map(|(index, classification)| {
                    (
                        RepositoryVersion::parse(&(101 + index).to_string()).unwrap(),
                        *classification,
                    )
                })
                .collect(),
            order_capability_id,
            ATOMIC_COMMIT_CAPABILITY_ID,
        )
        .unwrap()
    }

    struct TestInitialHistoryPort {
        partition: ValidatedRepositoryHistoryPartition,
        repository_identity: Option<Sha256Digest>,
        configuration_identity: Option<ConfigurationIdentity>,
        configuration_fingerprint: Option<Sha256Digest>,
    }

    impl TestInitialHistoryPort {
        fn exact(partition: ValidatedRepositoryHistoryPartition) -> Self {
            Self {
                partition,
                repository_identity: None,
                configuration_identity: None,
                configuration_fingerprint: None,
            }
        }
    }

    impl PostMergeCommitGuardPort for TestInitialHistoryPort {
        fn observe_post_merge_commit_guard(
            &mut self,
            request: PostMergeCommitGuardRequest<'_>,
        ) -> Result<PostMergeCommitGuardCompletion, RepositoryResultContractError> {
            let authority = PostMergeHistoryGuardAuthority::from_capability_adapter(
                &self.partition,
                request.merge_receipt_cursor().clone(),
                request.reference_closure_digest().clone(),
                CapabilityRowId::parse(ATOMIC_COMMIT_CAPABILITY_ID).unwrap(),
            )
            .map_err(|_| RepositoryResultContractError("initial history authority failed"))?;
            let history = PostMergeHistoryGuardEvidence::new(self.partition.clone(), &authority)
                .map_err(|_| RepositoryResultContractError("initial history evidence failed"))?;
            let expected_root = request.expected_plan_root_anchor();
            let observed_fingerprint = self
                .configuration_fingerprint
                .clone()
                .unwrap_or_else(|| request.authorized_result_fingerprint().clone());
            let observed_repository_anchor = request.observe_repository_anchor(
                &history,
                self.repository_identity
                    .clone()
                    .unwrap_or_else(|| expected_root.repository_identity().clone()),
                self.configuration_identity
                    .clone()
                    .unwrap_or_else(|| expected_root.configuration_identity().clone()),
                observed_fingerprint.clone(),
            )?;
            let lineage_witness = request.commit_safety_lineage_witness();
            Ok(request.complete(Box::new(TestPostMergeGuardLease {
                lineage_witness,
                history,
                observed_fingerprint,
                observed_repository_anchor,
                capability_id: CapabilityRowId::parse("repository.original-fingerprint.gate-b2")
                    .unwrap(),
                binds: true,
            })))
        }
    }

    fn initial_guard_with_history(
        classifications: &[RepositoryHistoryPartitionClassification],
        order_capability_id: &str,
    ) -> Result<
        (
            PostMergeCommitGuardAuthority,
            ValidatedRepositoryHistoryPartition,
        ),
        Box<PostMergeCommitGuardBlockedAuthority>,
    > {
        let source = context(id("b3100000-0000-4000-8000-000000000024"), digest('9'));
        let partition = history_partition(
            source.lineage().merge_receipt_cursor().clone(),
            classifications,
            order_capability_id,
        );
        let guard = PostMergeCommitGuardAuthority::from_authoritative_consumed_lineage(
            source,
            &mut TestInitialHistoryPort::exact(partition.clone()),
        )?;
        Ok((guard, partition))
    }

    fn approved_preview_with_history(
        classifications: &[RepositoryHistoryPartitionClassification],
        order_capability_id: &str,
    ) -> (
        ApprovedCommitPreviewAuthority,
        ValidatedRepositoryHistoryPartition,
        Value,
    ) {
        let (guard, partition) =
            initial_guard_with_history(classifications, order_capability_id).unwrap();
        let preview_request = commit_preview_request_value(
            &guard,
            "/original/project",
            "PR-137",
            PREVIEW_OPERATION_ID,
        );
        let preview = CommitPreviewAuthority::from_validated_post_merge_guard(
            validated_commit_preview_request(preview_request.clone()),
            guard,
            validated_comment_policy(),
        )
        .unwrap();
        let approved_digest = preview.commit_digest().clone();
        let apply = serde_json::from_value(commit_apply_request_value(
            preview_request.clone(),
            APPLY_OPERATION_ID,
            &approved_digest,
        ))
        .unwrap();
        let approved = ApprovedCommitPreviewAuthority::from_validated_request(
            preview.validate_apply(apply).unwrap(),
        );
        (approved, partition, preview_request)
    }

    fn approved_preview_with_lock_identity(
        lock_set_id: UnicaId,
        observed_at: NormalizedUtcInstant,
    ) -> (
        ApprovedCommitPreviewAuthority,
        ValidatedRepositoryHistoryPartition,
    ) {
        let source = validated_main_integration_commit_context_with_lock_identity_fixture_test_only(
            id("b3100000-0000-4000-8000-000000000025"),
            digest('9'),
            lock_set_id,
            observed_at,
        );
        let partition = history_partition(
            source.lineage().merge_receipt_cursor().clone(),
            &[RepositoryHistoryPartitionClassification::UnrelatedRoutine],
            "repository.history-order.slice3-foreign-lock-initial",
        );
        let guard = PostMergeCommitGuardAuthority::from_authoritative_consumed_lineage(
            source,
            &mut TestInitialHistoryPort::exact(partition.clone()),
        )
        .unwrap();
        let preview_request = commit_preview_request_value(
            &guard,
            "/original/project",
            "PR-137",
            PREVIEW_OPERATION_ID,
        );
        let preview = CommitPreviewAuthority::from_validated_post_merge_guard(
            validated_commit_preview_request(preview_request.clone()),
            guard,
            validated_comment_policy(),
        )
        .unwrap();
        let approved_digest = preview.commit_digest().clone();
        let apply = serde_json::from_value(commit_apply_request_value(
            preview_request,
            APPLY_OPERATION_ID,
            &approved_digest,
        ))
        .unwrap();
        (
            ApprovedCommitPreviewAuthority::from_validated_request(
                preview.validate_apply(apply).unwrap(),
            ),
            partition,
        )
    }

    #[derive(Default)]
    struct ImmediateLeaseCounters {
        witnesses: Cell<usize>,
        binds: Cell<usize>,
        getters: Cell<usize>,
        drops: Cell<usize>,
    }

    fn target_states_for_exact_objects(
        exact_objects: &CommitExactObjects,
        establishing_version: &RepositoryVersion,
    ) -> RepositoryTargetStates {
        let values = exact_objects
            .iter()
            .map(|exact| match exact {
                CommitExactObjectRef::RootModify => json!({
                    "targetKind": "configurationRoot",
                    "state": "present",
                    "repositoryVersion": establishing_version,
                    "targetFingerprint": digest('6'),
                }),
                CommitExactObjectRef::ObjectAdd { object_id } => json!({
                    "targetKind": "developmentObject",
                    "state": "absent",
                    "objectId": object_id,
                    "absenceEstablishedAtVersion": establishing_version,
                    "expectedAbsent": true,
                }),
                CommitExactObjectRef::ObjectModify { object_id } => json!({
                    "targetKind": "developmentObject",
                    "state": "present",
                    "objectId": object_id,
                    "repositoryVersion": establishing_version,
                    "targetFingerprint": digest('7'),
                }),
                CommitExactObjectRef::ObjectDelete { object_id } => json!({
                    "targetKind": "developmentObject",
                    "state": "present",
                    "objectId": object_id,
                    "repositoryVersion": establishing_version,
                    "targetFingerprint": digest('8'),
                }),
            })
            .collect::<Vec<_>>();
        serde_json::from_value(Value::Array(values)).unwrap()
    }

    struct TestImmediateRecheckLease {
        lineage_witness: CommitSafetyLineageWitness,
        binds: bool,
        bound_plan_digest: Sha256Digest,
        partition: ValidatedRepositoryHistoryPartition,
        recomputed_reference_closure_digest: Sha256Digest,
        observed_original_fingerprint: Sha256Digest,
        observed_repository_anchor: RepositoryAnchor,
        consumed_state_revision: Sha256Digest,
        consumed_state_observation_capability_id: CapabilityRowId,
        original_fingerprint_capability_id: CapabilityRowId,
        root_reread_capability_id: CapabilityRowId,
        atomic_commit_safety_capability_id: CapabilityRowId,
        pre_command_target_states: RepositoryTargetStates,
        pre_command_target_snapshot_observation_capability_id: CapabilityRowId,
        counters: Rc<ImmediateLeaseCounters>,
    }

    impl TestImmediateRecheckLease {
        fn getter(&self) {
            self.counters.getters.set(self.counters.getters.get() + 1);
        }
    }

    impl Drop for TestImmediateRecheckLease {
        fn drop(&mut self) {
            self.counters.drops.set(self.counters.drops.get() + 1);
        }
    }

    impl CommitImmediateRecheckLease for TestImmediateRecheckLease {
        fn commit_safety_lineage_witness(&self) -> &CommitSafetyLineageWitness {
            self.counters
                .witnesses
                .set(self.counters.witnesses.get() + 1);
            &self.lineage_witness
        }

        fn binds(&self, request: &CommitImmediateRecheckRequest<'_>) -> bool {
            self.counters.binds.set(self.counters.binds.get() + 1);
            let _complete_immediate_scope = (
                request.session_id(),
                request.resolved_session_digest(),
                request.plan_id(),
                request.plan_digest(),
                request.merge_receipt_cursor(),
                request.support_gate_history_evidence(),
                request.expected_support_gate_history_evidence_digest(),
                request.lock_plan(),
                request.integration_entries(),
                request.planned_locks(),
                request.journaled_lock_receipts(),
                request.rollback_checkpoint_id(),
                request.exact_objects(),
                request.guard_locks(),
                request.validated_preview_request(),
                request.validated_apply_request(),
            );
            self.binds && &self.bound_plan_digest == request.plan_digest()
        }

        fn history_partition(&self) -> &ValidatedRepositoryHistoryPartition {
            self.getter();
            &self.partition
        }

        fn recomputed_reference_closure_digest(&self) -> &Sha256Digest {
            self.getter();
            &self.recomputed_reference_closure_digest
        }

        fn observed_original_fingerprint(&self) -> &Sha256Digest {
            self.getter();
            &self.observed_original_fingerprint
        }

        fn observed_repository_anchor(&self) -> &RepositoryAnchor {
            self.getter();
            &self.observed_repository_anchor
        }

        fn consumed_state_revision(&self) -> &Sha256Digest {
            self.getter();
            &self.consumed_state_revision
        }

        fn consumed_state_observation_capability_id(&self) -> &CapabilityRowId {
            self.getter();
            &self.consumed_state_observation_capability_id
        }

        fn original_fingerprint_capability_id(&self) -> &CapabilityRowId {
            self.getter();
            &self.original_fingerprint_capability_id
        }

        fn root_reread_capability_id(&self) -> &CapabilityRowId {
            self.getter();
            &self.root_reread_capability_id
        }

        fn atomic_commit_safety_capability_id(&self) -> &CapabilityRowId {
            self.getter();
            &self.atomic_commit_safety_capability_id
        }

        fn pre_command_target_states(&self) -> &RepositoryTargetStates {
            self.getter();
            &self.pre_command_target_states
        }

        fn pre_command_target_snapshot_observation_capability_id(&self) -> &CapabilityRowId {
            self.getter();
            &self.pre_command_target_snapshot_observation_capability_id
        }

        fn commit_exact_once(
            self: Box<Self>,
            _request: CommitAtomicCommitRequest<'_>,
        ) -> Result<CommitAtomicCommitCompletion, RepositoryResultContractError> {
            Err(RepositoryResultContractError(
                "test immediate lease has no atomic commit script",
            ))
        }
    }

    #[derive(Default)]
    struct ImmediateObservationOverrides {
        binds: Option<bool>,
        closure: Option<Sha256Digest>,
        fingerprint: Option<Sha256Digest>,
        repository_identity: Option<Sha256Digest>,
        configuration_identity: Option<ConfigurationIdentity>,
        anchor_fingerprint: Option<Sha256Digest>,
        consumed_revision: Option<Sha256Digest>,
        consumed_capability: Option<CapabilityRowId>,
        fingerprint_capability: Option<CapabilityRowId>,
        root_capability: Option<CapabilityRowId>,
        atomic_capability: Option<CapabilityRowId>,
        pre_command_target_states: Option<RepositoryTargetStates>,
        pre_command_target_snapshot_capability: Option<CapabilityRowId>,
        binding_plan_digest: Option<Sha256Digest>,
    }

    fn immediate_recheck_lease(
        request: &CommitImmediateRecheckRequest<'_>,
        partition: ValidatedRepositoryHistoryPartition,
        overrides: &ImmediateObservationOverrides,
        counters: Rc<ImmediateLeaseCounters>,
    ) -> Result<TestImmediateRecheckLease, RepositoryResultContractError> {
        let retained_anchor = request.retained_post_merge_repository_anchor();
        let observed_original_fingerprint = overrides
            .fingerprint
            .clone()
            .unwrap_or_else(|| request.authorized_post_merge_fingerprint().clone());
        let observed_repository_anchor = request.observe_repository_anchor(
            &partition,
            overrides
                .repository_identity
                .clone()
                .unwrap_or_else(|| retained_anchor.repository_identity().clone()),
            overrides
                .configuration_identity
                .clone()
                .unwrap_or_else(|| retained_anchor.configuration_identity().clone()),
            overrides
                .anchor_fingerprint
                .clone()
                .unwrap_or_else(|| retained_anchor.configuration_fingerprint().clone()),
        )?;
        let pre_command_target_states =
            overrides
                .pre_command_target_states
                .clone()
                .unwrap_or_else(|| {
                    target_states_for_exact_objects(
                        request.exact_objects(),
                        partition.through_inclusive().through_version(),
                    )
                });
        Ok(TestImmediateRecheckLease {
            lineage_witness: request.commit_safety_lineage_witness(),
            binds: overrides.binds.unwrap_or(true),
            bound_plan_digest: overrides
                .binding_plan_digest
                .clone()
                .unwrap_or_else(|| request.plan_digest().clone()),
            partition,
            recomputed_reference_closure_digest: overrides
                .closure
                .clone()
                .unwrap_or_else(|| request.expected_reference_closure_digest().clone()),
            observed_original_fingerprint,
            observed_repository_anchor,
            consumed_state_revision: overrides
                .consumed_revision
                .clone()
                .unwrap_or_else(|| request.consumed_state_revision().clone()),
            consumed_state_observation_capability_id: overrides
                .consumed_capability
                .clone()
                .unwrap_or_else(|| request.consumed_state_observation_capability_id().clone()),
            original_fingerprint_capability_id: overrides
                .fingerprint_capability
                .clone()
                .unwrap_or_else(|| request.original_fingerprint_capability_id().clone()),
            root_reread_capability_id: overrides
                .root_capability
                .clone()
                .unwrap_or_else(|| request.root_reread_capability_id().clone()),
            atomic_commit_safety_capability_id: overrides
                .atomic_capability
                .clone()
                .unwrap_or_else(|| request.atomic_commit_safety_capability_id().clone()),
            pre_command_target_states,
            pre_command_target_snapshot_observation_capability_id: overrides
                .pre_command_target_snapshot_capability
                .clone()
                .unwrap_or_else(|| {
                    CapabilityRowId::parse("repository.commit-target-snapshot.immediate").unwrap()
                }),
            counters,
        })
    }

    fn complete_immediate_recheck(
        request: CommitImmediateRecheckRequest<'_>,
        partition: ValidatedRepositoryHistoryPartition,
        overrides: &ImmediateObservationOverrides,
        counters: Rc<ImmediateLeaseCounters>,
    ) -> Result<CommitImmediateRecheckCompletion, RepositoryResultContractError> {
        let lease = immediate_recheck_lease(&request, partition, overrides, counters)?;
        Ok(request.complete(Box::new(lease)))
    }

    struct TestImmediateRecheckPort {
        partition: Option<ValidatedRepositoryHistoryPartition>,
        overrides: ImmediateObservationOverrides,
        counters: Rc<ImmediateLeaseCounters>,
        port_error: bool,
    }

    impl TestImmediateRecheckPort {
        fn exact(partition: ValidatedRepositoryHistoryPartition) -> Self {
            Self {
                partition: Some(partition),
                overrides: ImmediateObservationOverrides::default(),
                counters: Rc::new(ImmediateLeaseCounters::default()),
                port_error: false,
            }
        }
    }

    impl CommitImmediateRecheckPort for TestImmediateRecheckPort {
        fn recheck_before_commit_intent(
            &mut self,
            request: CommitImmediateRecheckRequest<'_>,
        ) -> Result<CommitImmediateRecheckCompletion, RepositoryResultContractError> {
            if self.port_error {
                return Err(RepositoryResultContractError(
                    "immediate commit recheck adapter failed",
                ));
            }
            complete_immediate_recheck(
                request,
                self.partition.take().unwrap(),
                &self.overrides,
                Rc::clone(&self.counters),
            )
        }
    }

    struct ReplayImmediateRecheckPort {
        stored_completion: Option<CommitImmediateRecheckCompletion>,
        first_partition: Option<ValidatedRepositoryHistoryPartition>,
        counters: Rc<ImmediateLeaseCounters>,
    }

    impl CommitImmediateRecheckPort for ReplayImmediateRecheckPort {
        fn recheck_before_commit_intent(
            &mut self,
            request: CommitImmediateRecheckRequest<'_>,
        ) -> Result<CommitImmediateRecheckCompletion, RepositoryResultContractError> {
            if self.stored_completion.is_none() {
                self.stored_completion = Some(complete_immediate_recheck(
                    request,
                    self.first_partition.take().unwrap(),
                    &ImmediateObservationOverrides::default(),
                    Rc::clone(&self.counters),
                )?);
                return Err(RepositoryResultContractError(
                    "immediate recheck response was lost after completion",
                ));
            }
            self.stored_completion
                .take()
                .ok_or(RepositoryResultContractError(
                    "stored immediate completion already replayed",
                ))
        }
    }

    struct CachedForeignImmediateLeasePort {
        cached_lease: Option<Box<dyn CommitImmediateRecheckLease>>,
        first_partition: Option<ValidatedRepositoryHistoryPartition>,
        counters: Rc<ImmediateLeaseCounters>,
    }

    impl CommitImmediateRecheckPort for CachedForeignImmediateLeasePort {
        fn recheck_before_commit_intent(
            &mut self,
            request: CommitImmediateRecheckRequest<'_>,
        ) -> Result<CommitImmediateRecheckCompletion, RepositoryResultContractError> {
            if self.cached_lease.is_none() {
                let lease = immediate_recheck_lease(
                    &request,
                    self.first_partition.take().unwrap(),
                    &ImmediateObservationOverrides::default(),
                    Rc::clone(&self.counters),
                )?;
                self.cached_lease = Some(Box::new(lease));
                return Err(RepositoryResultContractError(
                    "foreign immediate lease cached before completion",
                ));
            }
            Ok(request.complete(self.cached_lease.take().unwrap()))
        }
    }

    struct CountingInitialGuardLease {
        inner: TestPostMergeGuardLease,
        counters: Rc<ImmediateLeaseCounters>,
    }

    impl Drop for CountingInitialGuardLease {
        fn drop(&mut self) {
            self.counters.drops.set(self.counters.drops.get() + 1);
        }
    }

    impl PostMergeCommitGuardLease for CountingInitialGuardLease {
        fn commit_safety_lineage_witness(&self) -> &CommitSafetyLineageWitness {
            self.counters
                .witnesses
                .set(self.counters.witnesses.get() + 1);
            self.inner.commit_safety_lineage_witness()
        }

        fn binds(&self, request: &PostMergeCommitGuardRequest<'_>) -> bool {
            self.counters.binds.set(self.counters.binds.get() + 1);
            self.inner.binds(request)
        }

        fn history_guard_evidence(&self) -> &PostMergeHistoryGuardEvidence {
            self.counters.getters.set(self.counters.getters.get() + 1);
            self.inner.history_guard_evidence()
        }

        fn observed_original_fingerprint(&self) -> &Sha256Digest {
            self.counters.getters.set(self.counters.getters.get() + 1);
            self.inner.observed_original_fingerprint()
        }

        fn observed_repository_anchor(&self) -> &RepositoryAnchor {
            self.counters.getters.set(self.counters.getters.get() + 1);
            self.inner.observed_repository_anchor()
        }

        fn original_fingerprint_capability_id(&self) -> &CapabilityRowId {
            self.counters.getters.set(self.counters.getters.get() + 1);
            self.inner.original_fingerprint_capability_id()
        }
    }

    struct ReplayInitialGuardPort {
        stored_completion: Option<PostMergeCommitGuardCompletion>,
        counters: Rc<ImmediateLeaseCounters>,
    }

    impl PostMergeCommitGuardPort for ReplayInitialGuardPort {
        fn observe_post_merge_commit_guard(
            &mut self,
            request: PostMergeCommitGuardRequest<'_>,
        ) -> Result<PostMergeCommitGuardCompletion, RepositoryResultContractError> {
            if self.stored_completion.is_none() {
                let (_, history) = empty_commit_history_evidence_fixture_test_only(
                    request.merge_receipt_cursor().clone(),
                    digest('d'),
                    request.reference_closure_digest().clone(),
                    CapabilityRowId::parse(ATOMIC_COMMIT_CAPABILITY_ID).unwrap(),
                )
                .map_err(|_| RepositoryResultContractError("history fixture failed"))?;
                let fingerprint = request.authorized_result_fingerprint().clone();
                let root = request.expected_plan_root_anchor();
                let anchor = request.observe_repository_anchor(
                    &history,
                    root.repository_identity().clone(),
                    root.configuration_identity().clone(),
                    fingerprint.clone(),
                )?;
                let lineage_witness = request.commit_safety_lineage_witness();
                self.stored_completion = Some(
                    request.complete(Box::new(CountingInitialGuardLease {
                        inner: TestPostMergeGuardLease {
                            lineage_witness,
                            history,
                            observed_fingerprint: fingerprint,
                            observed_repository_anchor: anchor,
                            capability_id: CapabilityRowId::parse(
                                "repository.original-fingerprint.gate-b2",
                            )
                            .unwrap(),
                            binds: true,
                        },
                        counters: Rc::clone(&self.counters),
                    })),
                );
                return Err(RepositoryResultContractError(
                    "initial guard response was lost after completion",
                ));
            }
            self.stored_completion
                .take()
                .ok_or(RepositoryResultContractError(
                    "stored initial completion already replayed",
                ))
        }
    }

    struct CachedForeignInitialLeasePort {
        cached_lease: Option<Box<dyn PostMergeCommitGuardLease>>,
        counters: Rc<ImmediateLeaseCounters>,
    }

    impl PostMergeCommitGuardPort for CachedForeignInitialLeasePort {
        fn observe_post_merge_commit_guard(
            &mut self,
            request: PostMergeCommitGuardRequest<'_>,
        ) -> Result<PostMergeCommitGuardCompletion, RepositoryResultContractError> {
            if self.cached_lease.is_none() {
                let (_, history) = empty_commit_history_evidence_fixture_test_only(
                    request.merge_receipt_cursor().clone(),
                    digest('d'),
                    request.reference_closure_digest().clone(),
                    CapabilityRowId::parse(ATOMIC_COMMIT_CAPABILITY_ID).unwrap(),
                )
                .map_err(|_| RepositoryResultContractError("history fixture failed"))?;
                let fingerprint = request.authorized_result_fingerprint().clone();
                let root = request.expected_plan_root_anchor();
                let anchor = request.observe_repository_anchor(
                    &history,
                    root.repository_identity().clone(),
                    root.configuration_identity().clone(),
                    fingerprint.clone(),
                )?;
                let lease = CountingInitialGuardLease {
                    inner: TestPostMergeGuardLease {
                        lineage_witness: request.commit_safety_lineage_witness(),
                        history,
                        observed_fingerprint: fingerprint,
                        observed_repository_anchor: anchor,
                        capability_id: CapabilityRowId::parse(
                            "repository.original-fingerprint.gate-b2",
                        )
                        .unwrap(),
                        binds: true,
                    },
                    counters: Rc::clone(&self.counters),
                };
                self.cached_lease = Some(Box::new(lease));
                return Err(RepositoryResultContractError(
                    "foreign initial lease cached before completion",
                ));
            }
            Ok(request.complete(self.cached_lease.take().unwrap()))
        }
    }

    struct InitialBindingFailurePort {
        counters: Rc<ImmediateLeaseCounters>,
    }

    impl PostMergeCommitGuardPort for InitialBindingFailurePort {
        fn observe_post_merge_commit_guard(
            &mut self,
            request: PostMergeCommitGuardRequest<'_>,
        ) -> Result<PostMergeCommitGuardCompletion, RepositoryResultContractError> {
            let (_, history) = empty_commit_history_evidence_fixture_test_only(
                request.merge_receipt_cursor().clone(),
                digest('d'),
                request.reference_closure_digest().clone(),
                CapabilityRowId::parse(ATOMIC_COMMIT_CAPABILITY_ID).unwrap(),
            )
            .map_err(|_| RepositoryResultContractError("history fixture failed"))?;
            let fingerprint = request.authorized_result_fingerprint().clone();
            let root = request.expected_plan_root_anchor();
            let anchor = request.observe_repository_anchor(
                &history,
                root.repository_identity().clone(),
                root.configuration_identity().clone(),
                fingerprint.clone(),
            )?;
            let lineage_witness = request.commit_safety_lineage_witness();
            Ok(request.complete(Box::new(CountingInitialGuardLease {
                inner: TestPostMergeGuardLease {
                    lineage_witness,
                    history,
                    observed_fingerprint: fingerprint,
                    observed_repository_anchor: anchor,
                    capability_id: CapabilityRowId::parse(
                        "repository.original-fingerprint.gate-b2",
                    )
                    .unwrap(),
                    binds: false,
                },
                counters: Rc::clone(&self.counters),
            })))
        }
    }

    fn unrelated_fresh_outcome() -> (
        CommitFreshPreviewRequiredAuthority,
        Value,
        Rc<ImmediateLeaseCounters>,
    ) {
        let (approved, old_partition, preview_request) = approved_preview_with_history(
            &[RepositoryHistoryPartitionClassification::UnrelatedRoutine],
            "repository.history-order.initial",
        );
        let current = history_partition(
            old_partition.start_cursor().clone(),
            &[
                RepositoryHistoryPartitionClassification::UnrelatedRoutine,
                RepositoryHistoryPartitionClassification::UnrelatedRoutine,
            ],
            "repository.history-order.immediate",
        );
        let mut port = TestImmediateRecheckPort::exact(current);
        let counters = Rc::clone(&port.counters);
        let outcome = approved.recheck_before_commit_intent(&mut port);
        let CommitImmediateRecheckOutcome::FreshPreviewRequired(fresh) = outcome else {
            panic!("strict all-unrelated extension must require a fresh preview")
        };
        (fresh, preview_request, counters)
    }

    struct CommitPreviewFailureExpectation {
        request: Value,
        receipt_id: UnicaId,
        lock_set_id: UnicaId,
        rollback_checkpoint_id: UnicaId,
        journaled_lock_receipts: Vec<JournaledRepositoryLock>,
        consumed_state_revision: Sha256Digest,
        comment_policy_digest: Sha256Digest,
    }

    fn commit_preview_failure_expectation(
        request: &ValidatedRepositoryCommitPreviewRequest,
        guard: &PostMergeCommitGuardAuthority,
        comment_policy: &ValidatedCommitCommentPolicyAuthority,
    ) -> CommitPreviewFailureExpectation {
        CommitPreviewFailureExpectation {
            request: serde_json::to_value(request.request()).unwrap(),
            receipt_id: guard.source.lineage().merge_receipt_id().clone(),
            lock_set_id: guard.source.lineage().lock_set_id().clone(),
            rollback_checkpoint_id: guard.source.lineage().rollback_checkpoint_id().clone(),
            journaled_lock_receipts: guard.source.lineage().journaled_lock_receipts().to_vec(),
            consumed_state_revision: guard
                .source
                .consumed_gate_observation()
                .consumed_state_revision()
                .clone(),
            comment_policy_digest: comment_policy.policy_digest().clone(),
        }
    }

    fn assert_commit_preview_failure_retains_both_inputs(
        blocked: Box<CommitPreviewBlockedAuthority>,
        expected: CommitPreviewFailureExpectation,
        expected_failure: CommitPreviewFailureEvidence,
    ) {
        let (request, guard, comment_policy, failure) = blocked.into_recovery_parts();
        assert_eq!(failure, expected_failure);
        assert_eq!(
            serde_json::to_value(request.into_request()).unwrap(),
            expected.request
        );
        assert_eq!(
            guard.source.lineage().merge_receipt_id(),
            &expected.receipt_id
        );
        assert_eq!(guard.source.lineage().lock_set_id(), &expected.lock_set_id);
        assert_eq!(
            guard.source.lineage().rollback_checkpoint_id(),
            &expected.rollback_checkpoint_id
        );
        assert_eq!(
            guard.source.lineage().journaled_lock_receipts(),
            expected.journaled_lock_receipts
        );
        assert_eq!(
            guard
                .source
                .consumed_gate_observation()
                .consumed_state_revision(),
            &expected.consumed_state_revision
        );
        assert_eq!(
            comment_policy.policy_digest(),
            &expected.comment_policy_digest
        );
    }

    #[test]
    fn commit_comment_revalidation_failure_retains_frozen_authority() {
        let frozen = FrozenCommitCommentPolicyAuthority::from_task_start_renderer_adapter(
            Comment::parse("{taskId}: {summary}").unwrap(),
            TaskId::parse("PR-137").unwrap(),
            TaskSummary::parse("Consumed gate B2").unwrap(),
            ProjectId::parse("b2100000-0000-4000-8000-000000000001").unwrap(),
            Comment::parse("PR-137: Consumed gate B2").unwrap(),
            CapabilityRowId::parse("profile.commit-comment.gate-b2").unwrap(),
        )
        .unwrap();
        let expected_frozen_digest = frozen.policy_digest.clone();
        let expected_frozen_record = frozen.record.clone();
        let expected_frozen_capability = frozen.renderer_capability_id.clone();

        let blocked = ValidatedCommitCommentPolicyAuthority::revalidate(
            frozen,
            Comment::parse("{taskId}: {summary}").unwrap(),
            TaskId::parse("PR-137").unwrap(),
            TaskSummary::parse("Changed summary").unwrap(),
            ProjectId::parse("b2100000-0000-4000-8000-000000000001").unwrap(),
            Comment::parse("PR-137: Changed summary").unwrap(),
            CapabilityRowId::parse("profile.commit-comment.gate-b2").unwrap(),
        )
        .unwrap_err();
        let (retained_frozen, candidate_record, candidate_capability, failure) =
            blocked.into_recovery_parts();
        assert!(matches!(
            failure,
            CommitCommentPolicyRevalidationFailureEvidence::FrozenPolicyMismatch
        ));
        assert_eq!(retained_frozen.policy_digest, expected_frozen_digest);
        assert_eq!(retained_frozen.record, expected_frozen_record);
        assert_eq!(
            retained_frozen.renderer_capability_id,
            expected_frozen_capability
        );
        assert_eq!(candidate_record.task_summary.as_str(), "Changed summary");
        assert_eq!(
            candidate_capability,
            CapabilityRowId::parse("profile.commit-comment.gate-b2").unwrap()
        );
    }

    #[test]
    fn commit_preview_digest_failures_retain_guard_and_comment_policy() {
        for fail_exact_objects_digest in [true, false] {
            let guard = validated_guard(id("b2100000-0000-4000-8000-000000000014"), digest('9'));
            let comment_policy = validated_comment_policy();
            let request_value = commit_preview_request_value(
                &guard,
                "/original/project",
                "PR-137",
                PREVIEW_OPERATION_ID,
            );
            let request = validated_commit_preview_request(request_value);
            let expected = commit_preview_failure_expectation(&request, &guard, &comment_policy);
            let blocked = CommitPreviewAuthority::from_validated_post_merge_guard_using_digests(
                request,
                guard,
                comment_policy,
                |record| {
                    if fail_exact_objects_digest {
                        Err(RepositoryResultContractError(
                            "forced exact-object digest failure",
                        ))
                    } else {
                        result_digest(record, "exact commit-object digest failed")
                    }
                },
                |record| {
                    if fail_exact_objects_digest {
                        result_digest(record, "commit preview digest failed")
                    } else {
                        Err(RepositoryResultContractError(
                            "forced commit-preview digest failure",
                        ))
                    }
                },
            )
            .unwrap_err();
            let expected_failure = if fail_exact_objects_digest {
                CommitPreviewFailureEvidence::ExactObjectsDigest(RepositoryResultContractError(
                    "forced exact-object digest failure",
                ))
            } else {
                CommitPreviewFailureEvidence::CommitDigest(RepositoryResultContractError(
                    "forced commit-preview digest failure",
                ))
            };
            assert_commit_preview_failure_retains_both_inputs(blocked, expected, expected_failure);
        }
    }

    #[test]
    fn commit_preview_consumes_persisted_consumed_gate_not_current_gate() {
        let source = context(id("b2100000-0000-4000-8000-000000000010"), digest('7'));
        let expected_revision = source
            .consumed_gate_observation()
            .consumed_state_revision()
            .clone();
        let guard = PostMergeCommitGuardAuthority::from_authoritative_consumed_lineage(
            source,
            &mut TestPostMergeGuardPort {
                mismatch: GuardMismatch::None,
            },
        )
        .unwrap();
        let request = validated_commit_preview_request(commit_preview_request_value(
            &guard,
            "/original/project",
            "PR-137",
            PREVIEW_OPERATION_ID,
        ));
        let preview = CommitPreviewAuthority::from_validated_post_merge_guard(
            request,
            guard,
            validated_comment_policy(),
        )
        .unwrap();
        assert!(preview.has_persisted_consumed_gate_lineage());
        assert_eq!(
            preview.consumed_gate_observation_revision(),
            &expected_revision
        );
        assert_eq!(
            preview.record.consumed_support_gate_digest,
            *preview.validated_consumed_gate_digest()
        );
    }

    #[test]
    fn gate_b3_commit_preview_retains_exact_validated_request_and_rejects_spliced_lineage() {
        let substitutions = [
            ("taskId", json!("PR-138")),
            ("integrationSetId", json!(OTHER_LINEAGE_ID)),
            ("expectedIntegrationSetDigest", json!(digest('1'))),
            ("lockSetId", json!(OTHER_LINEAGE_ID)),
            ("expectedLockSetDigest", json!(digest('2'))),
            ("verificationId", json!(OTHER_LINEAGE_ID)),
            ("expectedVerificationDigest", json!(digest('3'))),
            ("mergeReceiptId", json!(OTHER_LINEAGE_ID)),
            ("supportGateId", json!(OTHER_LINEAGE_ID)),
            ("expectedSupportGateDigest", json!(digest('4'))),
            (
                "expectedSupportGateHistoryEvidenceDigest",
                json!(digest('5')),
            ),
            ("expectedAuthorizedPostMergeFingerprint", json!(digest('6'))),
        ];

        for (field, replacement) in substitutions {
            let guard = validated_guard(id("b3100000-0000-4000-8000-000000000014"), digest('9'));
            let mut request_value = commit_preview_request_value(
                &guard,
                "/original/project",
                "PR-137",
                PREVIEW_OPERATION_ID,
            );
            request_value
                .as_object_mut()
                .unwrap()
                .insert(field.to_owned(), replacement);
            let request = validated_commit_preview_request(request_value.clone());
            let blocked = CommitPreviewAuthority::from_validated_post_merge_guard(
                request,
                guard,
                validated_comment_policy(),
            )
            .unwrap_err();
            assert!(matches!(
                blocked.failure(),
                CommitPreviewFailureEvidence::RequestLineageMismatch
            ));
            let (request, retained_guard, retained_policy, failure) = blocked.into_recovery_parts();
            assert_eq!(
                serde_json::to_value(request.into_request()).unwrap(),
                request_value,
                "request field {field} was not retained"
            );
            assert_eq!(
                retained_guard.source.lineage().merge_receipt_id(),
                &id("b3100000-0000-4000-8000-000000000014")
            );
            assert_eq!(
                retained_policy.rendered_comment().as_str(),
                "PR-137: Consumed gate B2"
            );
            assert!(matches!(
                failure,
                CommitPreviewFailureEvidence::RequestLineageMismatch
            ));
        }

        let (preview, request_value) = validated_preview_authority();
        assert_eq!(
            serde_json::to_value(preview.validated_preview_request().request()).unwrap(),
            request_value
        );
        assert_eq!(
            preview.validated_preview_request().operation_id(),
            &OperationId::parse(PREVIEW_OPERATION_ID).unwrap()
        );
        let retained_policy = preview.validated_comment_policy();
        assert_eq!(
            retained_policy.record.template.as_str(),
            "{taskId}: {summary}"
        );
        assert_eq!(retained_policy.record.task_id.as_str(), "PR-137");
        assert_eq!(
            retained_policy.record.task_summary.as_str(),
            "Consumed gate B2"
        );
        assert_eq!(
            retained_policy.record.project_id,
            ProjectId::parse("b2100000-0000-4000-8000-000000000001").unwrap()
        );
        assert_eq!(
            retained_policy.renderer_capability_id,
            CapabilityRowId::parse("profile.commit-comment.gate-b2").unwrap()
        );
    }

    #[test]
    fn gate_b3_commit_apply_exact_match_mints_approved_authority_once() {
        let (preview, preview_request) = validated_preview_authority();
        let expected_digest = preview.commit_digest().clone();
        let apply: RepositoryCommitRequest = serde_json::from_value(commit_apply_request_value(
            preview_request,
            APPLY_OPERATION_ID,
            &expected_digest,
        ))
        .unwrap();

        let validated = preview
            .validate_apply(apply)
            .expect("exact apply must validate against its exact preview");
        let approved = ApprovedCommitPreviewAuthority::from_validated_request(validated);
        assert_eq!(approved.preview().commit_digest(), &expected_digest);
        assert_eq!(
            approved.validated_apply_request().operation_id(),
            &OperationId::parse(APPLY_OPERATION_ID).unwrap()
        );
    }

    #[test]
    fn gate_b3_commit_apply_rejects_wrong_digest_and_returns_both_owners_for_retry() {
        let (preview, preview_request) = validated_preview_authority();
        let wrong_apply_value =
            commit_apply_request_value(preview_request, APPLY_OPERATION_ID, &digest('e'));
        let apply: RepositoryCommitRequest =
            serde_json::from_value(wrong_apply_value.clone()).unwrap();
        let blocked = preview.validate_apply(apply).unwrap_err();
        assert!(matches!(
            blocked.failure(),
            CommitApplyApprovalFailureEvidence::ApprovedCommitDigestMismatch
        ));
        let (preview, apply, failure) = blocked.into_recovery_parts();
        assert!(matches!(
            failure,
            CommitApplyApprovalFailureEvidence::ApprovedCommitDigestMismatch
        ));
        assert_eq!(serde_json::to_value(&apply).unwrap(), wrong_apply_value);

        let approved_digest = preview.commit_digest().clone();
        let mut corrected = serde_json::to_value(apply).unwrap();
        corrected.as_object_mut().unwrap().insert(
            "approvedCommitDigest".to_owned(),
            serde_json::to_value(&approved_digest).unwrap(),
        );
        let corrected: RepositoryCommitRequest = serde_json::from_value(corrected).unwrap();
        let validated = preview.validate_apply(corrected).unwrap();
        let _approved = ApprovedCommitPreviewAuthority::from_validated_request(validated);
    }

    #[test]
    fn gate_b3_commit_apply_wrong_variant_returns_preview_and_exact_request() {
        let (preview, preview_request) = validated_preview_authority();
        let request: RepositoryCommitRequest =
            serde_json::from_value(preview_request.clone()).unwrap();
        let blocked = preview.validate_apply(request).unwrap_err();
        assert!(matches!(
            blocked.failure(),
            CommitApplyApprovalFailureEvidence::NotApply
        ));
        let (preview, request, failure) = blocked.into_recovery_parts();
        assert_eq!(serde_json::to_value(request).unwrap(), preview_request);
        assert!(!preview.commit_digest().as_str().is_empty());
        assert!(matches!(
            failure,
            CommitApplyApprovalFailureEvidence::NotApply
        ));
    }

    #[test]
    fn gate_b3_commit_apply_rejects_another_production_preview() {
        let (preview_a, _request_a) = validated_preview_authority();
        let guard_b = validated_guard(id("b3100000-0000-4000-8000-000000000015"), digest('8'));
        let request_b = commit_preview_request_value(
            &guard_b,
            "/original/project",
            "PR-137",
            PREVIEW_OPERATION_ID,
        );
        let preview_b = CommitPreviewAuthority::from_validated_post_merge_guard(
            validated_commit_preview_request(request_b.clone()),
            guard_b,
            validated_comment_policy(),
        )
        .unwrap();
        assert_ne!(preview_a.commit_digest(), preview_b.commit_digest());
        let apply_b: RepositoryCommitRequest = serde_json::from_value(commit_apply_request_value(
            request_b,
            APPLY_OPERATION_ID,
            preview_b.commit_digest(),
        ))
        .unwrap();

        let expected_a_digest = preview_a.commit_digest().clone();
        let blocked = preview_a.validate_apply(apply_b).unwrap_err();
        assert!(matches!(
            blocked.failure(),
            CommitApplyApprovalFailureEvidence::RequestLineageMismatch
        ));
        let (retained_a, _request_b, failure) = blocked.into_recovery_parts();
        assert_eq!(retained_a.commit_digest(), &expected_a_digest);
        assert!(matches!(
            failure,
            CommitApplyApprovalFailureEvidence::RequestLineageMismatch
        ));
    }

    #[test]
    fn gate_b3_commit_apply_rejects_cross_task_cwd_preview_and_same_operation() {
        let substitutions = [
            ("cwd", json!("/other/project")),
            ("taskId", json!("PR-138")),
            ("integrationSetId", json!(OTHER_LINEAGE_ID)),
            ("expectedIntegrationSetDigest", json!(digest('1'))),
            ("lockSetId", json!(OTHER_LINEAGE_ID)),
            ("expectedLockSetDigest", json!(digest('2'))),
            ("verificationId", json!(OTHER_LINEAGE_ID)),
            ("expectedVerificationDigest", json!(digest('3'))),
            ("mergeReceiptId", json!(OTHER_LINEAGE_ID)),
            ("supportGateId", json!(OTHER_LINEAGE_ID)),
            ("expectedSupportGateDigest", json!(digest('4'))),
            (
                "expectedSupportGateHistoryEvidenceDigest",
                json!(digest('5')),
            ),
            ("expectedAuthorizedPostMergeFingerprint", json!(digest('6'))),
        ];

        for (field, replacement) in substitutions {
            let (preview, preview_request) = validated_preview_authority();
            let approved_digest = preview.commit_digest().clone();
            let mut apply_value =
                commit_apply_request_value(preview_request, APPLY_OPERATION_ID, &approved_digest);
            apply_value
                .as_object_mut()
                .unwrap()
                .insert(field.to_owned(), replacement);
            let apply: RepositoryCommitRequest =
                serde_json::from_value(apply_value.clone()).unwrap();
            let blocked = preview.validate_apply(apply).unwrap_err();
            assert!(matches!(
                blocked.failure(),
                CommitApplyApprovalFailureEvidence::RequestLineageMismatch
            ));
            let (retained_preview, retained_apply, failure) = blocked.into_recovery_parts();
            assert_eq!(
                serde_json::to_value(retained_apply).unwrap(),
                apply_value,
                "apply field {field} was not retained"
            );
            assert!(!retained_preview.commit_digest().as_str().is_empty());
            assert!(matches!(
                failure,
                CommitApplyApprovalFailureEvidence::RequestLineageMismatch
            ));
        }

        let (preview, preview_request) = validated_preview_authority();
        let approved_digest = preview.commit_digest().clone();
        let same_operation: RepositoryCommitRequest = serde_json::from_value(
            commit_apply_request_value(preview_request, PREVIEW_OPERATION_ID, &approved_digest),
        )
        .unwrap();
        let blocked = preview.validate_apply(same_operation).unwrap_err();
        assert!(matches!(
            blocked.failure(),
            CommitApplyApprovalFailureEvidence::SameOperationId
        ));
        let (_preview, apply, failure) = blocked.into_recovery_parts();
        let apply = apply.into_validated_apply().unwrap();
        assert_eq!(
            apply.operation_id(),
            &OperationId::parse(PREVIEW_OPERATION_ID).unwrap()
        );
        assert!(matches!(
            failure,
            CommitApplyApprovalFailureEvidence::SameOperationId
        ));
    }

    #[test]
    fn gate_b3_commit_immediate_recheck_exact_unchanged_mints_exact_commit_scope() {
        let (approved, initial_partition, _) = approved_preview_with_history(
            &[RepositoryHistoryPartitionClassification::UnrelatedRoutine],
            "repository.history-order.initial",
        );
        let independently_proven_partition = history_partition(
            initial_partition.start_cursor().clone(),
            &[RepositoryHistoryPartitionClassification::UnrelatedRoutine],
            "repository.history-order.independent-immediate",
        );
        assert_ne!(initial_partition, independently_proven_partition);
        assert_eq!(
            serde_json::to_value(&initial_partition).unwrap(),
            serde_json::to_value(&independently_proven_partition).unwrap()
        );
        let expected_cursor = independently_proven_partition.through_inclusive().clone();
        let mut port = TestImmediateRecheckPort::exact(independently_proven_partition);
        let counters = Rc::clone(&port.counters);

        let outcome = approved.recheck_before_commit_intent(&mut port);
        let CommitImmediateRecheckOutcome::Ready(scope) = outcome else {
            panic!("semantic exact history with an independent hidden proof must be ready")
        };
        assert_eq!(scope.before_repository_cursor(), &expected_cursor);
        assert_eq!(
            scope.post_merge_repository_anchor().history_cursor(),
            &expected_cursor
        );
        assert_eq!(counters.binds.get(), 1);
        assert!(counters.getters.get() > 0);
        assert_eq!(counters.drops.get(), 0);
        drop(scope);
        assert_eq!(counters.drops.get(), 1);
    }

    #[test]
    fn gate_b3_commit_immediate_recheck_unrelated_strict_extension_requires_fresh_preview() {
        let (fresh, preview_request, counters) = unrelated_fresh_outcome();
        assert_eq!(
            fresh
                .fresh_history_guard_evidence()
                .classified_through_cursor()
                .through_version(),
            &RepositoryVersion::parse("102").unwrap()
        );
        assert_eq!(counters.drops.get(), 0);

        let mut refreshed = preview_request.clone();
        refreshed["operationId"] = json!(REFRESH_OPERATION_ID);
        let refreshed: RepositoryCommitRequest = serde_json::from_value(refreshed).unwrap();
        let refresh = fresh
            .validate_refresh_preview_request(refreshed)
            .expect("fresh preview must require a distinct validated preview operation");
        assert_eq!(
            refresh.validated_preview_request().operation_id(),
            &OperationId::parse(REFRESH_OPERATION_ID).unwrap()
        );
        assert_eq!(counters.drops.get(), 0);
        drop(refresh);
        assert_eq!(counters.drops.get(), 1);

        for reused_operation in [PREVIEW_OPERATION_ID, APPLY_OPERATION_ID] {
            let (fresh, mut request, _counters) = unrelated_fresh_outcome();
            request["operationId"] = json!(reused_operation);
            let request: RepositoryCommitRequest = serde_json::from_value(request).unwrap();
            assert!(fresh.validate_refresh_preview_request(request).is_err());
        }
    }

    #[test]
    fn gate_b3_commit_immediate_recheck_relevant_referrer_or_support_tail_requires_recovery() {
        for classification in [
            RepositoryHistoryPartitionClassification::RelevantRoutine,
            RepositoryHistoryPartitionClassification::AuthorizedSupport,
            RepositoryHistoryPartitionClassification::ExternalSupport,
            RepositoryHistoryPartitionClassification::PreArmExternal,
            RepositoryHistoryPartitionClassification::Invalid,
            RepositoryHistoryPartitionClassification::Corrective,
            RepositoryHistoryPartitionClassification::TaskCommit,
        ] {
            let (approved, old_partition, _) = approved_preview_with_history(
                &[RepositoryHistoryPartitionClassification::UnrelatedRoutine],
                "repository.history-order.initial",
            );
            let current = history_partition(
                old_partition.start_cursor().clone(),
                &[
                    RepositoryHistoryPartitionClassification::UnrelatedRoutine,
                    classification,
                ],
                "repository.history-order.unsafe-tail",
            );
            let mut port = TestImmediateRecheckPort::exact(current);
            let outcome = approved.recheck_before_commit_intent(&mut port);
            let CommitImmediateRecheckOutcome::RecoveryRequired(recovery) = outcome else {
                panic!("unsafe tail {classification:?} must require recovery")
            };
            assert!(matches!(
                recovery.failure(),
                CommitImmediateRecheckFailureEvidence::UnsafeHistoryAdvance { .. }
            ));
            assert!(!recovery.approved_commit_digest().as_str().is_empty());
        }
    }

    #[test]
    fn gate_b3_commit_immediate_recheck_consumed_gate_revision_or_capability_drift_requires_recovery(
    ) {
        for case in 0..5 {
            let (approved, old_partition, _) = approved_preview_with_history(
                &[RepositoryHistoryPartitionClassification::UnrelatedRoutine],
                "repository.history-order.initial",
            );
            let current = history_partition(
                old_partition.start_cursor().clone(),
                &[RepositoryHistoryPartitionClassification::UnrelatedRoutine],
                "repository.history-order.immediate",
            );
            let mut port = TestImmediateRecheckPort::exact(current);
            match case {
                0 => port.overrides.consumed_revision = Some(digest('1')),
                1 => {
                    port.overrides.consumed_capability =
                        Some(CapabilityRowId::parse("repository.consumed-gate.foreign").unwrap())
                }
                2 => {
                    port.overrides.fingerprint_capability = Some(
                        CapabilityRowId::parse("repository.original-fingerprint.foreign").unwrap(),
                    )
                }
                3 => {
                    port.overrides.root_capability =
                        Some(CapabilityRowId::parse("repository.root-reread.foreign").unwrap())
                }
                4 => {
                    port.overrides.atomic_capability =
                        Some(CapabilityRowId::parse("repository.atomic-commit.foreign").unwrap())
                }
                _ => unreachable!(),
            }
            let outcome = approved.recheck_before_commit_intent(&mut port);
            let CommitImmediateRecheckOutcome::RecoveryRequired(recovery) = outcome else {
                panic!("consumed gate/capability drift case {case} must require recovery")
            };
            assert!(matches!(
                recovery.failure(),
                CommitImmediateRecheckFailureEvidence::ConsumedGateOrCapabilityChanged { .. }
            ));
        }
    }

    #[test]
    fn gate_b3_commit_immediate_recheck_fingerprint_root_or_closure_drift_requires_recovery() {
        for case in 0..4 {
            let (approved, old_partition, _) = approved_preview_with_history(
                &[RepositoryHistoryPartitionClassification::UnrelatedRoutine],
                "repository.history-order.initial",
            );
            let current = history_partition(
                old_partition.start_cursor().clone(),
                &[RepositoryHistoryPartitionClassification::UnrelatedRoutine],
                "repository.history-order.immediate",
            );
            let mut port = TestImmediateRecheckPort::exact(current);
            match case {
                0 => port.overrides.fingerprint = Some(digest('1')),
                1 => port.overrides.repository_identity = Some(digest('2')),
                2 => port.overrides.anchor_fingerprint = Some(digest('3')),
                3 => port.overrides.closure = Some(digest('4')),
                _ => unreachable!(),
            }
            let outcome = approved.recheck_before_commit_intent(&mut port);
            let CommitImmediateRecheckOutcome::RecoveryRequired(recovery) = outcome else {
                panic!("post-merge state drift case {case} must require recovery")
            };
            assert!(matches!(
                recovery.failure(),
                CommitImmediateRecheckFailureEvidence::PostMergeStateChanged { .. }
            ));
        }

        let source = context(id("b3100000-0000-4000-8000-000000000025"), digest('9'));
        let partition = history_partition(
            source.lineage().merge_receipt_cursor().clone(),
            &[RepositoryHistoryPartitionClassification::UnrelatedRoutine],
            "repository.history-order.initial-anchor-mismatch",
        );
        let blocked = PostMergeCommitGuardAuthority::from_authoritative_consumed_lineage(
            source,
            &mut TestInitialHistoryPort {
                partition,
                repository_identity: Some(digest('f')),
                configuration_identity: None,
                configuration_fingerprint: None,
            },
        )
        .unwrap_err();
        assert!(matches!(
            blocked.failure(),
            PostMergeCommitGuardFailureEvidence::PostMergeDrift { .. }
        ));
    }

    #[test]
    fn gate_b3_commit_immediate_recheck_rejects_rewind_gap_reorder_and_prefix_substitution() {
        for case in 0..4 {
            let (approved, old_partition, _) = approved_preview_with_history(
                &[
                    RepositoryHistoryPartitionClassification::UnrelatedRoutine,
                    RepositoryHistoryPartitionClassification::UnrelatedRoutine,
                ],
                "repository.history-order.initial",
            );
            let current = match case {
                0 => history_partition(
                    old_partition.start_cursor().clone(),
                    &[RepositoryHistoryPartitionClassification::UnrelatedRoutine],
                    "repository.history-order.rewind",
                ),
                1 => history_partition(
                    RepositoryHistoryCursor::new(
                        RepositoryVersion::parse("999").unwrap(),
                        digest('f'),
                    ),
                    &[RepositoryHistoryPartitionClassification::UnrelatedRoutine],
                    "repository.history-order.gap",
                ),
                2 => repository_history_partition_fixture_test_only(
                    old_partition.start_cursor().clone(),
                    vec![
                        (
                            RepositoryVersion::parse("102").unwrap(),
                            RepositoryHistoryPartitionClassification::UnrelatedRoutine,
                        ),
                        (
                            RepositoryVersion::parse("101").unwrap(),
                            RepositoryHistoryPartitionClassification::UnrelatedRoutine,
                        ),
                    ],
                    "repository.history-order.reordered",
                    ATOMIC_COMMIT_CAPABILITY_ID,
                )
                .unwrap(),
                3 => history_partition(
                    old_partition.start_cursor().clone(),
                    &[
                        RepositoryHistoryPartitionClassification::RelevantRoutine,
                        RepositoryHistoryPartitionClassification::UnrelatedRoutine,
                    ],
                    "repository.history-order.prefix-substitution",
                ),
                _ => unreachable!(),
            };
            let mut port = TestImmediateRecheckPort::exact(current);
            let outcome = approved.recheck_before_commit_intent(&mut port);
            let CommitImmediateRecheckOutcome::RecoveryRequired(recovery) = outcome else {
                panic!("history shape case {case} must require recovery")
            };
            assert!(matches!(
                recovery.failure(),
                CommitImmediateRecheckFailureEvidence::HistoryLineageChanged { .. }
                    | CommitImmediateRecheckFailureEvidence::UnsafeHistoryAdvance { .. }
            ));
        }
    }

    #[test]
    fn gate_b3_commit_immediate_recheck_unscoped_ncc_never_reaches_ready_or_fresh_preview() {
        let initial = initial_guard_with_history(
            &[RepositoryHistoryPartitionClassification::NonConflictingConcurrent],
            "repository.history-order.initial-ncc",
        )
        .unwrap_err();
        assert!(matches!(
            initial.failure(),
            PostMergeCommitGuardFailureEvidence::UnscopedNonConflictingConcurrent { .. }
        ));

        let (approved, old_partition, _) = approved_preview_with_history(
            &[RepositoryHistoryPartitionClassification::UnrelatedRoutine],
            "repository.history-order.initial",
        );
        let current = history_partition(
            old_partition.start_cursor().clone(),
            &[
                RepositoryHistoryPartitionClassification::UnrelatedRoutine,
                RepositoryHistoryPartitionClassification::NonConflictingConcurrent,
            ],
            "repository.history-order.immediate-ncc",
        );
        let mut port = TestImmediateRecheckPort::exact(current);
        let outcome = approved.recheck_before_commit_intent(&mut port);
        let CommitImmediateRecheckOutcome::RecoveryRequired(recovery) = outcome else {
            panic!("unscoped immediate NCC must not authorize commit or fresh preview")
        };
        assert!(matches!(
            recovery.failure(),
            CommitImmediateRecheckFailureEvidence::UnscopedNonConflictingConcurrent { .. }
        ));
    }

    #[test]
    fn gate_b3_commit_immediate_recheck_stale_completion_replay_is_recovery_before_bind() {
        let (approved_a, old_partition, _) = approved_preview_with_history(
            &[RepositoryHistoryPartitionClassification::UnrelatedRoutine],
            "repository.history-order.initial",
        );
        let (approved_b, _, _) = approved_preview_with_history(
            &[RepositoryHistoryPartitionClassification::UnrelatedRoutine],
            "repository.history-order.initial",
        );
        let counters = Rc::new(ImmediateLeaseCounters::default());
        let mut port = ReplayImmediateRecheckPort {
            stored_completion: None,
            first_partition: Some(history_partition(
                old_partition.start_cursor().clone(),
                &[RepositoryHistoryPartitionClassification::UnrelatedRoutine],
                "repository.history-order.immediate",
            )),
            counters: Rc::clone(&counters),
        };
        let first = approved_a.recheck_before_commit_intent(&mut port);
        assert!(matches!(
            first,
            CommitImmediateRecheckOutcome::RecoveryRequired(_)
        ));
        let second = approved_b.recheck_before_commit_intent(&mut port);
        let CommitImmediateRecheckOutcome::RecoveryRequired(recovery) = second else {
            panic!("replayed completion must require recovery")
        };
        assert!(matches!(
            recovery.failure(),
            CommitImmediateRecheckFailureEvidence::CompletionAttemptMismatch { .. }
        ));
        assert_eq!(counters.witnesses.get(), 0);
        assert_eq!(counters.binds.get(), 0);
        assert_eq!(counters.getters.get(), 0);
        assert_eq!(counters.drops.get(), 0);
        drop(recovery);
        assert_eq!(counters.drops.get(), 1);

        let receipt = id("b3100000-0000-4000-8000-000000000026");
        let initial_counters = Rc::new(ImmediateLeaseCounters::default());
        let mut initial_port = ReplayInitialGuardPort {
            stored_completion: None,
            counters: Rc::clone(&initial_counters),
        };
        let first = PostMergeCommitGuardAuthority::from_authoritative_consumed_lineage(
            context(receipt.clone(), digest('9')),
            &mut initial_port,
        )
        .unwrap_err();
        drop(first);
        let second = PostMergeCommitGuardAuthority::from_authoritative_consumed_lineage(
            context(receipt, digest('9')),
            &mut initial_port,
        )
        .unwrap_err();
        assert!(matches!(
            second.failure(),
            PostMergeCommitGuardFailureEvidence::CompletionAttemptMismatch { .. }
        ));
        assert_eq!(initial_counters.witnesses.get(), 0);
        assert_eq!(initial_counters.binds.get(), 0);
        assert_eq!(initial_counters.getters.get(), 0);
        assert_eq!(initial_counters.drops.get(), 0);
        drop(second);
        assert_eq!(initial_counters.drops.get(), 1);

        let binding_counters = Rc::new(ImmediateLeaseCounters::default());
        let blocked = PostMergeCommitGuardAuthority::from_authoritative_consumed_lineage(
            context(id("b3100000-0000-4000-8000-000000000027"), digest('9')),
            &mut InitialBindingFailurePort {
                counters: Rc::clone(&binding_counters),
            },
        )
        .unwrap_err();
        assert!(matches!(
            blocked.failure(),
            PostMergeCommitGuardFailureEvidence::CapabilityBindingMismatch { .. }
        ));
        assert_eq!(binding_counters.witnesses.get(), 1);
        assert_eq!(binding_counters.binds.get(), 1);
        assert_eq!(binding_counters.getters.get(), 0);
        assert_eq!(binding_counters.drops.get(), 0);
        drop(blocked);
        assert_eq!(binding_counters.drops.get(), 1);
    }

    #[test]
    fn gate_b3_commit_lineage_witness_rejects_current_completion_wrapping_foreign_leases() {
        let receipt = id("b3100000-0000-4000-8000-000000000028");
        let source_a = context(receipt.clone(), digest('9'));
        let source_b = context(receipt, digest('9'));
        assert_eq!(source_a, source_b);
        let initial_counters = Rc::new(ImmediateLeaseCounters::default());
        let mut initial_port = CachedForeignInitialLeasePort {
            cached_lease: None,
            counters: Rc::clone(&initial_counters),
        };

        let first = PostMergeCommitGuardAuthority::from_authoritative_consumed_lineage(
            source_a,
            &mut initial_port,
        )
        .unwrap_err();
        assert!(matches!(
            first.failure(),
            PostMergeCommitGuardFailureEvidence::PortError(_)
        ));
        drop(first);
        assert_eq!(initial_counters.witnesses.get(), 0);
        assert_eq!(initial_counters.binds.get(), 0);
        assert_eq!(initial_counters.getters.get(), 0);
        assert_eq!(initial_counters.drops.get(), 0);

        let blocked = PostMergeCommitGuardAuthority::from_authoritative_consumed_lineage(
            source_b,
            &mut initial_port,
        )
        .unwrap_err();
        assert!(matches!(
            blocked.failure(),
            PostMergeCommitGuardFailureEvidence::CapabilityBindingMismatch { .. }
        ));
        assert_eq!(initial_counters.witnesses.get(), 1);
        assert_eq!(initial_counters.binds.get(), 0);
        assert_eq!(initial_counters.getters.get(), 0);
        assert_eq!(initial_counters.drops.get(), 0);
        drop(blocked);
        assert_eq!(initial_counters.drops.get(), 1);

        let (approved_a, old_partition, _) = approved_preview_with_history(
            &[RepositoryHistoryPartitionClassification::UnrelatedRoutine],
            "repository.history-order.initial",
        );
        let (approved_b, _, _) = approved_preview_with_history(
            &[RepositoryHistoryPartitionClassification::UnrelatedRoutine],
            "repository.history-order.initial",
        );
        assert_eq!(approved_a.preview(), approved_b.preview());
        let immediate_counters = Rc::new(ImmediateLeaseCounters::default());
        let mut immediate_port = CachedForeignImmediateLeasePort {
            cached_lease: None,
            first_partition: Some(history_partition(
                old_partition.start_cursor().clone(),
                &[RepositoryHistoryPartitionClassification::UnrelatedRoutine],
                "repository.history-order.immediate",
            )),
            counters: Rc::clone(&immediate_counters),
        };

        let first = approved_a.recheck_before_commit_intent(&mut immediate_port);
        assert!(matches!(
            first,
            CommitImmediateRecheckOutcome::RecoveryRequired(_)
        ));
        drop(first);
        assert_eq!(immediate_counters.witnesses.get(), 0);
        assert_eq!(immediate_counters.binds.get(), 0);
        assert_eq!(immediate_counters.getters.get(), 0);
        assert_eq!(immediate_counters.drops.get(), 0);

        let second = approved_b.recheck_before_commit_intent(&mut immediate_port);
        let CommitImmediateRecheckOutcome::RecoveryRequired(recovery) = second else {
            panic!("foreign immediate lineage witness must require recovery")
        };
        assert!(matches!(
            recovery.failure(),
            CommitImmediateRecheckFailureEvidence::CapabilityBindingMismatch { .. }
        ));
        assert_eq!(immediate_counters.witnesses.get(), 1);
        assert_eq!(immediate_counters.binds.get(), 0);
        assert_eq!(immediate_counters.getters.get(), 0);
        assert_eq!(immediate_counters.drops.get(), 0);
        drop(recovery);
        assert_eq!(immediate_counters.drops.get(), 1);
    }

    #[test]
    fn gate_b3_commit_immediate_recheck_port_or_lease_failure_retains_source_and_evidence() {
        let (approved, old_partition, _) = approved_preview_with_history(
            &[RepositoryHistoryPartitionClassification::UnrelatedRoutine],
            "repository.history-order.initial",
        );
        let expected_digest = approved.preview().commit_digest().clone();
        let mut port = TestImmediateRecheckPort::exact(history_partition(
            old_partition.start_cursor().clone(),
            &[RepositoryHistoryPartitionClassification::UnrelatedRoutine],
            "repository.history-order.port-error",
        ));
        port.port_error = true;
        let outcome = approved.recheck_before_commit_intent(&mut port);
        let CommitImmediateRecheckOutcome::RecoveryRequired(recovery) = outcome else {
            panic!("port error must require recovery")
        };
        assert_eq!(recovery.approved_commit_digest(), &expected_digest);
        assert!(matches!(
            recovery.failure(),
            CommitImmediateRecheckFailureEvidence::PortError(_)
        ));

        let (approved, old_partition, _) = approved_preview_with_history(
            &[RepositoryHistoryPartitionClassification::UnrelatedRoutine],
            "repository.history-order.initial",
        );
        let mut port = TestImmediateRecheckPort::exact(history_partition(
            old_partition.start_cursor().clone(),
            &[RepositoryHistoryPartitionClassification::UnrelatedRoutine],
            "repository.history-order.bind-failure",
        ));
        port.overrides.binds = Some(false);
        let counters = Rc::clone(&port.counters);
        let outcome = approved.recheck_before_commit_intent(&mut port);
        let CommitImmediateRecheckOutcome::RecoveryRequired(recovery) = outcome else {
            panic!("lease binding failure must require recovery")
        };
        assert!(matches!(
            recovery.failure(),
            CommitImmediateRecheckFailureEvidence::CapabilityBindingMismatch { .. }
        ));
        assert_eq!(counters.witnesses.get(), 1);
        assert_eq!(counters.binds.get(), 1);
        assert_eq!(counters.getters.get(), 0);
        assert_eq!(counters.drops.get(), 0);
        drop(recovery);
        assert_eq!(counters.drops.get(), 1);

        let (approved, old_partition, _) = approved_preview_with_history(
            &[RepositoryHistoryPartitionClassification::UnrelatedRoutine],
            "repository.history-order.initial",
        );
        let mut port = TestImmediateRecheckPort::exact(history_partition(
            old_partition.start_cursor().clone(),
            &[RepositoryHistoryPartitionClassification::UnrelatedRoutine],
            "repository.history-order.cross-plan-bind",
        ));
        port.overrides.binding_plan_digest = Some(digest('f'));
        let counters = Rc::clone(&port.counters);
        let outcome = approved.recheck_before_commit_intent(&mut port);
        let CommitImmediateRecheckOutcome::RecoveryRequired(recovery) = outcome else {
            panic!("cross-plan lease binding must require recovery")
        };
        assert!(matches!(
            recovery.failure(),
            CommitImmediateRecheckFailureEvidence::CapabilityBindingMismatch { .. }
        ));
        assert_eq!(counters.witnesses.get(), 1);
        assert_eq!(counters.binds.get(), 1);
        assert_eq!(counters.getters.get(), 0);
        assert_eq!(counters.drops.get(), 0);
        drop(recovery);
        assert_eq!(counters.drops.get(), 1);
    }

    #[test]
    fn gate_b3_commit_immediate_recheck_contract_is_linear_non_wire_and_request_completes_once() {
        fn downstream_accepts_only_ready_scope(_scope: CommitScopedAtomicSafetyAuthority) {}

        let (approved, initial_partition, _) = approved_preview_with_history(
            &[RepositoryHistoryPartitionClassification::UnrelatedRoutine],
            "repository.history-order.initial",
        );
        let current = history_partition(
            initial_partition.start_cursor().clone(),
            &[RepositoryHistoryPartitionClassification::UnrelatedRoutine],
            "repository.history-order.immediate",
        );
        let mut port = TestImmediateRecheckPort::exact(current);
        let counters = Rc::clone(&port.counters);
        let outcome = approved.recheck_before_commit_intent(&mut port);
        let CommitImmediateRecheckOutcome::Ready(scope) = outcome else {
            panic!("exact recheck must be the sole downstream commit scope")
        };
        assert_eq!(counters.drops.get(), 0);
        downstream_accepts_only_ready_scope(scope);
        assert_eq!(counters.drops.get(), 1);
    }

    #[test]
    fn post_merge_guard_rejects_completed_lease_replay_for_hidden_field_equal_lineage() {
        let receipt = id("b2100000-0000-4000-8000-000000000015");
        let source_a = context(receipt.clone(), digest('7'));
        let source_b = context(receipt.clone(), digest('7'));
        assert_eq!(source_a, source_b);
        let expected_b_lock = source_b.lineage().lock_set_id().clone();
        let expected_b_revision = source_b
            .consumed_gate_observation()
            .consumed_state_revision()
            .clone();
        let mut port = ObserveThenReplayPostMergeGuardPort {
            stored_completion: None,
        };

        assert!(PostMergeCommitGuardAuthority::from_authoritative_consumed_lineage(
            source_a,
            &mut port,
        )
        .is_err());
        let blocked =
            PostMergeCommitGuardAuthority::from_authoritative_consumed_lineage(source_b, &mut port)
                .unwrap_err();
        assert!(matches!(
            blocked.failure(),
            PostMergeCommitGuardFailureEvidence::CompletionAttemptMismatch { .. }
        ));
        let (retained_b, _) = blocked.into_recovery_parts();
        assert_eq!(retained_b.lineage().merge_receipt_id(), &receipt);
        assert_eq!(retained_b.lineage().lock_set_id(), &expected_b_lock);
        assert_eq!(
            retained_b
                .consumed_gate_observation()
                .consumed_state_revision(),
            &expected_b_revision
        );
    }

    #[test]
    fn post_merge_guard_rejects_completed_lease_replay_on_same_input_retry() {
        let receipt = id("b2100000-0000-4000-8000-000000000016");
        let source = context(receipt.clone(), digest('7'));
        let mut port = ObserveThenReplayPostMergeGuardPort {
            stored_completion: None,
        };
        let first =
            PostMergeCommitGuardAuthority::from_authoritative_consumed_lineage(source, &mut port)
                .unwrap_err();
        let (source, _) = first.into_recovery_parts();

        let blocked =
            PostMergeCommitGuardAuthority::from_authoritative_consumed_lineage(source, &mut port)
                .unwrap_err();
        assert!(matches!(
            blocked.failure(),
            PostMergeCommitGuardFailureEvidence::CompletionAttemptMismatch { .. }
        ));
        let (retained, _) = blocked.into_recovery_parts();
        assert_eq!(retained.lineage().merge_receipt_id(), &receipt);
        assert_eq!(
            retained
                .consumed_gate_observation()
                .observation_capability_id(),
            &CapabilityRowId::parse("repository.consumed-gate.commit-fixture").unwrap()
        );
    }

    #[test]
    fn post_merge_guard_failures_retain_commit_locks_rollback_and_fresh_observation() {
        for mismatch in [
            GuardMismatch::Receipt,
            GuardMismatch::Plan,
            GuardMismatch::Lock,
            GuardMismatch::Integration,
            GuardMismatch::ConsumedCapability,
            GuardMismatch::RootRereadCapability,
            GuardMismatch::PortError,
            GuardMismatch::History,
            GuardMismatch::Closure,
            GuardMismatch::Fingerprint,
        ] {
            let source = context(id("b2100000-0000-4000-8000-000000000011"), digest('8'));
            let expected_receipt = source.lineage().merge_receipt_id().clone();
            let expected_lock = source.lineage().lock_set_id().clone();
            let expected_rollback = source.lineage().rollback_checkpoint_id().clone();
            let expected_receipts = source.lineage().journaled_lock_receipts().to_vec();
            let expected_revision = source
                .consumed_gate_observation()
                .consumed_state_revision()
                .clone();
            let expected_observation_capability = source
                .consumed_gate_observation()
                .observation_capability_id()
                .clone();
            let blocked = PostMergeCommitGuardAuthority::from_authoritative_consumed_lineage(
                source,
                &mut TestPostMergeGuardPort { mismatch },
            )
            .unwrap_err();
            match (mismatch, blocked.failure()) {
                (
                    GuardMismatch::Receipt
                    | GuardMismatch::Plan
                    | GuardMismatch::Lock
                    | GuardMismatch::Integration
                    | GuardMismatch::ConsumedCapability
                    | GuardMismatch::RootRereadCapability,
                    PostMergeCommitGuardFailureEvidence::CapabilityBindingMismatch { .. },
                )
                | (GuardMismatch::PortError, PostMergeCommitGuardFailureEvidence::PortError(_))
                | (
                    GuardMismatch::History | GuardMismatch::Closure | GuardMismatch::Fingerprint,
                    PostMergeCommitGuardFailureEvidence::PostMergeDrift { .. },
                ) => {}
                _ => panic!("wrong post-merge guard failure evidence"),
            }
            let (retained, _failure) = blocked.into_recovery_parts();
            assert_eq!(retained.lineage().merge_receipt_id(), &expected_receipt);
            assert_eq!(retained.lineage().lock_set_id(), &expected_lock);
            assert_eq!(
                retained.lineage().rollback_checkpoint_id(),
                &expected_rollback
            );
            assert_eq!(
                retained.lineage().journaled_lock_receipts(),
                expected_receipts
            );
            assert_eq!(
                retained
                    .consumed_gate_observation()
                    .consumed_state_revision(),
                &expected_revision
            );
            assert_eq!(
                retained
                    .consumed_gate_observation()
                    .observation_capability_id(),
                &expected_observation_capability
            );
        }
    }

    assert_not_clone!(RegisteredCommitOperationAuthority);
    assert_not_clone!(CommitEffectIntentRequest<'static>);
    assert_not_clone!(CommitEffectIntentCompletion);
    assert_not_clone!(CommitAtomicCommitRequest<'static>);
    assert_not_clone!(CommitAtomicCommitCompletion);
    assert_not_clone!(CommitPostCommandLockAuthority);
    assert_not_clone!(CommitEffectIntentOutcome);
    assert_not_clone!(CommitExactOnceOutcome);
    assert_not_clone!(CommitCommittedAuthority);
    assert_not_clone!(CommitProvenZeroEffectAuthority);
    assert_not_clone!(CommitZeroEffectCleanupRequiredAuthority);
    assert_not_clone!(CommitZeroEffectReleasedAuthority);
    assert_not_clone!(CommitAmbiguousAuthority);
    assert_not_serialize!(RegisteredCommitOperationAuthority);
    assert_not_serialize!(CommitEffectIntentRequest<'static>);
    assert_not_serialize!(CommitEffectIntentCompletion);
    assert_not_serialize!(CommitAtomicCommitRequest<'static>);
    assert_not_serialize!(CommitAtomicCommitCompletion);
    assert_not_serialize!(CommitPostCommandLockAuthority);
    assert_not_serialize!(CommitEffectIntentOutcome);
    assert_not_serialize!(CommitExactOnceOutcome);
    assert_not_serialize!(CommitCommittedAuthority);
    assert_not_serialize!(CommitProvenZeroEffectAuthority);
    assert_not_serialize!(CommitZeroEffectCleanupRequiredAuthority);
    assert_not_serialize!(CommitZeroEffectReleasedAuthority);
    assert_not_serialize!(CommitAmbiguousAuthority);
    assert_not_deserialize_owned!(RegisteredCommitOperationAuthority);
    assert_not_deserialize_owned!(CommitEffectIntentRequest<'static>);
    assert_not_deserialize_owned!(CommitEffectIntentCompletion);
    assert_not_deserialize_owned!(CommitAtomicCommitRequest<'static>);
    assert_not_deserialize_owned!(CommitAtomicCommitCompletion);
    assert_not_deserialize_owned!(CommitPostCommandLockAuthority);
    assert_not_deserialize_owned!(CommitEffectIntentOutcome);
    assert_not_deserialize_owned!(CommitExactOnceOutcome);
    assert_not_deserialize_owned!(CommitCommittedAuthority);
    assert_not_deserialize_owned!(CommitProvenZeroEffectAuthority);
    assert_not_deserialize_owned!(CommitZeroEffectCleanupRequiredAuthority);
    assert_not_deserialize_owned!(CommitZeroEffectReleasedAuthority);
    assert_not_deserialize_owned!(CommitAmbiguousAuthority);

    #[derive(Default)]
    struct Slice3Counters {
        events: RefCell<Vec<&'static str>>,
        registered_calls: Cell<usize>,
        registered_witnesses: Cell<usize>,
        registered_binds: Cell<usize>,
        registered_getters: Cell<usize>,
        observed_intent_record_digests: RefCell<Vec<Sha256Digest>>,
        observed_intent_record_bytes: RefCell<Vec<Vec<u8>>>,
        observed_intent_typed_records: RefCell<Vec<Value>>,
        intent_calls: Cell<usize>,
        intent_witnesses: Cell<usize>,
        intent_binds: Cell<usize>,
        intent_getters: Cell<usize>,
        command_calls: Cell<usize>,
        physical_effects: Cell<usize>,
        observed_command_comments: RefCell<Vec<Comment>>,
        observed_command_targets: RefCell<Vec<Vec<String>>>,
        observed_command_receipt_ids: RefCell<Vec<UnicaId>>,
        observed_command_before_cursors: RefCell<Vec<RepositoryHistoryCursor>>,
        observed_command_release_objects: RefCell<Vec<CanonicalRepositoryTargets>>,
        observed_command_release_guards: RefCell<Vec<CanonicalRepositoryTargets>>,
        captured_foreign_lock_authority: RefCell<Option<CommitPostCommandLockAuthority>>,
        captured_foreign_zero_observation: RefCell<Option<CommitProvenZeroEffectObservationInput>>,
        atomic_witnesses: Cell<usize>,
        atomic_binds: Cell<usize>,
        atomic_getters: Cell<usize>,
    }

    #[derive(Clone, Copy)]
    enum Slice3RegistrationMode {
        Exact,
        DifferentLeaseDigest,
        BindingOperation,
        BindingInput,
        BindingRecordDigest,
        BindingLeaseDigest,
        AuthoritativeStartAttempt,
        AuthoritativeForeignTask,
        AuthoritativeForeignContainer,
    }

    #[derive(Clone, Copy)]
    enum Slice3IntentMode {
        Written,
        ProvenNotWritten,
        Error,
        ResponseLoss,
        StaleCompletion,
        BindingScope,
        BindingGeneration,
    }

    #[derive(Clone, Copy)]
    enum Slice3AtomicMode {
        Committed,
        PortError,
        ResponseLoss,
        CompletionMismatch,
        WitnessMismatch,
        BindingPreview,
        BindingOperation,
        BindingIntent,
        BindingScope,
        EndpointSubstitution,
        ObjectSubstitution,
        VersionSubstitution,
        DigestSubstitution,
        CapabilitySubstitution,
        IdentitySubstitution,
        ReceiptSubstitution,
        SemanticDigestSubstitution,
        EqualScalarForeignPartition,
        MissingTaskCommit,
        DuplicateTaskCommit,
        GappedHistory,
        UnlockUnknown,
        ReleaseForeignLockSet,
        ReleaseForeignReceipt,
        ReleaseForeignInvocation,
        CaptureLockAuthority,
        SpliceLockAuthority,
        ZeroEffect,
        ZeroBadCertificate,
        ZeroForeignCapability,
        ZeroForeignInvocation,
        ZeroTerminalAnchorSubstitution,
        ZeroIncompleteObjects,
        ZeroStaleObjectVersion,
        ZeroIncompleteInventory,
        ZeroTaskCommitPresent,
        ZeroReleased,
        ZeroReleaseUnknown,
        ZeroForeignLockSet,
        ZeroForeignReceipt,
        ZeroLockForeignInvocation,
        CaptureZeroObservation,
        SpliceZeroObservation,
    }

    #[derive(Clone)]
    struct RegisteredBindingSnapshot {
        operation_id: OperationId,
        scope: OperationScope,
        canonical_input_digest: Sha256Digest,
        record_digest: Sha256Digest,
        lease_digest: Sha256Digest,
    }

    impl RegisteredBindingSnapshot {
        fn exact(request: &CommitRegisteredOperationRequest<'_>) -> Self {
            Self {
                operation_id: request.apply_operation_id().clone(),
                scope: request.operation_scope().clone(),
                canonical_input_digest: request.canonical_input_digest().clone(),
                record_digest: digest('a'),
                lease_digest: digest('b'),
            }
        }

        fn binds(&self, request: &CommitRegisteredOperationRequest<'_>) -> bool {
            self.operation_id == *request.apply_operation_id()
                && self.scope == *request.operation_scope()
                && self.canonical_input_digest == *request.canonical_input_digest()
                && self.record_digest == digest('a')
                && matches!(self.lease_digest.as_str(), value if value == digest('b').as_str() || value == digest('c').as_str())
        }
    }

    struct Slice3RegisteredOperationLease {
        lineage_witness: CommitSafetyLineageWitness,
        binding: RegisteredBindingSnapshot,
        observation: RegisteredCommitOperationObservation,
        counters: Rc<Slice3Counters>,
    }

    impl CommitRegisteredOperationLease for Slice3RegisteredOperationLease {
        fn commit_safety_lineage_witness(&self) -> &CommitSafetyLineageWitness {
            self.counters
                .registered_witnesses
                .set(self.counters.registered_witnesses.get() + 1);
            &self.lineage_witness
        }

        fn binds(&self, request: &CommitRegisteredOperationRequest<'_>) -> bool {
            self.counters
                .registered_binds
                .set(self.counters.registered_binds.get() + 1);
            self.binding.binds(request)
        }

        fn into_current_operation(self: Box<Self>) -> RegisteredCommitOperationObservation {
            self.counters
                .registered_getters
                .set(self.counters.registered_getters.get() + 1);
            self.observation
        }
    }

    struct Slice3RegisteredOperationPort {
        mode: Slice3RegistrationMode,
        counters: Rc<Slice3Counters>,
    }

    impl CommitRegisteredOperationPort for Slice3RegisteredOperationPort {
        fn load_registered_commit_operation(
            &mut self,
            request: CommitRegisteredOperationRequest<'_>,
        ) -> Result<CommitRegisteredOperationCompletion, RepositoryResultContractError> {
            self.counters
                .registered_calls
                .set(self.counters.registered_calls.get() + 1);
            let mut binding = RegisteredBindingSnapshot::exact(&request);
            match self.mode {
                Slice3RegistrationMode::BindingOperation => {
                    binding.operation_id = OperationId::parse(OTHER_LINEAGE_ID).unwrap();
                }
                Slice3RegistrationMode::BindingInput => {
                    binding.canonical_input_digest = digest('f');
                }
                Slice3RegistrationMode::BindingRecordDigest => {
                    binding.record_digest = digest('f');
                }
                Slice3RegistrationMode::BindingLeaseDigest => {
                    binding.lease_digest = digest('f');
                }
                Slice3RegistrationMode::Exact
                | Slice3RegistrationMode::DifferentLeaseDigest
                | Slice3RegistrationMode::AuthoritativeStartAttempt
                | Slice3RegistrationMode::AuthoritativeForeignTask
                | Slice3RegistrationMode::AuthoritativeForeignContainer => {}
            }
            let lease_digest = if matches!(self.mode, Slice3RegistrationMode::DifferentLeaseDigest)
            {
                digest('c')
            } else {
                digest('b')
            };
            if matches!(self.mode, Slice3RegistrationMode::DifferentLeaseDigest) {
                binding.lease_digest = lease_digest.clone();
            }
            let lineage_witness = request.commit_safety_lineage_witness();
            let authoritative_scope = match self.mode {
                Slice3RegistrationMode::AuthoritativeStartAttempt => OperationScope::StartAttempt {
                    workspace_identity_digest: digest('d'),
                    task_id: TaskId::parse("PR-137").unwrap(),
                },
                Slice3RegistrationMode::AuthoritativeForeignTask => OperationScope::Task {
                    project_id: ProjectId::parse("b3300000-0000-4000-8000-000000000001").unwrap(),
                    task_id: TaskId::parse("PR-999").unwrap(),
                    instance_id: id("b3300000-0000-4000-8000-000000000002"),
                },
                Slice3RegistrationMode::AuthoritativeForeignContainer => OperationScope::Task {
                    project_id: ProjectId::parse("b3300000-0000-4000-8000-000000000099").unwrap(),
                    task_id: TaskId::parse("PR-137").unwrap(),
                    instance_id: id("b3300000-0000-4000-8000-000000000098"),
                },
                _ => request.operation_scope().clone(),
            };
            let observation = RegisteredCommitOperationObservation::from_current_record(
                request.apply_operation_id().clone(),
                authoritative_scope,
                request.operation(),
                request.policy(),
                request.canonical_input_digest().clone(),
                RegisteredCommitStorageEvidence::new(
                    PositiveGeneration::new(7).unwrap(),
                    digest('a'),
                    PositiveGeneration::new(11).unwrap(),
                    lease_digest,
                ),
            );
            Ok(request.complete(Box::new(Slice3RegisteredOperationLease {
                lineage_witness,
                binding,
                observation,
                counters: Rc::clone(&self.counters),
            })))
        }
    }

    #[derive(Clone)]
    struct IntentBindingSnapshot {
        operation_id: OperationId,
        scope: OperationScope,
        record_revision: PositiveGeneration,
        lease_generation: PositiveGeneration,
        intent_record_digest: Sha256Digest,
    }

    impl IntentBindingSnapshot {
        fn exact(request: &CommitEffectIntentRequest<'_>) -> Self {
            Self {
                operation_id: request.apply_operation_id().clone(),
                scope: request.operation_scope().clone(),
                record_revision: request.registered_record_revision(),
                lease_generation: request.registered_lease_generation(),
                intent_record_digest: request.intent_record_digest().clone(),
            }
        }

        fn binds(&self, request: &CommitEffectIntentRequest<'_>) -> bool {
            self.operation_id == *request.apply_operation_id()
                && self.scope == *request.operation_scope()
                && self.record_revision == request.registered_record_revision()
                && self.lease_generation == request.registered_lease_generation()
                && self.intent_record_digest == *request.intent_record_digest()
        }
    }

    struct Slice3WrittenIntentLease {
        lineage_witness: CommitSafetyLineageWitness,
        binding: IntentBindingSnapshot,
        evidence: CommitEffectIntentFsyncEvidence,
        counters: Rc<Slice3Counters>,
    }

    impl CommitEffectIntentWrittenLease for Slice3WrittenIntentLease {
        fn commit_safety_lineage_witness(&self) -> &CommitSafetyLineageWitness {
            self.counters
                .intent_witnesses
                .set(self.counters.intent_witnesses.get() + 1);
            &self.lineage_witness
        }

        fn binds(&self, request: &CommitEffectIntentRequest<'_>) -> bool {
            self.counters
                .intent_binds
                .set(self.counters.intent_binds.get() + 1);
            self.binding.binds(request)
        }

        fn into_fsync_evidence(self: Box<Self>) -> CommitEffectIntentFsyncEvidence {
            self.counters
                .intent_getters
                .set(self.counters.intent_getters.get() + 1);
            self.counters.events.borrow_mut().push("intent-fsynced");
            self.evidence
        }
    }

    struct Slice3NotWrittenIntentLease {
        lineage_witness: CommitSafetyLineageWitness,
        binding: IntentBindingSnapshot,
        certificate: CommitEffectIntentNotWrittenCertificate,
        counters: Rc<Slice3Counters>,
    }

    impl CommitEffectIntentProvenNotWrittenLease for Slice3NotWrittenIntentLease {
        fn commit_safety_lineage_witness(&self) -> &CommitSafetyLineageWitness {
            self.counters
                .intent_witnesses
                .set(self.counters.intent_witnesses.get() + 1);
            &self.lineage_witness
        }

        fn binds(&self, request: &CommitEffectIntentRequest<'_>) -> bool {
            self.counters
                .intent_binds
                .set(self.counters.intent_binds.get() + 1);
            self.binding.binds(request)
        }

        fn into_certificate(self: Box<Self>) -> CommitEffectIntentNotWrittenCertificate {
            self.counters
                .intent_getters
                .set(self.counters.intent_getters.get() + 1);
            self.certificate
        }
    }

    struct Slice3IntentPort {
        mode: Slice3IntentMode,
        counters: Rc<Slice3Counters>,
    }

    impl CommitEffectIntentPort for Slice3IntentPort {
        fn write_and_fsync_commit_intent(
            &mut self,
            request: CommitEffectIntentRequest<'_>,
        ) -> Result<CommitEffectIntentCompletion, RepositoryResultContractError> {
            self.counters
                .intent_calls
                .set(self.counters.intent_calls.get() + 1);
            self.counters
                .observed_intent_record_digests
                .borrow_mut()
                .push(request.intent_record_digest().clone());
            self.counters
                .observed_intent_record_bytes
                .borrow_mut()
                .push(request.canonical_record_bytes().to_vec());
            self.counters
                .observed_intent_typed_records
                .borrow_mut()
                .push(serde_json::to_value(request.durable_record()).unwrap());
            self.counters.events.borrow_mut().push("intent-call");
            if matches!(self.mode, Slice3IntentMode::Error) {
                return Err(RepositoryResultContractError("intent adapter error"));
            }
            if matches!(self.mode, Slice3IntentMode::ResponseLoss) {
                self.counters.events.borrow_mut().push("intent-fsynced");
                return Err(RepositoryResultContractError(
                    "intent response lost after fsync",
                ));
            }

            let lineage_witness = request.commit_safety_lineage_witness();
            let mut binding = IntentBindingSnapshot::exact(&request);
            match self.mode {
                Slice3IntentMode::BindingScope => {
                    binding.scope = task_scope("b3300000-0000-4000-8000-000000000099");
                }
                Slice3IntentMode::BindingGeneration => {
                    binding.lease_generation = PositiveGeneration::new(99).unwrap();
                }
                Slice3IntentMode::Written
                | Slice3IntentMode::ProvenNotWritten
                | Slice3IntentMode::StaleCompletion
                | Slice3IntentMode::Error
                | Slice3IntentMode::ResponseLoss => {}
            }

            let mut completion = if matches!(self.mode, Slice3IntentMode::ProvenNotWritten) {
                let certificate = request
                    .observe_proven_not_written(
                        id("b3300000-0000-4000-8000-000000000031"),
                        CapabilityRowId::parse("repository.commit-intent.not-written").unwrap(),
                    )
                    .unwrap();
                request.complete_proven_not_written(Box::new(Slice3NotWrittenIntentLease {
                    lineage_witness,
                    binding,
                    certificate,
                    counters: Rc::clone(&self.counters),
                }))
            } else {
                let evidence = request
                    .observe_fsync(
                        id("b3300000-0000-4000-8000-000000000032"),
                        CapabilityRowId::parse("repository.commit-intent.fsync").unwrap(),
                    )
                    .unwrap();
                request.complete_written(Box::new(Slice3WrittenIntentLease {
                    lineage_witness,
                    binding,
                    evidence,
                    counters: Rc::clone(&self.counters),
                }))
            };
            if matches!(self.mode, Slice3IntentMode::StaleCompletion) {
                completion.completion = CommitEffectIntentInvocationCapability::mint().completion();
            }
            Ok(completion)
        }
    }

    #[derive(Clone)]
    struct AtomicBindingSnapshot {
        approved_commit_digest: Sha256Digest,
        operation_id: OperationId,
        scope: OperationScope,
        intent_digest: Sha256Digest,
    }

    impl AtomicBindingSnapshot {
        fn exact(request: &CommitAtomicCommitRequest<'_>) -> Self {
            Self {
                approved_commit_digest: request.approved_commit_digest().clone(),
                operation_id: request.apply_operation_id().clone(),
                scope: request.operation_scope().clone(),
                intent_digest: request.effect_intent_digest().clone(),
            }
        }

        fn binds(&self, request: &CommitAtomicCommitRequest<'_>) -> bool {
            self.approved_commit_digest == *request.approved_commit_digest()
                && self.operation_id == *request.apply_operation_id()
                && self.scope == *request.operation_scope()
                && self.intent_digest == *request.effect_intent_digest()
        }
    }

    struct Slice3AtomicPayload {
        lineage_witness: CommitSafetyLineageWitness,
        binding: AtomicBindingSnapshot,
        observation: CommitAtomicCommitObservation,
        counters: Rc<Slice3Counters>,
    }

    impl CommitAtomicCommitPayload for Slice3AtomicPayload {
        fn commit_safety_lineage_witness(&self) -> &CommitSafetyLineageWitness {
            self.counters
                .atomic_witnesses
                .set(self.counters.atomic_witnesses.get() + 1);
            &self.lineage_witness
        }

        fn binds(&self, request: &CommitAtomicCommitRequest<'_>) -> bool {
            self.counters
                .atomic_binds
                .set(self.counters.atomic_binds.get() + 1);
            self.binding.binds(request)
        }

        fn into_observation(self: Box<Self>) -> CommitAtomicCommitObservation {
            self.counters
                .atomic_getters
                .set(self.counters.atomic_getters.get() + 1);
            self.observation
        }
    }

    struct Slice3ImmediateLease {
        base: TestImmediateRecheckLease,
        atomic_mode: Slice3AtomicMode,
        counters: Rc<Slice3Counters>,
    }

    impl Slice3ImmediateLease {
        fn run_atomic(
            self: Box<Self>,
            request: CommitAtomicCommitRequest<'_>,
        ) -> Result<CommitAtomicCommitCompletion, RepositoryResultContractError> {
            self.counters
                .command_calls
                .set(self.counters.command_calls.get() + 1);
            self.counters.events.borrow_mut().push("command-call");
            self.counters
                .observed_command_comments
                .borrow_mut()
                .push(request.rendered_comment().clone());
            self.counters.observed_command_targets.borrow_mut().push(
                request
                    .exact_object_refs()
                    .map(|target| match target {
                        CommitExactObjectRef::RootModify => "root:modify".to_owned(),
                        CommitExactObjectRef::ObjectAdd { object_id } => {
                            format!("{}:add", object_id.as_str())
                        }
                        CommitExactObjectRef::ObjectModify { object_id } => {
                            format!("{}:modify", object_id.as_str())
                        }
                        CommitExactObjectRef::ObjectDelete { object_id } => {
                            format!("{}:delete", object_id.as_str())
                        }
                    })
                    .collect(),
            );
            self.counters
                .observed_command_receipt_ids
                .borrow_mut()
                .push(request.preallocated_commit_receipt_id().clone());
            self.counters
                .observed_command_before_cursors
                .borrow_mut()
                .push(request.before_repository_cursor().clone());
            let (release_objects, release_guards) = request.exact_release_projection()?;
            self.counters
                .observed_command_release_objects
                .borrow_mut()
                .push(release_objects);
            self.counters
                .observed_command_release_guards
                .borrow_mut()
                .push(release_guards);
            if matches!(self.atomic_mode, Slice3AtomicMode::PortError) {
                return Err(RepositoryResultContractError("atomic commit error"));
            }
            if matches!(self.atomic_mode, Slice3AtomicMode::ResponseLoss) {
                self.counters
                    .physical_effects
                    .set(self.counters.physical_effects.get() + 1);
                return Err(RepositoryResultContractError(
                    "atomic response lost after physical effect",
                ));
            }

            let mut binding = AtomicBindingSnapshot::exact(&request);
            match self.atomic_mode {
                Slice3AtomicMode::BindingPreview => {
                    binding.approved_commit_digest = digest('f');
                }
                Slice3AtomicMode::BindingOperation => {
                    binding.operation_id = OperationId::parse(OTHER_LINEAGE_ID).unwrap();
                }
                Slice3AtomicMode::BindingIntent => binding.intent_digest = digest('f'),
                Slice3AtomicMode::BindingScope => {
                    binding.scope = task_scope("b3300000-0000-4000-8000-000000000098");
                }
                _ => {}
            }
            let lineage_witness = if matches!(self.atomic_mode, Slice3AtomicMode::WitnessMismatch) {
                CommitSafetyLineageWitness(Arc::new(CommitSafetyLineageMarker))
            } else {
                request.commit_safety_lineage_witness()
            };
            let observation = if matches!(self.atomic_mode, Slice3AtomicMode::CaptureLockAuthority)
            {
                self.counters
                    .captured_foreign_lock_authority
                    .replace(Some(matching_released_lock_authority(&request)?));
                request.observe_ambiguous()
            } else if matches!(self.atomic_mode, Slice3AtomicMode::SpliceLockAuthority) {
                let mut observation = committed_observation(&request, self.atomic_mode)?;
                let CommitAtomicCommitObservation::Committed(committed) = &mut observation else {
                    unreachable!("committed splice fixture must produce committed observation")
                };
                committed.release = CommitLockReleaseObservation::Verified(Box::new(
                    self.counters
                        .captured_foreign_lock_authority
                        .borrow_mut()
                        .take()
                        .expect("foreign committed lock authority must be captured first"),
                ));
                observation
            } else if matches!(self.atomic_mode, Slice3AtomicMode::CaptureZeroObservation) {
                let observation = zero_effect_observation(
                    &request,
                    Slice3AtomicMode::ZeroReleased,
                    self.base.pre_command_target_states.clone(),
                )?;
                let CommitAtomicCommitObservation::ProvenZeroEffect(zero) = observation else {
                    unreachable!("zero capture fixture must produce zero observation")
                };
                self.counters
                    .captured_foreign_zero_observation
                    .replace(Some(*zero));
                request.observe_ambiguous()
            } else if matches!(self.atomic_mode, Slice3AtomicMode::SpliceZeroObservation) {
                request.observe_proven_zero_effect(
                    self.counters
                        .captured_foreign_zero_observation
                        .borrow_mut()
                        .take()
                        .expect("foreign zero observation must be captured first"),
                )
            } else if matches!(
                self.atomic_mode,
                Slice3AtomicMode::ZeroEffect
                    | Slice3AtomicMode::ZeroBadCertificate
                    | Slice3AtomicMode::ZeroForeignCapability
                    | Slice3AtomicMode::ZeroForeignInvocation
                    | Slice3AtomicMode::ZeroTerminalAnchorSubstitution
                    | Slice3AtomicMode::ZeroIncompleteObjects
                    | Slice3AtomicMode::ZeroStaleObjectVersion
                    | Slice3AtomicMode::ZeroIncompleteInventory
                    | Slice3AtomicMode::ZeroTaskCommitPresent
                    | Slice3AtomicMode::ZeroReleased
                    | Slice3AtomicMode::ZeroReleaseUnknown
                    | Slice3AtomicMode::ZeroForeignLockSet
                    | Slice3AtomicMode::ZeroForeignReceipt
                    | Slice3AtomicMode::ZeroLockForeignInvocation
            ) {
                zero_effect_observation(
                    &request,
                    self.atomic_mode,
                    self.base.pre_command_target_states.clone(),
                )?
            } else {
                committed_observation(&request, self.atomic_mode)?
            };
            let payload = Box::new(Slice3AtomicPayload {
                lineage_witness,
                binding,
                observation,
                counters: Rc::clone(&self.counters),
            });
            let mut completion = request.complete(payload);
            if matches!(self.atomic_mode, Slice3AtomicMode::CompletionMismatch) {
                completion.completion = CommitAtomicCommitInvocationCapability::mint().completion();
            }
            Ok(completion)
        }
    }

    impl CommitImmediateRecheckLease for Slice3ImmediateLease {
        fn commit_safety_lineage_witness(&self) -> &CommitSafetyLineageWitness {
            self.base.commit_safety_lineage_witness()
        }

        fn binds(&self, request: &CommitImmediateRecheckRequest<'_>) -> bool {
            self.base.binds(request)
        }

        fn history_partition(&self) -> &ValidatedRepositoryHistoryPartition {
            self.base.history_partition()
        }

        fn recomputed_reference_closure_digest(&self) -> &Sha256Digest {
            self.base.recomputed_reference_closure_digest()
        }

        fn observed_original_fingerprint(&self) -> &Sha256Digest {
            self.base.observed_original_fingerprint()
        }

        fn observed_repository_anchor(&self) -> &RepositoryAnchor {
            self.base.observed_repository_anchor()
        }

        fn consumed_state_revision(&self) -> &Sha256Digest {
            self.base.consumed_state_revision()
        }

        fn consumed_state_observation_capability_id(&self) -> &CapabilityRowId {
            self.base.consumed_state_observation_capability_id()
        }

        fn original_fingerprint_capability_id(&self) -> &CapabilityRowId {
            self.base.original_fingerprint_capability_id()
        }

        fn root_reread_capability_id(&self) -> &CapabilityRowId {
            self.base.root_reread_capability_id()
        }

        fn atomic_commit_safety_capability_id(&self) -> &CapabilityRowId {
            self.base.atomic_commit_safety_capability_id()
        }

        fn pre_command_target_states(&self) -> &RepositoryTargetStates {
            self.base.pre_command_target_states()
        }

        fn pre_command_target_snapshot_observation_capability_id(&self) -> &CapabilityRowId {
            self.base
                .pre_command_target_snapshot_observation_capability_id()
        }

        fn commit_exact_once(
            self: Box<Self>,
            request: CommitAtomicCommitRequest<'_>,
        ) -> Result<CommitAtomicCommitCompletion, RepositoryResultContractError> {
            self.run_atomic(request)
        }
    }

    struct Slice3ImmediatePort {
        partition: Option<ValidatedRepositoryHistoryPartition>,
        atomic_mode: Slice3AtomicMode,
        counters: Rc<Slice3Counters>,
    }

    impl CommitImmediateRecheckPort for Slice3ImmediatePort {
        fn recheck_before_commit_intent(
            &mut self,
            request: CommitImmediateRecheckRequest<'_>,
        ) -> Result<CommitImmediateRecheckCompletion, RepositoryResultContractError> {
            let base = immediate_recheck_lease(
                &request,
                self.partition.take().unwrap(),
                &ImmediateObservationOverrides::default(),
                Rc::new(ImmediateLeaseCounters::default()),
            )?;
            Ok(request.complete(Box::new(Slice3ImmediateLease {
                base,
                atomic_mode: self.atomic_mode,
                counters: Rc::clone(&self.counters),
            })))
        }
    }

    fn task_scope(instance_id: &str) -> OperationScope {
        OperationScope::Task {
            project_id: ProjectId::parse("b2100000-0000-4000-8000-000000000001").unwrap(),
            task_id: TaskId::parse("PR-137").unwrap(),
            instance_id: id(instance_id),
        }
    }

    fn ready_scope(
        atomic_mode: Slice3AtomicMode,
        counters: Rc<Slice3Counters>,
    ) -> CommitScopedAtomicSafetyAuthority {
        let (approved, initial_partition, _) = approved_preview_with_history(
            &[RepositoryHistoryPartitionClassification::UnrelatedRoutine],
            "repository.history-order.slice3-initial",
        );
        let current = history_partition(
            initial_partition.start_cursor().clone(),
            &[RepositoryHistoryPartitionClassification::UnrelatedRoutine],
            "repository.history-order.slice3-immediate",
        );
        let mut port = Slice3ImmediatePort {
            partition: Some(current),
            atomic_mode,
            counters,
        };
        match approved.recheck_before_commit_intent(&mut port) {
            CommitImmediateRecheckOutcome::Ready(scope) => scope,
            _ => panic!("slice3 fixture must reach Ready"),
        }
    }

    fn ready_scope_with_lock_identity(
        atomic_mode: Slice3AtomicMode,
        counters: Rc<Slice3Counters>,
        lock_set_id: UnicaId,
        observed_at: NormalizedUtcInstant,
    ) -> CommitScopedAtomicSafetyAuthority {
        let (approved, initial_partition) =
            approved_preview_with_lock_identity(lock_set_id, observed_at);
        let current = history_partition(
            initial_partition.start_cursor().clone(),
            &[RepositoryHistoryPartitionClassification::UnrelatedRoutine],
            "repository.history-order.slice3-foreign-lock-immediate",
        );
        let mut port = Slice3ImmediatePort {
            partition: Some(current),
            atomic_mode,
            counters,
        };
        match approved.recheck_before_commit_intent(&mut port) {
            CommitImmediateRecheckOutcome::Ready(scope) => scope,
            _ => panic!("foreign lock fixture must reach Ready"),
        }
    }

    fn postcommit_partition(
        start: RepositoryHistoryCursor,
        classifications: &[RepositoryHistoryPartitionClassification],
    ) -> ValidatedRepositoryHistoryPartition {
        repository_history_partition_fixture_test_only(
            start,
            classifications
                .iter()
                .enumerate()
                .map(|(index, classification)| {
                    (
                        RepositoryVersion::parse(&(201 + index).to_string()).unwrap(),
                        *classification,
                    )
                })
                .collect(),
            "repository.history-order.slice3-postcommit",
            ATOMIC_COMMIT_CAPABILITY_ID,
        )
        .unwrap()
    }

    #[derive(Clone)]
    struct Slice3HistoryIndex {
        candidates: BTreeMap<String, EvidenceSourceIndexCandidate>,
    }

    impl EvidenceSourceIndex for Slice3HistoryIndex {
        fn candidate_for(
            &self,
            repository_version: &RepositoryVersion,
            _registry: &EvidenceSourceRegistry,
        ) -> Result<EvidenceSourceIndexCandidate, RepositoryContractError> {
            Ok(self
                .candidates
                .get(repository_version.as_str())
                .cloned()
                .expect("Slice3 resolver must request a known source-index row"))
        }
    }

    #[derive(Clone)]
    struct Slice3HistoryOrder {
        evidence: RepositoryHistoryOrderEvidence,
    }

    impl RepositoryHistoryOrderResolver for Slice3HistoryOrder {
        fn order_evidence(
            &self,
            _from_exclusive: &RepositoryHistoryCursor,
            _through_inclusive: &RepositoryHistoryCursor,
        ) -> Result<RepositoryHistoryOrderEvidence, RepositoryContractError> {
            Ok(self.evidence.clone())
        }
    }

    #[derive(Default)]
    struct Slice3HistoryBytes {
        bytes: BTreeMap<(EvidenceKind, String), Vec<u8>>,
    }

    impl RepositoryHistoryEvidenceBytesResolver for Slice3HistoryBytes {
        fn load_canonical_evidence_bytes(
            &self,
            reference: &RepositoryHistorySourceEvidenceRef,
        ) -> Result<Vec<u8>, RepositoryContractError> {
            Ok(self
                .bytes
                .get(&(
                    reference.evidence_kind(),
                    reference.evidence_digest().as_str().to_owned(),
                ))
                .cloned()
                .expect("Slice3 resolver must request known evidence bytes"))
        }
    }

    fn slice3_json_digest(value: &Value) -> Sha256Digest {
        Sha256Digest::parse(&format!(
            "{:x}",
            Sha256::digest(serde_json_canonicalizer::to_vec(value).unwrap())
        ))
        .unwrap()
    }

    fn slice3_entry_cursor(
        index: usize,
        repository_version: RepositoryVersion,
    ) -> RepositoryHistoryCursor {
        let cursor_character = char::from_digit(((index + 1) % 15 + 1) as u32, 16).unwrap();
        RepositoryHistoryCursor::new(repository_version, digest(cursor_character))
    }

    fn resolve_slice3_task_commit_history(
        request: &CommitAtomicCommitRequest<'_>,
        core: &CommitCommittedCoreObservation,
        mode: Slice3AtomicMode,
        task_version_anchor: RepositoryAnchor,
        terminal_repository_anchor: RepositoryAnchor,
    ) -> Result<CommitCommittedHistoryObservation, RepositoryResultContractError> {
        let classifications = match mode {
            Slice3AtomicMode::MissingTaskCommit => {
                vec![RepositoryHistoryPartitionClassification::UnrelatedRoutine]
            }
            Slice3AtomicMode::DuplicateTaskCommit => vec![
                RepositoryHistoryPartitionClassification::TaskCommit,
                RepositoryHistoryPartitionClassification::TaskCommit,
            ],
            _ => vec![
                RepositoryHistoryPartitionClassification::TaskCommit,
                RepositoryHistoryPartitionClassification::UnrelatedRoutine,
            ],
        };
        let from_exclusive = if matches!(mode, Slice3AtomicMode::GappedHistory) {
            RepositoryHistoryCursor::new(RepositoryVersion::parse("199").unwrap(), digest('f'))
        } else {
            request.before_repository_cursor().clone()
        };
        let registry = EvidenceSourceRegistry::task9()
            .map_err(|_| RepositoryResultContractError("Slice3 registry failed"))?;
        let mut entries = Vec::with_capacity(classifications.len());
        let mut ordered_cursors = Vec::with_capacity(classifications.len());
        let mut candidates = BTreeMap::new();
        let mut evidence_bytes = BTreeMap::new();
        for (index, classification) in classifications.into_iter().enumerate() {
            let repository_version = RepositoryVersion::parse(&(201 + index).to_string()).unwrap();
            ordered_cursors.push(slice3_entry_cursor(index, repository_version.clone()));
            match classification {
                RepositoryHistoryPartitionClassification::TaskCommit => entries.push(json!({
                    "repositoryVersion": repository_version,
                    "classification": "taskCommit",
                    "semanticDeltaDigest": if matches!(mode, Slice3AtomicMode::SemanticDigestSubstitution) {
                        digest('f')
                    } else {
                        core.committed_objects_digest.clone()
                    },
                })),
                RepositoryHistoryPartitionClassification::UnrelatedRoutine => {
                    let routine = RoutineRepositoryVersionClassificationEvidence::new(
                        repository_version.as_str(),
                        "unrelated",
                        None,
                        digest('a').as_str(),
                        digest('b').as_str(),
                    )
                    .map_err(|_| RepositoryResultContractError("Slice3 routine evidence failed"))?;
                    let routine_value = serde_json::to_value(&routine)
                        .map_err(|_| RepositoryResultContractError("Slice3 routine encode failed"))?;
                    let evidence_digest = routine_value["classificationDigest"]
                        .as_str()
                        .ok_or(RepositoryResultContractError("Slice3 routine digest missing"))?;
                    let source_ref = RepositoryHistorySourceEvidenceRef::new(
                        EvidenceKind::RoutineClassification,
                        evidence_digest,
                    )
                    .map_err(|_| RepositoryResultContractError("Slice3 source ref failed"))?;
                    let semantic_record = json!({
                        "repositoryVersion": repository_version,
                        "partitionClassification": "unrelatedRoutine",
                        "rootDeltaDigest": digest('a'),
                        "contentDeltaDigest": digest('b'),
                        "classificationDigest": evidence_digest,
                        "externalSupportDisjointnessDigest": null,
                        "correctiveInstructionDigest": null,
                        "nonConflictingConcurrentEvidenceDigest": null,
                    });
                    entries.push(json!({
                        "repositoryVersion": repository_version,
                        "classification": "unrelatedRoutine",
                        "semanticDeltaDigest": slice3_json_digest(&semantic_record),
                        "sourceEvidenceRef": source_ref,
                    }));
                    candidates.insert(
                        repository_version.as_str().to_owned(),
                        EvidenceSourceIndexCandidate::from_capability_adapter(
                            repository_version.as_str(),
                            registry.registry_digest().as_str(),
                            "b3300000-0000-4000-8000-000000000061",
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
                        .map_err(|_| RepositoryResultContractError("Slice3 index row failed"))?,
                    );
                    evidence_bytes.insert(
                        (
                            EvidenceKind::RoutineClassification,
                            evidence_digest.to_owned(),
                        ),
                        serde_json_canonicalizer::to_vec(&routine)
                            .map_err(|_| RepositoryResultContractError("Slice3 evidence bytes failed"))?,
                    );
                }
                _ => unreachable!("Slice3 committed fixture uses only task and routine entries"),
            }
        }
        let through_inclusive = ordered_cursors
            .last()
            .cloned()
            .unwrap_or_else(|| from_exclusive.clone());
        let digest_record = json!({
            "fromExclusive": from_exclusive,
            "throughInclusive": through_inclusive,
            "entries": entries,
        });
        let mut raw_value = digest_record.as_object().unwrap().clone();
        raw_value.insert(
            "partitionDigest".into(),
            serde_json::to_value(slice3_json_digest(&digest_record)).unwrap(),
        );
        let raw_partition: UnvalidatedRepositoryHistoryPartition =
            serde_json::from_value(Value::Object(raw_value))
                .map_err(|_| RepositoryResultContractError("Slice3 raw partition failed"))?;
        let order = Slice3HistoryOrder {
            evidence: RepositoryHistoryOrderEvidence::from_capability_adapter(
                "repository.history-order.slice3-real-resolver",
                from_exclusive,
                through_inclusive,
                ordered_cursors,
            )
            .map_err(|_| RepositoryResultContractError("Slice3 order evidence failed"))?,
        };
        let index = Slice3HistoryIndex { candidates };
        let bytes = Slice3HistoryBytes {
            bytes: evidence_bytes,
        };
        let resolver = RepositoryHistoryPartitionResolver::new(&registry, &index, &order, &bytes);
        request.resolve_task_commit_history(
            core,
            raw_partition,
            &resolver,
            task_version_anchor,
            terminal_repository_anchor,
        )
    }

    fn matching_committed_objects(
        request: &CommitAtomicCommitRequest<'_>,
        version: &RepositoryVersion,
    ) -> CommittedRepositoryObjects {
        CommittedRepositoryObjects::new(
            request
                .exact_objects()
                .as_slice()
                .iter()
                .map(|object| match object {
                    CommitExactObject::RootModify(_) => {
                        CommittedRepositoryObject::root_modify(version.clone(), digest('8'))
                    }
                    CommitExactObject::ObjectAdd(value) => {
                        CommittedRepositoryObject::object_present(
                            value.target.object_id().clone(),
                            PresentObjectAction::Add,
                            version.clone(),
                            digest('8'),
                        )
                    }
                    CommitExactObject::ObjectModify(value) => {
                        CommittedRepositoryObject::object_present(
                            value.target.object_id().clone(),
                            PresentObjectAction::Modify,
                            version.clone(),
                            digest('8'),
                        )
                    }
                    CommitExactObject::ObjectDelete(value) => {
                        CommittedRepositoryObject::object_absent(
                            value.target.object_id().clone(),
                            version.clone(),
                        )
                    }
                })
                .collect(),
        )
        .unwrap()
    }

    fn matching_released_lock_authority(
        request: &CommitAtomicCommitRequest<'_>,
    ) -> Result<CommitPostCommandLockAuthority, RepositoryResultContractError> {
        request.observe_locks_released(
            CommitReleasedLocksObservationInput::from_repository_adapter(
                request.lock_set_id().clone(),
                request.journaled_lock_receipts().to_vec(),
                CanonicalRepositoryTargets::new(Vec::new()).unwrap(),
                request.atomic_commit_safety_capability_id().clone(),
            ),
        )
    }

    fn committed_observation(
        request: &CommitAtomicCommitRequest<'_>,
        mode: Slice3AtomicMode,
    ) -> Result<CommitAtomicCommitObservation, RepositoryResultContractError> {
        let task_version = RepositoryVersion::parse("201").unwrap();
        let repository_version = if matches!(mode, Slice3AtomicMode::VersionSubstitution) {
            RepositoryVersion::parse("999").unwrap()
        } else {
            task_version.clone()
        };
        let committed_objects = if matches!(mode, Slice3AtomicMode::ObjectSubstitution) {
            CommittedRepositoryObjects::new(vec![CommittedRepositoryObject::object_present(
                MetadataObjectId::parse("b3300000-0000-4000-8000-000000000088").unwrap(),
                PresentObjectAction::Modify,
                task_version.clone(),
                digest('8'),
            )])
            .unwrap()
        } else {
            matching_committed_objects(request, &task_version)
        };
        let committed_objects_digest = if matches!(mode, Slice3AtomicMode::DigestSubstitution) {
            digest('f')
        } else {
            request.committed_objects_digest(&committed_objects)?
        };
        let retained_anchor = request.post_merge_repository_anchor();
        let task_cursor = RepositoryHistoryCursor::new(task_version.clone(), digest('2'));
        let task_repository_identity = if matches!(mode, Slice3AtomicMode::IdentitySubstitution) {
            digest('f')
        } else {
            retained_anchor.repository_identity().clone()
        };
        let task_version_anchor = request.observe_repository_anchor(
            task_cursor,
            task_repository_identity,
            retained_anchor.configuration_identity().clone(),
            digest('8'),
        )?;
        let terminal_entry_index = if matches!(mode, Slice3AtomicMode::MissingTaskCommit) {
            0
        } else {
            1
        };
        let terminal_version =
            RepositoryVersion::parse(&(201 + terminal_entry_index).to_string()).unwrap();
        let terminal_cursor = if matches!(mode, Slice3AtomicMode::EndpointSubstitution) {
            RepositoryHistoryCursor::new(RepositoryVersion::parse("999").unwrap(), digest('e'))
        } else {
            slice3_entry_cursor(terminal_entry_index, terminal_version)
        };
        let terminal_anchor = request.observe_repository_anchor(
            terminal_cursor,
            retained_anchor.repository_identity().clone(),
            retained_anchor.configuration_identity().clone(),
            digest('9'),
        )?;
        let release = if matches!(mode, Slice3AtomicMode::UnlockUnknown) {
            CommitLockReleaseObservation::Unknown
        } else {
            let lock_set_id = if matches!(mode, Slice3AtomicMode::ReleaseForeignLockSet) {
                id("b3300000-0000-4000-8000-000000000099")
            } else {
                request.lock_set_id().clone()
            };
            let mut released_receipts = request.journaled_lock_receipts().to_vec();
            if matches!(mode, Slice3AtomicMode::ReleaseForeignReceipt) {
                released_receipts[0].observed_at =
                    NormalizedUtcInstant::parse("2026-07-23T23:59:59Z").unwrap();
            }
            let mut authority = request.observe_locks_released(
                CommitReleasedLocksObservationInput::from_repository_adapter(
                    lock_set_id,
                    released_receipts,
                    CanonicalRepositoryTargets::new(Vec::new()).unwrap(),
                    request.atomic_commit_safety_capability_id().clone(),
                ),
            )?;
            if matches!(mode, Slice3AtomicMode::ReleaseForeignInvocation) {
                request.replace_lock_witness_with_foreign_invocation_test_only(&mut authority);
            }
            CommitLockReleaseObservation::Verified(Box::new(authority))
        };
        let atomic_capability = if matches!(mode, Slice3AtomicMode::CapabilitySubstitution) {
            CapabilityRowId::parse("repository.atomic-commit.foreign").unwrap()
        } else {
            request.atomic_commit_safety_capability_id().clone()
        };
        let core = request.observe_committed_core(
            CommitCommittedCoreObservationInput::from_atomic_adapter(
                if matches!(mode, Slice3AtomicMode::ReceiptSubstitution) {
                    id("b3300000-0000-4000-8000-000000000099")
                } else {
                    request.preallocated_commit_receipt_id().clone()
                },
                repository_version,
                committed_objects,
                committed_objects_digest,
                atomic_capability,
            ),
        )?;
        let history = resolve_slice3_task_commit_history(
            request,
            &core,
            mode,
            task_version_anchor,
            terminal_anchor,
        )?;
        let history = if matches!(mode, Slice3AtomicMode::EqualScalarForeignPartition) {
            let CommitCommittedHistoryObservation {
                post_commit_history_partition,
                task_version_anchor,
                terminal_repository_anchor,
            } = history;
            let foreign_equal_scalar_core = request.observe_committed_core_unchecked_test_only(
                CommitCommittedCoreObservationInput::from_atomic_adapter(
                    core.commit_receipt_id.clone(),
                    core.repository_version.clone(),
                    core.committed_objects.clone(),
                    core.committed_objects_digest.clone(),
                    core.atomic_commit_safety_capability_id.clone(),
                ),
                true,
            );
            let forged_partition = task_commit_history_partition_fixture_test_only(
                post_commit_history_partition.into_partition(),
                &foreign_equal_scalar_core,
                core.committed_objects_digest.clone(),
            )
            .map_err(|_| RepositoryResultContractError("taskCommit fixture failed"))?;
            CommitCommittedHistoryObservation::new(
                forged_partition,
                task_version_anchor,
                terminal_repository_anchor,
            )
        } else {
            history
        };
        Ok(request.observe_committed(
            CommitCommittedObservationInput::from_validated_atomic_observation(
                core, history, release,
            ),
        ))
    }

    fn zero_effect_observation(
        request: &CommitAtomicCommitRequest<'_>,
        mode: Slice3AtomicMode,
        mut terminal_target_states: RepositoryTargetStates,
    ) -> Result<CommitAtomicCommitObservation, RepositoryResultContractError> {
        let classes = if matches!(mode, Slice3AtomicMode::ZeroTaskCommitPresent) {
            vec![RepositoryHistoryPartitionClassification::TaskCommit]
        } else {
            vec![RepositoryHistoryPartitionClassification::UnrelatedRoutine]
        };
        let partition = postcommit_partition(request.before_repository_cursor().clone(), &classes);
        let retained_anchor = request.post_merge_repository_anchor();
        let terminal_cursor = if matches!(mode, Slice3AtomicMode::ZeroTerminalAnchorSubstitution) {
            RepositoryHistoryCursor::new(RepositoryVersion::parse("999").unwrap(), digest('f'))
        } else {
            partition.through_inclusive().clone()
        };
        let terminal_anchor = request.observe_repository_anchor(
            terminal_cursor,
            retained_anchor.repository_identity().clone(),
            retained_anchor.configuration_identity().clone(),
            retained_anchor.configuration_fingerprint().clone(),
        )?;
        if matches!(mode, Slice3AtomicMode::ZeroIncompleteObjects) {
            terminal_target_states = serde_json::from_value(json!([])).unwrap();
        } else if matches!(mode, Slice3AtomicMode::ZeroStaleObjectVersion) {
            let mut value = serde_json::to_value(&terminal_target_states).unwrap();
            let first = value.as_array_mut().unwrap().first_mut().unwrap();
            if first.get("repositoryVersion").is_some() {
                first["repositoryVersion"] = json!("999");
            } else {
                first["absenceEstablishedAtVersion"] = json!("999");
            }
            terminal_target_states = serde_json::from_value(value).unwrap();
        }
        let mut terminal_target_snapshot = request.observe_terminal_target_snapshot(
            terminal_anchor,
            terminal_target_states,
            CapabilityRowId::parse("repository.commit-target-snapshot.terminal").unwrap(),
            if matches!(mode, Slice3AtomicMode::ZeroForeignCapability) {
                CapabilityRowId::parse("repository.atomic-commit.foreign").unwrap()
            } else {
                request.atomic_commit_safety_capability_id().clone()
            },
        )?;
        let lock_state = match mode {
            Slice3AtomicMode::ZeroReleased
            | Slice3AtomicMode::ZeroForeignLockSet
            | Slice3AtomicMode::ZeroForeignReceipt
            | Slice3AtomicMode::ZeroLockForeignInvocation => {
                let lock_set_id = if matches!(mode, Slice3AtomicMode::ZeroForeignLockSet) {
                    id("b3300000-0000-4000-8000-000000000099")
                } else {
                    request.lock_set_id().clone()
                };
                let mut released_receipts = request.journaled_lock_receipts().to_vec();
                if matches!(mode, Slice3AtomicMode::ZeroForeignReceipt) {
                    released_receipts[0].observed_at =
                        NormalizedUtcInstant::parse("2026-07-23T23:59:58Z").unwrap();
                }
                let mut authority = request.observe_locks_released(
                    CommitReleasedLocksObservationInput::from_repository_adapter(
                        lock_set_id,
                        released_receipts,
                        CanonicalRepositoryTargets::new(Vec::new()).unwrap(),
                        request.atomic_commit_safety_capability_id().clone(),
                    ),
                )?;
                if matches!(mode, Slice3AtomicMode::ZeroLockForeignInvocation) {
                    request.replace_lock_witness_with_foreign_invocation_test_only(&mut authority);
                }
                CommitZeroEffectLockState::VerifiedReleased(authority)
            }
            Slice3AtomicMode::ZeroReleaseUnknown => CommitZeroEffectLockState::Unknown,
            Slice3AtomicMode::ZeroIncompleteInventory => {
                CommitZeroEffectLockState::Held(request.observe_locks_held(
                    CommitHeldLocksObservationInput::from_repository_adapter(
                        request.lock_set_id().clone(),
                        request.journaled_lock_receipts().to_vec(),
                        CanonicalRepositoryTargets::new(Vec::new()).unwrap(),
                    ),
                )?)
            }
            _ => CommitZeroEffectLockState::Held(request.observe_locks_held(
                CommitHeldLocksObservationInput::from_repository_adapter(
                    request.lock_set_id().clone(),
                    request.journaled_lock_receipts().to_vec(),
                    request.full_lock_inventory()?,
                ),
            )?),
        };
        let mut certificate = request.observe_zero_effect_certificate(
            &terminal_target_snapshot,
            &partition,
            &lock_state,
            id("b3300000-0000-4000-8000-000000000042"),
            request.atomic_commit_safety_capability_id().clone(),
        )?;
        if matches!(mode, Slice3AtomicMode::ZeroBadCertificate) {
            certificate.certificate_digest = digest('f');
        }
        if matches!(mode, Slice3AtomicMode::ZeroForeignInvocation) {
            request.replace_zero_snapshot_witness_with_foreign_invocation_test_only(
                &mut terminal_target_snapshot,
            );
        }
        Ok(request.observe_proven_zero_effect(
            CommitProvenZeroEffectObservationInput::from_validated_atomic_observation(
                certificate,
                partition,
                terminal_target_snapshot,
                lock_state,
            ),
        ))
    }

    fn run_slice3_with_registration(
        registration_mode: Slice3RegistrationMode,
        operation_scope: OperationScope,
        intent_mode: Slice3IntentMode,
        atomic_mode: Slice3AtomicMode,
    ) -> (CommitEffectIntentOutcome, Rc<Slice3Counters>) {
        let counters = Rc::new(Slice3Counters::default());
        let scope = ready_scope(atomic_mode, Rc::clone(&counters));
        let outcome = scope.commit_exact_once(
            operation_scope,
            id("b3300000-0000-4000-8000-000000000041"),
            &mut Slice3RegisteredOperationPort {
                mode: registration_mode,
                counters: Rc::clone(&counters),
            },
            &mut Slice3IntentPort {
                mode: intent_mode,
                counters: Rc::clone(&counters),
            },
        );
        (outcome, counters)
    }

    fn run_slice3(
        intent_mode: Slice3IntentMode,
        atomic_mode: Slice3AtomicMode,
    ) -> (CommitEffectIntentOutcome, Rc<Slice3Counters>) {
        run_slice3_with_registration(
            Slice3RegistrationMode::Exact,
            task_scope("b3300000-0000-4000-8000-000000000002"),
            intent_mode,
            atomic_mode,
        )
    }

    fn execute_ready_slice3_scope(
        scope: CommitScopedAtomicSafetyAuthority,
        counters: Rc<Slice3Counters>,
        commit_receipt_id: UnicaId,
    ) -> CommitEffectIntentOutcome {
        scope.commit_exact_once(
            task_scope("b3300000-0000-4000-8000-000000000002"),
            commit_receipt_id,
            &mut Slice3RegisteredOperationPort {
                mode: Slice3RegistrationMode::Exact,
                counters: Rc::clone(&counters),
            },
            &mut Slice3IntentPort {
                mode: Slice3IntentMode::Written,
                counters,
            },
        )
    }

    fn run_genuine_foreign_lock_splice(zero_effect: bool) -> CommitEffectIntentOutcome {
        let counters = Rc::new(Slice3Counters::default());
        let capture_mode = if zero_effect {
            Slice3AtomicMode::CaptureZeroObservation
        } else {
            Slice3AtomicMode::CaptureLockAuthority
        };
        let foreign = ready_scope_with_lock_identity(
            capture_mode,
            Rc::clone(&counters),
            id("b3300000-0000-4000-8000-000000000090"),
            NormalizedUtcInstant::parse("2026-07-23T02:00:00Z").unwrap(),
        );
        let captured = execute_ready_slice3_scope(
            foreign,
            Rc::clone(&counters),
            id("b3300000-0000-4000-8000-000000000045"),
        );
        assert!(matches!(
            captured,
            CommitEffectIntentOutcome::PostIntent(CommitExactOnceOutcome::Ambiguous(_))
        ));
        let splice_mode = if zero_effect {
            Slice3AtomicMode::SpliceZeroObservation
        } else {
            Slice3AtomicMode::SpliceLockAuthority
        };
        let target = ready_scope(splice_mode, Rc::clone(&counters));
        execute_ready_slice3_scope(target, counters, id("b3300000-0000-4000-8000-000000000041"))
    }

    #[test]
    fn gate_b3_commit_effect_intent_fsync_precedes_command_and_binds_registered_apply() {
        let (outcome, counters) =
            run_slice3(Slice3IntentMode::Written, Slice3AtomicMode::Committed);
        assert!(matches!(
            outcome,
            CommitEffectIntentOutcome::PostIntent(CommitExactOnceOutcome::Committed(_))
        ));
        assert_eq!(counters.intent_calls.get(), 1);
        assert_eq!(counters.intent_witnesses.get(), 1);
        assert_eq!(counters.intent_binds.get(), 1);
        assert_eq!(counters.intent_getters.get(), 1);
        assert_eq!(counters.command_calls.get(), 1);
        let bytes = &counters.observed_intent_record_bytes.borrow()[0];
        let persisted: Value = crate::domain::i_json::from_slice(bytes).unwrap();
        assert_eq!(
            persisted,
            counters.observed_intent_typed_records.borrow()[0]
        );
        assert_eq!(
            format!("{:x}", Sha256::digest(bytes)),
            counters.observed_intent_record_digests.borrow()[0].as_str()
        );
        assert_eq!(
            persisted["digestKind"],
            json!("unica.repository.commit.effect-intent.v1")
        );
        assert_eq!(
            persisted["preallocatedCommitReceiptId"],
            json!("b3300000-0000-4000-8000-000000000041")
        );
        assert_eq!(persisted["operationScope"]["scopeKind"], json!("task"));
        assert_eq!(
            persisted["preCommandTargetSnapshot"]["targetStates"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            persisted["preCommandTargetSnapshot"]["repositoryAnchor"]["historyCursor"],
            persisted["beforeRepositoryCursor"]
        );
        assert_eq!(
            slice3_json_digest(&persisted["preCommandTargetSnapshot"]),
            serde_json::from_value(persisted["preCommandTargetSnapshotDigest"].clone()).unwrap()
        );
        assert_eq!(
            counters.observed_command_comments.borrow().as_slice(),
            &[Comment::parse("PR-137: Consumed gate B2").unwrap()]
        );
        assert_eq!(
            counters.observed_command_targets.borrow()[0].as_slice(),
            ["root:modify"]
        );
        assert_eq!(
            counters.observed_command_receipt_ids.borrow()[0],
            id("b3300000-0000-4000-8000-000000000041")
        );
        assert_eq!(
            counters.observed_command_before_cursors.borrow()[0],
            serde_json::from_value(persisted["beforeRepositoryCursor"].clone()).unwrap()
        );
        assert_eq!(counters.observed_command_release_objects.borrow().len(), 1);
        assert_eq!(counters.observed_command_release_guards.borrow().len(), 1);

        let object_add = MetadataObjectId::parse("b3300000-0000-4000-8000-000000000071").unwrap();
        let object_modify =
            MetadataObjectId::parse("b3300000-0000-4000-8000-000000000072").unwrap();
        let object_delete =
            MetadataObjectId::parse("b3300000-0000-4000-8000-000000000073").unwrap();
        let all_action_objects = CommitExactObjects::new(vec![
            CommitExactObject::RootModify(RootModifyExactObject {
                target: RootTargetIdentity::new(),
                action: ModifyAction::Value,
            }),
            CommitExactObject::ObjectAdd(ObjectAddExactObject {
                target: ObjectTargetIdentity::new(object_add.clone()),
                action: AddAction::Value,
            }),
            CommitExactObject::ObjectModify(ObjectModifyExactObject {
                target: ObjectTargetIdentity::new(object_modify.clone()),
                action: ModifyAction::Value,
            }),
            CommitExactObject::ObjectDelete(ObjectDeleteExactObject {
                target: ObjectTargetIdentity::new(object_delete.clone()),
                action: DeleteAction::Value,
            }),
        ])
        .unwrap();
        assert_eq!(
            all_action_objects
                .iter()
                .map(|target| match target {
                    CommitExactObjectRef::RootModify => "root:modify".to_owned(),
                    CommitExactObjectRef::ObjectAdd { object_id } => {
                        format!("{}:add", object_id.as_str())
                    }
                    CommitExactObjectRef::ObjectModify { object_id } => {
                        format!("{}:modify", object_id.as_str())
                    }
                    CommitExactObjectRef::ObjectDelete { object_id } => {
                        format!("{}:delete", object_id.as_str())
                    }
                })
                .collect::<Vec<_>>(),
            vec![
                "root:modify".to_owned(),
                format!("{}:add", object_add.as_str()),
                format!("{}:modify", object_modify.as_str()),
                format!("{}:delete", object_delete.as_str()),
            ]
        );
        assert_eq!(
            counters.events.borrow().as_slice(),
            ["intent-call", "intent-fsynced", "command-call"]
        );
    }

    #[test]
    fn gate_b3_commit_effect_intent_proven_not_written_retains_ready_and_calls_no_command() {
        let (outcome, counters) = run_slice3(
            Slice3IntentMode::ProvenNotWritten,
            Slice3AtomicMode::Committed,
        );
        let CommitEffectIntentOutcome::PreIntentBlocked(blocked) = outcome else {
            panic!("bound proven-not-written must be pre-intent blocked")
        };
        let approved_commit_digest = blocked.approved_commit_digest().clone();
        assert_ne!(&approved_commit_digest, &digest('f'));
        assert_eq!(counters.intent_getters.get(), 1);
        assert_eq!(counters.command_calls.get(), 0);
        let fresh_partition = blocked
            .scope
            .immediate_history_guard_evidence
            .partition()
            .clone();
        let fresh_counters = Rc::new(Slice3Counters::default());
        let refreshed = blocked.recheck_with_fresh_observation(&mut Slice3ImmediatePort {
            partition: Some(fresh_partition),
            atomic_mode: Slice3AtomicMode::Committed,
            counters: fresh_counters,
        });
        assert!(refreshed._registered.is_some());
        assert!(refreshed._record.is_some());
        assert!(refreshed._certificate.is_some());
        let CommitImmediateRecheckOutcome::Ready(refreshed_scope) = refreshed.into_recheck() else {
            panic!("pre-intent rejection must drive a fresh immediate recheck")
        };
        assert_eq!(
            refreshed_scope.approved.0.commit_digest(),
            &approved_commit_digest
        );
    }

    #[test]
    fn gate_b3_commit_effect_intent_error_or_response_loss_is_ambiguous_and_calls_no_command() {
        for mode in [Slice3IntentMode::Error, Slice3IntentMode::ResponseLoss] {
            let (outcome, counters) = run_slice3(mode, Slice3AtomicMode::Committed);
            let CommitEffectIntentOutcome::PostIntent(CommitExactOnceOutcome::Ambiguous(ambiguous)) =
                outcome
            else {
                panic!("intent error or response loss must be ambiguous")
            };
            let approved_commit_digest = ambiguous.approved_commit_digest().clone();
            let recovery_source = ambiguous.into_recovery_source();
            assert_eq!(
                recovery_source.approved_commit_digest(),
                &approved_commit_digest
            );
            assert_ne!(recovery_source.effect_intent_record_digest(), &digest('f'));
            assert_eq!(counters.intent_calls.get(), 1);
            assert_eq!(counters.command_calls.get(), 0);
        }
    }

    #[test]
    fn gate_b3_commit_effect_intent_rejects_scope_generation_operation_input_and_digest_substitution(
    ) {
        for mode in [
            Slice3IntentMode::BindingScope,
            Slice3IntentMode::BindingGeneration,
        ] {
            let (outcome, counters) = run_slice3(mode, Slice3AtomicMode::Committed);
            assert!(matches!(
                outcome,
                CommitEffectIntentOutcome::PostIntent(CommitExactOnceOutcome::Ambiguous(_))
            ));
            assert_eq!(counters.command_calls.get(), 0);
        }

        for mode in [
            Slice3RegistrationMode::BindingOperation,
            Slice3RegistrationMode::BindingInput,
            Slice3RegistrationMode::BindingRecordDigest,
            Slice3RegistrationMode::BindingLeaseDigest,
        ] {
            let (outcome, counters) = run_slice3_with_registration(
                mode,
                task_scope("b3300000-0000-4000-8000-000000000002"),
                Slice3IntentMode::Written,
                Slice3AtomicMode::Committed,
            );
            assert!(matches!(
                outcome,
                CommitEffectIntentOutcome::PreIntentBlocked(_)
            ));
            assert_eq!(counters.registered_calls.get(), 1);
            assert_eq!(counters.registered_getters.get(), 0);
            assert_eq!(counters.intent_calls.get(), 0);
        }

        let start_scope = OperationScope::StartAttempt {
            workspace_identity_digest: digest('a'),
            task_id: TaskId::parse("PR-137").unwrap(),
        };
        let foreign_task_scope = OperationScope::Task {
            project_id: ProjectId::parse("b2100000-0000-4000-8000-000000000001").unwrap(),
            task_id: TaskId::parse("PR-999").unwrap(),
            instance_id: id("b3300000-0000-4000-8000-000000000002"),
        };
        let foreign_project_scope = OperationScope::Task {
            project_id: ProjectId::parse("b3300000-0000-4000-8000-000000000099").unwrap(),
            task_id: TaskId::parse("PR-137").unwrap(),
            instance_id: id("b3300000-0000-4000-8000-000000000098"),
        };
        for operation_scope in [start_scope, foreign_task_scope, foreign_project_scope] {
            let (outcome, counters) = run_slice3_with_registration(
                Slice3RegistrationMode::Exact,
                operation_scope,
                Slice3IntentMode::Written,
                Slice3AtomicMode::Committed,
            );
            assert!(matches!(
                outcome,
                CommitEffectIntentOutcome::PreIntentBlocked(_)
            ));
            assert_eq!(counters.registered_calls.get(), 0);
            assert_eq!(counters.registered_getters.get(), 0);
            assert_eq!(counters.intent_calls.get(), 0);
        }

        for mode in [
            Slice3RegistrationMode::AuthoritativeStartAttempt,
            Slice3RegistrationMode::AuthoritativeForeignTask,
            Slice3RegistrationMode::AuthoritativeForeignContainer,
        ] {
            let (outcome, counters) = run_slice3_with_registration(
                mode,
                task_scope("b3300000-0000-4000-8000-000000000002"),
                Slice3IntentMode::Written,
                Slice3AtomicMode::Committed,
            );
            assert!(matches!(
                outcome,
                CommitEffectIntentOutcome::PreIntentBlocked(_)
            ));
            assert_eq!(counters.registered_calls.get(), 1);
            assert_eq!(counters.registered_getters.get(), 1);
            assert_eq!(counters.intent_calls.get(), 0);
        }

        let (first, first_counters) = run_slice3_with_registration(
            Slice3RegistrationMode::Exact,
            task_scope("b3300000-0000-4000-8000-000000000002"),
            Slice3IntentMode::Written,
            Slice3AtomicMode::Committed,
        );
        let (second, second_counters) = run_slice3_with_registration(
            Slice3RegistrationMode::DifferentLeaseDigest,
            task_scope("b3300000-0000-4000-8000-000000000002"),
            Slice3IntentMode::Written,
            Slice3AtomicMode::Committed,
        );
        assert!(matches!(
            first,
            CommitEffectIntentOutcome::PostIntent(CommitExactOnceOutcome::Committed(_))
        ));
        assert!(matches!(
            second,
            CommitEffectIntentOutcome::PostIntent(CommitExactOnceOutcome::Committed(_))
        ));
        assert_ne!(
            first_counters.observed_intent_record_digests.borrow()[0],
            second_counters.observed_intent_record_digests.borrow()[0]
        );
    }

    #[test]
    fn gate_b3_commit_effect_intent_rejects_equal_scalar_stale_completion_before_getters() {
        let (outcome, counters) = run_slice3(
            Slice3IntentMode::StaleCompletion,
            Slice3AtomicMode::Committed,
        );
        assert!(matches!(
            outcome,
            CommitEffectIntentOutcome::PostIntent(CommitExactOnceOutcome::Ambiguous(_))
        ));
        assert_eq!(counters.intent_witnesses.get(), 0);
        assert_eq!(counters.intent_binds.get(), 0);
        assert_eq!(counters.intent_getters.get(), 0);
        assert_eq!(counters.command_calls.get(), 0);
    }

    #[test]
    fn gate_b3_commit_exact_once_uses_same_immediate_lease_once() {
        let (outcome, counters) =
            run_slice3(Slice3IntentMode::Written, Slice3AtomicMode::Committed);
        assert!(matches!(
            outcome,
            CommitEffectIntentOutcome::PostIntent(CommitExactOnceOutcome::Committed(_))
        ));
        assert_eq!(counters.command_calls.get(), 1);
        assert_eq!(counters.atomic_getters.get(), 1);
    }

    #[test]
    fn gate_b3_commit_exact_once_rejects_cross_preview_operation_intent_scope_and_invocation() {
        for mode in [
            Slice3AtomicMode::WitnessMismatch,
            Slice3AtomicMode::BindingPreview,
            Slice3AtomicMode::BindingOperation,
            Slice3AtomicMode::BindingIntent,
            Slice3AtomicMode::BindingScope,
            Slice3AtomicMode::CompletionMismatch,
        ] {
            let (outcome, counters) = run_slice3(Slice3IntentMode::Written, mode);
            assert!(matches!(
                outcome,
                CommitEffectIntentOutcome::PostIntent(CommitExactOnceOutcome::Ambiguous(_))
            ));
            assert_eq!(counters.atomic_getters.get(), 0);
        }
    }

    #[test]
    fn gate_b3_commit_exact_once_generic_error_is_ambiguous_not_zero() {
        let (outcome, counters) =
            run_slice3(Slice3IntentMode::Written, Slice3AtomicMode::PortError);
        assert!(matches!(
            outcome,
            CommitEffectIntentOutcome::PostIntent(CommitExactOnceOutcome::Ambiguous(_))
        ));
        assert_eq!(counters.command_calls.get(), 1);
    }

    #[test]
    fn gate_b3_commit_exact_once_response_loss_never_retries() {
        let (outcome, counters) =
            run_slice3(Slice3IntentMode::Written, Slice3AtomicMode::ResponseLoss);
        assert!(matches!(
            outcome,
            CommitEffectIntentOutcome::PostIntent(CommitExactOnceOutcome::Ambiguous(_))
        ));
        assert_eq!(counters.command_calls.get(), 1);
        assert_eq!(counters.physical_effects.get(), 1);
    }

    #[test]
    fn gate_b3_commit_committed_separates_task_version_anchor_from_terminal_anchor() {
        let (outcome, _) = run_slice3(Slice3IntentMode::Written, Slice3AtomicMode::Committed);
        let CommitEffectIntentOutcome::PostIntent(CommitExactOnceOutcome::Committed(committed)) =
            outcome
        else {
            panic!("exact commit must be committed")
        };
        assert_eq!(
            committed
                .task_version_anchor()
                .history_cursor()
                .through_version()
                .as_str(),
            "201"
        );
        assert_eq!(
            committed
                .terminal_repository_anchor()
                .history_cursor()
                .through_version()
                .as_str(),
            "202"
        );
        assert_ne!(
            committed.task_version_anchor().history_cursor(),
            committed.terminal_repository_anchor().history_cursor()
        );
        let expected_before = committed.source.before_repository_cursor.clone();
        let expected_after = committed
            .terminal_repository_anchor()
            .history_cursor()
            .clone();
        let expected_semantic_digest = committed.observation.core.committed_objects_digest.clone();
        let expected_anchor = committed.terminal_repository_anchor().clone();
        let (expected_released_objects, expected_released_guard_locks) =
            match &committed.observation.release {
                CommitLockReleaseObservation::Verified(authority) => {
                    let (released_objects, released_guard_locks) = authority
                        .released_projection()
                        .expect("committed fixture must retain released authority");
                    (released_objects.clone(), released_guard_locks.clone())
                }
                CommitLockReleaseObservation::Unknown => {
                    panic!("committed fixture must retain exact release proof")
                }
            };
        let data = CommitData::from_committed_outcome(committed);
        assert_eq!(
            data.commit_receipt_id,
            id("b3300000-0000-4000-8000-000000000041")
        );
        assert_eq!(data.repository_version.as_str(), "201");
        assert_eq!(data.before_repository_cursor, expected_before);
        assert_eq!(data.after_repository_cursor, expected_after);
        assert_eq!(data.released_objects, expected_released_objects);
        assert_eq!(data.released_guard_locks, expected_released_guard_locks);
        assert_eq!(data.repository_anchor, expected_anchor);
        let value = serde_json::to_value(&data).unwrap();
        assert_eq!(
            value["postCommitHistoryPartition"]["entries"][0]["classification"],
            json!("taskCommit")
        );
        assert_eq!(
            value["postCommitHistoryPartition"]["entries"][0]["semanticDeltaDigest"],
            json!(expected_semantic_digest)
        );
        assert_eq!(
            value["beforeRepositoryCursor"],
            serde_json::to_value(&data.before_repository_cursor).unwrap()
        );
        assert_eq!(
            value["afterRepositoryCursor"],
            serde_json::to_value(&data.after_repository_cursor).unwrap()
        );
        assert_eq!(
            value["repositoryAnchor"],
            serde_json::to_value(&data.repository_anchor).unwrap()
        );
    }

    #[test]
    fn gate_b3_commit_committed_rejects_endpoint_object_version_digest_capability_and_identity_substitution(
    ) {
        for mode in [
            Slice3AtomicMode::EndpointSubstitution,
            Slice3AtomicMode::ObjectSubstitution,
            Slice3AtomicMode::VersionSubstitution,
            Slice3AtomicMode::DigestSubstitution,
            Slice3AtomicMode::CapabilitySubstitution,
            Slice3AtomicMode::IdentitySubstitution,
            Slice3AtomicMode::ReceiptSubstitution,
            Slice3AtomicMode::SemanticDigestSubstitution,
            Slice3AtomicMode::EqualScalarForeignPartition,
        ] {
            let (outcome, counters) = run_slice3(Slice3IntentMode::Written, mode);
            assert!(matches!(
                outcome,
                CommitEffectIntentOutcome::PostIntent(CommitExactOnceOutcome::Ambiguous(_))
            ));
            assert_eq!(counters.command_calls.get(), 1);
        }
    }

    #[test]
    fn gate_b3_commit_committed_rejects_missing_duplicate_or_gapped_task_history() {
        for mode in [
            Slice3AtomicMode::MissingTaskCommit,
            Slice3AtomicMode::DuplicateTaskCommit,
            Slice3AtomicMode::GappedHistory,
        ] {
            let (outcome, _) = run_slice3(Slice3IntentMode::Written, mode);
            assert!(matches!(
                outcome,
                CommitEffectIntentOutcome::PostIntent(CommitExactOnceOutcome::Ambiguous(_))
            ));
        }
    }

    #[test]
    fn gate_b3_commit_committed_release_or_unlock_unknown_is_ambiguous() {
        for mode in [
            Slice3AtomicMode::UnlockUnknown,
            Slice3AtomicMode::ReleaseForeignLockSet,
            Slice3AtomicMode::ReleaseForeignReceipt,
            Slice3AtomicMode::ReleaseForeignInvocation,
        ] {
            let (outcome, _) = run_slice3(Slice3IntentMode::Written, mode);
            assert!(matches!(
                outcome,
                CommitEffectIntentOutcome::PostIntent(CommitExactOnceOutcome::Ambiguous(_))
            ));
        }
        assert!(matches!(
            run_genuine_foreign_lock_splice(false),
            CommitEffectIntentOutcome::PostIntent(CommitExactOnceOutcome::Ambiguous(_))
        ));
    }

    #[test]
    fn gate_b3_commit_zero_effect_requires_complete_bound_certificate_and_full_lock_inventory() {
        let (outcome, _) = run_slice3(Slice3IntentMode::Written, Slice3AtomicMode::ZeroEffect);
        let CommitEffectIntentOutcome::PostIntent(CommitExactOnceOutcome::ProvenZeroEffect(zero)) =
            outcome
        else {
            panic!("exact unchanged target snapshot must produce zero evidence")
        };
        let CommitProvenZeroEffectDisposition::CleanupRequired(cleanup) = zero.into_disposition()
        else {
            panic!("zero evidence with held locks must remain nonterminal cleanup authority")
        };
        assert_ne!(cleanup.effect_intent_digest(), &digest('f'));
        assert_ne!(cleanup.certificate_digest(), &digest('f'));
        assert!(cleanup.owns_current_lock_evidence());
        assert_ne!(
            cleanup.source.before_repository_cursor,
            *cleanup
                .observation
                .terminal_target_snapshot
                .record
                .repository_anchor
                .history_cursor()
        );
        let (released, _) = run_slice3(Slice3IntentMode::Written, Slice3AtomicMode::ZeroReleased);
        let CommitEffectIntentOutcome::PostIntent(CommitExactOnceOutcome::ProvenZeroEffect(
            released,
        )) = released
        else {
            panic!("released zero evidence must remain an owning authority")
        };
        let CommitProvenZeroEffectDisposition::VerifiedReleased(released) =
            released.into_disposition()
        else {
            panic!("verified release must not be classified as held cleanup")
        };
        assert_ne!(released.effect_intent_digest(), &digest('f'));
        assert_ne!(released.certificate_digest(), &digest('f'));
        assert!(released.owns_current_lock_evidence());
        for mode in [
            Slice3AtomicMode::ZeroBadCertificate,
            Slice3AtomicMode::ZeroForeignCapability,
            Slice3AtomicMode::ZeroForeignInvocation,
            Slice3AtomicMode::ZeroTerminalAnchorSubstitution,
            Slice3AtomicMode::ZeroIncompleteObjects,
            Slice3AtomicMode::ZeroStaleObjectVersion,
            Slice3AtomicMode::ZeroIncompleteInventory,
            Slice3AtomicMode::ZeroTaskCommitPresent,
            Slice3AtomicMode::ZeroReleaseUnknown,
            Slice3AtomicMode::ZeroForeignLockSet,
            Slice3AtomicMode::ZeroForeignReceipt,
            Slice3AtomicMode::ZeroLockForeignInvocation,
        ] {
            let (outcome, _) = run_slice3(Slice3IntentMode::Written, mode);
            assert!(matches!(
                outcome,
                CommitEffectIntentOutcome::PostIntent(CommitExactOnceOutcome::Ambiguous(_))
            ));
        }
        assert!(matches!(
            run_genuine_foreign_lock_splice(true),
            CommitEffectIntentOutcome::PostIntent(CommitExactOnceOutcome::Ambiguous(_))
        ));

        let root = CommitExactObject::RootModify(RootModifyExactObject {
            target: RootTargetIdentity::new(),
            action: ModifyAction::Value,
        });
        let add_id = MetadataObjectId::parse("b3300000-0000-4000-8000-000000000071").unwrap();
        let modify_id = MetadataObjectId::parse("b3300000-0000-4000-8000-000000000072").unwrap();
        let delete_id = MetadataObjectId::parse("b3300000-0000-4000-8000-000000000073").unwrap();
        let exact = CommitExactObjects::new(vec![
            root,
            CommitExactObject::ObjectAdd(ObjectAddExactObject {
                target: ObjectTargetIdentity::new(add_id.clone()),
                action: AddAction::Value,
            }),
            CommitExactObject::ObjectModify(ObjectModifyExactObject {
                target: ObjectTargetIdentity::new(modify_id.clone()),
                action: ModifyAction::Value,
            }),
            CommitExactObject::ObjectDelete(ObjectDeleteExactObject {
                target: ObjectTargetIdentity::new(delete_id.clone()),
                action: DeleteAction::Value,
            }),
        ])
        .unwrap();
        let baseline =
            target_states_for_exact_objects(&exact, &RepositoryVersion::parse("177").unwrap());
        assert!(exact_zero_effect_target_transition(
            &exact, &baseline, &baseline
        ));

        let mut mutations = Vec::new();
        let baseline_value = serde_json::to_value(&baseline).unwrap();

        let mut root_fingerprint = baseline_value.clone();
        root_fingerprint[0]["targetFingerprint"] = json!(digest('f'));
        mutations.push(root_fingerprint);

        let mut add_became_present = baseline_value.clone();
        add_became_present[1] = json!({
            "targetKind": "developmentObject",
            "state": "present",
            "objectId": add_id,
            "repositoryVersion": "177",
            "targetFingerprint": digest('a'),
        });
        mutations.push(add_became_present);

        let mut modify_fingerprint = baseline_value.clone();
        modify_fingerprint[2]["targetFingerprint"] = json!(digest('f'));
        mutations.push(modify_fingerprint);

        let mut delete_became_absent = baseline_value.clone();
        delete_became_absent[3] = json!({
            "targetKind": "developmentObject",
            "state": "absent",
            "objectId": delete_id,
            "absenceEstablishedAtVersion": "177",
            "expectedAbsent": true,
        });
        mutations.push(delete_became_absent);

        let mut version_changed = baseline_value.clone();
        version_changed[2]["repositoryVersion"] = json!("178");
        mutations.push(version_changed);

        let mut missing = baseline_value.clone();
        missing.as_array_mut().unwrap().pop();
        mutations.push(missing);

        let mut extra = baseline_value.clone();
        extra.as_array_mut().unwrap().push(json!({
            "targetKind": "developmentObject",
            "state": "present",
            "objectId": "b3300000-0000-4000-8000-000000000074",
            "repositoryVersion": "177",
            "targetFingerprint": digest('a'),
        }));
        mutations.push(extra);

        for mutation in mutations {
            let terminal: RepositoryTargetStates = serde_json::from_value(mutation).unwrap();
            assert!(!exact_zero_effect_target_transition(
                &exact, &baseline, &terminal
            ));
        }

        let mut reordered = baseline_value;
        reordered.as_array_mut().unwrap().swap(1, 2);
        assert!(serde_json::from_value::<RepositoryTargetStates>(reordered).is_err());
    }

    #[test]
    fn gate_b3_commit_atomic_wrong_completion_or_binding_calls_no_getters_and_retains_scope() {
        for mode in [
            Slice3AtomicMode::CompletionMismatch,
            Slice3AtomicMode::BindingOperation,
        ] {
            let (outcome, counters) = run_slice3(Slice3IntentMode::Written, mode);
            let CommitEffectIntentOutcome::PostIntent(CommitExactOnceOutcome::Ambiguous(ambiguous)) =
                outcome
            else {
                panic!("wrong completion or binding must be ambiguous")
            };
            assert_ne!(ambiguous.approved_commit_digest(), &digest('f'));
            assert_eq!(counters.atomic_getters.get(), 0);
            if matches!(mode, Slice3AtomicMode::CompletionMismatch) {
                assert_eq!(counters.atomic_witnesses.get(), 0);
                assert_eq!(counters.atomic_binds.get(), 0);
            }
        }
    }
}
