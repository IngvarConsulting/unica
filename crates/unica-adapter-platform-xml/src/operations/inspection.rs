use serde_json::{Map, Value};
use std::fs;
use unica_format_core::{
    commands::{InspectionPort, InspectionRequest, InspectionResult},
    source::{SourceAdapterError, SourceAdapterErrorKind},
};

use super::{NativeWriterResult, WorkspaceContext};
use crate::versions::v2_20::writers::{common, inspection_registry};
#[cfg(test)]
use crate::versions::v2_20::writers::{external, form, legacy_registry, meta};

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
pub(crate) struct PlatformXmlInspector;

impl InspectionPort for PlatformXmlInspector {
    fn inspect(&self, request: &InspectionRequest) -> Result<InspectionResult, SourceAdapterError> {
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
            return Ok(InspectionResult::cancelled());
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
        Ok(InspectionResult::from_parts(
            outcome.ok,
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
    #[cfg(test)]
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
        if let Some(preview) = legacy_registry::invoke_preview(operation, args, context) {
            return match preview {
                legacy_registry::PreviewInvocation::Unavailable(error) => Ok(NativeWriterResult {
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
                legacy_registry::PreviewInvocation::Planned(result) => result,
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
        if let Some(outcome) = inspection_registry::invoke_read(operation, tool_name, args, context)
        {
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
            return legacy_registry::invoke_mutation(operation, tool_name, args, context)
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
