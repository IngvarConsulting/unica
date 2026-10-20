//! Finite causal observations captured by the owned stdio transport.
use std::io;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FailurePhase {
    WriteRequest,
    WaitResponse,
    DecodeToolResult,
}

impl FailurePhase {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::WriteRequest => "write_request",
            Self::WaitResponse => "wait_response",
            Self::DecodeToolResult => "decode_tool_result",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SafeIoKind {
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
    pub fn as_str(self) -> &'static str {
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
pub enum FailureCause {
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
    pub fn name(self) -> &'static str {
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

    pub fn io_kind(self) -> Option<&'static str> {
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
pub struct TransportFailure {
    pub cause: FailureCause,
    pub message: String,
}

impl TransportFailure {
    pub fn new(cause: FailureCause, message: String) -> Self {
        Self { cause, message }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PrivateFailureSnapshot {
    pub phase: FailurePhase,
    pub cause: FailureCause,
    pub service_pid: u32,
    pub helper_pid: u32,
    pub request_id: i64,
}
