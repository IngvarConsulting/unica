use serde::Serialize;
use serde_json::{Map, Value};
use std::{fs, path::PathBuf};
use unica_format_core::{
    commands::{
        InspectionPort, InspectionRequest, MutationMode, WriterEvidence, WriterFamily, WriterResult,
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
pub(crate) struct AdapterOutcome {
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) command: Option<Vec<String>>,
}

impl AdapterOutcome {
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
            command: None,
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
    operation: String,
    tool_name: String,
    args: Map<String, Value>,
    context: WorkspaceContext,
}

impl PlatformWriterSession {
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
        if request.command().intent() != operation_intent(&session.operation) {
            return Err(SourceAdapterError::new(
                SourceAdapterErrorKind::CapabilityBlocked,
                "writer command does not match the bound operation",
            ));
        }
        if request.cancellation().is_cancelled() {
            return Ok(WriterResult::cancelled());
        }
        let (outcome, evidence) = if session.operation == "form-edit"
            && form::has_edit_payload(&session.args)
        {
            let execution = match request.mode() {
                MutationMode::Preview => form::preview_with_data(&session.args, &session.context),
                MutationMode::Apply => form::apply_with_data(&session.args, &session.context),
            };
            let (outcome, data) = execution.into_core_parts();
            (outcome, data.map(WriterEvidence::FormEdit))
        } else {
            let outcome = match request.mode() {
                MutationMode::Preview => NativeOperationAdapter::preview(
                    &session.operation,
                    &session.tool_name,
                    &session.args,
                    &session.context,
                ),
                MutationMode::Apply => registry::invoke_mutation(
                    &session.operation,
                    &session.tool_name,
                    &session.args,
                    &session.context,
                )
                .ok_or_else(|| "Platform XML writer operation is not registered".to_string()),
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
            .adapter_state::<PlatformWriterSession>()
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
    ) -> Result<AdapterOutcome, String> {
        if let Some(outcome) = external::preview(operation, tool_name, args, context) {
            return Ok(outcome);
        }
        if operation == "form-edit" && form::has_edit_payload(args) {
            return Ok(form::preview_form_edit(args, context));
        }
        if operation == "meta-edit" {
            meta::validate_meta_edit_preview(args, context)?;
            return Ok(AdapterOutcome {
                ok: true,
                summary: format!("dry run: {tool_name} planned native metadata edit"),
                changes: vec!["no files changed because dryRun is true".to_string()],
                warnings: Vec::new(),
                errors: Vec::new(),
                artifacts: Vec::new(),
                stdout: None,
                stderr: None,
                command: None,
            });
        }
        if let Some(preview) = registry::invoke_preview(operation, args, context) {
            return match preview {
                registry::PreviewInvocation::Unavailable(error) => Ok(AdapterOutcome {
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
                    command: None,
                }),
                registry::PreviewInvocation::Planned(result) => result,
            };
        }
        Ok(AdapterOutcome {
            ok: true,
            summary: format!("dry run: {tool_name} would execute native XML/DSL operation"),
            changes: vec!["no files changed because dryRun is true".to_string()],
            warnings: Vec::new(),
            errors: Vec::new(),
            artifacts: Vec::new(),
            stdout: None,
            stderr: None,
            command: None,
        })
    }

    fn read(
        operation: &str,
        tool_name: &str,
        args: &Map<String, Value>,
        context: &WorkspaceContext,
    ) -> Result<AdapterOutcome, String> {
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
    ) -> Result<AdapterOutcome, String> {
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
    pub(crate) adapter: AdapterOutcome,
    pub(crate) data: Option<Value>,
}

fn operation_intent(operation: &str) -> &'static str {
    match operation {
        "cf-init" => "configuration.initialize",
        "cf-edit" => "configuration.edit",
        "cfe-init" => "extension.initialize",
        "cfe-borrow" => "extension.borrow",
        "cfe-patch-method" => "extension.patchMethod",
        "epf-init" => "externalArtifact.initializeProcessor",
        "erf-init" => "externalArtifact.initializeReport",
        "meta-compile" => "metadata.create",
        "meta-edit" => "metadata.edit",
        "meta-remove" => "metadata.remove",
        "form-add" => "form.create",
        "form-compile" => "form.compile",
        "form-edit" => "form.edit",
        "form-remove" => "form.remove",
        "template-add" => "template.create",
        "template-remove" => "template.remove",
        "help-add" => "help.create",
        "interface-edit" => "interface.edit",
        "role-compile" => "role.create",
        "subsystem-compile" => "subsystem.create",
        "subsystem-edit" => "subsystem.edit",
        "support-edit" => "support.edit",
        "dcs-compile" => "dataComposition.create",
        "dcs-edit" => "dataComposition.edit",
        "mxl-compile" => "spreadsheet.create",
        _ => "",
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
