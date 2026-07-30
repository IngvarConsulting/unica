# Structured BSL Outline Result Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Return `unica.code.outline` as typed JSON in `OperationResult.data`, without `stdout`, while deriving stable method and parameter fields from the current BSL syntax tree.

**Architecture:** Provider-neutral result types live in `domain::code_intelligence`; the BSL infrastructure builds them from one current syntax tree; `ProviderReadOutcome` carries typed read data to application, which serializes it into the existing `OperationResult.data` field. The MCP transport remains unchanged.

**Tech Stack:** Rust, serde/serde_json, pinned `bsl-parser` and `bsl-syntax`, existing `UnicaApplication` and platform integration-test harness.

## Global Constraints

- The source of truth remains the current BSL file selected through the normalized `CodeIntelligenceContext`.
- A successful outline has `data` and no `stdout`; the same result is never duplicated in both fields.
- Method kinds are exactly `procedure` or `function`.
- Parameters are structural objects with `name`, `byValue`, and nullable `defaultValue`.
- Unknown optional identity and region values serialize as explicit `null`.
- `includeMethods=false` empties every method array without changing totals.
- Any mandatory-field, read, containment, parse, cancellation, or deadline failure publishes neither `data` nor `stdout`.
- Outline keeps empty cache reads and writes and does not start RLM or write workspace state.
- ADR-0020 is superseded only when ADR-0021 and its executable checks become effective in the same implementation commit.

---

### Task 1: Public JSON regression tests

**Files:**
- Modify: `crates/unica-coder/tests/platform/code_intelligence_symlinked_workspace.rs`

**Interfaces:**
- Consumes: `UnicaApplication::call_tool("unica.code.outline", args) -> Result<OperationResult, String>`.
- Produces: exact observable JSON contract for Tasks 2 and 3.

- [ ] **Step 1: Replace the current text assertion with a typed-data assertion**

Use a valid mixed-case, multi-line BSL signature:

```rust
fs::write(
    &module,
    concat!(
        "#Область Public\n",
        "пРоЦеДуРа Run(\n",
        "    Знач\n",
        "    Value,\n",
        "    Optional =\n",
        "        1 + 2) Экспорт\n",
        "КонецПроцедуры\n",
        "#КонецОбласти\n",
    ),
)
.unwrap();
```

Assert `result.stdout.is_none()` and compare `result.data` with a hand-written
`serde_json::json!` literal containing `module`, nullable `identity`, `totals`,
recursive `regions`, root `methods`, canonical `kind: "procedure"`, and
parameters:

```rust
"parameters": [
    {"name": "Value", "byValue": true, "defaultValue": null},
    {"name": "Optional", "byValue": false, "defaultValue": "1 + 2"}
]
```

Retain the existing assertions that no index state or `.build/unica` directory
was created.

- [ ] **Step 2: Run the public regression and verify RED**

Run:

```sh
cargo test -p unica-coder --test platform_code_intelligence code_outline_answers_from_the_current_file_without_touching_the_index -- --exact --nocapture
```

Expected: FAIL because `result.data` is `None` and `stdout` still contains the
compact text.

- [ ] **Step 3: Add an `includeMethods=false` public regression**

Call the same module with `includeMethods: false`. Assert:

```rust
assert_eq!(result.data.as_ref().unwrap()["totals"]["methods"], 2);
assert_eq!(result.data.as_ref().unwrap()["regions"][0]["methods"], json!([]));
assert_eq!(result.data.as_ref().unwrap()["methods"], json!([]));
assert!(result.stdout.is_none());
```

- [ ] **Step 4: Run the second regression and verify RED**

Run:

```sh
cargo test -p unica-coder --test platform_code_intelligence code_outline_without_methods_keeps_totals_in_typed_data -- --exact --nocapture
```

Expected: FAIL because typed outline data is not yet published.

### Task 2: Typed outline model and AST builder

**Files:**
- Modify: `crates/unica-coder/src/domain/code_intelligence.rs`
- Modify: `crates/unica-coder/src/infrastructure/bsl_outline.rs`

**Interfaces:**
- Consumes: one normalized BSL path, `CodeIntelligenceContext`, `ProviderDeadline`, and `CancellationToken`.
- Produces: `render_current_source_outline(...) -> Result<CodeOutlineResult, String>` and `CodeIntelligenceReadData::Outline(CodeOutlineResult)`.

- [ ] **Step 1: Add provider-neutral result types**

Add serde-serializable camel-case structs:

```rust
pub struct CodeOutlineResult {
    pub module: String,
    pub identity: CodeOutlineIdentity,
    pub totals: CodeOutlineTotals,
    pub regions: Vec<CodeOutlineRegion>,
    pub methods: Vec<CodeOutlineMethod>,
}

pub struct CodeOutlineIdentity {
    pub category: Option<String>,
    pub object: Option<String>,
    pub module_type: Option<String>,
}

pub struct CodeOutlineTotals {
    pub methods: usize,
    pub exports: usize,
    pub regions: usize,
    pub loc: usize,
}

pub struct CodeOutlineRegion {
    pub name: Option<String>,
    pub line: usize,
    pub end_line: Option<usize>,
    pub regions: Vec<CodeOutlineRegion>,
    pub methods: Vec<CodeOutlineMethod>,
}

pub struct CodeOutlineMethod {
    pub name: String,
    pub kind: CodeOutlineMethodKind,
    pub parameters: Vec<CodeOutlineParameter>,
    #[serde(rename = "export")]
    pub is_export: bool,
    pub line: usize,
    pub end_line: usize,
}

pub enum CodeOutlineMethodKind {
    Procedure,
    Function,
}

pub struct CodeOutlineParameter {
    pub name: String,
    pub by_value: bool,
    pub default_value: Option<String>,
}

pub enum CodeIntelligenceReadData {
    Outline(CodeOutlineResult),
}
```

Use `#[serde(rename_all = "camelCase")]`, `#[serde(rename_all = "lowercase")]`
for `CodeOutlineMethodKind`, and `#[serde(untagged)]` for
`CodeIntelligenceReadData`.

- [ ] **Step 2: Build structural parameters**

Import `bsl_syntax::ast::Param`. Replace raw `param.text()` collection with:

```rust
fn parameters(syntax: &SyntaxNode) -> Result<Vec<CodeOutlineParameter>, String>
```

For each `Param`, require `name()`, derive `by_value` from `val_keyword()`, and
derive `default_value` from `default_value_expr()`. Render the expression from
`descendants_with_tokens()`, discard tokens for which `kind().is_trivia()`, and
join the remaining token texts with one ASCII space.

- [ ] **Step 3: Normalize method kinds by AST branch**

Pass `CodeOutlineMethodKind::Procedure` from `ProcedureDef` and
`CodeOutlineMethodKind::Function` from `FunctionDef`; remove the opening-keyword
text from the public result. Continue requiring the opening and closing tokens
for line coordinates and fail closed when either is absent.

- [ ] **Step 4: Replace text rendering with result materialization**

Replace `render(...) -> String`, `render_region`, `render_method`, and the
section heading with:

```rust
fn build_result(
    path: &str,
    identity: &ModuleIdentity,
    methods: &[CodeOutlineMethod],
    regions: &[OutlineRegion],
    include_methods: bool,
) -> CodeOutlineResult
```

Reuse `build_tree`, then recursively materialize `CodeOutlineRegion`. Put only
orphan methods in the root `methods` array. For `include_methods=false`, clone
no methods into either root or regions, while computing totals from the full
parsed method list.

- [ ] **Step 5: Run module tests and repair expectations**

Run:

```sh
cargo test -p unica-coder --lib bsl_outline -- --test-threads=1
```

Update existing text expectations to compare typed structs or
`serde_json::to_value` against hand-written literals. Preserve every existing
source-freshness, region, coordinate, containment, cancellation, and
fail-closed assertion.

Expected: all `bsl_outline` tests PASS.

### Task 3: Carry typed read data through application

**Files:**
- Modify: `crates/unica-coder/src/domain/code_intelligence.rs`
- Modify: `crates/unica-coder/src/infrastructure/code_intelligence.rs`
- Modify: `crates/unica-coder/src/infrastructure/rlm_navigation.rs`
- Modify: `crates/unica-coder/src/application/code_intelligence.rs`
- Modify: `crates/unica-coder/src/application/mod.rs`

**Interfaces:**
- Consumes: `CodeIntelligenceReadData::Outline` from Task 2.
- Produces: `OperationResult.data = Some(Value)` and `OperationResult.stdout = None` for successful outline.

- [ ] **Step 1: Extend `ProviderReadOutcome`**

Add:

```rust
pub data: Option<CodeIntelligenceReadData>,
```

Set `data: None` in RLM and test-only constructors. In
`BslAnalyzerProvider::read`, set `data` to
`Some(CodeIntelligenceReadData::Outline(result))` on success and leave
`stdout: None`. On failure leave both absent.

- [ ] **Step 2: Serialize read data at the application boundary**

In `invoke_code_intelligence_read`, convert optional typed data with
`serde_json::to_value`, returning
`HandlerOutcome::with_data(adapter, data)` when present and
`HandlerOutcome::plain(adapter)` otherwise. Do not parse provider `stdout`.

- [ ] **Step 3: Run provider and application tests**

Run:

```sh
cargo test -p unica-coder --lib code_intelligence -- --test-threads=1
cargo test -p unica-coder --lib code_outline_tool_declares_no_cache_access -- --exact
cargo test -p unica-coder --lib rlm_navigation -- --test-threads=1
```

Expected: all selected tests PASS and RLM still refuses outline before client
access.

- [ ] **Step 4: Run the public RED tests and verify GREEN**

Run:

```sh
cargo test -p unica-coder --test platform_code_intelligence -- --test-threads=1
```

Expected: both structured outline regressions PASS, including absence of
`stdout`, canonical method kinds, structural parameters, and no index state.

- [ ] **Step 5: Commit the application propagation**

```sh
git add crates/unica-coder/src/domain/code_intelligence.rs crates/unica-coder/src/infrastructure/bsl_outline.rs crates/unica-coder/src/infrastructure/code_intelligence.rs crates/unica-coder/src/infrastructure/rlm_navigation.rs crates/unica-coder/src/application/code_intelligence.rs crates/unica-coder/src/application/mod.rs crates/unica-coder/tests/platform/code_intelligence_symlinked_workspace.rs
git commit -m "fix(code): return outline as typed data"
```

### Task 4: Activate the architecture contract

**Files:**
- Modify: `spec/decisions/0020-current-source-bsl-outline.md`
- Modify: `spec/decisions/0021-structured-bsl-outline-result.md`
- Modify: `spec/architecture/invariants.md`
- Modify: `docs/design/2026-07-30-structured-bsl-outline-result-design.md`

**Interfaces:**
- Consumes: executable behavior from Tasks 2 and 3.
- Produces: active ADR-0021 and registry checks matching the delivered public contract.

- [ ] **Step 1: Transition decision lifecycle**

Change only ADR-0020's status line to:

```markdown
- Статус: `superseded` — заменено ADR-0021
```

Change ADR-0021's status from `proposed` to `accepted`. Do not rewrite the body
of accepted ADR-0020.

- [ ] **Step 2: Update registry ownership**

Change `INV-APP-OUTLINE-SOURCE` to reference ADR-0021. Add
`INV-MCP-OUTLINE-DATA` with a single rule: successful `unica.code.outline`
publishes its proven module structure only in `data`, with canonical method
kinds and structural parameters, never duplicating it in `stdout`. Name the
module and platform integration tests as executable checks.

- [ ] **Step 3: Mark the design verification as delivered**

Append a concise delivery note to the new design document naming the red-green
tests; do not turn the design document into another normative owner or create a
circular reference to the commit that contains the note.

- [ ] **Step 4: Run document and architecture checks**

Run:

```sh
python3.12 -m unittest tests.ci.test_design_documents tests.ci.test_architecture_registry tests.ci.test_architecture_sync_guard
python3.12 scripts/ci/check-architecture-sync.py --base origin/main --strict
git diff --check
```

Expected: all tests PASS and the strict guard reports contract-sync evidence.

- [ ] **Step 5: Commit the active contract**

```sh
git add spec/decisions/0020-current-source-bsl-outline.md spec/decisions/0021-structured-bsl-outline-result.md spec/architecture/invariants.md docs/design/2026-07-30-structured-bsl-outline-result-design.md
git commit -m "docs(architecture): activate structured outline contract"
```

### Task 5: Full verification and existing PR update

**Files:**
- Modify externally after verification: PR #265 description.
- No production files should change during this task.

**Interfaces:**
- Consumes: all implementation and architecture commits.
- Produces: verified branch and current PR description.

- [ ] **Step 1: Run fresh complete local verification**

Run:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace -- --test-threads=1
python3.12 -m unittest discover -s tests/ci --durations 20
python3.12 -m unittest discover -s tests/dev --durations 20
python3.12 scripts/ci/check-architecture-sync.py --base origin/main --strict
git diff --check
```

Expected: every command exits 0 with no failing tests or warnings.

- [ ] **Step 2: Inspect the final branch**

Run:

```sh
git status --short --branch
git diff --stat origin/main...HEAD
git log --oneline origin/main..HEAD
```

Expected: clean worktree, only issue #262 and structured-result changes.

- [ ] **Step 3: Push the existing PR branch**

```sh
git push origin codex/issue-262-stale-outline
```

- [ ] **Step 4: Update PR #265 description**

Replace the obsolete claim that compact `stdout` grammar is preserved. State
that outline now returns a typed `data` object without `stdout`, method kinds
are canonical, parameters are structural, ADR-0021 supersedes ADR-0020, and
include the final verification evidence.

- [ ] **Step 5: Re-read live review threads and checks**

Fetch thread-aware review state. Confirm the multi-line-parameter thread is
addressed by current code. Do not resolve or reply to the GitHub thread unless
the user explicitly authorizes that external write.
