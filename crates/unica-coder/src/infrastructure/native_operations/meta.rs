//! Host metadata presentation gateway.
//!
//! Validation and XML interpretation are adapter responsibilities. This module
//! only maps the closed validation DTOs to the established MCP presentation.

use super::common::{absolutize, bool_arg, int_arg, required_path};
use crate::{application::AdapterOutcome, domain::workspace::WorkspaceContext};
use serde::Serialize;
use serde_json::{Map, Value};
use std::path::PathBuf;
use unica_format_core::ports::{
    OperationalValidationRequest, ValidationCoverage, ValidationFinding, ValidationFindingCode,
    ValidationFindingSeverity, ValidationOptions, ValidationReport, ValidationStatus,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
enum PublicStatus {
    Valid,
    Partial,
    Invalid,
    Unavailable,
}

impl PublicStatus {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Valid => "valid",
            Self::Partial => "partial",
            Self::Invalid => "invalid",
            Self::Unavailable => "unavailable",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
enum PublicCoverage {
    Complete,
    Partial,
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Aggregate {
    Valid,
    Partial,
    InvalidComplete,
    InvalidPartial,
    Unavailable,
}

impl Aggregate {
    fn from_report(report: &ValidationReport) -> Self {
        if report
            .findings()
            .iter()
            .any(|finding| finding.code() == ValidationFindingCode::SourceUnreadable)
        {
            return Self::Unavailable;
        }
        match (report.status(), report.coverage()) {
            (ValidationStatus::Valid, ValidationCoverage::Complete) => Self::Valid,
            (ValidationStatus::Partial, ValidationCoverage::Partial) => Self::Partial,
            (ValidationStatus::Invalid, ValidationCoverage::Complete) => Self::InvalidComplete,
            (ValidationStatus::Invalid, ValidationCoverage::Partial) => Self::InvalidPartial,
            _ => Self::Unavailable,
        }
    }

    const fn join(self, other: Self) -> Self {
        if matches!(self, Self::Unavailable) || matches!(other, Self::Unavailable) {
            return Self::Unavailable;
        }
        let invalid = matches!(self, Self::InvalidComplete | Self::InvalidPartial)
            || matches!(other, Self::InvalidComplete | Self::InvalidPartial);
        let partial = matches!(self, Self::Partial | Self::InvalidPartial)
            || matches!(other, Self::Partial | Self::InvalidPartial);
        match (invalid, partial) {
            (true, true) => Self::InvalidPartial,
            (true, false) => Self::InvalidComplete,
            (false, true) => Self::Partial,
            (false, false) => Self::Valid,
        }
    }

    const fn status(self) -> PublicStatus {
        match self {
            Self::Valid => PublicStatus::Valid,
            Self::Partial => PublicStatus::Partial,
            Self::InvalidComplete | Self::InvalidPartial => PublicStatus::Invalid,
            Self::Unavailable => PublicStatus::Unavailable,
        }
    }

    const fn coverage(self) -> PublicCoverage {
        match self {
            Self::Valid | Self::InvalidComplete => PublicCoverage::Complete,
            Self::Partial | Self::InvalidPartial => PublicCoverage::Partial,
            Self::Unavailable => PublicCoverage::Unavailable,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
struct PublicDiagnostic {
    #[serde(skip_serializing_if = "Option::is_none")]
    subject: Option<unica_format_core::ports::SemanticArtifactId>,
    #[serde(flatten)]
    finding: ValidationFinding,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PublicValidation {
    status: PublicStatus,
    coverage: PublicCoverage,
    reports: Vec<ValidationReport>,
    diagnostics: Vec<PublicDiagnostic>,
}

pub(crate) struct MetaValidationInvocation {
    pub(crate) adapter: AdapterOutcome,
    pub(crate) data: Value,
}

pub(crate) fn validate_meta_with_data(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> MetaValidationInvocation {
    match run_validation(args, context) {
        Ok((aggregate, reports, diagnostics, stdout, artifacts, warnings, errors)) => {
            let ok = matches!(aggregate, Aggregate::Valid | Aggregate::Partial);
            let summary = match aggregate.status() {
                PublicStatus::Valid => "unica.meta.validate completed",
                PublicStatus::Partial => {
                    "unica.meta.validate completed with partial semantic coverage"
                }
                PublicStatus::Invalid => "unica.meta.validate found semantic violations",
                PublicStatus::Unavailable => {
                    "unica.meta.validate could not inspect the requested source"
                }
            };
            MetaValidationInvocation {
                adapter: AdapterOutcome {
                    ok,
                    summary: summary.to_string(),
                    changes: Vec::new(),
                    warnings,
                    errors,
                    artifacts,
                    stdout: Some(stdout),
                    stderr: None,
                    command: None,
                },
                data: public_data(aggregate, reports, diagnostics),
            }
        }
        Err(_) => unavailable_validation(),
    }
}

type ValidationPresentation = (
    Aggregate,
    Vec<ValidationReport>,
    Vec<PublicDiagnostic>,
    String,
    Vec<String>,
    Vec<String>,
    Vec<String>,
);

fn run_validation(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<ValidationPresentation, String> {
    let raw = required_path(
        args,
        &["objectPath", "ObjectPath", "path", "Path"],
        "ObjectPath",
    )?;
    let paths = raw
        .to_string_lossy()
        .split('|')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .collect::<Vec<_>>();
    if paths.is_empty() {
        return Err("validation request contains no subjects".to_string());
    }
    let factory = unica_adapter_platform_xml::PlatformXmlAdapterFactory::new();
    let sessions = paths
        .into_iter()
        .map(|path| {
            factory.capture_unscoped_validation_source(
                &absolutize(path, &context.cwd),
                &context.workspace_root,
                unica_format_core::ports::OwnerResolutionMode::Existing,
            )
        })
        .collect::<Vec<_>>();
    let detailed = bool_arg(args, &["detailed", "Detailed"]);
    let max_errors = int_arg(args, &["maxErrors", "MaxErrors"])
        .and_then(|value| u16::try_from(value).ok())
        .unwrap_or(30)
        .min(1_000);
    let request = OperationalValidationRequest::new(
        sessions,
        ValidationOptions::new(detailed, max_errors).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    let registration = factory.operational_registration();
    let result =
        unica_application::OperationalPolicyService::validate(registration.validation(), &request)
            .map_err(|_| "validation adapter operation failed".to_string())?;

    let mut reports = result.reports().to_vec();
    reports.sort_by_cached_key(|report| {
        serde_json::to_string(report).expect("validated report is serializable")
    });
    let aggregate = reports
        .iter()
        .map(Aggregate::from_report)
        .fold(Aggregate::Valid, Aggregate::join);
    let mut diagnostics = reports
        .iter()
        .flat_map(|report| {
            report
                .findings()
                .iter()
                .copied()
                .map(|finding| PublicDiagnostic {
                    subject: Some(report.subject().clone()),
                    finding,
                })
        })
        .collect::<Vec<_>>();
    diagnostics.sort();
    diagnostics.dedup();

    let mut stdout = Vec::new();
    let mut artifacts = Vec::new();
    let mut warnings = Vec::new();
    let mut errors = Vec::new();
    for report in &reports {
        let subject = report.subject().as_str().to_string();
        artifacts.push(subject.clone());
        stdout.push(format!("--- {subject} ---"));
        stdout.push(format!("checks: {}", report.checks()));
        for finding in report.findings() {
            let code = finding_code(finding.code());
            let severity = match finding.severity() {
                ValidationFindingSeverity::Warning => {
                    warnings.push(format!("validation:{code}"));
                    "WARN"
                }
                ValidationFindingSeverity::Error => {
                    errors.push(format!("validation:{code}"));
                    "ERROR"
                }
            };
            stdout.push(format!("[{severity}] {code}"));
        }
    }
    stdout.push(format!("validated: {}", artifacts.len()));
    stdout.push(format!("result: {}", aggregate.status().as_str()));
    Ok((
        aggregate,
        reports,
        diagnostics,
        format!("{}\n", stdout.join("\n")),
        artifacts,
        warnings,
        errors,
    ))
}

fn public_data(
    aggregate: Aggregate,
    reports: Vec<ValidationReport>,
    diagnostics: Vec<PublicDiagnostic>,
) -> Value {
    serde_json::json!({
        "validation": PublicValidation {
            status: aggregate.status(),
            coverage: aggregate.coverage(),
            reports,
            diagnostics,
        }
    })
}

fn unavailable_validation() -> MetaValidationInvocation {
    let finding = ValidationFinding::new(
        ValidationFindingSeverity::Error,
        ValidationFindingCode::SourceUnreadable,
    );
    MetaValidationInvocation {
        adapter: AdapterOutcome {
            ok: false,
            summary: "unica.meta.validate could not inspect the requested source".to_string(),
            changes: Vec::new(),
            warnings: Vec::new(),
            errors: vec!["validation:sourceUnreadable".to_string()],
            artifacts: Vec::new(),
            stdout: None,
            stderr: None,
            command: None,
        },
        data: public_data(
            Aggregate::Unavailable,
            Vec::new(),
            vec![PublicDiagnostic {
                subject: None,
                finding,
            }],
        ),
    }
}

fn finding_code(code: ValidationFindingCode) -> &'static str {
    match code {
        ValidationFindingCode::SourceUnreadable => "sourceUnreadable",
        ValidationFindingCode::SourceMalformed => "sourceMalformed",
        ValidationFindingCode::RevisionUnsupported => "revisionUnsupported",
        ValidationFindingCode::SemanticStructureInvalid => "semanticStructureInvalid",
        ValidationFindingCode::SemanticValueInvalid => "semanticValueInvalid",
        ValidationFindingCode::IdentityMissing => "identityMissing",
        ValidationFindingCode::IdentityInvalid => "identityInvalid",
        ValidationFindingCode::NameMissing => "nameMissing",
        ValidationFindingCode::RegistrationMissing => "registrationMissing",
        ValidationFindingCode::LanguageProfileMissing => "languageProfileMissing",
        ValidationFindingCode::ReferenceMissing => "referenceMissing",
        ValidationFindingCode::RegistrarMissing => "registrarMissing",
        ValidationFindingCode::RegistrarCoverageNotEvaluated => "registrarCoverageNotEvaluated",
        ValidationFindingCode::RegistrarCoveragePartial => "registrarCoveragePartial",
        ValidationFindingCode::MethodReferenceInvalid => "methodReferenceInvalid",
        ValidationFindingCode::DuplicateSemanticItem => "duplicateSemanticItem",
        ValidationFindingCode::CommandPresentationTooLong => "commandPresentationTooLong",
        ValidationFindingCode::UnsupportedCombination => "unsupportedCombination",
    }
}
