use super::*;
use std::collections::BTreeMap;
use std::sync::atomic::AtomicBool;
use unica_analysis_services::backend::BslBackend;
use unica_process::transport_failure::PrivateFailureSnapshot;

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
    context: WorkspaceContext,
    identity: WorkspaceServiceIdentity,
    preparation: Arc<HeldPreparation>,
    done: mpsc::Receiver<Result<(), String>>,
    server: Option<thread::JoinHandle<()>>,
    callers: Vec<(thread::JoinHandle<()>, mpsc::Receiver<()>)>,
}
impl RunningStartup {
    fn start(finite: bool) -> (Self, mpsc::Receiver<SourceImage>, mpsc::Receiver<bool>) {
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
            let result = unica_analysis_services::run_workspace_service_from_args(
                &args,
                backend,
                Arc::new(RlmWorkspaceBackend::new(Arc::new(
                    analysis_composition::AnalysisEnvironment,
                ))),
                analysis_composition::service_environment(),
            );
            let _ = completion.send(result);
        });
        (
            Self {
                context,
                identity,
                preparation,
                done,
                server: Some(server),
                callers: Vec::new(),
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
impl Drop for RunningStartup {
    fn drop(&mut self) {
        self.preparation.abort.store(true, Ordering::Release);
        self.preparation.wake.notify_all();
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
        let mut cleanup_error = None;
        match self.done.recv_timeout(Duration::from_secs(10)) {
            Ok(result) => {
                if let Some(server) = self.server.take() {
                    if server.join().is_err() {
                        cleanup_error = Some("owned server panicked".to_string());
                    }
                }
                if let Err(error) = result {
                    cleanup_error = Some(error);
                }
            }
            Err(error) => cleanup_error = Some(format!("owned server did not terminate: {error}")),
        }
        for (caller, done) in self.callers.drain(..) {
            match done.recv_timeout(Duration::from_secs(5)) {
                Ok(()) => {
                    if caller.join().is_err() {
                        cleanup_error = Some("owned caller panicked".to_string());
                    }
                }
                Err(error) => {
                    cleanup_error = Some(format!("owned caller did not terminate: {error}"))
                }
            }
        }
        cleanup(&self.context);
        if !thread::panicking() {
            assert!(cleanup_error.is_none(), "{cleanup_error:?}");
        }
    }
}

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
            testing::tree_snapshot(&server.context.workspace_root.join("src")),
            expected
        );
        RunningStartup::ping(&record);
    }
}
