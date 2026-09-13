# A0 analysis handlers implementation plan

**Goal:** Replace the provisional v0.13 `check` and `diff` behavior with independently testable typed handlers that run closed validator profiles, normalize diagnostics, and paginate revision-bound differences.

**Architecture:** `application/v13/check.rs` and `application/v13/diff.rs` own request validation, result shapes, bounded traversal, and opaque cursor semantics. `infrastructure/native_operations/v13_analysis.rs` owns the closed native-validator registry and converts existing family validators into the application diagnostic model. The shared daemon dispatcher, public catalog, and coverage ledger remain untouched for J0.

**Tech Stack:** Rust, serde/serde_json, existing native XML validators, UUID-backed process-local cursor capabilities.

**Spec:** GitHub issue #581, stream A0 (`check` and `diff`).

**Decision:** `none — no architectural contract changed`; this stream adds private typed seams and does not change the public dispatcher or tool catalog.

## Global constraints

- Start from `origin/main` at `8a13c2e711d0a31ed17d06f02bf08cd8eec36353`.
- Do not edit `v13_service.rs`, `tool_catalog.rs`, `interfaces/*`, package metadata, or `arch/tool-implementation-coverage.json`; J0 owns shared wiring.
- Keep validator profiles and filters closed; unknown profiles are typed unsupported errors.
- Never expose physical paths, provider identities, raw commands, stdout, or stderr in typed results.
- Apply the diff limit while traversing changes; do not materialize an unbounded change list.

### Task 1: Typed check request, registry, and normalized result

**Files:**

- Create: `crates/unica-coder/src/application/v13/check.rs`
- Modify: `crates/unica-coder/src/application/v13/mod.rs`

- [ ] Write tests for the closed profile/filter registry, logical request parsing, normalized diagnostics, and unavailable-validator errors.
- [ ] Run the focused tests and verify the new behavior fails because the module/types do not exist.
- [ ] Implement the smallest typed request/result API and diagnostic normalization.
- [ ] Run the focused tests and the v13 application test target.

### Task 2: Native validator adapter

**Files:**

- Create: `crates/unica-coder/src/infrastructure/native_operations/v13_analysis.rs`
- Modify: `crates/unica-coder/src/infrastructure/native_operations.rs`

- [ ] Write tests that invoke a real existing validator through the closed registry and prove that paths/raw streams are absent from the normalized result.
- [ ] Run the focused tests to record the RED state.
- [ ] Implement profile-to-validator dispatch for the proven native validators and typed unavailable/unsupported outcomes.
- [ ] Run the focused native-operation tests and the affected Rust suite.

### Task 3: Revision-aware bounded diff handler

**Files:**

- Create: `crates/unica-coder/src/application/v13/diff.rs`
- Modify: `crates/unica-coder/src/application/v13/mod.rs`

- [ ] Write tests for incomparable kinds, bounded output, cursor binding to both revisions, stale cursors, and filter/request replay binding.
- [ ] Run the focused tests to verify RED.
- [ ] Implement deterministic JSON traversal with a bounded skip/collect window and opaque process-local cursors.
- [ ] Run focused diff tests and the affected Rust suite.

### Task 4: Fixture and exit evidence

**Files:**

- Modify: `tests/fixtures/v013/domain-parity/check-diff.json`
- Add focused Rust tests only within the A0-owned modules.

- [ ] Record the supported profile/filter and diff lifecycle cases in the existing fixture without changing public coverage wiring.
- [ ] Run formatting, focused tests, full `cargo test -p unica-coder`, and repository architecture/source guards.
- [ ] Review the final diff for ownership violations and report the exact verification results.
