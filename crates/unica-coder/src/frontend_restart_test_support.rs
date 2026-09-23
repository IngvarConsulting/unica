//! Feature-only process fixture. Only the canonical provider is replaced; the
//! real daemon owns admission, durable receipt/Task transitions and recovery.
use crate::application::operation_descriptors::{ExecutionClass, KnownLongReason};
use crate::domain::cancellation::CancellationToken;
use crate::domain::invocation::{DomainResult, InvocationFailure};
use crate::infrastructure::daemon::identity::CoreIdentity;
use crate::infrastructure::daemon::runtime_v5;
use crate::infrastructure::daemon::server::{
    ActorBoundExecution, ActorBoundInvocation, CanonicalInvocationService, DaemonServerConfig,
};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

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

pub fn run_daemon(state_root: &Path, control: &Path) -> Result<(), String> {
    let config = DaemonServerConfig::new(
        state_root.to_owned(),
        CoreIdentity::production_v5(),
        Duration::from_secs(20),
    )
    .with_invocation_service(Arc::new(BlockingProvider {
        control: control.to_owned(),
    }));
    runtime_v5::run_daemon(config)
}
