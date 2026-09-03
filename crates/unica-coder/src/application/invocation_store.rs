//! Application port for durable Invocation and Task lifecycle state.

use crate::domain::invocation::{
    DomainResult, InvocationId, InvocationStatus, NormalizedArgumentsHash, ResumeDescriptor,
    SafeIdentityHash, TaskId,
};
use crate::domain::{cancellation::CancellationToken, code_intelligence::ProviderDeadline};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::io::{self, Write};

pub(crate) const MAX_CANONICAL_RESULT_BYTES: usize = 8 * 1024 * 1024;
pub(crate) const MAX_TASK_RECORD_ENVELOPE_BYTES: usize = 64 * 1024;
pub(crate) const MAX_TASK_RECORD_BYTES: usize =
    MAX_CANONICAL_RESULT_BYTES + MAX_TASK_RECORD_ENVELOPE_BYTES;

pub(crate) const LEGACY_INVOCATION_RECORD_SCHEMA_VERSION: u32 = 1;
pub(crate) const INVOCATION_RECORD_SCHEMA_VERSION: u32 = 2;

/// Restart-stable time used only for durable timestamps and retention.
///
/// This is deliberately separate from the monotonic Invocation handoff clock:
/// `std::time::Instant` is process-local and cannot cross a daemon restart.
pub(crate) trait EpochMillisClock: Send + Sync {
    fn now_epoch_millis(&self) -> u64;
}

/// Canonical invocation identity which cannot be populated with caller text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum ToolIdentity {
    #[serde(rename = "unica.view")]
    View,
    #[serde(rename = "unica.apply")]
    Apply,
    #[serde(rename = "unica.find")]
    Find,
    #[serde(rename = "unica.search")]
    Search,
    #[serde(rename = "unica.check")]
    Check,
    #[serde(rename = "unica.diff")]
    Diff,
    #[serde(rename = "unica.run")]
    Run,
    #[serde(rename = "unica.docs")]
    Docs,
}

impl ToolIdentity {
    pub(crate) const ALL: [Self; 8] = [
        Self::View,
        Self::Apply,
        Self::Find,
        Self::Search,
        Self::Check,
        Self::Diff,
        Self::Run,
        Self::Docs,
    ];

    pub(crate) const fn catalog_name(self) -> &'static str {
        match self {
            Self::View => "view",
            Self::Apply => "apply",
            Self::Find => "find",
            Self::Search => "search",
            Self::Check => "check",
            Self::Diff => "diff",
            Self::Run => "run",
            Self::Docs => "docs",
        }
    }

    pub(crate) fn from_wire_name(name: &str) -> Option<Self> {
        match name {
            "unica.view" => Some(Self::View),
            "unica.apply" => Some(Self::Apply),
            "unica.find" => Some(Self::Find),
            "unica.search" => Some(Self::Search),
            "unica.check" => Some(Self::Check),
            "unica.diff" => Some(Self::Diff),
            "unica.run" => Some(Self::Run),
            "unica.docs" => Some(Self::Docs),
            _ => None,
        }
    }
}

/// Closed status code which cannot be populated from caller-owned runtime strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum SafeStatusMessage {
    Queued,
    Working,
    Delivering,
    Completed,
    Failed,
    Interrupted,
    Cancelled,
}

/// Closed reason persisted only for failed schema-v2 records. Runtime/store
/// diagnostics are deliberately not representable here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum SafeFailureReason {
    InvocationFailed,
    ResultTooLarge,
    Interrupted,
    ResumeUnsupported,
    PersistenceFailed,
}

#[derive(Debug, Clone)]
pub(crate) struct NewInvocationRecord {
    task_id: TaskId,
    invocation_id: InvocationId,
    tool: ToolIdentity,
    normalized_arguments_hash: NormalizedArgumentsHash,
    workspace_identity_hash: SafeIdentityHash,
    status_message: SafeStatusMessage,
    poll_interval_ms: u64,
    ttl_ms: u64,
    resume: Option<ResumeDescriptor>,
}

impl NewInvocationRecord {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        invocation_id: InvocationId,
        tool: ToolIdentity,
        normalized_arguments_hash: NormalizedArgumentsHash,
        workspace_identity_hash: SafeIdentityHash,
        status_message: SafeStatusMessage,
        poll_interval_ms: u64,
        ttl_ms: u64,
        resume: Option<ResumeDescriptor>,
    ) -> Self {
        Self {
            task_id: TaskId::new(),
            invocation_id,
            tool,
            normalized_arguments_hash,
            workspace_identity_hash,
            status_message,
            poll_interval_ms,
            ttl_ms,
            resume,
        }
    }

    pub(crate) fn task_id(&self) -> TaskId {
        self.task_id
    }

    pub(crate) fn into_stored(self, now_epoch_ms: u64) -> StoredInvocationRecord {
        StoredInvocationRecord {
            schema_version: INVOCATION_RECORD_SCHEMA_VERSION,
            task_id: self.task_id,
            invocation_id: self.invocation_id,
            tool: self.tool,
            normalized_arguments_hash: self.normalized_arguments_hash,
            workspace_identity_hash: self.workspace_identity_hash,
            created_at_epoch_ms: now_epoch_ms,
            updated_at_epoch_ms: now_epoch_ms,
            status: InvocationStatus::Queued,
            status_message: self.status_message,
            poll_interval_ms: self.poll_interval_ms,
            ttl_ms: self.ttl_ms,
            result: None,
            failure_reason: None,
            resume: self.resume,
        }
    }

    pub(crate) fn into_working_stored(self, now_epoch_ms: u64) -> StoredInvocationRecord {
        let mut stored = self.into_stored(now_epoch_ms);
        stored.status = InvocationStatus::Working;
        stored.status_message = SafeStatusMessage::Working;
        stored
    }
}

/// The exact versioned record persisted per materialized Task.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct StoredInvocationRecord {
    pub(crate) schema_version: u32,
    pub(crate) task_id: TaskId,
    pub(crate) invocation_id: InvocationId,
    pub(crate) tool: ToolIdentity,
    pub(crate) normalized_arguments_hash: NormalizedArgumentsHash,
    pub(crate) workspace_identity_hash: SafeIdentityHash,
    pub(crate) created_at_epoch_ms: u64,
    pub(crate) updated_at_epoch_ms: u64,
    pub(crate) status: InvocationStatus,
    pub(crate) status_message: SafeStatusMessage,
    pub(crate) poll_interval_ms: u64,
    pub(crate) ttl_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) result: Option<DomainResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) failure_reason: Option<SafeFailureReason>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) resume: Option<ResumeDescriptor>,
}

impl StoredInvocationRecord {
    pub(crate) fn is_terminal(&self) -> bool {
        matches!(
            self.status,
            InvocationStatus::Completed | InvocationStatus::Failed | InvocationStatus::Cancelled
        )
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum TaskTransition {
    StartWorking {
        status_message: SafeStatusMessage,
    },
    Complete {
        status_message: SafeStatusMessage,
        result: Box<DomainResult>,
    },
    Fail {
        status_message: SafeStatusMessage,
        reason: SafeFailureReason,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CommitOperation {
    Create,
    Update,
    Cancel,
    Recovery,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum InvocationStoreError {
    NotFound,
    Expired,
    DeadlineExceeded,
    Cancelled,
    ActorUnavailable,
    Capacity {
        max_records: usize,
    },
    RecordTooLarge {
        max_bytes: usize,
    },
    ResultTooLarge {
        max_bytes: usize,
    },
    TaskIdCollision {
        task_id: TaskId,
    },
    AlreadyOwned,
    CommitUncertain {
        task_id: TaskId,
        operation: CommitOperation,
    },
    InvalidTransition {
        from: InvocationStatus,
        attempted: &'static str,
    },
    Corrupt(String),
    Storage(String),
}

impl fmt::Display for InvocationStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound => formatter.write_str("task record not found"),
            Self::Expired => formatter.write_str("task record expired"),
            Self::DeadlineExceeded => formatter.write_str("task store deadline exceeded"),
            Self::Cancelled => formatter.write_str("task store operation cancelled"),
            Self::ActorUnavailable => formatter.write_str("task store actor is unavailable"),
            Self::Capacity { max_records } => {
                write!(
                    formatter,
                    "task store retained record capacity reached ({max_records})"
                )
            }
            Self::RecordTooLarge { max_bytes } => {
                write!(formatter, "task record exceeds the {max_bytes}-byte limit")
            }
            Self::ResultTooLarge { max_bytes } => {
                write!(
                    formatter,
                    "canonical result exceeds the {max_bytes}-byte limit"
                )
            }
            Self::TaskIdCollision { task_id } => {
                write!(
                    formatter,
                    "preallocated task identity already exists: {task_id}"
                )
            }
            Self::AlreadyOwned => formatter.write_str("task store already has an active owner"),
            Self::CommitUncertain { task_id, operation } => write!(
                formatter,
                "task store {:?} commit durability is uncertain for {task_id}",
                operation
            ),
            Self::InvalidTransition { from, attempted } => {
                write!(
                    formatter,
                    "invalid task transition from {from:?}: {attempted}"
                )
            }
            Self::Corrupt(message) => write!(formatter, "corrupt task record: {message}"),
            Self::Storage(message) => write!(formatter, "task store failure: {message}"),
        }
    }
}

impl std::error::Error for InvocationStoreError {}

/// Application-owned port. The daemon supplies the sole-writer implementation.
pub(crate) trait InvocationStore: Send + Sync {
    fn create(
        &self,
        new_record: NewInvocationRecord,
    ) -> Result<StoredInvocationRecord, InvocationStoreError>;

    fn create_working(
        &self,
        new_record: NewInvocationRecord,
    ) -> Result<StoredInvocationRecord, InvocationStoreError>;

    fn get(&self, task_id: TaskId) -> Result<StoredInvocationRecord, InvocationStoreError>;

    fn update(
        &self,
        task_id: TaskId,
        transition: TaskTransition,
    ) -> Result<StoredInvocationRecord, InvocationStoreError>;

    fn cancel(
        &self,
        task_id: TaskId,
        status_message: SafeStatusMessage,
    ) -> Result<StoredInvocationRecord, InvocationStoreError>;

    fn create_working_before(
        &self,
        new_record: NewInvocationRecord,
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
    ) -> Result<StoredInvocationRecord, InvocationStoreError> {
        store_operation_checkpoint(deadline, cancellation)?;
        self.create_working(new_record)
    }

    fn get_before(
        &self,
        task_id: TaskId,
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
    ) -> Result<StoredInvocationRecord, InvocationStoreError> {
        store_operation_checkpoint(deadline, cancellation)?;
        self.get(task_id)
    }

    fn update_before(
        &self,
        task_id: TaskId,
        transition: TaskTransition,
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
    ) -> Result<StoredInvocationRecord, InvocationStoreError> {
        store_operation_checkpoint(deadline, cancellation)?;
        self.update(task_id, transition)
    }

    fn cancel_before(
        &self,
        task_id: TaskId,
        status_message: SafeStatusMessage,
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
    ) -> Result<StoredInvocationRecord, InvocationStoreError> {
        store_operation_checkpoint(deadline, cancellation)?;
        self.cancel(task_id, status_message)
    }
}

pub(crate) fn store_operation_checkpoint(
    deadline: ProviderDeadline,
    cancellation: &CancellationToken,
) -> Result<(), InvocationStoreError> {
    if cancellation.is_cancelled() {
        return Err(InvocationStoreError::Cancelled);
    }
    if deadline.remaining().is_zero() {
        return Err(InvocationStoreError::DeadlineExceeded);
    }
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum CanonicalResultSizeError<E> {
    TooLarge,
    Checkpoint(E),
    Serialization,
}

struct BoundedCountingWriter<'a, E, F>
where
    F: FnMut() -> Result<(), E>,
{
    bytes: usize,
    limit: usize,
    checkpoint: &'a mut F,
    failure: Option<CanonicalResultSizeError<E>>,
}

impl<E, F> Write for BoundedCountingWriter<'_, E, F>
where
    F: FnMut() -> Result<(), E>,
{
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        if let Err(error) = (self.checkpoint)() {
            self.failure = Some(CanonicalResultSizeError::Checkpoint(error));
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "canonical result checkpoint rejected",
            ));
        }
        let Some(next) = self.bytes.checked_add(buffer.len()) else {
            self.failure = Some(CanonicalResultSizeError::TooLarge);
            return Err(io::Error::new(
                io::ErrorKind::FileTooLarge,
                "canonical result exceeds byte limit",
            ));
        };
        if next > self.limit {
            self.failure = Some(CanonicalResultSizeError::TooLarge);
            return Err(io::Error::new(
                io::ErrorKind::FileTooLarge,
                "canonical result exceeds byte limit",
            ));
        }
        self.bytes = next;
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(crate) fn canonical_result_size_with_checkpoint<E, F>(
    result: &DomainResult,
    mut checkpoint: F,
) -> Result<usize, CanonicalResultSizeError<E>>
where
    F: FnMut() -> Result<(), E>,
{
    let mut writer = BoundedCountingWriter {
        bytes: 0,
        limit: MAX_CANONICAL_RESULT_BYTES,
        checkpoint: &mut checkpoint,
        failure: None,
    };
    let serialized = serde_json::to_writer(&mut writer, result);
    if let Some(error) = writer.failure.take() {
        return Err(error);
    }
    if serialized.is_err() {
        return Err(CanonicalResultSizeError::Serialization);
    }
    let bytes = writer.bytes;
    drop(writer);
    checkpoint().map_err(CanonicalResultSizeError::Checkpoint)?;
    Ok(bytes)
}

pub(crate) fn canonical_result_size(
    result: &DomainResult,
) -> Result<usize, CanonicalResultSizeError<std::convert::Infallible>> {
    canonical_result_size_with_checkpoint(result, || Ok(()))
}

#[cfg(test)]
mod tests {
    use super::{SafeFailureReason, SafeStatusMessage, ToolIdentity};
    use crate::application::tool_contracts::SurfaceRelease;
    use crate::application::v13::tool_catalog::catalog_for;

    #[test]
    fn persisted_tool_identity_matches_the_eight_invocation_catalog_entries() {
        let catalog = catalog_for(SurfaceRelease::V13).expect("hidden v0.13 catalog");
        let expected = catalog
            .tools
            .iter()
            .map(|tool| format!("unica.{}", tool.name))
            .collect::<Vec<_>>();
        let actual = ToolIdentity::ALL
            .iter()
            .map(|tool| {
                serde_json::to_value(tool)
                    .expect("tool serializes")
                    .as_str()
                    .expect("tool identity is a string")
                    .to_string()
            })
            .collect::<Vec<_>>();

        assert_eq!(actual, expected);
    }

    #[test]
    fn persisted_tool_identity_rejects_arbitrary_and_secret_bearing_values() {
        for rejected in [
            r#""unica.task.get""#,
            r#""unica.run?token=TASK_STORE_SECRET_SENTINEL""#,
            r#""https://user:password@example.invalid""#,
        ] {
            assert!(serde_json::from_str::<ToolIdentity>(rejected).is_err());
        }
    }

    #[test]
    fn safe_status_message_rejects_arbitrary_and_secret_bearing_values() {
        for rejected in [
            r#""cancelled again""#,
            r#""https://user:password@example.invalid/private""#,
            r#""TASK_STORE_SECRET_SENTINEL""#,
        ] {
            assert!(serde_json::from_str::<SafeStatusMessage>(rejected).is_err());
        }
    }

    #[test]
    fn safe_failure_reason_rejects_runtime_prose_paths_and_secrets() {
        for rejected in [
            r#""process exited with /private/tmp/secret""#,
            r#""TASK_STORE_SECRET_SENTINEL""#,
            r#""resumeOwner-vendor-extension""#,
        ] {
            assert!(serde_json::from_str::<SafeFailureReason>(rejected).is_err());
        }
        assert_eq!(
            serde_json::to_string(&SafeFailureReason::PersistenceFailed).unwrap(),
            r#""persistenceFailed""#
        );
    }
}
