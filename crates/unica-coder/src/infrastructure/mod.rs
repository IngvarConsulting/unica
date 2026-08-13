pub(crate) mod application_ports;
pub(crate) mod bsl_outline;
pub(crate) mod bundled_tools;
pub(crate) mod code_intelligence;
pub(crate) mod configuration_help;
pub(crate) mod diagnostics_jsonl;
pub(crate) mod documentation_policy;
pub(crate) mod documentation_retrieval;
pub(crate) mod format_guard;
pub mod internal_adapters;
pub(crate) mod kb_1ci;
pub(crate) mod metadata_kinds;
pub(crate) mod metadata_operations;
pub mod native_operations;
pub(crate) mod operational_config;
pub mod path_policy;
pub(crate) mod platform;
pub mod platform_help;
pub(crate) mod platform_xml_owner;
pub(crate) mod platform_xml_resources;
pub(crate) mod platform_xml_roots;
pub(crate) mod standards_documentation;
// This foundational provider is consumed by the public migration in the next slice.
#[allow(dead_code)]
pub(crate) mod platform_xml_source_targets;
pub mod plugin_runtime;
#[allow(dead_code)]
pub(crate) mod project_health;
pub(crate) mod project_sources;
pub(crate) mod redaction;
pub(crate) mod rlm_navigation;
pub(crate) mod runtime_jobs;
pub(crate) mod source_revision;
pub(crate) mod source_roots;
// The topology provider is introduced before both public consumers migrate to it.
#[allow(dead_code)]
pub(crate) mod subsystem_topology;
pub(crate) mod support_guard;
// The provider is introduced before the seven subject readers migrate to it.
#[allow(dead_code)]
pub(crate) mod support_state;
pub(crate) mod tool_context;
pub(crate) mod workspace;
pub(crate) mod workspace_config;
pub mod workspace_index;
pub mod workspace_services;
pub mod workspace_state;
