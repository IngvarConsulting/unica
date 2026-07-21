use super::requests::delivery::{
    DeliveryCreateRequest, DeliveryDeployRequest, DeliveryInspectRequest, DeliveryVerifyRequest,
};
use super::requests::merge::{
    MergeApplyRequest, MergeCompareRequest, MergeConflictsRequest, MergePrepareRequest,
    MergeResolveRequest, MergeVerifyRequest,
};
use super::requests::repository::{
    RepositoryCommitRequest, RepositoryLockRequest, RepositoryPlanLocksRequest,
    RepositoryRecoverRequest, RepositoryStatusRequest, RepositoryUnlockRequest,
    RepositoryUpdateRequest,
};
use super::requests::task::{
    BranchedArchiveRequest, BranchedCleanupRequest, BranchedStartRequest, BranchedStatusRequest,
};
use super::selectors::TaskOperationSelector;
use crate::domain::branched_development::{BranchedLifecycleToolName, ExecutionPolicy};
use schemars::{JsonSchema, Schema};
use serde_json::Value;
use std::convert::Infallible;

pub(crate) type RequestSchemaFactory = fn() -> Schema;
pub(crate) type PolicySelector = fn(&Value) -> Option<ExecutionPolicy>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HandlerBindingState {
    Absent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RequestVariantMetadata {
    request_variant: Option<&'static str>,
    policy: ExecutionPolicy,
    mutation: bool,
    preview: bool,
}

impl RequestVariantMetadata {
    const fn new(
        request_variant: Option<&'static str>,
        policy: ExecutionPolicy,
        mutation: bool,
        preview: bool,
    ) -> Self {
        Self {
            request_variant,
            policy,
            mutation,
            preview,
        }
    }

    pub(crate) const fn request_variant(&self) -> Option<&'static str> {
        self.request_variant
    }

    pub(crate) const fn policy(&self) -> ExecutionPolicy {
        self.policy
    }

    pub(crate) const fn is_mutation(&self) -> bool {
        self.mutation
    }

    pub(crate) const fn is_preview(&self) -> bool {
        self.preview
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct BranchedToolDescriptor {
    name: BranchedLifecycleToolName,
    request_schema: RequestSchemaFactory,
    variants: &'static [RequestVariantMetadata],
    policy_selector: PolicySelector,
    // Infallible has no values, so this phase cannot accidentally smuggle a
    // callable handler through the contract registry.
    handler_binding: Option<Infallible>,
}

impl BranchedToolDescriptor {
    const fn new(
        name: BranchedLifecycleToolName,
        request_schema: RequestSchemaFactory,
        variants: &'static [RequestVariantMetadata],
        policy_selector: PolicySelector,
    ) -> Self {
        Self {
            name,
            request_schema,
            variants,
            policy_selector,
            handler_binding: None,
        }
    }

    pub(crate) const fn name(&self) -> BranchedLifecycleToolName {
        self.name
    }

    pub(crate) fn request_schema(&self) -> Schema {
        (self.request_schema)()
    }

    pub(crate) const fn variants(&self) -> &'static [RequestVariantMetadata] {
        self.variants
    }

    pub(crate) fn select_policy(&self, request: &Value) -> Option<ExecutionPolicy> {
        (self.policy_selector)(request)
    }

    pub(crate) const fn handler_binding_state(&self) -> HandlerBindingState {
        HandlerBindingState::Absent
    }

    pub(crate) const fn has_handler_binding(&self) -> bool {
        self.handler_binding.is_some()
    }
}

fn request_schema<T: JsonSchema>() -> Schema {
    schemars::schema_for!(T)
}

macro_rules! metadata {
    ($name:ident = [$(($variant:expr, $policy:ident, $mutation:literal, $preview:literal)),+ $(,)?]) => {
        const $name: &[RequestVariantMetadata] = &[
            $(RequestVariantMetadata::new(
                $variant,
                ExecutionPolicy::$policy,
                $mutation,
                $preview,
            )),+
        ];
    };
}

metadata!(BRANCHED_START_VARIANTS = [(None, LocalJournaled, true, false)]);
metadata!(BRANCHED_STATUS_VARIANTS = [(None, ReadOnly, false, false)]);
metadata!(
    BRANCHED_ARCHIVE_VARIANTS = [
        (Some("successPreview"), PreviewedJournaledEffect, true, true),
        (Some("successApply"), PreviewedJournaledEffect, true, false),
        (
            Some("abandonedPreview"),
            PreviewedJournaledEffect,
            true,
            true
        ),
        (
            Some("abandonedApply"),
            PreviewedJournaledEffect,
            true,
            false
        ),
    ]
);
metadata!(
    BRANCHED_CLEANUP_VARIANTS = [
        (Some("preview"), PreviewedJournaledEffect, true, true),
        (Some("apply"), PreviewedJournaledEffect, true, false),
    ]
);
metadata!(DELIVERY_INSPECT_VARIANTS = [(None, ReadOnly, false, false)]);
metadata!(
    DELIVERY_CREATE_VARIANTS = [
        (
            Some("baselineDistributionPreview"),
            PreviewedJournaledEffect,
            true,
            true
        ),
        (
            Some("baselineDistributionApply"),
            PreviewedJournaledEffect,
            true,
            false
        ),
        (
            Some("refreshDistributionPreview"),
            PreviewedJournaledEffect,
            true,
            true
        ),
        (
            Some("refreshDistributionApply"),
            PreviewedJournaledEffect,
            true,
            false
        ),
    ]
);
metadata!(DELIVERY_VERIFY_VARIANTS = [(None, Contained, true, false)]);
metadata!(
    DELIVERY_DEPLOY_VARIANTS = [
        (Some("preview"), PreviewedJournaledEffect, true, true),
        (Some("apply"), PreviewedJournaledEffect, true, false),
    ]
);
metadata!(
    MERGE_COMPARE_VARIANTS = [
        (Some("projectDelta"), Contained, true, false),
        (Some("mainIntegration"), Contained, true, false),
    ]
);
metadata!(
    MERGE_PREPARE_VARIANTS = [
        (Some("supportedUpdate"), Contained, true, false),
        (Some("supportedUpdateReplacement"), Contained, true, false),
        (Some("resolvedReplay"), Contained, true, false),
        (Some("mainIntegration"), JournaledEffect, true, false),
    ]
);
metadata!(MERGE_CONFLICTS_VARIANTS = [(None, ReadOnly, false, false)]);
metadata!(
    MERGE_RESOLVE_VARIANTS = [
        (Some("takeOurs"), LocalJournaled, true, false),
        (Some("takeTheirs"), LocalJournaled, true, false),
        (Some("combine"), LocalJournaled, true, false),
        (Some("manual"), LocalJournaled, true, false),
        (Some("adaptedDelta"), LocalJournaled, true, false),
    ]
);
metadata!(
    MERGE_APPLY_VARIANTS = [
        (Some("task"), PreparedJournaledEffect, true, false),
        (Some("original"), PreparedJournaledEffect, true, false),
    ]
);
metadata!(
    MERGE_VERIFY_VARIANTS = [
        (Some("localCheckpoint"), Contained, true, false),
        (Some("synchronizedTask"), Contained, true, false),
        (Some("synchronizedTaskAdapted"), Contained, true, false),
        (Some("mainSandbox"), Contained, true, false),
        (Some("mainIntegration"), Contained, true, false),
    ]
);
metadata!(REPOSITORY_STATUS_VARIANTS = [(None, ReadOnly, false, false)]);
metadata!(
    REPOSITORY_UPDATE_VARIANTS = [
        (Some("routinePreview"), PreviewedJournaledEffect, true, true),
        (Some("routineApply"), PreviewedJournaledEffect, true, false),
        (Some("armPreview"), ReadOnly, false, true),
        (Some("armApply"), LocalJournaled, true, false),
        (
            Some("prerequisitePreview"),
            PreviewedJournaledEffect,
            true,
            true
        ),
        (
            Some("prerequisiteApply"),
            PreviewedJournaledEffect,
            true,
            false
        ),
        (
            Some("cancellationPreview"),
            PreviewedJournaledEffect,
            true,
            true
        ),
        (
            Some("cancellationApply"),
            PreviewedJournaledEffect,
            true,
            false
        ),
    ]
);
metadata!(REPOSITORY_PLAN_LOCKS_VARIANTS = [(None, Contained, true, false)]);
metadata!(REPOSITORY_LOCK_VARIANTS = [(None, JournaledEffect, true, false)]);
metadata!(
    REPOSITORY_UNLOCK_VARIANTS = [
        (Some("compensation"), JournaledEffect, true, false),
        (Some("rollback"), JournaledEffect, true, false),
        (Some("abandonment"), JournaledEffect, true, false),
    ]
);
metadata!(
    REPOSITORY_COMMIT_VARIANTS = [
        (Some("preview"), PreviewedJournaledEffect, true, true),
        (Some("apply"), PreviewedJournaledEffect, true, false),
    ]
);
metadata!(
    REPOSITORY_RECOVER_VARIANTS = [
        (Some("recoverApply"), JournaledEffect, true, false),
        (Some("recoverCancel"), LocalJournaled, true, false),
    ]
);

static BRANCHED_LIFECYCLE_DESCRIPTORS: [BranchedToolDescriptor; BranchedLifecycleToolName::COUNT] = [
    BranchedToolDescriptor::new(
        BranchedLifecycleToolName::BranchedStart,
        request_schema::<BranchedStartRequest>,
        BRANCHED_START_VARIANTS,
        BranchedStartRequest::execution_policy_for_json,
    ),
    BranchedToolDescriptor::new(
        BranchedLifecycleToolName::BranchedStatus,
        request_schema::<BranchedStatusRequest>,
        BRANCHED_STATUS_VARIANTS,
        BranchedStatusRequest::execution_policy_for_json,
    ),
    BranchedToolDescriptor::new(
        BranchedLifecycleToolName::BranchedArchive,
        request_schema::<BranchedArchiveRequest>,
        BRANCHED_ARCHIVE_VARIANTS,
        BranchedArchiveRequest::execution_policy_for_json,
    ),
    BranchedToolDescriptor::new(
        BranchedLifecycleToolName::BranchedCleanup,
        request_schema::<BranchedCleanupRequest>,
        BRANCHED_CLEANUP_VARIANTS,
        BranchedCleanupRequest::execution_policy_for_json,
    ),
    BranchedToolDescriptor::new(
        BranchedLifecycleToolName::DeliveryInspect,
        request_schema::<DeliveryInspectRequest>,
        DELIVERY_INSPECT_VARIANTS,
        DeliveryInspectRequest::execution_policy_for_json,
    ),
    BranchedToolDescriptor::new(
        BranchedLifecycleToolName::DeliveryCreate,
        request_schema::<DeliveryCreateRequest>,
        DELIVERY_CREATE_VARIANTS,
        DeliveryCreateRequest::execution_policy_for_json,
    ),
    BranchedToolDescriptor::new(
        BranchedLifecycleToolName::DeliveryVerify,
        request_schema::<DeliveryVerifyRequest>,
        DELIVERY_VERIFY_VARIANTS,
        DeliveryVerifyRequest::execution_policy_for_json,
    ),
    BranchedToolDescriptor::new(
        BranchedLifecycleToolName::DeliveryDeploy,
        request_schema::<DeliveryDeployRequest>,
        DELIVERY_DEPLOY_VARIANTS,
        DeliveryDeployRequest::execution_policy_for_json,
    ),
    BranchedToolDescriptor::new(
        BranchedLifecycleToolName::MergeCompare,
        request_schema::<MergeCompareRequest>,
        MERGE_COMPARE_VARIANTS,
        MergeCompareRequest::execution_policy_for_json,
    ),
    BranchedToolDescriptor::new(
        BranchedLifecycleToolName::MergePrepare,
        request_schema::<MergePrepareRequest>,
        MERGE_PREPARE_VARIANTS,
        MergePrepareRequest::execution_policy_for_json,
    ),
    BranchedToolDescriptor::new(
        BranchedLifecycleToolName::MergeConflicts,
        request_schema::<MergeConflictsRequest>,
        MERGE_CONFLICTS_VARIANTS,
        MergeConflictsRequest::execution_policy_for_json,
    ),
    BranchedToolDescriptor::new(
        BranchedLifecycleToolName::MergeResolve,
        request_schema::<MergeResolveRequest>,
        MERGE_RESOLVE_VARIANTS,
        MergeResolveRequest::execution_policy_for_json,
    ),
    BranchedToolDescriptor::new(
        BranchedLifecycleToolName::MergeApply,
        request_schema::<MergeApplyRequest>,
        MERGE_APPLY_VARIANTS,
        MergeApplyRequest::execution_policy_for_json,
    ),
    BranchedToolDescriptor::new(
        BranchedLifecycleToolName::MergeVerify,
        request_schema::<MergeVerifyRequest>,
        MERGE_VERIFY_VARIANTS,
        MergeVerifyRequest::execution_policy_for_json,
    ),
    BranchedToolDescriptor::new(
        BranchedLifecycleToolName::RepositoryStatus,
        request_schema::<RepositoryStatusRequest>,
        REPOSITORY_STATUS_VARIANTS,
        RepositoryStatusRequest::execution_policy_for_json,
    ),
    BranchedToolDescriptor::new(
        BranchedLifecycleToolName::RepositoryUpdate,
        request_schema::<RepositoryUpdateRequest>,
        REPOSITORY_UPDATE_VARIANTS,
        RepositoryUpdateRequest::execution_policy_for_json,
    ),
    BranchedToolDescriptor::new(
        BranchedLifecycleToolName::RepositoryPlanLocks,
        request_schema::<RepositoryPlanLocksRequest>,
        REPOSITORY_PLAN_LOCKS_VARIANTS,
        RepositoryPlanLocksRequest::execution_policy_for_json,
    ),
    BranchedToolDescriptor::new(
        BranchedLifecycleToolName::RepositoryLock,
        request_schema::<RepositoryLockRequest>,
        REPOSITORY_LOCK_VARIANTS,
        RepositoryLockRequest::execution_policy_for_json,
    ),
    BranchedToolDescriptor::new(
        BranchedLifecycleToolName::RepositoryUnlock,
        request_schema::<RepositoryUnlockRequest>,
        REPOSITORY_UNLOCK_VARIANTS,
        RepositoryUnlockRequest::execution_policy_for_json,
    ),
    BranchedToolDescriptor::new(
        BranchedLifecycleToolName::RepositoryCommit,
        request_schema::<RepositoryCommitRequest>,
        REPOSITORY_COMMIT_VARIANTS,
        RepositoryCommitRequest::execution_policy_for_json,
    ),
    BranchedToolDescriptor::new(
        BranchedLifecycleToolName::RepositoryRecover,
        request_schema::<RepositoryRecoverRequest>,
        REPOSITORY_RECOVER_VARIANTS,
        RepositoryRecoverRequest::execution_policy_for_json,
    ),
];

pub(crate) const fn branched_lifecycle_descriptors() -> &'static [BranchedToolDescriptor] {
    &BRANCHED_LIFECYCLE_DESCRIPTORS
}

pub(crate) fn canonical_selector_ordinal(selector: &TaskOperationSelector) -> usize {
    let mut offset = 0;
    for descriptor in branched_lifecycle_descriptors() {
        if descriptor.name == selector.tool_name() {
            return offset
                + descriptor
                    .variants
                    .iter()
                    .position(|variant| variant.request_variant == selector.request_variant())
                    .expect("selector and descriptor variants must be generated in lockstep");
        }
        offset += descriptor.variants.len();
    }
    unreachable!("every closed task operation selector has a lifecycle descriptor")
}

#[cfg(test)]
mod tests {
    use super::{branched_lifecycle_descriptors, HandlerBindingState};
    use crate::domain::branched_development::contracts::requests::{
        delivery, merge, repository, task,
    };
    use crate::domain::branched_development::contracts::schema::audit_json_schema;
    use crate::domain::branched_development::contracts::selectors::TaskOperationSelector;
    use crate::domain::branched_development::{BranchedLifecycleToolName, ExecutionPolicy};
    use serde_json::json;

    const EXPECTED_VARIANTS: &[(&str, Option<&str>, ExecutionPolicy, bool, bool)] = &[
        (
            "unica.branched.start",
            None,
            ExecutionPolicy::LocalJournaled,
            true,
            false,
        ),
        (
            "unica.branched.status",
            None,
            ExecutionPolicy::ReadOnly,
            false,
            false,
        ),
        (
            "unica.branched.archive",
            Some("successPreview"),
            ExecutionPolicy::PreviewedJournaledEffect,
            true,
            true,
        ),
        (
            "unica.branched.archive",
            Some("successApply"),
            ExecutionPolicy::PreviewedJournaledEffect,
            true,
            false,
        ),
        (
            "unica.branched.archive",
            Some("abandonedPreview"),
            ExecutionPolicy::PreviewedJournaledEffect,
            true,
            true,
        ),
        (
            "unica.branched.archive",
            Some("abandonedApply"),
            ExecutionPolicy::PreviewedJournaledEffect,
            true,
            false,
        ),
        (
            "unica.branched.cleanup",
            Some("preview"),
            ExecutionPolicy::PreviewedJournaledEffect,
            true,
            true,
        ),
        (
            "unica.branched.cleanup",
            Some("apply"),
            ExecutionPolicy::PreviewedJournaledEffect,
            true,
            false,
        ),
        (
            "unica.delivery.inspect",
            None,
            ExecutionPolicy::ReadOnly,
            false,
            false,
        ),
        (
            "unica.delivery.create",
            Some("baselineDistributionPreview"),
            ExecutionPolicy::PreviewedJournaledEffect,
            true,
            true,
        ),
        (
            "unica.delivery.create",
            Some("baselineDistributionApply"),
            ExecutionPolicy::PreviewedJournaledEffect,
            true,
            false,
        ),
        (
            "unica.delivery.create",
            Some("refreshDistributionPreview"),
            ExecutionPolicy::PreviewedJournaledEffect,
            true,
            true,
        ),
        (
            "unica.delivery.create",
            Some("refreshDistributionApply"),
            ExecutionPolicy::PreviewedJournaledEffect,
            true,
            false,
        ),
        (
            "unica.delivery.verify",
            None,
            ExecutionPolicy::Contained,
            true,
            false,
        ),
        (
            "unica.delivery.deploy",
            Some("preview"),
            ExecutionPolicy::PreviewedJournaledEffect,
            true,
            true,
        ),
        (
            "unica.delivery.deploy",
            Some("apply"),
            ExecutionPolicy::PreviewedJournaledEffect,
            true,
            false,
        ),
        (
            "unica.merge.compare",
            Some("projectDelta"),
            ExecutionPolicy::Contained,
            true,
            false,
        ),
        (
            "unica.merge.compare",
            Some("mainIntegration"),
            ExecutionPolicy::Contained,
            true,
            false,
        ),
        (
            "unica.merge.prepare",
            Some("supportedUpdate"),
            ExecutionPolicy::Contained,
            true,
            false,
        ),
        (
            "unica.merge.prepare",
            Some("supportedUpdateReplacement"),
            ExecutionPolicy::Contained,
            true,
            false,
        ),
        (
            "unica.merge.prepare",
            Some("resolvedReplay"),
            ExecutionPolicy::Contained,
            true,
            false,
        ),
        (
            "unica.merge.prepare",
            Some("mainIntegration"),
            ExecutionPolicy::JournaledEffect,
            true,
            false,
        ),
        (
            "unica.merge.conflicts",
            None,
            ExecutionPolicy::ReadOnly,
            false,
            false,
        ),
        (
            "unica.merge.resolve",
            Some("takeOurs"),
            ExecutionPolicy::LocalJournaled,
            true,
            false,
        ),
        (
            "unica.merge.resolve",
            Some("takeTheirs"),
            ExecutionPolicy::LocalJournaled,
            true,
            false,
        ),
        (
            "unica.merge.resolve",
            Some("combine"),
            ExecutionPolicy::LocalJournaled,
            true,
            false,
        ),
        (
            "unica.merge.resolve",
            Some("manual"),
            ExecutionPolicy::LocalJournaled,
            true,
            false,
        ),
        (
            "unica.merge.resolve",
            Some("adaptedDelta"),
            ExecutionPolicy::LocalJournaled,
            true,
            false,
        ),
        (
            "unica.merge.apply",
            Some("task"),
            ExecutionPolicy::PreparedJournaledEffect,
            true,
            false,
        ),
        (
            "unica.merge.apply",
            Some("original"),
            ExecutionPolicy::PreparedJournaledEffect,
            true,
            false,
        ),
        (
            "unica.merge.verify",
            Some("localCheckpoint"),
            ExecutionPolicy::Contained,
            true,
            false,
        ),
        (
            "unica.merge.verify",
            Some("synchronizedTask"),
            ExecutionPolicy::Contained,
            true,
            false,
        ),
        (
            "unica.merge.verify",
            Some("synchronizedTaskAdapted"),
            ExecutionPolicy::Contained,
            true,
            false,
        ),
        (
            "unica.merge.verify",
            Some("mainSandbox"),
            ExecutionPolicy::Contained,
            true,
            false,
        ),
        (
            "unica.merge.verify",
            Some("mainIntegration"),
            ExecutionPolicy::Contained,
            true,
            false,
        ),
        (
            "unica.repository.status",
            None,
            ExecutionPolicy::ReadOnly,
            false,
            false,
        ),
        (
            "unica.repository.update",
            Some("routinePreview"),
            ExecutionPolicy::PreviewedJournaledEffect,
            true,
            true,
        ),
        (
            "unica.repository.update",
            Some("routineApply"),
            ExecutionPolicy::PreviewedJournaledEffect,
            true,
            false,
        ),
        (
            "unica.repository.update",
            Some("armPreview"),
            ExecutionPolicy::ReadOnly,
            false,
            true,
        ),
        (
            "unica.repository.update",
            Some("armApply"),
            ExecutionPolicy::LocalJournaled,
            true,
            false,
        ),
        (
            "unica.repository.update",
            Some("prerequisitePreview"),
            ExecutionPolicy::PreviewedJournaledEffect,
            true,
            true,
        ),
        (
            "unica.repository.update",
            Some("prerequisiteApply"),
            ExecutionPolicy::PreviewedJournaledEffect,
            true,
            false,
        ),
        (
            "unica.repository.update",
            Some("cancellationPreview"),
            ExecutionPolicy::PreviewedJournaledEffect,
            true,
            true,
        ),
        (
            "unica.repository.update",
            Some("cancellationApply"),
            ExecutionPolicy::PreviewedJournaledEffect,
            true,
            false,
        ),
        (
            "unica.repository.planLocks",
            None,
            ExecutionPolicy::Contained,
            true,
            false,
        ),
        (
            "unica.repository.lock",
            None,
            ExecutionPolicy::JournaledEffect,
            true,
            false,
        ),
        (
            "unica.repository.unlock",
            Some("compensation"),
            ExecutionPolicy::JournaledEffect,
            true,
            false,
        ),
        (
            "unica.repository.unlock",
            Some("rollback"),
            ExecutionPolicy::JournaledEffect,
            true,
            false,
        ),
        (
            "unica.repository.unlock",
            Some("abandonment"),
            ExecutionPolicy::JournaledEffect,
            true,
            false,
        ),
        (
            "unica.repository.commit",
            Some("preview"),
            ExecutionPolicy::PreviewedJournaledEffect,
            true,
            true,
        ),
        (
            "unica.repository.commit",
            Some("apply"),
            ExecutionPolicy::PreviewedJournaledEffect,
            true,
            false,
        ),
        (
            "unica.repository.recover",
            Some("recoverApply"),
            ExecutionPolicy::JournaledEffect,
            true,
            false,
        ),
        (
            "unica.repository.recover",
            Some("recoverCancel"),
            ExecutionPolicy::LocalJournaled,
            true,
            false,
        ),
    ];

    #[test]
    fn descriptor_registry_has_the_authoritative_twenty_one_names_once_and_in_order() {
        let descriptors = branched_lifecycle_descriptors();
        assert_eq!(descriptors.len(), 21);
        assert_eq!(descriptors.len(), BranchedLifecycleToolName::ALL.len());
        assert_eq!(
            descriptors
                .iter()
                .map(|descriptor| descriptor.name())
                .collect::<Vec<_>>(),
            BranchedLifecycleToolName::ALL
        );
        let unique = descriptors
            .iter()
            .map(|descriptor| descriptor.name().as_str())
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(unique.len(), descriptors.len());
    }

    #[test]
    fn descriptor_variant_metadata_is_the_exact_fifty_three_entry_table() {
        let actual = branched_lifecycle_descriptors()
            .iter()
            .flat_map(|descriptor| {
                descriptor.variants().iter().map(move |variant| {
                    (
                        descriptor.name().as_str(),
                        variant.request_variant(),
                        variant.policy(),
                        variant.is_mutation(),
                        variant.is_preview(),
                    )
                })
            })
            .collect::<Vec<_>>();
        assert_eq!(actual, EXPECTED_VARIANTS);
    }

    #[test]
    fn every_descriptor_schema_is_closed_and_handler_binding_is_uninhabited() {
        for descriptor in branched_lifecycle_descriptors() {
            let schema = serde_json::to_value(descriptor.request_schema()).unwrap();
            audit_json_schema(&schema).unwrap_or_else(|error| {
                panic!(
                    "{} request schema is invalid: {error}",
                    descriptor.name().as_str()
                )
            });
            assert_eq!(
                descriptor.handler_binding_state(),
                HandlerBindingState::Absent
            );
            assert!(!descriptor.has_handler_binding());
            assert!(descriptor
                .select_policy(&json!({ "unknown": true }))
                .is_none());
        }
    }

    fn expected_request_schema(name: BranchedLifecycleToolName) -> serde_json::Value {
        macro_rules! schema {
            ($request:ty) => {
                serde_json::to_value(schemars::schema_for!($request)).unwrap()
            };
        }
        match name {
            BranchedLifecycleToolName::BranchedStart => schema!(task::BranchedStartRequest),
            BranchedLifecycleToolName::BranchedStatus => schema!(task::BranchedStatusRequest),
            BranchedLifecycleToolName::BranchedArchive => schema!(task::BranchedArchiveRequest),
            BranchedLifecycleToolName::BranchedCleanup => schema!(task::BranchedCleanupRequest),
            BranchedLifecycleToolName::DeliveryInspect => schema!(delivery::DeliveryInspectRequest),
            BranchedLifecycleToolName::DeliveryCreate => schema!(delivery::DeliveryCreateRequest),
            BranchedLifecycleToolName::DeliveryVerify => schema!(delivery::DeliveryVerifyRequest),
            BranchedLifecycleToolName::DeliveryDeploy => schema!(delivery::DeliveryDeployRequest),
            BranchedLifecycleToolName::MergeCompare => schema!(merge::MergeCompareRequest),
            BranchedLifecycleToolName::MergePrepare => schema!(merge::MergePrepareRequest),
            BranchedLifecycleToolName::MergeConflicts => schema!(merge::MergeConflictsRequest),
            BranchedLifecycleToolName::MergeResolve => schema!(merge::MergeResolveRequest),
            BranchedLifecycleToolName::MergeApply => schema!(merge::MergeApplyRequest),
            BranchedLifecycleToolName::MergeVerify => schema!(merge::MergeVerifyRequest),
            BranchedLifecycleToolName::RepositoryStatus => {
                schema!(repository::RepositoryStatusRequest)
            }
            BranchedLifecycleToolName::RepositoryUpdate => {
                schema!(repository::RepositoryUpdateRequest)
            }
            BranchedLifecycleToolName::RepositoryPlanLocks => {
                schema!(repository::RepositoryPlanLocksRequest)
            }
            BranchedLifecycleToolName::RepositoryLock => {
                schema!(repository::RepositoryLockRequest)
            }
            BranchedLifecycleToolName::RepositoryUnlock => {
                schema!(repository::RepositoryUnlockRequest)
            }
            BranchedLifecycleToolName::RepositoryCommit => {
                schema!(repository::RepositoryCommitRequest)
            }
            BranchedLifecycleToolName::RepositoryRecover => {
                schema!(repository::RepositoryRecoverRequest)
            }
        }
    }

    #[test]
    fn every_schema_factory_is_bound_to_its_named_request_type() {
        for descriptor in branched_lifecycle_descriptors() {
            assert_eq!(
                serde_json::to_value(descriptor.request_schema()).unwrap(),
                expected_request_schema(descriptor.name()),
                "{} schema factory drifted",
                descriptor.name().as_str()
            );
        }
    }

    #[test]
    fn mixed_policy_descriptors_select_from_validated_request_shapes() {
        const CWD: &str = "/original/project";
        const TASK: &str = "TASK-173";
        const OP: &str = "123e4567-e89b-12d3-a456-426614174000";
        const ID: &str = "223e4567-e89b-12d3-a456-426614174000";
        const OTHER_ID: &str = "323e4567-e89b-12d3-a456-426614174000";
        const DIGEST: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        const OTHER_DIGEST: &str =
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

        let descriptor = branched_lifecycle_descriptors()
            .iter()
            .find(|descriptor| descriptor.name() == BranchedLifecycleToolName::MergePrepare)
            .unwrap();
        for (request, policy) in [
            (
                json!({
                    "cwd": CWD,
                    "taskId": TASK,
                    "operationId": OP,
                    "mode": "supportedUpdate",
                    "checkpointId": ID,
                    "incomingDistributionId": OTHER_ID,
                    "comparisonId": ID
                }),
                ExecutionPolicy::Contained,
            ),
            (
                json!({
                    "cwd": CWD,
                    "taskId": TASK,
                    "operationId": OP,
                    "mode": "mainIntegration",
                    "checkpointId": ID,
                    "verificationId": OTHER_ID,
                    "expectedVerificationDigest": DIGEST,
                    "expectedRepositoryStatusDigest": OTHER_DIGEST
                }),
                ExecutionPolicy::JournaledEffect,
            ),
        ] {
            assert_eq!(descriptor.select_policy(&request), Some(policy));
        }

        let descriptor = branched_lifecycle_descriptors()
            .iter()
            .find(|descriptor| descriptor.name() == BranchedLifecycleToolName::RepositoryUpdate)
            .unwrap();
        for (request, policy) in [
            (
                json!({
                    "cwd": CWD,
                    "taskId": TASK,
                    "operationId": OP,
                    "mode": "routine",
                    "expectedStatusDigest": DIGEST
                }),
                ExecutionPolicy::PreviewedJournaledEffect,
            ),
            (
                json!({
                    "cwd": CWD,
                    "taskId": TASK,
                    "mode": "supportPrerequisiteArm",
                    "stage": "preview",
                    "expectedStatusDigest": DIGEST,
                    "supportActionId": ID,
                    "expectedSupportActionDigest": OTHER_DIGEST
                }),
                ExecutionPolicy::ReadOnly,
            ),
            (
                json!({
                    "cwd": CWD,
                    "taskId": TASK,
                    "operationId": OP,
                    "mode": "supportPrerequisiteArm",
                    "stage": "apply",
                    "expectedStatusDigest": DIGEST,
                    "supportActionId": ID,
                    "expectedSupportActionDigest": OTHER_DIGEST,
                    "approvedArmingDigest": DIGEST
                }),
                ExecutionPolicy::LocalJournaled,
            ),
        ] {
            assert_eq!(descriptor.select_policy(&request), Some(policy));
        }

        let descriptor = branched_lifecycle_descriptors()
            .iter()
            .find(|descriptor| descriptor.name() == BranchedLifecycleToolName::RepositoryRecover)
            .unwrap();
        for (request, policy) in [
            (
                json!({
                    "cwd": CWD,
                    "taskId": TASK,
                    "operationId": OP,
                    "decision": "apply",
                    "expectedRecoveryDigest": DIGEST,
                    "approval": { "digest": DIGEST, "decision": "apply" }
                }),
                ExecutionPolicy::JournaledEffect,
            ),
            (
                json!({
                    "cwd": CWD,
                    "taskId": TASK,
                    "operationId": OP,
                    "decision": "cancel",
                    "expectedRecoveryDigest": DIGEST
                }),
                ExecutionPolicy::LocalJournaled,
            ),
        ] {
            assert_eq!(descriptor.select_policy(&request), Some(policy));
        }
    }

    #[test]
    fn registry_selectors_and_descriptor_metadata_have_no_drift() {
        for (tool_name, request_variant, policy, mutation, preview) in EXPECTED_VARIANTS {
            let selector_value = match request_variant {
                Some(request_variant) => json!({
                    "toolName": tool_name,
                    "requestVariant": request_variant
                }),
                None => json!({ "toolName": tool_name }),
            };
            let selector: TaskOperationSelector = serde_json::from_value(selector_value).unwrap();
            let descriptor = branched_lifecycle_descriptors()
                .iter()
                .find(|descriptor| descriptor.name().as_str() == *tool_name)
                .unwrap();
            let metadata = descriptor
                .variants()
                .iter()
                .find(|metadata| metadata.request_variant() == *request_variant)
                .unwrap();
            assert_eq!(metadata.policy(), *policy);
            assert_eq!(metadata.is_mutation(), *mutation);
            assert_eq!(metadata.is_preview(), *preview);
            assert_eq!(selector.tool_name(), descriptor.name());
        }
    }
}
