# Typed reader completion implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: use `superpowers:executing-plans`
> inline. Steps use checkbox syntax so the red-green evidence remains reviewable.

**Goal:** Deliver #291 and the still-current `code.definition` part of #292 in
PR #425, together with ADR-0045 and regression tests, on top of #297 already
merged through PR #428 and ADR-0044.

**Architecture:** ADR-0044 supplies the closed execution category, two-value
result contract, reader `dryRun` rejection and missing-data postcondition. This
change adds the no-stdout postcondition. Diagnostics `mode=analyze` uses a
bounded line-streaming process path and a closed JSONL state machine. RLM
preserves pre- and post-execution readiness as structured state until one public
mapper constructs the definition outcome.

**Tech Stack:** Rust 2021, serde/serde_json, the existing `ManagedChild` process
lifecycle, Python 3.12 architecture guards and GitHub Actions.

## Global constraints

- The only public MCP server remains `unica`; no tool is added or renamed.
- ADR-0023 and ADR-0044 remain immutable; ADR-0045 becomes the accepted owner
  of the no-stdout enforcement and diagnostics/RLM subject protocols.
- Every production change follows a test that was observed failing for the
  intended reason.
- `unica.meta.profile` is not restored and `unica.meta.info` does not acquire an
  RLM dependency.
- Cancellation, timeout and non-zero process exit take precedence over JSONL
  protocol classification.
- Diagnostics raw stdout is never published or accumulated without a bound.

---

### Task 1: Executable tool contracts

**Files:**
- Modify: `crates/unica-coder/src/application/mod.rs`
- Modify: `crates/unica-coder/src/application/tool_contracts.rs`
- Test: unit tests in the same modules

**Interfaces:**
- Consumes: `ToolExecution::{Read, Mutation}` and
  `ResultContract::{Typed, ExternalStream}` from ADR-0044.
- Produces: a finalizer extension that returns `typed_result_textual:` for an
  otherwise successful `Read + Typed` outcome with a stdout duplicate, while
  preserving the ADR-0044 priority of `typed_result_missing:`.

- [x] Reuse the ADR-0044 table test proving every registered tool maps to its
  executable category and typed/external-stream result contract.
- [x] Run the table test and observe failure because the executable categories
  do not exist.
- [x] Add schema/validation tests proving every reader omits and rejects
  `dryRun`, while mutation schemas retain it.
- [x] Run the schema/validation tests and observe the current common argument
  admits `dryRun` for readers.
- [x] Add application finalizer tests with literal handler outcomes for missing
  `data`, textual duplicate, both violations, failed typed reads and non-typed
  reads.
- [x] Run them and observe that the current application returns empty or textual
  success.
- [x] Implement the closed categories, reader argument boundary and single
  postcondition; make native readers execute through the normal read path.
- [x] Run the focused tests and keep every existing mutation preview test green.

### Task 2: Typed diagnostics JSONL protocol

**Files:**
- Create: `crates/unica-coder/src/infrastructure/diagnostics_jsonl.rs`
- Modify: `crates/unica-coder/src/infrastructure/mod.rs`
- Modify: `crates/unica-coder/src/infrastructure/platform/process.rs`
- Modify: `crates/unica-coder/src/infrastructure/internal_adapters.rs`
- Modify: `crates/unica-coder/src/application/tool_contracts.rs`
- Test: unit tests in `diagnostics_jsonl.rs`, `process.rs`,
  `internal_adapters.rs` and `tool_contracts.rs`

**Interfaces:**
- Produces: a closed parser accepting only upstream `start`, `file`, `done`
  events from pinned bsl-analyzer `9a6cb15`.
- Produces: a `ManagedChild` line-drain API with an 8 MiB physical-line bound,
  bounded stderr and no raw stdout result.
- Produces: stable `OperationResult.data` with counters, filtering, ordering,
  limit and completion state from ADR-0045.

- [x] Add command contract tests proving default and explicit analyze always
  report `--format jsonl`; `json`/`jsonl` are migration aliases and all other
  formats or modes reject `format`.
- [x] Run them and observe default analyze still permits console output.
- [x] Add parser fixtures for clean, findings, file failure and zero files, with
  hand-derived typed payloads.
- [x] Run them and observe failure because no parser exists.
- [x] Add fail-closed fixtures for order, duplicate events/paths, unknown fields,
  invalid scalars/ranges/tags/paths and inconsistent totals.
- [x] Add filtering/sorting/limit tests proving file failures bypass filters and
  output is independent of upstream discovery order.
- [x] Implement the parser and projection without retaining raw JSONL lines.
- [x] Add a process test that streams more than 1 MiB successfully and rejects a
  physical line over 8 MiB while continuing to drain the child pipes.
- [x] Implement the general line drain on the existing `ManagedChild` lifecycle.
- [x] Route analyze through the streaming runner and verify cancellation,
  timeout, non-zero exit, stderr redaction and protocol priorities.

### Task 3: RLM definition readiness

**Files:**
- Modify: `crates/unica-coder/src/infrastructure/workspace_services.rs`
- Modify: `crates/unica-coder/src/infrastructure/rlm_navigation.rs`
- Modify: `crates/unica-coder/src/infrastructure/code_intelligence.rs`
- Test: unit tests in the same modules

**Interfaces:**
- Produces: a workspace RLM call result that distinguishes helper output from
  `IndexReadiness` at the execution boundary.
- Produces: one definition mapper with `index_pending:` only for `Building` and
  `index_unavailable:` for `Missing`, `Stale`, `Failed`, `Unavailable`.

- [x] Replace the warning-only regression test with a literal readiness matrix
  requiring `ok=false`, no `data`/stdout and the stable error prefixes.
- [x] Run it and observe `Missing` currently returns `ok=true` plus a warning.
- [x] Add a post-execution stale test proving helper output is discarded and the
  same readiness mapper is used.
- [x] Run it and observe the workspace service currently collapses readiness to
  a string error.
- [x] Preserve structured readiness across the service response and map it once
  in `RlmNavigationAdapter`.
- [x] Verify ready `definitions=[]` remains a successful typed result and
  cancellation remains a distinct outcome.

### Task 4: Architecture and publication

**Files:**
- Modify: `spec/decisions/0045-typed-reader-completion-contract.md`
- Modify: `spec/decisions/README.md`
- Modify: `spec/architecture/invariants.md`
- Modify: `docs/design/2026-08-10-typed-reader-completion-design.md`
- Modify: `plugins/unica/skills/code-diagnostics/SKILL.md`
- Modify: PR #425 title/body

**Interfaces:**
- Consumes: all executable contracts and passing regression tests from Tasks
  1-3.
- Produces: one mergeable implementation PR based on merged #297 that fixes
  #291 and the current `code.definition` part of #292.

- [x] Change ADR-0045 to `accepted` and make it own the diagnostics/RLM
  implementation plus no-stdout guard without duplicating ADR-0044.
- [x] Change the design delivery boundary to the final live ordering: merged
  #297 as the base and one PR for #291/#292.
- [x] Add the new executable check to `INV-MCP-TYPED-RESULT` without rewriting
  ADR-0023.
- [x] Update the diagnostics skill with typed completion/error semantics.
- [x] Run focused Rust tests after every red-green slice.
- [x] Run `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`,
  `cargo test --workspace -- --test-threads=1`, full Python CI/dev suites,
  architecture/platform guards and `git diff --check`.
- [ ] Commit, push the existing branch, update PR #425, mark it ready for review
  and wait for required GitHub checks.
