use super::schema::{one_of_schema, string_schema};
use crate::domain::branched_development::BranchedLifecycleToolName;
use schemars::{JsonSchema, Schema, SchemaGenerator};
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::borrow::Cow;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum CompatibleGeneralToolName {}

impl Serialize for CompatibleGeneralToolName {
    fn serialize<S>(&self, _: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match *self {}
    }
}

impl<'de> Deserialize<'de> for CompatibleGeneralToolName {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let _ = serde::de::IgnoredAny::deserialize(deserializer)?;
        Err(D::Error::custom(
            "no compatible general tool names are registered",
        ))
    }
}

impl JsonSchema for CompatibleGeneralToolName {
    fn schema_name() -> Cow<'static, str> {
        "CompatibleGeneralToolName".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        // `a^` cannot match any string, so this is a typed, auditable empty
        // string vocabulary rather than an untyped catch-all.
        string_schema(1, 1, Some("a^"), None)
    }
}

static COMPATIBLE_GENERAL_TOOL_NAMES: [CompatibleGeneralToolName; 0] = [];

pub(crate) const fn compatible_general_tool_names() -> &'static [CompatibleGeneralToolName] {
    &COMPATIBLE_GENERAL_TOOL_NAMES
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub(crate) enum TaskOperationToolName {
    Lifecycle(BranchedLifecycleToolName),
    General(CompatibleGeneralToolName),
}

impl JsonSchema for TaskOperationToolName {
    fn schema_name() -> Cow<'static, str> {
        "TaskOperationToolName".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<BranchedLifecycleToolName>(),
            generator.subschema_for::<CompatibleGeneralToolName>(),
        ])
    }
}

impl TaskOperationToolName {
    pub(crate) const fn as_str(&self) -> &'static str {
        match self {
            Self::Lifecycle(name) => name.as_str(),
            Self::General(name) => match *name {},
        }
    }
}

macro_rules! selector_single {
    ($tool_name:ident, $selector:ident, $wire:literal) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
        pub(crate) enum $tool_name {
            #[serde(rename = $wire)]
            Value,
        }

        #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        pub(crate) struct $selector {
            tool_name: $tool_name,
        }
    };
}

macro_rules! selector_multi {
    (
        $tool_name:ident,
        $request_variant:ident,
        $selector:ident,
        $tool_wire:literal,
        { $($variant:ident => $variant_wire:literal),+ $(,)? }
    ) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
        pub(crate) enum $tool_name {
            #[serde(rename = $tool_wire)]
            Value,
        }

        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
        pub(crate) enum $request_variant {
            $(#[serde(rename = $variant_wire)] $variant),+
        }

        impl $request_variant {
            pub(crate) const LITERALS: &'static [&'static str] = &[$($variant_wire),+];

            pub(crate) const fn as_str(&self) -> &'static str {
                match self {
                    $(Self::$variant => $variant_wire),+
                }
            }
        }

        #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        pub(crate) struct $selector {
            tool_name: $tool_name,
            request_variant: $request_variant,
        }
    };
}

selector_single!(
    BranchedStartToolName,
    BranchedStartSelector,
    "unica.branched.start"
);
selector_single!(
    BranchedStatusToolName,
    BranchedStatusSelector,
    "unica.branched.status"
);
selector_multi!(
    BranchedArchiveToolName,
    BranchedArchiveSelectorVariant,
    BranchedArchiveSelector,
    "unica.branched.archive",
    {
        SuccessPreview => "successPreview",
        SuccessApply => "successApply",
        AbandonedPreview => "abandonedPreview",
        AbandonedApply => "abandonedApply",
    }
);
selector_multi!(
    BranchedCleanupToolName,
    BranchedCleanupSelectorVariant,
    BranchedCleanupSelector,
    "unica.branched.cleanup",
    { Preview => "preview", Apply => "apply" }
);
selector_single!(
    DeliveryInspectToolName,
    DeliveryInspectSelector,
    "unica.delivery.inspect"
);
selector_multi!(
    DeliveryCreateToolName,
    DeliveryCreateSelectorVariant,
    DeliveryCreateSelector,
    "unica.delivery.create",
    {
        BaselineDistributionPreview => "baselineDistributionPreview",
        BaselineDistributionApply => "baselineDistributionApply",
        RefreshDistributionPreview => "refreshDistributionPreview",
        RefreshDistributionApply => "refreshDistributionApply",
    }
);
selector_single!(
    DeliveryVerifyToolName,
    DeliveryVerifySelector,
    "unica.delivery.verify"
);
selector_multi!(
    DeliveryDeployToolName,
    DeliveryDeploySelectorVariant,
    DeliveryDeploySelector,
    "unica.delivery.deploy",
    { Preview => "preview", Apply => "apply" }
);
selector_multi!(
    MergeCompareToolName,
    MergeCompareSelectorVariant,
    MergeCompareSelector,
    "unica.merge.compare",
    { ProjectDelta => "projectDelta", MainIntegration => "mainIntegration" }
);
selector_multi!(
    MergePrepareToolName,
    MergePrepareSelectorVariant,
    MergePrepareSelector,
    "unica.merge.prepare",
    {
        SupportedUpdate => "supportedUpdate",
        SupportedUpdateReplacement => "supportedUpdateReplacement",
        ResolvedReplay => "resolvedReplay",
        MainIntegration => "mainIntegration",
    }
);
selector_single!(
    MergeConflictsToolName,
    MergeConflictsSelector,
    "unica.merge.conflicts"
);
selector_multi!(
    MergeResolveToolName,
    MergeResolveSelectorVariant,
    MergeResolveSelector,
    "unica.merge.resolve",
    {
        TakeOurs => "takeOurs",
        TakeTheirs => "takeTheirs",
        Combine => "combine",
        Manual => "manual",
        AdaptedDelta => "adaptedDelta",
    }
);
selector_multi!(
    MergeApplyToolName,
    MergeApplySelectorVariant,
    MergeApplySelector,
    "unica.merge.apply",
    { Task => "task", Original => "original" }
);
selector_multi!(
    MergeVerifyToolName,
    MergeVerifySelectorVariant,
    MergeVerifySelector,
    "unica.merge.verify",
    {
        LocalCheckpoint => "localCheckpoint",
        SynchronizedTask => "synchronizedTask",
        SynchronizedTaskAdapted => "synchronizedTaskAdapted",
        MainSandbox => "mainSandbox",
        MainIntegration => "mainIntegration",
    }
);
selector_single!(
    RepositoryStatusToolName,
    RepositoryStatusSelector,
    "unica.repository.status"
);
selector_multi!(
    RepositoryUpdateToolName,
    RepositoryUpdateSelectorVariant,
    RepositoryUpdateSelector,
    "unica.repository.update",
    {
        RoutinePreview => "routinePreview",
        RoutineApply => "routineApply",
        ArmPreview => "armPreview",
        ArmApply => "armApply",
        PrerequisitePreview => "prerequisitePreview",
        PrerequisiteApply => "prerequisiteApply",
        CancellationPreview => "cancellationPreview",
        CancellationApply => "cancellationApply",
    }
);
selector_single!(
    RepositoryPlanLocksToolName,
    RepositoryPlanLocksSelector,
    "unica.repository.planLocks"
);
selector_single!(
    RepositoryLockToolName,
    RepositoryLockSelector,
    "unica.repository.lock"
);
selector_multi!(
    RepositoryUnlockToolName,
    RepositoryUnlockSelectorVariant,
    RepositoryUnlockSelector,
    "unica.repository.unlock",
    {
        Compensation => "compensation",
        Rollback => "rollback",
        Abandonment => "abandonment",
    }
);
selector_multi!(
    RepositoryCommitToolName,
    RepositoryCommitSelectorVariant,
    RepositoryCommitSelector,
    "unica.repository.commit",
    { Preview => "preview", Apply => "apply" }
);
selector_multi!(
    RepositoryRecoverToolName,
    RepositoryRecoverSelectorVariant,
    RepositoryRecoverSelector,
    "unica.repository.recover",
    { RecoverApply => "recoverApply", RecoverCancel => "recoverCancel" }
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub(crate) enum TaskOperationSelector {
    BranchedStart(BranchedStartSelector),
    BranchedStatus(BranchedStatusSelector),
    BranchedArchive(BranchedArchiveSelector),
    BranchedCleanup(BranchedCleanupSelector),
    DeliveryInspect(DeliveryInspectSelector),
    DeliveryCreate(DeliveryCreateSelector),
    DeliveryVerify(DeliveryVerifySelector),
    DeliveryDeploy(DeliveryDeploySelector),
    MergeCompare(MergeCompareSelector),
    MergePrepare(MergePrepareSelector),
    MergeConflicts(MergeConflictsSelector),
    MergeResolve(MergeResolveSelector),
    MergeApply(MergeApplySelector),
    MergeVerify(MergeVerifySelector),
    RepositoryStatus(RepositoryStatusSelector),
    RepositoryUpdate(RepositoryUpdateSelector),
    RepositoryPlanLocks(RepositoryPlanLocksSelector),
    RepositoryLock(RepositoryLockSelector),
    RepositoryUnlock(RepositoryUnlockSelector),
    RepositoryCommit(RepositoryCommitSelector),
    RepositoryRecover(RepositoryRecoverSelector),
}

impl JsonSchema for TaskOperationSelector {
    fn schema_name() -> Cow<'static, str> {
        "TaskOperationSelector".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<BranchedStartSelector>(),
            generator.subschema_for::<BranchedStatusSelector>(),
            generator.subschema_for::<BranchedArchiveSelector>(),
            generator.subschema_for::<BranchedCleanupSelector>(),
            generator.subschema_for::<DeliveryInspectSelector>(),
            generator.subschema_for::<DeliveryCreateSelector>(),
            generator.subschema_for::<DeliveryVerifySelector>(),
            generator.subschema_for::<DeliveryDeploySelector>(),
            generator.subschema_for::<MergeCompareSelector>(),
            generator.subschema_for::<MergePrepareSelector>(),
            generator.subschema_for::<MergeConflictsSelector>(),
            generator.subschema_for::<MergeResolveSelector>(),
            generator.subschema_for::<MergeApplySelector>(),
            generator.subschema_for::<MergeVerifySelector>(),
            generator.subschema_for::<RepositoryStatusSelector>(),
            generator.subschema_for::<RepositoryUpdateSelector>(),
            generator.subschema_for::<RepositoryPlanLocksSelector>(),
            generator.subschema_for::<RepositoryLockSelector>(),
            generator.subschema_for::<RepositoryUnlockSelector>(),
            generator.subschema_for::<RepositoryCommitSelector>(),
            generator.subschema_for::<RepositoryRecoverSelector>(),
        ])
    }
}

impl TaskOperationSelector {
    pub(crate) const fn tool_name(&self) -> BranchedLifecycleToolName {
        match self {
            Self::BranchedStart(_) => BranchedLifecycleToolName::BranchedStart,
            Self::BranchedStatus(_) => BranchedLifecycleToolName::BranchedStatus,
            Self::BranchedArchive(_) => BranchedLifecycleToolName::BranchedArchive,
            Self::BranchedCleanup(_) => BranchedLifecycleToolName::BranchedCleanup,
            Self::DeliveryInspect(_) => BranchedLifecycleToolName::DeliveryInspect,
            Self::DeliveryCreate(_) => BranchedLifecycleToolName::DeliveryCreate,
            Self::DeliveryVerify(_) => BranchedLifecycleToolName::DeliveryVerify,
            Self::DeliveryDeploy(_) => BranchedLifecycleToolName::DeliveryDeploy,
            Self::MergeCompare(_) => BranchedLifecycleToolName::MergeCompare,
            Self::MergePrepare(_) => BranchedLifecycleToolName::MergePrepare,
            Self::MergeConflicts(_) => BranchedLifecycleToolName::MergeConflicts,
            Self::MergeResolve(_) => BranchedLifecycleToolName::MergeResolve,
            Self::MergeApply(_) => BranchedLifecycleToolName::MergeApply,
            Self::MergeVerify(_) => BranchedLifecycleToolName::MergeVerify,
            Self::RepositoryStatus(_) => BranchedLifecycleToolName::RepositoryStatus,
            Self::RepositoryUpdate(_) => BranchedLifecycleToolName::RepositoryUpdate,
            Self::RepositoryPlanLocks(_) => BranchedLifecycleToolName::RepositoryPlanLocks,
            Self::RepositoryLock(_) => BranchedLifecycleToolName::RepositoryLock,
            Self::RepositoryUnlock(_) => BranchedLifecycleToolName::RepositoryUnlock,
            Self::RepositoryCommit(_) => BranchedLifecycleToolName::RepositoryCommit,
            Self::RepositoryRecover(_) => BranchedLifecycleToolName::RepositoryRecover,
        }
    }

    pub(crate) const fn request_variant(&self) -> Option<&'static str> {
        match self {
            Self::BranchedStart(_)
            | Self::BranchedStatus(_)
            | Self::DeliveryInspect(_)
            | Self::DeliveryVerify(_)
            | Self::MergeConflicts(_)
            | Self::RepositoryStatus(_)
            | Self::RepositoryPlanLocks(_)
            | Self::RepositoryLock(_) => None,
            Self::BranchedArchive(selector) => Some(selector.request_variant.as_str()),
            Self::BranchedCleanup(selector) => Some(selector.request_variant.as_str()),
            Self::DeliveryCreate(selector) => Some(selector.request_variant.as_str()),
            Self::DeliveryDeploy(selector) => Some(selector.request_variant.as_str()),
            Self::MergeCompare(selector) => Some(selector.request_variant.as_str()),
            Self::MergePrepare(selector) => Some(selector.request_variant.as_str()),
            Self::MergeResolve(selector) => Some(selector.request_variant.as_str()),
            Self::MergeApply(selector) => Some(selector.request_variant.as_str()),
            Self::MergeVerify(selector) => Some(selector.request_variant.as_str()),
            Self::RepositoryUpdate(selector) => Some(selector.request_variant.as_str()),
            Self::RepositoryUnlock(selector) => Some(selector.request_variant.as_str()),
            Self::RepositoryCommit(selector) => Some(selector.request_variant.as_str()),
            Self::RepositoryRecover(selector) => Some(selector.request_variant.as_str()),
        }
    }
}

pub(crate) const fn lifecycle_selector_variant_literals(
    tool_name: BranchedLifecycleToolName,
) -> &'static [&'static str] {
    match tool_name {
        BranchedLifecycleToolName::BranchedStart
        | BranchedLifecycleToolName::BranchedStatus
        | BranchedLifecycleToolName::DeliveryInspect
        | BranchedLifecycleToolName::DeliveryVerify
        | BranchedLifecycleToolName::MergeConflicts
        | BranchedLifecycleToolName::RepositoryStatus
        | BranchedLifecycleToolName::RepositoryPlanLocks
        | BranchedLifecycleToolName::RepositoryLock => &[],
        BranchedLifecycleToolName::BranchedArchive => BranchedArchiveSelectorVariant::LITERALS,
        BranchedLifecycleToolName::BranchedCleanup => BranchedCleanupSelectorVariant::LITERALS,
        BranchedLifecycleToolName::DeliveryCreate => DeliveryCreateSelectorVariant::LITERALS,
        BranchedLifecycleToolName::DeliveryDeploy => DeliveryDeploySelectorVariant::LITERALS,
        BranchedLifecycleToolName::MergeCompare => MergeCompareSelectorVariant::LITERALS,
        BranchedLifecycleToolName::MergePrepare => MergePrepareSelectorVariant::LITERALS,
        BranchedLifecycleToolName::MergeResolve => MergeResolveSelectorVariant::LITERALS,
        BranchedLifecycleToolName::MergeApply => MergeApplySelectorVariant::LITERALS,
        BranchedLifecycleToolName::MergeVerify => MergeVerifySelectorVariant::LITERALS,
        BranchedLifecycleToolName::RepositoryUpdate => RepositoryUpdateSelectorVariant::LITERALS,
        BranchedLifecycleToolName::RepositoryUnlock => RepositoryUnlockSelectorVariant::LITERALS,
        BranchedLifecycleToolName::RepositoryCommit => RepositoryCommitSelectorVariant::LITERALS,
        BranchedLifecycleToolName::RepositoryRecover => RepositoryRecoverSelectorVariant::LITERALS,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        compatible_general_tool_names, lifecycle_selector_variant_literals,
        CompatibleGeneralToolName, TaskOperationSelector, TaskOperationToolName,
    };
    use crate::domain::branched_development::contracts::registry::canonical_selector_ordinal;
    use crate::domain::branched_development::contracts::schema::audit_json_schema;
    use crate::domain::branched_development::BranchedLifecycleToolName;
    use schemars::{schema_for, JsonSchema};
    use serde::de::DeserializeOwned;
    use serde_json::{json, Value};

    const EXPECTED_SELECTORS: &[(&str, Option<&str>)] = &[
        ("unica.branched.start", None),
        ("unica.branched.status", None),
        ("unica.branched.archive", Some("successPreview")),
        ("unica.branched.archive", Some("successApply")),
        ("unica.branched.archive", Some("abandonedPreview")),
        ("unica.branched.archive", Some("abandonedApply")),
        ("unica.branched.cleanup", Some("preview")),
        ("unica.branched.cleanup", Some("apply")),
        ("unica.delivery.inspect", None),
        ("unica.delivery.create", Some("baselineDistributionPreview")),
        ("unica.delivery.create", Some("baselineDistributionApply")),
        ("unica.delivery.create", Some("refreshDistributionPreview")),
        ("unica.delivery.create", Some("refreshDistributionApply")),
        ("unica.delivery.verify", None),
        ("unica.delivery.deploy", Some("preview")),
        ("unica.delivery.deploy", Some("apply")),
        ("unica.merge.compare", Some("projectDelta")),
        ("unica.merge.compare", Some("mainIntegration")),
        ("unica.merge.prepare", Some("supportedUpdate")),
        ("unica.merge.prepare", Some("supportedUpdateReplacement")),
        ("unica.merge.prepare", Some("resolvedReplay")),
        ("unica.merge.prepare", Some("mainIntegration")),
        ("unica.merge.conflicts", None),
        ("unica.merge.resolve", Some("takeOurs")),
        ("unica.merge.resolve", Some("takeTheirs")),
        ("unica.merge.resolve", Some("combine")),
        ("unica.merge.resolve", Some("manual")),
        ("unica.merge.resolve", Some("adaptedDelta")),
        ("unica.merge.apply", Some("task")),
        ("unica.merge.apply", Some("original")),
        ("unica.merge.verify", Some("localCheckpoint")),
        ("unica.merge.verify", Some("synchronizedTask")),
        ("unica.merge.verify", Some("synchronizedTaskAdapted")),
        ("unica.merge.verify", Some("mainSandbox")),
        ("unica.merge.verify", Some("mainIntegration")),
        ("unica.repository.status", None),
        ("unica.repository.update", Some("routinePreview")),
        ("unica.repository.update", Some("routineApply")),
        ("unica.repository.update", Some("armPreview")),
        ("unica.repository.update", Some("armApply")),
        ("unica.repository.update", Some("prerequisitePreview")),
        ("unica.repository.update", Some("prerequisiteApply")),
        ("unica.repository.update", Some("cancellationPreview")),
        ("unica.repository.update", Some("cancellationApply")),
        ("unica.repository.planLocks", None),
        ("unica.repository.lock", None),
        ("unica.repository.unlock", Some("compensation")),
        ("unica.repository.unlock", Some("rollback")),
        ("unica.repository.unlock", Some("abandonment")),
        ("unica.repository.commit", Some("preview")),
        ("unica.repository.commit", Some("apply")),
        ("unica.repository.recover", Some("recoverApply")),
        ("unica.repository.recover", Some("recoverCancel")),
    ];

    fn selector_json(tool_name: &str, request_variant: Option<&str>) -> Value {
        match request_variant {
            Some(request_variant) => {
                json!({ "toolName": tool_name, "requestVariant": request_variant })
            }
            None => json!({ "toolName": tool_name }),
        }
    }

    fn rejects<T: DeserializeOwned>(value: Value) {
        assert!(
            serde_json::from_value::<T>(value.clone()).is_err(),
            "selector contract accepted {value}"
        );
    }

    fn validator<T: JsonSchema>() -> jsonschema::Validator {
        jsonschema::options()
            .with_draft(jsonschema::Draft::Draft202012)
            .build(&serde_json::to_value(schema_for!(T)).unwrap())
            .unwrap()
    }

    #[test]
    fn exact_fifty_three_selectors_round_trip_in_canonical_order() {
        assert_eq!(EXPECTED_SELECTORS.len(), 53);
        let schema = validator::<TaskOperationSelector>();
        for (index, (tool_name, request_variant)) in EXPECTED_SELECTORS.iter().enumerate() {
            let value = selector_json(tool_name, *request_variant);
            let parsed: TaskOperationSelector = serde_json::from_value(value.clone()).unwrap();
            assert_eq!(serde_json::to_value(&parsed).unwrap(), value);
            assert_eq!(parsed.tool_name().as_str(), *tool_name);
            assert_eq!(parsed.request_variant(), *request_variant);
            assert_eq!(canonical_selector_ordinal(&parsed), index);
            assert!(schema.is_valid(&value));
        }
    }

    #[test]
    fn single_and_multi_variant_tools_reject_every_presence_and_cross_tool_error() {
        let all_variant_literals = EXPECTED_SELECTORS
            .iter()
            .filter_map(|(_, request_variant)| *request_variant)
            .collect::<std::collections::BTreeSet<_>>();
        for tool in BranchedLifecycleToolName::ALL {
            let allowed = lifecycle_selector_variant_literals(*tool);
            if allowed.is_empty() {
                rejects::<TaskOperationSelector>(selector_json(tool.as_str(), Some("preview")));
            } else {
                rejects::<TaskOperationSelector>(selector_json(tool.as_str(), None));
            }
            for request_variant in &all_variant_literals {
                let value = selector_json(tool.as_str(), Some(request_variant));
                if allowed.contains(request_variant) {
                    let _: TaskOperationSelector = serde_json::from_value(value).unwrap();
                } else {
                    rejects::<TaskOperationSelector>(value);
                }
            }
        }

        for invalid in [
            json!({}),
            json!({ "toolName": "unica.unknown" }),
            json!({ "toolName": "unica.branched.start", "extra": true }),
            json!({ "toolName": "unica.merge.apply", "requestVariant": null }),
            json!({ "toolName": "unica.merge.apply", "requestVariant": "preview" }),
        ] {
            rejects::<TaskOperationSelector>(invalid.clone());
            assert!(!validator::<TaskOperationSelector>().is_valid(&invalid));
        }
    }

    #[test]
    fn task_operation_tool_name_is_closed_and_general_registry_is_explicitly_empty() {
        assert!(compatible_general_tool_names().is_empty());
        for name in BranchedLifecycleToolName::ALL {
            let value = json!(name.as_str());
            let parsed: TaskOperationToolName = serde_json::from_value(value.clone()).unwrap();
            assert_eq!(serde_json::to_value(parsed).unwrap(), value);
        }
        for invalid in [
            json!("unica.project.status"),
            json!("unica.unknown"),
            json!(7),
        ] {
            rejects::<TaskOperationToolName>(invalid.clone());
            assert!(!validator::<TaskOperationToolName>().is_valid(&invalid));
            rejects::<CompatibleGeneralToolName>(invalid);
        }
    }

    #[test]
    fn selector_and_tool_name_schemas_are_closed_exact_unions() {
        let selector_schema = serde_json::to_value(schema_for!(TaskOperationSelector)).unwrap();
        audit_json_schema(&selector_schema).unwrap();
        assert_eq!(selector_schema["oneOf"].as_array().unwrap().len(), 21);
        audit_json_schema(&serde_json::to_value(schema_for!(TaskOperationToolName)).unwrap())
            .unwrap();
    }
}
