use crate::application::{input_schema_for_tool, ToolSpec, UnicaApplication};
use crate::domain::cancellation::CancellationToken;
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::io::{self, BufRead, Write};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const PROTOCOL_VERSION: &str = "2024-11-05";
const EOF_DRAIN_GRACE: Duration = Duration::from_millis(250);
const EOF_CANCELLATION_GRACE: Duration = Duration::from_secs(2);
const MCP_INPUT_LINE_LIMIT: usize = 8 * 1024 * 1024;
const MCP_MAX_TOOL_WORKERS: usize = 32;

pub fn run_stdio() {
    let stdin = io::stdin();
    run_stdio_with(
        stdin.lock(),
        io::stdout(),
        Arc::new(UnicaApplication::new()),
    );
}

pub fn run_stdio_with<R, W>(reader: R, writer: W, app: Arc<UnicaApplication>)
where
    R: BufRead,
    W: Write + Send + 'static,
{
    let tool_app = Arc::clone(&app);
    let handler: Arc<ToolCallHandler> = Arc::new(move |name, arguments, cancellation| {
        call_tool_cancellable(&tool_app, name, arguments, cancellation)
    });
    run_stdio_with_handler(reader, writer, app, handler);
}

type ToolCallHandler = dyn Fn(&str, &Map<String, Value>, CancellationToken) -> Result<String, (i64, String)>
    + Send
    + Sync;

fn run_stdio_with_handler<R, W>(
    mut reader: R,
    writer: W,
    app: Arc<UnicaApplication>,
    handler: Arc<ToolCallHandler>,
) where
    R: BufRead,
    W: Write + Send + 'static,
{
    let registry = CancellationRegistry::default();
    let writer = Arc::new(Mutex::new(writer));

    loop {
        let line = match read_bounded_line(&mut reader, MCP_INPUT_LINE_LIMIT) {
            Ok(Some(BoundedLine::Line(line))) if !line.iter().all(u8::is_ascii_whitespace) => line,
            Ok(Some(BoundedLine::Line(_))) => continue,
            Ok(Some(BoundedLine::TooLarge)) => {
                if !registry.publish(
                    &writer,
                    error_response(
                        Value::Null,
                        -32700,
                        "parse error: input line exceeds 8388608 bytes",
                    ),
                ) {
                    break;
                }
                continue;
            }
            Ok(None) => break,
            Err(err) => {
                let _ = writeln!(io::stderr(), "failed to read stdin: {err}");
                break;
            }
        };

        let message = match serde_json::from_slice::<Value>(&line) {
            Ok(message) => message,
            Err(err) => {
                if !registry.publish(
                    &writer,
                    error_response(Value::Null, -32700, &format!("parse error: {err}")),
                ) {
                    break;
                }
                continue;
            }
        };
        let method = message.get("method").and_then(Value::as_str).unwrap_or("");

        if method == "notifications/cancelled" {
            if let Some(id) = message.pointer("/params/requestId") {
                registry.cancel(id);
            }
            continue;
        }

        if method == "tools/call" {
            dispatch_tool_call(
                message,
                Arc::clone(&handler),
                registry.clone(),
                Arc::clone(&writer),
            );
            continue;
        }

        if let Some(response) = handle_message(&app, message) {
            if !registry.publish(&writer, response) {
                break;
            }
        }
    }

    let drained = registry.wait_for_idle(EOF_DRAIN_GRACE);
    registry.cancel_all();
    if !drained {
        registry.wait_for_idle(EOF_CANCELLATION_GRACE);
    }
    registry.close();
}

enum BoundedLine {
    Line(Vec<u8>),
    TooLarge,
}

fn read_bounded_line<R: BufRead>(reader: &mut R, limit: usize) -> io::Result<Option<BoundedLine>> {
    let mut line = Vec::new();
    let mut too_large = false;
    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            return if line.is_empty() && !too_large {
                Ok(None)
            } else if too_large {
                Ok(Some(BoundedLine::TooLarge))
            } else {
                Ok(Some(BoundedLine::Line(line)))
            };
        }
        let newline = available.iter().position(|byte| *byte == b'\n');
        let consumed = newline.map_or(available.len(), |index| index + 1);
        let content_len = newline.unwrap_or(available.len());
        if !too_large {
            if line.len().saturating_add(content_len) > limit {
                too_large = true;
                line.clear();
            } else {
                line.extend_from_slice(&available[..content_len]);
            }
        }
        reader.consume(consumed);
        if newline.is_some() {
            return Ok(Some(if too_large {
                BoundedLine::TooLarge
            } else {
                BoundedLine::Line(line)
            }));
        }
    }
}

fn dispatch_tool_call<W: Write + Send + 'static>(
    message: Value,
    handler: Arc<ToolCallHandler>,
    registry: CancellationRegistry,
    writer: Arc<Mutex<W>>,
) -> bool {
    let id = message.get("id").cloned().unwrap_or(Value::Null);
    let cancellation = match registry.register(&id) {
        Ok(cancellation) => cancellation,
        Err(message) => {
            registry.publish(&writer, error_response(id, -32600, &message));
            return false;
        }
    };
    if let Err(message) = registry.worker_started() {
        registry.finish(&id);
        registry.publish(&writer, error_response(id, -32603, &message));
        return false;
    }

    let failure_id = id.clone();
    let failure_registry = registry.clone();
    let failure_writer = Arc::clone(&writer);
    let spawn = thread::Builder::new()
        .name("unica-mcp-tool".into())
        .spawn(move || {
            let _worker = WorkerCompletionGuard(registry.clone());
            let mut completion = RegistryCompletionGuard::new(registry.clone(), id.clone());
            let result = match tool_call_params(&message) {
                Ok((name, arguments)) => handler(&name, &arguments, cancellation.clone()),
                Err(error) => Err(error),
            };
            let cancelled = completion.finish();
            let response = if cancelled {
                error_response(id.clone(), -32800, "request cancelled")
            } else {
                match result {
                    Ok(result) => success_response(
                        id.clone(),
                        json!({ "content": [{ "type": "text", "text": result }] }),
                    ),
                    Err((code, message)) => error_response(id.clone(), code, &message),
                }
            };

            registry.publish(&writer, response);
        });
    if let Err(error) = spawn {
        failure_registry.worker_finished();
        failure_registry.finish(&failure_id);
        failure_registry.publish(
            &failure_writer,
            error_response(
                failure_id,
                -32603,
                &format!("dispatcher unavailable: failed to spawn tool worker: {error}"),
            ),
        );
        return false;
    }
    true
}

fn write_response<W: Write>(writer: &Arc<Mutex<W>>, response: Value) -> bool {
    let Ok(mut writer) = writer.lock() else {
        return false;
    };
    writeln!(writer, "{response}").is_ok() && writer.flush().is_ok()
}

#[derive(Clone, Default)]
pub struct CancellationRegistry {
    state: Arc<Mutex<CancellationRegistryState>>,
    activity: Arc<(Mutex<DispatcherActivity>, Condvar)>,
}

#[derive(Default)]
struct CancellationRegistryState {
    requests: HashMap<String, CancellationToken>,
    failed: bool,
}

#[derive(Default)]
struct DispatcherActivity {
    handlers: usize,
    publications: usize,
    closed: bool,
}

impl CancellationRegistry {
    pub fn register(&self, id: &Value) -> Result<CancellationToken, String> {
        let key = request_id_key(id)?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| "cancellation registry lock poisoned".to_string())?;
        if state.failed {
            return Err("dispatcher unavailable: response writer failed".to_string());
        }
        if state.requests.contains_key(&key) {
            return Err(format!("duplicate request id: {id}"));
        }
        let cancellation = CancellationToken::new();
        state.requests.insert(key, cancellation.clone());
        Ok(cancellation)
    }

    pub fn cancel(&self, id: &Value) -> bool {
        let Ok(key) = request_id_key(id) else {
            return false;
        };
        let Ok(state) = self.state.lock() else {
            return false;
        };
        if let Some(cancellation) = state.requests.get(&key) {
            cancellation.cancel();
            true
        } else {
            false
        }
    }

    pub fn finish(&self, id: &Value) -> bool {
        let Ok(key) = request_id_key(id) else {
            return false;
        };
        self.state
            .lock()
            .ok()
            .and_then(|mut state| state.requests.remove(&key))
            .is_some_and(|cancellation| cancellation.is_cancelled())
    }

    pub fn cancel_all(&self) {
        if let Ok(state) = self.state.lock() {
            for cancellation in state.requests.values() {
                cancellation.cancel();
            }
        }
    }

    fn worker_started(&self) -> Result<(), String> {
        let mut activity = self
            .activity
            .0
            .lock()
            .map_err(|_| "dispatcher activity lock poisoned".to_string())?;
        if activity.closed {
            return Err("dispatcher unavailable: publication gate closed".to_string());
        }
        if activity.handlers >= MCP_MAX_TOOL_WORKERS {
            return Err(format!("dispatcher overloaded: at most {MCP_MAX_TOOL_WORKERS} concurrent tools/call requests are allowed"));
        }
        activity.handlers += 1;
        Ok(())
    }

    fn worker_finished(&self) {
        let (activity, changed) = &*self.activity;
        if let Ok(mut activity) = activity.lock() {
            activity.handlers = activity.handlers.saturating_sub(1);
            changed.notify_all();
        }
    }

    fn publication_finished(&self) {
        let (activity, changed) = &*self.activity;
        if let Ok(mut activity) = activity.lock() {
            activity.publications = activity.publications.saturating_sub(1);
            changed.notify_all();
        }
    }

    fn wait_for_idle(&self, timeout: Duration) -> bool {
        let (activity, changed) = &*self.activity;
        let Ok(mut activity) = activity.lock() else {
            return false;
        };
        let deadline = Instant::now() + timeout;
        while activity.handlers != 0 || activity.publications != 0 {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return false;
            }
            let Ok((next, result)) = changed.wait_timeout(activity, remaining) else {
                return false;
            };
            activity = next;
            if result.timed_out() && (activity.handlers != 0 || activity.publications != 0) {
                return false;
            }
        }
        true
    }

    fn publish<W: Write>(&self, writer: &Arc<Mutex<W>>, response: Value) -> bool {
        {
            let Ok(mut activity) = self.activity.0.lock() else {
                return false;
            };
            if activity.closed {
                return false;
            }
            activity.publications += 1;
        }
        let _publication = PublicationCompletionGuard(self.clone());
        if write_response(writer, response) {
            return true;
        }
        self.fail();
        false
    }

    fn close(&self) {
        let (activity, changed) = &*self.activity;
        if let Ok(mut activity) = activity.lock() {
            activity.closed = true;
            changed.notify_all();
        }
        self.fail_state();
    }

    fn fail(&self) {
        self.close();
    }

    fn fail_state(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.failed = true;
            for cancellation in state.requests.values() {
                cancellation.cancel();
            }
        }
    }

    #[cfg(test)]
    fn is_failed(&self) -> bool {
        self.state.lock().map(|state| state.failed).unwrap_or(true)
    }
}

fn request_id_key(id: &Value) -> Result<String, String> {
    serde_json::to_string(id).map_err(|err| format!("invalid request id: {err}"))
}

struct RegistryCompletionGuard {
    registry: CancellationRegistry,
    id: Value,
    finished: bool,
}

struct WorkerCompletionGuard(CancellationRegistry);

impl Drop for WorkerCompletionGuard {
    fn drop(&mut self) {
        self.0.worker_finished();
    }
}

struct PublicationCompletionGuard(CancellationRegistry);

impl Drop for PublicationCompletionGuard {
    fn drop(&mut self) {
        self.0.publication_finished();
    }
}

impl RegistryCompletionGuard {
    fn new(registry: CancellationRegistry, id: Value) -> Self {
        Self {
            registry,
            id,
            finished: false,
        }
    }

    fn finish(&mut self) -> bool {
        let cancelled = self.registry.finish(&self.id);
        self.finished = true;
        cancelled
    }
}

impl Drop for RegistryCompletionGuard {
    fn drop(&mut self) {
        if !self.finished {
            self.registry.finish(&self.id);
        }
    }
}

pub fn handle_message(app: &UnicaApplication, message: Value) -> Option<Value> {
    let id = message.get("id").cloned().unwrap_or(Value::Null);
    let method = message.get("method").and_then(Value::as_str).unwrap_or("");

    if method.starts_with("notifications/") {
        return None;
    }

    match method {
        "initialize" => Some(success_response(
            id,
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {
                    "tools": { "listChanged": false }
                },
                "serverInfo": {
                    "name": "unica",
                    "version": env!("CARGO_PKG_VERSION")
                }
            }),
        )),
        "ping" => Some(success_response(id, json!({}))),
        "tools/list" => Some(success_response(
            id,
            json!({ "tools": list_tools(app.tools()) }),
        )),
        "tools/call" => Some(match call_tool_from_message(app, &message) {
            Ok(result) => success_response(
                id,
                json!({ "content": [{ "type": "text", "text": result }] }),
            ),
            Err((code, msg)) => error_response(id, code, &msg),
        }),
        _ => Some(error_response(
            id,
            -32601,
            &format!("method not found: {method}"),
        )),
    }
}

fn list_tools(tools: Vec<ToolSpec>) -> Vec<Value> {
    tools
        .iter()
        .map(|tool| {
            json!({
                "name": tool.name,
                "description": tool.description,
                "inputSchema": input_schema_for_tool(tool)
            })
        })
        .collect()
}

fn call_tool_from_message(
    app: &UnicaApplication,
    message: &Value,
) -> Result<String, (i64, String)> {
    let (name, args) = tool_call_params(message)?;
    call_tool(app, &name, &args)
}

fn tool_call_params(message: &Value) -> Result<(String, Map<String, Value>), (i64, String)> {
    let params = message
        .get("params")
        .ok_or((-32602, "missing params".to_string()))?;
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .ok_or((-32602, "missing tool name".to_string()))?;
    let args = params
        .get("arguments")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    Ok((name.to_string(), args))
}

fn call_tool(
    app: &UnicaApplication,
    name: &str,
    args: &Map<String, Value>,
) -> Result<String, (i64, String)> {
    let result = app.call_tool(name, args).map_err(|msg| (-32000, msg))?;
    serde_json::to_string_pretty(&result).map_err(|err| (-32603, err.to_string()))
}

fn call_tool_cancellable(
    app: &UnicaApplication,
    name: &str,
    args: &Map<String, Value>,
    cancellation: CancellationToken,
) -> Result<String, (i64, String)> {
    let result = app
        .call_tool_cancellable(name, args, cancellation)
        .map_err(|msg| (-32000, msg))?;
    serde_json::to_string_pretty(&result).map_err(|err| (-32603, err.to_string()))
}

fn success_response(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn error_response(id: Value, code: i64, message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {
            "code": code,
            "message": message
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::cancellation::CancellationToken;
    use serde_json::Map;
    use std::io::{BufReader, Read};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{mpsc, Arc, Mutex};
    use std::thread;
    use std::time::{Duration, Instant};

    struct ChannelReader {
        receiver: mpsc::Receiver<Vec<u8>>,
        pending: Vec<u8>,
    }

    impl ChannelReader {
        fn new(receiver: mpsc::Receiver<Vec<u8>>) -> Self {
            Self {
                receiver,
                pending: Vec::new(),
            }
        }
    }

    impl Read for ChannelReader {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            while self.pending.is_empty() {
                match self.receiver.recv() {
                    Ok(bytes) => self.pending = bytes,
                    Err(_) => return Ok(0),
                }
            }
            let count = buffer.len().min(self.pending.len());
            buffer[..count].copy_from_slice(&self.pending[..count]);
            self.pending.drain(..count);
            Ok(count)
        }
    }

    #[derive(Clone, Default)]
    struct SharedWriter(Arc<Mutex<Vec<u8>>>);

    impl Write for SharedWriter {
        fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buffer);
            Ok(buffer.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[derive(Clone, Default)]
    struct BlockingWriter {
        bytes: Arc<Mutex<Vec<u8>>>,
        entered: Arc<AtomicBool>,
        release: Arc<AtomicBool>,
        completed: Arc<(Mutex<bool>, Condvar)>,
    }

    impl Write for BlockingWriter {
        fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
            if !self.entered.swap(true, Ordering::SeqCst) {
                while !self.release.load(Ordering::SeqCst) {
                    thread::yield_now();
                }
            }
            self.bytes.lock().unwrap().extend_from_slice(buffer);
            Ok(buffer.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl BlockingWriter {
        fn wait_for_completion(&self, timeout: Duration) -> bool {
            let (completed, changed) = &*self.completed;
            let completed = completed.lock().unwrap();
            let (completed, _) = changed
                .wait_timeout_while(completed, timeout, |completed| !*completed)
                .unwrap();
            *completed
        }
    }

    impl Drop for BlockingWriter {
        fn drop(&mut self) {
            let (completed, changed) = &*self.completed;
            *completed.lock().unwrap() = true;
            changed.notify_all();
        }
    }

    #[derive(Clone, Default)]
    struct FailingWriter(Arc<AtomicBool>);

    impl Write for FailingWriter {
        fn write(&mut self, _buffer: &[u8]) -> io::Result<usize> {
            self.0.store(true, Ordering::SeqCst);
            Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "test writer failed",
            ))
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl SharedWriter {
        fn responses(&self) -> Vec<Value> {
            String::from_utf8(self.0.lock().unwrap().clone())
                .unwrap()
                .lines()
                .filter_map(|line| serde_json::from_str(line).ok())
                .collect()
        }

        fn wait_for_responses(&self, count: usize) -> Vec<Value> {
            let deadline = Instant::now() + Duration::from_secs(2);
            loop {
                let responses = self.responses();
                if responses.len() >= count {
                    return responses;
                }
                assert!(
                    Instant::now() < deadline,
                    "timed out waiting for {count} responses"
                );
                thread::sleep(Duration::from_millis(10));
            }
        }
    }

    fn send_message(sender: &mpsc::Sender<Vec<u8>>, message: Value) {
        sender.send(format!("{message}\n").into_bytes()).unwrap();
    }

    #[test]
    fn initialize_uses_single_public_server_name() {
        let app = UnicaApplication::new();
        let request = json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize" });
        let response = handle_message(&app, request).unwrap();
        assert_eq!(response["result"]["serverInfo"]["name"], "unica");
    }

    #[test]
    fn mcp_rejects_oversized_jsonl_without_allocating_the_line() {
        let mut input = vec![b'x'; MCP_INPUT_LINE_LIMIT + 1];
        input.push(b'\n');
        input.extend_from_slice(b"{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"ping\"}\n");
        let writer = SharedWriter::default();
        let output = writer.clone();
        run_stdio_with(
            BufReader::new(std::io::Cursor::new(input)),
            writer,
            Arc::new(UnicaApplication::new()),
        );
        let responses = output.responses();
        assert_eq!(responses.len(), 2);
        assert_eq!(responses[0]["error"]["code"], -32700);
        assert_eq!(responses[1]["id"], 2);
    }

    #[test]
    fn mcp_worker_admission_is_bounded_and_reusable() {
        let registry = CancellationRegistry::default();
        for _ in 0..MCP_MAX_TOOL_WORKERS {
            registry.worker_started().unwrap();
        }
        assert!(registry
            .worker_started()
            .unwrap_err()
            .contains("overloaded"));
        registry.worker_finished();
        registry.worker_started().unwrap();
        for _ in 0..MCP_MAX_TOOL_WORKERS {
            registry.worker_finished();
        }
    }

    #[test]
    fn mcp_overload_has_deterministic_json_rpc_error() {
        let registry = CancellationRegistry::default();
        for _ in 0..MCP_MAX_TOOL_WORKERS {
            registry.worker_started().unwrap();
        }
        let writer = SharedWriter::default();
        let output = writer.clone();
        let dispatched = dispatch_tool_call(
            json!({"id":"overload","params":{"name":"unica.code.search","arguments":{}}}),
            Arc::new(|_, _, _| Ok("must not run".into())),
            registry.clone(),
            Arc::new(Mutex::new(writer)),
        );
        assert!(!dispatched);
        let responses = output.responses();
        assert_eq!(responses[0]["id"], "overload");
        assert_eq!(responses[0]["error"]["code"], -32603);
        assert!(responses[0]["error"]["message"]
            .as_str()
            .unwrap()
            .contains("overloaded"));
        for _ in 0..MCP_MAX_TOOL_WORKERS {
            registry.worker_finished();
        }
    }

    #[test]
    fn tools_list_contains_orchestrated_tool_names() {
        let app = UnicaApplication::new();
        let request = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" });
        let response = handle_message(&app, request).unwrap();
        let listed = response["result"]["tools"].as_array().unwrap();
        assert_eq!(listed[0]["name"], "unica.cf.edit");
        assert!(listed
            .iter()
            .any(|tool| tool["name"] == "unica.project.status"));
        assert!(listed
            .iter()
            .any(|tool| tool["name"] == "unica.project.map"));
        assert!(listed
            .iter()
            .any(|tool| tool["name"] == "unica.standards.explain"));
        for name in [
            "unica.runtime.job.start",
            "unica.runtime.job.status",
            "unica.runtime.job.wait",
            "unica.runtime.job.logs",
            "unica.runtime.job.cancel",
            "unica.runtime.job.list",
        ] {
            assert!(
                listed.iter().any(|tool| tool["name"] == name),
                "missing {name}"
            );
        }
    }

    #[test]
    fn native_tool_schema_is_contract_specific_and_does_not_expose_raw_args() {
        let app = UnicaApplication::new();
        let request = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" });
        let response = handle_message(&app, request).unwrap();
        let listed = response["result"]["tools"].as_array().unwrap();
        let cf_info = listed
            .iter()
            .find(|tool| tool["name"] == "unica.cf.info")
            .expect("unica.cf.info must be listed");

        let schema = &cf_info["inputSchema"];
        assert_eq!(schema["additionalProperties"], false);
        assert!(schema["properties"].get("ConfigPath").is_some());
        assert!(schema["properties"].get("cwd").is_some());
        assert!(schema["properties"].get("dryRun").is_some());
        assert!(schema["properties"].get("args").is_none());
    }

    #[test]
    fn no_public_tool_schema_exposes_raw_adapter_args() {
        let app = UnicaApplication::new();
        let request = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" });
        let response = handle_message(&app, request).unwrap();
        let listed = response["result"]["tools"].as_array().unwrap();

        for tool in listed {
            let properties = &tool["inputSchema"]["properties"];
            assert!(
                properties.get("args").is_none(),
                "{} must not expose raw adapter args",
                tool["name"]
            );
        }
    }

    #[test]
    fn mcp_dispatcher_keeps_ping_responsive_and_cancels_the_requested_call() {
        let (sender, receiver) = mpsc::channel();
        let writer = SharedWriter::default();
        let output = writer.clone();
        let cancellation_seen = Arc::new(AtomicBool::new(false));
        let seen = Arc::clone(&cancellation_seen);
        let handler = Arc::new(
            move |_name: &str, _arguments: &Map<String, Value>, cancellation: CancellationToken| {
                while !cancellation.is_cancelled() {
                    thread::sleep(Duration::from_millis(5));
                }
                seen.store(true, Ordering::SeqCst);
                Ok("unreachable success".to_string())
            },
        );
        let dispatcher = thread::spawn(move || {
            run_stdio_with_handler(
                BufReader::new(ChannelReader::new(receiver)),
                writer,
                Arc::new(UnicaApplication::new()),
                handler,
            )
        });

        send_message(
            &sender,
            json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize" }),
        );
        send_message(
            &sender,
            json!({ "jsonrpc": "2.0", "id": 7, "method": "tools/call", "params": { "name": "unica.code.search", "arguments": {} } }),
        );
        send_message(
            &sender,
            json!({ "jsonrpc": "2.0", "id": 8, "method": "ping" }),
        );

        let first = output.wait_for_responses(2);
        assert_eq!(first[0]["id"], 1);
        assert_eq!(first[1]["id"], 8, "ping must not wait for tools/call");
        assert!(!cancellation_seen.load(Ordering::SeqCst));

        send_message(
            &sender,
            json!({ "jsonrpc": "2.0", "method": "notifications/cancelled", "params": { "requestId": 7, "reason": "test" } }),
        );
        let responses = output.wait_for_responses(3);
        assert_eq!(responses[2]["id"], 7);
        assert_eq!(responses[2]["error"]["code"], -32800);
        assert_eq!(responses[2]["error"]["message"], "request cancelled");
        assert!(cancellation_seen.load(Ordering::SeqCst));

        drop(sender);
        dispatcher.join().unwrap();
    }

    #[test]
    fn mcp_dispatcher_cancels_active_calls_on_eof() {
        let (sender, receiver) = mpsc::channel();
        let writer = SharedWriter::default();
        let output = writer.clone();
        let cancellation_seen = Arc::new(AtomicBool::new(false));
        let seen = Arc::clone(&cancellation_seen);
        let handler = Arc::new(
            move |_name: &str, _arguments: &Map<String, Value>, cancellation: CancellationToken| {
                while !cancellation.is_cancelled() {
                    thread::sleep(Duration::from_millis(5));
                }
                seen.store(true, Ordering::SeqCst);
                Ok("unreachable success".to_string())
            },
        );
        let dispatcher = thread::spawn(move || {
            run_stdio_with_handler(
                BufReader::new(ChannelReader::new(receiver)),
                writer,
                Arc::new(UnicaApplication::new()),
                handler,
            )
        });

        send_message(
            &sender,
            json!({ "jsonrpc": "2.0", "id": "work", "method": "tools/call", "params": { "name": "unica.code.search", "arguments": {} } }),
        );
        drop(sender);
        dispatcher.join().unwrap();

        let deadline = Instant::now() + Duration::from_secs(2);
        while !cancellation_seen.load(Ordering::SeqCst) {
            assert!(
                Instant::now() < deadline,
                "active call did not observe EOF cancellation"
            );
            thread::sleep(Duration::from_millis(5));
        }
        let responses = output.responses();
        assert!(
            responses.len() <= 1,
            "a request may emit at most one response"
        );
    }

    #[test]
    fn mcp_dispatcher_drains_a_finishing_call_before_eof_cancellation() {
        let (sender, receiver) = mpsc::channel();
        let writer = SharedWriter::default();
        let output = writer.clone();
        let handler = Arc::new(
            move |_name: &str,
                  _arguments: &Map<String, Value>,
                  _cancellation: CancellationToken| {
                thread::sleep(Duration::from_millis(50));
                Ok("completed before EOF grace expired".to_string())
            },
        );
        let dispatcher = thread::spawn(move || {
            run_stdio_with_handler(
                BufReader::new(ChannelReader::new(receiver)),
                writer,
                Arc::new(UnicaApplication::new()),
                handler,
            )
        });

        send_message(
            &sender,
            json!({ "jsonrpc": "2.0", "id": "finite", "method": "tools/call", "params": { "name": "unica.code.search", "arguments": {} } }),
        );
        drop(sender);
        dispatcher.join().unwrap();

        let responses = output.responses();
        assert_eq!(responses.len(), 1, "accepted call must publish before exit");
        assert_eq!(responses[0]["id"], "finite");
        assert_eq!(
            responses[0]["result"]["content"][0]["text"],
            "completed before EOF grace expired"
        );
    }

    #[test]
    fn mcp_dispatcher_closes_publication_after_bounded_eof_cancellation() {
        let (sender, receiver) = mpsc::channel();
        let writer = SharedWriter::default();
        let output = writer.clone();
        let release = Arc::new((Mutex::new(false), Condvar::new()));
        let release_for_handler = Arc::clone(&release);
        let finished = Arc::new(AtomicBool::new(false));
        let finished_for_handler = Arc::clone(&finished);
        let handler = Arc::new(
            move |_: &str, _: &Map<String, Value>, _: CancellationToken| {
                let (released, changed) = &*release_for_handler;
                let mut released = released.lock().unwrap();
                while !*released {
                    released = changed.wait(released).unwrap();
                }
                finished_for_handler.store(true, Ordering::SeqCst);
                Ok("late result must not publish".to_string())
            },
        );
        let dispatcher = thread::spawn(move || {
            run_stdio_with_handler(
                BufReader::new(ChannelReader::new(receiver)),
                writer,
                Arc::new(UnicaApplication::new()),
                handler,
            )
        });

        send_message(
            &sender,
            json!({ "jsonrpc": "2.0", "id": "stuck", "method": "tools/call", "params": { "name": "unica.code.search", "arguments": {} } }),
        );
        drop(sender);
        let started = Instant::now();
        dispatcher.join().unwrap();
        assert!(started.elapsed() < Duration::from_secs(3));
        assert!(output.responses().is_empty());

        let (released, changed) = &*release;
        *released.lock().unwrap() = true;
        changed.notify_all();
        let deadline = Instant::now() + Duration::from_secs(1);
        while !finished.load(Ordering::SeqCst) {
            assert!(Instant::now() < deadline, "late worker did not finish");
            thread::yield_now();
        }
        thread::sleep(Duration::from_millis(20));
        assert!(
            output.responses().is_empty(),
            "late response escaped terminal gate"
        );
    }

    #[test]
    fn mcp_dispatcher_close_does_not_wait_for_an_admitted_blocking_writer() {
        let (sender, receiver) = mpsc::channel();
        let writer = BlockingWriter::default();
        let entered = Arc::clone(&writer.entered);
        let release = Arc::clone(&writer.release);
        let output = writer.clone();
        let (done_sender, done_receiver) = mpsc::channel();
        let dispatcher = thread::spawn(move || {
            run_stdio_with_handler(
                BufReader::new(ChannelReader::new(receiver)),
                writer,
                Arc::new(UnicaApplication::new()),
                Arc::new(|_, _, _| Ok("admitted response".to_string())),
            );
            done_sender.send(()).unwrap();
        });

        send_message(
            &sender,
            json!({ "jsonrpc": "2.0", "id": "admitted", "method": "tools/call", "params": { "name": "unica.code.search", "arguments": {} } }),
        );
        let entered_deadline = Instant::now() + Duration::from_secs(1);
        while !entered.load(Ordering::SeqCst) {
            assert!(Instant::now() < entered_deadline, "writer was not admitted");
            thread::yield_now();
        }
        drop(sender);

        let returned_bounded = done_receiver.recv_timeout(Duration::from_secs(3)).is_ok();
        release.store(true, Ordering::SeqCst);
        let publication_completed = output.wait_for_completion(Duration::from_secs(1));
        dispatcher.join().unwrap();
        assert!(
            returned_bounded,
            "terminal close waited for blocking writer I/O"
        );
        assert!(
            publication_completed,
            "admitted publication did not finish after writer release"
        );

        let responses = String::from_utf8(output.bytes.lock().unwrap().clone())
            .unwrap()
            .lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .collect::<Vec<_>>();
        assert_eq!(responses.len(), 1, "the one admitted write may complete");
        assert_eq!(responses[0]["id"], "admitted");
    }

    #[test]
    fn terminal_registry_rejects_new_work_and_publication() {
        let registry = CancellationRegistry::default();
        let writer = SharedWriter::default();
        let output = writer.clone();
        let writer = Arc::new(Mutex::new(writer));

        registry.close();

        assert!(registry.register(&json!("late")).is_err());
        assert!(!registry.publish(&writer, success_response(json!("late"), json!({}))));
        assert!(output.responses().is_empty());
    }

    #[test]
    fn mcp_dispatcher_registry_keeps_numeric_and_string_ids_distinct() {
        let registry = CancellationRegistry::default();
        let numeric = registry.register(&json!(7)).unwrap();
        let string = registry.register(&json!("7")).unwrap();

        assert!(registry.cancel(&json!(7)));
        assert!(numeric.is_cancelled());
        assert!(!string.is_cancelled());
    }

    #[test]
    fn mcp_dispatcher_worker_panic_releases_request_id() {
        let registry = CancellationRegistry::default();
        let writer = SharedWriter::default();
        let output = writer.clone();
        let writer = Arc::new(Mutex::new(writer));
        let calls = Arc::new(AtomicUsize::new(0));
        let observed_calls = Arc::clone(&calls);
        let handler: Arc<ToolCallHandler> = Arc::new(
            move |_name: &str,
                  _arguments: &Map<String, Value>,
                  _cancellation: CancellationToken| {
                if observed_calls.fetch_add(1, Ordering::SeqCst) == 0 {
                    panic!("simulated tool panic");
                }
                Ok("second call completed".to_string())
            },
        );
        let request = json!({ "jsonrpc": "2.0", "id": 7, "method": "tools/call", "params": { "name": "unica.code.search", "arguments": {} } });

        assert!(dispatch_tool_call(
            request.clone(),
            Arc::clone(&handler),
            registry.clone(),
            Arc::clone(&writer),
        ));
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            match registry.register(&json!(7)) {
                Ok(_) => {
                    registry.finish(&json!(7));
                    break;
                }
                Err(error) if error.starts_with("duplicate request id:") => {
                    assert!(
                        Instant::now() < deadline,
                        "panic cleanup did not release request id"
                    );
                    thread::yield_now();
                }
                Err(error) => panic!("unexpected registry error: {error}"),
            }
        }
        assert!(dispatch_tool_call(request, handler, registry, writer));

        let responses = output.wait_for_responses(1);
        assert_eq!(responses[0]["id"], 7);
        assert!(
            responses[0].get("result").is_some(),
            "request id remained registered after worker panic: {}",
            responses[0]
        );
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn mcp_dispatcher_late_cancellation_cannot_change_a_fixed_result() {
        let registry = CancellationRegistry::default();
        let id = json!(7);
        let cancellation = registry.register(&id).unwrap();

        assert!(!registry.finish(&id));
        assert!(!registry.cancel(&id));
        assert!(!cancellation.is_cancelled());
    }

    #[test]
    fn mcp_dispatcher_reuses_id_while_completed_response_is_waiting_to_publish() {
        let registry = CancellationRegistry::default();
        let writer = BlockingWriter::default();
        let entered = Arc::clone(&writer.entered);
        let release = Arc::clone(&writer.release);
        let writer = Arc::new(Mutex::new(writer));
        let calls = Arc::new(AtomicUsize::new(0));
        let observed_calls = Arc::clone(&calls);
        let handler: Arc<ToolCallHandler> = Arc::new(move |_, _, _| {
            observed_calls.fetch_add(1, Ordering::SeqCst);
            Ok("done".to_string())
        });
        let request = json!({ "jsonrpc": "2.0", "id": 7, "method": "tools/call", "params": { "name": "unica.code.search", "arguments": {} } });

        dispatch_tool_call(
            request.clone(),
            Arc::clone(&handler),
            registry.clone(),
            Arc::clone(&writer),
        );
        let deadline = Instant::now() + Duration::from_secs(2);
        while !entered.load(Ordering::SeqCst) {
            assert!(
                Instant::now() < deadline,
                "first response did not reach writer"
            );
            thread::yield_now();
        }

        let second_dispatch = thread::spawn(move || {
            dispatch_tool_call(request, handler, registry, writer);
        });
        let deadline = Instant::now() + Duration::from_secs(2);
        while calls.load(Ordering::SeqCst) < 2 && Instant::now() < deadline {
            thread::yield_now();
        }
        let observed = calls.load(Ordering::SeqCst);
        release.store(true, Ordering::SeqCst);
        second_dispatch.join().unwrap();

        assert_eq!(
            observed, 2,
            "completed request id remained registered until response publication"
        );
    }

    #[test]
    fn mcp_dispatcher_writer_failure_rejects_later_work_without_side_effects() {
        let registry = CancellationRegistry::default();
        let writer = FailingWriter::default();
        let write_failed = Arc::clone(&writer.0);
        let writer = Arc::new(Mutex::new(writer));
        let calls = Arc::new(AtomicUsize::new(0));
        let observed_calls = Arc::clone(&calls);
        let handler: Arc<ToolCallHandler> = Arc::new(move |_, _, _| {
            observed_calls.fetch_add(1, Ordering::SeqCst);
            Ok("done".to_string())
        });

        assert!(dispatch_tool_call(
            json!({ "jsonrpc": "2.0", "id": 7, "method": "tools/call", "params": { "name": "unica.code.search", "arguments": {} } }),
            Arc::clone(&handler),
            registry.clone(),
            Arc::clone(&writer),
        ));
        let deadline = Instant::now() + Duration::from_secs(2);
        while !write_failed.load(Ordering::SeqCst) || !registry.is_failed() {
            assert!(
                Instant::now() < deadline,
                "writer failure did not become terminal"
            );
            thread::yield_now();
        }

        let spawned = dispatch_tool_call(
            json!({ "jsonrpc": "2.0", "id": 8, "method": "tools/call", "params": { "name": "unica.code.search", "arguments": {} } }),
            handler,
            registry,
            writer,
        );

        assert!(!spawned, "terminal dispatcher spawned a rejected worker");
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "work started after terminal writer failure"
        );
    }
}
