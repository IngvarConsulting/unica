//! RLM index and logical-session implementation of the workspace lifecycle ports.
use crate::environment::RlmEnvironment;
use crate::index::{
    IndexMaintenanceStatusExt, IndexReadiness, IndexRunner, IndexStartReport, SystemIndexRunner,
    UsableIndexBuild, WorkspaceIndexService,
};
use crate::session::{RlmMcpSession, RlmServiceResponse};
use std::path::Path;
use std::sync::Arc;
use unica_analysis_services::backend::{
    BackgroundTaskTracker, ReadyScope, RlmBackend, RlmIndex, RlmObservation,
};
use unica_analysis_services::protocol::{IndexMaintenanceStatus, ServiceResponse};
use unica_source_contract::workspace::WorkspaceContext;
use unica_work_control::cancellation::CancellationToken;
use unica_workspace::actor::ProviderSourceLease;

pub struct RlmWorkspaceBackend {
    environment: Arc<dyn RlmEnvironment>,
}
impl RlmWorkspaceBackend {
    pub fn new(environment: Arc<dyn RlmEnvironment>) -> Self {
        Self { environment }
    }
}
pub struct BoundRlmIndex<'a> {
    runner: IndexRunnerBinding<'a>,
    lease: ProviderSourceLease,
    environment: Arc<dyn RlmEnvironment>,
}
impl RlmBackend for RlmWorkspaceBackend {
    type Session = RlmMcpSession;
    type Index = BoundRlmIndex<'static>;
    fn index(
        &self,
        lease: ProviderSourceLease,
        tracker: Arc<dyn BackgroundTaskTracker>,
    ) -> BoundRlmIndex<'static> {
        BoundRlmIndex {
            runner: IndexRunnerBinding::System(
                SystemIndexRunner::tracked(tracker),
                std::marker::PhantomData,
            ),
            lease,
            environment: Arc::clone(&self.environment),
        }
    }
    fn start(
        &self,
        context: &WorkspaceContext,
        source_root: &Path,
        build_directory: &Path,
        build_id: &str,
        cancellation: &CancellationToken,
    ) -> Result<RlmMcpSession, String> {
        RlmMcpSession::start_pinned(
            context,
            source_root,
            build_directory,
            build_id,
            cancellation,
            self.environment.as_ref(),
        )
    }
}
impl ReadyScope for UsableIndexBuild {
    fn build_id(&self) -> &str {
        &self.id
    }
    fn directory(&self) -> &Path {
        &self.directory
    }
}
impl BoundRlmIndex<'_> {
    fn service(&self) -> WorkspaceIndexService<'_> {
        WorkspaceIndexService::with_runner(self.runner.runner(), self.environment.as_ref())
            .with_source_lease(self.lease.clone())
    }
}
impl RlmIndex for BoundRlmIndex<'_> {
    type Scope = UsableIndexBuild;
    fn active_build(
        &self,
        context: &WorkspaceContext,
        source_root: &Path,
    ) -> Result<Option<UsableIndexBuild>, String> {
        self.service().active_build(context, source_root)
    }
    fn observe_maintenance(
        &self,
        context: &WorkspaceContext,
        source_root: &Path,
        active: Option<UsableIndexBuild>,
        changed: bool,
        maintenance_cancellation: &CancellationToken,
    ) -> Result<RlmObservation<UsableIndexBuild>, String> {
        let service = self.service();
        let report = if active.is_none() || changed {
            Some(service.start_isolated_build(context, source_root, maintenance_cancellation))
        } else {
            service
                .maintenance_status(context, source_root)
                .map(IndexStartReport::observed)
        };
        let maintenance = report.as_ref().map(|report| report.maintenance.clone());
        let warnings = report.map(|report| report.warnings).unwrap_or_default();
        let keep_invalidated = changed
            && matches!(
                maintenance,
                Some(
                    IndexMaintenanceStatus::StartFailed(_)
                        | IndexMaintenanceStatus::Failed(_)
                        | IndexMaintenanceStatus::Cancelled(_)
                        | IndexMaintenanceStatus::Unknown
                )
            );
        let active =
            if active.is_none() && matches!(maintenance, Some(IndexMaintenanceStatus::Completed)) {
                service.active_build(context, source_root)?
            } else {
                active
            };
        Ok(RlmObservation {
            active,
            maintenance,
            warnings,
            keep_invalidated,
        })
    }
    fn readiness_response(
        &self,
        observation: &RlmObservation<UsableIndexBuild>,
        executing: bool,
    ) -> ServiceResponse {
        let readiness = observation
            .active
            .as_ref()
            .map(|build| IndexReadiness::Ready {
                db_path: build.db_path.clone(),
            })
            .unwrap_or_else(|| {
                observation
                    .maintenance
                    .as_ref()
                    .unwrap_or(&IndexMaintenanceStatus::Unknown)
                    .readiness()
            });
        let mut response = if executing {
            ServiceResponse::unavailable_rlm_execution(readiness, observation.warnings.clone())
        } else {
            ServiceResponse::from_readiness(readiness, observation.warnings.clone())
        };
        response.index_maintenance = observation.maintenance.clone();
        if response.index_status.as_deref() == Some("ready") {
            response.index_freshness = Some("unknown".to_string());
            response.index_build_id = observation.active.as_ref().map(|build| build.id.clone());
        }
        response
    }
}

enum IndexRunnerBinding<'a> {
    System(SystemIndexRunner, std::marker::PhantomData<&'a ()>),
    #[cfg(any(test, feature = "test-support"))]
    Injected(&'a dyn IndexRunner),
}
impl IndexRunnerBinding<'_> {
    fn runner(&self) -> &dyn IndexRunner {
        match self {
            Self::System(runner, _) => runner,
            #[cfg(any(test, feature = "test-support"))]
            Self::Injected(runner) => *runner,
        }
    }
}
#[cfg(any(test, feature = "test-support"))]
impl<'a> BoundRlmIndex<'a> {
    pub fn test_with_runner(
        environment: Arc<dyn RlmEnvironment>,
        lease: ProviderSourceLease,
        runner: &'a dyn IndexRunner,
    ) -> Self {
        Self {
            environment,
            lease,
            runner: IndexRunnerBinding::Injected(runner),
        }
    }
}
