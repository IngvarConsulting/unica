//! The sole opaque retirement authorization and projection failure facts.
use crate::receipt_ledger::{ReceiptLedgerError, TaskRetirementPendingReceipt};
use uuid::Uuid;

#[derive(Clone)]
pub struct V5TaskRetirementAuthorization {
    pub(super) process_instance_id: Uuid,
    pub(super) generation: u64,
    pub(super) issued_sequence: u64,
    pub(super) pending_record_sha256: String,
    pub(super) authorization_fingerprint: String,
    pub(super) pending: TaskRetirementPendingReceipt,
}

pub struct V5TaskProjectionFailure {
    pub(super) error: ReceiptLedgerError,
    pub(super) fail_stop: bool,
}
#[cfg(any(
    test,
    feature = "test-support",
    feature = "receipt-ledger-test-support"
))]
impl V5TaskProjectionFailure {
    pub fn error(&self) -> &ReceiptLedgerError {
        &self.error
    }
    pub fn requires_fail_stop(&self) -> bool {
        self.fail_stop
    }
}

#[cfg(any(
    test,
    feature = "test-support",
    feature = "receipt-ledger-test-support"
))]
impl V5TaskRetirementAuthorization {
    pub fn pending(&self) -> &TaskRetirementPendingReceipt {
        &self.pending
    }
    pub fn process_instance_id(&self) -> Uuid {
        self.process_instance_id
    }
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn issued_sequence(&self) -> u64 {
        self.issued_sequence
    }
    pub fn pending_record_sha256(&self) -> &str {
        &self.pending_record_sha256
    }
    pub fn authorization_fingerprint(&self) -> &str {
        &self.authorization_fingerprint
    }
}
