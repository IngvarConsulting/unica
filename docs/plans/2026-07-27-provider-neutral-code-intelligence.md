# Provider-Neutral Code Intelligence Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use
> `superpowers:executing-plans` to implement this plan task-by-task. Every
> production change follows a red-green-refactor cycle.

**Goal:** Complete issue #211 and PR #231 so `unica.code.search` resolves one
source context, queries RLM, bsl-analyzer, and git-grep concurrently, and
returns a deterministic typed result without exposing provider-specific public
tools or reading the RLM SQLite schema.

**Architecture:** The application resolves a `CodeIntelligenceContext` once and
hands it to a fixed provider registry. A deadline-aware coordinator starts all
providers concurrently, normalizes provider-local failures into typed sections,
and restores the fixed `rlm`, `bsl-analyzer`, `git-grep` output order. Provider
implementations live in infrastructure: git-grep is a bounded fixed-string
process, bsl-analyzer uses its existing persistent MCP workspace service, and
RLM uses a new persistent MCP session for `rlm_start`/`rlm_execute`/`rlm_end`.
The existing `rlm-bsl-index info/build/update` lifecycle remains unchanged.

**Tech Stack:** Rust 2021, serde/serde_json, existing platform child-process and
workspace-service abstractions, Python package-contract tests, upstream
`rlm-tools-bsl` v1.29.1.

## Global Constraints

- Keep one public MCP server named `unica`.
- Keep exactly three built-in search providers in the public result order:
  `rlm`, `bsl-analyzer`, `git-grep`.
- Resolve `sourceDir` once; providers must not rediscover or reinterpret it.
- `unica.code.search` has no public provider selector.
- Remove public `unica.code.grep`; keep git-grep only as the internal fallback.
- Keep result ordering provider-local. Do not cross-sort or deduplicate hits.
- Return partial success when at least one provider completes with `ok` or
  `empty`; fail the operation only when all providers fail/unavailable.
- Preserve cancellation as cancellation, not as ordinary provider failure.
- Public deadline is 120 seconds; git-grep gets at most 15 seconds, RLM execute
  at most 45 seconds, and bsl-analyzer gets the remaining public budget capped
  at 120 seconds.
- Production code must not import `rusqlite` or know RLM table names.
- RLM source is unmodified upstream v1.29.1 at
  `8bc6e9fc83b522f9a79eab3193eb13fc2cecb8ed`.
- Do not point `tools.lock.json` at unpublished bytes. The final coordinates
  use the fetched and checksum-verified
  `rlm-tools-bsl-v1.29.1-build.2` release. Build.1 remains immutable but is not
  consumed because packaged MCP execution exposed a generic PyInstaller
  `multiprocessing.freeze_support()` omission; the corrected generic packager
  produced build.2 without changing the pinned upstream RLM source.

## File Structure

- Modify `crates/unica-coder/src/domain/code_intelligence.rs`
  - Add the resolved provider context and serializable canonical result.
  - Make provider execution deadline-aware.
- Add `crates/unica-coder/src/application/code_intelligence.rs`
  - Implement concurrent orchestration, status policy, deterministic ordering,
    text rendering, and typed JSON conversion.
- Modify `crates/unica-coder/src/application/mod.rs`
  - Register unified search and dispatch RLM-backed reads through the
    provider-neutral registry.
- Modify `crates/unica-coder/src/application/ports.rs`
  - Add the provider-neutral search port.
- Modify `crates/unica-coder/src/infrastructure/application_ports.rs`
  - Resolve `sourceDir` once and compose the built-in providers.
- Add `crates/unica-coder/src/infrastructure/code_intelligence.rs`
  - Implement RLM, bsl-analyzer, and git-grep providers and parsers.
- Modify `crates/unica-coder/src/infrastructure/workspace_services.rs`
  - Add persistent RLM MCP session and typed search request.
- Modify `crates/unica-coder/src/infrastructure/internal_adapters.rs`
  - Remove the old sequential search path and every direct SQLite query.
- Modify Cargo manifests
  - Remove `rusqlite`.
- Modify `plugins/unica/third-party/tools.lock.json` and provenance fixtures
  - Pin and verify RLM v1.29.1 after its binary release exists.
- Modify code-search and dependent skills
  - Remove `unica.code.grep` guidance and describe the unified result.
- Modify Rust/Python acceptance tests and ADR verification evidence.

---

### Task 1: Lock the Provider-Neutral Domain Contract

**Files:**
- Modify: `crates/unica-coder/src/domain/code_intelligence.rs`
- Modify: `crates/unica-coder/src/domain/mod.rs`

- [x] Add failing tests proving every provider receives the same normalized
  workspace root, resolved source root, source-set identity, workspace epoch,
  and absolute deadline.
- [x] Add failing serde-shape tests for `provider`, `status`, `hits`,
  `diagnostics`, and `artifacts`, including provider-local ranks starting at 1.
- [x] Run the focused domain tests and record the expected compile/assertion
  failures.
- [x] Introduce `CodeIntelligenceContext`, `SearchDeadline`, and
  `CodeSearchResult`; derive serialization only on reader-facing DTOs.
- [x] Change `CodeIntelligenceProvider::search` to accept the resolved context
  and provider deadline.
- [x] Re-run focused tests green and keep duplicate-provider validation.

### Task 2: Implement the Concurrent Application Coordinator

**Files:**
- Add: `crates/unica-coder/src/application/code_intelligence.rs`
- Modify: `crates/unica-coder/src/application/mod.rs`
- Modify: `crates/unica-coder/src/application/ports.rs`

- [x] Add failing fake-provider tests for simultaneous start, fixed output
  order despite reversed completion, partial success, all-failed behavior,
  deadline exhaustion, panic isolation, and cancellation propagation.
- [x] Add failing rendering tests that require readable provider headings while
  treating typed JSON as canonical.
- [x] Implement an owned-worker coordinator that starts all providers before
  waiting, catches provider panics, enforces provider budgets against one public
  deadline even when a provider ignores cancellation, bounds retained workers
  per provider, tracks worker handles through the same aggregate MCP EOF
  shutdown grace as active tool calls, links worker cancellation to the MCP
  request, and restores registry order.
- [x] Implement the success policy and canonical `OperationResult.data`
  conversion.
- [x] Re-run application tests green, including MCP cancellation tests.

### Task 3: Add the Internal Git-Grep Provider and Remove Its Public Tool

**Files:**
- Add/Modify: `crates/unica-coder/src/infrastructure/code_intelligence.rs`
- Modify: `crates/unica-coder/src/application/mod.rs`
- Modify: `crates/unica-coder/src/application/tool_contracts.rs`
- Modify: `crates/unica-coder/src/infrastructure/internal_adapters.rs`
- Modify: `tests/ci/test_release_assessment.py`
- Modify: `tests/ci/test_unica_skills.py`

- [x] Add failing provider tests for literal-only `git grep -F -n`, source-root
  scoping, absolute 15-second deadline, stable path/line/snippet parsing,
  empty/unavailable/failed distinction, local ranking, and cancellation.
- [x] Add failing public-contract tests asserting `unica.code.grep` is absent
  and its exclusive arguments (`regex`, `ignoreCase`, `fileTypes`,
  `excludePath`) no longer appear in tool schemas.
- [x] Implement `GitGrepProvider` through the existing cancellable runner.
- [x] Delete public registration/routing/schema prose for
  `unica.code.grep`; retain no hidden public alias.
- [x] Re-run focused Rust and Python tests green.

### Task 4: Add the bsl-analyzer Search Provider

**Files:**
- Modify: `crates/unica-coder/src/infrastructure/code_intelligence.rs`
- Modify: `crates/unica-coder/src/infrastructure/workspace_services.rs`

- [x] Add failing parser fixtures for actual `search_code` ranked text,
  including Windows paths, optional line ranges, score extraction, malformed
  entries, and empty results.
- [x] Add failing service tests proving `action=search_code`, resolved
  `sourceDir`, query, limit, cancellation, and remaining-budget deadline reach
  the persistent bsl-analyzer MCP session.
- [x] Implement `BslAnalyzerProvider` using the existing workspace-service MCP
  session; do not spawn a process per query.
- [x] Normalize parse errors into provider diagnostics without discarding
  well-formed hits.
- [x] Re-run provider and workspace-service tests green.

### Task 5: Add Persistent RLM MCP Search and Remove SQLite Knowledge

**Files:**
- Modify: `crates/unica-coder/src/infrastructure/workspace_services.rs`
- Modify: `crates/unica-coder/src/infrastructure/code_intelligence.rs`
- Modify: `crates/unica-coder/src/infrastructure/internal_adapters.rs`
- Modify: `Cargo.toml`
- Modify: `crates/unica-coder/Cargo.toml`

- [x] Add failing fake-MCP transcript tests for exactly one `rlm_start` per
  workspace/source-root session, repeated `rlm_execute`, `rlm_end` on
  invalidation/drop, and one restart/retry after session expiry.
- [x] Add failing RLM provider tests for 45-second execution budget,
  readiness/building/unavailable mapping, result parsing, cancellation, and
  workspace/source-root isolation.
- [x] Generalize the persistent JSON-RPC session primitive enough to serve both
  bsl-analyzer and RLM while retaining tool-specific initialize arguments.
- [x] Implement `RlmProvider` over `rlm_start`, `rlm_execute`, and `rlm_end`;
  keep index lifecycle on `rlm-bsl-index info/build/update`.
- [x] Delete `search_rlm_index` and other direct table queries; migrate
  definition/outline/meta-profile callers to provider/tool APIs or existing
  non-SQL fallbacks without changing their public result contracts.
- [x] Remove `rusqlite` from both Cargo manifests and regenerate `Cargo.lock`.
- [x] Prove with `rg` and focused tests that production contains neither
  `rusqlite` nor RLM table names.

### Task 6: Wire the Unified Search End to End

**Files:**
- Modify: `crates/unica-coder/src/infrastructure/application_ports.rs`
- Modify: `crates/unica-coder/src/application/ports.rs`
- Modify: `crates/unica-coder/src/application/mod.rs`
- Modify: `crates/unica-coder/src/interfaces/mcp.rs`
- Modify: `crates/unica-coder/tests/platform/issue_89_workspace_service.rs`

- [x] Add failing application-port tests proving one source-root resolution,
  exact three-provider registration, canonical data, partial success,
  all-failed failure, and cancellation.
- [x] Route `unica.code.search` through the application coordinator and return
  `HandlerOutcome::with_data`.
- [x] Delete the old sequential `CodeSearchAdapter` composition path.
- [x] Update MCP integration assertions for typed data and stable readable
  text.
- [x] Run all `unica.code.search` Rust tests green.

### Task 7: Update RLM Packaging and User-Facing Guidance

**Files:**
- Modify: `plugins/unica/third-party/tools.lock.json`
- Modify: `tests/ci/test_skill_provenance.py`
- Modify: `plugins/unica/ATTRIBUTIONS.md`
- Modify: `plugins/unica/skills/code-search/SKILL.md`
- Modify: dependent skill files returned by `rg 'unica\.code\.grep'`
- Modify: `spec/decisions/0017-provider-neutral-code-intelligence.md`

- [x] Build unmodified RLM v1.29.1 through `unica-toolchain`, publish the final
  immutable build.2 release tag, and verify every platform asset plus SHA-256.
- [x] Add failing provenance/package tests for v1.29.1 source commit, release
  tag, platform assets, checksums, and expected MCP tool interface.
- [x] Update `tools.lock.json` only after the published bytes are fetchable.
- [x] Rewrite skill guidance to use unified search for literal and semantic
  cases; remove all public `unica.code.grep` examples.
- [x] Update ADR verification evidence from planned to implemented, with exact
  test and package-contract references.
- [x] Run all Python CI tests green.

### Task 8: Full Verification and PR Delivery

- [x] Run `cargo fmt --all -- --check`.
- [x] Run `cargo clippy --workspace --all-targets --all-features -- -D warnings`.
- [x] Run `cargo test --workspace`.
- [x] Run `python -m unittest discover -s tests -p 'test_*.py'` in the verified
  lxml-enabled environment.
- [x] Run package-contract, thin-bootstrap, and provenance checks documented by
  the repository CI.
- [x] Inspect `git diff --check`, the full changed-file diff, and public tool
  inventory; verify no unrelated user changes were touched.
- [x] Commit coherent implementation units on the existing PR head branch.
- [x] Push `HEAD` to
  `pr250fork:refs/heads/refactor/211-code-intelligence-providers`.
- [x] Wait for all PR checks, inspect any failures by root cause, fix/retest,
  and repeat until green.
