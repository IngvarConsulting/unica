use super::*;
use std::path::PathBuf;

fn direct_terminal_identity_without_opening_store() -> (ReceiptKey, TerminalDigest) {
    let key = receipt_key(INVOCATION_A, TASK_A, "workspace-a");
    let digest = canonical_v5_terminal(&ReceiptTerminalOutcome::Cancelled)
        .expect("canonical direct terminal")
        .digest()
        .clone();
    (key, digest)
}

const ACK_CRASH_CHILD_MARKER: &str = "UNICA_RECEIPT_LEDGER_ACK_CRASH_CHILD";
const ACK_CRASH_RECEIPTS_PATH: &str = "UNICA_RECEIPT_LEDGER_ACK_CRASH_RECEIPTS";

fn run_ack_crash_child(test_name: &str, receipts: &Path) {
    let status =
        crate::infrastructure::platform::testing::run_exact_test_in_owned_process_for_test(
            test_name,
            ACK_CRASH_CHILD_MARKER,
            &[(ACK_CRASH_RECEIPTS_PATH, receipts.as_os_str())],
        )
        .expect("run isolated ACK crash process");
    assert!(status.success(), "isolated ACK crash process failed");
}

fn ack_crash_child_receipts_path() -> PathBuf {
    PathBuf::from(std::env::var_os(ACK_CRASH_RECEIPTS_PATH).expect("child receipts path"))
}

#[test]
fn ack_crash_after_witness_row_before_generation_heals_and_compacts_on_reopen() {
    if std::env::var_os(ACK_CRASH_CHILD_MARKER).is_some() {
        let receipts = ack_crash_child_receipts_path();
        let (store, key, terminal_digest) = direct_terminal_fixture(&receipts);
        let stage_evidence = receipts.parent().expect("temporary root").join("ack-stage");
        let generation = receipts.join(GENERATION_FILE_NAME);
        set_after_receipt_row_rename_hook_for_test(move || {
            assert_eq!(
                fs::read(&generation).expect("read generation at witness publication"),
                b"2\n"
            );
            fs::write(&stage_evidence, b"witness-row-before-generation")
                .expect("record witness-row crash stage");
            std::process::exit(0);
        });
        store
            .acknowledge_direct(&key, &terminal_digest, 2_100, reserve_deadline())
            .expect("witness-row crash hook interrupts ACK");
        panic!("witness-row crash hook was not reached");
    }

    let root = tempfile::tempdir().expect("temporary root");
    let receipts = fs::canonicalize(root.path())
        .expect("physical temporary root")
        .join("receipts");
    let (key, _) = direct_terminal_identity_without_opening_store();
    run_ack_crash_child(
        "infrastructure::receipt_ledger::tests::crash_process_tests::ack_crash_after_witness_row_before_generation_heals_and_compacts_on_reopen",
        &receipts,
    );
    assert_eq!(
        fs::read(root.path().join("ack-stage")).expect("read witness-row crash stage"),
        b"witness-row-before-generation"
    );
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
    if std::env::var_os(ACK_CRASH_CHILD_MARKER).is_some() {
        let receipts = ack_crash_child_receipts_path();
        let (store, key, terminal_digest) = direct_terminal_fixture(&receipts);
        let row_path = receipts
            .join(ACTIVE_DIRECTORY_NAME)
            .join(format!("{}.json", receipt_key_digest(&key).as_str()));
        let stage_evidence = receipts.parent().expect("temporary root").join("ack-stage");
        set_after_generation_replace_hook_for_test(move || {
            let observed =
                fs::read_to_string(&row_path).expect("read ACK witness at generation publication");
            assert!(
                observed.contains("\"state\":\"acknowledgement_commit\""),
                "generation must not become authoritative while only a sequence-free tombstone is visible"
            );
            fs::write(&stage_evidence, observed).expect("record generation crash stage");
            std::process::exit(0);
        });
        store
            .acknowledge_direct(&key, &terminal_digest, 2_100, reserve_deadline())
            .expect("generation crash hook interrupts ACK");
        panic!("generation crash hook was not reached");
    }

    let root = tempfile::tempdir().expect("temporary root");
    let receipts = fs::canonicalize(root.path())
        .expect("physical temporary root")
        .join("receipts");
    let (key, _) = direct_terminal_identity_without_opening_store();
    run_ack_crash_child(
        "infrastructure::receipt_ledger::tests::crash_process_tests::ack_generation_is_published_while_the_durable_witness_is_still_visible",
        &receipts,
    );
    assert!(
        fs::read_to_string(root.path().join("ack-stage"))
            .expect("read generation crash stage")
            .contains("\"state\":\"acknowledgement_commit\""),
        "generation must not become authoritative while only a sequence-free tombstone is visible"
    );

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
    if std::env::var_os(ACK_CRASH_CHILD_MARKER).is_some() {
        let receipts = ack_crash_child_receipts_path();
        let (store, key, terminal_digest) = direct_terminal_fixture(&receipts);
        let generation = receipts.join(GENERATION_FILE_NAME);
        let row_path = receipts
            .join(ACTIVE_DIRECTORY_NAME)
            .join(format!("{}.json", receipt_key_digest(&key).as_str()));
        let stage_evidence = receipts.parent().expect("temporary root").join("ack-stage");
        set_after_generation_replace_hook_for_test(move || {
            set_after_receipt_row_rename_hook_for_test(move || {
                assert_eq!(
                    fs::read(&generation).expect("read committed generation at compact row"),
                    b"3\n"
                );
                assert!(
                    !fs::read(&row_path)
                        .expect("read compact ACK row")
                        .is_empty(),
                    "compact ACK row must be visible before process death"
                );
                fs::write(&stage_evidence, b"compact-row-after-generation")
                    .expect("record compact-row crash stage");
                std::process::exit(0);
            });
        });
        store
            .acknowledge_direct(&key, &terminal_digest, 2_100, reserve_deadline())
            .expect("compact-row crash hook interrupts ACK");
        panic!("compact-row crash hook was not reached");
    }

    let root = tempfile::tempdir().expect("temporary root");
    let receipts = fs::canonicalize(root.path())
        .expect("physical temporary root")
        .join("receipts");
    let (key, terminal_digest) = direct_terminal_identity_without_opening_store();
    run_ack_crash_child(
        "infrastructure::receipt_ledger::tests::crash_process_tests::ack_crash_after_compact_row_rename_reopens_the_same_tombstone",
        &receipts,
    );
    assert_eq!(
        fs::read(root.path().join("ack-stage")).expect("read compact-row crash stage"),
        b"compact-row-after-generation"
    );

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
