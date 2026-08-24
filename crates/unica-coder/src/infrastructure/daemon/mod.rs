pub(crate) mod client;
pub(crate) mod identity;
pub(crate) mod protocol;
pub(crate) mod server;

#[cfg(test)]
mod tests {
    use super::client::{
        DaemonClient, DaemonClientConfig, ExistingDaemon, ManualDaemonClientClock,
    };
    use super::identity::{CoreIdentity, DaemonStateDirectory};
    use super::protocol::{
        parse_response, read_bounded_json_line, ClientRequest, DaemonErrorCode, EndpointRecord,
        InvocationRequest, InvocationResponse, ServerResponse, DAEMON_PROTOCOL_VERSION,
        MAX_JSON_LINE_BYTES,
    };
    use super::server::{
        install_handshake_pause, run_daemon, workspace_capacity_protocol_code_for_test,
        ActorBoundExecution, ActorBoundInvocation, CanonicalInvocationService, DaemonServerConfig,
        MAX_HANDSHAKES, MAX_OWNER_SESSIONS,
    };
    use crate::application::invocation_store::{
        InvocationStore, InvocationStoreError, NewInvocationRecord, SafeFailureReason,
        SafeStatusMessage, StoredInvocationRecord, TaskTransition, ToolIdentity,
    };
    use crate::application::operation_descriptors::{ExecutionClass, KnownLongReason};
    use crate::domain::cancellation::CancellationToken;
    use crate::domain::invocation::{DomainResult, InvocationFailure, InvocationStatus, TaskId};
    use crate::infrastructure::platform::testing::{
        create_directory_link_fixture_for_test, set_unix_mode_for_test, unix_mode_for_test,
        FileLinkFixtureOutcome,
    };
    use crate::infrastructure::task_store::{FileInvocationStore, SystemEpochMillisClock};
    use std::collections::HashMap;
    use std::io::{BufReader, Cursor, Write};
    use std::net::{Ipv4Addr, TcpListener, TcpStream};
    use std::path::PathBuf;
    use std::process::{Child, Command, Stdio};
    use std::str::FromStr;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{mpsc, Arc, Mutex};
    use std::thread;
    use std::time::{Duration, Instant};

    // These waits only bound native thread/TCP coordination in integration tests. They exceed
    // the actor's seven-second operation budget so parallel test-runner scheduling cannot be
    // mistaken for the product deadline being exercised inside the daemon.
    const INTEGRATION_COORDINATION_TIMEOUT: Duration = Duration::from_secs(10);
    const INTEGRATION_TASK_WAIT_MS: u64 = 7_000;
    const FAIL_STOP_PROCESS_FIXTURE: &str = "UNICA_FAIL_STOP_PROCESS_FIXTURE";

    struct FailStopFixtureChild {
        child: Child,
        finished: bool,
    }

    impl Drop for FailStopFixtureChild {
        fn drop(&mut self) {
            if !self.finished {
                let _ = self.child.kill();
                let _ = self.child.wait();
            }
        }
    }

    struct BlockingTerminalFileStore {
        inner: FileInvocationStore,
    }

    impl InvocationStore for BlockingTerminalFileStore {
        fn create(
            &self,
            record: NewInvocationRecord,
        ) -> Result<StoredInvocationRecord, InvocationStoreError> {
            self.inner.create(record)
        }

        fn create_working(
            &self,
            record: NewInvocationRecord,
        ) -> Result<StoredInvocationRecord, InvocationStoreError> {
            self.inner.create_working(record)
        }

        fn get(&self, task_id: TaskId) -> Result<StoredInvocationRecord, InvocationStoreError> {
            self.inner.get(task_id)
        }

        fn update(
            &self,
            _task_id: TaskId,
            _transition: TaskTransition,
        ) -> Result<StoredInvocationRecord, InvocationStoreError> {
            loop {
                thread::park();
            }
        }

        fn cancel(
            &self,
            task_id: TaskId,
            status_message: SafeStatusMessage,
        ) -> Result<StoredInvocationRecord, InvocationStoreError> {
            self.inner.cancel(task_id, status_message)
        }
    }

    struct ProcessCountingService {
        executions: PathBuf,
    }

    impl CanonicalInvocationService for ProcessCountingService {
        fn prepare(
            &self,
            _invocation: &ActorBoundInvocation,
        ) -> Result<ExecutionClass, Box<DomainResult>> {
            Ok(ExecutionClass::KnownLong(KnownLongReason::ExternalProcess))
        }

        fn execute(
            &self,
            _invocation: &ActorBoundExecution,
            _cancellation: CancellationToken,
        ) -> Result<DomainResult, InvocationFailure> {
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.executions)
                .unwrap();
            writeln!(file, "execution").unwrap();
            file.sync_all().unwrap();
            Ok(DomainResult::success("staged child result"))
        }
    }

    fn alternate_identity() -> CoreIdentity {
        CoreIdentity::from_str("eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee")
            .unwrap()
    }

    fn server_config(root: PathBuf, identity: CoreIdentity) -> DaemonServerConfig {
        DaemonServerConfig::new(physical_root(&root), identity, Duration::from_millis(350))
    }

    fn physical_root(root: &std::path::Path) -> PathBuf {
        std::fs::canonicalize(root).unwrap()
    }

    fn wait_for_record(
        root: &std::path::Path,
        identity: &CoreIdentity,
    ) -> (DaemonStateDirectory, EndpointRecord) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let directory = DaemonStateDirectory::open(&physical_root(root), identity).unwrap();
            if let Some(record) = directory.read_endpoint_record().unwrap() {
                return (directory, record);
            }
            assert!(
                Instant::now() < deadline,
                "daemon endpoint was not published"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn wait_for_replaced_record(
        root: &std::path::Path,
        identity: &CoreIdentity,
        replaced_pid: u32,
    ) -> EndpointRecord {
        let deadline = Instant::now() + INTEGRATION_COORDINATION_TIMEOUT;
        loop {
            let directory = DaemonStateDirectory::open(root, identity).unwrap();
            if let Some(record) = directory.read_endpoint_record().unwrap() {
                if record.pid() != replaced_pid {
                    return record;
                }
            }
            assert!(
                Instant::now() < deadline,
                "successor did not replace the stale endpoint"
            );
            thread::sleep(Duration::from_millis(20));
        }
    }

    fn spawn_fail_stop_fixture(
        mode: &str,
        state_root: &std::path::Path,
        store_root: &std::path::Path,
        workspace: &std::path::Path,
        executions: &std::path::Path,
    ) -> FailStopFixtureChild {
        let child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "infrastructure::daemon::tests::durability_fail_stop_process_fixture",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(FAIL_STOP_PROCESS_FIXTURE, mode)
            .env("UNICA_FAIL_STOP_STATE_ROOT", state_root)
            .env("UNICA_FAIL_STOP_STORE_ROOT", store_root)
            .env("UNICA_FAIL_STOP_WORKSPACE", workspace)
            .env("UNICA_FAIL_STOP_EXECUTIONS", executions)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        FailStopFixtureChild {
            child,
            finished: false,
        }
    }

    fn assert_child_success_with_stderr(child: &mut FailStopFixtureChild, fixture: &str) {
        let deadline = Instant::now() + INTEGRATION_COORDINATION_TIMEOUT;
        loop {
            if let Some(status) = child.child.try_wait().unwrap() {
                if !status.success() {
                    let stderr = child
                        .child
                        .stderr
                        .take()
                        .map(|stderr| std::io::read_to_string(stderr).unwrap_or_default())
                        .unwrap_or_default();
                    panic!("{fixture} failed with {status}: {stderr}");
                }
                child.finished = true;
                return;
            }
            if Instant::now() >= deadline {
                let _ = child.child.kill();
                let _ = child.child.wait();
                child.finished = true;
                panic!("{fixture} did not exit");
            }
            thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn durability_fail_stop_process_fixture() {
        let Some(mode) = std::env::var_os(FAIL_STOP_PROCESS_FIXTURE) else {
            return;
        };
        let state_root = PathBuf::from(std::env::var_os("UNICA_FAIL_STOP_STATE_ROOT").unwrap());
        let store_root = PathBuf::from(std::env::var_os("UNICA_FAIL_STOP_STORE_ROOT").unwrap());
        let executions = PathBuf::from(std::env::var_os("UNICA_FAIL_STOP_EXECUTIONS").unwrap());
        let (store, _) =
            FileInvocationStore::open(&store_root, Arc::new(SystemEpochMillisClock)).unwrap();
        let identity = CoreIdentity::production();
        match mode.to_string_lossy().as_ref() {
            "fault" => {
                let config = DaemonServerConfig::new(state_root, identity, Duration::from_secs(30))
                    .with_invocation_store_for_test(Arc::new(BlockingTerminalFileStore {
                        inner: store,
                    }))
                    .with_invocation_service(Arc::new(ProcessCountingService { executions }))
                    .with_reconciliation_budget_for_test(Duration::from_millis(100));
                run_daemon(config).unwrap();
            }
            "successor" => {
                let config = DaemonServerConfig::new(state_root, identity, Duration::from_secs(2))
                    .with_invocation_store_for_test(Arc::new(store));
                run_daemon(config).unwrap();
            }
            other => panic!("unknown fail-stop process fixture mode: {other}"),
        }
    }

    fn write_json_line<T: serde::Serialize>(stream: &mut TcpStream, value: &T) {
        let mut bytes = serde_json::to_vec(value).unwrap();
        bytes.push(b'\n');
        stream.write_all(&bytes).unwrap();
        stream.flush().unwrap();
    }

    fn connect_raw_owner(
        record: &EndpointRecord,
        identity: &CoreIdentity,
    ) -> (TcpStream, ServerResponse) {
        let mut stream = TcpStream::connect(record.loopback_addr().unwrap()).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let hello = ClientRequest::hello_with_owner_for_test(
            record.token().to_string(),
            identity.clone(),
            uuid::Uuid::new_v4().to_string(),
        );
        write_json_line(&mut stream, &hello);
        let response = serde_json::from_slice(
            &read_bounded_json_line(&mut BufReader::new(stream.try_clone().unwrap())).unwrap(),
        )
        .unwrap();
        (stream, response)
    }

    struct BlockingCanonicalService {
        executions: Arc<AtomicUsize>,
        entered: mpsc::Sender<()>,
    }

    struct BoundReadingService {
        observed: mpsc::Sender<(crate::domain::invocation::SafeIdentityHash, Vec<u8>)>,
    }

    struct StagedActorService {
        entered: mpsc::Sender<()>,
        release: Mutex<mpsc::Receiver<()>>,
        staged_summary: &'static str,
    }

    #[derive(Default)]
    struct DaemonMemoryStore {
        records: Mutex<HashMap<TaskId, StoredInvocationRecord>>,
        update_attempts: AtomicUsize,
        fail_updates: AtomicUsize,
    }

    impl InvocationStore for DaemonMemoryStore {
        fn create(
            &self,
            record: NewInvocationRecord,
        ) -> Result<StoredInvocationRecord, InvocationStoreError> {
            let stored = record.into_stored(1);
            self.records
                .lock()
                .unwrap()
                .insert(stored.task_id, stored.clone());
            Ok(stored)
        }

        fn create_working(
            &self,
            record: NewInvocationRecord,
        ) -> Result<StoredInvocationRecord, InvocationStoreError> {
            let stored = record.into_working_stored(1);
            self.records
                .lock()
                .unwrap()
                .insert(stored.task_id, stored.clone());
            Ok(stored)
        }

        fn get(&self, task_id: TaskId) -> Result<StoredInvocationRecord, InvocationStoreError> {
            self.records
                .lock()
                .unwrap()
                .get(&task_id)
                .cloned()
                .ok_or(InvocationStoreError::NotFound)
        }

        fn update(
            &self,
            task_id: TaskId,
            transition: TaskTransition,
        ) -> Result<StoredInvocationRecord, InvocationStoreError> {
            self.update_attempts.fetch_add(1, Ordering::SeqCst);
            if self.fail_updates.load(Ordering::SeqCst) != 0 {
                return Err(InvocationStoreError::Storage(
                    "permanent daemon terminal failure".into(),
                ));
            }
            let mut records = self.records.lock().unwrap();
            let record = records
                .get_mut(&task_id)
                .ok_or(InvocationStoreError::NotFound)?;
            if record.is_terminal() {
                return Err(InvocationStoreError::InvalidTransition {
                    from: record.status,
                    attempted: "update",
                });
            }
            match transition {
                TaskTransition::StartWorking { status_message } => {
                    record.status = InvocationStatus::Working;
                    record.status_message = status_message;
                }
                TaskTransition::Complete {
                    status_message,
                    result,
                } => {
                    record.status = InvocationStatus::Completed;
                    record.status_message = status_message;
                    record.result = Some(*result);
                }
                TaskTransition::Fail {
                    status_message,
                    reason,
                } => {
                    record.status = InvocationStatus::Failed;
                    record.status_message = status_message;
                    record.failure_reason = Some(reason);
                }
            }
            Ok(record.clone())
        }

        fn cancel(
            &self,
            task_id: TaskId,
            status_message: SafeStatusMessage,
        ) -> Result<StoredInvocationRecord, InvocationStoreError> {
            let mut records = self.records.lock().unwrap();
            let record = records
                .get_mut(&task_id)
                .ok_or(InvocationStoreError::NotFound)?;
            if !record.is_terminal() {
                record.status = InvocationStatus::Cancelled;
                record.status_message = status_message;
            }
            Ok(record.clone())
        }
    }

    impl CanonicalInvocationService for BoundReadingService {
        fn prepare(
            &self,
            invocation: &ActorBoundInvocation,
        ) -> Result<ExecutionClass, Box<DomainResult>> {
            assert_eq!(invocation.tool(), ToolIdentity::Run);
            assert!(invocation.arguments().is_empty());
            Ok(ExecutionClass::KnownLong(KnownLongReason::ExternalProcess))
        }

        fn execute(
            &self,
            invocation: &ActorBoundExecution,
            _cancellation: CancellationToken,
        ) -> Result<DomainResult, InvocationFailure> {
            assert_eq!(invocation.tool(), ToolIdentity::Run);
            assert!(invocation.arguments().is_empty());
            let bytes = invocation
                .read_relative_file(std::path::Path::new("Module.bsl"), 1_024)
                .map_err(|_| InvocationFailure::new("workspace_changed", "bound read failed"))?;
            self.observed
                .send((invocation.workspace_identity_hash().clone(), bytes))
                .unwrap();
            Ok(DomainResult::success("actor-bound read"))
        }
    }

    impl CanonicalInvocationService for BlockingCanonicalService {
        fn prepare(
            &self,
            _request: &ActorBoundInvocation,
        ) -> Result<ExecutionClass, Box<DomainResult>> {
            Ok(ExecutionClass::KnownLong(KnownLongReason::ExternalProcess))
        }

        fn execute(
            &self,
            _request: &ActorBoundExecution,
            cancellation: CancellationToken,
        ) -> Result<DomainResult, InvocationFailure> {
            self.executions.fetch_add(1, Ordering::SeqCst);
            self.entered.send(()).unwrap();
            while !cancellation.is_cancelled() {
                thread::yield_now();
            }
            Err(InvocationFailure::new("cancelled", "test cancellation"))
        }
    }

    impl CanonicalInvocationService for StagedActorService {
        fn prepare(
            &self,
            _invocation: &ActorBoundInvocation,
        ) -> Result<ExecutionClass, Box<DomainResult>> {
            Ok(ExecutionClass::KnownLong(KnownLongReason::ExternalProcess))
        }

        fn execute(
            &self,
            invocation: &ActorBoundExecution,
            _cancellation: CancellationToken,
        ) -> Result<DomainResult, InvocationFailure> {
            let _ = invocation
                .read_relative_file(std::path::Path::new("Module.bsl"), 1_024)
                .map_err(|_| InvocationFailure::new("workspace_changed", "bound read failed"))?;
            self.entered.send(()).unwrap();
            self.release.lock().unwrap().recv().unwrap();
            Ok(DomainResult::success(self.staged_summary))
        }
    }

    #[test]
    fn core_identity_is_closed_compile_time_abi_protocol_digest() {
        let production = CoreIdentity::production();
        assert_eq!(production.as_str().len(), 64);
        assert!(production
            .as_str()
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit()));
        assert_ne!(production.as_str(), env!("CARGO_PKG_VERSION"));
        assert_ne!(production, alternate_identity());
        assert!(CoreIdentity::from_str(&production.as_str().to_uppercase()).is_err());
        assert!(CoreIdentity::from_str("v0.13").is_err());
    }

    #[test]
    fn world_readable_identity_directory_is_rejected() {
        let root = tempfile::tempdir().unwrap();
        let physical = physical_root(root.path());
        let identity = CoreIdentity::production();
        let path = DaemonStateDirectory::path_for(&physical, &identity);
        std::fs::create_dir_all(&path).unwrap();
        if !set_unix_mode_for_test(&path, 0o755).unwrap() {
            return;
        }

        let error = DaemonStateDirectory::open(&physical, &identity).unwrap_err();
        assert!(error.contains("owner-only"), "{error}");
        assert_eq!(unix_mode_for_test(&path).unwrap(), Some(0o755));
    }

    #[test]
    fn symlinked_provider_state_root_is_rejected_before_creating_identity_state() {
        let fixture = tempfile::tempdir().unwrap();
        let target = fixture.path().join("redirected-parent");
        std::fs::create_dir(&target).unwrap();
        let routed_parent = fixture.path().join("provider-parent-link");
        match create_directory_link_fixture_for_test(&target, &routed_parent).unwrap() {
            FileLinkFixtureOutcome::Created => {}
            FileLinkFixtureOutcome::Unsupported
            | FileLinkFixtureOutcome::WindowsPrivilegeUnavailable => return,
        }
        let routed = routed_parent.join("provider-state-created-through-link");
        let identity = CoreIdentity::production();

        assert!(DaemonStateDirectory::open(&routed, &identity).is_err());
        assert!(
            !target.join("provider-state-created-through-link").exists(),
            "rejected ambient symlink must not receive even the provider-state directory"
        );
    }

    #[test]
    fn missing_provider_state_root_is_created_before_private_identity_child() {
        let fixture = tempfile::tempdir().unwrap();
        let physical_fixture = std::fs::canonicalize(fixture.path()).unwrap();
        let state_root = physical_fixture.join("cold").join("provider-state");
        let identity = CoreIdentity::production();

        let state = DaemonStateDirectory::open(&state_root, &identity).unwrap();

        assert!(state_root.is_dir());
        assert_eq!(
            state.path(),
            DaemonStateDirectory::path_for(&state_root, &identity)
        );
    }

    #[test]
    fn protocol_rejects_oversized_and_noncanonical_lines() {
        let oversized = vec![b'x'; MAX_JSON_LINE_BYTES + 1];
        let error =
            read_bounded_json_line(&mut BufReader::new(Cursor::new(oversized))).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);

        let unknown = format!(
            "{{\"kind\":\"ping\",\"unknown\":true,\"protocolVersion\":{DAEMON_PROTOCOL_VERSION}}}\n"
        );
        let error = serde_json::from_slice::<ClientRequest>(unknown.as_bytes()).unwrap_err();
        assert!(error.to_string().contains("unknown field"));
    }

    #[test]
    fn invocation_protocol_round_trips_all_four_strict_requests_and_closed_responses() {
        let submit = ClientRequest::submit_invocation(
            InvocationRequest::new(
                ToolIdentity::Check,
                serde_json::json!({"cwd": "/workspace"}),
                "/workspace",
                6_875,
            )
            .unwrap(),
        );
        let task_id = TaskId::new();
        let requests = [
            submit,
            ClientRequest::get_task(task_id),
            ClientRequest::wait_task(task_id, 250),
            ClientRequest::cancel_task(task_id),
        ];
        for request in requests {
            let wire = serde_json::to_vec(&request).unwrap();
            assert_eq!(super::protocol::parse_request(&wire).unwrap(), request);
        }

        let direct =
            ServerResponse::invocation(InvocationResponse::Direct(DomainResult::success("ready")));
        let task = ServerResponse::invocation(InvocationResponse::Task(
            super::protocol::DaemonTaskSnapshot::working_for_test(task_id),
        ));
        let responses = [
            direct,
            task,
            ServerResponse::error(DaemonErrorCode::WorkspaceCapacity),
            ServerResponse::error(DaemonErrorCode::WorkspaceRegistryFailed),
            ServerResponse::error(DaemonErrorCode::TaskCapacity),
            ServerResponse::error(DaemonErrorCode::DurabilityUncertain),
        ];
        for response in responses {
            let wire = serde_json::to_vec(&response).unwrap();
            assert_eq!(parse_response(&wire).unwrap(), response);
        }
        assert_eq!(
            serde_json::to_value(ServerResponse::error(
                DaemonErrorCode::WorkspaceRegistryFailed
            ))
            .unwrap()["code"],
            "workspace_registry_failed"
        );
        assert_eq!(
            serde_json::to_value(ServerResponse::error(DaemonErrorCode::TaskCapacity)).unwrap()
                ["code"],
            "task_capacity"
        );
        assert_eq!(
            serde_json::to_value(ServerResponse::error(DaemonErrorCode::DurabilityUncertain))
                .unwrap()["code"],
            "durability_uncertain"
        );

        let mut noncanonical_response = serde_json::to_value(ServerResponse::invocation(
            InvocationResponse::Direct(DomainResult::success("ready")),
        ))
        .unwrap();
        noncanonical_response["outcome"]
            .as_object_mut()
            .unwrap()
            .insert("extra".to_string(), serde_json::json!(true));
        assert!(parse_response(&serde_json::to_vec(&noncanonical_response).unwrap()).is_err());

        for invalid in [
            br#"{"kind":"get_task","taskId":"not-a-task"}"#.as_slice(),
            br#"{"kind":"wait_task","taskId":"00000000-0000-0000-0000-000000000000","waitMs":1}"#.as_slice(),
            br#"{"kind":"cancel_task","taskId":"ffffffff-ffff-4fff-8fff-ffffffffffff","extra":true}"#.as_slice(),
            br#"{"kind":"unknown_future_message"}"#.as_slice(),
        ] {
            assert!(super::protocol::parse_request(invalid).is_err());
        }
    }

    #[test]
    fn daemon_executes_one_canonical_invocation_and_poll_cancel_never_relaunches_it() {
        let root = tempfile::tempdir().unwrap();
        let physical = physical_root(root.path());
        let workspace_hint = physical.to_string_lossy().into_owned();
        let identity = CoreIdentity::production();
        let executions = Arc::new(AtomicUsize::new(0));
        let (entered, entered_wait) = mpsc::channel();
        let service = Arc::new(BlockingCanonicalService {
            executions: Arc::clone(&executions),
            entered,
        });
        let config = server_config(root.path().to_path_buf(), identity.clone())
            .with_invocation_service(service);
        let server = thread::spawn(move || run_daemon(config));
        let (_directory, _record) = wait_for_record(root.path(), &identity);
        let client = DaemonClient::new(DaemonClientConfig::existing_only(physical, identity));
        let mut owner = match client.connect_existing().unwrap() {
            ExistingDaemon::Connected(owner) => owner,
            ExistingDaemon::Absent => panic!("published daemon must connect"),
        };

        let outcome = owner
            .submit_invocation(
                InvocationRequest::new(
                    ToolIdentity::Run,
                    serde_json::json!({}),
                    workspace_hint,
                    7_000,
                )
                .unwrap(),
            )
            .unwrap();
        let task_id = match outcome {
            InvocationResponse::Task(task) => task.task_id,
            other => panic!("known-long request did not return a task: {other:?}"),
        };
        entered_wait.recv().unwrap();
        assert_eq!(
            owner.get_task(task_id).unwrap().status,
            InvocationStatus::Working
        );
        assert_eq!(
            owner.wait_task(task_id, 0).unwrap().status,
            InvocationStatus::Working
        );
        assert_eq!(
            owner.cancel_task(task_id).unwrap().status,
            InvocationStatus::Cancelled
        );
        assert_eq!(
            owner.cancel_task(task_id).unwrap().status,
            InvocationStatus::Cancelled
        );
        assert_eq!(
            owner.get_task(task_id).unwrap().status,
            InvocationStatus::Cancelled
        );
        assert_eq!(executions.load(Ordering::SeqCst), 1);

        drop(owner);
        server.join().unwrap().unwrap();
    }

    #[test]
    fn durability_uncertainty_stops_the_daemon_before_idle_grace() {
        let root = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let physical = physical_root(root.path());
        let identity = CoreIdentity::production();
        let store = Arc::new(DaemonMemoryStore::default());
        store.fail_updates.store(1, Ordering::SeqCst);
        let config =
            DaemonServerConfig::new(physical.clone(), identity.clone(), Duration::from_secs(30))
                .with_invocation_store_for_test(store.clone())
                .with_reconciliation_budget_for_test(Duration::from_millis(100));
        let (done, done_wait) = mpsc::channel();
        thread::spawn(move || done.send(run_daemon(config)).unwrap());
        let (directory, record) = wait_for_record(root.path(), &identity);
        let client = DaemonClient::new(DaemonClientConfig::existing_only(physical, identity));
        let mut owner = match client.connect_existing().unwrap() {
            ExistingDaemon::Connected(owner) => owner,
            ExistingDaemon::Absent => panic!("published daemon must connect"),
        };
        let response = owner
            .submit_invocation(
                InvocationRequest::new(
                    ToolIdentity::Run,
                    serde_json::json!({}),
                    physical_root(workspace.path()).to_string_lossy(),
                    0,
                )
                .unwrap(),
            )
            .unwrap();
        assert!(matches!(response, InvocationResponse::Task(_)));

        assert!(matches!(
            done_wait.recv_timeout(INTEGRATION_COORDINATION_TIMEOUT),
            Ok(Ok(()))
        ));
        assert!(store.update_attempts.load(Ordering::SeqCst) <= 10);
        assert!(store
            .records
            .lock()
            .unwrap()
            .values()
            .all(|record| record.status == InvocationStatus::Working && record.result.is_none()));
        assert!(
            directory.read_endpoint_record().unwrap().is_some(),
            "restart handoff must retain the old PID endpoint until process death"
        );
        assert!(
            TcpStream::connect(record.loopback_addr().unwrap()).is_err(),
            "restart handoff must close admission while retaining the PID-bound record"
        );
        drop(owner);
    }

    #[test]
    fn process_death_owns_fail_stop_handoff_and_recovery() {
        let fixture = tempfile::tempdir().unwrap();
        let state_root = physical_root(fixture.path());
        let store_root = state_root.join("task-store");
        let workspace = state_root.join("workspace");
        let executions = state_root.join("executions.log");
        std::fs::create_dir(&store_root).unwrap();
        std::fs::create_dir(&workspace).unwrap();
        let identity = CoreIdentity::production();

        let mut faulting =
            spawn_fail_stop_fixture("fault", &state_root, &store_root, &workspace, &executions);
        let (_directory, first_endpoint) = wait_for_record(&state_root, &identity);
        let client = DaemonClient::new(DaemonClientConfig::existing_only(
            state_root.clone(),
            identity.clone(),
        ));
        let mut owner = match client.connect_existing().unwrap() {
            ExistingDaemon::Connected(owner) => owner,
            ExistingDaemon::Absent => panic!("fault fixture must publish an endpoint"),
        };
        let task = owner
            .submit_invocation(
                InvocationRequest::new(
                    ToolIdentity::Run,
                    serde_json::json!({}),
                    workspace.to_string_lossy(),
                    0,
                )
                .unwrap(),
            )
            .unwrap();
        let task_id = match task {
            InvocationResponse::Task(snapshot) => snapshot.task_id,
            other => panic!("fault fixture did not materialize a task: {other:?}"),
        };
        assert!(matches!(
            FileInvocationStore::open(&store_root, Arc::new(SystemEpochMillisClock)),
            Err(InvocationStoreError::AlreadyOwned)
        ));
        drop(owner);
        assert_child_success_with_stderr(&mut faulting, "faulting daemon fixture");

        let stale = DaemonStateDirectory::open(&state_root, &identity)
            .unwrap()
            .read_endpoint_record()
            .unwrap()
            .expect("fail-stop process must leave its PID-bound endpoint");
        assert_eq!(stale.pid(), first_endpoint.pid());

        let mut successor = spawn_fail_stop_fixture(
            "successor",
            &state_root,
            &store_root,
            &workspace,
            &executions,
        );
        let successor_endpoint =
            wait_for_replaced_record(&state_root, &identity, first_endpoint.pid());
        assert_ne!(successor_endpoint.pid(), first_endpoint.pid());
        let successor_client = DaemonClient::new(DaemonClientConfig::existing_only(
            state_root.clone(),
            identity,
        ));
        let mut successor_owner = match successor_client.connect_existing().unwrap() {
            ExistingDaemon::Connected(owner) => owner,
            ExistingDaemon::Absent => panic!("successor must own the endpoint"),
        };
        successor_owner.ping().unwrap();
        drop(successor_owner);
        assert_child_success_with_stderr(&mut successor, "successor daemon fixture");

        let (reopened, _) =
            FileInvocationStore::open(&store_root, Arc::new(SystemEpochMillisClock)).unwrap();
        let recovered = reopened.get(task_id).unwrap();
        assert_eq!(recovered.status, InvocationStatus::Failed);
        assert_eq!(
            recovered.failure_reason,
            Some(SafeFailureReason::Interrupted)
        );
        assert!(recovered.result.is_none());
        assert_eq!(
            std::fs::read_to_string(executions).unwrap().lines().count(),
            1,
            "successor recovery must not re-execute domain work"
        );
    }

    #[test]
    fn daemon_store_is_bounded_and_fail_stop_is_process_owned() {
        crate::application::invocation_store_actor::tests::daemon_store_actor_bounds_blocked_adapter_without_waiting();
        crate::infrastructure::task_store::tests::file_invocation_store_bounds_and_retention_are_enforced();
        crate::infrastructure::daemon::server::actor_capacity_tests::restart_request_does_not_claim_noncooperative_actor_released_in_process();
        process_death_owns_fail_stop_handoff_and_recovery();
    }

    fn canonical_service_reads_only_actor_bound_roots_and_persists_the_same_identity() {
        let daemon_root = tempfile::tempdir().unwrap();
        let workspace_a = tempfile::tempdir().unwrap();
        let workspace_b = tempfile::tempdir().unwrap();
        std::fs::write(workspace_a.path().join("Module.bsl"), b"workspace A").unwrap();
        std::fs::write(workspace_b.path().join("Module.bsl"), b"workspace B").unwrap();
        let identity = CoreIdentity::production();
        let (observed, observed_wait) = mpsc::channel();
        let store = Arc::new(DaemonMemoryStore::default());
        let config = server_config(daemon_root.path().to_path_buf(), identity.clone())
            .with_invocation_store_for_test(store.clone())
            .with_invocation_service(Arc::new(BoundReadingService { observed }));
        let server = thread::spawn(move || run_daemon(config));
        let (_directory, _record) = wait_for_record(daemon_root.path(), &identity);
        let client = DaemonClient::new(DaemonClientConfig::existing_only(
            physical_root(daemon_root.path()),
            identity,
        ));
        let mut owner = match client.connect_existing().unwrap() {
            ExistingDaemon::Connected(owner) => owner,
            ExistingDaemon::Absent => panic!("published daemon must connect"),
        };

        for (workspace, expected) in [
            (workspace_a.path(), b"workspace A".as_slice()),
            (workspace_b.path(), b"workspace B".as_slice()),
        ] {
            let response = owner
                .submit_invocation(
                    InvocationRequest::new(
                        ToolIdentity::Run,
                        serde_json::json!({}),
                        physical_root(workspace).to_string_lossy(),
                        0,
                    )
                    .unwrap(),
                )
                .unwrap();
            let task_id = match response {
                InvocationResponse::Task(snapshot) => snapshot.task_id,
                other => panic!("zero-budget actor-bound request was not durable: {other:?}"),
            };
            let (actor_hash, bytes) = observed_wait
                .recv_timeout(INTEGRATION_COORDINATION_TIMEOUT)
                .unwrap();
            assert_eq!(bytes, expected);
            let terminal = owner.wait_task(task_id, INTEGRATION_TASK_WAIT_MS).unwrap();
            assert_eq!(terminal.status, InvocationStatus::Completed);
            let record = store.get(task_id).unwrap();
            assert_eq!(record.workspace_identity_hash, actor_hash);
        }

        drop(owner);
        server.join().unwrap().unwrap();
    }

    fn run_actor_swap_case(replace_root: bool) {
        let daemon_root = tempfile::tempdir().unwrap();
        let workspace_parent = tempfile::tempdir().unwrap();
        let workspace = workspace_parent.path().join("workspace");
        std::fs::create_dir(&workspace).unwrap();
        std::fs::write(workspace.join("Module.bsl"), b"initial").unwrap();
        let identity = CoreIdentity::production();
        let (entered, entered_wait) = mpsc::channel();
        let (release, release_wait) = mpsc::channel();
        let staged = "STAGED_BYTES_MUST_NOT_ESCAPE_AFTER_SWAP";
        let store = Arc::new(DaemonMemoryStore::default());
        let config = server_config(daemon_root.path().to_path_buf(), identity.clone())
            .with_invocation_store_for_test(store)
            .with_invocation_service(Arc::new(StagedActorService {
                entered,
                release: Mutex::new(release_wait),
                staged_summary: staged,
            }));
        let server = thread::spawn(move || run_daemon(config));
        let (_directory, _record) = wait_for_record(daemon_root.path(), &identity);
        let client = DaemonClient::new(DaemonClientConfig::existing_only(
            physical_root(daemon_root.path()),
            identity,
        ));
        let mut owner = match client.connect_existing().unwrap() {
            ExistingDaemon::Connected(owner) => owner,
            ExistingDaemon::Absent => panic!("published daemon must connect"),
        };
        let response = owner
            .submit_invocation(
                InvocationRequest::new(
                    ToolIdentity::Run,
                    serde_json::json!({"args": {"ambientRoot": "/tmp/foreign"}}),
                    physical_root(&workspace).to_string_lossy(),
                    0,
                )
                .unwrap(),
            )
            .unwrap();
        let task_id = match response {
            InvocationResponse::Task(snapshot) => snapshot.task_id,
            other => panic!("expected task: {other:?}"),
        };
        entered_wait
            .recv_timeout(INTEGRATION_COORDINATION_TIMEOUT)
            .unwrap();
        if replace_root {
            let retained = workspace_parent.path().join("retained-old-workspace");
            std::fs::rename(&workspace, &retained).unwrap();
            std::fs::create_dir(&workspace).unwrap();
            std::fs::write(workspace.join("Module.bsl"), b"foreign replacement").unwrap();
        } else {
            std::fs::write(workspace.join("Module.bsl"), b"changed revision").unwrap();
        }
        release.send(()).unwrap();
        let terminal = owner.wait_task(task_id, INTEGRATION_TASK_WAIT_MS).unwrap();
        assert_eq!(terminal.status, InvocationStatus::Failed);
        assert!(terminal.result.is_none());
        assert_eq!(
            terminal.failure,
            Some(InvocationFailure::new(
                "invocation_failed",
                "daemon invocation failed",
            ))
        );
        assert!(!serde_json::to_string(&terminal).unwrap().contains(staged));

        drop(owner);
        server.join().unwrap().unwrap();
    }

    fn actor_bound_publication_rejects_root_replacement_and_hides_staged_bytes() {
        run_actor_swap_case(true);
    }

    fn actor_bound_publication_rejects_revision_swap_and_hides_staged_bytes() {
        run_actor_swap_case(false);
    }

    #[test]
    fn canonical_service_boundary_exposes_no_raw_request_or_workspace_hint() {
        let source = include_str!("server.rs");
        let trait_start = source
            .find("pub(crate) trait CanonicalInvocationService")
            .expect("canonical service trait");
        let trait_end = source[trait_start..]
            .find("\n}\n\nstruct DormantCanonicalV13Service")
            .expect("canonical service trait end")
            + trait_start;
        let boundary = &source[trait_start..trait_end];
        assert!(boundary.contains("&ActorBoundInvocation"));
        assert!(boundary.contains("&ActorBoundExecution"));
        assert!(!boundary.contains("InvocationRequest"));
        assert!(!boundary.contains("workspace_hint"));
    }

    #[test]
    fn canonical_invocation_authority_is_actor_bound_and_revision_fenced() {
        canonical_service_boundary_exposes_no_raw_request_or_workspace_hint();
        canonical_service_reads_only_actor_bound_roots_and_persists_the_same_identity();
        actor_bound_publication_rejects_root_replacement_and_hides_staged_bytes();
        actor_bound_publication_rejects_revision_swap_and_hides_staged_bytes();
    }

    #[test]
    fn workspace_actor_capacity_has_a_closed_retryable_protocol_code() {
        assert_eq!(
            workspace_capacity_protocol_code_for_test(),
            DaemonErrorCode::WorkspaceCapacity
        );
        let wire = serde_json::to_value(ServerResponse::error(DaemonErrorCode::WorkspaceCapacity))
            .unwrap();
        assert_eq!(wire["code"], "workspace_capacity");
    }

    #[test]
    fn canonical_frontend_opens_an_independent_owner_session_per_invocation() {
        let root = tempfile::tempdir().unwrap();
        let physical = physical_root(root.path());
        let identity = CoreIdentity::production();
        let config = server_config(root.path().to_path_buf(), identity.clone());
        let server = thread::spawn(move || run_daemon(config));
        let (_directory, _record) = wait_for_record(root.path(), &identity);
        let client = DaemonClient::new(DaemonClientConfig::existing_only(physical, identity));
        let mut anchor = match client.connect_existing().unwrap() {
            ExistingDaemon::Connected(owner) => owner,
            ExistingDaemon::Absent => panic!("published daemon must connect"),
        };

        let mut invocation_session = anchor.connect_peer(Duration::from_secs(1)).unwrap();
        assert_eq!(anchor.daemon_pid(), invocation_session.daemon_pid());
        anchor.ping().unwrap();
        invocation_session.ping().unwrap();

        drop(invocation_session);
        drop(anchor);
        server.join().unwrap().unwrap();
    }

    #[test]
    fn invalid_canonical_arguments_are_direct_before_workspace_or_domain_execution() {
        let root = tempfile::tempdir().unwrap();
        let physical = physical_root(root.path());
        let identity = CoreIdentity::production();
        let config = server_config(root.path().to_path_buf(), identity.clone());
        let server = thread::spawn(move || run_daemon(config));
        let (_directory, _record) = wait_for_record(root.path(), &identity);
        let client = DaemonClient::new(DaemonClientConfig::existing_only(physical, identity));
        let mut owner = match client.connect_existing().unwrap() {
            ExistingDaemon::Connected(owner) => owner,
            ExistingDaemon::Absent => panic!("published daemon must connect"),
        };

        let outcome = owner
            .submit_invocation(
                InvocationRequest::new(
                    ToolIdentity::View,
                    serde_json::json!({"at": ""}),
                    "/workspace/does-not-exist",
                    0,
                )
                .unwrap(),
            )
            .unwrap();
        let result = match outcome {
            InvocationResponse::Direct(result) => result,
            other => panic!("invalid arguments must not materialize a task: {other:?}"),
        };
        assert!(!result.ok);
        assert!(result.summary.contains("address argument `at` is invalid"));

        drop(owner);
        server.join().unwrap().unwrap();
    }

    #[test]
    fn durable_handoff_persists_only_closed_hashes_not_arguments_paths_or_failure_text() {
        let root = tempfile::tempdir().unwrap();
        let physical = physical_root(root.path());
        let workspace_hint = physical.to_string_lossy().into_owned();
        let identity = CoreIdentity::production();
        let executions = Arc::new(AtomicUsize::new(0));
        let (entered, entered_wait) = mpsc::channel();
        let service = Arc::new(BlockingCanonicalService {
            executions,
            entered,
        });
        let config = server_config(root.path().to_path_buf(), identity.clone())
            .with_invocation_service(service);
        let server = thread::spawn(move || run_daemon(config));
        let (_directory, _record) = wait_for_record(root.path(), &identity);
        let client = DaemonClient::new(DaemonClientConfig::existing_only(
            physical.clone(),
            identity.clone(),
        ));
        let mut owner = match client.connect_existing().unwrap() {
            ExistingDaemon::Connected(owner) => owner,
            ExistingDaemon::Absent => panic!("published daemon must connect"),
        };
        let secret = "task7-secret-never-persist";
        let raw_path = "/private/caller/path/never-persist";
        let outcome = owner
            .submit_invocation(
                InvocationRequest::new(
                    ToolIdentity::Run,
                    serde_json::json!({"args": {"secret": secret, "path": raw_path}}),
                    workspace_hint.clone(),
                    7_000,
                )
                .unwrap(),
            )
            .unwrap();
        let task_id = match outcome {
            InvocationResponse::Task(snapshot) => snapshot.task_id,
            other => panic!("known-long invocation must be durable: {other:?}"),
        };
        entered_wait.recv().unwrap();

        let record_path = DaemonStateDirectory::path_for(&physical, &identity)
            .join("tasks")
            .join(format!("{task_id}.json"));
        let bytes = std::fs::read(&record_path).unwrap();
        let wire = String::from_utf8(bytes.clone()).unwrap();
        for forbidden in [
            secret,
            raw_path,
            workspace_hint.as_str(),
            "test cancellation",
        ] {
            assert!(
                !wire.contains(forbidden),
                "persisted forbidden text: {forbidden}"
            );
        }
        let record: crate::application::invocation_store::StoredInvocationRecord =
            serde_json::from_slice(&bytes).unwrap();
        assert_eq!(record.tool, ToolIdentity::Run);
        assert_eq!(record.status, InvocationStatus::Working);

        owner.cancel_task(task_id).unwrap();
        drop(owner);
        server.join().unwrap().unwrap();
    }

    #[test]
    fn canonical_submit_disconnect_is_a_transport_error_without_frontend_fallback() {
        let root = tempfile::tempdir().unwrap();
        let physical = physical_root(root.path());
        let workspace_hint = physical.to_string_lossy().into_owned();
        let identity = CoreIdentity::production();
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let record = EndpointRecord::new(identity.clone(), listener.local_addr().unwrap().port());
        let directory = DaemonStateDirectory::open(&physical, &identity).unwrap();
        directory.write_endpoint_record_for_test(&record).unwrap();
        let peer_record = record.clone();
        let fake_peer = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let _hello = read_bounded_json_line(&mut reader).unwrap();
            write_json_line(&mut stream, &ServerResponse::ready(&peer_record));
            let submit = read_bounded_json_line(&mut reader).unwrap();
            assert!(matches!(
                super::protocol::parse_request(&submit).unwrap(),
                ClientRequest::SubmitInvocation { .. }
            ));
        });
        let client = DaemonClient::new(DaemonClientConfig::existing_only(physical, identity));
        let mut owner = match client.connect_existing().unwrap() {
            ExistingDaemon::Connected(owner) => owner,
            ExistingDaemon::Absent => panic!("fake endpoint must connect"),
        };
        let error = owner
            .submit_invocation(
                InvocationRequest::new(
                    ToolIdentity::Check,
                    serde_json::json!({}),
                    workspace_hint,
                    0,
                )
                .unwrap(),
            )
            .unwrap_err();
        fake_peer.join().unwrap();
        assert!(error.starts_with("read daemon response:"), "{error}");
    }

    #[test]
    fn wrong_token_and_protocol_version_are_rejected_without_echoing_token() {
        let root = tempfile::tempdir().unwrap();
        let identity = CoreIdentity::production();
        let config = server_config(root.path().to_path_buf(), identity.clone());
        let server = thread::spawn(move || run_daemon(config));
        let (_directory, record) = wait_for_record(root.path(), &identity);

        let mut stream = TcpStream::connect(record.loopback_addr().unwrap()).unwrap();
        let bad_token = "ffffffff-ffff-4fff-8fff-ffffffffffff";
        let request = ClientRequest::hello(
            DAEMON_PROTOCOL_VERSION,
            bad_token.to_string(),
            identity.clone(),
        );
        write_json_line(&mut stream, &request);
        let response: ServerResponse =
            serde_json::from_slice(&read_bounded_json_line(&mut BufReader::new(&stream)).unwrap())
                .unwrap();
        assert_eq!(response.error_code(), Some(DaemonErrorCode::Unauthorized));
        assert!(!serde_json::to_string(&response)
            .unwrap()
            .contains(bad_token));

        let mut stream = TcpStream::connect(record.loopback_addr().unwrap()).unwrap();
        let request = ClientRequest::hello(
            DAEMON_PROTOCOL_VERSION + 1,
            record.token().to_string(),
            identity.clone(),
        );
        write_json_line(&mut stream, &request);
        let response: ServerResponse =
            serde_json::from_slice(&read_bounded_json_line(&mut BufReader::new(&stream)).unwrap())
                .unwrap();
        assert_eq!(
            response.error_code(),
            Some(DaemonErrorCode::ProtocolMismatch)
        );

        server.join().unwrap().unwrap();
    }

    #[test]
    fn fake_peer_error_code_is_closed_and_never_reaches_client_diagnostic() {
        let root = tempfile::tempdir().unwrap();
        let physical = physical_root(root.path());
        let identity = CoreIdentity::production();
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let record = EndpointRecord::new(identity.clone(), listener.local_addr().unwrap().port());
        let directory = DaemonStateDirectory::open(&physical, &identity).unwrap();
        directory.write_endpoint_record_for_test(&record).unwrap();
        let fake_peer = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let _hello = read_bounded_json_line(&mut BufReader::new(&stream)).unwrap();
            write_json_line(
                &mut stream,
                &serde_json::json!({
                    "kind": "error",
                    "code": "credential\n\u{001b}[31msecret-looking-value"
                }),
            );
        });

        let client = DaemonClient::new(DaemonClientConfig::existing_only(physical, identity));
        let error = client.connect_existing().unwrap_err();
        fake_peer.join().unwrap();

        assert_eq!(error, "daemon response is not strict versioned JSON");
        assert!(!error.contains("credential"));
        assert!(!error.contains("secret-looking-value"));
        assert!(parse_response(br#"{"kind":"error","code":"future_code"}"#).is_err());
    }

    #[test]
    fn fake_peer_ready_at_deadline_cannot_restart_handshake_budget() {
        let root = tempfile::tempdir().unwrap();
        let physical = physical_root(root.path());
        let identity = CoreIdentity::production();
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let record = EndpointRecord::new(identity.clone(), listener.local_addr().unwrap().port());
        let directory = DaemonStateDirectory::open(&physical, &identity).unwrap();
        directory.write_endpoint_record_for_test(&record).unwrap();
        let clock = ManualDaemonClientClock::new();
        let peer_clock = clock.clone();
        let peer_record = record.clone();
        let fake_peer = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let _hello = read_bounded_json_line(&mut BufReader::new(&stream)).unwrap();
            peer_clock.advance(Duration::from_secs(5));
            write_json_line(&mut stream, &ServerResponse::ready(&peer_record));
        });
        let config = DaemonClientConfig::existing_only(physical, identity)
            .with_clock_for_test(clock)
            .with_connect_timeout_for_test(Duration::from_secs(5));
        let client = DaemonClient::new(config);

        let error = client.connect_existing().unwrap_err();
        fake_peer.join().unwrap();

        assert_eq!(error, "daemon deadline expired during handshake response");
    }

    #[test]
    fn late_malformed_peer_response_cannot_override_deadline() {
        let root = tempfile::tempdir().unwrap();
        let physical = physical_root(root.path());
        let identity = CoreIdentity::production();
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let record = EndpointRecord::new(identity.clone(), listener.local_addr().unwrap().port());
        let directory = DaemonStateDirectory::open(&physical, &identity).unwrap();
        directory.write_endpoint_record_for_test(&record).unwrap();
        let clock = ManualDaemonClientClock::new();
        let peer_clock = clock.clone();
        let fake_peer = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let _hello = read_bounded_json_line(&mut BufReader::new(&stream)).unwrap();
            peer_clock.advance(Duration::from_secs(5));
            write_json_line(
                &mut stream,
                &serde_json::json!({"kind": "error", "code": "future_code"}),
            );
        });
        let config = DaemonClientConfig::existing_only(physical, identity)
            .with_clock_for_test(clock)
            .with_connect_timeout_for_test(Duration::from_secs(5));
        let client = DaemonClient::new(config);

        let error = client.connect_existing().unwrap_err();
        fake_peer.join().unwrap();

        assert_eq!(error, "daemon deadline expired during handshake response");
    }

    #[test]
    fn late_peer_disconnect_cannot_override_deadline() {
        let root = tempfile::tempdir().unwrap();
        let physical = physical_root(root.path());
        let identity = CoreIdentity::production();
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let record = EndpointRecord::new(identity.clone(), listener.local_addr().unwrap().port());
        let directory = DaemonStateDirectory::open(&physical, &identity).unwrap();
        directory.write_endpoint_record_for_test(&record).unwrap();
        let clock = ManualDaemonClientClock::new();
        let peer_clock = clock.clone();
        let fake_peer = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let _hello = read_bounded_json_line(&mut BufReader::new(&stream)).unwrap();
            peer_clock.advance(Duration::from_secs(5));
        });
        let config = DaemonClientConfig::existing_only(physical, identity)
            .with_clock_for_test(clock)
            .with_connect_timeout_for_test(Duration::from_secs(5));
        let client = DaemonClient::new(config);

        let error = client.connect_existing().unwrap_err();
        fake_peer.join().unwrap();

        assert_eq!(error, "daemon deadline expired during handshake response");
    }

    #[test]
    fn ping_uses_one_aggregate_deadline_for_write_and_response() {
        let root = tempfile::tempdir().unwrap();
        let physical = physical_root(root.path());
        let identity = CoreIdentity::production();
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let record = EndpointRecord::new(identity.clone(), listener.local_addr().unwrap().port());
        let directory = DaemonStateDirectory::open(&physical, &identity).unwrap();
        directory.write_endpoint_record_for_test(&record).unwrap();
        let clock = ManualDaemonClientClock::new();
        let peer_clock = clock.clone();
        let peer_record = record.clone();
        let fake_peer = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let _hello = read_bounded_json_line(&mut reader).unwrap();
            write_json_line(&mut stream, &ServerResponse::ready(&peer_record));
            let ping = read_bounded_json_line(&mut reader).unwrap();
            assert_eq!(
                serde_json::from_slice::<ClientRequest>(&ping).unwrap(),
                ClientRequest::Ping {}
            );
            peer_clock.advance(Duration::from_secs(5));
            write_json_line(&mut stream, &ServerResponse::Pong);
        });
        let config = DaemonClientConfig::existing_only(physical, identity)
            .with_clock_for_test(clock)
            .with_connect_timeout_for_test(Duration::from_secs(5));
        let client = DaemonClient::new(config);
        let mut owner = match client.connect_existing().unwrap() {
            ExistingDaemon::Connected(owner) => owner,
            ExistingDaemon::Absent => panic!("fake endpoint must connect"),
        };

        let error = owner.ping().unwrap_err();
        fake_peer.join().unwrap();

        assert_eq!(error, "daemon deadline expired during ping response");
    }

    #[test]
    fn owner_drop_closes_connection_without_waiting_for_release_ack() {
        let root = tempfile::tempdir().unwrap();
        let physical = physical_root(root.path());
        let identity = CoreIdentity::production();
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let record = EndpointRecord::new(identity.clone(), listener.local_addr().unwrap().port());
        let directory = DaemonStateDirectory::open(&physical, &identity).unwrap();
        directory.write_endpoint_record_for_test(&record).unwrap();
        let peer_record = record.clone();
        let (observed_tx, observed_rx) = mpsc::channel();
        let fake_peer = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let _hello = read_bounded_json_line(&mut reader).unwrap();
            write_json_line(&mut stream, &ServerResponse::ready(&peer_record));
            let observed_eof = read_bounded_json_line(&mut reader).unwrap_err().kind()
                == std::io::ErrorKind::UnexpectedEof;
            observed_tx.send(observed_eof).unwrap();
        });
        let client = DaemonClient::new(DaemonClientConfig::existing_only(physical, identity));
        let owner = match client.connect_existing().unwrap() {
            ExistingDaemon::Connected(owner) => owner,
            ExistingDaemon::Absent => panic!("fake endpoint must connect"),
        };

        drop(owner);
        assert!(
            observed_rx.recv().unwrap(),
            "owner drop sent a release request"
        );
        fake_peer.join().unwrap();
    }

    #[test]
    fn exited_startup_child_is_reported_before_readiness_deadline() {
        let root = tempfile::tempdir().unwrap();
        let physical = physical_root(root.path());
        let identity = CoreIdentity::production();
        let config = DaemonClientConfig::new(
            physical,
            identity,
            std::env::current_exe().unwrap(),
            Duration::from_millis(350),
        )
        .with_connect_timeout_for_test(Duration::from_millis(500));
        let client = DaemonClient::new(config);

        let error = match client.connect_or_spawn() {
            Ok(_) => panic!("exited fixture unexpectedly became a daemon owner"),
            Err(error) => error,
        };

        assert!(error.contains("exited before readiness with"), "{error}");
    }

    #[test]
    fn owner_lease_keeps_daemon_alive_then_idle_removes_only_its_record() {
        let root = tempfile::tempdir().unwrap();
        let physical = physical_root(root.path());
        let identity = CoreIdentity::production();
        let config = server_config(root.path().to_path_buf(), identity.clone());
        let server = thread::spawn(move || run_daemon(config));
        let (directory, record) = wait_for_record(root.path(), &identity);
        let client = DaemonClient::new(DaemonClientConfig::existing_only(
            physical,
            identity.clone(),
        ));
        let mut owner = match client.connect_existing().unwrap() {
            ExistingDaemon::Connected(owner) => owner,
            ExistingDaemon::Absent => panic!("published daemon must connect"),
        };
        owner.ping().unwrap();
        thread::sleep(Duration::from_millis(600));
        owner.ping().unwrap();

        let replacement = EndpointRecord::test_replacement(&record);
        directory
            .write_endpoint_record_for_test(&replacement)
            .unwrap();
        drop(owner);
        server.join().unwrap().unwrap();
        assert_eq!(directory.read_endpoint_record().unwrap(), Some(replacement));
    }

    #[test]
    fn authenticated_owners_release_handshake_capacity() {
        let root = tempfile::tempdir().unwrap();
        let identity = CoreIdentity::production();
        let config = server_config(root.path().to_path_buf(), identity.clone());
        let server = thread::spawn(move || run_daemon(config));
        let (_directory, record) = wait_for_record(root.path(), &identity);
        let mut owners = Vec::new();

        for owner_index in 0..=MAX_HANDSHAKES {
            let (stream, response) = connect_raw_owner(&record, &identity);
            assert!(
                response.matches_record(&record),
                "owner {owner_index} was rejected after authentication: {response:?}"
            );
            owners.push(stream);
        }

        drop(owners);
        server.join().unwrap().unwrap();
    }

    #[test]
    fn owner_session_capacity_is_distinct_and_retryable() {
        const EXPECTED_OWNER_SESSION_LIMIT: usize = 64;
        assert_eq!(MAX_OWNER_SESSIONS, EXPECTED_OWNER_SESSION_LIMIT);

        let root = tempfile::tempdir().unwrap();
        let physical = physical_root(root.path());
        let identity = CoreIdentity::production();
        let config = server_config(root.path().to_path_buf(), identity.clone());
        let server = thread::spawn(move || run_daemon(config));
        let (_directory, record) = wait_for_record(root.path(), &identity);
        let mut owners = Vec::new();

        for owner_index in 0..EXPECTED_OWNER_SESSION_LIMIT {
            let (stream, response) = connect_raw_owner(&record, &identity);
            assert!(
                response.matches_record(&record),
                "owner {owner_index} was rejected below the owner-session bound: {response:?}"
            );
            owners.push(stream);
        }

        let client = DaemonClient::new(DaemonClientConfig::existing_only(physical, identity));
        let error = client.connect_existing().unwrap_err();
        assert_eq!(error, "daemon owner capacity reached; retry later");

        drop(owners.pop());
        let retry_deadline = Instant::now() + Duration::from_secs(2);
        let mut recovered = loop {
            match client.connect_existing() {
                Ok(ExistingDaemon::Connected(owner)) => break owner,
                Ok(ExistingDaemon::Absent) => panic!("published daemon disappeared during retry"),
                Err(error) if error == "daemon owner capacity reached; retry later" => {
                    assert!(
                        Instant::now() < retry_deadline,
                        "owner capacity did not recover after a live session closed"
                    );
                    thread::sleep(Duration::from_millis(10));
                }
                Err(error) => panic!("owner-capacity retry failed unexpectedly: {error}"),
            }
        };
        recovered.ping().unwrap();
        drop(recovered);

        drop(owners);
        server.join().unwrap().unwrap();
    }

    #[test]
    fn stale_record_is_never_signalled_and_foreign_identity_fails_closed() {
        let root = tempfile::tempdir().unwrap();
        let physical = physical_root(root.path());
        let identity = CoreIdentity::production();
        let directory = DaemonStateDirectory::open(&physical, &identity).unwrap();
        let stale = EndpointRecord::test_stale(identity.clone(), 4_294_967_000);
        directory.write_endpoint_record_for_test(&stale).unwrap();

        let client = DaemonClient::new(DaemonClientConfig::existing_only(
            physical.clone(),
            identity.clone(),
        ));
        assert!(matches!(
            client.connect_existing().unwrap(),
            ExistingDaemon::Absent
        ));
        assert_eq!(directory.read_endpoint_record().unwrap(), Some(stale));

        let foreign = EndpointRecord::test_stale(alternate_identity(), 42);
        directory.write_endpoint_record_for_test(&foreign).unwrap();
        let error = client.connect_existing().unwrap_err();
        assert!(error.contains("foreign core identity"), "{error}");
        assert_eq!(directory.read_endpoint_record().unwrap(), Some(foreign));
    }

    #[test]
    fn incompatible_core_identities_use_separate_endpoint_directories() {
        let root = tempfile::tempdir().unwrap();
        let physical = physical_root(root.path());
        let production = CoreIdentity::production();
        let alternate = alternate_identity();
        assert_ne!(
            DaemonStateDirectory::path_for(&physical, &production),
            DaemonStateDirectory::path_for(&physical, &alternate)
        );
        let first = DaemonStateDirectory::open(&physical, &production).unwrap();
        let second = DaemonStateDirectory::open(&physical, &alternate).unwrap();
        assert_ne!(first.path(), second.path());
    }

    #[test]
    fn admitted_handshake_blocks_idle_exit_until_lease_is_registered() {
        let pause = install_handshake_pause();
        let root = tempfile::tempdir().unwrap();
        let identity = CoreIdentity::production();
        let config = DaemonServerConfig::new(
            physical_root(root.path()),
            identity.clone(),
            Duration::from_millis(80),
        )
        .with_handshake_pause(&pause);
        let server = thread::spawn(move || run_daemon(config));
        let (directory, record) = wait_for_record(root.path(), &identity);
        let mut stream = TcpStream::connect(record.loopback_addr().unwrap()).unwrap();
        let request = ClientRequest::hello(
            DAEMON_PROTOCOL_VERSION,
            record.token().to_string(),
            identity,
        );
        write_json_line(&mut stream, &request);
        pause.wait_until_entered();

        thread::sleep(Duration::from_millis(160));
        assert!(
            !server.is_finished(),
            "admitted handshake lost the daemon to idle exit"
        );
        assert_eq!(
            directory.read_endpoint_record().unwrap(),
            Some(record.clone())
        );

        pause.release();
        let mut reader = BufReader::new(&stream);
        let ready: ServerResponse =
            serde_json::from_slice(&read_bounded_json_line(&mut reader).unwrap()).unwrap();
        assert!(ready.matches_record(&record));
        drop(reader);
        write_json_line(&mut stream, &ClientRequest::Release {});
        server.join().unwrap().unwrap();
    }

    #[test]
    fn unauthenticated_connection_admission_is_bounded() {
        let pause = install_handshake_pause();
        let root = tempfile::tempdir().unwrap();
        let identity = CoreIdentity::production();
        let config = DaemonServerConfig::new(
            physical_root(root.path()),
            identity.clone(),
            Duration::from_millis(120),
        )
        .with_handshake_pause(&pause);
        let server = thread::spawn(move || run_daemon(config));
        let (_directory, record) = wait_for_record(root.path(), &identity);
        let mut admitted = Vec::new();
        for _ in 0..MAX_HANDSHAKES {
            admitted.push(TcpStream::connect(record.loopback_addr().unwrap()).unwrap());
            thread::sleep(Duration::from_millis(30));
        }
        pause.wait_until_entered();
        let extra = TcpStream::connect(record.loopback_addr().unwrap()).unwrap();
        extra
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let response: ServerResponse =
            serde_json::from_slice(&read_bounded_json_line(&mut BufReader::new(&extra)).unwrap())
                .unwrap();
        assert_eq!(response.error_code(), Some(DaemonErrorCode::Overloaded));

        drop(extra);
        drop(admitted);
        pause.release();
        server.join().unwrap().unwrap();
    }

    #[test]
    fn duplicate_owner_lease_and_oversized_wire_request_fail_closed() {
        let root = tempfile::tempdir().unwrap();
        let identity = CoreIdentity::production();
        let config = server_config(root.path().to_path_buf(), identity.clone());
        let server = thread::spawn(move || run_daemon(config));
        let (_directory, record) = wait_for_record(root.path(), &identity);
        let lease = "dddddddd-dddd-4ddd-8ddd-dddddddddddd".to_string();
        let hello = ClientRequest::hello_with_owner_for_test(
            record.token().to_string(),
            identity.clone(),
            lease.clone(),
        );
        let mut first = TcpStream::connect(record.loopback_addr().unwrap()).unwrap();
        write_json_line(&mut first, &hello);
        let ready: ServerResponse =
            serde_json::from_slice(&read_bounded_json_line(&mut BufReader::new(&first)).unwrap())
                .unwrap();
        assert!(ready.matches_record(&record));

        let duplicate =
            ClientRequest::hello_with_owner_for_test(record.token().to_string(), identity, lease);
        let mut second = TcpStream::connect(record.loopback_addr().unwrap()).unwrap();
        write_json_line(&mut second, &duplicate);
        let response: ServerResponse =
            serde_json::from_slice(&read_bounded_json_line(&mut BufReader::new(&second)).unwrap())
                .unwrap();
        assert_eq!(response.error_code(), Some(DaemonErrorCode::DuplicateLease));

        write_json_line(&mut first, &ClientRequest::Release {});
        let released: ServerResponse =
            serde_json::from_slice(&read_bounded_json_line(&mut BufReader::new(&first)).unwrap())
                .unwrap();
        assert_eq!(released, ServerResponse::Released);
        let release_deadline = Instant::now() + Duration::from_secs(2);
        let mut reused = loop {
            let mut candidate = TcpStream::connect(record.loopback_addr().unwrap()).unwrap();
            write_json_line(&mut candidate, &hello);
            let response: ServerResponse = serde_json::from_slice(
                &read_bounded_json_line(&mut BufReader::new(&candidate)).unwrap(),
            )
            .unwrap();
            if response.matches_record(&record) {
                break candidate;
            }
            assert_eq!(response.error_code(), Some(DaemonErrorCode::DuplicateLease));
            assert!(
                Instant::now() < release_deadline,
                "released owner lease remained registered"
            );
            thread::sleep(Duration::from_millis(5));
        };

        let mut oversized = TcpStream::connect(record.loopback_addr().unwrap()).unwrap();
        oversized
            .write_all(&vec![b'x'; MAX_JSON_LINE_BYTES + 1])
            .unwrap();
        oversized.write_all(b"\n").unwrap();
        let response: ServerResponse = serde_json::from_slice(
            &read_bounded_json_line(&mut BufReader::new(&oversized)).unwrap(),
        )
        .unwrap();
        assert_eq!(response.error_code(), Some(DaemonErrorCode::InvalidRequest));

        write_json_line(&mut reused, &ClientRequest::Release {});
        server.join().unwrap().unwrap();
    }

    #[test]
    fn daemon_is_the_sole_invocation_store_writer_for_its_lifetime() {
        let root = tempfile::tempdir().unwrap();
        let identity = CoreIdentity::production();
        let config = server_config(root.path().to_path_buf(), identity.clone());
        let server = thread::spawn(move || run_daemon(config));
        let (directory, record) = wait_for_record(root.path(), &identity);
        let task_root = directory.path().join("tasks");

        assert!(matches!(
            FileInvocationStore::open(&task_root, Arc::new(SystemEpochMillisClock)),
            Err(InvocationStoreError::AlreadyOwned)
        ));
        let competing = run_daemon(server_config(root.path().to_path_buf(), identity));
        assert!(competing
            .unwrap_err()
            .contains("task store already has an active owner"));
        assert_eq!(directory.read_endpoint_record().unwrap(), Some(record));
        server.join().unwrap().unwrap();
        let reopened = FileInvocationStore::open(&task_root, Arc::new(SystemEpochMillisClock));
        assert!(reopened.is_ok());
    }
}
