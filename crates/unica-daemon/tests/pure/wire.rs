use super::*;

#[test]
fn lifecycle_reply_wrapping_preserves_frozen_outer_frame_bytes() {
    // The reserve key/order is the existing strict CR0 wire fixture.
    let key = r#"{"invocationId":"11111111-1111-4111-8111-111111111111","reservedTaskId":"22222222-2222-4222-8222-222222222222","coreIdentityDigest":"884b76181583ce34907a2a9758e2b493e5b40883e7cbb0d7f88dcec0e468cfa0","tool":"unica.view","normalizedArgumentsHash":"0000000000000000000000000000000000000000000000000000000000000000","requestScopeHash":"0000000000000000000000000000000000000000000000000000000000000000"}"#;
    let frames = [
        format!(r#"{{"kind":"invocation","outcome":{{"resultType":"receipt_pending","receiptKey":{key},"phase":"cancel_reserved","acceptedEpochMs":1700000000000,"originalBudgetMs":0,"cancelRequested":true}}}}"#),
        r#"{"kind":"task","snapshot":{"status":"queued","taskId":"22222222-2222-4222-8222-222222222222","invocationId":"11111111-1111-4111-8111-111111111111","receiptKeyDigest":"0000000000000000000000000000000000000000000000000000000000000000","createdAtEpochMs":1700000000000,"updatedAtEpochMs":1700000000000,"ttlMs":10000,"pollIntervalMs":100,"version":1,"cancelRequested":false}}"#.to_owned(),
        format!(r#"{{"kind":"invocation_acknowledged","acknowledgement":{{"receiptKey":{key},"terminalDigest":"0000000000000000000000000000000000000000000000000000000000000000","ackEpochMs":1700000000000,"expiresEpochMs":1700000010000}}}}"#),
    ];
    for frozen in frames {
        let original = crate::protocol_v5::decode_v5_server_response(frozen.as_bytes())
            .expect("the frozen existing outer frame is strict");
        let lifecycle = match original {
            V5ServerResponse::Invocation { outcome } => ReceiptResponse::Invocation { outcome },
            V5ServerResponse::Task { snapshot } => ReceiptResponse::Task { snapshot },
            V5ServerResponse::InvocationAcknowledged { acknowledgement } => {
                ReceiptResponse::InvocationAcknowledged { acknowledgement }
            }
            _ => panic!("fixture must be an execution lifecycle response"),
        };
        let framed = outer_response(lifecycle);
        assert_eq!(
            encode_json_line(&framed).expect("encode whole owned lifecycle value"),
            [frozen.as_bytes(), b"\n"].concat(),
        );
    }
}
