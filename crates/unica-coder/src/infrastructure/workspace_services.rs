use crate::domain::cancellation::{cancelled_error, CancellationToken};
use crate::domain::events::DomainEvent;
use crate::domain::source_roots::normalize_path_identity;
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::bundled_tools::resolve_bundled_tool;
use crate::infrastructure::plugin_runtime::find_plugin_root;
use crate::infrastructure::workspace_index::{IndexReadiness, WorkspaceIndexService};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::{hash_map::DefaultHasher, HashMap};
use std::env;
use std::fs::{self, OpenOptions};
use std::hash::{Hash, Hasher};
use std::io::{self, BufRead, BufReader, ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc, Mutex,
};
use std::thread;
use std::time::{Duration, Instant};
use uuid::Uuid;

const SERVICE_SCHEMA_VERSION: u32 = 1;
const DEFAULT_IDLE_SECS: u64 = 7200;
const DEFAULT_MAX_AGE_SECS: u64 = 28800;
const SERVICE_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const SERVICE_CONTROL_CONNECT_TIMEOUT: Duration = Duration::from_millis(500);
const SERVICE_REQUEST_TIMEOUT: Duration = Duration::from_secs(120);
const SERVICE_SHUTDOWN_GRACE: Duration = Duration::from_secs(2);
const SERVICE_RESPONSE_LINE_LIMIT: usize = 8 * 1024 * 1024;
const SERVICE_SPAWN_LOCK_STALE_SECS: u64 = 30;

static SYSTEM_SERVICE_CONNECTOR: SystemServiceConnector = SystemServiceConnector;
static SYSTEM_SERVICE_SPAWNER: SystemServiceSpawner = SystemServiceSpawner;

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

#[derive(Debug, Clone, Serialize, Deserialize)]
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
        cancellation_error(cancellation)?;
        let identity = WorkspaceServiceIdentity::new(context, source_root)?;
        if let Some(record) = self.reusable_record(&identity) {
            return Ok(record);
        }

        let started = Instant::now();
        loop {
            cancellation_error(cancellation)?;
            if let Some(spawn_lock) = acquire_spawn_lock(&identity)? {
                if let Some(record) = self.reusable_record(&identity) {
                    return Ok(record);
                }
                let token = new_token(&identity);
                let result = self.spawner.spawn(&identity, self.config, &token);
                drop(spawn_lock);
                return result;
            }

            if let Some(record) = self.wait_for_peer_service(&identity, Duration::from_millis(250))
            {
                return Ok(record);
            }
            if spawn_lock_is_stale(&identity) {
                let _ = fs::remove_file(spawn_lock_path(&identity));
                continue;
            }
            if started.elapsed() >= SERVICE_CONNECT_TIMEOUT {
                return Err(format!(
                    "workspace service spawn is locked and did not become ready at {}",
                    identity.record_path().display()
                ));
            }
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
        let record = self.ensure_service_cancellable(context, source_root, cancellation)?;
        cancellation_error(cancellation)?;
        let response = self.connector.send(
            &record,
            ServiceRequest {
                token: record.token.clone(),
                kind: ServiceRequestKind::BslMcp {
                    operation_id: Uuid::new_v4().to_string(),
                    tool_name: tool_name.to_string(),
                    tool_args,
                    timeout_secs: timeout.as_secs().max(1),
                },
            },
            cancellation,
        )?;
        if !response.ok {
            return Err(response
                .error
                .unwrap_or_else(|| "workspace service bsl request failed".to_string()));
        }
        Ok(WorkspaceServiceBslOutput {
            result_text: response.result_text.unwrap_or_default(),
            stderr: response.stderr.unwrap_or_default(),
        })
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
        let record = self.ensure_service_cancellable(context, source_root, cancellation)?;
        cancellation_error(cancellation)?;
        let response = self.connector.send(
            &record,
            ServiceRequest {
                token: record.token.clone(),
                kind: ServiceRequestKind::RlmReady {
                    operation_id: Uuid::new_v4().to_string(),
                    args: Value::Object(args.clone()),
                },
            },
            cancellation,
        )?;
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
        let event_names = events
            .iter()
            .map(|event| event.name().to_string())
            .collect::<Vec<_>>();
        let Ok(workspace_root) = normalize_path_identity(&context.workspace_root) else {
            return;
        };
        let workspace_root = workspace_root.display().to_string();
        for entry in entries.flatten() {
            let record_path = entry.path().join("service.json");
            let Ok(text) = fs::read_to_string(record_path) else {
                continue;
            };
            let Ok(record) = serde_json::from_str::<WorkspaceServiceRecord>(&text) else {
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
                        events: event_names.clone(),
                    },
                },
                &CancellationToken::new(),
            );
        }
    }

    fn service_is_alive(&self, record: &WorkspaceServiceRecord) -> bool {
        let request = ServiceRequest {
            token: record.token.clone(),
            kind: ServiceRequestKind::Ping,
        };
        self.connector
            .send(record, request, &CancellationToken::new())
            .map(|response| response.ok)
            .unwrap_or(false)
    }

    fn reusable_record(
        &self,
        identity: &WorkspaceServiceIdentity,
    ) -> Option<WorkspaceServiceRecord> {
        let record = read_record(identity)?;
        if record.matches(identity, env!("CARGO_PKG_VERSION")) && self.service_is_alive(&record) {
            return Some(record);
        }
        self.shutdown_record(&record);
        None
    }

    fn wait_for_peer_service(
        &self,
        identity: &WorkspaceServiceIdentity,
        timeout: Duration,
    ) -> Option<WorkspaceServiceRecord> {
        let started = Instant::now();
        while started.elapsed() < timeout {
            if let Some(record) = self.reusable_record(identity) {
                return Some(record);
            }
            thread::sleep(Duration::from_millis(50));
        }
        None
    }

    fn shutdown_record(&self, record: &WorkspaceServiceRecord) {
        if record.token.is_empty() || record.port == 0 {
            return;
        }
        let _ = self.connector.send(
            record,
            ServiceRequest {
                token: record.token.clone(),
                kind: ServiceRequestKind::Shutdown,
            },
            &CancellationToken::new(),
        );
    }
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
    ) -> Result<ServiceResponse, String>;
}

trait ServiceSpawner {
    fn spawn(
        &self,
        identity: &WorkspaceServiceIdentity,
        config: WorkspaceServiceConfig,
        token: &str,
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
    ) -> Result<ServiceResponse, String> {
        let clock = SystemClock::new();
        self.send_with(record, request, cancellation, &SystemConnectorIo, &clock)
    }
}

impl SystemServiceConnector {
    fn send_with(
        &self,
        record: &WorkspaceServiceRecord,
        request: ServiceRequest,
        cancellation: &CancellationToken,
        io: &dyn ConnectorIo,
        clock: &dyn ConnectorClock,
    ) -> Result<ServiceResponse, String> {
        let deadline = Deadline::new(clock, SERVICE_REQUEST_TIMEOUT);
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
        if let Err(error) = stream.set_write_timeout(Some(remaining)) {
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
    deadline.remaining(clock).ok_or_else(|| {
        format!(
            "timeout: workspace service request exceeded {} seconds",
            SERVICE_REQUEST_TIMEOUT.as_secs()
        )
    })
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
        if let Err(error) = stream.set_write_timeout(Some(remaining)) {
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
            Ok(count) => written += count,
            Err(error) => {
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
    cancellation_error(cancellation)?;
    let remaining = remaining_or_timeout(deadline, clock)?;
    if let Err(error) = stream.set_write_timeout(Some(remaining)) {
        cancellation_error(cancellation)?;
        remaining_or_timeout(deadline, clock)?;
        return Err(format!(
            "failed to set workspace service write timeout: {error}"
        ));
    }
    if let Err(error) = stream.flush() {
        cancellation_error(cancellation)?;
        remaining_or_timeout(deadline, clock)?;
        return Err(format!(
            "failed to flush workspace service request: {error}"
        ));
    }
    cancellation_error(cancellation)
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
    ) -> Result<WorkspaceServiceRecord, String> {
        fs::create_dir_all(&identity.service_dir)
            .map_err(|err| format!("failed to create workspace service directory: {err}"))?;
        let stdout = fs::File::create(identity.service_dir.join("service.stdout.log"))
            .map_err(|err| format!("failed to create workspace service stdout log: {err}"))?;
        let stderr = fs::File::create(identity.service_dir.join("service.stderr.log"))
            .map_err(|err| format!("failed to create workspace service stderr log: {err}"))?;
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
        command
            .spawn()
            .map_err(|err| format!("failed to spawn workspace service: {err}"))?;

        wait_for_record(identity)
    }
}

struct WorkspaceServiceRuntime {
    identity: WorkspaceServiceIdentity,
    token: String,
    context: WorkspaceContext,
    analyzer: Mutex<Option<BslMcpSession>>,
    source_generation: Mutex<u64>,
    operations: Mutex<HashMap<String, CancellationToken>>,
    shutting_down: AtomicBool,
}

impl WorkspaceServiceRuntime {
    fn new(identity: WorkspaceServiceIdentity, token: String) -> Self {
        let context = WorkspaceContext {
            cwd: PathBuf::from(&identity.workspace_root),
            workspace_root: PathBuf::from(&identity.workspace_root),
            cache_root: service_cache_root(&identity.service_dir),
            workspace_epoch: 0,
        };
        let source_generation = source_generation(Path::new(&identity.source_root));
        Self {
            identity,
            token,
            context,
            analyzer: Mutex::new(None),
            source_generation: Mutex::new(source_generation),
            operations: Mutex::new(HashMap::new()),
            shutting_down: AtomicBool::new(false),
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

    fn invalidate(&self, events: &[String]) -> ServiceResponse {
        if events.iter().any(|event| invalidates_analyzer(event)) {
            if let Ok(mut analyzer) = self.analyzer.lock() {
                *analyzer = None;
            }
            if let Ok(mut generation) = self.source_generation.lock() {
                *generation = source_generation(Path::new(&self.identity.source_root));
            }
        }
        ServiceResponse {
            ok: true,
            status: Some("invalidated".to_string()),
            ..ServiceResponse::default()
        }
    }

    fn handle_bsl_mcp(
        &self,
        tool_name: &str,
        tool_args: Value,
        timeout_secs: u64,
        cancellation: &CancellationToken,
    ) -> ServiceResponse {
        if cancellation.is_cancelled() {
            return ServiceResponse::error(cancelled_error("workspace analyzer operation stopped"));
        }
        let mut analyzer = match self.analyzer.lock() {
            Ok(analyzer) => analyzer,
            Err(_) => return ServiceResponse::error("workspace analyzer state is unavailable"),
        };
        if cancellation.is_cancelled() {
            return ServiceResponse::error(cancelled_error("workspace analyzer operation stopped"));
        }
        let current_generation = source_generation(Path::new(&self.identity.source_root));
        if let Ok(mut generation) = self.source_generation.lock() {
            if current_generation != *generation {
                *analyzer = None;
                *generation = current_generation;
            }
        }
        let timeout = Duration::from_secs(timeout_secs.max(1));
        let result = (|| {
            if analyzer.is_none() {
                *analyzer = Some(BslMcpSession::start(
                    &self.context,
                    Path::new(&self.identity.source_root),
                )?);
            }
            analyzer
                .as_mut()
                .expect("bsl session must exist after start")
                .call(tool_name, tool_args, timeout, cancellation)
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
                    thread::spawn(move || drop(stale_session));
                }
                ServiceResponse::error(error)
            }
        }
    }

    fn handle_rlm_ready(&self, args: Value, cancellation: &CancellationToken) -> ServiceResponse {
        let mut args = args.as_object().cloned().unwrap_or_default();
        args.insert(
            "sourceDir".to_string(),
            Value::String(self.identity.source_root.clone()),
        );
        let service = WorkspaceIndexService::new();
        let start_report =
            service.start_for_workspace_cancellable(&self.context, &args, false, cancellation);
        let readiness = service.ready_index_cancellable(&self.context, &args, cancellation);
        ServiceResponse::from_readiness(readiness, start_report.warnings)
    }
}

fn invalidates_analyzer(event: &str) -> bool {
    matches!(
        event,
        "ModuleChanged"
            | "SourceSetChanged"
            | "BuildCompleted"
            | "MetadataChanged"
            | "ConfigXmlChanged"
            | "CfeChanged"
            | "FormChanged"
            | "RoleChanged"
            | "SkdChanged"
    )
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
                timeout_secs,
                ..
            } => runtime.handle_bsl_mcp(&tool_name, tool_args, timeout_secs, cancellation),
            ServiceRequestKind::RlmReady { args, .. } => {
                runtime.handle_rlm_ready(args, cancellation)
            }
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
        timeout_secs: u64,
    },
    RlmReady {
        operation_id: String,
        args: Value,
    },
    Cancel {
        operation_id: String,
    },
    Invalidate {
        events: Vec<String>,
    },
    Shutdown,
}

impl ServiceRequestKind {
    fn operation_id(&self) -> Option<&str> {
        match self {
            Self::BslMcp { operation_id, .. } | Self::RlmReady { operation_id, .. } => {
                Some(operation_id)
            }
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
            IndexReadiness::Stale => Self {
                ok: true,
                index_status: Some("stale".to_string()),
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
            Some("stale") => IndexReadiness::Stale,
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

struct BslMcpSession {
    child: Child,
    stdin: ChildStdin,
    rx: mpsc::Receiver<String>,
    stdout_reader: Option<thread::JoinHandle<()>>,
    stderr_text: Arc<Mutex<String>>,
    next_id: i64,
}

impl BslMcpSession {
    fn start(context: &WorkspaceContext, source_root: &Path) -> Result<Self, String> {
        let plugin_root = find_plugin_root(&context.cwd).ok_or_else(|| {
            "could not locate Unica plugin root for workspace bsl-analyzer service".to_string()
        })?;
        let program = resolve_bundled_tool(&plugin_root, "bsl-analyzer", true)?.program;
        let source_arg = source_root.display().to_string();
        let mut child = Command::new(&program)
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
            .current_dir(&context.cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|err| format!("failed to start persistent bsl-analyzer MCP: {err}"))?;
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| "failed to open persistent bsl-analyzer stdin".to_string())?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "failed to open persistent bsl-analyzer stdout".to_string())?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| "failed to open persistent bsl-analyzer stderr".to_string())?;

        let (tx, rx) = mpsc::channel::<String>();
        let stdout_reader = thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let mut line = String::new();
                match reader.read_line(&mut line) {
                    Ok(0) => break,
                    Ok(_) => {
                        if tx.send(line).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });
        let stderr_text = Arc::new(Mutex::new(String::new()));
        let stderr_target = Arc::clone(&stderr_text);
        thread::spawn(move || {
            let mut reader = BufReader::new(stderr);
            let mut text = String::new();
            let _ = reader.read_to_string(&mut text);
            if let Ok(mut target) = stderr_target.lock() {
                *target = text;
            }
        });

        send_json_line(
            &mut stdin,
            &json!({
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
        )?;
        let _ = read_json_response(&rx, 1, SERVICE_REQUEST_TIMEOUT)?;
        send_json_line(
            &mut stdin,
            &json!({
                "jsonrpc": "2.0",
                "method": "notifications/initialized"
            }),
        )?;

        Ok(Self {
            child,
            stdin,
            rx,
            stdout_reader: Some(stdout_reader),
            stderr_text,
            next_id: 2,
        })
    }

    fn call(
        &mut self,
        tool_name: &str,
        tool_args: Value,
        timeout: Duration,
        cancellation: &CancellationToken,
    ) -> Result<WorkspaceServiceBslOutput, String> {
        if cancellation.is_cancelled() {
            return Err(cancelled_error("persistent bsl-analyzer request stopped"));
        }
        let id = self.next_id;
        self.next_id += 1;
        send_json_line(
            &mut self.stdin,
            &json!({
                "jsonrpc": "2.0",
                "id": id,
                "method": "tools/call",
                "params": {
                    "name": tool_name,
                    "arguments": tool_args
                }
            }),
        )?;
        let response = read_json_response_cancellable(&self.rx, id, timeout, cancellation)?;
        let result_text = mcp_tool_text(&response)?;
        let stderr = self
            .stderr_text
            .lock()
            .map(|text| text.clone())
            .unwrap_or_default();
        Ok(WorkspaceServiceBslOutput {
            result_text,
            stderr,
        })
    }
}

impl Drop for BslMcpSession {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(handle) = self.stdout_reader.take() {
            let _ = handle.join();
        }
    }
}

fn env_u64(name: &str, default: u64) -> u64 {
    env::var(name)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(default)
}

fn read_record(identity: &WorkspaceServiceIdentity) -> Option<WorkspaceServiceRecord> {
    let text = fs::read_to_string(identity.record_path()).ok()?;
    serde_json::from_str(&text).ok()
}

fn write_record(
    identity: &WorkspaceServiceIdentity,
    record: &WorkspaceServiceRecord,
) -> Result<(), String> {
    fs::create_dir_all(&identity.service_dir)
        .map_err(|err| format!("failed to create workspace service state directory: {err}"))?;
    let text = serde_json::to_string_pretty(record).map_err(|err| err.to_string())?;
    fs::write(identity.record_path(), text + "\n")
        .map_err(|err| format!("failed to write workspace service record: {err}"))
}

struct SpawnLock {
    path: PathBuf,
}

impl Drop for SpawnLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn spawn_lock_path(identity: &WorkspaceServiceIdentity) -> PathBuf {
    identity.service_dir.join("service.lock")
}

fn acquire_spawn_lock(identity: &WorkspaceServiceIdentity) -> Result<Option<SpawnLock>, String> {
    fs::create_dir_all(&identity.service_dir)
        .map_err(|err| format!("failed to create workspace service lock directory: {err}"))?;
    let path = spawn_lock_path(identity);
    match OpenOptions::new().create_new(true).write(true).open(&path) {
        Ok(mut file) => {
            let payload = format!("pid={}\nstarted_at={}\n", std::process::id(), now_secs());
            file.write_all(payload.as_bytes())
                .map_err(|err| format!("failed to write workspace service spawn lock: {err}"))?;
            Ok(Some(SpawnLock { path }))
        }
        Err(error) if error.kind() == ErrorKind::AlreadyExists => Ok(None),
        Err(error) => Err(format!(
            "failed to acquire workspace service spawn lock {}: {error}",
            path.display()
        )),
    }
}

fn spawn_lock_is_stale(identity: &WorkspaceServiceIdentity) -> bool {
    let Ok(metadata) = fs::metadata(spawn_lock_path(identity)) else {
        return false;
    };
    metadata
        .modified()
        .ok()
        .and_then(|modified| modified.elapsed().ok())
        .map(|age| age >= Duration::from_secs(SERVICE_SPAWN_LOCK_STALE_SECS))
        .unwrap_or(false)
}

fn wait_for_record(identity: &WorkspaceServiceIdentity) -> Result<WorkspaceServiceRecord, String> {
    let started = Instant::now();
    while started.elapsed() < SERVICE_CONNECT_TIMEOUT {
        if let Some(record) = read_record(identity) {
            if record.matches(identity, env!("CARGO_PKG_VERSION"))
                && SYSTEM_SERVICE_CONNECTOR
                    .send(
                        &record,
                        ServiceRequest {
                            token: record.token.clone(),
                            kind: ServiceRequestKind::Ping,
                        },
                        &CancellationToken::new(),
                    )
                    .map(|response| response.ok)
                    .unwrap_or(false)
            {
                return Ok(record);
            }
        }
        thread::sleep(Duration::from_millis(50));
    }
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

fn source_generation(source_root: &Path) -> u64 {
    let mut hasher = DefaultHasher::new();
    hash_source_path(&mut hasher, source_root, 0);
    hasher.finish()
}

fn hash_source_path(hasher: &mut DefaultHasher, path: &Path, depth: usize) {
    if depth > 8 {
        return;
    }
    let Ok(metadata) = path.metadata() else {
        0_u8.hash(hasher);
        return;
    };
    path.display().to_string().hash(hasher);
    metadata.len().hash(hasher);
    if let Ok(modified) = metadata.modified() {
        if let Ok(duration) = modified.duration_since(std::time::UNIX_EPOCH) {
            duration.as_secs().hash(hasher);
            duration.subsec_nanos().hash(hasher);
        }
    }
    if !metadata.is_dir() {
        return;
    }
    let Ok(entries) = fs::read_dir(path) else {
        return;
    };
    let mut paths = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_dir()
                || matches!(
                    path.extension().and_then(|value| value.to_str()),
                    Some("bsl" | "xml" | "yaml" | "yml")
                )
        })
        .collect::<Vec<_>>();
    paths.sort();
    for child in paths.into_iter().take(20_000) {
        hash_source_path(hasher, &child, depth + 1);
    }
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

    let runtime = Arc::new(WorkspaceServiceRuntime::new(identity, token));
    serve_workspace_service(
        listener,
        runtime,
        Arc::new(SystemWorkspaceServiceOperationExecutor),
        Duration::from_secs(idle_secs.max(1)),
        Duration::from_secs(max_age_secs.max(1)),
    )
}

fn serve_workspace_service<E: WorkspaceServiceOperationExecutor + 'static>(
    listener: TcpListener,
    runtime: Arc<WorkspaceServiceRuntime>,
    executor: Arc<E>,
    idle_timeout: Duration,
    max_age: Duration,
) -> Result<(), String> {
    let started = Instant::now();
    let mut last_access = Instant::now();
    let mut handlers: Vec<thread::JoinHandle<Result<(), String>>> = Vec::new();
    let mut result = Ok(());
    loop {
        let mut index = 0;
        while index < handlers.len() {
            if handlers[index].is_finished() {
                let handler = handlers.swap_remove(index);
                let _ = handler.join();
            } else {
                index += 1;
            }
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
                if let Some(mut record) = read_record(&runtime.identity) {
                    record.last_access_at = now_secs();
                    let _ = write_record(&runtime.identity, &record);
                }
                let handler_runtime = Arc::clone(&runtime);
                let handler_executor = Arc::clone(&executor);
                handlers.push(thread::spawn(move || {
                    handle_workspace_service_stream(stream, handler_runtime, handler_executor)
                }));
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
    let drain_deadline = Instant::now() + SERVICE_SHUTDOWN_GRACE;
    while !handlers.is_empty() && Instant::now() < drain_deadline {
        let mut index = 0;
        while index < handlers.len() {
            if handlers[index].is_finished() {
                let handler = handlers.swap_remove(index);
                let _ = handler.join();
            } else {
                index += 1;
            }
        }
        if !handlers.is_empty() {
            thread::sleep(Duration::from_millis(10));
        }
    }
    let _ = fs::remove_file(runtime.identity.record_path());
    result
}

fn handle_workspace_service_stream<E: WorkspaceServiceOperationExecutor + 'static>(
    stream: TcpStream,
    runtime: Arc<WorkspaceServiceRuntime>,
    executor: Arc<E>,
) -> Result<(), String> {
    stream
        .set_read_timeout(Some(SERVICE_REQUEST_TIMEOUT))
        .map_err(|err| format!("failed to set workspace service request read timeout: {err}"))?;
    stream
        .set_write_timeout(Some(SERVICE_REQUEST_TIMEOUT))
        .map_err(|err| format!("failed to set workspace service response write timeout: {err}"))?;
    let mut reader = BufReader::new(
        stream
            .try_clone()
            .map_err(|err| format!("failed to clone workspace service stream: {err}"))?,
    );
    let mut line = String::new();
    reader
        .read_line(&mut line)
        .map_err(|err| format!("failed to read workspace service request: {err}"))?;
    let request = match serde_json::from_str::<ServiceRequest>(line.trim()) {
        Ok(request) => request,
        Err(error) => {
            let response =
                ServiceResponse::error(format!("invalid workspace service request: {error}"));
            write_service_response(stream, &response, false)?;
            return Ok(());
        }
    };
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
        kind @ (ServiceRequestKind::BslMcp { .. } | ServiceRequestKind::RlmReady { .. }) => {
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
            thread::spawn(move || {
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

fn send_json_line(stdin: &mut impl Write, payload: &Value) -> Result<(), String> {
    stdin
        .write_all(payload.to_string().as_bytes())
        .and_then(|_| stdin.write_all(b"\n"))
        .and_then(|_| stdin.flush())
        .map_err(|err| format!("failed to write persistent bsl-analyzer request: {err}"))
}

fn read_json_response(
    rx: &mpsc::Receiver<String>,
    id: i64,
    timeout: Duration,
) -> Result<Value, String> {
    read_json_response_cancellable(rx, id, timeout, &CancellationToken::new())
}

fn read_json_response_cancellable(
    rx: &mpsc::Receiver<String>,
    id: i64,
    timeout: Duration,
    cancellation: &CancellationToken,
) -> Result<Value, String> {
    let started = Instant::now();
    while started.elapsed() < timeout {
        if cancellation.is_cancelled() {
            return Err(cancelled_error(format!(
                "persistent bsl-analyzer request {id} stopped"
            )));
        }
        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(line) => {
                if cancellation.is_cancelled() {
                    return Err(cancelled_error(format!(
                        "persistent bsl-analyzer request {id} stopped"
                    )));
                }
                let Ok(value) = serde_json::from_str::<Value>(line.trim()) else {
                    continue;
                };
                if value.get("id").and_then(Value::as_i64) == Some(id) {
                    return Ok(value);
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err("persistent bsl-analyzer stdout closed before response".to_string());
            }
        }
    }
    if cancellation.is_cancelled() {
        return Err(cancelled_error(format!(
            "persistent bsl-analyzer request {id} stopped"
        )));
    }
    Err(format!("persistent bsl-analyzer request {id} timed out"))
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
    use std::fs;
    use std::path::Path;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[derive(Default)]
    struct BlockingWorkspaceExecutor {
        started: Mutex<Vec<String>>,
        cancelled: Mutex<Vec<String>>,
        release_cancelled: Mutex<bool>,
        wake: std::sync::Condvar,
    }

    impl BlockingWorkspaceExecutor {
        fn wait_started(&self, expected: usize) {
            let deadline = Instant::now() + Duration::from_secs(2);
            let mut started = self.started.lock().unwrap();
            while started.len() < expected {
                let remaining = deadline.saturating_duration_since(Instant::now());
                assert!(
                    !remaining.is_zero(),
                    "work did not start before test deadline"
                );
                let (next, timeout) = self.wake.wait_timeout(started, remaining).unwrap();
                started = next;
                assert!(!timeout.timed_out() || started.len() >= expected);
            }
        }

        fn wait_cancelled(&self, expected: usize) {
            let deadline = Instant::now() + Duration::from_secs(2);
            let mut cancelled = self.cancelled.lock().unwrap();
            while cancelled.len() < expected {
                let remaining = deadline.saturating_duration_since(Instant::now());
                assert!(
                    !remaining.is_zero(),
                    "operation was not cancelled before test deadline"
                );
                let (next, timeout) = self.wake.wait_timeout(cancelled, remaining).unwrap();
                cancelled = next;
                assert!(!timeout.timed_out() || cancelled.len() >= expected);
            }
        }

        fn release_cancelled(&self) {
            *self.release_cancelled.lock().unwrap() = true;
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
            {
                let mut started = self.started.lock().unwrap();
                started.push(operation_id.clone());
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
                self.cancelled.lock().unwrap().push(operation_id.clone());
                self.wake.notify_all();
                if operation_id.starts_with("held-after-cancel") {
                    let mut released = self.release_cancelled.lock().unwrap();
                    while !*released {
                        released = self.wake.wait(released).unwrap();
                    }
                }
                ServiceResponse::error(cancelled_error("workspace operation stopped"))
            } else {
                ServiceResponse::error("test operation was not cancelled")
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
        let context = test_context(name);
        let identity =
            WorkspaceServiceIdentity::new(&context, &context.workspace_root.join("src")).unwrap();
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        let record = test_record(&identity, port, env!("CARGO_PKG_VERSION"));
        write_record(&identity, record.clone());
        let runtime = Arc::new(WorkspaceServiceRuntime::new(identity, record.token.clone()));
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
            )
        });
        (context, record, runtime, executor, server)
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
    fn workspace_service_control_path_shutdown_cancels_all_and_rejects_new_work() {
        let (context, record, runtime, executor, server) =
            workspace_control_test_server("control-shutdown");
        let mut first = open_test_request(
            &record,
            ServiceRequestKind::RlmReady {
                operation_id: "blocked-first".to_string(),
                args: json!({}),
            },
        );
        let mut second = open_test_request(
            &record,
            ServiceRequestKind::RlmReady {
                operation_id: "blocked-second".to_string(),
                args: json!({}),
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
    fn workspace_service_control_path_disconnect_cancels_only_its_operation() {
        let (context, record, _runtime, executor, server) =
            workspace_control_test_server("control-disconnect");
        let disconnected = open_test_request(
            &record,
            ServiceRequestKind::RlmReady {
                operation_id: "blocked-disconnected".to_string(),
                args: json!({}),
            },
        );
        executor.wait_started(1);
        drop(disconnected);
        executor.wait_cancelled(1);
        assert_eq!(
            executor.cancelled.lock().unwrap().as_slice(),
            &["blocked-disconnected".to_string()]
        );

        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let recovered = send_test_request(
                &record,
                ServiceRequestKind::RlmReady {
                    operation_id: "success-after-disconnect".to_string(),
                    args: json!({}),
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
        let (_tx, rx) = mpsc::channel::<String>();
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        let started = Instant::now();

        let error = read_json_response_cancellable(&rx, 7, Duration::from_secs(30), &cancellation)
            .unwrap_err();

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
            },
        );
        executor.wait_started(1);

        let duplicate = send_test_request(
            &record,
            ServiceRequestKind::RlmReady {
                operation_id: "duplicate-id".to_string(),
                args: json!({}),
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
    }

    impl ConnectorIo for BudgetIo {
        fn connect(&self, _port: u16, timeout: Duration) -> io::Result<Box<dyn ConnectorStream>> {
            assert_eq!(timeout, self.expected_connect_timeout);
            self.clock.advance(self.connect_advance);
            Ok(Box::new(BudgetStream {
                clock: self.clock.clone(),
                write_timeouts: Arc::clone(&self.write_timeouts),
                first_write_advance: self.first_write_advance,
                writes: 0,
            }))
        }
    }

    struct BudgetStream {
        clock: ManualClock,
        write_timeouts: Arc<Mutex<Vec<Duration>>>,
        first_write_advance: Duration,
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
    fn cancellable_connector_work_write_uses_budget_remaining_after_connect() {
        let clock = ManualClock::default();
        let write_timeouts = Arc::new(Mutex::new(Vec::new()));
        let io = BudgetIo {
            clock: clock.clone(),
            write_timeouts: Arc::clone(&write_timeouts),
            expected_connect_timeout: SERVICE_CONNECT_TIMEOUT,
            connect_advance: Duration::from_secs(3),
            first_write_advance: Duration::ZERO,
        };
        let cancellation = CancellationToken::new();
        let _ = SYSTEM_SERVICE_CONNECTOR.send_with(
            &connector_test_record(),
            connector_test_request(ServiceRequestKind::BslMcp {
                operation_id: "work-budget".to_string(),
                tool_name: "search".to_string(),
                tool_args: json!({}),
                timeout_secs: 120,
            }),
            &cancellation,
            &io,
            &clock,
        );

        assert_eq!(
            write_timeouts.lock().unwrap().first().copied(),
            Some(Duration::from_secs(117))
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
                        timeout_secs: 120,
                    }),
                    &cancellation,
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
            timeout_secs: 5,
        };
        let rlm = ServiceRequestKind::RlmReady {
            operation_id: Uuid::new_v4().to_string(),
            args: json!({"sourceDir": "src"}),
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
        write_record(&identity, record);
        let runtime = WorkspaceServiceRuntime::new(identity.clone(), "secret".to_string());
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
                        timeout_secs: 120,
                    },
                },
                &caller_token,
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
            schema_version: 1,
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

        let mut mismatched_version = record.clone();
        mismatched_version.version = "older".to_string();
        assert!(!mismatched_version.matches(&identity, env!("CARGO_PKG_VERSION")));

        let mut mismatched_source = record;
        mismatched_source.source_root = context.workspace_root.join("other").display().to_string();
        assert!(!mismatched_source.matches(&identity, env!("CARGO_PKG_VERSION")));

        cleanup(&context);
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
        let runtime = WorkspaceServiceRuntime::new(identity, "secret".to_string());

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
    fn manager_waits_for_peer_spawn_lock_and_reuses_record() {
        let context = test_context("peer-lock");
        let source_root = context.workspace_root.join("src");
        let identity = WorkspaceServiceIdentity::new(&context, &source_root).unwrap();
        let spawn_lock = acquire_spawn_lock(&identity).unwrap().unwrap();
        let writer_identity = identity.clone();
        let writer = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(75));
            write_record(
                &writer_identity,
                test_record(&writer_identity, 34567, env!("CARGO_PKG_VERSION")),
            );
        });
        let connector = RecordingConnector {
            ping_ok: true,
            ..Default::default()
        };
        let spawner = RecordingSpawner::default();
        let manager = WorkspaceServiceManager::with_io(&connector, &spawner);

        let record = manager.ensure_service(&context, &source_root).unwrap();

        writer.join().unwrap();
        drop(spawn_lock);
        assert_eq!(record.port, 34567);
        assert_eq!(*spawner.spawns.borrow(), 0);
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

        let ids = connector
            .requests
            .borrow()
            .iter()
            .filter_map(ServiceRequestKind::operation_id)
            .map(str::to_string)
            .collect::<Vec<_>>();
        assert_eq!(ids.len(), 2);
        assert_ne!(ids[0], ids[1]);
        assert!(ids
            .iter()
            .map(|id| Uuid::parse_str(id).unwrap())
            .all(|id| id.get_version_num() == 4));
        cleanup(&context);
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
        fs::create_dir_all(&identity.service_dir).unwrap();
        fs::write(
            identity.record_path(),
            serde_json::to_string_pretty(&record).unwrap() + "\n",
        )
        .unwrap();
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
    }

    impl ServiceConnector for RecordingConnector {
        fn send(
            &self,
            _record: &WorkspaceServiceRecord,
            request: ServiceRequest,
            _cancellation: &CancellationToken,
        ) -> Result<ServiceResponse, String> {
            self.requests.borrow_mut().push(request.kind.clone());
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
    }

    impl ServiceSpawner for RecordingSpawner {
        fn spawn(
            &self,
            identity: &WorkspaceServiceIdentity,
            _config: WorkspaceServiceConfig,
            _token: &str,
        ) -> Result<WorkspaceServiceRecord, String> {
            *self.spawns.borrow_mut() += 1;
            Ok(test_record(identity, 45678, env!("CARGO_PKG_VERSION")))
        }
    }
}
