use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;

const AT: &str = "main:CommonModule.Общий";
const WAIT: Duration = Duration::from_secs(10);

fn finding(line: usize, code: &str) -> LspFinding {
    LspFinding {
        code: code.into(),
        severity: LspSeverity::Information,
        start_line: line,
        start_column: 0,
        end_line: line,
        end_column: 1,
        message: "Строка длиннее допустимого".into(),
    }
}

fn complete() -> LspAnalysis {
    LspAnalysis {
        findings: vec![finding(0, "bsl.style.line_length")],
        gaps: Vec::new(),
    }
}

fn key(saved: &str) -> AnalysisKey {
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

fn never() -> AnalysisWaiter {
    Arc::new(|| {})
}

#[test]
fn columns_are_counted_in_utf16_units() {
    // `😀` is one scalar value and two UTF-16 units.
    let text = "А😀Б\n";
    let mut finding = finding(0, "X");
    finding.start_column = 2;
    finding.end_column = 9;
    let wire = wire_diagnostics(
        text,
        &DocumentDiagnostics {
            findings: vec![finding],
            gaps: Vec::new(),
        },
    );
    assert_eq!(
        wire[0].range.start,
        V5LspPosition {
            line: 0,
            character: 3
        }
    );
    assert_eq!(
        wire[0].range.end,
        V5LspPosition {
            line: 0,
            character: 4
        },
        "a column past the line ends at its end"
    );
}

/// rule: INV.LSP.PARTIAL-ANSWERS
#[test]
fn an_incomplete_answer_ends_with_a_marker_naming_what_it_lacks() {
    let wire = wire_diagnostics(
        "Процедура А()\n",
        &merge(
            vec![finding(0, "VariableDeclarationPlacement")],
            LspAnalysis {
                findings: Vec::new(),
                gaps: vec![
                    LspDiagnosticGap::UnsavedBuffer,
                    LspDiagnosticGap::AnalysisNotReady,
                ],
            },
        ),
    );
    assert_eq!(wire.len(), 2);
    let marker = wire.last().unwrap();
    assert_eq!(marker.code, INCOMPLETE_CODE);
    assert_eq!(marker.severity, V5LspSeverity::Information);
    assert_eq!(
        marker.message,
        "Unica: показаны не все находки: анализ кода ещё идёт (analysis_not_ready); \
         анализ кода видит только сохранённый файл (unsaved_buffer)"
    );
    let whole = wire_diagnostics("x\n", &merge(Vec::new(), complete()));
    assert!(whole.iter().all(|item| item.code != INCOMPLETE_CODE));
}

#[test]
fn an_answer_past_its_limit_keeps_the_first_findings_and_names_the_rest() {
    let lines = MAX_EDITOR_DIAGNOSTICS + 5;
    let merged = merge(
        Vec::new(),
        LspAnalysis {
            findings: (0..lines)
                .rev()
                .map(|line| finding(line, "bsl.style.line_length"))
                .collect(),
            gaps: Vec::new(),
        },
    );
    assert_eq!(merged.findings.len(), MAX_EDITOR_DIAGNOSTICS);
    assert_eq!(merged.findings[0].start_line, 0);
    assert_eq!(merged.gaps, [LspDiagnosticGap::Capped]);
    let exact = merge(
        Vec::new(),
        LspAnalysis {
            findings: (0..lines - 5)
                .map(|line| finding(line, "bsl.style.line_length"))
                .collect(),
            gaps: Vec::new(),
        },
    );
    assert!(exact.gaps.is_empty());
}

#[test]
fn own_and_analysis_findings_merge_in_text_order_with_the_analysis_gaps() {
    let merged = merge(
        vec![finding(2, "VariableDeclarationPlacement")],
        LspAnalysis {
            findings: vec![finding(0, "bsl.style.line_length")],
            gaps: vec![LspDiagnosticGap::UnsavedBuffer],
        },
    );
    assert_eq!(merged.findings[0].code, "bsl.style.line_length");
    assert_eq!(merged.findings[1].code, "VariableDeclarationPlacement");
    assert_eq!(merged.gaps, [LspDiagnosticGap::UnsavedBuffer]);
}

/// A request does not wait for the analysis; the first starts it once, the
/// finished analysis calls each waiter back once, and the next request has it.
#[test]
fn analysis_runs_once_in_the_background_and_calls_back() {
    let analysis = Arc::new(EditorAnalysis::default());
    let runs = Arc::new(AtomicUsize::new(0));
    let (release, wait) = mpsc::channel::<()>();
    let (ready, finished) = signal();
    let counted = Arc::clone(&runs);
    let first = analysis.findings(
        key("saved"),
        "editor",
        move |_| {
            counted.fetch_add(1, Ordering::SeqCst);
            wait.recv().unwrap();
            complete()
        },
        Arc::clone(&ready),
    );
    assert!(first.is_none(), "the request does not wait");
    for _ in 0..3 {
        let again = analysis.findings(
            key("saved"),
            "editor",
            |_| unreachable!("started once"),
            Arc::clone(&ready),
        );
        assert!(again.is_none(), "still running");
    }
    release.send(()).unwrap();
    finished.recv_timeout(WAIT).unwrap();
    let ready = analysis
        .findings(key("saved"), "editor", |_| unreachable!("kept"), never())
        .unwrap();
    assert_eq!(ready.findings.len(), 1);
    assert_eq!(runs.load(Ordering::SeqCst), 1);
    assert!(
        finished.recv_timeout(Duration::from_millis(200)).is_err(),
        "one waiter is called back once, however often it asked"
    );
}

/// A newer saved text cancels the older analysis; whoever waited for the
/// older one is called back with the newer answer, and the cancelled run
/// never stands for any text.
#[test]
fn a_newer_saved_text_cancels_the_older_and_takes_its_waiters() {
    let analysis = Arc::new(EditorAnalysis::default());
    let (started, running) = mpsc::channel::<()>();
    let (cancelled, saw_cancel) = mpsc::channel::<()>();
    let (old_ready, old_called) = signal();
    analysis.findings(
        key("old"),
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
            key("new"),
            "second editor",
            move |_| {
                wait.recv().unwrap();
                LspAnalysis::default()
            },
            new_ready,
        )
        .is_none());
    saw_cancel.recv_timeout(WAIT).unwrap();
    release.send(()).unwrap();
    new_called.recv_timeout(WAIT).unwrap();
    old_called.recv_timeout(WAIT).unwrap();
    let answer = analysis
        .findings(key("new"), "x", |_| unreachable!("kept"), never())
        .unwrap();
    assert!(answer.findings.is_empty(), "the newer text's answer");
    assert!(
        analysis
            .findings(key("old"), "x", |_| LspAnalysis::default(), never())
            .is_none(),
        "the cancelled run is no answer for the older text"
    );
}

/// An unfinished analysis calls its waiters back once more after the retry
/// delay, and the next request runs it again; so does one whose analyzer is
/// absent, since `check` may deliver it meanwhile.
#[test]
fn an_unfinished_or_absent_analysis_is_retried() {
    let analysis = Arc::new(EditorAnalysis::with_retry_delay(Duration::from_millis(50)));
    let (ready, called) = signal();
    let not_ready = || LspAnalysis {
        findings: Vec::new(),
        gaps: vec![LspDiagnosticGap::AnalysisNotReady],
    };
    analysis.findings(key("saved"), "editor", move |_| not_ready(), ready);
    called.recv_timeout(WAIT).unwrap();
    called.recv_timeout(WAIT).unwrap();
    let runs = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&runs);
    let (again, called_again) = signal();
    assert!(analysis
        .findings(
            key("saved"),
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
        AnalysisKey::new("ws", "main:CommonModule.Другой", "x"),
        "editor",
        |_| LspAnalysis {
            findings: Vec::new(),
            gaps: vec![LspDiagnosticGap::AnalysisAbsent],
        },
        absent_ready,
    );
    absent_called.recv_timeout(WAIT).unwrap();
    absent_called.recv_timeout(WAIT).unwrap();
}

/// A complete analysis answers for its text only for a while: the module's
/// findings depend on other modules too.
#[test]
fn a_complete_analysis_is_asked_again_after_its_retention() {
    let analysis = Arc::new(EditorAnalysis::with_retention(Duration::from_millis(50)));
    let (ready, called) = signal();
    analysis.findings(key("saved"), "editor", |_| complete(), ready);
    called.recv_timeout(WAIT).unwrap();
    assert!(analysis
        .findings(key("saved"), "editor", |_| unreachable!("kept"), never())
        .is_some());
    std::thread::sleep(Duration::from_millis(100));
    assert!(
        analysis
            .findings(key("saved"), "editor", |_| complete(), never())
            .is_none(),
        "past its retention the text is analysed again"
    );
}

/// An analysis that panics is an unavailable one and frees its turn.
#[test]
fn a_panicking_analysis_is_unavailable_and_frees_its_turn() {
    let analysis = Arc::new(EditorAnalysis::default());
    for module in 0..MAX_CONCURRENT_EDITOR_ANALYSES + 1 {
        let (ready, called) = signal();
        let at = format!("main:CommonModule.M{module}");
        analysis.findings(
            AnalysisKey::new("ws", at.as_str(), "x"),
            "editor",
            |_| panic!("analysis failed"),
            ready,
        );
        called.recv_timeout(WAIT).unwrap();
        let answer = analysis
            .findings(
                AnalysisKey::new("ws", at.as_str(), "x"),
                "editor",
                |_| unreachable!("kept"),
                never(),
            )
            .unwrap();
        assert_eq!(answer.gaps, [LspDiagnosticGap::AnalysisUnavailable]);
    }
}

/// The kept analyses are bounded; the oldest done one goes first and is
/// analysed again when asked.
#[test]
fn kept_analyses_are_bounded_and_the_oldest_goes_first() {
    let analysis = Arc::new(EditorAnalysis::default());
    let at = |module: usize| format!("main:CommonModule.M{module}");
    for module in 0..=MAX_EDITOR_ANALYSIS_ENTRIES {
        let (ready, called) = signal();
        analysis.findings(
            AnalysisKey::new("ws", at(module).as_str(), "x"),
            "editor",
            |_| complete(),
            ready,
        );
        called.recv_timeout(WAIT).unwrap();
    }
    assert_eq!(analysis.lock().by_key.len(), MAX_EDITOR_ANALYSIS_ENTRIES);
    assert!(analysis
        .findings(
            AnalysisKey::new("ws", at(0).as_str(), "x"),
            "editor",
            |_| complete(),
            never(),
        )
        .is_none());
    assert!(analysis
        .findings(
            AnalysisKey::new("ws", at(MAX_EDITOR_ANALYSIS_ENTRIES).as_str(), "x"),
            "editor",
            |_| unreachable!("kept"),
            never(),
        )
        .is_some());
}

/// An analysis nobody waits for any more is cancelled and leaves no answer;
/// one somebody still waits for runs on.
#[test]
fn an_analysis_nobody_waits_for_is_cancelled() {
    let analysis = Arc::new(EditorAnalysis::default());
    let (started, running) = mpsc::channel::<()>();
    let (cancelled, saw_cancel) = mpsc::channel::<()>();
    analysis.findings(
        key("saved"),
        "request",
        move |token| {
            started.send(()).unwrap();
            while !token.is_cancelled() {
                std::thread::sleep(Duration::from_millis(5));
            }
            cancelled.send(()).unwrap();
            complete()
        },
        never(),
    );
    running.recv_timeout(WAIT).unwrap();
    analysis.findings(key("saved"), "session", |_| unreachable!(), never());

    analysis.forget("request", None);
    assert!(
        saw_cancel.recv_timeout(Duration::from_millis(200)).is_err(),
        "the session still waits"
    );
    analysis.forget("session", Some(("ws", "main:CommonModule.Другой")));
    assert!(
        saw_cancel.recv_timeout(Duration::from_millis(200)).is_err(),
        "another module is forgotten, not this one"
    );
    analysis.forget("session", Some(("ws", AT)));
    saw_cancel.recv_timeout(WAIT).unwrap();
    assert!(analysis.lock().by_key.is_empty());
}
