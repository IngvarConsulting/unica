use crate::domain::events::DomainEvent;
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::bundled_tools::resolve_bundled_tool;
use crate::infrastructure::legacy_scripts::find_plugin_root;
use crate::infrastructure::workspace_index::{IndexReadiness, WorkspaceIndexService};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::hash_map::DefaultHasher;
use std::env;
use std::fs::{self, OpenOptions};
use std::hash::{Hash, Hasher};
use std::io::{BufRead, BufReader, ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Component, Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const SERVICE_SCHEMA_VERSION: u32 = 1;
const DEFAULT_IDLE_SECS: u64 = 7200;
const DEFAULT_MAX_AGE_SECS: u64 = 28800;
const SERVICE_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const SERVICE_REQUEST_TIMEOUT: Duration = Duration::from_secs(120);
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
        let workspace_root = canonical_display(&context.workspace_root);
        let source_root = canonical_display(source_root);
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

    pub fn ensure_service(
        &self,
        context: &WorkspaceContext,
        source_root: &Path,
    ) -> Result<WorkspaceServiceRecord, String> {
        let identity = WorkspaceServiceIdentity::new(context, source_root)?;
        if let Some(record) = self.reusable_record(&identity) {
            return Ok(record);
        }

        let started = Instant::now();
        loop {
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

    pub fn call_bsl_mcp(
        &self,
        context: &WorkspaceContext,
        source_root: &Path,
        tool_name: &str,
        tool_args: Value,
        timeout: Duration,
    ) -> Result<WorkspaceServiceBslOutput, String> {
        let record = self.ensure_service(context, source_root)?;
        let response = self.connector.send(
            &record,
            ServiceRequest {
                token: record.token.clone(),
                kind: ServiceRequestKind::BslMcp {
                    tool_name: tool_name.to_string(),
                    tool_args,
                    timeout_secs: timeout.as_secs().max(1),
                },
            },
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

    pub fn rlm_readiness(
        &self,
        context: &WorkspaceContext,
        source_root: &Path,
        args: &Map<String, Value>,
    ) -> Result<IndexReadiness, String> {
        let record = self.ensure_service(context, source_root)?;
        let response = self.connector.send(
            &record,
            ServiceRequest {
                token: record.token.clone(),
                kind: ServiceRequestKind::RlmReady {
                    args: Value::Object(args.clone()),
                },
            },
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
        let workspace_root = canonical_display(&context.workspace_root);
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
            );
        }
    }

    fn service_is_alive(&self, record: &WorkspaceServiceRecord) -> bool {
        let request = ServiceRequest {
            token: record.token.clone(),
            kind: ServiceRequestKind::Ping,
        };
        self.connector
            .send(record, request)
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
        );
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

impl ServiceConnector for SystemServiceConnector {
    fn send(
        &self,
        record: &WorkspaceServiceRecord,
        request: ServiceRequest,
    ) -> Result<ServiceResponse, String> {
        let mut stream = TcpStream::connect(("127.0.0.1", record.port))
            .map_err(|err| format!("failed to connect workspace service: {err}"))?;
        stream
            .set_read_timeout(Some(SERVICE_REQUEST_TIMEOUT))
            .map_err(|err| format!("failed to set workspace service read timeout: {err}"))?;
        stream
            .set_write_timeout(Some(SERVICE_REQUEST_TIMEOUT))
            .map_err(|err| format!("failed to set workspace service write timeout: {err}"))?;
        let payload = serde_json::to_string(&request).map_err(|err| err.to_string())?;
        stream
            .write_all(payload.as_bytes())
            .and_then(|_| stream.write_all(b"\n"))
            .and_then(|_| stream.flush())
            .map_err(|err| format!("failed to write workspace service request: {err}"))?;
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        reader
            .read_line(&mut line)
            .map_err(|err| format!("failed to read workspace service response: {err}"))?;
        serde_json::from_str(line.trim())
            .map_err(|err| format!("invalid workspace service response: {err}"))
    }
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

struct WorkspaceServiceState {
    identity: WorkspaceServiceIdentity,
    token: String,
    context: WorkspaceContext,
    bsl: Option<BslMcpSession>,
    source_generation: u64,
}

impl WorkspaceServiceState {
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
            bsl: None,
            source_generation,
        }
    }

    fn handle_request(&mut self, request: ServiceRequest) -> ServiceResponse {
        if request.token != self.token {
            return ServiceResponse::error("invalid workspace service token");
        }
        match request.kind {
            ServiceRequestKind::Ping => ServiceResponse {
                ok: true,
                status: Some("alive".to_string()),
                ..ServiceResponse::default()
            },
            ServiceRequestKind::BslMcp {
                tool_name,
                tool_args,
                timeout_secs,
            } => self.handle_bsl_mcp(&tool_name, tool_args, timeout_secs),
            ServiceRequestKind::RlmReady { args } => self.handle_rlm_ready(args),
            ServiceRequestKind::Invalidate { events } => {
                if events.iter().any(|event| {
                    matches!(
                        event.as_str(),
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
                }) {
                    self.bsl = None;
                    self.source_generation =
                        source_generation(Path::new(&self.identity.source_root));
                }
                ServiceResponse {
                    ok: true,
                    status: Some("invalidated".to_string()),
                    ..ServiceResponse::default()
                }
            }
            ServiceRequestKind::Shutdown => ServiceResponse {
                ok: true,
                status: Some("shutdown".to_string()),
                shutdown: true,
                ..ServiceResponse::default()
            },
        }
    }

    fn handle_bsl_mcp(
        &mut self,
        tool_name: &str,
        tool_args: Value,
        timeout_secs: u64,
    ) -> ServiceResponse {
        let current_generation = source_generation(Path::new(&self.identity.source_root));
        if current_generation != self.source_generation {
            self.bsl = None;
            self.source_generation = current_generation;
        }
        let timeout = Duration::from_secs(timeout_secs.max(1));
        let result = (|| {
            if self.bsl.is_none() {
                self.bsl = Some(BslMcpSession::start(
                    &self.context,
                    Path::new(&self.identity.source_root),
                )?);
            }
            self.bsl
                .as_mut()
                .expect("bsl session must exist after start")
                .call(tool_name, tool_args, timeout)
        })();
        match result {
            Ok(output) => ServiceResponse {
                ok: true,
                result_text: Some(output.result_text),
                stderr: Some(output.stderr),
                ..ServiceResponse::default()
            },
            Err(error) => {
                self.bsl = None;
                ServiceResponse::error(error)
            }
        }
    }

    fn handle_rlm_ready(&mut self, args: Value) -> ServiceResponse {
        let mut args = args.as_object().cloned().unwrap_or_default();
        args.insert(
            "sourceDir".to_string(),
            Value::String(self.identity.source_root.clone()),
        );
        let service = WorkspaceIndexService::new();
        let start_report = service.start_for_workspace(&self.context, &args, false);
        let readiness = service.ready_index(&self.context, &args);
        ServiceResponse::from_readiness(readiness, start_report.warnings)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ServiceRequest {
    token: String,
    kind: ServiceRequestKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "type")]
enum ServiceRequestKind {
    Ping,
    BslMcp {
        tool_name: String,
        tool_args: Value,
        timeout_secs: u64,
    },
    RlmReady {
        args: Value,
    },
    Invalidate {
        events: Vec<String>,
    },
    Shutdown,
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
        let program = resolve_bundled_tool(&plugin_root, "bsl-analyzer").map_err(|error| {
            format!(
                "workspace bsl-analyzer service could not resolve bundled bsl-analyzer: {error}"
            )
        })?;
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
            .env("UNICA_PLUGIN_ROOT", &plugin_root)
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
    ) -> Result<WorkspaceServiceBslOutput, String> {
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
        let response = read_json_response(&self.rx, id, timeout)?;
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
    let mut record = WorkspaceServiceRecord {
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

    let mut state = WorkspaceServiceState::new(identity.clone(), token);
    let started = Instant::now();
    let mut last_access = Instant::now();
    loop {
        if started.elapsed() >= Duration::from_secs(max_age_secs.max(1)) {
            break;
        }
        if last_access.elapsed() >= Duration::from_secs(idle_secs.max(1)) {
            break;
        }
        match listener.accept() {
            Ok((stream, _addr)) => {
                last_access = Instant::now();
                record.last_access_at = now_secs();
                let shutdown = handle_stream(stream, &mut state)?;
                let _ = write_record(&identity, &record);
                if shutdown {
                    break;
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(100));
            }
            Err(error) => return Err(format!("workspace service accept failed: {error}")),
        }
    }
    let _ = fs::remove_file(identity.record_path());
    Ok(())
}

fn handle_stream(mut stream: TcpStream, state: &mut WorkspaceServiceState) -> Result<bool, String> {
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
    let response = match serde_json::from_str::<ServiceRequest>(line.trim()) {
        Ok(request) => state.handle_request(request),
        Err(error) => ServiceResponse::error(format!("invalid workspace service request: {error}")),
    };
    let shutdown = response.shutdown;
    let payload = serde_json::to_string(&response).map_err(|err| err.to_string())?;
    stream
        .write_all(payload.as_bytes())
        .and_then(|_| stream.write_all(b"\n"))
        .and_then(|_| stream.flush())
        .map_err(|err| format!("failed to write workspace service response: {err}"))?;
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
    let started = Instant::now();
    while started.elapsed() < timeout {
        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(line) => {
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

fn canonical_display(path: &Path) -> String {
    fs::canonicalize(path)
        .unwrap_or_else(|_| normalize_lexical_path(path))
        .display()
        .to_string()
}

fn normalize_lexical_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::workspace::WorkspaceContext;
    use std::fs;
    use std::path::Path;
    use std::time::{SystemTime, UNIX_EPOCH};

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
        let mut state = WorkspaceServiceState::new(identity, "secret".to_string());

        let invalid = state.handle_request(ServiceRequest {
            token: "wrong".to_string(),
            kind: ServiceRequestKind::Ping,
        });
        assert!(!invalid.ok);
        assert_eq!(
            invalid.error.as_deref(),
            Some("invalid workspace service token")
        );

        let valid = state.handle_request(ServiceRequest {
            token: "secret".to_string(),
            kind: ServiceRequestKind::Ping,
        });
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
    }

    impl ServiceConnector for RecordingConnector {
        fn send(
            &self,
            _record: &WorkspaceServiceRecord,
            request: ServiceRequest,
        ) -> Result<ServiceResponse, String> {
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
