mod entrypoint;
mod filesystem;
#[cfg(windows)]
mod job_object;
mod process;

pub use entrypoint::run_platform_main;
pub(crate) use filesystem::file_uri_host_names_share;
pub(crate) use process::runtime_command;
#[cfg(test)]
pub(crate) use filesystem::link_directory_for_test;
pub use process::{launch_runtime, RuntimeHandoff};
