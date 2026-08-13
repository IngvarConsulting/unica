use super::ports::{ApplicationPorts, HandlerOutcome};
use super::AdapterOutcome;
use crate::domain::cancellation::CancellationToken;
use crate::domain::code_intelligence::ProviderDeadline;
use crate::domain::project_health::{evaluate_project_health, ProjectHealthInspectionError};
use crate::domain::workspace::WorkspaceContext;

pub(crate) fn invoke(
    ports: &dyn ApplicationPorts,
    context: &WorkspaceContext,
    cancellation: &CancellationToken,
    deadline: ProviderDeadline,
) -> HandlerOutcome {
    let snapshot = match ports.inspect_project_health(context, cancellation, deadline) {
        Ok(snapshot) => snapshot,
        Err(ProjectHealthInspectionError::Cancelled) => {
            return cancelled();
        }
        Err(ProjectHealthInspectionError::Fatal(reason)) => return failed(reason),
    };
    if cancellation.is_cancelled() {
        return cancelled();
    }
    let report = match evaluate_project_health(snapshot) {
        Ok(report) => report,
        Err(reason) => return failed(reason),
    };
    if cancellation.is_cancelled() {
        return cancelled();
    }
    let mut outcome = AdapterOutcome::ok(format!(
        "project health inspected: ready={}; repositoryReady={}",
        report.ready, report.repository_ready
    ));
    outcome.artifacts.push(report.workspace_root.clone());
    outcome.artifacts.push(report.cache_root.clone());
    let result = HandlerOutcome::with_data(
        outcome,
        serde_json::to_value(report).expect("project health report is always serializable"),
    );
    if cancellation.is_cancelled() {
        return cancelled();
    }
    result
}

fn cancelled() -> HandlerOutcome {
    HandlerOutcome::plain(AdapterOutcome::cancelled(
        "project health inspection stopped",
    ))
}

fn failed(reason: String) -> HandlerOutcome {
    HandlerOutcome::plain(AdapterOutcome {
        ok: false,
        summary: "project health inspection failed".into(),
        changes: Vec::new(),
        warnings: Vec::new(),
        errors: vec![format!("project_health_inspection_failed: {reason}")],
        artifacts: Vec::new(),
        stdout: None,
        stderr: None,
        command: None,
    })
}

#[cfg(test)]
mod tests {
    use super::invoke;
    use crate::application::ports::{ApplicationPorts, HandlerOutcome, SupportGuardCheck};
    use crate::application::{InvocationMode, ToolSpec};
    use crate::domain::cache::{CacheAccess, CacheReport};
    use crate::domain::cancellation::CancellationToken;
    use crate::domain::code_intelligence::ProviderDeadline;
    use crate::domain::events::DomainEvent;
    use crate::domain::project_health::{ProjectHealthInspectionError, ProjectHealthSnapshot};
    use crate::domain::workspace::WorkspaceContext;
    use serde_json::{Map, Value};
    use std::path::PathBuf;
    use std::time::Duration;

    enum ResultKind {
        Cancelled,
        CancelledAfterSnapshot,
        InvalidSnapshot,
    }

    struct FakePorts(ResultKind);

    impl ApplicationPorts for FakePorts {
        fn discover_workspace(
            &self,
            requested_cwd: Option<PathBuf>,
        ) -> Result<WorkspaceContext, String> {
            Ok(context(
                requested_cwd.unwrap_or_else(|| PathBuf::from("/workspace")),
            ))
        }

        fn validate_tool_context(
            &self,
            _spec: ToolSpec,
            _args: &Map<String, Value>,
            _mode: InvocationMode,
            _context: &WorkspaceContext,
        ) -> Result<(), String> {
            Ok(())
        }

        fn inspect_project_health(
            &self,
            _context: &WorkspaceContext,
            _cancellation: &CancellationToken,
            _deadline: ProviderDeadline,
        ) -> Result<ProjectHealthSnapshot, ProjectHealthInspectionError> {
            match self.0 {
                ResultKind::Cancelled => Err(ProjectHealthInspectionError::Cancelled),
                ResultKind::CancelledAfterSnapshot | ResultKind::InvalidSnapshot => {
                    if matches!(self.0, ResultKind::CancelledAfterSnapshot) {
                        _cancellation.cancel();
                    }
                    Ok(ProjectHealthSnapshot {
                        workspace_root: "/workspace".into(),
                        cache_root: "/workspace/.build/unica".into(),
                        repository_root: None,
                        source_sets: None,
                        source_targets_complete: true,
                        observations: Vec::new(),
                        facts: Vec::new(),
                    })
                }
            }
        }

        fn evaluate_support_guard(
            &self,
            _spec: ToolSpec,
            _args: &Map<String, Value>,
            _context: &WorkspaceContext,
        ) -> Result<SupportGuardCheck, String> {
            Ok(SupportGuardCheck::Allow)
        }

        fn invoke_handler(
            &self,
            _spec: ToolSpec,
            _args: &Map<String, Value>,
            _context: &WorkspaceContext,
            _mode: InvocationMode,
            _cancellation: &CancellationToken,
        ) -> Result<HandlerOutcome, String> {
            panic!("coordinator test must not use the generic handler")
        }

        fn cache_report(
            &self,
            context: &WorkspaceContext,
            _events: &[DomainEvent],
            _mode: InvocationMode,
            _cache_access: CacheAccess,
        ) -> Result<CacheReport, String> {
            Ok(CacheReport {
                mode: "read".into(),
                root: context.cache_root.display().to_string(),
                workspace_epoch: 1,
                events: Vec::new(),
                invalidated: Vec::new(),
                refreshed: Vec::new(),
                lazy_rebuilt: Vec::new(),
                stale: Vec::new(),
                fresh: Vec::new(),
                publication_warnings: Vec::new(),
            })
        }

        fn notify_invalidation(&self, _context: &WorkspaceContext, _events: &[DomainEvent]) {}
    }

    #[test]
    fn cancellation_is_an_operation_failure_without_typed_data() {
        let outcome = invoke(
            &FakePorts(ResultKind::Cancelled),
            &context(PathBuf::from("/workspace")),
            &CancellationToken::new(),
            ProviderDeadline::from_budget(Duration::from_secs(1)),
        );

        assert!(!outcome.adapter.ok);
        assert!(outcome.data.is_none());
        assert!(outcome.adapter.summary.contains("cancelled"));
    }

    #[test]
    fn cancellation_after_inspection_wins_over_snapshot_publication() {
        let cancellation = CancellationToken::new();
        let outcome = invoke(
            &FakePorts(ResultKind::CancelledAfterSnapshot),
            &context(PathBuf::from("/workspace")),
            &cancellation,
            ProviderDeadline::from_budget(Duration::from_secs(1)),
        );

        assert!(!outcome.adapter.ok);
        assert!(outcome.data.is_none());
        assert!(outcome.adapter.summary.contains("cancelled"));
    }

    #[test]
    fn invalid_snapshot_is_a_stable_operation_failure() {
        let outcome = invoke(
            &FakePorts(ResultKind::InvalidSnapshot),
            &context(PathBuf::from("/workspace")),
            &CancellationToken::new(),
            ProviderDeadline::from_budget(Duration::from_secs(1)),
        );

        assert!(!outcome.adapter.ok);
        assert!(outcome.data.is_none());
        assert!(outcome.adapter.errors[0].starts_with("project_health_inspection_failed:"));
    }

    fn context(root: PathBuf) -> WorkspaceContext {
        WorkspaceContext {
            cwd: root.clone(),
            workspace_root: root.clone(),
            cache_root: root.join(".build/unica"),
            workspace_epoch: 1,
        }
    }
}
