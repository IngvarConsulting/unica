# Issue #89 Cancellable Workspace Service Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Propagate MCP cancellation through Unica and keep workspace-service control operations responsive while analyzer and RLM requests are running.

**Architecture:** Turn the stdio interface into a request dispatcher with a cancellation registry, pass tokens through application ports, and add operation IDs plus cancel messages to the internal protocol. The workspace service uses a concurrent listener, independent RLM jobs, and one mutex-protected warm analyzer lane. `ManagedChild` from the preceding plan owns the persistent analyzer process tree.

**Tech Stack:** Rust 2021 threads/channels/Arc/Mutex/atomics, JSON-RPC over stdio, internal localhost JSONL, existing MCP and CI smoke tests.

## Global Constraints

- Requires completed plans `2026-07-16-issue-89-search-source-root.md` and `2026-07-16-issue-89-managed-child.md`.
- Preserve one public server `unica`; do not expose workspace control as public tools.
- `ping`, `cancel`, and `shutdown` must not wait for analyzer or RLM work.
- `shutdown` cancels active operations, rejects new work, removes `service.json`, and exits.
- A cancelled operation must not require a service restart before the next call.
- Mutable access to one warm `bsl-analyzer` session remains serialized.
- Controlled failures preserve the prefixes `invalid_source_root:`, `timeout:`, `cancelled:`, `backend_unavailable:`, and `process_failed:`.

---

## File Structure

- Modify `crates/unica-coder/src/application/mod.rs`: `Arc` application ports and cancellable call entry point.
- Modify `crates/unica-coder/src/application/ports.rs`: propagate request tokens through adapters.
- Modify `crates/unica-coder/src/interfaces/mcp.rs`: concurrent request dispatcher and cancellation registry.
- Modify `crates/unica-coder/src/infrastructure/internal_adapters.rs`: accept and forward request tokens.
- Modify `crates/unica-coder/src/infrastructure/workspace_index.rs`: accept request tokens for readiness commands.
- Modify `crates/unica-coder/src/infrastructure/workspace_services.rs`: cancellable connector, concurrent server runtime, managed persistent analyzer.
- Modify `spec/decisions/0006-workspace-scoped-internal-services.md`, runtime/acceptance specs, and CI smoke tests.

### Task 1: Cancellable application boundary

**Files:**
- Modify: `crates/unica-coder/src/application/mod.rs`
- Modify: `crates/unica-coder/src/application/ports.rs`
- Modify: `crates/unica-coder/src/infrastructure/internal_adapters.rs`
- Modify: `crates/unica-coder/src/infrastructure/workspace_index.rs`
- Test: `crates/unica-coder/src/application/mod.rs`

**Interfaces:**
- Consumes: `CancellationToken` and cancellable command types from plan 2.
- Produces: `UnicaApplication::call_tool_cancellable(name, args, token)` and `ApplicationPorts: Send + Sync`.

- [ ] **Step 1: Write a failing application propagation test**

Use a recording port backed by `Mutex` (not `RefCell`, because application ports become `Send + Sync`) that stores `token.is_cancelled()` in `invoke_handler`. Cancel before calling and assert the port sees it:

```rust
let token = CancellationToken::new();
token.cancel();
let app = UnicaApplication::with_ports(Arc::new(RecordingPorts::default()));
let result = app.call_tool_cancellable("unica.code.search", &args, token).unwrap();
assert!(result.errors.iter().any(|error| error.contains("cancelled")));
```

- [ ] **Step 2: Run and verify RED**

Run: `cargo test -p unica-coder call_tool_cancellable -- --nocapture`

Expected: FAIL because the cancellable entry point does not exist.

- [ ] **Step 3: Add the application API and thread-safety bounds**

Store `ports` as `Arc<dyn ApplicationPorts + Send + Sync>`. Update test doubles from `RefCell`/`Cell` to `Mutex`/atomics as required by these bounds. Keep `call_tool` as a compatibility wrapper that creates a fresh token. Add the token to `ApplicationPorts::invoke_handler` and every adapter invocation.

```rust
pub fn call_tool_cancellable(
    &self,
    name: &str,
    args: &Map<String, Value>,
    cancellation: CancellationToken,
) -> Result<OperationResult, String> {
    let spec = find_tool(name)?;
    call_tool(spec, args, self.ports.as_ref(), &cancellation)
}
```

Before starting work, adapters return a controlled `cancelled` error if the token is already set. Pass the same token into `ProcessCommand`, `IndexCommand`, and workspace-service manager calls.

- [ ] **Step 4: Run application and adapter suites**

Run: `cargo test -p unica-coder application -- --nocapture`

Run: `cargo test -p unica-coder infrastructure::internal_adapters -- --nocapture`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/unica-coder/src/application/mod.rs crates/unica-coder/src/application/ports.rs crates/unica-coder/src/infrastructure/internal_adapters.rs crates/unica-coder/src/infrastructure/workspace_index.rs
git commit -m "feat: propagate tool cancellation tokens"
```

### Task 2: Concurrent stdio MCP dispatcher

**Files:**
- Modify: `crates/unica-coder/src/interfaces/mcp.rs`
- Test: `crates/unica-coder/src/interfaces/mcp.rs`

**Interfaces:**
- Consumes: `UnicaApplication::call_tool_cancellable`.
- Produces: `run_stdio_with(reader, writer, app)`, `CancellationRegistry`, support for `notifications/cancelled`.

- [ ] **Step 1: Write failing dispatcher tests**

Refactor the stdio loop behind injectable buffered reader/writer types. Add a blocking fake tool and feed these lines without closing input: initialize, blocking `tools/call` id 7, `ping` id 8, and `notifications/cancelled` with `requestId: 7`. Assert ping is emitted before the cancelled response and the fake observes cancellation.

```rust
{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"unica.code.search","arguments":{}}}
{"jsonrpc":"2.0","id":8,"method":"ping"}
{"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":7,"reason":"test"}}
```

- [ ] **Step 2: Run and verify RED**

Run: `cargo test -p unica-coder mcp_dispatcher -- --nocapture`

Expected: FAIL or hang under a two-second test deadline because the current loop executes id 7 inline.

- [ ] **Step 3: Implement the registry and dispatcher**

Use an `Arc<Mutex<HashMap<String, CancellationToken>>>` represented by a small type with `register`, `cancel`, `complete`, and `cancel_all`. Convert JSON-RPC IDs to a stable key with `serde_json::to_string(id)` so numeric and string IDs remain distinct. The input thread handles initialize/list/ping synchronously and spawns one worker per `tools/call`. Workers serialize responses through `Arc<Mutex<W>>` and remove their registry entry in a guard.

```rust
match method {
    "notifications/cancelled" => {
        if let Some(id) = message.pointer("/params/requestId") {
            registry.cancel(id);
        }
    }
    "tools/call" => dispatch_tool_call(Arc::clone(&app), message, registry.clone(), writer.clone()),
    _ => write_response(writer.clone(), handle_control_message(&app, message)),
}
```

On stdin EOF or writer failure, call `cancel_all`. A cancelled request returns JSON-RPC error code `-32800` with message `request cancelled`; emit at most one response per request.

- [ ] **Step 4: Run MCP unit and smoke tests**

Run: `cargo test -p unica-coder interfaces::mcp -- --nocapture`

Run: `python -m unittest discover -s tests/ci -p 'test_unica_mcp_smoke.py'`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/unica-coder/src/interfaces/mcp.rs tests/ci/test_unica_mcp_smoke.py
git commit -m "feat: dispatch and cancel concurrent mcp calls"
```

### Task 3: Internal operation IDs and cancellable connector

**Files:**
- Modify: `crates/unica-coder/src/infrastructure/workspace_services.rs`
- Test: `crates/unica-coder/src/infrastructure/workspace_services.rs`

**Interfaces:**
- Consumes: request cancellation tokens.
- Produces: `operation_id` on `BslMcp`/`RlmReady`, `ServiceRequestKind::Cancel { operation_id }`, and `ServiceConnector::send(..., &CancellationToken)`.

- [ ] **Step 1: Write failing connector tests**

Use a local test listener that accepts a work request but never responds, then records a second connection. Cancel the token and assert that the second request is `cancel` for the same operation ID and the original call returns `cancelled` before two seconds.

```rust
assert_eq!(cancel.kind, ServiceRequestKind::Cancel { operation_id: work_id.clone() });
assert!(error.contains("cancelled"));
```

- [ ] **Step 2: Run and verify RED**

Run: `cargo test -p unica-coder cancellable_connector -- --nocapture`

Expected: FAIL because connector reads one response with a 120-second timeout and has no cancel message.

- [ ] **Step 3: Extend the internal protocol**

Generate operation IDs with `Uuid::new_v4().to_string()`. Add them to work variants and add `Cancel`. Change connector reads to a 100 ms socket timeout loop. On token cancellation, send a separate best-effort cancel request and return `cancelled`; on ordinary requests retain the overall `SERVICE_REQUEST_TIMEOUT`.

```rust
if cancellation.is_cancelled() {
    let _ = self.send_control(record, ServiceRequestKind::Cancel {
        operation_id: operation_id.to_string(),
    });
    return Err("workspace service request cancelled".to_string());
}
```

Treat `WouldBlock` and `TimedOut` as polling events, EOF as disconnect, and a complete JSON line as the response. Control calls use a fresh uncancelled token and the shorter connect timeout.

- [ ] **Step 4: Run connector tests**

Run: `cargo test -p unica-coder workspace_services::tests -- --nocapture`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/unica-coder/src/infrastructure/workspace_services.rs
git commit -m "feat: cancel workspace service operations"
```

### Task 4: Concurrent workspace-service runtime

**Files:**
- Modify: `crates/unica-coder/src/infrastructure/workspace_services.rs`
- Test: `crates/unica-coder/src/infrastructure/workspace_services.rs`

**Interfaces:**
- Consumes: internal operation IDs, cancellation tokens, existing `WorkspaceIndexService`.
- Produces: shared `WorkspaceServiceRuntime` and independent control path.

- [ ] **Step 1: Write failing protocol concurrency tests**

Start the real listener with a fake blocking RLM runner/runtime hook. Send a work request, then on separate connections send ping and cancel. Assert ping completes within 500 ms, the work response is cancelled, and a subsequent work request succeeds. Add a shutdown test that starts two operations and asserts both tokens are cancelled and new work is rejected.

- [ ] **Step 2: Run and verify RED**

Run: `cargo test -p unica-coder workspace_service_control_path -- --nocapture`

Expected: FAIL because `handle_stream` blocks the accept loop.

- [ ] **Step 3: Introduce shared runtime state**

Replace mutable `WorkspaceServiceState` with:

```rust
struct WorkspaceServiceRuntime {
    identity: WorkspaceServiceIdentity,
    token: String,
    context: WorkspaceContext,
    analyzer: Mutex<Option<BslMcpSession>>,
    source_generation: Mutex<u64>,
    operations: Mutex<HashMap<String, CancellationToken>>,
    shutting_down: AtomicBool,
}
```

The listener remains nonblocking but spawns one scoped thread per accepted stream. `Ping` reads only `shutting_down`. `Cancel` looks up and flips one token. `Shutdown` atomically marks shutdown, snapshots and cancels every operation, and lets the listener exit after active handler threads drain within the configured grace period.

Work registration must reject duplicate operation IDs and all work after shutdown. Use an RAII `OperationGuard` to remove IDs on every return path.

For a work connection, run the operation in a worker and let the connection handler wait on its result channel in 50 ms intervals. Between intervals, use nonblocking `TcpStream::peek`: `Ok(0)` means the caller disconnected and cancels that operation; `WouldBlock` means the connection is still open. This covers process/client loss even when no explicit cancel message arrives.

- [ ] **Step 4: Implement independent lanes**

RLM handlers run without the analyzer mutex and pass their operation token to index commands. Analyzer handlers lock only `analyzer`, verify source generation, start/reuse the session, and call it with the operation token. A cancelled analyzer call drops the session so the next request starts cleanly.

- [ ] **Step 5: Run concurrency tests**

Run: `cargo test -p unica-coder workspace_service -- --nocapture`

Expected: PASS with ping/cancel/shutdown deadlines.

- [ ] **Step 6: Commit**

```bash
git add crates/unica-coder/src/infrastructure/workspace_services.rs
git commit -m "feat: keep workspace control path responsive"
```

### Task 5: Manage the persistent analyzer tree

**Files:**
- Modify: `crates/unica-coder/src/infrastructure/managed_child.rs`
- Modify: `crates/unica-coder/src/infrastructure/workspace_services.rs`
- Test: `crates/unica-coder/src/infrastructure/workspace_services.rs`

**Interfaces:**
- Consumes: `ManagedChild` and operation cancellation.
- Produces: persistent-child accessors `take_stdin`, `take_stdout`, `take_stderr`, and bounded `terminate` used by `BslMcpSession`.

- [ ] **Step 1: Write a failing cancelled-session test**

Start a fixture JSONL child that completes initialize but ignores `tools/call` and spawns a descendant. Cancel the operation; assert the call returns `cancelled`, both PIDs die, and dropping the session returns within two seconds.

- [ ] **Step 2: Run and verify RED**

Run: `cargo test -p unica-coder cancelled_bsl_session -- --nocapture`

Expected: FAIL because `BslMcpSession::drop` uses immediate-child `kill`, blocking `wait`, and unbounded reader `join`.

- [ ] **Step 3: Move session process ownership to ManagedChild**

Change `BslMcpSession.child` to `ManagedChild`. Let the session own its protocol stdout receiver, but let `ManagedChild` own tree termination and bounded reader cleanup. Change `read_json_response` to check the operation token every 50 ms:

```rust
if cancellation.is_cancelled() {
    return Err("persistent bsl-analyzer request cancelled".to_string());
}
```

On timeout/cancel/protocol disconnect, call `terminate`, discard the session, and never join an unbounded reader thread.

- [ ] **Step 4: Run analyzer-session and workspace-service tests**

Run: `cargo test -p unica-coder bsl_session -- --nocapture`

Run: `cargo test -p unica-coder workspace_service -- --nocapture`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/unica-coder/src/infrastructure/managed_child.rs crates/unica-coder/src/infrastructure/workspace_services.rs
git commit -m "fix: cancel persistent analyzer process tree"
```

### Task 6: Multi-source-set end-to-end regression

**Files:**
- Create: `crates/unica-coder/tests/issue_89_workspace_service.rs`
- Modify: `tests/ci/test_unica_mcp_smoke.py`
- Test: same files.

**Interfaces:**
- Consumes: public dispatcher, shared source resolver, cancellable service runtime.
- Produces: regression coverage for the original issue sequence.

- [ ] **Step 1: Add the fixture and failing scenario**

Create a temporary workspace with:

```yaml
format: DESIGNER
source-set:
  main:
    type: CONFIGURATION
    path: src/cf
  TESTS:
    type: CONFIGURATION
    path: exts/TESTS
```

Provide minimal `Configuration.xml` and module fixtures. The integration test compiles one tiny Rust fake-tool source with `rustc`, copies the resulting executable under the required bundled-tool names in a temporary plugin layout, and points `UNICA_PLUGIN_ROOT` at that layout. The fake dispatches behavior from its executable name/arguments, records source roots and child PIDs, and implements only the analyzer/RLM commands exercised by the scenario. Send parallel `unica.code.search` and `unica.meta.profile`, cancel one, then send ping and a final `meta.profile` to `env!("CARGO_BIN_EXE_unica")`. Assert every response arrives under the test deadline, commands record `src/cf`, and no fake child PID survives.

- [ ] **Step 2: Run the scenario before completing fixtures**

Run: `cargo test -p unica-coder --test issue_89_workspace_service -- --nocapture`

Expected: FAIL until the fixture programs and environment injection cover both backends.

- [ ] **Step 3: Complete deterministic test seams**

Use `UNICA_PLUGIN_ROOT` and the normal bundled-tool directory contract; do not add a production raw executable argument or a `#[cfg(test)]` lookup branch. Ensure the child process owns a temporary `UNICA_CACHE_DIR`. Environment changes happen only on spawned `Command` values, not process-global variables, so parallel Rust tests remain isolated.

- [ ] **Step 4: Run the end-to-end regression repeatedly**

Run: `cargo test -p unica-coder --test issue_89_workspace_service -- --nocapture`

Run the same command three consecutive times.

Expected: all three runs PASS; no timeout and no surviving recorded PID.

- [ ] **Step 5: Commit**

```bash
git add crates/unica-coder/tests/issue_89_workspace_service.rs tests/ci/test_unica_mcp_smoke.py
git commit -m "test: cover issue 89 cancellation regression"
```

### Task 7: Architecture, acceptance, and final verification

**Files:**
- Modify: `spec/decisions/0006-workspace-scoped-internal-services.md`
- Modify: `spec/architecture/arc42/06-runtime-view.md`
- Modify: `spec/acceptance/unica-mcp-validation.md`
- Modify: any tests that enforce these contracts.

**Interfaces:**
- Consumes: final runtime behavior from Tasks 1-6.
- Produces: current architecture and operator-visible acceptance criteria.

- [ ] **Step 1: Update documentation with exact guarantees**

Document operation IDs, independent control handling, JSON-RPC cancellation propagation, platform process-tree ownership, deterministic source selection, and the rule that bundled-tool contract tests derive versions from `tools.lock.json`. Remove statements implying a fully sequential workspace service.

- [ ] **Step 2: Run documentation/contract tests**

Run: `python -m unittest discover -s tests/ci`

Expected: PASS.

- [ ] **Step 3: Run the complete verification gate**

Run: `cargo fmt --all -- --check`

Run: `cargo clippy -p unica-coder --all-targets -- -D warnings`

Run: `cargo test -p unica-coder`

Run: `python -m unittest discover -s tests/ci`

Run: `git diff --check`

Expected: every command PASS.

- [ ] **Step 4: Inspect process cleanup manually on Windows**

Run the issue-89 integration test, then execute:

```powershell
Get-Process rlm-bsl-index,bsl-analyzer -ErrorAction SilentlyContinue
```

Expected: no process created by the test remains. Pre-existing user processes must not be terminated by the test.

- [ ] **Step 5: Commit**

```bash
git add spec/decisions/0006-workspace-scoped-internal-services.md spec/architecture/arc42/06-runtime-view.md spec/acceptance/unica-mcp-validation.md
git commit -m "docs: specify cancellable workspace services"
```
