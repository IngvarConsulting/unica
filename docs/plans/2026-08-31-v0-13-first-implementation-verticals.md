# Unica v0.13 First Implementation Verticals Plan

> **For agentic workers:** use `superpowers:test-driven-development` for each
> production change and `superpowers:verification-before-completion` before
> reporting a slice complete. The integrator executes this plan with
> `superpowers:subagent-driven-development`.

**Goal:** implement the first connected metadata, read and runtime workflows
behind the already-selected 8/11 v0.13 surface without restoring legacy tools.

**Architecture:** actor-owned reads and retained apply remain the only source
authorities. Metadata dry-run and publication share one planner. Runtime adds
only bounded `syntax.check`; `query.execute` is absent from v0.13.

**Tech stack:** Rust (`unica-coder`, `unica-bootstrap`), serde JSON contracts,
Python architecture/CI guards, Cargo unit and integration tests.

**Spec:** `docs/design/2026-08-31-v0-13-first-implementation-verticals-design.md`

**Global constraints:** do not edit `plugins/unica/skills/**`; do not add legacy
aliases; do not bypass `WorkspaceActor`; preserve unrelated ReceiptLedger/v5
worktree changes; never run `cargo fmt --all` in this dirty worktree.

## Task 1: Machine-readable implementation coverage — completed

**Files:**

- Create: `arch/tool-implementation-coverage.json`
- Create: `tests/ci/test_v013_implementation_coverage.py`
- Modify: `arch/README.md`
- Modify: `arch/decisions/2026-08-31-v0-13-first-implementation-verticals.md`
- Create: `arch/invariants/INV.APP.V13-IMPLEMENTATION-COVERAGE.md`

1. Write a failing CI test that requires all eight public tools, all thirteen
   closed run dictionary operations, the three compatibility Task tools, one of
   four statuses, and non-empty executable evidence for `supported` modes.
2. Run `python3 -m pytest tests/ci/test_v013_implementation_coverage.py -q` and
   confirm failure because the record does not exist.
3. Add the coverage record with current truth: existing useful modes are
   `partial`, run operations begin `unsupported`, Task transport is
   `supported`, removed legacy behavior is not represented as a public mode;
   promote only `syntax.check` after its bounded evidence passes.
4. Register the decision/invariant and run the focused test plus
   `python3 scripts/arch/registry.py check`.

## Task 2: Metadata-family shared planner — partial, proved subset completed

**Files:**

- Modify: `crates/unica-coder/src/infrastructure/native_operations/apply_families/metadata.rs`
- Modify: `crates/unica-coder/src/infrastructure/native_operations/apply_families/mod.rs`
- Reuse: `crates/unica-coder/src/infrastructure/native_operations/meta/edit.rs`
- Reuse: `crates/unica-coder/src/infrastructure/native_operations/meta/publisher.rs`
- Test in: `crates/unica-coder/src/infrastructure/native_operations/apply_families/metadata.rs`

1. Replace the stable-unsupported seam test with failing table tests for
   `props.set`, `relation.*`, `attribute.*`, `object.create/remove`: exact
   argument validation, operation-index paths, deterministic staged changes,
   ordered multi-op postimage, and no disk mutation while planning.
2. Run the focused Rust tests and observe the expected unsupported failure.
3. Parse each operation into a closed enum; reject unknown or misplaced fields.
4. Resolve logical addresses to retained relative Platform XML resources and
   build each operation against the staged postimage, reusing the typed metadata
   editor/template/removal primitives instead of copying XML rules.
5. Emit closed domain events for every changed artifact and prove deduplication.
6. Re-run focused tests; do not integrate publication yet.

Result: `props.set` and `attribute.add/set/remove` are staged. `object.*` lacks
a retained template/remove planner and `relation.*` lacks a closed relation
selector/dependency contract, so those five names remain typed unsupported.

## Task 3: Read projections — first closed projections completed

**Files:**

- Create: `crates/unica-coder/src/infrastructure/daemon/v13_read_modes.rs`
- Modify only at integration: `crates/unica-coder/src/infrastructure/daemon/mod.rs`
- Modify only at integration: `crates/unica-coder/src/infrastructure/daemon/v13_service.rs`
- Test in: `crates/unica-coder/src/infrastructure/daemon/v13_read_modes.rs`

1. Add failing fixture-driven tests for `sections`, bounded object-subtree
   filtering, the reserved validation profile union, and path/section diff
   filters.
2. Confirm current behavior returns `unsupported_filter` or
   `unsupported_scope`.
3. Implement pure argument parsing/projection helpers in the new module. The
   module accepts already-authorized logical data/diagnostics and never opens a
   path itself.
4. Add negative tests for unknown members, wrong types, physical paths,
   descendant scopes without exact ownership and cross-source leakage.
5. Leave daemon routing changes to the integrator.

## Task 4: Bounded `syntax.check` — completed

**Files:**

- Create: `crates/unica-coder/src/infrastructure/daemon/v13_syntax_run.rs`
- Reuse: `crates/unica-coder/src/infrastructure/runtime_jobs.rs`
- Reuse: `crates/unica-coder/src/infrastructure/internal_adapters.rs`
- Modify only at integration: `crates/unica-coder/src/infrastructure/daemon/mod.rs`
- Modify only at integration: `crates/unica-coder/src/infrastructure/daemon/v13_service.rs`
- Test in: `crates/unica-coder/src/infrastructure/daemon/v13_syntax_run.rs`

1. Add failing tests for the accepted closed `syntax.check` arguments, command
   construction through the existing runtime abstraction, cancellation, typed
   provider failure, and rejection of command-line/query fields.
2. Confirm the current public handler returns `unsupported_operation`.
3. Implement a bounded adapter that converts canonical args to the existing
   runtime syntax request and maps its terminal result into `DomainResult`.
4. Keep `query.execute` absent from the adapter and dictionary, and add an
   explicit unknown-operation rejection regression test.
5. Leave daemon routing and dictionary `implemented` projection to integrator.

## Task 5: Integrate apply dry-run and publication — completed for proved metadata subset

**Files:**

- Modify: `crates/unica-coder/src/infrastructure/daemon/v13_service.rs`
- Modify: `crates/unica-coder/src/infrastructure/workspace_actor.rs` only if a
  missing narrow publication method is proven by a failing test
- Test in: `crates/unica-coder/src/infrastructure/daemon/server.rs`

1. Add failing end-to-end actor tests proving dry-run returns a non-empty plan
   without source/cache/revision changes, and real apply publishes the same plan
   atomically with effects and a new revision.
2. Feed `plan_hidden_v13_apply` into `ApplyAdmission::prepare_with_effects`.
3. For dry-run, report prepared source changes/effects without commit. For real
   apply, cross the existing final actor gate and commit the retained batch.
4. Map staging, concurrency, support and postcondition failures to canonical
   typed codes with exact `ops[i]` locations.
5. Add mixed supported/unsupported batch tests proving no partial publication.

## Task 6: Integrate read and run modules — completed for this iteration

**Files:**

- Modify: `crates/unica-coder/src/infrastructure/daemon/mod.rs`
- Modify: `crates/unica-coder/src/infrastructure/daemon/v13_service.rs`
- Modify: `crates/unica-coder/src/application/v13/tool_catalog.rs`
- Test in: `crates/unica-coder/src/infrastructure/daemon/server.rs`

1. Add failing production-composition tests for each new read mode and direct
   plus compatibility/native Task `syntax.check` projection.
2. Wire the read helpers behind existing actor authorities.
3. Wire `syntax.check`; set `implemented:true` only for that dictionary entry.
4. Assert `query.execute` is absent from discovery and rejected as unknown.
5. Run focused daemon, invocation and Task suites.

## Task 7: Synchronize truth and verify

**Files:**

- Modify: `arch/tool-implementation-coverage.json`
- Modify: `docs/design/2026-08-31-v0-13-surface-first-cutover-design.md`
- Modify: `arch/tool-surface.md` only if result-contract evidence changes
- Modify: `arch/tool-surface-review.json` only if result-contract evidence changes

1. Change coverage statuses only after their named executable tests pass.
2. Update affected migration-matrix rows from stub/unsupported to exact partial
   support; retain every loss and deferred boundary.
3. Run focused tests, then `cargo check -p unica-coder -p unica-bootstrap`, all
   affected Rust suites, matrix/coverage/ledger/design tests, architecture
   registry and bootstrap exact-set verification.
4. Run independent specification and code-quality reviews; fix findings with a
   new failing test first.
5. Report completed modes, remaining unsupported modes, release boundary, and
   token usage. Do not publish a release.
