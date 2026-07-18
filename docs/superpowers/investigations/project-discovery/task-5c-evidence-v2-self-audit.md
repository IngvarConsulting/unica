# Task 5C-Evidence v2 — conditional self-audit

Status: **STOP; draft audit only; no published SHA-256**.

This audit covers only
`.superpowers/sdd/task-5c-evidence-v2-design.md`. It does not audit or confer
authority on the older combined `.superpowers/sdd/task-5c-v2-design.md`, the
immutable v1 Task5C artifacts, or a future mutation addendum.

The design cannot be frozen or independently accepted until the four-document
Task4/Task5B/Task6/Task7 package is atomically co-frozen and Task5B-v7 is
implemented/reviewed and exports the exact neutral catalog plus
`support-state-query/v2` contract required by the design. Consequently this
file deliberately records no design hash, no self-audit hash, and no synthetic
dependency value.

## 1. Artifact and write-scope check

- The design and this audit are new ignored `.superpowers/sdd` files.
- No tracked production/spec/test file was changed by this design task.
- Immutable `.superpowers/sdd/task-5c-support-design.md` and
  `.superpowers/sdd/task-5c-root-prereview-notes.md` were not edited.
- The combined v2 draft remains working history and is explicitly non-
  authoritative.

Verdict: PASS for write scope; this does not clear dependency STOP.

## 2. Dependency-cycle audit

The Evidence graph is:

```text
accepted Task5B v7 -> Task6 implementation

Task4 snapshot boundary + Task5A facts + accepted/implemented Task5B v7 seam
  -> Task5C-Evidence design/implementation/acceptance

{Task4-v7, Task5B-v7, Task6-v2-v7, Task7-v7} owner designs
  -> their self-audits/reviews atomically co-freeze the provider/consumer tuple;
     no Task6/Task7 implementation OID is imported by Evidence

accepted Task6 implementation + accepted Task5C-Evidence implementation
  -> Task7 imports exact TASK5C_EVIDENCE_ACCEPTED_GIT_OID
  -> Task7 Support consumer production implementation
  -> later writer/receipt work
```

Checks:

- Task 7 production is a consumer, never an Evidence prerequisite. The
  co-frozen Task6/Task7 addenda are transitive lineage of the four-document
  package and therefore preexist; Evidence neither imports nor revalidates them
  and adds no direct Task6/Task7-owned gate. No Task6/Task7 implementation, OID
  or integration gates this slice;
- no Task 8/9/10 artifact, writer, WAL, lease, receipt, or implementation is an
  Evidence prerequisite;
- Task 5B acceptance exports future-consumer seams but cannot depend on an
  actual Task5C/Task7/Task8 consumer;
- Task 6 has no Task5C dependency; an obsolete whole-Task5C prerequisite is
  removed rather than renamed to Evidence;
- Task 7, the actual Support consumer, must name only
  `TASK5C_EVIDENCE_ACCEPTED_GIT_OID`, never whole Task 5C or future Mutation;
- the transitional applied support writer is disabled, so splitting Evidence
  before mutation does not leave the unsafe implementation live.

Verdict: PASS in the draft. Freeze must repeat the search after exact Task5B v7
imports.

This dependency paragraph changed during Task5B v7 coordination. The Evidence
design and this conditional audit remain unhashed/unaccepted and must be
re-audited after the v7 and owner-specific addendum hashes are frozen; this text
does not pre-accept Task5C-Evidence.

## 3. Root prereview P0/P1 closure

### 3.1 Writer/WAL dependency

Evidence makes no writer-safety claim and has no writer dependency. Applied
`unica.support.edit` returns `support_edit_atomic_writer_required` with zero
writes in all modes. A separate downstream addendum is required to restore
apply.

Verdict: CLOSED for the Evidence slice.

### 3.2 BOM/global/vendor/duplicate compatibility provenance

- only exact BOM-prefixed, one-vendor, marker-6, fully consumed input is
  accepted by v2 compatibility;
- BOM-less and zero/multi-vendor are Unsupported;
- the three 337-byte tracked copies are explicitly one synthetic parity corpus
  introduced by `9d8fdf90b806a8af0b34d8d632ef4dff669d9260`, not Designer
  exports/donors;
- every admitted document has only
  `AcceptedCurrentProductCompatibilityV2`; the exact corpus digest is not a
  stronger authority grade;
- global/vendor/object flag 1 are labelled compatibility, never export proof;
- vendor flag 1 is configuration-wide read-only and emits no per-object fact;
- identical and conflicting duplicates both reject; no deduplication claim;
- serialized rule order remains semantic because order-insensitivity is not
  proven.

Verdict: CLOSED.

### 3.3 Extension ownership

- `ExtensionWithoutParentConfigurations` never directly proves ownership;
- present extension ownership requires exact catalog flavor plus Own/Adopted;
- planned destination absence uses a distinct query authority and remains
  advice-only;
- ExtensionRequired issues no patch receipt/plan and performs no implicit
  borrow;
- raw/dropped Metadata or Support records and the authority index alone cannot
  project a positive state.

Verdict: CLOSED at design level; downstream Task7 must consume the exported
retained-join seam without weakening it.

### 3.4 Neutral MDClasses/catalog/query authority

- one Task5B-owned `PlatformCatalogContextV1` is built once and carries the
  matching configuration/registered-Form sets plus opaque snapshot-bound
  witness sets; all consumers and query constructors borrow that whole context;
- one context-owned handle/private constructor enforces build provenance;
  config-only/sidecar-only rebuilds and consumer-local descriptor reparse are
  forbidden; the upstream shared catalog preparation may perform its one
  verified descriptor guard/semantic pass;
  pointer/address identity is neither tested nor encoded, and equal immutable
  value movement is not semantic drift;
- Support never calls MetadataCatalog or reparses MDClasses;
- Task5B v7 owns `support-state-query/v2`, its tags/encoder/goldens/bound;
- query subjects bind full AtomicSourceIdentity plus Artifact identity and
  exact catalog/source freshness;
- Task5C defines no local query encoder.

Verdict: architecture CLOSED. The moving v7 draft now supplies the named seam;
exact imports remain OPEN only until four-document co-freeze and Task5B v7
implementation acceptance make those bytes/symbols immutable.

## 4. Synthetic configuration-root-key correction

The extracted design does not require a
`configuration_root_key: PlatformConfigurationObjectKeyV1`. It states:

- source group authority is full AtomicSourceIdentity + catalog/set/fingerprint
  membership;
- per-object Existing subjects use exact object keys;
- configuration root UUID is the separate Task5B v7 catalog-header
  `ConfigurationRootUuidAuthorityV1` (`Known=1`, `Inconclusive=2`) authority;
- the physical ParentConfigurations leaf/tombstone is resolved from the exact
  captured manifest;
- a fabricated Configuration ArtifactRef/root key is rejected by RED/product
  contract.

Verdict: CLOSED in Task5C design. The exact Task5B v7 header field/type remains
an upstream acceptance STOP.

## 5. Query, authority, and lossy-admission audit

- `MAX_SUPPORT_QUERY_SUBJECTS=4096` is owned by Task5B v7 query construction;
- identical duplicates canonicalize; conflicting authority duplicates reject;
- the provider has no local lossy record limit;
- Task5B v7 must export one authority-bound shared Support atomic-group variant;
  Task5C may not use legacy state-only StandaloneFact or a local custom hash;
- source-free semantic authority/group excludes fingerprints, catalog/set,
  object-key/query/composite identities, locations and evidence IDs;
- a separate snapshot authority digest plus upstream query digest binds the
  Support physical record and retained join, preventing Existing/Planned
  substitution without polluting semantic identity;
- Task7 is the first downstream lossy admission consumer;
- the query authority index travels with records but creates no evidence;
- final planned-absence projection requires retained analysis Metadata,
  retained destination Metadata, and retained Support, all freshness-matched;
- either half dropped yields Unknown; no cross-provider atomic-group claim.

Verdict: CLOSED in the exported Evidence contract.

## 6. Parser and bounded-I/O audit

- parser bounds are 64 MiB, 1,000,000 tokens, 200,000 rules, and 4096 quoted
  bytes;
- all arithmetic/capacities are checked and rejection returns no prefix;
- grammar, mirror recognition, count, duplicate, reason precedence, immutable
  flag spans, and semantic/skeleton/content digests are explicit;
- Support uses one optional manifest leaf and max+1 bounded verified read;
- if the accepted Task4 port lacks the max parameter, the smallest backward-
  compatible extension lands inside the Evidence implementation while
  preserving Task4 containment/identity/final-validation semantics;
- provider direct filesystem I/O and snapshot-wide 4-GiB allocation are STOP.

Verdict: CLOSED at design level. Implementation must prove the Task4 extension
by RED/GREEN and review.

## 7. Per-object rule UUID audit

The synthetic parity corpus/current generator is adopted as compatibility for
the configuration-root invariant and Base Own Catalog wrapper UUID rules only.
It is not Designer proof. The design does not silently generalize that mapping
to every root kind or select wrapper versus extended UUID for a present Enabled
extension:

- Task5B v7 Existing query authority carries a typed Known/Inconclusive support-
  rule lookup UUID authority in its digest;
- if v7 owns a real-evidence or explicitly accepted compatibility mapping,
  Support imports it;
- otherwise the provider emits the exact imported `support_rule_uuid_unproven`;
- missing extension policy and configuration-wide read-only remain safe to
  classify because they do not require object UUID lookup.

Verdict: CLOSED without synthetic proof; exact Task5B v7 outcome must be
imported before freeze.

## 8. Live/render/assessment and transitional apply audit

- live read is retained-component/no-follow and typed Missing/Parsed/Rejected/
  IoFailure;
- no unknown state renders writable/free/owned;
- assessment is Safe/Violation/Indeterminate and does not claim serializable
  write authority;
- residual race for legacy guarded operations is documented;
- applied support.edit is fail-closed and its old fs::write/string/path writer
  is removed or unreachable;
- dry-run preview is explicitly non-authoritative and issues no plan/receipt.

Verdict: CLOSED for Evidence scope.

## 9. Structural/static checks already run

At this draft checkpoint:

- design length at this checkpoint: 1,362 lines; line count must be refreshed
  at freeze;
- Markdown fence count: 72, even;
- no unmatched fence was found;
- no active accepted-Task7/Task8 prerequisite was found;
- Task8/9/10 and writer/WAL words occur only in explicit exclusion/cycle tests;
- `unica.support-query/v2` occurs only as a rejected local-encoder example;
- stale phrases occur only in the mandatory stale-search block or explicit
  forbidden assertions;
- design and audit are ignored by the existing `.superpowers/sdd/.gitignore`.

These are draft checks, not final hash evidence.

## 10. Open blockers

The following are hard STOP, not P2 follow-ups:

1. The atomic Task4-v7/Task5B-v7/Task6-v2-v7/Task7-v7 design co-freeze,
   independent reviews, Task5B implementation commit and exact accepted hashes
   do not yet exist.
2. Exact Task5B v7 Rust names, query field order/tags/bytes/goldens, reason
   spellings, configuration-root UUID authority, and extension support lookup
   UUID policy are not yet importable.
3. Exact accepted Task4 and Task5A implementation Git OIDs have not yet been
   recorded in this artifact.
4. No final stale scan, encoder golden execution, dependency-object validation,
   or fresh independent review can occur before blocker 1 closes.

Former blockers for missing `PlannedDestinationAbsentV1`, missing authority-
bound Support group tag 9, semantic/snapshot digest separation and pointer-
identity provenance are conditionally closed in the current moving Task5B-v7
draft. They reopen if any exact co-frozen or implemented symbol/byte differs;
Task5C never fills them locally.

## 11. Conditional verdict

The extracted Evidence design closes the known Task5C prereview P0/P1 logic and
removes the combined artifact-level Task8 cycle. It is **not accepted,
hashable, or implementation-ready** while the Task5B v7 seam is unresolved.

Final self-audit procedure after four-document co-freeze and Task5B-v7
implementation acceptance:

1. import exact names/tags/bytes/reasons and remove every descriptive placeholder;
2. reread Task4/5A/5B v7 in full;
3. rerun dependency-cycle, stale phrase, tag uniqueness, fence, diff, and RED
   ownership scans;
4. execute query/encoder goldens;
5. close this audit with exact design/dependency values and no-open-P0/P1 or
   retain STOP;
6. only then compute final design and self-audit SHA-256 and request a separate
   independent review.
