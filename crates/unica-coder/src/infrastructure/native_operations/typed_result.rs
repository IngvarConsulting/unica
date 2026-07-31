use super::{
    cf, cfe, code, external, form, help, meta, mxl, registry, role, subsystem, template,
    NativeOperationAdapter,
};
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
            if !dry_run {
                match operation {
                    "template-add" => {
                        let execution = template::add_template_with_data(args, context);
                        return typed_operation_result(
                            execution.outcome,
                            execution.data,
                            "template add",
                        );
                    }
                    "epf-init" | "erf-init" => {
                        if let Some((outcome, data)) =
                            external::apply_with_data(operation, tool_name, args, context)
                        {
                            return typed_operation_result(outcome, data, "external init");
                        }
                    }
                    "interface-edit" => {
                        let execution = super::interface::edit_interface_with_data(args, context);
                        return typed_operation_result(
                            execution.outcome,
                            execution.data,
                            "interface edit",
                        );
                    }
                    "support-edit" => {
                        let execution = super::support::edit_support_with_data(args, context);
                        return typed_operation_result(
                            execution.outcome,
                            execution.data,
                            "support edit",
                        );
                    }
                    "subsystem-edit" => {
                        let execution = subsystem::edit_subsystem_with_data(args, context);
                        return typed_operation_result(
                            execution.outcome,
                            execution.data,
                            "subsystem edit",
                        );
                    }
                    "cfe-patch-method" => {
                        let execution = cfe::patch_extension_method_with_data(args, context);
                        return typed_operation_result(
                            execution.outcome,
                            execution.data,
                            "cfe patch method",
                        );
                    }
                    "cfe-borrow" => {
                        let execution = cfe::borrow_cfe_with_data(args, context);
                        return typed_operation_result(
                            execution.outcome,
                            execution.data,
                            "cfe borrow",
                        );
                    }
                    "cf-init" => {
                        let execution = cf::create_configuration_scaffold_with_data(args, context);
                        return typed_operation_result(
                            execution.outcome,
                            execution.data,
                            "cf init",
                        );
                    }
                    "cf-edit" => {
                        let execution = cf::edit_cf_with_data(args, context);
                        return typed_operation_result(
                            execution.outcome,
                            execution.data,
                            "cf edit",
                        );
                    }
                    "cfe-init" => {
                        let execution = cfe::create_extension_scaffold_with_data(args, context);
                        return typed_operation_result(
                            execution.outcome,
                            execution.data,
                            "cfe init",
                        );
                    }
                    // `DryRun` is the tool's own argument and is reported
                    // through `dryRun` in the data; the protocol dry run still
                    // keeps its placeholder and performs nothing.
                    "meta-remove" => {
                        let execution = meta::remove_metadata_object_with_data(args, context);
                        return typed_operation_result(
                            execution.outcome,
                            execution.data,
                            "meta remove",
                        );
                    }
                    "form-remove" => {
                        let execution = form::remove_form_with_data(args, context);
                        return typed_operation_result(
                            execution.outcome,
                            execution.data,
                            "form remove",
                        );
                    }
                    "help-add" => {
                        let execution = help::add_help_with_data(args, context);
                        return typed_operation_result(
                            execution.outcome,
                            execution.data,
                            "help add",
                        );
                    }
                    "template-remove" => {
                        let execution = template::remove_template_with_data(args, context);
                        return typed_operation_result(
                            execution.outcome,
                            execution.data,
                            "template remove",
                        );
                    }
                    _ => {}
                }
            }
            if operation == "meta-edit" {
                let execution = if dry_run {
                    meta::preview_meta_edit_with_data(args, context)
                } else {
                    meta::edit_meta_with_data(args, context)
                };
                return typed_operation_result(execution.outcome, execution.data, "meta edit");
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
                "cfe-diff" => {
                    let execution = cfe::diff_cfe(args, context);
                    return typed_operation_result(execution.outcome, execution.data, "cfe diff");
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
