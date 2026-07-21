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
pub(crate) mod meta;
pub(crate) mod mxl;
pub(crate) mod registry;
pub(crate) mod role;
pub(crate) mod single_file_publisher;
pub(crate) mod subsystem;
pub(crate) mod support;
pub(crate) mod template;
pub(crate) mod typed_result;

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
        if registry::typed_mutation_handler(operation).is_some() {
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
mod tests;
