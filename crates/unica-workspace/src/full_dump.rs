//! The private runner project of a working copy: its projected configuration
//! and the ownership lock serializing runner invocations.
pub use crate::platform::full_dump::{
    current_host_platform, HostPlatform, RunnerProjectLockFailure, RunnerProjectStore,
    SerializedDumpProjection,
};
#[cfg(any(test, feature = "test-support"))]
pub mod test_support {
    #[cfg(windows)]
    pub use crate::platform::full_dump::with_effective_config_create_hook;
    pub use crate::platform::full_dump::{
        fixture_file_identity, fixture_verify_private_owner, set_before_fixture_config_rename_hook,
        with_runner_projection_failpoint, with_runner_projection_hook,
        with_test_project_lock_contention, RunnerProjectionCheckpoint,
    };
}
