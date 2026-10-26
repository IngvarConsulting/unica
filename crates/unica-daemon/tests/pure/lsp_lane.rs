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
}

impl LspHost for HeldHost {
    fn open(
        &self,
        _: &V5LspSessionId,
        _: &[V5LspWorkspaceFolder],
        client: LspClientChannel,
    ) -> Result<V5LspCapabilities, V5LspFailure> {
        *self.client.lock().unwrap() = Some(client);
        Ok(V5LspCapabilities {
            text_document_sync: crate::protocol_v5::lsp::V5LspTextSync::Full,
            hover: true,
        })
    }

    fn notify(&self, _: &V5LspSessionId, _: V5LspNotification) -> Result<(), V5LspFailure> {
        Ok(())
    }

    fn call(&self, _: &V5LspSessionId, call: V5LspCall, context: &LspCallContext) -> V5LspOutcome {
        self.calls.fetch_add(1, Ordering::AcqRel);
        let V5LspCall::Hover { uri, .. } = call;
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

    fn close(&self, _: &V5LspSessionId) {}
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
    let (session, capabilities) = lane.open(&[]).unwrap();
    assert!(capabilities.hover);
    (lane, session)
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

#[test]
fn a_full_lane_refuses_new_requests_instead_of_growing() {
    let host = Arc::new(HeldHost::default());
    let (lane, session) = lane_with(&host);
    let deadline = Instant::now() + Duration::from_secs(10);
    let callers = (0..MAX_LSP_LANE_WORKERS + LSP_LANE_QUEUE_CAPACITY)
        .map(|request| {
            let (lane, session) = (Arc::clone(&lane), session.clone());
            thread::spawn(move || {
                lane.call(&session, request as u64, hover("file:///a.bsl"), deadline)
            })
        })
        .collect::<Vec<_>>();
    wait_until(|| {
        host.started.load(Ordering::Acquire) == MAX_LSP_LANE_WORKERS
            && lane.shared.state.lock().unwrap().jobs.len() == LSP_LANE_QUEUE_CAPACITY
    });

    let refused = lane.call(&session, 10_000, hover("file:///a.bsl"), deadline);

    assert_eq!(
        refused,
        V5LspOutcome::Failed {
            failure: V5LspFailure::Busy
        }
    );
    host.release.store(true, Ordering::Release);
    for caller in callers {
        assert!(matches!(caller.join().unwrap(), V5LspOutcome::Hover { .. }));
    }
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
