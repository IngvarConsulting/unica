use super::identity::CoreIdentity;
use serde::{Deserialize, Serialize};
use std::io::{self, BufRead};
use std::net::{Ipv4Addr, SocketAddrV4};
use uuid::Uuid;

pub(crate) const DAEMON_PROTOCOL_VERSION: u32 = 1;
pub(crate) const ENDPOINT_SCHEMA_VERSION: u32 = 1;
pub(crate) const MAX_JSON_LINE_BYTES: usize = 16 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
            Self::Ping {} | Self::Release {} => Ok(()),
        }
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
        };
        formatter.write_str(code)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

pub(crate) fn read_bounded_json_line<R: BufRead>(reader: &mut R) -> io::Result<Vec<u8>> {
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
        if line.len().saturating_add(take) > MAX_JSON_LINE_BYTES {
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
    serde_json::from_slice(bytes)
        .map_err(|_| "daemon response is not strict versioned JSON".to_string())
}
