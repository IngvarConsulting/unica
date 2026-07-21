use super::{execution_policy_for_json, request_one_of_schema, FalseLiteral, TrueLiteral};
use crate::domain::branched_development::contracts::artifacts::AcceptedArtifactKind;
use crate::domain::branched_development::contracts::scalars::OriginalProjectCwd;
use crate::domain::branched_development::{
    ExecutionPolicy, OperationId, Sha256Digest, TaskId, UnicaId,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct DeliveryInspectRequest {
    cwd: OriginalProjectCwd,
    task_id: TaskId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DeliveryInspectRequestVariant {
    Inspect,
}

impl DeliveryInspectRequest {
    pub(crate) const fn request_variant(&self) -> DeliveryInspectRequestVariant {
        DeliveryInspectRequestVariant::Inspect
    }

    pub(crate) const fn execution_policy(&self) -> ExecutionPolicy {
        ExecutionPolicy::ReadOnly
    }

    pub(crate) fn execution_policy_for_json(value: &Value) -> Option<ExecutionPolicy> {
        execution_policy_for_json::<Self>(value, Self::execution_policy)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
enum BaselineDistributionRole {
    #[serde(rename = "baselineDistribution")]
    BaselineDistribution,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
enum RefreshDistributionRole {
    #[serde(rename = "refreshDistribution")]
    RefreshDistribution,
}

macro_rules! create_leaf {
    ($name:ident, $role:ty $(, $field:ident : $field_type:ty )* $(,)?) => {
        #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        pub(crate) struct $name {
            cwd: OriginalProjectCwd,
            task_id: TaskId,
            operation_id: OperationId,
            role: $role,
            inspection_digest: Sha256Digest,
            $($field: $field_type,)*
        }
    };
}

create_leaf!(CreateBaselinePreviewOmitted, BaselineDistributionRole);
create_leaf!(CreateBaselinePreviewExplicit, BaselineDistributionRole, dry_run: TrueLiteral);
create_leaf!(
    CreateBaselineApply,
    BaselineDistributionRole,
    dry_run: FalseLiteral,
    approved_preview_digest: Sha256Digest,
);
create_leaf!(CreateRefreshPreviewOmitted, RefreshDistributionRole);
create_leaf!(CreateRefreshPreviewExplicit, RefreshDistributionRole, dry_run: TrueLiteral);
create_leaf!(
    CreateRefreshApply,
    RefreshDistributionRole,
    dry_run: FalseLiteral,
    approved_preview_digest: Sha256Digest,
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub(crate) enum DeliveryCreateRequest {
    BaselinePreviewOmitted(CreateBaselinePreviewOmitted),
    BaselinePreviewExplicit(CreateBaselinePreviewExplicit),
    BaselineApply(CreateBaselineApply),
    RefreshPreviewOmitted(CreateRefreshPreviewOmitted),
    RefreshPreviewExplicit(CreateRefreshPreviewExplicit),
    RefreshApply(CreateRefreshApply),
}

request_one_of_schema!(
    DeliveryCreateRequest,
    "DeliveryCreateRequest",
    [
        CreateBaselinePreviewOmitted,
        CreateBaselinePreviewExplicit,
        CreateBaselineApply,
        CreateRefreshPreviewOmitted,
        CreateRefreshPreviewExplicit,
        CreateRefreshApply,
    ]
);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DeliveryCreateRequestVariant {
    BaselineDistributionPreview,
    BaselineDistributionApply,
    RefreshDistributionPreview,
    RefreshDistributionApply,
}

impl DeliveryCreateRequest {
    pub(crate) const fn request_variant(&self) -> DeliveryCreateRequestVariant {
        match self {
            Self::BaselinePreviewOmitted(_) | Self::BaselinePreviewExplicit(_) => {
                DeliveryCreateRequestVariant::BaselineDistributionPreview
            }
            Self::BaselineApply(_) => DeliveryCreateRequestVariant::BaselineDistributionApply,
            Self::RefreshPreviewOmitted(_) | Self::RefreshPreviewExplicit(_) => {
                DeliveryCreateRequestVariant::RefreshDistributionPreview
            }
            Self::RefreshApply(_) => DeliveryCreateRequestVariant::RefreshDistributionApply,
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
pub(crate) struct VerifyWithoutExpectedKind {
    cwd: OriginalProjectCwd,
    task_id: TaskId,
    operation_id: OperationId,
    artifact_id: UnicaId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct VerifyWithExpectedKind {
    cwd: OriginalProjectCwd,
    task_id: TaskId,
    operation_id: OperationId,
    artifact_id: UnicaId,
    expected_kind: AcceptedArtifactKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub(crate) enum DeliveryVerifyRequest {
    WithoutExpectedKind(VerifyWithoutExpectedKind),
    WithExpectedKind(VerifyWithExpectedKind),
}

request_one_of_schema!(
    DeliveryVerifyRequest,
    "DeliveryVerifyRequest",
    [VerifyWithoutExpectedKind, VerifyWithExpectedKind]
);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DeliveryVerifyRequestVariant {
    Verify,
}

impl DeliveryVerifyRequest {
    pub(crate) const fn request_variant(&self) -> DeliveryVerifyRequestVariant {
        DeliveryVerifyRequestVariant::Verify
    }

    pub(crate) const fn execution_policy(&self) -> ExecutionPolicy {
        ExecutionPolicy::Contained
    }

    pub(crate) fn execution_policy_for_json(value: &Value) -> Option<ExecutionPolicy> {
        execution_policy_for_json::<Self>(value, Self::execution_policy)
    }
}

macro_rules! deploy_leaf {
    ($name:ident $(, $field:ident : $field_type:ty )* $(,)?) => {
        #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        pub(crate) struct $name {
            cwd: OriginalProjectCwd,
            task_id: TaskId,
            operation_id: OperationId,
            distribution_id: UnicaId,
            $($field: $field_type,)*
        }
    };
}

deploy_leaf!(DeployPreviewOmitted);
deploy_leaf!(DeployPreviewExplicit, dry_run: TrueLiteral);
deploy_leaf!(
    DeployApply,
    dry_run: FalseLiteral,
    approved_preview_digest: Sha256Digest,
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub(crate) enum DeliveryDeployRequest {
    PreviewOmitted(DeployPreviewOmitted),
    PreviewExplicit(DeployPreviewExplicit),
    Apply(DeployApply),
}

request_one_of_schema!(
    DeliveryDeployRequest,
    "DeliveryDeployRequest",
    [DeployPreviewOmitted, DeployPreviewExplicit, DeployApply]
);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DeliveryDeployRequestVariant {
    Preview,
    Apply,
}

impl DeliveryDeployRequest {
    pub(crate) const fn request_variant(&self) -> DeliveryDeployRequestVariant {
        match self {
            Self::PreviewOmitted(_) | Self::PreviewExplicit(_) => {
                DeliveryDeployRequestVariant::Preview
            }
            Self::Apply(_) => DeliveryDeployRequestVariant::Apply,
        }
    }

    pub(crate) const fn execution_policy(&self) -> ExecutionPolicy {
        ExecutionPolicy::PreviewedJournaledEffect
    }

    pub(crate) fn execution_policy_for_json(value: &Value) -> Option<ExecutionPolicy> {
        execution_policy_for_json::<Self>(value, Self::execution_policy)
    }
}
