//! Native bootstrap for the thin Unica plugin.

mod archive;
mod attempt;
mod cache;
mod download;
mod error;
mod host;
mod manifest;
pub mod network;
mod platform;
mod verification;

pub use archive::{
    extract_verified_tar_gz, extract_verified_zip, sha256_file, verify_runtime_files,
};
pub use attempt::{diagnose, AttemptLog, AttemptSubject, OpenAttempt, Stage, UnfinishedAttempt};
pub use cache::{Prefetched, RuntimeInstallation, RuntimeInstaller};
pub use download::{DownloadObserver, Downloader, HttpDownloader, SilentDownload};
pub use error::{BootstrapError, Failure, Result};
pub use host::{
    capture_host_workspace_context, host_response_budget, host_tool_deadline,
    host_workspace_capabilities, host_workspace_environment_keys, known_client_budgets,
    provider_state_root, request_needs_client_roots, response_budget_environment_keys,
    runtime_cache_root, source_text_tool_meta, verify_installed_plugin_metadata,
    verify_installed_skill_package, ClientRoots, HostRequest, HostWorkspaceContext,
    ResolvedWorkspace, ResponseBudget, RootsFailure, RootsFallback, WorkspaceOrigin,
    DEFAULT_RESPONSE_BUDGET, MAX_RESPONSE_BUDGET,
};
pub use manifest::{
    Artifact, ArtifactRole, DeliveryForm, ReleaseIdentity, RuntimeAsset, RuntimeFile,
    RuntimeManifest, SourceIdentity, TargetRuntime, CORE_ARTIFACT,
};
pub use platform::{launch_runtime, run_platform_main, HostTarget, RuntimeHandoff};
pub use verification::verify_mcp_runtime;
