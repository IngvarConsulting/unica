use super::test_support::{bind_actor, policies_scope};
use super::*;
use crate::platform::testing::{
    attempt_retained_directory_replacement_for_test, create_directory_link_fixture_for_test,
    remove_dir_symlink_for_test, FileLinkFixtureOutcome, RetainedDirectoryReplacementOutcome,
};
use crate::test_support::{canonical_path, tree_snapshot};
use unica_source_contract::events::DomainEventKind;

fn project(root: &Path) {
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
    )
    .unwrap();
    std::fs::write(
        root.join("src/Configuration.xml"),
        "<MetaDataObject><Configuration/></MetaDataObject>",
    )
    .unwrap();
    std::fs::write(root.join("src/Module.bsl"), b"before").unwrap();
}

fn actor(workspace: &Path, cache: &Path) -> WorkspaceActor {
    let context = WorkspaceContext {
        cwd: workspace.to_path_buf(),
        workspace_root: workspace.to_path_buf(),
        cache_root: cache.to_path_buf(),
        workspace_epoch: 1,
    };
    let identity = WorkspaceIdentity::new(
        &context,
        [WorkspaceSourceSetInput::new(
            "main",
            workspace.join("src"),
            SourceSetKind::Configuration,
            SourceFormat::PlatformXml,
            SourceProfile::platform_xml_8_3_27_format_2_20(),
        )],
        "test-provider",
    )
    .unwrap();
    WorkspaceActor::new(identity, context)
        .map(bind_actor)
        .unwrap()
}

fn admit(actor: &WorkspaceActor) -> Result<ApplyAdmission, ApplyAdmissionError> {
    let binding = actor
        .bind_provider_root("main", &actor.context.workspace_root.join("src"))
        .unwrap();
    actor.admit_apply(
        &binding,
        None,
        false,
        ProviderDeadline::from_budget(Duration::from_secs(5)),
        &CancellationToken::new(),
    )
}

fn prepare(actor: &WorkspaceActor) -> PreparedApplyBatch {
    let admitted = admit(actor).unwrap();
    assert_eq!(
        admitted.context.cache_root,
        admitted.workspace_cache.logical_root
    );
    let mut state = admitted.staged_state().unwrap();
    state
        .replace("Module.bsl", b"before", b"after".to_vec())
        .unwrap();
    admitted
        .prepare_with_cache_effects(
            state,
            &[DomainEvent::new(
                DomainEventKind::MetadataChanged,
                "Catalog.Products",
            )],
        )
        .unwrap()
}

fn directory_link(source: &Path, target: &Path) -> bool {
    match create_directory_link_fixture_for_test(source, target).unwrap() {
        FileLinkFixtureOutcome::Created => true,
        FileLinkFixtureOutcome::Unsupported
        | FileLinkFixtureOutcome::WindowsPrivilegeUnavailable => {
            eprintln!("[SKIPPED FIXTURE] host cannot create a directory link");
            false
        }
    }
}

pub fn external_cache_parent_components_publish_without_creating_cancelled_directories(
    policies: crate::policies::WorkspacePolicies,
) {
    let _policy_scope = policies_scope(policies);
    for existing in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let base = canonical_path(temp.path());
        let workspace = base.join("workspace");
        project(&workspace);
        let outside = base.join("outside");
        std::fs::create_dir(&outside).unwrap();
        let cancelled = outside.join("cancelled");
        if existing {
            std::fs::create_dir(&cancelled).unwrap();
        }
        let cache = outside.join("cancelled/../missing/cache");
        let actor = actor(&workspace, &cache);
        let before = tree_snapshot(&base);
        let prepared = prepare(&actor);
        assert_eq!(tree_snapshot(&base), before);
        assert_eq!(cancelled.exists(), existing);
        assert!(!outside.join("missing").exists());
        assert_eq!(
            prepared.workspace_cache.logical_root,
            outside.join("missing/cache")
        );
        actor.publish_prepared_apply(prepared).unwrap();
        assert_eq!(
            std::fs::read(workspace.join("src/Module.bsl")).unwrap(),
            b"after"
        );
        assert!(outside.join("missing/cache/state.json").is_file());
        assert_eq!(cancelled.exists(), existing);
        assert!(!workspace.join(".build").exists());
    }
}

pub fn external_cache_route_rejects_links_including_components_cancelled_by_parent(
    policies: crate::policies::WorkspacePolicies,
) -> Result<(), crate::test_support::CapabilitySkip> {
    let _policy_scope = policies_scope(policies);
    for suffix in ["link", "link/cache", "link/../cache"] {
        let temp = tempfile::tempdir().unwrap();
        let base = canonical_path(temp.path());
        let workspace = base.join("workspace");
        project(&workspace);
        let outside = base.join("outside");
        let foreign = base.join("foreign");
        std::fs::create_dir(&outside).unwrap();
        std::fs::create_dir(&foreign).unwrap();
        std::fs::write(foreign.join("keep.bin"), b"foreign").unwrap();
        if !directory_link(&foreign, &outside.join("link")) {
            return Err(crate::test_support::CapabilitySkip {
                capability: "symlink",
                reason: "creating a directory link needs the symlink privilege",
            });
        }
        let actor = actor(&workspace, &outside.join(suffix));
        let before = tree_snapshot(&workspace);
        let error = admit(&actor).unwrap_err();
        assert!(
            error.to_string().contains("workspace cache authority"),
            "{suffix}: {error}"
        );
        assert_eq!(tree_snapshot(&workspace), before);
        assert_eq!(std::fs::read(foreign.join("keep.bin")).unwrap(), b"foreign");
        assert!(!foreign.join("state.json").exists());
        assert!(!outside.join("cache").exists());
        assert!(std::fs::symlink_metadata(outside.join("link")).is_ok());
    }
    Ok(())
}

pub fn external_cache_ancestor_replacement_does_not_publish_source_or_foreign_cache(
    policies: crate::policies::WorkspacePolicies,
) {
    let _policy_scope = policies_scope(policies);
    for existing in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let base = canonical_path(temp.path());
        let workspace = base.join("workspace");
        project(&workspace);
        let outside = base.join("outside");
        let cache = outside.join("nested/cache");
        std::fs::create_dir(&outside).unwrap();
        if existing {
            std::fs::create_dir_all(&cache).unwrap();
            std::fs::write(cache.join("keep.bin"), b"original cache").unwrap();
        }
        let actor = actor(&workspace, &cache);
        let prepared = prepare(&actor);
        let before = tree_snapshot(&outside);
        let displaced = base.join("displaced");
        match attempt_retained_directory_replacement_for_test(&outside, &displaced).unwrap() {
            RetainedDirectoryReplacementOutcome::Replaced => {
                std::fs::create_dir_all(&cache).unwrap();
                std::fs::write(cache.join("keep.bin"), b"foreign cache").unwrap();
                let error = actor.publish_prepared_apply(prepared).unwrap_err();
                assert_eq!(error.kind(), ApplyPublicationErrorKind::ContainmentIdentity);
                assert_eq!(
                    std::fs::read(workspace.join("src/Module.bsl")).unwrap(),
                    b"before"
                );
                assert_eq!(
                    std::fs::read(cache.join("keep.bin")).unwrap(),
                    b"foreign cache"
                );
                assert!(!cache.join("state.json").exists());
                assert_eq!(tree_snapshot(&displaced), before);
            }
            RetainedDirectoryReplacementOutcome::PreventedByRetainedHandle => {
                eprintln!("[SKIPPED FIXTURE] retained handle prevents ancestor replacement");
                assert!(!displaced.exists());
                actor.publish_prepared_apply(prepared).unwrap();
                assert!(cache.join("state.json").is_file());
            }
        }
    }
}

pub fn external_cache_missing_tail_occupation_keeps_foreign_entry_and_source(
    policies: crate::policies::WorkspacePolicies,
) {
    let _policy_scope = policies_scope(policies);
    let temp = tempfile::tempdir().unwrap();
    let base = canonical_path(temp.path());
    let workspace = base.join("workspace");
    project(&workspace);
    let outside = base.join("outside");
    std::fs::create_dir(&outside).unwrap();
    let actor = actor(&workspace, &outside.join("missing/cache"));
    let prepared = prepare(&actor);
    assert!(!outside.join("missing").exists());
    std::fs::write(outside.join("missing"), b"foreign occupant").unwrap();
    actor.publish_prepared_apply(prepared).unwrap_err();
    assert_eq!(
        std::fs::read(workspace.join("src/Module.bsl")).unwrap(),
        b"before"
    );
    assert_eq!(
        std::fs::read(outside.join("missing")).unwrap(),
        b"foreign occupant"
    );
}

pub fn shared_workspace_alias_does_not_retarget_prepared_external_or_default_cache(
    policies: crate::policies::WorkspacePolicies,
) -> Result<(), crate::test_support::CapabilitySkip> {
    let _policy_scope = policies_scope(policies);
    for external in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let base = canonical_path(temp.path());
        let original = base.join("original");
        let foreign = base.join("foreign");
        project(&original.join("workspace"));
        project(&foreign.join("workspace"));
        let alias = base.join("alias");
        if !directory_link(&original, &alias) {
            return Err(crate::test_support::CapabilitySkip {
                capability: "symlink",
                reason: "creating a directory link needs the symlink privilege",
            });
        }
        let cache_suffix = if external {
            "cache"
        } else {
            "workspace/.build/unica"
        };
        let actor = actor(&alias.join("workspace"), &alias.join(cache_suffix));
        let prepared = prepare(&actor);
        assert_eq!(
            prepared.workspace_cache.logical_root,
            original.join(cache_suffix)
        );
        assert!(!original.join(cache_suffix).exists());
        remove_dir_symlink_for_test(&alias).unwrap();
        assert!(directory_link(&foreign, &alias));
        actor.publish_prepared_apply(prepared).unwrap();
        assert_eq!(
            std::fs::read(original.join("workspace/src/Module.bsl")).unwrap(),
            b"after"
        );
        assert_eq!(
            std::fs::read(foreign.join("workspace/src/Module.bsl")).unwrap(),
            b"before"
        );
        assert!(original.join(cache_suffix).join("state.json").is_file());
        assert!(!foreign.join(cache_suffix).exists());
    }
    Ok(())
}

pub fn workspace_suffix_alias_cannot_disguise_a_different_shared_cache_ancestor(
    policies: crate::policies::WorkspacePolicies,
) -> Result<(), crate::test_support::CapabilitySkip> {
    let _policy_scope = policies_scope(policies);
    let temp = tempfile::tempdir().unwrap();
    let base = canonical_path(temp.path());
    let original = base.join("original");
    let foreign = base.join("foreign");
    project(&original.join("workspace"));
    std::fs::create_dir(&foreign).unwrap();
    let alias = base.join("alias");
    if !directory_link(&foreign, &alias)
        || !directory_link(&original.join("workspace"), &foreign.join("workspace"))
    {
        return Err(crate::test_support::CapabilitySkip {
            capability: "symlink",
            reason: "creating a directory link needs the symlink privilege",
        });
    }
    // The workspace still resolves to the original identity, but only through
    // another link beneath the common prefix. It cannot authorize foreign/cache.
    let actor = actor(&alias.join("workspace"), &alias.join("cache"));
    assert_eq!(actor.context.cache_root, alias.join("cache"));
    assert_eq!(actor.context.workspace_root, original.join("workspace"));
    let before = tree_snapshot(&original);
    let error = admit(&actor).unwrap_err();
    assert!(
        error.to_string().contains("workspace cache authority"),
        "{error}"
    );
    assert_eq!(tree_snapshot(&original), before);
    assert!(!foreign.join("cache").exists());
    Ok(())
}

pub fn relative_cache_uses_process_cwd_when_workspace_has_an_alias(
    policies: crate::policies::WorkspacePolicies,
) -> Result<(), crate::test_support::CapabilitySkip> {
    let _policy_scope = policies_scope(policies);
    let temp = tempfile::tempdir().unwrap();
    let base = canonical_path(temp.path());
    let workspace = base.join("workspace");
    project(&workspace);
    let alias = base.join("alias");
    if !directory_link(&workspace, &alias) {
        return Err(crate::test_support::CapabilitySkip {
            capability: "symlink",
            reason: "creating a directory link needs the symlink privilege",
        });
    }
    let _cwd = crate::test_support::ProcessCwdGuard::enter(&base).unwrap();
    let actor = actor(&alias, Path::new("alias/cache"));
    let before = tree_snapshot(&workspace);
    let prepared = prepare(&actor);
    assert_eq!(
        prepared.workspace_cache.logical_root,
        workspace.join("cache")
    );
    assert_eq!(tree_snapshot(&workspace), before);
    assert!(!workspace.join("cache").exists());
    actor.publish_prepared_apply(prepared).unwrap();
    assert_eq!(
        std::fs::read(workspace.join("src/Module.bsl")).unwrap(),
        b"after"
    );
    assert!(workspace.join("cache/state.json").is_file());
    assert!(!base.join("cache").exists());
    Ok(())
}

pub fn cache_override_drive_prefix_follows_host_path_semantics_without_erasing_links(
    policies: crate::policies::WorkspacePolicies,
) -> Result<(), crate::test_support::CapabilitySkip> {
    let _policy_scope = policies_scope(policies);
    use std::path::{Component, Prefix};

    let temp = tempfile::tempdir().unwrap();
    let base = canonical_path(temp.path());
    let workspace = base.join("workspace");
    project(&workspace);
    let (cache, cache_name, link_name, linked_cache) =
        match Path::new("C:cache").components().next() {
            Some(Component::Prefix(candidate)) if matches!(candidate.kind(), Prefix::Disk(_)) => {
                let Some(Component::Prefix(prefix)) = workspace.components().next() else {
                    panic!("drive-relative fixture workspace has no drive prefix");
                };
                assert!(matches!(prefix.kind(), Prefix::Disk(_)));
                let mut cache = PathBuf::from(prefix.as_os_str());
                cache.push("cache");
                let mut linked_cache = PathBuf::from(prefix.as_os_str());
                linked_cache.push("link/../other-cache");
                (cache, "cache", "link", linked_cache)
            }
            Some(Component::Normal(name)) => {
                assert_eq!(name, "C:cache");
                (
                    PathBuf::from("C:cache"),
                    "C:cache",
                    "C:link",
                    PathBuf::from("C:link/../other-cache"),
                )
            }
            component => panic!("unsupported drive-relative fixture component: {component:?}"),
        };
    let _cwd = crate::test_support::ProcessCwdGuard::enter(&workspace).unwrap();
    assert!(!cache.is_absolute());
    let owner = actor(&workspace, &cache);
    let before = tree_snapshot(&workspace);
    let prepared = prepare(&owner);
    assert_eq!(
        prepared.workspace_cache.logical_root,
        workspace.join(cache_name)
    );
    assert_eq!(tree_snapshot(&workspace), before);
    assert!(!workspace.join(cache_name).exists());
    owner.publish_prepared_apply(prepared).unwrap();
    assert!(workspace.join(cache_name).join("state.json").is_file());
    assert_eq!(
        std::fs::read(workspace.join("src/Module.bsl")).unwrap(),
        b"after"
    );

    let foreign = base.join("foreign");
    std::fs::create_dir(&foreign).unwrap();
    std::fs::write(foreign.join("keep.bin"), b"foreign").unwrap();
    if !directory_link(&foreign, &workspace.join(link_name)) {
        return Err(crate::test_support::CapabilitySkip {
            capability: "symlink",
            reason: "creating a directory link needs the symlink privilege",
        });
    }
    let owner = actor(&workspace, &linked_cache);
    admit(&owner).expect_err("cache route resolution erased link/.. before no-follow");
    assert!(!workspace.join("other-cache").exists());
    assert_eq!(std::fs::read(foreign.join("keep.bin")).unwrap(), b"foreign");
    Ok(())
}
