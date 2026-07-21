# Safe `unica.code.patch` v1 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Deliver a safe, honest `Refs #73` MCP tool that inserts into existing BSL modules without weakening Unica's publication contract.

**Architecture:** A native `code-patch` operation parses bytes into a small BSL structural index, resolves one typed selector, creates and validates a byte-exact postimage, then publishes only through `single_file_publisher`. The contract and runtime use one selector model; public MCP tests exercise the end-to-end result.

**Tech Stack:** Rust, serde_json, SHA-256, existing workspace/source/support guards, `single_file_publisher`.

---

### Task 1: Register the public contract

**Files:**
- Modify: `crates/unica-coder/src/application/operation_descriptors.rs`
- Modify: `crates/unica-coder/src/application/tool_contracts.rs`
- Modify: `crates/unica-coder/src/application/mod.rs`

- [ ] **Step 1: Write failing schema tests** for `{"selector":{"method":"Run"}}` and `{"selector":{"anchor":"Write()"}}`; also reject empty, both, and unknown selector members.

- [ ] **Step 2: Run RED.**

Run: `cargo test -p unica-coder code_patch_schema -- --nocapture`

Expected: the examples fail schema validation or the tool is unknown.

- [ ] **Step 3: Register `unica.code.patch` and implement one shared selector schema.**

The schema declares both `method` and `anchor` in `properties`, has `additionalProperties: false`, and uses `oneOf` to require exactly one. The allowlist includes `path`, `operation`, `selector`, `content`, `position`, `sourceDir`, and `supportPolicy`.

- [ ] **Step 4: Run GREEN** with the command from Step 2; commit as `feat: register typed code patch contract`.

### Task 2: Index BSL safely

**Files:**
- Create: `crates/unica-coder/src/infrastructure/native_operations/code.rs`

- [ ] **Step 1: Write failing parser tests** for BOM-prefixed Russian and English methods, and for declaration words in comments and strings.

- [ ] **Step 2: Run RED.**

Run: `cargo test -p unica-coder native_operations::code::tests::index -- --nocapture`

Expected: the index does not yet exist.

- [ ] **Step 3: Implement `BslModuleIndex`, `MethodRange`, and typed `PatchSelector`.**

Use a byte scanner with string/comment state. Recognize Russian and English open/close declarations only as line-leading tokens; retain borrowed byte ranges. Return typed errors for malformed UTF-8 or impossible structural state.

- [ ] **Step 4: Run GREEN** with the command from Step 2; commit as `feat: index BSL method boundaries for code patch`.

### Task 3: Plan a byte-exact mutation

**Files:**
- Modify: `crates/unica-coder/src/infrastructure/native_operations/code.rs`
- Modify: `crates/unica-coder/src/infrastructure/native_operations/registry.rs`
- Modify: `crates/unica-coder/src/infrastructure/native_operations.rs`

- [ ] **Step 1: Write failing integration tests** for existing `Module.bsl`, `ObjectModule.bsl`, and `ManagerModule.bsl`; assert unique/missing/ambiguous selectors, BOM/CRLF/LF/mixed-EOL preservation, byte and line/column ranges, hashes, and real unified diff.

- [ ] **Step 2: Run RED.**

Run: `cargo test -p unica-coder native_operations::code::tests::patch -- --nocapture`

Expected: target, result, and diff assertions fail.

- [ ] **Step 3: Implement target selection and `PatchPlan`.**

Resolve the selected platform-XML Configuration source root through existing source and support policies. Require an existing regular `*Module.bsl`. Preserve every untouched byte and select the local EOL at the insertion point. Build hashes, real unified diff, range coordinates, source set, module role, and affected-target data from the exact pre/post images.

- [ ] **Step 4: Run GREEN** with the command from Step 2; commit as `feat: plan safe BSL code patches`.

### Task 4: Validate and publish safely

**Files:**
- Modify/Test: `crates/unica-coder/src/infrastructure/native_operations/code.rs`

- [ ] **Step 1: Write failing tests** for dry run, applied mutation, repeated byte-identical no-op, invalid postimage, stale preimage, and publication failure preserving the original.

- [ ] **Step 2: Run RED.**

Run: `cargo test -p unica-coder native_operations::code::tests::apply -- --nocapture`

Expected: publication does not yet use the shared primitive.

- [ ] **Step 3: Validate postimage before any write and call:**

```rust
publish(PublishRequest {
    target: &plan.target,
    replacement: &plan.postimage,
    mode: PublishMode::ReplaceExisting {
        expected_preimage: &plan.preimage,
    },
})
```

Map the typed publisher error without discarding its source. Preview/no-op has no changed target; only `PublishEffect::Replaced` reports one changed target.

- [ ] **Step 4: Run GREEN** with the command from Step 2; commit as `feat: publish validated code patches safely`.

### Task 5: Align the public surface

**Files:**
- Modify: `plugins/unica/skills/code-patch/SKILL.md`
- Modify: `plugins/unica/provenance/skill-upstreams.json`
- Modify: `spec/architecture/code-patch-v1.md`
- Modify: `tests/ci/test_unica_mcp_script_parity.py`

- [ ] **Step 1: Write failing MCP-boundary tests** calling the public tool with each schema-valid selector and asserting typed result fields plus source selection.

- [ ] **Step 2: Run RED.**

Run: `cargo test -p unica-coder code_patch -- --nocapture && python3 -m unittest tests.ci.test_unica_mcp_script_parity`

Expected: the public boundary/doc contract is incomplete.

- [ ] **Step 3: Update material** to say `Refs #73`, accept the v1 architecture, document only existing-module insertion and its exclusions, then remove whitespace defects.

- [ ] **Step 4: Run GREEN** with the command from Step 2; commit as `docs: scope code patch as a safe v1 slice`.

### Task 6: Rebase and verify

- [ ] **Step 1: Rebase on current upstream main.**

Run: `git fetch upstream && git rebase upstream/main`

Expected: no conflicts; resolve only the planned files if upstream changed them.

- [ ] **Step 2: Run full verification.**

```bash
cargo fmt --all -- --check
cargo clippy -p unica-coder --all-targets -- -D warnings
cargo test -p unica-coder
python3 scripts/ci/check-rust-platform-boundary.py
git diff --check upstream/main...HEAD
```

Expected: every command exits 0.

