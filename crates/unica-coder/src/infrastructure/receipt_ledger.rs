use crate::application::invocation_store::MAX_TASK_RECORD_ENVELOPE_BYTES;
use crate::application::receipt_ledger::{
    receipt_key_digest, CancelExpiryOutcome, CancelReservedReceipt, CancelResolution,
    CommittedDirectPublication, DirectTerminalUnackedReceipt, OriginalCutoffDescriptor, ReceiptKey,
    ReceiptKeyDigest, ReceiptLedgerError, ReceiptLedgerPort, ReceiptRecordHeader, ReceiptState,
    ReceiptTerminalOutcome, ReceiptVersion, ReserveOutcome, ReservedPhase, ReservedReceipt,
    V5CanonicalTerminal, CANCEL_RESERVATION_TTL_MS, DIRECT_TERMINAL_RETENTION_MS,
    MAX_LIVE_RECEIPTS, MAX_LIVE_RECEIPT_BYTES, MAX_RECEIPT_ENTITLEMENT_BYTES,
};
#[cfg(feature = "receipt-ledger-test-support")]
use crate::application::receipt_ledger::{
    ReceiptLedgerCatalogSnapshot, ReceiptLedgerCatalogSnapshotAuthority,
    ReceiptLedgerCatalogSnapshotParts,
};
use crate::domain::invocation::{InvocationId, TaskId};
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
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use uuid::Uuid;

const ACTIVE_DIRECTORY_NAME: &str = "active";
const GENERATION_FILE_NAME: &str = "generation";
const LEDGER_LOCK_FILE_NAME: &str = ".receipt-ledger.lock";
const MAX_GENERATION_FILE_BYTES: usize = 32;
const RECEIPT_RECORD_SCHEMA_VERSION: u32 = 1;
const MAX_CANCEL_RESERVED_RECORD_BYTES: u64 = 1_024;
const MAX_ACTIVE_DIRECTORY_ENTRIES: usize = MAX_LIVE_RECEIPTS * 2;
const MAX_GENERATION_STAGING_ENTRIES: usize = MAX_LIVE_RECEIPTS;
const MAX_RECEIPT_ROOT_DIRECTORY_ENTRIES: usize = MAX_GENERATION_STAGING_ENTRIES + 3;
const DEFAULT_RECEIPT_RECOVERY_TIMEOUT: Duration = Duration::from_secs(5);

#[cfg(test)]
thread_local! {
    static TEST_RECEIPT_ROW_DIRECTORY_SYNC_FAILURE: std::cell::Cell<bool> = const {
        std::cell::Cell::new(false)
    };
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

#[cfg(test)]
fn inject_receipt_row_directory_sync_failure_for_test() {
    TEST_RECEIPT_ROW_DIRECTORY_SYNC_FAILURE.with(|slot| slot.set(true));
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
    ReservedUnbound {
        reserved_at_epoch_ms: u64,
        original_cutoff: OriginalCutoffDescriptor,
        cancel_requested: bool,
    },
    DirectTerminalUnacked {
        original_cutoff: OriginalCutoffDescriptor,
        terminal_epoch_ms: u64,
        terminal_digest: crate::application::receipt_ledger::TerminalDigest,
        terminal: Arc<ReceiptTerminalOutcome>,
    },
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
            | StoredActiveLifecycleV1::ExpiredDeletion { .. } => 0,
            StoredActiveLifecycleV1::ReservedUnbound { .. }
            | StoredActiveLifecycleV1::DirectTerminalUnacked { .. } => {
                MAX_RECEIPT_ENTITLEMENT_BYTES
                    .checked_sub(self.encoded_bytes)
                    .expect("validated receipt record fits its exact byte entitlement")
            }
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
            StoredActiveLifecycleV1::ExpiredDeletion { .. } => Err(ReceiptLedgerError::Corrupt(
                "expired deletion witness is not a live receipt state",
            )),
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
        }
    }

    fn reservation(&self) -> Result<ReservedReceipt, ReceiptLedgerError> {
        match self.state()? {
            ReceiptState::Reserved(reservation) => Ok(reservation),
            ReceiptState::CancelReserved(_) | ReceiptState::DirectTerminalUnacked(_) => {
                Err(ReceiptLedgerError::ReceiptRowPresentUnsupported)
            }
            _ => unreachable!("active receipt codec currently has three variants"),
        }
    }

    fn is_expired_deletion(&self) -> bool {
        matches!(
            &self.record.lifecycle,
            StoredActiveLifecycleV1::ExpiredDeletion { .. }
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
    invocation_index: HashMap<InvocationId, ReceiptKeyDigest>,
    reserved_task_index: HashMap<TaskId, ReceiptKeyDigest>,
    actual_bytes: u64,
    reserved_result_bytes: u64,
    unavailable: bool,
}

#[cfg(feature = "receipt-ledger-test-support")]
fn sort_receipt_keys_by_digest(keys: &mut [ReceiptKey]) {
    keys.sort_unstable_by(|left, right| {
        receipt_key_digest(left)
            .as_str()
            .cmp(receipt_key_digest(right).as_str())
    });
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
        let recovered = if let Some((active, active_file)) = &existing_active {
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
        if let Some(witness_sequence) = recovered.expired_deletion_mutation_sequence {
            let is_current_or_next = witness_sequence == persisted_generation
                || persisted_generation.checked_add(1) == Some(witness_sequence);
            if !is_current_or_next || witness_sequence != recovered.maximum_mutation_sequence {
                return Err(ReceiptLedgerError::Corrupt(
                    "expired deletion witness is not the next persisted mutation",
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
        Ok(value)
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

    #[cfg(feature = "receipt-ledger-test-support")]
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
                    .map(|(_, entry)| entry.record.key.clone())
                    .collect::<Vec<_>>();

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
                    invocation_index.push(entry.record.key.clone());
                }
                sort_receipt_keys_by_digest(&mut invocation_index);

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
                    reserved_task_index.push(entry.record.key.clone());
                }
                sort_receipt_keys_by_digest(&mut reserved_task_index);

                Ok((
                    generation,
                    keys,
                    invocation_index,
                    reserved_task_index,
                    u64::try_from(catalog.records.len()).map_err(|_| {
                        ReceiptLedgerError::Corrupt("receipt catalog count does not fit telemetry")
                    })?,
                    catalog.actual_bytes,
                    catalog.reserved_result_bytes,
                ))
            },
        )?;
        let (
            generation,
            keys,
            invocation_index,
            reserved_task_index,
            live_count,
            actual_bytes,
            reserved_result_bytes,
        ) = match observed {
            Ok(observed) => observed,
            Err(error) => return latch_catalog_error(&mut catalog, error),
        };
        let snapshot = authority.seal(ReceiptLedgerCatalogSnapshotParts::new(
            generation,
            keys,
            invocation_index,
            reserved_task_index,
            live_count,
            actual_bytes,
            reserved_result_bytes,
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
            return Ok(match state {
                ReceiptState::CancelReserved(receipt) => CancelResolution::ExistingExact(receipt),
                winner => CancelResolution::ExistingWinner(Box::new(winner)),
            });
        }
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
        if catalog.records.len() >= MAX_LIVE_RECEIPTS {
            return self.reject_before_mutation(
                &mut catalog,
                deadline,
                ReceiptLedgerError::CapacityExceeded,
            );
        }
        let expires_at_epoch_ms = cancel_reserved_at_epoch_ms
            .checked_add(CANCEL_RESERVATION_TTL_MS)
            .ok_or(ReceiptLedgerError::TimestampOverflow)?;

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
        let record = match build_expired_deletion_record(
            &persisted,
            observed_at_epoch_ms,
            mutation_sequence,
            next_record_version,
        ) {
            Ok(record) => record,
            Err(error) => return latch_catalog_error(&mut catalog, error),
        };
        let (record, encoded) =
            match serialize_reserved_record(record, MAX_CANCEL_RESERVED_RECORD_BYTES) {
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
        Ok(CancelExpiryOutcome::Expired)
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
            return match state {
                ReceiptState::CancelReserved(_) => self.convert_cancel_reserved_to_submit(
                    &mut catalog,
                    persisted,
                    key,
                    original_cutoff,
                    deadline,
                ),
                state => Ok(ReserveOutcome::ExistingExact(state)),
            };
        }
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
        if catalog.records.len() >= MAX_LIVE_RECEIPTS {
            return self.reject_before_mutation(
                &mut catalog,
                deadline,
                ReceiptLedgerError::CapacityExceeded,
            );
        }

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
            _ => unreachable!("active receipt codec currently has three variants"),
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
        let (
            binding,
            encoded,
            encoded_bytes,
            reserved_result_bytes,
            _record_sha256,
            committed_terminal,
        ) = prepared_record.into_parts();
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
            Ok(Some(committed)) if committed.as_slice() == encoded.as_ref() => {}
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
        Ok(CommittedDirectPublication::new(receipt, wire_frame))
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
                Ok(state) => Ok(state),
                Err(error) => return latch_catalog_error(&mut catalog, error),
            },
            None => Err(ReceiptLedgerError::ReceiptNotFound),
        };
        check_deadline(deadline)?;
        result
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
        let entry = match self.read_active_record(receipt_key_digest) {
            Ok(entry) => entry,
            Err(error) => return latch_catalog_error(catalog, error),
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
        names.sort();
        let mut catalog = ReceiptCatalog::default();
        let mut maximum_mutation_sequence = 0;
        let mut mutation_sequences = HashSet::new();
        let mut recovered_invocations = HashMap::new();
        let mut recovered_tasks = HashMap::new();
        let mut temporary_entries = Vec::new();
        let mut expired_deletions = Vec::new();
        let mut expired_deletion_mutation_sequence = None;
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
            let digest = parse_receipt_record_name(name_text)?;
            let mut retained = open_regular_child_nofollow(active_file, &name)
                .map_err(|error| storage_error("open receipt row during recovery", error))?;
            verify_owner_only_acl(&retained)
                .map_err(|error| storage_error("verify recovered receipt row ownership", error))?;
            let identity = file_identity(&retained)
                .map_err(|error| storage_error("identify recovered receipt row", error))?;
            let entry = read_active_record_from_retained(&mut retained, &digest)?;
            check_deadline(deadline)?;
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
            if !mutation_sequences.insert(entry.record.mutation_sequence) {
                return Err(ReceiptLedgerError::Corrupt(
                    "receipt recovery contains a duplicate mutation sequence",
                ));
            }
            maximum_mutation_sequence =
                maximum_mutation_sequence.max(entry.record.mutation_sequence);
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

impl ReceiptLedgerPort for ReceiptLedgerStore {
    #[cfg(feature = "receipt-ledger-test-support")]
    fn snapshot_catalog(
        &mut self,
        authority: ReceiptLedgerCatalogSnapshotAuthority,
        deadline: Instant,
    ) -> Result<ReceiptLedgerCatalogSnapshot, ReceiptLedgerError> {
        ReceiptLedgerStore::snapshot_catalog(self, authority, deadline)
    }

    fn generation(&mut self, deadline: Instant) -> Result<u64, ReceiptLedgerError> {
        check_deadline(deadline)?;
        let generation = ReceiptLedgerStore::generation(self)?;
        check_deadline(deadline)?;
        Ok(generation)
    }

    fn reserve(
        &mut self,
        key: ReceiptKey,
        original_cutoff: OriginalCutoffDescriptor,
        deadline: Instant,
    ) -> Result<ReserveOutcome, ReceiptLedgerError> {
        ReceiptLedgerStore::reserve(self, key, original_cutoff, deadline)
    }

    fn request_cancel_or_reserve(
        &mut self,
        key: ReceiptKey,
        cancel_reserved_at_epoch_ms: u64,
        deadline: Instant,
    ) -> Result<CancelResolution, ReceiptLedgerError> {
        ReceiptLedgerStore::request_cancel_or_reserve(
            self,
            key,
            cancel_reserved_at_epoch_ms,
            deadline,
        )
    }

    fn expire_cancel_reserved(
        &mut self,
        key: ReceiptKey,
        expected_version: ReceiptVersion,
        expected_mutation_sequence: u64,
        observed_at_epoch_ms: u64,
        deadline: Instant,
    ) -> Result<CancelExpiryOutcome, ReceiptLedgerError> {
        ReceiptLedgerStore::expire_cancel_reserved(
            self,
            key,
            expected_version,
            expected_mutation_sequence,
            observed_at_epoch_ms,
            deadline,
        )
    }

    fn publish_direct_terminal(
        &mut self,
        key: &ReceiptKey,
        expected_version: ReceiptVersion,
        terminal_epoch_ms: u64,
        terminal: V5CanonicalTerminal,
        deadline: Instant,
    ) -> Result<CommittedDirectPublication, ReceiptLedgerError> {
        ReceiptLedgerStore::publish_direct_terminal_publication(
            self,
            key,
            expected_version,
            terminal_epoch_ms,
            terminal,
            deadline,
        )
    }

    fn recover(
        &mut self,
        key: &ReceiptKey,
        deadline: Instant,
    ) -> Result<ReceiptState, ReceiptLedgerError> {
        self.recover_exact(key, deadline)
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
    let record: StoredActiveReceiptV1 = serde_json::from_slice(&bytes)
        .map_err(|_| ReceiptLedgerError::Corrupt("receipt row is not strict schema-v1 JSON"))?;
    validate_active_record(&record, &bytes, receipt_key_digest)?;
    if matches!(
        &record.lifecycle,
        StoredActiveLifecycleV1::CancelReserved { .. }
            | StoredActiveLifecycleV1::ExpiredDeletion { .. }
            | StoredActiveLifecycleV1::ReservedUnbound { .. }
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

fn build_reserved_record(
    key: ReceiptKey,
    key_digest: ReceiptKeyDigest,
    original_cutoff: OriginalCutoffDescriptor,
    mutation_sequence: u64,
    record_version: ReceiptVersion,
    cancel_requested: bool,
) -> StoredActiveReceiptV1 {
    StoredActiveReceiptV1 {
        schema_version: RECEIPT_RECORD_SCHEMA_VERSION,
        mutation_sequence,
        record_version,
        key,
        key_digest,
        lifecycle: StoredActiveLifecycleV1::ReservedUnbound {
            reserved_at_epoch_ms: original_cutoff.accepted_epoch_ms(),
            original_cutoff,
            cancel_requested,
        },
    }
}

fn build_cancel_reserved_record(
    key: ReceiptKey,
    key_digest: ReceiptKeyDigest,
    cancel_reserved_at_epoch_ms: u64,
    expires_at_epoch_ms: u64,
    mutation_sequence: u64,
) -> StoredActiveReceiptV1 {
    StoredActiveReceiptV1 {
        schema_version: RECEIPT_RECORD_SCHEMA_VERSION,
        mutation_sequence,
        record_version: ReceiptVersion::initial(),
        key,
        key_digest,
        lifecycle: StoredActiveLifecycleV1::CancelReserved {
            cancel_reserved_at_epoch_ms,
            expires_at_epoch_ms,
            cancel_requested: true,
        },
    }
}

fn build_expired_deletion_record(
    expected: &CatalogEntry,
    observed_at_epoch_ms: u64,
    mutation_sequence: u64,
    record_version: ReceiptVersion,
) -> Result<StoredActiveReceiptV1, ReceiptLedgerError> {
    let (prior_cancel_reserved_at_epoch_ms, prior_expires_at_epoch_ms) =
        match &expected.record.lifecycle {
            StoredActiveLifecycleV1::CancelReserved {
                cancel_reserved_at_epoch_ms,
                expires_at_epoch_ms,
                ..
            } => (*cancel_reserved_at_epoch_ms, *expires_at_epoch_ms),
            _ => {
                return Err(ReceiptLedgerError::Corrupt(
                    "expired deletion witness requires a CancelReserved predecessor",
                ))
            }
        };
    Ok(StoredActiveReceiptV1 {
        schema_version: RECEIPT_RECORD_SCHEMA_VERSION,
        mutation_sequence,
        record_version,
        key: expected.record.key.clone(),
        key_digest: expected.record.key_digest.clone(),
        lifecycle: StoredActiveLifecycleV1::ExpiredDeletion {
            observed_at_epoch_ms,
            prior_record_version: expected.record.record_version,
            prior_mutation_sequence: expected.record.mutation_sequence,
            prior_cancel_reserved_at_epoch_ms,
            prior_expires_at_epoch_ms,
        },
    })
}

/// Sole serializer for active non-terminal receipt records.
///
/// Direct terminal bytes are owned by `terminal_codec_v5`; accepting one here
/// would create a second, potentially divergent codec for the same lifecycle.
fn serialize_reserved_record(
    record: StoredActiveReceiptV1,
    maximum_encoded_bytes: u64,
) -> Result<(StoredActiveReceiptV1, Vec<u8>), ReceiptLedgerError> {
    if matches!(
        &record.lifecycle,
        StoredActiveLifecycleV1::DirectTerminalUnacked { .. }
    ) {
        return Err(ReceiptLedgerError::Corrupt(
            "Direct terminal rows must use the sole v5 terminal codec",
        ));
    }
    let encoded = serde_json::to_vec(&record)
        .map_err(|_| ReceiptLedgerError::Corrupt("receipt row serialization failed"))?;
    let encoded_bytes =
        u64::try_from(encoded.len()).map_err(|_| ReceiptLedgerError::RecordTooLarge)?;
    if encoded_bytes > maximum_encoded_bytes || encoded_bytes > MAX_RECEIPT_ENTITLEMENT_BYTES {
        return Err(ReceiptLedgerError::RecordTooLarge);
    }
    Ok((record, encoded))
}

fn validate_persisted_reserved_record_bytes(
    record: &StoredActiveReceiptV1,
    persisted_bytes: &[u8],
) -> Result<(), ReceiptLedgerError> {
    let maximum_encoded_bytes = match &record.lifecycle {
        StoredActiveLifecycleV1::CancelReserved { .. }
        | StoredActiveLifecycleV1::ExpiredDeletion { .. } => MAX_CANCEL_RESERVED_RECORD_BYTES,
        StoredActiveLifecycleV1::ReservedUnbound { .. } => MAX_TASK_RECORD_ENVELOPE_BYTES as u64,
        StoredActiveLifecycleV1::DirectTerminalUnacked { .. } => {
            return Err(ReceiptLedgerError::Corrupt(
                "non-terminal receipt validator received a Direct terminal lifecycle",
            ))
        }
    };
    let (canonical_record, canonical_bytes) =
        serialize_reserved_record(record.clone(), maximum_encoded_bytes)?;
    if &canonical_record != record || canonical_bytes != persisted_bytes {
        return Err(ReceiptLedgerError::Corrupt(
            "receipt row is not canonical schema-v1 JSON",
        ));
    }
    Ok(())
}

fn parse_receipt_record_name(name: &str) -> Result<ReceiptKeyDigest, ReceiptLedgerError> {
    let digest = name
        .strip_suffix(".json")
        .ok_or(ReceiptLedgerError::Corrupt(
            "receipt active entry has an unsupported name",
        ))?;
    digest
        .parse()
        .map_err(|_| ReceiptLedgerError::Corrupt("receipt row name is not a canonical digest"))
}

fn parse_receipt_temporary_name(name: &str) -> Result<bool, ReceiptLedgerError> {
    let Some(uuid_text) = name
        .strip_prefix(".receipt.")
        .and_then(|name| name.strip_suffix(".tmp"))
    else {
        return Ok(false);
    };
    let uuid = Uuid::parse_str(uuid_text).map_err(|_| {
        ReceiptLedgerError::Corrupt("receipt staging name does not contain a canonical UUIDv4")
    })?;
    if uuid.hyphenated().to_string() != uuid_text
        || uuid.get_version() != Some(uuid::Version::Random)
        || uuid.get_variant() != uuid::Variant::RFC4122
    {
        return Err(ReceiptLedgerError::Corrupt(
            "receipt staging name does not contain a canonical UUIDv4",
        ));
    }
    Ok(true)
}

fn parse_generation_temporary_name(name: &str) -> Result<bool, ReceiptLedgerError> {
    let Some(uuid_text) = name
        .strip_prefix(".generation.")
        .and_then(|name| name.strip_suffix(".tmp"))
    else {
        return Ok(false);
    };
    let uuid = Uuid::parse_str(uuid_text).map_err(|_| {
        ReceiptLedgerError::Corrupt("generation staging name does not contain a canonical UUIDv4")
    })?;
    if uuid.hyphenated().to_string() != uuid_text
        || uuid.get_version() != Some(uuid::Version::Random)
        || uuid.get_variant() != uuid::Variant::RFC4122
    {
        return Err(ReceiptLedgerError::Corrupt(
            "generation staging name does not contain a canonical UUIDv4",
        ));
    }
    Ok(true)
}

fn parse_cleanup_quarantine_name(name: &str) -> Result<bool, ReceiptLedgerError> {
    let Some(uuid_text) = name.strip_prefix(".unica-cleanup-") else {
        return Ok(false);
    };
    let uuid = Uuid::parse_str(uuid_text).map_err(|_| {
        ReceiptLedgerError::Corrupt("cleanup quarantine name is not a canonical UUIDv4")
    })?;
    if uuid.hyphenated().to_string() != uuid_text
        || uuid.get_version() != Some(uuid::Version::Random)
        || uuid.get_variant() != uuid::Variant::RFC4122
    {
        return Err(ReceiptLedgerError::Corrupt(
            "cleanup quarantine name is not a canonical UUIDv4",
        ));
    }
    Ok(true)
}

fn validate_active_record(
    record: &StoredActiveReceiptV1,
    encoded: &[u8],
    expected_digest: &ReceiptKeyDigest,
) -> Result<(), ReceiptLedgerError> {
    if record.schema_version != RECEIPT_RECORD_SCHEMA_VERSION {
        return Err(ReceiptLedgerError::Corrupt(
            "receipt row schema version is unsupported",
        ));
    }
    if record.mutation_sequence == 0 {
        return Err(ReceiptLedgerError::Corrupt(
            "receipt mutation sequence must be positive",
        ));
    }
    if &record.key_digest != expected_digest || receipt_key_digest(&record.key) != record.key_digest
    {
        return Err(ReceiptLedgerError::Corrupt(
            "receipt row digest does not match its name and exact key",
        ));
    }
    let per_record_limit = match &record.lifecycle {
        StoredActiveLifecycleV1::CancelReserved {
            cancel_reserved_at_epoch_ms,
            expires_at_epoch_ms,
            cancel_requested,
        } => {
            if record.record_version != ReceiptVersion::initial() {
                return Err(ReceiptLedgerError::Corrupt(
                    "CancelReserved receipt must retain its initial record version",
                ));
            }
            if !cancel_requested {
                return Err(ReceiptLedgerError::Corrupt(
                    "CancelReserved receipt must persist cancelRequested=true",
                ));
            }
            let expected_expiry = cancel_reserved_at_epoch_ms
                .checked_add(CANCEL_RESERVATION_TTL_MS)
                .ok_or(ReceiptLedgerError::Corrupt(
                    "CancelReserved expiry exceeds u64",
                ))?;
            if *expires_at_epoch_ms != expected_expiry {
                return Err(ReceiptLedgerError::Corrupt(
                    "CancelReserved expiry is not the fixed absolute TTL",
                ));
            }
            MAX_CANCEL_RESERVED_RECORD_BYTES
        }
        StoredActiveLifecycleV1::ExpiredDeletion {
            observed_at_epoch_ms,
            prior_record_version,
            prior_mutation_sequence,
            prior_cancel_reserved_at_epoch_ms,
            prior_expires_at_epoch_ms,
        } => {
            if *prior_record_version != ReceiptVersion::initial() {
                return Err(ReceiptLedgerError::Corrupt(
                    "expired deletion witness predecessor is not an initial CancelReserved",
                ));
            }
            if prior_record_version.checked_next() != Some(record.record_version) {
                return Err(ReceiptLedgerError::Corrupt(
                    "expired deletion witness does not advance its predecessor version",
                ));
            }
            if *prior_mutation_sequence == 0 || *prior_mutation_sequence >= record.mutation_sequence
            {
                return Err(ReceiptLedgerError::Corrupt(
                    "expired deletion witness does not follow its predecessor mutation",
                ));
            }
            if prior_cancel_reserved_at_epoch_ms.checked_add(CANCEL_RESERVATION_TTL_MS)
                != Some(*prior_expires_at_epoch_ms)
            {
                return Err(ReceiptLedgerError::Corrupt(
                    "expired deletion witness predecessor expiry is not its fixed absolute TTL",
                ));
            }
            if observed_at_epoch_ms < prior_expires_at_epoch_ms {
                return Err(ReceiptLedgerError::Corrupt(
                    "expired deletion witness predates the absolute expiry boundary",
                ));
            }
            MAX_CANCEL_RESERVED_RECORD_BYTES
        }
        StoredActiveLifecycleV1::ReservedUnbound {
            reserved_at_epoch_ms,
            original_cutoff,
            ..
        } => {
            if reserved_at_epoch_ms != &original_cutoff.accepted_epoch_ms() {
                return Err(ReceiptLedgerError::Corrupt(
                    "receipt reserve epoch does not match its accepted request epoch",
                ));
            }
            MAX_TASK_RECORD_ENVELOPE_BYTES as u64
        }
        StoredActiveLifecycleV1::DirectTerminalUnacked {
            original_cutoff,
            terminal_epoch_ms,
            terminal_digest,
            terminal,
            ..
        } => {
            if record.record_version == ReceiptVersion::initial() {
                return Err(ReceiptLedgerError::Corrupt(
                    "direct terminal receipt must advance its record version",
                ));
            }
            terminal_epoch_ms
                .checked_add(DIRECT_TERMINAL_RETENTION_MS)
                .ok_or(ReceiptLedgerError::Corrupt(
                    "receipt terminal expiry exceeds u64",
                ))?;
            validate_persisted_direct_record_bytes(
                record.mutation_sequence,
                record.record_version,
                &record.key,
                &record.key_digest,
                *original_cutoff,
                *terminal_epoch_ms,
                terminal_digest,
                Arc::clone(terminal),
                encoded,
            )?;
            MAX_RECEIPT_ENTITLEMENT_BYTES
        }
    };
    if u64::try_from(encoded.len()).map_or(true, |length| length > per_record_limit) {
        return Err(ReceiptLedgerError::Corrupt(
            "persisted receipt row exceeds its byte limit",
        ));
    }
    Ok(())
}

fn validate_catalog_insert(
    catalog: &ReceiptCatalog,
    entry: &CatalogEntry,
    recovering: bool,
) -> Result<(), ReceiptLedgerError> {
    if catalog.records.len() >= MAX_LIVE_RECEIPTS {
        return Err(if recovering {
            ReceiptLedgerError::Corrupt("receipt catalog exceeds the live-record limit")
        } else {
            ReceiptLedgerError::CapacityExceeded
        });
    }
    if catalog.records.contains_key(&entry.record.key_digest) {
        return Err(if recovering {
            ReceiptLedgerError::Corrupt("receipt catalog contains a duplicate key digest")
        } else {
            ReceiptLedgerError::ReceiptDigestCollision
        });
    }
    if catalog
        .invocation_index
        .contains_key(&entry.record.key.invocation_id())
    {
        return Err(if recovering {
            ReceiptLedgerError::Corrupt("receipt catalog contains a duplicate invocation id")
        } else {
            ReceiptLedgerError::InvocationIdentityMismatch
        });
    }
    if catalog
        .reserved_task_index
        .contains_key(&entry.record.key.reserved_task_id())
    {
        return Err(if recovering {
            ReceiptLedgerError::Corrupt("receipt catalog contains a duplicate reserved task id")
        } else {
            ReceiptLedgerError::ReservedTaskIdentityMismatch
        });
    }
    if catalog
        .records
        .values()
        .any(|stored| stored.record.mutation_sequence == entry.record.mutation_sequence)
    {
        return Err(ReceiptLedgerError::Corrupt(
            "receipt catalog contains a duplicate mutation sequence",
        ));
    }
    let next_actual_bytes = catalog
        .actual_bytes
        .checked_add(entry.encoded_bytes)
        .ok_or(ReceiptLedgerError::CapacityExceeded)?;
    let next_reserved_bytes = catalog
        .reserved_result_bytes
        .checked_add(entry.reserved_result_bytes())
        .ok_or(ReceiptLedgerError::CapacityExceeded)?;
    if next_actual_bytes
        .checked_add(next_reserved_bytes)
        .filter(|total| *total <= MAX_LIVE_RECEIPT_BYTES)
        .is_none()
    {
        return Err(if recovering {
            ReceiptLedgerError::Corrupt("receipt catalog exceeds the byte entitlement limit")
        } else {
            ReceiptLedgerError::CapacityExceeded
        });
    }
    Ok(())
}

fn commit_catalog_insert(catalog: &mut ReceiptCatalog, entry: CatalogEntry) {
    catalog.actual_bytes += entry.encoded_bytes;
    catalog.reserved_result_bytes += entry.reserved_result_bytes();
    catalog.invocation_index.insert(
        entry.record.key.invocation_id(),
        entry.record.key_digest.clone(),
    );
    catalog.reserved_task_index.insert(
        entry.record.key.reserved_task_id(),
        entry.record.key_digest.clone(),
    );
    catalog
        .records
        .insert(entry.record.key_digest.clone(), entry);
}

fn insert_catalog_entry(
    catalog: &mut ReceiptCatalog,
    entry: CatalogEntry,
    recovering: bool,
) -> Result<(), ReceiptLedgerError> {
    validate_catalog_insert(catalog, &entry, recovering)?;
    commit_catalog_insert(catalog, entry);
    Ok(())
}

fn validate_catalog_remove(
    catalog: &ReceiptCatalog,
    expected: &CatalogEntry,
) -> Result<(), ReceiptLedgerError> {
    let digest = &expected.record.key_digest;
    if catalog.records.get(digest) != Some(expected) {
        return Err(ReceiptLedgerError::Corrupt(
            "receipt catalog changed before exact removal",
        ));
    }
    if catalog
        .invocation_index
        .get(&expected.record.key.invocation_id())
        != Some(digest)
    {
        return Err(ReceiptLedgerError::Corrupt(
            "receipt invocation index changed before exact removal",
        ));
    }
    if catalog
        .reserved_task_index
        .get(&expected.record.key.reserved_task_id())
        != Some(digest)
    {
        return Err(ReceiptLedgerError::Corrupt(
            "receipt task index changed before exact removal",
        ));
    }
    catalog
        .actual_bytes
        .checked_sub(expected.encoded_bytes)
        .ok_or(ReceiptLedgerError::Corrupt(
            "receipt catalog actual-byte accounting underflowed",
        ))?;
    catalog
        .reserved_result_bytes
        .checked_sub(expected.reserved_result_bytes())
        .ok_or(ReceiptLedgerError::Corrupt(
            "receipt catalog reserved-byte accounting underflowed",
        ))?;
    Ok(())
}

fn commit_catalog_remove(catalog: &mut ReceiptCatalog, expected: &CatalogEntry) {
    let digest = &expected.record.key_digest;
    catalog.actual_bytes -= expected.encoded_bytes;
    catalog.reserved_result_bytes -= expected.reserved_result_bytes();
    catalog
        .invocation_index
        .remove(&expected.record.key.invocation_id());
    catalog
        .reserved_task_index
        .remove(&expected.record.key.reserved_task_id());
    catalog.records.remove(digest);
}

fn validate_catalog_replace(
    catalog: &ReceiptCatalog,
    expected: &CatalogEntry,
    replacement: &CatalogEntry,
) -> Result<(), ReceiptLedgerError> {
    if catalog.records.get(&expected.record.key_digest) != Some(expected) {
        return Err(ReceiptLedgerError::Corrupt(
            "receipt catalog changed before exact replacement",
        ));
    }
    if replacement.record.key_digest != expected.record.key_digest
        || replacement.record.key != expected.record.key
    {
        return Err(ReceiptLedgerError::Corrupt(
            "receipt replacement changed its exact identity",
        ));
    }
    if catalog.records.iter().any(|(digest, stored)| {
        digest != &expected.record.key_digest
            && stored.record.mutation_sequence == replacement.record.mutation_sequence
    }) {
        return Err(ReceiptLedgerError::Corrupt(
            "receipt replacement reuses a mutation sequence",
        ));
    }
    let next_actual_bytes = catalog
        .actual_bytes
        .checked_sub(expected.encoded_bytes)
        .and_then(|bytes| bytes.checked_add(replacement.encoded_bytes))
        .ok_or(ReceiptLedgerError::Corrupt(
            "receipt catalog actual-byte accounting underflowed",
        ))?;
    let next_reserved_bytes = catalog
        .reserved_result_bytes
        .checked_sub(expected.reserved_result_bytes())
        .and_then(|bytes| bytes.checked_add(replacement.reserved_result_bytes()))
        .ok_or(ReceiptLedgerError::Corrupt(
            "receipt catalog reserved-byte accounting underflowed",
        ))?;
    if next_actual_bytes
        .checked_add(next_reserved_bytes)
        .filter(|total| *total <= MAX_LIVE_RECEIPT_BYTES)
        .is_none()
    {
        return Err(ReceiptLedgerError::CapacityExceeded);
    }
    Ok(())
}

fn commit_catalog_replace(catalog: &mut ReceiptCatalog, replacement: CatalogEntry) {
    let digest = replacement.record.key_digest.clone();
    let replacement_encoded_bytes = replacement.encoded_bytes;
    let replacement_reserved_result_bytes = replacement.reserved_result_bytes();
    let previous = catalog
        .records
        .insert(digest, replacement)
        .expect("validated receipt replacement has an existing catalog entry");
    catalog.actual_bytes =
        catalog.actual_bytes - previous.encoded_bytes + replacement_encoded_bytes;
    catalog.reserved_result_bytes = catalog.reserved_result_bytes
        - previous.reserved_result_bytes()
        + replacement_reserved_result_bytes;
}

fn latch_catalog_error<T>(
    catalog: &mut ReceiptCatalog,
    error: ReceiptLedgerError,
) -> Result<T, ReceiptLedgerError> {
    catalog.unavailable = true;
    Err(error)
}

fn latch_catalog_result<T>(
    catalog: &mut ReceiptCatalog,
    result: Result<T, ReceiptLedgerError>,
) -> Result<T, ReceiptLedgerError> {
    match result {
        Ok(value) => Ok(value),
        Err(error) => latch_catalog_error(catalog, error),
    }
}

fn cleanup_staged_file(
    parent: &File,
    name: &OsStr,
    identity: FileIdentity,
    file: &File,
) -> Result<(), ReceiptLedgerError> {
    remove_identity_bound_regular_child(parent, name, identity, file)
        .map_err(|error| storage_error("clean up receipt staging file", error))
}

fn sync_receipt_row_directory(directory: &File) -> io::Result<()> {
    #[cfg(test)]
    if TEST_RECEIPT_ROW_DIRECTORY_SYNC_FAILURE.with(|slot| slot.replace(false)) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::invocation::normalized_arguments_hash;
    use crate::application::receipt_ledger::{
        canonical_v5_terminal, request_scope_hash, CoreIdentityDigest, OriginalCutoffDescriptor,
        ReceiptKey, ReceiptLedgerPort, ReceiptState, ReceiptTerminalOutcome, ReceiptVersion,
        RequestIdentity, ReserveOutcome, V5ToolIdentity,
    };
    use crate::domain::invocation::{DomainResult, InvocationId, TaskId};
    use crate::infrastructure::platform::filesystem::{
        open_directory_nofollow, open_regular_child_nofollow,
        set_before_identity_bound_cleanup_mutation_hook,
        set_before_identity_bound_no_replace_rename_hook, verify_owner_only_acl,
    };
    use crate::infrastructure::platform::testing::{
        attempt_retained_directory_replacement_for_test,
        attempt_retained_regular_file_relocation_for_test, create_directory_link_fixture_for_test,
        set_unix_mode_for_test, FileLinkFixtureOutcome, RetainedDirectoryReplacementOutcome,
        RetainedRegularFileRelocationOutcome,
    };
    use std::cell::Cell;
    use std::ffi::OsStr;
    use std::fs;
    use std::io::{Read, Write};
    use std::str::FromStr;
    use std::time::{Duration, Instant};

    const INVOCATION_A: &str = "11111111-1111-4111-8111-111111111111";
    const INVOCATION_B: &str = "22222222-2222-4222-8222-222222222222";
    const TASK_A: &str = "33333333-3333-4333-8333-333333333333";
    const TASK_B: &str = "44444444-4444-4444-8444-444444444444";

    fn digest(byte: char) -> ReceiptKeyDigest {
        ReceiptKeyDigest::from_str(&byte.to_string().repeat(64)).expect("checked digest")
    }

    fn receipt_key(
        invocation_id: &str,
        reserved_task_id: &str,
        workspace_hint: &str,
    ) -> ReceiptKey {
        ReceiptKey::new(
            InvocationId::from_str(invocation_id).expect("canonical invocation id"),
            TaskId::from_str(reserved_task_id).expect("canonical task id"),
            RequestIdentity::new(
                CoreIdentityDigest::from_sha256([0x55; 32]),
                V5ToolIdentity::View,
                normalized_arguments_hash(&serde_json::Map::new()),
                request_scope_hash(workspace_hint).expect("bounded request scope"),
            ),
        )
    }

    fn receipt_key_with_ids(
        invocation_id: InvocationId,
        reserved_task_id: TaskId,
        workspace_hint: &str,
    ) -> ReceiptKey {
        ReceiptKey::new(
            invocation_id,
            reserved_task_id,
            RequestIdentity::new(
                CoreIdentityDigest::from_sha256([0x55; 32]),
                V5ToolIdentity::View,
                normalized_arguments_hash(&serde_json::Map::new()),
                request_scope_hash(workspace_hint).expect("bounded request scope"),
            ),
        )
    }

    fn reserve_deadline() -> Instant {
        Instant::now() + Duration::from_secs(2)
    }

    fn directory_names(path: &Path) -> Vec<String> {
        let mut names = fs::read_dir(path)
            .expect("read receipt directory")
            .map(|entry| {
                entry
                    .expect("read receipt entry")
                    .file_name()
                    .into_string()
                    .expect("receipt entry name is UTF-8")
            })
            .collect::<Vec<_>>();
        names.sort();
        names
    }

    fn write_reserved_row_fixture(
        receipts: &Path,
        key: ReceiptKey,
        cutoff: OriginalCutoffDescriptor,
        mutation_sequence: u64,
    ) -> ReceiptKeyDigest {
        let key_digest = crate::application::receipt_ledger::receipt_key_digest(&key);
        let record = build_reserved_record(
            key,
            key_digest.clone(),
            cutoff,
            mutation_sequence,
            ReceiptVersion::initial(),
            false,
        );
        let (_, encoded) = serialize_reserved_record(record, MAX_TASK_RECORD_ENVELOPE_BYTES as u64)
            .expect("serialize valid reserved-row fixture");
        let receipts = open_directory_nofollow(receipts).expect("open receipts fixture");
        let active = crate::infrastructure::platform::filesystem::open_directory_child_nofollow(
            &receipts,
            OsStr::new(ACTIVE_DIRECTORY_NAME),
        )
        .expect("open active fixture");
        let name = format!("{}.json", key_digest.as_str());
        let mut row = create_owner_only_file_child(&active, OsStr::new(&name))
            .expect("create owner-only reserved-row fixture");
        row.write_all(&encoded)
            .and_then(|()| row.sync_all())
            .expect("persist reserved-row fixture");
        sync_directory(&active).expect("sync reserved-row fixture");
        key_digest
    }

    fn write_expiry_witness_fixture(
        receipts: &Path,
        key: ReceiptKey,
        prior_mutation_sequence: u64,
        mutation_sequence: u64,
    ) -> ReceiptKeyDigest {
        let key_digest = receipt_key_digest(&key);
        let predecessor = CatalogEntry {
            record: build_cancel_reserved_record(
                key,
                key_digest.clone(),
                1_000,
                8_125,
                prior_mutation_sequence,
            ),
            encoded_bytes: 512,
        };
        let record = build_expired_deletion_record(
            &predecessor,
            8_125,
            mutation_sequence,
            ReceiptVersion::new(2).expect("next expiry witness version"),
        )
        .expect("build valid expiry witness fixture");
        let (_, encoded) = serialize_reserved_record(record, MAX_CANCEL_RESERVED_RECORD_BYTES)
            .expect("serialize expiry witness fixture");
        let receipts = open_directory_nofollow(receipts).expect("open receipts fixture");
        let active = crate::infrastructure::platform::filesystem::open_directory_child_nofollow(
            &receipts,
            OsStr::new(ACTIVE_DIRECTORY_NAME),
        )
        .expect("open active fixture");
        let name = format!("{}.json", key_digest.as_str());
        let mut row = create_owner_only_file_child(&active, OsStr::new(&name))
            .expect("create owner-only expiry witness fixture");
        row.write_all(&encoded)
            .and_then(|()| row.sync_all())
            .expect("persist expiry witness fixture");
        sync_directory(&active).expect("sync expiry witness fixture");
        key_digest
    }

    #[test]
    fn open_persists_and_reopens_generation_zero_in_owner_only_receipts() {
        let root = tempfile::tempdir().expect("temporary root");
        let receipts = fs::canonicalize(root.path())
            .expect("physical temporary root")
            .join("receipts");

        {
            let store = ReceiptLedgerStore::open(&receipts).expect("open receipt ledger");
            assert_eq!(store.generation().expect("read generation"), 0);
        }

        let store = ReceiptLedgerStore::open(&receipts).expect("reopen receipt ledger");
        assert_eq!(store.generation().expect("reread generation"), 0);

        let receipts_handle = open_directory_nofollow(&receipts).expect("retained receipts");
        verify_owner_only_acl(&receipts_handle).expect("owner-only receipts");
        let generation = open_regular_child_nofollow(&receipts_handle, OsStr::new("generation"))
            .expect("generation record");
        verify_owner_only_acl(&generation).expect("owner-only generation");
        let active = crate::infrastructure::platform::filesystem::open_directory_child_nofollow(
            &receipts_handle,
            OsStr::new("active"),
        )
        .expect("retained active directory");
        verify_owner_only_acl(&active).expect("owner-only active directory");
    }

    #[test]
    fn recovery_staging_names_require_rfc4122_random_uuid_identity() {
        let receipt = ".receipt.aaaaaaaa-aaaa-4aaa-0aaa-aaaaaaaaaaaa.tmp";
        let generation = ".generation.bbbbbbbb-bbbb-4bbb-0bbb-bbbbbbbbbbbb.tmp";
        let cleanup = ".unica-cleanup-cccccccc-cccc-4ccc-0ccc-cccccccccccc";

        assert_eq!(
            parse_receipt_temporary_name(receipt)
                .expect_err("non-RFC UUID variant cannot identify our receipt staging"),
            ReceiptLedgerError::Corrupt("receipt staging name does not contain a canonical UUIDv4")
        );
        assert_eq!(
            parse_generation_temporary_name(generation)
                .expect_err("non-RFC UUID variant cannot identify our generation staging"),
            ReceiptLedgerError::Corrupt(
                "generation staging name does not contain a canonical UUIDv4"
            )
        );
        assert_eq!(
            parse_cleanup_quarantine_name(cleanup)
                .expect_err("non-RFC UUID variant cannot identify our cleanup quarantine"),
            ReceiptLedgerError::Corrupt("cleanup quarantine name is not a canonical UUIDv4")
        );
    }

    #[test]
    fn first_generation_publication_survives_a_crash_after_staging_creation() {
        let root = tempfile::tempdir().expect("temporary root");
        let receipts = fs::canonicalize(root.path())
            .expect("physical temporary root")
            .join("receipts");
        set_after_initial_generation_create_hook_for_test(|| {
            panic!("simulated crash after initial generation staging creation")
        });

        let crashed = std::panic::catch_unwind(|| {
            let _ = ReceiptLedgerStore::open(&receipts);
        });

        assert!(crashed.is_err(), "initial generation failpoint must run");
        assert!(
            !receipts.join(GENERATION_FILE_NAME).exists(),
            "a crash before atomic publication must not expose a partial final generation"
        );
        let reopened = ReceiptLedgerStore::open(&receipts)
            .expect("reopen replaces the abandoned initial staging publication");
        assert_eq!(reopened.generation().expect("recovered generation"), 0);
        let names = directory_names(&receipts);
        assert!(
            names.iter().any(|name| name == ACTIVE_DIRECTORY_NAME),
            "reopen restores the active directory"
        );
        assert!(
            names.iter().any(|name| name == GENERATION_FILE_NAME),
            "reopen publishes the canonical generation file"
        );
        assert!(
            names.iter().all(|name| !name.starts_with(".generation.")),
            "reopen removes the abandoned generation staging file"
        );
    }

    #[test]
    fn cancel_reserved_persists_exact_absolute_expiry_without_result_entitlement() {
        let root = tempfile::tempdir().expect("temporary root");
        let receipts = fs::canonicalize(root.path())
            .expect("physical temporary root")
            .join("receipts");
        let store = ReceiptLedgerStore::open(&receipts).expect("open receipt ledger");
        let key = receipt_key(INVOCATION_A, TASK_A, "workspace-a");

        let initial = store
            .request_cancel_or_reserve(key.clone(), 1_000, reserve_deadline())
            .expect("reserve cancellation before submit");
        let initial = match initial {
            crate::application::receipt_ledger::CancelResolution::NewlyReserved(receipt) => receipt,
            other => panic!("first cancel must create CancelReserved, got {other:?}"),
        };
        assert_eq!(initial.key(), &key);
        assert_eq!(initial.record_version(), ReceiptVersion::initial());
        assert_eq!(initial.mutation_sequence(), 1);
        assert_eq!(initial.cancel_reserved_at_epoch_ms(), 1_000);
        assert_eq!(initial.expires_at_epoch_ms(), 8_125);
        assert!(initial.cancel_requested());
        assert!(initial.encoded_bytes() <= 1_024);
        {
            let catalog = store.writer.lock().expect("inspect receipt catalog");
            assert_eq!(catalog.records.len(), 1);
            assert_eq!(catalog.actual_bytes, initial.encoded_bytes());
            assert_eq!(catalog.reserved_result_bytes, 0);
        }
        let row = receipts
            .join(ACTIVE_DIRECTORY_NAME)
            .join(format!("{}.json", initial.key_digest().as_str()));
        let bytes_before_duplicate = fs::read(&row).expect("read CancelReserved row");

        let duplicate = store
            .request_cancel_or_reserve(key.clone(), 4_000, reserve_deadline())
            .expect("repeat the exact cancellation reservation");
        let duplicate = match duplicate {
            crate::application::receipt_ledger::CancelResolution::ExistingExact(receipt) => receipt,
            other => panic!("duplicate cancel must reuse CancelReserved, got {other:?}"),
        };
        assert_eq!(duplicate, initial);
        assert_eq!(store.generation().expect("stable generation"), 1);
        let duplicate_with_irrelevant_overflow = store
            .request_cancel_or_reserve(
                key.clone(),
                u64::MAX - CANCEL_RESERVATION_TTL_MS + 1,
                reserve_deadline(),
            )
            .expect("an exact duplicate reuses the original timestamp before parsing a new one");
        assert_eq!(
            duplicate_with_irrelevant_overflow,
            crate::application::receipt_ledger::CancelResolution::ExistingExact(initial.clone())
        );
        assert_eq!(store.generation().expect("duplicate generation"), 1);
        assert_eq!(
            fs::read(&row).expect("reread exact CancelReserved row"),
            bytes_before_duplicate,
            "exact duplicate cannot extend TTL or rewrite durable bytes"
        );
        drop(store);

        let reopened = ReceiptLedgerStore::open(&receipts).expect("reopen receipt ledger");
        let state = reopened
            .recover_exact(&key, reserve_deadline())
            .expect("recover exact CancelReserved state");
        assert_eq!(
            state,
            ReceiptState::CancelReserved(initial),
            "reopen preserves the original absolute expiry and record identity"
        );
        let catalog = reopened.writer.lock().expect("inspect reopened catalog");
        assert_eq!(catalog.reserved_result_bytes, 0);
    }

    #[test]
    fn cancel_reserved_expires_at_the_absolute_boundary_and_releases_its_slot() {
        let root = tempfile::tempdir().expect("temporary root");
        let receipts = fs::canonicalize(root.path())
            .expect("physical temporary root")
            .join("receipts");
        let store = ReceiptLedgerStore::open(&receipts).expect("open receipt ledger");
        let key = receipt_key(INVOCATION_A, TASK_A, "workspace-a");
        let reserved = match store
            .request_cancel_or_reserve(key.clone(), 1_000, reserve_deadline())
            .expect("reserve cancellation before submit")
        {
            crate::application::receipt_ledger::CancelResolution::NewlyReserved(receipt) => receipt,
            other => panic!("first cancel must create CancelReserved, got {other:?}"),
        };

        assert_eq!(
            store
                .expire_cancel_reserved(
                    key.clone(),
                    reserved.record_version(),
                    reserved.mutation_sequence(),
                    8_124,
                    reserve_deadline(),
                )
                .expect("the half-open retention interval includes one millisecond before expiry"),
            crate::application::receipt_ledger::CancelExpiryOutcome::NotDue(reserved.clone())
        );
        assert_eq!(store.generation().expect("read pre-expiry generation"), 1);
        assert_eq!(
            store
                .expire_cancel_reserved(
                    key.clone(),
                    reserved.record_version(),
                    reserved.mutation_sequence(),
                    8_125,
                    reserve_deadline(),
                )
                .expect("the exact absolute boundary expires CancelReserved"),
            crate::application::receipt_ledger::CancelExpiryOutcome::Expired
        );
        assert_eq!(
            store
                .recover_exact(&key, reserve_deadline())
                .expect_err("expired receipt is no longer live"),
            ReceiptLedgerError::ReceiptNotFound
        );
        assert_eq!(store.generation().expect("expiry advances generation"), 2);
        {
            let catalog = store.writer.lock().expect("inspect expired catalog");
            assert!(catalog.records.is_empty());
            assert!(catalog.invocation_index.is_empty());
            assert!(catalog.reserved_task_index.is_empty());
            assert_eq!(catalog.actual_bytes, 0);
            assert_eq!(catalog.reserved_result_bytes, 0);
        }
        let row = receipts
            .join(ACTIVE_DIRECTORY_NAME)
            .join(format!("{}.json", reserved.key_digest().as_str()));
        assert!(!row.exists(), "expiry removes the durable payload row");
        drop(store);

        let reopened = ReceiptLedgerStore::open(&receipts).expect("reopen expired ledger");
        assert_eq!(reopened.generation().expect("reopened generation"), 2);
        assert_eq!(
            reopened
                .recover_exact(&key, reserve_deadline())
                .expect_err("expired exact key stays absent after reopen"),
            ReceiptLedgerError::ReceiptNotFound
        );
    }

    #[test]
    fn stale_expiry_cas_cannot_delete_a_recreated_cancel_reservation_with_version_one() {
        let root = tempfile::tempdir().expect("temporary root");
        let receipts = fs::canonicalize(root.path())
            .expect("physical temporary root")
            .join("receipts");
        let store = ReceiptLedgerStore::open(&receipts).expect("open receipt ledger");
        let key = receipt_key(INVOCATION_A, TASK_A, "workspace-a");
        let old = match store
            .request_cancel_or_reserve(key.clone(), 1_000, reserve_deadline())
            .expect("create old cancellation reservation")
        {
            CancelResolution::NewlyReserved(receipt) => receipt,
            other => panic!("old cancellation must be newly reserved, got {other:?}"),
        };
        assert_eq!(
            store
                .expire_cancel_reserved(
                    key.clone(),
                    old.record_version(),
                    old.mutation_sequence(),
                    old.expires_at_epoch_ms(),
                    reserve_deadline(),
                )
                .expect("expire old cancellation reservation"),
            CancelExpiryOutcome::Expired
        );
        let current = match store
            .request_cancel_or_reserve(key.clone(), 9_000, reserve_deadline())
            .expect("recreate the exact cancellation reservation")
        {
            CancelResolution::NewlyReserved(receipt) => receipt,
            other => panic!("recreated cancellation must be newly reserved, got {other:?}"),
        };
        assert_eq!(current.record_version(), ReceiptVersion::initial());
        assert_eq!(current.mutation_sequence(), 3);

        assert_eq!(
            store
                .expire_cancel_reserved(
                    key.clone(),
                    old.record_version(),
                    old.mutation_sequence(),
                    current.expires_at_epoch_ms(),
                    reserve_deadline(),
                )
                .expect_err("stale incarnation cannot delete the current version-one row"),
            ReceiptLedgerError::ReceiptMutationSequenceMismatch {
                expected: old.mutation_sequence(),
                actual: current.mutation_sequence(),
            }
        );
        assert_eq!(store.generation().expect("unchanged current generation"), 3);
        assert_eq!(
            store
                .recover_exact(&key, reserve_deadline())
                .expect("current cancellation survives stale expiry"),
            ReceiptState::CancelReserved(current)
        );
    }

    #[test]
    fn expired_deletion_witness_rejects_an_impossible_cancel_predecessor_version() {
        let key = receipt_key(INVOCATION_A, TASK_A, "workspace-a");
        let key_digest = receipt_key_digest(&key);
        let predecessor = CatalogEntry {
            record: build_cancel_reserved_record(key, key_digest.clone(), 1_000, 8_125, 1),
            encoded_bytes: 512,
        };
        let mut witness = build_expired_deletion_record(
            &predecessor,
            8_125,
            2,
            ReceiptVersion::new(2).expect("next version"),
        )
        .expect("build valid expiry witness");
        witness.record_version = ReceiptVersion::new(3).expect("impossible next version");
        match &mut witness.lifecycle {
            StoredActiveLifecycleV1::ExpiredDeletion {
                prior_record_version,
                ..
            } => *prior_record_version = ReceiptVersion::new(2).expect("impossible predecessor"),
            other => panic!("fixture must be an expiry witness, got {other:?}"),
        }
        let encoded = serde_json::to_vec(&witness).expect("encode canonical impossible witness");

        assert_eq!(
            validate_active_record(&witness, &encoded, &key_digest)
                .expect_err("CancelReserved can only expire from its initial version"),
            ReceiptLedgerError::Corrupt(
                "expired deletion witness predecessor is not an initial CancelReserved"
            )
        );
    }

    #[test]
    fn expired_deletion_witness_must_follow_its_predecessor_mutation() {
        let key = receipt_key(INVOCATION_A, TASK_A, "workspace-a");
        let key_digest = receipt_key_digest(&key);
        let predecessor = CatalogEntry {
            record: build_cancel_reserved_record(key, key_digest.clone(), 1_000, 8_125, 1),
            encoded_bytes: 512,
        };
        let witness = build_expired_deletion_record(
            &predecessor,
            8_125,
            1,
            ReceiptVersion::new(2).expect("next version"),
        )
        .expect("build sequence-one expiry witness fixture");
        let encoded = serde_json::to_vec(&witness).expect("encode canonical impossible witness");

        assert_eq!(
            validate_active_record(&witness, &encoded, &key_digest)
                .expect_err("deletion cannot share its predecessor mutation sequence"),
            ReceiptLedgerError::Corrupt(
                "expired deletion witness does not follow its predecessor mutation"
            )
        );
    }

    #[test]
    fn expired_deletion_witness_preserves_the_predecessor_fixed_absolute_ttl() {
        let key = receipt_key(INVOCATION_A, TASK_A, "workspace-a");
        let key_digest = receipt_key_digest(&key);
        let predecessor = CatalogEntry {
            record: build_cancel_reserved_record(key, key_digest.clone(), 1_000, 8_125, 1),
            encoded_bytes: 512,
        };
        let mut witness = build_expired_deletion_record(
            &predecessor,
            9_000,
            2,
            ReceiptVersion::new(2).expect("next version"),
        )
        .expect("build valid expiry witness");
        match &mut witness.lifecycle {
            StoredActiveLifecycleV1::ExpiredDeletion {
                prior_expires_at_epoch_ms,
                ..
            } => *prior_expires_at_epoch_ms = 9_000,
            other => panic!("fixture must be an expiry witness, got {other:?}"),
        }
        let encoded = serde_json::to_vec(&witness).expect("encode canonical impossible witness");

        assert_eq!(
            validate_active_record(&witness, &encoded, &key_digest)
                .expect_err("witness cannot rewrite the predecessor absolute TTL"),
            ReceiptLedgerError::Corrupt(
                "expired deletion witness predecessor expiry is not its fixed absolute TTL"
            )
        );
    }

    #[test]
    fn reopen_rejects_expiry_witness_invocation_collision_with_a_live_row() {
        let root = tempfile::tempdir().expect("temporary root");
        let receipts = fs::canonicalize(root.path())
            .expect("physical temporary root")
            .join("receipts");
        let live_key = receipt_key(INVOCATION_A, TASK_A, "workspace-a");
        {
            let store = ReceiptLedgerStore::open(&receipts).expect("open receipt ledger");
            store
                .request_cancel_or_reserve(live_key, 1_000, reserve_deadline())
                .expect("create live cancellation row");
        }
        write_expiry_witness_fixture(
            &receipts,
            receipt_key(INVOCATION_A, TASK_B, "workspace-b"),
            1,
            2,
        );

        assert_eq!(
            ReceiptLedgerStore::open(&receipts)
                .err()
                .expect("witness identity must participate in invocation collision checks"),
            ReceiptLedgerError::Corrupt("receipt catalog contains a duplicate invocation id")
        );
    }

    #[test]
    fn reopen_rejects_expiry_witness_task_collision_with_a_live_row() {
        let root = tempfile::tempdir().expect("temporary root");
        let receipts = fs::canonicalize(root.path())
            .expect("physical temporary root")
            .join("receipts");
        let live_key = receipt_key(INVOCATION_A, TASK_A, "workspace-a");
        {
            let store = ReceiptLedgerStore::open(&receipts).expect("open receipt ledger");
            store
                .request_cancel_or_reserve(live_key, 1_000, reserve_deadline())
                .expect("create live cancellation row");
        }
        write_expiry_witness_fixture(
            &receipts,
            receipt_key(INVOCATION_B, TASK_A, "workspace-b"),
            1,
            2,
        );

        assert_eq!(
            ReceiptLedgerStore::open(&receipts)
                .err()
                .expect("witness identity must participate in task collision checks"),
            ReceiptLedgerError::Corrupt("receipt catalog contains a duplicate reserved task id")
        );
    }

    #[test]
    fn reopen_rejects_a_mutation_sequence_shared_by_live_row_and_expiry_witness() {
        let root = tempfile::tempdir().expect("temporary root");
        let receipts = fs::canonicalize(root.path())
            .expect("physical temporary root")
            .join("receipts");
        let store = ReceiptLedgerStore::open(&receipts).expect("open receipt ledger");
        let live_key = receipt_key(INVOCATION_A, TASK_A, "workspace-a");
        let expiring_key = receipt_key(INVOCATION_B, TASK_B, "workspace-b");
        store
            .request_cancel_or_reserve(live_key.clone(), 1_000, reserve_deadline())
            .expect("create first cancellation reservation");
        let expiring = match store
            .request_cancel_or_reserve(expiring_key.clone(), 1_000, reserve_deadline())
            .expect("create second cancellation reservation")
        {
            CancelResolution::NewlyReserved(receipt) => receipt,
            other => panic!("second cancellation must be newly reserved, got {other:?}"),
        };
        set_after_receipt_row_rename_hook_for_test(|| {
            panic!("simulate process loss after the expiry witness rename")
        });
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let _ = store.expire_cancel_reserved(
                    expiring_key,
                    expiring.record_version(),
                    expiring.mutation_sequence(),
                    expiring.expires_at_epoch_ms(),
                    reserve_deadline(),
                );
            }))
            .is_err(),
            "expiry failpoint must leave its witness visible"
        );
        drop(store);

        let live_digest = receipt_key_digest(&live_key);
        let live_row = receipts
            .join(ACTIVE_DIRECTORY_NAME)
            .join(format!("{}.json", live_digest.as_str()));
        let mut record: StoredActiveReceiptV1 =
            serde_json::from_slice(&fs::read(&live_row).expect("read live cancellation row"))
                .expect("decode live cancellation row");
        record.mutation_sequence = 3;
        let (_, encoded) = serialize_reserved_record(record, MAX_CANCEL_RESERVED_RECORD_BYTES)
            .expect("encode canonical duplicate-sequence row");
        fs::write(&live_row, encoded).expect("persist duplicate-sequence fixture");

        assert_eq!(
            ReceiptLedgerStore::open(&receipts)
                .err()
                .expect("duplicate recovery sequence must be rejected"),
            ReceiptLedgerError::Corrupt("receipt recovery contains a duplicate mutation sequence")
        );
    }

    #[test]
    fn reopen_rejects_an_expiry_witness_that_skips_the_persisted_generation() {
        let root = tempfile::tempdir().expect("temporary root");
        let receipts = fs::canonicalize(root.path())
            .expect("physical temporary root")
            .join("receipts");
        let store = ReceiptLedgerStore::open(&receipts).expect("open receipt ledger");
        let key = receipt_key(INVOCATION_A, TASK_A, "workspace-a");
        let reserved = match store
            .request_cancel_or_reserve(key.clone(), 1_000, reserve_deadline())
            .expect("create cancellation reservation")
        {
            CancelResolution::NewlyReserved(receipt) => receipt,
            other => panic!("cancellation must be newly reserved, got {other:?}"),
        };
        set_after_receipt_row_rename_hook_for_test(|| {
            panic!("simulate process loss before expiry generation publication")
        });
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let _ = store.expire_cancel_reserved(
                    key.clone(),
                    reserved.record_version(),
                    reserved.mutation_sequence(),
                    reserved.expires_at_epoch_ms(),
                    reserve_deadline(),
                );
            }))
            .is_err(),
            "expiry failpoint must leave its witness visible"
        );
        drop(store);

        let row = receipts
            .join(ACTIVE_DIRECTORY_NAME)
            .join(format!("{}.json", reserved.key_digest().as_str()));
        let mut witness: StoredActiveReceiptV1 =
            serde_json::from_slice(&fs::read(&row).expect("read expiry witness"))
                .expect("decode expiry witness");
        assert!(matches!(
            &witness.lifecycle,
            StoredActiveLifecycleV1::ExpiredDeletion { .. }
        ));
        witness.mutation_sequence = 3;
        match &mut witness.lifecycle {
            StoredActiveLifecycleV1::ExpiredDeletion {
                prior_mutation_sequence,
                ..
            } => *prior_mutation_sequence = 2,
            other => panic!("fixture must remain an expiry witness, got {other:?}"),
        }
        let (_, encoded) = serialize_reserved_record(witness, MAX_CANCEL_RESERVED_RECORD_BYTES)
            .expect("encode canonical skipped-generation witness");
        fs::write(&row, encoded).expect("persist skipped-generation fixture");

        assert_eq!(
            ReceiptLedgerStore::open(&receipts)
                .err()
                .expect("witness cannot skip the next persisted generation"),
            ReceiptLedgerError::Corrupt(
                "expired deletion witness is not the next persisted mutation"
            )
        );
    }

    #[test]
    fn expiry_reopens_logically_absent_after_crash_between_witness_unlink_and_directory_sync() {
        let root = tempfile::tempdir().expect("temporary root");
        let receipts = fs::canonicalize(root.path())
            .expect("physical temporary root")
            .join("receipts");
        let store = ReceiptLedgerStore::open(&receipts).expect("open receipt ledger");
        let key = receipt_key(INVOCATION_A, TASK_A, "workspace-a");
        let reserved = match store
            .request_cancel_or_reserve(key.clone(), 1_000, reserve_deadline())
            .expect("create cancellation reservation")
        {
            CancelResolution::NewlyReserved(receipt) => receipt,
            other => panic!("cancellation must be newly reserved, got {other:?}"),
        };
        set_after_expired_deletion_witness_remove_hook_for_test(|| {
            panic!("simulate process loss before syncing the witness unlink")
        });

        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let _ = store.expire_cancel_reserved(
                    key.clone(),
                    reserved.record_version(),
                    reserved.mutation_sequence(),
                    reserved.expires_at_epoch_ms(),
                    reserve_deadline(),
                );
            }))
            .is_err(),
            "post-unlink failpoint must interrupt expiry"
        );
        drop(store);

        let reopened = ReceiptLedgerStore::open(&receipts)
            .expect("generation makes the receipt absent with or without durable unlink");
        assert_eq!(reopened.generation().expect("recovered generation"), 2);
        assert_eq!(
            reopened
                .recover_exact(&key, reserve_deadline())
                .expect_err("expired receipt cannot resurrect after the crash"),
            ReceiptLedgerError::ReceiptNotFound
        );
        assert!(
            !receipts
                .join(ACTIVE_DIRECTORY_NAME)
                .join(format!("{}.json", reserved.key_digest().as_str()))
                .exists(),
            "reopen finishes any witness cleanup"
        );
    }

    #[test]
    fn expiry_reopen_heals_generation_after_crash_at_visible_witness_rename() {
        let root = tempfile::tempdir().expect("temporary root");
        let receipts = fs::canonicalize(root.path())
            .expect("physical temporary root")
            .join("receipts");
        let store = ReceiptLedgerStore::open(&receipts).expect("open receipt ledger");
        let key = receipt_key(INVOCATION_A, TASK_A, "workspace-a");
        let reserved = match store
            .request_cancel_or_reserve(key.clone(), 1_000, reserve_deadline())
            .expect("create cancellation reservation")
        {
            CancelResolution::NewlyReserved(receipt) => receipt,
            other => panic!("cancellation must be newly reserved, got {other:?}"),
        };
        set_after_receipt_row_rename_hook_for_test(|| {
            panic!("simulate process loss at visible witness rename")
        });

        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let _ = store.expire_cancel_reserved(
                    key.clone(),
                    reserved.record_version(),
                    reserved.mutation_sequence(),
                    reserved.expires_at_epoch_ms(),
                    reserve_deadline(),
                );
            }))
            .is_err(),
            "witness rename failpoint must interrupt expiry"
        );
        drop(store);

        let reopened = ReceiptLedgerStore::open(&receipts)
            .expect("reopen must heal generation from the durable witness");
        assert_eq!(reopened.generation().expect("healed generation"), 2);
        assert_eq!(
            reopened
                .recover_exact(&key, reserve_deadline())
                .expect_err("witness is logically absent"),
            ReceiptLedgerError::ReceiptNotFound
        );
        let catalog = reopened.writer.lock().expect("inspect healed catalog");
        assert!(catalog.records.is_empty());
        assert!(catalog.invocation_index.is_empty());
        assert!(catalog.reserved_task_index.is_empty());
        assert_eq!(catalog.actual_bytes, 0);
        assert_eq!(catalog.reserved_result_bytes, 0);
        assert!(
            !receipts
                .join(ACTIVE_DIRECTORY_NAME)
                .join(format!("{}.json", reserved.key_digest().as_str()))
                .exists(),
            "reopen removes the healed witness"
        );
    }

    #[test]
    fn expiry_reopen_accepts_other_receipt_mutations_between_predecessor_and_witness() {
        let root = tempfile::tempdir().expect("temporary root");
        let receipts = fs::canonicalize(root.path())
            .expect("physical temporary root")
            .join("receipts");
        let store = ReceiptLedgerStore::open(&receipts).expect("open receipt ledger");
        let expiring_key = receipt_key(INVOCATION_A, TASK_A, "workspace-a");
        let surviving_key = receipt_key(INVOCATION_B, TASK_B, "workspace-b");
        let expiring = match store
            .request_cancel_or_reserve(expiring_key.clone(), 1_000, reserve_deadline())
            .expect("create first cancellation reservation")
        {
            CancelResolution::NewlyReserved(receipt) => receipt,
            other => panic!("first cancellation must be newly reserved, got {other:?}"),
        };
        let surviving = match store
            .request_cancel_or_reserve(surviving_key.clone(), 2_000, reserve_deadline())
            .expect("interleave a second receipt mutation")
        {
            CancelResolution::NewlyReserved(receipt) => receipt,
            other => panic!("second cancellation must be newly reserved, got {other:?}"),
        };
        assert_eq!(expiring.mutation_sequence(), 1);
        assert_eq!(surviving.mutation_sequence(), 2);
        set_after_receipt_row_rename_hook_for_test(|| {
            panic!("simulate process loss after interleaved expiry witness rename")
        });

        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let _ = store.expire_cancel_reserved(
                    expiring_key.clone(),
                    expiring.record_version(),
                    expiring.mutation_sequence(),
                    expiring.expires_at_epoch_ms(),
                    reserve_deadline(),
                );
            }))
            .is_err(),
            "witness rename failpoint must interrupt interleaved expiry"
        );
        drop(store);

        let reopened = ReceiptLedgerStore::open(&receipts)
            .expect("global sequence gaps are valid predecessor history");
        assert_eq!(reopened.generation().expect("healed generation"), 3);
        assert_eq!(
            reopened
                .recover_exact(&expiring_key, reserve_deadline())
                .expect_err("expired receipt remains absent"),
            ReceiptLedgerError::ReceiptNotFound
        );
        assert_eq!(
            reopened
                .recover_exact(&surviving_key, reserve_deadline())
                .expect("interleaved receipt survives recovery"),
            ReceiptState::CancelReserved(surviving)
        );
    }

    #[test]
    fn expiry_reopen_cleans_witness_after_crash_at_visible_generation_replace() {
        let root = tempfile::tempdir().expect("temporary root");
        let receipts = fs::canonicalize(root.path())
            .expect("physical temporary root")
            .join("receipts");
        let store = ReceiptLedgerStore::open(&receipts).expect("open receipt ledger");
        let key = receipt_key(INVOCATION_A, TASK_A, "workspace-a");
        let reserved = match store
            .request_cancel_or_reserve(key.clone(), 1_000, reserve_deadline())
            .expect("create cancellation reservation")
        {
            CancelResolution::NewlyReserved(receipt) => receipt,
            other => panic!("cancellation must be newly reserved, got {other:?}"),
        };
        set_after_generation_replace_hook_for_test(|| {
            panic!("simulate process loss before witness unlink")
        });

        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let _ = store.expire_cancel_reserved(
                    key.clone(),
                    reserved.record_version(),
                    reserved.mutation_sequence(),
                    reserved.expires_at_epoch_ms(),
                    reserve_deadline(),
                );
            }))
            .is_err(),
            "generation replace failpoint must interrupt expiry"
        );
        drop(store);

        let reopened = ReceiptLedgerStore::open(&receipts)
            .expect("reopen must accept either durable side of generation replacement");
        assert_eq!(reopened.generation().expect("recovered generation"), 2);
        assert_eq!(
            reopened
                .recover_exact(&key, reserve_deadline())
                .expect_err("published deletion cannot resurrect"),
            ReceiptLedgerError::ReceiptNotFound
        );
        assert!(
            !receipts
                .join(ACTIVE_DIRECTORY_NAME)
                .join(format!("{}.json", reserved.key_digest().as_str()))
                .exists(),
            "reopen removes the committed witness"
        );
    }

    #[test]
    fn cancel_reserved_shares_the_live_count_without_reserving_result_bytes() {
        let root = tempfile::tempdir().expect("temporary root");
        let receipts = fs::canonicalize(root.path())
            .expect("physical temporary root")
            .join("receipts");
        let store = ReceiptLedgerStore::open(&receipts).expect("open receipt ledger");

        for index in 0..MAX_LIVE_RECEIPTS {
            let key = receipt_key_with_ids(
                InvocationId::new(),
                TaskId::new(),
                &format!("cancel-workspace-{index}"),
            );
            assert!(matches!(
                store.request_cancel_or_reserve(key, 1_000, reserve_deadline()),
                Ok(crate::application::receipt_ledger::CancelResolution::NewlyReserved(_))
            ));
        }
        let overflow = store
            .request_cancel_or_reserve(
                receipt_key_with_ids(
                    InvocationId::new(),
                    TaskId::new(),
                    "cancel-workspace-overflow",
                ),
                1_000,
                reserve_deadline(),
            )
            .expect_err("the sixty-fifth live receipt must be rejected");
        assert_eq!(overflow, ReceiptLedgerError::CapacityExceeded);
        let catalog = store.writer.lock().expect("inspect full cancel catalog");
        assert_eq!(catalog.records.len(), MAX_LIVE_RECEIPTS);
        assert_eq!(catalog.reserved_result_bytes, 0);
        assert!(catalog.actual_bytes <= (MAX_LIVE_RECEIPTS * 1_024) as u64);
        assert!(catalog
            .records
            .values()
            .all(|entry| entry.encoded_bytes <= 1_024 && entry.reserved_result_bytes() == 0));
    }

    #[test]
    fn exact_submit_atomically_converts_cancel_reserved_to_full_cancelled_reservation() {
        let root = tempfile::tempdir().expect("temporary root");
        let receipts = fs::canonicalize(root.path())
            .expect("physical temporary root")
            .join("receipts");
        let store = ReceiptLedgerStore::open(&receipts).expect("open receipt ledger");
        let key = receipt_key(INVOCATION_A, TASK_A, "workspace-a");
        let cancel = store
            .request_cancel_or_reserve(key.clone(), 1_000, reserve_deadline())
            .expect("reserve cancellation before submit");
        assert!(matches!(
            cancel,
            crate::application::receipt_ledger::CancelResolution::NewlyReserved(_)
        ));
        let cutoff = OriginalCutoffDescriptor::new(2_000, 7_000).expect("valid submit cutoff");

        let converted = store
            .reserve(key.clone(), cutoff, reserve_deadline())
            .expect("convert the exact cancellation into a submit reservation");
        let converted = match converted {
            ReserveOutcome::Created(receipt) => receipt,
            other => panic!("cancel conversion is a durable mutation, got {other:?}"),
        };
        assert_eq!(converted.key(), &key);
        assert_eq!(converted.record_version().get(), 2);
        assert_eq!(converted.mutation_sequence(), 2);
        assert_eq!(converted.original_cutoff(), &cutoff);
        assert!(converted.cancel_requested());
        assert_eq!(
            converted.encoded_bytes() + converted.reserved_result_bytes(),
            MAX_RECEIPT_ENTITLEMENT_BYTES
        );
        assert_eq!(store.generation().expect("converted generation"), 2);
        {
            let catalog = store.writer.lock().expect("inspect converted catalog");
            assert_eq!(catalog.records.len(), 1);
            assert_eq!(
                catalog.actual_bytes + catalog.reserved_result_bytes,
                MAX_RECEIPT_ENTITLEMENT_BYTES
            );
        }
        drop(store);

        let reopened = ReceiptLedgerStore::open(&receipts).expect("reopen converted ledger");
        assert_eq!(
            reopened
                .recover_exact(&key, reserve_deadline())
                .expect("recover converted reservation"),
            ReceiptState::Reserved(converted)
        );
    }

    #[test]
    fn exact_submit_at_cancel_expiry_atomically_creates_a_fresh_uncancelled_reservation() {
        let root = tempfile::tempdir().expect("temporary root");
        let receipts = fs::canonicalize(root.path())
            .expect("physical temporary root")
            .join("receipts");
        let store = ReceiptLedgerStore::open(&receipts).expect("open receipt ledger");
        let key = receipt_key(INVOCATION_A, TASK_A, "workspace-a");
        let cancel = match store
            .request_cancel_or_reserve(key.clone(), 1_000, reserve_deadline())
            .expect("reserve cancellation before submit")
        {
            CancelResolution::NewlyReserved(receipt) => receipt,
            other => panic!("first cancellation must reserve, got {other:?}"),
        };
        let cutoff = OriginalCutoffDescriptor::new(cancel.expires_at_epoch_ms(), 7_000)
            .expect("valid submit cutoff at the half-open expiry boundary");

        let converted = match store
            .reserve(key.clone(), cutoff, reserve_deadline())
            .expect("replace expired cancellation with exact submit")
        {
            ReserveOutcome::Created(receipt) => receipt,
            other => panic!("expired exact conversion is a mutation, got {other:?}"),
        };

        assert!(!converted.cancel_requested());
        assert_eq!(converted.original_cutoff(), &cutoff);
        assert_eq!(converted.record_version().get(), 2);
        assert_eq!(converted.mutation_sequence(), 2);
        assert_eq!(store.generation().expect("converted generation"), 2);
        drop(store);

        assert_eq!(
            ReceiptLedgerStore::open(&receipts)
                .expect("reopen exact conversion")
                .recover_exact(&key, reserve_deadline())
                .expect("recover exact conversion"),
            ReceiptState::Reserved(converted)
        );
    }

    #[test]
    fn cancel_reserved_timestamp_overflow_and_partial_identity_collisions_do_not_mutate() {
        let root = tempfile::tempdir().expect("temporary root");
        let receipts = fs::canonicalize(root.path())
            .expect("physical temporary root")
            .join("receipts");
        let store = ReceiptLedgerStore::open(&receipts).expect("open receipt ledger");
        let key = receipt_key(INVOCATION_A, TASK_A, "workspace-a");
        assert_eq!(
            store
                .request_cancel_or_reserve(key.clone(), u64::MAX - 7_124, reserve_deadline(),)
                .expect_err("cancel expiry must not wrap"),
            ReceiptLedgerError::TimestampOverflow
        );
        assert_eq!(store.generation().expect("unchanged generation"), 0);

        store
            .request_cancel_or_reserve(key, 1_000, reserve_deadline())
            .expect("create the anchor CancelReserved");
        let invocation_collision = receipt_key(INVOCATION_A, TASK_B, "workspace-b");
        assert_eq!(
            store
                .request_cancel_or_reserve(invocation_collision, 1_000, reserve_deadline())
                .expect_err("partial invocation identity cannot cancel the anchor"),
            ReceiptLedgerError::InvocationIdentityMismatch
        );
        let task_collision = receipt_key(INVOCATION_B, TASK_A, "workspace-b");
        assert_eq!(
            store
                .request_cancel_or_reserve(task_collision, 1_000, reserve_deadline())
                .expect_err("partial task identity cannot cancel the anchor"),
            ReceiptLedgerError::ReservedTaskIdentityMismatch
        );
        assert_eq!(store.generation().expect("only anchor mutated"), 1);
        assert_eq!(
            store
                .writer
                .lock()
                .expect("inspect anchor catalog")
                .records
                .len(),
            1
        );
    }

    #[test]
    fn cancel_timestamp_overflow_cannot_bypass_an_already_fail_stopped_store() {
        let root = tempfile::tempdir().expect("temporary root");
        let receipts = fs::canonicalize(root.path())
            .expect("physical temporary root")
            .join("receipts");
        let store = ReceiptLedgerStore::open(&receipts).expect("open receipt ledger");
        let generation_path = receipts.join(GENERATION_FILE_NAME);
        if !set_unix_mode_for_test(&generation_path, 0o644).expect("weaken generation mode fixture")
        {
            return;
        }
        assert!(matches!(
            store
                .generation()
                .expect_err("authority drift fail-stops the store"),
            ReceiptLedgerError::Storage {
                operation: "verify generation record ownership",
                ..
            }
        ));
        set_unix_mode_for_test(&generation_path, 0o600).expect("restore generation mode fixture");
        let generation_before = fs::read(&generation_path).expect("read stable generation");
        let names_before = directory_names(&receipts.join(ACTIVE_DIRECTORY_NAME));

        assert_eq!(
            store
                .request_cancel_or_reserve(
                    receipt_key(INVOCATION_A, TASK_A, "workspace-a"),
                    u64::MAX - CANCEL_RESERVATION_TTL_MS + 1,
                    reserve_deadline(),
                )
                .expect_err("invalid input cannot bypass the latched store state"),
            ReceiptLedgerError::StoreUnavailable
        );
        assert_eq!(
            fs::read(&generation_path).expect("fail-stop leaves generation untouched"),
            generation_before
        );
        assert_eq!(
            directory_names(&receipts.join(ACTIVE_DIRECTORY_NAME)),
            names_before
        );
    }

    #[test]
    fn reserve_persists_exact_reserved_unbound_and_reopens_without_changing_cutoff() {
        let root = tempfile::tempdir().expect("temporary root");
        let receipts = fs::canonicalize(root.path())
            .expect("physical temporary root")
            .join("receipts");
        let key = receipt_key(INVOCATION_A, TASK_A, "workspace-a");
        let key_digest = crate::application::receipt_ledger::receipt_key_digest(&key);
        let cutoff =
            OriginalCutoffDescriptor::new(1_000, 7_000).expect("valid original response cutoff");

        let reserved = {
            let store = ReceiptLedgerStore::open(&receipts).expect("open receipt ledger");
            let outcome = store
                .reserve(key.clone(), cutoff, reserve_deadline())
                .expect("durably reserve exact receipt");
            assert!(matches!(outcome, ReserveOutcome::Created(_)));
            outcome
                .into_reservation()
                .expect("created receipt remains reserved")
        };

        let reopened = ReceiptLedgerStore::open(&receipts).expect("reopen receipt ledger");
        let recovered = reopened
            .read_reserved(&key_digest)
            .expect("read reserved receipt")
            .expect("reserved receipt survives reopen");
        assert_eq!(recovered, reserved);
        assert_eq!(recovered.key(), &key);
        assert_eq!(recovered.original_cutoff(), &cutoff);
        assert_eq!(reopened.generation().expect("reopened generation"), 1);
    }

    #[test]
    fn receipt_record_version_is_per_record_and_distinct_from_global_mutation_sequence() {
        let root = tempfile::tempdir().expect("temporary root");
        let receipts = fs::canonicalize(root.path())
            .expect("physical temporary root")
            .join("receipts");
        let store = ReceiptLedgerStore::open(&receipts).expect("open receipt ledger");
        let cutoff =
            OriginalCutoffDescriptor::new(1_000, 7_000).expect("valid original response cutoff");
        let first_key = receipt_key(INVOCATION_A, TASK_A, "workspace-a");
        let first_digest = receipt_key_digest(&first_key);
        let second_key = receipt_key(INVOCATION_B, TASK_B, "workspace-b");
        let second_digest = receipt_key_digest(&second_key);

        let first = store
            .reserve(first_key, cutoff, reserve_deadline())
            .expect("reserve first receipt")
            .into_reservation()
            .expect("receipt remains reserved");
        let second = store
            .reserve(second_key, cutoff, reserve_deadline())
            .expect("reserve second receipt")
            .into_reservation()
            .expect("receipt remains reserved");

        assert_eq!(first.record_version(), ReceiptVersion::initial());
        assert_eq!(first.mutation_sequence(), 1);
        assert_eq!(second.record_version(), ReceiptVersion::initial());
        assert_eq!(second.mutation_sequence(), 2);
        drop(store);

        let reopened = ReceiptLedgerStore::open(&receipts).expect("reopen receipt ledger");
        let first = reopened
            .read_reserved(&first_digest)
            .expect("read first receipt")
            .expect("first receipt survives reopen");
        let second = reopened
            .read_reserved(&second_digest)
            .expect("read second receipt")
            .expect("second receipt survives reopen");
        assert_eq!(first.record_version(), ReceiptVersion::initial());
        assert_eq!(first.mutation_sequence(), 1);
        assert_eq!(second.record_version(), ReceiptVersion::initial());
        assert_eq!(second.mutation_sequence(), 2);
    }

    #[test]
    fn recover_port_returns_the_exact_reserved_state_without_mutating_generation() {
        let root = tempfile::tempdir().expect("temporary root");
        let receipts = fs::canonicalize(root.path())
            .expect("physical temporary root")
            .join("receipts");
        let mut store = ReceiptLedgerStore::open(&receipts).expect("open receipt ledger");
        let key = receipt_key(INVOCATION_A, TASK_A, "workspace-a");
        let cutoff =
            OriginalCutoffDescriptor::new(1_000, 7_000).expect("valid original response cutoff");
        let reserved = store
            .reserve(key.clone(), cutoff, reserve_deadline())
            .expect("reserve exact receipt")
            .into_reservation()
            .expect("receipt remains reserved");
        let generation_before = store.generation().expect("generation before recovery");

        let recovered = ReceiptLedgerPort::recover(&mut store, &key, reserve_deadline())
            .expect("recover exact receipt");

        assert_eq!(recovered, ReceiptState::Reserved(reserved));
        assert_eq!(
            store.generation().expect("generation after recovery"),
            generation_before,
            "read-only recovery must not publish a mutation"
        );
    }

    #[test]
    fn direct_terminal_replace_advances_exact_record_and_generation_once() {
        let root = tempfile::tempdir().expect("temporary root");
        let receipts = fs::canonicalize(root.path())
            .expect("physical temporary root")
            .join("receipts");
        let store = ReceiptLedgerStore::open(&receipts).expect("open receipt ledger");
        let key = receipt_key(INVOCATION_A, TASK_A, "workspace-a");
        let cutoff =
            OriginalCutoffDescriptor::new(1_000, 7_000).expect("valid original response cutoff");
        let reserved = store
            .reserve(key.clone(), cutoff, reserve_deadline())
            .expect("reserve exact receipt")
            .into_reservation()
            .expect("receipt remains reserved");
        let terminal = canonical_v5_terminal(&ReceiptTerminalOutcome::Completed {
            result: Box::new(DomainResult::success("direct result")),
        })
        .expect("canonical direct terminal");

        let committed = store
            .publish_direct_terminal(
                &key,
                reserved.record_version(),
                2_000,
                terminal.clone(),
                reserve_deadline(),
            )
            .expect("publish exact direct terminal");

        assert_eq!(committed.key(), &key);
        assert_eq!(committed.record_version().get(), 2);
        assert_eq!(committed.mutation_sequence(), 2);
        assert_eq!(committed.terminal_epoch_ms(), 2_000);
        assert_eq!(committed.terminal(), &terminal);
        assert_eq!(
            committed.encoded_bytes() + committed.reserved_result_bytes(),
            MAX_RECEIPT_ENTITLEMENT_BYTES
        );
        assert_eq!(store.generation().expect("generation after terminal"), 2);
    }

    #[test]
    fn direct_terminal_reopens_byte_equivalent_with_exact_state() {
        let root = tempfile::tempdir().expect("temporary root");
        let receipts = fs::canonicalize(root.path())
            .expect("physical temporary root")
            .join("receipts");
        let mut store = ReceiptLedgerStore::open(&receipts).expect("open receipt ledger");
        let key = receipt_key(INVOCATION_A, TASK_A, "workspace-a");
        let key_digest = receipt_key_digest(&key);
        let cutoff =
            OriginalCutoffDescriptor::new(1_000, 7_000).expect("valid original response cutoff");
        let reserved = store
            .reserve(key.clone(), cutoff, reserve_deadline())
            .expect("reserve exact receipt")
            .into_reservation()
            .expect("receipt remains reserved");
        let terminal = canonical_v5_terminal(&ReceiptTerminalOutcome::Completed {
            result: Box::new(DomainResult::success("restart-stable direct result")),
        })
        .expect("canonical direct terminal");
        let committed = store
            .publish_direct_terminal(
                &key,
                reserved.record_version(),
                2_000,
                terminal,
                reserve_deadline(),
            )
            .expect("publish exact direct terminal");
        let row_path = receipts
            .join(ACTIVE_DIRECTORY_NAME)
            .join(format!("{}.json", key_digest.as_str()));
        let committed_bytes = fs::read(&row_path).expect("read committed direct terminal row");

        assert_eq!(
            ReceiptLedgerPort::recover(&mut store, &key, reserve_deadline())
                .expect("recover live direct terminal"),
            ReceiptState::DirectTerminalUnacked(committed.clone())
        );
        assert_eq!(
            fs::read(&row_path).expect("read row after live recover"),
            committed_bytes,
            "live recover must be byte-for-byte read-only"
        );
        drop(store);

        let mut reopened = ReceiptLedgerStore::open(&receipts).expect("reopen receipt ledger");
        assert_eq!(
            ReceiptLedgerPort::recover(&mut reopened, &key, reserve_deadline())
                .expect("recover reopened direct terminal"),
            ReceiptState::DirectTerminalUnacked(committed)
        );
        assert_eq!(
            fs::read(&row_path).expect("read row after reopen recover"),
            committed_bytes,
            "reopen and recover must preserve exact terminal bytes"
        );
        assert_eq!(reopened.generation().expect("reopened generation"), 2);
    }

    #[test]
    fn direct_terminal_persists_the_original_cutoff_for_exact_response_identity() {
        let root = tempfile::tempdir().expect("temporary root");
        let receipts = fs::canonicalize(root.path())
            .expect("physical temporary root")
            .join("receipts");
        let store = ReceiptLedgerStore::open(&receipts).expect("open receipt ledger");
        let key = receipt_key(INVOCATION_A, TASK_A, "workspace-a");
        let key_digest = receipt_key_digest(&key);
        let cutoff =
            OriginalCutoffDescriptor::new(1_000, 7_000).expect("valid original response cutoff");
        let reserved = store
            .reserve(key.clone(), cutoff, reserve_deadline())
            .expect("reserve exact receipt")
            .into_reservation()
            .expect("receipt remains reserved");
        let terminal = canonical_v5_terminal(&ReceiptTerminalOutcome::Cancelled)
            .expect("canonical direct terminal");

        store
            .publish_direct_terminal(
                &key,
                reserved.record_version(),
                2_000,
                terminal,
                reserve_deadline(),
            )
            .expect("publish exact direct terminal");
        let row = fs::read_to_string(
            receipts
                .join(ACTIVE_DIRECTORY_NAME)
                .join(format!("{}.json", key_digest.as_str())),
        )
        .expect("read direct terminal row");

        assert!(
            row.contains("\"originalCutoff\":{\"acceptedEpochMs\":1000,\"responseBudgetMs\":7000}"),
            "Direct must retain the original accepted epoch and response budget"
        );
    }

    #[test]
    fn direct_terminal_writes_the_preflighted_record_and_returns_the_same_wire_frame() {
        let root = tempfile::tempdir().expect("temporary root");
        let receipts = fs::canonicalize(root.path())
            .expect("physical temporary root")
            .join("receipts");
        let store = ReceiptLedgerStore::open(&receipts).expect("open receipt ledger");
        let key = receipt_key(INVOCATION_A, TASK_A, "workspace-a");
        let key_digest = receipt_key_digest(&key);
        let cutoff =
            OriginalCutoffDescriptor::new(1_000, 7_000).expect("valid original response cutoff");
        let reserved = store
            .reserve(key.clone(), cutoff, reserve_deadline())
            .expect("reserve exact receipt")
            .into_reservation()
            .expect("receipt remains reserved");
        let terminal = canonical_v5_terminal(&ReceiptTerminalOutcome::Cancelled)
            .expect("canonical direct terminal");
        let expected = crate::infrastructure::daemon::terminal_codec_v5::prepare_direct_terminal(
            crate::infrastructure::daemon::terminal_codec_v5::DirectReceiptWriteSlot::new(
                &key,
                reserved.record_version(),
                reserved
                    .record_version()
                    .checked_next()
                    .expect("next record version"),
                reserved.mutation_sequence(),
                reserved
                    .mutation_sequence()
                    .checked_add(1)
                    .expect("next mutation sequence"),
                cutoff,
            )
            .expect("exact ledger write slot"),
            terminal.clone(),
            2_000,
        )
        .expect("prepare expected publication");

        let committed = store
            .publish_direct_terminal_publication(
                &key,
                reserved.record_version(),
                2_000,
                terminal,
                reserve_deadline(),
            )
            .expect("commit exact direct publication");
        let row_path = receipts
            .join(ACTIVE_DIRECTORY_NAME)
            .join(format!("{}.json", key_digest.as_str()));

        assert_eq!(
            fs::read(row_path).expect("read committed Direct record"),
            expected.record().bytes()
        );
        assert_eq!(
            committed.wire_frame().jsonl(),
            expected.wire_frame().jsonl()
        );
        assert_eq!(committed.receipt().terminal(), expected.record().terminal());
    }

    #[test]
    fn exact_duplicate_direct_after_reopen_reads_the_existing_lifecycle_without_mutation() {
        let root = tempfile::tempdir().expect("temporary root");
        let receipts = fs::canonicalize(root.path())
            .expect("physical temporary root")
            .join("receipts");
        let store = ReceiptLedgerStore::open(&receipts).expect("open receipt ledger");
        let key = receipt_key(INVOCATION_A, TASK_A, "workspace-a");
        let key_digest = receipt_key_digest(&key);
        let original_cutoff =
            OriginalCutoffDescriptor::new(1_000, 7_000).expect("valid original response cutoff");
        let reserved = store
            .reserve(key.clone(), original_cutoff, reserve_deadline())
            .expect("reserve exact receipt")
            .into_reservation()
            .expect("receipt remains reserved");
        let terminal = canonical_v5_terminal(&ReceiptTerminalOutcome::Completed {
            result: Box::new(DomainResult::success("stable duplicate result")),
        })
        .expect("canonical direct terminal");
        let committed = store
            .publish_direct_terminal(
                &key,
                reserved.record_version(),
                2_000,
                terminal,
                reserve_deadline(),
            )
            .expect("publish exact direct terminal");
        let row_path = receipts
            .join(ACTIVE_DIRECTORY_NAME)
            .join(format!("{}.json", key_digest.as_str()));
        let winner_bytes = fs::read(&row_path).expect("read direct winner");
        drop(store);

        let mut reopened = ReceiptLedgerStore::open(&receipts).expect("reopen receipt ledger");
        let generation_before = reopened.generation().expect("generation before duplicate");
        let changed_cutoff =
            OriginalCutoffDescriptor::new(9_000, 1_000).expect("valid changed retry cutoff");
        let duplicate = reopened
            .reserve(key.clone(), changed_cutoff, reserve_deadline())
            .expect("exact duplicate must read the committed lifecycle");

        assert!(matches!(duplicate, ReserveOutcome::ExistingExact(_)));
        assert_eq!(
            ReceiptLedgerPort::recover(&mut reopened, &key, reserve_deadline())
                .expect("recover duplicate direct lifecycle"),
            ReceiptState::DirectTerminalUnacked(committed)
        );
        assert_eq!(
            reopened.generation().expect("unchanged generation"),
            generation_before
        );
        assert_eq!(
            fs::read(&row_path).expect("read unchanged winner"),
            winner_bytes
        );
    }

    #[test]
    fn reopen_rejects_direct_terminal_at_the_impossible_initial_record_version() {
        let root = tempfile::tempdir().expect("temporary root");
        let receipts = fs::canonicalize(root.path())
            .expect("physical temporary root")
            .join("receipts");
        let store = ReceiptLedgerStore::open(&receipts).expect("open receipt ledger");
        let key = receipt_key(INVOCATION_A, TASK_A, "workspace-a");
        let key_digest = receipt_key_digest(&key);
        let cutoff =
            OriginalCutoffDescriptor::new(1_000, 7_000).expect("valid original response cutoff");
        let reserved = store
            .reserve(key.clone(), cutoff, reserve_deadline())
            .expect("reserve exact receipt")
            .into_reservation()
            .expect("receipt remains reserved");
        store
            .publish_direct_terminal(
                &key,
                reserved.record_version(),
                2_000,
                canonical_v5_terminal(&ReceiptTerminalOutcome::Cancelled)
                    .expect("canonical direct terminal"),
                reserve_deadline(),
            )
            .expect("publish exact direct terminal");
        let row_path = receipts
            .join(ACTIVE_DIRECTORY_NAME)
            .join(format!("{}.json", key_digest.as_str()));
        let row = fs::read_to_string(&row_path).expect("read direct row");
        assert!(row.contains("\"recordVersion\":2"));
        drop(store);
        fs::write(
            &row_path,
            row.replacen("\"recordVersion\":2", "\"recordVersion\":1", 1),
        )
        .expect("forge impossible direct version");

        assert_eq!(
            ReceiptLedgerStore::open(&receipts)
                .err()
                .expect("reopen must reject impossible direct version"),
            ReceiptLedgerError::Corrupt("direct terminal receipt must advance its record version")
        );
    }

    #[test]
    fn direct_terminal_record_is_strict_canonical_schema_v1() {
        let root = tempfile::tempdir().expect("temporary root");
        let receipts = fs::canonicalize(root.path())
            .expect("physical temporary root")
            .join("receipts");
        let store = ReceiptLedgerStore::open(&receipts).expect("open receipt ledger");
        let key = receipt_key(INVOCATION_A, TASK_A, "workspace-a");
        let key_digest = receipt_key_digest(&key);
        let cutoff =
            OriginalCutoffDescriptor::new(1_000, 7_000).expect("valid original response cutoff");
        let reserved = store
            .reserve(key.clone(), cutoff, reserve_deadline())
            .expect("reserve exact receipt")
            .into_reservation()
            .expect("receipt remains reserved");
        let terminal = canonical_v5_terminal(&ReceiptTerminalOutcome::Completed {
            result: Box::new(DomainResult::success("strict direct result")),
        })
        .expect("canonical direct terminal");
        let terminal_digest = terminal.digest().clone();
        store
            .publish_direct_terminal(
                &key,
                reserved.record_version(),
                2_000,
                terminal,
                reserve_deadline(),
            )
            .expect("publish exact direct terminal");
        let row_path = receipts
            .join(ACTIVE_DIRECTORY_NAME)
            .join(format!("{}.json", key_digest.as_str()));
        let row = fs::read_to_string(&row_path).expect("read direct row as UTF-8");

        assert!(
            !row.ends_with('\n'),
            "persisted receipt records are JSON objects, not JSONL wire frames"
        );
        assert!(row.contains("\"schemaVersion\":1"));
        assert!(row.contains("\"recordVersion\":2"));
        assert!(row.contains("\"state\":\"direct_terminal_unacked\""));
        assert!(row.contains("\"terminalEpochMs\":2000"));
        assert!(row.contains("\"terminalDigest\":"));
        assert!(row.contains("\"terminal\":{\"status\":\"completed\""));
        assert!(
            row.contains("\"originalCutoff\":{\"acceptedEpochMs\":1000,\"responseBudgetMs\":7000}")
        );
        assert!(!row.contains("reservedAtEpochMs"));
        assert!(!row.contains("cancelRequested"));
        assert!(serde_json::from_str::<StoredActiveReceiptV1>(&row).is_ok());
        assert!(
            serde_json::from_str::<StoredActiveReceiptV1>(&row.replacen(
                "\"terminalEpochMs\":2000",
                "\"terminalEpochMs\":2000,\"unexpected\":true",
                1,
            ))
            .is_err(),
            "direct lifecycle body must reject unknown fields"
        );
        assert!(
            serde_json::from_str::<StoredActiveReceiptV1>(&row.replacen(
                &format!("\"terminalDigest\":\"{}\",", terminal_digest.as_str()),
                "",
                1,
            ))
            .is_err(),
            "direct lifecycle body must require its terminal digest"
        );
        drop(store);

        let forged = row.replacen(terminal_digest.as_str(), &"0".repeat(64), 1);
        fs::write(&row_path, forged).expect("persist forged terminal digest");
        let error = ReceiptLedgerStore::open(&receipts)
            .err()
            .expect("reopen must reject a noncanonical terminal digest");
        assert_eq!(
            error,
            ReceiptLedgerError::Corrupt(
                "receipt terminal digest does not match its canonical outcome"
            )
        );
    }

    #[test]
    fn direct_terminal_repeat_preserves_first_winner_and_clean_conflicts_do_not_latch() {
        let root = tempfile::tempdir().expect("temporary root");
        let receipts = fs::canonicalize(root.path())
            .expect("physical temporary root")
            .join("receipts");
        let mut store = ReceiptLedgerStore::open(&receipts).expect("open receipt ledger");
        let key = receipt_key(INVOCATION_A, TASK_A, "workspace-a");
        let key_digest = receipt_key_digest(&key);
        let cutoff =
            OriginalCutoffDescriptor::new(1_000, 7_000).expect("valid original response cutoff");
        let reserved = store
            .reserve(key.clone(), cutoff, reserve_deadline())
            .expect("reserve exact receipt")
            .into_reservation()
            .expect("receipt remains reserved");
        let first_terminal = canonical_v5_terminal(&ReceiptTerminalOutcome::Completed {
            result: Box::new(DomainResult::success("first winner")),
        })
        .expect("canonical first terminal");
        let first_publication = store
            .publish_direct_terminal_publication(
                &key,
                reserved.record_version(),
                2_000,
                first_terminal.clone(),
                reserve_deadline(),
            )
            .expect("publish first terminal winner");
        let first_wire = first_publication.wire_frame().jsonl().to_vec();
        let committed = first_publication.into_parts().0;
        let row_path = receipts
            .join(ACTIVE_DIRECTORY_NAME)
            .join(format!("{}.json", key_digest.as_str()));
        let winner_bytes = fs::read(&row_path).expect("read first terminal winner");

        let repeated = store
            .publish_direct_terminal_publication(
                &key,
                reserved.record_version(),
                2_000,
                first_terminal,
                reserve_deadline(),
            )
            .expect("exact repeat returns the committed winner and a preflighted frame");
        assert_eq!(repeated.receipt(), &committed);
        assert_eq!(repeated.wire_frame().jsonl(), first_wire);
        assert_eq!(store.generation().expect("unchanged generation"), 2);
        assert_eq!(
            fs::read(&row_path).expect("read winner after exact repeat"),
            winner_bytes
        );

        let foreign_terminal = canonical_v5_terminal(&ReceiptTerminalOutcome::Cancelled)
            .expect("canonical foreign terminal");
        assert_eq!(
            store
                .publish_direct_terminal(
                    &key,
                    reserved.record_version(),
                    2_001,
                    foreign_terminal,
                    reserve_deadline(),
                )
                .expect_err("a different terminal cannot replace the first winner"),
            ReceiptLedgerError::TerminalMismatch
        );
        assert_eq!(store.generation().expect("unchanged generation"), 2);
        assert_eq!(
            fs::read(&row_path).expect("read winner after terminal mismatch"),
            winner_bytes
        );
        assert_eq!(
            ReceiptLedgerPort::recover(&mut store, &key, reserve_deadline())
                .expect("clean conflict keeps the store reusable"),
            ReceiptState::DirectTerminalUnacked(committed)
        );

        let second_key = receipt_key(INVOCATION_B, TASK_B, "workspace-b");
        let second = store
            .reserve(second_key.clone(), cutoff, reserve_deadline())
            .expect("reserve second receipt")
            .into_reservation()
            .expect("receipt remains reserved");
        let terminal = canonical_v5_terminal(&ReceiptTerminalOutcome::Cancelled)
            .expect("canonical second terminal");
        assert_eq!(
            store
                .publish_direct_terminal(
                    &second_key,
                    ReceiptVersion::new(2).expect("nonzero stale expected version"),
                    3_000,
                    terminal,
                    reserve_deadline(),
                )
                .expect_err("stale expected version cannot replace a reservation"),
            ReceiptLedgerError::ReceiptVersionMismatch {
                expected: ReceiptVersion::new(2).expect("nonzero expected version"),
                actual: second.record_version(),
            }
        );
        assert!(matches!(
            ReceiptLedgerPort::recover(&mut store, &second_key, reserve_deadline()),
            Ok(ReceiptState::Reserved(_))
        ));
    }

    #[test]
    fn direct_terminal_catalog_invariant_failure_latches_the_live_writer() {
        let root = tempfile::tempdir().expect("temporary root");
        let receipts = fs::canonicalize(root.path())
            .expect("physical temporary root")
            .join("receipts");
        let mut store = ReceiptLedgerStore::open(&receipts).expect("open receipt ledger");
        let key = receipt_key(INVOCATION_A, TASK_A, "workspace-a");
        let cutoff =
            OriginalCutoffDescriptor::new(1_000, 7_000).expect("valid original response cutoff");
        let reserved = store
            .reserve(key.clone(), cutoff, reserve_deadline())
            .expect("reserve exact receipt")
            .into_reservation()
            .expect("receipt remains reserved");
        store
            .writer
            .lock()
            .expect("lock receipt catalog fixture")
            .actual_bytes = 0;
        let terminal = canonical_v5_terminal(&ReceiptTerminalOutcome::Cancelled)
            .expect("canonical direct terminal");

        assert_eq!(
            store
                .publish_direct_terminal(
                    &key,
                    reserved.record_version(),
                    2_000,
                    terminal,
                    reserve_deadline(),
                )
                .expect_err("catalog accounting corruption cannot publish a terminal"),
            ReceiptLedgerError::Corrupt("receipt catalog actual-byte accounting underflowed")
        );
        assert_eq!(
            ReceiptLedgerPort::recover(&mut store, &key, reserve_deadline())
                .expect_err("catalog invariant failure must fail-stop the live writer"),
            ReceiptLedgerError::StoreUnavailable
        );
    }

    #[test]
    fn direct_terminal_reader_accepts_payload_above_the_legacy_64_kib_bound() {
        let root = tempfile::tempdir().expect("temporary root");
        let receipts = fs::canonicalize(root.path())
            .expect("physical temporary root")
            .join("receipts");
        let store = ReceiptLedgerStore::open(&receipts).expect("open receipt ledger");
        let key = receipt_key(INVOCATION_A, TASK_A, "workspace-a");
        let key_digest = receipt_key_digest(&key);
        let cutoff =
            OriginalCutoffDescriptor::new(1_000, 7_000).expect("valid original response cutoff");
        let reserved = store
            .reserve(key.clone(), cutoff, reserve_deadline())
            .expect("reserve exact receipt")
            .into_reservation()
            .expect("receipt remains reserved");
        let terminal = canonical_v5_terminal(&ReceiptTerminalOutcome::Completed {
            result: Box::new(DomainResult::success("x".repeat(
                crate::application::invocation_store::MAX_CANONICAL_RESULT_BYTES - 4_096,
            ))),
        })
        .expect("canonical near-limit direct terminal");
        let committed = store
            .publish_direct_terminal(
                &key,
                reserved.record_version(),
                2_000,
                terminal,
                reserve_deadline(),
            )
            .expect("publish near-limit direct terminal");
        let row_path = receipts
            .join(ACTIVE_DIRECTORY_NAME)
            .join(format!("{}.json", key_digest.as_str()));
        assert!(
            fs::metadata(&row_path)
                .expect("near-limit direct row metadata")
                .len()
                > MAX_TASK_RECORD_ENVELOPE_BYTES as u64,
            "the fixture must cross the old 64 KiB reader ceiling"
        );
        drop(store);

        let mut reopened = ReceiptLedgerStore::open(&receipts)
            .expect("reopen must read the full direct-terminal bound");
        assert_eq!(
            ReceiptLedgerPort::recover(&mut reopened, &key, reserve_deadline())
                .expect("recover near-limit direct terminal"),
            ReceiptState::DirectTerminalUnacked(committed)
        );
    }

    #[test]
    fn direct_terminal_after_rename_sync_failure_is_uncertain_and_reopens_the_winner() {
        let root = tempfile::tempdir().expect("temporary root");
        let receipts = fs::canonicalize(root.path())
            .expect("physical temporary root")
            .join("receipts");
        let mut store = ReceiptLedgerStore::open(&receipts).expect("open receipt ledger");
        let key = receipt_key(INVOCATION_A, TASK_A, "workspace-a");
        let key_digest = receipt_key_digest(&key);
        let cutoff =
            OriginalCutoffDescriptor::new(1_000, 7_000).expect("valid original response cutoff");
        let reserved = store
            .reserve(key.clone(), cutoff, reserve_deadline())
            .expect("reserve exact receipt")
            .into_reservation()
            .expect("receipt remains reserved");
        let terminal = canonical_v5_terminal(&ReceiptTerminalOutcome::Completed {
            result: Box::new(DomainResult::success("visible uncertain winner")),
        })
        .expect("canonical direct terminal");
        inject_receipt_row_directory_sync_failure_for_test();

        assert_eq!(
            store
                .publish_direct_terminal(
                    &key,
                    reserved.record_version(),
                    2_000,
                    terminal.clone(),
                    reserve_deadline(),
                )
                .expect_err("post-rename sync failure cannot report a clean outcome"),
            ReceiptLedgerError::CommitUncertain {
                receipt_key_digest: key_digest,
            }
        );
        assert_eq!(
            ReceiptLedgerPort::recover(&mut store, &key, reserve_deadline())
                .expect_err("uncertain live writer stays fail-stopped"),
            ReceiptLedgerError::StoreUnavailable
        );
        drop(store);

        let mut reopened = ReceiptLedgerStore::open(&receipts)
            .expect("process-owned reopen resolves the visible direct winner");
        let recovered = ReceiptLedgerPort::recover(&mut reopened, &key, reserve_deadline())
            .expect("recover exact direct winner after uncertain commit");
        let ReceiptState::DirectTerminalUnacked(recovered) = recovered else {
            panic!("uncertain direct publication reopened as a different state")
        };
        assert_eq!(recovered.terminal_epoch_ms(), 2_000);
        assert_eq!(recovered.terminal(), &terminal);
        assert_eq!(recovered.record_version().get(), 2);
        assert_eq!(reopened.generation().expect("healed generation"), 2);
    }

    #[test]
    fn recover_port_returns_receipt_not_found_only_for_a_stably_missing_exact_key() {
        let root = tempfile::tempdir().expect("temporary root");
        let receipts = fs::canonicalize(root.path())
            .expect("physical temporary root")
            .join("receipts");
        let mut store = ReceiptLedgerStore::open(&receipts).expect("open receipt ledger");
        let key = receipt_key(INVOCATION_A, TASK_A, "workspace-a");

        let error = ReceiptLedgerPort::recover(&mut store, &key, reserve_deadline())
            .expect_err("stably missing exact receipt has a typed absence");

        assert_eq!(error, ReceiptLedgerError::ReceiptNotFound);
        assert_eq!(store.generation().expect("unchanged generation"), 0);
    }

    #[test]
    fn recover_rejects_same_invocation_id_bound_to_a_different_exact_key() {
        let root = tempfile::tempdir().expect("temporary root");
        let receipts = fs::canonicalize(root.path())
            .expect("physical temporary root")
            .join("receipts");
        let mut store = ReceiptLedgerStore::open(&receipts).expect("open receipt ledger");
        let original = receipt_key(INVOCATION_A, TASK_A, "workspace-a");
        let mismatch = receipt_key(INVOCATION_A, TASK_B, "workspace-b");
        let cutoff =
            OriginalCutoffDescriptor::new(1_000, 7_000).expect("valid original response cutoff");
        store
            .reserve(original.clone(), cutoff, reserve_deadline())
            .expect("reserve original exact receipt");
        let generation = store.generation().expect("generation after reserve");

        assert_eq!(
            ReceiptLedgerPort::recover(&mut store, &mismatch, reserve_deadline())
                .expect_err("partial invocation-id collision must not look absent"),
            ReceiptLedgerError::InvocationIdentityMismatch
        );
        assert!(matches!(
            ReceiptLedgerPort::recover(&mut store, &original, reserve_deadline()),
            Ok(ReceiptState::Reserved(_))
        ));
        assert_eq!(
            store.generation().expect("unchanged generation"),
            generation
        );
    }

    #[test]
    fn recover_rejects_same_reserved_task_id_bound_to_a_different_exact_key() {
        let root = tempfile::tempdir().expect("temporary root");
        let receipts = fs::canonicalize(root.path())
            .expect("physical temporary root")
            .join("receipts");
        let mut store = ReceiptLedgerStore::open(&receipts).expect("open receipt ledger");
        let original = receipt_key(INVOCATION_A, TASK_A, "workspace-a");
        let mismatch = receipt_key(INVOCATION_B, TASK_A, "workspace-b");
        let cutoff =
            OriginalCutoffDescriptor::new(1_000, 7_000).expect("valid original response cutoff");
        store
            .reserve(original.clone(), cutoff, reserve_deadline())
            .expect("reserve original exact receipt");
        let generation = store.generation().expect("generation after reserve");

        assert_eq!(
            ReceiptLedgerPort::recover(&mut store, &mismatch, reserve_deadline())
                .expect_err("partial task-id collision must not look absent"),
            ReceiptLedgerError::ReservedTaskIdentityMismatch
        );
        assert!(matches!(
            ReceiptLedgerPort::recover(&mut store, &original, reserve_deadline()),
            Ok(ReceiptState::Reserved(_))
        ));
        assert_eq!(
            store.generation().expect("unchanged generation"),
            generation
        );
    }

    #[test]
    fn fail_stopped_store_rejects_partial_identity_mismatch_as_unavailable() {
        let root = tempfile::tempdir().expect("temporary root");
        let receipts = fs::canonicalize(root.path())
            .expect("physical temporary root")
            .join("receipts");
        let mut store = ReceiptLedgerStore::open(&receipts).expect("open receipt ledger");
        let original = receipt_key(INVOCATION_A, TASK_A, "workspace-a");
        let mismatch = receipt_key(INVOCATION_A, TASK_B, "workspace-b");
        let original_digest = receipt_key_digest(&original);
        let cutoff =
            OriginalCutoffDescriptor::new(1_000, 7_000).expect("valid original response cutoff");
        store
            .reserve(original.clone(), cutoff, reserve_deadline())
            .expect("reserve original exact receipt");
        let row_path = receipts
            .join(ACTIVE_DIRECTORY_NAME)
            .join(format!("{}.json", original_digest.as_str()));
        fs::write(&row_path, vec![b' '; MAX_TASK_RECORD_ENVELOPE_BYTES + 1])
            .expect("replace row with corrupt persisted evidence");
        assert!(matches!(
            ReceiptLedgerPort::recover(&mut store, &original, reserve_deadline()),
            Err(ReceiptLedgerError::Corrupt(_))
        ));

        assert_eq!(
            ReceiptLedgerPort::recover(&mut store, &mismatch, reserve_deadline())
                .expect_err("latched store authority precedes clean mismatch classification"),
            ReceiptLedgerError::StoreUnavailable
        );
    }

    #[test]
    fn recover_rechecks_deadline_after_waiting_for_writer_before_classifying_mismatch() {
        let root = tempfile::tempdir().expect("temporary root");
        let receipts = fs::canonicalize(root.path())
            .expect("physical temporary root")
            .join("receipts");
        let store =
            std::sync::Arc::new(ReceiptLedgerStore::open(&receipts).expect("open receipt ledger"));
        let original = receipt_key(INVOCATION_A, TASK_A, "workspace-a");
        let mismatch = receipt_key(INVOCATION_A, TASK_B, "workspace-b");
        let cutoff =
            OriginalCutoffDescriptor::new(1_000, 7_000).expect("valid original response cutoff");
        store
            .reserve(original, cutoff, reserve_deadline())
            .expect("reserve original exact receipt");
        let writer = store
            .writer
            .lock()
            .expect("hold writer lock across recovery deadline");
        let blocked_store = std::sync::Arc::clone(&store);
        let deadline = Instant::now() + Duration::from_millis(40);
        let blocked = std::thread::spawn(move || blocked_store.recover_exact(&mismatch, deadline));
        std::thread::sleep(Duration::from_millis(80));
        drop(writer);

        assert_eq!(
            blocked
                .join()
                .expect("blocked recovery thread does not panic")
                .expect_err("expired stable-read fence precedes mismatch classification"),
            ReceiptLedgerError::DeadlineExceeded
        );
    }

    #[test]
    fn recover_port_rejects_an_expired_deadline_without_latching_the_store() {
        let root = tempfile::tempdir().expect("temporary root");
        let receipts = fs::canonicalize(root.path())
            .expect("physical temporary root")
            .join("receipts");
        let mut store = ReceiptLedgerStore::open(&receipts).expect("open receipt ledger");
        let key = receipt_key(INVOCATION_A, TASK_A, "workspace-a");

        let error = ReceiptLedgerPort::recover(&mut store, &key, Instant::now())
            .expect_err("expired recovery must not inspect storage");

        assert_eq!(error, ReceiptLedgerError::DeadlineExceeded);
        assert_eq!(
            ReceiptLedgerPort::recover(&mut store, &key, reserve_deadline())
                .expect_err("clean deadline rejection keeps the store reusable"),
            ReceiptLedgerError::ReceiptNotFound
        );
    }

    #[test]
    fn exact_recovery_rejects_a_digest_match_with_a_different_full_key() {
        let requested = receipt_key(INVOCATION_A, TASK_A, "workspace-a");
        let foreign = receipt_key(INVOCATION_B, TASK_B, "workspace-b");
        let cutoff =
            OriginalCutoffDescriptor::new(1_000, 7_000).expect("valid original response cutoff");
        let forged_collision = CatalogEntry {
            record: StoredActiveReceiptV1 {
                schema_version: RECEIPT_RECORD_SCHEMA_VERSION,
                mutation_sequence: 1,
                record_version: ReceiptVersion::initial(),
                key: foreign,
                key_digest: receipt_key_digest(&requested),
                lifecycle: StoredActiveLifecycleV1::ReservedUnbound {
                    reserved_at_epoch_ms: cutoff.accepted_epoch_ms(),
                    original_cutoff: cutoff,
                    cancel_requested: false,
                },
            },
            encoded_bytes: 512,
        }
        .reservation()
        .expect("forged fixture remains a reservation body");

        let error = exact_reserved_state(&requested, forged_collision)
            .expect_err("digest equality alone must not establish exact-key equality");

        assert_eq!(error, ReceiptLedgerError::ReceiptDigestCollision);
    }

    #[test]
    fn exact_duplicate_returns_original_cutoff_without_generation_or_file_mutation() {
        let root = tempfile::tempdir().expect("temporary root");
        let receipts = fs::canonicalize(root.path())
            .expect("physical temporary root")
            .join("receipts");
        let store = ReceiptLedgerStore::open(&receipts).expect("open receipt ledger");
        let key = receipt_key(INVOCATION_A, TASK_A, "workspace-a");
        let key_digest = crate::application::receipt_ledger::receipt_key_digest(&key);
        let original_cutoff =
            OriginalCutoffDescriptor::new(2_000, 7_000).expect("valid original response cutoff");
        let created = store
            .reserve(key.clone(), original_cutoff, reserve_deadline())
            .expect("create exact reservation")
            .into_reservation()
            .expect("receipt remains reserved");
        let row_path = receipts
            .join(ACTIVE_DIRECTORY_NAME)
            .join(format!("{}.json", key_digest.as_str()));
        let row_before = fs::read(&row_path).expect("read original receipt row");
        let names_before = directory_names(&receipts.join(ACTIVE_DIRECTORY_NAME));
        let generation_before = store.generation().expect("generation after reserve");

        let duplicate_cutoff =
            OriginalCutoffDescriptor::new(9_000, 1_000).expect("different current response cutoff");
        let duplicate = store
            .reserve(key, duplicate_cutoff, reserve_deadline())
            .expect("read exact duplicate reservation");

        assert!(matches!(duplicate, ReserveOutcome::ExistingExact(_)));
        assert_eq!(
            duplicate
                .into_reservation()
                .expect("duplicate reservation remains reserved"),
            created
        );
        assert_eq!(
            store.generation().expect("generation after duplicate"),
            generation_before
        );
        assert_eq!(fs::read(&row_path).expect("reread receipt row"), row_before);
        assert_eq!(
            directory_names(&receipts.join(ACTIVE_DIRECTORY_NAME)),
            names_before
        );
    }

    #[test]
    fn invocation_id_collision_rejects_before_any_mutation() {
        let root = tempfile::tempdir().expect("temporary root");
        let receipts = fs::canonicalize(root.path())
            .expect("physical temporary root")
            .join("receipts");
        let store = ReceiptLedgerStore::open(&receipts).expect("open receipt ledger");
        let cutoff =
            OriginalCutoffDescriptor::new(1_000, 7_000).expect("valid original response cutoff");
        store
            .reserve(
                receipt_key(INVOCATION_A, TASK_A, "workspace-a"),
                cutoff,
                reserve_deadline(),
            )
            .expect("create original reservation");
        let generation_before = store.generation().expect("generation after reserve");
        let names_before = directory_names(&receipts.join(ACTIVE_DIRECTORY_NAME));

        let error = store
            .reserve(
                receipt_key(INVOCATION_A, TASK_B, "workspace-b"),
                cutoff,
                reserve_deadline(),
            )
            .expect_err("one invocation id cannot identify two receipt keys");

        assert_eq!(error, ReceiptLedgerError::InvocationIdentityMismatch);
        assert_eq!(
            store.generation().expect("generation after collision"),
            generation_before
        );
        assert_eq!(
            directory_names(&receipts.join(ACTIVE_DIRECTORY_NAME)),
            names_before
        );
    }

    #[test]
    fn collision_rejection_requires_post_catalog_named_authority() {
        let root = tempfile::tempdir().expect("temporary root");
        let receipts = fs::canonicalize(root.path())
            .expect("physical temporary root")
            .join("receipts");
        let active_path = receipts.join(ACTIVE_DIRECTORY_NAME);
        let displaced_path = receipts.join("active-displaced-before-collision");
        let store = ReceiptLedgerStore::open(&receipts).expect("open receipt ledger");
        let cutoff =
            OriginalCutoffDescriptor::new(1_000, 7_000).expect("valid original response cutoff");
        store
            .reserve(
                receipt_key(INVOCATION_A, TASK_A, "workspace-a"),
                cutoff,
                reserve_deadline(),
            )
            .expect("create original reservation");
        let replacement = std::rc::Rc::new(Cell::new(None));
        let hook_replacement = std::rc::Rc::clone(&replacement);
        set_after_reserve_catalog_lock_hook_for_test(move || {
            hook_replacement.set(Some(
                attempt_retained_directory_replacement_for_test(&active_path, &displaced_path)
                    .expect("attempt named active displacement after catalog lock"),
            ));
        });

        let error = store
            .reserve(
                receipt_key(INVOCATION_A, TASK_B, "workspace-b"),
                cutoff,
                reserve_deadline(),
            )
            .expect_err("displaced owner cannot issue a catalog-derived collision verdict");

        match replacement.get().expect("replacement hook ran") {
            RetainedDirectoryReplacementOutcome::Replaced => assert!(matches!(
                error,
                ReceiptLedgerError::Storage {
                    operation: "validate named receipt active directory",
                    ..
                }
            )),
            RetainedDirectoryReplacementOutcome::PreventedByRetainedHandle => {
                assert_eq!(error, ReceiptLedgerError::InvocationIdentityMismatch)
            }
        }
    }

    #[test]
    fn reserved_task_id_collision_rejects_before_any_mutation() {
        let root = tempfile::tempdir().expect("temporary root");
        let receipts = fs::canonicalize(root.path())
            .expect("physical temporary root")
            .join("receipts");
        let store = ReceiptLedgerStore::open(&receipts).expect("open receipt ledger");
        let cutoff =
            OriginalCutoffDescriptor::new(1_000, 7_000).expect("valid original response cutoff");
        store
            .reserve(
                receipt_key(INVOCATION_A, TASK_A, "workspace-a"),
                cutoff,
                reserve_deadline(),
            )
            .expect("create original reservation");
        let generation_before = store.generation().expect("generation after reserve");
        let names_before = directory_names(&receipts.join(ACTIVE_DIRECTORY_NAME));

        let error = store
            .reserve(
                receipt_key(INVOCATION_B, TASK_A, "workspace-b"),
                cutoff,
                reserve_deadline(),
            )
            .expect_err("one reserved task id cannot identify two receipt keys");

        assert_eq!(error, ReceiptLedgerError::ReservedTaskIdentityMismatch);
        assert_eq!(
            store.generation().expect("generation after collision"),
            generation_before
        );
        assert_eq!(
            directory_names(&receipts.join(ACTIVE_DIRECTORY_NAME)),
            names_before
        );
    }

    #[test]
    fn same_id_pair_with_different_request_identity_rejects_before_any_mutation() {
        let root = tempfile::tempdir().expect("temporary root");
        let receipts = fs::canonicalize(root.path())
            .expect("physical temporary root")
            .join("receipts");
        let store = ReceiptLedgerStore::open(&receipts).expect("open receipt ledger");
        let cutoff =
            OriginalCutoffDescriptor::new(1_000, 7_000).expect("valid original response cutoff");
        store
            .reserve(
                receipt_key(INVOCATION_A, TASK_A, "workspace-a"),
                cutoff,
                reserve_deadline(),
            )
            .expect("create original reservation");
        let generation_before = store.generation().expect("generation after reserve");
        let names_before = directory_names(&receipts.join(ACTIVE_DIRECTORY_NAME));

        let error = store
            .reserve(
                receipt_key(INVOCATION_A, TASK_A, "workspace-b"),
                cutoff,
                reserve_deadline(),
            )
            .expect_err("the same id pair cannot change request identity");

        assert_eq!(error, ReceiptLedgerError::InvocationIdentityMismatch);
        assert_eq!(
            store.generation().expect("generation after collision"),
            generation_before
        );
        assert_eq!(
            directory_names(&receipts.join(ACTIVE_DIRECTORY_NAME)),
            names_before
        );
    }

    #[test]
    fn reserve_after_rename_sync_failure_is_commit_uncertain_and_store_fail_stops_until_reopen() {
        let root = tempfile::tempdir().expect("temporary root");
        let receipts = fs::canonicalize(root.path())
            .expect("physical temporary root")
            .join("receipts");
        let store = ReceiptLedgerStore::open(&receipts).expect("open receipt ledger");
        let key = receipt_key(INVOCATION_A, TASK_A, "workspace-a");
        let key_digest = crate::application::receipt_ledger::receipt_key_digest(&key);
        let cutoff =
            OriginalCutoffDescriptor::new(1_000, 7_000).expect("valid original response cutoff");
        inject_receipt_row_directory_sync_failure_for_test();

        let error = store
            .reserve(key.clone(), cutoff, reserve_deadline())
            .expect_err("post-rename sync failure cannot report a definite non-commit");

        assert_eq!(
            error,
            ReceiptLedgerError::CommitUncertain {
                receipt_key_digest: key_digest.clone(),
            }
        );
        let row_path = receipts
            .join(ACTIVE_DIRECTORY_NAME)
            .join(format!("{}.json", key_digest.as_str()));
        assert!(row_path.is_file(), "the uncertain row was already visible");
        let active = open_directory_nofollow(&receipts.join(ACTIVE_DIRECTORY_NAME))
            .expect("open retained active directory");
        let row = open_regular_child_nofollow(
            &active,
            OsStr::new(&format!("{}.json", key_digest.as_str())),
        )
        .expect("open uncertain receipt row");
        verify_owner_only_acl(&row).expect("uncertain row remains owner-only");

        let fail_stop = store
            .reserve(
                receipt_key(INVOCATION_B, TASK_B, "workspace-b"),
                cutoff,
                reserve_deadline(),
            )
            .expect_err("an uncertain owner must reject later mutations");
        assert_eq!(fail_stop, ReceiptLedgerError::StoreUnavailable);
        drop(store);

        let reopened = ReceiptLedgerStore::open(&receipts)
            .expect("reopen performs exact recovery after uncertain publication");
        assert_eq!(reopened.generation().expect("healed generation"), 1);
        let recovered = reopened
            .read_reserved(&key_digest)
            .expect("read recovered reservation")
            .expect("uncertain row is recovered as committed");
        assert_eq!(recovered.key(), &key);
        assert_eq!(recovered.original_cutoff(), &cutoff);
    }

    #[test]
    fn elapsed_deadline_after_row_rename_still_attempts_required_directory_sync() {
        let root = tempfile::tempdir().expect("temporary root");
        let receipts = fs::canonicalize(root.path())
            .expect("physical temporary root")
            .join("receipts");
        let store = ReceiptLedgerStore::open(&receipts).expect("open receipt ledger");
        let key = receipt_key(INVOCATION_A, TASK_A, "workspace-a");
        let key_digest = crate::application::receipt_ledger::receipt_key_digest(&key);
        let cutoff =
            OriginalCutoffDescriptor::new(1_000, 7_000).expect("valid original response cutoff");
        let deadline = Instant::now() + Duration::from_secs(2);
        set_after_receipt_row_rename_hook_for_test(move || {
            while Instant::now() < deadline {
                std::thread::yield_now();
            }
        });
        inject_receipt_row_directory_sync_failure_for_test();

        let error = store
            .reserve(key, cutoff, deadline)
            .expect_err("visible row after deadline is an uncertain commit");

        assert_eq!(
            error,
            ReceiptLedgerError::CommitUncertain {
                receipt_key_digest: key_digest,
            }
        );
        assert!(
            sync_receipt_row_directory(&store.active_file).is_ok(),
            "post-visibility durability sync must run even after the deadline expires"
        );
    }

    #[test]
    fn visible_create_updates_live_catalog_before_the_first_post_rename_hook() {
        let root = tempfile::tempdir().expect("temporary root");
        let receipts = fs::canonicalize(root.path())
            .expect("physical temporary root")
            .join("receipts");
        let store = ReceiptLedgerStore::open(&receipts).expect("open receipt ledger");
        let key = receipt_key(INVOCATION_A, TASK_A, "workspace-a");
        let key_digest = receipt_key_digest(&key);
        let cutoff =
            OriginalCutoffDescriptor::new(1_000, 7_000).expect("valid original response cutoff");
        set_after_receipt_row_rename_hook_for_test(|| {
            panic!("simulate process loss at the first post-rename instruction")
        });

        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let _ = store.reserve(key, cutoff, reserve_deadline());
            }))
            .is_err(),
            "fixture must interrupt publication immediately after visible rename"
        );
        let catalog = store
            .writer
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let visible = catalog
            .records
            .get(&key_digest)
            .expect("visible create must already own live accounting");
        assert!(matches!(
            visible.record.lifecycle,
            StoredActiveLifecycleV1::ReservedUnbound { .. }
        ));
        assert_eq!(
            catalog.actual_bytes + catalog.reserved_result_bytes,
            MAX_RECEIPT_ENTITLEMENT_BYTES
        );
    }

    #[test]
    fn visible_replace_updates_live_catalog_before_the_first_post_rename_hook() {
        let root = tempfile::tempdir().expect("temporary root");
        let receipts = fs::canonicalize(root.path())
            .expect("physical temporary root")
            .join("receipts");
        let store = ReceiptLedgerStore::open(&receipts).expect("open receipt ledger");
        let key = receipt_key(INVOCATION_A, TASK_A, "workspace-a");
        let key_digest = receipt_key_digest(&key);
        let cutoff =
            OriginalCutoffDescriptor::new(1_000, 7_000).expect("valid original response cutoff");
        let reserved = store
            .reserve(key.clone(), cutoff, reserve_deadline())
            .expect("reserve exact receipt")
            .into_reservation()
            .expect("created receipt remains reserved");
        set_after_receipt_row_rename_hook_for_test(|| {
            panic!("simulate process loss at the first post-replace instruction")
        });

        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let _ = store.publish_direct_terminal(
                    &key,
                    reserved.record_version(),
                    2_000,
                    canonical_v5_terminal(&ReceiptTerminalOutcome::Cancelled)
                        .expect("canonical direct terminal"),
                    reserve_deadline(),
                );
            }))
            .is_err(),
            "fixture must interrupt replacement immediately after visible rename"
        );
        let catalog = store
            .writer
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let visible = catalog
            .records
            .get(&key_digest)
            .expect("visible replacement must already own live accounting");
        assert!(matches!(
            visible.record.lifecycle,
            StoredActiveLifecycleV1::DirectTerminalUnacked { .. }
        ));
        assert_eq!(
            catalog.actual_bytes + catalog.reserved_result_bytes,
            MAX_RECEIPT_ENTITLEMENT_BYTES
        );
    }

    #[test]
    fn failed_prepublication_cleanup_fail_stops_the_store_until_reopen() {
        let root = tempfile::tempdir().expect("temporary root");
        let receipts = fs::canonicalize(root.path())
            .expect("physical temporary root")
            .join("receipts");
        let store = ReceiptLedgerStore::open(&receipts).expect("open receipt ledger");
        let active_path = receipts.join(ACTIVE_DIRECTORY_NAME);
        let hook_active_path = active_path.clone();
        let relocation = std::rc::Rc::new(Cell::new(None));
        let hook_relocation = std::rc::Rc::clone(&relocation);
        set_before_identity_bound_no_replace_rename_hook(move || {
            let staging_name = directory_names(&hook_active_path)
                .into_iter()
                .find(|name| name.starts_with(".receipt.") && name.ends_with(".tmp"))
                .expect("receipt staging exists before publication");
            hook_relocation.set(Some(
                attempt_retained_regular_file_relocation_for_test(
                    &hook_active_path.join(staging_name),
                    &hook_active_path.join(".unica-cleanup-cccccccc-cccc-4ccc-8ccc-cccccccccccc"),
                )
                .expect("attempt staging displacement before publication and cleanup"),
            ));
        });
        let cutoff =
            OriginalCutoffDescriptor::new(1_000, 7_000).expect("valid original response cutoff");

        let result = store.reserve(
            receipt_key(INVOCATION_A, TASK_A, "workspace-a"),
            cutoff,
            reserve_deadline(),
        );

        match relocation.get().expect("relocation hook ran") {
            RetainedRegularFileRelocationOutcome::PreventedByRetainedHandle => {
                assert!(
                    matches!(result, Ok(ReserveOutcome::Created(_))),
                    "a platform-prevented displacement leaves the ordinary publication valid"
                );
                return;
            }
            RetainedRegularFileRelocationOutcome::Relocated => {}
        }
        let error =
            result.expect_err("failed exact staging cleanup cannot be reported as a clean abort");

        assert_eq!(error, ReceiptLedgerError::StoreUnavailable);
        let fail_stop = store
            .reserve(
                receipt_key(INVOCATION_B, TASK_B, "workspace-b"),
                cutoff,
                reserve_deadline(),
            )
            .expect_err("failed cleanup requires process-owned reopen recovery");
        assert_eq!(fail_stop, ReceiptLedgerError::StoreUnavailable);
        drop(store);

        let reopened = ReceiptLedgerStore::open(&receipts)
            .expect("reopen removes the displaced staging quarantine");
        assert_eq!(reopened.generation().expect("stable generation"), 0);
        assert!(
            directory_names(&active_path).is_empty(),
            "reopen removes the failed staging publication without inventing a receipt"
        );
    }

    #[test]
    fn generation_reader_never_observes_the_replace_before_capability_swap_window() {
        let root = tempfile::tempdir().expect("temporary root");
        let receipts = fs::canonicalize(root.path())
            .expect("physical temporary root")
            .join("receipts");
        let store =
            std::sync::Arc::new(ReceiptLedgerStore::open(&receipts).expect("open receipt ledger"));
        let reader_store = std::sync::Arc::clone(&store);
        let (start_reader_tx, start_reader_rx) = std::sync::mpsc::channel();
        let reader = std::thread::spawn(move || {
            start_reader_rx
                .recv_timeout(Duration::from_secs(2))
                .expect("generation replacement hook starts reader");
            reader_store.generation()
        });
        set_after_generation_replace_hook_for_test(move || {
            start_reader_tx
                .send(())
                .expect("generation reader is waiting");
            std::thread::sleep(Duration::from_millis(100));
        });
        let cutoff =
            OriginalCutoffDescriptor::new(1_000, 7_000).expect("valid original response cutoff");

        store
            .reserve(
                receipt_key(INVOCATION_A, TASK_A, "workspace-a"),
                cutoff,
                reserve_deadline(),
            )
            .expect("reserve across generation replacement");

        assert_eq!(
            reader
                .join()
                .expect("generation reader thread does not panic")
                .expect("generation reader never sees displaced capability"),
            1
        );
    }

    #[test]
    fn concurrent_reserve_waits_for_generation_capability_swap_under_the_writer_lock() {
        let root = tempfile::tempdir().expect("temporary root");
        let receipts = fs::canonicalize(root.path())
            .expect("physical temporary root")
            .join("receipts");
        let store =
            std::sync::Arc::new(ReceiptLedgerStore::open(&receipts).expect("open receipt ledger"));
        let second_store = std::sync::Arc::clone(&store);
        let cutoff =
            OriginalCutoffDescriptor::new(1_000, 7_000).expect("valid original response cutoff");
        let (start_second_tx, start_second_rx) = std::sync::mpsc::channel();
        let second = std::thread::spawn(move || {
            start_second_rx
                .recv_timeout(Duration::from_secs(2))
                .expect("generation replacement hook starts second reserve");
            second_store.reserve(
                receipt_key(INVOCATION_B, TASK_B, "workspace-b"),
                cutoff,
                reserve_deadline(),
            )
        });
        set_after_generation_replace_hook_for_test(move || {
            start_second_tx.send(()).expect("second reserve is waiting");
            std::thread::sleep(Duration::from_millis(100));
        });

        store
            .reserve(
                receipt_key(INVOCATION_A, TASK_A, "workspace-a"),
                cutoff,
                reserve_deadline(),
            )
            .expect("first reserve crosses generation replacement");

        assert!(matches!(
            second.join().expect("second reserve does not panic"),
            Ok(ReserveOutcome::Created(_))
        ));
        assert_eq!(store.generation().expect("both reserves committed"), 2);
    }

    #[test]
    fn persisted_dual_index_collision_fails_reopen_before_temporary_cleanup_or_mutation() {
        let root = tempfile::tempdir().expect("temporary root");
        let receipts = fs::canonicalize(root.path())
            .expect("physical temporary root")
            .join("receipts");
        let cutoff =
            OriginalCutoffDescriptor::new(1_000, 7_000).expect("valid original response cutoff");
        {
            let store = ReceiptLedgerStore::open(&receipts).expect("open receipt ledger");
            store
                .reserve(
                    receipt_key(INVOCATION_A, TASK_A, "workspace-a"),
                    cutoff,
                    reserve_deadline(),
                )
                .expect("create original reservation");
        }
        let colliding_digest = write_reserved_row_fixture(
            &receipts,
            receipt_key(INVOCATION_A, TASK_B, "workspace-b"),
            cutoff,
            2,
        );
        let active = open_directory_nofollow(&receipts.join(ACTIVE_DIRECTORY_NAME))
            .expect("open active fixture");
        let temporary_name = ".receipt.55555555-5555-4555-8555-555555555555.tmp";
        let mut temporary = create_owner_only_file_child(&active, OsStr::new(temporary_name))
            .expect("create abandoned owner-only staging fixture");
        temporary
            .write_all(b"staged-but-not-published")
            .and_then(|()| temporary.sync_all())
            .expect("persist abandoned staging fixture");
        sync_directory(&active).expect("sync abandoned staging fixture");
        drop(temporary);
        drop(active);
        let temporary_path = receipts.join(ACTIVE_DIRECTORY_NAME).join(temporary_name);
        let temporary_before = fs::read(&temporary_path).expect("read abandoned staging bytes");
        let generation_before = fs::read(receipts.join(GENERATION_FILE_NAME))
            .expect("read generation before corrupt reopen");
        let collision_path = receipts
            .join(ACTIVE_DIRECTORY_NAME)
            .join(format!("{}.json", colliding_digest.as_str()));
        let collision_before = fs::read(&collision_path).expect("read colliding row bytes");

        let error = ReceiptLedgerStore::open(&receipts)
            .err()
            .expect("persisted invocation collision must reject reopen");

        assert!(matches!(error, ReceiptLedgerError::Corrupt(_)));
        assert_eq!(
            fs::read(&temporary_path).expect("failed reopen leaves staging bytes untouched"),
            temporary_before
        );
        assert_eq!(
            fs::read(receipts.join(GENERATION_FILE_NAME))
                .expect("failed reopen leaves generation untouched"),
            generation_before
        );
        assert_eq!(
            fs::read(&collision_path).expect("failed reopen leaves colliding row untouched"),
            collision_before
        );
    }

    #[test]
    fn persisted_collision_without_generation_fails_before_initialization_mutation() {
        let root = tempfile::tempdir().expect("temporary root");
        let root_path = fs::canonicalize(root.path()).expect("physical temporary root");
        let receipts = root_path.join("receipts");
        let root_file = open_directory_nofollow(&root_path).expect("open physical root");
        let receipts_file = create_owner_only_directory_child(&root_file, OsStr::new("receipts"))
            .expect("create owner-only receipts fixture");
        let active_file =
            create_owner_only_directory_child(&receipts_file, OsStr::new(ACTIVE_DIRECTORY_NAME))
                .expect("create owner-only active fixture");
        sync_directory(&receipts_file).expect("sync active fixture");
        sync_directory(&root_file).expect("sync receipts fixture");
        drop(active_file);
        drop(receipts_file);
        drop(root_file);
        let cutoff =
            OriginalCutoffDescriptor::new(1_000, 7_000).expect("valid original response cutoff");
        let first_digest = write_reserved_row_fixture(
            &receipts,
            receipt_key(INVOCATION_A, TASK_A, "workspace-a"),
            cutoff,
            1,
        );
        let second_digest = write_reserved_row_fixture(
            &receipts,
            receipt_key(INVOCATION_A, TASK_B, "workspace-b"),
            cutoff,
            2,
        );
        let first_path = receipts
            .join(ACTIVE_DIRECTORY_NAME)
            .join(format!("{}.json", first_digest.as_str()));
        let second_path = receipts
            .join(ACTIVE_DIRECTORY_NAME)
            .join(format!("{}.json", second_digest.as_str()));
        let first_before = fs::read(&first_path).expect("read first persisted collision row");
        let second_before = fs::read(&second_path).expect("read second persisted collision row");
        assert!(
            !receipts.join(GENERATION_FILE_NAME).exists(),
            "fixture intentionally has no generation"
        );

        let error = ReceiptLedgerStore::open(&receipts)
            .err()
            .expect("persisted invocation collision must reject open");

        assert_eq!(
            error,
            ReceiptLedgerError::Corrupt("receipt catalog contains a duplicate invocation id")
        );
        assert!(
            !receipts.join(GENERATION_FILE_NAME).exists(),
            "corrupt persisted evidence must be rejected before generation initialization"
        );
        assert_eq!(
            fs::read(first_path).expect("failed open leaves first row untouched"),
            first_before
        );
        assert_eq!(
            fs::read(second_path).expect("failed open leaves second row untouched"),
            second_before
        );
    }

    #[test]
    fn nonzero_generation_without_active_fails_before_recreating_empty_namespace() {
        let root = tempfile::tempdir().expect("temporary root");
        let root_path = fs::canonicalize(root.path()).expect("physical temporary root");
        let receipts = root_path.join("receipts");
        let root_file = open_directory_nofollow(&root_path).expect("open physical root");
        let receipts_file = create_owner_only_directory_child(&root_file, OsStr::new("receipts"))
            .expect("create owner-only receipts fixture");
        let mut generation =
            create_owner_only_file_child(&receipts_file, OsStr::new(GENERATION_FILE_NAME))
                .expect("create generation evidence fixture");
        generation
            .write_all(b"1\n")
            .and_then(|()| generation.sync_all())
            .expect("persist generation evidence fixture");
        sync_directory(&receipts_file).expect("sync generation evidence fixture");
        sync_directory(&root_file).expect("sync receipts fixture");
        drop(generation);
        drop(receipts_file);
        drop(root_file);

        let error = ReceiptLedgerStore::open(&receipts)
            .err()
            .expect("missing active evidence at nonzero generation must fail closed");

        assert_eq!(
            error,
            ReceiptLedgerError::Corrupt(
                "nonzero receipt generation is missing its active directory"
            )
        );
        assert!(
            !receipts.join(ACTIVE_DIRECTORY_NAME).exists(),
            "evidence loss must not be hidden by recreating an empty active directory"
        );
        assert_eq!(
            fs::read(receipts.join(GENERATION_FILE_NAME))
                .expect("failed open leaves generation evidence untouched"),
            b"1\n"
        );
    }

    #[test]
    fn reserved_record_uses_strict_camel_case_lifecycle_fields() {
        let root = tempfile::tempdir().expect("temporary root");
        let receipts = fs::canonicalize(root.path())
            .expect("physical temporary root")
            .join("receipts");
        let store = ReceiptLedgerStore::open(&receipts).expect("open receipt ledger");
        let key = receipt_key(INVOCATION_A, TASK_A, "workspace-a");
        let key_digest = crate::application::receipt_ledger::receipt_key_digest(&key);
        let cutoff =
            OriginalCutoffDescriptor::new(1_000, 7_000).expect("valid original response cutoff");
        store
            .reserve(key, cutoff, reserve_deadline())
            .expect("create reserved row");
        let row_path = receipts
            .join(ACTIVE_DIRECTORY_NAME)
            .join(format!("{}.json", key_digest.as_str()));
        let row = fs::read_to_string(&row_path).expect("read reserved row as UTF-8");

        assert!(row.contains("\"cancelRequested\":false"));
        assert!(row.contains("\"recordVersion\":1"));
        assert!(row.contains("\"reservedAtEpochMs\":1000"));
        assert!(!row.contains("cancel_requested"));
        let active = open_directory_nofollow(&receipts.join(ACTIVE_DIRECTORY_NAME))
            .expect("open active directory");
        let persisted = open_regular_child_nofollow(
            &active,
            OsStr::new(&format!("{}.json", key_digest.as_str())),
        )
        .expect("open reserved row no-follow");
        verify_owner_only_acl(&persisted).expect("reserved row is owner-only");

        let unknown_field = row.replacen(
            "\"cancelRequested\":false",
            "\"cancelRequested\":false,\"unexpected\":true",
            1,
        );
        assert!(
            serde_json::from_str::<StoredActiveReceiptV1>(&unknown_field).is_err(),
            "selected lifecycle variants must reject unknown fields"
        );
        assert!(
            serde_json::from_str::<StoredActiveReceiptV1>(&row.replacen(
                "\"recordVersion\":1,",
                "",
                1,
            ))
            .is_err(),
            "every persisted record must carry an explicit CAS version"
        );
        assert!(
            serde_json::from_str::<StoredActiveReceiptV1>(&row.replacen(
                "\"reservedAtEpochMs\":1000,",
                "",
                1,
            ))
            .is_err(),
            "every persisted reservation must carry its explicit reserve epoch"
        );
        assert!(
            serde_json::from_str::<StoredActiveReceiptV1>(&row.replacen(
                "\"recordVersion\":1",
                "\"recordVersion\":0",
                1,
            ))
            .is_err(),
            "persisted record versions must be nonzero"
        );
    }

    #[test]
    fn reopen_rejects_reserved_epoch_that_disagrees_with_the_accepted_epoch() {
        let root = tempfile::tempdir().expect("temporary root");
        let receipts = fs::canonicalize(root.path())
            .expect("physical temporary root")
            .join("receipts");
        let key = receipt_key(INVOCATION_A, TASK_A, "workspace-a");
        let key_digest = receipt_key_digest(&key);
        let cutoff =
            OriginalCutoffDescriptor::new(1_000, 7_000).expect("valid original response cutoff");
        {
            let store = ReceiptLedgerStore::open(&receipts).expect("open receipt ledger");
            store
                .reserve(key, cutoff, reserve_deadline())
                .expect("create reserved row");
        }
        let row_path = receipts
            .join(ACTIVE_DIRECTORY_NAME)
            .join(format!("{}.json", key_digest.as_str()));
        let mut record: StoredActiveReceiptV1 =
            serde_json::from_slice(&fs::read(&row_path).expect("read canonical receipt row"))
                .expect("decode strict receipt fixture");
        match &mut record.lifecycle {
            StoredActiveLifecycleV1::CancelReserved { .. } => {
                panic!("reserved fixture decoded as a cancellation reservation")
            }
            StoredActiveLifecycleV1::ExpiredDeletion { .. } => {
                panic!("reserved fixture decoded as an expiry deletion witness")
            }
            StoredActiveLifecycleV1::ReservedUnbound {
                reserved_at_epoch_ms,
                ..
            } => *reserved_at_epoch_ms = 1_001,
            StoredActiveLifecycleV1::DirectTerminalUnacked { .. } => {
                panic!("reserved fixture decoded as a direct terminal")
            }
        }
        let contradictory = serde_json::to_vec(&record).expect("encode contradictory row");
        fs::write(&row_path, contradictory).expect("persist contradictory reserve epoch");

        let error = ReceiptLedgerStore::open(&receipts)
            .err()
            .expect("contradictory reserve epoch must fail reopen");

        assert_eq!(
            error,
            ReceiptLedgerError::Corrupt(
                "receipt reserve epoch does not match its accepted request epoch"
            )
        );
    }

    #[test]
    fn oversized_live_persisted_row_is_corruption_and_fail_stops_the_store() {
        let root = tempfile::tempdir().expect("temporary root");
        let receipts = fs::canonicalize(root.path())
            .expect("physical temporary root")
            .join("receipts");
        let mut store = ReceiptLedgerStore::open(&receipts).expect("open receipt ledger");
        let key = receipt_key(INVOCATION_A, TASK_A, "workspace-a");
        let key_digest = receipt_key_digest(&key);
        let cutoff =
            OriginalCutoffDescriptor::new(1_000, 7_000).expect("valid original response cutoff");
        store
            .reserve(key.clone(), cutoff, reserve_deadline())
            .expect("create reserved row");
        let row_path = receipts
            .join(ACTIVE_DIRECTORY_NAME)
            .join(format!("{}.json", key_digest.as_str()));
        let mut oversized = fs::read(&row_path).expect("read canonical reserved row");
        oversized.resize(MAX_TASK_RECORD_ENVELOPE_BYTES + 1, b' ');
        assert!(
            serde_json::from_slice::<StoredActiveReceiptV1>(&oversized).is_ok(),
            "oversized fixture must remain strict Reserved JSON"
        );
        fs::write(&row_path, oversized).expect("replace row with oversized persisted evidence");

        let error = ReceiptLedgerPort::recover(&mut store, &key, reserve_deadline())
            .expect_err("persisted oversize is corruption, not prospective input rejection");

        assert_eq!(
            error,
            ReceiptLedgerError::Corrupt("persisted receipt row exceeds its byte limit")
        );
        assert!(error.requires_reopen());
        assert_eq!(
            ReceiptLedgerPort::recover(&mut store, &key, reserve_deadline())
                .expect_err("corrupt read latches the store"),
            ReceiptLedgerError::StoreUnavailable
        );
    }

    #[test]
    fn reopen_rejects_semantically_equivalent_but_noncanonical_receipt_json() {
        let root = tempfile::tempdir().expect("temporary root");
        let receipts = fs::canonicalize(root.path())
            .expect("physical temporary root")
            .join("receipts");
        let key = receipt_key(INVOCATION_A, TASK_A, "workspace-a");
        let key_digest = crate::application::receipt_ledger::receipt_key_digest(&key);
        let cutoff =
            OriginalCutoffDescriptor::new(1_000, 7_000).expect("valid original response cutoff");
        {
            let store = ReceiptLedgerStore::open(&receipts).expect("open receipt ledger");
            store
                .reserve(key, cutoff, reserve_deadline())
                .expect("create reserved row");
        }
        let row_path = receipts
            .join(ACTIVE_DIRECTORY_NAME)
            .join(format!("{}.json", key_digest.as_str()));
        let canonical = fs::read(&row_path).expect("read canonical receipt row");
        let canonical_text =
            String::from_utf8(canonical.clone()).expect("canonical receipt row is UTF-8");
        let reordered = canonical_text
            .replacen(
                "{\"schemaVersion\":1,\"mutationSequence\":1,",
                "{\"mutationSequence\":1,\"schemaVersion\":1,",
                1,
            )
            .into_bytes();
        assert_eq!(
            reordered.len(),
            canonical.len(),
            "the mutation must preserve accounting length"
        );
        assert_ne!(reordered, canonical, "the mutation must change byte order");
        fs::write(&row_path, &reordered).expect("persist noncanonical equivalent JSON");

        let error = ReceiptLedgerStore::open(&receipts)
            .err()
            .expect("reopen must reject noncanonical persisted bytes");

        assert_eq!(
            error,
            ReceiptLedgerError::Corrupt("receipt row is not canonical schema-v1 JSON")
        );
    }

    #[test]
    fn reopen_boundedly_removes_abandoned_generation_staging_after_validation() {
        let root = tempfile::tempdir().expect("temporary root");
        let receipts = fs::canonicalize(root.path())
            .expect("physical temporary root")
            .join("receipts");
        {
            let store = ReceiptLedgerStore::open(&receipts).expect("initialize receipt ledger");
            assert_eq!(store.generation().expect("initial generation"), 0);
        }
        let receipts_file = open_directory_nofollow(&receipts).expect("open receipts fixture");
        let temporary_name = ".generation.66666666-6666-4666-8666-666666666666.tmp";
        let mut temporary =
            create_owner_only_file_child(&receipts_file, OsStr::new(temporary_name))
                .expect("create abandoned generation staging fixture");
        temporary
            .write_all(b"1\n")
            .and_then(|()| temporary.sync_all())
            .expect("persist abandoned generation staging fixture");
        sync_directory(&receipts_file).expect("sync generation staging fixture");
        drop(temporary);
        drop(receipts_file);
        let temporary_path = receipts.join(temporary_name);

        let reopened = ReceiptLedgerStore::open(&receipts)
            .expect("reopen validates and cleans abandoned generation staging");

        assert_eq!(reopened.generation().expect("stable generation"), 0);
        assert!(
            !temporary_path.exists(),
            "validated abandoned generation staging was not removed"
        );
    }

    #[test]
    fn reopen_cleans_identity_bound_quarantine_left_by_interrupted_staging_cleanup() {
        let root = tempfile::tempdir().expect("temporary root");
        let receipts = fs::canonicalize(root.path())
            .expect("physical temporary root")
            .join("receipts");
        {
            let store = ReceiptLedgerStore::open(&receipts).expect("initialize receipt ledger");
            assert_eq!(store.generation().expect("initial generation"), 0);
        }
        let receipts_file = open_directory_nofollow(&receipts).expect("open receipts fixture");
        let active = crate::infrastructure::platform::filesystem::open_directory_child_nofollow(
            &receipts_file,
            OsStr::new(ACTIVE_DIRECTORY_NAME),
        )
        .expect("open active fixture");
        let root_quarantine_name = ".unica-cleanup-77777777-7777-4777-8777-777777777777";
        let active_quarantine_name = ".unica-cleanup-88888888-8888-4888-8888-888888888888";
        let mut root_quarantine =
            create_owner_only_file_child(&receipts_file, OsStr::new(root_quarantine_name))
                .expect("create root cleanup quarantine fixture");
        root_quarantine
            .write_all(b"generation-stage")
            .and_then(|()| root_quarantine.sync_all())
            .expect("persist root cleanup quarantine fixture");
        let mut active_quarantine =
            create_owner_only_file_child(&active, OsStr::new(active_quarantine_name))
                .expect("create active cleanup quarantine fixture");
        active_quarantine
            .write_all(b"receipt-stage")
            .and_then(|()| active_quarantine.sync_all())
            .expect("persist active cleanup quarantine fixture");
        sync_directory(&active).expect("sync active cleanup quarantine fixture");
        sync_directory(&receipts_file).expect("sync root cleanup quarantine fixture");
        drop(active_quarantine);
        drop(root_quarantine);
        drop(active);
        drop(receipts_file);
        let root_quarantine_path = receipts.join(root_quarantine_name);
        let active_quarantine_path = receipts
            .join(ACTIVE_DIRECTORY_NAME)
            .join(active_quarantine_name);

        let reopened = ReceiptLedgerStore::open(&receipts)
            .expect("reopen cleans its own interrupted cleanup quarantines");

        assert_eq!(reopened.generation().expect("stable generation"), 0);
        assert!(!root_quarantine_path.exists());
        assert!(!active_quarantine_path.exists());
    }

    #[test]
    fn expired_recovery_deadline_fails_before_staging_cleanup_or_catalog_mutation() {
        let root = tempfile::tempdir().expect("temporary root");
        let receipts = fs::canonicalize(root.path())
            .expect("physical temporary root")
            .join("receipts");
        {
            let store = ReceiptLedgerStore::open(&receipts).expect("initialize receipt ledger");
            assert_eq!(store.generation().expect("initial generation"), 0);
        }
        let receipts_file = open_directory_nofollow(&receipts).expect("open receipts fixture");
        let active = crate::infrastructure::platform::filesystem::open_directory_child_nofollow(
            &receipts_file,
            OsStr::new(ACTIVE_DIRECTORY_NAME),
        )
        .expect("open active fixture");
        let root_temporary_name = ".generation.99999999-9999-4999-8999-999999999999.tmp";
        let active_temporary_name = ".receipt.aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa.tmp";
        let mut root_temporary =
            create_owner_only_file_child(&receipts_file, OsStr::new(root_temporary_name))
                .expect("create root staging fixture");
        root_temporary
            .write_all(b"1\n")
            .and_then(|()| root_temporary.sync_all())
            .expect("persist root staging fixture");
        let mut active_temporary =
            create_owner_only_file_child(&active, OsStr::new(active_temporary_name))
                .expect("create active staging fixture");
        active_temporary
            .write_all(b"staged")
            .and_then(|()| active_temporary.sync_all())
            .expect("persist active staging fixture");
        sync_directory(&active).expect("sync active staging fixture");
        sync_directory(&receipts_file).expect("sync root staging fixture");
        drop(active_temporary);
        drop(root_temporary);
        drop(active);
        drop(receipts_file);
        let root_temporary_path = receipts.join(root_temporary_name);
        let active_temporary_path = receipts
            .join(ACTIVE_DIRECTORY_NAME)
            .join(active_temporary_name);
        let root_before = fs::read(&root_temporary_path).expect("read root staging fixture");
        let active_before = fs::read(&active_temporary_path).expect("read active staging fixture");

        let error = ReceiptLedgerStore::open_before(&receipts, Instant::now())
            .err()
            .expect("expired recovery budget must reject reopen");

        assert_eq!(error, ReceiptLedgerError::DeadlineExceeded);
        assert_eq!(
            fs::read(&root_temporary_path).expect("expired recovery leaves root staging"),
            root_before
        );
        assert_eq!(
            fs::read(&active_temporary_path).expect("expired recovery leaves active staging"),
            active_before
        );
        assert_eq!(
            fs::read(receipts.join(GENERATION_FILE_NAME))
                .expect("expired recovery leaves generation"),
            b"0\n"
        );
    }

    #[test]
    fn recovery_deadline_is_rechecked_after_generation_staging_cleanup() {
        let root = tempfile::tempdir().expect("temporary root");
        let receipts = fs::canonicalize(root.path())
            .expect("physical temporary root")
            .join("receipts");
        {
            let store = ReceiptLedgerStore::open(&receipts).expect("initialize receipt ledger");
            assert_eq!(store.generation().expect("initial generation"), 0);
        }
        let receipts_file = open_directory_nofollow(&receipts).expect("open receipts fixture");
        let temporary_name = ".generation.bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb.tmp";
        let mut temporary =
            create_owner_only_file_child(&receipts_file, OsStr::new(temporary_name))
                .expect("create generation staging fixture");
        temporary
            .write_all(b"staged")
            .and_then(|()| temporary.sync_all())
            .expect("persist generation staging fixture");
        sync_directory(&receipts_file).expect("sync generation staging fixture");
        drop(temporary);
        drop(receipts_file);
        let deadline = Instant::now() + Duration::from_secs(2);
        reset_recovery_cleanup_syncs_for_test();
        set_before_identity_bound_cleanup_mutation_hook(move || {
            while Instant::now() < deadline {
                std::thread::yield_now();
            }
        });

        let error = ReceiptLedgerStore::open_before(&receipts, deadline)
            .err()
            .expect("elapsed cleanup deadline must reject reopen");

        assert_eq!(error, ReceiptLedgerError::DeadlineExceeded);
        assert_eq!(
            recovery_cleanup_syncs_for_test(),
            1,
            "a visible staging removal must be directory-synced before deadline failure"
        );
        let reopened = ReceiptLedgerStore::open(&receipts)
            .expect("a later bounded reopen sees the completely recovered ledger");
        assert_eq!(reopened.generation().expect("stable generation"), 0);
    }

    #[test]
    fn recovery_deadline_is_rechecked_after_generation_healing_publication() {
        let root = tempfile::tempdir().expect("temporary root");
        let receipts = fs::canonicalize(root.path())
            .expect("physical temporary root")
            .join("receipts");
        {
            let store = ReceiptLedgerStore::open(&receipts).expect("initialize receipt ledger");
            assert_eq!(store.generation().expect("initial generation"), 0);
        }
        let key = receipt_key(INVOCATION_A, TASK_A, "workspace-a");
        let cutoff =
            OriginalCutoffDescriptor::new(1_000, 7_000).expect("valid original response cutoff");
        write_reserved_row_fixture(&receipts, key, cutoff, 1);
        let deadline = Instant::now() + Duration::from_secs(2);
        set_after_generation_replace_hook_for_test(move || {
            while Instant::now() < deadline {
                std::thread::yield_now();
            }
        });

        let error = ReceiptLedgerStore::open_before(&receipts, deadline)
            .err()
            .expect("elapsed healing deadline must reject reopen");

        assert_eq!(error, ReceiptLedgerError::DeadlineExceeded);
        let reopened = ReceiptLedgerStore::open(&receipts)
            .expect("a later bounded reopen adopts the visible generation heal");
        assert_eq!(reopened.generation().expect("healed generation"), 1);
    }

    #[test]
    fn sixty_four_exact_entitlements_reopen_and_sixty_fifth_rejects_without_mutation() {
        let root = tempfile::tempdir().expect("temporary root");
        let receipts = fs::canonicalize(root.path())
            .expect("physical temporary root")
            .join("receipts");
        let store = ReceiptLedgerStore::open(&receipts).expect("open receipt ledger");
        let cutoff =
            OriginalCutoffDescriptor::new(1_000, 7_000).expect("valid original response cutoff");
        let mut expected = Vec::new();
        let mut actual_bytes = 0_u64;
        let mut reserved_bytes = 0_u64;
        for index in 0..MAX_LIVE_RECEIPTS {
            let key = receipt_key_with_ids(
                InvocationId::new(),
                TaskId::new(),
                &format!("workspace-{index}"),
            );
            let digest = crate::application::receipt_ledger::receipt_key_digest(&key);
            let reservation = store
                .reserve(key.clone(), cutoff, reserve_deadline())
                .expect("reserve one exact entitlement")
                .into_reservation()
                .expect("receipt remains reserved");
            assert_eq!(
                reservation.encoded_bytes() + reservation.reserved_result_bytes(),
                MAX_RECEIPT_ENTITLEMENT_BYTES
            );
            actual_bytes += reservation.encoded_bytes();
            reserved_bytes += reservation.reserved_result_bytes();
            expected.push((digest, key, reservation));
        }
        assert_eq!(actual_bytes + reserved_bytes, MAX_LIVE_RECEIPT_BYTES);
        assert_eq!(
            store.generation().expect("generation at exact capacity"),
            MAX_LIVE_RECEIPTS as u64
        );
        let names_before = directory_names(&receipts.join(ACTIVE_DIRECTORY_NAME));

        let overflow = store
            .reserve(
                receipt_key_with_ids(InvocationId::new(), TaskId::new(), "workspace-overflow"),
                cutoff,
                reserve_deadline(),
            )
            .expect_err("the sixty-fifth live receipt must be rejected");

        assert_eq!(overflow, ReceiptLedgerError::CapacityExceeded);
        assert_eq!(
            store.generation().expect("capacity rejection is immutable"),
            MAX_LIVE_RECEIPTS as u64
        );
        assert_eq!(
            directory_names(&receipts.join(ACTIVE_DIRECTORY_NAME)),
            names_before
        );
        drop(store);

        let reopened = ReceiptLedgerStore::open(&receipts)
            .expect("reopen exact full receipt entitlement pool");
        assert_eq!(
            reopened.generation().expect("reopened full generation"),
            MAX_LIVE_RECEIPTS as u64
        );
        for (digest, key, reservation) in expected {
            let recovered = reopened
                .read_reserved(&digest)
                .expect("read one reopened reservation")
                .expect("full pool retains every reservation");
            assert_eq!(recovered, reservation);
            assert_eq!(recovered.key(), &key);
        }
    }

    #[test]
    fn live_read_rejects_a_row_that_was_not_admitted_into_the_recovered_catalog() {
        let root = tempfile::tempdir().expect("temporary root");
        let receipts = fs::canonicalize(root.path())
            .expect("physical temporary root")
            .join("receipts");
        let store = ReceiptLedgerStore::open(&receipts).expect("open receipt ledger");
        let key = receipt_key(INVOCATION_A, TASK_A, "workspace-a");
        let key_digest = crate::application::receipt_ledger::receipt_key_digest(&key);
        let cutoff =
            OriginalCutoffDescriptor::new(1_000, 7_000).expect("valid original response cutoff");
        let record = build_reserved_record(
            key,
            key_digest.clone(),
            cutoff,
            1,
            ReceiptVersion::initial(),
            false,
        );
        let (_, encoded) = serialize_reserved_record(record, MAX_TASK_RECORD_ENVELOPE_BYTES as u64)
            .expect("serialize foreign live row fixture");
        let name = format!("{}.json", key_digest.as_str());
        let mut row = create_owner_only_file_child(&store.active_file, OsStr::new(&name))
            .expect("create foreign owner-only live row fixture");
        row.write_all(&encoded)
            .and_then(|()| row.sync_all())
            .expect("persist foreign live row fixture");
        sync_directory(&store.active_file).expect("sync foreign live row fixture");
        let generation_before =
            fs::read(receipts.join(GENERATION_FILE_NAME)).expect("read generation before failure");
        let names_before = directory_names(&receipts.join(ACTIVE_DIRECTORY_NAME));

        let error = store
            .read_reserved(&key_digest)
            .expect_err("live disk state cannot bypass the recovered catalog");

        assert_eq!(
            error,
            ReceiptLedgerError::Corrupt("receipt row is present outside the recovered catalog")
        );
        let fail_stop = store
            .reserve(
                receipt_key(INVOCATION_B, TASK_B, "workspace-b"),
                cutoff,
                reserve_deadline(),
            )
            .expect_err("observed foreign row must latch the writer unavailable");
        assert_eq!(fail_stop, ReceiptLedgerError::StoreUnavailable);
        assert_eq!(
            fs::read(receipts.join(GENERATION_FILE_NAME))
                .expect("fail-stop leaves generation untouched"),
            generation_before
        );
        assert_eq!(
            directory_names(&receipts.join(ACTIVE_DIRECTORY_NAME)),
            names_before
        );
    }

    #[test]
    fn direct_reserve_collision_with_a_foreign_row_fail_stops_the_writer() {
        let root = tempfile::tempdir().expect("temporary root");
        let receipts = fs::canonicalize(root.path())
            .expect("physical temporary root")
            .join("receipts");
        let store = ReceiptLedgerStore::open(&receipts).expect("open receipt ledger");
        let key = receipt_key(INVOCATION_A, TASK_A, "workspace-a");
        let key_digest = crate::application::receipt_ledger::receipt_key_digest(&key);
        let cutoff =
            OriginalCutoffDescriptor::new(1_000, 7_000).expect("valid original response cutoff");
        let record = build_reserved_record(
            key.clone(),
            key_digest,
            cutoff,
            1,
            ReceiptVersion::initial(),
            false,
        );
        let (_, encoded) = serialize_reserved_record(record, MAX_TASK_RECORD_ENVELOPE_BYTES as u64)
            .expect("serialize foreign live row fixture");
        let name = format!("{}.json", receipt_key_digest(&key).as_str());
        let mut row = create_owner_only_file_child(&store.active_file, OsStr::new(&name))
            .expect("create foreign owner-only live row fixture");
        row.write_all(&encoded)
            .and_then(|()| row.sync_all())
            .expect("persist foreign live row fixture");
        sync_directory(&store.active_file).expect("sync foreign live row fixture");

        let collision = store
            .reserve(key, cutoff, reserve_deadline())
            .expect_err("foreign target row must reject no-replace publication");

        assert!(matches!(
            collision,
            ReceiptLedgerError::Storage {
                operation: "atomically publish receipt row",
                ..
            }
        ));
        let generation_before =
            fs::read(receipts.join(GENERATION_FILE_NAME)).expect("read generation after collision");
        let names_before = directory_names(&receipts.join(ACTIVE_DIRECTORY_NAME));
        let fail_stop = store
            .reserve(
                receipt_key(INVOCATION_B, TASK_B, "workspace-b"),
                cutoff,
                reserve_deadline(),
            )
            .expect_err("foreign publication collision must latch the writer unavailable");

        assert_eq!(fail_stop, ReceiptLedgerError::StoreUnavailable);
        assert_eq!(
            fs::read(receipts.join(GENERATION_FILE_NAME))
                .expect("fail-stop leaves generation untouched"),
            generation_before
        );
        assert_eq!(
            directory_names(&receipts.join(ACTIVE_DIRECTORY_NAME)),
            names_before
        );
    }

    #[test]
    fn exact_missing_observation_is_key_bound_and_store_minted() {
        let root = tempfile::tempdir().expect("temporary root");
        let receipts = fs::canonicalize(root.path())
            .expect("physical temporary root")
            .join("receipts");
        let store = ReceiptLedgerStore::open(&receipts).expect("open receipt ledger");
        let expected = digest('c');

        let observation = store
            .inspect_exact(&expected)
            .expect("inspect exact missing receipt");

        assert_eq!(observation.receipt_key_digest(), &expected);
        assert_eq!(observation.generation_before(), 0);
        assert_eq!(observation.generation_after(), 0);
    }

    #[test]
    fn exact_missing_observation_rejects_a_catalogued_row_missing_from_disk() {
        let root = tempfile::tempdir().expect("temporary root");
        let receipts = fs::canonicalize(root.path())
            .expect("physical temporary root")
            .join("receipts");
        let store = ReceiptLedgerStore::open(&receipts).expect("open receipt ledger");
        let key = receipt_key(INVOCATION_A, TASK_A, "workspace-a");
        let key_digest = crate::application::receipt_ledger::receipt_key_digest(&key);
        let cutoff =
            OriginalCutoffDescriptor::new(1_000, 7_000).expect("valid original response cutoff");
        store
            .reserve(key, cutoff, reserve_deadline())
            .expect("reserve catalogued receipt");
        let row_path = receipts
            .join(ACTIVE_DIRECTORY_NAME)
            .join(format!("{}.json", key_digest.as_str()));
        let displaced_path = receipts
            .join(ACTIVE_DIRECTORY_NAME)
            .join(".unica-cleanup-dddddddd-dddd-4ddd-8ddd-dddddddddddd");
        fs::rename(&row_path, &displaced_path)
            .expect("simulate a catalogued row displaced without generation change");
        let generation_before =
            fs::read(receipts.join(GENERATION_FILE_NAME)).expect("read generation before failure");
        let names_before = directory_names(&receipts.join(ACTIVE_DIRECTORY_NAME));

        let error = store
            .inspect_exact(&key_digest)
            .expect_err("catalog/disk disagreement cannot mint missing-receipt authority");

        assert_eq!(
            error,
            ReceiptLedgerError::Corrupt("catalogued receipt row is missing")
        );
        let fail_stop = store
            .reserve(
                receipt_key(INVOCATION_B, TASK_B, "workspace-b"),
                cutoff,
                reserve_deadline(),
            )
            .expect_err("observed catalog corruption must latch the writer unavailable");
        assert_eq!(fail_stop, ReceiptLedgerError::StoreUnavailable);
        assert_eq!(
            fs::read(receipts.join(GENERATION_FILE_NAME))
                .expect("fail-stop leaves generation untouched"),
            generation_before
        );
        assert_eq!(
            directory_names(&receipts.join(ACTIVE_DIRECTORY_NAME)),
            names_before
        );
    }

    #[test]
    fn observed_generation_drift_latches_the_writer_before_another_reservation() {
        let root = tempfile::tempdir().expect("temporary root");
        let receipts = fs::canonicalize(root.path())
            .expect("physical temporary root")
            .join("receipts");
        let store = ReceiptLedgerStore::open(&receipts).expect("open receipt ledger");
        let expected_digest = digest('a');

        let error = store
            .inspect_exact_after_row_lookup(&expected_digest, || {
                let mut generation = store.generation.lock().expect("lock generation fixture");
                generation
                    .file
                    .set_len(0)
                    .and_then(|()| generation.file.seek(SeekFrom::Start(0)).map(|_| ()))
                    .and_then(|()| generation.file.write_all(b"1\n"))
                    .and_then(|()| generation.file.sync_all())
                    .expect("persist external generation drift fixture");
            })
            .expect_err("generation drift must reject the exact observation");

        assert_eq!(
            error,
            ReceiptLedgerError::ConcurrentGenerationChange {
                generation_before: 0,
                generation_after: 1,
            }
        );
        let generation_before =
            fs::read(receipts.join(GENERATION_FILE_NAME)).expect("read drifted generation");
        let names_before = directory_names(&receipts.join(ACTIVE_DIRECTORY_NAME));
        let cutoff =
            OriginalCutoffDescriptor::new(1_000, 7_000).expect("valid original response cutoff");
        let fail_stop = store
            .reserve(
                receipt_key(INVOCATION_B, TASK_B, "workspace-b"),
                cutoff,
                reserve_deadline(),
            )
            .expect_err("generation drift must latch the writer unavailable");

        assert_eq!(fail_stop, ReceiptLedgerError::StoreUnavailable);
        assert_eq!(
            fs::read(receipts.join(GENERATION_FILE_NAME))
                .expect("fail-stop leaves drifted generation untouched"),
            generation_before
        );
        assert_eq!(
            directory_names(&receipts.join(ACTIVE_DIRECTORY_NAME)),
            names_before
        );
    }

    #[test]
    fn observed_receipt_acl_drift_latches_the_writer_before_another_reservation() {
        let root = tempfile::tempdir().expect("temporary root");
        let receipts = fs::canonicalize(root.path())
            .expect("physical temporary root")
            .join("receipts");
        let store = ReceiptLedgerStore::open(&receipts).expect("open receipt ledger");
        let key = receipt_key(INVOCATION_A, TASK_A, "workspace-a");
        let key_digest = crate::application::receipt_ledger::receipt_key_digest(&key);
        let cutoff =
            OriginalCutoffDescriptor::new(1_000, 7_000).expect("valid original response cutoff");
        store
            .reserve(key, cutoff, reserve_deadline())
            .expect("reserve catalogued receipt");
        let row_path = receipts
            .join(ACTIVE_DIRECTORY_NAME)
            .join(format!("{}.json", key_digest.as_str()));
        if !set_unix_mode_for_test(&row_path, 0o644).expect("weaken receipt row mode fixture") {
            return;
        }
        let generation_before =
            fs::read(receipts.join(GENERATION_FILE_NAME)).expect("read generation before failure");
        let names_before = directory_names(&receipts.join(ACTIVE_DIRECTORY_NAME));

        let error = store
            .inspect_exact(&key_digest)
            .expect_err("receipt ACL drift must reject the exact observation");

        assert!(matches!(
            error,
            ReceiptLedgerError::Storage {
                operation: "verify receipt row ownership",
                ..
            }
        ));
        let fail_stop = store
            .reserve(
                receipt_key(INVOCATION_B, TASK_B, "workspace-b"),
                cutoff,
                reserve_deadline(),
            )
            .expect_err("receipt ACL drift must latch the writer unavailable");
        assert_eq!(fail_stop, ReceiptLedgerError::StoreUnavailable);
        assert_eq!(
            fs::read(receipts.join(GENERATION_FILE_NAME))
                .expect("fail-stop leaves generation untouched"),
            generation_before
        );
        assert_eq!(
            directory_names(&receipts.join(ACTIVE_DIRECTORY_NAME)),
            names_before
        );
    }

    #[test]
    fn authority_failure_latches_every_live_entry_point_until_reopen() {
        for operation in ["generation", "observe", "read", "inspect", "reserve"] {
            let root = tempfile::tempdir().expect("temporary root");
            let receipts = fs::canonicalize(root.path())
                .expect("physical temporary root")
                .join(format!("receipts-{operation}"));
            let store = ReceiptLedgerStore::open(&receipts).expect("open receipt ledger");
            let generation_path = receipts.join(GENERATION_FILE_NAME);
            if !set_unix_mode_for_test(&generation_path, 0o644)
                .expect("weaken generation mode fixture")
            {
                return;
            }
            let cutoff = OriginalCutoffDescriptor::new(1_000, 7_000)
                .expect("valid original response cutoff");
            let missing_digest = digest('a');

            let authority_error = match operation {
                "generation" => store
                    .generation()
                    .map(|_| ())
                    .expect_err("generation ACL drift"),
                "observe" => store
                    .observe_stable_generation()
                    .map(|_| ())
                    .expect_err("observe ACL drift"),
                "read" => store
                    .read_reserved(&missing_digest)
                    .map(|_| ())
                    .expect_err("read ACL drift"),
                "inspect" => store
                    .inspect_exact(&missing_digest)
                    .map(|_| ())
                    .expect_err("inspect ACL drift"),
                "reserve" => store
                    .reserve(
                        receipt_key(INVOCATION_A, TASK_A, "workspace-a"),
                        cutoff,
                        reserve_deadline(),
                    )
                    .map(|_| ())
                    .expect_err("reserve ACL drift"),
                _ => unreachable!("closed live entry-point fixture"),
            };
            assert!(matches!(
                authority_error,
                ReceiptLedgerError::Storage {
                    operation: "verify generation record ownership",
                    ..
                }
            ));
            set_unix_mode_for_test(&generation_path, 0o600)
                .expect("restore generation mode fixture");
            let generation_before =
                fs::read(&generation_path).expect("read restored generation before fail-stop");
            let names_before = directory_names(&receipts.join(ACTIVE_DIRECTORY_NAME));

            let fail_stop = store
                .reserve(
                    receipt_key(INVOCATION_B, TASK_B, "workspace-b"),
                    cutoff,
                    reserve_deadline(),
                )
                .expect_err("observed authority failure must require reopen");

            assert_eq!(
                fail_stop,
                ReceiptLedgerError::StoreUnavailable,
                "entry point {operation} must latch authority failure"
            );
            assert_eq!(
                fs::read(&generation_path).expect("fail-stop leaves generation untouched"),
                generation_before
            );
            assert_eq!(
                directory_names(&receipts.join(ACTIVE_DIRECTORY_NAME)),
                names_before
            );
        }
    }

    #[test]
    fn stable_generation_observation_is_store_minted_after_two_sided_validation() {
        let root = tempfile::tempdir().expect("temporary root");
        let receipts = fs::canonicalize(root.path())
            .expect("physical temporary root")
            .join("receipts");
        let store = ReceiptLedgerStore::open(&receipts).expect("open receipt ledger");

        let observation = store
            .observe_stable_generation()
            .expect("observe a stable validated generation");

        assert_eq!(observation.generation_before(), 0);
        assert_eq!(observation.generation_after(), 0);
    }

    #[test]
    fn retained_named_capability_opens_without_reconstructing_the_receipts_path() {
        let root = tempfile::tempdir().expect("temporary root");
        let root = fs::canonicalize(root.path()).expect("physical temporary root");
        let receipts = root.join("receipts");
        {
            let store = ReceiptLedgerStore::open(&receipts).expect("initialize receipt ledger");
            assert_eq!(store.generation().expect("initial generation"), 0);
        }
        let parent = RetainedDirectoryCapability::open(&root).expect("retain state directory");
        let receipts = parent
            .retain_directory_child(OsStr::new("receipts"))
            .expect("retain named receipts child");

        let store = ReceiptLedgerStore::open_retained_directory(receipts)
            .expect("open from retained named capability");

        assert_eq!(store.generation().expect("retained generation"), 0);
    }

    #[test]
    fn exact_missing_receipt_keeps_the_same_persisted_generation_without_mutation() {
        let root = tempfile::tempdir().expect("temporary root");
        let receipts = fs::canonicalize(root.path())
            .expect("physical temporary root")
            .join("receipts");
        let store = ReceiptLedgerStore::open(&receipts).expect("open receipt ledger");
        let names_before = directory_names(&receipts);
        let mut generation_before = Vec::new();
        fs::File::open(receipts.join("generation"))
            .expect("generation record")
            .read_to_end(&mut generation_before)
            .expect("read generation bytes");

        let expected_digest = digest('a');
        let observation = store
            .inspect_exact(&expected_digest)
            .expect("inspect exact missing receipt");
        assert_eq!(observation.receipt_key_digest(), &expected_digest);
        assert_eq!(observation.generation_before(), 0);
        assert_eq!(observation.generation_after(), 0);

        assert_eq!(directory_names(&receipts), names_before);
        assert_eq!(
            fs::read(receipts.join("generation")).expect("reread generation bytes"),
            generation_before
        );
    }

    #[test]
    fn present_receipt_revalidates_named_active_authority_after_lookup() {
        let root = tempfile::tempdir().expect("active temporary root");
        let root = fs::canonicalize(root.path()).expect("physical active temporary root");
        let receipts = root.join("receipts");
        let active = receipts.join(ACTIVE_DIRECTORY_NAME);
        let displaced_active = receipts.join("active-displaced");
        let store = ReceiptLedgerStore::open(&receipts).expect("open receipt ledger");
        let expected_digest = digest('e');
        let record_name = format!("{}.json", expected_digest.as_str());
        let mut record = create_owner_only_file_child(&store.active_file, OsStr::new(&record_name))
            .expect("create owner-only receipt row");
        record
            .write_all(b"{}\n")
            .and_then(|()| record.sync_all())
            .expect("persist receipt row fixture");
        sync_directory(&store.active_file).expect("sync receipt row fixture");
        drop(record);
        let replacement = Cell::new(None);

        let error = store
            .inspect_exact_after_row_lookup(&expected_digest, || {
                let outcome =
                    attempt_retained_directory_replacement_for_test(&active, &displaced_active)
                        .expect("attempt named active replacement after row lookup");
                if outcome == RetainedDirectoryReplacementOutcome::Replaced {
                    drop(
                        create_owner_only_directory_child(
                            &store.receipts_file,
                            OsStr::new(ACTIVE_DIRECTORY_NAME),
                        )
                        .expect("create replacement owner-only active directory"),
                    );
                }
                replacement.set(Some(outcome));
            })
            .expect_err("a present receipt is not decodable in the W0a shell");
        let outcome = replacement
            .get()
            .expect("present-row lookup returned before the post-lookup authority checkpoint");

        match outcome {
            RetainedDirectoryReplacementOutcome::Replaced => assert!(matches!(
                error,
                ReceiptLedgerError::Storage {
                    operation: "validate named receipt active directory",
                    ..
                }
            )),
            RetainedDirectoryReplacementOutcome::PreventedByRetainedHandle => {
                assert_eq!(
                    error,
                    ReceiptLedgerError::Corrupt(
                        "receipt row is present outside the recovered catalog"
                    )
                )
            }
        }
    }

    #[test]
    fn receipt_directory_link_or_reparse_point_is_rejected_without_touching_target() {
        let root = tempfile::tempdir().expect("temporary root");
        let outside = tempfile::tempdir().expect("outside directory");
        let receipts = fs::canonicalize(root.path())
            .expect("physical temporary root")
            .join("receipts");
        let outside = fs::canonicalize(outside.path()).expect("physical outside directory");
        match create_directory_link_fixture_for_test(&outside, &receipts)
            .expect("create directory-link fixture")
        {
            FileLinkFixtureOutcome::Created => {}
            FileLinkFixtureOutcome::Unsupported
            | FileLinkFixtureOutcome::WindowsPrivilegeUnavailable => return,
        }

        assert!(ReceiptLedgerStore::open(&receipts).is_err());
        assert!(directory_names(&outside).is_empty());
    }

    #[test]
    fn named_receipts_replacement_never_leaves_two_usable_owners() {
        let root = tempfile::tempdir().expect("temporary root");
        let root = fs::canonicalize(root.path()).expect("physical temporary root");
        let receipts = root.join("receipts");
        let displaced = root.join("receipts-displaced");
        let first = ReceiptLedgerStore::open(&receipts).expect("open first receipt owner");

        match attempt_retained_directory_replacement_for_test(&receipts, &displaced)
            .expect("attempt named receipt replacement")
        {
            RetainedDirectoryReplacementOutcome::Replaced => {
                let second = ReceiptLedgerStore::open(&receipts)
                    .expect("replacement may become the named receipt owner");
                assert_eq!(second.generation().expect("replacement generation"), 0);
                assert!(
                    first.generation().is_err(),
                    "the displaced owner remained usable beside the replacement owner"
                );
            }
            RetainedDirectoryReplacementOutcome::PreventedByRetainedHandle => {
                assert_eq!(first.generation().expect("retained generation"), 0);
                assert!(matches!(
                    ReceiptLedgerStore::open(&receipts),
                    Err(ReceiptLedgerError::AlreadyOwned)
                ));
            }
        }
    }

    #[test]
    fn named_active_replacement_invalidates_the_retained_owner() {
        let active_root = tempfile::tempdir().expect("active temporary root");
        let active_root =
            fs::canonicalize(active_root.path()).expect("physical active temporary root");
        let active_receipts = active_root.join("receipts");
        let active = active_receipts.join("active");
        let displaced_active = active_receipts.join("active-displaced");
        let active_store =
            ReceiptLedgerStore::open(&active_receipts).expect("open active receipt owner");

        match attempt_retained_directory_replacement_for_test(&active, &displaced_active)
            .expect("attempt named active replacement")
        {
            RetainedDirectoryReplacementOutcome::Replaced => {
                fs::create_dir(&active).expect("create replacement active directory");
                assert!(
                    active_store.inspect_exact(&digest('d')).is_err(),
                    "the owner accepted a replacement active directory"
                );
            }
            RetainedDirectoryReplacementOutcome::PreventedByRetainedHandle => {
                let expected_digest = digest('d');
                let observation = active_store
                    .inspect_exact(&expected_digest)
                    .expect("retained active inspection");
                assert_eq!(observation.receipt_key_digest(), &expected_digest);
                assert_eq!(observation.generation_before(), 0);
                assert_eq!(observation.generation_after(), 0);
            }
        }
    }

    #[test]
    fn named_generation_replacement_invalidates_the_retained_owner() {
        let generation_root = tempfile::tempdir().expect("generation temporary root");
        let generation_root =
            fs::canonicalize(generation_root.path()).expect("physical generation temporary root");
        let generation_receipts = generation_root.join("receipts");
        let generation = generation_receipts.join("generation");
        let displaced_generation = generation_receipts.join("generation-displaced");
        let generation_store =
            ReceiptLedgerStore::open(&generation_receipts).expect("open generation receipt owner");

        match fs::rename(&generation, &displaced_generation) {
            Ok(()) => {
                fs::write(&generation, b"0\n").expect("write replacement generation");
                assert!(
                    generation_store.generation().is_err(),
                    "the owner accepted a replacement generation record"
                );
            }
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::PermissionDenied | io::ErrorKind::WouldBlock
                ) =>
            {
                assert_eq!(
                    generation_store
                        .generation()
                        .expect("retained generation after prevented replacement"),
                    0
                );
                assert!(!displaced_generation.exists());
            }
            Err(error) => panic!("attempt named generation replacement: {error}"),
        }
    }
}
