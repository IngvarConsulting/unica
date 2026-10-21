//! Typed provider bindings; service lanes and source leases stay with the service owner.
use crate::protocol::{IndexMaintenanceStatus, ServiceResponse, WorkspaceRlmOperation};
use std::path::Path;
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use unica_process::mcp_session::{McpToolOutput, PersistentMcpSession};
use unica_process::transport_failure::PrivateFailureSnapshot;
use unica_source_contract::workspace::WorkspaceContext;
use unica_work_control::cancellation::CancellationToken;
use unica_workspace::actor::ProviderSourceLease;
use unica_workspace::policies::WorkspacePolicies;

pub trait ServiceEnvironment: Send + Sync {
    fn core_identity(&self) -> &str;
    fn verify_executable_build(&self, executable: &Path) -> Result<(), String>;
    fn find_plugin_root(&self, workspace: &Path) -> Option<std::path::PathBuf>;
    fn workspace_policies(&self) -> WorkspacePolicies;
}

pub trait BslBackend: Send + Sync + 'static {
    fn start(
        &self,
        context: &WorkspaceContext,
        source_root: &Path,
        cancellation: &CancellationToken,
    ) -> Result<PersistentMcpSession, String>;
    fn report_failure(&self, snapshot: PrivateFailureSnapshot);
}

pub trait BackgroundTaskTracker: Send + Sync {
    fn track(&self, handle: thread::JoinHandle<()>);
}

pub trait RlmSession: Send + 'static {
    fn is_reusable(&mut self) -> bool;
    fn build_id(&self) -> &str;
    fn execute(
        &mut self,
        operation: &WorkspaceRlmOperation,
        timeout: Duration,
        cancellation: &CancellationToken,
    ) -> Result<McpToolOutput, String>;
}

pub trait ReadyScope: Send + 'static {
    fn build_id(&self) -> &str;
    fn directory(&self) -> &Path;
}

pub struct RlmObservation<S> {
    pub active: Option<S>,
    pub maintenance: Option<IndexMaintenanceStatus>,
    pub warnings: Vec<String>,
    pub keep_invalidated: bool,
}

pub trait RlmIndex {
    type Scope: ReadyScope;
    fn active_build(
        &self,
        context: &WorkspaceContext,
        source_root: &Path,
    ) -> Result<Option<Self::Scope>, String>;
    fn observe_maintenance(
        &self,
        context: &WorkspaceContext,
        source_root: &Path,
        active: Option<Self::Scope>,
        changed: bool,
        maintenance_cancellation: &CancellationToken,
    ) -> Result<RlmObservation<Self::Scope>, String>;
    fn readiness_response(
        &self,
        observation: &RlmObservation<Self::Scope>,
        executing: bool,
    ) -> ServiceResponse;
}

pub trait RlmBackend: Send + Sync + 'static {
    type Session: RlmSession;
    type Index: RlmIndex;
    fn index(
        &self,
        lease: ProviderSourceLease,
        tracker: Arc<dyn BackgroundTaskTracker>,
    ) -> Self::Index;
    fn start(
        &self,
        context: &WorkspaceContext,
        source_root: &Path,
        build_directory: &Path,
        build_id: &str,
        cancellation: &CancellationToken,
    ) -> Result<Self::Session, String>;
}
