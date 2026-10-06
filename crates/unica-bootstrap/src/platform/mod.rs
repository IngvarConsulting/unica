mod entrypoint;
mod filesystem;
mod process;
mod target;

pub use entrypoint::run_platform_main;
pub(crate) use filesystem::{file_uri_host_names_share, set_executable};
pub use process::{launch_runtime, RuntimeHandoff};
pub use target::HostTarget;
