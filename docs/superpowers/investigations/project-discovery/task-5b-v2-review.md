# Task 5B v2 — adversarial implementation-readiness review

> **RESOLVED HISTORICAL REVIEW.** Its findings were incorporated into
> the later Task 5B contracts. Keep this file as the evidence trail for why those
> boundaries exist; `task-5b-contract.md` v5 is current and its self-audit controls;
> do not use this v2 verdict as the
> current implementation gate.

Date: 2026-07-17

Reviewed checkout: `20f6afa7a09430614babebc0cdeebeb94c8a0189` plus an uncommitted Task 5A worktree.

Reviewed contract digests:

- `task-5b-contract.md`: `3a0d1db4f27ad48471e88ce61dc53e911a73e602a466107770a17944e838e3d5`
- `task-5b-brief.md`: `637588d768bd24d96c1f9852d1e766a848a0a330ac607893c4e9af1ae528e673`
- `task-8-design.md`: `9cd929f65e1bf02b9c700d36b01b24cea3b423d7c1e229832921bfaca2942d50`

## Verdict

**NEEDS-FIX — not implementation-ready.**

The v2 contract resolves most of the original review, but two P0 gates and the
P1 contradictions below still prevent a truthful RED suite and a stable final
provider boundary. This review made no tracked edits and did not run production
code or tests.

## P0

### P0-1 — the required accepted Task 5A base does not exist in this checkout

Evidence:

- `task-5b-brief.md:5-6` forbids implementation on a moving or partially reviewed
  Task 5A diff and requires the exact accepted Task 5A commit SHA.
- `task-5a-destination-membership-design.md:5-7` still says implementation is
  required before Task 5A may commit and names `20f6afa...` as the frozen base.
- Current `HEAD` is still `20f6afa7a09430614babebc0cdeebeb94c8a0189`; the Task 5A
  application/domain/product-contract files are modified but uncommitted.
- `task-5b-contract.md:40-57` says to stop and repair a missing prerequisite
  before infrastructure work.

Impact: no reproducible Task 5B base SHA exists, and provider implementation can
silently encode a still-changing application contract.

Required correction: finish the Task 5A RED/GREEN/review/spec synchronization,
commit it, record that exact SHA in the Task 5B report, then rebase this review's
remaining corrections on that commit before starting Task 5B production code.

### P0-2 — the mandatory permutation assertion is impossible if registration/document order means byte order

Evidence:

- `task-5b-contract.md:942-944` requires provider outcomes, record digests, gaps,
  evidence IDs, and analysis IDs to be byte-identical when registration/document/
  record/gap order is reversed; `:1090-1091` repeats byte identity for reversed
  descriptor order. `task-5b-brief.md:318-320` repeats the blanket requirement.
- The active spec `extension-point-discovery.md:547-553` requires an evidence ID
  to bind the canonical location and source fingerprint and an analysis ID to bind
  linked fingerprints and provider outcome digests.
- The same spec `:746-759` defines the source fingerprint over the complete
  content-addressed manifest.

Reordering registrations inside XML, changing namespace-prefix spelling, or
otherwise permuting document bytes changes the content fingerprint and usually
line/column coordinates. Therefore evidence and analysis IDs must change even if
the semantic fact digest stays equal. Making them equal would weaken the active
freshness/provenance contract.

Required correction: define two distinct assertions:

1. reversing only internal processing/enumeration order over the *same immutable
   bytes, manifest and locations* must make the whole result byte-identical;
2. byte-level XML permutations may preserve typed semantic values/fact digests,
   but must produce the source-bound IDs/fingerprints implied by the new bytes and
   coordinates.

Every RED row must name which axis it permutes. Do not weaken fingerprint or
location binding to satisfy the current wording.

## P1

### P1-1 — Task 4 shared-parser migration and provider-local resource outcomes are mutually underspecified

Evidence:

- `task-5b-contract.md:289-332` requires one parser family and says Task 4 consumes
  its registration/path/identity results.
- `:372-379` requires Task 4 to parse and identity-validate every registered root
  and nested descriptor before admitting its subtree.
- `:814-828` requires streaming depth/node preflight before any DOM.
- `:830-848` and `:891-892` require depth/node overflow in Configuration or a
  descriptor to become provider-local `Bounded(platform_xml_resource_limit)`.
- `:1095-1098` orders the shared preflight/parser migration into Task 4 before
  provider work.
- The live Task 4 path parses Configuration and root descriptors through DOM in
  `platform_xml.rs:19-25,54-69`, called during capture from
  `source_snapshot.rs:508-544`.

If Task 4 calls the shared bounded parser, a depth-129 or node-1,000,001
Configuration/root descriptor fails snapshot capture and the provider RED is
unreachable. If Task 4 bypasses the preflight and builds the DOM, the advertised
pre-allocation memory bound is not shared and capture still allocates the hostile
document first.

Required correction: specify a two-phase API and one outcome policy for each
document class. Either capture-authoritative documents apply the same resource
preflight and overflow is a typed snapshot resource failure (so remove the
provider-Bounded RED for those documents), or Task 4 performs a bounded streaming
envelope/identity projection that can admit a document without allocating its
mechanism view, leaving the later provider view to return Bounded. State exactly
which phase owns Configuration, root/nested descriptor, and Form.xml overflow.

### P1-2 — the live CFE preflight still violates the v2 non-early-return contract

Evidence:

- `task-5b-contract.md:138-145` explicitly says the per-proposal blocker is not
  an early-return report; advisory evidence may still run, and only the affected
  membership pair is removed.
- Current `use_case.rs:107-117` returns immediately when every proposal is blocked,
  then removes blocked proposals wholesale from the provider request.
- `task-5b-brief.md:298-300` says “zero membership/provider I/O”, which is
  ambiguous against the contract: the safe rule is zero invalid membership-join
  I/O for the affected proposal, not necessarily zero advisory provider work.

Impact: an all-blocked request loses the normal advisory report, and a mixed
request loses non-membership discovery inputs attached to the blocked proposal.

Required correction: keep one advisory discovery plan containing the original
request, derive a separate membership-eligible proposal/pair set, never construct
the invalid pair, merge the exact preflight check per affected proposal, and gate
the issuer independently. Rewrite the brief RED wording to “zero membership-pair
I/O for affected proposals”; assert that unrelated/advisory provider calls remain
possible and cannot inherit the blocker.

### P1-3 — missing mandatory registered Form.xml cannot use the promised optional verified-read contract

Evidence:

- `task-5b-contract.md:469-478` and failure row `:882` require absent registered
  analysis/destination `Ext/Form.xml` to produce a local Bounded gap and say this
  is decided through `read_optional_verified`.
- `source_snapshot.rs` domain `:123-136` has `AbsentOptional` tags only for
  ParentConfigurations and EDT markers.
- Live `FilesystemSourceSnapshots::verified_read` at
  `source_snapshot.rs:296-325` returns `None` only for an existing
  `AbsentOptional` manifest entry; an unlisted path is `NotInManifest`.
- Registered subtrees simply return when the `Ext` directory is absent and record
  no tombstone (`source_snapshot.rs:720-730`).
- The active spec `extension-point-discovery.md:746-756` permits
  `absent_optional` only for versioned optional-path registry entries.

Thus a genuinely missing registered Form.xml is indistinguishable at this API
boundary from a provider-probed noncatalog path, which the contract maps to
`platform_xml_snapshot_catalog_mismatch`.

Required correction: introduce an explicit snapshot-bound representation/API for
catalog-derived mandatory-but-absent registered Form material (with its own
versioned manifest tag, fingerprint encoding and verified live-absence race
check), or define a manifest-catalog membership query that authoritatively returns
that state without calling the reader. Do not overload a missing map key or the
existing optional tags. Add present/absent/reappeared/symlink-or-reparse race REDs
for analysis and destination Forms.

### P1-4 — the failure table gives invalid XML two incompatible owners

Evidence:

- `task-5b-contract.md:838-848`, `:885`, and `:898-905` say malformed
  capture-authoritative Configuration/root/nested envelope or identity is rejected
  by Task 4 before provider invocation.
- Failure row `:886` nevertheless maps document-local invalid UTF-8/XML to
  provider-local Bounded.
- Task 4 must parse the entire XML document before it can prove the envelope and
  identity (`platform_xml.rs:19-25,54-85`). An ill-formed root descriptor cannot
  simultaneously be capture-valid “otherwise valid registered material”.

Required correction: restrict provider-local invalid UTF-8/XML to documents that
Task 4 deliberately does not parse semantically (notably Form.xml). For
Configuration/root/nested descriptors, invalid UTF-8 or XML is capture-fatal;
only well-formed, identity-valid documents with malformed mechanism-specific
scalar/cardinality content may become provider-local Bounded. Split the failure
row and add one RED for each document class.

### P1-5 — Task 5B still cannot supply Task 8's complete Form-method negative proof

Evidence:

- `task-5b-contract.md:229-234` scans only registered Form Commands/Actions.
- Its schema at `:432-443` explicitly puts Form/nested Events outside the flow,
  and RED E7 at `:1031-1041` treats Events as decoys.
- The corrected Task 8 contract `task-8-design.md:341-368,432-468` requires one
  complete shared catalog covering form-level events, recursively supported item
  events, and every command Action; `:478-482` requires it for both analysis and
  destination Forms.
- Task 8's mandatory back-propagation row `:2476-2488` explicitly requires this
  change in Task 5B before Task 8 code. The independent review states the same
  missing proof at `task-8-design-review.md:94-120`.

Without this, a FormModule method bound by a form or item event can later be
misclassified as Ordinary and patched through the wrong mechanism.

Required correction: back-propagate a private, complete
`CompleteFormMethodBindingsV1` parser projection into the shared Task 5B parser
now, including the closed supported item registry, exact event/action paths,
recursive edges, handler/callType/action ordinal, completeness audit and
fail-closed unknown nodes. The public Task 5B v1 evidence output may remain limited
to the seven flows, but it must not encode Events as ignorable decoys in the shared
parser contract.

### P1-6 — analysis negative-proof subjects are defined as already registered

Evidence:

- `task-5b-contract.md:203-213` says `analysis_existence_subjects` contains only
  “registered roots” and “registered Forms”, derived by the application before
  provider I/O.
- `:224-227` requires the provider to emit `MetadataAbsent` when one of those
  subjects is missing after the full scan.

The application cannot know observed registration before the provider's
authoritative scan, and a subject proven already registered cannot be absent.
The likely intended meaning is a registration-*addressable* owner shape from the
proposal ownership chain, but the current word is also used elsewhere for an
observed positive fact.

Required correction: define these inputs as source-qualified potential
registration identities whose kind/ownership shape is valid for the Platform XML
catalog, whether present or absent. Constructor validation must not consult or
assume the observed catalog. Reserve “registered” for the provider's positive
scan result. Add absent-root and absent-Form constructor/provider REDs that prove
the query accepts the key before any positive registration exists.

### P1-7 — Task 5B's composite Metadata invocation conflicts with the already designed Task 7 source-scoped orchestration

Evidence:

- `task-5b-contract.md:241-276` defines one Complete Metadata outcome over the
  full analysis scope plus all exact destination pairs and validates both pair
  companions in that invocation.
- `task-7-design.md:763-786` requires exactly one MetadataCatalog invocation per
  captured source set, with QueryWide degradation local to that source invocation;
  it explicitly separates analysis and each destination.
- Task 7 RED `:1343-1354` locks “metadata runs once per captured source”.

The source-scoped destination invocation cannot return the analysis UUID companion
required by Task 5B's pair-complete response, while the composite Task 5B call
cannot give Task 7's per-source QueryWide isolation or invocation accounting.

Required correction: choose and back-propagate one boundary before provider code.
The safer shape is source-scoped Metadata queries/outcomes with an exact analysis
identity subject call and exact destination membership subjects per destination;
the application performs the already specified cross-snapshot join after both
source-scoped outcomes validate. If the composite call is retained, Task 7's
scope/fault/query-digest contracts and corpus REDs must be rewritten explicitly.

## P2

### P2-1 — superseded audit/review files still prescribe opposite outcomes

Evidence:

- `task-5b-preimplementation-audit.md:50` says source mismatch is retryable
  `Unavailable(source_set_mismatch)`, while v2 requires non-retryable
  `ContractViolation(platform_xml_source_mismatch)`.
- That audit `:52` assigns gap overflow the record-limit code
  `platform_xml_result_limit`, while v2 reserves `platform_xml_gap_limit`.
- `task-5b-design-review.md:409-421,526-535` requires missing registered Form
  material to be whole-port Failed, while v2 requires local Bounded.

Required correction: add an explicit superseded-by-v2 banner to both files and a
small outcome-delta table, or move them under a clearly historical filename. Do
not leave mutually exclusive acceptance instructions beside the authoritative
contract.

### P2-2 — two parser edge policies remain implicit

Evidence:

- `task-5b-contract.md:343-345` says comments/PIs supply no text but does not say
  whether direct text/CDATA chunks separated by comments concatenate or make a
  scalar malformed; RED B1 requires comments to be accepted.
- `:445-446` says unknown direct children are ignored unless a closed collection
  forbids them, but does not explicitly mark Configuration `ChildObjects` closed,
  despite the full-registration completeness claim at `:215-222`. The live Task 4
  parser rejects an unknown root registration kind (`platform_xml.rs:29-36`).

Required correction: define one direct scalar text algorithm and explicitly mark
Configuration `ChildObjects` closed/capture-fatal for unknown registration kinds.
Mechanism-specific extensible collections may keep the scoped unsupported/ignore
rules already stated elsewhere.
