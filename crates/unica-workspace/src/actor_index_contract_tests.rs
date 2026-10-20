use super::test_support::*;
use super::*;

pub fn remapped_names_and_profiles_do_not_share_index_or_coordination_state(
    observe_state_root: fn(
        &WorkspaceContext,
        &Path,
        Option<ProviderSourceLease>,
    ) -> Result<PathBuf, String>,
) {
    let root = temp_root("state-scope-separation");
    let source = root.join("src");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::write(source.join("Module.bsl"), "test").unwrap();
    let context = context(&root);
    let mut state_roots = Vec::new();

    for (name, profile) in [
        ("main", "program"),
        ("renamed", "program"),
        ("main", "program-and-service"),
    ] {
        let identity =
            WorkspaceIdentity::new(&context, [source_input(name, &source)], profile).unwrap();
        let actor = super::WorkspaceActor::<(), ()>::new(identity, context.clone())
            .map(bind_actor)
            .unwrap();
        let binding = actor.bind_provider_root(name, &source).unwrap();
        state_roots.push(
            observe_state_root(
                &context,
                &source,
                Some(actor.provider_source_lease(&binding).unwrap()),
            )
            .unwrap(),
        );
    }
    assert_ne!(state_roots[0], state_roots[1]);
    assert_ne!(state_roots[0], state_roots[2]);
    assert_ne!(state_roots[1], state_roots[2]);

    let legacy_identity = WorkspaceIdentity::new(
        &context,
        [WorkspaceSourceSetInput::new(
            "main",
            &source,
            SourceSetKind::Configuration,
            SourceFormat::Unknown,
            SourceProfile::legacy_workspace_service_compatibility(),
        )],
        "legacy-profile",
    )
    .unwrap();
    let legacy_actor =
        super::WorkspaceActor::<(), ()>::with_legacy_runtime(legacy_identity, context.clone(), ())
            .unwrap();
    let legacy_binding = legacy_actor.bind_provider_root("main", &source).unwrap();
    let direct_index = observe_state_root(&context, &source, None).unwrap();
    let actor_index = observe_state_root(
        &context,
        &source,
        Some(legacy_actor.provider_source_lease(&legacy_binding).unwrap()),
    )
    .unwrap();
    assert_eq!(
        direct_index, actor_index,
        "legacy adapter moved the v0.12 index/provider namespace"
    );
    if let Some((first, second)) =
        crate::platform::filesystem::distinct_non_unicode_paths_for_test()
    {
        let identity = |source_root| WorkspaceIdentity {
            workspace_root: PathBuf::from("/workspace"),
            source_sets: vec![super::WorkspaceSourceSetIdentity {
                name: "main".to_string(),
                root: source_root,
                kind: SourceSetKind::Configuration,
                source_format: SourceFormat::PlatformXml,
                source_profile: SourceProfile::platform_xml_8_3_27_format_2_20(),
            }],
            provider_profile: "profile".to_string(),
        };
        assert_ne!(
            identity(first).state_scope_digest().unwrap(),
            identity(second).state_scope_digest().unwrap(),
            "distinct native path identities collapsed actor state"
        );
    }
    let _ = std::fs::remove_dir_all(root);
}

pub fn actor_bound_index_session_rejects_another_source_set_root(
    observe_active_build: fn(&WorkspaceContext, &Path, ProviderSourceLease) -> Result<(), String>,
) {
    let fixture = actor_fixture("index-root-binding", &["A", "B"]);
    let binding = fixture
        .actor
        .bind_provider_root("A", &fixture.roots[0])
        .unwrap();
    let error = observe_active_build(
        fixture.actor.context(),
        &fixture.roots[1],
        fixture.actor.provider_source_lease(&binding).unwrap(),
    )
    .unwrap_err();

    assert!(
        error.contains("escaped its actor-bound source root"),
        "{error}"
    );
    fixture.cleanup();
}
