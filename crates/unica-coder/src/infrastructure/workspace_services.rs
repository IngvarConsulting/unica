use crate::domain::cancellation::{cancelled_error, CancellationToken};
use crate::domain::code_intelligence::ProviderDeadline;
use crate::domain::events::{DomainEvent, DomainEventKind};
use crate::domain::source_revision::SourceRevision;
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::bundled_tools::resolve_bundled_tool;
use crate::infrastructure::platform::{
    short_private_runtime_dir, ManagedChild, ManagedStartupChild,
};
use crate::infrastructure::plugin_runtime::find_plugin_root;
use crate::infrastructure::source_revision::SourceRevisionService;
use crate::infrastructure::source_roots::{normalize_path_identity, source_generation_until};
use crate::infrastructure::workspace_index::{
    ready_index_for_source_revision, IndexBackgroundTaskTracker, IndexReadiness, SystemIndexRunner,
    WorkspaceIndexService, SOURCE_GENERATION_STALE_STATUS, SOURCE_REVISION_ARG,
    SOURCE_REVISION_GENERATION_ARG,
};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::{hash_map::DefaultHasher, HashMap, VecDeque};
use std::env;
use std::fs::{self, OpenOptions};
use std::hash::{Hash, Hasher};
use std::io::{self, BufRead, BufReader, ErrorKind, Read, Seek, SeekFrom, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{ChildStdin, Command, Stdio};
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    mpsc, Arc, Condvar, Mutex,
};
use std::thread;
use std::time::{Duration, Instant};
use uuid::Uuid;

const SERVICE_SCHEMA_VERSION: u32 = 4;
const DEFAULT_IDLE_SECS: u64 = 7200;
const DEFAULT_MAX_AGE_SECS: u64 = 28800;
const SERVICE_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const SERVICE_CONTROL_CONNECT_TIMEOUT: Duration = Duration::from_millis(500);
const SERVICE_REQUEST_TIMEOUT: Duration = Duration::from_secs(120);
const RLM_LOGICAL_SESSION_RETRY_LIMIT: usize = 2;
const SERVICE_SHUTDOWN_GRACE: Duration = Duration::from_secs(2);
const SERVICE_RESPONSE_LINE_LIMIT: usize = 8 * 1024 * 1024;
const SERVICE_REQUEST_LINE_LIMIT: usize = 8 * 1024 * 1024;
const SERVICE_MAX_CONNECTION_HANDLERS: usize = 64;
const SERVICE_MAX_CONTROL_HANDLERS: usize = 8;
const SERVICE_MAX_PENDING_CONTROL: usize = 64;
const SERVICE_CONTROL_CLASSIFICATION_LIMIT: usize = 64 * 1024;
const SERVICE_MAX_WORKERS: usize = 8;
const SERVICE_REQUEST_HEADER_TIMEOUT: Duration = Duration::from_secs(5);
const SESSION_TEARDOWN_GRACE: Duration = Duration::from_secs(3);
const SERVICE_SPAWN_CLEANUP_WAIT: Duration = Duration::from_secs(2);
const SERVICE_STARTUP_STDERR_TAIL_LIMIT: u64 = 16 * 1024;
const BSL_STDERR_TAIL_LIMIT: usize = 64 * 1024;
const SERVICE_RECORD_LOCK_FILE: &str = "service.record.lock";

static SYSTEM_SERVICE_CONNECTOR: SystemServiceConnector = SystemServiceConnector;
static SYSTEM_SERVICE_SPAWNER: SystemServiceSpawner = SystemServiceSpawner;

struct AdmissionGate {
    active: AtomicUsize,
    limit: usize,
}

impl AdmissionGate {
    fn new(limit: usize) -> Arc<Self> {
        Arc::new(Self {
            active: AtomicUsize::new(0),
            limit,
        })
    }

    fn try_acquire(self: &Arc<Self>) -> Option<AdmissionPermit> {
        let mut active = self.active.load(Ordering::Acquire);
        loop {
            if active >= self.limit {
                return None;
            }
            match self.active.compare_exchange_weak(
                active,
                active + 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return Some(AdmissionPermit(Arc::clone(self))),
                Err(next) => active = next,
            }
        }
    }
}

struct AdmissionPermit(Arc<AdmissionGate>);

impl Drop for AdmissionPermit {
    fn drop(&mut self) {
        self.0.active.fetch_sub(1, Ordering::AcqRel);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceServiceIdentity {
    pub key: String,
    pub workspace_root: String,
    pub source_root: String,
    pub service_dir: PathBuf,
}

impl WorkspaceServiceIdentity {
    pub fn new(context: &WorkspaceContext, source_root: &Path) -> Result<Self, String> {
        let workspace_root = normalize_path_identity(&context.workspace_root)?;
        let source_root = normalize_path_identity(source_root)?;
        let workspace_root = workspace_root.display().to_string();
        let source_root = source_root.display().to_string();
        let key = service_key(&workspace_root, &source_root);
        let service_dir = context.cache_root.join("services").join(&key);
        Ok(Self {
            key,
            workspace_root,
            source_root,
            service_dir,
        })
    }

    fn record_path(&self) -> PathBuf {
        self.service_dir.join("service.json")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceServiceRecord {
    pub schema_version: u32,
    pub pid: u32,
    pub port: u16,
    pub token: String,
    pub version: String,
    pub workspace_root: String,
    pub source_root: String,
    pub started_at: u64,
    pub last_access_at: u64,
}

impl WorkspaceServiceRecord {
    pub fn matches(&self, identity: &WorkspaceServiceIdentity, version: &str) -> bool {
        self.schema_version == SERVICE_SCHEMA_VERSION
            && self.version == version
            && self.workspace_root == identity.workspace_root
            && self.source_root == identity.source_root
            && !self.token.is_empty()
            && self.port > 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkspaceServiceConfig {
    pub idle_secs: u64,
    pub max_age_secs: u64,
}

impl WorkspaceServiceConfig {
    pub fn from_env() -> Self {
        Self {
            idle_secs: env_u64("UNICA_WORKSPACE_SERVICE_IDLE_SECS", DEFAULT_IDLE_SECS),
            max_age_secs: env_u64("UNICA_WORKSPACE_SERVICE_MAX_AGE_SECS", DEFAULT_MAX_AGE_SECS),
        }
    }
}

struct WorkspaceServiceCallDeadline {
    started: Instant,
    budget: Duration,
}

struct ElapsedBudgetDeadline {
    started: Instant,
    budget: Duration,
}

impl ElapsedBudgetDeadline {
    fn new(budget: Duration) -> Self {
        Self {
            started: Instant::now(),
            budget,
        }
    }

    fn remaining(&self) -> Option<Duration> {
        self.budget
            .checked_sub(self.started.elapsed())
            .filter(|remaining| !remaining.is_zero())
    }

    fn remaining_cancellable(&self, cancellation: &CancellationToken) -> Result<Duration, String> {
        cancellation_error(cancellation)?;
        self.remaining()
            .ok_or_else(|| workspace_service_request_timeout_error(self.budget))
    }
}

pub(crate) struct WorkspaceServiceBslCall<'a> {
    tool_name: &'a str,
    tool_args: Value,
    timeout: Duration,
    request_budget: Duration,
}

impl<'a> WorkspaceServiceBslCall<'a> {
    pub(crate) fn new(
        tool_name: &'a str,
        tool_args: Value,
        timeout: Duration,
        request_budget: Duration,
    ) -> Self {
        Self {
            tool_name,
            tool_args,
            timeout,
            request_budget,
        }
    }
}

impl WorkspaceServiceCallDeadline {
    fn new(budget: Duration) -> Self {
        Self {
            started: Instant::now(),
            budget,
        }
    }

    fn remaining(&self, cancellation: &CancellationToken) -> Result<Duration, String> {
        cancellation_error(cancellation)?;
        self.budget
            .checked_sub(self.started.elapsed())
            .filter(|remaining| !remaining.is_zero())
            .ok_or_else(|| workspace_service_request_timeout_error(self.budget))
    }
}

pub struct WorkspaceServiceManager<'a> {
    connector: &'a dyn ServiceConnector,
    spawner: &'a dyn ServiceSpawner,
    config: WorkspaceServiceConfig,
}

impl WorkspaceServiceManager<'_> {
    pub fn new() -> Self {
        Self {
            connector: &SYSTEM_SERVICE_CONNECTOR,
            spawner: &SYSTEM_SERVICE_SPAWNER,
            config: WorkspaceServiceConfig::from_env(),
        }
    }
}

impl<'a> WorkspaceServiceManager<'a> {
    #[cfg(test)]
    fn with_io(connector: &'a dyn ServiceConnector, spawner: &'a dyn ServiceSpawner) -> Self {
        Self {
            connector,
            spawner,
            config: WorkspaceServiceConfig::from_env(),
        }
    }

    #[allow(dead_code)]
    pub fn ensure_service(
        &self,
        context: &WorkspaceContext,
        source_root: &Path,
    ) -> Result<WorkspaceServiceRecord, String> {
        self.ensure_service_cancellable(context, source_root, &CancellationToken::new())
    }

    pub fn ensure_service_cancellable(
        &self,
        context: &WorkspaceContext,
        source_root: &Path,
        cancellation: &CancellationToken,
    ) -> Result<WorkspaceServiceRecord, String> {
        let deadline = WorkspaceServiceCallDeadline::new(SERVICE_REQUEST_TIMEOUT);
        self.ensure_service_cancellable_with_deadline(context, source_root, cancellation, &deadline)
    }

    fn ensure_service_cancellable_with_deadline(
        &self,
        context: &WorkspaceContext,
        source_root: &Path,
        cancellation: &CancellationToken,
        deadline: &WorkspaceServiceCallDeadline,
    ) -> Result<WorkspaceServiceRecord, String> {
        deadline.remaining(cancellation)?;
        let identity = WorkspaceServiceIdentity::new(context, source_root)?;
        loop {
            deadline.remaining(cancellation)?;
            if let Some(spawn_lock) = acquire_spawn_lock(&identity)? {
                if let Some(record) = self.reusable_record(&identity, cancellation, deadline)? {
                    return Ok(record);
                }
                let token = new_token(&identity);
                let spawn_budget = deadline.remaining(cancellation)?;
                let result =
                    self.spawner
                        .spawn(&identity, self.config, &token, cancellation, spawn_budget);
                drop(spawn_lock);
                deadline.remaining(cancellation)?;
                return result;
            }

            let remaining = deadline.remaining(cancellation)?;
            thread::sleep(Duration::from_millis(50).min(remaining));
        }
    }

    #[allow(dead_code)]
    pub fn call_bsl_mcp(
        &self,
        context: &WorkspaceContext,
        source_root: &Path,
        tool_name: &str,
        tool_args: Value,
        timeout: Duration,
    ) -> Result<WorkspaceServiceBslOutput, String> {
        self.call_bsl_mcp_cancellable(
            context,
            source_root,
            tool_name,
            tool_args,
            timeout,
            &CancellationToken::new(),
        )
    }

    pub fn call_bsl_mcp_cancellable(
        &self,
        context: &WorkspaceContext,
        source_root: &Path,
        tool_name: &str,
        tool_args: Value,
        timeout: Duration,
        cancellation: &CancellationToken,
    ) -> Result<WorkspaceServiceBslOutput, String> {
        self.call_bsl_mcp_cancellable_with_budget(
            context,
            source_root,
            WorkspaceServiceBslCall::new(tool_name, tool_args, timeout, SERVICE_REQUEST_TIMEOUT),
            cancellation,
        )
    }

    pub(crate) fn call_bsl_mcp_cancellable_with_budget(
        &self,
        context: &WorkspaceContext,
        source_root: &Path,
        call: WorkspaceServiceBslCall<'_>,
        cancellation: &CancellationToken,
    ) -> Result<WorkspaceServiceBslOutput, String> {
        let deadline = WorkspaceServiceCallDeadline::new(call.request_budget);
        let mut retried_transport = false;
        loop {
            let record = self.ensure_service_cancellable_with_deadline(
                context,
                source_root,
                cancellation,
                &deadline,
            )?;
            let send_budget = deadline.remaining(cancellation)?;
            let attempt_timeout = call.timeout.min(send_budget);
            let send_result = self.connector.send(
                &record,
                ServiceRequest {
                    token: record.token.clone(),
                    kind: ServiceRequestKind::BslMcp {
                        operation_id: Uuid::new_v4().to_string(),
                        tool_name: call.tool_name.to_string(),
                        tool_args: call.tool_args.clone(),
                        timeout_seconds: attempt_timeout.as_secs(),
                        timeout_nanos: attempt_timeout.subsec_nanos(),
                    },
                },
                cancellation,
                send_budget,
            );
            deadline.remaining(cancellation)?;
            let response = match send_result {
                Ok(response) => response,
                Err(error)
                    if !retried_transport
                        && is_retry_safe_bsl_mcp_tool(call.tool_name)
                        && is_retryable_workspace_service_transport_error(&error) =>
                {
                    retried_transport = true;
                    continue;
                }
                Err(error) => return Err(error),
            };
            if !response.ok {
                return Err(response
                    .error
                    .unwrap_or_else(|| "workspace service bsl request failed".to_string()));
            }
            return Ok(WorkspaceServiceBslOutput {
                result_text: response.result_text.unwrap_or_default(),
                stderr: response.stderr.unwrap_or_default(),
            });
        }
    }

    pub fn call_rlm_cancellable(
        &self,
        context: &WorkspaceContext,
        source_root: &Path,
        operation: WorkspaceRlmOperation,
        timeout: Duration,
        cancellation: &CancellationToken,
    ) -> Result<WorkspaceServiceRlmCall, String> {
        let deadline = WorkspaceServiceCallDeadline::new(timeout);
        let mut retried_transport = false;
        loop {
            let record = self.ensure_service_cancellable_with_deadline(
                context,
                source_root,
                cancellation,
                &deadline,
            )?;
            let send_budget = deadline.remaining(cancellation)?;
            let response = match self.connector.send(
                &record,
                ServiceRequest {
                    token: record.token.clone(),
                    kind: ServiceRequestKind::RlmMcp {
                        operation_id: Uuid::new_v4().to_string(),
                        operation: operation.clone(),
                        timeout_seconds: send_budget.as_secs(),
                        timeout_nanos: send_budget.subsec_nanos(),
                    },
                },
                cancellation,
                send_budget,
            ) {
                Ok(response) => response,
                Err(error)
                    if !retried_transport
                        && is_retryable_workspace_service_transport_error(&error) =>
                {
                    retried_transport = true;
                    continue;
                }
                Err(error) => return Err(error),
            };
            deadline.remaining(cancellation)?;
            if !response.ok {
                if response.index_status.is_some() {
                    return Ok(WorkspaceServiceRlmCall::Unready(response.index_readiness()));
                }
                return Err(response
                    .error
                    .unwrap_or_else(|| "workspace service RLM request failed".to_string()));
            }
            return Ok(WorkspaceServiceRlmCall::Output(WorkspaceServiceRlmOutput {
                result_text: response.result_text.unwrap_or_default(),
                stderr: response.stderr.unwrap_or_default(),
            }));
        }
    }

    #[allow(dead_code)]
    pub fn rlm_readiness(
        &self,
        context: &WorkspaceContext,
        source_root: &Path,
        args: &Map<String, Value>,
    ) -> Result<IndexReadiness, String> {
        self.rlm_readiness_cancellable(context, source_root, args, &CancellationToken::new())
    }

    pub fn rlm_readiness_cancellable(
        &self,
        context: &WorkspaceContext,
        source_root: &Path,
        args: &Map<String, Value>,
        cancellation: &CancellationToken,
    ) -> Result<IndexReadiness, String> {
        self.rlm_readiness_cancellable_with_timeout(
            context,
            source_root,
            args,
            SERVICE_REQUEST_TIMEOUT,
            cancellation,
        )
    }

    pub fn rlm_readiness_cancellable_with_timeout(
        &self,
        context: &WorkspaceContext,
        source_root: &Path,
        args: &Map<String, Value>,
        timeout: Duration,
        cancellation: &CancellationToken,
    ) -> Result<IndexReadiness, String> {
        let deadline = WorkspaceServiceCallDeadline::new(timeout);
        let record = self.ensure_service_cancellable_with_deadline(
            context,
            source_root,
            cancellation,
            &deadline,
        )?;
        let send_budget = deadline.remaining(cancellation)?;
        let response = self.connector.send(
            &record,
            ServiceRequest {
                token: record.token.clone(),
                kind: ServiceRequestKind::RlmReady {
                    operation_id: Uuid::new_v4().to_string(),
                    args: Value::Object(args.clone()),
                    timeout_seconds: send_budget.as_secs(),
                    timeout_nanos: send_budget.subsec_nanos(),
                },
            },
            cancellation,
            send_budget,
        )?;
        deadline.remaining(cancellation)?;
        if !response.ok {
            return Err(response
                .error
                .unwrap_or_else(|| "workspace service rlm request failed".to_string()));
        }
        Ok(response.index_readiness())
    }

    pub fn notify_invalidation(&self, context: &WorkspaceContext, events: &[DomainEvent]) {
        if events.is_empty() {
            return;
        }
        let services_dir = context.cache_root.join("services");
        let Ok(entries) = fs::read_dir(services_dir) else {
            return;
        };
        let typed_events = events.to_vec();
        let Ok(workspace_root) = normalize_path_identity(&context.workspace_root) else {
            return;
        };
        let workspace_root = workspace_root.display().to_string();
        for entry in entries.flatten() {
            let service_dir = entry.path();
            let lock_identity = WorkspaceServiceIdentity {
                key: String::new(),
                workspace_root: String::new(),
                source_root: String::new(),
                service_dir,
            };
            let Some(record) = read_record(&lock_identity) else {
                continue;
            };
            if record.workspace_root != workspace_root {
                continue;
            }
            let _ = self.connector.send(
                &record,
                ServiceRequest {
                    token: record.token.clone(),
                    kind: ServiceRequestKind::Invalidate {
                        events: typed_events.clone(),
                    },
                },
                &CancellationToken::new(),
                SERVICE_REQUEST_TIMEOUT,
            );
        }
    }

    fn service_is_alive(
        &self,
        record: &WorkspaceServiceRecord,
        cancellation: &CancellationToken,
        deadline: &WorkspaceServiceCallDeadline,
    ) -> Result<bool, String> {
        let request = ServiceRequest {
            token: record.token.clone(),
            kind: ServiceRequestKind::Ping,
        };
        let send_budget = deadline.remaining(cancellation)?;
        let result = self
            .connector
            .send(record, request, cancellation, send_budget)
            .map(|response| service_response_is_alive(&response))
            .unwrap_or(false);
        deadline.remaining(cancellation)?;
        Ok(result)
    }

    fn reusable_record(
        &self,
        identity: &WorkspaceServiceIdentity,
        cancellation: &CancellationToken,
        deadline: &WorkspaceServiceCallDeadline,
    ) -> Result<Option<WorkspaceServiceRecord>, String> {
        let Some(record) = read_record(identity) else {
            return Ok(None);
        };
        if record.matches(identity, env!("CARGO_PKG_VERSION"))
            && self.service_is_alive(&record, cancellation, deadline)?
        {
            return Ok(Some(record));
        }
        self.shutdown_record(&record, cancellation, deadline);
        Ok(None)
    }

    fn shutdown_record(
        &self,
        record: &WorkspaceServiceRecord,
        cancellation: &CancellationToken,
        deadline: &WorkspaceServiceCallDeadline,
    ) {
        if record.token.is_empty() || record.port == 0 {
            return;
        }
        let Ok(send_budget) = deadline.remaining(cancellation) else {
            return;
        };
        let _ = self.connector.send(
            record,
            ServiceRequest {
                token: record.token.clone(),
                kind: ServiceRequestKind::Shutdown,
            },
            cancellation,
            send_budget,
        );
    }
}

fn service_response_is_alive(response: &ServiceResponse) -> bool {
    response.ok && !response.shutdown && response.status.as_deref() == Some("alive")
}

fn is_retry_safe_bsl_mcp_tool(tool_name: &str) -> bool {
    matches!(tool_name, "diagnostics" | "graph")
}

fn duration_timeout_millis(duration: Duration) -> u64 {
    duration
        .as_nanos()
        .div_ceil(1_000_000)
        .min(u128::from(u64::MAX))
        .max(1) as u64
}

fn duration_from_timeout_parts(seconds: u64, nanos: u32) -> Result<Duration, String> {
    if nanos >= 1_000_000_000 {
        return Err(format!(
            "workspace service timeout has invalid nanoseconds: {nanos}"
        ));
    }
    if seconds > i64::MAX as u64 || (seconds == i64::MAX as u64 && nanos != 0) {
        return Err(format!(
            "workspace service timeout exceeds the positive i64-second configuration domain: maximum is {} seconds",
            i64::MAX
        ));
    }
    if seconds == 0 && nanos == 0 {
        return Err("workspace service timeout must be positive".to_string());
    }
    Ok(Duration::new(seconds, nanos))
}

fn is_retryable_workspace_service_transport_error(error: &str) -> bool {
    [
        "failed to connect workspace service:",
        "failed to set workspace service read timeout:",
        "failed to set workspace service write timeout:",
        "failed to write workspace service request:",
        "failed to flush workspace service request:",
        "workspace service disconnected before responding",
        "failed to read workspace service response:",
    ]
    .iter()
    .any(|prefix| error.starts_with(prefix))
}

fn cancellation_error(cancellation: &CancellationToken) -> Result<(), String> {
    if cancellation.is_cancelled() {
        Err(cancelled_error("workspace service operation stopped"))
    } else {
        Ok(())
    }
}

impl Default for WorkspaceServiceManager<'_> {
    fn default() -> Self {
        Self::new()
    }
}

trait ServiceConnector {
    fn send(
        &self,
        record: &WorkspaceServiceRecord,
        request: ServiceRequest,
        cancellation: &CancellationToken,
        budget: Duration,
    ) -> Result<ServiceResponse, String>;
}

trait ServiceSpawner {
    fn spawn(
        &self,
        identity: &WorkspaceServiceIdentity,
        config: WorkspaceServiceConfig,
        token: &str,
        cancellation: &CancellationToken,
        budget: Duration,
    ) -> Result<WorkspaceServiceRecord, String>;
}

struct SystemServiceConnector;
struct SystemServiceSpawner;

trait ConnectorClock {
    fn elapsed(&self) -> Duration;
}

struct SystemClock(Instant);

impl SystemClock {
    fn new() -> Self {
        Self(Instant::now())
    }
}

impl ConnectorClock for SystemClock {
    fn elapsed(&self) -> Duration {
        self.0.elapsed()
    }
}

struct Deadline {
    started: Duration,
    budget: Duration,
}

impl Deadline {
    fn new(clock: &dyn ConnectorClock, budget: Duration) -> Self {
        Self {
            started: clock.elapsed(),
            budget,
        }
    }

    fn remaining(&self, clock: &dyn ConnectorClock) -> Option<Duration> {
        self.budget
            .checked_sub(clock.elapsed().saturating_sub(self.started))
            .filter(|remaining| !remaining.is_zero())
    }
}

trait ConnectorStream: Read + Write {
    fn set_read_timeout(&self, timeout: Option<Duration>) -> io::Result<()>;
    fn set_write_timeout(&self, timeout: Option<Duration>) -> io::Result<()>;
}

impl ConnectorStream for TcpStream {
    fn set_read_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
        TcpStream::set_read_timeout(self, timeout)
    }

    fn set_write_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
        TcpStream::set_write_timeout(self, timeout)
    }
}

trait ConnectorIo {
    fn connect(&self, port: u16, timeout: Duration) -> io::Result<Box<dyn ConnectorStream>>;
}

struct SystemConnectorIo;

impl ConnectorIo for SystemConnectorIo {
    fn connect(&self, port: u16, timeout: Duration) -> io::Result<Box<dyn ConnectorStream>> {
        TcpStream::connect_timeout(&([127, 0, 0, 1], port).into(), timeout)
            .map(|stream| Box::new(stream) as Box<dyn ConnectorStream>)
    }
}

impl ServiceConnector for SystemServiceConnector {
    fn send(
        &self,
        record: &WorkspaceServiceRecord,
        request: ServiceRequest,
        cancellation: &CancellationToken,
        budget: Duration,
    ) -> Result<ServiceResponse, String> {
        let clock = SystemClock::new();
        self.send_with(
            record,
            request,
            cancellation,
            budget,
            &SystemConnectorIo,
            &clock,
        )
    }
}

impl SystemServiceConnector {
    fn send_with(
        &self,
        record: &WorkspaceServiceRecord,
        request: ServiceRequest,
        cancellation: &CancellationToken,
        budget: Duration,
        io: &dyn ConnectorIo,
        clock: &dyn ConnectorClock,
    ) -> Result<ServiceResponse, String> {
        let deadline = Deadline::new(clock, budget);
        let operation_id = request.kind.operation_id().map(str::to_owned);
        cancellation_error(cancellation)?;
        let connect_cap = if request.kind.is_control() {
            SERVICE_CONTROL_CONNECT_TIMEOUT
        } else {
            SERVICE_CONNECT_TIMEOUT
        };
        let connect_timeout = remaining_or_timeout(&deadline, clock)?.min(connect_cap);
        let mut stream = match io.connect(record.port, connect_timeout) {
            Ok(stream) => stream,
            Err(error) => {
                cancellation_error(cancellation)?;
                remaining_or_timeout(&deadline, clock)?;
                return Err(format!("failed to connect workspace service: {error}"));
            }
        };
        cancellation_error(cancellation)?;
        if let Err(error) = stream.set_read_timeout(Some(Duration::from_millis(100))) {
            cancellation_error(cancellation)?;
            remaining_or_timeout(&deadline, clock)?;
            return Err(format!(
                "failed to set workspace service read timeout: {error}"
            ));
        }
        let payload = serde_json::to_string(&request).map_err(|err| err.to_string())?;
        if let Err(error) = write_with_deadline(
            stream.as_mut(),
            payload.as_bytes(),
            &deadline,
            clock,
            cancellation,
        ) {
            self.cancel_after_error(&error, operation_id.as_deref(), record, io, clock);
            return Err(error);
        }
        if let Err(error) =
            write_with_deadline(stream.as_mut(), b"\n", &deadline, clock, cancellation)
        {
            self.cancel_after_error(&error, operation_id.as_deref(), record, io, clock);
            return Err(error);
        }
        if let Err(error) = flush_with_deadline(stream.as_mut(), &deadline, clock, cancellation) {
            self.cancel_after_error(&error, operation_id.as_deref(), record, io, clock);
            return Err(error);
        }
        let result = read_service_response(stream.as_mut(), &deadline, clock, cancellation);
        if let Err(error) = &result {
            self.cancel_after_error(error, operation_id.as_deref(), record, io, clock);
        }
        result
    }

    fn send_control_with(
        &self,
        record: &WorkspaceServiceRecord,
        kind: ServiceRequestKind,
        io: &dyn ConnectorIo,
        clock: &dyn ConnectorClock,
    ) -> Result<(), String> {
        let deadline = Deadline::new(clock, SERVICE_CONTROL_CONNECT_TIMEOUT);
        let connect_timeout = remaining_or_control_timeout(&deadline, clock)
            .map_err(|error| format!("workspace service control request failed: {error}"))?;
        let mut stream = io
            .connect(record.port, connect_timeout)
            .map_err(|err| format!("failed to connect workspace service control path: {err}"))?;
        let request = ServiceRequest {
            token: record.token.clone(),
            kind,
        };
        let payload = serde_json::to_string(&request).map_err(|err| err.to_string())?;
        write_control_with_deadline(stream.as_mut(), payload.as_bytes(), &deadline, clock)
            .map_err(|error| {
                format!("failed to write workspace service control request: {error}")
            })?;
        write_control_with_deadline(stream.as_mut(), b"\n", &deadline, clock).map_err(|error| {
            format!("failed to write workspace service control request: {error}")
        })?;
        let remaining = remaining_or_control_timeout(&deadline, clock)
            .map_err(|error| format!("workspace service control request failed: {error}"))?;
        if let Err(error) =
            stream.set_write_timeout(Some(remaining.min(Duration::from_millis(100))))
        {
            remaining_or_control_timeout(&deadline, clock).map_err(|timeout| {
                format!("workspace service control request failed: {timeout}")
            })?;
            return Err(format!(
                "failed to set workspace service control timeout: {error}"
            ));
        }
        if let Err(error) = stream.flush() {
            remaining_or_control_timeout(&deadline, clock).map_err(|timeout| {
                format!("workspace service control request failed: {timeout}")
            })?;
            return Err(format!(
                "failed to flush workspace service control request: {error}"
            ));
        }
        remaining_or_control_timeout(&deadline, clock)
            .map_err(|error| format!("workspace service control request failed: {error}"))?;
        Ok(())
    }

    fn cancel_after_error(
        &self,
        error: &str,
        operation_id: Option<&str>,
        record: &WorkspaceServiceRecord,
        io: &dyn ConnectorIo,
        clock: &dyn ConnectorClock,
    ) {
        if error.starts_with("cancelled:") {
            self.send_cancel(operation_id, record, io, clock);
        }
    }

    fn send_cancel(
        &self,
        operation_id: Option<&str>,
        record: &WorkspaceServiceRecord,
        io: &dyn ConnectorIo,
        clock: &dyn ConnectorClock,
    ) {
        if let Some(operation_id) = operation_id {
            let _ = self.send_control_with(
                record,
                ServiceRequestKind::Cancel {
                    operation_id: operation_id.to_string(),
                },
                io,
                clock,
            );
        }
    }
}

fn read_service_response(
    stream: &mut dyn ConnectorStream,
    deadline: &Deadline,
    clock: &dyn ConnectorClock,
    cancellation: &CancellationToken,
) -> Result<ServiceResponse, String> {
    let mut response = Vec::new();
    let mut chunk = [0_u8; 8192];
    loop {
        cancellation_error(cancellation)?;
        let remaining = remaining_or_timeout(deadline, clock)?;
        if let Err(error) = stream.set_read_timeout(Some(remaining.min(Duration::from_millis(100))))
        {
            cancellation_error(cancellation)?;
            remaining_or_timeout(deadline, clock)?;
            return Err(format!(
                "failed to set workspace service read timeout: {error}"
            ));
        }
        let read = stream.read(&mut chunk);
        cancellation_error(cancellation)?;
        remaining_or_timeout(deadline, clock)?;
        match read {
            Ok(0) => {
                return Err("workspace service disconnected before responding".to_string());
            }
            Ok(count) => {
                let previous_len = response.len();
                response.extend_from_slice(&chunk[..count]);
                if let Some(newline) = response[previous_len..]
                    .iter()
                    .position(|byte| *byte == b'\n')
                    .map(|offset| previous_len + offset)
                {
                    if newline > SERVICE_RESPONSE_LINE_LIMIT {
                        return Err(response_line_too_large());
                    }
                    return serde_json::from_slice(&response[..newline])
                        .map_err(|error| format!("invalid workspace service response: {error}"));
                }
                if response.len() > SERVICE_RESPONSE_LINE_LIMIT {
                    return Err(response_line_too_large());
                }
            }
            Err(error) if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
            Err(error) => {
                return Err(format!(
                    "failed to read workspace service response: {error}"
                ));
            }
        }
    }
}

fn response_line_too_large() -> String {
    format!(
        "invalid workspace service response: response line exceeds {SERVICE_RESPONSE_LINE_LIMIT} bytes"
    )
}

fn remaining_or_timeout(
    deadline: &Deadline,
    clock: &dyn ConnectorClock,
) -> Result<Duration, String> {
    deadline
        .remaining(clock)
        .ok_or_else(|| workspace_service_request_timeout_error(deadline.budget))
}

fn workspace_service_request_timeout_error(budget: Duration) -> String {
    let rendered_budget = if budget.subsec_nanos() == 0 {
        format!("{} seconds", budget.as_secs())
    } else {
        format!("{} milliseconds", duration_timeout_millis(budget))
    };
    format!("timeout: workspace service request exceeded {rendered_budget}")
}

fn remaining_or_control_timeout(
    deadline: &Deadline,
    clock: &dyn ConnectorClock,
) -> io::Result<Duration> {
    deadline.remaining(clock).ok_or_else(|| {
        io::Error::new(
            ErrorKind::TimedOut,
            "workspace service control request timed out",
        )
    })
}

fn write_with_deadline(
    stream: &mut dyn ConnectorStream,
    bytes: &[u8],
    deadline: &Deadline,
    clock: &dyn ConnectorClock,
    cancellation: &CancellationToken,
) -> Result<(), String> {
    let mut written = 0;
    while written < bytes.len() {
        cancellation_error(cancellation)?;
        let remaining = remaining_or_timeout(deadline, clock)?;
        if let Err(error) =
            stream.set_write_timeout(Some(remaining.min(Duration::from_millis(100))))
        {
            cancellation_error(cancellation)?;
            remaining_or_timeout(deadline, clock)?;
            return Err(format!(
                "failed to set workspace service write timeout: {error}"
            ));
        }
        let result = stream.write(&bytes[written..]);
        cancellation_error(cancellation)?;
        match result {
            Ok(0) => {
                remaining_or_timeout(deadline, clock)?;
                return Err(format!(
                    "failed to write workspace service request: {}",
                    io::Error::from(ErrorKind::WriteZero)
                ));
            }
            Ok(count) => {
                remaining_or_timeout(deadline, clock)?;
                written += count;
            }
            Err(error) if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                cancellation_error(cancellation)?;
                remaining_or_timeout(deadline, clock)?;
            }
            Err(error) => {
                cancellation_error(cancellation)?;
                remaining_or_timeout(deadline, clock)?;
                return Err(format!(
                    "failed to write workspace service request: {error}"
                ));
            }
        }
    }
    Ok(())
}

fn flush_with_deadline(
    stream: &mut dyn ConnectorStream,
    deadline: &Deadline,
    clock: &dyn ConnectorClock,
    cancellation: &CancellationToken,
) -> Result<(), String> {
    loop {
        cancellation_error(cancellation)?;
        let remaining = remaining_or_timeout(deadline, clock)?;
        if let Err(error) =
            stream.set_write_timeout(Some(remaining.min(Duration::from_millis(100))))
        {
            cancellation_error(cancellation)?;
            remaining_or_timeout(deadline, clock)?;
            return Err(format!(
                "failed to set workspace service write timeout: {error}"
            ));
        }
        match stream.flush() {
            Ok(()) => {
                cancellation_error(cancellation)?;
                remaining_or_timeout(deadline, clock)?;
                return Ok(());
            }
            Err(error) if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                cancellation_error(cancellation)?;
                remaining_or_timeout(deadline, clock)?;
            }
            Err(error) => {
                cancellation_error(cancellation)?;
                remaining_or_timeout(deadline, clock)?;
                return Err(format!(
                    "failed to flush workspace service request: {error}"
                ));
            }
        }
    }
}

fn write_control_with_deadline(
    stream: &mut dyn ConnectorStream,
    bytes: &[u8],
    deadline: &Deadline,
    clock: &dyn ConnectorClock,
) -> io::Result<()> {
    let mut written = 0;
    while written < bytes.len() {
        stream.set_write_timeout(Some(remaining_or_control_timeout(deadline, clock)?))?;
        match stream.write(&bytes[written..]) {
            Ok(0) => {
                remaining_or_control_timeout(deadline, clock)?;
                return Err(io::Error::from(ErrorKind::WriteZero));
            }
            Ok(count) => written += count,
            Err(error) => {
                remaining_or_control_timeout(deadline, clock)?;
                return Err(error);
            }
        }
    }
    Ok(())
}

impl ServiceSpawner for SystemServiceSpawner {
    fn spawn(
        &self,
        identity: &WorkspaceServiceIdentity,
        config: WorkspaceServiceConfig,
        token: &str,
        cancellation: &CancellationToken,
        budget: Duration,
    ) -> Result<WorkspaceServiceRecord, String> {
        let deadline = WorkspaceServiceCallDeadline::new(budget);
        deadline.remaining(cancellation)?;
        fs::create_dir_all(&identity.service_dir)
            .map_err(|err| format!("failed to create workspace service directory: {err}"))?;
        deadline.remaining(cancellation)?;
        let stdout = fs::File::create(identity.service_dir.join("service.stdout.log"))
            .map_err(|err| format!("failed to create workspace service stdout log: {err}"))?;
        deadline.remaining(cancellation)?;
        let stderr_path = identity.service_dir.join("service.stderr.log");
        let stderr = fs::File::create(&stderr_path)
            .map_err(|err| format!("failed to create workspace service stderr log: {err}"))?;
        deadline.remaining(cancellation)?;
        let exe = env::current_exe()
            .map_err(|err| format!("failed to locate current unica executable: {err}"))?;
        let mut command = Command::new(exe);
        command
            .arg("--workspace-service")
            .arg("--workspace-root")
            .arg(&identity.workspace_root)
            .arg("--source-root")
            .arg(&identity.source_root)
            .arg("--service-dir")
            .arg(identity.service_dir.display().to_string())
            .arg("--token")
            .arg(token)
            .arg("--idle-secs")
            .arg(config.idle_secs.to_string())
            .arg("--max-age-secs")
            .arg(config.max_age_secs.to_string())
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr));
        if let Some(plugin_root) = find_plugin_root(Path::new(&identity.workspace_root)) {
            command.env("UNICA_PLUGIN_ROOT", plugin_root);
        }
        let mut child = ManagedStartupChild::spawn_configured(command)
            .map_err(|err| format!("failed to spawn workspace service: {err}"))?;
        let child_pid = child.id();
        let readiness: Result<WorkspaceServiceRecord, String> = (|| {
            let wait_budget = deadline
                .remaining(cancellation)?
                .min(SERVICE_CONNECT_TIMEOUT);
            let record = wait_for_record_with_connector(
                identity,
                &SYSTEM_SERVICE_CONNECTOR,
                child_pid,
                token,
                wait_budget,
                cancellation,
            )?;
            deadline.remaining(cancellation)?;
            Ok(record)
        })();
        match readiness {
            Ok(record) => match child.detach() {
                Ok(()) => Ok(record),
                Err(error) => {
                    let cleanup = terminate_failed_workspace_service_spawn(
                        &mut child,
                        identity,
                        token,
                        SERVICE_SPAWN_CLEANUP_WAIT,
                    );
                    match cleanup {
                        Ok(()) => Err(error),
                        Err(cleanup_error) => Err(format!("{error}; {cleanup_error}")),
                    }
                }
            },
            Err(error) => {
                let startup_error =
                    workspace_service_startup_failure(&error, &mut child, &stderr_path);
                let cleanup = terminate_failed_workspace_service_spawn(
                    &mut child,
                    identity,
                    token,
                    SERVICE_SPAWN_CLEANUP_WAIT,
                );
                match cleanup {
                    Ok(()) => Err(startup_error),
                    Err(cleanup_error) => Err(format!("{startup_error}; {cleanup_error}")),
                }
            }
        }
    }
}

fn workspace_service_startup_failure(
    readiness_error: &str,
    child: &mut ManagedStartupChild,
    stderr_path: &Path,
) -> String {
    let pid = child.id();
    match child.try_wait_status() {
        Ok(Some(status)) => {
            let stderr = read_workspace_service_stderr_tail(stderr_path).unwrap_or_default();
            if stderr.trim().is_empty() {
                format!(
                    "{readiness_error}; spawned workspace service {pid} exited before readiness with {status}"
                )
            } else {
                format!(
                    "{readiness_error}; spawned workspace service {pid} exited before readiness with {status}; stderr tail: {}",
                    stderr.trim()
                )
            }
        }
        Ok(None) => format!(
            "{readiness_error}; spawned workspace service {pid} was still running when readiness failed"
        ),
        Err(error) => format!(
            "{readiness_error}; failed to inspect spawned workspace service {pid}: {error}"
        ),
    }
}

fn read_workspace_service_stderr_tail(path: &Path) -> Result<String, String> {
    let mut file = fs::File::open(path).map_err(|error| {
        format!(
            "failed to open workspace service stderr log {}: {error}",
            path.display()
        )
    })?;
    let length = file
        .metadata()
        .map_err(|error| format!("failed to inspect workspace service stderr log: {error}"))?
        .len();
    file.seek(SeekFrom::Start(
        length.saturating_sub(SERVICE_STARTUP_STDERR_TAIL_LIMIT),
    ))
    .map_err(|error| format!("failed to seek workspace service stderr log: {error}"))?;
    let mut bytes = Vec::new();
    file.take(SERVICE_STARTUP_STDERR_TAIL_LIMIT)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("failed to read workspace service stderr log: {error}"))?;
    let mut rendered = String::from_utf8_lossy(&bytes).into_owned();
    let limit = SERVICE_STARTUP_STDERR_TAIL_LIMIT as usize;
    if rendered.len() > limit {
        let mut start = rendered.len() - limit;
        while !rendered.is_char_boundary(start) {
            start += 1;
        }
        rendered.drain(..start);
    }
    Ok(rendered)
}

fn source_generation_with_deadline(
    source_root: &Path,
    deadline: &ElapsedBudgetDeadline,
    cancellation: &CancellationToken,
) -> Result<u64, String> {
    let generation = source_generation_until(source_root, &|| {
        cancellation.is_cancelled() || deadline.remaining().is_none()
    });
    match generation {
        Some(generation) => Ok(generation),
        None => {
            deadline.remaining_cancellable(cancellation)?;
            unreachable!("source generation only stops on cancellation or timeout")
        }
    }
}

fn terminate_failed_workspace_service_spawn(
    child: &mut ManagedStartupChild,
    identity: &WorkspaceServiceIdentity,
    token: &str,
    wait_limit: Duration,
) -> Result<(), String> {
    let pid = child.id();
    child
        .terminate_bounded(wait_limit)
        .map_err(|error| format!("failed to clean up spawned workspace service {pid}: {error}"))?;
    remove_spawned_service_record_if_owned(identity, pid, token)
}

fn remove_spawned_service_record_if_owned(
    identity: &WorkspaceServiceIdentity,
    pid: u32,
    token: &str,
) -> Result<(), String> {
    with_record_lock(identity, || {
        let Some(record) = read_record_unlocked(identity) else {
            return Ok(());
        };
        if record.pid != pid || record.token != token {
            return Ok(());
        }
        fs::remove_file(identity.record_path()).map_err(|error| {
            format!("failed to remove failed workspace service record: {error}")
        })?;
        Ok(())
    })
}

#[derive(Default)]
struct AnalyzerLane {
    state: Mutex<AnalyzerLaneState>,
    wake: Condvar,
}

#[derive(Default)]
struct AnalyzerLaneState {
    held: bool,
    next_ticket: u64,
    waiters: VecDeque<u64>,
}

impl AnalyzerLane {
    #[cfg(test)]
    fn acquire(&self, cancellation: &CancellationToken) -> Result<AnalyzerLanePermit<'_>, String> {
        self.acquire_with_hook_before(cancellation, None, || {})
    }

    fn acquire_before(
        &self,
        cancellation: &CancellationToken,
        deadline: &ElapsedBudgetDeadline,
    ) -> Result<AnalyzerLanePermit<'_>, String> {
        self.acquire_with_hook_before(cancellation, Some(deadline), || {})
    }

    #[cfg(test)]
    fn acquire_with_hook(
        &self,
        cancellation: &CancellationToken,
        queued: impl FnOnce(),
    ) -> Result<AnalyzerLanePermit<'_>, String> {
        self.acquire_with_hook_before(cancellation, None, queued)
    }

    fn acquire_with_hook_before(
        &self,
        cancellation: &CancellationToken,
        deadline: Option<&ElapsedBudgetDeadline>,
        queued: impl FnOnce(),
    ) -> Result<AnalyzerLanePermit<'_>, String> {
        let ticket = {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            let ticket = state.next_ticket;
            state.next_ticket = state.next_ticket.wrapping_add(1);
            state.waiters.push_back(ticket);
            ticket
        };
        queued();
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        loop {
            if cancellation.is_cancelled() {
                if let Some(index) = state.waiters.iter().position(|waiting| *waiting == ticket) {
                    state.waiters.remove(index);
                }
                self.wake.notify_all();
                return Err(cancelled_error("workspace analyzer lane wait stopped"));
            }
            if let Some(deadline) = deadline {
                if deadline.remaining().is_none() {
                    if let Some(index) = state.waiters.iter().position(|waiting| *waiting == ticket)
                    {
                        state.waiters.remove(index);
                    }
                    self.wake.notify_all();
                    return Err(workspace_service_request_timeout_error(deadline.budget));
                }
            }
            if !state.held && state.waiters.front() == Some(&ticket) {
                state.waiters.pop_front();
                state.held = true;
                return Ok(AnalyzerLanePermit { lane: self });
            }
            let wait_slice = deadline
                .and_then(ElapsedBudgetDeadline::remaining)
                .map(|remaining| remaining.min(Duration::from_millis(10)))
                .unwrap_or_else(|| Duration::from_millis(10));
            let (next, _) = self
                .wake
                .wait_timeout(state, wait_slice)
                .unwrap_or_else(|error| error.into_inner());
            state = next;
        }
    }
}

struct AnalyzerLanePermit<'a> {
    lane: &'a AnalyzerLane,
}

impl Drop for AnalyzerLanePermit<'_> {
    fn drop(&mut self) {
        let mut state = self
            .lane
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        state.held = false;
        self.lane.wake.notify_all();
    }
}

struct WorkspaceServiceRuntime {
    identity: WorkspaceServiceIdentity,
    token: String,
    record_owner: WorkspaceServiceRecord,
    context: WorkspaceContext,
    analyzer_lane: AnalyzerLane,
    analyzer: Mutex<Option<PersistentMcpSession>>,
    analyzer_starter: Arc<BslSessionStarter>,
    rlm_lane: AnalyzerLane,
    rlm: Mutex<Option<RlmMcpSession>>,
    rlm_starter: Arc<RlmSessionStarter>,
    rlm_maintenance_requester: Arc<RlmIndexMaintenanceRequester>,
    rlm_maintenance_pending: Arc<AtomicBool>,
    /// Index maintenance outlives the read that noticed the index was stale, so
    /// it cannot borrow that read's token: the operation is unregistered as soon
    /// as it answers, which would both hide the thread from `begin_shutdown` and
    /// let a late `cancel` for the finished read kill the scheduled update.
    rlm_maintenance_cancellation: CancellationToken,
    rlm_maintenance_tasks: Arc<SessionTeardownLifecycle>,
    session_teardowns: SessionTeardownLifecycle,
    analyzer_source_generation: Mutex<Option<u64>>,
    rlm_source_generation: Mutex<Option<u64>>,
    rlm_source_revisions: Mutex<Option<Arc<SourceRevisionService>>>,
    analyzer_invalidated: AtomicBool,
    rlm_invalidated: AtomicBool,
    operations: Mutex<HashMap<String, CancellationToken>>,
    shutting_down: AtomicBool,
    work_admission: Arc<AdmissionGate>,
    general_admission: Arc<AdmissionGate>,
    control_admission: Arc<AdmissionGate>,
    #[cfg(test)]
    handler_started_hook: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
    #[cfg(test)]
    provider_sessions_drained_hook: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
}

type BslSessionStarter = dyn Fn(&WorkspaceContext, &Path, &CancellationToken) -> Result<PersistentMcpSession, String>
    + Send
    + Sync;
type RlmSessionStarter = dyn Fn(&WorkspaceContext, &Path, &CancellationToken) -> Result<RlmMcpSession, String>
    + Send
    + Sync;
type RlmIndexMaintenanceRequester = dyn Fn(WorkspaceContext, PathBuf, SourceRevision, Arc<SourceRevisionService>, CancellationToken)
    + Send
    + Sync;

struct RlmMaintenanceRequestGuard(Arc<AtomicBool>);

impl Drop for RlmMaintenanceRequestGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

fn observe_source_generation(
    observed: &Mutex<Option<u64>>,
    invalidated: &AtomicBool,
    current: u64,
) -> bool {
    let Ok(mut observed) = observed.lock() else {
        return false;
    };
    let changed = observed
        .replace(current)
        .is_some_and(|previous| previous != current);
    invalidated.swap(false, Ordering::AcqRel) || changed
}

impl WorkspaceServiceRuntime {
    fn new(identity: WorkspaceServiceIdentity, record_owner: &WorkspaceServiceRecord) -> Self {
        let context = WorkspaceContext {
            cwd: PathBuf::from(&identity.workspace_root),
            workspace_root: PathBuf::from(&identity.workspace_root),
            cache_root: service_cache_root(&identity.service_dir),
            workspace_epoch: 0,
        };
        let rlm_maintenance_tasks = Arc::new(SessionTeardownLifecycle::default());
        let requester_tasks = Arc::clone(&rlm_maintenance_tasks);
        Self {
            identity,
            token: record_owner.token.clone(),
            record_owner: record_owner.clone(),
            context,
            analyzer_lane: AnalyzerLane::default(),
            analyzer: Mutex::new(None),
            analyzer_starter: Arc::new(PersistentMcpSession::start),
            rlm_lane: AnalyzerLane::default(),
            rlm: Mutex::new(None),
            rlm_starter: Arc::new(RlmMcpSession::start),
            rlm_maintenance_requester: Arc::new(
                move |context, source_root, revision, revision_service, cancellation| {
                    let mut args = Map::new();
                    args.insert(
                        "sourceDir".to_string(),
                        Value::String(source_root.display().to_string()),
                    );
                    args.insert(
                        SOURCE_REVISION_GENERATION_ARG.to_string(),
                        Value::from(revision.generation),
                    );
                    args.insert(
                        SOURCE_REVISION_ARG.to_string(),
                        serde_json::to_value(revision)
                            .expect("source revision is always serializable"),
                    );
                    let runner = SystemIndexRunner::tracked(requester_tasks.clone());
                    let _ = WorkspaceIndexService::with_runner(&runner)
                        .with_source_revision_service(revision_service)
                        .start_for_workspace_cancellable(&context, &args, false, &cancellation);
                },
            ),
            rlm_maintenance_pending: Arc::new(AtomicBool::new(false)),
            rlm_maintenance_cancellation: CancellationToken::new(),
            rlm_maintenance_tasks,
            session_teardowns: SessionTeardownLifecycle::default(),
            analyzer_source_generation: Mutex::new(None),
            rlm_source_generation: Mutex::new(None),
            rlm_source_revisions: Mutex::new(None),
            analyzer_invalidated: AtomicBool::new(false),
            rlm_invalidated: AtomicBool::new(false),
            operations: Mutex::new(HashMap::new()),
            shutting_down: AtomicBool::new(false),
            work_admission: AdmissionGate::new(SERVICE_MAX_WORKERS),
            general_admission: AdmissionGate::new(SERVICE_MAX_CONNECTION_HANDLERS),
            control_admission: AdmissionGate::new(SERVICE_MAX_CONTROL_HANDLERS),
            #[cfg(test)]
            handler_started_hook: Mutex::new(None),
            #[cfg(test)]
            provider_sessions_drained_hook: Mutex::new(None),
        }
    }

    #[cfg(test)]
    fn notify_handler_started(&self) {
        let hook = self.handler_started_hook.lock().unwrap().clone();
        if let Some(hook) = hook {
            hook();
        }
    }

    fn ping(&self) -> ServiceResponse {
        ServiceResponse {
            ok: true,
            status: Some(if self.shutting_down.load(Ordering::Acquire) {
                "shutting-down".to_string()
            } else {
                "alive".to_string()
            }),
            ..ServiceResponse::default()
        }
    }

    fn authenticate(&self, token: &str) -> Result<(), &'static str> {
        if token == self.token {
            Ok(())
        } else {
            Err("invalid workspace service token")
        }
    }

    fn owns_record(&self, record: &WorkspaceServiceRecord) -> bool {
        record.schema_version == self.record_owner.schema_version
            && record.pid == self.record_owner.pid
            && record.port == self.record_owner.port
            && record.token == self.record_owner.token
            && record.version == self.record_owner.version
            && record.workspace_root == self.record_owner.workspace_root
            && record.source_root == self.record_owner.source_root
            && record.started_at == self.record_owner.started_at
            && record.matches(&self.identity, &self.record_owner.version)
    }

    fn register_operation(
        self: &Arc<Self>,
        operation_id: String,
    ) -> Result<(CancellationToken, OperationGuard), String> {
        if self.shutting_down.load(Ordering::Acquire) {
            return Err("workspace service is shutting down".to_string());
        }
        let mut operations = self
            .operations
            .lock()
            .map_err(|_| "workspace service operation registry is unavailable".to_string())?;
        if self.shutting_down.load(Ordering::Acquire) {
            return Err("workspace service is shutting down".to_string());
        }
        if operations.contains_key(&operation_id) {
            return Err(format!(
                "workspace service operation id is already active: {operation_id}"
            ));
        }
        let cancellation = CancellationToken::new();
        operations.insert(operation_id.clone(), cancellation.clone());
        Ok((
            cancellation,
            OperationGuard {
                runtime: Arc::clone(self),
                operation_id,
            },
        ))
    }

    fn cancel_operation(&self, operation_id: &str) -> ServiceResponse {
        let cancellation = self
            .operations
            .lock()
            .ok()
            .and_then(|operations| operations.get(operation_id).cloned());
        if let Some(cancellation) = cancellation {
            cancellation.cancel();
        }
        ServiceResponse {
            ok: true,
            status: Some("cancel-requested".to_string()),
            ..ServiceResponse::default()
        }
    }

    fn begin_shutdown(&self) -> ServiceResponse {
        self.shutting_down.store(true, Ordering::Release);
        self.session_teardowns.close();
        self.rlm_maintenance_tasks.close();
        self.rlm_maintenance_cancellation.cancel();
        let cancellations = self
            .operations
            .lock()
            .map(|operations| operations.values().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        for cancellation in cancellations {
            cancellation.cancel();
        }
        ServiceResponse {
            ok: true,
            status: Some("shutdown".to_string()),
            shutdown: true,
            ..ServiceResponse::default()
        }
    }

    fn retire_session<T: Send + 'static>(&self, session: T) {
        self.session_teardowns
            .track(thread::spawn(move || drop(session)));
    }

    fn shutdown_provider_sessions(&self) -> bool {
        let analyzer = self
            .analyzer
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take();
        let rlm = self
            .rlm
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take();
        if let Some(session) = analyzer {
            self.retire_session(session);
        }
        if let Some(session) = rlm {
            self.retire_session(session);
        }
        let sessions_drained = self.session_teardowns.drain(SESSION_TEARDOWN_GRACE);
        let maintenance_drained = self.rlm_maintenance_tasks.drain(SESSION_TEARDOWN_GRACE);
        #[cfg(test)]
        if let Some(hook) = self.provider_sessions_drained_hook.lock().unwrap().clone() {
            hook();
        }
        sessions_drained && maintenance_drained
    }

    fn invalidate(&self, events: &[DomainEvent]) -> ServiceResponse {
        if events.iter().any(|event| invalidates_analyzer(event.kind)) {
            self.analyzer_invalidated.store(true, Ordering::Release);
            self.rlm_invalidated.store(true, Ordering::Release);
            if let Some(revisions) = self
                .rlm_source_revisions
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .as_ref()
            {
                revisions.mark_dirty();
            }
        }
        ServiceResponse {
            ok: true,
            status: Some("invalidated".to_string()),
            ..ServiceResponse::default()
        }
    }

    #[cfg(test)]
    fn acquire_analyzer_lane(
        &self,
        cancellation: &CancellationToken,
    ) -> Result<AnalyzerLanePermit<'_>, String> {
        self.analyzer_lane.acquire(cancellation)
    }

    #[cfg(test)]
    fn acquire_rlm_lane(
        &self,
        cancellation: &CancellationToken,
    ) -> Result<AnalyzerLanePermit<'_>, String> {
        self.rlm_lane.acquire(cancellation)
    }

    fn acquire_analyzer_lane_before(
        &self,
        cancellation: &CancellationToken,
        deadline: &ElapsedBudgetDeadline,
    ) -> Result<AnalyzerLanePermit<'_>, String> {
        self.analyzer_lane.acquire_before(cancellation, deadline)
    }

    fn acquire_rlm_lane_before(
        &self,
        cancellation: &CancellationToken,
        deadline: &ElapsedBudgetDeadline,
    ) -> Result<AnalyzerLanePermit<'_>, String> {
        self.rlm_lane.acquire_before(cancellation, deadline)
    }

    fn handle_bsl_mcp(
        &self,
        tool_name: &str,
        tool_args: Value,
        timeout_seconds: u64,
        timeout_nanos: u32,
        cancellation: &CancellationToken,
    ) -> ServiceResponse {
        if cancellation.is_cancelled() {
            return ServiceResponse::error(cancelled_error("workspace analyzer operation stopped"));
        }
        let timeout = match duration_from_timeout_parts(timeout_seconds, timeout_nanos) {
            Ok(timeout) => timeout,
            Err(error) => return ServiceResponse::error(error),
        };
        let deadline = ElapsedBudgetDeadline::new(timeout);
        let _lane = match self.acquire_analyzer_lane_before(cancellation, &deadline) {
            Ok(lane) => lane,
            Err(error) => return ServiceResponse::error(error),
        };
        if let Err(error) = deadline.remaining_cancellable(cancellation) {
            return ServiceResponse::error(error);
        }
        let mut analyzer = self
            .analyzer
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let mut stale_session = None;
        let current_generation = match source_generation_with_deadline(
            Path::new(&self.identity.source_root),
            &deadline,
            cancellation,
        ) {
            Ok(generation) => generation,
            Err(error) => return ServiceResponse::error(error),
        };
        if observe_source_generation(
            &self.analyzer_source_generation,
            &self.analyzer_invalidated,
            current_generation,
        ) {
            stale_session = analyzer.take();
        }
        if stale_session.is_none()
            && analyzer
                .as_mut()
                .is_some_and(|session| !session.is_reusable())
        {
            stale_session = analyzer.take();
        }
        if let Some(stale_session) = stale_session {
            drop(analyzer);
            self.retire_session(stale_session);
            analyzer = self
                .analyzer
                .lock()
                .unwrap_or_else(|error| error.into_inner());
        }
        let result = (|| {
            deadline.remaining_cancellable(cancellation)?;
            if analyzer.is_none() {
                *analyzer = Some((self.analyzer_starter)(
                    &self.context,
                    Path::new(&self.identity.source_root),
                    cancellation,
                )?);
            }
            let remaining = deadline.remaining_cancellable(cancellation)?;
            analyzer
                .as_mut()
                .expect("bsl session must exist after start")
                .call(tool_name, tool_args, remaining, cancellation)
        })();
        match result {
            Ok(output) => ServiceResponse {
                ok: true,
                result_text: Some(output.result_text),
                stderr: Some(output.stderr),
                ..ServiceResponse::default()
            },
            Err(error) => {
                let stale_session = analyzer.take();
                drop(analyzer);
                if let Some(stale_session) = stale_session {
                    self.retire_session(stale_session);
                }
                ServiceResponse::error(error)
            }
        }
    }

    fn handle_rlm_ready(
        &self,
        args: Value,
        timeout_seconds: u64,
        timeout_nanos: u32,
        cancellation: &CancellationToken,
    ) -> ServiceResponse {
        let timeout = match duration_from_timeout_parts(timeout_seconds, timeout_nanos) {
            Ok(timeout) => timeout,
            Err(error) => return ServiceResponse::error(error),
        };
        if cancellation.is_cancelled() {
            return ServiceResponse::error(cancelled_error(
                "rlm index operation stopped before work",
            ));
        }
        let mut args = args.as_object().cloned().unwrap_or_default();
        args.insert(
            "sourceDir".to_string(),
            Value::String(self.identity.source_root.clone()),
        );
        let revisions = match self.rlm_source_revision_service() {
            Ok(revisions) => revisions,
            Err(error) => return ServiceResponse::error(error),
        };
        let revision =
            match revisions.snapshot(ProviderDeadline::from_budget(timeout), cancellation) {
                Ok(revision) => revision,
                Err(error) => return ServiceResponse::error(error),
            };
        args.insert(
            SOURCE_REVISION_GENERATION_ARG.to_string(),
            Value::from(revision.generation),
        );
        args.insert(
            SOURCE_REVISION_ARG.to_string(),
            serde_json::to_value(&revision).expect("source revision is always serializable"),
        );
        let runner = SystemIndexRunner::tracked(self.rlm_maintenance_tasks.clone());
        let service = WorkspaceIndexService::with_runner(&runner)
            .with_source_revision_service(Arc::clone(&revisions));
        let start_report = service.start_for_workspace_cancellable(
            &self.context,
            &args,
            false,
            &self.rlm_maintenance_cancellation,
        );
        let readiness = service.ready_index_cancellable(&self.context, &args, cancellation);
        ServiceResponse::from_readiness(readiness, start_report.warnings)
    }

    fn rlm_source_revision_service(&self) -> Result<Arc<SourceRevisionService>, String> {
        let mut revisions = self
            .rlm_source_revisions
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(service) = revisions.as_ref() {
            return Ok(Arc::clone(service));
        }
        let service = Arc::new(SourceRevisionService::new(
            &self.context,
            Path::new(&self.identity.source_root),
        )?);
        *revisions = Some(Arc::clone(&service));
        Ok(service)
    }

    fn request_rlm_index_maintenance(
        &self,
        revision: SourceRevision,
        revision_service: Arc<SourceRevisionService>,
        cancellation: &CancellationToken,
    ) -> Vec<String> {
        if cancellation.is_cancelled() {
            return vec![cancelled_error("rlm index operation stopped before work")];
        }
        if self
            .rlm_maintenance_pending
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Vec::new();
        }
        let requester = Arc::clone(&self.rlm_maintenance_requester);
        let context = self.context.clone();
        let source_root = PathBuf::from(&self.identity.source_root);
        let cancellation = self.rlm_maintenance_cancellation.clone();
        let pending = Arc::clone(&self.rlm_maintenance_pending);
        match thread::Builder::new()
            .name("unica-rlm-index-maintenance".to_string())
            .spawn(move || {
                let _pending = RlmMaintenanceRequestGuard(pending);
                requester(
                    context,
                    source_root,
                    revision,
                    revision_service,
                    cancellation,
                );
            }) {
            Ok(handle) => {
                self.rlm_maintenance_tasks.track(handle);
                Vec::new()
            }
            Err(error) => {
                self.rlm_maintenance_pending.store(false, Ordering::Release);
                vec![format!(
                    "rlm index maintenance could not be scheduled: {error}"
                )]
            }
        }
    }

    fn handle_rlm_mcp(
        &self,
        operation: WorkspaceRlmOperation,
        timeout_seconds: u64,
        timeout_nanos: u32,
        cancellation: &CancellationToken,
    ) -> ServiceResponse {
        if cancellation.is_cancelled() {
            return ServiceResponse::error(cancelled_error("workspace RLM operation stopped"));
        }
        let timeout = match duration_from_timeout_parts(timeout_seconds, timeout_nanos) {
            Ok(timeout) => timeout,
            Err(error) => return ServiceResponse::error(error),
        };
        let deadline = ElapsedBudgetDeadline::new(timeout);
        let _lane = match self.acquire_rlm_lane_before(cancellation, &deadline) {
            Ok(lane) => lane,
            Err(error) => return ServiceResponse::error(error),
        };
        if let Err(error) = deadline.remaining_cancellable(cancellation) {
            return ServiceResponse::error(error);
        }
        let mut rlm = self.rlm.lock().unwrap_or_else(|error| error.into_inner());
        let mut stale_session = None;
        let source_root = Path::new(&self.identity.source_root);
        let revisions = match self.rlm_source_revision_service() {
            Ok(revisions) => revisions,
            Err(error) => return ServiceResponse::error(error),
        };
        let pre_execution_revision =
            match deadline
                .remaining_cancellable(cancellation)
                .and_then(|remaining| {
                    revisions.snapshot(ProviderDeadline::from_budget(remaining), cancellation)
                }) {
                Ok(revision) => revision,
                Err(error) => return ServiceResponse::error(error),
            };
        let pre_execution_generation = pre_execution_revision.generation;
        if observe_source_generation(
            &self.rlm_source_generation,
            &self.rlm_invalidated,
            pre_execution_generation,
        ) {
            stale_session = rlm.take();
        }
        if stale_session.is_none() && rlm.as_mut().is_some_and(|session| !session.is_reusable()) {
            stale_session = rlm.take();
        }
        if let Some(stale_session) = stale_session {
            drop(rlm);
            self.retire_session(stale_session);
            rlm = self.rlm.lock().unwrap_or_else(|error| error.into_inner());
        }
        let pre_execution_readiness =
            ready_index_for_source_revision(&self.context, source_root, &pre_execution_revision);
        if !matches!(pre_execution_readiness, IndexReadiness::Ready { .. }) {
            let stale_session = rlm.take();
            drop(rlm);
            if let Some(stale_session) = stale_session {
                self.retire_session(stale_session);
            }
            let warnings = self.request_rlm_index_maintenance(
                pre_execution_revision.clone(),
                Arc::clone(&revisions),
                cancellation,
            );
            return ServiceResponse::unavailable_rlm_execution(pre_execution_readiness, warnings);
        }
        let result = (|| {
            deadline.remaining_cancellable(cancellation)?;
            if rlm.is_none() {
                *rlm = Some((self.rlm_starter)(
                    &self.context,
                    Path::new(&self.identity.source_root),
                    cancellation,
                )?);
            }
            let remaining = deadline.remaining_cancellable(cancellation)?;
            rlm.as_mut()
                .expect("RLM session must exist after start")
                .execute(&operation, remaining, cancellation)
        })();
        match result {
            Ok(output) => {
                let post_execution_revision = match deadline
                    .remaining_cancellable(cancellation)
                    .and_then(|remaining| {
                        revisions.snapshot(ProviderDeadline::from_budget(remaining), cancellation)
                    }) {
                    Ok(revision) => revision,
                    Err(error) => return ServiceResponse::error(error),
                };
                // Deliberately re-checked against the generation this read was
                // admitted under, not the one observed just now: a background
                // job that landed a newer marker mid-execute discards this
                // output rather than blessing it. That costs a repeated read
                // and never returns an answer built on sources RLM did not see.
                let boundary_readiness = ready_index_for_source_revision(
                    &self.context,
                    source_root,
                    &pre_execution_revision,
                );
                let post_execution_readiness = match (
                    post_execution_revision == pre_execution_revision,
                    boundary_readiness,
                ) {
                    (false, IndexReadiness::Ready { .. }) => IndexReadiness::Stale {
                        status: SOURCE_GENERATION_STALE_STATUS.to_string(),
                    },
                    (_, readiness) => readiness,
                };
                if !matches!(post_execution_readiness, IndexReadiness::Ready { .. }) {
                    let stale_session = rlm.take();
                    drop(rlm);
                    if let Some(stale_session) = stale_session {
                        self.retire_session(stale_session);
                    }
                    let warnings = self.request_rlm_index_maintenance(
                        post_execution_revision,
                        Arc::clone(&revisions),
                        cancellation,
                    );
                    return ServiceResponse::unavailable_rlm_execution(
                        post_execution_readiness,
                        warnings,
                    );
                }
                ServiceResponse {
                    ok: true,
                    result_text: Some(output.result_text),
                    stderr: Some(output.stderr),
                    ..ServiceResponse::default()
                }
            }
            Err(error) => {
                let stale_session = if rlm.as_mut().is_some_and(|session| !session.is_reusable()) {
                    rlm.take()
                } else {
                    None
                };
                drop(rlm);
                if let Some(stale_session) = stale_session {
                    self.retire_session(stale_session);
                }
                ServiceResponse::error(error)
            }
        }
    }
}

struct SessionTeardownLifecycle {
    state: Mutex<TrackedTaskState>,
}

struct TrackedTaskState {
    accepting: bool,
    handles: Vec<thread::JoinHandle<()>>,
}

impl Default for SessionTeardownLifecycle {
    fn default() -> Self {
        Self {
            state: Mutex::new(TrackedTaskState {
                accepting: true,
                handles: Vec::new(),
            }),
        }
    }
}

impl SessionTeardownLifecycle {
    fn track(&self, handle: thread::JoinHandle<()>) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        Self::reap_finished(&mut state.handles);
        if state.accepting {
            state.handles.push(handle);
            return;
        }
        drop(state);
        let _ = handle.join();
    }

    fn close(&self) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.accepting = false;
    }

    fn drain(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            {
                let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
                Self::reap_finished(&mut state.handles);
                if state.handles.is_empty() {
                    return true;
                }
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return false;
            }
            thread::sleep(remaining.min(Duration::from_millis(10)));
        }
    }

    fn reap_finished(handles: &mut Vec<thread::JoinHandle<()>>) {
        let mut index = 0;
        while index < handles.len() {
            if handles[index].is_finished() {
                let handle = handles.swap_remove(index);
                let _ = handle.join();
            } else {
                index += 1;
            }
        }
    }
}

impl IndexBackgroundTaskTracker for SessionTeardownLifecycle {
    fn track(&self, handle: thread::JoinHandle<()>) {
        SessionTeardownLifecycle::track(self, handle);
    }
}

fn invalidates_analyzer(event: DomainEventKind) -> bool {
    matches!(
        event,
        DomainEventKind::ModuleChanged
            | DomainEventKind::SourceResourcesReplaced
            | DomainEventKind::SourceSetChanged
            | DomainEventKind::BuildCompleted
            | DomainEventKind::MetadataChanged
            | DomainEventKind::ConfigXmlChanged
            | DomainEventKind::CfeChanged
            | DomainEventKind::FormChanged
            | DomainEventKind::RoleChanged
            | DomainEventKind::DcsChanged
    )
}

fn remove_service_record_if_owned(runtime: &WorkspaceServiceRuntime) {
    remove_service_record_if_owned_with_hook(runtime, || {});
}

fn remove_service_record_if_owned_with_hook(
    runtime: &WorkspaceServiceRuntime,
    inside_critical_section: impl FnOnce(),
) {
    let _ = with_record_lock(&runtime.identity, || {
        let Some(record) = read_record_unlocked(&runtime.identity) else {
            return Ok(());
        };
        if !runtime.owns_record(&record) {
            return Ok(());
        }
        inside_critical_section();
        let Some(confirmed) = read_record_unlocked(&runtime.identity) else {
            return Ok(());
        };
        if record == confirmed && runtime.owns_record(&confirmed) {
            fs::remove_file(runtime.identity.record_path()).map_err(|error| {
                format!("failed to remove owned workspace service record: {error}")
            })?;
        }
        Ok(())
    });
}

fn update_service_record_last_access(runtime: &WorkspaceServiceRuntime, last_access_at: u64) {
    update_service_record_last_access_with_hook(runtime, last_access_at, || {});
}

fn update_service_record_last_access_with_hook(
    runtime: &WorkspaceServiceRuntime,
    last_access_at: u64,
    inside_critical_section: impl FnOnce(),
) {
    let _ = with_record_lock(&runtime.identity, || {
        let Some(mut record) = read_record_unlocked(&runtime.identity) else {
            return Ok(());
        };
        if !runtime.owns_record(&record) {
            return Ok(());
        }
        inside_critical_section();
        record.last_access_at = last_access_at;
        write_record_unlocked(&runtime.identity, &record)
    });
}

struct OperationGuard {
    runtime: Arc<WorkspaceServiceRuntime>,
    operation_id: String,
}

impl Drop for OperationGuard {
    fn drop(&mut self) {
        if let Ok(mut operations) = self.runtime.operations.lock() {
            operations.remove(&self.operation_id);
        }
    }
}

trait WorkspaceServiceOperationExecutor: Send + Sync {
    fn execute(
        &self,
        runtime: &WorkspaceServiceRuntime,
        kind: ServiceRequestKind,
        cancellation: &CancellationToken,
    ) -> ServiceResponse;
}

struct SystemWorkspaceServiceOperationExecutor;

impl WorkspaceServiceOperationExecutor for SystemWorkspaceServiceOperationExecutor {
    fn execute(
        &self,
        runtime: &WorkspaceServiceRuntime,
        kind: ServiceRequestKind,
        cancellation: &CancellationToken,
    ) -> ServiceResponse {
        match kind {
            ServiceRequestKind::BslMcp {
                tool_name,
                tool_args,
                timeout_seconds,
                timeout_nanos,
                ..
            } => runtime.handle_bsl_mcp(
                &tool_name,
                tool_args,
                timeout_seconds,
                timeout_nanos,
                cancellation,
            ),
            ServiceRequestKind::RlmReady {
                args,
                timeout_seconds,
                timeout_nanos,
                ..
            } => runtime.handle_rlm_ready(args, timeout_seconds, timeout_nanos, cancellation),
            ServiceRequestKind::RlmMcp {
                operation,
                timeout_seconds,
                timeout_nanos,
                ..
            } => runtime.handle_rlm_mcp(operation, timeout_seconds, timeout_nanos, cancellation),
            _ => ServiceResponse::error("workspace service received non-work operation"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ServiceRequest {
    token: String,
    kind: ServiceRequestKind,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "type")]
enum ServiceRequestKind {
    Ping,
    BslMcp {
        operation_id: String,
        tool_name: String,
        tool_args: Value,
        timeout_seconds: u64,
        timeout_nanos: u32,
    },
    RlmReady {
        operation_id: String,
        args: Value,
        timeout_seconds: u64,
        timeout_nanos: u32,
    },
    RlmMcp {
        operation_id: String,
        operation: WorkspaceRlmOperation,
        timeout_seconds: u64,
        timeout_nanos: u32,
    },
    Cancel {
        operation_id: String,
    },
    Invalidate {
        events: Vec<DomainEvent>,
    },
    Shutdown,
}

impl ServiceRequestKind {
    fn operation_id(&self) -> Option<&str> {
        match self {
            Self::BslMcp { operation_id, .. }
            | Self::RlmReady { operation_id, .. }
            | Self::RlmMcp { operation_id, .. } => Some(operation_id),
            _ => None,
        }
    }

    fn is_control(&self) -> bool {
        matches!(
            self,
            Self::Ping | Self::Invalidate { .. } | Self::Shutdown | Self::Cancel { .. }
        )
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct ServiceResponse {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    result_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stderr: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    warnings: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    index_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    db_path: Option<String>,
    #[serde(default)]
    shutdown: bool,
}

impl ServiceResponse {
    fn error(message: impl Into<String>) -> Self {
        Self {
            ok: false,
            error: Some(message.into()),
            ..Self::default()
        }
    }

    fn from_readiness(readiness: IndexReadiness, warnings: Vec<String>) -> Self {
        match readiness {
            IndexReadiness::Ready { db_path } => Self {
                ok: true,
                index_status: Some("ready".to_string()),
                db_path: Some(db_path.display().to_string()),
                warnings,
                ..Self::default()
            },
            IndexReadiness::Missing => Self {
                ok: true,
                index_status: Some("missing".to_string()),
                warnings,
                ..Self::default()
            },
            IndexReadiness::Stale { status } => Self {
                ok: true,
                index_status: Some("stale".to_string()),
                error: Some(status),
                warnings,
                ..Self::default()
            },
            IndexReadiness::Building => Self {
                ok: true,
                index_status: Some("building".to_string()),
                warnings,
                ..Self::default()
            },
            IndexReadiness::Failed(error) => Self {
                ok: true,
                index_status: Some("failed".to_string()),
                error: Some(error),
                warnings,
                ..Self::default()
            },
            IndexReadiness::Unavailable(error) => Self {
                ok: true,
                index_status: Some("unavailable".to_string()),
                error: Some(error),
                warnings,
                ..Self::default()
            },
        }
    }

    fn unavailable_rlm_execution(readiness: IndexReadiness, warnings: Vec<String>) -> Self {
        let mut response = Self::from_readiness(readiness, warnings);
        response.ok = false;
        if response.error.is_none() {
            response.error = Some(match response.index_status.as_deref() {
                Some("missing") => "rlm index is missing".to_string(),
                Some("building") => "rlm index building".to_string(),
                _ => "rlm index unavailable at execution boundary".to_string(),
            });
        }
        response.result_text = None;
        response.stderr = None;
        response
    }

    fn index_readiness(&self) -> IndexReadiness {
        match self.index_status.as_deref() {
            Some("ready") => self
                .db_path
                .as_ref()
                .map(|path| IndexReadiness::Ready {
                    db_path: PathBuf::from(path),
                })
                .unwrap_or_else(|| {
                    IndexReadiness::Unavailable(
                        "workspace service reported ready without db path".to_string(),
                    )
                }),
            Some("missing") => IndexReadiness::Missing,
            Some("stale") => IndexReadiness::Stale {
                status: self.error.clone().unwrap_or_else(|| "stale".to_string()),
            },
            Some("building") => IndexReadiness::Building,
            Some("failed") => IndexReadiness::Failed(self.error.clone().unwrap_or_default()),
            Some("unavailable") => {
                IndexReadiness::Unavailable(self.error.clone().unwrap_or_default())
            }
            other => IndexReadiness::Unavailable(format!(
                "workspace service reported unknown RLM status {:?}",
                other
            )),
        }
    }
}

#[derive(Debug, Clone)]
pub struct WorkspaceServiceBslOutput {
    pub result_text: String,
    pub stderr: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "operation")]
pub enum WorkspaceRlmOperation {
    Search {
        query: String,
        limit: usize,
    },
    Definition {
        name: String,
        module_hint: String,
        limit: usize,
    },
    Outline {
        path: String,
        include_methods: bool,
    },
    ObjectProfile {
        name: String,
        sections: Option<Vec<String>>,
        limit: usize,
    },
}

impl WorkspaceRlmOperation {
    fn query_hint(&self) -> &str {
        match self {
            Self::Search { query, .. } => query,
            Self::Definition { name, .. } | Self::ObjectProfile { name, .. } => name,
            Self::Outline { path, .. } => path,
        }
    }
}

#[derive(Debug, Clone)]
pub struct WorkspaceServiceRlmOutput {
    pub result_text: String,
    pub stderr: String,
}

#[derive(Debug, Clone)]
pub enum WorkspaceServiceRlmCall {
    Output(WorkspaceServiceRlmOutput),
    Unready(IndexReadiness),
}

struct PersistentMcpSession {
    child: ManagedChild,
    writer: mpsc::SyncSender<BslWriteRequest>,
    rx: mpsc::Receiver<BslReaderEvent>,
    stderr_tail: Arc<Mutex<BoundedByteTail>>,
    reader_terminal: Arc<AtomicBool>,
    next_id: i64,
    valid: bool,
}

struct BoundedByteTail {
    bytes: VecDeque<u8>,
    limit: usize,
}

impl BoundedByteTail {
    fn new(limit: usize) -> Self {
        Self {
            bytes: VecDeque::with_capacity(limit),
            limit,
        }
    }

    fn append(&mut self, bytes: &[u8]) {
        let bytes = if bytes.len() > self.limit {
            &bytes[bytes.len() - self.limit..]
        } else {
            bytes
        };
        let overflow = self
            .bytes
            .len()
            .saturating_add(bytes.len())
            .saturating_sub(self.limit);
        self.bytes.drain(..overflow.min(self.bytes.len()));
        self.bytes.extend(bytes);
    }

    fn snapshot(&self) -> String {
        let bytes = self.bytes.iter().copied().collect::<Vec<_>>();
        let text = String::from_utf8_lossy(&bytes).into_owned();
        if text.len() <= self.limit {
            return text;
        }
        let mut start = text.len() - self.limit;
        while !text.is_char_boundary(start) {
            start += 1;
        }
        text[start..].to_string()
    }
}

struct BslWriteRequest {
    payload: Value,
    result: mpsc::SyncSender<Result<(), String>>,
}

enum BslReaderEvent {
    Message(Value),
    ProtocolError(String),
    Closed,
}

impl PersistentMcpSession {
    fn start(
        context: &WorkspaceContext,
        source_root: &Path,
        cancellation: &CancellationToken,
    ) -> Result<Self, String> {
        if cancellation.is_cancelled() {
            return Err(cancelled_error("persistent MCP start stopped"));
        }
        let plugin_root = find_plugin_root(&context.cwd).ok_or_else(|| {
            "could not locate Unica plugin root for workspace bsl-analyzer service".to_string()
        })?;
        let program = resolve_bundled_tool(&plugin_root, "bsl-analyzer", true)?.program;
        let source_arg = source_root.display().to_string();
        let mut command = Command::new(&program);
        command
            .args([
                "mcp",
                "serve",
                "--profile",
                "workspace",
                "--source-dir",
                &source_arg,
                "--mode",
                "stdio",
            ])
            .current_dir(&context.cwd);
        configure_bsl_analyzer_cache_dir(&mut command, context, source_root)?;
        configure_bsl_analyzer_runtime_dir(&mut command)?;
        Self::start_with_command(command, cancellation)
    }

    fn start_rlm_transport(
        context: &WorkspaceContext,
        cancellation: &CancellationToken,
    ) -> Result<Self, String> {
        if cancellation.is_cancelled() {
            return Err(cancelled_error("persistent RLM MCP start stopped"));
        }
        let plugin_root = find_plugin_root(&context.cwd).ok_or_else(|| {
            "could not locate Unica plugin root for workspace RLM service".to_string()
        })?;
        let program = resolve_bundled_tool(&plugin_root, "rlm-tools-bsl", true)?.program;
        let mut command = Command::new(program);
        command
            .current_dir(&context.cwd)
            .env("RLM_INDEX_DIR", context.cache_root.join("rlm-tools-bsl"));
        Self::start_with_command(command, cancellation)
    }

    fn start_with_command(
        command: Command,
        cancellation: &CancellationToken,
    ) -> Result<Self, String> {
        if cancellation.is_cancelled() {
            return Err(cancelled_error("persistent MCP start stopped"));
        }
        let mut child = ManagedChild::spawn_process(command, None, cancellation.clone())
            .map_err(|err| format!("failed to start persistent MCP transport: {err}"))?;
        let stdin = child
            .take_stdin()
            .ok_or_else(|| "failed to open persistent MCP stdin".to_string())?;
        let stdout = child
            .take_stdout()
            .ok_or_else(|| "failed to open persistent MCP stdout".to_string())?;
        let stderr = child
            .take_stderr()
            .ok_or_else(|| "failed to open persistent MCP stderr".to_string())?;

        let (writer, writer_rx) = mpsc::sync_channel::<BslWriteRequest>(1);
        thread::spawn(move || bsl_writer(stdin, writer_rx));
        let (tx, rx) = mpsc::sync_channel::<BslReaderEvent>(8);
        let reader_terminal = Arc::new(AtomicBool::new(false));
        let stdout_state = Arc::clone(&reader_terminal);
        thread::spawn(move || bsl_reader_with_state(stdout, tx, stdout_state));
        let stderr_tail = Arc::new(Mutex::new(BoundedByteTail::new(BSL_STDERR_TAIL_LIMIT)));
        let stderr_target = Arc::clone(&stderr_tail);
        thread::spawn(move || bsl_stderr_reader(stderr, stderr_target));

        let mut session = Self {
            child,
            writer,
            rx,
            stderr_tail,
            reader_terminal,
            next_id: 2,
            valid: true,
        };
        session.initialize(cancellation)?;
        Ok(session)
    }

    fn initialize(&mut self, cancellation: &CancellationToken) -> Result<(), String> {
        if cancellation.is_cancelled() {
            return Err(cancelled_error("persistent MCP initialize stopped"));
        }
        let deadline = ElapsedBudgetDeadline::new(SERVICE_REQUEST_TIMEOUT);
        self.send_json_cancellable(
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {
                    "protocolVersion": "2025-03-26",
                    "capabilities": {},
                    "clientInfo": {
                        "name": "unica",
                        "version": env!("CARGO_PKG_VERSION")
                    }
                }
            }),
            &deadline,
            cancellation,
        )?;
        let _ = self.read_response(1, &deadline, cancellation)?;
        if cancellation.is_cancelled() {
            return Err(cancelled_error("persistent MCP initialize stopped"));
        }
        self.send_json_cancellable(
            json!({
                "jsonrpc": "2.0",
                "method": "notifications/initialized"
            }),
            &deadline,
            cancellation,
        )?;
        if cancellation.is_cancelled() {
            return Err(cancelled_error("persistent MCP initialize stopped"));
        }
        Ok(())
    }

    fn call(
        &mut self,
        tool_name: &str,
        tool_args: Value,
        timeout: Duration,
        cancellation: &CancellationToken,
    ) -> Result<WorkspaceServiceBslOutput, String> {
        if cancellation.is_cancelled() {
            return Err(cancelled_error("persistent MCP request stopped"));
        }
        let id = self.next_id;
        self.next_id += 1;
        let deadline = ElapsedBudgetDeadline::new(timeout);
        self.send_json_cancellable(
            json!({
                "jsonrpc": "2.0",
                "id": id,
                "method": "tools/call",
                "params": {
                    "name": tool_name,
                    "arguments": tool_args
                }
            }),
            &deadline,
            cancellation,
        )?;
        let response = match self.read_response(id, &deadline, cancellation) {
            Ok(response) => response,
            Err(error) => {
                return Err(error);
            }
        };
        if cancellation.is_cancelled() {
            return self.fail(cancelled_error("persistent MCP request stopped"));
        }
        let result_text = match mcp_tool_text(&response) {
            Ok(text) => text,
            Err(error) => {
                if cancellation.is_cancelled() {
                    return self.fail(cancelled_error("persistent MCP request stopped"));
                }
                return self.fail(error);
            }
        };
        if cancellation.is_cancelled() {
            return self.fail(cancelled_error("persistent MCP request stopped"));
        }
        let stderr = self
            .stderr_tail
            .lock()
            .map(|tail| tail.snapshot())
            .unwrap_or_default();
        Ok(WorkspaceServiceBslOutput {
            result_text,
            stderr,
        })
    }

    fn is_reusable(&mut self) -> bool {
        self.valid
            && !self.reader_terminal.load(Ordering::Acquire)
            && self.child.is_running().unwrap_or(false)
    }

    fn send_json_cancellable(
        &mut self,
        payload: Value,
        deadline: &ElapsedBudgetDeadline,
        cancellation: &CancellationToken,
    ) -> Result<(), String> {
        let (result, result_rx) = mpsc::sync_channel(1);
        let mut request = BslWriteRequest { payload, result };
        loop {
            if cancellation.is_cancelled() {
                return self.fail(cancelled_error("persistent MCP write stopped"));
            }
            if deadline.remaining().is_none() {
                return self.fail("timeout: persistent MCP write timed out".to_string());
            }
            match self.writer.try_send(request) {
                Ok(()) => break,
                Err(mpsc::TrySendError::Full(returned)) => request = returned,
                Err(mpsc::TrySendError::Disconnected(_)) => {
                    return self.fail("persistent MCP stdin writer stopped".to_string());
                }
            }
            thread::sleep(Duration::from_millis(10));
        }
        loop {
            if cancellation.is_cancelled() {
                return self.fail(cancelled_error("persistent MCP write stopped"));
            }
            let Some(remaining) = deadline.remaining() else {
                return self.fail("timeout: persistent MCP write timed out".to_string());
            };
            let received = result_rx.recv_timeout(remaining.min(Duration::from_millis(25)));
            if cancellation.is_cancelled() {
                return self.fail(cancelled_error("persistent MCP write stopped"));
            }
            if deadline.remaining().is_none() {
                return self.fail("timeout: persistent MCP write timed out".to_string());
            }
            match received {
                Ok(Ok(())) => {
                    return Ok(());
                }
                Ok(Err(error)) => {
                    return self.fail(error);
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return self.fail("persistent MCP stdin writer stopped".to_string());
                }
            }
        }
    }

    fn read_response(
        &mut self,
        id: i64,
        deadline: &ElapsedBudgetDeadline,
        cancellation: &CancellationToken,
    ) -> Result<Value, String> {
        match read_json_response_cancellable(&self.rx, id, deadline, cancellation) {
            Ok(value) => Ok(value),
            Err(error) => self.fail(error),
        }
    }

    fn fail<T>(&mut self, error: String) -> Result<T, String> {
        let stderr = self
            .stderr_tail
            .lock()
            .map(|tail| tail.snapshot())
            .unwrap_or_default();
        self.invalidate();
        if stderr.is_empty() {
            Err(error)
        } else {
            Err(format!("{error}; stderr tail: {stderr}"))
        }
    }

    fn invalidate(&mut self) {
        if self.valid {
            self.valid = false;
            let _ = self.child.terminate();
        }
    }
}

impl Drop for PersistentMcpSession {
    fn drop(&mut self) {
        self.invalidate();
    }
}

/// bsl-analyzer otherwise defaults its cache below the indexed source tree.
/// The resulting filesystem events keep invalidating the index that produced
/// them. Prefer the existing service identity, but move the analyzer cache to
/// the private runtime root when the configured Unica cache is itself below
/// the selected source root (for example `sourceDir = "."`).
fn configure_bsl_analyzer_cache_dir(
    command: &mut Command,
    context: &WorkspaceContext,
    source_root: &Path,
) -> Result<(), String> {
    let identity = WorkspaceServiceIdentity::new(context, source_root)?;
    let preferred = identity.service_dir.join("bsl-analyzer");
    let source_root = PathBuf::from(&identity.source_root);
    let preferred_identity = normalize_path_identity(&preferred)?;
    let cache_dir = if crate::infrastructure::platform::filesystem::path_starts_with_host_root(
        &preferred_identity,
        &source_root,
    ) {
        let external_root = short_private_runtime_dir()
            .map_err(|error| {
                format!("failed to prepare external bsl-analyzer cache directory: {error}")
            })?
            .unwrap_or_else(|| std::env::temp_dir().join("unica-bsl"));
        let external = external_root.join("cache").join(&identity.key);
        let external_identity = normalize_path_identity(&external)?;
        if crate::infrastructure::platform::filesystem::path_starts_with_host_root(
            &external_identity,
            &source_root,
        ) {
            return Err(
                "failed to place bsl-analyzer cache outside the indexed source tree".to_string(),
            );
        }
        external
    } else {
        preferred
    };
    command.arg("--cache-dir").arg(cache_dir);
    Ok(())
}

/// A persistent session outlives the tool call that started it, by design
/// (ADR-0006). While its working directory sits inside the source tree, the
/// process holds that tree open, and `git worktree remove` fails long after the
/// call returned (#204). The tree is addressed by `--source-dir` as an absolute
/// path, so the working directory carries no information — it only holds.
///
/// The child therefore runs from the short private runtime directory it already
/// uses for its socket, outside any workspace.
fn configure_bsl_analyzer_runtime_dir(command: &mut Command) -> Result<(), String> {
    if let Some(runtime_dir) = short_private_runtime_dir().map_err(|error| {
        format!("failed to prepare short runtime directory for bsl-analyzer: {error}")
    })? {
        command.env("XDG_RUNTIME_DIR", &runtime_dir);
        command.current_dir(&runtime_dir);
    }
    Ok(())
}

struct RlmMcpSession {
    transport: PersistentMcpSession,
    source_root: PathBuf,
    session_id: Option<String>,
}

impl RlmMcpSession {
    fn start(
        context: &WorkspaceContext,
        source_root: &Path,
        cancellation: &CancellationToken,
    ) -> Result<Self, String> {
        Ok(Self {
            transport: PersistentMcpSession::start_rlm_transport(context, cancellation)?,
            source_root: source_root.to_path_buf(),
            session_id: None,
        })
    }

    fn is_reusable(&mut self) -> bool {
        self.transport.is_reusable()
    }

    fn execute(
        &mut self,
        operation: &WorkspaceRlmOperation,
        timeout: Duration,
        cancellation: &CancellationToken,
    ) -> Result<WorkspaceServiceRlmOutput, String> {
        let deadline = ElapsedBudgetDeadline::new(timeout);
        for attempt in 0..=RLM_LOGICAL_SESSION_RETRY_LIMIT {
            match self.ensure_logical_session(operation.query_hint(), &deadline, cancellation) {
                Ok(()) => {}
                Err(error)
                    if attempt < RLM_LOGICAL_SESSION_RETRY_LIMIT
                        && is_retryable_rlm_session_error(&error) =>
                {
                    self.close_logical_session(&deadline, cancellation);
                    continue;
                }
                Err(error) => return Err(error),
            }
            match self.execute_once(operation, &deadline, cancellation) {
                Ok(output) => return Ok(output),
                Err(error)
                    if attempt < RLM_LOGICAL_SESSION_RETRY_LIMIT
                        && is_retryable_rlm_session_error(&error) =>
                {
                    self.close_logical_session(&deadline, cancellation);
                }
                Err(error) => {
                    self.close_logical_session(&deadline, cancellation);
                    return Err(error);
                }
            }
        }
        Err("persistent RLM session retry exhausted".to_string())
    }

    fn ensure_logical_session(
        &mut self,
        query_hint: &str,
        deadline: &ElapsedBudgetDeadline,
        cancellation: &CancellationToken,
    ) -> Result<(), String> {
        if self.session_id.is_some() {
            return Ok(());
        }
        let timeout = remaining_rlm_time(deadline, cancellation)?;
        let output = self.transport.call(
            "rlm_start",
            rlm_start_arguments(&self.source_root, query_hint, timeout)?,
            timeout,
            cancellation,
        )?;
        let envelope: Value = serde_json::from_str(output.result_text.trim())
            .map_err(|error| format!("invalid rlm_start JSON response: {error}"))?;
        if let Some(error) = envelope.get("error").and_then(Value::as_str) {
            return Err(format!("rlm_start failed: {error}"));
        }
        let session_id = envelope
            .get("session_id")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "rlm_start response is missing session_id".to_string())?
            .to_string();
        self.session_id = Some(session_id);
        let index_status = envelope
            .pointer("/index/index_status")
            .and_then(Value::as_str);
        if matches!(index_status, Some("missing" | "incomplete")) {
            let error = format!(
                "rlm_start reported unavailable index status {}",
                index_status.unwrap_or("unknown")
            );
            self.close_logical_session(deadline, cancellation);
            return Err(error);
        }
        Ok(())
    }

    fn execute_once(
        &mut self,
        operation: &WorkspaceRlmOperation,
        deadline: &ElapsedBudgetDeadline,
        cancellation: &CancellationToken,
    ) -> Result<WorkspaceServiceRlmOutput, String> {
        let session_id = self
            .session_id
            .as_deref()
            .ok_or_else(|| "persistent RLM logical session is missing".to_string())?;
        let code = fixed_rlm_helper_code(operation)?;
        let timeout = remaining_rlm_time(deadline, cancellation)?;
        let output = self.transport.call(
            "rlm_execute",
            json!({
                "session_id": session_id,
                "code": code,
                "detail_level": "compact"
            }),
            timeout,
            cancellation,
        )?;
        let envelope: Value = serde_json::from_str(output.result_text.trim())
            .map_err(|error| format!("invalid rlm_execute JSON response: {error}"))?;
        if let Some(error) = envelope
            .get("error")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
        {
            return Err(format!("rlm_execute failed: {error}"));
        }
        let stdout = envelope
            .get("stdout")
            .and_then(Value::as_str)
            .ok_or_else(|| "rlm_execute response is missing stdout".to_string())?
            .trim();
        serde_json::from_str::<Value>(stdout)
            .map_err(|error| format!("rlm_execute stdout is not helper JSON: {error}"))?;
        Ok(WorkspaceServiceRlmOutput {
            result_text: stdout.to_string(),
            stderr: output.stderr,
        })
    }

    fn close_logical_session(
        &mut self,
        deadline: &ElapsedBudgetDeadline,
        cancellation: &CancellationToken,
    ) {
        let Some(session_id) = self.session_id.take() else {
            return;
        };
        let Ok(timeout) = remaining_rlm_time(deadline, cancellation) else {
            self.transport.invalidate();
            return;
        };
        let _ = self.transport.call(
            "rlm_end",
            json!({"session_id": session_id}),
            timeout.min(Duration::from_secs(2)),
            cancellation,
        );
    }
}

fn duration_seconds_ceil(duration: Duration) -> Result<u64, String> {
    duration
        .as_secs()
        .checked_add(u64::from(duration.subsec_nanos() != 0))
        .ok_or_else(|| {
            "workspace service timeout cannot be rounded up to whole seconds".to_string()
        })
}

fn rlm_start_arguments(
    source_root: &Path,
    query: &str,
    timeout: Duration,
) -> Result<Value, String> {
    let execution_timeout_seconds = duration_seconds_ceil(timeout)?;
    Ok(json!({
        "path": source_root.display().to_string(),
        "query": query,
        "effort": "low",
        "max_output_chars": 100_000,
        "max_execute_calls": 10_000,
        "execution_timeout_seconds": execution_timeout_seconds,
        "include_metadata": false
    }))
}

impl Drop for RlmMcpSession {
    fn drop(&mut self) {
        let deadline = ElapsedBudgetDeadline::new(Duration::from_secs(2));
        self.close_logical_session(&deadline, &CancellationToken::new());
    }
}

fn fixed_rlm_helper_code(operation: &WorkspaceRlmOperation) -> Result<String, String> {
    let serialized = serde_json::to_string(operation)
        .map_err(|error| format!("failed to encode RLM operation: {error}"))?;
    let data_literal = serde_json::to_string(&serialized)
        .map_err(|error| format!("failed to encode RLM operation literal: {error}"))?;
    let body = match operation {
        WorkspaceRlmOperation::Search { .. } => {
            "_result = search(_args[\"query\"], scope=\"all\", limit=_args[\"limit\"])\nprint(json.dumps(_result, ensure_ascii=False))"
        }
        WorkspaceRlmOperation::Definition { .. } => {
            "_result = find_definition(_args[\"name\"], module_hint=_args[\"module_hint\"], limit=_args[\"limit\"])\nprint(json.dumps(_result, ensure_ascii=False))"
        }
        WorkspaceRlmOperation::Outline { .. } => {
            "_result = get_module_outline(_args[\"path\"], include_methods=_args[\"include_methods\"])\nprint(json.dumps(_result, ensure_ascii=False))"
        }
        // `find_predefined` is the only supported way to reach predefined items and
        // it has neither a count nor a category argument. When the cross-category
        // fetch is capped, post-filtering cannot establish a complete answer for
        // the requested category, so the section fails closed instead of claiming
        // an authoritative `ok` or `empty`.
        WorkspaceRlmOperation::ObjectProfile { .. } => {
            "_raw_sections = _args[\"sections\"]\n_section_aliases = {\"functionalOptions\": \"functional_options\"}\n_profile_sections = None if _raw_sections is None else [_section_aliases.get(_section, _section) for _section in _raw_sections if _section != \"predefinedItems\"]\n_result = get_object_profile(_args[\"name\"], sections=_profile_sections, include_flow=False, include_code_usages=False, limit=_args[\"limit\"])\nif _raw_sections is not None and \"predefinedItems\" in _raw_sections and \"error\" not in _result:\n    _category = _result.get(\"category\")\n    _fetched = find_predefined(object_name=_result.get(\"object_name\") or _args[\"name\"], limit=_args[\"limit\"] + 1)\n    _has_more = len(_fetched) > _args[\"limit\"]\n    _items = _fetched\n    if _category:\n        _items = [_item for _item in _items if not _item.get(\"category\") or _item.get(\"category\") == _category]\n    _items = _items[:_args[\"limit\"]]\n    if _category and _has_more:\n        _predefined = {\"status\": \"unavailable\", \"summary\": {}, \"items\": [], \"total\": 0, \"returned\": 0, \"has_more\": True, \"_meta\": {\"source\": \"index\", \"truncated\": True, \"error\": \"supported RLM API cannot establish category-complete predefined items after a capped cross-category fetch\"}}\n    else:\n        _predefined = {\"status\": \"ok\" if _items else \"empty\", \"summary\": {}, \"items\": _items, \"total\": len(_items), \"returned\": len(_items), \"has_more\": _has_more, \"_meta\": {\"source\": \"index\", \"truncated\": _has_more, \"total_is_lower_bound\": _has_more}}\n    _result.setdefault(\"sections\", {})[\"predefined_items\"] = _predefined\nprint(json.dumps(_result, ensure_ascii=False))"
        }
    };
    Ok(format!(
        "import json\n_args = json.loads({data_literal})\n{body}"
    ))
}

fn remaining_rlm_time(
    deadline: &ElapsedBudgetDeadline,
    cancellation: &CancellationToken,
) -> Result<Duration, String> {
    if cancellation.is_cancelled() {
        return Err(cancelled_error("persistent RLM request stopped"));
    }
    deadline
        .remaining()
        .ok_or_else(|| "timeout: persistent RLM request timed out".to_string())
}

fn is_retryable_rlm_session_error(error: &str) -> bool {
    [
        "not found or expired",
        "Sandbox not found",
        "was closed",
        "Execution call limit exceeded",
    ]
    .iter()
    .any(|marker| error.contains(marker))
}

fn env_u64(name: &str, default: u64) -> u64 {
    env::var(name)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(default)
}

struct ServiceRecordLock {
    file: fs::File,
}

impl Drop for ServiceRecordLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.file);
    }
}

fn acquire_record_lock(identity: &WorkspaceServiceIdentity) -> Result<ServiceRecordLock, String> {
    fs::create_dir_all(&identity.service_dir)
        .map_err(|error| format!("failed to create workspace service state directory: {error}"))?;
    let path = identity.service_dir.join(SERVICE_RECORD_LOCK_FILE);
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&path)
        .map_err(|error| {
            format!(
                "failed to open workspace service record lock {}: {error}",
                path.display()
            )
        })?;
    file.lock_exclusive().map_err(|error| {
        format!(
            "failed to acquire workspace service record lock {}: {error}",
            path.display()
        )
    })?;
    Ok(ServiceRecordLock { file })
}

fn with_record_lock<T>(
    identity: &WorkspaceServiceIdentity,
    operation: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    let _lock = acquire_record_lock(identity)?;
    operation()
}

fn read_record_unlocked(identity: &WorkspaceServiceIdentity) -> Option<WorkspaceServiceRecord> {
    let text = fs::read_to_string(identity.record_path()).ok()?;
    serde_json::from_str(&text).ok()
}

fn read_record(identity: &WorkspaceServiceIdentity) -> Option<WorkspaceServiceRecord> {
    with_record_lock(identity, || Ok(read_record_unlocked(identity)))
        .ok()
        .flatten()
}

fn write_record_unlocked(
    identity: &WorkspaceServiceIdentity,
    record: &WorkspaceServiceRecord,
) -> Result<(), String> {
    let text = serde_json::to_string_pretty(record).map_err(|err| err.to_string())?;
    fs::write(identity.record_path(), text + "\n")
        .map_err(|err| format!("failed to write workspace service record: {err}"))
}

fn write_record(
    identity: &WorkspaceServiceIdentity,
    record: &WorkspaceServiceRecord,
) -> Result<(), String> {
    write_record_with_hook(identity, record, || {})
}

fn write_record_with_hook(
    identity: &WorkspaceServiceIdentity,
    record: &WorkspaceServiceRecord,
    inside_critical_section: impl FnOnce(),
) -> Result<(), String> {
    with_record_lock(identity, || {
        inside_critical_section();
        write_record_unlocked(identity, record)
    })
}

struct SpawnLock {
    file: fs::File,
}

impl Drop for SpawnLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.file);
    }
}

fn spawn_lock_path(identity: &WorkspaceServiceIdentity) -> PathBuf {
    identity.service_dir.join("service.lock")
}

fn acquire_spawn_lock(identity: &WorkspaceServiceIdentity) -> Result<Option<SpawnLock>, String> {
    fs::create_dir_all(&identity.service_dir)
        .map_err(|err| format!("failed to create workspace service lock directory: {err}"))?;
    let path = spawn_lock_path(identity);
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&path)
        .map_err(|error| {
            format!(
                "failed to acquire workspace service spawn lock {}: {error}",
                path.display()
            )
        })?;
    match file.try_lock_exclusive() {
        Ok(()) => {
            file.set_len(0)
                .map_err(|err| format!("failed to reset workspace service spawn lock: {err}"))?;
            let payload = format!("pid={}\nstarted_at={}\n", std::process::id(), now_secs());
            file.write_all(payload.as_bytes())
                .map_err(|err| format!("failed to write workspace service spawn lock: {err}"))?;
            Ok(Some(SpawnLock { file }))
        }
        Err(error) if spawn_lock_is_contended(&error) => Ok(None),
        Err(error) => Err(format!(
            "failed to lock workspace service spawn lock {}: {error}",
            path.display()
        )),
    }
}

fn spawn_lock_is_contended(error: &io::Error) -> bool {
    let expected = fs2::lock_contended_error();
    error.kind() == ErrorKind::WouldBlock
        || error
            .raw_os_error()
            .zip(expected.raw_os_error())
            .is_some_and(|(actual, expected)| actual == expected)
}

fn wait_for_record_with_connector(
    identity: &WorkspaceServiceIdentity,
    connector: &dyn ServiceConnector,
    expected_pid: u32,
    expected_token: &str,
    timeout: Duration,
    cancellation: &CancellationToken,
) -> Result<WorkspaceServiceRecord, String> {
    let started = Instant::now();
    while started.elapsed() < timeout {
        cancellation_error(cancellation)?;
        let Some(remaining) = timeout
            .checked_sub(started.elapsed())
            .filter(|remaining| !remaining.is_zero())
        else {
            break;
        };
        if let Some(record) = read_record(identity) {
            if record.pid == expected_pid
                && record.token == expected_token
                && record.matches(identity, env!("CARGO_PKG_VERSION"))
                && connector
                    .send(
                        &record,
                        ServiceRequest {
                            token: record.token.clone(),
                            kind: ServiceRequestKind::Ping,
                        },
                        cancellation,
                        remaining,
                    )
                    .map(|response| service_response_is_alive(&response))
                    .unwrap_or(false)
            {
                return Ok(record);
            }
            cancellation_error(cancellation)?;
        }
        thread::sleep(Duration::from_millis(50).min(remaining));
    }
    cancellation_error(cancellation)?;
    Err(format!(
        "workspace service did not become ready at {}",
        identity.record_path().display()
    ))
}

fn new_token(identity: &WorkspaceServiceIdentity) -> String {
    let mut hasher = DefaultHasher::new();
    identity.key.hash(&mut hasher);
    std::process::id().hash(&mut hasher);
    now_secs().hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}

fn service_cache_root(service_dir: &Path) -> PathBuf {
    service_dir
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .unwrap_or_else(|| service_dir.to_path_buf())
}

pub fn run_workspace_service_from_args(args: &[String]) -> Result<(), String> {
    let workspace_root = required_arg(args, "--workspace-root")?;
    let source_root = required_arg(args, "--source-root")?;
    let service_dir = PathBuf::from(required_arg(args, "--service-dir")?);
    let token = required_arg(args, "--token")?;
    let idle_secs = optional_u64_arg(args, "--idle-secs", DEFAULT_IDLE_SECS);
    let max_age_secs = optional_u64_arg(args, "--max-age-secs", DEFAULT_MAX_AGE_SECS);
    let key = service_dir
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| "workspace service dir must end with service key".to_string())?
        .to_string();
    let identity = WorkspaceServiceIdentity {
        key,
        workspace_root,
        source_root,
        service_dir,
    };
    run_workspace_service(identity, token, idle_secs, max_age_secs)
}

fn run_workspace_service(
    identity: WorkspaceServiceIdentity,
    token: String,
    idle_secs: u64,
    max_age_secs: u64,
) -> Result<(), String> {
    fs::create_dir_all(&identity.service_dir)
        .map_err(|err| format!("failed to create workspace service directory: {err}"))?;
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .map_err(|err| format!("failed to bind workspace service listener: {err}"))?;
    listener
        .set_nonblocking(true)
        .map_err(|err| format!("failed to configure workspace service listener: {err}"))?;
    let port = listener
        .local_addr()
        .map_err(|err| format!("failed to read workspace service listener address: {err}"))?
        .port();
    let record = WorkspaceServiceRecord {
        schema_version: SERVICE_SCHEMA_VERSION,
        pid: std::process::id(),
        port,
        token: token.clone(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        workspace_root: identity.workspace_root.clone(),
        source_root: identity.source_root.clone(),
        started_at: now_secs(),
        last_access_at: now_secs(),
    };
    write_record(&identity, &record)?;

    let runtime = Arc::new(WorkspaceServiceRuntime::new(identity, &record));
    serve_workspace_service(
        listener,
        runtime,
        Arc::new(SystemWorkspaceServiceOperationExecutor),
        Duration::from_secs(idle_secs.max(1)),
        Duration::from_secs(max_age_secs.max(1)),
        SERVICE_REQUEST_HEADER_TIMEOUT,
        SERVICE_SHUTDOWN_GRACE,
    )
}

struct PendingControlConnection {
    stream: TcpStream,
    accepted_at: Instant,
    bytes: Vec<u8>,
}

enum PendingControlPoll {
    Pending,
    Request(ServiceRequest),
    Invalid(String),
    Closed,
}

impl PendingControlConnection {
    fn poll(&mut self) -> PendingControlPoll {
        if self.accepted_at.elapsed() >= SERVICE_CONTROL_CONNECT_TIMEOUT {
            return PendingControlPoll::Closed;
        }
        let mut chunk = [0_u8; 8192];
        match self.stream.read(&mut chunk) {
            Ok(0) => PendingControlPoll::Closed,
            Ok(count) => {
                self.bytes.extend_from_slice(&chunk[..count]);
                if self.bytes.len() > SERVICE_CONTROL_CLASSIFICATION_LIMIT {
                    return PendingControlPoll::Invalid(
                        "workspace service overloaded: control classification line is too large"
                            .into(),
                    );
                }
                let Some(newline) = self.bytes.iter().position(|byte| *byte == b'\n') else {
                    return PendingControlPoll::Pending;
                };
                match serde_json::from_slice::<ServiceRequest>(&self.bytes[..newline]) {
                    Ok(request) => PendingControlPoll::Request(request),
                    Err(error) => PendingControlPoll::Invalid(format!(
                        "invalid workspace service request: {error}"
                    )),
                }
            }
            Err(error) if error.kind() == ErrorKind::WouldBlock => PendingControlPoll::Pending,
            Err(_) => PendingControlPoll::Closed,
        }
    }
}

fn serve_workspace_service(
    listener: TcpListener,
    runtime: Arc<WorkspaceServiceRuntime>,
    executor: Arc<dyn WorkspaceServiceOperationExecutor>,
    idle_timeout: Duration,
    max_age: Duration,
    request_header_timeout: Duration,
    shutdown_grace: Duration,
) -> Result<(), String> {
    let started = Instant::now();
    let mut last_access = Instant::now();
    let mut handlers: Vec<thread::JoinHandle<Result<(), String>>> = Vec::new();
    let mut pending_control = Vec::<PendingControlConnection>::new();
    let mut result = Ok(());
    loop {
        let mut index = 0;
        while index < handlers.len() {
            if handlers[index].is_finished() {
                let handler = handlers.swap_remove(index);
                report_workspace_service_handler_result(handler.join());
            } else {
                index += 1;
            }
        }
        let mut index = 0;
        while index < pending_control.len() {
            match pending_control[index].poll() {
                PendingControlPoll::Pending => index += 1,
                outcome => {
                    let pending = pending_control.swap_remove(index);
                    match outcome {
                        PendingControlPoll::Request(request) if request.kind.is_control() => {
                            if let Some(permit) = runtime.control_admission.try_acquire() {
                                let response_timeout = SERVICE_CONTROL_CONNECT_TIMEOUT
                                    .saturating_sub(pending.accepted_at.elapsed());
                                let handler_runtime = Arc::clone(&runtime);
                                let handler_executor = Arc::clone(&executor);
                                match thread::Builder::new().name("unica-workspace-control".into()).spawn(move || {
                                    let _permit = permit;
                                    pending.stream.set_nonblocking(false).map_err(|error| format!("failed to restore workspace control stream: {error}"))?;
                                    pending.stream.set_write_timeout(Some(response_timeout)).map_err(|error| format!("failed to set workspace control response timeout: {error}"))?;
                                    handle_workspace_service_request(pending.stream, handler_runtime, handler_executor, request)
                                }) {
                                    Ok(handler) => handlers.push(handler),
                                    Err(error) => { result = Err(format!("workspace service control handler spawn failed: {error}")); break; }
                                }
                            } else {
                                let _ = pending.stream.shutdown(std::net::Shutdown::Both);
                            }
                        }
                        PendingControlPoll::Request(_) => {
                            let _ = pending.stream.set_nonblocking(false);
                            let _ = pending
                                .stream
                                .set_write_timeout(Some(Duration::from_millis(100)));
                            let _ = write_service_response(pending.stream, &ServiceResponse::error("workspace service overloaded: general connection handlers are saturated"), true);
                        }
                        PendingControlPoll::Invalid(error) => {
                            let _ = pending.stream.set_nonblocking(false);
                            let _ = pending
                                .stream
                                .set_write_timeout(Some(Duration::from_millis(100)));
                            let _ = write_service_response(
                                pending.stream,
                                &ServiceResponse::error(error),
                                true,
                            );
                        }
                        PendingControlPoll::Closed | PendingControlPoll::Pending => {}
                    }
                }
            }
        }
        if result.is_err() {
            break;
        }
        if runtime.shutting_down.load(Ordering::Acquire) {
            break;
        }
        if started.elapsed() >= max_age || last_access.elapsed() >= idle_timeout {
            runtime.begin_shutdown();
            break;
        }
        match listener.accept() {
            Ok((stream, _addr)) => {
                last_access = Instant::now();
                update_service_record_last_access(&runtime, now_secs());
                let Some(handler_permit) = runtime.general_admission.try_acquire() else {
                    if pending_control.len() >= SERVICE_MAX_PENDING_CONTROL {
                        let evicted = pending_control.remove(0);
                        let _ = evicted.stream.shutdown(std::net::Shutdown::Both);
                    }
                    if stream.set_nonblocking(true).is_ok() {
                        pending_control.push(PendingControlConnection {
                            stream,
                            accepted_at: Instant::now(),
                            bytes: Vec::new(),
                        });
                    }
                    continue;
                };
                let header_clock = SystemClock::new();
                let header_deadline = Deadline::new(&header_clock, request_header_timeout);
                let handler_runtime = Arc::clone(&runtime);
                let handler_executor = Arc::clone(&executor);
                match thread::Builder::new()
                    .name("unica-workspace-connection".into())
                    .spawn(move || {
                        let _permit = handler_permit;
                        #[cfg(test)]
                        handler_runtime.notify_handler_started();
                        handle_workspace_service_stream(
                            stream,
                            handler_runtime,
                            handler_executor,
                            header_clock,
                            header_deadline,
                        )
                    }) {
                    Ok(handler) => handlers.push(handler),
                    Err(error) => {
                        result = Err(format!("workspace service handler spawn failed: {error}"));
                        break;
                    }
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(10));
            }
            Err(error) => {
                result = Err(format!("workspace service accept failed: {error}"));
                break;
            }
        }
    }

    runtime.begin_shutdown();
    if !runtime.shutdown_provider_sessions() && result.is_ok() {
        result = Err(format!(
            "workspace provider sessions or maintenance tasks did not stop within {} ms",
            SESSION_TEARDOWN_GRACE.as_millis()
        ));
    }
    for pending in pending_control.drain(..) {
        let _ = pending.stream.shutdown(std::net::Shutdown::Both);
    }
    let drain_deadline = Instant::now() + shutdown_grace;
    while !handlers.is_empty() && Instant::now() < drain_deadline {
        let mut index = 0;
        while index < handlers.len() {
            if handlers[index].is_finished() {
                let handler = handlers.swap_remove(index);
                report_workspace_service_handler_result(handler.join());
            } else {
                index += 1;
            }
        }
        if !handlers.is_empty() {
            thread::sleep(Duration::from_millis(10));
        }
    }
    remove_service_record_if_owned(&runtime);
    result
}

fn report_workspace_service_handler_result(result: thread::Result<Result<(), String>>) {
    let stderr = io::stderr();
    report_workspace_service_handler_result_to(&mut stderr.lock(), result);
}

fn report_workspace_service_handler_result_to(
    writer: &mut dyn Write,
    result: thread::Result<Result<(), String>>,
) {
    if let Some(diagnostic) = workspace_service_handler_diagnostic(result) {
        let _ = writeln!(writer, "{diagnostic}");
    }
}

fn workspace_service_handler_diagnostic(
    result: thread::Result<Result<(), String>>,
) -> Option<String> {
    match result {
        Ok(Ok(())) => None,
        Ok(Err(error)) => Some(format!("workspace service connection failed: {error}")),
        Err(_) => Some("workspace service connection handler panicked".to_string()),
    }
}

fn handle_workspace_service_stream(
    stream: TcpStream,
    runtime: Arc<WorkspaceServiceRuntime>,
    executor: Arc<dyn WorkspaceServiceOperationExecutor>,
    header_clock: SystemClock,
    header_deadline: Deadline,
) -> Result<(), String> {
    stream
        .set_write_timeout(Some(SERVICE_REQUEST_TIMEOUT))
        .map_err(|err| format!("failed to set workspace service response write timeout: {err}"))?;
    let header_stream = stream
        .try_clone()
        .map_err(|err| format!("failed to clone workspace service stream: {err}"))?;
    let mut reader = BufReader::new(
        header_stream
            .try_clone()
            .map_err(|err| format!("failed to clone workspace service stream: {err}"))?,
    );
    let line = match read_bounded_service_line_with_deadline(
        &mut reader,
        &header_deadline,
        &header_clock,
        |remaining| header_stream.set_read_timeout(Some(remaining)),
    )
    .map_err(|err| format!("failed to read workspace service request: {err}"))?
    {
        Some(BoundedServiceLine::Line(line)) => line,
        Some(BoundedServiceLine::TooLarge) => {
            write_service_response(stream, &ServiceResponse::error(format!("invalid workspace service request: request line exceeds {SERVICE_REQUEST_LINE_LIMIT} bytes")), false)?;
            return Ok(());
        }
        None => return Ok(()),
    };
    let request = match serde_json::from_slice::<ServiceRequest>(&line) {
        Ok(request) => request,
        Err(error) => {
            let response =
                ServiceResponse::error(format!("invalid workspace service request: {error}"));
            write_service_response(stream, &response, false)?;
            return Ok(());
        }
    };
    handle_workspace_service_request(stream, runtime, executor, request)
}

fn handle_workspace_service_request(
    stream: TcpStream,
    runtime: Arc<WorkspaceServiceRuntime>,
    executor: Arc<dyn WorkspaceServiceOperationExecutor>,
    request: ServiceRequest,
) -> Result<(), String> {
    if let Err(error) = runtime.authenticate(&request.token) {
        write_service_response(stream, &ServiceResponse::error(error), false)?;
        return Ok(());
    }

    match request.kind {
        ServiceRequestKind::Ping => {
            write_service_response(stream, &runtime.ping(), false)?;
        }
        ServiceRequestKind::Cancel { operation_id } => {
            write_service_response(stream, &runtime.cancel_operation(&operation_id), true)?;
        }
        ServiceRequestKind::Invalidate { events } => {
            write_service_response(stream, &runtime.invalidate(&events), false)?;
        }
        ServiceRequestKind::Shutdown => {
            write_service_response(stream, &runtime.begin_shutdown(), false)?;
        }
        kind @ (ServiceRequestKind::BslMcp { .. }
        | ServiceRequestKind::RlmReady { .. }
        | ServiceRequestKind::RlmMcp { .. }) => {
            let Some(worker_permit) = runtime.work_admission.try_acquire() else {
                write_service_response(stream, &ServiceResponse::error(format!("workspace service overloaded: at most {SERVICE_MAX_WORKERS} concurrent work requests are allowed")), false)?;
                return Ok(());
            };
            let operation_id = kind
                .operation_id()
                .expect("work request must carry operation id")
                .to_string();
            let (cancellation, guard) = match runtime.register_operation(operation_id) {
                Ok(registered) => registered,
                Err(error) => {
                    write_service_response(stream, &ServiceResponse::error(error), false)?;
                    return Ok(());
                }
            };
            let (result_tx, result_rx) = mpsc::sync_channel(1);
            let worker_runtime = Arc::clone(&runtime);
            let worker_cancellation = cancellation.clone();
            let _ = thread::Builder::new()
                .name("unica-workspace-worker".into())
                .spawn(move || {
                    let _permit = worker_permit;
                    let response = executor.execute(&worker_runtime, kind, &worker_cancellation);
                    drop(guard);
                    let _ = result_tx.send(response);
                });
            if let Err(error) = stream.set_nonblocking(true) {
                cancellation.cancel();
                return Err(format!(
                    "failed to monitor workspace service caller: {error}"
                ));
            }
            let mut caller_connected = true;
            loop {
                match result_rx.recv_timeout(Duration::from_millis(50)) {
                    Ok(response) => {
                        if caller_connected {
                            stream.set_nonblocking(false).map_err(|err| {
                                format!(
                                    "failed to restore workspace service response stream: {err}"
                                )
                            })?;
                            write_service_response(stream, &response, false)?;
                        }
                        break;
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => {
                        cancellation.cancel();
                        if caller_connected {
                            stream.set_nonblocking(false).map_err(|err| {
                                format!(
                                    "failed to restore workspace service response stream: {err}"
                                )
                            })?;
                            write_service_response(
                                stream,
                                &ServiceResponse::error("workspace service worker disconnected"),
                                false,
                            )?;
                        }
                        break;
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        if !caller_connected {
                            continue;
                        }
                        let mut byte = [0_u8; 1];
                        match stream.peek(&mut byte) {
                            Ok(0) => {
                                cancellation.cancel();
                                caller_connected = false;
                            }
                            Ok(_) => {}
                            Err(error) if error.kind() == ErrorKind::WouldBlock => {}
                            Err(error)
                                if matches!(
                                    error.kind(),
                                    ErrorKind::ConnectionAborted
                                        | ErrorKind::ConnectionReset
                                        | ErrorKind::BrokenPipe
                                        | ErrorKind::NotConnected
                                ) =>
                            {
                                cancellation.cancel();
                                caller_connected = false;
                            }
                            Err(_) => {
                                cancellation.cancel();
                                caller_connected = false;
                            }
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

#[derive(Debug)]
enum BoundedServiceLine {
    Line(Vec<u8>),
    TooLarge,
}

#[cfg(test)]
fn read_bounded_service_line<R: BufRead>(reader: &mut R) -> io::Result<Option<BoundedServiceLine>> {
    let mut line = Vec::new();
    let mut too_large = false;
    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            return if line.is_empty() && !too_large {
                Ok(None)
            } else if too_large {
                Ok(Some(BoundedServiceLine::TooLarge))
            } else {
                Ok(Some(BoundedServiceLine::Line(line)))
            };
        }
        let newline = available.iter().position(|byte| *byte == b'\n');
        let consumed = newline.map_or(available.len(), |index| index + 1);
        let content_len = newline.unwrap_or(available.len());
        if !too_large {
            if line.len().saturating_add(content_len) > SERVICE_REQUEST_LINE_LIMIT {
                too_large = true;
                line.clear();
            } else {
                line.extend_from_slice(&available[..content_len]);
            }
        }
        reader.consume(consumed);
        if newline.is_some() {
            return Ok(Some(if too_large {
                BoundedServiceLine::TooLarge
            } else {
                BoundedServiceLine::Line(line)
            }));
        }
    }
}

fn read_bounded_service_line_with_deadline<R, F>(
    reader: &mut R,
    deadline: &Deadline,
    clock: &dyn ConnectorClock,
    mut set_timeout: F,
) -> io::Result<Option<BoundedServiceLine>>
where
    R: BufRead,
    F: FnMut(Duration) -> io::Result<()>,
{
    let mut line = Vec::new();
    loop {
        let remaining = deadline.remaining(clock).ok_or_else(|| {
            io::Error::new(
                ErrorKind::TimedOut,
                "workspace service request header timed out",
            )
        })?;
        set_timeout(remaining.min(Duration::from_millis(100)))?;
        let available = match reader.fill_buf() {
            Ok(available) => available,
            Err(error) if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                continue
            }
            Err(error) => return Err(error),
        };
        deadline.remaining(clock).ok_or_else(|| {
            io::Error::new(
                ErrorKind::TimedOut,
                "workspace service request header timed out",
            )
        })?;
        if available.is_empty() {
            return if line.is_empty() {
                Ok(None)
            } else {
                Ok(Some(BoundedServiceLine::Line(line)))
            };
        }
        let newline = available.iter().position(|byte| *byte == b'\n');
        let consumed = newline.map_or(available.len(), |index| index + 1);
        let content_len = newline.unwrap_or(available.len());
        if line.len().saturating_add(content_len) > SERVICE_REQUEST_LINE_LIMIT {
            reader.consume(consumed);
            if newline.is_some() {
                return Ok(Some(BoundedServiceLine::TooLarge));
            }
            loop {
                let remaining = deadline.remaining(clock).ok_or_else(|| {
                    io::Error::new(
                        ErrorKind::TimedOut,
                        "workspace service request header timed out",
                    )
                })?;
                set_timeout(remaining.min(Duration::from_millis(100)))?;
                let available = reader.fill_buf()?;
                if available.is_empty() {
                    return Ok(Some(BoundedServiceLine::TooLarge));
                }
                let newline = available.iter().position(|byte| *byte == b'\n');
                let consumed = newline.map_or(available.len(), |index| index + 1);
                reader.consume(consumed);
                if newline.is_some() {
                    return Ok(Some(BoundedServiceLine::TooLarge));
                }
            }
        }
        line.extend_from_slice(&available[..content_len]);
        reader.consume(consumed);
        if newline.is_some() {
            return Ok(Some(BoundedServiceLine::Line(line)));
        }
    }
}

fn write_service_response(
    mut writer: impl Write,
    response: &ServiceResponse,
    best_effort: bool,
) -> Result<bool, String> {
    let shutdown = response.shutdown;
    let payload = serde_json::to_string(&response).map_err(|err| err.to_string())?;
    let write_result = writer
        .write_all(payload.as_bytes())
        .and_then(|_| writer.write_all(b"\n"))
        .and_then(|_| writer.flush());
    if let Err(error) = write_result {
        if best_effort {
            return Ok(false);
        }
        return Err(format!(
            "failed to write workspace service response: {error}"
        ));
    }
    Ok(shutdown)
}

fn required_arg(args: &[String], name: &str) -> Result<String, String> {
    args.windows(2)
        .find_map(|pair| (pair[0] == name).then(|| pair[1].clone()))
        .ok_or_else(|| format!("missing required workspace service argument {name}"))
}

fn optional_u64_arg(args: &[String], name: &str, default: u64) -> u64 {
    args.windows(2)
        .find_map(|pair| {
            (pair[0] == name)
                .then(|| pair[1].parse::<u64>().ok())
                .flatten()
        })
        .filter(|value| *value > 0)
        .unwrap_or(default)
}

fn bsl_writer(mut stdin: ChildStdin, requests: mpsc::Receiver<BslWriteRequest>) {
    while let Ok(request) = requests.recv() {
        let result = serde_json::to_vec(&request.payload)
            .map_err(|error| format!("failed to encode persistent MCP request: {error}"))
            .and_then(|mut bytes| {
                bytes.push(b'\n');
                stdin
                    .write_all(&bytes)
                    .map_err(|error| format!("failed to write persistent MCP request: {error}"))
            })
            .and_then(|_| {
                stdin
                    .flush()
                    .map_err(|error| format!("failed to flush persistent MCP request: {error}"))
            });
        let failed = result.is_err();
        let _ = request.result.send(result);
        if failed {
            break;
        }
    }
}

fn bsl_stderr_reader(mut stderr: impl Read, tail: Arc<Mutex<BoundedByteTail>>) {
    let mut chunk = [0_u8; 8192];
    loop {
        match stderr.read(&mut chunk) {
            Ok(0) | Err(_) => return,
            Ok(count) => {
                if let Ok(mut tail) = tail.lock() {
                    tail.append(&chunk[..count]);
                } else {
                    return;
                }
            }
        }
    }
}

#[cfg(test)]
fn bsl_reader(stdout: impl Read, events: mpsc::SyncSender<BslReaderEvent>) {
    bsl_reader_with_state(stdout, events, Arc::new(AtomicBool::new(false)));
}

fn bsl_reader_with_state(
    mut stdout: impl Read,
    events: mpsc::SyncSender<BslReaderEvent>,
    closed: Arc<AtomicBool>,
) {
    let _terminal = BslReaderTerminalGuard(closed);
    let mut pending = Vec::new();
    let mut chunk = [0_u8; 8192];
    loop {
        match stdout.read(&mut chunk) {
            Ok(0) => {
                let event = if pending.is_empty() {
                    BslReaderEvent::Closed
                } else {
                    BslReaderEvent::ProtocolError(
                        "persistent MCP stdout closed with an incomplete JSON line".to_string(),
                    )
                };
                let _ = events.send(event);
                return;
            }
            Ok(count) => {
                for &byte in &chunk[..count] {
                    if byte == b'\n' {
                        if pending.last() == Some(&b'\r') {
                            pending.pop();
                        }
                        let event = match serde_json::from_slice::<Value>(&pending) {
                            Ok(value) => BslReaderEvent::Message(value),
                            Err(error) => BslReaderEvent::ProtocolError(format!(
                                "persistent MCP emitted malformed JSON: {error}"
                            )),
                        };
                        let malformed = matches!(event, BslReaderEvent::ProtocolError(_));
                        if events.send(event).is_err() || malformed {
                            return;
                        }
                        pending.clear();
                    } else {
                        pending.push(byte);
                        if pending.len() > SERVICE_RESPONSE_LINE_LIMIT {
                            let _ = events.send(BslReaderEvent::ProtocolError(format!(
                                "persistent MCP response exceeds {SERVICE_RESPONSE_LINE_LIMIT} bytes"
                            )));
                            return;
                        }
                    }
                }
            }
            Err(error) => {
                let _ = events.send(BslReaderEvent::ProtocolError(format!(
                    "failed to read persistent MCP stdout: {error}"
                )));
                return;
            }
        }
    }
}

struct BslReaderTerminalGuard(Arc<AtomicBool>);

impl Drop for BslReaderTerminalGuard {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

fn read_json_response_cancellable(
    rx: &mpsc::Receiver<BslReaderEvent>,
    id: i64,
    deadline: &ElapsedBudgetDeadline,
    cancellation: &CancellationToken,
) -> Result<Value, String> {
    while let Some(remaining) = deadline.remaining() {
        if cancellation.is_cancelled() {
            return Err(cancelled_error(format!(
                "persistent MCP request {id} stopped"
            )));
        }
        let received = rx.recv_timeout(remaining.min(Duration::from_millis(50)));
        if cancellation.is_cancelled() {
            return Err(cancelled_error(format!(
                "persistent MCP request {id} stopped"
            )));
        }
        if deadline.remaining().is_none() {
            return Err(format!("timeout: persistent MCP request {id} timed out"));
        }
        match received {
            Ok(BslReaderEvent::Message(value)) => {
                if value.get("id").and_then(Value::as_i64) == Some(id) {
                    return Ok(value);
                }
            }
            Ok(BslReaderEvent::ProtocolError(error)) => {
                return Err(error);
            }
            Ok(BslReaderEvent::Closed) => {
                return Err("persistent MCP stdout closed before response".to_string());
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err("persistent MCP stdout closed before response".to_string());
            }
        }
    }
    if cancellation.is_cancelled() {
        return Err(cancelled_error(format!(
            "persistent MCP request {id} stopped"
        )));
    }
    Err(format!("timeout: persistent MCP request {id} timed out"))
}

fn mcp_tool_text(response: &Value) -> Result<String, String> {
    if let Some(error) = response.get("error") {
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("bsl-analyzer MCP JSON-RPC error");
        return Err(message.to_string());
    }
    let result = response
        .get("result")
        .ok_or_else(|| "bsl-analyzer MCP response is missing result".to_string())?;
    if result
        .get("isError")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        let message = result
            .get("content")
            .and_then(Value::as_array)
            .and_then(|content| {
                content
                    .iter()
                    .filter_map(|item| item.get("text").and_then(Value::as_str))
                    .next()
            })
            .unwrap_or("bsl-analyzer MCP tool returned an error");
        return Err(message.to_string());
    }
    if let Some(content) = result.get("content").and_then(Value::as_array) {
        let parts = content
            .iter()
            .filter_map(|item| item.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>();
        if !parts.is_empty() {
            return Ok(parts.join("\n"));
        }
    }
    Ok(result.to_string())
}

fn service_key(workspace_root: &str, source_root: &str) -> String {
    let mut hasher = DefaultHasher::new();
    workspace_root.hash(&mut hasher);
    source_root.hash(&mut hasher);
    format!("svc-{:016x}", hasher.finish())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::workspace::WorkspaceContext;
    use crate::infrastructure::platform::testing;
    use crate::infrastructure::source_roots::source_generation;
    use std::fs;
    use std::path::Path;
    use std::sync::OnceLock;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn rlm_start_uses_remaining_caller_deadline_for_execution_timeout() {
        let args = rlm_start_arguments(
            Path::new("/workspace/src"),
            "needle",
            Duration::from_millis(299_001),
        )
        .unwrap();

        assert_eq!(args["execution_timeout_seconds"], 300);
    }

    #[test]
    fn persistent_rlm_deadline_handles_full_configured_range_without_panicking() {
        let context = test_context("rlm-full-range-deadline");
        let fixture = compile_exit_after_call_fixture(&context);
        let pid_file = context.cache_root.join("rlm-full-range-pids.txt");
        let completion_file = context.cache_root.join("rlm-full-range-completed.txt");
        let mut command = Command::new(&fixture);
        command.args([&pid_file, &completion_file]);
        let cancellation = CancellationToken::new();
        let mut session = RlmMcpSession {
            transport: PersistentMcpSession::start_with_command(command, &cancellation).unwrap(),
            source_root: context.workspace_root.join("src"),
            session_id: None,
        };
        cancellation.cancel();

        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            session.execute(
                &WorkspaceRlmOperation::Search {
                    query: "needle".to_string(),
                    limit: 20,
                },
                Duration::from_secs(i64::MAX as u64),
                &cancellation,
            )
        }));

        drop(session);
        cleanup(&context);
        let error = outcome.expect("full configured timeout must not overflow Instant");
        assert!(error.unwrap_err().starts_with("cancelled:"));
    }

    #[test]
    fn persistent_transport_deadline_handles_full_configured_range_without_panicking() {
        let context = test_context("persistent-transport-full-range-deadline");
        let fixture = compile_exit_after_call_fixture(&context);
        let pid_file = context.cache_root.join("transport-full-range-pids.txt");
        let completion_file = context
            .cache_root
            .join("transport-full-range-completed.txt");
        let mut command = Command::new(&fixture);
        command.args([&pid_file, &completion_file]);
        let cancellation = CancellationToken::new();
        let mut session = PersistentMcpSession::start_with_command(command, &cancellation).unwrap();

        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            session.call(
                "rlm_start",
                json!({}),
                Duration::from_secs(i64::MAX as u64),
                &cancellation,
            )
        }));

        drop(session);
        cleanup(&context);
        let output = outcome.expect("full configured timeout must not overflow Instant");
        assert_eq!(output.unwrap().result_text, "ok");
    }

    #[test]
    fn rlm_protocol_roundtrips_full_positive_i64_seconds() {
        let wire = json!({
            "type": "rlm-mcp",
            "operation_id": "max-timeout",
            "operation": {
                "operation": "search",
                "query": "needle",
                "limit": 20
            },
            "timeout_seconds": i64::MAX,
            "timeout_nanos": 0
        });

        let request: ServiceRequestKind = serde_json::from_value(wire.clone()).unwrap();

        assert_eq!(serde_json::to_value(request).unwrap(), wire);
    }

    #[test]
    fn bsl_protocol_roundtrips_full_positive_i64_seconds() {
        let wire = json!({
            "type": "bsl-mcp",
            "operation_id": "max-timeout",
            "tool_name": "search",
            "tool_args": {"query": "needle"},
            "timeout_seconds": i64::MAX,
            "timeout_nanos": 0
        });

        let request: ServiceRequestKind = serde_json::from_value(wire.clone()).unwrap();

        assert_eq!(serde_json::to_value(request).unwrap(), wire);
    }

    #[test]
    fn rlm_readiness_protocol_roundtrips_full_positive_i64_seconds() {
        let wire = json!({
            "type": "rlm-ready",
            "operation_id": "max-timeout",
            "args": {},
            "timeout_seconds": i64::MAX,
            "timeout_nanos": 123_456_789
        });

        let request: ServiceRequestKind = serde_json::from_value(wire.clone()).unwrap();

        assert_eq!(serde_json::to_value(request).unwrap(), wire);
    }

    #[test]
    fn service_timeout_parts_reject_seconds_outside_positive_i64_config_domain() {
        let error = duration_from_timeout_parts(u64::MAX, 1)
            .expect_err("wire timeout must stay inside the accepted config domain");

        assert!(error.contains("exceeds"), "{error}");
        assert!(error.contains(&i64::MAX.to_string()), "{error}");
    }

    #[test]
    fn duration_seconds_ceil_reports_overflow_instead_of_panicking() {
        let error = duration_seconds_ceil(Duration::new(u64::MAX, 1))
            .expect_err("invalid wire-derived duration must not panic or wrap");

        assert!(error.contains("rounded up"), "{error}");
    }

    #[test]
    fn rlm_wire_boundary_rejects_out_of_domain_timeout_before_runtime_work() {
        let context = test_context("rlm-out-of-domain-wire-timeout");
        let source_root = context.workspace_root.join("src");
        let identity = WorkspaceServiceIdentity::new(&context, &source_root).unwrap();
        let record = test_record(&identity, 1, env!("CARGO_PKG_VERSION"));
        let runtime = WorkspaceServiceRuntime::new(identity, &record);

        let response = runtime.handle_rlm_mcp(
            WorkspaceRlmOperation::Search {
                query: "needle".to_string(),
                limit: 20,
            },
            u64::MAX,
            1,
            &CancellationToken::new(),
        );

        cleanup(&context);
        assert!(!response.ok, "{response:?}");
        let error = response.error.expect("protocol error");
        assert!(error.contains("exceeds"), "{error}");
        assert!(error.contains(&i64::MAX.to_string()), "{error}");
    }

    #[test]
    fn rlm_readiness_wire_boundary_rejects_out_of_domain_timeout_before_runtime_work() {
        let context = test_context("rlm-readiness-out-of-domain-wire-timeout");
        let source_root = context.workspace_root.join("src");
        let identity = WorkspaceServiceIdentity::new(&context, &source_root).unwrap();
        let record = test_record(&identity, 1, env!("CARGO_PKG_VERSION"));
        let runtime = WorkspaceServiceRuntime::new(identity, &record);

        let response = runtime.handle_rlm_ready(json!({}), u64::MAX, 1, &CancellationToken::new());

        cleanup(&context);
        assert!(!response.ok, "{response:?}");
        let error = response.error.expect("protocol error");
        assert!(error.contains("exceeds"), "{error}");
        assert!(error.contains(&i64::MAX.to_string()), "{error}");
    }

    #[test]
    fn fixed_rlm_helper_keeps_untrusted_values_inside_json_data() {
        let query = "x'); __import__('os').system('boom')\n#";
        let code = fixed_rlm_helper_code(&WorkspaceRlmOperation::Search {
            query: query.to_string(),
            limit: 7,
        })
        .unwrap();

        assert!(!code.contains("_result = search(\"x');"));
        let literal = code
            .lines()
            .find_map(|line| line.strip_prefix("_args = json.loads("))
            .and_then(|line| line.strip_suffix(')'))
            .expect("serialized argument literal");
        let serialized: String = serde_json::from_str(literal).unwrap();
        let decoded: WorkspaceRlmOperation = serde_json::from_str(&serialized).unwrap();
        assert_eq!(
            decoded,
            WorkspaceRlmOperation::Search {
                query: query.to_string(),
                limit: 7
            }
        );
    }

    /// #204. A persistent session outlives its tool call, so a working
    /// directory inside the source tree keeps that tree open and
    /// `git worktree remove` fails long after the call returned. The tree is
    /// addressed by `--source-dir`, so the working directory only holds.
    #[test]
    fn bsl_analyzer_child_does_not_run_from_inside_the_workspace() {
        let Some(runtime_dir) = short_private_runtime_dir().unwrap() else {
            return;
        };
        let mut command = Command::new("bsl-analyzer");
        command.current_dir(std::env::temp_dir().join("unica-worktree-under-test"));

        configure_bsl_analyzer_runtime_dir(&mut command).unwrap();

        assert_eq!(command.get_current_dir(), Some(runtime_dir.as_path()));
        assert_eq!(runtime_dir.parent(), Some(Path::new("/tmp")));
    }

    #[test]
    fn bsl_analyzer_runtime_directory_is_passed_to_the_child_and_fits_socket_budget() {
        use std::ffi::OsStr;

        let expected = short_private_runtime_dir().unwrap();
        let mut command = Command::new("bsl-analyzer");
        configure_bsl_analyzer_runtime_dir(&mut command).unwrap();
        let actual = command
            .get_envs()
            .find(|(key, _)| *key == OsStr::new("XDG_RUNTIME_DIR"))
            .and_then(|(_, value)| value.map(PathBuf::from));

        assert_eq!(actual, expected);
        let Some(runtime_dir) = actual else {
            return;
        };

        let socket = runtime_dir
            .join("bsl-mcp")
            .join("0123456789abcdef0123456789abcdef.sock");

        assert_eq!(runtime_dir.parent(), Some(Path::new("/tmp")));
        assert!(socket.as_os_str().as_encoded_bytes().len() < 104);
    }

    #[test]
    fn bsl_analyzer_child_uses_source_specific_cache_outside_the_source_tree() {
        let context = test_context("bsl-analyzer-cache");
        let first_source = context.workspace_root.join("src/main");
        let second_source = context.workspace_root.join("src/extension");
        fs::create_dir_all(&first_source).unwrap();
        fs::create_dir_all(&second_source).unwrap();

        let mut first = Command::new("bsl-analyzer");
        configure_bsl_analyzer_cache_dir(&mut first, &context, &first_source).unwrap();
        let mut second = Command::new("bsl-analyzer");
        configure_bsl_analyzer_cache_dir(&mut second, &context, &second_source).unwrap();

        let cache_arg = |command: &Command| {
            let arguments = command.get_args().collect::<Vec<_>>();
            let position = arguments
                .iter()
                .position(|argument| *argument == "--cache-dir")
                .expect("cache argument");
            PathBuf::from(arguments[position + 1])
        };
        let first_cache = cache_arg(&first);
        let second_cache = cache_arg(&second);

        assert!(first_cache.starts_with(&context.cache_root));
        assert!(!first_cache.starts_with(&first_source));
        assert_ne!(first_cache, second_cache);
    }

    #[test]
    fn bsl_analyzer_cache_stays_outside_a_workspace_wide_source_root() {
        let mut context = test_context("bsl-analyzer-workspace-cache");
        context.cache_root = context.workspace_root.join(".build/unica");

        let mut command = Command::new("bsl-analyzer");
        configure_bsl_analyzer_cache_dir(&mut command, &context, &context.workspace_root).unwrap();

        let arguments = command.get_args().collect::<Vec<_>>();
        let position = arguments
            .iter()
            .position(|argument| *argument == "--cache-dir")
            .expect("cache argument");
        let cache = PathBuf::from(arguments[position + 1]);

        assert!(!cache.starts_with(&context.workspace_root));
    }

    #[test]
    fn object_profile_helper_maps_public_sections_and_composes_predefined_items() {
        let code = fixed_rlm_helper_code(&WorkspaceRlmOperation::ObjectProfile {
            name: "Document.Заказ".to_string(),
            sections: Some(vec![
                "functionalOptions".to_string(),
                "predefinedItems".to_string(),
            ]),
            limit: 20,
        })
        .unwrap();

        assert!(code.contains("\"functionalOptions\": \"functional_options\""));
        assert!(code.contains("find_predefined("));
        assert!(code.contains("[\"predefined_items\"]"));
        assert_eq!(code.matches("print(json.dumps(").count(), 1);
    }

    #[test]
    fn object_profile_helper_reports_predefined_items_it_holds_instead_of_a_fabricated_total() {
        let code = fixed_rlm_helper_code(&WorkspaceRlmOperation::ObjectProfile {
            name: "Document.Заказ".to_string(),
            sections: Some(vec!["predefinedItems".to_string()]),
            limit: 20,
        })
        .unwrap();

        assert!(code.contains("\"total\": len(_items),"), "{code}");
        assert!(!code.contains("(1 if _has_more else 0)"), "{code}");
        // The over-fetch decides truncation before the category filter can hide it.
        let fetch = code
            .find("_has_more = len(_fetched) > _args[\"limit\"]")
            .expect("truncation is measured from the raw fetch window");
        let filter = code
            .find("_item.get(\"category\") == _category")
            .expect("the category filter is still applied");
        assert!(fetch < filter, "{code}");
    }

    #[test]
    fn object_profile_helper_does_not_claim_category_complete_results_after_a_capped_fetch() {
        let code = fixed_rlm_helper_code(&WorkspaceRlmOperation::ObjectProfile {
            name: "Document.Заказ".to_string(),
            sections: Some(vec!["predefinedItems".to_string()]),
            limit: 20,
        })
        .unwrap();

        assert!(
            code.contains("if _category and _has_more:"),
            "category ambiguity must be handled explicitly: {code}"
        );
        assert!(
            code.contains("\"status\": \"unavailable\""),
            "a capped cross-category fetch must not claim ok or empty: {code}"
        );
        assert!(
            code.contains("category-complete predefined items"),
            "the unavailable result must explain why completeness is unknown: {code}"
        );
    }

    #[test]
    fn service_response_preserves_exact_stale_status() {
        let response = ServiceResponse::from_readiness(
            IndexReadiness::Stale {
                status: "stale (content)".to_string(),
            },
            Vec::new(),
        );

        assert_eq!(
            response.index_readiness(),
            IndexReadiness::Stale {
                status: "stale (content)".to_string(),
            }
        );
    }

    #[test]
    fn service_response_preserves_failed_index_message() {
        let response = ServiceResponse::from_readiness(
            IndexReadiness::Failed(
                "update left stale (content); recovery build failed".to_string(),
            ),
            Vec::new(),
        );

        assert_eq!(
            response.index_readiness(),
            IndexReadiness::Failed(
                "update left stale (content); recovery build failed".to_string(),
            )
        );
    }

    #[derive(Default)]
    struct BlockingWorkspaceState {
        started: Vec<String>,
        cancelled: Vec<String>,
        release_cancelled: bool,
    }

    #[derive(Default)]
    struct BlockingWorkspaceExecutor {
        state: Mutex<BlockingWorkspaceState>,
        wake: std::sync::Condvar,
    }

    impl BlockingWorkspaceExecutor {
        fn wait_started(&self, expected: usize) {
            let deadline = Instant::now() + Duration::from_secs(2);
            let mut state = self.state.lock().unwrap();
            while state.started.len() < expected {
                let remaining = deadline.saturating_duration_since(Instant::now());
                assert!(
                    !remaining.is_zero(),
                    "work did not start before test deadline"
                );
                let (next, timeout) = self.wake.wait_timeout(state, remaining).unwrap();
                state = next;
                assert!(!timeout.timed_out() || state.started.len() >= expected);
            }
        }

        fn wait_cancelled(&self, expected: usize) {
            let deadline = Instant::now() + Duration::from_secs(2);
            let mut state = self.state.lock().unwrap();
            while state.cancelled.len() < expected {
                let remaining = deadline.saturating_duration_since(Instant::now());
                assert!(
                    !remaining.is_zero(),
                    "operation was not cancelled before test deadline"
                );
                let (next, timeout) = self.wake.wait_timeout(state, remaining).unwrap();
                state = next;
                assert!(!timeout.timed_out() || state.cancelled.len() >= expected);
            }
        }

        fn release_cancelled(&self) {
            self.state.lock().unwrap().release_cancelled = true;
            self.wake.notify_all();
        }
    }

    impl WorkspaceServiceOperationExecutor for BlockingWorkspaceExecutor {
        fn execute(
            &self,
            _runtime: &WorkspaceServiceRuntime,
            kind: ServiceRequestKind,
            cancellation: &CancellationToken,
        ) -> ServiceResponse {
            let operation_id = kind.operation_id().unwrap_or("missing").to_string();
            if operation_id.starts_with("panic-worker") {
                panic!("intentional workspace worker panic");
            }
            {
                let mut state = self.state.lock().unwrap();
                state.started.push(operation_id.clone());
                self.wake.notify_all();
            }
            if operation_id.starts_with("success") {
                return ServiceResponse {
                    ok: true,
                    status: Some("ready".to_string()),
                    ..ServiceResponse::default()
                };
            }
            let deadline = Instant::now() + Duration::from_secs(2);
            while !cancellation.is_cancelled() && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(10));
            }
            if cancellation.is_cancelled() {
                let mut state = self.state.lock().unwrap();
                state.cancelled.push(operation_id.clone());
                self.wake.notify_all();
                if operation_id.starts_with("held-after-cancel") {
                    while !state.release_cancelled {
                        state = self.wake.wait(state).unwrap();
                    }
                }
                ServiceResponse::error(cancelled_error("workspace operation stopped"))
            } else {
                ServiceResponse::error("test operation was not cancelled")
            }
        }
    }

    #[derive(Default)]
    struct AnalyzerLaneExecutor {
        holder_started: Mutex<bool>,
        wake: std::sync::Condvar,
    }

    impl AnalyzerLaneExecutor {
        fn wait_holder(&self) {
            let deadline = Instant::now() + Duration::from_secs(2);
            let mut started = self.holder_started.lock().unwrap();
            while !*started {
                let remaining = deadline.saturating_duration_since(Instant::now());
                assert!(!remaining.is_zero(), "analyzer lane holder did not start");
                (started, _) = self.wake.wait_timeout(started, remaining).unwrap();
            }
        }
    }

    impl WorkspaceServiceOperationExecutor for AnalyzerLaneExecutor {
        fn execute(
            &self,
            runtime: &WorkspaceServiceRuntime,
            kind: ServiceRequestKind,
            cancellation: &CancellationToken,
        ) -> ServiceResponse {
            let operation_id = kind.operation_id().unwrap().to_string();
            let _lane = match runtime.acquire_analyzer_lane(cancellation) {
                Ok(lane) => lane,
                Err(error) => return ServiceResponse::error(error),
            };
            if operation_id.starts_with("analyzer-holder") {
                *self.holder_started.lock().unwrap() = true;
                self.wake.notify_all();
                while !cancellation.is_cancelled() {
                    thread::sleep(Duration::from_millis(10));
                }
                return ServiceResponse::error(cancelled_error("analyzer holder stopped"));
            }
            ServiceResponse {
                ok: true,
                status: Some("analyzer-lane-acquired".to_string()),
                ..ServiceResponse::default()
            }
        }
    }

    type WorkspaceControlTestServer = (
        WorkspaceContext,
        WorkspaceServiceRecord,
        Arc<WorkspaceServiceRuntime>,
        Arc<BlockingWorkspaceExecutor>,
        thread::JoinHandle<Result<(), String>>,
    );

    fn workspace_control_test_server(name: &str) -> WorkspaceControlTestServer {
        workspace_control_test_server_with_options(
            name,
            SERVICE_MAX_CONNECTION_HANDLERS,
            SERVICE_REQUEST_HEADER_TIMEOUT,
            None,
        )
    }

    fn workspace_control_test_server_with_general_limit(
        name: &str,
        general_limit: usize,
    ) -> WorkspaceControlTestServer {
        workspace_control_test_server_with_options(
            name,
            general_limit,
            SERVICE_REQUEST_HEADER_TIMEOUT,
            None,
        )
    }

    fn workspace_control_test_server_with_header_timeout(
        name: &str,
        request_header_timeout: Duration,
        handler_started_hook: Arc<dyn Fn() + Send + Sync>,
    ) -> WorkspaceControlTestServer {
        workspace_control_test_server_with_options(
            name,
            SERVICE_MAX_CONNECTION_HANDLERS,
            request_header_timeout,
            Some(handler_started_hook),
        )
    }

    fn workspace_control_test_server_with_options(
        name: &str,
        general_limit: usize,
        request_header_timeout: Duration,
        handler_started_hook: Option<Arc<dyn Fn() + Send + Sync>>,
    ) -> WorkspaceControlTestServer {
        let context = test_context(name);
        let identity =
            WorkspaceServiceIdentity::new(&context, &context.workspace_root.join("src")).unwrap();
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        let record = test_record(&identity, port, env!("CARGO_PKG_VERSION"));
        write_record(&identity, record.clone());
        let mut runtime = WorkspaceServiceRuntime::new(identity, &record);
        runtime.general_admission = AdmissionGate::new(general_limit);
        *runtime.handler_started_hook.lock().unwrap() = handler_started_hook;
        let runtime = Arc::new(runtime);
        let executor = Arc::new(BlockingWorkspaceExecutor::default());
        let server_runtime = Arc::clone(&runtime);
        let server_executor = Arc::clone(&executor);
        let server = thread::spawn(move || {
            serve_workspace_service(
                listener,
                server_runtime,
                server_executor,
                Duration::from_secs(30),
                Duration::from_secs(30),
                request_header_timeout,
                SERVICE_SHUTDOWN_GRACE,
            )
        });
        (context, record, runtime, executor, server)
    }

    fn workspace_test_server_with_executor(
        name: &str,
        executor: Arc<dyn WorkspaceServiceOperationExecutor>,
        shutdown_grace: Duration,
    ) -> (
        WorkspaceContext,
        WorkspaceServiceRecord,
        Arc<WorkspaceServiceRuntime>,
        thread::JoinHandle<Result<(), String>>,
    ) {
        let context = test_context(name);
        let identity =
            WorkspaceServiceIdentity::new(&context, &context.workspace_root.join("src")).unwrap();
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        listener.set_nonblocking(true).unwrap();
        let record = test_record(
            &identity,
            listener.local_addr().unwrap().port(),
            env!("CARGO_PKG_VERSION"),
        );
        write_record(&identity, record.clone());
        let runtime = Arc::new(WorkspaceServiceRuntime::new(identity, &record));
        let server_runtime = Arc::clone(&runtime);
        let server = thread::spawn(move || {
            serve_workspace_service(
                listener,
                server_runtime,
                executor,
                Duration::from_secs(30),
                Duration::from_secs(30),
                SERVICE_REQUEST_HEADER_TIMEOUT,
                shutdown_grace,
            )
        });
        (context, record, runtime, server)
    }

    fn open_test_request(
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

    fn read_test_response(reader: &mut BufReader<TcpStream>) -> ServiceResponse {
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        serde_json::from_str(line.trim()).unwrap()
    }

    fn send_test_request(
        record: &WorkspaceServiceRecord,
        kind: ServiceRequestKind,
    ) -> ServiceResponse {
        read_test_response(&mut open_test_request(record, kind))
    }

    #[test]
    fn workspace_service_control_path_ping_cancel_and_recover() {
        let (context, record, runtime, executor, server) =
            workspace_control_test_server("control-ping-cancel");
        let mut work = open_test_request(
            &record,
            ServiceRequestKind::RlmReady {
                operation_id: "blocked-1".to_string(),
                args: json!({}),
                timeout_seconds: 5,
                timeout_nanos: 0,
            },
        );
        executor.wait_started(1);

        let ping_started = Instant::now();
        let ping = send_test_request(&record, ServiceRequestKind::Ping);
        assert!(ping.ok);
        assert!(ping_started.elapsed() < Duration::from_millis(500));

        let cancel = send_test_request(
            &record,
            ServiceRequestKind::Cancel {
                operation_id: "blocked-1".to_string(),
            },
        );
        assert!(cancel.ok);
        let cancelled = read_test_response(&mut work);
        assert!(!cancelled.ok);
        assert!(cancelled.error.unwrap().starts_with("cancelled:"));
        assert!(runtime.operations.lock().unwrap().is_empty());

        let recovered = send_test_request(
            &record,
            ServiceRequestKind::RlmReady {
                operation_id: "success-2".to_string(),
                args: json!({}),
                timeout_seconds: 5,
                timeout_nanos: 0,
            },
        );
        assert!(recovered.ok);
        assert!(runtime.operations.lock().unwrap().is_empty());

        assert!(send_test_request(&record, ServiceRequestKind::Shutdown).ok);
        server.join().unwrap().unwrap();
        let identity =
            WorkspaceServiceIdentity::new(&context, &context.workspace_root.join("src")).unwrap();
        assert!(!identity.record_path().exists());
        cleanup(&context);
    }

    #[test]
    fn workspace_service_request_lines_are_bounded_and_resynchronize() {
        let mut bytes = vec![b'x'; SERVICE_REQUEST_LINE_LIMIT + 1];
        bytes.push(b'\n');
        bytes.extend_from_slice(b"{}\n");
        let mut reader = BufReader::new(std::io::Cursor::new(bytes));
        assert!(matches!(
            read_bounded_service_line(&mut reader).unwrap(),
            Some(BoundedServiceLine::TooLarge)
        ));
        assert!(
            matches!(read_bounded_service_line(&mut reader).unwrap(), Some(BoundedServiceLine::Line(line)) if line == b"{}")
        );
    }

    #[test]
    fn workspace_service_header_deadline_is_aggregate_across_drip_bytes() {
        struct DripReader {
            bytes: Vec<u8>,
            offset: usize,
            clock: ManualClock,
        }
        impl Read for DripReader {
            fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
                if self.offset == self.bytes.len() {
                    return Ok(0);
                }
                self.clock.advance(Duration::from_millis(1));
                buf[0] = self.bytes[self.offset];
                self.offset += 1;
                Ok(1)
            }
        }
        let clock = ManualClock::default();
        let mut reader = BufReader::with_capacity(
            1,
            DripReader {
                bytes: b"{}\n".to_vec(),
                offset: 0,
                clock: clock.clone(),
            },
        );
        let deadline = Deadline::new(&clock, Duration::from_millis(2));
        let error =
            read_bounded_service_line_with_deadline(&mut reader, &deadline, &clock, |_| Ok(()))
                .unwrap_err();
        assert_eq!(error.kind(), ErrorKind::TimedOut);
    }

    #[test]
    fn workspace_service_header_timeout_is_connection_local() {
        let (handler_started_tx, handler_started_rx) = mpsc::sync_channel(0);
        let handler_started_tx = Mutex::new(Some(handler_started_tx));
        let handler_started_hook = Arc::new(move || {
            if let Some(sender) = handler_started_tx.lock().unwrap().take() {
                let _ = sender.send(());
            }
        });
        let (context, record, runtime, _executor, server) =
            workspace_control_test_server_with_header_timeout(
                "header-timeout-recovery",
                Duration::from_millis(50),
                handler_started_hook,
            );
        let mut stalled = TcpStream::connect(("127.0.0.1", record.port)).unwrap();
        stalled.write_all(b"{").unwrap();
        stalled.flush().unwrap();

        handler_started_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("stalled connection handler did not start");
        let timeout_deadline = Instant::now() + Duration::from_secs(2);
        while runtime.general_admission.active.load(Ordering::Acquire) != 0 {
            assert!(
                Instant::now() < timeout_deadline,
                "timed-out connection did not release its handler"
            );
            thread::yield_now();
        }

        assert!(send_test_request(&record, ServiceRequestKind::Ping).ok);
        assert!(send_test_request(&record, ServiceRequestKind::Shutdown).ok);
        drop(stalled);
        server.join().unwrap().unwrap();
        cleanup(&context);
    }

    #[test]
    fn workspace_service_handler_failures_have_stable_diagnostics() {
        assert_eq!(workspace_service_handler_diagnostic(Ok(Ok(()))), None);
        assert_eq!(
            workspace_service_handler_diagnostic(Ok(Err("header timed out".to_string()))),
            Some("workspace service connection failed: header timed out".to_string())
        );
        let panic_result: thread::Result<Result<(), String>> = Err(Box::new("panic"));
        assert_eq!(
            workspace_service_handler_diagnostic(panic_result),
            Some("workspace service connection handler panicked".to_string())
        );

        report_workspace_service_handler_result_to(
            &mut FailingWriter,
            Ok(Err("header timed out".to_string())),
        );
    }

    #[test]
    fn slow_general_headers_cannot_exhaust_control_capacity() {
        const GENERAL_LIMIT: usize = 4;
        let (context, record, runtime, _executor, server) =
            workspace_control_test_server_with_general_limit(
                "header-control-reserve",
                GENERAL_LIMIT,
            );
        let mut slow = Vec::new();
        for _ in 0..GENERAL_LIMIT {
            let mut stream = TcpStream::connect(("127.0.0.1", record.port)).unwrap();
            stream.write_all(b"{").unwrap();
            slow.push(stream);
            let expected = slow.len();
            let deadline = Instant::now() + Duration::from_secs(2);
            while runtime.general_admission.active.load(Ordering::Acquire) < expected {
                assert!(
                    Instant::now() < deadline,
                    "general header permit {expected} was not admitted"
                );
                thread::yield_now();
            }
        }
        let started = Instant::now();
        let overloaded = send_test_request(
            &record,
            ServiceRequestKind::RlmReady {
                operation_id: "header-overload".into(),
                args: json!({}),
                timeout_seconds: 5,
                timeout_nanos: 0,
            },
        );
        assert!(!overloaded.ok);
        assert_eq!(
            overloaded.error.as_deref(),
            Some("workspace service overloaded: general connection handlers are saturated")
        );
        assert!(send_test_request(&record, ServiceRequestKind::Ping).ok);
        assert!(
            send_test_request(
                &record,
                ServiceRequestKind::Cancel {
                    operation_id: "none".into()
                }
            )
            .ok
        );
        assert!(send_test_request(&record, ServiceRequestKind::Shutdown).ok);
        assert!(started.elapsed() < SERVICE_CONTROL_CONNECT_TIMEOUT);
        drop(slow);
        server.join().unwrap().unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while runtime.general_admission.active.load(Ordering::Acquire) != 0
            || runtime.control_admission.active.load(Ordering::Acquire) != 0
        {
            assert!(
                Instant::now() < deadline,
                "connection permits were not returned"
            );
            thread::yield_now();
        }
        let identity =
            WorkspaceServiceIdentity::new(&context, &context.workspace_root.join("src")).unwrap();
        assert!(!identity.record_path().exists());
    }

    #[test]
    fn workspace_service_work_saturation_preserves_control_path() {
        let (context, record, _runtime, executor, server) =
            workspace_control_test_server("work-saturation");
        let mut work = Vec::new();
        for index in 0..SERVICE_MAX_WORKERS {
            work.push(open_test_request(
                &record,
                ServiceRequestKind::RlmReady {
                    operation_id: format!("blocked-{index}"),
                    args: json!({}),
                    timeout_seconds: 5,
                    timeout_nanos: 0,
                },
            ));
        }
        executor.wait_started(SERVICE_MAX_WORKERS);
        let overloaded = send_test_request(
            &record,
            ServiceRequestKind::RlmReady {
                operation_id: "overloaded".into(),
                args: json!({}),
                timeout_seconds: 5,
                timeout_nanos: 0,
            },
        );
        assert!(!overloaded.ok);
        assert!(overloaded.error.unwrap().contains("overloaded"));
        assert!(send_test_request(&record, ServiceRequestKind::Ping).ok);
        for index in 0..SERVICE_MAX_WORKERS {
            assert!(
                send_test_request(
                    &record,
                    ServiceRequestKind::Cancel {
                        operation_id: format!("blocked-{index}")
                    }
                )
                .ok
            );
        }
        for reader in &mut work {
            assert!(!read_test_response(reader).ok);
        }
        assert!(send_test_request(&record, ServiceRequestKind::Shutdown).ok);
        server.join().unwrap().unwrap();
        let identity =
            WorkspaceServiceIdentity::new(&context, &context.workspace_root.join("src")).unwrap();
        assert!(!identity.record_path().exists());
    }

    #[test]
    fn workspace_service_control_path_shutdown_cancels_all_and_rejects_new_work() {
        let (context, record, runtime, executor, server) =
            workspace_control_test_server("control-shutdown");
        let mut first = open_test_request(
            &record,
            ServiceRequestKind::RlmReady {
                operation_id: "blocked-first".to_string(),
                args: json!({}),
                timeout_seconds: 5,
                timeout_nanos: 0,
            },
        );
        let mut second = open_test_request(
            &record,
            ServiceRequestKind::RlmReady {
                operation_id: "blocked-second".to_string(),
                args: json!({}),
                timeout_seconds: 5,
                timeout_nanos: 0,
            },
        );
        executor.wait_started(2);
        let shutdown = send_test_request(&record, ServiceRequestKind::Shutdown);
        assert!(shutdown.ok);
        for response in [
            read_test_response(&mut first),
            read_test_response(&mut second),
        ] {
            assert!(response.error.unwrap().starts_with("cancelled:"));
        }

        let rejected = match runtime.register_operation("late-work".to_string()) {
            Ok(_) => panic!("workspace service accepted work after shutdown"),
            Err(error) => error,
        };
        assert!(rejected.contains("shutting down"));
        executor.wait_cancelled(2);

        server.join().unwrap().unwrap();
        assert!(runtime.operations.lock().unwrap().is_empty());
        cleanup(&context);
    }

    #[test]
    fn workspace_service_record_outlives_late_maintenance_registration() {
        struct LateMaintenanceExecutor {
            before_registration: mpsc::Sender<()>,
            allow_registration: Mutex<mpsc::Receiver<()>>,
            task_started: mpsc::Sender<()>,
            task_release: Arc<Mutex<mpsc::Receiver<()>>>,
        }

        impl WorkspaceServiceOperationExecutor for LateMaintenanceExecutor {
            fn execute(
                &self,
                runtime: &WorkspaceServiceRuntime,
                _kind: ServiceRequestKind,
                _cancellation: &CancellationToken,
            ) -> ServiceResponse {
                self.before_registration.send(()).unwrap();
                self.allow_registration.lock().unwrap().recv().unwrap();
                let task_started = self.task_started.clone();
                let task_release = Arc::clone(&self.task_release);
                let handle = thread::spawn(move || {
                    task_started.send(()).unwrap();
                    task_release.lock().unwrap().recv().unwrap();
                });
                runtime.rlm_maintenance_tasks.track(handle);
                ServiceResponse {
                    ok: true,
                    status: Some("late-maintenance-finished".to_string()),
                    ..ServiceResponse::default()
                }
            }
        }

        let (before_registration_tx, before_registration_rx) = mpsc::channel();
        let (allow_registration_tx, allow_registration_rx) = mpsc::channel();
        let (task_started_tx, task_started_rx) = mpsc::channel();
        let (task_release_tx, task_release_rx) = mpsc::channel();
        let executor = Arc::new(LateMaintenanceExecutor {
            before_registration: before_registration_tx,
            allow_registration: Mutex::new(allow_registration_rx),
            task_started: task_started_tx,
            task_release: Arc::new(Mutex::new(task_release_rx)),
        });
        let (context, record, _runtime, server) = workspace_test_server_with_executor(
            "late-maintenance-record",
            executor,
            SERVICE_SHUTDOWN_GRACE,
        );
        let mut request = open_test_request(
            &record,
            ServiceRequestKind::RlmReady {
                operation_id: "late-maintenance".to_string(),
                args: json!({}),
                timeout_seconds: 5,
                timeout_nanos: 0,
            },
        );
        before_registration_rx
            .recv_timeout(Duration::from_secs(1))
            .unwrap();

        assert!(send_test_request(&record, ServiceRequestKind::Shutdown).ok);
        let record_path =
            WorkspaceServiceIdentity::new(&context, &context.workspace_root.join("src"))
                .unwrap()
                .record_path();
        let (server_finished_tx, server_finished_rx) = mpsc::channel();
        let joiner = thread::spawn(move || {
            server_finished_tx.send(server.join().unwrap()).unwrap();
        });
        allow_registration_tx.send(()).unwrap();
        task_started_rx
            .recv_timeout(Duration::from_secs(1))
            .unwrap();

        assert!(matches!(
            server_finished_rx.recv_timeout(Duration::from_millis(100)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ));
        assert!(record_path.exists());

        task_release_tx.send(()).unwrap();
        assert!(read_test_response(&mut request).ok);
        server_finished_rx
            .recv_timeout(Duration::from_secs(1))
            .unwrap()
            .unwrap();
        joiner.join().unwrap();
        assert!(!record_path.exists());
        cleanup(&context);
    }

    #[test]
    fn workspace_service_record_outlives_late_session_retirement() {
        struct BlockingRetiredSession {
            drop_started: mpsc::Sender<()>,
            drop_release: Arc<Mutex<mpsc::Receiver<()>>>,
        }

        impl Drop for BlockingRetiredSession {
            fn drop(&mut self) {
                self.drop_started.send(()).unwrap();
                self.drop_release.lock().unwrap().recv().unwrap();
            }
        }

        struct LateSessionRetirementExecutor {
            before_retirement: mpsc::Sender<()>,
            allow_retirement: Mutex<mpsc::Receiver<()>>,
            drop_started: mpsc::Sender<()>,
            drop_release: Arc<Mutex<mpsc::Receiver<()>>>,
        }

        impl WorkspaceServiceOperationExecutor for LateSessionRetirementExecutor {
            fn execute(
                &self,
                runtime: &WorkspaceServiceRuntime,
                _kind: ServiceRequestKind,
                _cancellation: &CancellationToken,
            ) -> ServiceResponse {
                self.before_retirement.send(()).unwrap();
                self.allow_retirement.lock().unwrap().recv().unwrap();
                runtime.retire_session(BlockingRetiredSession {
                    drop_started: self.drop_started.clone(),
                    drop_release: Arc::clone(&self.drop_release),
                });
                ServiceResponse {
                    ok: true,
                    status: Some("late-session-retired".to_string()),
                    ..ServiceResponse::default()
                }
            }
        }

        let (before_retirement_tx, before_retirement_rx) = mpsc::channel();
        let (allow_retirement_tx, allow_retirement_rx) = mpsc::channel();
        let (drop_started_tx, drop_started_rx) = mpsc::channel();
        let (drop_release_tx, drop_release_rx) = mpsc::channel();
        let executor = Arc::new(LateSessionRetirementExecutor {
            before_retirement: before_retirement_tx,
            allow_retirement: Mutex::new(allow_retirement_rx),
            drop_started: drop_started_tx,
            drop_release: Arc::new(Mutex::new(drop_release_rx)),
        });
        let (context, record, runtime, server) = workspace_test_server_with_executor(
            "late-session-record",
            executor,
            SERVICE_SHUTDOWN_GRACE,
        );
        let (providers_drained_tx, providers_drained_rx) = mpsc::channel();
        *runtime.provider_sessions_drained_hook.lock().unwrap() =
            Some(Arc::new(move || providers_drained_tx.send(()).unwrap()));
        let mut request = open_test_request(
            &record,
            ServiceRequestKind::RlmReady {
                operation_id: "late-session".to_string(),
                args: json!({}),
                timeout_seconds: 5,
                timeout_nanos: 0,
            },
        );
        before_retirement_rx
            .recv_timeout(Duration::from_secs(1))
            .unwrap();

        assert!(send_test_request(&record, ServiceRequestKind::Shutdown).ok);
        providers_drained_rx
            .recv_timeout(Duration::from_secs(1))
            .unwrap();
        let record_path =
            WorkspaceServiceIdentity::new(&context, &context.workspace_root.join("src"))
                .unwrap()
                .record_path();
        let (server_finished_tx, server_finished_rx) = mpsc::channel();
        let joiner = thread::spawn(move || {
            server_finished_tx.send(server.join().unwrap()).unwrap();
        });
        allow_retirement_tx.send(()).unwrap();
        drop_started_rx
            .recv_timeout(Duration::from_secs(1))
            .unwrap();

        assert!(matches!(
            server_finished_rx.recv_timeout(Duration::from_millis(500)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ));
        assert!(record_path.exists());

        drop_release_tx.send(()).unwrap();
        assert!(read_test_response(&mut request).ok);
        server_finished_rx
            .recv_timeout(Duration::from_secs(1))
            .unwrap()
            .unwrap();
        joiner.join().unwrap();
        assert!(!record_path.exists());
        cleanup(&context);
    }

    #[test]
    fn workspace_service_control_path_disconnect_cancels_only_its_operation() {
        let (context, record, _runtime, executor, server) =
            workspace_control_test_server("control-disconnect");
        let disconnected = open_test_request(
            &record,
            ServiceRequestKind::RlmReady {
                operation_id: "blocked-disconnected".to_string(),
                args: json!({}),
                timeout_seconds: 5,
                timeout_nanos: 0,
            },
        );
        executor.wait_started(1);
        drop(disconnected);
        executor.wait_cancelled(1);
        assert_eq!(
            executor.state.lock().unwrap().cancelled.as_slice(),
            &["blocked-disconnected".to_string()]
        );

        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let recovered = send_test_request(
                &record,
                ServiceRequestKind::RlmReady {
                    operation_id: "success-after-disconnect".to_string(),
                    args: json!({}),
                    timeout_seconds: 5,
                    timeout_nanos: 0,
                },
            );
            if recovered.ok {
                break;
            }
            assert!(Instant::now() < deadline);
        }

        assert!(send_test_request(&record, ServiceRequestKind::Shutdown).ok);
        server.join().unwrap().unwrap();
        cleanup(&context);
    }

    #[test]
    fn workspace_service_control_path_analyzer_wait_observes_operation_token() {
        let (_tx, rx) = mpsc::channel::<BslReaderEvent>();
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        let started = Instant::now();
        let deadline = ElapsedBudgetDeadline::new(Duration::from_secs(30));

        let error = read_json_response_cancellable(&rx, 7, &deadline, &cancellation).unwrap_err();

        assert!(error.starts_with("cancelled:"));
        assert!(started.elapsed() < Duration::from_millis(500));
    }

    #[test]
    fn workspace_service_control_path_rejects_duplicate_active_operation_id() {
        let (context, record, _runtime, executor, server) =
            workspace_control_test_server("control-duplicate");
        let mut first = open_test_request(
            &record,
            ServiceRequestKind::RlmReady {
                operation_id: "duplicate-id".to_string(),
                args: json!({}),
                timeout_seconds: 5,
                timeout_nanos: 0,
            },
        );
        executor.wait_started(1);

        let duplicate = send_test_request(
            &record,
            ServiceRequestKind::RlmReady {
                operation_id: "duplicate-id".to_string(),
                args: json!({}),
                timeout_seconds: 5,
                timeout_nanos: 0,
            },
        );
        assert!(!duplicate.ok);
        assert!(duplicate.error.unwrap().contains("already active"));

        assert!(
            send_test_request(
                &record,
                ServiceRequestKind::Cancel {
                    operation_id: "duplicate-id".to_string(),
                },
            )
            .ok
        );
        assert!(read_test_response(&mut first)
            .error
            .unwrap()
            .starts_with("cancelled:"));
        assert!(send_test_request(&record, ServiceRequestKind::Shutdown).ok);
        server.join().unwrap().unwrap();
        cleanup(&context);
    }

    #[test]
    fn workspace_service_control_path_drains_disconnected_worker_before_cleanup() {
        let (context, record, runtime, executor, server) =
            workspace_control_test_server("control-disconnected-drain");
        let disconnected = open_test_request(
            &record,
            ServiceRequestKind::RlmReady {
                operation_id: "held-after-cancel-1".to_string(),
                args: json!({}),
                timeout_seconds: 5,
                timeout_nanos: 0,
            },
        );
        executor.wait_started(1);
        drop(disconnected);
        executor.wait_cancelled(1);
        assert!(send_test_request(&record, ServiceRequestKind::Shutdown).ok);

        let (joined_tx, joined_rx) = mpsc::channel();
        let joiner = thread::spawn(move || {
            let result = server.join().unwrap();
            let _ = joined_tx.send(result);
        });
        assert!(joined_rx.recv_timeout(Duration::from_millis(150)).is_err());
        executor.release_cancelled();
        joined_rx
            .recv_timeout(Duration::from_secs(2))
            .unwrap()
            .unwrap();
        joiner.join().unwrap();

        assert!(runtime.operations.lock().unwrap().is_empty());
        cleanup(&context);
    }

    #[test]
    fn workspace_service_control_path_cancels_analyzer_waiter_and_shutdown_releases_lane() {
        let executor = Arc::new(AnalyzerLaneExecutor::default());
        let (context, record, runtime, server) = workspace_test_server_with_executor(
            "analyzer-lane-cancel",
            executor.clone(),
            Duration::from_secs(2),
        );
        let mut holder = open_test_request(
            &record,
            ServiceRequestKind::BslMcp {
                operation_id: "analyzer-holder-1".to_string(),
                tool_name: "hold".to_string(),
                tool_args: json!({}),
                timeout_seconds: 30,
                timeout_nanos: 0,
            },
        );
        executor.wait_holder();
        let mut waiter = open_test_request(
            &record,
            ServiceRequestKind::BslMcp {
                operation_id: "analyzer-waiter-2".to_string(),
                tool_name: "wait".to_string(),
                tool_args: json!({}),
                timeout_seconds: 30,
                timeout_nanos: 0,
            },
        );
        let cancel_started = Instant::now();
        assert!(
            send_test_request(
                &record,
                ServiceRequestKind::Cancel {
                    operation_id: "analyzer-waiter-2".to_string(),
                },
            )
            .ok
        );
        let cancelled_waiter = read_test_response(&mut waiter);
        assert!(cancelled_waiter.error.unwrap().starts_with("cancelled:"));
        assert!(cancel_started.elapsed() < Duration::from_millis(500));

        assert!(send_test_request(&record, ServiceRequestKind::Shutdown).ok);
        assert!(read_test_response(&mut holder)
            .error
            .unwrap()
            .starts_with("cancelled:"));
        server.join().unwrap().unwrap();
        assert!(runtime.operations.lock().unwrap().is_empty());
        cleanup(&context);
    }

    #[test]
    fn runtime_serializes_rlm_operations_on_one_lane() {
        let context = test_context("rlm-lane-serialization");
        let identity =
            WorkspaceServiceIdentity::new(&context, &context.workspace_root.join("src")).unwrap();
        let record = test_record(&identity, 34567, env!("CARGO_PKG_VERSION"));
        let runtime = Arc::new(WorkspaceServiceRuntime::new(identity, &record));
        let first = runtime.acquire_rlm_lane(&CancellationToken::new()).unwrap();
        let second_runtime = Arc::clone(&runtime);
        let (acquired_tx, acquired_rx) = mpsc::channel();
        let waiter = thread::spawn(move || {
            let cancellation = CancellationToken::new();
            let _second = second_runtime.acquire_rlm_lane(&cancellation).unwrap();
            acquired_tx.send(()).unwrap();
        });

        assert!(acquired_rx
            .recv_timeout(Duration::from_millis(100))
            .is_err());
        drop(first);
        acquired_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        waiter.join().unwrap();
        cleanup(&context);
    }

    #[test]
    fn bsl_operation_times_out_while_analyzer_lane_remains_held() {
        let context = test_context("bsl-lane-deadline");
        let source_root = context.workspace_root.join("src");
        let identity = WorkspaceServiceIdentity::new(&context, &source_root).unwrap();
        let record = test_record(&identity, 34567, env!("CARGO_PKG_VERSION"));
        let runtime = Arc::new(WorkspaceServiceRuntime::new(identity, &record));
        let holder = runtime
            .analyzer_lane
            .acquire(&CancellationToken::new())
            .unwrap();
        let waiter_runtime = Arc::clone(&runtime);
        let (response_tx, response_rx) = mpsc::channel();
        let waiter = thread::spawn(move || {
            response_tx
                .send(waiter_runtime.handle_bsl_mcp(
                    "diagnostics_status",
                    json!({}),
                    0,
                    50_000_000,
                    &CancellationToken::new(),
                ))
                .unwrap();
        });

        let response = match response_rx.recv_timeout(Duration::from_secs(2)) {
            Ok(response) => response,
            Err(error) => {
                drop(holder);
                let late_response = response_rx.recv_timeout(Duration::from_secs(2)).ok();
                waiter.join().unwrap();
                drop(runtime);
                cleanup(&context);
                panic!("analyzer lane wait exceeded its budget: {error}; {late_response:?}");
            }
        };

        assert!(
            response
                .error
                .as_deref()
                .is_some_and(|error| error.starts_with("timeout:")),
            "{response:?}"
        );
        drop(holder);
        waiter.join().unwrap();
        drop(runtime);
        cleanup(&context);
    }

    #[test]
    fn rlm_operation_times_out_while_rlm_lane_remains_held() {
        let context = test_context("rlm-lane-deadline");
        let source_root = context.workspace_root.join("src");
        let identity = WorkspaceServiceIdentity::new(&context, &source_root).unwrap();
        let record = test_record(&identity, 34567, env!("CARGO_PKG_VERSION"));
        let runtime = Arc::new(WorkspaceServiceRuntime::new(identity, &record));
        let holder = runtime.rlm_lane.acquire(&CancellationToken::new()).unwrap();
        let waiter_runtime = Arc::clone(&runtime);
        let (response_tx, response_rx) = mpsc::channel();
        let waiter = thread::spawn(move || {
            response_tx
                .send(waiter_runtime.handle_rlm_mcp(
                    WorkspaceRlmOperation::Search {
                        query: "Тест".to_string(),
                        limit: 20,
                    },
                    0,
                    50_000_000,
                    &CancellationToken::new(),
                ))
                .unwrap();
        });

        let response = match response_rx.recv_timeout(Duration::from_secs(2)) {
            Ok(response) => response,
            Err(error) => {
                drop(holder);
                let late_response = response_rx.recv_timeout(Duration::from_secs(2)).ok();
                waiter.join().unwrap();
                drop(runtime);
                cleanup(&context);
                panic!("RLM lane wait exceeded its budget: {error}; {late_response:?}");
            }
        };

        assert!(
            response
                .error
                .as_deref()
                .is_some_and(|error| error.starts_with("timeout:")),
            "{response:?}"
        );
        drop(holder);
        waiter.join().unwrap();
        drop(runtime);
        cleanup(&context);
    }

    #[test]
    fn runtime_tracks_session_teardown_until_shutdown_drain() {
        struct DropSignal(Option<mpsc::Sender<()>>);
        impl Drop for DropSignal {
            fn drop(&mut self) {
                self.0.take().unwrap().send(()).unwrap();
            }
        }

        let context = test_context("tracked-session-teardown");
        let identity =
            WorkspaceServiceIdentity::new(&context, &context.workspace_root.join("src")).unwrap();
        let record = test_record(&identity, 34567, env!("CARGO_PKG_VERSION"));
        let runtime = WorkspaceServiceRuntime::new(identity, &record);
        let (dropped_tx, dropped_rx) = mpsc::channel();

        runtime.retire_session(DropSignal(Some(dropped_tx)));

        assert!(runtime.shutdown_provider_sessions());
        dropped_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        cleanup(&context);
    }

    #[test]
    fn runtime_shutdown_drains_cancelled_rlm_maintenance() {
        let context = test_context("tracked-rlm-maintenance");
        let identity =
            WorkspaceServiceIdentity::new(&context, &context.workspace_root.join("src")).unwrap();
        let record = test_record(&identity, 34567, env!("CARGO_PKG_VERSION"));
        let mut runtime = WorkspaceServiceRuntime::new(identity, &record);
        let (started_tx, started_rx) = mpsc::channel();
        let (cancelled_tx, cancelled_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let release_rx = Mutex::new(release_rx);
        runtime.rlm_maintenance_requester = Arc::new(
            move |_context, _source_root, _revision, _revision_service, cancellation| {
                started_tx.send(()).unwrap();
                while !cancellation.is_cancelled() {
                    thread::sleep(Duration::from_millis(5));
                }
                cancelled_tx.send(()).unwrap();
                release_rx.lock().unwrap().recv().unwrap();
            },
        );
        let revision_service = runtime.rlm_source_revision_service().unwrap();
        let revision = SourceRevision {
            generation: 1,
            digest: "0".repeat(64),
            algorithm: crate::domain::source_revision::SOURCE_REVISION_ALGORITHM.to_string(),
        };
        let runtime = Arc::new(runtime);

        assert!(runtime
            .request_rlm_index_maintenance(revision, revision_service, &CancellationToken::new(),)
            .is_empty());
        started_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        runtime.begin_shutdown();
        cancelled_rx.recv_timeout(Duration::from_secs(1)).unwrap();

        let shutdown_runtime = Arc::clone(&runtime);
        let (drained_tx, drained_rx) = mpsc::channel();
        let shutdown = thread::spawn(move || {
            drained_tx
                .send(shutdown_runtime.shutdown_provider_sessions())
                .unwrap();
        });
        assert!(matches!(
            drained_rx.recv_timeout(Duration::from_millis(100)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ));

        release_tx.send(()).unwrap();
        assert!(drained_rx.recv_timeout(Duration::from_secs(1)).unwrap());
        shutdown.join().unwrap();
        drop(runtime);
        cleanup(&context);
    }

    #[test]
    fn runtime_shutdown_joins_rlm_maintenance_registered_after_drain_started() {
        let context = test_context("late-rlm-maintenance-registration");
        let identity =
            WorkspaceServiceIdentity::new(&context, &context.workspace_root.join("src")).unwrap();
        let record = test_record(&identity, 34567, env!("CARGO_PKG_VERSION"));
        let runtime = Arc::new(WorkspaceServiceRuntime::new(identity, &record));
        let (allow_registration_tx, allow_registration_rx) = mpsc::channel();
        let (task_release_tx, task_release_rx) = mpsc::channel();
        let (registration_finished_tx, registration_finished_rx) = mpsc::channel();
        let tracker = Arc::clone(&runtime.rlm_maintenance_tasks);
        let registrar = thread::spawn(move || {
            allow_registration_rx.recv().unwrap();
            let handle = thread::spawn(move || task_release_rx.recv().unwrap());
            tracker.track(handle);
            registration_finished_tx.send(()).unwrap();
        });

        runtime.begin_shutdown();
        assert!(runtime.shutdown_provider_sessions());
        allow_registration_tx.send(()).unwrap();
        assert!(matches!(
            registration_finished_rx.recv_timeout(Duration::from_millis(100)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ));

        task_release_tx.send(()).unwrap();
        registration_finished_rx
            .recv_timeout(Duration::from_secs(1))
            .unwrap();
        registrar.join().unwrap();
        drop(runtime);
        cleanup(&context);
    }

    #[test]
    fn workspace_service_worker_panic_cleans_registry_and_next_request_succeeds() {
        let (context, record, runtime, _executor, server) =
            workspace_control_test_server("worker-panic");
        let panic_response = send_test_request(
            &record,
            ServiceRequestKind::RlmReady {
                operation_id: "panic-worker-1".to_string(),
                args: json!({}),
                timeout_seconds: 5,
                timeout_nanos: 0,
            },
        );
        assert!(!panic_response.ok);
        assert!(panic_response
            .error
            .unwrap()
            .contains("worker disconnected"));
        assert!(runtime.operations.lock().unwrap().is_empty());
        assert!(
            send_test_request(
                &record,
                ServiceRequestKind::RlmReady {
                    operation_id: "success-after-panic".to_string(),
                    args: json!({}),
                    timeout_seconds: 5,
                    timeout_nanos: 0,
                },
            )
            .ok
        );
        assert!(send_test_request(&record, ServiceRequestKind::Shutdown).ok);
        server.join().unwrap().unwrap();
        cleanup(&context);
    }

    #[test]
    fn workspace_service_cleanup_preserves_replacement_record() {
        let context = test_context("cleanup-owner-race");
        let identity =
            WorkspaceServiceIdentity::new(&context, &context.workspace_root.join("src")).unwrap();
        let original = test_record(&identity, 31001, env!("CARGO_PKG_VERSION"));
        write_record(&identity, original.clone());
        let runtime = WorkspaceServiceRuntime::new(identity.clone(), &original);
        let mut replacement = original;
        replacement.token = "replacement-token".to_string();
        replacement.pid = replacement.pid.saturating_add(1);
        replacement.started_at = replacement.started_at.saturating_add(1);
        write_record(&identity, replacement.clone());

        remove_service_record_if_owned(&runtime);

        assert_eq!(read_record(&identity).unwrap().token, replacement.token);
        cleanup(&context);
    }

    #[test]
    fn workspace_service_record_cleanup_serializes_concurrent_replacement() {
        let context = test_context("cleanup-serialized-race");
        let identity =
            WorkspaceServiceIdentity::new(&context, &context.workspace_root.join("src")).unwrap();
        let original = test_record(&identity, 31002, env!("CARGO_PKG_VERSION"));
        write_record(&identity, original.clone());
        let runtime = Arc::new(WorkspaceServiceRuntime::new(identity.clone(), &original));
        let mut replacement = original;
        replacement.token = "serialized-replacement".to_string();
        replacement.started_at = replacement.started_at.saturating_add(1);
        let (inside_tx, inside_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let cleanup_runtime = runtime.clone();
        let cleanup_thread = thread::spawn(move || {
            remove_service_record_if_owned_with_hook(&cleanup_runtime, || {
                inside_tx.send(()).unwrap();
                release_rx.recv().unwrap();
            });
        });
        inside_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let replacement_identity = identity.clone();
        let expected = replacement.clone();
        let (writer_inside_tx, writer_inside_rx) = mpsc::channel();
        let writer = thread::spawn(move || {
            write_record_with_hook(&replacement_identity, &replacement, || {
                writer_inside_tx.send(()).unwrap();
            })
            .unwrap();
        });
        assert!(writer_inside_rx
            .recv_timeout(Duration::from_millis(100))
            .is_err());
        release_tx.send(()).unwrap();
        cleanup_thread.join().unwrap();
        writer_inside_rx
            .recv_timeout(Duration::from_secs(2))
            .unwrap();
        writer.join().unwrap();

        assert_eq!(read_record(&identity).unwrap(), expected);
        assert!(identity
            .service_dir
            .join(SERVICE_RECORD_LOCK_FILE)
            .is_file());
        cleanup(&context);
    }

    #[test]
    fn workspace_service_record_lock_survives_holder_panic_and_is_reusable() {
        let context = test_context("record-lock-panic");
        let identity =
            WorkspaceServiceIdentity::new(&context, &context.workspace_root.join("src")).unwrap();
        let panic_identity = identity.clone();
        let _ = thread::spawn(move || {
            let _lock = acquire_record_lock(&panic_identity).unwrap();
            panic!("intentional record lock holder panic");
        })
        .join();
        let record = test_record(&identity, 31004, env!("CARGO_PKG_VERSION"));

        super::write_record(&identity, &record).unwrap();

        assert_eq!(read_record(&identity).unwrap(), record);
        assert!(identity
            .service_dir
            .join(SERVICE_RECORD_LOCK_FILE)
            .is_file());
        cleanup(&context);
    }

    #[test]
    fn workspace_service_last_access_update_cannot_overwrite_replacement() {
        let context = test_context("last-access-serialized-race");
        let identity =
            WorkspaceServiceIdentity::new(&context, &context.workspace_root.join("src")).unwrap();
        let original = test_record(&identity, 31003, env!("CARGO_PKG_VERSION"));
        write_record(&identity, original.clone());
        let runtime = Arc::new(WorkspaceServiceRuntime::new(identity.clone(), &original));
        let mut replacement = original;
        replacement.token = "last-access-replacement".to_string();
        replacement.started_at = replacement.started_at.saturating_add(1);
        let expected = replacement.clone();
        let (inside_tx, inside_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let update_runtime = runtime.clone();
        let updater = thread::spawn(move || {
            update_service_record_last_access_with_hook(&update_runtime, 999, || {
                inside_tx.send(()).unwrap();
                release_rx.recv().unwrap();
            });
        });
        inside_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let replacement_identity = identity.clone();
        let (writer_inside_tx, writer_inside_rx) = mpsc::channel();
        let writer = thread::spawn(move || {
            write_record_with_hook(&replacement_identity, &replacement, || {
                writer_inside_tx.send(()).unwrap();
            })
            .unwrap();
        });
        assert!(writer_inside_rx
            .recv_timeout(Duration::from_millis(100))
            .is_err());
        release_tx.send(()).unwrap();
        updater.join().unwrap();
        writer_inside_rx
            .recv_timeout(Duration::from_secs(2))
            .unwrap();
        writer.join().unwrap();

        assert_eq!(read_record(&identity).unwrap(), expected);
        cleanup(&context);
    }

    #[test]
    fn analyzer_lane_is_fifo_and_cancelled_middle_ticket_advances_queue() {
        let lane = Arc::new(AnalyzerLane::default());
        let holder = lane.acquire(&CancellationToken::new()).unwrap();
        let (queued_tx, queued_rx) = mpsc::channel();
        let (acquired_tx, acquired_rx) = mpsc::channel();
        let middle_cancellation = CancellationToken::new();
        let mut waiters = Vec::new();
        for id in [1_u8, 2, 3] {
            let lane = lane.clone();
            let queued_tx = queued_tx.clone();
            let acquired_tx = acquired_tx.clone();
            let cancellation = if id == 2 {
                middle_cancellation.clone()
            } else {
                CancellationToken::new()
            };
            waiters.push(thread::spawn(move || {
                let result = lane.acquire_with_hook(&cancellation, || {
                    queued_tx.send(id).unwrap();
                });
                match result {
                    Ok(_permit) => acquired_tx.send((id, "acquired")).unwrap(),
                    Err(error) => {
                        assert!(error.starts_with("cancelled:"));
                        acquired_tx.send((id, "cancelled")).unwrap();
                    }
                }
            }));
            assert_eq!(queued_rx.recv_timeout(Duration::from_secs(2)).unwrap(), id);
        }
        middle_cancellation.cancel();
        assert_eq!(
            acquired_rx.recv_timeout(Duration::from_secs(2)).unwrap(),
            (2, "cancelled")
        );
        drop(holder);
        assert_eq!(
            acquired_rx.recv_timeout(Duration::from_secs(2)).unwrap(),
            (1, "acquired")
        );
        assert_eq!(
            acquired_rx.recv_timeout(Duration::from_secs(2)).unwrap(),
            (3, "acquired")
        );
        for waiter in waiters {
            waiter.join().unwrap();
        }
    }

    #[test]
    fn analyzer_lane_recovers_from_poison_without_losing_progress() {
        let lane = Arc::new(AnalyzerLane::default());
        let poison_lane = lane.clone();
        let _ = thread::spawn(move || {
            let _state = poison_lane.state.lock().unwrap();
            panic!("intentional analyzer queue poison");
        })
        .join();

        let permit = lane.acquire(&CancellationToken::new()).unwrap();
        drop(permit);
        let second = lane.acquire(&CancellationToken::new()).unwrap();
        drop(second);
    }

    #[test]
    fn workspace_service_zero_grace_cutoff_preserves_new_owner_record() {
        let executor = Arc::new(BlockingWorkspaceExecutor::default());
        let (context, record, runtime, server) = workspace_test_server_with_executor(
            "zero-grace-owner",
            executor.clone(),
            Duration::ZERO,
        );
        let disconnected = open_test_request(
            &record,
            ServiceRequestKind::RlmReady {
                operation_id: "held-after-cancel-zero-grace".to_string(),
                args: json!({}),
                timeout_seconds: 5,
                timeout_nanos: 0,
            },
        );
        executor.wait_started(1);
        drop(disconnected);
        executor.wait_cancelled(1);
        let identity = runtime.identity.clone();
        let mut replacement = record.clone();
        replacement.token = "new-owner".to_string();
        replacement.pid = replacement.pid.saturating_add(1);
        replacement.started_at = replacement.started_at.saturating_add(1);
        write_record(&identity, replacement.clone());
        assert!(send_test_request(&record, ServiceRequestKind::Shutdown).ok);

        let started = Instant::now();
        server.join().unwrap().unwrap();
        assert!(started.elapsed() < Duration::from_millis(500));
        assert_eq!(read_record(&identity).unwrap().token, replacement.token);
        executor.release_cancelled();
        cleanup(&context);
    }

    #[test]
    fn bsl_session_initialize_uses_operation_cancellation_for_stuck_and_fragmented_output() {
        let context = test_context("cancelled-initialize");
        let fixture = compile_initialize_fixture(&context);
        for mode in ["stuck", "fragmented"] {
            let mut command = Command::new(&fixture);
            command.arg(mode);
            let cancellation = CancellationToken::new();
            let cancel = cancellation.clone();
            let canceller = thread::spawn(move || {
                thread::sleep(Duration::from_millis(100));
                cancel.cancel();
            });
            let started = Instant::now();
            let error = match PersistentMcpSession::start_with_command(command, &cancellation) {
                Ok(_) => panic!("initialize unexpectedly completed"),
                Err(error) => error,
            };
            canceller.join().unwrap();
            assert!(error.starts_with("cancelled:"), "{error}");
            assert!(started.elapsed() < Duration::from_secs(2));
        }
        let pid_file = context.cache_root.join("incomplete-eof-child-pid.txt");
        let read_marker = context
            .cache_root
            .join("incomplete-eof-initialize-read.txt");
        let mut command = Command::new(&fixture);
        command.args([
            "incomplete-eof",
            pid_file.to_str().unwrap(),
            read_marker.to_str().unwrap(),
        ]);
        let started = Instant::now();
        let error = PersistentMcpSession::start_with_command(command, &CancellationToken::new())
            .err()
            .expect("incomplete initialize response must fail");
        let process_pid = wait_for_recorded_pid(&pid_file);
        assert_eq!(fs::read_to_string(&read_marker).unwrap(), "initialize-read");
        assert!(error.contains("incomplete JSON line"), "{error}");
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(wait_for_process_exit(process_pid, Duration::from_secs(2)));
        cleanup(&context);
    }

    #[test]
    fn cancelled_bsl_session_terminates_process_tree_without_blocking_drop() {
        let context = test_context("cancelled-session-tree");
        let fixture = compile_session_tree_fixture(&context);
        let pid_file = context.cache_root.join("session-tree-pids.txt");
        let mut command = Command::new(&fixture);
        command.arg(&pid_file);
        let cancellation = CancellationToken::new();
        let mut session = PersistentMcpSession::start_with_command(command, &cancellation).unwrap();
        let parent_pid = session.child.id();
        let child_pid = wait_for_recorded_pid(&pid_file);
        let cancel = cancellation.clone();
        let canceller = thread::spawn(move || {
            thread::sleep(Duration::from_millis(100));
            cancel.cancel();
        });

        let error = session
            .call(
                "unica.code.search",
                json!({"query": "x".repeat(16 * 1024 * 1024)}),
                Duration::from_secs(30),
                &cancellation,
            )
            .unwrap_err();
        assert!(error.starts_with("cancelled:"), "{error}");
        let drop_started = Instant::now();
        drop(session);
        canceller.join().unwrap();

        assert!(drop_started.elapsed() < Duration::from_secs(2));
        assert!(wait_for_process_exit(parent_pid, Duration::from_secs(2)));
        assert!(wait_for_process_exit(child_pid, Duration::from_secs(2)));
        cleanup(&context);
    }

    struct ChunkedReader {
        bytes: std::io::Cursor<Vec<u8>>,
        chunk_size: usize,
    }

    impl Read for ChunkedReader {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            let limit = buffer.len().min(self.chunk_size);
            self.bytes.read(&mut buffer[..limit])
        }
    }

    #[test]
    fn bsl_session_stdout_framing_accepts_fragments_and_rejects_bad_or_oversized_lines() {
        let (tx, rx) = mpsc::sync_channel(8);
        bsl_reader(
            ChunkedReader {
                bytes: std::io::Cursor::new(
                    b"{\"jsonrpc\":\"2.0\",\"method\":\"notifications/progress\"}\n{\"jsonrpc\":\"2.0\",\"id\":7,\"result\":{}}\n".to_vec(),
                ),
                chunk_size: 3,
            },
            tx,
        );
        let deadline = ElapsedBudgetDeadline::new(Duration::from_secs(1));
        let response =
            read_json_response_cancellable(&rx, 7, &deadline, &CancellationToken::new()).unwrap();
        assert_eq!(response["id"], 7);

        let (tx, rx) = mpsc::sync_channel(8);
        bsl_reader(std::io::Cursor::new(b"not-json\n"), tx);
        assert!(matches!(
            rx.recv().unwrap(),
            BslReaderEvent::ProtocolError(error) if error.contains("malformed JSON")
        ));

        let (tx, rx) = mpsc::sync_channel(8);
        let terminal = Arc::new(AtomicBool::new(false));
        bsl_reader_with_state(
            std::io::Cursor::new(vec![b'x'; SERVICE_RESPONSE_LINE_LIMIT + 1]),
            tx,
            Arc::clone(&terminal),
        );
        assert!(matches!(
            rx.recv().unwrap(),
            BslReaderEvent::ProtocolError(error) if error.contains("exceeds")
        ));
        assert!(terminal.load(Ordering::Acquire));
    }

    #[test]
    fn bsl_session_incomplete_eof_is_an_immediate_protocol_error() {
        let (tx, rx) = mpsc::sync_channel(8);
        bsl_reader(std::io::Cursor::new(b"{\"jsonrpc\":\"2.0\""), tx);
        let started = Instant::now();
        let deadline = ElapsedBudgetDeadline::new(Duration::from_secs(30));
        let error = read_json_response_cancellable(&rx, 1, &deadline, &CancellationToken::new())
            .unwrap_err();
        assert!(error.contains("incomplete JSON line"), "{error}");
        assert!(started.elapsed() < Duration::from_millis(100));
    }

    #[test]
    fn bsl_session_stderr_reader_keeps_only_a_bounded_lossy_tail() {
        let mut noisy = vec![b'x'; BSL_STDERR_TAIL_LIMIT * 4];
        noisy.extend_from_slice(&[0xff, 0xfe]);
        noisy.extend_from_slice(b"TAIL-MARKER");
        let tail = Arc::new(Mutex::new(BoundedByteTail::new(BSL_STDERR_TAIL_LIMIT)));
        let started = Instant::now();
        bsl_stderr_reader(std::io::Cursor::new(noisy), Arc::clone(&tail));
        let snapshot = tail.lock().unwrap().snapshot();
        assert!(snapshot.len() <= BSL_STDERR_TAIL_LIMIT);
        assert!(snapshot.ends_with("TAIL-MARKER"), "{snapshot}");
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn bsl_session_cancellation_wins_over_protocol_and_eof_races() {
        for event in [
            BslReaderEvent::ProtocolError("bad response".to_string()),
            BslReaderEvent::Closed,
        ] {
            let (tx, rx) = mpsc::channel();
            tx.send(event).unwrap();
            let cancellation = CancellationToken::new();
            cancellation.cancel();
            let deadline = ElapsedBudgetDeadline::new(Duration::from_secs(1));
            let error =
                read_json_response_cancellable(&rx, 7, &deadline, &cancellation).unwrap_err();
            assert!(error.starts_with("cancelled:"), "{error}");
        }
    }

    #[test]
    fn bsl_session_replaces_dead_warm_process_before_next_request() {
        let context = test_context("dead-warm-session");
        let fixture = compile_exit_after_call_fixture(&context);
        let pid_file = context.cache_root.join("warm-session-pids.txt");
        let completion_file = context.cache_root.join("warm-session-completed.txt");
        let source_root = context.workspace_root.join("src");
        fs::create_dir_all(&source_root).unwrap();
        let identity = WorkspaceServiceIdentity::new(&context, &source_root).unwrap();
        let record = test_record(&identity, 1, env!("CARGO_PKG_VERSION"));
        let mut runtime = WorkspaceServiceRuntime::new(identity, &record);
        let fixture_for_start = fixture.clone();
        let pid_file_for_start = pid_file.clone();
        let completion_file_for_start = completion_file.clone();
        runtime.analyzer_starter = Arc::new(move |_context, _source_root, cancellation| {
            let mut command = Command::new(&fixture_for_start);
            command.args([&pid_file_for_start, &completion_file_for_start]);
            PersistentMcpSession::start_with_command(command, cancellation)
        });
        let first = runtime.handle_bsl_mcp(
            "unica.code.search",
            json!({}),
            2,
            0,
            &CancellationToken::new(),
        );
        assert!(first.ok, "{:?}", first.error);
        let first_pid = wait_for_recorded_pid(&pid_file);
        wait_for_file(&completion_file, Duration::from_secs(2));
        wait_for_runtime_reader_terminal(&runtime, Duration::from_secs(2));

        let second = runtime.handle_bsl_mcp(
            "unica.code.search",
            json!({}),
            2,
            0,
            &CancellationToken::new(),
        );
        assert!(second.ok, "{:?}", second.error);
        assert!(wait_for_process_exit(first_pid, Duration::from_secs(2)));
        let pids = fs::read_to_string(&pid_file).unwrap();
        let pids = pids.lines().collect::<Vec<_>>();
        assert_eq!(pids.len(), 2, "{pids:?}");
        assert_ne!(pids[0], pids[1]);
        cleanup(&context);
    }

    #[test]
    fn bsl_session_replaces_reader_terminal_session_before_next_request() {
        let context = test_context(&format!("terminal-warm-session-{}", Uuid::new_v4()));
        let fixture = compile_terminal_after_call_fixture(&context);
        let pid_file = context.cache_root.join("terminal-session-pids.txt");
        let terminal_marker = context.cache_root.join("terminal-session-marker.txt");
        let source_root = context.workspace_root.join("src");
        fs::create_dir_all(&source_root).unwrap();
        let identity = WorkspaceServiceIdentity::new(&context, &source_root).unwrap();
        let record = test_record(&identity, 1, env!("CARGO_PKG_VERSION"));
        let mut runtime = WorkspaceServiceRuntime::new(identity, &record);
        runtime.analyzer_starter = Arc::new({
            let fixture = fixture.clone();
            let pid_file = pid_file.clone();
            let terminal_marker = terminal_marker.clone();
            move |_context, _source_root, cancellation| {
                let mut command = Command::new(&fixture);
                command.args([&pid_file, &terminal_marker]);
                PersistentMcpSession::start_with_command(command, cancellation)
            }
        });

        let first = runtime.handle_bsl_mcp(
            "unica.code.search",
            json!({}),
            2,
            0,
            &CancellationToken::new(),
        );
        assert!(first.ok, "{:?}", first.error);
        let first_pid = wait_for_recorded_pid(&pid_file);
        wait_for_file(&terminal_marker, Duration::from_secs(5));
        wait_for_runtime_reader_terminal(&runtime, Duration::from_secs(5));

        let second = runtime.handle_bsl_mcp(
            "unica.code.search",
            json!({}),
            2,
            0,
            &CancellationToken::new(),
        );
        assert!(second.ok, "{:?}", second.error);
        assert!(wait_for_process_exit(first_pid, Duration::from_secs(2)));
        let pids = fs::read_to_string(&pid_file).unwrap();
        let pids = pids.lines().collect::<Vec<_>>();
        assert_eq!(pids.len(), 2, "{pids:?}");
        assert_ne!(pids[0], pids[1]);
        cleanup(&context);
    }

    fn compile_terminal_after_call_fixture(context: &WorkspaceContext) -> PathBuf {
        let source = context.cache_root.join("terminal-after-call-fixture.rs");
        let executable =
            testing::fixture_executable_path(&context.cache_root, "terminal-after-call-fixture");
        fs::create_dir_all(&context.cache_root).unwrap();
        fs::write(
            &source,
            r#"use std::{env, fs::{self, OpenOptions}, io::{self, BufRead, Write}, thread, time::Duration};
fn main() {
    let pid_file = env::args().nth(1).unwrap();
    let marker = env::args().nth(2).unwrap();
    let first = fs::read_to_string(&pid_file).unwrap_or_default().lines().count() == 0;
    writeln!(OpenOptions::new().create(true).append(true).open(&pid_file).unwrap(), "{}", std::process::id()).unwrap();
    for (index, line) in io::stdin().lock().lines().enumerate() {
        line.unwrap();
        match index {
            0 => println!("{{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{{}}}}"),
            2 => {
                println!("{{\"jsonrpc\":\"2.0\",\"id\":2,\"result\":{{\"content\":[{{\"type\":\"text\",\"text\":\"ok\"}}]}}}}");
                io::stdout().flush().unwrap();
                if first {
                    println!("malformed-after-success");
                    io::stdout().flush().unwrap();
                    fs::write(&marker, b"malformed-written").unwrap();
                    thread::sleep(Duration::from_secs(30));
                }
            }
            _ => {}
        }
        io::stdout().flush().unwrap();
    }
}"#,
        )
        .unwrap();
        let status = Command::new("rustc")
            .arg(&source)
            .arg("-o")
            .arg(&executable)
            .status()
            .unwrap();
        assert!(status.success());
        executable
    }

    fn compile_exit_after_call_fixture(context: &WorkspaceContext) -> PathBuf {
        let source = context.cache_root.join("exit-after-call-fixture.rs");
        let executable =
            testing::fixture_executable_path(&context.cache_root, "exit-after-call-fixture");
        fs::create_dir_all(&context.cache_root).unwrap();
        fs::write(
            &source,
            r#"use std::{env, fs::OpenOptions, io::{self, BufRead, Write}};
fn main() {
    let pid_file = env::args().nth(1).unwrap();
    let completion_file = env::args().nth(2).unwrap();
    writeln!(OpenOptions::new().create(true).append(true).open(pid_file).unwrap(), "{}", std::process::id()).unwrap();
    for (index, line) in io::stdin().lock().lines().enumerate() {
        line.unwrap();
        match index {
            0 => println!("{{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{{}}}}"),
            2 => { println!("{{\"jsonrpc\":\"2.0\",\"id\":2,\"result\":{{\"content\":[{{\"type\":\"text\",\"text\":\"ok\"}}]}}}}"); io::stdout().flush().unwrap(); std::fs::write(completion_file, b"completed").unwrap(); break; }
            _ => {}
        }
        io::stdout().flush().unwrap();
    }
}"#,
        )
        .unwrap();
        let status = Command::new("rustc")
            .arg(&source)
            .arg("-o")
            .arg(&executable)
            .status()
            .unwrap();
        assert!(status.success());
        executable
    }

    fn compile_session_tree_fixture(context: &WorkspaceContext) -> PathBuf {
        let source = context.cache_root.join("session-tree-fixture.rs");
        let executable =
            testing::fixture_executable_path(&context.cache_root, "session-tree-fixture");
        fs::create_dir_all(&context.cache_root).unwrap();
        fs::write(
            &source,
            r#"use std::{env, fs, io::{self, BufRead, Write}, process::Command, thread, time::Duration};
fn main() {
    if env::args().nth(1).as_deref() == Some("child") {
        thread::sleep(Duration::from_secs(5));
        return;
    }
    let pid_file = env::args().nth(1).unwrap();
    let child = Command::new(env::current_exe().unwrap()).arg("child").spawn().unwrap();
    fs::write(pid_file, child.id().to_string()).unwrap();
    let stdin = io::stdin();
    for (index, line) in stdin.lock().lines().enumerate() {
        line.unwrap();
        if index == 0 {
            println!("{{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{{}}}}");
            io::stdout().flush().unwrap();
        } else {
            thread::sleep(Duration::from_secs(30));
        }
    }
}"#,
        )
        .unwrap();
        let status = Command::new("rustc")
            .arg(&source)
            .arg("-o")
            .arg(&executable)
            .status()
            .unwrap();
        assert!(status.success());
        executable
    }

    fn wait_for_recorded_pid(path: &Path) -> u32 {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if let Ok(text) = fs::read_to_string(path) {
                if let Some(pid) = text.lines().find_map(|line| line.trim().parse().ok()) {
                    return pid;
                }
            }
            assert!(
                Instant::now() < deadline,
                "fixture did not record child pid"
            );
            thread::sleep(Duration::from_millis(25));
        }
    }

    fn wait_for_file(path: &Path, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        while !path.is_file() {
            assert!(Instant::now() < deadline, "fixture marker was not written");
            thread::yield_now();
        }
    }

    fn wait_for_atomic_value(value: &AtomicUsize, expected: usize, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        while value.load(Ordering::Acquire) != expected {
            assert!(
                Instant::now() < deadline,
                "atomic value did not become {expected}"
            );
            thread::yield_now();
        }
    }

    fn wait_for_runtime_reader_terminal(runtime: &WorkspaceServiceRuntime, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        loop {
            let closed = runtime
                .analyzer
                .lock()
                .unwrap()
                .as_ref()
                .is_some_and(|session| session.reader_terminal.load(Ordering::Acquire));
            if closed {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "analyzer stdout reader terminal state was not observed"
            );
            thread::yield_now();
        }
    }

    fn wait_for_process_exit(pid: u32, timeout: Duration) -> bool {
        testing::wait_for_process_exit(pid, timeout)
    }

    fn write_ready_rlm_status_for_current_source(
        context: &WorkspaceContext,
        source_root: &Path,
    ) -> SourceRevision {
        let revision = SourceRevisionService::new(context, source_root)
            .unwrap()
            .snapshot(
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        let db_path = context.cache_root.join("rlm-tools-bsl/test/bsl_index.db");
        fs::create_dir_all(db_path.parent().unwrap()).unwrap();
        fs::write(&db_path, "ready index").unwrap();
        let status = crate::infrastructure::workspace_index::BslIndexStatus {
            status: "ready".to_string(),
            source_root: Some(source_root.display().to_string()),
            db_path: Some(db_path.display().to_string()),
            message: None,
            failure_class: None,
            source_generation: Some(revision.generation),
            indexed_revision: Some(revision.clone()),
            observed_revision: None,
            next_action: None,
            updated_at: now_secs_for_test(),
            last_run: None,
        };
        let status_path = crate::infrastructure::workspace_index::status_path(context);
        fs::create_dir_all(status_path.parent().unwrap()).unwrap();
        fs::write(
            status_path,
            serde_json::to_string_pretty(&status).unwrap() + "\n",
        )
        .unwrap();
        revision
    }

    fn write_active_rlm_index_lock(context: &WorkspaceContext, source_root: &Path) {
        let lock_path = context.cache_root.join("locks/bsl_index.lock");
        fs::create_dir_all(lock_path.parent().unwrap()).unwrap();
        let now = now_secs_for_test();
        fs::write(
            lock_path,
            serde_json::to_string_pretty(&json!({
                "schema_version": 1,
                "lock_id": "blocking-execute-lock",
                "owner_pid": std::process::id(),
                "action": "update",
                "source_root": source_root.display().to_string(),
                "started_at": now,
                "updated_at": now,
                "state": "active"
            }))
            .unwrap()
                + "\n",
        )
        .unwrap();
    }

    struct BlockingRlmObservation {
        response: ServiceResponse,
        session_retired: bool,
        maintenance_requests: usize,
        pre_execution_revision: SourceRevision,
        maintenance_revisions: Vec<SourceRevision>,
        teardown_drained: bool,
    }

    fn run_blocking_rlm_execute(
        name: &str,
        during_execute: impl FnOnce(&WorkspaceContext, &Path, &Path),
    ) -> BlockingRlmObservation {
        let context = test_context(name);
        let source_root = context.workspace_root.join("src");
        let module = source_root.join("CommonModules/SmokeModule.bsl");
        fs::write(&module, "Процедура Smoke()\nКонецПроцедуры\n").unwrap();
        let pre_execution_revision =
            write_ready_rlm_status_for_current_source(&context, &source_root);
        let fixture = blocking_rlm_fixture();
        fs::create_dir_all(&context.cache_root).unwrap();
        let execute_started = context.cache_root.join("blocking-rlm-execute-started.txt");
        let execute_release = context.cache_root.join("blocking-rlm-execute-release.txt");
        let identity = WorkspaceServiceIdentity::new(&context, &source_root).unwrap();
        let record = test_record(&identity, 1, env!("CARGO_PKG_VERSION"));
        let mut runtime = WorkspaceServiceRuntime::new(identity, &record);
        let maintenance_requests = Arc::new(AtomicUsize::new(0));
        let maintenance_revisions = Arc::new(Mutex::new(Vec::new()));
        runtime.rlm_maintenance_requester = Arc::new({
            let maintenance_requests = Arc::clone(&maintenance_requests);
            let maintenance_revisions = Arc::clone(&maintenance_revisions);
            move |_context, _source_root, revision, _revision_service, _cancellation| {
                maintenance_revisions.lock().unwrap().push(revision);
                maintenance_requests.fetch_add(1, Ordering::AcqRel);
            }
        });
        runtime.rlm_starter = Arc::new({
            let fixture = fixture.clone();
            let execute_started = execute_started.clone();
            let execute_release = execute_release.clone();
            move |_context, source_root, cancellation| {
                let mut command = Command::new(&fixture);
                command.args([&execute_started, &execute_release]);
                Ok(RlmMcpSession {
                    transport: PersistentMcpSession::start_with_command(command, cancellation)?,
                    source_root: source_root.to_path_buf(),
                    session_id: None,
                })
            }
        });
        let runtime = Arc::new(runtime);
        let caller_runtime = Arc::clone(&runtime);
        let caller = thread::spawn(move || {
            caller_runtime.handle_rlm_mcp(
                WorkspaceRlmOperation::Search {
                    query: "Smoke".to_string(),
                    limit: 20,
                },
                5,
                0,
                &CancellationToken::new(),
            )
        });

        // With the binary already built and validated, this wait measured 12
        // and 14 ms under the full parallel suite, against 1822 to 2243 ms
        // before. Five seconds is the bound the sibling terminal-marker wait
        // already uses, and over that distribution it is headroom of two and a
        // half orders of magnitude instead of the margin of tens of
        // milliseconds the previous two seconds left.
        wait_for_file(&execute_started, Duration::from_secs(5));
        during_execute(&context, &source_root, &module);
        fs::write(&execute_release, "release").unwrap();
        let response = caller.join().unwrap();
        let session_retired = runtime.rlm.lock().unwrap().is_none();
        wait_for_atomic_value(&maintenance_requests, 1, Duration::from_secs(2));
        let maintenance_requests = maintenance_requests.load(Ordering::Acquire);
        let maintenance_revisions = maintenance_revisions.lock().unwrap().clone();
        let teardown_drained = runtime.session_teardowns.drain(SESSION_TEARDOWN_GRACE);
        drop(runtime);
        cleanup(&context);

        BlockingRlmObservation {
            response,
            session_retired,
            maintenance_requests,
            pre_execution_revision,
            maintenance_revisions,
            teardown_drained,
        }
    }

    /// The helper binary is built and first run once per test process, before
    /// any test starts timing a spawn.
    ///
    /// Measured on this fixture under the full parallel suite: `rustc` costs
    /// ~100 ms, the JSON-RPC handshake to the first marker costs 0-1 ms, and
    /// everything else — 1.8 to 2.2 s — was the child getting from `spawn` to
    /// the first line of `main`. That gap is the first execution of a
    /// freshly written binary: macOS validates its code signature through a
    /// shared system service, and a suite that writes a new binary per call
    /// pays that serialized validation every time. Idle, the same binary needs
    /// ~300 ms on its first run and ~20 ms on every later one.
    ///
    /// Building once per process leaves exactly one validation, and running it
    /// here moves that validation out of the timed section. What a test waits
    /// on afterwards is the fixture's own progress, which is what the deadline
    /// is meant to bound.
    fn blocking_rlm_fixture() -> PathBuf {
        static FIXTURE: OnceLock<PathBuf> = OnceLock::new();
        FIXTURE
            .get_or_init(|| {
                let directory = std::env::temp_dir()
                    .join(format!("unica-blocking-rlm-fixture-{}", std::process::id()));
                let executable = compile_blocking_rlm_fixture(&directory);
                // The first run pays the code-signature validation; the fixture
                // exits on its own once stdin closes, so no marker is needed.
                let mut warmup = Command::new(&executable)
                    .arg(directory.join("warmup-started.txt"))
                    .arg(directory.join("warmup-release.txt"))
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                    .unwrap();
                warmup.wait().unwrap();
                executable
            })
            .clone()
    }

    fn compile_blocking_rlm_fixture(directory: &Path) -> PathBuf {
        let source = directory.join("blocking-rlm-fixture.rs");
        let executable = testing::fixture_executable_path(directory, "blocking-rlm-fixture");
        fs::create_dir_all(directory).unwrap();
        fs::write(
            &source,
            r##"use std::{env, fs, io::{self, BufRead, Write}, thread, time::{Duration, Instant}};
fn main() {
    let execute_started = env::args().nth(1).unwrap();
    let execute_release = env::args().nth(2).unwrap();
    for (index, line) in io::stdin().lock().lines().enumerate() {
        line.unwrap();
        match index {
            0 => println!("{}", r#"{"jsonrpc":"2.0","id":1,"result":{}}"#),
            2 => println!("{}", r#"{"jsonrpc":"2.0","id":2,"result":{"content":[{"type":"text","text":"{\"session_id\":\"session-1\",\"index\":{\"index_status\":\"ready\"}}"}]}}"#),
            3 => {
                fs::write(&execute_started, "started").unwrap();
                let deadline = Instant::now() + Duration::from_secs(5);
                while !std::path::Path::new(&execute_release).is_file() && Instant::now() < deadline {
                    thread::sleep(Duration::from_millis(5));
                }
                assert!(std::path::Path::new(&execute_release).is_file());
                println!("{}", r#"{"jsonrpc":"2.0","id":3,"result":{"content":[{"type":"text","text":"{\"stdout\":\"[\\\"old-index-result\\\"]\"}"}]}}"#);
            }
            4 => {
                println!("{}", r#"{"jsonrpc":"2.0","id":4,"result":{"content":[{"type":"text","text":"{}"}]}}"#);
                io::stdout().flush().unwrap();
                break;
            }
            _ => {}
        }
        io::stdout().flush().unwrap();
    }
}"##,
        )
        .unwrap();
        let status = Command::new("rustc")
            .arg(&source)
            .arg("-o")
            .arg(&executable)
            .status()
            .unwrap();
        assert!(status.success());
        executable
    }

    fn compile_initialize_fixture(context: &WorkspaceContext) -> PathBuf {
        let source = context.cache_root.join("initialize-fixture.rs");
        let executable =
            testing::fixture_executable_path(&context.cache_root, "initialize-fixture");
        fs::create_dir_all(&context.cache_root).unwrap();
        fs::write(
            &source,
            r#"use std::{env, fs, io::{self, BufRead, Write}, thread, time::Duration};
fn main() {
    let mode = env::args().nth(1).unwrap_or_default();
    if mode == "incomplete-eof" {
        fs::write(env::args().nth(2).unwrap(), std::process::id().to_string()).unwrap();
        let mut request = String::new();
        io::stdin().lock().read_line(&mut request).unwrap();
        assert!(request.contains("\"method\":\"initialize\""));
        fs::write(env::args().nth(3).unwrap(), "initialize-read").unwrap();
        print!("{{\"jsonrpc\":\"2.0\",\"id\":1");
        io::stdout().flush().unwrap();
        return;
    }
    if mode == "fragmented" {
        print!("{{\"jsonrpc\":\"2.0\",\"id\":1");
        io::stdout().flush().unwrap();
    }
    thread::sleep(Duration::from_secs(30));
}"#,
        )
        .unwrap();
        let status = Command::new("rustc")
            .arg(&source)
            .arg("-o")
            .arg(&executable)
            .status()
            .unwrap();
        assert!(status.success());
        executable
    }

    #[derive(Clone, Default)]
    struct ManualClock(Arc<Mutex<Duration>>);

    impl ManualClock {
        fn advance(&self, duration: Duration) {
            *self.0.lock().unwrap() += duration;
        }
    }

    impl ConnectorClock for ManualClock {
        fn elapsed(&self) -> Duration {
            *self.0.lock().unwrap()
        }
    }

    #[derive(Clone, Copy)]
    enum FailureStage {
        Connect,
        Write,
        Read,
        Eof,
    }

    struct RacingIo {
        cancellation: CancellationToken,
        stage: FailureStage,
        connections: Mutex<u32>,
        connect_timeouts: Mutex<Vec<Duration>>,
    }

    impl RacingIo {
        fn new(cancellation: CancellationToken, stage: FailureStage) -> Self {
            Self {
                cancellation,
                stage,
                connections: Mutex::new(0),
                connect_timeouts: Mutex::new(Vec::new()),
            }
        }
    }

    impl ConnectorIo for RacingIo {
        fn connect(&self, _port: u16, timeout: Duration) -> io::Result<Box<dyn ConnectorStream>> {
            self.connect_timeouts.lock().unwrap().push(timeout);
            let mut connections = self.connections.lock().unwrap();
            *connections += 1;
            if *connections == 1 && matches!(self.stage, FailureStage::Connect) {
                self.cancellation.cancel();
                return Err(io::Error::new(ErrorKind::ConnectionRefused, "connect race"));
            }
            Ok(Box::new(RacingStream {
                cancellation: self.cancellation.clone(),
                stage: (*connections == 1).then_some(self.stage),
                writes: 0,
            }))
        }
    }

    struct RacingStream {
        cancellation: CancellationToken,
        stage: Option<FailureStage>,
        writes: usize,
    }

    struct BudgetIo {
        clock: ManualClock,
        write_timeouts: Arc<Mutex<Vec<Duration>>>,
        expected_connect_timeout: Duration,
        connect_advance: Duration,
        first_write_advance: Duration,
        flush_advance: Duration,
    }

    impl ConnectorIo for BudgetIo {
        fn connect(&self, _port: u16, timeout: Duration) -> io::Result<Box<dyn ConnectorStream>> {
            assert_eq!(timeout, self.expected_connect_timeout);
            self.clock.advance(self.connect_advance);
            Ok(Box::new(BudgetStream {
                clock: self.clock.clone(),
                write_timeouts: Arc::clone(&self.write_timeouts),
                first_write_advance: self.first_write_advance,
                flush_advance: self.flush_advance,
                writes: 0,
            }))
        }
    }

    struct BudgetStream {
        clock: ManualClock,
        write_timeouts: Arc<Mutex<Vec<Duration>>>,
        first_write_advance: Duration,
        flush_advance: Duration,
        writes: usize,
    }

    impl Read for BudgetStream {
        fn read(&mut self, _buf: &mut [u8]) -> io::Result<usize> {
            Ok(0)
        }
    }

    impl Write for BudgetStream {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.writes += 1;
            if self.writes == 1 {
                self.clock.advance(self.first_write_advance);
            }
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            self.clock.advance(self.flush_advance);
            Ok(())
        }
    }

    impl ConnectorStream for BudgetStream {
        fn set_read_timeout(&self, _timeout: Option<Duration>) -> io::Result<()> {
            Ok(())
        }

        fn set_write_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
            self.write_timeouts.lock().unwrap().push(timeout.unwrap());
            Ok(())
        }
    }

    struct ChunkStream {
        chunks: std::collections::VecDeque<Vec<u8>>,
        chunk_offset: usize,
        clock: ManualClock,
        advance_per_read: Duration,
        cancel_after_reads: Option<(CancellationToken, usize)>,
        reads: usize,
    }

    impl Read for ChunkStream {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            self.clock.advance(self.advance_per_read);
            self.reads += 1;
            if let Some((token, target)) = &self.cancel_after_reads {
                if self.reads == *target {
                    token.cancel();
                }
            }
            let Some(chunk) = self.chunks.front() else {
                return Err(io::Error::new(ErrorKind::TimedOut, "poll"));
            };
            let remaining = &chunk[self.chunk_offset..];
            let count = remaining.len().min(buf.len());
            buf[..count].copy_from_slice(&remaining[..count]);
            self.chunk_offset += count;
            if self.chunk_offset == chunk.len() {
                self.chunks.pop_front();
                self.chunk_offset = 0;
            }
            Ok(count)
        }
    }

    impl Write for ChunkStream {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl ConnectorStream for ChunkStream {
        fn set_read_timeout(&self, _timeout: Option<Duration>) -> io::Result<()> {
            Ok(())
        }

        fn set_write_timeout(&self, _timeout: Option<Duration>) -> io::Result<()> {
            Ok(())
        }
    }

    enum PartialWriteFailure {
        None,
        TimeoutAfterBudget,
        CancelAfterBudget,
        WouldBlockThenCancel,
    }

    struct PartialWriteStream {
        clock: ManualClock,
        cancellation: CancellationToken,
        max_write: usize,
        advance_per_write: Duration,
        failure: PartialWriteFailure,
        writes: Vec<u8>,
        timeouts: Mutex<Vec<Duration>>,
    }

    impl Read for PartialWriteStream {
        fn read(&mut self, _buf: &mut [u8]) -> io::Result<usize> {
            Ok(0)
        }
    }

    impl Write for PartialWriteStream {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.clock.advance(self.advance_per_write);
            match self.failure {
                PartialWriteFailure::TimeoutAfterBudget => {
                    return Err(io::Error::new(ErrorKind::TimedOut, "write timeout"));
                }
                PartialWriteFailure::CancelAfterBudget => {
                    self.cancellation.cancel();
                    return Err(io::Error::new(ErrorKind::BrokenPipe, "write cancelled"));
                }
                PartialWriteFailure::WouldBlockThenCancel => {
                    self.cancellation.cancel();
                    return Err(io::Error::new(ErrorKind::WouldBlock, "blocked"));
                }
                PartialWriteFailure::None => {}
            }
            let count = self.max_write.min(buf.len());
            self.writes.extend_from_slice(&buf[..count]);
            Ok(count)
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl ConnectorStream for PartialWriteStream {
        fn set_read_timeout(&self, _timeout: Option<Duration>) -> io::Result<()> {
            Ok(())
        }

        fn set_write_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
            self.timeouts.lock().unwrap().push(timeout.unwrap());
            Ok(())
        }
    }

    struct DeadlineFlushStream {
        clock: ManualClock,
        advance: Duration,
    }

    impl Read for DeadlineFlushStream {
        fn read(&mut self, _buf: &mut [u8]) -> io::Result<usize> {
            Ok(0)
        }
    }

    impl Write for DeadlineFlushStream {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            self.clock.advance(self.advance);
            Ok(())
        }
    }

    impl ConnectorStream for DeadlineFlushStream {
        fn set_read_timeout(&self, _timeout: Option<Duration>) -> io::Result<()> {
            Ok(())
        }
        fn set_write_timeout(&self, _timeout: Option<Duration>) -> io::Result<()> {
            Ok(())
        }
    }

    impl Read for RacingStream {
        fn read(&mut self, _buf: &mut [u8]) -> io::Result<usize> {
            match self.stage {
                Some(FailureStage::Read) => {
                    self.cancellation.cancel();
                    Err(io::Error::new(ErrorKind::ConnectionReset, "read race"))
                }
                Some(FailureStage::Eof) => {
                    self.cancellation.cancel();
                    Ok(0)
                }
                _ => Ok(0),
            }
        }
    }

    impl Write for RacingStream {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.writes += 1;
            if self.writes == 1 && matches!(self.stage, Some(FailureStage::Write)) {
                self.cancellation.cancel();
                return Err(io::Error::new(ErrorKind::BrokenPipe, "write race"));
            }
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl ConnectorStream for RacingStream {
        fn set_read_timeout(&self, _timeout: Option<Duration>) -> io::Result<()> {
            Ok(())
        }

        fn set_write_timeout(&self, _timeout: Option<Duration>) -> io::Result<()> {
            Ok(())
        }
    }

    fn connector_test_record() -> WorkspaceServiceRecord {
        WorkspaceServiceRecord {
            schema_version: SERVICE_SCHEMA_VERSION,
            pid: std::process::id(),
            port: 1,
            token: "secret".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            workspace_root: "workspace".to_string(),
            source_root: "source".to_string(),
            started_at: now_secs_for_test(),
            last_access_at: now_secs_for_test(),
        }
    }

    fn connector_test_request(kind: ServiceRequestKind) -> ServiceRequest {
        ServiceRequest {
            token: "secret".to_string(),
            kind,
        }
    }

    #[test]
    fn cancellable_connector_deadline_is_aggregate() {
        let clock = ManualClock::default();
        let deadline = Deadline::new(&clock, Duration::from_millis(500));
        clock.advance(Duration::from_millis(300));
        assert_eq!(deadline.remaining(&clock), Some(Duration::from_millis(200)));
        clock.advance(Duration::from_millis(200));
        assert_eq!(deadline.remaining(&clock), None);
    }

    #[test]
    fn cancellable_connector_cancel_control_uses_one_aggregate_500ms_budget() {
        let clock = ManualClock::default();
        let write_timeouts = Arc::new(Mutex::new(Vec::new()));
        let io = BudgetIo {
            clock: clock.clone(),
            write_timeouts: Arc::clone(&write_timeouts),
            expected_connect_timeout: SERVICE_CONTROL_CONNECT_TIMEOUT,
            connect_advance: Duration::from_millis(300),
            first_write_advance: Duration::from_millis(100),
            flush_advance: Duration::ZERO,
        };
        SYSTEM_SERVICE_CONNECTOR
            .send_control_with(
                &connector_test_record(),
                ServiceRequestKind::Cancel {
                    operation_id: "budget-operation".to_string(),
                },
                &io,
                &clock,
            )
            .unwrap();

        assert_eq!(
            write_timeouts.lock().unwrap().as_slice(),
            &[
                Duration::from_millis(200),
                Duration::from_millis(100),
                Duration::from_millis(100)
            ]
        );
    }

    #[test]
    fn control_flush_success_cannot_cross_aggregate_500ms_budget() {
        let clock = ManualClock::default();
        let io = BudgetIo {
            clock: clock.clone(),
            write_timeouts: Arc::new(Mutex::new(Vec::new())),
            expected_connect_timeout: SERVICE_CONTROL_CONNECT_TIMEOUT,
            connect_advance: Duration::from_millis(300),
            first_write_advance: Duration::from_millis(100),
            flush_advance: Duration::from_millis(100),
        };
        let error = SYSTEM_SERVICE_CONNECTOR
            .send_control_with(
                &connector_test_record(),
                ServiceRequestKind::Cancel {
                    operation_id: "flush-deadline".into(),
                },
                &io,
                &clock,
            )
            .unwrap_err();
        assert!(error.contains("timed out"), "{error}");
    }

    #[test]
    fn cancellable_connector_work_write_uses_budget_remaining_after_connect() {
        let clock = ManualClock::default();
        let write_timeouts = Arc::new(Mutex::new(Vec::new()));
        let io = BudgetIo {
            clock: clock.clone(),
            write_timeouts: Arc::clone(&write_timeouts),
            expected_connect_timeout: SERVICE_CONNECT_TIMEOUT,
            connect_advance: Duration::from_secs(3),
            first_write_advance: Duration::ZERO,
            flush_advance: Duration::ZERO,
        };
        let cancellation = CancellationToken::new();
        let _ = SYSTEM_SERVICE_CONNECTOR.send_with(
            &connector_test_record(),
            connector_test_request(ServiceRequestKind::BslMcp {
                operation_id: "work-budget".to_string(),
                tool_name: "search".to_string(),
                tool_args: json!({}),
                timeout_seconds: 120,
                timeout_nanos: 0,
            }),
            &cancellation,
            SERVICE_REQUEST_TIMEOUT,
            &io,
            &clock,
        );

        assert_eq!(
            write_timeouts.lock().unwrap().first().copied(),
            Some(Duration::from_millis(100))
        );
    }

    #[test]
    fn cancellable_connector_reads_fragmented_response_and_ignores_bytes_after_newline() {
        let clock = ManualClock::default();
        let deadline = Deadline::new(&clock, Duration::from_secs(1));
        let mut stream = ChunkStream {
            chunks: [b"{\"ok\":".to_vec(), b"true}\nignored".to_vec()].into(),
            chunk_offset: 0,
            clock: clock.clone(),
            advance_per_read: Duration::from_millis(1),
            cancel_after_reads: None,
            reads: 0,
        };
        let response =
            read_service_response(&mut stream, &deadline, &clock, &CancellationToken::new())
                .unwrap();
        assert!(response.ok);
    }

    #[test]
    fn cancellable_connector_partial_response_cannot_bypass_deadline() {
        let clock = ManualClock::default();
        let deadline = Deadline::new(&clock, Duration::from_millis(3));
        let mut stream = ChunkStream {
            chunks: [b"a".to_vec(), b"b".to_vec(), b"c".to_vec(), b"d".to_vec()].into(),
            chunk_offset: 0,
            clock: clock.clone(),
            advance_per_read: Duration::from_millis(1),
            cancel_after_reads: None,
            reads: 0,
        };
        let error =
            read_service_response(&mut stream, &deadline, &clock, &CancellationToken::new())
                .unwrap_err();
        assert!(error.starts_with("timeout:"), "{error}");
    }

    #[test]
    fn cancellable_connector_partial_response_cannot_bypass_cancellation() {
        let clock = ManualClock::default();
        let cancellation = CancellationToken::new();
        let deadline = Deadline::new(&clock, Duration::from_secs(1));
        let mut stream = ChunkStream {
            chunks: [b"partial".to_vec(), b"still-partial".to_vec()].into(),
            chunk_offset: 0,
            clock: clock.clone(),
            advance_per_read: Duration::ZERO,
            cancel_after_reads: Some((cancellation.clone(), 2)),
            reads: 0,
        };
        let error =
            read_service_response(&mut stream, &deadline, &clock, &cancellation).unwrap_err();
        assert!(error.starts_with("cancelled:"), "{error}");
    }

    #[test]
    fn cancellable_connector_rejects_oversized_response_line() {
        let clock = ManualClock::default();
        let deadline = Deadline::new(&clock, Duration::from_secs(1));
        let mut stream = ChunkStream {
            chunks: [vec![b'x'; SERVICE_RESPONSE_LINE_LIMIT + 1]].into(),
            chunk_offset: 0,
            clock: clock.clone(),
            advance_per_read: Duration::ZERO,
            cancel_after_reads: None,
            reads: 0,
        };
        let error =
            read_service_response(&mut stream, &deadline, &clock, &CancellationToken::new())
                .unwrap_err();
        assert!(error.contains("response line exceeds"), "{error}");
    }

    #[test]
    fn cancellable_connector_partial_writes_recompute_remaining_budget() {
        let clock = ManualClock::default();
        let cancellation = CancellationToken::new();
        let deadline = Deadline::new(&clock, Duration::from_millis(100));
        let mut stream = PartialWriteStream {
            clock: clock.clone(),
            cancellation: cancellation.clone(),
            max_write: 2,
            advance_per_write: Duration::from_millis(10),
            failure: PartialWriteFailure::None,
            writes: Vec::new(),
            timeouts: Mutex::new(Vec::new()),
        };
        write_with_deadline(&mut stream, b"abcdef", &deadline, &clock, &cancellation).unwrap();
        assert_eq!(stream.writes, b"abcdef");
        assert_eq!(
            *stream.timeouts.lock().unwrap(),
            vec![
                Duration::from_millis(100),
                Duration::from_millis(90),
                Duration::from_millis(80)
            ]
        );
    }

    #[test]
    fn cancellable_connector_successful_final_write_cannot_cross_deadline() {
        let clock = ManualClock::default();
        let cancellation = CancellationToken::new();
        let deadline = Deadline::new(&clock, Duration::from_millis(10));
        let mut stream = PartialWriteStream {
            clock: clock.clone(),
            cancellation: cancellation.clone(),
            max_write: 1,
            advance_per_write: Duration::from_millis(10),
            failure: PartialWriteFailure::None,
            writes: Vec::new(),
            timeouts: Mutex::new(Vec::new()),
        };
        let error =
            write_with_deadline(&mut stream, b"x", &deadline, &clock, &cancellation).unwrap_err();
        assert!(error.starts_with("timeout:"), "{error}");
    }

    #[test]
    fn cancellable_connector_successful_flush_cannot_cross_deadline() {
        let clock = ManualClock::default();
        let deadline = Deadline::new(&clock, Duration::from_millis(10));
        let mut stream = DeadlineFlushStream {
            clock: clock.clone(),
            advance: Duration::from_millis(10),
        };
        let error = flush_with_deadline(&mut stream, &deadline, &clock, &CancellationToken::new())
            .unwrap_err();
        assert!(error.starts_with("timeout:"), "{error}");
    }

    #[test]
    fn cancellable_connector_rechecks_cancel_after_blocked_write_slice() {
        let clock = ManualClock::default();
        let cancellation = CancellationToken::new();
        let deadline = Deadline::new(&clock, Duration::from_secs(5));
        let mut stream = PartialWriteStream {
            clock: clock.clone(),
            cancellation: cancellation.clone(),
            max_write: 1,
            advance_per_write: Duration::from_millis(100),
            failure: PartialWriteFailure::WouldBlockThenCancel,
            writes: Vec::new(),
            timeouts: Mutex::new(Vec::new()),
        };
        let error =
            write_with_deadline(&mut stream, b"x", &deadline, &clock, &cancellation).unwrap_err();
        assert!(error.starts_with("cancelled:"), "{error}");
        assert_eq!(
            stream.timeouts.lock().unwrap().as_slice(),
            &[Duration::from_millis(100)]
        );
    }

    #[test]
    fn cancellable_connector_write_error_uses_timeout_unless_cancelled() {
        for (failure, expected) in [
            (PartialWriteFailure::TimeoutAfterBudget, "timeout:"),
            (PartialWriteFailure::CancelAfterBudget, "cancelled:"),
        ] {
            let clock = ManualClock::default();
            let cancellation = CancellationToken::new();
            let deadline = Deadline::new(&clock, Duration::from_millis(10));
            let mut stream = PartialWriteStream {
                clock: clock.clone(),
                cancellation: cancellation.clone(),
                max_write: 1,
                advance_per_write: Duration::from_millis(10),
                failure,
                writes: Vec::new(),
                timeouts: Mutex::new(Vec::new()),
            };
            let error = write_with_deadline(&mut stream, b"x", &deadline, &clock, &cancellation)
                .unwrap_err();
            assert!(error.starts_with(expected), "{error}");
        }
    }

    #[test]
    fn cancellable_connector_rejects_write_zero() {
        let clock = ManualClock::default();
        let cancellation = CancellationToken::new();
        let deadline = Deadline::new(&clock, Duration::from_secs(1));
        let mut stream = PartialWriteStream {
            clock: clock.clone(),
            cancellation: cancellation.clone(),
            max_write: 0,
            advance_per_write: Duration::ZERO,
            failure: PartialWriteFailure::None,
            writes: Vec::new(),
            timeouts: Mutex::new(Vec::new()),
        };
        let error =
            write_with_deadline(&mut stream, b"x", &deadline, &clock, &cancellation).unwrap_err();
        assert!(error.contains("failed to write workspace service request"));
    }

    #[test]
    fn cancellable_connector_prioritizes_cancel_over_transport_races() {
        for stage in [
            FailureStage::Connect,
            FailureStage::Write,
            FailureStage::Read,
            FailureStage::Eof,
        ] {
            let cancellation = CancellationToken::new();
            let io = RacingIo::new(cancellation.clone(), stage);
            let clock = ManualClock::default();
            let error = SYSTEM_SERVICE_CONNECTOR
                .send_with(
                    &connector_test_record(),
                    connector_test_request(ServiceRequestKind::BslMcp {
                        operation_id: "race-operation".to_string(),
                        tool_name: "search".to_string(),
                        tool_args: json!({}),
                        timeout_seconds: 120,
                        timeout_nanos: 0,
                    }),
                    &cancellation,
                    SERVICE_REQUEST_TIMEOUT,
                    &io,
                    &clock,
                )
                .unwrap_err();
            assert!(error.starts_with("cancelled:"), "{error}");
        }
    }

    #[test]
    fn cancellable_connector_uses_short_connect_budget_for_every_control_kind() {
        for kind in [
            ServiceRequestKind::Ping,
            ServiceRequestKind::Invalidate { events: vec![] },
            ServiceRequestKind::Shutdown,
        ] {
            let cancellation = CancellationToken::new();
            let io = RacingIo::new(cancellation.clone(), FailureStage::Eof);
            let _ = SYSTEM_SERVICE_CONNECTOR.send_with(
                &connector_test_record(),
                connector_test_request(kind),
                &cancellation,
                SERVICE_REQUEST_TIMEOUT,
                &io,
                &ManualClock::default(),
            );
            assert_eq!(
                io.connect_timeouts.lock().unwrap().as_slice(),
                &[SERVICE_CONTROL_CONNECT_TIMEOUT]
            );
        }
    }

    #[test]
    fn cancellable_connector_protocol_roundtrips_work_and_cancel_shapes() {
        let bsl = ServiceRequestKind::BslMcp {
            operation_id: Uuid::new_v4().to_string(),
            tool_name: "search".to_string(),
            tool_args: json!({"query": "needle"}),
            timeout_seconds: 5,
            timeout_nanos: 0,
        };
        let rlm = ServiceRequestKind::RlmReady {
            operation_id: Uuid::new_v4().to_string(),
            args: json!({"sourceDir": "src"}),
            timeout_seconds: 5,
            timeout_nanos: 123,
        };
        assert_ne!(bsl.operation_id(), rlm.operation_id());
        for (kind, expected_tag) in [
            (bsl, "bsl-mcp"),
            (rlm, "rlm-ready"),
            (
                ServiceRequestKind::Cancel {
                    operation_id: "cancel-id".to_string(),
                },
                "cancel",
            ),
        ] {
            let json = serde_json::to_value(&kind).unwrap();
            assert_eq!(json.get("type").and_then(Value::as_str), Some(expected_tag));
            assert!(json.get("operation_id").is_some());
            assert_eq!(
                serde_json::from_value::<ServiceRequestKind>(json).unwrap(),
                kind
            );
        }

        let invalidation = ServiceRequestKind::Invalidate {
            events: vec![DomainEvent::source_resources_replaced(
                crate::domain::events::SourceResourcesReplaced {
                    source_set: "main".to_string(),
                    owner: "CommonModule.Shared".to_string(),
                    roles: vec![crate::domain::source_resources::ResourceRole::BslModule],
                    preimage_hashes: vec!["sha256:before".to_string()],
                    postimage_hashes: vec!["sha256:after".to_string()],
                    affected_targets: vec!["CommonModule.Shared.Module".to_string()],
                },
            )],
        };
        let json = serde_json::to_value(&invalidation).unwrap();
        assert_eq!(json["type"], "invalidate");
        assert_eq!(
            json["events"][0]["details"]["affectedTargets"],
            json!(["CommonModule.Shared.Module"])
        );
        assert_eq!(
            serde_json::from_value::<ServiceRequestKind>(json).unwrap(),
            invalidation
        );
    }

    struct FailingWriter;

    impl Write for FailingWriter {
        fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
            Err(io::Error::new(ErrorKind::BrokenPipe, "caller disconnected"))
        }

        fn flush(&mut self) -> io::Result<()> {
            Err(io::Error::new(ErrorKind::BrokenPipe, "caller disconnected"))
        }
    }

    #[test]
    fn cancel_response_disconnect_is_non_fatal_and_service_record_remains() {
        let context = test_context("cancel-disconnect");
        let identity =
            WorkspaceServiceIdentity::new(&context, &context.workspace_root.join("src")).unwrap();
        let record = test_record(&identity, 34567, env!("CARGO_PKG_VERSION"));
        write_record(&identity, record.clone());
        let runtime = WorkspaceServiceRuntime::new(identity.clone(), &record);
        let response = runtime.cancel_operation("gone-caller");

        assert!(!write_service_response(FailingWriter, &response, true).unwrap());
        assert!(read_record(&identity).is_some());
        let ping = runtime.ping();
        assert!(ping.ok);
        cleanup(&context);
    }

    #[test]
    fn cancellable_connector_sends_cancel_on_a_separate_connection() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let (request_tx, request_rx) = mpsc::channel();
        let server = thread::spawn(move || {
            let (work_stream, _) = listener.accept().unwrap();
            let mut work_reader = BufReader::new(work_stream);
            let mut work_line = String::new();
            work_reader.read_line(&mut work_line).unwrap();
            request_tx
                .send(serde_json::from_str::<ServiceRequest>(work_line.trim()).unwrap())
                .unwrap();

            let (cancel_stream, _) = listener.accept().unwrap();
            let mut cancel_line = String::new();
            BufReader::new(cancel_stream)
                .read_line(&mut cancel_line)
                .unwrap();
            request_tx
                .send(serde_json::from_str::<ServiceRequest>(cancel_line.trim()).unwrap())
                .unwrap();
        });
        let record = WorkspaceServiceRecord {
            schema_version: SERVICE_SCHEMA_VERSION,
            pid: std::process::id(),
            port,
            token: "secret".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            workspace_root: "workspace".to_string(),
            source_root: "source".to_string(),
            started_at: now_secs_for_test(),
            last_access_at: now_secs_for_test(),
        };
        let operation_id = "operation-1".to_string();
        let cancellation = CancellationToken::new();
        let caller_token = cancellation.clone();
        let caller = thread::spawn(move || {
            SYSTEM_SERVICE_CONNECTOR.send(
                &record,
                ServiceRequest {
                    token: "secret".to_string(),
                    kind: ServiceRequestKind::BslMcp {
                        operation_id: operation_id.clone(),
                        tool_name: "search".to_string(),
                        tool_args: json!({}),
                        timeout_seconds: 120,
                        timeout_nanos: 0,
                    },
                },
                &caller_token,
                SERVICE_REQUEST_TIMEOUT,
            )
        });

        let work = request_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        let work_id = match work.kind {
            ServiceRequestKind::BslMcp { operation_id, .. } => operation_id,
            other => panic!("expected bsl work request, got {other:?}"),
        };
        let cancelled_at = Instant::now();
        cancellation.cancel();
        let cancel = request_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let error = caller.join().unwrap().unwrap_err();

        assert_eq!(
            cancel.kind,
            ServiceRequestKind::Cancel {
                operation_id: work_id
            }
        );
        assert!(error.starts_with("cancelled:"));
        assert!(cancelled_at.elapsed() < Duration::from_secs(2));
        server.join().unwrap();
    }

    #[test]
    fn cancellation_prefix_is_stable_for_pre_cancelled_manager_call() {
        let context = test_context("pre-cancelled-manager");
        let cancellation = CancellationToken::new();
        cancellation.cancel();

        let error = WorkspaceServiceManager::new()
            .ensure_service_cancellable(
                &context,
                &context.workspace_root.join("src"),
                &cancellation,
            )
            .unwrap_err();

        assert!(error.starts_with("cancelled:"));
        cleanup(&context);
    }

    #[test]
    fn mcp_tool_text_rejects_tool_level_error() {
        let response = json!({
            "jsonrpc": "2.0",
            "id": 2,
            "result": {
                "content": [{ "type": "text", "text": "schema is unavailable" }],
                "isError": true
            }
        });

        let error = mcp_tool_text(&response).unwrap_err();

        assert_eq!(error, "schema is unavailable");
    }

    #[test]
    fn source_generation_ignores_generated_build_cache_but_tracks_bsl_source() {
        let context = test_context("source-generation");
        let source_root = context.workspace_root.join("src");
        let module = source_root.join("CommonModules/SmokeModule.bsl");
        fs::write(&module, "Процедура Тест() Экспорт\nКонецПроцедуры\n").unwrap();
        let baseline = source_generation(&source_root);

        let generated = source_root.join(".build/bsl-graph.db");
        fs::create_dir_all(generated.parent().unwrap()).unwrap();
        fs::write(&generated, "generated cache").unwrap();
        assert_eq!(source_generation(&source_root), baseline);

        fs::write(
            &module,
            "Процедура Тест() Экспорт\n\tСообщить(\"Изменено\");\nКонецПроцедуры\n",
        )
        .unwrap();
        assert_ne!(source_generation(&source_root), baseline);
        cleanup(&context);
    }

    #[test]
    fn first_source_generation_observation_is_not_stale() {
        let observed = Mutex::new(None);
        let invalidated = AtomicBool::new(false);

        assert!(!observe_source_generation(&observed, &invalidated, 41));
        assert_eq!(*observed.lock().unwrap(), Some(41));
        assert!(!observe_source_generation(&observed, &invalidated, 41));
        assert!(observe_source_generation(&observed, &invalidated, 42));

        invalidated.store(true, Ordering::Release);
        assert!(observe_source_generation(&observed, &invalidated, 42));
    }

    #[test]
    fn workspace_service_runtime_starts_without_observed_source_generations() {
        let context = test_context("lazy-runtime-generation");
        let source_root = context.workspace_root.join("src");
        let identity = WorkspaceServiceIdentity::new(&context, &source_root).unwrap();
        let record = test_record(&identity, 1, env!("CARGO_PKG_VERSION"));

        let runtime = WorkspaceServiceRuntime::new(identity, &record);

        assert_eq!(*runtime.analyzer_source_generation.lock().unwrap(), None);
        assert_eq!(*runtime.rlm_source_generation.lock().unwrap(), None);
        cleanup(&context);
    }

    #[test]
    fn bsl_generation_exhausts_operation_budget_before_provider_start() {
        let context = test_context("bsl-generation-budget");
        let source_root = context.workspace_root.join("src");
        fs::write(
            source_root.join("Module.bsl"),
            "Процедура Тест()\nКонецПроцедуры\n",
        )
        .unwrap();
        let identity = WorkspaceServiceIdentity::new(&context, &source_root).unwrap();
        let record = test_record(&identity, 1, env!("CARGO_PKG_VERSION"));
        let mut runtime = WorkspaceServiceRuntime::new(identity, &record);
        let starts = Arc::new(AtomicUsize::new(0));
        runtime.analyzer_starter = Arc::new({
            let starts = Arc::clone(&starts);
            move |_context, _source_root, _cancellation| {
                starts.fetch_add(1, Ordering::AcqRel);
                Err("provider started after operation budget expired".to_string())
            }
        });

        let response = runtime.handle_bsl_mcp(
            "diagnostics_status",
            json!({}),
            0,
            1,
            &CancellationToken::new(),
        );

        assert_eq!(starts.load(Ordering::Acquire), 0, "{response:?}");
        assert!(
            response
                .error
                .as_deref()
                .is_some_and(|error| error.starts_with("timeout:")),
            "{response:?}"
        );
        cleanup(&context);
    }

    #[test]
    fn rlm_generation_exhausts_operation_budget_before_provider_start() {
        let context = test_context("rlm-generation-budget");
        let source_root = context.workspace_root.join("src");
        fs::write(
            source_root.join("Module.bsl"),
            "Процедура Тест()\nКонецПроцедуры\n",
        )
        .unwrap();
        write_ready_rlm_status_for_current_source(&context, &source_root);
        let identity = WorkspaceServiceIdentity::new(&context, &source_root).unwrap();
        let record = test_record(&identity, 1, env!("CARGO_PKG_VERSION"));
        let mut runtime = WorkspaceServiceRuntime::new(identity, &record);
        let starts = Arc::new(AtomicUsize::new(0));
        runtime.rlm_starter = Arc::new({
            let starts = Arc::clone(&starts);
            move |_context, _source_root, _cancellation| {
                starts.fetch_add(1, Ordering::AcqRel);
                Err("provider started after operation budget expired".to_string())
            }
        });

        let response = runtime.handle_rlm_mcp(
            WorkspaceRlmOperation::Search {
                query: "Тест".to_string(),
                limit: 20,
            },
            0,
            1,
            &CancellationToken::new(),
        );

        assert_eq!(starts.load(Ordering::Acquire), 0, "{response:?}");
        assert!(
            response
                .error
                .as_deref()
                .is_some_and(|error| error.starts_with("timeout:")),
            "{response:?}"
        );
        cleanup(&context);
    }

    #[test]
    fn rlm_execute_rechecks_generation_after_readiness_before_starting_session() {
        let context = test_context("rlm-generation-before-execute");
        let source_root = context.workspace_root.join("src");
        let module = source_root.join("CommonModules/SmokeModule.bsl");
        fs::write(&module, "Процедура Smoke()\nКонецПроцедуры\n").unwrap();
        write_ready_rlm_status_for_current_source(&context, &source_root);
        let identity = WorkspaceServiceIdentity::new(&context, &source_root).unwrap();
        let record = test_record(&identity, 1, env!("CARGO_PKG_VERSION"));
        let mut runtime = WorkspaceServiceRuntime::new(identity, &record);
        let starts = Arc::new(AtomicUsize::new(0));
        let maintenance_requests = Arc::new(AtomicUsize::new(0));
        runtime.rlm_maintenance_requester = Arc::new({
            let maintenance_requests = Arc::clone(&maintenance_requests);
            move |_context, _source_root, _generation, _revision_service, _cancellation| {
                maintenance_requests.fetch_add(1, Ordering::AcqRel);
            }
        });
        runtime.rlm_starter = Arc::new({
            let starts = Arc::clone(&starts);
            move |_context, _source_root, _cancellation| {
                starts.fetch_add(1, Ordering::AcqRel);
                Err("RLM execute crossed a stale generation boundary".to_string())
            }
        });

        fs::write(&module, "Процедура Smoke(НовыйПараметр)\nКонецПроцедуры\n").unwrap();
        let response = runtime.handle_rlm_mcp(
            WorkspaceRlmOperation::Search {
                query: "Smoke".to_string(),
                limit: 20,
            },
            5,
            0,
            &CancellationToken::new(),
        );
        wait_for_atomic_value(&maintenance_requests, 1, Duration::from_secs(2));
        cleanup(&context);

        assert_eq!(
            starts.load(Ordering::Acquire),
            0,
            "RLM must not start over a ready marker for an older source generation"
        );
        assert!(!response.ok);
        assert_eq!(response.index_status.as_deref(), Some("stale"));
        assert_eq!(response.error.as_deref(), Some("stale (source generation)"));
        assert!(response.result_text.is_none());
        assert!(response.stderr.is_none());
        assert_eq!(maintenance_requests.load(Ordering::Acquire), 1);
    }

    #[test]
    fn rlm_execute_returns_stale_responses_and_deduplicates_blocked_maintenance() {
        let context = test_context("rlm-nonblocking-maintenance");
        let source_root = context.workspace_root.join("src");
        let module = source_root.join("CommonModules/SmokeModule.bsl");
        fs::write(&module, "Процедура Smoke()\nКонецПроцедуры\n").unwrap();
        write_ready_rlm_status_for_current_source(&context, &source_root);
        let identity = WorkspaceServiceIdentity::new(&context, &source_root).unwrap();
        let record = test_record(&identity, 1, env!("CARGO_PKG_VERSION"));
        let mut runtime = WorkspaceServiceRuntime::new(identity, &record);
        let maintenance_requests = Arc::new(AtomicUsize::new(0));
        let (maintenance_started_tx, maintenance_started_rx) = mpsc::channel();
        let (maintenance_release_tx, maintenance_release_rx) = mpsc::channel();
        let maintenance_release_rx = Arc::new(Mutex::new(maintenance_release_rx));
        runtime.rlm_maintenance_requester = Arc::new({
            let maintenance_requests = Arc::clone(&maintenance_requests);
            let maintenance_release_rx = Arc::clone(&maintenance_release_rx);
            move |_context, _source_root, _generation, _revision_service, _cancellation| {
                let request_index = maintenance_requests.fetch_add(1, Ordering::AcqRel);
                maintenance_started_tx.send(request_index).unwrap();
                if request_index == 0 {
                    maintenance_release_rx.lock().unwrap().recv().unwrap();
                }
            }
        });
        fs::write(&module, "Процедура Smoke(НовыйПараметр)\nКонецПроцедуры\n").unwrap();
        let runtime = Arc::new(runtime);
        let first_runtime = Arc::clone(&runtime);
        let (first_response_tx, first_response_rx) = mpsc::channel();
        let first_caller = thread::spawn(move || {
            let response = first_runtime.handle_rlm_mcp(
                WorkspaceRlmOperation::Search {
                    query: "Smoke".to_string(),
                    limit: 20,
                },
                5,
                0,
                &CancellationToken::new(),
            );
            first_response_tx.send(response).unwrap();
        });

        assert_eq!(
            maintenance_started_rx
                .recv_timeout(Duration::from_secs(2))
                .expect("maintenance request was not started"),
            0
        );
        let first_response = first_response_rx
            .recv_timeout(Duration::from_millis(250))
            .expect("first stale response waited for index maintenance");
        let second_runtime = Arc::clone(&runtime);
        let (second_response_tx, second_response_rx) = mpsc::channel();
        let second_caller = thread::spawn(move || {
            let response = second_runtime.handle_rlm_mcp(
                WorkspaceRlmOperation::Search {
                    query: "Smoke".to_string(),
                    limit: 20,
                },
                5,
                0,
                &CancellationToken::new(),
            );
            second_response_tx.send(response).unwrap();
        });
        let second_response = second_response_rx
            .recv_timeout(Duration::from_millis(250))
            .expect("second stale response waited for index maintenance");
        assert!(
            matches!(
                maintenance_started_rx.recv_timeout(Duration::from_millis(250)),
                Err(mpsc::RecvTimeoutError::Timeout)
            ),
            "a second stale request started duplicate index maintenance"
        );

        assert!(!first_response.ok);
        assert_eq!(first_response.index_status.as_deref(), Some("stale"));
        assert!(!second_response.ok);
        assert_eq!(second_response.index_status.as_deref(), Some("stale"));
        assert_eq!(
            maintenance_requests.load(Ordering::Acquire),
            1,
            "a second stale request started duplicate index maintenance"
        );

        maintenance_release_tx.send(()).unwrap();
        first_caller.join().unwrap();
        second_caller.join().unwrap();
        drop(runtime);
        cleanup(&context);
    }

    #[test]
    fn rlm_execute_discards_output_when_source_changes_before_fake_execute_returns() {
        let observation = run_blocking_rlm_execute(
            "rlm-generation-during-execute",
            |_context, _source_root, module| {
                fs::write(module, "Процедура Smoke(НовыйПараметр)\nКонецПроцедуры\n").unwrap();
            },
        );

        assert!(!observation.response.ok);
        assert_eq!(observation.response.index_status.as_deref(), Some("stale"));
        assert_eq!(
            observation.response.error.as_deref(),
            Some("stale (source generation)")
        );
        assert!(observation.response.result_text.is_none());
        assert!(observation.response.stderr.is_none());
        assert!(
            observation.session_retired,
            "stale logical RLM session was retained"
        );
        assert_eq!(observation.maintenance_requests, 1);
        assert_eq!(observation.maintenance_revisions.len(), 1);
        assert_ne!(
            observation.maintenance_revisions[0], observation.pre_execution_revision,
            "maintenance was scheduled for the already stale pre-execution revision"
        );
        assert!(
            observation.teardown_drained,
            "retired fake RLM session teardown exceeded the shutdown grace"
        );
    }

    #[test]
    fn rlm_execute_preserves_active_index_lock_priority_after_source_change() {
        let observation = run_blocking_rlm_execute(
            "rlm-active-lock-during-execute",
            |context, source_root, module| {
                fs::write(module, "Процедура Smoke(НовыйПараметр)\nКонецПроцедуры\n").unwrap();
                write_active_rlm_index_lock(context, source_root);
            },
        );

        assert!(!observation.response.ok);
        assert_eq!(
            observation.response.index_status.as_deref(),
            Some("building")
        );
        assert_eq!(
            observation.response.error.as_deref(),
            Some("rlm index building")
        );
        assert!(observation.response.result_text.is_none());
        assert!(observation.response.stderr.is_none());
        assert!(
            observation.session_retired,
            "stale logical RLM session was retained"
        );
        assert_eq!(observation.maintenance_requests, 1);
        assert!(
            observation.teardown_drained,
            "retired fake RLM session teardown exceeded the shutdown grace"
        );
    }

    #[test]
    fn service_identity_reuses_same_workspace_source_root_and_separates_other_roots() {
        let context = test_context("identity");
        let source_root = context.workspace_root.join("src");
        let same = WorkspaceServiceIdentity::new(&context, &source_root).unwrap();
        let repeated = WorkspaceServiceIdentity::new(&context, &source_root).unwrap();
        let other_source =
            WorkspaceServiceIdentity::new(&context, &context.workspace_root.join("extension"))
                .unwrap();
        let other_workspace = test_context("identity-other");
        let other_workspace_identity =
            WorkspaceServiceIdentity::new(&other_workspace, &other_workspace.workspace_root)
                .unwrap();

        assert_eq!(same.key, repeated.key);
        assert_ne!(same.key, other_source.key);
        assert_ne!(same.key, other_workspace_identity.key);
        assert!(same
            .service_dir
            .ends_with(Path::new("services").join(&same.key)));

        cleanup(&context);
        cleanup(&other_workspace);
    }

    #[test]
    fn service_identity_reuses_normalized_paths() {
        let context = test_context("normalized-identity");
        let plain =
            WorkspaceServiceIdentity::new(&context, &context.workspace_root.join("src")).unwrap();
        let dotted =
            WorkspaceServiceIdentity::new(&context, &context.workspace_root.join("src/./"))
                .unwrap();

        assert_eq!(plain.key, dotted.key);
        cleanup(&context);
    }

    #[test]
    fn service_record_is_reusable_only_for_matching_live_version_and_paths() {
        let context = test_context("record");
        let source_root = context.workspace_root.join("src");
        let identity = WorkspaceServiceIdentity::new(&context, &source_root).unwrap();
        let record = WorkspaceServiceRecord {
            schema_version: SERVICE_SCHEMA_VERSION,
            pid: std::process::id(),
            port: 34567,
            token: "token".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            workspace_root: identity.workspace_root.clone(),
            source_root: identity.source_root.clone(),
            started_at: now_secs_for_test(),
            last_access_at: now_secs_for_test(),
        };

        assert!(record.matches(&identity, env!("CARGO_PKG_VERSION")));

        let mut mismatched_schema = record.clone();
        mismatched_schema.schema_version = SERVICE_SCHEMA_VERSION - 1;
        assert!(!mismatched_schema.matches(&identity, env!("CARGO_PKG_VERSION")));

        let mut mismatched_version = record.clone();
        mismatched_version.version = "older".to_string();
        assert!(!mismatched_version.matches(&identity, env!("CARGO_PKG_VERSION")));

        let mut mismatched_source = record;
        mismatched_source.source_root = context.workspace_root.join("other").display().to_string();
        assert!(!mismatched_source.matches(&identity, env!("CARGO_PKG_VERSION")));

        cleanup(&context);
    }

    #[test]
    fn service_record_v2_is_not_reused_after_typed_invalidation_protocol() {
        let context = test_context("typed-invalidation-record-version");
        let source_root = context.workspace_root.join("src");
        let identity = WorkspaceServiceIdentity::new(&context, &source_root).unwrap();
        let mut record = test_record(&identity, 34567, env!("CARGO_PKG_VERSION"));
        record.schema_version = 2;

        assert!(
            !record.matches(&identity, env!("CARGO_PKG_VERSION")),
            "the pre-typed-invalidation service protocol must not be reused"
        );
        cleanup(&context);
    }

    #[test]
    fn service_record_v3_is_not_reused_after_lossless_worker_timeout_protocol() {
        let context = test_context("lossless-worker-timeout-record-version");
        let source_root = context.workspace_root.join("src");
        let identity = WorkspaceServiceIdentity::new(&context, &source_root).unwrap();
        let mut record = test_record(&identity, 34567, env!("CARGO_PKG_VERSION"));
        record.schema_version = 3;

        assert!(
            !record.matches(&identity, env!("CARGO_PKG_VERSION")),
            "the millisecond-saturated worker timeout protocol must not be reused"
        );
        cleanup(&context);
    }

    #[test]
    fn service_record_with_shutting_down_ping_is_not_reusable() {
        struct ShuttingDownConnector;
        impl ServiceConnector for ShuttingDownConnector {
            fn send(
                &self,
                _record: &WorkspaceServiceRecord,
                _request: ServiceRequest,
                _cancellation: &CancellationToken,
                _budget: Duration,
            ) -> Result<ServiceResponse, String> {
                Ok(ServiceResponse {
                    ok: true,
                    status: Some("shutting-down".to_string()),
                    ..ServiceResponse::default()
                })
            }
        }

        let context = test_context("shutting-down-record");
        let identity =
            WorkspaceServiceIdentity::new(&context, &context.workspace_root.join("src")).unwrap();
        let record = test_record(&identity, 34567, env!("CARGO_PKG_VERSION"));
        write_record(&identity, record.clone());
        let spawner = RecordingSpawner::default();
        let manager = WorkspaceServiceManager::with_io(&ShuttingDownConnector, &spawner);

        let replacement = manager.ensure_service(&context, Path::new(&identity.source_root));

        assert_eq!(replacement.unwrap().port, 45678);
        assert_eq!(*spawner.spawns.borrow(), 1);
        cleanup(&context);
    }

    #[test]
    fn spawn_wait_rejects_shutting_down_record() {
        struct ShuttingDownConnector;
        impl ServiceConnector for ShuttingDownConnector {
            fn send(
                &self,
                _record: &WorkspaceServiceRecord,
                _request: ServiceRequest,
                _cancellation: &CancellationToken,
                _budget: Duration,
            ) -> Result<ServiceResponse, String> {
                Ok(ServiceResponse {
                    ok: true,
                    status: Some("shutting-down".to_string()),
                    ..ServiceResponse::default()
                })
            }
        }
        let context = test_context("spawn-wait-shutting-down");
        let identity =
            WorkspaceServiceIdentity::new(&context, &context.workspace_root.join("src")).unwrap();
        let record = test_record(&identity, 34567, env!("CARGO_PKG_VERSION"));
        write_record(&identity, record.clone());

        let result = wait_for_record_with_connector(
            &identity,
            &ShuttingDownConnector,
            record.pid,
            &record.token,
            Duration::from_millis(75),
            &CancellationToken::new(),
        );

        assert!(result.is_err());
        cleanup(&context);
    }

    #[test]
    fn spawn_wait_prioritizes_cancellation_over_its_timeout() {
        let context = test_context("spawn-wait-cancelled");
        let identity =
            WorkspaceServiceIdentity::new(&context, &context.workspace_root.join("src")).unwrap();
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        let started = Instant::now();

        let error = wait_for_record_with_connector(
            &identity,
            &RecordingConnector::default(),
            34567,
            "secret",
            SERVICE_CONNECT_TIMEOUT,
            &cancellation,
        )
        .unwrap_err();

        assert!(error.starts_with("cancelled:"), "{error}");
        assert!(started.elapsed() < Duration::from_millis(100));
        cleanup(&context);
    }

    #[test]
    fn failed_spawn_cleanup_reaps_child_and_preserves_replacement_record() {
        let context = test_context("failed-spawn-child-cleanup");
        let source_root = context.workspace_root.join("src");
        let identity = WorkspaceServiceIdentity::new(&context, &source_root).unwrap();

        let mut owned_command = Command::new(std::env::current_exe().unwrap());
        owned_command
            .args([
                "--exact",
                "infrastructure::workspace_services::tests::spawn_cleanup_child_fixture",
                "--nocapture",
            ])
            .env("UNICA_SPAWN_CLEANUP_CHILD_FIXTURE", "1")
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let mut owned_child = ManagedStartupChild::spawn_configured(owned_command).unwrap();
        thread::sleep(Duration::from_millis(75));
        assert!(owned_child.is_running().unwrap());
        let mut owned_record = test_record(&identity, 34567, env!("CARGO_PKG_VERSION"));
        owned_record.pid = owned_child.id();
        owned_record.token = "owned-spawn-token".to_string();
        write_record(&identity, owned_record);

        terminate_failed_workspace_service_spawn(
            &mut owned_child,
            &identity,
            "owned-spawn-token",
            Duration::from_secs(1),
        )
        .unwrap();

        assert!(read_record(&identity).is_none());

        let mut replaced_command = Command::new(std::env::current_exe().unwrap());
        replaced_command
            .args([
                "--exact",
                "infrastructure::workspace_services::tests::spawn_cleanup_child_fixture",
                "--nocapture",
            ])
            .env("UNICA_SPAWN_CLEANUP_CHILD_FIXTURE", "1")
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let mut replaced_child = ManagedStartupChild::spawn_configured(replaced_command).unwrap();
        thread::sleep(Duration::from_millis(75));
        assert!(replaced_child.is_running().unwrap());
        let replacement = test_record(&identity, 45678, env!("CARGO_PKG_VERSION"));
        write_record(&identity, replacement.clone());

        terminate_failed_workspace_service_spawn(
            &mut replaced_child,
            &identity,
            "different-spawn-token",
            Duration::from_secs(1),
        )
        .unwrap();

        assert_eq!(read_record(&identity), Some(replacement));
        cleanup(&context);
    }

    #[test]
    fn startup_failure_reports_exit_status_and_bounded_stderr_tail() {
        let context = test_context("startup-failure-diagnostic");
        let identity =
            WorkspaceServiceIdentity::new(&context, &context.workspace_root.join("src")).unwrap();
        let stderr_path = identity.service_dir.join("service.stderr.log");
        fs::create_dir_all(&identity.service_dir).unwrap();
        let stderr = fs::File::create(&stderr_path).unwrap();
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "infrastructure::workspace_services::tests::startup_failure_child_fixture",
                "--nocapture",
            ])
            .env("UNICA_STARTUP_FAILURE_CHILD_FIXTURE", "1")
            .stdout(Stdio::null())
            .stderr(Stdio::from(stderr));
        let mut child = ManagedStartupChild::spawn_configured(command).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while child.is_running().unwrap() {
            assert!(Instant::now() < deadline, "fixture did not exit");
            thread::yield_now();
        }

        let error = workspace_service_startup_failure(
            "workspace service did not become ready",
            &mut child,
            &stderr_path,
        );

        assert!(error.contains("exited before readiness"), "{error}");
        assert!(error.contains("23"), "{error}");
        assert!(error.contains("issue-339-startup-tail-marker"), "{error}");
        assert!(!error.contains("issue-339-startup-head-marker"), "{error}");
        assert!(error.len() <= SERVICE_STARTUP_STDERR_TAIL_LIMIT as usize + 512);
        child.terminate_bounded(Duration::from_secs(1)).unwrap();
        cleanup(&context);
    }

    #[test]
    fn startup_stderr_tail_stays_bounded_after_lossy_utf8_decoding() {
        let context = test_context("startup-stderr-invalid-utf8");
        let stderr_path = context.cache_root.join("service.stderr.log");
        fs::create_dir_all(&context.cache_root).unwrap();
        let marker = b"issue-339-startup-tail-marker";
        let mut stderr = vec![0xff; SERVICE_STARTUP_STDERR_TAIL_LIMIT as usize];
        stderr.extend_from_slice(marker);
        fs::write(&stderr_path, stderr).unwrap();

        let rendered = read_workspace_service_stderr_tail(&stderr_path).unwrap();

        assert!(rendered.contains("issue-339-startup-tail-marker"));
        assert!(
            rendered.len() <= SERVICE_STARTUP_STDERR_TAIL_LIMIT as usize,
            "lossy UTF-8 expanded the diagnostic to {} bytes",
            rendered.len()
        );
        cleanup(&context);
    }

    #[test]
    fn startup_failure_reports_live_child_without_assuming_deadline_expired() {
        let context = test_context("startup-failure-live-child");
        let identity =
            WorkspaceServiceIdentity::new(&context, &context.workspace_root.join("src")).unwrap();
        let stderr_path = identity.service_dir.join("service.stderr.log");
        fs::create_dir_all(&identity.service_dir).unwrap();
        let stderr = fs::File::create(&stderr_path).unwrap();
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "infrastructure::workspace_services::tests::spawn_cleanup_child_fixture",
                "--nocapture",
            ])
            .env("UNICA_SPAWN_CLEANUP_CHILD_FIXTURE", "1")
            .stdout(Stdio::null())
            .stderr(Stdio::from(stderr));
        let mut child = ManagedStartupChild::spawn_configured(command).unwrap();
        thread::sleep(Duration::from_millis(75));
        assert!(child.is_running().unwrap());

        let error = workspace_service_startup_failure(
            "cancelled: workspace service readiness was cancelled",
            &mut child,
            &stderr_path,
        );

        assert!(
            error.contains("was still running when readiness failed"),
            "{error}"
        );
        assert!(!error.contains("readiness deadline"), "{error}");
        child.terminate_bounded(Duration::from_secs(1)).unwrap();
        cleanup(&context);
    }

    #[test]
    fn startup_failure_child_fixture() {
        if std::env::var_os("UNICA_STARTUP_FAILURE_CHILD_FIXTURE").is_some() {
            eprintln!("issue-339-startup-head-marker");
            eprintln!(
                "{}",
                "x".repeat(SERVICE_STARTUP_STDERR_TAIL_LIMIT as usize + 1024)
            );
            eprintln!("issue-339-startup-tail-marker");
            std::process::exit(23);
        }
    }

    #[test]
    fn spawn_cleanup_child_fixture() {
        if std::env::var_os("UNICA_SPAWN_CLEANUP_CHILD_FIXTURE").is_some() {
            thread::sleep(Duration::from_secs(30));
        }
    }

    #[test]
    fn service_config_uses_defaults_and_env_overrides() {
        std::env::remove_var("UNICA_WORKSPACE_SERVICE_IDLE_SECS");
        std::env::remove_var("UNICA_WORKSPACE_SERVICE_MAX_AGE_SECS");
        let defaults = WorkspaceServiceConfig::from_env();
        assert_eq!(defaults.idle_secs, 7200);
        assert_eq!(defaults.max_age_secs, 28800);

        std::env::set_var("UNICA_WORKSPACE_SERVICE_IDLE_SECS", "10");
        std::env::set_var("UNICA_WORKSPACE_SERVICE_MAX_AGE_SECS", "20");
        let configured = WorkspaceServiceConfig::from_env();
        assert_eq!(configured.idle_secs, 10);
        assert_eq!(configured.max_age_secs, 20);

        std::env::remove_var("UNICA_WORKSPACE_SERVICE_IDLE_SECS");
        std::env::remove_var("UNICA_WORKSPACE_SERVICE_MAX_AGE_SECS");
    }

    #[test]
    fn service_protocol_rejects_invalid_token_and_accepts_ping() {
        let context = test_context("protocol");
        let identity =
            WorkspaceServiceIdentity::new(&context, &context.workspace_root.join("src")).unwrap();
        let record = test_record(&identity, 34567, env!("CARGO_PKG_VERSION"));
        let runtime = WorkspaceServiceRuntime::new(identity, &record);

        let invalid = ServiceResponse::error(runtime.authenticate("wrong").unwrap_err());
        assert!(!invalid.ok);
        assert_eq!(
            invalid.error.as_deref(),
            Some("invalid workspace service token")
        );

        runtime.authenticate("secret").unwrap();
        let valid = runtime.ping();
        assert!(valid.ok);
        assert_eq!(valid.status.as_deref(), Some("alive"));

        cleanup(&context);
    }

    #[test]
    fn manager_reuses_matching_live_record_without_spawning() {
        let context = test_context("reuse");
        let source_root = context.workspace_root.join("src");
        let identity = WorkspaceServiceIdentity::new(&context, &source_root).unwrap();
        write_record(
            &identity,
            test_record(&identity, 34567, env!("CARGO_PKG_VERSION")),
        );
        let connector = RecordingConnector {
            ping_ok: true,
            ..Default::default()
        };
        let spawner = RecordingSpawner::default();
        let manager = WorkspaceServiceManager::with_io(&connector, &spawner);

        let record = manager.ensure_service(&context, &source_root).unwrap();

        assert_eq!(record.port, 34567);
        assert_eq!(*connector.pings.borrow(), 1);
        assert_eq!(*spawner.spawns.borrow(), 0);
        cleanup(&context);
    }

    #[test]
    fn manager_spawns_when_record_is_unreachable_or_version_mismatched() {
        let context = test_context("spawn");
        let source_root = context.workspace_root.join("src");
        let identity = WorkspaceServiceIdentity::new(&context, &source_root).unwrap();
        write_record(&identity, test_record(&identity, 34567, "older"));
        let connector = RecordingConnector::default();
        let spawner = RecordingSpawner::default();
        let manager = WorkspaceServiceManager::with_io(&connector, &spawner);

        let record = manager.ensure_service(&context, &source_root).unwrap();

        assert_eq!(record.port, 45678);
        assert_eq!(*connector.pings.borrow(), 0);
        assert_eq!(*spawner.spawns.borrow(), 1);
        cleanup(&context);
    }

    #[test]
    fn manager_waits_for_peer_spawn_lock_release_before_reusing_record() {
        let context = test_context("peer-lock");
        let source_root = context.workspace_root.join("src");
        let identity = WorkspaceServiceIdentity::new(&context, &source_root).unwrap();
        let spawn_lock = acquire_spawn_lock(&identity).unwrap().unwrap();
        write_record(
            &identity,
            test_record(&identity, 34567, env!("CARGO_PKG_VERSION")),
        );
        let releaser = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(150));
            drop(spawn_lock);
        });
        let connector = RecordingConnector {
            ping_ok: true,
            ..Default::default()
        };
        let spawner = RecordingSpawner::default();
        let manager = WorkspaceServiceManager::with_io(&connector, &spawner);
        let started = Instant::now();

        let record = manager.ensure_service(&context, &source_root).unwrap();

        releaser.join().unwrap();
        assert!(started.elapsed() >= Duration::from_millis(125));
        assert_eq!(record.port, 34567);
        assert_eq!(*connector.pings.borrow(), 1);
        assert_eq!(*spawner.spawns.borrow(), 0);
        cleanup(&context);
    }

    #[test]
    fn manager_spawn_lock_wait_observes_cancellation_and_shared_deadline() {
        let context = test_context("peer-lock-deadline");
        let source_root = context.workspace_root.join("src");
        let identity = WorkspaceServiceIdentity::new(&context, &source_root).unwrap();
        let spawn_lock = acquire_spawn_lock(&identity).unwrap().unwrap();
        let connector = RecordingConnector::default();
        let spawner = RecordingSpawner::default();
        let manager = WorkspaceServiceManager::with_io(&connector, &spawner);

        let cancellation = CancellationToken::new();
        cancellation.cancel();
        let cancelled = manager
            .ensure_service_cancellable(&context, &source_root, &cancellation)
            .unwrap_err();
        assert!(cancelled.starts_with("cancelled:"), "{cancelled}");

        let started = Instant::now();
        let timed_out = manager
            .ensure_service_cancellable_with_deadline(
                &context,
                &source_root,
                &CancellationToken::new(),
                &WorkspaceServiceCallDeadline::new(Duration::from_millis(75)),
            )
            .unwrap_err();
        assert!(timed_out.starts_with("timeout:"), "{timed_out}");
        assert!(started.elapsed() < Duration::from_millis(300));
        assert!(connector.requests.borrow().is_empty());
        assert_eq!(*spawner.spawns.borrow(), 0);

        drop(spawn_lock);
        cleanup(&context);
    }

    #[test]
    fn workspace_service_deadline_reports_actual_request_budget() {
        let deadline = WorkspaceServiceCallDeadline {
            started: Instant::now(),
            budget: Duration::ZERO,
        };

        let error = deadline.remaining(&CancellationToken::new()).unwrap_err();

        assert_eq!(
            error,
            "timeout: workspace service request exceeded 0 seconds"
        );
    }

    #[test]
    fn workspace_service_timeout_reports_a_subsecond_budget_in_milliseconds() {
        assert_eq!(
            workspace_service_request_timeout_error(Duration::from_millis(750)),
            "timeout: workspace service request exceeded 750 milliseconds"
        );
    }

    #[test]
    fn spawn_lock_contention_classifier_uses_fs2_platform_error() {
        assert!(spawn_lock_is_contended(&fs2::lock_contended_error()));
        assert!(spawn_lock_is_contended(&io::Error::from(
            ErrorKind::WouldBlock
        )));
        assert!(!spawn_lock_is_contended(&io::Error::from(
            ErrorKind::PermissionDenied
        )));
    }

    #[test]
    fn spawn_wait_ignores_live_record_not_owned_by_spawned_child() {
        let context = test_context("spawn-wait-owned-record");
        let identity =
            WorkspaceServiceIdentity::new(&context, &context.workspace_root.join("src")).unwrap();
        write_record(
            &identity,
            test_record(&identity, 34567, env!("CARGO_PKG_VERSION")),
        );
        let connector = RecordingConnector {
            ping_ok: true,
            ..RecordingConnector::default()
        };

        let result = wait_for_record_with_connector(
            &identity,
            &connector,
            45678,
            "new-spawn-token",
            Duration::from_millis(75),
            &CancellationToken::new(),
        );

        assert!(result.is_err());
        assert!(connector.requests.borrow().is_empty());
        cleanup(&context);
    }

    #[test]
    fn manager_generates_unique_uuid_operation_ids_for_bsl_and_rlm() {
        let context = test_context("operation-ids");
        let source_root = context.workspace_root.join("src");
        let identity = WorkspaceServiceIdentity::new(&context, &source_root).unwrap();
        write_record(
            &identity,
            test_record(&identity, 34567, env!("CARGO_PKG_VERSION")),
        );
        let connector = RecordingConnector {
            ping_ok: true,
            ..Default::default()
        };
        let spawner = RecordingSpawner::default();
        let manager = WorkspaceServiceManager::with_io(&connector, &spawner);

        manager
            .call_bsl_mcp(
                &context,
                &source_root,
                "search",
                json!({}),
                Duration::from_secs(1),
            )
            .unwrap();
        manager
            .rlm_readiness(&context, &source_root, &Map::new())
            .unwrap();
        manager
            .call_rlm_cancellable(
                &context,
                &source_root,
                WorkspaceRlmOperation::Search {
                    query: "needle".to_string(),
                    limit: 20,
                },
                Duration::from_secs(1),
                &CancellationToken::new(),
            )
            .unwrap();

        let ids = connector
            .requests
            .borrow()
            .iter()
            .filter_map(ServiceRequestKind::operation_id)
            .map(str::to_string)
            .collect::<Vec<_>>();
        assert_eq!(ids.len(), 3);
        assert_eq!(
            ids.iter().collect::<std::collections::HashSet<_>>().len(),
            3
        );
        assert!(ids
            .iter()
            .map(|id| Uuid::parse_str(id).unwrap())
            .all(|id| id.get_version_num() == 4));
        cleanup(&context);
    }

    #[test]
    fn rlm_request_preserves_caller_budget_above_service_default() {
        let context = test_context("rlm-caller-budget");
        let source_root = context.workspace_root.join("src");
        let identity = WorkspaceServiceIdentity::new(&context, &source_root).unwrap();
        write_record(
            &identity,
            test_record(&identity, 34567, env!("CARGO_PKG_VERSION")),
        );
        let connector = RecordingConnector {
            ping_ok: true,
            ..RecordingConnector::default()
        };
        let spawner = RecordingSpawner::default();
        let manager = WorkspaceServiceManager::with_io(&connector, &spawner);

        manager
            .call_rlm_cancellable(
                &context,
                &source_root,
                WorkspaceRlmOperation::Search {
                    query: "needle".to_string(),
                    limit: 20,
                },
                Duration::from_secs(300),
                &CancellationToken::new(),
            )
            .unwrap();

        let budget = *connector.budgets.borrow().last().unwrap();
        let request = serde_json::to_value(connector.requests.borrow().last().unwrap()).unwrap();
        let wire_timeout = Duration::new(
            request["timeout_seconds"].as_u64().unwrap(),
            request["timeout_nanos"].as_u64().unwrap() as u32,
        );
        cleanup(&context);
        assert!(budget > Duration::from_secs(299), "{budget:?}");
        assert!(budget <= Duration::from_secs(300), "{budget:?}");
        assert_eq!(wire_timeout, budget);
    }

    #[test]
    fn rlm_request_preserves_budget_beyond_u64_max_milliseconds() {
        let context = test_context("rlm-caller-budget-beyond-u64-millis");
        let source_root = context.workspace_root.join("src");
        let identity = WorkspaceServiceIdentity::new(&context, &source_root).unwrap();
        write_record(
            &identity,
            test_record(&identity, 34567, env!("CARGO_PKG_VERSION")),
        );
        let connector = RecordingConnector {
            ping_ok: true,
            ..RecordingConnector::default()
        };
        let spawner = RecordingSpawner::default();
        let manager = WorkspaceServiceManager::with_io(&connector, &spawner);

        manager
            .call_rlm_cancellable(
                &context,
                &source_root,
                WorkspaceRlmOperation::Search {
                    query: "needle".to_string(),
                    limit: 20,
                },
                Duration::from_secs(i64::MAX as u64),
                &CancellationToken::new(),
            )
            .unwrap();

        let budget = *connector.budgets.borrow().last().unwrap();
        let request = serde_json::to_value(connector.requests.borrow().last().unwrap()).unwrap();
        let wire_timeout = Duration::new(
            request["timeout_seconds"].as_u64().unwrap(),
            request["timeout_nanos"].as_u64().unwrap() as u32,
        );
        cleanup(&context);
        assert!(wire_timeout > Duration::from_millis(u64::MAX));
        assert_eq!(wire_timeout, budget);
    }

    #[test]
    fn bsl_request_preserves_budget_beyond_u64_max_milliseconds() {
        let context = test_context("bsl-caller-budget-beyond-u64-millis");
        let source_root = context.workspace_root.join("src");
        let identity = WorkspaceServiceIdentity::new(&context, &source_root).unwrap();
        write_record(
            &identity,
            test_record(&identity, 34567, env!("CARGO_PKG_VERSION")),
        );
        let connector = RecordingConnector {
            ping_ok: true,
            ..RecordingConnector::default()
        };
        let spawner = RecordingSpawner::default();
        let manager = WorkspaceServiceManager::with_io(&connector, &spawner);

        manager
            .call_bsl_mcp_cancellable_with_budget(
                &context,
                &source_root,
                WorkspaceServiceBslCall {
                    tool_name: "search",
                    tool_args: json!({"query": "needle"}),
                    timeout: Duration::from_secs(i64::MAX as u64),
                    request_budget: Duration::from_secs(i64::MAX as u64),
                },
                &CancellationToken::new(),
            )
            .unwrap();

        let budget = *connector.budgets.borrow().last().unwrap();
        let request = serde_json::to_value(connector.requests.borrow().last().unwrap()).unwrap();
        let wire_timeout = Duration::new(
            request["timeout_seconds"].as_u64().unwrap(),
            request["timeout_nanos"].as_u64().unwrap() as u32,
        );
        cleanup(&context);
        assert!(wire_timeout > Duration::from_millis(u64::MAX));
        assert_eq!(wire_timeout, budget);
    }

    #[test]
    fn rlm_readiness_preserves_caller_budget_above_service_default() {
        let context = test_context("rlm-readiness-caller-budget");
        let source_root = context.workspace_root.join("src");
        let identity = WorkspaceServiceIdentity::new(&context, &source_root).unwrap();
        write_record(
            &identity,
            test_record(&identity, 34567, env!("CARGO_PKG_VERSION")),
        );
        let connector = RecordingConnector {
            ping_ok: true,
            ..RecordingConnector::default()
        };
        let spawner = RecordingSpawner::default();
        let manager = WorkspaceServiceManager::with_io(&connector, &spawner);

        manager
            .rlm_readiness_cancellable_with_timeout(
                &context,
                &source_root,
                &Map::new(),
                Duration::new(300, 123_456_789),
                &CancellationToken::new(),
            )
            .unwrap();

        let budget = *connector.budgets.borrow().last().unwrap();
        let request = serde_json::to_value(connector.requests.borrow().last().unwrap()).unwrap();
        let wire_timeout = Duration::new(
            request["timeout_seconds"].as_u64().unwrap(),
            request["timeout_nanos"].as_u64().unwrap() as u32,
        );
        cleanup(&context);
        assert!(budget > Duration::from_secs(299), "{budget:?}");
        assert!(budget <= Duration::new(300, 123_456_789), "{budget:?}");
        assert_eq!(wire_timeout, budget);
    }

    #[test]
    fn manager_retries_rlm_request_once_after_transport_reset() {
        let context = test_context("rlm-transport-reset-retry");
        let source_root = context.workspace_root.join("src");
        let identity = WorkspaceServiceIdentity::new(&context, &source_root).unwrap();
        write_record(
            &identity,
            test_record(&identity, 34567, env!("CARGO_PKG_VERSION")),
        );
        let connector = ResetRlmConnector::default();
        let spawner = RecordingSpawner::default();
        let manager = WorkspaceServiceManager::with_io(&connector, &spawner);

        let output = manager
            .call_rlm_cancellable(
                &context,
                &source_root,
                WorkspaceRlmOperation::Search {
                    query: "needle".to_string(),
                    limit: 20,
                },
                Duration::from_secs(1),
                &CancellationToken::new(),
            )
            .unwrap();

        let WorkspaceServiceRlmCall::Output(output) = output else {
            panic!("successful retry must return RLM output");
        };
        assert_eq!(output.result_text, "[]");
        assert_eq!(*connector.calls.borrow(), 2);
        assert_eq!(connector.operation_ids.borrow().len(), 2);
        assert_ne!(
            connector.operation_ids.borrow()[0],
            connector.operation_ids.borrow()[1]
        );
        assert!(connector
            .timeouts
            .borrow()
            .iter()
            .all(|timeout| *timeout <= Duration::from_secs(1)));
        cleanup(&context);
    }

    #[test]
    fn manager_preserves_structured_post_execution_readiness() {
        struct StaleRlmConnector;

        impl ServiceConnector for StaleRlmConnector {
            fn send(
                &self,
                _record: &WorkspaceServiceRecord,
                request: ServiceRequest,
                _cancellation: &CancellationToken,
                _budget: Duration,
            ) -> Result<ServiceResponse, String> {
                match request.kind {
                    ServiceRequestKind::Ping => Ok(ServiceResponse {
                        ok: true,
                        status: Some("alive".to_string()),
                        ..ServiceResponse::default()
                    }),
                    ServiceRequestKind::RlmMcp { .. } => {
                        Ok(ServiceResponse::unavailable_rlm_execution(
                            IndexReadiness::Stale {
                                status: "source generation changed".to_string(),
                            },
                            Vec::new(),
                        ))
                    }
                    _ => panic!("unexpected request"),
                }
            }
        }

        let context = test_context("rlm-structured-post-readiness");
        let source_root = context.workspace_root.join("src");
        let identity = WorkspaceServiceIdentity::new(&context, &source_root).unwrap();
        write_record(
            &identity,
            test_record(&identity, 34567, env!("CARGO_PKG_VERSION")),
        );
        let spawner = RecordingSpawner::default();
        let manager = WorkspaceServiceManager::with_io(&StaleRlmConnector, &spawner);

        let result = manager
            .call_rlm_cancellable(
                &context,
                &source_root,
                WorkspaceRlmOperation::Definition {
                    name: "Needle".to_string(),
                    module_hint: String::new(),
                    limit: 50,
                },
                Duration::from_secs(1),
                &CancellationToken::new(),
            )
            .unwrap();

        let WorkspaceServiceRlmCall::Unready(IndexReadiness::Stale { status }) = result else {
            panic!("stale service response must stay structured");
        };
        assert_eq!(status, "source generation changed");
        cleanup(&context);
    }

    #[test]
    fn rlm_readiness_observes_cancellation_after_transport_returns() {
        let context = test_context("rlm-readiness-post-send-cancel");
        let source_root = context.workspace_root.join("src");
        let identity = WorkspaceServiceIdentity::new(&context, &source_root).unwrap();
        write_record(
            &identity,
            test_record(&identity, 34567, env!("CARGO_PKG_VERSION")),
        );
        let connector = CancelAfterRlmReadyConnector;
        let spawner = RecordingSpawner::default();
        let manager = WorkspaceServiceManager::with_io(&connector, &spawner);
        let cancellation = CancellationToken::new();

        let error = manager
            .rlm_readiness_cancellable_with_timeout(
                &context,
                &source_root,
                &Map::new(),
                Duration::from_secs(1),
                &cancellation,
            )
            .unwrap_err();

        assert!(error.starts_with("cancelled:"), "{error}");
        cleanup(&context);
    }

    #[test]
    fn manager_retries_bsl_request_once_after_transport_reset() {
        let context = test_context("bsl-transport-reset-retry");
        let source_root = context.workspace_root.join("src");
        let identity = WorkspaceServiceIdentity::new(&context, &source_root).unwrap();
        write_record(
            &identity,
            test_record(&identity, 34567, env!("CARGO_PKG_VERSION")),
        );
        let connector = ResetBslConnector {
            recover: true,
            ..ResetBslConnector::default()
        };
        let spawner = RecordingSpawner::default();
        let manager = WorkspaceServiceManager::with_io(&connector, &spawner);

        let output = manager
            .call_bsl_mcp(
                &context,
                &source_root,
                "diagnostics",
                json!({"mode": "file", "path": "CommonModules/Example/Module.bsl"}),
                Duration::from_secs(1),
            )
            .unwrap();

        assert_eq!(output.result_text, "recovered");
        assert_eq!(*connector.pings.borrow(), 2);
        assert_eq!(*connector.bsl_calls.borrow(), 2);
        let operation_ids = connector.operation_ids.borrow();
        assert_eq!(operation_ids.len(), 2);
        assert_ne!(operation_ids[0], operation_ids[1]);
        assert_eq!(*spawner.spawns.borrow(), 0);
        cleanup(&context);
    }

    #[test]
    fn manager_shares_one_deadline_across_late_transport_retry() {
        let context = test_context("bsl-transport-shared-deadline");
        let source_root = context.workspace_root.join("src");
        let identity = WorkspaceServiceIdentity::new(&context, &source_root).unwrap();
        write_record(
            &identity,
            test_record(&identity, 34567, env!("CARGO_PKG_VERSION")),
        );
        let connector = ResetBslConnector {
            recover: true,
            first_bsl_delay: Duration::from_millis(250),
            ..ResetBslConnector::default()
        };
        let spawner = RecordingSpawner::default();
        let manager = WorkspaceServiceManager::with_io(&connector, &spawner);

        let output = manager
            .call_bsl_mcp_cancellable_with_budget(
                &context,
                &source_root,
                WorkspaceServiceBslCall {
                    tool_name: "diagnostics",
                    tool_args: json!({"mode": "file"}),
                    timeout: Duration::from_secs(120),
                    request_budget: Duration::from_secs(1),
                },
                &CancellationToken::new(),
            )
            .unwrap();

        assert_eq!(output.result_text, "recovered");
        let ping_budgets = connector.ping_budgets.borrow();
        let bsl_budgets = connector.bsl_budgets.borrow();
        assert_eq!(ping_budgets.len(), 2);
        assert_eq!(bsl_budgets.len(), 2);
        assert!(bsl_budgets[0] <= ping_budgets[0]);
        assert!(ping_budgets[1] < bsl_budgets[0]);
        assert!(bsl_budgets[1] <= ping_budgets[1]);
        let timeouts = connector.bsl_timeouts.borrow();
        assert_eq!(timeouts.len(), 2);
        assert!(timeouts
            .iter()
            .all(|timeout| *timeout <= Duration::from_secs(1)));
        assert!(timeouts[1] < timeouts[0]);
        assert_eq!(*spawner.spawns.borrow(), 0);
        cleanup(&context);
    }

    #[test]
    fn manager_passes_remaining_budget_and_fresh_token_to_retry_spawn() {
        let context = test_context("bsl-retry-spawn-budget-token");
        let source_root = context.workspace_root.join("src");
        let identity = WorkspaceServiceIdentity::new(&context, &source_root).unwrap();
        write_record(
            &identity,
            test_record(&identity, 34567, env!("CARGO_PKG_VERSION")),
        );
        let connector = ResetBslConnector {
            recover: true,
            restart_after_reset: true,
            first_bsl_delay: Duration::from_millis(250),
            ..ResetBslConnector::default()
        };
        let spawner = RecordingSpawner::default();
        let manager = WorkspaceServiceManager::with_io(&connector, &spawner);

        let output = manager
            .call_bsl_mcp_cancellable_with_budget(
                &context,
                &source_root,
                WorkspaceServiceBslCall {
                    tool_name: "diagnostics",
                    tool_args: json!({"mode": "file"}),
                    timeout: Duration::from_secs(120),
                    request_budget: Duration::from_secs(1),
                },
                &CancellationToken::new(),
            )
            .unwrap();

        assert_eq!(output.result_text, "recovered");
        assert_eq!(*spawner.spawns.borrow(), 1);
        let spawn_budgets = spawner.budgets.borrow();
        assert_eq!(spawn_budgets.len(), 1);
        assert!(spawn_budgets[0] < Duration::from_secs(1));
        let spawned_tokens = spawner.tokens.borrow();
        let bsl_record_tokens = connector.bsl_record_tokens.borrow();
        let bsl_request_tokens = connector.bsl_request_tokens.borrow();
        assert_eq!(bsl_record_tokens.len(), 2);
        assert_eq!(bsl_record_tokens[0], "secret");
        assert_eq!(bsl_record_tokens[1], spawned_tokens[0]);
        assert_eq!(*bsl_request_tokens, *bsl_record_tokens);
        cleanup(&context);
    }

    #[test]
    fn manager_bounds_delayed_retry_spawn_by_remaining_deadline() {
        let context = test_context("bsl-retry-delayed-spawn-budget");
        let source_root = context.workspace_root.join("src");
        let identity = WorkspaceServiceIdentity::new(&context, &source_root).unwrap();
        write_record(
            &identity,
            test_record(&identity, 34567, env!("CARGO_PKG_VERSION")),
        );
        let connector = ResetBslConnector {
            restart_after_reset: true,
            first_bsl_delay: Duration::from_millis(100),
            ..ResetBslConnector::default()
        };
        let spawner = RecordingSpawner {
            delay: Duration::from_secs(2),
            ..RecordingSpawner::default()
        };
        let manager = WorkspaceServiceManager::with_io(&connector, &spawner);
        let started = Instant::now();

        let error = manager
            .call_bsl_mcp_cancellable_with_budget(
                &context,
                &source_root,
                WorkspaceServiceBslCall {
                    tool_name: "diagnostics",
                    tool_args: json!({"mode": "file"}),
                    timeout: Duration::from_secs(120),
                    request_budget: Duration::from_secs(1),
                },
                &CancellationToken::new(),
            )
            .unwrap_err();

        assert!(error.starts_with("timeout:"), "{error}");
        assert!(started.elapsed() < Duration::from_millis(1_500));
        assert_eq!(*spawner.spawns.borrow(), 1);
        assert!(spawner.budgets.borrow()[0] < Duration::from_secs(1));
        cleanup(&context);
    }

    #[test]
    fn manager_does_not_spawn_when_retry_shutdown_observes_cancellation() {
        let context = test_context("bsl-retry-cancel-before-spawn");
        let source_root = context.workspace_root.join("src");
        let identity = WorkspaceServiceIdentity::new(&context, &source_root).unwrap();
        write_record(
            &identity,
            test_record(&identity, 34567, env!("CARGO_PKG_VERSION")),
        );
        let connector = ResetBslConnector {
            restart_after_reset: true,
            cancel_on_shutdown: true,
            ..ResetBslConnector::default()
        };
        let spawner = RecordingSpawner::default();
        let manager = WorkspaceServiceManager::with_io(&connector, &spawner);
        let cancellation = CancellationToken::new();

        let error = manager
            .call_bsl_mcp_cancellable_with_budget(
                &context,
                &source_root,
                WorkspaceServiceBslCall {
                    tool_name: "diagnostics",
                    tool_args: json!({}),
                    timeout: Duration::from_secs(120),
                    request_budget: Duration::from_secs(1),
                },
                &cancellation,
            )
            .unwrap_err();

        assert!(error.starts_with("cancelled:"), "{error}");
        assert_eq!(*spawner.spawns.borrow(), 0);
        cleanup(&context);
    }

    #[test]
    fn manager_prioritizes_cancellation_over_exhausted_retry_budget() {
        let context = test_context("bsl-cancel-over-zero-budget");
        let source_root = context.workspace_root.join("src");
        let connector = ResetBslConnector::default();
        let spawner = RecordingSpawner::default();
        let manager = WorkspaceServiceManager::with_io(&connector, &spawner);
        let cancellation = CancellationToken::new();
        cancellation.cancel();

        let error = manager
            .call_bsl_mcp_cancellable_with_budget(
                &context,
                &source_root,
                WorkspaceServiceBslCall {
                    tool_name: "diagnostics",
                    tool_args: json!({}),
                    timeout: Duration::from_secs(120),
                    request_budget: Duration::ZERO,
                },
                &cancellation,
            )
            .unwrap_err();

        assert!(error.starts_with("cancelled:"), "{error}");
        assert_eq!(*connector.pings.borrow(), 0);
        assert_eq!(*connector.bsl_calls.borrow(), 0);
        assert_eq!(*spawner.spawns.borrow(), 0);
        cleanup(&context);
    }

    #[test]
    fn manager_rejects_exhausted_budget_without_resetting_it() {
        let context = test_context("bsl-zero-budget");
        let source_root = context.workspace_root.join("src");
        let connector = ResetBslConnector::default();
        let spawner = RecordingSpawner::default();
        let manager = WorkspaceServiceManager::with_io(&connector, &spawner);

        let error = manager
            .call_bsl_mcp_cancellable_with_budget(
                &context,
                &source_root,
                WorkspaceServiceBslCall {
                    tool_name: "diagnostics",
                    tool_args: json!({}),
                    timeout: Duration::from_secs(120),
                    request_budget: Duration::ZERO,
                },
                &CancellationToken::new(),
            )
            .unwrap_err();

        assert!(error.starts_with("timeout:"), "{error}");
        assert_eq!(*connector.pings.borrow(), 0);
        assert_eq!(*connector.bsl_calls.borrow(), 0);
        assert_eq!(*spawner.spawns.borrow(), 0);
        cleanup(&context);
    }

    #[test]
    fn manager_stops_after_one_transport_retry() {
        let context = test_context("bsl-transport-reset-twice");
        let source_root = context.workspace_root.join("src");
        let identity = WorkspaceServiceIdentity::new(&context, &source_root).unwrap();
        write_record(
            &identity,
            test_record(&identity, 34567, env!("CARGO_PKG_VERSION")),
        );
        let connector = ResetBslConnector::default();
        let spawner = RecordingSpawner::default();
        let manager = WorkspaceServiceManager::with_io(&connector, &spawner);

        let error = manager
            .call_bsl_mcp(
                &context,
                &source_root,
                "diagnostics",
                json!({"mode": "file"}),
                Duration::from_secs(1),
            )
            .unwrap_err();

        assert_eq!(
            error,
            "failed to read workspace service response: Connection reset by peer (os error 54)"
        );
        assert_eq!(*connector.pings.borrow(), 2);
        assert_eq!(*connector.bsl_calls.borrow(), 2);
        assert_eq!(*spawner.spawns.borrow(), 0);
        cleanup(&context);
    }

    #[test]
    fn manager_does_not_retry_unknown_bsl_tool_after_ambiguous_reset() {
        let context = test_context("unknown-bsl-tool-no-retry");
        let source_root = context.workspace_root.join("src");
        let identity = WorkspaceServiceIdentity::new(&context, &source_root).unwrap();
        write_record(
            &identity,
            test_record(&identity, 34567, env!("CARGO_PKG_VERSION")),
        );
        let connector = ResetBslConnector::default();
        let spawner = RecordingSpawner::default();
        let manager = WorkspaceServiceManager::with_io(&connector, &spawner);

        let error = manager
            .call_bsl_mcp(
                &context,
                &source_root,
                "future_mutation",
                json!({}),
                Duration::from_secs(1),
            )
            .unwrap_err();

        assert!(error.starts_with("failed to read workspace service response:"));
        assert_eq!(*connector.pings.borrow(), 1);
        assert_eq!(*connector.bsl_calls.borrow(), 1);
        assert_eq!(*spawner.spawns.borrow(), 0);
        cleanup(&context);
    }

    #[test]
    fn manager_does_not_retry_typed_bsl_failure() {
        let context = test_context("typed-bsl-failure-no-retry");
        let source_root = context.workspace_root.join("src");
        let identity = WorkspaceServiceIdentity::new(&context, &source_root).unwrap();
        write_record(
            &identity,
            test_record(&identity, 34567, env!("CARGO_PKG_VERSION")),
        );
        let connector = TypedFailureBslConnector::default();
        let spawner = RecordingSpawner::default();
        let manager = WorkspaceServiceManager::with_io(&connector, &spawner);

        let error = manager
            .call_bsl_mcp(
                &context,
                &source_root,
                "diagnostics",
                json!({"mode": "file"}),
                Duration::from_secs(1),
            )
            .unwrap_err();

        assert_eq!(error, "typed analyzer failure");
        assert_eq!(*connector.pings.borrow(), 1);
        assert_eq!(*connector.bsl_calls.borrow(), 1);
        assert_eq!(*spawner.spawns.borrow(), 0);
        cleanup(&context);
    }

    #[test]
    fn transport_retry_classifier_excludes_terminal_errors() {
        assert_eq!(duration_timeout_millis(Duration::ZERO), 1);
        assert_eq!(duration_timeout_millis(Duration::from_nanos(1)), 1);
        assert_eq!(duration_timeout_millis(Duration::from_secs(1)), 1_000);
        assert_eq!(
            duration_timeout_millis(Duration::from_secs(1) + Duration::from_nanos(1)),
            1_001
        );
        assert!(is_retry_safe_bsl_mcp_tool("diagnostics"));
        assert!(is_retry_safe_bsl_mcp_tool("graph"));
        assert!(!is_retry_safe_bsl_mcp_tool("future_mutation"));
        for error in [
            "failed to connect workspace service: refused",
            "failed to set workspace service read timeout: invalid argument",
            "failed to set workspace service write timeout: invalid argument",
            "failed to write workspace service request: broken pipe",
            "failed to flush workspace service request: broken pipe",
            "workspace service disconnected before responding",
            "failed to read workspace service response: connection reset",
        ] {
            assert!(
                is_retryable_workspace_service_transport_error(error),
                "{error}"
            );
        }
        assert!(!is_retryable_workspace_service_transport_error(
            "timeout: workspace service request exceeded 60 seconds"
        ));
        assert!(!is_retryable_workspace_service_transport_error(
            "cancelled: workspace service operation stopped"
        ));
        assert!(!is_retryable_workspace_service_transport_error(
            "invalid workspace service response: expected value"
        ));
    }

    fn test_context(name: &str) -> WorkspaceContext {
        let root = std::env::temp_dir().join(format!(
            "unica-workspace-service-{name}-{}",
            std::process::id()
        ));
        let workspace = root.join("workspace");
        fs::create_dir_all(workspace.join("src/CommonModules")).unwrap();
        fs::create_dir_all(workspace.join("extension/CommonModules")).unwrap();
        WorkspaceContext {
            cwd: workspace.clone(),
            workspace_root: workspace.clone(),
            cache_root: root.join("cache"),
            workspace_epoch: 1,
        }
    }

    fn cleanup(context: &WorkspaceContext) {
        let _ = fs::remove_dir_all(context.cache_root.parent().unwrap_or(&context.cache_root));
    }

    fn now_secs_for_test() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
    }

    fn write_record(identity: &WorkspaceServiceIdentity, record: WorkspaceServiceRecord) {
        super::write_record(identity, &record).unwrap();
    }

    fn test_record(
        identity: &WorkspaceServiceIdentity,
        port: u16,
        version: &str,
    ) -> WorkspaceServiceRecord {
        WorkspaceServiceRecord {
            schema_version: SERVICE_SCHEMA_VERSION,
            pid: std::process::id(),
            port,
            token: "secret".to_string(),
            version: version.to_string(),
            workspace_root: identity.workspace_root.clone(),
            source_root: identity.source_root.clone(),
            started_at: now_secs_for_test(),
            last_access_at: now_secs_for_test(),
        }
    }

    #[derive(Default)]
    struct RecordingConnector {
        ping_ok: bool,
        pings: std::cell::RefCell<u32>,
        requests: std::cell::RefCell<Vec<ServiceRequestKind>>,
        budgets: std::cell::RefCell<Vec<Duration>>,
    }

    #[derive(Default)]
    struct ResetBslConnector {
        recover: bool,
        restart_after_reset: bool,
        cancel_on_shutdown: bool,
        first_bsl_delay: Duration,
        pings: std::cell::RefCell<u32>,
        bsl_calls: std::cell::RefCell<u32>,
        operation_ids: std::cell::RefCell<Vec<String>>,
        ping_budgets: std::cell::RefCell<Vec<Duration>>,
        bsl_budgets: std::cell::RefCell<Vec<Duration>>,
        bsl_timeouts: std::cell::RefCell<Vec<Duration>>,
        bsl_record_tokens: std::cell::RefCell<Vec<String>>,
        bsl_request_tokens: std::cell::RefCell<Vec<String>>,
    }

    #[derive(Default)]
    struct ResetRlmConnector {
        calls: std::cell::RefCell<u32>,
        operation_ids: std::cell::RefCell<Vec<String>>,
        timeouts: std::cell::RefCell<Vec<Duration>>,
    }

    impl ServiceConnector for ResetRlmConnector {
        fn send(
            &self,
            _record: &WorkspaceServiceRecord,
            request: ServiceRequest,
            _cancellation: &CancellationToken,
            _budget: Duration,
        ) -> Result<ServiceResponse, String> {
            match request.kind {
                ServiceRequestKind::Ping => Ok(ServiceResponse {
                    ok: true,
                    status: Some("alive".to_string()),
                    ..ServiceResponse::default()
                }),
                ServiceRequestKind::RlmMcp {
                    operation_id,
                    timeout_seconds,
                    timeout_nanos,
                    ..
                } => {
                    self.operation_ids.borrow_mut().push(operation_id);
                    self.timeouts
                        .borrow_mut()
                        .push(Duration::new(timeout_seconds, timeout_nanos));
                    let mut calls = self.calls.borrow_mut();
                    *calls += 1;
                    if *calls == 1 {
                        Err(
                            "workspace service disconnected before responding to RLM request"
                                .to_string(),
                        )
                    } else {
                        Ok(ServiceResponse {
                            ok: true,
                            result_text: Some("[]".to_string()),
                            ..ServiceResponse::default()
                        })
                    }
                }
                _ => panic!("unexpected request kind"),
            }
        }
    }

    struct CancelAfterRlmReadyConnector;

    impl ServiceConnector for CancelAfterRlmReadyConnector {
        fn send(
            &self,
            _record: &WorkspaceServiceRecord,
            request: ServiceRequest,
            cancellation: &CancellationToken,
            _budget: Duration,
        ) -> Result<ServiceResponse, String> {
            match request.kind {
                ServiceRequestKind::Ping => Ok(ServiceResponse {
                    ok: true,
                    status: Some("alive".to_string()),
                    ..ServiceResponse::default()
                }),
                ServiceRequestKind::RlmReady { .. } => {
                    cancellation.cancel();
                    Ok(ServiceResponse {
                        ok: true,
                        ..ServiceResponse::default()
                    })
                }
                _ => panic!("unexpected request kind"),
            }
        }
    }

    impl ServiceConnector for ResetBslConnector {
        fn send(
            &self,
            record: &WorkspaceServiceRecord,
            request: ServiceRequest,
            cancellation: &CancellationToken,
            budget: Duration,
        ) -> Result<ServiceResponse, String> {
            let request_token = request.token;
            match request.kind {
                ServiceRequestKind::Ping => {
                    *self.pings.borrow_mut() += 1;
                    self.ping_budgets.borrow_mut().push(budget);
                    if self.restart_after_reset && *self.bsl_calls.borrow() > 0 {
                        return Err(
                            "failed to connect workspace service: connection refused".to_string()
                        );
                    }
                    Ok(ServiceResponse {
                        ok: true,
                        status: Some("alive".to_string()),
                        ..ServiceResponse::default()
                    })
                }
                ServiceRequestKind::BslMcp {
                    operation_id,
                    timeout_seconds,
                    timeout_nanos,
                    ..
                } => {
                    self.operation_ids.borrow_mut().push(operation_id);
                    self.bsl_budgets.borrow_mut().push(budget);
                    self.bsl_timeouts
                        .borrow_mut()
                        .push(Duration::new(timeout_seconds, timeout_nanos));
                    self.bsl_record_tokens
                        .borrow_mut()
                        .push(record.token.clone());
                    self.bsl_request_tokens.borrow_mut().push(request_token);
                    let mut calls = self.bsl_calls.borrow_mut();
                    *calls += 1;
                    if *calls == 1 && !self.first_bsl_delay.is_zero() {
                        thread::sleep(self.first_bsl_delay);
                    }
                    if *calls > 1 && self.recover {
                        Ok(ServiceResponse {
                            ok: true,
                            result_text: Some("recovered".to_string()),
                            ..ServiceResponse::default()
                        })
                    } else {
                        Err(
                            "failed to read workspace service response: Connection reset by peer (os error 54)"
                                .to_string(),
                        )
                    }
                }
                ServiceRequestKind::Shutdown => {
                    if self.cancel_on_shutdown {
                        cancellation.cancel();
                    }
                    Ok(ServiceResponse {
                        ok: true,
                        shutdown: true,
                        ..ServiceResponse::default()
                    })
                }
                _ => panic!("unexpected request kind"),
            }
        }
    }

    #[derive(Default)]
    struct TypedFailureBslConnector {
        pings: std::cell::RefCell<u32>,
        bsl_calls: std::cell::RefCell<u32>,
    }

    impl ServiceConnector for TypedFailureBslConnector {
        fn send(
            &self,
            _record: &WorkspaceServiceRecord,
            request: ServiceRequest,
            _cancellation: &CancellationToken,
            _budget: Duration,
        ) -> Result<ServiceResponse, String> {
            match request.kind {
                ServiceRequestKind::Ping => {
                    *self.pings.borrow_mut() += 1;
                    Ok(ServiceResponse {
                        ok: true,
                        status: Some("alive".to_string()),
                        ..ServiceResponse::default()
                    })
                }
                ServiceRequestKind::BslMcp { .. } => {
                    *self.bsl_calls.borrow_mut() += 1;
                    Ok(ServiceResponse::error("typed analyzer failure"))
                }
                _ => panic!("unexpected request kind"),
            }
        }
    }

    impl ServiceConnector for RecordingConnector {
        fn send(
            &self,
            _record: &WorkspaceServiceRecord,
            request: ServiceRequest,
            _cancellation: &CancellationToken,
            budget: Duration,
        ) -> Result<ServiceResponse, String> {
            self.requests.borrow_mut().push(request.kind.clone());
            self.budgets.borrow_mut().push(budget);
            if matches!(request.kind, ServiceRequestKind::Ping) {
                *self.pings.borrow_mut() += 1;
            }
            if self.ping_ok {
                Ok(ServiceResponse {
                    ok: true,
                    status: Some("alive".to_string()),
                    ..ServiceResponse::default()
                })
            } else {
                Err("connection refused".to_string())
            }
        }
    }

    #[derive(Default)]
    struct RecordingSpawner {
        spawns: std::cell::RefCell<u32>,
        delay: Duration,
        budgets: std::cell::RefCell<Vec<Duration>>,
        tokens: std::cell::RefCell<Vec<String>>,
    }

    impl ServiceSpawner for RecordingSpawner {
        fn spawn(
            &self,
            identity: &WorkspaceServiceIdentity,
            _config: WorkspaceServiceConfig,
            token: &str,
            cancellation: &CancellationToken,
            budget: Duration,
        ) -> Result<WorkspaceServiceRecord, String> {
            cancellation_error(cancellation)?;
            *self.spawns.borrow_mut() += 1;
            self.budgets.borrow_mut().push(budget);
            self.tokens.borrow_mut().push(token.to_string());
            let started = Instant::now();
            while started.elapsed() < self.delay {
                cancellation_error(cancellation)?;
                let Some(remaining) = budget
                    .checked_sub(started.elapsed())
                    .filter(|remaining| !remaining.is_zero())
                else {
                    return Err(workspace_service_request_timeout_error(budget));
                };
                thread::sleep(
                    Duration::from_millis(10)
                        .min(remaining)
                        .min(self.delay.saturating_sub(started.elapsed())),
                );
            }
            cancellation_error(cancellation)?;
            if started.elapsed() >= budget {
                return Err(workspace_service_request_timeout_error(budget));
            }
            let mut record = test_record(identity, 45678, env!("CARGO_PKG_VERSION"));
            record.token = token.to_string();
            Ok(record)
        }
    }
}
