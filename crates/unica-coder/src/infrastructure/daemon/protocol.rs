use super::identity::CoreIdentity;
use crate::application::invocation_store::{
    canonical_result_size, CanonicalResultSizeError, ToolIdentity, MAX_CANONICAL_RESULT_BYTES,
    MAX_TASK_RECORD_ENVELOPE_BYTES,
};
use crate::domain::invocation::{
    DomainResult, InvocationFailure, InvocationId, InvocationStatus, TaskId,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::io::{self, BufRead};
use std::net::{Ipv4Addr, SocketAddrV4};
use uuid::Uuid;

pub(crate) const DAEMON_PROTOCOL_VERSION: u32 = 3;
pub(crate) const ENDPOINT_SCHEMA_VERSION: u32 = 1;
pub(crate) const MAX_DAEMON_REQUEST_LINE_BYTES: usize = 16 * 1024;
pub(crate) const MAX_ENDPOINT_RECORD_BYTES: usize = 16 * 1024;
pub(crate) const MAX_DAEMON_RESPONSE_LINE_BYTES: usize =
    MAX_CANONICAL_RESULT_BYTES + MAX_TASK_RECORD_ENVELOPE_BYTES;
pub(crate) const MAX_TASK_WAIT_MS: u64 = 7_000;

/// One canonical v0.13 call submitted to the daemon. Raw arguments exist only
/// on this authenticated live connection; durable state receives their digest.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct InvocationRequest {
    tool: ToolIdentity,
    arguments: Map<String, Value>,
    workspace_hint: String,
    response_budget_ms: u64,
}

impl InvocationRequest {
    pub(crate) fn new(
        tool: ToolIdentity,
        arguments: Value,
        workspace_hint: impl Into<String>,
        response_budget_ms: u64,
    ) -> Result<Self, String> {
        let arguments = arguments
            .as_object()
            .cloned()
            .ok_or_else(|| "canonical invocation arguments must be an object".to_string())?;
        let request = Self {
            tool,
            arguments,
            workspace_hint: workspace_hint.into(),
            response_budget_ms,
        };
        request.validate()?;
        Ok(request)
    }

    pub(crate) fn tool(&self) -> ToolIdentity {
        self.tool
    }

    pub(crate) fn arguments(&self) -> &Map<String, Value> {
        &self.arguments
    }

    pub(crate) fn workspace_hint(&self) -> &str {
        &self.workspace_hint
    }

    pub(crate) fn response_budget_ms(&self) -> u64 {
        self.response_budget_ms
    }

    fn validate(&self) -> Result<(), String> {
        if self.response_budget_ms > MAX_TASK_WAIT_MS {
            return Err("canonical invocation response budget must be within 0..=7000 ms".into());
        }
        if self.workspace_hint.is_empty() || self.workspace_hint.chars().any(char::is_control) {
            return Err("canonical invocation workspace hint must be non-empty text".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct DaemonTaskSnapshot {
    pub(crate) task_id: TaskId,
    pub(crate) invocation_id: InvocationId,
    pub(crate) status: InvocationStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) result: Option<DomainResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) failure: Option<InvocationFailure>,
    pub(crate) poll_interval_ms: u64,
    pub(crate) created_at_epoch_ms: u64,
    pub(crate) updated_at_epoch_ms: u64,
    pub(crate) ttl_ms: u64,
}

impl DaemonTaskSnapshot {
    pub(crate) fn from_domain(snapshot: crate::domain::invocation::TaskSnapshot) -> Self {
        Self {
            task_id: snapshot.task_id,
            invocation_id: snapshot.invocation_id,
            status: snapshot.status,
            result: snapshot.result,
            failure: snapshot.failure,
            poll_interval_ms: snapshot.poll_interval_ms,
            created_at_epoch_ms: snapshot.created_at_epoch_ms,
            updated_at_epoch_ms: snapshot.updated_at_epoch_ms,
            ttl_ms: snapshot.ttl_ms,
        }
    }

    #[cfg(test)]
    pub(crate) fn working_for_test(task_id: TaskId) -> Self {
        Self {
            task_id,
            invocation_id: InvocationId::new(),
            status: InvocationStatus::Working,
            result: None,
            failure: None,
            poll_interval_ms: 250,
            created_at_epoch_ms: 1,
            updated_at_epoch_ms: 1,
            ttl_ms: 3_600_000,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "resultType",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub(crate) enum InvocationResponse {
    Direct(DomainResult),
    Task(DaemonTaskSnapshot),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct EndpointRecord {
    schema_version: u32,
    protocol_version: u32,
    core_identity: CoreIdentity,
    pid: u32,
    host: String,
    port: u16,
    token: String,
    instance_id: String,
}

impl EndpointRecord {
    pub(crate) fn new(core_identity: CoreIdentity, port: u16) -> Self {
        Self {
            schema_version: ENDPOINT_SCHEMA_VERSION,
            protocol_version: DAEMON_PROTOCOL_VERSION,
            core_identity,
            pid: std::process::id(),
            host: Ipv4Addr::LOCALHOST.to_string(),
            port,
            token: Uuid::new_v4().to_string(),
            instance_id: Uuid::new_v4().to_string(),
        }
    }

    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.schema_version != ENDPOINT_SCHEMA_VERSION {
            return Err("unsupported daemon endpoint schema".to_string());
        }
        if self.protocol_version != DAEMON_PROTOCOL_VERSION {
            return Err("unsupported daemon endpoint protocol".to_string());
        }
        if self.pid == 0 || self.port == 0 || self.host != Ipv4Addr::LOCALHOST.to_string() {
            return Err("daemon endpoint is not a valid loopback process record".to_string());
        }
        validate_uuid_v4(&self.token, "daemon endpoint token")?;
        validate_uuid_v4(&self.instance_id, "daemon endpoint instance")?;
        Ok(())
    }

    pub(crate) fn core_identity(&self) -> &CoreIdentity {
        &self.core_identity
    }

    pub(crate) fn pid(&self) -> u32 {
        self.pid
    }

    pub(crate) fn token(&self) -> &str {
        &self.token
    }

    pub(crate) fn instance_id(&self) -> &str {
        &self.instance_id
    }

    pub(crate) fn loopback_addr(&self) -> Result<SocketAddrV4, String> {
        self.validate()?;
        Ok(SocketAddrV4::new(Ipv4Addr::LOCALHOST, self.port))
    }

    #[cfg(test)]
    pub(crate) fn test_stale(core_identity: CoreIdentity, pid: u32) -> Self {
        Self {
            pid,
            port: 9,
            ..Self::new(core_identity, 9)
        }
    }

    #[cfg(test)]
    pub(crate) fn test_replacement(record: &Self) -> Self {
        let mut replacement = Self::new(record.core_identity.clone(), record.port);
        replacement.pid = record.pid;
        replacement
    }
}

fn validate_uuid_v4(value: &str, field: &str) -> Result<(), String> {
    let parsed = Uuid::parse_str(value).map_err(|_| format!("{field} is not a UUID"))?;
    if parsed.get_version_num() != 4 || parsed.hyphenated().to_string() != value {
        return Err(format!("{field} is not a canonical UUIDv4"));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum ClientRequest {
    Hello {
        #[serde(rename = "protocolVersion")]
        protocol_version: u32,
        token: String,
        #[serde(rename = "coreIdentity")]
        core_identity: CoreIdentity,
        #[serde(rename = "ownerLease")]
        owner_lease: String,
    },
    Ping {},
    Release {},
    SubmitInvocation {
        invocation: InvocationRequest,
    },
    GetTask {
        #[serde(rename = "taskId")]
        task_id: TaskId,
    },
    WaitTask {
        #[serde(rename = "taskId")]
        task_id: TaskId,
        #[serde(rename = "waitMs")]
        wait_ms: u64,
    },
    CancelTask {
        #[serde(rename = "taskId")]
        task_id: TaskId,
    },
}

impl ClientRequest {
    pub(crate) fn hello(protocol_version: u32, token: String, core_identity: CoreIdentity) -> Self {
        let request = Self::Hello {
            protocol_version,
            token,
            core_identity,
            owner_lease: Uuid::new_v4().to_string(),
        };
        debug_assert!(
            serde_json::to_vec(&request)
                .ok()
                .and_then(|bytes| parse_request(&bytes).ok())
                .is_some(),
            "locally constructed daemon hello must satisfy the strict protocol"
        );
        request
    }

    #[cfg(test)]
    pub(crate) fn hello_with_owner_for_test(
        token: String,
        core_identity: CoreIdentity,
        owner_lease: String,
    ) -> Self {
        Self::Hello {
            protocol_version: DAEMON_PROTOCOL_VERSION,
            token,
            core_identity,
            owner_lease,
        }
    }

    pub(crate) fn validate(&self) -> Result<(), String> {
        match self {
            Self::Hello {
                token, owner_lease, ..
            } => {
                validate_uuid_v4(token, "daemon handshake token")?;
                validate_uuid_v4(owner_lease, "daemon owner lease")
            }
            Self::SubmitInvocation { invocation } => invocation.validate(),
            Self::WaitTask { wait_ms, .. } if *wait_ms > MAX_TASK_WAIT_MS => {
                Err("daemon task wait exceeds the 7000 ms request bound".to_string())
            }
            Self::Ping {}
            | Self::Release {}
            | Self::GetTask { .. }
            | Self::WaitTask { .. }
            | Self::CancelTask { .. } => Ok(()),
        }
    }

    pub(crate) fn submit_invocation(invocation: InvocationRequest) -> Self {
        Self::SubmitInvocation { invocation }
    }

    #[allow(dead_code)]
    pub(crate) fn get_task(task_id: TaskId) -> Self {
        Self::GetTask { task_id }
    }

    #[allow(dead_code)]
    pub(crate) fn wait_task(task_id: TaskId, wait_ms: u64) -> Self {
        Self::WaitTask { task_id, wait_ms }
    }

    #[allow(dead_code)]
    pub(crate) fn cancel_task(task_id: TaskId) -> Self {
        Self::CancelTask { task_id }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DaemonErrorCode {
    InvalidRequest,
    HandshakeRequired,
    ProtocolMismatch,
    CoreMismatch,
    Unauthorized,
    DuplicateLease,
    Overloaded,
    OwnerCapacity,
    WorkspaceCapacity,
    WorkspaceRegistryFailed,
    TaskCapacity,
    TaskNotFound,
    TaskExpired,
    InvocationFailed,
    ResultTooLarge,
    StoreFailed,
    DurabilityUncertain,
}

impl std::fmt::Display for DaemonErrorCode {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let code = match self {
            Self::InvalidRequest => "invalid_request",
            Self::HandshakeRequired => "handshake_required",
            Self::ProtocolMismatch => "protocol_mismatch",
            Self::CoreMismatch => "core_mismatch",
            Self::Unauthorized => "unauthorized",
            Self::DuplicateLease => "duplicate_lease",
            Self::Overloaded => "overloaded",
            Self::OwnerCapacity => "owner_capacity",
            Self::WorkspaceCapacity => "workspace_capacity",
            Self::WorkspaceRegistryFailed => "workspace_registry_failed",
            Self::TaskCapacity => "task_capacity",
            Self::TaskNotFound => "task_not_found",
            Self::TaskExpired => "task_expired",
            Self::InvocationFailed => "invocation_failed",
            Self::ResultTooLarge => "result_too_large",
            Self::StoreFailed => "store_failed",
            Self::DurabilityUncertain => "durability_uncertain",
        };
        formatter.write_str(code)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum ServerResponse {
    Ready {
        #[serde(rename = "protocolVersion")]
        protocol_version: u32,
        #[serde(rename = "coreIdentity")]
        core_identity: CoreIdentity,
        #[serde(rename = "daemonPid")]
        daemon_pid: u32,
        #[serde(rename = "instanceId")]
        instance_id: String,
    },
    Pong,
    Released,
    Invocation {
        outcome: InvocationResponse,
    },
    Task {
        snapshot: DaemonTaskSnapshot,
    },
    Error {
        code: DaemonErrorCode,
    },
}

impl ServerResponse {
    pub(crate) fn ready(record: &EndpointRecord) -> Self {
        Self::Ready {
            protocol_version: DAEMON_PROTOCOL_VERSION,
            core_identity: record.core_identity.clone(),
            daemon_pid: record.pid,
            instance_id: record.instance_id.clone(),
        }
    }

    pub(crate) fn error(code: DaemonErrorCode) -> Self {
        Self::Error { code }
    }

    pub(crate) fn invocation(outcome: InvocationResponse) -> Self {
        Self::Invocation { outcome }
    }

    pub(crate) fn task(snapshot: DaemonTaskSnapshot) -> Self {
        Self::Task { snapshot }
    }

    pub(crate) fn error_code(&self) -> Option<DaemonErrorCode> {
        match self {
            Self::Error { code } => Some(*code),
            _ => None,
        }
    }

    pub(crate) fn matches_record(&self, record: &EndpointRecord) -> bool {
        matches!(
            self,
            Self::Ready {
                protocol_version: DAEMON_PROTOCOL_VERSION,
                core_identity,
                daemon_pid,
                instance_id,
            } if core_identity == record.core_identity()
                && *daemon_pid == record.pid()
                && instance_id == record.instance_id()
        )
    }
}

fn read_bounded_json_line_with_limit<R: BufRead>(
    reader: &mut R,
    max_bytes: usize,
) -> io::Result<Vec<u8>> {
    let mut line = Vec::new();
    loop {
        let buffer = reader.fill_buf()?;
        if buffer.is_empty() {
            return if line.is_empty() {
                Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "JSON line ended before data",
                ))
            } else {
                Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "JSON line is missing its terminator",
                ))
            };
        }
        let newline = buffer.iter().position(|byte| *byte == b'\n');
        let take = newline.map_or(buffer.len(), |position| position + 1);
        if line.len().saturating_add(take) > max_bytes {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "daemon JSON line exceeds the byte limit",
            ));
        }
        line.extend_from_slice(&buffer[..take]);
        reader.consume(take);
        if newline.is_some() {
            if line.last() == Some(&b'\n') {
                line.pop();
            }
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            if line.is_empty() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "daemon JSON line is empty",
                ));
            }
            return Ok(line);
        }
    }
}

pub(crate) fn read_bounded_request_line<R: BufRead>(reader: &mut R) -> io::Result<Vec<u8>> {
    read_bounded_json_line_with_limit(reader, MAX_DAEMON_REQUEST_LINE_BYTES)
}

pub(crate) fn read_bounded_response_line<R: BufRead>(reader: &mut R) -> io::Result<Vec<u8>> {
    read_bounded_json_line_with_limit(reader, MAX_DAEMON_RESPONSE_LINE_BYTES)
}

#[cfg(test)]
pub(crate) fn read_bounded_json_line<R: BufRead>(reader: &mut R) -> io::Result<Vec<u8>> {
    read_bounded_request_line(reader)
}

pub(crate) fn parse_endpoint_record(bytes: &[u8]) -> Result<EndpointRecord, String> {
    let record: EndpointRecord = serde_json::from_slice(bytes)
        .map_err(|_| "daemon endpoint record is not strict versioned JSON".to_string())?;
    record.validate()?;
    Ok(record)
}

pub(crate) fn parse_request(bytes: &[u8]) -> Result<ClientRequest, String> {
    let request: ClientRequest = serde_json::from_slice(bytes)
        .map_err(|_| "daemon request is not strict versioned JSON".to_string())?;
    request.validate()?;
    Ok(request)
}

pub(crate) fn parse_response(bytes: &[u8]) -> Result<ServerResponse, String> {
    let response: ServerResponse = serde_json::from_slice(bytes)
        .map_err(|_| "daemon response is not strict versioned JSON".to_string())?;
    for result in response_domain_results(&response) {
        match canonical_result_size(result) {
            Ok(_) => {}
            Err(CanonicalResultSizeError::TooLarge) => {
                return Err("daemon response canonical result exceeds the byte limit".to_string())
            }
            Err(CanonicalResultSizeError::Checkpoint(never)) => match never {},
            Err(CanonicalResultSizeError::Serialization) => {
                return Err("daemon response canonical result is not serializable".to_string())
            }
        }
    }
    Ok(response)
}

fn response_domain_results(response: &ServerResponse) -> impl Iterator<Item = &DomainResult> {
    let direct = match response {
        ServerResponse::Invocation {
            outcome: InvocationResponse::Direct(result),
        } => Some(result),
        _ => None,
    };
    let task = match response {
        ServerResponse::Invocation {
            outcome: InvocationResponse::Task(snapshot),
        }
        | ServerResponse::Task { snapshot } => snapshot.result.as_ref(),
        _ => None,
    };
    direct.into_iter().chain(task)
}
