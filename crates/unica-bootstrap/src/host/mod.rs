//! Everything the bootstrap knows about the coding hosts that install the
//! plugin. Names of hosts, of their manifest directories and of their
//! environment variables live behind this facade; call sites stay host-neutral.

mod descriptor;
mod plugin_manifest;
mod runtime_cache;

pub use plugin_manifest::verify_installed_plugin_metadata;
pub use runtime_cache::{provider_state_root, runtime_cache_root};
