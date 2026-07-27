use crate::application::operation_descriptors::native_operation_descriptor;
use crate::application::ports::FormatGuardCheck;
use crate::application::{AdapterOutcome, ToolHandler, ToolSpec};
use crate::domain::workspace::WorkspaceContext;
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};
use unica_application::{
    CompatibilityPolicyCommand, OperationalPolicyDecision, OperationalPolicyService,
};
use unica_format_core::ports::{
    CompatibilityIssueKind, CompatibilityRequest, FormatDiagnostic, FormatDiagnosticDetail,
    OwnerResolutionMode,
};

pub(crate) fn evaluate_format_guard(
    spec: ToolSpec,
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<FormatGuardCheck, String> {
    let ToolHandler::NativeOperation { operation, .. } = spec.handler else {
        return Ok(FormatGuardCheck::Allow);
    };
    let Some(descriptor) = native_operation_descriptor(operation) else {
        return Ok(FormatGuardCheck::Allow);
    };
    let targets = command_private_targets(descriptor, args, context);
    if targets.is_empty() {
        return Ok(FormatGuardCheck::Allow);
    }

    let factory = unica_adapter_platform_xml::PlatformXmlAdapterFactory::new();
    let sessions = targets
        .iter()
        .map(|target| {
            let mode = if target.exists() {
                OwnerResolutionMode::Existing
            } else {
                OwnerResolutionMode::ExistingForNewOutput
            };
            if operation == "meta-validate" {
                factory.capture_unscoped_validation_source(
                    target,
                    &context.workspace_root,
                    mode,
                )
            } else {
                factory.capture_unscoped_source(target, &context.workspace_root, mode)
            }
        })
        .collect::<Vec<_>>();
    let request = CompatibilityRequest::new(sessions).map_err(|error| error.to_string())?;
    let registration = factory.operational_registration();
    let decision = OperationalPolicyService::check_compatibility(
        registration.compatibility(),
        CompatibilityPolicyCommand::new(request, spec.mutating),
    )
    .map_err(|_| "source compatibility inspection failed".to_string())?;

    Ok(match decision {
        OperationalPolicyDecision::Allow => FormatGuardCheck::Allow,
        OperationalPolicyDecision::Warn(diagnostic) => FormatGuardCheck::Warn {
            warning: format!(
                "{} Read-only policy applies.",
                public_compatibility_message(&diagnostic)
            ),
            diagnostic: diagnostic_json(&diagnostic),
        },
        OperationalPolicyDecision::Block(diagnostic) => {
            let warning = format!(
                "{} The requested change was not started.",
                public_compatibility_message(&diagnostic)
            );
            FormatGuardCheck::Block {
                outcome: AdapterOutcome {
                    ok: false,
                    summary: format!("{} blocked by source compatibility policy", spec.name),
                    changes: Vec::new(),
                    warnings: vec![warning.clone()],
                    errors: vec![warning.clone()],
                    artifacts: Vec::new(),
                    stdout: None,
                    stderr: Some(format!("{warning}\n")),
                    command: None,
                },
                diagnostic: diagnostic_json(&diagnostic),
            }
        }
    })
}

fn command_private_targets(
    descriptor: &crate::application::operation_descriptors::OperationDescriptor,
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Vec<PathBuf> {
    let mut targets = descriptor
        .source_path_args
        .iter()
        .chain(descriptor.write_path_args.iter())
        .filter_map(|name| args.get(*name).and_then(Value::as_str))
        .flat_map(|raw| raw.split('|'))
        .map(str::trim)
        .filter(|raw| !raw.is_empty())
        .map(|raw| resolve_command_path(raw, &context.cwd))
        .collect::<Vec<_>>();
    targets.sort();
    targets.dedup();
    targets
}

fn resolve_command_path(raw: &str, cwd: &Path) -> PathBuf {
    let path = PathBuf::from(raw);
    if path.is_absolute() {
        path
    } else {
        cwd.join(path)
    }
}

fn public_compatibility_message(diagnostic: &FormatDiagnostic) -> &'static str {
    match diagnostic.code() {
        unica_format_core::ports::FormatDiagnosticCode::SourceRevisionOlder => {
            "The source revision requires explicit migration before editing."
        }
        unica_format_core::ports::FormatDiagnosticCode::SourceRevisionNewer => {
            "The source revision is newer than this adapter supports."
        }
        unica_format_core::ports::FormatDiagnosticCode::SourceFamilyIncompatible => {
            "The selected source belongs to another source family."
        }
        _ => "The selected source could not be validated for this operation.",
    }
}

fn diagnostic_json(diagnostic: &FormatDiagnostic) -> Value {
    let compatibility = match diagnostic.detail() {
        FormatDiagnosticDetail::Compatibility(kind) => Some(match kind {
            CompatibilityIssueKind::Older => "older",
            CompatibilityIssueKind::Newer => "newer",
            CompatibilityIssueKind::Malformed => "malformed",
        }),
        _ => None,
    };
    json!({
        "code": diagnostic.code().as_str(),
        "compatibility": compatibility,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use unica_format_core::ports::{FormatDiagnosticCode, FormatDiagnosticDetail};

    #[test]
    fn public_diagnostic_uses_only_closed_semantic_evidence() {
        let diagnostic = FormatDiagnostic::new(
            FormatDiagnosticCode::SourceRevisionNewer,
            FormatDiagnosticDetail::Compatibility(CompatibilityIssueKind::Newer),
        )
        .unwrap();

        let public = json!({
            "message": public_compatibility_message(&diagnostic),
            "diagnostic": diagnostic_json(&diagnostic),
        })
        .to_string();

        assert!(public.contains("sourceRevisionNewer"));
        for forbidden in [
            "/private/",
            r"C:\private",
            "Configuration.xml",
            "MetaDataObject",
            "MDClasses",
        ] {
            assert!(!public.contains(forbidden), "{public}");
        }
    }
}
