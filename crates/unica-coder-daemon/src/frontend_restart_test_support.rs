//! Feature-only process fixture. Only the canonical provider is replaced; the
//! real daemon owns admission, durable receipt/Task transitions and recovery.
use crate::infrastructure::daemon::server::{
    ActorBoundExecution, ActorBoundInvocation, CanonicalInvocationService, DaemonServerConfig,
};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use unica_daemon::identity::CoreIdentity;
use unica_execution::receipt_runtime::ports::{ExecutionClass, KnownLongReason};
use unica_execution_contract::invocation::{DomainResult, InvocationFailure};
use unica_work_control::cancellation::CancellationToken;

struct BlockingProvider {
    control: PathBuf,
}

impl CanonicalInvocationService for BlockingProvider {
    fn prepare(&self, _: &ActorBoundInvocation) -> Result<ExecutionClass, Box<DomainResult>> {
        Ok(ExecutionClass::KnownLong(KnownLongReason::ExternalProcess))
    }

    fn execute(
        &self,
        _: &ActorBoundExecution,
        cancellation: CancellationToken,
    ) -> Result<DomainResult, InvocationFailure> {
        let mut count = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.control.join("executions"))
            .map_err(|e| InvocationFailure::new("fixture_io", e.to_string()))?;
        writeln!(count, "execute")
            .and_then(|()| count.sync_all())
            .map_err(|e| InvocationFailure::new("fixture_io", e.to_string()))?;
        while !self.control.join("release").exists() {
            if cancellation.is_cancelled() {
                return Err(InvocationFailure::new("cancelled", "fixture cancellation"));
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        Ok(DomainResult::success("provider completed"))
    }
}

/// Serves as the daemon of the built binary whose identity `core_identity` names.
pub fn run_daemon(state_root: &Path, core_identity: &str, control: &Path) -> Result<(), String> {
    use std::str::FromStr;
    let identity = CoreIdentity::install(CoreIdentity::from_str(core_identity)?)?;
    let config = DaemonServerConfig::new(state_root.to_owned(), identity, Duration::from_secs(20))
        .with_invocation_service(Arc::new(BlockingProvider {
            control: control.to_owned(),
        }));
    unica_daemon::transport::run_daemon(config.into_transport_config())
}
