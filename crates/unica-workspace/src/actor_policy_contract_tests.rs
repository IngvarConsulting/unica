use super::test_support::*;
use super::*;
use crate::platform::testing::*;
use crate::policy_evidence::*;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::thread;
use unica_source_contract::events::DomainEventKind;

pub fn workspace_actor_poisoned_mutation_lane_fails_closed_before_publication(
    policies: crate::policies::WorkspacePolicies,
) {
    let _policy_scope = policies_scope(policies);

    let fixture = actor_fixture("poisoned-mutation-lane", &["src"]);
    std::fs::write(fixture.roots[0].join("Module.bsl"), b"before").unwrap();
    let binding = fixture
        .actor
        .bind_provider_root("src", &fixture.roots[0])
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
    let mut state = admitted.staged_state().unwrap();
    state
        .replace("Module.bsl", b"before", b"POISONED-STAGED-TEXT".to_vec())
        .unwrap();
    let prepared = admitted.prepare(state).unwrap();
    let poison_actor = Arc::clone(&fixture.actor);
    let poison = thread::spawn(move || {
        let _lane = poison_actor.mutation_lane.hold_for_test();
        panic!("poison workspace actor mutation lane");
    });
    assert!(poison.join().is_err());
    let source_before = snapshot_tree(&fixture.roots[0]);

    let error = fixture.actor.publish_prepared_apply(prepared).unwrap_err();

    assert_eq!(error.kind(), super::ApplyPublicationErrorKind::Invariant);
    assert!(error.contains("poisoned"), "{error}");
    assert!(!error.contains("POISONED-STAGED-TEXT"), "{error}");
    assert_eq!(snapshot_tree(&fixture.roots[0]), source_before);
    fixture.cleanup();
}

pub fn prepared_apply_lock_deadline_is_not_reclassified_by_later_cancellation(
    policies: crate::policies::WorkspacePolicies,
) {
    let _policy_scope = policies_scope(policies);

    let fixture = actor_fixture("typed-deadline-lock-race", &["src"]);
    std::fs::write(fixture.roots[0].join("Module.bsl"), b"before").unwrap();
    let binding = fixture
        .actor
        .bind_provider_root("src", &fixture.roots[0])
        .unwrap();
    let cancellation = CancellationToken::new();
    let admitted = fixture
        .actor
        .admit_apply(
            &binding,
            None,
            false,
            ProviderDeadline::from_budget(Duration::from_secs(5)),
            &cancellation,
        )
        .unwrap();
    let mut state = admitted.staged_state().unwrap();
    state
        .replace("Module.bsl", b"before", b"after".to_vec())
        .unwrap();
    let mut prepared = admitted.prepare(state).unwrap();
    prepared.deadline = ProviderDeadline::from_budget(Duration::from_millis(40));
    let owner = fixture.actor.mutation_lane.hold_for_test();
    let cancel_after_deadline = cancellation.clone();
    crate::deadline_lock::set_after_deadline_error_hook_for_test(move || {
        std::thread::spawn(move || cancel_after_deadline.cancel())
            .join()
            .unwrap();
    });

    let error = fixture.actor.publish_prepared_apply(prepared).unwrap_err();

    assert_eq!(error.kind(), super::ApplyPublicationErrorKind::Deadline);
    drop(owner);
    fixture.cleanup();
}

pub fn capabilities_do_not_cross_distinct_actor_instances_with_equal_identity(
    policies: crate::policies::WorkspacePolicies,
) {
    let _policy_scope = policies_scope(policies);

    let root = temp_root("actor-instance-capability");
    let source = root.join("src");
    std::fs::create_dir_all(&source).unwrap();
    let context = context(&root);
    let identity =
        WorkspaceIdentity::new(&context, [source_input("main", &source)], "profile").unwrap();
    let first = super::WorkspaceActor::<(), ()>::new(identity.clone(), context.clone())
        .map(bind_actor)
        .unwrap();
    let second = super::WorkspaceActor::<(), ()>::new(identity, context)
        .map(bind_actor)
        .unwrap();
    let binding = first.bind_provider_root("main", &source).unwrap();

    assert!(second.read(&binding, |_| Ok(())).is_err());
    assert!(second.validate_binding(&binding).is_err());
    assert!(second
        .admit_apply(
            &binding,
            None,
            true,
            ProviderDeadline::from_budget(Duration::from_secs(5)),
            &CancellationToken::new(),
        )
        .is_err());
    assert!(second.provider_source_lease(&binding).is_err());
    assert!(first.validate_binding(&binding).is_ok());
    let _ = std::fs::remove_dir_all(root);
}

pub fn workspace_actor_capabilities_enforce_identity_physical_and_bounded_publication(
    policies: crate::policies::WorkspacePolicies,
) {
    let _policy_scope = policies_scope(policies);

    use crate::platform::testing::{
        create_directory_link_fixture_for_test, FileLinkFixtureOutcome,
    };

    let root = temp_root("active-capability-contract");
    let source = root.join("src");
    let outside = root.join("outside");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(source.join("Module.bsl"), "test").unwrap();
    std::fs::write(outside.join("Secret.bsl"), "outside").unwrap();
    let context = context(&root);
    assert!(WorkspaceIdentity::new(
        &context,
        [
            source_input("main", &source),
            source_input("alias", &source)
        ],
        "profile",
    )
    .is_err());
    let identity =
        WorkspaceIdentity::new(&context, [source_input("main", &source)], "profile").unwrap();
    let first = super::WorkspaceActor::<(), ()>::new(identity.clone(), context.clone())
        .map(bind_actor)
        .unwrap();
    let second = super::WorkspaceActor::<(), ()>::new(identity, context)
        .map(bind_actor)
        .unwrap();
    let binding = first.bind_provider_root("main", &source).unwrap();

    assert!(second.read(&binding, |_| Ok(())).is_err());
    assert!(second.validate_binding(&binding).is_err());

    let nested = source.join("Nested");
    if matches!(
        create_directory_link_fixture_for_test(&outside, &nested).unwrap(),
        FileLinkFixtureOutcome::Created
    ) {
        assert!(first
            .read_relative_file(&binding, Path::new("Nested/Secret.bsl"), 1024)
            .is_err());
    }

    let displaced = root.join("src-displaced");
    let replacement = source.clone();
    let external = first.read(&binding, |root| {
        let output = std::fs::read(root.join("Module.bsl")).map_err(|error| error.to_string())?;
        std::fs::rename(root, &displaced).map_err(|error| error.to_string())?;
        std::fs::create_dir_all(&replacement).map_err(|error| error.to_string())?;
        Ok(output)
    });
    assert!(external.is_err());
    assert!(first.validate_binding(&binding).is_err());
    assert!(first
        .admit_apply(
            &binding,
            None,
            true,
            ProviderDeadline::from_budget(Duration::from_secs(5)),
            &CancellationToken::new(),
        )
        .is_err());
    let _ = std::fs::remove_dir_all(root);
}

pub fn borrow_dependency_revision_rejects_parent_change_since_preview(
    policies: crate::policies::WorkspacePolicies,
) {
    let _policy_scope = policies_scope(policies);

    let fixture = actor_fixture("borrow-preview-parent-fence", &["dst", "parent", "other"]);
    let binding = fixture
        .actor
        .bind_provider_root("dst", &fixture.roots[0])
        .unwrap();
    let parent = fixture
        .actor
        .bind_provider_root("parent", &fixture.roots[1])
        .unwrap();
    let other = fixture
        .actor
        .bind_provider_root("other", &fixture.roots[2])
        .unwrap();
    let path = fixture.roots[1].join("Configuration.xml");
    std::fs::write(&path, b"parent before").unwrap();
    let deadline = ProviderDeadline::from_budget(Duration::from_secs(10));
    let cancellation = CancellationToken::new();
    let admit = |parents: &[super::ProviderRootBinding], rev: Option<&str>| {
        fixture.actor.admit_apply_with_dependencies(
            &binding,
            parents,
            rev,
            true,
            deadline,
            &cancellation,
        )
    };
    let plan = |parents: &[super::ProviderRootBinding], expected: Option<&str>| {
        let admission = admit(parents, expected).unwrap();
        admission
            .read_apply_dependency("parent", Path::new("Configuration.xml"))
            .unwrap();
        let state = admission.staged_state().unwrap();
        admission.prepare(state)
    };
    let before = fixture
        .actor
        .publish_prepared_apply(plan(&[parent.clone(), other.clone()], None).unwrap())
        .unwrap();
    let reordered = fixture
        .actor
        .publish_prepared_apply(
            plan(&[other.clone(), parent.clone(), parent.clone()], None).unwrap(),
        )
        .unwrap();
    assert_eq!(
        before.rev(),
        reordered.rev(),
        "dependency order and duplicates must not alter the plan"
    );
    std::fs::write(&path, b"parent after").unwrap();
    assert_eq!(
        plan(&[parent, other], Some(before.rev()))
            .unwrap_err()
            .kind(),
        ApplyStagingErrorKind::ConcurrentRevision
    );
    assert!(
        !fixture.actor.context().cache_root.exists(),
        "preview admission must not publish revision caches"
    );
    fixture.cleanup();
}

pub fn borrow_dependency_change_before_publication_refuses_preview_and_commit(
    policies: crate::policies::WorkspacePolicies,
) {
    let _policy_scope = policies_scope(policies);

    for dry_run in [true, false] {
        let fixture = actor_fixture("borrow-prepublication-parent-fence", &["dst", "parent"]);
        let binding = fixture
            .actor
            .bind_provider_root("dst", &fixture.roots[0])
            .unwrap();
        let parent = fixture
            .actor
            .bind_provider_root("parent", &fixture.roots[1])
            .unwrap();
        let path = fixture.roots[1].join("Configuration.xml");
        std::fs::write(&path, b"parent before").unwrap();
        let admission = fixture
            .actor
            .admit_apply_with_dependencies(
                &binding,
                &[parent],
                None,
                dry_run,
                ProviderDeadline::from_budget(Duration::from_secs(10)),
                &CancellationToken::new(),
            )
            .unwrap();
        assert_eq!(
            admission
                .read_apply_dependency("parent", Path::new("Configuration.xml"))
                .unwrap(),
            b"parent before"
        );
        let mut staged = admission.staged_state().unwrap();
        staged
            .create("Borrowed.bsl", b"borrowed result".to_vec())
            .unwrap();
        let prepared = admission
            .prepare_with_effects(staged, Default::default())
            .unwrap();
        std::fs::write(&path, b"parent after").unwrap();
        let error = fixture.actor.publish_prepared_apply(prepared).unwrap_err();
        assert_eq!(
            error.kind(),
            super::ApplyPublicationErrorKind::ConcurrentRevision
        );
        assert!(error.contains("parent:Configuration.xml"), "{error}");
        assert!(error.contains("read-only input"), "{error}");
        assert!(error.contains("prepared using"), "{error}");
        assert!(!fixture.roots[0].join("Borrowed.bsl").exists());
        assert!(!fixture.actor.context().cache_root.exists());
        fixture.cleanup();
    }
}

pub fn borrow_dependency_final_preview_confirmation_detects_late_parent_change(
    policies: crate::policies::WorkspacePolicies,
) {
    let _policy_scope = policies_scope(policies);

    let fixture = actor_fixture("borrow-final-preview-parent-fence", &["dst", "parent"]);
    let binding = fixture
        .actor
        .bind_provider_root("dst", &fixture.roots[0])
        .unwrap();
    let parent = fixture
        .actor
        .bind_provider_root("parent", &fixture.roots[1])
        .unwrap();
    let path = fixture.roots[1].join("Configuration.xml");
    std::fs::write(&path, b"parent before").unwrap();
    let admission = fixture
        .actor
        .admit_apply_with_dependencies(
            &binding,
            &[parent],
            None,
            true,
            ProviderDeadline::from_budget(Duration::from_secs(10)),
            &CancellationToken::new(),
        )
        .unwrap();
    admission
        .read_apply_dependency("parent", Path::new("Configuration.xml"))
        .unwrap();
    let staged = admission.staged_state().unwrap();
    let prepared = admission
        .prepare_with_effects(staged, Default::default())
        .unwrap();
    set_apply_dry_run_after_confirmation_hook(move || {
        std::fs::write(path, b"parent after").unwrap();
    });
    assert!(fixture.actor.publish_prepared_apply(prepared).is_err());
    assert!(!fixture.actor.context().cache_root.exists());
    fixture.cleanup();
}

pub fn borrow_dependency_changed_after_destination_write_rolls_back_destination_only(
    policies: crate::policies::WorkspacePolicies,
) {
    let _policy_scope = policies_scope(policies);

    let fixture = actor_fixture("borrow-after-write-parent-fence", &["dst", "parent"]);
    let binding = fixture
        .actor
        .bind_provider_root("dst", &fixture.roots[0])
        .unwrap();
    let parent = fixture
        .actor
        .bind_provider_root("parent", &fixture.roots[1])
        .unwrap();
    let parent_path = fixture.roots[1].join("Module.bsl");
    let destination_path = fixture.roots[0].join("Module.bsl");
    std::fs::write(&parent_path, b"parent before").unwrap();
    std::fs::write(&destination_path, b"destination before").unwrap();
    let admission = fixture
        .actor
        .admit_apply_with_dependencies(
            &binding,
            &[parent],
            None,
            false,
            ProviderDeadline::from_budget(Duration::from_secs(10)),
            &CancellationToken::new(),
        )
        .unwrap();
    admission
        .read_apply_dependency("parent", Path::new("Module.bsl"))
        .unwrap();
    let mut staged = admission.staged_state().unwrap();
    let before = staged.read(Path::new("Module.bsl")).unwrap().unwrap();
    staged
        .replace("Module.bsl", &before, b"destination after".to_vec())
        .unwrap();
    let prepared = admission
        .prepare_with_effects(staged, Default::default())
        .unwrap();
    let changed_parent = parent_path.clone();
    let published_destination = destination_path.clone();
    crate::compile_transaction::set_retained_apply_before_post_validation_hook(move || {
        assert_eq!(
            std::fs::read(&published_destination).unwrap(),
            b"destination after",
            "race must happen after the real source write"
        );
        std::fs::write(&changed_parent, b"parent concurrent edit").unwrap();
    });
    assert!(fixture.actor.publish_prepared_apply(prepared).is_err());
    assert_eq!(
        std::fs::read(&destination_path).unwrap(),
        b"destination before"
    );
    assert_eq!(
        std::fs::read(&parent_path).unwrap(),
        b"parent concurrent edit"
    );
    assert!(!fixture.actor.context().cache_root.exists());
    fixture.cleanup();
}

pub fn borrow_dependency_rejects_capability_from_equal_identity_other_actor(
    policies: crate::policies::WorkspacePolicies,
) {
    let _policy_scope = policies_scope(policies);

    let fixture = actor_fixture("borrow-other-actor-parent-fence", &["dst", "parent"]);
    let binding = fixture
        .actor
        .bind_provider_root("dst", &fixture.roots[0])
        .unwrap();
    let foreign = super::WorkspaceActor::<(), ()>::new(
        fixture.actor.identity().clone(),
        fixture.actor.context().clone(),
    )
    .map(bind_actor)
    .unwrap();
    let parent = foreign
        .bind_provider_root("parent", &fixture.roots[1])
        .unwrap();
    assert!(fixture
        .actor
        .admit_apply_with_dependencies(
            &binding,
            &[parent],
            None,
            true,
            ProviderDeadline::from_budget(Duration::from_secs(10)),
            &CancellationToken::new()
        )
        .is_err());
    fixture.cleanup();
}

pub fn apply_plan_fence_is_targeted_and_binds_read_only_inputs(
    policies: crate::policies::WorkspacePolicies,
) {
    let _policy_scope = policies_scope(policies);

    let fixture = actor_fixture("targeted-plan-fence", &["src"]);
    let binding = fixture
        .actor
        .bind_provider_root("src", &fixture.roots[0])
        .unwrap();
    std::fs::write(fixture.roots[0].join("Module.bsl"), b"before").unwrap();
    std::fs::write(fixture.roots[0].join("Dependency.bsl"), b"dependency").unwrap();
    let cancellation = CancellationToken::new();
    let stage = |dry_run, expected: Option<&str>| {
        let admission = fixture
            .actor
            .admit_apply(
                &binding,
                expected,
                dry_run,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &cancellation,
            )
            .unwrap();
        let mut state = admission.staged_state().unwrap();
        state.read(Path::new("Dependency.bsl")).unwrap();
        state
            .replace("Module.bsl", b"before", b"after".to_vec())
            .unwrap();
        admission.prepare(state)
    };
    let preview = fixture
        .actor
        .publish_prepared_apply(stage(true, None).unwrap())
        .unwrap();
    assert!(preview.rev().starts_with("unica-apply-plan-v2:"));
    assert_eq!(
        std::fs::read(fixture.roots[0].join("Module.bsl")).unwrap(),
        b"before"
    );
    std::fs::write(fixture.roots[0].join("unrelated.xml"), b"unrelated").unwrap();
    let repeated = fixture
        .actor
        .publish_prepared_apply(stage(true, None).unwrap())
        .unwrap();
    assert_eq!(
        repeated.rev(),
        preview.rev(),
        "unrelated source changes must not invalidate the plan"
    );
    std::fs::write(fixture.roots[0].join("Dependency.bsl"), b"changed").unwrap();
    assert_eq!(
        stage(false, Some(preview.rev())).unwrap_err().kind(),
        ApplyStagingErrorKind::ConcurrentRevision
    );
    assert_eq!(
        std::fs::read(fixture.roots[0].join("Module.bsl")).unwrap(),
        b"before"
    );
    std::fs::write(fixture.roots[0].join("Dependency.bsl"), b"dependency").unwrap();
    let real = fixture
        .actor
        .publish_prepared_apply(stage(false, Some(preview.rev())).unwrap())
        .unwrap();
    assert_eq!(real.commit_count_for_test(), 1);
    assert_eq!(
        std::fs::read(fixture.roots[0].join("Module.bsl")).unwrap(),
        b"after"
    );
    fixture.cleanup();
}

pub fn disjoint_apply_plans_publish_without_a_global_revision_conflict(
    policies: crate::policies::WorkspacePolicies,
) {
    let _policy_scope = policies_scope(policies);

    let fixture = actor_fixture("disjoint-plan-fences", &["src"]);
    let binding = fixture
        .actor
        .bind_provider_root("src", &fixture.roots[0])
        .unwrap();
    for name in ["First.bsl", "Second.bsl"] {
        std::fs::write(fixture.roots[0].join(name), b"before").unwrap();
    }
    let cancellation = CancellationToken::new();
    let stage = |name, dry_run, expected: Option<&str>| {
        let admission = fixture
            .actor
            .admit_apply(
                &binding,
                expected,
                dry_run,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &cancellation,
            )
            .unwrap();
        let mut state = admission.staged_state().unwrap();
        state.replace(name, b"before", b"after".to_vec()).unwrap();
        admission.prepare(state).unwrap()
    };
    let first = fixture
        .actor
        .publish_prepared_apply(stage("First.bsl", true, None))
        .unwrap();
    let second = fixture
        .actor
        .publish_prepared_apply(stage("Second.bsl", true, None))
        .unwrap();
    fixture
        .actor
        .publish_prepared_apply(stage("First.bsl", false, Some(first.rev())))
        .unwrap();
    fixture
        .actor
        .publish_prepared_apply(stage("Second.bsl", false, Some(second.rev())))
        .unwrap();
    for name in ["First.bsl", "Second.bsl"] {
        assert_eq!(
            std::fs::read(fixture.roots[0].join(name)).unwrap(),
            b"after"
        );
    }
    fixture.cleanup();
}

pub fn apply_admission_rejects_source_inside_cache(policies: crate::policies::WorkspacePolicies) {
    let _policy_scope = policies_scope(policies);

    let root = temp_root("source-inside-cache");
    let source = root.join("cache/src");
    let cache = root.join("cache");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::write(
            root.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: src\n    type: CONFIGURATION\n    path: cache/src\n",
        )
        .unwrap();
    std::fs::write(
        source.join("Configuration.xml"),
        "<MetaDataObject><Configuration/></MetaDataObject>",
    )
    .unwrap();
    let mut actor_context = context(&root);
    actor_context.cache_root = cache;
    let identity = WorkspaceIdentity::new(
        &actor_context,
        [source_input("src", &source)],
        "test-provider",
    )
    .unwrap();
    let actor = super::WorkspaceActor::<(), ()>::new(identity, actor_context)
        .map(bind_actor)
        .unwrap();
    let binding = actor.bind_provider_root("src", &source).unwrap();

    let error = actor
        .admit_apply(
            &binding,
            None,
            true,
            ProviderDeadline::from_budget(Duration::from_secs(5)),
            &CancellationToken::new(),
        )
        .unwrap_err();
    assert!(error.to_string().contains("overlap"), "{error}");
    let _ = std::fs::remove_dir_all(root);
}

pub fn workspace_root_source_allows_exact_generated_cache_descendant(
    policies: crate::policies::WorkspacePolicies,
) {
    let _policy_scope = policies_scope(policies);

    let root = temp_root("workspace-root-source-cache-descendant");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: src\n    type: CONFIGURATION\n    path: .\n",
    )
    .unwrap();
    std::fs::write(
        root.join("Configuration.xml"),
        "<MetaDataObject><Configuration/></MetaDataObject>",
    )
    .unwrap();
    std::fs::write(root.join("Module.bsl"), b"source").unwrap();
    let actor_context = context(&root);
    let identity = WorkspaceIdentity::new(
        &actor_context,
        [source_input("src", &root)],
        "test-provider",
    )
    .unwrap();
    let actor = super::WorkspaceActor::<(), ()>::new(identity, actor_context)
        .map(bind_actor)
        .unwrap();
    let binding = actor.bind_provider_root("src", &root).unwrap();

    let admitted = actor
        .admit_apply(
            &binding,
            None,
            true,
            ProviderDeadline::from_budget(Duration::from_secs(5)),
            &CancellationToken::new(),
        )
        .unwrap();
    let mut state = admitted.staged_state().unwrap();
    let error = state
        .create(".build/unica/forged.json", b"forged".to_vec())
        .unwrap_err();
    assert_eq!(error.kind(), ApplyStagingErrorKind::ContainmentIdentity);
    let error = state
        .create("Nested/.build/forged.json", b"forged".to_vec())
        .unwrap_err();
    assert_eq!(error.kind(), ApplyStagingErrorKind::ContainmentIdentity);

    let mut other_context = context(&root);
    other_context.cache_root = root.join("other-cache");
    let other_identity = WorkspaceIdentity::new(
        &other_context,
        [source_input("src", &root)],
        "test-provider",
    )
    .unwrap();
    let other_actor = super::WorkspaceActor::<(), ()>::new(other_identity, other_context)
        .map(bind_actor)
        .unwrap();
    let other_binding = other_actor.bind_provider_root("src", &root).unwrap();
    let error = other_actor
        .admit_apply(
            &other_binding,
            None,
            true,
            ProviderDeadline::from_budget(Duration::from_secs(5)),
            &CancellationToken::new(),
        )
        .unwrap_err();
    assert!(error.to_string().contains("exact .build/unica"), "{error}");
    let _ = std::fs::remove_dir_all(root);
}

pub fn source_role_rejects_platform_equivalent_generated_components(
    policies: crate::policies::WorkspacePolicies,
) {
    let _policy_scope = policies_scope(policies);

    let root = temp_root("platform-equivalent-generated-source");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: src\n    type: CONFIGURATION\n    path: .\n",
    )
    .unwrap();
    std::fs::write(
        root.join("Configuration.xml"),
        "<MetaDataObject><Configuration/></MetaDataObject>",
    )
    .unwrap();
    let canonical_root = std::fs::canonicalize(&root).unwrap();
    if crate::platform::filesystem::host_filesystem_case_sensitive(&canonical_root).unwrap() {
        let _ = std::fs::remove_dir_all(root);
        return;
    }
    let actor_context = context(&root);
    let identity = WorkspaceIdentity::new(
        &actor_context,
        [source_input("src", &root)],
        "test-provider",
    )
    .unwrap();
    let actor = super::WorkspaceActor::<(), ()>::new(identity, actor_context)
        .map(bind_actor)
        .unwrap();
    let binding = actor.bind_provider_root("src", &root).unwrap();
    let admitted = actor
        .admit_apply(
            &binding,
            None,
            true,
            ProviderDeadline::from_budget(Duration::from_secs(5)),
            &CancellationToken::new(),
        )
        .unwrap();
    let mut state = admitted.staged_state().unwrap();

    for target in [".BUILD/unica/forged.json", "Nested/.BUILD/forged.json"] {
        let error = state
            .create(target, b"forged".to_vec())
            .expect_err("platform-equivalent generated component reached Source role");
        assert_eq!(error.kind(), ApplyStagingErrorKind::ContainmentIdentity);
    }
    let _ = std::fs::remove_dir_all(root);
}

pub fn workspace_root_source_and_missing_cache_publish_through_disjoint_shared_anchor(
    policies: crate::policies::WorkspacePolicies,
) {
    let _policy_scope = policies_scope(policies);

    let root = temp_root("workspace-root-shared-cache-anchor");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: src\n    type: CONFIGURATION\n    path: .\n",
    )
    .unwrap();
    std::fs::write(
        root.join("Configuration.xml"),
        "<MetaDataObject><Configuration/></MetaDataObject>",
    )
    .unwrap();
    std::fs::write(root.join("Module.bsl"), b"before").unwrap();
    let actor_context = context(&root);
    let identity = WorkspaceIdentity::new(
        &actor_context,
        [source_input("src", &root)],
        "test-provider",
    )
    .unwrap();
    let actor = super::WorkspaceActor::<(), ()>::new(identity, actor_context)
        .map(bind_actor)
        .unwrap();
    let binding = actor.bind_provider_root("src", &root).unwrap();
    let admitted = actor
        .admit_apply(
            &binding,
            None,
            false,
            ProviderDeadline::from_budget(Duration::from_secs(5)),
            &CancellationToken::new(),
        )
        .unwrap();
    let mut state = admitted.staged_state().unwrap();
    state
        .replace("Module.bsl", b"before", b"after".to_vec())
        .unwrap();

    actor
        .publish_prepared_apply(
            admitted
                .prepare_with_cache_effects(
                    state,
                    &[unica_source_contract::events::DomainEvent::new(
                        unica_source_contract::events::DomainEventKind::MetadataChanged,
                        "Catalog.Products",
                    )],
                )
                .unwrap(),
        )
        .unwrap();

    assert_eq!(std::fs::read(root.join("Module.bsl")).unwrap(), b"after");
    assert!(root.join(".build/unica/state.json").is_file());
    let _ = std::fs::remove_dir_all(root);
}

pub fn prepared_apply_publishes_classified_binary_resource(
    policies: crate::policies::WorkspacePolicies,
) {
    let _policy_scope = policies_scope(policies);

    let fixture = actor_fixture("revision-package-projection", &["src"]);
    let package = fixture.roots[0].join("XDTOPackages/Sample/Ext/Package.bin");
    std::fs::create_dir_all(package.parent().unwrap()).unwrap();
    std::fs::write(&package, b"package-before").unwrap();
    let binding = fixture
        .actor
        .bind_provider_root("src", &fixture.roots[0])
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
    let mut state = admitted.staged_state().unwrap();
    state
        .replace(
            "XDTOPackages/Sample/Ext/Package.bin",
            b"package-before",
            b"package-after".to_vec(),
        )
        .unwrap();
    let result = fixture
        .actor
        .publish_prepared_apply(admitted.prepare(state).unwrap())
        .expect("classified Platform resource must publish through retained equality");

    assert_eq!(std::fs::read(package).unwrap(), b"package-after");
    assert_eq!(result.commit_count_for_test(), 1);
    fixture.cleanup();
}

pub fn replacement_commit_cleans_its_owned_recovery(policies: crate::policies::WorkspacePolicies) {
    let _policy_scope = policies_scope(policies);

    let fixture = actor_fixture("revision-replacement-entry-limit", &["src"]);
    let relative = "XDTOPackages/Sample/Ext/Package.bin";
    let package = fixture.roots[0].join(relative);
    std::fs::create_dir_all(package.parent().unwrap()).unwrap();
    std::fs::write(&package, b"before").unwrap();
    let binding = fixture
        .actor
        .bind_provider_root("src", &fixture.roots[0])
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
    let mut state = admitted.staged_state().unwrap();
    state
        .replace(relative, b"before", b"after".to_vec())
        .unwrap();
    let result = fixture
        .actor
        .publish_prepared_apply(admitted.prepare(state).unwrap())
        .expect("replacement publication must retain and clean its recovery");

    assert_eq!(std::fs::read(package).unwrap(), b"after");
    assert_eq!(result.commit_count_for_test(), 1);
    assert!(
        !snapshot_tree(&fixture.roots[0]).iter().any(|(path, _)| path
            .file_name()
            .is_some_and(|name| name.to_string_lossy().starts_with(".unica-apply-")))
    );
    fixture.cleanup();
}

pub fn replace_and_create_commit_cleans_the_owned_recovery(
    policies: crate::policies::WorkspacePolicies,
) {
    let _policy_scope = policies_scope(policies);

    let fixture = actor_fixture("revision-new-leaf-entry-limit", &["src"]);
    let first = "Catalogs/Items/Forms/Main/Ext/Form/Items/a.png";
    let second = "Catalogs/Items/Forms/Main/Ext/Form/Items/b.png";
    let first_path = fixture.roots[0].join(first);
    let second_path = fixture.roots[0].join(second);
    std::fs::create_dir_all(first_path.parent().unwrap()).unwrap();
    std::fs::write(&first_path, b"before").unwrap();
    let binding = fixture
        .actor
        .bind_provider_root("src", &fixture.roots[0])
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
    let mut state = admitted.staged_state().unwrap();
    state.replace(first, b"before", b"after".to_vec()).unwrap();
    state.create(second, b"created".to_vec()).unwrap();
    let result = fixture
        .actor
        .publish_prepared_apply(admitted.prepare(state).unwrap())
        .expect("replace/create batch must publish both postimages");

    assert_eq!(std::fs::read(first_path).unwrap(), b"after");
    assert_eq!(std::fs::read(second_path).unwrap(), b"created");
    assert_eq!(result.commit_count_for_test(), 1);
    fixture.cleanup();
}

pub fn multiple_recoveries_across_parents_preserve_each_postimage(
    policies: crate::policies::WorkspacePolicies,
) {
    let _policy_scope = policies_scope(policies);

    let fixture = actor_fixture("revision-multiple-recovery-entry-limit", &["src"]);
    let paths = [
        "Catalogs/Items/Forms/Main/Ext/Form/Items/a.png",
        "Catalogs/Items/Forms/Main/Ext/Form/Items/b.png",
        "XDTOPackages/Sample/Ext/Package.bin",
    ];
    for path in paths {
        let target = fixture.roots[0].join(path);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(target, b"before").unwrap();
    }
    let binding = fixture
        .actor
        .bind_provider_root("src", &fixture.roots[0])
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
    let mut state = admitted.staged_state().unwrap();
    for path in paths {
        state
            .replace(path, b"before", format!("after-{path}").into_bytes())
            .unwrap();
    }
    let result = fixture
        .actor
        .publish_prepared_apply(admitted.prepare(state).unwrap())
        .expect("multiple recoveries must preserve every published postimage");

    assert_eq!(result.commit_count_for_test(), 1);
    for path in paths {
        assert_eq!(
            std::fs::read(fixture.roots[0].join(path)).unwrap(),
            format!("after-{path}").into_bytes()
        );
    }
    assert!(
        !snapshot_tree(&fixture.roots[0]).iter().any(|(path, _)| path
            .file_name()
            .is_some_and(|name| name.to_string_lossy().starts_with(".unica-apply-")))
    );
    fixture.cleanup();
}

pub fn remove_create_batch_preserves_unmodified_sibling(
    policies: crate::policies::WorkspacePolicies,
) {
    let _policy_scope = policies_scope(policies);

    let fixture = actor_fixture("revision-remove-create-entry-limit", &["src"]);
    let first = "Catalogs/Items/Forms/Main/Ext/Form/Items/a.png";
    let second = "Catalogs/Items/Forms/Main/Ext/Form/Items/b.png";
    let created = "Catalogs/Items/Forms/Main/Ext/Form/Items/c.png";
    for path in [first, second] {
        let target = fixture.roots[0].join(path);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(target, b"before").unwrap();
    }
    let binding = fixture
        .actor
        .bind_provider_root("src", &fixture.roots[0])
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
    let mut state = admitted.staged_state().unwrap();
    state.remove(first, b"before").unwrap();
    state.create(created, b"created".to_vec()).unwrap();
    let result = fixture
        .actor
        .publish_prepared_apply(admitted.prepare(state).unwrap())
        .expect("remove/create batch must preserve the unmodified sibling");

    assert!(!fixture.roots[0].join(first).exists());
    assert_eq!(
        std::fs::read(fixture.roots[0].join(created)).unwrap(),
        b"created"
    );
    assert_eq!(result.commit_count_for_test(), 1);
    assert_eq!(
        std::fs::read(fixture.roots[0].join(second)).unwrap(),
        b"before"
    );
    fixture.cleanup();
}

pub fn actor_revision_recovery_identity_swap_is_rejected_before_revision_install(
    policies: crate::policies::WorkspacePolicies,
) {
    let _policy_scope = policies_scope(policies);

    let fixture = actor_fixture("revision-recovery-identity-swap", &["src"]);
    let relative = "XDTOPackages/Sample/Ext/Package.bin";
    let target = fixture.roots[0].join(relative);
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    std::fs::write(&target, b"before").unwrap();
    let binding = fixture
        .actor
        .bind_provider_root("src", &fixture.roots[0])
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
    let mut state = admitted.staged_state().unwrap();
    state
        .replace(relative, b"before", b"after".to_vec())
        .unwrap();
    let prepared = admitted.prepare(state).unwrap();
    let parent = target.parent().unwrap().to_path_buf();
    let moved = parent.join("recovery-moved-aside");
    let hook_parent = parent.clone();
    let hook_moved = moved.clone();
    crate::compile_transaction::set_retained_apply_before_revision_validation_hook(move || {
        let recovery = retained_apply_recovery_path(&hook_parent);
        std::fs::rename(&recovery, &hook_moved).unwrap();
        std::fs::write(&recovery, b"decoy").unwrap();
    });

    let error = fixture
        .actor
        .publish_prepared_apply(prepared)
        .expect_err("a same-name recovery decoy returned a receipt");

    assert_eq!(
        error.kind(),
        super::ApplyPublicationErrorKind::RollbackIncomplete
    );
    assert!(error.contains("recovery"), "{error}");
    assert_eq!(std::fs::read(&moved).unwrap(), b"before");
    fixture.cleanup();
}

pub fn actor_revision_recovery_hard_link_alias_is_never_discounted_or_restored(
    policies: crate::policies::WorkspacePolicies,
) {
    let _policy_scope = policies_scope(policies);

    let fixture = actor_fixture("revision-recovery-hard-link", &["src"]);
    let relative = "XDTOPackages/Sample/Ext/Package.bin";
    let target = fixture.roots[0].join(relative);
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    std::fs::write(&target, b"before").unwrap();
    let binding = fixture
        .actor
        .bind_provider_root("src", &fixture.roots[0])
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
    let mut state = admitted.staged_state().unwrap();
    state
        .replace(relative, b"before", b"after".to_vec())
        .unwrap();
    let prepared = admitted.prepare(state).unwrap();
    let parent = target.parent().unwrap().to_path_buf();
    let alias = fixture.root.join("recovery-alias.bin");
    let hook_parent = parent.clone();
    let hook_alias = alias.clone();
    crate::compile_transaction::set_retained_apply_before_revision_validation_hook(move || {
        let recovery = retained_apply_recovery_path(&hook_parent);
        std::fs::hard_link(recovery, &hook_alias).unwrap();
    });

    let error = fixture
        .actor
        .publish_prepared_apply(prepared)
        .expect_err("a hard-linked recovery returned a receipt");

    assert_eq!(
        error.kind(),
        super::ApplyPublicationErrorKind::RollbackIncomplete
    );
    assert!(error.contains("hard-link"), "{error}");
    assert_eq!(std::fs::read(alias).unwrap(), b"before");
    fixture.cleanup();
}

pub fn retained_recovery_late_failure_reaches_phase_and_rolls_back_without_receipt(
    policies: crate::policies::WorkspacePolicies,
) {
    let _policy_scope = policies_scope(policies);

    use crate::compile_transaction::RetainedApplyFailpoint;

    for (label, failpoint) in [
        ("state-marker", RetainedApplyFailpoint::StateMarker),
        (
            "after-postimages",
            RetainedApplyFailpoint::AfterAllPostimages,
        ),
    ] {
        let fixture = actor_fixture(&format!("revision-exact-limit-late-{label}"), &["src"]);
        let relative = "XDTOPackages/Sample/Ext/Package.bin";
        let target = fixture.roots[0].join(relative);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(&target, b"before").unwrap();
        let binding = fixture
            .actor
            .bind_provider_root("src", &fixture.roots[0])
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
        let mut state = admitted.staged_state().unwrap();
        state
            .replace(relative, b"before", b"after".to_vec())
            .unwrap();
        let prepared = admitted.prepare(state).unwrap();
        let source_before = snapshot_tree(&fixture.roots[0]);
        let cache_before = snapshot_tree(&fixture.root.join(".build/unica"));
        crate::compile_transaction::set_retained_apply_failpoint(failpoint);

        let error = fixture.actor.publish_prepared_apply(prepared).unwrap_err();

        assert!(
            error.contains("injected retained apply failure"),
            "{label} stopped before its selected late phase: {error}"
        );
        assert_eq!(snapshot_tree(&fixture.roots[0]), source_before, "{label}");
        assert_eq!(
            snapshot_tree(&fixture.root.join(".build/unica")),
            cache_before,
            "{label}"
        );
        fixture.cleanup();
    }
}

pub fn unrelated_recovery_lookalikes_do_not_gain_cleanup_authority(
    policies: crate::policies::WorkspacePolicies,
) {
    let _policy_scope = policies_scope(policies);

    for foreign_name in [".unica-apply-spoof", "ordinary-ignored.bin"] {
        let fixture = actor_fixture(
            &format!("revision-transient-spoof-{foreign_name}"),
            &["src"],
        );
        let relative = "XDTOPackages/Sample/Ext/Package.bin";
        let target = fixture.roots[0].join(relative);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(&target, b"before").unwrap();
        let binding = fixture
            .actor
            .bind_provider_root("src", &fixture.roots[0])
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
        let mut state = admitted.staged_state().unwrap();
        state
            .replace(relative, b"before", b"after".to_vec())
            .unwrap();
        let prepared = admitted.prepare(state).unwrap();
        let foreign = target.parent().unwrap().join(foreign_name);
        let hook_foreign = foreign.clone();
        crate::compile_transaction::set_retained_apply_before_revision_validation_hook(move || {
            std::fs::write(hook_foreign, b"foreign").unwrap()
        });

        let result = fixture.actor.publish_prepared_apply(prepared).unwrap();
        assert_eq!(result.commit_count_for_test(), 1);
        assert_eq!(std::fs::read(&target).unwrap(), b"after");
        assert_eq!(std::fs::read(&foreign).unwrap(), b"foreign");
        fixture.cleanup();
    }
}

pub fn create_only_plan_survives_actor_restart_and_commits_exactly_once(
    policies: crate::policies::WorkspacePolicies,
) {
    let _policy_scope = policies_scope(policies);

    let fixture = actor_fixture("revision-transient-create-only", &["src"]);
    let relative = "XDTOPackages/Sample/Ext/Package.bin";
    let binding = fixture
        .actor
        .bind_provider_root("src", &fixture.roots[0])
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
    let mut state = admitted.staged_state().unwrap();
    state.create(relative, b"created".to_vec()).unwrap();
    let result = fixture
        .actor
        .publish_prepared_apply(admitted.prepare(state).unwrap())
        .expect("create-only publication must issue no recovery allowance");

    let restart_context = context(&fixture.root);
    let restart_identity = WorkspaceIdentity::new(
        &restart_context,
        [source_input("src", &fixture.roots[0])],
        "test-provider",
    )
    .unwrap();
    let restarted = super::WorkspaceActor::<(), ()>::new(restart_identity, restart_context)
        .map(bind_actor)
        .unwrap();
    let restarted_binding = restarted
        .bind_provider_root("src", &fixture.roots[0])
        .unwrap();
    let reproduced = restarted
        .admit_apply(
            &restarted_binding,
            Some(result.rev()),
            false,
            ProviderDeadline::from_budget(Duration::from_secs(5)),
            &CancellationToken::new(),
        )
        .unwrap();

    let mut state = reproduced.staged_state().unwrap();
    state.create(relative, b"created".to_vec()).unwrap();
    let committed = restarted
        .publish_prepared_apply(reproduced.prepare(state).unwrap())
        .unwrap();
    assert_eq!(committed.rev(), result.rev());
    assert_eq!(committed.commit_count_for_test(), 1);
    assert_eq!(
        std::fs::read(fixture.roots[0].join(relative)).unwrap(),
        b"created"
    );
    fixture.cleanup();
}

pub fn retained_cleanup_failure_preserves_foreign_names_across_restart(
    policies: crate::policies::WorkspacePolicies,
) {
    let _policy_scope = policies_scope(policies);

    let fixture = actor_fixture("revision-transient-cleanup-lifetime", &["src"]);
    let relative = "XDTOPackages/Sample/Ext/Package.bin";
    let target = fixture.roots[0].join(relative);
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    std::fs::write(&target, b"before").unwrap();
    let binding = fixture
        .actor
        .bind_provider_root("src", &fixture.roots[0])
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
    let mut state = admitted.staged_state().unwrap();
    state
        .replace(relative, b"before", b"after".to_vec())
        .unwrap();
    let prepared = admitted.prepare(state).unwrap();
    let parent = target.parent().unwrap().to_path_buf();
    let moved = parent.join("recovery-left-after-cleanup");
    let hook_parent = parent.clone();
    let hook_moved = moved.clone();
    crate::compile_transaction::set_retained_apply_before_post_validation_hook(move || {
        let recovery = retained_apply_recovery_path(&hook_parent);
        std::fs::rename(&recovery, &hook_moved).unwrap();
        std::fs::write(recovery, b"decoy").unwrap();
    });

    let result = fixture
        .actor
        .publish_prepared_apply(prepared)
        .expect("cleanup failure occurs after successful publication");

    assert_eq!(result.cleanup_diagnostics().len(), 1);
    assert!(moved.exists());
    let retained_after_cleanup = snapshot_tree(&fixture.roots[0]);
    let restart_context = context(&fixture.root);
    let restart_identity = WorkspaceIdentity::new(
        &restart_context,
        [source_input("src", &fixture.roots[0])],
        "test-provider",
    )
    .unwrap();
    let restarted = super::WorkspaceActor::<(), ()>::new(restart_identity, restart_context)
        .map(bind_actor)
        .unwrap();
    let restarted_binding = restarted
        .bind_provider_root("src", &fixture.roots[0])
        .unwrap();
    let admission = restarted
        .admit_apply(
            &restarted_binding,
            None,
            true,
            ProviderDeadline::from_budget(Duration::from_secs(5)),
            &CancellationToken::new(),
        )
        .unwrap();
    let mut state = admission.staged_state().unwrap();
    state.replace(relative, b"after", b"next".to_vec()).unwrap();
    let preview = restarted
        .publish_prepared_apply(admission.prepare(state).unwrap())
        .unwrap();
    assert_eq!(preview.commit_count_for_test(), 0);
    assert_eq!(snapshot_tree(&fixture.roots[0]), retained_after_cleanup);
    assert_eq!(std::fs::read(&moved).unwrap(), b"before");
    fixture.cleanup();
}

pub fn retained_apply_stop_causes_preserve_rollback_at_every_late_gate(
    policies: crate::policies::WorkspacePolicies,
) {
    let _policy_scope = policies_scope(policies);

    for (phase, deadline_case) in [
        ("recovery", false),
        ("postimages", false),
        ("postimages", true),
    ] {
        let fixture = actor_fixture(&format!("retained-stop-{phase}-{deadline_case}"), &["src"]);
        let relative = "XDTOPackages/Sample/Ext/Package.bin";
        let target = fixture.roots[0].join(relative);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(&target, b"before").unwrap();
        let binding = fixture
            .actor
            .bind_provider_root("src", &fixture.roots[0])
            .unwrap();
        let cancellation = CancellationToken::new();
        let admitted = fixture
            .actor
            .admit_apply(
                &binding,
                None,
                false,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &cancellation,
            )
            .unwrap();
        let mut state = admitted.staged_state().unwrap();
        state
            .replace(relative, b"before", b"after".to_vec())
            .unwrap();
        let mut prepared = admitted.prepare(state).unwrap();
        let source_before = snapshot_tree(&fixture.roots[0]);
        let cache_before = snapshot_tree(&fixture.root.join(".build/unica"));
        if deadline_case {
            let now = Instant::now();
            RETAINED_APPLY_DEADLINE_TEST_NOW.set(now);
            prepared.deadline = ProviderDeadline::with_clock(
                now + Duration::from_secs(1),
                retained_apply_deadline_test_now,
            );
            crate::compile_transaction::set_retained_apply_before_post_validation_hook(move || {
                RETAINED_APPLY_DEADLINE_TEST_NOW.set(now + Duration::from_secs(2))
            });
        } else if phase == "recovery" {
            let cancel = cancellation.clone();
            crate::compile_transaction::set_retained_apply_before_revision_validation_hook(
                move || cancel.cancel(),
            );
        } else {
            let cancel = cancellation.clone();
            crate::compile_transaction::set_retained_apply_before_post_validation_hook(move || {
                cancel.cancel()
            });
        }
        let error = fixture.actor.publish_prepared_apply(prepared).unwrap_err();
        assert_eq!(
            error.kind(),
            if deadline_case {
                super::ApplyPublicationErrorKind::Deadline
            } else {
                super::ApplyPublicationErrorKind::Cancelled
            }
        );
        assert_eq!(snapshot_tree(&fixture.roots[0]), source_before, "{phase}");
        assert_eq!(
            snapshot_tree(&fixture.root.join(".build/unica")),
            cache_before,
            "{phase}"
        );
        fixture.cleanup();
    }
}

pub fn actor_revision_unknown_staged_artifact_is_rejected_before_publication(
    policies: crate::policies::WorkspacePolicies,
) {
    let _policy_scope = policies_scope(policies);

    let fixture = actor_fixture("revision-unknown-stage", &["src"]);
    let target = fixture.roots[0].join("Loose/random.bin");
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    std::fs::write(&target, b"before").unwrap();
    let binding = fixture
        .actor
        .bind_provider_root("src", &fixture.roots[0])
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
    let mut state = admitted.staged_state().unwrap();
    state
        .replace("Loose/random.bin", b"before", b"after".to_vec())
        .unwrap();
    let source_before = snapshot_tree(&fixture.roots[0]);
    let cache_before = snapshot_tree(&fixture.root.join(".build/unica"));

    let error = match admitted.prepare(state) {
        Err(error) => error,
        Ok(_) => panic!("unknown staged artifact crossed revision preparation"),
    };

    assert_eq!(error.kind(), ApplyStagingErrorKind::Invariant);
    assert!(
        error.to_string().contains("supported source profile"),
        "{error}"
    );
    assert_eq!(snapshot_tree(&fixture.roots[0]), source_before);
    assert_eq!(
        snapshot_tree(&fixture.root.join(".build/unica")),
        cache_before
    );
    fixture.cleanup();
}

pub fn actor_revision_lookalike_resource_is_rejected_before_publication(
    policies: crate::policies::WorkspacePolicies,
) {
    let _policy_scope = policies_scope(policies);

    let fixture = actor_fixture("revision-lookalike-stage", &["src"]);
    let relative = "Loose/Templates/Junk/Ext/Template.bin";
    let target = fixture.roots[0].join(relative);
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    std::fs::write(&target, b"before").unwrap();
    let binding = fixture
        .actor
        .bind_provider_root("src", &fixture.roots[0])
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
    let mut state = admitted.staged_state().unwrap();
    state
        .replace(relative, b"before", b"after".to_vec())
        .unwrap();
    let source_before = snapshot_tree(&fixture.roots[0]);
    let cache_before = snapshot_tree(&fixture.root.join(".build/unica"));

    let error = match admitted.prepare(state) {
        Err(error) => error,
        Ok(_) => panic!("lookalike resource crossed revision preparation"),
    };

    assert_eq!(error.kind(), ApplyStagingErrorKind::Invariant);
    assert!(
        error.to_string().contains("supported source profile"),
        "{error}"
    );
    assert_eq!(snapshot_tree(&fixture.roots[0]), source_before);
    assert_eq!(
        snapshot_tree(&fixture.root.join(".build/unica")),
        cache_before
    );
    fixture.cleanup();
}

pub fn actor_revision_late_failure_rolls_back_targeted_resource_without_receipt(
    policies: crate::policies::WorkspacePolicies,
) {
    let _policy_scope = policies_scope(policies);

    use crate::compile_transaction::RetainedApplyFailpoint;

    let mut failures = Vec::new();
    for (label, failpoint) in [
        ("state-marker", RetainedApplyFailpoint::StateMarker),
        (
            "after-postimages",
            RetainedApplyFailpoint::AfterAllPostimages,
        ),
    ] {
        let fixture = actor_fixture(&format!("revision-targeted-rollback-{label}"), &["src"]);
        let package = fixture.roots[0].join("XDTOPackages/Sample/Ext/Package.bin");
        std::fs::create_dir_all(package.parent().unwrap()).unwrap();
        std::fs::write(&package, b"before").unwrap();
        let binding = fixture
            .actor
            .bind_provider_root("src", &fixture.roots[0])
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
        let mut state = admitted.staged_state().unwrap();
        state
            .replace(
                "XDTOPackages/Sample/Ext/Package.bin",
                b"before",
                b"after".to_vec(),
            )
            .unwrap();
        let prepared = admitted.prepare(state).unwrap();
        let source_before = snapshot_tree(&fixture.roots[0]);
        let cache_before = snapshot_tree(&fixture.root.join(".build/unica"));
        crate::compile_transaction::set_retained_apply_failpoint(failpoint);

        let error = match fixture.actor.publish_prepared_apply(prepared) {
            Err(error) => error,
            Ok(_) => panic!("{label} late failure returned a receipt"),
        };
        if !error.contains("injected retained apply failure") {
            failures.push(format!(
                "{label} stopped before selected late phase: {error}"
            ));
        }
        assert_eq!(snapshot_tree(&fixture.roots[0]), source_before, "{label}");
        assert_eq!(
            snapshot_tree(&fixture.root.join(".build/unica")),
            cache_before,
            "{label}"
        );
        fixture.cleanup();
    }

    assert!(
        failures.is_empty(),
        "targeted rollback phase failures: {failures:?}"
    );
}

pub fn actor_platform_commit_accepts_a_classified_binary_resource(
    policies: crate::policies::WorkspacePolicies,
) {
    let _policy_scope = policies_scope(policies);

    let fixture = actor_fixture("revision-platform-compatibility", &["src"]);
    let package = fixture.roots[0].join("XDTOPackages/Sample/Ext/Package.bin");
    std::fs::create_dir_all(package.parent().unwrap()).unwrap();
    std::fs::write(&package, b"before").unwrap();
    let binding = fixture
        .actor
        .bind_provider_root("src", &fixture.roots[0])
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
    let mut state = admitted.staged_state().unwrap();
    state
        .replace(
            "XDTOPackages/Sample/Ext/Package.bin",
            b"before",
            b"after".to_vec(),
        )
        .unwrap();
    let platform_commit = fixture
        .actor
        .publish_prepared_apply(admitted.prepare(state).unwrap());

    let result = platform_commit
        .expect("classified Platform resource commit must precede compatibility guards");
    assert_eq!(result.commit_count_for_test(), 1);
    fixture.cleanup();
}

pub fn prepared_apply_observer_sees_source_eager_cache_and_state_marker_order(
    policies: crate::policies::WorkspacePolicies,
) {
    let _policy_scope = policies_scope(policies);

    let fixture = actor_fixture("prepared-apply-publication-order", &["src"]);
    std::fs::write(fixture.roots[0].join("Module.bsl"), b"original").unwrap();
    let binding = fixture
        .actor
        .bind_provider_root("src", &fixture.roots[0])
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
    let mut state = admitted.staged_state().unwrap();
    state
        .replace("Module.bsl", b"original", b"published".to_vec())
        .unwrap();
    let prepared = admitted
        .prepare_with_cache_effects(
            state,
            &[unica_source_contract::events::DomainEvent::new(
                unica_source_contract::events::DomainEventKind::MetadataChanged,
                "Catalog.Products",
            )],
        )
        .unwrap();
    let _ = crate::compile_transaction::take_retained_apply_observed_events();

    fixture.actor.publish_prepared_apply(prepared).unwrap();

    use crate::compile_transaction::RetainedApplyObservedEvent;
    let observed = crate::compile_transaction::take_retained_apply_observed_events();
    assert!(
        matches!(
            observed.as_slice(),
            [
                RetainedApplyObservedEvent::Source(_),
                RetainedApplyObservedEvent::EagerMetadata(_),
                RetainedApplyObservedEvent::EagerMetadata(_),
                RetainedApplyObservedEvent::StateMarker(_),
            ]
        ),
        "unexpected retained publication order: {observed:?}"
    );
    fixture.cleanup();
}

pub fn prepared_apply_success_publishes_source_cache_and_state_without_revision_record(
    policies: crate::policies::WorkspacePolicies,
) {
    let _policy_scope = policies_scope(policies);

    let fixture = actor_fixture("prepared-apply-cache-success", &["src"]);
    let target = fixture.roots[0].join("Module.bsl");
    std::fs::write(&target, b"original").unwrap();
    let binding = fixture
        .actor
        .bind_provider_root("src", &fixture.roots[0])
        .unwrap();
    let cancellation = CancellationToken::new();
    let deadline = ProviderDeadline::from_budget(Duration::from_secs(5));

    let admitted = fixture
        .actor
        .admit_apply(&binding, None, false, deadline, &cancellation)
        .unwrap();
    let mut state = admitted.staged_state().unwrap();
    state
        .replace("Module.bsl", b"original", b"published".to_vec())
        .unwrap();
    let prepared = admitted
        .prepare_with_cache_effects(
            state,
            &[unica_source_contract::events::DomainEvent::new(
                unica_source_contract::events::DomainEventKind::MetadataChanged,
                "Catalog.Products",
            )],
        )
        .unwrap();

    let plan = prepared.plan_identity.clone();
    let result = fixture.actor.publish_prepared_apply(prepared).unwrap();
    let cache_root = fixture.root.join(".build/unica");
    assert_eq!(std::fs::read(&target).unwrap(), b"published");
    assert!(cache_root.join("caches/workspace_graph.json").is_file());
    assert!(cache_root.join("caches/metadata_graph.json").is_file());
    assert!(cache_root.join("state.json").is_file());
    assert!(!cache_root.join("source-revisions").exists());
    assert_eq!(result.rev(), plan);

    assert_eq!(result.commit_count_for_test(), 1);
    fixture.cleanup();
}

pub fn retained_apply_failures_restore_source_cache_and_revision_machine_exactly(
    policies: crate::policies::WorkspacePolicies,
) {
    let _policy_scope = policies_scope(policies);

    use crate::compile_transaction::RetainedApplyFailpoint;

    for (name, failpoint) in [
        ("second-source", RetainedApplyFailpoint::Source(2)),
        ("eager-cache", RetainedApplyFailpoint::EagerMetadata(1)),
        ("state-marker", RetainedApplyFailpoint::StateMarker),
        ("postimages", RetainedApplyFailpoint::AfterAllPostimages),
    ] {
        let fixture = actor_fixture(&format!("retained-failure-{name}"), &["src"]);
        std::fs::write(fixture.roots[0].join("A.bsl"), b"a-before").unwrap();
        std::fs::write(fixture.roots[0].join("B.bsl"), b"b-before").unwrap();
        let binding = fixture
            .actor
            .bind_provider_root("src", &fixture.roots[0])
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
        let mut state = admitted.staged_state().unwrap();
        state
            .replace("A.bsl", b"a-before", b"a-after".to_vec())
            .unwrap();
        state
            .replace("B.bsl", b"b-before", b"b-after".to_vec())
            .unwrap();
        let prepared = admitted
            .prepare_with_cache_effects(
                state,
                &[unica_source_contract::events::DomainEvent::new(
                    unica_source_contract::events::DomainEventKind::MetadataChanged,
                    "Catalog.Products",
                )],
            )
            .unwrap();
        let source_before = snapshot_tree(&fixture.roots[0]);
        let cache_before = snapshot_tree(&fixture.root.join(".build/unica"));
        crate::compile_transaction::set_retained_apply_failpoint(failpoint);

        let error = fixture
            .actor
            .publish_prepared_apply(prepared)
            .expect_err(&format!("{name} failpoint did not stop publication"));

        assert!(
            error.contains("injected retained apply failure"),
            "{name}: {error}"
        );
        assert_eq!(
            error.kind(),
            super::ApplyPublicationErrorKind::ProviderPostvalidation,
            "{name} failure lost its typed publication phase"
        );
        assert_eq!(snapshot_tree(&fixture.roots[0]), source_before, "{name}");
        assert_eq!(
            snapshot_tree(&fixture.root.join(".build/unica")),
            cache_before,
            "{name}"
        );
        fixture.cleanup();
    }
}

pub fn unchanged_selection_input_replaced_after_writes_refuses_and_rolls_back(
    policies: crate::policies::WorkspacePolicies,
) {
    let _policy_scope = policies_scope(policies);

    use crate::compile_transaction::RetainedApplyObservedEvent;
    use crate::platform::filesystem::file_identity;

    let fixture = actor_fixture("unchanged-selection-late-identity", &["src"]);
    let source = &fixture.roots[0];
    write_actor_event_fixture(source);
    let configuration = source.join("Configuration.xml");
    let original = std::fs::read(&configuration).unwrap();
    let original_identity = file_identity(&std::fs::File::open(&configuration).unwrap()).unwrap();
    let target = source.join("Module.bsl");
    std::fs::write(&target, b"before").unwrap();
    let binding = fixture.actor.bind_provider_root("src", source).unwrap();
    let admission = fixture
        .actor
        .admit_apply(
            &binding,
            None,
            false,
            ProviderDeadline::from_budget(Duration::from_secs(10)),
            &CancellationToken::new(),
        )
        .unwrap();
    let mut state = admission.staged_state().unwrap();
    assert_eq!(
        state.read(Path::new("Configuration.xml")).unwrap(),
        Some(original.clone())
    );
    state
        .replace("Module.bsl", b"before", b"after".to_vec())
        .unwrap();
    let prepared = admission
        .prepare_with_cache_effects(
            state,
            &[unica_source_contract::events::DomainEvent::new(
                DomainEventKind::MetadataChanged,
                "Catalog.Products",
            )],
        )
        .unwrap();
    let source_before = snapshot_tree(source);
    let cache = fixture.root.join(".build/unica");
    let cache_before = snapshot_tree(&cache);
    let replacement_identity = Arc::new(std::sync::Mutex::new(None));
    let hook_identity = Arc::clone(&replacement_identity);
    let hook_configuration = configuration.clone();
    let hook_target = target.clone();
    let hook_cache_before = cache_before.clone();
    let _ = crate::compile_transaction::take_retained_apply_observed_events();
    crate::compile_transaction::set_retained_apply_before_post_validation_hook(move || {
        assert_eq!(std::fs::read(&hook_target).unwrap(), b"after");
        assert_ne!(
            snapshot_tree(&cache),
            hook_cache_before,
            "cache publication must precede the race"
        );
        let replacement = hook_configuration.with_extension("replacement");
        std::fs::write(&replacement, &original).unwrap();
        let identity = file_identity(&std::fs::File::open(&replacement).unwrap()).unwrap();
        assert_ne!(identity, original_identity);
        std::fs::rename(&replacement, &hook_configuration).unwrap();
        *hook_identity.lock().unwrap() = Some(identity);
    });

    let error = fixture.actor.publish_prepared_apply(prepared).expect_err(
        "an unchanged input is not an owned replacement and must retain its original identity",
    );
    assert_eq!(
        error.kind(),
        super::ApplyPublicationErrorKind::SourceSelectionChanged
    );
    assert_eq!(snapshot_tree(source), source_before);
    assert_eq!(
        snapshot_tree(&fixture.root.join(".build/unica")),
        cache_before
    );
    let external_identity = replacement_identity.lock().unwrap().expect("late hook ran");
    assert_eq!(
        file_identity(&std::fs::File::open(&configuration).unwrap()).unwrap(),
        external_identity
    );
    let observed = crate::compile_transaction::take_retained_apply_observed_events();
    assert!(observed
        .iter()
        .any(|event| matches!(event, RetainedApplyObservedEvent::StateMarker(_))));
    assert!(observed
        .iter()
        .any(|event| matches!(event, RetainedApplyObservedEvent::Rollback(_))));
    fixture.cleanup();
}

pub fn retained_apply_final_cancellation_gate_rolls_back_all_participants(
    policies: crate::policies::WorkspacePolicies,
) {
    let _policy_scope = policies_scope(policies);

    let fixture = actor_fixture("retained-final-cancellation", &["src"]);
    std::fs::write(fixture.roots[0].join("Module.bsl"), b"before").unwrap();
    let binding = fixture
        .actor
        .bind_provider_root("src", &fixture.roots[0])
        .unwrap();
    let cancellation = CancellationToken::new();
    let admitted = fixture
        .actor
        .admit_apply(
            &binding,
            None,
            false,
            ProviderDeadline::from_budget(Duration::from_secs(5)),
            &cancellation,
        )
        .unwrap();
    let mut state = admitted.staged_state().unwrap();
    state
        .replace("Module.bsl", b"before", b"after".to_vec())
        .unwrap();
    let prepared = admitted
        .prepare_with_cache_effects(
            state,
            &[unica_source_contract::events::DomainEvent::new(
                unica_source_contract::events::DomainEventKind::MetadataChanged,
                "Catalog.Products",
            )],
        )
        .unwrap();
    let source_before = snapshot_tree(&fixture.roots[0]);
    let cache_before = snapshot_tree(&fixture.root.join(".build/unica"));
    let cancel_at_gate = cancellation.clone();
    crate::compile_transaction::set_retained_apply_before_post_validation_hook(move || {
        cancel_at_gate.cancel()
    });

    let error = fixture.actor.publish_prepared_apply(prepared).unwrap_err();

    assert!(error.contains("cancel"), "{error}");
    assert_eq!(error.kind(), super::ApplyPublicationErrorKind::Cancelled);
    assert_eq!(snapshot_tree(&fixture.roots[0]), source_before);
    assert_eq!(
        snapshot_tree(&fixture.root.join(".build/unica")),
        cache_before
    );
    fixture.cleanup();
}

pub fn retained_apply_late_deadline_after_all_writes_rolls_back_all_participants(
    policies: crate::policies::WorkspacePolicies,
) {
    let _policy_scope = policies_scope(policies);

    use crate::compile_transaction::RetainedApplyObservedEvent;

    let fixture = actor_fixture("retained-final-deadline", &["src"]);
    std::fs::write(fixture.roots[0].join("Module.bsl"), b"before").unwrap();
    let binding = fixture
        .actor
        .bind_provider_root("src", &fixture.roots[0])
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
    let mut state = admitted.staged_state().unwrap();
    state
        .replace("Module.bsl", b"before", b"after".to_vec())
        .unwrap();
    let mut prepared = admitted
        .prepare_with_cache_effects(
            state,
            &[unica_source_contract::events::DomainEvent::new(
                unica_source_contract::events::DomainEventKind::MetadataChanged,
                "Catalog.Products",
            )],
        )
        .unwrap();
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
    assert!(
        observed
            .iter()
            .any(|event| matches!(event, RetainedApplyObservedEvent::StateMarker(_))),
        "deadline fired before all participant writes: {observed:?}"
    );
    assert!(
        observed
            .iter()
            .any(|event| matches!(event, RetainedApplyObservedEvent::Rollback(_))),
        "late deadline did not enter rollback: {observed:?}"
    );
    assert_eq!(snapshot_tree(&fixture.roots[0]), source_before);
    assert_eq!(
        snapshot_tree(&fixture.root.join(".build/unica")),
        cache_before
    );
    fixture.cleanup();
}

pub fn retained_apply_observer_sees_exact_reverse_rollback_after_state_marker(
    policies: crate::policies::WorkspacePolicies,
) {
    let _policy_scope = policies_scope(policies);

    use crate::compile_transaction::{RetainedApplyFailpoint, RetainedApplyObservedEvent};

    let fixture = actor_fixture("retained-reverse-rollback-order", &["src"]);
    std::fs::write(fixture.roots[0].join("A.bsl"), b"a-before").unwrap();
    std::fs::write(fixture.roots[0].join("B.bsl"), b"b-before").unwrap();
    let binding = fixture
        .actor
        .bind_provider_root("src", &fixture.roots[0])
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
    let mut state = admitted.staged_state().unwrap();
    state
        .replace("A.bsl", b"a-before", b"a-after".to_vec())
        .unwrap();
    state
        .replace("B.bsl", b"b-before", b"b-after".to_vec())
        .unwrap();
    let prepared = admitted
        .prepare_with_cache_effects(
            state,
            &[unica_source_contract::events::DomainEvent::new(
                unica_source_contract::events::DomainEventKind::MetadataChanged,
                "Catalog.Products",
            )],
        )
        .unwrap();
    let _ = crate::compile_transaction::take_retained_apply_observed_events();
    crate::compile_transaction::set_retained_apply_failpoint(RetainedApplyFailpoint::StateMarker);

    fixture.actor.publish_prepared_apply(prepared).unwrap_err();

    let observed = crate::compile_transaction::take_retained_apply_observed_events();
    let forward = observed
        .iter()
        .take_while(|event| !matches!(event, RetainedApplyObservedEvent::Rollback(_)))
        .map(|event| match event {
            RetainedApplyObservedEvent::Source(path)
            | RetainedApplyObservedEvent::EagerMetadata(path)
            | RetainedApplyObservedEvent::StateMarker(path) => path.clone(),
            RetainedApplyObservedEvent::Rollback(_) => unreachable!(),
        })
        .collect::<Vec<_>>();
    let rollback = observed
        .iter()
        .filter_map(|event| match event {
            RetainedApplyObservedEvent::Rollback(path) => Some(path.clone()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(rollback, forward.into_iter().rev().collect::<Vec<_>>());
    fixture.cleanup();
}

pub fn retained_apply_publication_preserves_exact_typed_causes_end_to_end(
    policies: crate::policies::WorkspacePolicies,
) {
    let _policy_scope = policies_scope(policies);

    let fixture = actor_fixture("typed-cause-cancelled-scan", &["src"]);
    std::fs::write(fixture.roots[0].join("Module.bsl"), b"before").unwrap();
    let binding = fixture
        .actor
        .bind_provider_root("src", &fixture.roots[0])
        .unwrap();
    let cancellation = CancellationToken::new();
    let admitted = fixture
        .actor
        .admit_apply(
            &binding,
            None,
            false,
            ProviderDeadline::from_budget(Duration::from_secs(5)),
            &cancellation,
        )
        .unwrap();
    let mut state = admitted.staged_state().unwrap();
    state
        .replace("Module.bsl", b"before", b"after".to_vec())
        .unwrap();
    let prepared = admitted.prepare(state).unwrap();
    let cancel_during_confirmation = cancellation.clone();
    set_apply_before_input_confirmation_hook(move || cancel_during_confirmation.cancel());
    let error = fixture.actor.publish_prepared_apply(prepared).unwrap_err();
    assert_eq!(error.kind(), super::ApplyPublicationErrorKind::Cancelled);
    fixture.cleanup();

    let fixture = actor_fixture("typed-cause-deadline-scan", &["src"]);
    std::fs::write(fixture.roots[0].join("Module.bsl"), b"before").unwrap();
    let binding = fixture
        .actor
        .bind_provider_root("src", &fixture.roots[0])
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
    let mut state = admitted.staged_state().unwrap();
    state
        .replace("Module.bsl", b"before", b"after".to_vec())
        .unwrap();
    let mut prepared = admitted.prepare(state).unwrap();
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
    fixture.cleanup();

    let fixture = actor_fixture("typed-cause-concurrent-scan", &["src"]);
    std::fs::write(fixture.roots[0].join("Module.bsl"), b"before").unwrap();
    let binding = fixture
        .actor
        .bind_provider_root("src", &fixture.roots[0])
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
    let mut state = admitted.staged_state().unwrap();
    state
        .replace("Module.bsl", b"before", b"after".to_vec())
        .unwrap();
    let prepared = admitted.prepare(state).unwrap();
    let concurrent = fixture.roots[0].join("Module.bsl");
    set_apply_before_input_confirmation_hook(move || {
        std::fs::write(&concurrent, b"foreign").unwrap()
    });
    let error = fixture.actor.publish_prepared_apply(prepared).unwrap_err();
    assert_eq!(
        error.kind(),
        super::ApplyPublicationErrorKind::ConcurrentRevision
    );
    fixture.cleanup();

    let fixture = actor_fixture("typed-cause-containment", &["src"]);
    std::fs::write(fixture.roots[0].join("Module.bsl"), b"before").unwrap();
    let binding = fixture
        .actor
        .bind_provider_root("src", &fixture.roots[0])
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
    let mut state = admitted.staged_state().unwrap();
    state
        .replace("Module.bsl", b"before", b"after".to_vec())
        .unwrap();
    let prepared = admitted.prepare(state).unwrap();
    let displaced = fixture.root.join("source-displaced");
    let retained_identity = path_identity_for_test(&fixture.roots[0])
        .unwrap()
        .expect("source root identity must be available on supported CI platforms");
    match attempt_retained_directory_replacement_for_test(&fixture.roots[0], &displaced).unwrap() {
        RetainedDirectoryReplacementOutcome::Replaced => {
            std::fs::create_dir_all(&fixture.roots[0]).unwrap();
            std::fs::write(fixture.roots[0].join("Module.bsl"), b"foreign").unwrap();
            let error = fixture.actor.publish_prepared_apply(prepared).unwrap_err();
            assert_eq!(
                error.kind(),
                super::ApplyPublicationErrorKind::ContainmentIdentity
            );
            assert_eq!(
                std::fs::read(displaced.join("Module.bsl")).unwrap(),
                b"before"
            );
            assert_eq!(
                std::fs::read(fixture.roots[0].join("Module.bsl")).unwrap(),
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
            fixture.actor.publish_prepared_apply(prepared).unwrap();
            assert_eq!(
                std::fs::read(fixture.roots[0].join("Module.bsl")).unwrap(),
                b"after"
            );
        }
    }
    fixture.cleanup();

    let fixture = actor_fixture("typed-cause-publish-provider", &["src"]);
    std::fs::write(fixture.roots[0].join("Module.bsl"), b"before").unwrap();
    let binding = fixture
        .actor
        .bind_provider_root("src", &fixture.roots[0])
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
    let mut state = admitted.staged_state().unwrap();
    state
        .replace("Module.bsl", b"before", b"after".to_vec())
        .unwrap();
    let prepared = admitted.prepare(state).unwrap();
    let source_before = snapshot_tree(&fixture.roots[0]);
    let cache_root = fixture.root.join(".build/unica");
    let cache_before = snapshot_tree(&cache_root);
    let source_root = fixture.roots[0].clone();
    let module = source_root.join("Module.bsl");
    let held_provider_io_blocker = std::rc::Rc::new(std::cell::RefCell::new(None));
    let hook_blocker = std::rc::Rc::clone(&held_provider_io_blocker);
    crate::compile_transaction::set_retained_apply_before_provider_io_hook(move || {
        let unix_mode_set =
            crate::platform::testing::set_unix_mode_for_test(&source_root, 0o500).unwrap();
        if !unix_mode_set {
            *hook_blocker.borrow_mut() = Some(
                crate::platform::testing::hold_regular_file_without_delete_sharing_for_test(
                    &module,
                )
                .unwrap()
                .expect("Windows must support a retained fixture without delete sharing"),
            );
        }
    });
    let error = fixture.actor.publish_prepared_apply(prepared).unwrap_err();
    held_provider_io_blocker.borrow_mut().take();
    crate::platform::testing::set_unix_mode_for_test(&fixture.roots[0], 0o700).unwrap();
    assert_eq!(
        error.kind(),
        super::ApplyPublicationErrorKind::ProviderPostvalidation
    );
    assert_eq!(
        std::fs::read(fixture.roots[0].join("Module.bsl")).unwrap(),
        b"before"
    );
    assert_eq!(snapshot_tree(&fixture.roots[0]), source_before);
    assert_eq!(snapshot_tree(&cache_root), cache_before);
    fixture.cleanup();
}

pub fn prepared_apply_cache_authority_diagnostic_never_exposes_absolute_root(
    policies: crate::policies::WorkspacePolicies,
) {
    let _policy_scope = policies_scope(policies);

    let (fixture, binding) = publish_cache_fixture("cache-authority-redaction");
    let cache_root = fixture.root.join(".build/unica");
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
    let state = admitted.staged_state().unwrap();
    let prepared = admitted.prepare(state).unwrap();
    let displaced = fixture.root.join("cache-displaced");
    let retained_identity = path_identity_for_test(&cache_root)
        .unwrap()
        .expect("cache root identity must be available on supported CI platforms");
    match attempt_retained_directory_replacement_for_test(&cache_root, &displaced).unwrap() {
        RetainedDirectoryReplacementOutcome::Replaced => {
            std::fs::create_dir_all(&cache_root).unwrap();
            let error = fixture.actor.publish_prepared_apply(prepared).unwrap_err();

            assert_eq!(
                error.kind(),
                super::ApplyPublicationErrorKind::ContainmentIdentity
            );
            assert!(
                !error
                    .to_string()
                    .contains(&cache_root.display().to_string()),
                "cache authority diagnostic exposed its absolute root: {error}"
            );
        }
        RetainedDirectoryReplacementOutcome::PreventedByRetainedHandle => {
            eprintln!(
                "[SKIPPED FIXTURE] cache-root replacement is prevented by its retained handle"
            );
            assert_eq!(
                path_identity_for_test(&cache_root).unwrap().as_deref(),
                Some(retained_identity.as_str())
            );
            assert!(!displaced.exists());
            fixture.actor.publish_prepared_apply(prepared).unwrap();
        }
    }
    fixture.cleanup();
}

pub fn cache_preimage_races_fail_closed_and_preserve_foreign_names(
    policies: crate::policies::WorkspacePolicies,
) {
    let _policy_scope = policies_scope(policies);

    for case in ["cache-replaced", "state-disappeared", "cache-hard-link"] {
        let (fixture, binding) = publish_cache_fixture(&format!("preimage-{case}"));
        let prepared = prepare_cache_fixture_apply(&fixture, &binding);
        let cache_root = fixture.root.join(".build/unica");
        let metadata = cache_root.join("caches/metadata_graph.json");
        let state_marker = cache_root.join("state.json");
        match case {
            "cache-replaced" => std::fs::write(&metadata, b"foreign-cache").unwrap(),
            "state-disappeared" => std::fs::remove_file(&state_marker).unwrap(),
            "cache-hard-link" => {
                let foreign = fixture.root.join("foreign-cache.json");
                std::fs::write(&foreign, b"foreign-hard-link").unwrap();
                std::fs::remove_file(&metadata).unwrap();
                std::fs::hard_link(&foreign, &metadata).unwrap();
            }
            _ => unreachable!(),
        }

        let error = fixture
            .actor
            .publish_prepared_apply(prepared)
            .expect_err(&format!("{case} race was accepted"));

        assert!(
            error.contains("preimage") || error.contains("identity") || error.contains("hard-link"),
            "{case}: {error}"
        );
        assert_eq!(
            std::fs::read(fixture.roots[0].join("Module.bsl")).unwrap(),
            b"baseline"
        );
        match case {
            "cache-replaced" => assert_eq!(std::fs::read(metadata).unwrap(), b"foreign-cache"),
            "state-disappeared" => assert!(!state_marker.exists()),
            "cache-hard-link" => {
                assert_eq!(std::fs::read(metadata).unwrap(), b"foreign-hard-link")
            }
            _ => unreachable!(),
        }
        fixture.cleanup();
    }
}

pub fn cache_preimage_link_race_and_absent_chain_occupation_preserve_foreign_namespace(
    policies: crate::policies::WorkspacePolicies,
) {
    let _policy_scope = policies_scope(policies);

    let (fixture, binding) = publish_cache_fixture("preimage-cache-link");
    let prepared = prepare_cache_fixture_apply(&fixture, &binding);
    let state_path = fixture.root.join(".build/unica/state.json");
    let foreign = fixture.root.join("foreign-state.json");
    std::fs::write(&foreign, b"foreign-state").unwrap();
    std::fs::remove_file(&state_path).unwrap();
    match create_file_link_fixture_for_test(&foreign, &state_path).unwrap() {
        FileLinkFixtureOutcome::Created => {}
        FileLinkFixtureOutcome::Unsupported
        | FileLinkFixtureOutcome::WindowsPrivilegeUnavailable => {
            fixture.cleanup();
            return;
        }
    }

    fixture.actor.publish_prepared_apply(prepared).unwrap_err();

    assert!(std::fs::symlink_metadata(&state_path)
        .unwrap()
        .file_type()
        .is_symlink());
    assert_eq!(std::fs::read(&foreign).unwrap(), b"foreign-state");
    assert_eq!(
        std::fs::read(fixture.roots[0].join("Module.bsl")).unwrap(),
        b"baseline"
    );
    fixture.cleanup();

    let fixture = actor_fixture("preimage-cache-chain", &["src"]);
    std::fs::write(fixture.roots[0].join("Module.bsl"), b"baseline").unwrap();
    let binding = fixture
        .actor
        .bind_provider_root("src", &fixture.roots[0])
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
    let mut state = admitted.staged_state().unwrap();
    state
        .replace("Module.bsl", b"baseline", b"must-rollback".to_vec())
        .unwrap();
    let prepared = admitted
        .prepare_with_cache_effects(
            state,
            &[unica_source_contract::events::DomainEvent::new(
                unica_source_contract::events::DomainEventKind::MetadataChanged,
                "Catalog.Products",
            )],
        )
        .unwrap();
    std::fs::write(fixture.root.join(".build"), b"foreign-occupant").unwrap();

    fixture.actor.publish_prepared_apply(prepared).unwrap_err();

    assert_eq!(
        std::fs::read(fixture.root.join(".build")).unwrap(),
        b"foreign-occupant"
    );
    assert_eq!(
        std::fs::read(fixture.roots[0].join("Module.bsl")).unwrap(),
        b"baseline"
    );
    fixture.cleanup();
}

pub fn malformed_state_fallback_restores_exact_bytes_when_later_gate_fails(
    policies: crate::policies::WorkspacePolicies,
) {
    let _policy_scope = policies_scope(policies);

    let fixture = actor_fixture("malformed-state-rollback", &["src"]);
    std::fs::write(fixture.roots[0].join("Module.bsl"), b"before").unwrap();
    std::fs::create_dir_all(fixture.root.join(".build/unica")).unwrap();
    let malformed = b"{malformed-state\0bytes".to_vec();
    std::fs::write(fixture.root.join(".build/unica/state.json"), &malformed).unwrap();
    let binding = fixture
        .actor
        .bind_provider_root("src", &fixture.roots[0])
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
    let mut state = admitted.staged_state().unwrap();
    state
        .replace("Module.bsl", b"before", b"after".to_vec())
        .unwrap();
    let prepared = admitted
        .prepare_with_cache_effects(
            state,
            &[unica_source_contract::events::DomainEvent::new(
                unica_source_contract::events::DomainEventKind::MetadataChanged,
                "Catalog.Products",
            )],
        )
        .unwrap();
    crate::compile_transaction::set_retained_apply_failpoint(
        crate::compile_transaction::RetainedApplyFailpoint::AfterAllPostimages,
    );

    fixture.actor.publish_prepared_apply(prepared).unwrap_err();

    assert_eq!(
        std::fs::read(fixture.root.join(".build/unica/state.json")).unwrap(),
        malformed
    );
    assert_eq!(
        std::fs::read(fixture.roots[0].join("Module.bsl")).unwrap(),
        b"before"
    );
    fixture.cleanup();
}

pub fn rollback_incomplete_failure_has_a_typed_non_success_category(
    policies: crate::policies::WorkspacePolicies,
) {
    let _policy_scope = policies_scope(policies);

    let fixture = actor_fixture("typed-rollback-incomplete", &["src"]);
    let target = fixture.roots[0].join("Module.bsl");
    let moved = fixture.roots[0].join("published-moved.bsl");
    std::fs::write(&target, b"before").unwrap();
    let binding = fixture
        .actor
        .bind_provider_root("src", &fixture.roots[0])
        .unwrap();
    let cancellation = CancellationToken::new();
    let admitted = fixture
        .actor
        .admit_apply(
            &binding,
            None,
            false,
            ProviderDeadline::from_budget(Duration::from_secs(5)),
            &cancellation,
        )
        .unwrap();
    let mut state = admitted.staged_state().unwrap();
    state
        .replace("Module.bsl", b"before", b"published".to_vec())
        .unwrap();
    let prepared = admitted.prepare(state).unwrap();
    let raced_target = target.clone();
    let raced_moved = moved.clone();
    let cancel = cancellation.clone();
    crate::compile_transaction::set_retained_apply_before_post_validation_hook(move || {
        std::fs::rename(&raced_target, &raced_moved).unwrap();
        std::fs::write(&raced_target, b"foreign").unwrap();
        cancel.cancel();
    });

    let typed = fixture.actor.publish_prepared_apply(prepared).unwrap_err();

    assert_eq!(
        typed.kind(),
        super::ApplyPublicationErrorKind::RollbackIncomplete
    );
    assert!(typed.contains("rollback encountered"));
    assert_eq!(std::fs::read(&target).unwrap(), b"foreign");
    assert_eq!(std::fs::read(&moved).unwrap(), b"published");
    fixture.cleanup();
}

pub fn apply_admission_and_dry_run_are_cache_tree_write_free(
    policies: crate::policies::WorkspacePolicies,
) {
    let _policy_scope = policies_scope(policies);

    for existing_cache in [false, true] {
        let fixture = actor_fixture("apply-observation-write-free", &["src"]);
        let target = fixture.roots[0].join("Module.bsl");
        std::fs::write(&target, b"original").unwrap();
        let binding = fixture
            .actor
            .bind_provider_root("src", &fixture.roots[0])
            .unwrap();
        let cache_root = fixture.root.join(".build/unica");
        assert!(!cache_root.exists());
        if existing_cache {
            std::fs::create_dir_all(cache_root.join("source-revisions/empty")).unwrap();
            std::fs::write(
                cache_root.join("source-revisions/keep.bin"),
                b"retained cache",
            )
            .unwrap();
        }
        let before = snapshot_tree(&cache_root);

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
        assert_eq!(snapshot_tree(&cache_root), before);

        let mut state = admitted.staged_state().unwrap();
        state
            .replace("Module.bsl", b"original", b"dry-run".to_vec())
            .unwrap();
        fixture
            .actor
            .publish_prepared_apply(admitted.prepare(state).unwrap())
            .unwrap();

        assert_eq!(snapshot_tree(&cache_root), before);
        assert_eq!(std::fs::read(&target).unwrap(), b"original");
        fixture.cleanup();
    }
}

pub fn prepared_apply_dry_run_is_byte_identical_and_real_apply_commits_the_same_plan_once(
    policies: crate::policies::WorkspacePolicies,
) {
    let _policy_scope = policies_scope(policies);

    let fixture = actor_fixture("prepared-apply", &["src"]);
    let target = fixture.roots[0].join("Module.bsl");
    std::fs::write(&target, b"original").unwrap();
    let binding = fixture
        .actor
        .bind_provider_root("src", &fixture.roots[0])
        .unwrap();
    let cancellation = CancellationToken::new();
    let admitted = fixture
        .actor
        .admit_apply(
            &binding,
            None,
            true,
            ProviderDeadline::from_budget(Duration::from_secs(5)),
            &cancellation,
        )
        .unwrap();
    let mut state = admitted.staged_state().unwrap();
    state
        .replace("Module.bsl", b"original", b"published".to_vec())
        .unwrap();
    state
        .create("Ext/Form/Module.bsl", b"published-create".to_vec())
        .unwrap();
    let dry_result = fixture
        .actor
        .publish_prepared_apply(admitted.prepare(state).unwrap())
        .unwrap();
    let preview_plan = dry_result.rev().to_string();
    assert_eq!(std::fs::read(&target).unwrap(), b"original");
    assert!(!fixture.roots[0].join("Ext").exists());
    assert_eq!(dry_result.commit_count_for_test(), 0);
    assert!(dry_result.cleanup_diagnostics().is_empty());

    let admitted = fixture
        .actor
        .admit_apply(
            &binding,
            Some(&preview_plan),
            false,
            ProviderDeadline::from_budget(Duration::from_secs(5)),
            &cancellation,
        )
        .unwrap();
    let mut state = admitted.staged_state().unwrap();
    state
        .replace("Module.bsl", b"original", b"published".to_vec())
        .unwrap();
    state
        .create("Ext/Form/Module.bsl", b"published-create".to_vec())
        .unwrap();
    let result = fixture
        .actor
        .publish_prepared_apply(admitted.prepare(state).unwrap())
        .unwrap();
    assert_eq!(std::fs::read(&target).unwrap(), b"published");
    assert_eq!(
        std::fs::read(fixture.roots[0].join("Ext/Form/Module.bsl")).unwrap(),
        b"published-create"
    );
    assert_eq!(result.rev(), preview_plan);
    assert_eq!(result.commit_count_for_test(), 1);
    assert!(result.cleanup_diagnostics().is_empty());
    fixture.cleanup();
}

pub fn apply_selection_rejects_v8project_kind_change_after_prepare(
    policies: crate::policies::WorkspacePolicies,
) {
    let _policy_scope = policies_scope(policies);

    let fixture = actor_fixture("selection-kind-change", &["src/cf"]);
    let target = fixture.roots[0].join("Module.bsl");
    std::fs::write(&target, b"before").unwrap();
    let binding = fixture
        .actor
        .bind_provider_root("src/cf", &fixture.roots[0])
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
    let mut state = admitted.staged_state().unwrap();
    state
        .replace("Module.bsl", b"before", b"after".to_vec())
        .unwrap();
    let prepared = admitted
        .prepare_with_cache_effects(
            state,
            &[unica_source_contract::events::DomainEvent::new(
                unica_source_contract::events::DomainEventKind::MetadataChanged,
                "Catalog.Products",
            )],
        )
        .unwrap();
    let source_before = snapshot_tree(&fixture.roots[0]);
    let cache_before = snapshot_tree(&fixture.root.join(".build/unica"));
    std::fs::write(
        fixture.root.join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: src/cf\n    type: EXTENSION\n    path: src/cf\n",
    )
    .unwrap();

    let error = fixture
        .actor
        .publish_prepared_apply(prepared)
        .expect_err("changed source-map kind published and returned a committed receipt");

    assert_eq!(
        error.kind(),
        super::ApplyPublicationErrorKind::SourceSelectionChanged
    );
    assert_eq!(snapshot_tree(&fixture.roots[0]), source_before);
    assert_eq!(
        snapshot_tree(&fixture.root.join(".build/unica")),
        cache_before
    );
    fixture.cleanup();
}

pub fn apply_selection_rejects_v8project_absence_to_appearance_after_prepare(
    policies: crate::policies::WorkspacePolicies,
) {
    let _policy_scope = policies_scope(policies);

    let fixture = actor_fixture_without_source_map("selection-map-appears", "src/cf");
    std::fs::write(
        fixture.roots[0].join("Configuration.xml"),
        b"<MetaDataObject/>",
    )
    .unwrap();
    let target = fixture.roots[0].join("Module.bsl");
    std::fs::write(&target, b"before").unwrap();
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
    let mut state = admitted.staged_state().unwrap();
    state
        .replace("Module.bsl", b"before", b"after".to_vec())
        .unwrap();
    let prepared = admitted
        .prepare_with_cache_effects(
            state,
            &[unica_source_contract::events::DomainEvent::new(
                unica_source_contract::events::DomainEventKind::MetadataChanged,
                "Catalog.Products",
            )],
        )
        .unwrap();
    let source_before = snapshot_tree(&fixture.roots[0]);
    let cache_before = snapshot_tree(&fixture.root.join(".build/unica"));
    std::fs::write(
            fixture.root.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src/cf\n",
        )
        .unwrap();

    let error = fixture.actor.publish_prepared_apply(prepared).expect_err(
        "an appearing semantically equivalent source map published and returned a receipt",
    );

    assert_eq!(
        error.kind(),
        super::ApplyPublicationErrorKind::SourceSelectionChanged
    );
    assert_eq!(snapshot_tree(&fixture.roots[0]), source_before);
    assert_eq!(
        snapshot_tree(&fixture.root.join(".build/unica")),
        cache_before
    );
    fixture.cleanup();
}

pub fn apply_selection_rejects_autodetected_extension_membership_change(
    policies: crate::policies::WorkspacePolicies,
) {
    let _policy_scope = policies_scope(policies);

    let fixture = actor_fixture_without_source_map("selection-extension-membership", "src/cf");
    std::fs::write(
        fixture.roots[0].join("Configuration.xml"),
        b"<MetaDataObject/>",
    )
    .unwrap();
    std::fs::create_dir_all(fixture.root.join("src/cfe")).unwrap();
    let target = fixture.roots[0].join("Module.bsl");
    std::fs::write(&target, b"before").unwrap();
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
    let mut state = admitted.staged_state().unwrap();
    state
        .replace("Module.bsl", b"before", b"after".to_vec())
        .unwrap();
    let prepared = admitted
        .prepare_with_cache_effects(
            state,
            &[unica_source_contract::events::DomainEvent::new(
                unica_source_contract::events::DomainEventKind::MetadataChanged,
                "Catalog.Products",
            )],
        )
        .unwrap();
    let source_before = snapshot_tree(&fixture.roots[0]);
    let cache_before = snapshot_tree(&fixture.root.join(".build/unica"));
    std::fs::create_dir_all(fixture.root.join("src/cfe/late")).unwrap();
    std::fs::write(
        fixture.root.join("src/cfe/late/Configuration.xml"),
        b"<MetaDataObject/>",
    )
    .unwrap();

    let error = fixture
        .actor
        .publish_prepared_apply(prepared)
        .expect_err("changed autodetection membership published and returned a committed receipt");

    assert_eq!(
        error.kind(),
        super::ApplyPublicationErrorKind::SourceSelectionChanged
    );
    assert_eq!(snapshot_tree(&fixture.roots[0]), source_before);
    assert_eq!(
        snapshot_tree(&fixture.root.join(".build/unica")),
        cache_before
    );
    fixture.cleanup();
}

pub fn apply_selection_rejects_unselected_declared_parent_appearance(
    policies: crate::policies::WorkspacePolicies,
) {
    let _policy_scope = policies_scope(policies);

    let fixture = actor_fixture_without_source_map("selection-parent-appears", "src/cf");
    std::fs::write(
        fixture.roots[0].join("Configuration.xml"),
        b"<MetaDataObject/>",
    )
    .unwrap();
    std::fs::write(
        fixture.root.join("v8project.yaml"),
        concat!(
            "format: EDT\n",
            "source-set:\n",
            "  - name: main\n",
            "    type: CONFIGURATION\n",
            "    path: src/cf\n",
            "  - name: optional-edt\n",
            "    type: CONFIGURATION\n",
            "    path: optional/edt\n",
        ),
    )
    .unwrap();
    let target = fixture.roots[0].join("Module.bsl");
    std::fs::write(&target, b"before").unwrap();
    assert!(!fixture.root.join("optional").exists());
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
    let mut state = admitted.staged_state().unwrap();
    state
        .replace("Module.bsl", b"before", b"after".to_vec())
        .unwrap();
    let prepared = admitted
        .prepare_with_cache_effects(
            state,
            &[unica_source_contract::events::DomainEvent::new(
                unica_source_contract::events::DomainEventKind::MetadataChanged,
                "Catalog.Products",
            )],
        )
        .unwrap();
    let cache_before = snapshot_tree(&fixture.root.join(".build/unica"));
    std::fs::create_dir_all(fixture.root.join("optional/edt")).unwrap();
    std::fs::write(
        fixture.root.join("optional/edt/Configuration.xml"),
        b"<MetaDataObject/>",
    )
    .unwrap();
    std::fs::write(fixture.root.join("optional/edt/.project"), b"project").unwrap();

    let error = fixture
        .actor
        .publish_prepared_apply(prepared)
        .expect_err("appearing intermediate full-map parent published and returned a receipt");

    assert_eq!(
        error.kind(),
        super::ApplyPublicationErrorKind::SourceSelectionChanged
    );
    assert_eq!(std::fs::read(&target).unwrap(), b"before");
    assert_eq!(
        snapshot_tree(&fixture.root.join(".build/unica")),
        cache_before
    );
    fixture.cleanup();
}

pub fn apply_selection_rejects_unselected_non_platform_map_input_change(
    policies: crate::policies::WorkspacePolicies,
) {
    let _policy_scope = policies_scope(policies);

    let fixture = actor_fixture_without_source_map("selection-unselected-edt", "src/cf");
    std::fs::write(
        fixture.roots[0].join("Configuration.xml"),
        b"<MetaDataObject/>",
    )
    .unwrap();
    std::fs::create_dir_all(fixture.root.join("src/edt")).unwrap();
    std::fs::write(fixture.root.join("src/edt/.project"), b"project").unwrap();
    std::fs::write(
        fixture.root.join("v8project.yaml"),
        concat!(
            "format: DESIGNER\n",
            "source-set:\n",
            "  - name: main\n",
            "    type: CONFIGURATION\n",
            "    path: src/cf\n",
            "  - name: edt\n",
            "    type: CONFIGURATION\n",
            "    path: src/edt\n",
        ),
    )
    .unwrap();
    let target = fixture.roots[0].join("Module.bsl");
    std::fs::write(&target, b"before").unwrap();
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
    let mut state = admitted.staged_state().unwrap();
    state
        .replace("Module.bsl", b"before", b"after".to_vec())
        .unwrap();
    let prepared = admitted
        .prepare_with_cache_effects(
            state,
            &[unica_source_contract::events::DomainEvent::new(
                unica_source_contract::events::DomainEventKind::MetadataChanged,
                "Catalog.Products",
            )],
        )
        .unwrap();
    let source_before = snapshot_tree(&fixture.roots[0]);
    let cache_before = snapshot_tree(&fixture.root.join(".build/unica"));
    std::fs::write(
        fixture.root.join("src/edt/Configuration.xml"),
        b"<MetaDataObject/>",
    )
    .unwrap();

    let error = fixture.actor.publish_prepared_apply(prepared).expect_err(
        "unselected EDT-to-invalid map change published and returned a committed receipt",
    );

    assert_eq!(
        error.kind(),
        super::ApplyPublicationErrorKind::SourceSelectionChanged
    );
    assert_eq!(snapshot_tree(&fixture.roots[0]), source_before);
    assert_eq!(
        snapshot_tree(&fixture.root.join(".build/unica")),
        cache_before
    );
    fixture.cleanup();
}

pub fn apply_selection_rejects_repaired_large_unselected_external_descriptor(
    policies: crate::policies::WorkspacePolicies,
) {
    let _policy_scope = policies_scope(policies);

    let fixture = actor_fixture_without_source_map("selection-repaired-large-external", "src/cf");
    std::fs::write(
        fixture.roots[0].join("Configuration.xml"),
        b"<MetaDataObject/>",
    )
    .unwrap();
    std::fs::create_dir_all(fixture.root.join("epf")).unwrap();
    std::fs::write(
        fixture.root.join("v8project.yaml"),
        concat!(
            "format: EDT\n",
            "source-set:\n",
            "  - name: main\n",
            "    type: CONFIGURATION\n",
            "    path: src/cf\n",
            "  - name: unselected-processors\n",
            "    type: EXTERNAL_DATA_PROCESSORS\n",
            "    path: epf\n",
        ),
    )
    .unwrap();
    let descriptor = fixture.root.join("epf/ConfigDumpInfo.xml");
    std::fs::write(&descriptor, vec![b'x'; 8 * 1024 * 1024 + 1]).unwrap();
    let target = fixture.roots[0].join("Module.bsl");
    std::fs::write(&target, b"before").unwrap();
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
    let mut state = admitted.staged_state().unwrap();
    state
        .replace("Module.bsl", b"before", b"after".to_vec())
        .unwrap();
    let prepared = admitted
        .prepare_with_cache_effects(
            state,
            &[unica_source_contract::events::DomainEvent::new(
                unica_source_contract::events::DomainEventKind::MetadataChanged,
                "Catalog.Products",
            )],
        )
        .unwrap();
    let source_before = snapshot_tree(&fixture.roots[0]);
    let cache_before = snapshot_tree(&fixture.root.join(".build/unica"));
    std::fs::write(
        descriptor,
        b"<MetaDataObject><ExternalDataProcessor/></MetaDataObject>",
    )
    .unwrap();

    let error = fixture
        .actor
        .publish_prepared_apply(prepared)
        .expect_err("repaired large unselected descriptor published and returned a receipt");

    assert_eq!(
        error.kind(),
        super::ApplyPublicationErrorKind::SourceSelectionChanged
    );
    assert_eq!(snapshot_tree(&fixture.roots[0]), source_before);
    assert_eq!(
        snapshot_tree(&fixture.root.join(".build/unica")),
        cache_before
    );
    fixture.cleanup();
}

pub fn apply_selection_dry_run_rejects_late_map_change_without_receipt(
    policies: crate::policies::WorkspacePolicies,
) {
    let _policy_scope = policies_scope(policies);

    let fixture = actor_fixture("selection-dry-late", &["src/cf"]);
    let target = fixture.roots[0].join("Module.bsl");
    std::fs::write(&target, b"before").unwrap();
    let binding = fixture
        .actor
        .bind_provider_root("src/cf", &fixture.roots[0])
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
    let mut state = admitted.staged_state().unwrap();
    state
        .replace("Module.bsl", b"before", b"projected".to_vec())
        .unwrap();
    let prepared = admitted
        .prepare_with_cache_effects(
            state,
            &[unica_source_contract::events::DomainEvent::new(
                unica_source_contract::events::DomainEventKind::MetadataChanged,
                "Catalog.Products",
            )],
        )
        .unwrap();
    let source_before = snapshot_tree(&fixture.roots[0]);
    let cache_before = snapshot_tree(&fixture.root.join(".build/unica"));
    let map = fixture.root.join("v8project.yaml");
    set_apply_dry_run_after_confirmation_hook(move || {
        std::fs::write(
                map,
                "format: DESIGNER\nsource-set:\n  - name: src/cf\n    type: EXTENSION\n    path: src/cf\n",
            )
            .unwrap();
    });

    let error = fixture
        .actor
        .publish_prepared_apply(prepared)
        .expect_err("late dry-run source-map change returned a projected result and receipt");

    assert_eq!(
        error.kind(),
        super::ApplyPublicationErrorKind::SourceSelectionChanged
    );
    assert_eq!(snapshot_tree(&fixture.roots[0]), source_before);
    assert_eq!(
        snapshot_tree(&fixture.root.join(".build/unica")),
        cache_before
    );
    fixture.cleanup();
}

pub fn apply_selection_late_change_rolls_back_source_cache_revision_and_receipt(
    policies: crate::policies::WorkspacePolicies,
) {
    let _policy_scope = policies_scope(policies);

    let fixture = actor_fixture_without_source_map("selection-real-late", "src/cf");
    std::fs::write(
        fixture.roots[0].join("Configuration.xml"),
        b"<MetaDataObject/>",
    )
    .unwrap();
    std::fs::create_dir_all(fixture.root.join("src/edt")).unwrap();
    std::fs::write(fixture.root.join("src/edt/.project"), b"project").unwrap();
    std::fs::write(
        fixture.root.join("v8project.yaml"),
        concat!(
            "format: DESIGNER\n",
            "source-set:\n",
            "  - name: main\n",
            "    type: CONFIGURATION\n",
            "    path: src/cf\n",
            "  - name: edt\n",
            "    type: CONFIGURATION\n",
            "    path: src/edt\n",
        ),
    )
    .unwrap();
    let target = fixture.roots[0].join("Module.bsl");
    std::fs::write(&target, b"before").unwrap();
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
    let mut state = admitted.staged_state().unwrap();
    state
        .replace("Module.bsl", b"before", b"published".to_vec())
        .unwrap();
    let prepared = admitted
        .prepare_with_cache_effects(
            state,
            &[unica_source_contract::events::DomainEvent::new(
                unica_source_contract::events::DomainEventKind::MetadataChanged,
                "Catalog.Products",
            )],
        )
        .unwrap();
    let source_before = snapshot_tree(&fixture.roots[0]);
    let cache_before = snapshot_tree(&fixture.root.join(".build/unica"));
    let late_marker = fixture.root.join("src/edt/Configuration.xml");
    crate::compile_transaction::set_retained_apply_before_post_validation_hook(move || {
        std::fs::write(late_marker, b"<MetaDataObject/>").unwrap()
    });

    let error = fixture
        .actor
        .publish_prepared_apply(prepared)
        .expect_err("late full-map change committed source/cache/revision and returned a receipt");

    assert_eq!(
        error.kind(),
        super::ApplyPublicationErrorKind::SourceSelectionChanged
    );
    assert_eq!(snapshot_tree(&fixture.roots[0]), source_before);
    assert_eq!(
        snapshot_tree(&fixture.root.join(".build/unica")),
        cache_before
    );
    fixture.cleanup();
}

pub fn apply_selection_rejects_autodetection_container_identity_replacement(
    policies: crate::policies::WorkspacePolicies,
) {
    let _policy_scope = policies_scope(policies);

    let fixture = actor_fixture_without_source_map("selection-container-swap", "src/cf");
    std::fs::write(
        fixture.roots[0].join("Configuration.xml"),
        b"<MetaDataObject/>",
    )
    .unwrap();
    let container = fixture.root.join("src/cfe");
    std::fs::create_dir_all(&container).unwrap();
    let target = fixture.roots[0].join("Module.bsl");
    std::fs::write(&target, b"before").unwrap();
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
    let mut state = admitted.staged_state().unwrap();
    state
        .replace("Module.bsl", b"before", b"after".to_vec())
        .unwrap();
    let prepared = admitted
        .prepare_with_cache_effects(
            state,
            &[unica_source_contract::events::DomainEvent::new(
                unica_source_contract::events::DomainEventKind::MetadataChanged,
                "Catalog.Products",
            )],
        )
        .unwrap();
    let source_before = snapshot_tree(&fixture.roots[0]);
    let cache_before = snapshot_tree(&fixture.root.join(".build/unica"));
    std::fs::rename(&container, fixture.root.join("src/cfe-displaced")).unwrap();
    std::fs::create_dir(&container).unwrap();

    let error = fixture.actor.publish_prepared_apply(prepared).expect_err(
        "replacement container with identical membership published and returned a receipt",
    );

    assert_eq!(
        error.kind(),
        super::ApplyPublicationErrorKind::SourceSelectionChanged
    );
    assert_eq!(snapshot_tree(&fixture.roots[0]), source_before);
    assert_eq!(
        snapshot_tree(&fixture.root.join(".build/unica")),
        cache_before
    );
    fixture.cleanup();
}

pub fn apply_preparation_preserves_all_six_typed_staging_error_kinds(
    policies: crate::policies::WorkspacePolicies,
) {
    let _policy_scope = policies_scope(policies);

    let mut observed = Vec::new();

    let fixture = actor_fixture("prepare-kind-cancelled", &["src"]);
    let binding = fixture
        .actor
        .bind_provider_root("src", &fixture.roots[0])
        .unwrap();
    let cancellation = CancellationToken::new();
    let admitted = fixture
        .actor
        .admit_apply(
            &binding,
            None,
            false,
            ProviderDeadline::from_budget(Duration::from_secs(5)),
            &cancellation,
        )
        .unwrap();
    let state = admitted.staged_state().unwrap();
    cancellation.cancel();
    observed.push((
        ApplyStagingErrorKind::Cancelled,
        admitted.prepare(state).unwrap_err().kind(),
    ));
    fixture.cleanup();

    let fixture = actor_fixture("prepare-kind-deadline", &["src"]);
    let binding = fixture
        .actor
        .bind_provider_root("src", &fixture.roots[0])
        .unwrap();
    let mut admitted = fixture
        .actor
        .admit_apply(
            &binding,
            None,
            false,
            ProviderDeadline::from_budget(Duration::from_secs(5)),
            &CancellationToken::new(),
        )
        .unwrap();
    let state = admitted.staged_state().unwrap();
    admitted.deadline = ProviderDeadline::from_budget(Duration::ZERO);
    observed.push((
        ApplyStagingErrorKind::Deadline,
        admitted.prepare(state).unwrap_err().kind(),
    ));
    fixture.cleanup();

    let fixture = actor_fixture("prepare-kind-containment", &["src"]);
    std::fs::write(fixture.roots[0].join("Module.bsl"), b"original").unwrap();
    let binding = fixture
        .actor
        .bind_provider_root("src", &fixture.roots[0])
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
    let other_fixture = actor_fixture("prepare-kind-containment-other", &["src"]);
    std::fs::write(other_fixture.roots[0].join("Module.bsl"), b"original").unwrap();
    let other_binding = other_fixture
        .actor
        .bind_provider_root("src", &other_fixture.roots[0])
        .unwrap();
    let mut state = ApplyStagedState::from_retained_root(
        Arc::clone(&other_binding.source_root),
        admitted.deadline,
        admitted.cancellation.clone(),
        admitted.writer_authority.clone(),
    );
    state
        .replace("Module.bsl", b"original", b"replacement".to_vec())
        .unwrap();
    observed.push((
        ApplyStagingErrorKind::ContainmentIdentity,
        admitted.prepare(state).unwrap_err().kind(),
    ));
    other_fixture.cleanup();
    fixture.cleanup();

    let fixture = actor_fixture("prepare-kind-occupied", &["src"]);
    std::fs::create_dir(fixture.roots[0].join("Ext")).unwrap();
    let binding = fixture
        .actor
        .bind_provider_root("src", &fixture.roots[0])
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
    let mut state = admitted.staged_state().unwrap();
    state
        .create("Ext/Form/Module.bsl", b"created".to_vec())
        .unwrap();
    std::fs::create_dir(fixture.roots[0].join("Ext/Form")).unwrap();
    observed.push((
        ApplyStagingErrorKind::AbsentChainOccupied,
        admitted.prepare(state).unwrap_err().kind(),
    ));
    fixture.cleanup();

    let fixture = actor_fixture("prepare-kind-provider", &["src"]);
    let binding = fixture
        .actor
        .bind_provider_root("src", &fixture.roots[0])
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
    let mut state = admitted.staged_state().unwrap();
    state.create("Module.bsl", b"created".to_vec()).unwrap();
    crate::compile_transaction::inject_retained_apply_validation_provider_failure_for_test();
    observed.push((
        ApplyStagingErrorKind::UnsupportedProvider,
        admitted.prepare(state).unwrap_err().kind(),
    ));
    fixture.cleanup();

    let fixture = actor_fixture("prepare-kind-invariant", &["src"]);
    let binding = fixture
        .actor
        .bind_provider_root("src", &fixture.roots[0])
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
    let mut state = admitted.staged_state().unwrap();
    state
        .create("Malformed.xml", b"<not-closed".to_vec())
        .unwrap();
    observed.push((
        ApplyStagingErrorKind::Invariant,
        admitted.prepare(state).unwrap_err().kind(),
    ));
    fixture.cleanup();

    assert_eq!(
        observed.len(),
        6,
        "typed staging error matrix is incomplete"
    );
    for (expected, actual) in observed {
        assert_eq!(actual, expected, "actor preparation erased {expected:?}");
    }
}

pub fn apply_preparation_classifies_an_occupied_absent_final_file_by_cause(
    policies: crate::policies::WorkspacePolicies,
) {
    let _policy_scope = policies_scope(policies);

    let fixture = actor_fixture("prepare-occupied-absent-final", &["src"]);
    std::fs::create_dir(fixture.roots[0].join("Ext")).unwrap();
    let binding = fixture
        .actor
        .bind_provider_root("src", &fixture.roots[0])
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
    let mut state = admitted.staged_state().unwrap();
    state
        .create("Ext/Module.bsl", b"must not replace".to_vec())
        .unwrap();
    std::fs::write(
        fixture.roots[0].join("Ext/Module.bsl"),
        b"external regular file",
    )
    .unwrap();

    let error = admitted.prepare(state).unwrap_err();

    assert_eq!(error.kind(), ApplyStagingErrorKind::AbsentChainOccupied);
    assert_eq!(
        std::fs::read(fixture.roots[0].join("Ext/Module.bsl")).unwrap(),
        b"external regular file"
    );
    fixture.cleanup();
}

pub fn prepared_apply_cleanup_race_surfaces_a_relative_actor_diagnostic(
    policies: crate::policies::WorkspacePolicies,
) {
    let _policy_scope = policies_scope(policies);

    if !crate::platform::testing::can_swap_named_child_behind_retained_handle_for_test() {
        return;
    }
    let fixture = actor_fixture("prepared-cleanup-diagnostic", &["src"]);
    let parent = fixture.roots[0].join("Nested");
    std::fs::create_dir_all(&parent).unwrap();
    let target = parent.join("Module.bsl");
    let owned_recovery = parent.join("owned-recovery.bsl");
    std::fs::write(&target, b"original").unwrap();
    let binding = fixture
        .actor
        .bind_provider_root("src", &fixture.roots[0])
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
    let mut state = admitted.staged_state().unwrap();
    state
        .replace("Nested/Module.bsl", b"original", b"published".to_vec())
        .unwrap();
    let prepared = admitted.prepare(state).unwrap();
    let hook_parent = parent.clone();
    let hook_owned = owned_recovery.clone();
    crate::platform::filesystem::set_before_identity_bound_cleanup_mutation_hook(move || {
        let recovery = std::fs::read_dir(&hook_parent)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with(".unica-apply-"))
            })
            .expect("retained apply recovery artifact");
        std::fs::rename(&recovery, &hook_owned).unwrap();
        std::fs::write(&recovery, b"concurrent-recovery").unwrap();
    });

    let result = fixture.actor.publish_prepared_apply(prepared).unwrap();

    assert_eq!(std::fs::read(&target).unwrap(), b"published");
    assert_eq!(std::fs::read(&owned_recovery).unwrap(), b"original");
    assert!(std::fs::read_dir(&parent).unwrap().any(|entry| {
        let path = entry.unwrap().path();
        path.file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with(".unica-apply-"))
            && std::fs::read(path).unwrap() == b"concurrent-recovery"
    }));
    let [diagnostic] = result.cleanup_diagnostics() else {
        panic!("successful actor result discarded the retained cleanup diagnostic: {result:?}");
    };
    assert_eq!(
        diagnostic.kind(),
        super::ApplyCleanupDiagnosticKind::RetainedRecoveryCleanupIncomplete
    );
    assert_eq!(diagnostic.logical_target(), Path::new("Nested/Module.bsl"));
    let artifact_name = diagnostic.last_known_artifact_name();
    assert!(artifact_name.to_string_lossy().starts_with(".unica-apply-"));
    assert_eq!(Path::new(artifact_name).components().count(), 1);
    assert!(!format!("{diagnostic:?}").contains(&fixture.root.display().to_string()));
    fixture.cleanup();
}

pub fn prepared_apply_rejects_stale_revision_source_change_cancellation_and_deadline_before_publication(
    policies: crate::policies::WorkspacePolicies,
) {
    let _policy_scope = policies_scope(policies);

    let fixture = actor_fixture("prepared-apply-fences", &["src"]);
    let target = fixture.roots[0].join("Module.bsl");
    std::fs::write(&target, b"original").unwrap();
    let binding = fixture
        .actor
        .bind_provider_root("src", &fixture.roots[0])
        .unwrap();
    let cancellation = CancellationToken::new();
    let stale = fixture
        .actor
        .admit_apply(
            &binding,
            Some("stale-revision"),
            false,
            ProviderDeadline::from_budget(Duration::from_secs(5)),
            &cancellation,
        )
        .unwrap();
    let mut stale_state = stale.staged_state().unwrap();
    stale_state
        .replace("Module.bsl", b"original", b"must-not-publish".to_vec())
        .unwrap();
    let error = stale.prepare(stale_state).unwrap_err();
    assert_eq!(error.kind(), ApplyStagingErrorKind::ConcurrentRevision);
    assert!(error.to_string().contains("ifRev"));

    let admitted = fixture
        .actor
        .admit_apply(
            &binding,
            None,
            false,
            ProviderDeadline::from_budget(Duration::from_secs(5)),
            &cancellation,
        )
        .unwrap();
    let mut state = admitted.staged_state().unwrap();
    state
        .replace("Module.bsl", b"original", b"must-not-publish".to_vec())
        .unwrap();
    let prepared = admitted.prepare(state).unwrap();
    std::fs::write(&target, b"concurrent").unwrap();
    let error = fixture.actor.publish_prepared_apply(prepared).unwrap_err();
    assert_eq!(
        error.kind(),
        super::ApplyPublicationErrorKind::ConcurrentRevision
    );
    assert_eq!(std::fs::read(&target).unwrap(), b"concurrent");

    let cancelled = CancellationToken::new();
    let admitted = fixture
        .actor
        .admit_apply(
            &binding,
            None,
            false,
            ProviderDeadline::from_budget(Duration::from_secs(5)),
            &cancelled,
        )
        .unwrap();
    let mut state = admitted.staged_state().unwrap();
    state
        .replace("Module.bsl", b"concurrent", b"cancelled".to_vec())
        .unwrap();
    let prepared = admitted.prepare(state).unwrap();
    cancelled.cancel();
    assert!(fixture
        .actor
        .publish_prepared_apply(prepared)
        .unwrap_err()
        .contains("cancel"));
    assert_eq!(std::fs::read(&target).unwrap(), b"concurrent");

    assert!(fixture
        .actor
        .admit_apply(
            &binding,
            None,
            false,
            ProviderDeadline::from_budget(Duration::ZERO),
            &CancellationToken::new(),
        )
        .unwrap_err()
        .to_string()
        .contains("deadline"));
    fixture.cleanup();
}

pub fn prepared_apply_root_and_actor_capabilities_cannot_be_redirected_or_replayed(
    policies: crate::policies::WorkspacePolicies,
) {
    let _policy_scope = policies_scope(policies);

    let first = actor_fixture("prepared-capability-a", &["src"]);
    let second = actor_fixture("prepared-capability-b", &["src"]);
    let target = first.roots[0].join("Module.bsl");
    std::fs::write(&target, b"original").unwrap();
    let binding = first
        .actor
        .bind_provider_root("src", &first.roots[0])
        .unwrap();
    let admitted = first
        .actor
        .admit_apply(
            &binding,
            None,
            false,
            ProviderDeadline::from_budget(Duration::from_secs(5)),
            &CancellationToken::new(),
        )
        .unwrap();
    let mut state = admitted.staged_state().unwrap();
    state
        .replace("Module.bsl", b"original", b"must-not-publish".to_vec())
        .unwrap();
    let prepared = admitted.prepare(state).unwrap();
    assert!(second
        .actor
        .publish_prepared_apply(prepared)
        .unwrap_err()
        .contains("another workspace actor"));
    assert_eq!(std::fs::read(&target).unwrap(), b"original");

    let admitted = first
        .actor
        .admit_apply(
            &binding,
            None,
            false,
            ProviderDeadline::from_budget(Duration::from_secs(5)),
            &CancellationToken::new(),
        )
        .unwrap();
    let mut state = admitted.staged_state().unwrap();
    state
        .replace("Module.bsl", b"original", b"redirected".to_vec())
        .unwrap();
    let prepared = admitted.prepare(state).unwrap();
    let displaced = first.root.join("src-displaced");
    let retained_identity = path_identity_for_test(&first.roots[0])
        .unwrap()
        .expect("source root identity must be available on supported CI platforms");
    match attempt_retained_directory_replacement_for_test(&first.roots[0], &displaced).unwrap() {
        RetainedDirectoryReplacementOutcome::Replaced => {
            std::fs::create_dir_all(&first.roots[0]).unwrap();
            std::fs::write(first.roots[0].join("Module.bsl"), b"replacement-tree").unwrap();
            assert!(first
                .actor
                .publish_prepared_apply(prepared)
                .unwrap_err()
                .contains("physical identity"));
            assert_eq!(
                std::fs::read(displaced.join("Module.bsl")).unwrap(),
                b"original"
            );
            assert_eq!(
                std::fs::read(first.roots[0].join("Module.bsl")).unwrap(),
                b"replacement-tree"
            );
        }
        RetainedDirectoryReplacementOutcome::PreventedByRetainedHandle => {
            assert_eq!(
                path_identity_for_test(&first.roots[0]).unwrap().as_deref(),
                Some(retained_identity.as_str())
            );
            assert!(!displaced.exists());
            first.actor.publish_prepared_apply(prepared).unwrap();
            assert_eq!(std::fs::read(&target).unwrap(), b"redirected");
        }
    }
    first.cleanup();
    second.cleanup();
}

pub fn prepared_apply_root_replacement_during_commit_rolls_back_before_result(
    policies: crate::policies::WorkspacePolicies,
) {
    let _policy_scope = policies_scope(policies);

    let fixture = actor_fixture("prepared-root-race", &["src"]);
    let target = fixture.roots[0].join("Module.bsl");
    std::fs::write(&target, b"original").unwrap();
    let binding = fixture
        .actor
        .bind_provider_root("src", &fixture.roots[0])
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
    let mut state = admitted.staged_state().unwrap();
    state
        .replace("Module.bsl", b"original", b"published-postimage".to_vec())
        .unwrap();
    let prepared = admitted.prepare(state).unwrap();
    let named_root = fixture.roots[0].clone();
    let displaced = fixture.root.join("src-race-displaced");
    let hook_displaced = displaced.clone();
    let retained_identity = path_identity_for_test(&named_root)
        .unwrap()
        .expect("source root identity must be available on supported CI platforms");
    let (replacement_tx, replacement_rx) = mpsc::sync_channel(1);
    crate::compile_transaction::set_retained_apply_before_post_validation_hook(move || {
        let replacement =
            attempt_retained_directory_replacement_for_test(&named_root, &hook_displaced).unwrap();
        if replacement == RetainedDirectoryReplacementOutcome::Replaced {
            std::fs::create_dir_all(&named_root).unwrap();
            std::fs::write(named_root.join("Module.bsl"), b"replacement-tree").unwrap();
        }
        replacement_tx.send(replacement).unwrap();
    });

    let publication = fixture.actor.publish_prepared_apply(prepared);
    let replacement = replacement_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("post-validation replacement hook did not report its platform outcome");
    match replacement {
        RetainedDirectoryReplacementOutcome::Replaced => {
            let error = publication.unwrap_err();
            assert!(
                error.contains("physical identity")
                    || error.contains("identity changed after admission"),
                "{error}"
            );
            assert_eq!(
                std::fs::read(displaced.join("Module.bsl")).unwrap(),
                b"original"
            );
            assert_eq!(
                std::fs::read(fixture.roots[0].join("Module.bsl")).unwrap(),
                b"replacement-tree"
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
            publication.unwrap();
            assert_eq!(std::fs::read(&target).unwrap(), b"published-postimage");
        }
    }
    fixture.cleanup();
}

pub fn postimage_validation_preserves_typed_root_containment_cause(
    policies: crate::policies::WorkspacePolicies,
) {
    let _policy_scope = policies_scope(policies);

    let fixture = actor_fixture("typed-postimage-root", &["src"]);
    let target = fixture.roots[0].join("Module.bsl");
    std::fs::write(&target, b"original").unwrap();
    let binding = fixture
        .actor
        .bind_provider_root("src", &fixture.roots[0])
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
    let mut state = admitted.staged_state().unwrap();
    state
        .replace("Module.bsl", b"original", b"published".to_vec())
        .unwrap();
    let prepared = admitted.prepare(state).unwrap();
    let named_root = fixture.roots[0].clone();
    let displaced = fixture.root.join("typed-postimage-root-displaced");
    let hook_displaced = displaced.clone();
    let retained_identity = path_identity_for_test(&named_root)
        .unwrap()
        .expect("source root identity must be available on supported CI platforms");
    let (replacement_tx, replacement_rx) = mpsc::sync_channel(1);
    crate::compile_transaction::set_retained_apply_before_postimages_hook(move || {
        let replacement =
            attempt_retained_directory_replacement_for_test(&named_root, &hook_displaced).unwrap();
        if replacement == RetainedDirectoryReplacementOutcome::Replaced {
            std::fs::create_dir_all(&named_root).unwrap();
            std::fs::write(named_root.join("foreign.txt"), b"foreign").unwrap();
        }
        replacement_tx.send(replacement).unwrap();
    });

    let publication = fixture.actor.publish_prepared_apply(prepared);
    let replacement = replacement_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("postimage replacement hook did not report its platform outcome");
    match replacement {
        RetainedDirectoryReplacementOutcome::Replaced => {
            let error = publication.unwrap_err();
            assert_eq!(
                error.kind(),
                super::ApplyPublicationErrorKind::ContainmentIdentity
            );
            assert_eq!(
                std::fs::read(displaced.join("Module.bsl")).unwrap(),
                b"original"
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
            publication.unwrap();
            assert_eq!(std::fs::read(&target).unwrap(), b"published");
        }
    }
    fixture.cleanup();
}

pub fn postimage_validation_preserves_typed_absent_chain_containment_cause(
    policies: crate::policies::WorkspacePolicies,
) {
    let _policy_scope = policies_scope(policies);

    let fixture = actor_fixture("typed-postimage-absent-chain", &["src"]);
    let binding = fixture
        .actor
        .bind_provider_root("src", &fixture.roots[0])
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
    let mut state = admitted.staged_state().unwrap();
    state
        .create("Nested/Module.bsl", b"published".to_vec())
        .unwrap();
    let prepared = admitted.prepare(state).unwrap();
    let nested = fixture.roots[0].join("Nested");
    let displaced = fixture.roots[0].join("Nested-displaced");
    let hook_nested = nested.clone();
    let hook_displaced = displaced.clone();
    let (replacement_tx, replacement_rx) = mpsc::sync_channel(1);
    crate::compile_transaction::set_retained_apply_before_postimages_hook(move || {
        let retained_identity = path_identity_for_test(&hook_nested)
            .unwrap()
            .expect("created parent identity must be available on supported CI platforms");
        let replacement =
            attempt_retained_directory_replacement_for_test(&hook_nested, &hook_displaced).unwrap();
        if replacement == RetainedDirectoryReplacementOutcome::Replaced {
            std::fs::create_dir_all(&hook_nested).unwrap();
            std::fs::write(hook_nested.join("foreign.txt"), b"foreign").unwrap();
            let rollback_nested = hook_nested.clone();
            let rollback_displaced = hook_displaced.clone();
            crate::compile_transaction::set_retained_apply_before_rollback_hook(move || {
                std::fs::remove_dir_all(&rollback_nested).unwrap();
                std::fs::rename(&rollback_displaced, &rollback_nested).unwrap();
            });
        }
        replacement_tx
            .send((replacement, retained_identity))
            .unwrap();
    });

    let publication = fixture.actor.publish_prepared_apply(prepared);
    let (replacement, retained_identity) = replacement_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("absent-chain replacement hook did not report its platform outcome");
    match replacement {
        RetainedDirectoryReplacementOutcome::Replaced => {
            let error = publication.unwrap_err();
            assert_eq!(
                error.kind(),
                super::ApplyPublicationErrorKind::ContainmentIdentity
            );
            assert!(
                !nested.exists(),
                "transaction-created parent was not rolled back"
            );
            assert!(!displaced.exists(), "test race recovery name leaked");
        }
        RetainedDirectoryReplacementOutcome::PreventedByRetainedHandle => {
            assert_eq!(
                path_identity_for_test(&nested).unwrap().as_deref(),
                Some(retained_identity.as_str())
            );
            assert!(!displaced.exists());
            publication.unwrap();
            assert_eq!(
                std::fs::read(nested.join("Module.bsl")).unwrap(),
                b"published"
            );
        }
    }
    fixture.cleanup();
}

pub fn prepared_apply_create_post_rename_failure_rolls_back_the_published_name(
    policies: crate::policies::WorkspacePolicies,
) {
    let _policy_scope = policies_scope(policies);

    let fixture = actor_fixture("prepared-create-post-rename-failure", &["src"]);
    let target = fixture.roots[0].join("created.bsl");
    let binding = fixture
        .actor
        .bind_provider_root("src", &fixture.roots[0])
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
    let mut state = admitted.staged_state().unwrap();
    state.create("created.bsl", b"published".to_vec()).unwrap();
    let prepared = admitted.prepare(state).unwrap();
    crate::platform::filesystem::inject_post_rename_sync_failure_for_test();

    let error = fixture.actor.publish_prepared_apply(prepared).unwrap_err();
    assert!(error.contains("post-rename"), "{error}");
    assert!(
        !target.exists(),
        "reported failure left created bytes published"
    );
    fixture.cleanup();
}

pub fn apply_admission_refuses_staged_state_from_another_actor_issued_authority(
    policies: crate::policies::WorkspacePolicies,
) {
    let _policy_scope = policies_scope(policies);

    let fixture = actor_fixture("prepared-foreign-writer-authority", &["src"]);
    let binding = fixture
        .actor
        .bind_provider_root("src", &fixture.roots[0])
        .unwrap();
    let cancellation = CancellationToken::new();
    let first = fixture
        .actor
        .admit_apply(
            &binding,
            None,
            true,
            ProviderDeadline::from_budget(Duration::from_secs(5)),
            &cancellation,
        )
        .unwrap();
    let second = fixture
        .actor
        .admit_apply(
            &binding,
            None,
            true,
            ProviderDeadline::from_budget(Duration::from_secs(5)),
            &cancellation,
        )
        .unwrap();
    let foreign_state = second.staged_state().unwrap();

    let error = first.prepare(foreign_state).unwrap_err();
    assert_eq!(error.kind(), ApplyStagingErrorKind::Invariant);
    fixture.cleanup();
}

pub fn prepared_apply_replace_post_rename_failure_restores_the_exact_preimage(
    policies: crate::policies::WorkspacePolicies,
) {
    let _policy_scope = policies_scope(policies);

    let fixture = actor_fixture("prepared-replace-post-rename-failure", &["src"]);
    let target = fixture.roots[0].join("Module.bsl");
    std::fs::write(&target, b"original").unwrap();
    let binding = fixture
        .actor
        .bind_provider_root("src", &fixture.roots[0])
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
    let mut state = admitted.staged_state().unwrap();
    state
        .replace("Module.bsl", b"original", b"published".to_vec())
        .unwrap();
    let prepared = admitted.prepare(state).unwrap();
    crate::platform::filesystem::inject_post_rename_sync_failure_for_test();

    let error = fixture.actor.publish_prepared_apply(prepared).unwrap_err();
    assert!(error.contains("post-rename"), "{error}");
    assert_eq!(std::fs::read(&target).unwrap(), b"original");
    fixture.cleanup();
}

pub fn prepared_apply_replace_preserves_a_destination_replaced_at_the_mutation_boundary(
    policies: crate::policies::WorkspacePolicies,
) {
    let _policy_scope = policies_scope(policies);

    let fixture = actor_fixture("prepared-replace-destination-race", &["src"]);
    let target = fixture.roots[0].join("Module.bsl");
    let displaced = fixture.roots[0].join("concurrent-old.bsl");
    std::fs::write(&target, b"original").unwrap();
    let binding = fixture
        .actor
        .bind_provider_root("src", &fixture.roots[0])
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
    let mut state = admitted.staged_state().unwrap();
    state
        .replace("Module.bsl", b"original", b"apply-bytes".to_vec())
        .unwrap();
    let prepared = admitted.prepare(state).unwrap();
    let raced_target = target.clone();
    let raced_displaced = displaced.clone();
    crate::platform::filesystem::set_before_identity_bound_no_replace_rename_hook(move || {
        std::fs::rename(&raced_target, &raced_displaced).unwrap();
        std::fs::write(&raced_target, b"concurrent").unwrap();
    });

    let error = fixture.actor.publish_prepared_apply(prepared).unwrap_err();
    assert!(
        error.contains("preimage") || error.contains("identity"),
        "{error}"
    );
    assert_eq!(std::fs::read(&target).unwrap(), b"concurrent");
    fixture.cleanup();
}

pub fn prepared_apply_remove_preserves_a_destination_replaced_at_the_mutation_boundary(
    policies: crate::policies::WorkspacePolicies,
) {
    let _policy_scope = policies_scope(policies);

    let fixture = actor_fixture("prepared-remove-destination-race", &["src"]);
    let target = fixture.roots[0].join("Module.bsl");
    let displaced = fixture.roots[0].join("concurrent-old.bsl");
    std::fs::write(&target, b"original").unwrap();
    let binding = fixture
        .actor
        .bind_provider_root("src", &fixture.roots[0])
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
    let mut state = admitted.staged_state().unwrap();
    state.remove("Module.bsl", b"original").unwrap();
    let prepared = admitted.prepare(state).unwrap();
    let raced_target = target.clone();
    let raced_displaced = displaced.clone();
    crate::platform::filesystem::set_before_identity_bound_no_replace_rename_hook(move || {
        std::fs::rename(&raced_target, &raced_displaced).unwrap();
        std::fs::write(&raced_target, b"concurrent").unwrap();
    });

    let error = fixture.actor.publish_prepared_apply(prepared).unwrap_err();
    assert!(
        error.contains("preimage") || error.contains("identity"),
        "{error}"
    );
    assert_eq!(std::fs::read(&target).unwrap(), b"concurrent");
    fixture.cleanup();
}

pub fn prepared_apply_dry_run_rejects_root_replacement_before_result(
    policies: crate::policies::WorkspacePolicies,
) {
    let _policy_scope = policies_scope(policies);

    let fixture = actor_fixture("prepared-dry-root-race", &["src"]);
    let target = fixture.roots[0].join("Module.bsl");
    std::fs::write(&target, b"original").unwrap();
    let binding = fixture
        .actor
        .bind_provider_root("src", &fixture.roots[0])
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
    let mut state = admitted.staged_state().unwrap();
    state
        .replace("Module.bsl", b"original", b"dry-postimage".to_vec())
        .unwrap();
    let prepared = admitted.prepare(state).unwrap();
    let named_root = fixture.roots[0].clone();
    let displaced = fixture.root.join("src-dry-race-displaced");
    let hook_displaced = displaced.clone();
    let retained_identity = path_identity_for_test(&named_root)
        .unwrap()
        .expect("source root identity must be available on supported CI platforms");
    let (replacement_tx, replacement_rx) = mpsc::sync_channel(1);
    set_apply_dry_run_after_confirmation_hook(move || {
        let replacement =
            attempt_retained_directory_replacement_for_test(&named_root, &hook_displaced).unwrap();
        if replacement == RetainedDirectoryReplacementOutcome::Replaced {
            std::fs::create_dir_all(&named_root).unwrap();
            std::fs::write(named_root.join("Module.bsl"), b"replacement-tree").unwrap();
        }
        replacement_tx.send(replacement).unwrap();
    });

    let publication = fixture.actor.publish_prepared_apply(prepared);
    let replacement = replacement_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("dry-run replacement hook did not report its platform outcome");
    match replacement {
        RetainedDirectoryReplacementOutcome::Replaced => {
            let error = publication.unwrap_err();
            assert!(error.contains("physical identity"), "{error}");
            assert_eq!(
                std::fs::read(displaced.join("Module.bsl")).unwrap(),
                b"original"
            );
            assert_eq!(
                std::fs::read(fixture.roots[0].join("Module.bsl")).unwrap(),
                b"replacement-tree"
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
            publication.unwrap();
            assert_eq!(std::fs::read(&target).unwrap(), b"original");
        }
    }
    fixture.cleanup();
}

pub fn saved_apply_plans_do_not_retain_per_change_descriptors(
    policies: crate::policies::WorkspacePolicies,
) {
    let _policy_scope = policies_scope(policies);

    use crate::platform::testing::open_handle_count_for_test;

    const PLAN_COUNT: usize = 8;
    const CHANGES_PER_PLAN: usize = 16;

    let fixture = actor_fixture("saved-plan-bounded-write-descriptors", &["main"]);
    let source = &fixture.roots[0];
    for index in 0..CHANGES_PER_PLAN {
        let relative = PathBuf::from(format!("Nested/Item{index}/Ext/Module.bsl"));
        std::fs::create_dir_all(source.join(relative.parent().unwrap())).unwrap();
        std::fs::write(source.join(relative), b"before").unwrap();
    }
    let cache_state = fixture.root.join(".build/unica/state.json");
    std::fs::create_dir_all(cache_state.parent().unwrap()).unwrap();
    std::fs::write(
        &cache_state,
        serde_json::to_vec(&serde_json::json!({
            "workspace_root": fixture.root.display().to_string(),
            "workspace_epoch": 1,
            "caches": {}
        }))
        .unwrap(),
    )
    .unwrap();
    let binding = fixture.actor.bind_provider_root("main", source).unwrap();
    let before = open_handle_count_for_test().unwrap();

    let mut tokens = Vec::new();
    let mut after_one_change_plan = before;
    for plan in 0..=PLAN_COUNT {
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
        let mut effects = PlannedApplyEffects::default();
        let change_count = if plan == 0 { 1 } else { CHANGES_PER_PLAN };
        for index in 0..change_count {
            let relative = PathBuf::from(format!("Nested/Item{index}/Ext/Module.bsl"));
            state
                .replace(
                    &relative,
                    b"before",
                    format!("after-{plan}-{index}").into_bytes(),
                )
                .unwrap();
            effects.append(unica_source_contract::events::DomainEvent::new(
                DomainEventKind::ModuleChanged,
                format!("main:{}", relative.display()),
            ));
        }
        let batch = admission.prepare_with_effects(state, effects).unwrap();
        let preview = fixture.actor.preview_prepared_apply(&batch).unwrap();
        tokens.push(
            fixture
                .actor
                .save_prepared_apply(
                    batch,
                    unica_execution_contract::invocation::DomainResult::success(format!(
                        "preview-{}",
                        preview.rev()
                    )),
                )
                .unwrap(),
        );
        if plan == 0 {
            after_one_change_plan = open_handle_count_for_test().unwrap();
        }
    }

    let after = open_handle_count_for_test().unwrap();
    let fixed_plan_cost = after_one_change_plan.saturating_sub(before);
    let many_change_plan_cost = after.saturating_sub(after_one_change_plan);
    assert!(
            many_change_plan_cost <= PLAN_COUNT * fixed_plan_cost,
            "{PLAN_COUNT} pending plans with {CHANGES_PER_PLAN} changes retained {many_change_plan_cost} handles after one-change plan cost {fixed_plan_cost}"
        );
    assert_eq!(tokens.len(), PLAN_COUNT + 1);
    fixture.cleanup();
}

pub fn saved_source_diff_detects_changed_inputs_without_consuming_plan(
    policies: crate::policies::WorkspacePolicies,
    renderer: crate::preview::SavedSourceDiffFactory,
) {
    let _policy_scope = policies_scope(policies);

    let fixture = actor_fixture("saved-source-diff-stale", &["main"]);
    let source = &fixture.roots[0];
    let path = source.join("Module.bsl");
    std::fs::write(&path, b"before").unwrap();
    std::fs::write(source.join("Other.bsl"), b"unrelated").unwrap();
    let binding = fixture.actor.bind_provider_root("main", source).unwrap();
    let cancellation = CancellationToken::new();
    let admission = fixture
        .actor
        .admit_apply(
            &binding,
            None,
            true,
            ProviderDeadline::no_deadline(),
            &cancellation,
        )
        .unwrap();
    let mut state = admission.staged_state().unwrap();
    state
        .replace("Module.bsl", b"before", b"after".to_vec())
        .unwrap();
    let batch = admission.prepare(state).unwrap();
    fixture.actor.preview_prepared_apply(&batch).unwrap();
    let token = fixture
        .actor
        .save_prepared_apply(
            batch,
            unica_execution_contract::invocation::DomainResult::success("preview"),
        )
        .unwrap();
    std::fs::write(source.join("Other.bsl"), b"unrelated edit").unwrap();
    let page = fixture
        .actor
        .read_saved_apply_diff(&token, 100, None, &cancellation, renderer)
        .unwrap();
    assert_eq!(page["chunks"][0]["content"], "before");
    assert_eq!(page["chunks"][1]["content"], "after");
    std::fs::write(&path, b"foreign edit").unwrap();
    assert!(
        matches!(fixture.actor.read_saved_apply_diff(&token, 100, None, &cancellation, renderer), Err(super::SavedApplyExecutionError::Publication(error)) if error.kind() == super::ApplyPublicationErrorKind::ConcurrentRevision)
    );
    let entry = fixture.actor.saved_apply_plans.lock().unwrap().entries[&token].clone();
    assert!(matches!(
        *entry.state.lock().unwrap(),
        super::SavedApplyState::Pending { .. }
    ));
    let result = fixture
        .actor
        .execute_saved_apply(
            &token,
            ProviderDeadline::no_deadline(),
            &cancellation,
            finish_saved_apply_test,
        )
        .unwrap();
    assert!(!result.ok);
    assert_eq!(std::fs::read(&path).unwrap(), b"foreign edit");
    fixture.cleanup();
}

pub fn saved_apply_rebinds_nested_namespace_guards_to_the_execution_budget(
    policies: crate::policies::WorkspacePolicies,
) {
    let _policy_scope = policies_scope(policies);

    let fixture = actor_fixture("saved-plan-nested-namespace-budget", &["main"]);
    let source = &fixture.roots[0];
    std::fs::create_dir_all(source.join("Nested")).unwrap();
    std::fs::write(source.join("Module.bsl"), b"before").unwrap();
    std::fs::write(source.join("Nested/Keep.bsl"), b"unchanged").unwrap();
    let binding = fixture.actor.bind_provider_root("main", source).unwrap();
    let planning_cancellation = CancellationToken::new();
    let admission = fixture
        .actor
        .admit_apply(
            &binding,
            None,
            true,
            ProviderDeadline::from_budget(Duration::from_secs(5)),
            &planning_cancellation,
        )
        .unwrap();
    let mut state = admission.staged_state().unwrap();
    let files = state.enumerate_tree(Path::new("")).unwrap();
    assert!(files.contains(&PathBuf::from("Nested/Keep.bsl")));
    state
        .replace("Module.bsl", b"before", b"after".to_vec())
        .unwrap();
    let mut batch = admission.prepare(state).unwrap();
    fixture.actor.preview_prepared_apply(&batch).unwrap();
    planning_cancellation.cancel();
    batch.deadline = ProviderDeadline::from_budget(Duration::ZERO);
    let token = fixture
        .actor
        .save_prepared_apply(
            batch,
            unica_execution_contract::invocation::DomainResult::success("preview"),
        )
        .unwrap();

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
    assert_eq!(std::fs::read(source.join("Module.bsl")).unwrap(), b"after");
    assert_eq!(
        std::fs::read(source.join("Nested/Keep.bsl")).unwrap(),
        b"unchanged"
    );
    fixture.cleanup();
}

pub fn saved_apply_large_retained_inputs_can_be_saved_and_executed(
    policies: crate::policies::WorkspacePolicies,
) {
    let _policy_scope = policies_scope(policies);

    let fixture = actor_fixture("saved-plan-large-inputs", &["main"]);
    let source = &fixture.roots[0];
    let body = vec![b'x'; 11 * 1024 * 1024];
    for index in 0..3 {
        std::fs::write(source.join(format!("Input{index}.bsl")), &body).unwrap();
    }
    std::fs::write(source.join("Module.bsl"), b"before").unwrap();
    let binding = fixture.actor.bind_provider_root("main", source).unwrap();
    let admission = fixture
        .actor
        .admit_apply(
            &binding,
            None,
            true,
            ProviderDeadline::from_budget(Duration::from_secs(10)),
            &CancellationToken::new(),
        )
        .unwrap();
    let mut state = admission.staged_state().unwrap();
    // Three actual read-only inputs retain more than the former 64 MiB
    // ceiling once their original/current states are captured.
    for index in 0..3 {
        assert_eq!(
            state
                .read(Path::new(&format!("Input{index}.bsl")))
                .unwrap()
                .unwrap(),
            body
        );
    }
    state
        .replace("Module.bsl", b"before", b"after".to_vec())
        .unwrap();
    let batch = admission.prepare(state).unwrap();
    fixture.actor.preview_prepared_apply(&batch).unwrap();
    let token = fixture
        .actor
        .save_prepared_apply(
            batch,
            unica_execution_contract::invocation::DomainResult::success("preview"),
        )
        .expect("a valid plan must not be refused by an unmeasured byte quota");
    assert_eq!(std::fs::read(source.join("Module.bsl")).unwrap(), b"before");
    let published = fixture
        .actor
        .execute_saved_apply(
            &token,
            ProviderDeadline::from_budget(Duration::from_secs(10)),
            &CancellationToken::new(),
            finish_saved_apply_test,
        )
        .unwrap();
    assert!(published.ok, "{}", published.summary);
    assert_eq!(std::fs::read(source.join("Module.bsl")).unwrap(), b"after");
    for index in 0..3 {
        assert_eq!(
            std::fs::read(source.join(format!("Input{index}.bsl"))).unwrap(),
            body
        );
    }
    fixture.cleanup();
}

pub fn saved_apply_shared_cache_preimage_refuses_second_token_without_overwriting_either_source(
    policies: crate::policies::WorkspacePolicies,
) {
    let _policy_scope = policies_scope(policies);

    let fixture = actor_fixture("saved-plan-shared-cache-overlap", &["main"]);
    let source = &fixture.roots[0];
    let module_a = source.join("ModuleA.bsl");
    let module_b = source.join("ModuleB.bsl");
    std::fs::write(&module_a, b"A before").unwrap();
    std::fs::write(&module_b, b"B before").unwrap();
    // Start from an existing valid state with fresh cache entries absent.
    // A changes this exact preimage; B must then see a cache-byte conflict.
    let cache_state = fixture.root.join(".build/unica/state.json");
    std::fs::create_dir_all(cache_state.parent().unwrap()).unwrap();
    std::fs::write(
        &cache_state,
        serde_json::to_vec(&serde_json::json!({
            "workspace_root": fixture.root.display().to_string(),
            "workspace_epoch": 1,
            "caches": {}
        }))
        .unwrap(),
    )
    .unwrap();
    let cache_root = cache_state.parent().unwrap();
    let foreign_cache = cache_root.join(".unica-apply-spoof");
    std::fs::write(&foreign_cache, b"foreign cache bytes").unwrap();
    let binding = fixture.actor.bind_provider_root("main", source).unwrap();
    let prepare = |name: &str, before: &[u8], after: &[u8]| {
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
        state.replace(name, before, after.to_vec()).unwrap();
        let mut effects = PlannedApplyEffects::default();
        effects.append(unica_source_contract::events::DomainEvent::new(
            DomainEventKind::ModuleChanged,
            format!("main:{name}"),
        ));
        let batch = admission.prepare_with_effects(state, effects).unwrap();
        fixture.actor.preview_prepared_apply(&batch).unwrap();
        batch
    };
    let preview = unica_execution_contract::invocation::DomainResult::success("preview");
    let token_a = fixture
        .actor
        .save_prepared_apply(
            prepare("ModuleA.bsl", b"A before", b"A after"),
            preview.clone(),
        )
        .unwrap();
    let token_b = fixture
        .actor
        .save_prepared_apply(
            prepare("ModuleB.bsl", b"B before", b"B after"),
            preview.clone(),
        )
        .unwrap();
    assert_eq!(std::fs::read(&module_a).unwrap(), b"A before");
    assert_eq!(std::fs::read(&module_b).unwrap(), b"B before");
    let mut expected_cache_after_a = snapshot_tree(cache_root);

    let first = fixture
        .actor
        .execute_saved_apply(
            &token_a,
            ProviderDeadline::from_budget(Duration::from_secs(5)),
            &CancellationToken::new(),
            finish_saved_apply_test,
        )
        .unwrap();
    assert!(first.ok, "{}", first.summary);
    assert_eq!(std::fs::read(&module_a).unwrap(), b"A after");
    assert_eq!(std::fs::read(&module_b).unwrap(), b"B before");
    let state_after_a = std::fs::read(&cache_state).unwrap();
    let expected_state = &mut expected_cache_after_a
        .iter_mut()
        .find(|(relative, _)| relative == Path::new("state.json"))
        .expect("the existing cache marker is part of the preimage")
        .1;
    assert_ne!(*expected_state, Some(state_after_a.clone()));
    *expected_state = Some(state_after_a);
    // B is still pending and holds A's former marker. A's recovery copy
    // of that marker must leave the namespace now: a delete-pending name
    // cannot be reopened and broke reading the tree on Windows (#1463).
    assert_eq!(snapshot_tree(cache_root), expected_cache_after_a);
    assert_eq!(
        snapshot_tree(source)
            .into_iter()
            .filter(|(relative, _)| relative
                .file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with(".unica-apply-")))
            .count(),
        0,
        "a successful apply leaves no recovery names in the source tree"
    );
    let second_kind = std::cell::Cell::new(None);
    let second = fixture
        .actor
        .execute_saved_apply(
            &token_b,
            ProviderDeadline::from_budget(Duration::from_secs(5)),
            &CancellationToken::new(),
            |result, publication| {
                second_kind.set(publication.as_ref().err().map(|error| error.kind()));
                finish_saved_apply_test(result, publication)
            },
        )
        .unwrap();
    assert!(
        !second.ok,
        "the shared cache preimage must reject the old B plan"
    );
    assert_eq!(
        second_kind.get(),
        Some(super::ApplyPublicationErrorKind::ConcurrentRevision)
    );
    assert!(second.summary.contains("state.json"), "{}", second.summary);
    assert!(
        second.summary.contains("shared cache"),
        "{}",
        second.summary
    );
    assert!(
        second.summary.contains("atomic publication"),
        "{}",
        second.summary
    );
    assert_eq!(std::fs::read(&module_a).unwrap(), b"A after");
    assert_eq!(std::fs::read(&module_b).unwrap(), b"B before");
    assert_eq!(snapshot_tree(cache_root), expected_cache_after_a);

    let fresh_b = fixture
        .actor
        .save_prepared_apply(prepare("ModuleB.bsl", b"B before", b"B after"), preview)
        .unwrap();
    let fresh_result = fixture
        .actor
        .execute_saved_apply(
            &fresh_b,
            ProviderDeadline::from_budget(Duration::from_secs(5)),
            &CancellationToken::new(),
            finish_saved_apply_test,
        )
        .unwrap();
    assert!(fresh_result.ok, "{}", fresh_result.summary);
    assert_eq!(std::fs::read(&module_a).unwrap(), b"A after");
    assert_eq!(std::fs::read(&module_b).unwrap(), b"B after");
    let refusal_replay = fixture
        .actor
        .execute_saved_apply(
            &token_b,
            ProviderDeadline::from_budget(Duration::from_secs(5)),
            &CancellationToken::new(),
            |_, _| panic!("refused saved plan must replay its terminal result"),
        )
        .unwrap();
    assert_eq!(refusal_replay, second);
    fixture.cleanup();
}

pub fn saved_apply_read_only_input_conflict_explains_dependency_without_publishing(
    policies: crate::policies::WorkspacePolicies,
) {
    let _policy_scope = policies_scope(policies);

    for after_publication in [false, true] {
        let fixture = actor_fixture("saved-plan-read-only-context", &["main"]);
        let source = &fixture.roots[0];
        std::fs::write(source.join("Module.bsl"), b"before").unwrap();
        std::fs::write(source.join("Dependency.bsl"), b"dependency").unwrap();
        let binding = fixture.actor.bind_provider_root("main", source).unwrap();
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
        state.read(Path::new("Dependency.bsl")).unwrap();
        state
            .replace("Module.bsl", b"before", b"after".to_vec())
            .unwrap();
        let batch = admission
            .prepare_with_effects(state, Default::default())
            .unwrap();
        fixture.actor.preview_prepared_apply(&batch).unwrap();
        let token = fixture
            .actor
            .save_prepared_apply(
                batch,
                unica_execution_contract::invocation::DomainResult::success("preview"),
            )
            .unwrap();
        if after_publication {
            let hook_source = source.clone();
            crate::compile_transaction::set_retained_apply_before_postimages_hook(move || {
                assert_eq!(
                    std::fs::read(hook_source.join("Module.bsl")).unwrap(),
                    b"after"
                );
                std::fs::write(hook_source.join("Dependency.bsl"), b"changed").unwrap();
            });
        } else {
            std::fs::write(source.join("Dependency.bsl"), b"changed").unwrap();
        }
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
        assert!(!result.ok);
        assert!(
            result.summary.contains("Dependency.bsl"),
            "{}",
            result.summary
        );
        assert!(
            result.summary.contains("read-only input"),
            "{}",
            result.summary
        );
        assert!(
            !result.summary.contains("source write target"),
            "{}",
            result.summary
        );
        assert_eq!(std::fs::read(source.join("Module.bsl")).unwrap(), b"before");
        assert_eq!(
            std::fs::read(source.join("Dependency.bsl")).unwrap(),
            b"changed"
        );
        assert_eq!(
            snapshot_tree(&fixture.root.join(".build/unica")),
            cache_before
        );
        fixture.cleanup();
    }
}

pub fn apply_policy_preserves_workspace_ancestor_precedence_over_source_local_policy(
    policies: crate::policies::WorkspacePolicies,
) {
    let _policy_scope = policies_scope(policies);

    let container = temp_root("apply-policy-precedence");
    let workspace = container.join("worktrees/one");
    let source = workspace.join("src");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::write(
        workspace.join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
    )
    .unwrap();
    std::fs::write(
        source.join("Configuration.xml"),
        "<MetaDataObject><Configuration/></MetaDataObject>",
    )
    .unwrap();
    std::fs::write(
        container.join(".v8-project.json"),
        br#"{"editingAllowedCheck":"warn"}"#,
    )
    .unwrap();
    std::fs::write(
        source.join(".v8-project.json"),
        br#"{"editingAllowedCheck":"off"}"#,
    )
    .unwrap();
    let context = context(&workspace);
    let identity = WorkspaceIdentity::new(
        &context,
        [source_input("main", source.as_path())],
        "test-provider",
    )
    .unwrap();
    let actor = super::WorkspaceActor::<(), ()>::new(identity, context)
        .map(bind_actor)
        .unwrap();
    let binding = actor.bind_provider_root("main", &source).unwrap();

    let admission = actor
        .admit_apply(
            &binding,
            None,
            true,
            ProviderDeadline::from_budget(Duration::from_secs(5)),
            &CancellationToken::new(),
        )
        .unwrap();

    assert_eq!(admission.support_policy_mode(), SupportPolicyMode::Warn);
    std::fs::remove_dir_all(container).unwrap();
}

pub fn apply_policy_absent_chain_rejects_nearer_policy_insertion_before_publication(
    policies: crate::policies::WorkspacePolicies,
) {
    let _policy_scope = policies_scope(policies);

    let fixture = actor_fixture("apply-policy-absent-insertion", &["src"]);
    let target = fixture.roots[0].join("Module.bsl");
    std::fs::write(&target, b"original").unwrap();
    let binding = fixture
        .actor
        .bind_provider_root("src", &fixture.roots[0])
        .unwrap();
    let cancellation = CancellationToken::new();
    let admission = fixture
        .actor
        .admit_apply(
            &binding,
            None,
            false,
            ProviderDeadline::from_budget(Duration::from_secs(5)),
            &cancellation,
        )
        .unwrap();
    let mut state = admission.staged_state().unwrap();
    state
        .replace("Module.bsl", b"original", b"published".to_vec())
        .unwrap();
    let prepared = admission.prepare(state).unwrap();
    let cache_before = snapshot_tree(&fixture.root.join(".build/unica"));
    std::fs::write(
        fixture.root.join(".v8-project.json"),
        br#"{"editingAllowedCheck":"off"}"#,
    )
    .unwrap();

    let error = fixture.actor.publish_prepared_apply(prepared).unwrap_err();

    assert_eq!(
        error.kind(),
        super::ApplyPublicationErrorKind::ContainmentIdentity
    );
    assert_eq!(std::fs::read(&target).unwrap(), b"original");
    assert_eq!(
        snapshot_tree(&fixture.root.join(".build/unica")),
        cache_before
    );
    fixture.cleanup();
}

pub fn apply_policy_exact_file_rejects_byte_change_and_rename_replacement(
    policies: crate::policies::WorkspacePolicies,
) {
    let _policy_scope = policies_scope(policies);

    let mut rejected = Vec::new();
    for replacement in [false, true] {
        let fixture = actor_fixture(
            if replacement {
                "apply-policy-identity-replacement"
            } else {
                "apply-policy-byte-change"
            },
            &["src"],
        );
        let target = fixture.roots[0].join("Module.bsl");
        let policy = fixture.root.join(".v8-project.json");
        std::fs::write(&target, b"original").unwrap();
        std::fs::write(&policy, br#"{"editingAllowedCheck":"off"}"#).unwrap();
        let binding = fixture
            .actor
            .bind_provider_root("src", &fixture.roots[0])
            .unwrap();
        let admission = fixture
            .actor
            .admit_apply(
                &binding,
                None,
                false,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        assert_eq!(admission.support_policy_mode(), SupportPolicyMode::Off);
        let mut state = admission.staged_state().unwrap();
        state
            .replace("Module.bsl", b"original", b"published".to_vec())
            .unwrap();
        let prepared = admission.prepare(state).unwrap();
        if replacement {
            std::fs::rename(&policy, fixture.root.join("policy-displaced.json")).unwrap();
            std::fs::write(&policy, br#"{"editingAllowedCheck":"off"}"#).unwrap();
        } else {
            std::fs::write(&policy, br#"{"editingAllowedCheck":"warn"}"#).unwrap();
        }
        rejected.push(fixture.actor.publish_prepared_apply(prepared).is_err());
        fixture.cleanup();
    }
    assert_eq!(rejected, [true, true]);
}

pub fn apply_policy_stable_deny_evidence_allows_unrelated_dry_run_and_real_publication(
    policies: crate::policies::WorkspacePolicies,
) {
    let _policy_scope = policies_scope(policies);

    for category in ["wrong-kind", "oversized", "unreadable"] {
        for dry_run in [true, false] {
            let fixture = actor_fixture(
                &format!(
                    "apply-policy-stable-{category}-{}",
                    if dry_run { "dry" } else { "real" }
                ),
                &["src"],
            );
            let policy = fixture.root.join(".v8-project.json");
            let unreadable = match category {
                "wrong-kind" => {
                    std::fs::create_dir(&policy).unwrap();
                    false
                }
                "oversized" => {
                    std::fs::write(&policy, vec![b' '; 32 * 1024 * 1024 + 1]).unwrap();
                    false
                }
                "unreadable" => {
                    std::fs::write(&policy, br#"{"editingAllowedCheck":"off"}"#).unwrap();
                    if !set_unix_mode_for_test(&policy, 0o000).unwrap() {
                        eprintln!(
                                "[SKIPPED FIXTURE] unreadable support-policy publication requires Unix permission bits"
                            );
                        fixture.cleanup();
                        continue;
                    }
                    true
                }
                _ => unreachable!(),
            };
            let target = fixture.roots[0].join("Module.bsl");
            std::fs::write(&target, b"original").unwrap();
            let binding = fixture
                .actor
                .bind_provider_root("src", &fixture.roots[0])
                .unwrap();
            let admission = fixture
                .actor
                .admit_apply(
                    &binding,
                    None,
                    dry_run,
                    ProviderDeadline::from_budget(Duration::from_secs(10)),
                    &CancellationToken::new(),
                )
                .unwrap();
            assert_eq!(admission.support_policy_mode(), SupportPolicyMode::Deny);
            let mut state = admission.staged_state().unwrap();
            if !dry_run {
                state
                    .replace("Module.bsl", b"original", b"published".to_vec())
                    .unwrap();
            }
            let result = fixture
                .actor
                .publish_prepared_apply(admission.prepare(state).unwrap())
                .unwrap();

            assert_eq!(
                result.effects().disposition(),
                if dry_run {
                    ApplyEffectDisposition::Projected
                } else {
                    ApplyEffectDisposition::Committed
                },
                "category={category}, dry_run={dry_run}"
            );
            assert_eq!(
                result.commit_count_for_test(),
                usize::from(!dry_run),
                "category={category}, dry_run={dry_run}"
            );
            assert_eq!(
                std::fs::read(&target).unwrap(),
                if dry_run {
                    b"original".as_slice()
                } else {
                    b"published".as_slice()
                },
                "category={category}, dry_run={dry_run}"
            );
            if unreadable {
                assert!(set_unix_mode_for_test(&policy, 0o600).unwrap());
            }
            fixture.cleanup();
        }
    }
}

pub fn apply_policy_category_and_identity_transitions_are_rejected(
    policies: crate::policies::WorkspacePolicies,
) {
    let _policy_scope = policies_scope(policies);

    let wrong_kind = actor_fixture("apply-policy-wrong-kind", &["src"]);
    let policy = wrong_kind.root.join(".v8-project.json");
    std::fs::create_dir(&policy).unwrap();
    let binding = wrong_kind
        .actor
        .bind_provider_root("src", &wrong_kind.roots[0])
        .unwrap();
    let admission = wrong_kind
        .actor
        .admit_apply(
            &binding,
            None,
            true,
            ProviderDeadline::from_budget(Duration::from_secs(5)),
            &CancellationToken::new(),
        )
        .unwrap();
    assert_eq!(admission.support_policy_mode(), SupportPolicyMode::Deny);
    let state = admission.staged_state().unwrap();
    let prepared = admission.prepare(state).unwrap();
    std::fs::remove_dir(&policy).unwrap();
    std::fs::write(&policy, br#"{"editingAllowedCheck":"off"}"#).unwrap();
    assert_eq!(
        wrong_kind
            .actor
            .publish_prepared_apply(prepared)
            .unwrap_err()
            .kind(),
        super::ApplyPublicationErrorKind::ContainmentIdentity
    );
    wrong_kind.cleanup();

    let symlink = actor_fixture("apply-policy-symlink", &["src"]);
    let policy = symlink.root.join(".v8-project.json");
    let target = symlink.root.join("linked-policy.json");
    std::fs::write(&target, br#"{"editingAllowedCheck":"off"}"#).unwrap();
    let link_outcome = create_file_link_fixture_for_test(&target, &policy).unwrap();
    match link_outcome {
            FileLinkFixtureOutcome::Created => {
                let binding = symlink
                    .actor
                    .bind_provider_root("src", &symlink.roots[0])
                    .unwrap();
                let admission = symlink
                    .actor
                    .admit_apply(
                        &binding,
                        None,
                        true,
                        ProviderDeadline::from_budget(Duration::from_secs(5)),
                        &CancellationToken::new(),
                    )
                    .unwrap();
                assert_eq!(admission.support_policy_mode(), SupportPolicyMode::Deny);
                let state = admission.staged_state().unwrap();
                let prepared = admission.prepare(state).unwrap();
                std::fs::remove_file(&policy).unwrap();
                std::fs::write(&policy, br#"{"editingAllowedCheck":"off"}"#).unwrap();
                assert_eq!(
                    symlink
                        .actor
                        .publish_prepared_apply(prepared)
                        .unwrap_err()
                        .kind(),
                    super::ApplyPublicationErrorKind::ContainmentIdentity
                );
            }
            FileLinkFixtureOutcome::Unsupported => eprintln!(
                "[SKIPPED FIXTURE] support-policy symlink transition is unsupported on this platform"
            ),
            FileLinkFixtureOutcome::WindowsPrivilegeUnavailable => eprintln!(
                "[SKIPPED FIXTURE] support-policy symlink transition needs an unavailable Windows privilege"
            ),
        }
    symlink.cleanup();

    if can_swap_named_child_behind_retained_handle_for_test() {
        let oversized = actor_fixture("apply-policy-oversized-identity", &["src"]);
        let policy = oversized.root.join(".v8-project.json");
        std::fs::write(&policy, vec![b' '; 32 * 1024 * 1024 + 1]).unwrap();
        let binding = oversized
            .actor
            .bind_provider_root("src", &oversized.roots[0])
            .unwrap();
        let admission = oversized
            .actor
            .admit_apply(
                &binding,
                None,
                true,
                ProviderDeadline::from_budget(Duration::from_secs(10)),
                &CancellationToken::new(),
            )
            .unwrap();
        assert_eq!(admission.support_policy_mode(), SupportPolicyMode::Deny);
        let state = admission.staged_state().unwrap();
        let prepared = admission.prepare(state).unwrap();
        std::fs::rename(&policy, oversized.root.join("oversized-displaced.json")).unwrap();
        std::fs::write(&policy, vec![b' '; 32 * 1024 * 1024 + 1]).unwrap();
        assert_eq!(
            oversized
                .actor
                .publish_prepared_apply(prepared)
                .unwrap_err()
                .kind(),
            super::ApplyPublicationErrorKind::ContainmentIdentity
        );
        oversized.cleanup();
    } else {
        eprintln!(
                "[SKIPPED FIXTURE] oversized support-policy identity replacement is unsupported while a retained handle is open"
            );
    }

    let unreadable = actor_fixture("apply-policy-unreadable-transition", &["src"]);
    let policy = unreadable.root.join(".v8-project.json");
    std::fs::write(&policy, br#"{"editingAllowedCheck":"off"}"#).unwrap();
    if set_unix_mode_for_test(&policy, 0o000).unwrap() {
        let binding = unreadable
            .actor
            .bind_provider_root("src", &unreadable.roots[0])
            .unwrap();
        let admission = unreadable
            .actor
            .admit_apply(
                &binding,
                None,
                true,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        assert_eq!(admission.support_policy_mode(), SupportPolicyMode::Deny);
        let state = admission.staged_state().unwrap();
        let prepared = admission.prepare(state).unwrap();
        assert!(set_unix_mode_for_test(&policy, 0o600).unwrap());
        assert_eq!(
            unreadable
                .actor
                .publish_prepared_apply(prepared)
                .unwrap_err()
                .kind(),
            super::ApplyPublicationErrorKind::ContainmentIdentity
        );
    } else {
        eprintln!(
            "[SKIPPED FIXTURE] unreadable support-policy transition requires Unix permission bits"
        );
    }
    unreadable.cleanup();
}

pub fn apply_policy_dry_run_churn_is_write_free_and_returns_no_receipt(
    policies: crate::policies::WorkspacePolicies,
) {
    let _policy_scope = policies_scope(policies);

    let fixture = actor_fixture("apply-policy-dry-run-churn", &["src"]);
    let target = fixture.roots[0].join("Module.bsl");
    let policy = fixture.root.join(".v8-project.json");
    std::fs::write(&target, b"original").unwrap();
    std::fs::write(&policy, br#"{"editingAllowedCheck":"off"}"#).unwrap();
    let binding = fixture
        .actor
        .bind_provider_root("src", &fixture.roots[0])
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
    let prepared = admission.prepare(state).unwrap();
    let raced_policy = policy.clone();
    set_apply_dry_run_after_confirmation_hook(move || {
        std::fs::write(raced_policy, br#"{"editingAllowedCheck":"warn"}"#).unwrap();
    });

    let error = fixture.actor.publish_prepared_apply(prepared).unwrap_err();

    assert_eq!(
        error.kind(),
        super::ApplyPublicationErrorKind::ContainmentIdentity
    );
    assert_eq!(std::fs::read(&target).unwrap(), b"original");
    assert!(!fixture.root.join(".build/unica").exists());
    fixture.cleanup();
}

pub fn apply_policy_churn_before_source_publication_is_write_free(
    policies: crate::policies::WorkspacePolicies,
) {
    let _policy_scope = policies_scope(policies);

    let fixture = actor_fixture("apply-policy-prepublication-churn", &["src"]);
    let target = fixture.roots[0].join("Module.bsl");
    let policy = fixture.root.join(".v8-project.json");
    std::fs::write(&target, b"original").unwrap();
    std::fs::write(&policy, br#"{"editingAllowedCheck":"off"}"#).unwrap();
    let binding = fixture
        .actor
        .bind_provider_root("src", &fixture.roots[0])
        .unwrap();
    let admission = fixture
        .actor
        .admit_apply(
            &binding,
            None,
            false,
            ProviderDeadline::from_budget(Duration::from_secs(5)),
            &CancellationToken::new(),
        )
        .unwrap();
    let mut state = admission.staged_state().unwrap();
    state
        .replace("Module.bsl", b"original", b"published".to_vec())
        .unwrap();
    let prepared = admission.prepare(state).unwrap();
    std::fs::write(&policy, br#"{"editingAllowedCheck":"warn"}"#).unwrap();

    let error = fixture.actor.publish_prepared_apply(prepared).unwrap_err();

    assert_eq!(
        error.kind(),
        super::ApplyPublicationErrorKind::ContainmentIdentity
    );
    assert_eq!(std::fs::read(&target).unwrap(), b"original");
    assert!(!fixture.root.join(".build/unica").exists());
    fixture.cleanup();
}

pub fn apply_policy_churn_after_source_publication_rolls_back_all_retained_state(
    policies: crate::policies::WorkspacePolicies,
) {
    let _policy_scope = policies_scope(policies);

    let fixture = actor_fixture("apply-policy-postpublication-churn", &["src"]);
    let target = fixture.roots[0].join("Module.bsl");
    let policy = fixture.root.join(".v8-project.json");
    std::fs::write(&target, b"original").unwrap();
    std::fs::write(&policy, br#"{"editingAllowedCheck":"off"}"#).unwrap();
    let binding = fixture
        .actor
        .bind_provider_root("src", &fixture.roots[0])
        .unwrap();
    let admission = fixture
        .actor
        .admit_apply(
            &binding,
            None,
            false,
            ProviderDeadline::from_budget(Duration::from_secs(5)),
            &CancellationToken::new(),
        )
        .unwrap();
    let mut state = admission.staged_state().unwrap();
    state
        .replace("Module.bsl", b"original", b"published".to_vec())
        .unwrap();
    let prepared = admission.prepare(state).unwrap();
    let source_before = snapshot_tree(&fixture.roots[0]);
    let cache_before = snapshot_tree(&fixture.root.join(".build/unica"));
    let raced_policy = policy.clone();
    crate::compile_transaction::set_retained_apply_before_post_validation_hook(move || {
        std::fs::write(raced_policy, br#"{"editingAllowedCheck":"warn"}"#).unwrap();
    });

    let error = fixture.actor.publish_prepared_apply(prepared).unwrap_err();

    assert_eq!(
        error.kind(),
        super::ApplyPublicationErrorKind::ContainmentIdentity
    );
    assert_eq!(snapshot_tree(&fixture.roots[0]), source_before);
    assert_eq!(
        snapshot_tree(&fixture.root.join(".build/unica")),
        cache_before
    );
    fixture.cleanup();
}

pub fn apply_policy_same_inode_churn_during_late_final_gate_rolls_back_all_retained_state(
    policies: crate::policies::WorkspacePolicies,
) {
    let _policy_scope = policies_scope(policies);

    let _test_state = SupportPolicyTestStateGuard::new(reset_support_policy_actor_test_now);
    let fixture = actor_fixture("apply-policy-late-final-same-inode-churn", &["src"]);
    let target = fixture.roots[0].join("Module.bsl");
    let policy = fixture.root.join(".v8-project.json");
    std::fs::write(&target, b"original").unwrap();
    let admitted = br#"{"editingAllowedCheck":"off"}"#;
    let changed = br#"{"editingAllowedCheck":"bad"}"#;
    assert_eq!(admitted.len(), changed.len());
    std::fs::write(&policy, admitted).unwrap();
    let policy_identity =
        crate::platform::filesystem::file_identity(&std::fs::File::open(&policy).unwrap()).unwrap();
    let binding = fixture
        .actor
        .bind_provider_root("src", &fixture.roots[0])
        .unwrap();
    let admission = fixture
        .actor
        .admit_apply(
            &binding,
            None,
            false,
            ProviderDeadline::from_budget(Duration::from_secs(10)),
            &CancellationToken::new(),
        )
        .unwrap();
    let mut state = admission.staged_state().unwrap();
    state
        .replace("Module.bsl", b"original", b"published".to_vec())
        .unwrap();
    let prepared = admission.prepare(state).unwrap();
    let source_before = snapshot_tree(&fixture.roots[0]);
    let cache_before = snapshot_tree(&fixture.root.join(".build/unica"));
    let hook_policy = policy.clone();
    crate::compile_transaction::set_retained_apply_before_post_validation_hook(move || {
        crate::policy_evidence::set_support_policy_after_retained_read_before_acceptance_hook(
            move || std::fs::write(hook_policy, changed).unwrap(),
        );
    });

    let error = fixture.actor.publish_prepared_apply(prepared).unwrap_err();
    let current_policy_identity =
        crate::platform::filesystem::file_identity(&std::fs::File::open(&policy).unwrap()).unwrap();

    assert_eq!(
        error.kind(),
        super::ApplyPublicationErrorKind::ContainmentIdentity
    );
    assert_eq!(current_policy_identity, policy_identity);
    assert_eq!(snapshot_tree(&fixture.roots[0]), source_before);
    assert_eq!(
        snapshot_tree(&fixture.root.join(".build/unica")),
        cache_before
    );
    fixture.cleanup();
}

pub fn apply_policy_same_ancestor_can_govern_two_worktrees_without_authority_aliasing(
    policies: crate::policies::WorkspacePolicies,
) {
    let _policy_scope = policies_scope(policies);

    let container = temp_root("apply-policy-shared-ancestor");
    std::fs::create_dir_all(&container).unwrap();
    std::fs::write(
        container.join(".v8-project.json"),
        br#"{"editingAllowedCheck":"off"}"#,
    )
    .unwrap();
    let mut actors = Vec::new();
    for name in ["one", "two"] {
        let workspace = container.join("worktrees").join(name);
        let source = workspace.join("src");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::write(
                workspace.join("v8project.yaml"),
                "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
            )
            .unwrap();
        std::fs::write(
            source.join("Configuration.xml"),
            "<MetaDataObject><Configuration/></MetaDataObject>",
        )
        .unwrap();
        let context = context(&workspace);
        let identity = WorkspaceIdentity::new(
            &context,
            [source_input("main", source.as_path())],
            "test-provider",
        )
        .unwrap();
        actors.push((
            Arc::new(
                super::WorkspaceActor::<(), ()>::new(identity, context)
                    .map(bind_actor)
                    .unwrap(),
            ),
            source,
        ));
    }
    let first_binding = actors[0]
        .0
        .bind_provider_root("main", &actors[0].1)
        .unwrap();
    let first_admission = actors[0]
        .0
        .admit_apply(
            &first_binding,
            None,
            true,
            ProviderDeadline::from_budget(Duration::from_secs(5)),
            &CancellationToken::new(),
        )
        .unwrap();
    assert_eq!(
        first_admission.support_policy_mode(),
        SupportPolicyMode::Off
    );
    let first_state = first_admission.staged_state().unwrap();
    let first_prepared = first_admission.prepare(first_state).unwrap();
    let replay = actors[1].0.publish_prepared_apply(first_prepared);
    assert_eq!(
        replay.unwrap_err().kind(),
        super::ApplyPublicationErrorKind::Invariant
    );

    for (actor, source) in &actors {
        let binding = actor.bind_provider_root("main", source).unwrap();
        let admission = actor
            .admit_apply(
                &binding,
                None,
                true,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        assert_eq!(admission.support_policy_mode(), SupportPolicyMode::Off);
        let state = admission.staged_state().unwrap();
        let result = actor
            .publish_prepared_apply(admission.prepare(state).unwrap())
            .unwrap();
        assert_eq!(result.commit_count_for_test(), 0);
    }
    std::fs::remove_dir_all(container).unwrap();
}

pub fn apply_policy_deadline_and_cancellation_during_capture_are_write_free(
    policies: crate::policies::WorkspacePolicies,
) {
    let _policy_scope = policies_scope(policies);

    let _test_state = SupportPolicyTestStateGuard::new(reset_support_policy_actor_test_now);
    let mut rejected = Vec::new();
    for gate in ["cancelled", "deadline"] {
        let fixture = actor_fixture(&format!("apply-policy-capture-{gate}"), &["src"]);
        std::fs::write(
            fixture.root.join(".v8-project.json"),
            br#"{"editingAllowedCheck":"off"}"#,
        )
        .unwrap();
        let binding = fixture
            .actor
            .bind_provider_root("src", &fixture.roots[0])
            .unwrap();
        let cancellation = CancellationToken::new();
        if gate == "cancelled" {
            let cancel = cancellation.clone();
            set_support_policy_capture_hook(move || cancel.cancel());
        } else {
            set_support_policy_capture_hook(|| {
                std::thread::sleep(Duration::from_millis(80));
            });
        }
        let result = fixture.actor.admit_apply(
            &binding,
            None,
            false,
            ProviderDeadline::from_budget(if gate == "deadline" {
                Duration::from_millis(50)
            } else {
                Duration::from_secs(5)
            }),
            &cancellation,
        );
        rejected.push(result.is_err());
        assert!(!fixture.root.join(".build/unica").exists());
        fixture.cleanup();
    }
    assert_eq!(rejected, [true, true]);
}

pub fn apply_policy_capture_stops_after_first_retained_read_chunk_write_free(
    policies: crate::policies::WorkspacePolicies,
) {
    let _policy_scope = policies_scope(policies);

    let _test_state = SupportPolicyTestStateGuard::new(reset_support_policy_actor_test_now);
    let mut observed = Vec::new();
    for gate in ["cancelled", "deadline"] {
        let fixture = actor_fixture(&format!("apply-policy-chunked-capture-{gate}"), &["src"]);
        let policy = fixture.root.join(".v8-project.json");
        let prefix = br#"{"editingAllowedCheck":"off"}"#;
        let mut policy_bytes = vec![b' '; 3 * 64 * 1024];
        policy_bytes[..prefix.len()].copy_from_slice(prefix);
        std::fs::write(&policy, policy_bytes).unwrap();
        let target = fixture.roots[0].join("Module.bsl");
        std::fs::write(&target, b"original").unwrap();
        let binding = fixture
            .actor
            .bind_provider_root("src", &fixture.roots[0])
            .unwrap();
        let source_before = snapshot_tree(&fixture.roots[0]);
        let cache_before = snapshot_tree(&fixture.root.join(".build/unica"));
        let cancellation = CancellationToken::new();
        let started = Instant::now();
        set_support_policy_actor_test_now(started);
        let deadline = if gate == "deadline" {
            ProviderDeadline::with_clock(
                started + Duration::from_secs(1),
                support_policy_actor_test_now,
            )
        } else {
            ProviderDeadline::from_budget(Duration::from_secs(10))
        };
        let chunks = Arc::new(AtomicUsize::new(0));
        let observed_chunks = Arc::clone(&chunks);
        if gate == "cancelled" {
            let cancel = cancellation.clone();
            set_support_policy_read_chunk_hook_once(move |_| {
                observed_chunks.fetch_add(1, Ordering::SeqCst);
                cancel.cancel();
            });
        } else {
            set_support_policy_read_chunk_hook_once(move |_| {
                observed_chunks.fetch_add(1, Ordering::SeqCst);
                set_support_policy_actor_test_now(started + Duration::from_secs(2));
            });
        }

        let result = fixture
            .actor
            .admit_apply(&binding, None, false, deadline, &cancellation);
        observed.push((
            result.err().map(|error| error.to_string()),
            chunks.load(Ordering::SeqCst),
        ));

        assert_eq!(std::fs::read(&target).unwrap(), b"original", "{gate}");
        assert_eq!(snapshot_tree(&fixture.roots[0]), source_before, "{gate}");
        assert_eq!(
            snapshot_tree(&fixture.root.join(".build/unica")),
            cache_before,
            "{gate}"
        );
        fixture.cleanup();
    }

    assert_eq!(
        observed,
        [
            (Some("support-policy capture cancelled".to_string()), 1),
            (
                Some("support-policy capture deadline exceeded".to_string()),
                1,
            ),
        ]
    );
}

pub fn apply_policy_all_absent_capture_rejects_terminal_cancellation_and_deadline_write_free(
    policies: crate::policies::WorkspacePolicies,
) {
    let _policy_scope = policies_scope(policies);

    let _test_state = SupportPolicyTestStateGuard::new(reset_support_policy_actor_test_now);
    let mut observed = Vec::new();
    for gate in ["cancelled", "deadline"] {
        let container = temp_root(&format!("apply-policy-all-absent-capture-{gate}"));
        let root = (0..24).fold(container.clone(), |path, index| {
            path.join(format!("level-{index}"))
        });
        let source = root.join("src");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::write(
                root.join("v8project.yaml"),
                "format: DESIGNER\nsource-set:\n  - name: src\n    type: CONFIGURATION\n    path: src\n",
            )
            .unwrap();
        std::fs::write(
            source.join("Configuration.xml"),
            "<MetaDataObject><Configuration/></MetaDataObject>",
        )
        .unwrap();
        let context = context(&root);
        let identity = WorkspaceIdentity::new(
            &context,
            [source_input("src", source.as_path())],
            "test-provider",
        )
        .unwrap();
        let actor = super::WorkspaceActor::<(), ()>::new(identity, context)
            .map(bind_actor)
            .unwrap();
        let binding = actor.bind_provider_root("src", &source).unwrap();
        let before = snapshot_tree(&root);
        let cancellation = CancellationToken::new();
        let started = Instant::now();
        set_support_policy_actor_test_now(started);
        let deadline = if gate == "deadline" {
            ProviderDeadline::with_clock(
                started + Duration::from_secs(1),
                support_policy_actor_test_now,
            )
        } else {
            ProviderDeadline::from_budget(Duration::from_secs(5))
        };
        if gate == "cancelled" {
            let cancel = cancellation.clone();
            set_support_policy_capture_hook(move || cancel.cancel());
        } else {
            set_support_policy_capture_hook(move || {
                set_support_policy_actor_test_now(started + Duration::from_secs(2));
            });
        }

        let result = actor.admit_apply(&binding, None, false, deadline, &cancellation);

        observed.push(result.err().map(|error| error.to_string()));
        assert_eq!(snapshot_tree(&root), before, "{gate}");
        assert!(!root.join(".build/unica").exists(), "{gate}");
        std::fs::remove_dir_all(container).unwrap();
    }
    assert_eq!(
        observed,
        [
            Some("support-policy capture cancelled".to_string()),
            Some("support-policy capture deadline exceeded".to_string()),
        ]
    );
}

pub fn apply_policy_deadline_and_cancellation_during_final_validation_roll_back(
    policies: crate::policies::WorkspacePolicies,
) {
    let _policy_scope = policies_scope(policies);

    let _test_state = SupportPolicyTestStateGuard::new(reset_support_policy_actor_test_now);
    for gate in ["cancelled", "deadline"] {
        let fixture = actor_fixture(&format!("apply-policy-final-{gate}"), &["src"]);
        let target = fixture.roots[0].join("Module.bsl");
        std::fs::write(&target, b"original").unwrap();
        std::fs::write(
            fixture.root.join(".v8-project.json"),
            br#"{"editingAllowedCheck":"off"}"#,
        )
        .unwrap();
        let binding = fixture
            .actor
            .bind_provider_root("src", &fixture.roots[0])
            .unwrap();
        let cancellation = CancellationToken::new();
        let started = Instant::now();
        set_support_policy_actor_test_now(started);
        let admission = fixture
            .actor
            .admit_apply(
                &binding,
                None,
                false,
                if gate == "deadline" {
                    ProviderDeadline::with_clock(
                        started + Duration::from_secs(1),
                        support_policy_actor_test_now,
                    )
                } else {
                    ProviderDeadline::from_budget(Duration::from_secs(5))
                },
                &cancellation,
            )
            .unwrap();
        let mut state = admission.staged_state().unwrap();
        state
            .replace("Module.bsl", b"original", b"published".to_vec())
            .unwrap();
        let prepared = admission.prepare(state).unwrap();
        let source_before = snapshot_tree(&fixture.roots[0]);
        let cache_before = snapshot_tree(&fixture.root.join(".build/unica"));
        if gate == "cancelled" {
            let cancel = cancellation.clone();
            crate::compile_transaction::set_retained_apply_before_post_validation_hook(move || {
                set_support_policy_validation_hook(move || cancel.cancel());
            });
        } else {
            crate::compile_transaction::set_retained_apply_before_post_validation_hook(move || {
                set_support_policy_validation_hook(move || {
                    set_support_policy_actor_test_now(started + Duration::from_secs(2));
                });
            });
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
        assert_eq!(snapshot_tree(&fixture.roots[0]), source_before);
        assert_eq!(
            snapshot_tree(&fixture.root.join(".build/unica")),
            cache_before
        );
        fixture.cleanup();
    }
}

pub fn apply_policy_final_gate_stops_after_first_retained_read_chunk_and_rolls_back(
    policies: crate::policies::WorkspacePolicies,
) {
    let _policy_scope = policies_scope(policies);

    let _test_state = SupportPolicyTestStateGuard::new(reset_support_policy_actor_test_now);
    let mut observed = Vec::new();
    for gate in ["cancelled", "deadline"] {
        let fixture = actor_fixture(&format!("apply-policy-chunked-final-{gate}"), &["src"]);
        let target = fixture.roots[0].join("Module.bsl");
        std::fs::write(&target, b"original").unwrap();
        let policy = fixture.root.join(".v8-project.json");
        let prefix = br#"{"editingAllowedCheck":"off"}"#;
        let mut policy_bytes = vec![b' '; 3 * 64 * 1024];
        policy_bytes[..prefix.len()].copy_from_slice(prefix);
        std::fs::write(&policy, policy_bytes).unwrap();
        let binding = fixture
            .actor
            .bind_provider_root("src", &fixture.roots[0])
            .unwrap();
        let cancellation = CancellationToken::new();
        let started = Instant::now();
        set_support_policy_actor_test_now(started);
        let deadline = if gate == "deadline" {
            ProviderDeadline::with_clock(
                started + Duration::from_secs(1),
                support_policy_actor_test_now,
            )
        } else {
            ProviderDeadline::from_budget(Duration::from_secs(10))
        };
        let admission = fixture
            .actor
            .admit_apply(&binding, None, false, deadline, &cancellation)
            .unwrap();
        let mut state = admission.staged_state().unwrap();
        state
            .replace("Module.bsl", b"original", b"published".to_vec())
            .unwrap();
        let prepared = admission.prepare(state).unwrap();
        let source_before = snapshot_tree(&fixture.roots[0]);
        let cache_before = snapshot_tree(&fixture.root.join(".build/unica"));
        let chunks = Arc::new(AtomicUsize::new(0));
        let observed_chunks = Arc::clone(&chunks);
        if gate == "cancelled" {
            let cancel = cancellation.clone();
            crate::compile_transaction::set_retained_apply_before_post_validation_hook(move || {
                set_support_policy_read_chunk_hook_once(move |_| {
                    observed_chunks.fetch_add(1, Ordering::SeqCst);
                    cancel.cancel();
                });
            });
        } else {
            crate::compile_transaction::set_retained_apply_before_post_validation_hook(move || {
                set_support_policy_read_chunk_hook_once(move |_| {
                    observed_chunks.fetch_add(1, Ordering::SeqCst);
                    set_support_policy_actor_test_now(started + Duration::from_secs(2));
                });
            });
        }

        let result = fixture.actor.publish_prepared_apply(prepared);
        observed.push((
            result.as_ref().err().map(|error| error.kind()),
            chunks.load(Ordering::SeqCst),
        ));
        assert!(result.is_err(), "{gate} returned a terminal receipt");
        assert_eq!(snapshot_tree(&fixture.roots[0]), source_before, "{gate}");
        assert_eq!(
            snapshot_tree(&fixture.root.join(".build/unica")),
            cache_before,
            "{gate}"
        );
        fixture.cleanup();
    }

    assert_eq!(
        observed,
        [
            (Some(super::ApplyPublicationErrorKind::Cancelled), 1),
            (Some(super::ApplyPublicationErrorKind::Deadline), 1),
        ]
    );
}

pub fn apply_policy_warn_off_deny_database_and_malformed_match_v12(
    policies: crate::policies::WorkspacePolicies,
) {
    let _policy_scope = policies_scope(policies);

    let cases: &[(&str, &[u8], SupportPolicyMode)] = &[
            (
                "warn",
                br#"{"editingAllowedCheck":"warn"}"#,
                SupportPolicyMode::Warn,
            ),
            (
                "off",
                br#"{"editingAllowedCheck":"off"}"#,
                SupportPolicyMode::Off,
            ),
            (
                "deny",
                br#"{"editingAllowedCheck":"deny"}"#,
                SupportPolicyMode::Deny,
            ),
            ("missing-value", br#"{}"#, SupportPolicyMode::Deny),
            ("malformed", b"not json", SupportPolicyMode::Deny),
            (
                "unknown",
                br#"{"editingAllowedCheck":"WARN"}"#,
                SupportPolicyMode::Deny,
            ),
            (
                "bom",
                b"\xef\xbb\xbf{\"editingAllowedCheck\":\"off\"}",
                SupportPolicyMode::Off,
            ),
            (
                "database",
                br#"{"editingAllowedCheck":"off","databases":[{"configSrc":"src","editingAllowedCheck":"warn"}]}"#,
                SupportPolicyMode::Warn,
            ),
        ];
    for (name, bytes, expected) in cases {
        let fixture = actor_fixture(&format!("apply-policy-mode-{name}"), &["src"]);
        std::fs::write(fixture.root.join(".v8-project.json"), bytes).unwrap();
        let binding = fixture
            .actor
            .bind_provider_root("src", &fixture.roots[0])
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
        assert_eq!(admission.support_policy_mode(), *expected, "{name}");
        fixture.cleanup();
    }

    let missing = actor_fixture("apply-policy-mode-missing", &["src"]);
    let binding = missing
        .actor
        .bind_provider_root("src", &missing.roots[0])
        .unwrap();
    let admission = missing
        .actor
        .admit_apply(
            &binding,
            None,
            true,
            ProviderDeadline::from_budget(Duration::from_secs(5)),
            &CancellationToken::new(),
        )
        .unwrap();
    assert_eq!(admission.support_policy_mode(), SupportPolicyMode::Deny);
    missing.cleanup();

    let bounded = actor_fixture("apply-policy-mode-size-boundary", &["src"]);
    let binding = bounded
        .actor
        .bind_provider_root("src", &bounded.roots[0])
        .unwrap();
    for (size, expected) in [
        (32 * 1024 * 1024, SupportPolicyMode::Off),
        (32 * 1024 * 1024 + 1, SupportPolicyMode::Deny),
    ] {
        let mut bytes = br#"{"editingAllowedCheck":"off"}"#.to_vec();
        bytes.resize(size, b' ');
        std::fs::write(bounded.root.join(".v8-project.json"), bytes).unwrap();
        let admission = bounded
            .actor
            .admit_apply(
                &binding,
                None,
                true,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        assert_eq!(admission.support_policy_mode(), expected, "size={size}");
    }
    bounded.cleanup();

    let unreadable = actor_fixture("apply-policy-mode-unreadable", &["src"]);
    let policy = unreadable.root.join(".v8-project.json");
    std::fs::write(&policy, br#"{"editingAllowedCheck":"off"}"#).unwrap();
    if set_unix_mode_for_test(&policy, 0o000).unwrap() {
        let binding = unreadable
            .actor
            .bind_provider_root("src", &unreadable.roots[0])
            .unwrap();
        let admission = unreadable
            .actor
            .admit_apply(
                &binding,
                None,
                true,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        assert_eq!(admission.support_policy_mode(), SupportPolicyMode::Deny);
        drop(admission);
        assert!(set_unix_mode_for_test(&policy, 0o600).unwrap());
    } else {
        eprintln!("[SKIPPED FIXTURE] unreadable support-policy mode requires Unix permission bits");
    }
    unreadable.cleanup();
}

pub fn retained_apply_support_policy_evidence_does_not_add_a_third_writer_participant(
    policies: crate::policies::WorkspacePolicies,
) {
    let _policy_scope = policies_scope(policies);

    let fixture = actor_fixture("apply-policy-two-writers", &["src"]);
    let binding = fixture
        .actor
        .bind_provider_root("src", &fixture.roots[0])
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
    let state = admission.staged_state().unwrap();
    let prepared = admission.prepare(state).unwrap();
    assert_eq!(
        prepared.transaction.retained_role_root_counts_for_test(),
        (1, 1)
    );
    fixture.cleanup();
}
