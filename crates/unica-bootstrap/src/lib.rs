//! Native bootstrap for the thin Unica plugin.

mod host;
mod platform;
mod verification;

pub use host::{
    capture_host_workspace_context, find_workspace_root, host_response_budget, host_tool_deadline,
    host_workspace_capabilities, host_workspace_environment_keys, known_client_budgets,
    provider_state_root, request_needs_client_roots, response_budget_environment_keys,
    runtime_cache_root, source_text_tool_meta, verify_installed_plugin_metadata,
    verify_installed_skill_package, AllowedDirectory, ClientRoots, HostRequest,
    HostWorkspaceContext, ResolvedWorkspace, ResponseBudget, RootsFailure, RootsFallback,
    WorkspaceOrigin, DEFAULT_RESPONSE_BUDGET, MAX_RESPONSE_BUDGET, WORKSPACE_ARGUMENT,
};
pub use platform::{launch_runtime, run_platform_main, RuntimeHandoff};
pub use verification::verify_mcp_runtime;

#[doc(hidden)]
pub use platform::{
    await_bootstrap_daemon_publication, daemon_owner_supported, launch_plugin_runtime,
    run_daemon_owner, BootstrapDaemonChild, BootstrapDaemonCommit, BootstrapDaemonStatus,
};
