//! Workspace-owned source access, retained state and publication.
mod platform;

pub mod paths;
pub mod read;
pub mod state_fs;

pub mod preview;

pub mod actor;
pub mod apply;
pub mod cache;
pub mod compile_transaction;
mod deadline_lock;
pub mod path_policy;
pub mod policies;
#[allow(dead_code)]
pub mod policy_evidence;
mod retained_directory;
pub mod selection_evidence;
pub mod single_file_publisher;
pub mod state;
pub mod state_scope;

#[cfg(any(test, feature = "test-support"))]
pub mod test_support {
    /// A contract test that cannot run on this host (#1893). Workspace may use
    /// `unica-process` only in tests, so the contract names the capability and
    /// its `#[test]` wrapper reports the skip through
    /// `unica_process::test_support::skip_named_host_capability`.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct CapabilitySkip {
        pub capability: &'static str,
        pub reason: &'static str,
    }

    pub use crate::platform::filesystem_contract_tests::{
        saved_source_diff_keeps_literal_backslash_and_nested_paths_distinct,
        saved_source_diff_keeps_native_names_and_replacement_character_distinct,
        source_root_policy_rejects_parent_traversal_after_directory_symlink,
        source_set_relative_path_accepts_a_verbatim_root_without_widening_containment,
    };
    mod shared;
    pub use shared::{
        canonical_path, tree_snapshot, tree_snapshot_with_empty_owned_files, ProcessCwdGuard,
    };
    pub fn read_file_from_fixture_file(file: std::fs::File) -> crate::read::ReadFile {
        crate::read::ReadFile { file }
    }
    pub fn prepare_file_for_removal(path: &std::path::Path) -> std::io::Result<()> {
        crate::platform::filesystem::prepare_file_for_removal(path)
    }

    pub struct FixturePermissions(crate::platform::filesystem::PortablePermissions);

    impl FixturePermissions {
        pub fn matches(&self, metadata: &std::fs::Metadata) -> bool {
            self.0.matches(metadata)
        }
    }

    pub fn portable_permissions_for_test(metadata: &std::fs::Metadata) -> FixturePermissions {
        FixturePermissions(crate::platform::filesystem::portable_permissions(metadata))
    }

    /// Construct the same retained fixture state without exporting its raw
    /// filesystem capability or its writer token to another test crate.
    pub fn staged_state(
        root: &std::path::Path,
        deadline: unica_work_control::provider_deadline::ProviderDeadline,
        cancellation: unica_work_control::cancellation::CancellationToken,
    ) -> std::io::Result<crate::apply::ApplyStagedState> {
        let root = crate::platform::filesystem::RetainedDirectoryCapability::open(root)?;
        Ok(crate::apply::ApplyStagedState::from_retained_root(
            std::sync::Arc::new(root),
            deadline,
            cancellation,
            crate::actor::apply_writer_authority_for_test(),
        ))
    }

    pub fn provider_source_lease(
        root: &std::path::Path,
    ) -> std::io::Result<crate::actor::ProviderSourceLease> {
        let root = crate::platform::filesystem::RetainedDirectoryCapability::open(root)?;
        Ok(crate::actor::ProviderSourceLease::for_test(
            std::sync::Arc::new(root),
        ))
    }

    pub use crate::platform::test_support as native;

    pub mod retained_read_probe {
        pub use crate::platform::filesystem::retained_read_probe::{
            RetainedRead, RetainedReadProbe,
        };
    }
    pub use crate::platform::filesystem::{
        create_test_directory_link, directory_is_owner_only, distinct_non_unicode_paths_for_test,
        inject_post_rename_sync_failure_for_test, supports_retained_root_replacement_test,
    };
    pub use crate::platform::testing::{
        attempt_retained_directory_replacement_for_test,
        attempt_retained_regular_file_relocation_for_test,
        can_rename_parent_with_retained_cleanup_child_for_test,
        can_swap_named_child_behind_retained_handle_for_test,
        case_distinct_provider_paths_for_test, clear_execute_permission_for_test,
        create_dir_symlink_for_test, create_directory_link_fixture_for_test,
        create_file_link_fixture_for_test, create_file_symlink_for_test,
        distinct_non_utf8_provider_paths_for_test, file_identity_for_test,
        non_utf8_relative_path_for_test, normalize_path_text_for_test, open_handle_count_for_test,
        path_identity_for_test, path_text_for_test, remove_dir_symlink_for_test,
        set_unix_mode_for_test, sharing_violation_error_cases_for_test, unix_mode_for_test,
        windows_extended_length_path_for_test, FileLinkFixtureOutcome,
        RetainedDirectoryReplacementOutcome, RetainedRegularFileRelocationOutcome,
    };
}

pub mod full_dump;

pub mod discovery;
