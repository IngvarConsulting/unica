# Issue #89 Managed Child Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give every timed CLI and RLM process one bounded, platform-aware owner that can cancel the complete process tree without hanging on inherited pipes.

**Architecture:** Introduce a small cancellation primitive in the domain layer and a `ManagedChild` infrastructure module. `ManagedChild` drains pipes on dedicated readers, owns a Windows Job Object or Unix process group, and returns within bounded shutdown deadlines. Existing `ProcessRunner` and `IndexRunner` delegate to it without changing public MCP behavior.

**Tech Stack:** Rust 2021, `libc` on Unix, `windows-sys` Job Objects on Windows, std process/thread/channel APIs.

## Global Constraints

- Preserve existing adapter and index result formats.
- No unbounded `wait()`, `join()`, or `wait_with_output()` is allowed after cancellation or timeout.
- Windows cancellation must terminate descendants through a Job Object with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`.
- Unix cancellation must target a dedicated process group.
- Process-tree tests must use short deterministic deadlines and leave no child PID alive.
- Process launch/collection errors use `process_failed:`, while output status distinguishes `timeout` and `cancelled`.

---

## File Structure

- Create `crates/unica-coder/src/domain/cancellation.rs`: cloneable cancellation token.
- Modify `crates/unica-coder/src/domain/mod.rs`: export cancellation.
- Create `crates/unica-coder/src/infrastructure/managed_child.rs`: spawn, poll, collect, and terminate process trees.
- Modify `crates/unica-coder/src/infrastructure/mod.rs`: export managed-child internally.
- Modify root and crate `Cargo.toml`: target-specific low-level dependencies.
- Modify `crates/unica-coder/src/infrastructure/internal_adapters.rs`: delegate `SystemProcessRunner`.
- Modify `crates/unica-coder/src/infrastructure/workspace_index.rs`: delegate foreground/background index commands.

### Task 1: Cancellation token

**Files:**
- Create: `crates/unica-coder/src/domain/cancellation.rs`
- Modify: `crates/unica-coder/src/domain/mod.rs`
- Test: `crates/unica-coder/src/domain/cancellation.rs`

**Interfaces:**
- Produces: `CancellationToken::new()`, `cancel()`, `is_cancelled()`, and `Default + Clone`.

- [ ] **Step 1: Write the failing token test**

```rust
#[test]
fn clones_observe_cancellation() {
    let first = CancellationToken::new();
    let second = first.clone();
    assert!(!second.is_cancelled());
    first.cancel();
    assert!(second.is_cancelled());
}
```

- [ ] **Step 2: Run and verify RED**

Run: `cargo test -p unica-coder cancellation -- --nocapture`

Expected: FAIL because the module does not exist.

- [ ] **Step 3: Implement the token**

```rust
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};

#[derive(Debug, Clone, Default)]
pub struct CancellationToken(Arc<AtomicBool>);

impl CancellationToken {
    pub fn new() -> Self { Self::default() }
    pub fn cancel(&self) { self.0.store(true, Ordering::Release); }
    pub fn is_cancelled(&self) -> bool { self.0.load(Ordering::Acquire) }
}
```

- [ ] **Step 4: Run and verify GREEN**

Run: `cargo test -p unica-coder cancellation -- --nocapture`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/unica-coder/src/domain/cancellation.rs crates/unica-coder/src/domain/mod.rs
git commit -m "feat: add request cancellation token"
```

### Task 2: Managed process output and bounded pipe readers

**Files:**
- Create: `crates/unica-coder/src/infrastructure/managed_child.rs`
- Modify: `crates/unica-coder/src/infrastructure/mod.rs`
- Test: `crates/unica-coder/src/infrastructure/managed_child.rs`

**Interfaces:**
- Consumes: `CancellationToken`.
- Produces: `ManagedCommand`, `ManagedOutput`, `ManagedChild::spawn`, `ManagedChild::run`, `take_stdin`, `take_stdout`, `take_stderr`, `wait_for_output`, `wait_for_output_with_poll`, and `terminate`.

- [ ] **Step 1: Write failing success and timeout tests**

Use a test helper that runs the current test executable with an environment mode. Assert stdout/stderr collection on success and an upper bound on timeout return:

```rust
let started = Instant::now();
let output = run_helper("sleep", Duration::from_millis(100), CancellationToken::new()).unwrap();
assert!(output.timed_out);
assert!(started.elapsed() < Duration::from_secs(2));
```

- [ ] **Step 2: Run and verify RED**

Run: `cargo test -p unica-coder managed_child -- --nocapture`

Expected: FAIL because `ManagedChild` does not exist.

- [ ] **Step 3: Implement the platform-neutral owner**

Create these types:

```rust
pub struct ManagedCommand {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub env: Vec<(OsString, OsString)>,
    pub timeout: Option<Duration>,
    pub cancellation: CancellationToken,
}

pub struct ManagedOutput {
    pub status_success: bool,
    pub status: String,
    pub stdout: String,
    pub stderr: String,
    pub timed_out: bool,
    pub cancelled: bool,
}
```

`ManagedChild::spawn` must configure the platform tree before spawn and retain optional stdin/stdout/stderr handles. `ManagedChild::run` is `spawn` followed by `wait_for_output`. `take_stdin`, `take_stdout`, and `take_stderr` transfer handles to persistent protocol owners. For one-shot commands, `wait_for_output` lazily takes stdout/stderr and starts one reader per pipe; each reader sends its final `Vec<u8>` through an `mpsc` channel. `wait_for_output_with_poll(interval, callback)` is the same loop with a callback invoked after each interval, and `wait_for_output` delegates to it with a no-op callback. It polls `try_wait()` every 25 ms and calls `terminate()` on timeout or cancellation. After termination it polls for at most 500 ms and receives reader output for at most 500 ms per reader; expired readers are detached rather than joined.

```rust
loop {
    if let Some(status) = self.child.try_wait().map_err(process_error)? {
        return self.finish(status, false, false);
    }
    if self.cancellation.is_cancelled() {
        self.terminate()?;
        return self.finish_after_termination(false, true);
    }
    if self.timeout.is_some_and(|limit| started.elapsed() >= limit) {
        self.terminate()?;
        return self.finish_after_termination(true, false);
    }
    thread::sleep(Duration::from_millis(25));
}
```

- [ ] **Step 4: Run neutral tests**

Run: `cargo test -p unica-coder managed_child -- --nocapture`

Expected: success/timeout/cancellation tests PASS on the host platform.

- [ ] **Step 5: Commit**

```bash
git add crates/unica-coder/src/infrastructure/mod.rs crates/unica-coder/src/infrastructure/managed_child.rs
git commit -m "feat: add bounded managed child process"
```

### Task 3: Platform process-tree ownership

**Files:**
- Modify: `Cargo.toml`
- Modify: `crates/unica-coder/Cargo.toml`
- Modify: `crates/unica-coder/src/infrastructure/managed_child.rs`
- Test: `crates/unica-coder/src/infrastructure/managed_child.rs`

**Interfaces:**
- Consumes: `ManagedChild` from Task 2.
- Produces: internal `ProcessTree` with `prepare(&mut Command)`, `attach(&Child)`, and `terminate(&mut Child)`.

- [ ] **Step 1: Add a failing descendant test**

The helper test process must spawn a second copy of the current test executable, write both PIDs to a temporary file, and block. Cancel the parent through `ManagedChild`, then poll both PIDs for at most two seconds and assert both are gone.

```rust
token.cancel();
let output = managed.wait_for_output().unwrap();
assert!(output.cancelled);
assert!(wait_until_dead(parent_pid, Duration::from_secs(2)));
assert!(wait_until_dead(child_pid, Duration::from_secs(2)));
```

- [ ] **Step 2: Run and verify RED**

Run: `cargo test -p unica-coder managed_child_kills_descendants -- --nocapture`

Expected: FAIL because only the immediate child is terminated.

- [ ] **Step 3: Add target dependencies**

```toml
# Cargo.toml
[workspace.dependencies]
libc = "0.2"
windows-sys = { version = "0.59", features = [
  "Win32_Foundation",
  "Win32_System_JobObjects",
  "Win32_System_Threading",
] }

# crates/unica-coder/Cargo.toml
[target.'cfg(unix)'.dependencies]
libc.workspace = true

[target.'cfg(windows)'.dependencies]
windows-sys.workspace = true
```

- [ ] **Step 4: Implement Unix process groups**

In `CommandExt::pre_exec`, call `libc::setpgid(0, 0)` and propagate `last_os_error()` on failure. On termination, send `SIGKILL` to `-(child.id() as i32)`. Keep the unsafe block limited to the OS adapter and add a safety comment explaining the async-signal-safe call.

```rust
unsafe {
    command.pre_exec(|| {
        if libc::setpgid(0, 0) == -1 { return Err(std::io::Error::last_os_error()); }
        Ok(())
    });
}
```

- [ ] **Step 5: Implement Windows Job Objects**

Create the job before spawning, set `JOBOBJECT_EXTENDED_LIMIT_INFORMATION.BasicLimitInformation.LimitFlags` to `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, and call `AssignProcessToJobObject(job, child.as_raw_handle())`. `ProcessTree::drop` closes the job handle; explicit termination calls `TerminateJobObject`. Every failed Win32 call returns `last_os_error()` and closes any created handle.

```rust
let job = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
if job.is_null() { return Err(std::io::Error::last_os_error()); }
// SetInformationJobObject(... JobObjectExtendedLimitInformation ...)
// AssignProcessToJobObject(job, child.as_raw_handle() as HANDLE)
```

- [ ] **Step 6: Run the descendant test on the host**

Run: `cargo test -p unica-coder managed_child_kills_descendants -- --nocapture`

Expected: PASS and both recorded PIDs are no longer alive.

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml Cargo.lock crates/unica-coder/Cargo.toml crates/unica-coder/src/infrastructure/managed_child.rs
git commit -m "feat: terminate managed process trees"
```

### Task 4: Replace generic CLI process waiting

**Files:**
- Modify: `crates/unica-coder/src/infrastructure/internal_adapters.rs`
- Test: `crates/unica-coder/src/infrastructure/internal_adapters.rs`

**Interfaces:**
- Consumes: `ManagedCommand` and `ManagedOutput`.
- Produces: `ProcessCommand.cancellation: CancellationToken`; `ProcessOutput.cancelled: bool`.

- [ ] **Step 1: Write a failing cancelled-runner test**

```rust
let token = CancellationToken::new();
token.cancel();
let output = SystemProcessRunner.run(&ProcessCommand {
    program: helper_program(), args: helper_args("sleep"), cwd: temp_dir(),
    timeout: Some(Duration::from_secs(10)), cancellation: token,
}).unwrap();
assert!(output.cancelled);
assert!(!output.timed_out);
```

- [ ] **Step 2: Run and verify RED**

Run: `cargo test -p unica-coder cancelled_runner -- --nocapture`

Expected: FAIL because `ProcessCommand` has no cancellation field.

- [ ] **Step 3: Delegate `SystemProcessRunner`**

Add `cancellation` to all `ProcessCommand` constructors, using `CancellationToken::new()` until plan 3 supplies request tokens. Replace the hand-written `try_wait/kill/wait_with_output` loop with `ManagedChild::run` and map fields one-to-one.

```rust
let output = ManagedChild::run(ManagedCommand::from_process(command))?;
Ok(ProcessOutput {
    status_success: output.status_success,
    status: output.status,
    stdout: output.stdout,
    stderr: output.stderr,
    timed_out: output.timed_out,
    cancelled: output.cancelled,
})
```

- [ ] **Step 4: Run adapter tests**

Run: `cargo test -p unica-coder infrastructure::internal_adapters -- --nocapture`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/unica-coder/src/infrastructure/internal_adapters.rs
git commit -m "fix: bound cli process shutdown"
```

### Task 5: Replace RLM process waiting

**Files:**
- Modify: `crates/unica-coder/src/infrastructure/workspace_index.rs`
- Test: `crates/unica-coder/src/infrastructure/workspace_index.rs`

**Interfaces:**
- Consumes: `ManagedChild` and `CancellationToken`.
- Produces: `IndexCommand.cancellation`; foreground and background index processes with bounded tree shutdown.

- [ ] **Step 1: Write failing timeout and cancellation tests**

Extend the existing `print_lines_command` helper with a token and assert that a cancelled `index info` returns promptly with `cancelled == true`. Retain the heartbeat assertion for a normally running background build.

- [ ] **Step 2: Run and verify RED**

Run: `cargo test -p unica-coder workspace_index::tests -- --nocapture`

Expected: FAIL until `IndexCommand` and `IndexOutput` expose cancellation.

- [ ] **Step 3: Delegate index execution**

Add `cancellation: CancellationToken` to `IndexCommand` and `cancelled: bool` to `IndexOutput`. Replace `run_index_command_with_heartbeat` process polling with `ManagedChild`; preserve lease refresh by accepting a polling callback invoked every 50 ms while the child is alive.

```rust
managed.wait_for_output_with_poll(Duration::from_millis(50), || {
    if last_heartbeat.elapsed() >= LOCK_HEARTBEAT_INTERVAL {
        lease.refresh(pid);
        last_heartbeat = Instant::now();
    }
})
```

Map cancelled runs to a failed status with message `rlm index <action> cancelled`; do not leave an owned lock in running state.

- [ ] **Step 4: Run index and plan-level tests**

Run: `cargo test -p unica-coder workspace_index -- --nocapture`

Run: `cargo fmt --all -- --check`

Run: `cargo clippy -p unica-coder --all-targets -- -D warnings`

Run: `cargo test -p unica-coder`

Run: `git diff --check`

Expected: all commands PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/unica-coder/src/infrastructure/workspace_index.rs
git commit -m "fix: manage rlm process trees"
```
