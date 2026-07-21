use super::actions::{
    AdaptationRefreshActions, CommitSafeExitActions, ConflictReviewActions,
    IntegrationRecoveryExitActions, IntegrationUnlockExitActions, NoNextActions,
    RecoveryApplyActions, RecoveryApplyOrCancelActions, StartAndStatusActions, StatusOnlyActions,
};
use super::artifacts::{ArtifactKind, ArtifactKindRole, ArtifactRole};
use super::scalars::{LocalProfileName, PropertyPath, RequiredNullable};
use super::schema::{one_of_schema, string_schema};
use crate::domain::branched_development::{
    CapabilityRowId, OperationId, ProjectId, Sha256Digest, TaskId, TaskPhase, UnicaId,
};
use schemars::{JsonSchema, Schema, SchemaGenerator};
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};
use std::borrow::Cow;
use std::str::FromStr;

const MAX_RESULT_ITEMS: usize = 1024;

macro_rules! closed_string_enum {
    ($name:ident { $($variant:ident => $wire:literal),+ $(,)? }) => {
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash,
            Serialize, Deserialize, JsonSchema,
        )]
        pub(crate) enum $name {
            $(#[serde(rename = $wire)] $variant),+
        }

        impl $name {
            pub(crate) const ALL: &'static [Self] = &[$(Self::$variant),+];

            pub(crate) const fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $wire),+
                }
            }
        }
    };
}

closed_string_enum!(StableErrorCode {
    RepositoryBindingMismatch => "repositoryBindingMismatch",
    MainDiffersFromRepository => "mainDiffersFromRepository",
    ArtifactKindMismatch => "artifactKindMismatch",
    ArtifactNotDistribution => "artifactNotDistribution",
    PlatformWarningRejected => "platformWarningRejected",
    VendorAncestryMismatch => "vendorAncestryMismatch",
    TwiceChangedProperties => "twiceChangedProperties",
    UnresolvedReferences => "unresolvedReferences",
    UnexpectedDelta => "unexpectedDelta",
    AdaptationDecisionAlreadyRecorded => "adaptationDecisionAlreadyRecorded",
    ConflictDecisionsIncomplete => "conflictDecisionsIncomplete",
    UnboundResolutionChanges => "unboundResolutionChanges",
    ValidationFailed => "validationFailed",
    RepositoryLockConflict => "repositoryLockConflict",
    OperationTimedOut => "operationTimedOut",
    RepositoryLockRollbackFailed => "repositoryLockRollbackFailed",
    RepositoryUpdatePlanStale => "repositoryUpdatePlanStale",
    RepositoryStructureConfirmationUnproven => "repositoryStructureConfirmationUnproven",
    ManualSupportRequired => "manualSupportRequired",
    ManualSupportCleanupRequired => "manualSupportCleanupRequired",
    VendorForbidsChanges => "vendorForbidsChanges",
    SupportPreflightInconclusive => "supportPreflightInconclusive",
    ManualSupportRootLockRequired => "manualSupportRootLockRequired",
    SupportPrerequisiteArmStale => "supportPrerequisiteArmStale",
    ManualSupportActionPending => "manualSupportActionPending",
    ManualSupportLocksRemain => "manualSupportLocksRemain",
    ManualSupportLocalChangesRemain => "manualSupportLocalChangesRemain",
    ManualSupportPrerequisiteInvalid => "manualSupportPrerequisiteInvalid",
    SupportPrerequisiteConflict => "supportPrerequisiteConflict",
    SupportCorrectionPending => "supportCorrectionPending",
    SupportRecoveryReapprovalRequired => "supportRecoveryReapprovalRequired",
    RecoveryReapprovalRequired => "recoveryReapprovalRequired",
    SupportConflictResolutionPending => "supportConflictResolutionPending",
    SupportRecoveryBlockedByLock => "supportRecoveryBlockedByLock",
    PreArmCancellationRecoveryBlocked => "preArmCancellationRecoveryBlocked",
    SupportPrerequisiteReconciliationRequired => "supportPrerequisiteReconciliationRequired",
    RelevantBaselineChanged => "relevantBaselineChanged",
    SupportPreflightStale => "supportPreflightStale",
    MainPreparationMismatch => "mainPreparationMismatch",
    AdditionalLocksRequired => "additionalLocksRequired",
    MainMergeValidationFailed => "mainMergeValidationFailed",
    PostMergeLineageChanged => "postMergeLineageChanged",
    RepositoryCommitFailed => "repositoryCommitFailed",
    RepositoryCommitAmbiguous => "repositoryCommitAmbiguous",
    RepositoryUnlockUnverified => "repositoryUnlockUnverified",
    CleanupNotAllowed => "cleanupNotAllowed",
    AbandonmentRecoveryRequired => "abandonmentRecoveryRequired",
    UnsafeTaskPath => "unsafeTaskPath",
    OperationReplayMismatch => "operationReplayMismatch",
    TargetReservationBusy => "targetReservationBusy",
    RepositoryAccountReservationBusy => "repositoryAccountReservationBusy",
    OperationEffectUnknown => "operationEffectUnknown",
    RecoveryPlanPending => "recoveryPlanPending",
    TaskPhaseMismatch => "taskPhaseMismatch",
    ApprovalDigestMismatch => "approvalDigestMismatch",
    ChangeReceiptStale => "changeReceiptStale",
    ConflictResolutionNotAllowed => "conflictResolutionNotAllowed",
    TaskMutationBlocked => "taskMutationBlocked",
    PlatformCapabilityUnproven => "platformCapabilityUnproven",
    SupportLayerAmbiguous => "supportLayerAmbiguous",
    UnsupportedChangeKind => "unsupportedChangeKind",
    ProjectIdentityCollision => "projectIdentityCollision",
    StateRootRelocationRequired => "stateRootRelocationRequired",
    ExclusiveRepositoryUserRequired => "exclusiveRepositoryUserRequired",
    RollbackUnproven => "rollbackUnproven",
    TaskAbandonmentNotSafe => "taskAbandonmentNotSafe",
    ProfileInvalid => "profileInvalid",
    SecretUnavailable => "secretUnavailable",
    StateCorrupt => "stateCorrupt",
    OperationInProgress => "operationInProgress",
    TaskNotFound => "taskNotFound",
    TaskWorkspaceContextInvalid => "taskWorkspaceContextInvalid",
    ToolNotBranchedCompatible => "toolNotBranchedCompatible",
    CommitCommentPolicyMismatch => "commitCommentPolicyMismatch",
    IntegrationSetMismatch => "integrationSetMismatch",
});

closed_string_enum!(RejectedCode {
    RepositoryBindingMismatch => "repositoryBindingMismatch",
    MainDiffersFromRepository => "mainDiffersFromRepository",
    ArtifactNotDistribution => "artifactNotDistribution",
    CleanupNotAllowed => "cleanupNotAllowed",
    TaskAbandonmentNotSafe => "taskAbandonmentNotSafe",
    OperationReplayMismatch => "operationReplayMismatch",
    RecoveryPlanPending => "recoveryPlanPending",
    TaskPhaseMismatch => "taskPhaseMismatch",
    ApprovalDigestMismatch => "approvalDigestMismatch",
    ChangeReceiptStale => "changeReceiptStale",
    ConflictResolutionNotAllowed => "conflictResolutionNotAllowed",
    AdaptationDecisionAlreadyRecorded => "adaptationDecisionAlreadyRecorded",
    TaskMutationBlocked => "taskMutationBlocked",
    PlatformCapabilityUnproven => "platformCapabilityUnproven",
    SupportLayerAmbiguous => "supportLayerAmbiguous",
    UnsupportedChangeKind => "unsupportedChangeKind",
    ProjectIdentityCollision => "projectIdentityCollision",
    StateRootRelocationRequired => "stateRootRelocationRequired",
    ExclusiveRepositoryUserRequired => "exclusiveRepositoryUserRequired",
    TargetReservationBusy => "targetReservationBusy",
    RepositoryAccountReservationBusy => "repositoryAccountReservationBusy",
    ProfileInvalid => "profileInvalid",
    SecretUnavailable => "secretUnavailable",
    StateCorrupt => "stateCorrupt",
    OperationInProgress => "operationInProgress",
    TaskNotFound => "taskNotFound",
    TaskWorkspaceContextInvalid => "taskWorkspaceContextInvalid",
    ToolNotBranchedCompatible => "toolNotBranchedCompatible",
    CommitCommentPolicyMismatch => "commitCommentPolicyMismatch",
    IntegrationSetMismatch => "integrationSetMismatch",
});

mod stable_code_marker_sealed {
    pub trait Sealed {}
}

pub(super) trait StableCodeMarker:
    stable_code_marker_sealed::Sealed + Copy + Default
{
    const CODE: StableErrorCode;
}

macro_rules! stable_code_marker {
    ($name:ident, $code:path) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
        pub(super) struct $name;

        impl stable_code_marker_sealed::Sealed for $name {}

        impl StableCodeMarker for $name {
            const CODE: StableErrorCode = $code;
        }

        impl Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: serde::Serializer,
            {
                Self::CODE.serialize(serializer)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                let observed = StableErrorCode::deserialize(deserializer)?;
                (observed == Self::CODE)
                    .then_some(Self)
                    .ok_or_else(|| D::Error::custom("stable error code marker mismatch"))
            }
        }

        impl JsonSchema for $name {
            fn schema_name() -> Cow<'static, str> {
                stringify!($name).into()
            }

            fn json_schema(_: &mut SchemaGenerator) -> Schema {
                let literal = Self::CODE.as_str();
                schemars::json_schema!({ "type": "string", "const": literal })
            }
        }
    };
}

stable_code_marker!(
    RepositoryBindingMismatchMarker,
    StableErrorCode::RepositoryBindingMismatch
);
stable_code_marker!(
    MainDiffersFromRepositoryMarker,
    StableErrorCode::MainDiffersFromRepository
);
stable_code_marker!(
    ArtifactKindMismatchMarker,
    StableErrorCode::ArtifactKindMismatch
);
stable_code_marker!(
    ArtifactNotDistributionMarker,
    StableErrorCode::ArtifactNotDistribution
);
stable_code_marker!(
    PlatformWarningRejectedMarker,
    StableErrorCode::PlatformWarningRejected
);
stable_code_marker!(
    VendorAncestryMismatchMarker,
    StableErrorCode::VendorAncestryMismatch
);
stable_code_marker!(
    TwiceChangedPropertiesMarker,
    StableErrorCode::TwiceChangedProperties
);
stable_code_marker!(
    UnresolvedReferencesMarker,
    StableErrorCode::UnresolvedReferences
);
stable_code_marker!(UnexpectedDeltaMarker, StableErrorCode::UnexpectedDelta);
stable_code_marker!(
    AdaptationDecisionAlreadyRecordedMarker,
    StableErrorCode::AdaptationDecisionAlreadyRecorded
);
stable_code_marker!(
    ConflictDecisionsIncompleteMarker,
    StableErrorCode::ConflictDecisionsIncomplete
);
stable_code_marker!(
    UnboundResolutionChangesMarker,
    StableErrorCode::UnboundResolutionChanges
);
stable_code_marker!(ValidationFailedMarker, StableErrorCode::ValidationFailed);
stable_code_marker!(
    RepositoryLockConflictMarker,
    StableErrorCode::RepositoryLockConflict
);
stable_code_marker!(OperationTimedOutMarker, StableErrorCode::OperationTimedOut);
stable_code_marker!(
    RepositoryLockRollbackFailedMarker,
    StableErrorCode::RepositoryLockRollbackFailed
);
stable_code_marker!(
    RepositoryUpdatePlanStaleMarker,
    StableErrorCode::RepositoryUpdatePlanStale
);
stable_code_marker!(
    RepositoryStructureConfirmationUnprovenMarker,
    StableErrorCode::RepositoryStructureConfirmationUnproven
);
stable_code_marker!(
    ManualSupportRequiredMarker,
    StableErrorCode::ManualSupportRequired
);
stable_code_marker!(
    ManualSupportCleanupRequiredMarker,
    StableErrorCode::ManualSupportCleanupRequired
);
stable_code_marker!(
    VendorForbidsChangesMarker,
    StableErrorCode::VendorForbidsChanges
);
stable_code_marker!(
    SupportPreflightInconclusiveMarker,
    StableErrorCode::SupportPreflightInconclusive
);
stable_code_marker!(
    ManualSupportRootLockRequiredMarker,
    StableErrorCode::ManualSupportRootLockRequired
);
stable_code_marker!(
    SupportPrerequisiteArmStaleMarker,
    StableErrorCode::SupportPrerequisiteArmStale
);
stable_code_marker!(
    ManualSupportActionPendingMarker,
    StableErrorCode::ManualSupportActionPending
);
stable_code_marker!(
    ManualSupportLocksRemainMarker,
    StableErrorCode::ManualSupportLocksRemain
);
stable_code_marker!(
    ManualSupportLocalChangesRemainMarker,
    StableErrorCode::ManualSupportLocalChangesRemain
);
stable_code_marker!(
    ManualSupportPrerequisiteInvalidMarker,
    StableErrorCode::ManualSupportPrerequisiteInvalid
);
stable_code_marker!(
    SupportPrerequisiteConflictMarker,
    StableErrorCode::SupportPrerequisiteConflict
);
stable_code_marker!(
    SupportCorrectionPendingMarker,
    StableErrorCode::SupportCorrectionPending
);
stable_code_marker!(
    SupportRecoveryReapprovalRequiredMarker,
    StableErrorCode::SupportRecoveryReapprovalRequired
);
stable_code_marker!(
    RecoveryReapprovalRequiredMarker,
    StableErrorCode::RecoveryReapprovalRequired
);
stable_code_marker!(
    SupportConflictResolutionPendingMarker,
    StableErrorCode::SupportConflictResolutionPending
);
stable_code_marker!(
    SupportRecoveryBlockedByLockMarker,
    StableErrorCode::SupportRecoveryBlockedByLock
);
stable_code_marker!(
    PreArmCancellationRecoveryBlockedMarker,
    StableErrorCode::PreArmCancellationRecoveryBlocked
);
stable_code_marker!(
    SupportPrerequisiteReconciliationRequiredMarker,
    StableErrorCode::SupportPrerequisiteReconciliationRequired
);
stable_code_marker!(
    RelevantBaselineChangedMarker,
    StableErrorCode::RelevantBaselineChanged
);
stable_code_marker!(
    SupportPreflightStaleMarker,
    StableErrorCode::SupportPreflightStale
);
stable_code_marker!(
    MainPreparationMismatchMarker,
    StableErrorCode::MainPreparationMismatch
);
stable_code_marker!(
    AdditionalLocksRequiredMarker,
    StableErrorCode::AdditionalLocksRequired
);
stable_code_marker!(
    MainMergeValidationFailedMarker,
    StableErrorCode::MainMergeValidationFailed
);
stable_code_marker!(
    PostMergeLineageChangedMarker,
    StableErrorCode::PostMergeLineageChanged
);
stable_code_marker!(
    RepositoryCommitFailedMarker,
    StableErrorCode::RepositoryCommitFailed
);
stable_code_marker!(
    RepositoryCommitAmbiguousMarker,
    StableErrorCode::RepositoryCommitAmbiguous
);
stable_code_marker!(
    RepositoryUnlockUnverifiedMarker,
    StableErrorCode::RepositoryUnlockUnverified
);
stable_code_marker!(CleanupNotAllowedMarker, StableErrorCode::CleanupNotAllowed);
stable_code_marker!(
    AbandonmentRecoveryRequiredMarker,
    StableErrorCode::AbandonmentRecoveryRequired
);
stable_code_marker!(UnsafeTaskPathMarker, StableErrorCode::UnsafeTaskPath);
stable_code_marker!(
    OperationReplayMismatchMarker,
    StableErrorCode::OperationReplayMismatch
);
stable_code_marker!(
    TargetReservationBusyMarker,
    StableErrorCode::TargetReservationBusy
);
stable_code_marker!(
    RepositoryAccountReservationBusyMarker,
    StableErrorCode::RepositoryAccountReservationBusy
);
stable_code_marker!(
    OperationEffectUnknownMarker,
    StableErrorCode::OperationEffectUnknown
);
stable_code_marker!(
    RecoveryPlanPendingMarker,
    StableErrorCode::RecoveryPlanPending
);
stable_code_marker!(TaskPhaseMismatchMarker, StableErrorCode::TaskPhaseMismatch);
stable_code_marker!(
    ApprovalDigestMismatchMarker,
    StableErrorCode::ApprovalDigestMismatch
);
stable_code_marker!(
    ChangeReceiptStaleMarker,
    StableErrorCode::ChangeReceiptStale
);
stable_code_marker!(
    ConflictResolutionNotAllowedMarker,
    StableErrorCode::ConflictResolutionNotAllowed
);
stable_code_marker!(
    TaskMutationBlockedMarker,
    StableErrorCode::TaskMutationBlocked
);
stable_code_marker!(
    PlatformCapabilityUnprovenMarker,
    StableErrorCode::PlatformCapabilityUnproven
);
stable_code_marker!(
    SupportLayerAmbiguousMarker,
    StableErrorCode::SupportLayerAmbiguous
);
stable_code_marker!(
    UnsupportedChangeKindMarker,
    StableErrorCode::UnsupportedChangeKind
);
stable_code_marker!(
    ProjectIdentityCollisionMarker,
    StableErrorCode::ProjectIdentityCollision
);
stable_code_marker!(
    StateRootRelocationRequiredMarker,
    StableErrorCode::StateRootRelocationRequired
);
stable_code_marker!(
    ExclusiveRepositoryUserRequiredMarker,
    StableErrorCode::ExclusiveRepositoryUserRequired
);
stable_code_marker!(RollbackUnprovenMarker, StableErrorCode::RollbackUnproven);
stable_code_marker!(
    TaskAbandonmentNotSafeMarker,
    StableErrorCode::TaskAbandonmentNotSafe
);
stable_code_marker!(ProfileInvalidMarker, StableErrorCode::ProfileInvalid);
stable_code_marker!(SecretUnavailableMarker, StableErrorCode::SecretUnavailable);
stable_code_marker!(StateCorruptMarker, StableErrorCode::StateCorrupt);
stable_code_marker!(
    OperationInProgressMarker,
    StableErrorCode::OperationInProgress
);
stable_code_marker!(TaskNotFoundMarker, StableErrorCode::TaskNotFound);
stable_code_marker!(
    TaskWorkspaceContextInvalidMarker,
    StableErrorCode::TaskWorkspaceContextInvalid
);
stable_code_marker!(
    ToolNotBranchedCompatibleMarker,
    StableErrorCode::ToolNotBranchedCompatible
);
stable_code_marker!(
    CommitCommentPolicyMismatchMarker,
    StableErrorCode::CommitCommentPolicyMismatch
);
stable_code_marker!(
    IntegrationSetMismatchMarker,
    StableErrorCode::IntegrationSetMismatch
);

pub(super) trait RejectedCodeMarker: StableCodeMarker {
    fn data_schema(generator: &mut SchemaGenerator) -> Schema;
}

impl From<RejectedCode> for StableErrorCode {
    fn from(code: RejectedCode) -> Self {
        StableErrorCode::ALL
            .iter()
            .copied()
            .find(|stable| stable.as_str() == code.as_str())
            .expect("the rejected vocabulary is a closed stable-code subset")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub(crate) struct IncomingToolName(String);

impl IncomingToolName {
    pub(crate) fn parse(value: &str) -> Result<Self, &'static str> {
        let suffix = value
            .strip_prefix("unica.")
            .ok_or("incoming tool name must start with unica.")?;
        if suffix.is_empty()
            || suffix.len() > 122
            || !suffix.as_bytes()[0].is_ascii_alphanumeric()
            || !suffix
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
        {
            return Err("incoming tool name is not a bounded registered-name candidate");
        }
        Ok(Self(value.to_owned()))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for IncomingToolName {
    type Err = &'static str;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

impl<'de> Deserialize<'de> for IncomingToolName {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::parse(&String::deserialize(deserializer)?).map_err(D::Error::custom)
    }
}

impl JsonSchema for IncomingToolName {
    fn inline_schema() -> bool {
        true
    }

    fn schema_name() -> Cow<'static, str> {
        "IncomingToolName".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        string_schema(
            7,
            128,
            Some(r"^unica\.[A-Za-z0-9][A-Za-z0-9._-]{0,121}$"),
            None,
        )
    }
}

closed_string_enum!(ConflictKind {
    TwiceChanged => "twiceChanged",
    DeleteModify => "deleteModify",
    AddAddNameCollision => "addAddNameCollision",
    UuidMismatch => "uuidMismatch",
    UnresolvedReference => "unresolvedReference",
    SupportRuleBlocked => "supportRuleBlocked",
    MergeSettingsRejected => "mergeSettingsRejected",
});

closed_string_enum!(ConflictResolution {
    TakeOurs => "takeOurs",
    TakeTheirs => "takeTheirs",
    Combine => "combine",
    Manual => "manual",
});

closed_string_enum!(CommitCommentPolicyMismatchKind {
    TemplateChanged => "templateChanged",
    TaskMetadataChanged => "taskMetadataChanged",
    RenderEmpty => "renderEmpty",
    RenderNotTaskBound => "renderNotTaskBound",
});

closed_string_enum!(IntegrationSetMismatchKind {
    Plan => "planSet",
    Merge => "mergeSet",
    Verification => "verificationSet",
    Commit => "commitSet",
    Lock => "lockSet",
});

closed_string_enum!(WorkspaceMismatchKind {
    ProjectMismatch => "projectMismatch",
    MarkerMissing => "markerMissing",
    MarkerMismatch => "markerMismatch",
    LeaseMissing => "leaseMissing",
    LeaseInvalid => "leaseInvalid",
});

macro_rules! canonical_list {
    ($name:ident, $item:ty, $ordinal:expr, $non_empty:literal) => {
        #[derive(Debug, Clone, PartialEq, Eq, Serialize)]
        #[serde(transparent)]
        struct $name(Vec<$item>);

        impl $name {
            fn new(values: Vec<$item>) -> Result<Self, &'static str> {
                if values.len() > MAX_RESULT_ITEMS || ($non_empty && values.is_empty()) {
                    return Err("collection length violates the exact contract");
                }
                let ordinal: fn(&$item) -> usize = $ordinal;
                if !values.windows(2).all(|pair| ordinal(&pair[0]) < ordinal(&pair[1])) {
                    return Err("collection is not canonical and duplicate-free");
                }
                Ok(Self(values))
            }

            fn as_slice(&self) -> &[$item] {
                &self.0
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                Self::new(Vec::<$item>::deserialize(deserializer)?).map_err(D::Error::custom)
            }
        }

        impl JsonSchema for $name {
            fn schema_name() -> Cow<'static, str> {
                stringify!($name).into()
            }

            fn json_schema(generator: &mut SchemaGenerator) -> Schema {
                let item = generator.subschema_for::<$item>();
                let minimum = if $non_empty { 1 } else { 0 };
                schemars::json_schema!({
                    "type": "array",
                    "items": item,
                    "minItems": minimum,
                    "maxItems": MAX_RESULT_ITEMS,
                    "uniqueItems": true,
                })
            }
        }
    };
}

fn stable_error_ordinal(value: &StableErrorCode) -> usize {
    StableErrorCode::ALL
        .iter()
        .position(|candidate| candidate == value)
        .expect("closed stable error code has an ordinal")
}

fn task_phase_ordinal(value: &TaskPhase) -> usize {
    TaskPhase::ALL
        .iter()
        .position(|candidate| candidate == value)
        .expect("closed task phase has an ordinal")
}

fn conflict_resolution_ordinal(value: &ConflictResolution) -> usize {
    ConflictResolution::ALL
        .iter()
        .position(|candidate| candidate == value)
        .expect("closed resolution has an ordinal")
}

fn commit_mismatch_ordinal(value: &CommitCommentPolicyMismatchKind) -> usize {
    CommitCommentPolicyMismatchKind::ALL
        .iter()
        .position(|candidate| candidate == value)
        .expect("closed mismatch kind has an ordinal")
}

fn integration_mismatch_ordinal(value: &IntegrationSetMismatchKind) -> usize {
    IntegrationSetMismatchKind::ALL
        .iter()
        .position(|candidate| candidate == value)
        .expect("closed mismatch kind has an ordinal")
}

canonical_list!(
    CanonicalStableCodes,
    StableErrorCode,
    stable_error_ordinal,
    false
);
canonical_list!(
    NonEmptyStableCodes,
    StableErrorCode,
    stable_error_ordinal,
    true
);
canonical_list!(CanonicalTaskPhases, TaskPhase, task_phase_ordinal, false);
canonical_list!(NonEmptyTaskPhases, TaskPhase, task_phase_ordinal, true);
canonical_list!(
    NonEmptyConflictResolutions,
    ConflictResolution,
    conflict_resolution_ordinal,
    true
);
canonical_list!(
    NonEmptyCommitMismatchKinds,
    CommitCommentPolicyMismatchKind,
    commit_mismatch_ordinal,
    true
);
canonical_list!(
    NonEmptyIntegrationMismatchKinds,
    IntegrationSetMismatchKind,
    integration_mismatch_ordinal,
    true
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
struct CanonicalAcceptedArtifactKinds(Vec<ArtifactKindRole>);

impl CanonicalAcceptedArtifactKinds {
    fn ordinal(value: &ArtifactKindRole) -> usize {
        match value {
            ArtifactKindRole::BaselineDistribution(_) => 0,
            ArtifactKindRole::RefreshDistribution(_) => 1,
            ArtifactKindRole::SupportRecoveryDistribution(_) => 2,
            ArtifactKindRole::OrdinaryResult(_) => 3,
        }
    }

    fn new(values: Vec<ArtifactKindRole>) -> Result<Self, &'static str> {
        if values.is_empty() || values.len() > 4 {
            return Err("accepted artifact tuples must be non-empty and bounded");
        }
        if !values
            .windows(2)
            .all(|pair| Self::ordinal(&pair[0]) < Self::ordinal(&pair[1]))
        {
            return Err("accepted artifact tuples are not canonical and duplicate-free");
        }
        Ok(Self(values))
    }

    fn contains_observed(&self, kind: ArtifactKind, role: ArtifactRole) -> bool {
        self.0.iter().any(|tuple| {
            matches!(
                (tuple, kind, role),
                (
                    ArtifactKindRole::BaselineDistribution(_),
                    ArtifactKind::ConfigurationDistribution,
                    ArtifactRole::BaselineDistribution
                ) | (
                    ArtifactKindRole::RefreshDistribution(_),
                    ArtifactKind::ConfigurationDistribution,
                    ArtifactRole::RefreshDistribution
                ) | (
                    ArtifactKindRole::SupportRecoveryDistribution(_),
                    ArtifactKind::ConfigurationDistribution,
                    ArtifactRole::SupportRecoveryDistribution
                ) | (
                    ArtifactKindRole::OrdinaryResult(_),
                    ArtifactKind::OrdinaryConfiguration,
                    ArtifactRole::OrdinaryResult
                )
            )
        })
    }
}

impl<'de> Deserialize<'de> for CanonicalAcceptedArtifactKinds {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::new(Vec::<ArtifactKindRole>::deserialize(deserializer)?).map_err(D::Error::custom)
    }
}

impl JsonSchema for CanonicalAcceptedArtifactKinds {
    fn schema_name() -> Cow<'static, str> {
        "CanonicalAcceptedArtifactKinds".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        let item = generator.subschema_for::<ArtifactKindRole>();
        schemars::json_schema!({
            "type": "array",
            "items": item,
            "minItems": 1,
            "maxItems": 4,
            "uniqueItems": true,
        })
    }
}

macro_rules! exact_empty_list {
    ($name:ident, $item:ty) => {
        #[derive(Debug, Clone, PartialEq, Eq, Serialize)]
        #[serde(transparent)]
        struct $name(Vec<$item>);

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                let values = Vec::<$item>::deserialize(deserializer)?;
                values
                    .is_empty()
                    .then_some(Self(values))
                    .ok_or_else(|| D::Error::custom(concat!(stringify!($name), " must be empty")))
            }
        }

        impl JsonSchema for $name {
            fn schema_name() -> Cow<'static, str> {
                stringify!($name).into()
            }

            fn json_schema(generator: &mut SchemaGenerator) -> Schema {
                let item = generator.subschema_for::<$item>();
                schemars::json_schema!({
                    "type": "array",
                    "items": item,
                    "minItems": 0,
                    "maxItems": 0,
                })
            }
        }
    };
}

exact_empty_list!(EmptyTaskPhases, TaskPhase);
exact_empty_list!(EmptyStableCodes, StableErrorCode);

macro_rules! string_literal {
    ($name:ident, $wire:literal) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
        enum $name {
            #[serde(rename = $wire)]
            Value,
        }
    };
}

macro_rules! boolean_literal {
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

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                let value = bool::deserialize(deserializer)?;
                (value == $value)
                    .then_some(Self)
                    .ok_or_else(|| D::Error::custom("boolean literal mismatch"))
            }
        }

        impl JsonSchema for $name {
            fn schema_name() -> Cow<'static, str> {
                stringify!($name).into()
            }

            fn json_schema(_: &mut SchemaGenerator) -> Schema {
                schemars::json_schema!({ "type": "boolean", "const": $value })
            }
        }
    };
}

boolean_literal!(FalseLiteral, false);
boolean_literal!(TrueLiteral, true);

string_literal!(BindingContextKind, "binding");
string_literal!(ArtifactInputContextKind, "artifactInput");
string_literal!(LifecycleContextKind, "lifecycle");
string_literal!(OperationContextKind, "operation");
string_literal!(TargetReservationContextKind, "targetReservation");
string_literal!(
    RepositoryAccountReservationContextKind,
    "repositoryAccountReservation"
);
string_literal!(DigestContextKind, "digest");
string_literal!(AdaptationDecisionContextKind, "adaptationDecision");
string_literal!(ConflictResolutionContextKind, "conflictResolution");
string_literal!(CommitCommentPolicyContextKind, "commitCommentPolicy");
string_literal!(IntegrationSetContextKind, "integrationSet");
string_literal!(CapabilityContextKind, "capability");
string_literal!(ProfileStateContextKind, "profileState");
string_literal!(StateCorruptContextKind, "stateCorrupt");
string_literal!(TaskContextKind, "taskContext");

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RepositoryBindingMismatchContext {
    context_kind: BindingContextKind,
    expected_binding_digest: Sha256Digest,
    observed_binding_digest: Sha256Digest,
    #[serde(skip_serializing_if = "Option::is_none")]
    original_fingerprint: Option<Sha256Digest>,
    #[serde(skip_serializing_if = "Option::is_none")]
    repository_fingerprint: Option<Sha256Digest>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MainDiffersFromRepositoryContext {
    context_kind: BindingContextKind,
    expected_binding_digest: Sha256Digest,
    observed_binding_digest: Sha256Digest,
    original_fingerprint: Sha256Digest,
    repository_fingerprint: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ArtifactInputErrorContext {
    context_kind: ArtifactInputContextKind,
    artifact_id: UnicaId,
    observed_kind: ArtifactKind,
    observed_role: ArtifactRole,
    accepted_inputs: CanonicalAcceptedArtifactKinds,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BlockedLifecycleErrorContext {
    context_kind: LifecycleContextKind,
    phase: TaskPhase,
    allowed_phases: EmptyTaskPhases,
    blocker_codes: NonEmptyStableCodes,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RecoveryPlanPendingContext<CancellationLiteral> {
    context_kind: LifecycleContextKind,
    phase: TaskPhase,
    allowed_phases: EmptyTaskPhases,
    blocker_codes: EmptyStableCodes,
    recovery_digest: Sha256Digest,
    recovery_cancellation_allowed: CancellationLiteral,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TaskPhaseMismatchContext {
    context_kind: LifecycleContextKind,
    phase: TaskPhase,
    allowed_phases: NonEmptyTaskPhases,
    blocker_codes: EmptyStableCodes,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct OperationReplayMismatchContext {
    context_kind: OperationContextKind,
    operation_id: OperationId,
    expected_input_digest: Sha256Digest,
    observed_input_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct OperationInProgressContext {
    context_kind: OperationContextKind,
    operation_id: OperationId,
    active_operation_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
enum ReservationOwnerRef {
    StartAttempt(StartAttemptReservationOwnerRef),
    UnresolvedTask(UnresolvedTaskReservationOwnerRef),
}

impl JsonSchema for ReservationOwnerRef {
    fn schema_name() -> Cow<'static, str> {
        "ReservationOwnerRef".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<StartAttemptReservationOwnerRef>(),
            generator.subschema_for::<UnresolvedTaskReservationOwnerRef>(),
        ])
    }
}

string_literal!(StartAttemptOwnerKind, "startAttempt");
string_literal!(UnresolvedTaskOwnerKind, "unresolvedTask");

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StartAttemptReservationOwnerRef {
    owner_kind: StartAttemptOwnerKind,
    project_id: ProjectId,
    task_id: TaskId,
    operation_id: OperationId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UnresolvedTaskReservationOwnerRef {
    owner_kind: UnresolvedTaskOwnerKind,
    project_id: ProjectId,
    task_id: TaskId,
    instance_id: UnicaId,
    phase: TaskPhase,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TargetReservationBusyContext {
    context_kind: TargetReservationContextKind,
    repository_identity_digest: Sha256Digest,
    original_infobase_identity_digest: Sha256Digest,
    reservation_key_digest: Sha256Digest,
    owner: ReservationOwnerRef,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RepositoryAccountReservationBusyContext {
    context_kind: RepositoryAccountReservationContextKind,
    repository_identity_digest: Sha256Digest,
    normalized_username_digest: Sha256Digest,
    reservation_key_digest: Sha256Digest,
    owner: ReservationOwnerRef,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DigestErrorContext {
    context_kind: DigestContextKind,
    expected_digest: Sha256Digest,
    observed_digest: Sha256Digest,
    producer_id: UnicaId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AdaptationDecisionConflictContext {
    context_kind: AdaptationDecisionContextKind,
    verification_id: UnicaId,
    existing_decision_id: UnicaId,
    existing_adaptation_decision_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ConflictResolutionErrorContext {
    context_kind: ConflictResolutionContextKind,
    session_id: UnicaId,
    conflict_id: UnicaId,
    conflict_kind: ConflictKind,
    requested_resolution: ConflictResolution,
    allowed_resolutions: NonEmptyConflictResolutions,
}

string_literal!(MainValidatedPhase, "mainValidated");

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CommitCommentPolicyErrorContext {
    context_kind: CommitCommentPolicyContextKind,
    phase: MainValidatedPhase,
    expected_policy_digest: Sha256Digest,
    observed_policy_digest: Sha256Digest,
    mismatch_kinds: NonEmptyCommitMismatchKinds,
}

string_literal!(LockedPhase, "locked");
string_literal!(RecoveryRequiredPhase, "recoveryRequired");
string_literal!(UnlockExitKind, "unlock");
string_literal!(RecoveryExitKind, "recovery");

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct IntegrationSetUnlockContext {
    context_kind: IntegrationSetContextKind,
    phase: LockedPhase,
    expected_lineage_digest: Sha256Digest,
    observed_lineage_digest: Sha256Digest,
    mismatch_kinds: NonEmptyIntegrationMismatchKinds,
    exit_kind: UnlockExitKind,
    lock_set_id: UnicaId,
    expected_lock_set_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct IntegrationSetRecoveryContext {
    context_kind: IntegrationSetContextKind,
    phase: RecoveryRequiredPhase,
    expected_lineage_digest: Sha256Digest,
    observed_lineage_digest: Sha256Digest,
    mismatch_kinds: NonEmptyIntegrationMismatchKinds,
    exit_kind: RecoveryExitKind,
    recovery_digest: Sha256Digest,
}

string_literal!(PlatformCapabilityKind, "platform");
string_literal!(SupportLayerCapabilityKind, "supportLayer");
string_literal!(
    RepositoryUserExclusivityCapabilityKind,
    "repositoryUserExclusivity"
);
string_literal!(ChangeSemanticsCapabilityKind, "changeSemantics");

macro_rules! capability_context {
    ($name:ident, $kind:ty) => {
        #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct $name {
            context_kind: CapabilityContextKind,
            capability_kind: $kind,
            #[serde(skip_serializing_if = "Option::is_none")]
            capability_row_id: Option<CapabilityRowId>,
            evidence_digest: Sha256Digest,
        }
    };
}

capability_context!(PlatformCapabilityContext, PlatformCapabilityKind);
capability_context!(SupportLayerCapabilityContext, SupportLayerCapabilityKind);
capability_context!(
    RepositoryUserExclusivityCapabilityContext,
    RepositoryUserExclusivityCapabilityKind
);
capability_context!(
    ChangeSemanticsCapabilityContext,
    ChangeSemanticsCapabilityKind
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ProjectDigestProfileStateContext {
    context_kind: ProfileStateContextKind,
    project_id: ProjectId,
    expected_digest: Sha256Digest,
    observed_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ProfilePropertyStateContext {
    context_kind: ProfileStateContextKind,
    profile: LocalProfileName,
    property_path: PropertyPath,
}

string_literal!(WorkspaceStateRefKind, "workspace");
string_literal!(StartAttemptStateRefKind, "startAttempt");
string_literal!(ProjectStateRefKind, "project");
string_literal!(TaskStateRefKind, "task");
string_literal!(TaskOperationStateRefKind, "taskOperation");
string_literal!(ExactBytesObservationKind, "exactBytes");
string_literal!(UnavailableObservationKind, "unavailable");

closed_string_enum!(StateCorruptUnavailableReason {
    Missing => "missing",
    PermissionDenied => "permissionDenied",
});

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WorkspaceStateCorruptStateRef {
    state_ref_kind: WorkspaceStateRefKind,
    workspace_identity_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StartAttemptStateCorruptStateRef {
    state_ref_kind: StartAttemptStateRefKind,
    workspace_identity_digest: Sha256Digest,
    task_id: TaskId,
    operation_id: OperationId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ProjectStateCorruptStateRef {
    state_ref_kind: ProjectStateRefKind,
    project_id: ProjectId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TaskStateCorruptStateRef {
    state_ref_kind: TaskStateRefKind,
    project_id: ProjectId,
    task_id: TaskId,
    instance_id: UnicaId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TaskOperationStateCorruptStateRef {
    state_ref_kind: TaskOperationStateRefKind,
    project_id: ProjectId,
    task_id: TaskId,
    instance_id: UnicaId,
    operation_id: OperationId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
enum StateCorruptStateRef {
    Workspace(WorkspaceStateCorruptStateRef),
    StartAttempt(StartAttemptStateCorruptStateRef),
    Project(ProjectStateCorruptStateRef),
    Task(TaskStateCorruptStateRef),
    TaskOperation(TaskOperationStateCorruptStateRef),
}

impl JsonSchema for StateCorruptStateRef {
    fn schema_name() -> Cow<'static, str> {
        "StateCorruptStateRef".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<WorkspaceStateCorruptStateRef>(),
            generator.subschema_for::<StartAttemptStateCorruptStateRef>(),
            generator.subschema_for::<ProjectStateCorruptStateRef>(),
            generator.subschema_for::<TaskStateCorruptStateRef>(),
            generator.subschema_for::<TaskOperationStateCorruptStateRef>(),
        ])
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StateCorruptExactBytesObservation {
    observation_kind: ExactBytesObservationKind,
    observed_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StateCorruptUnavailableObservation {
    observation_kind: UnavailableObservationKind,
    reason: StateCorruptUnavailableReason,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
enum StateCorruptObservation {
    ExactBytes(StateCorruptExactBytesObservation),
    Unavailable(StateCorruptUnavailableObservation),
}

impl JsonSchema for StateCorruptObservation {
    fn schema_name() -> Cow<'static, str> {
        "StateCorruptObservation".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<StateCorruptExactBytesObservation>(),
            generator.subschema_for::<StateCorruptUnavailableObservation>(),
        ])
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StateCorruptErrorContext {
    context_kind: StateCorruptContextKind,
    state_ref: StateCorruptStateRef,
    expected_digest: Sha256Digest,
    observation: StateCorruptObservation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BasicTaskContextErrorContext {
    context_kind: TaskContextKind,
    requested_task_id: TaskId,
    requested_tool_name: IncomingToolName,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
struct RequiredNullDigest(RequiredNullable<Sha256Digest>);

impl RequiredNullDigest {
    fn deserialize_required<'de, D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = RequiredNullable::<Sha256Digest>::deserialize_required(deserializer)?;
        value
            .as_ref()
            .is_none()
            .then_some(Self(value))
            .ok_or_else(|| D::Error::custom("missing-observation digest must be explicit null"))
    }
}

impl<'de> Deserialize<'de> for RequiredNullDigest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::deserialize_required(deserializer)
    }
}

impl JsonSchema for RequiredNullDigest {
    fn schema_name() -> Cow<'static, str> {
        "RequiredNullDigest".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        schemars::json_schema!({ "type": "null" })
    }
}

macro_rules! exact_workspace_mismatch_list {
    ($name:ident, [$($variant:ident),+ $(,)?]) => {
        #[derive(Debug, Clone, PartialEq, Eq, Serialize)]
        #[serde(transparent)]
        struct $name(Vec<WorkspaceMismatchKind>);

        impl $name {
            fn exact() -> Vec<WorkspaceMismatchKind> {
                vec![$(WorkspaceMismatchKind::$variant),+]
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                let values = Vec::<WorkspaceMismatchKind>::deserialize(deserializer)?;
                (values == Self::exact())
                    .then_some(Self(values))
                    .ok_or_else(|| D::Error::custom(concat!(stringify!($name), " is not exact")))
            }
        }

        impl JsonSchema for $name {
            fn schema_name() -> Cow<'static, str> {
                stringify!($name).into()
            }

            fn json_schema(_: &mut SchemaGenerator) -> Schema {
                let prefix_items = vec![$(
                    {
                        let literal = WorkspaceMismatchKind::$variant.as_str();
                        schemars::json_schema!({ "type": "string", "const": literal })
                    }
                ),+];
                let length = prefix_items.len();
                schemars::json_schema!({
                    "type": "array",
                    "prefixItems": prefix_items,
                    "items": false,
                    "minItems": length,
                    "maxItems": length,
                })
            }
        }
    };
}

exact_workspace_mismatch_list!(LeaseMissingKinds, [LeaseMissing]);
exact_workspace_mismatch_list!(LeaseInvalidKinds, [LeaseInvalid]);
exact_workspace_mismatch_list!(MarkerMissingKinds, [MarkerMissing]);
exact_workspace_mismatch_list!(
    MarkerMissingLeaseMissingKinds,
    [MarkerMissing, LeaseMissing]
);
exact_workspace_mismatch_list!(
    MarkerMissingLeaseInvalidKinds,
    [MarkerMissing, LeaseInvalid]
);
exact_workspace_mismatch_list!(MarkerMismatchKinds, [MarkerMismatch]);
exact_workspace_mismatch_list!(
    MarkerMismatchLeaseMissingKinds,
    [MarkerMismatch, LeaseMissing]
);
exact_workspace_mismatch_list!(
    MarkerMismatchLeaseInvalidKinds,
    [MarkerMismatch, LeaseInvalid]
);
exact_workspace_mismatch_list!(ProjectMismatchKinds, [ProjectMismatch]);
exact_workspace_mismatch_list!(ProjectLeaseMissingKinds, [ProjectMismatch, LeaseMissing]);
exact_workspace_mismatch_list!(ProjectLeaseInvalidKinds, [ProjectMismatch, LeaseInvalid]);
exact_workspace_mismatch_list!(ProjectMarkerMissingKinds, [ProjectMismatch, MarkerMissing]);
exact_workspace_mismatch_list!(
    ProjectMarkerMissingLeaseMissingKinds,
    [ProjectMismatch, MarkerMissing, LeaseMissing]
);
exact_workspace_mismatch_list!(
    ProjectMarkerMissingLeaseInvalidKinds,
    [ProjectMismatch, MarkerMissing, LeaseInvalid]
);
exact_workspace_mismatch_list!(
    ProjectMarkerMismatchKinds,
    [ProjectMismatch, MarkerMismatch]
);
exact_workspace_mismatch_list!(
    ProjectMarkerMismatchLeaseMissingKinds,
    [ProjectMismatch, MarkerMismatch, LeaseMissing]
);
exact_workspace_mismatch_list!(
    ProjectMarkerMismatchLeaseInvalidKinds,
    [ProjectMismatch, MarkerMismatch, LeaseInvalid]
);

macro_rules! workspace_context {
    ($name:ident, $mismatch_kinds:ty, { $($field:tt)* }) => {
        #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct $name {
            context_kind: TaskContextKind,
            requested_task_id: TaskId,
            requested_tool_name: IncomingToolName,
            mismatch_kinds: $mismatch_kinds,
            $($field)*
        }
    };
}

workspace_context!(LeaseMissingWorkspaceContext, LeaseMissingKinds, {
    expected_lease_digest: Sha256Digest,
    #[serde(deserialize_with = "RequiredNullDigest::deserialize_required")]
    observed_lease_digest: RequiredNullDigest,
});
workspace_context!(LeaseInvalidWorkspaceContext, LeaseInvalidKinds, {
    expected_lease_digest: Sha256Digest,
    observed_lease_digest: Sha256Digest,
});
workspace_context!(MarkerMissingWorkspaceContext, MarkerMissingKinds, {
    expected_marker_digest: Sha256Digest,
    #[serde(deserialize_with = "RequiredNullDigest::deserialize_required")]
    observed_marker_digest: RequiredNullDigest,
});
workspace_context!(MarkerMissingLeaseMissingWorkspaceContext, MarkerMissingLeaseMissingKinds, {
    expected_marker_digest: Sha256Digest,
    #[serde(deserialize_with = "RequiredNullDigest::deserialize_required")]
    observed_marker_digest: RequiredNullDigest,
    expected_lease_digest: Sha256Digest,
    #[serde(deserialize_with = "RequiredNullDigest::deserialize_required")]
    observed_lease_digest: RequiredNullDigest,
});
workspace_context!(MarkerMissingLeaseInvalidWorkspaceContext, MarkerMissingLeaseInvalidKinds, {
    expected_marker_digest: Sha256Digest,
    #[serde(deserialize_with = "RequiredNullDigest::deserialize_required")]
    observed_marker_digest: RequiredNullDigest,
    expected_lease_digest: Sha256Digest,
    observed_lease_digest: Sha256Digest,
});
workspace_context!(MarkerMismatchWorkspaceContext, MarkerMismatchKinds, {
    expected_marker_digest: Sha256Digest,
    observed_marker_digest: Sha256Digest,
});
workspace_context!(MarkerMismatchLeaseMissingWorkspaceContext, MarkerMismatchLeaseMissingKinds, {
    expected_marker_digest: Sha256Digest,
    observed_marker_digest: Sha256Digest,
    expected_lease_digest: Sha256Digest,
    #[serde(deserialize_with = "RequiredNullDigest::deserialize_required")]
    observed_lease_digest: RequiredNullDigest,
});
workspace_context!(MarkerMismatchLeaseInvalidWorkspaceContext, MarkerMismatchLeaseInvalidKinds, {
    expected_marker_digest: Sha256Digest,
    observed_marker_digest: Sha256Digest,
    expected_lease_digest: Sha256Digest,
    observed_lease_digest: Sha256Digest,
});
workspace_context!(ProjectMismatchWorkspaceContext, ProjectMismatchKinds, {
    expected_project_id: ProjectId,
    observed_project_id: ProjectId,
});
workspace_context!(ProjectLeaseMissingWorkspaceContext, ProjectLeaseMissingKinds, {
    expected_project_id: ProjectId,
    observed_project_id: ProjectId,
    expected_lease_digest: Sha256Digest,
    #[serde(deserialize_with = "RequiredNullDigest::deserialize_required")]
    observed_lease_digest: RequiredNullDigest,
});
workspace_context!(ProjectLeaseInvalidWorkspaceContext, ProjectLeaseInvalidKinds, {
    expected_project_id: ProjectId,
    observed_project_id: ProjectId,
    expected_lease_digest: Sha256Digest,
    observed_lease_digest: Sha256Digest,
});
workspace_context!(ProjectMarkerMissingWorkspaceContext, ProjectMarkerMissingKinds, {
    expected_project_id: ProjectId,
    observed_project_id: ProjectId,
    expected_marker_digest: Sha256Digest,
    #[serde(deserialize_with = "RequiredNullDigest::deserialize_required")]
    observed_marker_digest: RequiredNullDigest,
});
workspace_context!(ProjectMarkerMissingLeaseMissingWorkspaceContext, ProjectMarkerMissingLeaseMissingKinds, {
    expected_project_id: ProjectId,
    observed_project_id: ProjectId,
    expected_marker_digest: Sha256Digest,
    #[serde(deserialize_with = "RequiredNullDigest::deserialize_required")]
    observed_marker_digest: RequiredNullDigest,
    expected_lease_digest: Sha256Digest,
    #[serde(deserialize_with = "RequiredNullDigest::deserialize_required")]
    observed_lease_digest: RequiredNullDigest,
});
workspace_context!(ProjectMarkerMissingLeaseInvalidWorkspaceContext, ProjectMarkerMissingLeaseInvalidKinds, {
    expected_project_id: ProjectId,
    observed_project_id: ProjectId,
    expected_marker_digest: Sha256Digest,
    #[serde(deserialize_with = "RequiredNullDigest::deserialize_required")]
    observed_marker_digest: RequiredNullDigest,
    expected_lease_digest: Sha256Digest,
    observed_lease_digest: Sha256Digest,
});
workspace_context!(ProjectMarkerMismatchWorkspaceContext, ProjectMarkerMismatchKinds, {
    expected_project_id: ProjectId,
    observed_project_id: ProjectId,
    expected_marker_digest: Sha256Digest,
    observed_marker_digest: Sha256Digest,
});
workspace_context!(ProjectMarkerMismatchLeaseMissingWorkspaceContext, ProjectMarkerMismatchLeaseMissingKinds, {
    expected_project_id: ProjectId,
    observed_project_id: ProjectId,
    expected_marker_digest: Sha256Digest,
    observed_marker_digest: Sha256Digest,
    expected_lease_digest: Sha256Digest,
    #[serde(deserialize_with = "RequiredNullDigest::deserialize_required")]
    observed_lease_digest: RequiredNullDigest,
});
workspace_context!(ProjectMarkerMismatchLeaseInvalidWorkspaceContext, ProjectMarkerMismatchLeaseInvalidKinds, {
    expected_project_id: ProjectId,
    observed_project_id: ProjectId,
    expected_marker_digest: Sha256Digest,
    observed_marker_digest: Sha256Digest,
    expected_lease_digest: Sha256Digest,
    observed_lease_digest: Sha256Digest,
});

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
enum TaskWorkspaceContextInvalidContext {
    LeaseMissing(LeaseMissingWorkspaceContext),
    LeaseInvalid(LeaseInvalidWorkspaceContext),
    MarkerMissing(MarkerMissingWorkspaceContext),
    MarkerMissingLeaseMissing(MarkerMissingLeaseMissingWorkspaceContext),
    MarkerMissingLeaseInvalid(MarkerMissingLeaseInvalidWorkspaceContext),
    MarkerMismatch(MarkerMismatchWorkspaceContext),
    MarkerMismatchLeaseMissing(MarkerMismatchLeaseMissingWorkspaceContext),
    MarkerMismatchLeaseInvalid(MarkerMismatchLeaseInvalidWorkspaceContext),
    ProjectMismatch(ProjectMismatchWorkspaceContext),
    ProjectLeaseMissing(ProjectLeaseMissingWorkspaceContext),
    ProjectLeaseInvalid(ProjectLeaseInvalidWorkspaceContext),
    ProjectMarkerMissing(ProjectMarkerMissingWorkspaceContext),
    ProjectMarkerMissingLeaseMissing(ProjectMarkerMissingLeaseMissingWorkspaceContext),
    ProjectMarkerMissingLeaseInvalid(ProjectMarkerMissingLeaseInvalidWorkspaceContext),
    ProjectMarkerMismatch(ProjectMarkerMismatchWorkspaceContext),
    ProjectMarkerMismatchLeaseMissing(ProjectMarkerMismatchLeaseMissingWorkspaceContext),
    ProjectMarkerMismatchLeaseInvalid(ProjectMarkerMismatchLeaseInvalidWorkspaceContext),
}

impl JsonSchema for TaskWorkspaceContextInvalidContext {
    fn schema_name() -> Cow<'static, str> {
        "TaskWorkspaceContextInvalidContext".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<LeaseMissingWorkspaceContext>(),
            generator.subschema_for::<LeaseInvalidWorkspaceContext>(),
            generator.subschema_for::<MarkerMissingWorkspaceContext>(),
            generator.subschema_for::<MarkerMissingLeaseMissingWorkspaceContext>(),
            generator.subschema_for::<MarkerMissingLeaseInvalidWorkspaceContext>(),
            generator.subschema_for::<MarkerMismatchWorkspaceContext>(),
            generator.subschema_for::<MarkerMismatchLeaseMissingWorkspaceContext>(),
            generator.subschema_for::<MarkerMismatchLeaseInvalidWorkspaceContext>(),
            generator.subschema_for::<ProjectMismatchWorkspaceContext>(),
            generator.subschema_for::<ProjectLeaseMissingWorkspaceContext>(),
            generator.subschema_for::<ProjectLeaseInvalidWorkspaceContext>(),
            generator.subschema_for::<ProjectMarkerMissingWorkspaceContext>(),
            generator.subschema_for::<ProjectMarkerMissingLeaseMissingWorkspaceContext>(),
            generator.subschema_for::<ProjectMarkerMissingLeaseInvalidWorkspaceContext>(),
            generator.subschema_for::<ProjectMarkerMismatchWorkspaceContext>(),
            generator.subschema_for::<ProjectMarkerMismatchLeaseMissingWorkspaceContext>(),
            generator.subschema_for::<ProjectMarkerMismatchLeaseInvalidWorkspaceContext>(),
        ])
    }
}

macro_rules! rejected_leaf {
    ($name:ident, $code_type:ident, $wire:literal, $context:ty, $actions:ty) => {
        string_literal!($code_type, $wire);

        #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct $name {
            code: $code_type,
            context: $context,
            allowed_next_actions: $actions,
        }
    };
}

rejected_leaf!(
    RepositoryBindingMismatchErrorData,
    RepositoryBindingMismatchCode,
    "repositoryBindingMismatch",
    RepositoryBindingMismatchContext,
    StatusOnlyActions
);
rejected_leaf!(
    MainDiffersFromRepositoryErrorData,
    MainDiffersFromRepositoryCode,
    "mainDiffersFromRepository",
    MainDiffersFromRepositoryContext,
    StatusOnlyActions
);
rejected_leaf!(
    ArtifactNotDistributionErrorData,
    ArtifactNotDistributionCode,
    "artifactNotDistribution",
    ArtifactInputErrorContext,
    StatusOnlyActions
);
rejected_leaf!(
    CleanupNotAllowedErrorData,
    CleanupNotAllowedCode,
    "cleanupNotAllowed",
    BlockedLifecycleErrorContext,
    StatusOnlyActions
);
rejected_leaf!(
    TaskAbandonmentNotSafeErrorData,
    TaskAbandonmentNotSafeCode,
    "taskAbandonmentNotSafe",
    BlockedLifecycleErrorContext,
    StatusOnlyActions
);

string_literal!(RecoveryPlanPendingCode, "recoveryPlanPending");

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RecoveryPlanPendingApplyOnlyErrorData {
    code: RecoveryPlanPendingCode,
    context: RecoveryPlanPendingContext<FalseLiteral>,
    allowed_next_actions: RecoveryApplyActions,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RecoveryPlanPendingCancelableErrorData {
    code: RecoveryPlanPendingCode,
    context: RecoveryPlanPendingContext<TrueLiteral>,
    allowed_next_actions: RecoveryApplyOrCancelActions,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
enum RecoveryPlanPendingErrorData {
    ApplyOnly(RecoveryPlanPendingApplyOnlyErrorData),
    Cancelable(RecoveryPlanPendingCancelableErrorData),
}

impl JsonSchema for RecoveryPlanPendingErrorData {
    fn schema_name() -> Cow<'static, str> {
        "RecoveryPlanPendingErrorData".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<RecoveryPlanPendingApplyOnlyErrorData>(),
            generator.subschema_for::<RecoveryPlanPendingCancelableErrorData>(),
        ])
    }
}

rejected_leaf!(
    TaskPhaseMismatchErrorData,
    TaskPhaseMismatchCode,
    "taskPhaseMismatch",
    TaskPhaseMismatchContext,
    StatusOnlyActions
);
rejected_leaf!(
    TaskMutationBlockedErrorData,
    TaskMutationBlockedCode,
    "taskMutationBlocked",
    BlockedLifecycleErrorContext,
    StatusOnlyActions
);
rejected_leaf!(
    OperationReplayMismatchErrorData,
    OperationReplayMismatchCode,
    "operationReplayMismatch",
    OperationReplayMismatchContext,
    NoNextActions
);
rejected_leaf!(
    OperationInProgressErrorData,
    OperationInProgressCode,
    "operationInProgress",
    OperationInProgressContext,
    StatusOnlyActions
);
rejected_leaf!(
    TargetReservationBusyErrorData,
    TargetReservationBusyCode,
    "targetReservationBusy",
    TargetReservationBusyContext,
    StatusOnlyActions
);
rejected_leaf!(
    RepositoryAccountReservationBusyErrorData,
    RepositoryAccountReservationBusyCode,
    "repositoryAccountReservationBusy",
    RepositoryAccountReservationBusyContext,
    StatusOnlyActions
);
rejected_leaf!(
    ApprovalDigestMismatchErrorData,
    ApprovalDigestMismatchCode,
    "approvalDigestMismatch",
    DigestErrorContext,
    StatusOnlyActions
);
rejected_leaf!(
    ChangeReceiptStaleErrorData,
    ChangeReceiptStaleCode,
    "changeReceiptStale",
    DigestErrorContext,
    StatusOnlyActions
);
rejected_leaf!(
    AdaptationDecisionAlreadyRecordedErrorData,
    AdaptationDecisionAlreadyRecordedCode,
    "adaptationDecisionAlreadyRecorded",
    AdaptationDecisionConflictContext,
    AdaptationRefreshActions
);
rejected_leaf!(
    ConflictResolutionNotAllowedErrorData,
    ConflictResolutionNotAllowedCode,
    "conflictResolutionNotAllowed",
    ConflictResolutionErrorContext,
    ConflictReviewActions
);
rejected_leaf!(
    CommitCommentPolicyMismatchErrorData,
    CommitCommentPolicyMismatchCode,
    "commitCommentPolicyMismatch",
    CommitCommentPolicyErrorContext,
    CommitSafeExitActions
);

string_literal!(IntegrationSetMismatchCode, "integrationSetMismatch");

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct IntegrationSetUnlockErrorData {
    code: IntegrationSetMismatchCode,
    context: IntegrationSetUnlockContext,
    allowed_next_actions: IntegrationUnlockExitActions,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct IntegrationSetRecoveryErrorData {
    code: IntegrationSetMismatchCode,
    context: IntegrationSetRecoveryContext,
    allowed_next_actions: IntegrationRecoveryExitActions,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
enum IntegrationSetMismatchErrorData {
    Unlock(IntegrationSetUnlockErrorData),
    Recovery(IntegrationSetRecoveryErrorData),
}

impl JsonSchema for IntegrationSetMismatchErrorData {
    fn schema_name() -> Cow<'static, str> {
        "IntegrationSetMismatchErrorData".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<IntegrationSetUnlockErrorData>(),
            generator.subschema_for::<IntegrationSetRecoveryErrorData>(),
        ])
    }
}

rejected_leaf!(
    PlatformCapabilityUnprovenErrorData,
    PlatformCapabilityUnprovenCode,
    "platformCapabilityUnproven",
    PlatformCapabilityContext,
    StatusOnlyActions
);
rejected_leaf!(
    SupportLayerAmbiguousErrorData,
    SupportLayerAmbiguousCode,
    "supportLayerAmbiguous",
    SupportLayerCapabilityContext,
    StatusOnlyActions
);
rejected_leaf!(
    UnsupportedChangeKindErrorData,
    UnsupportedChangeKindCode,
    "unsupportedChangeKind",
    ChangeSemanticsCapabilityContext,
    StatusOnlyActions
);
rejected_leaf!(
    ExclusiveRepositoryUserRequiredErrorData,
    ExclusiveRepositoryUserRequiredCode,
    "exclusiveRepositoryUserRequired",
    RepositoryUserExclusivityCapabilityContext,
    StatusOnlyActions
);
rejected_leaf!(
    ProjectIdentityCollisionErrorData,
    ProjectIdentityCollisionCode,
    "projectIdentityCollision",
    ProjectDigestProfileStateContext,
    StatusOnlyActions
);
rejected_leaf!(
    StateRootRelocationRequiredErrorData,
    StateRootRelocationRequiredCode,
    "stateRootRelocationRequired",
    ProjectDigestProfileStateContext,
    StatusOnlyActions
);
rejected_leaf!(
    StateCorruptErrorData,
    StateCorruptCode,
    "stateCorrupt",
    StateCorruptErrorContext,
    StatusOnlyActions
);
rejected_leaf!(
    ProfileInvalidErrorData,
    ProfileInvalidCode,
    "profileInvalid",
    ProfilePropertyStateContext,
    StatusOnlyActions
);
rejected_leaf!(
    SecretUnavailableErrorData,
    SecretUnavailableCode,
    "secretUnavailable",
    ProfilePropertyStateContext,
    StatusOnlyActions
);
rejected_leaf!(
    TaskNotFoundErrorData,
    TaskNotFoundCode,
    "taskNotFound",
    BasicTaskContextErrorContext,
    StartAndStatusActions
);
rejected_leaf!(
    TaskWorkspaceContextInvalidErrorData,
    TaskWorkspaceContextInvalidCode,
    "taskWorkspaceContextInvalid",
    TaskWorkspaceContextInvalidContext,
    NoNextActions
);
rejected_leaf!(
    ToolNotBranchedCompatibleErrorData,
    ToolNotBranchedCompatibleCode,
    "toolNotBranchedCompatible",
    BasicTaskContextErrorContext,
    NoNextActions
);

macro_rules! rejected_code_markers {
    ($($marker:ty => $data:ty),+ $(,)?) => {
        $(
            impl RejectedCodeMarker for $marker {
                fn data_schema(generator: &mut SchemaGenerator) -> Schema {
                    generator.subschema_for::<$data>()
                }
            }
        )+
    };
}

rejected_code_markers!(
    RepositoryBindingMismatchMarker => RepositoryBindingMismatchErrorData,
    MainDiffersFromRepositoryMarker => MainDiffersFromRepositoryErrorData,
    ArtifactNotDistributionMarker => ArtifactNotDistributionErrorData,
    CleanupNotAllowedMarker => CleanupNotAllowedErrorData,
    TaskAbandonmentNotSafeMarker => TaskAbandonmentNotSafeErrorData,
    OperationReplayMismatchMarker => OperationReplayMismatchErrorData,
    RecoveryPlanPendingMarker => RecoveryPlanPendingErrorData,
    TaskPhaseMismatchMarker => TaskPhaseMismatchErrorData,
    ApprovalDigestMismatchMarker => ApprovalDigestMismatchErrorData,
    ChangeReceiptStaleMarker => ChangeReceiptStaleErrorData,
    ConflictResolutionNotAllowedMarker => ConflictResolutionNotAllowedErrorData,
    AdaptationDecisionAlreadyRecordedMarker => AdaptationDecisionAlreadyRecordedErrorData,
    TaskMutationBlockedMarker => TaskMutationBlockedErrorData,
    PlatformCapabilityUnprovenMarker => PlatformCapabilityUnprovenErrorData,
    SupportLayerAmbiguousMarker => SupportLayerAmbiguousErrorData,
    UnsupportedChangeKindMarker => UnsupportedChangeKindErrorData,
    ProjectIdentityCollisionMarker => ProjectIdentityCollisionErrorData,
    StateRootRelocationRequiredMarker => StateRootRelocationRequiredErrorData,
    ExclusiveRepositoryUserRequiredMarker => ExclusiveRepositoryUserRequiredErrorData,
    TargetReservationBusyMarker => TargetReservationBusyErrorData,
    RepositoryAccountReservationBusyMarker => RepositoryAccountReservationBusyErrorData,
    ProfileInvalidMarker => ProfileInvalidErrorData,
    SecretUnavailableMarker => SecretUnavailableErrorData,
    StateCorruptMarker => StateCorruptErrorData,
    OperationInProgressMarker => OperationInProgressErrorData,
    TaskNotFoundMarker => TaskNotFoundErrorData,
    TaskWorkspaceContextInvalidMarker => TaskWorkspaceContextInvalidErrorData,
    ToolNotBranchedCompatibleMarker => ToolNotBranchedCompatibleErrorData,
    CommitCommentPolicyMismatchMarker => CommitCommentPolicyMismatchErrorData,
    IntegrationSetMismatchMarker => IntegrationSetMismatchErrorData,
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
enum BindingRejectedData {
    RepositoryBindingMismatch(RepositoryBindingMismatchErrorData),
    MainDiffersFromRepository(MainDiffersFromRepositoryErrorData),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
enum LifecycleRejectedData {
    CleanupNotAllowed(CleanupNotAllowedErrorData),
    TaskAbandonmentNotSafe(TaskAbandonmentNotSafeErrorData),
    RecoveryPlanPending(RecoveryPlanPendingErrorData),
    TaskPhaseMismatch(TaskPhaseMismatchErrorData),
    TaskMutationBlocked(TaskMutationBlockedErrorData),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
enum OperationRejectedData {
    OperationReplayMismatch(OperationReplayMismatchErrorData),
    OperationInProgress(OperationInProgressErrorData),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
enum ReservationRejectedData {
    TargetReservationBusy(TargetReservationBusyErrorData),
    RepositoryAccountReservationBusy(RepositoryAccountReservationBusyErrorData),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
enum DigestRejectedData {
    ApprovalDigestMismatch(ApprovalDigestMismatchErrorData),
    ChangeReceiptStale(ChangeReceiptStaleErrorData),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
enum CapabilityRejectedData {
    PlatformCapabilityUnproven(PlatformCapabilityUnprovenErrorData),
    SupportLayerAmbiguous(SupportLayerAmbiguousErrorData),
    UnsupportedChangeKind(UnsupportedChangeKindErrorData),
    ExclusiveRepositoryUserRequired(ExclusiveRepositoryUserRequiredErrorData),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
enum ProfileStateRejectedData {
    ProjectIdentityCollision(ProjectIdentityCollisionErrorData),
    StateRootRelocationRequired(StateRootRelocationRequiredErrorData),
    ProfileInvalid(ProfileInvalidErrorData),
    SecretUnavailable(SecretUnavailableErrorData),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
enum TaskContextRejectedData {
    TaskNotFound(TaskNotFoundErrorData),
    TaskWorkspaceContextInvalid(TaskWorkspaceContextInvalidErrorData),
    ToolNotBranchedCompatible(ToolNotBranchedCompatibleErrorData),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[allow(clippy::enum_variant_names)] // variants intentionally mirror the normative group names
enum TaskErrorGroupKind {
    BindingRejected,
    ArtifactRejected,
    LifecycleRejected,
    OperationRejected,
    ReservationRejected,
    DigestRejected,
    AdaptationDecisionRejected,
    ConflictResolutionRejected,
    CommitPolicyRejected,
    IntegrationSetRejected,
    CapabilityRejected,
    ProfileStateRejected,
    StateCorruptRejected,
    TaskContextRejected,
}

impl TaskErrorGroupKind {
    const ALL: &'static [Self] = &[
        Self::BindingRejected,
        Self::ArtifactRejected,
        Self::LifecycleRejected,
        Self::OperationRejected,
        Self::ReservationRejected,
        Self::DigestRejected,
        Self::AdaptationDecisionRejected,
        Self::ConflictResolutionRejected,
        Self::CommitPolicyRejected,
        Self::IntegrationSetRejected,
        Self::CapabilityRejected,
        Self::ProfileStateRejected,
        Self::StateCorruptRejected,
        Self::TaskContextRejected,
    ];
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
#[allow(clippy::enum_variant_names)] // variants intentionally mirror the normative group names
enum RawTaskErrorData {
    BindingRejected(BindingRejectedData),
    ArtifactRejected(ArtifactNotDistributionErrorData),
    LifecycleRejected(LifecycleRejectedData),
    OperationRejected(OperationRejectedData),
    ReservationRejected(ReservationRejectedData),
    DigestRejected(DigestRejectedData),
    AdaptationDecisionRejected(AdaptationDecisionAlreadyRecordedErrorData),
    ConflictResolutionRejected(ConflictResolutionNotAllowedErrorData),
    CommitPolicyRejected(CommitCommentPolicyMismatchErrorData),
    IntegrationSetRejected(IntegrationSetMismatchErrorData),
    CapabilityRejected(CapabilityRejectedData),
    ProfileStateRejected(ProfileStateRejectedData),
    StateCorruptRejected(StateCorruptErrorData),
    TaskContextRejected(TaskContextRejectedData),
}

impl RawTaskErrorData {
    const fn group(&self) -> TaskErrorGroupKind {
        match self {
            Self::BindingRejected(_) => TaskErrorGroupKind::BindingRejected,
            Self::ArtifactRejected(_) => TaskErrorGroupKind::ArtifactRejected,
            Self::LifecycleRejected(_) => TaskErrorGroupKind::LifecycleRejected,
            Self::OperationRejected(_) => TaskErrorGroupKind::OperationRejected,
            Self::ReservationRejected(_) => TaskErrorGroupKind::ReservationRejected,
            Self::DigestRejected(_) => TaskErrorGroupKind::DigestRejected,
            Self::AdaptationDecisionRejected(_) => TaskErrorGroupKind::AdaptationDecisionRejected,
            Self::ConflictResolutionRejected(_) => TaskErrorGroupKind::ConflictResolutionRejected,
            Self::CommitPolicyRejected(_) => TaskErrorGroupKind::CommitPolicyRejected,
            Self::IntegrationSetRejected(_) => TaskErrorGroupKind::IntegrationSetRejected,
            Self::CapabilityRejected(_) => TaskErrorGroupKind::CapabilityRejected,
            Self::ProfileStateRejected(_) => TaskErrorGroupKind::ProfileStateRejected,
            Self::StateCorruptRejected(_) => TaskErrorGroupKind::StateCorruptRejected,
            Self::TaskContextRejected(_) => TaskErrorGroupKind::TaskContextRejected,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct TaskErrorData(RawTaskErrorData);

impl<'de> Deserialize<'de> for TaskErrorData {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = Self(RawTaskErrorData::deserialize(deserializer)?);
        value.validate().map_err(D::Error::custom)?;
        Ok(value)
    }
}

impl TaskWorkspaceContextInvalidContext {
    fn validate(&self) -> Result<(), &'static str> {
        fn different<T: PartialEq>(expected: &T, observed: &T) -> Result<(), &'static str> {
            (expected != observed)
                .then_some(())
                .ok_or("workspace mismatch values must be unequal")
        }

        match self {
            Self::LeaseMissing(_) | Self::MarkerMissing(_) | Self::MarkerMissingLeaseMissing(_) => {
                Ok(())
            }
            Self::LeaseInvalid(context) => different(
                &context.expected_lease_digest,
                &context.observed_lease_digest,
            ),
            Self::MarkerMissingLeaseInvalid(context) => different(
                &context.expected_lease_digest,
                &context.observed_lease_digest,
            ),
            Self::MarkerMismatch(context) => different(
                &context.expected_marker_digest,
                &context.observed_marker_digest,
            ),
            Self::MarkerMismatchLeaseMissing(context) => different(
                &context.expected_marker_digest,
                &context.observed_marker_digest,
            ),
            Self::MarkerMismatchLeaseInvalid(context) => {
                different(
                    &context.expected_marker_digest,
                    &context.observed_marker_digest,
                )?;
                different(
                    &context.expected_lease_digest,
                    &context.observed_lease_digest,
                )
            }
            Self::ProjectMismatch(context) => {
                different(&context.expected_project_id, &context.observed_project_id)
            }
            Self::ProjectLeaseMissing(context) => {
                different(&context.expected_project_id, &context.observed_project_id)
            }
            Self::ProjectLeaseInvalid(context) => {
                different(&context.expected_project_id, &context.observed_project_id)?;
                different(
                    &context.expected_lease_digest,
                    &context.observed_lease_digest,
                )
            }
            Self::ProjectMarkerMissing(context) => {
                different(&context.expected_project_id, &context.observed_project_id)
            }
            Self::ProjectMarkerMissingLeaseMissing(context) => {
                different(&context.expected_project_id, &context.observed_project_id)
            }
            Self::ProjectMarkerMissingLeaseInvalid(context) => {
                different(&context.expected_project_id, &context.observed_project_id)?;
                different(
                    &context.expected_lease_digest,
                    &context.observed_lease_digest,
                )
            }
            Self::ProjectMarkerMismatch(context) => {
                different(&context.expected_project_id, &context.observed_project_id)?;
                different(
                    &context.expected_marker_digest,
                    &context.observed_marker_digest,
                )
            }
            Self::ProjectMarkerMismatchLeaseMissing(context) => {
                different(&context.expected_project_id, &context.observed_project_id)?;
                different(
                    &context.expected_marker_digest,
                    &context.observed_marker_digest,
                )
            }
            Self::ProjectMarkerMismatchLeaseInvalid(context) => {
                different(&context.expected_project_id, &context.observed_project_id)?;
                different(
                    &context.expected_marker_digest,
                    &context.observed_marker_digest,
                )?;
                different(
                    &context.expected_lease_digest,
                    &context.observed_lease_digest,
                )
            }
        }
    }
}

impl TaskErrorData {
    pub(crate) const GROUP_COUNT: usize = TaskErrorGroupKind::ALL.len();

    const fn group(&self) -> TaskErrorGroupKind {
        self.0.group()
    }

    pub(crate) const fn code(&self) -> RejectedCode {
        match &self.0 {
            RawTaskErrorData::BindingRejected(BindingRejectedData::RepositoryBindingMismatch(
                _,
            )) => RejectedCode::RepositoryBindingMismatch,
            RawTaskErrorData::BindingRejected(BindingRejectedData::MainDiffersFromRepository(
                _,
            )) => RejectedCode::MainDiffersFromRepository,
            RawTaskErrorData::ArtifactRejected(_) => RejectedCode::ArtifactNotDistribution,
            RawTaskErrorData::LifecycleRejected(LifecycleRejectedData::CleanupNotAllowed(_)) => {
                RejectedCode::CleanupNotAllowed
            }
            RawTaskErrorData::LifecycleRejected(LifecycleRejectedData::TaskAbandonmentNotSafe(
                _,
            )) => RejectedCode::TaskAbandonmentNotSafe,
            RawTaskErrorData::LifecycleRejected(LifecycleRejectedData::RecoveryPlanPending(_)) => {
                RejectedCode::RecoveryPlanPending
            }
            RawTaskErrorData::LifecycleRejected(LifecycleRejectedData::TaskPhaseMismatch(_)) => {
                RejectedCode::TaskPhaseMismatch
            }
            RawTaskErrorData::LifecycleRejected(LifecycleRejectedData::TaskMutationBlocked(_)) => {
                RejectedCode::TaskMutationBlocked
            }
            RawTaskErrorData::OperationRejected(
                OperationRejectedData::OperationReplayMismatch(_),
            ) => RejectedCode::OperationReplayMismatch,
            RawTaskErrorData::OperationRejected(OperationRejectedData::OperationInProgress(_)) => {
                RejectedCode::OperationInProgress
            }
            RawTaskErrorData::ReservationRejected(
                ReservationRejectedData::TargetReservationBusy(_),
            ) => RejectedCode::TargetReservationBusy,
            RawTaskErrorData::ReservationRejected(
                ReservationRejectedData::RepositoryAccountReservationBusy(_),
            ) => RejectedCode::RepositoryAccountReservationBusy,
            RawTaskErrorData::DigestRejected(DigestRejectedData::ApprovalDigestMismatch(_)) => {
                RejectedCode::ApprovalDigestMismatch
            }
            RawTaskErrorData::DigestRejected(DigestRejectedData::ChangeReceiptStale(_)) => {
                RejectedCode::ChangeReceiptStale
            }
            RawTaskErrorData::AdaptationDecisionRejected(_) => {
                RejectedCode::AdaptationDecisionAlreadyRecorded
            }
            RawTaskErrorData::ConflictResolutionRejected(_) => {
                RejectedCode::ConflictResolutionNotAllowed
            }
            RawTaskErrorData::CommitPolicyRejected(_) => RejectedCode::CommitCommentPolicyMismatch,
            RawTaskErrorData::IntegrationSetRejected(_) => RejectedCode::IntegrationSetMismatch,
            RawTaskErrorData::CapabilityRejected(
                CapabilityRejectedData::PlatformCapabilityUnproven(_),
            ) => RejectedCode::PlatformCapabilityUnproven,
            RawTaskErrorData::CapabilityRejected(
                CapabilityRejectedData::SupportLayerAmbiguous(_),
            ) => RejectedCode::SupportLayerAmbiguous,
            RawTaskErrorData::CapabilityRejected(
                CapabilityRejectedData::UnsupportedChangeKind(_),
            ) => RejectedCode::UnsupportedChangeKind,
            RawTaskErrorData::CapabilityRejected(
                CapabilityRejectedData::ExclusiveRepositoryUserRequired(_),
            ) => RejectedCode::ExclusiveRepositoryUserRequired,
            RawTaskErrorData::ProfileStateRejected(
                ProfileStateRejectedData::ProjectIdentityCollision(_),
            ) => RejectedCode::ProjectIdentityCollision,
            RawTaskErrorData::ProfileStateRejected(
                ProfileStateRejectedData::StateRootRelocationRequired(_),
            ) => RejectedCode::StateRootRelocationRequired,
            RawTaskErrorData::ProfileStateRejected(ProfileStateRejectedData::ProfileInvalid(_)) => {
                RejectedCode::ProfileInvalid
            }
            RawTaskErrorData::ProfileStateRejected(
                ProfileStateRejectedData::SecretUnavailable(_),
            ) => RejectedCode::SecretUnavailable,
            RawTaskErrorData::StateCorruptRejected(_) => RejectedCode::StateCorrupt,
            RawTaskErrorData::TaskContextRejected(TaskContextRejectedData::TaskNotFound(_)) => {
                RejectedCode::TaskNotFound
            }
            RawTaskErrorData::TaskContextRejected(
                TaskContextRejectedData::TaskWorkspaceContextInvalid(_),
            ) => RejectedCode::TaskWorkspaceContextInvalid,
            RawTaskErrorData::TaskContextRejected(
                TaskContextRejectedData::ToolNotBranchedCompatible(_),
            ) => RejectedCode::ToolNotBranchedCompatible,
        }
    }

    fn validate(&self) -> Result<(), &'static str> {
        fn unequal<T: PartialEq>(left: &T, right: &T) -> Result<(), &'static str> {
            (left != right)
                .then_some(())
                .ok_or("semantic mismatch digests or identities must be unequal")
        }

        match &self.0 {
            RawTaskErrorData::BindingRejected(BindingRejectedData::RepositoryBindingMismatch(
                data,
            )) => unequal(
                &data.context.expected_binding_digest,
                &data.context.observed_binding_digest,
            ),
            RawTaskErrorData::BindingRejected(BindingRejectedData::MainDiffersFromRepository(
                data,
            )) => {
                (data.context.expected_binding_digest == data.context.observed_binding_digest)
                    .then_some(())
                    .ok_or("main-difference binding digests must be equal")?;
                unequal(
                    &data.context.original_fingerprint,
                    &data.context.repository_fingerprint,
                )
            }
            RawTaskErrorData::ArtifactRejected(data) => (!data
                .context
                .accepted_inputs
                .contains_observed(data.context.observed_kind, data.context.observed_role))
            .then_some(())
            .ok_or("accepted artifact tuples must exclude the observed tuple"),
            RawTaskErrorData::LifecycleRejected(LifecycleRejectedData::TaskPhaseMismatch(data)) => {
                (!data
                    .context
                    .allowed_phases
                    .as_slice()
                    .contains(&data.context.phase))
                .then_some(())
                .ok_or("task phase mismatch cannot list the current phase as allowed")
            }
            RawTaskErrorData::OperationRejected(
                OperationRejectedData::OperationReplayMismatch(data),
            ) => unequal(
                &data.context.expected_input_digest,
                &data.context.observed_input_digest,
            ),
            RawTaskErrorData::DigestRejected(DigestRejectedData::ApprovalDigestMismatch(data)) => {
                unequal(&data.context.expected_digest, &data.context.observed_digest)
            }
            RawTaskErrorData::DigestRejected(DigestRejectedData::ChangeReceiptStale(data)) => {
                unequal(&data.context.expected_digest, &data.context.observed_digest)
            }
            RawTaskErrorData::ConflictResolutionRejected(data) => (!data
                .context
                .allowed_resolutions
                .as_slice()
                .contains(&data.context.requested_resolution))
            .then_some(())
            .ok_or("allowed resolutions must exclude the requested resolution"),
            RawTaskErrorData::CommitPolicyRejected(data) => unequal(
                &data.context.expected_policy_digest,
                &data.context.observed_policy_digest,
            ),
            RawTaskErrorData::IntegrationSetRejected(IntegrationSetMismatchErrorData::Unlock(
                data,
            )) => unequal(
                &data.context.expected_lineage_digest,
                &data.context.observed_lineage_digest,
            ),
            RawTaskErrorData::IntegrationSetRejected(
                IntegrationSetMismatchErrorData::Recovery(data),
            ) => unequal(
                &data.context.expected_lineage_digest,
                &data.context.observed_lineage_digest,
            ),
            RawTaskErrorData::ProfileStateRejected(
                ProfileStateRejectedData::ProjectIdentityCollision(data),
            ) => unequal(&data.context.expected_digest, &data.context.observed_digest),
            RawTaskErrorData::ProfileStateRejected(
                ProfileStateRejectedData::StateRootRelocationRequired(data),
            ) => unequal(&data.context.expected_digest, &data.context.observed_digest),
            RawTaskErrorData::StateCorruptRejected(data) => match &data.context.observation {
                StateCorruptObservation::ExactBytes(observation) => {
                    unequal(&data.context.expected_digest, &observation.observed_digest)
                }
                StateCorruptObservation::Unavailable(_) => Ok(()),
            },
            RawTaskErrorData::TaskContextRejected(
                TaskContextRejectedData::TaskWorkspaceContextInvalid(data),
            ) => data.context.validate(),
            _ => Ok(()),
        }
    }
}

impl JsonSchema for TaskErrorData {
    fn schema_name() -> Cow<'static, str> {
        "TaskErrorData".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<RepositoryBindingMismatchErrorData>(),
            generator.subschema_for::<MainDiffersFromRepositoryErrorData>(),
            generator.subschema_for::<ArtifactNotDistributionErrorData>(),
            generator.subschema_for::<CleanupNotAllowedErrorData>(),
            generator.subschema_for::<TaskAbandonmentNotSafeErrorData>(),
            generator.subschema_for::<RecoveryPlanPendingErrorData>(),
            generator.subschema_for::<TaskPhaseMismatchErrorData>(),
            generator.subschema_for::<TaskMutationBlockedErrorData>(),
            generator.subschema_for::<OperationReplayMismatchErrorData>(),
            generator.subschema_for::<OperationInProgressErrorData>(),
            generator.subschema_for::<TargetReservationBusyErrorData>(),
            generator.subschema_for::<RepositoryAccountReservationBusyErrorData>(),
            generator.subschema_for::<ApprovalDigestMismatchErrorData>(),
            generator.subschema_for::<ChangeReceiptStaleErrorData>(),
            generator.subschema_for::<AdaptationDecisionAlreadyRecordedErrorData>(),
            generator.subschema_for::<ConflictResolutionNotAllowedErrorData>(),
            generator.subschema_for::<CommitCommentPolicyMismatchErrorData>(),
            generator.subschema_for::<IntegrationSetMismatchErrorData>(),
            generator.subschema_for::<PlatformCapabilityUnprovenErrorData>(),
            generator.subschema_for::<SupportLayerAmbiguousErrorData>(),
            generator.subschema_for::<UnsupportedChangeKindErrorData>(),
            generator.subschema_for::<ExclusiveRepositoryUserRequiredErrorData>(),
            generator.subschema_for::<ProjectIdentityCollisionErrorData>(),
            generator.subschema_for::<StateRootRelocationRequiredErrorData>(),
            generator.subschema_for::<ProfileInvalidErrorData>(),
            generator.subschema_for::<SecretUnavailableErrorData>(),
            generator.subschema_for::<StateCorruptErrorData>(),
            generator.subschema_for::<TaskNotFoundErrorData>(),
            generator.subschema_for::<TaskWorkspaceContextInvalidErrorData>(),
            generator.subschema_for::<ToolNotBranchedCompatibleErrorData>(),
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::branched_development::contracts::schema::audit_json_schema;
    use schemars::schema_for;
    use serde_json::{json, Value};

    const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const C: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
    const ID1: &str = "11111111-1111-4111-8111-111111111111";
    const ID2: &str = "22222222-2222-4222-8222-222222222222";
    const OP: &str = "33333333-3333-4333-8333-333333333333";
    const EXPECTED_STABLE_CODES: &[&str] = &[
        "repositoryBindingMismatch",
        "mainDiffersFromRepository",
        "artifactKindMismatch",
        "artifactNotDistribution",
        "platformWarningRejected",
        "vendorAncestryMismatch",
        "twiceChangedProperties",
        "unresolvedReferences",
        "unexpectedDelta",
        "adaptationDecisionAlreadyRecorded",
        "conflictDecisionsIncomplete",
        "unboundResolutionChanges",
        "validationFailed",
        "repositoryLockConflict",
        "operationTimedOut",
        "repositoryLockRollbackFailed",
        "repositoryUpdatePlanStale",
        "repositoryStructureConfirmationUnproven",
        "manualSupportRequired",
        "manualSupportCleanupRequired",
        "vendorForbidsChanges",
        "supportPreflightInconclusive",
        "manualSupportRootLockRequired",
        "supportPrerequisiteArmStale",
        "manualSupportActionPending",
        "manualSupportLocksRemain",
        "manualSupportLocalChangesRemain",
        "manualSupportPrerequisiteInvalid",
        "supportPrerequisiteConflict",
        "supportCorrectionPending",
        "supportRecoveryReapprovalRequired",
        "recoveryReapprovalRequired",
        "supportConflictResolutionPending",
        "supportRecoveryBlockedByLock",
        "preArmCancellationRecoveryBlocked",
        "supportPrerequisiteReconciliationRequired",
        "relevantBaselineChanged",
        "supportPreflightStale",
        "mainPreparationMismatch",
        "additionalLocksRequired",
        "mainMergeValidationFailed",
        "postMergeLineageChanged",
        "repositoryCommitFailed",
        "repositoryCommitAmbiguous",
        "repositoryUnlockUnverified",
        "cleanupNotAllowed",
        "abandonmentRecoveryRequired",
        "unsafeTaskPath",
        "operationReplayMismatch",
        "targetReservationBusy",
        "repositoryAccountReservationBusy",
        "operationEffectUnknown",
        "recoveryPlanPending",
        "taskPhaseMismatch",
        "approvalDigestMismatch",
        "changeReceiptStale",
        "conflictResolutionNotAllowed",
        "taskMutationBlocked",
        "platformCapabilityUnproven",
        "supportLayerAmbiguous",
        "unsupportedChangeKind",
        "projectIdentityCollision",
        "stateRootRelocationRequired",
        "exclusiveRepositoryUserRequired",
        "rollbackUnproven",
        "taskAbandonmentNotSafe",
        "profileInvalid",
        "secretUnavailable",
        "stateCorrupt",
        "operationInProgress",
        "taskNotFound",
        "taskWorkspaceContextInvalid",
        "toolNotBranchedCompatible",
        "commitCommentPolicyMismatch",
        "integrationSetMismatch",
    ];
    const EXPECTED_REJECTED_CODES: &[&str] = &[
        "repositoryBindingMismatch",
        "mainDiffersFromRepository",
        "artifactNotDistribution",
        "cleanupNotAllowed",
        "taskAbandonmentNotSafe",
        "operationReplayMismatch",
        "recoveryPlanPending",
        "taskPhaseMismatch",
        "approvalDigestMismatch",
        "changeReceiptStale",
        "conflictResolutionNotAllowed",
        "adaptationDecisionAlreadyRecorded",
        "taskMutationBlocked",
        "platformCapabilityUnproven",
        "supportLayerAmbiguous",
        "unsupportedChangeKind",
        "projectIdentityCollision",
        "stateRootRelocationRequired",
        "exclusiveRepositoryUserRequired",
        "targetReservationBusy",
        "repositoryAccountReservationBusy",
        "profileInvalid",
        "secretUnavailable",
        "stateCorrupt",
        "operationInProgress",
        "taskNotFound",
        "taskWorkspaceContextInvalid",
        "toolNotBranchedCompatible",
        "commitCommentPolicyMismatch",
        "integrationSetMismatch",
    ];

    fn tool(tool_name: &str, request_variant: Option<&str>) -> Value {
        let mut operation = serde_json::Map::new();
        operation.insert("toolName".to_owned(), json!(tool_name));
        if let Some(request_variant) = request_variant {
            operation.insert("requestVariant".to_owned(), json!(request_variant));
        }
        json!({"actionKind": "toolCall", "operation": operation})
    }

    fn status_only() -> Value {
        json!([tool("unica.branched.status", None)])
    }

    fn leaf(code: &str, context: Value, allowed_next_actions: Value) -> Value {
        json!({
            "code": code,
            "context": context,
            "allowedNextActions": allowed_next_actions,
        })
    }

    fn lifecycle_blocked() -> Value {
        json!({
            "contextKind": "lifecycle",
            "phase": "developing",
            "allowedPhases": [],
            "blockerCodes": ["operationInProgress"],
        })
    }

    fn basic_task_context() -> Value {
        json!({
            "contextKind": "taskContext",
            "requestedTaskId": "TASK-1",
            "requestedToolName": "unica.merge.verify",
        })
    }

    fn cases() -> Vec<(RejectedCode, &'static str, Value)> {
        let owner = json!({
            "ownerKind": "startAttempt",
            "projectId": ID1,
            "taskId": "TASK-1",
            "operationId": OP,
        });
        vec![
            (
                RejectedCode::RepositoryBindingMismatch,
                "binding",
                leaf(
                    "repositoryBindingMismatch",
                    json!({
                        "contextKind": "binding",
                        "expectedBindingDigest": A,
                        "observedBindingDigest": B,
                    }),
                    status_only(),
                ),
            ),
            (
                RejectedCode::MainDiffersFromRepository,
                "binding",
                leaf(
                    "mainDiffersFromRepository",
                    json!({
                        "contextKind": "binding",
                        "expectedBindingDigest": A,
                        "observedBindingDigest": A,
                        "originalFingerprint": B,
                        "repositoryFingerprint": C,
                    }),
                    status_only(),
                ),
            ),
            (
                RejectedCode::ArtifactNotDistribution,
                "artifactInput",
                leaf(
                    "artifactNotDistribution",
                    json!({
                        "contextKind": "artifactInput",
                        "artifactId": ID1,
                        "observedKind": "ordinaryConfiguration",
                        "observedRole": "ordinaryResult",
                        "acceptedInputs": [{
                            "kind": "configurationDistribution",
                            "role": "baselineDistribution"
                        }],
                    }),
                    status_only(),
                ),
            ),
            (
                RejectedCode::CleanupNotAllowed,
                "lifecycle",
                leaf("cleanupNotAllowed", lifecycle_blocked(), status_only()),
            ),
            (
                RejectedCode::TaskAbandonmentNotSafe,
                "lifecycle",
                leaf("taskAbandonmentNotSafe", lifecycle_blocked(), status_only()),
            ),
            (
                RejectedCode::OperationReplayMismatch,
                "operation",
                leaf(
                    "operationReplayMismatch",
                    json!({
                        "contextKind": "operation",
                        "operationId": OP,
                        "expectedInputDigest": A,
                        "observedInputDigest": B,
                    }),
                    json!([]),
                ),
            ),
            (
                RejectedCode::RecoveryPlanPending,
                "lifecycle",
                leaf(
                    "recoveryPlanPending",
                    json!({
                        "contextKind": "lifecycle",
                        "phase": "recoveryRequired",
                        "allowedPhases": [],
                        "blockerCodes": [],
                        "recoveryDigest": A,
                        "recoveryCancellationAllowed": true,
                    }),
                    json!([
                        tool("unica.branched.status", None),
                        tool("unica.repository.recover", Some("recoverApply")),
                        tool("unica.repository.recover", Some("recoverCancel")),
                    ]),
                ),
            ),
            (
                RejectedCode::TaskPhaseMismatch,
                "lifecycle",
                leaf(
                    "taskPhaseMismatch",
                    json!({
                        "contextKind": "lifecycle",
                        "phase": "developing",
                        "allowedPhases": ["localVerified", "synchronized"],
                        "blockerCodes": [],
                    }),
                    status_only(),
                ),
            ),
            (
                RejectedCode::ApprovalDigestMismatch,
                "digest",
                leaf(
                    "approvalDigestMismatch",
                    json!({
                        "contextKind": "digest",
                        "expectedDigest": A,
                        "observedDigest": B,
                        "producerId": ID1,
                    }),
                    status_only(),
                ),
            ),
            (
                RejectedCode::ChangeReceiptStale,
                "digest",
                leaf(
                    "changeReceiptStale",
                    json!({
                        "contextKind": "digest",
                        "expectedDigest": A,
                        "observedDigest": C,
                        "producerId": ID2,
                    }),
                    status_only(),
                ),
            ),
            (
                RejectedCode::ConflictResolutionNotAllowed,
                "conflictResolution",
                leaf(
                    "conflictResolutionNotAllowed",
                    json!({
                        "contextKind": "conflictResolution",
                        "sessionId": ID1,
                        "conflictId": ID2,
                        "conflictKind": "twiceChanged",
                        "requestedResolution": "takeOurs",
                        "allowedResolutions": ["takeTheirs", "combine"],
                    }),
                    json!([
                        tool("unica.branched.status", None),
                        tool("unica.merge.conflicts", None),
                    ]),
                ),
            ),
            (
                RejectedCode::AdaptationDecisionAlreadyRecorded,
                "adaptationDecision",
                leaf(
                    "adaptationDecisionAlreadyRecorded",
                    json!({
                        "contextKind": "adaptationDecision",
                        "verificationId": ID1,
                        "existingDecisionId": ID2,
                        "existingAdaptationDecisionDigest": A,
                    }),
                    json!([
                        tool("unica.branched.status", None),
                        tool("unica.merge.verify", Some("synchronizedTask")),
                    ]),
                ),
            ),
            (
                RejectedCode::TaskMutationBlocked,
                "lifecycle",
                leaf("taskMutationBlocked", lifecycle_blocked(), status_only()),
            ),
            (
                RejectedCode::PlatformCapabilityUnproven,
                "capability",
                leaf(
                    "platformCapabilityUnproven",
                    json!({
                        "contextKind": "capability",
                        "capabilityKind": "platform",
                        "capabilityRowId": "platform-row",
                        "evidenceDigest": A,
                    }),
                    status_only(),
                ),
            ),
            (
                RejectedCode::SupportLayerAmbiguous,
                "capability",
                leaf(
                    "supportLayerAmbiguous",
                    json!({
                        "contextKind": "capability",
                        "capabilityKind": "supportLayer",
                        "evidenceDigest": A,
                    }),
                    status_only(),
                ),
            ),
            (
                RejectedCode::UnsupportedChangeKind,
                "capability",
                leaf(
                    "unsupportedChangeKind",
                    json!({
                        "contextKind": "capability",
                        "capabilityKind": "changeSemantics",
                        "evidenceDigest": A,
                    }),
                    status_only(),
                ),
            ),
            (
                RejectedCode::ProjectIdentityCollision,
                "profileState",
                leaf(
                    "projectIdentityCollision",
                    json!({
                        "contextKind": "profileState",
                        "projectId": ID1,
                        "expectedDigest": A,
                        "observedDigest": B,
                    }),
                    status_only(),
                ),
            ),
            (
                RejectedCode::StateRootRelocationRequired,
                "profileState",
                leaf(
                    "stateRootRelocationRequired",
                    json!({
                        "contextKind": "profileState",
                        "projectId": ID1,
                        "expectedDigest": A,
                        "observedDigest": C,
                    }),
                    status_only(),
                ),
            ),
            (
                RejectedCode::ExclusiveRepositoryUserRequired,
                "capability",
                leaf(
                    "exclusiveRepositoryUserRequired",
                    json!({
                        "contextKind": "capability",
                        "capabilityKind": "repositoryUserExclusivity",
                        "evidenceDigest": B,
                    }),
                    status_only(),
                ),
            ),
            (
                RejectedCode::TargetReservationBusy,
                "targetReservation",
                leaf(
                    "targetReservationBusy",
                    json!({
                        "contextKind": "targetReservation",
                        "repositoryIdentityDigest": A,
                        "originalInfobaseIdentityDigest": B,
                        "reservationKeyDigest": C,
                        "owner": owner.clone(),
                    }),
                    status_only(),
                ),
            ),
            (
                RejectedCode::RepositoryAccountReservationBusy,
                "repositoryAccountReservation",
                leaf(
                    "repositoryAccountReservationBusy",
                    json!({
                        "contextKind": "repositoryAccountReservation",
                        "repositoryIdentityDigest": A,
                        "normalizedUsernameDigest": B,
                        "reservationKeyDigest": C,
                        "owner": owner,
                    }),
                    status_only(),
                ),
            ),
            (
                RejectedCode::ProfileInvalid,
                "profileState",
                leaf(
                    "profileInvalid",
                    json!({
                        "contextKind": "profileState",
                        "profile": "default",
                        "propertyPath": "repository.username",
                    }),
                    status_only(),
                ),
            ),
            (
                RejectedCode::SecretUnavailable,
                "profileState",
                leaf(
                    "secretUnavailable",
                    json!({
                        "contextKind": "profileState",
                        "profile": "default",
                        "propertyPath": "repository.passwordSecret",
                    }),
                    status_only(),
                ),
            ),
            (
                RejectedCode::StateCorrupt,
                "stateCorrupt",
                leaf(
                    "stateCorrupt",
                    json!({
                        "contextKind": "stateCorrupt",
                        "stateRef": {
                            "stateRefKind": "workspace",
                            "workspaceIdentityDigest": C,
                        },
                        "expectedDigest": A,
                        "observation": {
                            "observationKind": "exactBytes",
                            "observedDigest": B,
                        },
                    }),
                    status_only(),
                ),
            ),
            (
                RejectedCode::OperationInProgress,
                "operation",
                leaf(
                    "operationInProgress",
                    json!({
                        "contextKind": "operation",
                        "operationId": OP,
                        "activeOperationDigest": A,
                    }),
                    status_only(),
                ),
            ),
            (
                RejectedCode::TaskNotFound,
                "taskContext",
                leaf(
                    "taskNotFound",
                    basic_task_context(),
                    json!([
                        tool("unica.branched.start", None),
                        tool("unica.branched.status", None),
                    ]),
                ),
            ),
            (
                RejectedCode::TaskWorkspaceContextInvalid,
                "taskContext",
                leaf(
                    "taskWorkspaceContextInvalid",
                    json!({
                        "contextKind": "taskContext",
                        "requestedTaskId": "TASK-1",
                        "requestedToolName": "unica.merge.verify",
                        "mismatchKinds": ["markerMissing"],
                        "expectedMarkerDigest": A,
                        "observedMarkerDigest": null,
                    }),
                    json!([]),
                ),
            ),
            (
                RejectedCode::ToolNotBranchedCompatible,
                "taskContext",
                leaf("toolNotBranchedCompatible", basic_task_context(), json!([])),
            ),
            (
                RejectedCode::CommitCommentPolicyMismatch,
                "commitCommentPolicy",
                leaf(
                    "commitCommentPolicyMismatch",
                    json!({
                        "contextKind": "commitCommentPolicy",
                        "phase": "mainValidated",
                        "expectedPolicyDigest": A,
                        "observedPolicyDigest": B,
                        "mismatchKinds": ["templateChanged"],
                    }),
                    json!([
                        tool("unica.branched.status", None),
                        tool("unica.branched.archive", Some("abandonedPreview")),
                    ]),
                ),
            ),
            (
                RejectedCode::IntegrationSetMismatch,
                "integrationSet",
                leaf(
                    "integrationSetMismatch",
                    json!({
                        "contextKind": "integrationSet",
                        "phase": "locked",
                        "expectedLineageDigest": A,
                        "observedLineageDigest": B,
                        "mismatchKinds": ["lockSet"],
                        "exitKind": "unlock",
                        "lockSetId": ID1,
                        "expectedLockSetDigest": C,
                    }),
                    json!([
                        tool("unica.branched.status", None),
                        tool("unica.repository.unlock", Some("rollback")),
                    ]),
                ),
            ),
        ]
    }

    fn schema_validator<T: JsonSchema>() -> jsonschema::Validator {
        let schema = serde_json::to_value(schema_for!(T)).unwrap();
        jsonschema::options()
            .with_draft(jsonschema::Draft::Draft202012)
            .build(&schema)
            .unwrap()
    }

    struct MarkerDataSchema<Marker>(std::marker::PhantomData<Marker>);

    impl<Marker: RejectedCodeMarker> JsonSchema for MarkerDataSchema<Marker> {
        fn schema_name() -> Cow<'static, str> {
            format!("TestMarkerDataSchemaOf{}", Marker::CODE.as_str()).into()
        }

        fn json_schema(generator: &mut SchemaGenerator) -> Schema {
            Marker::data_schema(generator)
        }
    }

    fn marker_schemas() -> Vec<(RejectedCode, Value)> {
        macro_rules! collect {
            ($($marker:ty),+ $(,)?) => {
                vec![$(
                    (
                        RejectedCode::ALL
                            .iter()
                            .copied()
                            .find(|code| {
                                StableErrorCode::from(*code)
                                    == <$marker as StableCodeMarker>::CODE
                            })
                            .unwrap(),
                        serde_json::to_value(schema_for!(MarkerDataSchema<$marker>)).unwrap(),
                    )
                ),+]
            };
        }
        collect!(
            RepositoryBindingMismatchMarker,
            MainDiffersFromRepositoryMarker,
            ArtifactNotDistributionMarker,
            CleanupNotAllowedMarker,
            TaskAbandonmentNotSafeMarker,
            OperationReplayMismatchMarker,
            RecoveryPlanPendingMarker,
            TaskPhaseMismatchMarker,
            ApprovalDigestMismatchMarker,
            ChangeReceiptStaleMarker,
            ConflictResolutionNotAllowedMarker,
            AdaptationDecisionAlreadyRecordedMarker,
            TaskMutationBlockedMarker,
            PlatformCapabilityUnprovenMarker,
            SupportLayerAmbiguousMarker,
            UnsupportedChangeKindMarker,
            ProjectIdentityCollisionMarker,
            StateRootRelocationRequiredMarker,
            ExclusiveRepositoryUserRequiredMarker,
            TargetReservationBusyMarker,
            RepositoryAccountReservationBusyMarker,
            ProfileInvalidMarker,
            SecretUnavailableMarker,
            StateCorruptMarker,
            OperationInProgressMarker,
            TaskNotFoundMarker,
            TaskWorkspaceContextInvalidMarker,
            ToolNotBranchedCompatibleMarker,
            CommitCommentPolicyMismatchMarker,
            IntegrationSetMismatchMarker,
        )
    }

    fn code_only_shapes_are_equivalent(left: RejectedCode, right: RejectedCode) -> bool {
        if left == right {
            return true;
        }
        let same_class = |class: &[RejectedCode]| class.contains(&left) && class.contains(&right);
        same_class(&[
            RejectedCode::CleanupNotAllowed,
            RejectedCode::TaskAbandonmentNotSafe,
            RejectedCode::TaskMutationBlocked,
        ]) || same_class(&[
            RejectedCode::ApprovalDigestMismatch,
            RejectedCode::ChangeReceiptStale,
        ]) || same_class(&[
            RejectedCode::ProjectIdentityCollision,
            RejectedCode::StateRootRelocationRequired,
        ]) || same_class(&[
            RejectedCode::ProfileInvalid,
            RejectedCode::SecretUnavailable,
        ])
    }

    fn schema_only_relational_superset(source: RejectedCode, target: RejectedCode) -> bool {
        // Draft 2020-12 cannot express equality between sibling properties.  This exact
        // substitution is structurally valid for the target leaf, while runtime validation
        // still rejects it because a binding mismatch requires unequal digests.
        source == RejectedCode::MainDiffersFromRepository
            && target == RejectedCode::RepositoryBindingMismatch
    }

    #[test]
    fn stable_and_rejected_vocabularies_are_exact_75_and_30() {
        assert_eq!(StableErrorCode::ALL.len(), 75);
        assert_eq!(RejectedCode::ALL.len(), 30);
        let stable = StableErrorCode::ALL
            .iter()
            .map(|code| code.as_str())
            .collect::<Vec<_>>();
        let rejected = RejectedCode::ALL
            .iter()
            .map(|code| code.as_str())
            .collect::<Vec<_>>();
        assert_eq!(stable, EXPECTED_STABLE_CODES);
        assert_eq!(rejected, EXPECTED_REJECTED_CODES);
        assert_eq!(
            stable
                .iter()
                .filter(|code| **code == "commitCommentPolicyMismatch")
                .count(),
            1
        );
        assert_eq!(
            stable
                .iter()
                .filter(|code| **code == "integrationSetMismatch")
                .count(),
            1
        );
        assert!(rejected.iter().all(|code| stable.contains(code)));
        assert!(serde_json::from_value::<StableErrorCode>(json!("commitPolicyMismatch")).is_err());
        assert!(serde_json::from_value::<RejectedCode>(json!("artifactKindMismatch")).is_err());

        let marker_codes = [
            RepositoryBindingMismatchMarker::CODE,
            MainDiffersFromRepositoryMarker::CODE,
            ArtifactKindMismatchMarker::CODE,
            ArtifactNotDistributionMarker::CODE,
            PlatformWarningRejectedMarker::CODE,
            VendorAncestryMismatchMarker::CODE,
            TwiceChangedPropertiesMarker::CODE,
            UnresolvedReferencesMarker::CODE,
            UnexpectedDeltaMarker::CODE,
            AdaptationDecisionAlreadyRecordedMarker::CODE,
            ConflictDecisionsIncompleteMarker::CODE,
            UnboundResolutionChangesMarker::CODE,
            ValidationFailedMarker::CODE,
            RepositoryLockConflictMarker::CODE,
            OperationTimedOutMarker::CODE,
            RepositoryLockRollbackFailedMarker::CODE,
            RepositoryUpdatePlanStaleMarker::CODE,
            RepositoryStructureConfirmationUnprovenMarker::CODE,
            ManualSupportRequiredMarker::CODE,
            ManualSupportCleanupRequiredMarker::CODE,
            VendorForbidsChangesMarker::CODE,
            SupportPreflightInconclusiveMarker::CODE,
            ManualSupportRootLockRequiredMarker::CODE,
            SupportPrerequisiteArmStaleMarker::CODE,
            ManualSupportActionPendingMarker::CODE,
            ManualSupportLocksRemainMarker::CODE,
            ManualSupportLocalChangesRemainMarker::CODE,
            ManualSupportPrerequisiteInvalidMarker::CODE,
            SupportPrerequisiteConflictMarker::CODE,
            SupportCorrectionPendingMarker::CODE,
            SupportRecoveryReapprovalRequiredMarker::CODE,
            RecoveryReapprovalRequiredMarker::CODE,
            SupportConflictResolutionPendingMarker::CODE,
            SupportRecoveryBlockedByLockMarker::CODE,
            PreArmCancellationRecoveryBlockedMarker::CODE,
            SupportPrerequisiteReconciliationRequiredMarker::CODE,
            RelevantBaselineChangedMarker::CODE,
            SupportPreflightStaleMarker::CODE,
            MainPreparationMismatchMarker::CODE,
            AdditionalLocksRequiredMarker::CODE,
            MainMergeValidationFailedMarker::CODE,
            PostMergeLineageChangedMarker::CODE,
            RepositoryCommitFailedMarker::CODE,
            RepositoryCommitAmbiguousMarker::CODE,
            RepositoryUnlockUnverifiedMarker::CODE,
            CleanupNotAllowedMarker::CODE,
            AbandonmentRecoveryRequiredMarker::CODE,
            UnsafeTaskPathMarker::CODE,
            OperationReplayMismatchMarker::CODE,
            TargetReservationBusyMarker::CODE,
            RepositoryAccountReservationBusyMarker::CODE,
            OperationEffectUnknownMarker::CODE,
            RecoveryPlanPendingMarker::CODE,
            TaskPhaseMismatchMarker::CODE,
            ApprovalDigestMismatchMarker::CODE,
            ChangeReceiptStaleMarker::CODE,
            ConflictResolutionNotAllowedMarker::CODE,
            TaskMutationBlockedMarker::CODE,
            PlatformCapabilityUnprovenMarker::CODE,
            SupportLayerAmbiguousMarker::CODE,
            UnsupportedChangeKindMarker::CODE,
            ProjectIdentityCollisionMarker::CODE,
            StateRootRelocationRequiredMarker::CODE,
            ExclusiveRepositoryUserRequiredMarker::CODE,
            RollbackUnprovenMarker::CODE,
            TaskAbandonmentNotSafeMarker::CODE,
            ProfileInvalidMarker::CODE,
            SecretUnavailableMarker::CODE,
            StateCorruptMarker::CODE,
            OperationInProgressMarker::CODE,
            TaskNotFoundMarker::CODE,
            TaskWorkspaceContextInvalidMarker::CODE,
            ToolNotBranchedCompatibleMarker::CODE,
            CommitCommentPolicyMismatchMarker::CODE,
            IntegrationSetMismatchMarker::CODE,
        ];
        assert_eq!(marker_codes, StableErrorCode::ALL);
    }

    #[test]
    fn exhaustive_rejected_table_accepts_all_30_exact_context_action_rows() {
        let cases = cases();
        assert_eq!(cases.len(), 30);
        assert_eq!(
            cases.iter().map(|(code, _, _)| *code).collect::<Vec<_>>(),
            RejectedCode::ALL
        );
        assert_eq!(TaskErrorData::GROUP_COUNT, 14);
        let expected_groups = [
            TaskErrorGroupKind::BindingRejected,
            TaskErrorGroupKind::BindingRejected,
            TaskErrorGroupKind::ArtifactRejected,
            TaskErrorGroupKind::LifecycleRejected,
            TaskErrorGroupKind::LifecycleRejected,
            TaskErrorGroupKind::OperationRejected,
            TaskErrorGroupKind::LifecycleRejected,
            TaskErrorGroupKind::LifecycleRejected,
            TaskErrorGroupKind::DigestRejected,
            TaskErrorGroupKind::DigestRejected,
            TaskErrorGroupKind::ConflictResolutionRejected,
            TaskErrorGroupKind::AdaptationDecisionRejected,
            TaskErrorGroupKind::LifecycleRejected,
            TaskErrorGroupKind::CapabilityRejected,
            TaskErrorGroupKind::CapabilityRejected,
            TaskErrorGroupKind::CapabilityRejected,
            TaskErrorGroupKind::ProfileStateRejected,
            TaskErrorGroupKind::ProfileStateRejected,
            TaskErrorGroupKind::CapabilityRejected,
            TaskErrorGroupKind::ReservationRejected,
            TaskErrorGroupKind::ReservationRejected,
            TaskErrorGroupKind::ProfileStateRejected,
            TaskErrorGroupKind::ProfileStateRejected,
            TaskErrorGroupKind::StateCorruptRejected,
            TaskErrorGroupKind::OperationRejected,
            TaskErrorGroupKind::TaskContextRejected,
            TaskErrorGroupKind::TaskContextRejected,
            TaskErrorGroupKind::TaskContextRejected,
            TaskErrorGroupKind::CommitPolicyRejected,
            TaskErrorGroupKind::IntegrationSetRejected,
        ];
        assert_eq!(
            cases
                .iter()
                .map(|(_, context_kind, _)| *context_kind)
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            15
        );
        let validator = schema_validator::<TaskErrorData>();
        let mut observed_groups = Vec::with_capacity(cases.len());
        for (expected_code, _, value) in cases {
            let parsed: TaskErrorData = serde_json::from_value(value.clone())
                .unwrap_or_else(|error| panic!("{expected_code:?} rejected {value}: {error}"));
            assert_eq!(parsed.code(), expected_code);
            observed_groups.push(parsed.group());
            assert!(
                validator.is_valid(&value),
                "schema rejected {expected_code:?}: {value}"
            );
        }
        assert_eq!(observed_groups, expected_groups);
        assert_eq!(
            observed_groups
                .iter()
                .copied()
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>(),
            TaskErrorGroupKind::ALL
        );
    }

    #[test]
    fn every_rejected_code_rejects_a_different_named_context_branch() {
        let cases = cases();
        for (index, (_, context_kind, value)) in cases.iter().enumerate() {
            let replacement = cases
                .iter()
                .cycle()
                .skip(index + 1)
                .find(|(_, other_kind, _)| other_kind != context_kind)
                .unwrap();
            let mut invalid = value.clone();
            invalid["context"] = replacement.2["context"].clone();
            assert!(
                serde_json::from_value::<TaskErrorData>(invalid.clone()).is_err(),
                "cross-context substitution survived: {invalid}"
            );
        }
    }

    #[test]
    fn exhaustive_code_substitution_matches_only_normative_shared_shapes() {
        let cases = cases();
        let validator = schema_validator::<TaskErrorData>();
        for (source_code, _, source) in &cases {
            for target_code in RejectedCode::ALL {
                let mut candidate = source.clone();
                candidate["code"] = json!(target_code.as_str());
                let expected = code_only_shapes_are_equivalent(*source_code, *target_code);
                assert_eq!(
                    serde_json::from_value::<TaskErrorData>(candidate.clone()).is_ok(),
                    expected,
                    "Serde substitution {source_code:?} -> {target_code:?}: {candidate}"
                );
                let schema_expected =
                    expected || schema_only_relational_superset(*source_code, *target_code);
                assert_eq!(
                    validator.is_valid(&candidate),
                    schema_expected,
                    "schema substitution {source_code:?} -> {target_code:?}: {candidate}"
                );
            }
        }
    }

    #[test]
    fn all_30_rejected_markers_select_their_exact_literal_leaf_schema() {
        let cases = cases();
        let schemas = marker_schemas();
        assert_eq!(schemas.len(), 30);
        assert_eq!(
            schemas.iter().map(|(code, _)| *code).collect::<Vec<_>>(),
            RejectedCode::ALL
        );
        for (marker_code, schema) in schemas {
            audit_json_schema(&schema).unwrap_or_else(|error| panic!("{error}: {schema}"));
            let validator = jsonschema::options()
                .with_draft(jsonschema::Draft::Draft202012)
                .build(&schema)
                .unwrap();
            for (fixture_code, _, fixture) in &cases {
                assert_eq!(
                    validator.is_valid(fixture),
                    *fixture_code == marker_code,
                    "marker {marker_code:?} vs fixture {fixture_code:?}"
                );
            }
        }
    }

    #[test]
    fn semantic_equalities_exclusions_and_exact_actions_fail_closed() {
        let validator = schema_validator::<TaskErrorData>();
        let mut binding = cases()[0].2.clone();
        binding["context"]["observedBindingDigest"] = json!(A);
        assert!(serde_json::from_value::<TaskErrorData>(binding.clone()).is_err());
        assert!(validator.is_valid(&binding));

        let mut main = cases()[1].2.clone();
        main["context"]["repositoryFingerprint"] = json!(B);
        assert!(serde_json::from_value::<TaskErrorData>(main.clone()).is_err());
        assert!(validator.is_valid(&main));
        let mut main_binding_unequal = cases()[1].2.clone();
        main_binding_unequal["context"]["observedBindingDigest"] = json!(B);
        assert!(serde_json::from_value::<TaskErrorData>(main_binding_unequal.clone()).is_err());
        assert!(validator.is_valid(&main_binding_unequal));

        let mut artifact = cases()[2].2.clone();
        artifact["context"]["acceptedInputs"] = json!([{
            "kind": "ordinaryConfiguration",
            "role": "ordinaryResult"
        }]);
        assert!(serde_json::from_value::<TaskErrorData>(artifact.clone()).is_err());
        assert!(validator.is_valid(&artifact));

        let mut conflict = cases()[10].2.clone();
        conflict["context"]["allowedResolutions"] = json!(["takeOurs"]);
        assert!(serde_json::from_value::<TaskErrorData>(conflict.clone()).is_err());
        assert!(validator.is_valid(&conflict));

        let mut phase = cases()[7].2.clone();
        phase["context"]["allowedPhases"] = json!(["developing"]);
        assert!(serde_json::from_value::<TaskErrorData>(phase.clone()).is_err());
        assert!(validator.is_valid(&phase));

        let mut adaptation = cases()[11].2.clone();
        adaptation["allowedNextActions"][1]["operation"]["requestVariant"] =
            json!("synchronizedTaskAdapted");
        assert!(serde_json::from_value::<TaskErrorData>(adaptation.clone()).is_err());
        assert!(!validator.is_valid(&adaptation));

        let mut policy = cases()[28].2.clone();
        policy["context"]["observedPolicyDigest"] = json!(A);
        assert!(serde_json::from_value::<TaskErrorData>(policy.clone()).is_err());
        assert!(validator.is_valid(&policy));
        let mut policy_wrong_action = cases()[28].2.clone();
        policy_wrong_action["allowedNextActions"][1] =
            tool("unica.repository.recover", Some("recoverApply"));
        assert!(serde_json::from_value::<TaskErrorData>(policy_wrong_action.clone()).is_err());
        assert!(!validator.is_valid(&policy_wrong_action));

        let mut integration = cases()[29].2.clone();
        integration["allowedNextActions"][1] =
            tool("unica.repository.recover", Some("recoverApply"));
        assert!(serde_json::from_value::<TaskErrorData>(integration.clone()).is_err());
        assert!(!validator.is_valid(&integration));

        let mut unlock_equal = cases()[29].2.clone();
        unlock_equal["context"]["observedLineageDigest"] = json!(A);
        assert!(serde_json::from_value::<TaskErrorData>(unlock_equal.clone()).is_err());
        assert!(validator.is_valid(&unlock_equal));

        let mut recovery_equal = cases()[29].2.clone();
        recovery_equal["context"] = json!({
            "contextKind": "integrationSet",
            "phase": "recoveryRequired",
            "expectedLineageDigest": A,
            "observedLineageDigest": A,
            "mismatchKinds": ["commitSet"],
            "exitKind": "recovery",
            "recoveryDigest": C,
        });
        recovery_equal["allowedNextActions"][1] =
            tool("unica.repository.recover", Some("recoverApply"));
        assert!(serde_json::from_value::<TaskErrorData>(recovery_equal.clone()).is_err());
        assert!(validator.is_valid(&recovery_equal));

        for (index, expected_field, observed_field) in [
            (5, "expectedInputDigest", "observedInputDigest"),
            (8, "expectedDigest", "observedDigest"),
            (9, "expectedDigest", "observedDigest"),
            (16, "expectedDigest", "observedDigest"),
            (17, "expectedDigest", "observedDigest"),
        ] {
            let mut equal = cases()[index].2.clone();
            equal["context"][observed_field] = equal["context"][expected_field].clone();
            assert!(
                serde_json::from_value::<TaskErrorData>(equal.clone()).is_err(),
                "accepted equal semantic mismatch at row {index}: {equal}"
            );
            assert!(
                validator.is_valid(&equal),
                "schema should retain the documented relational superset at row {index}: {equal}"
            );
        }
    }

    #[test]
    fn state_corrupt_has_exactly_five_closed_trusted_state_reference_shapes() {
        let state_refs = [
            (
                json!({
                    "stateRefKind": "workspace",
                    "workspaceIdentityDigest": C,
                }),
                "workspaceIdentityDigest",
            ),
            (
                json!({
                    "stateRefKind": "startAttempt",
                    "workspaceIdentityDigest": C,
                    "taskId": "TASK-1",
                    "operationId": OP,
                }),
                "operationId",
            ),
            (
                json!({
                    "stateRefKind": "project",
                    "projectId": ID1,
                }),
                "projectId",
            ),
            (
                json!({
                    "stateRefKind": "task",
                    "projectId": ID1,
                    "taskId": "TASK-1",
                    "instanceId": ID2,
                }),
                "instanceId",
            ),
            (
                json!({
                    "stateRefKind": "taskOperation",
                    "projectId": ID1,
                    "taskId": "TASK-1",
                    "instanceId": ID2,
                    "operationId": OP,
                }),
                "operationId",
            ),
        ];
        let validator = schema_validator::<TaskErrorData>();

        for (state_ref, required_field) in state_refs {
            let valid = leaf(
                "stateCorrupt",
                json!({
                    "contextKind": "stateCorrupt",
                    "stateRef": state_ref,
                    "expectedDigest": A,
                    "observation": {
                        "observationKind": "exactBytes",
                        "observedDigest": B,
                    },
                }),
                status_only(),
            );
            assert!(serde_json::from_value::<TaskErrorData>(valid.clone()).is_ok());
            assert!(validator.is_valid(&valid), "schema rejected {valid}");

            let mut missing = valid.clone();
            missing["context"]["stateRef"]
                .as_object_mut()
                .unwrap()
                .remove(required_field);
            assert!(serde_json::from_value::<TaskErrorData>(missing.clone()).is_err());
            assert!(!validator.is_valid(&missing), "schema accepted {missing}");

            let mut extra = valid;
            extra["context"]["stateRef"]["pathRecoveredFromCorruptBytes"] = json!("/untrusted");
            assert!(serde_json::from_value::<TaskErrorData>(extra.clone()).is_err());
            assert!(!validator.is_valid(&extra), "schema accepted {extra}");
        }
    }

    #[test]
    fn state_corrupt_observation_is_exact_bytes_or_typed_unavailable_without_sentinels() {
        let validator = schema_validator::<TaskErrorData>();
        let base_context = json!({
            "contextKind": "stateCorrupt",
            "stateRef": {
                "stateRefKind": "project",
                "projectId": ID1,
            },
            "expectedDigest": A,
        });
        let state_corrupt = |observation: Value| {
            let mut context = base_context.clone();
            context["observation"] = observation;
            leaf("stateCorrupt", context, status_only())
        };

        let exact = state_corrupt(json!({
            "observationKind": "exactBytes",
            "observedDigest": B,
        }));
        let missing = state_corrupt(json!({
            "observationKind": "unavailable",
            "reason": "missing",
        }));
        let permission_denied = state_corrupt(json!({
            "observationKind": "unavailable",
            "reason": "permissionDenied",
        }));
        for valid in [&exact, &missing, &permission_denied] {
            assert!(
                serde_json::from_value::<TaskErrorData>(valid.clone()).is_ok(),
                "Serde rejected state observation: {valid}"
            );
            assert!(validator.is_valid(valid), "schema rejected {valid}");
        }

        let mut equal_exact = exact.clone();
        equal_exact["context"]["observation"]["observedDigest"] = json!(A);
        assert!(serde_json::from_value::<TaskErrorData>(equal_exact.clone()).is_err());
        assert!(validator.is_valid(&equal_exact));

        let mut omitted_observation = exact.clone();
        omitted_observation["context"]
            .as_object_mut()
            .unwrap()
            .remove("observation");
        let exact_missing_digest = state_corrupt(json!({
            "observationKind": "exactBytes",
        }));
        let exact_with_reason = state_corrupt(json!({
            "observationKind": "exactBytes",
            "observedDigest": B,
            "reason": "missing",
        }));
        let unavailable_missing_reason = state_corrupt(json!({
            "observationKind": "unavailable",
        }));
        let unavailable_with_digest = state_corrupt(json!({
            "observationKind": "unavailable",
            "reason": "missing",
            "observedDigest": B,
        }));
        let unknown_reason = state_corrupt(json!({
            "observationKind": "unavailable",
            "reason": "ioError",
        }));
        let unknown_kind = state_corrupt(json!({
            "observationKind": "metadataOnly",
            "observedDigest": B,
        }));
        for invalid in [
            omitted_observation,
            exact_missing_digest,
            exact_with_reason,
            unavailable_missing_reason,
            unavailable_with_digest,
            unknown_reason,
            unknown_kind,
        ] {
            assert!(
                serde_json::from_value::<TaskErrorData>(invalid.clone()).is_err(),
                "Serde accepted invalid state observation: {invalid}"
            );
            assert!(
                !validator.is_valid(&invalid),
                "schema accepted invalid state observation: {invalid}"
            );
        }
    }

    #[test]
    fn workspace_required_null_and_integration_exit_variants_are_physical() {
        let workspace = cases()[26].2.clone();
        let mut missing = workspace.clone();
        missing["context"]
            .as_object_mut()
            .unwrap()
            .remove("observedMarkerDigest");
        assert!(serde_json::from_value::<TaskErrorData>(missing.clone()).is_err());
        assert!(!schema_validator::<TaskErrorData>().is_valid(&missing));

        let mut non_null_missing = workspace;
        non_null_missing["context"]["observedMarkerDigest"] = json!(B);
        assert!(serde_json::from_value::<TaskErrorData>(non_null_missing.clone()).is_err());
        assert!(!schema_validator::<TaskErrorData>().is_valid(&non_null_missing));

        let mut recovery_exit = cases()[29].2.clone();
        recovery_exit["context"] = json!({
            "contextKind": "integrationSet",
            "phase": "recoveryRequired",
            "expectedLineageDigest": A,
            "observedLineageDigest": B,
            "mismatchKinds": ["commitSet"],
            "exitKind": "recovery",
            "recoveryDigest": C,
        });
        recovery_exit["allowedNextActions"][1] =
            tool("unica.repository.recover", Some("recoverApply"));
        assert!(serde_json::from_value::<TaskErrorData>(recovery_exit.clone()).is_ok());
        assert!(schema_validator::<TaskErrorData>().is_valid(&recovery_exit));

        recovery_exit["context"]["lockSetId"] = json!(ID1);
        assert!(serde_json::from_value::<TaskErrorData>(recovery_exit).is_err());
    }

    #[test]
    fn all_17_workspace_mismatch_presence_combinations_are_physical() {
        let validator = schema_validator::<TaskErrorData>();
        let mut count = 0;
        for project_mismatch in [false, true] {
            for marker_state in 0..=2 {
                for lease_state in 0..=2 {
                    if !project_mismatch && marker_state == 0 && lease_state == 0 {
                        continue;
                    }
                    count += 1;
                    let mut context = serde_json::Map::from_iter([
                        ("contextKind".to_owned(), json!("taskContext")),
                        ("requestedTaskId".to_owned(), json!("TASK-1")),
                        ("requestedToolName".to_owned(), json!("unica.merge.verify")),
                    ]);
                    let mut mismatch_kinds = Vec::new();
                    if project_mismatch {
                        mismatch_kinds.push(json!("projectMismatch"));
                        context.insert("expectedProjectId".to_owned(), json!(ID1));
                        context.insert("observedProjectId".to_owned(), json!(ID2));
                    }
                    if marker_state != 0 {
                        mismatch_kinds.push(json!(if marker_state == 1 {
                            "markerMissing"
                        } else {
                            "markerMismatch"
                        }));
                        context.insert("expectedMarkerDigest".to_owned(), json!(A));
                        context.insert(
                            "observedMarkerDigest".to_owned(),
                            if marker_state == 1 {
                                Value::Null
                            } else {
                                json!(B)
                            },
                        );
                    }
                    if lease_state != 0 {
                        mismatch_kinds.push(json!(if lease_state == 1 {
                            "leaseMissing"
                        } else {
                            "leaseInvalid"
                        }));
                        context.insert("expectedLeaseDigest".to_owned(), json!(A));
                        context.insert(
                            "observedLeaseDigest".to_owned(),
                            if lease_state == 1 {
                                Value::Null
                            } else {
                                json!(C)
                            },
                        );
                    }
                    context.insert("mismatchKinds".to_owned(), Value::Array(mismatch_kinds));
                    let value = leaf(
                        "taskWorkspaceContextInvalid",
                        Value::Object(context),
                        json!([]),
                    );
                    assert!(
                        serde_json::from_value::<TaskErrorData>(value.clone()).is_ok(),
                        "Serde rejected workspace combination: {value}"
                    );
                    assert!(validator.is_valid(&value), "schema rejected {value}");

                    if project_mismatch {
                        let mut equal = value.clone();
                        equal["context"]["observedProjectId"] = json!(ID1);
                        assert!(serde_json::from_value::<TaskErrorData>(equal.clone()).is_err());
                        assert!(validator.is_valid(&equal));
                    }
                    if marker_state == 1 {
                        let mut omitted = value.clone();
                        omitted["context"]
                            .as_object_mut()
                            .unwrap()
                            .remove("observedMarkerDigest");
                        assert!(serde_json::from_value::<TaskErrorData>(omitted.clone()).is_err());
                        assert!(!validator.is_valid(&omitted));
                    } else if marker_state == 2 {
                        let mut equal = value.clone();
                        equal["context"]["observedMarkerDigest"] = json!(A);
                        assert!(serde_json::from_value::<TaskErrorData>(equal.clone()).is_err());
                        assert!(validator.is_valid(&equal));
                    }
                    if lease_state == 1 {
                        let mut omitted = value.clone();
                        omitted["context"]
                            .as_object_mut()
                            .unwrap()
                            .remove("observedLeaseDigest");
                        assert!(serde_json::from_value::<TaskErrorData>(omitted.clone()).is_err());
                        assert!(!validator.is_valid(&omitted));
                    } else if lease_state == 2 {
                        let mut equal = value.clone();
                        equal["context"]["observedLeaseDigest"] = json!(A);
                        assert!(serde_json::from_value::<TaskErrorData>(equal.clone()).is_err());
                        assert!(validator.is_valid(&equal));
                    }

                    let exact_kinds = value["context"]["mismatchKinds"].as_array().unwrap();
                    if exact_kinds.len() > 1 {
                        let mut missing = value.clone();
                        missing["context"]["mismatchKinds"]
                            .as_array_mut()
                            .unwrap()
                            .pop();

                        let mut extra = value.clone();
                        let duplicate = exact_kinds.last().unwrap().clone();
                        extra["context"]["mismatchKinds"]
                            .as_array_mut()
                            .unwrap()
                            .push(duplicate);

                        let mut reordered = value.clone();
                        reordered["context"]["mismatchKinds"]
                            .as_array_mut()
                            .unwrap()
                            .reverse();

                        let present = exact_kinds
                            .iter()
                            .map(|kind| kind.as_str().unwrap())
                            .collect::<Vec<_>>();
                        let replacement = [
                            "projectMismatch",
                            "markerMissing",
                            "markerMismatch",
                            "leaseMissing",
                            "leaseInvalid",
                        ]
                        .into_iter()
                        .find(|candidate| !present.contains(candidate))
                        .unwrap();
                        let mut substituted = value.clone();
                        *substituted["context"]["mismatchKinds"]
                            .as_array_mut()
                            .unwrap()
                            .last_mut()
                            .unwrap() = json!(replacement);

                        for invalid in [missing, extra, reordered, substituted] {
                            assert!(
                                serde_json::from_value::<TaskErrorData>(invalid.clone()).is_err(),
                                "Serde accepted non-exact mismatch tuple: {invalid}"
                            );
                            assert!(
                                !validator.is_valid(&invalid),
                                "schema accepted non-exact mismatch tuple: {invalid}"
                            );
                        }
                    }
                }
            }
        }
        assert_eq!(count, 17);
    }

    #[test]
    fn task_error_schema_has_30_literal_leaves_and_is_recursively_closed() {
        let schema = serde_json::to_value(schema_for!(TaskErrorData)).unwrap();
        assert_eq!(schema["oneOf"].as_array().unwrap().len(), 30);
        audit_json_schema(&schema).unwrap_or_else(|error| panic!("{error}: {schema}"));
    }
}
