# Issue #89 Search and Source Root Implementation Plan

> **Historical execution record.** Current requirements live in code, tests,
> package metadata, and `spec/`; this completed plan is retained for traceability.

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Remove the invalid analyzer search backend and make every workspace analyzer/index operation choose the same deterministic 1C source root.

**Architecture:** Add source selection to the project-source domain model and expose one resolver consumed by infrastructure adapters. Keep `unica.project.map` read-only while reporting the effective selection. Normalize path identity before workspace-service keys are calculated.

**Tech Stack:** Rust 2021, serde/serde_json, existing Rust unit tests, Python unittest contract gates.

## Global Constraints

- Keep one public MCP server named `unica` and only `unica.*` public tools.
- Do not persist source selection as call-order-dependent session state.
- Selection order is explicit `sourceDir`, source set named `main`, then the only configuration source set; ambiguity is an error.
- Do not hard-code a `bsl-analyzer` version in contract tests; `tools.lock.json` is the version source of truth.
- Source selection failures use the stable prefix `invalid_source_root:`.
- Do not scan `docs/research`, `docs/its`, `target`, `.build`, or `dist`.

---

## File Structure

- Create `crates/unica-coder/src/domain/source_roots.rs`: source-set selection, workspace containment, and stable path identity.
- Modify `crates/unica-coder/src/domain/mod.rs`: export the resolver module.
- Modify `crates/unica-coder/src/domain/project_sources.rs`: report effective selection in `ProjectSourceMap`.
- Modify `crates/unica-coder/src/application/mod.rs`: surface selection warnings from `project.map`.
- Modify `crates/unica-coder/src/infrastructure/internal_adapters.rs`: use the resolver and remove analyzer text search.
- Modify `crates/unica-coder/src/infrastructure/workspace_index.rs`: delegate source-root resolution.
- Modify `crates/unica-coder/src/infrastructure/workspace_services.rs`: normalize service identity paths.
- Modify `scripts/ci/check-tool-contracts.py` and `tests/ci/test_product_contracts.py`: check only CLI operations Unica actually invokes.

### Task 1: Deterministic source-root resolver

**Files:**
- Create: `crates/unica-coder/src/domain/source_roots.rs`
- Modify: `crates/unica-coder/src/domain/mod.rs`
- Test: `crates/unica-coder/src/domain/source_roots.rs`

**Interfaces:**
- Consumes: `WorkspaceContext`, `discover_project_source_map`, `ProjectSourceSet`, `SourceSetKind`.
- Produces: `ResolvedSourceRoot { source_set: Option<String>, path: PathBuf }`, `select_default_source_set(&[ProjectSourceSet]) -> Result<&ProjectSourceSet, String>`, `resolve_source_root(&WorkspaceContext, Option<&str>) -> Result<ResolvedSourceRoot, String>`, and `normalize_path_identity(&Path) -> Result<PathBuf, String>`.

- [ ] **Step 1: Write failing selection tests**

Add tests covering explicit `sourceDir`, `main`, the sole configuration, ambiguity, and escape from the workspace:

```rust
#[test]
fn selects_main_before_other_configurations() {
    let context = fixture(&[
        ("main", "CONFIGURATION", "src/cf"),
        ("TESTS", "CONFIGURATION", "exts/TESTS"),
    ]);
    let selected = resolve_source_root(&context, None).unwrap();
    assert_eq!(selected.source_set.as_deref(), Some("main"));
    assert_eq!(selected.path, context.workspace_root.join("src/cf"));
}

#[test]
fn rejects_ambiguous_configurations_without_main() {
    let context = fixture(&[
        ("app", "CONFIGURATION", "app"),
        ("tests", "CONFIGURATION", "tests"),
    ]);
    let error = resolve_source_root(&context, None).unwrap_err();
    assert!(error.contains("sourceDir"));
    assert!(error.contains("app"));
    assert!(error.contains("tests"));
}
```

- [ ] **Step 2: Run the tests and verify RED**

Run: `cargo test -p unica-coder domain::source_roots -- --nocapture`

Expected: FAIL because `domain::source_roots` and its public interfaces do not exist.

- [ ] **Step 3: Implement the resolver**

Implement the public shape below; keep helper functions private:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedSourceRoot {
    pub source_set: Option<String>,
    pub path: PathBuf,
}

pub fn resolve_source_root(
    context: &WorkspaceContext,
    explicit: Option<&str>,
) -> Result<ResolvedSourceRoot, String> {
    if let Some(raw) = explicit.filter(|value| !value.trim().is_empty()) {
        return resolve_explicit(context, raw);
    }
    let map = discover_project_source_map(&context.workspace_root)?;
    let selected = select_default_source_set(&map.source_sets)?;
    Ok(ResolvedSourceRoot {
        source_set: Some(selected.name.clone()),
        path: normalize_path_identity(&context.workspace_root.join(&selected.path))?,
    })
}
```

`resolve_explicit` must normalize an absolute path resolved from `context.cwd`, normalize the workspace root, enforce `path.starts_with(workspace)`, and return `source_set` by matching the path against configured source sets when possible.

- [ ] **Step 4: Run resolver tests and full domain tests**

Run: `cargo test -p unica-coder domain:: -- --nocapture`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/unica-coder/src/domain/mod.rs crates/unica-coder/src/domain/source_roots.rs
git commit -m "feat: resolve effective 1c source root"
```

### Task 2: Stable Windows path identity

**Files:**
- Modify: `crates/unica-coder/src/domain/source_roots.rs`
- Modify: `crates/unica-coder/src/infrastructure/workspace_services.rs`
- Test: `crates/unica-coder/src/domain/source_roots.rs`
- Test: `crates/unica-coder/src/infrastructure/workspace_services.rs`

**Interfaces:**
- Consumes: `normalize_path_identity(&Path)` from Task 1.
- Produces: service keys based on normalized `workspace_root` and `source_root` strings.

- [ ] **Step 1: Write failing identity tests**

```rust
#[cfg(windows)]
#[test]
fn extended_length_and_regular_paths_have_same_identity() {
    let root = temp_workspace("path-identity");
    let regular = normalize_path_identity(&root).unwrap();
    let extended = PathBuf::from(format!(r"\\?\{}", root.display()));
    assert_eq!(regular, normalize_path_identity(&extended).unwrap());
}

#[test]
fn service_identity_reuses_normalized_paths() {
    let context = test_context("normalized-identity");
    let plain = WorkspaceServiceIdentity::new(&context, &context.workspace_root.join("src")).unwrap();
    let dotted = WorkspaceServiceIdentity::new(&context, &context.workspace_root.join("src/./")).unwrap();
    assert_eq!(plain.key, dotted.key);
}
```

- [ ] **Step 2: Run tests and verify RED**

Run: `cargo test -p unica-coder path_identity normalized_paths -- --nocapture`

Expected: at least one test FAILS because `canonical_display` preserves platform-specific aliases.

- [ ] **Step 3: Complete normalization and use it for service identity**

`normalize_path_identity` must make relative paths absolute, lexically remove `.`/`..`, canonicalize existing paths, and on Windows translate `\\?\UNC\server\share` to `\\server\share` and `\\?\C:\...` to `C:\...`. Change `WorkspaceServiceIdentity::new` to call this function for both roots before hashing.

```rust
let workspace_root = normalize_path_identity(&context.workspace_root)?;
let source_root = normalize_path_identity(source_root)?;
let workspace_root = workspace_root.display().to_string();
let source_root = source_root.display().to_string();
```

- [ ] **Step 4: Run identity tests**

Run: `cargo test -p unica-coder identity -- --nocapture`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/unica-coder/src/domain/source_roots.rs crates/unica-coder/src/infrastructure/workspace_services.rs
git commit -m "fix: normalize workspace service path identity"
```

### Task 3: Report effective selection from project.map

**Files:**
- Modify: `crates/unica-coder/src/domain/project_sources.rs`
- Modify: `crates/unica-coder/src/application/mod.rs`
- Test: `crates/unica-coder/src/application/mod.rs`

**Interfaces:**
- Consumes: deterministic default selection rules from Task 1.
- Produces: JSON fields `effectiveSourceSet`, `effectiveSourceRoot`, and optional `sourceSelectionError`.

- [ ] **Step 1: Extend the project.map test first**

```rust
assert!(stdout.contains(r#""effectiveSourceSet": "main""#));
assert!(stdout.contains(r#""effectiveSourceRoot""#));
assert!(!stdout.contains("sourceSelectionError"));
```

Add a second fixture with two configuration source sets and no `main`; assert that map discovery remains successful, both candidates are present, and `sourceSelectionError` asks for `sourceDir`.

- [ ] **Step 2: Run the focused tests and verify RED**

Run: `cargo test -p unica-coder project_map_reports -- --nocapture`

Expected: FAIL because the new JSON fields are absent.

- [ ] **Step 3: Add selection fields without hidden state**

Extend `ProjectSourceMap`:

```rust
#[serde(skip_serializing_if = "Option::is_none")]
pub effective_source_set: Option<String>,
#[serde(skip_serializing_if = "Option::is_none")]
pub effective_source_root: Option<String>,
#[serde(skip_serializing_if = "Option::is_none")]
pub source_selection_error: Option<String>,
```

Populate them during discovery using `select_default_source_set`; this helper only examines the already-built `source_sets` vector and therefore cannot recurse into project discovery. Do not write cache or service state. In `project_map`, copy `source_selection_error` into `outcome.warnings` while still returning the complete source-set JSON.

- [ ] **Step 4: Run project-source and application tests**

Run: `cargo test -p unica-coder project_map -- --nocapture`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/unica-coder/src/domain/project_sources.rs crates/unica-coder/src/application/mod.rs
git commit -m "feat: report effective project source root"
```

### Task 4: Route adapters and RLM through the resolver

**Files:**
- Modify: `crates/unica-coder/src/infrastructure/internal_adapters.rs`
- Modify: `crates/unica-coder/src/infrastructure/workspace_index.rs`
- Test: both modules above.

**Interfaces:**
- Consumes: `resolve_source_root(context, args.get("sourceDir").and_then(Value::as_str))`.
- Produces: analyzer and RLM operations keyed to exactly the same absolute root.

- [ ] **Step 1: Write failing multi-source-set routing tests**

Create a fixture with `main -> src/cf` and `TESTS -> exts/TESTS`. Assert that both adapter resolution and `WorkspaceIndexService` generate commands whose source directory is `src/cf`, not the workspace root.

```rust
let selected = resolve_source_dir(&context, &Map::new()).unwrap();
assert_eq!(selected, context.workspace_root.join("src/cf"));
```

- [ ] **Step 2: Run and verify RED**

Run: `cargo test -p unica-coder multi_source_set -- --nocapture`

Expected: FAIL with the current `context.cwd` fallback or independent `src/src-cf/root` heuristic.

- [ ] **Step 3: Replace both fallback implementations**

In `internal_adapters.rs`, reduce `resolve_source_dir` to the shared resolver. In `workspace_index.rs`, remove the heuristic `resolve_source_root` and call the shared resolver from command construction and readiness paths. Preserve explicit `path` only for APIs where it denotes a source root; do not reinterpret code-file `path` as a root.

```rust
fn resolve_source_dir(context: &WorkspaceContext, args: &Map<String, Value>) -> Result<PathBuf, String> {
    resolve_source_root(context, args.get("sourceDir").and_then(Value::as_str))
        .map(|resolved| resolved.path)
}
```

- [ ] **Step 4: Run adapter and index suites**

Run: `cargo test -p unica-coder infrastructure::internal_adapters infrastructure::workspace_index`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/unica-coder/src/infrastructure/internal_adapters.rs crates/unica-coder/src/infrastructure/workspace_index.rs
git commit -m "fix: share source root across analyzer and rlm"
```

### Task 5: Remove the invalid bsl-analyzer text-search backend

**Files:**
- Modify: `crates/unica-coder/src/infrastructure/internal_adapters.rs`
- Modify: `scripts/ci/check-tool-contracts.py`
- Modify: `tests/ci/test_product_contracts.py`
- Test: the same files.

**Interfaces:**
- Consumes: existing RLM and git-grep search backends.
- Produces: `CodeSearchAdapter` with only `rlm` and `git grep` sections.

- [ ] **Step 1: Replace regression expectations before production code**

Change the Rust test to assert no analyzer process is invoked and output ordering is RLM then git grep:

```rust
assert!(bsl.commands.borrow().is_empty());
assert!(!stdout.contains("=== bsl-analyzer ==="));
assert!(stdout.find("=== rlm ===").unwrap() < stdout.find("=== git grep ===").unwrap());
```

Change Python contract fixtures so they assert commands actually used by Unica (`analyze --help`, `mcp serve --help`, `smoke --help`) and no longer describe `search` as a generic text-search contract.

- [ ] **Step 2: Run tests and verify RED**

Run: `cargo test -p unica-coder code_search_adapter -- --nocapture`

Run: `python -m unittest discover -s tests/ci -p 'test_product_contracts.py'`

Expected: Rust FAIL because `bsl_search` still runs; Python FAIL until the contract list is updated consistently.

- [ ] **Step 3: Remove the invalid backend**

Delete `bsl_runner`, `bsl_search`, its constructor plumbing, and analyzer sections from `CodeSearchAdapter`. Build the result from:

```rust
let sections = [
    self.rlm_search(context, args),
    self.git_grep_search(tool_name, args, context),
];
```

Remove the obsolete dry-run assertion that constructs `bsl-analyzer search --query`. Keep provenance/version checks in `test_skill_provenance.py` unchanged.

- [ ] **Step 4: Run search and contract tests**

Run: `cargo test -p unica-coder code_search -- --nocapture`

Run: `python -m unittest discover -s tests/ci -p 'test_product_contracts.py'`

Run: `python -m unittest discover -s tests/ci -p 'test_skill_provenance.py'`

Expected: PASS.

- [ ] **Step 5: Run the plan-level regression gate**

Run: `cargo fmt --all -- --check`

Run: `cargo clippy -p unica-coder --all-targets -- -D warnings`

Run: `cargo test -p unica-coder`

Run: `python -m unittest discover -s tests/ci`

Run: `git diff --check`

Expected: all commands PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/unica-coder/src/infrastructure/internal_adapters.rs scripts/ci/check-tool-contracts.py tests/ci/test_product_contracts.py
git commit -m "fix: remove unsupported analyzer search command"
```
