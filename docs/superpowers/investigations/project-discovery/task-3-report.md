# Task 3 Report: Typed Evidence Graph And Validation

## Status

DONE

## RED

- Command: `cargo test --locked -p unica-coder discovery::use_case -- --nocapture`
- Result: exit 101.
- Expected failure: the application slice did not exist yet. The compiler
  reported missing `ports`, `evidence_graph`, and `proposal_validator` modules,
  the domain `source_snapshot` module, typed call semantics, all six evidence
  port traits, `ProviderOutcome`, `DiscoveryExecutionContext`, the use case, and
  `NoopReceiptIssuer`.
- Self-review regression RED: `cargo test --locked -p unica-coder disabled_scheduled_job_binding_is_observed_but_not_actionable -- --nocapture`
  exited 101 because a disabled scheduled-job binding was incorrectly promoted
  to an actionable candidate. The production fix retains the fact as observed
  evidence but does not build a runtime edge.

## Files Changed

- `crates/unica-coder/src/application/discovery/ports.rs`
- `crates/unica-coder/src/application/discovery/evidence_graph.rs`
- `crates/unica-coder/src/application/discovery/proposal_validator.rs`
- `crates/unica-coder/src/application/discovery/use_case.rs`
- `crates/unica-coder/src/application/discovery/mod.rs`
- `crates/unica-coder/src/application/discovery/model.rs`
- `crates/unica-coder/src/application/discovery/determinism.rs`
- `crates/unica-coder/src/domain/source_snapshot.rs`
- `crates/unica-coder/src/domain/mod.rs`
- `spec/architecture/extension-point-discovery.md`
- `docs/superpowers/plans/2026-07-17-project-discovery-receipts.md`

## Design Decisions

- Defined exactly six evidence traits and kept source resolution, source
  snapshots, and receipt eligibility behind three separate orchestration
  traits.
- Modeled all provider states explicitly. Only contract violations return a
  fatal evidence error; bounded, unavailable, and failed outcomes remain typed
  inconclusive checks.
- Kept every fact typed and provenance-bound. Port/fact mismatches, provider or
  coverage mismatches, invalid locations/freshness, and invalid stable digests
  become provider contract violations.
- Bound binding mechanism details and call resolution/type/context into stable
  evidence digests with frozen stable tags. Disabled scheduled jobs remain
  observed but cannot establish runtime reachability.
- Built only the six accepted edge kinds and derived lexical, observed,
  connected, and actionable levels from facts rather than ordering.
- Evaluated materiality per proposal and kept optional degradation separate
  from receipt safety. Validate-mode material unknowns produce `insufficient`;
  conclusive conclusions with only optional degradation may be `partial`.
- Retained conflicting facts and emitted blocking conflict checks for affected
  conclusions.
- Modeled one content-fingerprint analysis snapshot plus sorted/deduplicated
  mutation snapshots with the existing domain `SourceFormat`.
- Kept receipt persistence out of scope. The no-op issuer always returns
  `eligible=false` with `receipt_store_not_implemented`; deterministic tests may
  inject an allow-only fake after all application eligibility checks pass.
- Synchronized the accepted architecture spec and historical execution plan
  with the resolved outcome/snapshot/mechanism contracts and corrected commit
  scope.

## Verification

- Focused: `cargo test --locked -p unica-coder discovery::use_case -- --nocapture`
  — exit 0, 13 passed, 0 failed.
- Required discovery suite: `cargo test --locked -p unica-coder discovery -- --nocapture`
  — exit 0, 46 passed, 0 failed.
- Full crate: `cargo test --locked -p unica-coder`
  — exit 0, 441 passed, 0 failed; doc tests also passed.
- Format: `cargo fmt --all -- --check`
  — exit 0, no differences.
- Clippy: `cargo clippy --locked -p unica-coder --all-targets -- -D warnings`
  — exit 0, no warnings.
- Product contracts: `python3 tests/ci/test_product_contracts.py`
  — exit 0, 14 passed.
- Diff: `git diff --cached --check`
  — exit 0, no whitespace errors; exactly the 11 intended files were staged.
- Boundary: negated `rg` scan for infrastructure, filesystem, and SQLite access
  in the new application modules — exit 0, no forbidden matches.
- Port count: `rg -c '^evidence_port!' crates/unica-coder/src/application/discovery/ports.rs`
  — exactly 6.

## Commit

- `f4ddc91e4ee826289bf850160ec254a381ff901e` (`feat: реализовать evidence graph и validation`)

## Concerns

- No unresolved Task 3 concerns. Concrete filesystem snapshot capture,
  infrastructure providers, public MCP registration, and persistent receipt
  issuance remain deliberately blocked behind later tasks.
- The subagent runtime does not expose a token-usage counter (`get_goal`
  returned no active goal), so an exact token count cannot be reported.

## Review Fix Wave 1

### RED

- `cargo test -p unica-coder application::discovery::use_case::tests -- --nocapture`
  — exit 101, 13 passed and 6 failed after correcting the bounded-outcome
  fixture provider identity. Expected failures proved that unavailable and
  bounded metadata were converted to an exact negative runtime conclusion,
  incompatible definition shapes still produced `supported`, a bounded
  material check still allowed `complete`, and analysis/mutation snapshot
  aliases, omissions, and extras were accepted.
- `cargo test -p unica-coder application::discovery::ports::tests -- --nocapture`
  — exit 101, 1 passed and 2 failed. Expected failures proved that impossible
  binding relation/detail/port combinations, including
  `FormInspection + calls + structural`, crossed the provider boundary.
- `cargo test -p unica-coder domain::source_snapshot::tests -- --nocapture`
  — exit 101, 2 passed and 1 failed. Expected failure proved that two mutation
  snapshots with the same name and role but conflicting mapping identities
  were accepted.

### Files Changed

- `crates/unica-coder/src/application/discovery/evidence_graph.rs`
- `crates/unica-coder/src/application/discovery/ports.rs`
- `crates/unica-coder/src/application/discovery/proposal_validator.rs`
- `crates/unica-coder/src/application/discovery/use_case.rs`
- `crates/unica-coder/src/domain/source_snapshot.rs`

### Fixes

- Exact negative runtime reachability now requires complete coverage from
  `CallGraph`, `FormInspection`, and `MetadataCatalog`.
- Distinct exact `DefinitionShape` values for one canonical subject retain all
  evidence, emit `conflicting_definition_shapes`, downgrade the candidate, and
  block the affected proposal and receipt.
- One canonical provider-boundary validator now enforces every accepted and
  rejected `BindingDetails`/`FlowKind`/port combination before graph promotion.
- Blocking unresolved checks take precedence over conclusive positive facts;
  `partial` remains limited to optional warning degradation.
- Snapshot identity now includes role, source-set name/kind/format/root,
  mapping digest, and content fingerprint. The use case rejects captured
  analysis/mutation identities that differ from the resolved request by alias,
  omission, or extra source.

### Verification

- Focused use case: `cargo test -p unica-coder application::discovery::use_case::tests -- --nocapture`
  — exit 0, 19 passed.
- Exhaustive binding contract: `cargo test --locked -p unica-coder application::discovery::ports::tests -- --nocapture`
  — exit 0, 3 passed; all 6 ports x 6 relations x 7 detail variants were
  classified against the canonical allow-list.
- Snapshot identity: `cargo test -p unica-coder domain::source_snapshot::tests -- --nocapture`
  — exit 0, 4 passed.
- Discovery suite: `cargo test --locked -p unica-coder discovery -- --nocapture`
  — exit 0, 55 passed.
- Full crate: `cargo test --locked -p unica-coder`
  — exit 0, 453 passed; doc tests passed.
- Format: `cargo fmt --all -- --check` — exit 0.
- Clippy: `cargo clippy --locked -p unica-coder --all-targets -- -D warnings`
  — exit 0, no warnings.
- Product contracts: `python3 tests/ci/test_product_contracts.py`
  — exit 0, 14 passed.
- Boundary scan, exact six-port count, `git diff --cached --check`, and final
  `git diff --check` — exit 0.

### Commit

- `abbc0416bbf8443f3f0949ed2413c13e6554dc0c`
  (`fix: усилить discovery evidence validation`).

### Review State

- Fix wave completed and committed. Review is deliberately not marked clean;
  the commit is awaiting re-review.
- Exact token usage remains unavailable because this subagent runtime exposes
  no active goal/token counter.

## Review Fix Wave 2

### RED

- `cargo test --locked -p unica-coder application::discovery::use_case::tests::metadata_callback_makes_ -- --nocapture`
  — exit 101, 0 passed and 2 failed. Metadata callback evidence established
  runtime reachability, but unavailable and bounded `FormInspectionPort`
  outcomes were still material, producing `insufficient` instead of `partial`
  and blocking the fake issuer's otherwise eligible receipt.
- `cargo test --locked -p unica-coder application::discovery::use_case::tests::every_runtime_port_that_contributes_a_connection_is_material -- --nocapture`
  — exit 101, 0 passed and 1 failed. With both CallGraph and FormInspection
  connections, bounded FormInspection was incorrectly optional and produced
  `partial` instead of `insufficient`.
- `python3 tests/ci/test_product_contracts.py`
  — exit 1 with 16 matrix subtest failures before documentation changes: all
  seven exact rows plus the contract-violation clause were absent from both the
  active spec and historical Task 3 plan. A second RED assertion produced two
  failures for the missing per-connection runtime-materiality rule.

### Files Changed

- `crates/unica-coder/src/application/discovery/proposal_validator.rs`
- `crates/unica-coder/src/application/discovery/use_case.rs`
- `spec/architecture/extension-point-discovery.md`
- `docs/superpowers/plans/2026-07-17-project-discovery-receipts.md`
- `tests/ci/test_product_contracts.py`

### Fixes

- A positive runtime verdict now makes every port in the target's actual
  `connection_ports` material. Potential runtime providers that contributed no
  connection remain optional; exact negative proof still requires complete
  MetadataCatalog, CallGraph, and FormInspection coverage.
- Metadata-only callback evidence with unavailable or bounded form inspection
  remains `supported + partial`; an injected eligible issuer may still return
  `eligible=true` because the degraded form provider did not support the
  conclusion.
- The active architecture spec and historical Task 3 plan now contain the
  exact seven-row `BindingDetails` x `FlowKind` x evidence-port allow-list,
  reject every other combination before graph promotion, and document runtime
  materiality by actual evidence contribution.
- The product-contract test parses both Markdown tables, rejects duplicate or
  malformed rows, and compares the complete relation-set/provider mapping; it
  does not rely only on substring anchors.

### GREEN Verification

- Focused use case: `cargo test --locked -p unica-coder application::discovery::use_case::tests -- --nocapture`
  — exit 0, 22 passed.
- Discovery suite: `cargo test --locked -p unica-coder discovery -- --nocapture`
  — exit 0, 58 passed.
- Full crate: `cargo test --locked -p unica-coder`
  — exit 0, 456 passed; doc tests passed.
- Format: `cargo fmt --all -- --check` — exit 0.
- Clippy: `cargo clippy --locked -p unica-coder --all-targets -- -D warnings`
  — exit 0, no warnings.
- Product contracts: `python3 tests/ci/test_product_contracts.py`
  — exit 0, 14 passed.
- Boundary scan, `git diff --check`, and `git diff --cached --check`
  — exit 0.

### Commit

- `eae064ad2daa5316e6ceb2962a45be8d0a659feb`
  (`fix: уточнить materiality discovery evidence`).

### Review State

- Fix wave completed and committed. Review remains awaiting re-review and is
  not self-approved.
- Exact token usage remains unavailable because this subagent runtime exposes
  no active goal/token counter.

## Review Fix Wave 3

### RED

- `cargo test --locked -p unica-coder application::discovery::use_case::tests::structural_ -- --nocapture`
  — exit 101, 0 passed and 2 failed. Both `Defines` and `Contains` structural
  bindings produced `Supported` instead of `Contradicted`, because structural
  edge endpoints were incorrectly promoted to runtime-connected artifacts.
- `python3 tests/ci/test_product_contracts.py`
  — exit 1 with 2 subtest failures, one for each document missing the explicit
  structural-edge semantics clause.

### Files Changed

- `crates/unica-coder/src/application/discovery/evidence_graph.rs`
- `crates/unica-coder/src/application/discovery/use_case.rs`
- `spec/architecture/extension-point-discovery.md`
- `docs/superpowers/plans/2026-07-17-project-discovery-receipts.md`
- `tests/ci/test_product_contracts.py`

### Fixes

- Only `Calls`, `Handles`, `Subscribes`, and `Uses` edges now mark artifacts as
  runtime-connected and populate `connection_ports`.
- `Contains` and `Defines` remain typed graph edges with `Observed` evidence and
  `binding_observed` reason codes, but never establish runtime reachability or
  make a candidate actionable.
- Complete-empty CallGraph and FormInspection evidence now correctly disproves
  runtime reachability when metadata contains only structural bindings, blocks
  receipt eligibility, and retains the structural graph evidence.
- Wave 2 materiality behavior is preserved: every actual runtime provider in
  `connection_ports` remains material.
- The active spec, historical Task 3 plan, and product-contract checks now state
  and enforce the structural-edge rule explicitly.

### GREEN Verification

- Focused use case: `cargo test --locked -p unica-coder application::discovery::use_case::tests -- --nocapture`
  — exit 0, 24 passed.
- Discovery suite: `cargo test --locked -p unica-coder discovery -- --nocapture`
  — exit 0, 60 passed.
- Full crate: `cargo test --locked -p unica-coder`
  — exit 0, 458 passed; doc tests passed.
- Explicit runtime-materiality regression:
  `cargo test --locked -p unica-coder application::discovery::use_case::tests::every_runtime_port_that_contributes_a_connection_is_material -- --nocapture`
  — exit 0, 1 passed.
- Format: `cargo fmt --all -- --check` — exit 0.
- Clippy: `cargo clippy --locked -p unica-coder --all-targets -- -D warnings`
  — exit 0, no warnings.
- Product contracts: `python3 tests/ci/test_product_contracts.py`
  — exit 0, 14 passed.
- Architecture boundary scan, `git diff --check`, and
  `git diff --cached --check` — exit 0.

### Commit

- `0fce5d9b031a43dfce47a89e7df0343f32e4bae2`
  (`fix: отделить structural edges от runtime`).

### Review State

- Fix wave completed and committed. Review remains awaiting re-review and is
  not self-approved.
- Exact token usage remains unavailable because this subagent runtime exposes
  no active goal/token counter.
