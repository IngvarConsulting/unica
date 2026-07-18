# Task 5A intermediate review: discovery application contract

Review snapshot:

- base/HEAD: `20f6afa7a09430614babebc0cdeebeb94c8a0189` plus uncommitted Task 5A changes;
- line references were captured at `2026-07-17 20:05 +07` while the implementation was still moving;
- fully read: `task-5a-brief.md` and `task-5-preflight.md`;
- inspected the diff over `model`, `ports`, `determinism`, `evidence_graph`,
  `proposal_validator`, `use_case`, and discovery tests;
- deliberately not repeated without new evidence: explicit analysis-source selection,
  same-source record joins/conflicts, both ExchangePlan edge IDs, complete-vs-bounded
  absence, and the direct-parent structural matrix.

The focused suite compiled. The live mid-review run was:

```text
cargo test --locked -p unica-coder application::discovery -- --nocapture
88 tests: 81 passed, 7 failed
```

The seven failures are listed near the end. They are evidence from a moving RED
worktree, not a request to preserve every old assertion.

## Critical

### C1. The staged support response is not bound back to the exact `SupportQueryPlan`

**References:**

- `crates/unica-coder/src/application/discovery/ports.rs:302-329` creates the
  canonical exact source-scoped subject set.
- `crates/unica-coder/src/application/discovery/ports.rs:915-920` passes that plan
  to `SupportStatePort`.
- `crates/unica-coder/src/application/discovery/use_case.rs:155-158` then collects
  the result through the generic snapshot collector.
- `crates/unica-coder/src/application/discovery/ports.rs:818-864` validates only
  “belongs to any linked snapshot” plus the fact variant. It never compares a
  support record or artifact-scoped gap to `plan.subjects()`.
- The current staged test itself exposes the contradiction:
  `use_case.rs:2584-2604` declares a **Complete** response for only the two target
  facts, while `use_case.rs:2628-2635` proves that the actual query contains four
  keys: analysis owner+target and destination owner+target.

This leaves the exact staged boundary unenforced. A buggy adapter can return a
support fact for an unrequested linked source/artifact, and a `Complete` outcome
can silently omit requested keys. The preflight contract says one raw fact per
queried source artifact. Missing/extra keys cannot be treated as a truthful
complete answer.

**Required RED tests:**

1. `support_complete_must_exactly_cover_query_subjects`: query
   `{main: owner, main: target}` and return only `main: target` as `Complete`;
   collection must fail with `ProviderContractViolation(support_query_subject_missing)`.
2. `support_rejects_unrequested_source_scoped_fact`: return a fact for a linked
   but unrequested `(destination, other-target)`; collection must fail before graph
   construction.
3. `support_bounded_must_account_for_every_missing_key`: a bounded response may
   omit a requested key only when an exact source-scoped gap (or a legitimate
   query-wide gap) accounts for it. Artifact-scoped gaps outside the plan are a
   provider contract violation.

The validation belongs in a support-specific collector accepting both the
`SupportQueryPlan` and snapshot; the generic `collect_for_snapshot_limited` is
not sufficient.

### C2. `maxCandidates` is accepted and hashed but does not bound preliminary work or output

**References:**

- `crates/unica-coder/src/application/discovery/contract.rs:464-475` exposes the
  limit and default.
- `crates/unica-coder/src/application/discovery/use_case.rs:138-152` feeds every
  preliminary candidate into support-subject derivation.
- `crates/unica-coder/src/application/discovery/use_case.rs:205-215` returns all
  final `graph.candidates` unchanged.
- There is no application read of `limits.max_candidates` outside parsing/hashing.

So `maxCandidates=1` can still return and query support for many candidates. It
is not merely an output bug: staged support I/O grows with the unbounded
preliminary candidate set and can consume `maxEvidence` before the one requested
candidate is finalized.

**Required RED test:**

`max_candidates_bounds_preliminary_support_and_final_output` supplies at least
three actionable candidates in forward and reversed provider order with
`maxCandidates=1`. Both executions must:

- select the same one canonical candidate;
- pass only that candidate's ownership chain to support (explicit proposal
  subjects are never dropped);
- return exactly one candidate;
- emit a deterministic non-fatal `candidate_limit` diagnostic when candidates
  were omitted.

Do not take a provider/filesystem prefix.

## Important

### I1. Canonical record deduplication currently treats diagnostic epoch drift as a digest collision

**References:**

- `crates/unica-coder/src/application/discovery/determinism.rs:400-406` explicitly
  excludes `workspaceEpoch` from the evidence digest.
- `crates/unica-coder/src/application/discovery/ports.rs:669-719` deduplicates in
  `collect_ready` and compares duplicate-digest records with derived full `Eq`.
  `Freshness.workspace_epoch` therefore participates in that equality.
- Snapshot epoch normalization happens only later in
  `ports.rs:447-457` (`collect_for_snapshot`).

Two semantically identical records with the same source fingerprint but
diagnostic epochs `1` and `2` have the same correct digest and are incorrectly
reported as a non-identical digest collision before both epochs can be normalized.

**Required RED test:**

`diagnostic_epoch_does_not_create_a_duplicate_digest_collision` returns two
otherwise byte-identical facts at epochs `1` and `2`; `collect_for_snapshot`
must produce one canonical record at the captured snapshot epoch, not a provider
contract violation.

Normalize diagnostic metadata before canonical duplicate comparison, or compare
the exact digest-bound payload rather than derived `EvidenceRecord::Eq`.

### I2. Public checks collapse scoped gaps and contaminate unrelated material consumers

**References:**

- `crates/unica-coder/src/application/discovery/proposal_validator.rs:146-161`
  now preserves scoped gap reasons in each proposal verdict.
- `crates/unica-coder/src/application/discovery/use_case.rs:510-610` still emits
  exactly one check per provider.
- `use_case.rs:529-550` assigns a degraded provider to every proposal using the
  port, regardless of each gap's source-scoped artifact set.
- Candidate effects at `use_case.rs:536-547` are inferred only from retained
  record IDs. A query-wide gap with zero or unrelated retained records therefore
  affects no candidate.
- `use_case.rs:561-567` collapses all bounded gap reasons to
  `provider_inconclusive`; `use_case.rs:605` always drops gap locations/details.
- The live RED `query_wide_gap_is_material_to_every_port_consumer` fails at
  `use_case.rs:2695-2701` for exactly the missing candidate affect.

Multiple independent `ProviderGap`s cannot be represented truthfully by one
generic provider check: their reasons, scopes, affected proposals/candidates,
and locations differ. Port-wide unioning also creates false blocking checks for
an exact proposal untouched by the gap.

**Required RED tests:**

1. `scoped_gap_check_does_not_block_unrelated_proposal`: a provider is bounded
   only for source-scoped artifact A while exact proposal B is fully proved. The
   check must not affect B and must not deny B's otherwise eligible receipt.
2. `multiple_gaps_keep_independent_public_scopes`: `g_a` affects only proposal A
   and `g_b` only proposal B. Public checks must preserve both reason/scope pairs
   deterministically (one check per gap is the simplest representation).
3. `query_wide_zero_record_gap_affects_every_material_candidate`: a query-wide
   gap must affect all candidate consumers of that port even with zero retained
   IDs.

### I3. Runtime materiality is widened to every possible method port after one path is already proved

**References:**

- `crates/unica-coder/src/application/discovery/proposal_validator.rs:97-122`
  allows one exact runtime connection to prove reachability.
- `proposal_validator.rs:137-145` nevertheless marks the whole
  `runtime_ports_for` set material.
- `proposal_validator.rs:275-285` defines every method as consuming Metadata,
  CallGraph, FormInspection, and Definition for runtime materiality.
- Existing REDs prove the regression:
  `metadata_callback_makes_unavailable_form_inspection_optional`
  (`use_case.rs:1442`) and
  `metadata_callback_makes_bounded_form_inspection_optional`
  (`use_case.rs:1469`) now fail.

Once a compatible metadata callback+definition proves runtime reachability,
unavailable FormInspection is optional for an ordinary method. Conversely, an
exact form-command binding keeps FormInspection mandatory. The material set must
be derived from the actual proof path (or from every possible path only while
reachability remains unknown), not from target kind alone.

**Required RED:** preserve the two existing optional-form tests and add their
dual: when a FormInspection edge is the proof actually used, degrading that port
must remain blocking even if other unused ports are complete.

### I4. `extension_owned` is inferred from source-set equality, not destination ownership

**References:**

- `crates/unica-coder/src/application/discovery/proposal_validator.rs:308-365`
  performs CFE support projection.
- `proposal_validator.rs:349-354` returns `ExtensionOwned` only when
  `destination_source_set == analysis_source_set`; otherwise every safe
  destination becomes `ExtensionRequired`.
- Controller correction in `task-5a-brief.md:200-204` requires exact destination
  membership: safely writable destination is `extension_owned` only if the
  target already belongs to that same extension.
- Current test `use_case.rs:2501` covers extension-required, locked, and missing
  analysis, but has no already-owned destination case.

Source-set equality is not ownership evidence and is normally false for a CFE
destination. The validator must use exact destination catalog/ownership evidence
for the target's chain.

**Required RED:** two requests have the same analysis support fact and same safe
destination support state. In the first, exact destination metadata proves the
target/owner already exists and the projection must be `ExtensionOwned`; in the
second it is absent/not owned and the projection must be `ExtensionRequired`.
Missing or conflicting destination membership remains `Unknown`.

### I5. `maxEvidence` still does not bound a single semantic fact or the gap vector

**References:**

- `crates/unica-coder/src/application/discovery/model.rs:1437-1469` accepts an
  unbounded `DefinitionShape.parameters` vector.
- `model.rs:1533-1575` accepts an unbounded callback parameter vector.
- Those fields are `pub(crate)`, so sibling adapters can bypass the smart
  constructors entirely.
- `crates/unica-coder/src/application/discovery/ports.rs:840-864` validates only
  the fact variant/port (plus bindings), not shape invariants.
- `crates/unica-coder/src/application/discovery/determinism.rs:424-463` hashes
  both vectors without a count cap.
- `crates/unica-coder/src/application/discovery/ports.rs:789-795` canonicalizes
  an unbounded number of `ProviderGap`s; only each artifact list is capped.

Therefore `maxEvidence=1` can still admit one arbitrarily large fact or millions
of gaps. This is not a real memory/CPU boundary.

**Required RED tests:**

1. Choose and document one exact `MAX_SIGNATURE_PARAMETERS`. Both shape
   constructors reject `MAX+1`; a deliberately forged invalid struct returned
   by an adapter is rejected by `validate_fact_for_port` as a provider contract
   violation.
2. Add a documented per-outcome gap-count cap. A provider-supplied `MAX+1` gap
   list is rejected deterministically rather than hashed/retained unboundedly.
3. Apply the same bounded-payload validation to `CodeOccurrence.search_term`
   and other directly constructible strings; one record must not hide an
   unbounded payload.

### I6. Exact truncation becomes a fatal operation when the dropped scope has more than 2000 identities

**References:**

- `crates/unica-coder/src/application/discovery/model.rs:1223-1239` rejects an
  artifact gap with more than 2000 source-scoped artifacts.
- Per-port truncation builds one gap from every dropped endpoint at
  `crates/unica-coder/src/application/discovery/ports.rs:522-540`.
- Global truncation does the same at `ports.rs:655-664`.

A provider returning more than 2000 distinct omitted subjects/endpoints can
therefore turn a valid resource-limit path into `DiscoveryError::Operation`.
The limit path must be fail-closed but non-fatal.

**Required RED:** return at least 2001 distinct dropped source-scoped subjects
with `maxEvidence=1`. Execution must return a bounded/insufficient report, never
an operation error. Use deterministic exact chunks while reasonably bounded, or
a documented query-wide overflow fallback once an exact scope cannot be encoded.

### I7. Existing bounded-positive verdict test and new gap policy disagree

**References:**

- Existing test `bounded_material_records_keep_supported_verdict_but_status_insufficient`
  at `crates/unica-coder/src/application/discovery/use_case.rs:1742` expects an
  exact positive fact under a query-wide material gap to keep verdict
  `Supported`, while receipt/status remain blocked.
- New validator logic at
  `crates/unica-coder/src/application/discovery/proposal_validator.rs:173-185`
  makes any material `coverage_gaps` force verdict `Unknown`.
- The live test failed with `left: Unknown, right: Supported`.

Do not repair this by weakening whichever side happens to be newest. The active
spec must say whether a positive fact plus an unrelated-but-query-wide material
gap is a supported conclusion with an ineligible receipt, or an unknown
conclusion. Then align validator, report consistency, and the test. The controller
phrase “degrades every material consumer” is not by itself precise enough about
the public verdict enum.

## Minor / explicit defer

### M1. `maxGraphDepth` needs an explicit Task 7 defer in the active spec

**References:**

- `crates/unica-coder/src/application/discovery/contract.rs:464-475` accepts and
  defaults `maxGraphDepth`.
- `spec/architecture/extension-point-discovery.md:258-260` currently calls it a
  bounded resource limit.
- Task 5A has no application traversal-root/depth semantics and does not enforce
  it.

Controller decision: traversal depth belongs to Task 7. Task 5A should therefore
update the active spec/product-contract text explicitly: the field is accepted
and analysis-ID-bound now for forward compatibility, while traversal semantics
and enforcement are deferred to Task 7. Without that sentence the current spec
promises an effective resource bound that does not exist.

**RED:** product-contract test requires the explicit defer text and Task 7 owner;
the future Task 7 RED must define roots and prove that depth `N` excludes depth
`N+1` independent of provider order.

## Findings corrected in the worktree during this review

These were reported to the implementer and then corrected before this file was
finalized; keep their regressions:

1. `ProviderGap::Artifacts` is now source-qualified through
   `SourceScopedArtifact` (`model.rs:1175-1247`), preventing analysis/destination
   scope aliasing.
2. Known-record truncation now emits exact artifact-scoped gaps rather than the
   earlier contradictory query-wide gap (`ports.rs:522-540`, `655-664`).
3. The first global limiter version changed retained records to `Bounded` even
   for a provider that lost nothing, while leaving that provider's coverage and
   snapshot `Complete`. The current branch at `ports.rs:627-665` now changes and
   refreshes only actually truncated providers.
4. Exact duplicate records no longer consume `maxEvidence` before truncation;
   canonicalization is now at `ports.rs:669-719`. The remaining diagnostic-epoch
   issue is I1 above.
5. A low `maxEvidence` no longer makes an oversized staged subject set fatal:
   `use_case.rs:147-167` and `272-317` retain a canonical bounded prefix and add
   an exact `support_subject_limit` gap for omitted subjects.

## Live RED failures at review handoff

The focused run had these seven failures:

1. `bounded_material_records_keep_supported_verdict_but_status_insufficient`
   — policy contradiction captured as I7.
2. `metadata_callback_makes_unavailable_form_inspection_optional`
   — materiality regression captured as I3.
3. `metadata_callback_makes_bounded_form_inspection_optional`
   — materiality regression captured as I3.
4. `query_wide_gap_is_material_to_every_port_consumer`
   — candidate gap materialization captured as I2.
5. `callback_definition_absence_is_no_only_with_complete_definition_coverage`
   — its newly constructed bounded record carries the wrong provider identity;
   fix the test fixture, do not weaken the batch provider contract.
6. `structural_contains_edge_is_observed_but_never_runtime_reachable`
   — the old helper now constructs a relation rejected by the accepted
   direct-parent matrix; this is a stale test fixture in a known review area.
7. `every_runtime_port_that_contributes_a_connection_is_material`
   — currently returns `Contradicted` instead of `Supported`; re-evaluate after
   I3 is fixed because its runtime proof/material-port selection changed.

## Handoff order

1. Add C1, C2, I1, I2, and I4 REDs before more orchestration refactoring.
2. Fix I3 without weakening the exact FormCommand rule.
3. Close I5/I6 resource boundaries.
4. Resolve and document I7 explicitly.
5. Record M1 in the active spec as Task 7 work.
6. Re-run the focused suite, full crate suite, fmt, clippy `-D warnings`, product
   contract, and `git diff --check`.
