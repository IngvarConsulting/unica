use serde_json::json;
use unica_format_core::ports::{
    SemanticArtifactId, ValidationCoverage, ValidationFinding, ValidationFindingCode,
    ValidationFindingSeverity, ValidationReport, ValidationStatus,
};

fn subject() -> SemanticArtifactId {
    SemanticArtifactId::new("object:target").unwrap()
}

fn coverage_finding(code: ValidationFindingCode) -> ValidationFinding {
    ValidationFinding::new(ValidationFindingSeverity::Warning, code)
}

#[test]
fn complete_partial_and_invalid_reports_have_distinct_closed_statuses() {
    let complete = ValidationReport::new(subject(), 1, Vec::new()).unwrap();
    let partial = ValidationReport::new_with_coverage(
        subject(),
        1,
        vec![coverage_finding(
            ValidationFindingCode::RegistrarCoverageNotEvaluated,
        )],
        ValidationCoverage::Partial,
    )
    .unwrap();
    let invalid = ValidationReport::new(
        subject(),
        1,
        vec![ValidationFinding::new(
            ValidationFindingSeverity::Error,
            ValidationFindingCode::SemanticValueInvalid,
        )],
    )
    .unwrap();

    assert_eq!(complete.status(), ValidationStatus::Valid);
    assert_eq!(partial.status(), ValidationStatus::Partial);
    assert_eq!(invalid.status(), ValidationStatus::Invalid);
    assert_eq!(serde_json::to_value(complete).unwrap()["status"], "valid");
    assert_eq!(serde_json::to_value(partial).unwrap()["status"], "partial");
    assert_eq!(serde_json::to_value(invalid).unwrap()["status"], "invalid");
}

#[test]
fn partial_coverage_requires_a_closed_semantic_area_diagnostic() {
    assert!(ValidationReport::new_with_coverage(
        subject(),
        1,
        Vec::new(),
        ValidationCoverage::Partial,
    )
    .is_err());
    assert!(ValidationReport::new_with_coverage(
        subject(),
        1,
        vec![coverage_finding(
            ValidationFindingCode::RegistrarCoverageNotEvaluated,
        )],
        ValidationCoverage::Complete,
    )
    .is_err());
    assert!(ValidationReport::new_with_coverage(
        subject(),
        1,
        vec![ValidationFinding::new(
            ValidationFindingSeverity::Error,
            ValidationFindingCode::RegistrarCoverageNotEvaluated,
        )],
        ValidationCoverage::Partial,
    )
    .is_err());
}

#[test]
fn serde_rejects_every_contradictory_status_coverage_and_finding_combination() {
    let contradictory = [
        json!({
            "subject": "object:target",
            "status": "valid",
            "coverage": "partial",
            "checks": 1,
            "findings": [{
                "severity": "warning",
                "code": "registrarCoverageNotEvaluated"
            }]
        }),
        json!({
            "subject": "object:target",
            "status": "partial",
            "coverage": "complete",
            "checks": 1,
            "findings": []
        }),
        json!({
            "subject": "object:target",
            "status": "partial",
            "coverage": "partial",
            "checks": 1,
            "findings": []
        }),
        json!({
            "subject": "object:target",
            "status": "invalid",
            "coverage": "partial",
            "checks": 1,
            "findings": [{
                "severity": "warning",
                "code": "registrarCoverageNotEvaluated"
            }]
        }),
    ];

    for value in contradictory {
        assert!(
            serde_json::from_value::<ValidationReport>(value.clone()).is_err(),
            "accepted contradictory validation report: {value}"
        );
    }
}

#[test]
fn invalid_findings_dominate_partial_coverage_without_erasing_it() {
    let report = ValidationReport::new_with_coverage(
        subject(),
        2,
        vec![
            coverage_finding(ValidationFindingCode::RegistrarCoveragePartial),
            ValidationFinding::new(
                ValidationFindingSeverity::Error,
                ValidationFindingCode::SemanticValueInvalid,
            ),
        ],
        ValidationCoverage::Partial,
    )
    .unwrap();

    assert_eq!(report.status(), ValidationStatus::Invalid);
    assert_eq!(report.coverage(), ValidationCoverage::Partial);
    assert_eq!(
        serde_json::from_value::<ValidationReport>(serde_json::to_value(&report).unwrap()).unwrap(),
        report
    );
}
