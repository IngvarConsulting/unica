use crate::identity::{CoreIdentity, DaemonStateDirectory};
use std::path::PathBuf;
use unica_workspace::test_support::{
    create_directory_link_fixture_for_test, set_unix_mode_for_test, unix_mode_for_test,
    FileLinkFixtureOutcome,
};

fn physical_root(root: &std::path::Path) -> PathBuf {
    std::fs::canonicalize(root).unwrap()
}

#[test]
fn world_readable_identity_directory_is_rejected() {
    let root = tempfile::tempdir().unwrap();
    let physical = physical_root(root.path());
    let identity = CoreIdentity::production();
    let path = DaemonStateDirectory::path_for(&physical, &identity);
    std::fs::create_dir_all(&path).unwrap();
    if !set_unix_mode_for_test(&path, 0o755).unwrap() {
        return;
    }

    let error = DaemonStateDirectory::open(&physical, &identity).unwrap_err();
    assert!(error.contains("owner-only"), "{error}");
    assert_eq!(unix_mode_for_test(&path).unwrap(), Some(0o755));
}

#[test]
fn symlinked_provider_state_root_is_rejected_before_creating_identity_state() {
    let fixture = tempfile::tempdir().unwrap();
    let target = fixture.path().join("redirected-parent");
    std::fs::create_dir(&target).unwrap();
    let routed_parent = fixture.path().join("provider-parent-link");
    match create_directory_link_fixture_for_test(&target, &routed_parent).unwrap() {
        FileLinkFixtureOutcome::Created => {}
        FileLinkFixtureOutcome::Unsupported
        | FileLinkFixtureOutcome::WindowsPrivilegeUnavailable => return,
    }
    let routed = routed_parent.join("provider-state-created-through-link");
    let identity = CoreIdentity::production();

    assert!(DaemonStateDirectory::open(&routed, &identity).is_err());
    assert!(
        !target.join("provider-state-created-through-link").exists(),
        "rejected ambient symlink must not receive even the provider-state directory"
    );
}

#[test]
fn missing_provider_state_root_is_created_before_private_identity_child() {
    let fixture = tempfile::tempdir().unwrap();
    let physical_fixture = std::fs::canonicalize(fixture.path()).unwrap();
    let state_root = physical_fixture.join("cold").join("provider-state");
    let identity = CoreIdentity::production();

    let state = DaemonStateDirectory::open(&state_root, &identity).unwrap();

    assert!(state_root.is_dir());
    assert_eq!(
        state.path(),
        DaemonStateDirectory::path_for(&state_root, &identity)
    );
}
