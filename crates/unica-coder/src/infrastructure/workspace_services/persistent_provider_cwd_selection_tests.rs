use super::persistent_provider_cwd_command_tests::CwdFixture;
use super::*;
use crate::infrastructure::platform::filesystem::path_starts_with_host_root;
use crate::infrastructure::platform::testing::{
    create_directory_link_fixture_for_test, FileLinkFixtureOutcome,
};
use crate::test_support::canonical_path;

#[test]
fn working_directory_selection_falls_back_when_workspace_contains_the_preferred_runtime() {
    let fixture = CwdFixture::new(true);
    let preferred =
        persistent_provider_working_directory(&fixture.context.workspace_root, &fixture.source)
            .unwrap();
    let fallback = persistent_provider_working_directory(&preferred, &fixture.source)
        .expect("an available external directory keeps providers usable");
    assert!(fallback.is_absolute() && fallback.is_dir());
    for excluded in [&preferred, &fixture.source] {
        assert!(!path_starts_with_host_root(
            &fallback,
            &canonical_path(excluded)
        ));
    }
}

#[test]
fn working_directory_selection_stays_external_when_temp_and_executable_are_excluded() {
    let temp = canonical_path(&env::temp_dir());
    let executable = canonical_path(env::current_exe().unwrap().parent().unwrap());
    let fallback = persistent_provider_working_directory(&temp, &executable)
        .expect("opening a user profile must not exclude every available cwd");
    assert!(fallback.is_absolute() && fallback.is_dir());
    assert!(!path_starts_with_host_root(&fallback, &temp));
    assert!(!path_starts_with_host_root(&fallback, &executable));
}

#[test]
fn working_directory_selection_accepts_canonical_external_directories() {
    let fixture = CwdFixture::new(true);
    let child = fixture.external.join("child");
    let sibling = fixture.root.join("workspace-adjacent");
    fs::create_dir_all(&child).unwrap();
    fs::create_dir_all(&sibling).unwrap();
    for candidate in [fixture.external.clone(), child.join(".."), sibling] {
        let directory = checked_persistent_provider_working_directory(
            &candidate,
            &fixture.context.workspace_root,
            &fixture.source,
        )
        .expect("an existing physically external directory is usable");
        assert_eq!(directory, canonical_path(&candidate));
        assert!(directory.is_absolute() && directory.is_dir());
    }
}

#[test]
fn working_directory_selection_rejects_workspace_root_and_descendants() {
    let fixture = CwdFixture::new(true);
    for candidate in [&fixture.context.workspace_root, &fixture.context.cwd] {
        assert!(
            checked_persistent_provider_working_directory(
                candidate,
                &fixture.context.workspace_root,
                &fixture.source,
            )
            .is_err(),
            "a project directory must not become the persistent cwd: {candidate:?}"
        );
    }
}

#[test]
fn working_directory_selection_rejects_independent_source_root_and_descendants() {
    let fixture = CwdFixture::new(true);
    let source = fixture.root.join("independent-source");
    let nested = source.join("nested");
    fs::create_dir_all(&nested).unwrap();
    assert!(!source.starts_with(&fixture.context.workspace_root));
    for candidate in [&source, &nested] {
        assert!(
            checked_persistent_provider_working_directory(
                candidate,
                &fixture.context.workspace_root,
                &source,
            )
            .is_err(),
            "a source outside the project still excludes its own cwd: {candidate:?}"
        );
    }
}

#[test]
fn working_directory_selection_rejects_missing_and_regular_file_candidates() {
    let fixture = CwdFixture::new(false);
    let absent = fixture.external.join("missing");
    let file = fixture.external.join("not-a-directory");
    fs::write(&file, b"preserve this file").unwrap();
    for candidate in [&absent, &file] {
        assert!(
            checked_persistent_provider_working_directory(
                candidate,
                &fixture.context.workspace_root,
                &fixture.source,
            )
            .is_err(),
            "unusable candidate must refuse before spawn: {candidate:?}"
        );
    }
    assert!(!absent.exists());
    assert_eq!(fs::read(&file).unwrap(), b"preserve this file");
}

fn directory_alias(source: &Path, alias: &Path) {
    let outcome = create_directory_link_fixture_for_test(source, alias)
        .expect("directory alias fixture creation must report its real OS error");
    assert_eq!(
        outcome,
        FileLinkFixtureOutcome::Created,
        "directory alias fixture unavailable; this is not alias-containment acceptance"
    );
}

#[test]
fn working_directory_selection_uses_physical_candidate_and_root_identity() {
    let fixture = CwdFixture::new(true);
    let source = fixture.root.join("independent-source");
    let nested_source = source.join("nested");
    fs::create_dir_all(&nested_source).unwrap();
    let workspace_alias = fixture.external.join("workspace-alias");
    let source_alias = fixture.external.join("source-alias");
    let external_alias = fixture.root.join("external-alias");
    directory_alias(&fixture.context.workspace_root, &workspace_alias);
    directory_alias(&source, &source_alias);
    directory_alias(&fixture.external, &external_alias);

    for candidate in [
        workspace_alias.join("nested/deeper"),
        source_alias.join("nested"),
    ] {
        assert!(
            checked_persistent_provider_working_directory(
                &candidate,
                &fixture.context.workspace_root,
                &source,
            )
            .is_err(),
            "an external alias cannot hide physical containment: {candidate:?}"
        );
    }
    assert!(checked_persistent_provider_working_directory(
        &fixture.context.cwd,
        &workspace_alias,
        &source,
    )
    .is_err());
    assert!(checked_persistent_provider_working_directory(
        &nested_source,
        &fixture.context.workspace_root,
        &source_alias,
    )
    .is_err());

    assert_eq!(
        checked_persistent_provider_working_directory(
            &external_alias,
            &workspace_alias,
            &source_alias,
        )
        .expect("an alias to a physically external directory remains usable"),
        canonical_path(&fixture.external)
    );
}

#[test]
fn persistent_commands_reject_when_every_candidate_is_inside_workspace_or_source() {
    let fixture = CwdFixture::new(true);
    let physical_temp = canonical_path(&env::temp_dir());
    let anchor = physical_temp.ancestors().last().unwrap().to_path_buf();
    let executable = canonical_path(env::current_exe().unwrap().parent().unwrap());
    // On Windows the executable may live on another drive. Exclude its own
    // directory separately while leaving the calculated cache outside source.
    let source = executable.as_path();
    let calculated_cache = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(".session-temp")
        .join(format!("uncreated-cwd-selection-cache-{}", Uuid::new_v4()));
    assert!(!calculated_cache.exists());
    let context = WorkspaceContext {
        cwd: fixture.context.cwd.clone(),
        workspace_root: anchor,
        cache_root: calculated_cache.clone(),
        workspace_epoch: 1,
    };
    let cache = bsl_analyzer_cache_dir(&context, source)
        .expect("the cache remains valid so it cannot mask a cwd refusal");
    assert!(!path_starts_with_host_root(
        &cache,
        &normalize_path_identity(source).unwrap()
    ));
    let identity = WorkspaceServiceIdentity::new(&context, source).unwrap();
    let commands = [
        (
            "RLM",
            rlm_transport_command_for_build(
                PathBuf::from("never-spawned-rlm"),
                &context,
                source,
                &fixture.build,
            ),
        ),
        (
            "BSL",
            bsl_analyzer_command(Path::new("never-spawned-bsl"), &context, source),
        ),
        (
            "helper",
            workspace_service_command(
                Path::new("never-spawned-helper"),
                &identity,
                WorkspaceServiceConfig { idle_secs: 321 },
                "selection-test-token",
                &CoreIdentity::production(),
            ),
        ),
    ];
    for (provider, result) in commands {
        let error = result.expect_err("every contained candidate must prevent Command delivery");
        assert!(
            error.contains("persistent provider working directory")
                && error.contains("outside the workspace and source tree"),
            "{provider} must refuse at the cwd owner, not at an unrelated guard: {error}"
        );
    }
    assert!(!calculated_cache.exists());
}
