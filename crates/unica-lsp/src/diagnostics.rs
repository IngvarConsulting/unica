//! Находки документа для редактора.
//!
//! Собственные проверки Unica читают текст, который видит соединение, и
//! следуют за буфером без сохранения. Анализ поставщика читает только
//! сохранённый файл: он идёт в фоне по сохранённому тексту, один раз на
//! текст, и запрос ждёт его не дольше своего срока. Пока буфер расходится с
//! файлом, находки поставщика не показываются: они стоят на строках другого
//! текста. Источник, которого нет в ответе, назван находкой-отметкой, так что
//! неполный ответ не выдаётся за полный.
//!
//! Коды и тексты находок — слова Unica: продукт переводит находки поставщика
//! до порта.

use crate::position::LineIndex;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};
use unica_lsp_contract::protocol::{V5LspDiagnostic, V5LspPosition, V5LspRange, V5LspSeverity};
use unica_work_control::cancellation::CancellationToken;

/// Уровень находки.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LspSeverity {
    Error,
    Warning,
    Information,
    Hint,
}

/// Находка в словах Unica. Строки с нуля, колонки — в символах Unicode
/// текста, по которому она найдена.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LspFinding {
    pub code: String,
    pub severity: LspSeverity,
    pub start_line: usize,
    pub start_column: usize,
    pub end_line: usize,
    pub end_column: usize,
    pub message: String,
}

/// Почему в ответе нет части находок. Каждая причина называет источник.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LspDiagnosticGap {
    /// Анализ кода ещё не закончился.
    AnalysisNotReady,
    /// Анализ кода ответил отказом.
    AnalysisUnavailable,
    /// Анализа кода нет на машине, или он не запускается.
    AnalysisAbsent,
    /// Буфер расходится с сохранённым файлом, а анализ кода читает файл.
    UnsavedBuffer,
    /// Находок больше, чем несёт один ответ.
    Capped,
}

impl LspDiagnosticGap {
    fn code(self) -> &'static str {
        match self {
            Self::AnalysisNotReady => "analysis_not_ready",
            Self::AnalysisUnavailable => "analysis_unavailable",
            Self::AnalysisAbsent => "analysis_absent",
            Self::UnsavedBuffer => "unsaved_buffer",
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

/// Находки сохранённого модуля от анализа кода и то, чего в них нет.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LspAnalysis {
    pub findings: Vec<LspFinding>,
    pub gaps: Vec<LspDiagnosticGap>,
}

/// Диагностики допущенной рабочей области. Реализует продукт: у редактора
/// и у `check` одни проверки Unica и один поставщик анализа.
pub trait LspDiagnostics: Send + Sync {
    /// Собственные проверки Unica над текстом.
    fn own_findings(&self, text: &str) -> Vec<LspFinding>;
    /// Анализ кода сохранённого модуля, текст которого `saved`. Отказ
    /// поставщика — пробел, а не ошибка: ответ называет, чего в нём нет.
    /// Файл, изменившийся за время анализа, — тоже пробел: находки стоят
    /// на строках другого текста.
    fn analyse(
        &self,
        source_set: &str,
        module: &str,
        saved: &str,
        cancellation: &CancellationToken,
    ) -> LspAnalysis;
}

/// Код находки-отметки неполного ответа.
pub(crate) const INCOMPLETE_CODE: &str = "unica.diagnostics_incomplete";

/// Больше находок один ответ не несёт; остальные названы пробелом.
const MAX_EDITOR_DIAGNOSTICS: usize = 1000;
/// Сколько сохранённых текстов помнят свой анализ, ждущий или готовый.
/// Первым уходит самый старый готовый, затем самый старый в очереди.
const MAX_EDITOR_ANALYSIS_ENTRIES: usize = 256;
/// Сколько анализов идёт одновременно; остальные ждут очереди, чтобы
/// открытие многих документов не запускало анализатор на каждый и не
/// отнимало машину у вызовов MCP.
const MAX_CONCURRENT_EDITOR_ANALYSES: usize = 2;
/// Сколько неполный анализ отвечает за свой текст, прежде чем его спросят
/// заново.
const EDITOR_ANALYSIS_RETRY_DELAY: Duration = Duration::from_secs(30);
/// Сколько полный анализ отвечает за свой текст. Находки модуля зависят и
/// от других модулей и метаданных, поэтому и полный ответ со временем
/// спрашивают заново.
const EDITOR_ANALYSIS_RETENTION: Duration = Duration::from_secs(60);

/// Ответ о документе: находки текста, который видит соединение, и пробелы.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct DocumentDiagnostics {
    pub(crate) findings: Vec<LspFinding>,
    pub(crate) gaps: Vec<LspDiagnosticGap>,
}

/// Свои находки и находки анализа в порядке текста, с пробелами обоих.
pub(crate) fn merge(own: Vec<LspFinding>, analysis: LspAnalysis) -> DocumentDiagnostics {
    let mut findings = own;
    findings.extend(analysis.findings);
    let mut gaps = analysis.gaps;
    findings.sort_by(|left, right| {
        (left.start_line, left.start_column, &left.code).cmp(&(
            right.start_line,
            right.start_column,
            &right.code,
        ))
    });
    if findings.len() > MAX_EDITOR_DIAGNOSTICS {
        findings.truncate(MAX_EDITOR_DIAGNOSTICS);
        gaps.push(LspDiagnosticGap::Capped);
    }
    gaps.sort();
    gaps.dedup();
    DocumentDiagnostics { findings, gaps }
}

/// Находки как диагностики протокола, колонки в UTF-16 текста `text`.
/// Неполный ответ завершается отметкой в начале документа.
pub(crate) fn wire_diagnostics(
    text: &str,
    diagnostics: &DocumentDiagnostics,
) -> Vec<V5LspDiagnostic> {
    let lines = LineIndex::new(text);
    let point = |line: usize, column: usize| {
        let position = lines.scalar_position(text, line, column);
        V5LspPosition {
            line: position.line,
            character: position.character,
        }
    };
    let mut items = diagnostics
        .findings
        .iter()
        .map(|finding| V5LspDiagnostic {
            range: V5LspRange {
                start: point(finding.start_line, finding.start_column),
                end: point(finding.end_line, finding.end_column),
            },
            severity: match finding.severity {
                LspSeverity::Error => V5LspSeverity::Error,
                LspSeverity::Warning => V5LspSeverity::Warning,
                LspSeverity::Information => V5LspSeverity::Information,
                LspSeverity::Hint => V5LspSeverity::Hint,
            },
            code: finding.code.clone(),
            message: finding.message.clone(),
        })
        .collect::<Vec<_>>();
    if !diagnostics.gaps.is_empty() {
        let start = V5LspPosition {
            line: 0,
            character: 0,
        };
        let named = diagnostics
            .gaps
            .iter()
            .map(|gap| format!("{} ({})", gap.message(), gap.code()))
            .collect::<Vec<_>>()
            .join("; ");
        items.push(V5LspDiagnostic {
            range: V5LspRange { start, end: start },
            severity: V5LspSeverity::Information,
            code: INCOMPLETE_CODE.to_owned(),
            message: format!("Unica: показаны не все находки: {named}"),
        });
    }
    items
}

/// Модуль рабочей области и отпечаток его сохранённого текста.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct AnalysisKey {
    workspace: String,
    module: String,
    saved_digest: [u8; 32],
}

impl AnalysisKey {
    pub(crate) fn new(
        workspace: impl Into<String>,
        module: impl Into<String>,
        saved: &str,
    ) -> Self {
        Self {
            workspace: workspace.into(),
            module: module.into(),
            saved_digest: Sha256::digest(saved.as_bytes()).into(),
        }
    }

    fn same_module(&self, other: &Self) -> bool {
        self.workspace == other.workspace && self.module == other.module
    }
}

/// Зовётся, когда анализ кончился, и ещё раз после задержки повтора, если
/// он кончился неполным.
pub(crate) type AnalysisWaiter = Arc<dyn Fn() + Send + Sync>;
type Job = Box<dyn FnOnce(&CancellationToken) -> LspAnalysis + Send>;

/// Анализы сохранённых текстов, по одному на текст, в фоне. Запрос не
/// ждёт анализ дольше своего срока: он отвечает готовым и запускает
/// недостающее один раз, а кончившийся анализ зовёт всех, кто его ждал.
/// Неполный анализ отвечает за свой текст до задержки повтора, затем его
/// спрашивают заново. Новый сохранённый текст того же модуля отменяет
/// прежний анализ и забирает тех, кто его ждал.
pub(crate) struct EditorAnalysis {
    entries: Mutex<EditorAnalysisEntries>,
    retry_delay: Duration,
    retention: Duration,
}

impl Default for EditorAnalysis {
    fn default() -> Self {
        Self {
            entries: Mutex::default(),
            retry_delay: EDITOR_ANALYSIS_RETRY_DELAY,
            retention: EDITOR_ANALYSIS_RETENTION,
        }
    }
}

#[derive(Default)]
struct EditorAnalysisEntries {
    order: VecDeque<AnalysisKey>,
    by_key: HashMap<AnalysisKey, AnalysisSlot>,
    queue: VecDeque<(AnalysisKey, u64, Job)>,
    running: usize,
    next_generation: u64,
}

enum AnalysisSlot {
    /// В очереди или идёт. Анализ другого поколения того же ключа отменён и
    /// не отвечает за ключ.
    Pending {
        generation: u64,
        waiters: BTreeMap<String, AnalysisWaiter>,
        cancellation: CancellationToken,
    },
    /// Ответ и момент, после которого о тексте спросят заново.
    Done {
        analysis: LspAnalysis,
        retry_after: Instant,
    },
}

impl EditorAnalysis {
    #[cfg(test)]
    pub(crate) fn with_retry_delay(retry_delay: Duration) -> Self {
        Self {
            retry_delay,
            ..Self::default()
        }
    }

    #[cfg(test)]
    pub(crate) fn with_retention(retention: Duration) -> Self {
        Self {
            retention,
            ..Self::default()
        }
    }

    /// Готовый анализ `key` или `None`, пока он ждёт или идёт. Первый вопрос
    /// ставит `analyse` в очередь; `ready` зовётся, когда анализ кончится.
    /// Один ждущий на имя `waiter`: повторный вопрос не зовёт его дважды.
    pub(crate) fn findings(
        self: &Arc<Self>,
        key: AnalysisKey,
        waiter: &str,
        analyse: impl FnOnce(&CancellationToken) -> LspAnalysis + Send + 'static,
        ready: AnalysisWaiter,
    ) -> Option<LspAnalysis> {
        let mut entries = self.lock();
        match entries.by_key.get_mut(&key) {
            Some(AnalysisSlot::Done {
                analysis,
                retry_after,
            }) if Instant::now() < *retry_after => return Some(analysis.clone()),
            Some(AnalysisSlot::Pending { waiters, .. }) => {
                waiters.insert(waiter.to_owned(), ready);
                return None;
            }
            _ => {}
        }
        // О прежнем сохранённом тексте модуля больше не спрашивают: его
        // ждавшие ждут этот.
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
        // Сбой вне поставщика — недоступный анализ, а не занятое навсегда
        // место в очереди.
        let done = catch_unwind(AssertUnwindSafe(|| job(token))).unwrap_or_else(|_| LspAnalysis {
            findings: Vec::new(),
            gaps: vec![LspDiagnosticGap::AnalysisUnavailable],
        });
        // Неполный ответ спрашивают заново через время: анализ может
        // закончиться, а анализатор — появиться, когда его доставит `check`.
        let retry = !done.gaps.is_empty();
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
                            analysis: done,
                            retry_after: Instant::now()
                                + if retry {
                                    self.retry_delay
                                } else {
                                    self.retention
                                },
                        },
                    ) else {
                        unreachable!("the slot was pending");
                    };
                    waiters.into_values().collect::<Vec<_>>()
                }
                // Отменён ради нового сохранённого текста: отвечает тот.
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

    /// Не больше `MAX_EDITOR_ANALYSIS_ENTRIES`: первым уходит самый старый
    /// готовый анализ, затем самый старый в очереди. Его ждущих не зовут:
    /// их документы покажут анализ идущим, пока о них не спросят снова.
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

#[cfg(test)]
#[path = "diagnostics_tests.rs"]
mod tests;
