use unica_application::{
    AuthorabilityPolicyCommand, GuardEnforcement, OperationalPolicyDecision,
    OperationalPolicyService,
};
use unica_format_core::{
    navigation::Authorability,
    ports::{
        AuthorabilityPort, AuthorabilityRequest, AuthorabilityRequirement, AuthorabilityResult,
        FormatDiagnostic, FormatDiagnosticCode, FormatDiagnosticDetail,
        OperationalEvidenceRevision, OperationalSourceSession, SupportState, SupportSummary,
    },
    source::SourceAdapterError,
};

#[derive(Debug)]
struct AlternateCapability;

struct AlternateDeniedPort;

impl AuthorabilityPort for AlternateDeniedPort {
    fn inspect(
        &self,
        _request: &AuthorabilityRequest,
    ) -> Result<AuthorabilityResult, SourceAdapterError> {
        AuthorabilityResult::denied(
            Authorability::UnknownSupportState,
            SupportSummary::new(SupportState::Unreadable, None, 0, [0; 3]).unwrap(),
            FormatDiagnostic::new(
                FormatDiagnosticCode::SupportStateUnreadable,
                FormatDiagnosticDetail::Support(SupportState::Unreadable),
            )
            .unwrap(),
            OperationalEvidenceRevision::from_digest([3; 32]),
        )
        .map_err(|error| {
            SourceAdapterError::new(
                unica_format_core::source::SourceAdapterErrorKind::SourceUnavailable,
                error.to_string(),
            )
        })
    }
}

#[test]
fn alternate_adapter_denial_cannot_be_interpreted_as_missing_evidence() {
    let request = AuthorabilityRequest::new(
        OperationalSourceSession::new(AlternateCapability),
        AuthorabilityRequirement::Editable,
    );

    let decision = OperationalPolicyService::check_authorability(
        &AlternateDeniedPort,
        AuthorabilityPolicyCommand::new(request, GuardEnforcement::Deny),
    )
    .unwrap();

    assert!(matches!(decision, OperationalPolicyDecision::Block(_)));
}

#[test]
fn application_policy_source_has_no_optional_authorability_violation_path() {
    let source = include_str!("../src/commands.rs");
    assert!(!source.contains("Option<FormatDiagnostic>"));
    assert!(!source.contains("into_violation"));
    assert!(source.contains("AuthorabilityResult::Allowed"));
    assert!(source.contains("AuthorabilityResult::Denied"));
}
