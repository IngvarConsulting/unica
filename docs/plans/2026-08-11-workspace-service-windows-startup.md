# Workspace Service Windows Startup Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make workspace-service readiness independent of source-tree size and return actionable diagnostics when the startup child exits before readiness.

**Architecture:** `WorkspaceServiceRuntime` starts with unobserved analyzer/RLM generations and computes each generation at the first corresponding work boundary, after the service already accepts ping. The startup owner keeps the existing bounded cleanup but probes the child and reads a bounded stderr tail before constructing a readiness failure.

**Tech Stack:** Rust 2024 workspace, `std::process`, `std::fs`, `fs2`, existing `CancellationToken`, Cargo unit/integration tests, Windows Job Objects through the existing platform facade.

## Global Constraints

- Preserve ADR-0018 and `INV-APP-LAZY-HIDDEN-SERVICES`; do not add a public MCP server or tool.
- Do not change the five-second startup budget, the 120-second request deadline, or the `service.json` schema.
- Compute source generation before the first analyzer/RLM operation is admitted and keep the existing invalidation/freshness rules.
- Keep `service.lock` as the persistent OS advisory-lock file; do not delete or reinterpret it as child liveness.
- Include only a bounded best-effort stderr tail in startup failures; do not expose stdout.
- Add no dependency and keep OS-specific process operations in `infrastructure/platform/` (ADR-0009).
- Follow test-first development: observe the expected RED result before each production change.

---

## File map

- `crates/unica-coder/src/infrastructure/workspace_services.rs`: lazy source-generation state, startup-failure formatting, bounded stderr-tail reading, and unit fixtures/tests.
- `crates/unica-coder/src/infrastructure/platform/process.rs`: non-destructive exit-status probe on `ManagedStartupChild` and its focused unit test.
- `crates/unica-coder/tests/platform/issue_89_workspace_service.rs`: existing Windows process-tree/service integration regression suite; no new large fixture is committed.
- `docs/design/2026-08-11-workspace-service-windows-startup-design.md`: approved design and acceptance rationale; read-only during implementation.

### Task 1: Defer source generation until provider work

**Files:**
- Modify: `crates/unica-coder/src/infrastructure/workspace_services.rs:1338-1432`
- Modify: `crates/unica-coder/src/infrastructure/workspace_services.rs:1595-1640`
- Modify: `crates/unica-coder/src/infrastructure/workspace_services.rs:1720-1770`
- Test: `crates/unica-coder/src/infrastructure/workspace_services.rs` (`#[cfg(test)]` module)

**Interfaces:**
- Consumes: `source_generation(&Path) -> u64`, `AtomicBool`, and the existing analyzer/RLM lane boundaries.
- Produces: `observe_source_generation(&Mutex<Option<u64>>, &AtomicBool, u64) -> bool`; `WorkspaceServiceRuntime::{analyzer_source_generation, rlm_source_generation}` become `Mutex<Option<u64>>`.

- [ ] **Step 1: Add the failing lazy-generation tests**

Add these tests beside the existing source-generation tests:

```rust
#[test]
fn first_source_generation_observation_is_not_stale() {
    let observed = Mutex::new(None);
    let invalidated = AtomicBool::new(false);

    assert!(!observe_source_generation(
        &observed,
        &invalidated,
        41,
    ));
    assert_eq!(*observed.lock().unwrap(), Some(41));
    assert!(!observe_source_generation(
        &observed,
        &invalidated,
        41,
    ));
    assert!(observe_source_generation(
        &observed,
        &invalidated,
        42,
    ));

    invalidated.store(true, Ordering::Release);
    assert!(observe_source_generation(
        &observed,
        &invalidated,
        42,
    ));
}

#[test]
fn workspace_service_runtime_starts_without_observed_source_generations() {
    let context = test_context("lazy-runtime-generation");
    let source_root = context.workspace_root.join("src");
    let identity = WorkspaceServiceIdentity::new(&context, &source_root).unwrap();
    let record = test_record(&identity, 1, env!("CARGO_PKG_VERSION"));

    let runtime = WorkspaceServiceRuntime::new(identity, &record);

    assert_eq!(*runtime.analyzer_source_generation.lock().unwrap(), None);
    assert_eq!(*runtime.rlm_source_generation.lock().unwrap(), None);
    cleanup(&context);
}
```

- [ ] **Step 2: Run the tests and verify RED**

Run:

```powershell
cargo test -p unica-coder first_source_generation_observation_is_not_stale -- --nocapture
```

Expected: compilation fails because `observe_source_generation` does not exist.

Then run:

```powershell
cargo test -p unica-coder workspace_service_runtime_starts_without_observed_source_generations -- --nocapture
```

Expected: compilation fails because the runtime fields are `u64`, not `Option<u64>`.

- [ ] **Step 3: Implement the minimal lazy observation state**

Change both generation fields to `Mutex<Option<u64>>`, remove the eager
`source_generation` call from `WorkspaceServiceRuntime::new`, and initialize
both fields with `None`.

Add this helper near `WorkspaceServiceRuntime`:

```rust
fn observe_source_generation(
    observed: &Mutex<Option<u64>>,
    invalidated: &AtomicBool,
    current: u64,
) -> bool {
    let Ok(mut observed) = observed.lock() else {
        return false;
    };
    let changed = observed
        .replace(current)
        .is_some_and(|previous| previous != current);
    invalidated.swap(false, Ordering::AcqRel) || changed
}
```

In `handle_bsl_mcp`, replace the inline generation lock/swap block with:

```rust
let current_generation = source_generation(Path::new(&self.identity.source_root));
if observe_source_generation(
    &self.analyzer_source_generation,
    &self.analyzer_invalidated,
    current_generation,
) {
    stale_session = analyzer.take();
}
```

In `handle_rlm_mcp`, apply the same pattern with
`rlm_source_generation`, `rlm_invalidated`, and
`pre_execution_generation`. Do not change the readiness checks that use
`pre_execution_generation` and `post_execution_generation`.

- [ ] **Step 4: Verify GREEN and generation regressions**

Run:

```powershell
cargo test -p unica-coder first_source_generation_observation_is_not_stale -- --nocapture
cargo test -p unica-coder workspace_service_runtime_starts_without_observed_source_generations -- --nocapture
cargo test -p unica-coder source_generation -- --nocapture
cargo test -p unica-coder rlm_execute_rechecks_generation -- --nocapture
```

Expected: all matching tests pass; the constructor test completes without a
source-tree fingerprint.

- [ ] **Step 5: Commit the lazy-start fix**

```powershell
git add crates/unica-coder/src/infrastructure/workspace_services.rs
git commit -m "fix(code): defer workspace source generation until work"
```

### Task 2: Report startup-child exit diagnostics

**Files:**
- Modify: `crates/unica-coder/src/infrastructure/platform/process.rs:410-470`
- Modify: `crates/unica-coder/src/infrastructure/workspace_services.rs:20-45`
- Modify: `crates/unica-coder/src/infrastructure/workspace_services.rs:1144-1265`
- Test: `crates/unica-coder/src/infrastructure/platform/process.rs` (`#[cfg(test)]` module)
- Test: `crates/unica-coder/src/infrastructure/workspace_services.rs` (`#[cfg(test)]` module)

**Interfaces:**
- Consumes: Task 1's unchanged `SystemServiceSpawner` readiness flow and existing `terminate_failed_workspace_service_spawn` cleanup.
- Produces: `ManagedStartupChild::try_wait_status(&mut self) -> Result<Option<ExitStatus>, String>` and `workspace_service_startup_failure(&str, &mut ManagedStartupChild, &Path) -> String`.

- [ ] **Step 1: Add a failing process-facade test**

Add a platform-process unit test using the existing managed-child helper:

```rust
#[test]
fn startup_child_exposes_exit_status_without_detaching() {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "infrastructure::platform::process::tests::managed_child_test_helper",
            "--nocapture",
        ])
        .env(HELPER_ENV, "success")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut child = ManagedStartupChild::spawn_configured(command).unwrap();

    let deadline = Instant::now() + Duration::from_secs(2);
    let status = loop {
        if let Some(status) = child.try_wait_status().unwrap() {
            break status;
        }
        assert!(Instant::now() < deadline, "startup child did not exit");
        thread::yield_now();
    };

    assert!(status.success());
    child.terminate_bounded(Duration::from_secs(1)).unwrap();
}
```

- [ ] **Step 2: Run the facade test and verify RED**

```powershell
cargo test -p unica-coder startup_child_exposes_exit_status_without_detaching -- --nocapture
```

Expected: compilation fails because `try_wait_status` does not exist.

- [ ] **Step 3: Add the minimal exit-status probe**

Import `ExitStatus` from `std::process` and add:

```rust
pub(crate) fn try_wait_status(&mut self) -> Result<Option<ExitStatus>, String> {
    self.child
        .as_mut()
        .expect("startup child exists")
        .try_wait()
        .map_err(process_error)
}
```

Keep this operation free of Job Object calls and cleanup. Make the existing
test-only `is_running` delegate to `try_wait_status` so both paths use the same
probe.

- [ ] **Step 4: Verify the process-facade test GREEN**

```powershell
cargo test -p unica-coder startup_child_exposes_exit_status_without_detaching -- --nocapture
cargo test -p unica-coder infrastructure::platform::process::tests -- --nocapture
```

Expected: the focused test and all platform-process unit tests pass.

- [ ] **Step 5: Add the failing startup-diagnostic fixture and test**

Add a 16 KiB limit near the workspace-service constants:

```rust
const SERVICE_STARTUP_STDERR_TAIL_LIMIT: u64 = 16 * 1024;
```

Add a fixture test that exits only in the spawned harness:

```rust
#[test]
fn startup_failure_child_fixture() {
    if std::env::var_os("UNICA_STARTUP_FAILURE_CHILD_FIXTURE").is_some() {
        eprintln!("issue-339-startup-marker");
        std::process::exit(23);
    }
}
```

Add the behavior test beside `failed_spawn_cleanup_reaps_child_and_preserves_replacement_record`:

```rust
#[test]
fn startup_failure_reports_exit_status_and_bounded_stderr_tail() {
    let context = test_context("startup-failure-diagnostic");
    let identity =
        WorkspaceServiceIdentity::new(&context, &context.workspace_root.join("src")).unwrap();
    let stderr_path = identity.service_dir.join("service.stderr.log");
    fs::create_dir_all(&identity.service_dir).unwrap();
    let stderr = fs::File::create(&stderr_path).unwrap();
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "infrastructure::workspace_services::tests::startup_failure_child_fixture",
            "--nocapture",
        ])
        .env("UNICA_STARTUP_FAILURE_CHILD_FIXTURE", "1")
        .stdout(Stdio::null())
        .stderr(Stdio::from(stderr));
    let mut child = ManagedStartupChild::spawn_configured(command).unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while child.is_running().unwrap() {
        assert!(Instant::now() < deadline, "fixture did not exit");
        thread::yield_now();
    }

    let error = workspace_service_startup_failure(
        "workspace service did not become ready",
        &mut child,
        &stderr_path,
    );

    assert!(error.contains("exited before readiness"), "{error}");
    assert!(error.contains("23"), "{error}");
    assert!(error.contains("issue-339-startup-marker"), "{error}");
    assert!(error.len() <= SERVICE_STARTUP_STDERR_TAIL_LIMIT as usize + 512);
    child.terminate_bounded(Duration::from_secs(1)).unwrap();
    cleanup(&context);
}
```

- [ ] **Step 6: Run the diagnostic test and verify RED**

```powershell
cargo test -p unica-coder startup_failure_reports_exit_status_and_bounded_stderr_tail -- --nocapture
```

Expected: compilation fails because `workspace_service_startup_failure` does
not exist.

- [ ] **Step 7: Implement bounded stderr-tail diagnostics**

Import `Seek` and `SeekFrom`. Add a helper that opens the stderr file, seeks to
`length.saturating_sub(SERVICE_STARTUP_STDERR_TAIL_LIMIT)`, reads at most the
limit into a buffer, and converts it with `String::from_utf8_lossy`.

```rust
fn read_workspace_service_stderr_tail(path: &Path) -> Result<String, String> {
    let mut file = fs::File::open(path).map_err(|error| {
        format!(
            "failed to open workspace service stderr log {}: {error}",
            path.display()
        )
    })?;
    let length = file
        .metadata()
        .map_err(|error| format!("failed to inspect workspace service stderr log: {error}"))?
        .len();
    file.seek(SeekFrom::Start(
        length.saturating_sub(SERVICE_STARTUP_STDERR_TAIL_LIMIT),
    ))
    .map_err(|error| format!("failed to seek workspace service stderr log: {error}"))?;
    let mut bytes = Vec::new();
    file.take(SERVICE_STARTUP_STDERR_TAIL_LIMIT)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("failed to read workspace service stderr log: {error}"))?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}
```

Implement the formatter with these stable branches:

```rust
fn workspace_service_startup_failure(
    readiness_error: &str,
    child: &mut ManagedStartupChild,
    stderr_path: &Path,
) -> String {
    let pid = child.id();
    match child.try_wait_status() {
        Ok(Some(status)) => {
            let stderr = read_workspace_service_stderr_tail(stderr_path).unwrap_or_default();
            if stderr.trim().is_empty() {
                format!(
                    "{readiness_error}; spawned workspace service {pid} exited before readiness with {status}"
                )
            } else {
                format!(
                    "{readiness_error}; spawned workspace service {pid} exited before readiness with {status}; stderr tail: {}",
                    stderr.trim()
                )
            }
        }
        Ok(None) => format!(
            "{readiness_error}; spawned workspace service {pid} remained running until the readiness deadline"
        ),
        Err(error) => format!(
            "{readiness_error}; failed to inspect spawned workspace service {pid}: {error}"
        ),
    }
}
```

In `SystemServiceSpawner::spawn`, retain `stderr_path` before opening the log.
When readiness returns `Err`, call `workspace_service_startup_failure` before
`terminate_failed_workspace_service_spawn`, and preserve the existing rule that
cleanup errors append to the startup error. Do not change the success/detach
path or record ownership check.

- [ ] **Step 8: Verify diagnostic and cleanup tests GREEN**

```powershell
cargo test -p unica-coder startup_failure_reports_exit_status_and_bounded_stderr_tail -- --nocapture
cargo test -p unica-coder failed_spawn_cleanup_reaps_child_and_preserves_replacement_record -- --nocapture
cargo test -p unica-coder spawn_wait -- --nocapture
```

Expected: all matching tests pass, the fixture is reaped, and replacement
records remain intact.

- [ ] **Step 9: Commit startup diagnostics**

```powershell
git add crates/unica-coder/src/infrastructure/platform/process.rs crates/unica-coder/src/infrastructure/workspace_services.rs
git commit -m "fix(code): report workspace service startup exits"
```

### Task 3: Verify the issue boundary on Windows

**Files:**
- Verify: `crates/unica-coder/src/infrastructure/workspace_services.rs`
- Verify: `crates/unica-coder/src/infrastructure/platform/process.rs`
- Verify: `crates/unica-coder/tests/platform/issue_89_workspace_service.rs`
- Verify: `tests/ci/test_design_documents.py`

**Interfaces:**
- Consumes: Task 1 lazy generation and Task 2 startup diagnostics.
- Produces: evidence that unit, platform, architecture-document, and real Windows self-spawn boundaries pass without changing tracked files.

- [ ] **Step 1: Run formatting and focused Rust suites**

```powershell
cargo fmt --all -- --check
cargo test -p unica-coder infrastructure::workspace_services::tests -- --nocapture
cargo test -p unica-coder infrastructure::platform::process::tests -- --nocapture
cargo test -p unica-coder --test issue_89_workspace_service -- --nocapture
```

Expected: all commands exit zero. The intentional RAII-unwind fixture may print
its expected panic while the integration test still reports success.

- [ ] **Step 2: Run repository contract checks affected by the design**

```powershell
& 'C:\Users\user\.cache\codex-runtimes\codex-primary-runtime\dependencies\python\python.exe' tests/ci/test_design_documents.py
& 'C:\Users\user\.cache\codex-runtimes\codex-primary-runtime\dependencies\python\python.exe' tests/ci/test_architecture_registry.py
git diff --check
```

Expected: both Python suites report `OK`; `git diff --check` emits no output.

- [ ] **Step 3: Build the real Windows executable**

```powershell
cargo build -p unica-coder --bin unica
```

Expected: `target/debug/unica.exe` is produced. Existing unrelated compiler
warnings are recorded in the handoff but do not invalidate the issue-specific
result.

- [ ] **Step 4: Repeat the large-tree self-spawn acceptance test**

Run this bounded harness from the issue worktree. It creates and removes only
`.build/issue-339-large-repro` under that resolved worktree and uses the
installed 0.11.0 runtime only as a checked manifest/tool-binary source:

```powershell
$python = 'C:\Users\user\.cache\codex-runtimes\codex-primary-runtime\dependencies\python\python.exe'
@'
import json
import ctypes
import os
import pathlib
import queue
import shutil
import socket
import subprocess
import threading
import time

repo = pathlib.Path.cwd().resolve()
root = repo / ".build" / "issue-339-large-repro"
runtime = pathlib.Path(r"C:\Users\user\.codex\unica\runtimes\0.11.0\win-x64")
binary = repo / "target" / "debug" / "unica.exe"
assert root.parent == repo / ".build"
if root.exists():
    shutil.rmtree(root)

try:
    source = root / "src"
    source.mkdir(parents=True)
    for index in range(20_000):
        object_dir = source / f"Object{index:05d}"
        object_dir.mkdir()
        (object_dir / "Module.bsl").write_text(
            "Процедура Тест()\nКонецПроцедуры\n",
            encoding="utf-8",
        )

    environment = os.environ.copy()
    environment["UNICA_PLUGIN_ROOT"] = str(runtime)
    environment["UNICA_WORKSPACE_SERVICE_IDLE_SECS"] = "30"
    environment["UNICA_WORKSPACE_SERVICE_MAX_AGE_SECS"] = "120"
    process = subprocess.Popen(
        [str(binary)],
        cwd=repo,
        env=environment,
        text=True,
        encoding="utf-8",
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    responses = queue.Queue()
    threading.Thread(
        target=lambda: [responses.put(line) for line in process.stdout],
        daemon=True,
    ).start()

    def request(message, timeout=150):
        process.stdin.write(
            json.dumps(message, ensure_ascii=False, separators=(",", ":")) + "\n"
        )
        process.stdin.flush()
        deadline = time.time() + timeout
        while time.time() < deadline:
            response = json.loads(responses.get(timeout=max(0.1, deadline - time.time())))
            if response.get("id") == message.get("id"):
                return response
        raise TimeoutError(message)

    request({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2025-03-26",
            "capabilities": {},
            "clientInfo": {"name": "issue-339-acceptance", "version": "1"},
        },
    })
    process.stdin.write(
        '{"jsonrpc":"2.0","method":"notifications/initialized","params":{}}\n'
    )
    process.stdin.flush()
    tool_message = {
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {
            "name": "unica.code.diagnostics",
            "arguments": {
                "cwd": str(root),
                "sourceDir": "src",
                "mode": "status",
                "detail": "concise",
            },
        },
    }
    call_result = {}

    def call_tool():
        try:
            call_result["response"] = request(tool_message)
        except BaseException as error:
            call_result["error"] = error

    caller = threading.Thread(target=call_tool)
    caller.start()
    service_root = repo / ".build" / "unica" / "services"
    source_identity = source.resolve()
    record_path = None
    record = None
    readiness_deadline = time.time() + 10
    while time.time() < readiness_deadline and caller.is_alive():
        for candidate in service_root.glob("*/service.json"):
            candidate_record = json.loads(candidate.read_text(encoding="utf-8"))
            if pathlib.Path(candidate_record["source_root"]).resolve() == source_identity:
                record_path = candidate
                record = candidate_record
                break
        if record is not None:
            break
        time.sleep(0.05)
    caller.join(timeout=150)
    assert not caller.is_alive(), "tools/call exceeded 150 seconds"
    if "error" in call_result:
        raise call_result["error"]
    response = call_result["response"]
    encoded = json.dumps(response, ensure_ascii=False)
    assert "workspace service did not become ready" not in encoded, encoded
    assert record_path is not None and record is not None, encoded

    shutdown_request = json.dumps({
        "token": record["token"],
        "kind": {"type": "shutdown"},
    }, separators=(",", ":")) + "\n"
    with socket.create_connection(("127.0.0.1", record["port"]), timeout=2) as control:
        control.settimeout(10)
        control.sendall(shutdown_request.encode("utf-8"))
        shutdown_line = control.makefile("r", encoding="utf-8").readline()
    shutdown_response = json.loads(shutdown_line)
    assert shutdown_response.get("ok") is True, shutdown_response
    assert shutdown_response.get("shutdown") is True, shutdown_response

    process_handle = ctypes.windll.kernel32.OpenProcess(0x00100000, False, record["pid"])
    if process_handle:
        try:
            wait_result = ctypes.windll.kernel32.WaitForSingleObject(process_handle, 10_000)
            assert wait_result == 0, f"workspace service PID {record['pid']} did not exit"
        finally:
            ctypes.windll.kernel32.CloseHandle(process_handle)
    process.stdin.close()
    process.wait(timeout=5)
finally:
    if root.exists():
        shutil.rmtree(root)
'@ | & $python -
```

Expected: the call does not contain `workspace service did not become ready`;
the matching `service.json` is observed under the discovered worktree cache
during the call. A later analyzer-specific error is acceptable because this
acceptance step isolates the workspace-service startup boundary. The shutdown
response has `ok: true` and `shutdown: true`, and the service PID exits within
10 seconds before the fixture is removed.

- [ ] **Step 5: Review final scope and history**

```powershell
git status --short
git diff main...HEAD --stat
git log --oneline main..HEAD
```

Expected: only the approved design, this plan, and the two focused implementation
areas are changed; history contains the design commit plus the two implementation
commits. No `service.lock`, runtime cache, generated binary, or synthetic fixture
is tracked.
