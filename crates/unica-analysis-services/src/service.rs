//! Authenticated workspace provider leases, operation lanes and session lifecycle.
use crate::backend::{
    BackgroundTaskTracker, BslBackend, ReadyScope, RlmBackend, RlmIndex, RlmSession,
    ServiceEnvironment,
};
use crate::protocol::{ServiceRequest, ServiceRequestKind, ServiceResponse, WorkspaceRlmOperation};
use unica_process::mcp_session::{McpToolOutput, PersistentMcpSession};
use unica_work_control::elapsed_budget::ElapsedBudgetDeadline;
pub trait ServiceElapsedBudget {
    fn remaining_cancellable(&self, cancellation: &CancellationToken) -> Result<Duration, String>;
}
impl ServiceElapsedBudget for ElapsedBudgetDeadline {
    fn remaining_cancellable(&self, cancellation: &CancellationToken) -> Result<Duration, String> {
        cancellation_error(cancellation)?;
        self.remaining()
            .ok_or_else(|| workspace_service_request_timeout_error(self.budget))
    }
}

use unica_source_contract::events::{DomainEvent, DomainEventKind};
use unica_source_contract::{SourceFormat, SourceProfile, SourceSetKind};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::{hash_map::DefaultHasher, HashMap, VecDeque};
use std::env;
use std::fs;
use std::hash::{Hash, Hasher};
use std::io::{self, BufRead, BufReader, ErrorKind, Read, Seek, SeekFrom, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc, Condvar, Mutex,
};
use std::thread;
use std::time::{Duration, Instant};
use unica_process::ManagedStartupChild;
use unica_process::ProcessLog;
use unica_source_contract::workspace::WorkspaceContext;
use unica_work_control::cancellation::{cancelled_error, CancellationToken};
use unica_work_control::operation_deadline::OperationDeadline;
#[cfg(any(test, feature = "test-support"))]
use unica_workspace::actor::WorkspaceActorRuntimeTestProjection;
use unica_workspace::actor::{
    ProviderRootBinding, ProviderSourceLease, WorkspaceActor, WorkspaceActorRuntimeProjection,
    WorkspaceIdentity, WorkspaceSourceSetInput,
};
use unica_workspace::paths::normalize_path_identity;
use unica_workspace::state_fs::short_private_runtime_dir;
use unica_workspace::state_fs::{open_state_lock, open_state_reader, StateHandle};
use uuid::Uuid;

/// v5: a record names the build that serves it, not the package version.
pub const SERVICE_SCHEMA_VERSION: u32 = 5;
pub const DEFAULT_IDLE_SECS: u64 = 7200;
pub const SERVICE_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
pub const SERVICE_CONTROL_CONNECT_TIMEOUT: Duration = Duration::from_millis(500);
pub const SERVICE_REQUEST_TIMEOUT: Duration = Duration::from_secs(120);
pub const SERVICE_SHUTDOWN_GRACE: Duration = Duration::from_secs(2);
pub const SERVICE_RESPONSE_LINE_LIMIT: usize = 8 * 1024 * 1024;
pub const SERVICE_REQUEST_LINE_LIMIT: usize = 8 * 1024 * 1024;
pub const SERVICE_REQUEST_HEADER_TIMEOUT: Duration = Duration::from_secs(5);
pub const SESSION_TEARDOWN_GRACE: Duration = Duration::from_secs(3);
pub const SERVICE_SPAWN_CLEANUP_WAIT: Duration = Duration::from_secs(2);
pub const SERVICE_STARTUP_STDERR_TAIL_LIMIT: u64 = 16 * 1024;
pub const SERVICE_RECORD_LOCK_FILE: &str = "service.record.lock";
pub const LEGACY_WORKSPACE_SERVICE_PROVIDER_PROFILE: &str = "legacy-bsl-rlm";
pub const LEGACY_WORKSPACE_SERVICE_SOURCE_SET: &str = "compat";

pub static SYSTEM_SERVICE_CONNECTOR: SystemServiceConnector = SystemServiceConnector;

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
    #[cfg(any(test, feature = "test-support"))]
    pub fn test_record_path(&self) -> PathBuf {
        self.record_path()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceServiceRecord {
    pub schema_version: u32,
    pub pid: u32,
    pub port: u16,
    pub token: String,
    /// The core identity of the build serving this record. The protocol
    /// between a daemon and its service is internal and keeps no backward
    /// compatibility, so only the same build reuses a service. A record of
    /// an older schema carries none; it still parses so that its service is
    /// shut down instead of being left beside the replacement.
    #[serde(default)]
    pub build: String,
    /// The package version. Matching ignores it; it is still written because
    /// a schema-4 binary requires the field: without it that binary cannot read
    /// this record, so it neither reuses nor shuts down this service and
    /// leaves it running beside its own.
    #[serde(default)]
    pub version: String,
    pub workspace_root: String,
    pub source_root: String,
    pub started_at: u64,
    pub last_access_at: u64,
}

impl WorkspaceServiceRecord {
    pub fn matches(&self, identity: &WorkspaceServiceIdentity, build: &str) -> bool {
        self.schema_version == SERVICE_SCHEMA_VERSION
            && self.build == build
            && self.workspace_root == identity.workspace_root
            && self.source_root == identity.source_root
            && !self.token.is_empty()
            && self.port > 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkspaceServiceConfig {
    pub idle_secs: u64,
}

impl WorkspaceServiceConfig {
    pub fn from_env() -> Self {
        Self {
            idle_secs: env_u64("UNICA_WORKSPACE_SERVICE_IDLE_SECS", DEFAULT_IDLE_SECS),
        }
    }
}

pub struct WorkspaceServiceCallDeadline {
    started: Instant,
    budget: Duration,
}

pub struct WorkspaceServiceBslCall<'a> {
    tool_name: &'a str,
    tool_args: Value,
    timeout: Duration,
    request_budget: Duration,
    background: bool,
}

/// The answer to a background call that found the analyzer taken by an
/// agent's call: the editor's answer is incomplete, not failed.
pub const ANALYZER_BUSY: &str = "workspace analyzer is busy with an agent call";

impl<'a> WorkspaceServiceBslCall<'a> {
    pub fn new(
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
            background: false,
        }
    }

    /// An editor's call: answered busy instead of waiting behind an agent's.
    pub fn background(mut self) -> Self {
        self.background = true;
        self
    }
}

impl WorkspaceServiceCallDeadline {
    pub fn new(budget: Duration) -> Self {
        Self {
            started: Instant::now(),
            budget,
        }
    }

    pub fn remaining(&self, cancellation: &CancellationToken) -> Result<Duration, String> {
        cancellation_error(cancellation)?;
        self.budget
            .checked_sub(self.started.elapsed())
            .filter(|remaining| !remaining.is_zero())
            .ok_or_else(|| workspace_service_request_timeout_error(self.budget))
    }
}

pub struct WorkspaceServiceManager<'a> {
    connector: &'a dyn ServiceConnector,
    spawner: Option<&'a dyn ServiceSpawner>,
    environment: Arc<dyn ServiceEnvironment>,
    config: WorkspaceServiceConfig,
}

impl WorkspaceServiceManager<'_> {
    pub fn new(environment: Arc<dyn ServiceEnvironment>) -> Self {
        Self {
            connector: &SYSTEM_SERVICE_CONNECTOR,
            spawner: None,
            environment,
            config: WorkspaceServiceConfig::from_env(),
        }
    }
}

impl<'a> WorkspaceServiceManager<'a> {
    #[cfg(any(test, feature = "test-support"))]
    pub fn with_io(
        connector: &'a dyn ServiceConnector,
        spawner: &'a dyn ServiceSpawner,
        environment: Arc<dyn ServiceEnvironment>,
    ) -> Self {
        Self {
            connector,
            spawner: Some(spawner),
            environment,
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

    #[cfg(any(test, feature = "test-support"))]
    pub fn test_ensure_service_cancellable_with_deadline(
        &self,
        context: &WorkspaceContext,
        source_root: &Path,
        cancellation: &CancellationToken,
        deadline: &WorkspaceServiceCallDeadline,
    ) -> Result<WorkspaceServiceRecord, String> {
        self.ensure_service_cancellable_with_deadline(context, source_root, cancellation, deadline)
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
                let result = self
                    .spawner
                    .unwrap_or(&SystemServiceSpawner {
                        environment: Arc::clone(&self.environment),
                    })
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
    ) -> Result<McpToolOutput, String> {
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
    ) -> Result<McpToolOutput, String> {
        self.call_bsl_mcp_cancellable_with_budget(
            context,
            source_root,
            WorkspaceServiceBslCall::new(tool_name, tool_args, timeout, SERVICE_REQUEST_TIMEOUT),
            cancellation,
        )
    }

    pub fn call_bsl_mcp_cancellable_with_budget(
        &self,
        context: &WorkspaceContext,
        source_root: &Path,
        call: WorkspaceServiceBslCall<'_>,
        cancellation: &CancellationToken,
    ) -> Result<McpToolOutput, String> {
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
                        background: call.background,
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
            return Ok(McpToolOutput {
                result_text: response.result_text.unwrap_or_default(),
                stderr: response.stderr.unwrap_or_default(),
            });
        }
    }

    pub fn call_rlm_response(
        &self,
        context: &WorkspaceContext,
        source_root: &Path,
        operation: WorkspaceRlmOperation,
        timeout: Duration,
        cancellation: &CancellationToken,
    ) -> Result<ServiceResponse, String> {
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
            return Ok(response);
        }
    }

    #[allow(dead_code)]
    pub fn rlm_readiness_response(
        &self,
        context: &WorkspaceContext,
        source_root: &Path,
        args: &Map<String, Value>,
    ) -> Result<ServiceResponse, String> {
        self.rlm_readiness_cancellable_response(
            context,
            source_root,
            args,
            &CancellationToken::new(),
        )
    }

    pub fn rlm_readiness_cancellable_response(
        &self,
        context: &WorkspaceContext,
        source_root: &Path,
        args: &Map<String, Value>,
        cancellation: &CancellationToken,
    ) -> Result<ServiceResponse, String> {
        self.rlm_readiness_cancellable_with_timeout_response(
            context,
            source_root,
            args,
            SERVICE_REQUEST_TIMEOUT,
            cancellation,
        )
    }

    pub fn rlm_readiness_cancellable_with_timeout_response(
        &self,
        context: &WorkspaceContext,
        source_root: &Path,
        args: &Map<String, Value>,
        timeout: Duration,
        cancellation: &CancellationToken,
    ) -> Result<ServiceResponse, String> {
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
        Ok(response)
    }

    pub fn notify_invalidation(&self, context: &WorkspaceContext, events: &[DomainEvent]) {
        self.notify_changes(context, events, &CancellationToken::new(), None);
    }

    pub fn notify_committed_changes(
        &self,
        context: &WorkspaceContext,
        events: &[DomainEvent],
        cancellation: &CancellationToken,
        deadline: OperationDeadline,
    ) {
        self.notify_changes(context, events, cancellation, Some(deadline));
    }

    fn notify_changes(
        &self,
        context: &WorkspaceContext,
        events: &[DomainEvent],
        cancellation: &CancellationToken,
        caller_deadline: Option<OperationDeadline>,
    ) {
        if events.is_empty() || invalidation_wait_stopped(cancellation, caller_deadline) {
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
        #[cfg(any(test, feature = "test-support"))]
        notify_invalidation_dispatch_for_test(context, 0);
        #[cfg(any(test, feature = "test-support"))]
        let mut dispatched = 0;
        for entry in entries.flatten() {
            if invalidation_wait_stopped(cancellation, caller_deadline) {
                break;
            }
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
            if invalidation_wait_stopped(cancellation, caller_deadline) {
                break;
            }
            let request = ServiceRequest {
                token: record.token.clone(),
                kind: ServiceRequestKind::Invalidate {
                    events: typed_events.clone(),
                },
            };
            match caller_deadline {
                Some(deadline) => {
                    let _ = self
                        .connector
                        .send_before(&record, request, cancellation, deadline);
                }
                None => {
                    let _ = self.connector.send(
                        &record,
                        request,
                        cancellation,
                        SERVICE_REQUEST_TIMEOUT,
                    );
                }
            }
            #[cfg(any(test, feature = "test-support"))]
            {
                dispatched += 1;
                notify_invalidation_dispatch_for_test(context, dispatched);
            }
            if invalidation_wait_stopped(cancellation, caller_deadline) {
                break;
            }
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
        if record.matches(identity, self.environment.core_identity())
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

pub fn service_response_is_alive(response: &ServiceResponse) -> bool {
    response.ok && !response.shutdown && response.status.as_deref() == Some("alive")
}

pub fn is_retry_safe_bsl_mcp_tool(tool_name: &str) -> bool {
    matches!(tool_name, "diagnostics" | "graph")
}

pub fn duration_timeout_millis(duration: Duration) -> u64 {
    duration
        .as_nanos()
        .div_ceil(1_000_000)
        .min(u128::from(u64::MAX))
        .max(1) as u64
}

pub fn duration_from_timeout_parts(seconds: u64, nanos: u32) -> Result<Duration, String> {
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

pub fn is_retryable_workspace_service_transport_error(error: &str) -> bool {
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

#[cfg(any(test, feature = "test-support"))]
pub type InvalidationDispatchHook = dyn Fn(usize) + Send + Sync;

#[cfg(any(test, feature = "test-support"))]
pub fn invalidation_dispatch_hooks(
) -> &'static Mutex<HashMap<PathBuf, Arc<InvalidationDispatchHook>>> {
    static HOOKS: std::sync::OnceLock<Mutex<HashMap<PathBuf, Arc<InvalidationDispatchHook>>>> =
        std::sync::OnceLock::new();
    HOOKS.get_or_init(Mutex::default)
}

#[cfg(any(test, feature = "test-support"))]
pub fn notify_invalidation_dispatch_for_test(context: &WorkspaceContext, dispatched: usize) {
    let hook = invalidation_dispatch_hooks()
        .lock()
        .unwrap()
        .get(&context.workspace_root)
        .cloned();
    if let Some(hook) = hook {
        hook(dispatched);
    }
}

pub fn invalidation_wait_stopped(
    cancellation: &CancellationToken,
    deadline: Option<OperationDeadline>,
) -> bool {
    cancellation.is_cancelled()
        || deadline.is_some_and(|deadline| deadline.is_elapsed_at(Instant::now()))
}

pub fn cancellation_error(cancellation: &CancellationToken) -> Result<(), String> {
    if cancellation.is_cancelled() {
        Err(cancelled_error("workspace service operation stopped"))
    } else {
        Ok(())
    }
}

pub trait ServiceConnector {
    fn send(
        &self,
        record: &WorkspaceServiceRecord,
        request: ServiceRequest,
        cancellation: &CancellationToken,
        budget: Duration,
    ) -> Result<ServiceResponse, String>;

    fn send_before(
        &self,
        record: &WorkspaceServiceRecord,
        request: ServiceRequest,
        cancellation: &CancellationToken,
        deadline: OperationDeadline,
    ) -> Result<ServiceResponse, String>;
}

pub trait ServiceSpawner {
    fn spawn(
        &self,
        identity: &WorkspaceServiceIdentity,
        config: WorkspaceServiceConfig,
        token: &str,
        cancellation: &CancellationToken,
        budget: Duration,
    ) -> Result<WorkspaceServiceRecord, String>;
}

pub struct SystemServiceConnector;
pub struct SystemServiceSpawner {
    environment: Arc<dyn ServiceEnvironment>,
}

pub trait ConnectorClock {
    fn elapsed(&self) -> Duration;

    fn now(&self) -> Instant {
        Instant::now()
    }
}

pub struct SystemClock(Instant);

impl SystemClock {
    pub fn start() -> Self {
        Self(Instant::now())
    }
}

impl ConnectorClock for SystemClock {
    fn elapsed(&self) -> Duration {
        self.0.elapsed()
    }
}

pub struct Deadline {
    started: Duration,
    budget: Duration,
}

impl Deadline {
    pub fn new(clock: &dyn ConnectorClock, budget: Duration) -> Self {
        Self {
            started: clock.elapsed(),
            budget,
        }
    }

    pub fn remaining(&self, clock: &dyn ConnectorClock) -> Option<Duration> {
        self.budget
            .checked_sub(clock.elapsed().saturating_sub(self.started))
            .filter(|remaining| !remaining.is_zero())
    }
}

pub trait ConnectorRequestDeadline {
    fn remaining_or_timeout(&self, clock: &dyn ConnectorClock) -> Result<Option<Duration>, String>;
}

impl ConnectorRequestDeadline for Deadline {
    fn remaining_or_timeout(&self, clock: &dyn ConnectorClock) -> Result<Option<Duration>, String> {
        self.remaining(clock)
            .map(Some)
            .ok_or_else(|| workspace_service_request_timeout_error(self.budget))
    }
}

impl ConnectorRequestDeadline for OperationDeadline {
    fn remaining_or_timeout(&self, clock: &dyn ConnectorClock) -> Result<Option<Duration>, String> {
        match self.remaining_at(clock.now()) {
            Some(remaining) if remaining.is_zero() => {
                Err("timeout: workspace service operation deadline elapsed".into())
            }
            remaining => Ok(remaining),
        }
    }
}

pub trait ConnectorStream: Read + Write {
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

pub trait ConnectorIo {
    fn connect(&self, port: u16, timeout: Duration) -> io::Result<Box<dyn ConnectorStream>>;
}

pub struct SystemConnectorIo;

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
        let clock = SystemClock::start();
        self.send_with(
            record,
            request,
            cancellation,
            budget,
            &SystemConnectorIo,
            &clock,
        )
    }

    fn send_before(
        &self,
        record: &WorkspaceServiceRecord,
        request: ServiceRequest,
        cancellation: &CancellationToken,
        deadline: OperationDeadline,
    ) -> Result<ServiceResponse, String> {
        self.send_with_deadline(
            record,
            request,
            cancellation,
            &deadline,
            &SystemConnectorIo,
            &SystemClock::start(),
        )
    }
}

impl SystemServiceConnector {
    pub fn send_with(
        &self,
        record: &WorkspaceServiceRecord,
        request: ServiceRequest,
        cancellation: &CancellationToken,
        budget: Duration,
        io: &dyn ConnectorIo,
        clock: &dyn ConnectorClock,
    ) -> Result<ServiceResponse, String> {
        let deadline = Deadline::new(clock, budget);
        self.send_with_deadline(record, request, cancellation, &deadline, io, clock)
    }

    pub fn send_with_deadline(
        &self,
        record: &WorkspaceServiceRecord,
        request: ServiceRequest,
        cancellation: &CancellationToken,
        deadline: &dyn ConnectorRequestDeadline,
        io: &dyn ConnectorIo,
        clock: &dyn ConnectorClock,
    ) -> Result<ServiceResponse, String> {
        let operation_id = request.kind.operation_id().map(str::to_owned);
        cancellation_error(cancellation)?;
        let connect_cap = if request.kind.is_control() {
            SERVICE_CONTROL_CONNECT_TIMEOUT
        } else {
            SERVICE_CONNECT_TIMEOUT
        };
        let connect_timeout = remaining_or_timeout(deadline, clock)?
            .map_or(connect_cap, |remaining| remaining.min(connect_cap));
        let mut stream = match io.connect(record.port, connect_timeout) {
            Ok(stream) => stream,
            Err(error) => {
                cancellation_error(cancellation)?;
                remaining_or_timeout(deadline, clock)?;
                return Err(format!("failed to connect workspace service: {error}"));
            }
        };
        cancellation_error(cancellation)?;
        if let Err(error) = stream.set_read_timeout(Some(Duration::from_millis(100))) {
            cancellation_error(cancellation)?;
            remaining_or_timeout(deadline, clock)?;
            return Err(format!(
                "failed to set workspace service read timeout: {error}"
            ));
        }
        let payload = serde_json::to_string(&request).map_err(|err| err.to_string())?;
        if let Err(error) = write_with_deadline(
            stream.as_mut(),
            payload.as_bytes(),
            deadline,
            clock,
            cancellation,
        ) {
            self.cancel_after_error(&error, operation_id.as_deref(), record, io, clock);
            return Err(error);
        }
        if let Err(error) =
            write_with_deadline(stream.as_mut(), b"\n", deadline, clock, cancellation)
        {
            self.cancel_after_error(&error, operation_id.as_deref(), record, io, clock);
            return Err(error);
        }
        if let Err(error) = flush_with_deadline(stream.as_mut(), deadline, clock, cancellation) {
            self.cancel_after_error(&error, operation_id.as_deref(), record, io, clock);
            return Err(error);
        }
        let result = read_service_response(stream.as_mut(), deadline, clock, cancellation);
        if let Err(error) = &result {
            self.cancel_after_error(error, operation_id.as_deref(), record, io, clock);
        }
        result
    }

    pub fn send_control_with(
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

    pub fn cancel_after_error(
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

    pub fn send_cancel(
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

pub fn read_service_response(
    stream: &mut dyn ConnectorStream,
    deadline: &dyn ConnectorRequestDeadline,
    clock: &dyn ConnectorClock,
    cancellation: &CancellationToken,
) -> Result<ServiceResponse, String> {
    let mut response = Vec::new();
    let mut chunk = [0_u8; 8192];
    loop {
        cancellation_error(cancellation)?;
        let remaining = remaining_or_timeout(deadline, clock)?;
        if let Err(error) = stream.set_read_timeout(Some(
            remaining.map_or(Duration::from_millis(100), |remaining| {
                remaining.min(Duration::from_millis(100))
            }),
        )) {
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

pub fn response_line_too_large() -> String {
    format!(
        "invalid workspace service response: response line exceeds {SERVICE_RESPONSE_LINE_LIMIT} bytes"
    )
}

pub fn remaining_or_timeout(
    deadline: &dyn ConnectorRequestDeadline,
    clock: &dyn ConnectorClock,
) -> Result<Option<Duration>, String> {
    deadline.remaining_or_timeout(clock)
}

pub fn workspace_service_request_timeout_error(budget: Duration) -> String {
    let rendered_budget = if budget.subsec_nanos() == 0 {
        format!("{} seconds", budget.as_secs())
    } else {
        format!("{} milliseconds", duration_timeout_millis(budget))
    };
    format!("timeout: workspace service request exceeded {rendered_budget}")
}

pub fn remaining_or_control_timeout(
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

pub fn write_with_deadline(
    stream: &mut dyn ConnectorStream,
    bytes: &[u8],
    deadline: &dyn ConnectorRequestDeadline,
    clock: &dyn ConnectorClock,
    cancellation: &CancellationToken,
) -> Result<(), String> {
    let mut written = 0;
    while written < bytes.len() {
        cancellation_error(cancellation)?;
        let remaining = remaining_or_timeout(deadline, clock)?;
        if let Err(error) = stream.set_write_timeout(Some(
            remaining.map_or(Duration::from_millis(100), |remaining| {
                remaining.min(Duration::from_millis(100))
            }),
        )) {
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

pub fn flush_with_deadline(
    stream: &mut dyn ConnectorStream,
    deadline: &dyn ConnectorRequestDeadline,
    clock: &dyn ConnectorClock,
    cancellation: &CancellationToken,
) -> Result<(), String> {
    loop {
        cancellation_error(cancellation)?;
        let remaining = remaining_or_timeout(deadline, clock)?;
        if let Err(error) = stream.set_write_timeout(Some(
            remaining.map_or(Duration::from_millis(100), |remaining| {
                remaining.min(Duration::from_millis(100))
            }),
        )) {
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

pub fn write_control_with_deadline(
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

pub fn workspace_service_command(
    exe: &Path,
    identity: &WorkspaceServiceIdentity,
    config: WorkspaceServiceConfig,
    token: &str,
    build: &str,
    environment: &dyn ServiceEnvironment,
) -> Result<Command, String> {
    let mut command = Command::new(exe);
    command
        .current_dir(persistent_provider_working_directory(
            Path::new(&identity.workspace_root),
            Path::new(&identity.source_root),
        )?)
        .arg("--workspace-service")
        .arg("--core-identity")
        .arg(build)
        .arg("--workspace-root")
        .arg(&identity.workspace_root)
        .arg("--source-root")
        .arg(&identity.source_root)
        .arg("--service-dir")
        .arg(identity.service_dir.display().to_string())
        .arg("--token")
        .arg(token)
        .arg("--idle-secs")
        .arg(config.idle_secs.to_string());
    if let Some(plugin_root) = environment.find_plugin_root(Path::new(&identity.workspace_root)) {
        command.env("UNICA_PLUGIN_ROOT", plugin_root);
    }
    Ok(command)
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
        let stdout = ProcessLog::create(&identity.service_dir.join("service.stdout.log"))
            .map_err(|err| format!("failed to create workspace service stdout log: {err}"))?;
        deadline.remaining(cancellation)?;
        let stderr_path = identity.service_dir.join("service.stderr.log");
        let stderr = ProcessLog::create(&stderr_path)
            .map_err(|err| format!("failed to create workspace service stderr log: {err}"))?;
        deadline.remaining(cancellation)?;
        let exe = env::current_exe()
            .map_err(|err| format!("failed to locate current unica executable: {err}"))?;
        // Unit tests run inside libtest, which cannot serve --workspace-service.
        // Exercise the real built Unica service while retaining the test-only
        // downloader in the invoking process.
        #[cfg(any(test, feature = "test-support"))]
        let exe = env::var_os("UNICA_TEST_WORKSPACE_SERVICE_EXE")
            .map(PathBuf::from)
            .unwrap_or(exe);
        // A service of another build would answer this daemon under its
        // identity; the file it starts from must still be this build. The
        // service records the identity it is given instead of recomputing it:
        // a test service binary is not the test process's build.
        let build = self.environment.core_identity();
        self.environment.verify_executable_build(&exe)?;
        let mut command = workspace_service_command(
            &exe,
            identity,
            config,
            token,
            build,
            self.environment.as_ref(),
        )?;
        stdout.attach_stdout(&mut command);
        stderr.attach_stderr(&mut command);
        let mut child = ManagedStartupChild::spawn_configured(command)
            .map_err(|err| format!("failed to spawn workspace service: {err}"))?;
        let child_pid = child.id();
        let readiness: Result<WorkspaceServiceRecord, String> = (|| {
            let wait_budget = deadline
                .remaining(cancellation)?
                .min(SERVICE_CONNECT_TIMEOUT);
            // Сервис, завершившийся до готовности, записи уже не напишет:
            // ожидание прекращается сразу, а не досиживает весь срок.
            let record = wait_for_record_while(
                identity,
                &SYSTEM_SERVICE_CONNECTOR,
                build,
                child_pid,
                token,
                wait_budget,
                cancellation,
                // Ошибка опроса — тоже конец ожидания, как у демона v5: процессом
                // запуск уже не владеет, а причину назовёт отчёт о старте.
                &mut || !matches!(child.try_wait_status(), Ok(None)),
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

pub fn workspace_service_startup_failure(
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

pub fn read_workspace_service_stderr_tail(path: &Path) -> Result<String, String> {
    let mut file = open_state_reader(path).map_err(|error| {
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

pub fn terminate_failed_workspace_service_spawn(
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

pub fn remove_spawned_service_record_if_owned(
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
pub struct AnalyzerLane {
    state: Mutex<AnalyzerLaneState>,
    wake: Condvar,
}

#[derive(Default)]
pub struct AnalyzerLaneState {
    held: bool,
    next_ticket: u64,
    waiters: VecDeque<u64>,
}

impl AnalyzerLane {
    #[cfg(any(test, feature = "test-support"))]
    pub fn test_waiters_is_empty(&self) -> bool {
        self.state.lock().unwrap().waiters.is_empty()
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn test_poison_state(&self) {
        let _state = self.state.lock().unwrap();
        panic!("intentional analyzer queue poison");
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn acquire(
        &self,
        cancellation: &CancellationToken,
    ) -> Result<AnalyzerLanePermit<'_>, String> {
        self.acquire_with_hook_before(cancellation, None, || {})
    }

    pub fn acquire_before(
        &self,
        cancellation: &CancellationToken,
        deadline: &ElapsedBudgetDeadline,
    ) -> Result<AnalyzerLanePermit<'_>, String> {
        self.acquire_with_hook_before(cancellation, Some(deadline), || {})
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn acquire_with_hook(
        &self,
        cancellation: &CancellationToken,
        queued: impl FnOnce(),
    ) -> Result<AnalyzerLanePermit<'_>, String> {
        self.acquire_with_hook_before(cancellation, None, queued)
    }

    pub fn acquire_with_hook_before(
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

impl AnalyzerLane {
    /// The lane, only when no call holds it or waits for it: a background
    /// call never makes a foreground one wait in line behind it.
    pub fn try_acquire_idle(&self) -> Option<AnalyzerLanePermit<'_>> {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if state.held || !state.waiters.is_empty() {
            return None;
        }
        state.held = true;
        Some(AnalyzerLanePermit { lane: self })
    }
}

pub struct AnalyzerLanePermit<'a> {
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

pub struct WorkspaceServicesState<B: BslBackend, R: RlmBackend> {
    identity: WorkspaceServiceIdentity,
    token: String,
    record_owner: WorkspaceServiceRecord,
    analyzer_lane: AnalyzerLane,
    analyzer: Mutex<Option<PersistentMcpSession>>,
    analyzer_starter: Arc<BslSessionStarter>,
    bsl_backend: Arc<B>,
    rlm_backend: Arc<R>,
    rlm_lane: AnalyzerLane,
    rlm: Mutex<Option<<R as RlmBackend>::Session>>,
    rlm_starter: Arc<RlmSessionStarter<R>>,
    /// Index maintenance outlives the read that noticed the index was stale, so
    /// it cannot borrow that read's token: the operation is unregistered as soon
    /// as it answers, which would both hide the thread from `begin_shutdown` and
    /// let a late `cancel` for the finished read kill the scheduled update.
    rlm_maintenance_cancellation: CancellationToken,
    rlm_maintenance_tasks: Arc<SessionTeardownLifecycle>,
    session_teardowns: SessionTeardownLifecycle,
    analyzer_invalidated: AtomicBool,
    rlm_invalidated: AtomicBool,
    operations: Mutex<HashMap<String, CancellationToken>>,
    shutting_down: AtomicBool,
    #[cfg(any(test, feature = "test-support"))]
    handler_started_hook: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
    #[cfg(any(test, feature = "test-support"))]
    authenticated_request_hook: Mutex<Option<Arc<AuthenticatedWorkspaceRequestHook>>>,
    #[cfg(any(test, feature = "test-support"))]
    invalidation_response_hook: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
    #[cfg(any(test, feature = "test-support"))]
    provider_sessions_drained_hook: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
    #[cfg(any(test, feature = "test-support"))]
    caller_detached_hook: Mutex<Option<Arc<DetachedWorkspaceCallerHook>>>,
    #[cfg(any(test, feature = "test-support"))]
    caller_monitor_fault: Mutex<Option<WorkspaceCallerMonitorPoint>>,
    #[cfg(any(test, feature = "test-support"))]
    handler_finished_hook: Mutex<Option<Arc<WorkspaceHandlerFinishedHook>>>,
}

pub struct WorkspaceServicesProjection<'a, B: BslBackend, R: RlmBackend>(
    &'a WorkspaceServicesState<B, R>,
);

#[cfg(any(test, feature = "test-support"))]
pub type AuthenticatedWorkspaceRequestHook = dyn Fn(u16, &ServiceRequestKind) + Send + Sync;

#[cfg(any(test, feature = "test-support"))]
pub type DetachedWorkspaceCallerHook = dyn Fn(&CancellationToken) + Send + Sync;

#[cfg(any(test, feature = "test-support"))]
pub type WorkspaceHandlerFinishedHook = dyn Fn(&thread::Result<Result<(), String>>) + Send + Sync;

#[cfg(any(test, feature = "test-support"))]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceCallerMonitorPoint {
    Setup,
    Peek,
}

#[cfg(any(test, feature = "test-support"))]
pub struct WorkspaceServicesProjectionMut<'a, B: BslBackend, R: RlmBackend>(
    &'a mut WorkspaceServicesState<B, R>,
);

impl<B: BslBackend, R: RlmBackend> WorkspaceActorRuntimeProjection
    for WorkspaceServicesState<B, R>
{
    type Projection<'a> = WorkspaceServicesProjection<'a, B, R>;

    fn project_for_actor(&self) -> Self::Projection<'_> {
        WorkspaceServicesProjection(self)
    }
}

#[cfg(any(test, feature = "test-support"))]
impl<B: BslBackend, R: RlmBackend> WorkspaceActorRuntimeTestProjection
    for WorkspaceServicesState<B, R>
{
    type ProjectionMut<'a> = WorkspaceServicesProjectionMut<'a, B, R>;

    fn project_mut_for_actor_test(&mut self) -> Self::ProjectionMut<'_> {
        WorkspaceServicesProjectionMut(self)
    }
}

pub struct WorkspaceServices<B: BslBackend, R: RlmBackend> {
    actor: WorkspaceActor<WorkspaceServicesState<B, R>>,
}

pub type BslSessionStarter = dyn Fn(&WorkspaceContext, &Path, &CancellationToken) -> Result<PersistentMcpSession, String>
    + Send
    + Sync;
pub type RlmSessionStarter<R> = dyn Fn(
        &WorkspaceContext,
        &Path,
        &Path,
        &str,
        &CancellationToken,
    ) -> Result<<R as RlmBackend>::Session, String>
    + Send
    + Sync;
impl<B: BslBackend, R: RlmBackend> WorkspaceServices<B, R> {
    pub fn new(
        identity: WorkspaceServiceIdentity,
        record_owner: &WorkspaceServiceRecord,
        bsl_backend: Arc<B>,
        rlm_backend: Arc<R>,
        environment: Arc<dyn ServiceEnvironment>,
    ) -> Self {
        let context = WorkspaceContext {
            cwd: PathBuf::from(&identity.workspace_root),
            workspace_root: PathBuf::from(&identity.workspace_root),
            cache_root: service_cache_root(&identity.service_dir),
            workspace_epoch: 0,
        };
        let rlm_maintenance_tasks = Arc::new(SessionTeardownLifecycle::default());
        let actor_identity = WorkspaceIdentity::new(
            &context,
            [WorkspaceSourceSetInput::new(
                LEGACY_WORKSPACE_SERVICE_SOURCE_SET,
                &identity.source_root,
                SourceSetKind::Configuration,
                SourceFormat::Unknown,
                SourceProfile::legacy_workspace_service_compatibility(),
            )],
            LEGACY_WORKSPACE_SERVICE_PROVIDER_PROFILE,
        )
        .expect("legacy workspace service identity is already canonical and contained");
        let runtime = WorkspaceServicesState {
            identity,
            token: record_owner.token.clone(),
            record_owner: record_owner.clone(),
            analyzer_lane: AnalyzerLane::default(),
            analyzer: Mutex::new(None),
            analyzer_starter: Arc::new({
                let backend = Arc::clone(&bsl_backend);
                move |context, root, cancel| backend.start(context, root, cancel)
            }),
            bsl_backend,
            rlm_backend: Arc::clone(&rlm_backend),
            rlm_lane: AnalyzerLane::default(),
            rlm: Mutex::new(None),
            rlm_starter: Arc::new(move |context, root, directory, id, cancel| {
                rlm_backend.start(context, root, directory, id, cancel)
            }),
            rlm_maintenance_cancellation: CancellationToken::new(),
            rlm_maintenance_tasks,
            session_teardowns: SessionTeardownLifecycle::default(),
            analyzer_invalidated: AtomicBool::new(false),
            rlm_invalidated: AtomicBool::new(false),
            operations: Mutex::new(HashMap::new()),
            shutting_down: AtomicBool::new(false),
            #[cfg(any(test, feature = "test-support"))]
            handler_started_hook: Mutex::new(None),
            #[cfg(any(test, feature = "test-support"))]
            authenticated_request_hook: Mutex::new(None),
            #[cfg(any(test, feature = "test-support"))]
            invalidation_response_hook: Mutex::new(None),
            #[cfg(any(test, feature = "test-support"))]
            provider_sessions_drained_hook: Mutex::new(None),
            #[cfg(any(test, feature = "test-support"))]
            caller_detached_hook: Mutex::new(None),
            #[cfg(any(test, feature = "test-support"))]
            caller_monitor_fault: Mutex::new(None),
            #[cfg(any(test, feature = "test-support"))]
            handler_finished_hook: Mutex::new(None),
        };
        Self {
            actor: WorkspaceActor::with_legacy_runtime(actor_identity, context, runtime)
                .expect("legacy workspace service actor context matches its canonical identity")
                .with_policies(environment.workspace_policies()),
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    fn workspace_identity(&self) -> &WorkspaceIdentity {
        self.actor.workspace_identity()
    }

    fn identity(&self) -> &WorkspaceServiceIdentity {
        &self.actor.runtime_projection().0.identity
    }

    fn token(&self) -> &str {
        &self.actor.runtime_projection().0.token
    }

    fn record_owner(&self) -> &WorkspaceServiceRecord {
        &self.actor.runtime_projection().0.record_owner
    }

    fn analyzer_lane(&self) -> &AnalyzerLane {
        &self.actor.runtime_projection().0.analyzer_lane
    }

    fn analyzer(&self) -> &Mutex<Option<PersistentMcpSession>> {
        &self.actor.runtime_projection().0.analyzer
    }

    fn analyzer_starter(&self) -> &Arc<BslSessionStarter> {
        &self.actor.runtime_projection().0.analyzer_starter
    }

    fn rlm_lane(&self) -> &AnalyzerLane {
        &self.actor.runtime_projection().0.rlm_lane
    }

    fn rlm(&self) -> &Mutex<Option<<R as RlmBackend>::Session>> {
        &self.actor.runtime_projection().0.rlm
    }

    fn rlm_starter(&self) -> &Arc<RlmSessionStarter<R>> {
        &self.actor.runtime_projection().0.rlm_starter
    }

    fn rlm_maintenance_cancellation(&self) -> &CancellationToken {
        &self
            .actor
            .runtime_projection()
            .0
            .rlm_maintenance_cancellation
    }

    fn rlm_maintenance_tasks(&self) -> &Arc<SessionTeardownLifecycle> {
        &self.actor.runtime_projection().0.rlm_maintenance_tasks
    }

    fn session_teardowns(&self) -> &SessionTeardownLifecycle {
        &self.actor.runtime_projection().0.session_teardowns
    }

    fn analyzer_invalidated(&self) -> &AtomicBool {
        &self.actor.runtime_projection().0.analyzer_invalidated
    }

    fn rlm_invalidated(&self) -> &AtomicBool {
        &self.actor.runtime_projection().0.rlm_invalidated
    }

    fn operations(&self) -> &Mutex<HashMap<String, CancellationToken>> {
        &self.actor.runtime_projection().0.operations
    }

    fn shutting_down(&self) -> &AtomicBool {
        &self.actor.runtime_projection().0.shutting_down
    }

    #[cfg(any(test, feature = "test-support"))]
    fn runtime_mut_for_test(&mut self) -> &mut WorkspaceServicesState<B, R> {
        self.actor.runtime_projection_mut_for_test().0
    }

    #[cfg(any(test, feature = "test-support"))]
    fn handler_started_hook(&self) -> &Mutex<Option<Arc<dyn Fn() + Send + Sync>>> {
        &self.actor.runtime_projection().0.handler_started_hook
    }

    #[cfg(any(test, feature = "test-support"))]
    fn provider_sessions_drained_hook(&self) -> &Mutex<Option<Arc<dyn Fn() + Send + Sync>>> {
        &self
            .actor
            .runtime_projection()
            .0
            .provider_sessions_drained_hook
    }

    fn context(&self) -> &WorkspaceContext {
        self.actor.context()
    }

    fn bind_provider_root(
        &self,
        source_set_name: &str,
        source_root: &Path,
    ) -> Result<ProviderRootBinding, String> {
        self.actor.bind_provider_root(source_set_name, source_root)
    }

    fn validate_provider_binding(&self, binding: &ProviderRootBinding) -> Result<(), String> {
        self.actor.validate_binding(binding)
    }

    #[cfg(any(test, feature = "test-support"))]
    fn notify_handler_started(&self) {
        let hook = self.handler_started_hook().lock().unwrap().clone();
        if let Some(hook) = hook {
            hook();
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    fn notify_authenticated_request(&self, stream: &TcpStream, kind: &ServiceRequestKind) {
        let hook = self
            .actor
            .runtime_projection()
            .0
            .authenticated_request_hook
            .lock()
            .unwrap()
            .clone();
        if let (Some(hook), Ok(peer)) = (hook, stream.peer_addr()) {
            hook(peer.port(), kind);
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    fn notify_caller_detached(&self, cancellation: &CancellationToken) {
        let hook = self
            .actor
            .runtime_projection()
            .0
            .caller_detached_hook
            .lock()
            .unwrap()
            .clone();
        if let Some(hook) = hook {
            hook(cancellation);
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    fn take_caller_monitor_fault(&self, point: WorkspaceCallerMonitorPoint) -> bool {
        let mut fault = self
            .actor
            .runtime_projection()
            .0
            .caller_monitor_fault
            .lock()
            .unwrap();
        if *fault == Some(point) {
            *fault = None;
            true
        } else {
            false
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    fn notify_handler_finished(&self, result: &thread::Result<Result<(), String>>) {
        let hook = self
            .actor
            .runtime_projection()
            .0
            .handler_finished_hook
            .lock()
            .unwrap()
            .clone();
        if let Some(hook) = hook {
            hook(result);
        }
    }

    fn ping(&self) -> ServiceResponse {
        ServiceResponse {
            ok: true,
            status: Some(if self.shutting_down().load(Ordering::Acquire) {
                "shutting-down".to_string()
            } else {
                "alive".to_string()
            }),
            ..ServiceResponse::default()
        }
    }

    fn authenticate(&self, token: &str) -> Result<(), &'static str> {
        if token == self.token() {
            Ok(())
        } else {
            Err("invalid workspace service token")
        }
    }

    fn owns_record(&self, record: &WorkspaceServiceRecord) -> bool {
        record.schema_version == self.record_owner().schema_version
            && record.pid == self.record_owner().pid
            && record.port == self.record_owner().port
            && record.token == self.record_owner().token
            && record.build == self.record_owner().build
            && record.workspace_root == self.record_owner().workspace_root
            && record.source_root == self.record_owner().source_root
            && record.started_at == self.record_owner().started_at
            && record.matches(self.identity(), &self.record_owner().build)
    }

    fn register_operation(
        self: &Arc<Self>,
        operation_id: String,
    ) -> Result<(CancellationToken, OperationGuard<B, R>), String> {
        if self.shutting_down().load(Ordering::Acquire) {
            return Err("workspace service is shutting down".to_string());
        }
        let mut operations = self
            .operations()
            .lock()
            .map_err(|_| "workspace service operation registry is unavailable".to_string())?;
        if self.shutting_down().load(Ordering::Acquire) {
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
            .operations()
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
        self.shutting_down().store(true, Ordering::Release);
        self.session_teardowns().close();
        self.rlm_maintenance_tasks().close();
        self.rlm_maintenance_cancellation().cancel();
        let cancellations = self
            .operations()
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
        self.session_teardowns()
            .track(thread::spawn(move || drop(session)));
    }

    fn shutdown_provider_sessions(&self) -> bool {
        let analyzer = self
            .analyzer()
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take();
        let rlm = self
            .rlm()
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take();
        if let Some(session) = analyzer {
            self.retire_session(session);
        }
        if let Some(session) = rlm {
            self.retire_session(session);
        }
        let sessions_drained = self.session_teardowns().drain(SESSION_TEARDOWN_GRACE);
        let maintenance_drained = self.rlm_maintenance_tasks().drain(SESSION_TEARDOWN_GRACE);
        #[cfg(any(test, feature = "test-support"))]
        if let Some(hook) = self
            .provider_sessions_drained_hook()
            .lock()
            .unwrap()
            .clone()
        {
            hook();
        }
        sessions_drained && maintenance_drained
    }

    fn invalidate(&self, events: &[DomainEvent]) -> ServiceResponse {
        if events.iter().any(|event| invalidates_analyzer(event.kind)) {
            self.analyzer_invalidated().store(true, Ordering::Release);
            self.rlm_invalidated().store(true, Ordering::Release);
        }
        ServiceResponse {
            ok: true,
            status: Some("invalidated".to_string()),
            ..ServiceResponse::default()
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    fn acquire_analyzer_lane(
        &self,
        cancellation: &CancellationToken,
    ) -> Result<AnalyzerLanePermit<'_>, String> {
        self.analyzer_lane().acquire(cancellation)
    }

    #[cfg(any(test, feature = "test-support"))]
    fn acquire_rlm_lane(
        &self,
        cancellation: &CancellationToken,
    ) -> Result<AnalyzerLanePermit<'_>, String> {
        self.rlm_lane().acquire(cancellation)
    }

    fn acquire_analyzer_lane_before(
        &self,
        cancellation: &CancellationToken,
        deadline: &ElapsedBudgetDeadline,
    ) -> Result<AnalyzerLanePermit<'_>, String> {
        self.analyzer_lane().acquire_before(cancellation, deadline)
    }

    fn acquire_rlm_lane_before(
        &self,
        cancellation: &CancellationToken,
        deadline: &ElapsedBudgetDeadline,
    ) -> Result<AnalyzerLanePermit<'_>, String> {
        self.rlm_lane().acquire_before(cancellation, deadline)
    }

    fn handle_bsl_mcp(
        &self,
        tool_name: &str,
        tool_args: Value,
        timeout_seconds: u64,
        timeout_nanos: u32,
        background: bool,
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
        let lane = if background {
            self.analyzer_lane()
                .try_acquire_idle()
                .ok_or_else(|| ANALYZER_BUSY.to_string())
        } else {
            self.acquire_analyzer_lane_before(cancellation, &deadline)
        };
        let _lane = match lane {
            Ok(lane) => lane,
            Err(error) => return ServiceResponse::error(error),
        };
        if let Err(error) = deadline.remaining_cancellable(cancellation) {
            return ServiceResponse::error(error);
        }
        let binding = match self.provider_binding() {
            Ok(binding) => binding,
            Err(error) => return ServiceResponse::error(error),
        };
        let mut analyzer = self
            .analyzer()
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let mut stale_session = None;
        let invalidated = self.analyzer_invalidated().swap(false, Ordering::AcqRel);
        if invalidated {
            stale_session = analyzer.take();
        }
        if stale_session.is_none()
            && analyzer
                .as_mut()
                .is_some_and(|session| !session.is_reusable())
        {
            stale_session = analyzer.take();
        }
        if let Some(mut stale_session) = stale_session {
            drop(analyzer);
            if invalidated {
                let retirement = stale_session
                    .retire_for_replacement(|| deadline.remaining_cancellable(cancellation));
                analyzer = self
                    .analyzer()
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                if let Err(error) = retirement {
                    *analyzer = Some(stale_session);
                    self.analyzer_invalidated().store(true, Ordering::Release);
                    return ServiceResponse::error(error);
                }
            } else {
                self.retire_session(stale_session);
                analyzer = self
                    .analyzer()
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
            }
        }
        let result = self.actor.read(&binding, |source_root| {
            deadline.remaining_cancellable(cancellation)?;
            if analyzer.is_none() {
                *analyzer = Some((self.analyzer_starter())(
                    self.context(),
                    source_root,
                    cancellation,
                )?);
            }
            let remaining = deadline.remaining_cancellable(cancellation)?;
            analyzer
                .as_mut()
                .expect("bsl session must exist after start")
                .call_with_failure_capture(tool_name, tool_args, remaining, cancellation)
        });
        match result {
            Ok(output) => ServiceResponse {
                ok: true,
                result_text: Some(output.result_text),
                stderr: Some(output.stderr),
                ..ServiceResponse::default()
            },
            Err(error) => {
                let failure_snapshot = analyzer
                    .as_mut()
                    .and_then(|session| session.take_failure_snapshot());
                let stale_session = analyzer.take();
                drop(analyzer);
                if let Some(stale_session) = stale_session {
                    self.retire_session(stale_session);
                }
                if let Some(snapshot) = failure_snapshot {
                    self.actor
                        .runtime_projection()
                        .0
                        .bsl_backend
                        .report_failure(snapshot);
                }
                ServiceResponse::error(error)
            }
        }
    }

    fn provider_binding(&self) -> Result<ProviderRootBinding, String> {
        self.bind_provider_root(
            LEGACY_WORKSPACE_SERVICE_SOURCE_SET,
            Path::new(&self.identity().source_root),
        )
    }
}

pub struct SessionTeardownLifecycle {
    state: Mutex<TrackedTaskState>,
}

pub struct TrackedTaskState {
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
    pub fn track(&self, handle: thread::JoinHandle<()>) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        Self::reap_finished(&mut state.handles);
        if state.accepting {
            state.handles.push(handle);
            return;
        }
        drop(state);
        let _ = handle.join();
    }

    pub fn close(&self) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.accepting = false;
    }

    pub fn drain(&self, timeout: Duration) -> bool {
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

    pub fn reap_finished(handles: &mut Vec<thread::JoinHandle<()>>) {
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

impl BackgroundTaskTracker for SessionTeardownLifecycle {
    fn track(&self, handle: thread::JoinHandle<()>) {
        SessionTeardownLifecycle::track(self, handle);
    }
}

pub fn invalidates_analyzer(event: DomainEventKind) -> bool {
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

pub fn remove_service_record_if_owned<B: BslBackend, R: RlmBackend>(
    runtime: &WorkspaceServices<B, R>,
) {
    remove_service_record_if_owned_with_hook(runtime, || {});
}

pub fn remove_service_record_if_owned_with_hook<B: BslBackend, R: RlmBackend>(
    runtime: &WorkspaceServices<B, R>,
    inside_critical_section: impl FnOnce(),
) {
    let _ = with_record_lock(runtime.identity(), || {
        let Some(record) = read_record_unlocked(runtime.identity()) else {
            return Ok(());
        };
        if !runtime.owns_record(&record) {
            return Ok(());
        }
        inside_critical_section();
        let Some(confirmed) = read_record_unlocked(runtime.identity()) else {
            return Ok(());
        };
        if record == confirmed && runtime.owns_record(&confirmed) {
            fs::remove_file(runtime.identity().record_path()).map_err(|error| {
                format!("failed to remove owned workspace service record: {error}")
            })?;
        }
        Ok(())
    });
}

pub fn update_service_record_last_access<B: BslBackend, R: RlmBackend>(
    runtime: &WorkspaceServices<B, R>,
    last_access_at: u64,
) {
    update_service_record_last_access_with_hook(runtime, last_access_at, || {});
}

pub fn update_service_record_last_access_with_hook<B: BslBackend, R: RlmBackend>(
    runtime: &WorkspaceServices<B, R>,
    last_access_at: u64,
    inside_critical_section: impl FnOnce(),
) {
    let _ = with_record_lock(runtime.identity(), || {
        let Some(mut record) = read_record_unlocked(runtime.identity()) else {
            return Ok(());
        };
        if !runtime.owns_record(&record) {
            return Ok(());
        }
        inside_critical_section();
        record.last_access_at = last_access_at;
        write_record_unlocked(runtime.identity(), &record)
    });
}

pub struct OperationGuard<B: BslBackend, R: RlmBackend> {
    runtime: Arc<WorkspaceServices<B, R>>,
    operation_id: String,
}

impl<B: BslBackend, R: RlmBackend> Drop for OperationGuard<B, R> {
    fn drop(&mut self) {
        if let Ok(mut operations) = self.runtime.operations().lock() {
            operations.remove(&self.operation_id);
        }
    }
}

pub trait WorkspaceServiceOperationExecutor<B: BslBackend, R: RlmBackend>: Send + Sync {
    fn execute(
        &self,
        runtime: &WorkspaceServices<B, R>,
        kind: ServiceRequestKind,
        cancellation: &CancellationToken,
    ) -> ServiceResponse;
}

pub struct SystemWorkspaceServiceOperationExecutor;

impl<B: BslBackend, R: RlmBackend> WorkspaceServiceOperationExecutor<B, R>
    for SystemWorkspaceServiceOperationExecutor
{
    fn execute(
        &self,
        runtime: &WorkspaceServices<B, R>,
        kind: ServiceRequestKind,
        cancellation: &CancellationToken,
    ) -> ServiceResponse {
        match kind {
            ServiceRequestKind::BslMcp {
                tool_name,
                tool_args,
                timeout_seconds,
                timeout_nanos,
                background,
                ..
            } => runtime.handle_bsl_mcp(
                &tool_name,
                tool_args,
                timeout_seconds,
                timeout_nanos,
                background,
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

pub fn persistent_provider_working_directory(
    workspace_root: &Path,
    source_root: &Path,
) -> Result<PathBuf, String> {
    let runtime = short_private_runtime_dir()
        .map_err(|error| format!("failed to prepare persistent provider directory: {error}"))?;
    let temp = env::temp_dir();
    let executable_directory = env::current_exe()
        .ok()
        .and_then(|executable| executable.parent().map(Path::to_path_buf));
    let temp_anchor = normalize_path_identity(&temp)
        .ok()
        .and_then(|physical| physical.ancestors().last().map(Path::to_path_buf));
    let mut last_error = None;
    for candidate in [runtime, Some(temp), executable_directory, temp_anchor]
        .into_iter()
        .flatten()
    {
        match checked_persistent_provider_working_directory(&candidate, workspace_root, source_root)
        {
            Ok(directory) => return Ok(directory),
            Err(error) => last_error = Some(error),
        }
    }
    Err(last_error.unwrap_or_else(|| {
        "persistent provider working directory must be outside the workspace and source tree"
            .to_string()
    }))
}

pub fn checked_persistent_provider_working_directory(
    candidate: &Path,
    workspace_root: &Path,
    source_root: &Path,
) -> Result<PathBuf, String> {
    let directory = normalize_path_identity(candidate)?;
    let metadata = fs::metadata(&directory)
        .map_err(|error| format!("failed to inspect persistent provider directory: {error}"))?;
    if !metadata.is_dir() {
        return Err("persistent provider working directory must be a directory".to_string());
    }
    for root in [workspace_root, source_root] {
        let root = normalize_path_identity(root)?;
        if unica_workspace::paths::path_starts_with_host_root(&directory, &root) {
            return Err(
                "persistent provider working directory must be outside the workspace and source tree"
                    .to_string(),
            );
        }
    }
    Ok(directory)
}

pub fn env_u64(name: &str, default: u64) -> u64 {
    env::var(name)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(default)
}

pub struct ServiceRecordLock {
    file: StateHandle,
}

impl Drop for ServiceRecordLock {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

pub fn acquire_record_lock(
    identity: &WorkspaceServiceIdentity,
) -> Result<ServiceRecordLock, String> {
    fs::create_dir_all(&identity.service_dir)
        .map_err(|error| format!("failed to create workspace service state directory: {error}"))?;
    let path = identity.service_dir.join(SERVICE_RECORD_LOCK_FILE);
    let file = open_state_lock(&path, true).map_err(|error| {
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

pub fn with_record_lock<T>(
    identity: &WorkspaceServiceIdentity,
    operation: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    let _lock = acquire_record_lock(identity)?;
    operation()
}

pub fn read_record_unlocked(identity: &WorkspaceServiceIdentity) -> Option<WorkspaceServiceRecord> {
    let text = fs::read_to_string(identity.record_path()).ok()?;
    serde_json::from_str(&text).ok()
}

pub fn read_record(identity: &WorkspaceServiceIdentity) -> Option<WorkspaceServiceRecord> {
    with_record_lock(identity, || Ok(read_record_unlocked(identity)))
        .ok()
        .flatten()
}

pub fn write_record_unlocked(
    identity: &WorkspaceServiceIdentity,
    record: &WorkspaceServiceRecord,
) -> Result<(), String> {
    let text = serde_json::to_string_pretty(record).map_err(|err| err.to_string())?;
    fs::write(identity.record_path(), text + "\n")
        .map_err(|err| format!("failed to write workspace service record: {err}"))
}

pub fn write_record(
    identity: &WorkspaceServiceIdentity,
    record: &WorkspaceServiceRecord,
) -> Result<(), String> {
    write_record_with_hook(identity, record, || {})
}

pub fn write_record_with_hook(
    identity: &WorkspaceServiceIdentity,
    record: &WorkspaceServiceRecord,
    inside_critical_section: impl FnOnce(),
) -> Result<(), String> {
    with_record_lock(identity, || {
        inside_critical_section();
        write_record_unlocked(identity, record)
    })
}

pub struct SpawnLock {
    file: StateHandle,
}

impl Drop for SpawnLock {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

pub fn spawn_lock_path(identity: &WorkspaceServiceIdentity) -> PathBuf {
    identity.service_dir.join("service.lock")
}

pub fn acquire_spawn_lock(
    identity: &WorkspaceServiceIdentity,
) -> Result<Option<SpawnLock>, String> {
    fs::create_dir_all(&identity.service_dir)
        .map_err(|err| format!("failed to create workspace service lock directory: {err}"))?;
    let path = spawn_lock_path(identity);
    let mut file = open_state_lock(&path, true).map_err(|error| {
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

pub fn spawn_lock_is_contended(error: &io::Error) -> bool {
    let expected = fs2::lock_contended_error();
    error.kind() == ErrorKind::WouldBlock
        || error
            .raw_os_error()
            .zip(expected.raw_os_error())
            .is_some_and(|(actual, expected)| actual == expected)
}

#[cfg(any(test, feature = "test-support"))]
pub fn wait_for_record_with_connector(
    identity: &WorkspaceServiceIdentity,
    connector: &dyn ServiceConnector,
    build: &str,
    expected_pid: u32,
    expected_token: &str,
    timeout: Duration,
    cancellation: &CancellationToken,
) -> Result<WorkspaceServiceRecord, String> {
    wait_for_record_while(
        identity,
        connector,
        build,
        expected_pid,
        expected_token,
        timeout,
        cancellation,
        &mut || false,
    )
}

/// Как `wait_for_record_with_connector`, но `exited` сообщает, что
/// ожидаемый процесс уже завершился: тогда ждать записи бессмысленно.
#[allow(clippy::too_many_arguments)]
pub fn wait_for_record_while(
    identity: &WorkspaceServiceIdentity,
    connector: &dyn ServiceConnector,
    build: &str,
    expected_pid: u32,
    expected_token: &str,
    timeout: Duration,
    cancellation: &CancellationToken,
    exited: &mut dyn FnMut() -> bool,
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
                && record.matches(identity, build)
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
        if exited() {
            break;
        }
        thread::sleep(Duration::from_millis(50).min(remaining));
    }
    cancellation_error(cancellation)?;
    Err(format!(
        "workspace service did not become ready at {}",
        identity.record_path().display()
    ))
}

pub fn new_token(identity: &WorkspaceServiceIdentity) -> String {
    let mut hasher = DefaultHasher::new();
    identity.key.hash(&mut hasher);
    std::process::id().hash(&mut hasher);
    now_secs().hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}

pub fn service_cache_root(service_dir: &Path) -> PathBuf {
    service_dir
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .unwrap_or_else(|| service_dir.to_path_buf())
}

pub fn run_workspace_service_from_args<B: BslBackend, R: RlmBackend>(
    args: &[String],
    bsl: Arc<B>,
    rlm: Arc<R>,
    environment: Arc<dyn ServiceEnvironment>,
) -> Result<(), String> {
    let workspace_root = required_arg(args, "--workspace-root")?;
    let source_root = required_arg(args, "--source-root")?;
    let service_dir = PathBuf::from(required_arg(args, "--service-dir")?);
    let token = required_arg(args, "--token")?;
    let build = required_arg(args, "--core-identity")?;
    let idle_secs = optional_u64_arg(args, "--idle-secs", DEFAULT_IDLE_SECS);
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
    run_workspace_service(identity, token, build, idle_secs, bsl, rlm, environment)
}

pub fn run_workspace_service<B: BslBackend, R: RlmBackend>(
    identity: WorkspaceServiceIdentity,
    token: String,
    build: String,
    idle_secs: u64,
    bsl: Arc<B>,
    rlm: Arc<R>,
    environment: Arc<dyn ServiceEnvironment>,
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
        build,
        version: env!("CARGO_PKG_VERSION").to_string(),
        workspace_root: identity.workspace_root.clone(),
        source_root: identity.source_root.clone(),
        started_at: now_secs(),
        last_access_at: now_secs(),
    };
    write_record(&identity, &record)?;

    let runtime = Arc::new(WorkspaceServices::new(
        identity,
        &record,
        bsl,
        rlm,
        environment,
    ));
    serve_workspace_service(
        listener,
        runtime,
        Arc::new(SystemWorkspaceServiceOperationExecutor),
        Duration::from_secs(idle_secs.max(1)),
        SERVICE_REQUEST_HEADER_TIMEOUT,
        SERVICE_SHUTDOWN_GRACE,
    )
}

pub fn serve_workspace_service<B: BslBackend, R: RlmBackend>(
    listener: TcpListener,
    runtime: Arc<WorkspaceServices<B, R>>,
    executor: Arc<dyn WorkspaceServiceOperationExecutor<B, R>>,
    idle_timeout: Duration,
    request_header_timeout: Duration,
    shutdown_grace: Duration,
) -> Result<(), String> {
    let mut last_access = Instant::now();
    let mut handlers: Vec<thread::JoinHandle<Result<(), String>>> = Vec::new();
    let mut result = Ok(());
    loop {
        let mut index = 0;
        while index < handlers.len() {
            if handlers[index].is_finished() {
                let handler = handlers.swap_remove(index);
                let finished = handler.join();
                #[cfg(any(test, feature = "test-support"))]
                runtime.notify_handler_finished(&finished);
                report_workspace_service_handler_result(finished);
            } else {
                index += 1;
            }
        }
        if runtime.shutting_down().load(Ordering::Acquire) {
            break;
        }
        // The service has no maximum age: it lives until it stays idle. A
        // request still in flight is not idleness, so the service never
        // stops under a running request (#1251).
        if !handlers.is_empty() {
            last_access = Instant::now();
        }
        if last_access.elapsed() >= idle_timeout {
            runtime.begin_shutdown();
            break;
        }
        match listener.accept() {
            Ok((stream, _addr)) => {
                last_access = Instant::now();
                update_service_record_last_access(&runtime, now_secs());
                let header_clock = SystemClock::start();
                let header_deadline = Deadline::new(&header_clock, request_header_timeout);
                let handler_runtime = Arc::clone(&runtime);
                let handler_executor = Arc::clone(&executor);
                match thread::Builder::new()
                    .name("unica-workspace-connection".into())
                    .spawn(move || {
                        #[cfg(any(test, feature = "test-support"))]
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
    let drain_deadline = Instant::now() + shutdown_grace;
    while !handlers.is_empty() && Instant::now() < drain_deadline {
        let mut index = 0;
        while index < handlers.len() {
            if handlers[index].is_finished() {
                let handler = handlers.swap_remove(index);
                let finished = handler.join();
                #[cfg(any(test, feature = "test-support"))]
                runtime.notify_handler_finished(&finished);
                report_workspace_service_handler_result(finished);
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

pub fn report_workspace_service_handler_result(result: thread::Result<Result<(), String>>) {
    let stderr = io::stderr();
    report_workspace_service_handler_result_to(&mut stderr.lock(), result);
}

pub fn report_workspace_service_handler_result_to(
    writer: &mut dyn Write,
    result: thread::Result<Result<(), String>>,
) {
    if let Some(diagnostic) = workspace_service_handler_diagnostic(result) {
        let _ = writeln!(writer, "{diagnostic}");
    }
}

pub fn workspace_service_handler_diagnostic(
    result: thread::Result<Result<(), String>>,
) -> Option<String> {
    match result {
        Ok(Ok(())) => None,
        Ok(Err(error)) => Some(format!("workspace service connection failed: {error}")),
        Err(_) => Some("workspace service connection handler panicked".to_string()),
    }
}

pub fn handle_workspace_service_stream<B: BslBackend, R: RlmBackend>(
    stream: TcpStream,
    runtime: Arc<WorkspaceServices<B, R>>,
    executor: Arc<dyn WorkspaceServiceOperationExecutor<B, R>>,
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

pub fn handle_workspace_service_request<B: BslBackend, R: RlmBackend>(
    stream: TcpStream,
    runtime: Arc<WorkspaceServices<B, R>>,
    executor: Arc<dyn WorkspaceServiceOperationExecutor<B, R>>,
    request: ServiceRequest,
) -> Result<(), String> {
    if let Err(error) = runtime.authenticate(&request.token) {
        write_service_response(stream, &ServiceResponse::error(error), false)?;
        return Ok(());
    }

    #[cfg(any(test, feature = "test-support"))]
    runtime.notify_authenticated_request(&stream, &request.kind);

    match request.kind {
        ServiceRequestKind::Ping => {
            write_service_response(stream, &runtime.ping(), false)?;
        }
        ServiceRequestKind::Cancel { operation_id } => {
            write_service_response(stream, &runtime.cancel_operation(&operation_id), true)?;
        }
        ServiceRequestKind::Invalidate { events } => {
            let response = runtime.invalidate(&events);
            #[cfg(any(test, feature = "test-support"))]
            {
                let hook = runtime
                    .actor
                    .runtime_projection()
                    .0
                    .invalidation_response_hook
                    .lock()
                    .unwrap()
                    .clone();
                if let Some(hook) = hook {
                    hook();
                }
            }
            write_service_response(stream, &response, false)?;
        }
        ServiceRequestKind::Shutdown => {
            write_service_response(stream, &runtime.begin_shutdown(), false)?;
        }
        kind @ (ServiceRequestKind::BslMcp { .. }
        | ServiceRequestKind::RlmReady { .. }
        | ServiceRequestKind::RlmMcp { .. }) => {
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
                    let response = executor.execute(&worker_runtime, kind, &worker_cancellation);
                    drop(guard);
                    let _ = result_tx.send(response);
                });
            let mut caller_error = configure_workspace_caller_monitor(&stream, &runtime)
                .err()
                .map(|error| format!("failed to monitor workspace service caller: {error}"));
            let mut monitor_caller = caller_error.is_none();
            #[cfg(any(test, feature = "test-support"))]
            if !monitor_caller {
                runtime.notify_caller_detached(&cancellation);
            }
            loop {
                match result_rx.recv_timeout(Duration::from_millis(50)) {
                    Ok(response) => {
                        // An inbound FIN can be a half-close: the peer may still
                        // read the result. Delivery is attempted only after the
                        // accepted worker has really completed, even if its
                        // transport could no longer be monitored.
                        stream.set_nonblocking(false).map_err(|err| {
                            format!("failed to restore workspace service response stream: {err}")
                        })?;
                        write_service_response(stream, &response, false)?;
                        break;
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => {
                        stream.set_nonblocking(false).map_err(|err| {
                            format!("failed to restore workspace service response stream: {err}")
                        })?;
                        write_service_response(
                            stream,
                            &ServiceResponse::error("workspace service worker disconnected"),
                            false,
                        )?;
                        return Err("workspace service worker disconnected".into());
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        if !monitor_caller {
                            continue;
                        }
                        let mut byte = [0_u8; 1];
                        match peek_workspace_caller(&stream, &mut byte, &runtime) {
                            Ok(0) => {
                                monitor_caller = false;
                            }
                            Ok(_) => {}
                            Err(error)
                                if matches!(
                                    error.kind(),
                                    ErrorKind::WouldBlock | ErrorKind::Interrupted
                                ) => {}
                            Err(error) => {
                                caller_error = Some(format!(
                                    "failed to monitor workspace service caller: {error}"
                                ));
                                monitor_caller = false;
                            }
                        }
                        #[cfg(any(test, feature = "test-support"))]
                        if !monitor_caller {
                            runtime.notify_caller_detached(&cancellation);
                        }
                    }
                }
            }
            if let Some(error) = caller_error {
                return Err(error);
            }
        }
    }
    Ok(())
}

pub fn configure_workspace_caller_monitor<B: BslBackend, R: RlmBackend>(
    stream: &TcpStream,
    _runtime: &WorkspaceServices<B, R>,
) -> io::Result<()> {
    #[cfg(any(test, feature = "test-support"))]
    if _runtime.take_caller_monitor_fault(WorkspaceCallerMonitorPoint::Setup) {
        return Err(io::Error::other("injected caller monitor setup failure"));
    }
    stream.set_nonblocking(true)
}

pub fn peek_workspace_caller<B: BslBackend, R: RlmBackend>(
    stream: &TcpStream,
    byte: &mut [u8],
    _runtime: &WorkspaceServices<B, R>,
) -> io::Result<usize> {
    #[cfg(any(test, feature = "test-support"))]
    if _runtime.take_caller_monitor_fault(WorkspaceCallerMonitorPoint::Peek) {
        return Err(io::Error::other("injected caller monitor peek failure"));
    }
    stream.peek(byte)
}

#[derive(Debug)]
pub enum BoundedServiceLine {
    Line(Vec<u8>),
    TooLarge,
}

#[cfg(any(test, feature = "test-support"))]
pub fn read_bounded_service_line<R: BufRead>(
    reader: &mut R,
) -> io::Result<Option<BoundedServiceLine>> {
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

pub fn read_bounded_service_line_with_deadline<R, F>(
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

pub fn write_service_response(
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

pub fn required_arg(args: &[String], name: &str) -> Result<String, String> {
    args.windows(2)
        .find_map(|pair| (pair[0] == name).then(|| pair[1].clone()))
        .ok_or_else(|| format!("missing required workspace service argument {name}"))
}

pub fn optional_u64_arg(args: &[String], name: &str, default: u64) -> u64 {
    args.windows(2)
        .find_map(|pair| {
            (pair[0] == name)
                .then(|| pair[1].parse::<u64>().ok())
                .flatten()
        })
        .filter(|value| *value > 0)
        .unwrap_or(default)
}

pub fn service_key(workspace_root: &str, source_root: &str) -> String {
    let mut hasher = DefaultHasher::new();
    workspace_root.hash(&mut hasher);
    source_root.hash(&mut hasher);
    format!("svc-{:016x}", hasher.finish())
}

impl<B: BslBackend, R: RlmBackend> WorkspaceServices<B, R> {}

impl<B: BslBackend, R: RlmBackend> WorkspaceServices<B, R> {
    fn handle_rlm_ready_using<I: RlmIndex>(
        &self,
        _args: Value,
        timeout_seconds: u64,
        timeout_nanos: u32,
        cancellation: &CancellationToken,
        make_index: impl FnOnce(ProviderSourceLease, Arc<dyn BackgroundTaskTracker>) -> I,
    ) -> ServiceResponse {
        let timeout = match duration_from_timeout_parts(timeout_seconds, timeout_nanos) {
            Ok(timeout) => timeout,
            Err(error) => return ServiceResponse::error(error),
        };
        let deadline = ElapsedBudgetDeadline::new(timeout);
        if let Err(error) = deadline.remaining_cancellable(cancellation) {
            return ServiceResponse::error(error);
        }
        let binding = match self.provider_binding() {
            Ok(binding) => binding,
            Err(error) => return ServiceResponse::error(error),
        };
        let service = match self
            .actor
            .provider_source_lease(&binding)
            .map(|lease| make_index(lease, self.rlm_maintenance_tasks().clone()))
        {
            Ok(service) => service,
            Err(error) => return ServiceResponse::error(error),
        };
        let active = match service.active_build(self.context(), binding.source_root()) {
            Ok(active) => active,
            Err(error) => return ServiceResponse::error(error),
        };
        if let Err(error) = deadline.remaining_cancellable(cancellation) {
            return ServiceResponse::error(error);
        }
        let changed = self.rlm_invalidated().swap(false, Ordering::AcqRel);
        let observation = match service.observe_maintenance(
            self.context(),
            binding.source_root(),
            active,
            changed,
            self.rlm_maintenance_cancellation(),
        ) {
            Ok(observation) => observation,
            Err(error) => return ServiceResponse::error(error),
        };
        if observation.keep_invalidated {
            self.rlm_invalidated().store(true, Ordering::Release);
        }
        service.readiness_response(&observation, false)
    }

    fn handle_rlm_ready(
        &self,
        _args: Value,
        timeout_seconds: u64,
        timeout_nanos: u32,
        cancellation: &CancellationToken,
    ) -> ServiceResponse {
        self.handle_rlm_ready_using(
            _args,
            timeout_seconds,
            timeout_nanos,
            cancellation,
            |lease, tracker| {
                self.actor
                    .runtime_projection()
                    .0
                    .rlm_backend
                    .index(lease, tracker)
            },
        )
    }

    fn handle_rlm_mcp_using<I: RlmIndex>(
        &self,
        operation: WorkspaceRlmOperation,
        timeout_seconds: u64,
        timeout_nanos: u32,
        cancellation: &CancellationToken,
        make_index: impl FnOnce(ProviderSourceLease, Arc<dyn BackgroundTaskTracker>) -> I,
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
        let binding = match self.provider_binding() {
            Ok(binding) => binding,
            Err(error) => {
                let stale_session = self
                    .rlm()
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .take();
                if let Some(stale_session) = stale_session {
                    self.retire_session(stale_session);
                }
                return ServiceResponse::error(error);
            }
        };
        let mut rlm = self.rlm().lock().unwrap_or_else(|error| error.into_inner());
        let mut stale_session = None;
        let service = match self
            .actor
            .provider_source_lease(&binding)
            .map(|lease| make_index(lease, self.rlm_maintenance_tasks().clone()))
        {
            Ok(service) => service,
            Err(error) => return ServiceResponse::error(error),
        };
        let active = match service.active_build(self.context(), binding.source_root()) {
            Ok(active) => active,
            Err(error) => return ServiceResponse::error(error),
        };
        if let Err(error) = deadline.remaining_cancellable(cancellation) {
            return ServiceResponse::error(error);
        }
        let changed = self.rlm_invalidated().swap(false, Ordering::AcqRel);
        let observation = match service.observe_maintenance(
            self.context(),
            binding.source_root(),
            active,
            changed,
            self.rlm_maintenance_cancellation(),
        ) {
            Ok(observation) => observation,
            Err(error) => return ServiceResponse::error(error),
        };
        if observation.keep_invalidated {
            self.rlm_invalidated().store(true, Ordering::Release);
        }
        let Some(active) = observation.active.as_ref() else {
            return service.readiness_response(&observation, true);
        };
        if rlm.as_mut().is_some_and(|session| {
            session.build_id() != active.build_id() || !session.is_reusable()
        }) {
            stale_session = rlm.take();
        }
        if let Some(stale_session) = stale_session {
            drop(rlm);
            self.retire_session(stale_session);
            rlm = self.rlm().lock().unwrap_or_else(|error| error.into_inner());
        }
        let result = self.actor.read(&binding, |source_root| {
            deadline.remaining_cancellable(cancellation)?;
            if rlm.is_none() {
                *rlm = Some((self.rlm_starter())(
                    self.context(),
                    source_root,
                    active.directory(),
                    active.build_id(),
                    cancellation,
                )?);
            }
            let remaining = deadline.remaining_cancellable(cancellation)?;
            rlm.as_mut()
                .expect("RLM session must exist after start")
                .execute(&operation, remaining, cancellation)
        });
        match result {
            Ok(output) => {
                if let Err(error) = self.validate_provider_binding(&binding) {
                    let stale_session = rlm.take();
                    drop(rlm);
                    if let Some(stale_session) = stale_session {
                        self.retire_session(stale_session);
                    }
                    return ServiceResponse::error(error);
                }
                ServiceResponse {
                    ok: true,
                    result_text: Some(output.result_text),
                    stderr: Some(output.stderr),
                    index_freshness: Some("unknown".to_string()),
                    index_build_id: Some(active.build_id().to_string()),
                    index_maintenance: observation.maintenance,
                    warnings: observation.warnings,
                    ..ServiceResponse::default()
                }
            }
            Err(error) => {
                let root_changed = self.validate_provider_binding(&binding).is_err();
                let stale_session =
                    if root_changed || rlm.as_mut().is_some_and(|session| !session.is_reusable()) {
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

    fn handle_rlm_mcp(
        &self,
        operation: WorkspaceRlmOperation,
        timeout_seconds: u64,
        timeout_nanos: u32,
        cancellation: &CancellationToken,
    ) -> ServiceResponse {
        self.handle_rlm_mcp_using(
            operation,
            timeout_seconds,
            timeout_nanos,
            cancellation,
            |lease, tracker| {
                self.actor
                    .runtime_projection()
                    .0
                    .rlm_backend
                    .index(lease, tracker)
            },
        )
    }
}

#[cfg(any(test, feature = "test-support"))]
impl<B: BslBackend, R: RlmBackend> WorkspaceServices<B, R> {
    pub fn test_set_analyzer_starter(&mut self, starter: Arc<BslSessionStarter>) {
        self.runtime_mut_for_test().analyzer_starter = starter;
    }
    pub fn test_set_rlm_starter(&mut self, starter: Arc<RlmSessionStarter<R>>) {
        self.runtime_mut_for_test().rlm_starter = starter;
    }
    pub fn test_set_authenticated_request_hook(
        &self,
        hook: Option<Arc<AuthenticatedWorkspaceRequestHook>>,
    ) {
        *self
            .actor
            .runtime_projection()
            .0
            .authenticated_request_hook
            .lock()
            .unwrap() = hook;
    }
    pub fn test_set_handler_finished_hook(&self, hook: Option<Arc<WorkspaceHandlerFinishedHook>>) {
        *self
            .actor
            .runtime_projection()
            .0
            .handler_finished_hook
            .lock()
            .unwrap() = hook;
    }
    pub fn test_set_caller_detached_hook(&self, hook: Option<Arc<DetachedWorkspaceCallerHook>>) {
        *self
            .actor
            .runtime_projection()
            .0
            .caller_detached_hook
            .lock()
            .unwrap() = hook;
    }
    pub fn test_set_caller_monitor_fault(&self, fault: Option<WorkspaceCallerMonitorPoint>) {
        *self
            .actor
            .runtime_projection()
            .0
            .caller_monitor_fault
            .lock()
            .unwrap() = fault;
    }
    pub fn test_set_invalidation_response_hook(&self, hook: Option<Arc<dyn Fn() + Send + Sync>>) {
        *self
            .actor
            .runtime_projection()
            .0
            .invalidation_response_hook
            .lock()
            .unwrap() = hook;
    }
    pub fn test_handle_rlm_ready_using<I: RlmIndex>(
        &self,
        args: Value,
        seconds: u64,
        nanos: u32,
        cancel: &CancellationToken,
        index: impl FnOnce(ProviderSourceLease, Arc<dyn BackgroundTaskTracker>) -> I,
    ) -> ServiceResponse {
        self.handle_rlm_ready_using(args, seconds, nanos, cancel, index)
    }
    pub fn test_handle_rlm_mcp_using<I: RlmIndex>(
        &self,
        operation: WorkspaceRlmOperation,
        seconds: u64,
        nanos: u32,
        cancel: &CancellationToken,
        index: impl FnOnce(ProviderSourceLease, Arc<dyn BackgroundTaskTracker>) -> I,
    ) -> ServiceResponse {
        self.handle_rlm_mcp_using(operation, seconds, nanos, cancel, index)
    }
}

#[cfg(any(test, feature = "test-support"))]
impl<B: BslBackend, R: RlmBackend> WorkspaceServices<B, R> {
    pub fn test_workspace_identity(&self) -> &WorkspaceIdentity {
        self.workspace_identity()
    }
    pub fn test_identity(&self) -> &WorkspaceServiceIdentity {
        self.identity()
    }
    pub fn test_token(&self) -> &str {
        self.token()
    }
    pub fn test_record_owner(&self) -> &WorkspaceServiceRecord {
        self.record_owner()
    }
    pub fn test_analyzer_lane(&self) -> &AnalyzerLane {
        self.analyzer_lane()
    }
    pub fn test_analyzer(&self) -> &Mutex<Option<PersistentMcpSession>> {
        self.analyzer()
    }
    pub fn test_analyzer_starter(&self) -> &Arc<BslSessionStarter> {
        self.analyzer_starter()
    }
    pub fn test_rlm_lane(&self) -> &AnalyzerLane {
        self.rlm_lane()
    }
    pub fn test_rlm(&self) -> &Mutex<Option<<R as RlmBackend>::Session>> {
        self.rlm()
    }
    pub fn test_rlm_starter(&self) -> &Arc<RlmSessionStarter<R>> {
        self.rlm_starter()
    }
    pub fn test_rlm_maintenance_cancellation(&self) -> &CancellationToken {
        self.rlm_maintenance_cancellation()
    }
    pub fn test_rlm_maintenance_tasks(&self) -> &Arc<SessionTeardownLifecycle> {
        self.rlm_maintenance_tasks()
    }
    pub fn test_session_teardowns(&self) -> &SessionTeardownLifecycle {
        self.session_teardowns()
    }
    pub fn test_analyzer_invalidated(&self) -> &AtomicBool {
        self.analyzer_invalidated()
    }
    pub fn test_rlm_invalidated(&self) -> &AtomicBool {
        self.rlm_invalidated()
    }
    pub fn test_operations(&self) -> &Mutex<HashMap<String, CancellationToken>> {
        self.operations()
    }
    pub fn test_shutting_down(&self) -> &AtomicBool {
        self.shutting_down()
    }
    pub fn test_handler_started_hook(&self) -> &Mutex<Option<Arc<dyn Fn() + Send + Sync>>> {
        self.handler_started_hook()
    }
    pub fn test_provider_sessions_drained_hook(
        &self,
    ) -> &Mutex<Option<Arc<dyn Fn() + Send + Sync>>> {
        self.provider_sessions_drained_hook()
    }
    pub fn test_context(&self) -> &WorkspaceContext {
        self.context()
    }
    pub fn test_bind_provider_root(
        &self,
        source_set_name: &str,
        source_root: &Path,
    ) -> Result<ProviderRootBinding, String> {
        self.bind_provider_root(source_set_name, source_root)
    }
    pub fn test_validate_provider_binding(
        &self,
        binding: &ProviderRootBinding,
    ) -> Result<(), String> {
        self.validate_provider_binding(binding)
    }
    pub fn test_ping(&self) -> ServiceResponse {
        self.ping()
    }
    pub fn test_authenticate(&self, token: &str) -> Result<(), &'static str> {
        self.authenticate(token)
    }
    pub fn test_owns_record(&self, record: &WorkspaceServiceRecord) -> bool {
        self.owns_record(record)
    }
    pub fn test_register_operation(
        self: &Arc<Self>,
        operation_id: String,
    ) -> Result<(CancellationToken, OperationGuard<B, R>), String> {
        self.register_operation(operation_id)
    }
    pub fn test_cancel_operation(&self, operation_id: &str) -> ServiceResponse {
        self.cancel_operation(operation_id)
    }
    pub fn test_begin_shutdown(&self) -> ServiceResponse {
        self.begin_shutdown()
    }
    pub fn test_shutdown_provider_sessions(&self) -> bool {
        self.shutdown_provider_sessions()
    }
    pub fn test_invalidate(&self, events: &[DomainEvent]) -> ServiceResponse {
        self.invalidate(events)
    }
    pub fn test_acquire_analyzer_lane(
        &self,
        cancellation: &CancellationToken,
    ) -> Result<AnalyzerLanePermit<'_>, String> {
        self.acquire_analyzer_lane(cancellation)
    }
    pub fn test_acquire_rlm_lane(
        &self,
        cancellation: &CancellationToken,
    ) -> Result<AnalyzerLanePermit<'_>, String> {
        self.acquire_rlm_lane(cancellation)
    }
    pub fn test_handle_bsl_mcp(
        &self,
        tool_name: &str,
        tool_args: Value,
        timeout_seconds: u64,
        timeout_nanos: u32,
        cancellation: &CancellationToken,
    ) -> ServiceResponse {
        self.handle_bsl_mcp(
            tool_name,
            tool_args,
            timeout_seconds,
            timeout_nanos,
            false,
            cancellation,
        )
    }
    pub fn test_handle_background_bsl_mcp(
        &self,
        tool_name: &str,
        cancellation: &CancellationToken,
    ) -> ServiceResponse {
        self.handle_bsl_mcp(tool_name, Value::Null, 30, 0, true, cancellation)
    }
    pub fn test_provider_binding(&self) -> Result<ProviderRootBinding, String> {
        self.provider_binding()
    }
    pub fn test_retire_session<T: Send + 'static>(&self, session: T) {
        self.retire_session(session)
    }
}

#[cfg(any(test, feature = "test-support"))]
impl<B: BslBackend, R: RlmBackend> WorkspaceServices<B, R> {
    pub fn test_handle_rlm_ready(
        &self,
        args: Value,
        seconds: u64,
        nanos: u32,
        cancel: &CancellationToken,
    ) -> ServiceResponse {
        self.handle_rlm_ready(args, seconds, nanos, cancel)
    }
    pub fn test_handle_rlm_mcp(
        &self,
        operation: WorkspaceRlmOperation,
        seconds: u64,
        nanos: u32,
        cancel: &CancellationToken,
    ) -> ServiceResponse {
        self.handle_rlm_mcp(operation, seconds, nanos, cancel)
    }
}

#[cfg(any(test, feature = "test-support"))]
impl<B: BslBackend, R: RlmBackend> WorkspaceServices<B, R> {
    pub fn test_assert_actor_owned(&self) {
        fn actor_owned<R>(_: &unica_workspace::actor::WorkspaceActor<R>) {}
        actor_owned(&self.actor);
    }
}
#[cfg(any(test, feature = "test-support"))]
impl WorkspaceServiceCallDeadline {
    pub fn test_with_started(started: Instant, budget: Duration) -> Self {
        Self { started, budget }
    }
}
