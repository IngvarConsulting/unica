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
pub(crate) use process::{
    ensure_truncation_diagnostics, ManagedChild, ManagedCommand, ManagedLineOutput, ManagedOutput,
    ManagedStartupChild, RuntimeProcessTreeHandle, RuntimeProcessTreeState, StreamControl,
    STDERR_CAPTURE_LIMIT, STDOUT_CAPTURE_LIMIT,
};
#[cfg(test)]
pub(crate) use process::{
    inject_runtime_tree_cleanup_timeout_for_test, reset_runtime_tree_cleanup_calls_for_test,
    runtime_process_tree_test_scenario_for_test, runtime_tree_cleanup_calls_for_test,
};
pub(crate) use target::current_target_id;

#[cfg(feature = "receipt-ledger-test-support")]
pub(crate) const fn receipt_writer_wall_load_supported_for_test() -> bool {
    cfg!(unix)
}

#[cfg(test)]
pub(crate) fn unix_runtime_authority_tests_supported() -> bool {
    cfg!(unix)
}

#[cfg(test)]
pub(crate) fn create_runtime_directory_link_for_test(
    target: &std::path::Path,
    link: &std::path::Path,
) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link)
    }
    #[cfg(not(unix))]
    {
        let _ = (target, link);
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "directory-link race fixture is unavailable on this host",
        ))
    }
}

#[cfg(test)]
pub(crate) fn inject_unix_waitid_error_for_test() {
    #[cfg(unix)]
    process::inject_unix_waitid_error_for_test();
}

#[cfg(test)]
pub(crate) fn inject_unix_reap_error_for_test() {
    #[cfg(unix)]
    process::inject_unix_reap_error_for_test();
}

#[cfg(test)]
pub(crate) fn reset_unix_signal_count_for_test() {
    #[cfg(unix)]
    process::reset_unix_signal_count_for_test();
}

#[cfg(test)]
pub(crate) fn unix_signal_count_for_test() -> u32 {
    #[cfg(unix)]
    {
        process::unix_signal_count_for_test()
    }
    #[cfg(not(unix))]
    0
}

#[cfg(test)]
pub(crate) fn reap_runtime_authority_test_child(process_id: u32) {
    #[cfg(unix)]
    process::reap_runtime_authority_test_child(process_id);
    #[cfg(not(unix))]
    let _ = process_id;
}

#[cfg(test)]
pub(crate) fn assert_runtime_generation_authority_for_test() {
    #[cfg(unix)]
    {
        process::assert_released_unix_group_never_signals_reused_identity_for_test();
        process::assert_runtime_ownership_sentinel_for_test();
    }
    #[cfg(windows)]
    process::assert_windows_runtime_process_tree_semantics_for_test()
        .expect("windows runtime process tree semantics must hold");
}
