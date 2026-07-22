use super::{AdapterOutcome, ToolSpec};
use crate::application::discovery::contract::DiscoverRequest;
use crate::domain::cache::{CacheAccess, CacheReport};
use crate::domain::cancellation::CancellationToken;
use crate::domain::discovery::{DiscoveryError, DiscoveryReport};
use crate::domain::events::DomainEvent;
use crate::domain::workspace::WorkspaceContext;
use serde_json::{Map, Value};
use std::path::PathBuf;

pub(crate) struct HandlerOutcome {
    pub(crate) adapter: AdapterOutcome,
    pub(crate) job: Option<Value>,
}

impl HandlerOutcome {
    pub(crate) fn plain(adapter: AdapterOutcome) -> Self {
        Self { adapter, job: None }
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

    fn discover_extension_points(
        &self,
        request: &DiscoverRequest,
        context: &WorkspaceContext,
        cancellation: &CancellationToken,
    ) -> Result<DiscoveryReport, DiscoveryError>;

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
