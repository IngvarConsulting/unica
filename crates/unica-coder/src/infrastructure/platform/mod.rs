mod entrypoint;
pub(crate) mod filesystem;
pub(crate) mod full_dump_publication;
mod process;
pub(crate) mod secure_read;
pub(crate) mod source_revision_fence;
mod target;
#[cfg(test)]
pub(crate) mod testing;

pub use entrypoint::run_platform_main;
pub(crate) use filesystem::short_private_runtime_dir;
#[cfg(all(test, windows))]
pub(crate) use process::assert_windows_runtime_process_tree_semantics_for_test;
#[cfg(test)]
pub(crate) use process::runtime_process_tree_test_scenario_for_test;
pub(crate) use process::{
    ensure_truncation_diagnostics, ManagedChild, ManagedCommand, ManagedLineOutput, ManagedOutput,
    ManagedStartupChild, RuntimeProcessTreeHandle, RuntimeProcessTreeState, StreamControl,
    STDERR_CAPTURE_LIMIT, STDOUT_CAPTURE_LIMIT,
};
pub(crate) use target::current_target_id;

#[cfg(test)]
pub(crate) fn assert_runtime_generation_authority_for_test() {
    #[cfg(unix)]
    process::assert_released_unix_group_never_signals_reused_identity_for_test();
    #[cfg(windows)]
    process::assert_windows_runtime_process_tree_semantics_for_test();
}
