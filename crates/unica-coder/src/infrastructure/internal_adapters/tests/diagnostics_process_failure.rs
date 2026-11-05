use super::*;

const CHILD_MARKER: &str = "UNICA_958_DRAIN_COMPLETED";

struct ContextCleanup(WorkspaceContext);

impl Drop for ContextCleanup {
    fn drop(&mut self) {
        cleanup_context(&self.0);
    }
}

struct FloodRunner {
    marker: PathBuf,
}

impl ProcessRunner for FloodRunner {
    fn run(&self, _: &ProcessCommand) -> Result<ProcessOutput, String> {
        panic!("the adapter must consume a live stream")
    }

    fn run_streaming_staged(
        &self,
        command: &ProcessCommand,
        max_line_bytes: usize,
        on_line: &mut dyn FnMut(usize, &[u8]) -> StreamControl,
    ) -> Result<ProcessStreamOutput, ProcessStreamFailure> {
        let mut child = command.clone();
        child.program = std::env::current_exe().unwrap();
        child.args = vec![
            "--exact".to_string(),
            concat!(
                module_path!(),
                "::invalid_jsonl_does_not_stop_draining_owned_process_pipes"
            )
            .split_once("::")
            .unwrap()
            .1
            .to_string(),
            "--nocapture".to_string(),
        ];
        child
            .env
            .push((CHILD_MARKER.into(), self.marker.clone().into_os_string()));
        // A deadlock guard for this deliberately oversized test producer.
        child.timeout = Some(Duration::from_secs(15));
        system_process_runner().run_streaming_staged(&child, max_line_bytes, on_line)
    }
}

#[test]
fn invalid_jsonl_does_not_stop_draining_owned_process_pipes() {
    if let Some(marker) = std::env::var_os(CHILD_MARKER) {
        let mut stdout = std::io::stdout().lock();
        let mut stderr = std::io::stderr().lock();
        stdout
            .write_all(PARTIAL_FINDING_THEN_INVALID.as_bytes())
            .unwrap();
        // Four MiB per pipe exceeds the pipe buffer; the child cannot reach
        // its marker unless both pipes continue to be consumed after rejection.
        let line = vec![b'x'; 1023];
        for _ in 0..4096 {
            stdout.write_all(&line).unwrap();
            stdout.write_all(b"\n").unwrap();
            stderr.write_all(&line).unwrap();
            stderr.write_all(b"\n").unwrap();
        }
        stdout.flush().unwrap();
        stderr.flush().unwrap();
        fs::write(marker, b"both-streams-written").unwrap();
        std::process::exit(23);
    }

    let context = ContextCleanup(temp_context("958-real-stream-draining"));
    let marker = context.0.workspace_root.join("drain-completed");
    assert!(!marker.exists());
    let runner = FloodRunner {
        marker: marker.clone(),
    };
    let result = invoke_analyze(
        &BslAnalyzerMcpAdapter::with_process_runner(
            &runner,
            crate::infrastructure::analysis_composition::environment(),
        ),
        &Map::new(),
        &context.0,
        false,
    );

    assert!(
        marker.exists(),
        "child did not finish writing: {:?}: {}",
        result.run_failure,
        result.outcome.summary
    );
    assert_eq!(fs::read(&marker).unwrap(), b"both-streams-written");
    assert!(!result.outcome.ok);
    assert_eq!(result.run_failure, Some(AnalyzerRunFailureKind::Exited));
    assert!(result
        .outcome
        .warnings
        .iter()
        .any(|warning| warning.contains("23")));
    assert!(result.diagnostics.is_none());
    assert!(result.data.is_none());
    assert!(result.outcome.stdout.is_none());
    assert!(!result.outcome.errors.iter().any(|error| {
        error.contains("diagnostics_invalid")
            || error.contains("partial-finding-sentinel")
            || error.contains("invalid-jsonl-sentinel")
    }));
}
