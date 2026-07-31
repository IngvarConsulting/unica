use super::{cf, code, form, meta, mxl, registry, role, subsystem, NativeOperationAdapter};
use crate::{application::AdapterOutcome, domain::workspace::WorkspaceContext};
use serde::Serialize;
use serde_json::{Map, Value};

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
        if mutating {
            match registry::typed_mutation_handler(operation) {
                Some(registry::TypedMutationHandler::CodePatch) => {
                    let execution = if dry_run {
                        code::preview_with_data(args, context)
                    } else {
                        code::apply_with_data(args, context)
                    };
                    return typed_operation_result(execution.outcome, execution.data, "code patch");
                }
                Some(registry::TypedMutationHandler::FormEdit) if form::has_edit_payload(args) => {
                    let execution = if dry_run {
                        form::preview_with_data(args, context)
                    } else {
                        form::apply_with_data(args, context)
                    };
                    return typed_operation_result(execution.outcome, execution.data, "form edit");
                }
                Some(registry::TypedMutationHandler::FormEdit) => {}
                None => {}
            }
        }
        // A dry run keeps its placeholder outcome: previewing a read must not
        // perform it, even though these reads change nothing.
        if !dry_run {
            match operation {
                "meta-info" => {
                    let execution = meta::analyze_meta_info_with_data(args, context);
                    return typed_operation_result(execution.outcome, execution.data, "meta info");
                }
                "cf-info" => {
                    let execution = cf::analyze_cf_info(args, context);
                    return typed_operation_result(execution.outcome, execution.data, "cf info");
                }
                "role-info" => {
                    let execution = role::analyze_role_info(args, context);
                    return typed_operation_result(execution.outcome, execution.data, "role info");
                }
                "mxl-info" => {
                    let execution = mxl::analyze_mxl_info(args, context);
                    return typed_operation_result(execution.outcome, execution.data, "mxl info");
                }
                "subsystem-info" => {
                    let execution = subsystem::analyze_subsystem_info(args, context);
                    return typed_operation_result(
                        execution.outcome,
                        execution.data,
                        "subsystem info",
                    );
                }
                _ => {}
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

/// Serializes a handler's typed payload beside its human-readable outcome.
fn typed_operation_result<T: Serialize>(
    adapter: AdapterOutcome,
    data: Option<T>,
    operation: &str,
) -> Result<NativeOperationResult, String> {
    let data = data
        .map(serde_json::to_value)
        .transpose()
        .map_err(|error| format!("serialize typed {operation} result: {error}"))?;
    Ok(NativeOperationResult { adapter, data })
}
