use std::collections::BTreeSet;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::error::{BootstrapError, Result};

const REQUIRED_TOOLS: [&str; 3] = [
    "unica.project.status",
    "unica.standards.search",
    "unica.standards.explain",
];

/// #490: every launch verifies both guaranteed lifecycles on fresh processes —
/// a legacy `initialize` session (the 2025-06-18 offer real hosts still send)
/// and the modern 2026-07-28 direct-first path opened by `server/discover`.
pub fn verify_mcp_runtime(
    entrypoint: &Path,
    runtime_root: &Path,
    provider_state_root: &Path,
    timeout: Duration,
) -> Result<()> {
    verify_legacy_session(entrypoint, runtime_root, provider_state_root, timeout)?;
    verify_modern_direct(entrypoint, runtime_root, provider_state_root, timeout)
}

struct RuntimeProbe {
    child: Child,
    stdin: ChildStdin,
    receiver: Receiver<std::result::Result<String, String>>,
}

impl RuntimeProbe {
    fn spawn(entrypoint: &Path, runtime_root: &Path, provider_state_root: &Path) -> Result<Self> {
        let mut child = Command::new(entrypoint)
            .env("UNICA_PLUGIN_ROOT", runtime_root)
            .env("UNICA_PROVIDER_STATE_DIR", provider_state_root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|error| {
                BootstrapError::new(format!(
                    "failed to start Unica runtime {}: {error}",
                    entrypoint.display()
                ))
            })?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| BootstrapError::new("failed to open Unica runtime stdin"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| BootstrapError::new("failed to open Unica runtime stdout"))?;
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                if sender
                    .send(line.map_err(|error| error.to_string()))
                    .is_err()
                {
                    break;
                }
            }
        });
        Ok(Self {
            child,
            stdin,
            receiver,
        })
    }
}

impl Drop for RuntimeProbe {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn verify_legacy_session(
    entrypoint: &Path,
    runtime_root: &Path,
    provider_state_root: &Path,
    timeout: Duration,
) -> Result<()> {
    let mut probe = RuntimeProbe::spawn(entrypoint, runtime_root, provider_state_root)?;
    send_json(
        &mut probe.stdin,
        &json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {"name": "unica-bootstrap", "version": env!("CARGO_PKG_VERSION")}
            }
        }),
    )?;
    let initialize = receive_response(&probe.receiver, 1, timeout)?;
    if initialize.get("result").is_none() {
        return Err(BootstrapError::new(
            "Unica initialize response does not contain result",
        ));
    }
    send_json(
        &mut probe.stdin,
        &json!({"jsonrpc": "2.0", "method": "notifications/initialized", "params": {}}),
    )?;
    send_json(
        &mut probe.stdin,
        &json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}}),
    )?;
    let tools_response = receive_response(&probe.receiver, 2, timeout)?;
    check_required_tools(&tools_response)
}

fn verify_modern_direct(
    entrypoint: &Path,
    runtime_root: &Path,
    provider_state_root: &Path,
    timeout: Duration,
) -> Result<()> {
    let mut probe = RuntimeProbe::spawn(entrypoint, runtime_root, provider_state_root)?;
    let meta = json!({
        "io.modelcontextprotocol/protocolVersion": "2026-07-28",
        "io.modelcontextprotocol/clientCapabilities": {}
    });
    send_json(
        &mut probe.stdin,
        &json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "server/discover",
            "params": {"_meta": meta}
        }),
    )?;
    let discover = receive_response(&probe.receiver, 1, timeout)?;
    let supported: BTreeSet<&str> = discover
        .pointer("/result/supportedVersions")
        .and_then(Value::as_array)
        .map(|versions| {
            versions
                .iter()
                .filter_map(Value::as_str)
                .collect::<BTreeSet<_>>()
        })
        .unwrap_or_default();
    for guaranteed in ["2025-06-18", "2025-11-25", "2026-07-28"] {
        if !supported.contains(guaranteed) {
            return Err(BootstrapError::new(format!(
                "Unica server/discover does not list guaranteed protocol version {guaranteed}: {discover}"
            )));
        }
    }
    send_json(
        &mut probe.stdin,
        &json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/list",
            "params": {"_meta": meta}
        }),
    )?;
    let tools_response = receive_response(&probe.receiver, 2, timeout)?;
    check_required_tools(&tools_response)
}

fn check_required_tools(tools_response: &Value) -> Result<()> {
    let tools = tools_response
        .pointer("/result/tools")
        .and_then(Value::as_array)
        .ok_or_else(|| BootstrapError::new("Unica tools/list response has no tools array"))?;
    let names = tools
        .iter()
        .filter_map(|tool| tool.get("name").and_then(Value::as_str))
        .collect::<BTreeSet<_>>();
    let missing = REQUIRED_TOOLS
        .iter()
        .copied()
        .filter(|name| !names.contains(name))
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        return Err(BootstrapError::new(format!(
            "Unica tools/list is missing required tools: {}",
            missing.join(", ")
        )));
    }
    Ok(())
}

fn send_json(stdin: &mut impl Write, value: &Value) -> Result<()> {
    serde_json::to_writer(&mut *stdin, value)?;
    stdin.write_all(b"\n")?;
    stdin.flush()?;
    Ok(())
}

fn receive_response(
    receiver: &Receiver<std::result::Result<String, String>>,
    id: u64,
    timeout: Duration,
) -> Result<Value> {
    let deadline = Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(BootstrapError::new(format!(
                "timed out waiting for Unica JSON-RPC response {id}"
            )));
        }
        let line = receiver.recv_timeout(remaining).map_err(|error| {
            BootstrapError::new(format!(
                "failed waiting for Unica JSON-RPC response {id}: {error}"
            ))
        })?;
        let line = line.map_err(BootstrapError::new)?;
        let value: Value = serde_json::from_str(&line).map_err(|error| {
            BootstrapError::new(format!("invalid JSON from Unica runtime: {error}"))
        })?;
        if value.get("id").and_then(Value::as_u64) != Some(id) {
            continue;
        }
        if let Some(error) = value.get("error") {
            return Err(BootstrapError::new(format!(
                "Unica JSON-RPC response {id} returned error: {error}"
            )));
        }
        return Ok(value);
    }
}
