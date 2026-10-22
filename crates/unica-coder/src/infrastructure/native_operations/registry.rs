use serde_json::{Map, Value};
use unica_execution_contract::adapter_outcome::AdapterOutcome;
use unica_source_contract::workspace::WorkspaceContext;

use super::{cf, cfe, dcs, mxl_composition, role, xdto};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TypedMutationHandler {
    CodePatch,
    FormEdit,
    RoleEdit,
    XdtoEdit,
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TopLevelJsonInput {
    None,
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
/// bound to the writer transaction. Form compilation
/// from `FromObject`/`ObjectPath` binds the selected platform XML snapshot.
///
/// Mutation targets and platform owner/provenance files are guarded separately
/// and are intentionally outside this caller-input classification.
#[cfg(test)]
pub(crate) fn native_mutation_file_input_contract(
    operation: &str,
) -> Option<NativeMutationFileInputContract> {
    let contract = match operation {
        "cf-edit" | "interface-edit" | "subsystem-edit" => NativeMutationFileInputContract {
            top_level: TopLevelJsonInput::OptionalDefinitionFile,
            secondary_at_query_files: false,
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
        "code-patch" | "xdto-edit" | "support-edit" | "cfe-borrow" | "form-add" | "form-remove"
        | "role-edit" => NO_FILE_INPUT,
        _ => return None,
    };
    Some(contract)
}

pub(crate) fn typed_mutation_handler(operation: &str) -> Option<TypedMutationHandler> {
    match operation {
        "code-patch" => Some(TypedMutationHandler::CodePatch),
        "form-edit" => Some(TypedMutationHandler::FormEdit),
        "role-edit" => Some(TypedMutationHandler::RoleEdit),
        "xdto-edit" => Some(TypedMutationHandler::XdtoEdit),
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
        .or_else(|| {
            crate::infrastructure::form_composition::invoke_read(
                operation, tool_name, args, context,
            )
        })
        .or_else(|| {
            crate::infrastructure::interface_composition::invoke_read(
                operation, tool_name, args, context,
            )
        })
        .or_else(|| {
            crate::infrastructure::subsystem_composition::invoke_read(
                operation, tool_name, args, context,
            )
        })
        .or_else(|| dcs::invoke_read(operation, tool_name, args, context))
        .or_else(|| mxl_composition::invoke_read(operation, tool_name, args, context))
        .or_else(|| role::invoke_read(operation, tool_name, args, context))
        .or_else(|| xdto::invoke_read(operation, tool_name, args, context))
}

pub(crate) enum PreviewInvocation {
    Planned(Result<AdapterOutcome, String>),
}

pub(crate) fn invoke_preview(
    operation: &str,
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Option<PreviewInvocation> {
    if operation != "form-compile" || !unica_forms_designer::codec::has_compile_payload(args) {
        return None;
    }
    Some(PreviewInvocation::Planned(
        crate::infrastructure::form_composition::preview_form_compile(args, context),
    ))
}

pub(crate) fn invoke_mutation(
    operation: &str,
    tool_name: &str,
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Option<AdapterOutcome> {
    cf::invoke_mutation(operation, tool_name, args, context)
        .or_else(|| cfe::invoke_mutation(operation, tool_name, args, context))
        .or_else(|| {
            crate::infrastructure::form_composition::invoke_mutation(
                operation, tool_name, args, context,
            )
        })
        .or_else(|| {
            crate::infrastructure::interface_composition::invoke_mutation(
                operation, tool_name, args, context,
            )
        })
        .or_else(|| {
            crate::infrastructure::subsystem_composition::invoke_mutation(
                operation, tool_name, args, context,
            )
        })
        .or_else(|| {
            crate::infrastructure::support_composition::invoke_mutation(
                operation, tool_name, args, context,
            )
        })
}

#[cfg(test)]
mod tests {
    use super::{
        invoke_mutation, native_mutation_file_input_contract, typed_mutation_handler,
        NativeMutationFileInputContract, TopLevelJsonInput,
    };
    use crate::application::{tools, ToolHandler};
    use serde_json::Map;
    use std::collections::BTreeMap;
    use unica_source_contract::workspace::WorkspaceContext;

    #[test]
    fn mutating_native_tools_have_registered_mutation_handlers() {
        let args = Map::new();
        for tool in tools() {
            if !tool.execution.is_mutating() {
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

    #[test]
    fn every_native_mutator_has_an_explicit_file_input_contract() {
        let expected_file_backed = BTreeMap::from([
            (
                "cf-edit",
                (TopLevelJsonInput::OptionalDefinitionFile, false, false),
            ),
            (
                "form-compile",
                (TopLevelJsonInput::OptionalJsonPath, false, true),
            ),
            (
                "form-edit",
                (TopLevelJsonInput::OptionalJsonPath, false, false),
            ),
            (
                "interface-edit",
                (TopLevelJsonInput::OptionalDefinitionFile, false, false),
            ),
            (
                "subsystem-edit",
                (TopLevelJsonInput::OptionalDefinitionFile, false, false),
            ),
        ]);
        let mut actual_file_backed = BTreeMap::new();

        for tool in tools()
            .into_iter()
            .filter(|tool| tool.execution.is_mutating())
        {
            let ToolHandler::NativeOperation { operation, .. } = tool.handler else {
                continue;
            };
            let contract = native_mutation_file_input_contract(operation).unwrap_or_else(|| {
                panic!(
                    "{} native mutator `{operation}` lacks a file-input audit classification",
                    tool.name
                )
            });
            if contract.top_level != TopLevelJsonInput::None
                || contract.secondary_at_query_files
                || contract.secondary_from_object_platform_xml
            {
                actual_file_backed.insert(
                    operation,
                    (
                        contract.top_level,
                        contract.secondary_at_query_files,
                        contract.secondary_from_object_platform_xml,
                    ),
                );
            }
        }

        assert_eq!(actual_file_backed, expected_file_backed);
        assert_eq!(actual_file_backed.len(), 5);
        assert_eq!(
            actual_file_backed
                .values()
                .map(|(top_level, at_query_files, from_object_platform_xml)| {
                    usize::from(*top_level != TopLevelJsonInput::None)
                        + usize::from(*at_query_files)
                        + usize::from(*from_object_platform_xml)
                })
                .sum::<usize>(),
            6
        );
        assert_eq!(native_mutation_file_input_contract("unknown-mutator"), None);
        assert_eq!(
            native_mutation_file_input_contract("form-add"),
            Some(NativeMutationFileInputContract {
                top_level: TopLevelJsonInput::None,
                secondary_at_query_files: false,
                secondary_from_object_platform_xml: false,
            })
        );
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
