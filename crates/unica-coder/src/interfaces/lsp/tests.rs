//! Дверь LSP над живым демоном в потоке: жизненный цикл, отмена,
//! приоритет MCP и уход редактора посреди запроса.
use super::*;
use crate::infrastructure::daemon::protocol::InvocationRequest;
use crate::infrastructure::daemon::server::actor_capacity_tests::{
    canonical_v13_service, LiveV5Daemon, V5Submission,
};
use std::collections::VecDeque;
use std::io::{BufReader, Cursor, Read};
use std::path::Path;
use std::sync::atomic::AtomicUsize;
use std::sync::{mpsc, Condvar};
use unica_daemon::lsp_lane::{LspCallContext, LspClientChannel, LspHost, MAX_LSP_LANE_WORKERS};
use unica_daemon::protocol_v5::lsp::{V5LspCapabilities, V5LspWorkspaceSpace};
use unica_execution::invocation_store::ToolIdentity;

/// Допуск задержки MCP под потоком hover: медиана вызова MCP при занятой
/// полосе LSP не больше полутора медиан без LSP плюс `MCP_LATENCY_TOLERANCE`.
/// Доля следует за скоростью машины: на загруженной машине медленнее и
/// вызов без LSP. Hover стоит `HOVER_COST`, больше допуска: вызов MCP,
/// вставший в очередь хотя бы за одним hover, его превысит.
const MCP_LATENCY_TOLERANCE: Duration = Duration::from_millis(100);
const HOVER_COST: Duration = Duration::from_millis(300);
const MCP_SAMPLES: usize = 9;
const WAIT: Duration = Duration::from_secs(10);

#[test]
fn lsp_frames_round_trip_and_broken_frames_are_refused() {
    let mut bytes = Vec::new();
    codec::write_message(&mut bytes, &json!({"jsonrpc": "2.0", "method": "exit"})).unwrap();
    codec::write_message(&mut bytes, &json!({"id": 1})).unwrap();
    let mut reader = BufReader::new(Cursor::new(bytes));
    assert_eq!(
        codec::read_message(&mut reader).unwrap().unwrap(),
        br#"{"jsonrpc":"2.0","method":"exit"}"#
    );
    assert_eq!(
        codec::read_message(&mut reader).unwrap().unwrap(),
        br#"{"id":1}"#
    );
    assert!(codec::read_message(&mut reader).unwrap().is_none());

    for broken in [
        b"Content-Type: x\r\n\r\n{}".to_vec(),
        b"Content-Length: 2\n\n{}".to_vec(),
        b"Content-Length: 10\r\n\r\n{}".to_vec(),
        b"Content-Length: 2\r\nContent-Length: 2\r\n\r\n{}".to_vec(),
        format!(
            "Content-Length: {}\r\n\r\n",
            codec::MAX_LSP_MESSAGE_BYTES + 1
        )
        .into_bytes(),
        b"Content-Length: 2\r\n".to_vec(),
    ] {
        let mut reader = BufReader::new(Cursor::new(broken.clone()));
        assert!(
            codec::read_message(&mut reader).is_err(),
            "{}",
            String::from_utf8_lossy(&broken)
        );
    }
}

struct ReleaseOnDrop(Option<mpsc::Sender<()>>);

struct StopOnDrop(Arc<AtomicBool>);

impl Drop for StopOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

impl ReleaseOnDrop {
    fn release(&mut self) {
        if let Some(sender) = self.0.take() {
            let _ = sender.send(());
        }
    }
}

impl Drop for ReleaseOnDrop {
    fn drop(&mut self) {
        self.release();
    }
}

fn output_only_editor() -> (Editor, SharedWriter) {
    let output = Arc::new((Mutex::new(EditorOutput::default()), Condvar::new()));
    let writer = SharedWriter::new(Arc::clone(&output));
    (
        Editor {
            input: None,
            output,
            received: VecDeque::new(),
            next_id: 0,
            thread: None,
            lost: Arc::new(AtomicBool::new(false)),
        },
        writer,
    )
}

#[test]
fn editor_output_waits_for_a_complete_fragmented_frame() {
    let expected = json!({"jsonrpc": "2.0", "id": 7, "result": "готово"});
    let mut frame = Vec::new();
    codec::write_message(&mut frame, &expected).unwrap();
    let split = b"Content-Len".len();
    assert!(frame.starts_with(b"Content-Len"));
    let (mut editor, mut writer) = output_only_editor();
    let (prefix_written, prefix_ready) = mpsc::channel();
    let (release, released) = mpsc::channel();
    thread::scope(|scope| {
        let writing = scope.spawn(move || {
            writer.write_all(&frame[..split]).unwrap();
            prefix_written.send(()).unwrap();
            released.recv_timeout(WAIT).unwrap();
            for chunk in frame[split..].chunks(3) {
                writer.write_all(chunk).unwrap();
            }
            writer.flush().unwrap();
        });
        // Release the writer before the scope joins, including on assertion unwind.
        let mut release = ReleaseOnDrop(Some(release));
        prefix_ready.recv_timeout(WAIT).unwrap();
        editor.receive(Instant::now());
        assert!(editor.received.is_empty());
        release.release();
        writing.join().unwrap();
    });
    editor.receive(Instant::now());
    assert_eq!(editor.received.pop_front(), Some(expected));
    assert!(editor.received.is_empty());
}

#[test]
fn editor_output_reads_complete_and_coalesced_frames_then_terminal_eof() {
    let first = json!({"id": 1, "result": "первый"});
    let second = json!({"id": 2, "result": "second"});
    for messages in [vec![first.clone()], vec![first.clone(), second]] {
        let (mut editor, mut writer) = output_only_editor();
        let mut frames = Vec::new();
        for message in &messages {
            codec::write_message(&mut frames, message).unwrap();
        }
        writer.write_all(&frames).unwrap();
        writer.flush().unwrap();
        editor.receive_result(Instant::now()).unwrap();
        assert_eq!(editor.received.drain(..).collect::<Vec<_>>(), messages);
        drop(writer);
        assert!(editor.output.0.lock().unwrap().closed);
        assert!(!editor.receive_result(Instant::now() + WAIT).unwrap());
        assert!(editor.received.is_empty());
    }
}

#[test]
fn editor_output_releases_and_joins_the_writer_when_a_reader_assertion_unwinds() {
    let expected = json!({"id": 9, "result": "after unwind"});
    let mut frame = Vec::new();
    codec::write_message(&mut frame, &expected).unwrap();
    let (mut editor, mut writer) = output_only_editor();
    let (prefix_written, prefix_ready) = mpsc::channel();
    let (release, released) = mpsc::channel();
    let completed = Arc::new(AtomicBool::new(false));
    let written = Arc::clone(&completed);
    let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        thread::scope(|scope| {
            scope.spawn(move || {
                writer.write_all(&frame[..11]).unwrap();
                prefix_written.send(()).unwrap();
                released.recv_timeout(WAIT).unwrap();
                writer.write_all(&frame[11..]).unwrap();
                writer.flush().unwrap();
                written.store(true, Ordering::Release);
            });
            let _release = ReleaseOnDrop(Some(release));
            prefix_ready.recv_timeout(WAIT).unwrap();
            panic!("injected reader assertion with the writer waiting for release");
        });
    }));
    assert!(unwound.is_err());
    assert!(completed.load(Ordering::Acquire));
    assert!(editor.output.0.lock().unwrap().closed);
    editor.receive(Instant::now());
    assert_eq!(editor.received.pop_front(), Some(expected));
}

#[test]
fn editor_output_reports_truncated_published_frames_without_poisoning_writer() {
    for (prefix, expected_error) in [
        (b"Content-Len".as_slice(), io::ErrorKind::InvalidData),
        (
            b"Content-Length: 2\r\n\r\n{".as_slice(),
            io::ErrorKind::UnexpectedEof,
        ),
    ] {
        for terminal_eof in [false, true] {
            let (mut editor, mut writer) = output_only_editor();
            writer.write_all(prefix).unwrap();
            assert!(editor.receive_result(Instant::now()).unwrap());
            assert!(editor.received.is_empty());
            if terminal_eof {
                drop(writer);
            } else {
                // An explicit flush of an incomplete frame violates the producer contract.
                writer.flush().unwrap();
            }
            let error = editor.receive_result(Instant::now()).unwrap_err();
            assert_eq!(error.kind(), expected_error);
            assert!(editor.received.is_empty());
            assert!(!editor.output.0.is_poisoned());
        }
    }
}

#[test]
fn lsp_lifecycle_declares_only_ready_capabilities_and_ends_with_exit() {
    let daemon = LiveV5Daemon::start(canonical_v13_service());
    let mut editor = Editor::start(&daemon);

    let early = editor.request("textDocument/hover", hover_params("file:///a.bsl"));
    assert_eq!(error_code(&early), SERVER_NOT_INITIALIZED);
    let initialized = editor.initialize();
    let capabilities = &initialized["result"]["capabilities"];
    assert_eq!(capabilities["positionEncoding"], "utf-16");
    for provider in [
        "hoverProvider",
        "definitionProvider",
        "referencesProvider",
        "callHierarchyProvider",
    ] {
        assert_eq!(capabilities[provider], true, "{provider}");
    }
    assert_eq!(
        capabilities["experimental"]["unica"]["workspaces"],
        json!([{"folder": "file:///work", "failure": "lsp_workspace_unavailable"}]),
        "a folder that is no workspace gets no space"
    );
    assert_eq!(initialized["result"]["serverInfo"]["name"], "unica");
    assert_eq!(
        error_code(&editor.request("initialize", json!({}))),
        INVALID_REQUEST
    );
    // A file outside every space names no document of this session.
    let hover = editor.request("textDocument/hover", hover_params("file:///a.bsl"));
    assert_eq!(
        hover["error"]["message"], "unica: lsp_document_unknown",
        "{hover}"
    );
    assert_eq!(
        error_code(&editor.request("textDocument/definition", json!({}))),
        INVALID_PARAMS
    );
    assert_eq!(
        error_code(&editor.request("textDocument/rename", json!({}))),
        METHOD_NOT_FOUND
    );

    assert_eq!(
        editor.request("shutdown", Value::Null)["result"],
        Value::Null
    );
    let late = editor.request("textDocument/hover", hover_params("file:///a.bsl"));
    assert_eq!(error_code(&late), INVALID_REQUEST);
    editor.notify("exit", Value::Null);
    assert_eq!(editor.finish(), 0);
}

#[test]
fn an_editor_that_leaves_without_shutdown_exits_with_failure() {
    let daemon = LiveV5Daemon::start(canonical_v13_service());
    let mut editor = Editor::start(&daemon);
    editor.initialize();
    editor.notify("exit", Value::Null);
    assert_eq!(editor.finish(), 1);
}

#[test]
fn capabilities_follow_what_the_lsp_service_declares() {
    let host = Arc::new(HeldHost::default());
    let daemon = LiveV5Daemon::start_with_lsp_host(
        canonical_v13_service(),
        Arc::clone(&host) as Arc<dyn LspHost>,
    );
    let mut editor = Editor::start(&daemon);
    let initialized = editor.initialize();
    assert_eq!(
        initialized["result"]["capabilities"],
        json!({
            "positionEncoding": "utf-16",
            "textDocumentSync": {"openClose": true, "change": 2},
            "hoverProvider": true,
            "workspace": {"textDocumentContent": {"schemes": ["unica"]}},
            "experimental": {"unica": {"workspaces": [{"folder": "file:///work", "space": "w0"}]}},
        })
    );
    assert_eq!(
        *host.folders.lock().unwrap(),
        vec![V5LspWorkspaceFolder {
            uri: "file:///work".into(),
            name: "work".into(),
        }]
    );
    host.push(V5LspServerNotification::LogMessage {
        level: V5LspMessageLevel::Info,
        message: "index ready".into(),
    });
    let notification = editor.next_notification("window/logMessage");
    assert_eq!(
        notification["params"],
        json!({"type": 3, "message": "index ready"})
    );
    host.release();
    let hover = editor.request("textDocument/hover", hover_params("file:///work/a.bsl"));
    assert_eq!(
        hover["result"],
        json!({"contents": {"kind": "markdown", "value": "answer"}})
    );
}

/// #2176: a service with diagnostics offers them by request and asks to
/// hear of saves; the editor's requests and saves reach it in its terms.
#[test]
fn diagnostics_are_offered_and_their_requests_reach_the_service() {
    let capabilities = capabilities_json(V5LspOpening {
        capabilities: V5LspCapabilities {
            text_document_sync: V5LspTextSync::Incremental,
            diagnostics: true,
            ..V5LspCapabilities::NONE
        },
        workspaces: Vec::new(),
    });
    assert_eq!(
        capabilities["diagnosticProvider"],
        json!({"interFileDependencies": false, "workspaceDiagnostics": false})
    );
    assert_eq!(capabilities["textDocumentSync"]["save"], true);
    let uri = "file:///work/src/CommonModules/Main/Ext/Module.bsl";
    assert_eq!(
        document_call(
            "textDocument/diagnostic",
            &json!({"textDocument": {"uri": uri}, "previousResultId": "r1"}),
        ),
        Some(Some(V5LspCall::Diagnostic {
            uri: uri.into(),
            previous_result_id: Some("r1".into()),
        }))
    );
    assert_eq!(
        document_notification(
            "textDocument/didSave",
            &json!({"textDocument": {"uri": uri}})
        ),
        Some(V5LspNotification::DidSave { uri: uri.into() })
    );
}

/// #2176: an editor that asks for diagnostics itself is not sent them as
/// well; it is asked once to ask again when analyses finish, and only if it
/// can be. An editor that does not ask gets every publication.
#[test]
fn diagnostics_reach_an_editor_the_way_it_takes_them() {
    let publish = |analysed| V5LspServerNotification::PublishDiagnostics {
        uri: "file:///work/Module.bsl".into(),
        params: json!({"uri": "file:///work/Module.bsl", "diagnostics": []}),
        analysed,
    };
    let batch = || vec![publish(false), publish(true), publish(true)];
    let methods = |messages: Vec<Value>| {
        messages
            .iter()
            .map(|message| message["method"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>()
    };
    let pull = DiagnosticDelivery::of_client(&json!({"capabilities": {
        "textDocument": {"diagnostic": {}},
        "workspace": {"diagnostics": {"refreshSupport": true}},
    }}));
    assert_eq!(pull, DiagnosticDelivery::Pull { refresh: true });
    let mut refreshes = 0;
    let refreshed = server_messages(batch(), pull, &mut refreshes);
    assert_eq!(methods(refreshed.clone()), ["workspace/diagnostic/refresh"]);
    assert!(refreshed[0]["id"].is_string(), "a refresh is a request");
    assert!(server_messages(vec![publish(false)], pull, &mut refreshes).is_empty());

    let silent = DiagnosticDelivery::of_client(&json!({"capabilities": {
        "textDocument": {"diagnostic": {}},
    }}));
    assert_eq!(silent, DiagnosticDelivery::Pull { refresh: false });
    assert!(server_messages(batch(), silent, &mut refreshes).is_empty());

    let push = DiagnosticDelivery::of_client(&json!({"capabilities": {}}));
    assert_eq!(push, DiagnosticDelivery::Push);
    assert_eq!(
        methods(server_messages(batch(), push, &mut refreshes)),
        ["textDocument/publishDiagnostics"; 3]
    );
}

#[test]
fn every_workspace_folder_is_answered_with_its_space_or_a_failure_in_order() {
    let host = Arc::new(HeldHost::default());
    let daemon = LiveV5Daemon::start_with_lsp_host(
        canonical_v13_service(),
        Arc::clone(&host) as Arc<dyn LspHost>,
    );
    let mut editor = Editor::start(&daemon);
    let initialized = editor.request(
        "initialize",
        json!({"workspaceFolders": [
            {"uri": "file:///b", "name": "b"},
            {"uri": "file:///broken", "name": "broken"},
            {"uri": "file:///a", "name": "a"},
        ]}),
    );
    assert_eq!(
        initialized["result"]["capabilities"]["experimental"]["unica"]["workspaces"],
        json!([
            {"folder": "file:///b", "space": "w0"},
            {"folder": "file:///broken", "failure": "lsp_workspace_unavailable"},
            {"folder": "file:///a", "space": "w2"},
        ])
    );

    let content = editor.request(
        "workspace/textDocumentContent",
        json!({"uri": "unica://w0/main/CommonModule.A.Module"}),
    );
    assert_eq!(
        content["result"],
        json!({"text": "// unica://w0/main/CommonModule.A.Module"})
    );
    assert_eq!(
        error_code(&editor.request("workspace/textDocumentContent", json!({}))),
        INVALID_PARAMS
    );
}

#[test]
fn a_document_notification_the_daemon_would_refuse_keeps_the_session() {
    let host = Arc::new(HeldHost::default());
    let daemon = LiveV5Daemon::start_with_lsp_host(
        canonical_v13_service(),
        Arc::clone(&host) as Arc<dyn LspHost>,
    );
    let mut editor = Editor::start(&daemon);
    editor.initialize();
    let waiting = editor.send_request("textDocument/hover", hover_params("file:///work/a.bsl"));
    wait_until(|| host.started.load(Ordering::Acquire) == 1);

    editor.notify(
        "textDocument/didOpen",
        json!({"textDocument": {
            "uri": "file:///work/a.bsl", "languageId": "", "version": 1, "text": "",
        }}),
    );
    editor.notify(
        "textDocument/didChange",
        json!({
            "textDocument": {"uri": "file:///work/a.bsl", "version": 3_000_000_000_u64},
            "contentChanges": [{"text": ""}],
        }),
    );

    assert_eq!(error_code(&editor.response(&waiting)), CONTENT_MODIFIED);
    wait_until(|| host.notified.lock().unwrap().len() == 2);
    assert_eq!(
        *host.notified.lock().unwrap(),
        vec![
            "textDocument/didClose file:///work/a.bsl".to_string(),
            "textDocument/didClose file:///work/a.bsl".to_string(),
        ],
        "the daemon drops the document instead of keeping a text the editor no longer has"
    );
    host.release();
    // The document is out of sync until the editor opens it again.
    let held = editor.request("textDocument/hover", hover_params("file:///work/a.bsl"));
    assert_eq!(error_code(&held), CONTENT_MODIFIED);
    editor.notify(
        "textDocument/didOpen",
        json!({"textDocument": {
            "uri": "file:///work/a.bsl", "languageId": "bsl", "version": 4, "text": "",
        }}),
    );
    let hover = editor.request("textDocument/hover", hover_params("file:///work/a.bsl"));
    assert!(hover["result"]["contents"].is_object(), "{hover}");
    assert!(!editor.lost.load(Ordering::Acquire));
}

#[test]
fn a_session_the_daemon_no_longer_serves_ends_the_frontend() {
    let host = Arc::new(HeldHost::default());
    let mut daemon = LiveV5Daemon::start_with_lsp_host(
        canonical_v13_service(),
        Arc::clone(&host) as Arc<dyn LspHost>,
    );
    let editor = Editor::start(&daemon);
    let mut editor = editor;
    editor.initialize();
    assert!(!editor.lost.load(Ordering::Acquire));

    daemon.stop(daemon.owner());

    wait_until(|| editor.lost.load(Ordering::Acquire));
}

/// Служба продукта: папка редактора и вызов MCP той же рабочей области
/// приходят к одному актору; буфер редактора виден в его ответах и не
/// попадает на диск.
#[test]
fn the_product_service_serves_an_editor_folder_over_the_mcp_workspace_actor() {
    let daemon = LiveV5Daemon::start_with_product_lsp(canonical_v13_service());
    let owner = daemon.owner();
    let workspace = catalog_workspace();
    // No platform installation: completion stays without the global context.
    std::fs::write(
        workspace.path().join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\ntools:\n  platform:\n    path: absent/8.3.27.1\n",
    )
    .unwrap();
    let module = workspace
        .path()
        .join("src/CommonModules/Main/Ext/Module.bsl");
    std::fs::create_dir_all(module.parent().unwrap()).unwrap();
    std::fs::write(&module, "Procedure Disk() EndProcedure\n").unwrap();
    let root = std::fs::canonicalize(workspace.path()).unwrap();
    let folder: String = url::Url::from_file_path(&root).unwrap().into();
    // The document is the module body, as MCP `view` and `resolve` name it.
    let module_uri = "unica://work/main/CommonModule.Main.Body".to_owned();
    let mut editor = Editor::start(&daemon);

    let initialized = editor.request(
        "initialize",
        json!({"workspaceFolders": [{"uri": folder, "name": "Work"}]}),
    );
    let capabilities = &initialized["result"]["capabilities"];
    assert_eq!(
        capabilities["experimental"]["unica"]["workspaces"],
        json!([{"folder": folder, "space": "work"}])
    );
    assert_eq!(
        capabilities["workspace"]["textDocumentContent"],
        json!({"schemes": ["unica"]})
    );
    assert_eq!(capabilities["textDocumentSync"]["change"], 2);
    mcp_view(&daemon, &owner, &view_request(workspace.path()));
    assert_eq!(
        daemon.live_workspace_actors(),
        1,
        "MCP and LSP share the actor"
    );

    let content = |editor: &mut Editor| {
        editor.request(
            "workspace/textDocumentContent",
            json!({"uri": "unica://work/main/CommonModule.Main.Body"}),
        )["result"]["text"]
            .clone()
    };
    assert_eq!(
        content(&mut editor),
        json!("Procedure Disk() EndProcedure\n")
    );
    editor.notify(
        "textDocument/didOpen",
        json!({"textDocument": {
            "uri": module_uri, "languageId": "bsl", "version": 1,
            "text": "Procedure Buffer() EndProcedure\n",
        }}),
    );
    assert_eq!(
        content(&mut editor),
        json!("Procedure Buffer() EndProcedure\n")
    );
    assert_eq!(
        std::fs::read_to_string(&module).unwrap(),
        "Procedure Disk() EndProcedure\n"
    );
    let foreign = editor.request(
        "workspace/textDocumentContent",
        json!({"uri": "unica://other/main/CommonModule.Main.Body"}),
    );
    assert_eq!(foreign["error"]["message"], "unica: lsp_document_unknown");

    // Editor facts come from the buffer the session sees.
    let document = json!({"textDocument": {"uri": "unica://work/main/CommonModule.Main.Body"}});
    let symbols = editor.request("textDocument/documentSymbol", document.clone());
    assert_eq!(symbols["result"][0]["name"], "Buffer", "{symbols}");
    assert!(editor.request("textDocument/foldingRange", document.clone())["result"].is_array());
    let tokens = editor.request("textDocument/semanticTokens/full", document.clone());
    assert!(tokens["result"]["data"].is_array(), "{tokens}");
    let help = editor.request(
        "textDocument/signatureHelp",
        json!({
            "textDocument": {"uri": "unica://work/main/CommonModule.Main.Body"},
            "position": {"line": 0, "character": 0},
        }),
    );
    assert!(help.get("result").is_some(), "{help}");
    for method in ["documentSymbolProvider", "foldingRangeProvider"] {
        assert_eq!(capabilities[method], true, "{method}");
    }
    assert_eq!(capabilities["semanticTokensProvider"]["full"], true);
    assert!(capabilities["semanticTokensProvider"]["legend"]["tokenTypes"].is_array());
    assert_eq!(
        capabilities["signatureHelpProvider"],
        json!({"triggerCharacters": ["(", ","]})
    );
    assert_eq!(
        capabilities["completionProvider"],
        json!({"triggerCharacters": ["."], "resolveProvider": true})
    );
    let completion = editor.request(
        "textDocument/completion",
        json!({
            "textDocument": {"uri": "unica://work/main/CommonModule.Main.Body"},
            "position": {"line": 0, "character": 0},
        }),
    );
    assert_eq!(completion["result"]["isIncomplete"], true, "{completion}");
    let item = completion["result"]["items"]
        .as_array()
        .and_then(|items| items.first())
        .unwrap_or_else(|| panic!("completion offers the module's names: {completion}"))
        .clone();
    let resolved = editor.request("completionItem/resolve", item.clone());
    assert_eq!(resolved["result"]["label"], item["label"], "{resolved}");

    // A change the buffer cannot take puts the document out of sync: no
    // answer about it until the editor sends its whole text again.
    editor.notify(
        "textDocument/didChange",
        json!({
            "textDocument": {"uri": module_uri, "version": 2},
            "contentChanges": [{
                "range": {"start": {"line": 99, "character": 0}, "end": {"line": 99, "character": 0}},
                "text": "x",
            }],
        }),
    );
    let stale = editor.request(
        "textDocument/documentSymbol",
        json!({"textDocument": {"uri": module_uri}}),
    );
    assert_eq!(error_code(&stale), CONTENT_MODIFIED, "{stale}");
    editor.notify(
        "textDocument/didOpen",
        json!({"textDocument": {
            "uri": module_uri, "languageId": "bsl", "version": 3,
            "text": "Procedure Reopened() EndProcedure\n",
        }}),
    );
    let symbols = editor.request(
        "textDocument/documentSymbol",
        json!({"textDocument": {"uri": module_uri}}),
    );
    assert_eq!(symbols["result"][0]["name"], "Reopened", "{symbols}");
    assert!(!editor.lost.load(Ordering::Acquire));
}

/// Приёмка #2173: поток hover не задерживает параллельный вызов MCP.
///
/// Сначала полоса LSP занята целиком и стоит: вызов MCP той же рабочей
/// области отвечает, не дожидаясь её. Затем полоса непрерывно занята
/// расчётом hover, и медиана задержки MCP сравнивается с медианой без LSP.
#[test]
fn an_lsp_hover_stream_does_not_delay_a_parallel_mcp_call() {
    let host = Arc::new(HeldHost::default());
    let daemon = LiveV5Daemon::start_with_lsp_host(
        canonical_v13_service(),
        Arc::clone(&host) as Arc<dyn LspHost>,
    );
    let owner = daemon.owner();
    let workspace = catalog_workspace();
    let view = view_request(workspace.path());
    // A warm call: the first one opens the workspace actor and its index.
    mcp_view(&daemon, &owner, &view);
    let baseline = median((0..MCP_SAMPLES).map(|_| mcp_view(&daemon, &owner, &view)));

    let mut editor = Editor::start(&daemon);
    editor.initialize();
    let held = (0..MAX_LSP_LANE_WORKERS + 6)
        .map(|_| editor.send_request("textDocument/hover", hover_params("file:///work/a.bsl")))
        .collect::<Vec<_>>();
    wait_until(|| host.started.load(Ordering::Acquire) == MAX_LSP_LANE_WORKERS);

    mcp_view(&daemon, &owner, &view);
    assert_eq!(
        host.started.load(Ordering::Acquire),
        MAX_LSP_LANE_WORKERS,
        "MCP answered while every LSP worker was still held"
    );
    assert!(editor.try_response(&held[0]).is_none());

    host.spin_for(HOVER_COST);
    for id in &held {
        assert!(editor.response(id)["result"]["contents"].is_object());
    }
    let stop = Arc::new(AtomicBool::new(false));
    let hovers = Arc::new(AtomicUsize::new(0));
    thread::scope(|scope| {
        let stream = {
            let (stop, hovers) = (Arc::clone(&stop), Arc::clone(&hovers));
            scope.spawn(move || {
                while !stop.load(Ordering::Acquire) {
                    let ids = (0..MAX_LSP_LANE_WORKERS * 4)
                        .map(|_| {
                            editor.send_request(
                                "textDocument/hover",
                                hover_params("file:///work/a.bsl"),
                            )
                        })
                        .collect::<Vec<_>>();
                    for id in ids {
                        assert!(editor.response(&id)["result"]["contents"].is_object());
                        hovers.fetch_add(1, Ordering::AcqRel);
                    }
                }
                editor
            })
        };
        let _stop_on_drop = StopOnDrop(Arc::clone(&stop));
        wait_until(|| hovers.load(Ordering::Acquire) > 0);
        let before = host.answered.load(Ordering::Acquire);
        let started = Instant::now();
        let under_lsp = median((0..MCP_SAMPLES).map(|_| mcp_view(&daemon, &owner, &view)));
        let window = started.elapsed();
        let during = host.answered.load(Ordering::Acquire) - before;
        stop.store(true, Ordering::Release);
        let _editor = stream.join().unwrap();

        // The lane was computing hovers through the window: at least half of
        // what its workers fit in it, the rest lost between the stream's batches.
        let capacity =
            (window.as_millis() / HOVER_COST.as_millis()) as usize * MAX_LSP_LANE_WORKERS;
        assert!(
            during * 2 >= capacity,
            "the LSP lane answered {during} hovers in {window:?}, under half of its {capacity}"
        );
        let allowed = baseline * 3 / 2 + MCP_LATENCY_TOLERANCE;
        assert!(
        under_lsp <= allowed,
        "MCP median {under_lsp:?} under a hover stream exceeds {allowed:?} (median {baseline:?} without LSP)"
    );
    });
}

/// Приёмка #2173: устаревший запрос отменяется и его результат не выдаётся.
#[test]
fn a_stale_lsp_request_is_cancelled_and_its_answer_withheld() {
    let host = Arc::new(HeldHost::default());
    let daemon = LiveV5Daemon::start_with_lsp_host(
        canonical_v13_service(),
        Arc::clone(&host) as Arc<dyn LspHost>,
    );
    let mut editor = Editor::start(&daemon);
    editor.initialize();

    let cancelled = editor.send_request("textDocument/hover", hover_params("file:///work/a.bsl"));
    wait_until(|| host.started.load(Ordering::Acquire) == 1);
    editor.notify("$/cancelRequest", json!({"id": cancelled}));
    assert_eq!(error_code(&editor.response(&cancelled)), REQUEST_CANCELLED);
    wait_until(|| host.saw_cancellation.load(Ordering::Acquire) == 1);

    let outdated = editor.send_request("textDocument/hover", hover_params("file:///work/a.bsl"));
    let other = editor.send_request("textDocument/hover", hover_params("file:///work/b.bsl"));
    wait_until(|| host.started.load(Ordering::Acquire) == 3);
    editor.notify(
        "textDocument/didChange",
        json!({
            "textDocument": {"uri": "file:///work/a.bsl", "version": 2},
            "contentChanges": [{"text": "Процедура А() КонецПроцедуры"}],
        }),
    );
    assert_eq!(error_code(&editor.response(&outdated)), CONTENT_MODIFIED);
    wait_until(|| host.saw_cancellation.load(Ordering::Acquire) == 2);
    assert_eq!(
        *host.notified.lock().unwrap(),
        vec!["textDocument/didChange file:///work/a.bsl v2".to_string()]
    );

    host.release();
    assert!(editor.response(&other)["result"]["contents"].is_object());
    assert_eq!(host.answered.load(Ordering::Acquire), 1);
}

/// Приёмка #2173: редактор, пропавший посреди запроса, не роняет демон и MCP.
#[test]
fn a_vanished_lsp_client_leaves_the_daemon_and_mcp_serving() {
    let host = Arc::new(HeldHost::default());
    let daemon = LiveV5Daemon::start_with_lsp_host(
        canonical_v13_service(),
        Arc::clone(&host) as Arc<dyn LspHost>,
    );
    let owner = daemon.owner();
    let deadline = || Instant::now() + WAIT;
    let mut anchor = owner.connect_peer_before(deadline()).unwrap();
    let (session, _) = anchor
        .lsp_open_before(vec![], V5LspDiagnosticsDelivery::Push, deadline())
        .expect("open an LSP session");
    let in_flight = {
        let mut peer = owner.connect_peer_before(deadline()).unwrap();
        let session = session.clone();
        thread::spawn(move || {
            peer.lsp_call_before(
                &session,
                1,
                5_000,
                hover_call(&hover_params("file:///a.bsl")).unwrap(),
                deadline(),
            )
        })
    };
    wait_until(|| host.started.load(Ordering::Acquire) == 1);

    // The socket closes as it would when the editor's process dies: no
    // shutdown, no close, a request still running.
    drop(anchor);

    assert_eq!(
        in_flight.join().unwrap().unwrap(),
        V5LspOutcome::Cancelled {}
    );
    wait_until(|| host.closed.load(Ordering::Acquire) == 1);
    assert_eq!(host.saw_cancellation.load(Ordering::Acquire), 1);
    let workspace = catalog_workspace();
    mcp_view(&daemon, &owner, &view_request(workspace.path()));
    let mut probe = owner.connect_peer_before(deadline()).unwrap();
    probe.ping().expect("the daemon still answers");
    assert!(matches!(
        probe.lsp_call_before(
            &session,
            2,
            1_000,
            hover_call(&hover_params("file:///a.bsl")).unwrap(),
            deadline()
        ),
        Ok(V5LspOutcome::Failed {
            failure: V5LspFailure::SessionUnknown
        })
    ));
    let mut editor = Editor::start(&daemon);
    editor.initialize();
}

/// Служба LSP, чей hover держит исполнителя, пока тест его не отпустит.
#[derive(Default)]
struct HeldHost {
    folders: Mutex<Vec<V5LspWorkspaceFolder>>,
    client: Mutex<Option<LspClientChannel>>,
    notified: Mutex<Vec<String>>,
    started: AtomicUsize,
    saw_cancellation: AtomicUsize,
    answered: AtomicUsize,
    closed: AtomicUsize,
    released: AtomicBool,
    spin_ms: std::sync::atomic::AtomicU64,
}

impl HeldHost {
    fn release(&self) {
        self.released.store(true, Ordering::Release);
    }

    fn spin_for(&self, duration: Duration) {
        self.spin_ms
            .store(duration.as_millis() as u64, Ordering::Release);
        self.release();
    }

    fn push(&self, notification: V5LspServerNotification) {
        let client = self.client.lock().unwrap().clone().unwrap();
        assert!(client.push(notification));
    }
}

impl LspHost for HeldHost {
    fn open(
        &self,
        _: &V5LspSessionId,
        folders: &[V5LspWorkspaceFolder],
        client: LspClientChannel,
        _: Instant,
    ) -> Result<V5LspOpening, V5LspFailure> {
        *self.folders.lock().unwrap() = folders.to_vec();
        *self.client.lock().unwrap() = Some(client);
        Ok(V5LspOpening {
            capabilities: V5LspCapabilities {
                text_document_sync: V5LspTextSync::Incremental,
                hover: true,
                text_document_content: true,
                ..V5LspCapabilities::NONE
            },
            workspaces: folders
                .iter()
                .enumerate()
                .map(|(index, folder)| match folder.name.as_str() {
                    "broken" => V5LspWorkspaceSpace::refused(
                        &folder.uri,
                        V5LspFailure::WorkspaceUnavailable,
                    ),
                    _ => V5LspWorkspaceSpace::opened(&folder.uri, format!("w{index}")),
                })
                .collect(),
        })
    }

    fn notify(
        &self,
        _: &V5LspSessionId,
        notification: V5LspNotification,
        _: Instant,
    ) -> Result<(), V5LspFailure> {
        let entry = match &notification {
            V5LspNotification::DidChange { uri, version, .. } => {
                format!("textDocument/didChange {uri} v{version}")
            }
            V5LspNotification::DidClose { uri } => format!("textDocument/didClose {uri}"),
            other => format!("{other:?}"),
        };
        self.notified.lock().unwrap().push(entry);
        Ok(())
    }

    fn call(&self, _: &V5LspSessionId, call: V5LspCall, context: &LspCallContext) -> V5LspOutcome {
        if let V5LspCall::TextDocumentContent { uri } = call {
            return V5LspOutcome::Text {
                text: format!("// {uri}"),
            };
        }
        self.started.fetch_add(1, Ordering::AcqRel);
        while !self.released.load(Ordering::Acquire) {
            if context.is_cancelled() {
                self.saw_cancellation.fetch_add(1, Ordering::AcqRel);
                return V5LspOutcome::Cancelled {};
            }
            thread::sleep(Duration::from_millis(1));
        }
        // A hover that costs processor time, as analysis does.
        let spin = Duration::from_millis(self.spin_ms.load(Ordering::Acquire));
        let started = Instant::now();
        let mut work = 0_u64;
        while started.elapsed() < spin && !context.is_cancelled() {
            work = std::hint::black_box(work.wrapping_mul(6364136223846793005).wrapping_add(1));
        }
        self.answered.fetch_add(1, Ordering::AcqRel);
        V5LspOutcome::Hover {
            contents: Some("answer".into()),
        }
    }

    fn close(&self, _: &V5LspSessionId) {
        self.closed.fetch_add(1, Ordering::AcqRel);
    }
}

/// Редактор на другом конце stdio: пишет кадры во вход `serve` и разбирает
/// кадры её вывода.
#[derive(Default)]
struct EditorOutput {
    bytes: Vec<u8>,
    closed: bool,
}

struct Editor {
    input: Option<mpsc::Sender<Vec<u8>>>,
    output: Arc<(Mutex<EditorOutput>, Condvar)>,
    received: VecDeque<Value>,
    next_id: u64,
    thread: Option<thread::JoinHandle<i32>>,
    lost: Arc<AtomicBool>,
}

impl Editor {
    fn start(daemon: &LiveV5Daemon) -> Self {
        let (input, receiver) = mpsc::channel();
        let output = Arc::new((Mutex::new(EditorOutput::default()), Condvar::new()));
        let client = Arc::new(V5DaemonClient::from(daemon.owner()));
        let writer = SharedWriter::new(Arc::clone(&output));
        let lost = Arc::new(AtomicBool::new(false));
        let session_lost: SessionLost = {
            let lost = Arc::clone(&lost);
            Arc::new(move || lost.store(true, Ordering::Release))
        };
        let thread = thread::spawn(move || {
            serve(
                BufReader::new(ChannelReader {
                    receiver,
                    pending: Cursor::new(Vec::new()),
                }),
                writer,
                client,
                session_lost,
            )
        });
        Self {
            input: Some(input),
            output,
            received: VecDeque::new(),
            next_id: 0,
            thread: Some(thread),
            lost,
        }
    }

    fn initialize(&mut self) -> Value {
        let initialized = self.request(
            "initialize",
            json!({
                "processId": null,
                "capabilities": {},
                "workspaceFolders": [{"uri": "file:///work", "name": "work"}],
            }),
        );
        assert!(initialized.get("result").is_some(), "{initialized}");
        self.notify("initialized", json!({}));
        initialized
    }

    fn send(&self, message: Value) {
        let mut bytes = Vec::new();
        codec::write_message(&mut bytes, &message).unwrap();
        self.input.as_ref().unwrap().send(bytes).unwrap();
    }

    fn send_request(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        let id = json!(self.next_id);
        self.send(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        id
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.send_request(method, params);
        self.response(&id)
    }

    fn notify(&self, method: &str, params: Value) {
        self.send(json!({"jsonrpc": "2.0", "method": method, "params": params}));
    }

    fn response(&mut self, id: &Value) -> Value {
        let deadline = Instant::now() + WAIT;
        loop {
            if let Some(message) = self.take(|message| message.get("id") == Some(id)) {
                return message;
            }
            assert!(Instant::now() < deadline, "no response to request {id}");
            self.receive(deadline);
        }
    }

    fn try_response(&mut self, id: &Value) -> Option<Value> {
        self.receive(Instant::now() + Duration::from_millis(50));
        self.take(|message| message.get("id") == Some(id))
    }

    fn next_notification(&mut self, method: &str) -> Value {
        let deadline = Instant::now() + WAIT;
        loop {
            if let Some(message) = self.take(|message| {
                message.get("id").is_none() && message.get("method") == Some(&json!(method))
            }) {
                return message;
            }
            assert!(Instant::now() < deadline, "no {method} notification");
            self.receive(deadline);
        }
    }

    fn take(&mut self, matches: impl Fn(&Value) -> bool) -> Option<Value> {
        let index = self.received.iter().position(matches)?;
        self.received.remove(index)
    }

    /// Move every complete frame written so far into `received`, waiting
    /// for output until `deadline` when there is none.
    fn receive(&mut self, deadline: Instant) {
        assert!(
            self.receive_result(deadline).unwrap(),
            "LSP output ended while awaiting a message"
        );
    }

    /// `false` is terminal EOF between messages, not a temporarily empty buffer.
    fn receive_result(&mut self, deadline: Instant) -> io::Result<bool> {
        let (incoming, closed) = {
            let (output, written) = &*self.output;
            let output = output.lock().unwrap();
            let wait = deadline.saturating_duration_since(Instant::now());
            let mut output = written
                .wait_timeout_while(output, wait, |output| {
                    output.bytes.is_empty() && !output.closed
                })
                .unwrap()
                .0;
            (std::mem::take(&mut output.bytes), output.closed)
        };
        let ended = closed && incoming.is_empty();
        // A malformed terminal frame must not poison the writer's mutex.
        let mut reader = BufReader::new(Cursor::new(incoming));
        while let Some(body) = codec::read_message(&mut reader)? {
            self.received.push_back(
                serde_json::from_slice(&body)
                    .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?,
            );
        }
        Ok(!ended)
    }

    fn finish(mut self) -> i32 {
        self.input.take();
        self.thread.take().unwrap().join().unwrap()
    }
}

impl Drop for Editor {
    fn drop(&mut self) {
        self.input.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// `codec::write_message` ends each whole message with `flush`.
/// Keep its individual write chunks private until that publication point.
struct SharedWriter {
    output: Arc<(Mutex<EditorOutput>, Condvar)>,
    pending: Vec<u8>,
}

impl SharedWriter {
    fn new(output: Arc<(Mutex<EditorOutput>, Condvar)>) -> Self {
        Self {
            output,
            pending: Vec::new(),
        }
    }

    fn publish(&mut self, closed: bool) {
        let (output, written) = &*self.output;
        let mut output = output.lock().unwrap();
        output.bytes.append(&mut self.pending);
        output.closed |= closed;
        drop(output);
        written.notify_all();
    }
}

impl Write for SharedWriter {
    fn write(&mut self, chunk: &[u8]) -> io::Result<usize> {
        self.pending.extend_from_slice(chunk);
        Ok(chunk.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.publish(false);
        Ok(())
    }
}

impl Drop for SharedWriter {
    fn drop(&mut self) {
        // Do not hide an unflushed truncated prefix when the producer ends.
        self.publish(true);
    }
}

struct ChannelReader {
    receiver: mpsc::Receiver<Vec<u8>>,
    pending: Cursor<Vec<u8>>,
}

impl Read for ChannelReader {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        loop {
            let read = self.pending.read(buffer)?;
            if read > 0 {
                return Ok(read);
            }
            match self.receiver.recv() {
                Ok(bytes) => self.pending = Cursor::new(bytes),
                Err(_) => return Ok(0),
            }
        }
    }
}

fn hover_params(uri: &str) -> Value {
    json!({"textDocument": {"uri": uri}, "position": {"line": 0, "character": 0}})
}

fn error_code(response: &Value) -> i64 {
    response["error"]["code"]
        .as_i64()
        .unwrap_or_else(|| panic!("not an error response: {response}"))
}

fn wait_until(condition: impl Fn() -> bool) {
    let deadline = Instant::now() + WAIT;
    while !condition() {
        assert!(Instant::now() < deadline, "condition did not hold in time");
        thread::sleep(Duration::from_millis(1));
    }
}

fn median(samples: impl Iterator<Item = Duration>) -> Duration {
    let mut samples = samples.collect::<Vec<_>>();
    samples.sort();
    samples[samples.len() / 2]
}

fn catalog_workspace() -> tempfile::TempDir {
    let workspace = tempfile::tempdir().unwrap();
    let source = workspace.path().join("src");
    std::fs::create_dir_all(source.join("Catalogs")).unwrap();
    std::fs::write(
        workspace.path().join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
    )
    .unwrap();
    std::fs::write(
        source.join("Configuration.xml"),
        r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Store</Name></Properties><ChildObjects><Catalog>Items</Catalog></ChildObjects></Configuration></MetaDataObject>"#,
    )
    .unwrap();
    std::fs::write(
        source.join("Catalogs/Items.xml"),
        r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Catalog uuid="aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"><Properties><Name>Items</Name></Properties><ChildObjects/></Catalog></MetaDataObject>"#,
    )
    .unwrap();
    workspace
}

fn view_request(workspace: &Path) -> InvocationRequest {
    let hint = std::fs::canonicalize(workspace).unwrap();
    InvocationRequest::new(
        ToolIdentity::View,
        json!({"at": "main:Catalog.Items"}),
        hint.to_string_lossy(),
        7_000,
    )
    .unwrap()
}

/// One MCP `unica.view` over the daemon wire; returns how long it took.
fn mcp_view(
    daemon: &LiveV5Daemon,
    owner: &V5DaemonProcessOwner,
    request: &InvocationRequest,
) -> Duration {
    let started = Instant::now();
    let V5Submission::Direct(result) = daemon.submit(owner, request) else {
        panic!("a warm view answers inline");
    };
    assert!(result.ok, "{} {:?}", result.summary, result.diagnostics);
    started.elapsed()
}
