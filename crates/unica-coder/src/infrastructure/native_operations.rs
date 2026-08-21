//! Thin facade over family-owned native XML/DSL operations.
pub(crate) mod cf;
pub(crate) mod cfe;
pub(crate) mod code;
pub(crate) mod common;
pub(crate) mod compile_transaction;
pub(crate) mod dcs;
pub(crate) mod external;
pub(crate) mod form;
pub(crate) mod form_event_registry;
pub(crate) mod help;
pub(crate) mod interface;
pub(crate) mod logical_selector;
pub(crate) mod meta;
pub(crate) mod mxl;
pub(crate) mod registry;
pub(crate) mod role;
pub(crate) mod single_file_publisher;
pub(crate) mod subsystem;
pub(crate) mod support;
pub(crate) mod template;
pub(crate) mod text_snapshot;
pub(crate) mod typed_result;
pub(crate) mod xdto;

use crate::{application::AdapterOutcome, domain::workspace::WorkspaceContext};
use serde_json::{Map, Value};
use std::fs;

pub struct NativeOperationAdapter;
impl NativeOperationAdapter {
    pub fn invoke(
        operation: &str,
        tool_name: &str,
        args: &Map<String, Value>,
        context: &WorkspaceContext,
        dry_run: bool,
        mutating: bool,
    ) -> Result<AdapterOutcome, String> {
        if operation == "subsystem-info" {
            return Err(
                "subsystem-info requires the controlled prepared invocation path".to_string(),
            );
        }
        let form_edit_without_payload_preview =
            operation == "form-edit" && dry_run && !form::has_edit_payload(args);
        if registry::typed_mutation_handler(operation).is_some()
            && !form_edit_without_payload_preview
        {
            return Err(format!(
                "{operation} requires the typed native-operation result path"
            ));
        }
        if dry_run {
            if let Some(outcome) = external::preview(operation, tool_name, args, context) {
                return Ok(outcome);
            }
            if operation == "form-edit" && form::has_edit_payload(args) {
                return Ok(form::preview_form_edit(args, context));
            }
            let mut fallback = AdapterOutcome {
                ok: true,
                summary: format!("dry run: {tool_name} would execute native XML/DSL operation"),
                changes: if mutating {
                    vec!["no files changed because dryRun is true".to_string()]
                } else {
                    Vec::new()
                },
                warnings: Vec::new(),
                errors: Vec::new(),
                artifacts: Vec::new(),
                stdout: None,
                stderr: None,
                command: None,
            };
            if let Some(preview) = registry::invoke_preview(operation, args, context) {
                return match preview {
                    registry::PreviewInvocation::Unavailable(error) => {
                        fallback.warnings.push(format!(
                            "detailed compile preview is unavailable; using safe placeholder: {error}"
                        ));
                        Ok(fallback)
                    }
                    registry::PreviewInvocation::Planned(Ok(outcome)) => Ok(outcome),
                    registry::PreviewInvocation::Planned(Err(error)) => Ok(AdapterOutcome {
                        ok: false,
                        summary: format!("dry run: {tool_name} compile planning failed"),
                        changes: Vec::new(),
                        warnings: Vec::new(),
                        errors: vec![error.clone()],
                        artifacts: Vec::new(),
                        stdout: None,
                        stderr: Some(format!("{error}\n")),
                        command: None,
                    }),
                };
            }
            return Ok(fallback);
        }

        if mutating {
            return registry::invoke_mutation(operation, tool_name, args, context).ok_or_else(|| {
                format!(
                    "native mutation handler is not registered for {tool_name} operation `{operation}`"
                )
            });
        }

        if let Some(outcome) = registry::invoke_read(operation, tool_name, args, context) {
            return outcome;
        }

        let target = common::resolve_target(operation, args, context)?;
        let text = fs::read_to_string(&target)
            .map_err(|err| format!("failed to read {}: {err}", target.display()))?;
        Ok(common::analyze_xml(operation, tool_name, &target, &text))
    }
}

#[cfg(test)]
mod source_invariant_tests {
    /// One registry-facing falsifier for response parity across the complete
    /// subject-reader bridge. Each family keeps its focused fixture below its
    /// handler; this aggregate prevents a partial family list from grounding
    /// the cross-family invariant.
    #[test]
    fn bridged_reader_outputs_are_identical_for_logical_and_physical_selectors() {
        super::cf::cf_read_selector_bridge_tests::cf_info_answers_identically_for_a_source_set_and_a_config_path();
        super::cf::cf_read_selector_bridge_tests::cf_validate_answers_identically_for_a_source_set_and_a_config_path();
        super::form::form_read_selector_bridge_tests::form_info_answers_identically_for_a_logical_and_a_physical_selector();
        super::form::form_read_selector_bridge_tests::form_validate_answers_identically_for_a_logical_and_a_physical_selector();
        super::role::role_info_typed_result_tests::role_info_answers_identically_for_a_logical_and_a_physical_selector();
        super::role::role_info_typed_result_tests::role_validate_answers_identically_for_a_logical_and_a_physical_selector();
        super::mxl::mxl_read_selector_bridge_tests::mxl_info_answers_identically_for_a_logical_and_a_physical_selector();
        super::mxl::mxl_read_selector_bridge_tests::mxl_validate_answers_identically_for_a_logical_and_a_physical_selector();
        super::mxl::mxl_read_selector_bridge_tests::mxl_decompile_answers_identically_for_a_logical_and_a_physical_selector();
        super::mxl::mxl_read_selector_bridge_tests::dcs_info_answers_identically_for_a_logical_and_a_physical_selector();
        super::mxl::mxl_read_selector_bridge_tests::dcs_validate_answers_identically_for_a_logical_and_a_physical_selector();
        super::subsystem::subsystem_read_selector_bridge_tests::subsystem_info_answers_identically_for_a_logical_and_a_physical_selector();
        super::subsystem::subsystem_read_selector_bridge_tests::subsystem_validate_answers_identically_for_a_logical_and_a_physical_selector();
    }

    /// One registry-facing falsifier for every clause of selector-free tail
    /// insertion, whose public schema and write behavior live on opposite
    /// sides of the application/infrastructure boundary.
    #[test]
    fn tail_insert_contract_is_complete() {
        crate::application::tool_contracts::tests::code_patch_schema_accepts_each_documented_selector_variant();
        super::code::tests::code_patch_without_a_selector_appends_to_the_end_and_proves_the_repeat(
        );
        super::code::tests::code_patch_writes_the_first_body_of_an_empty_or_bom_only_module();
        super::code::tests::code_patch_creates_a_module_file_the_platform_never_exported();
        super::code::tests::code_patch_refuses_a_module_role_the_metadata_kind_never_owns();
    }
}
#[cfg(test)]
mod tests;
