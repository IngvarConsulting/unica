use super::registry::{canonical_selector_count, canonical_selector_ordinal};
use super::schema::one_of_schema;
use super::selectors::{
    BranchedArchiveSelector, BranchedArchiveSelectorVariant, BranchedStartSelector,
    BranchedStatusSelector, MergeConflictsSelector, MergeVerifySelector,
    MergeVerifySelectorVariant, RepositoryRecoverSelector, RepositoryRecoverSelectorVariant,
    RepositoryUnlockSelector, RepositoryUnlockSelectorVariant, TaskOperationSelector,
};
use schemars::{JsonSchema, Schema, SchemaGenerator};
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};
use std::borrow::Cow;

const MAX_RESULT_ITEMS: usize = 1024;

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub(crate) enum ExternalInstructionKind {
    AcquireSupportRoot,
    ReleaseRepositoryLocks,
    PerformManualSupportAction,
    CleanManualWorkingInfobase,
    CloseReservedOriginalDesigner,
    ResolveSupportConflict,
    ProvideSupportEvidence,
    DecideVendorRestriction,
}

impl ExternalInstructionKind {
    pub(crate) const ALL: &[Self] = &[
        Self::AcquireSupportRoot,
        Self::ReleaseRepositoryLocks,
        Self::PerformManualSupportAction,
        Self::CleanManualWorkingInfobase,
        Self::CloseReservedOriginalDesigner,
        Self::ResolveSupportConflict,
        Self::ProvideSupportEvidence,
        Self::DecideVendorRestriction,
    ];

    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::AcquireSupportRoot => "acquireSupportRoot",
            Self::ReleaseRepositoryLocks => "releaseRepositoryLocks",
            Self::PerformManualSupportAction => "performManualSupportAction",
            Self::CleanManualWorkingInfobase => "cleanManualWorkingInfobase",
            Self::CloseReservedOriginalDesigner => "closeReservedOriginalDesigner",
            Self::ResolveSupportConflict => "resolveSupportConflict",
            Self::ProvideSupportEvidence => "provideSupportEvidence",
            Self::DecideVendorRestriction => "decideVendorRestriction",
        }
    }

    const fn ordinal(self) -> usize {
        match self {
            Self::AcquireSupportRoot => 0,
            Self::ReleaseRepositoryLocks => 1,
            Self::PerformManualSupportAction => 2,
            Self::CleanManualWorkingInfobase => 3,
            Self::CloseReservedOriginalDesigner => 4,
            Self::ResolveSupportConflict => 5,
            Self::ProvideSupportEvidence => 6,
            Self::DecideVendorRestriction => 7,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
enum ToolCallActionKind {
    #[serde(rename = "toolCall")]
    Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
enum ExternalInstructionActionKind {
    #[serde(rename = "externalInstruction")]
    Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ToolCallNextAction {
    action_kind: ToolCallActionKind,
    operation: TaskOperationSelector,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ExternalInstructionNextAction {
    action_kind: ExternalInstructionActionKind,
    instruction_kind: ExternalInstructionKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub(crate) enum NextAction {
    ToolCall(ToolCallNextAction),
    ExternalInstruction(ExternalInstructionNextAction),
}

impl NextAction {
    pub(crate) const fn tool_call(operation: TaskOperationSelector) -> Self {
        Self::ToolCall(ToolCallNextAction {
            action_kind: ToolCallActionKind::Value,
            operation,
        })
    }

    pub(crate) const fn external_instruction(instruction_kind: ExternalInstructionKind) -> Self {
        Self::ExternalInstruction(ExternalInstructionNextAction {
            action_kind: ExternalInstructionActionKind::Value,
            instruction_kind,
        })
    }

    fn ordinal(&self) -> usize {
        match self {
            Self::ToolCall(action) => canonical_selector_ordinal(&action.operation),
            Self::ExternalInstruction(action) => {
                canonical_selector_count() + action.instruction_kind.ordinal()
            }
        }
    }

    fn operation(&self) -> Option<&TaskOperationSelector> {
        match self {
            Self::ToolCall(action) => Some(&action.operation),
            Self::ExternalInstruction(_) => None,
        }
    }
}

impl JsonSchema for NextAction {
    fn schema_name() -> Cow<'static, str> {
        "NextAction".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<ToolCallNextAction>(),
            generator.subschema_for::<ExternalInstructionNextAction>(),
        ])
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct CanonicalNextActions(Vec<NextAction>);

impl CanonicalNextActions {
    pub(crate) fn new(actions: Vec<NextAction>) -> Result<Self, &'static str> {
        if actions.len() > MAX_RESULT_ITEMS {
            return Err("allowed next actions exceed the result collection bound");
        }
        if !actions
            .windows(2)
            .all(|pair| pair[0].ordinal() < pair[1].ordinal())
        {
            return Err("allowed next actions are not canonical and duplicate-free");
        }
        Ok(Self(actions))
    }

    pub(crate) fn as_slice(&self) -> &[NextAction] {
        &self.0
    }
}

impl<'de> Deserialize<'de> for CanonicalNextActions {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::new(Vec::<NextAction>::deserialize(deserializer)?).map_err(D::Error::custom)
    }
}

impl JsonSchema for CanonicalNextActions {
    fn schema_name() -> Cow<'static, str> {
        "CanonicalNextActions".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        let item = generator.subschema_for::<NextAction>();
        schemars::json_schema!({
            "type": "array",
            "items": item,
            "maxItems": MAX_RESULT_ITEMS,
            "uniqueItems": true,
        })
    }
}

fn tool_call_schema(operation: &TaskOperationSelector) -> Schema {
    let tool_name = operation.tool_name().as_str();
    let operation_schema = match operation.request_variant() {
        Some(request_variant) => schemars::json_schema!({
            "type": "object",
            "properties": {
                "toolName": { "type": "string", "const": tool_name },
                "requestVariant": { "type": "string", "const": request_variant }
            },
            "required": ["toolName", "requestVariant"],
            "additionalProperties": false
        }),
        None => schemars::json_schema!({
            "type": "object",
            "properties": {
                "toolName": { "type": "string", "const": tool_name }
            },
            "required": ["toolName"],
            "additionalProperties": false
        }),
    };
    schemars::json_schema!({
        "type": "object",
        "properties": {
            "actionKind": { "type": "string", "const": "toolCall" },
            "operation": operation_schema
        },
        "required": ["actionKind", "operation"],
        "additionalProperties": false
    })
}

fn exact_action_array_schema(actions: &[NextAction]) -> Schema {
    if actions.is_empty() {
        return schemars::json_schema!({
            "type": "array",
            "items": { "type": "string", "pattern": "a^" },
            "minItems": 0,
            "maxItems": 0,
        });
    }
    let prefix_items = actions
        .iter()
        .map(|action| match action {
            NextAction::ToolCall(tool_call) => tool_call_schema(&tool_call.operation),
            NextAction::ExternalInstruction(external) => {
                let instruction_kind = external.instruction_kind.as_str();
                schemars::json_schema!({
                    "type": "object",
                    "properties": {
                        "actionKind": { "type": "string", "const": "externalInstruction" },
                        "instructionKind": { "type": "string", "const": instruction_kind }
                    },
                    "required": ["actionKind", "instructionKind"],
                    "additionalProperties": false
                })
            }
        })
        .collect::<Vec<_>>();
    let length = actions.len();
    schemars::json_schema!({
        "type": "array",
        "prefixItems": prefix_items,
        "items": false,
        "minItems": length,
        "maxItems": length,
    })
}

macro_rules! exact_tool_actions {
    ($name:ident, [$($selector:expr),* $(,)?]) => {
        #[derive(Debug, Clone, PartialEq, Eq, Serialize)]
        #[serde(transparent)]
        pub(crate) struct $name(Vec<NextAction>);

        impl $name {
            pub(crate) fn canonical() -> Self {
                let actions = vec![$(NextAction::tool_call($selector)),*];
                debug_assert!(CanonicalNextActions::new(actions.clone()).is_ok());
                Self(actions)
            }

            pub(crate) fn as_slice(&self) -> &[NextAction] {
                &self.0
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                let actions = Vec::<NextAction>::deserialize(deserializer)?;
                let canonical = Self::canonical();
                (actions == canonical.0)
                    .then_some(canonical)
                    .ok_or_else(|| D::Error::custom(concat!(stringify!($name), " is not exact")))
            }
        }

        impl JsonSchema for $name {
            fn schema_name() -> Cow<'static, str> {
                stringify!($name).into()
            }

            fn json_schema(_: &mut SchemaGenerator) -> Schema {
                exact_action_array_schema(Self::canonical().as_slice())
            }
        }
    };
}

exact_tool_actions!(NoNextActions, []);
exact_tool_actions!(
    StatusOnlyActions,
    [TaskOperationSelector::BranchedStatus(
        BranchedStatusSelector::new()
    ),]
);
exact_tool_actions!(
    AdaptationRefreshActions,
    [
        TaskOperationSelector::BranchedStatus(BranchedStatusSelector::new()),
        TaskOperationSelector::MergeVerify(MergeVerifySelector::new(
            MergeVerifySelectorVariant::SynchronizedTask,
        )),
    ]
);
exact_tool_actions!(
    CommitSafeExitActions,
    [
        TaskOperationSelector::BranchedStatus(BranchedStatusSelector::new()),
        TaskOperationSelector::BranchedArchive(BranchedArchiveSelector::new(
            BranchedArchiveSelectorVariant::AbandonedPreview,
        )),
    ]
);
exact_tool_actions!(
    ConflictReviewActions,
    [
        TaskOperationSelector::BranchedStatus(BranchedStatusSelector::new()),
        TaskOperationSelector::MergeConflicts(MergeConflictsSelector::new()),
    ]
);
exact_tool_actions!(
    StartAndStatusActions,
    [
        TaskOperationSelector::BranchedStart(BranchedStartSelector::new()),
        TaskOperationSelector::BranchedStatus(BranchedStatusSelector::new()),
    ]
);
exact_tool_actions!(
    RecoveryApplyActions,
    [
        TaskOperationSelector::BranchedStatus(BranchedStatusSelector::new()),
        TaskOperationSelector::RepositoryRecover(RepositoryRecoverSelector::new(
            RepositoryRecoverSelectorVariant::RecoverApply,
        )),
    ]
);
exact_tool_actions!(
    RecoveryApplyOrCancelActions,
    [
        TaskOperationSelector::BranchedStatus(BranchedStatusSelector::new()),
        TaskOperationSelector::RepositoryRecover(RepositoryRecoverSelector::new(
            RepositoryRecoverSelectorVariant::RecoverApply,
        )),
        TaskOperationSelector::RepositoryRecover(RepositoryRecoverSelector::new(
            RepositoryRecoverSelectorVariant::RecoverCancel,
        )),
    ]
);
exact_tool_actions!(
    IntegrationUnlockExitActions,
    [
        TaskOperationSelector::BranchedStatus(BranchedStatusSelector::new()),
        TaskOperationSelector::RepositoryUnlock(RepositoryUnlockSelector::new(
            RepositoryUnlockSelectorVariant::Rollback,
        )),
    ]
);
exact_tool_actions!(
    IntegrationRecoveryExitActions,
    [
        TaskOperationSelector::BranchedStatus(BranchedStatusSelector::new()),
        TaskOperationSelector::RepositoryRecover(RepositoryRecoverSelector::new(
            RepositoryRecoverSelectorVariant::RecoverApply,
        )),
    ]
);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RecoveryResumeKind {
    ApplyOnly,
    ApplyOrCancel,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct RecoveryResumeActions(Vec<NextAction>);

impl RecoveryResumeActions {
    pub(crate) fn canonical(kind: RecoveryResumeKind) -> Self {
        let mut actions = vec![
            NextAction::tool_call(TaskOperationSelector::BranchedStatus(
                BranchedStatusSelector::new(),
            )),
            NextAction::tool_call(TaskOperationSelector::RepositoryRecover(
                RepositoryRecoverSelector::new(RepositoryRecoverSelectorVariant::RecoverApply),
            )),
        ];
        if kind == RecoveryResumeKind::ApplyOrCancel {
            actions.push(NextAction::tool_call(
                TaskOperationSelector::RepositoryRecover(RepositoryRecoverSelector::new(
                    RepositoryRecoverSelectorVariant::RecoverCancel,
                )),
            ));
        }
        Self(actions)
    }

    pub(crate) fn kind(&self) -> RecoveryResumeKind {
        if self.0.len() == 3 {
            RecoveryResumeKind::ApplyOrCancel
        } else {
            RecoveryResumeKind::ApplyOnly
        }
    }
}

impl<'de> Deserialize<'de> for RecoveryResumeActions {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let actions = Vec::<NextAction>::deserialize(deserializer)?;
        [
            RecoveryResumeKind::ApplyOnly,
            RecoveryResumeKind::ApplyOrCancel,
        ]
        .into_iter()
        .map(Self::canonical)
        .find(|candidate| candidate.0 == actions)
        .ok_or_else(|| D::Error::custom("recoveryResume actions are not exact"))
    }
}

impl JsonSchema for RecoveryResumeActions {
    fn schema_name() -> Cow<'static, str> {
        "RecoveryResumeActions".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            exact_action_array_schema(&Self::canonical(RecoveryResumeKind::ApplyOnly).0),
            exact_action_array_schema(&Self::canonical(RecoveryResumeKind::ApplyOrCancel).0),
        ])
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IntegrationSetExitKind {
    Unlock,
    Recovery,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct IntegrationSetExitActions(Vec<NextAction>);

impl IntegrationSetExitActions {
    pub(crate) fn canonical(kind: IntegrationSetExitKind) -> Self {
        let exit = match kind {
            IntegrationSetExitKind::Unlock => TaskOperationSelector::RepositoryUnlock(
                RepositoryUnlockSelector::new(RepositoryUnlockSelectorVariant::Rollback),
            ),
            IntegrationSetExitKind::Recovery => TaskOperationSelector::RepositoryRecover(
                RepositoryRecoverSelector::new(RepositoryRecoverSelectorVariant::RecoverApply),
            ),
        };
        Self(vec![
            NextAction::tool_call(TaskOperationSelector::BranchedStatus(
                BranchedStatusSelector::new(),
            )),
            NextAction::tool_call(exit),
        ])
    }

    pub(crate) fn kind(&self) -> IntegrationSetExitKind {
        match self.0[1].operation().expect("exit is a tool action") {
            TaskOperationSelector::RepositoryUnlock(_) => IntegrationSetExitKind::Unlock,
            TaskOperationSelector::RepositoryRecover(_) => IntegrationSetExitKind::Recovery,
            _ => unreachable!("validated integration-set exit has one exact exit selector"),
        }
    }
}

impl<'de> Deserialize<'de> for IntegrationSetExitActions {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let actions = Vec::<NextAction>::deserialize(deserializer)?;
        [
            IntegrationSetExitKind::Unlock,
            IntegrationSetExitKind::Recovery,
        ]
        .into_iter()
        .map(Self::canonical)
        .find(|candidate| candidate.0 == actions)
        .ok_or_else(|| D::Error::custom("integrationSetExit actions are not exact"))
    }
}

impl JsonSchema for IntegrationSetExitActions {
    fn schema_name() -> Cow<'static, str> {
        "IntegrationSetExitActions".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            exact_action_array_schema(&Self::canonical(IntegrationSetExitKind::Unlock).0),
            exact_action_array_schema(&Self::canonical(IntegrationSetExitKind::Recovery).0),
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::branched_development::contracts::schema::audit_json_schema;
    use schemars::schema_for;
    use serde::de::DeserializeOwned;
    use serde_json::{json, Value};

    fn accepts<T: DeserializeOwned>(value: Value) {
        serde_json::from_value::<T>(value.clone())
            .unwrap_or_else(|error| panic!("contract rejected {value}: {error}"));
    }

    fn rejects<T: DeserializeOwned>(value: Value) {
        assert!(
            serde_json::from_value::<T>(value.clone()).is_err(),
            "contract accepted {value}"
        );
    }

    fn schema_accepts<T: JsonSchema>(value: &Value) -> bool {
        let schema = serde_json::to_value(schema_for!(T)).unwrap();
        jsonschema::options()
            .with_draft(jsonschema::Draft::Draft202012)
            .build(&schema)
            .unwrap()
            .is_valid(value)
    }

    fn assert_exact_tuple<T>(valid: Value, substitute: Value)
    where
        T: DeserializeOwned + JsonSchema,
    {
        accepts::<T>(valid.clone());
        assert!(schema_accepts::<T>(&valid), "schema rejected {valid}");

        let items = valid.as_array().unwrap();
        let mut invalid = Vec::new();
        if !items.is_empty() {
            let mut missing = items.clone();
            missing.pop();
            invalid.push(Value::Array(missing));

            let mut cross_branch = items.clone();
            *cross_branch.last_mut().unwrap() = substitute.clone();
            invalid.push(Value::Array(cross_branch));
        }
        if items.len() > 1 {
            let mut reordered = items.clone();
            reordered.reverse();
            invalid.push(Value::Array(reordered));
        }
        let mut extra = items.clone();
        extra.push(substitute);
        invalid.push(Value::Array(extra));

        for candidate in invalid {
            rejects::<T>(candidate.clone());
            assert!(
                !schema_accepts::<T>(&candidate),
                "schema accepted non-exact tuple {candidate}"
            );
        }
    }

    fn tool(tool_name: &str, request_variant: Option<&str>) -> Value {
        let mut operation = serde_json::Map::new();
        operation.insert("toolName".to_owned(), json!(tool_name));
        if let Some(request_variant) = request_variant {
            operation.insert("requestVariant".to_owned(), json!(request_variant));
        }
        json!({"actionKind": "toolCall", "operation": operation})
    }

    #[test]
    fn next_action_has_exact_53_plus_8_closed_vocabulary_and_canonical_order() {
        let selector_schema = serde_json::to_value(schema_for!(TaskOperationSelector)).unwrap();
        assert_eq!(selector_schema["oneOf"].as_array().unwrap().len(), 21);
        assert_eq!(canonical_selector_count(), 53);
        assert_eq!(ExternalInstructionKind::ALL.len(), 8);
        for kind in ExternalInstructionKind::ALL {
            let value = json!({
                "actionKind": "externalInstruction",
                "instructionKind": kind.as_str(),
            });
            accepts::<NextAction>(value.clone());
            assert!(schema_accepts::<NextAction>(&value));
        }
        rejects::<NextAction>(json!({
            "actionKind": "externalInstruction",
            "instructionKind": "runCommand"
        }));

        let status = tool("unica.branched.status", None);
        let start = tool("unica.branched.start", None);
        accepts::<CanonicalNextActions>(json!([start.clone(), status.clone()]));
        rejects::<CanonicalNextActions>(json!([status.clone(), start.clone()]));
        rejects::<CanonicalNextActions>(json!([status.clone(), status]));
    }

    #[test]
    fn exact_action_grammars_reject_substitution_reordering_and_injection() {
        let status = tool("unica.branched.status", None);
        let refresh = tool("unica.merge.verify", Some("synchronizedTask"));
        let adapted_refresh = tool("unica.merge.verify", Some("synchronizedTaskAdapted"));
        let exact = json!([status.clone(), refresh.clone()]);
        accepts::<AdaptationRefreshActions>(exact.clone());
        assert!(schema_accepts::<AdaptationRefreshActions>(&exact));
        for invalid in [
            json!([refresh, status.clone()]),
            json!([status.clone(), adapted_refresh]),
            json!([status.clone()]),
            json!([status.clone(), tool("unica.merge.conflicts", None)]),
        ] {
            rejects::<AdaptationRefreshActions>(invalid.clone());
            assert!(!schema_accepts::<AdaptationRefreshActions>(&invalid));
        }

        let unlock = json!([
            status.clone(),
            tool("unica.repository.unlock", Some("rollback"))
        ]);
        let recovery = json!([
            status,
            tool("unica.repository.recover", Some("recoverApply"))
        ]);
        accepts::<IntegrationSetExitActions>(unlock.clone());
        accepts::<IntegrationSetExitActions>(recovery.clone());
        assert!(schema_accepts::<IntegrationSetExitActions>(&unlock));
        assert!(schema_accepts::<IntegrationSetExitActions>(&recovery));
    }

    #[test]
    fn every_named_fixed_action_tuple_rejects_missing_extra_reordered_and_substituted_items() {
        let status = tool("unica.branched.status", None);
        let start = tool("unica.branched.start", None);
        let refresh = tool("unica.merge.verify", Some("synchronizedTask"));
        let archive = tool("unica.branched.archive", Some("abandonedPreview"));
        let conflicts = tool("unica.merge.conflicts", None);
        let apply = tool("unica.repository.recover", Some("recoverApply"));
        let cancel = tool("unica.repository.recover", Some("recoverCancel"));
        let unlock = tool("unica.repository.unlock", Some("rollback"));

        assert_exact_tuple::<NoNextActions>(json!([]), status.clone());
        assert_exact_tuple::<StatusOnlyActions>(json!([status.clone()]), start.clone());
        assert_exact_tuple::<AdaptationRefreshActions>(
            json!([status.clone(), refresh]),
            conflicts.clone(),
        );
        assert_exact_tuple::<CommitSafeExitActions>(
            json!([status.clone(), archive]),
            conflicts.clone(),
        );
        assert_exact_tuple::<ConflictReviewActions>(
            json!([status.clone(), conflicts.clone()]),
            start.clone(),
        );
        assert_exact_tuple::<StartAndStatusActions>(
            json!([start, status.clone()]),
            conflicts.clone(),
        );
        assert_exact_tuple::<RecoveryApplyActions>(
            json!([status.clone(), apply.clone()]),
            unlock.clone(),
        );
        assert_exact_tuple::<RecoveryApplyOrCancelActions>(
            json!([status.clone(), apply.clone(), cancel]),
            unlock.clone(),
        );
        assert_exact_tuple::<IntegrationUnlockExitActions>(
            json!([status.clone(), unlock]),
            conflicts.clone(),
        );
        assert_exact_tuple::<IntegrationRecoveryExitActions>(json!([status, apply]), conflicts);
    }

    #[test]
    fn action_schemas_are_recursively_closed() {
        for schema in [
            serde_json::to_value(schema_for!(NextAction)).unwrap(),
            serde_json::to_value(schema_for!(CanonicalNextActions)).unwrap(),
            serde_json::to_value(schema_for!(NoNextActions)).unwrap(),
            serde_json::to_value(schema_for!(StatusOnlyActions)).unwrap(),
            serde_json::to_value(schema_for!(RecoveryResumeActions)).unwrap(),
            serde_json::to_value(schema_for!(AdaptationRefreshActions)).unwrap(),
            serde_json::to_value(schema_for!(CommitSafeExitActions)).unwrap(),
            serde_json::to_value(schema_for!(ConflictReviewActions)).unwrap(),
            serde_json::to_value(schema_for!(IntegrationSetExitActions)).unwrap(),
            serde_json::to_value(schema_for!(StartAndStatusActions)).unwrap(),
            serde_json::to_value(schema_for!(RecoveryApplyActions)).unwrap(),
            serde_json::to_value(schema_for!(RecoveryApplyOrCancelActions)).unwrap(),
            serde_json::to_value(schema_for!(IntegrationUnlockExitActions)).unwrap(),
            serde_json::to_value(schema_for!(IntegrationRecoveryExitActions)).unwrap(),
        ] {
            audit_json_schema(&schema).unwrap_or_else(|error| panic!("{error}: {schema}"));
        }
    }
}
