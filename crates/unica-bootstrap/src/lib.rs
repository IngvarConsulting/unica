//! Native bootstrap for the thin Unica plugin.

mod archive;
mod attempt;
mod cache;
mod download;
mod error;
mod host;
mod manifest;
mod platform;
mod verification;

pub use archive::{extract_verified_tar_gz, sha256_file, verify_runtime_files};
pub use attempt::{diagnose, AttemptLog, AttemptSubject, OpenAttempt, Stage, UnfinishedAttempt};
pub use cache::{Prefetched, RuntimeInstallation, RuntimeInstaller};
pub use download::{DownloadObserver, Downloader, HttpDownloader, SilentDownload};
pub use error::{BootstrapError, Failure, Result};
pub use host::{
    host_tool_deadline, provider_state_root, runtime_cache_root, verify_installed_plugin_metadata,
    verify_installed_skill_package,
};
pub use manifest::{
    Artifact, ArtifactRole, DeliveryForm, ReleaseIdentity, RuntimeAsset, RuntimeFile,
    RuntimeManifest, SourceIdentity, TargetRuntime, CORE_ARTIFACT,
};
pub use platform::{launch_runtime, run_platform_main, HostTarget, RuntimeHandoff};
pub use verification::verify_mcp_runtime;
