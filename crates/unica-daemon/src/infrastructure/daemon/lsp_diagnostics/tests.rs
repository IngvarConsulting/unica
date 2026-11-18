use super::*;
use unica_address::source_target::TargetKind;
use unica_diagnostics::model::{
    DiagnosticContext, DiagnosticError, DiagnosticFocusKind, DiagnosticProvider,
    DiagnosticProviderDescriptor, DiagnosticProviderId, DiagnosticProviderOutcome,
    DiagnosticProviderRequest, DiagnosticProviderSection, DiagnosticRange, DiagnosticResultState,
    DiagnosticSelection, EditorFinding,
};
use unica_source_contract::location::SourceLocation;

const PROVIDER: &str = "secret-analyzer";
static DESCRIPTOR: DiagnosticProviderDescriptor = DiagnosticProviderDescriptor {
    id: DiagnosticProviderId::new_const(PROVIDER),
    actions: &[DiagnosticAction::Analyze],
    findings_target_kinds: &[TargetKind::Module],
    emits_focus_kinds: &[DiagnosticFocusKind::SourceRange],
};

struct Provider;

impl DiagnosticProvider for Provider {
    fn descriptor(&self) -> &'static DiagnosticProviderDescriptor {
        &DESCRIPTOR
    }

    fn execute(
        &self,
        _: &DiagnosticProviderRequest,
        _: &DiagnosticContext,
        _: ProviderDeadline,
        _: &CancellationToken,
    ) -> DiagnosticProviderOutcome {
        unreachable!("the translation does not run the provider")
    }

    fn editor_finding(&self, code: &str, severity: DiagnosticSeverity) -> EditorFinding {
        match code {
            "ProviderRuleName" => EditorFinding {
                code: "bsl.code.known",
                message: "Текст Unica",
            },
            _ => EditorFinding::generic(severity),
        }
    }
}

fn finding(code: &str, line: usize) -> DiagnosticItem {
    DiagnosticItem::Diagnostic {
        provider: PROVIDER,
        location: SourceLocation::Addressed {
            source_set: "main".to_string(),
            metadata_path: None,
            target_kind: TargetKind::Module,
        },
        location_reason: None,
        focus: DiagnosticFocus::SourceRange {
            range: DiagnosticRange {
                start_line: line,
                start_column: 0,
                end_line: line,
                end_column: 3,
            },
        },
        code: code.to_string(),
        severity: DiagnosticSeverity::Warning,
        message: "provider wording".to_string(),
        tags: Vec::new(),
    }
}

fn result(status: DiagnosticProviderStatus, error: Option<&str>) -> DiagnosticResult {
    DiagnosticResult {
        ok: true,
        action: DiagnosticAction::Analyze,
        selection: DiagnosticSelection {
            source_set: "main".to_string(),
            metadata_path: None,
            target_kind: None,
            providers: vec![PROVIDER],
            filter: None,
            limit: None,
        },
        state: DiagnosticResultState::Completed,
        complete: error.is_none(),
        providers: vec![DiagnosticProviderSection {
            id: PROVIDER,
            status,
            complete: error.is_none(),
            version: None,
            capabilities: None,
            readiness: None,
            items_total: None,
            items_returned: None,
            resource_failures: None,
            truncated: None,
            error: error.map(|code| DiagnosticError {
                code: code.to_string(),
                message: "provider wording".to_string(),
                retryable: true,
            }),
            suppressions: Vec::new(),
        }],
        items_total: None,
        items_returned: None,
        truncated: None,
        items: vec![finding("ProviderRuleName", 0), finding("UnlistedRule", 1)],
    }
}

fn registry() -> DiagnosticProviderRegistry {
    DiagnosticProviderRegistry::new(vec![Arc::new(Provider)]).unwrap()
}

/// rule: INV.LSP.PROVIDER-HIDDEN
#[test]
fn provider_findings_reach_the_editor_in_unica_words_only() {
    let analysis = provider_analysis(
        &result(DiagnosticProviderStatus::Completed, None),
        &registry(),
    );
    assert!(analysis.gaps.is_empty());
    assert_eq!(analysis.findings[0].code, "bsl.code.known");
    assert_eq!(analysis.findings[0].message, "Текст Unica");
    assert_eq!(analysis.findings[0].severity, LspSeverity::Warning);
    assert_eq!(analysis.findings[1].code, EditorFinding::GENERIC_CODE);
    let told = format!("{analysis:?}");
    for provider in [
        PROVIDER,
        "ProviderRuleName",
        "UnlistedRule",
        "provider wording",
    ] {
        assert!(!told.contains(provider), "{provider} reached the editor");
    }
}

/// rule: INV.LSP.PARTIAL-ANSWERS
#[test]
fn an_unfinished_provider_is_a_gap() {
    for (status, error, gap) in [
        (
            DiagnosticProviderStatus::Failed,
            Some("provider_not_ready"),
            LspDiagnosticGap::AnalysisNotReady,
        ),
        (
            DiagnosticProviderStatus::Unavailable,
            Some("bundled_tool_missing"),
            LspDiagnosticGap::AnalysisAbsent,
        ),
        (
            DiagnosticProviderStatus::Failed,
            Some("provider_timeout"),
            LspDiagnosticGap::AnalysisUnavailable,
        ),
    ] {
        let analysis = provider_analysis(&result(status, error), &registry());
        assert_eq!(analysis.gaps, [gap], "{error:?}");
    }
}

#[test]
fn own_findings_follow_the_text_and_speak_unica() {
    let findings = own_findings("Процедура А()\r\nКонецПроцедуры\r\nПерем Поздно;\r\n");
    assert_eq!(findings.len(), 1, "{findings:?}");
    assert_eq!(findings[0].code, MISPLACED_VARIABLE_CODE);
    assert_eq!(findings[0].severity, LspSeverity::Error);
    assert_eq!(findings[0].start_line, 2);
    assert!(own_findings("Перем Рано;\r\nПроцедура А()\r\nКонецПроцедуры\r\n").is_empty());
}

fn status(
    readiness: Option<unica_diagnostics::model::DiagnosticReadiness>,
    error: Option<&str>,
) -> DiagnosticResult {
    let mut result = result(
        if error.is_some() {
            DiagnosticProviderStatus::Unavailable
        } else {
            DiagnosticProviderStatus::Completed
        },
        error,
    );
    result.items.clear();
    result.providers[0].readiness = readiness;
    result
}

fn readiness(
    state: DiagnosticReadinessState,
    progress: unica_diagnostics::model::DiagnosticBuildProgress,
) -> Option<unica_diagnostics::model::DiagnosticReadiness> {
    Some(unica_diagnostics::model::DiagnosticReadiness {
        state,
        retryable: true,
        progress: Some(progress),
    })
}

/// The preparation reads the resident's own facts: the build's age and
/// retries while it builds, the modules once ready; a build that failed is
/// retried by the resident, anything else is no resident (#2790).
#[test]
fn a_status_answer_tells_how_the_resident_database_goes() {
    use unica_diagnostics::model::DiagnosticBuildProgress;
    assert_eq!(
        resident_database(&status(
            readiness(
                DiagnosticReadinessState::Building,
                DiagnosticBuildProgress {
                    elapsed_ms: Some(25_000),
                    attempts: Some(1),
                    files: None,
                },
            ),
            None,
        )),
        ResidentDatabase::Building {
            elapsed_ms: Some(25_000),
            attempts: Some(1),
        }
    );
    assert_eq!(
        resident_database(&status(
            readiness(
                DiagnosticReadinessState::Ready,
                DiagnosticBuildProgress {
                    files: Some(5902),
                    ..DiagnosticBuildProgress::default()
                },
            ),
            None,
        )),
        ResidentDatabase::Ready { files: Some(5902) }
    );
    assert_eq!(
        resident_database(&status(None, Some("provider_not_ready"))),
        ResidentDatabase::Building {
            elapsed_ms: None,
            attempts: None,
        }
    );
    assert_eq!(
        resident_database(&status(None, Some("provider_start_failed"))),
        ResidentDatabase::Unavailable
    );
}

#[test]
fn the_stages_are_told_in_unica_words() {
    assert_eq!(
        building_message(Duration::from_millis(25_400), None),
        "Строится база: 25 с"
    );
    assert_eq!(
        building_message(Duration::from_secs(3), Some(1)),
        "Повтор после сбоя, попытка 2"
    );
    for (files, told) in [
        (5902, "База готова: 5902 модуля"),
        (1, "База готова: 1 модуль"),
        (11, "База готова: 11 модулей"),
        (21, "База готова: 21 модуль"),
        (25, "База готова: 25 модулей"),
        (112, "База готова: 112 модулей"),
    ] {
        assert_eq!(ready_message(Some(files)), told);
    }
    assert_eq!(ready_message(None), "База готова");
}
