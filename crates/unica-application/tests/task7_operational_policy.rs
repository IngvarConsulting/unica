use std::sync::{Arc, Mutex};

use unica_application::{
    AuthorabilityPolicyCommand, CompatibilityPolicyCommand, GuardEnforcement,
    OperationalPolicyDecision, OperationalPolicyService,
};
use unica_format_core::{
    navigation::Authorability,
    ports::{
        AuthorabilityPort, AuthorabilityRequest, AuthorabilityResult, AuthorabilityViolation,
        CompatibilityIssue, CompatibilityIssueKind, CompatibilityPort, CompatibilityRequest,
        CompatibilityResult, FormatDiagnostic, FormatDiagnosticCode, OperationalSourceSession,
        OperationCancellation, PublicationCancellation, PublicationCleanup, PublicationPort,
        PublicationRecovery, PublicationRequest, PublicationResult, PublicationRollback,
        PublicationStatus, SupportState, SupportSummary,
    },
    source::{SourceAdapterError, SourceAdapterErrorKind},
};

#[derive(Debug)]
struct AlternateRevision(u64);

fn alternate_session() -> OperationalSourceSession {
    OperationalSourceSession::new(AlternateRevision(41))
}

struct FakeCompatibility {
    result: CompatibilityResult,
    seen_generation: Arc<Mutex<Option<u64>>>,
}

impl CompatibilityPort for FakeCompatibility {
    fn inspect(
        &self,
        request: &CompatibilityRequest,
    ) -> Result<CompatibilityResult, SourceAdapterError> {
        *self.seen_generation.lock().unwrap() = request.sessions()[0]
            .adapter_state::<AlternateRevision>()
            .map(|revision| revision.0);
        Ok(self.result.clone())
    }
}

struct FakeAuthorability {
    result: AuthorabilityResult,
}

impl AuthorabilityPort for FakeAuthorability {
    fn inspect(
        &self,
        _request: &AuthorabilityRequest,
    ) -> Result<AuthorabilityResult, SourceAdapterError> {
        Ok(self.result.clone())
    }
}

fn compatibility_request() -> CompatibilityRequest {
    CompatibilityRequest::new(vec![alternate_session()]).unwrap()
}

fn compatibility_issue(kind: CompatibilityIssueKind) -> CompatibilityIssue {
    CompatibilityIssue::new(
        kind,
        FormatDiagnostic::new(
            match kind {
                CompatibilityIssueKind::Older => FormatDiagnosticCode::SourceRevisionOlder,
                CompatibilityIssueKind::Newer => FormatDiagnosticCode::SourceRevisionNewer,
                CompatibilityIssueKind::Malformed => FormatDiagnosticCode::SourceMalformed,
            },
            "alternate adapter rejected its source revision",
        ),
    )
}

#[test]
fn task7_alternate_fake_adapter_proves_compatibility_policy_is_format_agnostic() {
    let seen_generation = Arc::new(Mutex::new(None));
    let port = FakeCompatibility {
        result: CompatibilityResult::incompatible(compatibility_issue(
            CompatibilityIssueKind::Older,
        )),
        seen_generation: seen_generation.clone(),
    };

    let read = OperationalPolicyService::check_compatibility(
        &port,
        CompatibilityPolicyCommand::new(compatibility_request(), false),
    )
    .unwrap();
    let write = OperationalPolicyService::check_compatibility(
        &port,
        CompatibilityPolicyCommand::new(compatibility_request(), true),
    )
    .unwrap();

    assert!(matches!(read, OperationalPolicyDecision::Warn(_)));
    assert!(matches!(write, OperationalPolicyDecision::Block(_)));
    assert_eq!(*seen_generation.lock().unwrap(), Some(41));
}

#[test]
fn task7_application_policy_treats_newer_and_malformed_without_version_concepts() {
    for kind in [
        CompatibilityIssueKind::Newer,
        CompatibilityIssueKind::Malformed,
    ] {
        let port = FakeCompatibility {
            result: CompatibilityResult::incompatible(compatibility_issue(kind)),
            seen_generation: Arc::new(Mutex::new(None)),
        };
        assert!(matches!(
            OperationalPolicyService::check_compatibility(
                &port,
                CompatibilityPolicyCommand::new(compatibility_request(), true),
            )
            .unwrap(),
            OperationalPolicyDecision::Block(_)
        ));
    }
}

#[test]
fn task7_authorability_enforcement_is_application_policy_not_adapter_policy() {
    let port = FakeAuthorability {
        result: AuthorabilityResult::new(
            Authorability::SupportLocked,
            SupportSummary::new(SupportState::Locked, Some(true), 1, [1, 0, 0]),
            Some(AuthorabilityViolation::new(FormatDiagnostic::new(
                FormatDiagnosticCode::SupportLocked,
                "alternate adapter says this source is read-only",
            ))),
        ),
    };
    let request = AuthorabilityRequest::new(
        alternate_session(),
        unica_format_core::ports::AuthorabilityRequirement::Editable,
    );

    for (enforcement, expected) in [
        (GuardEnforcement::Off, "allow"),
        (GuardEnforcement::Warn, "warn"),
        (GuardEnforcement::Deny, "block"),
    ] {
        let decision = OperationalPolicyService::check_authorability(
            &port,
            AuthorabilityPolicyCommand::new(request.clone(), enforcement),
        )
        .unwrap();
        assert_eq!(decision.label(), expected);
    }
}

#[test]
fn task7_port_contract_error_does_not_become_an_allow_decision() {
    struct Broken;
    impl AuthorabilityPort for Broken {
        fn inspect(
            &self,
            _request: &AuthorabilityRequest,
        ) -> Result<AuthorabilityResult, SourceAdapterError> {
            Err(SourceAdapterError::new(
                SourceAdapterErrorKind::SourceUnavailable,
                "alternate inspection failed",
            ))
        }
    }

    assert!(OperationalPolicyService::check_authorability(
        &Broken,
        AuthorabilityPolicyCommand::new(
            AuthorabilityRequest::new(
                alternate_session(),
                unica_format_core::ports::AuthorabilityRequirement::Editable,
            ),
            GuardEnforcement::Deny,
        ),
    )
    .is_err());
}

#[test]
fn task7_alternate_publication_port_preserves_typed_lifecycle_without_format_concepts() {
    #[derive(Clone)]
    struct AlternatePublication {
        result: PublicationResult,
    }

    impl PublicationPort for AlternatePublication {
        fn publish(
            &self,
            _request: &PublicationRequest,
        ) -> Result<PublicationResult, SourceAdapterError> {
            Ok(self.result.clone())
        }
    }

    let expected = PublicationResult::new(
        PublicationStatus::Failed,
        PublicationCancellation::DuringPublication,
        PublicationRollback::Failed,
        PublicationCleanup::RetainedForRecovery,
        PublicationRecovery::Required,
        "alternate source publication requires recovery",
        vec![FormatDiagnostic::new(
            FormatDiagnosticCode::PublicationRecoveryRequired,
            "alternate adapter retained recovery state",
        )],
        Vec::new(),
        Vec::new(),
    )
    .unwrap();
    let actual = OperationalPolicyService::publish(
        &AlternatePublication {
            result: expected.clone(),
        },
        &PublicationRequest::new(
            alternate_session(),
            unica_format_core::ports::PublicationInvocation::BuildDump,
            OperationCancellation::new(),
        ),
    )
    .unwrap();

    assert_eq!(actual.status(), expected.status());
    assert_eq!(actual.cancellation(), expected.cancellation());
    assert_eq!(actual.rollback(), expected.rollback());
    assert_eq!(actual.cleanup(), expected.cleanup());
    assert_eq!(actual.recovery(), expected.recovery());
}
