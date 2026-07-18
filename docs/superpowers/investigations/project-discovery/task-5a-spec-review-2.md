# Task 5A frozen-slice spec review

Date: 2026-07-17

Base: `20f6afa7a09430614babebc0cdeebeb94c8a0189`

Scope: the stable uncommitted Task 5A application diff only. No tracked file
was edited by this review. Future Platform XML and ParentConfigurations
providers are outside the code-review scope, except where their contract is a
required Task 5A boundary.

Reviewed against the accepted Task 5A preflight/controller corrections,
`.superpowers/sdd/task-5a-brief.md`, the Task 5B audit/contract, and
`spec/architecture/extension-point-discovery.md`.

## Verdict

**NOT READY TO COMMIT.** The frozen slice closes the earlier callback-registry,
candidate-finalization, query-completeness, limit, and deterministic fan-out
holes. It still contains one unsafe destination-ownership inference, four
smaller public projection/materiality defects, one over-conservative runtime
scope, and unsynchronized required documentation.

The blocking destination correction is specified in
`.superpowers/sdd/task-5a-destination-membership-design.md`.

## Frozen-slice defect checklist

### P0 — destination same-name presence is not extension ownership

Current `support_projection()` maps destination `MetadataPresent` for every
registered root/Form directly to `SupportState::ExtensionOwned`. A same-name
own CFE object and an adopted CFE object pointing to another base UUID therefore
pass as the requested borrowed object. This can authorize a patch against the
wrong destination object.

Required correction:

- analysis-source `MetadataIdentity(subject, canonical UUID)`;
- destination-source `CfeObjectMembership(subject, Own | Adopted { extended
  UUID })`;
- exact application join over separately fresh analysis/destination records;
- every required root and registered Form must be Adopted with the matching
  analysis UUID before `ExtensionOwned`;
- Own or mismatched UUID => Unknown with an exact blocker;
- missing destination owner(s), with no unsafe/inconclusive owner =>
  `ExtensionRequired` plus `destination_borrow_required`;
- `ExtensionRequired` is receipt-ineligible;
- `CfePatchMethod` never invokes borrow implicitly.

The complete type, query, response, projection, determinism, provider, and RED
matrix is in the destination-membership design handoff.

### P0 — active architecture and product contract are unsynchronized

The active spec still exposes the old `BindingDetails` table and stale callback
directions. It does not document the accepted typed query completeness,
source-scoped gaps, callback registry/gating, staged support, exact UUID-bound
CFE membership, `destination_borrow_required`, or the no-implicit-borrow rule.
The historical plan and Task 5A implementation report are also not finalized.

Observed product-contract result at the frozen checkpoint:

```text
python3 tests/ci/test_product_contracts.py
14 tests: 13 passed, 1 failed
first mismatch: expected `ValidatedBinding kind`, live spec has `BindingDetails`
```

Task 5A requires the active spec, historical plan/report, and product-contract
assertions to land with the implementation. The gate must be green.

### P1 — zero-record base provider checks lose candidate consumers

In `build_checks`, proposal consumers are selected from exact material ports,
but candidate consumers additionally require intersection between the
candidate's retained evidence IDs and this provider's retained evidence IDs.
Consequently a zero-record Complete/Unavailable Metadata or Support outcome can
be material to a candidate while its base provider Check has `affects=[]`.

The exact `candidate_material_subjects()` helper already excludes optional
ports. Base candidate affects must therefore be derived from a non-empty exact
material subject set, independent of whether the provider retained a record.
Do not change the already-correct conflict/gap exact scoping.

REDs: a connected Method candidate with zero-record Metadata Complete and
Unavailable; a connected support-unknown candidate with zero-record Support
Unavailable; an unrelated port remains absent from affects; reversed input is
byte-identical.

### P1 — callback rejection provenance is not projected into verdicts

The graph's `RuntimeRejection` and its Check correctly aggregate the canonical
callback record plus every canonical/official-alias Definition record
considered for the semantic slot. `relevant_evidence_ids()` independently
selects object-scoped provider records, so:

- an alias-target proposal omits the canonical callback evidence ID;
- a canonical-target proposal omits the considered alias Definition ID.

For every proposal whose target matches a `RuntimeRejection`, union the full
rejection evidence-ID set into `ProposalVerdict.evidenceIds`. Keep the current
sibling exclusion and canonical ordering. Add canonical-target, alias-target,
duplicate-location, and reversed-record REDs.

### P1 — callback rejection reason is absent from verdict diagnostics

The validator consumes `RuntimeRejection.answer` but never projects
`RuntimeRejection.reason_code`. Exact signature mismatch is therefore merely
Contradicted, and an unsupported alias/signature is merely Unknown with the
generic `runtime_reachability_inconclusive`, even though the exact Check knows
the cause.

Preserve the exact rejection reason in the matching verdict's blockers:
`callback_signature_mismatch` for the exact No result and the exact
unsupported-alias/signature reason for Unknown. A generic inconclusive code may
accompany but must not replace it. Add both answer classes and permutations.

### P1 — verdict gap projection recomputes a narrower material scope

The final `ProposalValidation.material_subjects` correctly adds the proposal
target for every potential runtime port when runtime reachability is not Yes.
Earlier, `gap_is_material()` rebuilds only the base root/Form scope and actual
connection endpoints. For a Method with a target-scoped Metadata gap this can
make runtime Unknown and produce a blocking Check, while omitting the exact gap
reason from `ProposalVerdict.coverageGaps`.

Build one canonical material-subject map (registered owners, actual connection
endpoints, and potential-port target when runtime is not Yes) and reuse it for
verdict gaps, Checks, and returned validation. RED: target-scoped Metadata gap
for a Method must appear in the verdict and Check; an unrelated target gap must
not.

### P1 — negative runtime potential ports remain too broad

`runtime_ports_for()` currently distinguishes only FormModule from every other
Method. It assigns MetadataCatalog + CallGraph to all non-form methods, so an
irrelevant Metadata provider can change the result of an ordinary ObjectModule
or non-canonical CommandModule method.

The v1 negative mechanism set must be derived from the closed ownership shape:

- every Method: CallGraph;
- FormModule Method: additionally FormInspection;
- CommonModule Method: additionally MetadataCatalog for event/job bindings;
- HTTPService Module Method: additionally MetadataCatalog for routes;
- exact canonical or official-alias callback name on Document ObjectModule or
  CommonCommand CommandModule: additionally MetadataCatalog;
- ordinary ObjectModule, unsupported ExchangePlan callback, and non-canonical
  CommandModule Method: no MetadataCatalog runtime mechanism;
- FormCommand/declarative targets keep their existing exact mechanisms.

Add ownership-shape tests showing that an unrelated provider state/gap cannot
change the result, while each real mechanism remains material when negative.

## Previously blocking findings confirmed closed

- Callback evidence is now a closed `platform-callbacks/v1` registry with
  semantic slots and exactly four canonical language rows; version and slot are
  digest-bound.
- Provider callback facts must carry the canonical selected row. Only the one
  official opposite-language method is considered as an alias Definition.
- A batch cannot contain two script variants for one `(source, owner, slot)`.
- Document module-default signature is compatible; explicit AtServer is the
  deterministic unsupported-signature Unknown; CommonCommand requires exact
  AtClient.
- Callback positive/negative aggregation retains callback plus every considered
  Definition ID in graph results and Checks.
- Form action identifiers are matched case-insensitively to the handler method
  and canonicalized; Form query subjects are bound to the analysis source.
- Candidate finalization now requires registered root/Form ownership, applies
  exact gaps/conflicts, downgrades Actionable, and retains full-chain provenance
  without sibling leakage.
- Candidate/proposal Metadata materiality now uses registered root/Form owners;
  actual connection endpoints are added only where the port contributes.
- Base and conflict checks no longer rewrite each other's scope and all base,
  conflict, and gap affects deterministically chunk at 128.
- Candidate-limit identity/kind ordering, explicit proposals outside the
  exploratory prefix, preliminary/final omissions, and omitted-candidate
  downgrade are covered.
- Provider-gap overflow uses a deterministic limit sentinel; 256-gap and
  2,000-subject boundaries are covered.
- Typed Metadata/Form response completeness runs after freshness/epoch
  canonicalization and before limits; out-of-plan facts and uncovered missing
  keys are rejected.

## Verification at the frozen checkpoint

```text
cargo test --locked -p unica-coder application::discovery -- --nocapture
123 passed; 0 failed

cargo fmt --all -- --check
passed

git diff --check
passed

python3 tests/ci/test_product_contracts.py
13 passed; 1 failed (required live spec synchronization)
```

These green application tests do not invalidate the defects above: the unsafe
name-only membership has no UUID model yet, and the four P1 projection cases
lack RED coverage.

## Close-out gate

1. Implement the destination-membership design and its complete RED matrix.
2. Fix all five P1 items above with focused REDs.
3. Synchronize the active architecture spec, historical plan/report, and
   product-contract assertions.
4. Run the full locked `unica-coder` suite, fmt, clippy with warnings denied,
   product-contract tests, and `git diff --check`.
5. Re-review the immutable diff from `20f6afa` before the Task 5A commit.

