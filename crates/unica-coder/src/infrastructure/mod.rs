pub(crate) mod application_ports;

pub(crate) mod bundled_tools;
pub(crate) mod capacity_observation;
#[cfg(test)]
#[path = "../../tests/platform/code_intelligence_integration.rs"]
pub(crate) mod code_intelligence;
pub(crate) mod daemon;
pub(crate) mod diagnostics;
#[cfg(feature = "receipt-ledger-test-support")]
#[path = "../../tests/support/receipt_scenario.rs"]
pub(crate) mod receipt_scenario;
#[cfg(test)]
mod receipt_wire_integration;

pub(crate) mod engine_delivery;
#[allow(dead_code)]
pub(crate) mod event_views;
pub(crate) mod format_guard;
pub mod internal_adapters;
// Hidden v0.13 routing remains unreachable from the package-selected v0.12 surface.
#[allow(dead_code)]
pub(crate) mod event_sources;
#[allow(dead_code)]
pub(crate) mod logical_tree;
pub(crate) mod metadata_kinds;
pub(crate) mod metadata_operations;
pub mod native_operations;
pub(crate) mod operational_config;
pub(crate) mod platform;
pub(crate) mod platform_xml_owner;
// This foundational provider is consumed by the public migration in the next slice.
#[allow(dead_code)]
pub(crate) mod platform_xml_source_targets;
pub mod plugin_runtime;
#[allow(dead_code)]
pub(crate) mod project_sources;
mod revision_artifact_policy;

pub(crate) mod runtime_composition;
#[cfg(test)]
pub(crate) mod runtime_jobs_integration;
#[cfg(test)]
mod runtime_preflight_integration;
pub(crate) mod source_roots;
// The topology provider is introduced before both public consumers migrate to it.
#[allow(dead_code)]
pub(crate) mod support_guard;
#[allow(dead_code)]
// Hidden v0.13 typed read adapter remains unreachable from the v0.12 tool ledger.
pub(crate) mod support_state;
pub(crate) mod tool_context;
mod v13_large_configuration;
#[allow(dead_code)]
pub(crate) mod v13_read;
pub(crate) mod v13_read_port;
mod v13_read_projection;
// Production v0.12 routing stays unchanged until the atomic v0.13 cutover.
#[allow(dead_code)]
pub(crate) mod v13_find;
pub(crate) mod workspace;
// Publication and Invocation routing consume the remaining actor seams in Task 7.
#[allow(dead_code)]
#[cfg(test)]
pub mod workspace_index;
#[cfg(test)]
#[path = "../../tests/platform/workspace_services_integration.rs"]
pub mod workspace_services;

#[cfg(test)]
pub(crate) static V8TR_CONFIG_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod documentation_policy_integration;
#[cfg(test)]
mod platform_help_oracle;

mod source_diff;

pub(crate) mod source_workspace;

#[cfg(test)]
mod workspace_state_integration_tests;

#[cfg(test)]
mod workspace_actor_contract_tests;
#[cfg(test)]
mod workspace_metadata_publication_tests;

#[cfg(test)]
#[path = "../../tests/platform/full_dump_integration.rs"]
mod full_dump_integration;

#[cfg(test)]
mod workspace_transaction_contract_tests;

#[cfg(test)]
mod source_discovery_integration_tests;

pub(crate) mod analysis_composition;

pub(crate) mod form_composition;

pub(crate) mod subsystem_composition;

pub(crate) mod interface_composition;

pub(crate) mod support_composition;

mod source_ids;
