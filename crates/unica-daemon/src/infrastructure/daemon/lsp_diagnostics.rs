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
use std::time::{Duration, Instant};
use unica_address::address::QualifiedAddress;
use unica_diagnostics::coordinator::DiagnosticCoordinator;
use unica_diagnostics::model::{
    DiagnosticAction, DiagnosticFilter, DiagnosticFocus, DiagnosticItem,
    DiagnosticProviderRegistry, DiagnosticProviderStatus, DiagnosticReadinessState,
    DiagnosticRequest, DiagnosticResult, DiagnosticSeverity,
};
use unica_editor_analysis::{LspAnalysis, LspDiagnosticGap, LspFinding, LspSeverity};
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
            message: unica_diagnostics::source_ranges::MISPLACED_VARIABLE_MESSAGE.to_owned(),
        })
        .collect()
}

/// Срок одного вопроса резидентному анализатору о модуле. Готовая база
/// отвечает за миллисекунды, строящаяся — пометкой `loading` сразу; срок
/// ограничивает только зависший ответ.
// resource-limit: lsp.editor-resident-call
// class: deadline
// unit: ms
// scope: request
// value: 60000
// resource: Один вопрос резидентному анализатору о находках модуля для редактора.
// status: unproven
// origin: #2790
// problem: неизвестно
// on-reach: вопрос считается отказом резидента, модуль анализируется разовым процессом, как до резидента
// issue: #2790
const RESIDENT_CALL_BUDGET: Duration = Duration::from_secs(60);
/// Как часто анализ модуля переспрашивает резидента, пока тот строит базу.
// resource-limit: lsp.editor-resident-poll
// class: retry
// unit: ms
// scope: request
// value: 500
// resource: Пауза между вопросами резиденту, пока его база строится.
// status: unproven
// origin: #2790
// problem: неизвестно
// on-reach: анализ модуля спрашивает резидента снова; готовая база отвечает за миллисекунды
// issue: #2790
const RESIDENT_READINESS_POLL: Duration = Duration::from_millis(500);
/// Сколько анализ модуля ждёт, пока резидент построит базу.
// resource-limit: lsp.editor-resident-readiness
// class: deadline
// unit: ms
// scope: request
// value: 600000
// resource: Ожидание готовности базы резидентного анализатора одним анализом модуля для редактора.
// status: unproven
// origin: #2790
// problem: неизвестно
// on-reach: анализ кончается пробелом `analysis_not_ready`; следующий вопрос после задержки повтора спрашивает снова
// issue: #2790
const RESIDENT_READINESS_WAIT: Duration = Duration::from_secs(600);

/// Анализ кода сохранённого модуля тем же координатором, что у `check`, на
/// один модуль. Находки даёт резидентный анализатор: его база строится один
/// раз на рабочую область, и потом модуль отвечает за миллисекунды, а не
/// разбором всей конфигурации на каждый вопрос (`INV.LSP.MCP-PRIORITY`).
/// Пока база строится, анализ ждёт её в фоне и кончается, когда она
/// готова: тогда редактор слышит, что можно спросить снова. Резидент,
/// который отказал, заменяет прежний разовый анализ модуля; его срок —
/// только из настройки пользователя, как у `check`. Анализ прежнего
/// сохранённого текста отменяет `cancellation`. Недоставленный анализатор
/// этот вызов не доставляет: его нет — пробел.
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
    let coordinator = DiagnosticCoordinator::new(running, &mapping);
    let findings = DiagnosticRequest {
        action: DiagnosticAction::Findings,
        source_set: module.source_set().to_string(),
        metadata_path: mapping.target().metadata_path.clone(),
        // The editor shows every level and filters them itself.
        filter: DiagnosticFilter {
            min_severity: None,
            codes: Vec::new(),
        },
        range: None,
        limit: usize::MAX,
        timeout: Some(RESIDENT_CALL_BUDGET),
    };
    let waiting_until = Instant::now() + RESIDENT_READINESS_WAIT;
    loop {
        let resident = match coordinator.execute(&findings, workspace, cancellation) {
            Ok(result) => provider_analysis(&result, &words),
            Err(error) if error.code == "cancelled" => return not_ready(),
            // A registry without a resident provider answers as before.
            Err(_) => break,
        };
        if resident.gaps.is_empty() {
            return resident;
        }
        let building = resident
            .gaps
            .iter()
            .all(|gap| *gap == LspDiagnosticGap::AnalysisNotReady);
        if !building {
            // The resident failed: the module is analysed on its own.
            break;
        }
        if cancellation.is_cancelled() {
            return not_ready();
        }
        if Instant::now() + RESIDENT_READINESS_POLL > waiting_until {
            return resident;
        }
        std::thread::sleep(RESIDENT_READINESS_POLL);
        if cancellation.is_cancelled() {
            return not_ready();
        }
    }
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
    match coordinator.execute_scoped(
        &request,
        workspace,
        cancellation,
        mapping
            .module_scope()
            .map(|scope| (scope, ProviderDeadline::no_deadline())),
    ) {
        Ok(result) => provider_analysis(&result, &words),
        Err(error) if error.code == "cancelled" => not_ready(),
        Err(error) if error.code == "no_applicable_provider" => LspAnalysis {
            findings: Vec::new(),
            gaps: vec![LspDiagnosticGap::AnalysisAbsent],
        },
        Err(_) => unavailable(),
    }
}

fn not_ready() -> LspAnalysis {
    LspAnalysis {
        findings: Vec::new(),
        gaps: vec![LspDiagnosticGap::AnalysisNotReady],
    }
}

/// The resident analysis of `source_set` starts building as soon as an
/// editor opens it: one status question to the configuration root brings
/// the resident up. Its answer is not needed: the analysis of a module asks
/// for itself.
pub(super) fn prepare(
    registry: &dyn Fn() -> Result<DiagnosticProviderRegistry, String>,
    authority: &LogicalViewReadAuthority<'_>,
    workspace: &unica_source_contract::workspace::WorkspaceContext,
    source_set: &str,
    cancellation: &CancellationToken,
) {
    let Ok(running) = registry() else {
        return;
    };
    let Ok(root) = QualifiedAddress::parse(&format!("{source_set}:Configuration")) else {
        return;
    };
    let Ok(mapping) = authority.diagnostic_node_mapping(&root, workspace) else {
        return;
    };
    let request = DiagnosticRequest {
        action: DiagnosticAction::Status,
        source_set: source_set.to_string(),
        metadata_path: None,
        filter: DiagnosticFilter {
            min_severity: None,
            codes: Vec::new(),
        },
        range: None,
        limit: 0,
        timeout: Some(RESIDENT_CALL_BUDGET),
    };
    let _ =
        DiagnosticCoordinator::new(running, &mapping).execute(&request, workspace, cancellation);
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
