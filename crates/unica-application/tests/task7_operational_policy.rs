use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

use unica_application::{
    AuthorabilityPolicyCommand, CompatibilityPolicyCommand, GuardEnforcement,
    OperationalPolicyDecision, OperationalPolicyService,
};
use unica_format_core::{
    navigation::Authorability,
    ports::{
        AuthorabilityPort, AuthorabilityRequest, AuthorabilityResult, AuthorabilityViolation,
        CompatibilityIssue, CompatibilityIssueKind, CompatibilityPort, CompatibilityRequest,
        CompatibilityResult, CompatibilityTarget, FormatDiagnostic, OwnerResolutionMode,
    },
    source::{FormatVersion, SourceAdapterError, SourceContext, SourceFamily, SourceLocation},
};

fn alternate_source() -> SourceContext {
    SourceContext::new(
        SourceLocation::new(
            PathBuf::from("/workspace"),
            PathBuf::from("/workspace/alternate"),
            PathBuf::from("/workspace/alternate/Object.source"),
        ),
        Some("alternate".to_string()),
        SourceFamily::Edt,
        None,
    )
}

struct FakeCompatibility {
    result: CompatibilityResult,
    seen_family: Arc<Mutex<Option<SourceFamily>>>,
}

impl CompatibilityPort for FakeCompatibility {
    fn inspect(
        &self,
        request: &CompatibilityRequest,
    ) -> Result<CompatibilityResult, SourceAdapterError> {
        *self.seen_family.lock().unwrap() =
            Some(request.targets[0].source.declared_family().clone());
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
    CompatibilityRequest {
        targets: vec![CompatibilityTarget {
            source: alternate_source(),
            mode: OwnerResolutionMode::Existing,
        }],
    }
}

fn compatibility_issue(kind: CompatibilityIssueKind) -> CompatibilityIssue {
    CompatibilityIssue {
        kind,
        diagnostic: FormatDiagnostic::new(
            "alternateCompatibility",
            "alternate adapter rejected its source revision",
        ),
        actual_format: Some(FormatVersion::parse("17.4").unwrap()),
        target_format: Some(FormatVersion::parse("18.0").unwrap()),
        producer_version: None,
        source_kind: None,
    }
}

#[test]
fn task7_alternate_fake_adapter_proves_compatibility_policy_is_format_agnostic() {
    let seen_family = Arc::new(Mutex::new(None));
    let port = FakeCompatibility {
        result: CompatibilityResult {
            issue: Some(compatibility_issue(CompatibilityIssueKind::Older)),
        },
        seen_family: seen_family.clone(),
    };

    let read = OperationalPolicyService::check_compatibility(
        &port,
        CompatibilityPolicyCommand {
            request: compatibility_request(),
            mutating: false,
        },
    )
    .unwrap();
    let write = OperationalPolicyService::check_compatibility(
        &port,
        CompatibilityPolicyCommand {
            request: compatibility_request(),
            mutating: true,
        },
    )
    .unwrap();

    assert!(matches!(read, OperationalPolicyDecision::Warn(_)));
    assert!(matches!(write, OperationalPolicyDecision::Block(_)));
    assert_eq!(*seen_family.lock().unwrap(), Some(SourceFamily::Edt));
}

#[test]
fn task7_application_policy_treats_newer_and_malformed_adapter_results_without_versions() {
    for kind in [
        CompatibilityIssueKind::Newer,
        CompatibilityIssueKind::Malformed,
    ] {
        let port = FakeCompatibility {
            result: CompatibilityResult {
                issue: Some(compatibility_issue(kind)),
            },
            seen_family: Arc::new(Mutex::new(None)),
        };

        assert!(matches!(
            OperationalPolicyService::check_compatibility(
                &port,
                CompatibilityPolicyCommand {
                    request: compatibility_request(),
                    mutating: true,
                },
            )
            .unwrap(),
            OperationalPolicyDecision::Block(_)
        ));
    }
}

#[test]
fn task7_authorability_enforcement_is_application_policy_not_adapter_policy() {
    let port = FakeAuthorability {
        result: AuthorabilityResult {
            authorability: Authorability::SupportLocked,
            violation: Some(AuthorabilityViolation {
                diagnostic: FormatDiagnostic::new(
                    "locked",
                    "alternate adapter says this target is read-only",
                ),
                target: PathBuf::from("/workspace/alternate/Object.source"),
                source_root: PathBuf::from("/workspace/alternate"),
            }),
        },
    };
    let request = AuthorabilityRequest {
        source: alternate_source(),
        requirement: unica_format_core::ports::AuthorabilityRequirement::Editable,
    };

    for (enforcement, expected) in [
        (GuardEnforcement::Off, "allow"),
        (GuardEnforcement::Warn, "warn"),
        (GuardEnforcement::Deny, "block"),
    ] {
        let decision = OperationalPolicyService::check_authorability(
            &port,
            AuthorabilityPolicyCommand {
                request: request.clone(),
                enforcement,
            },
        )
        .unwrap();
        assert_eq!(decision.label(), expected);
    }
}
