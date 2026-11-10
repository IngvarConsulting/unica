use super::*;

/// A workspace with one declared configuration set and an infobase, enough
/// to bind `pull`, `push` and `make` and to admit an actor for an edit.
fn runtime_writer_workspace() -> tempfile::TempDir {
    let workspace = subsystem_picture_workspace("", true);
    std::fs::write(
        workspace.path().join("v8project.yaml"),
        "format: DESIGNER\ninfobase:\n  connection: 'File=base'\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
    )
    .unwrap();
    workspace
}

fn run_request(
    workspace: &std::path::Path,
    op: &str,
    args: serde_json::Value,
    dry_run: bool,
) -> InvocationRequest {
    InvocationRequest::new(
        ToolIdentity::Run,
        serde_json::json!({"op": op, "args": args, "dryRun": dry_run}),
        workspace.display().to_string(),
        7_000,
    )
    .unwrap()
}

fn bound_source_writes(bound: V5ActorBoundCanonicalInvocation) -> (SourceWriteLane, bool) {
    match bound.0 {
        ActorBoundCanonicalInvocationState::SourceImport {
            source_writes,
            import,
            ..
        } => (source_writes, import.is_preview()),
        ActorBoundCanonicalInvocationState::SourceExport {
            source_writes,
            export,
            ..
        } => (source_writes, export.is_preview()),
        ActorBoundCanonicalInvocationState::ArtifactBuild {
            source_writes,
            build,
            ..
        } => (source_writes, build.is_preview()),
        _ => panic!("a runtime writer bound to another state"),
    }
}

/// `pull`, `push` and `make` take the lane that the workspace actor admitted
/// for an edit publishes under: an edit and a runtime writer exclude each
/// other whichever of them came first.
/// rule: INV.RUNTIME.SOURCE-SET-WRITE-EXCLUSION
#[test]
fn runtime_writers_share_the_source_write_lane_of_the_admitted_workspace_actor() {
    let workspace = runtime_writer_workspace();
    let runtime = bootstrap_runtime();
    let writers = [
        (
            "pull",
            serde_json::json!({"sourceSet": "main", "force": true}),
        ),
        ("push", serde_json::json!({"sourceSet": "main"})),
        ("make", serde_json::json!({"output": "dist/main.cf"})),
    ]
    .map(|(op, args)| {
        let bound = runtime
            .bind(run_request(workspace.path(), op, args.clone(), true))
            .unwrap_or_else(|error| panic!("{op} preview did not bind: {error:?}"));
        assert!(
            bound_source_writes(bound).1,
            "{op} preview would take the lane"
        );
        let bound = runtime
            .bind(run_request(workspace.path(), op, args, false))
            .unwrap_or_else(|error| panic!("{op} did not bind: {error:?}"));
        let (lane, preview) = bound_source_writes(bound);
        assert!(!preview, "{op} execution would skip the lane");
        (op, lane)
    });

    let view = InvocationRequest::new(
        ToolIdentity::View,
        serde_json::json!({"at": "main:Subsystem.Sales"}),
        workspace.path().display().to_string(),
        7_000,
    )
    .unwrap();
    let edit = match runtime
        .bind(view)
        .expect("view admits the workspace actor")
        .0
    {
        ActorBoundCanonicalInvocationState::Workspace { invocation, .. } => invocation,
        _ => panic!("view did not bind to the workspace actor"),
    };
    let publication = edit.actor_for_test().hold_mutation_lane_for_test();
    for (op, lane) in &writers {
        let error = lane
            .hold(
                ProviderDeadline::from_budget(Duration::from_millis(40)),
                &CancellationToken::new(),
                || panic!("{op} ran during an actor publication"),
            )
            .unwrap_err();
        assert!(!error.is_cancelled(), "{op}: {error}");
        assert!(lane.same_lane(&writers[0].1), "{op} took another lane");
    }
    drop(publication);
}

/// A preview neither writes nor loads sources and runs at once; an execution
/// waits for the lane and names its operation when cancelled during the wait.
/// rule: INV.RUNTIME.SOURCE-SET-WRITE-EXCLUSION
#[test]
fn runtime_writer_execution_waits_for_the_lane_and_a_preview_does_not() {
    let workspace = runtime_writer_workspace();
    let runtime = bootstrap_runtime();
    let lane = runtime
        .workspace_actors
        .source_write_lane(workspace.path())
        .unwrap();
    let held = lane.hold_for_test();

    let preview = exclude_source_writes(&lane, "pull", true, CancellationToken::new(), |_| {
        DomainResult::canonical_rejection(Some("pull".into()), RefusalCode::BadValue, "ran")
    });
    assert_eq!(preview.diagnostics[0]["message"], "ran");

    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let cancelled = exclude_source_writes(&lane, "push", false, cancellation, |_| {
        panic!("push ran while an actor publication held the lane")
    });
    assert_eq!(cancelled.diagnostics[0]["code"], "cancelled");
    assert!(
        cancelled.summary.contains("push") && cancelled.summary.contains("source edits"),
        "{cancelled:?}"
    );

    drop(held);
    let executed = exclude_source_writes(&lane, "make", false, CancellationToken::new(), |_| {
        DomainResult::canonical_rejection(Some("make".into()), RefusalCode::BadValue, "built")
    });
    assert_eq!(executed.diagnostics[0]["message"], "built");
}
