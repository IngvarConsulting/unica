# Project Source Health Review Remediation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use
> `superpowers:executing-plans` to implement this plan task-by-task. Every
> behavior change follows RED → GREEN and no test is weakened to make it pass.

**Goal:** Bring the implemented `unica.project.status` behavior into exact
agreement with approved ADR-0060 after independent semantic review.

**Architecture:** Git inspection remains read-only and publishes one typed
snapshot. Each check starts `notRun` and becomes `completed` only after its own
prerequisites and protocol have completed; infrastructure incompleteness never
coexists with ordinary facts for the same observation. Portable repository
policy is derived from staged Git state, with explicit environment isolation,
version compatibility and bounded output sized for real Platform XML exports.

**Tech Stack:** Rust, Git plumbing protocols with NUL records, serde, existing
MCP typed result envelope, Python contract tests.

## Global Constraints

- Preserve ADR-0060 semantics: no project or Git mutation, `ready` independent
  from `repositoryReady`, and problems remain successful typed inspection data.
- Preserve one public MCP server and the existing `unica.project.status` tool.
- Support the documented generic Git prerequisite; do not silently require Git
  2.40.
- Commands in remediation serialize as `program` / `argv` / `cwd` and are never
  executed automatically.
- Use a 64 MiB project-health stdout budget (stderr remains 256 KiB): it is
  finite, covers the validated 43k-file class with headroom, and leaves the
  1 MiB default unchanged for unrelated processes.
- Update existing ADR-0060-derived design and plan text where it contradicts the
  accepted decision; do not create a replacement ADR.

---

### Task 1: Make check outcomes truthful under partial inspection

**Files:**
- Modify: `crates/unica-coder/src/infrastructure/project_health.rs`
- Modify: `crates/unica-coder/src/infrastructure/project_health/git.rs`
- Modify: `crates/unica-coder/src/infrastructure/project_health/resources.rs`
- Test: the corresponding inline test modules

- [x] Add failing tests for failed index prerequisites, failed attributes,
  ordinary EOL facts followed by incomplete EOL, and CDFI timeout/truncation.
- [x] Verify each test fails because a downstream check is incorrectly passed,
  an incompatible fact remains, or CDFI incompleteness is misclassified.
- [x] Introduce explicit prerequisite state and causal CDFI classification;
  suppress ordinary facts when a check cannot complete.
- [x] Verify focused domain and infrastructure tests pass.

### Task 2: Prove ignore rules from staged state

**Files:**
- Modify: `crates/unica-coder/src/infrastructure/project_health/git.rs`
- Test: `crates/unica-coder/src/infrastructure/project_health/git.rs`

- [x] Add failing real-Git tests for staged-empty/working-valid and
  staged-valid/working-empty `.gitignore` states.
- [x] Verify the first incorrectly passes and the second incorrectly fails.
- [x] Evaluate ignore provenance from staged `.gitignore` blobs while preserving
  nested-rule precedence, negation and local-only origin detection.
- [x] Verify both directions and existing parent/nested/local-origin tests pass.

### Task 3: Isolate and scale Git subprocess protocols

**Files:**
- Modify: `crates/unica-coder/src/infrastructure/internal_adapters.rs`
- Modify: `crates/unica-coder/src/infrastructure/platform/process.rs`
- Modify: `crates/unica-coder/src/infrastructure/project_health/git.rs`
- Modify: `crates/unica-coder/src/infrastructure/project_health/resources.rs`
- Test: the same modules

- [x] Add failing tests proving project-health Git commands remove repository,
  index and object-selection `GIT_*` variables and accept output above the
  legacy 1 MiB capture limit.
- [x] Add per-command environment removals and an explicit project-health Git
  stdout budget without raising unrelated process budgets.
- [x] Keep the complete staged index snapshot needed for parent-repository
  provenance, but enforce a project-health-only 64 MiB capture bound and prove
  it with a real 43k-sibling-path repository.
- [x] Verify contaminated-environment and large-output tests pass.

### Task 4: Close compatibility and bounded-discovery gaps

**Files:**
- Modify: `crates/unica-coder/src/infrastructure/project_sources.rs`
- Modify: `crates/unica-coder/src/infrastructure/project_health/layout.rs`
- Modify: `crates/unica-coder/src/infrastructure/project_health/resources.rs`
- Test: corresponding inline modules

- [x] Add failing tests for Git without `check-attr --source`, oversized project
  config, cancellation/deadline during discovery and unknown `ls-files --eol`
  protocol values.
- [x] Replace the Git-2.40-only local-attribute probe with a compatible staged
  policy proof or an explicit capability fallback that remains trustworthy.
- [x] Bound `v8project.yaml` reads, check cancellation/deadline between chunks
  and source sets, and expose typed incomplete source discovery.
- [x] Parse EOL kinds as a closed protocol enum.
- [x] Verify focused compatibility and layout tests pass.

### Task 5: Publish exact remediation contract

**Files:**
- Modify: `crates/unica-coder/src/domain/project_health.rs`
- Modify: `docs/design/2026-08-13-project-source-health-design.md`
- Modify: `docs/plans/2026-08-13-project-source-health.md`
- Modify: relevant plugin references and CI contract tests

- [x] Add failing serialization tests for `argv` and diagnostic-specific tests
  for `source_set.root_is_workspace`, ignore, attributes and EOL remediation.
- [x] Replace generic explanation with typed remediation variants containing
  safe, problem-specific steps; retain empty commands for ambiguous or
  destructive corrections.
- [x] Synchronize ADR-derived design, implementation plan and public examples.
- [x] Verify Rust serialization, MCP smoke and skill contract tests pass.

### Task 6: Verify the merged result

- [x] Run formatting and diff checks.
- [x] Run all `unica-coder` tests, clippy and Python CI tests.
- [x] Run MCP smoke and architecture synchronization checks.
- [x] Run merge-tree/status checks and confirm the worktree contains only the
  intended implementation and documentation changes.

### Task 7: Close findings from the independent semantic re-review

**Files:**
- Modify: `crates/unica-coder/src/infrastructure/project_health.rs`
- Modify: `crates/unica-coder/src/infrastructure/project_health/git.rs`
- Modify: `crates/unica-coder/src/infrastructure/project_health/layout.rs`
- Modify: `crates/unica-coder/src/infrastructure/project_health/resources.rs`
- Modify: `crates/unica-coder/src/infrastructure/project_sources.rs`
- Test: corresponding inline and platform modules

- [x] Reproduce false `Completed` outcomes for incomplete targets and the
  EDT-only `NotApplicable` overwrite, then keep repository checks that require
  complete source target identities `NotRun` until those identities are proven.
  Continue independent repository checks for each separately proven sibling,
  but keep source-derived Git checks `NotRun` for a rejected workspace-root
  target so `source_set.root_is_workspace` remains its single primary cause.
- [x] Replace `checkout-index` materialization with exact staged blob reads that
  cannot run smudge filters; bound file count and aggregate bytes.
- [x] Remove inherited `GIT_*` variables case-insensitively and restore
  no-fetch, no-replacement, no-lock and no-prompt protections explicitly.
- [x] Stop health discovery from retaining marker bytes and bound the number of
  declared source sets while preserving transactional provenance callers.
- [x] Reject incomplete EOL protocol values, suppress derivative EOL facts for
  `-text`, and preserve EDT-only `NotApplicable` observations.
- [x] Make source and EOL remediations evidence-specific and synchronize the
  older implementation plan where its outcome semantics contradicted code.
- [x] Repeat the full verification matrix and obtain an independent clean
  semantic re-review of the final diff.
- [x] Perform a fresh independent semantic review before declaring merge-ready.

### Task 8: Close findings from the second independent pre-merge review

**Files:**
- Modify: `crates/unica-coder/src/infrastructure/project_health.rs`
- Modify: `crates/unica-coder/src/infrastructure/project_health/git.rs`
- Modify: `crates/unica-coder/src/infrastructure/project_health/layout.rs`
- Modify: `crates/unica-coder/src/infrastructure/project_health/resources.rs`
- Modify: `crates/unica-coder/src/infrastructure/project_sources.rs`
- Test: corresponding inline and platform modules

- [x] Add a failing regression proving a resource below a nested EDT root is
  not assigned to an outer Platform XML source set, then select the deepest
  owner across all proven roots before applying the owner's format profile.
- [x] Add failing regressions for `Unknown` and `Invalid` source formats that
  currently let format-dependent ignore policy pass; keep only those dependent
  ignore observations `NotRun` until the source profile is proven.
- [x] Add a failing unknown-field YAML depth/node bomb that exceeds the health
  deadline inside monolithic serde parsing; enforce a document-wide health-only
  event depth/node/expanded-byte budget before materialization.
- [x] Add failing late-cancellation runner tests for staged attributes,
  isolated-index creation and EOL inspection; make cancellation sticky across
  every process success and error branch.
- [x] Add failing mixed Platform XML/EDT and no-Git regressions for the public
  `checks[]` matrix; publish per-source-set Platform results or `NotRun` and EDT
  `NotApplicable` whenever source-set identities are known, retaining aggregate-
  only observations while `sourceSets` is `null`.
- [x] Add failing cancellation/deadline regressions for maximum-size Git
  attribute and EOL protocols; parse NUL records incrementally with periodic
  checkpoints instead of materializing and cloning every field first.
- [x] Run focused RED -> GREEN verification after every item, then the full Rust,
  clippy, formatting, Python contract and MCP smoke matrix and obtain a new
  independent semantic re-review before declaring the branch merge-ready.
