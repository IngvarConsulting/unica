use crate::application::AdapterOutcome;
use crate::domain::workspace::WorkspaceContext;
use serde_json::{Map, Value};
use std::path::PathBuf;

use super::{
    cf, cfe, dcs, external, form, help, interface, meta, mxl, role, subsystem, support, template,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TypedMutationHandler {
    CodePatch,
}

pub(crate) fn typed_mutation_handler(operation: &str) -> Option<TypedMutationHandler> {
    match operation {
        "code-patch" => Some(TypedMutationHandler::CodePatch),
        _ => None,
    }
}

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
        .or_else(|| dcs::invoke_read(operation, tool_name, args, context))
        .or_else(|| mxl::invoke_read(operation, tool_name, args, context))
        .or_else(|| role::invoke_read(operation, tool_name, args, context))
}

pub(crate) enum PreviewInvocation {
    Unavailable(String),
    Planned(Result<AdapterOutcome, String>),
}

pub(crate) fn invoke_preview(
    operation: &str,
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Option<PreviewInvocation> {
    if operation == "form-compile" && !form::has_compile_payload(args) {
        return None;
    }
    if !matches!(
        operation,
        "form-compile" | "meta-compile" | "role-compile" | "subsystem-compile"
    ) {
        return None;
    }
    if let Some(reason) = compile_preview_unavailable(operation, args, context) {
        return Some(PreviewInvocation::Unavailable(reason));
    }
    let planned = match operation {
        "form-compile" => form::preview_form_compile(args, context),
        "meta-compile" => meta::preview_meta_compile(args, context),
        "role-compile" => role::preview_role_compile(args, context),
        "subsystem-compile" => subsystem::preview_subsystem_compile(args, context),
        _ => unreachable!("compile preview operations were checked above"),
    };
    Some(PreviewInvocation::Planned(planned))
}

fn compile_preview_unavailable(
    operation: &str,
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Option<String> {
    if operation == "form-compile" {
        return None;
    }
    if string_arg(args, &["OutputDir", "outputDir"]).is_none() {
        return Some("missing required OutputDir argument".to_string());
    }

    match operation {
        "meta-compile" | "role-compile" => {
            let Some(json_path) = string_arg(args, &["JsonPath", "jsonPath"]) else {
                return Some("missing required JsonPath argument".to_string());
            };
            let json_path = PathBuf::from(json_path);
            let json_path = if json_path.is_absolute() {
                json_path
            } else {
                context.cwd.join(json_path)
            };
            (!json_path.is_file())
                .then(|| format!("definition file is unavailable: {}", json_path.display()))
        }
        "subsystem-compile" => {
            if let Some(parent) = string_arg(args, &["Parent", "parent"]) {
                let parent = PathBuf::from(parent);
                let parent = if parent.is_absolute() {
                    parent
                } else {
                    context.cwd.join(parent)
                };
                if !parent.exists() {
                    return Some(format!(
                        "parent subsystem is unavailable: {}",
                        parent.display()
                    ));
                }
            }
            if string_arg(args, &["Value", "value"]).is_some() {
                return None;
            }
            let Some(definition) = string_arg(args, &["DefinitionFile", "definitionFile"]) else {
                return Some("missing Value or DefinitionFile argument".to_string());
            };
            let definition = PathBuf::from(definition);
            let definition = if definition.is_absolute() {
                definition
            } else {
                context.cwd.join(definition)
            };
            (!definition.is_file())
                .then(|| format!("definition file is unavailable: {}", definition.display()))
        }
        _ => None,
    }
}

fn string_arg<'a>(args: &'a Map<String, Value>, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .find_map(|key| args.get(*key).and_then(Value::as_str))
}

pub(crate) fn invoke_mutation(
    operation: &str,
    tool_name: &str,
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Option<AdapterOutcome> {
    cf::invoke_mutation(operation, tool_name, args, context)
        .or_else(|| cfe::invoke_mutation(operation, tool_name, args, context))
        .or_else(|| external::apply(operation, tool_name, args, context))
        .or_else(|| meta::invoke_mutation(operation, tool_name, args, context))
        .or_else(|| match operation {
            "help-add" => Some(help::add_help(args, context)),
            _ => None,
        })
        .or_else(|| form::invoke_mutation(operation, tool_name, args, context))
        .or_else(|| interface::invoke_mutation(operation, tool_name, args, context))
        .or_else(|| subsystem::invoke_mutation(operation, tool_name, args, context))
        .or_else(|| template::invoke_mutation(operation, tool_name, args, context))
        .or_else(|| dcs::invoke_mutation(operation, tool_name, args, context))
        .or_else(|| mxl::invoke_mutation(operation, tool_name, args, context))
        .or_else(|| role::invoke_mutation(operation, tool_name, args, context))
        .or_else(|| support::invoke_mutation(operation, tool_name, args, context))
}

#[cfg(test)]
mod tests {
    use super::{invoke_mutation, typed_mutation_handler};
    use crate::application::{tools, ToolHandler};
    use crate::domain::workspace::WorkspaceContext;
    use serde_json::Map;

    #[test]
    fn mutating_native_tools_have_registered_mutation_handlers() {
        let args = Map::new();
        for tool in tools() {
            if !tool.mutating {
                continue;
            }
            let ToolHandler::NativeOperation { operation, .. } = tool.handler else {
                continue;
            };
            let context = mutation_probe_context(operation);
            assert!(
                invoke_mutation(operation, tool.name, &args, &context).is_some()
                    || typed_mutation_handler(operation).is_some(),
                "{} routes to native mutation operation `{}` without a registered handler",
                tool.name,
                operation
            );
        }
    }

    fn mutation_probe_context(operation: &str) -> WorkspaceContext {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "unica-mutation-probe-{operation}-{}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(root.join("src")).unwrap();
        WorkspaceContext {
            cwd: root.clone(),
            workspace_root: root.clone(),
            cache_root: root.join(".build").join("unica"),
            workspace_epoch: 1,
        }
    }
}
