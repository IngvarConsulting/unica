# Task 5C-Evidence v2 — production ParentConfigurations evidence boundary

Status: **conditional draft; not hashable and not implementation-ready**.

This artifact is the sole candidate contract for the read-only Evidence slice
of Task 5C. It intentionally contains no artifact-writer, WAL, mutation lease,
receipt transition, Task 8, Task 9, or Task 10 dependency. The older combined
`.superpowers/sdd/task-5c-v2-design.md` is working history only and supplies no
prerequisite authority for this artifact.

The delivery includes the parser, provenance, shared-catalog consumption,
Support query/provider, authority-preserving projection, live read, rendering,
typed assessment, and a temporary fail-closed applied `unica.support.edit`.
Durable support mutation is a separate downstream addendum.

## 0. Acyclic dependency and acceptance ledger

The only upstream implementation authorities are:

```text
accepted Task 5B v7 -> Task 6 implementation

accepted Task 4 snapshot/read boundary
  + accepted Task 5A raw support-fact/domain baseline
  + accepted Task 5B v7 neutral catalog + support-state-query/v2 seam
    -> accepted Task5C-Evidence design/review
    -> Task5C-Evidence implementation/review/commit

{Task4-v7 dynamic-material addendum,
 Task5B-v7 contract,
 Task6-v2-v7 addendum,
 Task7-v7 addendum} DESIGNS,
their owner self-audits and independent design reviews
  -> atomically co-freeze now as one provider/consumer package,
     without Task5A, Task4-v7, Task6 or Task5C-Evidence implementation OIDs

accepted Task 6 implementation + accepted Task5C-Evidence implementation
  -> Task 7 imports exact TASK5C_EVIDENCE_ACCEPTED_GIT_OID
  -> Task 7 production implementation/acceptance
  -> later writer/receipt work
```

Task 7 production is a **consumer**, never an Evidence prerequisite. The
co-frozen Task6/Task7 addenda are transitive lineage of the four-document design
package and therefore preexist this slice; Task5C-Evidence neither imports nor
revalidates either addendum and adds no direct Task6/Task7-owned gate. It imports
only the later accepted implementation boundaries of Task4-v7, Task5A and
Task5B-v7 named in the table below. No Task 6/7 implementation/OID/integration,
Task 8 import, or whole-Task5C state may gate this design, implementation,
review, or acceptance. Actual Task 7 code remains downstream and cannot start
until both accepted Task 6 and Task5C-Evidence implementation commits exist.

| Gate | Required immutable authority | Current state |
| --- | --- | --- |
| Task 4 | accepted 40-lowercase-hex implementation Git OID containing the verified optional snapshot leaf and retained containment/identity boundary | value must be recorded before freeze; the backward-compatible per-leaf max+1 method is owned by this Evidence implementation if absent |
| Task 5A | accepted 40-lowercase-hex implementation Git OID containing the seven append-only raw `SupportFactState` tags and typed metadata/membership facts | value must be recorded before freeze |
| Task 5B v7 | accepted contract SHA-256, separate no-P0/P1 independent-review SHA-256, and accepted 40-lowercase-hex implementation Git OID exporting the exact neutral catalog and `support-state-query/v2` seam in section 5 | does not yet exist; hard STOP |
| Task5C-Evidence design | final SHA-256 plus separate no-P0/P1 review SHA-256 | this file is conditional and unhashed |
| Task5C-Evidence implementation | accepted 40-lowercase-hex Git OID plus independent code/spec review SHA-256 | downstream |

Document hashes are exactly 64 lowercase hexadecimal characters. Repository
implementation identities are exact 40-lowercase-hex Git OIDs. A branch,
`HEAD`, dirty-tree pseudo-hash, self-audit substituted for independent review,
64-character value in a Git slot, or a generic `TASK5C_ACCEPTED_SHA` cannot
satisfy any row.

Task 5B v7 acceptance must prove/export its future-consumer seams; it must not
require this provider, Task 6, Task 7, or a later writer to exist.
Task5C-Evidence imports the exact Task 5B v7 names and encoders without aliases.
Task 6 is an independent Task5B consumer and has no Task5C dependency; any old
whole-Task5C prerequisite is deleted rather than replaced with an unnecessary
Evidence OID. Task 7, the real Support consumer, later names exactly
`TASK5C_EVIDENCE_ACCEPTED_GIT_OID`, never whole Task 5C or the future Mutation
slice. The already-preexisting co-frozen Task6/Task7 design addenda are only
transitive four-document lineage: Evidence neither imports/revalidates them nor
adds a direct Task6/Task7-owned gate, and it cannot satisfy, invent or move the
later Task7 Evidence OID.

Task5C-Evidence is an internal prerequisite commit, not a releasable endpoint.
It wires projection through the current application's exact final canonical
retained-record view; Task 7 later replaces/extends that one adapter for its new
admission pipeline without changing this semantic boundary. Until the later
mutation addendum lands, applied support.edit is intentionally disabled.
Product/release CI must reject publishing this intermediate state as a
completed feature.

This artifact remains unhashed until the four-document design package is
co-frozen, Task5B-v7 is implemented/accepted, and every provisional imported
type name below is replaced verbatim from that immutable boundary. There are no
hash-shaped placeholder values.

## 1. Production decision

One strict bounded byte parser owns the accepted ParentConfigurations subset.
One neutral Task 5B catalog owns MDClasses namespace, configuration flavor,
registered artifacts, UUIDs, and Own/Adopted membership. One Task 5B query seam
binds full source, artifact, catalog, freshness, and planned-absence authority.
The Support provider consumes those typed values; it never reparses MDClasses,
calls MetadataCatalog, or creates a local query encoder.

The corrections over the historical implementation are non-negotiable:

1. The only tracked serialized corpus is a synthetic parity/current-product
   compatibility fixture: BOM-prefixed, marker `6`, exactly one vendor, fully
   consumed, and duplicate-free. It is not Designer/export evidence. BOM-less
   and zero/multi-vendor inputs are unsupported by this compatibility version.
2. Global flag `1`, vendor editing flag `1`, and object flag `1` are explicit
   current-product compatibility contracts, not claimed Designer-fixture
   proof. Either enclosing flag `1` is configuration-wide read-only.
3. `ExtensionWithoutParentConfigurations` proves only a missing policy leaf
   for one exact extension snapshot. It never proves existence or ownership.
4. Existing extension ownership requires the exact Task 5B flavor plus
   Own/Adopted membership join. Planned destination absence has a distinct
   query authority and remains advice-only.
5. Raw preliminary Metadata facts may permit a Support query, but only facts
   retained by the downstream admission pipeline may participate in final
   projection. An authority index never resurrects a dropped fact.
6. Applied `unica.support.edit` performs zero writes in this slice and returns
   `support_edit_atomic_writer_required`. The unsafe path/string writer is
   removed or unreachable before Evidence acceptance.

## 2. Scope

### 2.1 Included

- canonical synthetic compatibility corpus and provenance manifest;
- pure bounded lexer/parser, immutable edit spans, semantic/skeleton/content
  digests, and stable reason vocabulary;
- exact consumption of the Task 5B v7 neutral catalog set;
- exact consumption of Task 5B v7 `support-state-query/v2`;
- one snapshot-bound Support provider with no lossy local record ceiling;
- authority index and provider-local atomic-group identity;
- ownership-aware direct and `CfePatchMethod` projection;
- one contained live reader and typed renderer/assessment migration;
- transitional fail-closed applied `support.edit` and read-only preview;
- exact RED/GREEN, documentation, package-contract, and acceptance gates.

### 2.2 Excluded

- any filesystem writer, temp/rename protocol, WAL, mutation lease, durability,
  receipt issue/advance/revoke, or serializable mutation claim;
- explicit present-file “not under support” serialization;
- zero/multiple vendor interpretation or duplicate aggregation;
- BOM-less, UTF-16, compressed, encrypted, or unknown containers;
- nested Form/Template/Command destructive authorization without a real
  owner-plus-child export and approved composition matrix;
- implicit `cfe.borrow`, borrow inside patch, or patch receipt for an absent
  destination object;
- a second configuration flavor/UUID/membership/MDClasses parser;
- local Support query encoding or decoding renderer/evidence text as authority;
- Task 7 admission implementation. This slice exports a complete group
  contract and compile fake for that downstream consumer.

## 3. Synthetic corpus provenance and compatibility

### 3.1 Canonical tracked synthetic corpus

The three tracked `ParentConfigurations.bin` files are byte-identical copies of
one synthetic parity corpus. Their test UUIDs (`aaaa...`, `111...`, `222...`,
`333...`) and `ТестВендор`/`ТестКонфиг` values match the repository's
`support_test_configuration_xml` generators; history introduces them in
commit `9d8fdf90b806a8af0b34d8d632ef4dff669d9260` as
`test: sync cc-1c parity corpus`. There is no verified
Designer donor/export manifest. Even if the bytes had been anonymized, exact-
byte export authority would have been lost. They are compatibility evidence
only.

The canonical tracked synthetic content is 337 bytes, strict UTF-8, begins with
exactly one `EF BB BF`, has no final newline, and has SHA-256:

```text
6750bbf0b567b5bf475ee8a3b2b00c5391dba487358cf05c47c77c07e01e90e3
```

It fixes only this versioned current-product/parity compatibility behavior:

- marker `6`;
- global flag `0`;
- exactly one vendor and vendor flag `0`;
- three object records;
- object flags `0` (`Locked`) and `2` (`Removed`);
- one record without a mirror and two with an equal mirrored UUID.

Implementation copies the bytes once to:

```text
tests/fixtures/project_discovery/parent_configurations/synthetic_parity_one_vendor_v2.bin
```

Its manifest records all source-copy paths, the introducing commit/message,
generator linkage, exact digest/length/BOM/newline facts, and says
`one synthetic compatibility corpus; zero verified Designer exports`. A test
compares each source copy byte-for-byte. Neither the path nor digest upgrades
the epistemic grade.

### 3.2 Compatibility-only rows

Current product code/tests additionally synthesize global flag `1`, vendor flag
`1`, object flag `1`, and capability-off output where all these flags are `1`.
Together with the tracked parity bytes, those values form one explicit
versioned compatibility subset because public tools already produce/consume
them. None may be described as exported Designer evidence.

```text
SupportSerializationAuthorityV2 =
    AcceptedCurrentProductCompatibilityV2    // sole v2 tag 1
```

Every accepted v2 document receives this same compatibility grade. The exact
337-byte digest is corpus provenance only and never a stronger authority tag.
The grade is diagnostic, not an authorization override. V2 applies:

```text
(global=0, vendor=0) -> Enabled
(global=0, vendor=1) -> ReadOnlyVendor
(global=1, vendor=0) -> ReadOnlyGlobal
(global=1, vendor=1) -> ReadOnlyGlobalAndVendor
```

Only `Enabled` permits an object-level conclusion. The other three states emit
`ConfigurationReadOnly` regardless of object rule bytes.

Unsupported means “Unica lacks interpretation authority”, not “the 1C file is
corrupt”. Malformed and unsupported present material both emit zero support
facts, but diagnostics keep the classes distinct.

## 4. Pure ParentConfigurations parser

Create the neutral module:

```text
crates/unica-coder/src/infrastructure/parent_configurations/
  mod.rs
  lexer.rs
  parser.rs
  encoder.rs
```

It has no filesystem, snapshot, catalog, provider, renderer, guard, or native
operation dependency.

### 4.1 Bounds

```text
MAX_PARENT_CONFIGURATIONS_BYTES        = 67_108_864
MAX_PARENT_CONFIGURATIONS_TOKENS       = 1_000_000
MAX_PARENT_CONFIGURATIONS_OBJECT_RULES = 200_000
MAX_PARENT_CONFIGURATIONS_QUOTED_BYTES = 4_096 per scalar
PARENT_CONFIGURATIONS_VENDOR_COUNT_V2  = 1
```

All arithmetic and capacities are checked. Reaching an N+1 bound returns one
non-retryable `parent_configurations_resource_limit`; no prefix, span, rule, or
digest survives.

### 4.2 Framing and lexer

Accepted bytes begin exactly `EF BB BF 7B` (BOM then `{`) and end at the
matching `}` at EOF. Leading/trailing whitespace, final newline, a second/mid
BOM, embedded NUL, invalid UTF-8, control byte, unbalanced quote/brace, or any
byte after the closing brace is rejected.

Outside quoted scalars, accepted bytes are ASCII canonical decimal digits,
lowercase canonical UUID bytes, comma, `{`, and `}`. `0` is the only zero
spelling; nonzero decimals have no leading zero. Serialized UUIDs are exact
lowercase, non-nil, 36-byte hyphenated `PlatformUuid` values. The neutral UUID
type still owns identity; this parser adds the serialization restriction.

Quoted version/vendor/configuration labels are strict UTF-8, nonempty, at most
4096 bytes, and contain no quote, backslash, NUL, Unicode control, or invalid
encoding. Commas/braces inside a quoted scalar are data. V2 defines no escape
syntax.

### 4.3 Accepted grammar

```text
Document := BOM "{" 6 "," Global "," 1 "," Vendor "," Rules "}"
Global   := 0 | 1

Vendor :=
  vendor_configuration_uuid ","
  vendor_editing_flag        ","
  vendor_instance_uuid       ","
  quoted_version             ","
  quoted_vendor              ","
  quoted_configuration       ","
  object_rule_count

Rules  := exactly object_rule_count Record values
Record := rule_flag "," 0 "," object_uuid [ "," mirrored_object_uuid ]
rule_flag := 0 | 1 | 2
```

The optional mirror is recognized only by its complete UUID token shape and
must equal `object_uuid`. It is part of one record, never a second rule. Every
primary object UUID is unique. An identical duplicate rule is
`duplicate_parent_configuration_rule`; a differing duplicate is
`conflicting_parent_configuration_rules`; both reject the entire document.

The declared count must equal the parsed record count and be at most 200,000.
Because the sole tracked compatibility corpus contains a positive record count,
zero records are an unsupported variant, not inferred empty-policy semantics.
Marker other than `6`, vendor count other than `1`, zero rules, or another
structurally coherent unproven shape is Unsupported. An out-of-domain flag or
broken token is Malformed. Extra well-formed tokens are trailing material.

Reason precedence is fixed: byte/resource; BOM/framing; UTF-8/NUL/control;
lexical truncation; marker/vendor/count variant; scalar/flag/UUID/record;
mirror; duplicate/conflict; declared-count mismatch; trailing material. Within
one stage the lowest byte offset wins, then stable reason tag. Empty/no-BOM is
unsupported framing; a valid BOM followed by incomplete material is truncated.

### 4.4 Typed result and spans

```rust
pub(crate) enum ParentConfigurationsParseV2 {
    Parsed(ParentConfigurationsDocumentV2),
    Rejected(ParentConfigurationsProblemV2),
}

pub(crate) enum ParentConfigurationsProblemClassV2 {
    Malformed, Unsupported, ResourceLimit,
}

pub(crate) struct ParentConfigurationsDocumentV2 {
    framing: ParentConfigurationsFramingV2,
    authority: SupportSerializationAuthorityV2,
    global_editing: GlobalEditingV2,
    vendor: ParentConfigurationVendorV2,
    rules: BTreeMap<PlatformUuid, ParentConfigurationRuleV2>,
    serialized_order: Vec<PlatformUuid>,
    edit_spans: ParentConfigurationsEditSpansV2,
    semantic_digest: Digest32,
    skeleton_digest: Digest32,
    content_digest: Digest32,
}
```

Fields and constructors are private. Every span slices exactly one admitted
ASCII flag byte from the same immutable byte buffer; spans are sorted,
disjoint, and in bounds. Mirror UUID bytes are immutable. `serialized_order`
is retained for exact-byte reasoning. The BTreeMap is lookup only; v2 does not
claim order-insensitive semantics.

### 4.5 Encoders

All encoders use SHA-256 over a NUL-terminated domain and unambiguous fixed/
length-prefixed payload. Integers are big-endian. UUID serialization is its
canonical 36-byte ASCII. `digest32` decodes exactly 64 lowercase hex (an
optional `sha256:` wire prefix is validated before decoding).

```text
H("unica.parent-configurations.semantic/v2",
  u16be(2) || framing_tag || authority_tag ||
  global_raw_tag || vendor_raw_tag || effective_editing_tag ||
  uuid(vendor_configuration_uuid) || uuid(vendor_instance_uuid) ||
  string(version) || string(vendor) || string(configuration) ||
  u32be(rule_count) ||
  vec(serialized order: uuid || rule_tag || mirror_tag))
```

Stable tags:

```text
framing Utf8Bom=1
authority AcceptedCurrentProductCompatibilityV2=1
raw flag 0=1, 1=2
effective Enabled=1, ReadOnlyVendor=2,
          ReadOnlyGlobal=3, ReadOnlyGlobalAndVendor=4
rule Locked=1, Editable=2, Removed=3
mirror Absent=1, PresentEqual=2
```

```text
H("unica.parent-configurations.skeleton/v2",
  u64be(original_len) || exact_bytes_with_each_valid_flag_byte_replaced_by_FF)
```

Content digest is ordinary SHA-256 over exact bytes. Semantic order is retained
deliberately. Debug/serde/display order, paths, timestamps, epoch, pointer
addresses, and provenance file location never enter these digests.

### 4.6 Root semantic boundary

The parser does not guess the configuration root UUID. The provider/live
assessment receives Task 5B v7's exact catalog-header
`configuration_root_uuid: ConfigurationRootUuidAuthorityV1` authority. This is
**not** a synthetic `ArtifactRef` or
`PlatformConfigurationObjectKeyV1`: Configuration itself is not assumed to be
an ArtifactKind/catalog entry. An accepted under-
support document missing that root UUID yields
`support_configuration_root_rule_missing` at the semantic join and zero facts
for the source group. It is never `ObjectNotListed` for the root. A registered
child absent from the complete rule map may become `ObjectNotListed`.

## 5. Imported Task 5B v7 authority

### 5.1 Neutral catalog

Task 5B v7 must freeze and implement the exact successor of these v6-neutral
authorities (final names are imported verbatim before this file is hashable):

```text
platform-configuration-catalog/v1
platform-configuration-catalog-encoder/v1
platform-configuration-catalog-set/v1
platform-configuration-object-key/v1

PlatformConfigurationCatalogV1
PlatformConfigurationObjectAuthorityV1
PlatformConfigurationCatalogSetV1
RegisteredFormCatalogSetV1
PlatformCatalogContextV1
PlatformConfigurationCatalogWitnessSetV1
RegisteredFormCatalogWitnessSetV1
PlatformConfigurationObjectKeyV1
PlatformCatalogPort
ConfigurationRootUuidAuthorityV1
```

The application invokes the port exactly once per composite snapshot before
Metadata, FormInspection or Support. It stores the one atomic
`PlatformCatalogContextV1` containing the exact matching configuration and
registered-Form sets plus their opaque witness sets in
`EvidenceExecutionContext`; every consumer/query constructor borrows it through
the context-owned lifetime/opaque handle. The context/catalog/set values are not
externally constructible from public fields, so a consumer cannot substitute a
config-only, sidecar-only or second parse/build. A semantically equal immutable
clone is not itself drift and pointer/object address is never authority; the API
prevents reconstruction and the recording seam proves one build call plus all
borrows from that context. The set/catalog semantic digests bind full source identity, source fingerprint,
capture-catalog digest, semantic Base/Extension flavor, registered ArtifactRef,
wrapper UUID, and Own/Adopted membership.

Support must not import a Metadata adapter, parse ProviderFact/display/JSON,
reparse MDClasses, use a local-name XML helper, or trust declared source kind.

Task 5B v7 adds the exact catalog-header field:

```text
PlatformConfigurationCatalogV1.configuration_root_uuid:
  ConfigurationRootUuidAuthorityV1

ConfigurationRootUuidAuthorityV1 =
    Known(PlatformUuid)                  tag 1
  | Inconclusive(ConfigurationRootUuidProblemV1) tag 2

ConfigurationRootUuidProblemV1 =
    Missing          tag 1
  | Duplicate        tag 2
  | WrongNamespace   tag 3
  | InvalidOrNil     tag 4
```

It is parsed only from the one direct exact-`MDCLASSES_NS`
`MetaDataObject/Configuration@uuid` and enters the catalog payload/digest.
Missing, duplicate, wrong-namespace, invalid, or nil values are Inconclusive;
they never fabricate an entry/key. A verified Missing ParentConfigurations leaf
does not need a root-rule lookup and may still produce the exact missing-policy
fact when flavor/subject authority is otherwise complete. Any Present document
requires `Known(root_uuid)` for root-rule validation; Inconclusive then produces
the exact scoped gap and zero facts.

### 5.2 Task 5B-owned Support query seam

Task 5B v7, not Task 5C, owns the query types, private constructors, stable
tags, encoder, digest goldens, and maximum subject bound. The coordinated
domain and policy versions, still conditional until v7 acceptance, are:

```text
SUPPORT_STATE_QUERY_ENCODER = "support-state-query/v2"
SUPPORT_LOOKUP_UUID_POLICY  = "support-lookup-uuid/v2"

unica.support-state-query/v2
```

This draft intentionally does not invent an alternate `unica.support-query`
encoder. Before freeze, the exact accepted Task 5B v7 Rust type/constant names
replace the descriptive names in this subsection verbatim. The imported seam
must provide these semantic fields with no lossy substitute:

```rust
pub(crate) struct SupportStateQueryV2<'catalog> {
    composite_snapshot_id: /* exact Task 5B v7 snapshot-id type */,
    catalog_context: &'catalog PlatformCatalogContextV1,
    groups: Vec<SupportSourceGroupV2>,
}

pub(crate) struct SupportSourceGroupV2 {
    source: AtomicSourceIdentityV2,
    catalog_digest: Digest32,
    subjects: Vec<SupportQuerySubjectV2>,
}

pub(crate) struct SupportQuerySubjectV2 {
    source_scoped_artifact: SourceScopedArtifact,
    authority: /* exact private Task 5B v7 closed enum */,
    semantic_authority_digest: SupportSubjectSemanticAuthorityDigestV2,
    snapshot_authority_digest: SupportSubjectSnapshotAuthorityDigestV2,
}

// Existing tag 1:
//   { object_key: PlatformConfigurationObjectKeyV1,
//     lookup_uuid: SupportLookupUuidAuthorityV2 }
// PlannedDestinationAbsent tag 2:
//   { authority: PlannedDestinationAbsentV1,
//     lookup_uuid: SupportLookupUuidAuthorityV2 }
```

`ArtifactRef` means its complete `ArtifactIdentityBytesV1`, never a canonical
name/string. `AtomicSourceIdentityV2` includes role plus complete resolved source
identity; equal artifacts in base and extension remain distinct. The private
query constructor resolves every `(source, catalog_digest)` to exactly one
catalog in the borrowed set and thereby binds that catalog's source
fingerprint, capture-catalog digest, configuration-root authority, set digest,
and composite snapshot identity without duplicating drift-prone fields in the
group. A caller cannot supply an arbitrary digest, UUID, source label, or
display path.

The Existing key bytes remain exactly:

```text
digest32(catalog_digest) || ArtifactIdentityBytesV1(artifact)
```

They resolve to exactly one entry in exactly one borrowed catalog. Missing,
duplicate, flavor/membership-inconclusive, source/fingerprint mismatch, or a
key from another catalog cannot construct a subject.

The key proves catalog membership; it does not by itself prove which serialized
UUID a ParentConfigurations object rule uses. Both query authority variants
therefore carry exact Task 5B type `SupportLookupUuidAuthorityV2`:

```text
Known { uuid, basis }                 tag 1
Inconclusive(exact problem)           tag 2

Known basis:
BaseOwnCatalogWrapper                 tag 1
PlannedBaseOwnCatalog                 tag 2

Inconclusive problem:
UnsupportedRootMetadataKind           tag 1
NestedArtifactMappingUnproven         tag 2
ExtensionOwnMappingUnproven           tag 3
ExtensionAdoptedMappingUnproven       tag 4
CatalogAuthorityInconclusive          tag 5
```

That authority and tag enter the query bytes. The synthetic parity corpus plus
current product generator behavior are adopted as an explicit compatibility
contract for the configuration-root invariant and exact
BaseConfiguration+Own Catalog wrapper UUID rules only; these are currently the
sole Existing Known row. They are not Designer proof. Other root
kinds, every nested Form/Template/Command, and Extension Own/Adopted remain
Inconclusive. A new Known row requires real accepted evidence or an explicitly
labelled versioned compatibility contract plus a new reviewed policy version;
it is never generalized from ArtifactRef membership. Inconclusive lookup
is not a group-wide failure: verified Missing policy and Present configuration-
wide ReadOnly remain classifiable when source/flavor/membership/root authority
needed by those rows is exact. Only Enabled per-object rule lookup or
ObjectNotListed requires `Known`; there Inconclusive produces its exact scoped
support gap and never a guessed fact.

The source group's resolved catalog supplies the separate Task 5B v7
`ConfigurationRootUuidAuthorityV1` bound by the catalog digest and complete
source freshness. It is
used only for the required root-rule invariant. The ParentConfigurations leaf
location/absence comes from the captured manifest for that exact source. A
fake `ArtifactRef::Configuration`, a root `PlatformConfigurationObjectKeyV1`,
or any fabricated catalog entry is a constructor and product-contract failure.

The planned-absence variant is Task 5B-owned because it is made solely from
the exact cross-source Metadata pair/absence authority. Its full snapshot
constructor validates/retains:

- the exact `DestinationMembershipPair`;
- full analysis and destination `AtomicSourceIdentityV2` values;
- the exact analysis BaseConfiguration+Own present-object observation;
- exact destination ExtensionConfiguration + object Absent observation;
- analysis and destination semantic fact digests;
- separate analysis/destination source fingerprints and catalog digests;
- the common catalog-set/composite-snapshot authority;
- sorted unique nonempty provenance evidence IDs as diagnostics only.

Task 5B v7 must export two non-interchangeable authority layers:

```text
SupportSubjectSemanticAuthorityDigestV2
  source-free: authority tag + source-free subject/lookup/pair semantics

SupportSubjectSnapshotAuthorityDigestV2
  semantic authority digest + full source/catalog/composite/freshness/fact
  binding used by query, physical record and retained join
```

The source-free digest excludes AtomicSourceIdentity, source fingerprints,
catalog/set/object-key digests, composite/query identity, physical location,
and evidence IDs. Existing semantics bind the subject Artifact identity,
authority tag and `SupportLookupUuidAuthorityV2`. Planned semantics additionally
bind the source-free pair artifact, analysis base UUID, exact Base+Own and
destination Extension+Absent semantic polarities, and lookup authority.

The snapshot digest binds that semantic digest plus full analysis/destination
AtomicSourceIdentity values, exact object key or full planned pair authority,
separate source fingerprints/catalog/capture digests, fact digests,
catalog-set/composite identity, and all constructor invariants. Provenance
evidence IDs are retained only to locate/validate the exact records; after
their fact/freshness digests are recomputed, the IDs remain diagnostics-only and
enter neither digest. This avoids both location-dependent semantics and an
evidence-ID hash cycle.

The authority cannot be constructed from generic MetadataAbsent, a name, a
UUID alone, a declared extension label, stale record, or one source half.

The current moving Task 5B v7 draft now owns and defines
`PlannedDestinationAbsentV1`, its private constructor invariants, encoder
domain/field order/tags and exact goldens. It also owns the distinct source-free
semantic and full snapshot-authority digests plus authority-bound Support
atomic-group tag 9. That conditionally closes the earlier architecture P1;
Task5C still imports nothing until the atomic Task4/Task5B/Task6/Task7 design
co-freeze and accepted Task5B implementation OID make those exact symbols
immutable. Task5C may not copy, alias, or redefine them if the moving names
change; importing a Task5C-defined planned authority would recreate a cycle.

The variant is query authority only. It permits asking how the destination
policy treats the analysis UUID; it does not assert that the destination object
exists, is owned, is adopted, or is patchable.

Task 5B v7 owns `MAX_SUPPORT_QUERY_SUBJECTS = 4096` as an all-or-nothing query
constructor bound. Exactly 4096 distinct semantic subjects pass; 4097 rejects
before provider I/O. Identical duplicate subjects canonicalize before the
bound. Two entries with equal source/artifact but different authority reject as
`support_query_authority_conflict`; they are never silently deduplicated.

The coordinated exact query payload is:

```text
u16be(schema=2) || bytes(composite_snapshot_id) ||
digest32(platform_configuration_catalog_set_digest) ||
vec(groups:
  bytes(AtomicSourceIdentityV2(source)) || digest32(catalog_digest) ||
  vec(subjects:
    SourceScopedArtifactIdentityBytesV2(subject) || u16be(authority_tag) ||
    [bytes(PlatformConfigurationObjectKeyBytesV1(existing_key)) |
     digest32(planned_absence_digest)] ||
    support_lookup_uuid(
      Known: u16be(1) || string(canonical_uuid) || u16be(basis_tag) |
      Inconclusive: u16be(2) || u16be(problem_tag))))
```

The digest is `H("unica.support-state-query/v2", payload)`. Group order is
`AtomicSourceIdentityBytesV2`; subject order is complete source-scoped artifact
bytes followed by authority payload; vectors are sorted/unique. Exact full
goldens must be imported from accepted v7; abbreviated values are forbidden.
The digest excludes absolute path, provider order, display, wall clock, public
`maxEvidence`, task/search text, and caller-provided opaque digest.
Task5C-Evidence calls only the imported digest; there is no local query encoder
or adapter wrapper.

The accepted v7 golden must remain exactly:

```text
Known Existing subject = MetadataObject "Catalog.Σ"
payload length = 567
SHA-256(payload) =
  b914c02ae099b3f92bb7113cde6882bc71d373552eb90524ef2dfe991ba8c5e7
H("unica.support-state-query/v2", payload) =
  a32ee5f4f9f3145a8127a00e09160ce5f37415fe09dcf84c85768a775e400ed4
```

Task5C tests call the imported encoder and assert these values; they do not
copy its implementation.

### 5.3 Dependency gate for this draft

Until the atomic Task4-v7/Task5B-v7/Task6-v2-v7/Task7-v7 package publishes its
immutable design/review hash tuple and Task5B-v7 publishes the implementation
Git OID containing these exact type names, tag tables and query bytes/goldens,
this section is `UNRESOLVED_DEPENDENCY` and the whole Evidence artifact remains
unhashed. Evidence imports the implemented Task4/Task5B seams, not the Task6/
Task7 addenda; their co-freeze is transitive acceptance lineage. The resolution
procedure imports exact names/bytes and must not “make them match” through
aliases or a Task5C encoder.

## 6. Snapshot-bound Support provider

Create:

```text
crates/unica-coder/src/infrastructure/discovery/support.rs
```

Provider identity:

```text
provider = unica.support_state / 2
parser   = parent-configurations/v2
reasons  = parent-configurations-reasons/v2
```

It accepts only the borrowed Task 5B v7 query plus verified snapshot reader.
One source group causes one optional leaf read/parse and complete conclusions
for all its subjects.

### 6.1 Bounded snapshot read

The exact optional manifest leaf is `Ext/ParentConfigurations.bin` under each
source root. The provider examines the immutable manifest entry first:

- verified tombstone -> Missing without body I/O;
- Present declared length above 64 MiB -> resource-limited without body read;
- eligible Present -> one verified bounded read that consumes at most
  `min(declared_length + 1, 64 MiB + 1)` and proves manifest identity/content;
- NotInManifest for this declared optional key -> provider contract violation.

Task 4 currently exposes an optional verified reader whose implementation may
allocate by captured file length. Task5C-Evidence owns the smallest backward-
compatible `read_optional_verified_bounded(..., max_bytes)` extension in the
Task 4 port/implementation when that exact method is absent. It preserves the
accepted containment, manifest identity, final revalidation, and optional-leaf
semantics; the only new authority is the caller-supplied checked max+1 ceiling.
It lands and is reviewed in the Evidence commit, not as a new upstream task and
not through direct provider filesystem I/O. Any implementation that cannot add
this method without weakening Task 4 is STOP.

| Snapshot outcome | Support outcome | Retryable |
| --- | --- | --- |
| verified Missing | complete missing-policy fact per resolvable subject | no |
| verified Present + accepted v2 | complete known fact per subject | no |
| malformed/unsupported/resource-limited Present | one exact source-group Bounded gap, zero facts | no |
| source identity/content changed | Unavailable `source_fingerprint_mismatch`, zero group facts | yes |
| reader unavailable/deadline | exact inherited Unavailable reason, zero group facts | inherited closed bit |
| optional key absent from manifest | ProviderContractViolation | no |
| query/catalog/fingerprint mismatch | ProviderContractViolation | no |

No prefix from a failed group survives. A shared catalog-set invariant failure
fails the whole invocation. Otherwise source groups remain independently
scoped; this never claims cross-provider atomicity.

### 6.2 Raw facts

Task 5A's append-only tags remain unchanged:

```text
Editable                              1
Locked                                2
ConfigurationReadOnly                 3
Removed                               4
ObjectNotListed                       5
BaseWithoutParentConfigurations       6
ExtensionWithoutParentConfigurations  7
```

For Existing authority:

- known Base + verified Missing -> `BaseWithoutParentConfigurations`;
- known Extension + verified Missing ->
  `ExtensionWithoutParentConfigurations`;
- unknown/inconclusive flavor -> exact gap, no fact;
- parsed effective state other than Enabled ->
  `ConfigurationReadOnly` for every exact subject;
- Enabled + exact registered lookup UUID -> its Locked/Editable/Removed rule;
- Enabled + exact registered child lookup UUID absent from complete map ->
  `ObjectNotListed`;
- known configuration root absent ->
  `support_configuration_root_rule_missing`, zero source-group facts;
- Present policy plus Inconclusive configuration-root UUID -> the exact Task 5B
  header-authority gap, zero source-group facts; Missing policy remains
  classifiable without a rule lookup;
- unresolved entry/UUID/membership/source -> exact gap, never ObjectNotListed.

The imported Task 5B v7 Existing authority supplies the exact typed support
lookup UUID authority. The synthetic parity/current-product corpus is adopted
only as the explicit Base Own Catalog compatibility row, not as Designer proof
or an all-kinds rule. A Base or Extension
kind/membership mapping may be used only if Task 5B v7 freezes it from verified
primary evidence or another explicitly versioned compatibility contract.
Otherwise an Enabled Present file yields the exact
`support_rule_uuid_unproven` gap, not a guessed per-object fact. Missing policy
and configuration-wide read-only remain classifiable because they do not need
that lookup.

The total lookup-use matrix is therefore:

| Leaf/effective policy | Lookup Known | Lookup Inconclusive |
| --- | --- | --- |
| verified Missing + exact flavor/subject authority | exact BaseWithout/ExtensionWithout | same exact missing-policy fact |
| Present ReadOnly + Known configuration-root invariant | ConfigurationReadOnly | ConfigurationReadOnly |
| Present Enabled | exact rule or ObjectNotListed | exact lookup problem gap, zero fact |
| Present + configuration-root authority/rule invalid | source-group gap | source-group gap |

For PlannedDestinationAbsent only:

- verified Missing extension policy -> scoped
  `ExtensionWithoutParentConfigurations`;
- parsed configuration-wide read-only -> `ConfigurationReadOnly`;
- Enabled and exact analysis-base lookup UUID absent from complete map ->
  scoped `ObjectNotListed`;
- any rule for that UUID contradicts destination Metadata absence ->
  `support_absent_destination_rule_conflict`, no fact;
- it never emits Editable, Locked, or Removed.

The provider never emits public Unknown, ExtensionOwned, ExtensionRequired,
receipt eligibility, or mutation authority.

### 6.3 Authority survives collection but cannot create evidence

The imported query exposes a canonical subject-authority index. Support
collection returns:

```rust
pub(crate) struct CollectedSupportStateV2 {
    provider_outcome: CollectedProviderOutcome,
    authority_index: SupportAuthorityIndexV2,
}
```

`SupportAuthorityIndexV2` is a Task5C view over exact imported subject authority
digests; it stores both imported semantic/snapshot authority digests, the
imported Task 5B query digest, and borrowed canonical subject -> authority
entries. It has no independent semantic digest and does
not re-encode the query. Every returned support record resolves to one exact
query subject. A record without authority, authority mismatch, or an authority
without its exact query subject is a contract violation.

The index travels with records through the downstream consumer boundary. It is
audit/join context only:

- index without retained support record -> no fact/projection;
- support record without retained Metadata companions for planned absence ->
  final Unknown;
- dropped Metadata or support record cannot be reconstructed from raw outcome,
  index, provenance ID, or query;
- a source/fingerprint/catalog mismatch invalidates the join.

### 6.4 Provider-local atomic group

Support has no local lossy record ceiling and never emits
`platform_xml_result_limit`. It fully parses, validates root/catalog/query, and
builds all records before returning.

Current Task 5B `StandaloneFact` is insufficient: its Support fact payload
contains only `SupportFactState`, so equal source/subject/state values queried
under Existing versus PlannedDestinationAbsent authority could collapse or be
substituted. Task5C must not repair that with a private nested hash.

Task 5B v7 therefore owns and exports a new closed authority-bound shared
`SemanticAtomicGroupIdV2` variant (coordinated shape, exact accepted name/tag
and encoder still required):

```text
SupportStateObservation {
  source: AtomicSourceIdentityV2,
  subject: SourceScopedArtifact,
  support_subject_semantic_authority_digest: Digest32,
}
```

Its exact tag/payload/domain/golden enter the shared Task 5B atomic-group and
semantic-cluster contract. The group digest is deliberately source-free apart
from the existing outer source key: it must not contain fingerprint, catalog
set/digest, object-key digest, query/composite ID, evidence ID, or location.

The shared **physical** Support record binding separately includes the exact
`SupportSubjectSnapshotAuthorityDigestV2` and upstream
`SupportStateQueryV2.query_digest`; Task 5B owns its tag/encoding/goldens. The
Support record constructor receives the smart-constructed imported query
subject, recomputes both authority digests, and can produce only this semantic
group plus physical binding. Response validation proves the record's source,
subject, semantic authority, snapshot authority, and query all equal the exact
query entry. An Existing record cannot be relabelled PlannedAbsent (or the
reverse) even when raw state is equal. A legacy Support `StandaloneFact`,
freshness in the source-free group, missing physical authority/query binding,
Task5C-local group encoder, or authority stored only in the side index is a
contract failure.

This remains one provider-local group per exact observation, not a cross-
provider group: Metadata companions remain separate. Until Task 5B v7 freezes
and implements the shared variant/tag/golden, Evidence is STOP.

Task 7 later imports this Task5B-owned, Evidence-populated group value and is the first
lossy admission layer. It must drop/admit complete groups and make the final
Metadata+Support join Unknown when either half is absent. That future import is
tested by a compile/recording consumer in this slice but cannot gate Evidence
acceptance.

## 7. Ownership-aware projection

Delete context-free `SupportFactState::direct_projection()`. Raw support state
alone has no public projection.

```rust
fn project_support_v2(
    observation: RetainedSupportObservationV2,
    authority: RetainedSupportOwnershipAuthorityV2,
    intent: SupportProjectionIntentV2,
) -> SupportProjectionV2;
```

The retained input types are smart-constructed from exact source-scoped records
plus the query authority index. For planned absence they require both exact
Task 5B Metadata halves and the support record. Their constructor rejects raw-
only/pre-admission handles and mismatched fact/source/catalog/authority digests.
Evidence acceptance supplies the pure constructor/projector and a recording
future-consumer fake; Task 7 later supplies its canonical retained-record view.

The compile seam is `RetainedEvidenceSetViewV2`: it exposes membership only by
exact physical record digest plus the admission-snapshot digest, never a
boolean supplied beside a raw record. `validate_retained_support_join_v2`
resolves all requested digests through that view, the authority index, and the
borrowed catalog set before it can privately construct either retained input.
There is no public `new_unchecked`, serde constructor, or blanket trait
implementation. Evidence lands the trait/validator/projector, malicious
recording fakes, and exactly one production adapter over the current
application's final canonical retained-record set (never a raw provider batch).
Task 7 later migrates that same adapter to its canonical accumulator; a
product-contract implementation count prevents parallel/raw adapters. This is
an acyclic future-consumer contract, not an upstream Task 7 dependency.

### 7.1 Direct candidate matrix

Exact present BaseConfiguration + Own:

| Raw state | Public state |
| --- | --- |
| Editable | editable |
| Locked | locked |
| ConfigurationReadOnly | configuration_read_only |
| Removed | removed |
| ObjectNotListed | not_under_support |
| BaseWithoutParentConfigurations | not_under_support |
| extension-only or authority gap | unknown |

Exact present ExtensionConfiguration + exact Own/Adopted membership in the
same source:

| Raw state | Public state |
| --- | --- |
| Locked | locked |
| ConfigurationReadOnly | configuration_read_only |
| Editable, Removed, ObjectNotListed | extension_owned, only when Task 5B v7 owns the exact extension rule lookup UUID |
| ExtensionWithoutParentConfigurations | extension_owned |
| base-only, absent target, or authority gap | unknown |

`ExtensionWithoutParentConfigurations` participates only after independent
present ownership is proven. It never supplies that proof.

### 7.2 CfePatchMethod matrix

The current patch intent writes a distinct destination extension. Required
authority is:

1. exact analysis BaseConfiguration + Own UUID for every required owner;
2. exact destination ExtensionConfiguration;
3. exact Task 5B destination membership polarity for every owner;
4. exact support query authority and retained support observation;
5. freshness of analysis and destination halves against their own catalogs.

Precedence:

1. malformed/unavailable/gapped/missing retained authority -> Unknown,
   `support_state_inconclusive` plus exact reasons;
2. destination ConfigurationReadOnly -> blocking configuration_read_only;
3. destination Locked -> blocking locked;
4. destination Own, wrong adopted UUID, or inconclusive membership -> exact
   Task 5B Unknown blocker;
5. one or more destination owners exact Absent plus retained safe planned-
   absence support -> extension_required + `destination_borrow_required`;
6. every owner exact Adopted from the matching analysis UUID plus safe retained
   destination support -> extension_owned;
7. no default row; a new intent requires a versioned matrix.

ExtensionRequired is visible actionable guidance for a separate explicit
`unica.cfe.borrow` operation only. For `unica.cfe.patch_method` it has:

```text
receiptEligibility = false
resolver calls      = 0
issuer calls        = 0
handler calls       = 0
implicit borrow     = forbidden
```

Analysis Locked/read-only does not override a proven safe destination because
the patch does not directly edit analysis, but unknown analysis support remains
blocking. Identical artifact refs in different sources never merge.

## 8. Live read, renderers, and assessment

### 8.1 Contained live reader

Create:

```text
crates/unica-coder/src/infrastructure/parent_configurations/live_reader.rs
```

It opens the trusted workspace/configuration root once and walks relative
components through the existing retained no-follow capability. It never uses
`canonicalize`, `exists`, `is_file`, prefix-string containment, or an absolute
reopen.

```rust
pub(crate) struct ParentConfigurationsObservationV2 {
    source: AtomicSourceIdentityV2,
    source_fingerprint: Digest32,
    capture_catalog_digest: Digest32,
    catalog_digest: Digest32,
    optional_leaf_authority_digest: Digest32,
    read: ParentConfigurationsReadV2,
}

pub(crate) enum ParentConfigurationsReadV2 {
    Missing(VerifiedOptionalAbsenceV1),
    Parsed(ParentConfigurationsDocumentV2),
    Rejected(ParentConfigurationsProblemV2),
    IoFailure(ParentConfigurationsIoFailureV2),
}
```

Missing requires an exact negative lookup at the expected leaf below a retained
parent. Present reads are bounded max+1. Symlink/reparse/special object,
containment ambiguity, identity change, short/long read, resource excess, or
metadata/query failure is explicit Rejected/IoFailure, never Missing.

This live boundary is informational/preflight only. It is not serializable
mutation authority and makes no race-free write claim.

The reader does not resolve target UUID/flavor/membership itself. A legacy
info/assessment caller first captures the exact one-source Task 4 snapshot and
invokes the same neutral `PlatformCatalogPort` once for that
capture. The bounded optional read is verified against that snapshot and the
outer `ParentConfigurationsObservationV2` binds the same source identity,
fingerprint, capture/catalog digests, and exact optional-leaf manifest
authority. A constructor rejects
any mismatch before assessment. If the caller cannot establish that shared
freshness, assessment is Indeterminate. A live-only local Configuration.xml or
object-XML parser is forbidden.

### 8.2 Rendering

Every current support renderer receives the typed result. Required visible
states are:

- missing policy;
- Enabled with exact object state;
- configuration read-only with global/vendor reason;
- known object not listed;
- unsupported format;
- malformed material;
- resource-limited material;
- I/O/identity unavailable;
- unresolved catalog/UUID/membership;
- unproven extension rule UUID.

Unsupported/malformed/unavailable must never render as “not under support”,
“free”, editable, or owned. Human strings are presentation only and never
parsed back into discovery, guard, or query authority.

### 8.3 Typed assessment

```rust
pub(crate) enum SupportAssessmentV2 {
    Safe(SupportSafetyProofV2),
    Violation(SupportViolationV2),
    Indeterminate(SupportIndeterminateV2),
}
```

Assessment receives typed parsed/read state, exact catalog target, operation
requirement, and policy mode. Known rules preserve current operation
requirements:

- ConfigurationReadOnly blocks every guarded direct mutation;
- Editable requirement blocks Locked;
- Removed requirement blocks Locked and Editable, allows Removed;
- exact ObjectNotListed/BaseWithout permits compatible direct mutation;
- safe owned extension uses the ownership matrix above;
- unresolved target/parser/I/O/catalog/membership is Indeterminate.

`deny` blocks Violation and Indeterminate. `warn`/`off` may alter only support-
policy disposition for the existing legacy operation path; they do not turn
this read into writer authority. Evidence docs/tests must state the residual
race until the separate mutation addendum lands.

## 9. Transitional applied support.edit STOP

Evidence acceptance removes or makes unreachable the existing
`fs::write`/string-replacement/path-reread implementation. Applied
`unica.support.edit` returns:

```text
ok=false
reasonCode=support_edit_atomic_writer_required
source writes=0
temporary files=0
renames=0
```

This applies in every policy mode and for every action. A dry-run may expose a
clearly non-authoritative typed preview derived from one parsed byte buffer,
including exact proposed flag spans and postcondition, but it returns no
mutation plan, receipt eligibility, or claim that apply is available.

Missing file remains a typed preview NoChange; it is not created. Explicit
not-under-support serialization is unavailable. Malformed/unsupported/I/O is
fail-closed. No alternate legacy operation, script, direct packaged path, or
fallback may reactivate the writer.

## 10. Stable reasons and problem encoder

`ParentConfigurationsProblemV2` stores class tag, stable reason tag, and an
optional bounded byte offset only. It never stores a path/raw token. Digest:

```text
H("unica.parent-configurations.problem/v2",
  class_tag || u16be(reason_tag) || offset_option_tag || [u64be(offset)])
```

| Tag | Class | Code |
| ---: | --- | --- |
| 1 | malformed | invalid_parent_configurations_encoding |
| 2 | unsupported | unsupported_parent_configurations_framing |
| 3 | malformed | parent_configurations_embedded_nul |
| 4 | resource | parent_configurations_resource_limit |
| 5 | malformed | truncated_parent_configurations |
| 6 | unsupported | unsupported_parent_configurations_variant |
| 7 | malformed | invalid_parent_configurations_global_flag |
| 8 | malformed | invalid_parent_configurations_vendor_flag |
| 9 | malformed | invalid_parent_configurations_uuid |
| 10 | malformed | invalid_parent_configurations_quoted_scalar |
| 11 | malformed | invalid_parent_configuration_rule |
| 12 | malformed | parent_configuration_mirror_uuid_mismatch |
| 13 | unsupported | duplicate_parent_configuration_rule |
| 14 | unsupported | conflicting_parent_configuration_rules |
| 15 | malformed | parent_configurations_object_count_mismatch |
| 16 | malformed | parent_configurations_trailing_material |

Application/provider reasons are separately closed:

```text
support_subject_unresolved
support_configuration_flavor_inconclusive
support_membership_inconclusive
support_configuration_root_rule_missing
support_rule_uuid_unproven             // exact imported Task5B spelling
support_absent_destination_rule_conflict
support_query_authority_conflict
support_state_inconclusive
source_fingerprint_mismatch
support_query_subject_limit            // exact imported Task5B spelling
support_edit_atomic_writer_required
```

The exact Task 5B v7 query-limit/conflict spellings replace the descriptive
rows above before freeze; Task5C does not alias them.

## 11. Mandatory RED -> GREEN

Every RED is written first, run alone, and recorded with a nonzero test count.
A zero-test filter is failure.

### 11.1 Provenance/parser

- all three tracked copies equal the canonical 337-byte synthetic corpus;
- manifest records `9d8fdf90b806a8af0b34d8d632ef4dff669d9260`, generator
  linkage, one synthetic corpus,
  zero verified Designer exports, and never calls the copies donors;
- every accepted document, including flag-1 helpers, has only
  AcceptedCurrentProductCompatibilityV2 authority;
- exact tracked synthetic file parses with expected record/mirror states;
- all four global/vendor pairs cross all object flags; only `(0,0)` emits
  object state;
- missing/double/mid BOM, BOM after whitespace, final newline, invalid UTF-8,
  UTF-16, NUL/control, escape, truncation, trailing bytes reject exactly;
- marker, vendor count 0/2, zero rules, integer overflow are Unsupported;
- noncanonical numbers/UUIDs/scalars/flags reject;
- equal and unequal mirror cases classify exactly;
- identical/conflicting duplicate UUIDs reject independent of order;
- declared count lower/higher rejects;
- exact N bounds pass where a complete valid corpus fits; N+1 rejects before
  oversized allocation;
- rejected parse exposes no spans/rules/digests;
- semantic/skeleton/content goldens and tag uniqueness pass;
- flag-only changes preserve skeleton and change semantic/content;
- order/framing/UUID/label/mirror changes alter skeleton.

### 11.2 Imported catalog/query seam

- neutral catalog port runs once per composite snapshot;
- catalog port build count is exactly one and both consumers accept only the
  context-owned borrowed handle/lifetime plus matching digests;
- no test uses pointer/address equality; private construction prevents an
  external second parse/rebuild while a moved/equal immutable value is not
  misclassified as semantic drift;
- same artifact across base/extension stays source-distinct;
- Support has no Metadata adapter/display/local MDClasses dependency;
- exact Task 5B v7 query digest goldens are consumed unchanged;
- a Task5C local query encoder symbol/path causes product-contract failure;
- full AtomicSourceIdentity and Artifact identity differences change subject;
- source fingerprint, capture/catalog/set digest drift rejects pre-I/O;
- synthetic Configuration ArtifactRef/root object key is rejected; the exact
  catalog-header configuration UUID and manifest leaf are used instead;
- exactly 4096 unique subjects pass and 4097 reject pre-I/O;
- identical duplicates canonicalize; conflicting authority duplicates reject;
- maxEvidence/display/order/path do not enter imported query identity;
- planned absence cannot construct from one half, generic absence, stale fact,
  name, or UUID alone.

### 11.3 Provider

- one read per source group, never per subject;
- verified Base/Extension Missing emits exact missing fact but ExtensionWithout
  never direct-projects ownership;
- four-state enclosing flags and vendor=1 no-object-fact invariant;
- lookup Known/Inconclusive crossed with Missing/ReadOnly/Enabled: only Enabled
  requires Known; Missing and configuration-wide ReadOnly retain exact facts;
- Enabled exact existing UUID maps Locked/Editable/Removed;
- registered child absence maps ObjectNotListed, root absence is group gap;
- per-object lookup for an unproven base/extension kind or nested artifact is
  exact Unknown gap; only Task5B-owned lookup authority may select a UUID;
- malformed/unsupported/resource material emits zero group facts;
- 64 MiB is read-eligible, 64 MiB+1 performs zero body read/allocation;
- source race is retryable Unavailable, no prefix;
- NotInManifest/catalog mismatch is contract violation;
- planned absence Missing/read-only/Enabled-absent/conflicting-rule matrix;
- returned record authority mismatch is contract violation;
- no provider-local lossy ceiling or `platform_xml_result_limit` branch;
- every Support record uses the imported authority-bound shared atomic-group
  variant plus snapshot/query-bound physical record; legacy StandaloneFact,
  freshness-dependent semantic groups, side-index-only authority, and
  Task5C-local group hashes fail response/product contracts;
- forward/reverse source/subject order yields byte-identical output.

### 11.4 Admission/projection

- authority index alone creates no fact;
- retained Support but dropped analysis Metadata -> Unknown;
- retained Support but dropped destination Metadata -> Unknown;
- retained Metadata but dropped Support -> Unknown;
- raw preliminary companions cannot substitute for retained companions;
- fact/catalog/fingerprint/authority digest mismatch -> Unknown/contract error;
- no public/serde/unchecked retained-input constructor exists; exactly one
  production retained-view implementation reads the current final canonical
  set, while malicious recording fakes exercise rejection paths; a raw-batch
  or second implementation fails product contracts;
- exhaustive base and present-extension direct matrices;
- ExtensionWithout without exact present Own/Adopted -> Unknown;
- same-name/same-artifact cross-source collision never merges;
- CFE Locked/read-only precedence;
- safe exact Absent -> ExtensionRequired advice with zero patch receipt/
  resolver/issuer/handler calls and no implicit borrow;
- exact equal Adopted + safe policy -> ExtensionOwned;
- Own/wrong Adopted/inconclusive -> exact blocker;
- unknown analysis support remains Unknown.

### 11.5 Live/render/assessment/transitional STOP

- no-follow Missing versus symlink/reparse/special/I/O distinction;
- live read max+1 and exact typed parser result;
- legacy info/assessment uses one same-snapshot neutral catalog build; a second
  metadata parser or catalog/read freshness mismatch is Indeterminate;
- every renderer state is explicit and no unknown renders writable/free/owned;
- deny/warn/off assessment matrix uses typed state, never display parsing;
- applied support.edit in every action/mode returns
  `support_edit_atomic_writer_required` and a panic writer observes zero calls;
- preexisting bytes, metadata, directory entries, and receipts remain unchanged;
- static scan rejects `fs::write`, direct temp/rename, regex/substrings,
  `canonicalize`, second semantic read, and legacy writer fallback from the
  support-edit apply path.

## 12. Implementation sequence

1. **Close dependency gate.** Record exact accepted Task4/5A Git OIDs and Task
   5B v7 contract/review/implementation authorities. Import v7 names/goldens.
2. **Compatibility corpus/provenance RED then GREEN.** Add canonical synthetic
   bytes and manifest with the epistemic limit.
3. **Parser RED then GREEN.** Land pure lexer/parser/types/spans/encoders only.
4. **Shared authority RED then GREEN.** Borrow exact once-built catalog and
   consume Task5B-owned query; add static no-adapter/no-local-encoder tests.
5. **Provider RED then GREEN.** Add bounded read, complete group mapping,
   planned-absence matrix, authority index, and group digest.
6. **Projection RED then GREEN.** Remove context-free projection; add retained
   join types and direct/CFE total matrices.
7. **Live/render/assessment RED then GREEN.** Migrate all consumers to typed
   read results and state residual non-serializable boundary honestly.
8. **Transitional STOP RED then GREEN.** Remove/unreach unsafe support.edit
   apply writer; keep typed preview only.
9. **Docs/spec/product contract.** Synchronize active spec and user-facing
   docs: provenance limits, unsupported shapes, shared query, no ownership from
   missing policy, no implicit borrow, 4096 bound, no local loss, transitional
   apply STOP, and downstream Task7 relationship.
10. **Verify/review/commit.** Run section 13, obtain independent no-P0/P1
    code/spec review, then commit with exact Git OID named
    `TASK5C_EVIDENCE_ACCEPTED_GIT_OID`.

No step changes the immutable historical Task5C v1 design/notes or treats the
combined v2 working history as accepted authority.

## 13. Verification gate

```text
cargo fmt --all -- --check
cargo test --locked -p unica-coder parent_configurations -- --nocapture
cargo test --locked -p unica-coder platform_configuration_catalog_shared -- --nocapture
cargo test --locked -p unica-coder support_state_query_v2 -- --nocapture
cargo test --locked -p unica-coder discovery::support -- --nocapture
cargo test --locked -p unica-coder support_projection_v2 -- --nocapture
cargo test --locked -p unica-coder support_renderer_v2 -- --nocapture
cargo test --locked -p unica-coder support_assessment_v2 -- --nocapture
cargo test --locked -p unica-coder application::discovery -- --nocapture
cargo test --locked -p unica-coder
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
python3.12 tests/ci/test_product_contracts.py
git diff --check
```

Focused filters must list the named RED/GREEN tests and nonzero test counts.
Product contracts prove no old support-edit writer remains reachable, Task7 is
not an upstream gate, Task6 has no Task5C prerequisite, Task7 names Evidence
only, and no Task8/9/10 contract is imported by this slice.

## 14. Hard STOP conditions

Stop rather than guess if work requires:

1. accepted Task5C Evidence before Task5B v7 or a Task7/8 reverse prerequisite;
2. local Task5C Support query types/encoder/digest instead of Task5B v7;
3. a bare name/path/artifact without full AtomicSourceIdentity and freshness;
4. a second MDClasses/flavor/UUID/membership parser or adapter chaining;
5. BOM-less/UTF-16/zero-multi-vendor/duplicate aggregation;
6. any format/flag/record shape outside the explicit v2 compatibility subset;
7. an unbounded optional support-file allocation/read;
8. extension per-object UUID guessing not owned by accepted Task5B v7;
9. ownership from ExtensionWithout, source kind, name, path, or generic absence;
10. projection from raw/dropped records or authority index alone;
11. cross-provider atomicity or Support-local lossy limit;
12. patch receipt/resolved plan/implicit borrow for ExtensionRequired;
13. nested destructive policy without real owner+child evidence/ADR;
14. any applied support.edit write, temp, rename, fallback, or writer claim;
15. unsupported/malformed/unavailable rendered or assessed as writable/free;
16. hash/freeze before all imported Task5B v7 names/tags/goldens and dependency
    values are exact and independently reviewed.
17. release/publication of the intermediate Evidence-only state as a completed
    Project Discovery/support-mutation feature.

Clearing a format or extension-UUID STOP requires the smallest redistributable
verified Designer export with donor manifest, or an explicit approved
compatibility contract with provenance and before/after product behavior, plus
a new versioned encoder when semantics change and fresh review. Synthetic
material may justify only the labelled compatibility branch, never Designer or
primary-export proof.

## 15. Freeze and self-audit

This conditional artifact is not hashable now. Freeze only after accepted Task
5B v7 exists:

1. read Task4/5A and accepted Task5B v7 fully;
2. replace every descriptive Task5B query type/reason with exact exported name;
3. copy exact domain/tags/bytes/goldens without alias or local re-encoding;
4. record exact 40-hex Git OIDs and 64-hex design/review hashes in disjoint
   fields and verify each object/file;
5. run stale/placeholder/cycle/tag/encoder/RED-owner scans;
6. finalize `.superpowers/sdd/task-5c-evidence-v2-self-audit.md` with the exact
   final design hash and a no-open-P0/P1 verdict or explicit STOP;
7. compute design and audit SHA-256 only after each file is closed;
8. obtain a separate independent review of those immutable exact files.

Mandatory stale search rejects:

```text
Task7 prerequisite for Task5C-Evidence
Task8 prerequisite for Task5C-Evidence
whole Task5C accepted before Task7
unica.support-query/v2 local encoder
bare ArtifactRef support subject
optional BOM
duplicate rules dedupe
ExtensionWithoutParentConfigurations -> ExtensionOwned
support provider-local result limit
implicit borrow
fs::write ParentConfigurations
```

## 16. Result

After this slice lands, ParentConfigurations evidence is strict, bounded,
source/freshness-bound, catalog-shared, ownership-aware, loss-safe, and visibly
unknown when authority is missing. The unsafe applied support writer is gone.
Task 7 may then consume the accepted complete-group/projection boundary without
being an upstream dependency. Durable mutation remains intentionally separate.
