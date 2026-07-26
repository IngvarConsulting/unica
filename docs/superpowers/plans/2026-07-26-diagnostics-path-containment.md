# Diagnostics path containment Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Accept `unica.code.diagnostics` `path` only for file mode and reject malformed or out-of-root file paths before invoking `bsl-analyzer`.

**Architecture:** Enforce mode applicability in the application contract, where the diagnostics mode is resolved, and enforce type and source-root containment again at the BSL MCP adapter before runner selection. Add a small path resolver that canonically identifies an existing path ancestor (including symlinks), checks containment in `sourceDir`, and returns the path only after that check.

**Tech Stack:** Rust, `std::path`, existing `normalize_path_identity`, `tool_contracts` tests, `BslAnalyzerMcpAdapter` unit tests.

## Global Constraints

- Preserve the existing source-root selection and graph-tool behavior.
- Permit `path` only for `mode=file`; omitted mode resolves to `analyze` and rejects `path`.
- Reject every present non-string `path` before selecting or invoking a runner.
- Reject relative `..`, absolute external paths, and symlink escapes with `invalid_diagnostics_path:` before the BSL MCP runner is called.
- Preserve the typed diagnostics payload for valid source-root-relative file paths.

---

### Task 1: Enforce diagnostics-file containment

**Files:**
- Modify: `crates/unica-coder/src/infrastructure/internal_adapters.rs:19, 2218-2225, 3214-3220, 6600-6640`
- Test: `crates/unica-coder/src/infrastructure/internal_adapters.rs:6600-6640`

**Interfaces:**
- Consumes: `resolve_source_dir(context, args) -> Result<PathBuf, String>` and `normalize_path_identity(path) -> Result<PathBuf, String>`.
- Produces: `validate_diagnostics_path(source_dir: &Path, raw_path: &str) -> Result<(), String>`, whose errors start with `invalid_diagnostics_path:`.
- Preserves: `BslAnalyzerMcpAdapter::invoke("unica.code.diagnostics", ...)` builds its existing `diagnostics` MCP request only after validation.

- [ ] **Step 1: Write the failing adapter tests**

Add a table-driven test that invokes `unica.code.diagnostics` with `mode: "file"`, an explicit `sourceDir`, and each of these paths: `"../outside/Module.bsl"`, an absolute temporary file outside the source root, and `"escape/Module.bsl"` where `escape` is an in-root symlink to an outside temporary directory. Add a second test covering present non-string JSON values. For every rejected case, assert:

```rust
let error = BslAnalyzerMcpAdapter::with_runner(&runner)
    .invoke("unica.code.diagnostics", &args, &context, false)
    .unwrap_err();
assert!(error.starts_with("invalid_diagnostics_path:"), "{error}");
assert!(runner.commands.borrow().is_empty());
```

In the existing valid typed-diagnostics test, add an assertion that its relative `path` remains accepted and is copied as `tool_args["path"]`.

- [ ] **Step 2: Run the new test and verify it fails**

Run:

```bash
cargo test -p unica-coder --lib diagnostics_mcp_adapter_rejects_paths_outside_source_dir -- --nocapture
cargo test -p unica-coder --lib diagnostics_mcp_adapter_rejects_non_string_path_before_runner -- --nocapture
```

Expected: the tests fail because the adapter currently forwards each path to the recording runner.

- [ ] **Step 3: Add the minimal source-root containment helper**

Import `Path` next to `PathBuf`. Before selecting the diagnostics runner, reject
every present non-string `path` with an `invalid_diagnostics_path:` error.
Implement `validate_diagnostics_path` beside `resolve_source_dir`:

```rust
fn validate_diagnostics_path(source_dir: &Path, raw_path: &str) -> Result<(), String> {
    let raw_path = Path::new(raw_path);
    let candidate = if raw_path.is_absolute() {
        raw_path.to_path_buf()
    } else {
        source_dir.join(raw_path)
    };
    let path = normalize_path_identity(&candidate)
        .map_err(|error| format!("invalid_diagnostics_path: {error}"))?;
    let source_dir = normalize_path_identity(source_dir)
        .map_err(|error| format!("invalid_diagnostics_path: {error}"))?;
    if path.starts_with(&source_dir) {
        Ok(())
    } else {
        Err(format!(
            "invalid_diagnostics_path: path {} is outside sourceDir {}",
            path.display(),
            source_dir.display()
        ))
    }
}
```

After resolving `source_dir` in `BslAnalyzerMcpAdapter::invoke_cancellable`, call the helper when `tool_name == "unica.code.diagnostics"` and the typed arguments contain a string `path`. Do this before `bsl_mcp_tool_request` and before constructing `BslMcpCommand`.

- [ ] **Step 4: Run the focused tests**

Run:

```bash
cargo test -p unica-coder --lib diagnostics_mcp_adapter_rejects_paths_outside_source_dir -- --nocapture
cargo test -p unica-coder --lib diagnostics_mcp_adapter_rejects_non_string_path_before_runner -- --nocapture
cargo test -p unica-coder --lib bsl_diagnostics_adapter_maps_file_mode_to_allowlisted_mcp_call -- --nocapture
```

Expected: rejected cases return the stable prefix without a runner command; the valid file request retains its original typed payload.

- [ ] **Step 5: Run full verification and commit**

Run:

```bash
cargo fmt --all --check
cargo test -p unica-coder --lib --quiet -- --test-threads=1
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

Commit:

```bash
git add crates/unica-coder/src/infrastructure/internal_adapters.rs
git commit -m "fix(diagnostics): contain file paths in source root"
```

---

### Task 2: Restrict `path` to file diagnostics

**Files:**
- Modify: `crates/unica-coder/src/application/tool_contracts.rs:1083-1118`
- Test: `crates/unica-coder/src/application/tool_contracts.rs:3023-3090`

**Interfaces:**
- Resolves: an omitted diagnostics `mode` as `analyze`.
- Rejects: `path` for default or explicit `analyze`, `status`, `catalog`, and `workspace`.
- Preserves: `mode=file` requires `path` for non-dry-run requests.

- [ ] **Step 1: Write the failing contract tests**

Extend `bsl_diagnostics_contract_exposes_modes_and_keeps_analyze_default` with
literal cases that supply `path` for explicit `analyze`, `status`, `catalog`,
and `workspace` requests. Assert that every case reports that `path` is only
supported for `file`; keep the existing omitted-mode `analyze` default.

- [ ] **Step 2: Run the contract test and verify it fails**

Run:

```bash
cargo test -p unica-coder --lib bsl_diagnostics_contract_exposes_modes_and_keeps_analyze_default -- --nocapture
```

Expected: non-file modes other than `analyze` accept `path`.

- [ ] **Step 3: Enforce the file-only contract**

Resolve the diagnostics mode once, defaulting it to `analyze`. If `path` is
present and the mode is not `file`, return the file-only contract error before
the timeout and required-argument checks.

- [ ] **Step 4: Run the focused contract test**

Run:

```bash
cargo test -p unica-coder --lib bsl_diagnostics_contract_exposes_modes_and_keeps_analyze_default -- --nocapture
```

Expected: default and explicit non-file modes reject `path`, while file mode
retains its required-path behavior.

- [ ] **Step 5: Run full verification and commit**

Run:

```bash
cargo fmt --all --check
cargo test -p unica-coder --lib --quiet -- --test-threads=1
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

Commit:

```bash
git add crates/unica-coder/src/application/tool_contracts.rs
git commit -m "fix(diagnostics): reject path outside file mode"
```
