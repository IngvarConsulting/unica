//! `unica lsp` процессом: убитый редактор не роняет демон и не мешает MCP.
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::time::{Duration, Instant};

#[path = "platform/daemon_teardown.rs"]
mod daemon_teardown;

const RESPONSE_DEADLINE: Duration = Duration::from_secs(20);

#[test]
fn a_killed_lsp_client_leaves_the_same_daemon_serving_mcp_and_a_new_editor() {
    let workspace = catalog_workspace();
    let state = workspace.path().join(".unica-test-state");
    std::fs::create_dir_all(&state).unwrap();
    let state = std::fs::canonicalize(state).unwrap();
    let _teardown = Teardown(state.clone());

    let mut editor = Frontend::start(workspace.path(), &state, &["lsp"]);
    let initialized = editor.lsp_request(1, "initialize", initialize_params(workspace.path()));
    assert_eq!(initialized["result"]["serverInfo"]["name"], "unica");
    let daemon = daemon_pid(&state);
    editor.lsp_send(json!({
        "jsonrpc": "2.0",
        "method": "textDocument/didOpen",
        "params": {"textDocument": {
            "uri": "file:///work/a.bsl", "languageId": "bsl", "version": 1,
            "text": "Процедура А() КонецПроцедуры",
        }},
    }));
    editor.lsp_send(json!({
        "jsonrpc": "2.0", "id": 2, "method": "textDocument/hover",
        "params": {"textDocument": {"uri": "file:///work/a.bsl"}, "position": {"line": 0, "character": 0}},
    }));
    // The editor's process dies with its session open and a request sent.
    editor.kill();

    let mut mcp = Frontend::start(workspace.path(), &state, &[]);
    mcp.mcp_request(json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "lsp-door", "version": "0"}},
    }));
    mcp.mcp_send(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
    let view = mcp.mcp_request(json!({
        "jsonrpc": "2.0", "id": 2, "method": "tools/call",
        "params": {"name": "unica.view", "arguments": {"at": "main:Catalog.Items"}},
    }));
    assert_eq!(view["result"]["isError"], json!(false), "{view:#}");
    assert_eq!(daemon_pid(&state), daemon, "the same daemon kept serving");

    let mut next = Frontend::start(workspace.path(), &state, &["lsp"]);
    next.lsp_request(1, "initialize", initialize_params(workspace.path()));
    assert_eq!(
        next.lsp_request(2, "shutdown", Value::Null)["result"],
        Value::Null
    );
    next.lsp_send(json!({"jsonrpc": "2.0", "method": "exit"}));
    assert_eq!(next.wait(), Some(0));
    assert_eq!(daemon_pid(&state), daemon);
    mcp.close();
}

struct Teardown(PathBuf);

impl Drop for Teardown {
    fn drop(&mut self) {
        daemon_teardown::stop_daemons_under(&self.0);
    }
}

struct Frontend {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
}

impl Frontend {
    fn start(workspace: &Path, state: &Path, args: &[&str]) -> Self {
        let mut command = Command::new(env!("CARGO_BIN_EXE_unica"));
        for key in unica_bootstrap::response_budget_environment_keys() {
            command.env_remove(key);
        }
        let mut child = command
            .args(args)
            .current_dir(workspace)
            .env("UNICA_PROVIDER_STATE_DIR", state)
            .env("UNICA_DAEMON_IDLE_GRACE_MS", "5000")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("start unica");
        let stdin = child.stdin.take();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        Self {
            child,
            stdin,
            stdout,
        }
    }

    fn lsp_send(&mut self, message: Value) {
        let body = serde_json::to_vec(&message).unwrap();
        let stdin = self.stdin.as_mut().unwrap();
        write!(stdin, "Content-Length: {}\r\n\r\n", body.len()).unwrap();
        stdin.write_all(&body).unwrap();
        stdin.flush().unwrap();
    }

    fn lsp_request(&mut self, id: u64, method: &str, params: Value) -> Value {
        self.lsp_send(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        let deadline = Instant::now() + RESPONSE_DEADLINE;
        loop {
            assert!(Instant::now() < deadline, "no LSP response to {method}");
            let message = self.lsp_read();
            if message.get("id") == Some(&json!(id)) {
                return message;
            }
        }
    }

    fn lsp_read(&mut self) -> Value {
        let mut length = None;
        loop {
            let mut line = String::new();
            assert!(
                self.stdout.read_line(&mut line).unwrap() > 0,
                "unica lsp exited"
            );
            let line = line.trim_end();
            if line.is_empty() {
                break;
            }
            if let Some(value) = line.strip_prefix("Content-Length: ") {
                length = Some(value.parse::<usize>().unwrap());
            }
        }
        let mut body = vec![0; length.expect("Content-Length header")];
        self.stdout.read_exact(&mut body).unwrap();
        serde_json::from_slice(&body).unwrap()
    }

    fn mcp_send(&mut self, message: Value) {
        let stdin = self.stdin.as_mut().unwrap();
        serde_json::to_writer(&mut *stdin, &message).unwrap();
        stdin.write_all(b"\n").unwrap();
        stdin.flush().unwrap();
    }

    fn mcp_request(&mut self, request: Value) -> Value {
        let id = request["id"].clone();
        self.mcp_send(request);
        let deadline = Instant::now() + RESPONSE_DEADLINE;
        loop {
            assert!(Instant::now() < deadline, "no MCP response");
            let mut line = String::new();
            assert!(self.stdout.read_line(&mut line).unwrap() > 0, "MCP exited");
            let response: Value = serde_json::from_str(&line).unwrap();
            if response.get("id") == Some(&id) {
                return response;
            }
        }
    }

    fn kill(&mut self) {
        self.child.kill().unwrap();
        self.child.wait().unwrap();
    }

    fn close(&mut self) {
        self.stdin.take();
        self.wait();
    }

    fn wait(&mut self) -> Option<i32> {
        let deadline = Instant::now() + RESPONSE_DEADLINE;
        while Instant::now() < deadline {
            if let Some(status) = self.child.try_wait().unwrap() {
                return status.code();
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("unica did not exit");
    }
}

impl Drop for Frontend {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

fn daemon_pid(state: &Path) -> u64 {
    let pids = std::fs::read_dir(state)
        .unwrap()
        .flatten()
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with("daemon-p5-")
        })
        .filter_map(|entry| std::fs::read(entry.path().join("endpoint.json")).ok())
        .filter_map(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .filter_map(|record| record["pid"].as_u64())
        .collect::<Vec<_>>();
    assert_eq!(pids.len(), 1, "exactly one daemon serves this state");
    pids[0]
}

fn initialize_params(workspace: &Path) -> Value {
    json!({
        "processId": null,
        "capabilities": {},
        "workspaceFolders": [{
            "uri": format!("file://{}", std::fs::canonicalize(workspace).unwrap().display()),
            "name": "work",
        }],
    })
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
