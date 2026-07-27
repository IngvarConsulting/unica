use serde_json::json;
use unica_format_core::ports::{
    SemanticArtifactId, ValidationCoverage, ValidationErrorTruncation, ValidationFinding,
    ValidationFindingCode, ValidationFindingSeverity, ValidationOptions, ValidationReport,
    ValidationStatus,
};

fn subject() -> SemanticArtifactId {
    SemanticArtifactId::new("object:target").unwrap()
}

fn error(code: ValidationFindingCode) -> ValidationFinding {
    ValidationFinding::new(ValidationFindingSeverity::Error, code)
}

fn warning(code: ValidationFindingCode) -> ValidationFinding {
    ValidationFinding::new(ValidationFindingSeverity::Warning, code)
}

#[test]
fn max_errors_accepts_zero_and_names_the_error_limit() {
    let options = ValidationOptions::new(false, 0).unwrap();
    assert_eq!(options.max_errors(), 0);
}

#[test]
fn omitted_errors_preserve_invalid_partial_status_and_mandatory_coverage() {
    let report = ValidationReport::new_with_coverage_and_truncation(
        subject(),
        3,
        vec![warning(
            ValidationFindingCode::RegistrarCoverageNotEvaluated,
        )],
        ValidationCoverage::Partial,
        ValidationErrorTruncation::Truncated,
    )
    .unwrap();

    assert_eq!(report.status(), ValidationStatus::Invalid);
    assert_eq!(report.coverage(), ValidationCoverage::Partial);
    assert_eq!(
        report.error_truncation(),
        ValidationErrorTruncation::Truncated
    );
    assert_eq!(report.findings().len(), 1);
    assert_eq!(
        report.findings()[0].code(),
        ValidationFindingCode::RegistrarCoverageNotEvaluated
    );
    assert_eq!(
        serde_json::to_value(&report).unwrap()["errorTruncation"],
        "truncated"
    );
}

#[test]
fn finding_order_is_deterministic_and_places_mandatory_evidence_first() {
    let report = ValidationReport::new_with_coverage_and_truncation(
        subject(),
        5,
        vec![
            warning(ValidationFindingCode::CommandPresentationTooLong),
            error(ValidationFindingCode::NameMissing),
            warning(ValidationFindingCode::RegistrarCoveragePartial),
            error(ValidationFindingCode::IdentityInvalid),
        ],
        ValidationCoverage::Partial,
        ValidationErrorTruncation::Complete,
    )
    .unwrap();

    assert_eq!(
        report
            .findings()
            .iter()
            .map(ValidationFinding::code)
            .collect::<Vec<_>>(),
        vec![
            ValidationFindingCode::RegistrarCoveragePartial,
            ValidationFindingCode::IdentityInvalid,
            ValidationFindingCode::NameMissing,
            ValidationFindingCode::CommandPresentationTooLong,
        ]
    );
}

#[test]
fn serde_rejects_status_truncation_and_coverage_contradictions() {
    let contradictory = [
        json!({
            "subject": "object:target",
            "status": "valid",
            "coverage": "complete",
            "checks": 1,
            "findings": [],
            "errorTruncation": "truncated"
        }),
        json!({
            "subject": "object:target",
            "status": "partial",
            "coverage": "partial",
            "checks": 2,
            "findings": [{
                "severity": "warning",
                "code": "registrarCoverageNotEvaluated"
            }],
            "errorTruncation": "truncated"
        }),
        json!({
            "subject": "object:target",
            "status": "invalid",
            "coverage": "partial",
            "checks": 2,
            "findings": [{
                "severity": "error",
                "code": "semanticValueInvalid"
            }],
            "errorTruncation": "complete"
        }),
    ];

    for value in contradictory {
        assert!(
            serde_json::from_value::<ValidationReport>(value.clone()).is_err(),
            "accepted contradictory report: {value}"
        );
    }
}

#[test]
fn complete_and_truncated_reports_round_trip_through_the_closed_wire_contract() {
    for truncation in [
        ValidationErrorTruncation::Complete,
        ValidationErrorTruncation::Truncated,
    ] {
        let findings = if truncation == ValidationErrorTruncation::Complete {
            vec![error(ValidationFindingCode::SemanticValueInvalid)]
        } else {
            Vec::new()
        };
        let report = ValidationReport::new_with_coverage_and_truncation(
            subject(),
            2,
            findings,
            ValidationCoverage::Complete,
            truncation,
        )
        .unwrap();
        let wire = serde_json::to_value(&report).unwrap();
        assert_eq!(
            serde_json::from_value::<ValidationReport>(wire).unwrap(),
            report
        );
    }
}
