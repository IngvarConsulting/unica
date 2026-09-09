use crate::application::invocation_store::MAX_TASK_RECORD_ENVELOPE_BYTES;
#[cfg(feature = "receipt-ledger-test-support")]
use crate::application::receipt_ledger::RequestIdentity;
use crate::application::receipt_ledger::{
    receipt_key_digest, AcknowledgedTombstoneReceipt, AttemptPhase, CancelExpiryOutcome,
    CancelReservedReceipt, CancelResolution, CommittedDirectPublication,
    DirectTerminalUnackedReceipt, HandoffTerminalStage, OriginalCutoffDescriptor,
    ProvenTaskLinkCapacity, ProvisionalTaskStatus, ReceiptKey, ReceiptKeyDigest,
    ReceiptLedgerError, ReceiptLedgerPort, ReceiptRecordHeader, ReceiptState,
    ReceiptTaskProjection, ReceiptTerminalOutcome, ReceiptVersion, ReserveOutcome, ReservedPhase,
    ReservedReceipt, StagedCapacityFallbackCase, StagedTaskPublicationCase,
    StagedTerminalTransferCertificate, TaskBoundReceipt, TaskCancellationReceipt,
    TaskHandoffActorBoundReceipt, TaskLinkDigest, TaskLinkReference, TaskPromisedActorBoundReceipt,
    TaskPromisedUnboundReceipt, TaskReceiptOwnedActorBoundReceipt, TaskTerminalBoundReceipt,
    TaskTerminalReceiptBackedReceipt, TerminalDigest, V5CanonicalTerminal,
    ACKNOWLEDGED_TOMBSTONE_TTL_MS, CANCEL_RESERVATION_TTL_MS, DIRECT_TERMINAL_RETENTION_MS,
    MAX_ACKNOWLEDGED_TOMBSTONES, MAX_ACKNOWLEDGED_TOMBSTONE_BYTES,
    MAX_ACKNOWLEDGED_TOMBSTONE_POOL_BYTES, MAX_LIVE_RECEIPTS, MAX_LIVE_RECEIPT_BYTES,
    MAX_RECEIPT_ENTITLEMENT_BYTES, MAX_TASK_LIFECYCLE_LINK_RECORD_BYTES,
};
use crate::application::receipt_ledger::{
    ReceiptLedgerCatalogSnapshot, ReceiptLedgerCatalogSnapshotAuthority,
    ReceiptLedgerCatalogSnapshotParts,
};
use crate::domain::invocation::{InvocationId, SafeIdentityHash, TaskId};
use crate::infrastructure::daemon::terminal_codec_v5::{
    prepare_committed_direct_wire, prepare_direct_terminal, restore_canonical_terminal,
    validate_persisted_direct_record_bytes, DirectReceiptWriteSlot,
};
use crate::infrastructure::platform::filesystem::{
    create_owner_only_directory_child, create_owner_only_file_child, file_identity,
    open_absolute_directory_path_nofollow, open_directory_child_nofollow,
    open_directory_ownership_lock, open_regular_child_nofollow, read_directory_names_bounded,
    remove_identity_bound_regular_child, rename_identity_bound_regular_child_no_replace,
    replace_identity_bound_regular_child, sync_directory, verify_owner_only_acl, FileIdentity,
    RetainedDirectoryCapability, RetainedRegularFileCapability,
};
use fs2::FileExt;
use std::collections::{HashMap, HashSet};
use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Component, Path};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use uuid::Uuid;

const ACTIVE_DIRECTORY_NAME: &str = "active";
const GENERATION_FILE_NAME: &str = "generation";
const LEDGER_LOCK_FILE_NAME: &str = ".receipt-ledger.lock";
const MAX_GENERATION_FILE_BYTES: usize = 32;
const RECEIPT_RECORD_SCHEMA_VERSION: u32 = 1;
const MAX_CANCEL_RESERVED_RECORD_BYTES: u64 = 1_024;
const MAX_RETAINED_RECEIPT_ROWS: usize = MAX_LIVE_RECEIPTS + MAX_ACKNOWLEDGED_TOMBSTONES;
const MAX_ACTIVE_DIRECTORY_ENTRIES: usize = MAX_RETAINED_RECEIPT_ROWS * 2;
const MAX_GENERATION_STAGING_ENTRIES: usize = MAX_RETAINED_RECEIPT_ROWS;
const MAX_RECEIPT_ROOT_DIRECTORY_ENTRIES: usize = MAX_GENERATION_STAGING_ENTRIES + 3;
const DEFAULT_RECEIPT_RECOVERY_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_COMPLETED_TASK_HANDOFF_WITNESS_BYTES: u64 = 2_048;
const RECEIPT_BATCH_SCHEMA_VERSION: u32 = 1;
const MAX_RECEIPT_MUTATION_BATCH_ROWS: usize = 32;
const MAX_RECEIPT_BATCH_ENVELOPE_ROWS: usize = 256;
const MAX_RECEIPT_BATCH_BYTES: u64 = 1_048_576;

#[derive(Clone)]
struct ReceiptBatchRow {
    record: StoredActiveReceiptV1,
    encoded: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StoredReceiptBatchV1 {
    schema_version: u32,
    rows: Vec<StoredReceiptBatchRowV1>,
}

#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StoredReceiptBatchRowV1 {
    mutation_sequence: u64,
    record_version: ReceiptVersion,
    persisted_hex: String,
}

struct DecodedReceiptBatchV1 {
    rows: Vec<ReceiptBatchRow>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReceiptBatchBacking {
    name: String,
    identity: FileIdentity,
    encoded: Arc<Vec<u8>>,
}

// A row-directory sync fault armed by the runtime hooks (process-wide, the
// daemon thread differs from the observer) or by a unit test (this thread
// only, so parallel tests never consume each other's fault). Production arms
// neither; both slots are read on every row sync so the store behaves the
// same whatever the build.
static ARMED_RECEIPT_ROW_DIRECTORY_SYNC_FAULT: AtomicBool = AtomicBool::new(false);
thread_local! {
    static ARMED_RECEIPT_ROW_DIRECTORY_SYNC_FAULT_ON_THIS_THREAD: std::cell::Cell<bool> = const {
        std::cell::Cell::new(false)
    };
}

#[cfg(test)]
thread_local! {
    static TEST_AFTER_GENERATION_REPLACE: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const {
        std::cell::RefCell::new(None)
    };
    static TEST_AFTER_RECEIPT_ROW_RENAME: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const {
        std::cell::RefCell::new(None)
    };
    static TEST_AFTER_INITIAL_GENERATION_CREATE: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const {
        std::cell::RefCell::new(None)
    };
    static TEST_RECOVERY_CLEANUP_SYNCS: std::cell::Cell<usize> = const {
        std::cell::Cell::new(0)
    };
    static TEST_AFTER_RESERVE_CATALOG_LOCK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const {
        std::cell::RefCell::new(None)
    };
    static TEST_AFTER_EXPIRED_DELETION_WITNESS_REMOVE: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const {
        std::cell::RefCell::new(None)
    };
}

/// Arms one row-directory sync failure for whichever store syncs next in
/// this process. Only the runtime hooks arm it.
pub(crate) fn arm_receipt_row_directory_sync_fault() {
    ARMED_RECEIPT_ROW_DIRECTORY_SYNC_FAULT.store(true, Ordering::Release);
}

#[cfg(test)]
fn inject_receipt_row_directory_sync_failure_for_test() {
    ARMED_RECEIPT_ROW_DIRECTORY_SYNC_FAULT_ON_THIS_THREAD.with(|slot| slot.set(true));
}

#[cfg(test)]
fn set_after_generation_replace_hook_for_test(hook: impl FnOnce() + 'static) {
    TEST_AFTER_GENERATION_REPLACE.with(|slot| slot.replace(Some(Box::new(hook))));
}

#[cfg(test)]
fn run_after_generation_replace_hook_for_test() {
    if let Some(hook) = TEST_AFTER_GENERATION_REPLACE.with(|slot| slot.borrow_mut().take()) {
        hook();
    }
}

#[cfg(test)]
fn set_after_receipt_row_rename_hook_for_test(hook: impl FnOnce() + 'static) {
    TEST_AFTER_RECEIPT_ROW_RENAME.with(|slot| slot.replace(Some(Box::new(hook))));
}

#[cfg(test)]
fn run_after_receipt_row_rename_hook_for_test() {
    if let Some(hook) = TEST_AFTER_RECEIPT_ROW_RENAME.with(|slot| slot.borrow_mut().take()) {
        hook();
    }
}

#[cfg(test)]
fn set_after_initial_generation_create_hook_for_test(hook: impl FnOnce() + 'static) {
    TEST_AFTER_INITIAL_GENERATION_CREATE.with(|slot| slot.replace(Some(Box::new(hook))));
}

#[cfg(test)]
fn run_after_initial_generation_create_hook_for_test() {
    if let Some(hook) = TEST_AFTER_INITIAL_GENERATION_CREATE.with(|slot| slot.borrow_mut().take()) {
        hook();
    }
}

#[cfg(test)]
fn reset_recovery_cleanup_syncs_for_test() {
    TEST_RECOVERY_CLEANUP_SYNCS.with(|slot| slot.set(0));
}

#[cfg(test)]
fn recovery_cleanup_syncs_for_test() -> usize {
    TEST_RECOVERY_CLEANUP_SYNCS.with(std::cell::Cell::get)
}

fn sync_recovery_cleanup_directory(directory: &File) -> io::Result<()> {
    #[cfg(test)]
    TEST_RECOVERY_CLEANUP_SYNCS.with(|slot| slot.set(slot.get().saturating_add(1)));
    sync_directory(directory)
}

#[cfg(test)]
fn set_after_reserve_catalog_lock_hook_for_test(hook: impl FnOnce() + 'static) {
    TEST_AFTER_RESERVE_CATALOG_LOCK.with(|slot| slot.replace(Some(Box::new(hook))));
}

#[cfg(test)]
fn run_after_reserve_catalog_lock_hook_for_test() {
    if let Some(hook) = TEST_AFTER_RESERVE_CATALOG_LOCK.with(|slot| slot.borrow_mut().take()) {
        hook();
    }
}

#[cfg(test)]
fn set_after_expired_deletion_witness_remove_hook_for_test(hook: impl FnOnce() + 'static) {
    TEST_AFTER_EXPIRED_DELETION_WITNESS_REMOVE.with(|slot| slot.replace(Some(Box::new(hook))));
}

#[cfg(test)]
fn run_after_expired_deletion_witness_remove_hook_for_test() {
    if let Some(hook) =
        TEST_AFTER_EXPIRED_DELETION_WITNESS_REMOVE.with(|slot| slot.borrow_mut().take())
    {
        hook();
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StoredActiveReceiptV1 {
    schema_version: u32,
    mutation_sequence: u64,
    record_version: ReceiptVersion,
    key: ReceiptKey,
    key_digest: ReceiptKeyDigest,
    lifecycle: StoredActiveLifecycleV1,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredAcknowledgedTombstoneV1 {
    #[serde(rename = "k")]
    key: ReceiptKey,
    #[serde(rename = "d")]
    terminal_digest: TerminalDigest,
    #[serde(rename = "a")]
    ack_epoch_ms: u64,
    #[serde(rename = "v", default, skip_serializing_if = "Option::is_none")]
    record_version: Option<ReceiptVersion>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(
    tag = "state",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
enum StoredActiveLifecycleV1 {
    CancelReserved {
        cancel_reserved_at_epoch_ms: u64,
        expires_at_epoch_ms: u64,
        cancel_requested: bool,
    },
    ExpiredDeletion {
        observed_at_epoch_ms: u64,
        prior_record_version: ReceiptVersion,
        prior_mutation_sequence: u64,
        prior_cancel_reserved_at_epoch_ms: u64,
        prior_expires_at_epoch_ms: u64,
    },
    ExpiredTombstoneDeletion {
        observed_at_epoch_ms: u64,
        prior_acknowledged_at_epoch_ms: u64,
        prior_terminal_digest: TerminalDigest,
    },
    ExpiredDirectDeletion {
        observed_at_epoch_ms: u64,
        prior_record_version: ReceiptVersion,
        prior_mutation_sequence: u64,
        prior_terminal_epoch_ms: u64,
        prior_terminal_digest: TerminalDigest,
    },
    ExpiredTaskReceiptDeletion {
        observed_at_epoch_ms: u64,
        prior_record_version: ReceiptVersion,
        prior_mutation_sequence: u64,
        prior_terminal_epoch_ms: u64,
        prior_ttl_ms: u64,
        prior_terminal_digest: TerminalDigest,
    },
    CompletedTaskHandoffDeletion {
        prior_record_version: ReceiptVersion,
        prior_mutation_sequence: u64,
        prior_created_at_epoch_ms: u64,
        prior_task_version: u64,
        workspace_identity_hash: SafeIdentityHash,
        task_link_digest: TaskLinkDigest,
        task_bound_lifecycle_link_version: u64,
        task_bound_mutation_sequence: u64,
        task_record_version: u64,
        bind_epoch_ms: u64,
        phase: AttemptPhase,
        #[serde(default)]
        terminal_staged: bool,
    },
    ReservedUnbound {
        reserved_at_epoch_ms: u64,
        original_cutoff: OriginalCutoffDescriptor,
        cancel_requested: bool,
    },
    ReservedActorBound {
        reserved_at_epoch_ms: u64,
        original_cutoff: OriginalCutoffDescriptor,
        bound_workspace_identity: SafeIdentityHash,
        cancel_requested: bool,
    },
    ReservedBegun {
        reserved_at_epoch_ms: u64,
        original_cutoff: OriginalCutoffDescriptor,
        bound_workspace_identity: SafeIdentityHash,
        cancel_requested: bool,
    },
    TaskPromisedUnbound {
        original_cutoff: OriginalCutoffDescriptor,
        task_id: TaskId,
        invocation_id: InvocationId,
        created_at_epoch_ms: u64,
        updated_at_epoch_ms: u64,
        ttl_ms: u64,
        poll_interval_ms: u64,
        task_version: u64,
        cancel_requested: bool,
    },
    TaskPromisedActorBound {
        original_cutoff: OriginalCutoffDescriptor,
        task_id: TaskId,
        invocation_id: InvocationId,
        created_at_epoch_ms: u64,
        updated_at_epoch_ms: u64,
        ttl_ms: u64,
        poll_interval_ms: u64,
        task_version: u64,
        workspace_identity_hash: SafeIdentityHash,
        task_link_digest: TaskLinkDigest,
        cancel_requested: bool,
    },
    TaskHandoffActorBound {
        original_cutoff: OriginalCutoffDescriptor,
        task_id: TaskId,
        invocation_id: InvocationId,
        created_at_epoch_ms: u64,
        updated_at_epoch_ms: u64,
        ttl_ms: u64,
        poll_interval_ms: u64,
        task_version: u64,
        workspace_identity_hash: SafeIdentityHash,
        task_link_digest: TaskLinkDigest,
        phase: AttemptPhase,
        cancel_requested: bool,
        #[serde(default)]
        terminal_stage: StoredHandoffTerminalStageV1,
    },
    TaskReceiptOwnedActorBound {
        original_cutoff: OriginalCutoffDescriptor,
        task_id: TaskId,
        invocation_id: InvocationId,
        created_at_epoch_ms: u64,
        updated_at_epoch_ms: u64,
        ttl_ms: u64,
        poll_interval_ms: u64,
        task_version: u64,
        workspace_identity_hash: SafeIdentityHash,
        task_link_digest: TaskLinkDigest,
        proven_link_capacity: StoredProvenTaskLinkCapacityV1,
        cancel_requested: bool,
    },
    DirectTerminalUnacked {
        original_cutoff: OriginalCutoffDescriptor,
        terminal_epoch_ms: u64,
        terminal_digest: crate::application::receipt_ledger::TerminalDigest,
        terminal: Arc<ReceiptTerminalOutcome>,
    },
    TaskTerminalReceiptBacked {
        task_id: TaskId,
        invocation_id: InvocationId,
        created_at_epoch_ms: u64,
        updated_at_epoch_ms: u64,
        ttl_ms: u64,
        poll_interval_ms: u64,
        task_version: u64,
        terminal_epoch_ms: u64,
        terminal_digest: TerminalDigest,
        terminal: Arc<ReceiptTerminalOutcome>,
        cancel_requested: bool,
    },
    AcknowledgementCommit {
        terminal_digest: TerminalDigest,
        acknowledged_at_epoch_ms: u64,
        prior_record_version: ReceiptVersion,
        prior_mutation_sequence: u64,
    },
    AcknowledgedTombstone {
        terminal_digest: TerminalDigest,
        acknowledged_at_epoch_ms: u64,
    },
}

#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(
    tag = "state",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
enum StoredHandoffTerminalStageV1 {
    #[default]
    NoTerminal,
    Staged {
        terminal_epoch_ms: u64,
        terminal_digest: TerminalDigest,
        terminal: Arc<ReceiptTerminalOutcome>,
    },
}

pub(crate) fn canonical_staged_transfer_certificate(
    key: &ReceiptKey,
    key_digest: &ReceiptKeyDigest,
    link: &TaskLinkReference,
    terminal_epoch_ms: u64,
    terminal: &V5CanonicalTerminal,
) -> Result<StagedTerminalTransferCertificate, ReceiptLedgerError> {
    let task_record_max_bytes = MAX_RECEIPT_ENTITLEMENT_BYTES;
    let response_frame_max_bytes = MAX_RECEIPT_ENTITLEMENT_BYTES;
    StagedTerminalTransferCertificate::new(
        key.core_identity_digest().clone(),
        key_digest.clone(),
        key.reserved_task_id(),
        key.invocation_id(),
        link.digest().clone(),
        terminal.digest().clone(),
        terminal_epoch_ms,
        MAX_RECEIPT_ENTITLEMENT_BYTES,
        MAX_TASK_LIFECYCLE_LINK_RECORD_BYTES,
        [
            StagedTaskPublicationCase::Absent {
                final_task_record_max_bytes: task_record_max_bytes,
                task_response_frame_max_bytes: response_frame_max_bytes,
            },
            StagedTaskPublicationCase::ExactProvisional {
                status: ProvisionalTaskStatus::Queued,
                version: u64::MAX,
                cancel_requested: false,
                final_task_record_max_bytes: task_record_max_bytes,
                task_response_frame_max_bytes: response_frame_max_bytes,
            },
            StagedTaskPublicationCase::ExactProvisional {
                status: ProvisionalTaskStatus::Queued,
                version: u64::MAX,
                cancel_requested: true,
                final_task_record_max_bytes: task_record_max_bytes,
                task_response_frame_max_bytes: response_frame_max_bytes,
            },
            StagedTaskPublicationCase::ExactProvisional {
                status: ProvisionalTaskStatus::Working,
                version: u64::MAX,
                cancel_requested: false,
                final_task_record_max_bytes: task_record_max_bytes,
                task_response_frame_max_bytes: response_frame_max_bytes,
            },
            StagedTaskPublicationCase::ExactProvisional {
                status: ProvisionalTaskStatus::Working,
                version: u64::MAX,
                cancel_requested: true,
                final_task_record_max_bytes: task_record_max_bytes,
                task_response_frame_max_bytes: response_frame_max_bytes,
            },
        ],
        [StagedCapacityFallbackCase::LinkCapacity {
            receipt_backed_record_max_bytes: MAX_RECEIPT_ENTITLEMENT_BYTES,
            task_response_frame_max_bytes: MAX_RECEIPT_ENTITLEMENT_BYTES,
        }],
    )
    .map_err(|_| ReceiptLedgerError::Corrupt("invalid staged terminal transfer certificate"))
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(
    tag = "dimension",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
enum StoredProvenTaskLinkCapacityV1 {
    Count {
        observed_live_links: u64,
        maximum_live_links: u64,
    },
    Bytes {
        required_link_bytes: u64,
        available_link_bytes: u64,
    },
}

impl From<&ProvenTaskLinkCapacity> for StoredProvenTaskLinkCapacityV1 {
    fn from(value: &ProvenTaskLinkCapacity) -> Self {
        match value {
            ProvenTaskLinkCapacity::Count {
                observed_live_links,
                maximum_live_links,
            } => Self::Count {
                observed_live_links: *observed_live_links,
                maximum_live_links: *maximum_live_links,
            },
            ProvenTaskLinkCapacity::Bytes {
                required_link_bytes,
                available_link_bytes,
            } => Self::Bytes {
                required_link_bytes: *required_link_bytes,
                available_link_bytes: *available_link_bytes,
            },
        }
    }
}

impl From<&StoredProvenTaskLinkCapacityV1> for ProvenTaskLinkCapacity {
    fn from(value: &StoredProvenTaskLinkCapacityV1) -> Self {
        match value {
            StoredProvenTaskLinkCapacityV1::Count {
                observed_live_links,
                maximum_live_links,
            } => Self::Count {
                observed_live_links: *observed_live_links,
                maximum_live_links: *maximum_live_links,
            },
            StoredProvenTaskLinkCapacityV1::Bytes {
                required_link_bytes,
                available_link_bytes,
            } => Self::Bytes {
                required_link_bytes: *required_link_bytes,
                available_link_bytes: *available_link_bytes,
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
struct CatalogEntry {
    record: StoredActiveReceiptV1,
    encoded_bytes: u64,
}

impl CatalogEntry {
    fn record_header(&self) -> ReceiptRecordHeader {
        ReceiptRecordHeader::new(
            self.record.key.clone(),
            self.record.key_digest.clone(),
            self.record.record_version,
            self.record.mutation_sequence,
            self.encoded_bytes,
        )
    }

    fn reserved_result_bytes(&self) -> u64 {
        match &self.record.lifecycle {
            StoredActiveLifecycleV1::CancelReserved { .. }
            | StoredActiveLifecycleV1::ExpiredDeletion { .. }
            | StoredActiveLifecycleV1::ExpiredTombstoneDeletion { .. }
            | StoredActiveLifecycleV1::ExpiredDirectDeletion { .. }
            | StoredActiveLifecycleV1::ExpiredTaskReceiptDeletion { .. }
            | StoredActiveLifecycleV1::CompletedTaskHandoffDeletion { .. }
            | StoredActiveLifecycleV1::AcknowledgementCommit { .. }
            | StoredActiveLifecycleV1::AcknowledgedTombstone { .. } => 0,
            StoredActiveLifecycleV1::ReservedUnbound { .. }
            | StoredActiveLifecycleV1::ReservedActorBound { .. }
            | StoredActiveLifecycleV1::ReservedBegun { .. }
            | StoredActiveLifecycleV1::TaskPromisedUnbound { .. }
            | StoredActiveLifecycleV1::TaskPromisedActorBound { .. }
            | StoredActiveLifecycleV1::TaskHandoffActorBound { .. }
            | StoredActiveLifecycleV1::TaskReceiptOwnedActorBound { .. }
            | StoredActiveLifecycleV1::DirectTerminalUnacked { .. }
            | StoredActiveLifecycleV1::TaskTerminalReceiptBacked { .. } => {
                MAX_RECEIPT_ENTITLEMENT_BYTES
                    .checked_sub(self.encoded_bytes)
                    .expect("validated receipt record fits its exact byte entitlement")
            }
        }
    }

    fn is_tombstone(&self) -> bool {
        matches!(
            &self.record.lifecycle,
            StoredActiveLifecycleV1::AcknowledgedTombstone { .. }
        )
    }

    fn live_actual_bytes(&self) -> u64 {
        if self.is_tombstone() {
            0
        } else {
            self.encoded_bytes
        }
    }

    fn tombstone_bytes(&self) -> u64 {
        if self.is_tombstone() {
            self.encoded_bytes
        } else {
            0
        }
    }

    fn state(&self) -> Result<ReceiptState, ReceiptLedgerError> {
        match &self.record.lifecycle {
            StoredActiveLifecycleV1::CancelReserved {
                cancel_reserved_at_epoch_ms,
                expires_at_epoch_ms,
                cancel_requested,
            } => {
                let receipt = CancelReservedReceipt::new(
                    self.record.key.clone(),
                    self.record.record_version,
                    self.record.mutation_sequence,
                    self.encoded_bytes,
                    *cancel_reserved_at_epoch_ms,
                )
                .map_err(|_| ReceiptLedgerError::Corrupt("CancelReserved expiry exceeds u64"))?;
                if !cancel_requested || receipt.expires_at_epoch_ms() != *expires_at_epoch_ms {
                    return Err(ReceiptLedgerError::Corrupt(
                        "CancelReserved row contradicts its fixed cancellation reservation",
                    ));
                }
                Ok(ReceiptState::CancelReserved(receipt))
            }
            StoredActiveLifecycleV1::ExpiredDeletion { .. }
            | StoredActiveLifecycleV1::ExpiredTombstoneDeletion { .. }
            | StoredActiveLifecycleV1::ExpiredDirectDeletion { .. }
            | StoredActiveLifecycleV1::ExpiredTaskReceiptDeletion { .. }
            | StoredActiveLifecycleV1::CompletedTaskHandoffDeletion { .. } => Err(
                ReceiptLedgerError::Corrupt("expired deletion witness is not a live receipt state"),
            ),
            StoredActiveLifecycleV1::ReservedUnbound {
                reserved_at_epoch_ms,
                original_cutoff,
                cancel_requested,
            } => Ok(ReceiptState::Reserved(ReservedReceipt::new(
                self.record_header(),
                *reserved_at_epoch_ms,
                *original_cutoff,
                ReservedPhase::Unbound,
                *cancel_requested,
                self.reserved_result_bytes(),
            ))),
            StoredActiveLifecycleV1::ReservedActorBound {
                reserved_at_epoch_ms,
                original_cutoff,
                bound_workspace_identity,
                cancel_requested,
            } => Ok(ReceiptState::Reserved(ReservedReceipt::new(
                self.record_header(),
                *reserved_at_epoch_ms,
                *original_cutoff,
                ReservedPhase::ActorBound {
                    bound_workspace_identity: bound_workspace_identity.clone(),
                },
                *cancel_requested,
                self.reserved_result_bytes(),
            ))),
            StoredActiveLifecycleV1::ReservedBegun {
                reserved_at_epoch_ms,
                original_cutoff,
                bound_workspace_identity,
                cancel_requested,
            } => Ok(ReceiptState::Reserved(ReservedReceipt::new(
                self.record_header(),
                *reserved_at_epoch_ms,
                *original_cutoff,
                ReservedPhase::Begun {
                    bound_workspace_identity: bound_workspace_identity.clone(),
                },
                *cancel_requested,
                self.reserved_result_bytes(),
            ))),
            StoredActiveLifecycleV1::TaskPromisedUnbound {
                task_id,
                invocation_id,
                created_at_epoch_ms,
                updated_at_epoch_ms,
                ttl_ms,
                poll_interval_ms,
                task_version,
                cancel_requested,
                ..
            } => Ok(ReceiptState::TaskPromisedUnbound(
                TaskPromisedUnboundReceipt::new(
                    self.record_header(),
                    ReceiptTaskProjection::new(
                        *task_id,
                        *invocation_id,
                        *created_at_epoch_ms,
                        *updated_at_epoch_ms,
                        *ttl_ms,
                        *poll_interval_ms,
                        *task_version,
                    )?,
                    *cancel_requested,
                    self.reserved_result_bytes(),
                )?,
            )),
            StoredActiveLifecycleV1::TaskPromisedActorBound {
                task_id,
                invocation_id,
                created_at_epoch_ms,
                updated_at_epoch_ms,
                ttl_ms,
                poll_interval_ms,
                task_version,
                workspace_identity_hash,
                task_link_digest,
                cancel_requested,
                ..
            } => {
                let task = ReceiptTaskProjection::new(
                    *task_id,
                    *invocation_id,
                    *created_at_epoch_ms,
                    *updated_at_epoch_ms,
                    *ttl_ms,
                    *poll_interval_ms,
                    *task_version,
                )?;
                let link = TaskLinkReference::new(
                    self.record.key_digest.clone(),
                    *task_id,
                    *invocation_id,
                    workspace_identity_hash.clone(),
                );
                if link.digest() != task_link_digest {
                    return Err(ReceiptLedgerError::Corrupt(
                        "promised Task link digest contradicts its actor identity",
                    ));
                }
                Ok(ReceiptState::TaskPromisedActorBound(
                    TaskPromisedActorBoundReceipt::new(
                        self.record_header(),
                        task,
                        link,
                        *cancel_requested,
                        self.reserved_result_bytes(),
                    )?,
                ))
            }
            StoredActiveLifecycleV1::TaskHandoffActorBound {
                task_id,
                invocation_id,
                created_at_epoch_ms,
                updated_at_epoch_ms,
                ttl_ms,
                poll_interval_ms,
                task_version,
                workspace_identity_hash,
                task_link_digest,
                phase,
                cancel_requested,
                terminal_stage,
                ..
            } => {
                let task = ReceiptTaskProjection::new(
                    *task_id,
                    *invocation_id,
                    *created_at_epoch_ms,
                    *updated_at_epoch_ms,
                    *ttl_ms,
                    *poll_interval_ms,
                    *task_version,
                )?;
                let link = TaskLinkReference::new(
                    self.record.key_digest.clone(),
                    *task_id,
                    *invocation_id,
                    workspace_identity_hash.clone(),
                );
                if link.digest() != task_link_digest {
                    return Err(ReceiptLedgerError::Corrupt(
                        "Task handoff link digest contradicts its actor identity",
                    ));
                }
                let terminal_stage = match terminal_stage {
                    StoredHandoffTerminalStageV1::NoTerminal => HandoffTerminalStage::NoTerminal,
                    StoredHandoffTerminalStageV1::Staged {
                        terminal_epoch_ms,
                        terminal_digest,
                        terminal,
                    } => {
                        let terminal =
                            restore_canonical_terminal(Arc::clone(terminal), terminal_digest)?;
                        let certificate = canonical_staged_transfer_certificate(
                            &self.record.key,
                            &self.record.key_digest,
                            &link,
                            *terminal_epoch_ms,
                            &terminal,
                        )?;
                        HandoffTerminalStage::Staged {
                            terminal_epoch_ms: *terminal_epoch_ms,
                            terminal,
                            certificate: Box::new(certificate),
                        }
                    }
                };
                Ok(ReceiptState::TaskHandoffActorBound(
                    TaskHandoffActorBoundReceipt::new(
                        self.record_header(),
                        task,
                        link,
                        *phase,
                        *cancel_requested,
                        self.reserved_result_bytes(),
                        terminal_stage,
                    )?,
                ))
            }
            StoredActiveLifecycleV1::TaskReceiptOwnedActorBound {
                task_id,
                invocation_id,
                created_at_epoch_ms,
                updated_at_epoch_ms,
                ttl_ms,
                poll_interval_ms,
                task_version,
                workspace_identity_hash,
                task_link_digest,
                proven_link_capacity,
                cancel_requested,
                ..
            } => {
                let task = ReceiptTaskProjection::new(
                    *task_id,
                    *invocation_id,
                    *created_at_epoch_ms,
                    *updated_at_epoch_ms,
                    *ttl_ms,
                    *poll_interval_ms,
                    *task_version,
                )?;
                let link = TaskLinkReference::new(
                    self.record.key_digest.clone(),
                    *task_id,
                    *invocation_id,
                    workspace_identity_hash.clone(),
                );
                if link.digest() != task_link_digest {
                    return Err(ReceiptLedgerError::Corrupt(
                        "receipt-owned Task link digest contradicts its actor identity",
                    ));
                }
                Ok(ReceiptState::TaskReceiptOwnedActorBound(
                    TaskReceiptOwnedActorBoundReceipt::new(
                        self.record_header(),
                        task,
                        link,
                        *cancel_requested,
                        self.reserved_result_bytes(),
                        proven_link_capacity.into(),
                    )?,
                ))
            }
            StoredActiveLifecycleV1::DirectTerminalUnacked {
                original_cutoff,
                terminal_epoch_ms,
                terminal_digest,
                terminal,
                ..
            } => {
                let terminal = restore_canonical_terminal(Arc::clone(terminal), terminal_digest)?;
                Ok(ReceiptState::DirectTerminalUnacked(
                    DirectTerminalUnackedReceipt::new(
                        self.record_header(),
                        *original_cutoff,
                        *terminal_epoch_ms,
                        terminal,
                        self.reserved_result_bytes(),
                    ),
                ))
            }
            StoredActiveLifecycleV1::TaskTerminalReceiptBacked {
                task_id,
                invocation_id,
                created_at_epoch_ms,
                updated_at_epoch_ms,
                ttl_ms,
                poll_interval_ms,
                task_version,
                terminal_epoch_ms,
                terminal_digest,
                terminal,
                cancel_requested,
            } => {
                let terminal = restore_canonical_terminal(Arc::clone(terminal), terminal_digest)?;
                let task = ReceiptTaskProjection::new(
                    *task_id,
                    *invocation_id,
                    *created_at_epoch_ms,
                    *updated_at_epoch_ms,
                    *ttl_ms,
                    *poll_interval_ms,
                    *task_version,
                )?;
                Ok(ReceiptState::TaskTerminalReceiptBacked(
                    TaskTerminalReceiptBackedReceipt::new(
                        self.record_header(),
                        task,
                        *terminal_epoch_ms,
                        terminal,
                        *cancel_requested,
                        self.reserved_result_bytes(),
                    )?,
                ))
            }
            StoredActiveLifecycleV1::AcknowledgementCommit { .. } => {
                Err(ReceiptLedgerError::Corrupt(
                    "acknowledgement commit witness is not a live receipt state",
                ))
            }
            StoredActiveLifecycleV1::AcknowledgedTombstone {
                terminal_digest,
                acknowledged_at_epoch_ms,
            } => Ok(ReceiptState::AcknowledgedTombstone(
                AcknowledgedTombstoneReceipt::new(
                    self.record.key.clone(),
                    self.record.key_digest.clone(),
                    terminal_digest.clone(),
                    *acknowledged_at_epoch_ms,
                    self.encoded_bytes,
                )
                .map_err(|_| {
                    ReceiptLedgerError::Corrupt("acknowledged tombstone expiry exceeds u64")
                })?,
            )),
        }
    }

    fn reservation(&self) -> Result<ReservedReceipt, ReceiptLedgerError> {
        match self.state()? {
            ReceiptState::Reserved(reservation) => Ok(reservation),
            ReceiptState::CancelReserved(_)
            | ReceiptState::DirectTerminalUnacked(_)
            | ReceiptState::TaskTerminalReceiptBacked(_)
            | ReceiptState::AcknowledgedTombstone(_) => {
                Err(ReceiptLedgerError::ReceiptRowPresentUnsupported)
            }
            _ => unreachable!("active receipt codec does not expose this state as a reservation"),
        }
    }

    fn is_expired_deletion(&self) -> bool {
        matches!(
            &self.record.lifecycle,
            StoredActiveLifecycleV1::ExpiredDeletion { .. }
                | StoredActiveLifecycleV1::ExpiredTombstoneDeletion { .. }
                | StoredActiveLifecycleV1::ExpiredDirectDeletion { .. }
                | StoredActiveLifecycleV1::CompletedTaskHandoffDeletion { .. }
        )
    }

    fn is_acknowledgement_commit(&self) -> bool {
        matches!(
            &self.record.lifecycle,
            StoredActiveLifecycleV1::AcknowledgementCommit { .. }
        )
    }
}

fn exact_reserved_state(
    expected_key: &ReceiptKey,
    reservation: ReservedReceipt,
) -> Result<ReceiptState, ReceiptLedgerError> {
    if reservation.key() != expected_key {
        return Err(ReceiptLedgerError::ReceiptDigestCollision);
    }
    Ok(ReceiptState::Reserved(reservation))
}

#[derive(Clone, Default)]
struct ReceiptCatalog {
    records: HashMap<ReceiptKeyDigest, CatalogEntry>,
    batch_backing: HashMap<ReceiptKeyDigest, ReceiptBatchBacking>,
    invocation_index: HashMap<InvocationId, ReceiptKeyDigest>,
    reserved_task_index: HashMap<TaskId, ReceiptKeyDigest>,
    actual_bytes: u64,
    reserved_result_bytes: u64,
    tombstone_bytes: u64,
    live_records: usize,
    tombstone_records: usize,
    tombstone_compaction_backing: Option<ReceiptBatchBacking>,
    unavailable: bool,
}

impl ReceiptCatalog {
    fn live_count(&self) -> usize {
        self.live_records
    }

    fn tombstone_count(&self) -> usize {
        self.tombstone_records
    }
}

struct GenerationState {
    capability: RetainedRegularFileCapability,
    file: File,
}

type RecoveryStagingEntry = (OsString, FileIdentity, File);

struct RecoveredCatalog {
    catalog: ReceiptCatalog,
    maximum_mutation_sequence: u64,
    staging: Vec<RecoveryStagingEntry>,
    expired_deletions: Vec<RecoveryStagingEntry>,
    expired_deletion_mutation_sequence: Option<u64>,
    acknowledgement_recovery: Option<AcknowledgementRecovery>,
}

struct AcknowledgementRecovery {
    compact_record: StoredActiveReceiptV1,
    compact_encoded: Vec<u8>,
    mutation_sequence: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MissingReceiptObservation {
    receipt_key_digest: ReceiptKeyDigest,
    generation_before: u64,
    generation_after: u64,
}

impl MissingReceiptObservation {
    pub(crate) fn receipt_key_digest(&self) -> &ReceiptKeyDigest {
        &self.receipt_key_digest
    }

    pub(crate) const fn generation_before(&self) -> u64 {
        self.generation_before
    }

    pub(crate) const fn generation_after(&self) -> u64 {
        self.generation_after
    }
}

/// Opaque proof that the retained ledger authority and its generation stayed
/// stable across a complete read observation.
///
/// Only `ReceiptLedgerStore` can construct this value. Feature-only production
/// reachability owners may consume its read-only generation evidence, but they
/// cannot forge a successful validation step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StableReceiptLedgerObservation {
    generation_before: u64,
    generation_after: u64,
}

impl StableReceiptLedgerObservation {
    pub(crate) const fn generation_before(&self) -> u64 {
        self.generation_before
    }

    pub(crate) const fn generation_after(&self) -> u64 {
        self.generation_after
    }
}

#[cfg(feature = "receipt-ledger-test-support")]
pub(crate) struct ReceiptBackedTaskTerminalSeed {
    key: ReceiptKey,
    original_cutoff: OriginalCutoffDescriptor,
    task: ReceiptTaskProjection,
    terminal_epoch_ms: u64,
    terminal: V5CanonicalTerminal,
    cancel_requested: bool,
}

#[cfg(feature = "receipt-ledger-test-support")]
impl ReceiptBackedTaskTerminalSeed {
    pub(crate) fn new(
        key: ReceiptKey,
        original_cutoff: OriginalCutoffDescriptor,
        task: ReceiptTaskProjection,
        terminal_epoch_ms: u64,
        terminal: V5CanonicalTerminal,
        cancel_requested: bool,
    ) -> Self {
        Self {
            key,
            original_cutoff,
            task,
            terminal_epoch_ms,
            terminal,
            cancel_requested,
        }
    }
}

/// Retained receipt namespace authority.
///
/// The ownership lock lives inside the replaceable `receipts/` directory, so
/// every operation validates the complete named receipts/active/generation
/// chain before and after reading. Future mutating operations must preserve
/// that two-sided validation and classify a lost post-write name binding as an
/// uncertain commit; they must never report a successful mutation through the
/// displaced descriptor.
pub(crate) struct ReceiptLedgerStore {
    receipts: RetainedDirectoryCapability,
    receipts_file: File,
    active: RetainedDirectoryCapability,
    active_file: File,
    generation: Mutex<GenerationState>,
    generation_highwater: AtomicU64,
    writer: Mutex<ReceiptCatalog>,
    _ownership_lock: File,
}

impl ReceiptLedgerStore {
    pub(crate) fn open(receipts_path: impl AsRef<Path>) -> Result<Self, ReceiptLedgerError> {
        Self::open_before(
            receipts_path,
            Instant::now() + DEFAULT_RECEIPT_RECOVERY_TIMEOUT,
        )
    }

    pub(crate) fn open_before(
        receipts_path: impl AsRef<Path>,
        deadline: Instant,
    ) -> Result<Self, ReceiptLedgerError> {
        check_deadline(deadline)?;
        let receipts_path = receipts_path.as_ref();
        let receipts_file = open_or_create_owner_only_directory(receipts_path)?;
        check_deadline(deadline)?;
        let receipts = RetainedDirectoryCapability::open(receipts_path)
            .map_err(|error| storage_error("retain named receipts directory", error))?;
        if file_identity(&receipts_file)
            .map_err(|error| storage_error("identify receipts directory", error))?
            != receipts.identity()
        {
            return Err(ReceiptLedgerError::Corrupt(
                "receipts directory changed while retaining its named identity",
            ));
        }
        Self::open_retained_directory_with_file(receipts, receipts_file, deadline)
    }

    pub(crate) fn open_retained_directory(
        receipts: RetainedDirectoryCapability,
    ) -> Result<Self, ReceiptLedgerError> {
        Self::open_retained_directory_before(
            receipts,
            Instant::now() + DEFAULT_RECEIPT_RECOVERY_TIMEOUT,
        )
    }

    pub(crate) fn open_retained_directory_before(
        receipts: RetainedDirectoryCapability,
        deadline: Instant,
    ) -> Result<Self, ReceiptLedgerError> {
        check_deadline(deadline)?;
        let receipts_file = receipts
            .try_clone_directory()
            .map_err(|error| storage_error("clone retained receipts directory", error))?;
        Self::open_retained_directory_with_file(receipts, receipts_file, deadline)
    }

    fn open_retained_directory_with_file(
        receipts: RetainedDirectoryCapability,
        receipts_file: File,
        deadline: Instant,
    ) -> Result<Self, ReceiptLedgerError> {
        check_deadline(deadline)?;
        receipts
            .validate_named_identity()
            .map_err(|error| storage_error("validate named receipts directory", error))?;
        verify_owner_only_acl(&receipts_file)
            .map_err(|error| storage_error("verify receipts directory ownership", error))?;
        let ownership_lock =
            open_directory_ownership_lock(&receipts_file, OsStr::new(LEDGER_LOCK_FILE_NAME))
                .map_err(|error| storage_error("open receipt ledger ownership object", error))?;
        verify_owner_only_acl(&ownership_lock)
            .map_err(|error| storage_error("verify receipt ledger ownership object", error))?;
        match FileExt::try_lock_exclusive(&ownership_lock) {
            Ok(()) => {}
            Err(error) if lock_is_contended(&error) => {
                return Err(ReceiptLedgerError::AlreadyOwned)
            }
            Err(error) => {
                return Err(storage_error(
                    "acquire receipt ledger ownership lock",
                    error,
                ))
            }
        }

        let generation_staging = Self::inspect_generation_staging_before_initialization(
            &receipts,
            &receipts_file,
            deadline,
        )?;
        let existing_active = match open_directory_child_nofollow(
            &receipts_file,
            OsStr::new(ACTIVE_DIRECTORY_NAME),
        ) {
            Ok(active_file) => {
                verify_owner_only_acl(&active_file).map_err(|error| {
                    storage_error("verify existing receipt active directory ownership", error)
                })?;
                let active = receipts
                    .retain_directory_child(OsStr::new(ACTIVE_DIRECTORY_NAME))
                    .map_err(|error| {
                        storage_error("retain existing receipt active directory", error)
                    })?;
                if file_identity(&active_file).map_err(|error| {
                    storage_error("identify existing receipt active directory", error)
                })? != active.identity()
                {
                    return Err(ReceiptLedgerError::Corrupt(
                        "receipt active directory changed during recovery preflight",
                    ));
                }
                Some((active, active_file))
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => {
                return Err(storage_error(
                    "open existing receipt active directory no-follow",
                    error,
                ))
            }
        };
        let mut recovered = if let Some((active, active_file)) = &existing_active {
            Self::recover_existing_catalog(
                &receipts,
                &receipts_file,
                active,
                active_file,
                deadline,
            )?
        } else {
            RecoveredCatalog {
                catalog: ReceiptCatalog::default(),
                maximum_mutation_sequence: 0,
                staging: Vec::new(),
                expired_deletions: Vec::new(),
                expired_deletion_mutation_sequence: None,
                acknowledgement_recovery: None,
            }
        };
        check_deadline(deadline)?;
        let (generation_file, persisted_generation) = open_or_initialize_generation(
            &receipts_file,
            recovered.maximum_mutation_sequence,
            deadline,
        )?;
        let generation = receipts
            .retain_regular_child(OsStr::new(GENERATION_FILE_NAME))
            .map_err(|error| storage_error("retain named generation record", error))?;
        if file_identity(&generation_file)
            .map_err(|error| storage_error("identify generation record", error))?
            != generation.identity()
        {
            return Err(ReceiptLedgerError::Corrupt(
                "generation record changed while retaining its named identity",
            ));
        }
        if existing_active.is_none() && persisted_generation != 0 {
            return Err(ReceiptLedgerError::Corrupt(
                "nonzero receipt generation is missing its active directory",
            ));
        }
        if recovered.expired_deletion_mutation_sequence.is_some()
            && recovered.acknowledgement_recovery.is_some()
        {
            return Err(ReceiptLedgerError::Corrupt(
                "receipt recovery contains more than one pending mutation witness",
            ));
        }
        let pending_witness_sequence = recovered.expired_deletion_mutation_sequence.or_else(|| {
            recovered
                .acknowledgement_recovery
                .as_ref()
                .map(|recovery| recovery.mutation_sequence)
        });
        if let Some(witness_sequence) = pending_witness_sequence {
            let is_current_or_next = witness_sequence == persisted_generation
                || persisted_generation.checked_add(1) == Some(witness_sequence);
            if !is_current_or_next || witness_sequence != recovered.maximum_mutation_sequence {
                return Err(ReceiptLedgerError::Corrupt(
                    "pending receipt mutation witness is not the next persisted mutation",
                ));
            }
        }
        let (active, active_file) = match existing_active {
            Some(existing) => existing,
            None => {
                let active_file =
                    open_or_create_owner_only_child(&receipts_file, ACTIVE_DIRECTORY_NAME)?;
                let active = receipts
                    .retain_directory_child(OsStr::new(ACTIVE_DIRECTORY_NAME))
                    .map_err(|error| {
                        storage_error("retain initialized receipt active directory", error)
                    })?;
                (active, active_file)
            }
        };
        if file_identity(&active_file)
            .map_err(|error| storage_error("identify receipt active directory", error))?
            != active.identity()
        {
            return Err(ReceiptLedgerError::Corrupt(
                "receipt active directory changed while retaining its named identity",
            ));
        }
        let store = Self {
            receipts,
            receipts_file,
            active,
            active_file,
            generation: Mutex::new(GenerationState {
                capability: generation,
                file: generation_file,
            }),
            generation_highwater: AtomicU64::new(persisted_generation),
            writer: Mutex::new(ReceiptCatalog::default()),
            _ownership_lock: ownership_lock,
        };
        store.verify_named_authority()?;
        check_deadline(deadline)?;
        let confirmed_generation = store.generation()?;
        if confirmed_generation != persisted_generation {
            return Err(ReceiptLedgerError::Corrupt(
                "receipt generation changed during recovery",
            ));
        }
        check_deadline(deadline)?;
        store.remove_active_staging(recovered.staging, deadline)?;
        store.remove_generation_staging(generation_staging, deadline)?;
        if persisted_generation < recovered.maximum_mutation_sequence {
            check_deadline(deadline)?;
            store.publish_generation(recovered.maximum_mutation_sequence, None, Some(deadline))?;
        }
        if let Some(recovery) = recovered.acknowledgement_recovery.take() {
            store.publish_replacement_record(
                &recovery.compact_record,
                &recovery.compact_encoded,
                deadline,
                || {},
            )?;
            match store.read_active_record_bytes(&recovery.compact_record.key_digest) {
                Ok(Some(committed)) if committed == recovery.compact_encoded => {}
                Ok(Some(_)) | Ok(None) | Err(_) => {
                    return Err(ReceiptLedgerError::CommitUncertain {
                        receipt_key_digest: recovery.compact_record.key_digest,
                    })
                }
            }
        }
        // An ExpiredDeletion row is the durable commit witness for logical
        // removal.  Its mutation sequence must become authoritative before the
        // witness is unlinked, otherwise a crash could resurrect the previous
        // generation without any evidence that the receipt was deleted.
        store.remove_active_staging(recovered.expired_deletions, deadline)?;
        check_deadline(deadline)?;
        *store
            .writer
            .lock()
            .map_err(|_| ReceiptLedgerError::Corrupt("receipt catalog lock was poisoned"))? =
            recovered.catalog;
        check_deadline(deadline)?;
        Ok(store)
    }

    pub(crate) fn generation(&self) -> Result<u64, ReceiptLedgerError> {
        let mut catalog = self
            .writer
            .lock()
            .map_err(|_| ReceiptLedgerError::Corrupt("receipt catalog lock was poisoned"))?;
        if catalog.unavailable {
            return Err(ReceiptLedgerError::StoreUnavailable);
        }
        latch_catalog_result(&mut catalog, self.generation_under_writer_lock())
    }

    pub(crate) fn rotate_generation_for_test(
        &self,
        deadline: Instant,
    ) -> Result<u64, ReceiptLedgerError> {
        check_deadline(deadline)?;
        let mut catalog = self
            .writer
            .lock()
            .map_err(|_| ReceiptLedgerError::Corrupt("receipt catalog lock was poisoned"))?;
        if catalog.unavailable {
            return Err(ReceiptLedgerError::StoreUnavailable);
        }
        let generation = latch_catalog_result(&mut catalog, self.generation_under_writer_lock())?;
        let next = generation
            .checked_add(1)
            .ok_or(ReceiptLedgerError::Corrupt(
                "receipt generation exhausted u64",
            ))?;
        latch_catalog_result(
            &mut catalog,
            self.publish_generation(next, None, Some(deadline)),
        )?;
        Ok(next)
    }

    fn generation_under_writer_lock(&self) -> Result<u64, ReceiptLedgerError> {
        self.verify_named_authority()?;
        let mut generation = self
            .generation
            .lock()
            .map_err(|_| ReceiptLedgerError::Corrupt("generation reader lock was poisoned"))?;
        verify_owner_only_acl(&generation.file)
            .map_err(|error| storage_error("verify generation record ownership", error))?;
        generation
            .file
            .seek(SeekFrom::Start(0))
            .map_err(|error| storage_error("rewind generation record", error))?;
        let mut bytes = Vec::new();
        Read::by_ref(&mut generation.file)
            .take((MAX_GENERATION_FILE_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|error| storage_error("read generation record", error))?;
        let value = parse_generation(&bytes)?;
        drop(generation);
        self.verify_named_authority()?;
        Ok(value.max(self.generation_highwater.load(Ordering::Acquire)))
    }

    pub(crate) fn observe_stable_generation(
        &self,
    ) -> Result<StableReceiptLedgerObservation, ReceiptLedgerError> {
        let mut catalog = self
            .writer
            .lock()
            .map_err(|_| ReceiptLedgerError::Corrupt("receipt catalog lock was poisoned"))?;
        if catalog.unavailable {
            return Err(ReceiptLedgerError::StoreUnavailable);
        }
        latch_catalog_result(&mut catalog, self.verify_named_authority())?;
        let generation_before =
            latch_catalog_result(&mut catalog, self.generation_under_writer_lock())?;
        let generation_after =
            latch_catalog_result(&mut catalog, self.generation_under_writer_lock())?;
        if generation_after != generation_before {
            return latch_catalog_error(
                &mut catalog,
                ReceiptLedgerError::ConcurrentGenerationChange {
                    generation_before,
                    generation_after,
                },
            );
        }
        latch_catalog_result(&mut catalog, self.verify_named_authority())?;
        Ok(StableReceiptLedgerObservation {
            generation_before,
            generation_after,
        })
    }

    /// Returns the bounded set of active receipt keys under the same stable
    /// writer/generation fence used by startup inspection. Tombstones are not
    /// recovery work and are intentionally excluded.
    pub(crate) fn recovery_keys(
        &self,
        deadline: Instant,
    ) -> Result<Vec<ReceiptKey>, ReceiptLedgerError> {
        check_deadline(deadline)?;
        let mut catalog = self
            .writer
            .lock()
            .map_err(|_| ReceiptLedgerError::Corrupt("receipt catalog lock was poisoned"))?;
        if catalog.unavailable {
            return Err(ReceiptLedgerError::StoreUnavailable);
        }
        latch_catalog_result(&mut catalog, self.verify_named_authority())?;
        let generation_before =
            latch_catalog_result(&mut catalog, self.generation_under_writer_lock())?;
        let mut keys = catalog
            .records
            .iter()
            .filter(|(_, entry)| !entry.is_tombstone())
            .map(|(digest, entry)| (digest.clone(), entry.record.key.clone()))
            .collect::<Vec<_>>();
        if keys.len() != catalog.live_count() || keys.len() > MAX_LIVE_RECEIPTS {
            return latch_catalog_error(
                &mut catalog,
                ReceiptLedgerError::Corrupt(
                    "receipt startup catalog count contradicts its active keys",
                ),
            );
        }
        keys.sort_unstable_by(|(left, _), (right, _)| left.as_str().cmp(right.as_str()));
        let keys = keys.into_iter().map(|(_, key)| key).collect::<Vec<_>>();
        check_deadline(deadline)?;
        let generation_after =
            latch_catalog_result(&mut catalog, self.generation_under_writer_lock())?;
        if generation_after != generation_before {
            return latch_catalog_error(
                &mut catalog,
                ReceiptLedgerError::ConcurrentGenerationChange {
                    generation_before,
                    generation_after,
                },
            );
        }
        latch_catalog_result(&mut catalog, self.verify_named_authority())?;
        Ok(keys)
    }

    pub(crate) fn snapshot_catalog(
        &self,
        authority: ReceiptLedgerCatalogSnapshotAuthority,
        deadline: Instant,
    ) -> Result<ReceiptLedgerCatalogSnapshot, ReceiptLedgerError> {
        check_deadline(deadline)?;
        let mut catalog = self
            .writer
            .lock()
            .map_err(|_| ReceiptLedgerError::Corrupt("receipt catalog lock was poisoned"))?;
        let observed = self.inspect_catalog_with_generation_under_stable_fence(
            &mut catalog,
            Some(deadline),
            |catalog, generation| {
                let mut records = catalog.records.iter().collect::<Vec<_>>();
                records.sort_unstable_by(|(left, _), (right, _)| left.as_str().cmp(right.as_str()));
                let keys = records
                    .iter()
                    .filter(|(_, entry)| !entry.is_tombstone())
                    .map(|(_, entry)| entry.record.key.clone())
                    .collect::<Vec<_>>();
                let tombstones = records
                    .iter()
                    .filter(|(_, entry)| entry.is_tombstone())
                    .map(|(_, entry)| match entry.state()? {
                        ReceiptState::AcknowledgedTombstone(receipt) => Ok(receipt),
                        _ => Err(ReceiptLedgerError::Corrupt(
                            "tombstone catalog entry decoded as a live receipt",
                        )),
                    })
                    .collect::<Result<Vec<_>, _>>()?;

                let mut invocation_index = Vec::with_capacity(catalog.invocation_index.len());
                for (invocation_id, key_digest) in &catalog.invocation_index {
                    let entry =
                        catalog
                            .records
                            .get(key_digest)
                            .ok_or(ReceiptLedgerError::Corrupt(
                                "receipt invocation index points outside the catalog",
                            ))?;
                    if entry.record.key.invocation_id() != *invocation_id {
                        return Err(ReceiptLedgerError::Corrupt(
                            "receipt invocation index contradicts its catalog key",
                        ));
                    }
                    invocation_index.push((key_digest.clone(), entry.record.key.clone()));
                }
                invocation_index
                    .sort_unstable_by(|(left, _), (right, _)| left.as_str().cmp(right.as_str()));
                let invocation_index = invocation_index
                    .into_iter()
                    .map(|(_, key)| key)
                    .collect::<Vec<_>>();

                let mut reserved_task_index = Vec::with_capacity(catalog.reserved_task_index.len());
                for (reserved_task_id, key_digest) in &catalog.reserved_task_index {
                    let entry =
                        catalog
                            .records
                            .get(key_digest)
                            .ok_or(ReceiptLedgerError::Corrupt(
                                "receipt reserved-task index points outside the catalog",
                            ))?;
                    if entry.record.key.reserved_task_id() != *reserved_task_id {
                        return Err(ReceiptLedgerError::Corrupt(
                            "receipt reserved-task index contradicts its catalog key",
                        ));
                    }
                    reserved_task_index.push((key_digest.clone(), entry.record.key.clone()));
                }
                reserved_task_index
                    .sort_unstable_by(|(left, _), (right, _)| left.as_str().cmp(right.as_str()));
                let reserved_task_index = reserved_task_index
                    .into_iter()
                    .map(|(_, key)| key)
                    .collect::<Vec<_>>();

                Ok((
                    generation,
                    keys,
                    tombstones,
                    invocation_index,
                    reserved_task_index,
                    u64::try_from(catalog.live_count()).map_err(|_| {
                        ReceiptLedgerError::Corrupt("receipt catalog count does not fit telemetry")
                    })?,
                    catalog.actual_bytes,
                    catalog.reserved_result_bytes,
                    catalog.tombstone_bytes,
                ))
            },
        )?;
        let (
            generation,
            keys,
            tombstones,
            invocation_index,
            reserved_task_index,
            live_count,
            actual_bytes,
            reserved_result_bytes,
            tombstone_bytes,
        ) = match observed {
            Ok(observed) => observed,
            Err(error) => return latch_catalog_error(&mut catalog, error),
        };
        let snapshot = authority.seal(ReceiptLedgerCatalogSnapshotParts::new(
            generation,
            keys,
            tombstones,
            invocation_index,
            reserved_task_index,
            live_count,
            actual_bytes,
            reserved_result_bytes,
            tombstone_bytes,
        ));
        latch_catalog_result(&mut catalog, snapshot)
    }

    pub(crate) fn request_cancel_or_reserve(
        &self,
        key: ReceiptKey,
        cancel_reserved_at_epoch_ms: u64,
        deadline: Instant,
    ) -> Result<CancelResolution, ReceiptLedgerError> {
        check_deadline(deadline)?;
        let key_digest = receipt_key_digest(&key);
        let mut catalog = self
            .writer
            .lock()
            .map_err(|_| ReceiptLedgerError::Corrupt("receipt catalog lock was poisoned"))?;
        if catalog.unavailable {
            return Err(ReceiptLedgerError::StoreUnavailable);
        }
        check_deadline(deadline)?;
        latch_catalog_result(&mut catalog, self.verify_named_authority())?;

        let expires_at_epoch_ms = if let Some(existing) = catalog.records.get(&key_digest) {
            if existing.record.key != key {
                return self.reject_before_mutation(
                    &mut catalog,
                    deadline,
                    ReceiptLedgerError::ReceiptDigestCollision,
                );
            }
            let persisted = self
                .read_entry_under_writer_lock(&mut catalog, &key_digest, Some(deadline))?
                .ok_or(ReceiptLedgerError::Corrupt(
                    "catalogued receipt row is missing",
                ))?;
            let state = match persisted.state() {
                Ok(state) => state,
                Err(error) => return latch_catalog_error(&mut catalog, error),
            };
            match state {
                ReceiptState::CancelReserved(receipt)
                    if cancel_reserved_at_epoch_ms < receipt.expires_at_epoch_ms() =>
                {
                    return Ok(CancelResolution::ExistingExact(receipt));
                }
                ReceiptState::CancelReserved(_) => {
                    let expires_at_epoch_ms = cancel_reserved_at_epoch_ms
                        .checked_add(CANCEL_RESERVATION_TTL_MS)
                        .ok_or(ReceiptLedgerError::TimestampOverflow)?;
                    self.expire_cancel_reserved_entry_under_writer_lock(
                        &mut catalog,
                        persisted,
                        cancel_reserved_at_epoch_ms,
                        deadline,
                    )?;
                    expires_at_epoch_ms
                }
                ReceiptState::AcknowledgedTombstone(receipt)
                    if cancel_reserved_at_epoch_ms >= receipt.expires_at_epoch_ms() =>
                {
                    let expires_at_epoch_ms = cancel_reserved_at_epoch_ms
                        .checked_add(CANCEL_RESERVATION_TTL_MS)
                        .ok_or(ReceiptLedgerError::TimestampOverflow)?;
                    self.reclaim_expired_tombstone_under_writer_lock(
                        &mut catalog,
                        &key_digest,
                        cancel_reserved_at_epoch_ms,
                        deadline,
                    )?;
                    expires_at_epoch_ms
                }
                ReceiptState::DirectTerminalUnacked(receipt)
                    if receipt
                        .terminal_epoch_ms()
                        .checked_add(DIRECT_TERMINAL_RETENTION_MS)
                        .is_some_and(|expires_at_epoch_ms| {
                            cancel_reserved_at_epoch_ms >= expires_at_epoch_ms
                        }) =>
                {
                    let expires_at_epoch_ms = cancel_reserved_at_epoch_ms
                        .checked_add(CANCEL_RESERVATION_TTL_MS)
                        .ok_or(ReceiptLedgerError::TimestampOverflow)?;
                    self.reclaim_expired_direct_terminal_under_writer_lock(
                        &mut catalog,
                        &key_digest,
                        cancel_reserved_at_epoch_ms,
                        deadline,
                    )?;
                    expires_at_epoch_ms
                }
                ReceiptState::Reserved(receipt) if receipt.cancel_requested() => {
                    return Ok(CancelResolution::ExistingWinner(Box::new(
                        ReceiptState::Reserved(receipt),
                    )));
                }
                ReceiptState::Reserved(_) => {
                    let cancelled = self.commit_reserved_cancel_under_writer_lock(
                        &mut catalog,
                        persisted,
                        deadline,
                    )?;
                    return Ok(CancelResolution::ExistingWinner(Box::new(
                        ReceiptState::Reserved(cancelled),
                    )));
                }
                winner => return Ok(CancelResolution::ExistingWinner(Box::new(winner))),
            }
        } else {
            cancel_reserved_at_epoch_ms
                .checked_add(CANCEL_RESERVATION_TTL_MS)
                .ok_or(ReceiptLedgerError::TimestampOverflow)?
        };

        self.prepare_new_admission_under_writer_lock(
            &mut catalog,
            &key,
            cancel_reserved_at_epoch_ms,
            deadline,
        )?;
        let generation = latch_catalog_result(&mut catalog, self.generation_under_writer_lock())?;
        let mutation_sequence = match generation.checked_add(1) {
            Some(sequence) => sequence,
            None => {
                return self.reject_before_mutation(
                    &mut catalog,
                    deadline,
                    ReceiptLedgerError::Corrupt("receipt generation exhausted u64"),
                )
            }
        };
        let record = build_cancel_reserved_record(
            key,
            key_digest.clone(),
            cancel_reserved_at_epoch_ms,
            expires_at_epoch_ms,
            mutation_sequence,
        );
        let (record, encoded) =
            match serialize_reserved_record(record, MAX_CANCEL_RESERVED_RECORD_BYTES) {
                Ok(serialized) => serialized,
                Err(error) => return self.reject_before_mutation(&mut catalog, deadline, error),
            };
        let encoded_bytes = match u64::try_from(encoded.len()) {
            Ok(encoded_bytes) => encoded_bytes,
            Err(_) => {
                return self.reject_before_mutation(
                    &mut catalog,
                    deadline,
                    ReceiptLedgerError::RecordTooLarge,
                )
            }
        };
        let entry = CatalogEntry {
            record: record.clone(),
            encoded_bytes,
        };
        if let Err(error) = validate_catalog_insert(&catalog, &entry, false) {
            return self.reject_before_mutation(&mut catalog, deadline, error);
        }
        if let Err(error) = self.publish_new_record(&record, &encoded, deadline, || {
            commit_catalog_insert(&mut catalog, entry);
        }) {
            if !matches!(error, ReceiptLedgerError::DeadlineExceeded) {
                catalog.unavailable = true;
            }
            return Err(error);
        }
        if let Err(error) =
            self.publish_generation(mutation_sequence, Some(&key_digest), Some(deadline))
        {
            catalog.unavailable = true;
            return Err(error);
        }
        match self.read_active_record_bytes(&key_digest) {
            Ok(Some(committed)) if committed.as_slice() == encoded.as_slice() => {}
            Ok(Some(_)) | Ok(None) | Err(_) => {
                catalog.unavailable = true;
                return Err(ReceiptLedgerError::CommitUncertain {
                    receipt_key_digest: key_digest.clone(),
                });
            }
        }
        if check_deadline(deadline).is_err() || self.verify_named_authority().is_err() {
            catalog.unavailable = true;
            return Err(ReceiptLedgerError::CommitUncertain {
                receipt_key_digest: key_digest.clone(),
            });
        }
        let committed = match catalog.records.get(&key_digest).cloned() {
            Some(committed) if committed.record == record => committed,
            Some(_) | None => {
                catalog.unavailable = true;
                return Err(ReceiptLedgerError::CommitUncertain {
                    receipt_key_digest: key_digest,
                });
            }
        };
        match committed.state() {
            Ok(ReceiptState::CancelReserved(receipt)) => {
                Ok(CancelResolution::NewlyReserved(receipt))
            }
            Ok(_) | Err(_) => {
                catalog.unavailable = true;
                Err(ReceiptLedgerError::CommitUncertain {
                    receipt_key_digest: key_digest,
                })
            }
        }
    }

    pub(crate) fn reserve_batch(
        &self,
        requests: Vec<(ReceiptKey, OriginalCutoffDescriptor)>,
        deadline: Instant,
    ) -> Result<Vec<ReserveOutcome>, ReceiptLedgerError> {
        if requests.is_empty() || requests.len() > MAX_RECEIPT_MUTATION_BATCH_ROWS {
            return Err(ReceiptLedgerError::ReceiptRowPresentUnsupported);
        }
        check_deadline(deadline)?;
        let mut catalog = self
            .writer
            .lock()
            .map_err(|_| ReceiptLedgerError::Corrupt("receipt catalog lock was poisoned"))?;
        if catalog.unavailable {
            return Err(ReceiptLedgerError::StoreUnavailable);
        }
        latch_catalog_result(&mut catalog, self.verify_named_authority())?;
        let mut generation =
            latch_catalog_result(&mut catalog, self.generation_under_writer_lock())?;
        let mut unique_digests = HashSet::with_capacity(requests.len());
        let mut unique_invocations = HashSet::with_capacity(requests.len());
        let mut unique_tasks = HashSet::with_capacity(requests.len());
        let mut rows = Vec::with_capacity(requests.len());
        let mut entries = Vec::with_capacity(requests.len());
        for (key, cutoff) in &requests {
            let digest = receipt_key_digest(key);
            if catalog.records.contains_key(&digest)
                || catalog.invocation_index.contains_key(&key.invocation_id())
                || catalog
                    .reserved_task_index
                    .contains_key(&key.reserved_task_id())
                || !unique_digests.insert(digest.clone())
                || !unique_invocations.insert(key.invocation_id())
                || !unique_tasks.insert(key.reserved_task_id())
            {
                return Err(ReceiptLedgerError::ReceiptRowPresentUnsupported);
            }
            generation = generation
                .checked_add(1)
                .ok_or(ReceiptLedgerError::Corrupt(
                    "receipt generation exhausted u64",
                ))?;
            let record = build_reserved_record(
                key.clone(),
                digest,
                *cutoff,
                generation,
                ReceiptVersion::initial(),
                false,
            );
            let (record, encoded) =
                serialize_reserved_record(record, MAX_TASK_RECORD_ENVELOPE_BYTES as u64)?;
            let entry = CatalogEntry {
                record: record.clone(),
                encoded_bytes: encoded.len() as u64,
            };
            entries.push(entry);
            rows.push(ReceiptBatchRow { record, encoded });
        }
        validate_catalog_insert_batch(&catalog, &entries)?;
        self.publish_record_batch(&mut catalog, &rows, generation, deadline, move |catalog| {
            for entry in entries {
                commit_catalog_insert(catalog, entry);
            }
        })?;
        requests
            .iter()
            .map(|(key, _)| {
                let entry = catalog
                    .records
                    .get(&receipt_key_digest(key))
                    .cloned()
                    .ok_or(ReceiptLedgerError::ReceiptNotFound)?;
                Ok(ReserveOutcome::Created(entry.reservation()?))
            })
            .collect()
    }

    pub(crate) fn bind_reserved_actor_batch(
        &self,
        requests: Vec<(ReceiptKey, ReceiptVersion, SafeIdentityHash)>,
        deadline: Instant,
    ) -> Result<Vec<ReservedReceipt>, ReceiptLedgerError> {
        self.transition_reserved_batch(requests, false, deadline)
    }

    pub(crate) fn mark_reserved_begun_batch(
        &self,
        requests: Vec<(ReceiptKey, ReceiptVersion, SafeIdentityHash)>,
        deadline: Instant,
    ) -> Result<Vec<ReservedReceipt>, ReceiptLedgerError> {
        self.transition_reserved_batch(requests, true, deadline)
    }

    fn transition_reserved_batch(
        &self,
        requests: Vec<(ReceiptKey, ReceiptVersion, SafeIdentityHash)>,
        begun: bool,
        deadline: Instant,
    ) -> Result<Vec<ReservedReceipt>, ReceiptLedgerError> {
        if requests.is_empty() || requests.len() > MAX_RECEIPT_MUTATION_BATCH_ROWS {
            return Err(ReceiptLedgerError::ReceiptRowPresentUnsupported);
        }
        check_deadline(deadline)?;
        let mut catalog = self
            .writer
            .lock()
            .map_err(|_| ReceiptLedgerError::Corrupt("receipt catalog lock was poisoned"))?;
        if catalog.unavailable {
            return Err(ReceiptLedgerError::StoreUnavailable);
        }
        latch_catalog_result(&mut catalog, self.verify_named_authority())?;
        let mut generation =
            latch_catalog_result(&mut catalog, self.generation_under_writer_lock())?;
        let mut unique_digests = HashSet::with_capacity(requests.len());
        let mut rows = Vec::with_capacity(requests.len());
        let mut replacements = Vec::with_capacity(requests.len());
        for (key, expected_version, workspace_identity) in &requests {
            let digest = receipt_key_digest(key);
            if !unique_digests.insert(digest.clone()) {
                return Err(ReceiptLedgerError::ReceiptRowPresentUnsupported);
            }
            let expected = catalog
                .records
                .get(&digest)
                .cloned()
                .ok_or(ReceiptLedgerError::ReceiptNotFound)?;
            if expected.record.key != *key || expected.record.record_version != *expected_version {
                return Err(ReceiptLedgerError::ReceiptVersionMismatch {
                    expected: *expected_version,
                    actual: expected.record.record_version,
                });
            }
            let reserved = expected.reservation()?;
            let next_phase = if begun {
                match reserved.phase() {
                    ReservedPhase::ActorBound {
                        bound_workspace_identity,
                    } if bound_workspace_identity == workspace_identity => ReservedPhase::Begun {
                        bound_workspace_identity: workspace_identity.clone(),
                    },
                    _ => return Err(ReceiptLedgerError::ReceiptRowPresentUnsupported),
                }
            } else if matches!(reserved.phase(), ReservedPhase::Unbound) {
                ReservedPhase::ActorBound {
                    bound_workspace_identity: workspace_identity.clone(),
                }
            } else {
                return Err(ReceiptLedgerError::ReceiptRowPresentUnsupported);
            };
            generation = generation
                .checked_add(1)
                .ok_or(ReceiptLedgerError::Corrupt(
                    "receipt generation exhausted u64",
                ))?;
            let next_version =
                expected_version
                    .checked_next()
                    .ok_or(ReceiptLedgerError::Corrupt(
                        "receipt record version exhausted u64",
                    ))?;
            let record =
                build_reserved_phase_record(&expected, next_phase, generation, next_version)?;
            let (record, encoded) =
                serialize_reserved_record(record, MAX_TASK_RECORD_ENVELOPE_BYTES as u64)?;
            let replacement = CatalogEntry {
                record: record.clone(),
                encoded_bytes: encoded.len() as u64,
            };
            replacements.push((expected, replacement));
            rows.push(ReceiptBatchRow { record, encoded });
        }
        validate_catalog_replace_batch(&catalog, &replacements)?;
        self.publish_record_batch(&mut catalog, &rows, generation, deadline, move |catalog| {
            for (_, replacement) in replacements {
                commit_catalog_replace(catalog, replacement);
            }
        })?;
        requests
            .iter()
            .map(|(key, _, _)| {
                catalog
                    .records
                    .get(&receipt_key_digest(key))
                    .ok_or(ReceiptLedgerError::ReceiptNotFound)?
                    .reservation()
            })
            .collect()
    }

    pub(crate) fn publish_direct_terminal_batch(
        &self,
        requests: Vec<(ReceiptKey, ReceiptVersion, u64, V5CanonicalTerminal)>,
        deadline: Instant,
    ) -> Result<Vec<CommittedDirectPublication>, ReceiptLedgerError> {
        if requests.is_empty() || requests.len() > MAX_RECEIPT_MUTATION_BATCH_ROWS {
            return Err(ReceiptLedgerError::ReceiptRowPresentUnsupported);
        }
        check_deadline(deadline)?;
        let mut catalog = self
            .writer
            .lock()
            .map_err(|_| ReceiptLedgerError::Corrupt("receipt catalog lock was poisoned"))?;
        if catalog.unavailable {
            return Err(ReceiptLedgerError::StoreUnavailable);
        }
        latch_catalog_result(&mut catalog, self.verify_named_authority())?;
        let mut generation =
            latch_catalog_result(&mut catalog, self.generation_under_writer_lock())?;
        let mut unique_digests = HashSet::with_capacity(requests.len());
        let mut rows = Vec::with_capacity(requests.len());
        let mut replacements = Vec::with_capacity(requests.len());
        let mut publications = Vec::with_capacity(requests.len());
        for (key, expected_version, terminal_epoch_ms, terminal) in &requests {
            let digest = receipt_key_digest(key);
            if !unique_digests.insert(digest.clone()) {
                return Err(ReceiptLedgerError::ReceiptRowPresentUnsupported);
            }
            let expected = catalog
                .records
                .get(&digest)
                .cloned()
                .ok_or(ReceiptLedgerError::ReceiptNotFound)?;
            if expected.record.key != *key || expected.record.record_version != *expected_version {
                return Err(ReceiptLedgerError::ReceiptVersionMismatch {
                    expected: *expected_version,
                    actual: expected.record.record_version,
                });
            }
            let reserved = expected.reservation()?;
            let next_version =
                expected_version
                    .checked_next()
                    .ok_or(ReceiptLedgerError::Corrupt(
                        "receipt record version exhausted u64",
                    ))?;
            let generation_before = generation;
            generation = generation
                .checked_add(1)
                .ok_or(ReceiptLedgerError::Corrupt(
                    "receipt generation exhausted u64",
                ))?;
            let prepared = prepare_direct_terminal(
                DirectReceiptWriteSlot::new(
                    key,
                    *expected_version,
                    next_version,
                    generation_before,
                    generation,
                    *reserved.original_cutoff(),
                )?,
                terminal.clone(),
                *terminal_epoch_ms,
            )?;
            let (prepared_record, wire_frame) = prepared.into_parts();
            let binding = prepared_record.binding();
            let record = StoredActiveReceiptV1 {
                schema_version: RECEIPT_RECORD_SCHEMA_VERSION,
                mutation_sequence: binding.mutation_sequence(),
                record_version: binding.committed_version(),
                key: binding.key().clone(),
                key_digest: binding.key_digest().clone(),
                lifecycle: StoredActiveLifecycleV1::DirectTerminalUnacked {
                    original_cutoff: binding.original_cutoff(),
                    terminal_epoch_ms: binding.terminal_epoch_ms(),
                    terminal_digest: binding.terminal_digest().clone(),
                    terminal: prepared_record.terminal().outcome_shared(),
                },
            };
            let encoded = prepared_record.bytes().to_vec();
            let replacement = CatalogEntry {
                record: record.clone(),
                encoded_bytes: prepared_record.encoded_bytes(),
            };
            replacements.push((expected, replacement));
            let receipt = DirectTerminalUnackedReceipt::new(
                ReceiptRecordHeader::new(
                    binding.key().clone(),
                    binding.key_digest().clone(),
                    binding.committed_version(),
                    binding.mutation_sequence(),
                    prepared_record.encoded_bytes(),
                ),
                binding.original_cutoff(),
                binding.terminal_epoch_ms(),
                prepared_record.terminal().clone(),
                prepared_record.reserved_result_bytes(),
            );
            publications.push(CommittedDirectPublication::with_prepared_record(
                receipt,
                wire_frame,
                prepared_record,
            ));
            rows.push(ReceiptBatchRow { record, encoded });
        }
        validate_catalog_replace_batch(&catalog, &replacements)?;
        self.publish_record_batch(&mut catalog, &rows, generation, deadline, move |catalog| {
            for (_, replacement) in replacements {
                commit_catalog_replace(catalog, replacement);
            }
        })?;
        Ok(publications)
    }

    pub(crate) fn acknowledge_direct_batch(
        &self,
        requests: Vec<(ReceiptKey, TerminalDigest, u64)>,
        deadline: Instant,
    ) -> Result<Vec<AcknowledgedTombstoneReceipt>, ReceiptLedgerError> {
        if requests.is_empty() || requests.len() > MAX_RECEIPT_MUTATION_BATCH_ROWS {
            return Err(ReceiptLedgerError::ReceiptRowPresentUnsupported);
        }
        check_deadline(deadline)?;
        let mut catalog = self
            .writer
            .lock()
            .map_err(|_| ReceiptLedgerError::Corrupt("receipt catalog lock was poisoned"))?;
        if catalog.unavailable {
            return Err(ReceiptLedgerError::StoreUnavailable);
        }
        latch_catalog_result(&mut catalog, self.verify_named_authority())?;
        let mut generation =
            latch_catalog_result(&mut catalog, self.generation_under_writer_lock())?;
        let mut unique_digests = HashSet::with_capacity(requests.len());
        let mut rows = Vec::with_capacity(requests.len());
        let mut replacements = Vec::with_capacity(requests.len());
        let mut acknowledgements = Vec::with_capacity(requests.len());
        for (key, terminal_digest, acknowledged_at_epoch_ms) in &requests {
            let digest = receipt_key_digest(key);
            if !unique_digests.insert(digest.clone()) {
                return Err(ReceiptLedgerError::ReceiptRowPresentUnsupported);
            }
            let expected = catalog
                .records
                .get(&digest)
                .cloned()
                .ok_or(ReceiptLedgerError::ReceiptNotFound)?;
            let direct = match expected.state()? {
                ReceiptState::DirectTerminalUnacked(receipt) => receipt,
                _ => return Err(ReceiptLedgerError::ReceiptRowPresentUnsupported),
            };
            if direct.terminal().digest() != terminal_digest {
                return Err(ReceiptLedgerError::TerminalMismatch);
            }
            generation = generation
                .checked_add(1)
                .ok_or(ReceiptLedgerError::Corrupt(
                    "receipt generation exhausted u64",
                ))?;
            let record = StoredActiveReceiptV1 {
                schema_version: RECEIPT_RECORD_SCHEMA_VERSION,
                mutation_sequence: generation,
                record_version: expected.record.record_version.checked_next().ok_or(
                    ReceiptLedgerError::Corrupt("receipt record version exhausted u64"),
                )?,
                key: key.clone(),
                key_digest: digest,
                lifecycle: StoredActiveLifecycleV1::AcknowledgedTombstone {
                    terminal_digest: terminal_digest.clone(),
                    acknowledged_at_epoch_ms: *acknowledged_at_epoch_ms,
                },
            };
            let (record, encoded) =
                serialize_reserved_record(record, MAX_ACKNOWLEDGED_TOMBSTONE_BYTES)?;
            let replacement = CatalogEntry {
                record: record.clone(),
                encoded_bytes: encoded.len() as u64,
            };
            replacements.push((expected, replacement));
            acknowledgements.push(AcknowledgedTombstoneReceipt::new(
                key.clone(),
                receipt_key_digest(key),
                terminal_digest.clone(),
                *acknowledged_at_epoch_ms,
                encoded.len() as u64,
            )?);
            rows.push(ReceiptBatchRow { record, encoded });
        }
        if let Some(backing) = catalog.tombstone_compaction_backing.clone() {
            let mut compacted = decode_receipt_batch(backing.encoded.as_slice())?
                .rows
                .into_iter()
                .map(|row| row.record)
                .filter(|record| {
                    let digest = &record.key_digest;
                    catalog.batch_backing.get(digest) == Some(&backing)
                        && catalog
                            .records
                            .get(digest)
                            .is_some_and(|entry| entry.is_tombstone() && entry.record == *record)
                })
                .collect::<Vec<_>>();
            compacted.sort_unstable_by(|left, right| {
                left.key_digest.as_str().cmp(right.key_digest.as_str())
            });
            if rows.len().saturating_add(compacted.len()) <= MAX_RECEIPT_BATCH_ENVELOPE_ROWS {
                for record in compacted {
                    let (record, encoded) =
                        serialize_reserved_record(record, MAX_ACKNOWLEDGED_TOMBSTONE_BYTES)?;
                    rows.push(ReceiptBatchRow { record, encoded });
                }
            }
        }
        validate_catalog_replace_batch(&catalog, &replacements)?;
        self.publish_record_batch(&mut catalog, &rows, generation, deadline, move |catalog| {
            for (_, replacement) in replacements {
                commit_catalog_replace(catalog, replacement);
            }
        })?;
        Ok(acknowledgements)
    }

    pub(crate) fn publish_cancelled_direct_batch(
        &self,
        requests: Vec<(ReceiptKey, OriginalCutoffDescriptor)>,
        terminal_epoch_ms: u64,
        terminal: V5CanonicalTerminal,
        deadline: Instant,
    ) -> Result<Vec<DirectTerminalUnackedReceipt>, ReceiptLedgerError> {
        if requests.is_empty() || requests.len() > 32 {
            return Err(ReceiptLedgerError::ReceiptRowPresentUnsupported);
        }
        if !matches!(terminal.outcome(), ReceiptTerminalOutcome::Cancelled) {
            return Err(ReceiptLedgerError::TerminalMismatch);
        }
        check_deadline(deadline)?;
        let mut catalog = self
            .writer
            .lock()
            .map_err(|_| ReceiptLedgerError::Corrupt("receipt catalog lock was poisoned"))?;
        if catalog.unavailable {
            return Err(ReceiptLedgerError::StoreUnavailable);
        }
        latch_catalog_result(&mut catalog, self.verify_named_authority())?;
        let mut unique_invocations = HashSet::with_capacity(requests.len());
        let mut unique_tasks = HashSet::with_capacity(requests.len());
        let mut unique_digests = HashSet::with_capacity(requests.len());
        for (key, _) in &requests {
            if !unique_invocations.insert(key.invocation_id())
                || !unique_tasks.insert(key.reserved_task_id())
                || !unique_digests.insert(receipt_key_digest(key))
            {
                return Err(ReceiptLedgerError::InvocationIdentityMismatch);
            }
        }

        let mut generation =
            latch_catalog_result(&mut catalog, self.generation_under_writer_lock())?;
        let mut shadow = catalog.clone();
        for (key, cutoff) in &requests {
            generation = generation
                .checked_add(1)
                .ok_or(ReceiptLedgerError::Corrupt(
                    "receipt generation exhausted u64",
                ))?;
            let expires_at_epoch_ms = cutoff
                .accepted_epoch_ms()
                .checked_add(CANCEL_RESERVATION_TTL_MS)
                .ok_or(ReceiptLedgerError::TimestampOverflow)?;
            let record = build_cancel_reserved_record(
                key.clone(),
                receipt_key_digest(key),
                cutoff.accepted_epoch_ms(),
                expires_at_epoch_ms,
                generation,
            );
            let (record, encoded) =
                serialize_reserved_record(record, MAX_CANCEL_RESERVED_RECORD_BYTES)?;
            let entry = CatalogEntry {
                record: record.clone(),
                encoded_bytes: u64::try_from(encoded.len())
                    .map_err(|_| ReceiptLedgerError::RecordTooLarge)?,
            };
            validate_catalog_insert(&shadow, &entry, false)?;
            commit_catalog_insert(&mut shadow, entry);
            let _ = encoded;
        }

        for (key, cutoff) in &requests {
            let digest = receipt_key_digest(key);
            let expected = shadow
                .records
                .get(&digest)
                .cloned()
                .ok_or(ReceiptLedgerError::ReceiptNotFound)?;
            generation = generation
                .checked_add(1)
                .ok_or(ReceiptLedgerError::Corrupt(
                    "receipt generation exhausted u64",
                ))?;
            let record = build_reserved_record(
                key.clone(),
                digest,
                *cutoff,
                generation,
                expected.record.record_version.checked_next().ok_or(
                    ReceiptLedgerError::Corrupt("receipt record version exhausted u64"),
                )?,
                true,
            );
            let (record, encoded) =
                serialize_reserved_record(record, MAX_TASK_RECORD_ENVELOPE_BYTES as u64)?;
            let replacement = CatalogEntry {
                record: record.clone(),
                encoded_bytes: u64::try_from(encoded.len())
                    .map_err(|_| ReceiptLedgerError::RecordTooLarge)?,
            };
            validate_catalog_replace(&shadow, &expected, &replacement)?;
            commit_catalog_replace(&mut shadow, replacement);
            let _ = encoded;
        }

        let mut rows = Vec::with_capacity(requests.len());
        for (key, cutoff) in &requests {
            let digest = receipt_key_digest(key);
            let expected = shadow
                .records
                .get(&digest)
                .cloned()
                .ok_or(ReceiptLedgerError::ReceiptNotFound)?;
            let expected_version = expected.record.record_version;
            let committed_version =
                expected_version
                    .checked_next()
                    .ok_or(ReceiptLedgerError::Corrupt(
                        "receipt record version exhausted u64",
                    ))?;
            let generation_before = generation;
            generation = generation
                .checked_add(1)
                .ok_or(ReceiptLedgerError::Corrupt(
                    "receipt generation exhausted u64",
                ))?;
            let prepared = prepare_direct_terminal(
                DirectReceiptWriteSlot::new(
                    key,
                    expected_version,
                    committed_version,
                    generation_before,
                    generation,
                    *cutoff,
                )?,
                terminal.clone(),
                terminal_epoch_ms,
            )?;
            let prepared_record = prepared.record();
            let binding = prepared_record.binding();
            let record = StoredActiveReceiptV1 {
                schema_version: RECEIPT_RECORD_SCHEMA_VERSION,
                mutation_sequence: binding.mutation_sequence(),
                record_version: binding.committed_version(),
                key: binding.key().clone(),
                key_digest: binding.key_digest().clone(),
                lifecycle: StoredActiveLifecycleV1::DirectTerminalUnacked {
                    original_cutoff: binding.original_cutoff(),
                    terminal_epoch_ms: binding.terminal_epoch_ms(),
                    terminal_digest: binding.terminal_digest().clone(),
                    terminal: prepared_record.terminal().outcome_shared(),
                },
            };
            let encoded = prepared_record.bytes().to_vec();
            let replacement = CatalogEntry {
                record: record.clone(),
                encoded_bytes: prepared_record.encoded_bytes(),
            };
            validate_catalog_replace(&shadow, &expected, &replacement)?;
            commit_catalog_replace(&mut shadow, replacement);
            rows.push(ReceiptBatchRow {
                record,
                encoded,
                // The three lifecycle transitions above form one coordinated
                // cancel-before-submit transaction.  No intermediate state is
                // externally observable, so only the final winner is made
                // durable.  Mutation sequences and record versions still
                // account for every logical transition.
            });
        }
        self.publish_record_batch(&mut catalog, &rows, generation, deadline, move |catalog| {
            *catalog = shadow;
        })?;
        requests
            .iter()
            .map(|(key, _)| {
                let digest = receipt_key_digest(key);
                let entry = catalog
                    .records
                    .get(&digest)
                    .cloned()
                    .ok_or(ReceiptLedgerError::ReceiptNotFound)?;
                match entry.state()? {
                    ReceiptState::DirectTerminalUnacked(receipt) => Ok(receipt),
                    _ => Err(ReceiptLedgerError::ReceiptRowPresentUnsupported),
                }
            })
            .collect()
    }

    fn commit_reserved_cancel_under_writer_lock(
        &self,
        catalog: &mut ReceiptCatalog,
        persisted: CatalogEntry,
        deadline: Instant,
    ) -> Result<ReservedReceipt, ReceiptLedgerError> {
        let key_digest = persisted.record.key_digest.clone();
        let expected_version = persisted.record.record_version;
        let next_record_version =
            expected_version
                .checked_next()
                .ok_or(ReceiptLedgerError::Corrupt(
                    "receipt record version exhausted u64",
                ))?;
        let generation = latch_catalog_result(catalog, self.generation_under_writer_lock())?;
        let mutation_sequence = generation
            .checked_add(1)
            .ok_or(ReceiptLedgerError::Corrupt(
                "receipt generation exhausted u64",
            ))?;
        let record =
            build_reserved_cancel_record(&persisted, mutation_sequence, next_record_version)?;
        let (record, encoded) =
            serialize_reserved_record(record, MAX_TASK_RECORD_ENVELOPE_BYTES as u64)?;
        let replacement = CatalogEntry {
            record: record.clone(),
            encoded_bytes: u64::try_from(encoded.len())
                .map_err(|_| ReceiptLedgerError::RecordTooLarge)?,
        };
        validate_catalog_replace(catalog, &persisted, &replacement)?;
        if let Err(error) = self.publish_replacement_record(&record, &encoded, deadline, || {
            commit_catalog_replace(catalog, replacement);
        }) {
            if !matches!(error, ReceiptLedgerError::DeadlineExceeded) {
                catalog.unavailable = true;
            }
            return Err(error);
        }
        if let Err(error) =
            self.publish_generation(mutation_sequence, Some(&key_digest), Some(deadline))
        {
            catalog.unavailable = true;
            return Err(error);
        }
        match self.read_active_record_bytes(&key_digest) {
            Ok(Some(committed)) if committed == encoded => {}
            Ok(Some(_)) | Ok(None) | Err(_) => {
                catalog.unavailable = true;
                return Err(ReceiptLedgerError::CommitUncertain {
                    receipt_key_digest: key_digest,
                });
            }
        }
        let committed =
            catalog
                .records
                .get(&key_digest)
                .ok_or(ReceiptLedgerError::CommitUncertain {
                    receipt_key_digest: key_digest.clone(),
                })?;
        committed.reservation()
    }

    pub(crate) fn expire_cancel_reserved(
        &self,
        key: ReceiptKey,
        expected_version: ReceiptVersion,
        expected_mutation_sequence: u64,
        observed_at_epoch_ms: u64,
        deadline: Instant,
    ) -> Result<CancelExpiryOutcome, ReceiptLedgerError> {
        check_deadline(deadline)?;
        let key_digest = receipt_key_digest(&key);
        let mut catalog = self
            .writer
            .lock()
            .map_err(|_| ReceiptLedgerError::Corrupt("receipt catalog lock was poisoned"))?;
        if catalog.unavailable {
            return Err(ReceiptLedgerError::StoreUnavailable);
        }
        check_deadline(deadline)?;
        latch_catalog_result(&mut catalog, self.verify_named_authority())?;

        let Some(expected) = catalog.records.get(&key_digest).cloned() else {
            if catalog.invocation_index.contains_key(&key.invocation_id()) {
                return self.reject_before_mutation(
                    &mut catalog,
                    deadline,
                    ReceiptLedgerError::InvocationIdentityMismatch,
                );
            }
            if catalog
                .reserved_task_index
                .contains_key(&key.reserved_task_id())
            {
                return self.reject_before_mutation(
                    &mut catalog,
                    deadline,
                    ReceiptLedgerError::ReservedTaskIdentityMismatch,
                );
            }
            return match self.read_entry_under_writer_lock(
                &mut catalog,
                &key_digest,
                Some(deadline),
            )? {
                None => Ok(CancelExpiryOutcome::Missing),
                Some(_) => latch_catalog_error(
                    &mut catalog,
                    ReceiptLedgerError::Corrupt(
                        "receipt row is present outside the recovered catalog",
                    ),
                ),
            };
        };
        if expected.record.key != key {
            return self.reject_before_mutation(
                &mut catalog,
                deadline,
                ReceiptLedgerError::ReceiptDigestCollision,
            );
        }
        let persisted = self
            .read_entry_under_writer_lock(&mut catalog, &key_digest, Some(deadline))?
            .ok_or(ReceiptLedgerError::Corrupt(
                "catalogued receipt row is missing",
            ))?;
        if persisted != expected {
            return latch_catalog_error(
                &mut catalog,
                ReceiptLedgerError::Corrupt("catalogued receipt row changed on disk"),
            );
        }
        let state = match persisted.state() {
            Ok(state) => state,
            Err(error) => return latch_catalog_error(&mut catalog, error),
        };
        let cancel_reserved = match state {
            ReceiptState::CancelReserved(receipt) => receipt,
            winner => return Ok(CancelExpiryOutcome::ExistingWinner(Box::new(winner))),
        };
        if cancel_reserved.record_version() != expected_version {
            return Err(ReceiptLedgerError::ReceiptVersionMismatch {
                expected: expected_version,
                actual: cancel_reserved.record_version(),
            });
        }
        if cancel_reserved.mutation_sequence() != expected_mutation_sequence {
            return Err(ReceiptLedgerError::ReceiptMutationSequenceMismatch {
                expected: expected_mutation_sequence,
                actual: cancel_reserved.mutation_sequence(),
            });
        }
        if observed_at_epoch_ms < cancel_reserved.expires_at_epoch_ms() {
            return Ok(CancelExpiryOutcome::NotDue(cancel_reserved));
        }

        self.expire_cancel_reserved_entry_under_writer_lock(
            &mut catalog,
            persisted,
            observed_at_epoch_ms,
            deadline,
        )?;
        Ok(CancelExpiryOutcome::Expired)
    }

    fn prepare_new_admission_under_writer_lock(
        &self,
        catalog: &mut ReceiptCatalog,
        key: &ReceiptKey,
        observed_at_epoch_ms: u64,
        deadline: Instant,
    ) -> Result<(), ReceiptLedgerError> {
        let mut reclaim = Vec::with_capacity(2);
        if let Some(digest) = catalog.invocation_index.get(&key.invocation_id()).cloned() {
            let expired = match catalog_entry_is_expired_identity_reclaimable(
                catalog,
                &digest,
                observed_at_epoch_ms,
            ) {
                Ok(expired) => expired,
                Err(error) => return latch_catalog_error(catalog, error),
            };
            if !expired {
                return self.reject_before_mutation(
                    catalog,
                    deadline,
                    ReceiptLedgerError::InvocationIdentityMismatch,
                );
            }
            reclaim.push(digest);
        }
        if let Some(digest) = catalog
            .reserved_task_index
            .get(&key.reserved_task_id())
            .cloned()
        {
            let expired = match catalog_entry_is_expired_identity_reclaimable(
                catalog,
                &digest,
                observed_at_epoch_ms,
            ) {
                Ok(expired) => expired,
                Err(error) => return latch_catalog_error(catalog, error),
            };
            if !expired {
                return self.reject_before_mutation(
                    catalog,
                    deadline,
                    ReceiptLedgerError::ReservedTaskIdentityMismatch,
                );
            }
            reclaim.push(digest);
        }
        reclaim.sort_unstable_by(|left, right| left.as_str().cmp(right.as_str()));
        reclaim.dedup();

        let reclaimed_live = reclaim
            .iter()
            .filter(|digest| {
                catalog
                    .records
                    .get(*digest)
                    .is_some_and(|entry| !entry.is_tombstone())
            })
            .count();
        let projected_count =
            catalog
                .live_count()
                .checked_sub(reclaimed_live)
                .ok_or(ReceiptLedgerError::Corrupt(
                    "receipt reclamation exceeds the live catalog",
                ))?;
        if projected_count >= MAX_LIVE_RECEIPTS {
            let capacity_candidate = catalog
                .records
                .iter()
                .filter(|(digest, _)| !reclaim.contains(digest))
                .filter(|(_, entry)| {
                    entry_is_expired_cancel_reserved(entry, observed_at_epoch_ms)
                        || entry_is_expired_direct_terminal(entry, observed_at_epoch_ms)
                })
                .map(|(digest, _)| digest.clone())
                .min_by(|left, right| left.as_str().cmp(right.as_str()));
            let Some(capacity_candidate) = capacity_candidate else {
                return self.reject_before_mutation(
                    catalog,
                    deadline,
                    ReceiptLedgerError::CapacityExceeded,
                );
            };
            reclaim.push(capacity_candidate);
            reclaim.sort_unstable_by(|left, right| left.as_str().cmp(right.as_str()));
        }

        for digest in reclaim {
            match catalog
                .records
                .get(&digest)
                .map(|entry| &entry.record.lifecycle)
            {
                Some(StoredActiveLifecycleV1::CancelReserved { .. }) => {
                    self.reclaim_expired_cancel_reservation_under_writer_lock(
                        catalog,
                        &digest,
                        observed_at_epoch_ms,
                        deadline,
                    )?;
                }
                Some(StoredActiveLifecycleV1::AcknowledgedTombstone { .. }) => {
                    self.reclaim_expired_tombstone_under_writer_lock(
                        catalog,
                        &digest,
                        observed_at_epoch_ms,
                        deadline,
                    )?;
                }
                Some(StoredActiveLifecycleV1::DirectTerminalUnacked { .. }) => {
                    self.reclaim_expired_direct_terminal_under_writer_lock(
                        catalog,
                        &digest,
                        observed_at_epoch_ms,
                        deadline,
                    )?;
                }
                Some(_) => {
                    return latch_catalog_error(
                        catalog,
                        ReceiptLedgerError::Corrupt(
                            "identity reclamation candidate changed lifecycle",
                        ),
                    )
                }
                None => {
                    return latch_catalog_error(
                        catalog,
                        ReceiptLedgerError::Corrupt("identity reclamation candidate disappeared"),
                    )
                }
            }
        }
        Ok(())
    }

    fn reclaim_expired_cancel_reservation_under_writer_lock(
        &self,
        catalog: &mut ReceiptCatalog,
        key_digest: &ReceiptKeyDigest,
        observed_at_epoch_ms: u64,
        deadline: Instant,
    ) -> Result<(), ReceiptLedgerError> {
        check_deadline(deadline)?;
        let expected =
            catalog
                .records
                .get(key_digest)
                .cloned()
                .ok_or(ReceiptLedgerError::Corrupt(
                    "expired receipt disappeared while the writer lock was held",
                ))?;
        let persisted = self
            .read_entry_under_writer_lock(catalog, key_digest, Some(deadline))?
            .ok_or(ReceiptLedgerError::Corrupt(
                "catalogued expired receipt row is missing",
            ))?;
        if persisted != expected {
            return latch_catalog_error(
                catalog,
                ReceiptLedgerError::Corrupt("catalogued expired receipt row changed on disk"),
            );
        }
        match persisted.state() {
            Ok(ReceiptState::CancelReserved(receipt))
                if observed_at_epoch_ms >= receipt.expires_at_epoch_ms() => {}
            Ok(ReceiptState::CancelReserved(_)) => {
                return latch_catalog_error(
                    catalog,
                    ReceiptLedgerError::Corrupt("selected cancellation reservation is not expired"),
                )
            }
            Ok(_) => {
                return latch_catalog_error(
                    catalog,
                    ReceiptLedgerError::Corrupt(
                        "expired cancellation candidate changed lifecycle under writer lock",
                    ),
                )
            }
            Err(error) => return latch_catalog_error(catalog, error),
        }
        self.expire_cancel_reserved_entry_under_writer_lock(
            catalog,
            persisted,
            observed_at_epoch_ms,
            deadline,
        )
    }

    fn expire_cancel_reserved_entry_under_writer_lock(
        &self,
        catalog: &mut ReceiptCatalog,
        persisted: CatalogEntry,
        observed_at_epoch_ms: u64,
        deadline: Instant,
    ) -> Result<(), ReceiptLedgerError> {
        let key_digest = persisted.record.key_digest.clone();
        let expected_version = persisted.record.record_version;

        let next_record_version = match expected_version.checked_next() {
            Some(version) => version,
            None => {
                return latch_catalog_error(
                    catalog,
                    ReceiptLedgerError::Corrupt("receipt record version exhausted u64"),
                )
            }
        };
        let generation = latch_catalog_result(catalog, self.generation_under_writer_lock())?;
        let mutation_sequence = match generation.checked_add(1) {
            Some(sequence) => sequence,
            None => {
                return latch_catalog_error(
                    catalog,
                    ReceiptLedgerError::Corrupt("receipt generation exhausted u64"),
                )
            }
        };
        let record = match build_expired_deletion_record(
            &persisted,
            observed_at_epoch_ms,
            mutation_sequence,
            next_record_version,
        ) {
            Ok(record) => record,
            Err(error) => return latch_catalog_error(catalog, error),
        };
        let (record, encoded) =
            match serialize_reserved_record(record, MAX_CANCEL_RESERVED_RECORD_BYTES) {
                Ok(serialized) => serialized,
                Err(error) => return self.reject_before_mutation(catalog, deadline, error),
            };
        if let Err(error) = validate_catalog_remove(catalog, &persisted) {
            return latch_catalog_error(catalog, error);
        }
        if let Err(error) = self.publish_replacement_record(&record, &encoded, deadline, || {
            commit_catalog_remove(catalog, &persisted);
        }) {
            if !matches!(error, ReceiptLedgerError::DeadlineExceeded) {
                catalog.unavailable = true;
            }
            return Err(error);
        }
        if let Err(error) =
            self.publish_generation(mutation_sequence, Some(&key_digest), Some(deadline))
        {
            catalog.unavailable = true;
            return Err(error);
        }
        if let Err(error) = self.remove_expired_deletion_witness(&key_digest, &encoded, deadline) {
            catalog.unavailable = true;
            return Err(error);
        }
        if check_deadline(deadline).is_err() || self.verify_named_authority().is_err() {
            catalog.unavailable = true;
            return Err(ReceiptLedgerError::CommitUncertain {
                receipt_key_digest: key_digest,
            });
        }
        Ok(())
    }

    pub(crate) fn reserve(
        &self,
        key: ReceiptKey,
        original_cutoff: OriginalCutoffDescriptor,
        deadline: Instant,
    ) -> Result<ReserveOutcome, ReceiptLedgerError> {
        check_deadline(deadline)?;
        let key_digest = receipt_key_digest(&key);
        let mut catalog = self
            .writer
            .lock()
            .map_err(|_| ReceiptLedgerError::Corrupt("receipt catalog lock was poisoned"))?;
        #[cfg(test)]
        run_after_reserve_catalog_lock_hook_for_test();
        if catalog.unavailable {
            return Err(ReceiptLedgerError::StoreUnavailable);
        }
        check_deadline(deadline)?;
        latch_catalog_result(&mut catalog, self.verify_named_authority())?;

        if let Some(existing) = catalog.records.get(&key_digest) {
            if existing.record.key != key {
                return self.reject_before_mutation(
                    &mut catalog,
                    deadline,
                    ReceiptLedgerError::ReceiptDigestCollision,
                );
            }
            let persisted = self
                .read_entry_under_writer_lock(&mut catalog, &key_digest, Some(deadline))?
                .ok_or(ReceiptLedgerError::Corrupt(
                    "catalogued receipt row is missing",
                ))?;
            let state = match persisted.state() {
                Ok(state) => state,
                Err(error) => return latch_catalog_error(&mut catalog, error),
            };
            match state {
                ReceiptState::CancelReserved(_) => {
                    return self.convert_cancel_reserved_to_submit(
                        &mut catalog,
                        persisted,
                        key,
                        original_cutoff,
                        deadline,
                    )
                }
                ReceiptState::AcknowledgedTombstone(receipt)
                    if original_cutoff.accepted_epoch_ms() >= receipt.expires_at_epoch_ms() =>
                {
                    self.reclaim_expired_tombstone_under_writer_lock(
                        &mut catalog,
                        &key_digest,
                        original_cutoff.accepted_epoch_ms(),
                        deadline,
                    )?;
                }
                ReceiptState::DirectTerminalUnacked(receipt)
                    if receipt
                        .terminal_epoch_ms()
                        .checked_add(DIRECT_TERMINAL_RETENTION_MS)
                        .is_some_and(|expires_at_epoch_ms| {
                            original_cutoff.accepted_epoch_ms() >= expires_at_epoch_ms
                        }) =>
                {
                    self.reclaim_expired_direct_terminal_under_writer_lock(
                        &mut catalog,
                        &key_digest,
                        original_cutoff.accepted_epoch_ms(),
                        deadline,
                    )?;
                }
                state => return Ok(ReserveOutcome::ExistingExact(state)),
            }
        }

        self.prepare_new_admission_under_writer_lock(
            &mut catalog,
            &key,
            original_cutoff.accepted_epoch_ms(),
            deadline,
        )?;

        let generation = latch_catalog_result(&mut catalog, self.generation_under_writer_lock())?;
        let mutation_sequence = match generation.checked_add(1) {
            Some(sequence) => sequence,
            None => {
                return self.reject_before_mutation(
                    &mut catalog,
                    deadline,
                    ReceiptLedgerError::Corrupt("receipt generation exhausted u64"),
                )
            }
        };
        let record = build_reserved_record(
            key,
            key_digest.clone(),
            original_cutoff,
            mutation_sequence,
            ReceiptVersion::initial(),
            false,
        );
        let (record, encoded) =
            match serialize_reserved_record(record, MAX_TASK_RECORD_ENVELOPE_BYTES as u64) {
                Ok(serialized) => serialized,
                Err(error) => return self.reject_before_mutation(&mut catalog, deadline, error),
            };
        let encoded_bytes = match u64::try_from(encoded.len()) {
            Ok(encoded_bytes) => encoded_bytes,
            Err(_) => {
                return self.reject_before_mutation(
                    &mut catalog,
                    deadline,
                    ReceiptLedgerError::RecordTooLarge,
                )
            }
        };
        let next_actual_bytes = match catalog.actual_bytes.checked_add(encoded_bytes) {
            Some(next_actual_bytes) => next_actual_bytes,
            None => {
                return self.reject_before_mutation(
                    &mut catalog,
                    deadline,
                    ReceiptLedgerError::CapacityExceeded,
                )
            }
        };
        let derived_reserved_result_bytes =
            match MAX_RECEIPT_ENTITLEMENT_BYTES.checked_sub(encoded_bytes) {
                Some(bytes) => bytes,
                None => {
                    return self.reject_before_mutation(
                        &mut catalog,
                        deadline,
                        ReceiptLedgerError::RecordTooLarge,
                    )
                }
            };
        let next_reserved_bytes = match catalog
            .reserved_result_bytes
            .checked_add(derived_reserved_result_bytes)
        {
            Some(next_reserved_bytes) => next_reserved_bytes,
            None => {
                return self.reject_before_mutation(
                    &mut catalog,
                    deadline,
                    ReceiptLedgerError::CapacityExceeded,
                )
            }
        };
        if next_actual_bytes
            .checked_add(next_reserved_bytes)
            .filter(|total| *total <= MAX_LIVE_RECEIPT_BYTES)
            .is_none()
        {
            return self.reject_before_mutation(
                &mut catalog,
                deadline,
                ReceiptLedgerError::CapacityExceeded,
            );
        }

        let entry = CatalogEntry {
            record: record.clone(),
            encoded_bytes,
        };
        if let Err(error) = validate_catalog_insert(&catalog, &entry, false) {
            return self.reject_before_mutation(&mut catalog, deadline, error);
        }
        if let Err(error) = self.publish_new_record(&record, &encoded, deadline, || {
            commit_catalog_insert(&mut catalog, entry);
        }) {
            if !matches!(error, ReceiptLedgerError::DeadlineExceeded) {
                catalog.unavailable = true;
            }
            return Err(error);
        }
        if let Err(error) =
            self.publish_generation(mutation_sequence, Some(&key_digest), Some(deadline))
        {
            catalog.unavailable = true;
            return Err(error);
        }
        match self.read_active_record_bytes(&key_digest) {
            Ok(Some(committed)) if committed.as_slice() == encoded.as_slice() => {}
            Ok(Some(_)) | Ok(None) | Err(_) => {
                catalog.unavailable = true;
                return Err(ReceiptLedgerError::CommitUncertain {
                    receipt_key_digest: key_digest,
                });
            }
        }
        if check_deadline(deadline).is_err() || self.verify_named_authority().is_err() {
            catalog.unavailable = true;
            return Err(ReceiptLedgerError::CommitUncertain {
                receipt_key_digest: key_digest.clone(),
            });
        }
        let committed = match catalog.records.get(&key_digest) {
            Some(committed) if committed.record == record => committed,
            Some(_) | None => {
                catalog.unavailable = true;
                return Err(ReceiptLedgerError::CommitUncertain {
                    receipt_key_digest: key_digest,
                });
            }
        };
        Ok(ReserveOutcome::Created(committed.reservation()?))
    }

    pub(crate) fn bind_reserved_actor(
        &self,
        key: &ReceiptKey,
        expected_version: ReceiptVersion,
        bound_workspace_identity: SafeIdentityHash,
        deadline: Instant,
    ) -> Result<ReservedReceipt, ReceiptLedgerError> {
        self.transition_reserved_phase(
            key,
            expected_version,
            ReservedPhaseTransition::BindActor(bound_workspace_identity),
            deadline,
        )
    }

    pub(crate) fn mark_reserved_begun(
        &self,
        key: &ReceiptKey,
        expected_version: ReceiptVersion,
        deadline: Instant,
    ) -> Result<ReservedReceipt, ReceiptLedgerError> {
        self.transition_reserved_phase(
            key,
            expected_version,
            ReservedPhaseTransition::MarkBegun,
            deadline,
        )
    }

    pub(crate) fn promise_task_unbound(
        &self,
        key: &ReceiptKey,
        expected_version: ReceiptVersion,
        created_at_epoch_ms: u64,
        ttl_ms: u64,
        poll_interval_ms: u64,
        deadline: Instant,
    ) -> Result<TaskPromisedUnboundReceipt, ReceiptLedgerError> {
        check_deadline(deadline)?;
        let task = ReceiptTaskProjection::new(
            key.reserved_task_id(),
            key.invocation_id(),
            created_at_epoch_ms,
            created_at_epoch_ms,
            ttl_ms,
            poll_interval_ms,
            1,
        )?;
        let key_digest = receipt_key_digest(key);
        let mut catalog = self
            .writer
            .lock()
            .map_err(|_| ReceiptLedgerError::Corrupt("receipt catalog lock was poisoned"))?;
        if catalog.unavailable {
            return Err(ReceiptLedgerError::StoreUnavailable);
        }
        latch_catalog_result(&mut catalog, self.verify_named_authority())?;
        let expected = catalog
            .records
            .get(&key_digest)
            .cloned()
            .ok_or(ReceiptLedgerError::ReceiptNotFound)?;
        if &expected.record.key != key {
            return latch_catalog_error(&mut catalog, ReceiptLedgerError::ReceiptDigestCollision);
        }
        let persisted = self
            .read_entry_under_writer_lock(&mut catalog, &key_digest, Some(deadline))?
            .ok_or(ReceiptLedgerError::Corrupt(
                "catalogued receipt row is missing",
            ))?;
        if persisted != expected {
            return latch_catalog_error(
                &mut catalog,
                ReceiptLedgerError::Corrupt("catalogued receipt row changed on disk"),
            );
        }
        if persisted.record.record_version != expected_version {
            return Err(ReceiptLedgerError::ReceiptVersionMismatch {
                expected: expected_version,
                actual: persisted.record.record_version,
            });
        }
        let reserved = persisted.reservation()?;
        if !matches!(reserved.phase(), ReservedPhase::Unbound) {
            return Err(ReceiptLedgerError::ReceiptRowPresentUnsupported);
        }
        let next_record_version =
            expected_version
                .checked_next()
                .ok_or(ReceiptLedgerError::Corrupt(
                    "receipt record version exhausted u64",
                ))?;
        let generation = latch_catalog_result(&mut catalog, self.generation_under_writer_lock())?;
        let mutation_sequence = generation
            .checked_add(1)
            .ok_or(ReceiptLedgerError::Corrupt(
                "receipt generation exhausted u64",
            ))?;
        let record = StoredActiveReceiptV1 {
            schema_version: RECEIPT_RECORD_SCHEMA_VERSION,
            mutation_sequence,
            record_version: next_record_version,
            key: key.clone(),
            key_digest: key_digest.clone(),
            lifecycle: StoredActiveLifecycleV1::TaskPromisedUnbound {
                original_cutoff: *reserved.original_cutoff(),
                task_id: task.task_id(),
                invocation_id: task.invocation_id(),
                created_at_epoch_ms: task.created_at_epoch_ms(),
                updated_at_epoch_ms: task.updated_at_epoch_ms(),
                ttl_ms: task.ttl_ms(),
                poll_interval_ms: task.poll_interval_ms(),
                task_version: task.version(),
                cancel_requested: reserved.cancel_requested(),
            },
        };
        let (record, encoded) =
            serialize_reserved_record(record, MAX_TASK_RECORD_ENVELOPE_BYTES as u64)?;
        let replacement = CatalogEntry {
            record: record.clone(),
            encoded_bytes: u64::try_from(encoded.len())
                .map_err(|_| ReceiptLedgerError::RecordTooLarge)?,
        };
        validate_catalog_replace(&catalog, &expected, &replacement)?;
        if let Err(error) = self.publish_replacement_record(&record, &encoded, deadline, || {
            commit_catalog_replace(&mut catalog, replacement);
        }) {
            if !matches!(error, ReceiptLedgerError::DeadlineExceeded) {
                catalog.unavailable = true;
            }
            return Err(error);
        }
        if let Err(error) =
            self.publish_generation(mutation_sequence, Some(&key_digest), Some(deadline))
        {
            catalog.unavailable = true;
            return Err(error);
        }
        match self.read_active_record_bytes(&key_digest) {
            Ok(Some(committed)) if committed == encoded => {}
            Ok(Some(_)) | Ok(None) | Err(_) => {
                catalog.unavailable = true;
                return Err(ReceiptLedgerError::CommitUncertain {
                    receipt_key_digest: key_digest,
                });
            }
        }
        let committed =
            catalog
                .records
                .get(&key_digest)
                .ok_or(ReceiptLedgerError::CommitUncertain {
                    receipt_key_digest: key_digest.clone(),
                })?;
        match committed.state()? {
            ReceiptState::TaskPromisedUnbound(receipt) => Ok(receipt),
            _ => Err(ReceiptLedgerError::CommitUncertain {
                receipt_key_digest: key_digest,
            }),
        }
    }

    pub(crate) fn bind_promised_task_actor(
        &self,
        key: &ReceiptKey,
        expected_version: ReceiptVersion,
        workspace_identity_hash: SafeIdentityHash,
        deadline: Instant,
    ) -> Result<TaskPromisedActorBoundReceipt, ReceiptLedgerError> {
        check_deadline(deadline)?;
        let key_digest = receipt_key_digest(key);
        let mut catalog = self
            .writer
            .lock()
            .map_err(|_| ReceiptLedgerError::Corrupt("receipt catalog lock was poisoned"))?;
        if catalog.unavailable {
            return Err(ReceiptLedgerError::StoreUnavailable);
        }
        latch_catalog_result(&mut catalog, self.verify_named_authority())?;
        let expected = catalog
            .records
            .get(&key_digest)
            .cloned()
            .ok_or(ReceiptLedgerError::ReceiptNotFound)?;
        if &expected.record.key != key {
            return latch_catalog_error(&mut catalog, ReceiptLedgerError::ReceiptDigestCollision);
        }
        let persisted = self
            .read_entry_under_writer_lock(&mut catalog, &key_digest, Some(deadline))?
            .ok_or(ReceiptLedgerError::Corrupt(
                "catalogued receipt row is missing",
            ))?;
        if persisted != expected {
            return latch_catalog_error(
                &mut catalog,
                ReceiptLedgerError::Corrupt("catalogued receipt row changed on disk"),
            );
        }
        if persisted.record.record_version != expected_version {
            return Err(ReceiptLedgerError::ReceiptVersionMismatch {
                expected: expected_version,
                actual: persisted.record.record_version,
            });
        }
        let original_cutoff = match &persisted.record.lifecycle {
            StoredActiveLifecycleV1::TaskPromisedUnbound {
                original_cutoff, ..
            } => *original_cutoff,
            _ => return Err(ReceiptLedgerError::ReceiptRowPresentUnsupported),
        };
        let promised = match persisted.state()? {
            ReceiptState::TaskPromisedUnbound(promised) => promised,
            _ => return Err(ReceiptLedgerError::ReceiptRowPresentUnsupported),
        };
        let link = TaskLinkReference::new(
            key_digest.clone(),
            promised.task().task_id(),
            promised.task().invocation_id(),
            workspace_identity_hash.clone(),
        );
        let next_record_version =
            expected_version
                .checked_next()
                .ok_or(ReceiptLedgerError::Corrupt(
                    "receipt record version exhausted u64",
                ))?;
        let generation = latch_catalog_result(&mut catalog, self.generation_under_writer_lock())?;
        let mutation_sequence = generation
            .checked_add(1)
            .ok_or(ReceiptLedgerError::Corrupt(
                "receipt generation exhausted u64",
            ))?;
        let task = promised.task();
        let record = StoredActiveReceiptV1 {
            schema_version: RECEIPT_RECORD_SCHEMA_VERSION,
            mutation_sequence,
            record_version: next_record_version,
            key: key.clone(),
            key_digest: key_digest.clone(),
            lifecycle: StoredActiveLifecycleV1::TaskPromisedActorBound {
                original_cutoff,
                task_id: task.task_id(),
                invocation_id: task.invocation_id(),
                created_at_epoch_ms: task.created_at_epoch_ms(),
                updated_at_epoch_ms: task.updated_at_epoch_ms(),
                ttl_ms: task.ttl_ms(),
                poll_interval_ms: task.poll_interval_ms(),
                task_version: task.version(),
                workspace_identity_hash,
                task_link_digest: link.digest().clone(),
                cancel_requested: promised.cancel_requested(),
            },
        };
        let (record, encoded) =
            serialize_reserved_record(record, MAX_TASK_RECORD_ENVELOPE_BYTES as u64)?;
        let replacement = CatalogEntry {
            record: record.clone(),
            encoded_bytes: u64::try_from(encoded.len())
                .map_err(|_| ReceiptLedgerError::RecordTooLarge)?,
        };
        validate_catalog_replace(&catalog, &expected, &replacement)?;
        if let Err(error) = self.publish_replacement_record(&record, &encoded, deadline, || {
            commit_catalog_replace(&mut catalog, replacement);
        }) {
            if !matches!(error, ReceiptLedgerError::DeadlineExceeded) {
                catalog.unavailable = true;
            }
            return Err(error);
        }
        if let Err(error) =
            self.publish_generation(mutation_sequence, Some(&key_digest), Some(deadline))
        {
            catalog.unavailable = true;
            return Err(error);
        }
        match self.read_active_record_bytes(&key_digest) {
            Ok(Some(committed)) if committed == encoded => {}
            Ok(Some(_)) | Ok(None) | Err(_) => {
                catalog.unavailable = true;
                return Err(ReceiptLedgerError::CommitUncertain {
                    receipt_key_digest: key_digest,
                });
            }
        }
        let committed =
            catalog
                .records
                .get(&key_digest)
                .ok_or(ReceiptLedgerError::CommitUncertain {
                    receipt_key_digest: key_digest.clone(),
                })?;
        match committed.state()? {
            ReceiptState::TaskPromisedActorBound(receipt) => Ok(receipt),
            _ => Err(ReceiptLedgerError::CommitUncertain {
                receipt_key_digest: key_digest,
            }),
        }
    }

    pub(crate) fn begin_bound_task_handoff(
        &self,
        key: &ReceiptKey,
        expected_version: ReceiptVersion,
        created_at_epoch_ms: u64,
        ttl_ms: u64,
        poll_interval_ms: u64,
        deadline: Instant,
    ) -> Result<TaskHandoffActorBoundReceipt, ReceiptLedgerError> {
        check_deadline(deadline)?;
        let task = ReceiptTaskProjection::new(
            key.reserved_task_id(),
            key.invocation_id(),
            created_at_epoch_ms,
            created_at_epoch_ms,
            ttl_ms,
            poll_interval_ms,
            1,
        )?;
        let key_digest = receipt_key_digest(key);
        let mut catalog = self
            .writer
            .lock()
            .map_err(|_| ReceiptLedgerError::Corrupt("receipt catalog lock was poisoned"))?;
        if catalog.unavailable {
            return Err(ReceiptLedgerError::StoreUnavailable);
        }
        latch_catalog_result(&mut catalog, self.verify_named_authority())?;
        let expected = catalog
            .records
            .get(&key_digest)
            .cloned()
            .ok_or(ReceiptLedgerError::ReceiptNotFound)?;
        if &expected.record.key != key {
            return latch_catalog_error(&mut catalog, ReceiptLedgerError::ReceiptDigestCollision);
        }
        let persisted = self
            .read_entry_under_writer_lock(&mut catalog, &key_digest, Some(deadline))?
            .ok_or(ReceiptLedgerError::Corrupt(
                "catalogued receipt row is missing",
            ))?;
        if persisted != expected {
            return latch_catalog_error(
                &mut catalog,
                ReceiptLedgerError::Corrupt("catalogued receipt row changed on disk"),
            );
        }
        if persisted.record.record_version != expected_version {
            return Err(ReceiptLedgerError::ReceiptVersionMismatch {
                expected: expected_version,
                actual: persisted.record.record_version,
            });
        }
        let (original_cutoff, workspace_identity_hash, phase, cancel_requested) =
            match &persisted.record.lifecycle {
                StoredActiveLifecycleV1::ReservedActorBound {
                    original_cutoff,
                    bound_workspace_identity,
                    cancel_requested,
                    ..
                } => (
                    *original_cutoff,
                    bound_workspace_identity.clone(),
                    AttemptPhase::NotBegun,
                    *cancel_requested,
                ),
                StoredActiveLifecycleV1::ReservedBegun {
                    original_cutoff,
                    bound_workspace_identity,
                    cancel_requested,
                    ..
                } => (
                    *original_cutoff,
                    bound_workspace_identity.clone(),
                    AttemptPhase::Begun,
                    *cancel_requested,
                ),
                StoredActiveLifecycleV1::TaskPromisedActorBound {
                    original_cutoff,
                    workspace_identity_hash,
                    cancel_requested,
                    ..
                } => (
                    *original_cutoff,
                    workspace_identity_hash.clone(),
                    AttemptPhase::NotBegun,
                    *cancel_requested,
                ),
                _ => return Err(ReceiptLedgerError::ReceiptRowPresentUnsupported),
            };
        let link = TaskLinkReference::new(
            key_digest.clone(),
            task.task_id(),
            task.invocation_id(),
            workspace_identity_hash.clone(),
        );
        let next_record_version =
            expected_version
                .checked_next()
                .ok_or(ReceiptLedgerError::Corrupt(
                    "receipt record version exhausted u64",
                ))?;
        let generation = latch_catalog_result(&mut catalog, self.generation_under_writer_lock())?;
        let mutation_sequence = generation
            .checked_add(1)
            .ok_or(ReceiptLedgerError::Corrupt(
                "receipt generation exhausted u64",
            ))?;
        let record = StoredActiveReceiptV1 {
            schema_version: RECEIPT_RECORD_SCHEMA_VERSION,
            mutation_sequence,
            record_version: next_record_version,
            key: key.clone(),
            key_digest: key_digest.clone(),
            lifecycle: StoredActiveLifecycleV1::TaskHandoffActorBound {
                original_cutoff,
                task_id: task.task_id(),
                invocation_id: task.invocation_id(),
                created_at_epoch_ms: task.created_at_epoch_ms(),
                updated_at_epoch_ms: task.updated_at_epoch_ms(),
                ttl_ms: task.ttl_ms(),
                poll_interval_ms: task.poll_interval_ms(),
                task_version: task.version(),
                workspace_identity_hash,
                task_link_digest: link.digest().clone(),
                phase,
                cancel_requested,
                terminal_stage: StoredHandoffTerminalStageV1::NoTerminal,
            },
        };
        let (record, encoded) =
            serialize_reserved_record(record, MAX_TASK_RECORD_ENVELOPE_BYTES as u64)?;
        let replacement = CatalogEntry {
            record: record.clone(),
            encoded_bytes: u64::try_from(encoded.len())
                .map_err(|_| ReceiptLedgerError::RecordTooLarge)?,
        };
        validate_catalog_replace(&catalog, &expected, &replacement)?;
        if let Err(error) = self.publish_replacement_record(&record, &encoded, deadline, || {
            commit_catalog_replace(&mut catalog, replacement);
        }) {
            if !matches!(error, ReceiptLedgerError::DeadlineExceeded) {
                catalog.unavailable = true;
            }
            return Err(error);
        }
        if let Err(error) =
            self.publish_generation(mutation_sequence, Some(&key_digest), Some(deadline))
        {
            catalog.unavailable = true;
            return Err(error);
        }
        match self.read_active_record_bytes(&key_digest) {
            Ok(Some(committed)) if committed == encoded => {}
            Ok(Some(_)) | Ok(None) | Err(_) => {
                catalog.unavailable = true;
                return Err(ReceiptLedgerError::CommitUncertain {
                    receipt_key_digest: key_digest,
                });
            }
        }
        let committed =
            catalog
                .records
                .get(&key_digest)
                .ok_or(ReceiptLedgerError::CommitUncertain {
                    receipt_key_digest: key_digest.clone(),
                })?;
        match committed.state()? {
            ReceiptState::TaskHandoffActorBound(receipt) => Ok(receipt),
            _ => Err(ReceiptLedgerError::CommitUncertain {
                receipt_key_digest: key_digest,
            }),
        }
    }

    pub(crate) fn complete_bound_task_handoff(
        &self,
        key: &ReceiptKey,
        expected_version: ReceiptVersion,
        confirmed_task_bound: TaskBoundReceipt,
        deadline: Instant,
    ) -> Result<TaskBoundReceipt, ReceiptLedgerError> {
        check_deadline(deadline)?;
        let key_digest = receipt_key_digest(key);
        let mut catalog = self
            .writer
            .lock()
            .map_err(|_| ReceiptLedgerError::Corrupt("receipt catalog lock was poisoned"))?;
        if catalog.unavailable {
            return Err(ReceiptLedgerError::StoreUnavailable);
        }
        check_deadline(deadline)?;
        latch_catalog_result(&mut catalog, self.verify_named_authority())?;

        let expected = match catalog.records.get(&key_digest).cloned() {
            Some(expected) => expected,
            None => {
                let error = if catalog.invocation_index.contains_key(&key.invocation_id()) {
                    ReceiptLedgerError::InvocationIdentityMismatch
                } else if catalog
                    .reserved_task_index
                    .contains_key(&key.reserved_task_id())
                {
                    ReceiptLedgerError::ReservedTaskIdentityMismatch
                } else {
                    ReceiptLedgerError::ReceiptNotFound
                };
                return self.reject_before_mutation(&mut catalog, deadline, error);
            }
        };
        if &expected.record.key != key {
            return self.reject_before_mutation(
                &mut catalog,
                deadline,
                ReceiptLedgerError::ReceiptDigestCollision,
            );
        }
        let persisted = self
            .read_entry_under_writer_lock(&mut catalog, &key_digest, Some(deadline))?
            .ok_or(ReceiptLedgerError::Corrupt(
                "catalogued receipt row is missing",
            ))?;
        if persisted != expected {
            return latch_catalog_error(
                &mut catalog,
                ReceiptLedgerError::Corrupt("catalogued receipt row changed on disk"),
            );
        }
        if persisted.record.record_version != expected_version {
            return Err(ReceiptLedgerError::ReceiptVersionMismatch {
                expected: expected_version,
                actual: persisted.record.record_version,
            });
        }
        let (expected_link, expected_task, expected_phase, expected_cancel_requested) =
            match persisted.state() {
                Ok(ReceiptState::TaskPromisedActorBound(promised)) => (
                    promised.link().clone(),
                    promised.task().clone(),
                    AttemptPhase::NotBegun,
                    promised.cancel_requested(),
                ),
                Ok(ReceiptState::TaskHandoffActorBound(handoff)) => (
                    handoff.link().clone(),
                    handoff.task().clone(),
                    handoff.phase(),
                    handoff.cancel_requested(),
                ),
                Ok(_) => return Err(ReceiptLedgerError::ReceiptRowPresentUnsupported),
                Err(error) => return latch_catalog_error(&mut catalog, error),
            };
        let confirmed_task = confirmed_task_bound.task();
        let task_matches = confirmed_task.task_id() == expected_task.task_id()
            && confirmed_task.invocation_id() == expected_task.invocation_id()
            && confirmed_task.created_at_epoch_ms() == expected_task.created_at_epoch_ms()
            && confirmed_task.ttl_ms() == expected_task.ttl_ms()
            && confirmed_task.poll_interval_ms() == expected_task.poll_interval_ms()
            && if expected_cancel_requested {
                (expected_phase == AttemptPhase::Begun && confirmed_task == &expected_task)
                    || (expected_task
                        .version()
                        .checked_add(1)
                        .is_some_and(|version| confirmed_task.version() == version)
                        && confirmed_task.updated_at_epoch_ms()
                            >= expected_task.updated_at_epoch_ms())
            } else {
                confirmed_task == &expected_task
            };
        if confirmed_task_bound.key() != key
            || confirmed_task_bound.key_digest() != &key_digest
            || confirmed_task_bound.link() != &expected_link
            || !task_matches
            || confirmed_task_bound.phase() != expected_phase
        {
            return self.reject_before_mutation(
                &mut catalog,
                deadline,
                ReceiptLedgerError::TaskBoundMismatch,
            );
        }

        let generation = latch_catalog_result(&mut catalog, self.generation_under_writer_lock())?;
        let mutation_sequence = generation
            .checked_add(1)
            .ok_or(ReceiptLedgerError::Corrupt(
                "receipt generation exhausted u64",
            ))?;
        let record = match build_completed_task_handoff_deletion_record(
            &persisted,
            &confirmed_task_bound,
            mutation_sequence,
        ) {
            Ok(record) => record,
            Err(error) => return latch_catalog_error(&mut catalog, error),
        };
        let (record, encoded) =
            match serialize_reserved_record(record, MAX_COMPLETED_TASK_HANDOFF_WITNESS_BYTES) {
                Ok(serialized) => serialized,
                Err(error) => return self.reject_before_mutation(&mut catalog, deadline, error),
            };
        if let Err(error) = validate_catalog_remove(&catalog, &persisted) {
            return latch_catalog_error(&mut catalog, error);
        }
        if let Err(error) = self.publish_replacement_record(&record, &encoded, deadline, || {
            commit_catalog_remove(&mut catalog, &persisted);
        }) {
            if !matches!(error, ReceiptLedgerError::DeadlineExceeded) {
                catalog.unavailable = true;
            }
            return Err(error);
        }
        if let Err(error) =
            self.publish_generation(mutation_sequence, Some(&key_digest), Some(deadline))
        {
            catalog.unavailable = true;
            return Err(error);
        }
        if let Err(error) = self.remove_expired_deletion_witness(&key_digest, &encoded, deadline) {
            catalog.unavailable = true;
            return Err(error);
        }
        if check_deadline(deadline).is_err() || self.verify_named_authority().is_err() {
            catalog.unavailable = true;
            return Err(ReceiptLedgerError::CommitUncertain {
                receipt_key_digest: key_digest,
            });
        }
        Ok(confirmed_task_bound)
    }

    pub(crate) fn complete_staged_task_handoff(
        &self,
        key: &ReceiptKey,
        expected_version: ReceiptVersion,
        confirmed_terminal_bound: TaskTerminalBoundReceipt,
        deadline: Instant,
    ) -> Result<TaskTerminalBoundReceipt, ReceiptLedgerError> {
        check_deadline(deadline)?;
        let key_digest = receipt_key_digest(key);
        let mut catalog = self
            .writer
            .lock()
            .map_err(|_| ReceiptLedgerError::Corrupt("receipt catalog lock was poisoned"))?;
        if catalog.unavailable {
            return Err(ReceiptLedgerError::StoreUnavailable);
        }
        latch_catalog_result(&mut catalog, self.verify_named_authority())?;
        let expected = catalog
            .records
            .get(&key_digest)
            .cloned()
            .ok_or(ReceiptLedgerError::ReceiptNotFound)?;
        if &expected.record.key != key {
            return latch_catalog_error(&mut catalog, ReceiptLedgerError::ReceiptDigestCollision);
        }
        let persisted = self
            .read_entry_under_writer_lock(&mut catalog, &key_digest, Some(deadline))?
            .ok_or(ReceiptLedgerError::Corrupt(
                "catalogued receipt row is missing",
            ))?;
        if persisted != expected {
            return latch_catalog_error(
                &mut catalog,
                ReceiptLedgerError::Corrupt("catalogued receipt row changed on disk"),
            );
        }
        if persisted.record.record_version != expected_version {
            return Err(ReceiptLedgerError::ReceiptVersionMismatch {
                expected: expected_version,
                actual: persisted.record.record_version,
            });
        }
        let handoff = match persisted.state() {
            Ok(ReceiptState::TaskHandoffActorBound(handoff)) => handoff,
            Ok(_) => return Err(ReceiptLedgerError::ReceiptRowPresentUnsupported),
            Err(error) => return latch_catalog_error(&mut catalog, error),
        };
        let (terminal_epoch_ms, terminal_digest) = match handoff.terminal_stage() {
            HandoffTerminalStage::Staged {
                terminal_epoch_ms,
                terminal,
                ..
            } => (*terminal_epoch_ms, terminal.digest()),
            HandoffTerminalStage::NoTerminal => {
                return Err(ReceiptLedgerError::ReceiptRowPresentUnsupported)
            }
        };
        let confirmed_task = confirmed_terminal_bound.task();
        if confirmed_terminal_bound.key() != key
            || confirmed_terminal_bound.key_digest() != &key_digest
            || confirmed_terminal_bound.link() != handoff.link()
            || confirmed_task.task_id() != handoff.task().task_id()
            || confirmed_task.invocation_id() != handoff.task().invocation_id()
            || confirmed_task.created_at_epoch_ms() != handoff.task().created_at_epoch_ms()
            || confirmed_task.ttl_ms() != handoff.task().ttl_ms()
            || confirmed_task.poll_interval_ms() != handoff.task().poll_interval_ms()
            || confirmed_terminal_bound.terminal_epoch_ms() != terminal_epoch_ms
            || confirmed_terminal_bound.terminal_digest() != terminal_digest
        {
            return self.reject_before_mutation(
                &mut catalog,
                deadline,
                ReceiptLedgerError::TaskBoundMismatch,
            );
        }

        let generation = latch_catalog_result(&mut catalog, self.generation_under_writer_lock())?;
        let mutation_sequence = generation
            .checked_add(1)
            .ok_or(ReceiptLedgerError::Corrupt(
                "receipt generation exhausted u64",
            ))?;
        let record = match build_completed_staged_task_handoff_deletion_record(
            &persisted,
            &confirmed_terminal_bound,
            mutation_sequence,
        ) {
            Ok(record) => record,
            Err(error) => return latch_catalog_error(&mut catalog, error),
        };
        let (record, encoded) =
            match serialize_reserved_record(record, MAX_COMPLETED_TASK_HANDOFF_WITNESS_BYTES) {
                Ok(serialized) => serialized,
                Err(error) => return self.reject_before_mutation(&mut catalog, deadline, error),
            };
        if let Err(error) = validate_catalog_remove(&catalog, &persisted) {
            return latch_catalog_error(&mut catalog, error);
        }
        if let Err(error) = self.publish_replacement_record(&record, &encoded, deadline, || {
            commit_catalog_remove(&mut catalog, &persisted);
        }) {
            if !matches!(error, ReceiptLedgerError::DeadlineExceeded) {
                catalog.unavailable = true;
            }
            return Err(error);
        }
        if let Err(error) =
            self.publish_generation(mutation_sequence, Some(&key_digest), Some(deadline))
        {
            catalog.unavailable = true;
            return Err(error);
        }
        if let Err(error) = self.remove_expired_deletion_witness(&key_digest, &encoded, deadline) {
            catalog.unavailable = true;
            return Err(error);
        }
        if check_deadline(deadline).is_err() || self.verify_named_authority().is_err() {
            catalog.unavailable = true;
            return Err(ReceiptLedgerError::CommitUncertain {
                receipt_key_digest: key_digest,
            });
        }
        Ok(confirmed_terminal_bound)
    }

    pub(crate) fn stage_bound_task_handoff_terminal(
        &self,
        key: &ReceiptKey,
        expected_version: ReceiptVersion,
        terminal_epoch_ms: u64,
        terminal: V5CanonicalTerminal,
        certificate: StagedTerminalTransferCertificate,
        deadline: Instant,
    ) -> Result<TaskHandoffActorBoundReceipt, ReceiptLedgerError> {
        check_deadline(deadline)?;
        let key_digest = receipt_key_digest(key);
        let mut catalog = self
            .writer
            .lock()
            .map_err(|_| ReceiptLedgerError::Corrupt("receipt catalog lock was poisoned"))?;
        if catalog.unavailable {
            return Err(ReceiptLedgerError::StoreUnavailable);
        }
        latch_catalog_result(&mut catalog, self.verify_named_authority())?;
        let expected = catalog
            .records
            .get(&key_digest)
            .cloned()
            .ok_or(ReceiptLedgerError::ReceiptNotFound)?;
        if &expected.record.key != key {
            return latch_catalog_error(&mut catalog, ReceiptLedgerError::ReceiptDigestCollision);
        }
        let persisted = self
            .read_entry_under_writer_lock(&mut catalog, &key_digest, Some(deadline))?
            .ok_or(ReceiptLedgerError::Corrupt(
                "catalogued receipt row is missing",
            ))?;
        if persisted != expected {
            return latch_catalog_error(
                &mut catalog,
                ReceiptLedgerError::Corrupt("catalogued receipt row changed on disk"),
            );
        }
        let handoff = match persisted.state() {
            Ok(ReceiptState::TaskHandoffActorBound(handoff)) => handoff,
            Ok(_) => return Err(ReceiptLedgerError::ReceiptRowPresentUnsupported),
            Err(error) => return latch_catalog_error(&mut catalog, error),
        };
        if let HandoffTerminalStage::Staged {
            terminal_epoch_ms: actual_epoch,
            terminal: actual_terminal,
            certificate: actual_certificate,
        } = handoff.terminal_stage()
        {
            if *actual_epoch == terminal_epoch_ms
                && actual_terminal == &terminal
                && actual_certificate.as_ref() == &certificate
            {
                return Ok(handoff);
            }
            return Err(ReceiptLedgerError::TerminalMismatch);
        }
        if handoff.record_version() != expected_version {
            return Err(ReceiptLedgerError::ReceiptVersionMismatch {
                expected: expected_version,
                actual: handoff.record_version(),
            });
        }
        if !certificate.matches_staged_terminal(
            key,
            &key_digest,
            handoff.link(),
            terminal_epoch_ms,
            &terminal,
        ) {
            return Err(ReceiptLedgerError::TerminalMismatch);
        }

        let next_record_version =
            expected_version
                .checked_next()
                .ok_or(ReceiptLedgerError::Corrupt(
                    "receipt record version exhausted u64",
                ))?;
        let generation = latch_catalog_result(&mut catalog, self.generation_under_writer_lock())?;
        let mutation_sequence = generation
            .checked_add(1)
            .ok_or(ReceiptLedgerError::Corrupt(
                "receipt generation exhausted u64",
            ))?;
        let mut lifecycle = persisted.record.lifecycle.clone();
        let StoredActiveLifecycleV1::TaskHandoffActorBound { terminal_stage, .. } = &mut lifecycle
        else {
            return Err(ReceiptLedgerError::ReceiptRowPresentUnsupported);
        };
        *terminal_stage = StoredHandoffTerminalStageV1::Staged {
            terminal_epoch_ms,
            terminal_digest: terminal.digest().clone(),
            terminal: terminal.outcome_shared(),
        };
        let record = StoredActiveReceiptV1 {
            schema_version: RECEIPT_RECORD_SCHEMA_VERSION,
            mutation_sequence,
            record_version: next_record_version,
            key: key.clone(),
            key_digest: key_digest.clone(),
            lifecycle,
        };
        let (record, encoded) = serialize_reserved_record(record, MAX_RECEIPT_ENTITLEMENT_BYTES)?;
        let replacement = CatalogEntry {
            record: record.clone(),
            encoded_bytes: u64::try_from(encoded.len())
                .map_err(|_| ReceiptLedgerError::RecordTooLarge)?,
        };
        validate_catalog_replace(&catalog, &expected, &replacement)?;
        if let Err(error) = self.publish_replacement_record(&record, &encoded, deadline, || {
            commit_catalog_replace(&mut catalog, replacement);
        }) {
            if !matches!(error, ReceiptLedgerError::DeadlineExceeded) {
                catalog.unavailable = true;
            }
            return Err(error);
        }
        if let Err(error) =
            self.publish_generation(mutation_sequence, Some(&key_digest), Some(deadline))
        {
            catalog.unavailable = true;
            return Err(error);
        }
        match self.read_active_record_bytes(&key_digest) {
            Ok(Some(committed)) if committed == encoded => {}
            Ok(Some(_)) | Ok(None) | Err(_) => {
                catalog.unavailable = true;
                return Err(ReceiptLedgerError::CommitUncertain {
                    receipt_key_digest: key_digest,
                });
            }
        }
        match catalog
            .records
            .get(&key_digest)
            .and_then(|entry| entry.state().ok())
        {
            Some(ReceiptState::TaskHandoffActorBound(committed))
                if matches!(
                    committed.terminal_stage(),
                    HandoffTerminalStage::Staged { .. }
                ) =>
            {
                Ok(committed)
            }
            _ => {
                catalog.unavailable = true;
                Err(ReceiptLedgerError::CommitUncertain {
                    receipt_key_digest: key_digest,
                })
            }
        }
    }

    pub(crate) fn retain_begun_task_after_link_capacity(
        &self,
        key: &ReceiptKey,
        expected_version: ReceiptVersion,
        proven_link_capacity: ProvenTaskLinkCapacity,
        deadline: Instant,
    ) -> Result<TaskReceiptOwnedActorBoundReceipt, ReceiptLedgerError> {
        check_deadline(deadline)?;
        let key_digest = receipt_key_digest(key);
        let mut catalog = self
            .writer
            .lock()
            .map_err(|_| ReceiptLedgerError::Corrupt("receipt catalog lock was poisoned"))?;
        if catalog.unavailable {
            return Err(ReceiptLedgerError::StoreUnavailable);
        }
        latch_catalog_result(&mut catalog, self.verify_named_authority())?;
        let expected = self
            .read_entry_under_writer_lock(&mut catalog, &key_digest, Some(deadline))?
            .ok_or(ReceiptLedgerError::ReceiptNotFound)?;
        if expected.record.key != *key {
            return latch_catalog_error(&mut catalog, ReceiptLedgerError::ReceiptDigestCollision);
        }
        if expected.record.record_version != expected_version {
            return Err(ReceiptLedgerError::ReceiptVersionMismatch {
                expected: expected_version,
                actual: expected.record.record_version,
            });
        }
        let (
            original_cutoff,
            task_id,
            invocation_id,
            created_at_epoch_ms,
            updated_at_epoch_ms,
            ttl_ms,
            poll_interval_ms,
            task_version,
            workspace_identity_hash,
            task_link_digest,
            cancel_requested,
        ) = match &expected.record.lifecycle {
            StoredActiveLifecycleV1::TaskHandoffActorBound {
                original_cutoff,
                task_id,
                invocation_id,
                created_at_epoch_ms,
                updated_at_epoch_ms,
                ttl_ms,
                poll_interval_ms,
                task_version,
                workspace_identity_hash,
                task_link_digest,
                phase: AttemptPhase::Begun,
                cancel_requested,
                ..
            } => (
                *original_cutoff,
                *task_id,
                *invocation_id,
                *created_at_epoch_ms,
                *updated_at_epoch_ms,
                *ttl_ms,
                *poll_interval_ms,
                *task_version,
                workspace_identity_hash.clone(),
                task_link_digest.clone(),
                *cancel_requested,
            ),
            _ => return Err(ReceiptLedgerError::ReceiptRowPresentUnsupported),
        };
        let record_version = expected_version
            .checked_next()
            .ok_or(ReceiptLedgerError::Corrupt(
                "receipt record version exhausted u64",
            ))?;
        let mutation_sequence =
            latch_catalog_result(&mut catalog, self.generation_under_writer_lock())?
                .checked_add(1)
                .ok_or(ReceiptLedgerError::Corrupt(
                    "receipt generation exhausted u64",
                ))?;
        let record = StoredActiveReceiptV1 {
            schema_version: RECEIPT_RECORD_SCHEMA_VERSION,
            mutation_sequence,
            record_version,
            key: key.clone(),
            key_digest: key_digest.clone(),
            lifecycle: StoredActiveLifecycleV1::TaskReceiptOwnedActorBound {
                original_cutoff,
                task_id,
                invocation_id,
                created_at_epoch_ms,
                updated_at_epoch_ms,
                ttl_ms,
                poll_interval_ms,
                task_version,
                workspace_identity_hash,
                task_link_digest,
                proven_link_capacity: (&proven_link_capacity).into(),
                cancel_requested,
            },
        };
        let (record, encoded) =
            serialize_reserved_record(record, MAX_TASK_RECORD_ENVELOPE_BYTES as u64)?;
        let replacement = CatalogEntry {
            record: record.clone(),
            encoded_bytes: u64::try_from(encoded.len())
                .map_err(|_| ReceiptLedgerError::RecordTooLarge)?,
        };
        validate_catalog_replace(&catalog, &expected, &replacement)?;
        if let Err(error) = self.publish_replacement_record(&record, &encoded, deadline, || {
            commit_catalog_replace(&mut catalog, replacement);
        }) {
            if !matches!(error, ReceiptLedgerError::DeadlineExceeded) {
                catalog.unavailable = true;
            }
            return Err(error);
        }
        if let Err(error) =
            self.publish_generation(mutation_sequence, Some(&key_digest), Some(deadline))
        {
            catalog.unavailable = true;
            return Err(error);
        }
        match self.read_active_record_bytes(&key_digest) {
            Ok(Some(committed)) if committed == encoded => {}
            Ok(Some(_)) | Ok(None) | Err(_) => {
                catalog.unavailable = true;
                return Err(ReceiptLedgerError::CommitUncertain {
                    receipt_key_digest: key_digest,
                });
            }
        }
        match catalog
            .records
            .get(&key_digest)
            .cloned()
            .and_then(|entry| entry.state().ok())
        {
            Some(ReceiptState::TaskReceiptOwnedActorBound(receipt)) => Ok(receipt),
            _ => {
                catalog.unavailable = true;
                Err(ReceiptLedgerError::CommitUncertain {
                    receipt_key_digest: key_digest,
                })
            }
        }
    }

    pub(crate) fn request_task_cancel(
        &self,
        key: &ReceiptKey,
        expected_state: TaskCancellationReceipt,
        deadline: Instant,
    ) -> Result<TaskCancellationReceipt, ReceiptLedgerError> {
        check_deadline(deadline)?;
        if expected_state.key() != key {
            return Err(ReceiptLedgerError::TaskCancellationMismatch);
        }
        let key_digest = receipt_key_digest(key);
        let mut catalog = self
            .writer
            .lock()
            .map_err(|_| ReceiptLedgerError::Corrupt("receipt catalog lock was poisoned"))?;
        if catalog.unavailable {
            return Err(ReceiptLedgerError::StoreUnavailable);
        }
        latch_catalog_result(&mut catalog, self.verify_named_authority())?;
        let expected_entry = catalog
            .records
            .get(&key_digest)
            .cloned()
            .ok_or(ReceiptLedgerError::ReceiptNotFound)?;
        if &expected_entry.record.key != key {
            return latch_catalog_error(&mut catalog, ReceiptLedgerError::ReceiptDigestCollision);
        }
        let persisted = self
            .read_entry_under_writer_lock(&mut catalog, &key_digest, Some(deadline))?
            .ok_or(ReceiptLedgerError::Corrupt(
                "catalogued receipt row is missing",
            ))?;
        if persisted != expected_entry {
            return latch_catalog_error(
                &mut catalog,
                ReceiptLedgerError::Corrupt("catalogued receipt row changed on disk"),
            );
        }
        let actual_state = match persisted.state() {
            Ok(ReceiptState::TaskPromisedUnbound(receipt)) => {
                TaskCancellationReceipt::PromisedUnbound(receipt)
            }
            Ok(ReceiptState::TaskPromisedActorBound(receipt)) => {
                TaskCancellationReceipt::PromisedActorBound(receipt)
            }
            Ok(ReceiptState::TaskHandoffActorBound(receipt)) => {
                TaskCancellationReceipt::HandoffActorBound(receipt)
            }
            Ok(_) => return Err(ReceiptLedgerError::ReceiptRowPresentUnsupported),
            Err(error) => return latch_catalog_error(&mut catalog, error),
        };
        if actual_state == expected_state && actual_state.cancel_requested() {
            return Ok(actual_state);
        }
        if actual_state.is_exact_cancel_successor_of(&expected_state) {
            return Ok(actual_state);
        }
        if actual_state != expected_state {
            return Err(ReceiptLedgerError::TaskCancellationMismatch);
        }

        let next_record_version =
            actual_state
                .record_version()
                .checked_next()
                .ok_or(ReceiptLedgerError::Corrupt(
                    "receipt record version exhausted u64",
                ))?;
        let generation = latch_catalog_result(&mut catalog, self.generation_under_writer_lock())?;
        let mutation_sequence = generation
            .checked_add(1)
            .ok_or(ReceiptLedgerError::Corrupt(
                "receipt generation exhausted u64",
            ))?;
        let mut lifecycle = persisted.record.lifecycle.clone();
        match &mut lifecycle {
            StoredActiveLifecycleV1::TaskPromisedUnbound {
                cancel_requested, ..
            }
            | StoredActiveLifecycleV1::TaskPromisedActorBound {
                cancel_requested, ..
            }
            | StoredActiveLifecycleV1::TaskHandoffActorBound {
                cancel_requested, ..
            }
            | StoredActiveLifecycleV1::TaskReceiptOwnedActorBound {
                cancel_requested, ..
            } => *cancel_requested = true,
            _ => return Err(ReceiptLedgerError::ReceiptRowPresentUnsupported),
        }
        let record = StoredActiveReceiptV1 {
            schema_version: RECEIPT_RECORD_SCHEMA_VERSION,
            mutation_sequence,
            record_version: next_record_version,
            key: key.clone(),
            key_digest: key_digest.clone(),
            lifecycle,
        };
        let (record, encoded) =
            serialize_reserved_record(record, MAX_TASK_RECORD_ENVELOPE_BYTES as u64)?;
        let replacement = CatalogEntry {
            record: record.clone(),
            encoded_bytes: u64::try_from(encoded.len())
                .map_err(|_| ReceiptLedgerError::RecordTooLarge)?,
        };
        validate_catalog_replace(&catalog, &expected_entry, &replacement)?;
        if let Err(error) = self.publish_replacement_record(&record, &encoded, deadline, || {
            commit_catalog_replace(&mut catalog, replacement);
        }) {
            if !matches!(error, ReceiptLedgerError::DeadlineExceeded) {
                catalog.unavailable = true;
            }
            return Err(error);
        }
        if let Err(error) =
            self.publish_generation(mutation_sequence, Some(&key_digest), Some(deadline))
        {
            catalog.unavailable = true;
            return Err(error);
        }
        match self.read_active_record_bytes(&key_digest) {
            Ok(Some(committed)) if committed == encoded => {}
            Ok(Some(_)) | Ok(None) | Err(_) => {
                catalog.unavailable = true;
                return Err(ReceiptLedgerError::CommitUncertain {
                    receipt_key_digest: key_digest,
                });
            }
        }
        let committed = catalog.records.get(&key_digest).cloned().ok_or(
            ReceiptLedgerError::CommitUncertain {
                receipt_key_digest: key_digest.clone(),
            },
        )?;
        let committed_state = match committed.state() {
            Ok(ReceiptState::TaskPromisedUnbound(receipt)) => {
                TaskCancellationReceipt::PromisedUnbound(receipt)
            }
            Ok(ReceiptState::TaskPromisedActorBound(receipt)) => {
                TaskCancellationReceipt::PromisedActorBound(receipt)
            }
            Ok(ReceiptState::TaskHandoffActorBound(receipt)) => {
                TaskCancellationReceipt::HandoffActorBound(receipt)
            }
            Ok(_) => {
                catalog.unavailable = true;
                return Err(ReceiptLedgerError::CommitUncertain {
                    receipt_key_digest: key_digest,
                });
            }
            Err(error) => return latch_catalog_error(&mut catalog, error),
        };
        if !committed_state.is_exact_cancel_successor_of(&expected_state) {
            catalog.unavailable = true;
            return Err(ReceiptLedgerError::CommitUncertain {
                receipt_key_digest: key_digest,
            });
        }
        Ok(committed_state)
    }

    fn transition_reserved_phase(
        &self,
        key: &ReceiptKey,
        expected_version: ReceiptVersion,
        transition: ReservedPhaseTransition,
        deadline: Instant,
    ) -> Result<ReservedReceipt, ReceiptLedgerError> {
        check_deadline(deadline)?;
        let key_digest = receipt_key_digest(key);
        let mut catalog = self
            .writer
            .lock()
            .map_err(|_| ReceiptLedgerError::Corrupt("receipt catalog lock was poisoned"))?;
        if catalog.unavailable {
            return Err(ReceiptLedgerError::StoreUnavailable);
        }
        latch_catalog_result(&mut catalog, self.verify_named_authority())?;
        let expected = catalog
            .records
            .get(&key_digest)
            .cloned()
            .ok_or(ReceiptLedgerError::ReceiptNotFound)?;
        if &expected.record.key != key {
            return latch_catalog_error(&mut catalog, ReceiptLedgerError::ReceiptDigestCollision);
        }
        let persisted = self
            .read_entry_under_writer_lock(&mut catalog, &key_digest, Some(deadline))?
            .ok_or(ReceiptLedgerError::Corrupt(
                "catalogued receipt row is missing",
            ))?;
        if persisted != expected {
            return latch_catalog_error(
                &mut catalog,
                ReceiptLedgerError::Corrupt("catalogued receipt row changed on disk"),
            );
        }
        if persisted.record.record_version != expected_version {
            return Err(ReceiptLedgerError::ReceiptVersionMismatch {
                expected: expected_version,
                actual: persisted.record.record_version,
            });
        }
        let reserved = persisted.reservation()?;
        if reserved.cancel_requested() {
            return Err(ReceiptLedgerError::ReceiptRowPresentUnsupported);
        }
        let next_phase = match (transition, reserved.phase()) {
            (ReservedPhaseTransition::BindActor(identity), ReservedPhase::Unbound) => {
                ReservedPhase::ActorBound {
                    bound_workspace_identity: identity,
                }
            }
            (
                ReservedPhaseTransition::MarkBegun,
                ReservedPhase::ActorBound {
                    bound_workspace_identity,
                },
            ) => ReservedPhase::Begun {
                bound_workspace_identity: bound_workspace_identity.clone(),
            },
            _ => return Err(ReceiptLedgerError::ReceiptRowPresentUnsupported),
        };
        let next_record_version =
            expected_version
                .checked_next()
                .ok_or(ReceiptLedgerError::Corrupt(
                    "receipt record version exhausted u64",
                ))?;
        let generation = latch_catalog_result(&mut catalog, self.generation_under_writer_lock())?;
        let mutation_sequence = generation
            .checked_add(1)
            .ok_or(ReceiptLedgerError::Corrupt(
                "receipt generation exhausted u64",
            ))?;
        let record = build_reserved_phase_record(
            &persisted,
            next_phase,
            mutation_sequence,
            next_record_version,
        )?;
        let (record, encoded) =
            serialize_reserved_record(record, MAX_TASK_RECORD_ENVELOPE_BYTES as u64)?;
        let replacement = CatalogEntry {
            record: record.clone(),
            encoded_bytes: u64::try_from(encoded.len())
                .map_err(|_| ReceiptLedgerError::RecordTooLarge)?,
        };
        validate_catalog_replace(&catalog, &expected, &replacement)?;
        if let Err(error) = self.publish_replacement_record(&record, &encoded, deadline, || {
            commit_catalog_replace(&mut catalog, replacement);
        }) {
            if !matches!(error, ReceiptLedgerError::DeadlineExceeded) {
                catalog.unavailable = true;
            }
            return Err(error);
        }
        if let Err(error) =
            self.publish_generation(mutation_sequence, Some(&key_digest), Some(deadline))
        {
            catalog.unavailable = true;
            return Err(error);
        }
        match self.read_active_record_bytes(&key_digest) {
            Ok(Some(committed)) if committed == encoded => {}
            Ok(Some(_)) | Ok(None) | Err(_) => {
                catalog.unavailable = true;
                return Err(ReceiptLedgerError::CommitUncertain {
                    receipt_key_digest: key_digest,
                });
            }
        }
        let committed =
            catalog
                .records
                .get(&key_digest)
                .ok_or(ReceiptLedgerError::CommitUncertain {
                    receipt_key_digest: key_digest,
                })?;
        committed.reservation()
    }

    fn convert_cancel_reserved_to_submit(
        &self,
        catalog: &mut ReceiptCatalog,
        expected: CatalogEntry,
        key: ReceiptKey,
        original_cutoff: OriginalCutoffDescriptor,
        deadline: Instant,
    ) -> Result<ReserveOutcome, ReceiptLedgerError> {
        let key_digest = expected.record.key_digest.clone();
        let cancel_requested = match &expected.record.lifecycle {
            StoredActiveLifecycleV1::CancelReserved {
                expires_at_epoch_ms,
                ..
            } => original_cutoff.accepted_epoch_ms() < *expires_at_epoch_ms,
            _ => {
                return latch_catalog_error(
                    catalog,
                    ReceiptLedgerError::Corrupt(
                        "cancel conversion requires a CancelReserved predecessor",
                    ),
                )
            }
        };
        let next_record_version = match expected.record.record_version.checked_next() {
            Some(version) => version,
            None => {
                return latch_catalog_error(
                    catalog,
                    ReceiptLedgerError::Corrupt("receipt record version exhausted u64"),
                )
            }
        };
        let generation = latch_catalog_result(catalog, self.generation_under_writer_lock())?;
        let mutation_sequence = match generation.checked_add(1) {
            Some(sequence) => sequence,
            None => {
                return latch_catalog_error(
                    catalog,
                    ReceiptLedgerError::Corrupt("receipt generation exhausted u64"),
                )
            }
        };
        let record = build_reserved_record(
            key,
            key_digest.clone(),
            original_cutoff,
            mutation_sequence,
            next_record_version,
            cancel_requested,
        );
        let (record, encoded) =
            match serialize_reserved_record(record, MAX_TASK_RECORD_ENVELOPE_BYTES as u64) {
                Ok(serialized) => serialized,
                Err(error) => return self.reject_before_mutation(catalog, deadline, error),
            };
        let encoded_bytes = match u64::try_from(encoded.len()) {
            Ok(encoded_bytes) => encoded_bytes,
            Err(_) => {
                return self.reject_before_mutation(
                    catalog,
                    deadline,
                    ReceiptLedgerError::RecordTooLarge,
                )
            }
        };
        let replacement = CatalogEntry {
            record: record.clone(),
            encoded_bytes,
        };
        if let Err(error) = validate_catalog_replace(catalog, &expected, &replacement) {
            if error.requires_reopen() {
                return latch_catalog_error(catalog, error);
            }
            return self.reject_before_mutation(catalog, deadline, error);
        }
        if let Err(error) = self.publish_replacement_record(&record, &encoded, deadline, || {
            commit_catalog_replace(catalog, replacement);
        }) {
            if !matches!(error, ReceiptLedgerError::DeadlineExceeded) {
                catalog.unavailable = true;
            }
            return Err(error);
        }
        if let Err(error) =
            self.publish_generation(mutation_sequence, Some(&key_digest), Some(deadline))
        {
            catalog.unavailable = true;
            return Err(error);
        }
        match self.read_active_record_bytes(&key_digest) {
            Ok(Some(committed)) if committed.as_slice() == encoded.as_slice() => {}
            Ok(Some(_)) | Ok(None) | Err(_) => {
                catalog.unavailable = true;
                return Err(ReceiptLedgerError::CommitUncertain {
                    receipt_key_digest: key_digest,
                });
            }
        }
        if check_deadline(deadline).is_err() || self.verify_named_authority().is_err() {
            catalog.unavailable = true;
            return Err(ReceiptLedgerError::CommitUncertain {
                receipt_key_digest: key_digest.clone(),
            });
        }
        let committed = match catalog.records.get(&key_digest).cloned() {
            Some(committed) if committed.record == record => committed,
            Some(_) | None => {
                catalog.unavailable = true;
                return Err(ReceiptLedgerError::CommitUncertain {
                    receipt_key_digest: key_digest,
                });
            }
        };
        match committed.reservation() {
            Ok(reservation) => Ok(ReserveOutcome::Created(reservation)),
            Err(_) => {
                catalog.unavailable = true;
                Err(ReceiptLedgerError::CommitUncertain {
                    receipt_key_digest: key_digest,
                })
            }
        }
    }

    pub(crate) fn publish_receipt_backed_task_terminal(
        &self,
        key: &ReceiptKey,
        expected_state: TaskCancellationReceipt,
        terminal_epoch_ms: u64,
        terminal: V5CanonicalTerminal,
        deadline: Instant,
    ) -> Result<TaskTerminalReceiptBackedReceipt, ReceiptLedgerError> {
        check_deadline(deadline)?;
        if expected_state.key() != key {
            return Err(ReceiptLedgerError::TaskBoundMismatch);
        }
        if let TaskCancellationReceipt::HandoffActorBound(receipt) = &expected_state {
            match receipt.terminal_stage() {
                HandoffTerminalStage::NoTerminal if receipt.phase() == AttemptPhase::Begun => {
                    return Err(ReceiptLedgerError::ReceiptRowPresentUnsupported)
                }
                HandoffTerminalStage::Staged {
                    terminal_epoch_ms: staged_epoch_ms,
                    terminal: staged_terminal,
                    ..
                } if *staged_epoch_ms != terminal_epoch_ms || staged_terminal != &terminal => {
                    return Err(ReceiptLedgerError::TerminalMismatch)
                }
                HandoffTerminalStage::NoTerminal | HandoffTerminalStage::Staged { .. } => {}
            }
        }
        let expected_task = expected_state.task();
        let task_version =
            expected_task
                .version()
                .checked_add(1)
                .ok_or(ReceiptLedgerError::Corrupt(
                    "Task projection version exhausted u64",
                ))?;
        let terminal_task = ReceiptTaskProjection::new(
            expected_task.task_id(),
            expected_task.invocation_id(),
            expected_task.created_at_epoch_ms(),
            terminal_epoch_ms,
            expected_task.ttl_ms(),
            expected_task.poll_interval_ms(),
            task_version,
        )?;
        let key_digest = receipt_key_digest(key);
        let mut catalog = self
            .writer
            .lock()
            .map_err(|_| ReceiptLedgerError::Corrupt("receipt catalog lock was poisoned"))?;
        if catalog.unavailable {
            return Err(ReceiptLedgerError::StoreUnavailable);
        }
        latch_catalog_result(&mut catalog, self.verify_named_authority())?;
        let expected = self
            .read_entry_under_writer_lock(&mut catalog, &key_digest, Some(deadline))?
            .ok_or(ReceiptLedgerError::ReceiptNotFound)?;
        if expected.record.key != *key {
            return latch_catalog_error(&mut catalog, ReceiptLedgerError::ReceiptDigestCollision);
        }
        match expected.state() {
            Ok(ReceiptState::TaskTerminalReceiptBacked(receipt))
                if receipt.task() == &terminal_task
                    && receipt.terminal_epoch_ms() == terminal_epoch_ms
                    && receipt.terminal() == &terminal
                    && receipt.cancel_requested() == expected_state.cancel_requested() =>
            {
                return Ok(receipt)
            }
            Ok(actual) if actual == expected_state.clone().into_receipt_state() => {}
            Ok(_) => {
                return Err(ReceiptLedgerError::ReceiptVersionMismatch {
                    expected: expected_state.record_version(),
                    actual: expected.record.record_version,
                })
            }
            Err(error) => return latch_catalog_error(&mut catalog, error),
        }
        if expected.record.record_version != expected_state.record_version() {
            return Err(ReceiptLedgerError::ReceiptVersionMismatch {
                expected: expected_state.record_version(),
                actual: expected.record.record_version,
            });
        }
        let record_version =
            expected
                .record
                .record_version
                .checked_next()
                .ok_or(ReceiptLedgerError::Corrupt(
                    "receipt record version exhausted u64",
                ))?;
        let generation = latch_catalog_result(&mut catalog, self.generation_under_writer_lock())?;
        let mutation_sequence = generation
            .checked_add(1)
            .ok_or(ReceiptLedgerError::Corrupt(
                "receipt generation exhausted u64",
            ))?;
        let record = StoredActiveReceiptV1 {
            schema_version: RECEIPT_RECORD_SCHEMA_VERSION,
            mutation_sequence,
            record_version,
            key: key.clone(),
            key_digest: key_digest.clone(),
            lifecycle: StoredActiveLifecycleV1::TaskTerminalReceiptBacked {
                task_id: terminal_task.task_id(),
                invocation_id: terminal_task.invocation_id(),
                created_at_epoch_ms: terminal_task.created_at_epoch_ms(),
                updated_at_epoch_ms: terminal_task.updated_at_epoch_ms(),
                ttl_ms: terminal_task.ttl_ms(),
                poll_interval_ms: terminal_task.poll_interval_ms(),
                task_version: terminal_task.version(),
                terminal_epoch_ms,
                terminal_digest: terminal.digest().clone(),
                terminal: terminal.outcome_shared(),
                cancel_requested: expected_state.cancel_requested(),
            },
        };
        let (record, encoded) = serialize_reserved_record(record, MAX_RECEIPT_ENTITLEMENT_BYTES)?;
        let replacement = CatalogEntry {
            record: record.clone(),
            encoded_bytes: u64::try_from(encoded.len())
                .map_err(|_| ReceiptLedgerError::RecordTooLarge)?,
        };
        if let Err(error) = validate_catalog_replace(&catalog, &expected, &replacement) {
            if error.requires_reopen() {
                return latch_catalog_error(&mut catalog, error);
            }
            return Err(error);
        }
        if let Err(error) = self.publish_replacement_record(&record, &encoded, deadline, || {
            commit_catalog_replace(&mut catalog, replacement);
        }) {
            if !matches!(error, ReceiptLedgerError::DeadlineExceeded) {
                catalog.unavailable = true;
            }
            return Err(error);
        }
        if let Err(error) =
            self.publish_generation(mutation_sequence, Some(&key_digest), Some(deadline))
        {
            catalog.unavailable = true;
            return Err(error);
        }
        match self.read_active_record_bytes(&key_digest) {
            Ok(Some(committed)) if committed == encoded => {}
            Ok(Some(_)) | Ok(None) | Err(_) => {
                catalog.unavailable = true;
                return Err(ReceiptLedgerError::CommitUncertain {
                    receipt_key_digest: key_digest,
                });
            }
        }
        let committed = catalog.records.get(&key_digest).cloned().ok_or(
            ReceiptLedgerError::CommitUncertain {
                receipt_key_digest: key_digest,
            },
        )?;
        match committed.state() {
            Ok(ReceiptState::TaskTerminalReceiptBacked(receipt)) => Ok(receipt),
            Ok(_) | Err(_) => {
                catalog.unavailable = true;
                Err(ReceiptLedgerError::CommitUncertain {
                    receipt_key_digest: committed.record.key_digest,
                })
            }
        }
    }

    #[cfg(feature = "receipt-ledger-test-support")]
    pub(crate) fn seed_task_terminal_receipt_backed_for_test(
        &self,
        seed: ReceiptBackedTaskTerminalSeed,
        deadline: Instant,
    ) -> Result<TaskTerminalReceiptBackedReceipt, ReceiptLedgerError> {
        let ReceiptBackedTaskTerminalSeed {
            key,
            original_cutoff,
            task,
            terminal_epoch_ms,
            terminal,
            cancel_requested,
        } = seed;
        let reservation = match self.reserve(key.clone(), original_cutoff, deadline)? {
            ReserveOutcome::Created(reservation) => reservation,
            ReserveOutcome::ExistingExact(_) => {
                return Err(ReceiptLedgerError::Corrupt(
                    "test fixture receipt already exists",
                ))
            }
        };
        if task.task_id() != key.reserved_task_id()
            || task.invocation_id() != key.invocation_id()
            || task.updated_at_epoch_ms() != terminal_epoch_ms
        {
            return Err(ReceiptLedgerError::Corrupt(
                "test fixture Task projection contradicts its receipt identity",
            ));
        }

        let key_digest = receipt_key_digest(&key);
        let mut catalog = self
            .writer
            .lock()
            .map_err(|_| ReceiptLedgerError::Corrupt("receipt catalog lock was poisoned"))?;
        if catalog.unavailable {
            return Err(ReceiptLedgerError::StoreUnavailable);
        }
        check_deadline(deadline)?;
        latch_catalog_result(&mut catalog, self.verify_named_authority())?;
        let expected = self
            .read_entry_under_writer_lock(&mut catalog, &key_digest, Some(deadline))?
            .ok_or(ReceiptLedgerError::Corrupt(
                "test fixture reservation disappeared",
            ))?;
        if expected.record.record_version != reservation.record_version()
            || !matches!(expected.state()?, ReceiptState::Reserved(_))
        {
            return latch_catalog_error(
                &mut catalog,
                ReceiptLedgerError::Corrupt("test fixture terminal requires its exact reservation"),
            );
        }
        let record_version =
            expected
                .record
                .record_version
                .checked_next()
                .ok_or(ReceiptLedgerError::Corrupt(
                    "receipt record version exhausted u64",
                ))?;
        let generation = latch_catalog_result(&mut catalog, self.generation_under_writer_lock())?;
        let mutation_sequence = generation
            .checked_add(1)
            .ok_or(ReceiptLedgerError::Corrupt(
                "receipt generation exhausted u64",
            ))?;
        let record = StoredActiveReceiptV1 {
            schema_version: RECEIPT_RECORD_SCHEMA_VERSION,
            mutation_sequence,
            record_version,
            key,
            key_digest: key_digest.clone(),
            lifecycle: StoredActiveLifecycleV1::TaskTerminalReceiptBacked {
                task_id: task.task_id(),
                invocation_id: task.invocation_id(),
                created_at_epoch_ms: task.created_at_epoch_ms(),
                updated_at_epoch_ms: task.updated_at_epoch_ms(),
                ttl_ms: task.ttl_ms(),
                poll_interval_ms: task.poll_interval_ms(),
                task_version: task.version(),
                terminal_epoch_ms,
                terminal_digest: terminal.digest().clone(),
                terminal: terminal.outcome_shared(),
                cancel_requested,
            },
        };
        let (record, encoded) = serialize_reserved_record(record, MAX_RECEIPT_ENTITLEMENT_BYTES)?;
        let replacement = CatalogEntry {
            record: record.clone(),
            encoded_bytes: u64::try_from(encoded.len())
                .map_err(|_| ReceiptLedgerError::RecordTooLarge)?,
        };
        if let Err(error) = validate_catalog_replace(&catalog, &expected, &replacement) {
            if error.requires_reopen() {
                return latch_catalog_error(&mut catalog, error);
            }
            return Err(error);
        }
        if let Err(error) = self.publish_replacement_record(&record, &encoded, deadline, || {
            commit_catalog_replace(&mut catalog, replacement);
        }) {
            if !matches!(error, ReceiptLedgerError::DeadlineExceeded) {
                catalog.unavailable = true;
            }
            return Err(error);
        }
        if let Err(error) =
            self.publish_generation(mutation_sequence, Some(&key_digest), Some(deadline))
        {
            catalog.unavailable = true;
            return Err(error);
        }
        match self.read_active_record_bytes(&key_digest) {
            Ok(Some(committed)) if committed == encoded => {}
            Ok(Some(_)) | Ok(None) | Err(_) => {
                catalog.unavailable = true;
                return Err(ReceiptLedgerError::CommitUncertain {
                    receipt_key_digest: key_digest,
                });
            }
        }
        let committed = catalog.records.get(&key_digest).cloned().ok_or(
            ReceiptLedgerError::CommitUncertain {
                receipt_key_digest: key_digest,
            },
        )?;
        match committed.state()? {
            ReceiptState::TaskTerminalReceiptBacked(receipt) => Ok(receipt),
            _ => Err(ReceiptLedgerError::Corrupt(
                "test fixture terminal committed an unexpected state",
            )),
        }
    }

    #[cfg(feature = "receipt-ledger-test-support")]
    pub(crate) fn inject_identity_index_collision_for_test(
        &self,
        collide_on_invocation_id: bool,
        deadline: Instant,
    ) -> Result<(), ReceiptLedgerError> {
        check_deadline(deadline)?;
        let catalog = self
            .writer
            .lock()
            .map_err(|_| ReceiptLedgerError::Corrupt("receipt catalog lock was poisoned"))?;
        if catalog.unavailable {
            return Err(ReceiptLedgerError::StoreUnavailable);
        }
        let mut record = catalog
            .records
            .values()
            .find(|entry| !entry.is_tombstone())
            .map(|entry| entry.record.clone())
            .ok_or(ReceiptLedgerError::ReceiptNotFound)?;
        let colliding_key = ReceiptKey::new(
            if collide_on_invocation_id {
                record.key.invocation_id()
            } else {
                InvocationId::new()
            },
            if collide_on_invocation_id {
                TaskId::new()
            } else {
                record.key.reserved_task_id()
            },
            RequestIdentity::new(
                record.key.core_identity_digest().clone(),
                record.key.tool(),
                record.key.normalized_arguments_hash().clone(),
                record.key.request_scope_hash().clone(),
            ),
        );
        record.key_digest = receipt_key_digest(&colliding_key);
        record.key = colliding_key;
        let (record, encoded) = serialize_reserved_record(record, MAX_RECEIPT_ENTITLEMENT_BYTES)?;
        self.publish_new_record(&record, &encoded, deadline, || {})
    }

    #[cfg(feature = "receipt-ledger-test-support")]
    pub(crate) fn seed_tombstones_for_test(
        &self,
        keys: Vec<ReceiptKey>,
        acknowledged_at_epoch_ms: u64,
        terminal_digest: TerminalDigest,
        deadline: Instant,
    ) -> Result<Vec<AcknowledgedTombstoneReceipt>, ReceiptLedgerError> {
        let mut catalog = self
            .writer
            .lock()
            .map_err(|_| ReceiptLedgerError::Corrupt("receipt catalog lock was poisoned"))?;
        if catalog.unavailable {
            return Err(ReceiptLedgerError::StoreUnavailable);
        }
        latch_catalog_result(&mut catalog, self.verify_named_authority())?;
        if catalog
            .tombstone_count()
            .checked_add(keys.len())
            .filter(|count| *count <= MAX_ACKNOWLEDGED_TOMBSTONES)
            .is_none()
        {
            return Err(ReceiptLedgerError::TombstoneCapacityExceeded);
        }
        let mut generation =
            latch_catalog_result(&mut catalog, self.generation_under_writer_lock())?;
        let mut rows = Vec::with_capacity(keys.len());
        let mut entries = Vec::with_capacity(keys.len());
        let mut receipts = Vec::with_capacity(keys.len());
        for key in keys {
            check_deadline(deadline)?;
            let key_digest = receipt_key_digest(&key);
            generation = generation
                .checked_add(1)
                .ok_or(ReceiptLedgerError::Corrupt(
                    "receipt generation exhausted u64",
                ))?;
            let record = StoredActiveReceiptV1 {
                schema_version: RECEIPT_RECORD_SCHEMA_VERSION,
                mutation_sequence: generation,
                record_version: ReceiptVersion::new(3).ok_or(ReceiptLedgerError::Corrupt(
                    "tombstone fixture version must be nonzero",
                ))?,
                key,
                key_digest,
                lifecycle: StoredActiveLifecycleV1::AcknowledgedTombstone {
                    terminal_digest: terminal_digest.clone(),
                    acknowledged_at_epoch_ms,
                },
            };
            let (record, encoded) =
                serialize_reserved_record(record, MAX_ACKNOWLEDGED_TOMBSTONE_BYTES)?;
            let entry = CatalogEntry {
                record: record.clone(),
                encoded_bytes: u64::try_from(encoded.len())
                    .map_err(|_| ReceiptLedgerError::RecordTooLarge)?,
            };
            entries.push(entry);
            receipts.push(AcknowledgedTombstoneReceipt::new(
                record.key.clone(),
                record.key_digest.clone(),
                terminal_digest.clone(),
                acknowledged_at_epoch_ms,
                u64::try_from(encoded.len()).map_err(|_| ReceiptLedgerError::RecordTooLarge)?,
            )?);
            rows.push(ReceiptBatchRow { record, encoded });
        }

        validate_catalog_insert_batch(&catalog, &entries)?;
        // The fixture uses the production batch envelope and bounded segment
        // size. It therefore measures recovery and retention of the actual
        // durable shape without issuing 28,864 unrelated directory fsyncs.
        for (row_chunk, entry_chunk) in rows
            .chunks(MAX_RECEIPT_BATCH_ENVELOPE_ROWS)
            .zip(entries.chunks(MAX_RECEIPT_BATCH_ENVELOPE_ROWS))
        {
            let final_generation = row_chunk
                .last()
                .map(|row| row.record.mutation_sequence)
                .ok_or(ReceiptLedgerError::Corrupt(
                    "tombstone fixture produced an empty batch",
                ))?;
            let committed = entry_chunk.to_vec();
            self.publish_record_batch(
                &mut catalog,
                row_chunk,
                final_generation,
                deadline,
                move |catalog| {
                    for entry in committed {
                        commit_catalog_insert(catalog, entry);
                    }
                },
            )?;
        }
        Ok(receipts)
    }

    pub(crate) fn publish_direct_terminal_publication(
        &self,
        key: &ReceiptKey,
        expected_version: ReceiptVersion,
        terminal_epoch_ms: u64,
        terminal: V5CanonicalTerminal,
        deadline: Instant,
    ) -> Result<CommittedDirectPublication, ReceiptLedgerError> {
        check_deadline(deadline)?;
        let key_digest = receipt_key_digest(key);
        let mut catalog = self
            .writer
            .lock()
            .map_err(|_| ReceiptLedgerError::Corrupt("receipt catalog lock was poisoned"))?;
        let classified =
            self.inspect_catalog_under_stable_fence(&mut catalog, Some(deadline), |catalog| {
                if let Some(existing) = catalog.records.get(&key_digest) {
                    if &existing.record.key != key {
                        return Err(ReceiptLedgerError::ReceiptDigestCollision);
                    }
                    return Ok(existing.clone());
                }
                if catalog.invocation_index.contains_key(&key.invocation_id()) {
                    return Err(ReceiptLedgerError::InvocationIdentityMismatch);
                }
                if catalog
                    .reserved_task_index
                    .contains_key(&key.reserved_task_id())
                {
                    return Err(ReceiptLedgerError::ReservedTaskIdentityMismatch);
                }
                Err(ReceiptLedgerError::ReceiptNotFound)
            })?;
        let expected = match classified {
            Ok(existing) => existing,
            Err(error) if error.requires_reopen() => {
                return latch_catalog_error(&mut catalog, error)
            }
            Err(error) => return Err(error),
        };
        let persisted =
            match self.read_entry_under_writer_lock(&mut catalog, &key_digest, Some(deadline))? {
                Some(persisted) if persisted == expected => persisted,
                Some(_) => {
                    return latch_catalog_error(
                        &mut catalog,
                        ReceiptLedgerError::Corrupt("catalogued receipt row changed on disk"),
                    )
                }
                None => {
                    return latch_catalog_error(
                        &mut catalog,
                        ReceiptLedgerError::Corrupt("catalogued receipt row is missing"),
                    )
                }
            };

        let persisted_state = match persisted.state() {
            Ok(state) => state,
            Err(error) => return latch_catalog_error(&mut catalog, error),
        };
        let original_cutoff = match persisted_state {
            ReceiptState::DirectTerminalUnacked(committed)
                if committed.terminal_epoch_ms() == terminal_epoch_ms
                    && committed.terminal() == &terminal =>
            {
                let wire_frame = match prepare_committed_direct_wire(&committed) {
                    Ok(wire_frame) => wire_frame,
                    Err(error) if error.requires_reopen() => {
                        return latch_catalog_error(&mut catalog, error)
                    }
                    Err(error) => return Err(error),
                };
                return Ok(CommittedDirectPublication::new(committed, wire_frame));
            }
            ReceiptState::DirectTerminalUnacked(_) => {
                return Err(ReceiptLedgerError::TerminalMismatch)
            }
            ReceiptState::CancelReserved(_) => {
                return Err(ReceiptLedgerError::ReceiptRowPresentUnsupported)
            }
            ReceiptState::Reserved(reserved) => *reserved.original_cutoff(),
            ReceiptState::TaskTerminalReceiptBacked(_)
            | ReceiptState::AcknowledgedTombstone(_)
            | ReceiptState::TaskPromisedUnbound(_)
            | ReceiptState::TaskPromisedActorBound(_)
            | ReceiptState::TaskHandoffActorBound(_)
            | ReceiptState::TaskReceiptOwnedActorBound(_)
            | ReceiptState::TaskBound(_)
            | ReceiptState::TaskTerminalBound(_)
            | ReceiptState::TaskRetirementPending(_) => {
                return Err(ReceiptLedgerError::ReceiptRowPresentUnsupported)
            }
        };
        if persisted.record.record_version != expected_version {
            return Err(ReceiptLedgerError::ReceiptVersionMismatch {
                expected: expected_version,
                actual: persisted.record.record_version,
            });
        }

        let next_record_version = match expected_version.checked_next() {
            Some(version) => version,
            None => {
                return latch_catalog_error(
                    &mut catalog,
                    ReceiptLedgerError::Corrupt("receipt record version exhausted u64"),
                )
            }
        };
        let generation = latch_catalog_result(&mut catalog, self.generation_under_writer_lock())?;
        let mutation_sequence = match generation.checked_add(1) {
            Some(sequence) => sequence,
            None => {
                return latch_catalog_error(
                    &mut catalog,
                    ReceiptLedgerError::Corrupt("receipt generation exhausted u64"),
                )
            }
        };
        let write_slot = match DirectReceiptWriteSlot::new(
            key,
            expected_version,
            next_record_version,
            generation,
            mutation_sequence,
            original_cutoff,
        ) {
            Ok(write_slot) => write_slot,
            Err(error) => return latch_catalog_error(&mut catalog, error),
        };
        if write_slot.generation_before() != generation {
            return latch_catalog_error(
                &mut catalog,
                ReceiptLedgerError::Corrupt("direct write slot changed its generation fence"),
            );
        }
        let prepared = match prepare_direct_terminal(write_slot, terminal, terminal_epoch_ms) {
            Ok(prepared) => prepared,
            Err(error) if error.requires_reopen() => {
                return latch_catalog_error(&mut catalog, error)
            }
            Err(error) => return Err(error),
        };
        if prepared.record().binding() != prepared.wire_frame().binding()
            || prepared.record().binding().key() != key
            || prepared.record().binding().key_digest() != &key_digest
            || prepared.record().binding().expected_version() != expected_version
            || prepared.record().binding().committed_version() != next_record_version
            || prepared.record().binding().mutation_sequence() != mutation_sequence
            || prepared.record().binding().original_cutoff() != original_cutoff
            || prepared.record().binding().terminal_epoch_ms() != terminal_epoch_ms
            || prepared.record().binding().terminal_digest()
                != prepared.record().terminal().digest()
            || u64::try_from(prepared.record().bytes().len())
                != Ok(prepared.record().encoded_bytes())
            || prepared
                .record()
                .encoded_bytes()
                .checked_add(prepared.record().reserved_result_bytes())
                != Some(MAX_RECEIPT_ENTITLEMENT_BYTES)
        {
            return latch_catalog_error(
                &mut catalog,
                ReceiptLedgerError::Corrupt(
                    "prepared Direct publication does not match its ledger write slot",
                ),
            );
        }
        let (prepared_record, wire_frame) = prepared.into_parts();
        let binding = prepared_record.binding();
        let encoded = prepared_record.bytes();
        let encoded_bytes = prepared_record.encoded_bytes();
        let reserved_result_bytes = prepared_record.reserved_result_bytes();
        let committed_terminal = prepared_record.terminal().clone();
        let committed_key = binding.key().clone();
        let committed_key_digest = binding.key_digest().clone();
        let record = StoredActiveReceiptV1 {
            schema_version: RECEIPT_RECORD_SCHEMA_VERSION,
            mutation_sequence: binding.mutation_sequence(),
            record_version: binding.committed_version(),
            key: committed_key.clone(),
            key_digest: committed_key_digest.clone(),
            lifecycle: StoredActiveLifecycleV1::DirectTerminalUnacked {
                original_cutoff: binding.original_cutoff(),
                terminal_epoch_ms: binding.terminal_epoch_ms(),
                terminal_digest: binding.terminal_digest().clone(),
                terminal: committed_terminal.outcome_shared(),
            },
        };
        let replacement = CatalogEntry {
            record: record.clone(),
            encoded_bytes,
        };
        if let Err(error) = validate_catalog_replace(&catalog, &expected, &replacement) {
            if error.requires_reopen() {
                return latch_catalog_error(&mut catalog, error);
            }
            return Err(error);
        }
        if let Err(error) = self.publish_replacement_record(&record, encoded, deadline, || {
            commit_catalog_replace(&mut catalog, replacement);
        }) {
            if !matches!(error, ReceiptLedgerError::DeadlineExceeded) {
                catalog.unavailable = true;
            }
            return Err(error);
        }
        if let Err(error) =
            self.publish_generation(mutation_sequence, Some(&key_digest), Some(deadline))
        {
            catalog.unavailable = true;
            return Err(error);
        }
        match self.read_active_record_bytes(&key_digest) {
            Ok(Some(committed)) if committed.as_slice() == encoded => {}
            Ok(Some(_)) | Ok(None) | Err(_) => {
                catalog.unavailable = true;
                return Err(ReceiptLedgerError::CommitUncertain {
                    receipt_key_digest: key_digest,
                });
            }
        }
        if check_deadline(deadline).is_err() || self.verify_named_authority().is_err() {
            catalog.unavailable = true;
            return Err(ReceiptLedgerError::CommitUncertain {
                receipt_key_digest: key_digest,
            });
        }
        let receipt = DirectTerminalUnackedReceipt::new(
            ReceiptRecordHeader::new(
                committed_key,
                committed_key_digest,
                next_record_version,
                mutation_sequence,
                encoded_bytes,
            ),
            original_cutoff,
            terminal_epoch_ms,
            committed_terminal,
            reserved_result_bytes,
        );
        Ok(CommittedDirectPublication::with_prepared_record(
            receipt,
            wire_frame,
            prepared_record,
        ))
    }

    #[cfg(test)]
    pub(crate) fn publish_direct_terminal(
        &self,
        key: &ReceiptKey,
        expected_version: ReceiptVersion,
        terminal_epoch_ms: u64,
        terminal: V5CanonicalTerminal,
        deadline: Instant,
    ) -> Result<DirectTerminalUnackedReceipt, ReceiptLedgerError> {
        self.publish_direct_terminal_publication(
            key,
            expected_version,
            terminal_epoch_ms,
            terminal,
            deadline,
        )
        .map(|publication| publication.into_parts().0)
    }

    pub(crate) fn acknowledge_direct(
        &self,
        key: &ReceiptKey,
        terminal_digest: &TerminalDigest,
        acknowledged_at_epoch_ms: u64,
        deadline: Instant,
    ) -> Result<AcknowledgedTombstoneReceipt, ReceiptLedgerError> {
        check_deadline(deadline)?;
        acknowledged_at_epoch_ms
            .checked_add(ACKNOWLEDGED_TOMBSTONE_TTL_MS)
            .ok_or(ReceiptLedgerError::TimestampOverflow)?;
        let key_digest = receipt_key_digest(key);
        let mut catalog = self
            .writer
            .lock()
            .map_err(|_| ReceiptLedgerError::Corrupt("receipt catalog lock was poisoned"))?;
        let classified =
            self.inspect_catalog_under_stable_fence(&mut catalog, Some(deadline), |catalog| {
                if let Some(existing) = catalog.records.get(&key_digest) {
                    if &existing.record.key != key {
                        return Err(ReceiptLedgerError::ReceiptDigestCollision);
                    }
                    return Ok(existing.clone());
                }
                if catalog.invocation_index.contains_key(&key.invocation_id()) {
                    return Err(ReceiptLedgerError::InvocationIdentityMismatch);
                }
                if catalog
                    .reserved_task_index
                    .contains_key(&key.reserved_task_id())
                {
                    return Err(ReceiptLedgerError::ReservedTaskIdentityMismatch);
                }
                Err(ReceiptLedgerError::ReceiptNotFound)
            })?;
        let expected = match classified {
            Ok(existing) => existing,
            Err(error) if error.requires_reopen() => {
                return latch_catalog_error(&mut catalog, error)
            }
            Err(error) => return Err(error),
        };
        let persisted =
            match self.read_entry_under_writer_lock(&mut catalog, &key_digest, Some(deadline))? {
                Some(persisted) if persisted == expected => persisted,
                Some(_) => {
                    return latch_catalog_error(
                        &mut catalog,
                        ReceiptLedgerError::Corrupt("catalogued receipt row changed on disk"),
                    )
                }
                None => {
                    return latch_catalog_error(
                        &mut catalog,
                        ReceiptLedgerError::Corrupt("catalogued receipt row is missing"),
                    )
                }
            };
        match persisted.state() {
            Ok(ReceiptState::AcknowledgedTombstone(tombstone)) => {
                if tombstone.terminal_digest() != terminal_digest {
                    return self.reject_before_mutation(
                        &mut catalog,
                        deadline,
                        ReceiptLedgerError::TerminalMismatch,
                    );
                }
                if acknowledged_at_epoch_ms >= tombstone.expires_at_epoch_ms() {
                    self.reclaim_expired_tombstone_under_writer_lock(
                        &mut catalog,
                        &key_digest,
                        acknowledged_at_epoch_ms,
                        deadline,
                    )?;
                    return Err(ReceiptLedgerError::ReceiptNotFound);
                }
                return Ok(tombstone);
            }
            Ok(ReceiptState::DirectTerminalUnacked(receipt)) => {
                if receipt
                    .terminal_epoch_ms()
                    .checked_add(DIRECT_TERMINAL_RETENTION_MS)
                    .is_some_and(|expires_at_epoch_ms| {
                        acknowledged_at_epoch_ms >= expires_at_epoch_ms
                    })
                {
                    self.reclaim_expired_direct_terminal_under_writer_lock(
                        &mut catalog,
                        &key_digest,
                        acknowledged_at_epoch_ms,
                        deadline,
                    )?;
                    return Err(ReceiptLedgerError::ReceiptNotFound);
                }
                if receipt.terminal().digest() != terminal_digest {
                    return self.reject_before_mutation(
                        &mut catalog,
                        deadline,
                        ReceiptLedgerError::TerminalMismatch,
                    );
                }
            }
            Ok(_) => {
                return self.reject_before_mutation(
                    &mut catalog,
                    deadline,
                    ReceiptLedgerError::ReceiptRowPresentUnsupported,
                )
            }
            Err(error) => return latch_catalog_error(&mut catalog, error),
        }
        if let Err(error) =
            self.materialize_batch_backed_record(&mut catalog, &key_digest, deadline)
        {
            return latch_catalog_error(&mut catalog, error);
        }

        let record = StoredActiveReceiptV1 {
            schema_version: RECEIPT_RECORD_SCHEMA_VERSION,
            mutation_sequence: 0,
            record_version: persisted.record.record_version.checked_next().ok_or(
                ReceiptLedgerError::Corrupt("receipt record version exhausted u64"),
            )?,
            key: key.clone(),
            key_digest: key_digest.clone(),
            lifecycle: StoredActiveLifecycleV1::AcknowledgedTombstone {
                terminal_digest: terminal_digest.clone(),
                acknowledged_at_epoch_ms,
            },
        };
        let (record, encoded) =
            serialize_reserved_record(record, MAX_ACKNOWLEDGED_TOMBSTONE_BYTES)?;
        let encoded_bytes =
            u64::try_from(encoded.len()).map_err(|_| ReceiptLedgerError::RecordTooLarge)?;
        let replacement = CatalogEntry {
            record: record.clone(),
            encoded_bytes,
        };
        // ACK owns only the minimum capacity work needed for this one
        // transition. Bulk expiry remains a bounded maintenance command.
        self.reclaim_expired_tombstones_for_ack_capacity_under_writer_lock(
            &mut catalog,
            &replacement,
            acknowledged_at_epoch_ms,
            deadline,
        )?;
        if let Err(error) = validate_catalog_replace(&catalog, &persisted, &replacement) {
            return self.reject_before_mutation(&mut catalog, deadline, error);
        }
        let generation = latch_catalog_result(&mut catalog, self.generation_under_writer_lock())?;
        let mutation_sequence = generation
            .checked_add(1)
            .ok_or(ReceiptLedgerError::Corrupt(
                "receipt generation exhausted u64",
            ))?;
        let witness = build_acknowledgement_commit_record(
            &persisted,
            terminal_digest.clone(),
            acknowledged_at_epoch_ms,
            mutation_sequence,
        )?;
        let (witness, witness_encoded) =
            serialize_reserved_record(witness, MAX_CANCEL_RESERVED_RECORD_BYTES)?;
        if let Err(error) =
            self.publish_replacement_record(&witness, &witness_encoded, deadline, || {})
        {
            if !matches!(error, ReceiptLedgerError::DeadlineExceeded) {
                catalog.unavailable = true;
            }
            return Err(error);
        }
        if let Err(error) =
            self.publish_generation(mutation_sequence, Some(&key_digest), Some(deadline))
        {
            catalog.unavailable = true;
            return Err(error);
        }
        if let Err(error) = self.publish_replacement_record(&record, &encoded, deadline, || {
            commit_catalog_replace(&mut catalog, replacement);
        }) {
            catalog.unavailable = true;
            return Err(after_row_error(Some(&key_digest), error));
        }
        match self.read_active_record_bytes(&key_digest) {
            Ok(Some(committed)) if committed == encoded => {}
            Ok(Some(_)) | Ok(None) | Err(_) => {
                catalog.unavailable = true;
                return Err(ReceiptLedgerError::CommitUncertain {
                    receipt_key_digest: key_digest,
                });
            }
        }
        if check_deadline(deadline).is_err() || self.verify_named_authority().is_err() {
            catalog.unavailable = true;
            return Err(ReceiptLedgerError::CommitUncertain {
                receipt_key_digest: key_digest,
            });
        }
        AcknowledgedTombstoneReceipt::new(
            key.clone(),
            receipt_key_digest(key),
            terminal_digest.clone(),
            acknowledged_at_epoch_ms,
            encoded_bytes,
        )
    }

    pub(crate) fn reclaim_expired_tombstones(
        &self,
        observed_at_epoch_ms: u64,
        deadline: Instant,
    ) -> Result<usize, ReceiptLedgerError> {
        check_deadline(deadline)?;
        let mut catalog = self
            .writer
            .lock()
            .map_err(|_| ReceiptLedgerError::Corrupt("receipt catalog lock was poisoned"))?;
        if catalog.unavailable {
            return Err(ReceiptLedgerError::StoreUnavailable);
        }
        latch_catalog_result(&mut catalog, self.verify_named_authority())?;
        self.reclaim_expired_tombstones_under_writer_lock(
            &mut catalog,
            observed_at_epoch_ms,
            deadline,
        )
    }

    fn reclaim_expired_tombstones_under_writer_lock(
        &self,
        catalog: &mut ReceiptCatalog,
        observed_at_epoch_ms: u64,
        deadline: Instant,
    ) -> Result<usize, ReceiptLedgerError> {
        let mut expired = catalog
            .records
            .iter()
            .filter(|(_, entry)| {
                entry_is_expired_tombstone(entry, observed_at_epoch_ms)
                    || entry_is_expired_direct_terminal(entry, observed_at_epoch_ms)
                    || entry_is_expired_task_receipt_terminal(entry, observed_at_epoch_ms)
            })
            .map(|(digest, _)| digest.clone())
            .collect::<Vec<_>>();
        expired.sort_unstable_by(|left, right| left.as_str().cmp(right.as_str()));
        let total_expired = expired.len();
        let expired_tombstones = expired
            .iter()
            .filter(|digest| {
                catalog
                    .records
                    .get(*digest)
                    .is_some_and(|entry| entry.is_tombstone())
            })
            .cloned()
            .collect::<HashSet<_>>();
        let mut backing_groups: HashMap<
            String,
            (ReceiptBatchBacking, Vec<ReceiptKeyDigest>, bool),
        > = HashMap::new();
        for (digest, backing) in &catalog.batch_backing {
            let group = backing_groups
                .entry(backing.name.clone())
                .or_insert_with(|| (backing.clone(), Vec::new(), true));
            group.1.push(digest.clone());
            group.2 &= expired_tombstones.contains(digest)
                && catalog
                    .records
                    .get(digest)
                    .is_some_and(CatalogEntry::is_tombstone);
        }
        let mut bulk_groups = backing_groups
            .into_values()
            .filter(|(_, digests, all_expired_tombstones)| {
                *all_expired_tombstones && !digests.is_empty()
            })
            .collect::<Vec<_>>();
        bulk_groups.sort_unstable_by(|left, right| left.0.name.cmp(&right.0.name));
        let mut bulk_reclaimed = HashSet::new();
        for (backing, mut digests, _) in bulk_groups {
            check_deadline(deadline)?;
            digests.sort_unstable_by(|left, right| left.as_str().cmp(right.as_str()));
            let expected = digests
                .iter()
                .map(|digest| {
                    catalog
                        .records
                        .get(digest)
                        .cloned()
                        .ok_or(ReceiptLedgerError::Corrupt(
                            "expired receipt batch entry disappeared",
                        ))
                })
                .collect::<Result<Vec<_>, _>>()?;
            for entry in &expected {
                validate_catalog_remove(catalog, entry)?;
            }
            let generation = self.generation_under_writer_lock()?;
            let final_generation = generation
                .checked_add(u64::try_from(expected.len()).map_err(|_| {
                    ReceiptLedgerError::Corrupt("expired receipt batch size exceeds u64")
                })?)
                .ok_or(ReceiptLedgerError::Corrupt(
                    "receipt generation exhausted u64",
                ))?;
            self.publish_generation(final_generation, digests.first(), Some(deadline))?;
            self.retire_receipt_batch_backings(std::slice::from_ref(&backing), deadline)?;
            for entry in &expected {
                commit_catalog_remove(catalog, entry);
                catalog.batch_backing.remove(&entry.record.key_digest);
                bulk_reclaimed.insert(entry.record.key_digest.clone());
            }
            if catalog.tombstone_compaction_backing.as_ref() == Some(&backing) {
                catalog.tombstone_compaction_backing = None;
            }
        }
        expired.retain(|digest| !bulk_reclaimed.contains(digest));
        for digest in &expired {
            match catalog
                .records
                .get(digest)
                .map(|entry| &entry.record.lifecycle)
            {
                Some(StoredActiveLifecycleV1::AcknowledgedTombstone { .. }) => {
                    self.reclaim_expired_tombstone_under_writer_lock(
                        catalog,
                        digest,
                        observed_at_epoch_ms,
                        deadline,
                    )?;
                }
                Some(StoredActiveLifecycleV1::DirectTerminalUnacked { .. }) => {
                    self.reclaim_expired_direct_terminal_under_writer_lock(
                        catalog,
                        digest,
                        observed_at_epoch_ms,
                        deadline,
                    )?;
                }
                Some(StoredActiveLifecycleV1::TaskTerminalReceiptBacked { .. }) => {
                    self.reclaim_expired_task_receipt_terminal_under_writer_lock(
                        catalog,
                        digest,
                        observed_at_epoch_ms,
                        deadline,
                    )?;
                }
                Some(_) => {
                    return latch_catalog_error(
                        catalog,
                        ReceiptLedgerError::Corrupt(
                            "expired retention candidate changed lifecycle",
                        ),
                    )
                }
                None => {
                    return latch_catalog_error(
                        catalog,
                        ReceiptLedgerError::Corrupt("expired retention candidate disappeared"),
                    )
                }
            }
        }
        Ok(total_expired)
    }

    fn reclaim_expired_tombstones_for_ack_capacity_under_writer_lock(
        &self,
        catalog: &mut ReceiptCatalog,
        replacement: &CatalogEntry,
        observed_at_epoch_ms: u64,
        deadline: Instant,
    ) -> Result<(), ReceiptLedgerError> {
        if ack_tombstone_has_capacity(catalog, replacement) {
            return Ok(());
        }

        let mut expired = catalog
            .records
            .iter()
            .filter(|(digest, _)| digest != &&replacement.record.key_digest)
            .filter(|(_, entry)| entry_is_expired_tombstone(entry, observed_at_epoch_ms))
            .map(|(digest, _)| digest.clone())
            .collect::<Vec<_>>();
        expired.sort_unstable_by(|left, right| left.as_str().cmp(right.as_str()));
        for digest in expired {
            self.reclaim_expired_tombstone_under_writer_lock(
                catalog,
                &digest,
                observed_at_epoch_ms,
                deadline,
            )?;
            if ack_tombstone_has_capacity(catalog, replacement) {
                return Ok(());
            }
        }
        Err(ReceiptLedgerError::TombstoneCapacityExceeded)
    }

    fn reclaim_expired_tombstone_under_writer_lock(
        &self,
        catalog: &mut ReceiptCatalog,
        key_digest: &ReceiptKeyDigest,
        observed_at_epoch_ms: u64,
        deadline: Instant,
    ) -> Result<(), ReceiptLedgerError> {
        check_deadline(deadline)?;
        let expected =
            catalog
                .records
                .get(key_digest)
                .cloned()
                .ok_or(ReceiptLedgerError::Corrupt(
                    "expired tombstone disappeared while the writer lock was held",
                ))?;
        let persisted = self
            .read_entry_under_writer_lock(catalog, key_digest, Some(deadline))?
            .ok_or(ReceiptLedgerError::Corrupt(
                "catalogued expired tombstone row is missing",
            ))?;
        if persisted != expected {
            return latch_catalog_error(
                catalog,
                ReceiptLedgerError::Corrupt("catalogued expired tombstone changed on disk"),
            );
        }
        let expires_at_epoch_ms = match persisted.state() {
            Ok(ReceiptState::AcknowledgedTombstone(receipt)) => receipt.expires_at_epoch_ms(),
            Ok(_) => {
                return latch_catalog_error(
                    catalog,
                    ReceiptLedgerError::Corrupt(
                        "expired tombstone candidate changed lifecycle under writer lock",
                    ),
                )
            }
            Err(error) => return latch_catalog_error(catalog, error),
        };
        if observed_at_epoch_ms < expires_at_epoch_ms {
            return latch_catalog_error(
                catalog,
                ReceiptLedgerError::Corrupt("selected acknowledged tombstone is not expired"),
            );
        }
        let generation = latch_catalog_result(catalog, self.generation_under_writer_lock())?;
        let mutation_sequence = generation
            .checked_add(1)
            .ok_or(ReceiptLedgerError::Corrupt(
                "receipt generation exhausted u64",
            ))?;
        let record = build_expired_tombstone_deletion_record(
            &persisted,
            observed_at_epoch_ms,
            mutation_sequence,
        )?;
        let (record, encoded) =
            serialize_reserved_record(record, MAX_CANCEL_RESERVED_RECORD_BYTES)?;
        if let Err(error) = validate_catalog_remove(catalog, &persisted) {
            return latch_catalog_error(catalog, error);
        }
        if let Err(error) = self.publish_replacement_record(&record, &encoded, deadline, || {
            commit_catalog_remove(catalog, &persisted);
        }) {
            if !matches!(error, ReceiptLedgerError::DeadlineExceeded) {
                catalog.unavailable = true;
            }
            return Err(error);
        }
        if let Err(error) =
            self.publish_generation(mutation_sequence, Some(key_digest), Some(deadline))
        {
            catalog.unavailable = true;
            return Err(error);
        }
        if let Err(error) = self.remove_expired_deletion_witness(key_digest, &encoded, deadline) {
            catalog.unavailable = true;
            return Err(error);
        }
        if check_deadline(deadline).is_err() || self.verify_named_authority().is_err() {
            catalog.unavailable = true;
            return Err(ReceiptLedgerError::CommitUncertain {
                receipt_key_digest: key_digest.clone(),
            });
        }
        Ok(())
    }

    fn reclaim_expired_direct_terminal_under_writer_lock(
        &self,
        catalog: &mut ReceiptCatalog,
        key_digest: &ReceiptKeyDigest,
        observed_at_epoch_ms: u64,
        deadline: Instant,
    ) -> Result<(), ReceiptLedgerError> {
        check_deadline(deadline)?;
        let expected =
            catalog
                .records
                .get(key_digest)
                .cloned()
                .ok_or(ReceiptLedgerError::Corrupt(
                    "expired Direct receipt disappeared while the writer lock was held",
                ))?;
        let persisted = self
            .read_entry_under_writer_lock(catalog, key_digest, Some(deadline))?
            .ok_or(ReceiptLedgerError::Corrupt(
                "catalogued expired Direct receipt row is missing",
            ))?;
        if persisted != expected {
            return latch_catalog_error(
                catalog,
                ReceiptLedgerError::Corrupt("catalogued expired Direct receipt changed on disk"),
            );
        }
        let expires_at_epoch_ms = match persisted.state() {
            Ok(ReceiptState::DirectTerminalUnacked(receipt)) => receipt
                .terminal_epoch_ms()
                .checked_add(DIRECT_TERMINAL_RETENTION_MS)
                .ok_or(ReceiptLedgerError::Corrupt(
                    "Direct terminal expiry exceeds u64",
                ))?,
            Ok(_) => {
                return latch_catalog_error(
                    catalog,
                    ReceiptLedgerError::Corrupt(
                        "expired Direct candidate changed lifecycle under writer lock",
                    ),
                )
            }
            Err(error) => return latch_catalog_error(catalog, error),
        };
        if observed_at_epoch_ms < expires_at_epoch_ms {
            return latch_catalog_error(
                catalog,
                ReceiptLedgerError::Corrupt("selected Direct terminal is not expired"),
            );
        }
        if let Err(error) = self.materialize_batch_backed_record(catalog, key_digest, deadline) {
            return latch_catalog_error(catalog, error);
        }

        let generation = latch_catalog_result(catalog, self.generation_under_writer_lock())?;
        let mutation_sequence = generation
            .checked_add(1)
            .ok_or(ReceiptLedgerError::Corrupt(
                "receipt generation exhausted u64",
            ))?;
        let record = build_expired_direct_deletion_record(
            &persisted,
            observed_at_epoch_ms,
            mutation_sequence,
        )?;
        let (record, encoded) =
            serialize_reserved_record(record, MAX_CANCEL_RESERVED_RECORD_BYTES)?;
        if let Err(error) = validate_catalog_remove(catalog, &persisted) {
            return latch_catalog_error(catalog, error);
        }
        if let Err(error) = self.publish_replacement_record(&record, &encoded, deadline, || {
            commit_catalog_remove(catalog, &persisted);
        }) {
            if !matches!(error, ReceiptLedgerError::DeadlineExceeded) {
                catalog.unavailable = true;
            }
            return Err(error);
        }
        if let Err(error) =
            self.publish_generation(mutation_sequence, Some(key_digest), Some(deadline))
        {
            catalog.unavailable = true;
            return Err(error);
        }
        if let Err(error) = self.remove_expired_deletion_witness(key_digest, &encoded, deadline) {
            catalog.unavailable = true;
            return Err(error);
        }
        if check_deadline(deadline).is_err() || self.verify_named_authority().is_err() {
            catalog.unavailable = true;
            return Err(ReceiptLedgerError::CommitUncertain {
                receipt_key_digest: key_digest.clone(),
            });
        }
        Ok(())
    }

    fn reclaim_expired_task_receipt_terminal_under_writer_lock(
        &self,
        catalog: &mut ReceiptCatalog,
        key_digest: &ReceiptKeyDigest,
        observed_at_epoch_ms: u64,
        deadline: Instant,
    ) -> Result<(), ReceiptLedgerError> {
        check_deadline(deadline)?;
        let expected =
            catalog
                .records
                .get(key_digest)
                .cloned()
                .ok_or(ReceiptLedgerError::Corrupt(
                    "expired receipt-backed Task disappeared while the writer lock was held",
                ))?;
        let persisted = self
            .read_entry_under_writer_lock(catalog, key_digest, Some(deadline))?
            .ok_or(ReceiptLedgerError::Corrupt(
                "catalogued expired receipt-backed Task row is missing",
            ))?;
        if persisted != expected {
            return latch_catalog_error(
                catalog,
                ReceiptLedgerError::Corrupt(
                    "catalogued expired receipt-backed Task changed on disk",
                ),
            );
        }
        let expires_at_epoch_ms = match persisted.state() {
            Ok(ReceiptState::TaskTerminalReceiptBacked(receipt)) => receipt.expires_at_epoch_ms(),
            Ok(_) => {
                return latch_catalog_error(
                    catalog,
                    ReceiptLedgerError::Corrupt(
                        "expired receipt-backed Task candidate changed lifecycle under writer lock",
                    ),
                )
            }
            Err(error) => return latch_catalog_error(catalog, error),
        };
        if observed_at_epoch_ms < expires_at_epoch_ms {
            return latch_catalog_error(
                catalog,
                ReceiptLedgerError::Corrupt("selected receipt-backed Task terminal is not expired"),
            );
        }

        let generation = latch_catalog_result(catalog, self.generation_under_writer_lock())?;
        let mutation_sequence = generation
            .checked_add(1)
            .ok_or(ReceiptLedgerError::Corrupt(
                "receipt generation exhausted u64",
            ))?;
        let record = build_expired_task_receipt_deletion_record(
            &persisted,
            observed_at_epoch_ms,
            mutation_sequence,
        )?;
        let (record, encoded) =
            serialize_reserved_record(record, MAX_CANCEL_RESERVED_RECORD_BYTES)?;
        if let Err(error) = validate_catalog_remove(catalog, &persisted) {
            return latch_catalog_error(catalog, error);
        }
        if let Err(error) = self.publish_replacement_record(&record, &encoded, deadline, || {
            commit_catalog_remove(catalog, &persisted);
        }) {
            if !matches!(error, ReceiptLedgerError::DeadlineExceeded) {
                catalog.unavailable = true;
            }
            return Err(error);
        }
        if let Err(error) =
            self.publish_generation(mutation_sequence, Some(key_digest), Some(deadline))
        {
            catalog.unavailable = true;
            return Err(error);
        }
        if let Err(error) = self.remove_expired_deletion_witness(key_digest, &encoded, deadline) {
            catalog.unavailable = true;
            return Err(error);
        }
        if check_deadline(deadline).is_err() || self.verify_named_authority().is_err() {
            catalog.unavailable = true;
            return Err(ReceiptLedgerError::CommitUncertain {
                receipt_key_digest: key_digest.clone(),
            });
        }
        Ok(())
    }

    fn reject_before_mutation<T>(
        &self,
        catalog: &mut ReceiptCatalog,
        deadline: Instant,
        error: ReceiptLedgerError,
    ) -> Result<T, ReceiptLedgerError> {
        check_deadline(deadline)?;
        latch_catalog_result(catalog, self.verify_named_authority())?;
        Err(error)
    }

    pub(crate) fn read_reserved(
        &self,
        receipt_key_digest: &ReceiptKeyDigest,
    ) -> Result<Option<ReservedReceipt>, ReceiptLedgerError> {
        let mut catalog = self
            .writer
            .lock()
            .map_err(|_| ReceiptLedgerError::Corrupt("receipt catalog lock was poisoned"))?;
        self.read_entry_under_writer_lock(&mut catalog, receipt_key_digest, None)?
            .map(|entry| entry.reservation())
            .transpose()
    }

    fn recover_exact(
        &self,
        key: &ReceiptKey,
        deadline: Instant,
    ) -> Result<ReceiptState, ReceiptLedgerError> {
        self.recover_exact_inner(key, None, deadline)
    }

    fn recover_exact_at(
        &self,
        key: &ReceiptKey,
        observed_at_epoch_ms: u64,
        deadline: Instant,
    ) -> Result<ReceiptState, ReceiptLedgerError> {
        self.recover_exact_inner(key, Some(observed_at_epoch_ms), deadline)
    }

    fn recover_exact_inner(
        &self,
        key: &ReceiptKey,
        observed_at_epoch_ms: Option<u64>,
        deadline: Instant,
    ) -> Result<ReceiptState, ReceiptLedgerError> {
        check_deadline(deadline)?;
        let key_digest = receipt_key_digest(key);
        let mut catalog = self
            .writer
            .lock()
            .map_err(|_| ReceiptLedgerError::Corrupt("receipt catalog lock was poisoned"))?;
        let identity_mismatch =
            self.inspect_catalog_under_stable_fence(&mut catalog, Some(deadline), |catalog| {
                if catalog
                    .invocation_index
                    .get(&key.invocation_id())
                    .is_some_and(|existing| existing != &key_digest)
                {
                    Some(ReceiptLedgerError::InvocationIdentityMismatch)
                } else if catalog
                    .reserved_task_index
                    .get(&key.reserved_task_id())
                    .is_some_and(|existing| existing != &key_digest)
                {
                    Some(ReceiptLedgerError::ReservedTaskIdentityMismatch)
                } else {
                    None
                }
            })?;
        if let Some(error) = identity_mismatch {
            return Err(error);
        }
        let recovered =
            self.read_entry_under_writer_lock(&mut catalog, &key_digest, Some(deadline))?;
        let result = match recovered {
            Some(entry) if &entry.record.key != key => {
                return latch_catalog_error(
                    &mut catalog,
                    ReceiptLedgerError::ReceiptDigestCollision,
                )
            }
            Some(entry) => match entry.state() {
                Ok(ReceiptState::AcknowledgedTombstone(receipt))
                    if observed_at_epoch_ms
                        .is_some_and(|observed| observed >= receipt.expires_at_epoch_ms()) =>
                {
                    self.reclaim_expired_tombstone_under_writer_lock(
                        &mut catalog,
                        &key_digest,
                        observed_at_epoch_ms.expect("expiry guard requires an epoch"),
                        deadline,
                    )?;
                    return Err(ReceiptLedgerError::ReceiptNotFound);
                }
                Ok(ReceiptState::DirectTerminalUnacked(receipt))
                    if observed_at_epoch_ms.is_some_and(|observed| {
                        receipt
                            .terminal_epoch_ms()
                            .checked_add(DIRECT_TERMINAL_RETENTION_MS)
                            .is_some_and(|expires_at_epoch_ms| observed >= expires_at_epoch_ms)
                    }) =>
                {
                    self.reclaim_expired_direct_terminal_under_writer_lock(
                        &mut catalog,
                        &key_digest,
                        observed_at_epoch_ms.expect("expiry guard requires an epoch"),
                        deadline,
                    )?;
                    return Err(ReceiptLedgerError::ReceiptNotFound);
                }
                Ok(state) => Ok(state),
                Err(error) => return latch_catalog_error(&mut catalog, error),
            },
            None => Err(ReceiptLedgerError::ReceiptNotFound),
        };
        check_deadline(deadline)?;
        result
    }

    fn resolve_task_exact(
        &self,
        task_id: TaskId,
        deadline: Instant,
    ) -> Result<ReceiptState, ReceiptLedgerError> {
        check_deadline(deadline)?;
        let mut catalog = self
            .writer
            .lock()
            .map_err(|_| ReceiptLedgerError::Corrupt("receipt catalog lock was poisoned"))?;
        let key_digest =
            self.inspect_catalog_under_stable_fence(&mut catalog, Some(deadline), |catalog| {
                catalog.reserved_task_index.get(&task_id).cloned()
            })?;
        let key_digest = key_digest.ok_or(ReceiptLedgerError::ReceiptNotFound)?;
        let entry = self
            .read_entry_under_writer_lock(&mut catalog, &key_digest, Some(deadline))?
            .ok_or(ReceiptLedgerError::ReceiptNotFound)?;
        if entry.record.key.reserved_task_id() != task_id {
            return latch_catalog_error(
                &mut catalog,
                ReceiptLedgerError::Corrupt(
                    "reserved Task index points to a different Task identity",
                ),
            );
        }
        let state = match entry.state() {
            Ok(state) => state,
            Err(error) => return latch_catalog_error(&mut catalog, error),
        };
        if !matches!(
            state,
            ReceiptState::TaskPromisedUnbound(_)
                | ReceiptState::TaskPromisedActorBound(_)
                | ReceiptState::TaskHandoffActorBound(_)
                | ReceiptState::TaskReceiptOwnedActorBound(_)
                | ReceiptState::TaskTerminalReceiptBacked(_)
                | ReceiptState::TaskBound(_)
                | ReceiptState::TaskTerminalBound(_)
                | ReceiptState::TaskRetirementPending(_)
        ) {
            return Err(ReceiptLedgerError::ReceiptNotFound);
        }
        check_deadline(deadline)?;
        Ok(state)
    }

    fn inspect_catalog_under_stable_fence<T>(
        &self,
        catalog: &mut ReceiptCatalog,
        deadline: Option<Instant>,
        inspect: impl FnOnce(&ReceiptCatalog) -> T,
    ) -> Result<T, ReceiptLedgerError> {
        self.inspect_catalog_with_generation_under_stable_fence(
            catalog,
            deadline,
            |catalog, _generation| inspect(catalog),
        )
    }

    fn inspect_catalog_with_generation_under_stable_fence<T>(
        &self,
        catalog: &mut ReceiptCatalog,
        deadline: Option<Instant>,
        inspect: impl FnOnce(&ReceiptCatalog, u64) -> T,
    ) -> Result<T, ReceiptLedgerError> {
        if catalog.unavailable {
            return Err(ReceiptLedgerError::StoreUnavailable);
        }
        check_optional_deadline(deadline)?;
        latch_catalog_result(catalog, self.verify_named_authority())?;
        check_optional_deadline(deadline)?;
        let generation_before = latch_catalog_result(catalog, self.generation_under_writer_lock())?;
        check_optional_deadline(deadline)?;
        let inspected = inspect(catalog, generation_before);
        check_optional_deadline(deadline)?;
        let generation_after = latch_catalog_result(catalog, self.generation_under_writer_lock())?;
        if generation_after != generation_before {
            return latch_catalog_error(
                catalog,
                ReceiptLedgerError::ConcurrentGenerationChange {
                    generation_before,
                    generation_after,
                },
            );
        }
        check_optional_deadline(deadline)?;
        latch_catalog_result(catalog, self.verify_named_authority())?;
        check_optional_deadline(deadline)?;
        Ok(inspected)
    }

    fn read_entry_under_writer_lock(
        &self,
        catalog: &mut ReceiptCatalog,
        receipt_key_digest: &ReceiptKeyDigest,
        deadline: Option<Instant>,
    ) -> Result<Option<CatalogEntry>, ReceiptLedgerError> {
        if catalog.unavailable {
            return Err(ReceiptLedgerError::StoreUnavailable);
        }
        check_optional_deadline(deadline)?;
        latch_catalog_result(catalog, self.verify_named_authority())?;
        check_optional_deadline(deadline)?;
        let generation_before = latch_catalog_result(catalog, self.generation_under_writer_lock())?;
        check_optional_deadline(deadline)?;
        let entry = if let Some(backing) = catalog.batch_backing.get(receipt_key_digest) {
            if let Err(error) = self.verify_receipt_batch_backing(backing) {
                return latch_catalog_error(catalog, error);
            }
            catalog.records.get(receipt_key_digest).cloned()
        } else {
            match self.read_active_record(receipt_key_digest) {
                Ok(entry) => entry,
                Err(error) => return latch_catalog_error(catalog, error),
            }
        };
        check_optional_deadline(deadline)?;
        let generation_after = latch_catalog_result(catalog, self.generation_under_writer_lock())?;
        if generation_after != generation_before {
            return latch_catalog_error(
                catalog,
                ReceiptLedgerError::ConcurrentGenerationChange {
                    generation_before,
                    generation_after,
                },
            );
        }
        check_optional_deadline(deadline)?;
        latch_catalog_result(catalog, self.verify_named_authority())?;
        check_optional_deadline(deadline)?;
        let result = match (catalog.records.get(receipt_key_digest), entry) {
            (None, None) => Ok(None),
            (None, Some(_)) => Err(ReceiptLedgerError::Corrupt(
                "receipt row is present outside the recovered catalog",
            )),
            (Some(_), None) => Err(ReceiptLedgerError::Corrupt(
                "catalogued receipt row is missing",
            )),
            (Some(expected), Some(actual)) if expected == &actual => Ok(Some(actual)),
            (Some(_), Some(_)) => Err(ReceiptLedgerError::Corrupt(
                "catalogued receipt row changed on disk",
            )),
        };
        if result.is_err() {
            catalog.unavailable = true;
        }
        result
    }

    fn recover_existing_catalog(
        receipts: &RetainedDirectoryCapability,
        receipts_file: &File,
        active: &RetainedDirectoryCapability,
        active_file: &File,
        deadline: Instant,
    ) -> Result<RecoveredCatalog, ReceiptLedgerError> {
        check_deadline(deadline)?;
        verify_recovery_authority(receipts, receipts_file, active, active_file)?;
        let mut names =
            read_directory_names_bounded(active_file, MAX_ACTIVE_DIRECTORY_ENTRIES, || {
                recovery_checkpoint(deadline)
            })
            .map_err(|error| recovery_error("enumerate receipt active directory", error))?;
        names.sort_by(|left, right| {
            let left_batch = left
                .to_str()
                .is_some_and(|name| name.starts_with("receipt-batch."));
            let right_batch = right
                .to_str()
                .is_some_and(|name| name.starts_with("receipt-batch."));
            right_batch.cmp(&left_batch).then_with(|| left.cmp(right))
        });
        let mut catalog = ReceiptCatalog::default();
        let mut maximum_mutation_sequence = 0;
        let mut mutation_sequences = HashSet::new();
        let mut recovered_invocations = HashMap::new();
        let mut recovered_tasks = HashMap::new();
        let mut temporary_entries = Vec::new();
        let mut expired_deletions = Vec::new();
        let mut expired_deletion_mutation_sequence = None;
        let mut acknowledgement_recovery = None;
        for name in names {
            check_deadline(deadline)?;
            let Some(name_text) = name.to_str() else {
                return Err(ReceiptLedgerError::Corrupt(
                    "receipt active entry name is not UTF-8",
                ));
            };
            if parse_receipt_temporary_name(name_text)? || parse_cleanup_quarantine_name(name_text)?
            {
                let temporary = open_regular_child_nofollow(active_file, &name)
                    .map_err(|error| storage_error("open abandoned receipt staging file", error))?;
                verify_owner_only_acl(&temporary).map_err(|error| {
                    storage_error("verify abandoned receipt staging ownership", error)
                })?;
                let identity = file_identity(&temporary).map_err(|error| {
                    storage_error("identify abandoned receipt staging file", error)
                })?;
                temporary_entries.push((name, identity, temporary));
                continue;
            }
            if parse_receipt_batch_name(name_text)?.is_some() {
                let mut retained = open_regular_child_nofollow(active_file, &name)
                    .map_err(|error| storage_error("open receipt batch during recovery", error))?;
                verify_owner_only_acl(&retained).map_err(|error| {
                    storage_error("verify recovered receipt batch ownership", error)
                })?;
                let identity = file_identity(&retained)
                    .map_err(|error| storage_error("identify recovered receipt batch", error))?;
                let mut encoded = Vec::new();
                Read::by_ref(&mut retained)
                    .take(MAX_RECEIPT_BATCH_BYTES + 1)
                    .read_to_end(&mut encoded)
                    .map_err(|error| storage_error("read recovered receipt batch", error))?;
                let batch = decode_receipt_batch(&encoded)?;
                let is_partial_tombstone_batch = batch.rows.len() < MAX_RECEIPT_BATCH_ENVELOPE_ROWS
                    && batch.rows.iter().all(|row| {
                        matches!(
                            &row.record.lifecycle,
                            StoredActiveLifecycleV1::AcknowledgedTombstone { .. }
                        )
                    });
                let backing = ReceiptBatchBacking {
                    name: name_text.to_owned(),
                    identity,
                    encoded: Arc::new(encoded),
                };
                for row in batch.rows {
                    let entry = CatalogEntry {
                        record: row.record,
                        encoded_bytes: row.encoded.len() as u64,
                    };
                    let digest = entry.record.key_digest.clone();
                    let sequence_is_new = mutation_sequences.insert(entry.record.mutation_sequence);
                    maximum_mutation_sequence =
                        maximum_mutation_sequence.max(entry.record.mutation_sequence);
                    if let Some(previous) = catalog.records.get(&digest).cloned() {
                        if previous == entry {
                            catalog.batch_backing.insert(digest, backing.clone());
                            continue;
                        }
                        if !sequence_is_new {
                            return Err(ReceiptLedgerError::Corrupt(
                                "receipt recovery contains a duplicate mutation sequence",
                            ));
                        }
                        if previous.record.key != entry.record.key
                            || previous.record.record_version == entry.record.record_version
                            || previous.record.mutation_sequence == entry.record.mutation_sequence
                        {
                            return Err(ReceiptLedgerError::Corrupt(
                                "receipt batch history has an ambiguous exact successor",
                            ));
                        }
                        if entry.record.record_version > previous.record.record_version
                            && entry.record.mutation_sequence > previous.record.mutation_sequence
                        {
                            validate_catalog_replace(&catalog, &previous, &entry)?;
                            commit_catalog_replace(&mut catalog, entry);
                            catalog.batch_backing.insert(digest, backing.clone());
                        }
                    } else {
                        if !sequence_is_new {
                            return Err(ReceiptLedgerError::Corrupt(
                                "receipt recovery contains a duplicate mutation sequence",
                            ));
                        }
                        insert_catalog_entry(&mut catalog, entry, true)?;
                        catalog.batch_backing.insert(digest, backing.clone());
                    }
                }
                if is_partial_tombstone_batch {
                    catalog.tombstone_compaction_backing = Some(backing);
                }
                continue;
            }
            let digest = parse_receipt_record_name(name_text)?;
            let mut retained = open_regular_child_nofollow(active_file, &name)
                .map_err(|error| storage_error("open receipt row during recovery", error))?;
            verify_owner_only_acl(&retained)
                .map_err(|error| storage_error("verify recovered receipt row ownership", error))?;
            let identity = file_identity(&retained)
                .map_err(|error| storage_error("identify recovered receipt row", error))?;
            let entry = read_active_record_from_retained(&mut retained, &digest)?;
            check_deadline(deadline)?;
            if let Some(previous) = catalog.records.get(&digest).cloned() {
                if catalog.batch_backing.contains_key(&digest) && previous == entry {
                    catalog.batch_backing.remove(&digest);
                    continue;
                }
                let compact_tombstone_successor = matches!(
                    (&previous.record.lifecycle, &entry.record.lifecycle),
                    (
                        StoredActiveLifecycleV1::DirectTerminalUnacked {
                            terminal_digest: previous_digest,
                            terminal_epoch_ms,
                            ..
                        },
                        StoredActiveLifecycleV1::AcknowledgedTombstone {
                            terminal_digest,
                            acknowledged_at_epoch_ms,
                        }
                    ) if terminal_digest == previous_digest
                        && acknowledged_at_epoch_ms >= terminal_epoch_ms
                );
                if !catalog.batch_backing.contains_key(&digest)
                    || previous.record.key != entry.record.key
                    || (!compact_tombstone_successor
                        && entry.record.mutation_sequence <= previous.record.mutation_sequence)
                    || (!compact_tombstone_successor
                        && entry.record.record_version <= previous.record.record_version)
                {
                    return Err(ReceiptLedgerError::Corrupt(
                        "standalone receipt row does not exactly supersede its batch-backed row",
                    ));
                }
                if !entry.is_tombstone() {
                    if !mutation_sequences.insert(entry.record.mutation_sequence) {
                        return Err(ReceiptLedgerError::Corrupt(
                            "receipt recovery contains a duplicate mutation sequence",
                        ));
                    }
                    maximum_mutation_sequence =
                        maximum_mutation_sequence.max(entry.record.mutation_sequence);
                }
                validate_catalog_replace(&catalog, &previous, &entry)?;
                commit_catalog_replace(&mut catalog, entry);
                catalog.batch_backing.remove(&digest);
                continue;
            }
            if recovered_invocations
                .insert(
                    entry.record.key.invocation_id(),
                    entry.record.key_digest.clone(),
                )
                .is_some()
            {
                return Err(ReceiptLedgerError::Corrupt(
                    "receipt catalog contains a duplicate invocation id",
                ));
            }
            if recovered_tasks
                .insert(
                    entry.record.key.reserved_task_id(),
                    entry.record.key_digest.clone(),
                )
                .is_some()
            {
                return Err(ReceiptLedgerError::Corrupt(
                    "receipt catalog contains a duplicate reserved task id",
                ));
            }
            if !entry.is_tombstone() {
                if !mutation_sequences.insert(entry.record.mutation_sequence) {
                    return Err(ReceiptLedgerError::Corrupt(
                        "receipt recovery contains a duplicate mutation sequence",
                    ));
                }
                maximum_mutation_sequence =
                    maximum_mutation_sequence.max(entry.record.mutation_sequence);
            }
            if entry.is_expired_deletion() {
                if expired_deletion_mutation_sequence
                    .replace(entry.record.mutation_sequence)
                    .is_some()
                {
                    return Err(ReceiptLedgerError::Corrupt(
                        "receipt recovery contains more than one expiry deletion witness",
                    ));
                }
                expired_deletions.push((name, identity, retained));
                continue;
            }
            if entry.is_acknowledgement_commit() {
                if acknowledgement_recovery.is_some() {
                    return Err(ReceiptLedgerError::Corrupt(
                        "receipt recovery contains more than one acknowledgement witness",
                    ));
                }
                let compact_record = build_acknowledged_tombstone_record_from_witness(&entry)?;
                let (compact_record, compact_encoded) =
                    serialize_reserved_record(compact_record, MAX_ACKNOWLEDGED_TOMBSTONE_BYTES)?;
                let compact_entry = CatalogEntry {
                    record: compact_record.clone(),
                    encoded_bytes: u64::try_from(compact_encoded.len())
                        .map_err(|_| ReceiptLedgerError::RecordTooLarge)?,
                };
                insert_catalog_entry(&mut catalog, compact_entry, true)?;
                acknowledgement_recovery = Some(AcknowledgementRecovery {
                    compact_record,
                    compact_encoded,
                    mutation_sequence: entry.record.mutation_sequence,
                });
                continue;
            }
            insert_catalog_entry(&mut catalog, entry, true)?;
        }
        check_deadline(deadline)?;
        verify_recovery_authority(receipts, receipts_file, active, active_file)?;
        check_deadline(deadline)?;
        Ok(RecoveredCatalog {
            catalog,
            maximum_mutation_sequence,
            staging: temporary_entries,
            expired_deletions,
            expired_deletion_mutation_sequence,
            acknowledgement_recovery,
        })
    }

    fn remove_active_staging(
        &self,
        temporary_entries: Vec<RecoveryStagingEntry>,
        deadline: Instant,
    ) -> Result<(), ReceiptLedgerError> {
        check_deadline(deadline)?;
        let mut cleanup_started = false;
        for (name, identity, temporary) in temporary_entries {
            if let Err(error) = check_deadline(deadline) {
                if cleanup_started {
                    sync_recovery_cleanup_directory(&self.active_file).map_err(|sync_error| {
                        storage_error("sync partial abandoned receipt cleanup", sync_error)
                    })?;
                }
                return Err(error);
            }
            cleanup_started = true;
            let removal =
                remove_identity_bound_regular_child(&self.active_file, &name, identity, &temporary);
            drop(temporary);
            if let Err(error) = removal {
                sync_recovery_cleanup_directory(&self.active_file).map_err(|sync_error| {
                    storage_error("sync failed abandoned receipt cleanup", sync_error)
                })?;
                return Err(storage_error(
                    "remove abandoned receipt staging file",
                    error,
                ));
            }
            if let Err(error) = check_deadline(deadline) {
                sync_recovery_cleanup_directory(&self.active_file).map_err(|sync_error| {
                    storage_error("sync partial abandoned receipt cleanup", sync_error)
                })?;
                return Err(error);
            }
        }
        if cleanup_started {
            sync_recovery_cleanup_directory(&self.active_file)
                .map_err(|error| storage_error("sync abandoned receipt cleanup", error))?;
            check_deadline(deadline)?;
        }
        self.verify_named_authority()?;
        check_deadline(deadline)
    }

    fn inspect_generation_staging_before_initialization(
        receipts: &RetainedDirectoryCapability,
        receipts_file: &File,
        deadline: Instant,
    ) -> Result<Vec<RecoveryStagingEntry>, ReceiptLedgerError> {
        check_deadline(deadline)?;
        verify_receipts_authority(receipts, receipts_file)?;
        let names =
            read_directory_names_bounded(receipts_file, MAX_RECEIPT_ROOT_DIRECTORY_ENTRIES, || {
                recovery_checkpoint(deadline)
            })
            .map_err(|error| recovery_error("enumerate receipt root directory", error))?;
        let mut staging = Vec::new();
        for name in names {
            check_deadline(deadline)?;
            let Some(name_text) = name.to_str() else {
                return Err(ReceiptLedgerError::Corrupt(
                    "receipt root entry name is not UTF-8",
                ));
            };
            if matches!(
                name_text,
                ACTIVE_DIRECTORY_NAME | GENERATION_FILE_NAME | LEDGER_LOCK_FILE_NAME
            ) {
                continue;
            }
            if !parse_generation_temporary_name(name_text)?
                && !parse_cleanup_quarantine_name(name_text)?
            {
                return Err(ReceiptLedgerError::Corrupt(
                    "receipt root entry has an unsupported name",
                ));
            }
            if staging.len() >= MAX_GENERATION_STAGING_ENTRIES {
                return Err(ReceiptLedgerError::Corrupt(
                    "receipt root exceeds the generation staging limit",
                ));
            }
            let file = open_regular_child_nofollow(receipts_file, &name)
                .map_err(|error| storage_error("open abandoned generation staging", error))?;
            verify_owner_only_acl(&file).map_err(|error| {
                storage_error("verify abandoned generation staging ownership", error)
            })?;
            let identity = file_identity(&file)
                .map_err(|error| storage_error("identify abandoned generation staging", error))?;
            staging.push((name, identity, file));
            check_deadline(deadline)?;
        }
        verify_receipts_authority(receipts, receipts_file)?;
        check_deadline(deadline)?;
        Ok(staging)
    }

    fn remove_generation_staging(
        &self,
        staging: Vec<(OsString, FileIdentity, File)>,
        deadline: Instant,
    ) -> Result<(), ReceiptLedgerError> {
        check_deadline(deadline)?;
        let mut cleanup_started = false;
        for (name, identity, file) in staging {
            if let Err(error) = check_deadline(deadline) {
                if cleanup_started {
                    sync_recovery_cleanup_directory(&self.receipts_file).map_err(|sync_error| {
                        storage_error("sync partial generation staging cleanup", sync_error)
                    })?;
                }
                return Err(error);
            }
            cleanup_started = true;
            let removal =
                remove_identity_bound_regular_child(&self.receipts_file, &name, identity, &file);
            drop(file);
            if let Err(error) = removal {
                sync_recovery_cleanup_directory(&self.receipts_file).map_err(|sync_error| {
                    storage_error("sync failed generation staging cleanup", sync_error)
                })?;
                return Err(storage_error("remove abandoned generation staging", error));
            }
            if let Err(error) = check_deadline(deadline) {
                sync_recovery_cleanup_directory(&self.receipts_file).map_err(|sync_error| {
                    storage_error("sync partial generation staging cleanup", sync_error)
                })?;
                return Err(error);
            }
        }
        if cleanup_started {
            sync_recovery_cleanup_directory(&self.receipts_file)
                .map_err(|error| storage_error("sync generation staging cleanup", error))?;
            check_deadline(deadline)?;
        }
        self.verify_named_authority()?;
        check_deadline(deadline)
    }

    fn read_active_record(
        &self,
        receipt_key_digest: &ReceiptKeyDigest,
    ) -> Result<Option<CatalogEntry>, ReceiptLedgerError> {
        read_active_record_from(&self.active_file, receipt_key_digest)
    }

    fn verify_receipt_batch_backing(
        &self,
        backing: &ReceiptBatchBacking,
    ) -> Result<(), ReceiptLedgerError> {
        let mut file = open_regular_child_nofollow(&self.active_file, OsStr::new(&backing.name))
            .map_err(|error| storage_error("open retained receipt batch", error))?;
        verify_owner_only_acl(&file)
            .map_err(|error| storage_error("verify retained receipt batch ownership", error))?;
        if file_identity(&file)
            .map_err(|error| storage_error("identify retained receipt batch", error))?
            != backing.identity
        {
            return Err(ReceiptLedgerError::Corrupt(
                "receipt batch identity changed after publication",
            ));
        }
        let mut encoded = Vec::new();
        Read::by_ref(&mut file)
            .take(MAX_RECEIPT_BATCH_BYTES + 1)
            .read_to_end(&mut encoded)
            .map_err(|error| storage_error("read retained receipt batch", error))?;
        if encoded != *backing.encoded {
            return Err(ReceiptLedgerError::Corrupt(
                "receipt batch bytes changed after publication",
            ));
        }
        Ok(())
    }

    fn materialize_batch_backed_record(
        &self,
        catalog: &mut ReceiptCatalog,
        receipt_key_digest: &ReceiptKeyDigest,
        deadline: Instant,
    ) -> Result<(), ReceiptLedgerError> {
        let Some(backing) = catalog.batch_backing.get(receipt_key_digest).cloned() else {
            return Ok(());
        };
        self.verify_receipt_batch_backing(&backing)?;
        let entry =
            catalog
                .records
                .get(receipt_key_digest)
                .cloned()
                .ok_or(ReceiptLedgerError::Corrupt(
                    "batch backing has no catalogued receipt row",
                ))?;
        let batch = decode_receipt_batch(backing.encoded.as_slice())?;
        let row = batch
            .rows
            .into_iter()
            .find(|row| row.record.key_digest == *receipt_key_digest)
            .ok_or(ReceiptLedgerError::Corrupt(
                "batch backing does not contain its catalogued receipt row",
            ))?;
        if row.record != entry.record || row.encoded.len() as u64 != entry.encoded_bytes {
            return Err(ReceiptLedgerError::Corrupt(
                "batch-backed receipt row changed record or encoded size",
            ));
        }
        self.publish_new_record(&entry.record, &row.encoded, deadline, || {})?;
        match self.read_active_record_bytes(receipt_key_digest) {
            Ok(Some(committed)) if committed == row.encoded => {}
            Ok(Some(_)) | Ok(None) | Err(_) => {
                catalog.unavailable = true;
                return Err(ReceiptLedgerError::CommitUncertain {
                    receipt_key_digest: receipt_key_digest.clone(),
                });
            }
        }
        catalog.batch_backing.remove(receipt_key_digest);
        Ok(())
    }

    fn read_active_record_bytes(
        &self,
        receipt_key_digest: &ReceiptKeyDigest,
    ) -> Result<Option<Vec<u8>>, ReceiptLedgerError> {
        read_active_record_bytes_from(&self.active_file, receipt_key_digest)
    }

    fn remove_expired_deletion_witness(
        &self,
        receipt_key_digest: &ReceiptKeyDigest,
        expected_bytes: &[u8],
        deadline: Instant,
    ) -> Result<(), ReceiptLedgerError> {
        let uncertain = || ReceiptLedgerError::CommitUncertain {
            receipt_key_digest: receipt_key_digest.clone(),
        };
        if check_deadline(deadline).is_err() || self.verify_named_authority().is_err() {
            return Err(uncertain());
        }
        let name = format!("{}.json", receipt_key_digest.as_str());
        let name = OsStr::new(&name);
        let mut witness =
            open_regular_child_nofollow(&self.active_file, name).map_err(|_| uncertain())?;
        verify_owner_only_acl(&witness).map_err(|_| uncertain())?;
        let identity = file_identity(&witness).map_err(|_| uncertain())?;
        let actual_bytes =
            read_active_record_bytes_from_retained(&mut witness).map_err(|_| uncertain())?;
        if actual_bytes != expected_bytes {
            return Err(uncertain());
        }
        if check_deadline(deadline).is_err() || self.verify_named_authority().is_err() {
            return Err(uncertain());
        }
        remove_identity_bound_regular_child(&self.active_file, name, identity, &witness)
            .map_err(|_| uncertain())?;
        drop(witness);
        #[cfg(test)]
        run_after_expired_deletion_witness_remove_hook_for_test();
        if sync_receipt_row_directory(&self.active_file).is_err()
            || check_deadline(deadline).is_err()
            || self.verify_named_authority().is_err()
        {
            return Err(uncertain());
        }
        Ok(())
    }

    fn publish_record_batch(
        &self,
        catalog: &mut ReceiptCatalog,
        rows: &[ReceiptBatchRow],
        final_generation: u64,
        deadline: Instant,
        commit: impl FnOnce(&mut ReceiptCatalog),
    ) -> Result<(), ReceiptLedgerError> {
        if rows.is_empty() || rows.len() > MAX_RECEIPT_BATCH_ENVELOPE_ROWS {
            return Err(ReceiptLedgerError::ReceiptRowPresentUnsupported);
        }
        let mut superseded_backings = Vec::new();
        for row in rows {
            if let Some(backing) = catalog.batch_backing.get(&row.record.key_digest) {
                if !superseded_backings.contains(backing) {
                    superseded_backings.push(backing.clone());
                }
            }
        }
        check_deadline(deadline)?;
        self.verify_named_authority()?;
        for row in rows {
            catalog_entry_from_batch_record(row.record.clone(), &row.encoded)?;
        }
        let encoded = encode_receipt_batch_envelope(rows)?;
        let batch_id = Uuid::new_v4();
        let temporary_name = format!(".receipt.{batch_id}.tmp");
        let final_name = format!("receipt-batch.{batch_id}.json");
        let mut file = create_owner_only_file_child(&self.active_file, OsStr::new(&temporary_name))
            .map_err(|error| storage_error("create receipt batch staging file", error))?;
        let identity = file_identity(&file).map_err(|_| ReceiptLedgerError::StoreUnavailable)?;
        if let Err(error) = file.write_all(&encoded).and_then(|()| file.sync_all()) {
            let _ = cleanup_staged_file(
                &self.active_file,
                OsStr::new(&temporary_name),
                identity,
                &file,
            );
            return Err(storage_error("persist receipt batch staging file", error));
        }
        if check_deadline(deadline).is_err() {
            let _ = cleanup_staged_file(
                &self.active_file,
                OsStr::new(&temporary_name),
                identity,
                &file,
            );
            return Err(ReceiptLedgerError::DeadlineExceeded);
        }
        if let Err(error) = rename_identity_bound_regular_child_no_replace(
            &self.active_file,
            OsStr::new(&temporary_name),
            identity,
            &file,
            &self.active_file,
            OsStr::new(&final_name),
        ) {
            let _ = cleanup_staged_file(
                &self.active_file,
                OsStr::new(&temporary_name),
                identity,
                &file,
            );
            return Err(storage_error("publish receipt batch", error));
        }
        let first_digest = rows[0].record.key_digest.clone();
        if sync_receipt_row_directory(&self.active_file).is_err()
            || check_deadline(deadline).is_err()
        {
            catalog.unavailable = true;
            return Err(ReceiptLedgerError::CommitUncertain {
                receipt_key_digest: first_digest,
            });
        }
        let backing = ReceiptBatchBacking {
            name: final_name,
            identity,
            encoded: Arc::new(encoded),
        };
        commit(catalog);
        for row in rows {
            catalog
                .batch_backing
                .insert(row.record.key_digest.clone(), backing.clone());
        }
        if rows.iter().all(|row| {
            matches!(
                &row.record.lifecycle,
                StoredActiveLifecycleV1::AcknowledgedTombstone { .. }
            )
        }) {
            catalog.tombstone_compaction_backing =
                (rows.len() < MAX_RECEIPT_BATCH_ENVELOPE_ROWS).then_some(backing.clone());
        }
        self.generation_highwater
            .fetch_max(final_generation, Ordering::AcqRel);
        if self.verify_receipt_batch_backing(&backing).is_err() {
            catalog.unavailable = true;
            return Err(ReceiptLedgerError::CommitUncertain {
                receipt_key_digest: first_digest,
            });
        }
        if check_deadline(deadline).is_err() || self.verify_named_authority().is_err() {
            catalog.unavailable = true;
            return Err(ReceiptLedgerError::CommitUncertain {
                receipt_key_digest: first_digest,
            });
        }
        let obsolete_backings = superseded_backings
            .into_iter()
            .filter(|superseded| {
                !catalog
                    .batch_backing
                    .values()
                    .any(|current| current == superseded)
            })
            .collect::<Vec<_>>();
        if self
            .retire_receipt_batch_backings(&obsolete_backings, deadline)
            .is_err()
        {
            catalog.unavailable = true;
            return Err(ReceiptLedgerError::CommitUncertain {
                receipt_key_digest: first_digest,
            });
        }
        Ok(())
    }

    fn retire_receipt_batch_backings(
        &self,
        backings: &[ReceiptBatchBacking],
        deadline: Instant,
    ) -> Result<(), ReceiptLedgerError> {
        if backings.is_empty() {
            return Ok(());
        }
        for backing in backings {
            check_deadline(deadline)?;
            if parse_receipt_batch_name(&backing.name)?.is_none() {
                return Err(ReceiptLedgerError::Corrupt(
                    "receipt batch backing has a non-batch retained name",
                ));
            }
            let name = OsStr::new(&backing.name);
            let mut retained = open_regular_child_nofollow(&self.active_file, name)
                .map_err(|error| storage_error("open superseded receipt batch", error))?;
            verify_owner_only_acl(&retained).map_err(|error| {
                storage_error("verify superseded receipt batch ownership", error)
            })?;
            let identity = file_identity(&retained)
                .map_err(|error| storage_error("identify superseded receipt batch", error))?;
            if identity != backing.identity {
                return Err(ReceiptLedgerError::Corrupt(
                    "superseded receipt batch identity changed before retirement",
                ));
            }
            let mut encoded = Vec::new();
            Read::by_ref(&mut retained)
                .take(MAX_RECEIPT_BATCH_BYTES + 1)
                .read_to_end(&mut encoded)
                .map_err(|error| storage_error("read superseded receipt batch", error))?;
            if encoded != *backing.encoded {
                return Err(ReceiptLedgerError::Corrupt(
                    "superseded receipt batch bytes changed before retirement",
                ));
            }
            remove_identity_bound_regular_child(
                &self.active_file,
                name,
                backing.identity,
                &retained,
            )
            .map_err(|error| storage_error("remove superseded receipt batch", error))?;
        }
        sync_receipt_row_directory(&self.active_file)
            .map_err(|error| storage_error("sync retired receipt batches", error))?;
        check_deadline(deadline)?;
        self.verify_named_authority()
    }

    fn publish_new_record(
        &self,
        record: &StoredActiveReceiptV1,
        encoded: &[u8],
        deadline: Instant,
        on_visible: impl FnOnce(),
    ) -> Result<(), ReceiptLedgerError> {
        check_deadline(deadline)?;
        self.verify_named_authority()?;
        let temporary_name = format!(".receipt.{}.tmp", Uuid::new_v4());
        let temporary_name = OsStr::new(&temporary_name);
        let mut file = create_owner_only_file_child(&self.active_file, temporary_name)
            .map_err(|error| storage_error("create owner-only receipt staging file", error))?;
        let temporary_identity = file_identity(&file).map_err(|_| {
            // The file already exists but cannot be bound to an identity for
            // safe cleanup. Reopen owns the only admissible recovery path.
            ReceiptLedgerError::StoreUnavailable
        })?;
        if let Err(error) = file.write_all(encoded).and_then(|()| file.sync_all()) {
            if cleanup_staged_file(&self.active_file, temporary_name, temporary_identity, &file)
                .is_err()
            {
                return Err(ReceiptLedgerError::StoreUnavailable);
            }
            return Err(storage_error("persist receipt staging file", error));
        }
        if check_deadline(deadline).is_err() {
            if cleanup_staged_file(&self.active_file, temporary_name, temporary_identity, &file)
                .is_err()
            {
                return Err(ReceiptLedgerError::StoreUnavailable);
            }
            return Err(ReceiptLedgerError::DeadlineExceeded);
        }
        let target_name = format!("{}.json", record.key_digest.as_str());
        if let Err(error) = rename_identity_bound_regular_child_no_replace(
            &self.active_file,
            temporary_name,
            temporary_identity,
            &file,
            &self.active_file,
            OsStr::new(&target_name),
        ) {
            if cleanup_staged_file(&self.active_file, temporary_name, temporary_identity, &file)
                .is_err()
            {
                return Err(ReceiptLedgerError::StoreUnavailable);
            }
            return Err(storage_error("atomically publish receipt row", error));
        }
        on_visible();
        #[cfg(test)]
        run_after_receipt_row_rename_hook_for_test();
        if sync_receipt_row_directory(&self.active_file).is_err() {
            return Err(ReceiptLedgerError::CommitUncertain {
                receipt_key_digest: record.key_digest.clone(),
            });
        }
        if check_deadline(deadline).is_err() {
            return Err(ReceiptLedgerError::CommitUncertain {
                receipt_key_digest: record.key_digest.clone(),
            });
        }
        Ok(())
    }

    fn publish_replacement_record(
        &self,
        record: &StoredActiveReceiptV1,
        encoded: &[u8],
        deadline: Instant,
        on_visible: impl FnOnce(),
    ) -> Result<(), ReceiptLedgerError> {
        check_deadline(deadline)?;
        self.verify_named_authority()?;
        let temporary_name = format!(".receipt.{}.tmp", Uuid::new_v4());
        let temporary_name = OsStr::new(&temporary_name);
        let mut file = create_owner_only_file_child(&self.active_file, temporary_name)
            .map_err(|error| storage_error("create owner-only receipt staging file", error))?;
        let temporary_identity =
            file_identity(&file).map_err(|_| ReceiptLedgerError::StoreUnavailable)?;
        if let Err(error) = file.write_all(encoded).and_then(|()| file.sync_all()) {
            if cleanup_staged_file(&self.active_file, temporary_name, temporary_identity, &file)
                .is_err()
            {
                return Err(ReceiptLedgerError::StoreUnavailable);
            }
            return Err(storage_error(
                "persist receipt replacement staging file",
                error,
            ));
        }
        if check_deadline(deadline).is_err() {
            if cleanup_staged_file(&self.active_file, temporary_name, temporary_identity, &file)
                .is_err()
            {
                return Err(ReceiptLedgerError::StoreUnavailable);
            }
            return Err(ReceiptLedgerError::DeadlineExceeded);
        }
        let target_name = format!("{}.json", record.key_digest.as_str());
        if let Err(error) = replace_identity_bound_regular_child(
            &self.active_file,
            temporary_name,
            temporary_identity,
            &file,
            OsStr::new(&target_name),
        ) {
            if cleanup_staged_file(&self.active_file, temporary_name, temporary_identity, &file)
                .is_err()
            {
                return Err(ReceiptLedgerError::StoreUnavailable);
            }
            return Err(storage_error("atomically replace receipt row", error));
        }
        on_visible();
        #[cfg(test)]
        run_after_receipt_row_rename_hook_for_test();
        if sync_receipt_row_directory(&self.active_file).is_err()
            || check_deadline(deadline).is_err()
        {
            return Err(ReceiptLedgerError::CommitUncertain {
                receipt_key_digest: record.key_digest.clone(),
            });
        }
        Ok(())
    }

    fn publish_generation(
        &self,
        next_generation: u64,
        receipt_key_digest: Option<&ReceiptKeyDigest>,
        deadline: Option<Instant>,
    ) -> Result<(), ReceiptLedgerError> {
        if deadline.is_some_and(|deadline| check_deadline(deadline).is_err()) {
            return Err(generation_deadline_error(receipt_key_digest));
        }
        self.verify_named_authority()
            .map_err(|error| after_row_error(receipt_key_digest, error))?;
        let temporary_name = format!(".generation.{}.tmp", Uuid::new_v4());
        let temporary_name = OsStr::new(&temporary_name);
        let mut file =
            create_owner_only_file_child(&self.receipts_file, temporary_name).map_err(|error| {
                after_row_error(
                    receipt_key_digest,
                    storage_error("create receipt generation staging file", error),
                )
            })?;
        let temporary_identity = file_identity(&file).map_err(|_| {
            after_row_error(receipt_key_digest, ReceiptLedgerError::StoreUnavailable)
        })?;
        let encoded = format!("{next_generation}\n");
        if let Err(error) = file
            .write_all(encoded.as_bytes())
            .and_then(|()| file.sync_all())
        {
            if cleanup_staged_file(
                &self.receipts_file,
                temporary_name,
                temporary_identity,
                &file,
            )
            .is_err()
            {
                return Err(after_row_error(
                    receipt_key_digest,
                    ReceiptLedgerError::StoreUnavailable,
                ));
            }
            return Err(after_row_error(
                receipt_key_digest,
                storage_error("persist receipt generation staging file", error),
            ));
        }
        if deadline.is_some_and(|deadline| check_deadline(deadline).is_err()) {
            if cleanup_staged_file(
                &self.receipts_file,
                temporary_name,
                temporary_identity,
                &file,
            )
            .is_err()
            {
                return Err(after_row_error(
                    receipt_key_digest,
                    ReceiptLedgerError::StoreUnavailable,
                ));
            }
            return Err(generation_deadline_error(receipt_key_digest));
        }
        if let Err(error) = replace_identity_bound_regular_child(
            &self.receipts_file,
            temporary_name,
            temporary_identity,
            &file,
            OsStr::new(GENERATION_FILE_NAME),
        ) {
            if cleanup_staged_file(
                &self.receipts_file,
                temporary_name,
                temporary_identity,
                &file,
            )
            .is_err()
            {
                return Err(after_row_error(
                    receipt_key_digest,
                    ReceiptLedgerError::StoreUnavailable,
                ));
            }
            return Err(after_row_error(
                receipt_key_digest,
                storage_error("replace receipt generation record", error),
            ));
        }
        #[cfg(test)]
        run_after_generation_replace_hook_for_test();
        if sync_directory(&self.receipts_file).is_err() {
            return Err(commit_or_storage_error(
                receipt_key_digest,
                "receipt generation commit could not be confirmed",
            ));
        }
        let capability = self
            .receipts
            .retain_regular_child(OsStr::new(GENERATION_FILE_NAME))
            .map_err(|error| {
                after_row_error(
                    receipt_key_digest,
                    storage_error("retain replaced generation record", error),
                )
            })?;
        if capability.identity() != temporary_identity
            || file_identity(&file).map_err(|error| {
                after_row_error(
                    receipt_key_digest,
                    storage_error("identify replaced generation record", error),
                )
            })? != temporary_identity
        {
            return Err(commit_or_storage_error(
                receipt_key_digest,
                "receipt generation identity changed after replacement",
            ));
        }
        *self.generation.lock().map_err(|_| {
            after_row_error(
                receipt_key_digest,
                ReceiptLedgerError::Corrupt("generation writer lock was poisoned"),
            )
        })? = GenerationState { capability, file };
        self.verify_named_authority().map_err(|error| {
            if let Some(digest) = receipt_key_digest {
                ReceiptLedgerError::CommitUncertain {
                    receipt_key_digest: digest.clone(),
                }
            } else {
                error
            }
        })?;
        if deadline.is_some_and(|deadline| check_deadline(deadline).is_err()) {
            return Err(generation_deadline_error(receipt_key_digest));
        }
        self.generation_highwater
            .fetch_max(next_generation, Ordering::AcqRel);
        Ok(())
    }

    pub(crate) fn inspect_exact(
        &self,
        receipt_key_digest: &ReceiptKeyDigest,
    ) -> Result<MissingReceiptObservation, ReceiptLedgerError> {
        self.inspect_exact_after_row_lookup(receipt_key_digest, || {})
    }

    fn inspect_exact_after_row_lookup(
        &self,
        receipt_key_digest: &ReceiptKeyDigest,
        after_row_lookup: impl FnOnce(),
    ) -> Result<MissingReceiptObservation, ReceiptLedgerError> {
        let mut catalog = self
            .writer
            .lock()
            .map_err(|_| ReceiptLedgerError::Corrupt("receipt catalog lock was poisoned"))?;
        if catalog.unavailable {
            return Err(ReceiptLedgerError::StoreUnavailable);
        }
        let catalogued = catalog.records.contains_key(receipt_key_digest);
        latch_catalog_result(&mut catalog, self.verify_named_authority())?;
        let generation_before =
            latch_catalog_result(&mut catalog, self.generation_under_writer_lock())?;
        let record_name = format!("{}.json", receipt_key_digest.as_str());
        let row_present =
            match open_regular_child_nofollow(&self.active_file, OsStr::new(&record_name)) {
                Ok(record) => {
                    if let Err(error) = verify_owner_only_acl(&record) {
                        return latch_catalog_error(
                            &mut catalog,
                            storage_error("verify receipt row ownership", error),
                        );
                    }
                    true
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => false,
                Err(error) => {
                    return latch_catalog_error(
                        &mut catalog,
                        storage_error("inspect exact receipt row", error),
                    )
                }
            };
        after_row_lookup();
        let generation_after =
            latch_catalog_result(&mut catalog, self.generation_under_writer_lock())?;
        if generation_after != generation_before {
            return latch_catalog_error(
                &mut catalog,
                ReceiptLedgerError::ConcurrentGenerationChange {
                    generation_before,
                    generation_after,
                },
            );
        }
        latch_catalog_result(&mut catalog, self.verify_named_authority())?;
        match (catalogued, row_present) {
            (true, false) => {
                return latch_catalog_error(
                    &mut catalog,
                    ReceiptLedgerError::Corrupt("catalogued receipt row is missing"),
                )
            }
            (false, true) => {
                return latch_catalog_error(
                    &mut catalog,
                    ReceiptLedgerError::Corrupt(
                        "receipt row is present outside the recovered catalog",
                    ),
                )
            }
            (true, true) => return Err(ReceiptLedgerError::ReceiptRowPresentUnsupported),
            (false, false) => {}
        }
        Ok(MissingReceiptObservation {
            receipt_key_digest: receipt_key_digest.clone(),
            generation_before,
            generation_after,
        })
    }

    fn verify_named_authority(&self) -> Result<(), ReceiptLedgerError> {
        self.receipts
            .validate_named_identity()
            .map_err(|error| storage_error("validate named receipts directory", error))?;
        self.active
            .validate_named_identity()
            .map_err(|error| storage_error("validate named receipt active directory", error))?;
        verify_owner_only_acl(&self.receipts_file)
            .map_err(|error| storage_error("verify receipts directory ownership", error))?;
        verify_owner_only_acl(&self.active_file)
            .map_err(|error| storage_error("verify receipt active directory ownership", error))?;
        let generation = self
            .generation
            .lock()
            .map_err(|_| ReceiptLedgerError::Corrupt("generation reader lock was poisoned"))?;
        generation
            .capability
            .validate_named_identity()
            .map_err(|error| storage_error("validate named generation record", error))?;
        verify_owner_only_acl(&generation.file)
            .map_err(|error| storage_error("verify generation record ownership", error))
    }
}

fn verify_receipts_authority(
    receipts: &RetainedDirectoryCapability,
    receipts_file: &File,
) -> Result<(), ReceiptLedgerError> {
    receipts
        .validate_named_identity()
        .map_err(|error| storage_error("validate named receipts directory", error))?;
    verify_owner_only_acl(receipts_file)
        .map_err(|error| storage_error("verify receipts directory ownership", error))
}

fn verify_recovery_authority(
    receipts: &RetainedDirectoryCapability,
    receipts_file: &File,
    active: &RetainedDirectoryCapability,
    active_file: &File,
) -> Result<(), ReceiptLedgerError> {
    verify_receipts_authority(receipts, receipts_file)?;
    active
        .validate_named_identity()
        .map_err(|error| storage_error("validate named receipt active directory", error))?;
    verify_owner_only_acl(active_file)
        .map_err(|error| storage_error("verify receipt active directory ownership", error))
}

fn read_active_record_from(
    active_file: &File,
    receipt_key_digest: &ReceiptKeyDigest,
) -> Result<Option<CatalogEntry>, ReceiptLedgerError> {
    let record_name = format!("{}.json", receipt_key_digest.as_str());
    let mut file = match open_regular_child_nofollow(active_file, OsStr::new(&record_name)) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(storage_error("open active receipt row", error)),
    };
    verify_owner_only_acl(&file)
        .map_err(|error| storage_error("verify active receipt row ownership", error))?;
    read_active_record_from_retained(&mut file, receipt_key_digest).map(Some)
}

fn read_active_record_from_retained(
    file: &mut File,
    receipt_key_digest: &ReceiptKeyDigest,
) -> Result<CatalogEntry, ReceiptLedgerError> {
    let bytes = read_active_record_bytes_from_retained(file)?;
    let record: StoredActiveReceiptV1 = match serde_json::from_slice(&bytes) {
        Ok(record) => record,
        Err(_) => {
            let tombstone: StoredAcknowledgedTombstoneV1 =
                serde_json::from_slice(&bytes).map_err(|_| {
                    ReceiptLedgerError::Corrupt("receipt row is not a strict supported JSON record")
                })?;
            let record_version = tombstone
                .record_version
                .unwrap_or(ReceiptVersion::new(3).expect("tombstone marker version is nonzero"));
            StoredActiveReceiptV1 {
                schema_version: RECEIPT_RECORD_SCHEMA_VERSION,
                mutation_sequence: 0,
                record_version,
                key_digest: crate::application::receipt_ledger::receipt_key_digest(&tombstone.key),
                key: tombstone.key,
                lifecycle: StoredActiveLifecycleV1::AcknowledgedTombstone {
                    terminal_digest: tombstone.terminal_digest,
                    acknowledged_at_epoch_ms: tombstone.ack_epoch_ms,
                },
            }
        }
    };
    validate_active_record(&record, &bytes, receipt_key_digest)?;
    if matches!(
        &record.lifecycle,
        StoredActiveLifecycleV1::CancelReserved { .. }
            | StoredActiveLifecycleV1::ExpiredDeletion { .. }
            | StoredActiveLifecycleV1::ExpiredTombstoneDeletion { .. }
            | StoredActiveLifecycleV1::ExpiredDirectDeletion { .. }
            | StoredActiveLifecycleV1::CompletedTaskHandoffDeletion { .. }
            | StoredActiveLifecycleV1::ReservedUnbound { .. }
            | StoredActiveLifecycleV1::ReservedActorBound { .. }
            | StoredActiveLifecycleV1::ReservedBegun { .. }
            | StoredActiveLifecycleV1::TaskPromisedUnbound { .. }
            | StoredActiveLifecycleV1::TaskPromisedActorBound { .. }
            | StoredActiveLifecycleV1::TaskHandoffActorBound { .. }
            | StoredActiveLifecycleV1::TaskReceiptOwnedActorBound { .. }
            | StoredActiveLifecycleV1::TaskTerminalReceiptBacked { .. }
            | StoredActiveLifecycleV1::AcknowledgementCommit { .. }
            | StoredActiveLifecycleV1::AcknowledgedTombstone { .. }
    ) {
        validate_persisted_reserved_record_bytes(&record, &bytes)?;
    }
    Ok(CatalogEntry {
        record,
        encoded_bytes: u64::try_from(bytes.len()).map_err(|_| {
            ReceiptLedgerError::Corrupt("persisted receipt row byte count exceeds u64")
        })?,
    })
}

fn catalog_entry_from_batch_record(
    record: StoredActiveReceiptV1,
    bytes: &[u8],
) -> Result<CatalogEntry, ReceiptLedgerError> {
    let digest = record.key_digest.clone();
    validate_active_record(&record, bytes, &digest)?;
    if !matches!(
        &record.lifecycle,
        StoredActiveLifecycleV1::DirectTerminalUnacked { .. }
    ) {
        validate_persisted_reserved_record_bytes(&record, bytes)?;
    }
    Ok(CatalogEntry {
        record,
        encoded_bytes: u64::try_from(bytes.len()).map_err(|_| {
            ReceiptLedgerError::Corrupt("persisted receipt batch row byte count exceeds u64")
        })?,
    })
}

fn encode_receipt_batch_envelope(rows: &[ReceiptBatchRow]) -> Result<Vec<u8>, ReceiptLedgerError> {
    if rows.is_empty() || rows.len() > MAX_RECEIPT_BATCH_ENVELOPE_ROWS {
        return Err(ReceiptLedgerError::Corrupt(
            "receipt batch has an unsupported row count",
        ));
    }
    let mut encoded = Vec::new();
    encoded.extend_from_slice(b"{\"schemaVersion\":1,\"rows\":[");
    for (index, row) in rows.iter().enumerate() {
        if index != 0 {
            encoded.push(b',');
        }
        encoded.extend_from_slice(b"{\"mutationSequence\":");
        encoded.extend_from_slice(row.record.mutation_sequence.to_string().as_bytes());
        encoded.extend_from_slice(b",\"recordVersion\":");
        encoded.extend_from_slice(row.record.record_version.get().to_string().as_bytes());
        encoded.extend_from_slice(b",\"persistedHex\":\"");
        append_lower_hex(&mut encoded, &row.encoded);
        encoded.extend_from_slice(b"\"}");
        if encoded.len() as u64 > MAX_RECEIPT_BATCH_BYTES {
            return Err(ReceiptLedgerError::RecordTooLarge);
        }
    }
    encoded.extend_from_slice(b"]}");
    if encoded.len() as u64 > MAX_RECEIPT_BATCH_BYTES {
        return Err(ReceiptLedgerError::RecordTooLarge);
    }
    Ok(encoded)
}

fn append_lower_hex(target: &mut Vec<u8>, bytes: &[u8]) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    target.reserve(bytes.len().saturating_mul(2));
    for byte in bytes {
        target.push(HEX[(byte >> 4) as usize]);
        target.push(HEX[(byte & 0x0f) as usize]);
    }
}

fn decode_lower_hex(encoded: &str) -> Result<Vec<u8>, ReceiptLedgerError> {
    if !encoded.len().is_multiple_of(2) {
        return Err(ReceiptLedgerError::Corrupt(
            "receipt batch row is not canonical lowercase hex",
        ));
    }
    let mut decoded = Vec::with_capacity(encoded.len() / 2);
    for pair in encoded.as_bytes().as_chunks::<2>().0 {
        let high = lower_hex_nibble(pair[0]).ok_or(ReceiptLedgerError::Corrupt(
            "receipt batch row is not canonical lowercase hex",
        ))?;
        let low = lower_hex_nibble(pair[1]).ok_or(ReceiptLedgerError::Corrupt(
            "receipt batch row is not canonical lowercase hex",
        ))?;
        decoded.push((high << 4) | low);
    }
    Ok(decoded)
}

const fn lower_hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

fn read_active_record_bytes_from(
    active_file: &File,
    receipt_key_digest: &ReceiptKeyDigest,
) -> Result<Option<Vec<u8>>, ReceiptLedgerError> {
    let record_name = format!("{}.json", receipt_key_digest.as_str());
    let mut file = match open_regular_child_nofollow(active_file, OsStr::new(&record_name)) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(storage_error("open active receipt row", error)),
    };
    verify_owner_only_acl(&file)
        .map_err(|error| storage_error("verify active receipt row ownership", error))?;
    read_active_record_bytes_from_retained(&mut file).map(Some)
}

fn read_active_record_bytes_from_retained(file: &mut File) -> Result<Vec<u8>, ReceiptLedgerError> {
    file.seek(SeekFrom::Start(0))
        .map_err(|error| storage_error("rewind active receipt row", error))?;
    let mut bytes = Vec::new();
    Read::by_ref(file)
        .take(MAX_RECEIPT_ENTITLEMENT_BYTES.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|error| storage_error("read active receipt row", error))?;
    if u64::try_from(bytes.len()).map_or(true, |length| length > MAX_RECEIPT_ENTITLEMENT_BYTES) {
        return Err(ReceiptLedgerError::Corrupt(
            "persisted receipt row exceeds its byte limit",
        ));
    }
    Ok(bytes)
}

fn open_or_create_owner_only_directory(path: &Path) -> Result<File, ReceiptLedgerError> {
    if !path.is_absolute() {
        return Err(ReceiptLedgerError::Storage {
            operation: "open receipts directory",
            message: "receipt ledger path must be absolute".to_string(),
        });
    }
    let parent_path = path.parent().ok_or_else(|| ReceiptLedgerError::Storage {
        operation: "open receipts directory",
        message: "receipt ledger path has no parent".to_string(),
    })?;
    let name = path
        .file_name()
        .ok_or_else(|| ReceiptLedgerError::Storage {
            operation: "open receipts directory",
            message: "receipt ledger path has no final component".to_string(),
        })?;
    if !matches!(path.components().next_back(), Some(Component::Normal(_))) {
        return Err(ReceiptLedgerError::Storage {
            operation: "open receipts directory",
            message: "receipt ledger path must end in one normal component".to_string(),
        });
    }
    let parent = open_absolute_directory_path_nofollow(parent_path)
        .map_err(|error| storage_error("open receipt ledger parent", error))?;
    let directory = match open_directory_child_nofollow(&parent, name) {
        Ok(directory) => directory,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            match create_owner_only_directory_child(&parent, name) {
                Ok(directory) => {
                    sync_directory(&parent).map_err(|error| {
                        storage_error("sync receipt ledger directory creation", error)
                    })?;
                    directory
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                    open_directory_child_nofollow(&parent, name)
                        .map_err(|error| storage_error("open raced receipts directory", error))?
                }
                Err(error) => {
                    return Err(storage_error("create owner-only receipts directory", error))
                }
            }
        }
        Err(error) => return Err(storage_error("open receipts directory no-follow", error)),
    };
    verify_owner_only_acl(&directory)
        .map_err(|error| storage_error("verify receipts directory ownership", error))?;
    Ok(directory)
}

fn open_or_create_owner_only_child(
    parent: &File,
    name: &'static str,
) -> Result<File, ReceiptLedgerError> {
    let name = OsStr::new(name);
    let directory = match open_directory_child_nofollow(parent, name) {
        Ok(directory) => directory,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            match create_owner_only_directory_child(parent, name) {
                Ok(directory) => {
                    sync_directory(parent)
                        .map_err(|error| storage_error("sync receipt subdirectory", error))?;
                    directory
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                    open_directory_child_nofollow(parent, name)
                        .map_err(|error| storage_error("open raced receipt subdirectory", error))?
                }
                Err(error) => {
                    return Err(storage_error(
                        "create owner-only receipt subdirectory",
                        error,
                    ))
                }
            }
        }
        Err(error) => return Err(storage_error("open receipt subdirectory no-follow", error)),
    };
    verify_owner_only_acl(&directory)
        .map_err(|error| storage_error("verify receipt subdirectory ownership", error))?;
    Ok(directory)
}

fn open_or_initialize_generation(
    receipts: &File,
    initial_generation: u64,
    deadline: Instant,
) -> Result<(File, u64), ReceiptLedgerError> {
    check_deadline(deadline)?;
    let name = OsStr::new(GENERATION_FILE_NAME);
    let generation = match open_regular_child_nofollow(receipts, name) {
        Ok(generation) => generation,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            check_deadline(deadline)?;
            let temporary_name = format!(".generation.{}.tmp", Uuid::new_v4());
            let temporary_name = OsStr::new(&temporary_name);
            let mut generation = create_owner_only_file_child(receipts, temporary_name)
                .map_err(|error| storage_error("create initial generation staging", error))?;
            let temporary_identity = file_identity(&generation)
                .map_err(|error| storage_error("identify initial generation staging", error))?;
            #[cfg(test)]
            run_after_initial_generation_create_hook_for_test();
            let encoded = format!("{initial_generation}\n");
            if let Err(error) = generation
                .write_all(encoded.as_bytes())
                .and_then(|()| generation.sync_all())
            {
                cleanup_staged_file(receipts, temporary_name, temporary_identity, &generation)?;
                return Err(storage_error("persist initial generation staging", error));
            }
            if check_deadline(deadline).is_err() {
                cleanup_staged_file(receipts, temporary_name, temporary_identity, &generation)?;
                return Err(ReceiptLedgerError::DeadlineExceeded);
            }
            if let Err(error) = rename_identity_bound_regular_child_no_replace(
                receipts,
                temporary_name,
                temporary_identity,
                &generation,
                receipts,
                name,
            ) {
                cleanup_staged_file(receipts, temporary_name, temporary_identity, &generation)?;
                return Err(storage_error(
                    "atomically publish initial generation",
                    error,
                ));
            }
            sync_directory(receipts)
                .map_err(|error| storage_error("sync initial generation", error))?;
            check_deadline(deadline)?;
            generation
        }
        Err(error) => return Err(storage_error("open generation record no-follow", error)),
    };
    let mut generation = generation;
    verify_owner_only_acl(&generation)
        .map_err(|error| storage_error("verify generation record ownership", error))?;
    generation
        .seek(SeekFrom::Start(0))
        .map_err(|error| storage_error("rewind generation record during recovery", error))?;
    let mut bytes = Vec::new();
    Read::by_ref(&mut generation)
        .take((MAX_GENERATION_FILE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| storage_error("read generation record during recovery", error))?;
    let persisted_generation = parse_generation(&bytes)?;
    check_deadline(deadline)?;
    Ok((generation, persisted_generation))
}

fn parse_generation(bytes: &[u8]) -> Result<u64, ReceiptLedgerError> {
    if bytes.is_empty() || bytes.len() > MAX_GENERATION_FILE_BYTES || !bytes.ends_with(b"\n") {
        return Err(ReceiptLedgerError::Corrupt(
            "generation record is not one bounded newline-terminated decimal",
        ));
    }
    let number = &bytes[..bytes.len() - 1];
    let text = std::str::from_utf8(number)
        .map_err(|_| ReceiptLedgerError::Corrupt("generation record is not UTF-8"))?;
    if text.is_empty()
        || !text.bytes().all(|byte| byte.is_ascii_digit())
        || (text.len() > 1 && text.starts_with('0'))
    {
        return Err(ReceiptLedgerError::Corrupt(
            "generation record is not canonical unsigned decimal",
        ));
    }
    text.parse()
        .map_err(|_| ReceiptLedgerError::Corrupt("generation record exceeds u64"))
}

fn check_deadline(deadline: Instant) -> Result<(), ReceiptLedgerError> {
    if Instant::now() >= deadline {
        Err(ReceiptLedgerError::DeadlineExceeded)
    } else {
        Ok(())
    }
}

fn check_optional_deadline(deadline: Option<Instant>) -> Result<(), ReceiptLedgerError> {
    match deadline {
        Some(deadline) => check_deadline(deadline),
        None => Ok(()),
    }
}

fn recovery_checkpoint(deadline: Instant) -> io::Result<()> {
    if Instant::now() >= deadline {
        Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "receipt recovery deadline expired",
        ))
    } else {
        Ok(())
    }
}

fn recovery_error(operation: &'static str, error: io::Error) -> ReceiptLedgerError {
    if error.kind() == io::ErrorKind::TimedOut {
        ReceiptLedgerError::DeadlineExceeded
    } else {
        storage_error(operation, error)
    }
}

fn sync_receipt_row_directory(directory: &File) -> io::Result<()> {
    let armed_on_this_thread =
        ARMED_RECEIPT_ROW_DIRECTORY_SYNC_FAULT_ON_THIS_THREAD.with(|slot| slot.replace(false));
    if armed_on_this_thread || ARMED_RECEIPT_ROW_DIRECTORY_SYNC_FAULT.swap(false, Ordering::AcqRel)
    {
        return Err(io::Error::other(
            "injected receipt row directory sync failure",
        ));
    }
    sync_directory(directory)
}

fn after_row_error(
    receipt_key_digest: Option<&ReceiptKeyDigest>,
    fallback: ReceiptLedgerError,
) -> ReceiptLedgerError {
    receipt_key_digest.map_or(fallback, |receipt_key_digest| {
        ReceiptLedgerError::CommitUncertain {
            receipt_key_digest: receipt_key_digest.clone(),
        }
    })
}

fn commit_or_storage_error(
    receipt_key_digest: Option<&ReceiptKeyDigest>,
    message: &'static str,
) -> ReceiptLedgerError {
    after_row_error(
        receipt_key_digest,
        ReceiptLedgerError::Storage {
            operation: "publish receipt generation",
            message: message.to_owned(),
        },
    )
}

fn generation_deadline_error(receipt_key_digest: Option<&ReceiptKeyDigest>) -> ReceiptLedgerError {
    after_row_error(receipt_key_digest, ReceiptLedgerError::DeadlineExceeded)
}

fn storage_error(operation: &'static str, error: io::Error) -> ReceiptLedgerError {
    ReceiptLedgerError::Storage {
        operation,
        message: error.to_string(),
    }
}

fn lock_is_contended(error: &io::Error) -> bool {
    let expected = fs2::lock_contended_error();
    error.kind() == io::ErrorKind::WouldBlock
        || error
            .raw_os_error()
            .zip(expected.raw_os_error())
            .is_some_and(|(actual, expected)| actual == expected)
}

mod catalog;
mod encoding;
mod port;
mod records;
mod validation;

use catalog::*;
use encoding::*;
use records::*;
use validation::*;

#[cfg(test)]
mod tests;
