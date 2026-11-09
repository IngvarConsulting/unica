//! Дополнение глазами редактора: служба спрашивает о байте текста, который
//! видит сессия, переводит замену в UTF-16, называет неполный список и
//! находит описание метода по документу элемента.

use super::*;
use crate::{LspCompletion, LspCompletionAnswer, LspCompletionRequest};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use unica_code_model::completion::{Completion, CompletionItem, CompletionKind, CompletionMethod};
use unica_lsp_contract::protocol::{V5LspCompletionData, V5LspCompletionItem, V5LspCompletionKind};

/// A port that offers `Сумма` and this module's `Пересчитать`, replacing
/// the name typed before the cursor.
#[derive(Default)]
pub(super) struct ScriptedCompletion {
    platform_loading: AtomicBool,
    asked: Mutex<Vec<(String, String, usize)>>,
    cancel_while_asked: Mutex<Option<unica_work_control::cancellation::CancellationToken>>,
}

impl LspCompletion for ScriptedCompletion {
    fn complete(
        &self,
        request: LspCompletionRequest<'_>,
        _: &unica_work_control::cancellation::CancellationToken,
    ) -> Result<LspCompletionAnswer, V5LspFailure> {
        self.asked.lock().unwrap().push((
            request.module.module().to_owned(),
            request.text.to_owned(),
            request.offset,
        ));
        if let Some(cancellation) = self.cancel_while_asked.lock().unwrap().as_ref() {
            cancellation.cancel();
        }
        let typed = request.text[..request.offset]
            .rfind(|character: char| !character.is_alphanumeric())
            .map_or(0, |at| at + 1);
        Ok(LspCompletionAnswer {
            completion: Some(Completion {
                items: vec![
                    CompletionItem {
                        label: "Сумма".into(),
                        kind: CompletionKind::Variable,
                        rank: 0,
                        detail: Some("Локальная переменная".into()),
                        method: None,
                    },
                    CompletionItem {
                        label: "Пересчитать".into(),
                        kind: CompletionKind::Method,
                        rank: 1,
                        detail: Some("Пересчитать()".into()),
                        method: Some(CompletionMethod {
                            name: "Пересчитать".into(),
                            common_module: None,
                        }),
                    },
                ],
                replaces: typed..request.offset,
            }),
            complete: !self.platform_loading.load(Ordering::Acquire),
        })
    }

    fn describe(
        &self,
        _: &str,
        _: &str,
        module: &crate::ModuleDocument,
        method: &CompletionMethod,
        _: &unica_work_control::cancellation::CancellationToken,
    ) -> Option<String> {
        (module.module() == "main:CommonModule.Main" && method.name == "Пересчитать")
            .then(|| "Пересчитывает строки.".to_owned())
    }
}

fn completion_at(
    host: &UnicaLspHost,
    session: &V5LspSessionId,
    line: u32,
    character: u32,
) -> V5LspOutcome {
    host.call(
        session,
        V5LspCall::Completion {
            uri: BODY_URI.into(),
            position: V5LspPosition { line, character },
        },
        &LspCallContext::for_test(soon()),
    )
}

fn list(outcome: V5LspOutcome) -> (bool, Vec<V5LspCompletionItem>) {
    match outcome {
        V5LspOutcome::CompletionList { incomplete, items } => (incomplete, items),
        other => panic!("not a completion list: {other:?}"),
    }
}

/// The port is asked about the byte of the buffer, and the edit replaces
/// the typed name in UTF-16 units; each item of a method carries its
/// document.
#[test]
fn completion_asks_about_the_buffer_and_replaces_the_typed_name() {
    let workspace = workspace();
    let (host, workspaces) = service();
    let session = open(&host, workspace.path(), "Work");
    // `😀` takes two UTF-16 units and four bytes.
    open_buffer(
        &host,
        &session,
        BODY_URI,
        "Procedure P()\n\t😀 = Су\nEndProcedure\n",
    );

    let (incomplete, items) = list(completion_at(&host, &session, 1, 8));

    assert!(!incomplete);
    let asked = workspaces.completion.asked.lock().unwrap().clone();
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].0, "main:CommonModule.Main");
    assert_eq!(&asked[0].1[..asked[0].2], "Procedure P()\n\t😀 = Су");
    let edit = items[0].text_edit.clone().unwrap();
    assert_eq!(
        (edit.range.start, edit.range.end),
        (
            V5LspPosition {
                line: 1,
                character: 6
            },
            V5LspPosition {
                line: 1,
                character: 8
            }
        )
    );
    assert_eq!(items[0].sort_text, "0сумма");
    assert_eq!(items[1].kind, V5LspCompletionKind::Method);
    assert_eq!(
        items[1].data,
        Some(V5LspCompletionData {
            uri: BODY_URI.into(),
            method: "Пересчитать".into(),
            common_module: None,
        })
    );
}

/// rule: INV.LSP.PARTIAL-ANSWERS
#[test]
fn a_list_without_the_platform_context_is_incomplete() {
    let workspace = workspace();
    let (host, workspaces) = service();
    workspaces
        .completion
        .platform_loading
        .store(true, Ordering::Release);
    let session = open(&host, workspace.path(), "Work");

    let (incomplete, items) = list(completion_at(&host, &session, 0, 0));

    assert!(incomplete, "the editor asks again on its next input");
    assert!(!items.is_empty());
}

/// An item resolves to the description of its method from the document it
/// names; an item of no document of this session comes back as it was.
#[test]
fn an_item_resolves_from_the_document_it_names() {
    let workspace = workspace();
    let (host, _workspaces) = service();
    let session = open(&host, workspace.path(), "Work");
    let (_, items) = list(completion_at(&host, &session, 0, 0));
    let resolve = |item: V5LspCompletionItem| match host.call(
        &session,
        V5LspCall::CompletionResolve {
            item: Box::new(item),
        },
        &LspCallContext::for_test(soon()),
    ) {
        V5LspOutcome::CompletionItem { item } => *item,
        other => panic!("not an item: {other:?}"),
    };

    let resolved = resolve(items[1].clone());
    assert_eq!(
        resolved.documentation.as_deref(),
        Some("Пересчитывает строки.")
    );
    let mut foreign = items[1].clone();
    foreign.data.as_mut().unwrap().uri = "unica://other/main/CommonModule.Main.Body".into();
    assert_eq!(resolve(foreign.clone()), foreign);
}

/// rule: INV.LSP.MCP-PRIORITY
#[test]
fn a_cancelled_completion_gives_no_list() {
    let workspace = workspace();
    let (host, workspaces) = service();
    let session = open(&host, workspace.path(), "Work");
    let cancellation = unica_work_control::cancellation::CancellationToken::new();
    *workspaces.completion.cancel_while_asked.lock().unwrap() = Some(cancellation.clone());

    let outcome = host.call(
        &session,
        V5LspCall::Completion {
            uri: BODY_URI.into(),
            position: V5LspPosition {
                line: 0,
                character: 0,
            },
        },
        &LspCallContext::new(cancellation, soon()),
    );

    assert_eq!(outcome, V5LspOutcome::Cancelled {});
}
