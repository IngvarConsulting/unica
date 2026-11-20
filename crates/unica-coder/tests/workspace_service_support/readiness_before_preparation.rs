use super::*;
use std::collections::BTreeMap;
use std::sync::atomic::AtomicBool;
use unica_analysis_services::backend::BslBackend;
use unica_process::transport_failure::PrivateFailureSnapshot;
use unica_workspace::test_support as testing;

type SourceImage = BTreeMap<PathBuf, Vec<u8>>;

struct HeldPreparation {
    starts: AtomicUsize,
    active: Mutex<Option<CancellationToken>>,
    abort: AtomicBool,
    wake: Condvar,
    gate: Mutex<()>,
    entered: mpsc::Sender<SourceImage>,
    finished: mpsc::Sender<bool>,
}
impl BslBackend for HeldPreparation {
    fn start(
        &self,
        _: &WorkspaceContext,
        source: &Path,
        cancel: &CancellationToken,
    ) -> Result<PersistentMcpSession, String> {
        self.starts.fetch_add(1, Ordering::AcqRel);
        *self.active.lock().unwrap() = Some(cancel.clone());
        // The test controls the provider boundary, not the listener/executor.
        // Signal entry only after actual reads of the source-tree fixture.
        let observed = testing::tree_snapshot(source);
        let _ = self.entered.send(observed);
        let mut gate = self.gate.lock().unwrap();
        while !cancel.is_cancelled() && !self.abort.load(Ordering::Acquire) {
            gate = self
                .wake
                .wait_timeout(gate, Duration::from_millis(5))
                .unwrap()
                .0;
        }
        let _ = self.finished.send(cancel.is_cancelled());
        Err(cancelled_error(
            "controlled provider source preparation stopped",
        ))
    }
    fn report_failure(&self, _: PrivateFailureSnapshot) {}
}

struct RunningStartup {
    context: Option<ServiceTestContext>,
    identity: WorkspaceServiceIdentity,
    preparation: Arc<HeldPreparation>,
    done: mpsc::Receiver<Result<(), String>>,
    server: Option<thread::JoinHandle<()>>,
    callers: Vec<(thread::JoinHandle<()>, mpsc::Receiver<()>)>,
    cleanup_started: Option<mpsc::Sender<()>>,
}
impl RunningStartup {
    fn start(finite: bool) -> (Self, mpsc::Receiver<SourceImage>, mpsc::Receiver<bool>) {
        Self::start_after(finite, None)
    }

    fn start_after(
        finite: bool,
        startup_gate: Option<mpsc::Receiver<()>>,
    ) -> (Self, mpsc::Receiver<SourceImage>, mpsc::Receiver<bool>) {
        let context = test_context(&format!(
            "readiness-preparation-{finite}-{}",
            Uuid::new_v4()
        ));
        let source = context.workspace_root.join("src");
        fs::write(source.join("One.bsl"), b"Procedure One()\nEndProcedure\n").unwrap();
        fs::create_dir_all(source.join("Nested")).unwrap();
        fs::write(
            source.join("Nested/Two.bsl"),
            b"Procedure Two()\nEndProcedure\n",
        )
        .unwrap();
        let identity = WorkspaceServiceIdentity::new(&context, &source).unwrap();
        let (entered, entries) = mpsc::channel();
        let (finished, completions) = mpsc::channel();
        let preparation = Arc::new(HeldPreparation {
            starts: AtomicUsize::new(0),
            active: Mutex::new(None),
            abort: AtomicBool::new(false),
            wake: Condvar::new(),
            gate: Mutex::new(()),
            entered,
            finished,
        });
        let token = Uuid::new_v4().to_string();
        let args = vec![
            "--workspace-service".to_string(),
            "--workspace-root".into(),
            identity.workspace_root.clone(),
            "--source-root".into(),
            identity.source_root.clone(),
            "--service-dir".into(),
            identity.service_dir.to_string_lossy().into_owned(),
            "--token".into(),
            token,
            "--core-identity".into(),
            CoreIdentity::production().as_str().to_string(),
            "--idle-secs".into(),
            "30".into(),
        ];
        let backend = Arc::clone(&preparation);
        let (completion, done) = mpsc::channel();
        let server = thread::spawn(move || {
            if let Some(gate) = startup_gate {
                let _ = gate.recv();
            }
            let result = unica_analysis_services::run_workspace_service_from_args(
                &args,
                backend,
                analysis_composition::service_environment(),
            );
            let _ = completion.send(result);
        });
        (
            Self {
                context: Some(context),
                identity,
                preparation,
                done,
                server: Some(server),
                callers: Vec::new(),
                cleanup_started: None,
            },
            entries,
            completions,
        )
    }

    fn record(&self) -> WorkspaceServiceRecord {
        let guard = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(record) = read_record(&self.identity) {
                return record;
            }
            assert!(
                Instant::now() < guard,
                "production startup did not publish its service record"
            );
            assert!(
                !self.server.as_ref().unwrap().is_finished(),
                "production startup exited before readiness"
            );
            thread::sleep(Duration::from_millis(5));
        }
    }

    fn ping(record: &WorkspaceServiceRecord) {
        let response = SYSTEM_SERVICE_CONNECTOR
            .send(
                record,
                ServiceRequest {
                    token: record.token.clone(),
                    kind: ServiceRequestKind::Ping,
                },
                &CancellationToken::new(),
                Duration::from_secs(2),
            )
            .expect("transport readiness must not start or wait for provider preparation");
        assert!(service_response_is_alive(&response), "{response:?}");
    }
}
struct StartupCleanup {
    context: ServiceTestContext,
    identity: WorkspaceServiceIdentity,
    done: mpsc::Receiver<Result<(), String>>,
    server: Option<thread::JoinHandle<()>>,
    callers: Vec<(thread::JoinHandle<()>, mpsc::Receiver<()>)>,
    started: Option<mpsc::Sender<()>>,
}
impl StartupCleanup {
    fn finish(mut self) -> Result<(), String> {
        // Startup may publish its record after Drop begins. Keep trying to
        // deliver shutdown while retaining the fixture and all thread handles.
        let server_result = loop {
            if let Some(record) = read_record(&self.identity) {
                let _ = SYSTEM_SERVICE_CONNECTOR.send(
                    &record,
                    ServiceRequest {
                        token: record.token.clone(),
                        kind: ServiceRequestKind::Shutdown,
                    },
                    &CancellationToken::new(),
                    Duration::from_secs(2),
                );
            }
            if let Some(started) = self.started.take() {
                let _ = started.send(());
            }
            match self.done.recv_timeout(Duration::from_millis(20)) {
                Ok(result) => break result,
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    break Err("owned server completion channel disconnected".to_string());
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
        };
        let mut cleanup_error = server_result.err();
        if let Some(server) = self.server.take() {
            if server.join().is_err() {
                cleanup_error = Some("owned server panicked".to_string());
            }
        }
        for (caller, done) in self.callers.drain(..) {
            let completed = done.recv();
            if caller.join().is_err() {
                cleanup_error = Some("owned caller panicked".to_string());
            } else if completed.is_err() {
                cleanup_error = Some("owned caller completion channel disconnected".to_string());
            }
        }
        cleanup(&self.context);
        cleanup_error.map_or(Ok(()), Err)
    }
}
impl Drop for RunningStartup {
    fn drop(&mut self) {
        self.preparation.abort.store(true, Ordering::Release);
        self.preparation.wake.notify_all();
        let resources = StartupCleanup {
            context: self.context.take().unwrap(),
            identity: self.identity.clone(),
            done: std::mem::replace(&mut self.done, mpsc::channel().1),
            server: self.server.take(),
            callers: std::mem::take(&mut self.callers),
            started: self.cleanup_started.take(),
        };
        let (finished, completion) = mpsc::channel();
        let cleanup_thread = thread::spawn(move || {
            let _ = finished.send(resources.finish());
        });
        // A failed observation must not delete files underneath live resources.
        // On timeout the cleanup thread retains ownership until every join.
        let cleanup_error = match completion.recv_timeout(Duration::from_secs(15)) {
            Ok(result) => {
                if cleanup_thread.join().is_err() {
                    Some("owned cleanup thread panicked".to_string())
                } else {
                    result.err()
                }
            }
            Err(error) => Some(format!("owned startup cleanup did not finish: {error}")),
        };
        if !thread::panicking() {
            assert!(cleanup_error.is_none(), "{cleanup_error:?}");
        }
    }
}

#[test]
fn startup_cleanup_retains_fixture_until_a_late_server_is_stopped() {
    let (release, startup_gate) = mpsc::channel();
    let (mut server, _, _) = RunningStartup::start_after(false, Some(startup_gate));
    let workspace = server.context.as_ref().unwrap().workspace_root.clone();
    let (started, cleanup_started) = mpsc::channel();
    server.cleanup_started = Some(started);
    thread::scope(|scope| {
        let dropper = scope.spawn(move || drop(server));
        cleanup_started
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        assert!(
            workspace.exists(),
            "cleanup removed the fixture before startup"
        );
        release.send(()).unwrap();
        dropper.join().unwrap();
    });
    assert!(!workspace.exists(), "the stopped fixture was not removed");
}

/// rule: INV.PERF.SERVICE-READINESS-WITHOUT-SOURCE-SCAN
#[test]
fn production_service_readiness_does_not_wait_for_provider_source_preparation() {
    for finite in [false, true] {
        let (mut server, entered, finished) = RunningStartup::start(finite);
        let expected = BTreeMap::from([
            (
                PathBuf::from("One.bsl"),
                b"Procedure One()\nEndProcedure\n".to_vec(),
            ),
            (
                PathBuf::from("Nested/Two.bsl"),
                b"Procedure Two()\nEndProcedure\n".to_vec(),
            ),
        ]);
        let record = server.record();
        RunningStartup::ping(&record);
        assert_eq!(
            server.preparation.starts.load(Ordering::Acquire),
            0,
            "startup/Ping prepared provider sources"
        );
        assert!(matches!(entered.try_recv(), Err(mpsc::TryRecvError::Empty)));

        let id = Uuid::new_v4().to_string();
        let record_for_caller = record.clone();
        let caller_id = id.clone();
        let (result_sender, result_receiver) = mpsc::channel();
        let (caller_done, caller_finished) = mpsc::channel();
        let caller = thread::spawn(move || {
            let deadline = if finite {
                ProviderDeadline::from_budget(Duration::from_secs(5))
            } else {
                ProviderDeadline::no_deadline()
            };
            let mut begin = prepared_begin(&caller_id);
            if let ServiceRequestKind::BslMcpBegin {
                deadline: offered, ..
            } = &mut begin
            {
                *offered = BslOperationDeadline::from_remaining(deadline.remaining());
            }
            let response = SYSTEM_SERVICE_CONNECTOR.send_operation_before(
                &record_for_caller,
                ServiceRequest {
                    token: record_for_caller.token.clone(),
                    kind: begin,
                },
                &CancellationToken::new(),
                deadline,
            );
            let _ = result_sender.send(response);
            let _ = caller_done.send(());
        });
        server.callers.push((caller, caller_finished));
        assert_eq!(
            entered.recv_timeout(Duration::from_secs(5)).unwrap(),
            expected,
            "the provider did not read the actual fixture tree"
        );
        assert_eq!(server.preparation.starts.load(Ordering::Acquire), 1);
        RunningStartup::ping(&record);
        assert!(
            matches!(finished.try_recv(), Err(mpsc::TryRecvError::Empty)),
            "preparation was not held during Ping"
        );

        let _ = send_test_request(
            &record,
            ServiceRequestKind::Cancel {
                operation_id: Uuid::new_v4().to_string(),
            },
        );
        RunningStartup::ping(&record);
        assert!(
            matches!(finished.try_recv(), Err(mpsc::TryRecvError::Empty)),
            "a different UUID cancelled the preparation"
        );
        assert!(
            !server
                .preparation
                .active
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .is_cancelled(),
            "a different UUID cancelled the actual provider token"
        );
        if !finite {
            assert!(send_test_request(&record, ServiceRequestKind::Cancel { operation_id: id }).ok);
        }
        // The real connector owns the budget and delivers Expire on its
        // separate control connection; the backend only sees cancellation.
        let result = result_receiver
            .recv_timeout(Duration::from_secs(10))
            .unwrap();
        if finite {
            let error = result.expect_err("the production caller must stop at its deadline");
            assert!(error.starts_with("timeout:"), "{error}");
            assert!(!error.contains("stop delivery is unconfirmed"), "{error}");
        } else {
            let response = result.unwrap();
            assert!(!response.ok, "{response:?}");
            assert!(
                response.error.as_deref().unwrap().starts_with("cancelled:"),
                "{response:?}"
            );
        }
        assert!(
            finished.recv_timeout(Duration::from_secs(2)).unwrap(),
            "provider was released by fixture cleanup rather than real cancellation"
        );
        assert_eq!(
            testing::tree_snapshot(&server.context.as_ref().unwrap().workspace_root.join("src")),
            expected
        );
        RunningStartup::ping(&record);
    }
}
