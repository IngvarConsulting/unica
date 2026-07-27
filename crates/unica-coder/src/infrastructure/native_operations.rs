//! Thin facade over family-owned native XML/DSL operations.
pub(crate) mod code;
pub(crate) mod common;
pub(crate) mod compile_transaction;
pub(crate) mod meta;
pub(crate) mod registry;
pub(crate) mod single_file_publisher;
pub(crate) mod typed_result;

use crate::{application::AdapterOutcome, domain::workspace::WorkspaceContext};
use serde_json::{Map, Value};

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
        let form_edit_without_payload_preview =
            operation == "form-edit" && dry_run && !registry::has_form_edit_payload(args);
        if registry::typed_mutation_handler(operation).is_some()
            && !form_edit_without_payload_preview
        {
            return Err(format!(
                "{operation} requires the typed native-operation result path"
            ));
        }
        if dry_run {
            if let Some(outcome) = registry::invoke_adapter_writer(
                operation,
                tool_name,
                args,
                context,
                unica_format_core::commands::MutationMode::Preview,
            ) {
                return Ok(outcome);
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

        Err(format!(
            "native read handler is not registered for {tool_name} operation `{operation}`"
        ))
    }
}
#[cfg(test)]
mod tests;
