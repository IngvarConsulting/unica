use serde_json::json;
use unica_format_core::{
    navigation::Authorability,
    ports::{
        AuthorabilityResult, CompatibilityIssueKind, FormatDiagnostic, FormatDiagnosticCode,
        FormatDiagnosticDetail, ObjectKindProjection, ObjectKindRegistryPort,
        ObjectKindSelector, PublicationCancellation, PublicationCleanup, PublicationFailureKind,
        PublicationArtifact, PublicationChange, PublicationIssueKind, PublicationLifecycle,
        PublicationRecovery, PublicationResult, PublicationRollback, SemanticArtifactLease,
        SemanticArtifactPort, SemanticArtifactReadRequest, SemanticArtifactReadResult,
        SemanticArtifactRole, SupportState, SupportSummary, ValidationFinding,
        ValidationFindingCode, ValidationFindingSeverity, ValidationIssueKind,
    },
    semantic_ids::SemanticObjectKind,
    source::SourceAdapterError,
};

#[test]
fn authorability_is_a_closed_allowed_or_denied_decision() {
    let writable = SupportSummary::new(SupportState::Absent, None, 0, [0; 3]).unwrap();
    let allowed = AuthorabilityResult::allowed(writable).unwrap();
    assert!(allowed.is_allowed());

    let unreadable = SupportSummary::new(SupportState::Unreadable, None, 0, [0; 3]).unwrap();
    let denial = FormatDiagnostic::new(
        FormatDiagnosticCode::SupportStateUnreadable,
        FormatDiagnosticDetail::Support(SupportState::Unreadable),
    )
    .unwrap();
    let denied = AuthorabilityResult::denied(
        Authorability::UnknownSupportState,
        unreadable,
        denial,
    )
    .unwrap();
    assert!(!denied.is_allowed());
    assert_eq!(
        denied.denial().unwrap().diagnostic().code(),
        FormatDiagnosticCode::SupportStateUnreadable
    );

    let editable = SupportSummary::new(SupportState::Editable, Some(true), 1, [1, 0, 0])
        .unwrap();
    let removal_required = AuthorabilityResult::denied(
        Authorability::Authorable,
        editable,
        FormatDiagnostic::new(
            FormatDiagnosticCode::SupportRemovalRequired,
            FormatDiagnosticDetail::Support(SupportState::Editable),
        )
        .unwrap(),
    )
    .unwrap();
    assert!(!removal_required.is_allowed());
}

#[test]
fn malicious_authorability_wire_states_are_rejected() {
    for invalid in [
        json!({
            "decision": "allowed",
            "authorability": "unknownSupportState",
            "summary": {
                "state": "unreadable",
                "editingEnabled": null,
                "vendorCount": 0,
                "ruleCounts": [0, 0, 0]
            }
        }),
        json!({
            "decision": "denied",
            "authorability": "authorable",
            "summary": {
                "state": "absent",
                "editingEnabled": null,
                "vendorCount": 0,
                "ruleCounts": [0, 0, 0]
            },
            "diagnostic": {
                "code": "supportStateUnreadable",
                "detail": {"support": "unreadable"}
            }
        }),
        json!({
            "decision": "allowed",
            "authorability": "authorable",
            "summary": {
                "state": "unreadable",
                "editingEnabled": true,
                "vendorCount": 9,
                "ruleCounts": [1, 2, 3]
            }
        }),
    ] {
        assert!(
            serde_json::from_value::<AuthorabilityResult>(invalid).is_err(),
            "invalid authorability state was accepted"
        );
    }
}

#[test]
fn diagnostic_code_and_detail_must_be_a_valid_closed_pair() {
    assert!(FormatDiagnostic::new(
        FormatDiagnosticCode::SourceRevisionOlder,
        FormatDiagnosticDetail::Compatibility(CompatibilityIssueKind::Older),
    )
    .is_ok());
    assert!(FormatDiagnostic::new(
        FormatDiagnosticCode::SourceRevisionOlder,
        FormatDiagnosticDetail::Validation(ValidationIssueKind::SourceUnreadable),
    )
    .is_err());
    assert!(serde_json::from_value::<FormatDiagnostic>(json!({
        "code": "supportLocked",
        "detail": {"compatibility": "newer"}
    }))
    .is_err());
}

#[test]
fn validation_findings_carry_only_closed_semantic_values() {
    let finding = ValidationFinding::new(
        ValidationFindingSeverity::Error,
        ValidationFindingCode::RegistrationMissing,
    );
    let value = serde_json::to_value(&finding).unwrap();
    let text = value.to_string();

    assert_eq!(value["severity"], "error");
    assert_eq!(value["code"], "registrationMissing");
    for forbidden in [
        "/private/",
        r"C:\private",
        "Configuration.xml",
        "MetaDataObject",
        "MDClasses",
    ] {
        assert!(!text.contains(forbidden), "{text}");
    }
}

#[test]
fn publication_lifecycle_is_closed_and_rejects_contradictory_wire_states() {
    let published = PublicationResult::new(
        PublicationLifecycle::published(),
        Vec::new(),
        vec![PublicationChange::FullSourceReplaced],
        vec![PublicationArtifact::PublishedSource],
    )
    .unwrap();
    assert!(published.lifecycle().is_published());

    let failed = PublicationResult::new(
        PublicationLifecycle::failed(
            PublicationFailureKind::Publication,
            PublicationCancellation::NotRequested,
            PublicationRollback::Failed,
            PublicationCleanup::RetainedForRecovery,
            PublicationRecovery::Required,
        )
        .unwrap(),
        vec![
            FormatDiagnostic::new(
                FormatDiagnosticCode::PublicationFailed,
                FormatDiagnosticDetail::Publication(PublicationIssueKind::Failed),
            )
            .unwrap(),
            FormatDiagnostic::new(
                FormatDiagnosticCode::PublicationRecoveryRequired,
                FormatDiagnosticDetail::Publication(PublicationIssueKind::RecoveryRequired),
            )
            .unwrap(),
        ],
        Vec::new(),
        vec![PublicationArtifact::RecoveryState],
    )
    .unwrap();
    assert!(failed.lifecycle().is_failed());

    for invalid in [
        json!({
            "state": "published",
            "rollback": "performed"
        }),
        json!({
            "state": "dryRun",
            "recovery": "required"
        }),
        json!({
            "state": "cancelled",
            "cancellation": "notRequested",
            "rollback": "notNeeded",
            "cleanup": "completed",
            "recovery": "notRequired"
        }),
        json!({
            "state": "failed",
            "failure": "publication",
            "cancellation": "notRequested",
            "rollback": "failed",
            "cleanup": "completed",
            "recovery": "notRequired"
        }),
    ] {
        assert!(
            serde_json::from_value::<PublicationLifecycle>(invalid).is_err(),
            "invalid publication lifecycle was accepted"
        );
    }

    assert!(PublicationLifecycle::cancelled(
        PublicationCancellation::BeforeExecution,
        PublicationRollback::Performed,
        PublicationCleanup::Completed,
        PublicationRecovery::NotRequired,
    )
    .is_err());
    assert!(PublicationLifecycle::failed(
        PublicationFailureKind::Preparation,
        PublicationCancellation::NotRequested,
        PublicationRollback::Performed,
        PublicationCleanup::Completed,
        PublicationRecovery::NotRequired,
    )
    .is_err());
    assert!(PublicationLifecycle::failed(
        PublicationFailureKind::Cleanup,
        PublicationCancellation::NotRequested,
        PublicationRollback::NotNeeded,
        PublicationCleanup::Completed,
        PublicationRecovery::NotRequired,
    )
    .is_err());

    for invalid in [
        json!({
            "lifecycle": {"state": "published"},
            "diagnostics": [],
            "changes": [],
            "artifacts": []
        }),
        json!({
            "lifecycle": {
                "state": "failed",
                "failure": "cleanup",
                "cancellation": "notRequested",
                "rollback": "notNeeded",
                "cleanup": "failed",
                "recovery": "required"
            },
            "diagnostics": [{
                "code": "publicationCleanupFailed",
                "detail": {"publication": "cleanupFailed"}
            }],
            "changes": [],
            "artifacts": ["recoveryState"]
        }),
    ] {
        assert!(
            serde_json::from_value::<PublicationResult>(invalid).is_err(),
            "incomplete publication result was accepted"
        );
    }
}

#[derive(Debug)]
struct AlternateKindLease;

struct AlternateKindRegistry;

impl ObjectKindRegistryPort for AlternateKindRegistry {
    fn resolve(&self, selector: &ObjectKindSelector) -> Option<SemanticObjectKind> {
        matches!(selector.as_str(), "Widget" | "Widgets")
            .then_some(SemanticObjectKind::Catalog)
    }

    fn ordered_kinds(&self) -> Vec<SemanticObjectKind> {
        vec![SemanticObjectKind::Catalog]
    }

    fn lease(&self, kind: SemanticObjectKind) -> Option<SemanticArtifactLease> {
        (kind == SemanticObjectKind::Catalog)
            .then(|| SemanticArtifactLease::new(AlternateKindLease))
    }

    fn project(&self, lease: &SemanticArtifactLease) -> Option<&'static ObjectKindProjection> {
        lease.adapter_state::<AlternateKindLease>()?;
        static PROJECTION: std::sync::OnceLock<ObjectKindProjection> =
            std::sync::OnceLock::new();
        Some(PROJECTION.get_or_init(|| {
            ObjectKindProjection::new(
                SemanticObjectKind::Catalog,
                ObjectKindSelector::new("Widget").unwrap(),
                ObjectKindSelector::new("Widgets").unwrap(),
                "Widget",
            )
            .unwrap()
        }))
    }
}

#[test]
fn alternate_object_kind_registry_needs_no_format_or_filesystem_concepts() {
    let registry = AlternateKindRegistry;
    let kind = registry
        .resolve(&ObjectKindSelector::new("Widgets").unwrap())
        .unwrap();
    let projection = registry.project(&registry.lease(kind).unwrap()).unwrap();

    assert_eq!(projection.kind(), SemanticObjectKind::Catalog);
    assert_eq!(projection.canonical_selector().as_str(), "Widget");
    assert_eq!(projection.collection_selector().as_str(), "Widgets");
}

#[derive(Debug)]
struct AlternateArtifact(Vec<u8>);

struct AlternateArtifactPort;

impl SemanticArtifactPort for AlternateArtifactPort {
    fn read(
        &self,
        _request: &SemanticArtifactReadRequest,
    ) -> Result<SemanticArtifactReadResult, SourceAdapterError> {
        Ok(SemanticArtifactReadResult::Present(
            SemanticArtifactLease::new(AlternateArtifact(b"semantic-payload".to_vec())),
        ))
    }

    fn bytes<'a>(&self, lease: &'a SemanticArtifactLease) -> Option<&'a [u8]> {
        lease
            .adapter_state::<AlternateArtifact>()
            .map(|artifact| artifact.0.as_slice())
    }
}

#[test]
fn alternate_semantic_artifact_port_needs_no_format_or_path_concepts() {
    let port = AlternateArtifactPort;
    let request = SemanticArtifactReadRequest::new(
        unica_format_core::ports::OperationalSourceSession::new(()),
        SemanticArtifactRole::FormDefinition,
    );
    let SemanticArtifactReadResult::Present(lease) = port.read(&request).unwrap() else {
        panic!("alternate port must return its opaque lease");
    };
    assert_eq!(port.bytes(&lease), Some(b"semantic-payload".as_slice()));
}
