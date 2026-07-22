use super::{AdapterOutcome, ToolSpec};
use crate::domain::cache::{CacheAccess, CacheReport};
use crate::domain::cancellation::CancellationToken;
use crate::domain::events::DomainEvent;
use crate::domain::workspace::WorkspaceContext;
use serde_json::{Map, Value};
use std::path::PathBuf;

pub(crate) struct HandlerOutcome {
    pub(crate) adapter: AdapterOutcome,
    pub(crate) data: Option<Value>,
    pub(crate) job: Option<Value>,
}

impl HandlerOutcome {
    pub(crate) fn plain(adapter: AdapterOutcome) -> Self {
        Self {
            adapter,
            data: None,
            job: None,
        }
    }

    pub(crate) fn with_data(adapter: AdapterOutcome, data: Value) -> Self {
        Self {
            adapter,
            data: Some(data),
            job: None,
        }
    }
}

pub(crate) enum SupportGuardCheck {
    Allow,
    Warn(String),
    Block(AdapterOutcome),
}

pub(crate) trait ApplicationPorts: Send + Sync {
    fn discover_workspace(
        &self,
        requested_cwd: Option<PathBuf>,
    ) -> Result<WorkspaceContext, String>;

    fn validate_tool_context(
        &self,
        spec: ToolSpec,
        args: &Map<String, Value>,
        dry_run: bool,
        context: &WorkspaceContext,
    ) -> Result<(), String>;

    fn evaluate_support_guard(
        &self,
        spec: ToolSpec,
        args: &Map<String, Value>,
        context: &WorkspaceContext,
    ) -> Result<SupportGuardCheck, String>;

    fn invoke_handler(
        &self,
        spec: ToolSpec,
        args: &Map<String, Value>,
        context: &WorkspaceContext,
        dry_run: bool,
        cancellation: &CancellationToken,
    ) -> Result<HandlerOutcome, String>;

    fn cache_report(
        &self,
        context: &WorkspaceContext,
        events: &[DomainEvent],
        dry_run: bool,
        cache_access: CacheAccess,
    ) -> Result<CacheReport, String>;

    fn notify_invalidation(&self, context: &WorkspaceContext, events: &[DomainEvent]);
}

#[cfg(test)]
mod tests {
    use super::HandlerOutcome;
    use crate::application::AdapterOutcome;
    use serde_json::json;

    #[test]
    fn plain_handler_outcome_has_no_typed_data() {
        let outcome = HandlerOutcome::plain(AdapterOutcome::ok("plain"));

        assert_eq!(outcome.data, None);
        assert_eq!(outcome.job, None);
    }

    #[test]
    fn handler_outcome_preserves_typed_data_separately_from_stdout() {
        let data = json!({"path": "src/Module.bsl", "noOp": false});
        let outcome = HandlerOutcome::with_data(AdapterOutcome::ok("structured"), data.clone());

        assert_eq!(outcome.data, Some(data));
        assert_eq!(outcome.job, None);
        assert_eq!(outcome.adapter.stdout, None);
    }
}
