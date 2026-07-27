use serde_json::json;
use unica_format_core::{
    navigation::Authorability,
    ports::{
        AuthorabilityResult, CompatibilityResult, FormatDiagnostic, FormatDiagnosticCode,
        FormatDiagnosticDetail, OperationalEvidenceRevision, OperationalValidationResult,
        SemanticArtifactId, SupportState, SupportSummary, ValidationContext,
        ValidationContextResult, ValidationFinding, ValidationFindingCode,
        ValidationFindingSeverity, ValidationOwnerKind, ValidationReport,
    },
};

fn evidence(byte: u8) -> OperationalEvidenceRevision {
    OperationalEvidenceRevision::from_digest([byte; 32])
}

#[test]
fn every_read_derived_operational_result_carries_opaque_evidence() {
    let revision = evidence(0x2a);
    let compatibility = CompatibilityResult::compatible(revision.clone());
    assert_eq!(compatibility.evidence_revision(), &revision);

    let summary = SupportSummary::new(SupportState::Absent, None, 0, [0; 3]).unwrap();
    let authorability = AuthorabilityResult::allowed(summary, revision.clone()).unwrap();
    assert_eq!(authorability.evidence_revision(), &revision);

    let context = ValidationContext::new(
        ValidationOwnerKind::Aggregate,
        Vec::new(),
        false,
        None,
        None,
        None,
    )
    .unwrap();
    let validation_context = ValidationContextResult::valid(context, revision.clone());
    assert_eq!(validation_context.evidence_revision(), &revision);

    let report = ValidationReport::new(
        SemanticArtifactId::new("artifact:target").unwrap(),
        1,
        vec![ValidationFinding::new(
            ValidationFindingSeverity::Warning,
            ValidationFindingCode::UnsupportedCombination,
        )],
    )
    .unwrap();
    let validation = OperationalValidationResult::new(vec![report], revision.clone()).unwrap();
    assert_eq!(validation.evidence_revision(), &revision);

    let denied = AuthorabilityResult::denied(
        Authorability::UnknownSupportState,
        SupportSummary::new(SupportState::Unreadable, None, 0, [0; 3]).unwrap(),
        FormatDiagnostic::new(
            FormatDiagnosticCode::SupportStateUnreadable,
            FormatDiagnosticDetail::Support(SupportState::Unreadable),
        )
        .unwrap(),
        revision.clone(),
    )
    .unwrap();
    assert_eq!(denied.evidence_revision(), &revision);
}

#[test]
fn evidence_wire_value_is_fixed_width_lower_hex_and_never_free_form() {
    let revision = evidence(0xab);
    assert_eq!(
        serde_json::to_value(&revision).unwrap(),
        json!("abababababababababababababababababababababababababababababababab")
    );
    assert_eq!(
        serde_json::from_value::<OperationalEvidenceRevision>(json!(
            "abababababababababababababababababababababababababababababababab"
        ))
        .unwrap(),
        revision
    );

    for invalid in [
        json!(""),
        json!("ABABABABABABABABABABABABABABABABABABABABABABABABABABABABABAB"),
        json!("/private/source/Configuration.xml"),
        json!("abab"),
        json!(vec!["ab"; 32]),
    ] {
        assert!(
            serde_json::from_value::<OperationalEvidenceRevision>(invalid).is_err(),
            "invalid or path-bearing evidence was accepted"
        );
    }
}

#[test]
fn authorability_wire_requires_and_preserves_evidence_revision() {
    let result = AuthorabilityResult::allowed(
        SupportSummary::new(SupportState::Absent, None, 0, [0; 3]).unwrap(),
        evidence(0x11),
    )
    .unwrap();
    let wire = serde_json::to_value(&result).unwrap();
    assert_eq!(
        wire["evidenceRevision"],
        json!("1111111111111111111111111111111111111111111111111111111111111111")
    );
    assert_eq!(
        serde_json::from_value::<AuthorabilityResult>(wire)
            .unwrap()
            .evidence_revision(),
        &evidence(0x11)
    );

    assert!(serde_json::from_value::<AuthorabilityResult>(json!({
        "decision": "allowed",
        "authorability": "authorable",
        "summary": {
            "state": "absent",
            "editingEnabled": null,
            "vendorCount": 0,
            "ruleCounts": [0, 0, 0]
        }
    }))
    .is_err());
}
