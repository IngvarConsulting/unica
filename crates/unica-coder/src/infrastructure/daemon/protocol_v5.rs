use super::identity::{CoreIdentity, DaemonProtocolIdentity};
use crate::application::invocation_store::{
    MAX_CANONICAL_RESULT_BYTES, MAX_TASK_RECORD_ENVELOPE_BYTES,
};
use crate::application::receipt_ledger::{ReceiptKey, TerminalDigest, V5ToolIdentity};
use crate::domain::invocation::{InvocationId, TaskId};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::fmt;
use std::io::{self, BufRead};
use uuid::{Uuid, Variant, Version};

pub(crate) const DAEMON_PROTOCOL_VERSION: u32 = DaemonProtocolIdentity::V5.protocol_version();
pub(crate) const DAEMON_PROTOCOL_IDENTITY: &str = "unica-daemon-jsonl-5";
pub(crate) const MAX_V5_REQUEST_LINE_BYTES: usize = 16 * 1024;
pub(crate) const MAX_V5_RESPONSE_LINE_BYTES: usize =
    MAX_CANONICAL_RESULT_BYTES + MAX_TASK_RECORD_ENVELOPE_BYTES;
const MAX_V5_WAIT_MS: u64 = 7_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum V5ClientRequestKind {
    Hello,
    Ping,
    Release,
    SubmitInvocation,
    GetTask,
    WaitTask,
    CancelTask,
    RecoverInvocationReceipt,
    AcknowledgeInvocationReceipt,
    CancelInvocation,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct V5InvocationRequest {
    invocation_id: InvocationId,
    reserved_task_id: TaskId,
    tool: V5ToolIdentity,
    arguments: Map<String, Value>,
    workspace_hint: String,
    response_budget_ms: u64,
}

impl V5InvocationRequest {
    pub(crate) fn invocation_id(&self) -> InvocationId {
        self.invocation_id
    }

    pub(crate) fn reserved_task_id(&self) -> TaskId {
        self.reserved_task_id
    }

    pub(crate) fn tool(&self) -> V5ToolIdentity {
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
        if self.response_budget_ms > MAX_V5_WAIT_MS {
            return Err("v5 invocation response budget exceeds 7000 ms".to_string());
        }
        if self.workspace_hint.is_empty() || self.workspace_hint.chars().any(char::is_control) {
            return Err("v5 invocation workspace hint must be non-empty text".to_string());
        }
        Ok(())
    }
}

impl fmt::Debug for V5InvocationRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("V5InvocationRequest")
            .field("invocation_id", &self.invocation_id)
            .field("reserved_task_id", &self.reserved_task_id)
            .field("tool", &self.tool)
            .field("arguments", &"<redacted>")
            .field("workspace_hint", &"<redacted>")
            .field("response_budget_ms", &self.response_budget_ms)
            .finish()
    }
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum V5ClientRequest {
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
        invocation: V5InvocationRequest,
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
    RecoverInvocationReceipt {
        #[serde(rename = "receiptKey")]
        receipt_key: ReceiptKey,
    },
    AcknowledgeInvocationReceipt {
        #[serde(rename = "receiptKey")]
        receipt_key: ReceiptKey,
        #[serde(rename = "terminalDigest")]
        terminal_digest: TerminalDigest,
    },
    CancelInvocation {
        #[serde(rename = "receiptKey")]
        receipt_key: ReceiptKey,
    },
}

impl V5ClientRequest {
    pub(crate) fn kind(&self) -> V5ClientRequestKind {
        match self {
            Self::Hello { .. } => V5ClientRequestKind::Hello,
            Self::Ping {} => V5ClientRequestKind::Ping,
            Self::Release {} => V5ClientRequestKind::Release,
            Self::SubmitInvocation { .. } => V5ClientRequestKind::SubmitInvocation,
            Self::GetTask { .. } => V5ClientRequestKind::GetTask,
            Self::WaitTask { .. } => V5ClientRequestKind::WaitTask,
            Self::CancelTask { .. } => V5ClientRequestKind::CancelTask,
            Self::RecoverInvocationReceipt { .. } => V5ClientRequestKind::RecoverInvocationReceipt,
            Self::AcknowledgeInvocationReceipt { .. } => {
                V5ClientRequestKind::AcknowledgeInvocationReceipt
            }
            Self::CancelInvocation { .. } => V5ClientRequestKind::CancelInvocation,
        }
    }

    pub(crate) fn hello_protocol_version(&self) -> Option<u32> {
        match self {
            Self::Hello {
                protocol_version, ..
            } => Some(*protocol_version),
            _ => None,
        }
    }

    fn validate(&self) -> Result<(), String> {
        match self {
            Self::Hello {
                token, owner_lease, ..
            } => {
                validate_uuid_v4(token, "v5 daemon handshake token")?;
                validate_uuid_v4(owner_lease, "v5 daemon owner lease")
            }
            Self::SubmitInvocation { invocation } => invocation.validate(),
            Self::WaitTask { wait_ms, .. } if *wait_ms > MAX_V5_WAIT_MS => {
                Err("v5 task wait exceeds 7000 ms".to_string())
            }
            Self::Ping {}
            | Self::Release {}
            | Self::GetTask { .. }
            | Self::WaitTask { .. }
            | Self::CancelTask { .. }
            | Self::RecoverInvocationReceipt { .. }
            | Self::AcknowledgeInvocationReceipt { .. }
            | Self::CancelInvocation { .. } => Ok(()),
        }
    }
}

impl fmt::Debug for V5ClientRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Hello {
                protocol_version,
                core_identity,
                ..
            } => formatter
                .debug_struct("V5Hello")
                .field("protocol_version", protocol_version)
                .field("token", &"<redacted>")
                .field("core_identity", core_identity)
                .field("owner_lease", &"<redacted>")
                .finish(),
            Self::Ping {} => formatter.write_str("V5Ping"),
            Self::Release {} => formatter.write_str("V5Release"),
            Self::SubmitInvocation { invocation } => formatter
                .debug_struct("V5SubmitInvocation")
                .field("invocation", invocation)
                .finish(),
            Self::GetTask { task_id } => formatter
                .debug_struct("V5GetTask")
                .field("task_id", task_id)
                .finish(),
            Self::WaitTask { task_id, wait_ms } => formatter
                .debug_struct("V5WaitTask")
                .field("task_id", task_id)
                .field("wait_ms", wait_ms)
                .finish(),
            Self::CancelTask { task_id } => formatter
                .debug_struct("V5CancelTask")
                .field("task_id", task_id)
                .finish(),
            Self::RecoverInvocationReceipt { .. } => {
                formatter.write_str("V5RecoverInvocationReceipt")
            }
            Self::AcknowledgeInvocationReceipt { .. } => {
                formatter.write_str("V5AcknowledgeInvocationReceipt")
            }
            Self::CancelInvocation { .. } => formatter.write_str("V5CancelInvocation"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum V5DaemonErrorCode {
    InvalidRequest,
    HandshakeRequired,
    ProtocolMismatch,
    CoreMismatch,
    Unauthorized,
    DuplicateLease,
    Overloaded,
    OwnerCapacity,
    ReceiptNotFound,
    ReceiptExpired,
    ReceiptCapacity,
    TombstoneCapacity,
    InvocationIdentityMismatch,
    TaskNotFound,
    TaskExpired,
    StoreFailed,
    DurabilityUncertain,
}

impl V5DaemonErrorCode {
    pub(crate) const ALL: [Self; 17] = [
        Self::InvalidRequest,
        Self::HandshakeRequired,
        Self::ProtocolMismatch,
        Self::CoreMismatch,
        Self::Unauthorized,
        Self::DuplicateLease,
        Self::Overloaded,
        Self::OwnerCapacity,
        Self::ReceiptNotFound,
        Self::ReceiptExpired,
        Self::ReceiptCapacity,
        Self::TombstoneCapacity,
        Self::InvocationIdentityMismatch,
        Self::TaskNotFound,
        Self::TaskExpired,
        Self::StoreFailed,
        Self::DurabilityUncertain,
    ];
}

impl fmt::Display for V5DaemonErrorCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let code = match self {
            Self::InvalidRequest => "invalid_request",
            Self::HandshakeRequired => "handshake_required",
            Self::ProtocolMismatch => "protocol_mismatch",
            Self::CoreMismatch => "core_mismatch",
            Self::Unauthorized => "unauthorized",
            Self::DuplicateLease => "duplicate_lease",
            Self::Overloaded => "overloaded",
            Self::OwnerCapacity => "owner_capacity",
            Self::ReceiptNotFound => "receipt_not_found",
            Self::ReceiptExpired => "receipt_expired",
            Self::ReceiptCapacity => "receipt_capacity",
            Self::TombstoneCapacity => "tombstone_capacity",
            Self::InvocationIdentityMismatch => "invocation_identity_mismatch",
            Self::TaskNotFound => "task_not_found",
            Self::TaskExpired => "task_expired",
            Self::StoreFailed => "store_failed",
            Self::DurabilityUncertain => "durability_uncertain",
        };
        formatter.write_str(code)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum V5ProbeResponseKind {
    Pong,
    Error,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum V5ProbeServerResponse {
    Pong {},
    Error { code: V5DaemonErrorCode },
}

impl V5ProbeServerResponse {
    pub(crate) fn kind(&self) -> V5ProbeResponseKind {
        match self {
            Self::Pong {} => V5ProbeResponseKind::Pong,
            Self::Error { .. } => V5ProbeResponseKind::Error,
        }
    }

    pub(crate) fn error_code(&self) -> Option<V5DaemonErrorCode> {
        match self {
            Self::Error { code } => Some(*code),
            Self::Pong {} => None,
        }
    }
}

pub(crate) struct DecodedV5Request {
    raw_frame: Vec<u8>,
    request: V5ClientRequest,
}

impl DecodedV5Request {
    pub(crate) fn raw_frame(&self) -> &[u8] {
        &self.raw_frame
    }

    pub(crate) fn request(&self) -> &V5ClientRequest {
        &self.request
    }

    pub(crate) fn into_request(self) -> V5ClientRequest {
        self.request
    }
}

#[derive(Debug)]
pub(crate) enum V5RequestFrameError {
    Read(io::Error),
    InvalidRequest(String),
}

impl fmt::Display for V5RequestFrameError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read(error) => write!(formatter, "read v5 daemon request frame: {error}"),
            Self::InvalidRequest(error) => formatter.write_str(error),
        }
    }
}

impl std::error::Error for V5RequestFrameError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Read(error) => Some(error),
            Self::InvalidRequest(_) => None,
        }
    }
}

pub(crate) fn read_and_decode_v5_request<R: BufRead>(
    reader: &mut R,
) -> Result<DecodedV5Request, V5RequestFrameError> {
    let raw_frame = read_bounded_v5_request_frame(reader).map_err(V5RequestFrameError::Read)?;
    let request =
        decode_v5_client_request(&raw_frame).map_err(V5RequestFrameError::InvalidRequest)?;
    Ok(DecodedV5Request { raw_frame, request })
}

pub(crate) fn read_bounded_v5_request_frame<R: BufRead>(reader: &mut R) -> io::Result<Vec<u8>> {
    read_bounded_json_line_with_limit(reader, MAX_V5_REQUEST_LINE_BYTES)
}

pub(crate) fn read_bounded_v5_probe_response_frame<R: BufRead>(
    reader: &mut R,
) -> io::Result<Vec<u8>> {
    read_bounded_json_line_with_limit(reader, MAX_V5_RESPONSE_LINE_BYTES)
}

fn read_bounded_json_line_with_limit<R: BufRead>(
    reader: &mut R,
    max_bytes: usize,
) -> io::Result<Vec<u8>> {
    let mut line = Vec::new();
    loop {
        let buffer = match reader.fill_buf() {
            Ok(buffer) => buffer,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        };
        if buffer.is_empty() {
            return if line.is_empty() {
                Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "v5 JSON line ended before data",
                ))
            } else {
                Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "v5 JSON line is missing its terminator",
                ))
            };
        }
        let newline = buffer.iter().position(|byte| *byte == b'\n');
        let take = newline.map_or(buffer.len(), |position| position + 1);
        if line.len().saturating_add(take) > max_bytes {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "v5 daemon JSON line exceeds the byte limit",
            ));
        }
        line.extend_from_slice(&buffer[..take]);
        reader.consume(take);
        if newline.is_some() {
            line.pop();
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            if line.is_empty() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "v5 daemon JSON line is empty",
                ));
            }
            return Ok(line);
        }
    }
}

pub(crate) fn decode_v5_client_request(bytes: &[u8]) -> Result<V5ClientRequest, String> {
    ensure_frame_fits(bytes, MAX_V5_REQUEST_LINE_BYTES, "request")?;
    let request: V5ClientRequest = serde_json::from_slice(bytes)
        .map_err(|_| "v5 daemon request is not strict versioned JSON".to_string())?;
    request.validate()?;
    Ok(request)
}

pub(crate) fn decode_v5_probe_response(bytes: &[u8]) -> Result<V5ProbeServerResponse, String> {
    ensure_frame_fits(bytes, MAX_V5_RESPONSE_LINE_BYTES, "response")?;
    serde_json::from_slice(bytes)
        .map_err(|_| "v5 daemon probe response is not strict versioned JSON".to_string())
}

fn ensure_frame_fits(bytes: &[u8], max_bytes: usize, kind: &str) -> Result<(), String> {
    if bytes.len().saturating_add(1) > max_bytes {
        return Err(format!("v5 daemon {kind} frame exceeds the byte limit"));
    }
    Ok(())
}

fn validate_uuid_v4(value: &str, field: &str) -> Result<(), String> {
    let parsed = Uuid::parse_str(value).map_err(|_| format!("{field} is not a UUID"))?;
    if value.len() != 36
        || parsed.hyphenated().to_string() != value
        || parsed.get_variant() != Variant::RFC4122
        || parsed.get_version() != Some(Version::Random)
    {
        return Err(format!("{field} is not a canonical UUIDv4"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Cursor, Read};

    const INVOCATION_ID: &str = "11111111-1111-4111-8111-111111111111";
    const TASK_ID: &str = "22222222-2222-4222-8222-222222222222";
    const CORE_IDENTITY: &str = "884b76181583ce34907a2a9758e2b493e5b40883e7cbb0d7f88dcec0e468cfa0";
    const ZERO_DIGEST: &str = "0000000000000000000000000000000000000000000000000000000000000000";

    fn receipt_key_json() -> String {
        format!(
            "{{\"invocationId\":\"{INVOCATION_ID}\",\"reservedTaskId\":\"{TASK_ID}\",\"coreIdentityDigest\":\"{CORE_IDENTITY}\",\"tool\":\"unica.view\",\"normalizedArgumentsHash\":\"{ZERO_DIGEST}\",\"requestScopeHash\":\"{ZERO_DIGEST}\"}}"
        )
    }

    fn valid_request_frames() -> Vec<(V5ClientRequestKind, String)> {
        vec![
            (
                V5ClientRequestKind::Hello,
                format!(
                    "{{\"kind\":\"hello\",\"protocolVersion\":5,\"token\":\"33333333-3333-4333-8333-333333333333\",\"coreIdentity\":\"{CORE_IDENTITY}\",\"ownerLease\":\"44444444-4444-4444-8444-444444444444\"}}"
                ),
            ),
            (V5ClientRequestKind::Ping, "{\"kind\":\"ping\"}".into()),
            (
                V5ClientRequestKind::Release,
                "{\"kind\":\"release\"}".into(),
            ),
            (
                V5ClientRequestKind::SubmitInvocation,
                format!(
                    "{{\"kind\":\"submit_invocation\",\"invocation\":{{\"invocationId\":\"{INVOCATION_ID}\",\"reservedTaskId\":\"{TASK_ID}\",\"tool\":\"unica.view\",\"arguments\":{{}},\"workspaceHint\":\"workspace-a\",\"responseBudgetMs\":7000}}}}"
                ),
            ),
            (
                V5ClientRequestKind::GetTask,
                format!("{{\"kind\":\"get_task\",\"taskId\":\"{TASK_ID}\"}}"),
            ),
            (
                V5ClientRequestKind::WaitTask,
                format!(
                    "{{\"kind\":\"wait_task\",\"taskId\":\"{TASK_ID}\",\"waitMs\":7000}}"
                ),
            ),
            (
                V5ClientRequestKind::CancelTask,
                format!("{{\"kind\":\"cancel_task\",\"taskId\":\"{TASK_ID}\"}}"),
            ),
            (
                V5ClientRequestKind::RecoverInvocationReceipt,
                format!(
                    "{{\"kind\":\"recover_invocation_receipt\",\"receiptKey\":{}}}",
                    receipt_key_json()
                ),
            ),
            (
                V5ClientRequestKind::AcknowledgeInvocationReceipt,
                format!(
                    "{{\"kind\":\"acknowledge_invocation_receipt\",\"receiptKey\":{},\"terminalDigest\":\"{ZERO_DIGEST}\"}}",
                    receipt_key_json()
                ),
            ),
            (
                V5ClientRequestKind::CancelInvocation,
                format!(
                    "{{\"kind\":\"cancel_invocation\",\"receiptKey\":{}}}",
                    receipt_key_json()
                ),
            ),
        ]
    }

    #[test]
    fn bounded_v5_reader_rejects_oversized_empty_and_unterminated_frames() {
        let mut exact = vec![b'x'; MAX_V5_REQUEST_LINE_BYTES - 1];
        exact.push(b'\n');
        assert_eq!(
            read_bounded_v5_request_frame(&mut BufReader::new(Cursor::new(exact)))
                .unwrap()
                .len(),
            MAX_V5_REQUEST_LINE_BYTES - 1
        );

        let mut oversized = vec![b'x'; MAX_V5_REQUEST_LINE_BYTES];
        oversized.push(b'\n');
        let error =
            read_bounded_v5_request_frame(&mut BufReader::new(Cursor::new(oversized))).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);

        let error =
            read_bounded_v5_request_frame(&mut BufReader::new(Cursor::new(b"\n"))).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);

        let error =
            read_bounded_v5_request_frame(&mut BufReader::new(Cursor::new(b"{\"kind\":\"ping\"}")))
                .unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::UnexpectedEof);
    }

    #[test]
    fn bounded_v5_read_and_decode_returns_raw_frame_and_typed_kind() {
        let expected = b"{\"kind\":\"ping\"}";
        let mut wire = expected.to_vec();
        wire.push(b'\n');
        let decoded = read_and_decode_v5_request(&mut BufReader::new(Cursor::new(wire))).unwrap();

        assert_eq!(decoded.raw_frame(), expected);
        assert_eq!(decoded.request().kind(), V5ClientRequestKind::Ping);
    }

    #[test]
    fn bounded_v5_reader_retries_an_interrupted_buffer_fill() {
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
                    return Err(io::Error::from(io::ErrorKind::Interrupted));
                }
                self.inner.fill_buf()
            }

            fn consume(&mut self, amount: usize) {
                self.inner.consume(amount);
            }
        }

        let mut reader = InterruptedOnce {
            inner: BufReader::new(Cursor::new(b"{\"kind\":\"ping\"}\n")),
            interrupted: false,
        };

        assert_eq!(
            read_bounded_v5_request_frame(&mut reader).unwrap(),
            b"{\"kind\":\"ping\"}"
        );
    }

    #[test]
    fn strict_v5_client_decoder_round_trips_every_closed_request_kind() {
        for (expected_kind, frame) in valid_request_frames() {
            let request = decode_v5_client_request(frame.as_bytes()).unwrap();

            assert_eq!(request.kind(), expected_kind);
            assert_eq!(serde_json::to_vec(&request).unwrap(), frame.as_bytes());
        }
    }

    #[test]
    fn v5_hello_probe_preserves_a_predecessor_version_for_protocol_mismatch_dispatch() {
        let frame = format!(
            "{{\"kind\":\"hello\",\"protocolVersion\":3,\"token\":\"33333333-3333-4333-8333-333333333333\",\"coreIdentity\":\"{CORE_IDENTITY}\",\"ownerLease\":\"44444444-4444-4444-8444-444444444444\"}}"
        );

        let request = decode_v5_client_request(frame.as_bytes()).unwrap();

        assert_eq!(request.kind(), V5ClientRequestKind::Hello);
        assert_eq!(request.hello_protocol_version(), Some(3));
    }

    #[test]
    fn strict_v5_client_decoder_rejects_unknown_missing_cross_variant_and_invalid_values() {
        let invalid = [
            "{\"kind\":\"ping\",\"unexpected\":true}".to_string(),
            "{\"kind\":\"get_task\"}".to_string(),
            format!("{{\"kind\":\"get_task\",\"taskId\":\"{TASK_ID}\",\"waitMs\":1}}"),
            format!(
                "{{\"kind\":\"wait_task\",\"taskId\":\"{TASK_ID}\",\"waitMs\":7001}}"
            ),
            format!(
                "{{\"kind\":\"submit_invocation\",\"invocation\":{{\"invocationId\":\"not-a-uuid\",\"reservedTaskId\":\"{TASK_ID}\",\"tool\":\"unica.view\",\"arguments\":{{}},\"workspaceHint\":\"workspace-a\",\"responseBudgetMs\":1}}}}"
            ),
            format!(
                "{{\"kind\":\"submit_invocation\",\"invocation\":{{\"invocationId\":\"{INVOCATION_ID}\",\"reservedTaskId\":\"{TASK_ID}\",\"tool\":\"unica.unknown\",\"arguments\":{{}},\"workspaceHint\":\"workspace-a\",\"responseBudgetMs\":1}}}}"
            ),
            format!(
                "{{\"kind\":\"submit_invocation\",\"invocation\":{{\"invocationId\":\"{INVOCATION_ID}\",\"reservedTaskId\":\"{TASK_ID}\",\"tool\":\"unica.view\",\"arguments\":{{}},\"workspaceHint\":\"\",\"responseBudgetMs\":1}}}}"
            ),
            format!(
                "{{\"kind\":\"submit_invocation\",\"invocation\":{{\"invocationId\":\"{INVOCATION_ID}\",\"reservedTaskId\":\"{TASK_ID}\",\"tool\":\"unica.view\",\"arguments\":{{}},\"workspaceHint\":\"workspace\\u0000a\",\"responseBudgetMs\":1}}}}"
            ),
            format!(
                "{{\"kind\":\"submit_invocation\",\"invocation\":{{\"invocationId\":\"{INVOCATION_ID}\",\"reservedTaskId\":\"{TASK_ID}\",\"tool\":\"unica.view\",\"arguments\":{{}},\"workspaceHint\":\"workspace-a\",\"responseBudgetMs\":7001}}}}"
            ),
            format!(
                "{{\"kind\":\"recover_invocation_receipt\",\"receiptKey\":{{\"invocationId\":\"{INVOCATION_ID}\"}}}}"
            ),
            format!(
                "{{\"kind\":\"acknowledge_invocation_receipt\",\"receiptKey\":{},\"terminalDigest\":\"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA\"}}",
                receipt_key_json()
            ),
        ];

        for frame in invalid {
            assert!(
                decode_v5_client_request(frame.as_bytes()).is_err(),
                "invalid v5 request was accepted: {frame}"
            );
        }

        let oversized_workspace = format!(
            "{{\"kind\":\"submit_invocation\",\"invocation\":{{\"invocationId\":\"{INVOCATION_ID}\",\"reservedTaskId\":\"{TASK_ID}\",\"tool\":\"unica.view\",\"arguments\":{{}},\"workspaceHint\":\"{}\",\"responseBudgetMs\":1}}}}",
            "x".repeat(MAX_V5_REQUEST_LINE_BYTES)
        );
        assert!(decode_v5_client_request(oversized_workspace.as_bytes()).is_err());
    }

    #[test]
    fn strict_v5_probe_response_accepts_only_pong_and_closed_error_codes() {
        let pong = b"{\"kind\":\"pong\"}";
        let response = decode_v5_probe_response(pong).unwrap();
        assert_eq!(response.kind(), V5ProbeResponseKind::Pong);
        assert_eq!(serde_json::to_vec(&response).unwrap(), pong);

        for code in V5DaemonErrorCode::ALL {
            let frame = format!("{{\"kind\":\"error\",\"code\":\"{code}\"}}");
            let response = decode_v5_probe_response(frame.as_bytes()).unwrap();
            assert_eq!(response.kind(), V5ProbeResponseKind::Error);
            assert_eq!(response.error_code(), Some(code));
            assert_eq!(serde_json::to_vec(&response).unwrap(), frame.as_bytes());
        }

        for frame in [
            "{\"kind\":\"pong\",\"unexpected\":true}",
            "{\"kind\":\"error\"}",
            "{\"kind\":\"error\",\"code\":\"workspace_capacity\"}",
            "{\"kind\":\"task\"}",
        ] {
            assert!(decode_v5_probe_response(frame.as_bytes()).is_err());
        }
    }
}
