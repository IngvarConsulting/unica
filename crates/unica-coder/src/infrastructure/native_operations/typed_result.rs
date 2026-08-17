use super::{
    cf, cfe, code, external, form, mxl, registry, role, subsystem, xdto, NativeOperationAdapter,
};
use crate::{
    application::AdapterOutcome,
    domain::{
        cache::CacheReport, cancellation::CancellationToken, code_intelligence::ProviderDeadline,
        events::DomainEvent, support_state::SupportStateReader, workspace::WorkspaceContext,
    },
};
use serde::Serialize;
use serde_json::{Map, Value};

pub(crate) struct NativeOperationResult {
    pub(crate) adapter: AdapterOutcome,
    pub(crate) data: Option<Value>,
    pub(crate) events: Vec<DomainEvent>,
    pub(crate) recorded_cache: Option<CacheReport>,
}

#[derive(Clone, Copy)]
pub(crate) struct NativeInvocationContext<'a> {
    pub(crate) support_reader: &'a dyn SupportStateReader,
    pub(crate) cancellation: &'a CancellationToken,
    pub(crate) deadline: ProviderDeadline,
}

impl<'a> NativeInvocationContext<'a> {
    pub(crate) fn new(
        support_reader: &'a dyn SupportStateReader,
        cancellation: &'a CancellationToken,
        deadline: ProviderDeadline,
    ) -> Self {
        Self {
            support_reader,
            cancellation,
            deadline,
        }
    }
}

impl NativeOperationAdapter {
    pub(crate) fn prepared_subsystem_info_with_data(
        execution: subsystem::SubsystemInfoExecution,
    ) -> Result<NativeOperationResult, String> {
        typed_operation_result(execution.outcome, execution.data, "subsystem info")
    }

    pub(crate) fn invoke_with_data(
        operation: &str,
        tool_name: &str,
        args: &Map<String, Value>,
        context: &WorkspaceContext,
        dry_run: bool,
        mutating: bool,
        invocation: NativeInvocationContext<'_>,
    ) -> Result<NativeOperationResult, String> {
        if operation == "xdto-info" {
            let execution = xdto::info_with_data(args, context)?;
            return typed_operation_result(execution.outcome, execution.data, "XDTO info");
        }
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
                Some(registry::TypedMutationHandler::RoleEdit) => {
                    let execution = if dry_run {
                        role::preview_edit_with_data(args, context)
                    } else {
                        role::apply_edit_with_data(args, context)
                    };
                    return typed_operation_result_with_publication(
                        execution.outcome,
                        execution.data,
                        "role edit",
                        execution.events,
                        execution.recorded_cache,
                    );
                }
                Some(registry::TypedMutationHandler::XdtoEdit) => {
                    let execution = if dry_run {
                        xdto::preview_with_data(args, context)
                    } else {
                        xdto::apply_with_data(args, context)
                    };
                    return typed_operation_result(execution.outcome, execution.data, "XDTO edit");
                }
                None => {}
            }
            // ADR-0073: cf-init previews with the same typed data the apply
            // returns; the shared planner just skips the commit.
            if dry_run && operation == "cf-init" {
                let execution = cf::preview_configuration_scaffold_with_data(args, context);
                return typed_operation_result(execution.outcome, execution.data, "cf init");
            }
            if !dry_run {
                match operation {
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
                    "dcs-edit" => {
                        let execution = super::dcs::edit_dcs_with_data(args, context);
                        return typed_operation_result(
                            execution.outcome,
                            execution.data,
                            "dcs edit",
                        );
                    }
                    "form-add" => {
                        let execution = form::add_form_with_data(args, context);
                        return typed_operation_result(
                            execution.outcome,
                            execution.data,
                            "form add",
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
                    "form-remove" => {
                        let execution = form::remove_form_with_data(args, context);
                        return typed_operation_result(
                            execution.outcome,
                            execution.data,
                            "form remove",
                        );
                    }
                    _ => {}
                }
            }
        }
        match operation {
            "cf-info" => {
                let execution = cf::analyze_cf_info(args, context, invocation.support_reader);
                return typed_operation_result(execution.outcome, execution.data, "cf info");
            }
            "role-info" => {
                let execution = role::analyze_role_info(args, context, invocation.support_reader);
                return typed_operation_result(execution.outcome, execution.data, "role info");
            }
            "cfe-diff" => {
                let execution = cfe::diff_cfe(args, context);
                return typed_operation_result(execution.outcome, execution.data, "cfe diff");
            }
            "dcs-info" => {
                let execution = super::dcs::analyze_dcs_info_with_data(
                    args,
                    context,
                    invocation.support_reader,
                );
                return typed_operation_result(execution.outcome, execution.data, "dcs info");
            }
            "form-info" => {
                let execution =
                    form::analyze_form_info_with_data(args, context, invocation.support_reader);
                return typed_operation_result(execution.outcome, execution.data, "form info");
            }
            "mxl-info" => {
                let execution = mxl::analyze_mxl_info(args, context, invocation.support_reader);
                return typed_operation_result(execution.outcome, execution.data, "mxl info");
            }
            "subsystem-info" => {
                let execution = subsystem::analyze_subsystem_info_cancellable(
                    args,
                    context,
                    invocation.cancellation,
                    invocation.deadline,
                    invocation.support_reader,
                );
                return typed_operation_result(execution.outcome, execution.data, "subsystem info");
            }
            _ => {}
        }

        Self::invoke(operation, tool_name, args, context, dry_run, mutating).map(|adapter| {
            NativeOperationResult {
                adapter,
                data: None,
                events: Vec::new(),
                recorded_cache: None,
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
    Ok(NativeOperationResult {
        adapter,
        data,
        events: Vec::new(),
        recorded_cache: None,
    })
}

fn typed_operation_result_with_publication<T: Serialize>(
    adapter: AdapterOutcome,
    data: Option<T>,
    operation: &str,
    events: Vec<DomainEvent>,
    recorded_cache: Option<CacheReport>,
) -> Result<NativeOperationResult, String> {
    let mut result = typed_operation_result(adapter, data, operation)?;
    result.events = events;
    result.recorded_cache = recorded_cache;
    Ok(result)
}
