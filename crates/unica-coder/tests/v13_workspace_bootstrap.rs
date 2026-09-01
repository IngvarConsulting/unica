use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

const RESPONSE_DEADLINE: Duration = Duration::from_secs(15);

struct McpProcess {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: Receiver<String>,
    stdout_reader: Option<JoinHandle<()>>,
}

impl McpProcess {
    fn start(workspace: &std::path::Path, state: &std::path::Path) -> Self {
        let workspace = std::fs::canonicalize(workspace).expect("canonical bootstrap workspace");
        let state = std::fs::canonicalize(state).expect("canonical bootstrap daemon state");
        let mut child = Command::new(env!("CARGO_BIN_EXE_unica"))
            .arg("mcp")
            .current_dir(workspace)
            .env("UNICA_PROVIDER_STATE_DIR", state)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("start canonical Unica MCP");
        let stdin = child.stdin.take().expect("MCP stdin");
        let stdout = child.stdout.take().expect("MCP stdout");
        let (line_sender, line_receiver) = mpsc::channel();
        let stdout_reader = std::thread::spawn(move || read_stdout_lines(stdout, line_sender));
        Self {
            child,
            stdin: Some(stdin),
            stdout: line_receiver,
            stdout_reader: Some(stdout_reader),
        }
    }

    fn exchange(&mut self, request: Value) -> Value {
        let id = request["id"].clone();
        let stdin = self.stdin.as_mut().expect("open MCP stdin");
        serde_json::to_writer(&mut *stdin, &request).expect("encode MCP request");
        stdin.write_all(b"\n").expect("terminate MCP request");
        stdin.flush().expect("flush MCP request");
        let deadline = Instant::now() + RESPONSE_DEADLINE;
        loop {
            assert!(Instant::now() < deadline, "MCP response deadline elapsed");
            let remaining = deadline.saturating_duration_since(Instant::now());
            let line = match self.stdout.recv_timeout(remaining) {
                Ok(line) => line,
                Err(RecvTimeoutError::Timeout) => panic!("MCP response deadline elapsed"),
                Err(RecvTimeoutError::Disconnected) => panic!("MCP exited before response"),
            };
            let response: Value = serde_json::from_str(&line).expect("decode MCP response");
            if response.get("id") == Some(&id) {
                return response;
            }
        }
    }

    fn notify(&mut self, notification: Value) {
        let stdin = self.stdin.as_mut().expect("open MCP stdin");
        serde_json::to_writer(&mut *stdin, &notification).expect("encode MCP notification");
        stdin.write_all(b"\n").expect("terminate MCP notification");
        stdin.flush().expect("flush MCP notification");
    }

    fn finish(&mut self) {
        drop(self.stdin.take());
        let deadline = Instant::now() + RESPONSE_DEADLINE;
        while Instant::now() < deadline {
            if self.child.try_wait().expect("poll MCP exit").is_some() {
                self.join_stdout_reader();
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        self.child.kill().expect("kill stalled MCP");
        self.child.wait().expect("reap stalled MCP");
        self.join_stdout_reader();
        panic!("MCP did not stop after stdin EOF");
    }

    fn join_stdout_reader(&mut self) {
        if let Some(reader) = self.stdout_reader.take() {
            reader.join().expect("join MCP stdout reader");
        }
    }
}

impl Drop for McpProcess {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
        self.join_stdout_reader();
    }
}

fn read_stdout_lines(stdout: ChildStdout, sender: mpsc::Sender<String>) {
    let mut stdout = BufReader::new(stdout);
    loop {
        let mut line = String::new();
        match stdout.read_line(&mut line) {
            Ok(0) | Err(_) => return,
            Ok(_) if sender.send(line).is_err() => return,
            Ok(_) => {}
        }
    }
}

#[test]
fn canonical_stdio_bootstraps_an_empty_workspace_before_address_discovery() {
    let root = tempfile::tempdir().expect("bootstrap integration root");
    let workspace = root.path().join("workspace");
    let state = root.path().join("state");
    std::fs::create_dir(&workspace).unwrap();
    std::fs::create_dir(&state).unwrap();
    let mut mcp = McpProcess::start(&workspace, &state);

    let initialized = mcp.exchange(json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2025-11-25",
            "capabilities": {},
            "clientInfo": {"name": "v13-bootstrap-ci", "version": "1"}
        }
    }));
    assert_eq!(initialized["result"]["serverInfo"]["name"], "unica");
    assert!(initialized["result"]["instructions"]
        .as_str()
        .unwrap()
        .contains("unica.view using an empty object"));
    mcp.notify(json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized",
        "params": {}
    }));

    let listed = mcp.exchange(json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/list",
        "params": {}
    }));
    let tools = listed["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 11);
    assert!(tools.iter().all(|tool| tool["description"]
        .as_str()
        .is_some_and(|description| !description.is_empty())));
    let view = tools
        .iter()
        .find(|tool| tool["name"] == "unica.view")
        .unwrap();
    assert!(!view["inputSchema"]["required"]
        .as_array()
        .is_some_and(|required| required.iter().any(|name| name == "at")));

    let response = mcp.exchange(json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "tools/call",
        "params": {"name": "unica.view", "arguments": {}}
    }));
    let result = &response["result"]["structuredContent"];
    assert_eq!(result["ok"], true, "{response:#}");
    assert_eq!(result["data"]["config"]["state"], "missing");
    assert_eq!(result["data"]["setup"]["path"], "v8project.yaml");
    assert!(result["data"]["setup"]["content"]
        .as_str()
        .unwrap()
        .contains("source-set:"));
    assert!(!serde_json::to_string(result)
        .unwrap()
        .contains("unica.project."));

    mcp.finish();
}
