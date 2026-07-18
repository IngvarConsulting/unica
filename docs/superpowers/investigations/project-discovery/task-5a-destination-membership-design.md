# Task 5A destination membership contract

Date: 2026-07-17

Status: **accepted design; implementation required before Task 5A may commit**

Base: frozen uncommitted Task 5A slice from `20f6afa7a09430614babebc0cdeebeb94c8a0189`.

## Problem and safety invariant

The current application equates `MetadataPresent` under a destination source
with `ExtensionOwned`. That inference is unsafe. A destination extension may
contain an own object with the same canonical name, or an adopted object whose
`ExtendedConfigurationObject` points to another base UUID. Neither object may
be patched as the requested base object.

The application may return `SupportState::ExtensionOwned` only when every
registered metadata owner required by the proposal (root and, where present,
registered Form) is an adopted destination object whose exact extended-object
UUID equals the exact UUID of the corresponding analysis object.

`CfePatchMethod` is a patch-only intent. It never borrows implicitly. If one or
more required owners are absent from the destination and none are unsafe or
inconclusive, the result is `ExtensionRequired`, carries the blocking reason
`destination_borrow_required`, and is not receipt-eligible. The caller must run
the explicit borrow operation and then repeat discovery.

## Why the join is split into two facts

Membership depends on two independently captured fingerprints:

1. the analysis source proves the canonical UUID of the base object;
2. the destination source proves whether its same-name object is own or
   adopted, and for an adopted object which base UUID it extends.

Do not place the analysis UUID or analysis fingerprint inside a destination
freshness record. That would create a hidden cross-snapshot claim that cannot
be validated against one record's `EvidenceFreshness`. Keep two source-bound
facts and perform one explicit application-layer join over the captured source
pair.

## Closed domain types

The model reuses the domain-owned smart-constructed canonical UUID value:

```rust
struct PlatformUuid(String);
```

It accepts exactly 36 ASCII bytes: hexadecimal digits at every position except
hyphens at byte offsets 8, 13, 18, and 23. Hex input is case-insensitive and is
stored as lowercase. Non-hyphenated, braced, URN, whitespace-padded, Unicode,
and otherwise permissive parser forms are rejected, as is
`00000000-0000-0000-0000-000000000000`. Equality, ordering, serialization, and
digest encoding use only that canonical value.

The destination membership is closed:

```rust
enum CfeMembershipKind {
    Own,
    Adopted {
        extended_configuration_object_uuid: PlatformUuid,
    },
}
```

Append two `ProviderFact` variants without renumbering tags 1 through 9:

```rust
MetadataIdentity {
    subject: ArtifactRef,
    object_uuid: PlatformUuid,
} // stable tag 10

CfeObjectMembership {
    subject: ArtifactRef,
    membership: CfeMembershipKind,
} // stable tag 11
```

Both project to `EvidenceType::Metadata` and both are emitted only by the
MetadataCatalog port. `CfeMembershipKind` has stable tags `Own=1`, `Adopted=2`.
The digest encodes the fact tag, complete typed subject, membership tag, and
canonical UUID where applicable. Add the two fact tags and membership tags to
the exhaustive uniqueness assertions and add collision tests against all
existing fact variants.

The fact constructors accept only a registered metadata root or registered
Form derived from `ArtifactOwnershipChain`. The sole Module-kind root exception
is the self-owned registered `CommonModule.<Name>` artifact for which
`chain.root_owner() == subject`; ObjectModule, ManagerModule, CommandModule,
FormModule, and every other nested Module remain forbidden. They also reject
Method, FormCommand, and arbitrary same-name artifacts. Existing
`MetadataPresent` / `MetadataAbsent` remains the existence polarity; neither new
fact silently implies presence.

Recommended stable fact codes are `metadata_identity`,
`cfe_membership_own`, and `cfe_membership_adopted`.

## Exact query plan

Add an exact pair to the metadata query model:

```rust
struct DestinationMembershipPair {
    analysis: SourceScopedArtifact,
    destination: SourceScopedArtifact,
}
```

Invariants enforced by its constructor before provider I/O:

- analysis and destination refer to the same typed artifact identity;
- analysis source is the captured analysis source;
- analysis source kind is exactly `configuration` and format is Platform XML;
- destination source is the captured mutation source named by
  `CfePatchMethod`;
- destination source kind is exactly `extension` and format is Platform XML;
- the two sources are distinct and both exist in the captured composite
  snapshot;
- the artifact is exactly a registered root (including only the narrow
  self-owned `CommonModule.<Name>` Module root) or registered Form in the
  proposal ownership chain;
- pairs are canonicalized, sorted, and deduplicated;
- at most 32 explicit proposals produce at most 64 pairs (root plus optional
  Form), hence at most 128 source-bound companion keys.

`MetadataCatalogQueryPlan` carries these exact pairs in addition to the normal
existence subjects. It must not reduce them to unqualified destination names.
For each `CfePatchMethod` proposal derive the registered root and optional
registered Form; never query a nested implementation module or the target
method. The self-owned `CommonModule.<Name>` root is the sole Module-kind
exception described above.

Each fact is first validated against its own source snapshot and fingerprint.
Only after freshness/epoch normalization and semantic deduplication does the
application join the analysis and destination halves of a pair.

## Response completeness and provider boundary

For every exact membership pair, the MetadataCatalog response obeys these
rules before evidence limits are applied:

- analysis `MetadataPresent` requires exactly one semantic
  `MetadataIdentity` companion for the same source and artifact;
- analysis `MetadataAbsent` permits no `MetadataIdentity` companion;
- destination `MetadataPresent` requires exactly one semantic
  `CfeObjectMembership` companion for the same source and artifact;
- destination `MetadataAbsent` permits no membership companion;
- location-distinct repetitions of the same semantic value are retained as
  provenance after semantic consistency is proven;
- different analysis UUIDs, `Own` plus `Adopted`, or two distinct adopted
  UUIDs for one exact key are impossible provider output and deterministically
  fail as `ProviderContractViolation`; the application never selects a winner
  or converts them into a nonfatal public conflict;
- a companion without Present, a companion together with Absent, Present
  without its companion in a Complete response, an out-of-plan source or
  artifact, or a wrong fact variant is a provider contract violation;
- in a Bounded response every missing polarity/companion key is covered by an
  exact `Artifacts`, matching `SourceSetWide`, or `QueryWide` gap;
- Complete silence is never converted to destination absence.

Validation order is fixed:

1. port/fact/source admissibility;
2. freshness and captured-source fingerprint validation;
3. epoch normalization and exact duplicate handling;
4. semantic consistency (impossible multi-value keys fail as
   `ProviderContractViolation`) and companion completeness validation;
5. per-port and global evidence limits;
6. application join and projection.

Forged internally inconsistent `Complete` responses fail as
`ProviderContractViolation`. A deterministic document-local semantic parse
failure returns `Bounded` with the narrowest exact source-qualified gap and
preserves fully validated unrelated documents/pairs. A whole-port `Failed` is
reserved for a shared/global parser invariant for which no reliable sub-batch
exists. Exact local reasons include `malformed_cfe_membership`,
`duplicate_cfe_membership_field`, and `invalid_metadata_uuid`; no partial
membership fact from the affected document is emitted.

Completeness is proven against the canonical pre-limit batch. If an evidence
limit subsequently removes a polarity or companion record, the generated
material `evidence_limit` / `global_evidence_limit` gap makes the join Unknown.
Post-limit companion loss is not reclassified as a second provider contract
violation.

## Task 5B Platform XML extraction contract

The Platform XML provider uses one shared schema-aware catalog for analysis and
destination sources.

For each requested analysis root/Form descriptor it emits the descriptor's
canonical direct-object UUID as `MetadataIdentity`:

```text
MetaDataObject/{RootKind}/@uuid
MetaDataObject/Form/@uuid
```

The attribute is exactly one direct, unqualified `uuid` on the already
identity-validated object element. `MetaDataObject/@uuid`, descendant,
namespaced, and case-lookalike attributes never count. It is parsed only through
domain-owned `PlatformUuid`.

For each requested destination root/Form descriptor it classifies only direct
properties of that descriptor:

- absence of both `Properties/ObjectBelonging` and
  `Properties/ExtendedConfigurationObject` is `Own` in the v1 XML encoding;
- `ObjectBelonging=Adopted` plus exactly one valid
  `ExtendedConfigurationObject` UUID is `Adopted { uuid }`;
- Adopted without an extended UUID, an extended UUID without Adopted,
  duplicate direct fields, an unknown belonging value, a nested/sibling value,
  or an invalid/nil UUID is a parse failure with an exact reason.

The local wrapper UUID of the destination descriptor is not the base identity
and must never be compared with the analysis UUID. Only
`ExtendedConfigurationObject` participates in the cross-source join.

The v1 UUID join is valid only when the captured analysis source kind is exactly
`configuration`. General Explore against an extension remains supported, but a
`unica.cfe.patch_method` proposal with analysis kind `extension` is stopped in
the application use case immediately after source resolution/capture and before
membership-query construction or provider I/O:

```text
blocker = cfe_analysis_configuration_required
verdict = Unknown
receipt eligible = false
receipt issuer calls = 0
membership pair count for the affected proposal = 0
```

The blocker is proposal-level, not a global SourceReadiness failure. A future
typed `BaseMetadataIdentity` lattice could support adopted analysis-extension
objects, but v1 must not approximate it from the extension wrapper UUID; Own
analysis-extension objects have no base-configuration identity.

This condition is represented by an application-owned Check, never by the
canonical resolver tuple (general extension analysis is valid):

```text
provider=DiscoveryPreflight
code=mutation_preflight
state=Skipped
outcome=Inconclusive
coverage=Unknown
severity=Blocking
affects=[nonempty sorted unique exact proposal:<id> values]
reasonCode=cfe_analysis_configuration_required
retryable=false
details=[]
evidenceIds=[]
```

Only affected CFE proposals appear in `affects`; unrelated proposals continue
and do not inherit the blocker.

## Application join and projection

Classify each required pair independently:

| Analysis side | Destination side | Pair result | Public blocker |
|---|---|---|---|
| Present + exact identity | Absent | `RequiresBorrow` | `destination_borrow_required` |
| Present + exact identity | Present + Own | `Indeterminate` | `destination_object_not_adopted` |
| Present + exact identity | Present + Adopted with equal UUID | `AlreadyBorrowed` | none |
| Present + exact identity | Present + Adopted with different UUID | `Indeterminate` | `destination_extended_object_mismatch` |
| missing/failed/gapped analysis identity | any | `Indeterminate` | `analysis_metadata_identity_inconclusive` plus exact provider/gap reason |
| valid analysis | missing/failed/gapped destination membership | `Indeterminate` | `destination_membership_inconclusive` plus exact provider/gap reason |

Aggregate root and optional Form with unsafe/inconclusive precedence:

- all `AlreadyBorrowed` => `SupportState::ExtensionOwned`;
- only `AlreadyBorrowed` and `RequiresBorrow`, with at least one
  `RequiresBorrow` => `SupportState::ExtensionRequired` plus
  `destination_borrow_required`;
- any `Indeterminate` => `SupportState::Unknown` plus every exact blocker;
  this takes precedence over any absent pair;
- existing exact `Locked` and `ConfigurationReadOnly` destination support
  states remain blocking public states; the membership lattice is used only
  for a destination support state already classified as safe for extension
  mutation;
- unknown/conflicting support remains Unknown and cannot be rescued by a
  positive membership join.

`ExtensionRequired` is deliberately not `Supported`. It blocks
`receiptEligibility` even when existence and runtime reachability are exact.
Only `ExtensionOwned`, with every other material fact complete and no blocker,
may produce a Supported `CfePatchMethod` verdict and an eligible receipt.

Verdict evidence IDs include, for every required owner, the analysis presence
and identity record IDs, destination presence/absence and membership record
IDs, and the exact source-support records. Evidence remains bound separately to
both source fingerprints. Full ownership-chain provenance is retained, while
sibling root/Form facts are excluded.

## Public reason codes

The minimum closed reason vocabulary added by this contract is:

- `destination_borrow_required`;
- `destination_object_not_adopted`;
- `destination_extended_object_mismatch`;
- `analysis_metadata_identity_inconclusive`;
- `destination_membership_inconclusive`;
- `malformed_cfe_membership`;
- `duplicate_cfe_membership_field`;
- `invalid_metadata_uuid`;
- `cfe_analysis_configuration_required`.

An exact provider gap reason is preserved in verdict coverage gaps and its
Check. Generic inconclusive reasons may accompany it but must not replace it.

## Mandatory RED matrix

1. Root only, destination Adopted with equal UUID => ExtensionOwned and,
   absent other blockers, eligible.
2. Root plus Form, both Adopted with equal corresponding UUIDs =>
   ExtensionOwned.
3. Root absent => ExtensionRequired, `destination_borrow_required`, ineligible.
4. Root adopted/equal but Form absent => ExtensionRequired and ineligible.
5. Same-name destination Own => Unknown,
   `destination_object_not_adopted`, ineligible.
6. Same-name Adopted with another UUID => Unknown,
   `destination_extended_object_mismatch`, ineligible.
7. UUID spelling/case variants normalize to one canonical matching value.
8. Only exact 36-byte hyphenated hex is accepted; uppercase canonicalizes to
   lowercase, while nil, braced, compact, URN, padded, Unicode, or malformed
   input never matches and yields the exact failure reason.
9. Location-distinct identical companions preserve provenance; two different
   UUIDs or Own plus Adopted deterministically cause
   `ProviderContractViolation`, independently of order.
10. Present without its required companion in Complete => contract violation.
11. Absent with a companion => contract violation.
12. Companion for an out-of-plan source/artifact or wrong source half =>
    contract violation.
13. Bounded exact companion gap => Unknown with the exact gap in verdict and
    Check.
14. Pre-limit Complete companions followed by canonical evidence truncation =>
    Unknown through the material evidence-limit gap, not a second contract
    violation.
15. Analysis and destination fingerprint mismatch are tested independently;
    neither half can be promoted under the other's freshness.
16. Two destination snapshots with the same artifact name but different
    membership UUIDs remain independent.
17. Proposal order, provider order, record order, and duplicate-location order
    produce byte-identical reports.
18. Root/Form provenance is included; sibling root/Form evidence is excluded.
19. ExtensionRequired is never receipt-eligible and patch execution never
    invokes borrow implicitly.
20. One Own/mismatched owner plus another absent owner aggregates to Unknown,
    not ExtensionRequired.
21. The 32-proposal/64-pair bound succeeds; one pair over the derived bound is
    rejected before provider I/O.
22. Fact and membership stable tags are exhaustive, unique, append-only, and
    digest-collision tested.
23. Complete destination silence cannot be interpreted as Absent or Adopted.
24. Analysis kind Extension plus CFE patch intent emits the exact
    `DiscoveryPreflight/mutation_preflight` Check, creates no pair, never calls
    the provider/issuer for that join, and leaves unrelated proposals unblocked.
25. `ProjectSourceResolverPort/source_readiness` rejects the CFE preflight
    reason tuple; supported general extension analysis remains unchanged.
26. Self-owned `CommonModule.Name` Module root is accepted as a membership owner;
    ObjectModule/ManagerModule/CommandModule/FormModule and arbitrary Module
    subjects are rejected before provider I/O.
27. UUID only at direct `MetaDataObject/{RootKind|Form}/@uuid` is accepted;
    MetaDataObject/nested/namespaced/case-lookalike UUIDs never match.
28. One malformed destination membership document yields an exact Bounded gap
    with no partial companion while independent valid destination pairs survive,
    stable under reversed order.

## Required task/spec changes

### Task 5A

- add typed UUID/membership facts, stable tags, canonical encoders, query pairs,
  response companion validation, exact join, blockers, materiality,
  provenance, receipt gating, and all application/determinism REDs;
- replace the current name-only `MetadataPresent => ExtensionOwned`
  projection;
- keep both source fingerprints explicit in report evidence;
- enforce the configuration-only CFE membership precondition before query/provider
  I/O and preserve `cfe_analysis_configuration_required` through receipt gating;
- synchronize the active architecture spec, historical execution plan, Task 5A
  report, and product-contract assertions.

### Task 5B

- parse exact root/Form UUID and direct destination belonging fields from
  Platform XML;
- add Own, Adopted/equal, Adopted/mismatch, malformed, duplicate, nested-field,
  root, and Form fixtures;
- emit no partial companion fact after document-local Bounded parse failure and
  retain unrelated valid pairs.

### Task 5C and patch tooling

- join membership before mutation policy and preserve the exact blocker;
- state and enforce the `CfePatchMethod` precondition: every required object is
  already adopted from the exact analysis UUID;
- keep borrow as the separate explicit `unica.cfe.borrow` operation; patch
  tooling must never borrow as a side effect.

### Active documentation

The architecture spec must explicitly state:

- name equality is not destination ownership;
- two independently fresh facts are joined by exact canonical UUID;
- root and registered Form are both required where applicable;
- `ExtensionRequired` carries `destination_borrow_required` and is
  receipt-ineligible;
- patching never borrows implicitly;
- receipt-grade CFE UUID membership joins require a configuration analysis
  source; extension analysis remains advisory-only in v1.
