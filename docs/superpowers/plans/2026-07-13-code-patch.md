# BSL-Aware Code Patch Implementation Plan

> **Historical execution record:** code, tests, package metadata, issue #73 and
> its eventual PR are the source of truth after implementation.
>
> **For agentic workers:** use the installed Superpowers execution workflow and
> keep this checklist current while implementing issue #73.

**Goal:** Add a typed public `unica.code.patch` tool that can safely preview and
atomically patch an existing BSL module, including the empty stubs produced by
`unica.meta.compile`.

**Architecture:** Add one native BSL mutation family with a pure byte-level
planner and a narrow atomic committer. Resolve exactly one platform XML source
root, keep the module path relative to that root, run all selector/cardinality
checks before staging, preserve raw bytes outside the selected ranges, and emit
`ModuleChanged` only after a real apply. Persistent dirty state and safe
build/dump conflict handling are explicitly deferred to #76.

**Tech Stack:** Rust `unica-coder`, SHA-256, `fs2` cross-process locking,
same-directory atomic replacement, existing source-map/support/path policies,
Python plugin guardrails, disposable 1C Designer workspace for acceptance.

---

## Contract decisions

- Exactly one of `sourceSet` or `sourceDir` is required.
- `modulePath` is a relative `.bsl` path inside the resolved source root;
  absolute paths, `..`, non-files, symlink escape, EDT, unknown and ambiguous
  source formats are rejected in preview and apply.
- `selector` is `module`, `method`, or `anchor`:
  - `module` selects all content after the optional UTF-8 BOM and safely
    initializes a BOM-only empty stub;
  - `method` selects only the method body, preserving annotations, declaration,
    signature and the ending keyword;
  - `anchor` selects a non-empty exact anchor, optionally scoped by
    `methodName`; matches beginning inside strings or `//` comments are ignored.
- `operation` is `insertBefore`, `insertAfter`, or `replace`.
- Positive `expectedCount` is mandatory. Cardinality mismatch and partially
  applied state are failures with zero writes.
- Payload line endings are normalized to the source EOL. LF and CRLF are
  preserved; mixed-EOL source is rejected here and left to the general writer
  policy in #74.
- Reapplying an achieved insert/replace is a byte-identical no-op.
- `dryRun` executes the real resolver and planner, returns unified diff,
  pre/post raw-byte hashes and changed ranges, and writes nothing.
- `platformSyntax` is `none` or `configuredInfobase`. The latter runs only
  v8-runner syntax after a successful apply; it never runs build/load/update,
  is non-transactional, does not suppress `ModuleChanged`, and explicitly
  reports `validatesPatchedSource=false` until #76 supplies the build contract.
- The original database `/Users/korolev/Bases/Trade_11_5_Demo_Unica` is never
  used. Platform acceptance uses a new disposable workspace and file IB only.

## Task 1: Typed public boundary and RED tests

**Files:**

- Modify `crates/unica-coder/src/application/mod.rs`
- Modify `crates/unica-coder/src/application/operation_descriptors.rs`
- Modify `crates/unica-coder/src/application/tool_contracts.rs`
- Modify `crates/unica-coder/src/infrastructure/native_operations.rs`
- Modify `crates/unica-coder/src/infrastructure/native_operations/registry.rs`
- Create `crates/unica-coder/src/infrastructure/native_operations/code.rs`

- [x] Register `unica.code.patch` as a mutating native operation with
      `ModuleChanged` and BSL cache invalidation.
- [x] Add a narrow schema with `additionalProperties=false`, typed enums,
      required payload even during dry-run, and cross-field validation.
- [x] Add RED tests for source-root exclusivity, selector combinations,
      expectedCount, path traversal, `.bsl`, enum/type validation and the real
      dry-run route.
- [x] Add target-aware event tests: changed apply emits one module event;
      dry-run, rejection and exact no-op emit none.

## Task 2: Pure BSL patch planner

**File:** `crates/unica-coder/src/infrastructure/native_operations/code.rs`

- [x] Resolve platform XML source set/root and the normalized module target;
      enforce workspace/source-root/symlink boundaries for dry-run and apply.
- [x] Read raw bytes, preserve exactly one optional BOM, validate UTF-8, detect
      LF/CRLF and reject mixed EOL.
- [x] Implement a bilingual BSL scanner for Procedure/Function method bodies,
      strings with escaped quotes, comments and method-scoped anchors.
- [x] Implement `module`, `method`, and `anchor` selection plus all three patch
      operations, positive cardinality, partial-state rejection and exact
      reapply no-op detection.
- [x] Return raw SHA-256, old/new byte+line ranges, affected target and a real
      unified diff generated from the same planned bytes.
- [x] Cover BOM-only module initialization; seven statements before one scoped
      anchor; string/comment decoys; zero/two matches; all operations; no-op;
      Cyrillic offsets; BOM/no-BOM; LF/CRLF; terminal newline; mixed EOL.

## Task 3: Safe apply, support guard and concurrency

**Files:**

- Modify `crates/unica-coder/src/application/mod.rs`
- Modify `crates/unica-coder/src/application/operation_descriptors.rs`
- Modify `crates/unica-coder/src/infrastructure/path_policy.rs` if a reusable
  source-root containment helper is needed
- Modify `crates/unica-coder/src/infrastructure/native_operations/code.rs`

- [x] Add a code-patch-specific support target resolver so Common/Object/
      Manager/Form modules inherit the owning metadata support rule.
- [x] Run the support/path/source preflight during both preview and apply;
      locked vendor modules fail before staging.
- [x] Serialize writers through a cache-root `fs2` lock, re-read and compare the
      planned preimage immediately before commit, then stage in the same
      directory, flush/sync, preserve permissions and atomically replace.
- [x] Test concurrent-preimage mismatch and injected staging/replacement
      failure; original bytes remain exact and temporary files are cleaned.

## Task 4: Structured result and optional platform syntax

**Files:**

- Modify `crates/unica-coder/src/application/mod.rs`
- Modify `crates/unica-coder/src/infrastructure/internal_adapters.rs`
- Modify `crates/unica-coder/src/infrastructure/native_operations/code.rs`

- [x] Expose typed operation details alongside stdout: normalized target,
      affected module id, hashes, ranges, diff, match count and no-op/apply
      status.
- [x] Add an internal JSON-output runtime syntax path so terminal status and
      redacted platform log path are available without raw command arguments.
- [x] For `configuredInfobase`, invoke syntax only, never build/load/update;
      report scope, non-transactional semantics and
      `validatesPatchedSource=false`. Syntax failure keeps the patch and event.
- [x] Test passed/failed/timeout/unavailable/skipped-dry-run results with a fake
      runner and assert that no hidden build command is executed.

## Task 5: Skill, review and acceptance

**Files:**

- Create `plugins/unica/skills/code-patch/SKILL.md`
- Modify relevant skill/package smoke tests under `tests/ci`
- Update this checklist and `/Users/korolev/Projects/UNICA_OVERNIGHT_STATUS.md`

- [x] Document the public contract, safe workflow and the #76 syntax/build
      boundary using the skill-creator guidance.
- [x] Run focused tests, full `cargo test --locked -p unica-coder`, all-target/
      all-feature Clippy with `-D warnings`, rustfmt, full Python CI and
      `git diff --check`.
- [x] Run personal Rust review and an independent acceptance/security review;
      fix all blocking/high findings and repeat gates.
- [x] In a new disposable file IB, create/build a CommonModule stub, patch it
      only through the branch tool, rebuild, run Designer module syntax and
      verify dump/bytes. Never touch the user database.
- [ ] Commit, push, open a separate ready PR that closes #73, monitor CI, then
      claim #76 with `Depends on #73`.
