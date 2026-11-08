//! Диагностики редактора на стороне продукта: те же проверки Unica и тот же
//! поставщик анализа, что у `check`, в словах Unica.
//!
//! Поставщик анализа переводит свои находки в код и текст Unica
//! ([`unica_diagnostics::model::DiagnosticProvider::editor_finding`]):
//! имя правила, текст и имя поставщика в редактор не попадают. Поставщик,
//! который не закончил, оставляет пробел, а не пустой полный ответ.

use super::v13_service::CanonicalV13ReadService;
use crate::infrastructure::v13_read::LogicalViewReadAuthority;
use std::sync::Arc;
use unica_address::address::QualifiedAddress;
use unica_diagnostics::coordinator::DiagnosticCoordinator;
use unica_diagnostics::model::{
    DiagnosticAction, DiagnosticFilter, DiagnosticFocus, DiagnosticItem,
    DiagnosticProviderRegistry, DiagnosticProviderStatus, DiagnosticReadinessState,
    DiagnosticRequest, DiagnosticResult, DiagnosticSeverity,
};
use unica_lsp::{LspAnalysis, LspDiagnosticGap, LspFinding, LspSeverity};
use unica_work_control::cancellation::CancellationToken;
use unica_work_control::provider_deadline::ProviderDeadline;

/// Код собственной проверки Unica, что `Перем` стоит там, где его
/// компилирует платформа; тот же код, что у `check`.
const MISPLACED_VARIABLE_CODE: &str = "VariableDeclarationPlacement";

/// Собственные проверки Unica над текстом, который видит соединение.
pub(super) fn own_findings(text: &str) -> Vec<LspFinding> {
    unica_diagnostics::source_ranges::misplaced_variable_declarations(text)
        .into_iter()
        .map(|range| LspFinding {
            code: MISPLACED_VARIABLE_CODE.to_owned(),
            severity: LspSeverity::Error,
            start_line: range.start_line,
            start_column: range.start_column,
            end_line: range.end_line,
            end_column: range.end_column,
            message: unica_bsl_parser::MISPLACED_VARIABLE_MESSAGE.to_owned(),
        })
        .collect()
}

/// Анализ кода сохранённого модуля тем же координатором, что у `check`, на
/// один модуль. Срок — только из настройки пользователя, как у `check`;
/// анализ прежнего сохранённого текста отменяет `cancellation`.
/// Недоставленный анализатор этот вызов не доставляет: его нет — пробел.
pub(super) fn analyse(
    registry: &dyn Fn() -> Result<DiagnosticProviderRegistry, String>,
    authority: &LogicalViewReadAuthority<'_>,
    workspace: &unica_source_contract::workspace::WorkspaceContext,
    module: &QualifiedAddress,
    cancellation: &CancellationToken,
) -> LspAnalysis {
    let unavailable = || LspAnalysis {
        findings: Vec::new(),
        gaps: vec![LspDiagnosticGap::AnalysisUnavailable],
    };
    let (Ok(running), Ok(words)) = (registry(), registry()) else {
        return unavailable();
    };
    let Ok(mapping) = authority.diagnostic_mapping(module, workspace) else {
        return unavailable();
    };
    let Ok(config) = crate::infrastructure::operational_config::load_operational_config(
        &workspace.workspace_root,
    ) else {
        return unavailable();
    };
    let request = DiagnosticRequest {
        action: DiagnosticAction::Analyze,
        source_set: module.source_set().to_string(),
        metadata_path: mapping.target().metadata_path.clone(),
        // The editor shows every level and filters them itself.
        filter: DiagnosticFilter {
            min_severity: None,
            codes: Vec::new(),
        },
        range: None,
        limit: usize::MAX,
        timeout: config.code_diagnostics().analyze_timeout(),
    };
    match DiagnosticCoordinator::new(running, &mapping).execute_scoped(
        &request,
        workspace,
        cancellation,
        mapping
            .module_scope()
            .map(|scope| (scope, ProviderDeadline::no_deadline())),
    ) {
        Ok(result) => provider_analysis(&result, &words),
        Err(error) if error.code == "cancelled" => LspAnalysis {
            findings: Vec::new(),
            gaps: vec![LspDiagnosticGap::AnalysisNotReady],
        },
        Err(error) if error.code == "no_applicable_provider" => LspAnalysis {
            findings: Vec::new(),
            gaps: vec![LspDiagnosticGap::AnalysisAbsent],
        },
        Err(_) => unavailable(),
    }
}

/// The provider part of a document's findings in Unica's words. Each finding
/// is renamed by the provider that made it; a provider that did not finish
/// leaves a gap instead of silence. A finding without a text range belongs
/// to the document as a whole and is no editor diagnostic.
pub(super) fn provider_analysis(
    result: &DiagnosticResult,
    registry: &DiagnosticProviderRegistry,
) -> LspAnalysis {
    let mut analysis = LspAnalysis::default();
    for section in &result.providers {
        let gap = match section.status {
            DiagnosticProviderStatus::Completed | DiagnosticProviderStatus::Empty => {
                match section.readiness.as_ref().map(|readiness| readiness.state) {
                    Some(
                        DiagnosticReadinessState::NotStarted | DiagnosticReadinessState::Building,
                    ) => Some(LspDiagnosticGap::AnalysisNotReady),
                    _ if !section.complete => Some(LspDiagnosticGap::AnalysisUnavailable),
                    _ => None,
                }
            }
            DiagnosticProviderStatus::Unsupported => None,
            DiagnosticProviderStatus::Unavailable | DiagnosticProviderStatus::Failed => Some(
                match section.error.as_ref().map(|error| error.code.as_str()) {
                    Some("provider_not_ready") => LspDiagnosticGap::AnalysisNotReady,
                    Some(
                        "bundled_tool_missing"
                        | "provider_install_invalid"
                        | "provider_start_failed",
                    ) => LspDiagnosticGap::AnalysisAbsent,
                    _ => LspDiagnosticGap::AnalysisUnavailable,
                },
            ),
        };
        analysis.gaps.extend(gap);
    }
    for item in &result.items {
        let DiagnosticItem::Diagnostic {
            provider,
            focus: DiagnosticFocus::SourceRange { range },
            code,
            severity,
            ..
        } = item
        else {
            continue;
        };
        // A finding is told only by a provider the registry knows.
        let Some(port) = registry
            .providers()
            .iter()
            .find(|port| port.descriptor().id.as_str() == *provider)
        else {
            continue;
        };
        let finding = port.editor_finding(code, *severity);
        analysis.findings.push(LspFinding {
            code: finding.code.to_owned(),
            severity: match severity {
                DiagnosticSeverity::Error => LspSeverity::Error,
                DiagnosticSeverity::Warning => LspSeverity::Warning,
                DiagnosticSeverity::Info => LspSeverity::Information,
                DiagnosticSeverity::Hint => LspSeverity::Hint,
            },
            start_line: range.start_line,
            start_column: range.start_column,
            end_line: range.end_line,
            end_column: range.end_column,
            message: finding.message.to_owned(),
        });
    }
    analysis
}

/// The registry the editor's analysis uses: the product's, or in a test the
/// provider that test put in place on its own thread.
pub(super) fn registry_for(
    service: Arc<CanonicalV13ReadService>,
) -> impl Fn() -> Result<DiagnosticProviderRegistry, String> + Send + Sync {
    #[cfg(test)]
    let substituted = crate::infrastructure::application_ports::DIAGNOSTIC_PROVIDER_OVERRIDE
        .with(|slot| slot.borrow().clone());
    move || {
        #[cfg(test)]
        if let Some(provider) = &substituted {
            return DiagnosticProviderRegistry::new(vec![Arc::clone(provider)])
                .map_err(|error| error.to_string());
        }
        service.diagnostic_registry()
    }
}

#[cfg(test)]
#[path = "lsp_diagnostics/tests.rs"]
mod tests;
