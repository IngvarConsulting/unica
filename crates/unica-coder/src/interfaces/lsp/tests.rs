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

#[test]
fn lsp_lifecycle_declares_only_ready_capabilities_and_ends_with_exit() {
    let daemon = LiveV5Daemon::start(canonical_v13_service());
    let mut editor = Editor::start(&daemon);

    let early = editor.request("textDocument/hover", hover_params("file:///a.bsl"));
    assert_eq!(error_code(&early), SERVER_NOT_INITIALIZED);
    let initialized = editor.initialize();
    let capabilities = &initialized["result"]["capabilities"];
    assert_eq!(
        capabilities,
        &json!({
            "positionEncoding": "utf-16",
            "experimental": {"unica": {"workspaces": [
                {"folder": "file:///work", "failure": "lsp_not_supported"},
            ]}},
        }),
        "the daemon has no LSP service yet: no capability, and no folder became a space"
    );
    assert_eq!(initialized["result"]["serverInfo"]["name"], "unica");
    assert_eq!(
        error_code(&editor.request("initialize", json!({}))),
        INVALID_REQUEST
    );
    let hover = editor.request("textDocument/hover", hover_params("file:///a.bsl"));
    assert_eq!(error_code(&hover), METHOD_NOT_FOUND);
    assert_eq!(
        error_code(&editor.request("textDocument/definition", json!({}))),
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
    let stream = {
        let (stop, hovers) = (Arc::clone(&stop), Arc::clone(&hovers));
        thread::spawn(move || {
            while !stop.load(Ordering::Acquire) {
                let ids = (0..MAX_LSP_LANE_WORKERS * 4)
                    .map(|_| {
                        editor
                            .send_request("textDocument/hover", hover_params("file:///work/a.bsl"))
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
    let capacity = (window.as_millis() / HOVER_COST.as_millis()) as usize * MAX_LSP_LANE_WORKERS;
    assert!(
        during * 2 >= capacity,
        "the LSP lane answered {during} hovers in {window:?}, under half of its {capacity}"
    );
    let allowed = baseline * 3 / 2 + MCP_LATENCY_TOLERANCE;
    assert!(
        under_lsp <= allowed,
        "MCP median {under_lsp:?} under a hover stream exceeds {allowed:?} (median {baseline:?} without LSP)"
    );
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
        .lsp_open_before(vec![], deadline())
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
struct Editor {
    input: Option<mpsc::Sender<Vec<u8>>>,
    output: Arc<(Mutex<Vec<u8>>, Condvar)>,
    received: VecDeque<Value>,
    next_id: u64,
    thread: Option<thread::JoinHandle<i32>>,
    lost: Arc<AtomicBool>,
}

impl Editor {
    fn start(daemon: &LiveV5Daemon) -> Self {
        let (input, receiver) = mpsc::channel();
        let output = Arc::new((Mutex::new(Vec::new()), Condvar::new()));
        let client = Arc::new(V5DaemonClient::from(daemon.owner()));
        let writer = SharedWriter(Arc::clone(&output));
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
        let (bytes, written) = &*self.output;
        let mut bytes = bytes.lock().unwrap();
        if bytes.is_empty() {
            let wait = deadline.saturating_duration_since(Instant::now());
            bytes = written.wait_timeout(bytes, wait).unwrap().0;
        }
        let mut reader = BufReader::new(Cursor::new(std::mem::take(&mut *bytes)));
        while let Some(body) = codec::read_message(&mut reader).unwrap() {
            self.received
                .push_back(serde_json::from_slice(&body).unwrap());
        }
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

/// Кадры приходят целиком: писатель держит блокировку на всё сообщение.
struct SharedWriter(Arc<(Mutex<Vec<u8>>, Condvar)>);

impl Write for SharedWriter {
    fn write(&mut self, chunk: &[u8]) -> io::Result<usize> {
        self.0 .0.lock().unwrap().extend_from_slice(chunk);
        Ok(chunk.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0 .1.notify_all();
        Ok(())
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
