use super::*;
use crate::test_support::*;
use serde_json::json;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::thread;
use unica_daemon_protocol::protocol_v5::V5ClientRequest;
use unica_execution::receipt_ledger::{canonical_v5_terminal, ReceiptTerminalOutcome};
use unica_execution::receipt_runtime::response::V5PendingDirectReceipt;

/// rule: INV.APP.DIRECT-RESPONSE-RECOVERY
#[test]
fn direct_terminal_is_projected_before_it_is_acknowledged_with_the_exact_digest() {
    let fake = FakeDaemon::start(Box::new(|raw, request| match request {
        V5ClientRequest::SubmitInvocation { .. } => {
            Step::Reply(direct_receipt(daemon_side_key(raw), completed("viewed")))
        }
        V5ClientRequest::AcknowledgeInvocationReceipt { .. } => {
            Step::Reply(V5ServerResponse::Error {
                code: V5DaemonErrorCode::TombstoneCapacity,
            })
        }
        other => panic!("unexpected frame {other:?}"),
    }));
    let router = canonical_daemon_router(fake.owner(), fake.workspace_hint());

    let result = direct_result(call(&router, None));

    assert_eq!(result.is_error, Some(false));
    assert!(result.content.is_empty());
    assert_eq!(
        result
            .structured_content
            .as_ref()
            .and_then(|value| value["summary"].as_str()),
        Some("viewed")
    );
    let seen = fake.seen();
    assert_eq!(seen.len(), 2, "submit then acknowledgement: {seen:?}");
    let V5ClientRequest::SubmitInvocation { invocation } = &seen[0] else {
        panic!("first frame must be the submission");
    };
    let V5ClientRequest::AcknowledgeInvocationReceipt {
        receipt_key,
        terminal_digest,
    } = &seen[1]
    else {
        panic!("second frame must be the acknowledgement");
    };
    assert_eq!(receipt_key.invocation_id(), invocation.invocation_id());
    assert_eq!(
        receipt_key.reserved_task_id(),
        invocation.reserved_task_id()
    );
    assert_eq!(
        terminal_digest,
        canonical_v5_terminal(&completed("viewed"), unica_bootstrap::MAX_RESPONSE_BUDGET)
            .unwrap()
            .digest()
    );
    assert!(
        (6_900..=7_000).contains(&invocation.response_budget_ms()),
        "the daemon budget is the remaining handoff window: {}",
        invocation.response_budget_ms()
    );
    assert_eq!(invocation.workspace_hint(), fake.workspace_hint());
}

/// rule: INV.APP.DIRECT-RESPONSE-RECOVERY
#[test]
fn failed_and_cancelled_direct_terminals_answer_closed_errors_after_acknowledgement() {
    use unica_execution::invocation_store_v5::V5SafeFailureReason;

    let terminals = Arc::new(Mutex::new(vec![
        ReceiptTerminalOutcome::Cancelled,
        ReceiptTerminalOutcome::Failed {
            reason: V5SafeFailureReason::OutcomeUncertain,
        },
        ReceiptTerminalOutcome::Failed {
            reason: V5SafeFailureReason::OutcomeUncertain,
        },
    ]));
    let script_terminals = Arc::clone(&terminals);
    let fake = FakeDaemon::start(Box::new(move |raw, request| match request {
        V5ClientRequest::SubmitInvocation { .. } => {
            let terminal = script_terminals.lock().unwrap().pop().unwrap();
            Step::Reply(direct_receipt(daemon_side_key(raw), terminal))
        }
        V5ClientRequest::AcknowledgeInvocationReceipt { .. } => Step::Close,
        other => panic!("unexpected frame {other:?}"),
    }));
    let router = canonical_daemon_router(fake.owner(), fake.workspace_hint());

    let uncertain = call(&router, None).expect_err("failed terminal is an error");
    let mutation = (router.call)(
        V5ToolIdentity::Apply,
        &arguments(),
        &unica_bootstrap::HostRequest::default(),
        deadline(None),
        CancellationToken::new().into(),
    )
    .expect_err("failed terminal is an error");
    let cancelled = call(&router, None).expect_err("cancelled terminal is an error");

    // A read-only call changed nothing: the refusal says so and offers
    // the same call as a new one.
    assert_eq!(uncertain.code, ErrorCode::INTERNAL_ERROR);
    assert_eq!(
            uncertain.message,
            "daemon invocation outcome is uncertain; `unica.view` only reads, so the call changed nothing: repeat it"
        );
    assert_eq!(
        uncertain.data,
        Some(json!({
            "code": "outcome_uncertain",
            "next": [{"tool": "unica.view", "args": arguments()}],
        }))
    );
    // A mutation may have taken effect: the closed refusal stays as it is.
    assert_eq!(mutation.message, "daemon invocation outcome is uncertain");
    assert_eq!(mutation.data, Some(json!({"code": "outcome_uncertain"})));
    assert_eq!(cancelled.message, "daemon invocation was cancelled");
    assert_eq!(
        cancelled.data,
        Some(json!({"code": "invocation_cancelled"}))
    );
    let acknowledgements = fake
        .seen()
        .iter()
        .filter(|request| {
            matches!(
                request,
                V5ClientRequest::AcknowledgeInvocationReceipt { .. }
            )
        })
        .count();
    assert_eq!(
        acknowledgements, 3,
        "every delivered terminal is acknowledged"
    );
}

/// rule: INV.APP.DIRECT-RESPONSE-RECOVERY
/// rule: INV.APP.RECEIPT-ACKNOWLEDGEMENT
#[test]
fn malformed_direct_receipt_is_recovered_by_key_and_only_the_strict_one_is_acknowledged() {
    let fake = FakeDaemon::start(Box::new(|raw, request| match request {
        V5ClientRequest::SubmitInvocation { .. } => {
            // A receipt whose digest does not match its terminal is not a
            // strict frame: the client poisons the session and recovers.
            let forged: unica_execution::receipt_ledger::TerminalDigest =
                "0a".repeat(32).parse().unwrap();
            Step::Reply(V5ServerResponse::Invocation {
                outcome: V5InvocationResponse::Direct {
                    receipt: V5PendingDirectReceipt::new(
                        daemon_side_key(raw),
                        completed("forged"),
                        forged,
                        epoch_ms_now(),
                    ),
                },
            })
        }
        V5ClientRequest::RecoverInvocationReceipt { receipt_key } => {
            Step::Reply(direct_receipt(receipt_key.clone(), completed("strict")))
        }
        V5ClientRequest::AcknowledgeInvocationReceipt { .. } => Step::Close,
        other => panic!("unexpected frame {other:?}"),
    }));
    let router = canonical_daemon_router(fake.owner(), fake.workspace_hint());

    let result = direct_result(call(&router, None));

    assert_eq!(
        result
            .structured_content
            .as_ref()
            .and_then(|value| value["summary"].as_str()),
        Some("strict")
    );
    let seen = fake.seen();
    assert_eq!(fake.submissions(), 1);
    let acknowledged = seen
        .iter()
        .filter_map(|request| match request {
            V5ClientRequest::AcknowledgeInvocationReceipt {
                terminal_digest, ..
            } => Some(terminal_digest.clone()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        acknowledged,
        vec![
            canonical_v5_terminal(&completed("strict"), unica_bootstrap::MAX_RESPONSE_BUDGET)
                .unwrap()
                .digest()
                .clone()
        ],
        "only the strict receipt is acknowledged"
    );
}

/// rule: INV.APP.DIRECT-RESPONSE-RECOVERY
#[test]
fn lost_submit_response_follows_replaced_endpoint_without_resubmitting() {
    use unica_daemon_protocol::identity::{CoreIdentity, DaemonStateDirectory};
    let root = tempfile::tempdir().unwrap();
    let state_root = std::fs::canonicalize(root.path()).unwrap();
    let identity = CoreIdentity::production_v5();
    let successor = FakeDaemon::start(Box::new(|_, request| match request {
        V5ClientRequest::RecoverInvocationReceipt { receipt_key } => Step::Reply(direct_receipt(
            receipt_key.clone(),
            completed("successor recovery"),
        )),
        V5ClientRequest::AcknowledgeInvocationReceipt { .. } => Step::Close,
        other => panic!("successor received an unexpected request: {other:?}"),
    }));
    let successor_record = successor.record.clone();
    let publish_root = state_root.clone();
    let first = FakeDaemon::start(Box::new(move |_, request| {
        assert!(matches!(request, V5ClientRequest::SubmitInvocation { .. }));
        let state =
            DaemonStateDirectory::open(&publish_root, &CoreIdentity::production_v5()).unwrap();
        state.publish_v5_endpoint_record(&successor_record).unwrap();
        Step::Close
    }));
    let state = DaemonStateDirectory::open(&state_root, &identity).unwrap();
    state.publish_v5_endpoint_record(&first.record).unwrap();
    let client = V5DaemonClient::connect(
        state_root,
        identity,
        root.path().join("must-not-spawn"),
        Duration::from_secs(1),
    )
    .unwrap();
    let router = canonical_daemon_router(client, first.workspace_hint());
    let result = direct_result(call(&router, None));
    assert_eq!(
        result.structured_content.as_ref().unwrap()["summary"],
        "successor recovery"
    );
    assert_eq!(first.submissions(), 1);
    assert_eq!(successor.submissions(), 0);
    let submissions = first.seen();
    let V5ClientRequest::SubmitInvocation { invocation } = &submissions[0] else {
        panic!("missing submit")
    };
    let recovery = successor.seen();
    let V5ClientRequest::RecoverInvocationReceipt { receipt_key } = &recovery[0] else {
        panic!("missing recovery")
    };
    assert_eq!(
        receipt_key,
        &receipt_key_for(invocation, &V5DaemonClient::from(first.owner())).unwrap()
    );
}

/// rule: INV.APP.DIRECT-RESPONSE-RECOVERY
#[test]
fn lost_submit_response_is_recovered_by_the_exact_key_without_a_second_submission() {
    let fake = FakeDaemon::start(Box::new(|raw, request| match request {
        V5ClientRequest::SubmitInvocation { .. } => Step::Close,
        V5ClientRequest::RecoverInvocationReceipt { receipt_key } => {
            Step::Reply(direct_receipt(receipt_key.clone(), completed("recovered")))
        }
        V5ClientRequest::AcknowledgeInvocationReceipt { .. } => Step::Close,
        other => panic!("unexpected frame {raw:?} {other:?}"),
    }));
    let router = canonical_daemon_router(fake.owner(), fake.workspace_hint());

    let result = direct_result(call(&router, None));

    assert_eq!(
        result
            .structured_content
            .as_ref()
            .and_then(|value| value["summary"].as_str()),
        Some("recovered")
    );
    let seen = fake.seen();
    assert_eq!(
        fake.submissions(),
        1,
        "a lost response never resubmits: {seen:?}"
    );
    let V5ClientRequest::SubmitInvocation { invocation } = &seen[0] else {
        panic!("first frame must be the submission");
    };
    let V5ClientRequest::RecoverInvocationReceipt { receipt_key } = &seen[1] else {
        panic!("second frame must recover by key: {seen:?}");
    };
    assert_eq!(receipt_key.invocation_id(), invocation.invocation_id());
    assert_eq!(
        receipt_key.reserved_task_id(),
        invocation.reserved_task_id()
    );
    assert_eq!(receipt_key.tool(), V5ToolIdentity::View);
    assert!(matches!(
        seen[2],
        V5ClientRequest::AcknowledgeInvocationReceipt { .. }
    ));
    assert_eq!(
        fake.sessions.load(Ordering::SeqCst),
        3,
        "anchor, submit peer, recovery peer"
    );
}

/// An answer to the submit that trickles in byte by byte does not renew
/// the frontend's deadline: the call ends within its own window plus the
/// recovery budget, long before the trickle would finish.
/// rule: INV.WIRE.DAEMON-RESPONSE-TIME
#[test]
fn a_trickling_submit_answer_does_not_renew_the_frontend_deadline() {
    use std::io::Write as _;
    let host = Duration::from_millis(300);
    let fake =
        FakeDaemon::start_with_raw_script(Box::new(move |raw, request, writer| match request {
            V5ClientRequest::SubmitInvocation { .. } => {
                let mut frame = serde_json::to_vec(&direct_receipt(
                    daemon_side_key(raw),
                    completed("trickled"),
                ))
                .unwrap();
                frame.push(b'\n');
                for byte in frame {
                    if writer
                        .write_all(&[byte])
                        .and_then(|()| writer.flush())
                        .is_err()
                    {
                        break;
                    }
                    thread::sleep(Duration::from_millis(50));
                }
                false
            }
            V5ClientRequest::RecoverInvocationReceipt { .. } => false,
            other => panic!("unexpected frame {other:?}"),
        }));
    let router = canonical_daemon_router(fake.owner(), fake.workspace_hint());

    let started = Instant::now();
    let outcome = call(&router, Some(host));
    let elapsed = started.elapsed();
    assert!(
        matches!(&outcome, Ok(CanonicalCallOutcome::Task(_))),
        "the trickled submit was not lost by its deadline: {outcome:?}"
    );
    assert!(
        elapsed < host + RECOVERY_BUDGET + Duration::from_secs(1),
        "the trickle renewed the deadline: {elapsed:?}"
    );
    assert_eq!(fake.submissions(), 1);
}

/// The frontend reads a real answer of exactly the frame limit. One byte
/// more, or an answer cut before its line ends, leaves the submit session
/// unusable: the frontend recovers that same receipt by key on a fresh
/// session and never submits again.
/// rule: INV.APP.DIRECT-RESPONSE-RECOVERY
/// rule: INV.WIRE.DAEMON-RESPONSE-SIZE
#[test]
fn submit_answer_at_the_frame_limit_is_read_and_one_byte_over_or_cut_is_recovered() {
    use std::io::Write as _;
    let limit = unica_daemon_protocol::protocol_v5::MAX_V5_RESPONSE_LINE_BYTES;
    // A valid answer padded inside its JSON to `line` bytes with the newline.
    let padded = |line: usize| {
        let compact = serde_json::to_vec(&V5ServerResponse::Error {
            code: V5DaemonErrorCode::Overloaded,
        })
        .unwrap();
        let mut frame = vec![b'{'];
        frame.extend(std::iter::repeat_n(b' ', line - compact.len() - 1));
        frame.extend_from_slice(&compact[1..]);
        frame.push(b'\n');
        assert_eq!(frame.len(), line);
        frame
    };
    for (case, frame, recovered) in [
        ("at the limit", padded(limit), false),
        ("one byte over", padded(limit + 1), true),
        ("cut", padded(limit)[..limit / 2].to_vec(), true),
    ] {
        let fake = FakeDaemon::start_with_raw_script(Box::new(move |_, request, writer| {
            match request {
                V5ClientRequest::SubmitInvocation { .. } => {
                    let _ = writer.write_all(&frame);
                    let _ = writer.flush();
                    // An answer that cannot be read closes this session.
                    !recovered
                }
                V5ClientRequest::RecoverInvocationReceipt { receipt_key } => {
                    crate::test_support::write_frame(
                        writer,
                        &direct_receipt(receipt_key.clone(), completed("recovered")),
                    );
                    true
                }
                V5ClientRequest::AcknowledgeInvocationReceipt { .. } => false,
                other => panic!("{case}: unexpected frame {other:?}"),
            }
        }));
        let router = canonical_daemon_router(fake.owner(), fake.workspace_hint());
        let outcome = call(&router, None);
        if recovered {
            let result = direct_result(outcome);
            assert_eq!(
                result
                    .structured_content
                    .as_ref()
                    .and_then(|value| value["summary"].as_str()),
                Some("recovered"),
                "{case}"
            );
        } else {
            let refused = outcome.expect_err("the read answer is a rejection");
            assert!(
                refused.message.contains("overloaded"),
                "{case}: {refused:?}"
            );
        }
        assert_eq!(fake.submissions(), 1, "{case}: submitted again");
    }
}

/// A first submit is never answered with a pending receipt. If one is,
/// the frontend refuses with the closed `receipt_pending` and neither
/// submits again nor starts a recovery of its own.
/// rule: INV.WIRE.DAEMON-INITIAL-SUBMIT-RESULT
#[test]
fn a_first_submit_answered_receipt_pending_is_a_closed_refusal_without_resubmit() {
    let frames = Arc::new(AtomicUsize::new(0));
    let script_frames = Arc::clone(&frames);
    let fake = FakeDaemon::start(Box::new(move |raw, request| {
        script_frames.fetch_add(1, Ordering::SeqCst);
        match request {
            V5ClientRequest::SubmitInvocation { .. } => Step::Reply(V5ServerResponse::Invocation {
                outcome: V5InvocationResponse::ReceiptPending {
                    receipt_key: daemon_side_key(raw),
                    phase:
                        unica_execution::receipt_runtime::response::V5InvocationPhase::ReservedBegun,
                    accepted_epoch_ms: epoch_ms_now(),
                    original_budget_ms: 100,
                    cancel_requested: false,
                },
            }),
            other => panic!("a pending first answer must not be recovered: {other:?}"),
        }
    }));
    let router = canonical_daemon_router(fake.owner(), fake.workspace_hint());

    let refused = call(&router, None).expect_err("a pending first answer is refused");
    assert_eq!(
        refused.data,
        Some(serde_json::json!({"code": "receipt_pending"}))
    );
    assert_eq!(fake.submissions(), 1);
    assert_eq!(
        frames.load(Ordering::SeqCst),
        1,
        "no frame follows the submit"
    );
}

/// rule: INV.APP.DIRECT-RESPONSE-RECOVERY
/// rule: INV.WIRE.DAEMON-INITIAL-SUBMIT-RESULT
#[test]
fn pending_receipt_is_polled_until_its_original_budget_settles() {
    let recoveries = Arc::new(AtomicUsize::new(0));
    let script_recoveries = Arc::clone(&recoveries);
    let fake = FakeDaemon::start(Box::new(move |_, request| match request {
        V5ClientRequest::SubmitInvocation { .. } => Step::Close,
        V5ClientRequest::RecoverInvocationReceipt { receipt_key } => {
            if script_recoveries.fetch_add(1, Ordering::SeqCst) == 0 {
                Step::Reply(V5ServerResponse::Invocation {
                        outcome: V5InvocationResponse::ReceiptPending {
                            receipt_key: receipt_key.clone(),
                            phase: unica_execution::receipt_runtime::response::V5InvocationPhase::ReservedBegun,
                            accepted_epoch_ms: epoch_ms_now(),
                            original_budget_ms: 100,
                            cancel_requested: false,
                        },
                    })
            } else {
                Step::Reply(direct_receipt(receipt_key.clone(), completed("settled")))
            }
        }
        V5ClientRequest::AcknowledgeInvocationReceipt { .. } => Step::Close,
        other => panic!("unexpected frame {other:?}"),
    }));
    let router = canonical_daemon_router(fake.owner(), fake.workspace_hint());

    let started = Instant::now();
    let result = direct_result(call(&router, None));

    assert_eq!(
        result
            .structured_content
            .as_ref()
            .and_then(|value| value["summary"].as_str()),
        Some("settled")
    );
    assert!(
        started.elapsed() >= Duration::from_millis(100),
        "recovery waited for the budget to settle"
    );
    assert_eq!(recoveries.load(Ordering::SeqCst), 2);
    assert_eq!(fake.submissions(), 1);
}

/// rule: INV.APP.DIRECT-RESPONSE-RECOVERY
/// rule: INV.WIRE.DAEMON-INITIAL-SUBMIT-RESULT
/// rule: INV.WIRE.LOST-SUBMIT-TASK-ID
#[test]
fn pending_receipt_beyond_the_frontend_cutoff_is_the_reserved_task_not_a_retry() {
    let fake = FakeDaemon::start(Box::new(move |_, request| {
        match request {
            V5ClientRequest::SubmitInvocation { .. } => Step::Close,
            V5ClientRequest::RecoverInvocationReceipt { receipt_key } => {
                Step::Reply(V5ServerResponse::Invocation {
                    outcome: V5InvocationResponse::ReceiptPending {
                        receipt_key: receipt_key.clone(),
                        phase: unica_execution::receipt_runtime::response::V5InvocationPhase::ReservedActorBound,
                        accepted_epoch_ms: epoch_ms_now(),
                        original_budget_ms: 7_000,
                        cancel_requested: false,
                    },
                })
            }
            other => panic!("unexpected frame {other:?}"),
        }
    }));
    let router = canonical_daemon_router(fake.owner(), fake.workspace_hint());

    let task = lost_task(call(&router, Some(Duration::from_millis(400))));

    assert_is_the_reserved_task(&task, &fake);
    assert_eq!(fake.submissions(), 1);
}

/// A lost submit answers with the Task reserved for the one submitted
/// call, never a Task of its own or of another call.
fn assert_is_the_reserved_task(task: &V5DaemonTaskSnapshot, fake: &FakeDaemon) {
    let seen = fake.seen();
    let invocation = seen
        .iter()
        .find_map(|request| match request {
            V5ClientRequest::SubmitInvocation { invocation } => Some(invocation),
            _ => None,
        })
        .expect("the call was submitted");
    assert_eq!(task.task_id(), invocation.reserved_task_id());
    assert_eq!(task.invocation_id(), invocation.invocation_id());
}

fn assert_not_delivered(refusal: &ErrorData) {
    assert_eq!(refusal.code, ErrorCode(TOOL_EXECUTION_ERROR));
    assert_eq!(
        refusal.data,
        Some(json!({"code": "submit_not_delivered", "outcome": "retry"}))
    );
    assert!(!refusal.message.contains("Task"), "{}", refusal.message);
}

/// rule: INV.WIRE.LOST-SUBMIT-TASK-ID
#[test]
fn a_lost_submit_is_the_queued_task_reserved_for_the_call() {
    let fake = FakeDaemon::start(Box::new(|_, request| match request {
        V5ClientRequest::SubmitInvocation { .. } => Step::Close,
        V5ClientRequest::RecoverInvocationReceipt { .. } => Step::Close,
        other => panic!("unexpected frame {other:?}"),
    }));
    let router = canonical_daemon_router(fake.owner(), fake.workspace_hint());

    let task = lost_task(call(&router, None));

    let seen = fake.seen();
    let V5ClientRequest::SubmitInvocation { invocation } = &seen[0] else {
        panic!("first frame must be the submission: {seen:?}");
    };
    let key = receipt_key_for(invocation, &V5DaemonClient::from(fake.owner())).unwrap();
    assert_eq!(
        task.receipt_key_digest(),
        &unica_execution::receipt_ledger::receipt_key_digest(&key)
    );
    assert_is_the_reserved_task(&task, &fake);
}

/// rule: INV.WIRE.LOST-SUBMIT-TASK-ID
#[test]
fn a_daemon_refusing_the_connection_needs_a_person_not_a_repeat() {
    // The anchor stands; every further session is dropped right after its
    // handshake request, long before the cutoff.
    let fake = FakeDaemon::start_rejecting_peers();
    let router = canonical_daemon_router(fake.owner(), fake.workspace_hint());

    let refusal = call(&router, None).expect_err("a refused connection is refused");

    assert_eq!(refusal.code, ErrorCode(TOOL_EXECUTION_ERROR));
    assert_eq!(
        refusal.data,
        Some(json!({
            "code": "task_backend_failed",
            "outcome": "needsHuman",
            "detailCode": "backend_incompatible",
        }))
    );
    assert!(
        refusal.message.contains("will not run"),
        "{}",
        refusal.message
    );
    assert_eq!(fake.submissions(), 0);
}

/// rule: INV.WIRE.LOST-SUBMIT-TASK-ID
#[test]
fn a_call_whose_connection_never_stood_was_not_delivered() {
    // A handshake slower than the whole host budget: no frame of the
    // call leaves the frontend.
    let fake = FakeDaemon::start_with_handshake_delay(
        Box::new(|_, request| panic!("nothing is submitted: {request:?}")),
        Duration::from_millis(600),
    );
    let router = canonical_daemon_router(fake.owner(), fake.workspace_hint());

    let refusal =
        call(&router, Some(Duration::from_millis(200))).expect_err("an unreachable daemon refuses");

    assert_not_delivered(&refusal);
    assert_eq!(fake.submissions(), 0);
}

#[test]
fn daemon_budget_is_what_remains_of_the_frontend_budget_once_the_connection_stands() {
    let handshake_delay = Duration::from_millis(400);
    let fake = FakeDaemon::start_with_handshake_delay(
        Box::new(|raw, request| match request {
            V5ClientRequest::SubmitInvocation { .. } => {
                Step::Reply(direct_receipt(daemon_side_key(raw), completed("viewed")))
            }
            V5ClientRequest::AcknowledgeInvocationReceipt { .. } => Step::Close,
            other => panic!("unexpected frame {other:?}"),
        }),
        handshake_delay,
    );
    let router = canonical_daemon_router(fake.owner(), fake.workspace_hint());

    let result = direct_result(call(&router, None));

    assert_eq!(result.is_error, Some(false));
    let seen = fake.seen();
    let V5ClientRequest::SubmitInvocation { invocation } = &seen[0] else {
        panic!("first frame must be the submission: {seen:?}");
    };
    let budget = Duration::from_millis(invocation.response_budget_ms());
    assert!(
        budget <= INVOCATION_HANDOFF_WINDOW - handshake_delay,
        "the handshake spent before the submission comes off the daemon budget: {budget:?}"
    );
    assert!(
        budget >= INVOCATION_HANDOFF_WINDOW - handshake_delay - Duration::from_secs(2),
        "only the time actually spent comes off the daemon budget: {budget:?}"
    );
}

/// rule: INV.WIRE.LOST-SUBMIT-TASK-ID
#[test]
fn recovery_without_a_receipt_fences_the_exact_key_and_answers_not_delivered() {
    let fake = FakeDaemon::start(Box::new(|_, request| match request {
        V5ClientRequest::SubmitInvocation { .. } => Step::Close,
        V5ClientRequest::RecoverInvocationReceipt { .. } => Step::Reply(V5ServerResponse::Error {
            code: V5DaemonErrorCode::ReceiptNotFound,
        }),
        // The daemon fences the key it has never seen.
        V5ClientRequest::FenceInvocationReceipt { receipt_key } => {
            Step::Reply(V5ServerResponse::Invocation {
                outcome: V5InvocationResponse::ReceiptPending {
                    receipt_key: receipt_key.clone(),
                    phase: V5InvocationPhase::CancelReserved,
                    accepted_epoch_ms: epoch_ms_now(),
                    original_budget_ms: 0,
                    cancel_requested: true,
                },
            })
        }
        other => panic!("unexpected frame {other:?}"),
    }));
    let router = canonical_daemon_router(fake.owner(), fake.workspace_hint());

    let refusal = call(&router, None).expect_err("lost submission is refused");

    assert_not_delivered(&refusal);
    assert_eq!(fake.submissions(), 1, "the call is never submitted again");
    let seen = fake.seen();
    let V5ClientRequest::SubmitInvocation { invocation } = &seen[0] else {
        panic!("first frame must be the submission: {seen:?}");
    };
    let fenced = seen
        .iter()
        .find_map(|request| match request {
            V5ClientRequest::FenceInvocationReceipt { receipt_key } => Some(receipt_key.clone()),
            _ => None,
        })
        .expect("the exact key is fenced");
    assert_eq!(
        fenced,
        receipt_key_for(invocation, &V5DaemonClient::from(fake.owner())).unwrap()
    );
}

/// rule: INV.WIRE.LOST-SUBMIT-TASK-ID
#[test]
fn recovery_without_a_receipt_whose_fence_meets_the_call_names_its_task() {
    // The frame overtook the fence: the cancel finds the call's own
    // receipt, so it may run, and the answer is its Task.
    let fake = FakeDaemon::start(Box::new(|_, request| match request {
        V5ClientRequest::SubmitInvocation { .. } => Step::Close,
        V5ClientRequest::RecoverInvocationReceipt { .. } => Step::Reply(V5ServerResponse::Error {
            code: V5DaemonErrorCode::ReceiptNotFound,
        }),
        V5ClientRequest::FenceInvocationReceipt { receipt_key } => {
            Step::Reply(V5ServerResponse::Invocation {
                outcome: V5InvocationResponse::ReceiptPending {
                    receipt_key: receipt_key.clone(),
                    phase: V5InvocationPhase::ReservedBegun,
                    accepted_epoch_ms: epoch_ms_now(),
                    original_budget_ms: 7_000,
                    cancel_requested: true,
                },
            })
        }
        other => panic!("unexpected frame {other:?}"),
    }));
    let router = canonical_daemon_router(fake.owner(), fake.workspace_hint());

    let task = lost_task(call(&router, None));

    assert_is_the_reserved_task(&task, &fake);
    assert_eq!(fake.submissions(), 1);
}

fn lost_cutoff_fake(foreign: bool) -> FakeDaemon {
    FakeDaemon::start(Box::new(move |_, request| match request {
        // The daemon is still working when the frontend cutoff passes: the
        // submit session sees nothing before the client gives up on it.
        V5ClientRequest::SubmitInvocation { .. } => {
            thread::sleep(Duration::from_millis(600));
            Step::Close
        }
        V5ClientRequest::RecoverInvocationReceipt { receipt_key } => {
            let task_id = if foreign {
                TaskId::new()
            } else {
                receipt_key.reserved_task_id()
            };
            Step::Reply(V5ServerResponse::Invocation {
                outcome: V5InvocationResponse::Task {
                    snapshot: V5DaemonTaskSnapshot::Working {
                        task_id,
                        invocation_id: receipt_key.invocation_id(),
                        receipt_key_digest: unica_execution::receipt_ledger::receipt_key_digest(
                            receipt_key,
                        ),
                        created_at_epoch_ms: epoch_ms_now(),
                        updated_at_epoch_ms: epoch_ms_now(),
                        ttl_ms: 3_600_000,
                        poll_interval_ms: 250,
                        version: 2,
                        cancel_requested: false,
                        awaits_interrupted_work: false,
                    },
                },
            })
        }
        other => panic!("unexpected frame {other:?}"),
    }))
}

/// rule: INV.APP.DAEMON-INVOCATION-HANDOFF
#[test]
fn response_lost_at_the_cutoff_is_recovered_into_the_handed_off_task() {
    let fake = lost_cutoff_fake(false);
    let router = canonical_daemon_router(fake.owner(), fake.workspace_hint());

    let started = Instant::now();
    let outcome = call(&router, Some(Duration::from_millis(300)));

    let Ok(CanonicalCallOutcome::Task(snapshot)) = outcome else {
        panic!("a lost response must recover the handed-off Task: {outcome:?}");
    };
    let reserved = fake
        .seen()
        .iter()
        .find_map(|request| match request {
            V5ClientRequest::SubmitInvocation { invocation } => Some(invocation.reserved_task_id()),
            _ => None,
        })
        .expect("the call was submitted");
    assert_eq!(snapshot.task_id(), reserved);
    assert!(
        started.elapsed()
            < Duration::from_millis(300) + RECOVERY_BUDGET + Duration::from_millis(500),
        "recovery stays inside its bounded window: {:?}",
        started.elapsed()
    );
    assert_eq!(fake.submissions(), 1, "recovery never resubmits");
}

/// Recovery answers only with the Task reserved for the exact key: a Task
/// of another invocation is a protocol refusal, never this call's Task.
/// rule: INV.APP.DAEMON-INVOCATION-HANDOFF
#[test]
fn a_recovered_task_of_another_key_is_refused() {
    let fake = lost_cutoff_fake(true);
    let router = canonical_daemon_router(fake.owner(), fake.workspace_hint());

    let outcome = call(&router, Some(Duration::from_millis(300)));

    assert!(
        !matches!(&outcome, Ok(CanonicalCallOutcome::Task(_))),
        "a foreign Task was taken as this call's: {outcome:?}"
    );
    assert_eq!(fake.submissions(), 1, "recovery never resubmits");
}

/// rule: INV.APP.DAEMON-INVOCATION-HANDOFF
/// rule: INV.WIRE.LOST-SUBMIT-TASK-ID
#[test]
fn recovery_window_closing_answers_the_task_reserved_for_the_call() {
    let fake = FakeDaemon::start(Box::new(move |_, request| match request {
        V5ClientRequest::SubmitInvocation { .. } => {
            thread::sleep(Duration::from_millis(500));
            Step::Close
        }
        // Recovery answers only after the recovery window is spent.
        V5ClientRequest::RecoverInvocationReceipt { .. } => {
            thread::sleep(RECOVERY_BUDGET + Duration::from_millis(300));
            Step::Close
        }
        other => panic!("unexpected frame {other:?}"),
    }));
    let router = canonical_daemon_router(fake.owner(), fake.workspace_hint());

    let task = lost_task(call(&router, Some(Duration::from_millis(300))));

    assert_is_the_reserved_task(&task, &fake);
    assert_eq!(fake.submissions(), 1);
}

#[test]
fn daemon_rejection_and_foreign_receipt_are_distinct_closed_refusals() {
    let fake = FakeDaemon::start(Box::new(|raw, request| match request {
        V5ClientRequest::SubmitInvocation { invocation }
            if invocation.arguments().contains_key("at") =>
        {
            Step::Reply(V5ServerResponse::Error {
                code: V5DaemonErrorCode::Overloaded,
            })
        }
        V5ClientRequest::SubmitInvocation { .. } => {
            let mut foreign = daemon_side_key(raw);
            foreign = ReceiptKey::new(
                InvocationId::new(),
                foreign.reserved_task_id(),
                foreign.request_identity(),
            );
            Step::Reply(direct_receipt(foreign, completed("foreign")))
        }
        other => panic!("unexpected frame {other:?}"),
    }));
    let router = canonical_daemon_router(fake.owner(), fake.workspace_hint());

    let rejected = call(&router, None).expect_err("overloaded daemon refuses");
    let foreign = (router.call)(
        V5ToolIdentity::Docs,
        &Map::new(),
        &unica_bootstrap::HostRequest::default(),
        deadline(None),
        CancellationToken::new().into(),
    )
    .expect_err("a foreign receipt is refused");

    assert_eq!(rejected.code, ErrorCode(TOOL_EXECUTION_ERROR));
    assert_eq!(
        rejected.message,
        "daemon invocation submission rejected: overloaded"
    );
    assert_eq!(
        foreign.data,
        Some(json!({"code": "daemon_protocol_failed"}))
    );
    thread::sleep(Duration::from_millis(50));
    assert!(
        !fake.seen().iter().any(|request| matches!(
            request,
            V5ClientRequest::AcknowledgeInvocationReceipt { .. }
        )),
        "neither refusal acknowledges anything"
    );
}

#[test]
fn task_operations_use_peer_sessions_bounded_by_the_frontend_cutoff() {
    let task_id = TaskId::new();
    let fake = FakeDaemon::start(Box::new(move |_, request| match request {
        V5ClientRequest::GetTask { task_id: asked } => {
            assert_eq!(*asked, task_id);
            Step::Reply(V5ServerResponse::Error {
                code: V5DaemonErrorCode::TaskExpired,
            })
        }
        V5ClientRequest::WaitTask { wait_ms, .. } => {
            assert!(
                *wait_ms <= 300,
                "wait shrank by the transport budget: {wait_ms}"
            );
            Step::Reply(V5ServerResponse::Pong)
        }
        V5ClientRequest::CancelTask { .. } => Step::Close,
        other => panic!("unexpected frame {other:?}"),
    }));
    let router = canonical_daemon_router(fake.owner(), fake.workspace_hint());

    let expired = (router.get)(task_id, deadline(None));
    let unexpected = (router.wait)(
        task_id,
        7_000,
        deadline(Some(Duration::from_millis(400))),
        CanonicalCancellation::default(),
    );
    let closed = (router.cancel)(task_id, deadline(None));
    let past = (router.get)(
        task_id,
        FrontendInvocationDeadline::new(Instant::now() - Duration::from_secs(8), None),
    );

    assert_eq!(
        expired,
        Err(V5TaskExchangeError::Protocol(
            V5DaemonErrorCode::TaskExpired
        ))
    );
    assert_eq!(unexpected, Err(V5TaskExchangeError::UnexpectedResponse));
    let Err(V5TaskExchangeError::Transport(closed)) = closed else {
        panic!("a closed response must retain its transport stage");
    };
    assert_eq!(
        closed.stage,
        unica_daemon_protocol::client_v5::TaskObservationStage::ReadResponse
    );
    assert!(closed.endpoint_generation_hash.is_some());
    let Err(V5TaskExchangeError::Transport(past)) = past else {
        panic!("an expired admission must retain its transport stage");
    };
    assert_eq!(
        past.stage,
        unica_daemon_protocol::client_v5::TaskObservationStage::AcquirePeer
    );
    assert!(past.cutoff_expired);
    assert!(past.endpoint_generation_hash.is_none());
}
