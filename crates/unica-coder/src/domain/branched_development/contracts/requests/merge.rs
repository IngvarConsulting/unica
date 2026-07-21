use super::{execution_policy_for_json, request_one_of_schema, DigestApproval};
use crate::domain::branched_development::contracts::scalars::{
    OriginalProjectCwd, PropertyPath, Rationale,
};
use crate::domain::branched_development::{
    ExecutionPolicy, MetadataObjectId, OperationId, Sha256Digest, TaskId, UnicaId,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

macro_rules! string_literal {
    ($name:ident, $variant:ident, $wire:literal) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
        enum $name {
            #[serde(rename = $wire)]
            $variant,
        }
    };
}

// Comparison sides deliberately have heterogeneous JSON shapes. A raw path,
// artifact identifier string, or object discriminator is not part of the wire
// contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
enum ComparisonAnchor {
    OriginalCurrent,
    Repository,
    TaskCurrent,
    TaskVendor,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ArtifactComparisonSide {
    artifact_id: UnicaId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
enum ComparisonSide {
    Anchor(ComparisonAnchor),
    Artifact(ArtifactComparisonSide),
}

request_one_of_schema!(
    ComparisonSide,
    "ComparisonSide",
    [ComparisonAnchor, ArtifactComparisonSide]
);

string_literal!(ProjectDeltaScope, Value, "projectDelta");
string_literal!(MainIntegrationScope, Value, "mainIntegration");

macro_rules! compare_leaf {
    ($name:ident, $scope:ty) => {
        #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        pub(crate) struct $name {
            cwd: OriginalProjectCwd,
            task_id: TaskId,
            operation_id: OperationId,
            left: ComparisonSide,
            right: ComparisonSide,
            scope: $scope,
        }
    };
}

compare_leaf!(CompareProjectDelta, ProjectDeltaScope);
compare_leaf!(CompareMainIntegration, MainIntegrationScope);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub(crate) enum MergeCompareRequest {
    ProjectDelta(CompareProjectDelta),
    MainIntegration(CompareMainIntegration),
}

request_one_of_schema!(
    MergeCompareRequest,
    "MergeCompareRequest",
    [CompareProjectDelta, CompareMainIntegration]
);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MergeCompareRequestVariant {
    ProjectDelta,
    MainIntegration,
}

impl MergeCompareRequest {
    pub(crate) const fn request_variant(&self) -> MergeCompareRequestVariant {
        match self {
            Self::ProjectDelta(_) => MergeCompareRequestVariant::ProjectDelta,
            Self::MainIntegration(_) => MergeCompareRequestVariant::MainIntegration,
        }
    }

    pub(crate) const fn execution_policy(&self) -> ExecutionPolicy {
        ExecutionPolicy::Contained
    }

    pub(crate) fn execution_policy_for_json(value: &Value) -> Option<ExecutionPolicy> {
        execution_policy_for_json::<Self>(value, Self::execution_policy)
    }
}

string_literal!(SupportedUpdateMode, Value, "supportedUpdate");
string_literal!(ResolvedReplayMode, Value, "resolvedReplay");
string_literal!(MainIntegrationMode, Value, "mainIntegration");

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct PrepareSupportedUpdate {
    cwd: OriginalProjectCwd,
    task_id: TaskId,
    operation_id: OperationId,
    mode: SupportedUpdateMode,
    checkpoint_id: UnicaId,
    incoming_distribution_id: UnicaId,
    comparison_id: UnicaId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct PrepareSupportedUpdateReplacement {
    cwd: OriginalProjectCwd,
    task_id: TaskId,
    operation_id: OperationId,
    mode: SupportedUpdateMode,
    checkpoint_id: UnicaId,
    incoming_distribution_id: UnicaId,
    comparison_id: UnicaId,
    replaces_session_id: UnicaId,
    expected_replaced_base_session_digest: Sha256Digest,
    expected_replaced_decision_set_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct PrepareResolvedReplay {
    cwd: OriginalProjectCwd,
    task_id: TaskId,
    operation_id: OperationId,
    mode: ResolvedReplayMode,
    session_id: UnicaId,
    expected_base_session_digest: Sha256Digest,
    expected_decision_set_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct PrepareMainIntegration {
    cwd: OriginalProjectCwd,
    task_id: TaskId,
    operation_id: OperationId,
    mode: MainIntegrationMode,
    checkpoint_id: UnicaId,
    verification_id: UnicaId,
    expected_verification_digest: Sha256Digest,
    expected_repository_status_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub(crate) enum MergePrepareRequest {
    SupportedUpdate(PrepareSupportedUpdate),
    SupportedUpdateReplacement(PrepareSupportedUpdateReplacement),
    ResolvedReplay(PrepareResolvedReplay),
    MainIntegration(PrepareMainIntegration),
}

request_one_of_schema!(
    MergePrepareRequest,
    "MergePrepareRequest",
    [
        PrepareSupportedUpdate,
        PrepareSupportedUpdateReplacement,
        PrepareResolvedReplay,
        PrepareMainIntegration,
    ]
);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MergePrepareRequestVariant {
    SupportedUpdate,
    SupportedUpdateReplacement,
    ResolvedReplay,
    MainIntegration,
}

impl MergePrepareRequest {
    pub(crate) const fn request_variant(&self) -> MergePrepareRequestVariant {
        match self {
            Self::SupportedUpdate(_) => MergePrepareRequestVariant::SupportedUpdate,
            Self::SupportedUpdateReplacement(_) => {
                MergePrepareRequestVariant::SupportedUpdateReplacement
            }
            Self::ResolvedReplay(_) => MergePrepareRequestVariant::ResolvedReplay,
            Self::MainIntegration(_) => MergePrepareRequestVariant::MainIntegration,
        }
    }

    pub(crate) const fn execution_policy(&self) -> ExecutionPolicy {
        match self {
            Self::MainIntegration(_) => ExecutionPolicy::JournaledEffect,
            Self::SupportedUpdate(_)
            | Self::SupportedUpdateReplacement(_)
            | Self::ResolvedReplay(_) => ExecutionPolicy::Contained,
        }
    }

    pub(crate) fn execution_policy_for_json(value: &Value) -> Option<ExecutionPolicy> {
        execution_policy_for_json::<Self>(value, Self::execution_policy)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct MergeConflictsRequest {
    cwd: OriginalProjectCwd,
    task_id: TaskId,
    session_id: UnicaId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MergeConflictsRequestVariant {
    Conflicts,
}

impl MergeConflictsRequest {
    pub(crate) const fn request_variant(&self) -> MergeConflictsRequestVariant {
        MergeConflictsRequestVariant::Conflicts
    }

    pub(crate) const fn execution_policy(&self) -> ExecutionPolicy {
        ExecutionPolicy::ReadOnly
    }

    pub(crate) fn execution_policy_for_json(value: &Value) -> Option<ExecutionPolicy> {
        execution_policy_for_json::<Self>(value, Self::execution_policy)
    }
}

string_literal!(ConflictDecisionKind, Value, "conflict");
string_literal!(AdaptedDeltaDecisionKind, Value, "adaptedDelta");
string_literal!(TakeOursResolution, Value, "takeOurs");
string_literal!(TakeTheirsResolution, Value, "takeTheirs");
string_literal!(CombineResolution, Value, "combine");
string_literal!(ManualResolution, Value, "manual");

macro_rules! conflict_resolution_leaf {
    ($name:ident, $resolution:ty $(, $field:ident : $field_type:ty )* $(,)?) => {
        #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        pub(crate) struct $name {
            cwd: OriginalProjectCwd,
            task_id: TaskId,
            operation_id: OperationId,
            decision_kind: ConflictDecisionKind,
            session_id: UnicaId,
            conflict_id: UnicaId,
            resolution: $resolution,
            rationale: Rationale,
            expected_base_session_digest: Sha256Digest,
            expected_decision_set_digest: Sha256Digest,
            $($field: $field_type,)*
        }
    };
}

conflict_resolution_leaf!(ResolveTakeOurs, TakeOursResolution);
conflict_resolution_leaf!(ResolveTakeTheirs, TakeTheirsResolution);
conflict_resolution_leaf!(
    ResolveCombine,
    CombineResolution,
    change_receipt_id: UnicaId,
    object_id: MetadataObjectId,
    property_path: PropertyPath,
    expected_result_sha256: Sha256Digest,
);
conflict_resolution_leaf!(
    ResolveManual,
    ManualResolution,
    change_receipt_id: UnicaId,
    object_id: MetadataObjectId,
    property_path: PropertyPath,
    expected_result_sha256: Sha256Digest,
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ResolveAdaptedDelta {
    cwd: OriginalProjectCwd,
    task_id: TaskId,
    operation_id: OperationId,
    decision_kind: AdaptedDeltaDecisionKind,
    verification_id: UnicaId,
    expected_verification_digest: Sha256Digest,
    canonical_delta_digest: Sha256Digest,
    difference_manifest_id: UnicaId,
    difference_digest: Sha256Digest,
    rationale: Rationale,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub(crate) enum MergeResolveRequest {
    TakeOurs(ResolveTakeOurs),
    TakeTheirs(ResolveTakeTheirs),
    Combine(ResolveCombine),
    Manual(ResolveManual),
    AdaptedDelta(ResolveAdaptedDelta),
}

request_one_of_schema!(
    MergeResolveRequest,
    "MergeResolveRequest",
    [
        ResolveTakeOurs,
        ResolveTakeTheirs,
        ResolveCombine,
        ResolveManual,
        ResolveAdaptedDelta,
    ]
);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MergeResolveRequestVariant {
    TakeOurs,
    TakeTheirs,
    Combine,
    Manual,
    AdaptedDelta,
}

impl MergeResolveRequest {
    pub(crate) const fn request_variant(&self) -> MergeResolveRequestVariant {
        match self {
            Self::TakeOurs(_) => MergeResolveRequestVariant::TakeOurs,
            Self::TakeTheirs(_) => MergeResolveRequestVariant::TakeTheirs,
            Self::Combine(_) => MergeResolveRequestVariant::Combine,
            Self::Manual(_) => MergeResolveRequestVariant::Manual,
            Self::AdaptedDelta(_) => MergeResolveRequestVariant::AdaptedDelta,
        }
    }

    pub(crate) const fn execution_policy(&self) -> ExecutionPolicy {
        ExecutionPolicy::LocalJournaled
    }

    pub(crate) fn execution_policy_for_json(value: &Value) -> Option<ExecutionPolicy> {
        execution_policy_for_json::<Self>(value, Self::execution_policy)
    }
}

string_literal!(TaskTarget, Value, "task");
string_literal!(OriginalTarget, Value, "original");

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ApplyTask {
    cwd: OriginalProjectCwd,
    task_id: TaskId,
    operation_id: OperationId,
    session_id: UnicaId,
    target: TaskTarget,
    approval: DigestApproval,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ApplyOriginal {
    cwd: OriginalProjectCwd,
    task_id: TaskId,
    operation_id: OperationId,
    session_id: UnicaId,
    target: OriginalTarget,
    approval: DigestApproval,
    plan_id: UnicaId,
    expected_plan_digest: Sha256Digest,
    integration_set_id: UnicaId,
    expected_integration_set_digest: Sha256Digest,
    lock_set_id: UnicaId,
    expected_lock_set_digest: Sha256Digest,
    support_gate_id: UnicaId,
    expected_support_gate_digest: Sha256Digest,
    expected_support_gate_history_evidence_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub(crate) enum MergeApplyRequest {
    Task(ApplyTask),
    Original(Box<ApplyOriginal>),
}

request_one_of_schema!(
    MergeApplyRequest,
    "MergeApplyRequest",
    [ApplyTask, ApplyOriginal]
);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MergeApplyRequestVariant {
    Task,
    Original,
}

impl MergeApplyRequest {
    pub(crate) const fn request_variant(&self) -> MergeApplyRequestVariant {
        match self {
            Self::Task(_) => MergeApplyRequestVariant::Task,
            Self::Original(_) => MergeApplyRequestVariant::Original,
        }
    }

    pub(crate) const fn execution_policy(&self) -> ExecutionPolicy {
        ExecutionPolicy::PreparedJournaledEffect
    }

    pub(crate) fn execution_policy_for_json(value: &Value) -> Option<ExecutionPolicy> {
        execution_policy_for_json::<Self>(value, Self::execution_policy)
    }
}

string_literal!(LocalCheckpointScope, Value, "localCheckpoint");
string_literal!(SynchronizedTaskScope, Value, "synchronizedTask");
string_literal!(MainSandboxScope, Value, "mainSandbox");

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct VerifyLocalCheckpoint {
    cwd: OriginalProjectCwd,
    task_id: TaskId,
    operation_id: OperationId,
    scope: LocalCheckpointScope,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct VerifySynchronizedTask {
    cwd: OriginalProjectCwd,
    task_id: TaskId,
    operation_id: OperationId,
    scope: SynchronizedTaskScope,
    session_id: UnicaId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct VerifySynchronizedTaskAdapted {
    cwd: OriginalProjectCwd,
    task_id: TaskId,
    operation_id: OperationId,
    scope: SynchronizedTaskScope,
    session_id: UnicaId,
    adaptation_decision_id: UnicaId,
    expected_adaptation_decision_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct VerifyMainSandbox {
    cwd: OriginalProjectCwd,
    task_id: TaskId,
    operation_id: OperationId,
    scope: MainSandboxScope,
    session_id: UnicaId,
    expected_resolved_session_digest: Sha256Digest,
    support_gate_id: UnicaId,
    expected_support_gate_digest: Sha256Digest,
    expected_support_gate_history_evidence_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct VerifyMainIntegration {
    cwd: OriginalProjectCwd,
    task_id: TaskId,
    operation_id: OperationId,
    scope: MainIntegrationScope,
    session_id: UnicaId,
    expected_resolved_session_digest: Sha256Digest,
    merge_receipt_id: UnicaId,
    integration_set_id: UnicaId,
    expected_integration_set_digest: Sha256Digest,
    support_gate_id: UnicaId,
    expected_support_gate_digest: Sha256Digest,
    expected_support_gate_history_evidence_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub(crate) enum MergeVerifyRequest {
    LocalCheckpoint(VerifyLocalCheckpoint),
    SynchronizedTask(VerifySynchronizedTask),
    SynchronizedTaskAdapted(VerifySynchronizedTaskAdapted),
    MainSandbox(VerifyMainSandbox),
    MainIntegration(VerifyMainIntegration),
}

request_one_of_schema!(
    MergeVerifyRequest,
    "MergeVerifyRequest",
    [
        VerifyLocalCheckpoint,
        VerifySynchronizedTask,
        VerifySynchronizedTaskAdapted,
        VerifyMainSandbox,
        VerifyMainIntegration,
    ]
);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MergeVerifyRequestVariant {
    LocalCheckpoint,
    SynchronizedTask,
    SynchronizedTaskAdapted,
    MainSandbox,
    MainIntegration,
}

impl MergeVerifyRequest {
    pub(crate) const fn request_variant(&self) -> MergeVerifyRequestVariant {
        match self {
            Self::LocalCheckpoint(_) => MergeVerifyRequestVariant::LocalCheckpoint,
            Self::SynchronizedTask(_) => MergeVerifyRequestVariant::SynchronizedTask,
            Self::SynchronizedTaskAdapted(_) => MergeVerifyRequestVariant::SynchronizedTaskAdapted,
            Self::MainSandbox(_) => MergeVerifyRequestVariant::MainSandbox,
            Self::MainIntegration(_) => MergeVerifyRequestVariant::MainIntegration,
        }
    }

    pub(crate) const fn execution_policy(&self) -> ExecutionPolicy {
        ExecutionPolicy::Contained
    }

    pub(crate) fn execution_policy_for_json(value: &Value) -> Option<ExecutionPolicy> {
        execution_policy_for_json::<Self>(value, Self::execution_policy)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ComparisonSide, MergeApplyRequest, MergeApplyRequestVariant, MergeCompareRequest,
        MergeCompareRequestVariant, MergeConflictsRequest, MergeConflictsRequestVariant,
        MergePrepareRequest, MergePrepareRequestVariant, MergeResolveRequest,
        MergeResolveRequestVariant, MergeVerifyRequest, MergeVerifyRequestVariant,
        TakeOursResolution,
    };
    use crate::domain::branched_development::contracts::scalars::PropertyPath;
    use crate::domain::branched_development::contracts::schema::{
        audit_json_schema, is_i_json_lf_text, is_i_json_single_line_text,
        is_normalized_utc_instant, I_JSON_LF_TEXT_FORMAT, I_JSON_SINGLE_LINE_TEXT_FORMAT,
        NORMALIZED_UTC_INSTANT_FORMAT,
    };
    use crate::domain::branched_development::{
        ExecutionPolicy, MetadataObjectId, Sha256Digest, UnicaId,
    };
    use schemars::{schema_for, JsonSchema};
    use serde::de::DeserializeOwned;
    use serde_json::{json, Value};

    const CWD: &str = "/original/project";
    const TASK_ID: &str = "TASK-173";
    const OPERATION_ID: &str = "123e4567-e89b-12d3-a456-426614174000";
    const ID: &str = "223e4567-e89b-12d3-a456-426614174000";
    const OTHER_ID: &str = "323e4567-e89b-12d3-a456-426614174000";
    const DIGEST: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const OTHER_DIGEST: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    fn mutation() -> Value {
        json!({ "cwd": CWD, "taskId": TASK_ID, "operationId": OPERATION_ID })
    }

    fn task() -> Value {
        json!({ "cwd": CWD, "taskId": TASK_ID })
    }

    fn with(mut value: Value, fields: &[(&str, Value)]) -> Value {
        let object = value.as_object_mut().expect("fixture must be an object");
        for (name, field) in fields {
            object.insert((*name).to_owned(), field.clone());
        }
        value
    }

    fn without(mut value: Value, field: &str) -> Value {
        value
            .as_object_mut()
            .expect("fixture must be an object")
            .remove(field);
        value
    }

    fn accepts<T: DeserializeOwned>(value: Value) -> T {
        serde_json::from_value(value.clone())
            .unwrap_or_else(|error| panic!("request contract rejected {value}: {error}"))
    }

    fn rejects<T: DeserializeOwned>(value: Value) {
        assert!(
            serde_json::from_value::<T>(value.clone()).is_err(),
            "request contract accepted {value}"
        );
    }

    fn typed_field<T: DeserializeOwned + JsonSchema>(
        name: &'static str,
        value: Value,
    ) -> (&'static str, Value) {
        accepts::<T>(value.clone());
        assert!(
            schema_validator::<T>().is_valid(&value),
            "field {name} fixture is invalid for its declared scalar schema: {value}"
        );
        (name, value)
    }

    fn assert_rejects_cross_variant_fields<T: DeserializeOwned + JsonSchema>(
        base: Value,
        fields: Vec<(&'static str, Value)>,
    ) {
        let validator = schema_validator::<T>();
        for (field, value) in fields {
            let request = with(base.clone(), &[(field, value)]);
            rejects::<T>(request.clone());
            assert!(
                !validator.is_valid(&request),
                "schema accepted cross-variant field {field} in {request}"
            );
        }
    }

    fn schema_validator<T: JsonSchema>() -> jsonschema::Validator {
        let schema = serde_json::to_value(schema_for!(T)).unwrap();
        jsonschema::options()
            .with_draft(jsonschema::Draft::Draft202012)
            .with_format(I_JSON_SINGLE_LINE_TEXT_FORMAT, is_i_json_single_line_text)
            .with_format(I_JSON_LF_TEXT_FORMAT, is_i_json_lf_text)
            .with_format(NORMALIZED_UTC_INSTANT_FORMAT, is_normalized_utc_instant)
            .should_validate_formats(true)
            .should_ignore_unknown_formats(false)
            .build(&schema)
            .expect("request schema must compile")
    }

    fn assert_runtime_and_schema_accept<T: DeserializeOwned + JsonSchema>(value: Value) {
        accepts::<T>(value.clone());
        assert!(
            schema_validator::<T>().is_valid(&value),
            "schema rejected runtime-valid request {value}"
        );
    }

    fn assert_runtime_and_schema_reject<T: DeserializeOwned + JsonSchema>(value: Value) {
        rejects::<T>(value.clone());
        assert!(
            !schema_validator::<T>().is_valid(&value),
            "schema accepted runtime-invalid request {value}"
        );
    }

    fn assert_schema_is_closed<T: JsonSchema>() {
        let schema = serde_json::to_value(schema_for!(T)).unwrap();
        audit_json_schema(&schema).expect("merge request schema must be recursively closed");
    }

    fn assert_exact_one_of<T: JsonSchema>(expected_branches: usize) {
        let schema = serde_json::to_value(schema_for!(T)).unwrap();
        assert_eq!(
            schema.get("oneOf").and_then(Value::as_array).map(Vec::len),
            Some(expected_branches),
            "schema must expose the exact physical union at its root"
        );
        assert!(
            !contains_keyword(&schema, "anyOf"),
            "schema retained an anyOf escape"
        );
    }

    fn contains_keyword(value: &Value, keyword: &str) -> bool {
        match value {
            Value::Object(object) => {
                object.contains_key(keyword)
                    || object
                        .values()
                        .any(|nested| contains_keyword(nested, keyword))
            }
            Value::Array(array) => array.iter().any(|nested| contains_keyword(nested, keyword)),
            _ => false,
        }
    }

    fn assert_common_mutation_is_physical<T: DeserializeOwned + JsonSchema>(values: &[Value]) {
        for value in values {
            for field in ["cwd", "taskId", "operationId"] {
                assert_runtime_and_schema_reject::<T>(without(value.clone(), field));
            }
        }
    }

    fn assert_rejects_generic_effect_controls<T: DeserializeOwned + JsonSchema>(values: &[Value]) {
        for value in values {
            for (field, injected) in [
                ("dryRun", json!(true)),
                ("approvedPreviewDigest", json!(DIGEST)),
                ("approval", json!({ "digest": DIGEST, "decision": "apply" })),
            ] {
                assert_runtime_and_schema_reject::<T>(with(value.clone(), &[(field, injected)]));
            }
        }
    }

    fn compare(scope: &str, left: Value, right: Value) -> Value {
        with(
            mutation(),
            &[("left", left), ("right", right), ("scope", json!(scope))],
        )
    }

    #[test]
    fn compare_uses_only_heterogeneous_closed_sides_and_exact_scopes() {
        for scope in ["projectDelta", "mainIntegration"] {
            for anchor in ["originalCurrent", "repository", "taskCurrent", "taskVendor"] {
                let request = compare(scope, json!(anchor), json!({ "artifactId": ID }));
                let parsed = accepts::<MergeCompareRequest>(request.clone());
                assert_eq!(parsed.execution_policy(), ExecutionPolicy::Contained);
                assert_eq!(
                    parsed.request_variant(),
                    if scope == "projectDelta" {
                        MergeCompareRequestVariant::ProjectDelta
                    } else {
                        MergeCompareRequestVariant::MainIntegration
                    }
                );
                assert_runtime_and_schema_accept::<MergeCompareRequest>(request);
            }
        }

        for side in [
            json!("artifact"),
            json!(ID),
            json!({ "artifactId": ID, "kind": "configurationDistribution" }),
            json!({ "artifactId": ID, "path": "/tmp/result.cf" }),
            json!({ "kind": "artifact", "artifactId": ID }),
            json!({}),
        ] {
            assert_runtime_and_schema_reject::<MergeCompareRequest>(compare(
                "projectDelta",
                side,
                json!("taskVendor"),
            ));
        }
        for invalid in [
            compare("delta", json!("taskCurrent"), json!("taskVendor")),
            without(
                compare("projectDelta", json!("taskCurrent"), json!("taskVendor")),
                "operationId",
            ),
            with(
                compare("mainIntegration", json!("repository"), json!("taskCurrent")),
                &[("approval", json!({ "digest": DIGEST, "decision": "apply" }))],
            ),
        ] {
            assert_runtime_and_schema_reject::<MergeCompareRequest>(invalid);
        }
    }

    fn supported_update() -> Value {
        with(
            mutation(),
            &[
                ("mode", json!("supportedUpdate")),
                ("checkpointId", json!(ID)),
                ("incomingDistributionId", json!(OTHER_ID)),
                ("comparisonId", json!(ID)),
            ],
        )
    }

    fn supported_update_replacement() -> Value {
        with(
            supported_update(),
            &[
                ("replacesSessionId", json!(ID)),
                ("expectedReplacedBaseSessionDigest", json!(DIGEST)),
                ("expectedReplacedDecisionSetDigest", json!(OTHER_DIGEST)),
            ],
        )
    }

    fn resolved_replay() -> Value {
        with(
            mutation(),
            &[
                ("mode", json!("resolvedReplay")),
                ("sessionId", json!(ID)),
                ("expectedBaseSessionDigest", json!(DIGEST)),
                ("expectedDecisionSetDigest", json!(OTHER_DIGEST)),
            ],
        )
    }

    fn main_integration_prepare() -> Value {
        with(
            mutation(),
            &[
                ("mode", json!("mainIntegration")),
                ("checkpointId", json!(ID)),
                ("verificationId", json!(OTHER_ID)),
                ("expectedVerificationDigest", json!(DIGEST)),
                ("expectedRepositoryStatusDigest", json!(OTHER_DIGEST)),
            ],
        )
    }

    #[test]
    fn prepare_separates_first_replacement_replay_and_main_lineage() {
        for (request, variant, policy) in [
            (
                supported_update(),
                MergePrepareRequestVariant::SupportedUpdate,
                ExecutionPolicy::Contained,
            ),
            (
                supported_update_replacement(),
                MergePrepareRequestVariant::SupportedUpdateReplacement,
                ExecutionPolicy::Contained,
            ),
            (
                resolved_replay(),
                MergePrepareRequestVariant::ResolvedReplay,
                ExecutionPolicy::Contained,
            ),
            (
                main_integration_prepare(),
                MergePrepareRequestVariant::MainIntegration,
                ExecutionPolicy::JournaledEffect,
            ),
        ] {
            let parsed = accepts::<MergePrepareRequest>(request.clone());
            assert_eq!(parsed.request_variant(), variant);
            assert_eq!(parsed.execution_policy(), policy);
            assert_runtime_and_schema_accept::<MergePrepareRequest>(request);
        }

        for field in [
            "replacesSessionId",
            "expectedReplacedBaseSessionDigest",
            "expectedReplacedDecisionSetDigest",
        ] {
            assert_runtime_and_schema_reject::<MergePrepareRequest>(without(
                supported_update_replacement(),
                field,
            ));
        }
        for (base, injected) in [
            (resolved_replay(), ("replacesSessionId", json!(ID))),
            (
                main_integration_prepare(),
                ("incomingDistributionId", json!(ID)),
            ),
            (supported_update(), ("sessionId", json!(ID))),
            (supported_update(), ("dryRun", json!(true))),
        ] {
            assert_runtime_and_schema_reject::<MergePrepareRequest>(with(base, &[injected]));
        }
        for common in ["cwd", "taskId", "operationId"] {
            assert_runtime_and_schema_reject::<MergePrepareRequest>(without(
                supported_update(),
                common,
            ));
        }

        assert_rejects_cross_variant_fields::<MergePrepareRequest>(
            supported_update(),
            vec![
                typed_field::<UnicaId>("replacesSessionId", json!(ID)),
                typed_field::<Sha256Digest>("expectedReplacedBaseSessionDigest", json!(DIGEST)),
                typed_field::<Sha256Digest>(
                    "expectedReplacedDecisionSetDigest",
                    json!(OTHER_DIGEST),
                ),
                typed_field::<UnicaId>("sessionId", json!(ID)),
                typed_field::<Sha256Digest>("expectedBaseSessionDigest", json!(DIGEST)),
                typed_field::<Sha256Digest>("expectedDecisionSetDigest", json!(OTHER_DIGEST)),
                typed_field::<UnicaId>("verificationId", json!(OTHER_ID)),
                typed_field::<Sha256Digest>("expectedVerificationDigest", json!(DIGEST)),
                typed_field::<Sha256Digest>("expectedRepositoryStatusDigest", json!(OTHER_DIGEST)),
            ],
        );
        assert_rejects_cross_variant_fields::<MergePrepareRequest>(
            supported_update_replacement(),
            vec![
                typed_field::<UnicaId>("sessionId", json!(ID)),
                typed_field::<Sha256Digest>("expectedBaseSessionDigest", json!(DIGEST)),
                typed_field::<Sha256Digest>("expectedDecisionSetDigest", json!(OTHER_DIGEST)),
                typed_field::<UnicaId>("verificationId", json!(OTHER_ID)),
                typed_field::<Sha256Digest>("expectedVerificationDigest", json!(DIGEST)),
                typed_field::<Sha256Digest>("expectedRepositoryStatusDigest", json!(OTHER_DIGEST)),
            ],
        );
        assert_rejects_cross_variant_fields::<MergePrepareRequest>(
            resolved_replay(),
            vec![
                typed_field::<UnicaId>("checkpointId", json!(ID)),
                typed_field::<UnicaId>("incomingDistributionId", json!(OTHER_ID)),
                typed_field::<UnicaId>("comparisonId", json!(ID)),
                typed_field::<UnicaId>("replacesSessionId", json!(ID)),
                typed_field::<Sha256Digest>("expectedReplacedBaseSessionDigest", json!(DIGEST)),
                typed_field::<Sha256Digest>(
                    "expectedReplacedDecisionSetDigest",
                    json!(OTHER_DIGEST),
                ),
                typed_field::<UnicaId>("verificationId", json!(OTHER_ID)),
                typed_field::<Sha256Digest>("expectedVerificationDigest", json!(DIGEST)),
                typed_field::<Sha256Digest>("expectedRepositoryStatusDigest", json!(OTHER_DIGEST)),
            ],
        );
        assert_rejects_cross_variant_fields::<MergePrepareRequest>(
            main_integration_prepare(),
            vec![
                typed_field::<UnicaId>("incomingDistributionId", json!(OTHER_ID)),
                typed_field::<UnicaId>("comparisonId", json!(ID)),
                typed_field::<UnicaId>("replacesSessionId", json!(ID)),
                typed_field::<Sha256Digest>("expectedReplacedBaseSessionDigest", json!(DIGEST)),
                typed_field::<Sha256Digest>(
                    "expectedReplacedDecisionSetDigest",
                    json!(OTHER_DIGEST),
                ),
                typed_field::<UnicaId>("sessionId", json!(ID)),
                typed_field::<Sha256Digest>("expectedBaseSessionDigest", json!(DIGEST)),
                typed_field::<Sha256Digest>("expectedDecisionSetDigest", json!(OTHER_DIGEST)),
            ],
        );
    }

    fn conflicts() -> Value {
        with(task(), &[("sessionId", json!(ID))])
    }

    #[test]
    fn conflicts_is_read_only_and_has_no_operation_id() {
        let parsed = accepts::<MergeConflictsRequest>(conflicts());
        assert_eq!(
            parsed.request_variant(),
            MergeConflictsRequestVariant::Conflicts
        );
        assert_eq!(parsed.execution_policy(), ExecutionPolicy::ReadOnly);

        for field in ["cwd", "taskId", "sessionId"] {
            assert_runtime_and_schema_reject::<MergeConflictsRequest>(without(conflicts(), field));
        }
        assert_runtime_and_schema_reject::<MergeConflictsRequest>(with(
            conflicts(),
            &[("operationId", json!(OPERATION_ID))],
        ));
        assert_runtime_and_schema_reject::<MergeConflictsRequest>(with(
            conflicts(),
            &[("resolution", json!("takeOurs"))],
        ));
        assert_runtime_and_schema_accept::<MergeConflictsRequest>(conflicts());
    }

    fn conflict_resolution(resolution: &str) -> Value {
        with(
            mutation(),
            &[
                ("decisionKind", json!("conflict")),
                ("sessionId", json!(ID)),
                ("conflictId", json!(OTHER_ID)),
                ("resolution", json!(resolution)),
                ("rationale", json!("Reviewed exact conflict evidence")),
                ("expectedBaseSessionDigest", json!(DIGEST)),
                ("expectedDecisionSetDigest", json!(OTHER_DIGEST)),
            ],
        )
    }

    fn changed_conflict_resolution(resolution: &str) -> Value {
        with(
            conflict_resolution(resolution),
            &[
                ("changeReceiptId", json!(ID)),
                ("objectId", json!(OTHER_ID)),
                ("propertyPath", json!("Attributes.Price.Type")),
                ("expectedResultSha256", json!(DIGEST)),
            ],
        )
    }

    fn adapted_delta() -> Value {
        with(
            mutation(),
            &[
                ("decisionKind", json!("adaptedDelta")),
                ("verificationId", json!(ID)),
                ("expectedVerificationDigest", json!(DIGEST)),
                ("canonicalDeltaDigest", json!(OTHER_DIGEST)),
                ("differenceManifestId", json!(OTHER_ID)),
                ("differenceDigest", json!(DIGEST)),
                ("rationale", json!("The unexpected delta is intentional")),
            ],
        )
    }

    #[test]
    fn resolve_has_four_exact_conflict_leaves_and_one_adapted_delta_leaf() {
        for (request, variant) in [
            (
                conflict_resolution("takeOurs"),
                MergeResolveRequestVariant::TakeOurs,
            ),
            (
                conflict_resolution("takeTheirs"),
                MergeResolveRequestVariant::TakeTheirs,
            ),
            (
                changed_conflict_resolution("combine"),
                MergeResolveRequestVariant::Combine,
            ),
            (
                changed_conflict_resolution("manual"),
                MergeResolveRequestVariant::Manual,
            ),
            (adapted_delta(), MergeResolveRequestVariant::AdaptedDelta),
        ] {
            let parsed = accepts::<MergeResolveRequest>(request.clone());
            assert_eq!(parsed.request_variant(), variant);
            assert_eq!(parsed.execution_policy(), ExecutionPolicy::LocalJournaled);
            assert_runtime_and_schema_accept::<MergeResolveRequest>(request);
        }

        for resolution in ["takeOurs", "takeTheirs"] {
            assert_rejects_cross_variant_fields::<MergeResolveRequest>(
                conflict_resolution(resolution),
                vec![
                    typed_field::<UnicaId>("changeReceiptId", json!(ID)),
                    typed_field::<MetadataObjectId>("objectId", json!(OTHER_ID)),
                    typed_field::<PropertyPath>("propertyPath", json!("Attributes.Price.Type")),
                    typed_field::<Sha256Digest>("expectedResultSha256", json!(DIGEST)),
                ],
            );
        }
        for resolution in ["combine", "manual"] {
            for field in [
                "changeReceiptId",
                "objectId",
                "propertyPath",
                "expectedResultSha256",
            ] {
                assert_runtime_and_schema_reject::<MergeResolveRequest>(without(
                    changed_conflict_resolution(resolution),
                    field,
                ));
            }
        }
        for invalid in [
            conflict_resolution("combine"),
            conflict_resolution("manual"),
            conflict_resolution("staticByKind"),
            with(conflict_resolution("takeOurs"), &[("rationale", json!(""))]),
        ] {
            assert_runtime_and_schema_reject::<MergeResolveRequest>(invalid);
        }

        for conflict in [
            conflict_resolution("takeOurs"),
            conflict_resolution("takeTheirs"),
            changed_conflict_resolution("combine"),
            changed_conflict_resolution("manual"),
        ] {
            assert_rejects_cross_variant_fields::<MergeResolveRequest>(
                conflict,
                vec![
                    typed_field::<UnicaId>("verificationId", json!(ID)),
                    typed_field::<Sha256Digest>("expectedVerificationDigest", json!(DIGEST)),
                    typed_field::<Sha256Digest>("canonicalDeltaDigest", json!(OTHER_DIGEST)),
                    typed_field::<UnicaId>("differenceManifestId", json!(OTHER_ID)),
                    typed_field::<Sha256Digest>("differenceDigest", json!(DIGEST)),
                ],
            );
        }
        assert_rejects_cross_variant_fields::<MergeResolveRequest>(
            adapted_delta(),
            vec![
                typed_field::<UnicaId>("sessionId", json!(ID)),
                typed_field::<UnicaId>("conflictId", json!(OTHER_ID)),
                typed_field::<TakeOursResolution>("resolution", json!("takeOurs")),
                typed_field::<Sha256Digest>("expectedBaseSessionDigest", json!(DIGEST)),
                typed_field::<Sha256Digest>("expectedDecisionSetDigest", json!(OTHER_DIGEST)),
                typed_field::<UnicaId>("changeReceiptId", json!(ID)),
                typed_field::<MetadataObjectId>("objectId", json!(OTHER_ID)),
                typed_field::<PropertyPath>("propertyPath", json!("Attributes.Price.Type")),
                typed_field::<Sha256Digest>("expectedResultSha256", json!(DIGEST)),
            ],
        );
    }

    fn approval() -> Value {
        json!({ "digest": DIGEST, "decision": "apply" })
    }

    fn task_apply() -> Value {
        with(
            mutation(),
            &[
                ("sessionId", json!(ID)),
                ("target", json!("task")),
                ("approval", approval()),
            ],
        )
    }

    fn original_apply() -> Value {
        with(
            mutation(),
            &[
                ("sessionId", json!(ID)),
                ("target", json!("original")),
                ("approval", approval()),
                ("planId", json!(OTHER_ID)),
                ("expectedPlanDigest", json!(DIGEST)),
                ("integrationSetId", json!(ID)),
                ("expectedIntegrationSetDigest", json!(OTHER_DIGEST)),
                ("lockSetId", json!(OTHER_ID)),
                ("expectedLockSetDigest", json!(DIGEST)),
                ("supportGateId", json!(ID)),
                ("expectedSupportGateDigest", json!(OTHER_DIGEST)),
                ("expectedSupportGateHistoryEvidenceDigest", json!(DIGEST)),
            ],
        )
    }

    #[test]
    fn apply_separates_task_and_original_approval_lineage() {
        for (request, variant) in [
            (task_apply(), MergeApplyRequestVariant::Task),
            (original_apply(), MergeApplyRequestVariant::Original),
        ] {
            let parsed = accepts::<MergeApplyRequest>(request.clone());
            assert_eq!(parsed.request_variant(), variant);
            assert_eq!(
                parsed.execution_policy(),
                ExecutionPolicy::PreparedJournaledEffect
            );
            assert_runtime_and_schema_accept::<MergeApplyRequest>(request);
        }

        for (field, value) in [
            typed_field::<UnicaId>("planId", json!(ID)),
            typed_field::<Sha256Digest>("expectedPlanDigest", json!(DIGEST)),
            typed_field::<UnicaId>("integrationSetId", json!(ID)),
            typed_field::<Sha256Digest>("expectedIntegrationSetDigest", json!(OTHER_DIGEST)),
            typed_field::<UnicaId>("lockSetId", json!(ID)),
            typed_field::<Sha256Digest>("expectedLockSetDigest", json!(DIGEST)),
            typed_field::<UnicaId>("supportGateId", json!(ID)),
            typed_field::<Sha256Digest>("expectedSupportGateDigest", json!(OTHER_DIGEST)),
            typed_field::<Sha256Digest>("expectedSupportGateHistoryEvidenceDigest", json!(DIGEST)),
        ] {
            assert_runtime_and_schema_reject::<MergeApplyRequest>(without(original_apply(), field));
            assert_runtime_and_schema_reject::<MergeApplyRequest>(with(
                task_apply(),
                &[(field, value)],
            ));
        }
        for invalid_approval in [
            json!({ "digest": DIGEST }),
            json!({ "digest": DIGEST, "decision": "approve" }),
            json!({ "digest": DIGEST, "decision": "apply", "extra": true }),
        ] {
            assert_runtime_and_schema_reject::<MergeApplyRequest>(with(
                task_apply(),
                &[("approval", invalid_approval)],
            ));
        }
    }

    fn local_checkpoint_verify() -> Value {
        with(mutation(), &[("scope", json!("localCheckpoint"))])
    }

    fn synchronized_task_verify() -> Value {
        with(
            mutation(),
            &[
                ("scope", json!("synchronizedTask")),
                ("sessionId", json!(ID)),
            ],
        )
    }

    fn synchronized_task_adapted_verify() -> Value {
        with(
            synchronized_task_verify(),
            &[
                ("adaptationDecisionId", json!(OTHER_ID)),
                ("expectedAdaptationDecisionDigest", json!(DIGEST)),
            ],
        )
    }

    fn main_sandbox_verify() -> Value {
        with(
            mutation(),
            &[
                ("scope", json!("mainSandbox")),
                ("sessionId", json!(ID)),
                ("expectedResolvedSessionDigest", json!(DIGEST)),
                ("supportGateId", json!(OTHER_ID)),
                ("expectedSupportGateDigest", json!(OTHER_DIGEST)),
                ("expectedSupportGateHistoryEvidenceDigest", json!(DIGEST)),
            ],
        )
    }

    fn main_integration_verify() -> Value {
        with(
            main_sandbox_verify(),
            &[
                ("scope", json!("mainIntegration")),
                ("mergeReceiptId", json!(ID)),
                ("integrationSetId", json!(OTHER_ID)),
                ("expectedIntegrationSetDigest", json!(DIGEST)),
            ],
        )
    }

    #[test]
    fn verify_encodes_all_scopes_and_the_exact_adaptation_pair() {
        for (request, variant) in [
            (
                local_checkpoint_verify(),
                MergeVerifyRequestVariant::LocalCheckpoint,
            ),
            (
                synchronized_task_verify(),
                MergeVerifyRequestVariant::SynchronizedTask,
            ),
            (
                synchronized_task_adapted_verify(),
                MergeVerifyRequestVariant::SynchronizedTaskAdapted,
            ),
            (
                main_sandbox_verify(),
                MergeVerifyRequestVariant::MainSandbox,
            ),
            (
                main_integration_verify(),
                MergeVerifyRequestVariant::MainIntegration,
            ),
        ] {
            let parsed = accepts::<MergeVerifyRequest>(request.clone());
            assert_eq!(parsed.request_variant(), variant);
            assert_eq!(parsed.execution_policy(), ExecutionPolicy::Contained);
            assert_runtime_and_schema_accept::<MergeVerifyRequest>(request);
        }

        for field in ["adaptationDecisionId", "expectedAdaptationDecisionDigest"] {
            assert_runtime_and_schema_reject::<MergeVerifyRequest>(without(
                synchronized_task_adapted_verify(),
                field,
            ));
        }
        assert_runtime_and_schema_reject::<MergeVerifyRequest>(without(
            main_integration_verify(),
            "expectedIntegrationSetDigest",
        ));

        let adaptation_fields = || {
            vec![
                typed_field::<UnicaId>("adaptationDecisionId", json!(OTHER_ID)),
                typed_field::<Sha256Digest>("expectedAdaptationDecisionDigest", json!(DIGEST)),
            ]
        };
        let main_fields = || {
            vec![
                typed_field::<Sha256Digest>("expectedResolvedSessionDigest", json!(DIGEST)),
                typed_field::<UnicaId>("supportGateId", json!(OTHER_ID)),
                typed_field::<Sha256Digest>("expectedSupportGateDigest", json!(OTHER_DIGEST)),
                typed_field::<Sha256Digest>(
                    "expectedSupportGateHistoryEvidenceDigest",
                    json!(DIGEST),
                ),
            ]
        };
        let integration_fields = || {
            vec![
                typed_field::<UnicaId>("mergeReceiptId", json!(ID)),
                typed_field::<UnicaId>("integrationSetId", json!(OTHER_ID)),
                typed_field::<Sha256Digest>("expectedIntegrationSetDigest", json!(DIGEST)),
            ]
        };

        let mut local_only_forbidden = vec![typed_field::<UnicaId>("sessionId", json!(ID))];
        local_only_forbidden.extend(adaptation_fields());
        local_only_forbidden.extend(main_fields());
        local_only_forbidden.extend(integration_fields());
        assert_rejects_cross_variant_fields::<MergeVerifyRequest>(
            local_checkpoint_verify(),
            local_only_forbidden,
        );

        let mut synchronized_forbidden = adaptation_fields();
        synchronized_forbidden.extend(main_fields());
        synchronized_forbidden.extend(integration_fields());
        assert_rejects_cross_variant_fields::<MergeVerifyRequest>(
            synchronized_task_verify(),
            synchronized_forbidden,
        );

        let mut adapted_forbidden = main_fields();
        adapted_forbidden.extend(integration_fields());
        assert_rejects_cross_variant_fields::<MergeVerifyRequest>(
            synchronized_task_adapted_verify(),
            adapted_forbidden,
        );

        let mut sandbox_forbidden = adaptation_fields();
        sandbox_forbidden.extend(integration_fields());
        assert_rejects_cross_variant_fields::<MergeVerifyRequest>(
            main_sandbox_verify(),
            sandbox_forbidden,
        );
        assert_rejects_cross_variant_fields::<MergeVerifyRequest>(
            main_integration_verify(),
            adaptation_fields(),
        );
    }

    #[test]
    fn every_untagged_physical_union_is_an_exact_closed_one_of() {
        assert_exact_one_of::<ComparisonSide>(2);
        assert_exact_one_of::<MergeCompareRequest>(2);
        assert_exact_one_of::<MergePrepareRequest>(4);
        assert_exact_one_of::<MergeResolveRequest>(5);
        assert_exact_one_of::<MergeApplyRequest>(2);
        assert_exact_one_of::<MergeVerifyRequest>(5);

        for schema_check in [
            assert_schema_is_closed::<ComparisonSide> as fn(),
            assert_schema_is_closed::<MergeCompareRequest>,
            assert_schema_is_closed::<MergePrepareRequest>,
            assert_schema_is_closed::<MergeConflictsRequest>,
            assert_schema_is_closed::<MergeResolveRequest>,
            assert_schema_is_closed::<MergeApplyRequest>,
            assert_schema_is_closed::<MergeVerifyRequest>,
        ] {
            schema_check();
        }
    }

    #[test]
    fn every_mutating_merge_leaf_physically_repeats_the_common_mutation_fields() {
        assert_common_mutation_is_physical::<MergeCompareRequest>(&[
            compare("projectDelta", json!("taskCurrent"), json!("taskVendor")),
            compare(
                "mainIntegration",
                json!("repository"),
                json!({ "artifactId": ID }),
            ),
        ]);
        assert_common_mutation_is_physical::<MergePrepareRequest>(&[
            supported_update(),
            supported_update_replacement(),
            resolved_replay(),
            main_integration_prepare(),
        ]);
        assert_common_mutation_is_physical::<MergeResolveRequest>(&[
            conflict_resolution("takeOurs"),
            conflict_resolution("takeTheirs"),
            changed_conflict_resolution("combine"),
            changed_conflict_resolution("manual"),
            adapted_delta(),
        ]);
        assert_common_mutation_is_physical::<MergeApplyRequest>(&[task_apply(), original_apply()]);
        assert_common_mutation_is_physical::<MergeVerifyRequest>(&[
            local_checkpoint_verify(),
            synchronized_task_verify(),
            synchronized_task_adapted_verify(),
            main_sandbox_verify(),
            main_integration_verify(),
        ]);
    }

    #[test]
    fn generic_effect_controls_are_absent_from_every_merge_wire_shape() {
        assert_rejects_generic_effect_controls::<MergeCompareRequest>(&[
            compare("projectDelta", json!("taskCurrent"), json!("taskVendor")),
            compare(
                "mainIntegration",
                json!("repository"),
                json!({ "artifactId": ID }),
            ),
        ]);
        assert_rejects_generic_effect_controls::<MergePrepareRequest>(&[
            supported_update(),
            supported_update_replacement(),
            resolved_replay(),
            main_integration_prepare(),
        ]);
        assert_rejects_generic_effect_controls::<MergeConflictsRequest>(&[conflicts()]);
        assert_rejects_generic_effect_controls::<MergeResolveRequest>(&[
            conflict_resolution("takeOurs"),
            conflict_resolution("takeTheirs"),
            changed_conflict_resolution("combine"),
            changed_conflict_resolution("manual"),
            adapted_delta(),
        ]);
        assert_rejects_generic_effect_controls::<MergeVerifyRequest>(&[
            local_checkpoint_verify(),
            synchronized_task_verify(),
            synchronized_task_adapted_verify(),
            main_sandbox_verify(),
            main_integration_verify(),
        ]);

        for request in [task_apply(), original_apply()] {
            for (field, value) in [
                ("dryRun", json!(false)),
                ("approvedPreviewDigest", json!(DIGEST)),
            ] {
                assert_runtime_and_schema_reject::<MergeApplyRequest>(with(
                    request.clone(),
                    &[(field, value)],
                ));
            }
        }
    }

    #[test]
    fn malformed_values_never_select_an_execution_policy() {
        assert!(MergeCompareRequest::execution_policy_for_json(&without(
            compare("projectDelta", json!("taskCurrent"), json!("taskVendor")),
            "taskId"
        ))
        .is_none());
        assert!(MergePrepareRequest::execution_policy_for_json(&with(
            supported_update(),
            &[("dryRun", json!(true))]
        ))
        .is_none());
        assert!(MergeConflictsRequest::execution_policy_for_json(&with(
            conflicts(),
            &[("operationId", json!(OPERATION_ID))]
        ))
        .is_none());
        assert!(
            MergeResolveRequest::execution_policy_for_json(&conflict_resolution("manual"))
                .is_none()
        );
        assert!(
            MergeApplyRequest::execution_policy_for_json(&without(task_apply(), "approval"))
                .is_none()
        );
        assert!(MergeVerifyRequest::execution_policy_for_json(&without(
            synchronized_task_adapted_verify(),
            "adaptationDecisionId"
        ))
        .is_none());
    }
}
