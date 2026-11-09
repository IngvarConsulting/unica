//! Typed provider bindings; service lanes and source leases stay with the service owner.
use std::path::Path;

use std::thread;

use unica_process::mcp_session::PersistentMcpSession;
use unica_process::transport_failure::PrivateFailureSnapshot;
use unica_source_contract::workspace::WorkspaceContext;
use unica_work_control::cancellation::CancellationToken;

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
