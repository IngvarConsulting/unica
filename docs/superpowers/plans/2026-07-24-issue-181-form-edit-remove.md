# `unica.form.edit` Element Removal Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Resolve issue #181 by making `removeElements` a strict, atomic, reference-safe part of the public `unica.form.edit` DSL.

**Architecture:** One shared domain contract defines the nested `form.edit` schema and runtime validation. The native editor resolves exact primary element names in the editable tree (never `BaseForm`), plans whole XML-subtree removals (therefore including structural companions), rejects conflicts and external XML references, validates the projected XML before publication, and returns typed removal data through the existing native result path. `CompileTransaction` publication and post-write validation remain the only apply path and provide the final TOCTOU guard.

**Tech Stack:** Rust, `serde_json`, `roxmltree`, existing `CompileTransaction`, managed-form validator, MCP contract tests.

---

### Task 1: Make the nested edit DSL strict

**Files:**
- Create: `crates/unica-coder/src/domain/form_edit.rs`
- Modify: `crates/unica-coder/src/domain/mod.rs`
- Modify: `crates/unica-coder/src/application/tool_contracts.rs`
- Test: `crates/unica-coder/src/application/tool_contracts.rs`

- [x] **Step 1: Write failing contract tests** requiring `definition.additionalProperties: false`, a typed `removeElements` array, and rejection of unknown sections and malformed removal entries.

- [x] **Step 2: Run RED.**

Run: `cargo test -p unica-coder form_edit_contract -- --nocapture`

Expected: the nested schema is still open and runtime validation accepts unknown sections.

- [x] **Step 3: Implement one shared definition contract** used by schema generation and inline/JSON-file runtime validation. Support the existing sections plus `removeElements: [{"name":"..."}]`; reject unknown sections, unknown removal fields, empty names, and duplicate removal requests with stable `FORM_EDIT_*` codes.

- [x] **Step 4: Run GREEN** with the command from Step 2.

### Task 2: Plan exact subtree removals

**Files:**
- Modify/Test: `crates/unica-coder/src/infrastructure/native_operations/form.rs`

- [x] **Step 1: Write failing preview tests** for exact element removal, contained companion reporting, same-prefix unrelated element preservation, missing/ambiguous/protected targets, and overlapping requests.

- [x] **Step 2: Run RED.**

Run: `cargo test -p unica-coder form_edit_remove -- --nocapture`

Expected: `removeElements` is currently ignored and preview is reported as an idempotent no-op.

- [x] **Step 3: Implement an XML-aware removal planner.** Only nodes directly owned by a `ChildItems` container in the editable tree are removable public elements. Select exact names, capture the complete subtree and its indentation/newline range, report contained form elements/companions, reject duplicate/overlapping ranges, and apply ranges in descending byte order.

- [x] **Step 4: Run GREEN** with the command from Step 2.

### Task 3: Reject dangling references and publish atomically

**Files:**
- Modify/Test: `crates/unica-coder/src/infrastructure/native_operations/form.rs`

- [x] **Step 1: Write failing tests** for add/remove and `into`/`after` conflicts, element events targeting removed nodes, `Items.<name>` bindings, `Form.Item.<name>.StandardCommand.*`, and `AdditionSource/Item`. Assert dry-run immutability and apply rollback/no-write on every failure.

- [x] **Step 2: Run RED.**

Run: `cargo test -p unica-coder form_edit_remove_rejects -- --nocapture`

Expected: dangling-reference cases are not recognized before mutation planning.

- [x] **Step 3: Validate the projected edit before publication.** Reject conflicting new-definition references, scan surviving editable XML nodes for supported element references, and run the full managed-form validator against the in-memory projection for preview/apply/no-op. Keep apply publication inside `CompileTransaction`; retain its managed-form post-validation as the final guard.

- [x] **Step 4: Run GREEN** with the command from Step 2.

### Task 4: Return typed removal data

**Files:**
- Modify: `crates/unica-coder/src/infrastructure/native_operations/form.rs`
- Modify: `crates/unica-coder/src/infrastructure/native_operations/registry.rs`
- Modify: `crates/unica-coder/src/infrastructure/native_operations/typed_result.rs`
- Test: `crates/unica-coder/src/application/mod.rs`

- [x] **Step 1: Write a failing public-boundary test** asserting `data.changed`, ordered `data.removed[]` entries (`name`, `kind`, `reason`), and `data.validation`.

- [x] **Step 2: Run RED.**

Run: `cargo test -p unica-coder form_edit_remove_returns_typed_data -- --nocapture`

Expected: `form-edit` currently returns no typed `data`.

- [x] **Step 3: Add a `FormEdit` typed mutation handler** and serialize the edit execution data through the existing native operation result path. Keep legacy human-readable `stdout`, but do not encode the machine contract there.

- [x] **Step 4: Run GREEN** with the command from Step 2.

### Task 5: Document and verify issue #181

**Files:**
- Modify: `plugins/unica/skills/form-edit/SKILL.md`
- Test: `crates/unica-coder/src/infrastructure/native_operations/form.rs`
- Test: `crates/unica-coder/src/application/mod.rs`

- [x] **Step 1: Add acceptance regressions** for BOM/CRLF preservation, multi-element atomic removal, idempotent non-removal behavior, cache invalidation on apply, no cache event on preview, successful `form.validate` after apply, nested table columns, referenced companions, extension `BaseForm`, dotted bindings, mixed rollback, repeated removal, and public `JsonPath`.

- [x] **Step 2: Update the MCP-first skill** with the exact `removeElements` contract, default not-found error, structural companion semantics, reference checks, preview/apply result, projected validation, and scope exclusions.

- [x] **Step 3: Run focused verification.**

```bash
cargo test -p unica-coder form_edit -- --nocapture
python3 -m unittest tests.ci.test_skill_provenance
```

Expected: both commands exit 0.

- [x] **Step 4: Run full verification.**

```bash
cargo fmt --all -- --check
cargo clippy -p unica-coder --all-targets -- -D warnings
cargo test -p unica-coder
python3 scripts/ci/check-rust-platform-boundary.py
git diff --check origin/main...HEAD
```

Expected: every command exits 0.
