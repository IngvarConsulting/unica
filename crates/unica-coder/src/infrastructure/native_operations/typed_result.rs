use super::{code, meta, registry, NativeOperationAdapter};
use crate::{application::AdapterOutcome, domain::workspace::WorkspaceContext};
use serde_json::{json, Map, Value};

pub(crate) struct NativeOperationResult {
    pub(crate) adapter: AdapterOutcome,
    pub(crate) data: Option<Value>,
}

impl NativeOperationAdapter {
    pub(crate) fn invoke_with_data(
        operation: &str,
        tool_name: &str,
        args: &Map<String, Value>,
        context: &WorkspaceContext,
        dry_run: bool,
        mutating: bool,
    ) -> Result<NativeOperationResult, String> {
        if !mutating && !dry_run && operation == "meta-info" {
            let execution = meta::analyze_meta_info_with_navigation(args, context);
            let data = execution
                .navigation
                .map(|navigation| json!({ "navigation": navigation }));
            return Ok(NativeOperationResult {
                adapter: execution.outcome,
                data,
            });
        }

        if mutating {
            let execution = match registry::typed_mutation_handler(operation) {
                Some(registry::TypedMutationHandler::CodePatch) if dry_run => {
                    Some(code::preview_with_data(args, context))
                }
                Some(registry::TypedMutationHandler::CodePatch) => {
                    Some(code::apply_with_data(args, context))
                }
                None => None,
            };
            if let Some(execution) = execution {
                let data = execution
                    .data
                    .map(serde_json::to_value)
                    .transpose()
                    .map_err(|error| format!("serialize typed code patch result: {error}"))?;
                return Ok(NativeOperationResult {
                    adapter: execution.outcome,
                    data,
                });
            }
        }

        Self::invoke(operation, tool_name, args, context, dry_run, mutating).map(|adapter| {
            NativeOperationResult {
                adapter,
                data: None,
            }
        })
    }
}
