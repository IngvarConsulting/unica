use super::{
    execution_policy_for_json, request_one_of_schema, DigestApproval, DigestApprovalMismatch,
    FalseLiteral, TrueLiteral,
};
use crate::domain::branched_development::contracts::scalars::OriginalProjectCwd;
use crate::domain::branched_development::{
    ExecutionPolicy, OperationId, Sha256Digest, TaskId, UnicaId,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RepositoryStatusRequest {
    cwd: OriginalProjectCwd,
    task_id: TaskId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RepositoryStatusRequestVariant {
    Status,
}

impl RepositoryStatusRequest {
    pub(crate) const fn request_variant(&self) -> RepositoryStatusRequestVariant {
        RepositoryStatusRequestVariant::Status
    }

    pub(crate) const fn execution_policy(&self) -> ExecutionPolicy {
        ExecutionPolicy::ReadOnly
    }

    pub(crate) fn execution_policy_for_json(value: &Value) -> Option<ExecutionPolicy> {
        execution_policy_for_json::<Self>(value, Self::execution_policy)
    }
}

string_literal!(RoutineMode, Value, "routine");
string_literal!(SupportPrerequisiteArmMode, Value, "supportPrerequisiteArm");
string_literal!(SupportPrerequisiteMode, Value, "supportPrerequisite");
string_literal!(
    SupportPrerequisiteCancellationMode,
    Value,
    "supportPrerequisiteCancellation"
);
string_literal!(PreviewStage, Value, "preview");
string_literal!(ApplyStage, Value, "apply");

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
enum SupportCancellationReason {
    TaskChanged,
    Abandoned,
    OperatorCancelled,
}

macro_rules! routine_update_leaf {
    ($name:ident $(, $field:ident : $field_type:ty )* $(,)?) => {
        #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        pub(crate) struct $name {
            cwd: OriginalProjectCwd,
            task_id: TaskId,
            operation_id: OperationId,
            mode: RoutineMode,
            expected_status_digest: Sha256Digest,
            $($field: $field_type,)*
        }
    };
}

routine_update_leaf!(RoutinePreviewOmitted);
routine_update_leaf!(RoutinePreviewExplicit, dry_run: TrueLiteral);
routine_update_leaf!(
    RoutineApply,
    dry_run: FalseLiteral,
    approved_update_digest: Sha256Digest,
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SupportPrerequisiteArmPreview {
    cwd: OriginalProjectCwd,
    task_id: TaskId,
    mode: SupportPrerequisiteArmMode,
    stage: PreviewStage,
    expected_status_digest: Sha256Digest,
    support_action_id: UnicaId,
    expected_support_action_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SupportPrerequisiteArmApply {
    cwd: OriginalProjectCwd,
    task_id: TaskId,
    operation_id: OperationId,
    mode: SupportPrerequisiteArmMode,
    stage: ApplyStage,
    expected_status_digest: Sha256Digest,
    support_action_id: UnicaId,
    expected_support_action_digest: Sha256Digest,
    approved_arming_digest: Sha256Digest,
}

macro_rules! prerequisite_update_leaf {
    ($name:ident $(, $field:ident : $field_type:ty )* $(,)?) => {
        #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        pub(crate) struct $name {
            cwd: OriginalProjectCwd,
            task_id: TaskId,
            operation_id: OperationId,
            mode: SupportPrerequisiteMode,
            expected_status_digest: Sha256Digest,
            support_action_id: UnicaId,
            expected_support_action_digest: Sha256Digest,
            expected_arming_receipt_id: UnicaId,
            expected_arming_receipt_digest: Sha256Digest,
            $($field: $field_type,)*
        }
    };
}

prerequisite_update_leaf!(PrerequisitePreviewOmitted);
prerequisite_update_leaf!(PrerequisitePreviewExplicit, dry_run: TrueLiteral);
prerequisite_update_leaf!(
    PrerequisiteApply,
    dry_run: FalseLiteral,
    approved_update_digest: Sha256Digest,
);

macro_rules! cancellation_update_leaf {
    ($name:ident $(, $field:ident : $field_type:ty )* $(,)?) => {
        #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        pub(crate) struct $name {
            cwd: OriginalProjectCwd,
            task_id: TaskId,
            operation_id: OperationId,
            mode: SupportPrerequisiteCancellationMode,
            expected_status_digest: Sha256Digest,
            support_action_id: UnicaId,
            expected_support_action_digest: Sha256Digest,
            reason: SupportCancellationReason,
            $($field: $field_type,)*
        }
    };
}

cancellation_update_leaf!(CancellationAwaitingPreviewOmitted);
cancellation_update_leaf!(CancellationAwaitingPreviewExplicit, dry_run: TrueLiteral);
cancellation_update_leaf!(
    CancellationAwaitingApply,
    dry_run: FalseLiteral,
    approved_cancellation_digest: Sha256Digest,
);
cancellation_update_leaf!(
    CancellationArmedPreviewOmitted,
    expected_arming_receipt_id: UnicaId,
    expected_arming_receipt_digest: Sha256Digest,
);
cancellation_update_leaf!(
    CancellationArmedPreviewExplicit,
    expected_arming_receipt_id: UnicaId,
    expected_arming_receipt_digest: Sha256Digest,
    dry_run: TrueLiteral,
);
cancellation_update_leaf!(
    CancellationArmedApply,
    expected_arming_receipt_id: UnicaId,
    expected_arming_receipt_digest: Sha256Digest,
    dry_run: FalseLiteral,
    approved_cancellation_digest: Sha256Digest,
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub(crate) enum RepositoryUpdateRequest {
    RoutinePreviewOmitted(RoutinePreviewOmitted),
    RoutinePreviewExplicit(RoutinePreviewExplicit),
    RoutineApply(RoutineApply),
    ArmPreview(SupportPrerequisiteArmPreview),
    ArmApply(SupportPrerequisiteArmApply),
    PrerequisitePreviewOmitted(PrerequisitePreviewOmitted),
    PrerequisitePreviewExplicit(PrerequisitePreviewExplicit),
    PrerequisiteApply(PrerequisiteApply),
    CancellationAwaitingPreviewOmitted(CancellationAwaitingPreviewOmitted),
    CancellationAwaitingPreviewExplicit(CancellationAwaitingPreviewExplicit),
    CancellationAwaitingApply(CancellationAwaitingApply),
    CancellationArmedPreviewOmitted(CancellationArmedPreviewOmitted),
    CancellationArmedPreviewExplicit(CancellationArmedPreviewExplicit),
    CancellationArmedApply(CancellationArmedApply),
}

request_one_of_schema!(
    RepositoryUpdateRequest,
    "RepositoryUpdateRequest",
    [
        RoutinePreviewOmitted,
        RoutinePreviewExplicit,
        RoutineApply,
        SupportPrerequisiteArmPreview,
        SupportPrerequisiteArmApply,
        PrerequisitePreviewOmitted,
        PrerequisitePreviewExplicit,
        PrerequisiteApply,
        CancellationAwaitingPreviewOmitted,
        CancellationAwaitingPreviewExplicit,
        CancellationAwaitingApply,
        CancellationArmedPreviewOmitted,
        CancellationArmedPreviewExplicit,
        CancellationArmedApply,
    ]
);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RepositoryUpdateRequestVariant {
    RoutinePreview,
    RoutineApply,
    ArmPreview,
    ArmApply,
    PrerequisitePreview,
    PrerequisiteApply,
    CancellationPreview,
    CancellationApply,
}

impl RepositoryUpdateRequest {
    pub(crate) const fn request_variant(&self) -> RepositoryUpdateRequestVariant {
        match self {
            Self::RoutinePreviewOmitted(_) | Self::RoutinePreviewExplicit(_) => {
                RepositoryUpdateRequestVariant::RoutinePreview
            }
            Self::RoutineApply(_) => RepositoryUpdateRequestVariant::RoutineApply,
            Self::ArmPreview(_) => RepositoryUpdateRequestVariant::ArmPreview,
            Self::ArmApply(_) => RepositoryUpdateRequestVariant::ArmApply,
            Self::PrerequisitePreviewOmitted(_) | Self::PrerequisitePreviewExplicit(_) => {
                RepositoryUpdateRequestVariant::PrerequisitePreview
            }
            Self::PrerequisiteApply(_) => RepositoryUpdateRequestVariant::PrerequisiteApply,
            Self::CancellationAwaitingPreviewOmitted(_)
            | Self::CancellationAwaitingPreviewExplicit(_)
            | Self::CancellationArmedPreviewOmitted(_)
            | Self::CancellationArmedPreviewExplicit(_) => {
                RepositoryUpdateRequestVariant::CancellationPreview
            }
            Self::CancellationAwaitingApply(_) | Self::CancellationArmedApply(_) => {
                RepositoryUpdateRequestVariant::CancellationApply
            }
        }
    }

    pub(crate) const fn execution_policy(&self) -> ExecutionPolicy {
        match self {
            Self::ArmPreview(_) => ExecutionPolicy::ReadOnly,
            Self::ArmApply(_) => ExecutionPolicy::LocalJournaled,
            Self::RoutinePreviewOmitted(_)
            | Self::RoutinePreviewExplicit(_)
            | Self::RoutineApply(_)
            | Self::PrerequisitePreviewOmitted(_)
            | Self::PrerequisitePreviewExplicit(_)
            | Self::PrerequisiteApply(_)
            | Self::CancellationAwaitingPreviewOmitted(_)
            | Self::CancellationAwaitingPreviewExplicit(_)
            | Self::CancellationAwaitingApply(_)
            | Self::CancellationArmedPreviewOmitted(_)
            | Self::CancellationArmedPreviewExplicit(_)
            | Self::CancellationArmedApply(_) => ExecutionPolicy::PreviewedJournaledEffect,
        }
    }

    pub(crate) fn execution_policy_for_json(value: &Value) -> Option<ExecutionPolicy> {
        execution_policy_for_json::<Self>(value, Self::execution_policy)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RepositoryPlanLocksRequest {
    cwd: OriginalProjectCwd,
    task_id: TaskId,
    operation_id: OperationId,
    comparison_id: UnicaId,
    merge_session_id: UnicaId,
    expected_resolved_session_digest: Sha256Digest,
    verification_id: UnicaId,
    expected_verification_digest: Sha256Digest,
    support_gate_id: UnicaId,
    expected_support_gate_digest: Sha256Digest,
    expected_support_gate_history_evidence_digest: Sha256Digest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RepositoryPlanLocksRequestVariant {
    PlanLocks,
}

impl RepositoryPlanLocksRequest {
    pub(crate) const fn request_variant(&self) -> RepositoryPlanLocksRequestVariant {
        RepositoryPlanLocksRequestVariant::PlanLocks
    }

    pub(crate) const fn execution_policy(&self) -> ExecutionPolicy {
        ExecutionPolicy::Contained
    }

    pub(crate) fn execution_policy_for_json(value: &Value) -> Option<ExecutionPolicy> {
        execution_policy_for_json::<Self>(value, Self::execution_policy)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RepositoryLockRequest {
    cwd: OriginalProjectCwd,
    task_id: TaskId,
    operation_id: OperationId,
    plan_id: UnicaId,
    approval: DigestApproval,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RepositoryLockRequestVariant {
    Lock,
}

impl RepositoryLockRequest {
    pub(crate) const fn request_variant(&self) -> RepositoryLockRequestVariant {
        RepositoryLockRequestVariant::Lock
    }

    pub(crate) const fn execution_policy(&self) -> ExecutionPolicy {
        ExecutionPolicy::JournaledEffect
    }

    pub(crate) fn execution_policy_for_json(value: &Value) -> Option<ExecutionPolicy> {
        execution_policy_for_json::<Self>(value, Self::execution_policy)
    }

    // The plan digest is intentionally not caller-supplied separately. The
    // handler must resolve the immutable plan and cross this validation
    // boundary before acquiring the first lock.
    pub(crate) fn validate_plan_digest(
        &self,
        plan_digest: &Sha256Digest,
    ) -> Result<(), DigestApprovalMismatch> {
        self.approval.validate_digest(plan_digest)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
enum UnlockReason {
    Compensation,
    Rollback,
    Abandonment,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RepositoryUnlockRequest {
    cwd: OriginalProjectCwd,
    task_id: TaskId,
    operation_id: OperationId,
    lock_set_id: UnicaId,
    expected_lock_set_digest: Sha256Digest,
    reason: UnlockReason,
    approval: DigestApproval,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RepositoryUnlockRequestVariant {
    Compensation,
    Rollback,
    Abandonment,
}

impl RepositoryUnlockRequest {
    pub(crate) const fn request_variant(&self) -> RepositoryUnlockRequestVariant {
        match self.reason {
            UnlockReason::Compensation => RepositoryUnlockRequestVariant::Compensation,
            UnlockReason::Rollback => RepositoryUnlockRequestVariant::Rollback,
            UnlockReason::Abandonment => RepositoryUnlockRequestVariant::Abandonment,
        }
    }

    pub(crate) const fn execution_policy(&self) -> ExecutionPolicy {
        ExecutionPolicy::JournaledEffect
    }

    pub(crate) fn execution_policy_for_json(value: &Value) -> Option<ExecutionPolicy> {
        execution_policy_for_json::<Self>(value, Self::execution_policy)
    }

    // Shape validation deliberately accepts a syntactically valid stale
    // approval so the handler can return approvalDigestMismatch. This domain
    // boundary must run before the first unlock effect.
    pub(crate) fn validate_approval_digest(&self) -> Result<(), DigestApprovalMismatch> {
        self.approval
            .validate_digest(&self.expected_lock_set_digest)
    }
}

macro_rules! commit_leaf {
    ($name:ident $(, $field:ident : $field_type:ty )* $(,)?) => {
        #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        pub(crate) struct $name {
            cwd: OriginalProjectCwd,
            task_id: TaskId,
            operation_id: OperationId,
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
            $($field: $field_type,)*
        }
    };
}

commit_leaf!(CommitPreviewOmitted);
commit_leaf!(CommitPreviewExplicit, dry_run: TrueLiteral);
commit_leaf!(
    CommitApply,
    dry_run: FalseLiteral,
    approved_commit_digest: Sha256Digest,
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub(crate) enum RepositoryCommitRequest {
    PreviewOmitted(CommitPreviewOmitted),
    PreviewExplicit(CommitPreviewExplicit),
    Apply(CommitApply),
}

request_one_of_schema!(
    RepositoryCommitRequest,
    "RepositoryCommitRequest",
    [CommitPreviewOmitted, CommitPreviewExplicit, CommitApply]
);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RepositoryCommitRequestVariant {
    Preview,
    Apply,
}

impl RepositoryCommitRequest {
    pub(crate) const fn request_variant(&self) -> RepositoryCommitRequestVariant {
        match self {
            Self::PreviewOmitted(_) | Self::PreviewExplicit(_) => {
                RepositoryCommitRequestVariant::Preview
            }
            Self::Apply(_) => RepositoryCommitRequestVariant::Apply,
        }
    }

    pub(crate) const fn execution_policy(&self) -> ExecutionPolicy {
        ExecutionPolicy::PreviewedJournaledEffect
    }

    pub(crate) fn execution_policy_for_json(value: &Value) -> Option<ExecutionPolicy> {
        execution_policy_for_json::<Self>(value, Self::execution_policy)
    }
}

string_literal!(RecoverApplyDecision, Value, "apply");
string_literal!(RecoverCancelDecision, Value, "cancel");

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RecoverApply {
    cwd: OriginalProjectCwd,
    task_id: TaskId,
    operation_id: OperationId,
    decision: RecoverApplyDecision,
    expected_recovery_digest: Sha256Digest,
    approval: DigestApproval,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RecoverCancel {
    cwd: OriginalProjectCwd,
    task_id: TaskId,
    operation_id: OperationId,
    decision: RecoverCancelDecision,
    expected_recovery_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub(crate) enum RepositoryRecoverRequest {
    Apply(RecoverApply),
    Cancel(RecoverCancel),
}

request_one_of_schema!(
    RepositoryRecoverRequest,
    "RepositoryRecoverRequest",
    [RecoverApply, RecoverCancel]
);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RepositoryRecoverRequestVariant {
    RecoverApply,
    RecoverCancel,
}

impl RepositoryRecoverRequest {
    pub(crate) const fn request_variant(&self) -> RepositoryRecoverRequestVariant {
        match self {
            Self::Apply(_) => RepositoryRecoverRequestVariant::RecoverApply,
            Self::Cancel(_) => RepositoryRecoverRequestVariant::RecoverCancel,
        }
    }

    pub(crate) const fn execution_policy(&self) -> ExecutionPolicy {
        match self {
            Self::Apply(_) => ExecutionPolicy::JournaledEffect,
            Self::Cancel(_) => ExecutionPolicy::LocalJournaled,
        }
    }

    pub(crate) fn execution_policy_for_json(value: &Value) -> Option<ExecutionPolicy> {
        execution_policy_for_json::<Self>(value, Self::execution_policy)
    }

    // Cancel has no approval. Apply is shape-valid even with a stale digest so
    // that this validation failure can be mapped to approvalDigestMismatch.
    pub(crate) fn validate_approval_digest(&self) -> Result<(), DigestApprovalMismatch> {
        match self {
            Self::Apply(request) => request
                .approval
                .validate_digest(&request.expected_recovery_digest),
            Self::Cancel(_) => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        RepositoryCommitRequest, RepositoryCommitRequestVariant, RepositoryLockRequest,
        RepositoryLockRequestVariant, RepositoryPlanLocksRequest,
        RepositoryPlanLocksRequestVariant, RepositoryRecoverRequest,
        RepositoryRecoverRequestVariant, RepositoryStatusRequest, RepositoryStatusRequestVariant,
        RepositoryUnlockRequest, RepositoryUnlockRequestVariant, RepositoryUpdateRequest,
        RepositoryUpdateRequestVariant,
    };
    use crate::domain::branched_development::contracts::schema::{
        audit_json_schema, is_i_json_lf_text, is_i_json_single_line_text,
        is_normalized_utc_instant, I_JSON_LF_TEXT_FORMAT, I_JSON_SINGLE_LINE_TEXT_FORMAT,
        NORMALIZED_UTC_INSTANT_FORMAT,
    };
    use crate::domain::branched_development::{ExecutionPolicy, Sha256Digest};
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

    fn task() -> Value {
        json!({ "cwd": CWD, "taskId": TASK_ID })
    }

    fn mutation() -> Value {
        json!({ "cwd": CWD, "taskId": TASK_ID, "operationId": OPERATION_ID })
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

    fn assert_accept<T: DeserializeOwned + JsonSchema>(value: Value) -> T {
        let parsed = accepts::<T>(value.clone());
        assert!(
            schema_validator::<T>().is_valid(&value),
            "schema rejected runtime-valid request {value}"
        );
        parsed
    }

    fn assert_reject<T: DeserializeOwned + JsonSchema>(value: Value) {
        rejects::<T>(value.clone());
        assert!(
            !schema_validator::<T>().is_valid(&value),
            "schema accepted runtime-invalid request {value}"
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
            Value::Array(values) => values
                .iter()
                .any(|nested| contains_keyword(nested, keyword)),
            _ => false,
        }
    }

    fn assert_closed<T: JsonSchema>() {
        let schema = serde_json::to_value(schema_for!(T)).unwrap();
        audit_json_schema(&schema).expect("repository request schema must be recursively closed");
    }

    fn assert_exact_one_of<T: JsonSchema>(branches: usize) {
        let schema = serde_json::to_value(schema_for!(T)).unwrap();
        assert_eq!(
            schema.get("oneOf").and_then(Value::as_array).map(Vec::len),
            Some(branches)
        );
        assert!(!contains_keyword(&schema, "anyOf"));
    }

    fn approval() -> Value {
        json!({ "digest": DIGEST, "decision": "apply" })
    }

    fn approval_with_digest(digest: &str) -> Value {
        json!({ "digest": digest, "decision": "apply" })
    }

    fn routine_base() -> Value {
        with(
            mutation(),
            &[
                ("mode", json!("routine")),
                ("expectedStatusDigest", json!(DIGEST)),
            ],
        )
    }

    fn arm_preview() -> Value {
        with(
            task(),
            &[
                ("mode", json!("supportPrerequisiteArm")),
                ("stage", json!("preview")),
                ("expectedStatusDigest", json!(DIGEST)),
                ("supportActionId", json!(ID)),
                ("expectedSupportActionDigest", json!(OTHER_DIGEST)),
            ],
        )
    }

    fn arm_apply() -> Value {
        with(
            mutation(),
            &[
                ("mode", json!("supportPrerequisiteArm")),
                ("stage", json!("apply")),
                ("expectedStatusDigest", json!(DIGEST)),
                ("supportActionId", json!(ID)),
                ("expectedSupportActionDigest", json!(OTHER_DIGEST)),
                ("approvedArmingDigest", json!(DIGEST)),
            ],
        )
    }

    fn prerequisite_base() -> Value {
        with(
            mutation(),
            &[
                ("mode", json!("supportPrerequisite")),
                ("expectedStatusDigest", json!(DIGEST)),
                ("supportActionId", json!(ID)),
                ("expectedSupportActionDigest", json!(OTHER_DIGEST)),
                ("expectedArmingReceiptId", json!(OTHER_ID)),
                ("expectedArmingReceiptDigest", json!(DIGEST)),
            ],
        )
    }

    fn cancellation_base(armed: bool) -> Value {
        let base = with(
            mutation(),
            &[
                ("mode", json!("supportPrerequisiteCancellation")),
                ("expectedStatusDigest", json!(DIGEST)),
                ("supportActionId", json!(ID)),
                ("expectedSupportActionDigest", json!(OTHER_DIGEST)),
                ("reason", json!("operatorCancelled")),
            ],
        );
        if armed {
            with(
                base,
                &[
                    ("expectedArmingReceiptId", json!(OTHER_ID)),
                    ("expectedArmingReceiptDigest", json!(DIGEST)),
                ],
            )
        } else {
            base
        }
    }

    fn preview_leaves(base: Value, approval_name: &str) -> [Value; 3] {
        [
            base.clone(),
            with(base.clone(), &[("dryRun", json!(true))]),
            with(
                base,
                &[("dryRun", json!(false)), (approval_name, json!(DIGEST))],
            ),
        ]
    }

    #[test]
    fn status_is_the_exact_read_only_common_task_record() {
        let parsed = assert_accept::<RepositoryStatusRequest>(task());
        assert_eq!(
            parsed.request_variant(),
            RepositoryStatusRequestVariant::Status
        );
        assert_eq!(parsed.execution_policy(), ExecutionPolicy::ReadOnly);

        for invalid in [
            without(task(), "cwd"),
            without(task(), "taskId"),
            with(task(), &[("operationId", json!(OPERATION_ID))]),
            with(task(), &[("extra", json!(true))]),
            with(task(), &[("cwd", json!("/invalid\tpath"))]),
        ] {
            assert_reject::<RepositoryStatusRequest>(invalid);
        }
    }

    #[test]
    fn update_accepts_all_fourteen_physical_leaves_and_maps_every_logical_variant() {
        let routine = preview_leaves(routine_base(), "approvedUpdateDigest");
        let prerequisite = preview_leaves(prerequisite_base(), "approvedUpdateDigest");
        let awaiting = preview_leaves(cancellation_base(false), "approvedCancellationDigest");
        let armed = preview_leaves(cancellation_base(true), "approvedCancellationDigest");

        let cases = [
            (
                routine[0].clone(),
                RepositoryUpdateRequestVariant::RoutinePreview,
                ExecutionPolicy::PreviewedJournaledEffect,
            ),
            (
                routine[1].clone(),
                RepositoryUpdateRequestVariant::RoutinePreview,
                ExecutionPolicy::PreviewedJournaledEffect,
            ),
            (
                routine[2].clone(),
                RepositoryUpdateRequestVariant::RoutineApply,
                ExecutionPolicy::PreviewedJournaledEffect,
            ),
            (
                arm_preview(),
                RepositoryUpdateRequestVariant::ArmPreview,
                ExecutionPolicy::ReadOnly,
            ),
            (
                arm_apply(),
                RepositoryUpdateRequestVariant::ArmApply,
                ExecutionPolicy::LocalJournaled,
            ),
            (
                prerequisite[0].clone(),
                RepositoryUpdateRequestVariant::PrerequisitePreview,
                ExecutionPolicy::PreviewedJournaledEffect,
            ),
            (
                prerequisite[1].clone(),
                RepositoryUpdateRequestVariant::PrerequisitePreview,
                ExecutionPolicy::PreviewedJournaledEffect,
            ),
            (
                prerequisite[2].clone(),
                RepositoryUpdateRequestVariant::PrerequisiteApply,
                ExecutionPolicy::PreviewedJournaledEffect,
            ),
            (
                awaiting[0].clone(),
                RepositoryUpdateRequestVariant::CancellationPreview,
                ExecutionPolicy::PreviewedJournaledEffect,
            ),
            (
                awaiting[1].clone(),
                RepositoryUpdateRequestVariant::CancellationPreview,
                ExecutionPolicy::PreviewedJournaledEffect,
            ),
            (
                awaiting[2].clone(),
                RepositoryUpdateRequestVariant::CancellationApply,
                ExecutionPolicy::PreviewedJournaledEffect,
            ),
            (
                armed[0].clone(),
                RepositoryUpdateRequestVariant::CancellationPreview,
                ExecutionPolicy::PreviewedJournaledEffect,
            ),
            (
                armed[1].clone(),
                RepositoryUpdateRequestVariant::CancellationPreview,
                ExecutionPolicy::PreviewedJournaledEffect,
            ),
            (
                armed[2].clone(),
                RepositoryUpdateRequestVariant::CancellationApply,
                ExecutionPolicy::PreviewedJournaledEffect,
            ),
        ];

        for (value, variant, policy) in cases {
            let parsed = assert_accept::<RepositoryUpdateRequest>(value.clone());
            assert_eq!(parsed.request_variant(), variant);
            assert_eq!(parsed.execution_policy(), policy);
            assert_eq!(
                RepositoryUpdateRequest::execution_policy_for_json(&value),
                Some(policy)
            );
        }
    }

    #[test]
    fn update_rejects_cross_leaf_controls_partial_pairs_and_invalid_scalars() {
        let routine = preview_leaves(routine_base(), "approvedUpdateDigest");
        let prerequisite = preview_leaves(prerequisite_base(), "approvedUpdateDigest");
        let awaiting = preview_leaves(cancellation_base(false), "approvedCancellationDigest");
        let armed = preview_leaves(cancellation_base(true), "approvedCancellationDigest");

        for invalid in [
            with(arm_preview(), &[("operationId", json!(OPERATION_ID))]),
            with(arm_preview(), &[("dryRun", json!(true))]),
            with(arm_preview(), &[("approvedArmingDigest", json!(DIGEST))]),
            with(arm_apply(), &[("dryRun", json!(false))]),
            without(arm_apply(), "approvedArmingDigest"),
            with(routine[0].clone(), &[("stage", json!("preview"))]),
            with(routine[0].clone(), &[("dryRun", Value::Null)]),
            with(
                routine[0].clone(),
                &[("approvedUpdateDigest", json!(DIGEST))],
            ),
            without(routine[2].clone(), "approvedUpdateDigest"),
            with(
                routine[2].clone(),
                &[("approvedCancellationDigest", json!(DIGEST))],
            ),
            without(prerequisite[0].clone(), "expectedArmingReceiptId"),
            without(prerequisite[0].clone(), "expectedArmingReceiptDigest"),
            with(
                prerequisite[2].clone(),
                &[("approvedCancellationDigest", json!(DIGEST))],
            ),
            with(
                cancellation_base(false),
                &[("expectedArmingReceiptId", json!(ID))],
            ),
            with(
                cancellation_base(false),
                &[("expectedArmingReceiptDigest", json!(DIGEST))],
            ),
            without(armed[0].clone(), "expectedArmingReceiptId"),
            without(armed[0].clone(), "expectedArmingReceiptDigest"),
            with(awaiting[0].clone(), &[("reason", json!("cancel"))]),
            with(awaiting[0].clone(), &[("mode", json!("routineUpdate"))]),
            with(awaiting[0].clone(), &[("operationId", json!("BAD"))]),
            with(awaiting[0].clone(), &[("taskId", json!("bad task"))]),
            with(
                awaiting[0].clone(),
                &[("expectedStatusDigest", json!("AA"))],
            ),
            with(
                awaiting[2].clone(),
                &[("approvedUpdateDigest", json!(DIGEST))],
            ),
        ] {
            assert_reject::<RepositoryUpdateRequest>(invalid.clone());
            assert!(RepositoryUpdateRequest::execution_policy_for_json(&invalid).is_none());
        }
    }

    #[test]
    fn every_physical_leaf_repeats_its_common_selectors_and_no_other_leaf_gets_stage() {
        for value in [task(), arm_preview()] {
            for field in ["cwd", "taskId"] {
                if value.get("mode").is_some() {
                    assert_reject::<RepositoryUpdateRequest>(without(value.clone(), field));
                } else {
                    assert_reject::<RepositoryStatusRequest>(without(value.clone(), field));
                }
            }
        }

        let routine = preview_leaves(routine_base(), "approvedUpdateDigest");
        let prerequisite = preview_leaves(prerequisite_base(), "approvedUpdateDigest");
        let awaiting = preview_leaves(cancellation_base(false), "approvedCancellationDigest");
        let armed = preview_leaves(cancellation_base(true), "approvedCancellationDigest");
        let mutating_update_leaves = routine
            .into_iter()
            .chain([arm_apply()])
            .chain(prerequisite)
            .chain(awaiting)
            .chain(armed)
            .collect::<Vec<_>>();

        for value in &mutating_update_leaves {
            for field in ["cwd", "taskId", "operationId"] {
                assert_reject::<RepositoryUpdateRequest>(without(value.clone(), field));
            }
        }

        for value in mutating_update_leaves
            .iter()
            .filter(|value| value.get("stage").is_none())
        {
            assert_reject::<RepositoryUpdateRequest>(with(
                value.clone(),
                &[("stage", json!("apply"))],
            ));
        }

        for value in [plan_locks()] {
            for field in ["cwd", "taskId", "operationId"] {
                assert_reject::<RepositoryPlanLocksRequest>(without(value.clone(), field));
            }
        }
        for value in [lock()] {
            for field in ["cwd", "taskId", "operationId"] {
                assert_reject::<RepositoryLockRequest>(without(value.clone(), field));
            }
        }
        for value in [
            unlock("compensation"),
            unlock("rollback"),
            unlock("abandonment"),
        ] {
            for field in ["cwd", "taskId", "operationId"] {
                assert_reject::<RepositoryUnlockRequest>(without(value.clone(), field));
            }
        }

        for value in preview_leaves(commit_base(), "approvedCommitDigest") {
            for field in ["cwd", "taskId", "operationId"] {
                assert_reject::<RepositoryCommitRequest>(without(value.clone(), field));
            }
        }
        for value in [recover_apply(), recover_cancel()] {
            for field in ["cwd", "taskId", "operationId"] {
                assert_reject::<RepositoryRecoverRequest>(without(value.clone(), field));
            }
        }
    }

    fn plan_locks() -> Value {
        with(
            mutation(),
            &[
                ("comparisonId", json!(ID)),
                ("mergeSessionId", json!(OTHER_ID)),
                ("expectedResolvedSessionDigest", json!(DIGEST)),
                ("verificationId", json!(ID)),
                ("expectedVerificationDigest", json!(OTHER_DIGEST)),
                ("supportGateId", json!(OTHER_ID)),
                ("expectedSupportGateDigest", json!(DIGEST)),
                (
                    "expectedSupportGateHistoryEvidenceDigest",
                    json!(OTHER_DIGEST),
                ),
            ],
        )
    }

    fn lock() -> Value {
        with(
            mutation(),
            &[("planId", json!(ID)), ("approval", approval())],
        )
    }

    fn unlock(reason: &str) -> Value {
        with(
            mutation(),
            &[
                ("lockSetId", json!(ID)),
                ("expectedLockSetDigest", json!(DIGEST)),
                ("reason", json!(reason)),
                ("approval", approval()),
            ],
        )
    }

    fn commit_base() -> Value {
        with(
            mutation(),
            &[
                ("integrationSetId", json!(ID)),
                ("expectedIntegrationSetDigest", json!(DIGEST)),
                ("lockSetId", json!(OTHER_ID)),
                ("expectedLockSetDigest", json!(OTHER_DIGEST)),
                ("verificationId", json!(ID)),
                ("expectedVerificationDigest", json!(DIGEST)),
                ("mergeReceiptId", json!(OTHER_ID)),
                ("supportGateId", json!(ID)),
                ("expectedSupportGateDigest", json!(OTHER_DIGEST)),
                ("expectedSupportGateHistoryEvidenceDigest", json!(DIGEST)),
                (
                    "expectedAuthorizedPostMergeFingerprint",
                    json!(OTHER_DIGEST),
                ),
            ],
        )
    }

    #[test]
    fn lock_window_requests_are_exact_and_have_exhaustive_policy_selection() {
        let parsed = assert_accept::<RepositoryPlanLocksRequest>(plan_locks());
        assert_eq!(
            parsed.request_variant(),
            RepositoryPlanLocksRequestVariant::PlanLocks
        );
        assert_eq!(parsed.execution_policy(), ExecutionPolicy::Contained);

        let parsed = assert_accept::<RepositoryLockRequest>(lock());
        assert_eq!(parsed.request_variant(), RepositoryLockRequestVariant::Lock);
        assert_eq!(parsed.execution_policy(), ExecutionPolicy::JournaledEffect);
        let plan_digest = Sha256Digest::parse(DIGEST).unwrap();
        let other_plan_digest = Sha256Digest::parse(OTHER_DIGEST).unwrap();
        assert!(parsed.validate_plan_digest(&plan_digest).is_ok());
        let mismatch = parsed.validate_plan_digest(&other_plan_digest).unwrap_err();
        assert_eq!(mismatch.expected().as_str(), OTHER_DIGEST);
        assert_eq!(mismatch.observed().as_str(), DIGEST);

        let externally_mismatched_lock =
            with(lock(), &[("approval", approval_with_digest(OTHER_DIGEST))]);
        let parsed = assert_accept::<RepositoryLockRequest>(externally_mismatched_lock);
        let mismatch = parsed.validate_plan_digest(&plan_digest).unwrap_err();
        assert_eq!(mismatch.expected().as_str(), DIGEST);
        assert_eq!(mismatch.observed().as_str(), OTHER_DIGEST);

        for (reason, variant) in [
            ("compensation", RepositoryUnlockRequestVariant::Compensation),
            ("rollback", RepositoryUnlockRequestVariant::Rollback),
            ("abandonment", RepositoryUnlockRequestVariant::Abandonment),
        ] {
            let value = unlock(reason);
            let parsed = assert_accept::<RepositoryUnlockRequest>(value.clone());
            assert_eq!(parsed.request_variant(), variant);
            assert_eq!(parsed.execution_policy(), ExecutionPolicy::JournaledEffect);
            assert!(parsed.validate_approval_digest().is_ok());
            assert_eq!(
                RepositoryUnlockRequest::execution_policy_for_json(&value),
                Some(ExecutionPolicy::JournaledEffect)
            );
        }

        for (value, missing) in [
            (plan_locks(), "expectedSupportGateHistoryEvidenceDigest"),
            (lock(), "approval"),
            (unlock("rollback"), "expectedLockSetDigest"),
        ] {
            let invalid = without(value, missing);
            assert!(RepositoryPlanLocksRequest::execution_policy_for_json(&invalid).is_none());
            assert!(RepositoryLockRequest::execution_policy_for_json(&invalid).is_none());
            assert!(RepositoryUnlockRequest::execution_policy_for_json(&invalid).is_none());
        }
        assert_reject::<RepositoryLockRequest>(with(
            lock(),
            &[(
                "approval",
                json!({ "digest": DIGEST, "decision": "approve" }),
            )],
        ));
        assert_reject::<RepositoryUnlockRequest>(with(
            unlock("rollback"),
            &[("reason", json!("force"))],
        ));
        assert_reject::<RepositoryUnlockRequest>(with(
            unlock("rollback"),
            &[("force", json!(true))],
        ));

        let mismatched_unlock = with(
            unlock("rollback"),
            &[("approval", approval_with_digest(OTHER_DIGEST))],
        );
        let parsed = assert_accept::<RepositoryUnlockRequest>(mismatched_unlock.clone());
        let mismatch = parsed.validate_approval_digest().unwrap_err();
        assert_eq!(mismatch.expected().as_str(), DIGEST);
        assert_eq!(mismatch.observed().as_str(), OTHER_DIGEST);
        assert_eq!(
            RepositoryUnlockRequest::execution_policy_for_json(&mismatched_unlock),
            Some(ExecutionPolicy::JournaledEffect)
        );
    }

    #[test]
    fn commit_has_only_omitted_true_preview_and_false_approved_apply() {
        let values = preview_leaves(commit_base(), "approvedCommitDigest");
        for (index, value) in values.iter().enumerate() {
            let parsed = assert_accept::<RepositoryCommitRequest>(value.clone());
            assert_eq!(
                parsed.request_variant(),
                if index < 2 {
                    RepositoryCommitRequestVariant::Preview
                } else {
                    RepositoryCommitRequestVariant::Apply
                }
            );
            assert_eq!(
                parsed.execution_policy(),
                ExecutionPolicy::PreviewedJournaledEffect
            );
        }

        for invalid in [
            with(values[0].clone(), &[("dryRun", Value::Null)]),
            with(
                values[0].clone(),
                &[("approvedCommitDigest", json!(DIGEST))],
            ),
            without(values[2].clone(), "approvedCommitDigest"),
            with(values[2].clone(), &[("comment", json!("caller text"))]),
            with(values[2].clone(), &[("keepLocked", json!(true))]),
            without(values[0].clone(), "expectedAuthorizedPostMergeFingerprint"),
        ] {
            assert_reject::<RepositoryCommitRequest>(invalid.clone());
            assert!(RepositoryCommitRequest::execution_policy_for_json(&invalid).is_none());
        }
    }

    fn recover_apply() -> Value {
        with(
            mutation(),
            &[
                ("decision", json!("apply")),
                ("expectedRecoveryDigest", json!(DIGEST)),
                ("approval", approval()),
            ],
        )
    }

    fn recover_cancel() -> Value {
        with(
            mutation(),
            &[
                ("decision", json!("cancel")),
                ("expectedRecoveryDigest", json!(DIGEST)),
            ],
        )
    }

    #[test]
    fn recovery_apply_and_cancel_are_separate_closed_decisions_and_policies() {
        let apply = assert_accept::<RepositoryRecoverRequest>(recover_apply());
        assert_eq!(
            apply.request_variant(),
            RepositoryRecoverRequestVariant::RecoverApply
        );
        assert_eq!(apply.execution_policy(), ExecutionPolicy::JournaledEffect);
        assert!(apply.validate_approval_digest().is_ok());

        let cancel = assert_accept::<RepositoryRecoverRequest>(recover_cancel());
        assert_eq!(
            cancel.request_variant(),
            RepositoryRecoverRequestVariant::RecoverCancel
        );
        assert_eq!(cancel.execution_policy(), ExecutionPolicy::LocalJournaled);
        assert!(cancel.validate_approval_digest().is_ok());

        for invalid in [
            without(recover_apply(), "approval"),
            with(recover_cancel(), &[("approval", approval())]),
            with(recover_cancel(), &[("dryRun", json!(false))]),
            with(
                recover_cancel(),
                &[("decision", json!("cancelPendingPlan"))],
            ),
            without(recover_cancel(), "operationId"),
        ] {
            assert_reject::<RepositoryRecoverRequest>(invalid.clone());
            assert!(RepositoryRecoverRequest::execution_policy_for_json(&invalid).is_none());
        }

        let mismatched_apply = with(
            recover_apply(),
            &[("approval", approval_with_digest(OTHER_DIGEST))],
        );
        let parsed = assert_accept::<RepositoryRecoverRequest>(mismatched_apply.clone());
        let mismatch = parsed.validate_approval_digest().unwrap_err();
        assert_eq!(mismatch.expected().as_str(), DIGEST);
        assert_eq!(mismatch.observed().as_str(), OTHER_DIGEST);
        assert_eq!(
            RepositoryRecoverRequest::execution_policy_for_json(&mismatched_apply),
            Some(ExecutionPolicy::JournaledEffect)
        );
    }

    #[test]
    fn all_repository_schemas_are_closed_and_physical_unions_are_exact() {
        assert_closed::<RepositoryStatusRequest>();
        assert_closed::<RepositoryUpdateRequest>();
        assert_closed::<RepositoryPlanLocksRequest>();
        assert_closed::<RepositoryLockRequest>();
        assert_closed::<RepositoryUnlockRequest>();
        assert_closed::<RepositoryCommitRequest>();
        assert_closed::<RepositoryRecoverRequest>();

        assert_exact_one_of::<RepositoryUpdateRequest>(14);
        assert_exact_one_of::<RepositoryCommitRequest>(3);
        assert_exact_one_of::<RepositoryRecoverRequest>(2);
    }

    #[test]
    fn every_repository_policy_selector_fails_closed_on_unknown_json() {
        let malformed = json!({
            "cwd": CWD,
            "taskId": TASK_ID,
            "operationId": OPERATION_ID,
            "unknown": true
        });
        assert!(RepositoryStatusRequest::execution_policy_for_json(&malformed).is_none());
        assert!(RepositoryUpdateRequest::execution_policy_for_json(&malformed).is_none());
        assert!(RepositoryPlanLocksRequest::execution_policy_for_json(&malformed).is_none());
        assert!(RepositoryLockRequest::execution_policy_for_json(&malformed).is_none());
        assert!(RepositoryUnlockRequest::execution_policy_for_json(&malformed).is_none());
        assert!(RepositoryCommitRequest::execution_policy_for_json(&malformed).is_none());
        assert!(RepositoryRecoverRequest::execution_policy_for_json(&malformed).is_none());
    }
}
