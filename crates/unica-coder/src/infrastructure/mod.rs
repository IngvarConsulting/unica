pub(crate) mod application_ports;
pub(crate) mod bsl_outline;
// Hidden v0.13 module projection remains unreachable from the package-selected v0.12 surface.
#[allow(dead_code)]
pub(crate) mod bsl_module_projection;
pub(crate) mod bundled_tools;
pub(crate) mod code_intelligence;
pub(crate) mod configuration_help;
pub(crate) mod daemon;
mod deadline_lock;
pub(crate) mod diagnostics;
pub(crate) mod diagnostics_jsonl;
pub(crate) mod documentation_policy;
pub(crate) mod documentation_retrieval;
pub(crate) mod engine_delivery;
#[allow(dead_code)]
pub(crate) mod event_projection;
pub(crate) mod format_guard;
pub mod internal_adapters;
pub(crate) mod kb_1ci;
// Hidden v0.13 routing remains unreachable from the package-selected v0.12 surface.
#[allow(dead_code)]
pub(crate) mod logical_event_source;
#[allow(dead_code)]
pub(crate) mod logical_tree;
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
// The v5 runtime consumes this production store in the following W0a slice.
#[allow(dead_code)]
pub(crate) mod receipt_ledger;
#[cfg(feature = "receipt-ledger-test-support")]
#[allow(dead_code)]
pub(crate) mod receipt_ledger_reachability;
#[cfg(feature = "receipt-ledger-test-support")]
#[allow(dead_code)]
pub(crate) mod receipt_ledger_test_evidence;
mod revision_artifact_policy;
pub(crate) mod rlm_navigation;
pub(crate) mod runtime_build_fallback;
pub(crate) mod runtime_build_preflight;
pub(crate) mod runtime_jobs;
pub(crate) mod source_revision;
pub(crate) mod source_roots;
mod source_selection_evidence;
// The topology provider is introduced before both public consumers migrate to it.
#[allow(dead_code)]
pub(crate) mod subsystem_topology;
pub(crate) mod support_guard;
#[allow(dead_code)]
pub(crate) mod support_policy_evidence;
// The daemon becomes this store's sole writer in the following migration slice.
#[allow(dead_code)]
pub(crate) mod task_store;
// The isolated protocol-v5 TaskStore remains unreachable from production v0.12 composition.
#[allow(dead_code)]
pub(crate) mod task_store_v5;
// Hidden v0.13 typed read adapter remains unreachable from the v0.12 tool ledger.
#[allow(dead_code)]
pub(crate) mod v13_read;
#[allow(dead_code)]
pub(crate) mod v13_read_port;
mod v13_read_projection;
// Production v0.12 routing stays unchanged until the atomic v0.13 cutover.
#[allow(dead_code)]
pub(crate) mod v13_find;
// The provider is introduced before the seven subject readers migrate to it.
#[allow(dead_code)]
pub(crate) mod support_state;
pub(crate) mod tool_context;
pub(crate) mod workspace;
// Publication and Invocation routing consume the remaining actor seams in Task 7.
#[allow(dead_code)]
pub(crate) mod workspace_actor;
pub(crate) mod workspace_config;
pub mod workspace_index;
pub mod workspace_services;
pub mod workspace_state;

#[cfg(test)]
pub(crate) static V8TR_CONFIG_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
