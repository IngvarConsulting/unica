//! Editor diagnostics of one BSL document (#2176), in Unica's own model and
//! codes: `textDocument/diagnostic` answers and `publishDiagnostics` payloads
//! in LSP JSON, zero-based lines and UTF-16 characters.
//!
//! Nothing of an analysis provider reaches the wire: a finding carries a
//! Unica code, a Unica message and `source: "unica"`. A source that is not
//! ready leaves a gap; a report with a gap is marked incomplete and is never
//! answered as `unchanged`, so the editor asks again once the source is ready.

use crate::application::lsp::position::LineIndex;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};
use unica_diagnostics::model::{
    DiagnosticFocus, DiagnosticItem, DiagnosticProviderRegistry, DiagnosticProviderStatus,
    DiagnosticRange, DiagnosticReadinessState, DiagnosticResult, DiagnosticSeverity,
};
use unica_work_control::cancellation::CancellationToken;

/// The code of Unica's own check that `Перем` stands where the platform
/// compiles it; the same code `check` reports.
const MISPLACED_VARIABLE_CODE: &str = "VariableDeclarationPlacement";
/// The code of the marker that a report misses a source.
const INCOMPLETE_CODE: &str = "DiagnosticsIncomplete";
const SOURCE: &str = "unica";

// LSP `DiagnosticSeverity`.
const SEVERITY_ERROR: u32 = 1;
const SEVERITY_WARNING: u32 = 2;
const SEVERITY_INFORMATION: u32 = 3;
const SEVERITY_HINT: u32 = 4;

/// Why a report is not complete. Each names a source in Unica's words.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum DiagnosticGap {
    /// The code analysis has not finished yet.
    AnalysisNotReady,
    /// The code analysis answered with a failure.
    AnalysisUnavailable,
    /// The code analysis is not installed or cannot start.
    AnalysisAbsent,
    /// The buffer differs from the saved file, and the code analysis only
    /// reads saved files.
    UnsavedBuffer,
    /// The document has more findings than one answer carries.
    Capped,
}

impl DiagnosticGap {
    fn code(self) -> &'static str {
        match self {
            Self::AnalysisNotReady => "analysisNotReady",
            Self::AnalysisUnavailable => "analysisUnavailable",
            Self::AnalysisAbsent => "analysisAbsent",
            Self::UnsavedBuffer => "unsavedBuffer",
            Self::Capped => "capped",
        }
    }

    fn message(self) -> &'static str {
        match self {
            Self::AnalysisNotReady => "анализ кода ещё идёт",
            Self::AnalysisUnavailable => "анализ кода недоступен",
            Self::AnalysisAbsent => "анализ кода не установлен или не запускается",
            Self::UnsavedBuffer => "анализ кода видит только сохранённый файл",
            Self::Capped => "показаны не все находки",
        }
    }
}

/// One finding in Unica's model: its code and message are Unica's own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DocumentFinding {
    pub(crate) code: String,
    pub(crate) severity: DiagnosticSeverity,
    /// Zero-based lines, columns in Unicode scalar values.
    pub(crate) range: DiagnosticRange,
    pub(crate) message: String,
}

/// The findings of one document and the sources the answer lacks.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct DocumentDiagnostics {
    pub(crate) findings: Vec<DocumentFinding>,
    pub(crate) gaps: Vec<DiagnosticGap>,
}

impl DocumentDiagnostics {
    pub(crate) fn complete(&self) -> bool {
        self.gaps.is_empty()
    }
}

/// Unica's own checks of a BSL text. They read the text the connection
/// sees, so they follow the open buffer without saving it.
pub(crate) fn own_findings(text: &str) -> Vec<DocumentFinding> {
    unica_diagnostics::source_ranges::misplaced_variable_declarations(text)
        .into_iter()
        .map(|range| DocumentFinding {
            code: MISPLACED_VARIABLE_CODE.to_string(),
            severity: DiagnosticSeverity::Error,
            range,
            message: unica_bsl_parser::MISPLACED_VARIABLE_MESSAGE.to_string(),
        })
        .collect()
}

/// The provider part of a document's diagnostics, in Unica's words. Each
/// finding is renamed by the provider that made it ([`unica_diagnostics::model::DiagnosticProvider::editor_finding`]);
/// a provider that did not finish leaves a gap instead of silence. A finding
/// without a text range belongs to the document as a whole and is not an
/// editor diagnostic.
pub(crate) fn provider_diagnostics(
    result: &DiagnosticResult,
    registry: &DiagnosticProviderRegistry,
) -> DocumentDiagnostics {
    let mut diagnostics = DocumentDiagnostics::default();
    for section in &result.providers {
        let gap = match section.status {
            DiagnosticProviderStatus::Completed | DiagnosticProviderStatus::Empty => {
                match section.readiness.as_ref().map(|readiness| readiness.state) {
                    Some(
                        DiagnosticReadinessState::NotStarted | DiagnosticReadinessState::Building,
                    ) => Some(DiagnosticGap::AnalysisNotReady),
                    _ if !section.complete => Some(DiagnosticGap::AnalysisUnavailable),
                    _ => None,
                }
            }
            DiagnosticProviderStatus::Unsupported => None,
            DiagnosticProviderStatus::Unavailable | DiagnosticProviderStatus::Failed => Some(
                match section.error.as_ref().map(|error| error.code.as_str()) {
                    Some("provider_not_ready") => DiagnosticGap::AnalysisNotReady,
                    Some(
                        "bundled_tool_missing"
                        | "provider_install_invalid"
                        | "provider_start_failed",
                    ) => DiagnosticGap::AnalysisAbsent,
                    _ => DiagnosticGap::AnalysisUnavailable,
                },
            ),
        };
        diagnostics.gaps.extend(gap);
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
        // The answer is never told by a provider the registry does not know.
        let Some(port) = registry
            .providers()
            .iter()
            .find(|port| port.descriptor().id.as_str() == *provider)
        else {
            continue;
        };
        let finding = port.editor_finding(code, *severity);
        diagnostics.findings.push(DocumentFinding {
            code: finding.code.to_string(),
            severity: *severity,
            range: *range,
            message: finding.message.to_string(),
        });
    }
    diagnostics
}

/// One document's answer: Unica's own findings over the buffer and the
/// provider's findings over the saved file, with every gap of both.
pub(crate) fn merge(
    own: Vec<DocumentFinding>,
    provider: DocumentDiagnostics,
) -> DocumentDiagnostics {
    let mut findings = own;
    findings.extend(provider.findings);
    let mut gaps = provider.gaps;
    findings.sort_by_key(|finding| {
        (
            finding.range.start_line,
            finding.range.start_column,
            finding.code.clone(),
        )
    });
    if findings.len() > MAX_EDITOR_DIAGNOSTICS {
        findings.truncate(MAX_EDITOR_DIAGNOSTICS);
        gaps.push(DiagnosticGap::Capped);
    }
    DocumentDiagnostics { findings, gaps }
}

/// The most findings one document answer carries; the rest are named.
const MAX_EDITOR_DIAGNOSTICS: usize = 1000;

/// How many saved texts the editor keeps an analysis for, waiting or done;
/// the oldest done goes first, then the oldest still waiting its turn.
const MAX_EDITOR_ANALYSIS_ENTRIES: usize = 256;
/// How many analyses run at once; the rest wait their turn, so opening many
/// documents does not start an analyzer for each.
const MAX_CONCURRENT_EDITOR_ANALYSES: usize = 2;
/// How long an unfinished analysis answers for its saved text before it is
/// run again for those who asked.
const EDITOR_ANALYSIS_RETRY_DELAY: Duration = Duration::from_secs(30);

/// The provider's findings by saved text, computed in the background (#2176).
/// A pull never waits for the analysis: it answers what is ready and starts
/// the rest once; the finished analysis calls back everyone who asked, so the
/// editor is offered the complete answer. An unfinished one calls them back
/// again after a while, so the editor asks anew. No deadline is put on the
/// provider (#2189); a newer saved text of the same module cancels the older
/// analysis and takes over those who waited for it.
pub(crate) struct EditorAnalysis {
    entries: Mutex<EditorAnalysisEntries>,
    retry_delay: Duration,
}

impl Default for EditorAnalysis {
    fn default() -> Self {
        Self {
            entries: Mutex::default(),
            retry_delay: EDITOR_ANALYSIS_RETRY_DELAY,
        }
    }
}

/// Called when an analysis ends; called again after the retry delay when
/// it ended unfinished.
pub(crate) type AnalysisWaiter = Arc<dyn Fn() + Send + Sync>;
type Job = Box<dyn FnOnce(&CancellationToken) -> DocumentDiagnostics + Send>;

#[derive(Default)]
struct EditorAnalysisEntries {
    order: VecDeque<AnalysisKey>,
    by_key: HashMap<AnalysisKey, AnalysisSlot>,
    queue: VecDeque<(AnalysisKey, u64, Job)>,
    running: usize,
    next_generation: u64,
}

enum AnalysisSlot {
    /// Queued or running. A job finishing for another generation of the
    /// key was cancelled and is not its answer.
    Pending {
        generation: u64,
        waiters: BTreeMap<String, AnalysisWaiter>,
        cancellation: CancellationToken,
    },
    /// The answer, and for an incomplete one the moment it may be retried.
    Done {
        diagnostics: DocumentDiagnostics,
        retry_after: Option<Instant>,
    },
}

impl EditorAnalysis {
    #[cfg(test)]
    pub(crate) fn with_retry_delay(retry_delay: Duration) -> Self {
        Self {
            entries: Mutex::default(),
            retry_delay,
        }
    }

    /// The finished analysis of `key`, or `None` while it waits or runs.
    /// The first request queues `analyse`; `ready` is called when it ends.
    /// One waiter per `waiter` name: asking again does not call back twice.
    pub(crate) fn findings(
        self: &Arc<Self>,
        key: AnalysisKey,
        waiter: &str,
        analyse: impl FnOnce(&CancellationToken) -> DocumentDiagnostics + Send + 'static,
        ready: AnalysisWaiter,
    ) -> Option<DocumentDiagnostics> {
        let mut entries = self.lock();
        match entries.by_key.get_mut(&key) {
            Some(AnalysisSlot::Done {
                diagnostics,
                retry_after,
            }) if retry_after.is_none_or(|retry| Instant::now() < retry) => {
                return Some(diagnostics.clone())
            }
            Some(AnalysisSlot::Pending { waiters, .. }) => {
                waiters.insert(waiter.to_owned(), ready);
                return None;
            }
            _ => {}
        }
        // An older saved text of the same module is no longer asked about;
        // those who waited for it wait for this one.
        let mut waiters = BTreeMap::new();
        let stale = entries
            .by_key
            .keys()
            .filter(|kept| kept.same_module(&key))
            .cloned()
            .collect::<Vec<_>>();
        for stale in stale {
            if let Some(AnalysisSlot::Pending {
                waiters: waited,
                cancellation,
                ..
            }) = entries.remove(&stale)
            {
                cancellation.cancel();
                waiters.extend(waited);
            }
        }
        waiters.insert(waiter.to_owned(), ready);
        let generation = entries.next_generation;
        entries.next_generation += 1;
        entries.by_key.insert(
            key.clone(),
            AnalysisSlot::Pending {
                generation,
                waiters,
                cancellation: CancellationToken::new(),
            },
        );
        entries.order.push_back(key.clone());
        entries
            .queue
            .push_back((key, generation, Box::new(analyse)));
        entries.evict();
        self.start_queued(&mut entries);
        None
    }

    fn lock(&self) -> MutexGuard<'_, EditorAnalysisEntries> {
        self.entries.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn start_queued(self: &Arc<Self>, entries: &mut EditorAnalysisEntries) {
        while entries.running < MAX_CONCURRENT_EDITOR_ANALYSES {
            let Some((key, generation, job)) = entries.queue.pop_front() else {
                return;
            };
            let cancellation = match entries.by_key.get(&key) {
                Some(AnalysisSlot::Pending {
                    generation: current,
                    cancellation,
                    ..
                }) if *current == generation => cancellation.clone(),
                _ => continue,
            };
            entries.running += 1;
            let analysis = Arc::clone(self);
            std::thread::spawn(move || analysis.run(key, generation, job, &cancellation));
        }
    }

    fn run(
        self: Arc<Self>,
        key: AnalysisKey,
        generation: u64,
        job: Job,
        token: &CancellationToken,
    ) {
        // A failure outside the provider is an unavailable analysis, never
        // a slot that stays taken.
        let done =
            catch_unwind(AssertUnwindSafe(|| job(token))).unwrap_or_else(|_| DocumentDiagnostics {
                findings: Vec::new(),
                gaps: vec![DiagnosticGap::AnalysisUnavailable],
            });
        // A gap the analyzer may close by itself is asked about again after
        // a while; an absent analyzer stays absent for this text.
        let retry = done
            .gaps
            .iter()
            .any(|gap| *gap != DiagnosticGap::AnalysisAbsent);
        let waiters = {
            let mut entries = self.lock();
            entries.running -= 1;
            let waiters = match entries.by_key.get(&key) {
                Some(AnalysisSlot::Pending {
                    generation: current,
                    ..
                }) if *current == generation => {
                    let Some(AnalysisSlot::Pending { waiters, .. }) = entries.by_key.insert(
                        key,
                        AnalysisSlot::Done {
                            diagnostics: done,
                            retry_after: retry.then(|| Instant::now() + self.retry_delay),
                        },
                    ) else {
                        unreachable!("the slot was pending");
                    };
                    waiters.into_values().collect::<Vec<_>>()
                }
                // Cancelled for a newer saved text: that one answers.
                _ => Vec::new(),
            };
            self.start_queued(&mut entries);
            waiters
        };
        for waiter in &waiters {
            waiter();
        }
        if retry && !waiters.is_empty() {
            let delay = self.retry_delay;
            std::thread::spawn(move || {
                std::thread::sleep(delay);
                for waiter in &waiters {
                    waiter();
                }
            });
        }
    }
}

impl EditorAnalysisEntries {
    fn remove(&mut self, key: &AnalysisKey) -> Option<AnalysisSlot> {
        self.queue.retain(|(queued, _, _)| queued != key);
        self.order.retain(|kept| kept != key);
        self.by_key.remove(key)
    }

    /// Keep at most `MAX_EDITOR_ANALYSIS_ENTRIES`: the oldest done analysis
    /// goes first, then the oldest one still queued. Its waiters are not
    /// called: their documents show the analysis as still running until
    /// they are asked about again.
    fn evict(&mut self) {
        while self.order.len() > MAX_EDITOR_ANALYSIS_ENTRIES {
            let done = self
                .order
                .iter()
                .find(|kept| matches!(self.by_key.get(*kept), Some(AnalysisSlot::Done { .. })));
            let queued = || {
                self.order
                    .iter()
                    .find(|kept| self.queue.iter().any(|(queued, _, _)| queued == *kept))
            };
            let Some(oldest) = done.or_else(queued).cloned() else {
                return;
            };
            if let Some(AnalysisSlot::Pending { cancellation, .. }) = self.remove(&oldest) {
                cancellation.cancel();
            }
        }
    }
}

/// A module of a workspace and the digest of its saved text.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct AnalysisKey {
    pub(crate) workspace: String,
    pub(crate) module: String,
    pub(crate) saved_digest: [u8; 32],
}

impl AnalysisKey {
    pub(crate) fn new(
        workspace: impl Into<String>,
        module: impl Into<String>,
        saved: &[u8],
    ) -> Self {
        Self {
            workspace: workspace.into(),
            module: module.into(),
            saved_digest: Sha256::digest(saved).into(),
        }
    }

    fn same_module(&self, other: &Self) -> bool {
        self.workspace == other.workspace && self.module == other.module
    }
}

/// LSP `Diagnostic[]` of a document. An incomplete report ends with one
/// marker at the start of the document that names the missing sources.
pub(crate) fn diagnostics_json(text: &str, diagnostics: &DocumentDiagnostics) -> Value {
    let lines = LineIndex::new(text);
    let mut items = diagnostics
        .findings
        .iter()
        .map(|finding| {
            json!({
                "range": range(text, &lines, finding.range),
                "severity": severity(finding.severity),
                "code": finding.code,
                "source": SOURCE,
                "message": finding.message,
            })
        })
        .collect::<Vec<_>>();
    if !diagnostics.complete() {
        let mut gaps = diagnostics.gaps.clone();
        gaps.sort();
        gaps.dedup();
        items.push(json!({
            "range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 0}},
            "severity": SEVERITY_INFORMATION,
            "code": INCOMPLETE_CODE,
            "source": SOURCE,
            "message": format!(
                "Диагностики неполные: {}",
                gaps.iter().map(|gap| gap.message()).collect::<Vec<_>>().join("; ")
            ),
            "data": {"complete": false, "gaps": gaps.iter().map(|gap| gap.code()).collect::<Vec<_>>()},
        }));
    }
    Value::Array(items)
}

/// `textDocument/diagnostic`: a full report, or `unchanged` when the
/// editor already holds this complete result. An incomplete result never
/// stays: the next pull recomputes it.
pub(crate) fn document_report_json(
    text: &str,
    diagnostics: &DocumentDiagnostics,
    previous_result_id: Option<&str>,
) -> Value {
    let items = diagnostics_json(text, diagnostics);
    let result_id = result_id(&items);
    if diagnostics.complete() && previous_result_id == Some(result_id.as_str()) {
        return json!({"kind": "unchanged", "resultId": result_id});
    }
    json!({"kind": "full", "resultId": result_id, "items": items})
}

/// `textDocument/publishDiagnostics` parameters for one document version.
pub(crate) fn publish_json(
    uri: &str,
    version: Option<i32>,
    text: &str,
    diagnostics: &DocumentDiagnostics,
) -> Value {
    let mut params = json!({"uri": uri, "diagnostics": diagnostics_json(text, diagnostics)});
    if let Some(version) = version {
        params["version"] = json!(version);
    }
    params
}

/// `textDocument/publishDiagnostics` params of a closed document: the
/// editor drops what it showed for it.
pub(crate) fn closed_json(uri: &str) -> Value {
    json!({"uri": uri, "diagnostics": []})
}

fn result_id(items: &Value) -> String {
    let digest = Sha256::digest(items.to_string().as_bytes());
    digest[..16]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn severity(severity: DiagnosticSeverity) -> u32 {
    match severity {
        DiagnosticSeverity::Error => SEVERITY_ERROR,
        DiagnosticSeverity::Warning => SEVERITY_WARNING,
        DiagnosticSeverity::Info => SEVERITY_INFORMATION,
        DiagnosticSeverity::Hint => SEVERITY_HINT,
    }
}

/// A range of zero-based lines and scalar-value columns as an LSP range. A
/// point past its line ends at the line's end.
fn range(text: &str, lines: &LineIndex, range: DiagnosticRange) -> Value {
    json!({
        "start": position(text, lines, range.start_line, range.start_column),
        "end": position(text, lines, range.end_line, range.end_column),
    })
}

fn position(text: &str, lines: &LineIndex, line: usize, column: usize) -> Value {
    let offset = lines
        .line_bytes(text, line)
        .map(|(start, end)| {
            text[start..end]
                .char_indices()
                .nth(column)
                .map_or(end, |(index, _)| start + index)
        })
        .unwrap_or(text.len());
    let position = lines.position(text, offset);
    json!({"line": position.line, "character": position.character})
}

#[cfg(test)]
mod tests {
    use super::*;

    const MODULE: &str = "Процедура А()\r\nКонецПроцедуры\r\nПерем Поздно;\r\n";

    #[test]
    fn own_findings_follow_the_text_and_speak_unica() {
        let findings = own_findings(MODULE);
        assert_eq!(findings.len(), 1, "{findings:?}");
        let report = diagnostics_json(
            MODULE,
            &DocumentDiagnostics {
                findings,
                gaps: Vec::new(),
            },
        );
        let item = &report[0];
        assert_eq!(item["code"], MISPLACED_VARIABLE_CODE);
        assert_eq!(item["source"], "unica");
        assert_eq!(item["severity"], SEVERITY_ERROR);
        assert_eq!(item["range"]["start"], json!({"line": 2, "character": 0}));
        assert!(own_findings("Перем Рано;\r\nПроцедура А()\r\nКонецПроцедуры\r\n").is_empty());
    }

    #[test]
    fn columns_are_counted_in_utf16_units() {
        // `😀` is one scalar value and two UTF-16 units.
        let text = "А😀Б\n";
        let finding = DocumentFinding {
            code: "X".into(),
            severity: DiagnosticSeverity::Warning,
            range: DiagnosticRange {
                start_line: 0,
                start_column: 2,
                end_line: 0,
                end_column: 9,
            },
            message: "m".into(),
        };
        let report = diagnostics_json(
            text,
            &DocumentDiagnostics {
                findings: vec![finding],
                gaps: Vec::new(),
            },
        );
        assert_eq!(
            report[0]["range"]["start"],
            json!({"line": 0, "character": 3})
        );
        assert_eq!(
            report[0]["range"]["end"],
            json!({"line": 0, "character": 4}),
            "a column past the line ends at its end"
        );
    }

    #[test]
    fn an_incomplete_report_is_marked_and_never_unchanged() {
        let incomplete = DocumentDiagnostics {
            findings: own_findings(MODULE),
            gaps: vec![
                DiagnosticGap::UnsavedBuffer,
                DiagnosticGap::AnalysisNotReady,
            ],
        };
        let first = document_report_json(MODULE, &incomplete, None);
        assert_eq!(first["kind"], "full");
        let marker = first["items"].as_array().unwrap().last().unwrap();
        assert_eq!(marker["code"], INCOMPLETE_CODE);
        assert_eq!(marker["data"]["complete"], false);
        assert_eq!(
            marker["data"]["gaps"],
            json!(["analysisNotReady", "unsavedBuffer"])
        );
        let again = document_report_json(MODULE, &incomplete, first["resultId"].as_str());
        assert_eq!(again["kind"], "full", "an incomplete result is recomputed");

        let complete = DocumentDiagnostics {
            findings: own_findings(MODULE),
            gaps: Vec::new(),
        };
        let full = document_report_json(MODULE, &complete, None);
        assert_ne!(full["resultId"], first["resultId"]);
        let unchanged = document_report_json(MODULE, &complete, full["resultId"].as_str());
        assert_eq!(
            unchanged,
            json!({"kind": "unchanged", "resultId": full["resultId"]})
        );
    }

    mod provider_words {
        use super::*;
        use std::sync::Arc;
        use unica_address::source_target::TargetKind;
        use unica_diagnostics::model::{
            DiagnosticAction, DiagnosticContext, DiagnosticError, DiagnosticFocusKind,
            DiagnosticProvider, DiagnosticProviderDescriptor, DiagnosticProviderId,
            DiagnosticProviderOutcome, DiagnosticProviderRequest, DiagnosticProviderSection,
            DiagnosticResultState, DiagnosticSelection, EditorFinding,
        };
        use unica_source_contract::location::SourceLocation;
        use unica_work_control::cancellation::CancellationToken;
        use unica_work_control::provider_deadline::ProviderDeadline;

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

        /// #2176: nothing of the provider reaches the editor; a finding is
        /// told in Unica's words, an unlisted one by its severity alone.
        #[test]
        fn provider_findings_reach_the_editor_in_unica_words_only() {
            let registry = DiagnosticProviderRegistry::new(vec![Arc::new(Provider)]).unwrap();
            let text = "abc\ndef\n";
            let diagnostics = provider_diagnostics(
                &result(DiagnosticProviderStatus::Completed, None),
                &registry,
            );
            assert!(diagnostics.complete());
            let wire = diagnostics_json(text, &diagnostics);
            assert_eq!(wire[0]["code"], "bsl.code.known");
            assert_eq!(wire[0]["message"], "Текст Unica");
            assert_eq!(wire[1]["code"], EditorFinding::GENERIC_CODE);
            let rendered = wire.to_string();
            for provider in [
                PROVIDER,
                "ProviderRuleName",
                "UnlistedRule",
                "provider wording",
            ] {
                assert!(!rendered.contains(provider), "{provider} reached the wire");
            }
        }

        /// #2176: a provider that did not finish is a gap, never an empty
        /// complete answer.
        #[test]
        fn an_unfinished_provider_is_a_gap() {
            let registry = DiagnosticProviderRegistry::new(vec![Arc::new(Provider)]).unwrap();
            for (status, error, gap) in [
                (
                    DiagnosticProviderStatus::Failed,
                    Some("provider_not_ready"),
                    DiagnosticGap::AnalysisNotReady,
                ),
                (
                    DiagnosticProviderStatus::Unavailable,
                    Some("bundled_tool_missing"),
                    DiagnosticGap::AnalysisAbsent,
                ),
                (
                    DiagnosticProviderStatus::Failed,
                    Some("provider_timeout"),
                    DiagnosticGap::AnalysisUnavailable,
                ),
            ] {
                let diagnostics = provider_diagnostics(&result(status, error), &registry);
                assert_eq!(diagnostics.gaps, [gap], "{error:?}");
            }
        }
    }

    mod background {
        use super::*;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::mpsc;

        const AT: &str = "main:CommonModule.Общий.Module";
        const WAIT: Duration = Duration::from_secs(10);

        fn key(saved: &[u8]) -> AnalysisKey {
            AnalysisKey::new("ws", AT, saved)
        }

        fn signal() -> (AnalysisWaiter, mpsc::Receiver<()>) {
            let (sent, received) = mpsc::channel();
            let sent = Mutex::new(sent);
            (
                Arc::new(move || {
                    let _ = sent.lock().unwrap().send(());
                }),
                received,
            )
        }

        fn complete() -> DocumentDiagnostics {
            DocumentDiagnostics {
                findings: own_findings(MODULE),
                gaps: Vec::new(),
            }
        }

        fn never() -> AnalysisWaiter {
            Arc::new(|| {})
        }

        /// #2176: a pull never waits for the analysis; the first one starts it
        /// once, the finished analysis calls each waiter back once, and the
        /// next pull has it.
        #[test]
        fn analysis_runs_once_in_the_background_and_calls_back() {
            let analysis = Arc::new(EditorAnalysis::default());
            let runs = Arc::new(AtomicUsize::new(0));
            let (release, wait) = mpsc::channel::<()>();
            let (ready, finished) = signal();
            let counted = Arc::clone(&runs);
            let first = analysis.findings(
                key(b"saved"),
                "editor",
                move |_| {
                    counted.fetch_add(1, Ordering::SeqCst);
                    wait.recv().unwrap();
                    complete()
                },
                Arc::clone(&ready),
            );
            assert!(first.is_none(), "the pull does not wait");
            for _ in 0..3 {
                let again = analysis.findings(
                    key(b"saved"),
                    "editor",
                    |_| unreachable!("started once"),
                    Arc::clone(&ready),
                );
                assert!(again.is_none(), "still running");
            }
            release.send(()).unwrap();
            finished.recv_timeout(WAIT).unwrap();
            let ready = analysis
                .findings(key(b"saved"), "editor", |_| unreachable!("kept"), never())
                .unwrap();
            assert_eq!(ready.findings.len(), 1);
            assert_eq!(runs.load(Ordering::SeqCst), 1);
            assert!(
                finished.recv_timeout(Duration::from_millis(200)).is_err(),
                "one waiter is called back once, however often it asked"
            );
        }

        /// #2176: a newer saved text cancels the older analysis; whoever
        /// waited for the older one is called back with the newer answer, and
        /// the cancelled run never stands for any text.
        #[test]
        fn a_newer_saved_text_cancels_the_older_and_takes_its_waiters() {
            let analysis = Arc::new(EditorAnalysis::default());
            let (started, running) = mpsc::channel::<()>();
            let (cancelled, saw_cancel) = mpsc::channel::<()>();
            let (old_ready, old_called) = signal();
            analysis.findings(
                key(b"old"),
                "first editor",
                move |token| {
                    started.send(()).unwrap();
                    while !token.is_cancelled() {
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    cancelled.send(()).unwrap();
                    complete()
                },
                old_ready,
            );
            running.recv_timeout(WAIT).unwrap();
            let (release, wait) = mpsc::channel::<()>();
            let (new_ready, new_called) = signal();
            assert!(analysis
                .findings(
                    key(b"new"),
                    "second editor",
                    move |_| {
                        wait.recv().unwrap();
                        DocumentDiagnostics::default()
                    },
                    new_ready,
                )
                .is_none());
            saw_cancel.recv_timeout(WAIT).unwrap();
            release.send(()).unwrap();
            new_called.recv_timeout(WAIT).unwrap();
            old_called.recv_timeout(WAIT).unwrap();
            let answer = analysis
                .findings(key(b"new"), "x", |_| unreachable!("kept"), never())
                .unwrap();
            assert!(answer.findings.is_empty(), "the newer text's answer");
            assert!(
                analysis
                    .findings(
                        key(b"old"),
                        "x",
                        |_| DocumentDiagnostics::default(),
                        never()
                    )
                    .is_none(),
                "the cancelled run is no answer for the older text"
            );
        }

        /// #2176: an unfinished analysis calls its waiters back once more
        /// after the retry delay, and the next request runs it again; an
        /// absent analyzer is not retried.
        #[test]
        fn an_unfinished_analysis_is_retried_and_an_absent_one_is_not() {
            let analysis = Arc::new(EditorAnalysis::with_retry_delay(Duration::from_millis(50)));
            let (ready, called) = signal();
            let not_ready = || DocumentDiagnostics {
                findings: Vec::new(),
                gaps: vec![DiagnosticGap::AnalysisNotReady],
            };
            analysis.findings(key(b"saved"), "editor", move |_| not_ready(), ready);
            called.recv_timeout(WAIT).unwrap();
            called.recv_timeout(WAIT).unwrap();
            let runs = Arc::new(AtomicUsize::new(0));
            let counted = Arc::clone(&runs);
            let (again, called_again) = signal();
            assert!(analysis
                .findings(
                    key(b"saved"),
                    "editor",
                    move |_| {
                        counted.fetch_add(1, Ordering::SeqCst);
                        complete()
                    },
                    again,
                )
                .is_none());
            called_again.recv_timeout(WAIT).unwrap();
            assert_eq!(runs.load(Ordering::SeqCst), 1);

            let (absent_ready, absent_called) = signal();
            analysis.findings(
                AnalysisKey::new("ws", "main:CommonModule.Другой.Module", b"x"),
                "editor",
                |_| DocumentDiagnostics {
                    findings: Vec::new(),
                    gaps: vec![DiagnosticGap::AnalysisAbsent],
                },
                absent_ready,
            );
            absent_called.recv_timeout(WAIT).unwrap();
            assert!(absent_called
                .recv_timeout(Duration::from_millis(300))
                .is_err());
        }

        /// #2176: an analysis that panics is an unavailable one and frees
        /// its turn for the next.
        #[test]
        fn a_panicking_analysis_is_unavailable_and_frees_its_turn() {
            let analysis = Arc::new(EditorAnalysis::default());
            for module in 0..MAX_CONCURRENT_EDITOR_ANALYSES + 1 {
                let (ready, called) = signal();
                let at = format!("main:CommonModule.M{module}.Module");
                analysis.findings(
                    AnalysisKey::new("ws", at.as_str(), b"x"),
                    "editor",
                    |_| panic!("analysis failed"),
                    ready,
                );
                called.recv_timeout(WAIT).unwrap();
                let answer = analysis
                    .findings(
                        AnalysisKey::new("ws", at.as_str(), b"x"),
                        "editor",
                        |_| unreachable!("kept"),
                        never(),
                    )
                    .unwrap();
                assert_eq!(answer.gaps, [DiagnosticGap::AnalysisUnavailable]);
            }
        }

        /// #2176: the kept analyses are bounded; the oldest done one goes
        /// first and is analysed again when asked.
        #[test]
        fn kept_analyses_are_bounded_and_the_oldest_goes_first() {
            let analysis = Arc::new(EditorAnalysis::default());
            let at = |module: usize| format!("main:CommonModule.M{module}.Module");
            for module in 0..=MAX_EDITOR_ANALYSIS_ENTRIES {
                let (ready, called) = signal();
                analysis.findings(
                    AnalysisKey::new("ws", at(module).as_str(), b"x"),
                    "editor",
                    |_| complete(),
                    ready,
                );
                called.recv_timeout(WAIT).unwrap();
            }
            assert_eq!(analysis.lock().by_key.len(), MAX_EDITOR_ANALYSIS_ENTRIES);
            assert!(analysis
                .findings(
                    AnalysisKey::new("ws", at(0).as_str(), b"x"),
                    "editor",
                    |_| complete(),
                    never(),
                )
                .is_none());
            assert!(analysis
                .findings(
                    AnalysisKey::new("ws", at(MAX_EDITOR_ANALYSIS_ENTRIES).as_str(), b"x"),
                    "editor",
                    |_| unreachable!("kept"),
                    never(),
                )
                .is_some());
        }
    }

    /// #2176: an answer keeps the first findings by text and names the rest
    /// missing.
    #[test]
    fn an_answer_past_its_limit_keeps_the_first_findings_and_names_the_rest() {
        let finding = |line| DocumentFinding {
            code: "bsl.style.line_length".into(),
            severity: DiagnosticSeverity::Info,
            range: DiagnosticRange {
                start_line: line,
                start_column: 0,
                end_line: line,
                end_column: 1,
            },
            message: "Строка длиннее допустимого".into(),
        };
        let lines = MAX_EDITOR_DIAGNOSTICS + 5;
        let merged = merge(
            Vec::new(),
            DocumentDiagnostics {
                findings: (0..lines).rev().map(finding).collect(),
                gaps: Vec::new(),
            },
        );
        assert_eq!(merged.findings.len(), MAX_EDITOR_DIAGNOSTICS);
        assert_eq!(merged.findings[0].range.start_line, 0);
        assert_eq!(merged.gaps, [DiagnosticGap::Capped]);
        let exact = merge(
            Vec::new(),
            DocumentDiagnostics {
                findings: (0..lines - 5).map(finding).collect(),
                gaps: Vec::new(),
            },
        );
        assert!(exact.complete());
    }

    #[test]
    fn own_and_provider_findings_merge_in_text_order_with_the_provider_gaps() {
        let provider = DocumentDiagnostics {
            findings: vec![DocumentFinding {
                code: "bsl.style.line_length".into(),
                severity: DiagnosticSeverity::Info,
                range: DiagnosticRange {
                    start_line: 0,
                    start_column: 0,
                    end_line: 0,
                    end_column: 1,
                },
                message: "Строка длиннее допустимого".into(),
            }],
            gaps: vec![DiagnosticGap::UnsavedBuffer],
        };
        let merged = merge(own_findings(MODULE), provider);
        assert_eq!(merged.findings[0].code, "bsl.style.line_length");
        assert_eq!(merged.findings[1].code, MISPLACED_VARIABLE_CODE);
        assert_eq!(merged.gaps, [DiagnosticGap::UnsavedBuffer]);
    }

    #[test]
    fn publish_names_the_document_and_its_version() {
        let params = publish_json(
            "unica://ws/main/CommonModule.Общий.Module",
            Some(7),
            MODULE,
            &DocumentDiagnostics::default(),
        );
        assert_eq!(
            params,
            json!({"uri": "unica://ws/main/CommonModule.Общий.Module", "version": 7, "diagnostics": []})
        );
    }
}
