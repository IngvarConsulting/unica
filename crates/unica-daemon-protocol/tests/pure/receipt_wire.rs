//! The execution owner's committed frame must be accepted by the real frontend.
use crate::protocol_v5::decode_v5_server_response;
use std::time::{Duration, Instant};
use unica_execution::invocation::normalized_arguments_hash;
use unica_execution::receipt_ledger::{
    canonical_v5_terminal, request_scope_hash, CoreIdentityDigest, OriginalCutoffDescriptor,
    ReceiptKey, ReceiptTerminalOutcome, RequestIdentity, V5ToolIdentity,
};
use unica_execution::receipt_store::ReceiptLedgerStore;
use unica_execution_contract::invocation::DomainResult;

#[test]
fn frontend_accepts_the_exact_preflighted_direct_receipt_frame() {
    let mut result = DomainResult::success("direct result with a floating-point score");
    result.data = Some(serde_json::json!({"score": 493.85565185546875}));
    for outcome in [
        ReceiptTerminalOutcome::Cancelled,
        ReceiptTerminalOutcome::Completed {
            result: Box::new(result),
        },
    ] {
        let root = tempfile::tempdir().unwrap();
        let receipts = std::fs::canonicalize(root.path()).unwrap().join("receipts");
        let store = ReceiptLedgerStore::open(&receipts).unwrap();
        let key = ReceiptKey::new(
            "11111111-1111-4111-8111-111111111111".parse().unwrap(),
            "33333333-3333-4333-8333-333333333333".parse().unwrap(),
            RequestIdentity::new(
                CoreIdentityDigest::from_sha256([0x55; 32]),
                V5ToolIdentity::View,
                normalized_arguments_hash(&serde_json::Map::new()),
                request_scope_hash("workspace-a").unwrap(),
            ),
        );
        let reserved = store
            .reserve(
                key.clone(),
                OriginalCutoffDescriptor::new(1_000, 7_000).unwrap(),
                Instant::now() + Duration::from_secs(2),
            )
            .unwrap()
            .into_reservation()
            .unwrap();
        let committed = store
            .publish_direct_terminal_publication(
                &key,
                reserved.record_version(),
                2_000,
                canonical_v5_terminal(&outcome, unica_bootstrap::MAX_RESPONSE_BUDGET).unwrap(),
                Instant::now() + Duration::from_secs(2),
            )
            .unwrap();
        decode_v5_server_response(committed.wire_frame().jsonl())
            .expect("frontend must accept the exact direct receipt wire frame");
    }
}
