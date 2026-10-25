//! Owned process trees, bounded streams, handoff and cleanup.

mod platform;

pub use platform::process::{
    detach_std_handles_from_inheritance, ensure_truncation_diagnostics, ManagedChild,
    ManagedCommand, ManagedLineOutput, ManagedOutput, ManagedSpawnFailure, ManagedStartupChild,
    PendingProcessHandoff, PendingRunFailure, ProcessLineReply, RuntimeProcessTreeHandle,
    RuntimeProcessTreeState, StreamControl, STDERR_CAPTURE_LIMIT, STDOUT_CAPTURE_LIMIT,
    STREAM_LINE_TOO_LONG_ERROR, STREAM_RECORD_UNTERMINATED_ERROR,
};
pub use platform::process_metrics::process_peak_rss_bytes;
#[cfg(any(test, feature = "test-support"))]
pub use platform::test_support;

pub mod runner;

pub mod arguments;

pub mod runtime_job;

pub mod mcp_session;

pub mod transport_failure;

pub use platform::log_stream::ProcessLog;
