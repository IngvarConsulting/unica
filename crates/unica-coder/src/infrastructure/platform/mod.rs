pub(crate) mod contained_file;
mod entrypoint;
pub(crate) mod filesystem;
mod process;
mod target;
#[cfg(test)]
pub(crate) mod testing;
pub(crate) mod verified_directory;

pub use entrypoint::run_platform_main;
pub(crate) use process::{
    cancel_runtime_job_process_tree, configure_runtime_job_command, ensure_truncation_diagnostics,
    ManagedChild, ManagedCommand, ManagedOutput, ManagedStartupChild,
};
pub(crate) use target::current_target_id;
