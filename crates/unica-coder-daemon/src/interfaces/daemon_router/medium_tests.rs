use super::test_support::{
    completed, daemon_side_key, direct_receipt, FakeDaemon, LiveDaemon, ScriptedService, Step,
};
use super::*;
use serde_json::json;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::thread;
use unica_daemon::protocol_v5::V5ClientRequest;
use unica_execution::receipt_ledger::{canonical_v5_terminal, ReceiptTerminalOutcome};
use unica_execution::receipt_runtime::response::V5PendingDirectReceipt;

fn arguments() -> Map<String, Value> {
    json!({"at": "main:Catalog.Товары"})
        .as_object()
        .expect("argument object")
        .clone()
}

fn deadline(host_remaining: Option<Duration>) -> FrontendInvocationDeadline {
    FrontendInvocationDeadline::new(Instant::now(), host_remaining)
}

fn call(
    router: &CanonicalDaemonRouter,
    host_remaining: Option<Duration>,
) -> Result<CanonicalCallOutcome, ErrorData> {
    (router.call)(
        V5ToolIdentity::View,
        &arguments(),
        &unica_bootstrap::HostRequest::default(),
        deadline(host_remaining),
        CancellationToken::new().into(),
    )
}

fn direct_result(outcome: Result<CanonicalCallOutcome, ErrorData>) -> CallToolResult {
    match outcome {
        Ok(CanonicalCallOutcome::Direct(result)) => result,
        Ok(CanonicalCallOutcome::Task(snapshot)) => {
            panic!("unexpected Task handoff: {snapshot:?}")
        }
        Err(error) => panic!("unexpected refusal: {error}"),
    }
}

/// A lost submit answers with the queued Task reserved for the call.
fn lost_task(outcome: Result<CanonicalCallOutcome, ErrorData>) -> V5DaemonTaskSnapshot {
    let task = match outcome {
        Ok(CanonicalCallOutcome::Task(task)) => task,
        Ok(CanonicalCallOutcome::Direct(result)) => panic!("unexpected direct: {result:?}"),
        Err(error) => panic!("unexpected refusal: {error}"),
    };
    assert_eq!(
        task.status(),
        unica_execution_contract::invocation::InvocationStatus::Queued
    );
    task
}

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
    // the same call as a new one (#1804).
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
    use unica_daemon::identity::{CoreIdentity, DaemonStateDirectory};
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
    let limit = unica_daemon::protocol_v5::MAX_V5_RESPONSE_LINE_BYTES;
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
                    super::test_support::write_frame(
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
        unica_daemon::client_v5::TaskObservationStage::ReadResponse
    );
    assert!(closed.endpoint_generation_hash.is_some());
    let Err(V5TaskExchangeError::Transport(past)) = past else {
        panic!("an expired admission must retain its transport stage");
    };
    assert_eq!(
        past.stage,
        unica_daemon::client_v5::TaskObservationStage::AcquirePeer
    );
    assert!(past.cutoff_expired);
    assert!(past.endpoint_generation_hash.is_none());
}

/// rule: INV.WIRE.LOST-SUBMIT-TASK-ID
#[test]
fn a_lost_task_the_daemon_does_not_know_answers_task_not_found() {
    // The answer is lost before the daemon could be asked: the call is
    // the Task reserved for it. A daemon that never took the call — or no
    // longer knows it after a restart — answers that id as not found.
    let fake = FakeDaemon::start(Box::new(|_, request| match request {
        V5ClientRequest::SubmitInvocation { .. } => Step::Close,
        V5ClientRequest::RecoverInvocationReceipt { .. } => Step::Close,
        other => panic!("unexpected frame {other:?}"),
    }));
    let lost = canonical_daemon_router(fake.owner(), fake.workspace_hint());
    let task = lost_task(call(&lost, None));

    let service = Arc::new(ScriptedService {
        delay: Duration::from_millis(10),
        known_long: false,
        outcome: Mutex::new(None),
        executions: AtomicUsize::new(0),
    });
    let daemon = LiveDaemon::start(service.clone());
    let router = canonical_daemon_router(daemon.owner(), daemon.workspace_hint.clone());
    let answer = (router.get)(task.task_id(), deadline(None));
    assert!(
        matches!(
            answer,
            Err(V5TaskExchangeError::Protocol(
                V5DaemonErrorCode::TaskNotFound
            ))
        ),
        "{answer:?}"
    );
    assert_eq!(service.executions.load(Ordering::SeqCst), 0);
    drop(router);
    daemon.finish();
}

/// rule: INV.APP.DAEMON-INVOCATION-HANDOFF
/// rule: INV.WIRE.DAEMON-INITIAL-SUBMIT-RESULT
#[test]
fn live_daemon_hands_inline_work_over_the_cutoff_to_a_task_the_same_attempt_completes() {
    let service = Arc::new(ScriptedService {
        delay: Duration::from_millis(9_000),
        known_long: false,
        outcome: Mutex::new(None),
        executions: AtomicUsize::new(0),
    });
    let daemon = LiveDaemon::start(service.clone());
    let router = canonical_daemon_router(daemon.owner(), daemon.workspace_hint.clone());

    let started = Instant::now();
    let outcome = call(&router, None);
    let answered_after = started.elapsed();

    let Ok(CanonicalCallOutcome::Task(snapshot)) = outcome else {
        panic!("inline work past the cutoff must become a Task: {outcome:?}");
    };
    assert!(
        snapshot.completed_result().is_none(),
        "the handoff answers before the attempt finishes: {snapshot:?}"
    );
    // The daemon hands off at its own seventh second and the frontend
    // still gets the answer inside its transport window plus recovery.
    assert!(
        answered_after
            < INVOCATION_HANDOFF_WINDOW + RESPONSE_SERIALIZATION_MARGIN + RECOVERY_BUDGET,
        "the Task answered at {answered_after:?}"
    );
    assert!(
        answered_after >= INVOCATION_HANDOFF_WINDOW - Duration::from_millis(500),
        "a direct attempt is not shortened before the cutoff: {answered_after:?}"
    );

    let task_id = snapshot.task_id();
    let settle_by = Instant::now() + Duration::from_secs(20);
    let terminal = loop {
        let observed = (router.wait)(
            task_id,
            2_000,
            deadline(None),
            CanonicalCancellation::default(),
        )
        .expect("wait on the handed-off Task");
        if observed.completed_result().is_some() {
            break observed;
        }
        assert!(
            Instant::now() < settle_by,
            "the Task never completed: {observed:?}"
        );
    };
    assert_eq!(
        terminal
            .completed_result()
            .map(|result| result.summary.as_str()),
        Some("executed"),
        "the worker's own outcome becomes the Task terminal: {terminal:?}"
    );
    assert_eq!(
        service.executions.load(Ordering::SeqCst),
        1,
        "the handoff keeps the only attempt; nothing is re-executed"
    );
    drop(router);
    daemon.finish();
}

/// rule: INV.APP.DAEMON-INVOCATION-HANDOFF
#[test]
fn live_daemon_hands_a_failing_inline_attempt_over_the_cutoff_and_the_task_fails_once() {
    // The production runtime (no `receipt-ledger-test-support` feature) owns
    // the cutoff: a slow inline attempt is answered as a Task at the seventh
    // second, and the same single attempt's failure becomes that Task's
    // terminal — proving the deadline owner, not the harness, drives it.
    let service = Arc::new(ScriptedService {
        delay: Duration::from_millis(9_000),
        known_long: false,
        outcome: Mutex::new(Some(Err(
            unica_execution_contract::invocation::InvocationFailure::new(
                "provider_exploded",
                "/private/workspace bearer-secret",
            ),
        ))),
        executions: AtomicUsize::new(0),
    });
    let daemon = LiveDaemon::start(service.clone());
    let router = canonical_daemon_router(daemon.owner(), daemon.workspace_hint.clone());

    let started = Instant::now();
    let outcome = call(&router, None);
    let answered_after = started.elapsed();

    let Ok(CanonicalCallOutcome::Task(snapshot)) = outcome else {
        panic!("a slow inline attempt past the cutoff must become a Task: {outcome:?}");
    };
    assert!(
        snapshot.completed_result().is_none(),
        "the handoff answers before the attempt finishes: {snapshot:?}"
    );
    assert!(
        answered_after
            < INVOCATION_HANDOFF_WINDOW + RESPONSE_SERIALIZATION_MARGIN + RECOVERY_BUDGET,
        "the Task answered at {answered_after:?}"
    );
    assert!(
        answered_after >= INVOCATION_HANDOFF_WINDOW - Duration::from_millis(500),
        "a running attempt is not shortened before the cutoff: {answered_after:?}"
    );

    let task_id = snapshot.task_id();
    let settle_by = Instant::now() + Duration::from_secs(20);
    let terminal = loop {
        let observed = (router.wait)(
            task_id,
            2_000,
            deadline(None),
            CanonicalCancellation::default(),
        )
        .expect("wait on the handed-off Task");
        if !matches!(
            observed.status(),
            unica_execution_contract::invocation::InvocationStatus::Working
                | unica_execution_contract::invocation::InvocationStatus::Queued
        ) {
            break observed;
        }
        assert!(
            Instant::now() < settle_by,
            "the Task never reached its terminal: {observed:?}"
        );
    };
    assert_eq!(
        terminal.status(),
        unica_execution_contract::invocation::InvocationStatus::Failed,
        "the worker's own failure becomes the Task terminal: {terminal:?}"
    );
    // The safe failure reason never leaks the provider's private text.
    assert!(!format!("{terminal:?}").contains("bearer-secret"));
    assert_eq!(
        service.executions.load(Ordering::SeqCst),
        1,
        "the handoff keeps the only attempt; nothing is re-executed"
    );
    drop(router);
    daemon.finish();
}

/// rule: INV.APP.DAEMON-INVOCATION-OWNERSHIP
/// rule: INV.APP.NEW-INVOCATION-IDENTITY
/// rule: INV.APP.RECEIPT-ACKNOWLEDGEMENT
#[test]
fn live_daemon_executes_once_and_compacts_the_acknowledged_receipt_to_a_tombstone() {
    let service = Arc::new(ScriptedService {
        delay: Duration::ZERO,
        known_long: false,
        outcome: Mutex::new(None),
        executions: AtomicUsize::new(0),
    });
    let daemon = LiveDaemon::start(service.clone());
    let observed = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&observed);
    let owner = daemon.owner();
    let router = canonical_daemon_router_observed(
        owner,
        daemon.workspace_hint.clone(),
        Arc::new(move |key: &ReceiptKey| sink.lock().unwrap().push(key.clone())),
    );

    for _ in 0..2 {
        let result = direct_result(call(&router, None));
        assert_eq!(
            result
                .structured_content
                .as_ref()
                .and_then(|value| value["summary"].as_str()),
            Some("executed")
        );
    }
    let keys = observed.lock().unwrap().clone();
    assert_eq!(keys.len(), 2);
    assert_ne!(keys[0].invocation_id(), keys[1].invocation_id());
    assert_ne!(keys[0].reserved_task_id(), keys[1].reserved_task_id());
    let mut peer = daemon.owner();
    for key in keys {
        let recovered = peer
            .recover_invocation_receipt_before(key, Instant::now() + Duration::from_secs(5))
            .expect("recover after acknowledgement");
        assert!(
            matches!(
                recovered,
                V5ServerResponse::Invocation {
                    outcome: V5InvocationResponse::Acknowledged { .. }
                }
            ),
            "acknowledged receipt must compact to a tombstone: {recovered:?}"
        );
    }
    assert_eq!(
        service.executions.load(Ordering::SeqCst),
        2,
        "each new call executes once; recovery does not execute it again"
    );
    drop(peer);
    drop(router);
    daemon.finish();
}

/// #1309: a call far above the former 16 KiB frame reaches the service and
/// runs; one above the request bound never leaves the frontend and is
/// refused with the limit and its own size, not a bare `invalid_request`.
/// rule: INV.WIRE.DAEMON-REQUEST-FRAMING
#[test]
fn live_daemon_runs_a_large_call_and_refuses_one_above_the_bound_unsent() {
    use unica_daemon::protocol_v5::MAX_V5_REQUEST_LINE_BYTES;
    let service = Arc::new(ScriptedService {
        delay: Duration::ZERO,
        known_long: false,
        outcome: Mutex::new(None),
        executions: AtomicUsize::new(0),
    });
    let daemon = LiveDaemon::start(service.clone());
    let router = canonical_daemon_router(daemon.owner(), daemon.workspace_hint.clone());
    let call_with = |text: String| {
        // The scripted service ignores the address; only its size matters.
        let arguments = json!({"at": format!("main:CommonModule.{text}")});
        (router.call)(
            V5ToolIdentity::View,
            arguments.as_object().expect("argument object"),
            &unica_bootstrap::HostRequest::default(),
            deadline(None),
            CancellationToken::new().into(),
        )
    };

    // Cyrillic text: one character is two bytes on the wire.
    let large = "Процедура".repeat(64 * 1024);
    assert!(large.len() > 1024 * 1024);
    let result = direct_result(call_with(large));
    assert_eq!(
        result
            .structured_content
            .as_ref()
            .and_then(|value| value["summary"].as_str()),
        Some("executed")
    );
    assert_eq!(service.executions.load(Ordering::SeqCst), 1);

    let refusal = direct_result(call_with("x".repeat(MAX_V5_REQUEST_LINE_BYTES)));
    let refusal = refusal.structured_content.expect("canonical refusal");
    assert_eq!(refusal["ok"], json!(false));
    let diagnostic = &refusal["diagnostics"][0];
    assert_eq!(diagnostic["code"], json!("provider_limit_exceeded"));
    assert_eq!(diagnostic["outcome"], json!("fixCall"));
    // Hosts often show the model only the message: it names both numbers.
    let message = diagnostic["message"].as_str().expect("refusal message");
    assert!(message.contains(&format!("{MAX_V5_REQUEST_LINE_BYTES}-byte request limit")));
    let actual: usize = message
        .strip_prefix("the call is ")
        .and_then(|rest| rest.split(' ').next())
        .and_then(|number| number.parse().ok())
        .expect("the message starts with the frame size");
    assert!(actual > MAX_V5_REQUEST_LINE_BYTES, "actual {actual}");
    assert_eq!(
        service.executions.load(Ordering::SeqCst),
        1,
        "the oversized call must not reach the service"
    );

    // The refused call did not cost the daemon its session: the next one runs.
    direct_result(call(&router, None));
    assert_eq!(service.executions.load(Ordering::SeqCst), 2);
    drop(router);
    daemon.finish();
}

/// rule: INV.APP.DAEMON-TASK-PERSISTENCE
#[test]
fn live_daemon_failure_is_delivered_as_the_closed_reason_without_its_text() {
    // A workspace that changed under the call keeps that cause and its
    // remedy (#1338); any other failure stays the internal one.
    for (failure_code, code, message) in [
            (
                "provider_exploded",
                "invocation_failed",
                "daemon invocation stopped without a result. Repeat the call; if the problem persists, report this reason.",
            ),
            (
                unica_execution_contract::invocation::WORKSPACE_CHANGED_FAILURE,
                "workspace_changed",
                "the workspace changed while the call ran, so its result was not published. Repeat the call.",
            ),
        ] {
            let service = Arc::new(ScriptedService {
                delay: Duration::ZERO,
                known_long: false,
                outcome: Mutex::new(Some(Err(
                    unica_execution_contract::invocation::InvocationFailure::new(
                        failure_code,
                        "/private/workspace bearer-secret",
                    ),
                ))),
                executions: AtomicUsize::new(0),
            });
            let daemon = LiveDaemon::start(service.clone());
            let router = canonical_daemon_router(daemon.owner(), daemon.workspace_hint.clone());

            let refusal = call(&router, None).expect_err("failed execution is an error");

            assert_eq!(refusal.code, ErrorCode::INTERNAL_ERROR);
            assert_eq!(refusal.message, message);
            assert_eq!(refusal.data, Some(json!({"code": code})));
            assert!(!format!("{refusal:?}").contains("bearer-secret"));
            drop(router);
            daemon.finish();
        }
}

/// rule: INV.APP.DAEMON-INVOCATION-OWNERSHIP
#[test]
fn live_daemon_hands_slow_work_off_to_a_task_that_get_wait_and_cancel_observe() {
    // Known-long work is handed off before execution, so the Task path is
    // exercised without racing the seven-second cutoff on a loaded runner.
    let service = Arc::new(ScriptedService {
        delay: Duration::from_millis(300),
        known_long: true,
        outcome: Mutex::new(None),
        executions: AtomicUsize::new(0),
    });
    let daemon = LiveDaemon::start(service.clone());
    let router = canonical_daemon_router(daemon.owner(), daemon.workspace_hint.clone());

    let handoff = match call(&router, None) {
        Ok(CanonicalCallOutcome::Task(snapshot)) => snapshot,
        Ok(CanonicalCallOutcome::Direct(result)) => {
            panic!("slow work answered directly: {result:?}")
        }
        Err(error) => panic!("slow work refused: {error}"),
    };
    let task_id = handoff.task_id();
    let observed = (router.get)(task_id, deadline(None)).expect("get the handed-off task");
    let terminal = (router.wait)(
        task_id,
        5_000,
        deadline(None),
        CanonicalCancellation::default(),
    )
    .expect("wait for the terminal");
    let cancelled = (router.cancel)(task_id, deadline(None)).expect("cancel after terminal");

    assert!(!matches!(
        handoff.status(),
        unica_execution_contract::invocation::InvocationStatus::Completed
    ));
    assert_eq!(observed.task_id(), task_id);
    assert_eq!(
        terminal
            .completed_result()
            .map(|result| result.summary.as_str()),
        Some("executed")
    );
    assert_eq!(
        cancelled.status(),
        unica_execution_contract::invocation::InvocationStatus::Completed
    );
    assert_eq!(service.executions.load(Ordering::SeqCst), 1);
    drop(router);
    daemon.finish();
}

/// Known-long work that runs until the test releases it and records
/// whether its cancellation reached it.
struct GatedService {
    executions: AtomicUsize,
    cancelled: AtomicUsize,
    released: std::sync::atomic::AtomicBool,
    /// Whether `apply` waits for the release too or ends at once.
    gate_mutations: bool,
}

impl GatedService {
    fn new() -> Arc<Self> {
        Self::gating_mutations(true)
    }

    fn gating_mutations(gate_mutations: bool) -> Arc<Self> {
        Arc::new(Self {
            executions: AtomicUsize::new(0),
            cancelled: AtomicUsize::new(0),
            released: std::sync::atomic::AtomicBool::new(false),
            gate_mutations,
        })
    }

    fn release(&self) {
        self.released.store(true, Ordering::SeqCst);
    }

    /// A known-long call answers with its Task before the execution
    /// thread enters `execute`, so the count is awaited, not read once.
    fn assert_executions_started(&self, expected: usize) {
        let started = Instant::now();
        while self.executions.load(Ordering::SeqCst) < expected
            && started.elapsed() < Duration::from_secs(10)
        {
            thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(self.executions.load(Ordering::SeqCst), expected);
    }
}

impl crate::infrastructure::daemon::server::CanonicalInvocationService for GatedService {
    fn prepare(
        &self,
        _invocation: &crate::infrastructure::daemon::server::ActorBoundInvocation,
    ) -> Result<unica_execution::receipt_runtime::ports::ExecutionClass, Box<DomainResultAlias>>
    {
        use unica_execution::receipt_runtime::ports::{ExecutionClass, KnownLongReason};
        Ok(ExecutionClass::KnownLong(KnownLongReason::ExternalProcess))
    }

    fn execute(
        &self,
        invocation: &crate::infrastructure::daemon::server::ActorBoundExecution,
        cancellation: CancellationToken,
    ) -> Result<DomainResultAlias, unica_execution_contract::invocation::InvocationFailure> {
        self.executions.fetch_add(1, Ordering::SeqCst);
        if !self.gate_mutations && invocation.arguments().contains_key("ops") {
            return Ok(DomainResultAlias::success("applied"));
        }
        let started = Instant::now();
        while !self.released.load(Ordering::SeqCst) && started.elapsed() < Duration::from_secs(20) {
            if cancellation.is_cancelled() {
                self.cancelled.fetch_add(1, Ordering::SeqCst);
                return Err(
                    unica_execution_contract::invocation::InvocationFailure::new(
                        "cancelled",
                        "cancelled",
                    ),
                );
            }
            thread::sleep(Duration::from_millis(5));
        }
        Ok(DomainResultAlias::success("shared traversal"))
    }
}

type DomainResultAlias = unica_execution_contract::invocation::DomainResult;

/// Loses the first submit answer and every recovery answer, as an
/// overloaded transport does: the frontend runs out of its recovery
/// window although the daemon holds the call.
#[derive(Default)]
struct LostSubmitHooks {
    submits: AtomicUsize,
    lose_recoveries: std::sync::atomic::AtomicBool,
}

impl
    unica_execution::receipt_runtime::hooks::V5RuntimeHooks<
        crate::infrastructure::daemon::server::CanonicalInvocationHost,
    > for LostSubmitHooks
{
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn submit_response_disconnect(&self) -> bool {
        self.submits.fetch_add(1, Ordering::SeqCst) == 0
    }

    fn recover_response_disconnect(&self) -> bool {
        self.lose_recoveries.load(Ordering::SeqCst)
    }
}

fn call_tool(
    router: &CanonicalDaemonRouter,
    tool: V5ToolIdentity,
    arguments: &Map<String, Value>,
) -> Result<CanonicalCallOutcome, ErrorData> {
    (router.call)(
        tool,
        arguments,
        &unica_bootstrap::HostRequest::default(),
        deadline(None),
        CancellationToken::new().into(),
    )
}

fn task_of(outcome: Result<CanonicalCallOutcome, ErrorData>) -> V5DaemonTaskSnapshot {
    match outcome {
        Ok(CanonicalCallOutcome::Task(snapshot)) => snapshot,
        Ok(CanonicalCallOutcome::Direct(result)) => panic!("unexpected direct: {result:?}"),
        Err(error) => panic!("unexpected refusal: {error}"),
    }
}

fn wait_terminal(router: &CanonicalDaemonRouter, task_id: TaskId) -> V5DaemonTaskSnapshot {
    let started = Instant::now();
    loop {
        let snapshot = (router.wait)(
            task_id,
            2_000,
            deadline(None),
            CanonicalCancellation::default(),
        )
        .expect("wait for the Task");
        if matches!(
            snapshot.status(),
            unica_execution_contract::invocation::InvocationStatus::Completed
                | unica_execution_contract::invocation::InvocationStatus::Failed
                | unica_execution_contract::invocation::InvocationStatus::Cancelled
        ) {
            return snapshot;
        }
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "Task never ended"
        );
    }
}

/// rule: INV.APP.READ-CALL-JOINING
/// rule: INV.WIRE.LOST-SUBMIT-TASK-ID
#[test]
fn lost_submit_names_its_task_and_a_retry_joins_it_instead_of_running_again() {
    let service = GatedService::new();
    let hooks = Arc::new(LostSubmitHooks::default());
    hooks.lose_recoveries.store(true, Ordering::SeqCst);
    let daemon = LiveDaemon::start_with_aged_sources(service.clone(), Some(hooks.clone()));
    let router = canonical_daemon_router(daemon.owner(), daemon.workspace_hint.clone());
    let search = json!({"query": "Товары"}).as_object().unwrap().clone();

    // The daemon hands the search off to a Task, and the host never
    // learns it: the submit answer and every recovery answer are lost.
    let task = lost_task(call_tool(&router, V5ToolIdentity::Search, &search));
    hooks.lose_recoveries.store(false, Ordering::SeqCst);
    let named = task.task_id();

    // The agent repeats the call anyway: it joins the running Task under
    // its own id instead of starting the same traversal again.
    let retry = task_of(call_tool(&router, V5ToolIdentity::Search, &search));
    assert_ne!(retry.task_id(), named);
    let first = (router.get)(named, deadline(None)).expect("the named Task is readable");
    assert_eq!(first.task_id(), named);

    service.release();
    let first = wait_terminal(&router, named);
    let joined = wait_terminal(&router, retry.task_id());
    assert_eq!(
        first
            .completed_result()
            .map(|result| result.summary.as_str()),
        Some("shared traversal")
    );
    assert_eq!(first.completed_result(), joined.completed_result());
    assert_eq!(joined.task_id(), retry.task_id());
    assert_eq!(
        service.executions.load(Ordering::SeqCst),
        1,
        "the retry never ran the traversal again"
    );
    drop(router);
    daemon.finish();
}

/// rule: INV.WIRE.LOST-SUBMIT-TASK-ID
#[test]
fn a_read_whose_answer_was_lost_inline_is_readable_by_its_task_id() {
    // Inline work that ends directly: its answer is lost and no Task
    // ever exists, yet the id of the reserved Task returns the result.
    let service = Arc::new(ScriptedService {
        delay: Duration::from_millis(10),
        known_long: false,
        outcome: Mutex::new(None),
        executions: AtomicUsize::new(0),
    });
    let hooks = Arc::new(LostSubmitHooks::default());
    hooks.lose_recoveries.store(true, Ordering::SeqCst);
    let daemon = LiveDaemon::start_with_aged_sources(service.clone(), Some(hooks.clone()));
    let router = canonical_daemon_router(daemon.owner(), daemon.workspace_hint.clone());

    let named = lost_task(call(&router, None)).task_id();
    let terminal = wait_terminal(&router, named);

    assert_eq!(terminal.task_id(), named);
    assert_eq!(
        terminal
            .completed_result()
            .map(|result| result.summary.as_str()),
        Some("executed")
    );
    assert_eq!(service.executions.load(Ordering::SeqCst), 1);
    drop(router);
    daemon.finish();
}

/// rule: INV.APP.READ-CALL-JOINING
#[test]
fn only_an_identical_read_joins_and_a_mutation_never_does() {
    let service = GatedService::new();
    let daemon = LiveDaemon::start_with_aged_sources(service.clone(), None);
    let router = canonical_daemon_router(daemon.owner(), daemon.workspace_hint.clone());
    let same = json!({"query": "Товары"}).as_object().unwrap().clone();
    let other = json!({"query": "Цены"}).as_object().unwrap().clone();
    let edit = json!({
        "at": "main:Catalog.Товары",
        "ops": [{"op": "props.set", "args": {"values": {"Comment": "joined"}}}]
    })
    .as_object()
    .unwrap()
    .clone();

    let leader = task_of(call_tool(&router, V5ToolIdentity::Search, &same));
    let joined = task_of(call_tool(&router, V5ToolIdentity::Search, &same));
    assert_ne!(joined.task_id(), leader.task_id());
    service.assert_executions_started(1);
    let different = task_of(call_tool(&router, V5ToolIdentity::Search, &other));
    let first_apply = task_of(call_tool(&router, V5ToolIdentity::Apply, &edit));
    let second_apply = task_of(call_tool(&router, V5ToolIdentity::Apply, &edit));
    // A read submitted after a mutation sees its own traversal, and while
    // the mutation runs no read becomes a leader another read could join.
    let after_mutation = task_of(call_tool(&router, V5ToolIdentity::Search, &same));
    let during_mutation = task_of(call_tool(&router, V5ToolIdentity::Search, &same));

    service.release();
    for task in [
        &leader,
        &joined,
        &different,
        &first_apply,
        &second_apply,
        &after_mutation,
        &during_mutation,
    ] {
        wait_terminal(&router, task.task_id());
    }
    assert_eq!(
            service.executions.load(Ordering::SeqCst),
            6,
            "leader, different read, both applies and both reads after them ran; the joined read did not"
        );
    drop(router);
    daemon.finish();
}

/// rule: INV.APP.READ-CALL-JOINING
#[test]
fn one_holder_cancelling_leaves_the_shared_work_to_the_others_until_all_cancel() {
    let service = GatedService::new();
    let daemon = LiveDaemon::start_with_aged_sources(service.clone(), None);
    let router = canonical_daemon_router(daemon.owner(), daemon.workspace_hint.clone());
    let same = json!({"query": "Товары"}).as_object().unwrap().clone();

    let leader = task_of(call_tool(&router, V5ToolIdentity::Search, &same));
    let first = task_of(call_tool(&router, V5ToolIdentity::Search, &same));
    let second = task_of(call_tool(&router, V5ToolIdentity::Search, &same));
    service.assert_executions_started(1);

    // One joined holder and then the leader's own holder let go: each sees
    // its own cancellation, the work goes on for the remaining holder.
    let first_cancelled = (router.cancel)(first.task_id(), deadline(None)).unwrap();
    let leader_cancelled = (router.cancel)(leader.task_id(), deadline(None)).unwrap();
    for (snapshot, holder) in [(&first_cancelled, &first), (&leader_cancelled, &leader)] {
        assert_eq!(snapshot.task_id(), holder.task_id());
        assert!(
            snapshot.version() > holder.version(),
            "the holder's own cancellation is a newer state than the one it saw"
        );
        assert_eq!(
            snapshot.status(),
            unica_execution_contract::invocation::InvocationStatus::Cancelled
        );
    }
    let remaining = (router.get)(second.task_id(), deadline(None)).unwrap();
    assert!(!remaining.cancel_requested());
    assert!(matches!(
        remaining.status(),
        unica_execution_contract::invocation::InvocationStatus::Working
            | unica_execution_contract::invocation::InvocationStatus::Queued
    ));
    thread::sleep(Duration::from_millis(100));
    assert_eq!(service.cancelled.load(Ordering::SeqCst), 0);

    // The last holder lets go: now the shared work itself is cancelled.
    let second_cancelled = (router.cancel)(second.task_id(), deadline(None)).unwrap();
    assert_eq!(
        second_cancelled.status(),
        unica_execution_contract::invocation::InvocationStatus::Cancelled
    );
    let started = Instant::now();
    while service.cancelled.load(Ordering::SeqCst) == 0 {
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "the shared work never saw its cancellation"
        );
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(service.executions.load(Ordering::SeqCst), 1);
    drop(router);
    daemon.finish();
}

struct FixedHolderEpoch;

impl unica_execution::invocation_store::EpochMillisClock for FixedHolderEpoch {
    fn now_epoch_millis(&self) -> u64 {
        1_000
    }
}

/// Settle the held service and listener even when an assertion is RED.
struct JoinedClockFixture {
    router: Option<CanonicalDaemonRouter>,
    daemon: Option<LiveDaemon>,
    service: Arc<GatedService>,
}

impl JoinedClockFixture {
    fn start() -> Self {
        let service = GatedService::new();
        let daemon = LiveDaemon::start_with_aged_sources_and_clock(
            service.clone(),
            None,
            Some(Arc::new(FixedHolderEpoch)),
        );
        let router = canonical_daemon_router(daemon.owner(), daemon.workspace_hint.clone());
        Self {
            router: Some(router),
            daemon: Some(daemon),
            service,
        }
    }

    fn router(&self) -> &CanonicalDaemonRouter {
        self.router.as_ref().unwrap()
    }
}

impl Drop for JoinedClockFixture {
    fn drop(&mut self) {
        self.service.release();
        drop(self.router.take());
        if let Some(daemon) = self.daemon.take() {
            daemon.finish();
        }
    }
}

/// rule: INV.APP.READ-CALL-JOINING
#[test]
fn holder_cancel_before_completion_stays_cancelled_at_the_same_epoch() {
    use unica_execution_contract::invocation::InvocationStatus;
    for cancel_leader in [true, false] {
        let fixture = JoinedClockFixture::start();
        let router = fixture.router();
        let same = json!({"query": "Товары"}).as_object().unwrap().clone();
        let leader = task_of(call_tool(router, V5ToolIdentity::Search, &same));
        let joined = task_of(call_tool(router, V5ToolIdentity::Search, &same));
        fixture.service.assert_executions_started(1);
        let (leaving, remaining) = if cancel_leader {
            (&leader, &joined)
        } else {
            (&joined, &leader)
        };
        let cancelled = (router.cancel)(leaving.task_id(), deadline(None)).unwrap();
        assert_eq!(cancelled.task_id(), leaving.task_id());
        assert_eq!(cancelled.invocation_id(), leaving.invocation_id());
        assert_eq!(
            cancelled.status(),
            InvocationStatus::Cancelled,
            "the holder's cancellation must complete before producer release"
        );
        assert!(cancelled.cancel_requested());
        let V5DaemonTaskSnapshot::Cancelled {
            terminal_epoch_ms, ..
        } = &cancelled
        else {
            unreachable!();
        };
        assert_eq!(*terminal_epoch_ms, 1_000);
        fixture.service.release();

        let kept = wait_terminal(router, remaining.task_id());
        assert_eq!(
            kept.completed_result()
                .map(|result| result.summary.as_str()),
            Some("shared traversal")
        );
        let V5DaemonTaskSnapshot::Completed {
            terminal_epoch_ms, ..
        } = &kept
        else {
            unreachable!();
        };
        assert_eq!(*terminal_epoch_ms, 1_000);
        let after = wait_terminal(router, leaving.task_id());
        assert_eq!(after.status(), InvocationStatus::Cancelled,
                "a completed holder cancellation must survive later shared completion in the same millisecond");
        assert_eq!(after.task_id(), leaving.task_id());
        assert_eq!(after.invocation_id(), leaving.invocation_id());
        assert_eq!(fixture.service.executions.load(Ordering::SeqCst), 1);
        assert_eq!(fixture.service.cancelled.load(Ordering::SeqCst), 0);
    }
}

/// rule: INV.APP.READ-CALL-JOINING
#[test]
fn shared_completion_before_holder_cancel_keeps_result_at_the_same_epoch() {
    use unica_execution_contract::invocation::InvocationStatus;
    for cancel_leader in [true, false] {
        let fixture = JoinedClockFixture::start();
        let router = fixture.router();
        let same = json!({"query": "Товары"}).as_object().unwrap().clone();
        let leader = task_of(call_tool(router, V5ToolIdentity::Search, &same));
        let joined = task_of(call_tool(router, V5ToolIdentity::Search, &same));
        fixture.service.assert_executions_started(1);
        fixture.service.release();
        let (leaving, remaining) = if cancel_leader {
            (&leader, &joined)
        } else {
            (&joined, &leader)
        };
        let completed = wait_terminal(router, leaving.task_id());
        assert_eq!(completed.status(), InvocationStatus::Completed);
        let V5DaemonTaskSnapshot::Completed {
            terminal_epoch_ms, ..
        } = &completed
        else {
            unreachable!();
        };
        assert_eq!(*terminal_epoch_ms, 1_000);
        let late_cancel = (router.cancel)(leaving.task_id(), deadline(None)).unwrap();
        assert_eq!(late_cancel, completed,
                "a holder cancelling after completion must retain the original terminal even at the same millisecond");
        assert_eq!(wait_terminal(router, leaving.task_id()), completed);
        let kept = wait_terminal(router, remaining.task_id());
        assert_eq!(kept.completed_result(), completed.completed_result());
        assert_eq!(fixture.service.executions.load(Ordering::SeqCst), 1);
        assert_eq!(fixture.service.cancelled.load(Ordering::SeqCst), 0);
    }
}

/// rule: INV.APP.READ-CALL-JOINING
#[test]
fn a_joined_holder_keeps_the_result_after_another_holder_cancelled() {
    let service = GatedService::new();
    let daemon = LiveDaemon::start_with_aged_sources(service.clone(), None);
    let router = canonical_daemon_router(daemon.owner(), daemon.workspace_hint.clone());
    let same = json!({"query": "Товары"}).as_object().unwrap().clone();

    let leader = task_of(call_tool(&router, V5ToolIdentity::Search, &same));
    let joined = task_of(call_tool(&router, V5ToolIdentity::Search, &same));
    (router.cancel)(leader.task_id(), deadline(None)).unwrap();
    service.release();

    let kept = wait_terminal(&router, joined.task_id());
    assert_eq!(
        kept.completed_result()
            .map(|result| result.summary.as_str()),
        Some("shared traversal")
    );
    assert_eq!(
        wait_terminal(&router, leader.task_id()).status(),
        unica_execution_contract::invocation::InvocationStatus::Cancelled
    );
    assert_eq!(service.cancelled.load(Ordering::SeqCst), 0);
    drop(router);
    daemon.finish();
}

/// rule: INV.APP.READ-CALL-JOINING
#[test]
fn an_exact_repeat_of_a_call_with_its_own_receipt_never_becomes_a_joiner() {
    let service = GatedService::gating_mutations(false);
    let daemon = LiveDaemon::start_with_aged_sources(service.clone(), None);
    let router = canonical_daemon_router(daemon.owner(), daemon.workspace_hint.clone());
    let same = json!({"query": "Товары"}).as_object().unwrap().clone();
    let edit = json!({
        "at": "main:Catalog.Товары",
        "ops": [{"op": "props.set", "args": {"values": {"Comment": "between"}}}]
    })
    .as_object()
    .unwrap()
    .clone();
    let workspace = unica_bootstrap::HostWorkspaceContext::from(daemon.workspace_hint.clone())
        .resolve_request(&unica_bootstrap::HostRequest::default())
        .unwrap();
    let own = V5InvocationRequest::new(
        InvocationId::new(),
        TaskId::new(),
        V5ToolIdentity::Search,
        same.clone(),
        workspace,
        5_000,
    )
    .unwrap();
    let submit = |invocation: V5InvocationRequest| {
        daemon
            .owner()
            .submit_invocation_before(invocation, Instant::now() + Duration::from_secs(10))
            .unwrap()
    };

    // The call reserves its own receipt and becomes a Task. A mutation
    // then ends, and a later identical read leads the next joins.
    let first = match submit(own.clone()) {
        V5ServerResponse::Invocation {
            outcome: V5InvocationResponse::Task { snapshot },
        } => snapshot,
        other => panic!("the first call becomes its own Task: {other:?}"),
    };
    let applied = task_of(call_tool(&router, V5ToolIdentity::Apply, &edit));
    wait_terminal(&router, applied.task_id());
    let later = task_of(call_tool(&router, V5ToolIdentity::Search, &same));
    service.assert_executions_started(3);

    // The exact repeat answers from its own receipt, never as a holder of
    // the later read: cancelling it still cancels its own work.
    let _ = submit(own);
    (router.cancel)(first.task_id(), deadline(None)).unwrap();
    let started = Instant::now();
    while service.cancelled.load(Ordering::SeqCst) == 0 {
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "the repeat was joined to another Task instead of reading its own"
        );
        thread::sleep(Duration::from_millis(10));
    }
    service.release();
    assert_eq!(
        wait_terminal(&router, later.task_id())
            .completed_result()
            .map(|result| result.summary.as_str()),
        Some("shared traversal")
    );
    assert_eq!(service.executions.load(Ordering::SeqCst), 3);
    drop(router);
    daemon.finish();
}

fn raw_invocation(
    daemon: &LiveDaemon,
    tool: V5ToolIdentity,
    arguments: &Map<String, Value>,
    budget_ms: u64,
) -> V5InvocationRequest {
    let workspace = unica_bootstrap::HostWorkspaceContext::from(daemon.workspace_hint.clone())
        .resolve_request(&unica_bootstrap::HostRequest::default())
        .unwrap();
    V5InvocationRequest::new(
        InvocationId::new(),
        TaskId::new(),
        tool,
        arguments.clone(),
        workspace,
        budget_ms,
    )
    .unwrap()
}

fn raw_key(daemon: &LiveDaemon, invocation: &V5InvocationRequest) -> ReceiptKey {
    receipt_key_for(invocation, &V5DaemonClient::from(daemon.owner())).unwrap()
}

/// rule: INV.APP.READ-CALL-JOINING
#[test]
fn an_external_edit_after_the_leader_started_makes_the_repeat_run_fresh() {
    let service = GatedService::new();
    let daemon = LiveDaemon::start_with_aged_sources(service.clone(), None);
    let router = canonical_daemon_router(daemon.owner(), daemon.workspace_hint.clone());
    let check = json!({"query": "Модуль"}).as_object().unwrap().clone();

    let leader = task_of(call_tool(&router, V5ToolIdentity::Search, &check));
    // Nothing changed: the identical check joins the running walk.
    let joined = task_of(call_tool(&router, V5ToolIdentity::Search, &check));
    service.assert_executions_started(1);

    // The agent edits a module directly, not through Unica.
    std::fs::write(
        std::path::Path::new(&daemon.workspace_hint).join("src/CommonModules/Module.bsl"),
        "// v2\n",
    )
    .unwrap();
    let fresh = task_of(call_tool(&router, V5ToolIdentity::Search, &check));
    assert_ne!(fresh.task_id(), leader.task_id());
    let started = Instant::now();
    while service.executions.load(Ordering::SeqCst) < 2 {
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "the check after the edit joined the walk that began before it"
        );
        thread::sleep(Duration::from_millis(10));
    }
    service.release();
    for task in [&leader, &joined, &fresh] {
        wait_terminal(&router, task.task_id());
    }
    assert_eq!(service.executions.load(Ordering::SeqCst), 2);
    drop(router);
    daemon.finish();
}

/// rule: INV.APP.READ-CALL-JOINING
#[test]
fn a_frame_that_arrives_cancelled_never_leads_identical_reads() {
    let service = Arc::new(ScriptedService {
        delay: Duration::from_millis(10),
        known_long: false,
        outcome: Mutex::new(None),
        executions: AtomicUsize::new(0),
    });
    let daemon = LiveDaemon::start_with_aged_sources(service.clone(), None);
    let router = canonical_daemon_router(daemon.owner(), daemon.workspace_hint.clone());

    // An early cancel for the exact key, then the frame itself; its
    // budget is spent by the time the identical read arrives.
    let late = raw_invocation(&daemon, V5ToolIdentity::View, &arguments(), 1_000);
    let key = raw_key(&daemon, &late);
    V5DaemonProcessOwner::cancel_at_endpoint(daemon.owner().control_endpoint(), key).unwrap();
    let late = daemon
        .owner()
        .submit_invocation_before(late, Instant::now() + Duration::from_secs(10))
        .unwrap();
    assert!(
        matches!(late, V5ServerResponse::Invocation { .. }),
        "the cancelled frame is answered: {late:?}"
    );
    thread::sleep(Duration::from_millis(1_200));

    let started = Instant::now();
    let result = direct_result(call(&router, None));
    assert_eq!(result.is_error, Some(false));
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "the identical read waited for a leader that never runs: {:?}",
        started.elapsed()
    );
    assert_eq!(service.executions.load(Ordering::SeqCst), 1);
    drop(router);
    daemon.finish();
}

/// The daemon answers a first ordinary submit with its result or its
/// Task, never with a pending receipt: inline, known as long, or failing.
/// Each call runs once. Past its cutoff the answer is the Task, as the
/// handoff tests show through the router.
/// rule: INV.WIRE.DAEMON-INITIAL-SUBMIT-RESULT
#[test]
fn a_first_submit_is_answered_directly_or_by_its_task_never_pending() {
    let failure = unica_execution_contract::invocation::InvocationFailure::new(
        "scripted",
        "the scripted call failed",
    );
    for (case, delay, known_long, outcome, budget_ms, expect_task) in [
        ("inline", Duration::ZERO, false, None, 5_000, false),
        ("known long", Duration::ZERO, true, None, 5_000, true),
        (
            "failing inline",
            Duration::ZERO,
            false,
            Some(Err(failure)),
            5_000,
            false,
        ),
    ] {
        let service = Arc::new(ScriptedService {
            delay,
            known_long,
            outcome: Mutex::new(outcome),
            executions: AtomicUsize::new(0),
        });
        let daemon = LiveDaemon::start(service.clone());
        let search = json!({"query": "Товары"}).as_object().unwrap().clone();
        let invocation = raw_invocation(&daemon, V5ToolIdentity::Search, &search, budget_ms);
        let answer = daemon
            .owner()
            .submit_invocation_before(invocation, Instant::now() + Duration::from_secs(10))
            .unwrap_or_else(|error| panic!("{case}: the first submit was not answered: {error:?}"));
        match &answer {
            V5ServerResponse::Invocation {
                outcome: V5InvocationResponse::Task { .. },
            } => assert!(expect_task, "{case}: {answer:?}"),
            V5ServerResponse::Invocation {
                outcome: V5InvocationResponse::Direct { .. },
            } => assert!(!expect_task, "{case}: {answer:?}"),
            other => panic!("{case}: a first submit must not answer {other:?}"),
        }
        let started = Instant::now();
        while service.executions.load(Ordering::SeqCst) == 0
            && started.elapsed() < Duration::from_secs(5)
        {
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(service.executions.load(Ordering::SeqCst), 1, "{case}");
    }
}

/// rule: INV.WIRE.DAEMON-INITIAL-SUBMIT-RESULT
/// rule: INV.WIRE.LOST-SUBMIT-TASK-ID
#[test]
fn a_fenced_frame_never_runs_and_a_held_one_is_answered_by_its_state() {
    let service = GatedService::new();
    let daemon = LiveDaemon::start_with_aged_sources(service.clone(), None);
    let router = canonical_daemon_router(daemon.owner(), daemon.workspace_hint.clone());
    let search = json!({"query": "Товары"}).as_object().unwrap().clone();

    let fenced = raw_invocation(&daemon, V5ToolIdentity::Search, &search, 5_000);
    let fenced_key = raw_key(&daemon, &fenced);
    let answer = daemon
        .owner()
        .fence_invocation_receipt_before(
            fenced_key.clone(),
            Instant::now() + Duration::from_secs(5),
        )
        .unwrap();
    assert!(matches!(
        answer,
        V5ServerResponse::Invocation {
            outcome: V5InvocationResponse::ReceiptPending {
                phase: V5InvocationPhase::CancelReserved,
                cancel_requested: true,
                ..
            }
        }
    ));
    let late = daemon
        .owner()
        .submit_invocation_before(fenced, Instant::now() + Duration::from_secs(10))
        .unwrap();
    assert!(
        !matches!(late, V5ServerResponse::Invocation { .. }),
        "{late:?}"
    );
    assert_eq!(service.executions.load(Ordering::SeqCst), 0);
    assert!((router.get)(fenced_key.reserved_task_id(), deadline(None)).is_err());

    // A call the daemon already holds is not fenced: it keeps running.
    let held = task_of(call_tool(&router, V5ToolIdentity::Search, &search));
    service.release();
    assert_eq!(
        wait_terminal(&router, held.task_id())
            .completed_result()
            .map(|result| result.summary.as_str()),
        Some("shared traversal")
    );
    drop(router);
    daemon.finish();
}

/// rule: INV.WIRE.LOST-SUBMIT-TASK-ID
#[test]
fn the_task_id_of_a_call_still_inline_can_be_cancelled() {
    let service = Arc::new(ScriptedService {
        delay: Duration::from_millis(1_500),
        known_long: false,
        outcome: Mutex::new(None),
        executions: AtomicUsize::new(0),
    });
    let daemon = LiveDaemon::start_with_aged_sources(service.clone(), None);
    let router = canonical_daemon_router(daemon.owner(), daemon.workspace_hint.clone());
    let inline = raw_invocation(&daemon, V5ToolIdentity::View, &arguments(), 7_000);
    let task_id = inline.reserved_task_id();
    let mut owner = daemon.owner();
    let submit = thread::spawn(move || {
        owner.submit_invocation_before(inline, Instant::now() + Duration::from_secs(10))
    });
    let started = Instant::now();
    while service.executions.load(Ordering::SeqCst) == 0 {
        assert!(started.elapsed() < Duration::from_secs(5), "never started");
        thread::sleep(Duration::from_millis(5));
    }

    let cancelled = (router.cancel)(task_id, deadline(None))
        .expect("the reserved id of an inline call is cancellable");

    assert_eq!(cancelled.task_id(), task_id);
    assert!(
        cancelled.cancel_requested()
            || cancelled.status()
                == unica_execution_contract::invocation::InvocationStatus::Cancelled,
        "{cancelled:?}"
    );
    let _ = submit.join().unwrap();
    drop(router);
    daemon.finish();
}
