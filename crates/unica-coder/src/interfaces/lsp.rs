//! `unica lsp`: дверь редактора в тот же демон, что обслуживает MCP.
//!
//! Тонкий frontend: читает LSP со stdio и передаёт его демону по маршруту
//! LSP протокола v5. Сессию открывает и держит одно соединение, по нему же
//! по порядку идут уведомления о документах. Каждый запрос исполняется
//! своим соединением на полосе LSP демона; отмена уходит отдельным
//! соединением. Возможности объявляет служба демона по своей готовности.

mod codec;
#[cfg(test)]
mod tests;

use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::io::{self, BufRead, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use unica_daemon::client_v5::{V5DaemonClient, V5DaemonProcessOwner, V5LspExchangeError};
use unica_daemon::protocol_v5::lsp::{
    V5LspCall, V5LspContentChange, V5LspFailure, V5LspMessageLevel, V5LspNotification,
    V5LspOpening, V5LspOutcome, V5LspPosition, V5LspRange, V5LspServerNotification, V5LspSessionId,
    V5LspTextSync, V5LspWorkspaceFolder,
};
use unica_daemon::protocol_v5::V5EndpointRecord;

/// Бюджет запроса на полосе демона.
const LSP_CALL_BUDGET: Duration = Duration::from_secs(5);
/// Запас на обмен поверх бюджета запроса.
const LSP_EXCHANGE_MARGIN: Duration = Duration::from_secs(1);
/// Срок обмена сессии: открытие, уведомление, закрытие.
/// Above the daemon's own budget for these steps (7 s), so the daemon
/// answers, even with a refusal, before the frontend gives up.
const LSP_SESSION_EXCHANGE_TIMEOUT: Duration = Duration::from_secs(8);
/// Ожидание уведомлений демона одним опросом.
const LSP_POLL_WAIT_MS: u64 = 2_000;
/// Подряд неудавшихся опросов, после которых сессия считается потерянной.
const MAX_LSP_POLL_FAILURES: u32 = 3;
/// Запросов редактора, ждущих ответа демона.
const MAX_LSP_REQUESTS_IN_FLIGHT: usize = 32;

const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;
const SERVER_NOT_INITIALIZED: i64 = -32002;
const REQUEST_FAILED: i64 = -32803;
const CONTENT_MODIFIED: i64 = -32801;
const REQUEST_CANCELLED: i64 = -32800;

pub fn run_stdio() -> i32 {
    let state_root = match crate::interfaces::daemon::default_user_daemon_state_root() {
        Ok(root) => root,
        Err(error) => {
            eprintln!("failed to resolve unica user daemon state: {error}");
            return 1;
        }
    };
    let client = match crate::interfaces::daemon::connect_default_user_daemon(&state_root) {
        Ok(client) => client,
        Err(error) => {
            eprintln!("failed to connect to unica user daemon: {error}");
            return 1;
        }
    };
    let stdin = io::stdin();
    // The main loop waits on the editor's input. A session the daemon no
    // longer has ends the process from the poller, so the editor restarts
    // the server instead of getting refusals forever.
    let session_lost: SessionLost = Arc::new(|| {
        eprintln!("unica lsp lost its daemon session");
        std::process::exit(1);
    });
    serve(stdin.lock(), io::stdout(), Arc::new(client), session_lost)
}

/// Called once the daemon no longer serves this connection's session.
pub(crate) type SessionLost = Arc<dyn Fn() + Send + Sync>;

/// Обслужить одно соединение редактора до `exit` или конца ввода.
/// Возвращает код выхода процесса.
pub(crate) fn serve(
    mut input: impl BufRead,
    output: impl Write + Send + 'static,
    client: Arc<V5DaemonClient>,
    session_lost: SessionLost,
) -> i32 {
    let mut frontend = Frontend {
        client,
        session_lost,
        output: Arc::new(Mutex::new(Box::new(output))),
        phase: Phase::Uninitialized,
        in_flight: Arc::new(Mutex::new(HashMap::new())),
        next_request: 0,
        unsynced: std::collections::HashSet::new(),
    };
    let code = loop {
        let body = match codec::read_message(&mut input) {
            Ok(Some(body)) => body,
            // A closed input without `exit` is an editor that went away.
            Ok(None) => break 1,
            Err(error) => {
                eprintln!("unica lsp input is not a valid LSP stream: {error}");
                break 1;
            }
        };
        match frontend.handle(&body) {
            Flow::Continue => {}
            Flow::Exit(code) => break code,
            Flow::Fail(error) => {
                eprintln!("unica lsp lost its daemon session: {error}");
                break 1;
            }
        }
    };
    frontend.end();
    code
}

type Output = Arc<Mutex<Box<dyn Write + Send>>>;

struct Frontend {
    client: Arc<V5DaemonClient>,
    session_lost: SessionLost,
    output: Output,
    phase: Phase,
    in_flight: InFlight,
    next_request: u64,
    /// Documents whose text the daemon lost track of. Requests about them
    /// are refused until the editor opens them again.
    unsynced: std::collections::HashSet<String>,
}

enum Phase {
    Uninitialized,
    Running(Box<Session>),
    ShutDown,
}

struct Session {
    id: V5LspSessionId,
    /// The connection that owns the session; it ends the session when it closes.
    anchor: V5DaemonProcessOwner,
    endpoint: V5EndpointRecord,
    poll_stop: Arc<AtomicBool>,
}

/// Editor requests waiting for the daemon, keyed by their JSON-RPC id.
/// Whoever removes an entry answers it, exactly once.
type InFlight = Arc<Mutex<HashMap<String, PendingRequest>>>;

struct PendingRequest {
    id: Value,
    request: u64,
    uri: Option<String>,
}

enum Flow {
    Continue,
    Exit(i32),
    Fail(String),
}

impl Frontend {
    fn handle(&mut self, body: &[u8]) -> Flow {
        let Ok(message) = serde_json::from_slice::<Value>(body) else {
            self.send(error_response(
                Value::Null,
                PARSE_ERROR,
                "message is not JSON",
            ));
            return Flow::Continue;
        };
        let method = message.get("method").and_then(Value::as_str);
        let id = message.get("id").cloned();
        let params = message.get("params").cloned().unwrap_or(Value::Null);
        match (method, id) {
            (Some(method), Some(id)) => self.request(method, id, params),
            (Some(method), None) => self.notification(method, params),
            // Answers to server requests: this server sends none.
            (None, Some(_))
                if message.get("result").is_some() || message.get("error").is_some() =>
            {
                Flow::Continue
            }
            (None, id) => {
                self.send(error_response(
                    id.unwrap_or(Value::Null),
                    INVALID_REQUEST,
                    "message has no method",
                ));
                Flow::Continue
            }
        }
    }

    fn request(&mut self, method: &str, id: Value, params: Value) -> Flow {
        match (&self.phase, method) {
            (Phase::Uninitialized, "initialize") => return self.initialize(id, &params),
            (_, "initialize") => {
                self.send(error_response(id, INVALID_REQUEST, "already initialized"));
            }
            (Phase::Uninitialized, _) => {
                self.send(error_response(
                    id,
                    SERVER_NOT_INITIALIZED,
                    "server is not initialized",
                ));
            }
            (Phase::ShutDown, _) => {
                self.send(error_response(id, INVALID_REQUEST, "server is shut down"));
            }
            (Phase::Running(_), "shutdown") => {
                self.close_session();
                self.phase = Phase::ShutDown;
                self.send(json!({"jsonrpc": "2.0", "id": id, "result": null}));
            }
            (Phase::Running(_), "textDocument/hover") => match hover_call(&params) {
                Some(call) => self.dispatch(id, call),
                None => self.send(error_response(id, INVALID_PARAMS, "invalid hover params")),
            },
            (Phase::Running(_), "workspace/textDocumentContent") => {
                match params.get("uri").and_then(Value::as_str) {
                    Some(uri) => self.dispatch(
                        id,
                        V5LspCall::TextDocumentContent {
                            uri: uri.to_owned(),
                        },
                    ),
                    None => self.send(error_response(id, INVALID_PARAMS, "invalid content params")),
                }
            }
            (Phase::Running(_), method) if document_call(method, &Value::Null).is_some() => {
                match document_call(method, &params).flatten() {
                    Some(call) => self.dispatch(id, call),
                    None => self.send(error_response(id, INVALID_PARAMS, "invalid params")),
                }
            }
            (Phase::Running(_), _) => {
                self.send(error_response(
                    id,
                    METHOD_NOT_FOUND,
                    "method is not supported",
                ));
            }
        }
        Flow::Continue
    }

    fn notification(&mut self, method: &str, params: Value) -> Flow {
        match method {
            "exit" => {
                let code = if matches!(self.phase, Phase::ShutDown) {
                    0
                } else {
                    1
                };
                Flow::Exit(code)
            }
            "$/cancelRequest" => {
                if let Some(id) = params.get("id") {
                    self.withdraw(&request_key(id), REQUEST_CANCELLED, "request cancelled");
                }
                Flow::Continue
            }
            "textDocument/didOpen" | "textDocument/didChange" | "textDocument/didClose" => {
                let Phase::Running(_) = self.phase else {
                    return Flow::Continue;
                };
                let notification = document_notification(method, &params)
                    .filter(|notification| notification.validate().is_ok());
                let Some(notification) = notification else {
                    return self.forget_document(&params);
                };
                if !matches!(notification, V5LspNotification::DidOpen { .. }) {
                    // A request about an older version would answer about a
                    // state the editor no longer has.
                    self.withdraw_document(notification.document_uri());
                }
                if !matches!(notification, V5LspNotification::DidChange { .. }) {
                    // Opening sends the whole text again; closing ends it.
                    self.unsynced.remove(notification.document_uri());
                }
                self.forward(notification)
            }
            _ => Flow::Continue,
        }
    }

    fn initialize(&mut self, id: Value, params: &Value) -> Flow {
        let deadline = Instant::now() + LSP_SESSION_EXCHANGE_TIMEOUT;
        let opened = self
            .client
            .connect_peer_before(deadline)
            .map_err(|error| V5LspExchangeError::Transport(error.to_string()))
            .and_then(|mut anchor| {
                let (session, opening) =
                    anchor.lsp_open_before(workspace_folders(params), deadline)?;
                Ok((anchor, session, opening))
            });
        let (anchor, session, opening) = match opened {
            Ok(opened) => opened,
            Err(error) => {
                self.send(error_response(
                    id,
                    REQUEST_FAILED,
                    &exchange_message(&error),
                ));
                return Flow::Continue;
            }
        };
        let poll_stop = Arc::new(AtomicBool::new(false));
        spawn_poller(
            Arc::clone(&self.client),
            session.clone(),
            Arc::clone(&self.output),
            Arc::clone(&poll_stop),
            Arc::clone(&self.session_lost),
        );
        self.phase = Phase::Running(Box::new(Session {
            id: session,
            endpoint: anchor.control_endpoint(),
            anchor,
            poll_stop,
        }));
        self.send(json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "capabilities": capabilities_json(opening),
                "serverInfo": {"name": "unica", "version": env!("CARGO_PKG_VERSION")},
            }
        }));
        Flow::Continue
    }

    fn dispatch(&mut self, id: Value, call: V5LspCall) {
        if call
            .document_uri()
            .is_some_and(|uri| self.unsynced.contains(uri))
        {
            self.send(error_response(
                id,
                CONTENT_MODIFIED,
                "document is out of sync",
            ));
            return;
        }
        let Phase::Running(session) = &self.phase else {
            return;
        };
        let key = request_key(&id);
        self.next_request += 1;
        let request = self.next_request;
        {
            let mut in_flight = lock(&self.in_flight);
            if in_flight.len() >= MAX_LSP_REQUESTS_IN_FLIGHT || in_flight.contains_key(&key) {
                drop(in_flight);
                self.send(error_response(id, REQUEST_FAILED, "unica: lsp_busy"));
                return;
            }
            in_flight.insert(
                key.clone(),
                PendingRequest {
                    id,
                    request,
                    uri: call.document_uri().map(str::to_owned),
                },
            );
        }
        let client = Arc::clone(&self.client);
        let session = session.id.clone();
        let in_flight = Arc::clone(&self.in_flight);
        let output = Arc::clone(&self.output);
        let worker_key = key.clone();
        let spawned = thread::Builder::new()
            .name("unica-lsp-request".into())
            .spawn(move || {
                let answer = call_daemon(&client, &session, request, call);
                if let Some(pending) = lock(&in_flight).remove(&worker_key) {
                    write(&output, &answer(pending.id));
                }
            });
        if spawned.is_err() {
            self.withdraw(&key, REQUEST_FAILED, "unica: lsp_unavailable");
        }
    }

    /// Answer a waiting request with `code` and stop its work in the daemon.
    fn withdraw(&self, key: &str, code: i64, message: &str) {
        let Some(pending) = lock(&self.in_flight).remove(key) else {
            return;
        };
        self.send(error_response(pending.id, code, message));
        self.cancel_in_daemon(pending.request);
    }

    fn withdraw_document(&self, uri: &str) {
        let keys = lock(&self.in_flight)
            .iter()
            .filter(|(_, pending)| pending.uri.as_deref() == Some(uri))
            .map(|(key, _)| key.clone())
            .collect::<Vec<_>>();
        for key in keys {
            self.withdraw(&key, CONTENT_MODIFIED, "document changed");
        }
    }

    fn cancel_in_daemon(&self, request: u64) {
        let Phase::Running(session) = &self.phase else {
            return;
        };
        let (endpoint, id) = (session.endpoint.clone(), session.id.clone());
        let _ = thread::Builder::new()
            .name("unica-lsp-cancel".into())
            .spawn(move || {
                let _ = V5DaemonProcessOwner::lsp_cancel_at_endpoint(endpoint, &id, request);
            });
    }

    /// A document notification the daemon would not take. Its requests are
    /// withdrawn and the daemon drops the document, so no answer is given
    /// about a text the editor no longer has; the session itself stays.
    fn forget_document(&mut self, params: &Value) -> Flow {
        let Some(uri) = document_uri(params) else {
            return Flow::Continue;
        };
        eprintln!("unica lsp ignored a document notification it cannot pass on");
        self.withdraw_document(&uri);
        self.unsynced.insert(uri.clone());
        let close = V5LspNotification::DidClose { uri };
        if close.validate().is_err() {
            return Flow::Continue;
        }
        self.forward(close)
    }

    fn forward(&mut self, notification: V5LspNotification) -> Flow {
        let Phase::Running(session) = &mut self.phase else {
            return Flow::Continue;
        };
        let deadline = Instant::now() + LSP_SESSION_EXCHANGE_TIMEOUT;
        let changed = match &notification {
            V5LspNotification::DidOpen { uri, .. } | V5LspNotification::DidChange { uri, .. } => {
                Some(uri.clone())
            }
            V5LspNotification::DidClose { .. } => None,
        };
        match session
            .anchor
            .lsp_notify_before(&session.id, notification, deadline)
        {
            Ok(()) => Flow::Continue,
            // The daemon refused this document; the session stays. A refused
            // change leaves the daemon's buffer behind the editor's text, so
            // the document is dropped there rather than answered from it.
            Err(V5LspExchangeError::Refused(failure))
                if failure != V5LspFailure::SessionUnknown =>
            {
                // The daemon closed the document on its side.
                if let Some(uri) = changed {
                    self.withdraw_document(&uri);
                    self.unsynced.insert(uri);
                }
                Flow::Continue
            }
            Err(error) => Flow::Fail(exchange_message(&error)),
        }
    }

    fn close_session(&mut self) {
        if !matches!(self.phase, Phase::Running(_)) {
            return;
        }
        let keys = lock(&self.in_flight).keys().cloned().collect::<Vec<_>>();
        for key in keys {
            self.withdraw(&key, REQUEST_CANCELLED, "server is shutting down");
        }
        let Phase::Running(session) = &mut self.phase else {
            return;
        };
        session.poll_stop.store(true, Ordering::Release);
        let deadline = Instant::now() + LSP_SESSION_EXCHANGE_TIMEOUT;
        let _ = session.anchor.lsp_close_before(&session.id, deadline);
    }

    /// Leave the daemon: closing the anchor ends the session there too.
    fn end(&mut self) {
        self.close_session();
        self.phase = Phase::ShutDown;
    }

    fn send(&self, message: Value) {
        write(&self.output, &message);
    }
}

fn call_daemon(
    client: &V5DaemonClient,
    session: &V5LspSessionId,
    request: u64,
    call: V5LspCall,
) -> impl FnOnce(Value) -> Value {
    let deadline = Instant::now() + LSP_CALL_BUDGET + LSP_EXCHANGE_MARGIN;
    let outcome = client
        .connect_peer_before(deadline)
        .map_err(|error| V5LspExchangeError::Transport(error.to_string()))
        .and_then(|mut peer| {
            peer.lsp_call_before(
                session,
                request,
                LSP_CALL_BUDGET.as_millis() as u64,
                call,
                deadline,
            )
        });
    move |id| match outcome {
        Ok(V5LspOutcome::Hover { contents }) => json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": contents.map(|value| json!({"contents": {"kind": "markdown", "value": value}})),
        }),
        Ok(V5LspOutcome::Text { text }) => {
            json!({"jsonrpc": "2.0", "id": id, "result": {"text": text}})
        }
        Ok(V5LspOutcome::Json { value }) => json!({"jsonrpc": "2.0", "id": id, "result": value}),
        Ok(V5LspOutcome::Cancelled {}) => {
            error_response(id, REQUEST_CANCELLED, "request cancelled")
        }
        Ok(V5LspOutcome::Failed {
            failure: V5LspFailure::NotSupported,
        }) => error_response(id, METHOD_NOT_FOUND, "method is not supported"),
        Ok(V5LspOutcome::Failed { failure }) => {
            error_response(id, REQUEST_FAILED, &failure_message(failure))
        }
        Err(error) => error_response(id, REQUEST_FAILED, &exchange_message(&error)),
    }
}

fn spawn_poller(
    client: Arc<V5DaemonClient>,
    session: V5LspSessionId,
    output: Output,
    stop: Arc<AtomicBool>,
    session_lost: SessionLost,
) {
    let _ = thread::Builder::new()
        .name("unica-lsp-notifications".into())
        .spawn(move || {
            let mut peer: Option<V5DaemonProcessOwner> = None;
            let mut failures = 0;
            while !stop.load(Ordering::Acquire) {
                let deadline =
                    Instant::now() + Duration::from_millis(LSP_POLL_WAIT_MS) + LSP_EXCHANGE_MARGIN;
                let polled = match peer.as_mut() {
                    Some(owner) => owner.lsp_poll_before(&session, LSP_POLL_WAIT_MS, deadline),
                    None => client
                        .connect_peer_before(deadline)
                        .map_err(|error| V5LspExchangeError::Transport(error.to_string()))
                        .and_then(|owner| {
                            peer.insert(owner)
                                .lsp_poll_before(&session, LSP_POLL_WAIT_MS, deadline)
                        }),
                };
                if stop.load(Ordering::Acquire) {
                    return;
                }
                match polled {
                    Ok(items) => {
                        failures = 0;
                        for item in items {
                            write(&output, &server_notification_json(item));
                        }
                    }
                    // The daemon answered: this session is gone there.
                    Err(V5LspExchangeError::Refused(_) | V5LspExchangeError::Unexpected) => {
                        return session_lost();
                    }
                    // A lost exchange may be load; a fresh connection decides.
                    Err(V5LspExchangeError::Transport(_)) => {
                        peer = None;
                        failures += 1;
                        if failures >= MAX_LSP_POLL_FAILURES {
                            return session_lost();
                        }
                    }
                }
            }
        });
}

fn write(output: &Output, message: &Value) {
    let mut output = lock(output);
    if codec::write_message(&mut *output, message).is_err() {
        // The editor stopped reading; its input end decides when we exit.
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn request_key(id: &Value) -> String {
    id.to_string()
}

fn error_response(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

fn failure_message(failure: V5LspFailure) -> String {
    format!("unica: {}", failure_code(failure))
}

fn failure_code(failure: V5LspFailure) -> &'static str {
    match failure {
        V5LspFailure::NotSupported => "lsp_not_supported",
        V5LspFailure::SessionUnknown => "lsp_session_unknown",
        V5LspFailure::DuplicateRequest => "lsp_duplicate_request",
        V5LspFailure::Busy => "lsp_busy",
        V5LspFailure::DeadlineExceeded => "lsp_deadline_exceeded",
        V5LspFailure::Unavailable => "lsp_unavailable",
        V5LspFailure::WorkspaceUnavailable => "lsp_workspace_unavailable",
        V5LspFailure::DocumentUnknown => "lsp_document_unknown",
    }
}

fn exchange_message(error: &V5LspExchangeError) -> String {
    match error {
        V5LspExchangeError::Refused(failure) => failure_message(*failure),
        V5LspExchangeError::Transport(_) | V5LspExchangeError::Unexpected => {
            failure_message(V5LspFailure::Unavailable)
        }
    }
}

/// Возможности по готовности службы и пространство каждой рабочей папки
/// в `experimental.unica.workspaces`, в порядке папок `initialize`.
fn capabilities_json(opening: V5LspOpening) -> Value {
    let capabilities = opening.capabilities;
    let mut result = Map::new();
    result.insert("positionEncoding".into(), json!("utf-16"));
    let sync = match capabilities.text_document_sync {
        V5LspTextSync::None => None,
        V5LspTextSync::Full => Some(1),
        V5LspTextSync::Incremental => Some(2),
    };
    if let Some(change) = sync {
        result.insert(
            "textDocumentSync".into(),
            json!({"openClose": true, "change": change}),
        );
    }
    if capabilities.hover {
        result.insert("hoverProvider".into(), json!(true));
    }
    if capabilities.text_document_content {
        result.insert(
            "workspace".into(),
            json!({"textDocumentContent": {"schemes": ["unica"]}}),
        );
    }
    if capabilities.document_symbols {
        result.insert("documentSymbolProvider".into(), json!(true));
    }
    if capabilities.folding_ranges {
        result.insert("foldingRangeProvider".into(), json!(true));
    }
    if let Some(legend) = capabilities.semantic_tokens_legend {
        result.insert(
            "semanticTokensProvider".into(),
            json!({"legend": legend, "full": true}),
        );
    }
    if capabilities.completion {
        result.insert(
            "completionProvider".into(),
            json!({"triggerCharacters": ["."], "resolveProvider": true}),
        );
    }
    if capabilities.signature_help {
        result.insert(
            "signatureHelpProvider".into(),
            json!({"triggerCharacters": ["(", ","]}),
        );
    }
    let workspaces = opening
        .workspaces
        .into_iter()
        .map(|workspace| match (workspace.space, workspace.failure) {
            (Some(space), _) => json!({"folder": workspace.folder, "space": space}),
            (None, failure) => json!({
                "folder": workspace.folder,
                "failure": failure_code(failure.unwrap_or(V5LspFailure::Unavailable)),
            }),
        })
        .collect::<Vec<_>>();
    result.insert(
        "experimental".into(),
        json!({"unica": {"workspaces": workspaces}}),
    );
    Value::Object(result)
}

fn server_notification_json(notification: V5LspServerNotification) -> Value {
    match notification {
        V5LspServerNotification::LogMessage { level, message } => {
            let kind = match level {
                V5LspMessageLevel::Error => 1,
                V5LspMessageLevel::Warning => 2,
                V5LspMessageLevel::Info => 3,
            };
            json!({
                "jsonrpc": "2.0",
                "method": "window/logMessage",
                "params": {"type": kind, "message": message},
            })
        }
    }
}

fn workspace_folders(params: &Value) -> Vec<V5LspWorkspaceFolder> {
    if let Some(folders) = params.get("workspaceFolders").and_then(Value::as_array) {
        return folders
            .iter()
            .filter_map(|folder| {
                Some(V5LspWorkspaceFolder {
                    uri: folder.get("uri")?.as_str()?.to_owned(),
                    name: folder
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                })
            })
            .collect();
    }
    // An editor without workspace folders names one root.
    params
        .get("rootUri")
        .and_then(Value::as_str)
        .map(|uri| {
            vec![V5LspWorkspaceFolder {
                uri: uri.to_owned(),
                name: uri.rsplit('/').next().unwrap_or_default().to_owned(),
            }]
        })
        .unwrap_or_default()
}

fn document_uri(params: &Value) -> Option<String> {
    Some(params.get("textDocument")?.get("uri")?.as_str()?.to_owned())
}

fn position(value: &Value) -> Option<V5LspPosition> {
    Some(V5LspPosition {
        line: u32::try_from(value.get("line")?.as_u64()?).ok()?,
        character: u32::try_from(value.get("character")?.as_u64()?).ok()?,
    })
}

/// A request about one document. `None` — not such a method; `Some(None)`
/// — the method, with params it cannot be built from.
fn document_call(method: &str, params: &Value) -> Option<Option<V5LspCall>> {
    let uri = || document_uri(params);
    Some(match method {
        "textDocument/documentSymbol" => uri().map(|uri| V5LspCall::DocumentSymbol { uri }),
        "textDocument/foldingRange" => uri().map(|uri| V5LspCall::FoldingRange { uri }),
        "textDocument/semanticTokens/full" => {
            uri().map(|uri| V5LspCall::SemanticTokensFull { uri })
        }
        "textDocument/signatureHelp" => uri().and_then(|uri| {
            Some(V5LspCall::SignatureHelp {
                uri,
                position: position(params.get("position")?)?,
            })
        }),
        "textDocument/completion" => uri().and_then(|uri| {
            Some(V5LspCall::Completion {
                uri,
                position: position(params.get("position")?)?,
            })
        }),
        // The params are the item the service gave, as the editor returns it.
        "completionItem/resolve" => params.is_object().then(|| V5LspCall::CompletionResolve {
            item: params.clone(),
        }),
        _ => return None,
    })
}

fn hover_call(params: &Value) -> Option<V5LspCall> {
    Some(V5LspCall::Hover {
        uri: document_uri(params)?,
        position: position(params.get("position")?)?,
    })
}

fn document_notification(method: &str, params: &Value) -> Option<V5LspNotification> {
    let document = params.get("textDocument")?;
    let uri = document.get("uri")?.as_str()?.to_owned();
    let version = || i32::try_from(document.get("version")?.as_i64()?).ok();
    match method {
        "textDocument/didOpen" => Some(V5LspNotification::DidOpen {
            uri,
            language_id: document.get("languageId")?.as_str()?.to_owned(),
            version: version()?,
            text: document.get("text")?.as_str()?.to_owned(),
        }),
        "textDocument/didChange" => Some(V5LspNotification::DidChange {
            uri,
            version: version()?,
            changes: params
                .get("contentChanges")?
                .as_array()?
                .iter()
                .map(|change| {
                    let range = match change.get("range") {
                        Some(range) => Some(V5LspRange {
                            start: position(range.get("start")?)?,
                            end: position(range.get("end")?)?,
                        }),
                        None => None,
                    };
                    Some(V5LspContentChange {
                        range,
                        text: change.get("text")?.as_str()?.to_owned(),
                    })
                })
                .collect::<Option<Vec<_>>>()?,
        }),
        "textDocument/didClose" => Some(V5LspNotification::DidClose { uri }),
        _ => None,
    }
}
