use super::test_support::*;
use super::*;
use std::sync::mpsc;
use std::thread;

#[test]
fn replaced_named_root_outweighs_unavailable_root_in_any_iteration_order() {
    for unavailable_first in [false, true] {
        let unavailable = Err(std::io::Error::new(
            ErrorKind::PermissionDenied,
            "injected temporary root validation failure",
        ));
        let matches = if unavailable_first {
            vec![unavailable, Ok(false)]
        } else {
            vec![Ok(false), unavailable]
        };
        assert!(!aggregate_named_root_matches(matches).unwrap());
    }

    let unavailable = aggregate_named_root_matches([
        Ok(true),
        Err(std::io::Error::new(
            ErrorKind::PermissionDenied,
            "injected temporary root validation failure",
        )),
    ])
    .unwrap_err();
    assert_eq!(unavailable.kind(), ErrorKind::PermissionDenied);
}

#[test]
fn index_coordination_has_no_raw_crate_level_constructor_or_owner_escape() {
    let shared_work = include_str!("../../unica-work-control/src/shared_work.rs");
    let actor = include_str!("actor.rs");
    assert!(
        !shared_work.contains("pub(crate) struct IndexWorkKey"),
        "raw IndexWorkKey remains constructible across the crate"
    );
    assert!(
        !shared_work.contains("IndexWorkOwner"),
        "raw IndexWorkOwner remains joinable outside the actor"
    );
    let raw_key_accessor = ["pub(crate) fn index_", "work_key"].concat();
    let raw_owner_accessor = ["pub(crate) fn index_", "work(&self)"].concat();
    assert!(
        !actor.contains(&raw_key_accessor) && !actor.contains(&raw_owner_accessor),
        "WorkspaceActor still exposes a movable key/owner pair"
    );
    assert!(
        actor.contains("pub fn join_index_work"),
        "the actor-owned exact revision join seam is absent"
    );
}

#[test]
fn workspace_actor_allows_reads_to_overlap() {
    let fixture = actor_fixture("concurrent-reads", &["src"]);
    let binding = fixture
        .actor
        .bind_provider_root("src", &fixture.roots[0])
        .unwrap();
    let (entered_tx, entered_rx) = mpsc::channel();
    let (first_release_tx, first_release_rx) = mpsc::channel();
    let (second_release_tx, second_release_rx) = mpsc::channel();

    let first_actor = Arc::clone(&fixture.actor);
    let first_binding = binding.clone();
    let first_entered = entered_tx.clone();
    let first = thread::spawn(move || {
        first_actor.read(&first_binding, |_| {
            first_entered.send("first").unwrap();
            first_release_rx.recv().unwrap();
            Ok(())
        })
    });
    assert_eq!(
        entered_rx.recv_timeout(Duration::from_secs(2)).unwrap(),
        "first"
    );

    let second_actor = Arc::clone(&fixture.actor);
    let second = thread::spawn(move || {
        second_actor.read(&binding, |_| {
            entered_tx.send("second").unwrap();
            second_release_rx.recv().unwrap();
            Ok(())
        })
    });
    assert_eq!(
        entered_rx.recv_timeout(Duration::from_secs(2)).unwrap(),
        "second",
        "reads through one actor must not share the exclusive mutation lane"
    );
    first_release_tx.send(()).unwrap();
    second_release_tx.send(()).unwrap();
    first.join().unwrap().unwrap();
    second.join().unwrap().unwrap();
    fixture.cleanup();
}

#[test]
fn retained_binding_rejects_a_root_replaced_by_another_source_set_link() {
    use crate::platform::testing::{
        create_directory_link_fixture_for_test, FileLinkFixtureOutcome,
    };

    let fixture = actor_fixture("root-link-replacement", &["A", "B"]);
    let relative = Path::new("CommonModules/Same/Ext/Module.bsl");
    for (root, contents) in fixture.roots.iter().zip(["root A", "root B"]) {
        std::fs::create_dir_all(root.join(relative).parent().unwrap()).unwrap();
        std::fs::write(root.join(relative), contents).unwrap();
    }
    let binding = fixture
        .actor
        .bind_provider_root("A", &fixture.roots[0])
        .unwrap();
    let displaced = fixture.root.join("A-displaced");
    std::fs::rename(&fixture.roots[0], &displaced).unwrap();
    match create_directory_link_fixture_for_test(&fixture.roots[1], &fixture.roots[0]).unwrap() {
        FileLinkFixtureOutcome::Created => {}
        FileLinkFixtureOutcome::Unsupported
        | FileLinkFixtureOutcome::WindowsPrivilegeUnavailable => {
            unica_process::test_support::skip_without_host_capability(
                unica_process::test_support::HostCapability::Symlink,
                "creating a link fixture needs the symlink privilege",
            );
            let _ = std::fs::rename(&displaced, &fixture.roots[0]);
            fixture.cleanup();
            return;
        }
    }

    let result = fixture.actor.read_relative_file(&binding, relative, 1024);

    assert!(
        result.is_err(),
        "retained binding followed replacement: {result:?}"
    );
    fixture.cleanup();
}

#[test]
fn retained_binding_rejects_a_same_path_directory_replacement() {
    let fixture = actor_fixture("root-directory-replacement", &["A"]);
    let relative = Path::new("Module.bsl");
    std::fs::write(fixture.roots[0].join(relative), "original").unwrap();
    let binding = fixture
        .actor
        .bind_provider_root("A", &fixture.roots[0])
        .unwrap();
    std::fs::rename(&fixture.roots[0], fixture.root.join("A-displaced")).unwrap();
    std::fs::create_dir_all(&fixture.roots[0]).unwrap();
    std::fs::write(fixture.roots[0].join(relative), "replacement").unwrap();

    let result = fixture.actor.read_relative_file(&binding, relative, 1024);

    assert!(
        result.is_err(),
        "retained binding accepted replacement: {result:?}"
    );
    fixture.cleanup();
}

#[test]
fn descriptor_relative_read_never_follows_a_nested_directory_link() {
    use crate::platform::testing::{
        create_directory_link_fixture_for_test, FileLinkFixtureOutcome,
    };

    let fixture = actor_fixture("nested-read-link", &["A", "B"]);
    std::fs::write(fixture.roots[1].join("Secret.bsl"), "root B").unwrap();
    let nested = fixture.roots[0].join("Nested");
    match create_directory_link_fixture_for_test(&fixture.roots[1], &nested).unwrap() {
        FileLinkFixtureOutcome::Created => {}
        FileLinkFixtureOutcome::Unsupported
        | FileLinkFixtureOutcome::WindowsPrivilegeUnavailable => {
            unica_process::test_support::skip_without_host_capability(
                unica_process::test_support::HostCapability::Symlink,
                "creating a link fixture needs the symlink privilege",
            );
            fixture.cleanup();
            return;
        }
    }
    let binding = fixture
        .actor
        .bind_provider_root("A", &fixture.roots[0])
        .unwrap();

    let result = fixture
        .actor
        .read_relative_file(&binding, Path::new("Nested/Secret.bsl"), 1024);

    assert!(result.is_err(), "nested link was followed: {result:?}");
    fixture.cleanup();
}

#[test]
fn path_provider_discards_output_when_root_changes_mid_operation() {
    let fixture = actor_fixture("root-mid-operation-swap", &["A"]);
    let relative = Path::new("Module.bsl");
    std::fs::write(fixture.roots[0].join(relative), "before swap").unwrap();
    let binding = fixture
        .actor
        .bind_provider_root("A", &fixture.roots[0])
        .unwrap();
    let displaced = fixture.root.join("A-displaced");
    let replacement = fixture.roots[0].clone();

    let result = fixture.actor.read(&binding, |root| {
        let output =
            std::fs::read_to_string(root.join(relative)).map_err(|error| error.to_string())?;
        std::fs::rename(root, &displaced).map_err(|error| error.to_string())?;
        std::fs::create_dir_all(&replacement).map_err(|error| error.to_string())?;
        Ok(output)
    });

    assert!(
        result.is_err(),
        "provider published output after swap: {result:?}"
    );
    fixture.cleanup();
}

#[test]
pub(crate) fn duplicate_physical_root_names_are_rejected_as_ambiguous() {
    let root = temp_root("duplicate-physical-root");
    let source = root.join("src");
    std::fs::create_dir_all(&source).unwrap();
    let context = context(&root);

    let result = WorkspaceIdentity::new(
        &context,
        [
            source_input("main", &source),
            source_input("alias", &source),
        ],
        "profile",
    );

    assert!(result.is_err(), "duplicate physical root was accepted");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
pub(crate) fn duplicate_source_set_names_with_distinct_roots_are_rejected() {
    let root = temp_root("duplicate-source-set-name");
    let first_source = root.join("first");
    let second_source = root.join("second");
    std::fs::create_dir_all(&first_source).unwrap();
    std::fs::create_dir_all(&second_source).unwrap();
    let context = context(&root);

    let result = WorkspaceIdentity::new(
        &context,
        [
            source_input("main", &first_source),
            source_input("main", &second_source),
        ],
        "profile",
    );

    assert!(
        result.is_err(),
        "duplicate source-set name with distinct retained roots was accepted"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn workspace_actor_exposes_no_raw_generic_runtime_payload() {
    let immutable_return = ["-> ", "&R"].concat();
    let mutable_return = ["-> ", "&mut R"].concat();
    let source = include_str!("actor.rs");

    assert!(
        !source.lines().any(|line| line.contains(&immutable_return)),
        "workspace actor exposes a raw immutable runtime payload"
    );
    assert!(
        !source.lines().any(|line| line.contains(&mutable_return)),
        "workspace actor exposes a raw mutable runtime payload"
    );
}

#[test]
fn actor_state_scope_digest_is_fallible_and_bounded() {
    fn digest(identity: &WorkspaceIdentity) -> Result<String, String> {
        identity.state_scope_digest()
    }

    let identity = WorkspaceIdentity {
        workspace_root: PathBuf::from("/workspace"),
        source_sets: vec![super::WorkspaceSourceSetIdentity {
            name: "main".to_string(),
            root: PathBuf::from("/workspace/source"),
            kind: SourceSetKind::Configuration,
            source_format: SourceFormat::PlatformXml,
            source_profile: SourceProfile::platform_xml_8_3_27_format_2_20(),
        }],
        provider_profile: "profile".to_string(),
    };
    let digest = digest(&identity).unwrap();

    assert_eq!(digest.len(), 64);
    assert!(digest.bytes().all(|byte| byte.is_ascii_hexdigit()));
}

#[test]
fn actor_state_scope_distinguishes_native_non_unicode_paths() {
    // Unix and Windows both have non-Unicode native paths.
    let (first, second) = crate::platform::filesystem::distinct_non_unicode_paths_for_test()
        .expect("non-Unicode paths exist on Unix and Windows");
    let identity = |root| WorkspaceIdentity {
        workspace_root: PathBuf::from("/workspace"),
        source_sets: vec![super::WorkspaceSourceSetIdentity {
            name: "main".to_string(),
            root,
            kind: SourceSetKind::Configuration,
            source_format: SourceFormat::PlatformXml,
            source_profile: SourceProfile::platform_xml_8_3_27_format_2_20(),
        }],
        provider_profile: "profile".to_string(),
    };

    assert_ne!(
        identity(first).state_scope_digest().unwrap(),
        identity(second).state_scope_digest().unwrap(),
        "distinct native non-Unicode roots collapsed to one actor scope"
    );
}

#[test]
pub(crate) fn same_name_root_changed_kind_rotates_actor_and_state_scope() {
    let root = temp_root("typed-kind-identity");
    let source = root.join("src");
    std::fs::create_dir_all(&source).unwrap();
    let context = context(&root);
    let input = |kind| {
        WorkspaceSourceSetInput::new(
            "main",
            &source,
            kind,
            SourceFormat::PlatformXml,
            SourceProfile::platform_xml_8_3_27_format_2_20(),
        )
    };
    let configuration_identity =
        WorkspaceIdentity::new(&context, [input(SourceSetKind::Configuration)], "provider")
            .unwrap();
    let extension_identity =
        WorkspaceIdentity::new(&context, [input(SourceSetKind::Extension)], "provider").unwrap();
    let registry = registry_for_test();
    let configuration = registry
        .get_or_create(&context, [input(SourceSetKind::Configuration)], "provider")
        .unwrap();
    let extension = registry
        .get_or_create(&context, [input(SourceSetKind::Extension)], "provider")
        .unwrap();

    let actor_was_reused = Arc::ptr_eq(&configuration, &extension);
    let scope_was_reused = configuration_identity.state_scope_digest().unwrap()
        == extension_identity.state_scope_digest().unwrap();
    assert!(
        !actor_was_reused && !scope_was_reused,
        "changed source kind reused actor={actor_was_reused}, state_scope={scope_was_reused}"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
pub(crate) fn same_name_root_changed_format_or_platform_profile_rotates_actor() {
    let root = temp_root("typed-format-profile-identity");
    let source = root.join("src");
    std::fs::create_dir_all(&source).unwrap();
    let context = context(&root);
    let input = |source_format, source_profile| {
        WorkspaceSourceSetInput::new(
            "main",
            &source,
            SourceSetKind::Configuration,
            source_format,
            source_profile,
        )
    };
    let cases = [
        (
            "format",
            SourceFormat::Edt,
            SourceProfile::platform_xml_8_3_27_format_2_20(),
        ),
        (
            "platform",
            SourceFormat::PlatformXml,
            SourceProfile::TestPlatform8_3_28Format2_20,
        ),
        (
            "serialization",
            SourceFormat::PlatformXml,
            SourceProfile::TestPlatform8_3_27Format2_21,
        ),
    ];
    let baseline_identity = WorkspaceIdentity::new(
        &context,
        [input(
            SourceFormat::PlatformXml,
            SourceProfile::platform_xml_8_3_27_format_2_20(),
        )],
        "provider",
    )
    .unwrap();
    let registry = registry_for_test();
    let baseline = registry
        .get_or_create(
            &context,
            [input(
                SourceFormat::PlatformXml,
                SourceProfile::platform_xml_8_3_27_format_2_20(),
            )],
            "provider",
        )
        .unwrap();
    let mut aliases = Vec::new();
    for (label, source_format, source_profile) in cases {
        let changed_identity =
            WorkspaceIdentity::new(&context, [input(source_format, source_profile)], "provider")
                .unwrap();
        let changed = registry
            .get_or_create(&context, [input(source_format, source_profile)], "provider")
            .unwrap();
        if Arc::ptr_eq(&baseline, &changed)
            || baseline_identity.state_scope_digest().unwrap()
                == changed_identity.state_scope_digest().unwrap()
        {
            aliases.push(label);
        }
    }

    assert!(
        aliases.is_empty(),
        "planner-significant typed source fields aliased baseline identity: {aliases:?}"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn warm_registry_reuses_the_same_actor_across_sequential_admissions() {
    super::retention_contract_tests::warm_registry_reuses_the_same_actor_across_sequential_admissions();
}

#[test]
fn warm_actor_expires_after_the_idle_ttl_and_is_rebuilt() {
    super::retention_contract_tests::warm_actor_expires_after_the_idle_ttl_and_is_rebuilt();
}

#[test]
fn idle_listener_retains_unknown_registry_ownership() {
    for poison_warm in [false, true] {
        let registry = registry_for_test();
        assert!(!registry.has_retained_work());
        let poisoned = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            if poison_warm {
                let _guard = registry.warm.lock().unwrap();
                panic!("poison warm ownership for idle listener test");
            } else {
                let _guard = registry.actors.lock().unwrap();
                panic!("poison weak ownership for idle listener test");
            }
        }));
        assert!(poisoned.is_err());
        assert!(
            registry.has_retained_work(),
            "unknown ownership must not be treated as idle"
        );
    }
}

#[test]
fn idle_cleanup_retains_unknown_actor_state_without_rejecting_other_profiles() {
    for poison_plans in [true, false] {
        let root = temp_root("retention-poison");
        std::fs::create_dir_all(root.join("src")).unwrap();
        let registry = registry_for_test();
        let started = Instant::now();
        let actor = super::with_actor_retention_time_for_test(started, || {
            registry.get_or_create(
                &context(&root),
                [source_input("main", root.join("src"))],
                "p",
            )
        })
        .unwrap();
        let observed = Arc::downgrade(&actor);
        if poison_plans {
            let poisoned = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let _plans = actor.saved_apply_plans.lock().unwrap();
                panic!("poison saved plans for retention test");
            }));
            assert!(poisoned.is_err());
        } else {
            actor.index_work.poison_registry_for_test();
        }
        drop(actor);
        assert!(
            registry.has_retained_work(),
            "unknown actor state must hold the listener"
        );
        super::with_actor_retention_time_for_test(started + Duration::from_secs(601), || {
            registry.evict_idle_warm_actors().unwrap();
        });
        registry.release_warm_actors().unwrap();
        assert!(
            observed.upgrade().is_some(),
            "unknown promised state was treated as empty"
        );
        let other = root.join("other");
        std::fs::create_dir_all(other.join("src")).unwrap();
        let admitted = registry
            .get_or_create(
                &context(&other),
                [source_input("main", other.join("src"))],
                "p",
            )
            .expect("an unrelated profile must not be refused by a poisoned cleanup query");
        assert_ne!(
            admitted.identity().workspace_root(),
            observed.upgrade().unwrap().identity().workspace_root()
        );
        let _ = std::fs::remove_dir_all(root);
    }
}

#[test]
fn warm_actor_whose_named_root_was_replaced_is_rebuilt() {
    super::retention_contract_tests::warm_actor_whose_named_root_was_replaced_is_rebuilt();
}

#[test]
fn warm_actors_do_not_block_a_distinct_identity() {
    super::retention_contract_tests::warm_actors_do_not_block_a_distinct_identity();
}

#[test]
pub(crate) fn workspace_actor_registry_keys_exact_identity_and_separates_worktrees_and_source_roots(
) {
    let root = temp_root("identity");
    let shared_git = root.join("repository/.git");
    let worktree_a = root.join("worktrees/a");
    let worktree_b = root.join("worktrees/b");
    std::fs::create_dir_all(&shared_git).unwrap();
    for worktree in [&worktree_a, &worktree_b] {
        std::fs::create_dir_all(worktree.join("src-a")).unwrap();
        std::fs::create_dir_all(worktree.join("src-b")).unwrap();
        std::fs::write(worktree.join("v8project.yaml"), "format: DESIGNER\n").unwrap();
        std::fs::write(
            worktree.join(".git"),
            format!("gitdir: {}\n", shared_git.display()),
        )
        .unwrap();
    }
    let registry = registry_for_test();
    let context_a = context(&worktree_a);
    let context_b = context(&worktree_b);
    let a = registry
        .get_or_create(
            &context_a,
            [
                source_input("main", worktree_a.join("src-a")),
                source_input("extension", worktree_a.join("src-b")),
            ],
            "bsl-ls:program",
        )
        .unwrap();
    let same_a = registry
        .get_or_create(
            &context_a,
            [
                source_input("extension", worktree_a.join("src-b")),
                source_input("main", worktree_a.join("src-a")),
            ],
            "bsl-ls:program",
        )
        .unwrap();
    let other_worktree = registry
        .get_or_create(
            &context_b,
            [
                source_input("main", worktree_b.join("src-a")),
                source_input("extension", worktree_b.join("src-b")),
            ],
            "bsl-ls:program",
        )
        .unwrap();
    let other_roots = registry
        .get_or_create(
            &context_a,
            [source_input("main", worktree_a.join("src-a"))],
            "bsl-ls:program",
        )
        .unwrap();
    let other_profile = registry
        .get_or_create(
            &context_a,
            [
                source_input("main", worktree_a.join("src-a")),
                source_input("extension", worktree_a.join("src-b")),
            ],
            "bsl-ls:program-and-service",
        )
        .unwrap();
    let remapped_names = registry
        .get_or_create(
            &context_a,
            [
                source_input("main", worktree_a.join("src-b")),
                source_input("extension", worktree_a.join("src-a")),
            ],
            "bsl-ls:program",
        )
        .unwrap();

    assert!(Arc::ptr_eq(&a, &same_a));
    assert!(!Arc::ptr_eq(&a, &other_worktree));
    assert!(!Arc::ptr_eq(&a, &other_roots));
    assert!(!Arc::ptr_eq(&a, &other_profile));
    assert!(!Arc::ptr_eq(&a, &remapped_names));
    assert_eq!(registry.len().unwrap(), 5);
    assert_ne!(
        a.identity().workspace_root(),
        other_worktree.identity().workspace_root()
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn two_frontends_reuse_the_actor_for_one_canonical_worktree() {
    let root = temp_root("frontend-reuse");
    let source = root.join("src");
    let nested = root.join("frontend/cwd");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::create_dir_all(&nested).unwrap();
    let mut first_context = context(&root);
    first_context.cwd = root.clone();
    let mut second_context = context(&root.join("frontend/.."));
    second_context.cwd = nested;
    let registry = registry_for_test();

    let first = registry
        .get_or_create(
            &first_context,
            [source_input("main", &source)],
            "legacy-bsl-rlm",
        )
        .unwrap();
    let second = registry
        .get_or_create(
            &second_context,
            [source_input("main", root.join("frontend/../src"))],
            "legacy-bsl-rlm",
        )
        .unwrap();

    assert!(Arc::ptr_eq(&first, &second));
    assert_eq!(registry.len().unwrap(), 1);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn daemon_actor_registry_prunes_dead_entries_and_bounds_sequential_roots() {
    let parent = temp_root("bounded-sequential");
    // The weak map is the admission authority. Idle cleanup releases
    // empty actors, and the next admission prunes their dead entries.
    let registry = idle_registry_for_test(Duration::ZERO);

    for index in 0..12 {
        let root = parent.join(format!("workspace-{index}"));
        let source = root.join("src");
        std::fs::create_dir_all(&source).unwrap();
        let actor = registry
            .get_or_create(
                &context(&root),
                [source_input("main", &source)],
                "canonical-v0.13",
            )
            .unwrap();
        assert_eq!(registry.live_len_for_test().unwrap(), 1);
        drop(actor);
        registry.evict_idle_warm_actors().unwrap();
    }

    registry.prune_dead_for_test().unwrap();
    assert_eq!(registry.entry_len_for_test().unwrap(), 0);
    let _ = std::fs::remove_dir_all(parent);
}

#[test]
fn daemon_actor_registry_admits_live_identities_and_releases_only_unowned_actors() {
    let parent = temp_root("live-admissions");
    let registry = registry_for_test();
    let mut actors = Vec::new();
    let mut observed = Vec::new();
    for index in 0..3 {
        let root = parent.join(format!("workspace-{index}"));
        let source = root.join("src");
        std::fs::create_dir_all(&source).unwrap();
        let actor = registry
            .get_or_create(
                &context(&root),
                [source_input("main", &source)],
                "canonical-v0.13",
            )
            .unwrap();
        observed.push(Arc::downgrade(&actor));
        actors.push(actor);
    }
    registry.release_warm_actors().unwrap();
    assert_eq!(registry.live_len_for_test().unwrap(), 3);
    drop(actors.pop());
    registry.release_warm_actors().unwrap();
    assert!(observed[2].upgrade().is_none());
    assert!(observed[..2].iter().all(|actor| actor.upgrade().is_some()));
    assert_eq!(registry.live_len_for_test().unwrap(), 2);
    drop(actors);
    registry.release_warm_actors().unwrap();
    registry.prune_dead_for_test().unwrap();
    assert!(observed.iter().all(|actor| actor.upgrade().is_none()));
    assert_eq!(registry.entry_len_for_test().unwrap(), 0);
    let _ = std::fs::remove_dir_all(parent);
}

#[test]
fn active_alias_reuses_actor_and_dropped_actor_recreates_a_new_instance() {
    let root = temp_root("weak-alias-recreate");
    let source = root.join("src");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::create_dir_all(root.join("nested")).unwrap();
    let registry = idle_registry_for_test(Duration::ZERO);
    let first = registry
        .get_or_create(
            &context(&root),
            [source_input("main", &source)],
            "canonical-v0.13",
        )
        .unwrap();
    let stale_binding = first.bind_provider_root("main", &source).unwrap();
    let alias = registry
        .get_or_create(
            &context(&root.join("nested/..")),
            [source_input("main", root.join("nested/../src"))],
            "canonical-v0.13",
        )
        .unwrap();
    assert!(Arc::ptr_eq(&first, &alias));

    drop(alias);
    drop(first);
    // Once the warm TTL has elapsed the released actor is gone and the
    // next admission builds a fresh instance with its own capability id.
    registry.evict_idle_warm_actors().unwrap();
    let replacement = registry
        .get_or_create(
            &context(&root),
            [source_input("main", &source)],
            "canonical-v0.13",
        )
        .unwrap();
    assert!(replacement.validate_binding(&stale_binding).is_err());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn multiroot_provider_keeps_identical_relative_paths_bound_to_the_requesting_root() {
    let fixture = actor_fixture("multiroot", &["A", "B"]);
    let relative = Path::new("CommonModules/Same/Ext/Module.bsl");
    for (root, contents) in fixture.roots.iter().zip(["root A", "root B"]) {
        std::fs::create_dir_all(root.join(relative).parent().unwrap()).unwrap();
        std::fs::write(root.join(relative), contents).unwrap();
    }
    let binding_a = fixture
        .actor
        .bind_provider_root("A", &fixture.roots[0])
        .unwrap();
    let binding_b = fixture
        .actor
        .bind_provider_root("B", &fixture.roots[1])
        .unwrap();
    assert!(fixture
        .actor
        .bind_provider_root("A", &fixture.roots[1])
        .is_err());

    let from_a = String::from_utf8(
        fixture
            .actor
            .read_relative_file(&binding_a, relative, 1024)
            .unwrap(),
    )
    .unwrap();
    let from_b = String::from_utf8(
        fixture
            .actor
            .read_relative_file(&binding_b, relative, 1024)
            .unwrap(),
    )
    .unwrap();

    assert_eq!(from_a, "root A");
    assert_eq!(from_b, "root B");
    fixture.cleanup();
}

#[test]
pub(crate) fn active_platform_actor_cannot_select_the_legacy_state_scope() {
    let fixture = actor_fixture("active-actor-legacy-bypass", &["src"]);
    let source = &fixture.roots[0];
    let identity = WorkspaceIdentity::new(
        &context(&fixture.root),
        [source_input("src", source)],
        "test-provider",
    )
    .unwrap();

    let bypass =
        super::WorkspaceActor::<(), ()>::with_legacy_runtime(identity, context(&fixture.root), ());

    assert!(
        bypass.is_err(),
        "active Platform actor selected the LegacyPhysical state scope"
    );
    fixture.cleanup();
}

#[test]
pub(crate) fn apply_publication_result_exposes_the_effect_receipt_accessor() {
    let _total_effects_accessor: for<'a> fn(
        &'a super::ApplyPublicationResult,
    ) -> &'a ApplyEffectReceipt = super::ApplyPublicationResult::effects;
}
