//! Separate causal control: the accepted staged transfer outlives its original
//! caller endpoint; the old observation fails while a fresh observation sees its winner.
use super::*;
use std::io::Write as _;

struct HoldBeforeTaskStoreCreate {
    first: AtomicBool,
    reached: Mutex<mpsc::Sender<Instant>>,
    gate: (Mutex<bool>, std::sync::Condvar),
    timed_out: AtomicBool,
}

impl HoldBeforeTaskStoreCreate {
    fn release(&self) {
        let (lock, changed) = &self.gate;
        *lock.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = true;
        changed.notify_all();
    }
}

impl V5RuntimeHooks for HoldBeforeTaskStoreCreate {
    fn as_any(&self) -> &dyn std::any::Any { self }

    fn pause(&self, point: V5PausePoint, _deadline: Instant) -> Result<(), ReceiptLedgerError> {
        if point == V5PausePoint::BeforeTaskStoreCreate && self.first.swap(false, Ordering::SeqCst) {
            let _ = self.reached.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
                .send(Instant::now());
            let (lock, changed) = &self.gate;
            // The same gate watchdog as MaterializationRaceHooks, not a product budget.
            let (open, _) = changed.wait_timeout_while(
                lock.lock().unwrap_or_else(std::sync::PoisonError::into_inner),
                Duration::from_secs(30), |open| !*open,
            ).unwrap_or_else(std::sync::PoisonError::into_inner);
            if !*open {
                self.timed_out.store(true, Ordering::SeqCst);
                return Err(ReceiptLedgerError::DeadlineExceeded);
            }
        }
        Ok(())
    }
}

struct ReleaseGateOnDrop<'a>(&'a HoldBeforeTaskStoreCreate);
impl Drop for ReleaseGateOnDrop<'_> {
    fn drop(&mut self) { self.0.release(); }
}

pub(super) fn assert_publication_preserves_winner_after_original_read_deadline_expires() {
    let root = tempfile::tempdir().expect("temporary staged deadline state");
    let state_root = std::fs::canonicalize(root.path()).expect("physical staged deadline state");
    let identity = CoreIdentity::production_v5();
    let state = DaemonStateDirectory::open(&state_root, &identity).expect("open staged deadline state");
    let (reached_tx, reached_rx) = mpsc::channel();
    let hooks = Arc::new(HoldBeforeTaskStoreCreate {
        first: AtomicBool::new(true), reached: Mutex::new(reached_tx),
        gate: (Mutex::new(false), std::sync::Condvar::new()), timed_out: AtomicBool::new(false),
    });
    let service = Arc::new(SourceAdmissionProbe::default());
    let clock = Arc::new(ManualEpochClock::new(1_009));
    let config = DaemonServerConfig::new(state_root, identity.clone(), Duration::from_millis(50))
        .with_v5_epoch_clock_for_test(clock.clone())
        .with_invocation_service(service.clone())
        .with_runtime_hooks_for_test(hooks.clone());
    let runtime = V5ReceiptRuntime::open(&state, &config).expect("open staged deadline runtime");
    let handoff = prepare_staged_startup_handoff(&runtime, &identity, AttemptPhase::NotBegun);
    let key = handoff.key().clone();
    let winner = canonical_v5_terminal(&ReceiptTerminalOutcome::Completed {
        result: Box::new(DomainResult::success("causal staged winner")),
    }).expect("canonical staged deadline winner");

    let origin = Instant::now();
    let deadline = origin + Duration::from_secs(5);
    let (reached, released_at, joined) = thread::scope(|scope| {
        // On every unwind this gate opens before thread::scope joins its publisher.
        // No publisher thread can escape the owned test process.
        let _release_on_drop = ReleaseGateOnDrop(hooks.as_ref());
        let runtime = &runtime;
        let terminal = winner.clone();
        let publisher = scope.spawn(move || {
            let result = runtime.publish_staged_handoff_terminal_reply(handoff, terminal, 1_009, deadline);
            (Instant::now(), result)
        });
        let reached = reached_rx.recv_timeout(Duration::from_secs(10));
        if reached.is_ok() {
            while Instant::now() < deadline {
                thread::sleep(deadline.saturating_duration_since(Instant::now()).min(Duration::from_millis(25)));
            }
        }
        let released_at = Instant::now();
        hooks.release();
        // Do not assert on the receive before saving the first publisher outcome.
        let joined = publisher.join();
        (reached, released_at, joined)
    });
    let (returned_at, publication) = joined.expect("staged publisher thread panicked");
    let old_read_started = Instant::now();
    let old_read = runtime.task_projection.task_store.get(
        key.reserved_task_id(),
        crate::domain::code_intelligence::ProviderDeadline::new(deadline),
    );
    let old_read_returned = Instant::now();
    let fresh_started = Instant::now();
    let fresh_deadline = fresh_started + Duration::from_secs(5);
    let fresh_read = runtime.task_projection.task_store.get(
        key.reserved_task_id(),
        crate::domain::code_intelligence::ProviderDeadline::new(fresh_deadline),
    );
    let fresh_returned = Instant::now();
    let elapsed = |at: Instant| u64::try_from(at.saturating_duration_since(origin).as_nanos()).unwrap_or(u64::MAX);
    let record = serde_json::json!({
        "taskId":key.reserved_task_id(),"invocationId":key.invocation_id(),
        "receiptKeyDigest":crate::application::receipt_ledger::receipt_key_digest(&key),
        "deadlineNs":elapsed(deadline),"gateEnteredNs":reached.as_ref().ok().map(|at|elapsed(*at)),
        "gateReceiveError":reached.as_ref().err().map(|error|format!("{error:?}")),
        "gateReleasedNs":elapsed(released_at),"gateTimedOut":hooks.timed_out.load(Ordering::SeqCst),
        "publicationReturnedNs":elapsed(returned_at),
        "publication":publication.as_ref().map(|_|"ok").map_err(|error|format!("{error:?}")),
        "oldReadStartedNs":elapsed(old_read_started),"oldReadReturnedNs":elapsed(old_read_returned),
        "oldRead":old_read.as_ref().map(|record|record.version).map_err(|error|format!("{error:?}")),
        "freshReadStartedNs":elapsed(fresh_started),"freshDeadlineNs":elapsed(fresh_deadline),
        "freshReadReturnedNs":elapsed(fresh_returned),
        "freshRead":fresh_read.as_ref().map(|record|record.version).map_err(|error|format!("{error:?}")),
        "freshTerminalDigest":fresh_read.as_ref().ok().and_then(|record|record.task.terminal_digest()),
    });
    let _ = writeln!(std::io::stderr().lock(), "STAGED_DEADLINE_CONTROL {record}");

    assert!(reached.is_ok(), "the committed transition did not reach its gate");
    assert!(!hooks.timed_out.load(Ordering::SeqCst), "the gate watchdog expired");
    assert!(released_at >= deadline && returned_at >= deadline);
    assert!(publication.is_ok(), "the committed publication failed: {:?}", publication.as_ref().err());
    assert!(matches!(old_read, Err(crate::application::invocation_store_v5::V5TaskStoreError::DeadlineExceeded)));
    let stored = fresh_read.expect("a fresh observation reads the exact winner");
    assert_eq!(stored.task_id, key.reserved_task_id());
    assert_eq!(stored.invocation_id, key.invocation_id());
    assert_eq!(stored.updated_at_epoch_ms, 1_009);
    assert_eq!(stored.task.terminal_digest(), Some(winner.digest()));
    match publication.unwrap() {
        V5RuntimeReply::Json(V5ServerResponse::Invocation {
            outcome: V5InvocationResponse::Task { snapshot },
        }) => assert_eq!(snapshot, task_store_snapshot(&stored)),
        _ => panic!("staged publication did not return its exact Task snapshot"),
    }
    assert!(matches!(
        runtime.receipt_ledger.recover(key.clone(), Instant::now() + Duration::from_secs(5)),
        Err(ReceiptLedgerError::ReceiptNotFound)
    ));
    let snapshot = runtime.resolve_task(key.reserved_task_id(), Instant::now() + Duration::from_secs(5))
        .expect("resolve the published winner");
    assert_eq!(service.prepares.load(Ordering::SeqCst), 0);
    drop(runtime);
    clock.set(3_000);
    let reopened = V5ReceiptRuntime::open(&state, &config).expect("reopen the published winner");
    assert_eq!(reopened.resolve_task(key.reserved_task_id(), Instant::now() + Duration::from_secs(5))
        .expect("resolve reopened winner"), snapshot);
    assert_eq!(reopened.task_projection.task_store.get(
        key.reserved_task_id(),
        crate::domain::code_intelligence::ProviderDeadline::new(Instant::now() + Duration::from_secs(5)),
    ).expect("read reopened winner"), stored);
    assert_eq!(service.prepares.load(Ordering::SeqCst), 0);
}
