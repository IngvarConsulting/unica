use crate::domain::source_target::MetadataAddress;
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MetaDiagnosticCode {
    InvalidArguments,
    UnsupportedKind,
    CapabilityUnavailable,
    TargetNotFound,
    AlreadyExists,
    SupportLocked,
    ReferenceConflict,
    ValidationFailed,
    RedundantListPresentation,
    CommandTextRecommendedLimit,
    CommandTextUpperLimit,
    ConcurrentModification,
    ProviderUnavailable,
    RollbackFailed,
}

impl MetaDiagnosticCode {
    #[cfg(test)]
    pub(crate) const ALL: &'static [Self] = &[
        Self::InvalidArguments,
        Self::UnsupportedKind,
        Self::CapabilityUnavailable,
        Self::TargetNotFound,
        Self::AlreadyExists,
        Self::SupportLocked,
        Self::ReferenceConflict,
        Self::ValidationFailed,
        Self::RedundantListPresentation,
        Self::CommandTextRecommendedLimit,
        Self::CommandTextUpperLimit,
        Self::ConcurrentModification,
        Self::ProviderUnavailable,
        Self::RollbackFailed,
    ];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MetaDiagnosticSeverity {
    Error,
    Warning,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MetaDiagnostic {
    pub(crate) code: MetaDiagnosticCode,
    pub(crate) severity: MetaDiagnosticSeverity,
    pub(crate) message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) metadata_path: Option<MetadataAddress>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) operation_index: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) field: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) language: Option<String>,
}

impl MetaDiagnostic {
    pub(crate) fn error(code: MetaDiagnosticCode, message: impl Into<String>) -> Self {
        Self {
            code,
            severity: MetaDiagnosticSeverity::Error,
            message: message.into(),
            metadata_path: None,
            operation_index: None,
            field: None,
            language: None,
        }
    }

    pub(crate) fn warning(code: MetaDiagnosticCode, message: impl Into<String>) -> Self {
        let mut diagnostic = Self::error(code, message);
        diagnostic.severity = MetaDiagnosticSeverity::Warning;
        diagnostic
    }

    pub(crate) fn with_field(mut self, field: impl Into<String>) -> Self {
        self.field = Some(field.into());
        self
    }

    pub(crate) fn with_metadata_path(mut self, metadata_path: MetadataAddress) -> Self {
        self.metadata_path = Some(metadata_path);
        self
    }

    pub(crate) fn with_operation_index(mut self, operation_index: usize) -> Self {
        self.operation_index = Some(operation_index);
        self
    }

    pub(crate) fn with_language(mut self, language: impl Into<String>) -> Self {
        let language = language.into();
        self.language = (!language.is_empty()).then_some(language);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostic_codes_serialize_to_the_stable_exhaustive_vocabulary() {
        let cases = [
            (
                MetaDiagnosticCode::InvalidArguments,
                "\"invalid_arguments\"",
            ),
            (MetaDiagnosticCode::UnsupportedKind, "\"unsupported_kind\""),
            (
                MetaDiagnosticCode::CapabilityUnavailable,
                "\"capability_unavailable\"",
            ),
            (MetaDiagnosticCode::TargetNotFound, "\"target_not_found\""),
            (MetaDiagnosticCode::AlreadyExists, "\"already_exists\""),
            (MetaDiagnosticCode::SupportLocked, "\"support_locked\""),
            (
                MetaDiagnosticCode::ReferenceConflict,
                "\"reference_conflict\"",
            ),
            (
                MetaDiagnosticCode::ValidationFailed,
                "\"validation_failed\"",
            ),
            (
                MetaDiagnosticCode::RedundantListPresentation,
                "\"redundant_list_presentation\"",
            ),
            (
                MetaDiagnosticCode::CommandTextRecommendedLimit,
                "\"command_text_recommended_limit\"",
            ),
            (
                MetaDiagnosticCode::CommandTextUpperLimit,
                "\"command_text_upper_limit\"",
            ),
            (
                MetaDiagnosticCode::ConcurrentModification,
                "\"concurrent_modification\"",
            ),
            (
                MetaDiagnosticCode::ProviderUnavailable,
                "\"provider_unavailable\"",
            ),
            (MetaDiagnosticCode::RollbackFailed, "\"rollback_failed\""),
        ];

        assert_eq!(MetaDiagnosticCode::ALL.len(), cases.len());
        for (code, expected) in cases {
            assert_eq!(serde_json::to_string(&code).unwrap(), expected);
        }
    }

    #[test]
    fn diagnostic_language_is_present_only_for_a_language_specific_finding() {
        let generic = serde_json::to_value(MetaDiagnostic::error(
            MetaDiagnosticCode::ValidationFailed,
            "generic validation failure",
        ))
        .unwrap();
        assert!(generic.get("language").is_none());

        let localized = serde_json::to_value(
            MetaDiagnostic::warning(
                MetaDiagnosticCode::CommandTextRecommendedLimit,
                "localized validation warning",
            )
            .with_language("ru"),
        )
        .unwrap();
        assert_eq!(localized["language"], "ru");

        let empty = serde_json::to_value(
            MetaDiagnostic::warning(
                MetaDiagnosticCode::CommandTextUpperLimit,
                "language-independent validation warning",
            )
            .with_language(""),
        )
        .unwrap();
        assert!(empty.get("language").is_none());
    }
}
