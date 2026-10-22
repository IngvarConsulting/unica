use std::path::Path;
use std::time::Duration;
use unica_work_control::cancellation::CancellationToken;
use unica_work_control::provider_deadline::ProviderDeadline;
use unica_workspace::actor::test_support::{actor_fixture_with_policies, snapshot_tree};
#[test]
fn typed_metadata_members_roll_back_source_neighbors_and_cache_on_publication_failure() {
    use crate::domain::apply::ApplyRequest;
    use crate::infrastructure::native_operations::apply_families::plan_hidden_v13_apply;
    use unica_workspace::compile_transaction::{
        set_retained_apply_failpoint, take_retained_apply_observed_events, RetainedApplyFailpoint,
        RetainedApplyObservedEvent,
    };

    for (family, owner, item) in [
        (
            "tabularSection",
            "Catalog.Items",
            serde_json::json!({"name":"NewSection"}),
        ),
        (
            "dimension",
            "InformationRegister.CorpusRegister",
            serde_json::json!({"name":"NewDimension","type":{"variants":[{"kind":"string","length":20,"allowedLength":"variable"}]}}),
        ),
        (
            "resource",
            "InformationRegister.CorpusRegister",
            serde_json::json!({"name":"NewResource","type":{"variants":[{"kind":"number","digits":10,"fraction":2,"sign":"any"}]}}),
        ),
        (
            "enumValue",
            "Enum.LanguageAware",
            serde_json::json!({"name":"NewValue"}),
        ),
        (
            "column",
            "DocumentJournal.ДокументыОрганизации",
            serde_json::json!({"name":"NewColumn"}),
        ),
    ] {
        let fixture = actor_fixture_with_policies(
            &format!("typed-member-rollback-{family}"),
            &["src"],
            crate::infrastructure::source_workspace::policies(),
        );
        let source = &fixture.roots[0];
        let corpus = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/acceptance/workspace-metadata-members/src");
        for (relative, bytes) in snapshot_tree(&corpus) {
            let target = source.join(relative);
            match bytes {
                Some(bytes) => std::fs::write(target, bytes).unwrap(),
                None => std::fs::create_dir_all(target).unwrap(),
            }
        }
        let cache = fixture.root.join(".build/unica");
        std::fs::create_dir_all(&cache).unwrap();
        std::fs::write(cache.join("witness.bin"), b"existing cache bytes").unwrap();
        let binding = fixture.actor.bind_provider_root("src", source).unwrap();
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
        let input = serde_json::json!({
            "at": format!("src:{owner}"),
            "ops": [{"op":format!("{family}.add"),"args":{"items":[item]}}],
            "dryRun": true,
        });
        let request = ApplyRequest::parse(input.as_object().unwrap(), &["src"]).unwrap();
        let before_source = snapshot_tree(source);
        let before_cache = snapshot_tree(&cache);
        let (staged, effects) = plan_hidden_v13_apply(&request, &binding, &admission).unwrap();
        assert_eq!(
            staged.planned_changes().len(),
            1,
            "{family}: inline owner only"
        );
        let prepared = admission.prepare_with_effects(staged, effects).unwrap();
        assert_eq!(snapshot_tree(source), before_source);
        assert_eq!(snapshot_tree(&cache), before_cache);
        let _ = take_retained_apply_observed_events();
        set_retained_apply_failpoint(RetainedApplyFailpoint::EagerMetadata(1));

        let error = fixture.actor.publish_prepared_apply(prepared).unwrap_err();

        assert_eq!(
            error.kind(),
            unica_workspace::actor::ApplyPublicationErrorKind::ProviderPostvalidation,
            "{family}: {error}"
        );
        let observed = take_retained_apply_observed_events();
        assert!(
            observed
                .iter()
                .any(|event| matches!(event, RetainedApplyObservedEvent::Source(_))),
            "{family}: a real source postimage must precede cache failure: {observed:?}"
        );
        assert!(
            observed
                .iter()
                .any(|event| matches!(event, RetainedApplyObservedEvent::Rollback(_))),
            "{family}: {observed:?}"
        );
        assert_eq!(
            snapshot_tree(source),
            before_source,
            "{family} source and neighbors"
        );
        assert_eq!(snapshot_tree(&cache), before_cache, "{family} cache");
        fixture.cleanup();
    }
}

#[test]
fn command_module_lifecycle_failures_roll_back_owner_module_neighbors_and_cache() {
    use crate::domain::apply::ApplyRequest;
    use crate::infrastructure::native_operations::apply_families::plan_hidden_v13_apply;
    use unica_workspace::compile_transaction::{
        set_retained_apply_before_post_validation_hook, set_retained_apply_failpoint,
        take_retained_apply_observed_events, RetainedApplyFailpoint, RetainedApplyObservedEvent,
    };

    for operation in ["command.set", "command.remove"] {
        for phase in ["second-source", "cache", "postimages", "cancelled"] {
            let fixture = actor_fixture_with_policies(
                &format!("command-module-rollback-{operation}-{phase}"),
                &["src"],
                crate::infrastructure::source_workspace::policies(),
            );
            let source = &fixture.roots[0];
            let corpus = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../tests/fixtures/acceptance/workspace-metadata-commands/src");
            for relative in [
                "Configuration.xml",
                "Catalogs/Items.xml",
                "Catalogs/Items/Commands/Refresh/Ext/CommandModule.bsl",
                "Catalogs/Items/Commands/Keep/Ext/CommandModule.bsl",
            ] {
                let target = source.join(relative);
                std::fs::create_dir_all(target.parent().unwrap()).unwrap();
                std::fs::copy(corpus.join(relative), target).unwrap();
            }
            std::fs::write(
                source.join("Catalogs/Items/Commands/Refresh/Ext/untouched.bin"),
                b"opaque neighboring bytes\0\xff",
            )
            .unwrap();
            let cache = fixture.root.join(".build/unica");
            std::fs::create_dir_all(&cache).unwrap();
            std::fs::write(cache.join("witness.bin"), b"existing cache bytes").unwrap();
            let binding = fixture.actor.bind_provider_root("src", source).unwrap();
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
            let input = serde_json::json!({
                "at": "src:Catalog.Items.Command.Refresh",
                "ops": [{
                    "op": operation,
                    "args": if operation == "command.set" {
                        serde_json::json!({"values": {"newName": "Renamed"}})
                    } else {
                        serde_json::json!({})
                    },
                }],
                "dryRun": true,
            });
            let request = ApplyRequest::parse(input.as_object().unwrap(), &["src"]).unwrap();
            let before_source = snapshot_tree(source);
            let before_cache = snapshot_tree(&cache);
            let (staged, effects) = plan_hidden_v13_apply(&request, &binding, &admission).unwrap();
            assert_eq!(
                staged.planned_changes().len(),
                if operation == "command.set" { 3 } else { 2 },
                "the real planner must include the module lifecycle"
            );
            let prepared = admission.prepare_with_effects(staged, effects).unwrap();
            assert_eq!(snapshot_tree(source), before_source);
            assert_eq!(snapshot_tree(&cache), before_cache);
            let _ = take_retained_apply_observed_events();
            match phase {
                "second-source" => {
                    set_retained_apply_failpoint(RetainedApplyFailpoint::Source(2));
                }
                "cache" => {
                    set_retained_apply_failpoint(RetainedApplyFailpoint::EagerMetadata(1));
                }
                "postimages" => {
                    set_retained_apply_failpoint(RetainedApplyFailpoint::AfterAllPostimages);
                }
                "cancelled" => {
                    let cancel = cancellation.clone();
                    set_retained_apply_before_post_validation_hook(move || cancel.cancel());
                }
                _ => unreachable!(),
            }

            let error = fixture.actor.publish_prepared_apply(prepared).unwrap_err();

            assert_eq!(
                error.kind(),
                if phase == "cancelled" {
                    unica_workspace::actor::ApplyPublicationErrorKind::Cancelled
                } else {
                    unica_workspace::actor::ApplyPublicationErrorKind::ProviderPostvalidation
                },
                "{operation}/{phase}: {error}"
            );
            let observed = take_retained_apply_observed_events();
            assert!(
                observed.iter().any(|event| matches!(
                    event,
                    RetainedApplyObservedEvent::Source(path)
                        if path.ends_with("CommandModule.bsl")
                )),
                "{operation}/{phase} must publish a module before rollback: {observed:?}"
            );
            assert_eq!(snapshot_tree(source), before_source, "{operation}/{phase}");
            assert_eq!(snapshot_tree(&cache), before_cache, "{operation}/{phase}");
            fixture.cleanup();
        }
    }
}

#[test]
fn command_module_lifecycle_late_case_rename_rolls_back_owned_files_and_cache() {
    use crate::domain::apply::ApplyRequest;
    use crate::infrastructure::native_operations::apply_families::plan_hidden_v13_apply;
    use unica_workspace::compile_transaction::{
        set_retained_apply_before_post_validation_hook, take_retained_apply_observed_events,
        RetainedApplyObservedEvent,
    };

    if !unica_workspace::test_support::can_swap_named_child_behind_retained_handle_for_test() {
        unica_process::test_support::skip_without_host_capability(
            unica_process::test_support::HostCapability::RenameUnderOpenHandle,
            "the platform keeps an open handle from being renamed or replaced",
        );
        return;
    }

    for phase in ["existing-ancestor", "created-ancestor", "created-leaf"] {
        let fixture = actor_fixture_with_policies(
            &format!("command-spelling-late-{phase}"),
            &["src"],
            crate::infrastructure::source_workspace::policies(),
        );
        let source = &fixture.roots[0];
        let corpus = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/acceptance/workspace-metadata-commands/src");
        for relative in [
            "Configuration.xml",
            "Catalogs/Items.xml",
            "Catalogs/Items/Commands/Refresh/Ext/CommandModule.bsl",
            "Catalogs/Items/Commands/Keep/Ext/CommandModule.bsl",
        ] {
            let target = source.join(relative);
            std::fs::create_dir_all(target.parent().unwrap()).unwrap();
            std::fs::copy(corpus.join(relative), target).unwrap();
        }
        if !source.join("Catalogs/Items/Commands/refresh").exists() {
            eprintln!("[SKIPPED FIXTURE] source filesystem has no case aliases for the late spelling race");
            fixture.cleanup();
            continue;
        }
        let destination = source.join("Catalogs/Items/Commands/Renamed/Ext");
        if phase == "existing-ancestor" {
            std::fs::create_dir_all(&destination).unwrap();
            std::fs::write(destination.join("foreign.bin"), b"foreign\0\xff").unwrap();
        }
        let cache = fixture.root.join(".build/unica");
        std::fs::create_dir_all(&cache).unwrap();
        std::fs::write(cache.join("witness.bin"), b"existing cache bytes").unwrap();
        let binding = fixture.actor.bind_provider_root("src", source).unwrap();
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
        let input = serde_json::json!({
            "at":"src:Catalog.Items.Command.Refresh", "dryRun":true,
            "ops":[{"op":"command.set", "args":{"values":{"newName":"Renamed"}}}]
        });
        let request = ApplyRequest::parse(input.as_object().unwrap(), &["src"]).unwrap();
        let before_source = snapshot_tree(source);
        let before_cache = snapshot_tree(&cache);
        let (staged, effects) = plan_hidden_v13_apply(&request, &binding, &admission).unwrap();
        let prepared = admission.prepare_with_effects(staged, effects).unwrap();
        let raced = match phase {
            "existing-ancestor" => destination.clone(),
            "created-ancestor" => destination.parent().unwrap().to_path_buf(),
            "created-leaf" => destination.join("CommandModule.bsl"),
            _ => unreachable!(),
        };
        let renamed =
            raced.with_file_name(raced.file_name().unwrap().to_str().unwrap().to_lowercase());
        let renamed_for_hook = renamed.clone();
        let _ = take_retained_apply_observed_events();
        set_retained_apply_before_post_validation_hook(move || {
            std::fs::rename(raced, renamed_for_hook).unwrap();
        });

        let error = fixture.actor.publish_prepared_apply(prepared).unwrap_err();

        assert_eq!(
            error.kind(),
            unica_workspace::actor::ApplyPublicationErrorKind::ConcurrentRevision,
            "{phase}: {error}"
        );
        assert_eq!(snapshot_tree(&cache), before_cache, "{phase}");
        if phase == "existing-ancestor" {
            // The external actor owns this spelling change; rollback owns only our files.
            std::fs::rename(&renamed, &destination).unwrap();
        }
        assert_eq!(snapshot_tree(source), before_source, "{phase}");
        let events = take_retained_apply_observed_events();
        assert!(
            events
                .iter()
                .any(|event| matches!(event, RetainedApplyObservedEvent::StateMarker(_))),
            "{phase}: {events:?}"
        );
        assert!(
            events
                .iter()
                .any(|event| matches!(event, RetainedApplyObservedEvent::Rollback(_))),
            "{phase}: {events:?}"
        );
        fixture.cleanup();
    }
}
