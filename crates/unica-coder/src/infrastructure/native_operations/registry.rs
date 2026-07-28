use super::code;
use crate::application::AdapterOutcome;
use crate::domain::workspace::WorkspaceContext;
use serde_json::{Map, Value};
use std::path::PathBuf;
use unica_format_core::{
    commands::{
        ArtifactVersion, BorrowScope, ChildSubsystemReference, ConfigurationChildReference,
        ConfigurationEdit, ConfigurationHomePageArrangement, ConfigurationInitialize,
        ConfigurationInspection, ConfigurationMutation, ConfigurationName,
        ConfigurationPanelArrangement, ConfigurationPropertyChange, ConfigurationRoleReference,
        ConfigurationRoleSet, DataCompositionCalculatedFieldEdit, DataCompositionCreate,
        DataCompositionDataParameterEdit, DataCompositionDataSetEdit,
        DataCompositionDataSetLinkEdit, DataCompositionEdit, DataCompositionFieldEdit,
        DataCompositionFilterEdit, DataCompositionInspection, DataCompositionMutation,
        DataCompositionOrderEdit, DataCompositionParameterEdit, DataCompositionQueryPatch,
        DataCompositionQueryText, DataCompositionScopeReference, DataCompositionSelectionEdit,
        DataCompositionTotalEdit, DataCompositionVariantEdit, DataSetName, DefaultFormAssignment,
        DiagnosticCode, DiagnosticDetail, ExecutionContext, ExtensionBorrow, ExtensionInitialize,
        ExtensionInspection, ExtensionName, ExtensionPatchMethod, ExtensionPurpose,
        ExternalArtifactInitialize, ExternalArtifactName, FormCompile, FormCompilePurpose,
        FormCreate, FormEdit, FormInspection, FormName, FormOwnerReference, FormPurpose,
        FormRemove, HelpCreate, HelpOwnerReference, InspectionCommand, InspectionRequest,
        InterceptorKind, InterfaceEdit, InterfaceGroupOrder, InterfaceInspection,
        InterfaceItemOrder, InterfaceItemReference, InterfacePlacement, InterfaceSubsystemOrder,
        LanguageCode, MetadataAttributeOperand, MetadataCommandOperand, MetadataCreate,
        MetadataDimensionOperand, MetadataEdit, MetadataEnumValueOperand, MetadataFormOperand,
        MetadataInspection, MetadataMutation, MetadataObjectReference, MetadataPropertyOperand,
        MetadataRemove, MetadataRequisiteOperand, MetadataResourceOperand,
        MetadataTabularSectionAttributeOperand, MetadataTabularSectionOperand,
        MetadataTemplateOperand, MethodName, ModuleReference, MutationMode, NamePrefix,
        ProcessorName, RoleCreate, RoleInspection, RoleName, SemanticArtifact, SemanticChange,
        SpreadsheetCreate, SpreadsheetInspection, SubsystemContentReference, SubsystemCreate,
        SubsystemEdit, SubsystemInspection, SubsystemName, SubsystemPropertyChange,
        SupportCapability, SupportEdit, SupportObjectRule, SynonymText, TemplateCreate,
        TemplateInspection, TemplateKind, TemplateName, TemplateOwnerReference, TemplateRemove,
        VariantName, VendorName, WriterCommand, WriterDiagnostic, WriterEvidence, WriterLifecycle,
        WriterMessageCode, WriterSourceRole,
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
    let session_result = if matches!(command, WriterCommand::ExtensionPatchMethod(_)) {
        factory.capture_writer_session_with_extension_emitter(
            sources,
            inline_definition,
            adapter_hint,
            &context.workspace_root,
            &context.cwd,
            &context.cache_root,
            context.workspace_epoch,
            code::render_extension_method_patch,
        )
    } else {
        factory.capture_writer_session(
            sources,
            inline_definition,
            adapter_hint,
            &context.workspace_root,
            &context.cwd,
            &context.cache_root,
            context.workspace_epoch,
        )
    };
    let session = match session_result {
        Ok(session) => session,
        Err(error) => return Some((writer_failure(tool_name, error.message), None)),
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
        Err(error) => (writer_failure(tool_name, error.message), None),
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
        .map(|artifact| semantic_artifact_label(*artifact).to_string())
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

fn semantic_artifact_label(artifact: SemanticArtifact) -> &'static str {
    match artifact {
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

fn writer_command(
    operation: &str,
    args: &Map<String, Value>,
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
                ConfigurationEdit::from_definition()
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
            required_text(
                args,
                &["modulePath", "ModulePath"],
                "ModulePath",
                ModuleReference::new,
            )?,
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
            MetadataCreate::new()
                .omit_default_role(bool_arg(args, &["noRole", "NoRole"]))
                .assign_default_form(bool_arg(args, &["setDefault", "SetDefault"])),
        ),
        "meta-edit" => {
            let mut value = match first_string(args, &["object", "Object"]) {
                Some(object) => MetadataEdit::new(
                    MetadataObjectReference::new(object.to_string())
                        .map_err(|error| format!("invalid Object: {error}"))?,
                ),
                None => MetadataEdit::selected_object(),
            }
            .create_if_missing(bool_arg(args, &["createIfMissing", "CreateIfMissing"]));
            if let Some(operation) = first_string(args, &["operation", "Operation"]) {
                value = value.with_mutation(Some(parse_metadata_mutation(
                    operation,
                    required_string(args, &["value", "Value"], "Value")?,
                )?));
            }
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
                FormCompile::new(skip_validation)
            };
            WriterCommand::FormCompile(value)
        }
        "form-edit" => {
            WriterCommand::FormEdit(FormEdit::new(bool_arg(args, &["noValidate", "NoValidate"])))
        }
        "form-remove" => WriterCommand::FormRemove(FormRemove::new(
            required_text(
                args,
                &["objectName", "ObjectName"],
                "ObjectName",
                FormOwnerReference::new,
            )?,
            required_text(args, &["formName", "FormName"], "FormName", FormName::new)?,
        )),
        "template-add" => WriterCommand::TemplateCreate(
            TemplateCreate::new(
                required_text(
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
            required_text(
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
            required_text(
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
                None => InterfaceEdit::FromDefinition,
            })
        }
        "role-compile" => WriterCommand::RoleCreate(RoleCreate::new(optional_text(
            args,
            &["name", "Name"],
            RoleName::new,
        )?)),
        "subsystem-compile" => WriterCommand::SubsystemCreate(SubsystemCreate::new(optional_text(
            args,
            &["name", "Name"],
            SubsystemName::new,
        )?)),
        "subsystem-edit" => {
            WriterCommand::SubsystemEdit(match first_string(args, &["operation", "Operation"]) {
                Some(operation) => parse_subsystem_edit(
                    operation,
                    required_string(args, &["value", "Value"], "Value")?,
                )?,
                None => SubsystemEdit::FromDefinition,
            })
        }
        "support-edit" => WriterCommand::SupportEdit(parse_support_edit(args)?),
        "dcs-compile" => WriterCommand::DataCompositionCreate(DataCompositionCreate),
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
        "mxl-compile" => WriterCommand::SpreadsheetCreate(
            SpreadsheetCreate::new()
                .with_processor(optional_text(
                    args,
                    &["processorName", "ProcessorName"],
                    ProcessorName::new,
                )?)
                .with_template(optional_text(
                    args,
                    &["templateName", "TemplateName"],
                    TemplateName::new,
                )?)
                .derive_from_object(bool_arg(args, &["fromObject", "FromObject"])),
        ),
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

fn semantic_operand<T>(
    label: &str,
    value: &str,
    constructor: impl FnOnce(String) -> Result<T, unica_format_core::commands::SemanticValueError>,
) -> Result<T, String> {
    constructor(value.to_string()).map_err(|error| format!("invalid {label}: {error}"))
}

fn parse_configuration_mutation(
    operation: &str,
    value: &str,
) -> Result<ConfigurationMutation, String> {
    match operation {
        "modify-property" => Ok(ConfigurationMutation::ModifyProperty(semantic_operand(
            "configuration property change",
            value,
            ConfigurationPropertyChange::new,
        )?)),
        "remove-childObject" => Ok(ConfigurationMutation::RemoveChild(semantic_operand(
            "configuration child reference",
            value,
            ConfigurationChildReference::new,
        )?)),
        "add-childObject" => Ok(ConfigurationMutation::AddChild(semantic_operand(
            "configuration child reference",
            value,
            ConfigurationChildReference::new,
        )?)),
        "set-defaultRoles" => Ok(ConfigurationMutation::SetDefaultRoles(semantic_operand(
            "configuration role set",
            value,
            ConfigurationRoleSet::new,
        )?)),
        "add-defaultRole" => Ok(ConfigurationMutation::AddDefaultRole(semantic_operand(
            "configuration role reference",
            value,
            ConfigurationRoleReference::new,
        )?)),
        "remove-defaultRole" => Ok(ConfigurationMutation::RemoveDefaultRole(semantic_operand(
            "configuration role reference",
            value,
            ConfigurationRoleReference::new,
        )?)),
        "set-panels" => Ok(ConfigurationMutation::SetPanels(semantic_operand(
            "configuration panel arrangement",
            value,
            ConfigurationPanelArrangement::new,
        )?)),
        "set-home-page" => Ok(ConfigurationMutation::SetHomePage(semantic_operand(
            "configuration home page arrangement",
            value,
            ConfigurationHomePageArrangement::new,
        )?)),
        _ => Err(format!(
            "unsupported configuration edit operation: {operation}"
        )),
    }
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

fn parse_metadata_mutation(operation: &str, value: &str) -> Result<MetadataMutation, String> {
    let (action, target) = operation
        .split_once('-')
        .ok_or_else(|| format!("invalid metadata edit operation: {operation}"))?;
    macro_rules! operand {
        ($type:ty, $label:literal) => {
            semantic_operand($label, value, <$type>::new)?
        };
    }
    match (action, target) {
        ("add", "property") => Ok(MetadataMutation::AddProperty(operand!(
            MetadataPropertyOperand,
            "metadata property operand"
        ))),
        ("remove", "property") => Ok(MetadataMutation::RemoveProperty(operand!(
            MetadataPropertyOperand,
            "metadata property operand"
        ))),
        ("modify" | "set", "property") => Ok(MetadataMutation::ModifyProperty(operand!(
            MetadataPropertyOperand,
            "metadata property operand"
        ))),
        ("add", "attribute") => Ok(MetadataMutation::AddAttribute(operand!(
            MetadataAttributeOperand,
            "metadata attribute operand"
        ))),
        ("remove", "attribute") => Ok(MetadataMutation::RemoveAttribute(operand!(
            MetadataAttributeOperand,
            "metadata attribute operand"
        ))),
        ("modify" | "set", "attribute") => Ok(MetadataMutation::ModifyAttribute(operand!(
            MetadataAttributeOperand,
            "metadata attribute operand"
        ))),
        ("add", "tabular-section" | "tabularSection") => {
            Ok(MetadataMutation::AddTabularSection(operand!(
                MetadataTabularSectionOperand,
                "metadata tabular section operand"
            )))
        }
        ("remove", "tabular-section" | "tabularSection") => {
            Ok(MetadataMutation::RemoveTabularSection(operand!(
                MetadataTabularSectionOperand,
                "metadata tabular section operand"
            )))
        }
        ("modify" | "set", "tabular-section" | "tabularSection") => {
            Ok(MetadataMutation::ModifyTabularSection(operand!(
                MetadataTabularSectionOperand,
                "metadata tabular section operand"
            )))
        }
        ("add", "ts-attribute" | "tabular-section-attribute") => {
            Ok(MetadataMutation::AddTabularSectionAttribute(operand!(
                MetadataTabularSectionAttributeOperand,
                "metadata tabular section attribute operand"
            )))
        }
        ("remove", "ts-attribute" | "tabular-section-attribute") => {
            Ok(MetadataMutation::RemoveTabularSectionAttribute(operand!(
                MetadataTabularSectionAttributeOperand,
                "metadata tabular section attribute operand"
            )))
        }
        ("modify" | "set", "ts-attribute" | "tabular-section-attribute") => {
            Ok(MetadataMutation::ModifyTabularSectionAttribute(operand!(
                MetadataTabularSectionAttributeOperand,
                "metadata tabular section attribute operand"
            )))
        }
        ("add", "form") => Ok(MetadataMutation::AddForm(operand!(
            MetadataFormOperand,
            "metadata form operand"
        ))),
        ("remove", "form") => Ok(MetadataMutation::RemoveForm(operand!(
            MetadataFormOperand,
            "metadata form operand"
        ))),
        ("modify" | "set", "form") => Ok(MetadataMutation::ModifyForm(operand!(
            MetadataFormOperand,
            "metadata form operand"
        ))),
        ("add", "template") => Ok(MetadataMutation::AddTemplate(operand!(
            MetadataTemplateOperand,
            "metadata template operand"
        ))),
        ("remove", "template") => Ok(MetadataMutation::RemoveTemplate(operand!(
            MetadataTemplateOperand,
            "metadata template operand"
        ))),
        ("modify" | "set", "template") => Ok(MetadataMutation::ModifyTemplate(operand!(
            MetadataTemplateOperand,
            "metadata template operand"
        ))),
        ("add", "command") => Ok(MetadataMutation::AddCommand(operand!(
            MetadataCommandOperand,
            "metadata command operand"
        ))),
        ("remove", "command") => Ok(MetadataMutation::RemoveCommand(operand!(
            MetadataCommandOperand,
            "metadata command operand"
        ))),
        ("modify" | "set", "command") => Ok(MetadataMutation::ModifyCommand(operand!(
            MetadataCommandOperand,
            "metadata command operand"
        ))),
        ("add", "dimension") => Ok(MetadataMutation::AddDimension(operand!(
            MetadataDimensionOperand,
            "metadata dimension operand"
        ))),
        ("remove", "dimension") => Ok(MetadataMutation::RemoveDimension(operand!(
            MetadataDimensionOperand,
            "metadata dimension operand"
        ))),
        ("modify" | "set", "dimension") => Ok(MetadataMutation::ModifyDimension(operand!(
            MetadataDimensionOperand,
            "metadata dimension operand"
        ))),
        ("add", "resource") => Ok(MetadataMutation::AddResource(operand!(
            MetadataResourceOperand,
            "metadata resource operand"
        ))),
        ("remove", "resource") => Ok(MetadataMutation::RemoveResource(operand!(
            MetadataResourceOperand,
            "metadata resource operand"
        ))),
        ("modify" | "set", "resource") => Ok(MetadataMutation::ModifyResource(operand!(
            MetadataResourceOperand,
            "metadata resource operand"
        ))),
        ("add", "requisite") => Ok(MetadataMutation::AddRequisite(operand!(
            MetadataRequisiteOperand,
            "metadata requisite operand"
        ))),
        ("remove", "requisite") => Ok(MetadataMutation::RemoveRequisite(operand!(
            MetadataRequisiteOperand,
            "metadata requisite operand"
        ))),
        ("modify" | "set", "requisite") => Ok(MetadataMutation::ModifyRequisite(operand!(
            MetadataRequisiteOperand,
            "metadata requisite operand"
        ))),
        ("add", "enum-value" | "value") => Ok(MetadataMutation::AddEnumValue(operand!(
            MetadataEnumValueOperand,
            "metadata enum value operand"
        ))),
        ("remove", "enum-value" | "value") => Ok(MetadataMutation::RemoveEnumValue(operand!(
            MetadataEnumValueOperand,
            "metadata enum value operand"
        ))),
        ("modify" | "set", "enum-value" | "value") => Ok(MetadataMutation::ModifyEnumValue(
            operand!(MetadataEnumValueOperand, "metadata enum value operand"),
        )),
        ("add" | "remove" | "modify" | "set", _) => {
            Err(format!("unsupported metadata edit target: {target}"))
        }
        _ => Err(format!("unsupported metadata edit action: {action}")),
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

fn parse_interface_edit(operation: &str, value: &str) -> Result<InterfaceEdit, String> {
    match operation {
        "hide" => Ok(InterfaceEdit::Hide(semantic_operand(
            "interface item reference",
            value,
            InterfaceItemReference::new,
        )?)),
        "show" => Ok(InterfaceEdit::Show(semantic_operand(
            "interface item reference",
            value,
            InterfaceItemReference::new,
        )?)),
        "place" => Ok(InterfaceEdit::Place(semantic_operand(
            "interface placement",
            value,
            InterfacePlacement::new,
        )?)),
        "order" => Ok(InterfaceEdit::Order(semantic_operand(
            "interface item order",
            value,
            InterfaceItemOrder::new,
        )?)),
        "subsystem-order" => Ok(InterfaceEdit::OrderSubsystems(semantic_operand(
            "interface subsystem order",
            value,
            InterfaceSubsystemOrder::new,
        )?)),
        "group-order" => Ok(InterfaceEdit::OrderGroups(semantic_operand(
            "interface group order",
            value,
            InterfaceGroupOrder::new,
        )?)),
        _ => Err(format!("unsupported interface edit operation: {operation}")),
    }
}

fn parse_subsystem_edit(operation: &str, value: &str) -> Result<SubsystemEdit, String> {
    match operation {
        "add-content" => Ok(SubsystemEdit::AddContent(semantic_operand(
            "subsystem content reference",
            value,
            SubsystemContentReference::new,
        )?)),
        "remove-content" => Ok(SubsystemEdit::RemoveContent(semantic_operand(
            "subsystem content reference",
            value,
            SubsystemContentReference::new,
        )?)),
        "add-child" => Ok(SubsystemEdit::AddChild(semantic_operand(
            "child subsystem reference",
            value,
            ChildSubsystemReference::new,
        )?)),
        "remove-child" => Ok(SubsystemEdit::RemoveChild(semantic_operand(
            "child subsystem reference",
            value,
            ChildSubsystemReference::new,
        )?)),
        "set-property" => Ok(SubsystemEdit::SetProperty(semantic_operand(
            "subsystem property change",
            value,
            SubsystemPropertyChange::new,
        )?)),
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
    macro_rules! operand {
        ($type:ty, $label:literal) => {
            semantic_operand($label, value, <$type>::new)?
        };
    }
    match operation {
        "add-field" => Ok(DataCompositionMutation::AddField(operand!(
            DataCompositionFieldEdit,
            "data composition field edit"
        ))),
        "add-total" => Ok(DataCompositionMutation::AddTotal(operand!(
            DataCompositionTotalEdit,
            "data composition total edit"
        ))),
        "add-calculated-field" => Ok(DataCompositionMutation::AddCalculatedField(operand!(
            DataCompositionCalculatedFieldEdit,
            "data composition calculated field edit"
        ))),
        "add-parameter" => Ok(DataCompositionMutation::AddParameter(operand!(
            DataCompositionParameterEdit,
            "data composition parameter edit"
        ))),
        "add-filter" => Ok(DataCompositionMutation::AddFilter(operand!(
            DataCompositionFilterEdit,
            "data composition filter edit"
        ))),
        "add-dataParameter" => Ok(DataCompositionMutation::AddDataParameter(operand!(
            DataCompositionDataParameterEdit,
            "data composition data parameter edit"
        ))),
        "set-query" => Ok(DataCompositionMutation::SetQuery(operand!(
            DataCompositionQueryText,
            "data composition query text"
        ))),
        "patch-query" => Ok(DataCompositionMutation::PatchQuery(operand!(
            DataCompositionQueryPatch,
            "data composition query patch"
        ))),
        "clear-selection" => Ok(DataCompositionMutation::ClearSelection(operand!(
            DataCompositionScopeReference,
            "data composition scope reference"
        ))),
        "clear-order" => Ok(DataCompositionMutation::ClearOrder(operand!(
            DataCompositionScopeReference,
            "data composition scope reference"
        ))),
        "clear-filter" => Ok(DataCompositionMutation::ClearFilter(operand!(
            DataCompositionScopeReference,
            "data composition scope reference"
        ))),
        "clear-conditionalAppearance" => Ok(DataCompositionMutation::ClearConditionalAppearance(
            operand!(
                DataCompositionScopeReference,
                "data composition scope reference"
            ),
        )),
        "add-selection" => Ok(DataCompositionMutation::AddSelection(operand!(
            DataCompositionSelectionEdit,
            "data composition selection edit"
        ))),
        "add-order" => Ok(DataCompositionMutation::AddOrder(operand!(
            DataCompositionOrderEdit,
            "data composition order edit"
        ))),
        "add-dataSetLink" => Ok(DataCompositionMutation::AddDataSetLink(operand!(
            DataCompositionDataSetLinkEdit,
            "data composition data set link edit"
        ))),
        "add-dataSet" => Ok(DataCompositionMutation::AddDataSet(operand!(
            DataCompositionDataSetEdit,
            "data composition data set edit"
        ))),
        "add-variant" => Ok(DataCompositionMutation::AddVariant(operand!(
            DataCompositionVariantEdit,
            "data composition variant edit"
        ))),
        _ => Err(format!(
            "unsupported data composition edit operation: {operation}"
        )),
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
