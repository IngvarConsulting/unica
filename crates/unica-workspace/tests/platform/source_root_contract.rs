//! The source root policy is checked against its actual workspace implementation.

#[cfg(unix)]
#[test]
fn source_root_policy_rejects_parent_traversal_after_directory_symlink() {
    crate::test_support::source_root_policy_rejects_parent_traversal_after_directory_symlink(
        crate::discovery::normalize_contained_source_root,
    );
}
