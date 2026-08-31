pub(crate) mod client;
pub(crate) mod client_v5;
pub(crate) mod identity;
pub(crate) mod protocol;
#[allow(dead_code)]
pub(crate) mod protocol_v5;
pub(crate) mod runtime_v5;
pub(crate) mod server;
pub(crate) mod terminal_codec_v5;
mod v13_read_modes;
#[allow(dead_code)]
mod v13_service;
mod v13_syntax_run;

#[cfg(test)]
mod tests {
    use super::client::{
        DaemonClient, DaemonClientConfig, DaemonTaskExchangeError, ExistingDaemon,
        ManualDaemonClientClock,
    };
    use super::identity::{CoreIdentity, DaemonStateDirectory};
    use super::protocol::{
        parse_response, read_bounded_json_line, ClientRequest, DaemonErrorCode, EndpointRecord,
        InvocationRequest, InvocationResponse, ServerResponse, DAEMON_PROTOCOL_VERSION,
        MAX_DAEMON_REQUEST_LINE_BYTES, MAX_DAEMON_RESPONSE_LINE_BYTES,
    };
    use super::server::{
        install_handshake_pause, install_startup_pause, run_daemon,
        workspace_capacity_protocol_code_for_test, write_bytes_before, DaemonServerConfig,
        MAX_HANDSHAKES, MAX_OWNER_SESSIONS,
    };
    use super::server::{ActorBoundExecution, ActorBoundInvocation, CanonicalInvocationService};
    use super::v13_service::CanonicalV13ReadService;
    use crate::application::invocation::RESPONSE_SERIALIZATION_MARGIN_MS;
    use crate::application::invocation_store::{
        InvocationStore, InvocationStoreError, NewInvocationRecord, SafeFailureReason,
        SafeStatusMessage, StoredInvocationRecord, TaskTransition, ToolIdentity,
        MAX_CANONICAL_RESULT_BYTES,
    };
    use crate::application::operation_descriptors::{ExecutionClass, KnownLongReason};
    use crate::application::ports::Clock;
    use crate::domain::cancellation::CancellationToken;
    use crate::domain::invocation::{DomainResult, InvocationFailure, InvocationStatus, TaskId};
    use crate::infrastructure::platform::testing::{
        create_directory_link_fixture_for_test, set_unix_mode_for_test, unix_mode_for_test,
        FileLinkFixtureOutcome,
    };
    use crate::infrastructure::task_store::{FileInvocationStore, SystemEpochMillisClock};
    use crate::test_support::tree_snapshot;
    use std::collections::HashMap;
    use std::io::{self, BufRead, BufReader, Cursor, Read, Write};
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
        create_delay: Duration,
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
            thread::sleep(self.create_delay);
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
        ensure_platform_xml_workspace(&root);
        DaemonServerConfig::new(physical_root(&root), identity, Duration::from_millis(350))
    }

    fn ensure_platform_xml_workspace(root: &std::path::Path) {
        let project = root.join("v8project.yaml");
        if project.exists() {
            return;
        }
        std::fs::write(
            project,
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: .\n",
        )
        .unwrap();
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
                        // Keep this lifecycle fixture independent from filesystem speed by
                        // proving that task creation may exceed the zero-budget response window.
                        create_delay: Duration::from_millis(RESPONSE_SERIALIZATION_MARGIN_MS + 50),
                    }))
                    .with_invocation_service(Arc::new(ProcessCountingService { executions }));
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

    fn stream_closed_without_response(stream: &mut TcpStream) -> bool {
        if let Err(error) = stream.set_read_timeout(Some(Duration::from_secs(1))) {
            assert_eq!(
                error.kind(),
                io::ErrorKind::InvalidInput,
                "configure handshake close observation: {error}"
            );
        }
        let mut byte = [0_u8; 1];
        match stream.read(&mut byte) {
            Ok(0) => true,
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::ConnectionAborted
                        | io::ErrorKind::ConnectionReset
                        | io::ErrorKind::BrokenPipe
                        | io::ErrorKind::NotConnected
                        | io::ErrorKind::UnexpectedEof
                ) =>
            {
                true
            }
            Ok(_) => false,
            Err(error) => panic!("handshake connection did not close silently: {error}"),
        }
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

    struct SizedResultService {
        summary_bytes: usize,
        class: ExecutionClass,
    }

    struct ManualInvocationClock(Mutex<Instant>);

    impl ManualInvocationClock {
        fn new(now: Instant) -> Self {
            Self(Mutex::new(now))
        }

        fn advance(&self, duration: Duration) {
            *self.0.lock().unwrap() += duration;
        }
    }

    impl Clock for ManualInvocationClock {
        fn now(&self) -> Instant {
            *self.0.lock().unwrap()
        }
    }

    struct DelayedPrepareService {
        clock: Arc<ManualInvocationClock>,
        delay: Duration,
        executions: Arc<AtomicUsize>,
    }

    #[derive(Default)]
    struct DaemonMemoryStore {
        records: Mutex<HashMap<TaskId, StoredInvocationRecord>>,
        update_attempts: AtomicUsize,
        fail_updates: AtomicUsize,
    }

    struct DelayedTaskReadStore {
        inner: DaemonMemoryStore,
        read_delay: Duration,
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

    impl InvocationStore for DelayedTaskReadStore {
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
            thread::sleep(self.read_delay);
            self.inner.get(task_id)
        }

        fn update(
            &self,
            task_id: TaskId,
            transition: TaskTransition,
        ) -> Result<StoredInvocationRecord, InvocationStoreError> {
            self.inner.update(task_id, transition)
        }

        fn cancel(
            &self,
            task_id: TaskId,
            status_message: SafeStatusMessage,
        ) -> Result<StoredInvocationRecord, InvocationStoreError> {
            self.inner.cancel(task_id, status_message)
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

    impl CanonicalInvocationService for SizedResultService {
        fn prepare(
            &self,
            _invocation: &ActorBoundInvocation,
        ) -> Result<ExecutionClass, Box<DomainResult>> {
            Ok(self.class)
        }

        fn execute(
            &self,
            _invocation: &ActorBoundExecution,
            _cancellation: CancellationToken,
        ) -> Result<DomainResult, InvocationFailure> {
            Ok(DomainResult::success("R".repeat(self.summary_bytes)))
        }
    }

    impl CanonicalInvocationService for DelayedPrepareService {
        fn prepare(
            &self,
            _invocation: &ActorBoundInvocation,
        ) -> Result<ExecutionClass, Box<DomainResult>> {
            self.clock.advance(self.delay);
            Ok(ExecutionClass::InlineCandidate)
        }

        fn execute(
            &self,
            _invocation: &ActorBoundExecution,
            _cancellation: CancellationToken,
        ) -> Result<DomainResult, InvocationFailure> {
            self.executions.fetch_add(1, Ordering::SeqCst);
            Ok(DomainResult::success("daemon receipt deadline result"))
        }
    }

    fn core_identity_is_closed_compile_time_abi_protocol_digest() {
        use sha2::{Digest, Sha256};

        let production = CoreIdentity::production();
        let identity_for = |protocol: &[u8]| {
            let mut digest = Sha256::new();
            digest.update(b"unica-v0.13-core-abi-1");
            digest.update(b"\0");
            digest.update(protocol);
            digest
                .finalize()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        };
        let expected = identity_for(b"unica-daemon-jsonl-3");
        assert_eq!(
            production.as_str(),
            expected,
            "core compatibility identity must include the exact daemon protocol v3 identity"
        );
        assert_ne!(
            production.as_str(),
            identity_for(b"unica-daemon-jsonl-2"),
            "a v3 process must not reuse the v2 compatibility key"
        );
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
        let oversized = vec![b'x'; MAX_DAEMON_REQUEST_LINE_BYTES + 1];
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
    fn protocol_reader_retries_an_interrupted_fill_without_losing_the_line() {
        struct InterruptedOnce<R> {
            inner: R,
            interrupted: bool,
        }

        impl<R: Read> Read for InterruptedOnce<R> {
            fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
                self.inner.read(buffer)
            }
        }

        impl<R: BufRead> BufRead for InterruptedOnce<R> {
            fn fill_buf(&mut self) -> io::Result<&[u8]> {
                if !self.interrupted {
                    self.interrupted = true;
                    return Err(io::ErrorKind::Interrupted.into());
                }
                self.inner.fill_buf()
            }

            fn consume(&mut self, amount: usize) {
                self.inner.consume(amount);
            }
        }

        let mut reader = InterruptedOnce {
            inner: Cursor::new(b"{\"kind\":\"ping\"}\n"),
            interrupted: false,
        };

        assert_eq!(
            read_bounded_json_line(&mut reader).unwrap(),
            b"{\"kind\":\"ping\"}"
        );
    }

    fn round_trip_sized_result(summary_bytes: usize) {
        let root = tempfile::tempdir().unwrap();
        let physical = physical_root(root.path());
        let identity = CoreIdentity::production();
        let config = server_config(root.path().to_path_buf(), identity.clone())
            .with_invocation_service(Arc::new(SizedResultService {
                summary_bytes,
                class: ExecutionClass::InlineCandidate,
            }));
        let server = thread::spawn(move || run_daemon(config));
        let (_directory, _record) = wait_for_record(root.path(), &identity);
        let client = DaemonClient::new(DaemonClientConfig::existing_only(
            physical.clone(),
            identity,
        ));
        let mut owner = match client.connect_existing().unwrap() {
            ExistingDaemon::Connected(owner) => owner,
            ExistingDaemon::Absent => panic!("published daemon must connect"),
        };

        let direct = owner
            .submit_invocation(
                InvocationRequest::new(
                    ToolIdentity::Run,
                    serde_json::json!({}),
                    physical.to_string_lossy(),
                    7_000,
                )
                .unwrap(),
            )
            .unwrap();
        assert_eq!(
            match direct {
                InvocationResponse::Direct(result) => result.summary.len(),
                other => panic!("sized inline result was not direct: {other:?}"),
            },
            summary_bytes
        );

        drop(owner);
        server.join().unwrap().unwrap();

        let task_root = tempfile::tempdir().unwrap();
        let task_physical = physical_root(task_root.path());
        let task_identity = CoreIdentity::production();
        let task_config = server_config(task_root.path().to_path_buf(), task_identity.clone())
            .with_invocation_service(Arc::new(SizedResultService {
                summary_bytes,
                class: ExecutionClass::KnownLong(KnownLongReason::ExternalProcess),
            }));
        let task_server = thread::spawn(move || run_daemon(task_config));
        let (_directory, _record) = wait_for_record(task_root.path(), &task_identity);
        let task_client = DaemonClient::new(DaemonClientConfig::existing_only(
            task_physical.clone(),
            task_identity,
        ));
        let mut task_owner = match task_client.connect_existing().unwrap() {
            ExistingDaemon::Connected(owner) => owner,
            ExistingDaemon::Absent => panic!("published daemon must connect"),
        };

        let task_id = match task_owner
            .submit_invocation(
                InvocationRequest::new(
                    ToolIdentity::Run,
                    serde_json::json!({}),
                    task_physical.to_string_lossy(),
                    7_000,
                )
                .unwrap(),
            )
            .unwrap()
        {
            InvocationResponse::Task(snapshot) => snapshot.task_id,
            other => panic!("known-long sized result was not a task: {other:?}"),
        };
        let terminal = task_owner
            .wait_task(task_id, INTEGRATION_TASK_WAIT_MS)
            .unwrap();
        assert_eq!(terminal.status, InvocationStatus::Completed);
        assert_eq!(terminal.result.unwrap().summary.len(), summary_bytes);

        drop(task_owner);
        task_server.join().unwrap().unwrap();
    }

    fn response_limit_round_trips_results_above_request_cap_and_near_canonical_cap() {
        round_trip_sized_result(32 * 1024);
        round_trip_sized_result(MAX_CANONICAL_RESULT_BYTES - 4_096);
    }

    fn result_over_canonical_cap_fails_closed_for_direct_and_task() {
        let root = tempfile::tempdir().unwrap();
        let physical = physical_root(root.path());
        let identity = CoreIdentity::production();
        let config = server_config(root.path().to_path_buf(), identity.clone())
            .with_invocation_service(Arc::new(SizedResultService {
                summary_bytes: MAX_CANONICAL_RESULT_BYTES + 1,
                class: ExecutionClass::InlineCandidate,
            }));
        let server = thread::spawn(move || run_daemon(config));
        let (_directory, _record) = wait_for_record(root.path(), &identity);
        let client = DaemonClient::new(DaemonClientConfig::existing_only(
            physical.clone(),
            identity,
        ));
        let mut owner = match client.connect_existing().unwrap() {
            ExistingDaemon::Connected(owner) => owner,
            ExistingDaemon::Absent => panic!("published daemon must connect"),
        };

        let direct_error = owner
            .submit_invocation(
                InvocationRequest::new(
                    ToolIdentity::Run,
                    serde_json::json!({}),
                    physical.to_string_lossy(),
                    7_000,
                )
                .unwrap(),
            )
            .unwrap_err();
        assert_eq!(
            direct_error,
            "daemon invocation submission rejected: result_too_large"
        );

        drop(owner);
        server.join().unwrap().unwrap();

        let task_root = tempfile::tempdir().unwrap();
        let task_physical = physical_root(task_root.path());
        let task_identity = CoreIdentity::production();
        let task_config = server_config(task_root.path().to_path_buf(), task_identity.clone())
            .with_invocation_service(Arc::new(SizedResultService {
                summary_bytes: MAX_CANONICAL_RESULT_BYTES + 1,
                class: ExecutionClass::KnownLong(KnownLongReason::ExternalProcess),
            }));
        let task_server = thread::spawn(move || run_daemon(task_config));
        let (_directory, _record) = wait_for_record(task_root.path(), &task_identity);
        let task_client = DaemonClient::new(DaemonClientConfig::existing_only(
            task_physical.clone(),
            task_identity,
        ));
        let mut task_owner = match task_client.connect_existing().unwrap() {
            ExistingDaemon::Connected(owner) => owner,
            ExistingDaemon::Absent => panic!("published daemon must connect"),
        };

        let task_id = match task_owner
            .submit_invocation(
                InvocationRequest::new(
                    ToolIdentity::Run,
                    serde_json::json!({}),
                    task_physical.to_string_lossy(),
                    7_000,
                )
                .unwrap(),
            )
            .unwrap()
        {
            InvocationResponse::Task(snapshot) => snapshot.task_id,
            other => panic!("known-long oversized result was not a task: {other:?}"),
        };
        let terminal = task_owner
            .wait_task(task_id, INTEGRATION_TASK_WAIT_MS)
            .unwrap();
        assert_eq!(terminal.status, InvocationStatus::Failed);
        assert_eq!(terminal.failure.unwrap().code, "result_too_large");
        assert!(terminal.result.is_none());

        drop(task_owner);
        task_server.join().unwrap().unwrap();
    }

    fn assert_hostile_response_closes_owner_session(payload: Vec<u8>, expected_error: &str) {
        let root = tempfile::tempdir().unwrap();
        let physical = physical_root(root.path());
        let identity = CoreIdentity::production();
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let record = EndpointRecord::new(identity.clone(), listener.local_addr().unwrap().port());
        let directory = DaemonStateDirectory::open(&physical, &identity).unwrap();
        directory.write_endpoint_record_for_test(&record).unwrap();
        let peer_record = record.clone();
        let (second_request_seen, second_request_seen_wait) = mpsc::channel();
        let fake_peer = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_write_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let _hello = read_bounded_json_line(&mut reader).unwrap();
            write_json_line(&mut stream, &ServerResponse::ready(&peer_record));
            let first = read_bounded_json_line(&mut reader).unwrap();
            assert_eq!(
                serde_json::from_slice::<ClientRequest>(&first).unwrap(),
                ClientRequest::Ping {}
            );
            let _ = stream.write_all(&payload).and_then(|_| stream.flush());
            let _ = stream.shutdown(std::net::Shutdown::Write);
            second_request_seen
                .send(read_bounded_json_line(&mut reader).is_ok())
                .unwrap();
        });
        let client = DaemonClient::new(DaemonClientConfig::existing_only(physical, identity));
        let mut owner = match client.connect_existing().unwrap() {
            ExistingDaemon::Connected(owner) => owner,
            ExistingDaemon::Absent => panic!("fake endpoint must connect"),
        };

        let error = owner.ping().unwrap_err();
        assert!(error.contains(expected_error), "{error}");
        assert!(owner.ping().is_err());
        assert!(!second_request_seen_wait.recv().unwrap());
        fake_peer.join().unwrap();
    }

    fn hostile_oversized_response_closes_owner_session_before_a_second_request() {
        let mut hostile = vec![b'x'; MAX_DAEMON_RESPONSE_LINE_BYTES + 1];
        hostile.push(b'\n');
        assert_hostile_response_closes_owner_session(hostile, "byte limit");
    }

    fn malformed_and_truncated_responses_close_owner_sessions_before_reuse() {
        assert_hostile_response_closes_owner_session(b"{not-json}\n".to_vec(), "strict versioned");
        assert_hostile_response_closes_owner_session(
            br#"{"kind":"pong"}"#.to_vec(),
            "missing its terminator",
        );
    }

    struct DeterministicBackpressure {
        now: Arc<Mutex<Instant>>,
        attempts: Arc<AtomicUsize>,
    }

    impl Write for DeterministicBackpressure {
        fn write(&mut self, _buffer: &[u8]) -> std::io::Result<usize> {
            self.attempts.fetch_add(1, Ordering::SeqCst);
            *self.now.lock().unwrap() += Duration::from_millis(60);
            Err(std::io::Error::new(
                std::io::ErrorKind::WouldBlock,
                "deterministic backpressure",
            ))
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn backpressured_response_uses_the_original_session_margin_without_reset() {
        let started = Instant::now();
        let now = Arc::new(Mutex::new(started));
        let attempts = Arc::new(AtomicUsize::new(0));
        let mut writer = DeterministicBackpressure {
            now: now.clone(),
            attempts: attempts.clone(),
        };
        let observed = now.clone();

        let error = write_bytes_before(
            &mut writer,
            &[b'x'; 32 * 1024],
            started + Duration::from_millis(125),
            move || *observed.lock().unwrap(),
            |_writer, _remaining| Ok(()),
        )
        .unwrap_err();

        assert!(
            error.contains("bounded response deadline elapsed"),
            "{error}"
        );
        assert_eq!(attempts.load(Ordering::SeqCst), 3);
        assert_eq!(*now.lock().unwrap(), started + Duration::from_millis(180));
    }

    fn server_captures_one_invocation_deadline_before_delayed_prepare_and_response_write() {
        for (delay, response_is_deliverable) in [
            (Duration::from_millis(110), true),
            (Duration::from_millis(226), false),
        ] {
            let root = tempfile::tempdir().unwrap();
            let physical = physical_root(root.path());
            let identity = CoreIdentity::production();
            let store = Arc::new(DaemonMemoryStore::default());
            let clock = Arc::new(ManualInvocationClock::new(Instant::now()));
            let executions = Arc::new(AtomicUsize::new(0));
            let config = server_config(physical.clone(), identity.clone())
                .with_invocation_store_for_test(store.clone())
                .with_invocation_clock_for_test(clock.clone())
                .with_invocation_service(Arc::new(DelayedPrepareService {
                    clock,
                    delay,
                    executions: Arc::clone(&executions),
                }));
            let server = thread::spawn(move || run_daemon(config));
            let (_directory, _record) = wait_for_record(root.path(), &identity);
            let client = DaemonClient::new(DaemonClientConfig::existing_only(
                physical.clone(),
                identity,
            ));
            let mut owner = match client.connect_existing().unwrap() {
                ExistingDaemon::Connected(owner) => owner,
                ExistingDaemon::Absent => panic!("published daemon must connect"),
            };
            let submission = owner.submit_invocation(
                InvocationRequest::new(
                    ToolIdentity::Run,
                    serde_json::json!({}),
                    physical.to_string_lossy(),
                    100,
                )
                .unwrap(),
            );

            if response_is_deliverable {
                let task_id = match submission.unwrap() {
                    InvocationResponse::Task(snapshot) => snapshot.task_id,
                    other => panic!("delayed prepare escaped as direct: {other:?}"),
                };
                let terminal = owner.wait_task(task_id, 7_000).unwrap();
                assert_eq!(terminal.status, InvocationStatus::Completed);
                assert_eq!(
                    terminal.result.unwrap().summary,
                    "daemon receipt deadline result"
                );
            } else {
                let error = submission.unwrap_err();
                assert!(error.starts_with("read daemon response:"), "{error}");
                let deadline = Instant::now() + INTEGRATION_COORDINATION_TIMEOUT;
                loop {
                    let records = store.records.lock().unwrap();
                    if let Some(record) = records.values().next() {
                        if record.status == InvocationStatus::Completed {
                            assert_eq!(
                                record.result.as_ref().unwrap().summary,
                                "daemon receipt deadline result"
                            );
                            break;
                        }
                    }
                    drop(records);
                    assert!(
                        Instant::now() < deadline,
                        "durable result was not published"
                    );
                    thread::yield_now();
                }
            }
            assert_eq!(executions.load(Ordering::SeqCst), 1);
            drop(owner);
            server.join().unwrap().unwrap();
        }
    }

    fn daemon_result_size_and_session_bounds_are_enforced() {
        response_limit_round_trips_results_above_request_cap_and_near_canonical_cap();
        result_over_canonical_cap_fails_closed_for_direct_and_task();
        hostile_oversized_response_closes_owner_session_before_a_second_request();
        malformed_and_truncated_responses_close_owner_sessions_before_reuse();
        backpressured_response_uses_the_original_session_margin_without_reset();
        server_captures_one_invocation_deadline_before_delayed_prepare_and_response_write();
    }

    #[test]
    fn daemon_invocation_receipt_deadline_is_single_and_never_replenished() {
        crate::application::invocation::tests::canonical_handoff_boundary_is_direct_before_7000_and_durable_at_or_before_deadline();
        crate::application::invocation::tests::every_known_long_reason_materializes_before_execution_and_invalid_preparation_is_direct();
        super::server::actor_capacity_tests::daemon_receipt_deadline_is_not_replenished_after_delayed_prepare();
        server_captures_one_invocation_deadline_before_delayed_prepare_and_response_write();
        backpressured_response_uses_the_original_session_margin_without_reset();
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
            ServerResponse::error(DaemonErrorCode::ResultTooLarge),
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
        core_identity_is_closed_compile_time_abi_protocol_digest();
        daemon_result_size_and_session_bounds_are_enforced();
        assert_daemon_executes_one_canonical_invocation_and_poll_cancel_never_relaunches_it();
    }

    fn assert_daemon_executes_one_canonical_invocation_and_poll_cancel_never_relaunches_it() {
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
        let client = DaemonClient::new(DaemonClientConfig::existing_only(
            physical.clone(),
            identity.clone(),
        ));
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
        let initial = match outcome {
            InvocationResponse::Task(task) => task,
            other => panic!("known-long request did not return a task: {other:?}"),
        };
        let task_id = initial.task_id;
        entered_wait
            .recv_timeout(INTEGRATION_COORDINATION_TIMEOUT)
            .expect("canonical invocation must enter the service within the bounded wait");
        let task_deadline = owner
            .begin_task_deadline(Duration::from_millis(
                INTEGRATION_TASK_WAIT_MS + RESPONSE_SERIALIZATION_MARGIN_MS,
            ))
            .unwrap();
        let observed = owner.get_task_before(task_id, &task_deadline).unwrap();
        assert_eq!(observed.status, InvocationStatus::Working);
        assert_eq!(observed.created_at_epoch_ms, initial.created_at_epoch_ms);
        assert_eq!(observed.updated_at_epoch_ms, initial.updated_at_epoch_ms);
        assert_eq!(observed.ttl_ms, initial.ttl_ms);
        assert_eq!(
            owner
                .wait_task_before(task_id, 0, &task_deadline)
                .unwrap()
                .status,
            InvocationStatus::Working
        );
        let cancelled = owner.cancel_task_before(task_id, &task_deadline).unwrap();
        assert_eq!(cancelled.status, InvocationStatus::Cancelled);
        assert_eq!(cancelled.created_at_epoch_ms, initial.created_at_epoch_ms);
        assert!(cancelled.updated_at_epoch_ms >= initial.updated_at_epoch_ms);
        assert_eq!(
            owner.cancel_task_before(task_id, &task_deadline).unwrap(),
            cancelled
        );
        assert_eq!(
            owner.get_task_before(task_id, &task_deadline).unwrap(),
            cancelled
        );
        assert_eq!(executions.load(Ordering::SeqCst), 1);

        drop(owner);
        server.join().unwrap().unwrap();

        let restarted = thread::spawn({
            let root = root.path().to_path_buf();
            let identity = identity.clone();
            move || run_daemon(server_config(root, identity))
        });
        let (_directory, _record) = wait_for_record(root.path(), &identity);
        let client = DaemonClient::new(DaemonClientConfig::existing_only(physical, identity));
        let mut owner = match client.connect_existing().unwrap() {
            ExistingDaemon::Connected(owner) => owner,
            ExistingDaemon::Absent => panic!("restarted daemon must connect"),
        };
        let task_deadline = owner
            .begin_task_deadline(Duration::from_millis(
                INTEGRATION_TASK_WAIT_MS + RESPONSE_SERIALIZATION_MARGIN_MS,
            ))
            .unwrap();
        assert_eq!(
            owner.get_task_before(task_id, &task_deadline).unwrap(),
            cancelled
        );
        drop(owner);
        restarted.join().unwrap().unwrap();
    }

    #[test]
    fn daemon_executes_one_canonical_invocation_and_poll_cancel_never_relaunches_it() {
        assert_daemon_executes_one_canonical_invocation_and_poll_cancel_never_relaunches_it();
    }

    #[test]
    fn task_exchange_does_not_replace_the_frontend_cutoff_with_a_125ms_server_window() {
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
        let store = Arc::new(DelayedTaskReadStore {
            inner: DaemonMemoryStore::default(),
            read_delay: Duration::from_millis(RESPONSE_SERIALIZATION_MARGIN_MS + 50),
        });
        let config = server_config(root.path().to_path_buf(), identity.clone())
            .with_invocation_service(service)
            .with_invocation_store_for_test(store);
        let server = thread::spawn(move || run_daemon(config));
        let (_directory, _record) = wait_for_record(root.path(), &identity);
        let client = DaemonClient::new(DaemonClientConfig::existing_only(
            physical.clone(),
            identity,
        ));
        let mut owner = match client.connect_existing().unwrap() {
            ExistingDaemon::Connected(owner) => owner,
            ExistingDaemon::Absent => panic!("published daemon must connect"),
        };
        let task = owner
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
        let task_id = match task {
            InvocationResponse::Task(snapshot) => snapshot.task_id,
            other => panic!("known-long invocation did not materialize: {other:?}"),
        };
        entered_wait
            .recv_timeout(INTEGRATION_COORDINATION_TIMEOUT)
            .expect("canonical invocation must enter the service");

        let deadline = owner
            .begin_task_deadline(Duration::from_millis(
                INTEGRATION_TASK_WAIT_MS + RESPONSE_SERIALIZATION_MARGIN_MS,
            ))
            .unwrap();
        assert_eq!(
            owner.get_task_before(task_id, &deadline).unwrap().status,
            InvocationStatus::Working
        );
        assert_eq!(
            owner.cancel_task_before(task_id, &deadline).unwrap().status,
            InvocationStatus::Cancelled
        );

        drop(owner);
        server.join().unwrap().unwrap();
    }

    #[test]
    fn durability_uncertainty_stops_the_daemon_before_idle_grace() {
        let root = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        ensure_platform_xml_workspace(workspace.path());
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
        ensure_platform_xml_workspace(&workspace);
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
                    // This fixture exercises process-owned fail-stop and durable recovery. The
                    // zero-budget response cutoff is covered independently at the wire boundary.
                    INTEGRATION_TASK_WAIT_MS,
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

    #[test]
    fn injected_hidden_v13_service_executes_real_view_and_find_through_actor_capabilities() {
        let daemon_root = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let source = workspace.path().join("src");
        std::fs::create_dir_all(source.join("Catalogs")).unwrap();
        std::fs::write(
            workspace.path().join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        std::fs::write(
            source.join("Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Store</Name></Properties><ChildObjects><Catalog>Items</Catalog></ChildObjects></Configuration></MetaDataObject>"#,
        )
        .unwrap();
        std::fs::write(
            source.join("Catalogs/Items.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Catalog uuid="aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"><Properties><Name>Items</Name></Properties><ChildObjects/></Catalog></MetaDataObject>"#,
        )
        .unwrap();

        let identity = CoreIdentity::production();
        let config = server_config(daemon_root.path().to_path_buf(), identity.clone())
            .with_invocation_service(Arc::new(CanonicalV13ReadService::default()));
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

        let view = owner
            .submit_invocation(
                InvocationRequest::new(
                    ToolIdentity::View,
                    serde_json::json!({"at": "main:Catalog.Items"}),
                    physical_root(workspace.path()).to_string_lossy(),
                    7_000,
                )
                .unwrap(),
            )
            .unwrap();
        let InvocationResponse::Direct(view) = view else {
            panic!("hidden view should complete inline")
        };
        assert!(view.ok, "{} {:?}", view.summary, view.diagnostics);
        assert_eq!(view.at.as_deref(), Some("main:Catalog.Items"));
        assert_eq!(view.data.as_ref().unwrap()["kind"], "Catalog");
        assert!(view.rev.is_some());

        let find = owner
            .submit_invocation(
                InvocationRequest::new(
                    ToolIdentity::Find,
                    serde_json::json!({"query": "Items"}),
                    physical_root(workspace.path()).to_string_lossy(),
                    7_000,
                )
                .unwrap(),
            )
            .unwrap();
        let find = match find {
            InvocationResponse::Direct(find) => find,
            InvocationResponse::Task(snapshot) => {
                let terminal = owner
                    .wait_task(snapshot.task_id, INTEGRATION_TASK_WAIT_MS)
                    .unwrap();
                assert_eq!(terminal.status, InvocationStatus::Completed);
                terminal
                    .result
                    .expect("the one handed-off hidden find must publish its result")
            }
        };
        assert!(find.ok, "{} {:?}", find.summary, find.diagnostics);
        assert_eq!(
            find.data.as_ref().unwrap()["candidates"][0]["at"],
            "main:Catalog.Items"
        );
        assert!(find.rev.is_some());

        let unknown = owner
            .submit_invocation(
                InvocationRequest::new(
                    ToolIdentity::View,
                    serde_json::json!({
                        "at": "main:Catalog.Items",
                        "raw": true,
                    }),
                    physical_root(workspace.path()).to_string_lossy(),
                    7_000,
                )
                .unwrap(),
            )
            .unwrap();
        let InvocationResponse::Direct(unknown) = unknown else {
            panic!("invalid hidden arguments must fail before task materialization")
        };
        assert!(!unknown.ok);
        assert!(unknown.summary.contains("unknown argument `raw`"));

        drop(owner);
        server.join().unwrap().unwrap();
    }

    #[test]
    fn canonical_v13_service_gives_each_of_the_eight_surface_tools_a_useful_closed_mode() {
        struct PlatformDocsStandIn;

        impl crate::domain::documentation::DocumentationProvider for PlatformDocsStandIn {
            fn id(&self) -> crate::domain::documentation::DocumentationProviderId {
                crate::domain::documentation::DocumentationProviderId::new("platform-test")
            }

            fn corpora(&self) -> Vec<crate::domain::documentation::DocumentationCorpus> {
                vec![crate::domain::documentation::DocumentationCorpus {
                    id: "platform-test".to_string(),
                    source_kind: crate::domain::documentation::SourceKind::PlatformHelp,
                    authority: crate::domain::documentation::Authority::Vendor,
                }]
            }

            fn needs_network(&self) -> bool {
                false
            }

            fn search(
                &self,
                request: &crate::domain::documentation::DocumentationSearchRequest,
                _: &crate::domain::documentation::DocumentationContext,
            ) -> Vec<crate::domain::documentation::DocumentationSection> {
                vec![crate::domain::documentation::DocumentationSection::empty(
                    self.id(),
                    "platform-test",
                    crate::domain::documentation::SourceKind::PlatformHelp,
                    crate::domain::documentation::Authority::Vendor,
                    &request.language,
                )]
            }
        }

        let _docs =
            crate::infrastructure::application_ports::install_documentation_registry_stand_in(
                Arc::new(PlatformDocsStandIn),
            );
        let daemon_root = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let source = workspace.path().join("src");
        std::fs::create_dir_all(source.join("Catalogs/Items/Ext")).unwrap();
        std::fs::write(
            workspace.path().join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        std::fs::write(
            source.join("Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Store</Name></Properties><ChildObjects><Catalog>Items</Catalog></ChildObjects></Configuration></MetaDataObject>"#,
        )
        .unwrap();
        std::fs::write(
            source.join("Catalogs/Items.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Catalog uuid="aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"><Properties><Name>Items</Name><Synonym/><Comment/></Properties><ChildObjects/></Catalog></MetaDataObject>"#,
        )
        .unwrap();
        std::fs::write(
            source.join("Catalogs/Items/Ext/ObjectModule.bsl"),
            "Процедура Проверка()\n    UniqueSearchNeedle = Истина;\nКонецПроцедуры\n",
        )
        .unwrap();

        let identity = CoreIdentity::production();
        let config = server_config(daemon_root.path().to_path_buf(), identity.clone())
            .with_invocation_service(Arc::new(CanonicalV13ReadService::default()));
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
        let workspace_hint = physical_root(workspace.path())
            .to_string_lossy()
            .into_owned();
        let mut call = |tool, arguments| {
            let response = owner
                .submit_invocation(
                    InvocationRequest::new(tool, arguments, workspace_hint.as_str(), 7_000)
                        .unwrap(),
                )
                .unwrap();
            match response {
                InvocationResponse::Direct(result) => result,
                InvocationResponse::Task(snapshot) => {
                    let terminal = owner
                        .wait_task(snapshot.task_id, INTEGRATION_TASK_WAIT_MS)
                        .unwrap();
                    assert_eq!(terminal.status, InvocationStatus::Completed);
                    terminal
                        .result
                        .expect("canonical tool must publish a result")
                }
            }
        };

        let cases = [
            (
                ToolIdentity::View,
                serde_json::json!({"at": "main:Catalog.Items"}),
                "kind",
            ),
            (
                ToolIdentity::Find,
                serde_json::json!({"query": "Items"}),
                "candidates",
            ),
            (
                ToolIdentity::Search,
                serde_json::json!({
                    "query": "UniqueSearchNeedle",
                    "scope": "main:Configuration"
                }),
                "matches",
            ),
            (ToolIdentity::Check, serde_json::json!({}), "sources"),
            (
                ToolIdentity::Diff,
                serde_json::json!({
                    "left": "main:Catalog.Items",
                    "right": "main:Catalog.Items"
                }),
                "equal",
            ),
            (ToolIdentity::Run, serde_json::json!({}), "operations"),
            (
                ToolIdentity::Docs,
                serde_json::json!({
                    "query": "Items",
                    "source": "platform-help"
                }),
                "sections",
            ),
            (
                ToolIdentity::Apply,
                serde_json::json!({
                    "at": "main:Catalog.Items",
                    "ops": [{"op": "props.set", "args": {"values": {"Comment": "Preview"}}}],
                    "dryRun": true
                }),
                "validated",
            ),
        ];
        for (tool, arguments, expected_data_key) in cases {
            let result = call(tool, arguments);
            assert!(
                result.ok,
                "{} useful mode failed: {} {:?}",
                tool.catalog_name(),
                result.summary,
                result.diagnostics
            );
            assert!(
                result
                    .data
                    .as_ref()
                    .and_then(|data| data.get(expected_data_key))
                    .is_some(),
                "{} omitted data.{expected_data_key}: {:?}",
                tool.catalog_name(),
                result.data
            );
        }

        let unsupported = call(
            ToolIdentity::Run,
            serde_json::json!({"op": "client.run", "args": {}}),
        );
        assert!(!unsupported.ok);
        assert_eq!(unsupported.diagnostics[0]["code"], "unsupported_operation");

        drop(call);
        drop(owner);
        server.join().unwrap().unwrap();
    }

    #[test]
    fn canonical_v13_apply_dry_run_and_real_noop_use_the_exact_target_without_writes() {
        let mut mode_write_free = [false; 2];
        for (mode_index, dry_run) in [false, true].into_iter().enumerate() {
            let daemon_root = tempfile::tempdir().unwrap();
            let workspace = tempfile::tempdir().unwrap();
            let main = workspace.path().join("main");
            let secondary = workspace.path().join("secondary");
            let cache_root = workspace.path().join(".build/unica");
            std::fs::create_dir_all(&main).unwrap();
            std::fs::write(
                main.join("ConfigDumpInfo.xml"),
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><ExternalDataProcessor><Properties><Name>MainProcessor</Name></Properties><ChildObjects/></ExternalDataProcessor></MetaDataObject>"#,
            )
            .unwrap();
            std::fs::create_dir_all(secondary.join("Catalogs")).unwrap();
            std::fs::write(
                secondary.join("Configuration.xml"),
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Secondary</Name></Properties><ChildObjects><Catalog>Items</Catalog></ChildObjects></Configuration></MetaDataObject>"#,
            )
            .unwrap();
            std::fs::write(
                secondary.join("Catalogs/Items.xml"),
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Catalog uuid="aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"><Properties><Name>Items</Name><Synonym/><Comment/></Properties><ChildObjects/></Catalog></MetaDataObject>"#,
            )
            .unwrap();
            std::fs::write(
                workspace.path().join("v8project.yaml"),
                concat!(
                    "format: DESIGNER\n",
                    "source-set:\n",
                    "  - name: main\n    type: EXTERNAL_DATA_PROCESSORS\n    path: main\n",
                    "  - name: secondary\n    type: CONFIGURATION\n    path: secondary\n",
                ),
            )
            .unwrap();
            assert!(
                !cache_root.exists(),
                "fresh dryRun={dry_run} fixture unexpectedly has a cache root"
            );
            let workspace_before = tree_snapshot(workspace.path());
            let main_before = tree_snapshot(&main);
            let secondary_before = tree_snapshot(&secondary);
            let cache_before = cache_root.exists().then(|| tree_snapshot(&cache_root));

            let identity = CoreIdentity::production();
            let config = server_config(daemon_root.path().to_path_buf(), identity.clone())
                .with_invocation_service(Arc::new(CanonicalV13ReadService::default()));
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

            // Regression oracle: both modes use the exact targeted source set and
            // the real typed planner, while a net-zero plan remains write-free.
            let response = owner
                .submit_invocation(
                    InvocationRequest::new(
                        ToolIdentity::Apply,
                        serde_json::json!({
                            "at": "secondary:Catalog.Items",
                            "ops": [{"op": "props.set", "args": {"values": {"Comment": ""}}}],
                            "dryRun": dry_run,
                        }),
                        physical_root(workspace.path()).to_string_lossy(),
                        7_000,
                    )
                    .unwrap(),
                )
                .unwrap();
            let InvocationResponse::Direct(result) = response else {
                panic!("canonical apply mode must complete inline")
            };
            assert!(result.ok, "dryRun={dry_run} no-op failed: {result:?}");
            assert_eq!(result.at.as_deref(), Some("secondary:Catalog.Items"));
            assert_eq!(result.data.as_ref().unwrap()["validated"], true);
            assert_eq!(result.data.as_ref().unwrap()["executable"], true);
            assert_eq!(
                result.data.as_ref().unwrap()["mode"],
                if dry_run { "preview" } else { "published" }
            );
            assert!(result.changed.is_empty());

            drop(owner);
            server.join().unwrap().unwrap();
            let workspace_after = tree_snapshot(workspace.path());
            let main_after = tree_snapshot(&main);
            let secondary_after = tree_snapshot(&secondary);
            let cache_after = cache_root.exists().then(|| tree_snapshot(&cache_root));
            mode_write_free[mode_index] = workspace_after == workspace_before
                && main_after == main_before
                && secondary_after == secondary_before
                && cache_after == cache_before
                && !cache_root.exists();
        }
        assert_eq!(
            mode_write_free,
            [true, true],
            "canonical Apply mutated source/cache topology or bytes for [dryRun=false, dryRun=true], including a prohibited .build/unica/source-revisions/*.json publication"
        );
    }

    #[test]
    fn production_v3_daemon_default_executes_the_canonical_v13_service() {
        let daemon_root = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        ensure_platform_xml_workspace(workspace.path());
        std::fs::write(
            workspace.path().join("Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Store</Name></Properties><ChildObjects/></Configuration></MetaDataObject>"#,
        )
        .unwrap();
        let identity = CoreIdentity::production();
        let config = server_config(daemon_root.path().to_path_buf(), identity.clone());
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
                    ToolIdentity::View,
                    serde_json::json!({"at": "main:Configuration"}),
                    physical_root(workspace.path()).to_string_lossy(),
                    7_000,
                )
                .unwrap(),
            )
            .unwrap();
        let InvocationResponse::Direct(result) = response else {
            panic!("canonical v0.13 view should complete inline")
        };
        assert!(
            result.ok,
            "production v3 default must not use an unavailable service: {result:?}"
        );

        drop(owner);
        server.join().unwrap().unwrap();
    }

    fn canonical_service_reads_only_actor_bound_roots_and_persists_the_same_identity() {
        let daemon_root = tempfile::tempdir().unwrap();
        let workspace_a = tempfile::tempdir().unwrap();
        let workspace_b = tempfile::tempdir().unwrap();
        ensure_platform_xml_workspace(workspace_a.path());
        ensure_platform_xml_workspace(workspace_b.path());
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
        ensure_platform_xml_workspace(&workspace);
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
    fn canonical_invocation_orchestration_is_private_to_server_facade() {
        use quote::ToTokens;
        use syn::visit::Visit;

        fn tokens(node: &impl ToTokens) -> String {
            node.to_token_stream().to_string()
        }

        fn expected_use(source: &str) -> String {
            tokens(&syn::parse_str::<syn::ItemUse>(source).expect("expected import parses"))
        }

        fn expected_visibility(source: &str) -> String {
            tokens(&syn::parse_str::<syn::Visibility>(source).expect("expected visibility parses"))
        }

        #[derive(Default)]
        struct Uses(Vec<String>);

        impl<'ast> Visit<'ast> for Uses {
            fn visit_item_use(&mut self, item: &'ast syn::ItemUse) {
                self.0.push(tokens(item));
                syn::visit::visit_item_use(self, item);
            }
        }

        let daemon = syn::parse_file(include_str!("mod.rs")).expect("daemon module parses");
        assert!(
            !daemon.items.iter().any(
                |item| matches!(item, syn::Item::Mod(module) if module.ident == "invocation_service")
            ),
            "canonical invocation orchestration must not be a daemon sibling module"
        );
        let mut daemon_uses = Uses::default();
        daemon_uses.visit_file(&daemon);
        assert!(
            !daemon_uses
                .0
                .iter()
                .any(|item| item.contains("super :: invocation_service")),
            "daemon tests must consume the canonical service seam through server"
        );
        let daemon_test_seam = expected_use(
            "use super::server::{ActorBoundExecution, ActorBoundInvocation, CanonicalInvocationService};",
        );
        assert!(
            daemon_uses.0.contains(&daemon_test_seam),
            "daemon tests must use the exact server facade seam"
        );

        let server = syn::parse_file(include_str!("server.rs")).expect("server source parses");
        let service_module = server
            .items
            .iter()
            .find_map(|item| match item {
                syn::Item::Mod(module) if module.ident == "invocation_service" => Some(module),
                _ => None,
            })
            .expect("server owns the canonical invocation implementation module");
        assert!(
            matches!(service_module.vis, syn::Visibility::Inherited),
            "the invocation implementation module must remain private to server"
        );
        let module_path = service_module
            .attrs
            .iter()
            .find_map(|attribute| match &attribute.meta {
                syn::Meta::NameValue(name_value) if name_value.path.is_ident("path") => {
                    match &name_value.value {
                        syn::Expr::Lit(syn::ExprLit {
                            lit: syn::Lit::Str(path),
                            ..
                        }) => Some(path.value()),
                        _ => None,
                    }
                }
                _ => None,
            });
        assert_eq!(
            module_path.as_deref(),
            Some("invocation_service.rs"),
            "server must remain the stable producer/facade while locating the extracted owner"
        );
        let mut server_uses = Uses::default();
        server_uses.visit_file(&server);
        let public_seam = expected_use(
            "pub(crate) use self::invocation_service::{ActorBoundExecution, ActorBoundInvocation, CanonicalInvocationService,};",
        );
        assert!(
            server_uses.0.contains(&public_seam),
            "server must expose only the exact canonical service seam"
        );

        let v13 = syn::parse_file(include_str!("v13_service.rs")).expect("v13 service parses");
        let mut v13_uses = Uses::default();
        v13_uses.visit_file(&v13);
        let v13_seam = expected_use(
            "use super::server::{ActorBoundExecution, ActorBoundInvocation, CanonicalInvocationService};",
        );
        assert_eq!(
            v13_uses
                .0
                .iter()
                .filter(|item| item.contains("CanonicalInvocationService"))
                .collect::<Vec<_>>(),
            vec![&v13_seam],
            "the canonical v0.13 consumer must enter through the server facade"
        );

        let owner = syn::parse_file(include_str!("invocation_service.rs"))
            .expect("invocation service owner parses");
        let server_only = expected_visibility("pub(super)");
        let bind_workspace_invocation = owner
            .items
            .iter()
            .find_map(|item| match item {
                syn::Item::Fn(function) if function.sig.ident == "bind_workspace_invocation" => {
                    Some(function)
                }
                _ => None,
            })
            .expect("missing server-only function bind_workspace_invocation");
        assert_eq!(
            tokens(&bind_workspace_invocation.vis),
            server_only,
            "bind_workspace_invocation escaped server"
        );
        let admission_error = owner
            .items
            .iter()
            .find_map(|item| match item {
                syn::Item::Enum(item) if item.ident == "WorkspaceAdmissionError" => Some(item),
                _ => None,
            })
            .expect("workspace admission error exists");
        assert_eq!(
            tokens(&admission_error.vis),
            server_only,
            "workspace admission routing escaped server"
        );
        for name in ["response_deadline", "begin_execution", "publish"] {
            let methods = owner
                .items
                .iter()
                .filter_map(|item| match item {
                    syn::Item::Impl(item) => Some(item),
                    _ => None,
                })
                .flat_map(|item| item.items.iter())
                .filter_map(|item| match item {
                    syn::ImplItem::Fn(method) if method.sig.ident == name => Some(method),
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert_eq!(methods.len(), 1, "expected one orchestration method {name}");
            assert_eq!(
                tokens(&methods[0].vis),
                server_only,
                "{name} escaped server"
            );
        }

        let daemon_visible = expected_visibility("pub(in crate::infrastructure::daemon)");
        let capability = owner
            .items
            .iter()
            .find_map(|item| match item {
                syn::Item::Struct(item) if item.ident == "ActorReadSourceCapability" => Some(item),
                _ => None,
            })
            .expect("actor read capability exists");
        assert_eq!(
            tokens(&capability.vis),
            daemon_visible,
            "the pre-existing daemon capability seam changed visibility"
        );
        for name in ["admitted_source_set_names", "admit_apply", "read_sources"] {
            let method = owner
                .items
                .iter()
                .filter_map(|item| match item {
                    syn::Item::Impl(item) => Some(item),
                    _ => None,
                })
                .flat_map(|item| item.items.iter())
                .find_map(|item| match item {
                    syn::ImplItem::Fn(method) if method.sig.ident == name => Some(method),
                    _ => None,
                })
                .unwrap_or_else(|| panic!("missing daemon-visible method {name}"));
            assert_eq!(
                tokens(&method.vis),
                daemon_visible,
                "the pre-existing daemon method {name} changed visibility"
            );
        }
    }

    #[test]
    fn canonical_service_boundary_exposes_no_raw_request_or_workspace_hint() {
        let source = include_str!("invocation_service.rs");
        let trait_start = source
            .find("pub(crate) trait CanonicalInvocationService")
            .expect("canonical service trait");
        let trait_end = source[trait_start..]
            .find("\n}\npub(super) fn bind_workspace_invocation")
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
        super::server::actor_capacity_tests::hidden_v13_logical_lease_survives_the_handoff_window_and_confirms_once();
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
                "persisted record contains forbidden text"
            );
        }
        let record: crate::application::invocation_store::StoredInvocationRecord =
            serde_json::from_slice(&bytes).unwrap();
        assert_eq!(record.tool, ToolIdentity::Run);
        assert_eq!(record.status, InvocationStatus::Working);

        let task_deadline = owner
            .begin_task_deadline(Duration::from_millis(
                INTEGRATION_TASK_WAIT_MS + RESPONSE_SERIALIZATION_MARGIN_MS,
            ))
            .unwrap();
        owner.cancel_task_before(task_id, &task_deadline).unwrap();
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
    fn secret_bearing_daemon_protocol_debug_is_redacted() {
        let identity = CoreIdentity::production();
        let record = EndpointRecord::new(identity.clone(), 4321);
        let endpoint_token = record.token().to_string();
        let endpoint_instance = record.instance_id().to_string();
        let record_debug = format!("{record:?}");
        assert!(!record_debug.contains(&endpoint_token));
        assert!(!record_debug.contains(&endpoint_instance));
        assert!(record_debug.contains("EndpointRecord"));
        assert_eq!(record_debug.matches("<redacted>").count(), 2);

        let owner_lease = "eeeeeeee-eeee-4eee-8eee-eeeeeeeeeeee";
        let hello = ClientRequest::hello_with_owner_for_test(
            endpoint_token.clone(),
            identity,
            owner_lease.to_string(),
        );
        let hello_debug = format!("{hello:?}");
        assert!(!hello_debug.contains(&endpoint_token));
        assert!(!hello_debug.contains(owner_lease));
        assert!(hello_debug.contains("Hello"));
        assert_eq!(hello_debug.matches("<redacted>").count(), 2);

        let argument_secret = "daemon-debug-argument-secret";
        let workspace_secret = "/private/daemon-debug-workspace-secret";
        let invocation = InvocationRequest::new(
            ToolIdentity::Run,
            serde_json::json!({"password": argument_secret}),
            workspace_secret,
            0,
        )
        .unwrap();
        let invocation_debug = format!("{invocation:?}");
        assert!(!invocation_debug.contains(argument_secret));
        assert!(!invocation_debug.contains(workspace_secret));
        assert!(invocation_debug.contains("InvocationRequest"));
        assert_eq!(invocation_debug.matches("<redacted>").count(), 2);

        let submit_debug = format!("{:?}", ClientRequest::submit_invocation(invocation));
        assert!(!submit_debug.contains(argument_secret));
        assert!(!submit_debug.contains(workspace_secret));
        assert!(submit_debug.contains("SubmitInvocation"));
        assert_eq!(submit_debug.matches("<redacted>").count(), 2);
    }

    #[test]
    fn complete_malformed_handshake_is_invalid_request() {
        let root = tempfile::tempdir().unwrap();
        let identity = CoreIdentity::production();
        let config = server_config(root.path().to_path_buf(), identity.clone());
        let server = thread::spawn(move || run_daemon(config));
        let (_directory, record) = wait_for_record(root.path(), &identity);
        let mut stream = TcpStream::connect(record.loopback_addr().unwrap()).unwrap();

        stream.write_all(b"{not-json}\n").unwrap();
        stream.flush().unwrap();
        let response: ServerResponse =
            serde_json::from_slice(&read_bounded_json_line(&mut BufReader::new(&stream)).unwrap())
                .unwrap();

        assert_eq!(response.error_code(), Some(DaemonErrorCode::InvalidRequest));
        server.join().unwrap().unwrap();
    }

    #[test]
    fn truncated_handshake_transport_closes_without_a_protocol_response() {
        let root = tempfile::tempdir().unwrap();
        let identity = CoreIdentity::production();
        let config = server_config(root.path().to_path_buf(), identity.clone());
        let server = thread::spawn(move || run_daemon(config));
        let (_directory, record) = wait_for_record(root.path(), &identity);
        let mut stream = TcpStream::connect(record.loopback_addr().unwrap()).unwrap();

        stream.write_all(b"{\"kind\":\"hello\"").unwrap();
        stream.flush().unwrap();
        stream.shutdown(std::net::Shutdown::Write).unwrap();

        assert!(
            stream_closed_without_response(&mut stream),
            "a truncated transport was mislabeled as invalid_request"
        );
        server.join().unwrap().unwrap();
    }

    #[test]
    fn established_handshake_transport_failure_reopens_the_endpoint() {
        let root = tempfile::tempdir().unwrap();
        let physical = physical_root(root.path());
        let identity = CoreIdentity::production();
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let record = EndpointRecord::new(identity.clone(), listener.local_addr().unwrap().port());
        let directory = DaemonStateDirectory::open(&physical, &identity).unwrap();
        directory.write_endpoint_record_for_test(&record).unwrap();
        let peer_record = record.clone();
        let fake_peer = thread::spawn(move || {
            let (first, _) = listener.accept().unwrap();
            let first_hello = read_bounded_json_line(&mut BufReader::new(&first)).unwrap();
            assert!(matches!(
                super::protocol::parse_request(&first_hello).unwrap(),
                ClientRequest::Hello { .. }
            ));
            drop(first);

            let (mut second, _) = listener.accept().unwrap();
            let second_hello = read_bounded_json_line(&mut BufReader::new(&second));
            if second_hello.is_err() {
                return false;
            }
            write_json_line(&mut second, &ServerResponse::ready(&peer_record));
            true
        });
        let config = DaemonClientConfig::existing_only(physical, identity)
            .with_connect_timeout_for_test(Duration::from_secs(1));
        let client = DaemonClient::new(config);

        let connection = client.connect_existing();
        if connection.is_err() {
            let _ = TcpStream::connect(record.loopback_addr().unwrap());
        }
        let served_retry = fake_peer.join().unwrap();

        assert!(
            matches!(connection, Ok(ExistingDaemon::Connected(_))),
            "established handshake transport failure was not retried"
        );
        assert!(served_retry, "retry did not use a fresh connection");
    }

    #[test]
    fn peer_handshake_transport_failure_reopens_under_the_original_deadline() {
        let root = tempfile::tempdir().unwrap();
        let physical = physical_root(root.path());
        let identity = CoreIdentity::production();
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let record = EndpointRecord::new(identity.clone(), listener.local_addr().unwrap().port());
        let directory = DaemonStateDirectory::open(&physical, &identity).unwrap();
        directory.write_endpoint_record_for_test(&record).unwrap();
        let peer_record = record.clone();
        let fake_peer = thread::spawn(move || {
            let (mut anchor, _) = listener.accept().unwrap();
            let _anchor_hello = read_bounded_json_line(&mut BufReader::new(&anchor)).unwrap();
            write_json_line(&mut anchor, &ServerResponse::ready(&peer_record));

            let (first_peer, _) = listener.accept().unwrap();
            let _first_hello = read_bounded_json_line(&mut BufReader::new(&first_peer)).unwrap();
            drop(first_peer);

            let (mut second_peer, _) = listener.accept().unwrap();
            let second_hello = read_bounded_json_line(&mut BufReader::new(&second_peer));
            if second_hello.is_err() {
                return false;
            }
            write_json_line(&mut second_peer, &ServerResponse::ready(&peer_record));
            true
        });
        let config = DaemonClientConfig::existing_only(physical, identity)
            .with_connect_timeout_for_test(Duration::from_secs(1));
        let client = DaemonClient::new(config);
        let anchor = match client.connect_existing().unwrap() {
            ExistingDaemon::Connected(owner) => owner,
            ExistingDaemon::Absent => panic!("fake endpoint must connect"),
        };

        let peer = anchor.connect_peer(Duration::from_secs(1));
        if peer.is_err() {
            let _ = TcpStream::connect(record.loopback_addr().unwrap());
        }
        let served_retry = fake_peer.join().unwrap();

        if let Err(error) = &peer {
            panic!("peer handshake transport was not retried: {error}");
        }
        assert!(served_retry, "peer retry did not use a fresh connection");
    }

    #[test]
    fn invalid_request_handshake_rejection_is_never_retried() {
        let root = tempfile::tempdir().unwrap();
        let physical = physical_root(root.path());
        let identity = CoreIdentity::production();
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        listener.set_nonblocking(true).unwrap();
        let record = EndpointRecord::new(identity.clone(), listener.local_addr().unwrap().port());
        let directory = DaemonStateDirectory::open(&physical, &identity).unwrap();
        directory.write_endpoint_record_for_test(&record).unwrap();
        let fake_peer = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(1);
            let (mut first, _) = loop {
                match listener.accept() {
                    Ok(connection) => break connection,
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        assert!(Instant::now() < deadline, "client did not connect");
                        thread::yield_now();
                    }
                    Err(error) => panic!("accept first handshake: {error}"),
                }
            };
            first.set_nonblocking(false).unwrap();
            let _hello = read_bounded_json_line(&mut BufReader::new(&first)).unwrap();
            write_json_line(
                &mut first,
                &ServerResponse::error(DaemonErrorCode::InvalidRequest),
            );
            drop(first);
            let retry_deadline = Instant::now() + Duration::from_millis(150);
            loop {
                match listener.accept() {
                    Ok(_) => return true,
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        if Instant::now() >= retry_deadline {
                            return false;
                        }
                        thread::yield_now();
                    }
                    Err(error) => panic!("observe handshake retry: {error}"),
                }
            }
        });
        let config = DaemonClientConfig::existing_only(physical, identity)
            .with_connect_timeout_for_test(Duration::from_secs(1));
        let client = DaemonClient::new(config);

        let error = client.connect_existing().unwrap_err();
        let retried = fake_peer.join().unwrap();

        assert_eq!(error, "daemon handshake rejected: invalid_request");
        assert!(!retried, "invalid_request opened a second connection");
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
    fn task_exchange_propagates_closed_codes_and_poison_without_parsing_text() {
        let root = tempfile::tempdir().unwrap();
        let physical = physical_root(root.path());
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
            for code in [DaemonErrorCode::TaskNotFound, DaemonErrorCode::TaskExpired] {
                assert!(matches!(
                    super::protocol::parse_request(&read_bounded_json_line(&mut reader).unwrap())
                        .unwrap(),
                    ClientRequest::GetTask { .. }
                ));
                write_json_line(&mut stream, &ServerResponse::error(code));
            }
            let _third = read_bounded_json_line(&mut reader).unwrap();
            stream.write_all(b"{malformed task response}\n").unwrap();
            stream.flush().unwrap();
        });
        let client = DaemonClient::new(DaemonClientConfig::existing_only(physical, identity));
        let mut owner = match client.connect_existing().unwrap() {
            ExistingDaemon::Connected(owner) => owner,
            ExistingDaemon::Absent => panic!("fake task endpoint must connect"),
        };

        assert_eq!(
            owner.get_task(TaskId::new()),
            Err(DaemonTaskExchangeError::Protocol(
                DaemonErrorCode::TaskNotFound
            ))
        );
        assert_eq!(
            owner.get_task(TaskId::new()),
            Err(DaemonTaskExchangeError::Protocol(
                DaemonErrorCode::TaskExpired
            ))
        );
        assert_eq!(
            owner.get_task(TaskId::new()),
            Err(DaemonTaskExchangeError::Transport)
        );
        assert_eq!(
            owner.get_task(TaskId::new()),
            Err(DaemonTaskExchangeError::SessionPoisoned)
        );
        fake_peer.join().unwrap();
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
        clock.advance_after_next_response_read(Duration::from_secs(5));
        let peer_record = record.clone();
        let fake_peer = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let _hello = read_bounded_json_line(&mut BufReader::new(&stream)).unwrap();
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
        clock.advance_after_next_response_read(Duration::from_secs(5));
        let fake_peer = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let _hello = read_bounded_json_line(&mut BufReader::new(&stream)).unwrap();
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
        clock.advance_after_next_response_read(Duration::from_secs(5));
        let fake_peer = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let _hello = read_bounded_json_line(&mut BufReader::new(&stream)).unwrap();
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
            write_json_line(&mut stream, &ServerResponse::Pong);
        });
        let config = DaemonClientConfig::existing_only(physical, identity)
            .with_clock_for_test(clock.clone())
            .with_connect_timeout_for_test(Duration::from_secs(5));
        let client = DaemonClient::new(config);
        let mut owner = match client.connect_existing().unwrap() {
            ExistingDaemon::Connected(owner) => owner,
            ExistingDaemon::Absent => panic!("fake endpoint must connect"),
        };
        clock.advance_after_next_response_read(Duration::from_secs(5));

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
        let startup_pause = install_startup_pause();
        let handshake_pause = install_handshake_pause();
        let root = tempfile::tempdir().unwrap();
        let identity = CoreIdentity::production();
        let config = DaemonServerConfig::new(
            physical_root(root.path()),
            identity.clone(),
            Duration::from_millis(80),
        )
        .with_startup_pause(&startup_pause)
        .with_handshake_pause(&handshake_pause);
        let server = thread::spawn(move || run_daemon(config));
        let (directory, record) = wait_for_record(root.path(), &identity);
        startup_pause.wait_until_entered();
        let mut stream = TcpStream::connect(record.loopback_addr().unwrap()).unwrap();
        let request = ClientRequest::hello(
            DAEMON_PROTOCOL_VERSION,
            record.token().to_string(),
            identity,
        );
        write_json_line(&mut stream, &request);
        startup_pause.release();
        handshake_pause.wait_until_entered();

        thread::sleep(Duration::from_millis(160));
        assert!(
            !server.is_finished(),
            "admitted handshake lost the daemon to idle exit"
        );
        assert_eq!(
            directory.read_endpoint_record().unwrap(),
            Some(record.clone())
        );

        handshake_pause.release();
        let mut reader = BufReader::new(&stream);
        let ready: ServerResponse =
            serde_json::from_slice(&read_bounded_json_line(&mut reader).unwrap()).unwrap();
        assert!(ready.matches_record(&record));
        drop(reader);
        write_json_line(&mut stream, &ClientRequest::Release {});
        server.join().unwrap().unwrap();
    }

    #[test]
    fn preauth_deadline_starts_before_the_handler_thread_runs() {
        let pause = install_handshake_pause();
        let root = tempfile::tempdir().unwrap();
        let identity = CoreIdentity::production();
        let config =
            server_config(root.path().to_path_buf(), identity.clone()).with_handshake_pause(&pause);
        let server = thread::spawn(move || run_daemon(config));
        let (_directory, record) = wait_for_record(root.path(), &identity);
        let mut stream = TcpStream::connect(record.loopback_addr().unwrap()).unwrap();
        let hello = ClientRequest::hello(
            DAEMON_PROTOCOL_VERSION,
            record.token().to_string(),
            identity,
        );
        write_json_line(&mut stream, &hello);
        pause.wait_until_entered();

        thread::sleep(Duration::from_millis(2_100));
        pause.release();

        assert!(
            stream_closed_without_response(&mut stream),
            "handler scheduling replenished the preauthentication deadline"
        );
        server.join().unwrap().unwrap();
    }

    #[test]
    fn partial_handshake_bytes_cannot_replenish_the_preauth_deadline() {
        let root = tempfile::tempdir().unwrap();
        let identity = CoreIdentity::production();
        let config = server_config(root.path().to_path_buf(), identity.clone());
        let server = thread::spawn(move || run_daemon(config));
        let (_directory, record) = wait_for_record(root.path(), &identity);
        let mut stream = TcpStream::connect(record.loopback_addr().unwrap()).unwrap();
        let hello = ClientRequest::hello(
            DAEMON_PROTOCOL_VERSION,
            record.token().to_string(),
            identity,
        );
        let mut bytes = serde_json::to_vec(&hello).unwrap();
        bytes.push(b'\n');
        let first = bytes.len() / 3;
        let second = first * 2;

        stream.write_all(&bytes[..first]).unwrap();
        stream.flush().unwrap();
        thread::sleep(Duration::from_millis(1_100));
        stream.write_all(&bytes[first..second]).unwrap();
        stream.flush().unwrap();
        thread::sleep(Duration::from_millis(1_100));
        let _ = stream.write_all(&bytes[second..]);
        let _ = stream.flush();
        let _ = stream.shutdown(std::net::Shutdown::Write);

        assert!(
            stream_closed_without_response(&mut stream),
            "partial reads replenished the two-second preauthentication deadline"
        );
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
            .write_all(&vec![b'x'; MAX_DAEMON_REQUEST_LINE_BYTES + 1])
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
