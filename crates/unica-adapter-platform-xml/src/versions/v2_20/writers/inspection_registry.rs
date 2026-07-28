use crate::application::NativeWriterResult;
use crate::domain::workspace::WorkspaceContext;
use serde_json::{Map, Value};

use super::{cf, cfe, dcs, form, interface, meta, mxl, role, subsystem, template};

pub(crate) fn invoke_read(
    operation: &str,
    tool_name: &str,
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Option<Result<NativeWriterResult, String>> {
    cf::invoke_read(operation, tool_name, args, context)
        .or_else(|| cfe::invoke_read(operation, tool_name, args, context))
        .or_else(|| meta::invoke_read(operation, tool_name, args, context))
        .or_else(|| form::invoke_read(operation, tool_name, args, context))
        .or_else(|| interface::invoke_read(operation, tool_name, args, context))
        .or_else(|| subsystem::invoke_read(operation, tool_name, args, context))
        .or_else(|| template::invoke_read(operation, tool_name, args, context))
        .or_else(|| dcs::invoke_read(operation, tool_name, args, context))
        .or_else(|| mxl::invoke_read(operation, tool_name, args, context))
        .or_else(|| role::invoke_read(operation, tool_name, args, context))
}
