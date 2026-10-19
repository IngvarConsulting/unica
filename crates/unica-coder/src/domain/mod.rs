// Hidden v0.13 address policy remains unreachable from the package-selected v0.12 surface.
#[allow(dead_code)]
pub(crate) mod address;
// Hidden v0.13 apply foundation remains unreachable from the package-selected v0.12 surface.
#[allow(dead_code)]
pub(crate) mod apply;
pub mod cache;
pub(crate) mod call_graph_identity;
pub mod cancellation;
pub mod code_intelligence;
pub mod diagnostics;
pub mod documentation;
pub mod engine;
pub(crate) mod event_operation;
pub mod events;
// P1 model is default-compiled; P3 connects the production execution route.
#[allow(dead_code)]
pub(crate) mod execution;
pub mod form_edit;
pub(crate) mod form_operation;
pub mod format_profile;
// This seam is intentionally dormant while production remains on v0.12.
#[allow(dead_code)]
pub(crate) mod invocation;
pub mod long_work;
// Hidden v0.13 module projection remains unreachable from the package-selected v0.12 surface.
pub(crate) mod logical_route;
pub(crate) mod metadata;
pub(crate) mod metadata_operation;
#[allow(dead_code)]
pub(crate) mod module_projection;
pub(crate) mod mxl_operation;
pub(crate) mod node_view;
pub(crate) mod operation_contract;
pub(crate) mod operation_deadline;
pub mod operational_config;
#[allow(dead_code)]
pub(crate) mod platform_profile;
pub mod progress;
#[allow(dead_code)]
pub(crate) mod project_health;
pub mod project_sources;
pub mod refusal;
pub mod role;
pub(crate) mod role_operation;
pub mod source_location;
pub mod source_resources;
pub mod source_roots;
pub mod source_target;
pub mod subsystem;
pub(crate) mod support_operation;
pub mod support_state;
pub mod workspace;
