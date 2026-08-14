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
    cancel_runtime_job_process_tree, configure_runtime_job_command, ensure_truncation_diagnostics,
    ManagedChild, ManagedCommand, ManagedLineOutput, ManagedOutput, ManagedStartupChild,
    StreamControl, STDERR_CAPTURE_LIMIT, STDOUT_CAPTURE_LIMIT,
};
pub(crate) use target::current_target_id;
