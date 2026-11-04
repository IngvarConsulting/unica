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
        unica_process::test_support::skip_without_host_capability(
            unica_process::test_support::HostCapability::UnixPermissionBits,
            "owner-only state checks Unix permission bits",
        );
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
        | FileLinkFixtureOutcome::WindowsPrivilegeUnavailable => {
            unica_process::test_support::skip_without_host_capability(
                unica_process::test_support::HostCapability::Symlink,
                "creating a directory link needs the symlink privilege",
            );
            return;
        }
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

/// #2252: a link among the ancestors of the state root (macOS `/var` →
/// `/private/var`) does not stop the daemon state from opening once the root
/// is resolved, while a state root that is itself a link is refused.
#[test]
#[cfg_attr(
    not(unix),
    ignore = "host capability: ancestor links of the state root are resolved on Unix"
)]
fn a_linked_ancestor_opens_and_a_linked_state_root_is_refused() {
    use unica_workspace::state_fs::state_root_with_resolved_ancestors;
    let fixture = tempfile::tempdir().unwrap();
    let real = fixture.path().join("real");
    std::fs::create_dir(&real).unwrap();
    let alias = fixture.path().join("alias");
    match create_directory_link_fixture_for_test(&real, &alias).unwrap() {
        FileLinkFixtureOutcome::Created => {}
        FileLinkFixtureOutcome::Unsupported
        | FileLinkFixtureOutcome::WindowsPrivilegeUnavailable => {
            unica_process::test_support::skip_without_host_capability(
                unica_process::test_support::HostCapability::Symlink,
                "creating a directory link needs the symlink privilege",
            );
            return;
        }
    }
    let identity = CoreIdentity::production();
    let given = alias.join("provider-state");
    assert!(DaemonStateDirectory::open(&given, &identity).is_err());
    let resolved = state_root_with_resolved_ancestors(&given).unwrap();
    assert_eq!(resolved, physical_root(&real).join("provider-state"));
    DaemonStateDirectory::open(&resolved, &identity)
        .expect("the state opens below a linked ancestor");

    let linked_root = fixture.path().join("linked-state");
    match create_directory_link_fixture_for_test(&resolved, &linked_root).unwrap() {
        FileLinkFixtureOutcome::Created => {}
        FileLinkFixtureOutcome::Unsupported
        | FileLinkFixtureOutcome::WindowsPrivilegeUnavailable => {
            unica_process::test_support::skip_without_host_capability(
                unica_process::test_support::HostCapability::Symlink,
                "creating a directory link needs the symlink privilege",
            );
            return;
        }
    }
    let resolved = state_root_with_resolved_ancestors(&linked_root).unwrap();
    assert!(
        DaemonStateDirectory::open(&resolved, &identity).is_err(),
        "a state root replaced by a link is still refused"
    );
}

/// A missing `..` ancestor is not dropped: the path keeps naming the same
/// directory.
#[test]
fn a_missing_parent_reference_keeps_the_given_state_root() {
    use unica_workspace::state_fs::state_root_with_resolved_ancestors;
    let fixture = tempfile::tempdir().unwrap();
    let given = fixture
        .path()
        .join("missing")
        .join("..")
        .join("provider-state");
    assert_eq!(state_root_with_resolved_ancestors(&given).unwrap(), given);
}
