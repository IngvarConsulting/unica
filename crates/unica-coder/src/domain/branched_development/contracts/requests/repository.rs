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
pub(crate) enum SupportCancellationReason {
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

/// Borrowed request lineage for a routine preview. Both the omitted and
/// explicit `dryRun: true` wire leaves project to this same semantic token.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ValidatedRoutineUpdatePreviewRequest<'a> {
    cwd: &'a OriginalProjectCwd,
    task_id: &'a TaskId,
    operation_id: &'a OperationId,
    expected_status_digest: &'a Sha256Digest,
}

impl ValidatedRoutineUpdatePreviewRequest<'_> {
    pub(crate) const fn cwd(&self) -> &OriginalProjectCwd {
        self.cwd
    }

    pub(crate) const fn task_id(&self) -> &TaskId {
        self.task_id
    }

    pub(crate) const fn operation_id(&self) -> &OperationId {
        self.operation_id
    }

    pub(crate) const fn expected_status_digest(&self) -> &Sha256Digest {
        self.expected_status_digest
    }
}

/// Borrowed request lineage for a support-prerequisite arming preview.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ValidatedArmPreviewRequest<'a> {
    cwd: &'a OriginalProjectCwd,
    task_id: &'a TaskId,
    expected_status_digest: &'a Sha256Digest,
    support_action_id: &'a UnicaId,
    expected_support_action_digest: &'a Sha256Digest,
}

impl ValidatedArmPreviewRequest<'_> {
    pub(crate) const fn cwd(&self) -> &OriginalProjectCwd {
        self.cwd
    }

    pub(crate) const fn task_id(&self) -> &TaskId {
        self.task_id
    }

    pub(crate) const fn expected_status_digest(&self) -> &Sha256Digest {
        self.expected_status_digest
    }

    pub(crate) const fn support_action_id(&self) -> &UnicaId {
        self.support_action_id
    }

    pub(crate) const fn expected_support_action_digest(&self) -> &Sha256Digest {
        self.expected_support_action_digest
    }
}

/// Borrowed request lineage for a support-prerequisite reconciliation preview.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ValidatedPrerequisiteUpdatePreviewRequest<'a> {
    cwd: &'a OriginalProjectCwd,
    task_id: &'a TaskId,
    operation_id: &'a OperationId,
    expected_status_digest: &'a Sha256Digest,
    support_action_id: &'a UnicaId,
    expected_support_action_digest: &'a Sha256Digest,
    expected_arming_receipt_id: &'a UnicaId,
    expected_arming_receipt_digest: &'a Sha256Digest,
}

impl ValidatedPrerequisiteUpdatePreviewRequest<'_> {
    pub(crate) const fn cwd(&self) -> &OriginalProjectCwd {
        self.cwd
    }

    pub(crate) const fn task_id(&self) -> &TaskId {
        self.task_id
    }

    pub(crate) const fn operation_id(&self) -> &OperationId {
        self.operation_id
    }

    pub(crate) const fn expected_status_digest(&self) -> &Sha256Digest {
        self.expected_status_digest
    }

    pub(crate) const fn support_action_id(&self) -> &UnicaId {
        self.support_action_id
    }

    pub(crate) const fn expected_support_action_digest(&self) -> &Sha256Digest {
        self.expected_support_action_digest
    }

    pub(crate) const fn expected_arming_receipt_id(&self) -> &UnicaId {
        self.expected_arming_receipt_id
    }

    pub(crate) const fn expected_arming_receipt_digest(&self) -> &Sha256Digest {
        self.expected_arming_receipt_digest
    }
}

/// Borrowed request lineage for an awaiting or armed cancellation preview.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ValidatedCancellationPreviewRequest<'a> {
    cwd: &'a OriginalProjectCwd,
    task_id: &'a TaskId,
    operation_id: &'a OperationId,
    expected_status_digest: &'a Sha256Digest,
    support_action_id: &'a UnicaId,
    expected_support_action_digest: &'a Sha256Digest,
    reason: SupportCancellationReason,
    arming: ValidatedCancellationArming<'a>,
}

impl ValidatedCancellationPreviewRequest<'_> {
    pub(crate) const fn cwd(&self) -> &OriginalProjectCwd {
        self.cwd
    }

    pub(crate) const fn task_id(&self) -> &TaskId {
        self.task_id
    }

    pub(crate) const fn operation_id(&self) -> &OperationId {
        self.operation_id
    }

    pub(crate) const fn expected_status_digest(&self) -> &Sha256Digest {
        self.expected_status_digest
    }

    pub(crate) const fn support_action_id(&self) -> &UnicaId {
        self.support_action_id
    }

    pub(crate) const fn expected_support_action_digest(&self) -> &Sha256Digest {
        self.expected_support_action_digest
    }

    pub(crate) const fn reason(&self) -> SupportCancellationReason {
        self.reason
    }

    pub(crate) const fn arming(&self) -> &ValidatedCancellationArming<'_> {
        &self.arming
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UpdatePreviewValidationError {
    Routine,
    Arm,
    Prerequisite,
    Cancellation,
}

impl std::fmt::Display for UpdatePreviewValidationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let message = match self {
            Self::Routine => "request is not a routine update preview",
            Self::Arm => "request is not a support arming preview",
            Self::Prerequisite => "request is not a support-prerequisite update preview",
            Self::Cancellation => "request is not a support cancellation preview",
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for UpdatePreviewValidationError {}

/// Borrowed, non-wire proof that an arming apply approved one exact preview
/// and still carries the action selectors that preview authorized.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ValidatedArmApplyRequest<'a> {
    cwd: &'a OriginalProjectCwd,
    task_id: &'a TaskId,
    operation_id: &'a OperationId,
    expected_status_digest: &'a Sha256Digest,
    support_action_id: &'a UnicaId,
    expected_support_action_digest: &'a Sha256Digest,
    approved_arming_digest: &'a Sha256Digest,
}

impl ValidatedArmApplyRequest<'_> {
    pub(crate) const fn cwd(&self) -> &OriginalProjectCwd {
        self.cwd
    }

    pub(crate) const fn task_id(&self) -> &TaskId {
        self.task_id
    }

    pub(crate) const fn operation_id(&self) -> &OperationId {
        self.operation_id
    }

    pub(crate) const fn expected_status_digest(&self) -> &Sha256Digest {
        self.expected_status_digest
    }

    pub(crate) const fn support_action_id(&self) -> &UnicaId {
        self.support_action_id
    }

    pub(crate) const fn expected_support_action_digest(&self) -> &Sha256Digest {
        self.expected_support_action_digest
    }

    pub(crate) const fn approved_arming_digest(&self) -> &Sha256Digest {
        self.approved_arming_digest
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ArmApprovalValidationError {
    NotArmApply,
    DigestMismatch(DigestApprovalMismatch),
}

impl std::fmt::Display for ArmApprovalValidationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotArmApply => formatter.write_str("request is not a support arming apply"),
            Self::DigestMismatch(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for ArmApprovalValidationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::NotArmApply => None,
            Self::DigestMismatch(error) => Some(error),
        }
    }
}

/// Borrowed, non-wire proof that a routine update apply approved one exact
/// preview digest. The selectors remain attached to the approval so a result
/// producer cannot splice an approval into another task or operation.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ValidatedRoutineUpdateApplyRequest<'a> {
    cwd: &'a OriginalProjectCwd,
    task_id: &'a TaskId,
    operation_id: &'a OperationId,
    expected_status_digest: &'a Sha256Digest,
    approved_update_digest: &'a Sha256Digest,
}

impl ValidatedRoutineUpdateApplyRequest<'_> {
    pub(crate) const fn cwd(&self) -> &OriginalProjectCwd {
        self.cwd
    }

    pub(crate) const fn task_id(&self) -> &TaskId {
        self.task_id
    }

    pub(crate) const fn operation_id(&self) -> &OperationId {
        self.operation_id
    }

    pub(crate) const fn expected_status_digest(&self) -> &Sha256Digest {
        self.expected_status_digest
    }

    pub(crate) const fn approved_update_digest(&self) -> &Sha256Digest {
        self.approved_update_digest
    }
}

/// Borrowed, non-wire proof that a support-prerequisite update apply approved
/// one exact preview while preserving its action and arming lineage.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ValidatedPrerequisiteUpdateApplyRequest<'a> {
    cwd: &'a OriginalProjectCwd,
    task_id: &'a TaskId,
    operation_id: &'a OperationId,
    expected_status_digest: &'a Sha256Digest,
    support_action_id: &'a UnicaId,
    expected_support_action_digest: &'a Sha256Digest,
    expected_arming_receipt_id: &'a UnicaId,
    expected_arming_receipt_digest: &'a Sha256Digest,
    approved_update_digest: &'a Sha256Digest,
}

impl ValidatedPrerequisiteUpdateApplyRequest<'_> {
    pub(crate) const fn cwd(&self) -> &OriginalProjectCwd {
        self.cwd
    }

    pub(crate) const fn task_id(&self) -> &TaskId {
        self.task_id
    }

    pub(crate) const fn operation_id(&self) -> &OperationId {
        self.operation_id
    }

    pub(crate) const fn expected_status_digest(&self) -> &Sha256Digest {
        self.expected_status_digest
    }

    pub(crate) const fn support_action_id(&self) -> &UnicaId {
        self.support_action_id
    }

    pub(crate) const fn expected_support_action_digest(&self) -> &Sha256Digest {
        self.expected_support_action_digest
    }

    pub(crate) const fn expected_arming_receipt_id(&self) -> &UnicaId {
        self.expected_arming_receipt_id
    }

    pub(crate) const fn expected_arming_receipt_digest(&self) -> &Sha256Digest {
        self.expected_arming_receipt_digest
    }

    pub(crate) const fn approved_update_digest(&self) -> &Sha256Digest {
        self.approved_update_digest
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum UpdateApprovalValidationError {
    NotRoutineApply,
    NotPrerequisiteApply,
    DigestMismatch(DigestApprovalMismatch),
}

impl std::fmt::Display for UpdateApprovalValidationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotRoutineApply => formatter.write_str("request is not a routine update apply"),
            Self::NotPrerequisiteApply => {
                formatter.write_str("request is not a support-prerequisite update apply")
            }
            Self::DigestMismatch(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for UpdateApprovalValidationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::NotRoutineApply | Self::NotPrerequisiteApply => None,
            Self::DigestMismatch(error) => Some(error),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ValidatedCancellationArming<'a> {
    Awaiting,
    Armed {
        expected_arming_receipt_id: &'a UnicaId,
        expected_arming_receipt_digest: &'a Sha256Digest,
    },
}

/// Borrowed, non-wire proof that an exact cancellation-apply request approved
/// the immutable preview digest supplied by the coordinator. Keeping all
/// selectors in one value prevents a later recovery producer from combining
/// the approval of one operation with another action, arming window, or reason.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ValidatedCancellationApplyRequest<'a> {
    cwd: &'a OriginalProjectCwd,
    task_id: &'a TaskId,
    operation_id: &'a OperationId,
    expected_status_digest: &'a Sha256Digest,
    support_action_id: &'a UnicaId,
    expected_support_action_digest: &'a Sha256Digest,
    reason: SupportCancellationReason,
    arming: ValidatedCancellationArming<'a>,
    approved_cancellation_digest: &'a Sha256Digest,
}

impl ValidatedCancellationApplyRequest<'_> {
    pub(crate) const fn cwd(&self) -> &OriginalProjectCwd {
        self.cwd
    }

    pub(crate) const fn task_id(&self) -> &TaskId {
        self.task_id
    }

    pub(crate) const fn operation_id(&self) -> &OperationId {
        self.operation_id
    }

    pub(crate) const fn expected_status_digest(&self) -> &Sha256Digest {
        self.expected_status_digest
    }

    pub(crate) const fn support_action_id(&self) -> &UnicaId {
        self.support_action_id
    }

    pub(crate) const fn expected_support_action_digest(&self) -> &Sha256Digest {
        self.expected_support_action_digest
    }

    pub(crate) const fn reason(&self) -> SupportCancellationReason {
        self.reason
    }

    pub(crate) const fn arming(&self) -> &ValidatedCancellationArming<'_> {
        &self.arming
    }

    pub(crate) const fn approved_cancellation_digest(&self) -> &Sha256Digest {
        self.approved_cancellation_digest
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum CancellationApprovalValidationError {
    NotCancellationApply,
    DigestMismatch(DigestApprovalMismatch),
}

impl std::fmt::Display for CancellationApprovalValidationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotCancellationApply => {
                formatter.write_str("request is not a support cancellation apply")
            }
            Self::DigestMismatch(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for CancellationApprovalValidationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::NotCancellationApply => None,
            Self::DigestMismatch(error) => Some(error),
        }
    }
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

    pub(crate) fn validate_routine_update_preview_context(
        &self,
    ) -> Result<ValidatedRoutineUpdatePreviewRequest<'_>, UpdatePreviewValidationError> {
        let (cwd, task_id, operation_id, expected_status_digest) = match self {
            Self::RoutinePreviewOmitted(request) => (
                &request.cwd,
                &request.task_id,
                &request.operation_id,
                &request.expected_status_digest,
            ),
            Self::RoutinePreviewExplicit(request) => (
                &request.cwd,
                &request.task_id,
                &request.operation_id,
                &request.expected_status_digest,
            ),
            _ => return Err(UpdatePreviewValidationError::Routine),
        };
        Ok(ValidatedRoutineUpdatePreviewRequest {
            cwd,
            task_id,
            operation_id,
            expected_status_digest,
        })
    }

    pub(crate) fn validate_arm_preview_context(
        &self,
    ) -> Result<ValidatedArmPreviewRequest<'_>, UpdatePreviewValidationError> {
        let Self::ArmPreview(request) = self else {
            return Err(UpdatePreviewValidationError::Arm);
        };
        Ok(ValidatedArmPreviewRequest {
            cwd: &request.cwd,
            task_id: &request.task_id,
            expected_status_digest: &request.expected_status_digest,
            support_action_id: &request.support_action_id,
            expected_support_action_digest: &request.expected_support_action_digest,
        })
    }

    pub(crate) fn validate_prerequisite_update_preview_context(
        &self,
    ) -> Result<ValidatedPrerequisiteUpdatePreviewRequest<'_>, UpdatePreviewValidationError> {
        let (
            cwd,
            task_id,
            operation_id,
            expected_status_digest,
            support_action_id,
            expected_support_action_digest,
            expected_arming_receipt_id,
            expected_arming_receipt_digest,
        ) = match self {
            Self::PrerequisitePreviewOmitted(request) => (
                &request.cwd,
                &request.task_id,
                &request.operation_id,
                &request.expected_status_digest,
                &request.support_action_id,
                &request.expected_support_action_digest,
                &request.expected_arming_receipt_id,
                &request.expected_arming_receipt_digest,
            ),
            Self::PrerequisitePreviewExplicit(request) => (
                &request.cwd,
                &request.task_id,
                &request.operation_id,
                &request.expected_status_digest,
                &request.support_action_id,
                &request.expected_support_action_digest,
                &request.expected_arming_receipt_id,
                &request.expected_arming_receipt_digest,
            ),
            _ => return Err(UpdatePreviewValidationError::Prerequisite),
        };
        Ok(ValidatedPrerequisiteUpdatePreviewRequest {
            cwd,
            task_id,
            operation_id,
            expected_status_digest,
            support_action_id,
            expected_support_action_digest,
            expected_arming_receipt_id,
            expected_arming_receipt_digest,
        })
    }

    pub(crate) fn validate_cancellation_preview_context(
        &self,
    ) -> Result<ValidatedCancellationPreviewRequest<'_>, UpdatePreviewValidationError> {
        let (
            cwd,
            task_id,
            operation_id,
            expected_status_digest,
            support_action_id,
            expected_support_action_digest,
            reason,
            arming,
        ) = match self {
            Self::CancellationAwaitingPreviewOmitted(request) => (
                &request.cwd,
                &request.task_id,
                &request.operation_id,
                &request.expected_status_digest,
                &request.support_action_id,
                &request.expected_support_action_digest,
                request.reason,
                ValidatedCancellationArming::Awaiting,
            ),
            Self::CancellationAwaitingPreviewExplicit(request) => (
                &request.cwd,
                &request.task_id,
                &request.operation_id,
                &request.expected_status_digest,
                &request.support_action_id,
                &request.expected_support_action_digest,
                request.reason,
                ValidatedCancellationArming::Awaiting,
            ),
            Self::CancellationArmedPreviewOmitted(request) => (
                &request.cwd,
                &request.task_id,
                &request.operation_id,
                &request.expected_status_digest,
                &request.support_action_id,
                &request.expected_support_action_digest,
                request.reason,
                ValidatedCancellationArming::Armed {
                    expected_arming_receipt_id: &request.expected_arming_receipt_id,
                    expected_arming_receipt_digest: &request.expected_arming_receipt_digest,
                },
            ),
            Self::CancellationArmedPreviewExplicit(request) => (
                &request.cwd,
                &request.task_id,
                &request.operation_id,
                &request.expected_status_digest,
                &request.support_action_id,
                &request.expected_support_action_digest,
                request.reason,
                ValidatedCancellationArming::Armed {
                    expected_arming_receipt_id: &request.expected_arming_receipt_id,
                    expected_arming_receipt_digest: &request.expected_arming_receipt_digest,
                },
            ),
            _ => return Err(UpdatePreviewValidationError::Cancellation),
        };
        Ok(ValidatedCancellationPreviewRequest {
            cwd,
            task_id,
            operation_id,
            expected_status_digest,
            support_action_id,
            expected_support_action_digest,
            reason,
            arming,
        })
    }

    pub(crate) fn validate_arm_approval(
        &self,
        expected_arming_digest: &Sha256Digest,
    ) -> Result<ValidatedArmApplyRequest<'_>, ArmApprovalValidationError> {
        let Self::ArmApply(request) = self else {
            return Err(ArmApprovalValidationError::NotArmApply);
        };
        if request.approved_arming_digest != *expected_arming_digest {
            return Err(ArmApprovalValidationError::DigestMismatch(
                DigestApprovalMismatch {
                    expected: expected_arming_digest.clone(),
                    observed: request.approved_arming_digest.clone(),
                },
            ));
        }
        Ok(ValidatedArmApplyRequest {
            cwd: &request.cwd,
            task_id: &request.task_id,
            operation_id: &request.operation_id,
            expected_status_digest: &request.expected_status_digest,
            support_action_id: &request.support_action_id,
            expected_support_action_digest: &request.expected_support_action_digest,
            approved_arming_digest: &request.approved_arming_digest,
        })
    }

    pub(crate) fn validate_routine_update_approval(
        &self,
        expected_update_digest: &Sha256Digest,
    ) -> Result<ValidatedRoutineUpdateApplyRequest<'_>, UpdateApprovalValidationError> {
        let Self::RoutineApply(request) = self else {
            return Err(UpdateApprovalValidationError::NotRoutineApply);
        };
        if request.approved_update_digest != *expected_update_digest {
            return Err(UpdateApprovalValidationError::DigestMismatch(
                DigestApprovalMismatch {
                    expected: expected_update_digest.clone(),
                    observed: request.approved_update_digest.clone(),
                },
            ));
        }
        Ok(ValidatedRoutineUpdateApplyRequest {
            cwd: &request.cwd,
            task_id: &request.task_id,
            operation_id: &request.operation_id,
            expected_status_digest: &request.expected_status_digest,
            approved_update_digest: &request.approved_update_digest,
        })
    }

    pub(crate) fn validate_prerequisite_update_approval(
        &self,
        expected_update_digest: &Sha256Digest,
    ) -> Result<ValidatedPrerequisiteUpdateApplyRequest<'_>, UpdateApprovalValidationError> {
        let Self::PrerequisiteApply(request) = self else {
            return Err(UpdateApprovalValidationError::NotPrerequisiteApply);
        };
        if request.approved_update_digest != *expected_update_digest {
            return Err(UpdateApprovalValidationError::DigestMismatch(
                DigestApprovalMismatch {
                    expected: expected_update_digest.clone(),
                    observed: request.approved_update_digest.clone(),
                },
            ));
        }
        Ok(ValidatedPrerequisiteUpdateApplyRequest {
            cwd: &request.cwd,
            task_id: &request.task_id,
            operation_id: &request.operation_id,
            expected_status_digest: &request.expected_status_digest,
            support_action_id: &request.support_action_id,
            expected_support_action_digest: &request.expected_support_action_digest,
            expected_arming_receipt_id: &request.expected_arming_receipt_id,
            expected_arming_receipt_digest: &request.expected_arming_receipt_digest,
            approved_update_digest: &request.approved_update_digest,
        })
    }

    pub(crate) fn validate_cancellation_approval(
        &self,
        expected_cancellation_digest: &Sha256Digest,
    ) -> Result<ValidatedCancellationApplyRequest<'_>, CancellationApprovalValidationError> {
        let (
            cwd,
            task_id,
            operation_id,
            expected_status_digest,
            support_action_id,
            expected_support_action_digest,
            reason,
            arming,
            approved_cancellation_digest,
        ) = match self {
            Self::CancellationAwaitingApply(request) => (
                &request.cwd,
                &request.task_id,
                &request.operation_id,
                &request.expected_status_digest,
                &request.support_action_id,
                &request.expected_support_action_digest,
                request.reason,
                ValidatedCancellationArming::Awaiting,
                &request.approved_cancellation_digest,
            ),
            Self::CancellationArmedApply(request) => (
                &request.cwd,
                &request.task_id,
                &request.operation_id,
                &request.expected_status_digest,
                &request.support_action_id,
                &request.expected_support_action_digest,
                request.reason,
                ValidatedCancellationArming::Armed {
                    expected_arming_receipt_id: &request.expected_arming_receipt_id,
                    expected_arming_receipt_digest: &request.expected_arming_receipt_digest,
                },
                &request.approved_cancellation_digest,
            ),
            Self::RoutinePreviewOmitted(_)
            | Self::RoutinePreviewExplicit(_)
            | Self::RoutineApply(_)
            | Self::ArmPreview(_)
            | Self::ArmApply(_)
            | Self::PrerequisitePreviewOmitted(_)
            | Self::PrerequisitePreviewExplicit(_)
            | Self::PrerequisiteApply(_)
            | Self::CancellationAwaitingPreviewOmitted(_)
            | Self::CancellationAwaitingPreviewExplicit(_)
            | Self::CancellationArmedPreviewOmitted(_)
            | Self::CancellationArmedPreviewExplicit(_) => {
                return Err(CancellationApprovalValidationError::NotCancellationApply);
            }
        };
        if approved_cancellation_digest != expected_cancellation_digest {
            return Err(CancellationApprovalValidationError::DigestMismatch(
                DigestApprovalMismatch {
                    expected: expected_cancellation_digest.clone(),
                    observed: approved_cancellation_digest.clone(),
                },
            ));
        }
        Ok(ValidatedCancellationApplyRequest {
            cwd,
            task_id,
            operation_id,
            expected_status_digest,
            support_action_id,
            expected_support_action_digest,
            reason,
            arming,
            approved_cancellation_digest,
        })
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

/// Owned, non-wire proof that the exact physical commit request was one of the
/// two preview leaves. Keeping the original enum preserves omitted versus
/// explicit `dryRun: true` together with every selector used by the preview.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ValidatedRepositoryCommitPreviewRequest {
    request: RepositoryCommitRequest,
}

/// Owned, non-wire proof that the exact physical commit request was the apply
/// leaf. Lineage approval happens only when this value is consumed together
/// with the preview authority.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ValidatedRepositoryCommitApplyRequest {
    request: RepositoryCommitRequest,
}

/// A consuming variant-validation failure. The original wire request remains
/// owned so callers can report or recover without reconstructing it.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct RepositoryCommitRequestValidationFailure {
    request: RepositoryCommitRequest,
    expected: RepositoryCommitRequestVariant,
}

impl RepositoryCommitRequestValidationFailure {
    pub(crate) const fn expected(&self) -> RepositoryCommitRequestVariant {
        self.expected
    }

    pub(crate) fn into_request(self: Box<Self>) -> RepositoryCommitRequest {
        self.request
    }
}

macro_rules! validated_commit_preview_accessor {
    ($name:ident, $field:ident, $type:ty) => {
        pub(crate) fn $name(&self) -> &$type {
            match &self.request {
                RepositoryCommitRequest::PreviewOmitted(request) => &request.$field,
                RepositoryCommitRequest::PreviewExplicit(request) => &request.$field,
                RepositoryCommitRequest::Apply(_) => {
                    unreachable!("validated commit preview contains an apply request")
                }
            }
        }
    };
}

impl ValidatedRepositoryCommitPreviewRequest {
    pub(crate) const fn request(&self) -> &RepositoryCommitRequest {
        &self.request
    }

    pub(crate) fn into_request(self) -> RepositoryCommitRequest {
        self.request
    }

    validated_commit_preview_accessor!(cwd, cwd, OriginalProjectCwd);
    validated_commit_preview_accessor!(task_id, task_id, TaskId);
    validated_commit_preview_accessor!(operation_id, operation_id, OperationId);
    validated_commit_preview_accessor!(integration_set_id, integration_set_id, UnicaId);
    validated_commit_preview_accessor!(
        expected_integration_set_digest,
        expected_integration_set_digest,
        Sha256Digest
    );
    validated_commit_preview_accessor!(lock_set_id, lock_set_id, UnicaId);
    validated_commit_preview_accessor!(
        expected_lock_set_digest,
        expected_lock_set_digest,
        Sha256Digest
    );
    validated_commit_preview_accessor!(verification_id, verification_id, UnicaId);
    validated_commit_preview_accessor!(
        expected_verification_digest,
        expected_verification_digest,
        Sha256Digest
    );
    validated_commit_preview_accessor!(merge_receipt_id, merge_receipt_id, UnicaId);
    validated_commit_preview_accessor!(support_gate_id, support_gate_id, UnicaId);
    validated_commit_preview_accessor!(
        expected_support_gate_digest,
        expected_support_gate_digest,
        Sha256Digest
    );
    validated_commit_preview_accessor!(
        expected_support_gate_history_evidence_digest,
        expected_support_gate_history_evidence_digest,
        Sha256Digest
    );
    validated_commit_preview_accessor!(
        expected_authorized_post_merge_fingerprint,
        expected_authorized_post_merge_fingerprint,
        Sha256Digest
    );
}

macro_rules! validated_commit_apply_accessor {
    ($name:ident, $field:ident, $type:ty) => {
        pub(crate) fn $name(&self) -> &$type {
            match &self.request {
                RepositoryCommitRequest::Apply(request) => &request.$field,
                RepositoryCommitRequest::PreviewOmitted(_)
                | RepositoryCommitRequest::PreviewExplicit(_) => {
                    unreachable!("validated commit apply contains a preview request")
                }
            }
        }
    };
}

impl ValidatedRepositoryCommitApplyRequest {
    pub(crate) const fn request(&self) -> &RepositoryCommitRequest {
        &self.request
    }

    pub(crate) fn into_request(self) -> RepositoryCommitRequest {
        self.request
    }

    validated_commit_apply_accessor!(cwd, cwd, OriginalProjectCwd);
    validated_commit_apply_accessor!(task_id, task_id, TaskId);
    validated_commit_apply_accessor!(operation_id, operation_id, OperationId);
    validated_commit_apply_accessor!(integration_set_id, integration_set_id, UnicaId);
    validated_commit_apply_accessor!(
        expected_integration_set_digest,
        expected_integration_set_digest,
        Sha256Digest
    );
    validated_commit_apply_accessor!(lock_set_id, lock_set_id, UnicaId);
    validated_commit_apply_accessor!(
        expected_lock_set_digest,
        expected_lock_set_digest,
        Sha256Digest
    );
    validated_commit_apply_accessor!(verification_id, verification_id, UnicaId);
    validated_commit_apply_accessor!(
        expected_verification_digest,
        expected_verification_digest,
        Sha256Digest
    );
    validated_commit_apply_accessor!(merge_receipt_id, merge_receipt_id, UnicaId);
    validated_commit_apply_accessor!(support_gate_id, support_gate_id, UnicaId);
    validated_commit_apply_accessor!(
        expected_support_gate_digest,
        expected_support_gate_digest,
        Sha256Digest
    );
    validated_commit_apply_accessor!(
        expected_support_gate_history_evidence_digest,
        expected_support_gate_history_evidence_digest,
        Sha256Digest
    );
    validated_commit_apply_accessor!(
        expected_authorized_post_merge_fingerprint,
        expected_authorized_post_merge_fingerprint,
        Sha256Digest
    );
    validated_commit_apply_accessor!(approved_commit_digest, approved_commit_digest, Sha256Digest);
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

    pub(crate) fn into_validated_preview(
        self,
    ) -> Result<
        ValidatedRepositoryCommitPreviewRequest,
        Box<RepositoryCommitRequestValidationFailure>,
    > {
        if self.request_variant() != RepositoryCommitRequestVariant::Preview {
            return Err(Box::new(RepositoryCommitRequestValidationFailure {
                request: self,
                expected: RepositoryCommitRequestVariant::Preview,
            }));
        }
        Ok(ValidatedRepositoryCommitPreviewRequest { request: self })
    }

    pub(crate) fn into_validated_apply(
        self,
    ) -> Result<ValidatedRepositoryCommitApplyRequest, Box<RepositoryCommitRequestValidationFailure>>
    {
        if self.request_variant() != RepositoryCommitRequestVariant::Apply {
            return Err(Box::new(RepositoryCommitRequestValidationFailure {
                request: self,
                expected: RepositoryCommitRequestVariant::Apply,
            }));
        }
        Ok(ValidatedRepositoryCommitApplyRequest { request: self })
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

/// Owned, non-wire proof that the exact recovery request was the apply leaf
/// and that its approval digest matched the request-bound recovery digest.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ValidatedRecoverApplyRequest {
    request: RepositoryRecoverRequest,
}

/// Owned, non-wire proof that the exact recovery request was the approval-free
/// cancel leaf.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ValidatedRecoverCancelRequest {
    request: RepositoryRecoverRequest,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum RepositoryRecoverRequestValidationCause {
    WrongVariant {
        expected: RepositoryRecoverRequestVariant,
    },
    ApprovalDigestMismatch(DigestApprovalMismatch),
}

/// A consuming recovery-request validation failure. It retains the exact wire
/// request so callers can report or retry without reconstructing its lineage.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct RepositoryRecoverRequestValidationFailure {
    request: RepositoryRecoverRequest,
    cause: RepositoryRecoverRequestValidationCause,
}

impl RepositoryRecoverRequestValidationFailure {
    pub(crate) const fn request(&self) -> &RepositoryRecoverRequest {
        &self.request
    }

    pub(crate) const fn cause(&self) -> &RepositoryRecoverRequestValidationCause {
        &self.cause
    }

    pub(crate) fn into_request(self) -> RepositoryRecoverRequest {
        self.request
    }
}

impl ValidatedRecoverApplyRequest {
    pub(crate) const fn request(&self) -> &RepositoryRecoverRequest {
        &self.request
    }

    pub(crate) fn into_request(self) -> RepositoryRecoverRequest {
        self.request
    }

    pub(crate) fn cwd(&self) -> &OriginalProjectCwd {
        let RepositoryRecoverRequest::Apply(request) = &self.request else {
            unreachable!("validated recover apply contains a cancel request")
        };
        &request.cwd
    }

    pub(crate) fn task_id(&self) -> &TaskId {
        let RepositoryRecoverRequest::Apply(request) = &self.request else {
            unreachable!("validated recover apply contains a cancel request")
        };
        &request.task_id
    }

    pub(crate) fn operation_id(&self) -> &OperationId {
        let RepositoryRecoverRequest::Apply(request) = &self.request else {
            unreachable!("validated recover apply contains a cancel request")
        };
        &request.operation_id
    }

    pub(crate) fn expected_recovery_digest(&self) -> &Sha256Digest {
        let RepositoryRecoverRequest::Apply(request) = &self.request else {
            unreachable!("validated recover apply contains a cancel request")
        };
        &request.expected_recovery_digest
    }

    pub(super) fn approval(&self) -> &DigestApproval {
        let RepositoryRecoverRequest::Apply(request) = &self.request else {
            unreachable!("validated recover apply contains a cancel request")
        };
        &request.approval
    }
}

impl ValidatedRecoverCancelRequest {
    pub(crate) const fn request(&self) -> &RepositoryRecoverRequest {
        &self.request
    }

    pub(crate) fn into_request(self) -> RepositoryRecoverRequest {
        self.request
    }

    pub(crate) fn cwd(&self) -> &OriginalProjectCwd {
        let RepositoryRecoverRequest::Cancel(request) = &self.request else {
            unreachable!("validated recover cancel contains an apply request")
        };
        &request.cwd
    }

    pub(crate) fn task_id(&self) -> &TaskId {
        let RepositoryRecoverRequest::Cancel(request) = &self.request else {
            unreachable!("validated recover cancel contains an apply request")
        };
        &request.task_id
    }

    pub(crate) fn operation_id(&self) -> &OperationId {
        let RepositoryRecoverRequest::Cancel(request) = &self.request else {
            unreachable!("validated recover cancel contains an apply request")
        };
        &request.operation_id
    }

    pub(crate) fn expected_recovery_digest(&self) -> &Sha256Digest {
        let RepositoryRecoverRequest::Cancel(request) = &self.request else {
            unreachable!("validated recover cancel contains an apply request")
        };
        &request.expected_recovery_digest
    }
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

    pub(crate) fn into_validated_apply(
        self,
    ) -> Result<ValidatedRecoverApplyRequest, Box<RepositoryRecoverRequestValidationFailure>> {
        let Self::Apply(request) = &self else {
            return Err(Box::new(RepositoryRecoverRequestValidationFailure {
                request: self,
                cause: RepositoryRecoverRequestValidationCause::WrongVariant {
                    expected: RepositoryRecoverRequestVariant::RecoverApply,
                },
            }));
        };
        if let Err(mismatch) = request
            .approval
            .validate_digest(&request.expected_recovery_digest)
        {
            return Err(Box::new(RepositoryRecoverRequestValidationFailure {
                request: self,
                cause: RepositoryRecoverRequestValidationCause::ApprovalDigestMismatch(mismatch),
            }));
        }
        Ok(ValidatedRecoverApplyRequest { request: self })
    }

    pub(crate) fn into_validated_cancel(
        self,
    ) -> Result<ValidatedRecoverCancelRequest, Box<RepositoryRecoverRequestValidationFailure>> {
        if !matches!(&self, Self::Cancel(_)) {
            return Err(Box::new(RepositoryRecoverRequestValidationFailure {
                request: self,
                cause: RepositoryRecoverRequestValidationCause::WrongVariant {
                    expected: RepositoryRecoverRequestVariant::RecoverCancel,
                },
            }));
        }
        Ok(ValidatedRecoverCancelRequest { request: self })
    }
}

#[cfg(test)]
mod tests {
    use super::{
        RepositoryCommitRequest, RepositoryCommitRequestValidationFailure,
        RepositoryCommitRequestVariant, RepositoryLockRequest, RepositoryLockRequestVariant,
        RepositoryPlanLocksRequest, RepositoryPlanLocksRequestVariant, RepositoryRecoverRequest,
        RepositoryRecoverRequestValidationCause, RepositoryRecoverRequestValidationFailure,
        RepositoryRecoverRequestVariant, RepositoryStatusRequest, RepositoryStatusRequestVariant,
        RepositoryUnlockRequest, RepositoryUnlockRequestVariant, RepositoryUpdateRequest,
        RepositoryUpdateRequestVariant, ValidatedRecoverApplyRequest,
        ValidatedRecoverCancelRequest, ValidatedRepositoryCommitApplyRequest,
        ValidatedRepositoryCommitPreviewRequest,
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

    assert_not_clone!(ValidatedRepositoryCommitPreviewRequest);
    assert_not_clone!(ValidatedRepositoryCommitApplyRequest);
    assert_not_clone!(RepositoryCommitRequestValidationFailure);
    assert_not_serialize!(ValidatedRepositoryCommitPreviewRequest);
    assert_not_serialize!(ValidatedRepositoryCommitApplyRequest);
    assert_not_serialize!(RepositoryCommitRequestValidationFailure);
    assert_not_deserialize_owned!(ValidatedRepositoryCommitPreviewRequest);
    assert_not_deserialize_owned!(ValidatedRepositoryCommitApplyRequest);
    assert_not_deserialize_owned!(RepositoryCommitRequestValidationFailure);
    assert_not_clone!(ValidatedRecoverApplyRequest);
    assert_not_clone!(ValidatedRecoverCancelRequest);
    assert_not_clone!(RepositoryRecoverRequestValidationFailure);
    assert_not_serialize!(ValidatedRecoverApplyRequest);
    assert_not_serialize!(ValidatedRecoverCancelRequest);
    assert_not_serialize!(RepositoryRecoverRequestValidationFailure);
    assert_not_deserialize_owned!(ValidatedRecoverApplyRequest);
    assert_not_deserialize_owned!(ValidatedRecoverCancelRequest);
    assert_not_deserialize_owned!(RepositoryRecoverRequestValidationFailure);

    const _: fn(
        RepositoryCommitRequest,
    ) -> Result<
        ValidatedRepositoryCommitPreviewRequest,
        Box<RepositoryCommitRequestValidationFailure>,
    > = RepositoryCommitRequest::into_validated_preview;
    const _: fn(
        RepositoryCommitRequest,
    ) -> Result<
        ValidatedRepositoryCommitApplyRequest,
        Box<RepositoryCommitRequestValidationFailure>,
    > = RepositoryCommitRequest::into_validated_apply;
    const _: fn(
        RepositoryRecoverRequest,
    ) -> Result<
        ValidatedRecoverApplyRequest,
        Box<RepositoryRecoverRequestValidationFailure>,
    > = RepositoryRecoverRequest::into_validated_apply;
    const _: fn(
        RepositoryRecoverRequest,
    ) -> Result<
        ValidatedRecoverCancelRequest,
        Box<RepositoryRecoverRequestValidationFailure>,
    > = RepositoryRecoverRequest::into_validated_cancel;

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
    fn cancellation_apply_approval_seals_exact_request_lineage_and_rejects_splices() {
        let approved_digest = Sha256Digest::parse(DIGEST).unwrap();
        let other_digest = Sha256Digest::parse(OTHER_DIGEST).unwrap();

        let awaiting = assert_accept::<RepositoryUpdateRequest>(
            preview_leaves(cancellation_base(false), "approvedCancellationDigest")[2].clone(),
        );
        let awaiting = awaiting
            .validate_cancellation_approval(&approved_digest)
            .unwrap();
        assert_eq!(awaiting.approved_cancellation_digest(), &approved_digest);
        assert_eq!(
            serde_json::to_value(awaiting.operation_id()).unwrap(),
            json!(OPERATION_ID)
        );
        assert_eq!(
            serde_json::to_value(awaiting.support_action_id()).unwrap(),
            json!(ID)
        );
        assert_eq!(awaiting.expected_support_action_digest(), &other_digest);
        assert_eq!(
            awaiting.reason(),
            super::SupportCancellationReason::OperatorCancelled
        );
        assert!(matches!(
            awaiting.arming(),
            super::ValidatedCancellationArming::Awaiting
        ));

        let armed = assert_accept::<RepositoryUpdateRequest>(
            preview_leaves(cancellation_base(true), "approvedCancellationDigest")[2].clone(),
        );
        let armed = armed
            .validate_cancellation_approval(&approved_digest)
            .unwrap();
        let super::ValidatedCancellationArming::Armed {
            expected_arming_receipt_id,
            expected_arming_receipt_digest,
        } = armed.arming()
        else {
            panic!("armed cancellation apply lost its arming pair");
        };
        assert_eq!(
            serde_json::to_value(expected_arming_receipt_id).unwrap(),
            json!(OTHER_ID)
        );
        assert_eq!(*expected_arming_receipt_digest, &approved_digest);

        let mismatched = assert_accept::<RepositoryUpdateRequest>(
            preview_leaves(cancellation_base(false), "approvedCancellationDigest")[2].clone(),
        );
        assert!(matches!(
            mismatched.validate_cancellation_approval(&other_digest),
            Err(super::CancellationApprovalValidationError::DigestMismatch(
                _
            ))
        ));

        let preview = assert_accept::<RepositoryUpdateRequest>(cancellation_base(false));
        assert!(matches!(
            preview.validate_cancellation_approval(&approved_digest),
            Err(super::CancellationApprovalValidationError::NotCancellationApply)
        ));
    }

    #[test]
    fn update_apply_approvals_seal_routine_and_prerequisite_lineage() {
        let approved_digest = Sha256Digest::parse(DIGEST).unwrap();
        let other_digest = Sha256Digest::parse(OTHER_DIGEST).unwrap();

        let routine = assert_accept::<RepositoryUpdateRequest>(
            preview_leaves(routine_base(), "approvedUpdateDigest")[2].clone(),
        );
        let routine = routine
            .validate_routine_update_approval(&approved_digest)
            .unwrap();
        assert_eq!(routine.approved_update_digest(), &approved_digest);
        assert_eq!(
            serde_json::to_value(routine.operation_id()).unwrap(),
            json!(OPERATION_ID)
        );
        assert_eq!(routine.expected_status_digest(), &approved_digest);

        let prerequisite = assert_accept::<RepositoryUpdateRequest>(
            preview_leaves(prerequisite_base(), "approvedUpdateDigest")[2].clone(),
        );
        let prerequisite = prerequisite
            .validate_prerequisite_update_approval(&approved_digest)
            .unwrap();
        assert_eq!(
            serde_json::to_value(prerequisite.support_action_id()).unwrap(),
            json!(ID)
        );
        assert_eq!(prerequisite.expected_support_action_digest(), &other_digest);
        assert_eq!(
            serde_json::to_value(prerequisite.expected_arming_receipt_id()).unwrap(),
            json!(OTHER_ID)
        );
        assert_eq!(
            prerequisite.expected_arming_receipt_digest(),
            &approved_digest
        );

        let mismatch = assert_accept::<RepositoryUpdateRequest>(
            preview_leaves(routine_base(), "approvedUpdateDigest")[2].clone(),
        );
        assert!(matches!(
            mismatch.validate_routine_update_approval(&other_digest),
            Err(super::UpdateApprovalValidationError::DigestMismatch(_))
        ));
        assert!(matches!(
            mismatch.validate_prerequisite_update_approval(&approved_digest),
            Err(super::UpdateApprovalValidationError::NotPrerequisiteApply)
        ));

        let preview = assert_accept::<RepositoryUpdateRequest>(routine_base());
        assert!(matches!(
            preview.validate_routine_update_approval(&approved_digest),
            Err(super::UpdateApprovalValidationError::NotRoutineApply)
        ));
    }

    #[test]
    fn arm_apply_approval_seals_exact_action_lineage() {
        let approved_digest = Sha256Digest::parse(DIGEST).unwrap();
        let other_digest = Sha256Digest::parse(OTHER_DIGEST).unwrap();

        let apply = assert_accept::<RepositoryUpdateRequest>(arm_apply());
        let validated = apply.validate_arm_approval(&approved_digest).unwrap();
        assert_eq!(validated.approved_arming_digest(), &approved_digest);
        assert_eq!(validated.expected_status_digest(), &approved_digest);
        assert_eq!(
            serde_json::to_value(validated.operation_id()).unwrap(),
            json!(OPERATION_ID)
        );
        assert_eq!(
            serde_json::to_value(validated.support_action_id()).unwrap(),
            json!(ID)
        );
        assert_eq!(validated.expected_support_action_digest(), &other_digest);

        let mismatch = assert_accept::<RepositoryUpdateRequest>(arm_apply());
        assert!(matches!(
            mismatch.validate_arm_approval(&other_digest),
            Err(super::ArmApprovalValidationError::DigestMismatch(_))
        ));

        let preview = assert_accept::<RepositoryUpdateRequest>(arm_preview());
        assert!(matches!(
            preview.validate_arm_approval(&approved_digest),
            Err(super::ArmApprovalValidationError::NotArmApply)
        ));
    }

    #[test]
    fn update_preview_contexts_seal_exact_request_lineage() {
        let status_digest = Sha256Digest::parse(DIGEST).unwrap();
        let action_digest = Sha256Digest::parse(OTHER_DIGEST).unwrap();

        for value in &preview_leaves(routine_base(), "approvedUpdateDigest")[..2] {
            let request = assert_accept::<RepositoryUpdateRequest>(value.clone());
            let context = request.validate_routine_update_preview_context().unwrap();
            assert_eq!(context.expected_status_digest(), &status_digest);
            assert_eq!(
                serde_json::to_value(context.operation_id()).unwrap(),
                json!(OPERATION_ID)
            );
        }

        let prerequisite = assert_accept::<RepositoryUpdateRequest>(
            preview_leaves(prerequisite_base(), "approvedUpdateDigest")[0].clone(),
        );
        let prerequisite = prerequisite
            .validate_prerequisite_update_preview_context()
            .unwrap();
        assert_eq!(
            prerequisite.expected_support_action_digest(),
            &action_digest
        );
        assert_eq!(
            serde_json::to_value(prerequisite.expected_arming_receipt_id()).unwrap(),
            json!(OTHER_ID)
        );

        let arm = assert_accept::<RepositoryUpdateRequest>(arm_preview());
        let arm = arm.validate_arm_preview_context().unwrap();
        assert_eq!(arm.expected_status_digest(), &status_digest);
        assert_eq!(arm.expected_support_action_digest(), &action_digest);

        let awaiting = assert_accept::<RepositoryUpdateRequest>(cancellation_base(false));
        let awaiting = awaiting.validate_cancellation_preview_context().unwrap();
        assert_eq!(
            awaiting.reason(),
            super::SupportCancellationReason::OperatorCancelled
        );
        assert!(matches!(
            awaiting.arming(),
            super::ValidatedCancellationArming::Awaiting
        ));

        let armed = assert_accept::<RepositoryUpdateRequest>(cancellation_base(true));
        let armed = armed.validate_cancellation_preview_context().unwrap();
        assert!(matches!(
            armed.arming(),
            super::ValidatedCancellationArming::Armed { .. }
        ));

        let apply = assert_accept::<RepositoryUpdateRequest>(arm_apply());
        assert!(matches!(
            apply.validate_arm_preview_context(),
            Err(super::UpdatePreviewValidationError::Arm)
        ));
        assert!(matches!(
            apply.validate_cancellation_preview_context(),
            Err(super::UpdatePreviewValidationError::Cancellation)
        ));
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

    #[test]
    fn commit_request_validation_consumes_and_retains_the_exact_wire_leaf() {
        let leaves = preview_leaves(commit_base(), "approvedCommitDigest");

        for preview_wire in &leaves[..2] {
            let validated = assert_accept::<RepositoryCommitRequest>(preview_wire.clone())
                .into_validated_preview()
                .unwrap();
            assert_eq!(serde_json::to_value(validated.cwd()).unwrap(), json!(CWD));
            assert_eq!(
                serde_json::to_value(validated.task_id()).unwrap(),
                json!(TASK_ID)
            );
            assert_eq!(
                serde_json::to_value(validated.operation_id()).unwrap(),
                json!(OPERATION_ID)
            );
            assert_eq!(
                serde_json::to_value(validated.integration_set_id()).unwrap(),
                json!(ID)
            );
            assert_eq!(
                serde_json::to_value(validated.expected_integration_set_digest()).unwrap(),
                json!(DIGEST)
            );
            assert_eq!(
                serde_json::to_value(validated.lock_set_id()).unwrap(),
                json!(OTHER_ID)
            );
            assert_eq!(
                serde_json::to_value(validated.expected_lock_set_digest()).unwrap(),
                json!(OTHER_DIGEST)
            );
            assert_eq!(
                serde_json::to_value(validated.verification_id()).unwrap(),
                json!(ID)
            );
            assert_eq!(
                serde_json::to_value(validated.expected_verification_digest()).unwrap(),
                json!(DIGEST)
            );
            assert_eq!(
                serde_json::to_value(validated.merge_receipt_id()).unwrap(),
                json!(OTHER_ID)
            );
            assert_eq!(
                serde_json::to_value(validated.support_gate_id()).unwrap(),
                json!(ID)
            );
            assert_eq!(
                serde_json::to_value(validated.expected_support_gate_digest()).unwrap(),
                json!(OTHER_DIGEST)
            );
            assert_eq!(
                serde_json::to_value(validated.expected_support_gate_history_evidence_digest())
                    .unwrap(),
                json!(DIGEST)
            );
            assert_eq!(
                serde_json::to_value(validated.expected_authorized_post_merge_fingerprint())
                    .unwrap(),
                json!(OTHER_DIGEST)
            );
            assert_eq!(
                serde_json::to_value(validated.into_request()).unwrap(),
                *preview_wire
            );
        }

        let apply_wire = leaves[2].clone();
        let validated = assert_accept::<RepositoryCommitRequest>(apply_wire.clone())
            .into_validated_apply()
            .unwrap();
        assert_eq!(
            serde_json::to_value(validated.approved_commit_digest()).unwrap(),
            json!(DIGEST)
        );
        assert_eq!(
            serde_json::to_value(validated.into_request()).unwrap(),
            apply_wire
        );
    }

    #[test]
    fn commit_request_wrong_variant_failure_retains_the_consumed_request() {
        let leaves = preview_leaves(commit_base(), "approvedCommitDigest");
        let preview_wire = leaves[0].clone();
        let blocked = assert_accept::<RepositoryCommitRequest>(preview_wire.clone())
            .into_validated_apply()
            .unwrap_err();
        assert_eq!(blocked.expected(), RepositoryCommitRequestVariant::Apply);
        assert_eq!(
            serde_json::to_value(blocked.into_request()).unwrap(),
            preview_wire
        );

        let apply_wire = leaves[2].clone();
        let blocked = assert_accept::<RepositoryCommitRequest>(apply_wire.clone())
            .into_validated_preview()
            .unwrap_err();
        assert_eq!(blocked.expected(), RepositoryCommitRequestVariant::Preview);
        assert_eq!(
            serde_json::to_value(blocked.into_request()).unwrap(),
            apply_wire
        );
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
    fn recover_request_apply_validation_consumes_and_retains_the_exact_request() {
        let wire = recover_apply();
        let request = assert_accept::<RepositoryRecoverRequest>(wire.clone());
        let request_bytes = serde_json::to_vec(&request).unwrap();
        let validated = request.into_validated_apply().unwrap();

        assert_eq!(
            validated.request().request_variant(),
            RepositoryRecoverRequestVariant::RecoverApply
        );
        assert_eq!(serde_json::to_value(validated.cwd()).unwrap(), json!(CWD));
        assert_eq!(
            serde_json::to_value(validated.task_id()).unwrap(),
            json!(TASK_ID)
        );
        assert_eq!(
            serde_json::to_value(validated.operation_id()).unwrap(),
            json!(OPERATION_ID)
        );
        assert_eq!(validated.expected_recovery_digest().as_str(), DIGEST);
        assert_eq!(
            serde_json::to_value(validated.approval()).unwrap(),
            approval()
        );
        assert_eq!(
            serde_json::to_vec(validated.request()).unwrap(),
            request_bytes
        );
        assert_eq!(
            serde_json::to_vec(&validated.into_request()).unwrap(),
            request_bytes
        );
    }

    #[test]
    fn recover_request_cancel_validation_consumes_and_retains_the_exact_request() {
        let wire = recover_cancel();
        let request = assert_accept::<RepositoryRecoverRequest>(wire.clone());
        let request_bytes = serde_json::to_vec(&request).unwrap();
        let validated = request.into_validated_cancel().unwrap();

        assert_eq!(
            validated.request().request_variant(),
            RepositoryRecoverRequestVariant::RecoverCancel
        );
        assert_eq!(serde_json::to_value(validated.cwd()).unwrap(), json!(CWD));
        assert_eq!(
            serde_json::to_value(validated.task_id()).unwrap(),
            json!(TASK_ID)
        );
        assert_eq!(
            serde_json::to_value(validated.operation_id()).unwrap(),
            json!(OPERATION_ID)
        );
        assert_eq!(validated.expected_recovery_digest().as_str(), DIGEST);
        assert_eq!(
            serde_json::to_vec(validated.request()).unwrap(),
            request_bytes
        );
        assert_eq!(
            serde_json::to_vec(&validated.into_request()).unwrap(),
            request_bytes
        );
    }

    #[test]
    fn recover_request_wrong_variant_failure_retains_the_exact_request_and_cause() {
        let cancel = assert_accept::<RepositoryRecoverRequest>(recover_cancel());
        let cancel_bytes = serde_json::to_vec(&cancel).unwrap();
        let blocked = cancel.into_validated_apply().unwrap_err();
        assert_eq!(
            blocked.cause(),
            &RepositoryRecoverRequestValidationCause::WrongVariant {
                expected: RepositoryRecoverRequestVariant::RecoverApply,
            }
        );
        assert_eq!(serde_json::to_vec(blocked.request()).unwrap(), cancel_bytes);
        assert_eq!(
            serde_json::to_vec(&blocked.into_request()).unwrap(),
            cancel_bytes
        );

        let apply = assert_accept::<RepositoryRecoverRequest>(recover_apply());
        let apply_bytes = serde_json::to_vec(&apply).unwrap();
        let blocked = apply.into_validated_cancel().unwrap_err();
        assert_eq!(
            blocked.cause(),
            &RepositoryRecoverRequestValidationCause::WrongVariant {
                expected: RepositoryRecoverRequestVariant::RecoverCancel,
            }
        );
        assert_eq!(serde_json::to_vec(blocked.request()).unwrap(), apply_bytes);
        assert_eq!(
            serde_json::to_vec(&blocked.into_request()).unwrap(),
            apply_bytes
        );
    }

    #[test]
    fn recover_request_stale_apply_digest_retains_the_exact_request_and_mismatch() {
        let wire = with(
            recover_apply(),
            &[("approval", approval_with_digest(OTHER_DIGEST))],
        );
        let request = assert_accept::<RepositoryRecoverRequest>(wire.clone());
        let request_bytes = serde_json::to_vec(&request).unwrap();
        let blocked = request.into_validated_apply().unwrap_err();

        let RepositoryRecoverRequestValidationCause::ApprovalDigestMismatch(mismatch) =
            blocked.cause()
        else {
            panic!("stale apply digest reported the wrong validation cause");
        };
        assert_eq!(mismatch.expected().as_str(), DIGEST);
        assert_eq!(mismatch.observed().as_str(), OTHER_DIGEST);
        assert_eq!(
            serde_json::to_vec(blocked.request()).unwrap(),
            request_bytes
        );
        assert_eq!(
            serde_json::to_vec(&blocked.into_request()).unwrap(),
            request_bytes
        );
    }

    #[test]
    fn recovery_apply_and_cancel_are_separate_closed_decisions_and_policies() {
        let apply = assert_accept::<RepositoryRecoverRequest>(recover_apply());
        assert_eq!(
            apply.request_variant(),
            RepositoryRecoverRequestVariant::RecoverApply
        );
        assert_eq!(apply.execution_policy(), ExecutionPolicy::JournaledEffect);

        let cancel = assert_accept::<RepositoryRecoverRequest>(recover_cancel());
        assert_eq!(
            cancel.request_variant(),
            RepositoryRecoverRequestVariant::RecoverCancel
        );
        assert_eq!(cancel.execution_policy(), ExecutionPolicy::LocalJournaled);

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
        assert_accept::<RepositoryRecoverRequest>(mismatched_apply.clone());
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
