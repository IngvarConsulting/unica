use super::prelude::*;
use serde_json::json;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::thread;
use unica_daemon_protocol::protocol_v5::V5ClientRequest;

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

/// A call far above the former 16 KiB frame reaches the service and
/// runs; one above the request bound never leaves the frontend and is
/// refused with the limit and its own size, not a bare `invalid_request`.
/// rule: INV.WIRE.DAEMON-REQUEST-FRAMING
#[test]
fn live_daemon_runs_a_large_call_and_refuses_one_above_the_bound_unsent() {
    use unica_daemon_protocol::protocol_v5::MAX_V5_REQUEST_LINE_BYTES;
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
    // remedy; any other failure stays the internal one.
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
        Ok(CanonicalCallOutcome::Choice { choice, .. }) => {
            panic!("slow work asked a choice: {choice:?}")
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
        Ok(CanonicalCallOutcome::Choice { choice, .. }) => panic!("unexpected choice: {choice:?}"),
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
