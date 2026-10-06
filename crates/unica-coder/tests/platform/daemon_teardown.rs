// Shared teardown is compiled into targets that use different subsets.
#![allow(dead_code)]

use std::path::Path;
use std::process::{Command, Stdio};

/// Stops every daemon that a test's frontends started under `state`.
///
/// The idle grace does not bound such a daemon: it keeps running while it
/// holds a saved apply plan, a replay result or shared index work, as
/// `arch/rules/workspace/actor-retention.md` requires. A test that previews
/// or applies a change therefore leaves its daemon alive after the frontend
/// exits. On Windows that process keeps `target\debug\unica.exe` open, and the
/// next build with other features cannot replace the file.
pub fn stop_daemons_under(state: &Path) {
    let Ok(entries) = std::fs::read_dir(state) else {
        return;
    };
    for entry in entries.flatten() {
        if !entry
            .file_name()
            .to_string_lossy()
            .starts_with("daemon-p5-")
        {
            continue;
        }
        let Some(pid) = std::fs::read(entry.path().join("endpoint.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
            .and_then(|record| record["pid"].as_u64())
        else {
            continue;
        };
        terminate(pid);
    }
}

#[cfg(unix)]
fn terminate(pid: u64) {
    let _ = Command::new("kill")
        .args(["-KILL", &pid.to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

#[cfg(windows)]
fn terminate(pid: u64) {
    let _ = Command::new("taskkill")
        .args(["/F", "/T", "/PID", &pid.to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}
