//! Native bootstrap for the thin Unica Codex plugin.

mod archive;
mod cache;
mod download;
mod error;
mod manifest;
mod platform;
mod verification;

pub use archive::{extract_verified_tar_gz, sha256_file, verify_runtime_files};
pub use cache::{RuntimeInstallation, RuntimeInstaller};
pub use download::{Downloader, HttpDownloader};
pub use error::{BootstrapError, Result};
pub use manifest::{
    ReleaseIdentity, RuntimeAsset, RuntimeFile, RuntimeManifest, SourceIdentity, TargetRuntime,
};
pub use platform::{launch_runtime, run_platform_main, HostTarget};
pub use verification::verify_mcp_runtime;
