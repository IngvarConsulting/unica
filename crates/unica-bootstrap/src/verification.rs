use std::collections::BTreeSet;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};
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

pub fn verify_mcp_runtime(entrypoint: &Path, runtime_root: &Path, timeout: Duration) -> Result<()> {
    let mut child = Command::new(entrypoint)
        .env("UNICA_PLUGIN_ROOT", runtime_root)
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
    let mut stdin = child
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

    let result = (|| {
        send_json(
            &mut stdin,
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
        let initialize = receive_response(&receiver, 1, timeout)?;
        if initialize.get("result").is_none() {
            return Err(BootstrapError::new(
                "Unica initialize response does not contain result",
            ));
        }
        send_json(
            &mut stdin,
            &json!({"jsonrpc": "2.0", "method": "notifications/initialized", "params": {}}),
        )?;
        send_json(
            &mut stdin,
            &json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}}),
        )?;
        let tools_response = receive_response(&receiver, 2, timeout)?;
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
    })();

    terminate(&mut child);
    result
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

fn terminate(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}
