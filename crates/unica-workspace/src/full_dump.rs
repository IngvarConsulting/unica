//! Opaque retained full-dump preparation and publication.
//! Filesystem objects remain private to the workspace owner.
pub use crate::platform::full_dump::{
    current_host_platform, observe_executable_permissions, CapturedDumpTree, ConfigInputSnapshot,
    DumpCleanupObservation, DumpTargetCapture, ExecutablePermissionObservation, FullDumpFailure,
    FullDumpFinish, FullDumpPhase, HostPlatform, PlatformAttestation, PreparationFailure,
    PreparedPublication, PublishedFullDump, RunnerProjectLockFailure, RunnerProjectStore,
    SerializedDumpProjection, UnconfiguredDump, VerifiedFullDumpPublication,
};
#[cfg(any(test, feature = "test-support"))]
pub mod test_support {
    pub use crate::platform::full_dump::retained_recovery_contract::run as retained_recovery_identity_contract;
    pub use crate::platform::full_dump::set_before_fixture_config_rename_hook;
    pub use crate::platform::full_dump::test_support_os::*;
    pub use crate::platform::full_dump::{
        capture_directory_snapshot, capture_tree_target_nofollow, fixture_file_identity,
        fixture_verify_private_owner, inspect_and_validate_stage, secure_read_regular_file,
        with_directory_cleanup_open_hook, with_effective_config_prepare_hook,
        with_private_create_hook, with_publication_failpoint, with_publication_hook,
        with_runner_projection_failpoint, with_runner_projection_hook, with_secure_read_hook,
        with_target_parent_capture_hook, with_targeted_read_hooks,
        with_test_project_lock_contention, with_tree_open_hook, PublicationCheckpoint,
        PublicationObservation, RunnerProjectionCheckpoint, TreeEntryKind, TreeEntrySnapshot,
        TreeSnapshot,
    };
}
