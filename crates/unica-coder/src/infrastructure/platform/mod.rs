mod entrypoint;
#[cfg(test)]
mod network_trust_tests;
#[cfg(test)]
mod process_integration_tests;
mod target;
mod view_snapshot_spill;

pub use entrypoint::run_platform_main;
pub(crate) use target::current_target_id;
pub(crate) use view_snapshot_spill::{TempFileSpill, VIEW_SNAPSHOT_MAX_RETAINED_BYTES};

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
pub(crate) use unica_process::test_support::{
    inject_unix_waitid_error_for_test, reap_runtime_authority_test_child,
};

#[cfg(test)]
pub(crate) fn assert_runtime_generation_authority_for_test() {
    #[cfg(unix)]
    {
        unica_process::test_support::assert_released_unix_group_never_signals_reused_identity_for_test();
    }
    #[cfg(windows)]
    unica_process::test_support::assert_windows_runtime_process_tree_semantics_for_test()
        .expect("windows runtime process tree semantics must hold");
}

#[cfg(test)]
mod workspace_contract_tests;
