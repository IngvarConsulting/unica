use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::AdapterOutcome;
use serde_json::{Map, Value};

use super::{cf, cfe, form, help, interface, meta, mxl, role, skd, subsystem, support, template};

pub(crate) fn invoke_read(
    operation: &str,
    tool_name: &str,
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Option<Result<AdapterOutcome, String>> {
    cf::invoke_read(operation, tool_name, args, context)
        .or_else(|| cfe::invoke_read(operation, tool_name, args, context))
        .or_else(|| meta::invoke_read(operation, tool_name, args, context))
        .or_else(|| form::invoke_read(operation, tool_name, args, context))
        .or_else(|| interface::invoke_read(operation, tool_name, args, context))
        .or_else(|| subsystem::invoke_read(operation, tool_name, args, context))
        .or_else(|| template::invoke_read(operation, tool_name, args, context))
        .or_else(|| skd::invoke_read(operation, tool_name, args, context))
        .or_else(|| mxl::invoke_read(operation, tool_name, args, context))
        .or_else(|| role::invoke_read(operation, tool_name, args, context))
}

pub(crate) fn invoke_mutation(
    operation: &str,
    tool_name: &str,
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Option<AdapterOutcome> {
    cf::invoke_mutation(operation, tool_name, args, context)
        .or_else(|| cfe::invoke_mutation(operation, tool_name, args, context))
        .or_else(|| meta::invoke_mutation(operation, tool_name, args, context))
        .or_else(|| match operation {
            "help-add" => Some(help::add_help(args, context)),
            _ => None,
        })
        .or_else(|| form::invoke_mutation(operation, tool_name, args, context))
        .or_else(|| interface::invoke_mutation(operation, tool_name, args, context))
        .or_else(|| subsystem::invoke_mutation(operation, tool_name, args, context))
        .or_else(|| template::invoke_mutation(operation, tool_name, args, context))
        .or_else(|| skd::invoke_mutation(operation, tool_name, args, context))
        .or_else(|| mxl::invoke_mutation(operation, tool_name, args, context))
        .or_else(|| role::invoke_mutation(operation, tool_name, args, context))
        .or_else(|| support::invoke_mutation(operation, tool_name, args, context))
}
