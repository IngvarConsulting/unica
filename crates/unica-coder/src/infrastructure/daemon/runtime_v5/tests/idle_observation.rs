//! Controlled schedules for the idle test observer; the listener and its
//! ownership decisions remain the production runtime.

use super::*;

const OBSERVER_WATCHDOG: Duration = Duration::from_secs(3);

fn receive_before<T>(receiver: &mpsc::Receiver<T>, deadline: Instant, event: &str) -> T {
    receiver
        .recv_timeout(deadline.saturating_duration_since(Instant::now()))
        .unwrap_or_else(|error| panic!("idle schedule did not reach {event}: {error}"))
}

fn idle_gate() -> IdleIndexGate {
    Arc::new((Mutex::new(false), Condvar::new()))
}

/// Created after the daemon so panic cleanup releases all fixture barriers
/// before the daemon guard tries to join their actual handlers and workers.
struct IdleObservationGates(Vec<IdleIndexGate>);

impl Drop for IdleObservationGates {
    fn drop(&mut self) {
        for gate in &self.0 {
            release_idle_handoff_gate(gate);
        }
    }
}

struct ExitIterationSchedule {
    armed: AtomicBool,
    empty_reads: AtomicUsize,
    handshake_gate: IdleIndexGate,
    first_decision_gate: IdleIndexGate,
    delay_cancel: IdleIndexGate,
    handshake_entered: mpsc::Sender<()>,
    handler_returned: mpsc::Sender<()>,
    first_decision: mpsc::Sender<bool>,
    second_entered: mpsc::Sender<Instant>,
    second_decision: mpsc::Sender<bool>,
    timestamps: Mutex<Vec<Instant>>,
    interval: Duration,
}

struct ExitIterationHooks {
    observations: mpsc::Sender<(Instant, bool)>,
    schedule: Arc<ExitIterationSchedule>,
}

impl V5RuntimeHooks for ExitIterationHooks {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn spawn_connection_handler(
        &self,
        handler: Box<dyn FnOnce() + Send + 'static>,
    ) -> io::Result<thread::JoinHandle<()>> {
        let returned = self.schedule.handler_returned.clone();
        thread::Builder::new()
            .name("idle-observer-handshake".into())
            .spawn(move || {
                handler();
                let _ = returned.send(());
            })
    }

    fn before_owner_handshake(&self, _peer_port: u16) {
        let _ = self.schedule.handshake_entered.send(());
        wait_idle_handoff_gate(&self.schedule.handshake_gate);
    }

    fn after_idle_owner_leases_read(&self, empty: bool) {
        if empty && self.schedule.armed.load(Ordering::SeqCst) {
            let ordinal = self.schedule.empty_reads.fetch_add(1, Ordering::SeqCst) + 1;
            if ordinal == 2 {
                // Inject a delayed listener iteration, anchored here rather
                // than at the first event: idle_since was reset between them.
                let entered = Instant::now();
                let _ = self.schedule.second_entered.send(entered);
                let until = entered + self.schedule.interval;
                let (cancelled, changed) = &*self.schedule.delay_cancel;
                let mut cancelled = cancelled.lock().unwrap();
                while !*cancelled {
                    let remaining = until.saturating_duration_since(Instant::now());
                    if remaining.is_zero() {
                        break;
                    }
                    cancelled = changed.wait_timeout(cancelled, remaining).unwrap().0;
                }
            }
            let observed = Instant::now();
            self.schedule.timestamps.lock().unwrap().push(observed);
            let _ = self.observations.send((observed, empty));
        } else {
            let _ = self.observations.send((Instant::now(), empty));
        }
    }

    fn listener_ownership_observed(&self, no_active_work: bool) {
        match self.schedule.empty_reads.load(Ordering::SeqCst) {
            1 => {
                let _ = self.schedule.first_decision.send(no_active_work);
                wait_idle_handoff_gate(&self.schedule.first_decision_gate);
            }
            2 => {
                let _ = self.schedule.second_decision.send(no_active_work);
            }
            _ => {}
        }
    }
}

#[test]
fn exiting_idle_iteration_is_not_reported_as_a_retained_daemon() {
    let idle = Duration::from_millis(80);
    let (handshake_entered, handshake) = mpsc::channel();
    let (handler_returned, handler_exit) = mpsc::channel();
    let (first_decision, first) = mpsc::channel();
    let (second_entered, second) = mpsc::channel();
    let (second_decision, decision) = mpsc::channel();
    let schedule = Arc::new(ExitIterationSchedule {
        armed: AtomicBool::new(false),
        empty_reads: AtomicUsize::new(0),
        handshake_gate: idle_gate(),
        first_decision_gate: idle_gate(),
        delay_cancel: idle_gate(),
        handshake_entered,
        handler_returned,
        first_decision,
        second_entered,
        second_decision,
        timestamps: Mutex::new(Vec::new()),
        interval: idle * 3,
    });
    let hook_schedule = Arc::clone(&schedule);
    let mut daemon = PromisedWorkIdleDaemon::with_service_and_hooks(
        idle,
        super::super::super::server::actor_capacity_tests::canonical_v13_service(),
        move |observations| {
            Arc::new(ExitIterationHooks {
                observations,
                schedule: hook_schedule,
            })
        },
    );
    let _cleanup = IdleObservationGates(vec![
        Arc::clone(&schedule.handshake_gate),
        Arc::clone(&schedule.first_decision_gate),
        Arc::clone(&schedule.delay_cancel),
    ]);
    let transport = TcpStream::connect(daemon.record.loopback_addr().unwrap()).unwrap();
    daemon.guard.streams.push(transport.try_clone().unwrap());
    receive_before(
        &handshake,
        Instant::now() + OBSERVER_WATCHDOG,
        "held handshake",
    );

    let anchor = Instant::now();
    let watchdog = anchor + OBSERVER_WATCHDOG;
    schedule.armed.store(true, Ordering::SeqCst);
    assert!(
        !receive_before(&first, watchdog, "first full ownership observation"),
        "the first lease-empty iteration must still own its accepted handshake"
    );
    transport.shutdown(std::net::Shutdown::Both).unwrap();
    release_idle_handoff_gate(&schedule.handshake_gate);
    receive_before(&handler_exit, watchdog, "actual handshake handler return");
    release_idle_handoff_gate(&schedule.first_decision_gate);
    let delayed_at = receive_before(&second, watchdog, "second idle callback");
    assert!(receive_before(
        &decision,
        watchdog,
        "owner-free exit iteration"
    ));
    daemon.await_idle_exit_before(watchdog);
    daemon.join();
    assert!(daemon.endpoint().is_none());

    let timestamps = schedule.timestamps.lock().unwrap();
    assert_eq!(timestamps.len(), 2);
    assert!(timestamps[0] >= anchor);
    assert!(timestamps[1].duration_since(delayed_at) >= idle * 3);
    assert!(timestamps[1].duration_since(timestamps[0]) >= idle * 3);
    eprintln!(
        "natural exit; first={:?}, second={:?}, injected pause={:?}, stop=false",
        timestamps[0].duration_since(anchor),
        timestamps[1].duration_since(anchor),
        timestamps[1].duration_since(delayed_at),
    );
}

struct HeldTerminalIdleHooks {
    observations: mpsc::Sender<(Instant, bool)>,
    ownerships: mpsc::Sender<(Instant, bool, bool)>,
    last_owner_read: Mutex<Option<(Instant, bool)>>,
    terminal_entered: mpsc::Sender<crate::domain::invocation::TaskId>,
    terminal_gate: IdleIndexGate,
}

impl V5RuntimeHooks for HeldTerminalIdleHooks {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn after_idle_owner_leases_read(&self, empty: bool) {
        let observed = Instant::now();
        *self.last_owner_read.lock().unwrap() = Some((observed, empty));
        let _ = self.observations.send((observed, empty));
    }

    fn listener_ownership_observed(&self, no_active_work: bool) {
        if let Some((observed, empty)) = *self.last_owner_read.lock().unwrap() {
            let _ = self.ownerships.send((observed, empty, no_active_work));
        }
    }

    fn terminal_bound_task(
        &self,
        record: &V5StoredInvocationRecord,
        _link: &TaskTerminalBoundReceipt,
    ) {
        let _ = self.terminal_entered.send(record.task_id);
        wait_idle_handoff_gate(&self.terminal_gate);
    }
}

#[test]
fn visible_terminal_and_replay_do_not_prove_the_task_owner_has_released() {
    let (_workspace, workspace) = catalog_workspace();
    let (entered, executing) = mpsc::channel();
    let service_gate = idle_gate();
    let terminal_gate = idle_gate();
    let service = Arc::new(IdleInvocationService {
        known_long: true,
        prepares: AtomicUsize::new(0),
        executions: AtomicUsize::new(0),
        entered,
        release: Arc::clone(&service_gate),
    });
    let (terminal_entered, terminal_at_gate) = mpsc::channel();
    let (ownerships, ownership_reads) = mpsc::channel();
    let hook_gate = Arc::clone(&terminal_gate);
    let idle = Duration::from_millis(80);
    let mut daemon = PromisedWorkIdleDaemon::with_service_and_hooks(
        idle,
        service.clone(),
        move |observations| {
            Arc::new(HeldTerminalIdleHooks {
                observations,
                ownerships,
                last_owner_read: Mutex::new(None),
                terminal_entered,
                terminal_gate: hook_gate,
            })
        },
    );
    let _cleanup =
        IdleObservationGates(vec![Arc::clone(&service_gate), Arc::clone(&terminal_gate)]);
    let invocation = view_catalog_request(TaskId::new(), &workspace);
    let mut owner = daemon.owner();
    let initial = submitted_task_snapshot(Ok(idle_request_over_tcp(
        &mut owner,
        V5ClientRequest::SubmitInvocation {
            invocation: invocation.clone(),
        },
    )));
    assert_eq!(initial.task_id(), invocation.reserved_task_id());
    assert_eq!(initial.invocation_id(), invocation.invocation_id());
    receive_before(
        &executing,
        Instant::now() + OBSERVER_WATCHDOG,
        "service execution",
    );
    release_promised_work_owner(owner);
    assert!(daemon.observe_idle_interval(idle * 3));
    assert_eq!(daemon.endpoint(), Some(daemon.record.clone()));

    let mut successor = daemon.owner();
    let V5ServerResponse::Task { snapshot: running } = idle_request_over_tcp(
        &mut successor,
        V5ClientRequest::GetTask {
            task_id: invocation.reserved_task_id(),
        },
    ) else {
        panic!("successor must see the original live Task");
    };
    assert_eq!(running.task_id(), initial.task_id());
    assert_eq!(running.invocation_id(), initial.invocation_id());
    assert!(!task_snapshot_is_terminal(&running));
    release_idle_handoff_gate(&service_gate);
    assert_eq!(
        receive_before(
            &terminal_at_gate,
            Instant::now() + OBSERVER_WATCHDOG,
            "published terminal before cancellation guard drop",
        ),
        invocation.reserved_task_id(),
    );
    let terminal = idle_task_terminal_over_tcp(&mut successor, &invocation);
    assert!(
        matches!(&terminal, V5DaemonTaskSnapshot::Completed { result, .. }
        if result.ok && result.data == Some(json!({"attempt": 1})))
    );
    let replay = submitted_task_snapshot(Ok(idle_request_over_tcp(
        &mut successor,
        V5ClientRequest::SubmitInvocation { invocation },
    )));
    assert_eq!(replay, terminal);
    assert_eq!(service.prepares.load(Ordering::SeqCst), 1);
    assert_eq!(service.executions.load(Ordering::SeqCst), 1);
    release_promised_work_owner(successor);
    let released_at = Instant::now();
    assert!(
        daemon.observe_idle_interval(idle * 3),
        "the terminal worker still owns its cancellation guard"
    );
    let held_until = Instant::now();
    let mut first_empty = None;
    loop {
        let (observed, empty, no_active_work) = receive_before(
            &ownership_reads,
            released_at + OBSERVER_WATCHDOG,
            "full ownership while terminal worker remains held",
        );
        if observed >= released_at && empty {
            assert!(
                !no_active_work,
                "the held Task guard must retain the runtime"
            );
            let start = *first_empty.get_or_insert(observed);
            if observed >= held_until && observed.duration_since(start) >= idle * 3 {
                break;
            }
        }
    }
    assert!(matches!(
        daemon.exited.try_recv(),
        Err(mpsc::TryRecvError::Empty)
    ));
    assert_eq!(daemon.endpoint(), Some(daemon.record.clone()));
    eprintln!("terminal and exact replay visible while the live Task guard retains the daemon");

    release_idle_handoff_gate(&terminal_gate);
    daemon.await_idle_exit();
    daemon.join();
    assert!(daemon.endpoint().is_none());
}
