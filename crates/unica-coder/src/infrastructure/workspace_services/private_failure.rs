use serde::Serialize;
use std::io::{self, Write};

pub(super) const PREFIX: &str = "UNICA_PRIVATE_FAILURE_V1 ";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum FailurePhase {
    WriteRequest,
    WaitResponse,
    DecodeToolResult,
}

impl FailurePhase {
    fn as_str(self) -> &'static str {
        match self {
            Self::WriteRequest => "write_request",
            Self::WaitResponse => "wait_response",
            Self::DecodeToolResult => "decode_tool_result",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SafeIoKind {
    BrokenPipe,
    ConnectionReset,
    ConnectionAborted,
    NotConnected,
    TimedOut,
    UnexpectedEof,
    Interrupted,
    WouldBlock,
    PermissionDenied,
    Other,
}

impl From<io::ErrorKind> for SafeIoKind {
    fn from(kind: io::ErrorKind) -> Self {
        match kind {
            io::ErrorKind::BrokenPipe => Self::BrokenPipe,
            io::ErrorKind::ConnectionReset => Self::ConnectionReset,
            io::ErrorKind::ConnectionAborted => Self::ConnectionAborted,
            io::ErrorKind::NotConnected => Self::NotConnected,
            io::ErrorKind::TimedOut => Self::TimedOut,
            io::ErrorKind::UnexpectedEof => Self::UnexpectedEof,
            io::ErrorKind::Interrupted => Self::Interrupted,
            io::ErrorKind::WouldBlock => Self::WouldBlock,
            io::ErrorKind::PermissionDenied => Self::PermissionDenied,
            // ErrorKind is non-exhaustive; future/other kinds stay a closed value.
            _ => Self::Other,
        }
    }
}

impl SafeIoKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::BrokenPipe => "broken_pipe",
            Self::ConnectionReset => "connection_reset",
            Self::ConnectionAborted => "connection_aborted",
            Self::NotConnected => "not_connected",
            Self::TimedOut => "timed_out",
            Self::UnexpectedEof => "unexpected_eof",
            Self::Interrupted => "interrupted",
            Self::WouldBlock => "would_block",
            Self::PermissionDenied => "permission_denied",
            Self::Other => "other",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum FailureCause {
    Cancelled,
    DeadlineElapsed,
    WriterUnavailable,
    EncodeRequest,
    WriteIo(SafeIoKind),
    FlushIo(SafeIoKind),
    StdoutEof,
    IncompleteFrame,
    InvalidJson,
    ResponseTooLarge,
    ReadIo(SafeIoKind),
    ReaderDisconnected,
    RemoteRpcError,
    RemoteToolError,
    MissingResult,
}

impl FailureCause {
    fn name(self) -> &'static str {
        match self {
            Self::Cancelled => "cancelled",
            Self::DeadlineElapsed => "deadline_elapsed",
            Self::WriterUnavailable => "writer_unavailable",
            Self::EncodeRequest => "encode_request",
            Self::WriteIo(_) => "write_io",
            Self::FlushIo(_) => "flush_io",
            Self::StdoutEof => "stdout_eof",
            Self::IncompleteFrame => "incomplete_frame",
            Self::InvalidJson => "invalid_json",
            Self::ResponseTooLarge => "response_too_large",
            Self::ReadIo(_) => "read_io",
            Self::ReaderDisconnected => "reader_disconnected",
            Self::RemoteRpcError => "remote_rpc_error",
            Self::RemoteToolError => "remote_tool_error",
            Self::MissingResult => "missing_result",
        }
    }

    fn io_kind(self) -> Option<&'static str> {
        match self {
            Self::WriteIo(kind) | Self::FlushIo(kind) | Self::ReadIo(kind) => Some(kind.as_str()),
            Self::Cancelled
            | Self::DeadlineElapsed
            | Self::WriterUnavailable
            | Self::EncodeRequest
            | Self::StdoutEof
            | Self::IncompleteFrame
            | Self::InvalidJson
            | Self::ResponseTooLarge
            | Self::ReaderDisconnected
            | Self::RemoteRpcError
            | Self::RemoteToolError
            | Self::MissingResult => None,
        }
    }
}

#[derive(Debug)]
pub(super) struct TransportFailure {
    pub(super) cause: FailureCause,
    pub(super) message: String,
}

impl TransportFailure {
    pub(super) fn new(cause: FailureCause, message: String) -> Self {
        Self { cause, message }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct PrivateFailureSnapshot {
    pub(super) phase: FailurePhase,
    pub(super) cause: FailureCause,
    pub(super) service_pid: u32,
    pub(super) helper_pid: u32,
    pub(super) request_id: i64,
}

// Every string is a static literal selected by a typed producer branch.
// Neither the transport error's message nor helper input/output is borrowed here.
#[derive(Serialize)]
struct SafeRecord {
    version: u8,
    provider: &'static str,
    phase: &'static str,
    cause: &'static str,
    io_kind: Option<&'static str>,
    service_pid: u32,
    helper_pid: u32,
    request_id: i64,
}

pub(super) fn write_record_to(
    writer: &mut dyn Write,
    snapshot: PrivateFailureSnapshot,
) -> io::Result<()> {
    let record = SafeRecord {
        version: 1,
        provider: "bsl_analyzer",
        phase: snapshot.phase.as_str(),
        cause: snapshot.cause.name(),
        io_kind: snapshot.cause.io_kind(),
        service_pid: snapshot.service_pid,
        helper_pid: snapshot.helper_pid,
        request_id: snapshot.request_id,
    };
    let encoded = serde_json::to_string(&record).map_err(io::Error::other)?;
    writeln!(writer, "{PREFIX}{encoded}")
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAXIMUM_LINE: &str = concat!(
        "UNICA_PRIVATE_FAILURE_V1 ",
        "{\"version\":1,\"provider\":\"bsl_analyzer\",\"phase\":\"write_request\",",
        "\"cause\":\"write_io\",\"io_kind\":\"connection_aborted\",",
        "\"service_pid\":4294967295,\"helper_pid\":4294967295,",
        "\"request_id\":9223372036854775807}\n"
    );

    #[test]
    fn formatter_maximum_is_derived_from_every_admitted_pair() {
        let kinds = [
            SafeIoKind::BrokenPipe,
            SafeIoKind::ConnectionReset,
            SafeIoKind::ConnectionAborted,
            SafeIoKind::NotConnected,
            SafeIoKind::TimedOut,
            SafeIoKind::UnexpectedEof,
            SafeIoKind::Interrupted,
            SafeIoKind::WouldBlock,
            SafeIoKind::PermissionDenied,
            SafeIoKind::Other,
        ];
        let mut pairs = vec![
            (FailurePhase::WriteRequest, FailureCause::Cancelled),
            (FailurePhase::WriteRequest, FailureCause::DeadlineElapsed),
            (FailurePhase::WriteRequest, FailureCause::WriterUnavailable),
            (FailurePhase::WriteRequest, FailureCause::EncodeRequest),
            (FailurePhase::WaitResponse, FailureCause::Cancelled),
            (FailurePhase::WaitResponse, FailureCause::DeadlineElapsed),
            (FailurePhase::WaitResponse, FailureCause::StdoutEof),
            (FailurePhase::WaitResponse, FailureCause::IncompleteFrame),
            (FailurePhase::WaitResponse, FailureCause::InvalidJson),
            (FailurePhase::WaitResponse, FailureCause::ResponseTooLarge),
            (FailurePhase::WaitResponse, FailureCause::ReaderDisconnected),
            (FailurePhase::DecodeToolResult, FailureCause::Cancelled),
            (FailurePhase::DecodeToolResult, FailureCause::RemoteRpcError),
            (
                FailurePhase::DecodeToolResult,
                FailureCause::RemoteToolError,
            ),
            (FailurePhase::DecodeToolResult, FailureCause::MissingResult),
        ];
        for kind in kinds {
            pairs.push((FailurePhase::WriteRequest, FailureCause::WriteIo(kind)));
            pairs.push((FailurePhase::WriteRequest, FailureCause::FlushIo(kind)));
            pairs.push((FailurePhase::WaitResponse, FailureCause::ReadIo(kind)));
        }
        let mut maximum = 0;
        for (phase, cause) in pairs {
            let mut bytes = Vec::new();
            write_record_to(
                &mut bytes,
                PrivateFailureSnapshot {
                    phase,
                    cause,
                    service_pid: u32::MAX,
                    helper_pid: u32::MAX,
                    request_id: i64::MAX,
                },
            )
            .unwrap();
            maximum = maximum.max(bytes.len());
            assert!(bytes.len() <= MAXIMUM_LINE.len());
            assert!(!String::from_utf8(bytes).unwrap().contains("PRIVATE_SECRET"));
        }
        assert_eq!(maximum, MAXIMUM_LINE.len());
        let mut actual = Vec::new();
        write_record_to(
            &mut actual,
            PrivateFailureSnapshot {
                phase: FailurePhase::WriteRequest,
                cause: FailureCause::WriteIo(SafeIoKind::ConnectionAborted),
                service_pid: u32::MAX,
                helper_pid: u32::MAX,
                request_id: i64::MAX,
            },
        )
        .unwrap();
        assert_eq!(actual, MAXIMUM_LINE.as_bytes());
        println!("PRIVATE_FAILURE_MAX_RECORD_BYTES={maximum}");
    }
}
