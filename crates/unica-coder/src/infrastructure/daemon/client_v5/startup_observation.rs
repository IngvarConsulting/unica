//! Opt-in, bounded evidence for the original #1714 fixture. No loop I/O.

use crate::infrastructure::platform::{ManagedStartupChild, StartupChildObservation};
use std::cell::RefCell;
use std::process::ExitStatus;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub(super) enum Stop {
    DeadlineBeforePoll,
    DeadlineBeforeSleep,
    ObservedExit,
    WaitError,
    OtherReadinessError,
}

#[derive(Clone, Copy, Debug, serde::Serialize)]
pub(super) struct Poll {
    at: Duration,
    finished: Duration,
    exited: Option<bool>,
    exit_code: Option<i32>,
}

#[derive(Debug, serde::Serialize)]
pub(super) struct Trace {
    #[serde(skip)]
    started: Instant,
    case: &'static str,
    pid: Option<u32>,
    recorded_pid: Option<u32>,
    spawned: Option<Duration>,
    record_published: Option<Duration>,
    entered: Option<Duration>,
    readiness_deadline: Option<Duration>,
    overall_deadline: Option<Duration>,
    pub(super) polls: u32,
    poll_started: Option<Duration>,
    first_poll: Option<Poll>,
    pub(super) last_poll: Option<Poll>,
    max_observation_interval: Duration,
    pub(super) stop: Option<(Stop, Duration)>,
    pub(super) before_cleanup: Option<StartupChildObservation>,
    probe_started: Option<Duration>,
    probe_finished: Option<Duration>,
    cleanup_finished: Option<Duration>,
    pub(super) cleanup_ok: Option<bool>,
    endpoint_cleanup_ok: Option<bool>,
}

thread_local! {
    static TRACE: RefCell<Option<Trace>> = const { RefCell::new(None) };
}

pub(super) struct Scope;

impl Scope {
    pub(super) fn start(case: &'static str, started: Instant) -> Self {
        TRACE.with(|slot| {
            let mut slot = slot.borrow_mut();
            assert!(slot.is_none(), "startup observation must not be nested");
            *slot = Some(Trace {
                started,
                case,
                pid: None,
                recorded_pid: None,
                spawned: None,
                record_published: None,
                entered: None,
                readiness_deadline: None,
                overall_deadline: None,
                polls: 0,
                poll_started: None,
                first_poll: None,
                last_poll: None,
                max_observation_interval: Duration::ZERO,
                stop: None,
                before_cleanup: None,
                probe_started: None,
                probe_finished: None,
                cleanup_finished: None,
                cleanup_ok: None,
                endpoint_cleanup_ok: None,
            });
        });
        Self
    }

    pub(super) fn finish(self) -> Trace {
        let trace = TRACE.with(|slot| slot.borrow_mut().take().expect("startup observation"));
        // Emission is after cleanup, including successful first attempts.
        eprintln!(
            "startup-observation: {}",
            serde_json::to_string(&trace).unwrap()
        );
        trace
    }
}

impl Drop for Scope {
    fn drop(&mut self) {
        // Also retain setup failure evidence; ordinary finish already took it.
        let trace = TRACE.with(|slot| slot.borrow_mut().take());
        if let Some(trace) = trace {
            if let Ok(json) = serde_json::to_string(&trace) {
                eprintln!("startup-observation: {json}");
            }
        }
    }
}

fn record(action: impl FnOnce(&mut Trace, Duration)) {
    TRACE.with(|slot| {
        if let Some(trace) = slot.borrow_mut().as_mut() {
            let at = trace.started.elapsed();
            action(trace, at);
        }
    });
}

pub(super) fn spawned(pid: u32) {
    record(|trace, at| {
        trace.pid = Some(pid);
        trace.spawned = Some(at);
    });
}

pub(super) fn published(recorded_pid: u32) {
    record(|trace, at| {
        trace.recorded_pid = Some(recorded_pid);
        trace.record_published = Some(at);
    });
}

pub(super) fn entered(readiness: Instant, overall: Instant) {
    record(|trace, at| {
        trace.entered = Some(at);
        trace.readiness_deadline = Some(readiness.saturating_duration_since(trace.started));
        trace.overall_deadline = Some(overall.saturating_duration_since(trace.started));
    });
}

pub(super) fn poll_started() {
    record(|trace, at| {
        trace.polls += 1;
        trace.poll_started = Some(at);
    });
}

pub(super) fn poll_finished(status: &Result<Option<ExitStatus>, String>) {
    record(|trace, at| {
        let started = trace.poll_started.take().expect("poll started");
        let previous_observation = trace.last_poll.map_or(started, |poll| poll.finished);
        trace.max_observation_interval = trace
            .max_observation_interval
            .max(at.saturating_sub(previous_observation));
        let poll = Poll {
            at: started,
            finished: at,
            exited: status.as_ref().ok().map(Option::is_some),
            exit_code: status
                .as_ref()
                .ok()
                .and_then(|status| status.and_then(|status| status.code())),
        };
        trace.first_poll.get_or_insert(poll);
        trace.last_poll = Some(poll);
        let stop = match status {
            Ok(Some(_)) => Some(Stop::ObservedExit),
            Err(_) => Some(Stop::WaitError),
            Ok(None) => None,
        };
        if let Some(stop) = stop {
            trace.stop.get_or_insert((stop, at));
        }
    });
}

pub(super) fn stopped(stop: Stop) {
    record(|trace, at| {
        if let Some(last) = trace.last_poll {
            trace.max_observation_interval = trace
                .max_observation_interval
                .max(at.saturating_sub(last.finished));
        }
        trace.stop.get_or_insert((stop, at));
    });
}

pub(super) fn before_cleanup(child: &ManagedStartupChild) {
    record(|trace, at| {
        trace.stop.get_or_insert((Stop::OtherReadinessError, at));
        trace.probe_started = Some(at);
        trace.before_cleanup = Some(child.observe_before_cleanup_for_test());
        trace.probe_finished = Some(trace.started.elapsed());
    });
}

pub(super) fn cleanup_finished(cleanup_ok: bool, endpoint_cleanup_ok: bool) {
    record(|trace, at| {
        trace.cleanup_finished = Some(at);
        trace.cleanup_ok = Some(cleanup_ok);
        trace.endpoint_cleanup_ok = Some(endpoint_cleanup_ok);
    });
}

#[test]
fn first_stop_and_observation_interval_survive_later_diagnostics() {
    let scope = Scope::start("recorder_control", Instant::now() - Duration::from_secs(10));
    record(|trace, _| {
        trace.last_poll = Some(Poll {
            at: Duration::from_secs(1),
            finished: Duration::from_secs(2),
            exited: Some(false),
            exit_code: None,
        });
    });
    stopped(Stop::DeadlineBeforePoll);
    let first = TRACE.with(|slot| slot.borrow().as_ref().unwrap().stop);
    stopped(Stop::WaitError);
    cleanup_finished(true, true);
    let trace = scope.finish();
    assert_eq!(trace.stop, first);
    assert!(trace.max_observation_interval >= Duration::from_secs(8));
    assert_eq!(trace.last_poll.unwrap().exited, Some(false));
    assert_eq!(trace.cleanup_ok, Some(true));
}
