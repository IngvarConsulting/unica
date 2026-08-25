// Hidden v0.13 address policy remains unreachable from the package-selected v0.12 surface.
#[allow(dead_code)]
pub(crate) mod address;
pub mod cache;
pub mod cancellation;
pub mod code_intelligence;
pub mod diagnostics;
pub mod documentation;
pub mod engine;
pub mod events;
pub mod form_edit;
pub mod format_profile;
// This seam is intentionally dormant while production remains on v0.12.
#[allow(dead_code)]
pub(crate) mod invocation;
pub mod long_work;
// Hidden v0.13 module projection remains unreachable from the package-selected v0.12 surface.
pub(crate) mod metadata;
#[allow(dead_code)]
pub(crate) mod module_projection;
pub(crate) mod node_view;
pub mod operational_config;
#[allow(dead_code)]
pub(crate) mod platform_profile;
pub mod progress;
#[allow(dead_code)]
pub(crate) mod project_health;
pub mod project_sources;
pub mod role;
pub mod source_location;
pub mod source_resources;
pub mod source_revision;
pub mod source_roots;
pub mod source_target;
pub mod subsystem;
pub mod support_state;
pub mod workspace;
