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
    FormEdit,
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TopLevelJsonInput {
    None,
    RequiredJsonPath,
    OptionalJsonPath,
    OptionalDefinitionFile,
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct NativeMutationFileInputContract {
    pub(crate) top_level: TopLevelJsonInput,
    pub(crate) secondary_at_query_files: bool,
    pub(crate) secondary_from_object_platform_xml: bool,
}

#[cfg(test)]
const NO_FILE_INPUT: NativeMutationFileInputContract = NativeMutationFileInputContract {
    top_level: TopLevelJsonInput::None,
    secondary_at_query_files: false,
    secondary_from_object_platform_xml: false,
};

/// Exhaustive audit classification of caller-selected file-backed derivation
/// inputs for every public native mutator.
///
/// A non-`None` top-level input is parsed from one exact byte snapshot and
/// bound to the writer transaction. DCS `@query-file` inputs are independently
/// snapshotted and bound only when they are actually selected. Form compilation
/// from `FromObject`/`ObjectPath` binds the selected platform XML snapshot.
///
/// Mutation targets and platform owner/provenance files are guarded separately
/// and are intentionally outside this caller-input classification.
#[cfg(test)]
pub(crate) fn native_mutation_file_input_contract(
    operation: &str,
) -> Option<NativeMutationFileInputContract> {
    let contract = match operation {
        "cf-edit" | "interface-edit" | "meta-edit" | "subsystem-edit" | "subsystem-compile" => {
            NativeMutationFileInputContract {
                top_level: TopLevelJsonInput::OptionalDefinitionFile,
                secondary_at_query_files: false,
                secondary_from_object_platform_xml: false,
            }
        }
        "dcs-compile" => NativeMutationFileInputContract {
            top_level: TopLevelJsonInput::OptionalDefinitionFile,
            secondary_at_query_files: true,
            secondary_from_object_platform_xml: false,
        },
        "form-compile" => NativeMutationFileInputContract {
            top_level: TopLevelJsonInput::OptionalJsonPath,
            secondary_at_query_files: false,
            secondary_from_object_platform_xml: true,
        },
        "form-edit" => NativeMutationFileInputContract {
            top_level: TopLevelJsonInput::OptionalJsonPath,
            secondary_at_query_files: false,
            secondary_from_object_platform_xml: false,
        },
        "meta-compile" | "mxl-compile" | "role-compile" => NativeMutationFileInputContract {
            top_level: TopLevelJsonInput::RequiredJsonPath,
            secondary_at_query_files: false,
            secondary_from_object_platform_xml: false,
        },
        "dcs-edit" => NativeMutationFileInputContract {
            top_level: TopLevelJsonInput::None,
            secondary_at_query_files: true,
            secondary_from_object_platform_xml: false,
        },
        "code-patch" | "cf-init" | "support-edit" | "cfe-borrow" | "cfe-init" | "epf-init"
        | "erf-init" | "cfe-patch-method" | "meta-remove" | "help-add" | "form-add"
        | "form-remove" | "template-add" | "template-remove" => NO_FILE_INPUT,
        _ => return None,
    };
    Some(contract)
}

pub(crate) fn typed_mutation_handler(operation: &str) -> Option<TypedMutationHandler> {
    match operation {
        "code-patch" => Some(TypedMutationHandler::CodePatch),
        "form-edit" => Some(TypedMutationHandler::FormEdit),
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
