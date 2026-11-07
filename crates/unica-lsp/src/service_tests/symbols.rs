//! Символы глазами редактора: служба переводит места службы символов в
//! адреса `unica://` своей сессии, не выдаёт неполный ответ за полный и
//! передаёт поставщику буфер, срок и отмену запроса.

use super::*;
use crate::{LspSymbolAnswer, LspSymbolOperation, LspSymbolRequest, LspSymbols};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use unica_code_intelligence::symbols::{
    Incompleteness, SymbolAnswer, SymbolInfo, SymbolKind, SymbolLocation, SymbolPosition,
    SymbolRange, SymbolRequestContext,
};
use unica_lsp_contract::protocol::{V5LspLocation, V5LspMessageLevel, V5LspServerNotification};

/// What the service asked the symbol port.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Asked {
    source_set: String,
    module: String,
    buffer: Option<String>,
    position: SymbolPosition,
    operation: LspSymbolOperation,
}

/// A symbol port that gives the answer a test set, and remembers the asking.
#[derive(Default)]
pub(super) struct ScriptedSymbols {
    answer: Mutex<Option<LspSymbolAnswer>>,
    asked: Mutex<Vec<Asked>>,
    /// Set while the port works: the transport cancelling the request.
    cancel_while_asked: Mutex<Option<unica_work_control::cancellation::CancellationToken>>,
    saw_cancellation: AtomicBool,
    deadlines: Mutex<Vec<unica_work_control::provider_deadline::ProviderDeadline>>,
}

impl ScriptedSymbols {
    fn answer(&self, answer: LspSymbolAnswer) {
        *self.answer.lock().unwrap() = Some(answer);
    }

    fn asked(&self) -> Vec<Asked> {
        self.asked.lock().unwrap().clone()
    }
}

impl LspSymbols for ScriptedSymbols {
    fn ask(
        &self,
        request: LspSymbolRequest<'_>,
        context: &SymbolRequestContext<'_>,
    ) -> Result<LspSymbolAnswer, V5LspFailure> {
        self.asked.lock().unwrap().push(Asked {
            source_set: request.source_set.to_owned(),
            module: request.module.module().to_owned(),
            buffer: request.buffer.map(str::to_owned),
            position: request.position,
            operation: request.operation,
        });
        if let Some(cancellation) = self.cancel_while_asked.lock().unwrap().as_ref() {
            cancellation.cancel();
        }
        self.saw_cancellation
            .store(context.cancellation.is_cancelled(), Ordering::Release);
        self.deadlines.lock().unwrap().push(context.deadline);
        self.answer
            .lock()
            .unwrap()
            .clone()
            .ok_or(V5LspFailure::Unavailable)
    }
}

fn at(line: usize, from: usize, to: usize) -> SymbolRange {
    SymbolRange {
        start: SymbolPosition { line, column: from },
        end: SymbolPosition { line, column: to },
    }
}

fn places(value: Vec<SymbolLocation>, reasons: Vec<Incompleteness>) -> LspSymbolAnswer {
    LspSymbolAnswer::Places(SymbolAnswer {
        complete: reasons.is_empty(),
        value,
        reasons,
    })
}

fn place(at_address: &str, range: Option<SymbolRange>) -> SymbolLocation {
    SymbolLocation {
        at: at_address.to_owned(),
        range,
    }
}

fn ask(host: &UnicaLspHost, session: &V5LspSessionId, call: V5LspCall) -> V5LspOutcome {
    host.call(session, call, &LspCallContext::for_test(soon()))
}

fn definition() -> V5LspCall {
    V5LspCall::Definition {
        uri: BODY_URI.into(),
        position: V5LspPosition {
            line: 0,
            character: 11,
        },
    }
}

/// The service asks about the module whose body the document is, and a
/// place in a module comes back as the document of its body; a metadata node
/// is no document the editor opens.
#[test]
fn symbol_places_reach_the_editor_as_documents_of_the_sessions_space() {
    let workspace = workspace();
    let (host, workspaces) = service();
    let session = open(&host, workspace.path(), "Work");
    let buffer = "Procedure Edited() EndProcedure\n";
    open_buffer(&host, &session, BODY_URI, buffer);
    workspaces.symbols.answer(places(
        vec![
            place("main:CommonModule.Main", Some(at(0, 10, 16))),
            place("main:Catalog.Items", None),
        ],
        Vec::new(),
    ));

    assert_eq!(
        ask(&host, &session, definition()),
        V5LspOutcome::Locations {
            locations: vec![V5LspLocation {
                uri: BODY_URI.into(),
                range: V5LspRange {
                    start: V5LspPosition {
                        line: 0,
                        character: 10
                    },
                    end: V5LspPosition {
                        line: 0,
                        character: 16
                    },
                },
            }],
        }
    );
    assert_eq!(
        workspaces.symbols.asked(),
        vec![Asked {
            source_set: "main".into(),
            module: "main:CommonModule.Main".into(),
            buffer: Some(buffer.into()),
            position: SymbolPosition {
                line: 0,
                column: 11
            },
            operation: LspSymbolOperation::Definition,
        }]
    );
}

/// Places the service names by module address, in modules other than the
/// one asked about, reach the editor as the documents of those modules'
/// bodies: none is dropped for missing the body segment, and none keeps it
/// twice.
#[test]
fn places_in_other_modules_come_back_as_the_documents_of_their_bodies() {
    let workspace = workspace();
    let (host, workspaces) = service();
    let session = open(&host, workspace.path(), "Work");
    let range = Some(at(2, 4, 9));
    workspaces.symbols.answer(places(
        vec![
            place("main:CommonModule.Other", range),
            place("main:Catalog.Items.Module.Object", range),
            place("main:CommonModule.Main", range),
        ],
        Vec::new(),
    ));

    let V5LspOutcome::Locations { locations } = ask(&host, &session, definition()) else {
        panic!("not locations");
    };
    assert_eq!(
        locations
            .iter()
            .map(|location| location.uri.as_str())
            .collect::<Vec<_>>(),
        [
            "unica://work/main/CommonModule.Other.Body",
            "unica://work/main/Catalog.Items.Module.Object.Body",
            BODY_URI,
        ]
    );
    assert_eq!(
        workspaces
            .symbols
            .asked()
            .iter()
            .map(|asked| asked.module.as_str())
            .collect::<Vec<_>>(),
        ["main:CommonModule.Main"]
    );
}

/// rule: INV.LSP.PARTIAL-ANSWERS
#[test]
fn nothing_found_while_a_source_is_not_ready_is_refused_and_a_partial_list_is_announced() {
    let workspace = workspace();
    let (host, workspaces) = service();
    let session = V5LspSessionId::new();
    let (client, inbox) = LspClientChannel::pair();
    host.open(
        &session,
        &[folder(workspace.path(), "Work")],
        client,
        soon(),
    )
    .unwrap();
    let references = V5LspCall::References {
        uri: BODY_URI.into(),
        position: V5LspPosition {
            line: 0,
            character: 11,
        },
    };

    workspaces
        .symbols
        .answer(places(Vec::new(), vec![Incompleteness::NamesNotIndexed]));
    assert_eq!(
        ask(&host, &session, references.clone()),
        V5LspOutcome::Failed {
            failure: V5LspFailure::Incomplete
        }
    );

    // A place in code no document of the session names is left out, and
    // the answer says so.
    workspaces.symbols.answer(places(
        vec![
            place("main:CommonModule.Main", Some(at(0, 10, 14))),
            place("other:CommonModule.Main", Some(at(0, 0, 1))),
        ],
        vec![Incompleteness::NamesNotIndexed],
    ));
    assert!(matches!(
        ask(&host, &session, references),
        V5LspOutcome::Locations { locations } if locations.len() == 1
    ));
    assert_eq!(
        inbox.take_before(soon()).unwrap(),
        vec![V5LspServerNotification::LogMessage {
            level: V5LspMessageLevel::Info,
            message: "unica: the answer is incomplete: names_not_indexed, unaddressable".into(),
        }]
    );
}

#[test]
fn hover_shows_the_signature_and_says_in_its_text_when_it_is_incomplete() {
    let workspace = workspace();
    let (host, workspaces) = service();
    let session = open(&host, workspace.path(), "Work");
    let hover = V5LspCall::Hover {
        uri: BODY_URI.into(),
        position: V5LspPosition {
            line: 0,
            character: 11,
        },
    };

    workspaces
        .symbols
        .answer(LspSymbolAnswer::Info(SymbolAnswer {
            value: Some(SymbolInfo {
                name: "Disk".into(),
                kind: SymbolKind::Method,
                signature: Some("Disk()".into()),
                return_type: Some("Число".into()),
                location: None,
            }),
            complete: false,
            reasons: vec![Incompleteness::Stale],
        }));
    assert_eq!(
        ask(&host, &session, hover.clone()),
        V5LspOutcome::Hover {
            contents: Some(
                "```bsl\nDisk()\n```\n\n→ Число\n\n_unica: the answer is incomplete (stale)_"
                    .into()
            )
        }
    );

    workspaces
        .symbols
        .answer(LspSymbolAnswer::Info(SymbolAnswer {
            value: None,
            complete: true,
            reasons: Vec::new(),
        }));
    assert_eq!(
        ask(&host, &session, hover),
        V5LspOutcome::Hover { contents: None }
    );
}

/// rule: INV.LSP.MCP-PRIORITY
#[test]
fn a_request_cancelled_while_the_provider_works_cancels_it_and_gives_no_answer() {
    let workspace = workspace();
    let (host, workspaces) = service();
    let session = open(&host, workspace.path(), "Work");
    let cancellation = unica_work_control::cancellation::CancellationToken::new();
    *workspaces.symbols.cancel_while_asked.lock().unwrap() = Some(cancellation.clone());
    workspaces.symbols.answer(places(
        vec![place("main:CommonModule.Main", Some(at(0, 10, 14)))],
        Vec::new(),
    ));

    let outcome = host.call(
        &session,
        definition(),
        &LspCallContext::new(cancellation.clone(), soon()),
    );

    assert!(workspaces.symbols.saw_cancellation.load(Ordering::Acquire));
    assert_eq!(outcome, V5LspOutcome::Cancelled {});
}

/// A read the request's cancellation stopped is a cancellation, not a
/// failure of the service.
#[test]
fn a_port_failing_because_the_request_was_cancelled_gives_a_cancellation() {
    let workspace = workspace();
    let (host, workspaces) = service();
    let session = open(&host, workspace.path(), "Work");
    let cancellation = unica_work_control::cancellation::CancellationToken::new();
    *workspaces.symbols.cancel_while_asked.lock().unwrap() = Some(cancellation.clone());

    let outcome = host.call(
        &session,
        definition(),
        &LspCallContext::new(cancellation.clone(), soon()),
    );

    assert_eq!(outcome, V5LspOutcome::Cancelled {});
}

/// rule: INV.RUNTIME.TIMERS-PRESERVE-WORK
#[test]
fn a_symbol_call_gives_the_provider_the_requests_own_deadline() {
    let workspace = workspace();
    let (host, workspaces) = service();
    let session = open(&host, workspace.path(), "Work");
    workspaces.symbols.answer(places(Vec::new(), Vec::new()));
    let deadline = soon();

    host.call(&session, definition(), &LspCallContext::for_test(deadline));

    assert_eq!(
        *workspaces.symbols.deadlines.lock().unwrap(),
        [unica_work_control::provider_deadline::ProviderDeadline::new(deadline)],
        "no new deadline is started for the provider"
    );
}
