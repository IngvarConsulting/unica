use super::code;
use crate::application::AdapterOutcome;
use crate::domain::workspace::WorkspaceContext;
use serde::de::Error as _;
use serde_json::{Map, Value};
use std::{fs, path::PathBuf};
use unica_format_core::{
    commands::*,
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
    cancellation: &OperationCancellation,
) -> Option<Result<AdapterOutcome, String>> {
    Some(Ok(invoke_adapter_inspection(
        operation,
        tool_name,
        args,
        context,
        cancellation,
    )?))
}

pub(crate) fn invoke_mutation(
    operation: &str,
    tool_name: &str,
    args: &Map<String, Value>,
    context: &WorkspaceContext,
    cancellation: &OperationCancellation,
) -> Option<AdapterOutcome> {
    invoke_adapter_writer(
        operation,
        tool_name,
        args,
        context,
        MutationMode::Apply,
        cancellation,
    )
}

pub(crate) fn invoke_adapter_writer(
    operation: &str,
    tool_name: &str,
    args: &Map<String, Value>,
    context: &WorkspaceContext,
    mode: MutationMode,
    cancellation: &OperationCancellation,
) -> Option<AdapterOutcome> {
    invoke_adapter_writer_with_evidence(operation, tool_name, args, context, mode, cancellation)
        .map(|(outcome, _)| outcome)
}

pub(crate) fn invoke_adapter_writer_with_evidence(
    operation: &str,
    tool_name: &str,
    args: &Map<String, Value>,
    context: &WorkspaceContext,
    mode: MutationMode,
    cancellation: &OperationCancellation,
) -> Option<(AdapterOutcome, Option<WriterEvidence>)> {
    let command = match writer_command(operation, args, context) {
        Ok(Some(command)) => command,
        Ok(None) => return None,
        Err(error) => return Some((writer_failure(tool_name, mode, error), None)),
    };
    let sources = writer_sources(operation, args);
    let factory = unica_adapter_platform_xml::PlatformXmlAdapterFactory::new();
    let session_result = if matches!(command, WriterCommand::ExtensionPatchMethod(_)) {
        factory.capture_writer_session_with_extension_emitter(
            sources,
            &context.workspace_root,
            &context.cwd,
            &context.cache_root,
            context.workspace_epoch,
            code::render_extension_method_patch,
        )
    } else {
        factory.capture_writer_session(
            sources,
            &context.workspace_root,
            &context.cwd,
            &context.cache_root,
            context.workspace_epoch,
        )
    };
    let session = match session_result {
        Ok(session) => session,
        Err(error) => return Some((writer_failure(tool_name, mode, error.message), None)),
    };
    let request = WriterRequest::new(session, command.clone(), mode, cancellation.clone());
    let result = factory
        .operational_registration()
        .writer()
        .execute(&request);
    Some(match result {
        Ok(result) => {
            let evidence = result.evidence().cloned();
            (writer_outcome(tool_name, &command, mode, &result), evidence)
        }
        Err(error) => (writer_failure(tool_name, mode, error.message), None),
    })
}

fn writer_outcome(
    tool_name: &str,
    command: &WriterCommand,
    mode: MutationMode,
    result: &unica_format_core::commands::WriterResult,
) -> AdapterOutcome {
    let ok = matches!(
        result.lifecycle(),
        WriterLifecycle::Applied | WriterLifecycle::Previewed
    );
    let summary = match result.lifecycle() {
        WriterLifecycle::Previewed => match result.message_code() {
            WriterMessageCode::NoChange => {
                format!("dry run: {tool_name} found no semantic changes")
            }
            _ => format!("dry run: {tool_name} planned semantic changes"),
        },
        WriterLifecycle::Applied => match command {
            WriterCommand::SupportEdit(_)
                if result.message_code() == WriterMessageCode::NoChange =>
            {
                "Конфигурация не на поддержке — изменений нет".to_string()
            }
            WriterCommand::SupportEdit(SupportEdit::ObjectRule(SupportObjectRule::Editable)) => {
                "Объект редактируется с сохранением поддержки".to_string()
            }
            WriterCommand::SupportEdit(SupportEdit::ObjectRule(SupportObjectRule::Locked)) => {
                "Объект установлен на замок поддержки".to_string()
            }
            WriterCommand::SupportEdit(SupportEdit::ObjectRule(SupportObjectRule::OffSupport)) => {
                "Объект снят с поддержки".to_string()
            }
            WriterCommand::SupportEdit(SupportEdit::Capability(SupportCapability::Enable)) => {
                "Возможность изменения конфигурации включена".to_string()
            }
            WriterCommand::SupportEdit(SupportEdit::Capability(SupportCapability::Disable)) => {
                "Возможность изменения конфигурации ВЫКЛЮЧЕНА".to_string()
            }
            _ => format!("{tool_name} completed"),
        },
        WriterLifecycle::Cancelled(_) => format!("{tool_name} cancelled"),
        WriterLifecycle::Rejected(_) if mode.is_preview() => {
            format!("dry run: {tool_name} failed")
        }
        WriterLifecycle::Rejected(_) => format!("{tool_name} failed"),
    };
    let changes = result
        .changes()
        .iter()
        .filter(|change| **change != SemanticChange::NoChange)
        .map(|change| match (mode, change) {
            (MutationMode::Preview, SemanticChange::SourceCreated) => {
                "would create semantic source artifact"
            }
            (MutationMode::Preview, SemanticChange::SourceUpdated) => {
                "would update semantic source artifact"
            }
            (MutationMode::Preview, SemanticChange::SourceRemoved) => {
                "would remove semantic source artifact"
            }
            (MutationMode::Preview, SemanticChange::RegistrationUpdated) => {
                "would update semantic registration"
            }
            (MutationMode::Preview, SemanticChange::SupportUpdated) => {
                "would update semantic support state"
            }
            (MutationMode::Preview, SemanticChange::ModuleUpdated) => "would update module source",
            (_, SemanticChange::SourceCreated) => "created semantic source artifact",
            (_, SemanticChange::SourceUpdated) => "updated semantic source artifact",
            (_, SemanticChange::SourceRemoved) => "removed semantic source artifact",
            (_, SemanticChange::RegistrationUpdated) => "updated semantic registration",
            (_, SemanticChange::SupportUpdated) => "updated semantic support state",
            (_, SemanticChange::ModuleUpdated) => "updated module source",
            (_, SemanticChange::NoChange) => unreachable!("no-change values are filtered"),
        })
        .map(str::to_string)
        .collect::<Vec<_>>();
    let artifacts = result
        .artifacts()
        .iter()
        .map(semantic_artifact_label)
        .collect::<Vec<_>>();
    let errors = result
        .diagnostics()
        .iter()
        .map(|diagnostic| writer_diagnostic_message(diagnostic, command))
        .collect::<Vec<_>>();
    AdapterOutcome {
        ok,
        summary,
        changes,
        warnings: Vec::new(),
        errors: errors.clone(),
        artifacts,
        stdout: None,
        stderr: (!errors.is_empty()).then(|| format!("{}\n", errors.join("\n"))),
        command: None,
    }
}

fn semantic_artifact_label(artifact: &SemanticArtifactRef) -> String {
    let kind = match artifact.kind() {
        SemanticArtifact::Configuration => "configuration",
        SemanticArtifact::Extension => "extension",
        SemanticArtifact::ExternalProcessor => "external processor",
        SemanticArtifact::ExternalReport => "external report",
        SemanticArtifact::MetadataObject => "metadata object",
        SemanticArtifact::Form => "form",
        SemanticArtifact::Template => "template",
        SemanticArtifact::Help => "help",
        SemanticArtifact::Interface => "interface",
        SemanticArtifact::Role => "role",
        SemanticArtifact::Subsystem => "subsystem",
        SemanticArtifact::SupportState => "support state",
        SemanticArtifact::DataComposition => "data composition",
        SemanticArtifact::Spreadsheet => "spreadsheet",
        SemanticArtifact::Module => "module",
        SemanticArtifact::RecoveryState => "recovery state",
    };
    match artifact.object() {
        SemanticObjectIdentity::Unspecified => kind.to_string(),
        SemanticObjectIdentity::ExternalObject { name, .. } => {
            format!("{kind} {}", name.as_str())
        }
        SemanticObjectIdentity::ExternalObjectModule { owner, .. } => {
            format!("{kind} for {}", owner.as_str())
        }
        SemanticObjectIdentity::ExternalPrimaryForm { owner, form, .. } => {
            format!("{kind} {} for {}", form.as_str(), owner.as_str())
        }
    }
}

fn writer_diagnostic_message(diagnostic: &WriterDiagnostic, command: &WriterCommand) -> String {
    if let (DiagnosticCode::PlannerRejected, Some(DiagnosticDetail::MetadataKind(metadata_kind))) =
        (diagnostic.code(), diagnostic.detail())
    {
        return format!("Unsupported type: {}", metadata_kind.as_str());
    }
    if let (
        DiagnosticCode::UnknownObjectKind,
        Some(DiagnosticDetail::MetadataKind(metadata_kind)),
    ) = (diagnostic.code(), diagnostic.detail())
    {
        return format!("Unknown type '{}'", metadata_kind.as_str());
    }
    if let (DiagnosticCode::InvalidObjectReference, Some(DiagnosticDetail::Object(object))) =
        (diagnostic.code(), diagnostic.detail())
    {
        return format!("Invalid format '{}', expected 'Type.Name'", object.as_str());
    }
    if let (DiagnosticCode::MissingFormCompanion, Some(DiagnosticDetail::FormElement(element))) =
        (diagnostic.code(), diagnostic.detail())
    {
        return format!("{}: missing companion", element.as_str());
    }
    match diagnostic.code() {
        DiagnosticCode::Cancelled => "operation cancelled",
        DiagnosticCode::InvalidRequest => "request arguments are invalid",
        DiagnosticCode::InvalidDefinition => "semantic definition is invalid",
        DiagnosticCode::NotFound => "requested semantic artifact was not found",
        DiagnosticCode::AlreadyExists => "requested semantic artifact already exists",
        DiagnosticCode::UnsupportedState => "source state is not supported for this operation",
        DiagnosticCode::UnsupportedFormat => "source format is not supported",
        DiagnosticCode::AuthorabilityBlocked => "source is not authorable",
        DiagnosticCode::SupportBlocked => "source support state blocks this operation",
        DiagnosticCode::NoDowngrade => "operation would downgrade the source format",
        DiagnosticCode::Conflict => "publication target differs from the expected preimage",
        DiagnosticCode::ValidationFailed => "semantic validation failed",
        DiagnosticCode::PlannerRejected if matches!(command, WriterCommand::MetadataCreate(_)) => {
            "metadata compilation planner rejected the semantic definition"
        }
        DiagnosticCode::PlannerRejected => "writer planner rejected the semantic definition",
        DiagnosticCode::OwnerResolutionFailed => {
            "semantic owner resolution failed before publication"
        }
        DiagnosticCode::PublicationFailed => "atomic publication failed",
        DiagnosticCode::RollbackFailed => "atomic publication rollback failed",
        DiagnosticCode::RecoveryRequired => "publication recovery is required",
        DiagnosticCode::PathRejected => "artifact containment validation failed",
        DiagnosticCode::ReadOnlyArtifact => "publication target is read-only",
        DiagnosticCode::AliasedArtifact => "publication target has multiple hard links",
        DiagnosticCode::InvalidMutation => "Invalid property format, expected 'Key=Value'",
        DiagnosticCode::InvalidObjectReference => {
            "invalid semantic object reference, expected 'Type.Name'"
        }
        DiagnosticCode::UnknownObjectKind => "unknown semantic object kind",
        DiagnosticCode::MissingFormCompanion => "form element is missing companion",
        DiagnosticCode::SupportCapabilityDisabled => {
            "configuration editing is disabled; set Capability=on before changing an object"
        }
        DiagnosticCode::InvalidModuleReference => {
            "module reference contains an invalid semantic name component"
        }
        DiagnosticCode::ObjectNotBorrowed => "selected object is not a borrowed extension object",
    }
    .to_string()
}

fn writer_failure(tool_name: &str, mode: MutationMode, message: String) -> AdapterOutcome {
    AdapterOutcome {
        ok: false,
        summary: if mode.is_preview() {
            format!("dry run: {tool_name} failed")
        } else {
            format!("{tool_name} failed")
        },
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
    cancellation: &OperationCancellation,
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
    let request = InspectionRequest::new(session, command, cancellation.clone());
    Some(match factory.inspection_port().inspect(&request) {
        Ok(result) => AdapterOutcome {
            ok: result.ok(),
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

fn parse_extension_module_target(value: &str) -> Result<ExtensionModuleTarget, String> {
    let invalid = || {
        "ModulePath contains an invalid semantic name component; expected Type.Name.Module, Type.Name.Form.FormName, or CommonModule.Name"
            .to_string()
    };
    let safe_segment = |segment: &str| {
        !segment.is_empty()
            && !segment.contains('/')
            && !segment.contains('\\')
            && segment != "."
            && segment != ".."
    };
    let parts = value.split('.').collect::<Vec<_>>();
    match parts.as_slice() {
        ["CommonModule", name] if safe_segment(name) => Ok(ExtensionModuleTarget::Common {
            module: CommonModuleName::new((*name).to_string()).map_err(|_| invalid())?,
        }),
        [kind, name, "Form", form]
            if safe_segment(kind) && safe_segment(name) && safe_segment(form) =>
        {
            Ok(ExtensionModuleTarget::Form {
                owner: MetadataObjectReference::new(format!("{kind}.{name}"))
                    .map_err(|_| invalid())?,
                form: FormName::new((*form).to_string()).map_err(|_| invalid())?,
            })
        }
        [kind, name, role] if safe_segment(kind) && safe_segment(name) => {
            let role = match *role {
                "ObjectModule" => ExtensionObjectModuleRole::Object,
                "ManagerModule" => ExtensionObjectModuleRole::Manager,
                "RecordSetModule" => ExtensionObjectModuleRole::RecordSet,
                "ValueManagerModule" => ExtensionObjectModuleRole::ValueManager,
                _ => return Err(invalid()),
            };
            Ok(ExtensionModuleTarget::Object {
                owner: MetadataObjectReference::new(format!("{kind}.{name}"))
                    .map_err(|_| invalid())?,
                role,
            })
        }
        _ => Err(invalid()),
    }
}

fn writer_command(
    operation: &str,
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<Option<WriterCommand>, String> {
    let command = match operation {
        "cf-init" => {
            let mut value = ConfigurationInitialize::new(required_text(
                args,
                &["name", "Name"],
                "Name",
                ConfigurationName::new,
            )?);
            value = value
                .with_synonym(optional_text(
                    args,
                    &["synonym", "Synonym"],
                    SynonymText::new,
                )?)
                .with_vendor(optional_text(args, &["vendor", "Vendor"], VendorName::new)?)
                .with_version(optional_text(
                    args,
                    &["version", "Version"],
                    ArtifactVersion::new,
                )?)
                .omit_default_role(bool_arg(args, &["noRole", "NoRole"]));
            WriterCommand::ConfigurationInitialize(value)
        }
        "cf-edit" => {
            let value = if let Some(verb) = first_string(args, &["operation", "Operation"]) {
                ConfigurationEdit::mutate(parse_configuration_mutation(
                    verb,
                    required_string(args, &["value", "Value"], "Value")?,
                )?)
            } else {
                ConfigurationEdit::from_patch(read_configuration_patch(
                    args,
                    &["definitionFile", "DefinitionFile"],
                    "DefinitionFile",
                    context,
                )?)
            };
            WriterCommand::ConfigurationEdit(value)
        }
        "cfe-init" => {
            let value = ExtensionInitialize::new(required_text(
                args,
                &["name", "Name"],
                "Name",
                ExtensionName::new,
            )?)
            .with_synonym(optional_text(
                args,
                &["synonym", "Synonym"],
                SynonymText::new,
            )?)
            .with_purpose(parse_optional_extension_purpose(args)?)
            .with_prefix(optional_text(
                args,
                &["namePrefix", "NamePrefix"],
                NamePrefix::new,
            )?);
            WriterCommand::ExtensionInitialize(value)
        }
        "cfe-borrow" => {
            let value = ExtensionBorrow::new(required_text(
                args,
                &["object", "Object"],
                "Object",
                MetadataObjectReference::new,
            )?)
            .with_main_attribute(parse_borrow_scope(args)?)
            .exclude_selection(bool_arg(args, &["noSelection", "NoSelection"]))
            .force(bool_arg(args, &["force", "Force"]));
            WriterCommand::ExtensionBorrow(value)
        }
        "cfe-patch-method" => WriterCommand::ExtensionPatchMethod(ExtensionPatchMethod::new(
            parse_extension_module_target(required_string(
                args,
                &["modulePath", "ModulePath"],
                "ModulePath",
            )?)?,
            required_text(
                args,
                &["methodName", "MethodName"],
                "MethodName",
                MethodName::new,
            )?,
            parse_interceptor(
                first_string(args, &["interceptorType", "InterceptorType"]).unwrap_or("before"),
            )?,
            parse_execution_context(first_string(args, &["context", "Context"]).unwrap_or("auto"))?,
            bool_arg(args, &["isFunction", "IsFunction"]),
        )),
        "epf-init" | "erf-init" => {
            let value = ExternalArtifactInitialize::new(required_text(
                args,
                &["name", "Name"],
                "Name",
                ExternalArtifactName::new,
            )?)
            .with_synonym(optional_text(
                args,
                &["synonym", "Synonym"],
                SynonymText::new,
            )?)
            .with_form(optional_text(
                args,
                &["formName", "FormName"],
                FormName::new,
            )?);
            if operation == "epf-init" {
                WriterCommand::ExternalProcessorInitialize(value)
            } else {
                WriterCommand::ExternalReportInitialize(value)
            }
        }
        "meta-compile" => WriterCommand::MetadataCreate(
            MetadataCreate::batch(read_metadata_definitions(
                args,
                &["jsonPath", "JsonPath"],
                "JsonPath",
                context,
            )?)
            .map_err(|error| format!("invalid metadata definition batch: {error}"))?
            .omit_default_role(bool_arg(args, &["noRole", "NoRole"]))
            .assign_default_form(bool_arg(args, &["setDefault", "SetDefault"])),
        ),
        "meta-edit" => {
            let patch = match first_string(args, &["operation", "Operation"]) {
                Some(operation) => parse_metadata_patch(
                    operation,
                    required_string(args, &["value", "Value"], "Value")?,
                )?,
                None => read_metadata_patch_definition(
                    args,
                    &["definitionFile", "DefinitionFile"],
                    "DefinitionFile",
                    context,
                )?,
            };
            let value = match first_string(args, &["object", "Object"]) {
                Some(object) => MetadataEdit::new(
                    MetadataObjectReference::new(object.to_string())
                        .map_err(|error| format!("invalid Object: {error}"))?,
                    patch,
                ),
                None => MetadataEdit::selected_object(patch),
            }
            .create_if_missing(bool_arg(args, &["createIfMissing", "CreateIfMissing"]));
            WriterCommand::MetadataEdit(value)
        }
        "meta-remove" => WriterCommand::MetadataRemove(MetadataRemove::new(
            required_text(
                args,
                &["object", "Object"],
                "Object",
                MetadataObjectReference::new,
            )?,
            bool_arg(args, &["keepFiles", "KeepFiles"]),
        )),
        "form-add" => WriterCommand::FormCreate(
            FormCreate::new(
                required_text(
                    args,
                    &["objectName", "ObjectName"],
                    "ObjectName",
                    FormOwnerReference::new,
                )?,
                required_text(args, &["formName", "FormName"], "FormName", FormName::new)?,
            )
            .with_synonym(optional_text(
                args,
                &["synonym", "Synonym"],
                SynonymText::new,
            )?)
            .with_purpose(parse_form_purpose(args)?)
            .with_default_assignment(parse_default_form_assignment(args)?),
        ),
        "form-compile" => {
            let skip_validation = bool_arg(args, &["noValidate", "NoValidate"]);
            let value = if bool_arg(args, &["fromObject", "FromObject"]) {
                FormCompile::from_object(
                    parse_optional_form_compile_purpose(args)?,
                    skip_validation,
                )
            } else {
                FormCompile::new(
                    read_managed_form_definition(
                        args,
                        &["jsonPath", "JsonPath"],
                        "JsonPath",
                        context,
                    )?,
                    skip_validation,
                )
            };
            WriterCommand::FormCompile(value)
        }
        "form-edit" => WriterCommand::FormEdit(parse_form_edit(args, context)?),
        "form-remove" => WriterCommand::FormRemove(FormRemove::new(
            required_public_owner(
                args,
                &["objectName", "ObjectName"],
                "ObjectName",
                FormOwnerReference::new,
            )?,
            required_text(args, &["formName", "FormName"], "FormName", FormName::new)?,
        )),
        "template-add" => WriterCommand::TemplateCreate(
            TemplateCreate::new(
                required_public_owner(
                    args,
                    &["objectName", "ObjectName"],
                    "ObjectName",
                    TemplateOwnerReference::new,
                )?,
                required_text(
                    args,
                    &["templateName", "TemplateName"],
                    "TemplateName",
                    TemplateName::new,
                )?,
                parse_template_kind(required_string(
                    args,
                    &["templateType", "TemplateType"],
                    "TemplateType",
                )?)?,
            )
            .with_synonym(optional_text(
                args,
                &["synonym", "Synonym"],
                SynonymText::new,
            )?)
            .assign_main_data_composition(bool_arg(args, &["setMainSKD", "SetMainSKD"])),
        ),
        "template-remove" => WriterCommand::TemplateRemove(TemplateRemove::new(
            required_public_owner(
                args,
                &["objectName", "ObjectName"],
                "ObjectName",
                TemplateOwnerReference::new,
            )?,
            required_text(
                args,
                &["templateName", "TemplateName"],
                "TemplateName",
                TemplateName::new,
            )?,
        )),
        "help-add" => WriterCommand::HelpCreate(HelpCreate::new(
            required_public_owner(
                args,
                &["objectName", "ObjectName", "processorName", "ProcessorName"],
                "ObjectName",
                HelpOwnerReference::new,
            )?,
            optional_text(
                args,
                &["lang", "Lang", "language", "Language"],
                LanguageCode::new,
            )?,
        )),
        "interface-edit" => {
            WriterCommand::InterfaceEdit(match first_string(args, &["operation", "Operation"]) {
                Some(operation) => parse_interface_edit(
                    operation,
                    required_string(args, &["value", "Value"], "Value")?,
                )?,
                None => InterfaceEdit::Replace(read_json_path(
                    args,
                    &["definitionFile", "DefinitionFile"],
                    "DefinitionFile",
                    context,
                )?),
            })
        }
        "role-compile" => WriterCommand::RoleCreate(RoleCreate::from_definition(
            read_role_definition(args, &["jsonPath", "JsonPath"], "JsonPath", context)?,
        )),
        "subsystem-compile" => WriterCommand::SubsystemCreate(SubsystemCreate::from_definition(
            read_subsystem_definition(
                args,
                &["definitionFile", "DefinitionFile"],
                &["value", "Value"],
                context,
            )?,
        )),
        "subsystem-edit" => {
            WriterCommand::SubsystemEdit(match first_string(args, &["operation", "Operation"]) {
                Some(operation) => parse_subsystem_edit(
                    operation,
                    required_string(args, &["value", "Value"], "Value")?,
                )?,
                None => SubsystemEdit::Replace(read_json_path(
                    args,
                    &["definitionFile", "DefinitionFile"],
                    "DefinitionFile",
                    context,
                )?),
            })
        }
        "support-edit" => WriterCommand::SupportEdit(parse_support_edit(args)?),
        "dcs-compile" => WriterCommand::DataCompositionCreate(DataCompositionCreate::new(
            read_definition_or_inline(
                args,
                &["definitionFile", "DefinitionFile"],
                &["value", "Value"],
                "data composition definition",
                context,
            )?,
        )),
        "dcs-edit" => {
            let value = DataCompositionEdit::new(parse_data_composition_mutation(
                required_string(args, &["operation", "Operation"], "Operation")?,
                required_string(args, &["value", "Value"], "Value")?,
            )?)
            .with_data_set(optional_text(
                args,
                &["dataSet", "DataSet"],
                DataSetName::new,
            )?)
            .with_variant(optional_text(
                args,
                &["variant", "Variant"],
                VariantName::new,
            )?)
            .omit_selection(bool_arg(args, &["noSelection", "NoSelection"]))
            .skip_validation(bool_arg(args, &["noValidate", "NoValidate"]));
            WriterCommand::DataCompositionEdit(value)
        }
        "mxl-compile" => WriterCommand::SpreadsheetCreate(SpreadsheetCreate::new(
            read_spreadsheet_document(args, &["jsonPath", "JsonPath"], "JsonPath", context)?,
        )),
        _ => return Ok(None),
    };
    Ok(Some(command))
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
        "help-add" => push(WriterSourceRole::SourceCollection, &["SrcDir"]),
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

fn read_json_path<T>(
    args: &Map<String, Value>,
    names: &[&str],
    label: &str,
    context: &WorkspaceContext,
) -> Result<T, String>
where
    T: serde::de::DeserializeOwned,
{
    serde_json::from_value(read_json_value(args, names, label, context)?)
        .map_err(|error| format!("failed to parse {label} semantic JSON: {error}"))
}

fn read_json_value(
    args: &Map<String, Value>,
    names: &[&str],
    label: &str,
    context: &WorkspaceContext,
) -> Result<Value, String> {
    let raw = required_string(args, names, label)?;
    let path = PathBuf::from(raw);
    let path = if path.is_absolute() {
        path
    } else {
        context.cwd.join(path)
    };
    let bytes = fs::read(&path)
        .map_err(|error| format!("failed to read {label} {}: {error}", path.display()))?;
    let bytes = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(&bytes);
    serde_json::from_slice(bytes)
        .map_err(|error| format!("failed to parse {label} semantic JSON: {error}"))
}

fn read_configuration_patch(
    args: &Map<String, Value>,
    names: &[&str],
    label: &str,
    context: &WorkspaceContext,
) -> Result<ConfigurationPatchSet, String> {
    let value = read_json_value(args, names, label, context)?;
    let items = match value {
        Value::Array(items) => items,
        value => vec![value],
    };
    let mut mutations = Vec::new();
    for item in items {
        let object = item
            .as_object()
            .ok_or_else(|| "configuration edit definition item must be an object".to_string())?;
        reject_unknown_fields(object, &["operation", "value"], "configuration edit item")?;
        let operation = object
            .get("operation")
            .and_then(Value::as_str)
            .ok_or_else(|| "configuration edit item requires operation".to_string())?;
        let value = object
            .get("value")
            .ok_or_else(|| "configuration edit item requires value".to_string())?;
        mutations.extend(parse_configuration_definition_mutations(operation, value)?);
    }
    ConfigurationPatchSet::new(mutations)
        .map_err(|error| format!("invalid configuration patch set: {error}"))
}

fn parse_configuration_definition_mutations(
    operation: &str,
    value: &Value,
) -> Result<Vec<ConfigurationMutation>, String> {
    fn values(value: &Value) -> Vec<&Value> {
        value
            .as_array()
            .map_or_else(|| vec![value], |items| items.iter().collect())
    }
    match operation {
        "set-panels" => Ok(vec![ConfigurationMutation::SetPanelLayout(
            parse_configuration_panel_layout(value)?,
        )]),
        "set-home-page" => Ok(vec![ConfigurationMutation::SetHomePageLayout(
            parse_configuration_home_page_layout(value)?,
        )]),
        "set-defaultRoles" => Ok(vec![ConfigurationMutation::SetDefaultRoles(
            values(value)
                .into_iter()
                .map(|item| {
                    item.as_str()
                        .ok_or_else(|| "default role must be text".to_string())
                        .and_then(|item| {
                            RoleName::new(item.to_string())
                                .map_err(|error| format!("invalid role name: {error}"))
                        })
                })
                .collect::<Result<Vec<_>, _>>()?,
        )]),
        "add-defaultRole" | "remove-defaultRole" | "add-childObject" | "remove-childObject" => {
            values(value)
                .into_iter()
                .map(|item| {
                    let item = item
                        .as_str()
                        .ok_or_else(|| format!("{operation} value must be text"))?;
                    parse_configuration_mutation(operation, item)
                })
                .collect()
        }
        _ => {
            let value = value
                .as_str()
                .ok_or_else(|| format!("{operation} value must be text"))?;
            Ok(vec![parse_configuration_mutation(operation, value)?])
        }
    }
}

fn parse_configuration_panel_layout(value: &Value) -> Result<ConfigurationPanelLayout, String> {
    let object = value
        .as_object()
        .ok_or_else(|| "set-panels value must be a JSON object".to_string())?;
    reject_unknown_fields(object, &["top", "left", "right", "bottom"], "panel layout")?;
    let mut sections = Vec::new();
    for (name, side) in [
        ("top", ConfigurationPanelSide::Top),
        ("left", ConfigurationPanelSide::Left),
        ("right", ConfigurationPanelSide::Right),
        ("bottom", ConfigurationPanelSide::Bottom),
    ] {
        let Some(value) = object.get(name) else {
            continue;
        };
        let values = value
            .as_array()
            .map_or_else(|| vec![value], |items| items.iter().collect());
        let entries = values
            .into_iter()
            .map(parse_configuration_panel_entry)
            .collect::<Result<Vec<_>, _>>()?;
        sections.push(
            ConfigurationPanelSection::new(side, entries)
                .map_err(|error| format!("invalid panel section: {error}"))?,
        );
    }
    ConfigurationPanelLayout::new(sections)
        .map_err(|error| format!("invalid panel layout: {error}"))
}

fn parse_configuration_panel_entry(value: &Value) -> Result<ConfigurationPanelEntry, String> {
    if let Some(value) = value.as_str() {
        let panel = match value.to_lowercase().as_str() {
            "sections" | "разделов" | "разделы" => ConfigurationPanel::Sections,
            "functions" | "функций" | "функции" => ConfigurationPanel::Functions,
            "favorites" | "избранного" | "избранное" => {
                ConfigurationPanel::Favorites
            }
            "history" | "истории" | "история" => ConfigurationPanel::History,
            "open" | "открытых" | "открытые" => ConfigurationPanel::Open,
            _ => return Err(format!("unknown panel alias: {value}")),
        };
        return Ok(ConfigurationPanelEntry::Panel(panel));
    }
    let object = value
        .as_object()
        .ok_or_else(|| "panel entry must be a panel alias or group".to_string())?;
    reject_unknown_fields(object, &["group"], "panel group")?;
    let children = object
        .get("group")
        .and_then(Value::as_array)
        .ok_or_else(|| "panel group requires a non-empty group array".to_string())?
        .iter()
        .map(parse_configuration_panel_entry)
        .collect::<Result<Vec<_>, _>>()?;
    if children.is_empty() {
        return Err("panel group requires a non-empty group array".to_string());
    }
    Ok(ConfigurationPanelEntry::Group(children))
}

fn parse_configuration_home_page_layout(
    value: &Value,
) -> Result<ConfigurationHomePageLayout, String> {
    let object = value
        .as_object()
        .ok_or_else(|| "set-home-page value must be a JSON object".to_string())?;
    reject_unknown_fields(
        object,
        &[
            "template",
            "WorkingAreaTemplate",
            "left",
            "LeftColumn",
            "right",
            "RightColumn",
        ],
        "home page layout",
    )?;
    let template = first_object_string(object, &["template", "WorkingAreaTemplate"])
        .unwrap_or("TwoColumnsEqualWidth");
    let template = match template {
        "OneColumn" => ConfigurationHomePageTemplate::OneColumn,
        "TwoColumnsEqualWidth" => ConfigurationHomePageTemplate::TwoColumnsEqualWidth,
        "TwoColumnsVariableWidth" => ConfigurationHomePageTemplate::TwoColumnsVariableWidth,
        value => return Err(format!("unknown home page template: {value}")),
    };
    let left = first_object_value(object, &["left", "LeftColumn"])
        .map(parse_configuration_home_page_entries)
        .transpose()?
        .unwrap_or_default();
    let right = first_object_value(object, &["right", "RightColumn"])
        .map(parse_configuration_home_page_entries)
        .transpose()?
        .unwrap_or_default();
    ConfigurationHomePageLayout::new(template, left, right)
        .map_err(|_| "OneColumn home page cannot contain right-column entries".to_string())
}

fn parse_configuration_home_page_entries(
    value: &Value,
) -> Result<Vec<ConfigurationHomePageEntry>, String> {
    let values = value
        .as_array()
        .map_or_else(|| vec![value], |items| items.iter().collect());
    values
        .into_iter()
        .map(parse_configuration_home_page_entry)
        .collect()
}

fn parse_configuration_home_page_entry(
    value: &Value,
) -> Result<ConfigurationHomePageEntry, String> {
    if let Some(form) = value.as_str() {
        return MetadataObjectReference::new(form.to_string())
            .map(ConfigurationHomePageEntry::new)
            .map_err(|error| format!("invalid home page form reference: {error}"));
    }
    let object = value
        .as_object()
        .ok_or_else(|| "home page entry must be text or an object".to_string())?;
    reject_unknown_fields(
        object,
        &[
            "form",
            "Form",
            "height",
            "Height",
            "visibility",
            "Visibility",
            "roles",
        ],
        "home page entry",
    )?;
    let form = first_object_string(object, &["form", "Form"])
        .ok_or_else(|| "home page entry requires form".to_string())?;
    let mut entry = ConfigurationHomePageEntry::new(
        MetadataObjectReference::new(form.to_string())
            .map_err(|error| format!("invalid home page form reference: {error}"))?,
    );
    if let Some(height) = first_object_value(object, &["height", "Height"]) {
        let height = height
            .as_u64()
            .and_then(|value| u16::try_from(value).ok())
            .ok_or_else(|| "home page entry height must be a positive integer".to_string())?;
        entry = entry
            .with_height(height)
            .map_err(|error| format!("invalid home page entry height: {error}"))?;
    }
    if let Some(visible) = first_object_value(object, &["visibility", "Visibility"]) {
        entry = entry.visible(
            visible
                .as_bool()
                .ok_or_else(|| "home page entry visibility must be boolean".to_string())?,
        );
    }
    let role_visibility = object
        .get("roles")
        .map(|roles| {
            roles
                .as_object()
                .ok_or_else(|| "home page entry roles must be an object".to_string())?
                .iter()
                .map(|(name, visible)| {
                    Ok(ConfigurationHomePageRoleVisibility::new(
                        RoleName::new(name.trim_start_matches("Role.").to_string())
                            .map_err(|error| format!("invalid role name: {error}"))?,
                        visible
                            .as_bool()
                            .ok_or_else(|| "role visibility must be boolean".to_string())?,
                    ))
                })
                .collect::<Result<Vec<_>, String>>()
        })
        .transpose()?
        .unwrap_or_default();
    Ok(entry.with_role_visibility(role_visibility))
}

fn reject_unknown_fields(
    object: &Map<String, Value>,
    allowed: &[&str],
    label: &str,
) -> Result<(), String> {
    if let Some(name) = object.keys().find(|name| !allowed.contains(&name.as_str())) {
        return Err(format!("{label} contains unknown field {name:?}"));
    }
    Ok(())
}

fn first_object_value<'a>(object: &'a Map<String, Value>, names: &[&str]) -> Option<&'a Value> {
    names.iter().find_map(|name| object.get(*name))
}

fn first_object_string<'a>(object: &'a Map<String, Value>, names: &[&str]) -> Option<&'a str> {
    first_object_value(object, names).and_then(Value::as_str)
}

fn read_metadata_definitions(
    args: &Map<String, Value>,
    names: &[&str],
    label: &str,
    context: &WorkspaceContext,
) -> Result<Vec<MetadataDefinition>, String> {
    let value = read_json_value(args, names, label, context)?;
    let values = match value {
        Value::Array(values) => values,
        value => vec![value],
    };
    values.iter().map(parse_metadata_definition_value).collect()
}

fn read_metadata_patch_definition(
    args: &Map<String, Value>,
    names: &[&str],
    label: &str,
    context: &WorkspaceContext,
) -> Result<MetadataPatch, String> {
    let value = read_json_value(args, names, label, context)?;
    if let Ok(patch) = serde_json::from_value::<MetadataPatch>(value.clone()) {
        return Ok(patch);
    }
    if value.get("type").is_some() || value.get("objectType").is_some() {
        return parse_metadata_definition_value(&value).map(MetadataPatch::Replace);
    }
    Err("metadata definition file must contain closed semantic patch operations".to_string())
}

fn parse_metadata_definition_value(value: &Value) -> Result<MetadataDefinition, String> {
    let object = value
        .as_object()
        .ok_or_else(|| "metadata definition must be an object".to_string())?;
    const FIELDS: &[&str] = &[
        "type",
        "objectType",
        "name",
        "synonym",
        "comment",
        "attributes",
        "tabularSections",
        "dimensions",
        "resources",
        "addressingAttributes",
        "columns",
        "forms",
        "templates",
        "commands",
        "requisites",
        "values",
        "hierarchical",
        "limitLevelCount",
        "levelCount",
        "foldersOnTop",
        "codeLength",
        "descriptionLength",
        "numberLength",
        "checkUnique",
        "autonumbering",
        "quickChoice",
        "sequenceFilling",
        "postInPrivilegedMode",
        "unpostInPrivilegedMode",
        "mainFilterOnPeriod",
        "enableTotalsSplitting",
        "correspondence",
        "periodAdjustmentLength",
        "actionPeriod",
        "basePeriod",
        "maxExtDimensionCount",
        "codeMask",
        "autoOrderByCode",
        "orderLength",
        "actionPeriodUse",
        "distributedInfoBase",
        "includeConfigurationExtensions",
        "restartCountOnFailure",
        "restartIntervalOnFailure",
        "sessionMaxAge",
        "length",
        "precision",
        "nonnegative",
        "createTaskInPrivilegedMode",
        "valueType",
        "valueTypes",
        "context",
        "returnValuesReuse",
        "hierarchyType",
        "periodicity",
        "registerType",
        "chartOfAccounts",
        "chartOfCalculationTypes",
        "extDimensionTypes",
        "accountingFlags",
        "extDimensionAccountingFlags",
        "dependenceOnCalculationTypes",
        "baseCalculationTypes",
        "task",
        "addressing",
        "mainAddressingAttribute",
        "registeredDocuments",
        "methodName",
        "description",
        "key",
        "use",
        "predefined",
        "source",
        "event",
        "handler",
        "rootURL",
        "rootUrl",
        "reuseSessions",
        "urlTemplates",
        "namespace",
        "operations",
    ];
    reject_unknown_fields(object, FIELDS, "metadata definition")?;
    let kind = parse_metadata_kind(
        first_object_string(object, &["type", "objectType"])
            .ok_or_else(|| "metadata definition requires type".to_string())?,
    )?;
    let name = MetadataChildName::new(
        first_object_string(object, &["name"])
            .ok_or_else(|| "metadata definition requires name".to_string())?
            .to_string(),
    )
    .map_err(|error| format!("invalid metadata name: {error}"))?;

    let mut common = MetadataCommonDefinition::new(name)
        .with_synonym(optional_semantic_text(
            object.get("synonym"),
            SynonymText::new,
            "synonym",
        )?)
        .with_comment(optional_semantic_text(
            object.get("comment"),
            CommentText::new,
            "comment",
        )?)
        .with_attributes(parse_metadata_fields(object.get("attributes"))?)
        .with_tabular_sections(parse_metadata_tabular_sections(
            object.get("tabularSections"),
        )?)
        .with_dimensions(parse_metadata_fields(object.get("dimensions"))?)
        .with_resources(parse_metadata_fields(object.get("resources"))?)
        .with_addressing_attributes(parse_metadata_fields(object.get("addressingAttributes"))?)
        .with_columns(parse_metadata_fields(object.get("columns"))?);
    let mut children = Vec::new();
    for (field, child_kind) in [
        ("forms", MetadataNamedChildKind::Form),
        ("templates", MetadataNamedChildKind::Template),
        ("commands", MetadataNamedChildKind::Command),
        ("requisites", MetadataNamedChildKind::Requisite),
        ("values", MetadataNamedChildKind::EnumValue),
    ] {
        children.extend(parse_metadata_named_children(
            object.get(field),
            child_kind,
        )?);
    }
    common = common.with_children(children);
    let properties = parse_metadata_kind_properties(object)?;
    Ok(MetadataDefinition::new(
        common,
        MetadataKindDefinition::new(kind, properties),
    ))
}

fn parse_metadata_kind(value: &str) -> Result<MetadataKind, String> {
    let value = match value {
        "Справочник" | "Каталог" => "Catalog",
        "Документ" => "Document",
        "Перечисление" => "Enum",
        "Константа" => "Constant",
        "РегистрСведений" => "InformationRegister",
        "РегистрНакопления" => "AccumulationRegister",
        "РегистрБухгалтерии" => "AccountingRegister",
        "РегистрРасчёта" | "РегистрРасчета" => "CalculationRegister",
        "ПланСчетов" => "ChartOfAccounts",
        "ПланВидовХарактеристик" => "ChartOfCharacteristicTypes",
        "ПланВидовРасчёта" | "ПланВидовРасчета" => {
            "ChartOfCalculationTypes"
        }
        "БизнесПроцесс" => "BusinessProcess",
        "Задача" => "Task",
        "ПланОбмена" => "ExchangePlan",
        "ЖурналДокументов" => "DocumentJournal",
        "Отчёт" | "Отчет" => "Report",
        "Обработка" => "DataProcessor",
        "ОбщийМодуль" => "CommonModule",
        "РегламентноеЗадание" => "ScheduledJob",
        "ПодпискаНаСобытие" => "EventSubscription",
        "HTTPСервис" => "HTTPService",
        "ВебСервис" => "WebService",
        "ОпределяемыйТип" => "DefinedType",
        value => value,
    };
    match value {
        "CommonModule" => Ok(MetadataKind::CommonModule),
        "SessionParameter" => Ok(MetadataKind::SessionParameter),
        "Role" => Ok(MetadataKind::Role),
        "CommonAttribute" => Ok(MetadataKind::CommonAttribute),
        "ExchangePlan" => Ok(MetadataKind::ExchangePlan),
        "XDTOPackage" => Ok(MetadataKind::XdtoPackage),
        "WebService" => Ok(MetadataKind::WebService),
        "HTTPService" => Ok(MetadataKind::HttpService),
        "WSReference" => Ok(MetadataKind::WsReference),
        "StyleItem" => Ok(MetadataKind::StyleItem),
        "CommonPicture" => Ok(MetadataKind::CommonPicture),
        "CommonTemplate" => Ok(MetadataKind::CommonTemplate),
        "FilterCriterion" => Ok(MetadataKind::FilterCriterion),
        "EventSubscription" => Ok(MetadataKind::EventSubscription),
        "ScheduledJob" => Ok(MetadataKind::ScheduledJob),
        "FunctionalOption" => Ok(MetadataKind::FunctionalOption),
        "FunctionalOptionsParameter" => Ok(MetadataKind::FunctionalOptionsParameter),
        "DefinedType" => Ok(MetadataKind::DefinedType),
        "SettingsStorage" => Ok(MetadataKind::SettingsStorage),
        "Language" => Ok(MetadataKind::Language),
        "CommandGroup" => Ok(MetadataKind::CommandGroup),
        "CommonCommand" => Ok(MetadataKind::CommonCommand),
        "DocumentNumerator" => Ok(MetadataKind::DocumentNumerator),
        "Sequence" => Ok(MetadataKind::Sequence),
        "Constant" => Ok(MetadataKind::Constant),
        "Catalog" => Ok(MetadataKind::Catalog),
        "Document" => Ok(MetadataKind::Document),
        "Enum" => Ok(MetadataKind::Enum),
        "Report" => Ok(MetadataKind::Report),
        "DataProcessor" => Ok(MetadataKind::DataProcessor),
        "ChartOfCharacteristicTypes" => Ok(MetadataKind::ChartOfCharacteristicTypes),
        "ChartOfAccounts" => Ok(MetadataKind::ChartOfAccounts),
        "ChartOfCalculationTypes" => Ok(MetadataKind::ChartOfCalculationTypes),
        "InformationRegister" => Ok(MetadataKind::InformationRegister),
        "AccumulationRegister" => Ok(MetadataKind::AccumulationRegister),
        "AccountingRegister" => Ok(MetadataKind::AccountingRegister),
        "CalculationRegister" => Ok(MetadataKind::CalculationRegister),
        "BusinessProcess" => Ok(MetadataKind::BusinessProcess),
        "Task" => Ok(MetadataKind::Task),
        "DocumentJournal" => Ok(MetadataKind::DocumentJournal),
        _ => Err(format!("Unsupported type: {value}")),
    }
}

fn parse_metadata_fields(
    value: Option<&Value>,
) -> Result<Vec<MetadataAttributeDefinition>, String> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    if let Some(object) = value.as_object() {
        return object
            .iter()
            .map(|(name, value)| {
                let value = if value.is_object() {
                    let mut value = value.as_object().expect("checked").clone();
                    value
                        .entry("name".to_string())
                        .or_insert_with(|| Value::String(name.clone()));
                    Value::Object(value)
                } else {
                    Value::String(format!("{name}:{}", json_scalar_text(value)?))
                };
                parse_metadata_field(&value)
            })
            .collect();
    }
    value
        .as_array()
        .ok_or_else(|| "metadata field collection must be an array or object".to_string())?
        .iter()
        .map(parse_metadata_field)
        .collect()
}

fn parse_metadata_field(value: &Value) -> Result<MetadataAttributeDefinition, String> {
    if let Some(value) = value.as_str() {
        let (main, flags) = value.split_once('|').map_or((value, ""), |parts| parts);
        let (name, value_type) = main.split_once(':').map_or((main, ""), |parts| parts);
        let mut field = MetadataAttributeDefinition::new(
            MetadataFieldName::new(name.trim().to_string())
                .map_err(|error| format!("invalid metadata field name: {error}"))?,
        );
        if !value_type.trim().is_empty() {
            field = field
                .with_type_expression(Some(parse_metadata_type_expression(value_type.trim())?));
        }
        return Ok(field.with_flags(parse_metadata_field_flags(flags)?));
    }
    let object = value
        .as_object()
        .ok_or_else(|| "metadata field must be text or an object".to_string())?;
    reject_unknown_fields(
        object,
        &[
            "name",
            "type",
            "valueType",
            "length",
            "precision",
            "nonneg",
            "nonnegative",
            "synonym",
            "comment",
            "flags",
            "fillChecking",
            "indexing",
            "multiLine",
            "choiceHistoryOnInput",
            "addressingDimension",
            "references",
        ],
        "metadata field",
    )?;
    let name = MetadataFieldName::new(
        first_object_string(object, &["name"])
            .ok_or_else(|| "metadata field requires name".to_string())?
            .to_string(),
    )
    .map_err(|error| format!("invalid metadata field name: {error}"))?;
    let type_text = first_object_string(object, &["valueType", "type"]);
    let type_expression = type_text
        .map(|value| {
            if !value.contains('(') {
                if value == "String" {
                    if let Some(length) = object.get("length").and_then(Value::as_u64) {
                        return parse_metadata_type_expression(&format!("String({length})"));
                    }
                } else if value == "Number" {
                    if let Some(length) = object.get("length").and_then(Value::as_u64) {
                        let precision =
                            object.get("precision").and_then(Value::as_u64).unwrap_or(0);
                        let suffix = if first_object_value(object, &["nonneg", "nonnegative"])
                            .and_then(Value::as_bool)
                            == Some(true)
                        {
                            ",nonneg"
                        } else {
                            ""
                        };
                        return parse_metadata_type_expression(&format!(
                            "Number({length},{precision}{suffix})"
                        ));
                    }
                }
            }
            parse_metadata_type_expression(value)
        })
        .transpose()?;
    let mut flags = object
        .get("flags")
        .map(|value| {
            value
                .as_array()
                .ok_or_else(|| "metadata field flags must be an array".to_string())?
                .iter()
                .map(|value| {
                    value
                        .as_str()
                        .ok_or_else(|| "metadata field flag must be text".to_string())
                })
                .collect::<Result<Vec<_>, _>>()
                .and_then(|values| parse_metadata_field_flags(&values.join(",")))
        })
        .transpose()?
        .unwrap_or_default();
    if object.get("multiLine").and_then(Value::as_bool) == Some(true)
        && !flags.contains(&MetadataFieldFlag::MultiLine)
    {
        flags.push(MetadataFieldFlag::MultiLine);
    }
    Ok(MetadataAttributeDefinition::new(name)
        .with_synonym(optional_semantic_text(
            object.get("synonym"),
            SynonymText::new,
            "synonym",
        )?)
        .with_comment(optional_semantic_text(
            object.get("comment"),
            CommentText::new,
            "comment",
        )?)
        .with_type_expression(type_expression)
        .with_flags(flags)
        .with_fill_checking(parse_optional_fill_checking(object.get("fillChecking"))?)
        .with_indexing(parse_optional_indexing(object.get("indexing"))?)
        .with_choice_history(parse_optional_choice_history(
            object.get("choiceHistoryOnInput"),
        )?)
        .with_addressing_dimension(optional_semantic_text(
            object.get("addressingDimension"),
            MetadataObjectReference::new,
            "addressingDimension",
        )?)
        .with_references(parse_object_references(object.get("references"))?))
}

fn parse_metadata_field_flags(value: &str) -> Result<Vec<MetadataFieldFlag>, String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| match value.to_ascii_lowercase().as_str() {
            "req" | "required" => Ok(MetadataFieldFlag::Required),
            "index" => Ok(MetadataFieldFlag::Index),
            "master" => Ok(MetadataFieldFlag::Master),
            "mainfilter" => Ok(MetadataFieldFlag::MainFilter),
            "denyincomplete" => Ok(MetadataFieldFlag::DenyIncomplete),
            "multiline" => Ok(MetadataFieldFlag::MultiLine),
            "nouseintotals" => Ok(MetadataFieldFlag::ExcludeFromTotals),
            _ => Err(format!("unknown metadata field flag: {value}")),
        })
        .collect()
}

fn parse_metadata_type_expression(value: &str) -> Result<MetadataTypeExpression, String> {
    let value = value.trim();
    if let Some(inner) = value
        .strip_prefix("String(")
        .and_then(|value| value.strip_suffix(')'))
    {
        let length = inner
            .trim()
            .parse::<u32>()
            .map_err(|_| format!("invalid String type length: {value}"))?;
        return Ok(MetadataTypeExpression::String {
            length: Some(length),
        });
    }
    if let Some(inner) = value
        .strip_prefix("Number(")
        .and_then(|value| value.strip_suffix(')'))
    {
        let mut parts = inner.split(',').map(str::trim);
        let length = parts
            .next()
            .ok_or_else(|| format!("invalid Number type: {value}"))?
            .parse::<u32>()
            .map_err(|_| format!("invalid Number type length: {value}"))?;
        let precision = parts
            .next()
            .unwrap_or("0")
            .parse::<u16>()
            .map_err(|_| format!("invalid Number type precision: {value}"))?;
        let nonnegative = parts
            .next()
            .is_some_and(|value| value.eq_ignore_ascii_case("nonneg"));
        if parts.next().is_some() {
            return Err(format!("invalid Number type: {value}"));
        }
        return Ok(MetadataTypeExpression::Number {
            length: Some(length),
            precision: Some(precision),
            nonnegative,
        });
    }
    let primitive = match value {
        "String" => Some(MetadataTypeExpression::String { length: None }),
        "Number" => Some(MetadataTypeExpression::Number {
            length: None,
            precision: None,
            nonnegative: false,
        }),
        "Boolean" => Some(MetadataTypeExpression::Boolean),
        "Date" => Some(MetadataTypeExpression::Date),
        "UUID" | "Uuid" => Some(MetadataTypeExpression::Uuid),
        "BinaryData" | "Binary" => Some(MetadataTypeExpression::Binary),
        "ValueStorage" => Some(MetadataTypeExpression::ValueStorage),
        "Any" => Some(MetadataTypeExpression::Any),
        _ => None,
    };
    if let Some(primitive) = primitive {
        return Ok(primitive);
    }
    let (prefix, target) = value
        .split_once('.')
        .ok_or_else(|| format!("unsupported metadata type expression: {value}"))?;
    let category = match prefix {
        "CatalogRef" => MetadataReferenceKind::Catalog,
        "DocumentRef" => MetadataReferenceKind::Document,
        "EnumRef" => MetadataReferenceKind::Enum,
        "DefinedType" => MetadataReferenceKind::DefinedType,
        "ChartOfCharacteristicTypesRef" => MetadataReferenceKind::ChartOfCharacteristicTypes,
        "ChartOfAccountsRef" => MetadataReferenceKind::ChartOfAccounts,
        "ChartOfCalculationTypesRef" => MetadataReferenceKind::ChartOfCalculationTypes,
        "BusinessProcessRef" => MetadataReferenceKind::BusinessProcess,
        "TaskRef" => MetadataReferenceKind::Task,
        "ExchangePlanRef" => MetadataReferenceKind::ExchangePlan,
        _ => return Err(format!("unsupported metadata reference category: {prefix}")),
    };
    Ok(MetadataTypeExpression::Reference {
        category,
        target: MetadataTypeTargetName::new(target.to_string())
            .map_err(|error| format!("invalid metadata type target: {error}"))?,
    })
}

fn parse_metadata_tabular_sections(
    value: Option<&Value>,
) -> Result<Vec<MetadataTabularSectionDefinition>, String> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    if let Some(object) = value.as_object() {
        return object
            .iter()
            .map(|(name, fields)| {
                Ok(MetadataTabularSectionDefinition::new(
                    MetadataChildName::new(name.clone())
                        .map_err(|error| format!("invalid tabular section name: {error}"))?,
                    parse_metadata_fields(Some(fields))?,
                ))
            })
            .collect();
    }
    value
        .as_array()
        .ok_or_else(|| "tabularSections must be an array or object".to_string())?
        .iter()
        .map(|value| {
            let object = value
                .as_object()
                .ok_or_else(|| "tabular section must be an object".to_string())?;
            reject_unknown_fields(
                object,
                &["name", "synonym", "attributes"],
                "tabular section",
            )?;
            Ok(MetadataTabularSectionDefinition::new(
                MetadataChildName::new(
                    first_object_string(object, &["name"])
                        .ok_or_else(|| "tabular section requires name".to_string())?
                        .to_string(),
                )
                .map_err(|error| format!("invalid tabular section name: {error}"))?,
                parse_metadata_fields(object.get("attributes"))?,
            )
            .with_synonym(optional_semantic_text(
                object.get("synonym"),
                SynonymText::new,
                "tabular section synonym",
            )?))
        })
        .collect()
}

fn parse_metadata_named_children(
    value: Option<&Value>,
    kind: MetadataNamedChildKind,
) -> Result<Vec<MetadataNamedChildDefinition>, String> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let values = value
        .as_array()
        .ok_or_else(|| "metadata child collection must be an array".to_string())?;
    values
        .iter()
        .map(|value| {
            let name = value.as_str().or_else(|| {
                value
                    .as_object()
                    .and_then(|value| value.get("name"))
                    .and_then(Value::as_str)
            });
            Ok(MetadataNamedChildDefinition::new(
                kind,
                MetadataChildName::new(
                    name.ok_or_else(|| "metadata child requires name".to_string())?
                        .to_string(),
                )
                .map_err(|error| format!("invalid metadata child name: {error}"))?,
            ))
        })
        .collect()
}

fn parse_metadata_kind_properties(
    object: &Map<String, Value>,
) -> Result<Vec<MetadataKindProperty>, String> {
    let mut result = Vec::new();
    for (name, property) in [
        ("hierarchical", MetadataKindPropertyName::Hierarchical),
        ("limitLevelCount", MetadataKindPropertyName::LimitLevelCount),
        ("levelCount", MetadataKindPropertyName::LevelCount),
        ("foldersOnTop", MetadataKindPropertyName::FoldersOnTop),
        ("codeLength", MetadataKindPropertyName::CodeLength),
        (
            "descriptionLength",
            MetadataKindPropertyName::DescriptionLength,
        ),
        ("numberLength", MetadataKindPropertyName::NumberLength),
        ("checkUnique", MetadataKindPropertyName::CheckUnique),
        ("autonumbering", MetadataKindPropertyName::Autonumbering),
        ("quickChoice", MetadataKindPropertyName::QuickChoice),
        ("sequenceFilling", MetadataKindPropertyName::SequenceFilling),
        (
            "postInPrivilegedMode",
            MetadataKindPropertyName::PostInPrivilegedMode,
        ),
        (
            "unpostInPrivilegedMode",
            MetadataKindPropertyName::UnpostInPrivilegedMode,
        ),
        (
            "mainFilterOnPeriod",
            MetadataKindPropertyName::MainFilterOnPeriod,
        ),
        (
            "enableTotalsSplitting",
            MetadataKindPropertyName::EnableTotalsSplitting,
        ),
        ("correspondence", MetadataKindPropertyName::Correspondence),
        (
            "periodAdjustmentLength",
            MetadataKindPropertyName::PeriodAdjustmentLength,
        ),
        ("actionPeriod", MetadataKindPropertyName::ActionPeriod),
        ("basePeriod", MetadataKindPropertyName::BasePeriod),
        (
            "maxExtDimensionCount",
            MetadataKindPropertyName::MaxExtDimensionCount,
        ),
        ("codeMask", MetadataKindPropertyName::CodeMask),
        ("autoOrderByCode", MetadataKindPropertyName::AutoOrderByCode),
        ("orderLength", MetadataKindPropertyName::OrderLength),
        ("actionPeriodUse", MetadataKindPropertyName::ActionPeriodUse),
        (
            "distributedInfoBase",
            MetadataKindPropertyName::DistributedInfoBase,
        ),
        (
            "includeConfigurationExtensions",
            MetadataKindPropertyName::IncludeConfigurationExtensions,
        ),
        (
            "restartCountOnFailure",
            MetadataKindPropertyName::RestartCountOnFailure,
        ),
        (
            "restartIntervalOnFailure",
            MetadataKindPropertyName::RestartIntervalOnFailure,
        ),
        ("sessionMaxAge", MetadataKindPropertyName::SessionMaxAge),
        ("length", MetadataKindPropertyName::Length),
        ("precision", MetadataKindPropertyName::Precision),
        ("nonnegative", MetadataKindPropertyName::Nonnegative),
        (
            "createTaskInPrivilegedMode",
            MetadataKindPropertyName::CreateTaskInPrivilegedMode,
        ),
    ] {
        let Some(value) = object.get(name) else {
            continue;
        };
        let value = match value {
            Value::Bool(value) => MetadataPropertyValue::Boolean(*value),
            Value::Number(value) => MetadataPropertyValue::Integer(
                value
                    .as_i64()
                    .ok_or_else(|| format!("{name} must be an integer"))?,
            ),
            Value::String(value) => MetadataPropertyValue::Text(
                MetadataPropertyText::new(value.clone())
                    .map_err(|error| format!("invalid {name}: {error}"))?,
            ),
            _ => return Err(format!("{name} must be boolean, integer, or text")),
        };
        result.push(
            MetadataKindProperty::new(property, value)
                .map_err(|error| format!("invalid {name}: {error}"))?,
        );
    }
    push_metadata_property(
        &mut result,
        object,
        "valueType",
        MetadataKindPropertyName::ValueType,
        |value| {
            Ok(MetadataPropertyValue::Type(parse_metadata_type_expression(
                required_json_text(value, "valueType")?,
            )?))
        },
    )?;
    push_metadata_property(
        &mut result,
        object,
        "valueTypes",
        MetadataKindPropertyName::ValueTypes,
        |value| {
            Ok(MetadataPropertyValue::Types(parse_type_expression_list(
                value,
            )?))
        },
    )?;
    push_metadata_property(
        &mut result,
        object,
        "context",
        MetadataKindPropertyName::Context,
        |value| {
            let value = match required_json_text(value, "context")?
                .to_ascii_lowercase()
                .as_str()
            {
                "client" => MetadataModuleContext::Client,
                "server" => MetadataModuleContext::Server,
                "clientandserver" | "client-server" => MetadataModuleContext::ClientAndServer,
                "externalconnection" | "external" => MetadataModuleContext::ExternalConnection,
                value => return Err(format!("unsupported common module context: {value}")),
            };
            Ok(MetadataPropertyValue::ModuleContext(value))
        },
    )?;
    push_metadata_property(
        &mut result,
        object,
        "returnValuesReuse",
        MetadataKindPropertyName::ReturnValuesReuse,
        |value| {
            let value = match required_json_text(value, "returnValuesReuse")? {
                "DontUse" => MetadataReturnValuesReuse::DoNotUse,
                "DuringRequest" => MetadataReturnValuesReuse::DuringRequest,
                "DuringSession" => MetadataReturnValuesReuse::DuringSession,
                value => return Err(format!("unsupported return-values reuse mode: {value}")),
            };
            Ok(MetadataPropertyValue::ReturnValuesReuse(value))
        },
    )?;
    push_metadata_property(
        &mut result,
        object,
        "hierarchyType",
        MetadataKindPropertyName::HierarchyType,
        |value| {
            let value = match required_json_text(value, "hierarchyType")? {
                "HierarchyOfItems" | "HierarchyItemsOnly" => MetadataHierarchyType::Items,
                "HierarchyFoldersAndItems" | "FoldersAndItems" => {
                    MetadataHierarchyType::GroupsAndItems
                }
                value => return Err(format!("unsupported hierarchy type: {value}")),
            };
            Ok(MetadataPropertyValue::HierarchyType(value))
        },
    )?;
    push_metadata_property(
        &mut result,
        object,
        "periodicity",
        MetadataKindPropertyName::Periodicity,
        |value| {
            Ok(MetadataPropertyValue::Periodicity(
                parse_metadata_periodicity(required_json_text(value, "periodicity")?)?,
            ))
        },
    )?;
    push_metadata_property(
        &mut result,
        object,
        "registerType",
        MetadataKindPropertyName::RegisterType,
        |value| {
            let value = match required_json_text(value, "registerType")? {
                "Balance" | "Balances" | "Остатки" => MetadataRegisterKind::Balance,
                "Turnovers" | "Обороты" => MetadataRegisterKind::Turnovers,
                value => return Err(format!("unsupported register type: {value}")),
            };
            Ok(MetadataPropertyValue::RegisterKind(value))
        },
    )?;
    for (name, property) in [
        ("chartOfAccounts", MetadataKindPropertyName::ChartOfAccounts),
        (
            "chartOfCalculationTypes",
            MetadataKindPropertyName::ChartOfCalculationTypes,
        ),
        (
            "extDimensionTypes",
            MetadataKindPropertyName::ExtDimensionTypes,
        ),
        ("task", MetadataKindPropertyName::Task),
    ] {
        push_metadata_property(&mut result, object, name, property, |value| {
            Ok(MetadataPropertyValue::Object(
                MetadataObjectReference::new(required_json_text(value, name)?.to_string())
                    .map_err(|error| format!("invalid {name}: {error}"))?,
            ))
        })?;
    }
    for (name, property) in [
        ("accountingFlags", MetadataKindPropertyName::AccountingFlags),
        (
            "extDimensionAccountingFlags",
            MetadataKindPropertyName::ExtDimensionAccountingFlags,
        ),
    ] {
        push_metadata_property(&mut result, object, name, property, |value| {
            Ok(MetadataPropertyValue::Texts(parse_property_text_list(
                value, name,
            )?))
        })?;
    }
    push_metadata_property(
        &mut result,
        object,
        "dependenceOnCalculationTypes",
        MetadataKindPropertyName::DependenceOnCalculationTypes,
        |value| {
            let value = match required_json_text(value, "dependenceOnCalculationTypes")? {
                "OnActionPeriod" | "Depend" => MetadataCalculationDependence::OnActionPeriod,
                "DontUse" | "NotDependOnCalculationTypes" | "NoDependence" | "NotUsed" => {
                    MetadataCalculationDependence::DoNotUse
                }
                value => return Err(format!("unsupported calculation dependence: {value}")),
            };
            Ok(MetadataPropertyValue::CalculationDependence(value))
        },
    )?;
    for (name, property) in [
        (
            "baseCalculationTypes",
            MetadataKindPropertyName::BaseCalculationTypes,
        ),
        (
            "registeredDocuments",
            MetadataKindPropertyName::RegisteredDocuments,
        ),
        ("source", MetadataKindPropertyName::Source),
    ] {
        push_metadata_property(&mut result, object, name, property, |value| {
            Ok(MetadataPropertyValue::Objects(parse_object_references(
                Some(value),
            )?))
        })?;
    }
    push_metadata_property(
        &mut result,
        object,
        "addressing",
        MetadataKindPropertyName::Addressing,
        |value| {
            Ok(MetadataPropertyValue::Type(parse_metadata_type_expression(
                required_json_text(value, "addressing")?,
            )?))
        },
    )?;
    push_text_property(
        &mut result,
        object,
        "mainAddressingAttribute",
        MetadataKindPropertyName::MainAddressingAttribute,
    )?;
    push_metadata_property(
        &mut result,
        object,
        "methodName",
        MetadataKindPropertyName::MethodName,
        |value| {
            Ok(MetadataPropertyValue::Method(
                MetadataMethodReference::new(required_json_text(value, "methodName")?.to_string())
                    .map_err(|error| format!("invalid methodName: {error}"))?,
            ))
        },
    )?;
    push_text_property(
        &mut result,
        object,
        "description",
        MetadataKindPropertyName::Description,
    )?;
    push_metadata_property(
        &mut result,
        object,
        "key",
        MetadataKindPropertyName::Key,
        |value| {
            Ok(MetadataPropertyValue::JobKey(
                MetadataJobKey::new(required_json_text(value, "key")?.to_string())
                    .map_err(|error| format!("invalid key: {error}"))?,
            ))
        },
    )?;
    push_bool_property(&mut result, object, "use", MetadataKindPropertyName::Use)?;
    push_bool_property(
        &mut result,
        object,
        "predefined",
        MetadataKindPropertyName::Predefined,
    )?;
    push_metadata_property(
        &mut result,
        object,
        "event",
        MetadataKindPropertyName::Event,
        |value| {
            Ok(MetadataPropertyValue::Event(
                MetadataEventName::new(required_json_text(value, "event")?.to_string())
                    .map_err(|error| format!("invalid event: {error}"))?,
            ))
        },
    )?;
    push_metadata_property(
        &mut result,
        object,
        "handler",
        MetadataKindPropertyName::Handler,
        |value| {
            Ok(MetadataPropertyValue::Method(
                MetadataMethodReference::new(required_json_text(value, "handler")?.to_string())
                    .map_err(|error| format!("invalid handler: {error}"))?,
            ))
        },
    )?;
    if let Some(value) = object.get("rootURL").or_else(|| object.get("rootUrl")) {
        result.push(
            MetadataKindProperty::new(
                MetadataKindPropertyName::RootUrl,
                MetadataPropertyValue::UrlRoot(
                    MetadataUrlRoot::new(required_json_text(value, "rootURL")?.to_string())
                        .map_err(|error| format!("invalid rootURL: {error}"))?,
                ),
            )
            .map_err(|error| format!("invalid rootURL: {error}"))?,
        );
    }
    push_metadata_property(
        &mut result,
        object,
        "reuseSessions",
        MetadataKindPropertyName::ReuseSessions,
        |value| {
            let value = match required_json_text(value, "reuseSessions")? {
                "DontUse" => MetadataSessionReuse::DoNotUse,
                "AutoUse" | "Automatic" => MetadataSessionReuse::Automatic,
                "Use" => MetadataSessionReuse::Use,
                value => return Err(format!("unsupported session reuse mode: {value}")),
            };
            Ok(MetadataPropertyValue::SessionReuse(value))
        },
    )?;
    push_metadata_property(
        &mut result,
        object,
        "urlTemplates",
        MetadataKindPropertyName::UrlTemplates,
        |value| {
            Ok(MetadataPropertyValue::UrlTemplates(parse_url_templates(
                value,
            )?))
        },
    )?;
    push_metadata_property(
        &mut result,
        object,
        "namespace",
        MetadataKindPropertyName::Namespace,
        |value| {
            Ok(MetadataPropertyValue::ServiceNamespace(
                MetadataServiceNamespace::new(required_json_text(value, "namespace")?.to_string())
                    .map_err(|error| format!("invalid namespace: {error}"))?,
            ))
        },
    )?;
    push_metadata_property(
        &mut result,
        object,
        "operations",
        MetadataKindPropertyName::Operations,
        |value| {
            Ok(MetadataPropertyValue::ServiceOperations(
                parse_service_operations(value)?,
            ))
        },
    )?;
    Ok(result)
}

fn push_metadata_property(
    output: &mut Vec<MetadataKindProperty>,
    object: &Map<String, Value>,
    name: &str,
    property: MetadataKindPropertyName,
    parse: impl FnOnce(&Value) -> Result<MetadataPropertyValue, String>,
) -> Result<(), String> {
    if let Some(value) = object.get(name) {
        output.push(
            MetadataKindProperty::new(property, parse(value)?)
                .map_err(|error| format!("invalid {name}: {error}"))?,
        );
    }
    Ok(())
}

fn push_text_property(
    output: &mut Vec<MetadataKindProperty>,
    object: &Map<String, Value>,
    name: &str,
    property: MetadataKindPropertyName,
) -> Result<(), String> {
    push_metadata_property(output, object, name, property, |value| {
        Ok(MetadataPropertyValue::Text(
            MetadataPropertyText::new(required_json_text(value, name)?.to_string())
                .map_err(|error| format!("invalid {name}: {error}"))?,
        ))
    })
}

fn push_bool_property(
    output: &mut Vec<MetadataKindProperty>,
    object: &Map<String, Value>,
    name: &str,
    property: MetadataKindPropertyName,
) -> Result<(), String> {
    push_metadata_property(output, object, name, property, |value| {
        Ok(MetadataPropertyValue::Boolean(
            value
                .as_bool()
                .ok_or_else(|| format!("{name} must be boolean"))?,
        ))
    })
}

fn parse_metadata_periodicity(value: &str) -> Result<MetadataPeriodicity, String> {
    match value {
        "None" | "Nonperiodical" | "Непериодический" => {
            Ok(MetadataPeriodicity::Nonperiodical)
        }
        "Second" | "Секунда" => Ok(MetadataPeriodicity::Second),
        "Day" | "Daily" | "День" => Ok(MetadataPeriodicity::Day),
        "Month" | "Monthly" | "Месяц" => Ok(MetadataPeriodicity::Month),
        "Quarter" | "Quarterly" | "Квартал" => Ok(MetadataPeriodicity::Quarter),
        "Year" | "Yearly" | "Год" => Ok(MetadataPeriodicity::Year),
        "RecorderPosition" | "ПозицияРегистратора" => {
            Ok(MetadataPeriodicity::RecorderPosition)
        }
        _ => Err(format!("unsupported periodicity: {value}")),
    }
}

fn parse_type_expression_list(value: &Value) -> Result<Vec<MetadataTypeExpression>, String> {
    value
        .as_array()
        .ok_or_else(|| "metadata type set must be an array".to_string())?
        .iter()
        .map(|value| parse_metadata_type_expression(required_json_text(value, "type")?))
        .collect()
}

fn parse_property_text_list(
    value: &Value,
    label: &str,
) -> Result<Vec<MetadataPropertyText>, String> {
    value
        .as_array()
        .ok_or_else(|| format!("{label} must be an array"))?
        .iter()
        .map(|value| {
            MetadataPropertyText::new(required_json_text(value, label)?.to_string())
                .map_err(|error| format!("invalid {label}: {error}"))
        })
        .collect()
}

fn parse_object_references(value: Option<&Value>) -> Result<Vec<MetadataObjectReference>, String> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let values = value
        .as_array()
        .map_or_else(|| vec![value], |values| values.iter().collect());
    values
        .into_iter()
        .map(|value| {
            MetadataObjectReference::new(required_json_text(value, "object reference")?.to_string())
                .map_err(|error| format!("invalid object reference: {error}"))
        })
        .collect()
}

fn parse_url_templates(value: &Value) -> Result<Vec<MetadataUrlTemplateDefinition>, String> {
    value
        .as_object()
        .ok_or_else(|| "urlTemplates must be an object".to_string())?
        .iter()
        .map(|(name, value)| {
            let object = value
                .as_object()
                .ok_or_else(|| "URL template must be an object".to_string())?;
            reject_unknown_fields(object, &["template", "methods"], "URL template")?;
            let template = MetadataUrlTemplatePath::new(
                first_object_string(object, &["template"])
                    .unwrap_or("/")
                    .to_string(),
            )
            .map_err(|error| format!("invalid URL template: {error}"))?;
            let methods = object
                .get("methods")
                .map(|value| {
                    value
                        .as_object()
                        .ok_or_else(|| "URL template methods must be an object".to_string())?
                        .iter()
                        .map(|(method_name, method)| {
                            Ok(MetadataHttpMethodDefinition::new(
                                MetadataHttpMethodName::new(method_name.clone())
                                    .map_err(|error| format!("invalid method name: {error}"))?,
                                MetadataHttpMethodName::new(
                                    required_json_text(method, "HTTP method")?.to_string(),
                                )
                                .map_err(|error| format!("invalid HTTP method: {error}"))?,
                            ))
                        })
                        .collect::<Result<Vec<_>, String>>()
                })
                .transpose()?
                .unwrap_or_default();
            Ok(MetadataUrlTemplateDefinition::new(
                MetadataChildName::new(name.clone())
                    .map_err(|error| format!("invalid URL template name: {error}"))?,
                template,
                methods,
            ))
        })
        .collect()
}

fn parse_service_operations(
    value: &Value,
) -> Result<Vec<MetadataServiceOperationDefinition>, String> {
    value
        .as_object()
        .ok_or_else(|| "operations must be an object".to_string())?
        .iter()
        .map(|(name, value)| {
            let object = value
                .as_object()
                .ok_or_else(|| "service operation must be an object".to_string())?;
            reject_unknown_fields(
                object,
                &[
                    "returnType",
                    "nillable",
                    "transactioned",
                    "handler",
                    "parameters",
                ],
                "service operation",
            )?;
            let operation_name = MetadataServiceOperationName::new(name.clone())
                .map_err(|error| format!("invalid service operation name: {error}"))?;
            let handler = MetadataMethodReference::new(
                first_object_string(object, &["handler"])
                    .unwrap_or(name)
                    .to_string(),
            )
            .map_err(|error| format!("invalid service operation handler: {error}"))?;
            let parameters = object
                .get("parameters")
                .map(parse_service_parameters)
                .transpose()?
                .unwrap_or_default();
            Ok(MetadataServiceOperationDefinition::new(
                operation_name,
                MetadataServiceTypeName::new(
                    first_object_string(object, &["returnType"])
                        .unwrap_or("xs:string")
                        .to_string(),
                )
                .map_err(|error| format!("invalid service return type: {error}"))?,
                handler,
                parameters,
            ))
        })
        .collect()
}

fn parse_service_parameters(
    value: &Value,
) -> Result<Vec<MetadataServiceParameterDefinition>, String> {
    value
        .as_object()
        .ok_or_else(|| "service parameters must be an object".to_string())?
        .iter()
        .map(|(name, value)| {
            let value_type = value
                .as_str()
                .or_else(|| {
                    value
                        .as_object()
                        .and_then(|value| value.get("type"))
                        .and_then(Value::as_str)
                })
                .ok_or_else(|| "service parameter requires a type".to_string())?;
            Ok(MetadataServiceParameterDefinition::new(
                MetadataServiceParameterName::new(name.clone())
                    .map_err(|error| format!("invalid service parameter name: {error}"))?,
                MetadataServiceTypeName::new(value_type.to_string())
                    .map_err(|error| format!("invalid service parameter type: {error}"))?,
            ))
        })
        .collect()
}

fn optional_semantic_text<T>(
    value: Option<&Value>,
    constructor: impl Fn(String) -> Result<T, SemanticValueError>,
    label: &str,
) -> Result<Option<T>, String> {
    value
        .map(|value| {
            constructor(required_json_text(value, label)?.to_string())
                .map_err(|error| format!("invalid {label}: {error}"))
        })
        .transpose()
}

fn required_json_text<'a>(value: &'a Value, label: &str) -> Result<&'a str, String> {
    value
        .as_str()
        .ok_or_else(|| format!("{label} must be text"))
}

fn json_scalar_text(value: &Value) -> Result<String, String> {
    match value {
        Value::String(value) => Ok(value.clone()),
        Value::Bool(value) => Ok(value.to_string()),
        Value::Number(value) => Ok(value.to_string()),
        _ => Err("metadata field type must be scalar".to_string()),
    }
}

fn parse_optional_fill_checking(
    value: Option<&Value>,
) -> Result<Option<MetadataFillChecking>, String> {
    value
        .map(|value| match required_json_text(value, "fillChecking")? {
            "DontCheck" | "НеПроверять" => Ok(MetadataFillChecking::None),
            "ShowWarning" | "Warning" => Ok(MetadataFillChecking::Warning),
            "ShowError" | "Error" | "Ошибка" => Ok(MetadataFillChecking::Error),
            value => Err(format!("unsupported fillChecking mode: {value}")),
        })
        .transpose()
}

fn parse_optional_indexing(value: Option<&Value>) -> Result<Option<MetadataIndexing>, String> {
    value
        .map(|value| match required_json_text(value, "indexing")? {
            "DontIndex" | "НеИндексировать" => Ok(MetadataIndexing::None),
            "Index" | "Индексировать" => Ok(MetadataIndexing::Index),
            "IndexWithAdditionalOrder" | "ИндексироватьСДопУпорядочиванием" => {
                Ok(MetadataIndexing::IndexWithAdditionalOrder)
            }
            value => Err(format!("unsupported indexing mode: {value}")),
        })
        .transpose()
}

fn parse_optional_choice_history(
    value: Option<&Value>,
) -> Result<Option<MetadataChoiceHistory>, String> {
    value
        .map(
            |value| match required_json_text(value, "choiceHistoryOnInput")? {
                "Auto" | "Automatic" => Ok(MetadataChoiceHistory::Automatic),
                "Use" => Ok(MetadataChoiceHistory::Use),
                "DontUse" => Ok(MetadataChoiceHistory::DoNotUse),
                value => Err(format!("unsupported choice history mode: {value}")),
            },
        )
        .transpose()
}

fn read_managed_form_definition(
    args: &Map<String, Value>,
    names: &[&str],
    label: &str,
    context: &WorkspaceContext,
) -> Result<ManagedFormDefinition, String> {
    parse_managed_form_value(&read_json_value(args, names, label, context)?)
}

fn parse_managed_form_value(value: &Value) -> Result<ManagedFormDefinition, String> {
    let object = value
        .as_object()
        .ok_or_else(|| "managed form definition must be an object".to_string())?;
    reject_unknown_fields(
        object,
        &[
            "title",
            "attributes",
            "commands",
            "parameters",
            "elements",
            "events",
            "formEvents",
        ],
        "managed form definition",
    )?;
    Ok(ManagedFormDefinition::new(
        optional_semantic_text(object.get("title"), SynonymText::new, "form title")?,
        parse_form_array(
            object.get("attributes"),
            parse_form_attribute,
            "form attributes",
        )?,
        parse_form_array(object.get("commands"), parse_form_command, "form commands")?,
        parse_form_array(
            object.get("parameters"),
            parse_form_parameter,
            "form parameters",
        )?,
        parse_form_array(object.get("elements"), parse_form_element, "form elements")?,
        parse_form_events(object.get("events").or_else(|| object.get("formEvents")))?,
    ))
}

fn parse_form_array<T>(
    value: Option<&Value>,
    parse: impl Fn(&Value) -> Result<T, String>,
    label: &str,
) -> Result<Vec<T>, String> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    value
        .as_array()
        .ok_or_else(|| format!("{label} must be an array"))?
        .iter()
        .map(parse)
        .collect()
}

fn parse_form_attribute(value: &Value) -> Result<FormAttributeDefinition, String> {
    let object = value
        .as_object()
        .ok_or_else(|| "form attribute must be an object".to_string())?;
    reject_unknown_fields(
        object,
        &["name", "type", "title", "main", "columns"],
        "form attribute",
    )?;
    let name = FormAttributeName::new(
        first_object_string(object, &["name"])
            .ok_or_else(|| "form attribute requires name".to_string())?
            .to_string(),
    )
    .map_err(|error| format!("invalid form attribute name: {error}"))?;
    Ok(FormAttributeDefinition::new(name)
        .with_value_type(
            first_object_string(object, &["type"])
                .map(parse_metadata_value_type)
                .transpose()?,
        )
        .with_title(optional_semantic_text(
            object.get("title"),
            SynonymText::new,
            "form attribute title",
        )?)
        .as_main(object.get("main").and_then(Value::as_bool).unwrap_or(false))
        .with_columns(parse_form_array(
            object.get("columns"),
            parse_form_attribute,
            "form attribute columns",
        )?))
}

fn parse_form_command(value: &Value) -> Result<FormCommandDefinition, String> {
    let object = value
        .as_object()
        .ok_or_else(|| "form command must be an object".to_string())?;
    reject_unknown_fields(object, &["name", "title", "action"], "form command")?;
    Ok(FormCommandDefinition::new(
        FormCommandName::new(
            first_object_string(object, &["name"])
                .ok_or_else(|| "form command requires name".to_string())?
                .to_string(),
        )
        .map_err(|error| format!("invalid form command name: {error}"))?,
    )
    .with_title(optional_semantic_text(
        object.get("title"),
        SynonymText::new,
        "form command title",
    )?)
    .with_action(optional_semantic_text(
        object.get("action"),
        FormHandlerName::new,
        "form command action",
    )?))
}

fn parse_form_parameter(value: &Value) -> Result<FormParameterDefinition, String> {
    let object = value
        .as_object()
        .ok_or_else(|| "form parameter must be an object".to_string())?;
    reject_unknown_fields(object, &["name", "type"], "form parameter")?;
    Ok(FormParameterDefinition::new(
        FormParameterName::new(
            first_object_string(object, &["name"])
                .ok_or_else(|| "form parameter requires name".to_string())?
                .to_string(),
        )
        .map_err(|error| format!("invalid form parameter name: {error}"))?,
    )
    .with_value_type(
        first_object_string(object, &["type"])
            .map(parse_metadata_value_type)
            .transpose()?,
    ))
}

fn parse_form_element(value: &Value) -> Result<FormElementDefinition, String> {
    let object = value
        .as_object()
        .ok_or_else(|| "form element must be an object".to_string())?;
    const DISCRIMINATORS: &[(&str, FormElementType)] = &[
        ("input", FormElementType::Input),
        ("group", FormElementType::Group),
        ("table", FormElementType::Table),
        ("button", FormElementType::Button),
        ("commandBar", FormElementType::CommandBar),
        ("labelField", FormElementType::Label),
        ("picture", FormElementType::Picture),
        ("calendar", FormElementType::Calendar),
        ("pages", FormElementType::Pages),
        ("page", FormElementType::Page),
    ];
    reject_unknown_fields(
        object,
        &[
            "name",
            "type",
            "input",
            "group",
            "table",
            "button",
            "commandBar",
            "labelField",
            "picture",
            "calendar",
            "pages",
            "page",
            "title",
            "path",
            "dataPath",
            "commandName",
            "command",
            "visible",
            "enabled",
            "readOnly",
            "events",
            "children",
        ],
        "form element",
    )?;
    let explicit_type = first_object_string(object, &["type"])
        .map(|value| match value {
            "input" => Ok(FormElementType::Input),
            "group" => Ok(FormElementType::Group),
            "table" => Ok(FormElementType::Table),
            "button" => Ok(FormElementType::Button),
            "commandBar" => Ok(FormElementType::CommandBar),
            "label" | "labelField" => Ok(FormElementType::Label),
            "picture" => Ok(FormElementType::Picture),
            "calendar" => Ok(FormElementType::Calendar),
            "pages" => Ok(FormElementType::Pages),
            "page" => Ok(FormElementType::Page),
            value => Err(format!("unknown form element type: {value}")),
        })
        .transpose()?;
    let discriminators = DISCRIMINATORS
        .iter()
        .filter_map(|(name, kind)| object.get(*name).map(|value| (*name, *kind, value)))
        .collect::<Vec<_>>();
    let (kind, name) = match (explicit_type, discriminators.as_slice()) {
        (Some(kind), []) => (
            kind,
            first_object_string(object, &["name"])
                .ok_or_else(|| "typed form element requires name".to_string())?,
        ),
        (None, [(_, kind, value)]) => (
            *kind,
            value
                .as_str()
                .ok_or_else(|| "form element discriminator value must be text".to_string())?,
        ),
        (Some(_), _) | (None, [_, ..]) => {
            return Err("form element must use exactly one type discriminator".to_string())
        }
        (None, []) => return Err("form element requires a type discriminator".to_string()),
    };
    Ok(FormElementDefinition::new(
        FormElementName::new(name.to_string())
            .map_err(|error| format!("invalid form element name: {error}"))?,
        kind,
    )
    .with_title(optional_semantic_text(
        object.get("title"),
        SynonymText::new,
        "form element title",
    )?)
    .with_data_path(optional_semantic_text(
        first_object_value(object, &["path", "dataPath"]),
        FormElementPath::new,
        "form element path",
    )?)
    .with_command(optional_semantic_text(
        first_object_value(object, &["commandName", "command"]),
        FormCommandName::new,
        "form element command",
    )?)
    .visible(
        object
            .get("visible")
            .and_then(Value::as_bool)
            .unwrap_or(true),
    )
    .enabled(
        object
            .get("enabled")
            .and_then(Value::as_bool)
            .unwrap_or(true),
    )
    .read_only(
        object
            .get("readOnly")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    )
    .with_events(parse_form_events(object.get("events"))?)
    .with_children(parse_form_array(
        object.get("children"),
        parse_form_element,
        "form element children",
    )?))
}

fn parse_form_events(value: Option<&Value>) -> Result<Vec<FormEventBinding>, String> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    if let Some(object) = value.as_object() {
        return object
            .iter()
            .map(|(event, handler)| {
                Ok(FormEventBinding::new(
                    FormEventName::new(event.clone())
                        .map_err(|error| format!("invalid form event: {error}"))?,
                    FormHandlerName::new(
                        required_json_text(handler, "form event handler")?.to_string(),
                    )
                    .map_err(|error| format!("invalid form event handler: {error}"))?,
                ))
            })
            .collect();
    }
    value
        .as_array()
        .ok_or_else(|| "form events must be an object or array".to_string())?
        .iter()
        .map(|value| {
            let object = value
                .as_object()
                .ok_or_else(|| "form event binding must be an object".to_string())?;
            reject_unknown_fields(object, &["event", "name", "handler"], "form event")?;
            Ok(FormEventBinding::new(
                FormEventName::new(
                    first_object_string(object, &["event", "name"])
                        .ok_or_else(|| "form event requires event".to_string())?
                        .to_string(),
                )
                .map_err(|error| format!("invalid form event: {error}"))?,
                FormHandlerName::new(
                    first_object_string(object, &["handler"])
                        .ok_or_else(|| "form event requires handler".to_string())?
                        .to_string(),
                )
                .map_err(|error| format!("invalid form event handler: {error}"))?,
            ))
        })
        .collect()
}

fn parse_metadata_value_type(value: &str) -> Result<MetadataValueType, String> {
    let base = value.split(['.', '(']).next().unwrap_or(value);
    match base {
        "String" => Ok(MetadataValueType::String),
        "Number" => Ok(MetadataValueType::Number),
        "Boolean" => Ok(MetadataValueType::Boolean),
        "Date" => Ok(MetadataValueType::Date),
        "UUID" | "Uuid" => Ok(MetadataValueType::Uuid),
        "Binary" | "BinaryData" => Ok(MetadataValueType::Binary),
        "ValueStorage" => Ok(MetadataValueType::ValueStorage),
        "Any" => Ok(MetadataValueType::Any),
        "CatalogRef" => Ok(MetadataValueType::CatalogReference),
        "DocumentRef" => Ok(MetadataValueType::DocumentReference),
        "EnumRef" => Ok(MetadataValueType::EnumReference),
        "DefinedType" => Ok(MetadataValueType::DefinedType),
        _ => Err(format!("unsupported semantic value type: {value}")),
    }
}

fn read_role_definition(
    args: &Map<String, Value>,
    names: &[&str],
    label: &str,
    context: &WorkspaceContext,
) -> Result<RoleDefinition, String> {
    parse_role_definition_value(&read_json_value(args, names, label, context)?)
}

fn parse_role_definition_value(value: &Value) -> Result<RoleDefinition, String> {
    let object = value
        .as_object()
        .ok_or_else(|| "role definition must be an object".to_string())?;
    reject_unknown_fields(
        object,
        &[
            "name",
            "synonym",
            "comment",
            "setForNewObjects",
            "setForAttributesByDefault",
            "independentRightsOfChildObjects",
            "objects",
            "rights",
            "templates",
        ],
        "role definition",
    )?;
    let name = first_object_string(object, &["name"])
        .ok_or_else(|| "role definition requires name".to_string())?;
    let mut definition = RoleDefinition::new(
        RoleName::new(name.to_string()).map_err(|error| format!("invalid role name: {error}"))?,
    )
    .with_synonym(
        first_object_string(object, &["synonym"])
            .map(|value| SynonymText::new(value.to_string()))
            .transpose()
            .map_err(|error| format!("invalid role synonym: {error}"))?,
    )
    .with_comment(
        first_object_string(object, &["comment"])
            .map(|value| CommentText::new(value.to_string()))
            .transpose()
            .map_err(|error| format!("invalid role comment: {error}"))?,
    )
    .set_for_new_objects(
        first_object_value(object, &["setForNewObjects"])
            .map(|value| {
                value
                    .as_bool()
                    .ok_or_else(|| "setForNewObjects must be boolean".to_string())
            })
            .transpose()?
            .unwrap_or(false),
    )
    .set_for_attributes_by_default(
        first_object_value(object, &["setForAttributesByDefault"])
            .map(|value| {
                value
                    .as_bool()
                    .ok_or_else(|| "setForAttributesByDefault must be boolean".to_string())
            })
            .transpose()?
            .unwrap_or(true),
    )
    .independent_rights_of_child_objects(
        first_object_value(object, &["independentRightsOfChildObjects"])
            .map(|value| {
                value
                    .as_bool()
                    .ok_or_else(|| "independentRightsOfChildObjects must be boolean".to_string())
            })
            .transpose()?
            .unwrap_or(false),
    );
    let objects = first_object_value(object, &["objects", "rights"])
        .map(|value| {
            value
                .as_array()
                .ok_or_else(|| "role objects must be an array".to_string())?
                .iter()
                .map(parse_role_object_rights)
                .collect::<Result<Vec<_>, String>>()
        })
        .transpose()?
        .unwrap_or_default();
    let templates = first_object_value(object, &["templates"])
        .map(|value| {
            value
                .as_array()
                .ok_or_else(|| "role templates must be an array".to_string())?
                .iter()
                .map(|value| {
                    let template = value
                        .as_object()
                        .ok_or_else(|| "role template must be an object".to_string())?;
                    reject_unknown_fields(template, &["name", "condition"], "role template")?;
                    Ok(RoleRestrictionTemplate::new(
                        RoleTemplateName::new(
                            first_object_string(template, &["name"])
                                .ok_or_else(|| "role template requires name".to_string())?
                                .to_string(),
                        )
                        .map_err(|error| format!("invalid role template name: {error}"))?,
                        RoleRestrictionText::new(
                            first_object_string(template, &["condition"])
                                .ok_or_else(|| "role template requires condition".to_string())?
                                .to_string(),
                        )
                        .map_err(|error| format!("invalid role template condition: {error}"))?,
                    ))
                })
                .collect::<Result<Vec<_>, String>>()
        })
        .transpose()?
        .unwrap_or_default();
    definition = definition.with_objects(objects).with_templates(templates);
    Ok(definition)
}

fn parse_role_object_rights(value: &Value) -> Result<RoleObjectRights, String> {
    if let Some(text) = value.as_str() {
        let (object, rights) = text
            .split_once(':')
            .ok_or_else(|| "role object string must contain ':'".to_string())?;
        let object = normalize_role_object_reference(object.trim());
        let rights = parse_role_right_tokens(&object, rights)?;
        return Ok(RoleObjectRights::new(
            RoleObjectReference::new(object)
                .map_err(|error| format!("invalid role object reference: {error}"))?,
            rights,
        ));
    }
    let object = value
        .as_object()
        .ok_or_else(|| "role object rights must be a string or object".to_string())?;
    reject_unknown_fields(
        object,
        &["name", "preset", "rights", "rls"],
        "role object rights",
    )?;
    let object_name = normalize_role_object_reference(
        first_object_string(object, &["name"])
            .ok_or_else(|| "role object rights requires name".to_string())?,
    );
    let mut assignments = Vec::new();
    if let Some(preset) = first_object_string(object, &["preset"]) {
        append_unique_role_rights(
            &mut assignments,
            parse_role_right_tokens(&object_name, preset)?,
        );
    }
    if let Some(rights) = first_object_value(object, &["rights"]) {
        match rights {
            Value::Array(values) => {
                for value in values {
                    let name = value
                        .as_str()
                        .ok_or_else(|| "role right array entries must be strings".to_string())?;
                    append_unique_role_rights(
                        &mut assignments,
                        vec![RoleRightAssignment::new(
                            parse_role_right(name)?,
                            RoleRightState::Allow,
                        )],
                    );
                }
            }
            Value::Object(values) => {
                for (name, state) in values {
                    let state = match state {
                        Value::Bool(true) => RoleRightState::Allow,
                        Value::Bool(false) => RoleRightState::Deny,
                        Value::String(value) if value.eq_ignore_ascii_case("true") => {
                            RoleRightState::Allow
                        }
                        Value::String(value) if value.eq_ignore_ascii_case("false") => {
                            RoleRightState::Deny
                        }
                        _ => return Err(format!("role right {name:?} state must be boolean")),
                    };
                    let right = parse_role_right(name)?;
                    if let Some(existing) = assignments
                        .iter_mut()
                        .find(|assignment| assignment.right() == right)
                    {
                        *existing = RoleRightAssignment::new(right, state);
                    } else {
                        assignments.push(RoleRightAssignment::new(right, state));
                    }
                }
            }
            _ => return Err("role rights must be an array or object".to_string()),
        }
    }
    let restriction = first_object_value(object, &["rls"])
        .map(|value| {
            let values = value
                .as_object()
                .ok_or_else(|| "role rls must be an object".to_string())?;
            let mut unique = Vec::<&str>::new();
            for (right, condition) in values {
                let parsed = parse_role_right(right)?;
                if !assignments
                    .iter()
                    .any(|assignment| assignment.right() == parsed)
                {
                    return Err(format!("role rls references unassigned right {right:?}"));
                }
                let condition = condition
                    .as_str()
                    .ok_or_else(|| "role rls conditions must be strings".to_string())?;
                if !unique.contains(&condition) {
                    unique.push(condition);
                }
            }
            if unique.len() > 1 {
                return Err(
                    "closed role DTO requires one semantic restriction per object".to_string(),
                );
            }
            unique
                .first()
                .map(|value| RoleRestrictionText::new((*value).to_string()))
                .transpose()
                .map_err(|error| format!("invalid role restriction: {error}"))
        })
        .transpose()?
        .flatten();
    Ok(RoleObjectRights::new(
        RoleObjectReference::new(object_name)
            .map_err(|error| format!("invalid role object reference: {error}"))?,
        assignments,
    )
    .with_restriction(restriction))
}

fn append_unique_role_rights(
    target: &mut Vec<RoleRightAssignment>,
    additions: Vec<RoleRightAssignment>,
) {
    for assignment in additions {
        if !target
            .iter()
            .any(|existing| existing.right() == assignment.right())
        {
            target.push(assignment);
        }
    }
}

fn parse_role_right_tokens(object: &str, value: &str) -> Result<Vec<RoleRightAssignment>, String> {
    let mut assignments = Vec::new();
    for token in value
        .split(|character: char| character.is_whitespace() || character == ',')
        .filter(|token| !token.is_empty())
    {
        let rights = if let Some(preset) = token.strip_prefix('@') {
            role_preset(object.split('.').next().unwrap_or(object), preset)?
        } else {
            vec![parse_role_right(token)?]
        };
        for right in rights {
            append_unique_role_rights(
                &mut assignments,
                vec![RoleRightAssignment::new(right, RoleRightState::Allow)],
            );
        }
    }
    Ok(assignments)
}

fn role_preset(object_type: &str, preset: &str) -> Result<Vec<RoleRight>, String> {
    use RoleRight::*;
    match (preset, object_type) {
        (
            "view",
            "Catalog"
            | "ExchangePlan"
            | "Document"
            | "ChartOfAccounts"
            | "ChartOfCharacteristicTypes"
            | "ChartOfCalculationTypes"
            | "BusinessProcess"
            | "Task",
        ) => Ok(vec![Read, View, InputByString]),
        (
            "view",
            "InformationRegister"
            | "AccumulationRegister"
            | "AccountingRegister"
            | "CalculationRegister"
            | "Constant"
            | "DocumentJournal",
        ) => Ok(vec![Read, View]),
        ("view", "CommonForm" | "CommonCommand" | "Subsystem" | "FilterCriterion") => {
            Ok(vec![View])
        }
        ("view", "DataProcessor" | "Report") => Ok(vec![Use, View]),
        ("view", "Configuration") => Ok(vec![
            ThinClient,
            WebClient,
            Output,
            SaveUserData,
            MainWindowModeNormal,
        ]),
        (
            "edit",
            "Catalog"
            | "ExchangePlan"
            | "ChartOfAccounts"
            | "ChartOfCharacteristicTypes"
            | "ChartOfCalculationTypes",
        ) => Ok(vec![
            Read,
            Insert,
            Update,
            Delete,
            View,
            Edit,
            InputByString,
            InteractiveInsert,
            InteractiveSetDeletionMark,
            InteractiveClearDeletionMark,
        ]),
        ("edit", "Document") => Ok(vec![
            Read,
            Insert,
            Update,
            Delete,
            View,
            Edit,
            InputByString,
            Posting,
            UndoPosting,
            InteractiveInsert,
            InteractiveSetDeletionMark,
            InteractiveClearDeletionMark,
            InteractivePosting,
            InteractivePostingRegular,
            InteractiveUndoPosting,
            InteractiveChangeOfPosted,
        ]),
        (
            "edit",
            "InformationRegister" | "AccumulationRegister" | "AccountingRegister" | "Constant",
        ) => Ok(vec![Read, Update, View, Edit]),
        ("edit", "SessionParameter") => Ok(vec![Get, Set]),
        ("edit", "CommonAttribute") => Ok(vec![View, Edit]),
        ("view", "SessionParameter") => Ok(vec![Get]),
        ("view", "CommonAttribute") => Ok(vec![View]),
        ("view", "Sequence") => Ok(vec![Read]),
        ("edit", "Sequence") => Ok(vec![Read, Update]),
        ("edit", "DocumentJournal") => Ok(vec![Read, View]),
        ("view" | "edit", _) => Err(format!(
            "role preset @{preset} is not defined for object type {object_type}"
        )),
        _ => Err(format!("unknown role preset @{preset}")),
    }
}

fn parse_role_right(value: &str) -> Result<RoleRight, String> {
    match value {
        "Read" | "Чтение" => Ok(RoleRight::Read),
        "Insert" | "Добавление" => Ok(RoleRight::Insert),
        "Update" | "Изменение" => Ok(RoleRight::Update),
        "Delete" | "Удаление" => Ok(RoleRight::Delete),
        "View" | "Просмотр" => Ok(RoleRight::View),
        "Edit" | "Редактирование" => Ok(RoleRight::Edit),
        "InputByString" | "ВводПоСтроке" => Ok(RoleRight::InputByString),
        "InteractiveInsert" => Ok(RoleRight::InteractiveInsert),
        "InteractiveUpdate" => Ok(RoleRight::InteractiveUpdate),
        "InteractiveDelete" => Ok(RoleRight::InteractiveDelete),
        "InteractiveDeleteMarked" => Ok(RoleRight::InteractiveDeleteMarked),
        "InteractiveSetDeletionMark" => Ok(RoleRight::InteractiveSetDeletionMark),
        "InteractiveClearDeletionMark" => Ok(RoleRight::InteractiveClearDeletionMark),
        "Posting" | "Проведение" => Ok(RoleRight::Posting),
        "UndoPosting" | "ОтменаПроведения" => Ok(RoleRight::UndoPosting),
        "InteractivePosting" => Ok(RoleRight::InteractivePosting),
        "InteractivePostingRegular" => Ok(RoleRight::InteractivePostingRegular),
        "InteractiveUndoPosting" => Ok(RoleRight::InteractiveUndoPosting),
        "InteractiveChangeOfPosted" => Ok(RoleRight::InteractiveChangeOfPosted),
        "Use" | "Использование" => Ok(RoleRight::Use),
        "Execute" => Ok(RoleRight::Execute),
        "Get" => Ok(RoleRight::Get),
        "Set" => Ok(RoleRight::Set),
        "Administration" => Ok(RoleRight::Administration),
        "DataAdministration" => Ok(RoleRight::DataAdministration),
        "ConfigurationAdministration" => Ok(RoleRight::ConfigurationAdministration),
        "ThinClient" => Ok(RoleRight::ThinClient),
        "WebClient" => Ok(RoleRight::WebClient),
        "MobileClient" => Ok(RoleRight::MobileClient),
        "Output" => Ok(RoleRight::Output),
        "SaveUserData" => Ok(RoleRight::SaveUserData),
        "MainWindowModeNormal" => Ok(RoleRight::MainWindowModeNormal),
        _ => Err(format!("unsupported role right {value:?}")),
    }
}

fn normalize_role_object_reference(value: &str) -> String {
    value
        .split('.')
        .map(|part| match part {
            "Справочник" => "Catalog",
            "Документ" => "Document",
            "РегистрСведений" => "InformationRegister",
            "РегистрНакопления" => "AccumulationRegister",
            "РегистрБухгалтерии" => "AccountingRegister",
            "РегистрРасчета" => "CalculationRegister",
            "Константа" => "Constant",
            "ПланСчетов" => "ChartOfAccounts",
            "ПланВидовХарактеристик" => "ChartOfCharacteristicTypes",
            "ПланВидовРасчета" => "ChartOfCalculationTypes",
            "ПланОбмена" => "ExchangePlan",
            "БизнесПроцесс" => "BusinessProcess",
            "Задача" => "Task",
            "Обработка" => "DataProcessor",
            "Отчет" => "Report",
            "ОбщаяФорма" => "CommonForm",
            "ОбщаяКоманда" => "CommonCommand",
            "Подсистема" => "Subsystem",
            "КритерийОтбора" => "FilterCriterion",
            "ЖурналДокументов" => "DocumentJournal",
            "Последовательность" => "Sequence",
            "ВебСервис" => "WebService",
            "HTTPСервис" => "HTTPService",
            "СервисИнтеграции" => "IntegrationService",
            "ПараметрСеанса" => "SessionParameter",
            "ОбщийРеквизит" => "CommonAttribute",
            "Конфигурация" => "Configuration",
            "Перечисление" => "Enum",
            "Реквизит" => "Attribute",
            "СтандартныйРеквизит" => "StandardAttribute",
            "ТабличнаяЧасть" => "TabularSection",
            "Измерение" => "Dimension",
            "Ресурс" => "Resource",
            "Команда" => "Command",
            "РеквизитАдресации" => "AddressingAttribute",
            part => part,
        })
        .collect::<Vec<_>>()
        .join(".")
}

fn read_spreadsheet_document(
    args: &Map<String, Value>,
    names: &[&str],
    label: &str,
    context: &WorkspaceContext,
) -> Result<SpreadsheetDocument, String> {
    parse_spreadsheet_document_value(&read_json_value(args, names, label, context)?)
}

fn parse_spreadsheet_document_value(value: &Value) -> Result<SpreadsheetDocument, String> {
    let object = value
        .as_object()
        .ok_or_else(|| "spreadsheet definition must be an object".to_string())?;
    reject_unknown_fields(
        object,
        &[
            "columns",
            "defaultWidth",
            "page",
            "fonts",
            "styles",
            "columnWidths",
            "areas",
        ],
        "spreadsheet definition",
    )?;
    let columns = required_positive_u32(object, "columns", "spreadsheet column count")?;
    let page = first_object_value(object, &["page"])
        .map(|value| {
            let value = value
                .as_str()
                .ok_or_else(|| "spreadsheet page must be a string".to_string())?;
            match value {
                "A4-portrait" => Ok(SpreadsheetPageOrientation::Portrait),
                "A4-landscape" => Ok(SpreadsheetPageOrientation::Landscape),
                _ => Err(format!("unsupported spreadsheet page {value:?}")),
            }
        })
        .transpose()?;
    let mut default_width = first_object_value(object, &["defaultWidth"])
        .map(|value| positive_u32(value, "spreadsheet default width"))
        .transpose()?
        .unwrap_or(10);
    let raw_widths =
        parse_spreadsheet_width_rules(first_object_value(object, &["columnWidths"]), columns)?;
    if let Some(target) = match page {
        Some(SpreadsheetPageOrientation::Portrait) => Some(540_u32),
        Some(SpreadsheetPageOrientation::Landscape) => Some(780_u32),
        None => None,
    } {
        let mut units = 0.0_f64;
        let mut absolute = 0_u32;
        let mut specified = Vec::new();
        for (column, width) in &raw_widths {
            if !specified.contains(column) {
                specified.push(*column);
            }
            match width {
                SpreadsheetWidthRule::Absolute(value) => {
                    absolute = absolute.saturating_add(*value);
                }
                SpreadsheetWidthRule::Relative(value) => units += value,
            }
        }
        units += f64::from(columns.saturating_sub(specified.len() as u32));
        if units > 0.0 && target > absolute {
            default_width = (f64::from(target - absolute) / units).round() as u32;
            if default_width == 0 {
                return Err("spreadsheet page calculation produced zero width".to_string());
            }
        }
    }
    let column_widths = raw_widths
        .into_iter()
        .map(|(column, width)| {
            let width = match width {
                SpreadsheetWidthRule::Absolute(value) => value,
                SpreadsheetWidthRule::Relative(value) => {
                    (value * f64::from(default_width)).round() as u32
                }
            };
            SpreadsheetColumnWidth::new(column, width)
                .map_err(|error| format!("invalid spreadsheet column width: {error}"))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let fonts = parse_spreadsheet_fonts(first_object_value(object, &["fonts"]))?;
    let styles = parse_spreadsheet_styles(first_object_value(object, &["styles"]))?;
    let areas = first_object_value(object, &["areas"])
        .ok_or_else(|| "spreadsheet areas are required".to_string())?
        .as_array()
        .ok_or_else(|| "spreadsheet areas must be an array".to_string())?
        .iter()
        .map(parse_spreadsheet_area)
        .collect::<Result<Vec<_>, String>>()?;
    SpreadsheetDocument::new(areas)
        .and_then(|document| document.with_column_count(columns))
        .and_then(|document| document.with_default_width(default_width))
        .map(|document| {
            document
                .with_page(page)
                .with_fonts(fonts)
                .with_styles(styles)
                .with_column_widths(column_widths)
        })
        .map_err(|error| format!("invalid spreadsheet document: {error}"))
}

enum SpreadsheetWidthRule {
    Absolute(u32),
    Relative(f64),
}

fn parse_spreadsheet_width_rules(
    value: Option<&Value>,
    columns: u32,
) -> Result<Vec<(u32, SpreadsheetWidthRule)>, String> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let object = value
        .as_object()
        .ok_or_else(|| "spreadsheet columnWidths must be an object".to_string())?;
    let mut result = Vec::new();
    for (specification, value) in object {
        let width = match value {
            Value::Number(value) => SpreadsheetWidthRule::Absolute(
                value
                    .as_u64()
                    .and_then(|value| u32::try_from(value).ok())
                    .filter(|value| *value > 0)
                    .ok_or_else(|| "spreadsheet width must be a positive integer".to_string())?,
            ),
            Value::String(value) if value.ends_with('x') => {
                let multiplier = value[..value.len() - 1]
                    .parse::<f64>()
                    .map_err(|_| format!("invalid relative spreadsheet width {value:?}"))?;
                if !multiplier.is_finite() || multiplier <= 0.0 {
                    return Err(format!("invalid relative spreadsheet width {value:?}"));
                }
                SpreadsheetWidthRule::Relative(multiplier)
            }
            Value::String(value) => SpreadsheetWidthRule::Absolute(
                value
                    .parse::<u32>()
                    .ok()
                    .filter(|value| *value > 0)
                    .ok_or_else(|| format!("invalid spreadsheet width {value:?}"))?,
            ),
            _ => return Err("spreadsheet width must be an integer or multiplier".to_string()),
        };
        for column in parse_column_specification(specification)? {
            if column == 0 || column > columns {
                return Err(format!(
                    "spreadsheet column {column} is outside 1..={columns}"
                ));
            }
            let cloned = match width {
                SpreadsheetWidthRule::Absolute(value) => SpreadsheetWidthRule::Absolute(value),
                SpreadsheetWidthRule::Relative(value) => SpreadsheetWidthRule::Relative(value),
            };
            if let Some(existing) = result.iter_mut().find(|(existing, _)| *existing == column) {
                existing.1 = cloned;
            } else {
                result.push((column, cloned));
            }
        }
    }
    Ok(result)
}

fn parse_column_specification(value: &str) -> Result<Vec<u32>, String> {
    let mut columns = Vec::new();
    for part in value
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
    {
        if let Some((start, end)) = part.split_once('-') {
            let start = start
                .parse::<u32>()
                .map_err(|_| format!("invalid spreadsheet column specification {value:?}"))?;
            let end = end
                .parse::<u32>()
                .map_err(|_| format!("invalid spreadsheet column specification {value:?}"))?;
            if start == 0 || start > end {
                return Err(format!("invalid spreadsheet column range {part:?}"));
            }
            columns.extend(start..=end);
        } else {
            columns.push(
                part.parse::<u32>()
                    .ok()
                    .filter(|column| *column > 0)
                    .ok_or_else(|| format!("invalid spreadsheet column specification {value:?}"))?,
            );
        }
    }
    if columns.is_empty() {
        return Err("spreadsheet column specification is empty".to_string());
    }
    Ok(columns)
}

fn parse_spreadsheet_fonts(value: Option<&Value>) -> Result<Vec<SpreadsheetFont>, String> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    value
        .as_object()
        .ok_or_else(|| "spreadsheet fonts must be an object".to_string())?
        .iter()
        .map(|(name, value)| {
            let object = value
                .as_object()
                .ok_or_else(|| "spreadsheet font must be an object".to_string())?;
            reject_unknown_fields(
                object,
                &["face", "size", "bold", "italic", "underline", "strikeout"],
                "spreadsheet font",
            )?;
            let font = SpreadsheetFont::new(
                SpreadsheetFontName::new(name.clone())
                    .map_err(|error| format!("invalid spreadsheet font name: {error}"))?,
                first_object_string(object, &["face"])
                    .map(|value| DescriptionText::new(value.to_string()))
                    .transpose()
                    .map_err(|error| format!("invalid spreadsheet font face: {error}"))?,
                optional_json_bool(object, "bold")?.unwrap_or(false),
                optional_json_bool(object, "italic")?.unwrap_or(false),
                first_object_value(object, &["size"])
                    .map(|value| positive_u16(value, "spreadsheet font size"))
                    .transpose()?,
            )
            .with_underline(optional_json_bool(object, "underline")?.unwrap_or(false))
            .with_strikeout(optional_json_bool(object, "strikeout")?.unwrap_or(false));
            Ok(font)
        })
        .collect()
}

fn parse_spreadsheet_styles(value: Option<&Value>) -> Result<Vec<SpreadsheetStyle>, String> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    value
        .as_object()
        .ok_or_else(|| "spreadsheet styles must be an object".to_string())?
        .iter()
        .map(|(name, value)| {
            let object = value
                .as_object()
                .ok_or_else(|| "spreadsheet style must be an object".to_string())?;
            reject_unknown_fields(
                object,
                &[
                    "font",
                    "border",
                    "borderWidth",
                    "align",
                    "valign",
                    "wrap",
                    "format",
                ],
                "spreadsheet style",
            )?;
            let border_spec = first_object_string(object, &["border"]).unwrap_or("none");
            let border = if border_spec == "none" || border_spec.is_empty() {
                SpreadsheetBorderStyle::None
            } else {
                match first_object_string(object, &["borderWidth"]).unwrap_or("thin") {
                    "thin" => SpreadsheetBorderStyle::Thin,
                    "thick" => SpreadsheetBorderStyle::Thick,
                    value => return Err(format!("unsupported spreadsheet border width {value:?}")),
                }
            };
            let sides = if matches!(border, SpreadsheetBorderStyle::None) {
                Vec::new()
            } else {
                let mut sides = Vec::new();
                for value in border_spec.split(',').map(str::trim) {
                    match value {
                        "all" => {
                            sides = vec![
                                SpreadsheetBorderSide::Left,
                                SpreadsheetBorderSide::Top,
                                SpreadsheetBorderSide::Right,
                                SpreadsheetBorderSide::Bottom,
                            ];
                            break;
                        }
                        "left" => sides.push(SpreadsheetBorderSide::Left),
                        "top" => sides.push(SpreadsheetBorderSide::Top),
                        "right" => sides.push(SpreadsheetBorderSide::Right),
                        "bottom" => sides.push(SpreadsheetBorderSide::Bottom),
                        value => {
                            return Err(format!("unsupported spreadsheet border side {value:?}"))
                        }
                    }
                }
                sides
            };
            let horizontal = first_object_string(object, &["align"])
                .map(|value| match value {
                    "left" => Ok(SpreadsheetHorizontalAlignment::Left),
                    "center" => Ok(SpreadsheetHorizontalAlignment::Center),
                    "right" => Ok(SpreadsheetHorizontalAlignment::Right),
                    _ => Err(format!("unsupported spreadsheet alignment {value:?}")),
                })
                .transpose()?;
            let vertical = first_object_string(object, &["valign"])
                .map(|value| match value {
                    "top" => Ok(SpreadsheetVerticalAlignment::Top),
                    "center" => Ok(SpreadsheetVerticalAlignment::Center),
                    "bottom" => Ok(SpreadsheetVerticalAlignment::Bottom),
                    _ => Err(format!(
                        "unsupported spreadsheet vertical alignment {value:?}"
                    )),
                })
                .transpose()?;
            Ok(SpreadsheetStyle::new(
                SpreadsheetStyleName::new(name.clone())
                    .map_err(|error| format!("invalid spreadsheet style name: {error}"))?,
                first_object_string(object, &["font"])
                    .map(|value| SpreadsheetFontName::new(value.to_string()))
                    .transpose()
                    .map_err(|error| format!("invalid spreadsheet style font: {error}"))?,
                border,
                horizontal,
                vertical,
                optional_json_bool(object, "wrap")?.unwrap_or(false),
                first_object_string(object, &["format"])
                    .map(|value| SpreadsheetNumberFormat::new(value.to_string()))
                    .transpose()
                    .map_err(|error| format!("invalid spreadsheet number format: {error}"))?,
            )
            .with_border_sides(sides))
        })
        .collect()
}

fn parse_spreadsheet_area(value: &Value) -> Result<SpreadsheetArea, String> {
    let object = value
        .as_object()
        .ok_or_else(|| "spreadsheet area must be an object".to_string())?;
    reject_unknown_fields(object, &["name", "rows"], "spreadsheet area")?;
    let name = SpreadsheetAreaName::new(
        first_object_string(object, &["name"])
            .ok_or_else(|| "spreadsheet area requires name".to_string())?
            .to_string(),
    )
    .map_err(|error| format!("invalid spreadsheet area name: {error}"))?;
    let mut rows = Vec::new();
    for value in first_object_value(object, &["rows"])
        .ok_or_else(|| "spreadsheet area requires rows".to_string())?
        .as_array()
        .ok_or_else(|| "spreadsheet rows must be an array".to_string())?
    {
        if let Some(cells) = value.as_array() {
            rows.push(SpreadsheetRow::new(
                cells
                    .iter()
                    .map(parse_spreadsheet_cell)
                    .collect::<Result<Vec<_>, String>>()?,
            ));
            continue;
        }
        let row = value
            .as_object()
            .ok_or_else(|| "spreadsheet row must be an object or cell array".to_string())?;
        reject_unknown_fields(
            row,
            &["height", "rowStyle", "cells", "empty"],
            "spreadsheet row",
        )?;
        if let Some(empty) = first_object_value(row, &["empty"]) {
            let count = positive_u32(empty, "spreadsheet empty row count")?;
            rows.extend((0..count).map(|_| SpreadsheetRow::new(Vec::new())));
            continue;
        }
        let cells = first_object_value(row, &["cells"])
            .map(|value| {
                value
                    .as_array()
                    .ok_or_else(|| "spreadsheet row cells must be an array".to_string())?
                    .iter()
                    .map(parse_spreadsheet_cell)
                    .collect::<Result<Vec<_>, String>>()
            })
            .transpose()?
            .unwrap_or_default();
        rows.push(
            SpreadsheetRow::new(cells)
                .with_height(
                    first_object_value(row, &["height"])
                        .map(|value| positive_u32(value, "spreadsheet row height"))
                        .transpose()?,
                )
                .with_style(
                    first_object_string(row, &["rowStyle"])
                        .map(|value| SpreadsheetStyleName::new(value.to_string()))
                        .transpose()
                        .map_err(|error| format!("invalid spreadsheet row style: {error}"))?,
                ),
        );
    }
    SpreadsheetArea::new(name, rows).map_err(|error| format!("invalid spreadsheet area: {error}"))
}

fn parse_spreadsheet_cell(value: &Value) -> Result<SpreadsheetCell, String> {
    let object = value
        .as_object()
        .ok_or_else(|| "spreadsheet cell must be an object".to_string())?;
    reject_unknown_fields(
        object,
        &[
            "col", "span", "rowspan", "style", "text", "param", "template", "detail",
        ],
        "spreadsheet cell",
    )?;
    let text = first_object_string(object, &["text"])
        .map(|value| SpreadsheetCellText::new(value.to_string()))
        .transpose()
        .map_err(|error| format!("invalid spreadsheet cell text: {error}"))?;
    let parameter = first_object_string(object, &["param"])
        .map(|value| FormParameterName::new(value.to_string()))
        .transpose()
        .map_err(|error| format!("invalid spreadsheet parameter: {error}"))?;
    let template = first_object_string(object, &["template"])
        .map(|value| SpreadsheetCellText::new(value.to_string()))
        .transpose()
        .map_err(|error| format!("invalid spreadsheet template text: {error}"))?;
    let detail = first_object_string(object, &["detail"])
        .map(|value| SpreadsheetCellText::new(value.to_string()))
        .transpose()
        .map_err(|error| format!("invalid spreadsheet detail text: {error}"))?;
    let count = usize::from(text.is_some())
        + usize::from(parameter.is_some())
        + usize::from(template.is_some())
        + usize::from(detail.is_some());
    let content = match count {
        0 => SpreadsheetCellValue::Empty,
        1 if text.is_some() => SpreadsheetCellValue::Text(text.unwrap()),
        1 if parameter.is_some() => SpreadsheetCellValue::Parameter(parameter.unwrap()),
        1 if template.is_some() => SpreadsheetCellValue::Template(template.unwrap()),
        1 => SpreadsheetCellValue::Detail(detail.unwrap()),
        _ => SpreadsheetCellValue::Composite(
            SpreadsheetCellContent::new(text, parameter, template, detail)
                .map_err(|error| format!("invalid spreadsheet cell content: {error}"))?,
        ),
    };
    let column_span = first_object_value(object, &["span"])
        .map(|value| positive_u16(value, "spreadsheet cell span"))
        .transpose()?
        .unwrap_or(1);
    let row_span = first_object_value(object, &["rowspan"])
        .map(|value| positive_u16(value, "spreadsheet cell rowspan"))
        .transpose()?
        .unwrap_or(1);
    let style = first_object_string(object, &["style"])
        .map(|value| SpreadsheetStyleName::new(value.to_string()))
        .transpose()
        .map_err(|error| format!("invalid spreadsheet cell style: {error}"))?;
    let cell = SpreadsheetCell::new(
        required_positive_u32(object, "col", "spreadsheet cell column")?,
        content,
    )
    .and_then(|cell| cell.with_span(column_span, row_span))
    .map_err(|error| format!("invalid spreadsheet cell: {error}"))?;
    Ok(cell.with_style(style))
}

fn required_positive_u32(
    object: &Map<String, Value>,
    field: &str,
    label: &str,
) -> Result<u32, String> {
    positive_u32(
        object
            .get(field)
            .ok_or_else(|| format!("{label} is required"))?,
        label,
    )
}

fn positive_u32(value: &Value, label: &str) -> Result<u32, String> {
    value
        .as_u64()
        .and_then(|value| u32::try_from(value).ok())
        .filter(|value| *value > 0)
        .ok_or_else(|| format!("{label} must be a positive integer"))
}

fn positive_u16(value: &Value, label: &str) -> Result<u16, String> {
    value
        .as_u64()
        .and_then(|value| u16::try_from(value).ok())
        .filter(|value| *value > 0)
        .ok_or_else(|| format!("{label} must be a positive integer"))
}

fn optional_json_bool(object: &Map<String, Value>, field: &str) -> Result<Option<bool>, String> {
    object
        .get(field)
        .map(|value| {
            value
                .as_bool()
                .ok_or_else(|| format!("{field} must be boolean"))
        })
        .transpose()
}

fn read_definition_or_inline<T>(
    args: &Map<String, Value>,
    path_names: &[&str],
    inline_names: &[&str],
    label: &str,
    context: &WorkspaceContext,
) -> Result<T, String>
where
    T: serde::de::DeserializeOwned,
{
    match (
        first_string(args, path_names),
        first_string(args, inline_names),
    ) {
        (Some(_), Some(_)) => Err(format!("{label} was supplied more than once")),
        (Some(_), None) => read_json_path(args, path_names, label, context),
        (None, Some(value)) => serde_json::from_str(value)
            .map_err(|error| format!("failed to parse {label} semantic JSON: {error}")),
        (None, None) => Err(format!("{label} is required")),
    }
}

fn read_subsystem_definition(
    args: &Map<String, Value>,
    path_names: &[&str],
    inline_names: &[&str],
    context: &WorkspaceContext,
) -> Result<SubsystemDefinition, String> {
    let mut value = match (
        first_string(args, path_names),
        first_string(args, inline_names),
    ) {
        (Some(_), Some(_)) => {
            return Err("subsystem definition was supplied more than once".to_string())
        }
        (Some(_), None) => read_json_value(args, path_names, "subsystem definition", context)?,
        (None, Some(value)) => serde_json::from_str(value)
            .map_err(|error| format!("failed to parse subsystem definition JSON: {error}"))?,
        (None, None) => return Err("subsystem definition is required".to_string()),
    };
    let object = value
        .as_object_mut()
        .ok_or_else(|| "subsystem definition must be an object".to_string())?;
    for (field, default) in [
        ("synonym", Value::Null),
        ("comment", Value::Null),
        ("explanation", Value::Null),
        ("content", Value::Array(Vec::new())),
        ("children", Value::Array(Vec::new())),
    ] {
        object.entry(field.to_string()).or_insert(default);
    }
    serde_json::from_value(value)
        .map_err(|error| format!("failed to parse subsystem definition semantic JSON: {error}"))
}

fn parse_form_edit(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<FormEdit, String> {
    let definition = match (
        args.get("definition"),
        first_string(args, &["jsonPath", "JsonPath"]),
    ) {
        (Some(_), Some(_)) => return Err("form definition was supplied more than once".to_string()),
        (Some(value), None) => value.clone(),
        (None, Some(_)) => read_json_value(args, &["jsonPath", "JsonPath"], "JsonPath", context)?,
        (None, None) => return Err("form edit requires a semantic definition".to_string()),
    };
    if definition.get("patches").is_some() {
        return serde_json::from_value(definition)
            .map_err(|error| format!("invalid form patch set: {error}"));
    }

    let mut patches = Vec::new();
    for value in definition
        .get("elements")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        patches.push(FormPatch::AddElement(parse_form_element(value)?));
    }
    for value in definition
        .get("attributes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        patches.push(FormPatch::UpsertAttribute(parse_form_attribute(value)?));
    }
    for value in definition
        .get("commands")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        patches.push(FormPatch::UpsertCommand(parse_form_command(value)?));
    }
    for value in definition
        .get("events")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        patches.push(FormPatch::BindEvent(
            serde_json::from_value(value.clone())
                .map_err(|error| format!("invalid form event: {error}"))?,
        ));
    }
    for value in definition
        .get("removeElements")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let name = value
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| "form element removal requires a name".to_string())?;
        patches.push(FormPatch::RemoveElement(
            FormElementName::new(name.to_string())
                .map_err(|error| format!("invalid form element name: {error}"))?,
        ));
    }
    if patches.is_empty() {
        if definition.as_object().is_some_and(Map::is_empty) {
            patches.push(FormPatch::NoOp);
        } else {
            patches.push(FormPatch::Replace(parse_managed_form_value(&definition)?));
        }
    }
    FormEdit::new(patches, bool_arg(args, &["noValidate", "NoValidate"]))
        .map_err(|error| format!("invalid form patch set: {error}"))
}

fn required_string<'a>(
    args: &'a Map<String, Value>,
    names: &[&str],
    label: &str,
) -> Result<&'a str, String> {
    first_string(args, names)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| format!("missing required {label} argument"))
}

fn required_text<T>(
    args: &Map<String, Value>,
    names: &[&str],
    label: &str,
    constructor: impl FnOnce(String) -> Result<T, unica_format_core::commands::SemanticValueError>,
) -> Result<T, String> {
    constructor(required_string(args, names, label)?.to_string())
        .map_err(|error| format!("invalid {label}: {error}"))
}

fn required_public_owner<T>(
    args: &Map<String, Value>,
    names: &[&str],
    label: &str,
    constructor: impl FnOnce(String) -> Result<T, unica_format_core::commands::SemanticValueError>,
) -> Result<T, String> {
    let value = required_string(args, names, label)?;
    if value.contains('\\') {
        return Err(format!("invalid {label}: invalid semantic name component"));
    }
    let semantic_name = if value.contains('/') {
        if value
            .split('/')
            .any(|component| component.is_empty() || matches!(component, "." | ".."))
        {
            return Err(format!("invalid {label}: invalid semantic name component"));
        }
        value
            .rsplit('/')
            .next()
            .expect("a slash-containing owner has a final component")
    } else {
        value
    };
    constructor(semantic_name.to_string()).map_err(|error| format!("invalid {label}: {error}"))
}

fn optional_text<T>(
    args: &Map<String, Value>,
    names: &[&str],
    constructor: impl FnOnce(String) -> Result<T, unica_format_core::commands::SemanticValueError>,
) -> Result<Option<T>, String> {
    first_string(args, names)
        .map(|value| constructor(value.to_string()).map_err(|error| error.to_string()))
        .transpose()
}

fn bool_arg(args: &Map<String, Value>, names: &[&str]) -> bool {
    names
        .iter()
        .find_map(|name| args.get(*name).and_then(Value::as_bool))
        .unwrap_or(false)
}

fn parse_optional_extension_purpose(
    args: &Map<String, Value>,
) -> Result<Option<ExtensionPurpose>, String> {
    first_string(args, &["purpose", "Purpose"])
        .map(|value| match value.to_ascii_lowercase().as_str() {
            "patch" => Ok(ExtensionPurpose::Patch),
            "customization" => Ok(ExtensionPurpose::Customization),
            "addon" => Ok(ExtensionPurpose::AddOn),
            _ => Err(format!(
                "invalid Purpose: expected Patch, Customization, or AddOn; got {value:?}"
            )),
        })
        .transpose()
}

fn parse_form_purpose(args: &Map<String, Value>) -> Result<FormPurpose, String> {
    match first_string(args, &["purpose", "Purpose"])
        .unwrap_or("Object")
        .to_ascii_lowercase()
        .as_str()
    {
        "object" => Ok(FormPurpose::Object),
        "list" => Ok(FormPurpose::List),
        "choice" => Ok(FormPurpose::Choice),
        "record" => Ok(FormPurpose::Record),
        other => Err(format!(
            "invalid Purpose: expected Object, List, Choice, or Record; got {other:?}"
        )),
    }
}

fn parse_default_form_assignment(
    args: &Map<String, Value>,
) -> Result<DefaultFormAssignment, String> {
    let value = ["setDefault", "SetDefault"]
        .iter()
        .find_map(|name| args.get(*name));
    match value {
        None => Ok(DefaultFormAssignment::IfVacant),
        Some(Value::Bool(true)) => Ok(DefaultFormAssignment::Always),
        Some(Value::Bool(false)) => Ok(DefaultFormAssignment::Never),
        Some(_) => Err("SetDefault argument must be boolean".to_string()),
    }
}

fn parse_optional_form_compile_purpose(
    args: &Map<String, Value>,
) -> Result<Option<FormCompilePurpose>, String> {
    first_string(args, &["purpose", "Purpose"])
        .map(|value| match value.to_ascii_lowercase().as_str() {
            "item" => Ok(FormCompilePurpose::Item),
            "folder" => Ok(FormCompilePurpose::Folder),
            "list" => Ok(FormCompilePurpose::List),
            "choice" => Ok(FormCompilePurpose::Choice),
            "record" => Ok(FormCompilePurpose::Record),
            _ => Err(format!(
                "invalid Purpose: expected Item, Folder, List, Choice, or Record; got {value:?}"
            )),
        })
        .transpose()
}

fn parse_configuration_mutation(
    operation: &str,
    value: &str,
) -> Result<ConfigurationMutation, String> {
    match operation {
        "modify-property" => {
            let (name, value) = value
                .split_once('=')
                .ok_or_else(|| "Invalid property format, expected 'Key=Value'".to_string())?;
            let property = match name.trim() {
                "Name" => ConfigurationProperty::Name,
                "Synonym" => ConfigurationProperty::Synonym,
                "Comment" => ConfigurationProperty::Comment,
                "Vendor" => ConfigurationProperty::Vendor,
                "Version" => ConfigurationProperty::Version,
                "DefaultLanguage" => ConfigurationProperty::DefaultLanguage,
                "BriefInformation" => ConfigurationProperty::BriefInformation,
                "DetailedInformation" => ConfigurationProperty::DetailedInformation,
                "Copyright" => ConfigurationProperty::Copyright,
                "VendorInformationAddress" => ConfigurationProperty::VendorInformationAddress,
                "ConfigurationInformationAddress" => {
                    ConfigurationProperty::ConfigurationInformationAddress
                }
                "UpdateCatalogAddress" => ConfigurationProperty::UpdateCatalogAddress,
                other => return Err(format!("unsupported configuration property: {other}")),
            };
            let value = if property == ConfigurationProperty::DefaultLanguage {
                ConfigurationPropertyValue::Language(
                    LanguageCode::new(value.trim().to_string())
                        .map_err(|error| format!("invalid configuration language: {error}"))?,
                )
            } else {
                ConfigurationPropertyValue::Text(
                    ConfigurationTextValue::new(value.trim().to_string()).map_err(|error| {
                        format!("invalid configuration property value: {error}")
                    })?,
                )
            };
            Ok(ConfigurationMutation::SetProperty(
                ConfigurationPropertyPatch::new(property, value)
                    .map_err(|error| format!("invalid configuration property patch: {error}"))?,
            ))
        }
        "remove-childObject" => Ok(ConfigurationMutation::RemoveChild(
            MetadataObjectReference::new(value.to_string())
                .map_err(|error| format!("invalid configuration child reference: {error}"))?,
        )),
        "add-childObject" => Ok(ConfigurationMutation::AddChild(
            MetadataObjectReference::new(value.to_string())
                .map_err(|error| format!("invalid configuration child reference: {error}"))?,
        )),
        "set-defaultRoles" => Ok(ConfigurationMutation::SetDefaultRoles(parse_typed_list(
            value,
            RoleName::new,
            "role",
        )?)),
        "add-defaultRole" => Ok(ConfigurationMutation::AddDefaultRole(
            RoleName::new(value.to_string())
                .map_err(|error| format!("invalid role name: {error}"))?,
        )),
        "remove-defaultRole" => Ok(ConfigurationMutation::RemoveDefaultRole(
            RoleName::new(value.to_string())
                .map_err(|error| format!("invalid role name: {error}"))?,
        )),
        "set-panels" => Ok(ConfigurationMutation::SetPanels(
            serde_json::from_str(value)
                .map_err(|error| format!("invalid panel arrangement: {error}"))?,
        )),
        "set-home-page" => Ok(ConfigurationMutation::SetHomePage(
            serde_json::from_str(value)
                .map_err(|error| format!("invalid home page arrangement: {error}"))?,
        )),
        _ => Err(format!(
            "unsupported configuration edit operation: {operation}"
        )),
    }
}

fn parse_typed_list<T>(
    value: &str,
    constructor: impl Fn(String) -> Result<T, SemanticValueError> + Copy,
    label: &str,
) -> Result<Vec<T>, String> {
    let values = serde_json::from_str::<Vec<String>>(value).unwrap_or_else(|_| {
        value
            .split([',', ';'])
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
            .collect()
    });
    values
        .into_iter()
        .map(|value| constructor(value).map_err(|error| format!("invalid {label} value: {error}")))
        .collect()
}

fn parse_borrow_scope(args: &Map<String, Value>) -> Result<Option<BorrowScope>, String> {
    let Some(value) = first_value(args, &["borrowMainAttribute", "BorrowMainAttribute"]) else {
        return Ok(None);
    };
    match value {
        Value::Null | Value::Bool(false) => Ok(None),
        Value::Bool(true) => Ok(Some(BorrowScope::Form)),
        Value::String(value) if value.trim().is_empty() || value == "Form" => {
            Ok(Some(BorrowScope::Form))
        }
        Value::String(value) if value == "All" => Ok(Some(BorrowScope::All)),
        _ => Err("-BorrowMainAttribute accepts 'Form' or 'All' (default: Form)".to_string()),
    }
}

fn parse_interceptor(value: &str) -> Result<InterceptorKind, String> {
    match value {
        "before" | "Before" | "Перед" => Ok(InterceptorKind::Before),
        "after" | "After" | "После" => Ok(InterceptorKind::After),
        "instead" | "Instead" | "Вместо" => Ok(InterceptorKind::Instead),
        _ => Err(format!("unsupported interceptor type: {value}")),
    }
}

fn parse_execution_context(value: &str) -> Result<ExecutionContext, String> {
    match value {
        "auto" | "automatic" => Ok(ExecutionContext::Automatic),
        "client" | "НаКлиенте" => Ok(ExecutionContext::Client),
        "server" | "НаСервере" => Ok(ExecutionContext::Server),
        "server-no-context" | "serverWithoutContext" | "НаСервереБезКонтекста" => {
            Ok(ExecutionContext::ServerWithoutContext)
        }
        _ => Err(format!("unsupported execution context: {value}")),
    }
}

fn parse_metadata_patch(operation: &str, value: &str) -> Result<MetadataPatch, String> {
    let (action, target) = operation
        .split_once('-')
        .ok_or_else(|| format!("invalid metadata edit operation: {operation}"))?;
    if target == "property" {
        let property = parse_metadata_object_property(value.split('=').next().unwrap_or(value))?;
        return match action {
            "add" | "modify" | "set" => Ok(MetadataPatch::SetProperties(
                MetadataPropertyChanges::one(parse_metadata_property_patch(value)?),
            )),
            "remove" => Ok(MetadataPatch::ClearProperties(
                MetadataPropertiesToClear::one(property),
            )),
            _ => Err(format!(
                "unsupported metadata property operation: {operation}"
            )),
        };
    }

    let kind = match target {
        "attribute" => MetadataNamedChildKind::Attribute,
        "tabular-section" | "tabularSection" => MetadataNamedChildKind::TabularSection,
        "ts-attribute" => MetadataNamedChildKind::Attribute,
        "form" => MetadataNamedChildKind::Form,
        "template" => MetadataNamedChildKind::Template,
        "command" => MetadataNamedChildKind::Command,
        "dimension" => MetadataNamedChildKind::Dimension,
        "resource" => MetadataNamedChildKind::Resource,
        "requisite" => MetadataNamedChildKind::Requisite,
        "enum-value" => MetadataNamedChildKind::EnumValue,
        _ => return Err(format!("unsupported metadata edit target: {target}")),
    };
    match action {
        "add" => Ok(MetadataPatch::AddChild(parse_metadata_child_definition(
            target, kind, value,
        )?)),
        "remove" => Ok(MetadataPatch::RemoveChild(parse_metadata_child_reference(
            target, kind, value,
        )?)),
        "modify" | "set" => Ok(MetadataPatch::ModifyChild(parse_metadata_child_patch(
            target, kind, value,
        )?)),
        _ => Err(format!("unsupported metadata edit operation: {operation}")),
    }
}

fn parse_metadata_child_patch(
    target: &str,
    kind: MetadataNamedChildKind,
    value: &str,
) -> Result<MetadataChildPatch, String> {
    if value.trim_start().starts_with('{') {
        return serde_json::from_str(value).map_err(|error| {
            format!("metadata child modification must be a typed JSON patch: {error}")
        });
    }
    let (raw_target, raw_changes) = value
        .split_once(':')
        .ok_or_else(|| format!("modify-{target} requires Value like Name: key=value"))?;
    let (parent, name) = if target == "ts-attribute" {
        let (parent, name) = raw_target.trim().split_once('.').ok_or_else(|| {
            "modify-ts-attribute requires Value like Section.Attribute: key=value".to_string()
        })?;
        (Some(parent), name)
    } else {
        (None, raw_target.trim())
    };
    let mut reference = MetadataChildReference::new(
        kind,
        MetadataChildName::new(name.to_string())
            .map_err(|error| format!("invalid metadata child name: {error}"))?,
    );
    if let Some(parent) = parent {
        reference = reference.with_parent(Some(
            MetadataChildName::new(parent.to_string())
                .map_err(|error| format!("invalid metadata parent name: {error}"))?,
        ));
    }
    let changes = raw_changes
        .split(";;")
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(parse_metadata_property_patch)
        .collect::<Result<Vec<_>, _>>()?;
    if changes.is_empty() {
        return Err(format!(
            "modify-{target} requires at least one property change"
        ));
    }
    MetadataChildPatch::new(reference, changes)
        .map_err(|error| format!("invalid metadata child patch: {error}"))
}

fn parse_metadata_object_property(value: &str) -> Result<MetadataObjectProperty, String> {
    match value.trim() {
        "Name" | "name" => Ok(MetadataObjectProperty::Name),
        "Synonym" | "synonym" => Ok(MetadataObjectProperty::Synonym),
        "Comment" | "comment" => Ok(MetadataObjectProperty::Comment),
        "Type" | "valueType" => Ok(MetadataObjectProperty::ValueType),
        "Length" | "length" => Ok(MetadataObjectProperty::Length),
        "Precision" | "precision" => Ok(MetadataObjectProperty::Precision),
        "Nonnegative" | "nonnegative" => Ok(MetadataObjectProperty::Nonnegative),
        "FillChecking" | "fillChecking" => Ok(MetadataObjectProperty::FillChecking),
        "Indexing" | "indexing" => Ok(MetadataObjectProperty::Indexing),
        "MultiLine" | "multiLine" => Ok(MetadataObjectProperty::MultiLine),
        "HierarchyType" | "hierarchyType" => Ok(MetadataObjectProperty::HierarchyType),
        "FillValue" | "fillValue" => Ok(MetadataObjectProperty::FillValue),
        other => Err(format!("unsupported metadata property: {other}")),
    }
}

fn parse_metadata_property_patch(value: &str) -> Result<MetadataPropertyPatch, String> {
    let (name, value) = value
        .split_once('=')
        .ok_or_else(|| "metadata property requires Name=Value".to_string())?;
    let property = parse_metadata_object_property(name)?;
    let value = match property {
        MetadataObjectProperty::Length | MetadataObjectProperty::Precision => {
            MetadataPropertyValue::Integer(
                value
                    .trim()
                    .parse()
                    .map_err(|_| format!("metadata property {name} requires an integer"))?,
            )
        }
        MetadataObjectProperty::Nonnegative | MetadataObjectProperty::MultiLine => {
            MetadataPropertyValue::Boolean(
                value
                    .trim()
                    .parse()
                    .map_err(|_| format!("metadata property {name} requires a boolean"))?,
            )
        }
        MetadataObjectProperty::ValueType => {
            MetadataPropertyValue::Type(parse_metadata_type_expression(value.trim())?)
        }
        MetadataObjectProperty::FillChecking => MetadataPropertyValue::FillChecking(
            parse_optional_fill_checking(Some(&Value::String(value.trim().to_string())))?
                .ok_or_else(|| format!("metadata property {name} requires a fill-checking mode"))?,
        ),
        MetadataObjectProperty::Indexing => MetadataPropertyValue::Indexing(
            parse_optional_indexing(Some(&Value::String(value.trim().to_string())))?
                .ok_or_else(|| format!("metadata property {name} requires an indexing mode"))?,
        ),
        MetadataObjectProperty::HierarchyType => {
            let value = match value.trim() {
                "HierarchyOfItems" | "HierarchyItemsOnly" => MetadataHierarchyType::Items,
                "HierarchyFoldersAndItems" | "FoldersAndItems" => {
                    MetadataHierarchyType::GroupsAndItems
                }
                value => return Err(format!("unsupported hierarchy type: {value}")),
            };
            MetadataPropertyValue::HierarchyType(value)
        }
        MetadataObjectProperty::FillValue => MetadataPropertyValue::FillValue(
            MetadataObjectReference::new(value.trim().to_string())
                .map_err(|error| format!("invalid metadata fill value: {error}"))?,
        ),
        MetadataObjectProperty::Name => MetadataPropertyValue::Name(
            MetadataChildName::new(value.trim().to_string())
                .map_err(|error| format!("invalid metadata object name: {error}"))?,
        ),
        MetadataObjectProperty::Synonym => MetadataPropertyValue::Synonym(
            SynonymText::new(value.trim().to_string())
                .map_err(|error| format!("invalid metadata synonym: {error}"))?,
        ),
        MetadataObjectProperty::Comment => MetadataPropertyValue::Comment(
            CommentText::new(value.trim().to_string())
                .map_err(|error| format!("invalid metadata comment: {error}"))?,
        ),
    };
    MetadataPropertyPatch::new(property, value)
        .map_err(|error| format!("invalid metadata property patch: {error}"))
}

fn parse_metadata_child_definition(
    target: &str,
    kind: MetadataNamedChildKind,
    value: &str,
) -> Result<MetadataChildDefinition, String> {
    if target == "attribute" || target == "ts-attribute" {
        let definition = serde_json::from_str::<MetadataAttributeDefinition>(value)
            .or_else(|_| {
                MetadataFieldName::new(value.trim().to_string())
                    .map(MetadataAttributeDefinition::new)
                    .map_err(serde_json::Error::custom)
            })
            .map_err(|error| format!("invalid metadata attribute definition: {error}"))?;
        return Ok(MetadataChildDefinition::Attribute(definition));
    }
    if target == "tabular-section" || target == "tabularSection" {
        let definition = serde_json::from_str::<MetadataTabularSectionDefinition>(value)
            .or_else(|_| {
                MetadataChildName::new(value.trim().to_string())
                    .map(|name| MetadataTabularSectionDefinition::new(name, Vec::new()))
                    .map_err(serde_json::Error::custom)
            })
            .map_err(|error| format!("invalid tabular section definition: {error}"))?;
        return Ok(MetadataChildDefinition::TabularSection(definition));
    }
    let definition = serde_json::from_str::<MetadataNamedChildDefinition>(value)
        .or_else(|_| {
            MetadataChildName::new(value.trim().to_string())
                .map(|name| MetadataNamedChildDefinition::new(kind, name))
                .map_err(serde_json::Error::custom)
        })
        .map_err(|error| format!("invalid metadata child definition: {error}"))?;
    Ok(MetadataChildDefinition::Named(definition))
}

fn parse_metadata_child_reference(
    target: &str,
    kind: MetadataNamedChildKind,
    value: &str,
) -> Result<MetadataChildReference, String> {
    let (parent, name) = if target == "ts-attribute" {
        value
            .split_once('.')
            .map_or((None, value), |(parent, name)| (Some(parent), name))
    } else {
        (None, value)
    };
    let reference = MetadataChildReference::new(
        kind,
        MetadataChildName::new(name.trim().to_string())
            .map_err(|error| format!("invalid metadata child name: {error}"))?,
    );
    Ok(reference.with_parent(
        parent
            .map(|value| MetadataChildName::new(value.trim().to_string()))
            .transpose()
            .map_err(|error| format!("invalid tabular section name: {error}"))?,
    ))
}

fn parse_interface_edit(operation: &str, value: &str) -> Result<InterfaceEdit, String> {
    match operation {
        "hide" => Ok(InterfaceEdit::Hide(parse_interface_item(value)?)),
        "show" => Ok(InterfaceEdit::Show(parse_interface_item(value)?)),
        "place" => {
            let value: Value = serde_json::from_str(value)
                .map_err(|error| format!("invalid interface placement: {error}"))?;
            let command = value
                .get("command")
                .and_then(Value::as_str)
                .ok_or_else(|| "interface placement requires command".to_string())?;
            let group = value
                .get("group")
                .and_then(Value::as_str)
                .ok_or_else(|| "interface placement requires group".to_string())?;
            let order = value.get("order").and_then(Value::as_u64).unwrap_or(0);
            Ok(InterfaceEdit::Place(InterfacePlacement::new(
                parse_interface_item(command)?,
                InterfaceGroupName::new(group.to_string())
                    .map_err(|error| format!("invalid interface group: {error}"))?,
                u16::try_from(order).map_err(|_| "interface order is too large".to_string())?,
            )))
        }
        "order" => {
            let value: Value = serde_json::from_str(value)
                .map_err(|error| format!("invalid interface order: {error}"))?;
            let group = value
                .get("group")
                .and_then(Value::as_str)
                .ok_or_else(|| "interface order requires group".to_string())?;
            let commands = value
                .get("commands")
                .and_then(Value::as_array)
                .ok_or_else(|| "interface order requires commands".to_string())?
                .iter()
                .map(|value| {
                    value
                        .as_str()
                        .ok_or_else(|| "interface command must be text".to_string())
                        .and_then(parse_interface_item)
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok(InterfaceEdit::Order(InterfaceCommandOrder::new(
                InterfaceGroupName::new(group.to_string())
                    .map_err(|error| format!("invalid interface group: {error}"))?,
                commands,
            )))
        }
        "subsystem-order" => Ok(InterfaceEdit::OrderSubsystems(parse_typed_list(
            value,
            SubsystemName::new,
            "subsystem",
        )?)),
        "group-order" => Ok(InterfaceEdit::OrderGroups(parse_typed_list(
            value,
            InterfaceGroupName::new,
            "interface group",
        )?)),
        _ => Err(format!("unsupported interface edit operation: {operation}")),
    }
}

fn parse_interface_item(value: &str) -> Result<InterfaceItemReference, String> {
    InterfaceItemName::new(value.to_string())
        .map(|name| InterfaceItemReference::new(InterfaceItemKind::Command, name))
        .map_err(|error| format!("invalid interface item: {error}"))
}

fn parse_subsystem_edit(operation: &str, value: &str) -> Result<SubsystemEdit, String> {
    match operation {
        "add-content" => Ok(SubsystemEdit::AddContent(
            MetadataObjectReference::new(value.to_string())
                .map_err(|error| format!("invalid subsystem content: {error}"))?,
        )),
        "remove-content" => Ok(SubsystemEdit::RemoveContent(
            MetadataObjectReference::new(value.to_string())
                .map_err(|error| format!("invalid subsystem content: {error}"))?,
        )),
        "add-child" => Ok(SubsystemEdit::AddChild(
            SubsystemName::new(value.to_string())
                .map_err(|error| format!("invalid child subsystem: {error}"))?,
        )),
        "remove-child" => Ok(SubsystemEdit::RemoveChild(
            SubsystemName::new(value.to_string())
                .map_err(|error| format!("invalid child subsystem: {error}"))?,
        )),
        "set-property" => {
            #[derive(serde::Deserialize)]
            #[serde(rename_all = "PascalCase")]
            enum PropertyName {
                Synonym,
                Comment,
                Explanation,
                IncludeInCommandInterface,
            }
            #[derive(serde::Deserialize)]
            #[serde(deny_unknown_fields)]
            struct PropertyWire {
                name: PropertyName,
                value: Value,
            }
            let value: PropertyWire = serde_json::from_str(value)
                .map_err(|error| format!("invalid subsystem property patch: {error}"))?;
            let patch = match value.name {
                PropertyName::Synonym => SubsystemPropertyPatch::SetSynonym(
                    SynonymText::new(
                        value
                            .value
                            .as_str()
                            .ok_or_else(|| "subsystem synonym requires text".to_string())?
                            .to_string(),
                    )
                    .map_err(|error| format!("invalid subsystem synonym: {error}"))?,
                ),
                PropertyName::Comment => SubsystemPropertyPatch::SetComment(
                    CommentText::new(
                        value
                            .value
                            .as_str()
                            .ok_or_else(|| "subsystem comment requires text".to_string())?
                            .to_string(),
                    )
                    .map_err(|error| format!("invalid subsystem comment: {error}"))?,
                ),
                PropertyName::Explanation => SubsystemPropertyPatch::SetExplanation(
                    DescriptionText::new(
                        value
                            .value
                            .as_str()
                            .ok_or_else(|| "subsystem explanation requires text".to_string())?
                            .to_string(),
                    )
                    .map_err(|error| format!("invalid subsystem explanation: {error}"))?,
                ),
                PropertyName::IncludeInCommandInterface => {
                    SubsystemPropertyPatch::SetCommandInterfaceVisibility(
                        value.value.as_bool().ok_or_else(|| {
                            "IncludeInCommandInterface requires a boolean".to_string()
                        })?,
                    )
                }
            };
            Ok(SubsystemEdit::SetProperty(patch))
        }
        _ => Err(format!("unsupported subsystem edit operation: {operation}")),
    }
}

fn parse_support_edit(args: &Map<String, Value>) -> Result<SupportEdit, String> {
    match (
        first_string(args, &["capability", "Capability"]),
        first_string(args, &["set", "Set"]),
    ) {
        (Some("on"), None) => Ok(SupportEdit::Capability(SupportCapability::Enable)),
        (Some("off"), None) => Ok(SupportEdit::Capability(SupportCapability::Disable)),
        (None, Some("locked")) => Ok(SupportEdit::ObjectRule(SupportObjectRule::Locked)),
        (None, Some("editable")) => Ok(SupportEdit::ObjectRule(SupportObjectRule::Editable)),
        (None, Some("off-support")) => Ok(SupportEdit::ObjectRule(SupportObjectRule::OffSupport)),
        _ => Err("support edit requires exactly one closed capability or object rule".to_string()),
    }
}

fn parse_data_composition_mutation(
    operation: &str,
    value: &str,
) -> Result<DataCompositionMutation, String> {
    fn json<T: serde::de::DeserializeOwned>(value: &str, label: &str) -> Result<T, String> {
        serde_json::from_str(value).map_err(|error| format!("invalid {label}: {error}"))
    }
    fn path(value: &str) -> Result<DataFieldPath, String> {
        DataFieldPath::new(value.to_string())
            .map_err(|error| format!("invalid data field path: {error}"))
    }
    match operation {
        "add-field" => Ok(DataCompositionMutation::AddField(
            json(value, "data composition field")
                .or_else(|_| path(value).map(DataCompositionFieldDefinition::new))?,
        )),
        "add-total" => Ok(DataCompositionMutation::AddTotal(json(value, "total")?)),
        "add-calculated-field" => Ok(DataCompositionMutation::AddCalculatedField(json(
            value,
            "calculated field",
        )?)),
        "add-parameter" => Ok(DataCompositionMutation::AddParameter(json(
            value,
            "parameter",
        )?)),
        "add-filter" => Ok(DataCompositionMutation::AddFilter(json(value, "filter")?)),
        "add-dataParameter" => Ok(DataCompositionMutation::AddDataParameter(json(
            value,
            "data parameter",
        )?)),
        "set-query" => Ok(DataCompositionMutation::SetQuery(
            DataCompositionQueryText::new(value.to_string())
                .map_err(|error| format!("invalid query text: {error}"))?,
        )),
        "patch-query" => {
            let (find, replace) = value
                .split_once(" => ")
                .ok_or_else(|| "query patch requires 'find => replace'".to_string())?;
            Ok(DataCompositionMutation::PatchQuery {
                find: DataCompositionQueryText::new(find.to_string())
                    .map_err(|error| format!("invalid query match: {error}"))?,
                replace: DataCompositionQueryText::new(replace.to_string())
                    .map_err(|error| format!("invalid query replacement: {error}"))?,
            })
        }
        "clear-selection" => Ok(DataCompositionMutation::Clear {
            target: DataCompositionClearTarget::Selection,
            scope: parse_data_composition_scope(value)?,
        }),
        "clear-order" => Ok(DataCompositionMutation::Clear {
            target: DataCompositionClearTarget::Order,
            scope: parse_data_composition_scope(value)?,
        }),
        "clear-filter" => Ok(DataCompositionMutation::Clear {
            target: DataCompositionClearTarget::Filter,
            scope: parse_data_composition_scope(value)?,
        }),
        "clear-conditionalAppearance" => Ok(DataCompositionMutation::Clear {
            target: DataCompositionClearTarget::ConditionalAppearance,
            scope: parse_data_composition_scope(value)?,
        }),
        "add-selection" => Ok(DataCompositionMutation::AddSelection(json(
            value,
            "selection",
        )?)),
        "add-order" => Ok(DataCompositionMutation::AddOrder(json(value, "order")?)),
        "add-dataSetLink" => Ok(DataCompositionMutation::AddDataSetLink(json(
            value,
            "data set link",
        )?)),
        "add-dataSet" => Ok(DataCompositionMutation::AddDataSet(json(
            value, "data set",
        )?)),
        "add-variant" => Ok(DataCompositionMutation::AddVariant(json(value, "variant")?)),
        "add-conditionalAppearance" => Ok(DataCompositionMutation::AddConditionalAppearance(json(
            value,
            "conditional appearance",
        )?)),
        "add-drilldown" => Ok(DataCompositionMutation::AddDrilldown(json(
            value,
            "drilldown",
        )?)),
        "set-outputParameter" => Ok(DataCompositionMutation::SetOutputParameter(json(
            value,
            "output parameter",
        )?)),
        "set-structure" => Ok(DataCompositionMutation::SetStructure(json(
            value,
            "structure",
        )?)),
        "modify-structure" => Ok(DataCompositionMutation::ModifyStructure(json(
            value,
            "structure patch",
        )?)),
        "remove-field" => Ok(DataCompositionMutation::RemoveField(path(value)?)),
        "remove-parameter" => Ok(DataCompositionMutation::RemoveParameter(
            DataCompositionParameterName::new(value.to_string())
                .map_err(|error| format!("invalid parameter name: {error}"))?,
        )),
        "modify-field" => Ok(DataCompositionMutation::ModifyField(json(
            value,
            "field patch",
        )?)),
        "set-field-role" => Ok(DataCompositionMutation::SetFieldRole(json(
            value,
            "field role",
        )?)),
        "modify-filter" => Ok(DataCompositionMutation::ModifyFilter(json(
            value,
            "filter patch",
        )?)),
        "modify-dataParameter" => Ok(DataCompositionMutation::ModifyDataParameter(json(
            value,
            "data parameter patch",
        )?)),
        "modify-parameter" => Ok(DataCompositionMutation::ModifyParameter(
            parse_data_composition_parameter_patch(value)?,
        )),
        "rename-parameter" => Ok(DataCompositionMutation::RenameParameter(json(
            value,
            "parameter rename",
        )?)),
        "reorder-parameters" => Ok(DataCompositionMutation::ReorderParameters(
            DataCompositionParameterOrder::new(parse_typed_list(
                value,
                DataCompositionParameterName::new,
                "parameter",
            )?)
            .map_err(|error| format!("invalid parameter order: {error}"))?,
        )),
        "remove-total" => Ok(DataCompositionMutation::RemoveTotal(path(value)?)),
        "remove-calculated-field" => {
            Ok(DataCompositionMutation::RemoveCalculatedField(path(value)?))
        }
        "remove-filter" => Ok(DataCompositionMutation::RemoveFilter(path(value)?)),
        _ => Err(format!(
            "unsupported data composition edit operation: {operation}"
        )),
    }
}

fn parse_data_composition_parameter_patch(
    value: &str,
) -> Result<DataCompositionParameterPatch, String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    enum Property {
        Title,
        Value,
        Expression,
        Hidden,
    }
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct Change {
        property: Property,
        value: DataCompositionValue,
    }
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct Patch {
        name: DataCompositionParameterName,
        changes: Vec<Change>,
    }

    let patch: Patch =
        serde_json::from_str(value).map_err(|error| format!("invalid parameter patch: {error}"))?;
    let changes = patch
        .changes
        .into_iter()
        .map(|change| match change.property {
            Property::Title => match change.value {
                DataCompositionValue::Text(value) => SynonymText::new(value.as_str().to_string())
                    .map(DataCompositionParameterChange::SetTitle)
                    .map_err(|error| format!("invalid parameter title: {error}")),
                _ => Err("parameter title requires text".to_string()),
            },
            Property::Value => Ok(DataCompositionParameterChange::SetValue(change.value)),
            Property::Expression => match change.value {
                DataCompositionValue::Text(value) => {
                    DataCompositionExpression::new(value.as_str().to_string())
                        .map(DataCompositionParameterChange::SetExpression)
                        .map_err(|error| format!("invalid parameter expression: {error}"))
                }
                _ => Err("parameter expression requires text".to_string()),
            },
            Property::Hidden => match change.value {
                DataCompositionValue::Boolean(value) => {
                    Ok(DataCompositionParameterChange::SetHidden(value))
                }
                _ => Err("parameter hidden flag requires a boolean".to_string()),
            },
        })
        .collect::<Result<Vec<_>, _>>()?;
    DataCompositionParameterPatch::new(patch.name, changes)
        .map_err(|error| format!("invalid parameter patch: {error}"))
}

fn parse_data_composition_scope(value: &str) -> Result<DataCompositionScope, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "" | "variant" => Ok(DataCompositionScope::Variant),
        "root" => Ok(DataCompositionScope::Root),
        other => Err(format!("unsupported data composition scope: {other}")),
    }
}

fn parse_template_kind(value: &str) -> Result<TemplateKind, String> {
    match value {
        "DataCompositionSchema" | "DCS" => Ok(TemplateKind::DataComposition),
        "SpreadsheetDocument" | "MXL" => Ok(TemplateKind::Spreadsheet),
        "TextDocument" | "Text" => Ok(TemplateKind::Text),
        "HTMLDocument" | "HTML" => Ok(TemplateKind::Html),
        "BinaryData" | "Binary" => Ok(TemplateKind::Binary),
        "GraphicalSchema" | "Graphical" => Ok(TemplateKind::Graphical),
        _ => Err(format!("unsupported template type: {value}")),
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
        writer_outcome, NativeMutationFileInputContract, TopLevelJsonInput,
    };
    use crate::application::{tools, ToolHandler};
    use crate::domain::workspace::WorkspaceContext;
    use serde_json::{Map, Value};
    use std::collections::BTreeMap;
    use unica_format_core::{
        commands::{
            DiagnosticCode, MutationMode, SemanticArtifact, SemanticChange, SupportCapability,
            SupportEdit, WriterCommand, WriterFailureKind, WriterLifecycle, WriterResult,
        },
        ports::OperationCancellation,
    };

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
                invoke_mutation(
                    operation,
                    tool.name,
                    &args,
                    &context,
                    &OperationCancellation::new(),
                )
                .is_some()
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

    #[test]
    fn writer_outcomes_recursively_exclude_paths_and_native_vocabulary() {
        fn collect_strings(value: &Value, strings: &mut Vec<String>) {
            match value {
                Value::String(value) => strings.push(value.clone()),
                Value::Array(values) => {
                    for value in values {
                        collect_strings(value, strings);
                    }
                }
                Value::Object(values) => {
                    for (key, value) in values {
                        strings.push(key.clone());
                        collect_strings(value, strings);
                    }
                }
                Value::Null | Value::Bool(_) | Value::Number(_) => {}
            }
        }

        let command =
            WriterCommand::SupportEdit(SupportEdit::Capability(SupportCapability::Enable));
        let applied = WriterResult::new(
            WriterLifecycle::Applied,
            [SemanticChange::SupportUpdated],
            [SemanticArtifact::SupportState],
            [],
        )
        .unwrap();
        let rejected =
            WriterResult::rejected(DiagnosticCode::PlannerRejected, WriterFailureKind::Planning);
        let recovery = WriterResult::publication_recovery_required();
        let outcomes = [
            writer_outcome(
                "unica.support.edit",
                &command,
                MutationMode::Apply,
                &applied,
            ),
            writer_outcome(
                "unica.support.edit",
                &command,
                MutationMode::Preview,
                &rejected,
            ),
            writer_outcome(
                "unica.support.edit",
                &command,
                MutationMode::Apply,
                &recovery,
            ),
        ];
        let mut strings = Vec::new();
        collect_strings(&serde_json::to_value(outcomes).unwrap(), &mut strings);
        let public_text = strings.join("\n");

        for forbidden in [
            "/Users/",
            r"C:\",
            ".xml",
            "platform_xml",
            "MetaDataObject",
            "MDClasses",
            "ParentConfigurations",
            "Configuration.xml",
            "2.20",
        ] {
            assert!(
                !public_text.contains(forbidden),
                "public writer outcome leaked `{forbidden}`: {public_text}"
            );
        }
        assert!(public_text.contains("support state"));
        assert!(public_text.contains("publication recovery is required"));
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
