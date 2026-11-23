#[cfg(windows)]
mod daemon_gate;
#[cfg(windows)]
mod daemon_owner;
mod entrypoint;
mod filesystem;
#[cfg(windows)]
mod job_object;
#[cfg(windows)]
mod pending_daemon;
mod process;

pub use entrypoint::run_platform_main;
pub(crate) use filesystem::file_uri_host_names_share;
pub(crate) use process::runtime_command;
pub use process::{launch_runtime, RuntimeHandoff};

#[cfg(windows)]
pub use daemon_gate::await_bootstrap_daemon_publication;
#[cfg(windows)]
pub use daemon_owner::{
    daemon_owner_enabled, launch_runtime_with_daemon_owner, run_daemon_owner, BootstrapDaemonChild,
    BootstrapDaemonCommit, BootstrapDaemonStatus,
};
