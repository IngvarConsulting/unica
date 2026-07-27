use std::{path::PathBuf, sync::Arc, time::Duration};

use unica_format_core::{
    ports::{
        AuthorabilityPort, CompatibilityPort, OperationCancellation, PublicationHostPort,
        PublicationLockResult, PublicationPort, PublicationProcessCommand,
        PublicationProcessOutput, ResolvedPublicationTool, ValidationContextPort,
    },
    source::{SourceContext, SourceFamily, SourceLocation},
};

fn assert_port<T: ?Sized + Send + Sync>() {}

#[test]
fn task7_operational_boundaries_are_format_neutral_ports() {
    assert_port::<dyn CompatibilityPort>();
    assert_port::<dyn AuthorabilityPort>();
    assert_port::<dyn ValidationContextPort>();
    assert_port::<dyn PublicationPort>();
    assert_port::<dyn PublicationHostPort>();
}

#[test]
fn task7_cancellation_is_shared_without_a_host_domain_type() {
    let first = OperationCancellation::new();
    let second = first.clone();

    assert!(!second.is_cancelled());
    first.cancel();
    assert!(second.is_cancelled());
}

#[test]
fn task7_source_paths_are_inert_request_data_not_public_navigation_data() {
    let source = SourceContext::new(
        SourceLocation::new(
            PathBuf::from("/private/workspace"),
            PathBuf::from("/private/workspace/source"),
            PathBuf::from("/private/workspace/source/Object.native"),
        ),
        Some("alternate".to_string()),
        SourceFamily::Edt,
        None,
    );

    assert_eq!(
        source.location().target(),
        PathBuf::from("/private/workspace/source/Object.native")
    );
    assert!(unica_format_core::navigation::ObjectKey::new(
        source.location().target().display().to_string()
    )
    .is_err());
}

struct FakeHost;

impl PublicationHostPort for FakeHost {
    fn run_process(
        &self,
        command: &PublicationProcessCommand,
    ) -> Result<PublicationProcessOutput, String> {
        assert_eq!(command.timeout, Some(Duration::from_secs(1)));
        Ok(PublicationProcessOutput {
            status_success: true,
            status: "0".to_string(),
            stdout: "ok".to_string(),
            stderr: String::new(),
            timed_out: false,
            cancelled: false,
            stdout_truncated: false,
        })
    }

    fn resolve_bundled_tool(
        &self,
        _cwd: &std::path::Path,
        _tool: &str,
        _require_executable: bool,
    ) -> Result<ResolvedPublicationTool, String> {
        Ok(ResolvedPublicationTool {
            program: PathBuf::from("/private/tool"),
            warnings: Vec::new(),
        })
    }

    fn with_exclusive_publication_lock(
        &self,
        _targets: &[PathBuf],
        action: &mut dyn FnMut() -> Result<Vec<String>, String>,
    ) -> Result<PublicationLockResult, String> {
        Ok(PublicationLockResult::Action(action()))
    }

    fn redact(&self, text: &str) -> String {
        text.replace("secret", "<redacted>")
    }
}

#[test]
fn task7_publication_host_is_injectable_without_native_format_types() {
    let host: Arc<dyn PublicationHostPort> = Arc::new(FakeHost);
    let output = host
        .run_process(&PublicationProcessCommand {
            program: PathBuf::from("/private/tool"),
            args: vec!["run".to_string()],
            cwd: PathBuf::from("/private/workspace"),
            timeout: Some(Duration::from_secs(1)),
            cancellation: OperationCancellation::new(),
        })
        .unwrap();

    assert!(output.status_success);
    assert_eq!(host.redact("token=secret"), "token=<redacted>");
}
