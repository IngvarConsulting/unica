//! Native bootstrap for the thin Unica Codex plugin.

mod archive;
mod cache;
mod codex;
mod download;
mod error;
mod manifest;
mod migration;
mod process;
mod target;
mod verification;

pub use archive::{extract_verified_tar_gz, sha256_file, verify_runtime_files};
pub use cache::{RuntimeInstallation, RuntimeInstaller};
pub use codex::{
    discover, CodexDiscovery, CommandRunner, CommandSpec, MarketplaceList, MarketplaceRecord,
    MarketplaceSource, PluginList, PluginRecord, SystemCommandRunner,
};
pub use download::{Downloader, HttpDownloader};
pub use error::{BootstrapError, Result};
pub use manifest::{
    ReleaseIdentity, RuntimeAsset, RuntimeFile, RuntimeManifest, SourceIdentity, TargetRuntime,
};
pub use migration::{
    classify_discovery, MigrationEngine, MigrationPlan, MigrationReport, CANONICAL_MARKETPLACE,
    CANONICAL_REF, CANONICAL_SOURCE,
};
pub use process::launch_runtime;
pub use target::HostTarget;
pub use verification::verify_mcp_runtime;
