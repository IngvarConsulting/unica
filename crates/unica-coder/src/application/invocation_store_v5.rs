use crate::application::invocation_store::SafeFailureReason;
use crate::application::receipt_ledger::{ReceiptKeyDigest, TerminalDigest, V5ToolIdentity};
use crate::domain::invocation::{
    DomainResult, InvocationId, NormalizedArgumentsHash, SafeIdentityHash, TaskId,
};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub(crate) const V5_INVOCATION_RECORD_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum V5SafeFailureReason {
    InvocationFailed,
    ResultTooLarge,
    Interrupted,
    ResumeUnsupported,
    PersistenceFailed,
    OutcomeUncertain,
    TaskCapacity,
    WorkspaceCapacity,
    WorkspaceRegistryFailed,
}

impl V5SafeFailureReason {
    pub(crate) const ALL: [Self; 9] = [
        Self::InvocationFailed,
        Self::ResultTooLarge,
        Self::Interrupted,
        Self::ResumeUnsupported,
        Self::PersistenceFailed,
        Self::OutcomeUncertain,
        Self::TaskCapacity,
        Self::WorkspaceCapacity,
        Self::WorkspaceRegistryFailed,
    ];

    pub(crate) const fn wire_name(self) -> &'static str {
        match self {
            Self::InvocationFailed => "invocation_failed",
            Self::ResultTooLarge => "result_too_large",
            Self::Interrupted => "interrupted",
            Self::ResumeUnsupported => "resume_unsupported",
            Self::PersistenceFailed => "persistence_failed",
            Self::OutcomeUncertain => "outcome_uncertain",
            Self::TaskCapacity => "task_capacity",
            Self::WorkspaceCapacity => "workspace_capacity",
            Self::WorkspaceRegistryFailed => "workspace_registry_failed",
        }
    }
}

impl From<SafeFailureReason> for V5SafeFailureReason {
    fn from(reason: SafeFailureReason) -> Self {
        match reason {
            SafeFailureReason::InvocationFailed => Self::InvocationFailed,
            SafeFailureReason::ResultTooLarge => Self::ResultTooLarge,
            SafeFailureReason::Interrupted => Self::Interrupted,
            SafeFailureReason::ResumeUnsupported => Self::ResumeUnsupported,
            SafeFailureReason::PersistenceFailed => Self::PersistenceFailed,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct V5StoredInvocationSchemaVersion;

impl Serialize for V5StoredInvocationSchemaVersion {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_u32(V5_INVOCATION_RECORD_SCHEMA_VERSION)
    }
}

impl<'de> Deserialize<'de> for V5StoredInvocationSchemaVersion {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        match u32::deserialize(deserializer)? {
            V5_INVOCATION_RECORD_SCHEMA_VERSION => Ok(Self),
            _ => Err(serde::de::Error::custom(
                "expected protocol-v5 invocation record schema version 1",
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(
    tag = "status",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub(crate) enum V5StoredTask {
    Queued,
    Working,
    Completed {
        terminal_epoch_ms: u64,
        terminal_digest: TerminalDigest,
        result: Box<DomainResult>,
    },
    Failed {
        terminal_epoch_ms: u64,
        terminal_digest: TerminalDigest,
        reason: V5SafeFailureReason,
    },
    Cancelled {
        terminal_epoch_ms: u64,
        terminal_digest: TerminalDigest,
    },
}

impl<'de> Deserialize<'de> for V5StoredTask {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum StrictTask {
            Queued(StrictQueued),
            Working(StrictWorking),
            Completed(StrictCompleted),
            Failed(StrictFailed),
            Cancelled(StrictCancelled),
        }

        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct StrictQueued {
            status: QueuedStatus,
        }

        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct StrictWorking {
            status: WorkingStatus,
        }

        #[derive(Deserialize)]
        #[serde(deny_unknown_fields, rename_all = "camelCase")]
        struct StrictCompleted {
            status: CompletedStatus,
            terminal_epoch_ms: u64,
            terminal_digest: TerminalDigest,
            result: Box<DomainResult>,
        }

        #[derive(Deserialize)]
        #[serde(deny_unknown_fields, rename_all = "camelCase")]
        struct StrictFailed {
            status: FailedStatus,
            terminal_epoch_ms: u64,
            terminal_digest: TerminalDigest,
            reason: V5SafeFailureReason,
        }

        #[derive(Deserialize)]
        #[serde(deny_unknown_fields, rename_all = "camelCase")]
        struct StrictCancelled {
            status: CancelledStatus,
            terminal_epoch_ms: u64,
            terminal_digest: TerminalDigest,
        }

        #[derive(Deserialize)]
        enum QueuedStatus {
            #[serde(rename = "queued")]
            Queued,
        }

        #[derive(Deserialize)]
        enum WorkingStatus {
            #[serde(rename = "working")]
            Working,
        }

        #[derive(Deserialize)]
        enum CompletedStatus {
            #[serde(rename = "completed")]
            Completed,
        }

        #[derive(Deserialize)]
        enum FailedStatus {
            #[serde(rename = "failed")]
            Failed,
        }

        #[derive(Deserialize)]
        enum CancelledStatus {
            #[serde(rename = "cancelled")]
            Cancelled,
        }

        match StrictTask::deserialize(deserializer)? {
            StrictTask::Queued(StrictQueued { status }) => {
                let QueuedStatus::Queued = status;
                Ok(Self::Queued)
            }
            StrictTask::Working(StrictWorking { status }) => {
                let WorkingStatus::Working = status;
                Ok(Self::Working)
            }
            StrictTask::Completed(StrictCompleted {
                status,
                terminal_epoch_ms,
                terminal_digest,
                result,
            }) => {
                let CompletedStatus::Completed = status;
                Ok(Self::Completed {
                    terminal_epoch_ms,
                    terminal_digest,
                    result,
                })
            }
            StrictTask::Failed(StrictFailed {
                status,
                terminal_epoch_ms,
                terminal_digest,
                reason,
            }) => {
                let FailedStatus::Failed = status;
                Ok(Self::Failed {
                    terminal_epoch_ms,
                    terminal_digest,
                    reason,
                })
            }
            StrictTask::Cancelled(StrictCancelled {
                status,
                terminal_epoch_ms,
                terminal_digest,
            }) => {
                let CancelledStatus::Cancelled = status;
                Ok(Self::Cancelled {
                    terminal_epoch_ms,
                    terminal_digest,
                })
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct V5StoredInvocationRecord {
    pub(crate) schema_version: V5StoredInvocationSchemaVersion,
    pub(crate) task_id: TaskId,
    pub(crate) invocation_id: InvocationId,
    pub(crate) receipt_key_digest: ReceiptKeyDigest,
    pub(crate) tool: V5ToolIdentity,
    pub(crate) normalized_arguments_hash: NormalizedArgumentsHash,
    pub(crate) workspace_identity_hash: SafeIdentityHash,
    pub(crate) created_at_epoch_ms: u64,
    pub(crate) updated_at_epoch_ms: u64,
    pub(crate) ttl_ms: u64,
    pub(crate) poll_interval_ms: u64,
    pub(crate) version: u64,
    pub(crate) cancel_requested: bool,
    pub(crate) task: V5StoredTask,
}

#[cfg(test)]
mod tests {
    use super::{V5SafeFailureReason, V5StoredInvocationRecord};
    use crate::application::invocation_store::SafeFailureReason;
    use serde_json::{json, Value};

    const QUEUED: &str = r#"{"schemaVersion":1,"taskId":"11111111-1111-4111-8111-111111111111","invocationId":"22222222-2222-4222-8222-222222222222","receiptKeyDigest":"0000000000000000000000000000000000000000000000000000000000000000","tool":"unica.view","normalizedArgumentsHash":"1111111111111111111111111111111111111111111111111111111111111111","workspaceIdentityHash":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","createdAtEpochMs":1,"updatedAtEpochMs":2,"ttlMs":3600000,"pollIntervalMs":100,"version":3,"cancelRequested":false,"task":{"status":"queued"}}"#;
    const TERMINAL_DIGEST: &str =
        "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    fn queued_record_value() -> Value {
        serde_json::from_str(QUEUED).unwrap()
    }

    fn record_with_task(task: Value) -> Value {
        let mut record = queued_record_value();
        record["task"] = task;
        record
    }

    fn assert_record_rejected(record: Value) {
        let encoded = serde_json::to_string(&record).unwrap();
        assert!(
            serde_json::from_str::<V5StoredInvocationRecord>(&encoded).is_err(),
            "strict v5 record decoder accepted {encoded}"
        );
    }

    fn assert_task_round_trip(task: Value) {
        let expected = record_with_task(task);
        let record = serde_json::from_value::<V5StoredInvocationRecord>(expected.clone()).unwrap();
        assert_eq!(serde_json::to_value(record).unwrap(), expected);
    }

    #[test]
    fn v5_safe_failure_reason_is_closed_and_converts_every_legacy_reason() {
        let expected = [
            (V5SafeFailureReason::InvocationFailed, "invocation_failed"),
            (V5SafeFailureReason::ResultTooLarge, "result_too_large"),
            (V5SafeFailureReason::Interrupted, "interrupted"),
            (V5SafeFailureReason::ResumeUnsupported, "resume_unsupported"),
            (V5SafeFailureReason::PersistenceFailed, "persistence_failed"),
            (V5SafeFailureReason::OutcomeUncertain, "outcome_uncertain"),
            (V5SafeFailureReason::TaskCapacity, "task_capacity"),
            (V5SafeFailureReason::WorkspaceCapacity, "workspace_capacity"),
            (
                V5SafeFailureReason::WorkspaceRegistryFailed,
                "workspace_registry_failed",
            ),
        ];
        assert_eq!(V5SafeFailureReason::ALL, expected.map(|(reason, _)| reason));
        for (reason, wire_name) in expected {
            assert_eq!(reason.wire_name(), wire_name);
            assert_eq!(
                serde_json::to_string(&reason).unwrap(),
                format!("\"{wire_name}\"")
            );
            assert_eq!(
                serde_json::from_str::<V5SafeFailureReason>(&format!("\"{wire_name}\"")).unwrap(),
                reason
            );
            assert_task_round_trip(json!({
                "status": "failed",
                "terminalEpochMs": 2,
                "terminalDigest": TERMINAL_DIGEST,
                "reason": wire_name,
            }));
        }

        for unknown in ["", "Interrupted", "taskCapacity", "unknown"] {
            assert!(
                serde_json::from_str::<V5SafeFailureReason>(&format!("\"{unknown}\"")).is_err()
            );
            assert_record_rejected(record_with_task(json!({
                "status": "failed",
                "terminalEpochMs": 2,
                "terminalDigest": TERMINAL_DIGEST,
                "reason": unknown,
            })));
        }

        for (legacy, expected) in [
            (
                SafeFailureReason::InvocationFailed,
                V5SafeFailureReason::InvocationFailed,
            ),
            (
                SafeFailureReason::ResultTooLarge,
                V5SafeFailureReason::ResultTooLarge,
            ),
            (
                SafeFailureReason::Interrupted,
                V5SafeFailureReason::Interrupted,
            ),
            (
                SafeFailureReason::ResumeUnsupported,
                V5SafeFailureReason::ResumeUnsupported,
            ),
            (
                SafeFailureReason::PersistenceFailed,
                V5SafeFailureReason::PersistenceFailed,
            ),
        ] {
            assert_eq!(V5SafeFailureReason::from(legacy), expected);
        }
    }

    #[test]
    fn all_five_task_statuses_round_trip_with_the_exact_selected_fields() {
        let record = serde_json::from_str::<V5StoredInvocationRecord>(QUEUED).unwrap();
        assert_eq!(serde_json::to_string(&record).unwrap(), QUEUED);

        for task in [
            json!({"status": "queued"}),
            json!({"status": "working"}),
            json!({
                "status": "completed",
                "terminalEpochMs": 2,
                "terminalDigest": TERMINAL_DIGEST,
                "result": {"ok": true, "summary": "done"},
            }),
            json!({
                "status": "failed",
                "terminalEpochMs": 2,
                "terminalDigest": TERMINAL_DIGEST,
                "reason": "outcome_uncertain",
            }),
            json!({
                "status": "cancelled",
                "terminalEpochMs": 2,
                "terminalDigest": TERMINAL_DIGEST,
            }),
        ] {
            assert_task_round_trip(task);
        }
    }

    #[test]
    fn record_rejects_wrong_schema_unknown_duplicate_and_every_missing_root_field() {
        let mut wrong_schema = queued_record_value();
        wrong_schema["schemaVersion"] = json!(2);
        assert_record_rejected(wrong_schema);

        let mut unknown = queued_record_value();
        unknown["unexpected"] = json!(true);
        assert_record_rejected(unknown);

        for required in [
            "schemaVersion",
            "taskId",
            "invocationId",
            "receiptKeyDigest",
            "tool",
            "normalizedArgumentsHash",
            "workspaceIdentityHash",
            "createdAtEpochMs",
            "updatedAtEpochMs",
            "ttlMs",
            "pollIntervalMs",
            "version",
            "cancelRequested",
            "task",
        ] {
            let mut missing = queued_record_value();
            missing.as_object_mut().unwrap().remove(required);
            assert_record_rejected(missing);
        }

        let mut wrong_case = queued_record_value();
        let poll_interval = wrong_case
            .as_object_mut()
            .unwrap()
            .remove("pollIntervalMs")
            .unwrap();
        wrong_case["poll_interval_ms"] = poll_interval;
        assert_record_rejected(wrong_case);

        for duplicate in [
            QUEUED.replace(
                "\"schemaVersion\":1",
                "\"schemaVersion\":1,\"schemaVersion\":1",
            ),
            QUEUED.replace(
                "\"status\":\"queued\"",
                "\"status\":\"queued\",\"status\":\"queued\"",
            ),
        ] {
            assert!(serde_json::from_str::<V5StoredInvocationRecord>(&duplicate).is_err());
        }
    }

    #[test]
    fn every_task_status_rejects_unknown_missing_wrong_case_and_cross_variant_fields() {
        let valid_tasks = [
            json!({"status": "queued"}),
            json!({"status": "working"}),
            json!({
                "status": "completed",
                "terminalEpochMs": 2,
                "terminalDigest": TERMINAL_DIGEST,
                "result": {"ok": true, "summary": "done"},
            }),
            json!({
                "status": "failed",
                "terminalEpochMs": 2,
                "terminalDigest": TERMINAL_DIGEST,
                "reason": "interrupted",
            }),
            json!({
                "status": "cancelled",
                "terminalEpochMs": 2,
                "terminalDigest": TERMINAL_DIGEST,
            }),
        ];

        for task in &valid_tasks {
            let mut unknown = task.clone();
            unknown["unexpected"] = json!(true);
            assert_record_rejected(record_with_task(unknown));

            let mut missing_status = task.clone();
            missing_status.as_object_mut().unwrap().remove("status");
            assert_record_rejected(record_with_task(missing_status));
        }

        for (task, required_fields) in [
            (
                valid_tasks[2].clone(),
                &["terminalEpochMs", "terminalDigest", "result"][..],
            ),
            (
                valid_tasks[3].clone(),
                &["terminalEpochMs", "terminalDigest", "reason"][..],
            ),
            (
                valid_tasks[4].clone(),
                &["terminalEpochMs", "terminalDigest"][..],
            ),
        ] {
            for required in required_fields {
                let mut missing = task.clone();
                missing.as_object_mut().unwrap().remove(*required);
                assert_record_rejected(record_with_task(missing));
            }
        }

        for task in &valid_tasks[2..] {
            for (camel_case, snake_case) in [
                ("terminalEpochMs", "terminal_epoch_ms"),
                ("terminalDigest", "terminal_digest"),
            ] {
                let mut wrong_case = task.clone();
                let value = wrong_case
                    .as_object_mut()
                    .unwrap()
                    .remove(camel_case)
                    .unwrap();
                wrong_case[snake_case] = value;
                assert_record_rejected(record_with_task(wrong_case));
            }
        }

        for status in ["queued", "working"] {
            for (field, value) in [
                ("terminalEpochMs", json!(2)),
                ("terminalDigest", json!(TERMINAL_DIGEST)),
                ("result", json!({"ok": true, "summary": "done"})),
                ("reason", json!("interrupted")),
            ] {
                let mut task = json!({"status": status});
                task[field] = value;
                assert_record_rejected(record_with_task(task));
            }
        }

        for (status_index, field, value) in [
            (2, "reason", json!("interrupted")),
            (3, "result", json!({"ok": true, "summary": "done"})),
            (4, "reason", json!("interrupted")),
            (4, "result", json!({"ok": true, "summary": "done"})),
        ] {
            let mut task = valid_tasks[status_index].clone();
            task[field] = value;
            assert_record_rejected(record_with_task(task));
        }

        for (task, wrong_case_status) in
            valid_tasks
                .iter()
                .zip(["Queued", "Working", "Completed", "Failed", "Cancelled"])
        {
            let mut wrong_case = task.clone();
            wrong_case["status"] = json!(wrong_case_status);
            assert_record_rejected(record_with_task(wrong_case));
        }

        for invalid_status in ["", "canceled", "unknown"] {
            let mut invalid = valid_tasks[4].clone();
            invalid["status"] = json!(invalid_status);
            assert_record_rejected(record_with_task(invalid));
        }
    }
}
