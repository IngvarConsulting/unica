#[cfg(windows)]
mod daemon_gate;
#[cfg(windows)]
mod daemon_owner;
#[cfg(not(windows))]
mod daemon_owner_unsupported;
mod entrypoint;
mod filesystem;
#[cfg(windows)]
mod job_object;
#[cfg(windows)]
mod pending_daemon;
mod process;

pub use entrypoint::run_platform_main;
pub(crate) use filesystem::file_uri_host_names_share;
#[cfg(test)]
pub(crate) use filesystem::link_directory_for_test;
pub(crate) use process::runtime_command;
pub use process::{launch_plugin_runtime, launch_runtime, RuntimeHandoff};

#[cfg(windows)]
pub use daemon_gate::await_bootstrap_daemon_publication;
#[cfg(windows)]
pub use daemon_owner::{run_daemon_owner, BootstrapDaemonChild};
#[cfg(not(windows))]
pub use daemon_owner_unsupported::{
    await_bootstrap_daemon_publication, run_daemon_owner, BootstrapDaemonChild,
};

pub const fn daemon_owner_supported() -> bool {
    cfg!(windows)
}

pub enum BootstrapDaemonStatus {
    Pending,
    Ready,
    Exited(std::process::ExitStatus),
}

/// A lost response is resolved by the original ticket or the retained
/// daemon's final authenticated endpoint, never by a replacement process.
pub enum BootstrapDaemonCommit {
    Committed,
    Unknown,
}
