//! Юнит-тесты хранилища квитанций. Вынесены из `receipt_ledger.rs`:
//! файл перевалил за шестнадцать тысяч строк, из них треть — тесты.
//! Путь модуля прежний (`infrastructure::receipt_ledger::tests`).

use super::*;
use crate::application::invocation::normalized_arguments_hash;
use crate::application::invocation_store_v5::V5SafeFailureReason;
use crate::application::receipt_ledger::{
    canonical_v5_terminal, request_scope_hash, CoreIdentityDigest, LifecycleLinkRecordHeader,
    OriginalCutoffDescriptor, ReceiptKey, ReceiptLedgerPort, ReceiptState, ReceiptTerminalOutcome,
    ReceiptVersion, RequestIdentity, ReserveOutcome, TaskBoundReceipt, V5ToolIdentity,
};
use crate::domain::invocation::{DomainResult, InvocationId, SafeIdentityHash, TaskId};
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

fn receipt_key(invocation_id: &str, reserved_task_id: &str, workspace_hint: &str) -> ReceiptKey {
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

fn direct_terminal_fixture(receipts: &Path) -> (ReceiptLedgerStore, ReceiptKey, TerminalDigest) {
    let store = ReceiptLedgerStore::open(receipts).expect("open receipt ledger");
    let key = receipt_key(INVOCATION_A, TASK_A, "workspace-a");
    let reserved = store
        .reserve(
            key.clone(),
            OriginalCutoffDescriptor::new(1_000, 7_000).expect("valid cutoff"),
            reserve_deadline(),
        )
        .expect("reserve exact receipt")
        .into_reservation()
        .expect("receipt remains reserved");
    let terminal = canonical_v5_terminal(&ReceiptTerminalOutcome::Cancelled)
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
        .expect("publish direct terminal");
    (store, key, terminal_digest)
}

fn reserve_deadline() -> Instant {
    Instant::now() + Duration::from_secs(2)
}

fn near_limit_payload_deadline() -> Instant {
    Instant::now() + Duration::from_secs(15)
}

fn confirmed_task_bound(handoff: &TaskHandoffActorBoundReceipt) -> TaskBoundReceipt {
    let header =
        LifecycleLinkRecordHeader::new(handoff.key().clone(), handoff.link().clone(), 2, 1, 512)
            .expect("valid lifecycle-link header");
    TaskBoundReceipt::new(
        header,
        handoff.task().clone(),
        handoff.task().version(),
        handoff.task().created_at_epoch_ms() + 1,
        handoff.phase(),
    )
    .expect("valid confirmed TaskBound proof")
}

fn confirmed_promised_task_bound(promised: &TaskPromisedActorBoundReceipt) -> TaskBoundReceipt {
    let header =
        LifecycleLinkRecordHeader::new(promised.key().clone(), promised.link().clone(), 2, 1, 512)
            .expect("valid promised lifecycle-link header");
    TaskBoundReceipt::new(
        header,
        promised.task().clone(),
        promised.task().version(),
        promised.task().created_at_epoch_ms() + 1,
        AttemptPhase::NotBegun,
    )
    .expect("valid promised TaskBound proof")
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
        ReceiptLedgerError::Corrupt("generation staging name does not contain a canonical UUIDv4")
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
        _ => panic!("first cancel must create CancelReserved"),
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
        _ => panic!("duplicate cancel must reuse CancelReserved"),
    };
    assert_eq!(duplicate, initial);
    assert_eq!(store.generation().expect("stable generation"), 1);
    let expired_duplicate_with_overflow = store
        .request_cancel_or_reserve(
            key.clone(),
            u64::MAX - CANCEL_RESERVATION_TTL_MS + 1,
            reserve_deadline(),
        )
        .expect_err("an expired duplicate must validate its new absolute expiry");
    assert_eq!(
        expired_duplicate_with_overflow,
        ReceiptLedgerError::TimestampOverflow
    );
    assert_eq!(
        store.generation().expect("rejected duplicate generation"),
        1
    );
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
fn duplicate_cancel_at_expiry_reclaims_the_stale_row_before_new_admission() {
    let root = tempfile::tempdir().expect("temporary root");
    let receipts = fs::canonicalize(root.path())
        .expect("physical temporary root")
        .join("receipts");
    let store = ReceiptLedgerStore::open(&receipts).expect("open receipt ledger");
    let key = receipt_key(INVOCATION_A, TASK_A, "workspace-a");
    let stale = match store
        .request_cancel_or_reserve(key.clone(), 1_000, reserve_deadline())
        .expect("reserve cancellation before submit")
    {
        CancelResolution::NewlyReserved(receipt) => receipt,
        _ => panic!("first cancel must create CancelReserved"),
    };

    let current = match store
        .request_cancel_or_reserve(key.clone(), stale.expires_at_epoch_ms(), reserve_deadline())
        .expect("the boundary call reclaims stale state before admitting a new cancel")
    {
        CancelResolution::NewlyReserved(receipt) => receipt,
        _ => panic!("expired duplicate must become a new admission"),
    };

    assert_eq!(current.key(), &key);
    assert_eq!(current.cancel_reserved_at_epoch_ms(), 8_125);
    assert_eq!(current.expires_at_epoch_ms(), 15_250);
    assert_eq!(current.record_version(), ReceiptVersion::initial());
    assert_eq!(current.mutation_sequence(), 3);
    assert_eq!(
        store
            .generation()
            .expect("expiry plus admission generation"),
        3
    );
    assert_eq!(
        store
            .recover_exact(&key, reserve_deadline())
            .expect("only the fresh reservation remains live"),
        ReceiptState::CancelReserved(current)
    );
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
        _ => panic!("first cancel must create CancelReserved"),
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
        _ => panic!("old cancellation must be newly reserved"),
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
        _ => panic!("recreated cancellation must be newly reserved"),
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
        _ => panic!("fixture must be an expiry witness"),
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
        _ => panic!("fixture must be an expiry witness"),
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
        _ => panic!("second cancellation must be newly reserved"),
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
        _ => panic!("cancellation must be newly reserved"),
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
        _ => panic!("fixture must remain an expiry witness"),
    }
    let (_, encoded) = serialize_reserved_record(witness, MAX_CANCEL_RESERVED_RECORD_BYTES)
        .expect("encode canonical skipped-generation witness");
    fs::write(&row, encoded).expect("persist skipped-generation fixture");

    assert_eq!(
        ReceiptLedgerStore::open(&receipts)
            .err()
            .expect("witness cannot skip the next persisted generation"),
        ReceiptLedgerError::Corrupt(
            "pending receipt mutation witness is not the next persisted mutation"
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
        _ => panic!("cancellation must be newly reserved"),
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
        _ => panic!("cancellation must be newly reserved"),
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
        _ => panic!("first cancellation must be newly reserved"),
    };
    let surviving = match store
        .request_cancel_or_reserve(surviving_key.clone(), 2_000, reserve_deadline())
        .expect("interleave a second receipt mutation")
    {
        CancelResolution::NewlyReserved(receipt) => receipt,
        _ => panic!("second cancellation must be newly reserved"),
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
        _ => panic!("cancellation must be newly reserved"),
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
fn submit_admission_reclaims_one_slot_from_a_full_expired_cancel_pool() {
    let root = tempfile::tempdir().expect("temporary root");
    let receipts = fs::canonicalize(root.path())
        .expect("physical temporary root")
        .join("receipts");
    let store = ReceiptLedgerStore::open(&receipts).expect("open receipt ledger");

    for index in 0..MAX_LIVE_RECEIPTS {
        let key = receipt_key_with_ids(
            InvocationId::new(),
            TaskId::new(),
            &format!("expired-cancel-workspace-{index}"),
        );
        assert!(matches!(
            store.request_cancel_or_reserve(key, 1_000, Instant::now() + Duration::from_secs(7),),
            Ok(CancelResolution::NewlyReserved(_))
        ));
    }

    let admitted_key = receipt_key_with_ids(
        InvocationId::new(),
        TaskId::new(),
        "admitted-after-expired-cancel-pool",
    );
    let cutoff =
        OriginalCutoffDescriptor::new(8_125, 7_000).expect("valid post-expiry submit cutoff");
    let admitted = store
        .reserve(
            admitted_key.clone(),
            cutoff,
            Instant::now() + Duration::from_secs(7),
        )
        .expect("expired cancel reservations cannot deny later admission")
        .into_reservation()
        .expect("new submit remains reserved");

    assert_eq!(admitted.key(), &admitted_key);
    assert_eq!(admitted.mutation_sequence(), 66);
    assert_eq!(store.generation().expect("reclaim plus admission"), 66);
    let catalog = store.writer.lock().expect("inspect reclaimed catalog");
    assert_eq!(catalog.records.len(), MAX_LIVE_RECEIPTS);
    assert_eq!(catalog.invocation_index.len(), MAX_LIVE_RECEIPTS);
    assert_eq!(catalog.reserved_task_index.len(), MAX_LIVE_RECEIPTS);
    assert_eq!(
        catalog
            .records
            .values()
            .filter(|entry| matches!(
                entry.record.lifecycle,
                StoredActiveLifecycleV1::CancelReserved { .. }
            ))
            .count(),
        MAX_LIVE_RECEIPTS - 1
    );
    assert_eq!(
        catalog.reserved_result_bytes,
        admitted.reserved_result_bytes()
    );
}

#[test]
fn partial_identity_rejection_does_not_reclaim_an_unrelated_expired_cancel() {
    let root = tempfile::tempdir().expect("temporary root");
    let receipts = fs::canonicalize(root.path())
        .expect("physical temporary root")
        .join("receipts");
    let store = ReceiptLedgerStore::open(&receipts).expect("open receipt ledger");
    let expired_key = receipt_key(INVOCATION_A, TASK_A, "expired-workspace");
    store
        .request_cancel_or_reserve(expired_key.clone(), 1_000, reserve_deadline())
        .expect("seed expired cancellation reservation");
    let live_key = receipt_key_with_ids(InvocationId::new(), TaskId::new(), "live-workspace");
    let live_cutoff = OriginalCutoffDescriptor::new(1_000, 7_000).expect("valid live cutoff");
    store
        .reserve(live_key.clone(), live_cutoff, reserve_deadline())
        .expect("seed live exact identity");
    let mismatch = receipt_key_with_ids(
        live_key.invocation_id(),
        TaskId::new(),
        "mismatching-workspace",
    );

    assert_eq!(
        store
            .request_cancel_or_reserve(mismatch, 8_125, reserve_deadline())
            .expect_err("live partial identity must reject"),
        ReceiptLedgerError::InvocationIdentityMismatch
    );
    assert_eq!(store.generation().expect("rejection generation"), 2);
    assert!(matches!(
        store
            .recover_exact(&expired_key, reserve_deadline())
            .expect("rejection cannot run unrelated housekeeping"),
        ReceiptState::CancelReserved(_)
    ));
}

#[test]
fn expired_partial_identity_is_reclaimed_before_new_admission() {
    let root = tempfile::tempdir().expect("temporary root");
    let receipts = fs::canonicalize(root.path())
        .expect("physical temporary root")
        .join("receipts");
    let store = ReceiptLedgerStore::open(&receipts).expect("open receipt ledger");
    let expired_key = receipt_key(INVOCATION_A, TASK_A, "expired-workspace");
    store
        .request_cancel_or_reserve(expired_key, 1_000, reserve_deadline())
        .expect("seed expired cancellation reservation");
    let admitted_key = receipt_key_with_ids(
        InvocationId::from_str(INVOCATION_A).expect("canonical reused invocation"),
        TaskId::new(),
        "new-workspace",
    );

    let admitted = match store
        .request_cancel_or_reserve(admitted_key.clone(), 8_125, reserve_deadline())
        .expect("expired identity owner is reclaimable")
    {
        CancelResolution::NewlyReserved(receipt) => receipt,
        _ => panic!("reclaimed identity must admit a new receipt"),
    };

    assert_eq!(admitted.key(), &admitted_key);
    assert_eq!(admitted.mutation_sequence(), 3);
    assert_eq!(store.generation().expect("reclaim plus admission"), 3);
    let catalog = store
        .writer
        .lock()
        .expect("inspect reused identity catalog");
    assert_eq!(catalog.records.len(), 1);
    assert_eq!(catalog.invocation_index.len(), 1);
    assert_eq!(catalog.reserved_task_index.len(), 1);
}

#[test]
fn exact_reserve_winner_does_not_reclaim_an_unrelated_expired_cancel() {
    let root = tempfile::tempdir().expect("temporary root");
    let receipts = fs::canonicalize(root.path())
        .expect("physical temporary root")
        .join("receipts");
    let store = ReceiptLedgerStore::open(&receipts).expect("open receipt ledger");
    let expired_key = receipt_key(INVOCATION_A, TASK_A, "expired-workspace");
    store
        .request_cancel_or_reserve(expired_key.clone(), 1_000, reserve_deadline())
        .expect("seed expired cancellation reservation");
    let live_key = receipt_key_with_ids(InvocationId::new(), TaskId::new(), "live-workspace");
    let initial_cutoff = OriginalCutoffDescriptor::new(1_000, 7_000).expect("valid initial cutoff");
    let initial = store
        .reserve(live_key.clone(), initial_cutoff, reserve_deadline())
        .expect("seed live exact identity");
    let duplicate_cutoff =
        OriginalCutoffDescriptor::new(8_125, 1).expect("irrelevant duplicate cutoff");

    let duplicate = store
        .reserve(live_key, duplicate_cutoff, reserve_deadline())
        .expect("exact duplicate returns its original winner");
    assert_eq!(duplicate.into_state(), initial.into_state());
    assert_eq!(store.generation().expect("duplicate generation"), 2);
    assert!(matches!(
        store
            .recover_exact(&expired_key, reserve_deadline())
            .expect("duplicate cannot run unrelated housekeeping"),
        ReceiptState::CancelReserved(_)
    ));
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
        _ => panic!("cancel conversion is a durable mutation"),
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
        _ => panic!("first cancellation must reserve"),
    };
    let cutoff = OriginalCutoffDescriptor::new(cancel.expires_at_epoch_ms(), 7_000)
        .expect("valid submit cutoff at the half-open expiry boundary");

    let converted = match store
        .reserve(key.clone(), cutoff, reserve_deadline())
        .expect("replace expired cancellation with exact submit")
    {
        ReserveOutcome::Created(receipt) => receipt,
        _ => panic!("expired exact conversion is a mutation"),
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
    if !set_unix_mode_for_test(&generation_path, 0o644).expect("weaken generation mode fixture") {
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
fn reserved_actor_binding_and_begun_are_durable_exact_cas() {
    let root = tempfile::tempdir().expect("temporary root");
    let receipts = fs::canonicalize(root.path())
        .expect("physical temporary root")
        .join("receipts");
    let key = receipt_key(INVOCATION_A, TASK_A, "workspace-a");
    let cutoff =
        OriginalCutoffDescriptor::new(1_000, 7_000).expect("valid original response cutoff");
    let actor_identity = SafeIdentityHash::from_sha256([0x77; 32]);

    let begun = {
        let store = ReceiptLedgerStore::open(&receipts).expect("open receipt ledger");
        let reserved = store
            .reserve(key.clone(), cutoff, reserve_deadline())
            .expect("reserve exact receipt")
            .into_reservation()
            .expect("receipt remains reserved");
        let bound = store
            .bind_reserved_actor(
                &key,
                reserved.record_version(),
                actor_identity.clone(),
                reserve_deadline(),
            )
            .expect("durably bind exact actor identity");
        assert_eq!(
            bound.phase(),
            &ReservedPhase::ActorBound {
                bound_workspace_identity: actor_identity.clone(),
            }
        );
        store
            .mark_reserved_begun(&key, bound.record_version(), reserve_deadline())
            .expect("durably mark exact attempt begun")
    };

    assert_eq!(
        begun.phase(),
        &ReservedPhase::Begun {
            bound_workspace_identity: actor_identity.clone(),
        }
    );
    assert_eq!(begun.record_version().get(), 3);
    assert_eq!(begun.mutation_sequence(), 3);

    let reopened = ReceiptLedgerStore::open(&receipts).expect("reopen receipt ledger");
    let recovered = reopened
        .recover_exact(&key, reserve_deadline())
        .expect("recover begun receipt");
    assert_eq!(recovered, ReceiptState::Reserved(begun));
    assert_eq!(reopened.generation().expect("reopened generation"), 3);
}

#[test]
fn reserved_to_unbound_task_promise_is_exact_and_reopens() {
    let root = tempfile::tempdir().expect("temporary root");
    let receipts = fs::canonicalize(root.path())
        .expect("physical temporary root")
        .join("receipts");
    let key = receipt_key(INVOCATION_A, TASK_A, "workspace-a");
    let cutoff =
        OriginalCutoffDescriptor::new(1_000, 7_000).expect("valid original response cutoff");

    let promised = {
        let store = ReceiptLedgerStore::open(&receipts).expect("open receipt ledger");
        let reserved = store
            .reserve(key.clone(), cutoff, reserve_deadline())
            .expect("reserve exact receipt")
            .into_reservation()
            .expect("receipt remains reserved");
        store
            .promise_task_unbound(
                &key,
                reserved.record_version(),
                1_007,
                3_600_000,
                250,
                reserve_deadline(),
            )
            .expect("durably promise exact reserved Task")
    };

    assert_eq!(promised.key(), &key);
    assert_eq!(
        promised.task().task_id(),
        TaskId::from_str(TASK_A).expect("valid task fixture id")
    );
    assert_eq!(
        promised.task().invocation_id(),
        InvocationId::from_str(INVOCATION_A).expect("valid invocation fixture id")
    );
    assert_eq!(promised.task().created_at_epoch_ms(), 1_007);
    assert_eq!(promised.record_version().get(), 2);
    assert_eq!(promised.mutation_sequence(), 2);

    let reopened = ReceiptLedgerStore::open(&receipts).expect("reopen receipt ledger");
    assert_eq!(
        reopened
            .recover_exact(&key, reserve_deadline())
            .expect("recover promised Task"),
        ReceiptState::TaskPromisedUnbound(promised)
    );
    assert_eq!(reopened.generation().expect("reopened generation"), 2);
}

#[test]
fn promised_task_actor_binding_is_exact_and_reopens() {
    let root = tempfile::tempdir().expect("temporary root");
    let receipts = fs::canonicalize(root.path())
        .expect("physical temporary root")
        .join("receipts");
    let key = receipt_key(INVOCATION_A, TASK_A, "workspace-a");
    let cutoff =
        OriginalCutoffDescriptor::new(1_000, 7_000).expect("valid original response cutoff");
    let workspace_identity = SafeIdentityHash::from_sha256([0x77; 32]);

    let bound = {
        let store = ReceiptLedgerStore::open(&receipts).expect("open receipt ledger");
        let reserved = store
            .reserve(key.clone(), cutoff, reserve_deadline())
            .expect("reserve exact receipt")
            .into_reservation()
            .expect("receipt remains reserved");
        let promised = store
            .promise_task_unbound(
                &key,
                reserved.record_version(),
                1_007,
                3_600_000,
                250,
                reserve_deadline(),
            )
            .expect("durably promise exact reserved Task");
        store
            .bind_promised_task_actor(
                &key,
                promised.record_version(),
                workspace_identity.clone(),
                reserve_deadline(),
            )
            .expect("durably bind promised Task actor")
    };

    assert_eq!(bound.key(), &key);
    assert_eq!(bound.task().task_id(), key.reserved_task_id());
    assert_eq!(bound.task().invocation_id(), key.invocation_id());
    assert_eq!(bound.workspace_identity_hash(), &workspace_identity);
    assert_eq!(bound.record_version().get(), 3);
    assert_eq!(bound.mutation_sequence(), 3);

    let reopened = ReceiptLedgerStore::open(&receipts).expect("reopen receipt ledger");
    assert_eq!(
        reopened
            .recover_exact(&key, reserve_deadline())
            .expect("recover actor-bound promised Task"),
        ReceiptState::TaskPromisedActorBound(bound)
    );
    assert_eq!(reopened.generation().expect("reopened generation"), 3);
}

#[test]
fn begun_reservation_handoff_intent_is_exact_and_reopens() {
    let root = tempfile::tempdir().expect("temporary root");
    let receipts = fs::canonicalize(root.path())
        .expect("physical temporary root")
        .join("receipts");
    let key = receipt_key(INVOCATION_A, TASK_A, "workspace-a");
    let cutoff =
        OriginalCutoffDescriptor::new(1_000, 7_000).expect("valid original response cutoff");
    let workspace_identity = SafeIdentityHash::from_sha256([0x77; 32]);

    let handoff = {
        let store = ReceiptLedgerStore::open(&receipts).expect("open receipt ledger");
        let reserved = store
            .reserve(key.clone(), cutoff, reserve_deadline())
            .expect("reserve exact receipt")
            .into_reservation()
            .expect("receipt remains reserved");
        let actor_bound = store
            .bind_reserved_actor(
                &key,
                reserved.record_version(),
                workspace_identity.clone(),
                reserve_deadline(),
            )
            .expect("bind exact actor");
        let begun = store
            .mark_reserved_begun(&key, actor_bound.record_version(), reserve_deadline())
            .expect("mark exact attempt begun");
        store
            .begin_bound_task_handoff(
                &key,
                begun.record_version(),
                1_009,
                3_600_000,
                250,
                reserve_deadline(),
            )
            .expect("persist begun Task handoff intent")
    };

    assert_eq!(handoff.key(), &key);
    assert_eq!(handoff.task().task_id(), key.reserved_task_id());
    assert_eq!(handoff.phase(), AttemptPhase::Begun);
    assert_eq!(handoff.workspace_identity_hash(), &workspace_identity);
    assert_eq!(handoff.record_version().get(), 4);
    assert_eq!(handoff.mutation_sequence(), 4);

    let reopened = ReceiptLedgerStore::open(&receipts).expect("reopen receipt ledger");
    assert_eq!(
        reopened
            .recover_exact(&key, reserve_deadline())
            .expect("recover begun handoff intent"),
        ReceiptState::TaskHandoffActorBound(handoff)
    );
    assert_eq!(reopened.generation().expect("reopened generation"), 4);
}

#[test]
fn promised_unbound_task_cancel_intent_is_exact_idempotent_and_reopens() {
    let root = tempfile::tempdir().expect("temporary root");
    let receipts = fs::canonicalize(root.path())
        .expect("physical temporary root")
        .join("receipts");
    let key = receipt_key(INVOCATION_A, TASK_A, "workspace-a");
    let cutoff =
        OriginalCutoffDescriptor::new(1_000, 7_000).expect("valid original response cutoff");
    let store = ReceiptLedgerStore::open(&receipts).expect("open receipt ledger");
    let reserved = store
        .reserve(key.clone(), cutoff, reserve_deadline())
        .expect("reserve exact receipt")
        .into_reservation()
        .expect("receipt remains reserved");
    let promised = store
        .promise_task_unbound(
            &key,
            reserved.record_version(),
            1_007,
            3_600_000,
            250,
            reserve_deadline(),
        )
        .expect("durably promise exact reserved Task");
    let expected = TaskCancellationReceipt::PromisedUnbound(promised.clone());

    let cancelled = store
        .request_task_cancel(&key, expected.clone(), reserve_deadline())
        .expect("persist exact Task cancellation intent");
    assert!(cancelled.cancel_requested());
    assert_eq!(cancelled.task(), promised.task());
    assert_eq!(
        cancelled.encoded_bytes() + cancelled.reserved_result_bytes(),
        promised.encoded_bytes() + promised.reserved_result_bytes()
    );
    assert_eq!(
        cancelled.record_version().get(),
        promised.record_version().get() + 1
    );
    assert_eq!(
        cancelled.mutation_sequence(),
        promised.mutation_sequence() + 1
    );

    let generation_after_cancel = store.generation().expect("generation after cancellation");
    assert_eq!(
        store
            .request_task_cancel(&key, expected, reserve_deadline())
            .expect("repeat exact cancellation is idempotent"),
        cancelled
    );
    assert_eq!(
        store.generation().expect("generation after exact repeat"),
        generation_after_cancel
    );

    drop(store);
    let reopened = ReceiptLedgerStore::open(&receipts).expect("reopen receipt ledger");
    assert_eq!(
        reopened
            .recover_exact(&key, reserve_deadline())
            .expect("recover durable cancellation intent"),
        cancelled.into_receipt_state()
    );
}

#[test]
fn promised_task_terminal_is_exact_idempotent_and_reopens() {
    let root = tempfile::tempdir().expect("temporary root");
    let receipts = fs::canonicalize(root.path())
        .expect("physical temporary root")
        .join("receipts");
    let key = receipt_key(INVOCATION_A, TASK_A, "workspace-a");
    let cutoff =
        OriginalCutoffDescriptor::new(1_000, 7_000).expect("valid original response cutoff");
    let store = ReceiptLedgerStore::open(&receipts).expect("open receipt ledger");
    let reserved = store
        .reserve(key.clone(), cutoff, reserve_deadline())
        .expect("reserve exact receipt")
        .into_reservation()
        .expect("receipt remains reserved");
    let promised = store
        .promise_task_unbound(
            &key,
            reserved.record_version(),
            1_007,
            3_600_000,
            250,
            reserve_deadline(),
        )
        .expect("durably promise exact reserved Task");
    let expected = TaskCancellationReceipt::PromisedUnbound(promised.clone());
    let terminal = canonical_v5_terminal(&ReceiptTerminalOutcome::Failed {
        reason: V5SafeFailureReason::Interrupted,
    })
    .expect("canonical recovery terminal");

    let committed = store
        .publish_receipt_backed_task_terminal(
            &key,
            expected.clone(),
            2_000,
            terminal.clone(),
            reserve_deadline(),
        )
        .expect("publish receipt-backed Task terminal");

    assert_eq!(committed.key(), &key);
    assert_eq!(committed.task().version(), promised.task().version() + 1);
    assert_eq!(committed.task().updated_at_epoch_ms(), 2_000);
    assert_eq!(committed.terminal_epoch_ms(), 2_000);
    assert_eq!(committed.terminal(), &terminal);
    assert!(!committed.cancel_requested());
    assert_eq!(
        committed.record_version().get(),
        promised.record_version().get() + 1
    );

    let generation = store.generation().expect("generation after terminal");
    assert_eq!(
        store
            .publish_receipt_backed_task_terminal(
                &key,
                expected,
                2_000,
                terminal,
                reserve_deadline(),
            )
            .expect("repeat exact terminal is idempotent"),
        committed
    );
    assert_eq!(
        store.generation().expect("generation after exact repeat"),
        generation
    );

    drop(store);
    let reopened = ReceiptLedgerStore::open(&receipts).expect("reopen receipt ledger");
    assert_eq!(
        reopened
            .recover_exact(&key, reserve_deadline())
            .expect("recover receipt-backed Task terminal"),
        ReceiptState::TaskTerminalReceiptBacked(committed)
    );
}

#[test]
fn receipt_backed_task_terminal_is_physically_reclaimed_at_its_absolute_expiry() {
    let root = tempfile::tempdir().expect("temporary root");
    let receipts = fs::canonicalize(root.path())
        .expect("physical temporary root")
        .join("receipts");
    let key = receipt_key(INVOCATION_A, TASK_A, "workspace-a");
    let store = ReceiptLedgerStore::open(&receipts).expect("open receipt ledger");
    let reserved = store
        .reserve(
            key.clone(),
            OriginalCutoffDescriptor::new(1_000, 7_000).expect("valid cutoff"),
            reserve_deadline(),
        )
        .expect("reserve exact receipt")
        .into_reservation()
        .expect("receipt remains reserved");
    let promised = store
        .promise_task_unbound(
            &key,
            reserved.record_version(),
            1_007,
            3_600_000,
            250,
            reserve_deadline(),
        )
        .expect("promise exact Task");
    let committed = store
        .publish_receipt_backed_task_terminal(
            &key,
            TaskCancellationReceipt::PromisedUnbound(promised),
            2_000,
            canonical_v5_terminal(&ReceiptTerminalOutcome::Failed {
                reason: V5SafeFailureReason::Interrupted,
            })
            .expect("canonical terminal"),
            reserve_deadline(),
        )
        .expect("publish receipt-backed Task terminal");

    assert_eq!(
        store
            .reclaim_expired_tombstones(committed.expires_at_epoch_ms() - 1, reserve_deadline(),)
            .expect("retain one millisecond before expiry"),
        0
    );
    assert!(matches!(
        store
            .recover_exact(&key, reserve_deadline())
            .expect("terminal remains before expiry"),
        ReceiptState::TaskTerminalReceiptBacked(_)
    ));
    assert_eq!(
        store
            .reclaim_expired_tombstones(committed.expires_at_epoch_ms(), reserve_deadline(),)
            .expect("reclaim at absolute expiry"),
        1
    );
    assert_eq!(
        store.recover_exact(&key, reserve_deadline()),
        Err(ReceiptLedgerError::ReceiptNotFound)
    );
    drop(store);
    let reopened = ReceiptLedgerStore::open(&receipts).expect("reopen reclaimed ledger");
    assert_eq!(
        reopened.recover_exact(&key, reserve_deadline()),
        Err(ReceiptLedgerError::ReceiptNotFound)
    );
}

#[test]
fn task_cancel_state_corruption_latches_catalog_before_second_read() {
    let root = tempfile::tempdir().expect("temporary root");
    let receipts = fs::canonicalize(root.path())
        .expect("physical temporary root")
        .join("receipts");
    let key = receipt_key(INVOCATION_A, TASK_A, "workspace-a");
    let key_digest = receipt_key_digest(&key);
    let cutoff =
        OriginalCutoffDescriptor::new(1_000, 7_000).expect("valid original response cutoff");
    let store = ReceiptLedgerStore::open(&receipts).expect("open receipt ledger");
    let reserved = store
        .reserve(key.clone(), cutoff, reserve_deadline())
        .expect("reserve exact receipt")
        .into_reservation()
        .expect("receipt remains reserved");
    let promised = store
        .promise_task_unbound(
            &key,
            reserved.record_version(),
            1_007,
            3_600_000,
            250,
            reserve_deadline(),
        )
        .expect("durably promise exact reserved Task");
    let expected = TaskCancellationReceipt::PromisedUnbound(promised.clone());

    let encoded = {
        let mut catalog = store.writer.lock().expect("retain receipt writer");
        let current = catalog
            .records
            .get(&key_digest)
            .cloned()
            .expect("promised receipt is catalogued");
        let record = StoredActiveReceiptV1 {
            schema_version: RECEIPT_RECORD_SCHEMA_VERSION,
            mutation_sequence: promised.mutation_sequence() + 1,
            record_version: promised
                .record_version()
                .checked_next()
                .expect("next witness version"),
            key: key.clone(),
            key_digest: key_digest.clone(),
            lifecycle: StoredActiveLifecycleV1::AcknowledgementCommit {
                terminal_digest: TerminalDigest::from_str(&"88".repeat(32))
                    .expect("terminal digest"),
                acknowledged_at_epoch_ms: 2_000,
                prior_record_version: promised.record_version(),
                prior_mutation_sequence: promised.mutation_sequence(),
            },
        };
        let (record, encoded) = serialize_reserved_record(record, MAX_CANCEL_RESERVED_RECORD_BYTES)
            .expect("serialize a valid transient witness");
        let replacement = CatalogEntry {
            record,
            encoded_bytes: u64::try_from(encoded.len()).expect("witness length fits u64"),
        };
        validate_catalog_replace(&catalog, &current, &replacement)
            .expect("transient witness preserves exact accounting");
        commit_catalog_replace(&mut catalog, replacement);
        encoded
    };
    fs::write(
        receipts
            .join(ACTIVE_DIRECTORY_NAME)
            .join(format!("{}.json", key_digest.as_str())),
        encoded,
    )
    .expect("persist runtime-visible transient witness");

    assert_eq!(
        store.request_task_cancel(&key, expected.clone(), reserve_deadline()),
        Err(ReceiptLedgerError::Corrupt(
            "acknowledgement commit witness is not a live receipt state"
        ))
    );
    assert_eq!(
        store.request_task_cancel(&key, expected, reserve_deadline()),
        Err(ReceiptLedgerError::StoreUnavailable)
    );
}

#[test]
fn actor_bound_promised_and_handoff_task_cancel_preserve_exact_state_and_quota() {
    let root = tempfile::tempdir().expect("temporary root");
    let receipts = fs::canonicalize(root.path())
        .expect("physical temporary root")
        .join("receipts");
    let store = ReceiptLedgerStore::open(&receipts).expect("open receipt ledger");
    let cutoff =
        OriginalCutoffDescriptor::new(1_000, 7_000).expect("valid original response cutoff");
    let workspace_identity = SafeIdentityHash::from_sha256([0x77; 32]);

    let promised_key = receipt_key(INVOCATION_A, TASK_A, "workspace-a");
    let promised_reserved = store
        .reserve(promised_key.clone(), cutoff, reserve_deadline())
        .expect("reserve promised receipt")
        .into_reservation()
        .expect("promised receipt remains reserved");
    let promised_unbound = store
        .promise_task_unbound(
            &promised_key,
            promised_reserved.record_version(),
            1_007,
            3_600_000,
            250,
            reserve_deadline(),
        )
        .expect("promise Task");
    let promised_actor_bound = store
        .bind_promised_task_actor(
            &promised_key,
            promised_unbound.record_version(),
            workspace_identity.clone(),
            reserve_deadline(),
        )
        .expect("bind promised Task actor");

    let handoff_key = receipt_key(INVOCATION_B, TASK_B, "workspace-b");
    let handoff_reserved = store
        .reserve(handoff_key.clone(), cutoff, reserve_deadline())
        .expect("reserve handoff receipt")
        .into_reservation()
        .expect("handoff receipt remains reserved");
    let handoff_actor_bound = store
        .bind_reserved_actor(
            &handoff_key,
            handoff_reserved.record_version(),
            workspace_identity,
            reserve_deadline(),
        )
        .expect("bind handoff actor");
    let handoff_begun = store
        .mark_reserved_begun(
            &handoff_key,
            handoff_actor_bound.record_version(),
            reserve_deadline(),
        )
        .expect("mark handoff begun");
    let handoff = store
        .begin_bound_task_handoff(
            &handoff_key,
            handoff_begun.record_version(),
            1_009,
            3_600_000,
            250,
            reserve_deadline(),
        )
        .expect("persist begun handoff");

    let promised_expected = TaskCancellationReceipt::PromisedActorBound(promised_actor_bound);
    let promised_cancelled = store
        .request_task_cancel(&promised_key, promised_expected.clone(), reserve_deadline())
        .expect("cancel actor-bound promised Task");
    assert!(promised_cancelled.is_exact_cancel_successor_of(&promised_expected));

    let handoff_expected = TaskCancellationReceipt::HandoffActorBound(handoff.clone());
    let handoff_cancelled = store
        .request_task_cancel(&handoff_key, handoff_expected.clone(), reserve_deadline())
        .expect("cancel actor-bound handoff Task");
    assert!(handoff_cancelled.is_exact_cancel_successor_of(&handoff_expected));
    let TaskCancellationReceipt::HandoffActorBound(cancelled_handoff) = &handoff_cancelled else {
        panic!("handoff cancellation changed state kind");
    };
    assert_eq!(cancelled_handoff.phase(), handoff.phase());
    assert_eq!(cancelled_handoff.link(), handoff.link());
    assert_eq!(cancelled_handoff.task(), handoff.task());
    assert_eq!(cancelled_handoff.terminal_stage(), handoff.terminal_stage());

    let generation_before_mismatch = store.generation().expect("generation before mismatch");
    assert_eq!(
        store.request_task_cancel(&promised_key, handoff_expected, reserve_deadline(),),
        Err(ReceiptLedgerError::TaskCancellationMismatch)
    );
    assert_eq!(
        store.generation().expect("generation after mismatch"),
        generation_before_mismatch
    );

    drop(store);
    let reopened = ReceiptLedgerStore::open(&receipts).expect("reopen receipt ledger");
    assert_eq!(
        reopened
            .recover_exact(&promised_key, reserve_deadline())
            .expect("recover promised cancellation"),
        promised_cancelled.into_receipt_state()
    );
    assert_eq!(
        reopened
            .recover_exact(&handoff_key, reserve_deadline())
            .expect("recover handoff cancellation"),
        handoff_cancelled.into_receipt_state()
    );
}

#[test]
fn confirmed_task_bound_completes_handoff_and_releases_receipt_ownership() {
    let root = tempfile::tempdir().expect("temporary root");
    let receipts = fs::canonicalize(root.path())
        .expect("physical temporary root")
        .join("receipts");
    let key = receipt_key(INVOCATION_A, TASK_A, "workspace-a");
    let key_digest = receipt_key_digest(&key);
    let cutoff =
        OriginalCutoffDescriptor::new(1_000, 7_000).expect("valid original response cutoff");
    let store = ReceiptLedgerStore::open(&receipts).expect("open receipt ledger");
    let reserved = store
        .reserve(key.clone(), cutoff, reserve_deadline())
        .expect("reserve exact receipt")
        .into_reservation()
        .expect("receipt remains reserved");
    let actor_bound = store
        .bind_reserved_actor(
            &key,
            reserved.record_version(),
            SafeIdentityHash::from_sha256([0x77; 32]),
            reserve_deadline(),
        )
        .expect("bind exact actor");
    let begun = store
        .mark_reserved_begun(&key, actor_bound.record_version(), reserve_deadline())
        .expect("mark exact attempt begun");
    let handoff = store
        .begin_bound_task_handoff(
            &key,
            begun.record_version(),
            1_009,
            3_600_000,
            250,
            reserve_deadline(),
        )
        .expect("persist begun Task handoff intent");
    let task_bound = confirmed_task_bound(&handoff);
    let row = receipts
        .join(ACTIVE_DIRECTORY_NAME)
        .join(format!("{}.json", key_digest.as_str()));

    let completed = store
        .complete_bound_task_handoff(
            &key,
            handoff.record_version(),
            task_bound.clone(),
            reserve_deadline(),
        )
        .expect("retire exact receipt after confirmed TaskBound");

    assert_eq!(completed, task_bound);
    assert_eq!(completed.phase(), AttemptPhase::Begun);
    assert_eq!(completed.link(), handoff.link());
    assert_eq!(store.generation().expect("completion generation"), 5);
    assert_eq!(
        store.recover_exact(&key, reserve_deadline()),
        Err(ReceiptLedgerError::ReceiptNotFound)
    );
    {
        let catalog = store.writer.lock().expect("inspect completed catalog");
        assert!(catalog.records.is_empty());
        assert!(catalog.invocation_index.is_empty());
        assert!(catalog.reserved_task_index.is_empty());
        assert_eq!(catalog.actual_bytes, 0);
        assert_eq!(catalog.reserved_result_bytes, 0);
        assert_eq!(catalog.tombstone_bytes, 0);
    }
    assert!(!row.exists(), "completion removes the receipt witness row");

    drop(store);
    let reopened = ReceiptLedgerStore::open(&receipts).expect("reopen completed ledger");
    assert_eq!(reopened.generation().expect("reopened generation"), 5);
    assert_eq!(
        reopened.recover_exact(&key, reserve_deadline()),
        Err(ReceiptLedgerError::ReceiptNotFound),
        "reopen cannot resurrect the completed handoff receipt"
    );
}

#[test]
fn confirmed_task_bound_completes_actor_bound_promise() {
    let root = tempfile::tempdir().expect("temporary root");
    let receipts = fs::canonicalize(root.path())
        .expect("physical temporary root")
        .join("receipts");
    let key = receipt_key(INVOCATION_A, TASK_A, "workspace-a");
    let store = ReceiptLedgerStore::open(&receipts).expect("open receipt ledger");
    let reserved = store
        .reserve(
            key.clone(),
            OriginalCutoffDescriptor::new(1_000, 7_000).expect("valid cutoff"),
            reserve_deadline(),
        )
        .expect("reserve exact receipt")
        .into_reservation()
        .expect("receipt remains reserved");
    let promised = store
        .promise_task_unbound(
            &key,
            reserved.record_version(),
            1_009,
            3_600_000,
            250,
            reserve_deadline(),
        )
        .expect("promise exact Task");
    let actor_bound = store
        .bind_promised_task_actor(
            &key,
            promised.record_version(),
            SafeIdentityHash::from_sha256([0x77; 32]),
            reserve_deadline(),
        )
        .expect("bind promised Task actor");
    let task_bound = confirmed_promised_task_bound(&actor_bound);

    assert_eq!(
        store
            .complete_bound_task_handoff(
                &key,
                actor_bound.record_version(),
                task_bound.clone(),
                reserve_deadline(),
            )
            .expect("retire actor-bound promise after confirmed TaskBound"),
        task_bound
    );
    assert_eq!(
        store.recover_exact(&key, reserve_deadline()),
        Err(ReceiptLedgerError::ReceiptNotFound)
    );
}

#[test]
fn mismatched_task_bound_cannot_mutate_handoff_receipt() {
    let root = tempfile::tempdir().expect("temporary root");
    let receipts = fs::canonicalize(root.path())
        .expect("physical temporary root")
        .join("receipts");
    let key = receipt_key(INVOCATION_A, TASK_A, "workspace-a");
    let key_digest = receipt_key_digest(&key);
    let store = ReceiptLedgerStore::open(&receipts).expect("open receipt ledger");
    let reserved = store
        .reserve(
            key.clone(),
            OriginalCutoffDescriptor::new(1_000, 7_000).expect("valid cutoff"),
            reserve_deadline(),
        )
        .expect("reserve exact receipt")
        .into_reservation()
        .expect("receipt remains reserved");
    let actor_bound = store
        .bind_reserved_actor(
            &key,
            reserved.record_version(),
            SafeIdentityHash::from_sha256([0x77; 32]),
            reserve_deadline(),
        )
        .expect("bind exact actor");
    let handoff = store
        .begin_bound_task_handoff(
            &key,
            actor_bound.record_version(),
            1_009,
            3_600_000,
            250,
            reserve_deadline(),
        )
        .expect("persist not-begun Task handoff intent");
    let mismatched_link = TaskLinkReference::new(
        key_digest.clone(),
        key.reserved_task_id(),
        key.invocation_id(),
        SafeIdentityHash::from_sha256([0x78; 32]),
    );
    let mismatched = TaskBoundReceipt::new(
        LifecycleLinkRecordHeader::new(key.clone(), mismatched_link, 2, 1, 512)
            .expect("valid alternate lifecycle-link header"),
        handoff.task().clone(),
        handoff.task().version(),
        handoff.task().created_at_epoch_ms() + 1,
        handoff.phase(),
    )
    .expect("structurally valid but non-matching TaskBound");
    let row = receipts
        .join(ACTIVE_DIRECTORY_NAME)
        .join(format!("{}.json", key_digest.as_str()));
    let row_before = fs::read(&row).expect("read handoff row");
    let generation_before = store.generation().expect("handoff generation");

    assert_eq!(
        store.complete_bound_task_handoff(
            &key,
            ReceiptVersion::new(handoff.record_version().get() - 1)
                .expect("prior receipt version is nonzero"),
            confirmed_task_bound(&handoff),
            reserve_deadline(),
        ),
        Err(ReceiptLedgerError::ReceiptVersionMismatch {
            expected: ReceiptVersion::new(handoff.record_version().get() - 1)
                .expect("prior receipt version is nonzero"),
            actual: handoff.record_version(),
        })
    );
    let mismatched_phase = TaskBoundReceipt::new(
        LifecycleLinkRecordHeader::new(key.clone(), handoff.link().clone(), 2, 1, 512)
            .expect("valid phase-mismatch lifecycle-link header"),
        handoff.task().clone(),
        handoff.task().version(),
        handoff.task().created_at_epoch_ms() + 1,
        AttemptPhase::Begun,
    )
    .expect("structurally valid but phase-mismatched TaskBound");
    assert_eq!(
        store.complete_bound_task_handoff(
            &key,
            handoff.record_version(),
            mismatched_phase,
            reserve_deadline(),
        ),
        Err(ReceiptLedgerError::TaskBoundMismatch)
    );

    assert_eq!(
        store.complete_bound_task_handoff(
            &key,
            handoff.record_version(),
            mismatched,
            reserve_deadline(),
        ),
        Err(ReceiptLedgerError::TaskBoundMismatch)
    );
    assert_eq!(
        store.generation().expect("unchanged generation"),
        generation_before
    );
    assert_eq!(fs::read(&row).expect("unchanged handoff row"), row_before);
    assert_eq!(
        store.recover_exact(&key, reserve_deadline()),
        Ok(ReceiptState::TaskHandoffActorBound(handoff))
    );
}

#[test]
fn handoff_completion_reopen_heals_generation_after_visible_deletion_witness() {
    let root = tempfile::tempdir().expect("temporary root");
    let receipts = fs::canonicalize(root.path())
        .expect("physical temporary root")
        .join("receipts");
    let key = receipt_key(INVOCATION_A, TASK_A, "workspace-a");
    let store = ReceiptLedgerStore::open(&receipts).expect("open receipt ledger");
    let reserved = store
        .reserve(
            key.clone(),
            OriginalCutoffDescriptor::new(1_000, 7_000).expect("valid cutoff"),
            reserve_deadline(),
        )
        .expect("reserve exact receipt")
        .into_reservation()
        .expect("receipt remains reserved");
    let actor_bound = store
        .bind_reserved_actor(
            &key,
            reserved.record_version(),
            SafeIdentityHash::from_sha256([0x77; 32]),
            reserve_deadline(),
        )
        .expect("bind exact actor");
    let handoff = store
        .begin_bound_task_handoff(
            &key,
            actor_bound.record_version(),
            1_009,
            3_600_000,
            250,
            reserve_deadline(),
        )
        .expect("persist not-begun Task handoff intent");
    let task_bound = confirmed_task_bound(&handoff);
    set_after_receipt_row_rename_hook_for_test(|| {
        panic!("simulate process loss after visible handoff deletion witness")
    });

    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        store.complete_bound_task_handoff(
            &key,
            handoff.record_version(),
            task_bound,
            reserve_deadline(),
        )
    }))
    .is_err());
    drop(store);

    let reopened = ReceiptLedgerStore::open(&receipts)
        .expect("reopen heals generation and removes deletion witness");
    assert_eq!(reopened.generation().expect("healed generation"), 4);
    assert_eq!(
        reopened.recover_exact(&key, reserve_deadline()),
        Err(ReceiptLedgerError::ReceiptNotFound)
    );
}

#[test]
fn actor_bound_cancel_is_durable_and_prevents_begun_transition() {
    let root = tempfile::tempdir().expect("temporary root");
    let receipts = fs::canonicalize(root.path())
        .expect("physical temporary root")
        .join("receipts");
    let key = receipt_key(INVOCATION_A, TASK_A, "workspace-a");
    let cutoff =
        OriginalCutoffDescriptor::new(1_000, 7_000).expect("valid original response cutoff");
    let store = ReceiptLedgerStore::open(&receipts).expect("open receipt ledger");
    let reserved = store
        .reserve(key.clone(), cutoff, reserve_deadline())
        .expect("reserve exact receipt")
        .into_reservation()
        .expect("receipt remains reserved");
    let actor_bound = store
        .bind_reserved_actor(
            &key,
            reserved.record_version(),
            SafeIdentityHash::from_sha256([0x77; 32]),
            reserve_deadline(),
        )
        .expect("bind exact actor");
    assert_eq!(actor_bound.record_version().get(), 2);

    let CancelResolution::ExistingWinner(cancelled) = store
        .request_cancel_or_reserve(key.clone(), 1_001, reserve_deadline())
        .expect("commit actor-bound cancellation")
    else {
        panic!("existing actor-bound receipt must remain the cancellation owner");
    };
    let ReceiptState::Reserved(cancelled) = *cancelled else {
        panic!("actor-bound cancellation changed receipt family");
    };
    assert!(cancelled.cancel_requested());
    assert_eq!(cancelled.record_version().get(), 3);
    assert_eq!(cancelled.mutation_sequence(), 3);
    assert!(matches!(
        store.mark_reserved_begun(&key, cancelled.record_version(), reserve_deadline()),
        Err(ReceiptLedgerError::ReceiptRowPresentUnsupported)
    ));

    let reopened = match ReceiptLedgerStore::open(&receipts) {
        Ok(_) => panic!("the first writer still owns the ledger"),
        Err(error) => error,
    };
    assert_eq!(reopened, ReceiptLedgerError::AlreadyOwned);
    drop(store);
    let reopened = ReceiptLedgerStore::open(&receipts).expect("reopen receipt ledger");
    let ReceiptState::Reserved(recovered) = reopened
        .recover_exact(&key, reserve_deadline())
        .expect("recover cancelled actor-bound receipt")
    else {
        panic!("reopened cancellation changed receipt family");
    };
    assert_eq!(recovered, cancelled);
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
fn direct_ack_compacts_payload_to_restart_stable_idempotent_tombstone() {
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
        result: Box::new(DomainResult::success("direct result to compact")),
    })
    .expect("canonical direct terminal");
    let terminal_digest = terminal.digest().clone();
    let committed = store
        .publish_direct_terminal(
            &key,
            reserved.record_version(),
            2_000,
            terminal,
            reserve_deadline(),
        )
        .expect("publish exact direct terminal");

    let acknowledged = store
        .acknowledge_direct(&key, &terminal_digest, 2_100, reserve_deadline())
        .expect("acknowledge committed direct terminal");
    assert_eq!(acknowledged.key(), &key);
    assert_eq!(acknowledged.terminal_digest(), &terminal_digest);
    assert_eq!(acknowledged.acknowledged_at_epoch_ms(), 2_100);
    assert_eq!(acknowledged.expires_at_epoch_ms(), 902_100);
    assert!(acknowledged.encoded_bytes() <= MAX_ACKNOWLEDGED_TOMBSTONE_BYTES);
    assert_eq!(store.generation().expect("generation after ack"), 3);
    {
        let catalog = store.writer.lock().expect("inspect acknowledged catalog");
        assert_eq!(catalog.live_count(), 0);
        assert_eq!(catalog.actual_bytes, 0);
        assert_eq!(catalog.reserved_result_bytes, 0);
        assert_eq!(catalog.tombstone_count(), 1);
        assert_eq!(catalog.tombstone_bytes, acknowledged.encoded_bytes());
    }
    drop(store);

    let reopened = ReceiptLedgerStore::open(&receipts).expect("reopen receipt ledger");
    let recovered = reopened
        .recover_exact(&key, reserve_deadline())
        .expect("recover exact acknowledged tombstone");
    assert_eq!(
        recovered,
        ReceiptState::AcknowledgedTombstone(acknowledged.clone())
    );
    let duplicate = reopened
        .acknowledge_direct(&key, &terminal_digest, 9_999, reserve_deadline())
        .expect("repeat exact acknowledgement");
    assert_eq!(duplicate, acknowledged);
    assert_eq!(
        reopened
            .generation()
            .expect("generation after duplicate ack"),
        3,
        "duplicate ACK must not rewrite first-ACK epoch or generation"
    );
    assert!(committed.encoded_bytes() > acknowledged.encoded_bytes());
}

#[test]
fn compact_tombstone_fits_512_bytes_at_the_maximum_valid_epoch_and_longest_tool_name() {
    let key = ReceiptKey::new(
        InvocationId::from_str(INVOCATION_A).expect("canonical invocation id"),
        TaskId::from_str(TASK_A).expect("canonical task id"),
        RequestIdentity::new(
            CoreIdentityDigest::from_sha256([0x55; 32]),
            V5ToolIdentity::Search,
            normalized_arguments_hash(&serde_json::Map::new()),
            request_scope_hash("workspace-a").expect("bounded request scope"),
        ),
    );
    let key_digest = receipt_key_digest(&key);
    let acknowledged_at_epoch_ms = u64::MAX
        .checked_sub(ACKNOWLEDGED_TOMBSTONE_TTL_MS)
        .expect("bounded epoch");
    let terminal_digest = canonical_v5_terminal(&ReceiptTerminalOutcome::Cancelled)
        .expect("canonical terminal")
        .digest()
        .clone();
    let record = StoredActiveReceiptV1 {
        schema_version: RECEIPT_RECORD_SCHEMA_VERSION,
        mutation_sequence: 0,
        record_version: ReceiptVersion::new(3).expect("tombstone record version"),
        key,
        key_digest: key_digest.clone(),
        lifecycle: StoredActiveLifecycleV1::AcknowledgedTombstone {
            terminal_digest,
            acknowledged_at_epoch_ms,
        },
    };

    let (_, encoded) = serialize_reserved_record(record, MAX_ACKNOWLEDGED_TOMBSTONE_BYTES)
        .expect("worst-case compact tombstone fits its contract");
    assert!(encoded.len() <= MAX_ACKNOWLEDGED_TOMBSTONE_BYTES as usize);
    let text = std::str::from_utf8(&encoded).expect("compact tombstone JSON is UTF-8");
    assert!(text.starts_with("{\"k\":"));
    assert!(text.contains("\"d\":"));
    assert!(text.contains("\"a\":18446744073708651615"));
    let mut persisted = tempfile::tempfile().expect("temporary file");
    persisted
        .write_all(&encoded)
        .and_then(|()| persisted.sync_all())
        .expect("persist compact tombstone fixture");
    let decoded = read_active_record_from_retained(&mut persisted, &key_digest)
        .expect("strict decoder accepts the worst-case compact tombstone");
    assert!(matches!(
        decoded.state(),
        Ok(ReceiptState::AcknowledgedTombstone(_))
    ));
}

#[test]
fn ack_crash_after_witness_row_before_generation_heals_and_compacts_on_reopen() {
    let root = tempfile::tempdir().expect("temporary root");
    let receipts = fs::canonicalize(root.path())
        .expect("physical temporary root")
        .join("receipts");
    let (store, key, terminal_digest) = direct_terminal_fixture(&receipts);
    set_after_receipt_row_rename_hook_for_test(|| panic!("simulated process crash"));

    let crashed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        store
            .acknowledge_direct(&key, &terminal_digest, 2_100, reserve_deadline())
            .expect("crash hook interrupts ACK")
    }));
    assert!(crashed.is_err());
    drop(store);
    assert_eq!(
        fs::read(receipts.join(GENERATION_FILE_NAME)).expect("read stale generation"),
        b"2\n"
    );

    let reopened = ReceiptLedgerStore::open(&receipts)
        .expect("reopen heals the durable acknowledgement witness");
    assert_eq!(reopened.generation().expect("healed generation"), 3);
    assert!(matches!(
        reopened.recover_exact(&key, reserve_deadline()),
        Ok(ReceiptState::AcknowledgedTombstone(_))
    ));
}

#[test]
fn ack_generation_is_published_while_the_durable_witness_is_still_visible() {
    let root = tempfile::tempdir().expect("temporary root");
    let receipts = fs::canonicalize(root.path())
        .expect("physical temporary root")
        .join("receipts");
    let (store, key, terminal_digest) = direct_terminal_fixture(&receipts);
    let row_path = receipts
        .join(ACTIVE_DIRECTORY_NAME)
        .join(format!("{}.json", receipt_key_digest(&key).as_str()));
    let observed = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    let hook_observed = std::sync::Arc::clone(&observed);
    set_after_generation_replace_hook_for_test(move || {
        *hook_observed.lock().expect("record observed ACK row") =
            fs::read_to_string(&row_path).expect("read ACK row at generation publication");
        panic!("simulated process crash");
    });

    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        store
            .acknowledge_direct(&key, &terminal_digest, 2_100, reserve_deadline())
            .expect("generation crash hook interrupts ACK")
    }))
    .is_err());
    assert!(
        observed
            .lock()
            .expect("inspect observed ACK row")
            .contains("\"state\":\"acknowledgement_commit\""),
        "generation must not become authoritative while only a sequence-free tombstone is visible"
    );
    drop(store);

    let reopened =
        ReceiptLedgerStore::open(&receipts).expect("reopen finalizes the acknowledged witness");
    assert_eq!(reopened.generation().expect("published generation"), 3);
    assert!(matches!(
        reopened.recover_exact(&key, reserve_deadline()),
        Ok(ReceiptState::AcknowledgedTombstone(_))
    ));
}

#[test]
fn ack_crash_after_compact_row_rename_reopens_the_same_tombstone() {
    let root = tempfile::tempdir().expect("temporary root");
    let receipts = fs::canonicalize(root.path())
        .expect("physical temporary root")
        .join("receipts");
    let (store, key, terminal_digest) = direct_terminal_fixture(&receipts);
    set_after_generation_replace_hook_for_test(|| {
        set_after_receipt_row_rename_hook_for_test(|| panic!("simulated process crash"));
    });

    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        store
            .acknowledge_direct(&key, &terminal_digest, 2_100, reserve_deadline())
            .expect("compact crash hook interrupts ACK")
    }))
    .is_err());
    drop(store);

    let reopened = ReceiptLedgerStore::open(&receipts)
        .expect("reopen accepts the compact row after generation commit");
    assert_eq!(reopened.generation().expect("published generation"), 3);
    let recovered = reopened
        .recover_exact(&key, reserve_deadline())
        .expect("recover compact acknowledged tombstone");
    let ReceiptState::AcknowledgedTombstone(tombstone) = recovered else {
        panic!("ACK crash reopened as a non-tombstone lifecycle")
    };
    assert_eq!(tombstone.terminal_digest(), &terminal_digest);
    assert_eq!(tombstone.acknowledged_at_epoch_ms(), 2_100);
}

#[test]
fn reopen_rejects_an_acknowledgement_witness_that_skips_generation() {
    let root = tempfile::tempdir().expect("temporary root");
    let receipts = fs::canonicalize(root.path())
        .expect("physical temporary root")
        .join("receipts");
    let (store, key, terminal_digest) = direct_terminal_fixture(&receipts);
    let key_digest = receipt_key_digest(&key);
    let row_path = receipts
        .join(ACTIVE_DIRECTORY_NAME)
        .join(format!("{}.json", key_digest.as_str()));
    let encoded = fs::read(&row_path).expect("read direct predecessor");
    let record: StoredActiveReceiptV1 =
        serde_json::from_slice(&encoded).expect("decode direct predecessor");
    let predecessor = CatalogEntry {
        record,
        encoded_bytes: u64::try_from(encoded.len()).expect("bounded predecessor bytes"),
    };
    let witness = build_acknowledgement_commit_record(&predecessor, terminal_digest, 2_100, 4)
        .expect("build forged ahead witness");
    let (_, witness_encoded) = serialize_reserved_record(witness, MAX_CANCEL_RESERVED_RECORD_BYTES)
        .expect("encode forged ahead witness");
    drop(store);
    fs::write(&row_path, witness_encoded).expect("persist forged ahead witness");

    assert_eq!(
        ReceiptLedgerStore::open(&receipts)
            .err()
            .expect("witness may only be current or next generation"),
        ReceiptLedgerError::Corrupt(
            "pending receipt mutation witness is not the next persisted mutation"
        )
    );
}

#[test]
fn acknowledged_tombstone_is_physically_reclaimed_at_its_absolute_expiry() {
    let root = tempfile::tempdir().expect("temporary root");
    let receipts = fs::canonicalize(root.path())
        .expect("physical temporary root")
        .join("receipts");
    let store = ReceiptLedgerStore::open(&receipts).expect("open receipt ledger");
    let key = receipt_key(INVOCATION_A, TASK_A, "workspace-a");
    let reserved = store
        .reserve(
            key.clone(),
            OriginalCutoffDescriptor::new(1_000, 7_000).expect("valid cutoff"),
            reserve_deadline(),
        )
        .expect("reserve exact receipt")
        .into_reservation()
        .expect("receipt remains reserved");
    let terminal = canonical_v5_terminal(&ReceiptTerminalOutcome::Cancelled)
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
        .expect("publish direct terminal");
    store
        .acknowledge_direct(&key, &terminal_digest, 2_100, reserve_deadline())
        .expect("acknowledge direct terminal");

    assert_eq!(
        store
            .reclaim_expired_tombstones(902_099, reserve_deadline())
            .expect("inspect one millisecond before expiry"),
        0
    );
    assert!(matches!(
        store.recover_exact(&key, reserve_deadline()),
        Ok(ReceiptState::AcknowledgedTombstone(_))
    ));
    assert_eq!(
        store
            .reclaim_expired_tombstones(902_100, reserve_deadline())
            .expect("reclaim at absolute expiry"),
        1
    );
    assert_eq!(
        store.recover_exact(&key, reserve_deadline()),
        Err(ReceiptLedgerError::ReceiptNotFound)
    );
    assert_eq!(store.generation().expect("generation after expiry"), 4);
    drop(store);

    let reopened = ReceiptLedgerStore::open(&receipts).expect("reopen after expiry");
    assert_eq!(
        reopened.recover_exact(&key, reserve_deadline()),
        Err(ReceiptLedgerError::ReceiptNotFound)
    );
}

#[test]
fn exact_cancel_request_reclaims_an_expired_tombstone_before_reserving_again() {
    let root = tempfile::tempdir().expect("temporary root");
    let receipts = fs::canonicalize(root.path())
        .expect("physical temporary root")
        .join("receipts");
    let store = ReceiptLedgerStore::open(&receipts).expect("open receipt ledger");
    let key = receipt_key(INVOCATION_A, TASK_A, "workspace-a");
    let reserved = store
        .reserve(
            key.clone(),
            OriginalCutoffDescriptor::new(1_000, 7_000).expect("valid cutoff"),
            reserve_deadline(),
        )
        .expect("reserve exact receipt")
        .into_reservation()
        .expect("receipt remains reserved");
    let terminal = canonical_v5_terminal(&ReceiptTerminalOutcome::Cancelled)
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
        .expect("publish direct terminal");
    let tombstone = store
        .acknowledge_direct(&key, &terminal_digest, 2_100, reserve_deadline())
        .expect("acknowledge direct terminal");

    assert!(matches!(
        store
            .request_cancel_or_reserve(
                key.clone(),
                tombstone.expires_at_epoch_ms() - 1,
                reserve_deadline(),
            )
            .expect("pre-expiry exact request returns the winner"),
        CancelResolution::ExistingWinner(_)
    ));
    let replacement = store
        .request_cancel_or_reserve(
            key.clone(),
            tombstone.expires_at_epoch_ms(),
            reserve_deadline(),
        )
        .expect("expiry boundary releases the exact key");
    let CancelResolution::NewlyReserved(replacement) = replacement else {
        panic!("expired exact tombstone did not yield a new cancellation reservation")
    };
    assert_eq!(replacement.key(), &key);
    assert_eq!(
        store.generation().expect("reclaim plus reserve generation"),
        5
    );
}

#[test]
fn exact_ack_reclaims_its_tombstone_at_expiry_and_reports_absence() {
    let root = tempfile::tempdir().expect("temporary root");
    let receipts = fs::canonicalize(root.path())
        .expect("physical temporary root")
        .join("receipts");
    let store = ReceiptLedgerStore::open(&receipts).expect("open receipt ledger");
    let key = receipt_key(INVOCATION_A, TASK_A, "workspace-a");
    let reserved = store
        .reserve(
            key.clone(),
            OriginalCutoffDescriptor::new(1_000, 7_000).expect("valid cutoff"),
            reserve_deadline(),
        )
        .expect("reserve exact receipt")
        .into_reservation()
        .expect("receipt remains reserved");
    let terminal = canonical_v5_terminal(&ReceiptTerminalOutcome::Cancelled)
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
        .expect("publish direct terminal");
    let tombstone = store
        .acknowledge_direct(&key, &terminal_digest, 2_100, reserve_deadline())
        .expect("acknowledge direct terminal");

    assert_eq!(
        store
            .acknowledge_direct(
                &key,
                &terminal_digest,
                tombstone.expires_at_epoch_ms() - 1,
                reserve_deadline(),
            )
            .expect("pre-expiry retry is idempotent"),
        tombstone
    );
    assert_eq!(
        store.acknowledge_direct(
            &key,
            &terminal_digest,
            tombstone.expires_at_epoch_ms(),
            reserve_deadline(),
        ),
        Err(ReceiptLedgerError::ReceiptNotFound)
    );
    assert_eq!(store.generation().expect("expiry generation"), 4);
}

#[test]
fn exact_recovery_reclaims_its_tombstone_at_expiry_and_reports_absence() {
    let root = tempfile::tempdir().expect("temporary root");
    let receipts = fs::canonicalize(root.path())
        .expect("physical temporary root")
        .join("receipts");
    let mut store = ReceiptLedgerStore::open(&receipts).expect("open receipt ledger");
    let key = receipt_key(INVOCATION_A, TASK_A, "workspace-a");
    let reserved = store
        .reserve(
            key.clone(),
            OriginalCutoffDescriptor::new(1_000, 7_000).expect("valid cutoff"),
            reserve_deadline(),
        )
        .expect("reserve exact receipt")
        .into_reservation()
        .expect("receipt remains reserved");
    let terminal = canonical_v5_terminal(&ReceiptTerminalOutcome::Cancelled)
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
        .expect("publish direct terminal");
    let tombstone = store
        .acknowledge_direct(&key, &terminal_digest, 2_100, reserve_deadline())
        .expect("acknowledge direct terminal");

    assert_eq!(
        ReceiptLedgerPort::recover_at(
            &mut store,
            &key,
            tombstone.expires_at_epoch_ms() - 1,
            reserve_deadline(),
        ),
        Ok(ReceiptState::AcknowledgedTombstone(tombstone.clone()))
    );
    assert_eq!(
        ReceiptLedgerPort::recover_at(
            &mut store,
            &key,
            tombstone.expires_at_epoch_ms(),
            reserve_deadline(),
        ),
        Err(ReceiptLedgerError::ReceiptNotFound)
    );
    assert_eq!(store.generation().expect("expiry generation"), 4);
}

#[test]
fn all_exact_tombstone_paths_remain_absent_after_the_expiry_boundary() {
    let root = tempfile::tempdir().expect("temporary root");
    let receipts = fs::canonicalize(root.path())
        .expect("physical temporary root")
        .join("cancel-receipts");
    let (store, key, terminal_digest) = direct_terminal_fixture(&receipts);
    let tombstone = store
        .acknowledge_direct(&key, &terminal_digest, 2_100, reserve_deadline())
        .expect("acknowledge cancel fixture");
    assert!(matches!(
        store
            .request_cancel_or_reserve(
                key,
                tombstone.expires_at_epoch_ms() + 1,
                reserve_deadline(),
            )
            .expect("post-expiry cancel reserves a new receipt"),
        CancelResolution::NewlyReserved(_)
    ));

    let receipts = fs::canonicalize(root.path())
        .expect("physical temporary root")
        .join("ack-receipts");
    let (store, key, terminal_digest) = direct_terminal_fixture(&receipts);
    let tombstone = store
        .acknowledge_direct(&key, &terminal_digest, 2_100, reserve_deadline())
        .expect("acknowledge ACK fixture");
    assert_eq!(
        store.acknowledge_direct(
            &key,
            &terminal_digest,
            tombstone.expires_at_epoch_ms() + 1,
            reserve_deadline(),
        ),
        Err(ReceiptLedgerError::ReceiptNotFound)
    );

    let receipts = fs::canonicalize(root.path())
        .expect("physical temporary root")
        .join("recover-receipts");
    let (mut store, key, terminal_digest) = direct_terminal_fixture(&receipts);
    let tombstone = store
        .acknowledge_direct(&key, &terminal_digest, 2_100, reserve_deadline())
        .expect("acknowledge recovery fixture");
    assert_eq!(
        ReceiptLedgerPort::recover_at(
            &mut store,
            &key,
            tombstone.expires_at_epoch_ms() + 1,
            reserve_deadline(),
        ),
        Err(ReceiptLedgerError::ReceiptNotFound)
    );
}

#[test]
fn expired_tombstone_releases_partial_identity_for_new_admission_only_at_expiry() {
    let root = tempfile::tempdir().expect("temporary root");
    let receipts = fs::canonicalize(root.path())
        .expect("physical temporary root")
        .join("receipts");
    let store = ReceiptLedgerStore::open(&receipts).expect("open receipt ledger");
    let original = receipt_key(INVOCATION_A, TASK_A, "workspace-a");
    let reserved = store
        .reserve(
            original.clone(),
            OriginalCutoffDescriptor::new(1_000, 7_000).expect("valid cutoff"),
            reserve_deadline(),
        )
        .expect("reserve original receipt")
        .into_reservation()
        .expect("original remains reserved");
    let terminal =
        canonical_v5_terminal(&ReceiptTerminalOutcome::Cancelled).expect("canonical terminal");
    let terminal_digest = terminal.digest().clone();
    store
        .publish_direct_terminal(
            &original,
            reserved.record_version(),
            2_000,
            terminal,
            reserve_deadline(),
        )
        .expect("publish original direct terminal");
    store
        .acknowledge_direct(&original, &terminal_digest, 2_100, reserve_deadline())
        .expect("acknowledge original direct terminal");
    let replacement = receipt_key(INVOCATION_A, TASK_B, "workspace-b");

    assert_eq!(
        store.reserve(
            replacement.clone(),
            OriginalCutoffDescriptor::new(902_099, 7_000).expect("pre-expiry cutoff"),
            reserve_deadline(),
        ),
        Err(ReceiptLedgerError::InvocationIdentityMismatch)
    );
    assert_eq!(store.generation().expect("pre-expiry generation"), 3);

    let admitted = store
        .reserve(
            replacement.clone(),
            OriginalCutoffDescriptor::new(902_100, 7_000).expect("expiry cutoff"),
            reserve_deadline(),
        )
        .expect("expired identity is reusable")
        .into_reservation()
        .expect("replacement is newly reserved");
    assert_eq!(admitted.key(), &replacement);
    assert_eq!(store.generation().expect("replacement generation"), 5);
    assert_eq!(
        store.recover_exact(&original, reserve_deadline()),
        Err(ReceiptLedgerError::InvocationIdentityMismatch)
    );
    assert!(!receipts
        .join(ACTIVE_DIRECTORY_NAME)
        .join(format!("{}.json", receipt_key_digest(&original).as_str()))
        .exists());
}

#[test]
fn tombstone_pool_does_not_consume_the_sixty_four_live_receipt_slots() {
    let root = tempfile::tempdir().expect("temporary root");
    let receipts = fs::canonicalize(root.path())
        .expect("physical temporary root")
        .join("receipts");
    let store = ReceiptLedgerStore::open(&receipts).expect("open receipt ledger");
    let terminal_digest = canonical_v5_terminal(&ReceiptTerminalOutcome::Cancelled)
        .expect("canonical terminal")
        .digest()
        .clone();
    let mut catalog = store.writer.lock().expect("inspect receipt catalog");
    for _ in 0..65 {
        let key = receipt_key_with_ids(InvocationId::new(), TaskId::new(), "workspace-a");
        let key_digest = receipt_key_digest(&key);
        insert_catalog_entry(
            &mut catalog,
            CatalogEntry {
                record: StoredActiveReceiptV1 {
                    schema_version: RECEIPT_RECORD_SCHEMA_VERSION,
                    mutation_sequence: 0,
                    record_version: ReceiptVersion::new(3).expect("tombstone marker version"),
                    key,
                    key_digest,
                    lifecycle: StoredActiveLifecycleV1::AcknowledgedTombstone {
                        terminal_digest: terminal_digest.clone(),
                        acknowledged_at_epoch_ms: 1_000,
                    },
                },
                encoded_bytes: 256,
            },
            false,
        )
        .expect("insert synthetic tombstone telemetry fixture");
    }
    assert_eq!(catalog.live_count(), 0);
    assert_eq!(catalog.tombstone_count(), 65);
    let fresh = receipt_key_with_ids(InvocationId::new(), TaskId::new(), "workspace-fresh");

    store
        .prepare_new_admission_under_writer_lock(&mut catalog, &fresh, 1_001, reserve_deadline())
        .expect("separate tombstone pool cannot block live admission");
}

#[test]
fn rejected_ack_does_not_reclaim_an_unrelated_expired_tombstone() {
    let root = tempfile::tempdir().expect("temporary root");
    let receipts = fs::canonicalize(root.path())
        .expect("physical temporary root")
        .join("receipts");
    let store = ReceiptLedgerStore::open(&receipts).expect("open receipt ledger");
    let tombstone_key = receipt_key(INVOCATION_A, TASK_A, "workspace-a");
    let reserved = store
        .reserve(
            tombstone_key.clone(),
            OriginalCutoffDescriptor::new(1_000, 7_000).expect("valid cutoff"),
            reserve_deadline(),
        )
        .expect("reserve original receipt")
        .into_reservation()
        .expect("original remains reserved");
    let terminal =
        canonical_v5_terminal(&ReceiptTerminalOutcome::Cancelled).expect("canonical terminal");
    let terminal_digest = terminal.digest().clone();
    store
        .publish_direct_terminal(
            &tombstone_key,
            reserved.record_version(),
            2_000,
            terminal,
            reserve_deadline(),
        )
        .expect("publish original terminal");
    let tombstone = store
        .acknowledge_direct(&tombstone_key, &terminal_digest, 2_100, reserve_deadline())
        .expect("acknowledge original terminal");
    let premature_key = receipt_key(INVOCATION_B, TASK_B, "workspace-b");
    store
        .reserve(
            premature_key.clone(),
            OriginalCutoffDescriptor::new(3_000, 7_000).expect("valid second cutoff"),
            reserve_deadline(),
        )
        .expect("reserve unrelated receipt");
    let generation_before = store.generation().expect("generation before rejected ACK");

    assert_eq!(
        store.acknowledge_direct(
            &premature_key,
            &terminal_digest,
            tombstone.expires_at_epoch_ms(),
            reserve_deadline(),
        ),
        Err(ReceiptLedgerError::ReceiptRowPresentUnsupported)
    );
    assert_eq!(
        store.generation().expect("generation after rejected ACK"),
        generation_before
    );
    assert_eq!(
        store.recover_exact(&tombstone_key, reserve_deadline()),
        Ok(ReceiptState::AcknowledgedTombstone(tombstone))
    );
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
fn direct_batch_compacts_superseded_envelopes_and_reopens_exact_tombstones() {
    let root = tempfile::tempdir().expect("temporary root");
    let receipts = fs::canonicalize(root.path())
        .expect("physical temporary root")
        .join("receipts");
    let store = ReceiptLedgerStore::open(&receipts).expect("open receipt ledger");
    let requests: Vec<_> = (1_u64..=32)
        .map(|index| {
            let invocation = format!("20000000-0000-4000-8000-{index:012x}");
            let task = format!("30000000-0000-4000-8000-{index:012x}");
            (
                receipt_key(&invocation, &task, &format!("workspace-{index}")),
                OriginalCutoffDescriptor::new(1_000, 7_000).expect("valid cutoff"),
            )
        })
        .collect();
    let reserved = store
        .reserve_batch(requests.clone(), reserve_deadline())
        .expect("reserve direct batch")
        .into_iter()
        .map(|outcome| outcome.into_reservation().expect("new reservation"))
        .collect::<Vec<_>>();
    let workspace_identity = SafeIdentityHash::from_sha256([0x77; 32]);
    let bound = store
        .bind_reserved_actor_batch(
            reserved
                .iter()
                .map(|receipt| {
                    (
                        receipt.key().clone(),
                        receipt.record_version(),
                        workspace_identity.clone(),
                    )
                })
                .collect(),
            reserve_deadline(),
        )
        .expect("bind direct batch");
    let begun = store
        .mark_reserved_begun_batch(
            bound
                .iter()
                .map(|receipt| {
                    (
                        receipt.key().clone(),
                        receipt.record_version(),
                        workspace_identity.clone(),
                    )
                })
                .collect(),
            reserve_deadline(),
        )
        .expect("begin direct batch");
    let terminal = canonical_v5_terminal(&ReceiptTerminalOutcome::Completed {
        result: Box::new(DomainResult::success("batch-terminal")),
    })
    .expect("canonical direct terminal");
    let committed = store
        .publish_direct_terminal_batch(
            begun
                .iter()
                .map(|receipt| {
                    (
                        receipt.key().clone(),
                        receipt.record_version(),
                        1_001,
                        terminal.clone(),
                    )
                })
                .collect(),
            reserve_deadline(),
        )
        .expect("publish direct batch");
    let acknowledged = store
        .acknowledge_direct_batch(
            committed
                .iter()
                .map(|publication| {
                    let receipt = publication.receipt();
                    (
                        receipt.key().clone(),
                        receipt.terminal().digest().clone(),
                        1_002,
                    )
                })
                .collect(),
            reserve_deadline(),
        )
        .expect("acknowledge direct batch");
    assert_eq!(store.generation().expect("batch generation"), 160);
    let active_names: Vec<_> = fs::read_dir(receipts.join(ACTIVE_DIRECTORY_NAME))
        .expect("enumerate active receipt directory")
        .map(|entry| entry.expect("active entry").file_name())
        .collect();
    assert_eq!(
        active_names.len(),
        1,
        "each replacement batch must retire its superseded durable envelope"
    );
    drop(store);

    let reopened = ReceiptLedgerStore::open(&receipts).expect("reopen direct batch ledger");
    assert_eq!(reopened.generation().expect("recovered generation"), 160);
    for (expected, tombstone) in requests.iter().zip(&acknowledged) {
        assert_eq!(
            reopened
                .recover_exact(&expected.0, reserve_deadline())
                .expect("recover exact acknowledged batch winner"),
            ReceiptState::AcknowledgedTombstone(tombstone.clone())
        );
    }
    let expiry = 1_002 + ACKNOWLEDGED_TOMBSTONE_TTL_MS;
    assert_eq!(
        reopened
            .reclaim_expired_tombstones(expiry, reserve_deadline())
            .expect("reclaim exact expired tombstone batch"),
        32
    );
    assert_eq!(reopened.generation().expect("reclaimed generation"), 192);
    assert_eq!(
        fs::read_dir(receipts.join(ACTIVE_DIRECTORY_NAME))
            .expect("enumerate reclaimed active receipt directory")
            .count(),
        0
    );
    drop(reopened);
    let reopened = ReceiptLedgerStore::open(&receipts)
        .expect("reopen after exact tombstone batch reclamation");
    assert_eq!(reopened.generation().expect("reopened generation"), 192);
    for (key, _) in &requests {
        assert_eq!(
            reopened.recover_exact(key, reserve_deadline()),
            Err(ReceiptLedgerError::ReceiptNotFound)
        );
    }
}

#[test]
fn cancelled_direct_batch_reopens_all_exact_winners_from_one_durable_envelope() {
    let root = tempfile::tempdir().expect("temporary root");
    let receipts = fs::canonicalize(root.path())
        .expect("physical temporary root")
        .join("receipts");
    let store = ReceiptLedgerStore::open(&receipts).expect("open receipt ledger");
    let requests: Vec<_> = (1_u64..=32)
        .map(|index| {
            let invocation = format!("00000000-0000-4000-8000-{index:012x}");
            let task = format!("10000000-0000-4000-8000-{index:012x}");
            (
                receipt_key(&invocation, &task, &format!("workspace-{index}")),
                OriginalCutoffDescriptor::new(1_000, 7_000).expect("valid cutoff"),
            )
        })
        .collect();
    let terminal = canonical_v5_terminal(&ReceiptTerminalOutcome::Cancelled)
        .expect("canonical cancelled terminal");
    let committed = store
        .publish_cancelled_direct_batch(
            requests.clone(),
            1_001,
            terminal,
            Instant::now() + Duration::from_secs(2),
        )
        .expect("publish one durable cancelled batch");
    assert_eq!(committed.len(), 32);
    assert_eq!(store.generation().expect("batch generation"), 96);
    let active_names: Vec<_> = fs::read_dir(receipts.join(ACTIVE_DIRECTORY_NAME))
        .expect("enumerate active receipt directory")
        .map(|entry| entry.expect("active entry").file_name())
        .collect();
    assert_eq!(active_names.len(), 1);
    assert!(active_names[0]
        .to_str()
        .is_some_and(|name| name.starts_with("receipt-batch.")));
    drop(store);

    let reopened = ReceiptLedgerStore::open(&receipts).expect("reopen batched receipt ledger");
    assert_eq!(reopened.generation().expect("recovered generation"), 96);
    for ((key, _), expected) in requests.iter().zip(&committed) {
        assert_eq!(
            reopened
                .recover_exact(key, reserve_deadline())
                .expect("recover exact batch-backed winner"),
            ReceiptState::DirectTerminalUnacked(expected.clone())
        );
    }
    let acknowledged = reopened
        .acknowledge_direct(
            &requests[0].0,
            committed[0].terminal().digest(),
            1_002,
            Instant::now() + Duration::from_secs(2),
        )
        .expect("acknowledge one batch-backed Direct winner");
    drop(reopened);
    let reopened = ReceiptLedgerStore::open(&receipts)
        .expect("reopen after materializing and acknowledging one batch row");
    assert_eq!(
        reopened
            .recover_exact(&requests[0].0, reserve_deadline())
            .expect("recover acknowledged batch-backed winner"),
        ReceiptState::AcknowledgedTombstone(acknowledged)
    );
    for ((key, _), expected) in requests.iter().skip(1).zip(committed.iter().skip(1)) {
        assert_eq!(
            reopened
                .recover_exact(key, reserve_deadline())
                .expect("recover untouched batch-backed winner"),
            ReceiptState::DirectTerminalUnacked(expected.clone())
        );
    }
}

#[test]
fn direct_terminal_expires_at_absolute_boundary_and_releases_exact_quota() {
    let root = tempfile::tempdir().expect("temporary root");
    let receipts = fs::canonicalize(root.path())
        .expect("physical temporary root")
        .join("receipts");
    let mut store = ReceiptLedgerStore::open(&receipts).expect("open receipt ledger");
    let key = receipt_key(INVOCATION_A, TASK_A, "workspace-a");
    let key_digest = receipt_key_digest(&key);
    let reserved = store
        .reserve(
            key.clone(),
            OriginalCutoffDescriptor::new(1_000, 7_000).expect("valid cutoff"),
            reserve_deadline(),
        )
        .expect("reserve exact receipt")
        .into_reservation()
        .expect("receipt remains reserved");
    let terminal = canonical_v5_terminal(&ReceiptTerminalOutcome::Completed {
        result: Box::new(DomainResult::success("expiry-owned direct result")),
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
    let expires_at_epoch_ms = committed
        .terminal_epoch_ms()
        .checked_add(DIRECT_TERMINAL_RETENTION_MS)
        .expect("direct expiry fits");
    let row = receipts
        .join(ACTIVE_DIRECTORY_NAME)
        .join(format!("{}.json", key_digest.as_str()));
    let (actual_bytes, reserved_result_bytes) = {
        let catalog = store.writer.lock().expect("inspect terminal accounting");
        assert_eq!(catalog.live_count(), 1);
        assert_eq!(catalog.records.len(), 1);
        assert_eq!(
            catalog.invocation_index.get(&key.invocation_id()),
            Some(&key_digest)
        );
        assert_eq!(
            catalog.reserved_task_index.get(&key.reserved_task_id()),
            Some(&key_digest)
        );
        (catalog.actual_bytes, catalog.reserved_result_bytes)
    };
    assert_eq!(
        actual_bytes.checked_add(reserved_result_bytes),
        Some(MAX_RECEIPT_ENTITLEMENT_BYTES)
    );

    assert_eq!(
        ReceiptLedgerPort::recover_at(
            &mut store,
            &key,
            expires_at_epoch_ms - 1,
            reserve_deadline(),
        ),
        Ok(ReceiptState::DirectTerminalUnacked(committed))
    );
    {
        let catalog = store.writer.lock().expect("inspect pre-expiry accounting");
        assert_eq!(catalog.actual_bytes, actual_bytes);
        assert_eq!(catalog.reserved_result_bytes, reserved_result_bytes);
    }

    assert_eq!(
        store
            .reclaim_expired_tombstones(expires_at_epoch_ms, reserve_deadline())
            .expect("expiry-boundary retention sweep"),
        1
    );
    assert_eq!(
        ReceiptLedgerPort::recover_at(&mut store, &key, expires_at_epoch_ms, reserve_deadline(),),
        Err(ReceiptLedgerError::ReceiptNotFound)
    );
    assert_eq!(store.generation().expect("expiry generation"), 3);
    {
        let catalog = store.writer.lock().expect("inspect expired accounting");
        assert_eq!(catalog.live_count(), 0);
        assert!(catalog.records.is_empty());
        assert!(catalog.invocation_index.is_empty());
        assert!(catalog.reserved_task_index.is_empty());
        assert_eq!(catalog.actual_bytes, 0);
        assert_eq!(catalog.reserved_result_bytes, 0);
        assert_eq!(catalog.tombstone_bytes, 0);
    }
    assert!(
        !row.exists(),
        "expiry physically removes the Direct payload row"
    );

    drop(store);
    let mut reopened = ReceiptLedgerStore::open(&receipts).expect("reopen expired ledger");
    assert_eq!(reopened.generation().expect("reopened generation"), 3);
    assert_eq!(
        ReceiptLedgerPort::recover_at(&mut reopened, &key, expires_at_epoch_ms, reserve_deadline(),),
        Err(ReceiptLedgerError::ReceiptNotFound)
    );
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
    assert!(row.contains("\"originalCutoff\":{\"acceptedEpochMs\":1000,\"responseBudgetMs\":7000}"));
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
        ReceiptLedgerError::Corrupt("receipt terminal digest does not match its canonical outcome")
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
        ReceiptLedgerPort::recover(&mut reopened, &key, near_limit_payload_deadline())
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
        ReceiptLedgerError::Corrupt("nonzero receipt generation is missing its active directory")
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
        serde_json::from_str::<StoredActiveReceiptV1>(
            &row.replacen("\"recordVersion\":1,", "", 1,)
        )
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
        StoredActiveLifecycleV1::ExpiredTombstoneDeletion { .. } => {
            panic!("reserved fixture decoded as a tombstone deletion witness")
        }
        StoredActiveLifecycleV1::ExpiredDirectDeletion { .. } => {
            panic!("reserved fixture decoded as a Direct deletion witness")
        }
        StoredActiveLifecycleV1::ExpiredTaskReceiptDeletion { .. } => {
            panic!("reserved fixture decoded as a receipt-backed Task deletion witness")
        }
        StoredActiveLifecycleV1::CompletedTaskHandoffDeletion { .. } => {
            panic!("reserved fixture decoded as a completed handoff deletion witness")
        }
        StoredActiveLifecycleV1::ReservedUnbound {
            reserved_at_epoch_ms,
            ..
        } => *reserved_at_epoch_ms = 1_001,
        StoredActiveLifecycleV1::ReservedActorBound { .. }
        | StoredActiveLifecycleV1::ReservedBegun { .. } => {
            panic!("unbound fixture decoded as an advanced reservation")
        }
        StoredActiveLifecycleV1::TaskPromisedUnbound { .. } => {
            panic!("unbound fixture decoded as a promised Task")
        }
        StoredActiveLifecycleV1::TaskPromisedActorBound { .. } => {
            panic!("unbound fixture decoded as an actor-bound promised Task")
        }
        StoredActiveLifecycleV1::TaskHandoffActorBound { .. } => {
            panic!("unbound fixture decoded as a Task handoff")
        }
        StoredActiveLifecycleV1::TaskReceiptOwnedActorBound { .. } => {
            panic!("unbound fixture decoded as a receipt-owned Task")
        }
        StoredActiveLifecycleV1::DirectTerminalUnacked { .. } => {
            panic!("reserved fixture decoded as a direct terminal")
        }
        StoredActiveLifecycleV1::TaskTerminalReceiptBacked { .. } => {
            panic!("reserved fixture decoded as a receipt-backed Task terminal")
        }
        StoredActiveLifecycleV1::AcknowledgementCommit { .. } => {
            panic!("reserved fixture decoded as an acknowledgement witness")
        }
        StoredActiveLifecycleV1::AcknowledgedTombstone { .. } => {
            panic!("reserved fixture decoded as an acknowledged tombstone")
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
    let mut temporary = create_owner_only_file_child(&receipts_file, OsStr::new(temporary_name))
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
        fs::read(receipts.join(GENERATION_FILE_NAME)).expect("expired recovery leaves generation"),
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
    let mut temporary = create_owner_only_file_child(&receipts_file, OsStr::new(temporary_name))
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

    let reopened =
        ReceiptLedgerStore::open(&receipts).expect("reopen exact full receipt entitlement pool");
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
        if !set_unix_mode_for_test(&generation_path, 0o644).expect("weaken generation mode fixture")
        {
            return;
        }
        let cutoff =
            OriginalCutoffDescriptor::new(1_000, 7_000).expect("valid original response cutoff");
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
        set_unix_mode_for_test(&generation_path, 0o600).expect("restore generation mode fixture");
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
                ReceiptLedgerError::Corrupt("receipt row is present outside the recovered catalog")
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
    let active_root = fs::canonicalize(active_root.path()).expect("physical active temporary root");
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
