mod entrypoint;
mod filesystem;
mod process;
mod target;

pub use entrypoint::run_platform_main;
pub(crate) use filesystem::set_executable;
pub use process::launch_runtime;
pub use target::HostTarget;
