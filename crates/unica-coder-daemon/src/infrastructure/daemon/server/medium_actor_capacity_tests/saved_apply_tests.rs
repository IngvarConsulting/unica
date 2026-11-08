use super::*;

/// rule: INV.APP.DAEMON-ACTOR-CAPACITY
/// rule: INV.WIRE.APPLY-REQUIRES-THE-FENCE
#[test]
fn saved_apply_token_survives_eight_other_workspace_profiles_and_replays_exactly() {
    let workspace = subsystem_picture_workspace("", true);
    let source = workspace.path().join("src");
    let before = unica_workspace::test_support::tree_snapshot(&source);
    let runtime = bootstrap_runtime();
    let arguments = serde_json::json!({
        "at": "main:Subsystem.Sales",
        "ops": [{"op": "props.set", "args": {"values": {"Comment": "retained exact plan"}}}]
    });
    let preview = submit_canonical(
        &runtime,
        workspace.path(),
        ToolIdentity::Apply,
        arguments.clone(),
    );
    assert!(preview.ok, "{preview:?}");
    let token = preview.data.as_ref().unwrap()["executionToken"].clone();
    let others = (0..8)
        .map(|_| subsystem_picture_workspace("", true))
        .collect::<Vec<_>>();
    let admit_others = || {
        for other in &others {
            let plan = submit_canonical(
                &runtime,
                other.path(),
                ToolIdentity::Apply,
                arguments.clone(),
            );
            assert!(plan.ok, "{plan:?}");
        }
    };
    // These are complete public plan calls, not held actor leases. Their
    // eight distinct profiles cross the production warm cache's LRU edge.
    admit_others();
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&source),
        before
    );
    let executed = submit_canonical(
        &runtime,
        workspace.path(),
        ToolIdentity::Apply,
        serde_json::json!({"executionToken": token}),
    );
    assert!(
        executed.ok,
        "an issued plan was lost after unrelated profiles: {executed:?}"
    );
    let published = unica_workspace::test_support::tree_snapshot(&source);
    assert!(std::fs::read_to_string(source.join("Subsystems/Sales.xml"))
        .unwrap()
        .contains("retained exact plan"));

    admit_others();
    let replayed = submit_canonical(
        &runtime,
        workspace.path(),
        ToolIdentity::Apply,
        serde_json::json!({"executionToken": token}),
    );
    assert_eq!(
        serde_json::to_value(&replayed).unwrap(),
        serde_json::to_value(&executed).unwrap()
    );
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&source),
        published,
        "replay changed the published source"
    );
}

/// rule: INV.APP.DAEMON-ACTOR-CAPACITY
/// rule: INV.WIRE.ADMISSION-NAMES-ITS-CAUSE
/// rule: INV.WIRE.APPLY-REQUIRES-THE-FENCE
/// rule: INV.WIRE.V13-REFUSAL-CHANNEL
#[test]
fn saved_apply_token_survives_temporary_named_root_validation_failure() {
    let workspace = subsystem_picture_workspace("", true);
    let source = workspace.path().join("src");
    let before = unica_workspace::test_support::tree_snapshot(&source);
    let runtime = bootstrap_runtime();
    let preview = submit_canonical(
        &runtime,
        workspace.path(),
        ToolIdentity::Apply,
        serde_json::json!({
            "at": "main:Subsystem.Sales",
            "ops": [{"op": "props.set", "args": {"values": {"Comment": "retained after temporary root failure"}}}]
        }),
    );
    assert!(preview.ok, "{preview:?}");
    let token = preview.data.as_ref().unwrap()["executionToken"].clone();

    runtime
        .workspace_actors
        .fail_next_named_root_validation_for_test(std::io::ErrorKind::PermissionDenied);
    let unavailable = submit_canonical(
        &runtime,
        workspace.path(),
        ToolIdentity::Apply,
        serde_json::json!({"executionToken": token}),
    );
    assert!(!unavailable.ok, "{unavailable:?}");
    assert_eq!(
        unavailable.diagnostics[0]["code"], "provider_unavailable",
        "временная ошибка открытия корня не должна превращать токен в неизвестный: {unavailable:?}"
    );
    assert_eq!(
        unavailable.diagnostics[0]["detailCode"],
        "source_temporarily_unavailable"
    );
    assert_eq!(unavailable.diagnostics[0]["outcome"], "retry");
    assert_eq!(unavailable.next[0]["tool"], "unica.apply");
    assert_eq!(unavailable.next[0]["args"]["executionToken"], token);
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&source),
        before,
        "отказ допуска не должен публиковать сохранённый план"
    );

    let executed = submit_canonical(
        &runtime,
        workspace.path(),
        ToolIdentity::Apply,
        serde_json::json!({"executionToken": token}),
    );
    assert!(
        executed.ok,
        "повтор после временной ошибки должен исполнить прежний токен: {executed:?}"
    );
    assert!(std::fs::read_to_string(source.join("Subsystems/Sales.xml"))
        .unwrap()
        .contains("retained after temporary root failure"));
}

fn saved_apply_survives_retention_age(age: Duration) {
    use unica_workspace::actor::with_actor_retention_time_for_test;

    let workspace = subsystem_picture_workspace("", true);
    let source = workspace.path().join("src");
    let before = unica_workspace::test_support::tree_snapshot(&source);
    let runtime = bootstrap_runtime();
    let started = Instant::now();
    let preview = with_actor_retention_time_for_test(started, || {
        submit_canonical(
            &runtime,
            workspace.path(),
            ToolIdentity::Apply,
            serde_json::json!({"at": "main:Subsystem.Sales", "ops": [{"op": "props.set", "args": {"values": {"Comment": "retained after former time limits"}}}]}),
        )
    });
    assert!(preview.ok, "{preview:?}");
    let token = preview.data.as_ref().unwrap()["executionToken"].clone();
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&source),
        before
    );
    let executed = with_actor_retention_time_for_test(started + age, || {
        submit_canonical(
            &runtime,
            workspace.path(),
            ToolIdentity::Apply,
            serde_json::json!({"executionToken": token}),
        )
    });
    assert!(
        executed.ok,
        "accepted plan was lost after {age:?}: {executed:?}"
    );
    let published = unica_workspace::test_support::tree_snapshot(&source);
    assert!(std::fs::read_to_string(source.join("Subsystems/Sales.xml"))
        .unwrap()
        .contains("retained after former time limits"));
    let replayed =
        with_actor_retention_time_for_test(started + age + Duration::from_secs(601), || {
            submit_canonical(
                &runtime,
                workspace.path(),
                ToolIdentity::Apply,
                serde_json::json!({"executionToken": token}),
            )
        });
    assert_eq!(
        serde_json::to_value(&replayed).unwrap(),
        serde_json::to_value(&executed).unwrap()
    );
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&source),
        published
    );
}

/// rule: INV.WIRE.APPLY-REQUIRES-THE-FENCE
#[test]
fn saved_apply_plan_survives_former_three_hundred_second_expiry() {
    saved_apply_survives_retention_age(Duration::from_secs(301));
}

/// rule: INV.APP.DAEMON-ACTOR-CAPACITY
/// rule: INV.WIRE.APPLY-REQUIRES-THE-FENCE
#[test]
fn saved_apply_plan_survives_former_six_hundred_second_actor_idle_window() {
    saved_apply_survives_retention_age(Duration::from_secs(601));
}

/// rule: INV.WIRE.APPLY-REQUIRES-THE-FENCE
#[test]
fn saved_apply_refusal_replays_after_former_actor_idle_window_without_replanning() {
    use unica_workspace::actor::with_actor_retention_time_for_test;
    let workspace = subsystem_picture_workspace("", true);
    let source = workspace.path().join("src");
    let runtime = bootstrap_runtime();
    let started = Instant::now();
    let preview = with_actor_retention_time_for_test(started, || {
        submit_canonical(
            &runtime,
            workspace.path(),
            ToolIdentity::Apply,
            serde_json::json!({"at": "main:Subsystem.Sales", "ops": [{"op": "props.set", "args": {"values": {"Comment": "saved refusal"}}}]}),
        )
    });
    assert!(preview.ok, "{preview:?}");
    let token = preview.data.as_ref().unwrap()["executionToken"].clone();
    let descriptor = source.join("Subsystems/Sales.xml");
    let mut changed = std::fs::read(&descriptor).unwrap();
    changed.extend_from_slice(b"\n");
    std::fs::write(&descriptor, changed).unwrap();
    let before = unica_workspace::test_support::tree_snapshot(&source);
    let refusal = with_actor_retention_time_for_test(started + Duration::from_secs(1), || {
        submit_canonical(
            &runtime,
            workspace.path(),
            ToolIdentity::Apply,
            serde_json::json!({"executionToken": token}),
        )
    });
    assert!(
        !refusal.ok,
        "changed inputs unexpectedly published: {refusal:?}"
    );
    assert_eq!(refusal.diagnostics[0]["code"], "stale_revision");
    let replay = with_actor_retention_time_for_test(started + Duration::from_secs(602), || {
        submit_canonical(
            &runtime,
            workspace.path(),
            ToolIdentity::Apply,
            serde_json::json!({"executionToken": token}),
        )
    });
    assert_eq!(
        serde_json::to_value(&replay).unwrap(),
        serde_json::to_value(&refusal).unwrap()
    );
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&source),
        before
    );
}

fn saved_apply_token_survives_cleanup_before_live_invocation_creates_plan(
    cleanup: CleanupBeforePromise,
) {
    use unica_workspace::actor::with_actor_retention_time_for_test;
    let workspace = subsystem_picture_workspace("", true);
    let source = workspace.path().join("src");
    let before = unica_workspace::test_support::tree_snapshot(&source);
    let runtime = bootstrap_runtime();
    let started = Instant::now();
    let request = InvocationRequest::new(
            ToolIdentity::Apply,
            serde_json::json!({"at": "main:Subsystem.Sales", "ops": [{"op": "props.set", "args": {"values": {"Comment": "plan created after cleanup"}}}]}),
            workspace.path().to_string_lossy(),
            7_000,
        ).unwrap();
    // Admission holds the actual actor while its promise tables are still
    // empty. Cleanup occurs before the real canonical handler saves a plan.
    let bound = with_actor_retention_time_for_test(started, || runtime.bind(request)).unwrap();
    let observed = match &bound.0 {
        ActorBoundCanonicalInvocationState::Workspace { invocation, .. } => {
            Arc::downgrade(invocation.actor_for_test())
        }
        _ => panic!("apply must bind its workspace actor"),
    };
    cleanup_before_live_invocation_creates_promise(&runtime, started, cleanup);
    let preview = bound
        .prepare()
        .unwrap()
        .execute(CancellationToken::new())
        .unwrap();
    assert!(preview.ok, "{preview:?}");
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&source),
        before,
        "preview must not write"
    );
    let token = preview.data.as_ref().unwrap()["executionToken"].clone();
    // execute consumes and drops the admitted invocation above. No test
    // Arc keeps that actor alive when the public token request rebinds it.
    let executed = submit_canonical(
        &runtime,
        workspace.path(),
        ToolIdentity::Apply,
        serde_json::json!({"executionToken": token}),
    );
    assert!(
        executed.ok,
        "cleanup lost a plan created by a live invocation: {executed:?}"
    );
    assert!(
        observed.upgrade().is_some(),
        "the promised plan retains its original actor"
    );
    let published = unica_workspace::test_support::tree_snapshot(&source);
    assert!(std::fs::read_to_string(source.join("Subsystems/Sales.xml"))
        .unwrap()
        .contains("plan created after cleanup"));
    let replay = submit_canonical(
        &runtime,
        workspace.path(),
        ToolIdentity::Apply,
        serde_json::json!({"executionToken": token}),
    );
    assert_eq!(
        serde_json::to_value(&replay).unwrap(),
        serde_json::to_value(&executed).unwrap()
    );
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&source),
        published,
        "exact replay must not write again"
    );
}

/// rule: INV.APP.DAEMON-ACTOR-CAPACITY
#[test]
fn saved_apply_token_survives_idle_cleanup_before_live_invocation_creates_plan() {
    saved_apply_token_survives_cleanup_before_live_invocation_creates_plan(
        CleanupBeforePromise::Idle,
    );
}

/// rule: INV.APP.DAEMON-ACTOR-CAPACITY
#[test]
fn saved_apply_token_survives_warm_release_before_live_invocation_creates_plan() {
    saved_apply_token_survives_cleanup_before_live_invocation_creates_plan(
        CleanupBeforePromise::Release,
    );
}

/// rule: INV.WIRE.APPLY-REQUIRES-THE-FENCE
#[test]
fn saved_apply_more_than_256_plans_keep_earlier_tokens_without_writes() {
    let workspace = subsystem_picture_workspace("", true);
    let source = workspace.path().join("src");
    let before = unica_workspace::test_support::tree_snapshot(&source);
    let runtime = bootstrap_runtime();
    let arguments = serde_json::json!({
        "at": "main:Subsystem.Sales",
        "ops": [{"op": "props.set", "args": {"values": {"Comment": "capacity probe"}}}]
    });
    let mut first_token = None;
    for _ in 0..256 {
        let plan = submit_canonical(
            &runtime,
            workspace.path(),
            ToolIdentity::Apply,
            arguments.clone(),
        );
        assert!(plan.ok, "{plan:?}");
        first_token.get_or_insert_with(|| plan.data.as_ref().unwrap()["executionToken"].clone());
    }
    let additional = submit_canonical(&runtime, workspace.path(), ToolIdentity::Apply, arguments);
    assert!(additional.ok, "{additional:?}");
    assert!(additional
        .data
        .as_ref()
        .and_then(|data| data.get("executionToken"))
        .is_some());
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&source),
        before
    );
    let executed = submit_canonical(
        &runtime,
        workspace.path(),
        ToolIdentity::Apply,
        serde_json::json!({"executionToken": first_token.unwrap()}),
    );
    assert!(executed.ok, "{executed:?}");
    assert!(std::fs::read_to_string(source.join("Subsystems/Sales.xml"))
        .unwrap()
        .contains("capacity probe"));
}

/// The saved-plan adapter carries the existing cutoff into real actor
/// publication. A finite cutoff must not become a fresh relative budget,
/// and an elapsed finite cutoff must not become absence or consume the plan.
/// rule: INV.RUNTIME.TIMERS-PRESERVE-WORK
#[test]
fn saved_apply_adapter_preserves_the_existing_cutoff_and_absence() {
    use unica_work_control::operation_deadline::OperationDeadline;
    use unica_workspace::actor::{ApplyPublicationErrorKind, SavedApplyExecutionError};

    let now = Instant::now();
    for deadline in [
        OperationDeadline::Finite(now + Duration::from_secs(3_600)),
        OperationDeadline::NoDeadline,
        OperationDeadline::Finite(now - Duration::from_secs(1)),
    ] {
        let workspace = subsystem_picture_workspace("", true);
        let source = workspace.path().join("src");
        let before = unica_workspace::test_support::tree_snapshot(&source);
        let runtime = bootstrap_runtime();
        let preview = submit_canonical(
            &runtime,
            workspace.path(),
            ToolIdentity::Apply,
            serde_json::json!({
                "at": "main:Subsystem.Sales",
                "ops": [{"op": "props.set", "args": {"values": {"Comment": "same saved cutoff"}}}],
            }),
        );
        assert!(preview.ok, "{preview:?}");
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&source),
            before
        );
        let token = preview.data.as_ref().unwrap()["executionToken"]
            .as_str()
            .unwrap();
        let request = InvocationRequest::new(
            ToolIdentity::Apply,
            serde_json::json!({"executionToken": token}),
            std::fs::canonicalize(workspace.path())
                .unwrap()
                .to_string_lossy(),
            7_000,
        )
        .unwrap();
        let cancellation = CancellationToken::new();
        let execution = bind_workspace_invocation(
            &request,
            &runtime.workspace_actors,
            Arc::clone(&runtime.deliveries),
            Arc::clone(&runtime.provider_hosts),
            Arc::clone(&runtime.runtime_resources),
            None,
            runtime.capture_response_deadline_for_test(),
        )
        .unwrap()
        .begin_execution(&cancellation)
        .unwrap();
        let expired = deadline.is_elapsed_at(Instant::now());
        let expected = match deadline {
            OperationDeadline::NoDeadline => ProviderDeadline::no_deadline(),
            OperationDeadline::Finite(cutoff) => ProviderDeadline::new(cutoff),
        };
        super::super::admission_test_control::take_apply_deadlines();
        let result = execution.execute_saved_apply(
            token,
            &cancellation,
            deadline,
            |preview, publication| {
                publication.expect("a live cutoff publishes the actual saved plan");
                preview
            },
        );
        let observed = super::super::admission_test_control::take_apply_deadline_values();
        assert_eq!(observed.len(), 1);
        if expired {
            assert_eq!(observed[0].remaining(), Some(Duration::ZERO));
            assert!(matches!(
                result,
                Err(SavedApplyExecutionError::Publication(error))
                    if error.kind() == ApplyPublicationErrorKind::Deadline
            ));
            assert_eq!(
                unica_workspace::test_support::tree_snapshot(&source),
                before
            );
            let retried = execution
                .execute_saved_apply(
                    token,
                    &cancellation,
                    OperationDeadline::NoDeadline,
                    |preview, publication| {
                        publication.expect("expiry must leave the saved plan pending");
                        preview
                    },
                )
                .unwrap();
            assert!(retried.ok, "{retried:?}");
            assert_eq!(
                super::super::admission_test_control::take_apply_deadline_values(),
                [ProviderDeadline::no_deadline()]
            );
        } else {
            assert_eq!(
                observed,
                [expected],
                "saved execution must preserve the input endpoint or its absence"
            );
            assert!(result.unwrap().ok);
        }
        assert!(std::fs::read_to_string(source.join("Subsystems/Sales.xml"))
            .unwrap()
            .contains("same saved cutoff"));
    }
}
