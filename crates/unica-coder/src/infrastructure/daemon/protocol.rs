//! The canonical v0.13 request the daemon's domain service takes. The wire
//! that carries it is protocol v5 (`protocol_v5.rs`); protocol v3 retired
//! with its own client, loop and stores.
use crate::application::invocation_store::ToolIdentity;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

pub(crate) const MAX_TASK_WAIT_MS: u64 = 7_000;

/// One canonical v0.13 call submitted to the daemon. Raw arguments exist only
/// on this authenticated live connection; durable state receives their digest.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct InvocationRequest {
    tool: ToolIdentity,
    arguments: Map<String, Value>,
    workspace_hint: String,
    response_budget_ms: u64,
}

impl std::fmt::Debug for InvocationRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("InvocationRequest")
            .field("tool", &self.tool)
            .field("arguments", &"<redacted>")
            .field("workspace_hint", &"<redacted>")
            .field("response_budget_ms", &self.response_budget_ms)
            .finish()
    }
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
