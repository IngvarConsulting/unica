use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use unica_runner::attempt::RetryStartFailure;

#[derive(Debug, Clone, Copy)]
enum ForeignScope {
    Operation,
    Cwd,
    Target,
    Artifact,
    Authorization,
}

#[derive(Debug, Clone)]
struct PreparedAttempt {
    scope: RuntimeAttemptScope,
    foreign_retry: Option<ForeignScope>,
}
impl RuntimeAttempt for PreparedAttempt {
    fn operation(&self) -> RuntimeJobOperation {
        self.scope.operation
    }
    fn safe_target(&self) -> &str {
        &self.scope.safe_target
    }
    fn artifact_path(&self) -> Option<&str> {
        self.scope.artifact_path.as_deref()
    }
    fn timeout_reason(&self) -> Option<&str> {
        None
    }
    fn reportable_argv(&self) -> Vec<String> {
        vec!["prepared-attempt".to_string()]
    }
    fn validate(&self) -> JobResult<()> {
        Ok(())
    }
    fn take_retry(&mut self) -> Option<Self> {
        let foreign = self.foreign_retry.take()?;
        let mut retry = self.clone();
        match foreign {
            ForeignScope::Operation => retry.scope.operation = RuntimeJobOperation::Dump,
            ForeignScope::Cwd => retry.scope.cwd.push("foreign"),
            ForeignScope::Target => retry.scope.safe_target.push_str("-foreign"),
            ForeignScope::Artifact => retry.scope.artifact_path = Some("foreign-artifact".into()),
            ForeignScope::Authorization => retry.scope.authorization.push(2),
        }
        Some(retry)
    }
    // This owner test accepts a dependency's retry decision deliberately. It
    // checks execution's admission binding, not any backend's receipt parser.
    fn classify_retry(&self, _observed: CompletedAttempt<'_>) -> Result<(), Option<String>> {
        Ok(())
    }
    fn retry_start_warning(&self, _stage: RetryStartFailure, error: &str) -> String {
        error.to_string()
    }
    fn retry_warning(&self) -> &'static str {
        "retry accepted"
    }
    fn combine_outputs(_initial: &RuntimeJobOutput, retry: &RuntimeJobOutput) -> RuntimeJobOutput {
        retry.clone()
    }
}
struct Backend {
    spawns: Arc<AtomicUsize>,
}
impl RuntimeJobRunner for Backend {
    type Request = PreparedAttempt;
    fn scope(&self, request: &PreparedAttempt) -> RuntimeAttemptScope {
        request.scope.clone()
    }
    fn reauthorize(&self, _request: &PreparedAttempt) -> JobResult<()> {
        Ok(())
    }
    fn spawn(
        &self,
        _request: &PreparedAttempt,
    ) -> unica_process::runtime_job::RuntimeJobSpawnResult {
        self.spawns.fetch_add(1, Ordering::SeqCst);
        Ok(Box::new(FinishedProcess))
    }
    fn attach(&self, _id: u32) -> JobResult<Box<dyn RuntimeJobProcess>> {
        Err("unowned".into())
    }
}
struct FinishedProcess;
impl RuntimeJobProcess for FinishedProcess {
    fn id(&self) -> u32 {
        7
    }
    fn try_wait(&mut self) -> JobResult<RuntimeJobProcessState> {
        Ok(RuntimeJobProcessState::Exited { exit_code: 1 })
    }
    fn cancel(&mut self) -> JobResult<()> {
        Ok(())
    }
    fn output_tails(&mut self, _max: usize) -> JobResult<RuntimeJobOutput> {
        Ok(RuntimeJobOutput {
            stdout: String::new(),
            stderr: String::new(),
            output_incomplete: false,
            fallback_receipt: Some(String::new()),
            fallback_receipt_truncated: false,
        })
    }
}

#[test]
fn retry_refuses_every_foreign_prepared_scope_before_spawn() {
    for foreign in [
        ForeignScope::Operation,
        ForeignScope::Cwd,
        ForeignScope::Target,
        ForeignScope::Artifact,
        ForeignScope::Authorization,
    ] {
        let root = tempfile::tempdir().unwrap();
        let owner = create_test_fixture_owner(root.path().to_path_buf());
        bind_test_service_root(owner, root.path().to_path_buf());
        let spawns = Arc::new(AtomicUsize::new(0));
        let service = RuntimeJobService::new(
            root.path(),
            Arc::new(Backend {
                spawns: spawns.clone(),
            }),
        );
        let request = PreparedAttempt {
            scope: RuntimeAttemptScope {
                operation: RuntimeJobOperation::Build,
                cwd: root.path().to_path_buf(),
                safe_target: "workspace:original".into(),
                artifact_path: None,
                authorization: vec![1],
            },
            foreign_retry: Some(foreign),
        };
        let original = service.start(request).unwrap();
        let terminal = service.poll(&original.id).unwrap();
        assert_eq!(terminal.id, original.id);
        assert_eq!(terminal.phase, RuntimeJobPhase::Failed, "{foreign:?}");
        assert_eq!(spawns.load(Ordering::SeqCst), 1, "{foreign:?}");
        assert!(
            terminal
                .warnings
                .iter()
                .any(|warning| warning.contains("prepared scope")),
            "{foreign:?}"
        );
        assert!(!service.store.active_lock_path().exists());
        drop(service);
        drain_test_fixture_owner(owner);
    }
}
