# Task 5A adversarial code review

Date: 2026-07-17

Base: `20f6afa7a09430614babebc0cdeebeb94c8a0189`

Scope: frozen uncommitted Task 5A application diff. The pending UUID-bound
destination-membership slice and documentation synchronization were excluded.
No tracked file was edited.

## Verdict

**Not ready.** In addition to the destination-membership/spec findings from the
parallel review, I found four distinct application-contract defects.

## Findings

### P1 — every connected positive artifact is promoted to a candidate

`EvidenceGraph::build_for_analysis()` creates a `Candidate` for every artifact
with `positive_existence && connected` (`evidence_graph.rs:455-502`). There is
no candidate-capability check for the mechanism/endpoint role. This promotes
runtime context nodes that are explicitly not extension points.

Concrete reproducer:

1. emit `MetadataPresent(Document.Sale)`;
2. emit the registered Document `BeforeWrite` callback and a compatible exact
   `DefinitionPresent(Document.Sale.ObjectModule.BeforeWrite)`;
3. return known support for both support-query subjects.

The callback edge marks both endpoints connected, so the result contains an
actionable `metadata_object:Document.Sale` candidate. With
`maxCandidates=1`, `limit_candidates()` (`use_case.rs:353-370`) sorts that kind
tag before `method`, drops the actual handler, and `report_status()` may report
Explore `complete` solely because the document owner is actionable. The same
generic rule also promotes an `ExchangePlan` after a complete uses/subscribes
chain, although the actionable hooks are the subscription/handler and the plan
is contextual ownership.

Fix candidate construction from a closed mechanism-role capability matrix
(or the planned `MechanismInstance.entry_candidates`), not from generic graph
connectedness. Add maxCandidates 1 regressions for Document lifecycle and the
complete Exchange subscription chain; owners/plans must remain related and
connected but must not consume candidate slots.

### P1 — a legal 129..=256-gap batch makes report construction fatal

The provider contract accepts up to 256 gaps, but public candidate blockers and
proposal coverage gaps are capped at 128. `ProposalValidator` inserts every
distinct material gap reason (`proposal_validator.rs:187-202`), and candidate
finalization does the same (`use_case.rs:1024-1076`).
`DiscoveryReport::validate()` then rejects either list above 128
(`model.rs:2583-2592`, `model.rs:2610-2612`).

Reproducer: return one valid Bounded material provider outcome with 129 sorted
query-wide gaps named `gap_000` through `gap_128`, plus otherwise positive
existence/runtime/support evidence. `ProviderOutcome::bounded` accepts it, but
`execute()` ends with `DiscoveryError::Operation` saying candidate blockers or
proposal coverage gaps exceed 128. A resource degradation has therefore become
a fatal operation failure.

Introduce a deterministic aggregation/overflow representation before public
projection (with an explicit sentinel that says reasons were compacted), or
align the public bound with the accepted provider bound. Add end-to-end
128/129/256 and reversed-gap-order tests for both Explore candidates and
Validate verdicts.

### P1 — a gap on a known partial Exchange chain yields a false contradiction

Negative runtime coverage checks only the proposal target
(`proposal_validator.rs:152-162`, `proposal_validator.rs:427-436`). Gap
materiality adds actual connection endpoints only after a complete runtime
connection (`proposal_validator.rs:607-680`). A `uses` edge is intentionally
non-runtime, so its known subscription endpoint is absent from both checks.

Reproducer:

1. `MetadataPresent(ExchangePlan.Sync)`;
2. `ExchangePlan.Sync --uses--> EventSubscription.SyncWrite`;
3. the Metadata outcome is Bounded by an exact gap on
   `EventSubscription.SyncWrite` and contains no `subscribes` edge;
4. support for `ExchangePlan.Sync` is known.

The current result is runtime `No` and proposal `Contradicted`: target coverage
looks complete, and the subscription gap is not projected. The evidence only
proves a partial chain whose second edge was not completely inspected, so the
result must be runtime `Unknown`, with the exact gap in verdict/check
diagnostics. This is not closed by merely sharing the current material-subject
map: that map also lacks partial-mechanism endpoints.

Expose potential/partial mechanism dependencies from the graph and use them in
negative completeness plus the single canonical material-subject map. Add
exact-subscription-gap, unrelated-sibling-gap, source-scoped, and permutation
tests.

### P1 — receipt eligibility is fail-open when mutationIntent is absent

The active contract says `mutationIntent` is optional for proposal validation
but required for receipt eligibility. `receipt_eligibility()`
(`use_case.rs:221-269`) checks only Validate mode, supported verdicts, and
material checks, then trusts `ReceiptIssuerPort::assess()`. It never requires
every selected proposal to carry a mutation intent, and the public report no
longer contains enough input to revalidate that invariant.

The existing `AllowReceiptIssuer` demonstrates the failure: `method_proposal()`
has no mutation intent, yet several tests obtain `eligible=true` (for example
`non_material_optional_degradation_is_partial_and_may_keep_eligibility`). The
production no-op issuer currently masks this with
`receipt_store_not_implemented`, but replacing it with the real issuer makes a
non-delegable application invariant depend on one adapter implementation.

Require a valid typed mutation intent for every selected proposal before
calling the issuer and return a stable blocker such as
`mutation_intent_required`. Keep resolver/tool compatibility in the later
typed mutation resolver, but add the missing-intent all-or-nothing regression
at this boundary now.

## Verification

```text
cargo test --locked -p unica-coder application::discovery -- --nocapture
123 passed; 0 failed
```

The green suite has no reproducer for the four cases above.
