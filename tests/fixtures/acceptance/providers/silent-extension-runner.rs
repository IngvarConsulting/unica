use std::{env, fs, path::Path, process::{Command, Stdio}, thread, time::{Duration, Instant}};
// Test-only release and fuse: neither adds a deadline to Unica operations.
fn hold(cwd: &Path) {
    let started = Instant::now();
    while cwd.exists() && !cwd.join("release.marker").exists() && started.elapsed() < Duration::from_secs(60) {
        thread::sleep(Duration::from_millis(20));
    }
}
fn main() {
    let cwd = env::current_dir().unwrap();
    if env::args().any(|a| a == "--descendant") {
        hold(&cwd);
        return;
    }
    if env::args().any(|a| a == "--dry-run") {
        println!(r#"{{"ok":true,"command":"extensions","data":{{"ok":true,"provider_dispatched":false,"provider":{{"selected":"ibcmd","origin":{{"kind":"default"}}}},"requested":{{"kind":"all"}},"extensions":[]}}}}"#);
        return;
    }
    let mut child = Command::new(env::current_exe().unwrap()).arg("--descendant")
        .stdin(Stdio::null()).spawn().unwrap();
    fs::write(cwd.join("tree.pids"), format!("{} {}", std::process::id(), child.id())).unwrap();
    // No output. Normal scenarios stop through the process owner's cancellation.
    hold(&cwd);
    let _ = child.wait();
}
