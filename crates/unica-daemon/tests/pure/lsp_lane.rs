//! Полоса LSP без транспорта: отмена, переполнение, паника службы, закрытие.
use super::*;
use crate::protocol_v5::lsp::{V5LspMessageLevel, V5LspPosition};
use std::sync::atomic::AtomicUsize;
use std::time::Duration;

/// Служба, чей hover держит исполнителя до отмены или до `release`.
#[derive(Default)]
struct HeldHost {
    started: AtomicUsize,
    saw_cancellation: AtomicUsize,
    calls: AtomicUsize,
    release: AtomicBool,
    client: Mutex<Option<LspClientChannel>>,
    /// Answer for fewer folders than asked: a broken service.
    skip_folder: AtomicBool,
    closed: AtomicUsize,
}

impl LspHost for HeldHost {
    fn open(
        &self,
        _: &V5LspSessionId,
        folders: &[V5LspWorkspaceFolder],
        client: LspClientChannel,
        _: Instant,
    ) -> Result<V5LspOpening, V5LspFailure> {
        *self.client.lock().unwrap() = Some(client);
        let skip = usize::from(self.skip_folder.load(Ordering::Acquire));
        Ok(V5LspOpening {
            capabilities: V5LspCapabilities {
                text_document_sync: crate::protocol_v5::lsp::V5LspTextSync::Full,
                hover: true,
                text_document_content: false,
            },
            workspaces: folders
                .iter()
                .skip(skip)
                .enumerate()
                .map(|(index, folder)| {
                    V5LspWorkspaceSpace::opened(&folder.uri, format!("w{index}"))
                })
                .collect(),
        })
    }

    fn notify(
        &self,
        _: &V5LspSessionId,
        _: V5LspNotification,
        _: Instant,
    ) -> Result<(), V5LspFailure> {
        Ok(())
    }

    fn call(&self, _: &V5LspSessionId, call: V5LspCall, context: &LspCallContext) -> V5LspOutcome {
        self.calls.fetch_add(1, Ordering::AcqRel);
        let uri = call.document_uri().unwrap_or_default();
        if uri.ends_with("panic.bsl") {
            panic!("the host failed inside one request");
        }
        self.started.fetch_add(1, Ordering::AcqRel);
        while !self.release.load(Ordering::Acquire) {
            if context.is_cancelled() {
                self.saw_cancellation.fetch_add(1, Ordering::AcqRel);
                return V5LspOutcome::Cancelled {};
            }
            thread::sleep(Duration::from_millis(1));
        }
        V5LspOutcome::Hover {
            contents: Some("answer".into()),
        }
    }

    fn close(&self, _: &V5LspSessionId) {
        self.closed.fetch_add(1, Ordering::AcqRel);
    }
}

fn hover(uri: &str) -> V5LspCall {
    V5LspCall::Hover {
        uri: uri.into(),
        position: V5LspPosition {
            line: 0,
            character: 0,
        },
    }
}

fn wait_until(condition: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !condition() {
        assert!(Instant::now() < deadline, "condition did not hold in time");
        thread::sleep(Duration::from_millis(1));
    }
}

fn lane_with(host: &Arc<HeldHost>) -> (Arc<LspLane>, V5LspSessionId) {
    let lane = Arc::new(LspLane::start(Arc::clone(host) as Arc<dyn LspHost>).unwrap());
    let (session, opening) = lane.open(&[], soon()).unwrap();
    assert!(opening.capabilities.hover);
    (lane, session)
}

fn soon() -> Instant {
    Instant::now() + Duration::from_secs(5)
}

#[test]
fn a_full_memory_of_early_cancellations_forgets_only_the_oldest() {
    let host = Arc::new(HeldHost::default());
    host.release.store(true, Ordering::Release);
    let (lane, session) = lane_with(&host);
    for request in 0..=MAX_EARLY_CANCELLATIONS as u64 {
        lane.cancel(&session, request).unwrap();
    }

    assert!(matches!(
        lane.call(&session, 0, hover("file:///a.bsl"), soon()),
        V5LspOutcome::Hover { .. }
    ));
    assert_eq!(
        lane.call(&session, 1, hover("file:///a.bsl"), soon()),
        V5LspOutcome::Cancelled {}
    );
    assert_eq!(
        lane.call(
            &session,
            MAX_EARLY_CANCELLATIONS as u64,
            hover("file:///a.bsl"),
            soon()
        ),
        V5LspOutcome::Cancelled {}
    );
}

fn folder(uri: &str) -> V5LspWorkspaceFolder {
    V5LspWorkspaceFolder {
        uri: uri.into(),
        name: "work".into(),
    }
}

#[test]
fn every_folder_gets_its_own_space_or_the_session_is_refused() {
    let host = Arc::new(HeldHost::default());
    let lane = LspLane::start(Arc::clone(&host) as Arc<dyn LspHost>).unwrap();
    let folders = [folder("file:///a"), folder("file:///b")];

    let (_, opening) = lane.open(&folders, soon()).unwrap();
    assert_eq!(
        opening.workspaces,
        vec![
            V5LspWorkspaceSpace::opened("file:///a", "w0"),
            V5LspWorkspaceSpace::opened("file:///b", "w1"),
        ]
    );

    host.skip_folder.store(true, Ordering::Release);
    assert_eq!(
        lane.open(&folders, soon()).err(),
        Some(V5LspFailure::Unavailable)
    );
    assert_eq!(host.closed.load(Ordering::Acquire), 1);
}

#[test]
fn a_cancelled_request_stops_its_work_and_withholds_its_answer() {
    let host = Arc::new(HeldHost::default());
    let (lane, session) = lane_with(&host);
    let caller = {
        let (lane, session) = (Arc::clone(&lane), session.clone());
        thread::spawn(move || {
            lane.call(
                &session,
                7,
                hover("file:///a.bsl"),
                Instant::now() + Duration::from_secs(5),
            )
        })
    };
    wait_until(|| host.started.load(Ordering::Acquire) == 1);

    lane.cancel(&session, 7).unwrap();

    assert_eq!(caller.join().unwrap(), V5LspOutcome::Cancelled {});
    assert_eq!(host.saw_cancellation.load(Ordering::Acquire), 1);
}

#[test]
fn a_cancellation_that_overtakes_its_request_keeps_the_request_from_running() {
    let host = Arc::new(HeldHost::default());
    let (lane, session) = lane_with(&host);

    lane.cancel(&session, 3).unwrap();
    let outcome = lane.call(
        &session,
        3,
        hover("file:///a.bsl"),
        Instant::now() + Duration::from_secs(5),
    );

    assert_eq!(outcome, V5LspOutcome::Cancelled {});
    assert_eq!(host.calls.load(Ordering::Acquire), 0);
}

/// Controls when the real workers first dequeue; admission still uses `call`.
struct HeldLane {
    host: Arc<HeldHost>,
    lane: Arc<LspLane>,
    session: V5LspSessionId,
    deadline: Instant,
    callers: Vec<thread::JoinHandle<V5LspOutcome>>,
    workers: Vec<thread::JoinHandle<()>>,
    busy: Arc<AtomicUsize>,
    completed: Arc<AtomicUsize>,
    workers_finished: Arc<AtomicUsize>,
    finished: bool,
}

impl HeldLane {
    fn before_workers_start() -> Self {
        let host = Arc::new(HeldHost::default());
        let lane = Arc::new(LspLane {
            shared: Arc::new(Shared {
                host: Arc::clone(&host) as Arc<dyn LspHost>,
                state: Mutex::new(LaneState::default()),
                queued: Condvar::new(),
            }),
            workers: Vec::new(),
        });
        let (session, _) = lane.open(&[], soon()).unwrap();
        Self {
            host,
            lane,
            session,
            deadline: Instant::now() + Duration::from_secs(10),
            callers: Vec::new(),
            workers: Vec::new(),
            busy: Arc::new(AtomicUsize::new(0)),
            completed: Arc::new(AtomicUsize::new(0)),
            workers_finished: Arc::new(AtomicUsize::new(0)),
            finished: false,
        }
    }

    fn spawn_calls(&mut self, requests: std::ops::Range<usize>) {
        for request in requests {
            let lane = Arc::clone(&self.lane);
            let session = self.session.clone();
            let deadline = self.deadline;
            let busy = Arc::clone(&self.busy);
            let completed = Arc::clone(&self.completed);
            self.callers.push(thread::spawn(move || {
                let outcome = lane.call(&session, request as u64, hover("file:///a.bsl"), deadline);
                if matches!(
                    outcome,
                    V5LspOutcome::Failed {
                        failure: V5LspFailure::Busy
                    }
                ) {
                    busy.fetch_add(1, Ordering::AcqRel);
                }
                completed.fetch_add(1, Ordering::AcqRel);
                outcome
            }));
        }
    }

    fn start_workers(&mut self) {
        assert!(self.workers.is_empty());
        for _ in 0..MAX_LSP_LANE_WORKERS {
            let shared = Arc::clone(&self.lane.shared);
            let finished = Arc::clone(&self.workers_finished);
            self.workers.push(thread::spawn(move || {
                run_worker(&shared);
                finished.fetch_add(1, Ordering::AcqRel);
            }));
        }
    }

    fn queued(&self) -> usize {
        self.lane.shared.state.lock().unwrap().jobs.len()
    }

    fn full(&self) -> bool {
        self.host.started.load(Ordering::Acquire) == MAX_LSP_LANE_WORKERS
            && self.queued() == LSP_LANE_QUEUE_CAPACITY
    }

    fn finish(&mut self) -> Vec<thread::Result<V5LspOutcome>> {
        if self.finished {
            return Vec::new();
        }
        self.host.release.store(true, Ordering::Release);
        if self.workers.is_empty() && !self.callers.is_empty() {
            self.start_workers();
        }
        // Join every caller even if one panicked; inspect outcomes afterwards.
        let outcomes = self.callers.drain(..).map(|caller| caller.join()).collect();
        self.lane.shared.state.lock().unwrap().stopping = true;
        self.lane.shared.queued.notify_all();
        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
        self.finished = true;
        outcomes
    }
}

impl Drop for HeldLane {
    fn drop(&mut self) {
        let _ = self.finish();
    }
}

#[test]
fn a_full_lane_refuses_new_requests_instead_of_growing() {
    let mut held = HeldLane::before_workers_start();
    held.spawn_calls(0..MAX_LSP_LANE_WORKERS);
    wait_until(|| held.queued() == MAX_LSP_LANE_WORKERS);
    held.start_workers();
    wait_until(|| held.host.started.load(Ordering::Acquire) == MAX_LSP_LANE_WORKERS);
    // Both workers now hold requests, so every subsequent admission queues.
    held.spawn_calls(MAX_LSP_LANE_WORKERS..MAX_LSP_LANE_WORKERS + LSP_LANE_QUEUE_CAPACITY);
    wait_until(|| held.full());
    assert_eq!(held.busy.load(Ordering::Acquire), 0);
    let refused = held
        .lane
        .call(&held.session, 10_000, hover("file:///a.bsl"), held.deadline);
    assert_eq!(refused, failed(V5LspFailure::Busy));
    assert_eq!(held.queued(), LSP_LANE_QUEUE_CAPACITY);
    let outcomes = held.finish();
    assert_eq!(
        outcomes.len(),
        MAX_LSP_LANE_WORKERS + LSP_LANE_QUEUE_CAPACITY
    );
    for outcome in outcomes {
        assert!(matches!(outcome, Ok(V5LspOutcome::Hover { .. })));
    }
}

#[test]
fn callers_admitted_before_workers_can_refuse_before_the_lane_is_full() {
    let mut held = HeldLane::before_workers_start();
    held.spawn_calls(0..MAX_LSP_LANE_WORKERS + LSP_LANE_QUEUE_CAPACITY);
    wait_until(|| {
        held.queued() == LSP_LANE_QUEUE_CAPACITY
            && held.busy.load(Ordering::Acquire) == MAX_LSP_LANE_WORKERS
    });
    assert_eq!(held.host.started.load(Ordering::Acquire), 0);
    held.start_workers();
    wait_until(|| held.host.started.load(Ordering::Acquire) == MAX_LSP_LANE_WORKERS);
    assert_eq!(
        held.queued(),
        LSP_LANE_QUEUE_CAPACITY - MAX_LSP_LANE_WORKERS
    );
    assert!(
        !held.full(),
        "early refusals leave no callers to fill the two newly free queue slots"
    );
    eprintln!(
        "controlled old schedule: started={}, queued={}, early_busy={}",
        held.host.started.load(Ordering::Acquire),
        held.queued(),
        held.busy.load(Ordering::Acquire)
    );
    let outcomes = held.finish();
    assert_eq!(
        outcomes.len(),
        MAX_LSP_LANE_WORKERS + LSP_LANE_QUEUE_CAPACITY
    );
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| matches!(
                outcome,
                Ok(V5LspOutcome::Failed {
                    failure: V5LspFailure::Busy
                })
            ))
            .count(),
        MAX_LSP_LANE_WORKERS
    );
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| matches!(outcome, Ok(V5LspOutcome::Hover { .. })))
            .count(),
        LSP_LANE_QUEUE_CAPACITY
    );
}

#[test]
fn held_lane_releases_and_joins_callers_and_workers_when_an_assertion_unwinds() {
    let mut held = HeldLane::before_workers_start();
    let host = Arc::clone(&held.host);
    let completed = Arc::clone(&held.completed);
    let workers_finished = Arc::clone(&held.workers_finished);
    let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
        held.spawn_calls(0..MAX_LSP_LANE_WORKERS);
        held.start_workers();
        wait_until(|| held.host.started.load(Ordering::Acquire) == MAX_LSP_LANE_WORKERS);
        panic!("injected assertion failure while both requests are held");
    }));
    assert!(unwound.is_err());
    assert!(host.release.load(Ordering::Acquire));
    assert_eq!(completed.load(Ordering::Acquire), MAX_LSP_LANE_WORKERS);
    assert_eq!(
        workers_finished.load(Ordering::Acquire),
        MAX_LSP_LANE_WORKERS
    );
}

#[test]
fn a_panicking_host_fails_only_its_own_request() {
    let host = Arc::new(HeldHost::default());
    host.release.store(true, Ordering::Release);
    let (lane, session) = lane_with(&host);
    let deadline = Instant::now() + Duration::from_secs(5);

    let failed = lane.call(&session, 1, hover("file:///panic.bsl"), deadline);
    let answered = lane.call(&session, 2, hover("file:///a.bsl"), deadline);

    assert_eq!(
        failed,
        V5LspOutcome::Failed {
            failure: V5LspFailure::Unavailable
        }
    );
    assert!(matches!(answered, V5LspOutcome::Hover { .. }));
}

#[test]
fn an_answer_past_its_deadline_is_not_given() {
    let host = Arc::new(HeldHost::default());
    let (lane, session) = lane_with(&host);

    let outcome = lane.call(
        &session,
        1,
        hover("file:///a.bsl"),
        Instant::now() + Duration::from_millis(50),
    );

    assert_eq!(
        outcome,
        V5LspOutcome::Failed {
            failure: V5LspFailure::DeadlineExceeded
        }
    );
    wait_until(|| host.saw_cancellation.load(Ordering::Acquire) == 1);
}

#[test]
fn closing_a_session_cancels_its_requests_and_ends_its_notifications() {
    let host = Arc::new(HeldHost::default());
    let (lane, session) = lane_with(&host);
    let client = host.client.lock().unwrap().clone().unwrap();
    assert!(client.push(V5LspServerNotification::LogMessage {
        level: V5LspMessageLevel::Info,
        message: "ready".into(),
    }));
    assert_eq!(
        lane.poll(&session, Instant::now()).unwrap(),
        vec![V5LspServerNotification::LogMessage {
            level: V5LspMessageLevel::Info,
            message: "ready".into(),
        }]
    );
    let caller = {
        let (lane, session) = (Arc::clone(&lane), session.clone());
        thread::spawn(move || {
            lane.call(
                &session,
                1,
                hover("file:///a.bsl"),
                Instant::now() + Duration::from_secs(5),
            )
        })
    };
    wait_until(|| host.started.load(Ordering::Acquire) == 1);

    lane.close(&session);

    assert_eq!(caller.join().unwrap(), V5LspOutcome::Cancelled {});
    assert!(!client.push(V5LspServerNotification::LogMessage {
        level: V5LspMessageLevel::Info,
        message: "late".into(),
    }));
    assert_eq!(
        lane.poll(&session, Instant::now()),
        Err(V5LspFailure::SessionUnknown)
    );
    assert_eq!(
        lane.call(
            &session,
            2,
            hover("file:///a.bsl"),
            Instant::now() + Duration::from_secs(1)
        ),
        V5LspOutcome::Failed {
            failure: V5LspFailure::SessionUnknown
        }
    );
}
