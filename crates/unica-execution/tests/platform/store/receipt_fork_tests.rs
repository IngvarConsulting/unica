#[cfg(unix)]
#[test]
fn forked_child_retains_receipt_lock_after_store_drop_until_child_exit() {
    use crate::receipt_ledger::ReceiptLedgerError;
    use crate::receipt_store::ReceiptLedgerStore;
    use std::fs;

    const HELPER_MARKER: &str = "UNICA_RECEIPT_LEDGER_FORKED_LOCK_PROBE_HELPER";
    if std::env::var_os(HELPER_MARKER).is_none() {
        let status = unica_process::test_support::run_exact_test_in_owned_process_for_test(
            "store::receipt_fork_tests::forked_child_retains_receipt_lock_after_store_drop_until_child_exit",
            HELPER_MARKER,
            &[],
        )
        .expect("run isolated fork/lock proof process");
        assert!(status.success(), "isolated fork/lock proof failed");
        return;
    }

    let root = tempfile::tempdir().expect("temporary root");
    let receipts = fs::canonicalize(root.path())
        .expect("physical temporary root")
        .join("receipts");
    let store = ReceiptLedgerStore::open(&receipts).expect("open receipt ledger");
    let child = unica_workspace::test_support::native::ForkedInheritedDescriptorForTest::start()
        .expect("fork a child retaining the receipt lock description");

    drop(store);
    assert_eq!(
        ReceiptLedgerStore::open(&receipts).err(),
        Some(ReceiptLedgerError::AlreadyOwned),
        "dropping only the parent owner must not release the inherited lock"
    );

    child
        .release_and_wait()
        .expect("reap the inherited lock holder");
    let reopened = ReceiptLedgerStore::open(&receipts)
        .expect("the lock becomes available after the child really exits");
    assert_eq!(reopened.generation().expect("recovered generation"), 0);
}
