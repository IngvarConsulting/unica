use serde::Serialize;
use serde_json::{Map, Value};
use std::{collections::BTreeMap, fs, path::PathBuf};
use unica_format_core::{
    commands::{
        InspectionPort, InspectionRequest, MutationMode, WriterArgument, WriterBorrowScope,
        WriterCommand, WriterEvidence, WriterFamily, WriterResult, WriterSourceRole,
    },
    ports::{WriterPort, WriterRequest},
    source::{SourceAdapterError, SourceAdapterErrorKind},
};

#[cfg(test)]
pub(crate) use crate::versions::v2_20::writers::testing;
pub(crate) use crate::versions::v2_20::writers::{
    cf, cfe, common, compile_transaction, dcs, external, filesystem, form, form_edit,
    form_event_registry, help, interface, meta, mxl, operation_descriptors, platform_xml_owner,
    project_source_types, project_sources, registry, role, single_file_publisher,
    source_root_types, source_roots, subsystem, support, template,
};
pub(crate) use crate::versions::v2_20::writers::{
    is_1c_identifier, is_1c_identifier_part, is_1c_identifier_start, FormatProfile,
    ACTIVE_FORMAT_PROFILE,
};

#[derive(Debug, Clone, Serialize)]
pub(crate) struct NativeWriterResult {
    pub(crate) ok: bool,
    pub(crate) summary: String,
    pub(crate) changes: Vec<String>,
    pub(crate) warnings: Vec<String>,
    pub(crate) errors: Vec<String>,
    pub(crate) artifacts: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) stdout: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) stderr: Option<String>,
}

impl NativeWriterResult {
    pub(crate) fn ok(summary: impl Into<String>) -> Self {
        Self {
            ok: true,
            summary: summary.into(),
            changes: Vec::new(),
            warnings: Vec::new(),
            errors: Vec::new(),
            artifacts: Vec::new(),
            stdout: None,
            stderr: None,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct WorkspaceContext {
    pub(crate) cwd: PathBuf,
    pub(crate) workspace_root: PathBuf,
    pub(crate) cache_root: PathBuf,
    pub(crate) workspace_epoch: u64,
}

#[derive(Debug, Clone)]
pub(crate) struct PlatformWriterSession {
    sources: BTreeMap<WriterSourceRole, PathBuf>,
    inline_definition: Option<Vec<u8>>,
    adapter_hint: Option<String>,
    context: WorkspaceContext,
}

impl PlatformWriterSession {
    pub(crate) fn new<I>(
        sources: I,
        inline_definition: Option<Vec<u8>>,
        adapter_hint: Option<String>,
        context: WorkspaceContext,
    ) -> Result<Self, String>
    where
        I: IntoIterator<Item = (WriterSourceRole, PathBuf)>,
    {
        let mut captured = BTreeMap::new();
        for (role, source) in sources {
            if captured.insert(role, source).is_some() {
                return Err("writer source role was bound more than once".to_string());
            }
        }
        Ok(Self {
            sources: captured,
            inline_definition,
            adapter_hint,
            context,
        })
    }
}

#[derive(Debug, Clone)]
pub(crate) struct PlatformInspectionSession {
    operation: String,
    tool_name: String,
    args: Map<String, Value>,
    context: WorkspaceContext,
}

impl PlatformInspectionSession {
    pub(crate) fn new(
        operation: impl Into<String>,
        tool_name: impl Into<String>,
        args: Map<String, Value>,
        context: WorkspaceContext,
    ) -> Self {
        Self {
            operation: operation.into(),
            tool_name: tool_name.into(),
            args,
            context,
        }
    }
}

pub(crate) struct NativeOperationAdapter;
pub(crate) struct PlatformXmlWriter;
pub(crate) struct PlatformXmlInspector;

impl WriterPort for PlatformXmlWriter {
    fn families(&self) -> &'static [WriterFamily] {
        &WriterFamily::ALL
    }

    fn execute(&self, request: &WriterRequest) -> Result<WriterResult, SourceAdapterError> {
        let session = request
            .session()
            .adapter_state::<PlatformWriterSession>()
            .ok_or_else(|| {
                SourceAdapterError::new(
                    SourceAdapterErrorKind::CapabilityBlocked,
                    "writer command has no bound Platform XML execution session",
                )
            })?;
        if request.cancellation().is_cancelled() {
            return Ok(WriterResult::cancelled());
        }
        let operation = writer_operation(request.command());
        let tool_name = writer_tool_name(operation);
        let args = writer_native_arguments(request.command(), session, request.mode())?;
        let (outcome, evidence) = if operation == "form-edit" && form::has_edit_payload(&args) {
            let execution = match request.mode() {
                MutationMode::Preview => form::preview_with_data(&args, &session.context),
                MutationMode::Apply => form::apply_with_data(&args, &session.context),
            };
            let (outcome, data) = execution.into_core_parts();
            (outcome, data.map(WriterEvidence::FormEdit))
        } else {
            let outcome = match request.mode() {
                MutationMode::Preview => {
                    NativeOperationAdapter::preview(operation, tool_name, &args, &session.context)
                }
                MutationMode::Apply => {
                    registry::invoke_mutation(operation, tool_name, &args, &session.context)
                        .ok_or_else(|| {
                            "Platform XML writer operation is not registered".to_string()
                        })
                }
            }
            .map_err(|message| {
                SourceAdapterError::new(SourceAdapterErrorKind::ValidationFailed, message)
            })?;
            (outcome, None)
        };
        Ok(WriterResult::from_parts(
            outcome.ok,
            request.mode(),
            outcome.summary,
            outcome.changes,
            outcome.warnings,
            outcome.errors,
            outcome.artifacts,
            outcome.stdout,
            outcome.stderr,
        )
        .with_evidence(evidence))
    }
}

impl InspectionPort for PlatformXmlInspector {
    fn inspect(&self, request: &InspectionRequest) -> Result<WriterResult, SourceAdapterError> {
        let session = request
            .session()
            .adapter_state::<PlatformInspectionSession>()
            .ok_or_else(|| {
                SourceAdapterError::new(
                    SourceAdapterErrorKind::CapabilityBlocked,
                    "inspection command has no bound Platform XML execution session",
                )
            })?;
        if request.command().intent() != inspection_intent(&session.operation) {
            return Err(SourceAdapterError::new(
                SourceAdapterErrorKind::CapabilityBlocked,
                "inspection command does not match the bound operation",
            ));
        }
        if request.cancellation().is_cancelled() {
            return Ok(WriterResult::cancelled());
        }
        let outcome = NativeOperationAdapter::read(
            &session.operation,
            &session.tool_name,
            &session.args,
            &session.context,
        )
        .map_err(|message| {
            SourceAdapterError::new(SourceAdapterErrorKind::ValidationFailed, message)
        })?;
        Ok(WriterResult::from_parts(
            outcome.ok,
            MutationMode::Apply,
            outcome.summary,
            outcome.changes,
            outcome.warnings,
            outcome.errors,
            outcome.artifacts,
            outcome.stdout,
            outcome.stderr,
        ))
    }
}

impl NativeOperationAdapter {
    fn preview(
        operation: &str,
        tool_name: &str,
        args: &Map<String, Value>,
        context: &WorkspaceContext,
    ) -> Result<NativeWriterResult, String> {
        if let Some(outcome) = external::preview(operation, tool_name, args, context) {
            return Ok(outcome);
        }
        if operation == "form-edit" && form::has_edit_payload(args) {
            return Ok(form::preview_form_edit(args, context));
        }
        if operation == "meta-edit" {
            meta::validate_meta_edit_preview(args, context)?;
            return Ok(NativeWriterResult {
                ok: true,
                summary: format!("dry run: {tool_name} planned native metadata edit"),
                changes: vec!["no files changed because dryRun is true".to_string()],
                warnings: Vec::new(),
                errors: Vec::new(),
                artifacts: Vec::new(),
                stdout: None,
                stderr: None,
            });
        }
        if let Some(preview) = registry::invoke_preview(operation, args, context) {
            return match preview {
                registry::PreviewInvocation::Unavailable(error) => Ok(NativeWriterResult {
                    ok: true,
                    summary: format!("dry run: {tool_name} would execute native XML/DSL operation"),
                    changes: vec!["no files changed because dryRun is true".to_string()],
                    warnings: vec![format!(
                        "detailed compile preview is unavailable; using safe placeholder: {error}"
                    )],
                    errors: Vec::new(),
                    artifacts: Vec::new(),
                    stdout: None,
                    stderr: None,
                }),
                registry::PreviewInvocation::Planned(result) => result,
            };
        }
        Ok(NativeWriterResult {
            ok: true,
            summary: format!("dry run: {tool_name} would execute native XML/DSL operation"),
            changes: vec!["no files changed because dryRun is true".to_string()],
            warnings: Vec::new(),
            errors: Vec::new(),
            artifacts: Vec::new(),
            stdout: None,
            stderr: None,
        })
    }

    fn read(
        operation: &str,
        tool_name: &str,
        args: &Map<String, Value>,
        context: &WorkspaceContext,
    ) -> Result<NativeWriterResult, String> {
        if let Some(outcome) = registry::invoke_read(operation, tool_name, args, context) {
            return outcome;
        }
        let target = common::resolve_target(operation, args, context)?;
        let text = fs::read_to_string(&target)
            .map_err(|error| format!("failed to read {}: {error}", target.display()))?;
        Ok(common::analyze_xml(operation, tool_name, &target, &text))
    }

    #[cfg(test)]
    pub(crate) fn invoke(
        operation: &str,
        tool_name: &str,
        args: &Map<String, Value>,
        context: &WorkspaceContext,
        dry_run: bool,
        mutating: bool,
    ) -> Result<NativeWriterResult, String> {
        if dry_run && mutating {
            return Self::preview(operation, tool_name, args, context);
        }
        if mutating {
            return registry::invoke_mutation(operation, tool_name, args, context)
                .ok_or_else(|| format!("writer operation is not registered: {operation}"));
        }
        Self::read(operation, tool_name, args, context)
    }

    #[cfg(test)]
    pub(crate) fn invoke_with_data(
        operation: &str,
        tool_name: &str,
        args: &Map<String, Value>,
        context: &WorkspaceContext,
        dry_run: bool,
        mutating: bool,
    ) -> Result<TestNativeOperationResult, String> {
        if operation == "form-edit" && form::has_edit_payload(args) {
            let execution = if dry_run {
                form::preview_with_data(args, context)
            } else {
                form::apply_with_data(args, context)
            };
            let data = execution
                .data
                .map(serde_json::to_value)
                .transpose()
                .map_err(|error| format!("serialize form edit result: {error}"))?;
            return Ok(TestNativeOperationResult {
                adapter: execution.outcome,
                data,
            });
        }
        Self::invoke(operation, tool_name, args, context, dry_run, mutating).map(|adapter| {
            TestNativeOperationResult {
                adapter,
                data: None,
            }
        })
    }
}

#[cfg(test)]
pub(crate) struct TestNativeOperationResult {
    pub(crate) adapter: NativeWriterResult,
    pub(crate) data: Option<Value>,
}

fn writer_native_arguments(
    command: &WriterCommand,
    session: &PlatformWriterSession,
    mode: MutationMode,
) -> Result<Map<String, Value>, SourceAdapterError> {
    let operation = writer_operation(command);
    let mut args = Map::new();
    for argument in command.arguments().items() {
        match argument {
            WriterArgument::Name(value) => insert_writer_text(&mut args, operation, "name", value),
            WriterArgument::Synonym(value) => {
                insert_writer_text(&mut args, operation, "synonym", value)
            }
            WriterArgument::Vendor(value) => insert_text(&mut args, "vendor", value),
            WriterArgument::ArtifactVersion(value) => insert_text(&mut args, "version", value),
            WriterArgument::Purpose(value) => insert_text(&mut args, "purpose", value),
            WriterArgument::BorrowMainAttribute(scope) => insert_text(
                &mut args,
                "borrowMainAttribute",
                match scope {
                    WriterBorrowScope::Form => "Form",
                    WriterBorrowScope::All => "All",
                },
            ),
            WriterArgument::Mode(value) => insert_text(&mut args, "mode", value),
            WriterArgument::ObjectReference(value) => insert_text(&mut args, "object", value),
            WriterArgument::NamePrefix(value) => insert_text(&mut args, "namePrefix", value),
            WriterArgument::ModuleReference(value) => insert_text(&mut args, "modulePath", value),
            WriterArgument::MethodName(value) => insert_text(&mut args, "methodName", value),
            WriterArgument::InterceptorType(value) => {
                insert_text(&mut args, "interceptorType", value)
            }
            WriterArgument::ExecutionContext(value) => insert_text(&mut args, "context", value),
            WriterArgument::ObjectName(value) => insert_text(&mut args, "objectName", value),
            WriterArgument::FormName(value) => {
                insert_writer_text(&mut args, operation, "formName", value)
            }
            WriterArgument::TemplateName(value) => insert_text(&mut args, "templateName", value),
            WriterArgument::TemplateType(value) => insert_text(&mut args, "templateType", value),
            WriterArgument::Language(value) => insert_text(&mut args, "lang", value),
            WriterArgument::MutationVerb(value) => insert_text(&mut args, "operation", value),
            WriterArgument::MutationValue(value) => insert_text(&mut args, "value", value),
            WriterArgument::DataSet(value) => insert_text(&mut args, "dataSet", value),
            WriterArgument::Variant(value) => insert_text(&mut args, "variant", value),
            WriterArgument::ProcessorName(value) => insert_text(&mut args, "processorName", value),
            WriterArgument::SupportCapability(value) => insert_text(&mut args, "capability", value),
            WriterArgument::SupportRule(value) => insert_text(&mut args, "set", value),
            WriterArgument::OmitRole(value) => insert_bool(&mut args, "noRole", *value),
            WriterArgument::Function(value) => insert_bool(&mut args, "isFunction", *value),
            WriterArgument::AssignDefaultForm(value) => {
                insert_bool(&mut args, "setDefault", *value)
            }
            WriterArgument::AssignMainDataComposition(value) => {
                insert_bool(&mut args, "setMainSKD", *value)
            }
            WriterArgument::SkipValidation(value) => insert_bool(&mut args, "noValidate", *value),
            WriterArgument::ExcludeSelection(value) => {
                insert_bool(&mut args, "noSelection", *value)
            }
            WriterArgument::CreateIfMissing(value) => {
                insert_bool(&mut args, "createIfMissing", *value)
            }
            WriterArgument::Force(value) => insert_bool(&mut args, "force", *value),
            WriterArgument::KeepFiles(value) => insert_bool(&mut args, "keepFiles", *value),
            WriterArgument::DeriveFromObject(value) => insert_bool(&mut args, "fromObject", *value),
        }
    }

    for (role, path) in &session.sources {
        let key = writer_source_key(command.intent(), *role).ok_or_else(|| {
            SourceAdapterError::new(
                SourceAdapterErrorKind::CapabilityBlocked,
                "writer source role does not belong to the semantic command",
            )
        })?;
        let key = if matches!(operation, "epf-init" | "erf-init") && key == "outputDir" {
            "OutputDir"
        } else {
            key
        };
        args.insert(
            key.to_string(),
            Value::String(path.to_string_lossy().into_owned()),
        );
    }

    if let Some(hint) = &session.adapter_hint {
        insert_text(&mut args, "compatibilityMode", hint);
    }
    if let Some(definition) = &session.inline_definition {
        match command.intent() {
            "form.edit" => {
                let value = serde_json::from_slice(definition).map_err(|error| {
                    SourceAdapterError::new(
                        SourceAdapterErrorKind::ValidationFailed,
                        format!("inline form definition is not valid JSON: {error}"),
                    )
                })?;
                args.insert("definition".to_string(), value);
            }
            "subsystem.create" | "dataComposition.create" => {
                let value = String::from_utf8(definition.clone()).map_err(|error| {
                    SourceAdapterError::new(
                        SourceAdapterErrorKind::ValidationFailed,
                        format!("inline semantic definition is not UTF-8: {error}"),
                    )
                })?;
                insert_text(&mut args, "value", &value);
            }
            _ => {
                return Err(SourceAdapterError::new(
                    SourceAdapterErrorKind::CapabilityBlocked,
                    "inline definition does not belong to the semantic command",
                ));
            }
        }
    }
    insert_bool(&mut args, "DryRun", mode.is_preview());
    Ok(args)
}

fn insert_writer_text(args: &mut Map<String, Value>, operation: &str, key: &str, value: &str) {
    let key = if matches!(operation, "epf-init" | "erf-init") {
        match key {
            "name" => "Name",
            "synonym" => "Synonym",
            "formName" => "FormName",
            _ => key,
        }
    } else {
        key
    };
    insert_text(args, key, value);
}

fn insert_text(args: &mut Map<String, Value>, key: &str, value: &str) {
    args.insert(key.to_string(), Value::String(value.to_string()));
}

fn insert_bool(args: &mut Map<String, Value>, key: &str, value: bool) {
    args.insert(key.to_string(), Value::Bool(value));
}

fn writer_source_key(intent: &str, role: WriterSourceRole) -> Option<&'static str> {
    Some(match role {
        WriterSourceRole::Configuration => "configPath",
        WriterSourceRole::ConfigurationDirectory => "configDir",
        WriterSourceRole::Extension => "extensionPath",
        WriterSourceRole::DestinationDirectory => "outputDir",
        WriterSourceRole::Definition => match intent {
            "metadata.create" | "form.compile" | "form.edit" | "role.create"
            | "spreadsheet.create" => "jsonPath",
            "configuration.edit"
            | "metadata.edit"
            | "interface.edit"
            | "subsystem.create"
            | "subsystem.edit"
            | "dataComposition.create"
            | "dataComposition.edit" => "definitionFile",
            _ => return None,
        },
        WriterSourceRole::Object if intent == "help.create" => "objectName",
        WriterSourceRole::Object => "objectPath",
        WriterSourceRole::SourceCollection => "srcDir",
        WriterSourceRole::Form => "formPath",
        WriterSourceRole::Interface => "ciPath",
        WriterSourceRole::Subsystem => "subsystemPath",
        WriterSourceRole::DestinationArtifact => "outputPath",
        WriterSourceRole::ParentSubsystem => "parent",
        WriterSourceRole::Template => "templatePath",
        WriterSourceRole::Rights => "rightsPath",
        WriterSourceRole::SupportTarget => "path",
    })
}

fn writer_operation(command: &WriterCommand) -> &'static str {
    match command.intent() {
        "configuration.initialize" => "cf-init",
        "configuration.edit" => "cf-edit",
        "extension.initialize" => "cfe-init",
        "extension.borrow" => "cfe-borrow",
        "extension.patchMethod" => "cfe-patch-method",
        "externalArtifact.initializeProcessor" => "epf-init",
        "externalArtifact.initializeReport" => "erf-init",
        "metadata.create" => "meta-compile",
        "metadata.edit" => "meta-edit",
        "metadata.remove" => "meta-remove",
        "form.create" => "form-add",
        "form.compile" => "form-compile",
        "form.edit" => "form-edit",
        "form.remove" => "form-remove",
        "template.create" => "template-add",
        "template.remove" => "template-remove",
        "help.create" => "help-add",
        "interface.edit" => "interface-edit",
        "role.create" => "role-compile",
        "subsystem.create" => "subsystem-compile",
        "subsystem.edit" => "subsystem-edit",
        "support.edit" => "support-edit",
        "dataComposition.create" => "dcs-compile",
        "dataComposition.edit" => "dcs-edit",
        "spreadsheet.create" => "mxl-compile",
        _ => unreachable!("WriterCommand intent is closed"),
    }
}

fn writer_tool_name(operation: &str) -> &'static str {
    match operation {
        "cf-init" => "unica.cf.init",
        "cf-edit" => "unica.cf.edit",
        "cfe-init" => "unica.cfe.init",
        "cfe-borrow" => "unica.cfe.borrow",
        "cfe-patch-method" => "unica.cfe.patch_method",
        "epf-init" => "unica.epf.init",
        "erf-init" => "unica.erf.init",
        "meta-compile" => "unica.meta.compile",
        "meta-edit" => "unica.meta.edit",
        "meta-remove" => "unica.meta.remove",
        "form-add" => "unica.form.add",
        "form-compile" => "unica.form.compile",
        "form-edit" => "unica.form.edit",
        "form-remove" => "unica.form.remove",
        "template-add" => "unica.template.add",
        "template-remove" => "unica.template.remove",
        "help-add" => "unica.help.add",
        "interface-edit" => "unica.interface.edit",
        "role-compile" => "unica.role.compile",
        "subsystem-compile" => "unica.subsystem.compile",
        "subsystem-edit" => "unica.subsystem.edit",
        "support-edit" => "unica.support.edit",
        "dcs-compile" => "unica.dcs.compile",
        "dcs-edit" => "unica.dcs.edit",
        "mxl-compile" => "unica.mxl.compile",
        _ => unreachable!("writer operation is closed"),
    }
}

fn inspection_intent(operation: &str) -> &'static str {
    match operation {
        "cf-info" => "configuration.describe",
        "cf-validate" => "configuration.validate",
        "cfe-diff" => "extension.compare",
        "cfe-validate" => "extension.validate",
        "meta-info" => "metadata.describe",
        "meta-validate" => "metadata.validate",
        "form-info" => "form.describe",
        "form-validate" => "form.validate",
        "interface-validate" => "interface.validate",
        "subsystem-info" => "subsystem.describe",
        "subsystem-validate" => "subsystem.validate",
        "template-info" => "template.describe",
        "template-validate" => "template.validate",
        "dcs-info" => "dataComposition.describe",
        "dcs-validate" => "dataComposition.validate",
        "mxl-decompile" => "spreadsheet.decompile",
        "mxl-info" => "spreadsheet.describe",
        "mxl-validate" => "spreadsheet.validate",
        "role-info" => "role.describe",
        "role-validate" => "role.validate",
        _ => "",
    }
}

#[cfg(test)]
mod semantic_role_tests {
    use super::writer_source_key;
    use unica_format_core::commands::WriterSourceRole;

    #[test]
    fn data_composition_edit_binds_definition_without_exposing_its_path_to_core() {
        assert_eq!(
            writer_source_key("dataComposition.edit", WriterSourceRole::Definition),
            Some("definitionFile")
        );
    }
}
