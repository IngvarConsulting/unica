mod entrypoint;
pub(crate) mod filesystem;
mod process;
mod target;
#[cfg(test)]
pub(crate) mod testing;

pub use entrypoint::run_platform_main;
pub(crate) use process::{
    cancel_runtime_job_process_tree, configure_runtime_job_command, ensure_truncation_diagnostics,
    ManagedChild, ManagedCommand, ManagedOutput, ManagedStartupChild,
};
#[cfg(test)]
pub(crate) use process::{
    runtime_job_process_tree_is_alive, runtime_job_process_tree_test_command,
};
pub(crate) use target::current_target_id;
