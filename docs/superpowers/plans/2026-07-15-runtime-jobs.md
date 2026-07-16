# Durable runtime jobs Implementation Plan

> Historical: this plan is preserved as execution context. Current source of truth is code/tests/package metadata, then `spec/`, not this plan.

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Добавить durable, typed и безопасный lifecycle для долгих `v8-runner` операций без изменения `unica.runtime.execute`.

**Architecture:** `unica.runtime.job.start` создаёт schema-versioned job record под workspace cache и запускает отдельный `unica --runtime-job-worker`; secret-bearing execution argv передаётся worker-у только по stdin. Worker владеет дочерним `v8-runner`, redacts bounded output, сохраняет ровно один terminal record. Новые job tools возвращают typed `OperationResult.job`; обычный runtime contract remains synchronous.

**Tech Stack:** Rust 2021, std `Command`/pipes/threading, `serde_json`, `uuid`, existing `fs2`, MCP stdio, cargo test/clippy/fmt.

---

### Task 1: Общий bounded redaction для worker logs

**Files:**
- Create: `crates/unica-coder/src/infrastructure/redaction.rs`
- Modify: `crates/unica-coder/src/infrastructure/mod.rs`
- Modify: `crates/unica-coder/src/infrastructure/internal_adapters.rs:1-16,651-706`
- Test: `crates/unica-coder/src/infrastructure/redaction.rs`

- [ ] **Step 1: Write the failing tests for redaction and chunk boundaries**

```rust
#[test]
fn stream_redactor_hides_secret_split_between_chunks() {
    let mut redactor = StreamRedactor::new();
    assert_eq!(redactor.push("starting; P"), "starting; ");
    assert_eq!(redactor.push("wd=super-secret\nfinished\n"), "Pwd=<redacted>\nfinished\n");
    assert_eq!(redactor.finish(), "");
}

#[test]
fn stream_redactor_keeps_non_secret_text_and_truncates_only_after_limit() {
    let mut redactor = StreamRedactor::new();
    assert_eq!(redactor.push("build source-set main\n"), "build source-set main\n");
    assert_eq!(redactor.finish(), "");
}
```

- [ ] **Step 2: Run the focused tests and verify RED**

Run: `cargo test --locked -p unica-coder stream_redactor_ -- --nocapture`

Expected: FAIL because module/type `StreamRedactor` does not exist.

- [ ] **Step 3: Implement the minimal shared redactor**

```rust
pub(crate) fn redact_sensitive_text(text: &str) -> String {
    let chars = text.chars().collect::<Vec<_>>();
    let mut output = String::with_capacity(text.len());
    let mut index = 0;
    while index < chars.len() {
        if secret_key_char(chars[index]) {
            let key_start = index;
            index += 1;
            while index < chars.len() && secret_key_char(chars[index]) { index += 1; }
            let key_end = index;
            let mut separator = index;
            while separator < chars.len() && chars[separator].is_whitespace() { separator += 1; }
            if separator < chars.len() && matches!(chars[separator], '=' | ':') {
                let mut value_start = separator + 1;
                while value_start < chars.len() && chars[value_start].is_whitespace() { value_start += 1; }
                let key = chars[key_start..key_end].iter().collect::<String>();
                if is_secret_key(&key) {
                    output.extend(chars[key_start..value_start].iter());
                    output.push_str("<redacted>");
                    index = value_start;
                    while index < chars.len() && !secret_value_delimiter(chars[index]) { index += 1; }
                    continue;
                }
            }
            output.extend(chars[key_start..key_end].iter());
            continue;
        }
        output.push(chars[index]);
        index += 1;
    }
    output
}

fn secret_key_char(ch: char) -> bool { ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-') }
fn secret_value_delimiter(ch: char) -> bool { matches!(ch, ';' | '&' | ',' | '\n' | '\r' | '}') }
fn is_secret_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    key == "connection" || key == "pwd" || key.contains("password") || key.contains("token") || key.contains("secret")
}

pub(crate) struct StreamRedactor { pending: String }

impl StreamRedactor {
    pub(crate) fn new() -> Self { Self { pending: String::new() } }
    pub(crate) fn push(&mut self, chunk: &str) -> String {
        self.pending.push_str(chunk);
        let boundary = self.pending.rfind(['\n', '\r', ';', '&', ',']).map(|i| i + 1);
        boundary.map(|end| redact_sensitive_text(&self.pending.drain(..end).collect::<String>())).unwrap_or_default()
    }
    pub(crate) fn finish(mut self) -> String { redact_sensitive_text(&std::mem::take(&mut self.pending)) }
}
```

Keep `pending` private, use borrowed `&str`, and make the existing adapters import the shared function. Do not keep a second redaction implementation.

- [ ] **Step 4: Run focused tests and the existing adapter redaction tests**

Run: `cargo test --locked -p unica-coder redacts -- --nocapture`

Expected: PASS; serialized outputs contain `<redacted>` and never the test secrets.

- [ ] **Step 5: Commit the isolated implementation**

```bash
git add crates/unica-coder/src/infrastructure/{mod.rs,redaction.rs,internal_adapters.rs}
git commit -m "refactor: вынести redaction runtime вывода"
```

### Task 2: Durable job record, state machine and worker core

**Files:**
- Create: `crates/unica-coder/src/infrastructure/runtime_jobs.rs`
- Modify: `crates/unica-coder/src/infrastructure/mod.rs`
- Test: `crates/unica-coder/src/infrastructure/runtime_jobs.rs`

- [ ] **Step 1: Write failing fake-runner lifecycle tests**

```rust
#[test]
fn worker_persists_long_success_for_reconnect() {
    let fixture = JobFixture::new(FakeRunner::after_polls(2, JobExit::success("ok\n")));
    let started = fixture.start("build").unwrap();
    assert!(matches!(started.phase, RuntimeJobPhase::Queued | RuntimeJobPhase::Running));
    assert_eq!(fixture.reconnected().status(&started.job_id).unwrap().phase, RuntimeJobPhase::Running);
    let done = fixture.wait_until_terminal(&started.job_id);
    assert_eq!(done.phase, RuntimeJobPhase::Succeeded);
    assert_eq!(done.exit_code, Some(0));
}

#[test]
fn safe_cancel_is_terminal_but_load_cancel_is_deferred() {
    let fixture = JobFixture::new(FakeRunner::never_finishes());
    let safe = fixture.start("test").unwrap();
    assert_eq!(fixture.cancel(&safe.job_id).unwrap().phase, RuntimeJobPhase::CancelRequested);
    assert_eq!(fixture.wait_until_terminal(&safe.job_id).phase, RuntimeJobPhase::Cancelled);
    let critical = fixture.start_after_terminal("load").unwrap();
    let deferred = fixture.cancel(&critical.job_id).unwrap();
    assert!(deferred.cancel_deferred);
    assert_eq!(deferred.unsafe_phase.as_deref(), Some("load"));
}

#[test]
fn stale_running_record_becomes_lost_without_deleting_fresh_lock() {
    let fixture = JobFixture::new(FakeRunner::never_finishes());
    let stale = fixture.start("build").unwrap();
    fixture.set_updated_at(&stale.job_id, 0).unwrap();
    let replacement = fixture.write_active_lock("replacement-job").unwrap();
    let recovered = fixture.reconnected().recover_stale().unwrap();
    assert!(recovered.iter().any(|job| job.job_id == stale.job_id && job.phase == RuntimeJobPhase::Lost));
    assert_eq!(fixture.read_active_lock().unwrap(), replacement);
}
```

- [ ] **Step 2: Run the lifecycle tests and verify RED**

Run: `cargo test --locked -p unica-coder runtime_jobs::tests -- --nocapture`

Expected: FAIL because `runtime_jobs` does not exist.

- [ ] **Step 3: Implement typed persistence and worker abstractions**

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum RuntimeJobPhase { Queued, Running, CancelRequested, Succeeded, Failed, Cancelled, TimedOut, Lost }

impl RuntimeJobPhase {
    fn is_terminal(self) -> bool { matches!(self, Self::Succeeded | Self::Failed | Self::Cancelled | Self::TimedOut | Self::Lost) }
}

pub(crate) trait RuntimeJobRunner: Send + Sync {
    fn spawn(&self, command: &WorkerCommand) -> Result<Box<dyn RuntimeJobProcess>, String>;
}
pub(crate) trait RuntimeJobProcess: Send {
    fn id(&self) -> Option<u32>;
    fn try_wait(&mut self) -> Result<Option<JobExit>, String>;
    fn cancel(&mut self) -> Result<(), String>;
    fn read_output(&mut self) -> Result<JobOutput, String>;
}
```

Implement `RuntimeJobStore` with `record.json` temp-file plus `rename`, strict UUID path lookup, one `active.lock` created with `create_new`, `cancel.json`, stale heartbeat recovery and exhaustive phase transition validation. Record stores schema version, timestamps, redacted argv, target, worker/runner identity, terminal data, paths and warnings—never actual argv. `RuntimeJobWorker::run` polls fake/system process, refreshes heartbeat, applies `CancelPolicy::{Safe,Critical}`, then writes exactly one terminal record and removes only its own active lock.

- [ ] **Step 4: Add failing diagnostics and conflict tests, then implement their minimum**

```rust
#[test]
fn active_job_conflict_names_existing_job_id() {
    let fixture = JobFixture::new(FakeRunner::never_finishes());
    let first = fixture.start("build").unwrap();
    let error = fixture.start("test").unwrap_err();
    assert!(error.contains(&first.job_id));
}
#[test]
fn terminal_view_contains_redacted_argv_and_log_tails() {
    let fixture = JobFixture::new(FakeRunner::after_polls(0, JobExit::success("Pwd=secret\n")));
    let started = fixture.start_with_argv("build", &["--connection", "Pwd=secret"]).unwrap();
    let terminal = fixture.wait_until_terminal(&started.job_id);
    let serialized = serde_json::to_string(&terminal).unwrap();
    assert!(serialized.contains("<redacted>"));
    assert!(!serialized.contains("secret"));
    assert_eq!(terminal.exit_code, Some(0));
}
#[test]
fn caller_wait_timeout_leaves_job_running() {
    let fixture = JobFixture::new(FakeRunner::after_polls(10, JobExit::success("ok\n")));
    let started = fixture.start("test").unwrap();
    let waited = fixture.wait(&started.job_id, Duration::ZERO).unwrap();
    assert_eq!(waited.phase, RuntimeJobPhase::Running);
    assert!(waited.wait_timed_out);
}
```

Implement `RuntimeJobService::{start,status,wait,logs,cancel,list}` over the store. `wait` only limits its loop; it does not kill, fail or mutate a job. `logs` reads bounded sanitized tails. `list` excludes corrupt records but reports a redacted warning; direct status of corrupt/unknown schema is an error and does not remove lock.

- [ ] **Step 5: Run all runtime-job unit tests and commit**

Run: `cargo test --locked -p unica-coder runtime_jobs -- --nocapture`

Expected: PASS with success, failure, reconnect, wait timeout, safe/deferred cancel, conflict, stale-lost and redaction cases.

```bash
git add crates/unica-coder/src/infrastructure/{mod.rs,runtime_jobs.rs}
git commit -m "feat: добавить durable runtime job worker"
```

### Task 3: Public typed MCP tools and result envelope

**Files:**
- Modify: `crates/unica-coder/src/application/mod.rs:22-61,120-230,294-370,720-735`
- Modify: `crates/unica-coder/src/application/ports.rs:1-95`
- Modify: `crates/unica-coder/src/application/tool_contracts.rs:10-330,551-620,1180-1420`
- Modify: `crates/unica-coder/src/infrastructure/internal_adapters.rs:223-348`
- Test: `crates/unica-coder/src/application/mod.rs`
- Test: `crates/unica-coder/src/application/tool_contracts.rs`

- [ ] **Step 1: Write failing public contract tests**

```rust
#[test]
fn runtime_job_tools_are_listed_and_keep_runtime_execute_unchanged() {
    let names = tools().into_iter().map(|tool| tool.name).collect::<Vec<_>>();
    assert!(names.contains(&"unica.runtime.job.start"));
    assert!(names.contains(&"unica.runtime.job.status"));
    assert!(names.contains(&"unica.runtime.job.wait"));
    assert!(names.contains(&"unica.runtime.job.logs"));
    assert!(names.contains(&"unica.runtime.job.cancel"));
    assert!(names.contains(&"unica.runtime.job.list"));
    assert!(names.contains(&"unica.runtime.execute"));
}

#[test]
fn runtime_job_start_schema_reuses_typed_runtime_arguments() {
    let schema = schema("unica.runtime.job.start");
    assert_eq!(schema["additionalProperties"], false);
    assert!(schema["properties"].get("operation").is_some());
    assert!(schema["properties"].get("args").is_none());
}

#[test]
fn runtime_job_wait_requires_job_id_and_bounded_timeout_seconds() {
    let tool = tool("unica.runtime.job.wait");
    assert!(validate_tool_arguments(tool, &json!({}).as_object().unwrap().clone(), false).is_err());
    assert!(validate_tool_arguments(tool, &json!({"jobId":"00000000-0000-4000-8000-000000000001","timeoutSeconds":0}).as_object().unwrap().clone(), false).is_err());
    assert!(validate_tool_arguments(tool, &json!({"jobId":"00000000-0000-4000-8000-000000000001","timeoutSeconds":61}).as_object().unwrap().clone(), false).is_err());
}
```

- [ ] **Step 2: Run the contract tests and verify RED**

Run: `cargo test --locked -p unica-coder runtime_job_ -- --nocapture`

Expected: FAIL because none of the new tools/schemas exists.

- [ ] **Step 3: Add the public boundary without altering existing runtime semantics**

```rust
#[derive(Debug, Clone, Copy)]
pub enum RuntimeJobAction { Start, Status, Wait, Logs, Cancel, List }

pub(crate) struct HandlerOutcome {
    pub(crate) adapter: AdapterOutcome,
    pub(crate) job: Option<Value>,
}
```

Add `ToolHandler::RuntimeJob { action: RuntimeJobAction }`, six `ToolSpec`s, and optional `job: Option<Value>` to `OperationResult`. Change only `ApplicationPorts::invoke_handler` to return `HandlerOutcome`; wrap every old handler with `HandlerOutcome::plain` so current behavior is byte-for-byte equivalent. `RuntimeJobAdapter` resolves bundled `v8-runner`, builds both actual and redacted typed argv with existing `runtime_args`, and calls `RuntimeJobService`; no raw argument path is introduced.

`tool_contracts` must allow start's existing runtime fields and validate it through `validate_runtime_arguments`; status/cancel/logs/wait only allow `jobId` plus their own typed argument. Require `jobId` for four tools, type `timeoutSeconds`/`tailChars` as integer, enforce 1..60 seconds and 1..32768 chars. `write_path_args` treats Start exactly like `RuntimeAdapter`.

- [ ] **Step 4: Run contract and application tests, then commit**

Run: `cargo test --locked -p unica-coder runtime_job -- --nocapture`

Expected: PASS; no schema accepts `args`, no job control command accepts runtime execution arguments, and `OperationResult.job` is serialized only for job tools.

```bash
git add crates/unica-coder/src/application/{mod.rs,ports.rs,tool_contracts.rs} crates/unica-coder/src/infrastructure/internal_adapters.rs
git commit -m "feat: открыть typed runtime job lifecycle"
```

### Task 4: Worker executable, cache completion and end-to-end verification

**Files:**
- Modify: `crates/unica-coder/src/main.rs:1-19`
- Modify: `crates/unica-coder/src/infrastructure/runtime_jobs.rs`
- Modify: `crates/unica-coder/src/domain/events.rs`
- Modify: `crates/unica-coder/src/application/mod.rs:720-735`
- Test: `crates/unica-coder/src/interfaces/mcp.rs`
- Test: `crates/unica-coder/src/infrastructure/runtime_jobs.rs`

- [ ] **Step 1: Write failing worker handoff and cache-event tests**

```rust
#[test]
fn worker_request_never_persists_actual_connection_secret() {
    let fixture = JobFixture::new(FakeRunner::after_polls(0, JobExit::success("Pwd=output-secret\n")));
    let started = fixture.start_with_argv("build", &["--connection", "Pwd=request-secret"]).unwrap();
    let _ = fixture.wait_until_terminal(&started.job_id);
    let persisted = fixture.read_job_directory(&started.job_id).unwrap();
    assert!(!persisted.contains("request-secret"));
    assert!(!persisted.contains("output-secret"));
}

#[test]
fn successful_runtime_job_applies_same_event_kind_as_runtime_execute() {
    assert_eq!(runtime_event_kind("dump"), Some(DomainEventKind::SourceSetChanged));
    assert_eq!(runtime_event_kind("build"), Some(DomainEventKind::BuildCompleted));
}

#[test]
fn mcp_reconnect_reads_existing_job_snapshot() {
    let fixture = JobFixture::new(FakeRunner::after_polls(1, JobExit::success("ok\n")));
    let started = fixture.start("build").unwrap();
    let first = fixture.application().call_tool("unica.runtime.job.status", &fixture.status_args(&started.job_id)).unwrap();
    let second = fixture.reconnected_application().call_tool("unica.runtime.job.status", &fixture.status_args(&started.job_id)).unwrap();
    assert_eq!(first.job.unwrap()["jobId"], second.job.unwrap()["jobId"]);
}
```

- [ ] **Step 2: Run the tests and verify RED**

Run: `cargo test --locked -p unica-coder 'worker_request|runtime_job_applies|mcp_reconnect' -- --nocapture`

Expected: FAIL because main does not route the internal worker and runtime event mapping is not shared.

- [ ] **Step 3: Implement worker process handoff and terminal cache effect**

```rust
if args.iter().any(|arg| arg == "--runtime-job-worker") {
    if let Err(error) = unica_coder::infrastructure::runtime_jobs::run_worker_from_args(&args) {
        eprintln!("{error}");
        std::process::exit(1);
    }
    return;
}
```

Parent uses `current_exe`, `Stdio::piped` stdin and a JSON `WorkerStartRequest`; worker reads one request, closes stdin, and owns the actual child. Move runtime operation→`DomainEventKind` mapping into `domain::events::runtime_event_kind`; both synchronous application and worker use it. After `Succeeded`, worker calls `WorkspaceStateRepository::report` and `WorkspaceServiceManager::notify_invalidation`; it appends a redacted warning if cache reporting fails and never changes a terminal success into failure.

- [ ] **Step 4: Run focused verification, then full verification**

Run:

```bash
cargo fmt --check
cargo clippy --locked -p unica-coder --all-targets -- -D warnings
cargo test --locked -p unica-coder
git diff --check
```

Expected: every command exits 0. Then use a disposable File IB through public `unica` MCP only: start an allowed long runtime operation, poll by jobId from a new MCP client process, call `wait` with a bounded timeout, and assert record/log redaction without changing a user IB.

- [ ] **Step 5: Commit the integration slice**

```bash
git add crates/unica-coder/src/{main.rs,domain/events.rs,infrastructure/runtime_jobs.rs,application/mod.rs,interfaces/mcp.rs}
git commit -m "feat: завершать runtime jobs отдельным worker"
```

### Task 5: Independent review gates and branch delivery

**Files:**
- Review: full diff from `c3b4372` to HEAD
- Verify: Rust contracts, worker lifecycle tests, full Rust suite, Python/parity suite, public-MCP disposable-IB acceptance

- [ ] **Step 1: Spec review**

Dispatch a fresh reviewer with the exact design acceptance: six public tools; original execute untouched; worker survives MCP server; durable redacted record/logs; all eight phases; reconnect/wait/cancel/conflict/lost; cache mapping. Reviewer must identify missing or extra behavior with file/line evidence.

- [ ] **Step 2: Fix every spec finding and request re-review**

Run the exact focused test that proves each fix before asking reviewer again. Do not begin quality review until spec verdict is approved.

- [ ] **Step 3: Rust quality review**

Dispatch a fresh reviewer using `rust-expert-best-practices-code-review`. Require audit of no panic paths, `Result` propagation, exhaustive phase matches, owned/borrowed arguments, process/log boundedness, lock ownership, redaction and public compatibility.

- [ ] **Step 4: Run completion evidence and deliver normally**

Run all commands from Task 4 plus `uv run pytest -q` or repository's documented Python/parity command. Inspect clean `git status`, commit only verified changes, normal-push the feature branch to `korolevpavel/unica`, create/update a Russian draft PR to `IngvarConsulting/unica`, and leave the worktree intact. Do not merge or force-push.
