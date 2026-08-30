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

    const QUEUED: &str = r#"{"schemaVersion":1,"taskId":"11111111-1111-4111-8111-111111111111","invocationId":"22222222-2222-4222-8222-222222222222","receiptKeyDigest":"0000000000000000000000000000000000000000000000000000000000000000","tool":"unica.view","normalizedArgumentsHash":"1111111111111111111111111111111111111111111111111111111111111111","workspaceIdentityHash":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","createdAtEpochMs":1,"updatedAtEpochMs":2,"ttlMs":3600000,"pollIntervalMs":100,"version":3,"cancelRequested":false,"task":{"status":"queued"}}"#;

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
    fn schema_v1_record_is_closed_and_rejects_wrong_or_cross_variant_fields() {
        let record = serde_json::from_str::<V5StoredInvocationRecord>(QUEUED).unwrap();
        assert_eq!(serde_json::to_string(&record).unwrap(), QUEUED);

        for invalid in [
            QUEUED.replace("\"schemaVersion\":1", "\"schemaVersion\":2"),
            QUEUED.replace("\"cancelRequested\":false,", ""),
            QUEUED.replace("\"status\":\"queued\"", "\"status\":\"queued\",\"reason\":\"interrupted\""),
            QUEUED.replace("\"status\":\"queued\"", "\"status\":\"completed\",\"terminalEpochMs\":2,\"terminalDigest\":\"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\"")
        ] {
            assert!(serde_json::from_str::<V5StoredInvocationRecord>(&invalid).is_err());
        }
    }

    #[test]
    fn terminal_task_fields_use_the_frozen_camel_case_field_algebra() {
        let completed = QUEUED.replace(
            r#""task":{"status":"queued"}"#,
            r#""task":{"status":"completed","terminalEpochMs":2,"terminalDigest":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","result":{"ok":true,"summary":"done"}}"#,
        );

        let record = serde_json::from_str::<V5StoredInvocationRecord>(&completed).unwrap();
        assert_eq!(serde_json::to_string(&record).unwrap(), completed);
    }
}
