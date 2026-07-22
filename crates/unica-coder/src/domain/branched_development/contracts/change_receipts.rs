use super::scalars::PropertyPath;
use super::schema::one_of_schema;
use crate::domain::branched_development::canonical_json::{
    canonical_contract_digest, contract_digest_record_sealed, ContractDigestRecord,
};
use crate::domain::branched_development::{
    MetadataObjectId, Sha256Digest, SupportLayerId, TaskPhase, UnicaId,
};
use schemars::{json_schema, JsonSchema, Schema, SchemaGenerator};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::borrow::Cow;
use std::fmt;

const MAX_RECEIPT_ITEMS: usize = 1_024;
const MAX_I_JSON_INTEGER: u64 = 9_007_199_254_740_991;

macro_rules! wire_literal {
    ($name:ident, $wire:literal) => {
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
        )]
        enum $name {
            #[serde(rename = $wire)]
            Value,
        }
    };
}

wire_literal!(TaskWorkspaceContextKind, "taskWorkspaceChange");
wire_literal!(MergeResolutionContextKind, "mergeResolutionChange");
wire_literal!(ChangedMutationOutcome, "changed");
wire_literal!(NoChangeMutationOutcome, "noChange");
wire_literal!(MetadataPropertyTargetKind, "metadataProperty");
wire_literal!(SupportLayerPropertyTargetKind, "supportLayerProperty");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MutationOutcome {
    Changed,
    NoChange,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) enum CompatibleTaskMutationPhase {
    Developing,
    LocalVerified,
    SynchronizationPrepared,
    SynchronizationConflicts,
    Synchronized,
    IntegrationPlanned,
    BlockedByForeignLock,
    UnexpectedDelta,
    ValidationFailed,
}

impl TryFrom<TaskPhase> for CompatibleTaskMutationPhase {
    type Error = ChangeReceiptContractError;

    fn try_from(value: TaskPhase) -> Result<Self, Self::Error> {
        match value {
            TaskPhase::Developing => Ok(Self::Developing),
            TaskPhase::LocalVerified => Ok(Self::LocalVerified),
            TaskPhase::SynchronizationPrepared => Ok(Self::SynchronizationPrepared),
            TaskPhase::SynchronizationConflicts => Ok(Self::SynchronizationConflicts),
            TaskPhase::Synchronized => Ok(Self::Synchronized),
            TaskPhase::IntegrationPlanned => Ok(Self::IntegrationPlanned),
            TaskPhase::BlockedByForeignLock => Ok(Self::BlockedByForeignLock),
            TaskPhase::UnexpectedDelta => Ok(Self::UnexpectedDelta),
            TaskPhase::ValidationFailed => Ok(Self::ValidationFailed),
            _ => Err(ChangeReceiptContractError(
                "task phase is incompatible with a branched mutation receipt",
            )),
        }
    }
}

impl CompatibleTaskMutationPhase {
    const ALL: [Self; 9] = [
        Self::Developing,
        Self::LocalVerified,
        Self::SynchronizationPrepared,
        Self::SynchronizationConflicts,
        Self::Synchronized,
        Self::IntegrationPlanned,
        Self::BlockedByForeignLock,
        Self::UnexpectedDelta,
        Self::ValidationFailed,
    ];

    const fn as_str(self) -> &'static str {
        match self {
            Self::Developing => "developing",
            Self::LocalVerified => "localVerified",
            Self::SynchronizationPrepared => "synchronizationPrepared",
            Self::SynchronizationConflicts => "synchronizationConflicts",
            Self::Synchronized => "synchronized",
            Self::IntegrationPlanned => "integrationPlanned",
            Self::BlockedByForeignLock => "blockedByForeignLock",
            Self::UnexpectedDelta => "unexpectedDelta",
            Self::ValidationFailed => "validationFailed",
        }
    }
}

impl From<CompatibleTaskMutationPhase> for TaskPhase {
    fn from(value: CompatibleTaskMutationPhase) -> Self {
        match value {
            CompatibleTaskMutationPhase::Developing => Self::Developing,
            CompatibleTaskMutationPhase::LocalVerified => Self::LocalVerified,
            CompatibleTaskMutationPhase::SynchronizationPrepared => Self::SynchronizationPrepared,
            CompatibleTaskMutationPhase::SynchronizationConflicts => Self::SynchronizationConflicts,
            CompatibleTaskMutationPhase::Synchronized => Self::Synchronized,
            CompatibleTaskMutationPhase::IntegrationPlanned => Self::IntegrationPlanned,
            CompatibleTaskMutationPhase::BlockedByForeignLock => Self::BlockedByForeignLock,
            CompatibleTaskMutationPhase::UnexpectedDelta => Self::UnexpectedDelta,
            CompatibleTaskMutationPhase::ValidationFailed => Self::ValidationFailed,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TaskWorkspaceChangedPhaseTransition {
    phase_before: CompatibleTaskMutationPhase,
    resulting_phase: DevelopingPhase,
}

wire_literal!(DevelopingPhase, "developing");

impl TaskWorkspaceChangedPhaseTransition {
    fn new(phase_before: CompatibleTaskMutationPhase) -> Self {
        Self {
            phase_before,
            resulting_phase: DevelopingPhase::Value,
        }
    }
}

impl JsonSchema for TaskWorkspaceChangedPhaseTransition {
    fn inline_schema() -> bool {
        true
    }

    fn schema_name() -> Cow<'static, str> {
        "TaskWorkspaceChangedPhaseTransition".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        one_of_schema(
            CompatibleTaskMutationPhase::ALL
                .into_iter()
                .map(|phase| phase_transition_branch(phase.as_str(), "developing"))
                .collect(),
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TaskWorkspaceNoChangePhaseTransition {
    phase_before: CompatibleTaskMutationPhase,
    resulting_phase: CompatibleTaskMutationPhase,
}

impl TaskWorkspaceNoChangePhaseTransition {
    fn new(phase: CompatibleTaskMutationPhase) -> Self {
        Self {
            phase_before: phase,
            resulting_phase: phase,
        }
    }
}

impl<'de> Deserialize<'de> for TaskWorkspaceNoChangePhaseTransition {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            phase_before: CompatibleTaskMutationPhase,
            resulting_phase: CompatibleTaskMutationPhase,
        }
        let wire = Wire::deserialize(deserializer)?;
        if wire.phase_before != wire.resulting_phase {
            return Err(serde::de::Error::custom(
                "a no-change receipt must preserve its task phase",
            ));
        }
        Ok(Self::new(wire.phase_before))
    }
}

impl JsonSchema for TaskWorkspaceNoChangePhaseTransition {
    fn inline_schema() -> bool {
        true
    }

    fn schema_name() -> Cow<'static, str> {
        "TaskWorkspaceNoChangePhaseTransition".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        one_of_schema(
            CompatibleTaskMutationPhase::ALL
                .into_iter()
                .map(|phase| phase_transition_branch(phase.as_str(), phase.as_str()))
                .collect(),
        )
    }
}

fn phase_transition_branch(phase_before: &'static str, resulting_phase: &'static str) -> Schema {
    json_schema!({
        "type": "object",
        "properties": {
            "phaseBefore": { "type": "string", "const": phase_before },
            "resultingPhase": { "type": "string", "const": resulting_phase }
        },
        "required": ["phaseBefore", "resultingPhase"],
        "additionalProperties": false
    })
}

wire_literal!(SynchronizationConflictsPhase, "synchronizationConflicts");

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MergeResolutionPhaseTransition {
    phase_before: SynchronizationConflictsPhase,
    resulting_phase: SynchronizationConflictsPhase,
}

impl MergeResolutionPhaseTransition {
    const VALUE: Self = Self {
        phase_before: SynchronizationConflictsPhase::Value,
        resulting_phase: SynchronizationConflictsPhase::Value,
    };
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct MetadataPropertyAffectedTarget {
    target_kind: MetadataPropertyTargetKind,
    object_id: MetadataObjectId,
    property_path: PropertyPath,
}

impl MetadataPropertyAffectedTarget {
    pub(crate) fn new(object_id: MetadataObjectId, property_path: PropertyPath) -> Self {
        Self {
            target_kind: MetadataPropertyTargetKind::Value,
            object_id,
            property_path,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SupportLayerCapabilityAffectedTarget {
    target_kind: SupportLayerPropertyTargetKind,
    layer_id: SupportLayerId,
    property_path: PropertyPath,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SupportLayerObjectAffectedTarget {
    target_kind: SupportLayerPropertyTargetKind,
    layer_id: SupportLayerId,
    object_id: MetadataObjectId,
    property_path: PropertyPath,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(untagged)]
enum SupportLayerPropertyAffectedTarget {
    LayerCapability(SupportLayerCapabilityAffectedTarget),
    ObjectState(SupportLayerObjectAffectedTarget),
}

impl JsonSchema for SupportLayerPropertyAffectedTarget {
    fn schema_name() -> Cow<'static, str> {
        "SupportLayerPropertyAffectedTarget".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<SupportLayerCapabilityAffectedTarget>(),
            generator.subschema_for::<SupportLayerObjectAffectedTarget>(),
        ])
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
enum BranchedAffectedTargetKind {
    Metadata(MetadataPropertyAffectedTarget),
    Support(SupportLayerPropertyAffectedTarget),
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub(crate) struct BranchedAffectedTarget(BranchedAffectedTargetKind);

impl BranchedAffectedTarget {
    pub(crate) fn metadata_property(target: MetadataPropertyAffectedTarget) -> Self {
        Self(BranchedAffectedTargetKind::Metadata(target))
    }

    #[allow(dead_code)]
    pub(crate) fn support_layer_property(
        layer_id: SupportLayerId,
        object_id: Option<MetadataObjectId>,
        property_path: PropertyPath,
    ) -> Self {
        let target = match object_id {
            Some(object_id) => {
                SupportLayerPropertyAffectedTarget::ObjectState(SupportLayerObjectAffectedTarget {
                    target_kind: SupportLayerPropertyTargetKind::Value,
                    layer_id,
                    object_id,
                    property_path,
                })
            }
            None => SupportLayerPropertyAffectedTarget::LayerCapability(
                SupportLayerCapabilityAffectedTarget {
                    target_kind: SupportLayerPropertyTargetKind::Value,
                    layer_id,
                    property_path,
                },
            ),
        };
        Self(BranchedAffectedTargetKind::Support(target))
    }
}

impl JsonSchema for BranchedAffectedTarget {
    fn inline_schema() -> bool {
        true
    }

    fn schema_name() -> Cow<'static, str> {
        "BranchedAffectedTarget".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<MetadataPropertyAffectedTarget>(),
            generator.subschema_for::<SupportLayerPropertyAffectedTarget>(),
        ])
    }
}

fn validate_canonical<T: Ord>(
    values: &[T],
    non_empty: bool,
    what: &'static str,
) -> Result<(), ChangeReceiptContractError> {
    if values.len() > MAX_RECEIPT_ITEMS || non_empty && values.is_empty() {
        return Err(ChangeReceiptContractError(what));
    }
    if values.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err(ChangeReceiptContractError(what));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
struct AffectedTargets(Vec<BranchedAffectedTarget>);

impl AffectedTargets {
    fn new(values: Vec<BranchedAffectedTarget>) -> Result<Self, ChangeReceiptContractError> {
        validate_canonical(
            &values,
            true,
            "affected targets must be non-empty and canonical",
        )?;
        Ok(Self(values))
    }
}

impl<'de> Deserialize<'de> for AffectedTargets {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Self::new(Vec::<BranchedAffectedTarget>::deserialize(deserializer)?)
            .map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for AffectedTargets {
    fn inline_schema() -> bool {
        true
    }
    fn schema_name() -> Cow<'static, str> {
        "BranchedAffectedTargets".into()
    }
    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "array",
            "items": generator.subschema_for::<BranchedAffectedTarget>(),
            "minItems": 1,
            "maxItems": MAX_RECEIPT_ITEMS,
            "uniqueItems": true
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
struct MergeResolutionAffectedTargets([MetadataPropertyAffectedTarget; 1]);

impl MergeResolutionAffectedTargets {
    fn new(target: MetadataPropertyAffectedTarget) -> Self {
        Self([target])
    }
}

impl JsonSchema for MergeResolutionAffectedTargets {
    fn inline_schema() -> bool {
        true
    }
    fn schema_name() -> Cow<'static, str> {
        "MergeResolutionAffectedTargets".into()
    }
    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "array",
            "items": generator.subschema_for::<MetadataPropertyAffectedTarget>(),
            "minItems": 1,
            "maxItems": 1,
            "uniqueItems": true
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
struct NonEmptyCanonicalIds(Vec<UnicaId>);

impl NonEmptyCanonicalIds {
    fn new(values: Vec<UnicaId>) -> Result<Self, ChangeReceiptContractError> {
        validate_canonical(&values, true, "IDs must be non-empty and canonical")?;
        Ok(Self(values))
    }
}

impl<'de> Deserialize<'de> for NonEmptyCanonicalIds {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Self::new(Vec::<UnicaId>::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for NonEmptyCanonicalIds {
    fn inline_schema() -> bool {
        true
    }
    fn schema_name() -> Cow<'static, str> {
        "NonEmptyCanonicalUnicaIds".into()
    }
    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "array", "items": generator.subschema_for::<UnicaId>(),
            "minItems": 1, "maxItems": MAX_RECEIPT_ITEMS, "uniqueItems": true
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
struct CanonicalIds(Vec<UnicaId>);

impl CanonicalIds {
    fn new(values: Vec<UnicaId>) -> Result<Self, ChangeReceiptContractError> {
        validate_canonical(&values, false, "IDs must be canonical and duplicate-free")?;
        Ok(Self(values))
    }
}

impl<'de> Deserialize<'de> for CanonicalIds {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Self::new(Vec::<UnicaId>::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for CanonicalIds {
    fn inline_schema() -> bool {
        true
    }
    fn schema_name() -> Cow<'static, str> {
        "CanonicalUnicaIds".into()
    }
    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "array", "items": generator.subschema_for::<UnicaId>(),
            "minItems": 0, "maxItems": MAX_RECEIPT_ITEMS, "uniqueItems": true
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
struct SequenceOrderedUniqueIds(Vec<UnicaId>);

impl SequenceOrderedUniqueIds {
    fn new(values: Vec<UnicaId>) -> Result<Self, ChangeReceiptContractError> {
        if values.len() > MAX_RECEIPT_ITEMS {
            return Err(ChangeReceiptContractError(
                "too many sequence-ordered receipt IDs",
            ));
        }
        let unique = values.iter().collect::<std::collections::BTreeSet<_>>();
        if unique.len() != values.len() {
            return Err(ChangeReceiptContractError(
                "sequence-ordered receipt IDs must be duplicate-free",
            ));
        }
        Ok(Self(values))
    }
}

impl<'de> Deserialize<'de> for SequenceOrderedUniqueIds {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Self::new(Vec::<UnicaId>::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for SequenceOrderedUniqueIds {
    fn inline_schema() -> bool {
        true
    }
    fn schema_name() -> Cow<'static, str> {
        "SequenceOrderedUniqueReceiptIds".into()
    }
    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "array", "items": generator.subschema_for::<UnicaId>(),
            "minItems": 0, "maxItems": MAX_RECEIPT_ITEMS, "uniqueItems": true
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(transparent)]
struct EmptyUnicaIds([UnicaId; 0]);

impl JsonSchema for EmptyUnicaIds {
    fn inline_schema() -> bool {
        true
    }
    fn schema_name() -> Cow<'static, str> {
        "EmptyUnicaIds".into()
    }
    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "array", "items": generator.subschema_for::<UnicaId>(),
            "minItems": 0, "maxItems": 0, "uniqueItems": true
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub(crate) struct ChangeReceiptSequence(u64);

impl ChangeReceiptSequence {
    pub(crate) fn new(value: u64) -> Result<Self, ChangeReceiptContractError> {
        (1..=MAX_I_JSON_INTEGER)
            .contains(&value)
            .then_some(Self(value))
            .ok_or(ChangeReceiptContractError(
                "receipt sequence must be a positive I-JSON safe integer",
            ))
    }
}

impl<'de> Deserialize<'de> for ChangeReceiptSequence {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Self::new(u64::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for ChangeReceiptSequence {
    fn inline_schema() -> bool {
        true
    }
    fn schema_name() -> Cow<'static, str> {
        "ChangeReceiptSequence".into()
    }
    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({"type":"integer", "minimum":1, "maximum":MAX_I_JSON_INTEGER})
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct TaskWorkspaceChangeReceiptDigestRecord {
    context_kind: TaskWorkspaceContextKind,
    mutation_outcome: ChangedMutationOutcome,
    change_receipt_id: UnicaId,
    affected_targets: AffectedTargets,
    before_sha256: Sha256Digest,
    after_sha256: Sha256Digest,
    event_ids: NonEmptyCanonicalIds,
    invalidated_evidence_ids: CanonicalIds,
    phase_transition: TaskWorkspaceChangedPhaseTransition,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct TaskWorkspaceNoChangeReceiptDigestRecord {
    context_kind: TaskWorkspaceContextKind,
    mutation_outcome: NoChangeMutationOutcome,
    change_receipt_id: UnicaId,
    affected_targets: AffectedTargets,
    content_sha256: Sha256Digest,
    event_ids: EmptyUnicaIds,
    invalidated_evidence_ids: EmptyUnicaIds,
    phase_transition: TaskWorkspaceNoChangePhaseTransition,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct MergeResolutionChangeReceiptDigestRecord {
    context_kind: MergeResolutionContextKind,
    mutation_outcome: ChangedMutationOutcome,
    change_receipt_id: UnicaId,
    affected_targets: MergeResolutionAffectedTargets,
    before_sha256: Sha256Digest,
    after_sha256: Sha256Digest,
    event_ids: NonEmptyCanonicalIds,
    invalidated_evidence_ids: EmptyUnicaIds,
    superseded_change_receipt_ids: SequenceOrderedUniqueIds,
    superseded_decision_ids: CanonicalIds,
    #[serde(skip_serializing_if = "Option::is_none")]
    pending_replacement_decision_id: Option<UnicaId>,
    decision_set_digest_before: Sha256Digest,
    revised_decision_set_digest: Sha256Digest,
    phase_transition: MergeResolutionPhaseTransition,
    base_session_digest: Sha256Digest,
    workspace_generation_id: UnicaId,
    receipt_sequence: ChangeReceiptSequence,
}

impl JsonSchema for MergeResolutionChangeReceiptDigestRecord {
    fn schema_name() -> Cow<'static, str> {
        "MergeResolutionChangeReceiptDigestRecord".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        merge_changed_schema(generator, false)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct MergeResolutionNoChangeReceiptDigestRecord {
    context_kind: MergeResolutionContextKind,
    mutation_outcome: NoChangeMutationOutcome,
    change_receipt_id: UnicaId,
    affected_targets: MergeResolutionAffectedTargets,
    content_sha256: Sha256Digest,
    event_ids: EmptyUnicaIds,
    invalidated_evidence_ids: EmptyUnicaIds,
    superseded_change_receipt_ids: EmptyUnicaIds,
    superseded_decision_ids: EmptyUnicaIds,
    decision_set_digest: Sha256Digest,
    phase_transition: MergeResolutionPhaseTransition,
    base_session_digest: Sha256Digest,
    workspace_generation_id: UnicaId,
    receipt_sequence: ChangeReceiptSequence,
}

#[derive(Clone, Copy)]
enum MergeDecisionSchemaMode {
    Undecided,
    Current,
    ReplacementPending,
}

fn schema_value<T: JsonSchema>(generator: &mut SchemaGenerator) -> Value {
    serde_json::to_value(generator.subschema_for::<T>())
        .expect("a typed receipt schema is serializable")
}

fn closed_object_schema(properties: Map<String, Value>, required: Vec<Value>) -> Schema {
    let mut object = Map::new();
    object.insert("type".to_owned(), Value::String("object".to_owned()));
    object.insert("properties".to_owned(), Value::Object(properties));
    object.insert("required".to_owned(), Value::Array(required));
    object.insert("additionalProperties".to_owned(), Value::Bool(false));
    Schema::from(object)
}

fn merge_changed_schema(generator: &mut SchemaGenerator, include_digest: bool) -> Schema {
    one_of_schema(
        [
            MergeDecisionSchemaMode::Undecided,
            MergeDecisionSchemaMode::Current,
            MergeDecisionSchemaMode::ReplacementPending,
        ]
        .into_iter()
        .map(|mode| merge_changed_branch_schema(generator, mode, include_digest))
        .collect(),
    )
}

fn merge_changed_branch_schema(
    generator: &mut SchemaGenerator,
    decision_mode: MergeDecisionSchemaMode,
    include_digest: bool,
) -> Schema {
    let mut properties = Map::new();
    let mut required = Vec::new();
    macro_rules! property {
        ($wire:literal, $type:ty) => {{
            properties.insert($wire.to_owned(), schema_value::<$type>(generator));
            required.push(Value::String($wire.to_owned()));
        }};
    }
    property!("contextKind", MergeResolutionContextKind);
    property!("mutationOutcome", ChangedMutationOutcome);
    property!("changeReceiptId", UnicaId);
    property!("affectedTargets", MergeResolutionAffectedTargets);
    property!("beforeSha256", Sha256Digest);
    property!("afterSha256", Sha256Digest);
    property!("eventIds", NonEmptyCanonicalIds);
    property!("invalidatedEvidenceIds", EmptyUnicaIds);
    property!("supersededChangeReceiptIds", SequenceOrderedUniqueIds);
    match decision_mode {
        MergeDecisionSchemaMode::Undecided => {
            property!("supersededDecisionIds", EmptyUnicaIds);
        }
        MergeDecisionSchemaMode::Current => {
            properties.insert(
                "supersededDecisionIds".to_owned(),
                serde_json::json!({
                    "type": "array",
                    "items": generator.subschema_for::<UnicaId>(),
                    "minItems": 1,
                    "maxItems": 1,
                    "uniqueItems": true
                }),
            );
            required.push(Value::String("supersededDecisionIds".to_owned()));
            property!("pendingReplacementDecisionId", UnicaId);
        }
        MergeDecisionSchemaMode::ReplacementPending => {
            property!("supersededDecisionIds", EmptyUnicaIds);
            property!("pendingReplacementDecisionId", UnicaId);
        }
    }
    property!("decisionSetDigestBefore", Sha256Digest);
    property!("revisedDecisionSetDigest", Sha256Digest);
    property!("phaseTransition", MergeResolutionPhaseTransition);
    property!("baseSessionDigest", Sha256Digest);
    property!("workspaceGenerationId", UnicaId);
    property!("receiptSequence", ChangeReceiptSequence);
    if include_digest {
        property!("changeReceiptDigest", Sha256Digest);
    }
    closed_object_schema(properties, required)
}

macro_rules! seal_digest_record {
    ($type:ty) => {
        impl contract_digest_record_sealed::Sealed for $type {}
        impl ContractDigestRecord for $type {}
    };
}

seal_digest_record!(TaskWorkspaceChangeReceiptDigestRecord);
seal_digest_record!(TaskWorkspaceNoChangeReceiptDigestRecord);
seal_digest_record!(MergeResolutionChangeReceiptDigestRecord);
seal_digest_record!(MergeResolutionNoChangeReceiptDigestRecord);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct TaskWorkspaceChangeReceipt {
    #[serde(flatten)]
    record: TaskWorkspaceChangeReceiptDigestRecord,
    change_receipt_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct TaskWorkspaceNoChangeReceipt {
    #[serde(flatten)]
    record: TaskWorkspaceNoChangeReceiptDigestRecord,
    change_receipt_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct MergeResolutionChangeReceipt {
    #[serde(flatten)]
    record: MergeResolutionChangeReceiptDigestRecord,
    change_receipt_digest: Sha256Digest,
}

/// Internal immutable projection shared with status/merge transition
/// authorities. It can only be produced from a digest-validated changed
/// merge-resolution receipt; no no-change or task receipt can be relabelled.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct MergeResolutionChangedReceiptProjection {
    pub(crate) change_receipt_id: UnicaId,
    pub(crate) affected_target: MetadataPropertyAffectedTarget,
    pub(crate) after_sha256: Sha256Digest,
    pub(crate) change_receipt_digest: Sha256Digest,
    pub(crate) superseded_change_receipt_ids: Vec<UnicaId>,
    pub(crate) superseded_decision_ids: Vec<UnicaId>,
    pub(crate) pending_replacement_decision_id: Option<UnicaId>,
    pub(crate) decision_set_digest_before: Sha256Digest,
    pub(crate) revised_decision_set_digest: Sha256Digest,
    pub(crate) base_session_digest: Sha256Digest,
    pub(crate) workspace_generation_id: UnicaId,
    pub(crate) receipt_sequence: ChangeReceiptSequence,
}

impl JsonSchema for MergeResolutionChangeReceipt {
    fn schema_name() -> Cow<'static, str> {
        "MergeResolutionChangeReceipt".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        merge_changed_schema(generator, true)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct MergeResolutionNoChangeReceipt {
    #[serde(flatten)]
    record: MergeResolutionNoChangeReceiptDigestRecord,
    change_receipt_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub(crate) enum TaskWorkspaceReceipt {
    Changed(TaskWorkspaceChangeReceipt),
    NoChange(TaskWorkspaceNoChangeReceipt),
}

impl JsonSchema for TaskWorkspaceReceipt {
    fn schema_name() -> Cow<'static, str> {
        "TaskWorkspaceChangeReceiptUnion".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<TaskWorkspaceChangeReceipt>(),
            generator.subschema_for::<TaskWorkspaceNoChangeReceipt>(),
        ])
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub(crate) enum MergeResolutionReceipt {
    Changed(MergeResolutionChangeReceipt),
    NoChange(MergeResolutionNoChangeReceipt),
}

impl JsonSchema for MergeResolutionReceipt {
    fn schema_name() -> Cow<'static, str> {
        "MergeResolutionChangeReceiptUnion".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<MergeResolutionChangeReceipt>(),
            generator.subschema_for::<MergeResolutionNoChangeReceipt>(),
        ])
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub(crate) enum BranchedChangeReceipt {
    TaskWorkspace(TaskWorkspaceReceipt),
    MergeResolution(MergeResolutionReceipt),
}

impl JsonSchema for BranchedChangeReceipt {
    fn schema_name() -> Cow<'static, str> {
        "BranchedChangeReceipt".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<TaskWorkspaceReceipt>(),
            generator.subschema_for::<MergeResolutionReceipt>(),
        ])
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct UnvalidatedTaskWorkspaceChangeReceipt {
    #[serde(flatten)]
    record: UnvalidatedTaskWorkspaceChangeReceiptDigestRecord,
    change_receipt_digest: Sha256Digest,
}

impl<'de> Deserialize<'de> for UnvalidatedTaskWorkspaceChangeReceipt {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            #[serde(flatten)]
            record: UnvalidatedTaskWorkspaceChangeReceiptDigestRecord,
            change_receipt_digest: Sha256Digest,
        }
        let wire = Wire::deserialize(deserializer)?;
        if wire.record.before_sha256 == wire.record.after_sha256 {
            return Err(serde::de::Error::custom("changed hashes must differ"));
        }
        Ok(Self {
            record: wire.record,
            change_receipt_digest: wire.change_receipt_digest,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UnvalidatedTaskWorkspaceChangeReceiptDigestRecord {
    context_kind: TaskWorkspaceContextKind,
    mutation_outcome: ChangedMutationOutcome,
    change_receipt_id: UnicaId,
    affected_targets: AffectedTargets,
    before_sha256: Sha256Digest,
    after_sha256: Sha256Digest,
    event_ids: NonEmptyCanonicalIds,
    invalidated_evidence_ids: CanonicalIds,
    phase_transition: TaskWorkspaceChangedPhaseTransition,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct UnvalidatedTaskWorkspaceNoChangeReceipt {
    #[serde(flatten)]
    record: UnvalidatedTaskWorkspaceNoChangeReceiptDigestRecord,
    change_receipt_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UnvalidatedTaskWorkspaceNoChangeReceiptDigestRecord {
    context_kind: TaskWorkspaceContextKind,
    mutation_outcome: NoChangeMutationOutcome,
    change_receipt_id: UnicaId,
    affected_targets: AffectedTargets,
    content_sha256: Sha256Digest,
    event_ids: EmptyUnicaIds,
    invalidated_evidence_ids: EmptyUnicaIds,
    phase_transition: TaskWorkspaceNoChangePhaseTransition,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct UnvalidatedMergeResolutionChangeReceipt {
    #[serde(flatten)]
    record: UnvalidatedMergeResolutionChangeReceiptDigestRecord,
    change_receipt_digest: Sha256Digest,
}

impl<'de> Deserialize<'de> for UnvalidatedMergeResolutionChangeReceipt {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            #[serde(flatten)]
            record: UnvalidatedMergeResolutionChangeReceiptDigestRecord,
            change_receipt_digest: Sha256Digest,
        }
        let wire = Wire::deserialize(deserializer)?;
        validate_unvalidated_merge_changed(&wire.record).map_err(serde::de::Error::custom)?;
        Ok(Self {
            record: wire.record,
            change_receipt_digest: wire.change_receipt_digest,
        })
    }
}

impl JsonSchema for UnvalidatedMergeResolutionChangeReceipt {
    fn schema_name() -> Cow<'static, str> {
        "UnvalidatedMergeResolutionChangeReceipt".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        merge_changed_schema(generator, true)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UnvalidatedMergeResolutionChangeReceiptDigestRecord {
    context_kind: MergeResolutionContextKind,
    mutation_outcome: ChangedMutationOutcome,
    change_receipt_id: UnicaId,
    affected_targets: MergeResolutionAffectedTargets,
    before_sha256: Sha256Digest,
    after_sha256: Sha256Digest,
    event_ids: NonEmptyCanonicalIds,
    invalidated_evidence_ids: EmptyUnicaIds,
    superseded_change_receipt_ids: SequenceOrderedUniqueIds,
    superseded_decision_ids: CanonicalIds,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pending_replacement_decision_id: Option<UnicaId>,
    decision_set_digest_before: Sha256Digest,
    revised_decision_set_digest: Sha256Digest,
    phase_transition: MergeResolutionPhaseTransition,
    base_session_digest: Sha256Digest,
    workspace_generation_id: UnicaId,
    receipt_sequence: ChangeReceiptSequence,
}

impl JsonSchema for UnvalidatedMergeResolutionChangeReceiptDigestRecord {
    fn schema_name() -> Cow<'static, str> {
        "UnvalidatedMergeResolutionChangeReceiptDigestRecord".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        merge_changed_schema(generator, false)
    }
}

fn validate_unvalidated_merge_changed(
    record: &UnvalidatedMergeResolutionChangeReceiptDigestRecord,
) -> Result<(), ChangeReceiptContractError> {
    if record.before_sha256 == record.after_sha256 {
        return Err(ChangeReceiptContractError("changed hashes must differ"));
    }
    let superseded_decisions = &record.superseded_decision_ids.0;
    match &record.pending_replacement_decision_id {
        None if superseded_decisions.is_empty()
            && record.decision_set_digest_before == record.revised_decision_set_digest =>
        {
            Ok(())
        }
        Some(pending)
            if record.decision_set_digest_before != record.revised_decision_set_digest
                && (superseded_decisions.is_empty()
                    || superseded_decisions.as_slice() == std::slice::from_ref(pending)) =>
        {
            Ok(())
        }
        _ => Err(ChangeReceiptContractError(
            "merge-resolution decision lineage is internally inconsistent",
        )),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct UnvalidatedMergeResolutionNoChangeReceipt {
    #[serde(flatten)]
    record: UnvalidatedMergeResolutionNoChangeReceiptDigestRecord,
    change_receipt_digest: Sha256Digest,
}

impl<'de> Deserialize<'de> for UnvalidatedMergeResolutionNoChangeReceipt {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            #[serde(flatten)]
            record: UnvalidatedMergeResolutionNoChangeReceiptDigestRecord,
            change_receipt_digest: Sha256Digest,
        }
        let wire = Wire::deserialize(deserializer)?;
        Ok(Self {
            record: wire.record,
            change_receipt_digest: wire.change_receipt_digest,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UnvalidatedMergeResolutionNoChangeReceiptDigestRecord {
    context_kind: MergeResolutionContextKind,
    mutation_outcome: NoChangeMutationOutcome,
    change_receipt_id: UnicaId,
    affected_targets: MergeResolutionAffectedTargets,
    content_sha256: Sha256Digest,
    event_ids: EmptyUnicaIds,
    invalidated_evidence_ids: EmptyUnicaIds,
    superseded_change_receipt_ids: EmptyUnicaIds,
    superseded_decision_ids: EmptyUnicaIds,
    decision_set_digest: Sha256Digest,
    phase_transition: MergeResolutionPhaseTransition,
    base_session_digest: Sha256Digest,
    workspace_generation_id: UnicaId,
    receipt_sequence: ChangeReceiptSequence,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub(crate) enum UnvalidatedBranchedChangeReceipt {
    TaskChanged(UnvalidatedTaskWorkspaceChangeReceipt),
    TaskNoChange(UnvalidatedTaskWorkspaceNoChangeReceipt),
    MergeChanged(UnvalidatedMergeResolutionChangeReceipt),
    MergeNoChange(UnvalidatedMergeResolutionNoChangeReceipt),
}

impl JsonSchema for UnvalidatedBranchedChangeReceipt {
    fn schema_name() -> Cow<'static, str> {
        "UnvalidatedBranchedChangeReceipt".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            one_of_schema(vec![
                generator.subschema_for::<UnvalidatedTaskWorkspaceChangeReceipt>(),
                generator.subschema_for::<UnvalidatedTaskWorkspaceNoChangeReceipt>(),
            ]),
            one_of_schema(vec![
                generator.subschema_for::<UnvalidatedMergeResolutionChangeReceipt>(),
                generator.subschema_for::<UnvalidatedMergeResolutionNoChangeReceipt>(),
            ]),
        ])
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MergeResolutionSelectableReceiptAuthority {
    change_receipt_id: UnicaId,
    receipt_sequence: ChangeReceiptSequence,
    workspace_generation_id: UnicaId,
    affected_target: MetadataPropertyAffectedTarget,
}

impl MergeResolutionSelectableReceiptAuthority {
    #[cfg(test)]
    pub(crate) fn test_only(
        change_receipt_id: UnicaId,
        receipt_sequence: ChangeReceiptSequence,
        workspace_generation_id: UnicaId,
        affected_target: MetadataPropertyAffectedTarget,
    ) -> Self {
        Self {
            change_receipt_id,
            receipt_sequence,
            workspace_generation_id,
            affected_target,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum DecisionLineageKind {
    Undecided {
        digest: Sha256Digest,
    },
    Current {
        decision_id: UnicaId,
        before: Sha256Digest,
        revised: Sha256Digest,
    },
    ReplacementPending {
        decision_id: UnicaId,
        before: Sha256Digest,
        revised: Sha256Digest,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MergeResolutionDecisionLineageAuthority(DecisionLineageKind);

impl MergeResolutionDecisionLineageAuthority {
    #[cfg(test)]
    pub(crate) fn current_test_only(
        decision_id: UnicaId,
        before: Sha256Digest,
        revised: Sha256Digest,
    ) -> Self {
        Self(DecisionLineageKind::Current {
            decision_id,
            before,
            revised,
        })
    }

    #[cfg(test)]
    pub(crate) fn replacement_pending_test_only(
        decision_id: UnicaId,
        before: Sha256Digest,
        revised: Sha256Digest,
    ) -> Self {
        Self(DecisionLineageKind::ReplacementPending {
            decision_id,
            before,
            revised,
        })
    }

    #[cfg(test)]
    pub(crate) fn undecided_test_only(digest: Sha256Digest) -> Self {
        Self(DecisionLineageKind::Undecided { digest })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ReceiptAuthorityKind {
    TaskChanged {
        change_receipt_id: UnicaId,
        affected_targets: AffectedTargets,
        before: Sha256Digest,
        after: Sha256Digest,
        event_ids: NonEmptyCanonicalIds,
        invalidated_evidence_ids: CanonicalIds,
        phase: CompatibleTaskMutationPhase,
    },
    TaskNoChange {
        change_receipt_id: UnicaId,
        affected_targets: AffectedTargets,
        content: Sha256Digest,
        phase: CompatibleTaskMutationPhase,
    },
    MergeChanged {
        change_receipt_id: UnicaId,
        target: MetadataPropertyAffectedTarget,
        before: Sha256Digest,
        after: Sha256Digest,
        event_ids: NonEmptyCanonicalIds,
        superseded: Vec<MergeResolutionSelectableReceiptAuthority>,
        decision: MergeResolutionDecisionLineageAuthority,
        base_session_digest: Sha256Digest,
        workspace_generation_id: UnicaId,
        receipt_sequence: ChangeReceiptSequence,
    },
    MergeNoChange {
        change_receipt_id: UnicaId,
        target: MetadataPropertyAffectedTarget,
        content: Sha256Digest,
        decision_set_digest: Sha256Digest,
        base_session_digest: Sha256Digest,
        workspace_generation_id: UnicaId,
        receipt_sequence: ChangeReceiptSequence,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BranchedChangeReceiptAuthority(ReceiptAuthorityKind);

impl BranchedChangeReceiptAuthority {
    #[cfg(test)]
    pub(crate) fn task_workspace_changed_test_only(
        change_receipt_id: UnicaId,
        affected_targets: Vec<BranchedAffectedTarget>,
        before: Sha256Digest,
        after: Sha256Digest,
        event_ids: Vec<UnicaId>,
        invalidated_evidence_ids: Vec<UnicaId>,
        phase: TaskPhase,
    ) -> Result<Self, ChangeReceiptContractError> {
        Ok(Self(ReceiptAuthorityKind::TaskChanged {
            change_receipt_id,
            affected_targets: AffectedTargets::new(affected_targets)?,
            before,
            after,
            event_ids: NonEmptyCanonicalIds::new(event_ids)?,
            invalidated_evidence_ids: CanonicalIds::new(invalidated_evidence_ids)?,
            phase: CompatibleTaskMutationPhase::try_from(phase)?,
        }))
    }

    #[cfg(test)]
    pub(crate) fn task_workspace_no_change_test_only(
        change_receipt_id: UnicaId,
        affected_targets: Vec<BranchedAffectedTarget>,
        content: Sha256Digest,
        phase: TaskPhase,
    ) -> Result<Self, ChangeReceiptContractError> {
        Ok(Self(ReceiptAuthorityKind::TaskNoChange {
            change_receipt_id,
            affected_targets: AffectedTargets::new(affected_targets)?,
            content,
            phase: CompatibleTaskMutationPhase::try_from(phase)?,
        }))
    }

    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn merge_resolution_changed_test_only(
        change_receipt_id: UnicaId,
        target: MetadataPropertyAffectedTarget,
        before: Sha256Digest,
        after: Sha256Digest,
        event_ids: Vec<UnicaId>,
        superseded: Vec<MergeResolutionSelectableReceiptAuthority>,
        decision: MergeResolutionDecisionLineageAuthority,
        base_session_digest: Sha256Digest,
        workspace_generation_id: UnicaId,
        receipt_sequence: ChangeReceiptSequence,
    ) -> Result<Self, ChangeReceiptContractError> {
        Ok(Self(ReceiptAuthorityKind::MergeChanged {
            change_receipt_id,
            target,
            before,
            after,
            event_ids: NonEmptyCanonicalIds::new(event_ids)?,
            superseded,
            decision,
            base_session_digest,
            workspace_generation_id,
            receipt_sequence,
        }))
    }

    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn merge_resolution_no_change_test_only(
        change_receipt_id: UnicaId,
        target: MetadataPropertyAffectedTarget,
        content: Sha256Digest,
        decision_set_digest: Sha256Digest,
        base_session_digest: Sha256Digest,
        workspace_generation_id: UnicaId,
        receipt_sequence: ChangeReceiptSequence,
    ) -> Result<Self, ChangeReceiptContractError> {
        Ok(Self(ReceiptAuthorityKind::MergeNoChange {
            change_receipt_id,
            target,
            content,
            decision_set_digest,
            base_session_digest,
            workspace_generation_id,
            receipt_sequence,
        }))
    }
}

impl BranchedChangeReceipt {
    pub(crate) fn new(
        authority: &BranchedChangeReceiptAuthority,
    ) -> Result<Self, ChangeReceiptContractError> {
        match &authority.0 {
            ReceiptAuthorityKind::TaskChanged {
                change_receipt_id,
                affected_targets,
                before,
                after,
                event_ids,
                invalidated_evidence_ids,
                phase,
            } => {
                if before == after {
                    return Err(ChangeReceiptContractError("changed hashes must differ"));
                }
                let record = TaskWorkspaceChangeReceiptDigestRecord {
                    context_kind: TaskWorkspaceContextKind::Value,
                    mutation_outcome: ChangedMutationOutcome::Value,
                    change_receipt_id: change_receipt_id.clone(),
                    affected_targets: affected_targets.clone(),
                    before_sha256: before.clone(),
                    after_sha256: after.clone(),
                    event_ids: event_ids.clone(),
                    invalidated_evidence_ids: invalidated_evidence_ids.clone(),
                    phase_transition: TaskWorkspaceChangedPhaseTransition::new(*phase),
                };
                let digest = receipt_digest(&record)?;
                Ok(Self::TaskWorkspace(TaskWorkspaceReceipt::Changed(
                    TaskWorkspaceChangeReceipt {
                        record,
                        change_receipt_digest: digest,
                    },
                )))
            }
            ReceiptAuthorityKind::TaskNoChange {
                change_receipt_id,
                affected_targets,
                content,
                phase,
            } => {
                let record = TaskWorkspaceNoChangeReceiptDigestRecord {
                    context_kind: TaskWorkspaceContextKind::Value,
                    mutation_outcome: NoChangeMutationOutcome::Value,
                    change_receipt_id: change_receipt_id.clone(),
                    affected_targets: affected_targets.clone(),
                    content_sha256: content.clone(),
                    event_ids: EmptyUnicaIds::default(),
                    invalidated_evidence_ids: EmptyUnicaIds::default(),
                    phase_transition: TaskWorkspaceNoChangePhaseTransition::new(*phase),
                };
                let digest = receipt_digest(&record)?;
                Ok(Self::TaskWorkspace(TaskWorkspaceReceipt::NoChange(
                    TaskWorkspaceNoChangeReceipt {
                        record,
                        change_receipt_digest: digest,
                    },
                )))
            }
            ReceiptAuthorityKind::MergeChanged {
                change_receipt_id,
                target,
                before,
                after,
                event_ids,
                superseded,
                decision,
                base_session_digest,
                workspace_generation_id,
                receipt_sequence,
            } => {
                if before == after {
                    return Err(ChangeReceiptContractError("changed hashes must differ"));
                }
                validate_superseded(
                    superseded,
                    target,
                    workspace_generation_id,
                    *receipt_sequence,
                    change_receipt_id,
                )?;
                let affected_targets = MergeResolutionAffectedTargets::new(target.clone());
                let superseded_change_receipt_ids = SequenceOrderedUniqueIds::new(
                    superseded
                        .iter()
                        .map(|item| item.change_receipt_id.clone())
                        .collect(),
                )?;
                let (
                    superseded_decision_ids,
                    pending_replacement_decision_id,
                    decision_set_digest_before,
                    revised_decision_set_digest,
                ) = decision_projection(decision)?;
                let record = MergeResolutionChangeReceiptDigestRecord {
                    context_kind: MergeResolutionContextKind::Value,
                    mutation_outcome: ChangedMutationOutcome::Value,
                    change_receipt_id: change_receipt_id.clone(),
                    affected_targets,
                    before_sha256: before.clone(),
                    after_sha256: after.clone(),
                    event_ids: event_ids.clone(),
                    invalidated_evidence_ids: EmptyUnicaIds::default(),
                    superseded_change_receipt_ids,
                    superseded_decision_ids,
                    pending_replacement_decision_id,
                    decision_set_digest_before,
                    revised_decision_set_digest,
                    phase_transition: MergeResolutionPhaseTransition::VALUE,
                    base_session_digest: base_session_digest.clone(),
                    workspace_generation_id: workspace_generation_id.clone(),
                    receipt_sequence: *receipt_sequence,
                };
                let digest = receipt_digest(&record)?;
                Ok(Self::MergeResolution(MergeResolutionReceipt::Changed(
                    MergeResolutionChangeReceipt {
                        record,
                        change_receipt_digest: digest,
                    },
                )))
            }
            ReceiptAuthorityKind::MergeNoChange {
                change_receipt_id,
                target,
                content,
                decision_set_digest,
                base_session_digest,
                workspace_generation_id,
                receipt_sequence,
            } => {
                let record = MergeResolutionNoChangeReceiptDigestRecord {
                    context_kind: MergeResolutionContextKind::Value,
                    mutation_outcome: NoChangeMutationOutcome::Value,
                    change_receipt_id: change_receipt_id.clone(),
                    affected_targets: MergeResolutionAffectedTargets::new(target.clone()),
                    content_sha256: content.clone(),
                    event_ids: EmptyUnicaIds::default(),
                    invalidated_evidence_ids: EmptyUnicaIds::default(),
                    superseded_change_receipt_ids: EmptyUnicaIds::default(),
                    superseded_decision_ids: EmptyUnicaIds::default(),
                    decision_set_digest: decision_set_digest.clone(),
                    phase_transition: MergeResolutionPhaseTransition::VALUE,
                    base_session_digest: base_session_digest.clone(),
                    workspace_generation_id: workspace_generation_id.clone(),
                    receipt_sequence: *receipt_sequence,
                };
                let digest = receipt_digest(&record)?;
                Ok(Self::MergeResolution(MergeResolutionReceipt::NoChange(
                    MergeResolutionNoChangeReceipt {
                        record,
                        change_receipt_digest: digest,
                    },
                )))
            }
        }
    }

    pub(crate) fn from_wire(
        wire: UnvalidatedBranchedChangeReceipt,
        authority: &BranchedChangeReceiptAuthority,
    ) -> Result<Self, ChangeReceiptContractError> {
        wire.validate_digest()?;
        let expected = Self::new(authority)?;
        if wire != expected.unvalidated_projection() {
            return Err(ChangeReceiptContractError(
                "receipt does not match its semantic authority",
            ));
        }
        Ok(expected)
    }

    pub(crate) const fn mutation_outcome(&self) -> MutationOutcome {
        match self {
            Self::TaskWorkspace(TaskWorkspaceReceipt::Changed(_))
            | Self::MergeResolution(MergeResolutionReceipt::Changed(_)) => MutationOutcome::Changed,
            Self::TaskWorkspace(TaskWorkspaceReceipt::NoChange(_))
            | Self::MergeResolution(MergeResolutionReceipt::NoChange(_)) => {
                MutationOutcome::NoChange
            }
        }
    }

    pub(crate) fn merge_resolution_changed_projection(
        &self,
    ) -> Option<MergeResolutionChangedReceiptProjection> {
        let Self::MergeResolution(MergeResolutionReceipt::Changed(value)) = self else {
            return None;
        };
        Some(MergeResolutionChangedReceiptProjection {
            change_receipt_id: value.record.change_receipt_id.clone(),
            affected_target: value.record.affected_targets.0[0].clone(),
            after_sha256: value.record.after_sha256.clone(),
            change_receipt_digest: value.change_receipt_digest.clone(),
            superseded_change_receipt_ids: value.record.superseded_change_receipt_ids.0.clone(),
            superseded_decision_ids: value.record.superseded_decision_ids.0.clone(),
            pending_replacement_decision_id: value.record.pending_replacement_decision_id.clone(),
            decision_set_digest_before: value.record.decision_set_digest_before.clone(),
            revised_decision_set_digest: value.record.revised_decision_set_digest.clone(),
            base_session_digest: value.record.base_session_digest.clone(),
            workspace_generation_id: value.record.workspace_generation_id.clone(),
            receipt_sequence: value.record.receipt_sequence,
        })
    }

    fn unvalidated_projection(&self) -> UnvalidatedBranchedChangeReceipt {
        match self {
            Self::TaskWorkspace(TaskWorkspaceReceipt::Changed(value)) => {
                UnvalidatedBranchedChangeReceipt::TaskChanged(
                    UnvalidatedTaskWorkspaceChangeReceipt {
                        record: UnvalidatedTaskWorkspaceChangeReceiptDigestRecord {
                            context_kind: value.record.context_kind,
                            mutation_outcome: value.record.mutation_outcome,
                            change_receipt_id: value.record.change_receipt_id.clone(),
                            affected_targets: value.record.affected_targets.clone(),
                            before_sha256: value.record.before_sha256.clone(),
                            after_sha256: value.record.after_sha256.clone(),
                            event_ids: value.record.event_ids.clone(),
                            invalidated_evidence_ids: value.record.invalidated_evidence_ids.clone(),
                            phase_transition: value.record.phase_transition,
                        },
                        change_receipt_digest: value.change_receipt_digest.clone(),
                    },
                )
            }
            Self::TaskWorkspace(TaskWorkspaceReceipt::NoChange(value)) => {
                UnvalidatedBranchedChangeReceipt::TaskNoChange(
                    UnvalidatedTaskWorkspaceNoChangeReceipt {
                        record: UnvalidatedTaskWorkspaceNoChangeReceiptDigestRecord {
                            context_kind: value.record.context_kind,
                            mutation_outcome: value.record.mutation_outcome,
                            change_receipt_id: value.record.change_receipt_id.clone(),
                            affected_targets: value.record.affected_targets.clone(),
                            content_sha256: value.record.content_sha256.clone(),
                            event_ids: value.record.event_ids.clone(),
                            invalidated_evidence_ids: value.record.invalidated_evidence_ids.clone(),
                            phase_transition: value.record.phase_transition,
                        },
                        change_receipt_digest: value.change_receipt_digest.clone(),
                    },
                )
            }
            Self::MergeResolution(MergeResolutionReceipt::Changed(value)) => {
                UnvalidatedBranchedChangeReceipt::MergeChanged(
                    UnvalidatedMergeResolutionChangeReceipt {
                        record: UnvalidatedMergeResolutionChangeReceiptDigestRecord {
                            context_kind: value.record.context_kind,
                            mutation_outcome: value.record.mutation_outcome,
                            change_receipt_id: value.record.change_receipt_id.clone(),
                            affected_targets: value.record.affected_targets.clone(),
                            before_sha256: value.record.before_sha256.clone(),
                            after_sha256: value.record.after_sha256.clone(),
                            event_ids: value.record.event_ids.clone(),
                            invalidated_evidence_ids: value.record.invalidated_evidence_ids.clone(),
                            superseded_change_receipt_ids: value
                                .record
                                .superseded_change_receipt_ids
                                .clone(),
                            superseded_decision_ids: value.record.superseded_decision_ids.clone(),
                            pending_replacement_decision_id: value
                                .record
                                .pending_replacement_decision_id
                                .clone(),
                            decision_set_digest_before: value
                                .record
                                .decision_set_digest_before
                                .clone(),
                            revised_decision_set_digest: value
                                .record
                                .revised_decision_set_digest
                                .clone(),
                            phase_transition: value.record.phase_transition,
                            base_session_digest: value.record.base_session_digest.clone(),
                            workspace_generation_id: value.record.workspace_generation_id.clone(),
                            receipt_sequence: value.record.receipt_sequence,
                        },
                        change_receipt_digest: value.change_receipt_digest.clone(),
                    },
                )
            }
            Self::MergeResolution(MergeResolutionReceipt::NoChange(value)) => {
                UnvalidatedBranchedChangeReceipt::MergeNoChange(
                    UnvalidatedMergeResolutionNoChangeReceipt {
                        record: UnvalidatedMergeResolutionNoChangeReceiptDigestRecord {
                            context_kind: value.record.context_kind,
                            mutation_outcome: value.record.mutation_outcome,
                            change_receipt_id: value.record.change_receipt_id.clone(),
                            affected_targets: value.record.affected_targets.clone(),
                            content_sha256: value.record.content_sha256.clone(),
                            event_ids: value.record.event_ids.clone(),
                            invalidated_evidence_ids: value.record.invalidated_evidence_ids.clone(),
                            superseded_change_receipt_ids: value
                                .record
                                .superseded_change_receipt_ids
                                .clone(),
                            superseded_decision_ids: value.record.superseded_decision_ids.clone(),
                            decision_set_digest: value.record.decision_set_digest.clone(),
                            phase_transition: value.record.phase_transition,
                            base_session_digest: value.record.base_session_digest.clone(),
                            workspace_generation_id: value.record.workspace_generation_id.clone(),
                            receipt_sequence: value.record.receipt_sequence,
                        },
                        change_receipt_digest: value.change_receipt_digest.clone(),
                    },
                )
            }
        }
    }
}

impl UnvalidatedBranchedChangeReceipt {
    fn validate_digest(&self) -> Result<(), ChangeReceiptContractError> {
        let (expected, actual) = match self {
            Self::TaskChanged(value) => {
                let record = TaskWorkspaceChangeReceiptDigestRecord {
                    context_kind: value.record.context_kind,
                    mutation_outcome: value.record.mutation_outcome,
                    change_receipt_id: value.record.change_receipt_id.clone(),
                    affected_targets: value.record.affected_targets.clone(),
                    before_sha256: value.record.before_sha256.clone(),
                    after_sha256: value.record.after_sha256.clone(),
                    event_ids: value.record.event_ids.clone(),
                    invalidated_evidence_ids: value.record.invalidated_evidence_ids.clone(),
                    phase_transition: value.record.phase_transition,
                };
                (receipt_digest(&record)?, &value.change_receipt_digest)
            }
            Self::TaskNoChange(value) => {
                let record = TaskWorkspaceNoChangeReceiptDigestRecord {
                    context_kind: value.record.context_kind,
                    mutation_outcome: value.record.mutation_outcome,
                    change_receipt_id: value.record.change_receipt_id.clone(),
                    affected_targets: value.record.affected_targets.clone(),
                    content_sha256: value.record.content_sha256.clone(),
                    event_ids: value.record.event_ids.clone(),
                    invalidated_evidence_ids: value.record.invalidated_evidence_ids.clone(),
                    phase_transition: value.record.phase_transition,
                };
                (receipt_digest(&record)?, &value.change_receipt_digest)
            }
            Self::MergeChanged(value) => {
                let record = MergeResolutionChangeReceiptDigestRecord {
                    context_kind: value.record.context_kind,
                    mutation_outcome: value.record.mutation_outcome,
                    change_receipt_id: value.record.change_receipt_id.clone(),
                    affected_targets: value.record.affected_targets.clone(),
                    before_sha256: value.record.before_sha256.clone(),
                    after_sha256: value.record.after_sha256.clone(),
                    event_ids: value.record.event_ids.clone(),
                    invalidated_evidence_ids: value.record.invalidated_evidence_ids.clone(),
                    superseded_change_receipt_ids: value
                        .record
                        .superseded_change_receipt_ids
                        .clone(),
                    superseded_decision_ids: value.record.superseded_decision_ids.clone(),
                    pending_replacement_decision_id: value
                        .record
                        .pending_replacement_decision_id
                        .clone(),
                    decision_set_digest_before: value.record.decision_set_digest_before.clone(),
                    revised_decision_set_digest: value.record.revised_decision_set_digest.clone(),
                    phase_transition: value.record.phase_transition,
                    base_session_digest: value.record.base_session_digest.clone(),
                    workspace_generation_id: value.record.workspace_generation_id.clone(),
                    receipt_sequence: value.record.receipt_sequence,
                };
                (receipt_digest(&record)?, &value.change_receipt_digest)
            }
            Self::MergeNoChange(value) => {
                let record = MergeResolutionNoChangeReceiptDigestRecord {
                    context_kind: value.record.context_kind,
                    mutation_outcome: value.record.mutation_outcome,
                    change_receipt_id: value.record.change_receipt_id.clone(),
                    affected_targets: value.record.affected_targets.clone(),
                    content_sha256: value.record.content_sha256.clone(),
                    event_ids: value.record.event_ids.clone(),
                    invalidated_evidence_ids: value.record.invalidated_evidence_ids.clone(),
                    superseded_change_receipt_ids: value
                        .record
                        .superseded_change_receipt_ids
                        .clone(),
                    superseded_decision_ids: value.record.superseded_decision_ids.clone(),
                    decision_set_digest: value.record.decision_set_digest.clone(),
                    phase_transition: value.record.phase_transition,
                    base_session_digest: value.record.base_session_digest.clone(),
                    workspace_generation_id: value.record.workspace_generation_id.clone(),
                    receipt_sequence: value.record.receipt_sequence,
                };
                (receipt_digest(&record)?, &value.change_receipt_digest)
            }
        };
        if &expected != actual {
            return Err(ChangeReceiptContractError("change receipt digest mismatch"));
        }
        Ok(())
    }
}

fn receipt_digest<T: ContractDigestRecord>(
    record: &T,
) -> Result<Sha256Digest, ChangeReceiptContractError> {
    canonical_contract_digest(record, None)
        .map_err(|_| ChangeReceiptContractError("change receipt digest computation failed"))
}

fn validate_superseded(
    values: &[MergeResolutionSelectableReceiptAuthority],
    target: &MetadataPropertyAffectedTarget,
    generation: &UnicaId,
    current_sequence: ChangeReceiptSequence,
    current_id: &UnicaId,
) -> Result<(), ChangeReceiptContractError> {
    if values.len() > MAX_RECEIPT_ITEMS {
        return Err(ChangeReceiptContractError("too many superseded receipts"));
    }
    let mut previous = None;
    for value in values {
        if &value.workspace_generation_id != generation
            || &value.affected_target != target
            || value.receipt_sequence >= current_sequence
            || &value.change_receipt_id == current_id
            || previous.is_some_and(|sequence| sequence >= value.receipt_sequence)
        {
            return Err(ChangeReceiptContractError(
                "superseded receipt lineage does not match the current target/generation/sequence",
            ));
        }
        previous = Some(value.receipt_sequence);
    }
    let ids = values
        .iter()
        .map(|value| &value.change_receipt_id)
        .collect::<std::collections::BTreeSet<_>>();
    if ids.len() != values.len() {
        return Err(ChangeReceiptContractError(
            "superseded receipt IDs must be unique",
        ));
    }
    Ok(())
}

fn decision_projection(
    authority: &MergeResolutionDecisionLineageAuthority,
) -> Result<(CanonicalIds, Option<UnicaId>, Sha256Digest, Sha256Digest), ChangeReceiptContractError>
{
    match &authority.0 {
        DecisionLineageKind::Undecided { digest } => Ok((
            CanonicalIds(Vec::new()),
            None,
            digest.clone(),
            digest.clone(),
        )),
        DecisionLineageKind::Current {
            decision_id,
            before,
            revised,
        } => {
            if before == revised {
                return Err(ChangeReceiptContractError(
                    "decision-set digests must change when a replacement is pending",
                ));
            }
            Ok((
                CanonicalIds(vec![decision_id.clone()]),
                Some(decision_id.clone()),
                before.clone(),
                revised.clone(),
            ))
        }
        DecisionLineageKind::ReplacementPending {
            decision_id,
            before,
            revised,
        } => {
            if before == revised {
                return Err(ChangeReceiptContractError(
                    "decision-set digests must change when a replacement is pending",
                ));
            }
            Ok((
                CanonicalIds(Vec::new()),
                Some(decision_id.clone()),
                before.clone(),
                revised.clone(),
            ))
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ChangeReceiptContractError(&'static str);

impl fmt::Display for ChangeReceiptContractError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.0)
    }
}

impl std::error::Error for ChangeReceiptContractError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::branched_development::contracts::schema::audit_json_schema;
    use crate::domain::branched_development::TaskPhase;
    use schemars::{schema_for, JsonSchema};
    use serde::de::DeserializeOwned;
    use serde_json::{json, Value};

    const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const OBJECT_A: &str = "00000000-0000-0000-0000-000000000001";
    const OBJECT_B: &str = "00000000-0000-0000-0000-000000000002";
    const ID_1: &str = "11111111-1111-4111-8111-111111111111";
    const ID_2: &str = "22222222-2222-4222-8222-222222222222";
    const ID_3: &str = "33333333-3333-4333-8333-333333333333";
    const ID_4: &str = "44444444-4444-4444-8444-444444444444";
    const ID_5: &str = "55555555-5555-4555-8555-555555555555";

    fn id(value: &str) -> UnicaId {
        UnicaId::parse(value).unwrap()
    }

    fn object(value: &str) -> MetadataObjectId {
        MetadataObjectId::parse(value).unwrap()
    }

    fn digest(value: &str) -> Sha256Digest {
        Sha256Digest::parse(value).unwrap()
    }

    fn metadata_target(object_id: &str, property: &str) -> MetadataPropertyAffectedTarget {
        MetadataPropertyAffectedTarget::new(
            object(object_id),
            PropertyPath::parse(property).unwrap(),
        )
    }

    fn task_changed_authority(
        before: &str,
        after: &str,
        phase: TaskPhase,
    ) -> BranchedChangeReceiptAuthority {
        BranchedChangeReceiptAuthority::task_workspace_changed_test_only(
            id(ID_1),
            vec![BranchedAffectedTarget::metadata_property(metadata_target(
                OBJECT_A,
                "Attributes.Name",
            ))],
            digest(before),
            digest(after),
            vec![id(ID_2)],
            vec![id(ID_3)],
            phase,
        )
        .unwrap()
    }

    fn task_no_change_authority(phase: TaskPhase) -> BranchedChangeReceiptAuthority {
        BranchedChangeReceiptAuthority::task_workspace_no_change_test_only(
            id(ID_1),
            vec![BranchedAffectedTarget::metadata_property(metadata_target(
                OBJECT_A,
                "Attributes.Name",
            ))],
            digest(A),
            phase,
        )
        .unwrap()
    }

    fn merge_changed_authority() -> BranchedChangeReceiptAuthority {
        let target = metadata_target(OBJECT_A, "Module.Text");
        let generation_id = id(ID_4);
        let prior = MergeResolutionSelectableReceiptAuthority::test_only(
            id(ID_2),
            ChangeReceiptSequence::new(1).unwrap(),
            generation_id.clone(),
            target.clone(),
        );
        let decision = MergeResolutionDecisionLineageAuthority::current_test_only(
            id(ID_3),
            digest(A),
            digest(B),
        );
        BranchedChangeReceiptAuthority::merge_resolution_changed_test_only(
            id(ID_1),
            target,
            digest(A),
            digest(B),
            vec![id(ID_5)],
            vec![prior],
            decision,
            digest(A),
            generation_id,
            ChangeReceiptSequence::new(2).unwrap(),
        )
        .unwrap()
    }

    fn merge_no_change_authority() -> BranchedChangeReceiptAuthority {
        BranchedChangeReceiptAuthority::merge_resolution_no_change_test_only(
            id(ID_1),
            metadata_target(OBJECT_A, "Module.Text"),
            digest(A),
            digest(B),
            digest(A),
            id(ID_4),
            ChangeReceiptSequence::new(1).unwrap(),
        )
        .unwrap()
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

    fn assert_closed<T: JsonSchema>() {
        audit_json_schema(&schema::<T>()).unwrap();
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

    assert_not_deserialize_owned!(BranchedChangeReceipt);
    assert_not_deserialize_owned!(TaskWorkspaceChangeReceipt);
    assert_not_deserialize_owned!(TaskWorkspaceNoChangeReceipt);
    assert_not_deserialize_owned!(MergeResolutionChangeReceipt);
    assert_not_deserialize_owned!(MergeResolutionNoChangeReceipt);
    assert_not_deserialize_owned!(BranchedChangeReceiptAuthority);
    assert_not_deserialize_owned!(MergeResolutionSelectableReceiptAuthority);
    assert_not_deserialize_owned!(MergeResolutionDecisionLineageAuthority);

    #[test]
    fn outer_union_has_two_contexts_and_exactly_four_closed_leaves() {
        let receipts = [
            BranchedChangeReceipt::new(&task_changed_authority(A, B, TaskPhase::LocalVerified))
                .unwrap(),
            BranchedChangeReceipt::new(&task_no_change_authority(TaskPhase::Synchronized)).unwrap(),
            BranchedChangeReceipt::new(&merge_changed_authority()).unwrap(),
            BranchedChangeReceipt::new(&merge_no_change_authority()).unwrap(),
        ];
        assert_eq!(
            receipts
                .iter()
                .map(|receipt| {
                    let value = serde_json::to_value(receipt).unwrap();
                    (
                        value["contextKind"].as_str().unwrap().to_owned(),
                        value["mutationOutcome"].as_str().unwrap().to_owned(),
                    )
                })
                .collect::<Vec<_>>(),
            vec![
                ("taskWorkspaceChange".to_owned(), "changed".to_owned()),
                ("taskWorkspaceChange".to_owned(), "noChange".to_owned()),
                ("mergeResolutionChange".to_owned(), "changed".to_owned()),
                ("mergeResolutionChange".to_owned(), "noChange".to_owned()),
            ]
        );
        assert_eq!(receipts[0].mutation_outcome(), MutationOutcome::Changed);
        assert_eq!(receipts[1].mutation_outcome(), MutationOutcome::NoChange);

        assert_closed::<BranchedChangeReceipt>();
        assert_closed::<UnvalidatedBranchedChangeReceipt>();
        assert_closed::<TaskWorkspaceChangeReceipt>();
        assert_closed::<TaskWorkspaceNoChangeReceipt>();
        assert_closed::<MergeResolutionChangeReceipt>();
        assert_closed::<MergeResolutionNoChangeReceipt>();
        assert_closed::<TaskWorkspaceChangeReceiptDigestRecord>();
        assert_closed::<TaskWorkspaceNoChangeReceiptDigestRecord>();
        assert_closed::<MergeResolutionChangeReceiptDigestRecord>();
        assert_closed::<MergeResolutionNoChangeReceiptDigestRecord>();
    }

    #[test]
    fn receipt_schema_preserves_the_outer_context_and_inner_outcome_boundaries() {
        let receipt_schema = schema::<BranchedChangeReceipt>();
        let outer = receipt_schema["oneOf"]
            .as_array()
            .expect("the context boundary must be an outer oneOf");
        assert_eq!(outer.len(), 2);
        for context in outer {
            let resolved = context
                .get("$ref")
                .and_then(Value::as_str)
                .and_then(|reference| reference.strip_prefix("#/$defs/"))
                .and_then(|name| receipt_schema["$defs"].get(name))
                .unwrap_or(context);
            assert_eq!(
                resolved["oneOf"].as_array().map(Vec::len),
                Some(2),
                "each context must contain exactly changed/noChange leaves"
            );
        }
    }

    #[test]
    fn task_workspace_phase_transitions_cover_only_the_nine_legal_phases() {
        let legal = TaskPhase::ALL
            .iter()
            .copied()
            .filter_map(|phase| CompatibleTaskMutationPhase::try_from(phase).ok())
            .collect::<Vec<_>>();
        assert_eq!(legal.len(), 9);

        for phase in legal {
            let changed_authority = task_changed_authority(A, B, phase.into());
            let changed = BranchedChangeReceipt::new(&changed_authority).unwrap();
            let changed_json = serde_json::to_value(changed).unwrap();
            assert_eq!(
                changed_json["phaseTransition"]["resultingPhase"],
                json!("developing")
            );

            let no_change_authority = task_no_change_authority(phase.into());
            let no_change = BranchedChangeReceipt::new(&no_change_authority).unwrap();
            let no_change_json = serde_json::to_value(no_change).unwrap();
            assert_eq!(
                no_change_json["phaseTransition"]["phaseBefore"],
                no_change_json["phaseTransition"]["resultingPhase"]
            );
        }
        assert!(
            BranchedChangeReceiptAuthority::task_workspace_no_change_test_only(
                id(ID_1),
                vec![BranchedAffectedTarget::metadata_property(metadata_target(
                    OBJECT_A,
                    "Attributes.Name"
                ),)],
                digest(A),
                TaskPhase::Locked,
            )
            .is_err()
        );
    }

    #[test]
    fn task_phase_transition_schemas_reject_cross_product_pairs() {
        let changed =
            BranchedChangeReceipt::new(&task_changed_authority(A, B, TaskPhase::LocalVerified))
                .unwrap();
        let mut changed_json = serde_json::to_value(changed).unwrap();
        changed_json["phaseTransition"]["resultingPhase"] = json!("localVerified");
        assert!(!schema_accepts::<BranchedChangeReceipt>(&changed_json));

        let no_change =
            BranchedChangeReceipt::new(&task_no_change_authority(TaskPhase::LocalVerified))
                .unwrap();
        let mut no_change_json = serde_json::to_value(no_change).unwrap();
        no_change_json["phaseTransition"]["resultingPhase"] = json!("developing");
        assert!(!schema_accepts::<BranchedChangeReceipt>(&no_change_json));
    }

    #[test]
    fn changed_hashes_events_and_targets_are_fail_closed() {
        let equal = task_changed_authority(A, A, TaskPhase::Developing);
        assert!(BranchedChangeReceipt::new(&equal).is_err());

        assert!(
            BranchedChangeReceiptAuthority::task_workspace_changed_test_only(
                id(ID_1),
                Vec::new(),
                digest(A),
                digest(B),
                vec![id(ID_2)],
                Vec::new(),
                TaskPhase::Developing,
            )
            .is_err()
        );
        assert!(
            BranchedChangeReceiptAuthority::task_workspace_changed_test_only(
                id(ID_1),
                vec![
                    BranchedAffectedTarget::metadata_property(metadata_target(
                        OBJECT_B,
                        "Attributes.Name",
                    )),
                    BranchedAffectedTarget::metadata_property(metadata_target(
                        OBJECT_A,
                        "Attributes.Name",
                    )),
                ],
                digest(A),
                digest(B),
                vec![id(ID_2)],
                Vec::new(),
                TaskPhase::Developing,
            )
            .is_err()
        );
        assert!(
            BranchedChangeReceiptAuthority::task_workspace_changed_test_only(
                id(ID_1),
                vec![BranchedAffectedTarget::metadata_property(metadata_target(
                    OBJECT_A,
                    "Attributes.Name"
                ),)],
                digest(A),
                digest(B),
                Vec::new(),
                Vec::new(),
                TaskPhase::Developing,
            )
            .is_err()
        );
    }

    #[test]
    fn strict_wire_dto_rejects_noncanonical_targets_and_event_ids() {
        let authority = BranchedChangeReceiptAuthority::task_workspace_changed_test_only(
            id(ID_1),
            vec![
                BranchedAffectedTarget::metadata_property(metadata_target(
                    OBJECT_A,
                    "Attributes.Name",
                )),
                BranchedAffectedTarget::metadata_property(metadata_target(
                    OBJECT_B,
                    "Attributes.Name",
                )),
            ],
            digest(A),
            digest(B),
            vec![id(ID_2), id(ID_3)],
            vec![id(ID_4), id(ID_5)],
            TaskPhase::Developing,
        )
        .unwrap();
        let encoded =
            serde_json::to_value(BranchedChangeReceipt::new(&authority).unwrap()).unwrap();

        let mut reversed_targets = encoded.clone();
        reversed_targets["affectedTargets"]
            .as_array_mut()
            .unwrap()
            .reverse();
        assert!(
            serde_json::from_value::<UnvalidatedBranchedChangeReceipt>(reversed_targets).is_err()
        );

        let mut reversed_events = encoded.clone();
        reversed_events["eventIds"]
            .as_array_mut()
            .unwrap()
            .reverse();
        assert!(
            serde_json::from_value::<UnvalidatedBranchedChangeReceipt>(reversed_events).is_err()
        );

        let mut duplicate_evidence = encoded;
        duplicate_evidence["invalidatedEvidenceIds"] = json!([ID_4, ID_4]);
        assert!(
            serde_json::from_value::<UnvalidatedBranchedChangeReceipt>(duplicate_evidence).is_err()
        );
    }

    #[test]
    fn support_layer_target_distinguishes_absent_and_present_object_identity() {
        let layer_only = json!({
            "targetKind": "supportLayerProperty",
            "layerId": "vendor",
            "propertyPath": "Capabilities.Version"
        });
        let object_state = json!({
            "targetKind": "supportLayerProperty",
            "layerId": "vendor",
            "objectId": OBJECT_A,
            "propertyPath": "Attributes.Name"
        });
        assert!(serde_json::from_value::<BranchedAffectedTarget>(layer_only.clone()).is_ok());
        assert!(serde_json::from_value::<BranchedAffectedTarget>(object_state.clone()).is_ok());
        assert!(schema_accepts::<BranchedAffectedTarget>(&layer_only));
        assert!(schema_accepts::<BranchedAffectedTarget>(&object_state));

        let null_object = json!({
            "targetKind": "supportLayerProperty",
            "layerId": "vendor",
            "objectId": null,
            "propertyPath": "Attributes.Name"
        });
        assert!(serde_json::from_value::<BranchedAffectedTarget>(null_object.clone()).is_err());
        assert!(!schema_accepts::<BranchedAffectedTarget>(&null_object));
    }

    #[test]
    fn no_change_leaves_physically_forbid_effect_and_supersession_fields() {
        let task = BranchedChangeReceipt::new(&task_no_change_authority(TaskPhase::LocalVerified))
            .unwrap();
        let task_json = serde_json::to_value(task).unwrap();
        assert_eq!(task_json["eventIds"], json!([]));
        assert_eq!(task_json["invalidatedEvidenceIds"], json!([]));
        assert!(task_json.get("beforeSha256").is_none());
        assert!(task_json.get("afterSha256").is_none());
        let mut task_splice = task_json;
        task_splice["beforeSha256"] = json!(A);
        assert!(!schema_accepts::<BranchedChangeReceipt>(&task_splice));

        let merge = BranchedChangeReceipt::new(&merge_no_change_authority()).unwrap();
        let merge_json = serde_json::to_value(merge).unwrap();
        assert_eq!(merge_json["eventIds"], json!([]));
        assert_eq!(merge_json["invalidatedEvidenceIds"], json!([]));
        assert_eq!(merge_json["supersededChangeReceiptIds"], json!([]));
        assert_eq!(merge_json["supersededDecisionIds"], json!([]));
        assert!(merge_json.get("pendingReplacementDecisionId").is_none());
    }

    #[test]
    fn merge_changed_receipt_binds_sequence_target_generation_and_decision_lineage() {
        let authority = merge_changed_authority();
        let receipt = BranchedChangeReceipt::new(&authority).unwrap();
        let encoded = serde_json::to_value(receipt).unwrap();
        assert_eq!(encoded["affectedTargets"].as_array().unwrap().len(), 1);
        assert_eq!(
            encoded["affectedTargets"][0]["targetKind"],
            json!("metadataProperty")
        );
        assert_eq!(encoded["supersededChangeReceiptIds"], json!([ID_2]));
        assert_eq!(encoded["supersededDecisionIds"], json!([ID_3]));
        assert_eq!(encoded["pendingReplacementDecisionId"], json!(ID_3));
        assert_ne!(
            encoded["decisionSetDigestBefore"],
            encoded["revisedDecisionSetDigest"]
        );
        assert_eq!(encoded["receiptSequence"], json!(2));

        let target = metadata_target(OBJECT_A, "Module.Text");
        let generation = id(ID_4);
        let bad_prior = MergeResolutionSelectableReceiptAuthority::test_only(
            id(ID_2),
            ChangeReceiptSequence::new(2).unwrap(),
            generation.clone(),
            target.clone(),
        );
        let malformed = BranchedChangeReceiptAuthority::merge_resolution_changed_test_only(
            id(ID_1),
            target,
            digest(A),
            digest(B),
            vec![id(ID_5)],
            vec![bad_prior],
            MergeResolutionDecisionLineageAuthority::undecided_test_only(digest(A)),
            digest(A),
            generation,
            ChangeReceiptSequence::new(2).unwrap(),
        )
        .unwrap();
        assert!(BranchedChangeReceipt::new(&malformed).is_err());
    }

    #[test]
    fn merge_changed_schema_and_wire_dto_reject_decision_lineage_splices() {
        let receipt = BranchedChangeReceipt::new(&merge_changed_authority()).unwrap();
        let encoded = serde_json::to_value(receipt).unwrap();

        let mut multiple_current_heads = encoded.clone();
        multiple_current_heads["supersededDecisionIds"] = json!([ID_3, ID_5]);
        assert!(!schema_accepts::<BranchedChangeReceipt>(
            &multiple_current_heads
        ));
        assert!(
            serde_json::from_value::<UnvalidatedBranchedChangeReceipt>(multiple_current_heads)
                .is_err()
        );

        // A singleton is structurally the current-head branch. Equality with
        // the pending ID is a cross-field semantic invariant enforced by the
        // strict promotion DTO rather than invented discriminator fields.
        let mut wrong_current_head = encoded.clone();
        wrong_current_head["supersededDecisionIds"] = json!([ID_5]);
        assert!(
            serde_json::from_value::<UnvalidatedBranchedChangeReceipt>(wrong_current_head).is_err()
        );

        let mut missing_pending_head = encoded;
        missing_pending_head
            .as_object_mut()
            .unwrap()
            .remove("pendingReplacementDecisionId");
        assert!(
            serde_json::from_value::<UnvalidatedBranchedChangeReceipt>(missing_pending_head)
                .is_err()
        );
    }

    #[test]
    fn merge_changed_receipt_distinguishes_current_pending_and_undecided_lineage() {
        let target = metadata_target(OBJECT_A, "Module.Text");
        let replacement_pending =
            MergeResolutionDecisionLineageAuthority::replacement_pending_test_only(
                id(ID_3),
                digest(A),
                digest(B),
            );
        let authority = BranchedChangeReceiptAuthority::merge_resolution_changed_test_only(
            id(ID_1),
            target.clone(),
            digest(A),
            digest(B),
            vec![id(ID_5)],
            Vec::new(),
            replacement_pending,
            digest(A),
            id(ID_4),
            ChangeReceiptSequence::new(2).unwrap(),
        )
        .unwrap();
        let pending =
            serde_json::to_value(BranchedChangeReceipt::new(&authority).unwrap()).unwrap();
        assert_eq!(pending["supersededDecisionIds"], json!([]));
        assert_eq!(pending["pendingReplacementDecisionId"], json!(ID_3));
        assert_ne!(
            pending["decisionSetDigestBefore"],
            pending["revisedDecisionSetDigest"]
        );

        let authority = BranchedChangeReceiptAuthority::merge_resolution_changed_test_only(
            id(ID_1),
            target,
            digest(A),
            digest(B),
            vec![id(ID_5)],
            Vec::new(),
            MergeResolutionDecisionLineageAuthority::undecided_test_only(digest(A)),
            digest(A),
            id(ID_4),
            ChangeReceiptSequence::new(2).unwrap(),
        )
        .unwrap();
        let undecided =
            serde_json::to_value(BranchedChangeReceipt::new(&authority).unwrap()).unwrap();
        assert_eq!(undecided["supersededDecisionIds"], json!([]));
        assert!(undecided.get("pendingReplacementDecisionId").is_none());
        assert_eq!(
            undecided["decisionSetDigestBefore"],
            undecided["revisedDecisionSetDigest"]
        );
    }

    #[test]
    fn strict_wire_promotion_rejects_digest_and_rehashed_authority_substitution() {
        let authority = task_changed_authority(A, B, TaskPhase::LocalVerified);
        let receipt = BranchedChangeReceipt::new(&authority).unwrap();
        let encoded = serde_json::to_value(&receipt).unwrap();
        let wire =
            serde_json::from_value::<UnvalidatedBranchedChangeReceipt>(encoded.clone()).unwrap();
        assert!(BranchedChangeReceipt::from_wire(wire, &authority).is_ok());

        let mut digest_substitution = encoded.clone();
        digest_substitution["changeReceiptDigest"] = json!(A);
        let wire = serde_json::from_value(digest_substitution).unwrap();
        assert!(BranchedChangeReceipt::from_wire(wire, &authority).is_err());

        let substituted_authority = task_changed_authority(B, A, TaskPhase::LocalVerified);
        let substituted = BranchedChangeReceipt::new(&substituted_authority).unwrap();
        let wire = serde_json::from_value(serde_json::to_value(substituted).unwrap()).unwrap();
        assert!(BranchedChangeReceipt::from_wire(wire, &authority).is_err());
    }

    #[test]
    fn receipt_sequence_is_the_exact_positive_i_json_safe_range() {
        assert!(ChangeReceiptSequence::new(0).is_err());
        assert!(ChangeReceiptSequence::new(9_007_199_254_740_991).is_ok());
        assert!(ChangeReceiptSequence::new(9_007_199_254_740_992).is_err());
    }
}
