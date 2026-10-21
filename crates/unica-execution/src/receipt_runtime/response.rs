//! The sole lifecycle response facts and their existing serde contracts.
use crate::invocation_store_v5::V5SafeFailureReason;
use crate::receipt_ledger::{
    AcknowledgedTombstoneReceipt, ReceiptKey, ReceiptKeyDigest, ReceiptTerminalOutcome,
    TerminalDigest,
};
use serde::{Deserialize, Serialize};
use unica_execution_contract::invocation::{DomainResult, InvocationId, InvocationStatus, TaskId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum V5InvocationPhase {
    CancelReserved,
    ReservedUnbound,
    ReservedActorBound,
    ReservedBegun,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct V5PendingDirectReceipt {
    receipt_key: ReceiptKey,
    terminal: ReceiptTerminalOutcome,
    terminal_digest: TerminalDigest,
    terminal_epoch_ms: u64,
}

impl V5PendingDirectReceipt {
    pub fn new(
        receipt_key: ReceiptKey,
        terminal: ReceiptTerminalOutcome,
        terminal_digest: TerminalDigest,
        terminal_epoch_ms: u64,
    ) -> Self {
        Self {
            receipt_key,
            terminal,
            terminal_digest,
            terminal_epoch_ms,
        }
    }

    pub fn receipt_key(&self) -> &ReceiptKey {
        &self.receipt_key
    }

    pub fn terminal(&self) -> &ReceiptTerminalOutcome {
        &self.terminal
    }

    pub fn terminal_digest(&self) -> &TerminalDigest {
        &self.terminal_digest
    }

    pub const fn terminal_epoch_ms(&self) -> u64 {
        self.terminal_epoch_ms
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct V5AcknowledgedReceipt {
    receipt_key: ReceiptKey,
    terminal_digest: TerminalDigest,
    ack_epoch_ms: u64,
    expires_epoch_ms: u64,
}

impl V5AcknowledgedReceipt {
    pub fn from_receipt(receipt: &AcknowledgedTombstoneReceipt) -> Self {
        Self {
            receipt_key: receipt.key().clone(),
            terminal_digest: receipt.terminal_digest().clone(),
            ack_epoch_ms: receipt.acknowledged_at_epoch_ms(),
            expires_epoch_ms: receipt.expires_at_epoch_ms(),
        }
    }

    pub fn receipt_key(&self) -> &ReceiptKey {
        &self.receipt_key
    }

    pub fn terminal_digest(&self) -> &TerminalDigest {
        &self.terminal_digest
    }

    pub const fn ack_epoch_ms(&self) -> u64 {
        self.ack_epoch_ms
    }

    pub const fn expires_epoch_ms(&self) -> u64 {
        self.expires_epoch_ms
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "status",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum V5DaemonTaskSnapshot {
    Queued {
        task_id: TaskId,
        invocation_id: InvocationId,
        receipt_key_digest: ReceiptKeyDigest,
        created_at_epoch_ms: u64,
        updated_at_epoch_ms: u64,
        ttl_ms: u64,
        poll_interval_ms: u64,
        version: u64,
        cancel_requested: bool,
    },
    Working {
        task_id: TaskId,
        invocation_id: InvocationId,
        receipt_key_digest: ReceiptKeyDigest,
        created_at_epoch_ms: u64,
        updated_at_epoch_ms: u64,
        ttl_ms: u64,
        poll_interval_ms: u64,
        version: u64,
        cancel_requested: bool,
    },
    Completed {
        task_id: TaskId,
        invocation_id: InvocationId,
        receipt_key_digest: ReceiptKeyDigest,
        created_at_epoch_ms: u64,
        updated_at_epoch_ms: u64,
        ttl_ms: u64,
        poll_interval_ms: u64,
        version: u64,
        cancel_requested: bool,
        terminal_epoch_ms: u64,
        terminal_digest: TerminalDigest,
        result: Box<DomainResult>,
    },
    Failed {
        task_id: TaskId,
        invocation_id: InvocationId,
        receipt_key_digest: ReceiptKeyDigest,
        created_at_epoch_ms: u64,
        updated_at_epoch_ms: u64,
        ttl_ms: u64,
        poll_interval_ms: u64,
        version: u64,
        cancel_requested: bool,
        terminal_epoch_ms: u64,
        terminal_digest: TerminalDigest,
        reason: V5SafeFailureReason,
    },
    Cancelled {
        task_id: TaskId,
        invocation_id: InvocationId,
        receipt_key_digest: ReceiptKeyDigest,
        created_at_epoch_ms: u64,
        updated_at_epoch_ms: u64,
        ttl_ms: u64,
        poll_interval_ms: u64,
        version: u64,
        cancel_requested: bool,
        terminal_epoch_ms: u64,
        terminal_digest: TerminalDigest,
    },
}

impl V5DaemonTaskSnapshot {
    pub fn receipt_key_digest(&self) -> &ReceiptKeyDigest {
        match self {
            Self::Queued {
                receipt_key_digest, ..
            }
            | Self::Working {
                receipt_key_digest, ..
            }
            | Self::Completed {
                receipt_key_digest, ..
            }
            | Self::Failed {
                receipt_key_digest, ..
            }
            | Self::Cancelled {
                receipt_key_digest, ..
            } => receipt_key_digest,
        }
    }
    pub fn task_id(&self) -> TaskId {
        match self {
            Self::Queued { task_id, .. }
            | Self::Working { task_id, .. }
            | Self::Completed { task_id, .. }
            | Self::Failed { task_id, .. }
            | Self::Cancelled { task_id, .. } => *task_id,
        }
    }

    pub fn invocation_id(&self) -> InvocationId {
        match self {
            Self::Queued { invocation_id, .. }
            | Self::Working { invocation_id, .. }
            | Self::Completed { invocation_id, .. }
            | Self::Failed { invocation_id, .. }
            | Self::Cancelled { invocation_id, .. } => *invocation_id,
        }
    }

    pub fn created_at_epoch_ms(&self) -> u64 {
        match self {
            Self::Queued {
                created_at_epoch_ms,
                ..
            }
            | Self::Working {
                created_at_epoch_ms,
                ..
            }
            | Self::Completed {
                created_at_epoch_ms,
                ..
            }
            | Self::Failed {
                created_at_epoch_ms,
                ..
            }
            | Self::Cancelled {
                created_at_epoch_ms,
                ..
            } => *created_at_epoch_ms,
        }
    }

    pub fn updated_at_epoch_ms(&self) -> u64 {
        match self {
            Self::Queued {
                updated_at_epoch_ms,
                ..
            }
            | Self::Working {
                updated_at_epoch_ms,
                ..
            }
            | Self::Completed {
                updated_at_epoch_ms,
                ..
            }
            | Self::Failed {
                updated_at_epoch_ms,
                ..
            }
            | Self::Cancelled {
                updated_at_epoch_ms,
                ..
            } => *updated_at_epoch_ms,
        }
    }

    pub fn ttl_ms(&self) -> u64 {
        match self {
            Self::Queued { ttl_ms, .. }
            | Self::Working { ttl_ms, .. }
            | Self::Completed { ttl_ms, .. }
            | Self::Failed { ttl_ms, .. }
            | Self::Cancelled { ttl_ms, .. } => *ttl_ms,
        }
    }

    pub fn version(&self) -> u64 {
        match self {
            Self::Queued { version, .. }
            | Self::Working { version, .. }
            | Self::Completed { version, .. }
            | Self::Failed { version, .. }
            | Self::Cancelled { version, .. } => *version,
        }
    }

    pub fn poll_interval_ms(&self) -> u64 {
        match self {
            Self::Queued {
                poll_interval_ms, ..
            }
            | Self::Working {
                poll_interval_ms, ..
            }
            | Self::Completed {
                poll_interval_ms, ..
            }
            | Self::Failed {
                poll_interval_ms, ..
            }
            | Self::Cancelled {
                poll_interval_ms, ..
            } => *poll_interval_ms,
        }
    }

    /// Closed lifecycle status of the durable Task behind this snapshot.
    pub fn status(&self) -> InvocationStatus {
        match self {
            Self::Queued { .. } => InvocationStatus::Queued,
            Self::Working { .. } => InvocationStatus::Working,
            Self::Completed { .. } => InvocationStatus::Completed,
            Self::Failed { .. } => InvocationStatus::Failed,
            Self::Cancelled { .. } => InvocationStatus::Cancelled,
        }
    }

    pub fn cancel_requested(&self) -> bool {
        match self {
            Self::Queued {
                cancel_requested, ..
            }
            | Self::Working {
                cancel_requested, ..
            }
            | Self::Completed {
                cancel_requested, ..
            }
            | Self::Failed {
                cancel_requested, ..
            }
            | Self::Cancelled {
                cancel_requested, ..
            } => *cancel_requested,
        }
    }

    pub fn completed_result(&self) -> Option<&DomainResult> {
        match self {
            Self::Completed { result, .. } => Some(result),
            _ => None,
        }
    }

    pub fn failure_reason(&self) -> Option<V5SafeFailureReason> {
        match self {
            Self::Failed { reason, .. } => Some(*reason),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "resultType",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum V5InvocationResponse {
    ReceiptPending {
        receipt_key: ReceiptKey,
        phase: V5InvocationPhase,
        accepted_epoch_ms: u64,
        original_budget_ms: u64,
        cancel_requested: bool,
    },
    Direct {
        receipt: V5PendingDirectReceipt,
    },
    Task {
        snapshot: V5DaemonTaskSnapshot,
    },
    Acknowledged {
        acknowledgement: V5AcknowledgedReceipt,
    },
}
