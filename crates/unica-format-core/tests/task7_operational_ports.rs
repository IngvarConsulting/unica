use unica_format_core::{
    navigation::Authorability,
    ports::{
        AuthorabilityPort, CompatibilityIssueKind, CompatibilityPort, FormatDiagnostic,
        FormatDiagnosticCode, FormatDiagnosticDetail, OperationCancellation,
        OperationalEvidenceRevision, OperationalSourceSession, PublicationCancellation,
        PublicationCleanup, PublicationFailureKind, PublicationLifecycle, PublicationPort,
        PublicationRecovery, PublicationResult, PublicationRollback, SupportState,
        ValidationContext, ValidationContextPort, ValidationOwnerKind,
    },
};

fn assert_port<T: ?Sized + Send + Sync>() {}

#[test]
fn task7_operational_boundaries_are_format_neutral_ports() {
    assert_port::<dyn CompatibilityPort>();
    assert_port::<dyn AuthorabilityPort>();
    assert_port::<dyn ValidationContextPort>();
    assert_port::<dyn PublicationPort>();
}

#[test]
fn task7_cancellation_is_shared_without_a_host_domain_type() {
    let first = OperationCancellation::new();
    let second = first.clone();

    assert!(!second.is_cancelled());
    first.cancel();
    assert!(second.is_cancelled());
}

#[derive(Debug)]
struct AlternateSourceRevision {
    generation: u64,
}

#[test]
fn task7_operational_source_session_is_opaque_and_format_agnostic() {
    let session = OperationalSourceSession::new(AlternateSourceRevision { generation: 7 });

    assert_eq!(
        session
            .adapter_state::<AlternateSourceRevision>()
            .unwrap()
            .generation,
        7
    );
    assert_eq!(format!("{session:?}"), "OperationalSourceSession(<opaque>)");
}

#[test]
fn task7_diagnostics_have_closed_codes_and_allowlisted_details() {
    let diagnostic = FormatDiagnostic::new(
        FormatDiagnosticCode::SourceRevisionOlder,
        FormatDiagnosticDetail::Compatibility(CompatibilityIssueKind::Older),
    )
    .unwrap();

    assert_eq!(diagnostic.code().as_str(), "sourceRevisionOlder");
    assert_eq!(
        diagnostic.detail(),
        FormatDiagnosticDetail::Compatibility(CompatibilityIssueKind::Older)
    );
}

#[test]
fn task7_validation_context_rejects_non_semantic_language_values() {
    assert!(ValidationContext::new(
        ValidationOwnerKind::Aggregate,
        vec!["/private/source".to_string()],
        true,
        None,
        None,
        None,
    )
    .is_err());
    assert!(ValidationContext::new(
        ValidationOwnerKind::Aggregate,
        vec!["ru".to_string(), "en-US".to_string()],
        true,
        Some(true),
        Some(false),
        None,
    )
    .is_ok());
}

#[test]
fn task7_publication_lifecycle_rejects_impossible_combinations() {
    assert!(PublicationResult::new(
        PublicationLifecycle::cancelled(
            PublicationCancellation::DuringExecution,
            PublicationRollback::NotNeeded,
            PublicationCleanup::Completed,
            PublicationRecovery::NotRequired,
        )
        .unwrap(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
    )
    .is_err());
    assert!(PublicationResult::new(
        PublicationLifecycle::failed(
            PublicationFailureKind::Publication,
            PublicationCancellation::NotRequested,
            PublicationRollback::Failed,
            PublicationCleanup::RetainedForRecovery,
            PublicationRecovery::Required,
        )
        .unwrap(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
    )
    .is_err());
}

#[test]
fn task7_support_state_is_closed_semantic_evidence() {
    let summary =
        unica_format_core::ports::SupportSummary::new(SupportState::Unreadable, None, 0, [0; 3])
            .unwrap();
    let result = unica_format_core::ports::AuthorabilityResult::denied(
        Authorability::UnknownSupportState,
        summary,
        FormatDiagnostic::new(
            FormatDiagnosticCode::SupportStateUnreadable,
            FormatDiagnosticDetail::Support(SupportState::Unreadable),
        )
        .unwrap(),
        OperationalEvidenceRevision::from_digest([9; 32]),
    )
    .unwrap();
    assert_eq!(
        result.denial().unwrap().summary().state(),
        SupportState::Unreadable
    );
}
