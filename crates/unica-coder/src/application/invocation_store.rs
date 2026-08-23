//! Application port for durable Invocation and Task lifecycle state.

use crate::domain::invocation::{
    DomainResult, InvocationId, InvocationStatus, NormalizedArgumentsHash, ResumeDescriptor,
    SafeIdentityHash, TaskId,
};
use serde::{Deserialize, Serialize};
use std::fmt;

pub(crate) const INVOCATION_RECORD_SCHEMA_VERSION: u32 = 1;

/// Restart-stable time used only for durable timestamps and retention.
///
/// This is deliberately separate from the monotonic Invocation handoff clock:
/// `std::time::Instant` is process-local and cannot cross a daemon restart.
pub(crate) trait EpochMillisClock: Send + Sync {
    fn now_epoch_millis(&self) -> u64;
}

/// Status text which cannot be populated from caller-owned runtime strings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub(crate) struct SafeStatusMessage(String);

impl SafeStatusMessage {
    pub(crate) fn from_static(message: &'static str) -> Self {
        Self(message.to_string())
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone)]
pub(crate) struct NewInvocationRecord {
    invocation_id: InvocationId,
    tool: &'static str,
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
        tool: &'static str,
        normalized_arguments_hash: NormalizedArgumentsHash,
        workspace_identity_hash: SafeIdentityHash,
        status_message: SafeStatusMessage,
        poll_interval_ms: u64,
        ttl_ms: u64,
        resume: Option<ResumeDescriptor>,
    ) -> Self {
        Self {
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

    pub(crate) fn into_stored(self, task_id: TaskId, now_epoch_ms: u64) -> StoredInvocationRecord {
        StoredInvocationRecord {
            schema_version: INVOCATION_RECORD_SCHEMA_VERSION,
            task_id,
            invocation_id: self.invocation_id,
            tool: self.tool.to_string(),
            normalized_arguments_hash: self.normalized_arguments_hash,
            workspace_identity_hash: self.workspace_identity_hash,
            created_at_epoch_ms: now_epoch_ms,
            updated_at_epoch_ms: now_epoch_ms,
            status: InvocationStatus::Queued,
            status_message: self.status_message,
            poll_interval_ms: self.poll_interval_ms,
            ttl_ms: self.ttl_ms,
            result: None,
            resume: self.resume,
        }
    }
}

/// The exact versioned record persisted per materialized Task.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct StoredInvocationRecord {
    pub(crate) schema_version: u32,
    pub(crate) task_id: TaskId,
    pub(crate) invocation_id: InvocationId,
    pub(crate) tool: String,
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
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum InvocationStoreError {
    NotFound,
    Expired,
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
}
