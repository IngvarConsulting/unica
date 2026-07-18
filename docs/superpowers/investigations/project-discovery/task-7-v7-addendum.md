# Task 7 v7 successor addendum — application associations and acyclic delivery

Owner document date: 2026-07-18.

This file defines Task 7 owner semantics and acceptance criteria. It neither
declares nor denies candidate or accepted design state. The sole design-status
authority is the external four-document package ledger
`.superpowers/sdd/task-4-7-v7-design-package-acceptance.md`; absence, presence
or later transition of a ledger row never changes these owner bytes.
Production authorization is separate and exists only when the exact
implementation OIDs required by section 1.2 are present in the implementation
ledger.

This addendum is read together with, and overrides only the conflicting clauses
of, the immutable base design:

```text
task-7-v6-design.md
SHA-256 = b307703e2f825d3218e8acc73d480372114e215593734e78bdf822e0588ddd9e
```

The base file is historical lineage and is not edited. This owner document
contains no current self-hash, peer hash, review hash, candidate label,
accepted-design label or implementation OID value. A branch, `HEAD`, mtime,
dirty-tree digest or an owner/audit hash outside the external package tuple is
not acceptance authority.

> **For agentic workers:** REQUIRED SUB-SKILL: use
> `superpowers:subagent-driven-development` or
> `superpowers:executing-plans`; implement the independently reviewable slices
> below in order and keep every RED before its GREEN.

**Goal:** preserve Task 7 v6's deterministic eight-family discovery while
making provider query/outcome/group/cache identity completely independent of
Request/Proposal/Mechanism association, importing the exact Task 5B v7 and
Task 6 query authorities verbatim, and splitting the Task 7 prerequisite from
the concrete Task 8 integration without a dependency cycle.

**Architecture:** one application-owned `MaterialAssociationMapV2` records why
provider material matters. Providers receive only validated smart queries;
their raw outcomes and semantic groups never carry application conclusion
identity. Task 7 stores provider invocation history, application stage trace,
material association, admission, traversal and final materiality as distinct
typed layers. The Task 7 prerequisite slice is independently implementable
before Task 8; Task 8 later delivers the concrete resolver/issuer integration.
Invocation/cache identity is the typed query digest, but association membership
is proved separately by the exact owner-minted opaque query authority moved
from registered plan to Finished entry; digest equality is never membership
authority.

**Tech Stack:** Rust, the Task 4 v7 snapshot boundary, Task 5B v7
Platform XML/catalog/query contracts, Task 6 snapshot BSL query v3 contracts,
Task5C-Evidence `support-state-query/v2`, the existing discovery application
modules, `sha2`, and stdlib-only product/corpus tests. No new public MCP,
package, skill, parser, filesystem reader, or runtime service is introduced.

## Global constraints

- Code/tests/package metadata remain above active spec, and active spec remains
  above historical plans.
- Every non-conflicting Task 7 v6 mechanism, corpus, traversal, candidate,
  support, proposal and private-composition rule remains mandatory.
- The exact Task 4/Task 5B/Task 6 names below are one cross-owner contract. A
  different spelling or API shape is nonconforming and requires the external
  package protocol in section 14 to restart; compatibility aliases are
  forbidden.
- `Request`, `Proposal`, `Mechanism`, `ConclusionScope`, proposal IDs and
  application stage/depth/round never enter a provider query, provider query
  digest, provider-local semantic group, provider gap, raw provider outcome,
  physical record, provider cache entry, or provider retention order.
- Adding a second proposal for byte-identical provider material may change only
  the normalized request, the application association map, traversal origins/
  application trace, proposal verdicts/final public conclusions and the
  enclosing analysis identity. It changes no provider call count, query byte,
  group byte, raw outcome byte, admission-order byte or retained prefix.
- Task 7 orchestration calls `PlatformCatalogPort` exactly once per captured
  composite snapshot; a recording spy enforces this execution invariant. The
  reusable borrowed API itself permits deterministic repeated builds whose
  values are byte-equal and does not pretend to enforce linear consumption.
  Metadata, Form, BSL and Support query constructors borrow the same
  non-forgeable `PlatformCatalogContextV1`; no consumer reparses or accepts a
  detached half.
- `EvidenceExecutionContext` carries the same exact
  `&PlatformCatalogContextV1`, composite `&SourceSnapshotV2` and injected
  `&dyn SourceSnapshotPort` to imported owner entrypoints. It contains no
  second/hidden reader. Task 7 never resolves or reads registered material;
  each imported provider may use the injected reader only through its
  owner-defined context validation. MetadataCatalog/FormInspection own their
  FormXml reads; Task 6 alone owns the BSL FormModule read path.
- Task 7 production requires the exact accepted Task5C-Evidence implementation
  OID. Task 7 **design co-freeze does not**. Task 6 has no Task5C dependency.
- Task 8 is downstream and cannot gate Task4/Task5B/Task6/Task7 design
  co-freeze or the Task 7 prerequisite implementation. Its concrete
  integration is delivered by Task 8 itself.
- Public `unica.project.discover`, MCP/package registration, persisted receipt
  issuance/store/lease and guard persistence remain outside this Task 7
  implementation boundary. Internal `ReceiptEligibility` assessment through
  the existing `ReceiptIssuerPort` remains Task 7 behavior.
- The fail-closed exact-artifact-spelling authority in section 0.1 replaces
  Task 7 v6 section 7.2's candidate rule that deduplicates a semantic identity
  and keeps its lexically smallest exact spelling. No first-wins,
  lexical-minimum or provider-order choice remains conforming.

### 0.1 Fail-closed exact-artifact-spelling authority

`ExactArtifactSpellingRegistryV1` and `ArtifactIdentityBytesV1` are both owned
and exported by the Task 5A application/domain layer. Task 5B owns their v7
catalog/provider usage contract; Task 5B, Task 6 and Task 7 MUST import the
types and MUST NOT redeclare a second registry or artifact-identity encoder.
This is a production export prerequisite only: it adds no Task 5A
implementation OID, hash or status to the four-owner design-freeze tuple in
section 1.1. Exact
spelling is never added to a provider query, query digest, provider semantic
identity, atomic-group key, physical-record identity, cache key or
admission-order key.

The registry key is exactly
`(AtomicSourceIdentityV2, ArtifactIdentityBytesV1)` and its value is the exact
`(kind stable tag, canonical-ref UTF-8 bytes)` spelling. Normalized-request
`knownArtifacts` and proposal targets, plus otherwise source-free Mechanism and
candidate projections, use the captured Analysis `AtomicSourceIdentityV2`.
Catalog/provider/gap/group/association/traversal material that is already
source-scoped keeps its own validated `AtomicSourceIdentityV2`. No source is
ever inferred from artifact text or copied from an unrelated occurrence.
Within one key, every occurrence in the complete execution MUST carry
byte-identical value bytes; different source identities remain distinct keys.

The Task 5A/domain shared validation/staging API has this normative ownership
and shape. It accepts only a validated typed `ArtifactRef`; callers cannot pass
semantic identity, kind tag or spelling bytes independently. The registry
itself constructs `ArtifactIdentityBytesV1`, the exact `u16` kind stable tag
and canonical-ref bytes from that one value, preventing a forged cross-field
combination. All fields and constructors other than the exact crate-private
empty constructor are private, and the staged delta is non-serde and
non-forgeable:

```rust
pub(crate) struct ExactArtifactSpellingRegistryV1 {
    entries: BTreeMap<(AtomicSourceIdentityV2, ArtifactIdentityBytesV1),
                      ExactArtifactSpellingValueV1>,
}

pub(crate) struct StagedExactArtifactSpellingDeltaV1 {
    // private complete validated additions plus exact baseline authority;
    // no Clone/serde/raw constructor
}

impl ExactArtifactSpellingRegistryV1 {
    pub(crate) fn empty_v1() -> Self;

    pub(crate) fn validate_occurrence(
        &mut self,
        source: &AtomicSourceIdentityV2,
        artifact: &ArtifactRef,
    ) -> Result<(), ExactArtifactSpellingViolationV1>;

    pub(crate) fn require_occurrence(
        &self,
        source: &AtomicSourceIdentityV2,
        artifact: &ArtifactRef,
    ) -> Result<(), ExactArtifactSpellingViolationV1>;

    pub(crate) fn stage_occurrences_v1<'a, I>(
        &self,
        occurrences: I,
    ) -> Result<StagedExactArtifactSpellingDeltaV1,
                ExactArtifactSpellingViolationV1>
    where
        I: IntoIterator<Item = (&'a AtomicSourceIdentityV2, &'a ArtifactRef)>;

    pub(crate) fn require_staged_occurrence_v1(
        &self,
        delta: &StagedExactArtifactSpellingDeltaV1,
        source: &AtomicSourceIdentityV2,
        artifact: &ArtifactRef,
    ) -> Result<(), ExactArtifactSpellingViolationV1>;

    pub(crate) fn commit_staged_v1(
        &mut self,
        delta: StagedExactArtifactSpellingDeltaV1,
    ) -> Result<(), ExactArtifactSpellingViolationV1>;
}
```

`validate_occurrence` inserts an absent exact source/semantic key or confirms
its byte-identical value; it rejects a different exact value. The read-only
`require_occurrence` succeeds only when that exact key/value is already present
and rejects both missing and different values without mutation. Both derive all
identity/value fields from the same validated `ArtifactRef`.

`require_staged_occurrence_v1` is also read-only. It first verifies that the
opaque delta belongs to this exact baseline, then requires the supplied exact
occurrence in the union of baseline entries and validated staged additions.
It exposes neither half of that union, cannot add an occurrence and returns
`StaleStagedBaseline` for a foreign delta. It exists solely so owner response
validation can prove that every post-staging derived group/material artifact is
an exact byte-identical clone of a raw/query occurrence before final commit.

Staging checks both collisions internal to the unsorted occurrence stream and
collisions with the baseline, but mutates neither. `commit_staged_v1` consumes
the delta, verifies its private exact baseline authority before mutation, and
either installs all additions or changes no entry. The typed invocation owns
the only mutable registry borrow from staging through terminal commit, so its
successful commit cannot become stale; any impossible stale-delta result
invalidates the execution before either registry or invocation entry changes.
Only the owning application transaction may consume a staged delta. Task 5B
Metadata/Form classification
and Task 6 BSL classification MUST invoke this same Task 5A/domain primitive
against their complete raw, pre-classification occurrence streams before their
own sort, set insertion, deduplication, semantic grouping or record/file
ceiling. Their provider-local baseline is empty and the validated delta is not
provider identity. This catches response-internal aliases before either owner
can select a spelling; Task 7 independently stages the returned response
against the execution-wide baseline to catch request/catalog and
cross-invocation conflicts. Neither use changes valid query bytes, provider
goldens, group bytes, admission bytes or cache keys.

The shared Task5A violation is closed and non-serde:

```rust
pub(crate) enum ExactArtifactSpellingViolationV1 {
    InvalidArtifact,
    Collision,
    MissingCommittedOccurrence,
    StaleStagedBaseline,
}
```

Task7 maps it only to the nonretryable closed
`DiscoveryError::ArtifactSpellingInvariant { reason }`: `InvalidArtifact` ->
`artifact_spelling_invalid_artifact`, `Collision` ->
`exact_artifact_spelling_collision`, `MissingCommittedOccurrence` ->
`artifact_spelling_registry_missing`, and `StaleStagedBaseline` ->
`artifact_spelling_registry_stale`. No raw source, spelling, key or registry
state is exposed. Task5B's catalog/provider-local mappings remain its own
frozen owner contract and do not re-enter this Task7 mapping.

Every Task7 path that can map this violation owns `&mut
ProviderInvocationRegistryV2`, sets `execution_invalidated=true` before
returning the mapped error, and performs no later state transition. This
includes request/catalog commit, application admission, all typed-query owner
rechecks, both association owner rechecks and provider-response stage/commit.
No read-only Task7 mapping wrapper exists.

Error priority is closed. Existing owner-defined constant-time raw cardinality
and basic typed-input validation run first. For an admitted occurrence/vector,
exact-spelling validation then precedes every per-element semantic comparison,
downstream consumer identity encoding, sort, insertion, deduplication, grouping
and resource ceiling. The registry's own atomic derivation of
`ArtifactIdentityBytesV1` and exact value bytes from the same validated
`ArtifactRef` is the required internal validation step, not downstream
canonicalization. A spelling failure invalidates the execution before later application
resource/gap logic. The inline application gate adds no batch scan or raw cap,
so this priority does not change the 4,097-identical-Support case.

The application applies that one execution-wide registry before each affected
canonical sort, set insertion, deduplication, grouping or graph construction
to every occurrence reachable from:

- the normalized request's known artifacts and proposal targets;
- both validated Platform catalog sets and their registered material;
- every returned provider record, raw gap, semantic/atomic group and nested
  material subject, including bindings, CFE observations, mechanism facts and
  support material;
- the application association map, traversal roots/edges, mechanism instances
  and candidate material.

The `ProviderInvocationRegistryV2` owns the one execution-wide spelling
registry. After the one Platform catalog result, the generative constructor
stages normalized-request occurrences plus both validated catalog sets and
commits the complete delta atomically before exposing its execution closure. A
request-internal or request/catalog collision may therefore occur after that
single context build but always before registration or downstream evidence-
provider I/O; no partial registry or later commit gate is observable. Every typed invoke builds a
private staged delta from the complete raw returned response after only
constant-time raw-cardinality/basic terminal-envelope checks and before any
Task 7 per-element response/completeness validation, sort, dedup, group,
admission or cache operation. The owner response validator receives the opaque
delta authority and must require every derived artifact before its first
semantic use; it cannot invent or reparse an `ArtifactRef`. Only after complete
response validation succeeds does the invoke atomically commit that unchanged
delta together with the collected raw outcome and exact entry's `Staged ->
Finished` transition. A collision transitions the entry to
`Invalidated`, invalidates the execution without committing any delta/outcome,
issues no `RecordedInvocationIdV2`, and leaves no finishable or accepted raw
snapshot, association map, report or receipt. The same transaction applies to
an adapter response materialized from cache and to every cross-port or repeated
invocation. In-execution cache reuse stores only an already validated recorded
ID and therefore cannot bypass the committed registry. Reversing any input,
catalog, live response, cache or traversal order cannot change rejection.

Only after this uniqueness proof may canonical order compare semantic identity
and exact bytes; equal semantic identities can then have only byte-identical
exact bytes. This validation-only rule changes no valid Task 5B/Task 6/Task 7
provider query bytes, published golden digest, provider call count, raw outcome
bytes or retained prefix.

The Task5A-owned shared response types provide the sealed zero-I/O operations;
Task7 never reconstructs an occurrence list:

```rust
impl ProviderOutcome<EvidenceRecord> {
    pub(crate) fn stage_complete_raw_artifact_spellings_v1(
        &self,
        baseline: &ExactArtifactSpellingRegistryV1,
    ) -> Result<StagedExactArtifactSpellingDeltaV1,
                ExactArtifactSpellingViolationV1>;
}

impl CollectedProviderOutcome {
    pub(crate) fn validate_staged_artifact_spellings_v1(
        &self,
        baseline: &ExactArtifactSpellingRegistryV1,
        delta: &StagedExactArtifactSpellingDeltaV1,
    ) -> Result<(), ExactArtifactSpellingViolationV1>;
}
```

The raw method exhaustively destructures the terminal variant and its private
batch without `..`, visiting every record, raw gap, fact field and nested raw
material occurrence in provider order. `Unavailable`, `Failed` and
`ContractViolation` carry no artifact. It performs no completeness check,
semantic classification, sort, deduplication, grouping or resource limiting.
The collected method exhaustively checks every retained record/gap and every
derived semantic/atomic-group or material artifact through
`baseline.require_staged_occurrence_v1(delta, ...)`; imported Task5B-private
group/material members delegate to their owner-provided staged rechecks.

Collection is artifact-conservative by type: every derived `ArtifactRef` must
be a byte-identical typed clone of a query member already in the baseline or a
raw response occurrence already in the delta. No collection/completeness API
accepts a string, parser, formatter or unchecked artifact constructor. Its
owner-specific entrypoint borrows the delta and requires each derived artifact
before its first semantic comparison. The final exhaustive collected check is
mandatory before commit and proves that no owner branch omitted that ingress
check. A missing/different occurrence invalidates the execution; the delta and
collected outcome are discarded.

Task 7 v6 acceptance row 24 remains normative only as an isolated-execution
metamorphic: each case-equivalent spelling variant run alone produces the same
query/group/admission bytes. It does not permit byte-different spellings for
one semantic artifact to coexist under the same source in one execution. Only
that concurrent same-source conflict rejects; byte-identical repeats and equal
semantic identities under different `AtomicSourceIdentityV2` keys remain
valid.

Permanent REDs cover both input orders for request-known/proposal,
request/catalog, record/record, record/gap, nested binding/material and
mechanism facts; Task 5B and Task 6 raw pre-classification streams; live
response against prior material; two different provider invocations; and
cached replay against live execution state. They also prove both isolated v6
row-24 variants remain byte-equivalent, while putting the two variants together
under one source rejects before an accepted prefix. An implementation that
retains Task 7 v6 section 7.2's lexical-minimum behavior, validates only a
public candidate projection, or validates only after canonicalization is a
hard STOP.

### 0.2 Registry-owned admission, finality and analysis identity

This section is the final ownership correction and supersedes every later
signature or sentence that lets orchestration supply any of the following:

- a retained-record vector as an accepted final prefix;
- an admission reason/port/owner/root/scope envelope;
- an `EvidenceAdmissionGapV3`, `EffectiveProviderInvocationV3` or
  `EffectiveGapV3` vector;
- an `EvidenceGapLimit` option or overflow decision;
- a composite snapshot ID, either catalog-set digest, traversal vector,
  normalized-request (including limits)/source/contract-registry identity
  fragment, or
  other analysis-ID prefix after registry construction; or
- a caller-owned byte buffer into which an opaque invocation key, association
  root or model DTO writes canonical identity bytes.

Those values are related state, not independent DTO inputs. One generative
`ProviderInvocationRegistryV2<'execution, 'context>` owns the complete relation
from the validated run inputs through final analysis identity. Its constructor
consumes one `PreparedProviderDiscoveryV4`, retains one same-`'context` borrow
of the checked `EvidenceExecutionContext` containing the complete captured
`SourceSnapshotV2`, once-built `PlatformCatalogContextV1`, workspace and
reader, and first proves that the token's exact composite snapshot ID and
diagnostic epoch equal that bundled snapshot. It then destructures the token;
the token's normalized request, source and preflight blockers become the only
registry-owned copies. The `EarlyTerminalPreparedDiscoveryV4` variant cannot
cross this type boundary. Before moving the registry by value into the
generative execution closure it:

1. obtains Task4's `SourceSnapshotDiscoveryProjectionV1` and converts it once
   into `CanonicalDiscoverySourceV4`; obtains Task5B's opaque
   `PlatformCatalogExecutionBindingV1`, validates it against the same bundled
   context/snapshot, and moves both into a private non-Clone
   `AnalysisIdentityPrefixV4` together with the exact normalized request
   (including limits) and contract registry;
2. leaves no second binding field, getter or detached component in the registry
   or any later registry projection; the prefix is the sole analysis-header
   binding field (query authorities intentionally own sealed copies) and writes
   its 96 bytes exactly once at the execution-snapshot header position;
3. stages and commits the complete request plus catalog spelling projection as
   one fail-closed initialization; and
4. reads
   `analysis_identity_prefix.validated_request_v4().limits.max_evidence`
   directly in the one admission algorithm; no detached request or scalar copy
   can drift from the encoded request.

There is no later `commit_platform_catalog_spellings`, detached binding setter
or analysis-prefix argument. Every Task5B/Task6 query-association authority
privately carries the same execution binding. One private closed dispatch
performs its fourth operation: Metadata/Form/Support call their owner API with
the bundled exact `PlatformCatalogContextV1` plus `SourceSnapshotV2`, while
CodeSearch/Definition/CallGraph compare against the prefix-owned binding through
their owner API. The dispatch never returns a binding or component.
Registration validates it before duplicate lookup/slot allocation, every invoke
revalidates it together with its `EvidenceExecutionContext` before I/O, and
final root replay repeats the same variant-exhaustive check. All six use the
single shared three-variant `ProviderQueryAssociationViolationV1`.
Binding state changes no provider query/cache/group/raw/effective identity; it
only prevents values from two otherwise valid executions being spliced.

The application fixed-point may request a read-only admission preview while
the registry still accepts new typed invocations:

```rust
pub(super) struct ApplicationAdmissionPreviewV3<
    'lookup,
    'execution,
    'context,
> {
    registry: &'lookup ProviderInvocationRegistryV2<'execution, 'context>,
    retained_by_slot: Vec<RegistryRetainedApplicationEvidenceV3>,
    // owned canonical index state plus one borrowed registry; no key, gap,
    // identity bytes, mutation, accepted DTO or validation authority can escape
}

pub(super) struct RetainedApplicationEvidenceViewV3<'lookup> {
    all_semantic_groups: &'lookup [SemanticAtomicEvidenceGroupV2],
    retained_group_indices: &'lookup [u32],
    all_physical_records: &'lookup [EvidenceRecord],
    retained_record_indices: &'lookup [u32],
    // full owner slices plus checked canonical selections; never fabricated
    // contiguous subset slices; no registry key/authority/gap DTO
}

struct RegistryRetainedApplicationEvidenceV3 {
    slot_id: ProviderInvocationRegistrySlotIdV2,
    retained_group_indices: Vec<u32>,
    retained_record_indices: Vec<u32>,
    // checked canonical indices into one current Finished entry
}

impl RetainedApplicationEvidenceViewV3<'_> {
    pub(super) fn visit_semantic_groups_v3(
        &self,
        visit: impl FnMut(&SemanticAtomicEvidenceGroupV2)
            -> Result<(), DiscoveryError>,
    ) -> Result<(), DiscoveryError>;

    pub(super) fn visit_physical_records_v3(
        &self,
        visit: impl FnMut(&EvidenceRecord) -> Result<(), DiscoveryError>,
    ) -> Result<(), DiscoveryError>;
}

impl<'lookup, 'execution, 'context>
    ApplicationAdmissionPreviewV3<'lookup, 'execution, 'context>
{
    pub(super) fn with_retained_evidence_for_v3<T: 'static>(
        &self,
        invocation: &RecordedInvocationIdV2<'execution>,
        read: impl FnOnce(RetainedApplicationEvidenceViewV3<'_>)
            -> Result<T, DiscoveryError>,
    ) -> Result<T, DiscoveryError>;
}

impl<'execution, 'context>
    ProviderInvocationRegistryV2<'execution, 'context>
{
    pub(super) fn preview_application_admission_v3<'lookup>(
        &'lookup self,
    ) -> Result<
        ApplicationAdmissionPreviewV3<'lookup, 'execution, 'context>,
                DiscoveryError>;
}
```

`RetainedApplicationEvidenceViewV3` has only the two sealed visitor operations.
Each visitor walks its retained-index slice in canonical order, converts every
`u32` with checked `usize::try_from`, resolves it with `slice.get`, and rejects
an invalid, duplicate or non-increasing index before invoking the callback.
This is deliberately not a pair of retained subset slices: a retained prefix
across atomic groups may correspond to non-contiguous physical records, and
Rust cannot safely manufacture such a slice. Neither full slice, index slice
nor an element reference may be returned as `T` because `T: 'static`.
Application code may copy only its normal owned graph/mechanism/query work
values. The static call-site whitelist contains only the retained-graph
fixed-point code in `use_case.rs`. The preview borrow must end before any
mutable registry operation.

The preview and final path call one private pure
`derive_application_admission_v3` implementation. It revalidates all current
raw atomic groups, applies the exact v6 per-port prefix-stop and then the one
six-port global prefix-stop using the prefix-owned validated
`DiscoverRequest::limits.max_evidence`, and derives
the retained physical-record subsequence for every invocation. Preview owns
that derived checked-index vector (it never borrows a temporary) while its
short registry borrow resolves each visit. A preview is
discarded before another mutable registry operation and is never receipt,
snapshot or negative-proof authority.

After the support fixed point, orchestration calls the consuming-state
transition below. It recomputes, rather than trusts, the same decision and
disables every further register/invoke/stage operation:

```rust
pub(super) struct PreparedDiscoveryFinalityV4<'execution> {
    execution_nonce: ProviderInvocationExecutionNonceV2,
    finality_generation: NonZeroU32,
    _brand: PhantomData<fn(&'execution ()) -> &'execution ()>,
    // private state-transition token only; all retained decisions, admission
    // buckets, provider classes and requirements remain registry-owned
    // no Clone/serde/Debug/Display/field, key, row or byte projection
}

impl<'execution, 'context>
    ProviderInvocationRegistryV2<'execution, 'context>
{
    pub(super) fn prepare_discovery_finality_v4(
        &mut self,
    ) -> Result<PreparedDiscoveryFinalityV4<'execution>, DiscoveryError>;
}
```

Each occupied admission bucket aggregates the complete dropped tail for one
exact `(PerPortEvidenceLimit | GlobalEvidenceLimit, EvidencePort)` pair across
all invocations of that port. The registry derives owners and AtomicGroup roots
only from current returned groups. Task5B's exhaustive
`&[&SemanticAtomicEvidenceGroupV2] -> ProviderMaterialArtifactSetV2`
projection derives the nonempty canonical union of real
`SourceScopedArtifact` values, including both real halves of a pair. Prepared
state stores checked `(slot_id, group_index)` coordinates into the full groups
owned by current Finished entries; it never attempts material projection from
the compact `SemanticAtomicGroupIdV2`. Therefore PerPort/Global admission is
always exact
`Artifacts(ProviderMaterialArtifactSetV2)`; callers cannot select `Artifacts`,
`SourceSetWide`, `QueryWide`, split one bucket, omit a group, relabel a drop or
keep any physical record from a dropped atomic group. The finalizer requires
the derived retained vector to equal raw records minus exactly those complete
buckets for every owner.

Provider raw gaps are similarly converted to the complete unique set of
location-erased effective classes inside the prepared finality. Orchestration
may only ask for the next owned, execution-branded association projection; it
cannot supply an index or count. That token contains exact opaque association
instructions; `association.rs` consumes it transactionally into its one sealed
sink and returns an opaque applied token, then the registry consumes that token
and records that exact finality requirement.
The same flow is used for admission buckets. No method returns an
`EffectiveGapV3` or
accepts a reason, port, owner, scope, root vector, retained vector or digest.
Missing, duplicate, foreign, reordered or unconsumed finality projections make
finish impossible.

Projection issuance is itself stateful. Prepared state stores private
`next_projection_issuance: u32` and
`outstanding_requirement: Option<(u32, NonZeroU32)>`. Project-next takes
`&mut self`, checked-increments a fresh nonzero issuance, stores the exact
`(requirement_index, issuance)` before returning an owned projection, and
rejects/invalidates a second project request while it is `Some`. Recording the
matching applied token alone clears the pair and advances
`next_requirement_index`; `None` is possible only when the pair is empty and
the complete requirement vector was recorded. Issuance overflow invalidates
the execution. If the transactional sink fails, the consumed projection leaves
the outstanding pair stranded and finish rejects, so catching the error cannot
retry, replay, skip or silently accept an unapplied requirement.

```rust
pub(super) struct ValidatedFinalityAssociationProjectionV4<'execution> {
    execution_nonce: ProviderInvocationExecutionNonceV2,
    finality_generation: NonZeroU32,
    requirement_index: u32,
    projection_issuance: NonZeroU32,
    instructions: Vec<FinalityAssociationInstructionV4>,
    _brand: PhantomData<fn(&'execution ()) -> &'execution ()>,
    // private; owned, non-Clone/non-serde/non-Debug/non-Display
}

pub(super) struct AppliedFinalityAssociationProjectionV4<'execution> {
    execution_nonce: ProviderInvocationExecutionNonceV2,
    finality_generation: NonZeroU32,
    requirement_index: u32,
    projection_issuance: NonZeroU32,
    _brand: PhantomData<fn(&'execution ()) -> &'execution ()>,
    // minted only after the association transaction commits
}

#[derive(Clone)]
enum FinalityAssociationInstructionV4 {
    RequirePresent {
        root: MaterialAssociationRootV2,
    },
    InheritScopes {
        source: MaterialAssociationRootV2,
        targets: Vec<MaterialAssociationRootV2>,
    },
}

pub(super) trait FinalityAssociationTransactionV4 {
    fn require_present_v4(
        &mut self,
        root: &MaterialAssociationRootV2,
    ) -> Result<(), DiscoveryError>;

    fn inherit_scopes_v4(
        &mut self,
        source: &MaterialAssociationRootV2,
        targets: &[MaterialAssociationRootV2],
    ) -> Result<(), DiscoveryError>;
}

pub(super) trait FinalityAssociationSinkV4 {
    fn transact_finality_associations_v4(
        &mut self,
        apply: impl FnOnce(
            &mut dyn FinalityAssociationTransactionV4,
        ) -> Result<(), DiscoveryError>,
    ) -> Result<(), DiscoveryError>;
}

impl<'execution> ValidatedFinalityAssociationProjectionV4<'execution> {
    pub(super) fn apply_finality_associations_v4(
        self,
        sink: &mut impl FinalityAssociationSinkV4,
    ) -> Result<AppliedFinalityAssociationProjectionV4<'execution>,
                DiscoveryError>;
}

impl<'execution, 'context>
    ProviderInvocationRegistryV2<'execution, 'context>
{
    pub(super) fn project_next_finality_association_v4(
        &mut self,
        prepared: &PreparedDiscoveryFinalityV4<'execution>,
    ) -> Result<Option<ValidatedFinalityAssociationProjectionV4<'execution>>,
                DiscoveryError>;

    pub(super) fn record_finality_association_v4(
        &mut self,
        prepared: &PreparedDiscoveryFinalityV4<'execution>,
        applied: AppliedFinalityAssociationProjectionV4<'execution>,
    ) -> Result<(), DiscoveryError>;
}
```

`apply_finality_associations_v4` is the projection's sole consuming operation and
is callable only from `association.rs` with its exact
`MaterialAssociationMapBuilderV2` sink implementation. It calls
`transact_finality_associations_v4` exactly once; only inside the supplied
closure does `ports.rs` walk the complete instruction vector and invoke the
transaction operations. The sole sink implementation clones/stages the full
map, exposes only that staging value as the transaction object, commits it only
after the closure returns `Ok(())`, and discards it byte-for-byte on either an
operation error or closure error. The applied token is minted only after the
outer transaction method returns `Ok(())`; a sink cannot acknowledge a partial
prefix. The instruction enum is module-private and is never matched outside
`ports.rs`. `RequirePresent`
requires an already nonempty-scoped root. `InheritScopes` copies the exact
nonempty scope set of one validated source root to each validated target; it is
the only path by which provider Artifacts material roots inherit their exact
SourceGroup scopes. The sink returns no scope/root vector. The applied token
has no constructor outside the successful projection method and carries the
exact fresh issuance. The registry consume call is callable only from
`use_case.rs` after the sink borrow ends. `None` is returned only when every
ordered requirement has been applied and recorded. Finish
replays every instruction and independently requires its exact source/targets
and resulting nonempty scope relation in the final map.

The only final boundary is:

```rust
pub(crate) struct ReceiptIssuanceRequest<'a> {
    pub(crate) proposals: &'a [Proposal],
    pub(crate) snapshot: &'a SourceSnapshotV2,
}

pub(crate) trait ReceiptIssuerPort {
    fn assess(
        &self,
        request: &ReceiptIssuanceRequest<'_>,
    ) -> Result<ReceiptEligibility, DiscoveryError>;
}

pub(super) struct UnsealedDiscoveryMaterialityPlanV4 {
    inputs: DiscoveryMaterialityInputsV4,
    // fields/private storage owned by ports.rs; construction only through the
    // checked factories below, statically called from projection.rs
}

struct DiscoveryMaterialityInputsV4 {
    related_artifacts: Vec<RelatedArtifact>,
    flow_edges: Vec<FlowEdge>,
    retained_evidence: Vec<Evidence>,
    candidate_inputs: Vec<CandidateMaterialityInputV4>,
    proposal_inputs: Vec<ProposalMaterialityInputV4>,
}

pub(super) struct CandidateMaterialityInputV4 {
    draft: Candidate,
    supporting_roots: Vec<MaterialAssociationRootV2>,
    negative_proof_roots: Vec<MaterialAssociationRootV2>,
}

pub(super) struct ProposalMaterialityInputV4 {
    proposal: NormalizedProposalIdV1,
    required_roots: Vec<MaterialAssociationRootV2>,
    candidate_indices: Vec<u32>,
}

impl CandidateMaterialityInputV4 {
    pub(super) fn from_rebuilt_candidate_v4(
        draft: Candidate,
        supporting_roots: Vec<MaterialAssociationRootV2>,
        negative_proof_roots: Vec<MaterialAssociationRootV2>,
    ) -> Result<Self, DiscoveryError>;
}

impl ProposalMaterialityInputV4 {
    pub(super) fn from_proof_obligation_v4(
        proposal: NormalizedProposalIdV1,
        required_roots: Vec<MaterialAssociationRootV2>,
        candidate_indices: Vec<u32>,
    ) -> Result<Self, DiscoveryError>;
}

impl UnsealedDiscoveryMaterialityPlanV4 {
    pub(super) fn from_rebuilt_projection_v4(
        related_artifacts: Vec<RelatedArtifact>,
        flow_edges: Vec<FlowEdge>,
        retained_evidence: Vec<Evidence>,
        candidate_inputs: Vec<CandidateMaterialityInputV4>,
        proposal_inputs: Vec<ProposalMaterialityInputV4>,
    ) -> Result<Self, DiscoveryError>;
}

struct RegistryValidatedDiscoveryMaterialityPlanV4 {
    inputs: DiscoveryMaterialityInputsV4,
    map_identity: CanonicalIdentityToken<MaterialAssociationMapIdentityV2>,
    required_root_identities:
        Vec<CanonicalIdentityToken<MaterialAssociationRootIdentityV2>>,
}

pub(super) struct SealedDiscoveryMaterialityPlanV4<'execution> {
    execution_nonce: ProviderInvocationExecutionNonceV2,
    finality_generation: NonZeroU32,
    plan: RegistryValidatedDiscoveryMaterialityPlanV4,
    _brand: PhantomData<fn(&'execution ()) -> &'execution ()>,
    // owned, non-Clone/non-serde/non-Debug/non-Display; no projection
}

pub(crate) struct FinalDiscoveryMaterialityV4 {
    related_artifacts: Vec<RelatedArtifact>,
    flow_edges: Vec<FlowEdge>,
    extension_point_candidates: Vec<Candidate>,
    proposal_verdicts: Vec<ProposalVerdict>,
    evidence: Vec<Evidence>,
    checks: Vec<Check>,
    status: DiscoveryStatus,
    receipt_eligibility: ReceiptEligibility,
    // immutable finalized public values only; no roots/keys/gap DTOs
}

pub(super) enum FinalDiscoveryReportProjectionV4 {
    ProviderFinalized(ProviderFinalizedReportProjectionV4),
    EarlyTerminal(EarlyTerminalReportProjectionV4),
    // private variants; no Clone/serde/getter/raw constructor
}

struct ProviderFinalizedReportProjectionV4 {
    analysis_id: AnalysisId,
    source: DiscoverySource,
    materiality: FinalDiscoveryMaterialityV4,
}

struct EarlyTerminalReportProjectionV4 {
    analysis_id: AnalysisId,
    source: DiscoverySource,
    proposal_verdicts: Vec<ProposalVerdict>,
    checks: Vec<Check>,
    receipt_eligibility: ReceiptEligibility,
    // status is exact Insufficient; related/edges/candidates/evidence empty
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum EarlyTerminalKindV4 {
    UnsupportedSourceFormat,
    AllProposalsPreflightBlocked,
}

impl FinalDiscoveryMaterialityV4 {
    pub(super) fn from_registry_finish_v4(
        authority: RegistryFinalMaterialityConstructionAuthorityV4<'_>,
        related_artifacts: Vec<RelatedArtifact>,
        flow_edges: Vec<FlowEdge>,
        extension_point_candidates: Vec<Candidate>,
        proposal_verdicts: Vec<ProposalVerdict>,
        evidence: Vec<Evidence>,
        checks: Vec<Check>,
        status: DiscoveryStatus,
        receipt_eligibility: ReceiptEligibility,
    ) -> Result<Self, DiscoveryError>;
}

impl FinalDiscoveryReportProjectionV4 {
    pub(super) fn from_finished_execution_v4(
        authority: RegistryFinalReportProjectionConstructionAuthorityV4,
        analysis_id: AnalysisId,
        source: DiscoverySource,
        materiality: FinalDiscoveryMaterialityV4,
    ) -> Result<Self, DiscoveryError>;

    pub(super) fn from_early_terminal_v4(
        authority: EarlyTerminalReportConstructionAuthorityV4,
        analysis_id: AnalysisId,
        source: DiscoverySource,
        proposal_verdicts: Vec<ProposalVerdict>,
        checks: Vec<Check>,
        receipt_eligibility: ReceiptEligibility,
    ) -> Result<Self, DiscoveryError>;
}

impl<'execution, 'context>
    ProviderInvocationRegistryV2<'execution, 'context>
{
    pub(super) fn seal_discovery_materiality_plan_v4(
        &mut self,
        prepared: &PreparedDiscoveryFinalityV4<'execution>,
        material_associations: &MaterialAssociationMapV2,
        plan: UnsealedDiscoveryMaterialityPlanV4,
    ) -> Result<SealedDiscoveryMaterialityPlanV4<'execution>, DiscoveryError>;

    pub(super) fn finish_execution_v4(
        self,
        prepared: PreparedDiscoveryFinalityV4<'execution>,
        material_associations: MaterialAssociationMapV2,
        materiality_plan: SealedDiscoveryMaterialityPlanV4<'execution>,
    ) -> Result<FinishedAnalysisExecutionProjectionV4, DiscoveryError>;
}

impl AnalysisExecutionSnapshotV4 {
    pub(super) fn from_finished_execution_v4(
        finished: FinishedAnalysisExecutionProjectionV4,
    ) -> Self;

    pub(super) fn analysis_id_v4(&self) -> &AnalysisId;

    pub(super) fn into_final_report_projection_v4(
        self,
    ) -> Result<FinalDiscoveryReportProjectionV4, DiscoveryError>;
}

impl DiscoveryReport {
    pub(super) fn from_final_projection_v4(
        projection: FinalDiscoveryReportProjectionV4,
    ) -> Result<Self, DiscoveryError>;
}
```

`finish_execution_v4` owns the sole receipt-assessment decision and the
registry-retained `ReceiptIssuerPort`; orchestration supplies neither an
eligibility value nor a finish-time issuer argument. It first finalizes
verdicts and checks. Unless mode is Validate, at least one selected proposal
exists, every selected proposal has mutation intent and is Supported with no
coverage gap/blocker, and every material Blocking check is Satisfied or
NotApplicable, it makes zero issuer calls and constructs the exact canonical
ineligible blocker set internally. When all gates pass it constructs one
borrowed `ReceiptIssuanceRequest` (the existing Task7 port name, upgraded only
to `SourceSnapshotV2`) from the registry's exact request and
bundled captured snapshot and calls `assess` exactly once. The returned value
must have `eligible=true` iff blockers is empty, otherwise finish rejects;
blockers are nonempty sorted unique validated reason strings. The value is
moved into final materiality. Task 7 assesses eligibility only: it creates no
receipt ID, store, lease or persistent receipt, and native composition retains
the no-op `receipt_store_not_implemented` issuer.

Provider-backed PlatformXml finalization is not the only legal report path.
The inherited EDT diagnostic-only report and the all-proposals-blocked
Configuration preflight report terminate after the exact Task4 composite
capture but before catalog build, registry construction, evidence I/O or issuer
assessment. `prepare_discovery_start_v4` is the sole derivation and returns the
nonforgeable `EarlyTerminalPreparedDiscoveryV4` variant only when the exact
validated request/snapshot condition holds. Consuming that token through
`into_final_report_projection_v4` builds one
`EarlyTerminalAnalysisIdentityV4`, asks the determinism finalizer for
its ID exactly once, consumes the same canonical source into the public DTO and
mints `EarlyTerminalReportConstructionAuthorityV4`. It internally constructs
the unchanged exact v6 check/verdict/blocker tuples, Insufficient status and
empty related/edge/candidate/evidence vectors. No caller supplies those parts.

Its exact framed identity tail is distinct and catalog-free:

```text
EarlyTerminalExecutionIdentityBytesV4 =
  u16be(schema=4)
  || u16be(kind: UnsupportedSourceFormat=1
                 | AllProposalsPreflightBlocked=2)

early analysisId payload =
  normalized request bytes
  || canonical source bytes
  || vec(exhaustive contract-registry strings)
  || bytes(EarlyTerminalExecutionIdentityBytesV4)
```

The full early writer is visible only to its determinism finalizer; its two
bytes after schema cannot collide with the provider-backed 96-byte binding
frame because the enclosing tail lengths differ. `FinalDiscoveryReportProjectionV4`
is the private two-variant one-shot sum consumed by the sole
`DiscoveryReport::from_final_projection_v4`: ProviderFinalized moves the
finished materiality, while EarlyTerminal validates the exact closed empty/
Insufficient shape. Thus “sole report projection” means this closed sum, not a
requirement to build a Platform catalog for an early terminal.

`projection.rs::build_unsealed_discovery_materiality_plan_v4` is the sole
caller of the three ports-owned checked factories and therefore the sole
producer of the unsealed plan. It exhaustively consumes the unchanged v6
rebuilt graph/mechanism/candidate/proposal proof-obligation inputs and accepts
opaque validated association roots, never a caller gap, affect, verdict,
status, sentinel choice or `DiscoverySource`. Registry sealing proves request/proposal
completeness, every root against the final map, and the exact plan/map relation.
`CandidateMaterialityInputV4::draft` carries only the existing structural
candidate fields; every evidence-derived affects/blocker/coverage field must be
unset and is rejected by its checked factory otherwise. Candidate indices are
checked, in range,
sorted and form the exact proposal relation. The sealed map token and complete
required-root token vector are re-encoded and compared again in finish.
The sealed plan cannot outlive or cross the branded execution. Finish alone
combines it with the internally constructed effective gaps and creates the
final public projection through the one-shot model authority. The consuming
snapshot method then creates the report projection through its separate
nonforgeable authority, and `DiscoveryReport::from_final_projection_v4` moves
those values without recalculating them.

`finish_execution_v4` reconstructs all raw rollups/traces, admission gaps,
effective invocations and provider/admission effective candidates internally.
The model-owned effective-invocation constructor computes its own effective
digest after consuming its one-shot registry authority. `ProviderOutcomeSnapshot` owns
raw `retryable`, raw identity token and raw digest; finish rebuilds it from the
still-current collected outcome and independently rechecks the digest before
moving that same value into the rollup. No caller supplies either digest,
coverage or DTO.

Before overflow normalization, finish validates the **complete** canonical
candidate set and every map/admission/fan-out/raw-class relation. The private
`RegistryFinishValidationV4::count_effective_gap_material_subjects_v4` gathers
references to the ports-owned opaque `Artifacts` scopes while they are still
registry candidate parts and is the sole production caller of Task5B's
`ProviderMaterialArtifactSetV2::canonical_union_cardinality_v2`. It receives
only the returned `u32`; no set member escapes. `SourceSetWide` and QueryWide
contribute no set reference and therefore zero material subjects. Finish alone
compares that exact union cardinality with the inherited limit:
Task5B `CardinalityOverflow` maps to the nonretryable registry-finish contract
mismatch before any threshold decision or finished value; it is not the
2,001-subject sentinel branch.

```text
MAX_EFFECTIVE_EVIDENCE_GAPS = 256
MAX_EFFECTIVE_GAP_SUBJECTS = 2_000
```

At at most 256 rows and 2,000 subjects the complete candidate vector is stored.
At 257 rows or 2,001 subjects, and only then, registry finish privately mints
one ownerless/portless/rootless QueryWide `EvidenceGapLimit` plus its one
Admission effective row and replaces only the effective-gap projection. Raw
provider gaps, sealed PerPort/Global admission histories and their per-owner
effective-invocation copies remain. There is no callable sentinel constructor,
`Some`/`None` argument or branch chosen by orchestration.

`FinishedAnalysisExecutionProjectionV4` is the one complete physical seal. It
owns the analysis prefix (and therefore the sole analysis-header binding field
and canonical source), raw rollups,
effective invocations, trace, map, internally derived sentinel option/effective
gaps, the validated canonical traversal-gap set and one already finalized
`FinalDiscoveryMaterialityV4`. The latter exposes no invocation key,
association root, admission DTO or recomputation input. The ports-owned
snapshot accepts nothing else. Its constructor is the sole caller of the
determinism-owned finalizer, computes the ID exactly once from the complete
finished seal and stores it beside that seal; `analysis_id_v4()` returns only
a shared reference and accepts no loose argument. The consuming
`into_final_report_projection_v4` consumes the prefix's exact
`CanonicalDiscoverySourceV4` into `DiscoverySource`, then moves that source,
the stored ID and stored final materiality/edge/evidence values into a
nonforgeable one-shot `FinalDiscoveryReportProjectionV4`; the model-owned
`DiscoveryReport::from_final_projection_v4` is its sole consumer. It validates
shape/order but never recomputes admission, gaps, affects, verdicts, status or
receipt eligibility. Thus provider, traversal,
source/catalog, materiality and analysis-prefix state cannot be mixed after
validation.

Canonical writing uses one Task7-private write-only
`CanonicalIdentitySinkV4`. Its byte storage, length and finalization are
private to `determinism.rs`; sibling-visible operations can only append typed
primitives or a length-framed nested closure. No `AsRef`, `Deref`, slice,
length, `Vec`, serde, `Debug` or `Display` projection exists. Opaque typed
`CanonicalIdentityToken<K>` values implement equality/order only and carry no
validation authority. All Task7 root/key/map/model/projection writers take
this sink, never caller-owned `&mut Vec<u8>`. Owner comparators decorate values
with opaque tokens, sort canonically and reject adjacent equality; the exact
retained raw subsequence is deliberately not digest-sorted. Explicit Task4/
Task5B binding and provider-material identity writers are permitted only for
their intentionally exported typed identity values; they never expose a query
membership capability, Task7 invocation key or association root.

---

## 1. Dependency graph and acceptance identities

### 1.1 External design-package protocol is not production acceptance

The only cycle-free design package is the exact owner tuple:

```text
Task 4 v7 dynamic registered-material addendum
  + Task 5B v7 contract
  + Task 6 v2-v7 owner addendum
  + this Task 7 v7 owner addendum
    -> close the exact capture/provider/consumer semantics across all four
    -> compute one external four-owner byte tuple
    -> reproduce Task 6's six query-v3 goldens and extra-frame negative with
       the external two-path generator against that tuple
    -> fresh owner self-audits over the exact tuple
    -> separate independent reviews over the same exact tuple
    -> one atomic external-ledger transition for the complete
       Task4/Task5B/Task6/Task7 design package
```

No Task5A implementation OID, Task5C-Evidence design/review/implementation OID,
Task 8 identity or production commit belongs to this design-package protocol.
Any edit to any owner byte makes the external package tuple and all derived
generator evidence, owner audits, independent reviews and ledger claim stale;
the complete protocol restarts. An unchanged individual file digest remains
mathematically correct but is not standalone package-acceptance evidence.

### 1.2 Production order

Production has a different, exact partial order:

```text
accepted Task4/Task5B/Task6/Task7 design package
  -> accepted TASK5A_ACCEPTED_SHA
  -> accepted TASK4_V7_ACCEPTED_GIT_OID
  -> accepted TASK5B_V7_ACCEPTED_GIT_OID
  -> accepted Task6-v2-v7 implementation

accepted Task4 + Task5A + Task5B-v7 implementation
  -> accepted Task5C-Evidence implementation
  -> exact TASK5C_EVIDENCE_ACCEPTED_GIT_OID

accepted Task6-v2-v7 implementation
  + exact TASK5C_EVIDENCE_ACCEPTED_GIT_OID
  + accepted Task5B-v7 implementation
    -> Task7PrerequisiteSliceV1 implementation and independent acceptance
    -> Task 8 implementation, including Task7Task8IntegrationV1
```

Before the first Task 7 production RED, the implementation ledger contains
verified repository-object values with the repository's real object format for:

```text
TASK5A_ACCEPTED_SHA
TASK4_V7_ACCEPTED_GIT_OID
TASK5B_V7_ACCEPTED_GIT_OID
TASK6_V2_V7_ACCEPTED_GIT_OID
TASK5C_EVIDENCE_ACCEPTED_GIT_OID
```

`TASK4_V7_ACCEPTED_GIT_OID` is the accepted implementation of the Task 4 v7
dynamic registered-material capture authority consumed by Task 5B v7 section
3.11: snapshot-owned FormXml/FormModule Present/Missing/NotApplicable
expectations, opaque keys and source/composite fingerprint v2 authority. Task 7
does not reimplement it, but the accepted Task 5B/context implementation it
consumes cannot exist without it.

Current repository commits are 40-lowercase-hex Git OIDs; immutable document,
self-audit, independent-review and evidence identities are 64-lowercase-hex
SHA-256 values. The ledger validates object/file existence and kind. Generic
`TASK5C_ACCEPTED_SHA`, whole Task 5C, the combined historical Task5C draft, a
future Mutation slice, a branch or a self-audit in place of independent review
cannot satisfy the Evidence row.

### 1.3 Precise Task5C-Evidence relationship

This Task 7 addendum is transitive lineage of the four-owner design package and
therefore preexists the later accepted Task5C-Evidence implementation.
Task5C-Evidence does not import, revalidate or gate on this
addendum, a Task 7 implementation, or Task 7 integration. It consumes only its
own Task 4/5A/5B upstream authorities.

Task 7 is the downstream Support consumer. It imports exactly
`TASK5C_EVIDENCE_ACCEPTED_GIT_OID` before production and never requires whole
Task 5C or Task5C-Mutation. Task 6 remains a sibling Task 5B consumer and imports
no Task5C type or OID. Any document that makes Task5C-Evidence wait for Task 7,
makes Task 6 wait for Evidence, or makes the Task4/Task5B/Task6/Task7 **design** package wait
for an Evidence implementation has created a reverse edge and is rejected.

### 1.4 Task 8 is downstream and non-gating

Task 8 is a downstream consumer, not an authority for the four-owner design
package. This addendum owns the generic prerequisite/integration boundary in
section 9. Task 8 must later import it and deliver the concrete integration; a
stale Task 8 file name, type spelling, dependency diagram or open downstream
finding cannot block or reopen an accepted Task4/Task5B/Task6/Task7 design package.

## 2. Exact upstream imports and superseded v6 query prose

### 2.1 One whole composite-bound Platform catalog context

Task 7 imports exactly the whole Task 5B context boundary, without a local
lookalike, half-context or consumer-specific catalog view:

```text
PlatformCatalogContextV1
PlatformCatalogPort
```

`PlatformCatalogPort::build_context` is the sole public construction boundary.
Its imported object-safe signature is exact:

```rust
fn build_context(
    &self,
    snapshot: &SourceSnapshotV2,
    source_reader: &dyn SourceSnapshotPort,
) -> Result<PlatformCatalogContextV1, PlatformCatalogBuildErrorV1>;
```

The returned non-forgeable value contains one composite snapshot identity, the
complete configuration-catalog set, the complete registered-Form catalog set
and all three exact snapshot-bound configuration, registered-Form and
Analysis-BSL witness sets. Task 7 stores that one value in
`EvidenceExecutionContext`; Metadata, Form, BSL and Support query smart
constructors receive only a borrow from the same whole context. Task 7 neither
accepts the two catalog sets separately nor creates an Analysis-only facade.

The Task 7-owned orchestration capability is exact:

```rust
pub(crate) struct EvidenceExecutionContext<'a> {
    workspace: &'a DiscoveryExecutionContext,
    platform_catalog_context: &'a PlatformCatalogContextV1,
    snapshot: &'a SourceSnapshotV2,
    source_reader: &'a dyn SourceSnapshotPort,
}

impl<'context> EvidenceExecutionContext<'context> {
    pub(super) fn from_execution_parts_v1(
        workspace: &'context DiscoveryExecutionContext,
        platform_catalog_context: &'context PlatformCatalogContextV1,
        snapshot: &'context SourceSnapshotV2,
        source_reader: &'context dyn SourceSnapshotPort,
    ) -> Self;
}
```

`SourceSnapshotV2` is Task 4's validated composite: one Analysis atomic
`SourceSetSnapshotV2`, a canonical unique Destination vector and one
`CompositeSnapshotIdV2`. Task 6 query smart constructors receive only
`platform_catalog_context`; Task 6 provider execution receives
`snapshot.analysis_snapshot()` and `source_reader`. Metadata/Form receive the
same composite/context/reader capabilities required by Task 5B. Task 7 never
replaces any value with a clone, detached catalog, raw root, free function,
callback or process-global reader.

The accepted Task 4 shape does not currently expose enough neutral report
data to construct the existing public `DiscoverySource`; accepting that DTO
from orchestration would allow a foreign source to be paired with a valid
snapshot. The four-owner co-freeze therefore adds this narrow Task4-owned,
Task7-neutral projection before production:

```rust
pub(crate) struct SourceSnapshotDiscoveryProjectionV1 {
    // private, owned, non-Clone/non-serde; exact snapshot-derived report data
}

pub(crate) struct LinkedSourceSnapshotProjectionV1 {
    // private; Analysis first, then canonical Destination order
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum SourceSnapshotProjectionRoleV1 {
    Analysis,
    Destination,
}

impl SourceSnapshotV2 {
    pub(crate) fn project_discovery_source_v1(
        &self,
    ) -> SourceSnapshotDiscoveryProjectionV1;
}

impl SourceSnapshotDiscoveryProjectionV1 {
    pub(crate) fn analysis_source_set_v1(&self) -> &str;
    pub(crate) fn analysis_source_format_v1(&self) -> SourceFormat;
    pub(crate) fn analysis_source_kind_v1(&self) -> SourceSetKind;
    pub(crate) fn diagnostic_workspace_epoch_v1(&self) -> u64;
    pub(crate) fn linked_sources_v1(
        &self,
    ) -> &[LinkedSourceSnapshotProjectionV1];
    pub(crate) fn composite_fingerprint_transport_v1(&self) -> &str;
}

impl LinkedSourceSnapshotProjectionV1 {
    pub(crate) fn source_set_v1(&self) -> &str;
    pub(crate) fn role_v1(&self) -> SourceSnapshotProjectionRoleV1;
    pub(crate) fn source_fingerprint_transport_v1(&self) -> &str;
}
```

Task 4 constructs it only from the complete validated snapshot: exact Analysis
display name/format, Analysis-first plus canonical Destination linked rows,
canonical source-fingerprint transports, canonical composite transport and
diagnostic epoch. It exposes no manifest/root/path/raw digest, constructor,
serde or mutation. The sole downstream production caller of every accessor is
`CanonicalDiscoverySourceV4::from_snapshot_projection_v1` in `ports.rs`;
Destination maps to the unchanged public `SourceSnapshotRole::Mutation`
spelling only at that compatibility boundary. That Task7 owner validates the
projection once, writes the unchanged v6 source identity and is later consumed
into the public DTO. The caller-built materiality plan contains no source.

The context has no public/general constructor. The checked factory above is
statically callable only from the exact Stage-0 code in `use_case.rs`, after
the one context build, and binds all four same-`'context` references into one
non-Clone value. The generative registry retains a borrow of that exact value;
a typed invoke borrows it directly and drops the provider-call borrow before
mutating its entry. An invoke overload accepts no second context, snapshot,
workspace or reader argument.

The context is also the only Task 7-visible bridge to Task 4's opaque dynamic
registered-material authority. Task 7 exposes and accepts no manifest key,
relationship, expectation state, expectation handle, path, suffix or raw
registered material, does not call Task 4/Task 5B specialized readers, and does
not reproduce their resolver chains. The imported Task 5B/Task 6 provider
entrypoints consume that authority through their exact owner-defined
whole-context APIs. Task 7 passes the same context-owned capability, schedules
the typed provider query and records its imported digest/outcome without
inspecting material authority. MetadataCatalog and FormInspection may perform
their owner-defined dynamic FormXml verifier/read calls on this injected
reader. For BSL FormModule material, only Task 6 obtains the exact Analysis
atomic snapshot and consumes Task 5B's
`analysis_bsl_material_scan_plan`/`read_analysis_bsl_material_verified`
boundary. The
Platform catalog context and every material view/ref/resolver/handle contain no
reader capability; the explicit `EvidenceExecutionContext.source_reader` is the
sole injected reader capability.

Task 7 preserves the imported Task 6 classification without catching or
renaming it. A semantic context/view/ref/resolver/handle/projection/state/key/
manifest mismatch is nonretryable `registered_material_handle_mismatch` before
injected-port I/O and yields zero invocation prefix. Only post-validation
external filesystem appearance/disappearance/content/identity/topology drift
is retryable `source_fingerprint_mismatch`, also with zero invocation prefix.
The unified scan dispatcher preserves the exact Task 6 outcome/counter matrix:
Ordinary Present performs zero registered-Form verifier calls, one verified
byte read and one parse; Registered Present performs one verifier, one verified
byte read and one parse; Registered Missing performs one verifier and zero byte
reads/parses; Registered NotApplicable, unsupported Ordinary and a per-item
`FileBytesLimit` perform zero verifier/read/parse calls and emit their
exact typed gap. `FileBytesLimit` consumes its one file, is nonterminal and
leaves later selected items eligible. Only precomputed `FileCount` and
`TotalBytes` are terminal, omit their selected suffix and permit no subsequent
material I/O. Association/admission never reclassifies, prefixes or retries
these provider outcomes.

The execution snapshot binds both the exact
`configuration_catalog_set_digest` and
`registered_form_catalog_set_digest`. The Task 7 v6 field that binds only the
configuration set is superseded. Witness bytes remain location authority and
do not enter source-free semantic digests, but records resolved from witnesses
retain their exact snapshot/leaf/fingerprint authority.

### 2.2 Metadata and Form query v2 are imported, not reconstructed

Task 7 uses the sole Task 5B v7 smart-constructed values:

```text
METADATA_COMPOSITE_QUERY_ENCODER = "metadata-composite-query/v2"
FORM_INSPECTION_QUERY_ENCODER = "form-inspection-query/v2"

MetadataCompositeQueryV2
FormSourceSetQueryV2
MetadataQueryAssociationAuthorityV1
FormQueryAssociationAuthorityV1
SupportQueryAssociationAuthorityV1
```

The Metadata query binds the composite snapshot, both
catalog-set digests, exact Analysis and canonical Destination identities,
destination pairs, analysis presence keys, requested Form material scopes and
`max_records`. The Form query binds the exact Analysis
identity/fingerprint, matching configuration-catalog digest, matching
registered-Form-catalog digest, requested Form material scopes and
`max_records`.

In particular, the imported private query authorities remain typed:

```text
MetadataCompositeQueryV2.registered_form_catalog_set_digest: Digest32
FormSourceSetQueryV2.analysis_registered_form_catalog_digest: Digest32
```

They are derived and validated by the Task 5B smart constructors from the one
borrowed context. Task 7 neither accepts their textual spelling nor stores a
caller-selected string placeholder for either digest.

Task 7 never accepts or stores caller-selected pair/presence/Form-scope hashes
or membership sets. It imports the three owner-minted association-authority
types above only as opaque values; Support's value is minted when the section
2.4 query is constructed.
It calls the imported constructors with typed vectors and records the imported
`query_digest()` byte-for-byte. The v6 `ProviderQueryScope::MetadataComposite`
and `FormSourceSet` field lists are deleted rather than kept as a second
encoder. Their old query-golden tests become negative fixtures.

`FormMaterialAssociationBuilderV1` remains the Task 5B-owned builder that
deduplicates bounded request contributions into one provider Form scope. It is
not `MaterialAssociationMapV2`: proposal IDs are stripped before it constructs
the query scope, while the separate Task 7 map retains why that scope matters.
Two proposals contributing an identical Form/command/runtime/pair scope produce
one identical query and two application associations.

Metadata runs once for the composite snapshot. FormInspection runs once for
Analysis. The complete registered-Form sidecar drives the exhaustive Form scan;
Task 7 does not cap, path-format or reconstruct it.

### 2.3 All BSL queries use Task 6 query encoder v3

Task 7 imports the exact Task 6 `CodeSearchQuery`, `DefinitionQuery` and
`CallGraphQuery` smart values under:

```text
BSL_PROVIDER_QUERY_ENCODER = "snapshot-bsl-provider-query/v3"
CodeSearchQueryAssociationAuthorityV1
DefinitionQueryAssociationAuthorityV1
CallGraphQueryAssociationAuthorityV1
```

Each query constructor borrows the same composite-bound
`PlatformCatalogContextV1` and privately binds its exact Analysis projection:

```text
exact Analysis AtomicSourceIdentityV2
analysis SourceFingerprintV1
analysis configuration-catalog digest
accepted RegisteredFormCatalogV1 contract_version
analysis registered-Form-catalog digest
canonical port-specific terms/methods/callers
max_records
```

Task 7 records the imported v3 digest without an application wrapper. Depth,
round, proposal, mechanism, traversal origin and stage are not v3 query fields.
The historical `snapshot-bsl-provider-query/v2` constant/golden is a negative
fixture after implementation. A FormModule key is obtained only through the
opaque whole-context material authority owned by Task 4/Task 5B; Task 7 and
Task 6 never format a suffix.

Task 6 section 7.1 publishes six positive query-v3 goldens. Task 7 imports only
their final `query_digest()` bytes for cross-owner conformance; it never copies
or reconstructs Task 6 payload bytes, payload lengths, payload SHA-256, source
identity framing or the `H` calculation:

```text
CodeSearch empty  f0c11bd41c207547a9eb7bc8f5230edc04e6ae0bef8340039b66979c4db90683
CodeSearch one    b14163b7ec4244043e4c98919c1ab2f3393fa39d8696c67a3bc96838ca2fda1a
Definition empty  2ca861ede84017e0bf6e8e110bb079bf784329a15ef325c24fcd9c962af6a9ae
Definition one    61d0dc8d91a05346311fcf5b8a087b19f6b7eed3e0bf5f0118765e24c12a2049
CallGraph empty   ea5f488e008df9ac016bf78b6c60d06193164d3d6f3e2fc45a02185772510060
CallGraph one     97f2faf6e7d9901b2b70d3d972492dfd19cfcc522c419bd68107034846008372
```

Task 6 owns the direct framing rule: its 148-byte
`encode(AtomicSourceIdentityV2)` is appended directly after the port tag, with
no second outer `bytes(...)` frame. Its published forbidden extra-frame
Definition-empty digest
`1ea160f0b9bfacbb134047a6d1be1d23dc45961ca0a8311505420e6d25520c18`
must never be accepted as an invocation key. The external package generator
reproduces all six positives and that negative; Task 7 merely compares the
imported typed digest accessor with the corresponding frozen bytes.

### 2.4 Support query v2

Task 7 imports the exact Task 5B-owned `SupportStateQueryV2` smart constructor
and the Task5C-Evidence implementation behind `SupportStatePort`:

```text
SUPPORT_STATE_QUERY_ENCODER = "support-state-query/v2"
MAX_SUPPORT_QUERY_SUBJECTS = 4096
```

The query contains only canonical typed source groups/subjects and their exact
semantic/snapshot authority. Proposal/Request/Mechanism association is absent.
Equal subjects requested by several conclusions are queried once and gain
several entries in `MaterialAssociationMapV2`. Task 7 applies no lossy local
Support ceiling before the imported 4,096 all-or-nothing constructor.

## 3. Exact application-owned MaterialAssociationMapV2

### 3.1 Closed types

The association layer is application state, not provider testimony:

```rust
pub(crate) const MATERIAL_ASSOCIATION_CONTRACT: &str =
    "project-material-association/v2";

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct ProviderInvocationKeyV2 {
    port: EvidencePort,
    query_digest: Digest32,
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct MaterialAssociationRootV2 {
    kind: MaterialAssociationRootKindV2,
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
enum MaterialAssociationRootKindV2 {
    Invocation {
        invocation: ProviderInvocationKeyV2,
    },
    SourceGroup {
        invocation: ProviderInvocationKeyV2,
        source: AtomicSourceIdentityV2,
    },
    Material {
        invocation: ProviderInvocationKeyV2,
        material: ProviderGroupMaterialIdentityV2,
    },
    AtomicGroup {
        invocation: ProviderInvocationKeyV2,
        group: SemanticAtomicGroupIdV2,
    },
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum ConclusionScope {
    Request,
    Proposal(NormalizedProposalIdV1),
    Mechanism(MechanismKey),
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct NormalizedProposalIdV1(String);

struct NormalizedDiscoverRequestIdentityV4 {
    request: DiscoverRequest,
    // owned validated request value; constructed by exhaustive field copy
}

struct CanonicalDiscoverySourceV4 {
    source: DiscoverySource,
    // constructed only from Task4 SourceSnapshotDiscoveryProjectionV1;
    // no Clone/serde/general constructor or component mutation
}

impl CanonicalDiscoverySourceV4 {
    fn from_snapshot_projection_v1(
        projection: SourceSnapshotDiscoveryProjectionV1,
    ) -> Result<Self, DiscoveryError>;

    fn write_identity_v4(&self, sink: &mut CanonicalIdentitySinkV4);

    fn into_discovery_source_v4(self) -> DiscoverySource;
}

struct AnalysisContractRegistryIdentityV4 {
    entries: Box<[&'static str]>,
    // exact exhaustive ordered registry from section 4.2
}

impl NormalizedDiscoverRequestIdentityV4 {
    fn from_validated_request_v4(
        request: &DiscoverRequest,
    ) -> Result<Self, DiscoveryError>;

    fn write_identity_v4(&self, sink: &mut CanonicalIdentitySinkV4);

    fn validated_request_v4(&self) -> &DiscoverRequest;
}

impl AnalysisContractRegistryIdentityV4 {
    fn current_v4() -> Self;
    fn write_identity_v4(&self, sink: &mut CanonicalIdentitySinkV4);
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum StablePreflightReasonV4 {
    CfeAnalysisConfigurationRequired,
}

pub(super) enum PreparedDiscoveryStartV4 {
    Early(EarlyTerminalPreparedDiscoveryV4),
    Provider(PreparedProviderDiscoveryV4),
}

pub(super) struct EarlyTerminalPreparedDiscoveryV4 {
    normalized_request: NormalizedDiscoverRequestIdentityV4,
    canonical_source: CanonicalDiscoverySourceV4,
    snapshot_id: CompositeSnapshotIdV2,
    diagnostic_workspace_epoch: u64,
    kind: EarlyTerminalKindV4,
    preflight_blockers:
        BTreeMap<NormalizedProposalIdV1, StablePreflightReasonV4>,
    // private non-Clone/non-serde one-shot owner
}

pub(super) struct PreparedProviderDiscoveryV4 {
    normalized_request: NormalizedDiscoverRequestIdentityV4,
    canonical_source: CanonicalDiscoverySourceV4,
    snapshot_id: CompositeSnapshotIdV2,
    diagnostic_workspace_epoch: u64,
    preflight_blockers:
        BTreeMap<NormalizedProposalIdV1, StablePreflightReasonV4>,
    // private non-Clone/non-serde one-shot owner
}

pub(super) fn prepare_discovery_start_v4(
    request: &DiscoverRequest,
    snapshot: &SourceSnapshotV2,
) -> Result<PreparedDiscoveryStartV4, DiscoveryError>;

impl PreparedProviderDiscoveryV4 {
    pub(super) fn provider_query_request_v4(&self) -> DiscoverRequest;
}

impl EarlyTerminalPreparedDiscoveryV4 {
    pub(super) fn into_final_report_projection_v4(
        self,
    ) -> Result<FinalDiscoveryReportProjectionV4, DiscoveryError>;
}

struct AnalysisIdentityPrefixV4 {
    normalized_request: NormalizedDiscoverRequestIdentityV4,
    canonical_source: CanonicalDiscoverySourceV4,
    contract_registry: AnalysisContractRegistryIdentityV4,
    platform_catalog_execution: PlatformCatalogExecutionBindingV1,
    // sole registry/header binding field; query authorities independently own
    // their sealed copies; no Clone/getter/serde/Debug/Display
}

impl AnalysisIdentityPrefixV4 {
    fn from_registry_initialization_v4(
        normalized_request: NormalizedDiscoverRequestIdentityV4,
        source: CanonicalDiscoverySourceV4,
        platform_catalog_execution: PlatformCatalogExecutionBindingV1,
    ) -> Result<Self, DiscoveryError>;

    fn write_analysis_identity_payload_v4(
        &self,
        sink: &mut CanonicalIdentitySinkV4,
        write_execution_tail: impl FnOnce(&mut CanonicalIdentitySinkV4),
    );

    fn into_discovery_source_v4(self) -> DiscoverySource;
}

struct EarlyTerminalAnalysisIdentityV4 {
    normalized_request: NormalizedDiscoverRequestIdentityV4,
    canonical_source: CanonicalDiscoverySourceV4,
    contract_registry: AnalysisContractRegistryIdentityV4,
    kind: EarlyTerminalKindV4,
    // ports-owned one-shot identity owner; no catalog/binding/provider state
}

impl EarlyTerminalAnalysisIdentityV4 {
    fn from_prepared_parts_v4(
        normalized_request: NormalizedDiscoverRequestIdentityV4,
        canonical_source: CanonicalDiscoverySourceV4,
        kind: EarlyTerminalKindV4,
    ) -> Self;

    pub(super) fn write_analysis_identity_payload_v4(
        &self,
        sink: &mut CanonicalIdentitySinkV4,
    );

    fn into_discovery_source_v4(self) -> DiscoverySource;
}

pub(super) struct ProviderInvocationRegistryV2<'execution, 'context> {
    execution_nonce: ProviderInvocationExecutionNonceV2,
    execution_context: &'context EvidenceExecutionContext<'context>,
    receipt_issuer: &'context dyn ReceiptIssuerPort,
    analysis_identity_prefix: AnalysisIdentityPrefixV4,
    preflight_blockers:
        BTreeMap<NormalizedProposalIdV1, StablePreflightReasonV4>,
    entries: Vec<ProviderInvocationRegistryEntryV2>,
    key_to_slot: BTreeMap<ProviderInvocationKeyV2,
                          ProviderInvocationRegistrySlotIdV2>,
    artifact_spellings: ExactArtifactSpellingRegistryV1,
    traversal: RegistryTraversalLedgerV3<'execution>,
    finality_generation: u32,
    finality_state: RegistryFinalityStateV4,
    execution_invalidated: bool,
    _brand: PhantomData<fn(&'execution ()) -> &'execution ()>,
}

enum RegistryFinalityStateV4 {
    Collecting,
    Prepared {
        generation: NonZeroU32,
        admission: RegistryDerivedApplicationAdmissionV3,
        provider_effective_classes: Vec<RegistryProviderEffectiveGapClassV3>,
        association_requirements: Vec<RegistryFinalityAssociationRequirementV4>,
        next_requirement_index: u32,
        next_projection_issuance: u32,
        outstanding_requirement: Option<(u32, NonZeroU32)>,
        materiality_seal_issued: bool,
    },
    Invalidated,
}

struct RegistryFinalityAssociationRequirementV4 {
    order: CanonicalIdentityToken<FinalityRequirementOrderIdentityV4>,
    source: RegistryFinalityRequirementSourceV4,
    instructions: Vec<FinalityAssociationInstructionV4>,
}

enum RegistryFinalityRequirementSourceV4 {
    ProviderEffectiveClass { class_index: u32 },
    AdmissionBucket { bucket_index: u32 },
}

struct FinalityRequirementOrderIdentityV4;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct ProviderInvocationExecutionNonceV2(NonZeroU64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct ProviderInvocationRegistrySlotIdV2(u32);

pub(super) struct TraversalRequirementSpecV3 {
    gap_reason: TraversalGapReason,
    phase: TraversalPhase,
    depth: Option<u8>,
    scope: TraversalGapScopeV3,
    conclusion_scopes: Vec<ConclusionScope>,
    // canonical closed decision expectation; checked factory only from
    // traversal.rs validates the exact reason/phase/round/depth combination
}

impl TraversalRequirementSpecV3 {
    pub(super) fn from_bounded_decision_v3(
        gap_reason: TraversalGapReason,
        phase: TraversalPhase,
        depth: Option<u8>,
        scope: TraversalGapScopeV3,
        conclusion_scopes: Vec<ConclusionScope>,
    ) -> Result<Self, DiscoveryError>;
}

pub(super) struct TraversalRequirementIdV3<'execution> {
    slot: u32,
    execution_nonce: ProviderInvocationExecutionNonceV2,
    _brand: PhantomData<fn(&'execution ()) -> &'execution ()>,
    // owned non-Clone/non-serde token; no raw slot projection
}

struct RegistryTraversalLedgerV3<'execution> {
    entries: Vec<RegistryTraversalRequirementV3>,
    sealed: bool,
    _brand: PhantomData<fn(&'execution ()) -> &'execution ()>,
}

struct RegistryTraversalRequirementV3 {
    spec: TraversalRequirementSpecV3,
    resolution: RegistryTraversalResolutionV3,
}

enum RegistryTraversalResolutionV3 {
    Pending,
    Completed,
    Gapped,
}

struct ProviderInvocationRegistryEntryV2 {
    slot_id: ProviderInvocationRegistrySlotIdV2,
    key: ProviderInvocationKeyV2,
    query_kind: ProviderInvocationQueryKindV2,
    state: ProviderInvocationRegistryEntryStateV2,
    application_stages: BTreeSet<ProviderInvocationStageV2>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProviderInvocationQueryKindV2 {
    Metadata,
    Form,
    CodeSearch,
    Definition,
    CallGraph,
    Support,
}

enum ProviderInvocationRegistryEntryStateV2 {
    Staged,
    Finished {
        provider: EvidenceProvider,
        raw_outcome: CollectedProviderOutcome,
        semantic_groups: Vec<SemanticAtomicEvidenceGroupV2>,
        association_authority: ProviderQueryAssociationAuthorityV2,
    },
    Invalidated,
}

enum ProviderQueryAssociationAuthorityV2 {
    Metadata(MetadataQueryAssociationAuthorityV1),
    Form(FormQueryAssociationAuthorityV1),
    CodeSearch(CodeSearchQueryAssociationAuthorityV1),
    Definition(DefinitionQueryAssociationAuthorityV1),
    CallGraph(CallGraphQueryAssociationAuthorityV1),
    Support(SupportQueryAssociationAuthorityV1),
}

pub(super) struct RegisteredProviderInvocationPlanV2<'execution> {
    key: ProviderInvocationKeyV2,
    slot_id: ProviderInvocationRegistrySlotIdV2,
    execution_nonce: ProviderInvocationExecutionNonceV2,
    association_authority: ProviderQueryAssociationAuthorityV2,
    _brand: PhantomData<fn(&'execution ()) -> &'execution ()>,
    // private fields; no Clone/serde/raw constructor
}

pub(super) struct RecordedInvocationIdV2<'execution> {
    slot_id: ProviderInvocationRegistrySlotIdV2,
    execution_nonce: ProviderInvocationExecutionNonceV2,
    _brand: PhantomData<fn(&'execution ()) -> &'execution ()>,
    // opaque owned ID; no port/digest constructor, Clone, serde or raw projection
}

pub(super) struct RecordedProviderInvocationHandleV2<'lookup, 'execution> {
    slot_id: ProviderInvocationRegistrySlotIdV2,
    execution_nonce: ProviderInvocationExecutionNonceV2,
    key: &'lookup ProviderInvocationKeyV2,
    provider: &'lookup EvidenceProvider,
    raw_outcome: &'lookup CollectedProviderOutcome,
    association_authority: &'lookup ProviderQueryAssociationAuthorityV2,
    semantic_groups: &'lookup [SemanticAtomicEvidenceGroupV2],
    _brand: PhantomData<fn(&'execution ()) -> &'execution ()>,
    // private short-lived borrow of one finished registry entry
}

pub(super) struct RecordedApplicationEvidenceViewV2<'lookup> {
    provider: &'lookup EvidenceProvider,
    semantic_groups: &'lookup [SemanticAtomicEvidenceGroupV2],
    physical_records: &'lookup [EvidenceRecord],
    provider_gaps: &'lookup [ProviderGap],
    // application work only; no invocation key/association authority
}

impl RecordedProviderInvocationHandleV2<'_, '_> {
    pub(super) fn with_application_evidence_v2<T: 'static>(
        &self,
        read: impl FnOnce(RecordedApplicationEvidenceViewV2<'_>)
            -> Result<T, DiscoveryError>,
    ) -> Result<T, DiscoveryError>;
}

impl RecordedApplicationEvidenceViewV2<'_> {
    pub(super) fn provider_v2(&self) -> EvidenceProvider;

    pub(super) fn visit_semantic_groups_v2(
        &self,
        visit: impl FnMut(&SemanticAtomicEvidenceGroupV2)
            -> Result<(), DiscoveryError>,
    ) -> Result<(), DiscoveryError>;

    pub(super) fn visit_physical_records_v2(
        &self,
        visit: impl FnMut(&EvidenceRecord) -> Result<(), DiscoveryError>,
    ) -> Result<(), DiscoveryError>;

    pub(super) fn visit_provider_gaps_v2(
        &self,
        visit: impl FnMut(&ProviderGap) -> Result<(), DiscoveryError>,
    ) -> Result<(), DiscoveryError>;
}

pub(crate) struct MaterialAssociationEntryV2 {
    root: MaterialAssociationRootV2,
    scopes: Vec<ConclusionScope>,
}

pub(crate) struct MaterialAssociationMapV2 {
    entries: Vec<MaterialAssociationEntryV2>,
}

impl MaterialAssociationMapV2 {
    pub(super) fn validate_for_registry_finish_v4(
        &self,
        validation: &mut RegistryFinishValidationV4<'_, '_, '_>,
    ) -> Result<(), DiscoveryError>;

    pub(super) fn identity_token_v2(
        &self,
    ) -> CanonicalIdentityToken<MaterialAssociationMapIdentityV2>;

    pub(super) fn write_identity_v2(&self, sink: &mut CanonicalIdentitySinkV4);
}
```

`ProviderInvocationKeyV2`, `MaterialAssociationRootV2` and its inner enum are
owned by `ports.rs`. The root is an opaque `pub(super)` struct; its `kind` field
and all four inner variant constructors/fields are module-private. It is Clone
only as an already validated immutable value: no consumer can extract its key,
switch a variant or replace a source/material/group. It has no serde/raw
constructor, variant getter, key projection, `Debug` or `Display`
implementation. `ProviderInvocationKeyV2` and the private inner enum likewise
implement neither `Debug` nor `Display`, so formatting a root cannot become an
inspection side channel. Thus direct enum construction or diagnostic formatting
cannot bypass plan/finished-handle authority even inside another crate module.
All other fields shown are private. `NormalizedProposalIdV1` is a non-serde validated
newtype created only while normalizing one request proposal. It contains the
existing exact proposal-ID spelling after the existing nonempty/length/unique
request validation; its sole identity projection is `exact_spelling()` for the
encoder below. No unvalidated `String`, alias or display spelling is accepted.

`AnalysisIdentityPrefixV4`, `CanonicalDiscoverySourceV4`,
`NormalizedDiscoverRequestIdentityV4` and
`AnalysisContractRegistryIdentityV4` are owned by `ports.rs`. The latter two are
the unchanged typed v6 canonical owners; they expose only their
`CanonicalIdentitySinkV4` writer. The normalized-request owner already owns
the validated limits and writes the unchanged complete v6 request identity
exactly once; the prefix does not carry a second `DiscoverLimits`. The prefix
also owns the sole canonical report/source owner and the one registry-header
Task5B binding field. Its closure-tail writer emits request, source and registry
in unchanged v6 order, then delegates the binding exactly once at the nested
execution-header position before the finished tail. It has no component/binding getter, comparison
callback, Clone, serde or raw-byte projection. Registry-private variant
dispatch may borrow its binding field directly; no sibling can.

`ProviderInvocationKeyV2` is constructible only by
`ProviderInvocationRegistryV2<'execution, 'context>` from one imported accepted smart
query and its exact port. Registry construction is behind one generative
`with_provider_invocation_registry_v2` execution closure. The invariant
`PhantomData<fn(&'execution ()) -> &'execution ()>` brand is shared by that
registry and the values it issues, but contains no reference to the registry.
The private nonce is fresh for that execution; a private slot ID is allocated
once and never reused. The registry alone stores the corresponding mutable
`ProviderInvocationRegistryEntryV2`; plans and recorded IDs store only its
opaque slot identity; the plan additionally owns its private key and one sealed
query-association authority, while the recorded ID contains neither. None holds
a registry reference. Consequently an issued ID can be stored across
short-lived registry borrows, a later typed invocation can still mutably borrow
the registry, and an immutable lookup borrows it only for the lifetime of the
returned handle. A design in which an ID/plan holds `&Registry`, or in which a
handle must remain live across a later mutable invocation, is forbidden as
non-constructible Rust.

Every successful typed invoke runs the accepted classifier once and stores the
complete `Vec<SemanticAtomicEvidenceGroupV2>` next to the still-current raw
outcome. `with_application_evidence_v2` is the only post-invoke read seam. Its
closure can produce ordinary owned application work but `T: 'static` prevents a
group/record/gap borrow or handle from escaping. It exposes no invocation key or
association authority and confers no finality authority: preview/prepare/finish
always rederive from the registry-owned current groups. The closure and every
short recorded handle end before a mutable registry borrow. For a post-response
association, `association.rs` first obtains an owned opaque root under the short
handle, drops the handle, then calls the root's fail-closed spelling recheck
with `&mut registry`, and only afterward mutates its builder. Holding the handle
while borrowing the same registry mutably is forbidden by contract and by the
compile fixture.

The registry begins in `Collecting`. Admission preview and the final prepared
state use the same private deterministic derivation over complete current raw
groups and retained `request.limits.max_evidence`; there is no mutable admission ledger fed
by caller envelopes. `Prepared` stores one canonical bucket per occupied
reason/port, the exact retained-by-slot decision, every location-erased provider
gap class and the complete finality association-requirement set. Its generation
is validation state only and never enters canonical identity. A second prepare,
any later invoke/stage, a missing/duplicate consumed requirement or a prepared
token from another execution invalidates finish.

The mixed requirement order is closed and independent of discovery order:

```text
FinalityRequirementOrderIdentityBytesV4 =
  u16be(kind) || bytes(complete owner identity)

kind 1 = ProviderEffectiveClass  # complete location-erased EffectiveGap identity
kind 2 = AdmissionBucket         # complete EvidenceAdmissionGap identity
```

The registry constructs one
`CanonicalIdentityToken<FinalityRequirementOrderIdentityV4>` by writing the
complete grammar above, including the `bytes(...)` length frame, and sorts by
that token's complete private bytes. It does not sort first by an enum and then
by the unframed owner token: those two orders can differ when owner-identity
lengths differ. It never uses a hash, root, reason, slot or insertion index.
Sorting rejects an adjacent equal complete token. These bytes order only the
private project/apply/record protocol; the kind frame is not added to an
evidence gap or analysis identity.
The private source index is checked against the corresponding prepared vector
before projection and again at finish; it is lookup state only and never an
order key or canonical byte.

A mechanical length-flip golden constructs two valid variable-length owner
identities whose unframed lexical order is the reverse of their
`bytes(owner identity)` order and proves projection follows the complete framed
token. Separate mutations that drop the length frame or restore derived-enum
ordering must fail.

`ProviderQueryAssociationAuthorityV2` is a closed module-private, non-Clone,
non-serde sum that only wraps the six exact owner-minted Task5B/Task6
capabilities. It has no raw constructor, iterator, digest-to-member lookup or
caller-supplied member set. Its private dispatch exposes only the enum-implied
exact `EvidencePort`, the owner `query_digest()`, and owner validation of one
`AtomicSourceIdentityV2` or one `ProviderGroupMaterialIdentityV2`. Equality of
a digest is never treated as proof of membership. Its fourth sealed operation
validates the authority's private `PlatformCatalogExecutionBindingV1`. For the
three Task5B variants it calls
`validate_platform_catalog_execution_v1(
registry.execution_context.platform_catalog_context,
registry.execution_context.snapshot)`; for the three Task6 variants it calls
`validate_execution_binding_v1` with the registry-private borrow of
`analysis_identity_prefix.platform_catalog_execution`. No generic
fallback or component comparison exists. Registration
obtains exactly
one authority from the exact accepted typed query and moves it into the plan;
the matching typed invoke consumes the plan and moves that same value into the
`Finished` entry. The recorded handle only borrows it. The authority and its
membership never enter provider query/raw/group/cache identity, association
encoding, receipts, serde or public output.

The Definition authority variant is imported and exhaustively covered by the
closed dispatch, but remains unreachable from registration/invocation until
the section-7.1 choice supplies its exact application boundary. Keeping the
variant in this private sum does not invent a sixth lifecycle overload or make
the reserved authority caller-obtainable.

The shared owner-violation variants
`SourceGroupNotMember`/`MaterialNotMember`, and any impossible authority
variant/port/digest mismatch discovered before root construction, map to the
single nonretryable `DiscoveryError::ProviderQueryAssociationMismatch` with
stable reason `provider_query_association_mismatch`. A material/AtomicGroup not
proved by the exact finished typed outcome maps to nonretryable
`DiscoveryError::ProviderOutcomeAssociationMismatch` with stable reason
`provider_outcome_association_mismatch`. Neither error is a provider terminal
outcome or retryable source drift; both return before association-builder
mutation and `finish_execution_v4` still enforces required-root completeness.
`PlatformCatalogExecutionMismatch` maps only to the nonretryable
`DiscoveryError::PlatformCatalogExecutionMismatch` with stable reason
`platform_catalog_execution_mismatch`, before slot allocation or provider I/O.

The generative constructor receives the exact normalized request including its
limits, one checked `EvidenceExecutionContext<'context>` containing the
execution workspace, captured `SourceSnapshotV2`, once-built
`PlatformCatalogContextV1` and injected `SourceSnapshotPort`, and one retained
`ReceiptIssuerPort`. It derives the Analysis
`AtomicSourceIdentityV2` only from that captured snapshot. Its
normative shape is:

```rust
pub(super) fn with_provider_invocation_registry_v2<'context, R>(
    prepared: PreparedProviderDiscoveryV4,
    execution_context: &'context EvidenceExecutionContext<'context>,
    receipt_issuer: &'context dyn ReceiptIssuerPort,
    run: impl for<'execution> FnOnce(
        ProviderInvocationRegistryV2<'execution, 'context>,
    ) -> Result<R, DiscoveryError>,
) -> Result<R, DiscoveryError>;
```

`DiscoverRequest` here is the existing fully deserialized and validated
request value, and `request.limits` is the sole limit authority; no raw wire
request, second limits argument or detached Analysis source reaches this
boundary. The context's retained workspace/reader are execution capabilities
only and never identity fields; the one bundled value cannot be substituted at
an invoke call. The issuer is likewise a capability, never identity, and can be
called only by finish after the closed gate. If implementation
introduces a separately named normalized newtype, that owner type replaces the
spelling in this signature without changing the boundary or identity bytes.
The closure receives the registry by value, normally as `|mut registry|`.
Consequently the same closure can legally call the consuming
`registry.finish_execution_v4(...)`; no move through `&mut`, placeholder,
`Default`, `Option` extraction, unsafe read or self-replacement is involved.
The higher-ranked execution brand prevents a registry, plan, ID, prepared
token, traversal seal or materiality seal from escaping as `R`.

Before `run` is called it performs all four initialization actions in section
0.2 as one transaction: constructs the analysis prefix, obtains and validates
Task5B's sealed execution binding, stages every request/catalog spelling and
commits the complete delta. Task5B's zero-I/O catalog staging covers both
catalog sets, nested registered material and captured Analysis BSL module
projections without returning an iterator, path, manifest key, witness, reader
or material handle. No registry is exposed if any initialization step fails;
there is no `catalog_spellings_committed` flag or later commit method. Every
registration/invoke/finality operation instead requires the registry to remain
valid and revalidates the same prefix-owned binding through the exact typed
authority dispatch at its specified boundary.

Application-derived artifacts use a second closed registry operation; callers
never borrow or mutate `artifact_spellings` directly:

```rust
impl ProviderInvocationRegistryV2<'_, '_> {
    pub(super) fn admit_application_artifact_spelling_v1(
        &mut self,
        source: &AtomicSourceIdentityV2,
        artifact: &ArtifactRef,
    ) -> Result<(), DiscoveryError>;

    fn require_provider_material_artifact_spellings_v1(
        &mut self,
        material: &ProviderGroupMaterialIdentityV2,
    ) -> Result<(), DiscoveryError>;

    fn require_atomic_group_artifact_spellings_v1(
        &mut self,
        group: &SemanticAtomicEvidenceGroupV2,
    ) -> Result<(), DiscoveryError>;
}
```

The exact Stage1-through-Stage5 producer calls this method inline for every raw
occurrence it already emits, before appending that occurrence to any query,
Support, association, traversal, mechanism, candidate or graph builder. The
method performs one `artifact_spellings.validate_occurrence(source, artifact)`;
it starts no collection, scan or second pass and introduces no new raw
cardinality limit. Existing producer/resource bounds and semantic grouping
rules remain unchanged: for example 4,097 byte-identical Support contributions
may still be visited and collapse to one subject. A same-source exact-different
occurrence invalidates the whole execution immediately and permits no query
registration, I/O, graph/report or receipt; any internal registry prefix is
unobservable and can never become an accepted execution projection.

Production call sites are statically closed to the five staged orchestration
builders. A RED inserts an occurrence before the call, skips the call, calls it
after canonicalization, or invokes it from any other production module and must
fail. This inline gate also covers complete mechanism/association artifacts
before either Stage 2 builder mutates its state. Task 7 owns these closed raw
ingress types in the exact owning sibling modules; their fields and constructors
remain private while the types/gates are visible only to the parent discovery
module:

```rust
pub(super) struct MechanismContributionV1 {
    key: MechanismKey,
    owners: Vec<ArtifactRef>,
    base_edges: Vec<EdgeIdentityV1>,
    entry_candidates: Vec<ArtifactRef>,
    evidence_ids: Vec<String>,
}

pub(super) struct MechanismAssociationContributionV1<'a> {
    key: &'a MechanismKey,
    root: &'a MaterialAssociationRootV2,
}

impl MechanismContributionV1 {
    pub(super) fn admit_complete_artifact_spellings_v1(
        &self,
        registry: &mut ProviderInvocationRegistryV2<'_, '_>,
        analysis_source: &AtomicSourceIdentityV2,
    ) -> Result<(), DiscoveryError>;
}

impl MechanismAssociationContributionV1<'_> {
    pub(super) fn admit_complete_artifact_spellings_v1(
        &self,
        registry: &mut ProviderInvocationRegistryV2<'_, '_>,
        analysis_source: &AtomicSourceIdentityV2,
    ) -> Result<(), DiscoveryError>;
}
```

The mechanism method destructures `Self`, `MechanismKey` and every
`EdgeIdentityV1` without `..`. It calls the registry's application-admission
operation for `key.entry`, `key.handler`, every owner, both endpoints of every
base edge and every entry candidate before the contribution reaches the
mechanism builder. Adding an artifact-bearing field therefore fails compilation
until the exhaustive gate is updated. Evidence IDs and `FlowKind` contain no
artifact and are excluded deliberately.

```rust
impl MaterialAssociationRootV2 {
    pub(super) fn require_complete_artifact_spellings_v1(
        &self,
        registry: &mut ProviderInvocationRegistryV2<'_, '_>,
    ) -> Result<(), DiscoveryError>;

    pub(super) fn write_identity_v2(&self, sink: &mut CanonicalIdentitySinkV4);

    pub(super) fn identity_token_v2(
        &self,
    ) -> CanonicalIdentityToken<MaterialAssociationRootIdentityV2>;
}

impl ProviderInvocationKeyV2 {
    pub(super) fn write_identity_v2(&self, sink: &mut CanonicalIdentitySinkV4);

    pub(super) fn identity_token_v2(
        &self,
    ) -> CanonicalIdentityToken<ProviderInvocationIdentityV2>;
}

pub(super) fn write_material_association_root_vector_v2(
    roots: &[MaterialAssociationRootV2],
    sink: &mut CanonicalIdentitySinkV4,
);

pub(super) struct CanonicalIdentitySinkV4 {
    bytes: Vec<u8>,
    // field, constructor, length and finalizer private to determinism.rs
}

pub(super) struct CanonicalIdentityToken<K> {
    encoded: Box<[u8]>,
    _kind: PhantomData<fn() -> K>,
    // fields/private constructor private to determinism.rs; Eq/Ord only
}

impl<K> PartialEq for CanonicalIdentityToken<K> {
    fn eq(&self, other: &Self) -> bool {
        self.encoded == other.encoded
    }
}

impl<K> Eq for CanonicalIdentityToken<K> {}

impl<K> PartialOrd for CanonicalIdentityToken<K> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl<K> Ord for CanonicalIdentityToken<K> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.encoded.cmp(&other.encoded)
    }
}

impl CanonicalIdentitySinkV4 {
    pub(super) fn append_u8(&mut self, value: u8);
    pub(super) fn append_u16be(&mut self, value: u16);
    pub(super) fn append_u32be(&mut self, value: u32);
    pub(super) fn append_bool(&mut self, value: bool);
    pub(super) fn append_digest32(&mut self, value: &Digest32);
    pub(super) fn append_string(&mut self, value: &str);
    pub(super) fn append_bytes_frame(
        &mut self,
        write: impl FnOnce(&mut CanonicalIdentitySinkV4),
    );
    pub(super) fn append_vector<T>(
        &mut self,
        values: &[T],
        write: impl FnMut(&T, &mut CanonicalIdentitySinkV4),
    );

    pub(super) fn append_platform_binding_v1(
        &mut self,
        binding: &PlatformCatalogExecutionBindingV1,
    );

    pub(super) fn append_provider_material_artifact_set_v2(
        &mut self,
        set: &ProviderMaterialArtifactSetV2,
    );
}

pub(super) fn canonical_identity_token_v4<K>(
    domain: &'static str,
    write: impl FnOnce(&mut CanonicalIdentitySinkV4),
) -> CanonicalIdentityToken<K>;

// Zero-sized private marker kinds; no value constructor or serialization.
pub(super) enum ProviderInvocationIdentityV2 {}
pub(super) enum MaterialAssociationRootIdentityV2 {}
pub(super) enum MaterialAssociationMapIdentityV2 {}
pub(super) enum RawProviderOutcomeIdentityV2 {}
pub(super) enum EffectiveProviderInvocationIdentityV3 {}
pub(super) enum EvidenceAdmissionGapIdentityV3 {}
pub(super) enum EffectiveGapIdentityV3 {}
pub(super) enum TraversalGapIdentityV3 {}
pub(super) enum TraversalGapSetIdentityV3 {}
pub(super) enum AnalysisExecutionSnapshotIdentityV4 {}

// Module-private inspection result; it never leaves ports.rs.
enum CurrentRootRelationV2<'a> {
    Invocation {
        invocation: &'a ProviderInvocationKeyV2,
    },
    SourceGroup {
        invocation: &'a ProviderInvocationKeyV2,
        source: &'a AtomicSourceIdentityV2,
    },
    Material {
        invocation: &'a ProviderInvocationKeyV2,
        material: &'a ProviderGroupMaterialIdentityV2,
    },
    AtomicGroup {
        invocation: &'a ProviderInvocationKeyV2,
        group: &'a SemanticAtomicGroupIdV2,
    },
}

struct RegistryFinishValidationStateV4 {
    expected_finished_keys:
        BTreeSet<CanonicalIdentityToken<ProviderInvocationIdentityV2>>,
    seen_invocation_map_keys:
        BTreeSet<CanonicalIdentityToken<ProviderInvocationIdentityV2>>,
    seen_map_roots:
        BTreeSet<CanonicalIdentityToken<MaterialAssociationRootIdentityV2>>,
    seen_effective_invocation_keys:
        BTreeSet<CanonicalIdentityToken<ProviderInvocationIdentityV2>>,
    seen_admission_owners: BTreeMap<
        CanonicalIdentityToken<EvidenceAdmissionGapIdentityV3>,
        BTreeSet<CanonicalIdentityToken<ProviderInvocationIdentityV2>>,
    >,
    seen_provider_effective_classes:
        BTreeSet<(ProviderInvocationRegistrySlotIdV2, u16)>,
    seen_provider_raw_gaps: BTreeSet<(ProviderInvocationRegistrySlotIdV2, u16)>,
    seen_admission_effective_identities:
        BTreeSet<CanonicalIdentityToken<EffectiveGapIdentityV3>>,
    seen_effective_gap_candidates:
        BTreeSet<CanonicalIdentityToken<EffectiveGapIdentityV3>>,
    expected_traversal_gap_identities:
        BTreeSet<CanonicalIdentityToken<TraversalGapIdentityV3>>,
    seen_traversal_gap_identities:
        BTreeSet<CanonicalIdentityToken<TraversalGapIdentityV3>>,
    // typed identity tokens are equality/order decorations only; finalization
    // independently re-encodes every typed owner before comparing relations
}

pub(super) struct RegistryFinishValidationV4<
    'registry,
    'execution,
    'context,
> {
    registry: &'registry mut ProviderInvocationRegistryV2<'execution, 'context>,
    state: RegistryFinishValidationStateV4,
    // fields/constructor/finalizer are private to ports.rs; no Clone/serde/
    // Debug/Display/raw projection and no token/identity bytes of its own
}

pub(super) struct RegistryProviderOutcomeConstructionAuthorityV4<'finish> {
    _brand: PhantomData<&'finish mut ()>,
}
pub(super) struct RegistryEffectiveInvocationConstructionAuthorityV4<'finish> {
    _brand: PhantomData<&'finish mut ()>,
}
pub(super) struct RegistryAdmissionGapConstructionAuthorityV4<'finish> {
    _brand: PhantomData<&'finish mut ()>,
}
pub(super) struct RegistryEvidenceGapLimitConstructionAuthorityV4<'finish> {
    _brand: PhantomData<&'finish mut ()>,
}
pub(super) struct RegistryProviderEffectiveGapConstructionAuthorityV4<'finish> {
    _brand: PhantomData<&'finish mut ()>,
}
pub(super) struct RegistryAdmissionEffectiveGapConstructionAuthorityV4<'finish> {
    _brand: PhantomData<&'finish mut ()>,
}
pub(super) struct RegistryFinalMaterialityConstructionAuthorityV4<'finish> {
    _brand: PhantomData<&'finish mut ()>,
}
pub(super) struct RegistryFinalReportProjectionConstructionAuthorityV4 {
    _private: (),
}
pub(super) struct EarlyTerminalReportConstructionAuthorityV4 {
    _private: (),
}

pub(super) struct FinishedAnalysisExecutionProjectionV4 {
    analysis_identity_prefix: AnalysisIdentityPrefixV4,
    provider_rollups: Vec<ScopedProviderRollupSnapshotV2>,
    effective_provider_invocations: Vec<EffectiveProviderInvocationV3>,
    application_invocation_trace: Vec<ApplicationInvocationTraceV2>,
    material_associations: MaterialAssociationMapV2,
    analysis_evidence_gap_limit: Option<EvidenceAdmissionGapV3>,
    effective_gap_projection: Vec<EffectiveGapV3>,
    traversal_gaps: TraversalGapSetV3,
    final_discovery_materiality: FinalDiscoveryMaterialityV4,
    // private fields; no Clone/serde/raw constructor, field/key/token accessor
}

impl FinishedAnalysisExecutionProjectionV4 {
    pub(super) fn write_analysis_identity_payload_v4(
        &self,
        sink: &mut CanonicalIdentitySinkV4,
    );

    fn write_execution_snapshot_tail_v4(
        &self,
        sink: &mut CanonicalIdentitySinkV4,
    );
}

impl<'execution, 'context>
    ProviderInvocationRegistryV2<'execution, 'context>
{
    fn require_current_root_v2<'a>(
        &'a self,
        root: &'a MaterialAssociationRootV2,
    ) -> Result<CurrentRootRelationV2<'a>, DiscoveryError>;

    fn validate_current_association_root_v2(
        &mut self,
        root: &MaterialAssociationRootV2,
    ) -> Result<(), DiscoveryError>;

    pub(super) fn record_stage_v2(
        &mut self,
        invocation: &RecordedInvocationIdV2<'execution>,
        stage: ProviderInvocationStageV2,
    ) -> Result<(), DiscoveryError>;

    pub(super) fn preview_application_admission_v3<'lookup>(
        &'lookup self,
    ) -> Result<
        ApplicationAdmissionPreviewV3<'lookup, 'execution, 'context>,
                DiscoveryError>;

    pub(super) fn register_traversal_requirement_v3(
        &mut self,
        requirement: TraversalRequirementSpecV3,
    ) -> Result<TraversalRequirementIdV3<'execution>, DiscoveryError>;

    pub(super) fn complete_traversal_requirement_v3(
        &mut self,
        requirement: TraversalRequirementIdV3<'execution>,
    ) -> Result<(), DiscoveryError>;

    pub(super) fn gap_traversal_requirement_v3(
        &mut self,
        requirement: TraversalRequirementIdV3<'execution>,
    ) -> Result<(), DiscoveryError>;

    pub(super) fn prepare_discovery_finality_v4(
        &mut self,
    ) -> Result<PreparedDiscoveryFinalityV4<'execution>, DiscoveryError>;

    pub(super) fn project_next_finality_association_v4(
        &mut self,
        prepared: &PreparedDiscoveryFinalityV4<'execution>,
    ) -> Result<Option<ValidatedFinalityAssociationProjectionV4<'execution>>,
                DiscoveryError>;

    pub(super) fn record_finality_association_v4(
        &mut self,
        prepared: &PreparedDiscoveryFinalityV4<'execution>,
        applied: AppliedFinalityAssociationProjectionV4<'execution>,
    ) -> Result<(), DiscoveryError>;

    pub(super) fn seal_discovery_materiality_plan_v4(
        &mut self,
        prepared: &PreparedDiscoveryFinalityV4<'execution>,
        material_associations: &MaterialAssociationMapV2,
        plan: UnsealedDiscoveryMaterialityPlanV4,
    ) -> Result<SealedDiscoveryMaterialityPlanV4<'execution>, DiscoveryError>;

    pub(super) fn finish_execution_v4(
        self,
        prepared: PreparedDiscoveryFinalityV4<'execution>,
        material_associations: MaterialAssociationMapV2,
        materiality_plan: SealedDiscoveryMaterialityPlanV4<'execution>,
    ) -> Result<FinishedAnalysisExecutionProjectionV4, DiscoveryError>;
}

impl RegistryFinishValidationV4<'_, '_, '_> {
    pub(super) fn validate_association_entry_v4(
        &mut self,
        root: &MaterialAssociationRootV2,
        scopes: &[ConclusionScope],
    ) -> Result<(), DiscoveryError>;

    fn count_effective_gap_material_subjects_v4(
        &self,
        scopes: &[&EvidenceAdmissionScopeV3],
    ) -> Result<u32, DiscoveryError>;
}
```

`CanonicalIdentityToken<K>` compares the complete private encoded byte string,
not a digest, pointer or display value; it has no Clone/serde/Debug/Display or
byte getter. `canonical_identity_token_v4` is statically whitelisted to the
named owner `identity_token_*` methods only. The sink's primitive operations
cannot append an arbitrary slice. Its two typed upstream adapters temporarily
lend only its private `Vec<u8>` to Task5B writers: the binding adapter is called
solely by `AnalysisIdentityPrefixV4` and the material-set adapter solely by
`EvidenceAdmissionScopeV3`. Neither adapter can be chained, aliased, used by a
callback/function pointer or called a second time in one owner encoding.

The association method first admits both artifacts in its own `MechanismKey`;
it never relies on a prior mechanism contribution. It then calls the opaque
root's sole `pub(super) require_complete_artifact_spellings_v1`. That method is
defined in `ports.rs` beside the private inner enum and exhaustively matches all
four variants without `..`: `Invocation` and `SourceGroup` contain no artifact;
`Material` calls only the private
`require_provider_material_artifact_spellings_v1`; `AtomicGroup` first resolves
the compact root ID through its exact current Finished invocation to the one
full `SemanticAtomicEvidenceGroupV2`, then calls only the private
`require_atomic_group_artifact_spellings_v1` with that owner. Those registry
helpers delegate to the Task5B owner checks, so neither association
code nor any other sibling inspects a key or imported private material. The
root also exposes one sealed `write_identity_v2` only to `association.rs`; it
encodes the exact section-3.4 tags/fields by exhaustive private-inner-enum match
and reveals no key, variant or member. Adding a fifth inner variant therefore
fails both exhaustive operations until they are updated.
`model.rs` never calls that per-root writer. Its admission/effective DTO writers
may call only `write_material_association_root_vector_v2`, whose exhaustive
implementation remains in `ports.rs` and appends a complete canonical root
vector without returning a key, variant, member or intermediate byte buffer.
Only after the complete gate succeeds may `MaterialAssociationMapBuilderV2`
insert, union, sort or deduplicate. Neither gate returns an iterator, canonical
contribution or builder handle, and neither constructs an identity key before
all of its raw occurrences have been admitted or required.

An opaque root is canonical immutable identity, not retained membership
authority. Before final map acceptance,
`MaterialAssociationMapV2::validate_for_registry_finish_v4` walks its own
private canonical entries and calls only
`RegistryFinishValidationV4::validate_association_entry_v4` for each root/scope
pair. That facade calls the registry-private
`validate_current_association_root_v2`; its private helper
matches the private enum and resolves the exact key in this execution's
registry, requires the entry to be `Finished`, revalidates `SourceGroup` and
query-time `Material` through the exact moved owner capability, revalidates
returned-only `Material` and every `AtomicGroup` through the exact current
recorded outcome, and repeats the complete spelling check. `Invocation` still
requires the exact current Finished entry. The wrapper exposes only `Result`;
`CurrentRootRelationV2` is module-private, non-Clone, non-serde and has no
`Debug`/`Display`, so no sibling obtains a key, variant, member or capability.
The facade records both the typed full-root token and, for Invocation roots,
the typed invocation token in its private finish state. Thus
`finish_execution_v4` proves the exact Finished/Invocation-root bijection and
requires every provider/admission finality root to occur in the final map with
a nonempty scope, without reading an `association.rs` field. Any failure first
invalidates the execution
and then preserves the exact
`provider_query_association_mismatch`, `provider_outcome_association_mismatch`
or spelling reason selected by that failed authority.

This current-registry revalidation is the replay boundary. A root copied from
another execution is rejected when the current entry is absent/unfinished or
its current query authority, outcome membership or spelling differs. If all of
those current authorities independently prove the exact same canonical root,
accepting its byte-equal immutable value is safe and deliberate; the old value
does not act as authority. Execution nonce/slot therefore remain outside the
root, equality/order and every canonical byte.

Admission is not recorded from caller-built gaps. The one private admission
derivation reads all current Finished raw atomic groups, applies the frozen
per-port/global orders and retained `request.limits.max_evidence`, and creates exactly
one bucket for every occupied `(reason, port)`. `prepare_discovery_finality_v4`
stores those buckets, exact retained-by-slot decisions, provider gap classes
and their complete ordered association requirements in the registry; its token
contains only execution nonce/generation/brand. A requirement is projected
only at `next_requirement_index`, which is first installed as the sole
outstanding index; it is associated, then consumed exactly once before the
outstanding slot clears and the index advances. Foreign, skipped, duplicate or
reordered issuance/consumption invalidates the execution.

Consuming `finish_execution_v4` alone constructs
`RegistryFinishValidationV4` around the registry and a fresh empty private
state. It rederives the prepared decisions from current raw outcomes, validates
the complete final map, branded traversal seal and branded materiality plan,
reconstructs all model DTOs through their sealed owner constructors, and records
typed identity tokens for the
Finished/map/effective/admission/provider-class relations. Its private
finalizer requires every key bijection, map-root presence, admission owner
fan-out and raw-gap-to-effective-class surjection before it evaluates the
256/2,000 sentinel thresholds. It returns only `Result<()>`; it exposes no
reusable witness, key/root/token projection or caller-visible constructor.

Every typed query registration also revalidates all artifact members of the
already owner-validated smart query against the committed registry before it
performs duplicate-key lookup or allocates a slot. Metadata/Form/Support call
the exact Task5B-owned
`validate_committed_artifact_spellings_v1`; Definition/CallGraph call the exact
Task6-owned method of the same name; CodeSearch has no artifact member. These
sealed owner methods call only `registry.require_occurrence` over their private
complete members and expose no iterator/member/delta. This final check cannot
replace the inline application gate or the owner constructor's internal
collision check; it proves only that the canonical typed query did not
substitute or omit registration of a spelling between those gates. None of
these application validations enters provider query bytes, query digest,
provider identity or association scope.

The two association overloads take `&mut self` only to enforce fail-closed
execution state. They delegate the actual read check solely to Task5B's exact
sealed owner methods from section 3.10.3 with `&self.artifact_spellings`; those
owner checks remain read-only and cannot add an occurrence. On any owner-check
error the wrapper first sets `execution_invalidated=true`, maps the exact
closed reason, and only then returns `Err`. Thus even a caller that catches or
ignores the result cannot register, invoke, obtain `recorded()`, prepare or call
`finish_execution_v4`, or expose a map/report/receipt. The wrappers expose neither the
registry nor a Task5B private member.
The application-admission spelling operation is `pub(super)` solely for the
five whitelisted sibling modules `use_case`, `association`, `traversal`,
`mechanisms` and `evidence_graph`; the two material/group `require_*` helpers
are private to `ports.rs` and are reachable only through the opaque root's
spelling gate called from `association`. The registry lifecycle boundary
in `ports.rs` is also explicitly `pub(super)`: the
generative `with_provider_invocation_registry_v2`, the five frozen
non-Definition `register_*`/`invoke_*` families, the one Definition family
only after section 7.1 is approved, `recorded`, `record_stage_v2`,
`preview_application_admission_v3`, `register_traversal_requirement_v3`,
`complete_traversal_requirement_v3`, `gap_traversal_requirement_v3`,
`prepare_discovery_finality_v4`, `project_next_finality_association_v4`,
`record_finality_association_v4`,
`seal_discovery_materiality_plan_v4` and
consuming `finish_execution_v4`. They are never bare private `fn` (which a sibling cannot
call), `pub(crate)` or public. Static call-site tests allow traversal
register/complete/gap only from `traversal`, all other lifecycle operations
only from `use_case`; allow plan/finished-handle root constructors,
root identity/spelling methods and finish-
context association-entry validator only from `association`; and allow only
`admit_application_artifact_spelling_v1` from all five named siblings. The
three checked materiality-input factories are callable only from
`projection.rs`; their private fields remain readable only in `ports.rs`.

Only `use_case` calls finality project-next/record/materiality-seal operations; only
`association` calls
`ValidatedFinalityAssociationProjectionV4::apply_finality_associations_v4`
during the short sink borrow before the same projection is consumed by the
registry. Only the exhaustive decision boundaries in `traversal.rs` call the
three traversal requirement operations; no sibling accumulates a second
traversal vector or can leave a registered decision unresolved.
All admission/effective-invocation/effective-gap constructors accept their
matching nonforgeable `Registry*ConstructionAuthorityV4<'finish>` by value.
Those authority fields and minting expressions are private to
`ports.rs::finish_execution_v4`; the exact `pub(super)` model factory is its
sole static-whitelisted consumer and cannot construct an authority itself.
There is no production call
site for a sentinel constructor: the QueryWide ownerless value is minted only
inside the private finish branch after complete candidate validation.

The private `require_current_root_v2`,
`validate_current_association_root_v2`,
finish-context constructor/finalizer and `CurrentRootRelationV2` have no caller
or name outside `ports.rs`. The per-root writer remains callable only from the
association map writer; the root-vector writer only from the model admission/
effective-gap writers; the key writer only from the exact ports raw/root/trace
and model effective-invocation/admission/effective-gap writers. The outcome/map/model/
projection/snapshot writer chain is exactly section 4.2. Every other production
caller, direct private-field access, second wrapper or call after its producer
starts canonicalization rejects statically. No validation witness, finish
context or finality projection has another constructor, projection,
Clone/serde/Debug/Display implementation or caller.
The determinism-owned finished-seal hash finalizer has exactly the ports-owned
snapshot constructor as its caller; the stored `AnalysisId` is moved, never
recomputed, by the consuming report projection.

The finish-context facade is equally exact: association-entry validation is
called only by `MaterialAssociationMapV2::validate_for_registry_finish_v4`.
The traversal set and every internally built model value similarly feed their
private typed envelopes only from owner methods invoked by
`finish_execution_v4`; no facade accepts caller-selected key, digest, retained
prefix, gap, scope, root vector, sentinel option or candidate vector.

`ProviderInvocationExecutionNonceV2` and
`ProviderInvocationRegistrySlotIdV2` are module-private non-serde newtypes with
no consumer-visible raw/string/integer constructor or projection. The slot ID
is internally `Copy + Clone + Eq + Ord` solely for checked registry indexing;
the plan and recorded ID themselves remain non-Clone. Only registry construction
can mint a process-unique nonzero nonce through one private checked atomic
counter (overflow is a pre-execution error and the nonce never enters receipts,
ordering or identity), and only registration can allocate a monotonically fresh
checked-`u32` slot ID and append the matching `Staged` entry. Plan consumption
carries those exact registry-issued values into the returned recorded ID rather
than accepting caller values. Query-kind and entry-state enums are private,
non-serde and never enter any public/canonical identity; the key and raw outcome
already have their separately frozen encoders.

Registration below closes the five non-Definition typed families:
`MetadataCompositeQueryV2`, `FormSourceSetQueryV2`, `CodeSearchQuery`,
`CallGraphQuery` and `SupportStateQueryV2`. The Definition registration shape
is intentionally absent until section 7.1 is approved. There is no
generic `(port, Digest32)` ingress. Each overload obtains exactly one
corresponding owner-minted association authority, checks that its closed
variant implies that overload's exact port and that its `query_digest()` is
byte-equal to the smart query's canonical typed digest, then copies those digest
bytes into the private key. It records its query-type/port discriminator in the
private entry and returns a non-cloneable
`RegisteredProviderInvocationPlanV2<'execution>` containing the exact key, slot
ID, execution nonce, owned association authority and invariant brand. Before allocating or
mutating either collection, registration performs an atomic
`key_to_slot` lookup and rejects an already registered exact `(port,
query_digest)` with the closed pre-I/O duplicate-plan contract error. It never
returns a second plan, reuses a staged/finished slot implicitly or performs
provider I/O. The application reuses an already recorded invocation only via
its owned `RecordedInvocationIdV2` and checked `recorded(&id)` path. No
constructor accepts a digest string or rehashes/re-encodes the query. The exact
duplicate rejection is nonretryable
`DiscoveryError::DuplicateProviderInvocationPlan` with stable reason
`duplicate_provider_invocation_plan`; it exposes no raw digest/slot in public
output.

Their normative shapes are below. Query-internal context borrow lifetimes are
explicit and equal the registry's retained `'context`; the compile fixtures run
with `#![deny(elided_lifetimes_in_paths)]`:

```rust
impl<'execution, 'context>
    ProviderInvocationRegistryV2<'execution, 'context>
{
    pub(super) fn register_metadata(&mut self, q: &MetadataCompositeQueryV2<'context>)
        -> Result<RegisteredProviderInvocationPlanV2<'execution>, DiscoveryError>;
    pub(super) fn register_form(&mut self, q: &FormSourceSetQueryV2<'context>)
        -> Result<RegisteredProviderInvocationPlanV2<'execution>, DiscoveryError>;
    pub(super) fn register_code_search(&mut self, q: &CodeSearchQuery)
        -> Result<RegisteredProviderInvocationPlanV2<'execution>, DiscoveryError>;
    pub(super) fn register_call_graph(&mut self, q: &CallGraphQuery)
        -> Result<RegisteredProviderInvocationPlanV2<'execution>, DiscoveryError>;
    pub(super) fn register_support(&mut self, q: &SupportStateQueryV2<'context>)
        -> Result<RegisteredProviderInvocationPlanV2<'execution>, DiscoveryError>;
}
```

At every observable registry boundary, `entries.len() == key_to_slot.len()`;
each entry's checked slot ID is its one vector position, each map key equals
that entry's complete key, and the map value equals that entry's slot ID. The
two collections are updated transactionally only after checked `usize -> u32`
allocation succeeds. Duplicate key, divergent key/slot, missing entry, reused
slot ID, arithmetic overflow or a second state transition rejects; no
deduplication, late finish-only repair or provider call is permitted.

The registry also exposes the five matching closed typed **invoke** overloads:
`invoke_metadata`, `invoke_form`, `invoke_code_search`, `invoke_call_graph` and
`invoke_support`. The approved Definition boundary will add its exact distinct
surface here. Each takes `&mut self`, consumes only
the corresponding registered plan, borrows the same imported typed query, the
matching provider port, and internally creates the exact owner-required
`EvidenceExecutionContext` from its retained workspace/context/snapshot/reader.
No invoke argument can substitute any of those four capabilities. Before I/O
it validates that the plan's
private slot ID resolves by checked indexing to the exact `Staged` entry, that
the entry has that overload's query-type/port discriminator and byte-equal key,
that the plan nonce and brand belong to this execution, and that the imported
query's canonical typed `query_digest()` is byte-for-byte equal to the private
planned key. It also requires the plan's closed association-authority variant,
enum-implied port and owner digest to match that same typed overload/query. It
then itself performs exactly one matching provider call. No
closure, callback or caller-supplied outcome is accepted by an invoke overload.

Here byte-for-byte equality of the exact owner-issued typed 32-byte
`query_digest()` carried by `ProviderInvocationKeyV2` proves invocation/cache
identity only; it does not prove source/material/group membership. The closed
owner-minted association authority proves query membership separately. Task 7
neither requests nor reconstructs upstream query payload bytes. The overload's
static query type plus its private query-type/port slot discriminator and
authority variant prevent a cross-port/type substitution, while the exact 32
bytes reject a different query of the same port. This preserves section 2's
rule that Task 7 imports only typed digest accessors and sealed capabilities.

```rust
impl<'execution, 'context>
    ProviderInvocationRegistryV2<'execution, 'context>
{
    pub(super) fn invoke_metadata(&mut self, p: RegisteredProviderInvocationPlanV2<'execution>,
        q: &MetadataCompositeQueryV2<'context>, provider: &dyn MetadataCatalogPort)
        -> Result<RecordedInvocationIdV2<'execution>, DiscoveryError>;
    pub(super) fn invoke_form(&mut self, p: RegisteredProviderInvocationPlanV2<'execution>,
        q: &FormSourceSetQueryV2<'context>, provider: &dyn FormInspectionPort)
        -> Result<RecordedInvocationIdV2<'execution>, DiscoveryError>;
    pub(super) fn invoke_code_search(&mut self, p: RegisteredProviderInvocationPlanV2<'execution>,
        q: &CodeSearchQuery, provider: &dyn CodeSearchPort)
        -> Result<RecordedInvocationIdV2<'execution>, DiscoveryError>;
    pub(super) fn invoke_call_graph(&mut self, p: RegisteredProviderInvocationPlanV2<'execution>,
        q: &CallGraphQuery, provider: &dyn CallGraphPort)
        -> Result<RecordedInvocationIdV2<'execution>, DiscoveryError>;
    pub(super) fn invoke_support(&mut self, p: RegisteredProviderInvocationPlanV2<'execution>,
        q: &SupportStateQueryV2<'context>, provider: &dyn SupportStatePort)
        -> Result<RecordedInvocationIdV2<'execution>, DiscoveryError>;
}
```

The provider response owns provider name/version; neither the registered plan
nor any pre-call association contains it. Inside each typed invoke overload,
the returned Successful/Unavailable/Failed raw `ProviderOutcome` first passes
only the closed O(1) cardinality/basic terminal-envelope checks needed to
identify its variant and safely borrow typed fields. Before any per-element
owner response/completeness validation, the overload calls the Task5A-owned
sealed `stage_complete_raw_artifact_spellings_v1` against the execution
registry. A spelling error first invalidates the entry/execution and returns
the fixed Task7 reason; therefore a response containing both a cross-execution
collision and a missing planned subject reports the spelling invariant.

The still-uncommitted delta is then borrowed by the exact owner-defined
artifact-conservative response/completeness validator. Every artifact is
required against baseline plus delta before its first semantic comparison;
only then may the validator construct one `CollectedProviderOutcome`.
Immediately afterward the shared sealed collected-outcome check exhaustively
validates records, gaps, semantic/atomic groups and nested material against the
same staged authority. Any semantic/completeness/derived-artifact failure
discards both raw/collected outcomes and delta, sets the entry to `Invalidated`
and `execution_invalidated=true`, and returns no recorded ID.

Only after those checks does the registry atomically record the collected raw
outcome, its exact provider identity and the v6
same-provider-name/version-per-port constraint while consuming the unchanged
delta through `commit_staged_v1` and transitioning the one resolved entry from
`Staged` to `Finished`; the same transaction moves the consumed plan's exact
non-cloneable association authority into that `Finished` entry, and no second
transition is legal. The resolved slot,
provider constraint and transition are fully validated before the
infallible-in-this-exclusive-borrow commit section, so no intermediate state is
observable. On an impossible stale-delta result the delta and outcomes are
discarded, the entry/execution and staged association roots are invalidated,
and no recorded ID is minted. The raw `record_terminal` primitive is private to
the five frozen overloads and the eventual approved Definition overload, and is
unreachable from Task 7 orchestration, tests and
every association builder. In particular there is no caller-visible
`record_terminal`, generic invoke, outcome callback or way to submit a
precomputed/swapped/empty-Complete `CollectedProviderOutcome`.

There is no caller-visible spelling commit, cached-outcome ingress,
`record_terminal`, generic invoke or raw-outcome callback. A provider adapter
that materializes a cache hit returns it through the same typed invoke and
transaction; later in-execution reuse can only reborrow the already committed
recorded ID.

If the one returned response fails its owner-defined port/query/completeness
contract (including a Complete response omitting a planned same-port subject),
the invoke overload returns the exact provider-contract error, invalidates that
staged entry and roots, and makes the execution unfinishable; it does not record
the malformed response, synthesize an Unavailable/Failed terminal, retry, or
issue an ID. This post-call failure remains distinguishable from the pre-I/O
zero-call/zero-prefix semantic-capability failures below, while neither can
leak a partial finished snapshot.

Every terminal Successful, Unavailable or Failed typed invocation returns an
opaque owned `RecordedInvocationIdV2<'execution>`. It contains only the private
finished slot ID, execution nonce and invariant brand; it exposes no port, query
digest, raw bytes or constructor and is neither serde nor forgeable. The sole
post-response lookup is
`registry.recorded(&RecordedInvocationIdV2<'execution>)`. It checks the same
execution nonce/brand, resolves a live in-range slot ID to an entry and requires
that entry's `Finished` state, then returns a short-lived
`RecordedProviderInvocationHandleV2<'lookup, 'execution>` borrowing the exact
key/provider/outcome and the exact moved query-association authority in that
entry. It does not project a gap class, finality requirement, nonce or slot.
There is no lookup by
`ProviderInvocationKeyV2`, digest, port, numeric slot or serialized ID. Later
cached-traversal and mechanism state stores the owned recorded ID, drops every
lookup handle before another mutable registry operation, and reborrows a handle
only for one association/trace update.

```rust
impl<'execution, 'context>
    ProviderInvocationRegistryV2<'execution, 'context>
{
    pub(super) fn recorded<'lookup>(
        &'lookup self,
        id: &RecordedInvocationIdV2<'execution>,
    ) -> Result<RecordedProviderInvocationHandleV2<'lookup, 'execution>,
                DiscoveryError>;
}
```

A pre-I/O query/semantic/capability error in a typed invoke overload aborts the
consumed staged plan and its staged associations together and returns the
owner-defined zero-prefix execution error; it cannot be represented as a
completed raw invocation and cannot leave an issued recorded ID. `finish_execution_v4`
rejects an unconsumed plan, a second terminal state, a foreign-execution or
foreign-slot recorded ID, a raw invocation with no typed plan, or staged
associations with no terminal raw invocation. Thus the finished execution has
a strict bijection:

```text
one committed typed invocation plan and consumed matching typed invoke
<-> one internally recorded raw terminal invocation
<-> one registry-issued recorded ID
<-> one exact association authority moved plan -> Finished entry
<-> one Invocation association root
```

`ProviderGroupMaterialIdentityV2` and `SemanticAtomicGroupIdV2` are imported
from the accepted Task 5B v7 shared grouping boundary. The `AtomicGroup` variant
uses that type's complete canonical group-key bytes, including the source-free
cluster digest; a caller-selected digest, partial primary subject or display
spelling cannot substitute.

Stable root tags are the private inner enum's declaration order 1..=4.
Conclusion tags remain
Request=1, Proposal=2, Mechanism=3. `NormalizedProposalIdV1` is the exact
normalized, validated request proposal identity defined above. `MechanismKey`
uses the exact Task 7 v6 `MechanismFamilyV1` plus entry/handler key. Neither is
a provider type.

### 3.2 Construction and validation

`MaterialAssociationMapBuilderV2` is the sole mutable builder. Its pre-I/O
methods accept a short borrow of a current-execution
`RegisteredProviderInvocationPlanV2<'execution>`, a typed root payload and a
scope, obtain the opaque root only from the plan's sealed constructor and union
byte-equal duplicates. The builder itself cannot construct/match a root or
extract a key. The borrow ends before
the registry's matching typed invoke overload consumes the plan.
Post-response `Material`/`AtomicGroup` contributions accept only a short-lived
`RecordedProviderInvocationHandleV2<'lookup, 'execution>`, never the consumed
plan or the owned recorded ID itself. Later-origin and proven-Mechanism state
stores `RecordedInvocationIdV2<'execution>` and obtains the handle solely
through the registry's checked `recorded(&id)` borrow. Pre-I/O
Invocation/SourceGroup/query-Material methods accept only the live plan;
post-I/O returned-gap/group methods accept only the recorded handle, so the
phase boundary is type-checked without holding an immutable registry borrow
across another typed invocation. `finish_execution_v4` jointly consumes the finished
invocation registry and emits canonical immutable entries only after the typed
plan/typed invoke/raw/recorded-ID/Invocation-root bijection above passes. This
union is the deliberate application boundary at which two equal proposal
contributions collapse for provider work while retaining both scopes.

During that joint finish, the immutable map's owner-side validator presents
each final opaque root/scope pair to the finish context's sealed association
facade. Neither the builder nor `finish` inspects or decodes the root itself.
The facade's registry-private current-root pass closes both a
stale cloned-root replay and any internal builder substitution; catching its
error leaves the execution invalidated and no map/snapshot/report/receipt can
escape.

The plan does not expose a separable validate-then-construct operation. It
exposes only `pub(super)` sealed root constructors to `association.rs`:

```rust
impl<'execution> RegisteredProviderInvocationPlanV2<'execution> {
    pub(super) fn invocation_root_v2(&self) -> MaterialAssociationRootV2;
    pub(super) fn source_group_root_v2(
        &self,
        source: &AtomicSourceIdentityV2,
    ) -> Result<MaterialAssociationRootV2, DiscoveryError>;
    pub(super) fn query_material_root_v2(
        &self,
        material: &ProviderGroupMaterialIdentityV2,
    ) -> Result<MaterialAssociationRootV2, DiscoveryError>;
}
```

Each checked constructor first requires plan key/port/digest coherence, calls
the exact owner capability and then constructs the root with the plan's private
key itself; it never accepts or returns a caller key/member list. The recorded
handle's direct root-construction surface exposes only
`returned_material_root_v2` and `atomic_group_root_v2`; its separate provider-
gap projection remains opaque and exact:

```rust
impl<'lookup, 'execution>
    RecordedProviderInvocationHandleV2<'lookup, 'execution>
{
    pub(super) fn returned_material_root_v2(
        &self,
        material: &ProviderGroupMaterialIdentityV2,
    ) -> Result<MaterialAssociationRootV2, DiscoveryError>;
    pub(super) fn atomic_group_root_v2(
        &self,
        group: &SemanticAtomicEvidenceGroupV2,
    ) -> Result<MaterialAssociationRootV2, DiscoveryError>;
}
```

Those methods require finished key/authority coherence
and validate the exact requested material/group through the sealed recorded
typed outcome; a material already proven as a query member may use the same
authority path, but an AtomicGroup always requires exact returned-outcome
membership. Thus a pre-I/O SourceGroup/query-Material root and a post-I/O gap/
group root cannot be constructed from key/digest equality alone, and a
synthetic AtomicGroup built from otherwise valid query material still rejects.
The group argument is the full lossless owner; the handle validates it against
its stored full-group slice and stores only its compact canonical ID in the
returned root. Application then ends the handle borrow, passes that owned root
through the mutable spelling gate, and only afterward inserts it into the map.
The association builder never walks authority/outcome internals, reparses
display text, or accepts a digest/port/member vector from the caller. Every
failed root constructor returns before builder mutation; `finish_execution_v4`
independently rejects any resulting missing required Invocation/material/group
association, so catching the error cannot turn an incomplete map into an
accepted projection.

Provider-gap projection is equally closed, but it is not a recorded-handle API.
Raw gap identity retains optional source location while `EffectiveGapV3`
deliberately does not. During `prepare_discovery_finality_v4`, the registry maps
every raw gap by QueryWide -> Invocation, SourceSetWide -> its one authorized
SourceGroup, and Artifacts -> the complete canonical Material-root set; groups
only byte-equal `(invocation, provider, stable reason, admission scope, roots)`
after location erasure; retains every member raw-gap index; and sorts unique
classes by typed full-identity token. Exact duplicate raw rows remain illegal,
but location-distinct raw rows may intentionally share one class. This is an
exact surjection from raw rows to effective rows, not silent raw-history
deduplication.

Provider classes and admission buckets are then merged into the one ordered
`RegistryFinalityAssociationRequirementV4` vector described in section 0.2.
The common finality projection is consumed only through the opaque association
instruction sink; `association.rs` cannot match an instruction or retain a
root/scope vector. The registry then consumes the applied token and records the
exact next requirement. No caller obtains an `EffectiveGapV3`, owner, provider, reason,
scope, raw index, class index, location or candidate vector. Finish rederives
the classes and requires every raw gap index to belong to exactly one class,
every class/bucket requirement to have been consumed once in order and every
projected root to occur in the final map before internally constructing model
rows.

Rules are exact:

1. Every entry has a nonempty, sorted, unique scope vector. Absence of a
   scheduled typed query creates no invocation plan or association. A scheduled
   non-Definition typed query is never omitted merely because its accepted
   owner contract permits an empty canonical member vector. Definition empty-
   plan invocation cardinality is deliberately unresolved only at section 7.1
   and becomes exact with the approved A/B/C choice. Task 7 never invents an
   invocation without that owner-approved scheduled unit.
2. An execution with no provider invocation may use the explicit empty map.
   Every actual provider invocation has exactly one `Invocation` root with the
   union of the application conclusions that scheduled it.
3. `SourceGroup` is legal only for an exact group inside that query: an
   internal Metadata composite source, the exact Analysis BSL/Form source, or
   one exact Support source group. A source with equal display name but unequal
   role/root/kind/format/mapping is rejected.
4. `Material` is legal only for a typed query member, a validated provider gap
   material member, or an actual complete returned group's material identity
   under that invocation. It never contains a conclusion scope.
5. `AtomicGroup` is added only after the accepted classifier validates a
   complete group returned by that exact invocation. The group may appear under
   several invocation roots; each owner association remains explicit.
6. Every retained or admission-dropped group that can affect a public fact,
   edge, mechanism, candidate, proposal or check has at least one AtomicGroup
   entry. An orphaned group remains associated in execution history but final
   materiality may project empty public affects.
7. A later traversal origin that reaches an already queried caller adds only a
   new map contribution and application trace stage. It never re-invokes or
   mutates the provider outcome.
8. A later proven mechanism may add `Mechanism(key)` to the exact supporting
   material/group roots. It does not rewrite old query/group bytes.
9. Builder allocation/arithmetic is checked. There is no independent lossy map
   prefix: size is transitively bounded by request, query, traversal, snapshot,
   group and evidence limits. An impossible overflow is an internal contract
   error, never a silent association drop.
10. `Request` is not a fallback guessed by a provider. The application adds it
    explicitly for request-wide authoritative scans/searches. Proposal and
    Mechanism scopes come only from normalized requests and proven mechanism
    instances.

### 3.3 Per-provider association derivation

The application derives associations without asking a provider:

- Metadata: authoritative Analysis scan material receives `Request`; an exact
  destination pair, presence key or requested Form scope additionally receives
  every Proposal/Mechanism scope whose normalized work contribution contains
  that exact typed material. Internal group roots remain source-qualified.
  A destination-membership pair contribution is projected to both of its real
  `SourceScopedArtifact` Material roots and to each corresponding retained
  `CfePairHalf` AtomicGroup root. If two proposals share one half, that one
  Material/AtomicGroup root contains both proposal scopes while each distinct
  other half contains only its own scope. The pair identity itself is never a
  fake artifact and loss of either real half still blocks exactly the dependent
  pair proposals.
- FormInspection: the exhaustive registered-Form scan receives `Request`;
  requested Form/command/runtime/pair enrichment receives its exact additional
  scopes. Ordinary/Inconclusive/Missing gaps use the same frozen effective Form
  material roots.
- CodeSearch: exact request search terms are Request-scoped. The term text is
  provider query material but never a conclusion identity.
- Definition: each exact queried Method carries its own origin-scope set.
  Returned tag-8 group, Absent conclusion or exact gap inherits only that
  Method's scopes, never every Method sharing an eventual plan/batch/chunk.
- CallGraph: each exact caller carries its own origin-scope set. Returned call
  groups/gaps inherit only the exact caller scopes. A cached caller later unions
  a new origin in the map and replays retained edges without a provider call.
- Support: each exact `SupportQuerySubjectV2` carries candidate/proposal/
  mechanism work scopes. Equal typed subjects dedupe in the imported query and
  union only in this map.

For Metadata/Form exhaustive material that was not a request member, the
application may create a post-collection Material/AtomicGroup entry only after
the provider's typed group validates; it inherits the exact predeclared
source-group Request scope. No raw fact/display text is reparsed to recover an
association.

### 3.4 Exact encoding and goldens

The encoder reuses the accepted Task 5B v7 primitives:

```text
ProviderInvocationIdentityBytesV2 =
  u16be(EvidencePort stable tag) || digest32(query_digest)

MaterialAssociationRootIdentityBytesV2 =
    u16be(1) || ProviderInvocationIdentityBytesV2
  | u16be(2) || ProviderInvocationIdentityBytesV2
             || bytes(AtomicSourceIdentityV2)
  | u16be(3) || ProviderInvocationIdentityBytesV2
             || bytes(ProviderGroupMaterialIdentityBytesV2)
  | u16be(4) || ProviderInvocationIdentityBytesV2
             || bytes(SemanticAtomicGroupKeyBytesV2)

ConclusionScopeIdentityBytesV2 =
    u16be(1)
  | u16be(2) || string(exact validated proposal id)
  | u16be(3) || u16be(MechanismFamilyV1 tag)
             || ArtifactIdentityBytesV1(entry)
             || ArtifactIdentityBytesV1(handler)

MaterialAssociationEntryIdentityBytesV2 =
  bytes(MaterialAssociationRootIdentityBytesV2)
  || vec(ConclusionScopeIdentityBytesV2 sorted unique)

MaterialAssociationMapIdentityBytesV2 =
  u16be(schema=2)
  || vec(entries sorted unique by complete root bytes)

material_association_digest =
  H("unica.project-material-association/v2",
    MaterialAssociationMapIdentityBytesV2)

bytes(x) = u32be(len(x)) || raw x bytes
H(domain, payload) = SHA-256(bytes(ASCII domain) || bytes(payload))
```

The grammar is implemented only by the opaque root's
`write_identity_v2` exhaustive private-inner-enum match in `ports.rs`.
The association-map writer receives the one borrowed
`CanonicalIdentitySinkV4` created by `determinism.rs` and delegates each root
to that method; no module supplies or obtains a buffer. It never extracts a variant,
invocation key, source, material or group. The four stable tags and every
published map golden remain byte-identical to v2 despite the sealed Rust shape.

Duplicate raw contributions union before encoding. Duplicate final roots or
scopes passed to the immutable constructor reject. Empty vectors encode their
explicit zero count. Debug/serde/display, exact artifact spelling, task prose,
provider order, evidence IDs, wall clock and every association-authority
type/field/member/state do not enter these bytes; the already frozen invocation
key digest is the sole authority-derived identity projection.

Normative independent fixtures use MetadataCatalog port tag 1 and query digest
`11` repeated 32 bytes:

```text
empty map:
  payload length = 6
  SHA-256(payload) =
    3686cb37b7fe4758ac2024a76e30a4c6ee2fdc25c66aa21dd55a9569b91ea504
  H("unica.project-material-association/v2", payload) =
    5e15eccbf7fa19a4f376efa6fcbb71ea64f771d72f220ae868754c709f515be4

one Invocation root with [Request, Proposal("p")]:
  root length = 36
  entry length = 53
  payload length = 59
  SHA-256(payload) =
    048ea3bd5d3218322592de805f5a0b10dc8c9e3829edb1eee68c40d37b4bd273
  H("unica.project-material-association/v2", payload) =
    82c0d668ce886e5c27bf241899d0fb72a227ad92bf6726e4657d28e0086e9d96

add Proposal("q") to the same root:
  payload length = 66
  SHA-256(payload) =
    09791f3637ceb7fc093eaf977931c5c8dc284a06bd796945eda07b21b466fc80
  H("unica.project-material-association/v2", payload) =
    f6ae6d13401e23ba46266033598c4edba3598909f4849acbedf7c85b014d9572
```

The corresponding mandatory negative fixtures for omitting both length frames
are:

```text
SHA-256(raw ASCII domain || raw empty payload) =
  7d2f555ab2846e8a1eb90489578d376296f4b5a8c3d423916efcdb12657a90f8
SHA-256(raw ASCII domain || raw p payload) =
  8a21a924d61360b7112728a7eb0b77287bd75ec0920ea1e310036e7be83f38c8
SHA-256(raw ASCII domain || raw p,q payload) =
  a922b904a2283e013d615a9f8e6c2f98c5ef8cc7bb5b9f040331789bb936f6c6
```

For the empty payload, the single-frame negative fixtures are:

```text
SHA-256(bytes(ASCII domain) || raw payload) =
  a7862a9248e2ba78a0939a09573001ebf27e99a04d578ff031431d71a9bddde5
SHA-256(raw ASCII domain || bytes(payload)) =
  8a421273ff37ae58b0e2414ccc035df9534286b01d519b8615b12819662b8b89
```

All three framing mistakes must reject; none is a valid
`material_association_digest`.

The `p` -> `p,q` metamorphic test simultaneously asserts byte-identical
provider query, provider call count, raw outcome, semantic group, provider gap,
local/global retention order and retained prefix. The normalized request,
map, traversal origins/trace, proposal verdicts/public conclusions and
enclosing analysis identity may change; no other provider or admission byte
may change.

The numeric fixtures above are supplemented by four mandatory cross-owner
structural goldens. Each test constructs its opaque typed input through the
accepted Task 5B smart constructor, then independently concatenates the bytes
shown in this section without calling the production root/scope encoder:

1. `SourceGroup` uses the Task 6 Analysis `AtomicSourceIdentityV2` fixture and
   must equal `0002 || ProviderInvocationIdentityBytesV2 ||
   bytes(the exact 148-byte AtomicSourceIdentityV2)`;
2. `Material` uses that source plus Method `CommonModule.Flow.Run` and must
   equal `0003 || ProviderInvocationIdentityBytesV2 ||
   bytes(0001 || SourceScopedArtifactIdentityBytesV2)`;
3. `AtomicGroup` uses Task 5B's normative StandaloneFact/`Document.Σ` group and
   must equal `0004 || ProviderInvocationIdentityBytesV2 ||
   bytes(the complete accepted SemanticAtomicGroupKeyBytesV2)`; the independent
   fixture also confirms the imported group-key length/hash before embedding;
4. a Mechanism scope uses `MechanismFamilyV1::DocumentLifecycle`, entry
   `Document.Invoice`, handler `CommonModule.Flow.BeforeWrite`, and must equal
   `0003 || 0001 || ArtifactIdentityBytesV1(entry) ||
   ArtifactIdentityBytesV1(handler)`.

With exactly one entry and the scope stated above, independent reconstruction
must reproduce these additional normative values:

```text
SourceGroup + Request:
  root/entry/payload lengths = 188 / 198 / 204
  SHA-256(payload) =
    05756f78177871ca24b0eecbe4d59b1b04b64b63372035d893c736393895c8af
  H = 7ff578af64e7949633080495e2cada6acc0b58307480690a1f3c76dbacc77fa2

Material(SourceScopedArtifact Method) + Request:
  root/entry/payload lengths = 221 / 231 / 237
  SHA-256(payload) =
    cf0610fc5ecf676c5788e01433fda2857b3429f548dec2caf1361a08c483558f
  H = 9e7fe772a06a3747f8d050ea15de703cb99f355c3e2d2d63b15618ea8dacf50e

AtomicGroup(Task 5B StandaloneFact Document.Σ) + Request:
  imported complete group-key length/SHA-256 =
    388 / b3c237eaccb7eb8732611804c1e972ec329c2f6a3d303ffd017863d4c16eed01
  root/entry/payload lengths = 428 / 438 / 444
  SHA-256(payload) =
    c69abb12f28c3d6cc8db6ffb32bb949f6473e5c3d8474e150a86f083199250d1
  H = c2f9bfbfc8b12e687e153762d902b544808e93a0234e799d06e0973d1e9ee553

Invocation + Mechanism(DocumentLifecycle):
  root/scope/entry/payload lengths = 36 / 61 / 105 / 111
  SHA-256(payload) =
    1da4310b9f021f65b020bcb733a0e056b2e02d6c43928c425713601b9bbbb33b
  H = 366f40dad89cc048ccbb8eec15e5284da60af21745e8d4401d7b04d221b744b6
```

For every structural golden, the test verifies the exact root/scope length,
the complete enclosing map payload, payload SHA-256 and domain-framed digest,
then mutates only the root tag, nested length frame, source role, material tag,
group key, mechanism-family tag, entry or handler and requires inequality or
constructor rejection. It also adds the exact shared-pair fixture described in
section 3.3. Calling the production encoder to manufacture the expected bytes,
or accepting only a digest copied from production output, is forbidden.

## 4. Provider invocation identity and application trace

### 4.1 Raw provider invocation has no conclusion scope

The v6 `ScopedProviderInvocation` and `ProviderInvocationSnapshot` fields named
`conclusion_scopes` are deleted. They are not renamed or hidden in a provider
context. The replacement boundary is:

```rust
pub(crate) struct RawProviderInvocationSnapshotV2 {
    key: ProviderInvocationKeyV2,
    raw_outcome: ProviderOutcomeSnapshot,
}

pub(crate) struct ScopedProviderRollupSnapshotV2 {
    port: EvidencePort,
    provider: EvidenceProvider,
    invocations: Vec<RawProviderInvocationSnapshotV2>,
}

pub(crate) struct EffectiveProviderInvocationV3 {
    key: ProviderInvocationKeyV2,
    raw_outcome_digest: Digest32,
    retained_record_digests: Vec<Digest32>,
    // PerPort/Global only, and every member owns this exact key.
    // EvidenceGapLimit is analysis-level and is never stored here.
    admission_gaps: Vec<EvidenceAdmissionGapV3>,
    effective_coverage: Coverage,
    effective_outcome_digest: Digest32,
}

pub(crate) struct ApplicationInvocationTraceV2 {
    invocation: ProviderInvocationKeyV2,
    stages: Vec<ProviderInvocationStageV2>,
}

pub(crate) struct ProviderOutcomeSnapshot {
    port: EvidencePort,
    provider: EvidenceProvider,
    readiness: ProviderReadiness,
    coverage: Coverage,
    reason_code: Option<StableReasonCodeV3>,
    retryable: bool,
    gaps: Box<[ProviderGap]>,
    record_digests: Box<[Digest32]>,
    raw_identity_token: CanonicalIdentityToken<RawProviderOutcomeIdentityV2>,
    raw_outcome_digest: Digest32,
    // private; no general constructor/Deserialize/field projection
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum ProviderInvocationStageV2 {
    MetadataComposite,
    FormAnalysis,
    CodeSearch,
    InitialDefinition,
    CallGraph { depth: u8 },
    TraversedDefinition { depth: u8 },
    Support { round: u8 },
}

impl ScopedProviderRollupSnapshotV2 {
    pub(super) fn write_identity_v2(&self, sink: &mut CanonicalIdentitySinkV4);
}

impl RawProviderInvocationSnapshotV2 {
    fn write_identity_v2(&self, sink: &mut CanonicalIdentitySinkV4);
}

impl ProviderOutcomeSnapshot {
    pub(super) fn from_registry_outcome_v4(
        authority: RegistryProviderOutcomeConstructionAuthorityV4<'_>,
        outcome: &CollectedProviderOutcome,
    ) -> Result<Self, DiscoveryError>;

    pub(super) fn raw_identity_token_v2(
        &self,
    ) -> &CanonicalIdentityToken<RawProviderOutcomeIdentityV2>;

    pub(super) fn raw_outcome_digest_v2(&self) -> Digest32;

    pub(super) fn write_raw_identity_v2(
        &self,
        sink: &mut CanonicalIdentitySinkV4,
    );
}

impl EffectiveProviderInvocationV3 {
    pub(super) fn from_registry_finality_v4(
        authority: RegistryEffectiveInvocationConstructionAuthorityV4<'_>,
        key: ProviderInvocationKeyV2,
        raw_outcome_digest: Digest32,
        retained_record_digests: Vec<Digest32>,
        admission_gaps: Vec<EvidenceAdmissionGapV3>,
        raw_coverage: Coverage,
    ) -> Result<Self, DiscoveryError>;

    pub(super) fn identity_token_v3(
        &self,
    ) -> CanonicalIdentityToken<EffectiveProviderInvocationIdentityV3>;

    pub(super) fn write_identity_v3(&self, sink: &mut CanonicalIdentitySinkV4);
}

impl ApplicationInvocationTraceV2 {
    pub(super) fn write_identity_v2(&self, sink: &mut CanonicalIdentitySinkV4);
}
```

`RawProviderInvocationSnapshotV2`, `ScopedProviderRollupSnapshotV2` and
`ApplicationInvocationTraceV2` are owned by `ports.rs`; consuming
`finish_execution_v4` may therefore construct their private fields directly
from Finished entries and registry-owned stages. There is no intermediate
`RawProviderInvocationV2`: moving then dropping the only full testimony would
serve no ownership or identity purpose.
`EffectiveProviderInvocationV3` and `ProviderOutcomeSnapshot` are owned by
`model.rs`. Each exact `pub(super)` factory consumes its distinct
ports-owned `Registry*ConstructionAuthorityV4<'finish>` plus the typed parts
shown. The authority fields are private to `ports.rs`, so model and every other
sibling can name but cannot mint one. Finish validates the full relation before
minting it; the model then validates local shape, canonicalizes order and
computes `effective_coverage`, raw identity/digest or
`effective_outcome_digest` itself. `ProviderOutcomeSnapshot` owns `retryable`,
the full raw token and raw digest shown above. No sibling supplies those derived
fields to a writer, and no factory returns a field, key, canonical byte slice or
reusable encoder. Static tests permit exactly the private finish call to each
factory and reject a shared/generic authority type.

Stage tags are declaration order 1..=7. Depth/round constraints remain the
Task 7 v6 bounds. An exact query is called at most once. Reuse unions canonical
stage values in the application trace and associations in the map; it does not
change the stored raw outcome or raw snapshot.

No type above creates or receives a caller key. Each registry entry owns its
private `BTreeSet<ProviderInvocationStageV2>`. The exact `pub(super)`
`record_stage_v2(&RecordedInvocationIdV2, stage)` boundary resolves only a
current Finished ID and validates stage/typed-query/port/depth/round
compatibility before lookup. An already present equal valid stage returns
`Ok(())` without mutation; a new valid stage inserts transactionally. Only
`use_case.rs`
calls it after the corresponding typed invoke and association update.
Consuming `finish_execution_v4` constructs every raw snapshot, rollup and
`ApplicationInvocationTraceV2` directly from private Finished entries
and those nonempty canonical stage sets. It accepts no key-bearing raw/trace DTO
from orchestration and rejects any Finished entry without a valid stage or any
stage with no Finished entry.

The private finalizer rederives the exact retained physical-record subsequence
from its admission buckets and each current raw outcome. It proves that the
sequence is duplicate-free, order-preserving and equals raw records minus
complete dropped atomic groups; derives the raw snapshot/digest; attaches
exactly the derived PerPort/Global buckets owning that invocation; and mints one
non-Clone/non-serde `RegistryEffectiveInvocationConstructionAuthorityV4` for
the model constructor. Immediately after construction, finish independently
rebuilds the raw snapshot from the still-current collected outcome, compares
its typed raw token and digest, validates the effective DTO through its owner,
and records the key in the finish ledger. No retained vector, gap, raw/effective
digest, coverage, key or witness crosses the orchestration boundary. Any error
invalidates the execution before a finished projection can exist.

`ProviderOutcomeSnapshot` above is the immutable raw Task 5A snapshot created
directly from `CollectedProviderOutcome`: raw provider gaps, raw coverage and
the complete raw physical-record digest set are preserved without application
admission. Its Task 7 identity is closed as follows, using the accepted Task
5A/5B typed gap/location/record models while owning the exact raw-gap encoding
published immediately below:

```text
RawProviderOutcomeIdentityBytesV2 =
  u16be(port tag)
  || string(provider name) || string(provider version)
  || u16be(readiness tag) || u16be(raw coverage tag)
  || option(string(validated provider reason code))
  || u8(retryable: false=0 | true=1)
  || vec(bytes(RawProviderGapIdentityBytesV2) in canonical order)
  || vec(digest32(raw physical-record digest) sorted unique)

raw_outcome_digest =
  H("unica.provider-outcome-snapshot/v2",
    RawProviderOutcomeIdentityBytesV2)

RawProviderInvocationSnapshotIdentityBytesV2 =
  ProviderInvocationIdentityBytesV2
  || bytes(RawProviderOutcomeIdentityBytesV2)

ScopedProviderRollupSnapshotIdentityBytesV2 =
  u16be(port tag)
  || string(provider name) || string(provider version)
  || vec(bytes(RawProviderInvocationSnapshotIdentityBytesV2)
         sorted by ProviderInvocationIdentityBytesV2)
```

`retryable` is copied directly from the raw `CollectedProviderOutcome`, remains
independent for Bounded/Unavailable/Failed, and validates false for a Complete
outcome with no retry condition. It is raw provider testimony, not derived from
readiness or reason. A mutation-only golden toggles it and must change both raw
and effective digests while leaving every other field byte-identical.

Task 7 owns the exact raw-gap projection because no upstream owner exports one:

```text
RawProviderGapIdentityBytesV2 =
  string(validated exact provider reason code)
  || (u16be(1)
      || vec(bytes(SourceScopedArtifactIdentityBytesV2) sorted unique)
      | u16be(2)
      | u16be(3) || bytes(AtomicSourceIdentityV2))
  || option(SourceLocationIdentityBytesV1)

SourceLocationIdentityBytesV1 =
  string(validated workspace-relative path)
  || option(u32be(line))
  || option(u32be(column))

option(None) = u8(0)
option(Some(x)) = u8(1) || x
```

Raw gap scope tags deliberately follow the upstream `ProviderGapScope`
declaration and are **not** the admission-scope order: Artifacts=1,
QueryWide=2, SourceSetWide=3. Artifacts is nonempty and its complete typed
identities are strictly increasing; SourceSetWide uses the complete
`AtomicSourceIdentityV2`; QueryWide has no payload. Location requires a
validated nonempty relative path, `line >= 1`, and `column >= 1` only when line
is Some; `column=Some` with `line=None` rejects. Raw gaps sort strictly by the
complete `RawProviderGapIdentityBytesV2`; duplicate final gaps reject rather
than deduplicate. Unknown tags, extra zero counts/frames, display paths and
string source/artifact reinterpretation reject.

Rollups sort by port stable tag and ports are unique. Every invocation key in a
rollup has that exact port; query digests are unique and strictly increasing.
Every raw outcome repeats and validates the same port/provider name/version.
Empty invocation rollups are forbidden. No conclusion, retained prefix,
admission gap, effective coverage, application stage/depth/round or map byte is
present in this raw DTO. Provider name/version consistency and physical-record
ownership remain as in v6. A Task 7 wrapper digest over port/depth/query is
forbidden: the invocation key uses the imported query digest byte-for-byte.

`EffectiveProviderInvocationV3` has this independent canonical identity and
digest:

```text
EffectiveProviderInvocationDigestPayloadV3 =
  u16be(schema=3)
  || ProviderInvocationIdentityBytesV2
  || digest32(raw_outcome_digest)
  || vec(digest32(retained physical-record digest)
         in exact retained group/record order)
  || vec(bytes(EvidenceAdmissionGapIdentityBytesV3)
         sorted unique by complete bytes)
  || u16be(effective coverage tag)

effective_outcome_digest =
  H("unica.project-evidence-admission/v3",
    EffectiveProviderInvocationDigestPayloadV3)
```

The retained vector is a subsequence of the raw records in the accepted global
semantic-group/physical-record order, not digest-sorted, and contains no
duplicate. Every admission gap names this invocation (or, for a global gap,
includes it among its canonical owners), and effective coverage is derived
from raw coverage plus those gaps; no caller supplies it independently. The
stored `raw_outcome_digest` is derived by the registry factory from the current
Finished outcome, revalidated during finish and retained so the model-owned
writer can emit the payload after the registry is consumed. The stored
`effective_outcome_digest` must equal the formula and is never included
recursively in its own payload. Effective invocations sort uniquely by
`ProviderInvocationIdentityBytesV2` and form a bijection with raw invocations.

Application trace identity is exact:

```text
ProviderInvocationStageIdentityBytesV2 =
    u16be(1)                                      # MetadataComposite
  | u16be(2)                                      # FormAnalysis
  | u16be(3)                                      # CodeSearch
  | u16be(4)                                      # InitialDefinition
  | u16be(5) || u16be(depth 0..=11)              # CallGraph
  | u16be(6) || u16be(depth 1..=12)              # TraversedDefinition
  | u16be(7) || u16be(round 1..=64)              # Support

ApplicationInvocationTraceIdentityBytesV2 =
  ProviderInvocationIdentityBytesV2
  || vec(bytes(stage bytes) sorted unique by complete bytes)
```

Trace rows sort uniquely by invocation identity, contain a nonempty stage
vector and form a bijection with raw invocations. A stage incompatible with
the invocation port rejects. Reuse inserts each distinct valid stage once; a
repeat of the same already validated stage is an idempotent no-op so two
origins at one depth cannot spuriously fail. Depth/round overflow, foreign
invocation, duplicate final trace row or input-order-dependent stage order
rejects.

### 4.2 Execution snapshot and contract versions

The successor execution identity is:

```rust
pub(crate) struct AnalysisExecutionSnapshotV4 {
    analysis_id: AnalysisId,
    finished: FinishedAnalysisExecutionProjectionV4,
}

impl AnalysisExecutionSnapshotV4 {
    pub(super) fn from_finished_execution_v4(
        finished: FinishedAnalysisExecutionProjectionV4,
    ) -> Self;

    pub(super) fn analysis_id_v4(&self) -> &AnalysisId;

    pub(super) fn into_final_report_projection_v4(
        self,
    ) -> Result<FinalDiscoveryReportProjectionV4, DiscoveryError>;
}

impl DiscoveryReport {
    pub(super) fn from_final_projection_v4(
        projection: FinalDiscoveryReportProjectionV4,
    ) -> Result<Self, DiscoveryError>;
}
```

`AnalysisExecutionSnapshotV4` and its private fields are owned by `ports.rs`
beside the finished seal. `finished` is the complete physical seal, not a new
identity frame. It is
returned only by consuming `ProviderInvocationRegistryV2::finish_execution_v4`
and owns the analysis prefix with its sole analysis-header Task5B binding field,
provider rollups,
effective invocations, application trace, material map, internally derived
optional EvidenceGapLimit/effective-gap projection, the registry-derived
`TraversalGapSetV3` and the finalized report materiality. It exposes no
field/key/token iterator or serde form. The
snapshot constructor accepts nothing else; in particular there is no loose
composite/catalog digest, traversal vector, request/source/limits/registry
fragment, sentinel branch or report parts to splice from another execution.
The snapshot constructor calls the determinism-owned finalizer exactly once,
stores that `analysisId`, and the consuming report projection moves the same
value with every already finalized public field; the `DiscoveryReport`
factory checks schema/order only and cannot rerun finality.

Only `determinism.rs` creates the root `CanonicalIdentitySinkV4`, asks the
finished projection to append the complete analysis-ID payload, and
finalizes the domain hash. Its one `pub(super)` finished-seal finalizer is
statically callable only by
`AnalysisExecutionSnapshotV4::from_finished_execution_v4` in `ports.rs`;
that constructor stores the returned `AnalysisId` and the consuming snapshot
never calls the finalizer again. Task7 owner writers receive only that
write-only sink. The sink's private adapters alone call Task4/Task5B's intentionally
exported typed binding/material identity writers with its internal buffer;
siblings never obtain that buffer. Static call-site tests reject every other
production caller, byte/length/slice projection, intermediate `Vec`, field
accessor or extra nesting frame.

Before returning the projection, `finish_execution_v4` revalidates every final
map root,
requires a one-to-one Finished/raw/effective/trace key set, reconstructs every
raw snapshot/rollup and trace internally, and checks raw-outcome digests,
retained-record subsequences and effective digests. With no EvidenceGapLimit
sentinel, provider-owned `EffectiveGapV3` rows are an exact bijective projection
of the canonical location-erased effective-gap classes, every raw provider gap
belongs to exactly one class, and Admission rows are an exact unique projection
of the registry-derived occupied PerPort/Global buckets; every bucket gap also
appears exactly once under each and only each owning effective invocation. With the
sentinel, `effective_gap_projection` is instead exactly one Admission-owned
EvidenceGapLimit row and contains no provider or other Admission row; raw
provider history remains in rollups and PerPort/Global admission history remains
inside the effective invocations. Missing, duplicate, foreign or extra rows in
the selected branch invalidate the whole projection. No caller can use an
effective DTO as a key accessor or submit raw/trace DTOs.

The consuming order is normative: reject invalidated/Staged/unprepared state,
any unresolved traversal requirement or
an unconsumed/out-of-order finality requirement; reconstruct and compare the
prepared derivation; validate the map, sealed traversal ledger and sealed
materiality plan; build every raw/effective/admission/provider candidate through
its distinct one-shot model authority; finalize all typed relation sets; derive
the sentinel branch from the full candidate set; create final materiality; and
only then construct `FinishedAnalysisExecutionProjectionV4`. Before that last step
no writer, snapshot, analysis ID, report or receipt is reachable. The finish
context drops before registry entries are moved into the seal.

The v4 registry is explicitly additive to the complete v6 registry: the four
successor values below replace their named v6 predecessor and every other v6
value remains present in the same position. The exact exhaustive ordered
registry is:

```text
ANALYSIS_CONTRACT_VERSION = "project-discovery-v4"
MECHANISM_REGISTRY_VERSION = "project-mechanisms/v2"
TRAVERSAL_CONTRACT_VERSION = "project-traversal/v3"
SEMANTIC_ATOMIC_GROUP_REGISTRY = "semantic-evidence-groups/v2"
SEMANTIC_ATOMIC_ENCODER = "semantic-evidence-group-encoder/v2"
SOURCE_SET_IDENTITY_ENCODER = "unica.source-set-identity.v1"
ARTIFACT_IDENTITY_ENCODER = "artifact-identity/v1"
EVIDENCE_ADMISSION_CONTRACT = "project-evidence-admission/v3"
GLOBAL_EVIDENCE_GROUP_ORDER = "project-global-evidence-order/v2"
METADATA_COMPOSITE_QUERY_ENCODER = "metadata-composite-query/v2"
FORM_INSPECTION_QUERY_ENCODER = "form-inspection-query/v2"
BSL_PROVIDER_QUERY_ENCODER = "snapshot-bsl-provider-query/v3"
SUPPORT_STATE_QUERY_ENCODER = "support-state-query/v2"
SCHEDULED_JOB_REGISTRY = "scheduled-jobs/v2"
PROJECT_DISCOVERY_CORPUS_VERSION = "project-discovery-corpus/v2"
FORM_COMMAND_HANDLER_POLICY = "form-command-handlers/v1"
HTTP_SERVICE_HANDLER_POLICY = "http-service-handlers/v1"
PLATFORM_CONFIGURATION_CATALOG = "platform-configuration-catalog/v1"
PLATFORM_CONFIGURATION_CATALOG_ENCODER =
  "platform-configuration-catalog-encoder/v1"
PLATFORM_CONFIGURATION_CATALOG_SET = "platform-configuration-catalog-set/v1"
PLATFORM_CONFIGURATION_OBJECT_KEY_ENCODER =
  "platform-configuration-object-key/v1"
REGISTERED_FORM_CATALOG = "registered-form-catalog/v1"
REGISTERED_FORM_CATALOG_SET = "registered-form-catalog-set/v1"
SOURCE_SNAPSHOT_FINGERPRINT_ENCODER = "source-set-snapshot/v2"
COMPOSITE_SNAPSHOT_FINGERPRINT_ENCODER = "source-composite/v2"
REGISTERED_MATERIAL_EXPECTATION_CATALOG =
  "registered-material-expectations/v1"
REGISTERED_MATERIAL_PATH_POLICY = "registered-form-material-paths/v1"
MATERIAL_ASSOCIATION_CONTRACT = "project-material-association/v2"
```

The registry is encoded exactly in the displayed order as
`vec(string(value))`; it is not lexically sorted. Adding, deleting, duplicating
or reordering an entry rejects. The only replacements from v6 are analysis
v3->v4, traversal v2->v3, evidence-admission v2->v3 and BSL query v2->v3;
Support, registered-Form, snapshot-v2 and association entries are additive.

Canonical execution bytes are:

```text
AnalysisExecutionSnapshotIdentityBytesV4 =
  u16be(schema=4)
  || digest32(composite_snapshot_id)
  || digest32(configuration_catalog_set_digest)
  || digest32(registered_form_catalog_set_digest)
  || vec(bytes(ScopedProviderRollupSnapshotIdentityBytesV2)
         sorted unique by port tag)
  || vec(bytes(EffectiveProviderInvocationIdentityBytesV3)
         sorted unique by invocation identity)
  || vec(bytes(ApplicationInvocationTraceIdentityBytesV2)
         sorted unique by invocation identity)
  || bytes(MaterialAssociationMapIdentityBytesV2)
  || option(bytes(EvidenceAdmissionGapIdentityBytesV3))
  || vec(bytes(EffectiveGapIdentityBytesV3) sorted unique)
  || vec(bytes(TraversalGapIdentityBytesV3) sorted unique)

EffectiveProviderInvocationIdentityBytesV3 =
  EffectiveProviderInvocationDigestPayloadV3
  || digest32(effective_outcome_digest)

analysis_execution_snapshot_digest =
  H("unica.project-discovery-execution-snapshot/v4",
    AnalysisExecutionSnapshotIdentityBytesV4)
```

Every vector arrives canonical at the immutable constructor; duplicate items
are rejected rather than silently deduplicated. The raw, effective and trace
vectors have the exact pairwise key bijections defined above. The composite ID
is the validated `CompositeSnapshotIdV2` raw digest projection from Task 4; a
transport string is never re-parsed here.

The three leading execution digests above are emitted only by the prefix's one
`append_platform_binding_v1` call: that upstream writer appends the composite,
configuration-catalog and registered-Form catalog digests as exactly 96 raw
bytes after `u16be(schema=4)`. They are not separately stored or written by the
finished projection. The sole full writer chain is:

```text
FinishedAnalysisExecutionProjectionV4::write_analysis_identity_payload_v4
  -> AnalysisIdentityPrefixV4::write_analysis_identity_payload_v4
     -> normalized request identity (including limits)
     -> CanonicalDiscoverySourceV4 identity
     -> exhaustive contract-registry vector
     -> bytes(
          u16be(schema=4)
          || append_platform_binding_v1(exact 96 bytes, once)
          || Finished::write_execution_snapshot_tail_v4(...)
        )
```

The tail begins with provider rollups and contains every remaining field shown
after the three binding digests. It cannot write schema/binding/request/source/
registry. The outer writer and tail are private to `ports.rs`; only the full
writer is visible to the determinism finalizer. A mechanical golden locates
the one exact 96-byte binding occurrence and rejects zero, two, reordered or
extra-framed occurrences.

`analysis_evidence_gap_limit` is the sole raw storage slot for the final
analysis-level admission sentinel, but no snapshot constructor accepts that
option. Registry finish stores `None` exactly when the complete validated
candidate set is within both limits; otherwise it privately stores one
`EvidenceAdmissionReasonV3::EvidenceGapLimit` with `port=None`, empty
owners/roots and `QueryWide`. The latter branch requires
`effective_gap_projection` to contain exactly one Admission-owned
`EffectiveGapV3` with reason
`project_discovery_evidence_gap_limit`, `QueryWide` and empty association roots;
it forbids every provider or other Admission row. The no-sentinel branch stores
the complete candidate vector. Final result construction requires exactly one
corresponding check in sentinel mode and forbids it otherwise. The option bytes
above preserve the canonical admission-gap identity; the effective row and
public check are derived projections and cannot replace that storage or choose
the branch.

The existing v6 normalized-request (including its limits), source and public
output encoders remain byte-for-byte authoritative, but public output bytes are
not an analysis-ID input. The v4 `analysisId` payload is exactly normalized
request bytes, source bytes, `vec(the exhaustive registry strings above)` and
`bytes(AnalysisExecutionSnapshotIdentityBytesV4)`, under the existing framed
`analysis-id` SHA-256 domain. `diagnostic_workspace_epoch` and display/serde
text remain excluded. The analysis encoder includes the complete canonical
association-map bytes, not a caller-supplied digest. Provider raw outcome
snapshots exclude map and trace bytes. This lets two requests share
byte-identical provider testimony while still receiving different correct
analysis IDs.

## 5. Evidence admission v3 and final materiality

### 5.1 Remove application conclusions from gap identity

The v6 `EvidenceMaterialSubjectV2::Conclusion`, every admission/effective-gap
`conclusion_scopes` field, and their encodings are superseded. Provider gaps
remain the exact Task 5B v7 artifact-only/source-wide/query-wide testimony.
Application admission contains provider material roots, never conclusion
identity:

```rust
pub(crate) enum EvidenceAdmissionReasonV3 {
    PerPortEvidenceLimit,
    GlobalEvidenceLimit,
    EvidenceGapLimit,
}

pub(super) struct EvidenceAdmissionScopeV3 {
    kind: EvidenceAdmissionScopeKindV3,
    // ports.rs-owned opaque scope; no public/general constructor
}

enum EvidenceAdmissionScopeKindV3 {
    Artifacts(ProviderMaterialArtifactSetV2),
    SourceSetWide(AtomicSourceIdentityV2),
    QueryWide,
}

pub(crate) struct EvidenceAdmissionGapV3 {
    reason: EvidenceAdmissionReasonV3,
    port: Option<EvidencePort>,
    owning_invocations: Vec<ProviderInvocationKeyV2>,
    dropped_group_roots: Vec<MaterialAssociationRootV2>,
    scope: EvidenceAdmissionScopeV3,
}

pub(crate) struct EffectiveGapV3 {
    owner: EffectiveGapOwnerV3,
    reason_code: StableReasonCodeV3,
    scope: EvidenceAdmissionScopeV3,
    association_roots: Vec<MaterialAssociationRootV2>,
}

pub(crate) enum EffectiveGapOwnerV3 {
    Provider {
        invocation: ProviderInvocationKeyV2,
        provider: EvidenceProvider,
    },
    Admission,
}

pub(crate) struct StableReasonCodeV3(String);

impl EvidenceAdmissionScopeV3 {
    pub(super) fn write_identity_v3(
        &self,
        sink: &mut CanonicalIdentitySinkV4,
    );

    fn material_artifact_set_for_finish_v4(
        &self,
    ) -> Option<&ProviderMaterialArtifactSetV2>;
}
```

`EvidenceAdmissionScopeV3` is owned by `ports.rs`; its inner enum and all three
constructors are private there. `model.rs` may only ask it to append canonical
identity while writing a validated DTO. Only
`RegistryFinishValidationV4::count_effective_gap_material_subjects_v4` calls
the private set-reference helper, immediately collects those opaque references
and calls Task5B's sealed union-cardinality aggregate. No model/report/
orchestration code can match a scope or obtain a set.

`EffectiveGapOwnerV3` is closed and private. Stable tags are Provider=1 and
Admission=2. Provider requires one exact committed invocation and the same
provider identity as its raw rollup; Admission is legal only for a validated
`EvidenceAdmissionReasonV3` projection and carries no provider/query payload.

`StableReasonCodeV3` is non-serde and has no public/raw `String` constructor.
Its exact grammar is 1..=128 UTF-8 bytes, all bytes ASCII lowercase letter,
ASCII digit, `_`, `.` or `-`, with no leading/trailing whitespace. It is created
only by:

- `from_provider_gap(&ProviderGap)`, which copies the already validated exact
  raw provider reason and rejects the three application-reserved codes below
  plus `material_gap_reason_overflow`;
- `from_admission_reason(EvidenceAdmissionReasonV3)`, which maps tags 1..=3
  exactly to `project_discovery_port_evidence_limit`,
  `project_discovery_global_evidence_limit`, and
  `project_discovery_evidence_gap_limit`.

Unknown/noncanonical strings, a provider attempting an application-reserved
code, or an Admission owner with a provider reason reject. Identity bytes are
exactly `string(the validated code)`; Debug/display/JSON spellings never enter.

Every `dropped_group_roots` member is exactly an `AtomicGroup` root derived
from a current returned group and owned by one listed same-port invocation.
PerPort/Global gaps have nonempty sorted unique owners and roots.
`Artifacts(ProviderMaterialArtifactSetV2)` is the exact nonempty canonical
union of real source-qualified artifacts projected by the Task5B group owner; a
pair contributes both real halves. It has no iterator/raw membership getter,
and no proposal/mechanism/callback-slot fake artifact can be admitted.

This invariant is constructed only from registry-owned finality. The private
derivation creates exactly one occupied bucket per `(reason, port)` across the
whole execution, resolves owners from the current Finished entries and roots
from their current returned groups, and stores the retained/drop complement.
No method accepts a reason, port, owner list, root list, scope or retained
prefix, and no orchestration method returns a gap or effective row.

```rust
impl EvidenceAdmissionGapV3 {
    pub(super) fn from_registry_finality_v4(
        authority: RegistryAdmissionGapConstructionAuthorityV4<'_>,
        reason: EvidenceAdmissionReasonV3,
        port: EvidencePort,
        owning_invocations: Vec<ProviderInvocationKeyV2>,
        dropped_group_roots: Vec<MaterialAssociationRootV2>,
        scope: EvidenceAdmissionScopeV3,
    ) -> Result<Self, DiscoveryError>;

    pub(super) fn evidence_gap_limit_from_finish_v4(
        authority: RegistryEvidenceGapLimitConstructionAuthorityV4<'_>,
        scope: EvidenceAdmissionScopeV3,
    ) -> Result<Self, DiscoveryError>;

    pub(super) fn identity_token_v3(
        &self,
    ) -> CanonicalIdentityToken<EvidenceAdmissionGapIdentityV3>;

    pub(super) fn write_identity_v3(&self, sink: &mut CanonicalIdentitySinkV4);
}

impl EffectiveGapV3 {
    pub(super) fn from_registry_provider_finality_v4(
        authority: RegistryProviderEffectiveGapConstructionAuthorityV4<'_>,
        invocation: ProviderInvocationKeyV2,
        provider: EvidenceProvider,
        reason_code: StableReasonCodeV3,
        scope: EvidenceAdmissionScopeV3,
        association_roots: Vec<MaterialAssociationRootV2>,
    ) -> Result<Self, DiscoveryError>;

    pub(super) fn from_registry_admission_finality_v4(
        authority: RegistryAdmissionEffectiveGapConstructionAuthorityV4<'_>,
        reason_code: StableReasonCodeV3,
        scope: EvidenceAdmissionScopeV3,
        association_roots: Vec<MaterialAssociationRootV2>,
    ) -> Result<Self, DiscoveryError>;

    pub(super) fn identity_token_v3(
        &self,
    ) -> CanonicalIdentityToken<EffectiveGapIdentityV3>;

    pub(super) fn write_identity_v3(&self, sink: &mut CanonicalIdentitySinkV4);
}
```

The four distinct `Registry*GapConstructionAuthorityV4<'finish>` values are
private one-shot authorities minted only inside
`ports.rs::finish_execution_v4`; they have no general constructor, projection,
Clone, serde, Debug or Display. Their only production consumers are the exact
`model.rs` constructors above. Ports validates the complete relation before
minting; each model constructor then consumes its authority, validates local
shape, canonicalizes by typed token and moves the typed parts without parsing a
root. A wrong root variant,
foreign/missing owner, mixed port, stale outcome group, spelling drift,
unfinished entry, omitted/extra class or duplicate bucket invalidates the
execution and yields no gap/effective row.

`EvidenceAdmissionGapV3` itself does not implement general `Clone`. Finish may
privately reproduce one already revalidated PerPort/Global identity for each
and only each listed owner while constructing effective invocations; the
owner-fan-out ledger requires exact equality. Provider effective rows are
built only from the complete prepared raw-gap-class set. Thus no stale history
value, detached envelope or foreign root can reach a second construction path.

`EvidenceGapLimit` retains the v6 all-or-nothing sentinel semantics: `port=None`,
empty owners/roots, QueryWide and one exact check. It is minted only after the
full candidate set and every association are valid: at most 256 candidate rows
and at most 2,000 distinct artifact subjects retain the full vector; 257 rows
or 2,001 subjects select the sentinel. Its affected conclusions are
the final evidence-dependent scopes derived from the complete association map,
not an embedded scope vector. The raw provider/admission histories remain in
the execution snapshot. `EffectiveProviderInvocationV3.admission_gaps` admits
only PerPort/Global gaps whose `owning_invocations` include that invocation's
exact key; the same complete gap may therefore be projected into each of its
owner-specific effective invocation calculations without changing its identity
bytes. The single ownerless EvidenceGapLimit is stored exactly once in
the finished projection's private `analysis_evidence_gap_limit`, and projected
exactly once as the sole analysis-level `EffectiveGapV3` plus its exact check.
When it exists the effective-gap vector contains no provider or PerPort/Global
Admission projection; their complete raw/admission histories remain only in the
rollups/effective invocations. It is never inserted into any per-invocation
`admission_gaps` vector and is never copied once per port/invocation. These are
storage/projection rules only: the
exact one canonical admission-gap identity from the final admission history is
preserved in the finished seal, not reconstructed from the effective row or
relabelled at either boundary.

Admission identity bytes are exact:

```text
EvidenceAdmissionGapIdentityBytesV3 =
  u16be(reason tag)
  || option(u16be(port tag))
  || vec(ProviderInvocationIdentityBytesV2 sorted unique)
  || vec(bytes(MaterialAssociationRootIdentityBytesV2(AtomicGroup only))
         sorted unique)
  || admission_scope(
       u16be(1) || vec(SourceScopedArtifactIdentityBytesV2 sorted unique)
     | u16be(2) || bytes(AtomicSourceIdentityV2)
     | u16be(3))
```

Reason tags are declaration order 1..=3. Every occupied PerPort/Global
`(reason, port)` bucket produces exactly one gap across all invocations of that
port, with `port=Some` and the complete nonempty owner/root sets. The global
selection decision is projected into one bucket per affected port rather than
one ownerless cross-port row. EvidenceGapLimit alone requires `port=None`, empty
owners/roots and QueryWide. No other combination validates. Admission gaps sort
by these complete bytes; duplicates at the immutable boundary reject.

`EffectiveGapV3` prepends its closed owner and reason, then scope and canonical
association-root vector. The encoder contains no conclusion bytes. Changing
only association scopes preserves admission/effective-gap/provider bytes while
changing final materiality and the enclosing analysis identity.

The previously summarized sentence is made exact by these identities:

```text
EffectiveGapOwnerIdentityBytesV3 =
    u16be(1) || ProviderInvocationIdentityBytesV2
               || string(provider name) || string(provider version)
  | u16be(2)

EffectiveGapIdentityBytesV3 =
  EffectiveGapOwnerIdentityBytesV3
  || string(StableReasonCodeV3 exact code)
  || admission_scope(the exact bytes above)
  || vec(bytes(MaterialAssociationRootIdentityBytesV2)
         sorted unique)
```

Provider effective gaps require one or more roots produced by the exact scope
projection in section 5.2. Admission gaps require exactly their dropped-group
roots, except the EvidenceGapLimit sentinel which has an explicit empty vector.
Foreign roots, duplicate roots or an owner/reason/scope mismatch reject.

Traversal remains application-only and deliberately retains conclusion scopes,
but there is no caller-built traversal vector or set. The registry owns the one
expected/resolved ledger from the moment a bounded traversal decision is
scheduled:

```rust
pub(super) struct TraversalGapScopeV3 {
    kind: TraversalGapScopeKindV3,
}

enum TraversalGapScopeKindV3 {
    Artifacts(Vec<SourceScopedArtifact>),
    QueryWide,
}

impl TraversalGapScopeV3 {
    pub(super) fn from_application_artifacts_v3(
        artifacts: Vec<SourceScopedArtifact>,
    ) -> Result<Self, DiscoveryError>;

    pub(super) fn query_wide_v3() -> Self;
}

pub(crate) struct TraversalGapSetV3 {
    gaps: Vec<TraversalGap>,
    // private canonical vector; constructed only while finish consumes ledger
}

impl TraversalGapSetV3 {
    pub(super) fn identity_token_v3(
        &self,
    ) -> CanonicalIdentityToken<TraversalGapSetIdentityV3>;

    pub(super) fn write_identity_v3(&self, sink: &mut CanonicalIdentitySinkV4);
}
```

Before mutating traversal state, each exhaustive decision boundary calls the
ports-owned `TraversalRequirementSpecV3::from_bounded_decision_v3` factory and
then `register_traversal_requirement_v3`. The factory is statically callable
only from `traversal.rs`; it validates the one exact gap reason against its
phase (including the Support round carried only by `TraversalPhase`), depth
bound, complete scope and canonical conclusion scopes. Registration rechecks
request/source authority and every artifact spelling, then returns a fresh
execution-branded non-Clone ID. The boundary must consume that ID exactly once
through either `complete_traversal_requirement_v3` or the argument-free
`gap_traversal_requirement_v3`. The latter marks the stored spec `Gapped`;
prepare validates its prospective canonical gap identity and consuming finish
moves the stored reason/phase/depth/scope/conclusions into `TraversalGapSetV3`.
No caller constructs or submits a `TraversalGap`. Wrong/foreign/duplicate
resolution invalidates the execution.

`prepare_discovery_finality_v4` is the only seal transition: it requires
`expected_count == completed_count + gapped_count`, no Pending entry, one
resolution per slot and unique canonical gap rows. It stores the sealed ledger
inside registry Prepared state. Finish replays those relations, revalidates
every gap against the retained request/snapshot/context and prefix-owned
binding, and moves exactly the canonical `Gapped` rows into
`TraversalGapSetV3`. The orchestration supplies no traversal argument to finish
and can neither omit a Pending decision nor splice a foreign empty set. Its v3
identity is the
non-conflicting v6 identity made explicit:

```text
TraversalGapIdentityBytesV3 =
  u16be(TraversalGapReason declaration-order tag)
  || u16be(TraversalPhase declaration-order tag)
  || [for SupportSelection only: u16be(round 1..=64); otherwise empty]
  || (u16be(1) || vec(SourceScopedArtifactIdentityBytesV2 sorted unique)
      | u16be(2))
  || option(u16be(depth within the owning phase bound))
  || vec(ConclusionScopeIdentityBytesV2 sorted unique)
```

Traversal gaps sort by these complete bytes and duplicate final rows reject.
The application may retain `ConclusionScope` in traversal state/gaps; it may
not place it in provider query, testimony, gap, outcome, cache or retention
identity.

### 5.2 Exact projection to association roots

- Provider `QueryWide` maps to the exact Invocation root.
- Provider `SourceSetWide` maps to the exact SourceGroup root under that
  invocation.
- Provider `Artifacts` maps to exact Material roots using
  `ProviderGroupMaterialIdentityV2::SourceScopedArtifact`.
- Per-port/global admission maps to its exact dropped AtomicGroup roots.
- The one overflow sentinel remains rootless; its affected conclusions are
  computed from the complete validated map at final projection, never encoded
  as fabricated sentinel roots.

If a provider returns a valid source-local artifact gap for exhaustive material
not named before I/O, Task 7 constructs the Material root only from the
validated typed gap and inherits the exact SourceGroup association declared
before I/O. A gap outside its query/source/material authority is a provider
contract violation.

Final check affects, candidate blockers and proposal coverage gaps are computed
by intersecting:

1. the gap's exact association roots and map scopes;
2. the rebuilt retained mechanism/call/ownership paths;
3. the candidate or proposal's exact positive/negative proof obligations.

The map is an upper-bound lineage record, not a shortcut to blocking. An
orphaned branch remains in execution history but projects empty public affects.

### 5.3 Admission and root selection are association-invariant

Per-provider, per-port and global prefixes sort/count only complete accepted
Task 5B v7 group bytes and physical record counts. `MaterialAssociationMapV2`
is never an order key. Adding/removing a conclusion cannot move a group across
`maxEvidence`, split a group or change a provider-local gap.

Root selection remains application-owned:

- merge equal Methods and all application origins before applying
  `MAX_TRAVERSAL_ROOTS`;
- sort by the existing runtime-first typed Method key, never proposal ID;
- retain `origin_depths: BTreeMap<ConclusionScope, u8>` only in traversal
  state;
- emit no provider query for an omitted root;
- create an application `TraversalGap`, not a ProviderGap;
- only a proven `Mechanism` origin grants runtime reachability;
  Request/Proposal origins are context-only;
- replay a cached caller separately for every later origin within that origin's
  own depth bound, adding association/trace only.

This is the root-admission back-propagation required by Task 5B/6. A proposal
identity in Definition/Call terms, group keys or admission order is a hard STOP.

## 6. Staged orchestration overrides

All non-conflicting Task 7 v6 stages remain. Apply these exact deltas:

Every actual provider invocation in every stage follows the same closed
lifecycle: register an imported typed query -> stage associations by borrowing
the non-cloneable plan -> pass that plan, the same typed query and exact
provider/context capability to its one matching registry invoke overload -> let
that overload perform exactly one call and internally record its returned raw
terminal outcome -> store the returned opaque recorded ID -> reborrow a
short-lived recorded handle only for ordinary returned material/group roots ->
drop the handle -> record the typed application stage by ID. No lookup handle
survives a mutable registry operation. Provider/admission effective classes are
not exposed during this lifecycle; the common ordered finality projection is
available only after the one prepare transition in Stage 5.
Task 7 orchestration never receives a private key or any caller-ingress outcome
that it can submit to the registry. It may copy ordinary owned work only inside
the sealed `with_application_evidence_v2` or retained-preview closure; registry
finality ignores those copies. The final registry/map/snapshot constructor
enforces the strict typed-plan/typed-invoke/raw/recorded-ID/Invocation-root
bijection from section 3.1. No stage may validate a pre-I/O root against a raw
outcome that does not yet exist.

### Stage 0 — prepare, capture, one catalog build

1. Normalize request and run source/preflight checks that require no material.
2. Resolve/capture the exact Task 4 v7 composite `SourceSnapshotV2` once
   through the injected `&dyn SourceSnapshotPort`. Its Analysis and canonical
   Destination `SourceSetSnapshotV2` atoms include dynamic registered Form
   material expectations.
3. Call `PlatformCatalogPort::build_context(snapshot, source_reader)` once to
   produce the one `PlatformCatalogContextV1`.
4. Enter `with_provider_invocation_registry_v2` with the exact validated
   request (including its limits), snapshot and context. The constructor derives
   the Analysis source only from that snapshot. Before exposing the
   by-value execution closure it atomically constructs the analysis prefix with
   its sole execution
   binding and commits the complete request-plus-catalog spelling projection.
   A request/catalog collision or binding mismatch exposes no registry and
   occurs before any smart query construction, registration or provider I/O;
   there is no later commit method.
5. Inside that closure store exact borrows of the context, snapshot and same injected
   reader in `EvidenceExecutionContext`. Every later query constructor borrows
   the context; every imported provider receives the exact capabilities its
   owner contract requires. The orchestration spy rejects a second build;
   snapshot substitution, reader substitution, half-context or detached catalog
   fails before provider I/O. Independently, a direct deterministic repeat of
   the borrowed build API is legal and byte-equal.
6. Task 7 never calls a specialized registered-material reader. Metadata/Form
   provider calls may use Task 5B's FormXml reader path; Task 6 receives
   `snapshot.analysis_snapshot()` and alone consumes the BSL FormModule scan/
   read path. Every such verifier/read remains visible on the one injected
   reader spy and occurs only inside its owning provider call.

### Stage 1 — Platform XML and search seeds

1. As each existing bounded producer emits a raw typed request contribution,
   admit every artifact occurrence through the execution registry before
   appending it; only then canonicalize the separate application scopes.
2. Use Task 5B's `FormMaterialAssociationBuilderV1` to produce canonical
   provider Form scopes without conclusion identity; admit every raw
   Form/pair/member artifact inline before that builder groups or unions it.
3. Construct exactly one imported `MetadataCompositeQueryV2`, one imported
   Analysis `FormSourceSetQueryV2`, and one Task 6 CodeSearch query v3 when its
   semantic worklist is nonempty.
4. Register each imported typed query in `ProviderInvocationRegistryV2`; use
   the returned non-cloneable plan to stage Invocation/SourceGroup/Material
   association roots before each call.
5. Pass the plan, that byte-equal imported typed query, the matching provider
   port and the same execution context/snapshot/reader capabilities to the
   registry's exact typed invoke overload. The overload, not orchestration,
   calls the imported provider entrypoint once, validates and internally records
   exactly one returned terminal raw outcome plus full semantic groups, and
   returns the opaque recorded ID. Use the handle's sealed evidence closure to
   copy ordinary graph work. Reborrow a short-lived recorded handle only to
   create owned returned material/group roots, drop it before their mutable
   spelling recheck and map update, then call
   `record_stage_v2(&id, stage)`. A pre-I/O owner-defined error aborts
   the consumed staged plan/roots with zero invocation prefix. Task 7 performs
   no material read around the call.

### Stage 2 — mechanism bases and initial definitions

1. Classify only retained complete Platform XML groups.
2. For every raw candidate-mechanism contribution from exact Task 5B
   compatible facts, call its sealed complete gate before the mechanism
   builder inserts, unions, sorts or deduplicates it. The gate exhaustively
   admits `key.entry`, `key.handler`, every `owner`, both `from` and `to` of
   every `base_edge`, and every `entry_candidate`; only then build candidate
   base mechanisms.
3. Before adding a raw Mechanism association, call its sealed association gate
   to admit both `MechanismKey` artifacts and every raw association-material
   artifact. Only after that complete gate succeeds may the association builder
   insert, union, sort or deduplicate the association into its exact supporting
   groups.
4. Admit each raw Method work item inline before appending it to the work
   vector, then canonicalize Methods independently of origin sets.
5. Stop at the explicit Definition-boundary decision in section 7.1. Until A,
   B or C is approved, no final Definition plan/chunk construction,
   registration, invocation or receipt identity is normative here. Every
   candidate keeps per-Method origin association outside any eventual query.
6. After that decision is co-frozen, invoke Task 6 with the exact Platform context,
   `snapshot.analysis_snapshot()` and injected reader; only Task 6 may consume
   the reader through the owner-defined Analysis BSL scan/read boundary.

### Stage 3 — directed BFS

1. Keep the v6 level-synchronous, caller-once, directed caller->callee rules.
2. At every frontier, admit each application-derived Method/caller inline
   before appending it to a frontier/query builder; then canonicalize and
   construct CallGraph v3 queries from Methods only. Definition work is held as
   the canonical application Method worklist at the section-7.1 STOP boundary;
   after the approved choice it follows that one co-frozen plan/chunk contract
   and the same registration checks as Stage 2.
3. Query cache key is exact `(port, imported query_digest)`; depth is an
   application trace value.
4. A later origin reuses the raw outcome, admits each raw association/edge
   artifact before builder insertion, adds association, and replays retained
   edges without a call.
5. Every actual Task 6 call receives the same Platform context, derived exact
   Analysis atom and injected reader; cache replay performs no reader call.
6. Use only read-only `preview_application_admission_v3` for current retained
   graph work. Per-port/global final admission remains a pure rederivation in
   prepare/finish and never mutates a caller ledger.
7. Before changing state for every bounded traversal decision, register its
   complete expected requirement in the registry and consume the ID exactly
   once as completed or gapped. A cache replay is still a decision and cannot
   bypass this ledger.

### Stage 4 — Support

1. Build the v6 exploratory prefix plus all explicit proposal ownership
   subjects.
2. Admit every source-qualified artifact occurrence inline before appending its
   raw Support contribution; then group identical `SupportQuerySubjectV2`
   values before the imported 4,096 semantic-subject bound. Existing duplicate
   semantics are unchanged; union conclusion scopes only in
   `MaterialAssociationMapV2`.
3. Invoke the accepted Task5C-Evidence provider through exact
   `SupportStateQueryV2`; no Task5C parser/query encoder exists in Task 7.

### Stage 5 — rebuild and project

1. Use `preview_application_admission_v3` only while computing the Support/
   retained-graph fixed point. Discard every borrowed preview before any
   mutable registry operation. As the final association/traversal/mechanism/
   candidate/graph builders emit application-derived occurrences, admit them
   before insertion/sort/dedup; provider-derived repeats must match their
   committed response spellings.
2. After the fixed point and all invocation stages are complete, call
   `prepare_discovery_finality_v4` exactly once. It independently recomputes
   the same admission result, freezes collecting mutation and stores all
   retained decisions, occupied admission buckets, provider classes and the
   complete ordered association-requirement vector. The transition also proves
   every registered traversal decision resolved exactly once and seals that
   ledger internally; any Pending entry rejects.
3. Repeatedly call `project_next_finality_association_v4(&prepared)`. For every
   `Some(projection)`, let `association.rs` consume it transactionally into the
   map builder and return `AppliedFinalityAssociationProjectionV4`, then consume
   that token through `record_finality_association_v4`. Stop only on `None`.
   Skipped, duplicate, foreign or reordered requirements reject. No
   gap/effective DTO or root vector is returned.
4. Complete the nonempty-scope material map. Intersect roots, retained paths
   and proof obligations to build the one
   `UnsealedDiscoveryMaterialityPlanV4`; every finality root must be present in
   the map. Pass the plan plus immutable map to
   `seal_discovery_materiality_plan_v4` and retain only its branded seal.
5. Call only `finish_execution_v4(prepared, map, materiality_plan)`. Registry
   finish rederives admission/raw digests, constructs effective invocations and
   the complete provider/admission candidate set, validates all relations plus
   the internal traversal ledger, and internally chooses the exact 256/2,000
   sentinel branch and final materiality.
6. Move the resulting `FinishedAnalysisExecutionProjectionV4` into
   `AnalysisExecutionSnapshotV4::from_finished_execution_v4`; consume the
   snapshot into `FinalDiscoveryReportProjectionV4`, which derives
   `analysis_id_v4()` without any loose request/source/limits/catalog/traversal
   argument, then
   consume that projection through `DiscoveryReport::from_final_projection_v4`.
   No outside checks/verdicts/candidates/status recomputation exists.

## 7. Active-spec and Task 5A back-propagation gate

Before Task 7 production, active spec, ADR, plan and product tests state all of
the following without copying provider-owned parser tables:

- one whole composite-bound `PlatformCatalogContextV1`, the exact composite
  `SourceSnapshotV2`,
  its exact Analysis `SourceSetSnapshotV2`, the injected
  `&dyn SourceSnapshotPort` and both catalog-set digests;
- `EvidenceExecutionContext` passes exact owner capabilities to imported
  providers; Task 7 never resolves/reads registered material and contains no
  hidden reader; Metadata/Form own FormXml reads and Task 6 alone owns the BSL
  FormModule read;
- Metadata/Form query v2 and BSL query v3 exact owner/version boundaries;
- `MaterialAssociationMapV2` is the only boundary associating provider
  material/testimony with Request/Proposal/Mechanism; application-only
  traversal origin state and `TraversalGap` scopes remain legal;
- provider queries/groups/gaps/outcomes/cache contain no application scope;
- evidence admission v3 contains group/material roots, not conclusions;
- directed runtime roots and root admission are application-only;
- the exact Task5C-Evidence production OID is a Task 7 production prerequisite,
  not a Task 7 design or Task 6 prerequisite;
- the prerequisite/integration split from section 9;
- Task 8 is downstream and non-gating for this design package;
- public MCP/package/skill registration remains untouched.

The Task 5A/domain implementation must already export the exact v7
`AtomicSourceIdentityV2`, `SourceScopedArtifact`, artifact identity,
ProviderFact tags, group registry/encoder, gap scope, provider-outcome seams,
and the application/domain-owned `ArtifactIdentityBytesV1`,
`ExactArtifactSpellingRegistryV1`, staged-delta API from section 0.1 and the
closed non-serde `ProviderQueryAssociationViolationV1` with exact
`SourceGroupNotMember`/`MaterialNotMember`/
`PlatformCatalogExecutionMismatch` variants. Task
5A also owns the sealed raw-outcome staging and collected-outcome staged
validation operations from section 0.1; Task 5B owns only its private
material/group committed/staged rechecks and v7 catalog/provider usage
contract. This is a production
export inventory item only and
does not add Task 5A identity or status to the four-document design-freeze.
Task 7 does not keep the current string-only `source_set`, v1 group or flat
eager six-port adapter as a compatibility layer.

### 7.1 Definition application-plan/chunk boundary — explicit approval STOP

All non-Definition ownership above is final, but Definition has one remaining
product/architecture choice. The current upstream prose mixes a Task5A
full-work-plan call with Task7 per-chunk invocation identity; treating that
hybrid as already approved would be a logical error. Exactly one option must be
confirmed and then co-frozen into Task5A, Task6, Task7, active spec, ADR, tests
and generator before any production Rust:

- **A — one application plan with adapter-owned chunks (recommended).** One
  `DefinitionQueryPlan` contains the complete sorted-unique Method worklist,
  bounded at 4,160. Application calls `DefinitionPort` once. The adapter
  deterministically creates Task6 `DefinitionQuery` chunks of at most 2,000;
  an empty plan creates exactly one empty chunk. Task7 records one parent plan
  authority plus a canonical child invocation/receipt for each exact typed
  chunk digest, while cache entries remain per chunk. Finish proves ordered
  lossless plan-to-chunk coverage, exactly one adapter call, exactly one result
  per chunk and one complete plan-level outcome; orchestration cannot invoke a
  chunk directly.
- **B — application-owned per-chunk units.** Application deterministically
  chunks before the port boundary and each chunk is an independent scheduled
  invocation, identity, association and receipt. There is no parent full-plan
  call/authority; retries and partial failure are chunk-visible.
- **C — no chunking.** The complete Definition plan/query is bounded at 2,000
  Methods and is one invocation/identity/receipt. Worklists of 2,001..=4,160
  reject instead of being adapted.

Until the user selects A, B or C, the symbols `DefinitionExecutionPlanV1`, its
registry overloads, exact empty-plan behavior, cache/receipt parent-child
grammar and Definition acceptance goldens are deliberately absent. Any Task7
owner-final hash, production implementation, inferred default or claim that
the current hybrid is final is a hard STOP. This is the sole intentional open
choice after the clean-room Task7 review.

## 8. Exact file map for Task7PrerequisiteSliceV1

### Application

- Create: `crates/unica-coder/src/application/discovery/association.rs`
  - private builder, validation and goldens for `MaterialAssociationMapV2`;
    stores opaque validated roots, owns the map's sealed finish validator and
    canonical writer, and calls root identity/spelling operations without
    matching or constructing a variant;
  - pre-I/O roots validate through the plan-owned query authority; post-I/O
    roots validate from a full group/material owner through a short finished-
    handle borrow, then pass as owned opaque roots through the mutable spelling
    gate after that borrow ends;
  - implements the sole transactional `FinalityAssociationSinkV4`; consumes
    each ordered instruction projection and returns only its nonforgeable
    applied token, including exact SourceGroup-scope inheritance;
  - closed `MechanismAssociationContributionV1` and exhaustive pre-mutation
    gate delegating imported material/group checks to Task5B owner methods.
- Create: `crates/unica-coder/src/application/discovery/projection.rs`
  - sole rebuilt-graph/proof-obligation consumer that calls the three
    ports-owned checked materiality-input factories;
  - returns only `UnsealedDiscoveryMaterialityPlanV4`; owns no admission,
    effective-gap, sentinel, verdict, status or receipt-finality constructor.
- Modify: `crates/unica-coder/src/application/discovery/ports.rs`
  - import exact Task 5B/6/5C smart queries;
  - store exact borrows of the one Platform catalog context, composite snapshot
    and injected source reader in `EvidenceExecutionContext`;
  - pass context, `snapshot.analysis_snapshot()` and reader to Task 6 without a
    Task 7 material resolver/read path;
  - by-value generative typed invocation-plan registry with explicit
    `'context` borrows; `AnalysisIdentityPrefixV4` owns the sole canonical
    source and registry-header execution binding field; raw invocation/rollup
    has no conclusion scopes;
  - opaque `MaterialAssociationRootV2` over a module-private four-variant enum;
    plan/handle-only constructors plus exhaustive sealed identity/spelling
    methods, with no root/key/variant projection;
  - Finished entries retain full `SemanticAtomicEvidenceGroupV2` owners plus
    sealed post-invoke/read-only and retained-preview application views;
  - module-private `CurrentRootRelationV2`, registry-owned admission/finality
    derivation, `RegistryFinishValidationV4` with typed relation tokens and exact
    finished/map-root/effective/admission/provider-class ledgers;
  - one prepared ordered finality-requirement project-next/apply/record API with
    explicit mixed-kind order covering both admission buckets and location-
    erased provider gap classes; no
    recorded-handle gap projector or caller-visible gap/effective DTO;
  - closed six-owner `ProviderQueryAssociationAuthorityV2`; its five frozen
    variants move plan -> Finished entry and are borrowed only by recorded
    handles, while Definition remains reserved until section 7.1;
  - one public(super) application spelling ingress and private full-owner
    material/group rechecks, plus fixed three-variant violation mapping;
  - registry-owned expected/resolved traversal ledger; prepare rejects Pending
    decisions and finish alone builds the canonical `TraversalGapSetV3`;
  - exact `pub(super)` lifecycle visibility and static sibling call-site
    whitelist for constructor/register/invoke/recorded/stage/preview/traversal-
    register/resolve/prepare/finality-project-next/finality-record/materiality-seal/
    finish; no late catalog commit;
  - registry-derived raw/effective/admission/provider candidate construction,
    exact 256/2,000 overflow decision through Task5B's sole opaque union-count
    bridge, distinct one-shot model authorities and opaque consuming
    `FinishedAnalysisExecutionProjectionV4`; ports owns raw snapshot/rollup/
    trace private fields, the one-shot `AnalysisExecutionSnapshotV4` and its
    stored `AnalysisId`, and builds them only during finish/snapshot
    construction;
  - no local Metadata/Form/BSL/Support encoder.
- Modify: `crates/unica-coder/src/application/discovery/model.rs`
  - admission/effective-gap v3, closed application checks and typed preflight;
  - exact `pub(super)` registry-finality-only admission/provider/effective
    factories, each requiring its distinct ports-owned nonforgeable authority;
    no general gap clone, caller sentinel constructor or sibling finish facade;
  - `ProviderOutcomeSnapshot` owns retryable/raw token/raw digest; the effective
    invocation constructor consumes typed parts plus one-shot authority and
    computes coverage/effective digest internally; final materiality and the
    one-shot final report projection contain every final public field;
  - delete `EvidenceMaterialSubjectV2::Conclusion` and every provider-side
    conclusion field.
- Modify: `crates/unica-coder/src/application/discovery/traversal.rs`
  - application-only origins, root merge/bound, caller reuse and trace updates;
    register every expected bounded decision before mutation and consume its
    branded ID exactly once as completed/gapped; no local final gap vector.
- Modify: `crates/unica-coder/src/application/discovery/mechanisms.rs`
  - closed raw `MechanismContributionV1`, exhaustive spelling gate and exact
    Mechanism associations only after retained complete proof.
- Modify: `crates/unica-coder/src/application/discovery/evidence_graph.rs`
  - rebuild from retained groups; no raw association shortcut.
- Modify: `crates/unica-coder/src/application/discovery/proposal_validator.rs`
  - intersect map roots with exact proof obligations and retained paths.
- Modify: `crates/unica-coder/src/application/discovery/use_case.rs`
  - one snapshot capture/catalog build and by-value generative registry init, one
    injected-reader pass-through, imported query flow, preview/fixed point,
    prepare plus project-next/apply/record association loop, materiality seal,
    single finished seal and consuming final-report projection;
- Modify: `crates/unica-coder/src/application/discovery/determinism.rs`
  - project-discovery-v4, material-map v2, traversal v3, admission v3 encoders;
    owns private `CanonicalIdentitySinkV4`, typed equality/order tokens and the
    only hash finalizer; exposes that finalizer only to the ports-owned
    snapshot constructor and preserves the existing unnested field frames.
- Modify: `crates/unica-coder/src/application/discovery/contract.rs`
  - prerequisite-only optional CFE assertions required by section 9; no Task 8
    concrete type.
- Modify: `crates/unica-coder/src/application/discovery/mod.rs`
  - declare `association` and `projection`; focused
    contract/metamorphic/corpus tests.

### Infrastructure and corpus

- Create/modify the private Task 7 v6 provider composition and 48-case corpus
  files exactly as its non-conflicting sections require.
- Composition borrows the one catalog context and accepted providers; it does
  not parse, select roots, attach scopes or register MCP.
- Corpus fake ports receive only provider query values. The case evaluator
  injects associations on the application side and asserts provider-invariance
  under a second equal proposal.

### Active documentation and guards

- Modify: `spec/architecture/extension-point-discovery.md`.
- Modify: `spec/decisions/0008-project-discovery-and-discovery-receipts.md`.
- Modify: `docs/superpowers/plans/2026-07-17-project-discovery-receipts.md`.
- Modify: `tests/ci/test_product_contracts.py`.
- Record immutable design/review/implementation/evidence identities in the
  accepted project-discovery ledger using their correct 64/40-hex classes.

Do not modify public MCP schemas/registry, plugin metadata, `.mcp.json`, skills,
provenance or package contracts in Task 7.

## 9. Acyclic Task 7 prerequisite / Task 8 integration split

### 9.1 Task7PrerequisiteSliceV1 — implemented before Task 8

The accepted Task 7 production commit is independently useful and contains:

- all Task 7 v6 + this addendum discovery orchestration, eight mechanisms,
  corpus, associations, traversal, admission and report behavior;
- optional raw `Context`/`IsFunction` assertions for CFE patch proposals rather
  than synthetic defaults;
- exact Configuration+PlatformXml mutation preflight and the existing closed
  proposal-only `DiscoveryPreflight/mutation_preflight` tuple;
- zero-material-read prepare orchestration before watched capture;
- generic injected resolver/issuer ports and recording fakes sufficient to
  prove call order, all-or-nothing eligibility and zero calls on preflight
  failure;
- continued report/evidence output when concrete mutation resolution is not yet
  available.

It contains no Task 8 module import, `ResolvedMutationPlan`,
`ResolvedProposalMutationPlan`, concrete CFE renderer, writer, WAL, lease,
backend, or claim that resolver-ready issuance is implemented. Its recording
fake is not production receipt evidence.

The generic preparation boundary carries normalized proposal identity, exact
analysis source and optional assertions but has no snapshot/material/filesystem
reader. Configuration-only preflight runs before provider interpretation. Task
8 later derives context/kind/async from exact source material; Task 7 does not
guess defaults.

### 9.2 Task7Task8IntegrationV1 — delivered by Task 8

The later Task 8 implementation commit owns:

- concrete resolution from prepared CFE requests to the accepted
  `ResolvedProposalMutationPlan` type;
- sorted all-or-nothing plan delivery to the issuer assessment seam;
- exact resolver blockers/diagnostics without erasing the Task 7 report;
- modifications to discovery application files needed for this concrete wire;
- distinct immutable `task7_task8_integration_evidence_sha256` proving the
  real integration, separate from Task 7 prerequisite tests and Task 8 writer
  evidence.

Task 8 may not redefine Task 7 provider query, association, traversal,
admission, preflight or mechanism semantics while wiring this seam. If a
semantic prerequisite is missing, it is returned through a versioned successor
and reviewed; it is not smuggled into the Task 8 integration commit.

An undifferentiated “full Task 7 including concrete Task 8 integration must be
complete before Task 8” gate is a dependency cycle and a hard STOP. Conversely,
the absence of Task8 integration is not a defect in
`Task7PrerequisiteSliceV1`.

## 10. RED -> GREEN implementation sequence

### 10.1 Co-frozen contract conformance

- [ ] Run the section-14 external protocol over the exact Task 4 v7, Task 5B
  v7, Task 6 v7 and Task 7 v7 owner tuple; owner files publish no local status
  or tuple hashes.
- [ ] Reproduce Task 6's six published query-v3 positives and one extra-frame
  negative with the external two-path generator against that same tuple; Task
  7 imports only the typed digest bytes.
- [ ] Run fresh owner self-audits and separate independent reviews over the
  exact tuple, then transition one atomic package ledger or restart all
  affected evidence after any owner-byte edit.
- [ ] Add compile/static REDs for local catalog/query encoders, detached context,
  string-only source identity and application scope in provider types.
- [ ] Obtain explicit A/B/C approval for section 7.1 and replace its symbolic
  Definition STOP with one exact Task5A/Task6/Task7 plan/chunk/invocation/cache/
  receipt grammar plus empty/N/N+1 goldens. Until then no owner-final Task7
  hash, package transition or production Rust is permitted.

### 10.2 Material association map

- [ ] Add all section-0.1 exact-spelling REDs in both orders. Prove the request
  or request/catalog prefix fails inside generative registry initialization
  after at most the one context build but before downstream evidence-provider
  I/O, and live/cached/cross-invocation conflicts
  invalidate atomically before any raw snapshot, map, report or receipt can be
  accepted. Prove the spelling registry is owned by
  `ProviderInvocationRegistryV2`, request plus both catalogs commit exactly
  once before the execution closure, a collision exposes no registry/ID, and no
  late commit method or separate delta/outcome commit is observable. Prove each
  existing bounded application producer admits every
  raw occurrence inline before builder insertion or query/Support/association/
  traversal/mechanism/candidate canonicalization, including complete Stage 2
  mechanism/association material. Mutate each of `MechanismKey.entry`,
  `MechanismKey.handler`, `owners`, `base_edges.from`, `base_edges.to` and
  `entry_candidates` independently and prove the collision is rejected before
  the mechanism builder changes; mutate the association key and each raw
  association-material artifact and prove the same before the association
  builder changes. Compile-fail/static REDs add a new artifact-bearing raw
  mechanism field, a fifth private association-root inner variant, a direct
  construction/destructure/key-extraction attempt from every non-`ports.rs`
  module, and a direct/nested
  artifact member to each imported Task5B material/group type without extending
  the corresponding exhaustive sealed gate. Prove no discovery sibling outside
  the five-call-site whitelist can invoke a gate and no sibling can access the
  registry field. Prove `InvalidArtifact`, `Collision`, missing committed
  occurrence and stale staged baseline map to their four exact nonretryable
  Task7 reasons with the frozen raw-bound -> spelling -> semantic/resource
  priority. Return one response containing both a cross-execution spelling
  collision and a missing planned subject; require the spelling reason before
  any completeness error. Mutation REDs omit each raw record/gap/nested
  occurrence from `stage_complete_raw_artifact_spellings_v1`, invent or
  respell an artifact during collection, omit the staged-authority ingress
  require, or omit the final collected-outcome check. Catch/ignore each returned spelling error and prove
  `register_*`, `invoke_*`, `recorded()` and `finish_execution_v4` still reject and no
  execution projection exists. Prove each
  typed registration calls its
  owner-defined sealed exhaustive member recheck before slot allocation. Prove
  Task 5B and Task 6 use the shared primitive on
  unsorted raw occurrences before classification/grouping/ceilings. Run each v6 row-24
  spelling variant in isolation and require byte-equivalent query/group/
  admission bytes, then combine the variants under one Analysis source and
  require rejection in both orders; byte-identical and different-source pairs
  remain valid. Every published provider-query golden stays unchanged.
- [ ] Add exact empty/one/two-scope numeric goldens and all four cross-owner
  SourceGroup/Material/AtomicGroup/Mechanism structural goldens from section
  3.4.
- [ ] Add constructor REDs for empty entry scopes, duplicate final roots,
  foreign invocation/source/material/group, forged digest and unknown proposal/
  mechanism. For each of the five frozen query families and the Definition
  family selected in section 7.1, prove registration mints exactly one owner
  authority, its digest/enum-implied port equals the typed
  query/key, the non-Clone value moves plan -> Finished entry, and the recorded
  handle only borrows it. Independently mutate an omitted/foreign source group,
  every query-time material member, returned-only material, atomic group,
  authority variant, port and digest; reject Clone/serde/raw construction,
  digest-only membership and a second locally reconstructed member set. Compile
  attempts outside `ports.rs` to name the private root enum, construct/switch/
  destructure a root, extract/clone its invocation key, or build a foreign
  Material/AtomicGroup directly must fail; `format!("{root:?}")` and
  `format!("{root}")` must also fail because the root/key/inner enum have no
  `Debug`/`Display`. Cloning an already validated opaque root remains legal and
  byte-identical.
- [ ] Add current-root and registry-owned admission/finality REDs. Require the
  preview and prepare paths to call the same pure derivation and produce the
  same retained/drop decision; mutation-test any divergence. For every occupied
  `(PerPort|Global, port)` require exactly one bucket across the execution, the
  exact owner/root sets, and `Artifacts(ProviderMaterialArtifactSetV2)` equal to
  the exhaustive union of real group artifacts including both pair halves.
  Prove callers cannot supply/extract reason, port, owners, roots, scope,
  retained prefix or gap DTO. Replay prior-execution roots and accept them only
  when current Finished query/outcome/spelling/binding authorities independently
  prove the exact value. After prepare, reject every register/invoke/stage or
  second prepare. Sort the mixed finality requirements by the exact kind-1/
  kind-2 plus full-owner identity grammar. Repeatedly project only the next
  requirement, transactionally consume its private RequirePresent/
  InheritScopes instructions in `association`, and record only the resulting
  nonforgeable applied token; reject skip, duplicate, reorder, foreign nonce/
  generation and missing/extra requirement. Require every target in the final
  nonempty map and exact SourceGroup-scope inheritance. Mutate a non-AtomicGroup
  admission root, missing/foreign owner, mixed port, absent current full group,
  compact-ID material projection or material set and reject before any finished
  DTO. All frozen root/map/admission bytes remain exact.
- [ ] Add finish-owned provider-gap/effective/trace constructibility REDs. For
  every raw gap scope require the private finality derivation's QueryWide ->
  Invocation, SourceSetWide -> exact SourceGroup and Artifacts -> complete exact
  Material-root mapping. Two valid raw gaps differing only by location remain
  separate raw rows but form one effective class; reason/scope/root changes form
  distinct classes. Require exact raw-row-to-class surjection and complete
  provider/admission candidate ownership without exposing class tokens or
  effective DTOs to orchestration. Prove `ProviderOutcomeSnapshot` alone owns
  `retryable`, raw token and raw digest; mutate testimony or stored digest and
  require finish's independent rebuild to reject. Prove every `model.rs`
  factory is `pub(super)`, consumes its own nonforgeable ports-owned authority
  and computes local order/coverage/digest without a sibling-private field
  read. Test exact internal thresholds: the private finish counter is the sole
  caller of Task5B's opaque union-cardinality API, overlaps/permutations count
  distinct artifacts once, and 256/2,000 retains all candidates; 257/2,001 yields one rootless
  Admission sentinel only after validating every hidden candidate/map root;
  raw/admission histories remain. Reject every missing/extra/duplicate raw/
  effective/trace/gap/map/root relation. The sole result is
  `FinishedAnalysisExecutionProjectionV4`; the snapshot accepts only that seal,
  `analysis_id_v4()` accepts no detached identity fragment, and the consuming
  final-report projection is the sole path to checks/verdicts/candidates/status/
  receipt eligibility. Golden-test the
  write-only sink chain and reject any Task7 `&mut Vec<u8>`, byte/length/slice
  getter or extra nesting frame.
- [ ] Add the shared-pair RED: both real halves and both AtomicGroups receive
  exact scopes, while one half shared by two proposals unions both scopes.
- [ ] Add invocation-lifecycle REDs for typed plan before I/O and the five
  frozen exact typed registry invoke overloads, then the Definition overload
  selected in section 7.1. Prove each overload consumes only its
  matching plan/query/provider and registry-retained execution context, checks
  byte-equal planned query
  authority, itself calls once, internally records exactly one raw terminal
  outcome plus complete full semantic groups and returns an owned recorded ID.
  Prove the sealed recorded/preview visitors permit ordinary owned graph work,
  cannot return a borrow or finality authority, and finish ignores copied work;
  no caller-visible terminal recorder or raw outcome ingress exists.
- [ ] Static-compile scan the exact `ports.rs` lifecycle boundary: the
  generative closure receives registry by value and can consume finish without
  unsafe/placeholder extraction; all context-bearing query paths pass under
  `deny(elided_lifetimes_in_paths)`. Constructor, the five frozen register/
  invoke families plus the approved Definition family,
  recorded, stage, preview, prepare, project-next, finality-record,
  materiality-seal and finish are exact `pub(super)` and called only by
  `use_case`; traversal register/complete/gap are exact `pub(super)` and called
  only by `traversal`. Material/group `require_*` helpers remain private to
  `ports.rs`, and the application admission spelling operation has no caller
  outside the five named siblings;
  only `association` calls plan/finished-handle root constructors, opaque root
  identity/spelling and finish-context association validation; only
  `association` alone implements the finality sink and calls the consuming
  `apply_finality_associations_v4`; it cannot match instructions or mint an
  applied token on failure. Only private registry finish can mint the distinct
  admission/provider-effective/effective-invocation/outcome/materiality model
  authorities; no sentinel constructor has a
  production caller. The private current-root/finality helpers, finish
  constructor/finalizer,
  `CurrentRootRelationV2` and relation state never leave `ports.rs`. Enforce the
  exact root/key/root-vector/outcome/map/model/projection/snapshot sink chain
  from section 4.2; only `determinism` owns/finalizes the sink, and no other
  projection field/token/byte/length/slice getter exists. Assert there is no
  late catalog commit symbol and no Task7 writer accepts `&mut Vec<u8>`.
  Each owner's
  `association_authority_v1` is called exactly once and only inside its matching
  `register_*`; no orchestration/builder can mint a second authority directly.
- [ ] Register the same exact typed `(port, query_digest)` twice in every one of
  the five frozen overload families and the approved Definition family, and
  require nonretryable
  `duplicate_provider_invocation_plan` before a second slot, association or
  provider call exists. Mutate the private entry/map/slot relation in a module
  test and reject every missing, divergent, reused or overflowed relation.
- [ ] Add same-port swapped-query and malformed empty-Complete response REDs,
  plus foreign-execution nonce/brand, foreign slot, unfinished slot, dangling,
  duplicate and phase-wrong recorded-ID REDs. Prove short recorded lookups do
  not block a later mutable invoke; post-response/later-origin/mechanism
  additions accept only the registry-borrowed handle; finish proves the typed
  plan/typed invoke/raw/recorded-ID/Invocation-root bijection and pre-I/O error
  rollback leaves zero prefix.
- [ ] Add provider-invariance metamorphic RED: a second equal proposal leaves
  query/call/raw/group/gap/admission order and retained prefix byte-identical;
  request/map/traversal trace/public proposal conclusions/analysis may change.
- [ ] Implement the private builder/immutable map/encoder minimally and make
  those REDs GREEN.

### 10.3 Imported queries and one context

- [ ] Add recording RED proving one `PlatformCatalogPort` build and all four
  provider families borrow that context.
- [ ] Add recording/compile RED proving `EvidenceExecutionContext` carries the
  exact one context/composite-snapshot/injected-reader triple; Task 6 receives
  the derived exact Analysis atom, while Task 7 cannot name an opaque material
  item/verification or call a specialized reader.
- [ ] Add a no-hidden-reader RED: every registered-material verifier/read count
  appears on the injected `SourceSnapshotPort` spy and only behind its owning
  provider call. Metadata/Form FormXml reads remain legal there; every BSL
  FormModule read is specifically behind Task 6.
- [ ] Add Metadata/Form v2 mutation/golden imports and reject the v6 duplicate
  scope encoder.
- [ ] Compare imported Task 6 query-v3 digest accessors with all six published
  digest values; reject v2 bytes, the extra-frame negative and cache reuse.
- [ ] Add Support query-v2 subject dedupe with two proposal associations and one
  provider invocation.

### 10.4 Raw invocation, trace and admission v3

- [ ] Add static/compile REDs rejecting `ConclusionScope` in provider query,
  group, gap, raw/effective provider outcome and cache DTOs.
- [ ] Add independent raw-rollup, effective-digest, trace-stage,
  EffectiveGap-owner/reason and complete AnalysisExecutionSnapshotV4 encoder
  goldens; mutate every tag/frame/depth/round/registry position and reject
  duplicate or noncanonical vectors.
- [ ] Toggle only raw `retryable` and require different raw/effective/analysis
  digests. Independently encode all three raw ProviderGap scopes, location
  None/Some, invalid location combinations and every raw-vs-admission tag-order
  negative.
- [ ] Add caller-reuse RED proving one raw invocation, two app stages/origins
  and unchanged provider bytes. Recording the same valid stage twice from two
  origins at the same depth is an idempotent no-op; a new valid stage inserts,
  while a phase/port/depth/round-invalid stage still rejects. Duplicate final
  trace rows remain illegal.
- [ ] Add per-port/global N/N+1 REDs proving scope changes do not reorder or
  split groups.
- [ ] Add effective-gap REDs for QueryWide/SourceSetWide/Artifacts/dropped
  group/sentinel exact root projection and final orphan filtering. Through
  private module mutation only, alter the internally derived analysis-level
  `EvidenceGapLimit` identity, its effective row or its exact check; reject
  missing, duplicate, non-sentinel or mismatched combinations while proving no
  public/orchestration input can choose that option.
- [ ] Implement v3 admission and project-discovery-v4 snapshot encoding.

### 10.5 Traversal, mechanisms, Support and corpus

- [ ] Run every non-conflicting Task 7 v6 traversal/mechanism/corpus RED.
- [ ] Add registry-owned traversal completeness REDs: every bounded decision
  registers before state mutation and consumes its branded ID exactly once as
  completed or gapped. Reject Pending at prepare, double resolution, foreign
  execution/slot, gap/spec reason/phase/round/depth/scope mismatch, missing or
  extra artifact spelling, duplicate canonical gap row and any caller-built
  final vector/set. Prove `expected == completed + gapped` and that finish moves
  exactly the sealed Gapped rows without a traversal argument.
- [ ] Add runtime-vs-context root and merged-origin-before-bound REDs using the
  new map.
- [ ] Implement the staged flow with imported queries and association updates.
- [ ] Freeze the 48-case corpus before mechanism behavior GREEN; never rewrite
  expectations from output.

### 10.6 Prerequisite split and documentation

- [ ] Add compile/static RED proving `Task7PrerequisiteSliceV1` imports no Task
  8 concrete type/module.
- [ ] Add zero-read prepare, optional assertion, Configuration-only preflight
  and recording resolver/issuer tests.
- [ ] Add product tests for the exact DAG, OID gate and downstream integration
  ownership.
- [ ] Synchronize active spec/ADR/plan without editing immutable historical
  designs.

## 11. Required acceptance matrix

Acceptance requires all non-conflicting Task 7 v6 rows plus:

1. immutable base SHA matches the header;
2. Task4/Task5B/Task6/Task7 candidates, audits and independent reviews refer to the
   same exact bytes and are accepted atomically;
3. external generator evidence mechanically reproduces Task 6's six published
   v3 query digests plus the extra-frame negative for the exact owner tuple,
   while Task 7 imports only those digest bytes;
4. one whole composite-bound catalog context and both set digests are
   execution-bound; `AnalysisIdentityPrefixV4` owns the sole registry/header
   96-byte binding field (query authorities own only their sealed copies), and
   the five frozen query authorities plus the approved
   Definition authority revalidate through the exact Task5B/Task6 variant
   dispatch;
5. Metadata/Form v2 and BSL v3 query digests are imported byte-for-byte;
6. Support query v2 dedupes equal subjects independently of conclusion scopes;
7. every provider query/group/gap/raw outcome/cache static scan contains no
   application conclusion identity;
8. all four association roots validate an exact live plan or recorded handle
   appropriate to their phase; the five frozen typed registry invoke overloads
   plus the Definition overload selected in section 7.1 alone
   call providers and internally record returned outcomes plus complete full
   semantic-group owners, sealed application views cannot escape a borrow or
   become finality authority, and finish proves the
   typed-plan/typed-invoke/raw/recorded-ID/Invocation-root bijection; later
   cached/mechanism contributions store only the owned recorded ID and reborrow
   only its checked finished registry entry; duplicate exact registration in
   any family rejects atomically before a second slot/association/provider I/O;
9. empty/one/two-scope numeric map goldens plus all cross-owner root/scope and
   shared-pair structural goldens reproduce exactly;
10. duplicate raw contributions union only in the application builder;
11. a second equal proposal preserves every provider query/call/raw/group/gap
    and admission-order/retained-prefix byte; only the normalized request,
    map, traversal origins/trace, public proposal conclusions and enclosing
    analysis identity may change;
12. provider/local/per-port/global admission order ignores the map;
13. admission v3 contains roots/material only and no conclusion variant;
14. provider gap scope maps exactly to Invocation/SourceGroup/Material roots;
15. dropped groups map exactly to their owner-specific AtomicGroup roots;
16. overflow sentinel derives final scopes from the map without embedding them;
17. orphaned associated issues project empty public affects;
18. merged equal roots retain all origins before the root bound;
19. Request/Proposal origins never create runtime reachability;
20. later cached-caller origin causes no second provider call;
21. application trace records the exact canonical stage/depth/round bytes
    outside provider identity and has a one-to-one invocation-key set; repeated
    equal valid stage insertion is idempotent, while incompatible stages and
    duplicate final trace rows reject;
22. raw rollup including `retryable` and all exact raw-gap/location bytes, v3
    effective digest, effective owner/reason and v4 analysis encoders reproduce
    independently; the analysis snapshot binds both catalog sets, complete
    additive registry, trace, map, admission and traversal, while report output
    is obtainable only from the consuming final-report projection stored in the
    finished seal;
23. exact Task5C Evidence OID is required only for Task 7 production;
24. Task 6 imports no Task5C type/OID;
25. Task5C-Evidence imports/revalidates no Task 7 design or implementation;
26. Task7PrerequisiteSliceV1 is independently implementable with zero Task 8
    concrete import;
27. Task7Task8IntegrationV1 is explicitly owned/evidenced by Task 8 delivery;
28. active spec/ADR/plan/product tests state this same DAG and identity split;
29. all 48 corpus cases plus every permanent v6/v7 RED pass;
30. public MCP/package/skill surface is byte-unchanged.
31. `EvidenceExecutionContext` carries the one exact Platform context,
    composite `SourceSnapshotV2` and injected `&dyn SourceSnapshotPort`; every
    Task 6 call derives the same exact Analysis `SourceSetSnapshotV2` without
    substitution;
32. Task 7 contains no registered-material view/ref/resolver/handle/read chain;
    Task 5B providers own FormXml reads, only Task 6 calls the injected BSL
    FormModule scan/read boundary, and no hidden reader capability exists.
33. nonretryable semantic mismatch, retryable post-validation external drift,
    zero-prefix behavior and the exact injected-reader counter matrix are
    preserved verbatim and never reclassified by Task 7 association/admission:
    Ordinary Present 0/1/1, Registered Present 1/1/1, Registered Missing
    1/0/0, and Registered NotApplicable, unsupported Ordinary or per-item
    `FileBytesLimit` 0/0/0 for verifier/read/parse calls respectively;
    `FileBytesLimit` is nonterminal and later selected items remain eligible,
    while terminal `FileCount`/`TotalBytes` omit the suffix and allow no later
    material I/O.
34. PerPort and Global admission produce exactly one row for every occupied
    `(reason, port)` bucket across the whole execution; only the final
    EvidenceGapLimit sentinel is portless.
35. `EffectiveProviderInvocationV3.admission_gaps` contains only PerPort/Global
    gaps that own its exact invocation key; one registry-derived Global bucket
    is fanned out internally exactly once to each and only each listed owner.
    The one EvidenceGapLimit sentinel is selected only at 257 candidates or
    2,001 material subjects, after all hidden candidates/map roots validate;
    it is stored once in `analysis_evidence_gap_limit`, encoded by its exact
    admission identity, projected once at analysis level as the sole effective-
    gap row with one exact check, never once per invocation, and rejects every
    missing/mismatched direction or accompanying effective row.
36. the Task 5A/domain-owned `ExactArtifactSpellingRegistryV1`, keyed exactly
    by `(AtomicSourceIdentityV2, ArtifactIdentityBytesV1)`, is owned by
    `ProviderInvocationRegistryV2`, seeded from request and both catalogs before
    downstream provider I/O, and covers every nested provider material/gap/
    group, association, traversal, mechanism and candidate before
    canonicalization; Task 5B and Task 6 use its shared staging primitive on
    raw occurrences before local sort/dedup/group/ceilings; Task 5B's sole
    sealed zero-I/O `stage_complete_catalog_spellings_v1` projection covers every
    context-retained catalog/nested-material/Analysis-BSL occurrence without
    exposing an iterator/path/witness/handle, and the generative Task 7
    constructor commits request plus that opaque delta before exposing its
    closure; no later catalog commit exists. Every application-derived
    occurrence is admitted inline by its existing bounded producer before
    builder insertion or query/Support/association/traversal/mechanism/
    candidate canonicalization, and typed registration uses the sealed Task5B/
    Task6 owner method to recheck every private final artifact member; each typed invoke
    atomically commits its staged response delta with `Staged -> Finished`,
    while collision yields `Invalidated`, no recorded ID and no accepted
    snapshot/map/report/receipt across live/cache/cross-invocation paths; v6
    row-24 variants remain byte-equivalent when isolated, but concurrent
    same-source byte-different variants reject in either order; byte-identical
    repeats, different-source identities, valid query bytes and goldens remain
    unchanged.
37. each of the five frozen typed query owners plus the approved Definition
    owner mints one opaque non-Clone/non-serde association authority; Task 7's
    closed enum verifies exact variant/port/
    digest, retains it in the registered plan, moves that same value into the
    Finished entry and exposes only a short borrow through the recorded handle;
    pre-I/O source/query-material roots validate through it, post-I/O material/
    group roots additionally validate against the exact recorded outcome, and
    no digest-only, caller-list, locally reconstructed or serialized membership
    path exists;
38. the generative closure owns the registry value, all Metadata/Form/Support
    query paths carry explicit context lifetimes, and every `ports.rs` lifecycle
    operation has exact `pub(super)` visibility and static call-site whitelist:
    constructor/register/invoke/recorded/stage/preview/prepare/project-next/
    finality-record/materiality-seal/finish only from use_case and traversal
    register/complete/gap only from traversal, plus owner-side validators and
    sink writers; no late commit, recorded-handle gap projector,
    caller gap/effective factory or sentinel constructor exists;
    no bare-private compile break, sibling-private read, key/byte projection or
    wider crate/public escape is accepted;
39. `MaterialAssociationRootV2` is an opaque `pub(super)` struct owned by
    `ports.rs` over a module-private four-variant enum; only plan/finished-handle
    methods construct it, association code can only clone/store it and call its
    sealed exhaustive identity/spelling operations, no key/variant/member can be
    extracted, replaced or observed through `Debug`/`Display`, and all v2
    root/map bytes and goldens remain exact;
40. final association-map roots and every admission/effective-gap root are
    revalidated through the current Finished registry entry and its exact moved
    query capability/current recorded outcome/binding before acceptance;
    prepared finality accepts AtomicGroup admission roots only, derives exact
    same-port owner-set equality from full groups itself, exposes no envelope
    field, sorts mixed requirements by explicit kind/full-owner identity, and
    requires every project-next instruction transaction applied once. Every
    RequirePresent target exists and every InheritScopes target receives the
    complete source scopes in the final map. Nonce/generation/slot remain
    validation state and enter no canonical identity;
41. provider `EffectiveGapV3` is constructed only inside registry finish from
    the complete rederived raw-gap-class set; raw location-distinct gaps surject
    exactly onto unique location-erased effective classes. Raw retryability,
    raw token and raw digest belong to `ProviderOutcomeSnapshot` and are
    independently rebuilt at finish; each model factory consumes a distinct
    nonforgeable ports-owned one-shot authority and computes its local shape/
    digest. Raw/rollup/trace are ports-owned,
    every sibling DTO is validated/encoded only by its owner, and consuming
    `finish_execution_v4` emits the sole complete analysis projection accepted
    by the v4 snapshot while preserving exact unnested canonical bytes.
42. preview and prepare invoke the same pure admission derivation; prepare
    disables every collecting mutation, seals an exact expected/resolved
    traversal ledger with no Pending entries, and orchestration cannot supply a
    retained prefix, admission/effective DTO, digest, candidate vector,
    sentinel option, execution binding, traversal vector or analysis-ID prefix.
43. every Task7 root/key/map/model/projection writer receives only
    `CanonicalIdentitySinkV4`; only `determinism.rs` owns/finalizes its private
    bytes, the prefix binding and admission material set each traverse their
    exact typed adapter once, and typed identity tokens compare complete private
    bytes while exposing no bytes, length, slice, serde, Debug or Display.
44. Task5B full `SemanticAtomicEvidenceGroupV2` is the only admission material
    projection owner; compact group ID is identity/root only. The private finish
    counter calls opaque `canonical_union_cardinality_v2` exactly once over the
    complete pre-sentinel Artifacts scopes; overlaps and permutations count
    once, empty is zero, and only finish compares the returned u32 with 2,000.
45. traversal proves `expected == completed + gapped`, every branded decision
    resolves once, prepare rejects Pending/foreign/duplicate/mismatched state,
    and finish constructs `TraversalGapSetV3` internally with no caller set.
46. the sealed materiality plan is complete against the final map and proposal
    set; finish alone combines it with effective gaps/sentinel and creates final
    checks/verdicts/candidates/status/receipt eligibility, and a one-shot report
    projection is the only constructible `DiscoveryReport` path.
47. section 7.1 has been replaced after explicit user approval by exactly one
    A/B/C Definition boundary with complete empty/N/N+1, invocation/cache/
    association/receipt and plan-to-chunk completeness goldens; no hybrid
    semantics remain.

## 12. Verification gate

Before claiming Task 7 production acceptance, run and record:

```text
python3.12 -m unittest tests.ci.test_project_discovery_corpus -v
python3.12 -m unittest tests.ci.test_product_contracts -v
cargo test --locked -p unica-coder application::discovery -- --nocapture
cargo test --locked -p unica-coder
cargo clippy --locked -p unica-coder --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
git diff --check
```

Also run every accepted focused Task 5B v7, Task 6 v2-v7 and Task5C-Evidence
provider/query/golden suite. Product/static scans reject local query encoders,
old BSL v2 live construction, detached catalogs, Form path inference,
conclusion fields in provider boundaries, a Task8 import in the prerequisite
slice, a Task7 reverse dependency in Evidence, any Task 7 call to the
registered-material reader, or a reader-bearing context/view/ref/resolver/
handle outside the explicit `EvidenceExecutionContext.source_reader` field.
The reader spy additionally attributes every FormXml call to Metadata/Form and
every FormModule call to Task 6; it does not globally forbid Task 5B provider
reads.

A green subset, copied golden, ignored test, self-audit or moving-tree hash is
not acceptance evidence. Independent code/spec review names the exact accepted
implementation commits and command outputs.

## 13. Hard STOP conditions

Stop and show the owner if any condition is true:

- immutable Task 7 v6 SHA differs from the header;
- an owner file declares its own package status/hash, or the external ledger
  names an inconsistent Task4/Task5B/Task6/Task7 tuple;
- Task 6's six v3 positive digests or extra-frame negative differ from section
  2.3, external generator evidence names a different owner tuple, or Task 7
  reconstructs the payload/framing instead of importing typed digest bytes;
- Task7 production starts without any exact required implementation OID;
- Definition production or owner-final co-freeze starts before explicit A/B/C
  approval replaces section 7.1, or the existing full-plan/per-chunk hybrid is
  silently treated as final;
- Task7 design freeze or Task6 is made to wait for Task5C-Evidence;
- Task5C-Evidence imports/revalidates Task7 or waits for its implementation;
- Task8 design/implementation is used as a Task4/Task5B/Task6/Task7 co-freeze gate;
- a provider receives or emits Request/Proposal/Mechanism identity;
- proposal ID, depth, round or application stage enters an imported query
  digest or cache key;
- Task 7 reconstructs Metadata/Form/BSL/Support query bytes;
- only one half of `PlatformCatalogContextV1` is borrowed or the catalog is
  rebuilt/reparsed;
- `EvidenceExecutionContext` omits/substitutes the captured composite
  `SourceSnapshotV2` or injected `&dyn SourceSnapshotPort`, a Task 6 call
  receives a different Analysis atom/capability, or Task 7 calls a specialized
  registered-material reader itself;
- `PlatformCatalogContextV1` or a material view/ref/resolver/handle contains a
  hidden reader, `EvidenceExecutionContext` contains any reader other than the
  explicit injected `source_reader`, or registered-material counters do not
  belong to that injected reader;
- Task 7 makes an internal semantic mismatch retryable, maps any condition
  other than post-validation external filesystem drift to
  `source_fingerprint_mismatch`, fails to map that exact external drift to
  `source_fingerprint_mismatch`, retains a partial invocation prefix for either
  failure, or lets association/admission alter the imported reader-call/error
  matrix;
- registered Form authority/version/digest is absent from Form/BSL query
  freshness, or a FormModule path is formatted;
- association roots can name foreign invocations/sources/material/groups;
- a final association root bypasses current Finished-entry/query-authority/
  recorded-outcome/spelling revalidation, a stale root is accepted solely
  because it was once constructed or has an equal key, or execution nonce/slot
  is added to root equality/order/canonical bytes instead of treating the root
  as revalidated immutable identity;
- `EvidenceAdmissionGapV3` or `EffectiveGapV3` matches/decodes a private root,
  accepts a caller-supplied envelope/vector, admits a non-AtomicGroup admission
  root, a root whose derived owner set is not exactly the same-port bucket set,
  a root not proved by the current Finished outcome/binding, implements a
  general `Clone`, or has any construction path outside private registry finish
  plus its exact model owner constructor;
- preview and prepare do not use the same pure admission derivation, an occupied
  `(reason, port)` does not yield exactly one execution-wide bucket, a caller
  supplies/extracts reason/port/owner/root/scope/retained prefix/gap, Task5B's
  `ProviderMaterialArtifactSetV2` is projected from a compact group ID instead
  of full `SemanticAtomicEvidenceGroupV2`, omits a real group half or exposes
  membership, its union cardinality is counted by roots/per-set lengths/a
  member iterator or any caller other than the exact private finish counter,
  collecting mutation remains legal after prepare, or a finality requirement
  lacks the exact mixed-kind/full-owner order, can be skipped/duplicated/
  reordered/replayed across nonce/generation, exposes roots instead of sealed
  instructions, inherits incomplete SourceGroup scopes, or can be recorded
  without a successfully applied token;
- provider effective-gap construction accepts a detached owner/provider/reason/
  scope/root/class vector, occurs outside private finish, maps
  QueryWide/SourceSetWide/Artifacts to anything other than exact
  Invocation/SourceGroup/Material roots, treats location-distinct raw gaps as
  requiring duplicate identical effective rows, loses either raw history row,
  or omits the exact raw-row-to-unique-class surjection;
- orchestration constructs a key-bearing raw/effective/trace DTO, records a
  stage without its current recorded ID, supplies retained records, coverage,
  raw/effective digest, key, gap, candidate set or sentinel option, raw
  retryability lives outside `ProviderOutcomeSnapshot`, the model constructor
  lacks its distinct nonforgeable ports-owned authority, any sibling can mint
  or share that authority, does not compute effective coverage/digest from its
  validated typed parts,
  the raw token/digest is absent or not independently rebuilt at finish, or the
  v4 snapshot accepts anything besides the complete consuming finished
  projection, final public values are recomputed outside finish, or a report can
  be built without consuming `FinalDiscoveryReportProjectionV4`;
- registry construction does not bind the exact normalized request, captured
  source snapshot, limits and complete Task5B execution binding; finish or
  snapshot accepts a detached composite/catalog digest, traversal vector,
  analysis prefix or contract-registry fragment; `analysis_id_v4()` accepts an
  argument; or a valid provider projection can be paired with foreign source,
  catalog or traversal state;
- the generative closure receives only `&mut ProviderInvocationRegistryV2`
  while finish consumes `self`, uses unsafe/default/placeholder extraction, or
  a query/context signature relies on an elided lifetime that fails under
  `deny(elided_lifetimes_in_paths)`;
- any Task7 root/key/map/model/projection writer accepts `&mut Vec<u8>`, the
  `CanonicalIdentitySinkV4` exposes bytes/length/slice/serde/Debug/Display or is
  finalized outside `determinism.rs`, a typed identity token exposes raw bytes
  or acts as validation authority, or an upstream Task4/Task5B `Vec` writer is
  called outside the sink's two private typed adapter methods;
- `MaterialAssociationRootV2` is a visible enum or has any visible field/inner
  variant/raw constructor/key/variant/member projection, any module outside
  `ports.rs` can construct/destructure/switch a root or extract its invocation
  key, the root/inner enum/key implements `Debug` or `Display` and therefore
  leaks a variant or member through formatting, or `association.rs`
  encodes/checks spellings by matching the private variant instead of the
  root's sealed exhaustive methods;
- any of the five frozen typed query owners or the approved Definition owner
  lacks its sealed non-Clone/non-serde association authority, Task 7 locally
  reconstructs/serializes/clones a member
  set, query digest/key equality is used as membership proof, registration does
  not verify exact authority variant/port/digest, invoke drops/substitutes the
  plan authority instead of moving it into `Finished`, or a recorded handle
  cannot borrow that exact value;
- a pre-I/O root bypasses a plan-owned sealed root constructor, a post-I/O
  Material/AtomicGroup bypasses exact finished-outcome membership, or a
  synthetic group is accepted merely because its material was queried;
- an association is validated only against an already-returned outcome, roots
  cannot be staged before I/O, or a finished typed-plan/typed-invoke/raw/
  recorded-ID/Invocation-root set is not a strict bijection;
- a post-response/later-origin/mechanism association reuses the consumed plan,
  accepts a bare digest/key, or lacks a registry-issued recorded handle;
- a caller-visible `record_terminal`, generic invoke, raw-outcome callback or
  lookup by bare `ProviderInvocationKeyV2`/digest/slot exists; a typed invoke
  does not itself perform exactly one matching provider call and atomically
  record its response, accepts a query not byte-equal to its plan authority, or
  allows a same-port query/outcome swap or malformed empty-Complete response;
- any generative constructor/register/invoke/recorded/stage/preview/prepare/
  project-next/finality-record/materiality-seal/finish lifecycle operation used
  from `use_case`, or traversal register/complete/gap used from `traversal`, is a
  bare private `fn`, `pub(crate)` or public instead of exact `pub(super)`, either
  material/group `require_*` helper is visible outside `ports.rs`, or
  any plan/finished-handle root constructor has a caller outside `association`,
  `apply_finality_associations_v4` has a caller outside `association`, an
  instruction can be matched there, or an applied token can be constructed
  without a successful transactional sink commit,
  the finished/snapshot writer or sink finalizer has a caller outside
  `determinism`,
  either opaque root identity/spelling method has a caller outside `association`,
  any key/root-vector/outcome/map/model/projection/snapshot writer violates the
  exact section-4.2 chain, any finality/model witness violates the section-3.1
  distinct-authority whitelist, sibling-private fields are read directly instead of through an
  owner method, a sentinel constructor has any production caller, or an owner
  `association_authority_v1` is called outside its matching
  `register_*` or more than once per successful registration, or
  `admit_application_artifact_spelling_v1` has a caller outside the exact five
  whitelisted discovery siblings;
- registry entries and opaque slot IDs are not separate types, their exact
  key-to-slot bijection can diverge, or duplicate registration can allocate a
  second slot/association or reach provider I/O instead of failing before I/O;
- `ProviderInvocationRegistryV2` does not own the Task 5A/domain spelling
  registry, request/catalog seeding is partial or occurs after downstream I/O,
  either catalog can be skipped/recommitted, a late catalog commit method
  exists, the generative constructor exposes a registry before atomic request+
  catalog staging/binding succeeds, or registration/invoke proceeds first;
- Task 7 walks private catalog/context fields, or Task 5B's sealed catalog
  spelling gate omits/substitutes an occurrence, performs I/O, exposes an
  iterator/path/manifest/witness/material handle, or returns anything except one
  opaque staged delta bound to the supplied registry baseline;
- an application producer appends/inserts/unions/sorts/deduplicates an
  occurrence before `admit_application_artifact_spelling_v1`, omits any Stage
  1-5 query/Support/association/traversal/mechanism/candidate/graph occurrence,
  the sealed raw mechanism gate omits either key artifact, an owner, either
  base-edge endpoint or an entry candidate, a Mechanism association gate omits
  either key artifact or any association-material artifact, either gate relies
  on a previous contribution instead of checking its complete own ingress,
  association code walks a Task5B-private material/group member or accepts a
  caller artifact list, a gate is callable outside the five sibling-module
  whitelist, the registry field is exposed, an association owner-check wrapper
  lacks mutable fail-closed invalidation, a caught/ignored spelling error leaves
  any registry/output operation reachable, or an exact-spelling violation lacks
  its fixed nonretryable Task7 mapping/priority,
  or typed query registration allocates a slot without calling the exact sealed
  Task5B/Task6 owner recheck over every private artifact member;
- a typed invoke performs per-element response/completeness validation,
  canonicalization, collection, sorting, grouping or limiting before building
  its complete raw spelling delta, lets collection invent/reparse an artifact
  or use one without the staged authority, omits the final exhaustive collected
  check, commits delta and `Staged -> Finished` separately, leaves a collision
  finishable, records any collided raw outcome, issues an ID, or permits an
  accepted raw snapshot/map/report/receipt after invalidation; a cache replay or
  cross-invocation response bypasses the same transaction;
- Task 5B or Task 6 sorts, deduplicates, groups or applies a record/file ceiling
  before invoking the shared Task 5A/domain spelling primitive on its complete
  raw pre-classification occurrence stream, or redeclares a local registry;
- `RecordedInvocationIdV2` borrows the registry across mutation, exposes a raw
  port/digest/slot constructor or serde form, or a foreign execution/slot/
  unfinished ID can yield a recorded handle; a recorded handle lacks its private
  checked nonce/slot or survives an association `&mut registry` borrow; its
  evidence/preview visitor can return a borrowed group/record/gap, key,
  authority or accepted DTO, Finished fails to retain the complete full groups,
  or any recorded handle exposes a provider-gap class/candidate/effective
  constructor;
- raw `retryable` is omitted/derived, raw gap bytes use admission tag order, or
  raw gap/location vectors are silently deduplicated/noncanonical;
- an association entry with an empty conclusion-scope vector is committed or
  silently dropped, or that empty association scope itself schedules provider
  I/O; an empty typed query member vector remains governed by section 3.2 rule
  1 and is not an empty association scope;
- association scope changes reorder/split provider or application admission;
- a same-source semantic artifact identity has two byte-different exact
  spellings, collision validation occurs only after sorting/deduplication, or a
  lexical-minimum, first-wins, provider-order or cache-order spelling is kept;
- v6 acceptance row 24 is interpreted to allow both byte-different variants in
  one same-source execution, or isolated case-equivalent variants cease to
  produce byte-equivalent query/group/admission bytes;
- a PerPort/Global occupied bucket is missing/duplicated/split per outcome, or
  Global is portless/cross-port instead of exactly one same-port row per
  affected port;
- the ownerless EvidenceGapLimit sentinel is stored in a per-invocation
  `admission_gaps` vector, duplicated per invocation/port, or loses its exact
  analysis-level admission identity; orchestration can choose `Some`/`None` or
  call its constructor; the full candidate set/map roots are not validated
  before the branch; 256 rows/2,000 subjects selects the sentinel or 257/2,001
  does not; sentinel mode lacks/mismatches its single derived Admission row/
  public check, admits an accompanying row, or no-sentinel mode omits any
  provider/admission candidate;
- admission/effective gaps contain `ConclusionScope` or a fake artifact;
- Request/Proposal context roots create runtime reachability;
- an omitted traversal root causes a provider query;
- traversal owns a final gap vector/set, a bounded decision mutates state before
  registry registration, an ID is unresolved/double/foreign/mismatched,
  prepare accepts Pending or unequal expected/resolved counts, or finish accepts
  any caller traversal argument instead of consuming its sealed ledger;
- full Task7 including concrete Task8 integration is an upstream Task8 gate;
- Task7PrerequisiteSliceV1 imports a concrete Task8 resolver/plan/writer;
- Task8 delivery omits distinct Task7Task8IntegrationV1 evidence;
- public MCP/package/skill files are modified by Task7.

## 14. External four-owner package protocol

This owner file has no mutable design-status field. Its owner-local P0/P1 count
is zero only when its own semantics are closed and exactly compatible with the
current Task 4 v7, Task 5B v7 and Task 6 v7 owner contracts. Missing external
evidence is an external package gate, not an owner-local semantic P1 and not a
reason to edit these bytes with a status or hash.

The sole design-acceptance protocol is time-invariant:

1. a coordinator makes the exact four owner documents immutable and computes
   their exact SHA-256 tuple outside every owner file;
2. the standalone two-path Task 6 generator records its exact bytes, the exact
   Task6-pinned external registry-manifest path/hash/bytes, command and output,
   proves `registries=PASS` only after loading that manifest, and reproduces the
   six positive query-v3 goldens plus the forbidden
   extra-`bytes(AtomicSourceIdentityV2)` negative against that same tuple;
3. each owner self-audit examines the exact tuple, independently executes its
   owner-local mechanical checks and reports no P0/P1;
4. separate independent reviewers examine the same immutable tuple, generator
   evidence and cross-owner DAG/API compatibility and report no P0/P1;
5. one atomic transition in
   `.superpowers/sdd/task-4-7-v7-design-package-acceptance.md` records the four
   owner hashes and the exact self-audit, independent-review and generator
   plus registry-manifest evidence identities for the complete package.

No current self-hash, peer hash, audit/review hash, package status or accepted
implementation OID is embedded in this owner file. Any edit to any owner byte
makes the external tuple and all derived generator output, affected audits,
independent reviews and ledger claim stale; the protocol repeats before a new
ledger transition. An unchanged peer digest remains mathematically correct but
cannot be reused as standalone acceptance evidence for the changed tuple.

Task5C-Evidence and Task 8 identities are intentionally outside this design
protocol. The exact Task5C-Evidence implementation OID is a later Task 7
production prerequisite; `Task7Task8IntegrationV1` is later Task 8 delivery.
Neither may create a reverse design-package edge.

## 15. Result

When accepted and implemented, Task 7 retains deterministic provider testimony
that is reusable across proposals, while application materiality remains exact
per request/proposal/mechanism. Registered-Form authority is bound end-to-end,
BSL query v3 is the sole BSL cache identity, admission cannot be influenced by
proposal ordering, and Task 8 integrates without making itself a prerequisite.
