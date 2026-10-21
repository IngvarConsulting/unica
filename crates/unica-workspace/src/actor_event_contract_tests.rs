use super::test_support::*;
use super::*;
use crate::platform::testing::*;
use std::sync::mpsc;
use std::thread;
use unica_source_contract::events::DomainEventKind;

pub type EventPlanner = fn(
    &ApplyAdmission,
    &[String],
) -> Result<(ApplyStagedState, PlannedApplyEffects), ApplyPlanError>;
thread_local! { static PLANNER: std::cell::Cell<Option<EventPlanner>> = const { std::cell::Cell::new(None) }; }
struct PlannerScope(Option<EventPlanner>);
impl Drop for PlannerScope {
    fn drop(&mut self) {
        PLANNER.with(|slot| slot.set(self.0.take()));
    }
}
fn planner_scope(planner: EventPlanner) -> PlannerScope {
    PlannerScope(PLANNER.with(|slot| slot.replace(Some(planner))))
}
fn plan_actor_events(
    admitted: &ApplyAdmission,
    operations: &[String],
) -> Result<(ApplyStagedState, PlannedApplyEffects), ApplyPlanError> {
    PLANNER.with(|slot| {
        slot.get()
            .expect("real event planner is bound by the composition test")
    })(admitted, operations)
}
fn event_operation(at: &str) -> String {
    at.to_string()
}
fn prepare_property_effect_batch(
    fixture: &ActorFixture,
    binding: &super::ProviderRootBinding,
    dry_run: bool,
    cancellation: &CancellationToken,
) -> PreparedApplyBatch {
    let admitted = fixture
        .actor
        .admit_apply(
            binding,
            None,
            dry_run,
            ProviderDeadline::from_budget(Duration::from_secs(5)),
            cancellation,
        )
        .unwrap();
    let (state, effects) = plan_actor_events(
        &admitted,
        &[event_operation(
            "main:Catalog.Products.Form.Main.Event.OnOpen",
        )],
    )
    .unwrap();
    admitted.prepare_with_effects(state, effects).unwrap()
}

pub fn prepared_apply_effects_are_retained_from_planner_to_result(
    policies: crate::policies::WorkspacePolicies,
    planner: EventPlanner,
) {
    let _policy_scope = policies_scope(policies);
    let _planner_scope = planner_scope(planner);

    let fixture = actor_fixture("effect-receipt-retained", &["main"]);
    write_actor_event_fixture(&fixture.roots[0]);
    let binding = fixture
        .actor
        .bind_provider_root("main", &fixture.roots[0])
        .unwrap();
    let admitted = fixture
        .actor
        .admit_apply(
            &binding,
            None,
            true,
            ProviderDeadline::from_budget(Duration::from_secs(5)),
            &CancellationToken::new(),
        )
        .unwrap();
    let (state, effects) = plan_actor_events(
        &admitted,
        &[event_operation(
            "main:Catalog.Products.Form.Main.Event.OnOpen",
        )],
    )
    .unwrap();

    let result = fixture
        .actor
        .publish_prepared_apply(admitted.prepare_with_effects(state, effects).unwrap())
        .unwrap();
    let receipt = result.effects();

    assert_form_module_effect_subject(receipt, ApplyEffectDisposition::Projected);
    fixture.cleanup();
}

pub fn prepared_apply_dry_run_returns_projected_effect_receipt_without_any_write(
    policies: crate::policies::WorkspacePolicies,
    planner: EventPlanner,
) {
    let _policy_scope = policies_scope(policies);
    let _planner_scope = planner_scope(planner);

    let fixture = actor_fixture("effect-receipt-dry-run", &["main"]);
    write_actor_event_fixture(&fixture.roots[0]);
    let binding = fixture
        .actor
        .bind_provider_root("main", &fixture.roots[0])
        .unwrap();
    let source_before = snapshot_tree(&fixture.roots[0]);
    let cache_root = fixture.root.join(".build/unica");
    let cache_before = snapshot_tree(&cache_root);
    let admitted = fixture
        .actor
        .admit_apply(
            &binding,
            None,
            true,
            ProviderDeadline::from_budget(Duration::from_secs(5)),
            &CancellationToken::new(),
        )
        .unwrap();
    let (state, effects) = plan_actor_events(
        &admitted,
        &[event_operation(
            "main:Catalog.Products.Form.Main.Event.OnOpen",
        )],
    )
    .unwrap();
    let prepared = admitted.prepare_with_effects(state, effects).unwrap();
    let plan = prepared.plan_identity.clone();

    let result = fixture.actor.publish_prepared_apply(prepared).unwrap();
    let receipt = result.effects();

    assert_form_module_effect_subject(receipt, ApplyEffectDisposition::Projected);
    assert_eq!(result.rev(), plan);
    assert_eq!(result.commit_count_for_test(), 0);
    assert_eq!(snapshot_tree(&fixture.roots[0]), source_before);
    assert_eq!(snapshot_tree(&cache_root), cache_before);
    assert!(std::ptr::eq(receipt, result.effects()));
    assert!(
        !cache_root.exists(),
        "dry run created the absent cache root"
    );
    fixture.cleanup();
}

pub fn prepared_apply_success_returns_committed_effect_receipt_after_one_commit(
    policies: crate::policies::WorkspacePolicies,
    planner: EventPlanner,
) {
    let _policy_scope = policies_scope(policies);
    let _planner_scope = planner_scope(planner);

    use crate::compile_transaction::RetainedApplyObservedEvent;

    let fixture = actor_fixture("effect-receipt-commit", &["main"]);
    write_actor_event_fixture(&fixture.roots[0]);
    let binding = fixture
        .actor
        .bind_provider_root("main", &fixture.roots[0])
        .unwrap();
    let admitted = fixture
        .actor
        .admit_apply(
            &binding,
            None,
            false,
            ProviderDeadline::from_budget(Duration::from_secs(5)),
            &CancellationToken::new(),
        )
        .unwrap();
    let (state, effects) = plan_actor_events(
        &admitted,
        &[event_operation(
            "main:Catalog.Products.Form.Main.Event.OnOpen",
        )],
    )
    .unwrap();
    let prepared = admitted.prepare_with_effects(state, effects).unwrap();
    let plan = prepared.plan_identity.clone();
    let _ = crate::compile_transaction::take_retained_apply_observed_events();

    let result = fixture.actor.publish_prepared_apply(prepared).unwrap();
    let receipt = result.effects();

    assert_form_module_effect_subject(receipt, ApplyEffectDisposition::Committed);
    assert_eq!(result.rev(), plan);
    assert_eq!(result.commit_count_for_test(), 1);
    assert!(fixture.roots[0]
        .join("Catalogs/Products/Forms/Main/Ext/Form/Module.bsl")
        .is_file());
    let observed = crate::compile_transaction::take_retained_apply_observed_events();
    assert!(
        matches!(
            observed.as_slice(),
            [
                RetainedApplyObservedEvent::Source(_),
                RetainedApplyObservedEvent::Source(_),
                RetainedApplyObservedEvent::Source(_),
                RetainedApplyObservedEvent::Source(_),
                RetainedApplyObservedEvent::Source(_),
                RetainedApplyObservedEvent::EagerMetadata(_),
                RetainedApplyObservedEvent::StateMarker(_),
            ]
        ),
        "receipt became committed outside the one retained publication: {observed:?}"
    );
    fixture.cleanup();
}

pub fn event_implement_planner_integrates_with_actor_effect_publication_matrix(
    policies: crate::policies::WorkspacePolicies,
    planner: EventPlanner,
) {
    let _policy_scope = policies_scope(policies);
    let _planner_scope = planner_scope(planner);

    for (name, target, missing, expected) in [
        (
            "platform-available",
            "main:Catalog.Products.Module.Object.Event.BeforeWrite",
            false,
            vec![DomainEventKind::ModuleChanged],
        ),
        (
            "property-available",
            "main:Catalog.Products.Form.Main.Event.OnOpen",
            false,
            vec![DomainEventKind::FormChanged, DomainEventKind::ModuleChanged],
        ),
        (
            "property-missing",
            "main:Catalog.Products.Form.Main.Event.OnOpen",
            true,
            vec![DomainEventKind::ModuleChanged],
        ),
    ] {
        let fixture = actor_fixture(&format!("effect-matrix-{name}"), &["main"]);
        write_actor_event_fixture(&fixture.roots[0]);
        if missing {
            write_missing_on_open_binding(&fixture.roots[0]);
        }
        let binding = fixture
            .actor
            .bind_provider_root("main", &fixture.roots[0])
            .unwrap();
        let admitted = fixture
            .actor
            .admit_apply(
                &binding,
                None,
                true,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        let (state, effects) = plan_actor_events(&admitted, &[event_operation(target)]).unwrap();
        let result = fixture
            .actor
            .publish_prepared_apply(admitted.prepare_with_effects(state, effects).unwrap())
            .unwrap();
        let receipt = result.effects();
        assert_eq!(
            receipt
                .events()
                .iter()
                .map(|event| event.kind)
                .collect::<Vec<_>>(),
            expected,
            "{name}"
        );
        fixture.cleanup();
    }

    let operations = [
        event_operation("main:Catalog.Products.Form.Main.Event.OnOpen"),
        event_operation("main:Catalog.Products.Form.Main.Event.OnClose"),
    ];
    let mut subjects = Vec::new();
    for dry_run in [true, false] {
        let fixture = actor_fixture(
            if dry_run {
                "effect-matrix-dry-parity"
            } else {
                "effect-matrix-real-parity"
            },
            &["main"],
        );
        write_actor_event_fixture(&fixture.roots[0]);
        let binding = fixture
            .actor
            .bind_provider_root("main", &fixture.roots[0])
            .unwrap();
        let admitted = fixture
            .actor
            .admit_apply(
                &binding,
                None,
                dry_run,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        let (state, effects) = plan_actor_events(&admitted, &operations).unwrap();
        let result = fixture
            .actor
            .publish_prepared_apply(admitted.prepare_with_effects(state, effects).unwrap())
            .unwrap();
        let receipt = result.effects();
        subjects.push((
            receipt.events().to_vec(),
            receipt.cache().events.clone(),
            receipt.cache().invalidated.clone(),
            receipt.cache().refreshed.clone(),
        ));
        fixture.cleanup();
    }
    assert_eq!(
        subjects[0], subjects[1],
        "dry/real receipt subjects diverged"
    );
    assert_eq!(
        subjects[0]
            .0
            .iter()
            .map(|event| event.kind)
            .collect::<Vec<_>>(),
        vec![DomainEventKind::FormChanged, DomainEventKind::ModuleChanged],
        "multi-op planner lost stable first-occurrence deduplication"
    );
}

pub fn event_implement_op_failure_returns_no_effect_receipt_and_preserves_all_state(
    policies: crate::policies::WorkspacePolicies,
    planner: EventPlanner,
) {
    let _policy_scope = policies_scope(policies);
    let _planner_scope = planner_scope(planner);

    let control = actor_fixture("effect-poison-control", &["main"]);
    write_actor_event_fixture(&control.roots[0]);
    let control_binding = control
        .actor
        .bind_provider_root("main", &control.roots[0])
        .unwrap();
    let control_admission = control
        .actor
        .admit_apply(
            &control_binding,
            None,
            true,
            ProviderDeadline::from_budget(Duration::from_secs(5)),
            &CancellationToken::new(),
        )
        .unwrap();
    let (control_state, control_effects) = plan_actor_events(
        &control_admission,
        &[event_operation(
            "main:Catalog.Products.Form.Main.Event.OnOpen",
        )],
    )
    .unwrap();
    let control_result = control
        .actor
        .publish_prepared_apply(
            control_admission
                .prepare_with_effects(control_state, control_effects)
                .unwrap(),
        )
        .unwrap();
    assert_form_module_effect_subject(control_result.effects(), ApplyEffectDisposition::Projected);
    control.cleanup();

    for dry_run in [true, false] {
        let fixture = actor_fixture(
            if dry_run {
                "effect-poison-dry"
            } else {
                "effect-poison-real"
            },
            &["main"],
        );
        write_actor_event_fixture(&fixture.roots[0]);
        let binding = fixture
            .actor
            .bind_provider_root("main", &fixture.roots[0])
            .unwrap();
        let source_before = snapshot_tree(&fixture.roots[0]);
        let cache_root = fixture.root.join(".build/unica");
        let cache_before = snapshot_tree(&cache_root);
        let admitted = fixture
            .actor
            .admit_apply(
                &binding,
                None,
                dry_run,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        let operations = [
            event_operation("main:Catalog.Products.Form.Main.Event.OnOpen"),
            event_operation("main:Catalog.Products.Form.Main.Event.DoesNotExist"),
        ];

        let error = plan_actor_events(&admitted, &operations)
            .expect_err("poisoned operation batch unexpectedly reached preparation");

        assert_eq!(error.kind(), ApplyPlanErrorKind::ProviderUnavailable);
        assert_eq!(snapshot_tree(&fixture.roots[0]), source_before);
        assert_eq!(snapshot_tree(&cache_root), cache_before);
        assert!(!cache_root.exists());
        fixture.cleanup();
    }
}

pub fn retained_apply_effect_failure_matrix_rolls_back_and_returns_no_receipt(
    policies: crate::policies::WorkspacePolicies,
    planner: EventPlanner,
) {
    let _policy_scope = policies_scope(policies);
    let _planner_scope = planner_scope(planner);

    use crate::compile_transaction::RetainedApplyFailpoint;

    for (name, failpoint) in [
        ("second-source", RetainedApplyFailpoint::Source(2)),
        ("eager-cache", RetainedApplyFailpoint::EagerMetadata(1)),
        ("state-marker", RetainedApplyFailpoint::StateMarker),
        (
            "final-validation",
            RetainedApplyFailpoint::AfterAllPostimages,
        ),
    ] {
        let fixture = actor_fixture(&format!("effect-failure-{name}"), &["main"]);
        write_actor_event_fixture(&fixture.roots[0]);
        let binding = fixture
            .actor
            .bind_provider_root("main", &fixture.roots[0])
            .unwrap();
        let prepared =
            prepare_property_effect_batch(&fixture, &binding, false, &CancellationToken::new());
        let prepared_receipt = &prepared.effects;
        assert_eq!(
            prepared_receipt
                .events
                .iter()
                .map(|event| event.kind)
                .collect::<Vec<_>>(),
            [DomainEventKind::FormChanged, DomainEventKind::ModuleChanged]
        );
        assert_eq!(prepared_receipt.cache.mode, "applied");
        let source_before = snapshot_tree(&fixture.roots[0]);
        let cache_root = fixture.root.join(".build/unica");
        let cache_before = snapshot_tree(&cache_root);
        crate::compile_transaction::set_retained_apply_failpoint(failpoint);

        let error = fixture.actor.publish_prepared_apply(prepared).unwrap_err();

        assert_eq!(
            error.kind(),
            super::ApplyPublicationErrorKind::ProviderPostvalidation,
            "{name}: {error}"
        );
        assert_eq!(snapshot_tree(&fixture.roots[0]), source_before, "{name}");
        assert_eq!(snapshot_tree(&cache_root), cache_before, "{name}");
        fixture.cleanup();
    }

    let fixture = actor_fixture("effect-failure-final-cancel", &["main"]);
    write_actor_event_fixture(&fixture.roots[0]);
    let binding = fixture
        .actor
        .bind_provider_root("main", &fixture.roots[0])
        .unwrap();
    let cancellation = CancellationToken::new();
    let prepared = prepare_property_effect_batch(&fixture, &binding, false, &cancellation);
    assert!(!prepared.effects.events.is_empty());
    let source_before = snapshot_tree(&fixture.roots[0]);
    let cache_root = fixture.root.join(".build/unica");
    let cache_before = snapshot_tree(&cache_root);
    let cancel_at_final_gate = cancellation.clone();
    crate::compile_transaction::set_retained_apply_before_post_validation_hook(move || {
        cancel_at_final_gate.cancel()
    });
    let error = fixture.actor.publish_prepared_apply(prepared).unwrap_err();
    assert_eq!(error.kind(), super::ApplyPublicationErrorKind::Cancelled);
    assert_eq!(snapshot_tree(&fixture.roots[0]), source_before);
    assert_eq!(snapshot_tree(&cache_root), cache_before);
    fixture.cleanup();

    let fixture = actor_fixture("effect-failure-rollback-incomplete", &["main"]);
    write_actor_event_fixture(&fixture.roots[0]);
    let binding = fixture
        .actor
        .bind_provider_root("main", &fixture.roots[0])
        .unwrap();
    let cancellation = CancellationToken::new();
    let prepared = prepare_property_effect_batch(&fixture, &binding, false, &cancellation);
    assert!(!prepared.effects.events.is_empty());
    let target = fixture.roots[0].join("Catalogs/Products/Forms/Main/Ext/Form.xml");
    let moved = fixture.roots[0].join("Catalogs/Products/Forms/Main/Ext/Form-published-moved.xml");
    let raced_target = target.clone();
    let raced_moved = moved.clone();
    let cancel = cancellation.clone();
    crate::compile_transaction::set_retained_apply_before_post_validation_hook(move || {
        std::fs::rename(&raced_target, &raced_moved).unwrap();
        std::fs::write(&raced_target, b"foreign-form").unwrap();
        cancel.cancel();
    });

    let error = fixture.actor.publish_prepared_apply(prepared).unwrap_err();

    assert_eq!(
        error.kind(),
        super::ApplyPublicationErrorKind::RollbackIncomplete
    );
    assert_eq!(std::fs::read(&target).unwrap(), b"foreign-form");
    assert!(
        moved.is_file(),
        "foreign replacement erased published evidence"
    );
    fixture.cleanup();
}

pub fn retained_apply_effect_races_never_publish_or_return_effects(
    policies: crate::policies::WorkspacePolicies,
    planner: EventPlanner,
) {
    let _policy_scope = policies_scope(policies);
    let _planner_scope = planner_scope(planner);

    let stale = actor_fixture("effect-race-stale", &["main"]);
    write_actor_event_fixture(&stale.roots[0]);
    let stale_binding = stale
        .actor
        .bind_provider_root("main", &stale.roots[0])
        .unwrap();
    let stale_admission = stale
        .actor
        .admit_apply(
            &stale_binding,
            Some("stale-revision"),
            false,
            ProviderDeadline::from_budget(Duration::from_secs(5)),
            &CancellationToken::new(),
        )
        .unwrap();
    let (state, effects) = plan_actor_events(
        &stale_admission,
        &[event_operation(
            "main:Catalog.Products.Form.Main.Event.OnOpen",
        )],
    )
    .unwrap();
    let stale_error = stale_admission
        .prepare_with_effects(state, effects)
        .unwrap_err();
    assert_eq!(
        stale_error.kind(),
        ApplyStagingErrorKind::ConcurrentRevision
    );
    assert!(stale_error.to_string().contains("ifRev"));
    assert!(!stale.root.join(".build/unica").exists());
    stale.cleanup();

    let fixture = actor_fixture("effect-race-revision", &["main"]);
    write_actor_event_fixture(&fixture.roots[0]);
    let binding = fixture
        .actor
        .bind_provider_root("main", &fixture.roots[0])
        .unwrap();
    let prepared =
        prepare_property_effect_batch(&fixture, &binding, false, &CancellationToken::new());
    assert!(
        !prepared.effects.events.is_empty(),
        "racing prepared batch discarded the receipt subject before the gate"
    );
    let cache_before = snapshot_tree(&fixture.root.join(".build/unica"));
    std::fs::write(
        fixture.roots[0].join("Catalogs/Products/Forms/Main/Ext/Form.xml"),
        b"foreign",
    )
    .unwrap();
    let error = fixture.actor.publish_prepared_apply(prepared).unwrap_err();
    assert_eq!(
        error.kind(),
        super::ApplyPublicationErrorKind::ConcurrentRevision
    );
    assert_eq!(
        snapshot_tree(&fixture.root.join(".build/unica")),
        cache_before
    );
    assert_eq!(
        std::fs::read(fixture.roots[0].join("Catalogs/Products/Forms/Main/Ext/Form.xml")).unwrap(),
        b"foreign"
    );
    fixture.cleanup();

    let fixture = actor_fixture("effect-race-source-root", &["main"]);
    write_actor_event_fixture(&fixture.roots[0]);
    let binding = fixture
        .actor
        .bind_provider_root("main", &fixture.roots[0])
        .unwrap();
    let prepared =
        prepare_property_effect_batch(&fixture, &binding, false, &CancellationToken::new());
    assert!(!prepared.effects.events.is_empty());
    let displaced = fixture.root.join("main-displaced");
    let retained_identity = path_identity_for_test(&fixture.roots[0])
        .unwrap()
        .expect("source root identity must be available on supported CI platforms");
    match attempt_retained_directory_replacement_for_test(&fixture.roots[0], &displaced).unwrap() {
        RetainedDirectoryReplacementOutcome::Replaced => {
            std::fs::create_dir_all(&fixture.roots[0]).unwrap();
            std::fs::write(fixture.roots[0].join("foreign.txt"), b"foreign").unwrap();
            let error = fixture.actor.publish_prepared_apply(prepared).unwrap_err();
            assert_eq!(
                error.kind(),
                super::ApplyPublicationErrorKind::ContainmentIdentity
            );
            assert_eq!(
                std::fs::read(fixture.roots[0].join("foreign.txt")).unwrap(),
                b"foreign"
            );
        }
        RetainedDirectoryReplacementOutcome::PreventedByRetainedHandle => {
            assert_eq!(
                path_identity_for_test(&fixture.roots[0])
                    .unwrap()
                    .as_deref(),
                Some(retained_identity.as_str())
            );
            assert!(!displaced.exists());
            let result = fixture.actor.publish_prepared_apply(prepared).unwrap();
            assert!(!result.effects().events().is_empty());
            assert!(!fixture.roots[0].join("foreign.txt").exists());
        }
    }
    fixture.cleanup();

    let fixture = actor_fixture("effect-race-cache-root", &["main"]);
    write_actor_event_fixture(&fixture.roots[0]);
    let binding = fixture
        .actor
        .bind_provider_root("main", &fixture.roots[0])
        .unwrap();
    let prepared =
        prepare_property_effect_batch(&fixture, &binding, false, &CancellationToken::new());
    assert!(!prepared.effects.events.is_empty());
    std::fs::write(fixture.root.join(".build"), b"foreign-cache-parent").unwrap();
    let error = fixture.actor.publish_prepared_apply(prepared).unwrap_err();
    assert_eq!(
        error.kind(),
        super::ApplyPublicationErrorKind::ContainmentIdentity
    );
    assert_eq!(
        std::fs::read(fixture.root.join(".build")).unwrap(),
        b"foreign-cache-parent"
    );
    fixture.cleanup();

    for gate in ["cancelled", "deadline"] {
        let fixture = actor_fixture(&format!("effect-race-{gate}"), &["main"]);
        write_actor_event_fixture(&fixture.roots[0]);
        let binding = fixture
            .actor
            .bind_provider_root("main", &fixture.roots[0])
            .unwrap();
        let cancellation = CancellationToken::new();
        let mut prepared = prepare_property_effect_batch(&fixture, &binding, false, &cancellation);
        assert!(!prepared.effects.events.is_empty());
        if gate == "cancelled" {
            cancellation.cancel();
        } else {
            prepared.deadline = ProviderDeadline::from_budget(Duration::ZERO);
        }
        let error = fixture.actor.publish_prepared_apply(prepared).unwrap_err();
        assert_eq!(
            error.kind(),
            if gate == "cancelled" {
                super::ApplyPublicationErrorKind::Cancelled
            } else {
                super::ApplyPublicationErrorKind::Deadline
            }
        );
        fixture.cleanup();
    }

    let fixture = actor_fixture("effect-race-dry-final", &["main"]);
    write_actor_event_fixture(&fixture.roots[0]);
    let binding = fixture
        .actor
        .bind_provider_root("main", &fixture.roots[0])
        .unwrap();
    let cancellation = CancellationToken::new();
    let prepared = prepare_property_effect_batch(&fixture, &binding, true, &cancellation);
    assert!(!prepared.effects.events.is_empty());
    let cancel = cancellation.clone();
    set_apply_dry_run_after_confirmation_hook(move || cancel.cancel());
    let error = fixture.actor.publish_prepared_apply(prepared).unwrap_err();
    assert_eq!(error.kind(), super::ApplyPublicationErrorKind::Cancelled);
    fixture.cleanup();
}

pub fn real_effect_foreign_actor_replay_preserves_both_actor_states(
    policies: crate::policies::WorkspacePolicies,
    planner: EventPlanner,
) {
    let _policy_scope = policies_scope(policies);
    let _planner_scope = planner_scope(planner);

    let first = actor_fixture("effect-foreign-actor-first", &["main"]);
    let second = actor_fixture("effect-foreign-actor-second", &["main"]);
    write_actor_event_fixture(&first.roots[0]);
    write_actor_event_fixture(&second.roots[0]);
    let first_binding = first
        .actor
        .bind_provider_root("main", &first.roots[0])
        .unwrap();
    let prepared =
        prepare_property_effect_batch(&first, &first_binding, false, &CancellationToken::new());
    assert_prepared_form_module_effect_subject(&prepared);
    let first_source_before = snapshot_tree(&first.roots[0]);
    let first_cache_before = snapshot_tree(&first.root.join(".build/unica"));
    let second_source_before = snapshot_tree(&second.roots[0]);
    let second_cache_before = snapshot_tree(&second.root.join(".build/unica"));

    let error = second.actor.publish_prepared_apply(prepared).unwrap_err();

    assert_eq!(error.kind(), super::ApplyPublicationErrorKind::Invariant);
    assert!(error.contains("another workspace actor"), "{error}");
    assert_eq!(snapshot_tree(&first.roots[0]), first_source_before);
    assert_eq!(
        snapshot_tree(&first.root.join(".build/unica")),
        first_cache_before
    );
    assert_eq!(snapshot_tree(&second.roots[0]), second_source_before);
    assert_eq!(
        snapshot_tree(&second.root.join(".build/unica")),
        second_cache_before
    );
    first.cleanup();
    second.cleanup();
}

pub fn real_effect_mutation_lane_cancellation_preserves_exact_state(
    policies: crate::policies::WorkspacePolicies,
    planner: EventPlanner,
) {
    let _policy_scope = policies_scope(policies);
    let _planner_scope = planner_scope(planner);

    let fixture = actor_fixture("effect-late-lane-cancel", &["main"]);
    write_actor_event_fixture(&fixture.roots[0]);
    let binding = fixture
        .actor
        .bind_provider_root("main", &fixture.roots[0])
        .unwrap();
    let cancellation = CancellationToken::new();
    let prepared = prepare_property_effect_batch(&fixture, &binding, false, &cancellation);
    assert_prepared_form_module_effect_subject(&prepared);
    let source_before = snapshot_tree(&fixture.roots[0]);
    let cache_before = snapshot_tree(&fixture.root.join(".build/unica"));
    let _ = crate::compile_transaction::take_retained_apply_observed_events();
    let owner = fixture.actor.mutation_lane.hold_for_test();
    let actor = Arc::clone(&fixture.actor);
    let (started_tx, started_rx) = mpsc::channel();
    let (result_tx, result_rx) = mpsc::channel();
    let publisher = thread::spawn(move || {
        started_tx.send(()).unwrap();
        result_tx
            .send(actor.publish_prepared_apply(prepared))
            .unwrap();
    });
    started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    assert!(
        result_rx.recv_timeout(Duration::from_millis(50)).is_err(),
        "real-effect publication crossed the held mutation lane"
    );
    cancellation.cancel();

    let error = result_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("late cancellation did not stop real-effect lane wait")
        .unwrap_err();
    assert_eq!(error.kind(), super::ApplyPublicationErrorKind::Cancelled);
    assert!(
        crate::compile_transaction::take_retained_apply_observed_events().is_empty(),
        "lane cancellation reached retained publication"
    );
    assert_eq!(snapshot_tree(&fixture.roots[0]), source_before);
    assert_eq!(
        snapshot_tree(&fixture.root.join(".build/unica")),
        cache_before
    );
    drop(owner);
    publisher.join().unwrap();
    fixture.cleanup();
}

pub fn real_effect_mutation_lane_deadline_preserves_exact_state(
    policies: crate::policies::WorkspacePolicies,
    planner: EventPlanner,
) {
    let _policy_scope = policies_scope(policies);
    let _planner_scope = planner_scope(planner);

    let fixture = actor_fixture("effect-late-lane-deadline", &["main"]);
    write_actor_event_fixture(&fixture.roots[0]);
    let binding = fixture
        .actor
        .bind_provider_root("main", &fixture.roots[0])
        .unwrap();
    let cancellation = CancellationToken::new();
    let mut prepared = prepare_property_effect_batch(&fixture, &binding, false, &cancellation);
    assert_prepared_form_module_effect_subject(&prepared);
    prepared.deadline = ProviderDeadline::from_budget(Duration::from_millis(40));
    let source_before = snapshot_tree(&fixture.roots[0]);
    let cache_before = snapshot_tree(&fixture.root.join(".build/unica"));
    let _ = crate::compile_transaction::take_retained_apply_observed_events();
    let owner = fixture.actor.mutation_lane.hold_for_test();
    let cancel_after_deadline = cancellation.clone();
    crate::deadline_lock::set_after_deadline_error_hook_for_test(move || {
        std::thread::spawn(move || cancel_after_deadline.cancel())
            .join()
            .unwrap();
    });

    let error = fixture.actor.publish_prepared_apply(prepared).unwrap_err();

    assert_eq!(error.kind(), super::ApplyPublicationErrorKind::Deadline);
    assert!(cancellation.is_cancelled());
    assert!(
        crate::compile_transaction::take_retained_apply_observed_events().is_empty(),
        "lane deadline reached retained publication"
    );
    assert_eq!(snapshot_tree(&fixture.roots[0]), source_before);
    assert_eq!(
        snapshot_tree(&fixture.root.join(".build/unica")),
        cache_before
    );
    drop(owner);
    fixture.cleanup();
}

pub fn real_effect_input_confirmation_cancellation_preserves_exact_state(
    policies: crate::policies::WorkspacePolicies,
    planner: EventPlanner,
) {
    let _policy_scope = policies_scope(policies);
    let _planner_scope = planner_scope(planner);

    let fixture = actor_fixture("effect-input confirmation-cancel", &["main"]);
    write_actor_event_fixture(&fixture.roots[0]);
    let binding = fixture
        .actor
        .bind_provider_root("main", &fixture.roots[0])
        .unwrap();
    let cancellation = CancellationToken::new();
    let prepared = prepare_property_effect_batch(&fixture, &binding, false, &cancellation);
    assert_prepared_form_module_effect_subject(&prepared);
    let source_before = snapshot_tree(&fixture.roots[0]);
    let cache_before = snapshot_tree(&fixture.root.join(".build/unica"));
    let _ = crate::compile_transaction::take_retained_apply_observed_events();
    let cancel_during_confirmation = cancellation.clone();
    set_apply_before_input_confirmation_hook(move || cancel_during_confirmation.cancel());
    let error = fixture.actor.publish_prepared_apply(prepared).unwrap_err();

    assert_eq!(error.kind(), super::ApplyPublicationErrorKind::Cancelled);
    assert!(
        crate::compile_transaction::take_retained_apply_observed_events().is_empty(),
        "input confirmation cancellation reached retained publication"
    );
    assert_eq!(snapshot_tree(&fixture.roots[0]), source_before);
    assert_eq!(
        snapshot_tree(&fixture.root.join(".build/unica")),
        cache_before
    );
    fixture.cleanup();
}

pub fn real_effect_input_confirmation_deadline_preserves_exact_state(
    policies: crate::policies::WorkspacePolicies,
    planner: EventPlanner,
) {
    let _policy_scope = policies_scope(policies);
    let _planner_scope = planner_scope(planner);

    let fixture = actor_fixture("effect-input confirmation-deadline", &["main"]);
    write_actor_event_fixture(&fixture.roots[0]);
    let binding = fixture
        .actor
        .bind_provider_root("main", &fixture.roots[0])
        .unwrap();
    let mut prepared =
        prepare_property_effect_batch(&fixture, &binding, false, &CancellationToken::new());
    assert_prepared_form_module_effect_subject(&prepared);
    prepared.deadline = ProviderDeadline::from_budget(Duration::from_millis(200));
    let source_before = snapshot_tree(&fixture.roots[0]);
    let cache_before = snapshot_tree(&fixture.root.join(".build/unica"));
    let _ = crate::compile_transaction::take_retained_apply_observed_events();
    let now = Instant::now();
    RETAINED_APPLY_DEADLINE_TEST_NOW.set(now);
    prepared.deadline = ProviderDeadline::with_clock(
        now + Duration::from_secs(1),
        retained_apply_deadline_test_now,
    );
    set_apply_before_input_confirmation_hook(move || {
        RETAINED_APPLY_DEADLINE_TEST_NOW.set(now + Duration::from_secs(2))
    });
    let error = fixture.actor.publish_prepared_apply(prepared).unwrap_err();

    assert_eq!(error.kind(), super::ApplyPublicationErrorKind::Deadline);
    assert!(
        crate::compile_transaction::take_retained_apply_observed_events().is_empty(),
        "input confirmation deadline reached retained publication"
    );
    assert_eq!(snapshot_tree(&fixture.roots[0]), source_before);
    assert_eq!(
        snapshot_tree(&fixture.root.join(".build/unica")),
        cache_before
    );
    fixture.cleanup();
}

pub fn real_effect_after_all_postimages_cancellation_rolls_back_exact_state(
    policies: crate::policies::WorkspacePolicies,
    planner: EventPlanner,
) {
    let _policy_scope = policies_scope(policies);
    let _planner_scope = planner_scope(planner);

    use crate::compile_transaction::RetainedApplyObservedEvent;

    let fixture = actor_fixture("effect-after-postimages-cancel", &["main"]);
    write_actor_event_fixture(&fixture.roots[0]);
    let binding = fixture
        .actor
        .bind_provider_root("main", &fixture.roots[0])
        .unwrap();
    let cancellation = CancellationToken::new();
    let prepared = prepare_property_effect_batch(&fixture, &binding, false, &cancellation);
    assert_prepared_form_module_effect_subject(&prepared);
    let source_before = snapshot_tree(&fixture.roots[0]);
    let cache_before = snapshot_tree(&fixture.root.join(".build/unica"));
    let _ = crate::compile_transaction::take_retained_apply_observed_events();
    let cancel_after_postimages = cancellation.clone();
    crate::compile_transaction::set_retained_apply_before_post_validation_hook(move || {
        cancel_after_postimages.cancel()
    });

    let error = fixture.actor.publish_prepared_apply(prepared).unwrap_err();

    assert_eq!(error.kind(), super::ApplyPublicationErrorKind::Cancelled);
    let observed = crate::compile_transaction::take_retained_apply_observed_events();
    assert!(observed
        .iter()
        .any(|event| matches!(event, RetainedApplyObservedEvent::StateMarker(_))));
    assert!(
        observed
            .iter()
            .any(|event| matches!(event, RetainedApplyObservedEvent::Rollback(_))),
        "after-postimages cancellation did not roll back: {observed:?}"
    );
    assert_eq!(snapshot_tree(&fixture.roots[0]), source_before);
    assert_eq!(
        snapshot_tree(&fixture.root.join(".build/unica")),
        cache_before
    );
    fixture.cleanup();
}

pub fn real_effect_after_all_postimages_deadline_rolls_back_exact_state(
    policies: crate::policies::WorkspacePolicies,
    planner: EventPlanner,
) {
    let _policy_scope = policies_scope(policies);
    let _planner_scope = planner_scope(planner);

    use crate::compile_transaction::RetainedApplyObservedEvent;

    let fixture = actor_fixture("effect-after-postimages-deadline", &["main"]);
    write_actor_event_fixture(&fixture.roots[0]);
    let binding = fixture
        .actor
        .bind_provider_root("main", &fixture.roots[0])
        .unwrap();
    let mut prepared =
        prepare_property_effect_batch(&fixture, &binding, false, &CancellationToken::new());
    assert_prepared_form_module_effect_subject(&prepared);
    let started = Instant::now();
    RETAINED_APPLY_DEADLINE_TEST_NOW.set(started);
    prepared.deadline = ProviderDeadline::with_clock(
        started + Duration::from_secs(1),
        retained_apply_deadline_test_now,
    );
    let source_before = snapshot_tree(&fixture.roots[0]);
    let cache_before = snapshot_tree(&fixture.root.join(".build/unica"));
    let _ = crate::compile_transaction::take_retained_apply_observed_events();
    crate::compile_transaction::set_retained_apply_before_post_validation_hook(move || {
        RETAINED_APPLY_DEADLINE_TEST_NOW.set(started + Duration::from_secs(2))
    });

    let error = fixture.actor.publish_prepared_apply(prepared).unwrap_err();

    assert_eq!(error.kind(), super::ApplyPublicationErrorKind::Deadline);
    let observed = crate::compile_transaction::take_retained_apply_observed_events();
    assert!(observed
        .iter()
        .any(|event| matches!(event, RetainedApplyObservedEvent::StateMarker(_))));
    assert!(
        observed
            .iter()
            .any(|event| matches!(event, RetainedApplyObservedEvent::Rollback(_))),
        "after-postimages deadline did not roll back: {observed:?}"
    );
    assert_eq!(snapshot_tree(&fixture.roots[0]), source_before);
    assert_eq!(
        snapshot_tree(&fixture.root.join(".build/unica")),
        cache_before
    );
    fixture.cleanup();
}

pub fn saved_apply_survives_unchanged_read_and_rejects_same_byte_replacement(
    policies: crate::policies::WorkspacePolicies,
    planner: EventPlanner,
) {
    let _policy_scope = policies_scope(policies);
    let _planner_scope = planner_scope(planner);

    let fixture = actor_fixture("saved-plan-read-fallback", &["main"]);
    write_actor_event_fixture(&fixture.roots[0]);
    let binding = fixture
        .actor
        .bind_provider_root("main", &fixture.roots[0])
        .unwrap();
    let form_relative = Path::new("Catalogs/Products/Forms/Main/Ext/Form.xml");
    let form_path = fixture.roots[0].join(form_relative);
    let read = || {
        fixture
            .actor
            .read_relative_file(&binding, form_relative, 1024 * 1024)
            .unwrap()
    };
    let form_before = read();
    let batch = prepare_property_effect_batch(&fixture, &binding, true, &CancellationToken::new());
    fixture.actor.preview_prepared_apply(&batch).unwrap();
    let token = fixture
        .actor
        .save_prepared_apply(
            batch,
            unica_execution_contract::invocation::DomainResult::success("preview"),
        )
        .unwrap();
    let source_before = snapshot_tree(&fixture.roots[0]);
    let cache_before = snapshot_tree(&fixture.root.join(".build/unica"));
    assert_eq!(read(), form_before);
    assert_eq!(snapshot_tree(&fixture.roots[0]), source_before);
    assert_eq!(
        snapshot_tree(&fixture.root.join(".build/unica")),
        cache_before
    );
    let result = fixture
        .actor
        .execute_saved_apply(
            &token,
            ProviderDeadline::from_budget(Duration::from_secs(5)),
            &CancellationToken::new(),
            finish_saved_apply_test,
        )
        .unwrap();
    assert!(result.ok, "{}", result.summary);
    assert_eq!(result.data.as_ref().unwrap()["commits"], 1);
    assert_ne!(snapshot_tree(&fixture.roots[0]), source_before);
    assert_ne!(read(), form_before);

    // A same-byte replacement by another writer must still invalidate the
    // saved authority; a later read must not adopt it into the old plan.
    let admitted = fixture
        .actor
        .admit_apply(
            &binding,
            None,
            true,
            ProviderDeadline::from_budget(Duration::from_secs(5)),
            &CancellationToken::new(),
        )
        .unwrap();
    let (state, effects) = plan_actor_events(
        &admitted,
        &[event_operation(
            "main:Catalog.Products.Form.Main.Event.OnClose",
        )],
    )
    .unwrap();
    let batch = admitted.prepare_with_effects(state, effects).unwrap();
    fixture.actor.preview_prepared_apply(&batch).unwrap();
    let foreign_token = fixture
        .actor
        .save_prepared_apply(
            batch,
            unica_execution_contract::invocation::DomainResult::success("preview"),
        )
        .unwrap();
    let replacement = form_path.with_extension("replacement");
    let same_bytes = read();
    std::fs::write(&replacement, &same_bytes).unwrap();
    std::fs::rename(&replacement, &form_path).unwrap();
    let source_before = snapshot_tree(&fixture.roots[0]);
    let cache_before = snapshot_tree(&fixture.root.join(".build/unica"));
    assert_eq!(read(), same_bytes);
    let refused = fixture
        .actor
        .execute_saved_apply(
            &foreign_token,
            ProviderDeadline::from_budget(Duration::from_secs(5)),
            &CancellationToken::new(),
            finish_saved_apply_test,
        )
        .unwrap();
    assert!(
        !refused.ok,
        "a read must not replace a saved file authority"
    );
    assert_eq!(snapshot_tree(&fixture.roots[0]), source_before);
    assert_eq!(
        snapshot_tree(&fixture.root.join(".build/unica")),
        cache_before
    );
    fixture.cleanup();
}

pub fn saved_source_diff_reads_without_write_lane_and_survives_busy_or_cancelled_reads(
    policies: crate::policies::WorkspacePolicies,
    planner: EventPlanner,
    renderer: crate::preview::SavedSourceDiffFactory,
) {
    let _policy_scope = policies_scope(policies);
    let _planner_scope = planner_scope(planner);

    let fixture = actor_fixture("saved-source-diff-lanes", &["main"]);
    write_actor_event_fixture(&fixture.roots[0]);
    let binding = fixture
        .actor
        .bind_provider_root("main", &fixture.roots[0])
        .unwrap();
    let planning_cancel = CancellationToken::new();
    let batch = prepare_property_effect_batch(&fixture, &binding, true, &planning_cancel);
    fixture.actor.preview_prepared_apply(&batch).unwrap();
    let token = fixture
        .actor
        .save_prepared_apply(
            batch,
            unica_execution_contract::invocation::DomainResult::success("preview"),
        )
        .unwrap();
    planning_cancel.cancel();
    let source_before = snapshot_tree(&fixture.roots[0]);
    let cache_before = snapshot_tree(&fixture.root.join(".build/unica"));
    let cancellation = CancellationToken::new();
    let lane = fixture
        .actor
        .mutation_lane
        .acquire_before(ProviderDeadline::no_deadline(), &cancellation, "test lane")
        .unwrap();
    let page = fixture
        .actor
        .read_saved_apply_diff(&token, 100, None, &cancellation, renderer)
        .unwrap();
    assert!(!page["chunks"].as_array().unwrap().is_empty());
    for chunk in page["chunks"].as_array().unwrap() {
        let path = chunk["path"].as_str().unwrap();
        assert!(!Path::new(path).is_absolute());
        assert!(!path.contains(".build"));
        assert!(!path.contains("state.json"));
    }
    drop(lane);
    let entry = fixture.actor.saved_apply_plans.lock().unwrap().entries[&token].clone();
    let state = entry.state.lock().unwrap();
    assert!(matches!(
        fixture
            .actor
            .read_saved_apply_diff(&token, 100, None, &cancellation, renderer),
        Err(super::SavedApplyExecutionError::Busy)
    ));
    drop(state);
    let cancelled = CancellationToken::new();
    cancelled.cancel();
    assert!(
        matches!(fixture.actor.read_saved_apply_diff(&token, 100, None, &cancelled, renderer), Err(super::SavedApplyExecutionError::Publication(error)) if error.kind() == super::ApplyPublicationErrorKind::Cancelled)
    );
    assert_eq!(
        fixture
            .actor
            .read_saved_apply_diff(&token, 100, None, &cancellation, renderer)
            .unwrap(),
        page
    );
    assert_eq!(snapshot_tree(&fixture.roots[0]), source_before);
    assert_eq!(
        snapshot_tree(&fixture.root.join(".build/unica")),
        cache_before
    );
    let result = fixture
        .actor
        .execute_saved_apply(
            &token,
            ProviderDeadline::no_deadline(),
            &cancellation,
            finish_saved_apply_test,
        )
        .unwrap();
    assert!(result.ok, "{}", result.summary);
    assert!(fixture
        .actor
        .read_saved_apply_diff(&token, 100, None, &cancellation, renderer)
        .is_err());
    fixture.cleanup();
}

pub fn saved_apply_executes_exact_preview_after_old_invocation_stops_and_replays_once(
    policies: crate::policies::WorkspacePolicies,
    planner: EventPlanner,
) {
    let _policy_scope = policies_scope(policies);
    let _planner_scope = planner_scope(planner);

    let fixture = actor_fixture("saved-plan-delayed-execution", &["main"]);
    write_actor_event_fixture(&fixture.roots[0]);
    let binding = fixture
        .actor
        .bind_provider_root("main", &fixture.roots[0])
        .unwrap();
    let cancellation = CancellationToken::new();
    let mut batch = prepare_property_effect_batch(&fixture, &binding, true, &cancellation);
    let before = snapshot_tree(&fixture.roots[0]);
    let preview = fixture.actor.preview_prepared_apply(&batch).unwrap();
    assert_eq!(
        preview.effects().disposition(),
        super::ApplyEffectDisposition::Projected
    );
    assert_eq!(snapshot_tree(&fixture.roots[0]), before);
    // Retained bytes and capabilities survive completion/cancellation of the
    // planning invocation; only the executing invocation owns its time budget.
    cancellation.cancel();
    batch.deadline = ProviderDeadline::from_budget(Duration::ZERO);
    let token = fixture
        .actor
        .save_prepared_apply(
            batch,
            unica_execution_contract::invocation::DomainResult::success("preview"),
        )
        .unwrap();
    let executions = std::sync::atomic::AtomicUsize::new(0);
    let execution_cancellation = CancellationToken::new();
    let result = fixture
        .actor
        .execute_saved_apply(
            &token,
            ProviderDeadline::from_budget(Duration::from_secs(5)),
            &execution_cancellation,
            |result, publication| {
                assert_eq!(
                    execution_cancellation.factual_outcome_authority(),
                    FactualOutcomeAuthority::CommittedWorkspaceApply
                );
                assert!(!execution_cancellation.protected_process_started());
                executions.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                finish_saved_apply_test(result, publication)
            },
        )
        .unwrap();
    assert!(result.ok, "{}", result.summary);
    assert_eq!(result.data.as_ref().unwrap()["commits"], 1);
    let published = snapshot_tree(&fixture.roots[0]);
    assert_ne!(published, before);
    let replay_cancellation = CancellationToken::new();
    let replay = fixture
        .actor
        .execute_saved_apply(
            &token,
            ProviderDeadline::from_budget(Duration::from_secs(5)),
            &replay_cancellation,
            |_, _| panic!("a token may not publish twice"),
        )
        .unwrap();
    assert_eq!(
        replay_cancellation.factual_outcome_authority(),
        FactualOutcomeAuthority::CommittedWorkspaceApply
    );
    assert!(!replay_cancellation.protected_process_started());
    assert_eq!(replay, result);
    assert_eq!(snapshot_tree(&fixture.roots[0]), published);
    assert_eq!(executions.load(std::sync::atomic::Ordering::SeqCst), 1);
    fixture.cleanup();
}

pub fn saved_apply_stale_sources_refuse_without_replanning_and_replay_refusal(
    policies: crate::policies::WorkspacePolicies,
    planner: EventPlanner,
) {
    let _policy_scope = policies_scope(policies);
    let _planner_scope = planner_scope(planner);

    let fixture = actor_fixture("saved-plan-stale-sources", &["main"]);
    write_actor_event_fixture(&fixture.roots[0]);
    let binding = fixture
        .actor
        .bind_provider_root("main", &fixture.roots[0])
        .unwrap();
    let batch = prepare_property_effect_batch(&fixture, &binding, true, &CancellationToken::new());
    fixture.actor.preview_prepared_apply(&batch).unwrap();
    let token = fixture
        .actor
        .save_prepared_apply(
            batch,
            unica_execution_contract::invocation::DomainResult::success("preview"),
        )
        .unwrap();
    let form = fixture.roots[0].join("Catalogs/Products/Forms/Main/Ext/Form.xml");
    let mut changed = std::fs::read(&form).unwrap();
    changed.extend_from_slice(b"\n");
    std::fs::write(&form, changed).unwrap();
    let before = snapshot_tree(&fixture.roots[0]);
    let cache_before = snapshot_tree(&fixture.root.join(".build/unica"));
    let result = fixture
        .actor
        .execute_saved_apply(
            &token,
            ProviderDeadline::from_budget(Duration::from_secs(5)),
            &CancellationToken::new(),
            finish_saved_apply_test,
        )
        .unwrap();
    assert!(!result.ok, "changed sources must invalidate saved bytes");
    assert!(result.summary.contains("Form.xml"), "{}", result.summary);
    assert!(
        result.summary.contains("source write target"),
        "{}",
        result.summary
    );
    assert!(
        result.summary.contains("prevent overwriting"),
        "{}",
        result.summary
    );
    assert_eq!(snapshot_tree(&fixture.roots[0]), before);
    assert_eq!(
        snapshot_tree(&fixture.root.join(".build/unica")),
        cache_before
    );
    let replay = fixture
        .actor
        .execute_saved_apply(
            &token,
            ProviderDeadline::from_budget(Duration::from_secs(5)),
            &CancellationToken::new(),
            |_, _| panic!("a refused plan must not be replanned"),
        )
        .unwrap();
    assert_eq!(replay, result);
    fixture.cleanup();
}

pub fn saved_apply_rechecks_same_inode_same_length_hash_before_first_write(
    policies: crate::policies::WorkspacePolicies,
    planner: EventPlanner,
) {
    let _policy_scope = policies_scope(policies);
    let _planner_scope = planner_scope(planner);

    use crate::compile_transaction::take_retained_apply_observed_events;
    use crate::platform::filesystem::file_identity;

    let fixture = actor_fixture("saved-plan-same-inode-hash", &["main"]);
    write_actor_event_fixture(&fixture.roots[0]);
    let binding = fixture
        .actor
        .bind_provider_root("main", &fixture.roots[0])
        .unwrap();
    let batch = prepare_property_effect_batch(&fixture, &binding, true, &CancellationToken::new());
    fixture.actor.preview_prepared_apply(&batch).unwrap();
    let token = fixture
        .actor
        .save_prepared_apply(
            batch,
            unica_execution_contract::invocation::DomainResult::success("preview"),
        )
        .unwrap();
    let form = fixture.roots[0].join("Catalogs/Products/Forms/Main/Ext/Form.xml");
    let before = std::fs::read(&form).unwrap();
    let identity_before = file_identity(&std::fs::File::open(&form).unwrap()).unwrap();
    let mut changed = before.clone();
    changed[0] ^= 1;
    std::fs::write(&form, &changed).unwrap();
    let identity_after = file_identity(&std::fs::File::open(&form).unwrap()).unwrap();
    assert_eq!(
        identity_after, identity_before,
        "правка должна сохранить inode"
    );
    assert_eq!(changed.len(), before.len(), "правка должна сохранить длину");
    assert_ne!(changed, before, "правка должна изменить байты");

    let source_before = snapshot_tree(&fixture.roots[0]);
    let cache_before = snapshot_tree(&fixture.root.join(".build/unica"));
    let _ = take_retained_apply_observed_events();
    let refusal_kind = std::cell::Cell::new(None);
    let result = fixture
        .actor
        .execute_saved_apply(
            &token,
            ProviderDeadline::from_budget(Duration::from_secs(5)),
            &CancellationToken::new(),
            |result, publication| {
                refusal_kind.set(publication.as_ref().err().map(|error| error.kind()));
                finish_saved_apply_test(result, publication)
            },
        )
        .unwrap();

    assert!(
        !result.ok,
        "изменённый хеш должен устарить сохранённый план"
    );
    assert_eq!(
        refusal_kind.get(),
        Some(super::ApplyPublicationErrorKind::ConcurrentRevision)
    );
    assert!(result.summary.contains("Form.xml"), "{}", result.summary);
    assert!(
        result.summary.contains("source write target"),
        "{}",
        result.summary
    );
    assert!(
        take_retained_apply_observed_events().is_empty(),
        "проверка хеша должна завершиться до первой записи исходников или кеша"
    );
    assert_eq!(snapshot_tree(&fixture.roots[0]), source_before);
    assert_eq!(
        snapshot_tree(&fixture.root.join(".build/unica")),
        cache_before
    );
    fixture.cleanup();
}

pub fn saved_apply_refuses_a_disappeared_write_target_without_writing_cache(
    policies: crate::policies::WorkspacePolicies,
    planner: EventPlanner,
) {
    let _policy_scope = policies_scope(policies);
    let _planner_scope = planner_scope(planner);

    let fixture = actor_fixture("saved-plan-missing-write-target", &["main"]);
    write_actor_event_fixture(&fixture.roots[0]);
    let binding = fixture
        .actor
        .bind_provider_root("main", &fixture.roots[0])
        .unwrap();
    let batch = prepare_property_effect_batch(&fixture, &binding, true, &CancellationToken::new());
    fixture.actor.preview_prepared_apply(&batch).unwrap();
    let token = fixture
        .actor
        .save_prepared_apply(
            batch,
            unica_execution_contract::invocation::DomainResult::success("preview"),
        )
        .unwrap();
    let form = fixture.roots[0].join("Catalogs/Products/Forms/Main/Ext/Form.xml");
    std::fs::remove_file(&form).unwrap();
    let source_before = snapshot_tree(&fixture.roots[0]);
    let cache_before = snapshot_tree(&fixture.root.join(".build/unica"));

    let result = fixture
        .actor
        .execute_saved_apply(
            &token,
            ProviderDeadline::from_budget(Duration::from_secs(5)),
            &CancellationToken::new(),
            finish_saved_apply_test,
        )
        .unwrap();

    assert!(!result.ok, "исчезнувшая цель должна устарить план");
    assert!(result.summary.contains("Form.xml"), "{}", result.summary);
    assert_eq!(snapshot_tree(&fixture.roots[0]), source_before);
    assert_eq!(
        snapshot_tree(&fixture.root.join(".build/unica")),
        cache_before
    );
    fixture.cleanup();
}

pub fn saved_apply_wait_preserves_cancelled_and_deadline_errors_without_consuming_plan(
    policies: crate::policies::WorkspacePolicies,
    planner: EventPlanner,
) {
    let _policy_scope = policies_scope(policies);
    let _planner_scope = planner_scope(planner);

    for workspace_lane in [false, true] {
        let fixture = actor_fixture("saved-plan-wait-stop", &["main"]);
        write_actor_event_fixture(&fixture.roots[0]);
        let binding = fixture
            .actor
            .bind_provider_root("main", &fixture.roots[0])
            .unwrap();
        let batch =
            prepare_property_effect_batch(&fixture, &binding, true, &CancellationToken::new());
        let token = fixture
            .actor
            .save_prepared_apply(
                batch,
                unica_execution_contract::invocation::DomainResult::success("preview"),
            )
            .unwrap();
        let entry = fixture.actor.saved_apply_plans.lock().unwrap().entries[&token].clone();
        let lane = if workspace_lane {
            fixture.actor.mutation_lane.hold_for_test()
        } else {
            entry.execution_lane.hold_for_test()
        };
        let before = snapshot_tree(&fixture.roots[0]);
        let cache_before = snapshot_tree(&fixture.root.join(".build/unica"));
        let deadline = fixture
            .actor
            .execute_saved_apply(
                &token,
                ProviderDeadline::from_budget(Duration::from_millis(20)),
                &CancellationToken::new(),
                |_, _| panic!("waiting execution may not consume the plan"),
            )
            .unwrap_err();
        assert!(
            matches!(deadline, super::SavedApplyExecutionError::Publication(error) if error.kind() == super::ApplyPublicationErrorKind::Deadline)
        );
        let cancellation = CancellationToken::new();
        std::thread::scope(|scope| {
            let (started, waiting) = std::sync::mpsc::channel();
            let actor = &fixture.actor;
            let token = &token;
            let cancellation_for_worker = &cancellation;
            let worker = scope.spawn(move || {
                started.send(()).unwrap();
                actor.execute_saved_apply(
                    token,
                    ProviderDeadline::from_budget(Duration::from_secs(5)),
                    cancellation_for_worker,
                    |_, _| panic!("cancelled waiting execution may not consume the plan"),
                )
            });
            waiting.recv().unwrap();
            if workspace_lane {
                // State is held only after the per-token lane has been acquired;
                // cancellation therefore stops the workspace-lane wait.
                let wait_until = std::time::Instant::now() + Duration::from_secs(5);
                loop {
                    match entry.state.try_lock() {
                        Err(std::sync::TryLockError::WouldBlock) => break,
                        Ok(guard) => drop(guard),
                        Err(error) => panic!("saved execution state poisoned: {error}"),
                    }
                    assert!(std::time::Instant::now() < wait_until);
                    std::thread::yield_now();
                }
            }
            cancellation.cancel();
            let cancelled = worker.join().unwrap().unwrap_err();
            assert!(
                matches!(cancelled, super::SavedApplyExecutionError::Publication(error) if error.kind() == super::ApplyPublicationErrorKind::Cancelled)
            );
        });
        assert_eq!(snapshot_tree(&fixture.roots[0]), before);
        assert_eq!(
            snapshot_tree(&fixture.root.join(".build/unica")),
            cache_before
        );
        drop(lane);
        let published = fixture
            .actor
            .execute_saved_apply(
                &token,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
                finish_saved_apply_test,
            )
            .unwrap();
        assert!(
            published.ok,
            "waiting stop must leave the pending plan usable"
        );
        let _publication_lane = fixture.actor.mutation_lane.hold_for_test();
        let replay = fixture
            .actor
            .execute_saved_apply(
                &token,
                ProviderDeadline::from_budget(Duration::from_millis(20)),
                &CancellationToken::new(),
                |_, _| panic!("completed replay must not wait for workspace publication"),
            )
            .unwrap();
        assert_eq!(replay, published);
        drop(_publication_lane);
        fixture.cleanup();
    }
}

pub fn saved_apply_execution_poison_is_internal_failure_and_writes_nothing(
    policies: crate::policies::WorkspacePolicies,
    planner: EventPlanner,
) {
    let _policy_scope = policies_scope(policies);
    let _planner_scope = planner_scope(planner);

    for poison_registry in [true, false] {
        let fixture = actor_fixture("saved-plan-execute-poison", &["main"]);
        write_actor_event_fixture(&fixture.roots[0]);
        let binding = fixture
            .actor
            .bind_provider_root("main", &fixture.roots[0])
            .unwrap();
        let batch =
            prepare_property_effect_batch(&fixture, &binding, true, &CancellationToken::new());
        fixture.actor.preview_prepared_apply(&batch).unwrap();
        let token = fixture
            .actor
            .save_prepared_apply(
                batch,
                unica_execution_contract::invocation::DomainResult::success("preview"),
            )
            .unwrap();
        let entry = fixture.actor.saved_apply_plans.lock().unwrap().entries[&token].clone();
        let source_before = snapshot_tree(&fixture.roots[0]);
        let cache_before = snapshot_tree(&fixture.root.join(".build/unica"));
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            if poison_registry {
                let _registry = fixture.actor.saved_apply_plans.lock().unwrap();
                panic!("poison saved apply registry");
            } else {
                let _state = entry.state.lock().unwrap();
                panic!("poison saved apply state");
            }
        }))
        .is_err());
        let error = fixture
            .actor
            .execute_saved_apply(
                &token,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
                |_, _| panic!("poisoned saved plan must not reach publication"),
            )
            .unwrap_err();
        assert!(matches!(
            (poison_registry, error),
            (true, super::SavedApplyExecutionError::RegistryUnavailable)
                | (false, super::SavedApplyExecutionError::StateUnavailable(_))
        ));
        assert_eq!(snapshot_tree(&fixture.roots[0]), source_before);
        assert_eq!(
            snapshot_tree(&fixture.root.join(".build/unica")),
            cache_before
        );
        fixture.cleanup();
    }
}

pub fn saved_apply_plan_poison_is_registry_state_failure_and_writes_nothing(
    policies: crate::policies::WorkspacePolicies,
    planner: EventPlanner,
) {
    let _policy_scope = policies_scope(policies);
    let _planner_scope = planner_scope(planner);

    let fixture = actor_fixture("saved-plan-registry-poison", &["main"]);
    write_actor_event_fixture(&fixture.roots[0]);
    let binding = fixture
        .actor
        .bind_provider_root("main", &fixture.roots[0])
        .unwrap();
    let batch = prepare_property_effect_batch(&fixture, &binding, true, &CancellationToken::new());
    fixture.actor.preview_prepared_apply(&batch).unwrap();
    let source_before = snapshot_tree(&fixture.roots[0]);
    let cache_before = snapshot_tree(&fixture.root.join(".build/unica"));
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _registry = fixture.actor.saved_apply_plans.lock().unwrap();
        panic!("poison saved apply registry for fail-closed check");
    }))
    .is_err());
    let error = fixture
        .actor
        .save_prepared_apply(
            batch,
            unica_execution_contract::invocation::DomainResult::success("preview"),
        )
        .unwrap_err();
    assert_eq!(error, super::SavedApplyPlanError::RegistryUnavailable);
    assert_eq!(snapshot_tree(&fixture.roots[0]), source_before);
    assert_eq!(
        snapshot_tree(&fixture.root.join(".build/unica")),
        cache_before
    );
    fixture.cleanup();
}

pub fn saved_apply_plan_actor_mismatch_refuses_without_writes(
    policies: crate::policies::WorkspacePolicies,
    planner: EventPlanner,
) {
    let _policy_scope = policies_scope(policies);
    let _planner_scope = planner_scope(planner);

    let fixture = actor_fixture("saved-plan-save-bound", &["main"]);
    let other = actor_fixture("saved-plan-save-foreign", &["main"]);
    write_actor_event_fixture(&fixture.roots[0]);
    let binding = fixture
        .actor
        .bind_provider_root("main", &fixture.roots[0])
        .unwrap();
    let batch = prepare_property_effect_batch(&fixture, &binding, true, &CancellationToken::new());
    let source_before = snapshot_tree(&fixture.roots[0]);
    let error = other
        .actor
        .save_prepared_apply(
            batch,
            unica_execution_contract::invocation::DomainResult::success("preview"),
        )
        .unwrap_err();
    assert_eq!(error, super::SavedApplyPlanError::ActorMismatch);
    assert!(other
        .actor
        .saved_apply_plans
        .lock()
        .unwrap()
        .entries
        .is_empty());
    assert_eq!(snapshot_tree(&fixture.roots[0]), source_before);
    fixture.cleanup();
    other.cleanup();
}

pub fn saved_apply_entries_survive_later_plans_after_executor_releases_entry(
    policies: crate::policies::WorkspacePolicies,
    planner: EventPlanner,
) {
    let _policy_scope = policies_scope(policies);
    let _planner_scope = planner_scope(planner);

    let fixture = actor_fixture("saved-plan-retained-entries", &["main"]);
    write_actor_event_fixture(&fixture.roots[0]);
    let binding = fixture
        .actor
        .bind_provider_root("main", &fixture.roots[0])
        .unwrap();
    let prepare =
        || prepare_property_effect_batch(&fixture, &binding, true, &CancellationToken::new());
    let preview = unica_execution_contract::invocation::DomainResult::success("preview");
    let token = fixture
        .actor
        .save_prepared_apply(prepare(), preview.clone())
        .unwrap();
    let retained = fixture.actor.saved_apply_plans.lock().unwrap().entries[&token].clone();
    let source_before = snapshot_tree(&fixture.roots[0]);
    let cache_before = snapshot_tree(&fixture.root.join(".build/unica"));
    let next = fixture
        .actor
        .save_prepared_apply(prepare(), preview.clone())
        .unwrap();
    drop(retained);
    let newest = fixture
        .actor
        .save_prepared_apply(prepare(), preview)
        .unwrap();
    {
        let plans = fixture.actor.saved_apply_plans.lock().unwrap();
        for issued in [&token, &next, &newest] {
            assert!(plans.entries.contains_key(issued));
        }
    }
    assert_eq!(snapshot_tree(&fixture.roots[0]), source_before);
    assert_eq!(
        snapshot_tree(&fixture.root.join(".build/unica")),
        cache_before
    );
    let result = fixture
        .actor
        .execute_saved_apply(
            &token,
            ProviderDeadline::from_budget(Duration::from_secs(5)),
            &CancellationToken::new(),
            finish_saved_apply_test,
        )
        .unwrap();
    assert!(result.ok, "the original issued plan was lost: {result:?}");
    fixture.cleanup();
}

pub fn saved_apply_token_is_actor_bound_and_foreign_execution_has_no_effect(
    policies: crate::policies::WorkspacePolicies,
    planner: EventPlanner,
) {
    let _policy_scope = policies_scope(policies);
    let _planner_scope = planner_scope(planner);

    let fixture = actor_fixture("saved-plan-bound-token", &["main"]);
    let other = actor_fixture("saved-plan-other-workspace", &["main"]);
    write_actor_event_fixture(&fixture.roots[0]);
    let binding = fixture
        .actor
        .bind_provider_root("main", &fixture.roots[0])
        .unwrap();
    let batch = prepare_property_effect_batch(&fixture, &binding, true, &CancellationToken::new());
    fixture.actor.preview_prepared_apply(&batch).unwrap();
    let token = fixture
        .actor
        .save_prepared_apply(
            batch,
            unica_execution_contract::invocation::DomainResult::success("preview"),
        )
        .unwrap();
    let before = snapshot_tree(&fixture.roots[0]);
    assert!(other
        .actor
        .execute_saved_apply(
            &token,
            ProviderDeadline::from_budget(Duration::from_secs(5)),
            &CancellationToken::new(),
            |_, _| panic!("foreign token cannot execute")
        )
        .is_err());
    assert_eq!(snapshot_tree(&fixture.roots[0]), before);
    let result = fixture
        .actor
        .execute_saved_apply(
            &token,
            ProviderDeadline::from_budget(Duration::from_secs(5)),
            &CancellationToken::new(),
            finish_saved_apply_test,
        )
        .unwrap();
    assert!(
        result.ok,
        "the owner's plan became unusable after foreign refusal: {result:?}"
    );
    fixture.cleanup();
    other.cleanup();
}

pub fn apply_policy_foreign_actor_and_sibling_worktree_replay_are_rejected(
    policies: crate::policies::WorkspacePolicies,
    planner: EventPlanner,
) {
    let _policy_scope = policies_scope(policies.clone());
    let _planner_scope = planner_scope(planner);

    real_effect_foreign_actor_replay_preserves_both_actor_states(policies, planner);

    let fixture = actor_fixture("apply-policy-sibling-source", &["one", "two"]);
    let target = fixture.roots[0].join("Module.bsl");
    std::fs::write(&target, b"original").unwrap();
    let binding = fixture
        .actor
        .bind_provider_root("one", &fixture.roots[0])
        .unwrap();
    let admission = fixture
        .actor
        .admit_apply(
            &binding,
            None,
            true,
            ProviderDeadline::from_budget(Duration::from_secs(5)),
            &CancellationToken::new(),
        )
        .unwrap();
    let mut state = admission.staged_state().unwrap();
    state
        .replace("Module.bsl", b"original", b"projected".to_vec())
        .unwrap();
    let mut prepared = admission.prepare(state).unwrap();
    prepared.source_set = fixture
        .actor
        .identity
        .source_sets
        .iter()
        .find(|source| source.name == "two")
        .unwrap()
        .clone();

    let error = fixture.actor.publish_prepared_apply(prepared).unwrap_err();

    assert_eq!(
        error.kind(),
        super::ApplyPublicationErrorKind::ContainmentIdentity
    );
    assert_eq!(std::fs::read(&target).unwrap(), b"original");
    fixture.cleanup();
}
