pub mod application_ports;

pub mod bundled_tools;
pub mod capacity_observation;
#[cfg(test)]
#[path = "../../tests/platform/code_intelligence_integration.rs"]
pub(crate) mod code_intelligence;
pub mod daemon_adapters;
pub mod diagnostics;

pub mod engine_delivery;
#[allow(dead_code)]
pub(crate) mod event_views;
pub mod family_codecs;
pub mod format_families;
#[cfg(test)]
mod format_family_tests;
pub mod format_guard;
pub mod internal_adapters;
// Hidden v0.13 routing remains unreachable from the package-selected v0.12 surface.
#[allow(dead_code)]
pub mod event_sources;
#[allow(dead_code)]
pub mod logical_tree;
pub(crate) mod metadata_kinds;
pub mod metadata_operations;
pub mod native_operations;
pub mod operational_config;
pub(crate) mod platform;
// Модуль `platform` закрыт: крейтам выше нужны только эти имена.
pub use platform::{current_target_id, TempFileSpill, VIEW_SNAPSHOT_MAX_RETAINED_BYTES};
#[cfg(any(test, feature = "test-support"))]
pub use platform::{inject_unix_waitid_error_for_test, reap_runtime_authority_test_child};
pub mod platform_xml_owner;
// This foundational provider is consumed by the public migration in the next slice.
#[allow(dead_code)]
pub(crate) mod platform_xml_source_targets;
pub mod plugin_runtime;
#[allow(dead_code)]
pub mod project_sources;
mod revision_artifact_policy;

pub mod runtime_composition;
#[cfg(any(test, feature = "test-support"))]
pub(crate) mod runtime_jobs_integration;
#[cfg(any(test, feature = "test-support"))]
mod runtime_preflight_integration;
pub mod source_roots;
// The topology provider is introduced before both public consumers migrate to it.
pub mod name_index;
#[allow(dead_code)]
pub(crate) mod support_guard;
#[allow(dead_code)]
// Hidden v0.13 typed read adapter remains unreachable from the v0.12 tool ledger.
pub mod support_state;
pub mod symbol_environment;
#[cfg(any(test, feature = "test-support"))]
pub(crate) mod test_fixtures;
pub(crate) mod tool_context;
pub mod v13_large_configuration;
#[allow(dead_code)]
pub mod v13_read;
pub mod v13_read_port;
pub mod v13_read_projection;
// Production v0.12 routing stays unchanged until the atomic v0.13 cutover.
#[allow(dead_code)]
pub mod v13_find;
pub mod workspace;
// Publication and Invocation routing consume the remaining actor seams in Task 7.
#[cfg(any(test, feature = "test-support"))]
#[path = "../../tests/platform/workspace_service_fixture.rs"]
pub mod workspace_service_fixture;
#[allow(dead_code)]
#[cfg(test)]
#[path = "../../tests/platform/workspace_services_integration.rs"]
pub mod workspace_services;

#[cfg(any(test, feature = "test-support"))]
pub(crate) static V8TR_CONFIG_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(any(test, feature = "test-support"))]
mod documentation_policy_integration;

pub mod source_diff;

pub mod source_workspace;

#[cfg(test)]
pub(crate) mod temp_test_root;

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
#[path = "source_discovery_integration_tests.rs"]
mod medium_source_discovery_integration_tests;

pub mod analysis_composition;

pub(crate) mod form_composition;

pub(crate) mod subsystem_composition;

pub(crate) mod interface_composition;

pub(crate) mod support_composition;

#[cfg(test)]
#[path = "meta_add_surface_tests.rs"]
mod medium_meta_add_surface_tests;
#[cfg(test)]
#[path = "meta_info_surface_tests.rs"]
mod medium_meta_info_surface_tests;
mod source_ids;

pub mod external_artifact_inventory;
