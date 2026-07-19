//! Native bootstrap for the thin Unica Codex plugin.

mod error;
mod manifest;
mod target;

pub use error::{BootstrapError, Result};
pub use manifest::{
    ReleaseIdentity, RuntimeAsset, RuntimeFile, RuntimeManifest, SourceIdentity, TargetRuntime,
};
pub use target::HostTarget;
