# Task 5B v7 owner self-audit

Fresh package re-audit observation time: 2026-07-18 12:46:46 +07,
Asia/Ho_Chi_Minh.

This audit is a time-bounded finding record, not design-status authority. It
neither grants nor denies candidate/sealed/accepted status. It records the exact
four-owner tuple re-audited below, but no audit/review hash, production OID or
status transition. Current design status is read only from
`.superpowers/sdd/task-4-7-v7-design-package-acceptance.md`; current production
authorization is read only from successor implementation reports/ledger.

This owner audit covers `.superpowers/sdd/task-5b-v7-contract.md`, its exact
Task5C-Evidence dependency wording, and the cross-document seams that must match
the Task4-v7, Task6-v2-v7 and Task7-v7 owner addenda. It does not mutate or
re-accept immutable historical Task4/v6/Task6-v2/Task7-v6 designs. A current
branch, `HEAD`, mtime or this audit cannot substitute for the external atomic
four-document ledger and independent-review evidence.

## 1. Frozen historical inputs

| Input | Immutable SHA-256 |
| --- | --- |
| `task-5b-v6-contract.md` | `665de15a59749bf935dd03b8e15558347db1f93d10dc3cbc2a248b61015c8712` |
| `task-5b-v6-independent-review.md` | `cef8ee1e8e12af88d10805c6824a38a2182c29b3a507becd5058a0a864d8de71` |
| `task-6-v2-design.md` | `5f2d859f77878b43e627930b46a99063972f0fe1a00b3bc692213beea76db4cc` |
| `task-7-v6-design.md` | `b307703e2f825d3218e8acc73d480372114e215593734e78bdf822e0588ddd9e` |

All are read-only historical authority.

### 1.1 Exact frozen v7 owner tuple re-audited

| Owner artifact | Observed SHA-256 |
| --- | --- |
| `task-4-v7-dynamic-material-addendum.md` | `1581d0b737a9e4e856526d67987a292edd39404ec5dda1cb3299c6041409cde2` |
| `task-5b-v7-contract.md` | `30430abeb69aeb83bd665a08b41fa1837675a651b3be736936c6e4e96e14f3ad` |
| `task-6-v2-v7-addendum.md` | `9f488f78ba20f188e1c28e5393eb9d5d16889cde8f8ca5363bb2ea476631fca0` |
| `task-7-v7-addendum.md` | `708022ff0b179092d5f23609449dfa8a7415adaa2e404179b9a24b43d95c1b7d` |

The four hashes bind this audit's observations to immutable owner bytes; they
do not grant status. Audit/review hashes and any package transition remain
external-ledger data.

The package tuple whose byte snapshots were evaluated at audit execution is
exactly:

```text
task-4-v7-dynamic-material-addendum.md
task-5b-v7-contract.md
task-6-v2-v7-addendum.md
task-7-v7-addendum.md
```

This audit grants or denies no member status; the external ledger may transition
only the entire tuple, never one member independently. Owner self-audits/reviews
are additional evidence, not a fifth owner contract. An edit to one ledger-
selected tuple member
invalidates that ledger-selected package tuple/reviews/ledger, not the mathematical
SHA-256 values of unchanged peer bytes; those peer hashes still require tuple
revalidation/review/resealing before reuse as acceptance evidence.
If the tuple is published, its status is recorded only in
`.superpowers/sdd/task-4-7-v7-design-package-acceptance.md`; owner contracts do
not self-embed their own or peer hashes, and that design ledger contains no
later production OIDs. This audit's observed tuple table is review evidence,
not an owner-contract field.

## 2. Dependency and write-scope audit

- Design freeze is separate from production acceptance. A frozen v7 design may
  preexist the later accepted Task5A implementation OID; no Task5B production
  RED may start until `TASK5A_ACCEPTED_SHA` is a reviewed exact commit and the
  subsequent Task4-v7 capture successor is reviewed, accepted, committed and
  recorded as exact `TASK4_V7_ACCEPTED_GIT_OID` in a successor implementation
  report/ledger, never by editing a frozen owner design.
- The production partial order is
  `Task5A -> Task4-v7 -> Task5B -> {Task6, Task5C-Evidence} -> Task7`.
  Task6 imports no Task5C type/OID. Task7 alone later imports exact
  `TASK5C_EVIDENCE_ACCEPTED_GIT_OID`.
- Task7 production here means the independently implementable
  `Task7PrerequisiteSliceV1`: it imports no concrete Task8 type/module. Task8 is
  downstream and later owns the distinct `Task7Task8IntegrationV1` wire and
  integration-evidence hash. Treating concrete Task8 delivery as an upstream
  Task7 prerequisite would create the dependency cycle the owner forbids.
- If the ledger selects the Task7 addendum, it is transitive lineage of the same
  selected Task5B design package and therefore precedes Task5C-Evidence. Evidence does
  not import/revalidate it and adds no direct Task7-owned gate. No Task7
  implementation/OID/integration gates Evidence.
- Task5B compile-tests its typed context seam plus the neutral future-consumer
  parser API, but imports no Task6/Task8 implementation and cannot wait for an
  actual consumer.
- The immutable v6/Task6-v2/Task7-v6 files were not edited by this successor.
- The four owner contracts must agree exactly on neutral FormType names,
  dynamic relationship tags/encoders/bounds, the Task5B capture resolver and
  the production DAG before one acceptance ledger records all four hashes.
  Provider/consumer drift after an individual freeze is a STOP.

Verdict: dependency graph is acyclic at design and production layers. This
fresh pass repeated the check against the exact four frozen owner hashes in
section 1.1 and found no owner-DAG mismatch.

## 3. v6 independent-review closure

| Finding class | v7 closure |
| --- | --- |
| Query identity omitted registered-Form authority | MetadataComposite encodes `registered_form_catalog_set_digest` immediately after the configuration-set digest; FormSourceSet encodes `analysis_registered_form_catalog_digest` immediately after its configuration-catalog digest. Both smart constructors borrow the same composite context; mutation REDs and exact 544/280-byte goldens are normative. |
| Catalog build could drift across consumers | Exact object-safe `PlatformCatalogPort::build_context(&SourceSnapshotV2, &dyn SourceSnapshotPort)` visits Analysis then canonical unique Destinations and returns one non-Clone/non-deserializable composite `PlatformCatalogContextV1` with both exact catalog sets plus configuration, registered-Form and Analysis-BSL witness sets. Equal repeated builds are allowed and deterministic; Task7 alone enforces exactly one production call. Config-only, sidecar-only, mixed-source and source-permuted detached authority cannot enter a consumer. |
| Registered Form authority silently assumed Managed | Neutral `RegisteredFormCatalogV1` retains every registered Form. Neutral-capture-owned `PlatformRegisteredFormTypeAuthorityV1` distinguishes Known Managed/Ordinary from deterministic Inconclusive. Task 4 and Task 5B import that same type/parser/span bundle without a local duplicate or module cycle. Only Managed may carry Form.xml/FormModule Missing/Present; Ordinary/Inconclusive carries NotApplicable and is never misreported as unregistered. |
| Form paths were inferred by suffix | Task4-v7 alone defines the two dynamic Form expectations and sole captured `.bsl` recognition while it owns private keys. Task5B retains FormXml relationship authority privately and builds one canonical merged `AnalysisBslMaterialScanPlanV1` for Task6. Task6/Task8 never import, name, borrow or observe relationship/capture enum, tags, state, key or projection; Task6 receives only plan items/dispatcher and Task8 gives an any-source Form view to the composite-context method. Formatting `Ext/Form.xml` or `Ext/Form/Module.bsl` outside Task4 is forbidden. |
| Dynamic expectation rows could replay a different captured Form | SourceManifestV2 fingerprints a bijective source-qualified handle catalog before its two-row relationship matrix. Each handle binds owner/Form descriptor keys, descriptor length/leaf, root/properties/FormType spans and exact neutral FormType authority; it must equal the ordinary Present descriptor entry. Orphan/duplicate/mismatched handles/rows or invalid spans reject before hashing. |
| Opaque handle API required allocation identity | `SourceSetSnapshotV2::registered_forms()` returns `RegisteredFormCaptureHandleV1<'snapshot>` bound to its enclosing manifest; `material(kind)` returns an opaque source-bound expectation handle and cannot consult another manifest. The by-value Task4 reader accepts only that handle and returns specialized verified NotApplicable/Missing/Present authority; Present bytes remain wrapped with exact relationship/key/length/leaf authority and expose only `bytes()`. A semantic mismatch is nonretryable `SourceReadError::RegisteredMaterialHandleMismatch` / `registered_material_handle_mismatch` before I/O. Equal reconstructed validated snapshot bytes remain equal authority; pointer/address/generation identity is forbidden. |
| Captured FormType versus Task5B semantic parsing conflicted | For each source-qualified capture handle across the composite's atomic sources, exact `PlatformCatalogPort::build_context(&SourceSnapshotV2, &dyn SourceSnapshotPort)` calls once `source_reader.read_registered_form_descriptor_verified(atomic_snapshot, &handle)`, receives `VerifiedRegisteredFormDescriptorBytesV1`, and performs one shared guard/semantic pass over `bytes()`, then invokes private pure `from_prepared`. Guard output must byte-equal `handle.view()`; Task5B losslessly clones the exact neutral Clone/Eq FormType authority/spans and only derives wrapper UUID/membership semantic views. There is no generic path-based descriptor read, external preparation phase, independent FormType parser or witness-recovery parse. At 200,000 Forms, spies prove N indexed handle checks, N reads/parses and zero catalog/manifest rescans: `O(N log N)`, not `O(N^2)`. |
| Registered readers bypassed the injected I/O authority | Descriptor, registered-material and captured-ordinary-BSL readers are lifetime-generic object-safe methods on `SourceSnapshotPort`. Catalog build and both consumer fakes use the one injected `&dyn SourceSnapshotPort`; Task8 may pass it only to the composite-context Form method and Task6 only to the Analysis-BSL dispatcher. Neither consumer can name or call a raw Task4 reader method. No free function, global/root fallback, second/hidden reader or reader capability inside a snapshot/handle/projection/result exists. |
| Captured Missing could become stale before provider execution | The public port performs zero dynamic Form.xml/FormModule verifier/probe calls and query construction performs zero I/O. MetadataCatalog/FormInspection call only the context-owned FormXml verification method; Task6 calls only the Analysis-BSL dispatcher. Ordinary/Inconclusive NotApplicable produces its typed gap with zero verifier calls, zero byte reads and zero XML parses. Only private Task5B context code resolves the expectation and invokes the raw registered reader: Managed+Missing performs one deduplicated verifier call with zero byte reads/parses; Managed+Present performs one such call and one byte read. Semantic handle/projection/state/key/entry disagreement is nonretryable `registered_material_handle_mismatch` before I/O; only later external filesystem appearance/disappearance/identity/content drift is retryable `source_fingerprint_mismatch` and discards the staged batch. The same bridge prevents stale Task6 `DefinitionAbsent`. |
| Private Task4 relationship could not be retained losslessly | `RegisteredMaterialExpectationHandleV1::relationship()` returns only `RegisteredMaterialRelationshipRefV1`; Task5B immediately stores Task4's `RegisteredMaterialRelationshipProjectionV1`, never the private key or a local tuple. The Task4 sealed `encode_source_local_identity_v1` validates enclosing source/fingerprint and emits only owner-key/Form-key/kind comparison bytes. It is resolver-only and cannot duplicate the catalog source header or change published registered-Form goldens. |
| Projection-only reverse resolution could rescan every Form | The required Task4 seam `SourceSetSnapshotV2::resolve_registered_material_projection` validates the projection's complete private semantic binding and uses the private ordered relationship index in `O(log N)`, returning the exact by-value handle or pre-I/O mismatch. Task5B performs exactly two indexed lookups per Form resolution and zero `registered_forms()` iterator steps; the up-to-200,000-Form fixture forbids `O(N^2)` behavior. |
| Task6 had no constructible context-only route to FormModule | `analysis_bsl_material_scan_plan(&analysis_snapshot)` returns one context/snapshot-bound plan over Task4's merged canonical partition. CodeSearch/CallGraph use `select_all()`; Definition uses `select_modules()` before limits. One consuming admission cursor owns all file/byte bounds, and the context dispatcher alone resolves each item to the captured ordinary or registered reader. The restricted `analysis_platform_catalog()` header exposes only typed source identity, source fingerprint, both catalog digests and the exact registered-Form contract version; it has no Form iterator/lookup/view/entry. Compile fixtures start from context+atomic snapshot+injected reader+typed scope and never possess catalogs, private entries, Task4 handles, raw keys/paths or detached digests. |
| Task8 future-consumer seam was analysis-only and unconstructible | `platform_catalog(&AtomicSourceIdentityV2)` returns the same context-bound view for exact Analysis or Destination and exposes typed configuration flavor, ScriptVariant, NamePrefix, root UUID, object wrapper UUID/membership and registered-Form wrapper UUID/membership. The pair fake verifies all those authorities, Known Managed, Analysis Plain/Destination Borrowed and both complete binding lookups; it reads each Form only through `read_registered_platform_form_verified`. The Present wrapper is constructed by one whitelisted neutral factory and parsed by `parse_platform_form_v2`; cross-role/context/object/Form/flavor/material swaps fail before I/O. |
| Relationship count mixed a memory cap with topology reachability | The global checked cap remains 400,000 across all source-qualified manifests; raw 400,001 rejects. 399,996 is only the single-source fixture derived from one Configuration, one owner and 199,998 Form descriptors. Overlapping roots can reuse physical descriptors, so a synthetic overlapping-source matrix proves global 400,000 pass/next valid pair reject without assuming path disjointness. |
| Full Form scan conflicted with request bounds | Every registered Form is classified; every exact Managed+Present Form is scanned in canonical order. The 32-form/1,024-command bounds apply only to request enrichment. Snapshot file/byte/node bounds constrain discovery; 2,000 is only post-scan exact-gap normalization and 2,001 becomes the one QueryWide sentinel. |
| Analysis Form companion depended on another provider | MetadataCatalog independently selects both typed source views, invokes the composite-context verification method for each pair side and parses only Present wrappers. It consumes no FormInspection outcome, raw bytes or Task4 handle, so each provider-local atomic limit/gap remains self-contained. |
| Complete binding failure erased proven CFE ownership | One neutral pass exposes independent `PlatformFormDocumentFlavorAuthorityV2` and method-binding projections. Only Known Plain/Borrowed is needed for CFE ownership. Unknown Event/Action/main-attribute defects may fail bindings while preserving byte-identical flavor/companion authority. Duplicate/misplaced/foreign BaseForm makes only flavor Inconclusive and gaps the companion. |
| Form snapshot binding could be replayed | The context method atomically constructs one `VerifiedRegisteredPlatformFormV1` containing exact context/source/catalog/Form/leaf authority and owned verified bytes; no separate handle/slice can be swapped. It and `RegisteredPlatformFormAuthorityBindingV1` bind the same source, fingerprints, catalog digests and Form identity. Lookup compares authority before validating/querying a Method. |
| Location evidence required reparsing | Atomic configuration/Form witness sets carry exact keys, leaf fingerprints, root/container/field spans and owner-registration linkage outside semantic digests. Their resolvers require current context/catalog equality; coverage is bijective. Required container spans give canonical locations for absence-derived Base/Own/Missing facts. |
| Source/capture/leaf fingerprints could be swapped | `SourceFingerprintV1`, `SnapshotLeafFingerprintV1` and `PlatformConfigurationCaptureCatalogDigestV1` are distinct private smart types with no cross conversion/equality/serde/raw constructor even when bytes match. |
| Form gap scope lost request material | `EffectiveFormMaterialScopeIdentityBytesV1` is derived pre-I/O from one sidecar entry plus optional request enrichment. Every frozen runtime subject enters dropped/missing/unsupported projections even with no command Action or emitted binding. |
| Atomic and composite snapshots were conflated | `SourceSetSnapshotV2` is exactly one atomic source; `SourceSnapshotV2` is the sole Analysis+canonical-Destinations composite and owns `CompositeSnapshotIdV2`. Diagnostic epoch is identity-free. Readers accept the atomic type; the catalog port accepts the composite type. Equal repeat builds are deterministic, while Task7 owns production exactly-once. |
| Ordinary and registered BSL scans could duplicate or split limits | Task4's checked projection replaces a claimed Present FormModule ordinary candidate exactly once, retains Missing/NotApplicable virtual slots and unsupported captured ordinary items, and excludes outside-capture decoys without changing any of eight fingerprint goldens. Builder-only typed `module()` returns Some for supported/registered and None only for unsupported ordinary; `admission_byte_length()` supplies exact equal Present lengths and None for Missing/NotApplicable without state/key/path escape. Task5B's plan applies one canonical merged budget with plan-owned `MAX_BSL_*` constants. Definition scopes before limits; conservative CallGraph selects all, uses one cursor and emits caller-scoped terminal gaps without false edges. |
| BSL receipts/cache required raw paths | Builder-only `CapturedBslLocationRefV1::to_verified_location()` is the sole zero-I/O transition from captured anchor to opaque owned `VerifiedBslSourceLocationV1`. Every plan state and unsupported item exposes only that authority; verified Present alone exposes checked range locations and `VerifiedBslCacheLocatorV1`. Task6 gets no key/path/String/raw Task4 reader method or hidden reader capability. Its sole permitted reader argument is the injected `&dyn SourceSnapshotPort` passed to `PlatformCatalogContextV1::read_analysis_bsl_material_verified(...)`; semantic replay maps exactly to nonretryable `registered_material_handle_mismatch` before I/O. |
| Crate-visible Present factory was described as module-private | Private result fields/tokens and external construction are compile-fail. The neutral Present factory is honestly `pub(crate)` and is sealed by an exact static/product whitelist permitting only the Task5B context method; a second same-crate call/reference/alias/re-export/function pointer fails that check, not Rust compilation. |

## 4. Exact CFE/Form outcome audit

- ProviderFact tag 10 is BaseConfiguration+Own only; tag 11 is Extension
  Own=1/Adopted=2 only. Destination absence is only MetadataAbsent tag 2 and
  cannot carry a membership companion.
- A Managed Form companion uses the nested Form wrapper UUID/membership, never
  the top-level owner UUID.
- Known Ordinary, FormType Inconclusive, Form.xml Missing, flavor Inconclusive
  and each of the three flavor/membership mismatch rows preserve independent
  MetadataPresent, emit zero companion and create the exact full effective-scope
  Bounded gap.
- Binding-only `TypedFormCatalogFailureV2` does not withdraw a Known matching
  flavor companion. Its gap is limited to FormInspection/Task8 binding-dependent
  conclusions.
- Every descriptor/witness path is source-qualified and snapshot-bound; generic
  UUID equality, display paths and source labels cannot promote ownership.

Verdict: companion polarity, materiality and witness authority are internally
consistent.

## 5. Atomic grouping and limiter audit

- The registry has nine closed semantic groups. `StandaloneFact` begins with
  the exact ProviderFact stable tag; there is no parallel fact-family registry.
- Form tag 4 contains only emitted FormCommand polarity/contains/CommandAction
  evidence. Complete FormEvent/ElementEvent bindings remain auxiliary lookup
  authority and consume no evidence admission.
- Classification precedes every provider-local record ceiling. Retention is an
  all-or-none canonical prefix of complete groups; the first non-fitting group
  and every later group form the dropped tail.
- Gap material contains only real source-qualified artifacts. Request/proposal/
  mechanism associations remain in Task7's separate application map.

Verdict: no partial semantic cluster can survive a provider limit.

## 6. Independent mechanical golden verification

The published framing was independently rebuilt with big-endian fixed widths,
length-delimited bytes, Rust char-wise Unicode lowercase and the exact `H`
domain framing. Existing pre-sidecar query bytes were reproduced before adding
the one new 32-byte digest field. The resulting fixed checks are:

```text
QUERY_OK metadata len=544
  SHA=65e6fed0a472f469256546211a9742c302285bacc95173c7f0078a012e770678
  H=264efe11644aaea7528f33b05269b1912b5fe358258303ec848d030ceec0c361
QUERY_OK form len=280
  SHA=2233ce6e902f793f8c20402261b1f4b9eda5f0b80b21aa73d725c87b73ad931e
  H=77855673179fe925318f5873863daa2ba34020c849a57ff9f44b46c13c39abcf
FORM_CATALOG_OK empty len=218 digest=cc7b8add787c08ad7678218574e5a9a55395c7959440208f9a635ed5ab222cd2
FORM_CATALOG_OK managed_present len=489 digest=56704cb084d99f5ffb4b3c037b1f0d1c2c9e40a13a1c03c68a5023ca8cc7a30f
FORM_CATALOG_OK form_xml_missing len=457 digest=bd701d64ac9614f99cf4eed66777bc270c9877742c752fbaa391e07ea7e909d6
FORM_CATALOG_OK form_module_missing len=457 digest=8f45120f8f3a95e033abc0b737bd53076083fbacf2fec9f793e26c9c5f3b14dd
FORM_CATALOG_OK ordinary len=340 digest=843a63f4e58f19439e6fa30dd4028050b90026455b423b5026df10e04e090924
FORM_CATALOG_OK destination_adopted len=564 digest=0cde8a5d1b8e0d340a1cd60ad7357924a37b95dcc547cce038ae661a07782504
FORM_CATALOG_SET_OK len=143 digest=1a9a9cc8c204bce7e293bd3e7dd8a333caf13d83d774665dc746e15b3523a2fc
FORM_AUTHORITY_OK len=306
  SHA=a3c227cf50383c310abc68c3dc959c6a5783e2890f3c39843cb41ecc0264b3ca
  H=1ca64ff3e092c6571adf6929f38223b76c0cfc8311b2e0ed6bbdf446d79b1bf1

TASK4_EXPECTATION_ROW_OK not_applicable len=61
  SHA=919a2297228863374bc95db5c2202b207a44800963d2f908832cd7d8974900f8
TASK4_EXPECTATION_ROW_OK missing len=104
  SHA=e1b7e076c32b256446871f8057a4b2302399d7cc4c111d0ec131ddaa01987bfe
TASK4_EXPECTATION_ROW_OK present len=144
  SHA=a5b89c72bf780da36a0a8f84115e3ef4314843ef578fd48cab35de1032fdb2f3
```

All five FormType Inconclusive digests, both material states and the two-source
set were also rebuilt exactly. Input permutation preserves bytes; key/state/
fingerprint/FormType/membership/source/catalog mutation changes the designated
digest.

The Task4/Task5B byte snapshots read during audit execution matched on every
exact dynamic-seam symbol checked: encoder/domain/path-policy IDs,
4,096/400,000 bounds, test-only 399,996 fixture name, immutable neutral
FormType authority/problem/capture types and their exact Clone/Eq retention
traits, the atomic `SourceSetSnapshotV2`, composite `SourceSnapshotV2`,
`CompositeSnapshotIdV2` and diagnostic-epoch exclusion, descriptor
length/leaf/root span accessors, opaque capture/expectation handles,
`SourceSnapshotPort::read_registered_form_descriptor_verified`, registered and
captured-ordinary material object-safe reader methods and specialized result/
Present wrappers, manifest-key ref/projection, relationship ref/projection,
`encode_source_local_identity_v1`, the indexed
`resolve_registered_material_projection` reverse bridge, and handle-mismatch
error/reason. It also matched `AnalysisBslMaterialScanPlanV1`, its one merged
selection/admission cursor, plan-owned `MAX_BSL_*` constants,
the builder-only typed `module()`, `admission_byte_length()` and
`CapturedBslLocationRefV1::to_verified_location()` seams,
`VerifiedBslSourceLocationV1`, `VerifiedBslCacheLocatorV1`, all five exact
reader counter rows and the CodeSearch/Definition/CallGraph selection split. The
relationship projection remains outside the published catalog payload, so every
nonempty registered-Form golden above and all eight existing Task4 fingerprint
goldens stayed byte-identical.

Verdict: PASS for the query, registered-Form catalog/set,
registered-Form authority, Task4 expectation-row and eight fingerprint goldens
plus the hash-bound cross-document seam token check. The fresh frozen-tuple
re-audit has now run against all four final owner bytes listed in section 1.1.

## 7. Static audit

- Markdown fences are balanced for v7/audit/Task5C design/audit and
  `git diff --check` reports no whitespace errors in the shared worktree.
- Stale registered-managed names, old query lengths 512/248, 4,096 Form scan,
  config-only catalog port, inferred Form suffixes, anonymous material state,
  untyped form flavor, destination-only Form material and the phrase “complete
  parsed Form catalog” are rejected by the final stale-pattern sweep.
- The same sweep rejects raw expectation reader input, generic Present
  `VerifiedBytesV1`, any foreign-capture auxiliary resolver, config-only
  manifest resolvers,
  zero-call Missing negatives, separate public descriptor preparation, pointer
  identity, production `MAX_` naming for the 399,996 fixture, and self-embedded
  OID/hash replacement slots.
- Query structs, formulas, goldens and mutation REDs all name the same neutral
  registered-Form digest fields.
- Type/signature scans distinguish atomic `SourceSetSnapshotV2` from composite
  `SourceSnapshotV2`, require exact object-safe
  `build_context(&SourceSnapshotV2, &dyn SourceSnapshotPort)`, canonical
  Analysis-first source traversal and identity-free diagnostic epoch. Recording
  REDs allow equal deterministic repeat builds; Task7 alone rejects a second
  production orchestration call.
- Configuration/registered-Form catalog and set numeric contract versions are
  four explicit private u16 constants equal to 1. Task6 imports exact
  `REGISTERED_FORM_CATALOG_CONTRACT_VERSION`; no `/v1` string inference or
  caller-selected number is permitted, and 1-to-2 mutation rejects.
- No witness appears in a source-free semantic digest; every witness resolver is
  bound to exact context/source/fingerprint/catalog/key/leaf authority.
- Task4-v7 publishes live code's existing 4,096-byte
  `contained_relative_file` invariant through `SnapshotManifestKeyV1`; it does
  not narrow the accepted key set. Exact 4,096/4,097 REDs, v2 relationship
  domains and the required immutable recorded Task4 OID prevent drift/mixing.
- API-token/compile checks require Task4's exact opaque capture view, descriptor
  reader, manifest-key and relationship projections. They reject Task5B private-
  field redeclarations of either handle, a raw
  `RegisteredMaterialRelationshipKeyV1` field, local tuple/key encoders, generic
  path-based descriptor reads, or source/fingerprint duplication inside a
  source-local relationship transcript.
- The same sweep rejects free `read_registered_*` functions, a hidden/global/
  root I/O capability in any handle/projection/result, or a consumer fixture
  without injected `&dyn SourceSnapshotPort`. Error tests keep every semantic
  handle/projection/relationship/state/key/entry mismatch nonretryable and
  pre-I/O; only externally observed filesystem drift is fingerprint mismatch.
- The projection reverse bridge is exactly
  `SourceSetSnapshotV2::resolve_registered_material_projection`; static and
  recording tests reject a Task5B linear `registered_forms()` resolver and prove
  two `O(log N)` lookups/zero iterator steps for one Form at the 200,000 bound.
- A fake Task6 consumer compiles from only context+Analysis atomic snapshot+
  injected source reader+typed query scope through
  `AnalysisBslMaterialScanPlanV1`, its plan-owned selection/admission cursor and
  context dispatcher. CodeSearch/CallGraph use `select_all()`; Definition uses
  final canonical `select_modules()` before limits. The CallGraph recording fake
  uses one merged cursor, reads callers then matching stored targets and accepts
  conservative caller-scoped terminal gaps. Compile/static checks reject raw
  catalogs, direct plan/item construction, separate kind counters/orders,
  detached digests, Task4 handles, direct Task4 reader methods, raw keys/paths/
  cache locators and serde/lifetime erasure; runtime spies reject replay before
  I/O. The one injected reader remains an explicit allowed argument to the
  context dispatcher.
- Consumer-boundary scans reject any Task6/Task8 import, type annotation,
  pattern match, tag/state/key/fingerprint/projection accessor or downcast of
  `RegisteredFormMaterialAuthorityV1`. The enum is Task5B-private only. The
  Task6 fake alone compiles through scan plan -> one merged admission cursor ->
  context dispatcher; the Task8 fake compiles only through typed any-source
  catalog/object/Form views -> composite-context verification result.
- Task8-specific compile/static checks require typed any-source catalog lookup,
  configuration flavor/ScriptVariant/NamePrefix/root UUID, object wrapper UUID/
  membership, nested-Form wrapper UUID/membership, the composite-context
  verification method, Analysis Plain/Destination Borrowed authority, both
  complete binding lookups and wrapper-only neutral parser. They
  reject the obsolete unverified Form handle, a raw `&[u8]` parser argument,
  analysis-only destination access, direct Task4/material-ref use and any second
  read/parse/path. Analysis and destination fixtures share the same API and
  cross-role mismatch is pre-I/O.
- `RegisteredPlatformFormVerificationV1` plus its opaque NotApplicable/Missing
  token types live in the Task5B context module and have constructors private to
  the composite-context method; compile tests exercise all three owned branches
  and reject private construction or field access elsewhere. Only the Present wrapper,
  parser and `pub(crate) assemble_verified_registered_platform_form_v1` factory
  live neutral. Because it is honestly crate-visible, same-crate calls are not
  claimed to be compile failures. Its exact static/product production call-site
  whitelist permits only the Task5B context method and rejects every second
  call/reference/alias/re-export/function pointer. Static dependency checks enforce
  `Task5B application -> {Task4, neutral}` and reject neutral imports of Task5B,
  Task4, Task6 or Task8; no fictional sibling privacy or module cycle is used.
- The verification enum/tokens, Present wrapper and parser are capability-only:
  static encoder coverage rejects them from registered-Form catalog, query,
  semantic-group and evidence bytes. Contract versions and every published
  manifest/catalog/query/group golden remain unchanged.
- Stale-name checks reject the superseded per-source Metadata plan type; the one
  composite Metadata provider input is exactly `MetadataCompositeQueryV2`.
- Analysis-BSL REDs prove one Task4-derived merged order, claimed Present
  FormModule exactly once, Missing/NotApplicable canonical virtual slots,
  unsupported captured ordinary visibility, outside-capture decoy absence,
  exact mixed-kind N/N+1 bounds, opaque all-state diagnostic locations and
  Present-only range/cache authority. All eight Task4 fingerprint goldens remain
  unchanged. Builder-only typed `module()`, pre-I/O
  `admission_byte_length()` and opaque-location conversion are whitelisted to
  the Task5B context/plan builder; Task6 cannot name/call them or observe state/
  key/path/leaf.

### 7.1 Fresh exact peer-tuple cross-audit

The SHA-256 check at the re-audit time matched all four section-1.1 owner
hashes byte-for-byte. The cross-owner API/semantic scan then confirmed:

- Task4, Task5B and Task7 use `SourceSetSnapshotV2` only for one atomic source,
  `SourceSnapshotV2` only for the Analysis-plus-canonical-Destinations composite,
  and `CompositeSnapshotIdV2` for its epoch-free identity. Task6 receives only
  the exact Analysis atom;
- Task5B and Task7 repeat the same object-safe
  `build_context(&SourceSnapshotV2, &dyn SourceSnapshotPort)` boundary. Equal
  direct builds are deterministic; Task7 alone enforces one production call;
- the returned context contains all three exact configuration, registered-Form
  and Analysis-BSL witness sets. No peer describes an Analysis-only, config-only
  or two-witness context;
- Task6's only catalog projection is the restricted Analysis header: typed
  source identity, source fingerprint, both catalog digests and exact contract
  version, with no Form iterator/lookup/view/entry. Its only material surface is
  plan -> typed/opaque item -> one admission cursor -> context dispatcher; the
  one injected reader is allowed only as the dispatcher argument, never as a
  raw Task4 method or hidden capability;
- Task4's builder-only typed `module()`, `admission_byte_length()` and
  `CapturedBslLocationRefV1::to_verified_location()` names and denial boundary
  are identical in Task5B, Task6 and Task7. Task6 receives only the unified
  `AnalysisBslMaterialScanPlanV1`, final typed/opaque items and one dispatcher;
- CodeSearch/CallGraph use `select_all()`, Definition selects its final typed
  module scope before limits, and CallGraph uses one conservative merged cursor.
  The exact reader matrix agrees across Task5B/Task6/Task7;
  `FileBytesLimit` is per-item/nonterminal, while only `FileCount`/`TotalBytes`
  are terminal and omit the suffix;
- Task5A owns the authoritative Definition work plan, Task6 owns the typed
  `DefinitionQuery`, and Task7 schedules/registers/invokes it. A scheduled query
  with `methods=[]` still has one typed invocation plan/raw terminal/Invocation
  root; only an unscheduled provider has no plan. The empty query-member vector
  is distinct from a forbidden empty association scope: actual association
  vectors remain nonempty and never schedule provider I/O by themselves;
- pre-I/O semantic mismatch remains nonretryable
  `registered_material_handle_mismatch`; exactly post-validation external
  filesystem drift, and no other condition, maps to retryable
  `source_fingerprint_mismatch`; and
- Task8's future fake has typed Analysis+Destination configuration flavor,
  ScriptVariant, NamePrefix, root UUID, object/Form wrapper UUID/membership,
  document flavor and complete-binding checks. The honestly `pub(crate)` Present
  factory remains sealed by a static/product one-caller whitelist, not a false
  same-crate compile-fail assertion. `Task7PrerequisiteSliceV1` imports no
  concrete Task8 type; Task8 later owns the distinct
  `Task7Task8IntegrationV1` delivery/evidence.

No new raw path/key/cache access, suffix inference, duplicate FormModule scan,
split counter, missing witness set, empty-invocation elision or owner-DAG cycle
was found in the exact tuple.

Verdict at audit execution: static checks pass. Independent-review evidence and
any hash publication are evaluated only by the external package ledger.

## 8. Remaining STOP gates

1. Resolve every finding from final independent reviews of the exact four owner
   documents and their owner self-audits.
2. Atomically co-freeze Task4-v7 addendum, Task5B-v7, Task6-v2-v7 addendum and
   Task7-v7 addendum only after exact names/tags/encoders/bounds/DAG imports
   match, including atomic/composite snapshot types, exact catalog-port
   signature, deterministic repeat-build versus Task7 exactly-once ownership,
   the Analysis-BSL plan/selection/cursor/location API, conservative CallGraph
   selection, typed Task8 pair authorities and the static Present-factory
   whitelist; record all design/review SHA-256 values in
   `task-4-7-v7-design-package-acceptance.md` without editing any frozen file
   afterward.
3. Implement/review/accept Task5A and record exact `TASK5A_ACCEPTED_SHA` only in
   the successor implementation report/ledger.
4. Then implement/review/accept the section-3.11 Task4-v7 capture seam and
   record exact `TASK4_V7_ACCEPTED_GIT_OID` only in the successor implementation
   report/ledger. Neither prerequisite may be changed
   behind its accepted ID, and no Task5B production RED may precede both.
5. Implement Task5B with all permanent REDs, synchronize active spec/product
   contracts, run focused/full/fmt/clippy/product/Windows checks, independently
   review the implementation and only then record its accepted commit.

Time-bounded owner findings after the checks recorded above: **P0=0, P1=0**.
Hash-bound four-owner package-seam findings: **P0=0, P1=0**.
This statement makes no candidate/accepted/frozen/implementation-status claim,
publishes no hash/OID, and does not predict the current external ledger state.
Four-document alignment and independent-review evidence remain conditions that
the external package authority evaluates.
