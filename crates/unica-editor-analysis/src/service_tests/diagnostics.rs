//! Находки документа глазами редактора: свои проверки Unica идут по буферу,
//! анализ кода — по сохранённому файлу и в фоне, а ответ без него говорит,
//! чего в нём нет.

use super::*;
use crate::{LspAnalysis, LspAnalysisProgress, LspDiagnostics, LspFinding, LspSeverity};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Mutex};
use unica_lsp_contract::protocol::{
    V5LspDiagnostic, V5LspIncompleteReason, V5LspProgressPhase, V5LspServerNotification,
    V5LspSeverity,
};
use unica_work_control::cancellation::CancellationToken;

/// A port whose own check flags every line that says `Bad`, and whose
/// analysis answers what a test set, after a gate the test may hold.
#[derive(Default)]
pub(super) struct ScriptedDiagnostics {
    analysed: Mutex<Vec<String>>,
    runs: AtomicUsize,
    gate: Mutex<Option<mpsc::Receiver<()>>>,
    answer: Mutex<LspAnalysis>,
    /// When set, a preparation begins and ends only once released.
    preparation: Mutex<Option<mpsc::Receiver<()>>>,
}

impl ScriptedDiagnostics {
    fn hold(&self) -> mpsc::Sender<()> {
        let (release, wait) = mpsc::channel();
        *self.gate.lock().unwrap() = Some(wait);
        release
    }

    fn hold_preparation(&self) -> mpsc::Sender<()> {
        let (release, wait) = mpsc::channel();
        *self.preparation.lock().unwrap() = Some(wait);
        release
    }
}

fn line_finding(line: usize, code: &str, severity: LspSeverity) -> LspFinding {
    LspFinding {
        code: code.into(),
        severity,
        start_line: line,
        start_column: 0,
        end_line: line,
        end_column: 3,
        message: format!("{code} message"),
    }
}

impl LspDiagnostics for ScriptedDiagnostics {
    fn own_findings(&self, text: &str) -> Vec<LspFinding> {
        text.lines()
            .enumerate()
            .filter(|(_, line)| line.contains("Bad"))
            .map(|(line, _)| line_finding(line, "unica.own", LspSeverity::Error))
            .collect()
    }

    fn analyse(
        &self,
        _: &str,
        module: &crate::ModuleDocument,
        _: &str,
        _: &CancellationToken,
    ) -> LspAnalysis {
        self.runs.fetch_add(1, Ordering::SeqCst);
        self.analysed
            .lock()
            .unwrap()
            .push(module.module().to_owned());
        let gate = self.gate.lock().unwrap().take();
        if let Some(gate) = gate {
            let _ = gate.recv();
        }
        self.answer.lock().unwrap().clone()
    }

    fn prepare(
        &self,
        _: &str,
        progress: &dyn Fn(LspAnalysisProgress),
        _: &CancellationToken,
    ) {
        let Some(gate) = self.preparation.lock().unwrap().take() else {
            return;
        };
        progress(LspAnalysisProgress::Begin {
            title: "Анализатор готовит базу".into(),
            message: "Запуск анализатора".into(),
        });
        let _ = gate.recv();
        progress(LspAnalysisProgress::End {
            message: "База готова: 1 модуль".into(),
        });
    }
}

fn diagnostic(
    host: &UnicaLspHost,
    session: &V5LspSessionId,
    context: &LspCallContext,
) -> V5LspOutcome {
    host.call(
        session,
        V5LspCall::Diagnostic {
            uri: BODY_URI.into(),
        },
        context,
    )
}

fn items(outcome: V5LspOutcome) -> Vec<V5LspDiagnostic> {
    match outcome {
        V5LspOutcome::Diagnostics { items } => items,
        other => panic!("not a diagnostic report: {other:?}"),
    }
}

fn codes(items: &[V5LspDiagnostic]) -> Vec<&str> {
    items.iter().map(|item| item.code.as_str()).collect()
}

fn analysis_finding() -> LspAnalysis {
    LspAnalysis {
        findings: vec![line_finding(
            0,
            "bsl.style.line_length",
            LspSeverity::Information,
        )],
        gaps: Vec::new(),
    }
}

/// The saved file and an unchanged buffer: Unica's own findings and the
/// analysis of that file, about the module whose body the document is.
#[test]
fn a_saved_document_gets_its_own_and_its_analysis_findings() {
    let workspace = tempfile::tempdir().unwrap();
    write_module(
        workspace.path(),
        "src",
        "Main",
        "Procedure Disk() EndProcedure\nBad\n",
    );
    let (host, workspaces) = service();
    *workspaces.diagnostics.answer.lock().unwrap() = analysis_finding();
    let session = open(&host, workspace.path(), "Work");

    let report = items(diagnostic(
        &host,
        &session,
        &LspCallContext::for_test(soon()),
    ));

    assert_eq!(codes(&report), ["bsl.style.line_length", "unica.own"]);
    assert_eq!(report[1].severity, V5LspSeverity::Error);
    assert_eq!(report[1].range.start.line, 1);
    assert_eq!(
        *workspaces.diagnostics.analysed.lock().unwrap(),
        ["main:CommonModule.Main"]
    );
    // The analysis of this saved text is kept: asking again does not rerun it.
    items(diagnostic(
        &host,
        &session,
        &LspCallContext::for_test(soon()),
    ));
    assert_eq!(workspaces.diagnostics.runs.load(Ordering::SeqCst), 1);
}

/// rule: INV.LSP.PARTIAL-ANSWERS
#[test]
fn an_unsaved_buffer_gets_its_own_findings_and_says_the_analysis_is_of_the_file() {
    let workspace = workspace();
    let (host, workspaces) = service();
    *workspaces.diagnostics.answer.lock().unwrap() = analysis_finding();
    let session = open(&host, workspace.path(), "Work");
    open_buffer(
        &host,
        &session,
        BODY_URI,
        "Procedure Edited() EndProcedure\nBad\n",
    );

    let report = items(diagnostic(
        &host,
        &session,
        &LspCallContext::for_test(soon()),
    ));

    assert_eq!(
        codes(&report),
        ["unica.own", "unica.diagnostics_incomplete"]
    );
    assert!(report[1].message.contains("unsaved_buffer"), "{report:?}");
    assert_eq!(workspaces.diagnostics.runs.load(Ordering::SeqCst), 0);
}

/// rule: INV.LSP.PARTIAL-ANSWERS
#[test]
fn an_analysis_slower_than_the_request_is_named_and_the_next_request_has_it() {
    let workspace = workspace();
    let (host, workspaces) = service();
    *workspaces.diagnostics.answer.lock().unwrap() = analysis_finding();
    let release = workspaces.diagnostics.hold();
    let session = open(&host, workspace.path(), "Work");

    let early = items(diagnostic(
        &host,
        &session,
        &LspCallContext::for_test(Instant::now() + Duration::from_millis(100)),
    ));
    assert_eq!(codes(&early), ["unica.diagnostics_incomplete"]);
    assert!(early[0].message.contains("analysis_not_ready"), "{early:?}");

    release.send(()).unwrap();
    let late = items(diagnostic(
        &host,
        &session,
        &LspCallContext::for_test(soon()),
    ));
    assert_eq!(codes(&late), ["bsl.style.line_length"]);
    assert_eq!(workspaces.diagnostics.runs.load(Ordering::SeqCst), 1);
}

/// rule: INV.LSP.MCP-PRIORITY
#[test]
fn a_request_cancelled_while_it_waits_for_the_analysis_gives_no_report() {
    let workspace = workspace();
    let (host, workspaces) = service();
    let release = workspaces.diagnostics.hold();
    let session = open(&host, workspace.path(), "Work");
    let cancellation = unica_work_control::cancellation::CancellationToken::new();
    let canceller = {
        let cancellation = cancellation.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            cancellation.cancel();
        })
    };

    let outcome = diagnostic(&host, &session, &LspCallContext::new(cancellation, soon()));

    canceller.join().unwrap();
    release.send(()).unwrap();
    assert_eq!(outcome, V5LspOutcome::Cancelled {});
}

/// rule: INV.LSP.PARTIAL-ANSWERS
#[test]
fn the_session_is_told_when_the_analysis_it_lacked_is_ready() {
    let workspace = workspace();
    let (host, workspaces) = service();
    *workspaces.diagnostics.answer.lock().unwrap() = analysis_finding();
    let release = workspaces.diagnostics.hold();
    let session = V5LspSessionId::new();
    let (client, inbox) = LspClientChannel::pair(false);
    host.open(
        &session,
        &[folder(workspace.path(), "Work")],
        client,
        soon(),
    )
    .unwrap();

    let early = items(diagnostic(
        &host,
        &session,
        &LspCallContext::for_test(Instant::now() + Duration::from_millis(100)),
    ));
    assert_eq!(codes(&early), ["unica.diagnostics_incomplete"]);
    assert!(inbox
        .take_before(Instant::now() + Duration::from_millis(50))
        .unwrap()
        .is_empty());

    release.send(()).unwrap();
    assert_eq!(
        inbox.take_before(soon()).unwrap(),
        [unica_lsp_contract::protocol::V5LspServerNotification::DiagnosticsRefresh {}]
    );
}

/// rule: INV.LSP.PARTIAL-ANSWERS
#[test]
fn an_editor_that_hears_the_refresh_is_answered_without_waiting_for_the_analysis() {
    let workspace = workspace();
    let (host, workspaces) = service();
    *workspaces.diagnostics.answer.lock().unwrap() = analysis_finding();
    let release = workspaces.diagnostics.hold();
    let session = V5LspSessionId::new();
    let (client, inbox) = LspClientChannel::pair(true);
    host.open(
        &session,
        &[folder(workspace.path(), "Work")],
        client,
        soon(),
    )
    .unwrap();

    // The deadline is far: the answer still goes before the analysis ends.
    let asked = Instant::now();
    let early = items(diagnostic(
        &host,
        &session,
        &LspCallContext::for_test(Instant::now() + Duration::from_secs(60)),
    ));
    assert!(
        asked.elapsed() < Duration::from_secs(5),
        "{:?}",
        asked.elapsed()
    );
    assert_eq!(codes(&early), ["unica.diagnostics_incomplete"]);
    assert!(early[0].message.contains("analysis_not_ready"), "{early:?}");

    release.send(()).unwrap();
    assert_eq!(
        inbox.take_before(soon()).unwrap(),
        [unica_lsp_contract::protocol::V5LspServerNotification::DiagnosticsRefresh {}]
    );
    let late = items(diagnostic(
        &host,
        &session,
        &LspCallContext::for_test(soon()),
    ));
    assert_eq!(codes(&late), ["bsl.style.line_length"]);
    assert_eq!(workspaces.diagnostics.runs.load(Ordering::SeqCst), 1);
}

/// An analysis that answers at once, as a ready resident analyzer does, is
/// whole in the first answer even to an editor that hears the refresh: it
/// is not told "not ready" only to ask again (#2790).
///
/// rule: INV.LSP.PARTIAL-ANSWERS
#[test]
fn an_editor_that_hears_the_refresh_gets_a_quick_analysis_in_the_first_answer() {
    let workspace = workspace();
    let (host, workspaces) = service();
    *workspaces.diagnostics.answer.lock().unwrap() = analysis_finding();
    let session = V5LspSessionId::new();
    let (client, inbox) = LspClientChannel::pair(true);
    host.open(
        &session,
        &[folder(workspace.path(), "Work")],
        client,
        soon(),
    )
    .unwrap();

    let first = items(diagnostic(
        &host,
        &session,
        &LspCallContext::for_test(Instant::now() + Duration::from_secs(60)),
    ));
    assert_eq!(codes(&first), ["bsl.style.line_length"]);
    // The session still hears that the analysis ended; asking again is
    // answered from the same analysis.
    drop(inbox);
    assert_eq!(workspaces.diagnostics.runs.load(Ordering::SeqCst), 1);
}

/// The editor hears the preparation of an opened space begin and end, and
/// an answer that lacks the analysis names why by code: the database still
/// building is `warming`, an analysis still going after it is `analysing`.
///
/// rule: INV.LSP.PARTIAL-ANSWERS
#[test]
fn the_preparation_is_heard_and_an_incomplete_answer_names_its_reason() {
    let workspace = workspace();
    let (host, workspaces) = service();
    let prepared = workspaces.diagnostics.hold_preparation();
    let _analysed = workspaces.diagnostics.hold();
    let session = V5LspSessionId::new();
    let (client, inbox) = LspClientChannel::pair(false);
    host.open(
        &session,
        &[folder(workspace.path(), "Work")],
        client,
        soon(),
    )
    .unwrap();
    let began = inbox.take_before(soon()).unwrap();
    assert_eq!(
        began,
        [V5LspServerNotification::AnalysisProgress {
            token: "unica-analysis-work-main".into(),
            phase: V5LspProgressPhase::Begin,
            title: Some("Анализатор готовит базу".into()),
            message: "Запуск анализатора".into(),
        }]
    );

    let reasons = |context: LspCallContext| {
        items(diagnostic(&host, &session, &context))
            .into_iter()
            .find(|item| item.code == "unica.diagnostics_incomplete")
            .map(|item| item.reasons)
            .unwrap()
    };
    let near = || LspCallContext::for_test(Instant::now() + Duration::from_millis(300));
    assert_eq!(reasons(near()), [V5LspIncompleteReason::Warming]);

    prepared.send(()).unwrap();
    let ended = inbox.take_before(soon()).unwrap();
    assert!(
        ended.contains(&V5LspServerNotification::AnalysisProgress {
            token: "unica-analysis-work-main".into(),
            phase: V5LspProgressPhase::End,
            title: None,
            message: "База готова: 1 модуль".into(),
        }),
        "{ended:?}"
    );
    assert_eq!(reasons(near()), [V5LspIncompleteReason::Analysing]);
}

/// rule: INV.LSP.MCP-PRIORITY
#[test]
fn closing_the_document_cancels_an_analysis_nobody_else_waits_for() {
    let workspace = workspace();
    let (host, workspaces) = service();
    let _release = workspaces.diagnostics.hold();
    let session = open(&host, workspace.path(), "Work");
    open_buffer(&host, &session, BODY_URI, DISK);

    items(diagnostic(
        &host,
        &session,
        &LspCallContext::for_test(Instant::now() + Duration::from_millis(100)),
    ));
    assert_eq!(host.analysis.lock_len_for_test(), 1);
    host.notify(
        &session,
        V5LspNotification::DidClose {
            uri: BODY_URI.into(),
        },
        soon(),
    )
    .unwrap();
    assert_eq!(host.analysis.lock_len_for_test(), 0);
}
