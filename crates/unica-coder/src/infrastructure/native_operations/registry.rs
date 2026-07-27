use crate::application::AdapterOutcome;
use crate::domain::workspace::WorkspaceContext;
use serde_json::{Map, Value};
use std::path::PathBuf;
use unica_format_core::{
    commands::{
        ConfigurationCommand, ConfigurationInspection, DataCompositionCommand,
        DataCompositionInspection, ExtensionCommand, ExtensionInspection, ExternalArtifactCommand,
        FormCommand, FormInspection, HelpCommand, InspectionCommand, InspectionRequest,
        InterfaceCommand, InterfaceInspection, MetadataCommand, MetadataInspection, MutationMode,
        RoleCommand, RoleInspection, SpreadsheetCommand, SpreadsheetInspection, SubsystemCommand,
        SubsystemInspection, SupportCommand, TemplateCommand, TemplateInspection, WriterArgument,
        WriterArguments, WriterBorrowScope, WriterCommand, WriterEvidence, WriterSourceRole,
        WriterStatus,
    },
    ports::{OperationCancellation, WriterRequest},
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
    Some(Ok(invoke_adapter_inspection(
        operation, tool_name, args, context,
    )?))
}

pub(crate) fn invoke_mutation(
    operation: &str,
    tool_name: &str,
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Option<AdapterOutcome> {
    invoke_adapter_writer(operation, tool_name, args, context, MutationMode::Apply)
}

pub(crate) fn invoke_adapter_writer(
    operation: &str,
    tool_name: &str,
    args: &Map<String, Value>,
    context: &WorkspaceContext,
    mode: MutationMode,
) -> Option<AdapterOutcome> {
    invoke_adapter_writer_with_evidence(operation, tool_name, args, context, mode)
        .map(|(outcome, _)| outcome)
}

pub(crate) fn invoke_adapter_writer_with_evidence(
    operation: &str,
    tool_name: &str,
    args: &Map<String, Value>,
    context: &WorkspaceContext,
    mode: MutationMode,
) -> Option<(AdapterOutcome, Option<WriterEvidence>)> {
    let command = match writer_command(operation, args) {
        Ok(Some(command)) => command,
        Ok(None) => return None,
        Err(error) => return Some((writer_failure(tool_name, error), None)),
    };
    let sources = writer_sources(operation, args);
    let inline_definition = match writer_inline_definition(operation, args) {
        Ok(definition) => definition,
        Err(error) => return Some((writer_failure(tool_name, error), None)),
    };
    let adapter_hint =
        first_string(args, &["compatibilityMode", "CompatibilityMode"]).map(ToOwned::to_owned);
    let factory = unica_adapter_platform_xml::PlatformXmlAdapterFactory::new();
    let session = match factory.capture_writer_session(
        sources,
        inline_definition,
        adapter_hint,
        &context.workspace_root,
        &context.cwd,
        &context.cache_root,
        context.workspace_epoch,
    ) {
        Ok(session) => session,
        Err(error) => return Some((writer_failure(tool_name, error.message), None)),
    };
    let request = WriterRequest::new(session, command, mode, OperationCancellation::new());
    let result = factory
        .operational_registration()
        .writer()
        .execute(&request);
    Some(match result {
        Ok(result) => {
            let evidence = result.evidence().cloned();
            (
                AdapterOutcome {
                    ok: matches!(
                        result.status(),
                        WriterStatus::Applied | WriterStatus::Previewed
                    ),
                    summary: result.summary().to_string(),
                    changes: result.changes().to_vec(),
                    warnings: result.warnings().to_vec(),
                    errors: result.errors().to_vec(),
                    artifacts: result.artifacts().to_vec(),
                    stdout: result.stdout().map(str::to_string),
                    stderr: result.stderr().map(str::to_string),
                    command: None,
                },
                evidence,
            )
        }
        Err(error) => (writer_failure(tool_name, error.message), None),
    })
}

fn writer_failure(tool_name: &str, message: String) -> AdapterOutcome {
    AdapterOutcome {
        ok: false,
        summary: format!("{tool_name} failed"),
        changes: Vec::new(),
        warnings: Vec::new(),
        errors: vec![message.clone()],
        artifacts: Vec::new(),
        stdout: None,
        stderr: Some(format!("{message}\n")),
        command: None,
    }
}

pub(crate) fn has_form_edit_payload(args: &Map<String, Value>) -> bool {
    const KEYS: &[&str] = &[
        "FormPath",
        "formPath",
        "Path",
        "path",
        "JsonPath",
        "jsonPath",
        "definition",
    ];
    args.keys().any(|key| KEYS.contains(&key.as_str()))
}

fn invoke_adapter_inspection(
    operation: &str,
    tool_name: &str,
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Option<AdapterOutcome> {
    let command = inspection_command(operation)?;
    let factory = unica_adapter_platform_xml::PlatformXmlAdapterFactory::new();
    let session = factory.capture_inspection_session(
        operation,
        tool_name,
        args,
        &context.workspace_root,
        &context.cwd,
        &context.cache_root,
        context.workspace_epoch,
    );
    let request = InspectionRequest::new(session, command, OperationCancellation::new());
    Some(match factory.inspection_port().inspect(&request) {
        Ok(result) => AdapterOutcome {
            ok: !matches!(
                result.status(),
                WriterStatus::Rejected | WriterStatus::Cancelled | WriterStatus::RecoveryRequired
            ),
            summary: result.summary().to_string(),
            changes: result.changes().to_vec(),
            warnings: result.warnings().to_vec(),
            errors: result.errors().to_vec(),
            artifacts: result.artifacts().to_vec(),
            stdout: result.stdout().map(str::to_string),
            stderr: result.stderr().map(str::to_string),
            command: None,
        },
        Err(error) => AdapterOutcome {
            ok: false,
            summary: format!("{tool_name} failed"),
            changes: Vec::new(),
            warnings: Vec::new(),
            errors: vec![error.message.clone()],
            artifacts: Vec::new(),
            stdout: None,
            stderr: Some(format!("{}\n", error.message)),
            command: None,
        },
    })
}

fn inspection_command(operation: &str) -> Option<InspectionCommand> {
    Some(match operation {
        "cf-info" => InspectionCommand::Configuration(ConfigurationInspection::Describe),
        "cf-validate" => InspectionCommand::Configuration(ConfigurationInspection::Validate),
        "cfe-diff" => InspectionCommand::Extension(ExtensionInspection::Compare),
        "cfe-validate" => InspectionCommand::Extension(ExtensionInspection::Validate),
        "meta-info" => InspectionCommand::Metadata(MetadataInspection::Describe),
        "meta-validate" => InspectionCommand::Metadata(MetadataInspection::Validate),
        "form-info" => InspectionCommand::Form(FormInspection::Describe),
        "form-validate" => InspectionCommand::Form(FormInspection::Validate),
        "interface-validate" => InspectionCommand::Interface(InterfaceInspection::Validate),
        "subsystem-info" => InspectionCommand::Subsystem(SubsystemInspection::Describe),
        "subsystem-validate" => InspectionCommand::Subsystem(SubsystemInspection::Validate),
        "template-info" => InspectionCommand::Template(TemplateInspection::Describe),
        "template-validate" => InspectionCommand::Template(TemplateInspection::Validate),
        "dcs-info" => InspectionCommand::DataComposition(DataCompositionInspection::Describe),
        "dcs-validate" => InspectionCommand::DataComposition(DataCompositionInspection::Validate),
        "mxl-decompile" => InspectionCommand::Spreadsheet(SpreadsheetInspection::Decompile),
        "mxl-info" => InspectionCommand::Spreadsheet(SpreadsheetInspection::Describe),
        "mxl-validate" => InspectionCommand::Spreadsheet(SpreadsheetInspection::Validate),
        "role-info" => InspectionCommand::Role(RoleInspection::Describe),
        "role-validate" => InspectionCommand::Role(RoleInspection::Validate),
        _ => return None,
    })
}

fn writer_command(
    operation: &str,
    args: &Map<String, Value>,
) -> Result<Option<WriterCommand>, String> {
    let command = match operation {
        "cf-init" => WriterCommand::configuration(ConfigurationCommand::Initialize),
        "cf-edit" => WriterCommand::configuration(ConfigurationCommand::Edit),
        "cfe-init" => WriterCommand::extension(ExtensionCommand::Initialize),
        "cfe-borrow" => WriterCommand::extension(ExtensionCommand::Borrow),
        "cfe-patch-method" => WriterCommand::extension(ExtensionCommand::PatchMethod),
        "epf-init" => {
            WriterCommand::external_artifact(ExternalArtifactCommand::InitializeProcessor)
        }
        "erf-init" => WriterCommand::external_artifact(ExternalArtifactCommand::InitializeReport),
        "meta-compile" => WriterCommand::metadata(MetadataCommand::Create),
        "meta-edit" => WriterCommand::metadata(MetadataCommand::Edit),
        "meta-remove" => WriterCommand::metadata(MetadataCommand::Remove),
        "form-add" => WriterCommand::form(FormCommand::Create),
        "form-compile" => WriterCommand::form(FormCommand::Compile),
        "form-edit" => WriterCommand::form(FormCommand::Edit),
        "form-remove" => WriterCommand::form(FormCommand::Remove),
        "template-add" => WriterCommand::template(TemplateCommand::Create),
        "template-remove" => WriterCommand::template(TemplateCommand::Remove),
        "help-add" => WriterCommand::help(HelpCommand::Create),
        "interface-edit" => WriterCommand::interface(InterfaceCommand::Edit),
        "role-compile" => WriterCommand::role(RoleCommand::Create),
        "subsystem-compile" => WriterCommand::subsystem(SubsystemCommand::Create),
        "subsystem-edit" => WriterCommand::subsystem(SubsystemCommand::Edit),
        "support-edit" => WriterCommand::support(SupportCommand::Edit),
        "dcs-compile" => WriterCommand::data_composition(DataCompositionCommand::Create),
        "dcs-edit" => WriterCommand::data_composition(DataCompositionCommand::Edit),
        "mxl-compile" => WriterCommand::spreadsheet(SpreadsheetCommand::Create),
        _ => return Ok(None),
    };
    Ok(Some(
        command.with_arguments(writer_arguments(operation, args)?),
    ))
}

fn writer_arguments(operation: &str, args: &Map<String, Value>) -> Result<WriterArguments, String> {
    let mut output = Vec::new();
    push_text(args, &["name", "Name"], WriterArgument::Name, &mut output);
    push_text(
        args,
        &["synonym", "Synonym"],
        WriterArgument::Synonym,
        &mut output,
    );
    push_text(
        args,
        &["vendor", "Vendor"],
        WriterArgument::Vendor,
        &mut output,
    );
    push_text(
        args,
        &["version", "Version"],
        WriterArgument::ArtifactVersion,
        &mut output,
    );
    push_text(
        args,
        &["purpose", "Purpose"],
        WriterArgument::Purpose,
        &mut output,
    );
    push_text(args, &["mode", "Mode"], WriterArgument::Mode, &mut output);
    push_text(
        args,
        &["object", "Object"],
        WriterArgument::ObjectReference,
        &mut output,
    );
    push_text(
        args,
        &["namePrefix", "NamePrefix"],
        WriterArgument::NamePrefix,
        &mut output,
    );
    push_text(
        args,
        &["modulePath", "ModulePath"],
        WriterArgument::ModuleReference,
        &mut output,
    );
    push_text(
        args,
        &["methodName", "MethodName"],
        WriterArgument::MethodName,
        &mut output,
    );
    push_text(
        args,
        &["interceptorType", "InterceptorType"],
        WriterArgument::InterceptorType,
        &mut output,
    );
    push_text(
        args,
        &["context", "Context"],
        WriterArgument::ExecutionContext,
        &mut output,
    );
    if operation != "help-add" {
        push_text(
            args,
            &["objectName", "ObjectName"],
            WriterArgument::ObjectName,
            &mut output,
        );
    }
    push_text(
        args,
        &["formName", "FormName"],
        WriterArgument::FormName,
        &mut output,
    );
    push_text(
        args,
        &["templateName", "TemplateName"],
        WriterArgument::TemplateName,
        &mut output,
    );
    push_text(
        args,
        &["templateType", "TemplateType"],
        WriterArgument::TemplateType,
        &mut output,
    );
    push_text(
        args,
        &["lang", "Lang", "language", "Language"],
        WriterArgument::Language,
        &mut output,
    );
    push_text(
        args,
        &["operation", "Operation"],
        WriterArgument::MutationVerb,
        &mut output,
    );
    if !matches!(operation, "subsystem-compile" | "dcs-compile") {
        push_text(
            args,
            &["value", "Value"],
            WriterArgument::MutationValue,
            &mut output,
        );
    }
    push_text(
        args,
        &["dataSet", "DataSet"],
        WriterArgument::DataSet,
        &mut output,
    );
    push_text(
        args,
        &["variant", "Variant"],
        WriterArgument::Variant,
        &mut output,
    );
    if operation != "help-add" {
        push_text(
            args,
            &["processorName", "ProcessorName"],
            WriterArgument::ProcessorName,
            &mut output,
        );
    }
    push_text(
        args,
        &["capability", "Capability"],
        WriterArgument::SupportCapability,
        &mut output,
    );
    push_text(
        args,
        &["set", "Set"],
        WriterArgument::SupportRule,
        &mut output,
    );

    push_bool(
        args,
        &["noRole", "NoRole"],
        WriterArgument::OmitRole,
        &mut output,
    );
    push_bool(
        args,
        &["isFunction", "IsFunction"],
        WriterArgument::Function,
        &mut output,
    );
    push_bool(
        args,
        &["setDefault", "SetDefault"],
        WriterArgument::AssignDefaultForm,
        &mut output,
    );
    push_bool(
        args,
        &["setMainSKD", "SetMainSKD"],
        WriterArgument::AssignMainDataComposition,
        &mut output,
    );
    push_bool(
        args,
        &["noValidate", "NoValidate"],
        WriterArgument::SkipValidation,
        &mut output,
    );
    push_bool(
        args,
        &["noSelection", "NoSelection"],
        WriterArgument::ExcludeSelection,
        &mut output,
    );
    push_bool(
        args,
        &["createIfMissing", "CreateIfMissing"],
        WriterArgument::CreateIfMissing,
        &mut output,
    );
    push_bool(
        args,
        &["force", "Force"],
        WriterArgument::Force,
        &mut output,
    );
    push_bool(
        args,
        &["keepFiles", "KeepFiles"],
        WriterArgument::KeepFiles,
        &mut output,
    );
    push_bool(
        args,
        &["fromObject", "FromObject"],
        WriterArgument::DeriveFromObject,
        &mut output,
    );

    if let Some(value) = first_value(args, &["borrowMainAttribute", "BorrowMainAttribute"]) {
        let scope = match value {
            Value::Null | Value::Bool(false) => None,
            Value::Bool(true) => Some(WriterBorrowScope::Form),
            Value::String(value) if value.trim().is_empty() || value == "Form" => {
                Some(WriterBorrowScope::Form)
            }
            Value::String(value) if value == "All" => Some(WriterBorrowScope::All),
            _ => {
                return Err(
                    "-BorrowMainAttribute accepts 'Form' or 'All' (default: Form)".to_string(),
                )
            }
        };
        if let Some(scope) = scope {
            output.push(WriterArgument::BorrowMainAttribute(scope));
        }
    }

    WriterArguments::new(output).map_err(|error| error.to_string())
}

fn writer_sources(operation: &str, args: &Map<String, Value>) -> Vec<(WriterSourceRole, PathBuf)> {
    let mut sources = Vec::new();
    let mut push = |role, names: &[&str]| {
        if let Some(value) = first_string(args, names) {
            sources.push((role, PathBuf::from(value)));
        }
    };
    match operation {
        "cf-init" | "epf-init" | "erf-init" => {
            push(WriterSourceRole::DestinationDirectory, &["OutputDir"])
        }
        "cf-edit" => {
            push(WriterSourceRole::Configuration, &["ConfigPath"]);
            push(WriterSourceRole::Definition, &["DefinitionFile"]);
        }
        "support-edit" => push(WriterSourceRole::SupportTarget, &["Path"]),
        "cfe-borrow" => {
            push(WriterSourceRole::Extension, &["ExtensionPath"]);
            push(WriterSourceRole::Configuration, &["ConfigPath"]);
        }
        "cfe-init" => {
            push(WriterSourceRole::Configuration, &["ConfigPath"]);
            push(WriterSourceRole::DestinationDirectory, &["OutputDir"]);
        }
        "cfe-patch-method" => push(WriterSourceRole::Extension, &["ExtensionPath"]),
        "meta-compile" | "role-compile" => {
            push(WriterSourceRole::Definition, &["JsonPath"]);
            push(WriterSourceRole::DestinationDirectory, &["OutputDir"]);
        }
        "meta-edit" => {
            push(WriterSourceRole::Object, &["ObjectPath"]);
            push(WriterSourceRole::Definition, &["DefinitionFile"]);
        }
        "meta-remove" => push(WriterSourceRole::ConfigurationDirectory, &["ConfigDir"]),
        "form-add" => push(WriterSourceRole::Object, &["ObjectPath"]),
        "form-compile" => {
            push(WriterSourceRole::Definition, &["JsonPath"]);
            push(WriterSourceRole::Object, &["ObjectPath"]);
            push(WriterSourceRole::DestinationArtifact, &["OutputPath"]);
        }
        "form-edit" => {
            push(WriterSourceRole::Form, &["FormPath"]);
            push(WriterSourceRole::Definition, &["JsonPath"]);
        }
        "form-remove" | "template-add" | "template-remove" => {
            push(WriterSourceRole::SourceCollection, &["SrcDir"])
        }
        "help-add" => {
            push(WriterSourceRole::SourceCollection, &["SrcDir"]);
            push(
                WriterSourceRole::Object,
                &["objectName", "ObjectName", "processorName", "ProcessorName"],
            );
        }
        "interface-edit" => {
            push(WriterSourceRole::Interface, &["CIPath"]);
            push(WriterSourceRole::Definition, &["DefinitionFile"]);
        }
        "subsystem-compile" => {
            push(WriterSourceRole::DestinationDirectory, &["OutputDir"]);
            push(WriterSourceRole::ParentSubsystem, &["Parent"]);
            push(WriterSourceRole::Definition, &["DefinitionFile"]);
        }
        "subsystem-edit" => {
            push(WriterSourceRole::Subsystem, &["SubsystemPath"]);
            push(WriterSourceRole::Definition, &["DefinitionFile"]);
        }
        "dcs-compile" => {
            push(WriterSourceRole::DestinationArtifact, &["OutputPath"]);
            push(WriterSourceRole::Definition, &["DefinitionFile"]);
        }
        "dcs-edit" => {
            push(WriterSourceRole::Template, &["TemplatePath"]);
            push(WriterSourceRole::Definition, &["DefinitionFile"]);
        }
        "mxl-compile" => {
            push(WriterSourceRole::Definition, &["JsonPath"]);
            push(WriterSourceRole::DestinationArtifact, &["OutputPath"]);
        }
        _ => {}
    }
    sources
}

fn writer_inline_definition(
    operation: &str,
    args: &Map<String, Value>,
) -> Result<Option<Vec<u8>>, String> {
    if operation == "form-edit" {
        return args
            .get("definition")
            .map(serde_json::to_vec)
            .transpose()
            .map_err(|error| format!("failed to capture inline form definition: {error}"));
    }
    if matches!(operation, "subsystem-compile" | "dcs-compile") {
        return Ok(first_string(args, &["value", "Value"]).map(|value| value.as_bytes().to_vec()));
    }
    Ok(None)
}

fn push_text(
    args: &Map<String, Value>,
    names: &[&str],
    constructor: fn(String) -> WriterArgument,
    output: &mut Vec<WriterArgument>,
) {
    if let Some(value) = first_string(args, names) {
        output.push(constructor(value.to_string()));
    }
}

fn push_bool(
    args: &Map<String, Value>,
    names: &[&str],
    constructor: fn(bool) -> WriterArgument,
    output: &mut Vec<WriterArgument>,
) {
    if let Some(value) = names
        .iter()
        .find_map(|name| args.get(*name).and_then(Value::as_bool))
    {
        output.push(constructor(value));
    }
}

fn first_string<'a>(args: &'a Map<String, Value>, names: &[&str]) -> Option<&'a str> {
    first_value(args, names).and_then(Value::as_str)
}

fn first_value<'a>(args: &'a Map<String, Value>, names: &[&str]) -> Option<&'a Value> {
    names.iter().find_map(|name| args.get(*name))
}

#[cfg(test)]
mod tests {
    use super::{
        invoke_mutation, native_mutation_file_input_contract, typed_mutation_handler,
        NativeMutationFileInputContract, TopLevelJsonInput,
    };
    use crate::application::{tools, ToolHandler};
    use crate::domain::workspace::WorkspaceContext;
    use serde_json::Map;
    use std::collections::BTreeMap;

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

    #[test]
    fn every_native_mutator_has_an_explicit_file_input_contract() {
        let expected_file_backed = BTreeMap::from([
            (
                "cf-edit",
                (TopLevelJsonInput::OptionalDefinitionFile, false, false),
            ),
            (
                "dcs-compile",
                (TopLevelJsonInput::OptionalDefinitionFile, true, false),
            ),
            ("dcs-edit", (TopLevelJsonInput::None, true, false)),
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
                "meta-compile",
                (TopLevelJsonInput::RequiredJsonPath, false, false),
            ),
            (
                "meta-edit",
                (TopLevelJsonInput::OptionalDefinitionFile, false, false),
            ),
            (
                "mxl-compile",
                (TopLevelJsonInput::RequiredJsonPath, false, false),
            ),
            (
                "role-compile",
                (TopLevelJsonInput::RequiredJsonPath, false, false),
            ),
            (
                "subsystem-compile",
                (TopLevelJsonInput::OptionalDefinitionFile, false, false),
            ),
            (
                "subsystem-edit",
                (TopLevelJsonInput::OptionalDefinitionFile, false, false),
            ),
        ]);
        let mut actual_file_backed = BTreeMap::new();

        for tool in tools().into_iter().filter(|tool| tool.mutating) {
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
        assert_eq!(actual_file_backed.len(), 12);
        assert_eq!(
            actual_file_backed
                .values()
                .map(|(top_level, at_query_files, from_object_platform_xml)| {
                    usize::from(*top_level != TopLevelJsonInput::None)
                        + usize::from(*at_query_files)
                        + usize::from(*from_object_platform_xml)
                })
                .sum::<usize>(),
            14
        );
        assert_eq!(native_mutation_file_input_contract("unknown-mutator"), None);
        assert_eq!(
            native_mutation_file_input_contract("template-add"),
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
