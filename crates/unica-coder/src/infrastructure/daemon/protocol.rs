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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    workspace_origin: Option<unica_bootstrap::WorkspaceOrigin>,
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
            workspace_origin: None,
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

    /// Removes spellings that mean an omitted argument, before validation,
    /// root routing and cursor identity are established. Check keeps its
    /// earlier contract that `null` options equal omitted ones. An empty
    /// `cursor` is the untouched field of a form client and means the first
    /// page (#1216) only when the selected tool mode publishes `cursor`.
    pub(super) fn normalize_omitted_options(&mut self) {
        let publishes_cursor = crate::application::v13::tool_catalog::catalog_for(
            crate::application::tool_contracts::SurfaceRelease::V13,
        )
        .and_then(|catalog| {
            catalog
                .tools
                .into_iter()
                .find(|contract| contract.name == self.tool.catalog_name())
        })
        .is_some_and(|contract| contract.input_schema["properties"].get("cursor").is_some())
            && (self.tool != ToolIdentity::Diff || self.arguments.contains_key("executionToken"));
        let check = self.tool == ToolIdentity::Check;
        self.arguments.retain(|name, value| match name.as_str() {
            "cursor" if publishes_cursor && value.as_str() == Some("") => false,
            "at" | "limit" | "cursor" if check => !value.is_null(),
            _ => true,
        });
    }

    /// The host channel that chose the workspace hint, when the wire carried it.
    pub(crate) fn with_workspace_origin(
        mut self,
        origin: unica_bootstrap::WorkspaceOrigin,
    ) -> Self {
        self.workspace_origin = Some(origin);
        self
    }

    pub(crate) fn workspace_origin(&self) -> Option<&unica_bootstrap::WorkspaceOrigin> {
        self.workspace_origin.as_ref()
    }

    pub(crate) fn workspace_hint(&self) -> &str {
        &self.workspace_hint
    }

    #[cfg(test)]
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

#[cfg(test)]
mod tests {
    use super::{InvocationRequest, ToolIdentity};
    use serde_json::json;

    #[test]
    fn omitted_cursor_normalization_respects_the_selected_diff_contract() {
        for (tool, arguments, expected) in [
            (
                ToolIdentity::Diff,
                json!({"left": "main:Catalog.Items", "right": "main:Catalog.Other", "cursor": ""}),
                json!({"left": "main:Catalog.Items", "right": "main:Catalog.Other", "cursor": ""}),
            ),
            (
                ToolIdentity::Diff,
                json!({"executionToken": "saved-plan", "cursor": ""}),
                json!({"executionToken": "saved-plan"}),
            ),
            (
                ToolIdentity::Diff,
                json!({"executionToken": null, "cursor": ""}),
                json!({"executionToken": null}),
            ),
            (
                ToolIdentity::Diff,
                json!({"executionToken": "", "cursor": ""}),
                json!({"executionToken": ""}),
            ),
            (
                ToolIdentity::Diff,
                json!({"executionToken": "saved-plan", "cursor": null}),
                json!({"executionToken": "saved-plan", "cursor": null}),
            ),
            (
                ToolIdentity::Check,
                json!({"at": null, "limit": null, "cursor": null}),
                json!({}),
            ),
        ] {
            let mut request =
                InvocationRequest::new(tool, arguments.clone(), "workspace", 7_000).unwrap();
            request.normalize_omitted_options();
            assert_eq!(
                request.arguments(),
                expected.as_object().unwrap(),
                "normalization must retain invalid arguments for refusal: {arguments}"
            );
        }
    }
}
