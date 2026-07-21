use super::errors::{RejectedCodeMarker, StableCodeMarker, StableErrorCode, TaskErrorData};
use super::scalars::{BoundedVec, Diagnostic, Summary};
use super::schema::one_of_schema;
use crate::domain::branched_development::{OperationId, TaskId, TaskPhase};
use schemars::{JsonSchema, Schema, SchemaGenerator};
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};
use std::borrow::Cow;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub(crate) enum NotCreatedStatus {
    #[serde(rename = "notCreated")]
    Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub(crate) enum TaskResultStatus {
    NotCreated(NotCreatedStatus),
    Existing(TaskPhase),
}

impl TaskResultStatus {
    pub(crate) const LITERALS: &[&str] = &[
        "notCreated",
        "created",
        "preflightPassed",
        "baselineReady",
        "developing",
        "localVerified",
        "synchronizationPrepared",
        "synchronizationConflicts",
        "synchronized",
        "integrationPlanned",
        "acquiringLocks",
        "locked",
        "mainMerged",
        "mainValidated",
        "committing",
        "committedAndUnlocked",
        "archivedSuccess",
        "cleanedSuccess",
        "blockedByForeignLock",
        "staleRelevantBaseline",
        "lockPlanExpansionRequired",
        "staleSupportPreflight",
        "unexpectedDelta",
        "validationFailed",
        "commitBlocked",
        "recoveryRequired",
        "committedUnverified",
        "abandonmentReady",
        "archivedAbandoned",
        "cleanedAbandoned",
    ];
}

impl JsonSchema for TaskResultStatus {
    fn schema_name() -> Cow<'static, str> {
        "TaskResultStatus".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<NotCreatedStatus>(),
            generator.subschema_for::<TaskPhase>(),
        ])
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TaskErrorEntry<Code: StableCodeMarker> {
    code: Code,
    diagnostic: Diagnostic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct EmptyTaskErrors;

impl EmptyTaskErrors {
    fn new() -> Self {
        Self
    }
}

impl Serialize for EmptyTaskErrors {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        Vec::<()>::new().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for EmptyTaskErrors {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let values = Vec::<serde::de::IgnoredAny>::deserialize(deserializer)?;
        values
            .is_empty()
            .then_some(Self)
            .ok_or_else(|| D::Error::custom("completed errors must be exactly empty"))
    }
}

impl JsonSchema for EmptyTaskErrors {
    fn schema_name() -> Cow<'static, str> {
        "EmptyTaskErrors".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        schemars::json_schema!({
            "type": "array",
            "items": { "type": "string", "pattern": "a^" },
            "minItems": 0,
            "maxItems": 0,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
struct SingletonTaskErrors<Code: StableCodeMarker>(Vec<TaskErrorEntry<Code>>);

impl<Code: StableCodeMarker> SingletonTaskErrors<Code> {
    fn new(diagnostic: Diagnostic) -> Self {
        Self(vec![TaskErrorEntry {
            code: Code::default(),
            diagnostic,
        }])
    }
}

impl<'de, Code> Deserialize<'de> for SingletonTaskErrors<Code>
where
    Code: StableCodeMarker + Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let values = Vec::<TaskErrorEntry<Code>>::deserialize(deserializer)?;
        (values.len() == 1)
            .then_some(Self(values))
            .ok_or_else(|| D::Error::custom("stopped/rejected errors must be exact singleton"))
    }
}

impl<Code> JsonSchema for SingletonTaskErrors<Code>
where
    Code: StableCodeMarker + JsonSchema,
{
    fn schema_name() -> Cow<'static, str> {
        format!("SingletonTaskErrorsOf{}", Code::CODE.as_str()).into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        let item = generator.subschema_for::<TaskErrorEntry<Code>>();
        schemars::json_schema!({
            "type": "array",
            "items": item,
            "minItems": 1,
            "maxItems": 1,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
struct RejectedTaskErrorData<Code: RejectedCodeMarker>(
    TaskErrorData,
    #[serde(skip)] std::marker::PhantomData<Code>,
);

impl<Code: RejectedCodeMarker> RejectedTaskErrorData<Code> {
    fn new(data: TaskErrorData) -> Result<Self, &'static str> {
        let expected = Code::CODE;
        let observed = StableErrorCode::from(data.code());
        (observed == expected)
            .then_some(Self(data, std::marker::PhantomData))
            .ok_or("rejected data code does not match its envelope marker")
    }
}

impl<'de, Code: RejectedCodeMarker> Deserialize<'de> for RejectedTaskErrorData<Code> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::new(TaskErrorData::deserialize(deserializer)?).map_err(D::Error::custom)
    }
}

impl<Code: RejectedCodeMarker> JsonSchema for RejectedTaskErrorData<Code> {
    fn schema_name() -> Cow<'static, str> {
        format!("RejectedTaskErrorDataOf{}", Code::CODE.as_str()).into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        Code::data_schema(generator)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
enum CompletedResultKind {
    #[serde(rename = "completed")]
    Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
enum StoppedResultKind {
    #[serde(rename = "stopped")]
    Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
enum RejectedResultKind {
    #[serde(rename = "rejected")]
    Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TrueLiteral;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FalseLiteral;

macro_rules! bool_literal_impl {
    ($name:ident, $value:literal) => {
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
                (bool::deserialize(deserializer)? == $value)
                    .then_some(Self)
                    .ok_or_else(|| D::Error::custom("result ok literal mismatch"))
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

bool_literal_impl!(TrueLiteral, true);
bool_literal_impl!(FalseLiteral, false);

macro_rules! completed_result {
    ($name:ident $(, operation_id: $operation_type:ty)?) => {
        #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct $name<Data, Changes, Warnings, Artifacts, Cache, Evidence> {
            ok: TrueLiteral,
            result_kind: CompletedResultKind,
            task_id: TaskId,
            status: TaskResultStatus,
            $(operation_id: $operation_type,)?
            summary: Summary,
            changes: BoundedVec<Changes, 1024>,
            warnings: BoundedVec<Warnings, 1024>,
            errors: EmptyTaskErrors,
            artifacts: BoundedVec<Artifacts, 1024>,
            cache: Cache,
            evidence: Evidence,
            data: Data,
        }

        impl<Data, Changes, Warnings, Artifacts, Cache, Evidence>
            $name<Data, Changes, Warnings, Artifacts, Cache, Evidence>
        {
            #[allow(clippy::too_many_arguments)]
            fn new(
                task_id: TaskId,
                status: TaskResultStatus,
                $(operation_id: $operation_type,)?
                summary: Summary,
                changes: BoundedVec<Changes, 1024>,
                warnings: BoundedVec<Warnings, 1024>,
                artifacts: BoundedVec<Artifacts, 1024>,
                cache: Cache,
                evidence: Evidence,
                data: Data,
            ) -> Self {
                Self {
                    ok: TrueLiteral,
                    result_kind: CompletedResultKind::Value,
                    task_id,
                    status,
                    $(operation_id: {
                        let value: $operation_type = operation_id;
                        value
                    },)?
                    summary,
                    changes,
                    warnings,
                    errors: EmptyTaskErrors::new(),
                    artifacts,
                    cache,
                    evidence,
                    data,
                }
            }
        }
    };
}

macro_rules! stopped_result {
    ($name:ident $(, operation_id: $operation_type:ty)?) => {
        #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct $name<
            Code: StableCodeMarker,
            Data,
            Changes,
            Warnings,
            Artifacts,
            Cache,
            Evidence,
        > {
            ok: FalseLiteral,
            result_kind: StoppedResultKind,
            task_id: TaskId,
            status: TaskResultStatus,
            $(operation_id: $operation_type,)?
            summary: Summary,
            changes: BoundedVec<Changes, 1024>,
            warnings: BoundedVec<Warnings, 1024>,
            stop_code: Code,
            errors: SingletonTaskErrors<Code>,
            artifacts: BoundedVec<Artifacts, 1024>,
            cache: Cache,
            evidence: Evidence,
            data: Data,
        }

        impl<
                Code: StableCodeMarker,
                Data,
                Changes,
                Warnings,
                Artifacts,
                Cache,
                Evidence,
            > $name<Code, Data, Changes, Warnings, Artifacts, Cache, Evidence>
        {
            #[allow(clippy::too_many_arguments)]
            fn new(
                task_id: TaskId,
                status: TaskResultStatus,
                $(operation_id: $operation_type,)?
                summary: Summary,
                changes: BoundedVec<Changes, 1024>,
                warnings: BoundedVec<Warnings, 1024>,
                diagnostic: Diagnostic,
                artifacts: BoundedVec<Artifacts, 1024>,
                cache: Cache,
                evidence: Evidence,
                data: Data,
            ) -> Self {
                Self {
                    ok: FalseLiteral,
                    result_kind: StoppedResultKind::Value,
                    task_id,
                    status,
                    $(operation_id: {
                        let value: $operation_type = operation_id;
                        value
                    },)?
                    summary,
                    changes,
                    warnings,
                    stop_code: Code::default(),
                    errors: SingletonTaskErrors::new(diagnostic),
                    artifacts,
                    cache,
                    evidence,
                    data,
                }
            }
        }
    };
}

macro_rules! rejected_result {
    ($name:ident $(, operation_id: $operation_type:ty)?) => {
        #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct $name<
            Code: RejectedCodeMarker,
            Changes,
            Warnings,
            Artifacts,
            Cache,
            Evidence,
        > {
            ok: FalseLiteral,
            result_kind: RejectedResultKind,
            task_id: TaskId,
            status: TaskResultStatus,
            $(operation_id: $operation_type,)?
            summary: Summary,
            changes: BoundedVec<Changes, 1024>,
            warnings: BoundedVec<Warnings, 1024>,
            errors: SingletonTaskErrors<Code>,
            artifacts: BoundedVec<Artifacts, 1024>,
            cache: Cache,
            evidence: Evidence,
            data: RejectedTaskErrorData<Code>,
        }

        impl<
                Code: RejectedCodeMarker,
                Changes,
                Warnings,
                Artifacts,
                Cache,
                Evidence,
            > $name<Code, Changes, Warnings, Artifacts, Cache, Evidence>
        {
            #[allow(clippy::too_many_arguments)]
            fn new(
                task_id: TaskId,
                status: TaskResultStatus,
                $(operation_id: $operation_type,)?
                summary: Summary,
                changes: BoundedVec<Changes, 1024>,
                warnings: BoundedVec<Warnings, 1024>,
                diagnostic: Diagnostic,
                artifacts: BoundedVec<Artifacts, 1024>,
                cache: Cache,
                evidence: Evidence,
                data: RejectedTaskErrorData<Code>,
            ) -> Self {
                Self {
                    ok: FalseLiteral,
                    result_kind: RejectedResultKind::Value,
                    task_id,
                    status,
                    $(operation_id: {
                        let value: $operation_type = operation_id;
                        value
                    },)?
                    summary,
                    changes,
                    warnings,
                    errors: SingletonTaskErrors::new(diagnostic),
                    artifacts,
                    cache,
                    evidence,
                    data,
                }
            }
        }
    };
}

completed_result!(ReadOnlyCompletedTaskResult);
stopped_result!(ReadOnlyStoppedTaskResult);
rejected_result!(ReadOnlyRejectedTaskResult);
completed_result!(MutatingCompletedTaskResult, operation_id: OperationId);
stopped_result!(MutatingStoppedTaskResult, operation_id: OperationId);
rejected_result!(MutatingRejectedTaskResult, operation_id: OperationId);

mod branch_sealed {
    pub trait Sealed {}
}

trait ReadOnlyCompletedBranch: branch_sealed::Sealed {}
trait ReadOnlyStoppedBranch: branch_sealed::Sealed {}
trait ReadOnlyRejectedBranch: branch_sealed::Sealed {}
trait MutatingCompletedBranch: branch_sealed::Sealed {}
trait MutatingStoppedBranch: branch_sealed::Sealed {}
trait MutatingRejectedBranch: branch_sealed::Sealed {}

macro_rules! seal_branch {
    ($role:ident for $name:ident < $($generic:ident),+ >) => {
        impl<$($generic),+> branch_sealed::Sealed for $name<$($generic),+> {}
        impl<$($generic),+> $role for $name<$($generic),+> {}
    };
    ($role:ident for $name:ident < Code, $($generic:ident),+ > where marker) => {
        impl<Code: StableCodeMarker, $($generic),+> branch_sealed::Sealed
            for $name<Code, $($generic),+>
        {
        }
        impl<Code: StableCodeMarker, $($generic),+> $role
            for $name<Code, $($generic),+>
        {
        }
    };
    ($role:ident for $name:ident < Code, $($generic:ident),+ > where rejected_marker) => {
        impl<Code: RejectedCodeMarker, $($generic),+> branch_sealed::Sealed
            for $name<Code, $($generic),+>
        {
        }
        impl<Code: RejectedCodeMarker, $($generic),+> $role
            for $name<Code, $($generic),+>
        {
        }
    };
}

seal_branch!(ReadOnlyCompletedBranch for ReadOnlyCompletedTaskResult<Data, Changes, Warnings, Artifacts, Cache, Evidence>);
seal_branch!(ReadOnlyStoppedBranch for ReadOnlyStoppedTaskResult<Code, Data, Changes, Warnings, Artifacts, Cache, Evidence> where marker);
seal_branch!(ReadOnlyRejectedBranch for ReadOnlyRejectedTaskResult<Code, Changes, Warnings, Artifacts, Cache, Evidence> where rejected_marker);
seal_branch!(MutatingCompletedBranch for MutatingCompletedTaskResult<Data, Changes, Warnings, Artifacts, Cache, Evidence>);
seal_branch!(MutatingStoppedBranch for MutatingStoppedTaskResult<Code, Data, Changes, Warnings, Artifacts, Cache, Evidence> where marker);
seal_branch!(MutatingRejectedBranch for MutatingRejectedTaskResult<Code, Changes, Warnings, Artifacts, Cache, Evidence> where rejected_marker);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
enum ReadOnlyTaskResult<
    Completed: ReadOnlyCompletedBranch,
    Stopped: ReadOnlyStoppedBranch,
    Rejected: ReadOnlyRejectedBranch,
> {
    Completed(Completed),
    Stopped(Stopped),
    Rejected(Rejected),
}

impl<Completed, Stopped, Rejected> JsonSchema for ReadOnlyTaskResult<Completed, Stopped, Rejected>
where
    Completed: ReadOnlyCompletedBranch + JsonSchema,
    Stopped: ReadOnlyStoppedBranch + JsonSchema,
    Rejected: ReadOnlyRejectedBranch + JsonSchema,
{
    fn schema_name() -> Cow<'static, str> {
        format!(
            "ReadOnlyTaskResultOf{}Or{}Or{}",
            Completed::schema_name(),
            Stopped::schema_name(),
            Rejected::schema_name()
        )
        .into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<Completed>(),
            generator.subschema_for::<Stopped>(),
            generator.subschema_for::<Rejected>(),
        ])
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
enum MutatingTaskResult<
    Completed: MutatingCompletedBranch,
    Stopped: MutatingStoppedBranch,
    Rejected: MutatingRejectedBranch,
> {
    Completed(Completed),
    Stopped(Stopped),
    Rejected(Rejected),
}

impl<Completed, Stopped, Rejected> JsonSchema for MutatingTaskResult<Completed, Stopped, Rejected>
where
    Completed: MutatingCompletedBranch + JsonSchema,
    Stopped: MutatingStoppedBranch + JsonSchema,
    Rejected: MutatingRejectedBranch + JsonSchema,
{
    fn schema_name() -> Cow<'static, str> {
        format!(
            "MutatingTaskResultOf{}Or{}Or{}",
            Completed::schema_name(),
            Stopped::schema_name(),
            Rejected::schema_name()
        )
        .into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<Completed>(),
            generator.subschema_for::<Stopped>(),
            generator.subschema_for::<Rejected>(),
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::branched_development::contracts::errors::{
        CleanupNotAllowedMarker, OperationReplayMismatchMarker,
    };
    use crate::domain::branched_development::contracts::schema::audit_json_schema;
    use crate::domain::branched_development::{Sha256Digest, UnicaId};
    use schemars::schema_for;
    use serde_json::{json, Value};

    const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const OP: &str = "33333333-3333-4333-8333-333333333333";

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct FixtureItem {
        item_id: UnicaId,
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct FixtureCache {
        cache_digest: Sha256Digest,
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct FixtureEvidence {
        evidence_digest: Sha256Digest,
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct FixtureData {
        data_digest: Sha256Digest,
    }

    type ReadCompleted = ReadOnlyCompletedTaskResult<
        FixtureData,
        FixtureItem,
        FixtureItem,
        FixtureItem,
        FixtureCache,
        FixtureEvidence,
    >;
    type ReadStopped = ReadOnlyStoppedTaskResult<
        CleanupNotAllowedMarker,
        FixtureData,
        FixtureItem,
        FixtureItem,
        FixtureItem,
        FixtureCache,
        FixtureEvidence,
    >;
    type ReadRejected = ReadOnlyRejectedTaskResult<
        OperationReplayMismatchMarker,
        FixtureItem,
        FixtureItem,
        FixtureItem,
        FixtureCache,
        FixtureEvidence,
    >;
    type ReadResult = ReadOnlyTaskResult<ReadCompleted, ReadStopped, ReadRejected>;

    type MutatingCompleted = MutatingCompletedTaskResult<
        FixtureData,
        FixtureItem,
        FixtureItem,
        FixtureItem,
        FixtureCache,
        FixtureEvidence,
    >;
    type MutatingStopped = MutatingStoppedTaskResult<
        CleanupNotAllowedMarker,
        FixtureData,
        FixtureItem,
        FixtureItem,
        FixtureItem,
        FixtureCache,
        FixtureEvidence,
    >;
    type MutatingRejected = MutatingRejectedTaskResult<
        OperationReplayMismatchMarker,
        FixtureItem,
        FixtureItem,
        FixtureItem,
        FixtureCache,
        FixtureEvidence,
    >;
    type MutationResult = MutatingTaskResult<MutatingCompleted, MutatingStopped, MutatingRejected>;

    fn common() -> Value {
        json!({
            "taskId": "TASK-1",
            "status": "developing",
            "summary": "bounded summary",
            "changes": [],
            "warnings": [],
            "artifacts": [],
            "cache": {"cacheDigest": A},
            "evidence": {"evidenceDigest": B},
        })
    }

    fn completed() -> Value {
        let mut value = common();
        let object = value.as_object_mut().unwrap();
        object.insert("ok".to_owned(), json!(true));
        object.insert("resultKind".to_owned(), json!("completed"));
        object.insert("errors".to_owned(), json!([]));
        object.insert("data".to_owned(), json!({"dataDigest": A}));
        value
    }

    fn stopped() -> Value {
        let mut value = common();
        let object = value.as_object_mut().unwrap();
        object.insert("ok".to_owned(), json!(false));
        object.insert("resultKind".to_owned(), json!("stopped"));
        object.insert("stopCode".to_owned(), json!("cleanupNotAllowed"));
        object.insert(
            "errors".to_owned(),
            json!([{"code": "cleanupNotAllowed", "diagnostic": "blocked"}]),
        );
        object.insert("data".to_owned(), json!({"dataDigest": A}));
        value
    }

    fn rejected() -> Value {
        let mut value = common();
        let object = value.as_object_mut().unwrap();
        object.insert("ok".to_owned(), json!(false));
        object.insert("resultKind".to_owned(), json!("rejected"));
        object.insert(
            "errors".to_owned(),
            json!([{"code": "operationReplayMismatch", "diagnostic": "replay"}]),
        );
        object.insert(
            "data".to_owned(),
            json!({
                "code": "operationReplayMismatch",
                "context": {
                    "contextKind": "operation",
                    "operationId": OP,
                    "expectedInputDigest": A,
                    "observedInputDigest": B,
                },
                "allowedNextActions": [],
            }),
        );
        value
    }

    fn validator<T: JsonSchema>() -> jsonschema::Validator {
        jsonschema::options()
            .with_draft(jsonschema::Draft::Draft202012)
            .build(&serde_json::to_value(schema_for!(T)).unwrap())
            .unwrap()
    }

    fn empty_items() -> BoundedVec<FixtureItem, 1024> {
        BoundedVec::new(Vec::new()).unwrap()
    }

    fn fixture_cache() -> FixtureCache {
        FixtureCache {
            cache_digest: A.parse().unwrap(),
        }
    }

    fn fixture_evidence() -> FixtureEvidence {
        FixtureEvidence {
            evidence_digest: B.parse().unwrap(),
        }
    }

    fn fixture_data() -> FixtureData {
        FixtureData {
            data_digest: A.parse().unwrap(),
        }
    }

    #[test]
    fn task_result_status_is_not_created_followed_by_all_29_phases() {
        assert_eq!(TaskResultStatus::LITERALS.len(), 30);
        assert_eq!(TaskResultStatus::LITERALS[0], "notCreated");
        assert_eq!(
            &TaskResultStatus::LITERALS[1..],
            TaskPhase::ALL
                .iter()
                .map(TaskPhase::as_str)
                .collect::<Vec<_>>()
                .as_slice()
        );
        for literal in TaskResultStatus::LITERALS {
            assert!(serde_json::from_value::<TaskResultStatus>(json!(literal)).is_ok());
        }
        assert!(serde_json::from_value::<TaskResultStatus>(json!("missing")).is_err());
    }

    #[test]
    fn read_only_three_way_presence_matrix_is_exact() {
        let schema = validator::<ReadResult>();
        for value in [completed(), stopped(), rejected()] {
            assert!(serde_json::from_value::<ReadResult>(value.clone()).is_ok());
            assert!(schema.is_valid(&value), "schema rejected {value}");
        }

        let mut with_operation = completed();
        with_operation["operationId"] = json!(OP);
        assert!(serde_json::from_value::<ReadResult>(with_operation.clone()).is_err());
        assert!(!schema.is_valid(&with_operation));

        let mut completed_with_stop = completed();
        completed_with_stop["stopCode"] = json!("cleanupNotAllowed");
        assert!(serde_json::from_value::<ReadResult>(completed_with_stop).is_err());

        let mut completed_error = completed();
        completed_error["errors"] =
            json!([{"code": "repositoryBindingMismatch", "diagnostic": "bad"}]);
        assert!(serde_json::from_value::<ReadResult>(completed_error).is_err());

        let mut object_changes = completed();
        object_changes["changes"] = json!({});
        assert!(serde_json::from_value::<ReadResult>(object_changes.clone()).is_err());
        assert!(!schema.is_valid(&object_changes));

        let item = json!({"itemId": "11111111-1111-4111-8111-111111111111"});
        let mut too_many_changes = completed();
        too_many_changes["changes"] = Value::Array(std::iter::repeat_n(item, 1025).collect());
        assert!(serde_json::from_value::<ReadResult>(too_many_changes.clone()).is_err());
        assert!(!schema.is_valid(&too_many_changes));
    }

    #[test]
    fn mutating_three_way_requires_the_same_typed_operation_field() {
        let schema = validator::<MutationResult>();
        for mut value in [completed(), stopped(), rejected()] {
            assert!(serde_json::from_value::<MutationResult>(value.clone()).is_err());
            assert!(!schema.is_valid(&value));
            value["operationId"] = json!(OP);
            assert!(serde_json::from_value::<MutationResult>(value.clone()).is_ok());
            assert!(schema.is_valid(&value));
        }

        let mut invalid = completed();
        invalid["operationId"] = json!("not-an-operation-id");
        assert!(serde_json::from_value::<MutationResult>(invalid).is_err());
    }

    #[test]
    fn result_kind_ok_and_all_three_code_occurrences_cannot_diverge() {
        let schema = validator::<ReadResult>();
        let mut stopped_code = stopped();
        stopped_code["errors"][0]["code"] = json!("operationReplayMismatch");
        assert!(serde_json::from_value::<ReadResult>(stopped_code.clone()).is_err());
        assert!(!schema.is_valid(&stopped_code));

        let mut rejected_error = rejected();
        rejected_error["errors"][0]["code"] = json!("taskNotFound");
        assert!(serde_json::from_value::<ReadResult>(rejected_error.clone()).is_err());
        assert!(!schema.is_valid(&rejected_error));

        let mut rejected_data = rejected();
        rejected_data["data"]["code"] = json!("operationInProgress");
        assert!(serde_json::from_value::<ReadResult>(rejected_data.clone()).is_err());
        assert!(!schema.is_valid(&rejected_data));

        let mut wrong_ok = stopped();
        wrong_ok["ok"] = json!(true);
        assert!(serde_json::from_value::<ReadResult>(wrong_ok).is_err());
        let mut wrong_kind = completed();
        wrong_kind["resultKind"] = json!("stopped");
        assert!(serde_json::from_value::<ReadResult>(wrong_kind).is_err());
    }

    #[test]
    fn instantiated_generic_envelope_schemas_are_closed_three_way_unions() {
        for schema in [
            serde_json::to_value(schema_for!(ReadResult)).unwrap(),
            serde_json::to_value(schema_for!(MutationResult)).unwrap(),
        ] {
            assert_eq!(schema["oneOf"].as_array().unwrap().len(), 3);
            audit_json_schema(&schema).unwrap_or_else(|error| panic!("{error}: {schema}"));
            let text = serde_json::to_string(&schema).unwrap();
            for forbidden in ["command", "stdout", "stderr", "credential", "path"] {
                assert!(!text.contains(forbidden), "schema leaked {forbidden}");
            }
        }
    }

    #[test]
    fn six_physical_branches_share_one_exact_common_property_contract() {
        let schemas = [
            (
                "readCompleted",
                serde_json::to_value(schema_for!(ReadCompleted)).unwrap(),
                false,
                false,
            ),
            (
                "readStopped",
                serde_json::to_value(schema_for!(ReadStopped)).unwrap(),
                false,
                true,
            ),
            (
                "readRejected",
                serde_json::to_value(schema_for!(ReadRejected)).unwrap(),
                false,
                false,
            ),
            (
                "mutatingCompleted",
                serde_json::to_value(schema_for!(MutatingCompleted)).unwrap(),
                true,
                false,
            ),
            (
                "mutatingStopped",
                serde_json::to_value(schema_for!(MutatingStopped)).unwrap(),
                true,
                true,
            ),
            (
                "mutatingRejected",
                serde_json::to_value(schema_for!(MutatingRejected)).unwrap(),
                true,
                false,
            ),
        ];
        let base_properties = [
            "ok",
            "resultKind",
            "taskId",
            "status",
            "summary",
            "changes",
            "warnings",
            "errors",
            "artifacts",
            "cache",
            "evidence",
            "data",
        ];
        let byte_identical_common_properties = [
            "taskId",
            "status",
            "summary",
            "changes",
            "warnings",
            "artifacts",
            "cache",
            "evidence",
        ];
        let baseline_properties = schemas[0].1["properties"].as_object().unwrap();
        assert!(
            !serde_json::to_string(&schemas[0].1)
                .unwrap()
                .contains("repositoryBindingMismatch"),
            "the exact-empty errors schema must not drag an arbitrary real error code into completed"
        );

        for (name, schema, has_operation_id, has_stop_code) in &schemas {
            let properties = schema["properties"].as_object().unwrap();
            let mut expected = base_properties.to_vec();
            if *has_operation_id {
                expected.push("operationId");
            }
            if *has_stop_code {
                expected.push("stopCode");
            }
            expected.sort_unstable();
            let mut actual = properties.keys().map(String::as_str).collect::<Vec<_>>();
            actual.sort_unstable();
            assert_eq!(actual, expected, "property drift in {name}");

            let mut required = schema["required"]
                .as_array()
                .unwrap()
                .iter()
                .map(|value| value.as_str().unwrap())
                .collect::<Vec<_>>();
            required.sort_unstable();
            assert_eq!(required, expected, "required-property drift in {name}");

            for property in byte_identical_common_properties {
                assert_eq!(
                    &properties[property], &baseline_properties[property],
                    "common property {property} drifted in {name}"
                );
            }
        }

        for (read_index, mutating_index) in [(0, 3), (1, 4), (2, 5)] {
            let read = schemas[read_index].1["properties"].as_object().unwrap();
            let mutating = schemas[mutating_index].1["properties"].as_object().unwrap();
            for (property, schema) in read {
                assert_eq!(
                    schema, &mutating[property],
                    "policy families differ at {property}"
                );
            }
        }
    }

    #[test]
    fn all_six_internal_constructors_emit_the_validated_physical_wire_shape() {
        let task_id = || "TASK-1".parse::<TaskId>().unwrap();
        let status = TaskResultStatus::Existing(TaskPhase::Developing);
        let summary = || "bounded summary".parse::<Summary>().unwrap();
        let diagnostic = || "blocked".parse::<Diagnostic>().unwrap();
        let operation_id = || OP.parse::<OperationId>().unwrap();
        let rejected_data = || {
            RejectedTaskErrorData::<OperationReplayMismatchMarker>::new(
                serde_json::from_value(rejected()["data"].clone()).unwrap(),
            )
            .unwrap()
        };

        let read_completed = ReadCompleted::new(
            task_id(),
            status,
            summary(),
            empty_items(),
            empty_items(),
            empty_items(),
            fixture_cache(),
            fixture_evidence(),
            fixture_data(),
        );
        assert_eq!(serde_json::to_value(read_completed).unwrap(), completed());

        let read_stopped = ReadStopped::new(
            task_id(),
            status,
            summary(),
            empty_items(),
            empty_items(),
            diagnostic(),
            empty_items(),
            fixture_cache(),
            fixture_evidence(),
            fixture_data(),
        );
        assert_eq!(serde_json::to_value(read_stopped).unwrap(), stopped());

        let read_rejected = ReadRejected::new(
            task_id(),
            status,
            summary(),
            empty_items(),
            empty_items(),
            "replay".parse().unwrap(),
            empty_items(),
            fixture_cache(),
            fixture_evidence(),
            rejected_data(),
        );
        assert_eq!(serde_json::to_value(read_rejected).unwrap(), rejected());

        let mutating_completed = MutatingCompleted::new(
            task_id(),
            status,
            operation_id(),
            summary(),
            empty_items(),
            empty_items(),
            empty_items(),
            fixture_cache(),
            fixture_evidence(),
            fixture_data(),
        );
        let mut expected = completed();
        expected["operationId"] = json!(OP);
        assert_eq!(serde_json::to_value(mutating_completed).unwrap(), expected);

        let mutating_stopped = MutatingStopped::new(
            task_id(),
            status,
            operation_id(),
            summary(),
            empty_items(),
            empty_items(),
            diagnostic(),
            empty_items(),
            fixture_cache(),
            fixture_evidence(),
            fixture_data(),
        );
        let mut expected = stopped();
        expected["operationId"] = json!(OP);
        assert_eq!(serde_json::to_value(mutating_stopped).unwrap(), expected);

        let mutating_rejected = MutatingRejected::new(
            task_id(),
            status,
            operation_id(),
            summary(),
            empty_items(),
            empty_items(),
            "replay".parse().unwrap(),
            empty_items(),
            fixture_cache(),
            fixture_evidence(),
            rejected_data(),
        );
        let mut expected = rejected();
        expected["operationId"] = json!(OP);
        assert_eq!(serde_json::to_value(mutating_rejected).unwrap(), expected);
    }
}
