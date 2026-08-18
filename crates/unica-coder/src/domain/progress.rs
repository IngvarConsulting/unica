//! Transport-neutral progress seam.
//!
//! A long call publishes events; the transport forwards them verbatim. The
//! domain that produces an event names its own meta key and owns the payload
//! shape, so adding a producer does not touch the MCP layer.

use serde_json::Value;

/// One published progress observation.
#[derive(Debug, Clone, PartialEq)]
pub struct ProgressEvent {
    /// Meta key carrying `payload` in `notifications/progress`.
    pub meta_key: &'static str,
    /// Domain-shaped payload. The transport never inspects it.
    pub payload: Value,
    /// Completed units. Never a percentage.
    pub progress: f64,
    /// Total units the producer expects.
    pub total: f64,
    /// Human-readable line for hosts that render one.
    pub message: String,
}

pub trait ProgressSink: Send + Sync {
    fn publish(&self, event: ProgressEvent);
}

#[derive(Debug, Default)]
pub struct NoopProgressSink;

impl ProgressSink for NoopProgressSink {
    fn publish(&self, _event: ProgressEvent) {}
}
