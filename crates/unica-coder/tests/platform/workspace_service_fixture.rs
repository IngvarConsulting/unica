//! Помощники тестов службы рабочей области: контролируемый помощник
//! инвалидации и запросы к службе. Ими пользуются тесты `workspace_services`
//! этого крейта и тесты демона, которые держат ту же службу под живым
//! демоном.

use crate::infrastructure::analysis_composition;
use serde_json::json;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex,
};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use unica_adapter_bsl_analyzer::session::BslWorkspaceBackend;
use unica_analysis_services::protocol::*;
use unica_analysis_services::test_support::*;
use unica_analysis_services::{
    WorkspaceServiceIdentity, WorkspaceServiceRecord, WorkspaceServices,
};
use unica_daemon::identity::CoreIdentity;
use unica_process::mcp_session::PersistentMcpSession;
use unica_source_contract::events::DomainEvent;
use unica_source_contract::workspace::WorkspaceContext;
use uuid::Uuid;

type WorkspaceServiceRuntime = WorkspaceServices<BslWorkspaceBackend>;

pub fn new_runtime(
    identity: WorkspaceServiceIdentity,
    record: &WorkspaceServiceRecord,
) -> WorkspaceServiceRuntime {
    WorkspaceServices::new(
        identity,
        record,
        Arc::new(BslWorkspaceBackend::new(Arc::new(
            analysis_composition::AnalysisEnvironment,
        ))),
        analysis_composition::service_environment(),
    )
}

pub const SNAPSHOT_MODULE: &str = "CommonModules/SearchProbe/Ext/Module.bsl";
pub const SNAPSHOT_BEFORE: &str = "Procedure AcceptanceExistingSymbol1306() Export\nEndProcedure\n";
pub const SNAPSHOT_AFTER: &str = "Procedure AcceptanceChangedSymbol978() Export\nEndProcedure\n";

pub struct CanonicalInvalidationHelper {
    pub record: WorkspaceServiceRecord,
    pub runtime: Arc<WorkspaceServiceRuntime>,
    pub notifications: Arc<Mutex<Vec<Vec<DomainEvent>>>>,
    pub connections: Arc<AtomicUsize>,
    pub pid_file: PathBuf,
    pub server: Option<thread::JoinHandle<Result<(), String>>>,
}

impl CanonicalInvalidationHelper {
    pub fn start(context: &WorkspaceContext, source_root: &Path, executable: &Path) -> Self {
        Self::start_with_cleanup_gate(context, source_root, executable, None)
    }

    pub fn start_with_cleanup_gate(
        context: &WorkspaceContext,
        source_root: &Path,
        executable: &Path,
        cleanup_gate: Option<PathBuf>,
    ) -> Self {
        Self::start_in_directory(context, source_root, executable, cleanup_gate, None)
    }

    pub fn start_in_directory(
        context: &WorkspaceContext,
        source_root: &Path,
        executable: &Path,
        cleanup_gate: Option<PathBuf>,
        service_dir: Option<PathBuf>,
    ) -> Self {
        let mut identity = WorkspaceServiceIdentity::new(context, source_root).unwrap();
        if let Some(service_dir) = service_dir {
            identity.service_dir = service_dir;
        }
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        listener.set_nonblocking(true).unwrap();
        let record = test_record(
            &identity,
            listener.local_addr().unwrap().port(),
            CoreIdentity::production().as_str(),
        );
        write_record(&identity, &record).unwrap();
        let pid_file = identity.service_dir.join("snapshot-pids.txt");
        let mut runtime = new_runtime(identity, &record);
        runtime.test_set_analyzer_starter(Arc::new({
            let executable = executable.to_path_buf();
            let pid_file = pid_file.clone();
            move |_, source_root, cancellation| {
                let mut command = Command::new(&executable);
                command
                    .args([source_root.join(SNAPSHOT_MODULE), pid_file.clone()])
                    .arg("analyzer")
                    .arg(pid_file.with_extension("lease"));
                if let Some(cleanup_gate) = &cleanup_gate {
                    command.arg(cleanup_gate);
                }
                PersistentMcpSession::start_with_command(command, cancellation)
            }
        }));
        let notifications = Arc::new(Mutex::new(Vec::new()));
        let connections = Arc::new(AtomicUsize::new(0));
        *runtime.test_handler_started_hook().lock().unwrap() = Some(Arc::new({
            let connections = Arc::clone(&connections);
            move || {
                connections.fetch_add(1, Ordering::AcqRel);
            }
        }));
        runtime.test_set_authenticated_request_hook(Some(Arc::new({
            let notifications = Arc::clone(&notifications);
            move |_, request| {
                if let ServiceRequestKind::Invalidate { events } = request {
                    notifications.lock().unwrap().push(events.clone());
                }
            }
        })));
        let runtime = Arc::new(runtime);
        let server_runtime = Arc::clone(&runtime);
        let server = thread::spawn(move || {
            serve_workspace_service(
                listener,
                server_runtime,
                Arc::new(SystemWorkspaceServiceOperationExecutor),
                Duration::from_secs(120),
                SERVICE_REQUEST_HEADER_TIMEOUT,
                SERVICE_SHUTDOWN_GRACE,
            )
        });
        Self {
            record,
            runtime,
            notifications,
            connections,
            pid_file,
            server: Some(server),
        }
    }

    pub fn notifications(&self) -> Vec<Vec<DomainEvent>> {
        self.notifications.lock().unwrap().clone()
    }

    pub fn snapshot(&self) -> String {
        let response = send_test_request(
            &self.record,
            ServiceRequestKind::BslMcp {
                operation_id: Uuid::new_v4().to_string(),
                tool_name: "search".into(),
                tool_args: json!({"action": "search_code", "query": "Acceptance"}),
                timeout_seconds: 10,
                timeout_nanos: 0,
                yields: false,
            },
        );
        assert!(response.ok, "{response:?}");
        response.result_text.unwrap()
    }

    pub fn pids(&self) -> Vec<u32> {
        fs::read_to_string(&self.pid_file)
            .unwrap()
            .lines()
            .map(|line| line.parse().unwrap())
            .collect()
    }

    pub fn stop_and_verify(mut self) {
        let pids = self.pids();
        let mut readers = Vec::new();
        if let Some(session) = self.runtime.test_analyzer().lock().unwrap().as_ref() {
            readers.push(session.test_reader_terminal());
        }
        assert!(
            !readers.is_empty(),
            "controlled providers must have been warmed"
        );
        self.runtime.test_begin_shutdown();
        self.server
            .take()
            .unwrap()
            .join()
            .expect("controlled helper server must join")
            .expect("controlled helper provider teardown must finish");
        for pid in pids {
            assert!(
                wait_for_process_exit(pid, Duration::from_secs(5)),
                "owned controlled provider {pid} did not exit"
            );
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        while readers.iter().any(|reader| !reader.load(Ordering::Acquire)) {
            assert!(
                Instant::now() < deadline,
                "controlled provider stdout did not close"
            );
            thread::yield_now();
        }
    }
}

impl Drop for CanonicalInvalidationHelper {
    fn drop(&mut self) {
        self.runtime.test_begin_shutdown();
        if let Some(server) = self.server.take() {
            let _ = server.join();
        }
    }
}

pub fn open_test_request(
    record: &WorkspaceServiceRecord,
    kind: ServiceRequestKind,
) -> BufReader<TcpStream> {
    let mut stream = TcpStream::connect(("127.0.0.1", record.port)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let request = ServiceRequest {
        token: record.token.clone(),
        kind,
    };
    writeln!(stream, "{}", serde_json::to_string(&request).unwrap()).unwrap();
    stream.flush().unwrap();
    BufReader::new(stream)
}

pub fn read_test_response(reader: &mut BufReader<TcpStream>) -> ServiceResponse {
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    serde_json::from_str(line.trim()).unwrap()
}

pub fn send_test_request(
    record: &WorkspaceServiceRecord,
    kind: ServiceRequestKind,
) -> ServiceResponse {
    read_test_response(&mut open_test_request(record, kind))
}

pub fn wait_for_file(path: &Path, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    while !path.is_file() {
        assert!(Instant::now() < deadline, "fixture marker was not written");
        thread::yield_now();
    }
}

pub fn wait_for_process_exit(pid: u32, timeout: Duration) -> bool {
    unica_process::test_support::wait_for_process_exit(pid, timeout)
}

pub fn now_secs_for_test() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

pub fn test_record(
    identity: &WorkspaceServiceIdentity,
    port: u16,
    build: &str,
) -> WorkspaceServiceRecord {
    WorkspaceServiceRecord {
        schema_version: SERVICE_SCHEMA_VERSION,
        pid: std::process::id(),
        port,
        token: "secret".to_string(),
        build: build.to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        workspace_root: identity.workspace_root.clone(),
        source_root: identity.source_root.clone(),
        started_at: now_secs_for_test(),
        last_access_at: now_secs_for_test(),
    }
}
