# Logical diagnostics review remediation

> **For Codex:** execute this plan task-by-task with RED/GREEN evidence and re-run the public-surface checks before publishing PR #475.

**Goal:** Finish the approved provider-neutral diagnostics contract after merging current `main`, and resolve every reproducible correctness issue from the independent review.

**Architecture:** Keep provider handles private, route execution from the composition registry, publish the shared `SourceLocation` plus diagnostic-only `locationReason`, and derive all operational inputs from one validated snapshot. Preserve provider sections as result provenance; remove only the public provider selector.

**Tech Stack:** Rust, serde/serde_json, Python unittest, repository architecture guards.

---

### Task 1: Align the public request and location DTO

**Files:** `domain/diagnostics.rs`, `application/tool_contracts.rs`, `application/operation_descriptors.rs`, `application/diagnostics.rs`, `infrastructure/diagnostics.rs` and their tests.

1. Add RED schema/serialization tests proving `providers` is rejected and diagnostics serialize the shared `SourceLocation` with `locationReason` only for unaddressable items.
2. Remove `DiagnosticLocation` and `requested_providers`; reuse `domain::source_location::SourceLocation`.
3. Route all applicable registered providers internally and keep `filter.codes` as a post-execution result filter.
4. Run the focused Rust tests.

### Task 2: Fix reproducible coordinator and adapter defects

**Files:** `application/diagnostics.rs`, `infrastructure/diagnostics.rs`, `domain/metadata/properties.rs` and their tests.

1. Add RED tests for catalog code filtering, `findings=[] + truncated=true`, stable provider error codes, and rejection of the nonexistent metadata property `Required`.
2. Make timeout ownership single-source and prove `timeoutSeconds` reaches the provider deadline end to end.
3. Cache resource-handle mapping per diagnostic call and cache sort keys per item.
4. Keep safe allowlisted public errors; do not re-expose provider path text.

### Task 3: Close public path leaks

**Files:** `application/mod.rs`, `application/diagnostics.rs`, `infrastructure/diagnostics_jsonl.rs` and MCP/application tests.

1. Add serialized RED regressions for `cache.root`, out-of-scope handles, and provider errors containing absolute paths.
2. Redact diagnostics cache transport and centralize safe public diagnostic errors.
3. Run focused boundary and serialization tests.

### Task 4: Repair release probes and user documentation

**Files:** `scripts/ci/release-assessment.py`, `tests/ci/test_release_assessment.py`, diagnostics skill, migration guide, generated surface sources.

1. Add a RED release-assessment test where the Platform XML source set is not named `main`.
2. Derive its name from `project_source_sets()` and `SOURCE_DIR`.
3. Restore Russian trigger wording and suppression keywords; document non-module migration and catalog severity limits.
4. Regenerate/check the public surface.

### Task 5: Verify and publish

1. Run `cargo fmt --check`, focused diagnostics tests, `cargo test -p unica-coder --lib`, relevant Python suites, and `check-architecture-sync.py --base origin/main`.
2. Inspect `git diff --check` and PR diff for unrelated changes.
3. Commit, push the existing PR head branch, and report accepted/rejected review findings with test evidence.
