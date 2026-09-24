//! Everything the bootstrap knows about the coding hosts that install the
//! plugin. Names of hosts, of their manifest directories and of their
//! environment variables live behind this facade; call sites stay host-neutral.

mod descriptor;
mod plugin_manifest;
mod runtime_cache;
mod skill_package;
mod tool_deadline;
mod workspace_context;

pub use plugin_manifest::verify_installed_plugin_metadata;
pub use runtime_cache::{provider_state_root, runtime_cache_root};
pub use skill_package::verify_installed_skill_package;
pub use tool_deadline::host_tool_deadline;

pub use workspace_context::{
    capture_host_workspace_context, host_workspace_capabilities, host_workspace_environment_keys,
    HostWorkspaceContext,
};
