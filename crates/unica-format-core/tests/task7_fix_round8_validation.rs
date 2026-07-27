use serde_json::json;
use unica_format_core::ports::{
    SemanticArtifactId, ValidationCoverage, ValidationErrorTruncation, ValidationFinding,
    ValidationFindingCode, ValidationFindingSeverity, ValidationReport, ValidationStatus,
};

fn subject() -> SemanticArtifactId {
    SemanticArtifactId::new("object:target").unwrap()
}

#[test]
fn truncation_carries_a_nonzero_structured_omitted_error_count() {
    assert!(ValidationErrorTruncation::truncated(0).is_err());

    let truncation = ValidationErrorTruncation::truncated(7).unwrap();
    assert!(truncation.is_truncated());
    assert_eq!(truncation.omitted_errors(), 7);
    assert_eq!(
        serde_json::to_value(truncation).unwrap(),
        json!({"state": "truncated", "omitted": 7})
    );
    assert!(serde_json::from_value::<ValidationErrorTruncation>(
        json!({"state": "truncated", "omitted": 0})
    )
    .is_err());
}

#[test]
fn omitted_errors_preserve_invalid_status_without_retained_error_findings() {
    let report = ValidationReport::new_with_coverage_and_truncation(
        subject(),
        3,
        Vec::new(),
        ValidationCoverage::Complete,
        ValidationErrorTruncation::truncated(3).unwrap(),
    )
    .unwrap();

    assert_eq!(report.status(), ValidationStatus::Invalid);
    assert!(report.findings().is_empty());
    assert_eq!(report.error_truncation().omitted_errors(), 3);
}

#[test]
fn report_constructor_normalizes_duplicate_findings_deterministically() {
    let duplicate = ValidationFinding::new(
        ValidationFindingSeverity::Error,
        ValidationFindingCode::SemanticValueInvalid,
    );
    let report = ValidationReport::new_with_coverage_and_truncation(
        subject(),
        3,
        vec![duplicate.clone(), duplicate],
        ValidationCoverage::Complete,
        ValidationErrorTruncation::Complete,
    )
    .unwrap();

    assert_eq!(report.findings().len(), 1);
}
