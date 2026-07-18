# Task 5A final-fix design: candidate roles, bounded diagnostics, partial dependencies, and receipt intent

Date: 2026-07-17

Base under review: `20f6afa7a09430614babebc0cdeebeb94c8a0189` plus the
uncommitted Task 5A application slice.

Scope: exact implementation decisions for four findings in
`.superpowers/sdd/task-5a-code-review.md`. The runtime positive-port/negative-
policy matrix is designed separately in
`.superpowers/sdd/task-5a-runtime-port-audit.md`; this report deliberately does
not replace it with the unsafe narrow `runtime_ports_for()` switch proposed in
the older spec review. No tracked file is edited by this report.

## Verdict

All four findings are valid, but two tempting fixes would create new contract
errors:

1. `connected` must not be renamed or weakened to mean `candidate`; Document
   and ExchangePlan roots must remain connected related artifacts while being
   excluded from candidate enumeration. CommonCommand is the one v1 callback
   owner which is also an accepted candidate kind.
2. Partial mechanism endpoints must not be inserted into
   `connection_endpoints` or `connection_ports`. Doing that would turn an
   incomplete ExchangePlan chain into positive runtime reachability. They need
   a separate negative-completeness dependency map.

The accepted design below keeps graph topology, candidate capability,
positive connection proof, negative completeness, public summary bounds, and
receipt eligibility as separate contracts.

## Source-of-truth contradictions resolved

### ExchangePlan candidate wording

The historical Task 5 preflight says that the complete two-edge chain is
required before an "ExchangePlan candidate/proposal is connected". That phrase
conflates three independent facts:

- the ExchangePlan root is a connected **related artifact** after the exact
  `uses + subscribes` chain;
- an explicit ExchangePlan proposal may consume that runtime fact under the
  current generic proposal-validation contract;
- the ExchangePlan root is **not** an extension-point candidate in v1.

The accepted interpretation is the first two statements only. In the v1
exchange mechanism the editable declarative hook is the exact
EventSubscription and the executable hook is its exact Method. The plan is
contextual ownership/source information. The historical plan and active spec
must replace the ambiguous phrase; it is not a compatibility promise.

This Task 5A fix does not invent a mutation resolver for explicit ExchangePlan
proposals. Target/tool compatibility remains the later typed resolver's job.

### Runtime-port review wording

The old review recommendation to remove MetadataCatalog from ordinary method
negative evaluation is rejected as a complete fix. The accepted Task 5B
contract requires unsupported callback, command, direct ExchangePlan, and BSP
variants to remain `unknown`. Positive connection capability is not negative
authority. This report only adds exact partial dependency subjects; the closed
runtime profile and unsupported-variant policy come from the separate runtime
audit.

## Decision 1: closed candidate-capability matrix

### Invariant

`ArtifactAccumulator.connected` means only that the artifact is an endpoint or
promoted owner in a proven runtime flow. Candidate creation additionally
requires an exact v1 mechanism role. It must never be inferred from generic
incident connectedness.

Add an internal, non-wire `candidate_targets` set while processing typed
runtime evidence, or an equivalent pure `candidate_endpoints_for_v1()` helper.
Candidate construction becomes the intersection of:

1. exact membership in `candidate_targets`;
2. positive kind-specific existence;
3. proven runtime connectedness;
4. the existing support/conflict/finalization policy.

Do not add a serialized enum or stable tag. This is an application policy
derived from already stable typed facts.

### Exact v1 matrix

| Proven typed mechanism | Related/connected endpoints | Candidate endpoints |
| --- | --- | --- |
| resolved `Method --calls--> Method` | both Methods | both Methods, subject to each Method's own positive Definition/owner proof |
| compatible Document PlatformCallback | Document root + exact ObjectModule Method | exact Method only |
| compatible CommonCommand PlatformCallback | CommonCommand root + exact CommandModule Method | CommonCommand + exact Method |
| `EventSubscription --subscribes--> Method` | subscription + Method | EventSubscription + Method |
| `FormCommand --handles--> Method` | command + Method | FormCommand + Method |
| enabled `ScheduledJob --handles--> Method` | job + Method | ScheduledJob + Method |
| `HttpRoute --handles--> Method` | route + Method | HttpRoute + Method |
| `ExchangePlan --uses--> EventSubscription` only | plan + subscription observed, not runtime connected | none from the `uses` edge |
| complete `ExchangePlan --uses--> EventSubscription --subscribes--> Method` | plan + subscription + Method connected | EventSubscription + Method; never ExchangePlan |
| disabled ScheduledJob | job + Method observed, not runtime connected | none |
| `contains` / `defines` | structural endpoints observed | none |

Therefore the exact candidate allowlist in v1 is `Method`, `FormCommand`,
`CommonCommand`, `EventSubscription`, `ScheduledJob`, and `HttpRoute`, and even
those kinds qualify only in the exact roles above. `MetadataObject`,
`MetadataAttribute`, `TabularSection`, `TabularSectionAttribute`, `Module`,
`Form`, `ExchangePlan`, `Report`, and `DataProcessor` are never candidates in
v1.

CommonCommand is intentional in this allowlist even though stable declarative
binding tag 4 remains reserved. Its candidate role comes only from the exact
compatible CommonCommand PlatformCallback + Definition join; an unconditional
CommonCommand Binding remains forbidden. Document callback ownership is
context-only, while CommonCommand represents the command hook as well as
owning its exact handler.

### Required graph behavior

- `add_edge()` may continue to mark both runtime endpoints `connected`.
- It must record candidate roles separately according to the table.
- `promote_complete_subscription_chains()` may mark ExchangePlan connected and
  attach full-chain evidence, but must not add ExchangePlan to
  `candidate_targets`.
- `relatedArtifacts` still reports connected Document and ExchangePlan owners
  at `connected`, never `actionable` solely because a child mechanism is
  actionable. An exact compatible CommonCommand callback may promote the
  CommonCommand itself because it is explicitly in the allowlist.
- Preliminary and final `maxCandidates` limiting operates only after this role
  filter. Context owners can no longer consume support subjects, evidence
  budget, or candidate slots.
- Explicit proposals remain independent of the exploratory candidate prefix;
  the existing proposal-outside-prefix rule remains unchanged.

### Accepted and rejected alternatives

Accepted: accumulate exact entry/change roles from the typed mechanism which
created the edge. This makes future mechanism additions fail closed.

Rejected:

- `positive_existence && connected` for every ArtifactKind: it recreates the
  reviewed defect.
- hiding owner endpoints or not marking them connected: it corrupts the
  related runtime graph and loses provenance.
- making only Methods candidates: it loses the explicitly supported
  declarative CommonCommand, EventSubscription, FormCommand, ScheduledJob, and
  HttpRoute hooks.
- retaining ExchangePlan as a candidate for compatibility with historical
  prose: the prose contradicts the active candidate definition and the exact
  v1 mechanism.
- a public `CandidateCapability` field or score: capability is closed graph
  policy, not new wire data.

## Decision 2: deterministic material-gap compaction at the 128-code boundary

### Invariant

Legal provider degradation must always produce a report. A provider may carry
up to 256 canonical `ProviderGap` values, while the wire contract intentionally
limits each candidate `blockers` and proposal `coverageGaps` list to 128 stable
codes. Exact gap reason, scope, location, affected consumers, and evidence stay
in `checks`; the two bounded lists are summaries.

Introduce the application-owned reserved stable code:

```text
material_gap_reason_overflow
```

`ProviderGap` constructors/validation must reject that exact reason code as
reserved for report projection. This avoids ambiguity between a real provider
reason and the application sentinel.

### Exact compaction algorithm

Collect two canonical `BTreeSet<String>` values separately for each public
consumer:

- `semantic`: non-provider-gap reasons already owned by application policy,
  such as existence/runtime/support inconclusive, support blockers, ownership
  blockers, and conflicts;
- `material_gap_reasons`: exact reason codes of every scoped provider gap that
  intersects that consumer's one canonical material-subject map.

Then project as follows:

```text
available = 128 - semantic.len
if material_gap_reasons.len <= available:
    result = canonical union(semantic, material_gap_reasons)
else:
    require available >= 1
    retained = first (available - 1) values of canonical material_gap_reasons
    result = canonical union(semantic, retained, {material_gap_reason_overflow})
```

On overflow, preserve every application-owned semantic/base diagnostic first,
then retain the largest canonical prefix of exact material-gap reason codes
which still leaves one slot for the overflow sentinel. The exact omitted
reasons remain in one Check per canonical ProviderGap. Prefix selection is
performed only after canonical sort/dedup, so it is independent of provider or
filesystem input order.

The closed current semantic reason set is far below 127. Add a local invariant
that `semantic + sentinel` fits 128 whenever material gaps overflow; exceeding
that bound is a separate application-model defect, not a provider-gap
degradation path. A legal
1..=256-gap batch can therefore never make report construction fatal.

Apply the same helper to:

- candidate blocker projection in `apply_candidate_provider_incompleteness()`;
- proposal `coverage_gaps` projection in `ProposalValidator`.

Do not compact:

- `CollectedProviderOutcome.gaps`;
- provider outcome snapshots used by analysis determinism;
- `Check.reasonCode`, `Check.details`, `Check.affects`, or Check evidence;
- non-gap semantic blockers.

Every canonical ProviderGap still produces its normal exact Check before
report canonicalization. With one affected consumer, 129 distinct gaps mean
129 exact gap Checks, not one synthetic Check. The sentinel never appears as a
Check reason.

### Boundary behavior

| Distinct material gap reasons after dedup | Semantic reasons | Public bounded summary | Exact Checks |
| ---: | ---: | --- | ---: |
| 128 | 0 | all 128 exact codes, no sentinel | 128 |
| 129 | 0 | first 127 canonical exact codes + sentinel | 129 |
| 256 | 0 | first 127 canonical exact codes + sentinel | 256 |
| 126 | 2 | all 128 exact/semantic codes | 126 gap Checks plus normal checks |
| 127 | 2 | 2 semantic codes + first 125 canonical gap codes + sentinel | 127 exact gap Checks plus normal checks |

Duplicate ProviderGaps with the same reason but different scope/location count
as distinct Checks but as one summary reason, which is already the wire-list
semantics.

### Accepted and rejected alternatives

Accepted: preserve base diagnostics, retain the maximum deterministic canonical
gap prefix, add one explicit overflow sentinel, and keep lossless exact Checks.

Rejected:

- increasing public list bounds to 256: this needlessly changes the accepted
  schema and does not address future fan-out.
- rejecting 129..=256 gaps: those batches are legal by provider contract.
- truncating provider gaps before Checks: it destroys scope/location evidence
  and can fail open.
- keeping an input-order prefix: nondeterministic.
- replacing the whole gap summary with only the sentinel: safe but discards
  useful exact codes even though the bounded list has room for them.
- placing scope/location text into the sentinel: bounded summaries contain
  stable codes only; exact diagnostics already have a typed Check location.

## Decision 3: exact partial runtime dependencies for negative completeness

### Separate proven connections from observed dependencies

Keep the existing meanings:

- `connection_ports`: ports that contributed to a proven positive runtime
  connection;
- `connection_endpoints[(target, port)]`: exact endpoints on that proven
  contributing path.

Add:

```text
potential_runtime_endpoints:
    BTreeMap<(ArtifactRef target, EvidencePort port), BTreeSet<ArtifactRef>>
```

This map means: exact artifacts already identified by an observed v1 mechanism
whose coverage is required before the application may conclude that `target`
has no runtime connection. It does not itself prove connection, candidate
capability, or positive port applicability. The separate runtime-mechanism
profile decides which ports/negative policy apply; this map refines the exact
subjects for those ports.

The graph is analysis-source-specific, so the map stores ArtifactRef. Projection
wraps each entry in the captured analysis source-set before comparing it to a
source-scoped gap.

### ExchangePlan population

For every validated observed
`ExchangePlan P --uses--> EventSubscription S` record, add:

```text
potential_runtime_endpoints[(P, MetadataCatalogPort)] += {P, S}
```

Do this even though `uses` is non-runtime. Do not mark either endpoint
connected from `uses` and do not modify `connection_ports`.

If an exact `S --subscribes--> M` edge also exists, the existing complete-chain
promotion records the proven path:

```text
connection_endpoints[(P, MetadataCatalogPort)] += {P, S, M}
```

At runtime `Yes`, materiality uses the proven endpoints only. At runtime not
`Yes`, negative completeness/materiality uses the potential endpoints. Thus a
gap on an incomplete alternative cannot downgrade an independently proven
complete chain, while a gap on the one known partial chain prevents false No.

### Pending callback population

The same abstraction closes the callback-owner hole identified by the runtime
audit. For an observed callback row with owner `O`, canonical method `M`, and
the one registry-owned official opposite alias `A`, record exact dependencies
for each relevant target identity (`O`, `M`, and `A`):

```text
potential_runtime_endpoints[(target, MetadataCatalogPort)] += {O, M}
potential_runtime_endpoints[(target, DefinitionPort)]      += {M, A}
```

The runtime profile decides whether the target is callback-dependent. The
Definition set includes the official alias because a scoped gap can hide that
exact definition; arbitrary sibling method names are not dependencies. Names
must come from the closed callback slot registry, never ASCII/Cyrillic
heuristics.

This does not reclassify DefinitionPort as a positive `connection_port` and
does not settle the separate runtime profile design. It supplies the exact
subjects that profile needs for owner-target negative completeness and scoped
gap diagnostics.

### One canonical negative material-subject path

Replace target-only `port_complete_for_runtime_negative()` with a helper whose
inputs include the graph/runtime profile and which evaluates one exact subject
set:

```text
negative_material_subjects(target, port) =
    port-specific base subjects
    union potential_runtime_endpoints[(target, port)]
```

Coverage for that set is complete only when:

1. the selected provider outcome is `Passed`;
2. no QueryWide gap exists;
3. no SourceSetWide gap names the analysis source;
4. no Artifacts gap intersects an exact source-qualified subject in the set;
5. the runtime profile's negative policy authorizes a conclusive negative.

An unrelated artifact gap or a gap in a mutation source does not make the
analysis dependency incomplete.

Proposal validation must derive and store material subjects before projecting
gap reasons. Reuse the exact same subject set for:

- conclusive runtime-negative evaluation;
- `ProposalValidation.material_subjects`;
- `gap_is_material` / proposal coverage gaps;
- `build_checks` affects and severity.

Do not reconstruct slightly different sets in those four places. Positive
runtime paths continue to use `connection_endpoints`; negative/unknown paths
use `potential_runtime_endpoints` plus port-specific base subjects.

Candidate finalization does not consume potential endpoints because a
candidate requires a proven positive connection. It continues to use exact
proven endpoints.

### Accepted and rejected alternatives

Accepted: a separate exact potential-dependency map, selected only for
negative/unknown runtime reasoning.

Rejected:

- treating `uses` as runtime: false positive reachability.
- inserting partial endpoints into `connection_endpoints`: it makes an
  unproven path look like a contributing positive path.
- checking only the proposal target: it misses the known EventSubscription and
  callback Definition dependencies.
- making every EventSubscription or every method globally material to every
  ExchangePlan/callback owner: sibling gaps would create false unknowns.
- degrading on a different source-set's same-named artifact: gaps are
  source-qualified.
- widening or narrowing generic runtime ports as a substitute: port policy and
  exact dependency subjects are different decisions.

## Decision 4: application-owned fail-closed mutationIntent eligibility

### Invariant

`mutationIntent` remains optional for proposal validation and must not change a
proposal verdict merely by being absent. It is mandatory for receipt
eligibility for every selected proposal. This all-or-nothing invariant belongs
to the use case and must be checked before `ReceiptIssuerPort::assess()`.

Compute independently:

```text
has_selected_proposals = !request.proposals.is_empty()
all_have_mutation_intent = has_selected_proposals
    && request.proposals.iter().all(|p| p.mutation_intent.is_some())
```

When at least one selected proposal lacks the typed intent, add the exact
receipt blocker:

```text
mutation_intent_required
```

and do not call the issuer. Preserve all other applicable blockers in canonical
order:

- `validate_mode_required` when mode is not Validate;
- `proposal_not_supported` when the selected verdict set is empty or not all
  fully Supported;
- `material_check_incomplete` when a blocking material Check remains;
- `mutation_intent_required` when a non-empty selected set has any missing
  intent.

Only if all four application gates pass may the use case call the issuer once
and return its assessment.

Typed target/tool/argument compatibility is deliberately not implemented in
this fix. Deserialization already yields the one closed MutationIntent variant;
the Task 7 resolver must later prove target, tool, normalized arguments,
destination, and allowed artifacts. This fix only closes the missing-intent
fail-open boundary.

Existing tests whose purpose is optional-provider materiality must stop using
`method_proposal()` when they expect receipt eligibility. In particular,
`non_material_optional_degradation_is_partial_and_may_keep_eligibility` must
use a fully supported proposal containing a typed mutation intent; otherwise
it tests the opposite of the accepted receipt contract.

### Accepted and rejected alternatives

Accepted: an application precondition with a stable public blocker and a spy
proving zero issuer calls.

Rejected:

- trusting each ReceiptIssuer adapter: it delegates a non-delegable invariant
  and the production issuer replacement can fail open.
- making mutationIntent required in the request schema: advisory validation of
  other future tools explicitly omits it.
- changing a Supported proposal verdict to Unknown solely because intent is
  absent: proposal evidence and receipt delegation are separate domains.
- allowing a receipt for the subset of proposals that has intent: v1 selection
  is all-or-nothing.
- implementing the full mutation resolver in Task 5A: that would duplicate and
  pre-empt the typed Task 7/8 contract.

## RED matrix

All rows must fail against the reviewed checkpoint before implementation. Each
behavioral row is run with forward and reversed provider/record/gap order where
ordering is applicable; report bytes (or canonical structured value), evidence
IDs, and analysis ID must be identical.

### A. Candidate role matrix

| ID | Evidence | Expected candidates | Expected related-only nodes |
| --- | --- | --- | --- |
| C1 | Document callback + compatible Definition + support, `maxCandidates=1` | exact callback Method | Document root connected, not candidate/actionable |
| C2 | CommonCommand callback + compatible Definition + support | CommonCommand and exact CommandModule Method | its module is related-only |
| C3 | EventSubscription binding + exact handler existence/support | EventSubscription and Method | owning CommonModule remains related |
| C4 | FormCommand binding + exact handler existence/support | FormCommand and Method | Form, owner, module related only |
| C5 | enabled ScheduledJob binding + exact handler existence/support | ScheduledJob and Method | CommonModule related only |
| C6 | disabled ScheduledJob | none from this mechanism | job/handler observed only |
| C7 | HttpRoute binding + exact handler existence/support | HttpRoute and Method | service/module related only |
| C8 | resolved Call with positive existence/support at both endpoints | both Methods | owners/modules related only |
| C9 | ExchangePlan uses-only | no candidate from uses | plan/subscription observed, not connected by uses |
| C10 | complete ExchangePlan chain, `maxCandidates=1` | exact handler Method (kind tag 6 wins canonical limit); plan absent | EventSubscription may be omitted by limit but is candidate-capable; ExchangePlan remains connected related-only |
| C11 | complete ExchangePlan chain without candidate limit | EventSubscription and Method | ExchangePlan connected related-only |
| C12 | structural contains/defines only | none | exact nodes/edges observed |

Add a table/property test over all 15 ArtifactKind tags proving that no kind
outside the six accepted candidate kinds can be promoted by generic
connectedness. Add a forged graph-unit regression with a connected,
positive-existence ExchangePlan and Document root to ensure candidate
construction still excludes them.

### B. Public material-gap boundary

For both an Explore candidate and a Validate proposal with otherwise positive
existence/runtime/support:

| ID | Canonical distinct material gaps | Expected bounded list | Expected gap Checks |
| --- | ---: | --- | ---: |
| G1 | 128 | 128 exact sorted reasons, no sentinel | 128 exact reasons |
| G2 | 129 | first 127 canonical exact codes + `material_gap_reason_overflow` (plus preserved semantic reasons if the available capacity is reduced accordingly) | all 129 exact reasons, no sentinel Check |
| G3 | 256 | first 127 canonical exact codes + sentinel (plus preserved semantic reasons if any) | all 256 exact reasons |
| G4 | 129 reversed | byte-identical to G2 | same exact canonical Checks |
| G5 | 256 reversed | byte-identical to G3 | same exact canonical Checks |
| G6 | 127 gaps + 2 semantic reasons | both semantic reasons + first 125 canonical gap codes + sentinel | all 127 gap reasons |
| G7 | provider tries sentinel as reason | ProviderContractViolation before graph/report | none |

Every G2/G3 execution must return a normal non-fatal report with the consumer
downgraded/ineligible. `DiscoveryReport::validate()` must remain unchanged at
the public 128 bound and pass.

### C. Partial ExchangePlan and callback dependencies

| ID | Partial/proven evidence and gap | Expected runtime/verdict effect |
| --- | --- | --- |
| P1 | P present, `P uses S`, no subscribes, exact analysis-source Metadata gap on S | runtime Unknown; verdict Unknown; exact reason in coverage/check; never Contradicted |
| P2 | same, exact gap on sibling S2 | S remains complete; conclusive No/Contradicted if the separate runtime profile authorizes the exact exchange negative |
| P3 | same, exact same-named S gap in mutation source | no analysis degradation; same result as complete analysis coverage |
| P4 | same, SourceSetWide analysis gap | Unknown with exact reason |
| P5 | same, SourceSetWide mutation gap | no analysis degradation |
| P6 | complete P->S->M chain plus gap on unrelated S2 | runtime Yes and no material coverage gap from S2 |
| P7 | complete P->S->M chain plus gap on contributing S or M | runtime remains Yes; exact material gap blocks Supported/receipt |
| P8 | observed callback O->M, Definition gap on exact M, proposal target O | owner runtime Unknown; exact gap affects owner proposal/check |
| P9 | observed callback O->M, gap on official alias A | Unknown, because A is an exact registry-owned dependency |
| P10 | observed callback O->M, Definition gap on arbitrary sibling Helper | no dependency match; sibling cannot affect O/M |

P2 is conditional on the closed runtime profile's negative policy, not on an
empty/narrow port set. Unsupported mechanism variants remain Unknown as
specified by the runtime audit.

Also assert the graph maps directly:

- uses-only: no connection port/endpoint for P, potential Metadata endpoints
  exactly `{P,S}`;
- complete chain: proven Metadata endpoints for P exactly `{P,S,M}`;
- callback: Definition potential endpoints exactly canonical+official alias,
  without arbitrary siblings.

### D. Receipt mutation-intent gate

Use a counting ReceiptIssuer fake.

| ID | Request/verdict state | Expected eligibility/blockers | Issuer calls |
| --- | --- | --- | ---: |
| R1 | Validate, one fully Supported proposal, no mutationIntent | false; exactly `mutation_intent_required` | 0 |
| R2 | Validate, two fully Supported, one intent missing | false; `mutation_intent_required` | 0 |
| R3 | Validate, all fully Supported, every proposal has typed intent, no material blockers | issuer result honored | 1 |
| R4 | Validate, one unsupported and one missing intent | false; `proposal_not_supported` + `mutation_intent_required` (and material blocker if present) | 0 |
| R5 | Explore with proposals/intents | false; `validate_mode_required` and other independently applicable blockers | 0 |
| R6 | Validate with empty proposal set | false; `proposal_not_supported`, not `mutation_intent_required` | 0 |
| R7 | optional CodeSearch degradation, all material evidence complete, typed intent present | proposal Supported; report may be Partial; issuer called once and eligibility may remain true | 1 |

R7 replaces the logically invalid existing no-intent expectation.

## Implementation order

1. Add all focused REDs above without weakening existing tests.
2. Add the closed candidate-role accumulator/helper and remove generic
   candidate promotion.
3. Add `potential_runtime_endpoints` and populate uses/callback dependencies;
   integrate it with the separate runtime-profile work.
4. Refactor proposal material-subject construction so negative completeness,
   gap projection, and Check affects consume one exact set.
5. Add the reserved sentinel and shared summary compactor; keep exact Checks
   untouched.
6. Add the application mutation-intent gate and issuer-spy tests.
7. Update the active spec, historical implementation plan, Task 5A report, and
   product-contract assertions in the same Task 5A commit.
8. Run focused discovery tests, the full locked crate suite, fmt, clippy with
   warnings denied, product-contract tests, and `git diff --check`.

## Required tracked-spec clauses

The active spec and historical plan must explicitly state all of the following:

1. connected related artifacts are not automatically candidates;
2. the exact six candidate-capable kinds and the mechanism-role table;
3. ExchangePlan is a connected contextual root after a complete chain but is
   never a v1 candidate;
4. provider gaps remain lossless in Checks while bounded consumer summaries
   preserve base diagnostics, retain a canonical gap prefix, and use the
   reserved `material_gap_reason_overflow` sentinel on overflow;
5. partial exact mechanism dependencies participate in negative completeness
   without becoming positive runtime edges;
6. mutationIntent remains optional for proposal validation but every selected
   proposal must carry it before the application calls the receipt issuer;
7. unsupported runtime variants remain Unknown according to the closed runtime
   profile; a narrower positive-port set is not negative proof.

Product-contract tests must reject the old `ExchangePlan/handles` matrix and
the ambiguous historical `ExchangePlan candidate` wording, and must assert the
new candidate/sentinel/receipt clauses rather than merely searching for generic
terms.
