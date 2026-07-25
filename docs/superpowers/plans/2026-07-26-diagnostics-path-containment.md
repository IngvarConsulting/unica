# Diagnostics path containment Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Reject `unica.code.diagnostics` file paths outside the resolved source root before invoking `bsl-analyzer`.

**Architecture:** Keep the boundary at the BSL MCP adapter, where both the resolved source root and typed diagnostics payload are available. Add a small path resolver that canonically identifies an existing path ancestor (including symlinks), checks containment in `sourceDir`, and returns the path only after that check. The adapter will use it only for diagnostics requests containing `path`.

**Tech Stack:** Rust, `std::path`, existing `normalize_path_identity`, `BslAnalyzerMcpAdapter` unit tests.

## Global Constraints

- Preserve the existing source-root selection and graph-tool behavior.
- `mode=analyze` continues to reject `path` at the contract layer.
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

Add a table-driven test that invokes `unica.code.diagnostics` with `mode: "file"`, an explicit `sourceDir`, and each of these paths: `"../outside/Module.bsl"`, an absolute temporary file outside the source root, and `"escape/Module.bsl"` where `escape` is an in-root symlink to an outside temporary directory. For every supported symlink case, assert:

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
```

Expected: the test fails because the adapter currently forwards each path to the recording runner.

- [ ] **Step 3: Add the minimal source-root containment helper**

Import `Path` next to `PathBuf`. Implement `validate_diagnostics_path` beside `resolve_source_dir`:

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
cargo test -p unica-coder --lib diagnostics_mcp_adapter_builds_typed_request -- --nocapture
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
