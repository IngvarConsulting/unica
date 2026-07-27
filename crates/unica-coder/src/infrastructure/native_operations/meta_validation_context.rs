use std::path::Path;

use unica_adapter_platform_xml::PlatformXmlAdapterFactory;
use unica_application::OperationalPolicyService;
use unica_format_core::ports::{
    FormatDiagnostic, FormatDiagnosticDetail, ValidationContextRequest, ValidationIssueKind,
    ValidationMethodReferenceStatus, ValidationOwnerKind,
};

use crate::{
    domain::workspace::WorkspaceContext,
    infrastructure::platform_xml_owner::validation_source_session,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MetaValidationOwnerKind {
    Configuration,
    Extension,
    External,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MetaValidationOwnerContext {
    pub owner_kind: MetaValidationOwnerKind,
    pub language_codes: Vec<String>,
    pub command_text_validation_required: bool,
    pub references_present: Option<bool>,
    pub registrar_present: Option<bool>,
    pub method_reference_status: Option<ValidationMethodReferenceStatus>,
}

#[derive(Debug, Clone)]
pub(crate) struct MetaValidationReadInspection {
    pub context: Result<MetaValidationOwnerContext, String>,
}

pub(crate) fn inspect_meta_validation_reads(
    object_path: &Path,
    workspace: &WorkspaceContext,
) -> MetaValidationReadInspection {
    let session = match validation_source_session(object_path, workspace) {
        Ok(session) => session,
        Err(_) => {
            return MetaValidationReadInspection {
                context: Err(
                    "validation source could not be authorized and captured".to_string(),
                ),
            }
        }
    };
    let registration = PlatformXmlAdapterFactory::new().operational_registration();
    match OperationalPolicyService::validation_context(
        registration.validation_context(),
        &ValidationContextRequest::new(session),
    ) {
        Ok(result) => {
            let diagnostics = result
                .diagnostics()
                .first()
                .map(|diagnostic| public_validation_diagnostic_message(diagnostic).to_string());
            let context = result
                .into_context()
                .map(|context| MetaValidationOwnerContext {
                    owner_kind: match context.owner_kind() {
                        ValidationOwnerKind::Aggregate => MetaValidationOwnerKind::Configuration,
                        ValidationOwnerKind::Extension => MetaValidationOwnerKind::Extension,
                        ValidationOwnerKind::Standalone => MetaValidationOwnerKind::External,
                    },
                    language_codes: context.language_codes().to_vec(),
                    command_text_validation_required: context
                        .command_text_validation_required(),
                    references_present: context.references_present(),
                    registrar_present: context.registrar_present(),
                    method_reference_status: context.method_reference_status(),
                })
                .ok_or_else(|| {
                    diagnostics.unwrap_or_else(|| {
                        "validation context is unavailable".to_string()
                    })
                });
            MetaValidationReadInspection { context }
        }
        Err(_) => MetaValidationReadInspection {
            context: Err("validation context inspection failed".to_string()),
        },
    }
}

fn public_validation_diagnostic_message(diagnostic: &FormatDiagnostic) -> &'static str {
    match diagnostic.details().iter().find_map(|detail| match detail {
        FormatDiagnosticDetail::Validation(issue) => Some(*issue),
        _ => None,
    }) {
        Some(ValidationIssueKind::SourceUnreadable) => {
            "Validation source could not be authorized and captured."
        }
        Some(ValidationIssueKind::OwnerUnavailable) => {
            "The validation source has no authoritative aggregate owner."
        }
        Some(ValidationIssueKind::RegistrationMissing) => {
            "The metadata object is not registered by its aggregate owner."
        }
        Some(ValidationIssueKind::LanguageProfileMissing) => {
            "The aggregate has no complete registered language profile."
        }
        Some(ValidationIssueKind::ReferenceMissing) => {
            "A referenced metadata object is missing."
        }
        Some(ValidationIssueKind::RegistrarMissing) => {
            "A required registrar relationship is missing."
        }
        None => "Validation context is unavailable.",
    }
}

#[cfg(test)]
mod tests {
    use super::public_validation_diagnostic_message;
    use unica_format_core::ports::{
        FormatDiagnostic, FormatDiagnosticCode, FormatDiagnosticDetail, ValidationIssueKind,
    };

    #[test]
    fn public_validation_message_ignores_adapter_free_form_text() {
        let diagnostic = FormatDiagnostic::new(
            FormatDiagnosticCode::ValidationContextUnavailable,
            r"/private/Configuration.xml C:\private MetaDataObject",
        )
        .with_detail(FormatDiagnosticDetail::Validation(
            ValidationIssueKind::SourceUnreadable,
        ));

        let public = public_validation_diagnostic_message(&diagnostic);

        assert_eq!(
            public,
            "Validation source could not be authorized and captured."
        );
        assert!(!public.contains("private"));
        assert!(!public.contains("Configuration.xml"));
        assert!(!public.contains("MetaDataObject"));
    }
}
