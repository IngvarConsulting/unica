# Task 4 v7 dynamic registered-material self-audit

Fresh package re-audit observation time: 2026-07-18 12:46:46 +07,
Asia/Ho_Chi_Minh.

This file is time-bound review evidence, not status authority. It has no
draft/candidate/accepted or implementation-status field. The external ledger
`.superpowers/sdd/task-4-7-v7-design-package-acceptance.md` is the sole design
status authority for exact document/audit/review hashes; implementation status
belongs to the later implementation ledger. Neither ledger status changes
these audit bytes.

Audited artifact:

```text
.superpowers/sdd/task-4-v7-dynamic-material-addendum.md
```

The audit checks the Task 4 root fix against the live snapshot/parser code and
the exact frozen Task 5B/6/7 peer bytes listed below. It changes no owner,
production, active spec, historical brief/report, ledger or downstream design
file.

## 1. Inputs and precedence

The audit read:

- `.superpowers/sdd/task-4-brief.md` and `task-4-report.md`;
- `crates/unica-coder/src/domain/source_snapshot.rs`;
- `crates/unica-coder/src/infrastructure/source_snapshot.rs`;
- `crates/unica-coder/src/infrastructure/platform_xml.rs`;
- the current dynamic-material/parser/provider clauses in
  `.superpowers/sdd/task-5b-v7-contract.md`;
- the current Task 6/7 successor addenda; and
- the active snapshot clauses in the architecture and ADR.

The exact immutable owner tuple re-audited in this pass is:

| Owner artifact | Observed SHA-256 |
| --- | --- |
| `task-4-v7-dynamic-material-addendum.md` | `1581d0b737a9e4e856526d67987a292edd39404ec5dda1cb3299c6041409cde2` |
| `task-5b-v7-contract.md` | `30430abeb69aeb83bd665a08b41fa1837675a651b3be736936c6e4e96e14f3ad` |
| `task-6-v2-v7-addendum.md` | `9f488f78ba20f188e1c28e5393eb9d5d16889cde8f8ca5363bb2ea476631fca0` |
| `task-7-v7-addendum.md` | `708022ff0b179092d5f23609449dfa8a7415adaa2e404179b9a24b43d95c1b7d` |

These hashes bind this audit's observations to bytes; they do not assign a
design status. The external package ledger remains the sole status authority.

Code/tests/package metadata remain above this owner design when they contradict it.
The addendum designs a missing successor capability; it does not claim that
live code already implements v2.

## 2. Live-code root-cause proof

Live domain grammar is:

```text
ManifestEntry = Present(MaterialFile) | AbsentOptional(OptionalMaterialTag)
OptionalMaterialTag = exactly five fixed tags
SourceManifest = BTreeMap<String, ManifestEntry>
SOURCE_FINGERPRINT_DOMAIN = "unica.source-set-snapshot.v1"
```

The infrastructure scan records present registered subtree files, but an absent
arbitrary registered Form material path has no manifest row. `read_verified`
requires an ordinary Present row; `read_optional_verified` can authorize only
the five fixed tags. Consequently live code cannot simultaneously guarantee:

1. capture-owned arbitrary Missing Form.xml/FormModule identity;
2. source-fingerprint binding of that absence; and
3. no downstream suffix inference.

The separate fingerprinted captured-Form and expectation catalogs are
therefore necessary. A provider-only suffix table, generic existence probe, or
reuse of `OptionalMaterialTag` leaves the contradiction unresolved.

Verdict: **PASS — root cause, not provider symptom, is addressed**.

## 3. Type, totality, and authority audit

### 3.1 Captured handle envelope

`CapturedRegisteredFormV1` binds exact owner/Form descriptor keys, descriptor
length and leaf, root span, and the neutral properties/FormType capture. Its
constructor/decoder must prove equality with the ordinary Present descriptor
entry and bound every span by the same descriptor length. The source encoder
includes every one of those fields.

This closes the replay gap where identical expectation rows could otherwise be
attached to a different descriptor leaf or different FormType witness.

Verdict: **PASS**.

### 3.2 Exact two-row matrix

The relationship identity is exact source-local owner key + Form key + closed
kind. Every captured handle has exactly `FormXml` and `FormModule`; every row
points to exactly one handle. Known Managed admits independent Missing/Present
states, while Known Ordinary and every Inconclusive problem admit only two
keyless NotApplicable rows.

This avoids the earlier logical error of deriving managed paths before the
captured FormType result. Completeness remains fingerprint-visible because even
NotApplicable has two relationship rows, without hidden paths.

Verdict: **PASS**.

### 3.3 Neutral ownership and descriptor semantics

The same neutral `platform_xml` grammar owns authority, closed problems, spans,
and the selector parser. Task 4 stores its exact result; Task 5B imports and
copies it rather than defining another authority.

The neutral types now expose only the exact read-only methods required by a
consumer: span start/end; authority/problem stable tags; and borrowed authority,
properties span and canonical FormType span slice. They expose no constructor,
serde, mutable slice or normalization/repair hook. The Task4-owned capture view
delegates to those same methods rather than creating a second FormType view.
Lossless retention is nevertheless implementable: spans have Copy/Clone/Eq/
Ord/Hash, authority/problem have Clone/Eq/Ord/Hash, capture has Clone/Eq, and
source/leaf fingerprint smart types have Clone/Eq/Ord/Hash. Task 5B clones the
same neutral/smart types; it does not define conversion copies.

Task 4 does not capture wrapper UUID/membership semantics, so the one public
object-safe `PlatformCatalogPort::build_context(&SourceSnapshotV2,
&dyn SourceSnapshotPort)` visits the Analysis and then canonical Destination
atomic snapshots, calls the new Task4-owned handle-bound descriptor reader and
shared-semantic-parses every captured Form descriptor exactly once per port
invocation. It cannot call raw
`read_verified(snapshot, path)`, independently choose FormType or replace
stored authority. A recomputed-envelope mismatch is an invariant/snapshot
failure. A pure `from_prepared` helper remains private rather than becoming a
second public construction path.

Verdict: **PASS**, conditional on replacing live `platform_xml.rs` local-name-
only/arbitrary-namespace behavior during implementation.

### 3.4 Single Task4-owned handle and capture view

`RegisteredFormCaptureHandleV1<'snapshot>` itself privately binds enclosing
source identity, source fingerprint, manifest and captured Form. Task 5B
imports that type; it must not duplicate those fields or assemble an equivalent
DTO. The former private-struct-returning accessor is removed.

`view()` returns only `CapturedRegisteredFormViewV1<'snapshot>`. Its exact
read-only accessors cover opaque owner/Form descriptor keys, descriptor
length/leaf, root span, and neutral FormType capture/authority/properties/type
spans. Thus Task 5B receives every fact required to build its catalog but never
the private capture struct, manifest spelling, constructor or mutable authority.

The Form handle resolves `.material(kind)` inside the same manifest it already
binds. The material handle retains the same source/fingerprint/manifest/Form
authority. No API accepts an arbitrary captured-Form reference plus a separate
snapshot, and no raw owner/Form/kind tuple exists.

Pointer identity is deliberately not authority: a separately allocated but
semantically equal validated snapshot may resolve a handle. Any changed source,
fingerprint, manifest, Form, kind, state or envelope returns nonretryable
`RegisteredMaterialHandleMismatch` before I/O.

Verdict: **PASS**.

### 3.5 Injected descriptor reader and lossless key projection

All three specialized readers are exact methods on the existing injected,
object-safe `SourceSnapshotPort`, with `&self`, the semantic snapshot and the
opaque Task4 handle. Their only generic parameters are lifetimes, so they remain
callable through `&dyn SourceSnapshotPort`. There is no free reader function,
hidden global/root I/O, or reader capability inside a snapshot, handle,
projection or verified result; all calls/counters are observable on the exact
injected port spy.

```text
trait SourceSnapshotPort {
  fn read_registered_form_descriptor_verified<'snapshot>(
    &self,
    snapshot: &'snapshot SourceSetSnapshotV2,
    handle: &RegisteredFormCaptureHandleV1<'snapshot>,
  ) -> Result<VerifiedRegisteredFormDescriptorBytesV1<'snapshot>,
              SourceReadError>;

  fn read_registered_material_verified<'snapshot>(
    &self,
    snapshot: &'snapshot SourceSetSnapshotV2,
    handle: RegisteredMaterialExpectationHandleV1<'snapshot>,
  ) -> Result<RegisteredMaterialReadV1<'snapshot>, SourceReadError>;

  fn read_captured_bsl_material_verified<'snapshot>(
    &self,
    snapshot: &'snapshot SourceSetSnapshotV2,
    handle: CapturedAnalysisBslMaterialHandleV1<'snapshot>,
  ) -> Result<VerifiedCapturedBslMaterialBytesV1<'snapshot>,
              SourceReadError>;
}
```

The exact descriptor method takes
`&RegisteredFormCaptureHandleV1<'snapshot>`, semantically validates it against
the supplied snapshot before I/O, delegates exactly once through the same port
to the ordinary Present verified reader, and returns
`VerifiedRegisteredFormDescriptorBytesV1<'snapshot>`. The wrapper binds the
same snapshot/source/Form/key/length/leaf authority and exposes only `bytes()`.
Source identity/fingerprint validation is constant in catalog cardinality; one
indexed `captured_registered_forms.get` then checks the exact key/envelope and
ordinary Present authority. It never compares the complete manifest or walks
Forms. The 200,000-Form recording RED therefore observes one captured-Form map
lookup and zero manifest/Form iterator steps per descriptor, avoiding `O(N^2)`
context construction.

An internal key/envelope/catalog/ordinary-entry disagreement is nonretryable
`RegisteredMaterialHandleMismatch` before I/O. Only external descriptor
disappearance, content/identity or ancestor drift after semantic validation is
retryable `SourceFingerprintMismatch`.

Capture/material state key accessors return only opaque
`SnapshotManifestKeyRefV1<'snapshot>`. Its sole transition creates an owned,
lossless `SnapshotManifestKeyProjectionV1`. A Task5B-private key wrapper may
contain only that projection token; normalization, equality/order/hash and
canonical key framing remain Task4-owned. The encoder appends exact key identity
through a sealed neutral append-only encoder and returns no raw string, bytes,
path or components. The dependency is one-way from Task 5B to Task 4, with no
Task4-to-Task5B type edge.

This closes both prior replay routes: duplicating capture-handle fields and
copying a normalized key spelling into a downstream String.

The material handle's `relationship()` follows the same pattern. It returns an
opaque relationship ref whose projection privately retains complete semantic
source/fingerprint/manifest/owner/Form/kind authority. Before nested encoding,
Task 4 validates the enclosing source identity/fingerprint; the nested encoder
then emits only the unchanged source-local owner/Form/kind row because source
authority already occurs in the catalog header. This prevents cross-source
replay without changing or duplicating published bytes. Task 5B stores the
projection only inside its context/dispatcher authority. Task 6 receives no
material accessor, ref or projection: it receives only
`AnalysisBslMaterialScanPlanV1`, its final typed/opaque items and
`AnalysisBslMaterialVerificationV1` through the context dispatcher.

Verdict: **PASS**.

### 3.6 Indexed projection-to-handle resolver

The owner now supplies the exact inverse seam on `SourceSetSnapshotV2`:
`resolve_registered_material_projection<'snapshot>(&'snapshot self,
&RegisteredMaterialRelationshipProjectionV1) ->
Result<RegisteredMaterialExpectationHandleV1<'snapshot>, SourceReadError>`.
The projection is Task4-owned and unforgeable, with private source identity,
source fingerprint, accepted manifest/catalog relationship and state witness;
it exposes no tuple, key, path or raw component.

Resolution validates source/fingerprint/manifest authority first, performs one
indexed `registered_material_expectations.get` by the private relationship key,
and validates the referenced Form, relationship and exact expectation state.
It returns the Task4 handle by value. A foreign or changed snapshot/source/
fingerprint/Form/kind/state fails with nonretryable
`RegisteredMaterialHandleMismatch` before I/O, while a separately allocated but
semantically equal reconstructed snapshot is accepted.

The looked-up `registered_material_expectations` `BTreeMap` is explicitly the
private ordered relationship index in authoritative `SourceManifestV2` state.
It is completely constructed, totality/state/backreference-validated and bound
by the source fingerprint before snapshot acceptance; it is not a later cache
that could drift from canonical authority. `SourceSetSnapshotV2` retains and
queries that accepted index directly.

The 200,000-row RED records one relationship-map lookup and zero map/handle
iterator calls, proving the existing `BTreeMap` `O(log N)` path rather than an
`O(N)` scan. Resolution itself records zero verifier, filesystem, byte-read and
parse calls. Task 5B's private resolver is the sole current consumer; Task 6
receives only the Task5B Analysis-BSL plan/item API and never sees the projection
or resolver.
No canonical encoder input changes, so all published goldens remain unchanged.

Verdict: **PASS**.

### 3.7 Atomic source versus composite snapshot authority

The revised grammar has no overloaded snapshot word: `SourceSetSnapshotV2`
contains exactly one `ResolvedSourceSet`, fingerprint and manifest, while
`SourceSnapshotV2` alone contains Analysis plus a canonical unique Destination
vector and `CompositeSnapshotIdV2`. The composite constructor rejects an
Analysis identity repeated as Destination and duplicate/conflicting complete
Destination identities, then exposes Analysis first and the sorted Destination
slice verbatim.

`diagnostic_workspace_epoch` is explicitly diagnostic-only. It participates in
neither source/composite/catalog/query identity nor semantic equality. Equal
composites under different epochs therefore have equal
`CompositeSnapshotIdV2`; a role, source count, complete source identity or v2
source-fingerprint change does not. Byte readers accept only the atomic type;
the catalog port accepts only the composite type.

The borrowed catalog port is deterministic rather than fictitiously one-shot:
equal repeated builds are allowed and equal. Task7 alone owns the production
exactly-once orchestration/static/spy invariant.

Verdict: **PASS**.

### 3.8 Canonical captured Analysis-BSL partition

Task4 alone recognizes `.bsl` while it still owns the private manifest key. Its
derived checked projection merges every captured ordinary Present BSL material
with every registered FormModule Present/Missing/NotApplicable obligation in
one total order. A claimed Present FormModule replaces, rather than supplements,
the equal ordinary candidate at that candidate's private order slot. Missing is
anchored by its private expected key; keyless NotApplicable is anchored after
its captured Form descriptor with a complete relationship tie-break. A captured
unsupported ordinary BSL item remains visible, while a Form-shaped file outside
accepted capture is absent.

The projection is a checked index over authority already encoded in v2, capped
at `MAX_CAPTURED_ANALYSIS_BSL_SCAN_ITEMS=400,000`; it is not a new fingerprint
field. Mechanical REDs retain all eight existing Task4 fingerprint goldens.
Task6 cannot name the capture handle or location ref and receives no key, path,
suffix, private order component, raw Task4 reader method or hidden reader
capability. Its sole permitted reader argument is the one injected
`&dyn SourceSnapshotPort` passed to
`PlatformCatalogContextV1::read_analysis_bsl_material_verified(...)`. Task5B
alone converts the projection to `AnalysisBslMaterialScanPlanV1`, owns the merged
`MAX_BSL_FILES`/total/per-file admission semantics, and dispatches one consumed
item back through the injected reader.

The only pre-I/O size seam is builder-whitelisted
`admission_byte_length() -> Option<u64>` on the opaque Task4 handle. It is
`Some(exact captured length)` for ordinary and registered Present and `None` for
Missing/NotApplicable; a claimed Present FormModule must equal the ordinary
entry it replaced. It exposes no state, key, path, fingerprint or reader
argument and is statically unavailable to Task6.

The same handle's builder-only `module() -> Option<&ArtifactRef>` is the sole
identity seam. Task4 derives `Some` for supported ordinary and every registered
FormModule while it owns the private key; `None` means unsupported captured
ordinary. Task5B only looks up the equal context-owned typed ref. Thus the
design does not secretly require Task5B/Task6 to inspect a path in order to make
the plan constructible.

The companion builder-only
`CapturedBslLocationRefV1::to_verified_location()` is the sole zero-I/O
transition from a private captured diagnostic anchor to the opaque owned
`VerifiedBslSourceLocationV1`. It exposes no key/path and cannot authorize a
read; Task6 sees only the owned Task5B item location.

Every state and unsupported item has an opaque receipt-grade diagnostic
location. Present wrappers alone expose checked byte-range locations and opaque
typed cache locators. Equal bytes/ranges produce equal 1-based coordinates;
empty, reversed, out-of-range or non-code-point-boundary ranges fail closed.

Verdict: **PASS**.

## 4. Path, containment, and absence audit

The serializer has one capture-only input: an already validated registered Form
descriptor key. It runs only for exact Known Managed, strips one exact terminal
`.xml`, appends one of two closed suffixes, and revalidates the result with the
same contained-key grammar.

Live `contained_relative_file` already rejects paths longer than 4,096 bytes.
The addendum correctly promotes that live boundary rather than describing it as
a new v2 compatibility narrowing.

Missing is proven at the first absent suffix component after every existing
ancestor opens relative to a retained contained handle with no-follow/reparse
checks. This is necessary and implementable:

- absent `Ext` is a normal Missing result;
- absent final `Module.bsl` is the same semantic Missing result;
- symlink, junction, special/non-directory object, permission/share error,
  unstable identity, or case alias proves neither absence nor safety.

The design does not use `exists`, path-wide metadata, canonicalize, or a
check-then-open sequence. Exact Present-key equality is the required ordinary
manifest binding, while differently spelled case/alias collisions are rejected.

Verdict: **PASS**.

## 5. Race and error-boundary audit

Capture proof chain:

```text
initial complete registration/present enumeration
  -> stable hashing + primary descriptor envelope
  -> total expectation matrix + contained Missing proofs
  -> injected mutation point
  -> independent enumeration/parser/matrix
  -> exact initial/final plan equality
  -> final bounded Present rereads + final Missing proofs
  -> immutable SourceManifestV2
```

Every structural, FormType/span, state/key, identity, or content drift discards
the whole snapshot. Capture-time drift maps to retryable
`SourceChangedDuringCapture`; it is not the post-capture verified-reader error
`SourceFingerprintMismatch`.

After capture, the injected port method first validates the complete private
handle/projection/relationship/state/key and ordinary-entry authority. Any
internal disagreement is nonretryable `RegisteredMaterialHandleMismatch` before
I/O (and is impossible after valid construction). It then rechecks Missing with
the same contained primitive and Present with the equal ordinary manifest
authority. Only external Missing appearance, Present disappearance/content/
identity change, or ancestor topology/identity drift after that validation maps
to retryable `SourceFingerprintMismatch`. NotApplicable performs no verifier I/O.
All three results are specialized opaque authorities. In particular, Present
wraps bytes together with semantic snapshot/source/relationship/key/length/leaf
authority and exposes only `bytes()`, preventing cross-Form or cross-kind replay
of a generic verified byte buffer.

Stable malformed registration/XML, unsafe topology, unavailable I/O,
deterministic resource overflow, deadline, and impossible matrices retain
closed Task 4 classifications; none becomes provisional Missing or a provider
gap.

Verdict: **PASS**.

## 6. Bounds and overlap audit

The addendum distinguishes two facts that previous drafts conflated:

1. `MAX_REGISTERED_MATERIAL_EXPECTATIONS=400,000` is the independent global
   allocation/resource cap; the isolated raw-count accumulator accepts 400,000
   and rejects 400,001 with checked arithmetic before side effects, while the
   two-rows-per-Form end-to-end matrix tests 400,000/400,002.
2. `SINGLE_SOURCE_EXPECTATION_FIXTURE_AT_FILE_LIMIT=399,996` is only the
   one-source/disjoint test boundary produced by
   `2 * (200,000 - Configuration - one owner)`; it is not a production maximum.

Live capture deduplicates the file/byte budget by globally unique present path,
but source resolution rejects identical roots only. Nested/overlapping
non-identical roots can reuse physical present paths while retaining distinct
source-local Form handles. Summed expectations can therefore exceed 399,996;
calling that value the absolute reachable maximum would be wrong.

Present material charges the globally unique file/byte budget once even though
an expectation references it. Missing and NotApplicable charge no file/bytes.
All enumeration, absence, parsing, final validation, and accumulator work stays
under traversal/XML/deadline bounds.

Verdict: **PASS**.

## 7. Encoder audit

The source v2 encoder preserves v1 source-identity and ordinary-entry bytes,
then encodes counted/sorted captured handles followed by counted/sorted
expectations. Handle fields include descriptor length/raw leaf, root span,
u16 authority/value tags, properties span, and counted u32 spans. Expectations
use u16 kind/state tags and raw leaf bytes only for Present. Composite identity
changes domain because it embeds source fingerprints; v1/v2 are not dual
accepted.

The new view/ref/projection types add no source-snapshot bytes. They are opaque
consumption authorities over the same already-encoded fields. Manifest-key
projection delegates the existing Task4 key framing. Relationship projection
first validates its private source binding against the enclosing catalog, then
emits only the pre-existing owner-key/Form-key/kind nested row; it does not
repeat source/fingerprint. Therefore neither source/composite goldens nor
downstream nonempty catalog/query goldens require a version bump.

An independent byte encoder reproduced the three canonical single-row values:

```text
NotApplicable  len 61   919a2297228863374bc95db5c2202b207a44800963d2f908832cd7d8974900f8
Missing        len 104  e1b7e076c32b256446871f8057a4b2302399d7cc4c111d0ec131ddaa01987bfe
Present        len 144  a5b89c72bf780da36a0a8f84115e3ef4314843ef578fd48cab35de1032fdb2f3
```

Using the addendum's complete `main` fixture, the same independent encoder
reproduced:

```text
zero Forms       len 283   c43a6977111248ecd07529e915d8df5161b807c85040a4c0e52073d9e504276a
NotApplicable    len 835   b9cbb191b66c5d77a72b3778bf00b112fcb856672cdc37283e32739dcc715372
Missing          len 944   1efbe72c8f737a8457da4549f7de58f964dd4181630f8d82bfcd04c0cb66c353
Present          len 1309  f6dfa986d1db47f20889d61abad6a2e2748ed34da9903a85f47d4dcf803c811c
Missing composite len 226  1b71f1419ff84829592480f1c6c810f1b3df20a6bcc39e025eb4904785d4cd16
```

The Present fixture includes both material files as ordinary entries, proving
double reference but single captured file authority. The required tests rebuild
production bytes and independently assert order/length, so copied literals
alone cannot pass.

The derived Analysis-BSL scan index encodes no new field. Its ordinary/
registered partition, virtual Missing/NotApplicable anchors, diagnostic
locations and cache capabilities are consumption projections only. The audit
therefore requires the exact eight pre-projection Task4 expectation/source/
composite fingerprint goldens to remain byte-identical, not merely to be
replaced by new expected values.

Verdict: **PASS**.

## 8. Downstream I/O audit

The public context boundary is intentionally not a blanket zero-I/O operation.
It accepts the composite `SourceSnapshotV2`, visits Analysis then canonical
Destinations and, for `N` captured Forms across those atomic snapshots, calls
`reader.read_registered_form_descriptor_verified(atomic_snapshot,
&capture_handle)` exactly `N` times per invocation. Each call pre-validates the full handle, delegates once to the
ordinary Present reader and returns an authority-bound wrapper; Task 5B uses
only `bytes()` for exactly `N` shared semantic-view parses. It makes zero dynamic
Form.xml/FormModule verifier/probe calls. This is necessary because wrapper
UUID/membership semantics are not stored in the Task 4 handle. Query
constructors then perform zero I/O.

Repeating the build with semantically equal composite authority and equal
verified bytes is permitted and yields equal context/catalog/set digests. A
diagnostic epoch-only change is irrelevant. A second production build is not a
Task4/port error; Task7's orchestration/static call-site check and recording spy
reject it.

The semantic provider verifies only a deduplicated demanded dynamic
relationship:

| State | verifier calls | byte reads | XML parses |
| --- | ---: | ---: | ---: |
| NotApplicable | 0 | 0 | 0 |
| Missing | 1 | 0 | 0 |
| Present | 1 | 1 | only if its consumer parses bytes |

This is internally consistent: Missing still needs a post-capture freshness
check, but that check is one handle-only absence verifier call, not a byte read
or parse. Present delegates once to the equal ordinary verified entry.
NotApplicable does nothing. Repeated demand deduplicates by the complete private
source-fingerprint/Form/kind identity.

Before that state-specific verification, Task 5B's private resolver makes one
zero-I/O call to `resolve_registered_material_projection` for the deduplicated
authority. The call returns a Task4 handle by value through one logarithmic map
lookup; it does not add a verifier, file-read or parse count. Task 6 never
receives the projection used for this lookup.

The later raw registered-material operation remains private to the Task5B
context implementation. MetadataCatalog/FormInspection call only the
context-owned FormXml verification method; Task6 calls only the Analysis-BSL
dispatcher with one admitted item and the same injected
`&dyn SourceSnapshotPort`. Only that private context code invokes
`reader.read_registered_material_verified(atomic_snapshot, handle)`. All
descriptor/material counters in this section are port-spy observations;
neither providers, semantic snapshots nor handles can bypass the context
boundary through a raw Task4 reader method or embedded reader/root capability.

Task6 BSL reads use a separate Task5B-owned plan dispatcher over the one Task4
canonical partition. Its exact Task4 counter ownership is:

| Scan item/state | registered verifier calls | ordinary verified byte reads |
| --- | ---: | ---: |
| ordinary Present | 0 | 1 |
| registered FormModule Present | 1 | 1 delegated inside registered read |
| registered FormModule Missing | 1 | 0 |
| registered FormModule NotApplicable | 0 | 0 |

A claimed Present FormModule cannot take both branches. Unsupported ordinary
and per-file rejected items perform no read. Missing/NotApplicable retain their
canonical virtual slots; all states expose only opaque diagnostic authority,
and cache authority exists only after a verified Present read.

The exact-once Form-descriptor reads happen at public context construction, not
later provider demand. They serve shared wrapper/membership semantic views and
cannot become a second FormType authority. Capture keys and material
relationships retained by that context are Task4-owned projection tokens; no
raw path/string or reconstructed tuple crosses the boundary.

Verdict: **PASS**.

## 9. Four-document consistency audit

The addendum correctly forbids standalone acceptance and requires one atomic
co-freeze of Task 4 addendum, Task 5B v7, Task 6 addendum, and Task 7 addendum.
A byte change alters only that file's mathematical SHA. The changed bytes do
not inherit the former package tuple/status/evidence and all four documents
must be resealed/reviewed as a new tuple. Any external ledger row for former
immutable bytes remains historically true only for its exact former hashes;
the edit neither rewrites that history nor transfers it to new bytes.

The design has no self-referential acceptance field: external ledger
`.superpowers/sdd/task-4-7-v7-design-package-acceptance.md` records document/
audit/review hashes, and a later implementation report/ledger records the Task
4 production OID without editing frozen design bytes.

At the audit observation time, the inspected clauses are semantically
cross-consumable:

- Task 5B imports the Task4-owned capture/material handles without repeating
  their source/fingerprint/manifest/captured fields; consumes only
  `CapturedRegisteredFormViewV1`; reads every descriptor through the exact
  injected-port handle-bound method; retains key and complete relationship
  projection tokens;
  delegates both key/relationship encoding to Task 4 without raw spelling or
  duplicated source authority, and uses the Task4 indexed projection resolver
  only inside its private semantic resolver;
- Task 6 removes executable suffix/manifest scanning and consumes only the
  Task5B-owned `AnalysisBslMaterialScanPlanV1` plus dispatcher. Definition
  selects its final modules before limits; CodeSearch and conservative
  CallGraph select all, and CallGraph uses one merged canonical cursor. A
  claimed Present FormModule is one item/read branch, unsupported captured
  ordinary BSL stays visible, material outside capture stays absent, and Task6
  never receives a relationship projection, key, path or cache spelling. Its
  restricted `analysis_platform_catalog()` header exposes only typed source
  identity, source fingerprint, both catalog digests and the fixed registered-
  Form contract version; it exposes no Form iterator, lookup, view or entry;
  and
- Task 7 imports one composite Platform catalog context built from the composite
  `SourceSnapshotV2`, enforces the sole production exactly-once port call, and
  retains the same four-document co-freeze/production order without constructing
  material paths. It names all three configuration/registered-Form/Analysis-BSL
  witness sets, imports the exact reader matrix, keeps `FileBytesLimit` per-item
  and nonterminal, and reserves terminal behavior for `FileCount`/`TotalBytes`.
  `Task7PrerequisiteSliceV1` imports no concrete Task8 type; Task8 later owns
  the distinct `Task7Task8IntegrationV1` delivery/evidence rather than becoming
  a Task7 prerequisite.

The fresh frozen-tuple pass additionally compared the exact public seam tokens
across all four owner hashes. They agree that:

- only `SourceSetSnapshotV2` is atomic, while `SourceSnapshotV2` and
  `CompositeSnapshotIdV2` bind Analysis plus canonical unique Destinations;
- the catalog port accepts `&SourceSnapshotV2` plus the injected
  `&dyn SourceSnapshotPort`, may deterministically rebuild equal inputs, and
  Task7 alone owns one production call;
- Task6 receives one `AnalysisBslMaterialScanPlanV1`, one merged admission
  cursor and only final typed/opaque items, while Task4's builder-only
  `module()`, `admission_byte_length()` and
  `CapturedBslLocationRefV1::to_verified_location()` are named identically and
  denied to Task6 in Task4, Task5B, Task6 and Task7; and
- CodeSearch/CallGraph select all, Definition selects final typed modules before
  limits, and CallGraph has one conservative canonical cursor with no second
  counter/read pass. Task5A owns the authoritative Definition work plan, Task6
  owns the typed `DefinitionQuery`, and Task7 schedules/registers/invokes it. A
  scheduled query with `methods=[]` still receives one invocation; an actually
  unscheduled provider alone has no plan. An empty query member vector is not
  an empty association scope: association vectors remain nonempty and cannot
  themselves schedule I/O.

No owner byte in the observed tuple reintroduces a raw path/key/cache spelling,
separate ordinary/registered scan, duplicate claimed Present FormModule,
one-shot port fiction or atomic/composite type alias.

This result applies only to the exact bytes inspected at the audit observation
time. It neither assigns nor denies a package status; the external ledger does
that. Any later byte change requires a fresh semantic diff and package review
even when an unchanged peer's mathematical SHA naturally stays unchanged.

Verdict: **PASS — the exact frozen owner tuple above has no cross-document
P0/P1 mismatch in the audited dynamic-material/composite seam**.

## 10. Scope and static audit

- No production, active spec, historical Task 4 file, or downstream contract
  was edited by this slice.
- No self-hash, status label, review hash or production OID field is embedded;
  external ledgers own those identities/statuses.
- Dynamic material does not broaden the five-tag optional reader.
- All capture/relationship/result constructors are private and non-serde.
- Task 5B receives Task4-owned handles, views, descriptor bytes and key/
  relationship projection tokens without raw strings/paths or duplicated fields.
- Projection-to-handle resolution is Task4-owned, uses one logarithmic
  relationship-map lookup without iteration, and performs zero I/O.
- All three specialized I/O operations are object-safe `SourceSnapshotPort`
  methods;
  snapshots/handles contain no reader capability and every counter follows the
  injected port.
- The only suffix serializer is Task 4 capture-owned.
- The RED matrix covers parser, envelope, encoder, bounds/overlap, collision,
  race, handle replay, exact-once handle-bound descriptor reads, lossless opaque
  projections, unchanged nested encoder bytes, 200,000-row indexed resolution,
  200,000-Form nonquadratic descriptor validation, injected-port object safety,
  internal-vs-external error separation, cross-source/state rejection, zero-I/O
  resolution, atomic/composite type separation, canonical Destination order,
  diagnostic-epoch exclusion, deterministic repeat context builds, the complete
  merged Analysis-BSL partition, claimed-FormModule deduplication, virtual
  Missing/NotApplicable slots, unsupported captured ordinary and outside-
  capture decoy behavior, opaque receipt/cache locations, demand-scoped dynamic-
  material I/O, and static dependencies.
- The addendum does not implement Task 5B/6/7/8, receipts, MCP, or a watcher.

Verdict: **PASS**.

## 11. Severity ledger

### P0

None.

### P1

None. No design-local or currently visible cross-document P1 remains in the
audited dynamic-material seam.

### P2

1. Active spec/ADR still describe the implemented v1 snapshot/fixed optional
   model. They must be synchronized with accepted v2 code in the implementation
   slice, not pre-emptively treated as current behavior.
2. Existing tests/fakes use raw String manifest paths/fingerprints; private
   smart-type migration will require broad fixture changes. That is expected
   implementation work, not evidence against the design.

## 12. Implementation observations and mandatory STOP gates

At the audit observation time, live code still has the v1 manifest/encoder,
local-name parser, no captured Form catalog/view/projections, no first-absent
primitive, and neither handle-only reader. This is time-bound implementation
evidence, not a persistent status field or design P1; the implementation ledger
is authoritative after later delivery.

Mandatory implementation STOP gates after design co-freeze are:

- every RED must fail before and pass after its root fix;
- Unix and Windows first-absent primitives need separate implementation review;
- source/composite v2 migration is atomic, with no v1 fallback;
- type/API tests prove `SourceSetSnapshotV2` is one atomic source,
  `SourceSnapshotV2` is the sole composite, Destinations are canonical unique,
  diagnostic epoch is identity-free and equal repeated catalog builds are
  deterministic; Task7 separately proves one production build;
- context spies show one handle-bound descriptor verifier call, one ordinary
  descriptor byte read and one parse per Form, with zero raw descriptor path
  reads and zero dynamic-material verifier/probe calls; query spies remain zero;
- compile/static tests call all three specialized lifetime-only methods through
  `&dyn SourceSnapshotPort` and reject any free/global reader or reader/root
  capability inside semantic snapshots/handles/results;
- a 200,000-Form recording test proves each descriptor method uses one indexed
  captured-Form lookup, zero full-manifest/Form iteration and one ordinary
  verified read;
- Task 5B compile/static tests prove no duplicated handle fields, private
  capture struct escape, raw key/path/relationship tuple or independent key/
  relationship encoder;
- a 200,000-row recording-map test proves projection resolution performs one
  `BTreeMap` relationship lookup, zero handle/map iteration and zero I/O;
- the checked captured-BSL projection preserves all eight fingerprint goldens,
  merges ordinary/registered candidates in one order, replaces claimed Present
  FormModule exactly once, retains Missing/NotApplicable virtual slots and
  unsupported captured ordinary items, and excludes outside-capture decoys;
- Task6 static/recording fakes can use only the Task5B plan/dispatcher and opaque
  locations/cache capability; they cannot access Task4 scan handles, raw keys/
  paths, suffix tests or direct ordinary/registered readers;
- provider demand counters match the exact 0/1 table;
- active spec/product contracts land with implementation; and
- focused/full/fmt/clippy/product/Windows compile evidence plus independent
  review and exact production OID are recorded before acceptance.

## 13. Audit verdict

The Task 4 addendum is an implementable root-cause design for capture-owned
registered Form.xml/FormModule expectations. Its descriptor-bound neutral
authority, exact two-row matrix, keyless NotApplicable, first-absent no-follow
proof, full v2 encoder, source-bound handle/read-only capture view, handle-bound
descriptor reader, lossless opaque key/relationship projections, logarithmic
zero-I/O projection-to-handle resolution, injected object-safe I/O ownership,
nonquadratic descriptor validation, strict semantic-mismatch/external-drift
error separation, global overlap-safe cap, capture/read race split, and
demand-scoped provider boundary are internally consistent. The explicit atomic-
source/composite-snapshot split, deterministic repeat-build semantics, merged
Analysis-BSL partition and opaque receipt/cache authorities close the final
Task6 scan seam without changing the eight fingerprint goldens.

Design-local result: **P0=0, P1=0, P2=2**.

Audited package-seam result at the observation time: **P0=0, P1=0**. This
sentence reports findings only; it assigns and denies no external status. The
external ledger alone records a package label for exact hashes. Any later byte
change requires this self-audit and the separate four-document review to run
again before a new tuple can receive a ledger row.
