use super::{AdapterOutcome, ToolSpec};
use crate::application::source_navigation::{
    SourceChildrenRequest, SourceChildrenResult, SourceResolveRequest, SourceResolveResult,
};
use crate::application::source_resources::{
    SourceApplyExecution, SourceApplyRequest, SourceReadRequest, SourceResourcesRequest,
};
use crate::domain::cache::{CacheAccess, CacheReport};
use crate::domain::cancellation::CancellationToken;
use crate::domain::code_intelligence::{
    CodeIntelligenceContext, CodeIntelligenceReadRequest, CodeIntelligenceRegistry,
};
use crate::domain::events::DomainEvent;
use crate::domain::source_resources::{
    ResourceManifestPage, SourceReadResult, SourceResourceError,
};
use crate::domain::workspace::WorkspaceContext;
use serde_json::{Map, Value};
use std::path::PathBuf;

pub(crate) struct HandlerOutcome {
    pub(crate) adapter: AdapterOutcome,
    pub(crate) data: Option<Value>,
    pub(crate) job: Option<Value>,
    pub(crate) events: Vec<DomainEvent>,
    pub(crate) projected_events: Vec<DomainEvent>,
    pub(crate) recorded_cache: Option<CacheReport>,
}

impl HandlerOutcome {
    pub(crate) fn plain(adapter: AdapterOutcome) -> Self {
        Self {
            adapter,
            data: None,
            job: None,
            events: Vec::new(),
            projected_events: Vec::new(),
            recorded_cache: None,
        }
    }

    pub(crate) fn with_data(adapter: AdapterOutcome, data: Value) -> Self {
        Self {
            adapter,
            data: Some(data),
            job: None,
            events: Vec::new(),
            projected_events: Vec::new(),
            recorded_cache: None,
        }
    }

    pub(crate) fn with_data_and_events(
        adapter: AdapterOutcome,
        data: Value,
        events: Vec<DomainEvent>,
    ) -> Self {
        Self {
            adapter,
            data: Some(data),
            job: None,
            events,
            projected_events: Vec::new(),
            recorded_cache: None,
        }
    }

    pub(crate) fn with_data_events_and_projection(
        adapter: AdapterOutcome,
        data: Value,
        events: Vec<DomainEvent>,
        projected_events: Vec<DomainEvent>,
    ) -> Self {
        Self {
            adapter,
            data: Some(data),
            job: None,
            events,
            projected_events,
            recorded_cache: None,
        }
    }
}

pub(crate) enum SupportGuardCheck {
    Allow,
    Warn(String),
    Block(AdapterOutcome),
}

pub(crate) enum FormatGuardCheck {
    Allow,
    Warn {
        warning: String,
        diagnostic: Value,
    },
    Block {
        outcome: AdapterOutcome,
        diagnostic: Value,
    },
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

    fn resolve_code_intelligence_context(
        &self,
        _context: &WorkspaceContext,
        _args: &Map<String, Value>,
    ) -> Result<CodeIntelligenceContext, String> {
        Err("code intelligence context resolver is not configured".to_string())
    }

    fn normalize_code_intelligence_read_request(
        &self,
        _request: CodeIntelligenceReadRequest,
        _context: &CodeIntelligenceContext,
    ) -> Result<CodeIntelligenceReadRequest, String> {
        Err("code intelligence path resolver is not configured".to_string())
    }

    fn code_intelligence_registry(&self) -> Result<CodeIntelligenceRegistry, String> {
        Err("code intelligence provider registry is not configured".to_string())
    }

    fn resolve_source_navigation(
        &self,
        _request: SourceResolveRequest,
        _context: &WorkspaceContext,
        _cancellation: &CancellationToken,
    ) -> Result<SourceResolveResult, String> {
        Err("source navigation resolver is not configured".to_string())
    }

    fn children_source_navigation(
        &self,
        _request: SourceChildrenRequest,
        _context: &WorkspaceContext,
        _cancellation: &CancellationToken,
    ) -> Result<SourceChildrenResult, String> {
        Err("source navigation traversal is not configured".to_string())
    }

    fn source_resources(
        &self,
        _request: SourceResourcesRequest,
        _context: &WorkspaceContext,
        _cancellation: &CancellationToken,
    ) -> Result<ResourceManifestPage, SourceResourceError> {
        Err(SourceResourceError::new(
            crate::domain::source_resources::SourceResourceErrorCode::SourceUnavailable,
            "source resource provider is not configured",
        ))
    }

    fn read_source_resource(
        &self,
        _request: SourceReadRequest,
        _context: &WorkspaceContext,
        _cancellation: &CancellationToken,
    ) -> Result<SourceReadResult, SourceResourceError> {
        Err(SourceResourceError::new(
            crate::domain::source_resources::SourceResourceErrorCode::SourceUnavailable,
            "source resource provider is not configured",
        ))
    }

    fn apply_source_resource(
        &self,
        _request: SourceApplyRequest,
        _context: &WorkspaceContext,
        _dry_run: bool,
        _cancellation: &CancellationToken,
    ) -> Result<SourceApplyExecution, SourceResourceError> {
        Err(SourceResourceError::new(
            crate::domain::source_resources::SourceResourceErrorCode::SourceUnavailable,
            "source resource provider is not configured",
        ))
    }

    fn evaluate_format_guard(
        &self,
        _spec: ToolSpec,
        _args: &Map<String, Value>,
        _context: &WorkspaceContext,
    ) -> Result<FormatGuardCheck, String> {
        Ok(FormatGuardCheck::Allow)
    }

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
    use crate::domain::events::{DomainEvent, SourceResourcesReplaced};
    use crate::domain::source_resources::ResourceRole;
    use serde_json::json;

    #[test]
    fn plain_handler_outcome_has_no_typed_data() {
        let outcome = HandlerOutcome::plain(AdapterOutcome::ok("plain"));

        assert_eq!(outcome.data, None);
        assert_eq!(outcome.job, None);
        assert!(outcome.events.is_empty());
        assert!(outcome.projected_events.is_empty());
    }

    #[test]
    fn handler_outcome_preserves_typed_data_separately_from_stdout() {
        let data = json!({"path": "src/Module.bsl", "noOp": false});
        let outcome = HandlerOutcome::with_data(AdapterOutcome::ok("structured"), data.clone());

        assert_eq!(outcome.data, Some(data));
        assert_eq!(outcome.job, None);
        assert!(outcome.events.is_empty());
        assert!(outcome.projected_events.is_empty());
        assert_eq!(outcome.adapter.stdout, None);
    }

    #[test]
    fn handler_outcome_carries_verified_events_separately_from_request_arguments() {
        let event = DomainEvent::source_resources_replaced(SourceResourcesReplaced {
            source_set: "main".to_string(),
            owner: "CommonModule.Shared".to_string(),
            roles: vec![ResourceRole::BslModule],
            preimage_hashes: vec!["sha256:before".to_string()],
            postimage_hashes: vec!["sha256:after".to_string()],
            affected_targets: vec!["CommonModule.Shared.Module".to_string()],
        });

        let outcome = HandlerOutcome::with_data_and_events(
            AdapterOutcome::ok("applied"),
            json!({"noOp": false}),
            vec![event.clone()],
        );

        assert_eq!(outcome.events, vec![event]);
        assert!(outcome.projected_events.is_empty());
    }
}
