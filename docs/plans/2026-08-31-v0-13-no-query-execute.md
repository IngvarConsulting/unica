# Unica v0.13 No Query Execute Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use
> `superpowers:subagent-driven-development` (recommended) or
> `superpowers:executing-plans` to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking.

**Goal:** remove `query.execute` from the complete v0.13 public Run contract.

**Architecture:** The v0.13 Rust catalog is the runtime source of truth. Active
coverage and parity records mirror its exact twelve-operation dictionary, while
an explicit decision and invariant prevent the removed name from returning.

**Tech Stack:** Rust, serde JSON, Python `unittest`/pytest, architecture registry.

**Spec:** `docs/design/2026-08-31-v0-13-no-query-execute-design.md`

## Global Constraints

- Do not modify `plugins/unica/skills/**`.
- Preserve the eight subject and three compatibility Task tool names.
- Preserve unrelated ReceiptLedger/v5 worktree changes.
- Use targeted `rustfmt`; do not run `cargo fmt --all`.
- A direct removed-name call must return typed `unsupported_operation`.

---

### Task 1: Lock the exact v0.13 Run dictionary

**Files:**

- Modify: `crates/unica-coder/src/application/v13/tool_catalog.rs`
- Modify: `crates/unica-coder/src/infrastructure/daemon/server.rs`
- Modify: `tests/ci/test_v013_implementation_coverage.py`
- Modify: `tests/ci/test_v013_parity_inventory.py`

**Interfaces:**

- Consumes: `catalog_for(SurfaceRelease::V13)` and `RunOperation::name()`.
- Produces: an exact twelve-name Run dictionary with no query execution entry.

- [x] **Step 1: Write failing exact-set tests**

  Require length twelve, absence of `query.execute`, absence from coverage and
  parity, and production discovery omission.

- [x] **Step 2: Verify RED**

  Run focused Rust and Python selectors and confirm they fail because the
  thirteenth entry still exists.

- [x] **Step 3: Remove the production variant**

  Delete `RunIntent::QueryExecute`, its name arm and its dictionary member.
  Keep generic unknown-operation handling unchanged.

- [x] **Step 4: Verify GREEN**

  Re-run the same focused selectors and require zero failures.

### Task 2: Synchronize normative truth

**Files:**

- Create: `arch/decisions/2026-08-31-v0-13-no-query-execute.md`
- Create: `arch/invariants/INV.APP.V13-RUN-DICTIONARY.md`
- Modify: `arch/invariants/INV.APP.V13-IMPLEMENTATION-COVERAGE.md`
- Modify: `arch/tool-implementation-coverage.json`
- Modify: active v0.13 design/plan prose that currently declares the operation
- Regenerate: `arch/index.md`, `arch/tool-surface.md`

**Interfaces:**

- Consumes: the green Rust catalog exact-set test.
- Produces: current architectural ownership and machine-readable coverage.

- [x] **Step 1: Record the successor decision and invariant**

  Make the new decision the owner of implementation coverage and the exact Run
  dictionary rule; preserve the superseded decision as history.

- [x] **Step 2: Remove the coverage entry and stale active claims**

  Change thirteen to twelve wherever it describes current v0.13 behavior and
  state that the legacy query capability is removed without successor.

- [x] **Step 3: Regenerate derived files and validate the registry**

  Run `python3 scripts/arch/registry.py --write-index`, the surface generator,
  and their focused tests.

### Task 3: Verify the complete change boundary

**Files:** no additional production files.

**Interfaces:**

- Consumes: Tasks 1 and 2.
- Produces: merge/release evidence for this scoped contract change.

- [x] **Step 1: Run focused Rust and Python suites**

  Run catalog, production surface, implementation coverage, parity inventory,
  surface ledger and design-document tests.

- [x] **Step 2: Run package checks**

  Run `cargo check -p unica-coder -p unica-bootstrap` and
  `cargo test -p unica-bootstrap --quiet`.

- [x] **Step 3: Run hygiene guards**

  Run targeted `rustfmt --check`, `git diff --check`, registry validation and
  assert that `plugins/unica/skills/**` has no diff.
