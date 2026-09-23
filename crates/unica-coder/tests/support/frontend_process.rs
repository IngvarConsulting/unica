// Shared process helpers are compiled into targets that use different subsets.
#![allow(dead_code)]

use serde_json::Value;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

pub const PRODUCTION_V5_IDENTITY: &str =
    "884b76181583ce34907a2a9758e2b493e5b40883e7cbb0d7f88dcec0e468cfa0";
pub struct OwnedProcess(pub Child);

impl OwnedProcess {
    pub fn stop(&mut self) {
        if self.0.try_wait().unwrap().is_none() {
            self.0.kill().unwrap();
        }
        self.0.wait().unwrap();
    }
}

impl Drop for OwnedProcess {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

pub fn spawn_owned_daemon(state: &Path) -> OwnedProcess {
    std::fs::create_dir_all(state).unwrap();
    let process = OwnedProcess(
        Command::new(env!("CARGO_BIN_EXE_unica"))
            .args(["--daemon", "--state-root"])
            .arg(state)
            .args([
                "--core-identity",
                PRODUCTION_V5_IDENTITY,
                "--idle-grace-ms",
                "20000",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap(),
    );
    wait_until(
        Duration::from_secs(10),
        || {
            std::fs::read(endpoint_path(state, PRODUCTION_V5_IDENTITY))
                .ok()
                .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
                .is_some_and(|record| record["pid"] == process.0.id())
        },
        "owned daemon endpoint",
    );
    process
}

pub fn make_stdio_workspace(root: &Path) -> PathBuf {
    let workspace = root.join("workspace");
    std::fs::create_dir_all(workspace.join("src")).unwrap();
    assert!(Command::new("git")
        .args(["init", "-q"])
        .arg(&workspace)
        .status()
        .unwrap()
        .success());
    std::fs::write(
        workspace.join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
    )
    .unwrap();
    workspace
}

pub struct StdioFrontend {
    pub process: OwnedProcess,
    input: ChildStdin,
    responses: Receiver<Value>,
}

impl StdioFrontend {
    pub fn spawn(state: &Path, workspace: &Path) -> Self {
        Self::spawn_profile(state, workspace, false)
    }

    pub fn spawn_native(state: &Path, workspace: &Path) -> Self {
        Self::spawn_profile(state, workspace, true)
    }

    fn spawn_profile(state: &Path, workspace: &Path, native: bool) -> Self {
        let mut process = OwnedProcess(
            Command::new(env!("CARGO_BIN_EXE_unica"))
                .current_dir(workspace)
                .env("UNICA_PROVIDER_STATE_DIR", state)
                .env("UNICA_DAEMON_IDLE_GRACE_MS", "400")
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        );
        let input = process.0.stdin.take().unwrap();
        let output = process.0.stdout.take().unwrap();
        let (sender, responses) = mpsc::channel();
        thread::spawn(move || {
            for line in BufReader::new(output).lines() {
                let Ok(line) = line else { break };
                let Ok(value) = serde_json::from_str(&line) else {
                    break;
                };
                if sender.send(value).is_err() {
                    break;
                }
            }
        });
        let mut frontend = Self {
            process,
            input,
            responses,
        };
        let response = frontend.request(
            1,
            "initialize",
            serde_json::json!({
                "protocolVersion": if native { "2026-07-28" } else { "2025-11-25" },
                "capabilities": if native { serde_json::json!({"extensions": {"io.modelcontextprotocol/tasks": {}}}) } else { serde_json::json!({}) },
                "clientInfo": {"name": "daemon-replacement-test", "version": "1"}
            }),
        );
        assert!(response.get("result").is_some(), "initialize: {response}");
        frontend.send(serde_json::json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
        frontend
    }

    fn send(&mut self, message: Value) {
        writeln!(self.input, "{message}").unwrap();
        self.input.flush().unwrap();
    }

    pub fn request(&mut self, id: u64, method: &str, params: Value) -> Value {
        self.send(
            serde_json::json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}),
        );
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            let response = self
                .responses
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .expect("bounded MCP response from the same frontend");
            if response["id"] == id {
                return response;
            }
        }
    }

    pub fn check(&mut self, id: u64) {
        let response = self.request(
            id,
            "tools/call",
            serde_json::json!({"name": "unica.check", "arguments": {}}),
        );
        assert!(
            response.get("result").is_some(),
            "ordinary call through surviving frontend: {response}"
        );
        assert_ne!(response["result"]["isError"], true, "{response}");
    }
}
pub fn endpoint_path(state_root: &Path, identity: &str) -> PathBuf {
    unica_coder::interfaces::daemon::endpoint_path_for_protocol_test(state_root, identity)
}

pub fn read_endpoint(state_root: &Path, identity: &str) -> Value {
    serde_json::from_slice(&std::fs::read(endpoint_path(state_root, identity)).unwrap()).unwrap()
}

pub fn wait_until(timeout: Duration, predicate: impl Fn() -> bool, what: &str) {
    let deadline = Instant::now() + timeout;
    while !predicate() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        thread::sleep(Duration::from_millis(20));
    }
}
