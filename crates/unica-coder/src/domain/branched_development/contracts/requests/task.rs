use super::{execution_policy_for_json, request_one_of_schema, FalseLiteral, TrueLiteral};
use crate::domain::branched_development::contracts::scalars::{
    LocalProfileName, OriginalProjectCwd, Reason, TaskSummary,
};
use crate::domain::branched_development::{
    ExecutionPolicy, OperationId, Sha256Digest, TaskId, UnicaId,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CommonTaskRequest {
    cwd: OriginalProjectCwd,
    task_id: TaskId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CommonMutationRequest {
    cwd: OriginalProjectCwd,
    task_id: TaskId,
    operation_id: OperationId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct BranchedStartRequest {
    cwd: OriginalProjectCwd,
    task_id: TaskId,
    operation_id: OperationId,
    profile: LocalProfileName,
    task_summary: TaskSummary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BranchedStartRequestVariant {
    Start,
}

impl BranchedStartRequest {
    pub(crate) const fn request_variant(&self) -> BranchedStartRequestVariant {
        BranchedStartRequestVariant::Start
    }

    pub(crate) const fn execution_policy(&self) -> ExecutionPolicy {
        ExecutionPolicy::LocalJournaled
    }

    pub(crate) fn execution_policy_for_json(value: &Value) -> Option<ExecutionPolicy> {
        execution_policy_for_json::<Self>(value, Self::execution_policy)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct BranchedStatusRequest {
    cwd: OriginalProjectCwd,
    task_id: TaskId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BranchedStatusRequestVariant {
    Status,
}

impl BranchedStatusRequest {
    pub(crate) const fn request_variant(&self) -> BranchedStatusRequestVariant {
        BranchedStatusRequestVariant::Status
    }

    pub(crate) const fn execution_policy(&self) -> ExecutionPolicy {
        ExecutionPolicy::ReadOnly
    }

    pub(crate) fn execution_policy_for_json(value: &Value) -> Option<ExecutionPolicy> {
        execution_policy_for_json::<Self>(value, Self::execution_policy)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
enum SuccessOutcome {
    #[serde(rename = "success")]
    Success,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
enum AbandonedOutcome {
    #[serde(rename = "abandoned")]
    Abandoned,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ArchiveSuccessPreviewOmitted {
    cwd: OriginalProjectCwd,
    task_id: TaskId,
    operation_id: OperationId,
    outcome: SuccessOutcome,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ArchiveSuccessPreviewExplicit {
    cwd: OriginalProjectCwd,
    task_id: TaskId,
    operation_id: OperationId,
    outcome: SuccessOutcome,
    dry_run: TrueLiteral,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ArchiveSuccessApply {
    cwd: OriginalProjectCwd,
    task_id: TaskId,
    operation_id: OperationId,
    outcome: SuccessOutcome,
    dry_run: FalseLiteral,
    approved_preview_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ArchiveAbandonedPreviewOmitted {
    cwd: OriginalProjectCwd,
    task_id: TaskId,
    operation_id: OperationId,
    outcome: AbandonedOutcome,
    reason: Reason,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ArchiveAbandonedPreviewExplicit {
    cwd: OriginalProjectCwd,
    task_id: TaskId,
    operation_id: OperationId,
    outcome: AbandonedOutcome,
    reason: Reason,
    dry_run: TrueLiteral,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ArchiveAbandonedApply {
    cwd: OriginalProjectCwd,
    task_id: TaskId,
    operation_id: OperationId,
    outcome: AbandonedOutcome,
    reason: Reason,
    dry_run: FalseLiteral,
    approved_preview_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub(crate) enum BranchedArchiveRequest {
    SuccessPreviewOmitted(ArchiveSuccessPreviewOmitted),
    SuccessPreviewExplicit(ArchiveSuccessPreviewExplicit),
    SuccessApply(ArchiveSuccessApply),
    AbandonedPreviewOmitted(ArchiveAbandonedPreviewOmitted),
    AbandonedPreviewExplicit(ArchiveAbandonedPreviewExplicit),
    AbandonedApply(ArchiveAbandonedApply),
}

request_one_of_schema!(
    BranchedArchiveRequest,
    "BranchedArchiveRequest",
    [
        ArchiveSuccessPreviewOmitted,
        ArchiveSuccessPreviewExplicit,
        ArchiveSuccessApply,
        ArchiveAbandonedPreviewOmitted,
        ArchiveAbandonedPreviewExplicit,
        ArchiveAbandonedApply,
    ]
);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BranchedArchiveRequestVariant {
    SuccessPreview,
    SuccessApply,
    AbandonedPreview,
    AbandonedApply,
}

impl BranchedArchiveRequest {
    pub(crate) const fn request_variant(&self) -> BranchedArchiveRequestVariant {
        match self {
            Self::SuccessPreviewOmitted(_) | Self::SuccessPreviewExplicit(_) => {
                BranchedArchiveRequestVariant::SuccessPreview
            }
            Self::SuccessApply(_) => BranchedArchiveRequestVariant::SuccessApply,
            Self::AbandonedPreviewOmitted(_) | Self::AbandonedPreviewExplicit(_) => {
                BranchedArchiveRequestVariant::AbandonedPreview
            }
            Self::AbandonedApply(_) => BranchedArchiveRequestVariant::AbandonedApply,
        }
    }

    pub(crate) const fn execution_policy(&self) -> ExecutionPolicy {
        ExecutionPolicy::PreviewedJournaledEffect
    }

    pub(crate) fn execution_policy_for_json(value: &Value) -> Option<ExecutionPolicy> {
        execution_policy_for_json::<Self>(value, Self::execution_policy)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CleanupPreviewOmitted {
    cwd: OriginalProjectCwd,
    task_id: TaskId,
    operation_id: OperationId,
    archive_id: UnicaId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CleanupPreviewExplicit {
    cwd: OriginalProjectCwd,
    task_id: TaskId,
    operation_id: OperationId,
    archive_id: UnicaId,
    dry_run: TrueLiteral,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CleanupApply {
    cwd: OriginalProjectCwd,
    task_id: TaskId,
    operation_id: OperationId,
    archive_id: UnicaId,
    dry_run: FalseLiteral,
    approved_preview_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub(crate) enum BranchedCleanupRequest {
    PreviewOmitted(CleanupPreviewOmitted),
    PreviewExplicit(CleanupPreviewExplicit),
    Apply(CleanupApply),
}

request_one_of_schema!(
    BranchedCleanupRequest,
    "BranchedCleanupRequest",
    [CleanupPreviewOmitted, CleanupPreviewExplicit, CleanupApply]
);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BranchedCleanupRequestVariant {
    Preview,
    Apply,
}

impl BranchedCleanupRequest {
    pub(crate) const fn request_variant(&self) -> BranchedCleanupRequestVariant {
        match self {
            Self::PreviewOmitted(_) | Self::PreviewExplicit(_) => {
                BranchedCleanupRequestVariant::Preview
            }
            Self::Apply(_) => BranchedCleanupRequestVariant::Apply,
        }
    }

    pub(crate) const fn execution_policy(&self) -> ExecutionPolicy {
        ExecutionPolicy::PreviewedJournaledEffect
    }

    pub(crate) fn execution_policy_for_json(value: &Value) -> Option<ExecutionPolicy> {
        execution_policy_for_json::<Self>(value, Self::execution_policy)
    }
}
