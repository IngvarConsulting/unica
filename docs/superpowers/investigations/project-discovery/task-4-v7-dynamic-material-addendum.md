# Task 4 v7 addendum — capture-owned registered Form material

Owner document date: 2026-07-18.

This file has no mutable draft/candidate/accepted status field. The sole design
status authority is the external package ledger
`.superpowers/sdd/task-4-7-v7-design-package-acceptance.md`; absence or presence
of a ledger row never changes these owner bytes. This file contains no self-hash,
review hash, implementation OID slot, or implementation-status assertion.

This addendum is read with the historical Task 4 brief and report. It changes
only the snapshot authority needed for registered managed-Form material whose
path is registration-dependent. It does not re-accept, edit, or weaken the
existing Task 4 source-map, containment, EDT, budget, verified-read, or typed
error contracts.

This Task 4 slice writes only:

```text
.superpowers/sdd/task-4-v7-dynamic-material-addendum.md
.superpowers/sdd/task-4-v7-dynamic-material-self-audit.md
```

Design acceptance is one atomic four-document co-freeze, never a standalone
Task 4 acceptance:

```text
.superpowers/sdd/task-4-v7-dynamic-material-addendum.md
.superpowers/sdd/task-5b-v7-contract.md
.superpowers/sdd/task-6-v2-v7-addendum.md
.superpowers/sdd/task-7-v7-addendum.md
```

Every one of those four design files must have immutable candidate bytes and a
fresh compatible review before one ledger accepts the four-identity tuple in
one operation. Cross-review must follow the exact
`CompositeSnapshotIdV2` raw writer -> Task5B
`PlatformCatalogExecutionBindingV1` -> Task7 execution-header encoder chain;
Task4 exposes no parallel raw projection. A byte change to one document changes that file's mathematical
SHA but not the SHA of an unchanged peer. The new bytes do not inherit the old
package tuple, status labels, mechanical goldens, audits, or reviews; all four
documents must be resealed and reviewed as one new tuple. An external ledger
row for the former immutable bytes remains historically true only for its exact
former hashes and is never retroactively altered or applied to the new bytes.
An owner document is never standalone acceptance or production authority.
Production remains sequential after design co-freeze: the Task 4 successor
lands and is independently accepted before any Task 5B provider implementation
depends on it.

No sealed owner document embeds its own accepted SHA or a later implementation
OID. The external package acceptance ledger
`.superpowers/sdd/task-4-7-v7-design-package-acceptance.md` records the four
document hashes plus their audit/review hashes after co-freeze. A later
implementation report and implementation ledger record the accepted Task 4
production OID without changing these frozen design bytes.

## 1. Live root cause and decision

The live Task 4 `SourceManifest` contains one path map whose values are either a
present regular file or one of five fixed optional tombstones. Its v1 source
fingerprint covers only that map. An absent arbitrary
`<registered-form-stem>/Ext/Form.xml` or
`<registered-form-stem>/Ext/Form/Module.bsl` therefore has no
capture-owned key or negative authority. The raw verified reader rejects a path
outside the map, while the optional reader can authorize only the five closed
tags.

A later provider cannot safely repair this by appending suffixes or probing the
live filesystem. That would move registration semantics outside capture, omit
the negative fact from the source fingerprint, and allow a race-prone live
absence observation to masquerade as captured evidence.

The successor adds two immutable, fingerprinted catalogs to the manifest:

1. a captured registered-Form handle catalog, including descriptor leaf and
   neutral FormType witness authority; and
2. a registered-material expectation catalog containing exactly two
   source-local relationships per captured Form, `FormXml` and `FormModule`.

Each expectation has exactly one state:

- `NotApplicable`, with no expected key;
- `Missing`, with one opaque capture-owned expected key; or
- `Present`, with that key plus exact captured length and leaf fingerprint.

Only exact `Known(Managed)` FormType authority may produce `Missing` or
`Present`. Exact `Known(Ordinary)` and every typed `Inconclusive` authority
produce two keyless `NotApplicable` rows. Completeness is therefore
fingerprinted without inventing a managed path for a Form whose captured type
does not prove managed semantics.

Rejected alternatives:

1. a generic dynamic tombstone in the ordinary path map cannot represent
   pathless `NotApplicable` and still needs a relationship index;
2. an absence token outside the fingerprinted manifest can drift independently;
3. suffix formatting in Task 5B, Task 6, Task 7, or Task 8 repeats the authority
   defect instead of fixing it; and
4. eagerly reading all dynamic material while building a Task 5B context turns
   an immutable catalog projection into an unnecessary I/O pass.

## 2. Version registry, limits, and ownership

```text
SOURCE_SNAPSHOT_FINGERPRINT_ENCODER = "source-set-snapshot/v2"
SOURCE_FINGERPRINT_DOMAIN = "unica.source-set-snapshot.v2"
COMPOSITE_SNAPSHOT_FINGERPRINT_ENCODER = "source-composite/v2"
COMPOSITE_FINGERPRINT_DOMAIN = "unica.source-composite.v2"
REGISTERED_MATERIAL_EXPECTATION_CATALOG =
  "registered-material-expectations/v1"
REGISTERED_MATERIAL_PATH_POLICY = "registered-form-material-paths/v1"
ANALYSIS_BSL_MODULE_PROJECTION_POLICY =
  "analysis-bsl-manifest-module-projection/v1"
ANALYSIS_BSL_MODULE_PROJECTION_CONTRACT_VERSION: u16 = 1

MAX_SNAPSHOT_FILES = 200_000
MAX_SNAPSHOT_BYTES = 4 GiB
MAX_SNAPSHOT_TRAVERSAL_ENTRIES = 1_600_000
MAX_SNAPSHOT_TRAVERSAL_DEPTH = 64
MAX_SNAPSHOT_XML_BYTES = 64 MiB per material
MAX_SNAPSHOT_XML_DEPTH = 128
MAX_SNAPSHOT_XML_NODES = 1_000_000 per material
MAX_SNAPSHOT_ELAPSED = 120 s
MAX_SNAPSHOT_MANIFEST_KEY_BYTES = 4_096
MAX_REGISTERED_MATERIAL_EXPECTATIONS = 400_000
MAX_CAPTURED_ANALYSIS_BSL_SCAN_ITEMS = 400_000
SINGLE_SOURCE_EXPECTATION_FIXTURE_AT_FILE_LIMIT =
  2 * (MAX_SNAPSHOT_FILES - 2) = 399_996  // test fixture only
```

The 4,096-byte manifest-key bound is not a new v2 compatibility narrowing. Live
Task 4 already rejects `path.len() > 4096`; v2 promotes that existing byte
boundary into an explicit named contract and applies the same grammar to every
derived expected key. The v2 identity change is required by the new captured
handle/expectation bytes, not by a path-length change.

The 400,000 expectation cap is one independent global allocation/resource cap
for the complete analysis-plus-destinations capture. It is checked with exact
addition before allocation, decode, hashing, or provider I/O; 400,000 is
accepted and 400,001 is rejected. It must not be silently replaced by the
single-source arithmetic value 399,996. Live capture counts globally unique
present paths, while different non-identical source roots may be nested or
overlap; source-local Form handles and expectations can therefore sum beyond a
single-source file-count derivation even when physical present files dedupe.

Neutral `infrastructure::platform_xml` owns:

- `PlatformXmlSourceSpanV1`;
- the closed FormType authority/problem grammar;
- `PlatformRegisteredFormTypeCaptureV1`; and
- the sole bounded parser that selects that authority.

It imports neither Task 4 snapshot types nor Task 5B catalog/provider types.
Task 4 and Task 5B import the same neutral types directly; neither defines an
alias, duplicate enum, conversion copy, or callback into the other task.

Task 4 capture owns normalized manifest keys, registered owner/Form handle
construction, the sole managed-material path serializer, state classification,
catalog validation, source/composite v2 encoding, the contained absence
primitive, the verified registered-material reader, and the sole sealed raw
digest projection of `CompositeSnapshotIdV2`. Task 5B may borrow and project
this authority but must not reconstruct it. The raw composite projection is
callable only by Task 5B's
`PlatformCatalogExecutionBindingV1::write_identity_v1`; downstream tasks
receive that opaque 96-byte binding writer, never a composite byte accessor or
constructor for a manifest key, relationship, expected path, or snapshot ID.

The existing `OptionalMaterialTag` registry remains exactly five tags. Dynamic
registered material is not added to it and never passes through
`read_optional_verified(path)`.

## 3. Exact domain and capture-envelope grammar

All fields and constructors below are private unless an accessor is explicitly
shown. None of the capture or handle types is serde-enabled.

```text
SnapshotManifestKeyV1                // normalized contained UTF-8 key
SnapshotManifestKeyRefV1<'snapshot>  // opaque borrowed validated key
SnapshotManifestKeyProjectionV1      // opaque lossless owned key projection
RegisteredMaterialRelationshipRefV1<'snapshot>
RegisteredMaterialRelationshipProjectionV1
SourceFingerprintV1([u8; 32])        // exact sha256: transport grammar
SnapshotLeafFingerprintV1([u8; 32])  // exact captured leaf authority
CompositeSnapshotIdV2([u8; 32])      // exact composite sha256: authority

PlatformXmlSourceSpanV1 {
  start_byte: u32,
  end_byte_exclusive: u32,
}

PlatformRegisteredFormTypeAuthorityV1
  Known(Managed)                                  tag 1, value tag 1
  Known(Ordinary)                                 tag 1, value tag 2
  Inconclusive(PlatformRegisteredFormTypeProblemV1) tag 2

PlatformRegisteredFormTypeProblemV1
  Missing                tag 1
  Duplicate              tag 2
  WrongNamespace         tag 3
  MixedContent           tag 4
  UnsupportedOrOverLimit tag 5

PlatformRegisteredFormTypeCaptureV1 {
  authority: PlatformRegisteredFormTypeAuthorityV1,
  form_properties_span: PlatformXmlSourceSpanV1,
  form_type_spans: Box<[PlatformXmlSourceSpanV1]>,
}

impl PlatformXmlSourceSpanV1 {
  fn start_byte(&self) -> u32;
  fn end_byte_exclusive(&self) -> u32;
}

impl PlatformRegisteredFormTypeAuthorityV1 {
  fn stable_tag(&self) -> u16;
  fn known_value_tag(&self) -> Option<u16>;
  fn problem(&self)
    -> Option<&PlatformRegisteredFormTypeProblemV1>;
}

impl PlatformRegisteredFormTypeProblemV1 {
  fn stable_tag(&self) -> u16;
}

impl PlatformRegisteredFormTypeCaptureV1 {
  fn authority(&self)
    -> &PlatformRegisteredFormTypeAuthorityV1;
  fn form_properties_span(&self)
    -> &PlatformXmlSourceSpanV1;
  fn form_type_spans(&self)
    -> &[PlatformXmlSourceSpanV1];
}

impl CompositeSnapshotIdV2 {
  // Sealed owner projection: appends exactly the private raw 32-byte SHA-256
  // value and returns no slice, array, digest wrapper or intermediate buffer.
  // The only production caller is Task 5B's
  // PlatformCatalogExecutionBindingV1::write_identity_v1.
  pub(crate) fn write_platform_catalog_execution_digest_v1(
    &self,
    out: &mut Vec<u8>,
  );
}

PlatformXmlSourceSpanV1: Copy + Clone + Eq + Ord + Hash
PlatformRegisteredFormTypeAuthorityV1: Clone + Eq + Ord + Hash
PlatformRegisteredFormTypeProblemV1: Clone + Eq + Ord + Hash
PlatformRegisteredFormTypeCaptureV1: Clone + Eq
SourceFingerprintV1: Clone + Eq + Ord + Hash
SnapshotLeafFingerprintV1: Clone + Eq + Ord + Hash
CompositeSnapshotIdV2: Clone + Eq + Ord + Hash
SnapshotManifestKeyProjectionV1: Clone + Eq + Ord + Hash
RegisteredMaterialRelationshipProjectionV1: Clone + Eq + Ord + Hash
VerifiedBslSourceLocationV1: Clone + Eq + Ord + Hash
VerifiedBslCacheLocatorV1: Clone + Eq + Hash

impl SnapshotManifestKeyRefV1<'snapshot> {
  fn to_projection(self) -> SnapshotManifestKeyProjectionV1;
}

impl SnapshotManifestKeyProjectionV1 {
  fn encode_identity_v1(
    &self,
    encoder: &mut CanonicalIdentityEncoderV1,
  ) -> Result<(), CanonicalEncodingErrorV1>;

  // Purpose-specific sealed projection for a catalog `string` field. This is
  // not an alternate identity encoder and exposes no spelling to the caller.
  fn encode_catalog_string_u32_v1(
    &self,
    encoder: &mut CanonicalIdentityEncoderV1,
  ) -> Result<(), CanonicalEncodingErrorV1>;
}

impl RegisteredMaterialRelationshipRefV1<'snapshot> {
  fn to_projection(self)
    -> RegisteredMaterialRelationshipProjectionV1;
}

impl RegisteredMaterialRelationshipProjectionV1 {
  fn encode_source_local_identity_v1(
    &self,
    enclosing_source: &ResolvedSourceSet,
    enclosing_source_fingerprint: &SourceFingerprintV1,
    encoder: &mut CanonicalIdentityEncoderV1,
  ) -> Result<(), CanonicalEncodingErrorV1>;
}

CapturedRegisteredFormHandleKeyV1 {
  owner_descriptor_manifest_key: SnapshotManifestKeyV1,
  form_descriptor_manifest_key: SnapshotManifestKeyV1,
}

CapturedRegisteredFormV1 {
  handle_key: CapturedRegisteredFormHandleKeyV1,
  form_descriptor_byte_length: u64,
  form_descriptor_content_fingerprint: SnapshotLeafFingerprintV1,
  form_root_span: PlatformXmlSourceSpanV1,
  form_type_capture: PlatformRegisteredFormTypeCaptureV1,
}

RegisteredMaterialKindV1
  FormXml    tag 1
  FormModule tag 2

RegisteredMaterialRelationshipKeyV1 {
  owner_descriptor_manifest_key: SnapshotManifestKeyV1,
  form_descriptor_manifest_key: SnapshotManifestKeyV1,
  kind: RegisteredMaterialKindV1,
}

RegisteredMaterialExpectationV1 {
  relationship: RegisteredMaterialRelationshipKeyV1,
  state:
    NotApplicable tag 1
    | Missing tag 2 {
        expected_manifest_key: SnapshotManifestKeyV1,
      }
    | Present tag 3 {
        expected_manifest_key: SnapshotManifestKeyV1,
        byte_length: u64,
        content_fingerprint: SnapshotLeafFingerprintV1,
      },
}

ManifestEntryV2
  Present(MaterialFileV2)                    tag 1
  AbsentOptional(the existing five-tag enum) tag 2

MaterialFileV2 {
  byte_length: u64,
  content_fingerprint: SnapshotLeafFingerprintV1,
}

SourceManifestV2 {
  entries:
    BTreeMap<SnapshotManifestKeyV1, ManifestEntryV2>,
  captured_registered_forms:
    BTreeMap<CapturedRegisteredFormHandleKeyV1,
             CapturedRegisteredFormV1>,
  registered_material_expectations:
    BTreeMap<RegisteredMaterialRelationshipKeyV1,
             RegisteredMaterialExpectationV1>,
}

// Entirely private Task4-owned derived backing. Internal Clone/Eq supports
// construction validation only; no type/clone/serde capability is visible to
// Task5B/Task6.
CapturedAnalysisBslMaterialIndexV1 {
  entries: Vec<CapturedAnalysisBslMaterialV1>,
}

CapturedAnalysisBslMaterialV1 {
  authority: CapturedAnalysisBslMaterialAuthorityV1,
  module: Option<ArtifactRef>,
  admission_byte_length: Option<u64>,
  diagnostic_location_authority: CapturedBslLocationAuthorityV1,
  order_key: CapturedAnalysisBslMaterialOrderKeyV1,
}

CapturedAnalysisBslMaterialAuthorityV1
  Ordinary {
    manifest_key: SnapshotManifestKeyV1,
  }
  | RegisteredFormModule {
      relationship_key: RegisteredMaterialRelationshipKeyV1,
    }

CapturedBslLocationAuthorityV1
  PresentManifestLeaf {
    manifest_key: SnapshotManifestKeyV1,
  }
  | MissingRegisteredLeaf {
      relationship_key: RegisteredMaterialRelationshipKeyV1,
      expected_manifest_key: SnapshotManifestKeyV1,
    }
  | NotApplicableRegisteredLeaf {
      relationship_key: RegisteredMaterialRelationshipKeyV1,
      form_descriptor_manifest_key: SnapshotManifestKeyV1,
    }

CapturedAnalysisBslMaterialOrderKeyV1
  Addressable {
    anchor_manifest_key: SnapshotManifestKeyV1,
    // None for Ordinary; Some(exact relationship) for RegisteredFormModule.
    relationship_tie_break: Option<RegisteredMaterialRelationshipKeyV1>,
  }
  | AfterCapturedFormDescriptor {
      anchor_manifest_key: SnapshotManifestKeyV1,
      relationship_tie_break: RegisteredMaterialRelationshipKeyV1,
    }

CapturedAnalysisBslMaterialIndexV1: Clone + Eq
CapturedAnalysisBslMaterialV1: Clone + Eq
CapturedAnalysisBslMaterialAuthorityV1: Clone + Eq
CapturedBslLocationAuthorityV1: Clone + Eq
CapturedAnalysisBslMaterialOrderKeyV1: Clone + Eq + Ord

SourceSetSnapshotV2 {
  // one atomic source only
  resolved_source_set: ResolvedSourceSet,
  source_fingerprint: SourceFingerprintV1,
  manifest: SourceManifestV2,
  // Private checked derivative of `manifest`; deliberately excluded from all
  // source/composite fingerprint payloads.
  analysis_bsl_material_index: CapturedAnalysisBslMaterialIndexV1,
}

SourceSnapshotV2 {
  // one composite capture; fields and constructor are private
  analysis: SourceSetSnapshotV2,
  destinations: Vec<SourceSetSnapshotV2>,
  composite_snapshot_id: CompositeSnapshotIdV2,
  diagnostic_workspace_epoch: u64,
}

impl SourceSnapshotV2 {
  fn analysis_snapshot(&self) -> &SourceSetSnapshotV2;
  fn destination_snapshots(&self) -> &[SourceSetSnapshotV2];
  fn composite_snapshot_id(&self) -> &CompositeSnapshotIdV2;
  fn diagnostic_workspace_epoch(&self) -> u64;
}
```

`SourceManifestV2::registered_material_expectations` is the private ordered
relationship index and authoritative snapshot state, not a post-fingerprint
cache. Task 4 constructs its complete `BTreeMap`, validates key/value equality,
two-row totality, captured-Form backreferences, state matrix and expected-entry
consistency before source fingerprint construction, and the source encoder binds
that same canonical map. `SourceSetSnapshotV2` retains this accepted manifest
authority unchanged; projection resolution uses this index directly.

`SourceSetSnapshotV2` is therefore always **atomic**: it owns exactly one
`ResolvedSourceSet`, one source fingerprint and one `SourceManifestV2`. It is
never an alias for an analysis-plus-destinations capture. The separate
`SourceSnapshotV2` is the sole composite authority. Its private constructor
validates the analysis snapshot, validates every destination snapshot, sorts
destinations by complete `ResolvedSourceSetIdentityBytesV1`, rejects duplicate
or conflicting complete source identities, rejects an analysis identity reused
as a destination, and computes `CompositeSnapshotIdV2` from the exact section-10
v2 composite encoder. `destination_snapshots()` returns that canonical unique
order verbatim.

`diagnostic_workspace_epoch` is retained only for diagnostics and receipt
freshness reporting. It is not an input to `CompositeSnapshotIdV2`, either
source fingerprint, a catalog/query/group/evidence identity or semantic
equality. Rebuilding a semantically equal composite under another diagnostic
epoch produces the same `CompositeSnapshotIdV2`; changing any role, complete
source identity, source count/order after canonicalization, or v2 source
fingerprint changes it. V1 `SourceSnapshot`/`SourceSetSnapshot` and these v2
types are not mutually accepted or bridged by aliases.

`SnapshotManifestKeyV1` accepts exactly one nonempty normalized
workspace-relative slash path of at most 4,096 UTF-8 bytes. It rejects absolute,
drive, UNC, backslash, colon, control, empty, `.` and `..` components. There is
no `From<String>`, public join, raw digest/path constructor, ArtifactRef
constructor, display-name constructor, serde ingress, public `as_str`, byte
slice, `Display`, or path accessor. Only Task 4's contained reader/encoder may
borrow the private spelling for filesystem access and diagnostics.

Task 4 alone constructs `SnapshotManifestKeyRefV1<'snapshot>` from an accepted
manifest capture view or expectation state. The ref is `Copy` but opaque. Its
sole ownership transition creates `SnapshotManifestKeyProjectionV1`, an exact
lossless private clone of the already validated key. The projection is
`Clone + Eq + Ord + Hash`, has no raw accessor or constructor, and is the only
key value a downstream private wrapper may retain.

Task 4 likewise constructs `RegisteredMaterialRelationshipRefV1<'snapshot>`
only from one material expectation handle. The ref privately binds the complete
semantic source identity, source fingerprint, manifest and exact owner/Form/
kind relationship; none is individually exposed. Its sole ownership transition
creates `RegisteredMaterialRelationshipProjectionV1`, an exact lossless
`Clone + Eq + Ord + Hash` projection with private fields and no constructor.
The projection is unforgeable outside Task 4 and privately retains the accepted
source identity, source fingerprint, manifest/catalog relationship and exact
expectation-state witness required to resolve the same row in a semantically
equal snapshot. It exposes no source, state, tuple, key, string, bytes, path or
component accessor.
Its Task4-owned encoder first requires the enclosing catalog source identity and
fingerprint to equal its private semantic binding, then appends only the
existing source-local `owner-key || Form-key || u16be(kind)` bytes. Source and
fingerprint remain exactly once in the enclosing source/catalog header; they are
not duplicated in the nested row. The encoder returns no tuple, key, string,
bytes, path or component. This separation preserves all published source and
downstream catalog/query goldens while preventing cross-source replay.

Task 4 owns both sealed projections of the key spelling. `encode_identity_v1`
retains the existing manifest/source-identity field and appends exactly
`u64be(UTF-8 byte length) || UTF-8`. The distinct purpose-specific
`encode_catalog_string_u32_v1` appends exactly
`u32be(UTF-8 byte length) || UTF-8`, using a checked `u32` conversion before it
appends any byte. It exists only so a downstream catalog `string` field can
consume the already validated opaque projection without recovering the raw key
or locally reframing it; it is not used by a source fingerprint, relationship
identity, manifest identity or any existing u64-framed transcript. Neither
method calls the other, truncates a u64 prefix, returns a temporary byte vector,
or exposes a string, bytes, path, iterator, length, or component.

For an exact valid key of `N` UTF-8 bytes, the catalog projection contributes
exactly `4 + N` bytes and begins with `u32be(N)`; an otherwise equal valid key
of `N + 1` UTF-8 bytes begins with `u32be(N + 1)`. The count is bytes, not
Unicode scalars. The live maximum key contributes
`00 00 10 00 || 4,096 key bytes`; a 4,097-byte key is rejected by the common
key constructor before either projection can exist. The corresponding identity
projection of the 4,096-byte key still begins with the distinct eight-byte
`u64be(4,096)` prefix. Task 5B may pass its complete semantic encoder to either
listed method for the field whose grammar selects it, but cannot reimplement
framing or extract the key. The neutral encoder imports neither Task 4 nor Task
5B, so the dependency remains `Task5B -> Task4 -> neutral`, never
`Task4 -> Task5B`.

The neutral span/FormType types above are immutable and non-serde, with private
construction. `PlatformXmlSourceSpanV1` is `Copy + Clone + Eq + Ord + Hash`;
`PlatformRegisteredFormTypeAuthorityV1` and its problem type are
`Clone + Eq + Ord + Hash`; `PlatformRegisteredFormTypeCaptureV1` is
`Clone + Eq` and its span slice clones losslessly into the same neutral span
type. Their listed methods are the complete read-only API: callers can observe
numeric span boundaries, stable closed tags, problem authority, and the borrowed
canonical span slice, and may retain exact clones in catalog/witness values,
but cannot construct, convert-copy, normalize, reinterpret or repair authority.

`SourceFingerprintV1` and `SnapshotLeafFingerprintV1` remain distinct smart
types even though both render as exact lowercase `sha256:<64 hex>`. There is no
cross-type conversion. Both are immutable `Clone + Eq + Ord + Hash` so Task 5B
can retain exact authority; constructors, raw digest access and cross-conversion
remain private. Leaf fingerprints come only from captured bytes; the source
fingerprint is computed, never caller-supplied. The `V1` suffix names the stable
transport grammar, while the source hash domain is v2.

Every captured Form constructor/decoder requires its Form descriptor key to
resolve to an ordinary `Present` manifest entry with exactly equal
`form_descriptor_byte_length` and leaf fingerprint. Missing, AbsentOptional,
non-regular, unequal, or orphan descriptors reject the whole manifest.

Spans are zero-based, half-open, nonempty, UTF-8/node-boundary ranges. All span
ends are at most `form_descriptor_byte_length`; `form_root_span` contains
`form_properties_span`, which contains each direct FormType observation span.
`form_type_spans` is canonical sorted-unique. Root/properties/type spans and the
exact authority are fingerprinted, not diagnostic-only fields.

### 3.1 Closed manifest-BSL-to-module projection v1

The historical Task 6 text said to strip `relative_root` and map a
`<RegisteredDir>`/`<ModuleKind>` shape, but it never closed either registry or
specified who could turn a manifest row into `Option<ArtifactRef>`. Taken
literally, different consumers could accept different directory case, silently
add a newly registered kind, or format a registered FormModule from public
names. That is the same authority defect this addendum removes. The historical
Task 6 section-10 table is therefore descriptive history only; this section is
the sole normative projection grammar.

`ANALYSIS_BSL_MODULE_PROJECTION_POLICY =
"analysis-bsl-manifest-module-projection/v1"` and
`ANALYSIS_BSL_MODULE_PROJECTION_CONTRACT_VERSION = 1` are the Task4-owned
stable registry identity/version. They are registered as a subordinate contract of
`SOURCE_SNAPSHOT_FINGERPRINT_ENCODER = "source-set-snapshot/v2"`; it is not an
extra field appended to the already frozen v2 fingerprint payload. Changing a
directory row, module-kind row, path shape, identifier rule or supported/
unsupported decision requires a new projection-contract version **and** a new
source fingerprint encoder/domain version. Otherwise equal v2 snapshot bytes
could acquire a different typed module surface while retaining one composite
identity. Task5B and Task6 neither carry nor branch on this version: their
source fingerprint already binds the v2 contract, and a compile/static gate
requires them to import only the Task4 projection result. The current value
therefore changes none of the section-10 goldens.

The Task4 implementation owns exactly two private constructors, called only by
`CapturedAnalysisBslMaterialIndexV1::derive_checked` and its exact rebuild:

```text
project_ordinary_present_bsl_module_v1(
  resolved_source_set,
  accepted_manifest,
  manifest_key,
) -> Result<Option<ArtifactRef>, SnapshotCaptureError>

project_registered_form_module_v1(
  resolved_source_set,
  accepted_manifest,
  captured_form,
  exact_form_module_relationship,
) -> Result<ArtifactRef, SnapshotCaptureError>
```

Both functions borrow the already accepted `SourceManifestV2`; a manifest key
alone is deliberately insufficient authority. Neither function, its
source-local spelling view nor the registry below is exported. There is no
constructor accepting a caller path, filename, directory, owner/Form name or
preformatted canonical ref. `ArtifactRef::parse(Module, exact_ref)` is the
final typed validation step; a crate-visible forged struct is not accepted.
Task5B receives only the already typed `module()` projection and Task6 cannot
name the capture handle. No Task5B/6/7/8 production module may strip a source
root, recognize a BSL filename, map a metadata directory, append a module kind
or format a FormModule ref.

`SourceManifestV2` has one private registration-plan constructor, no raw
`BTreeMap` constructor, serde ingress or field mutation. It accepts only the
initial/final-equal Platform XML capture plan produced by the shared exact
`Configuration.xml/Configuration/ChildObjects` parser. For every registered
root the plan binds exact `(canonical kind, exact directory, exact name)`,
requires one exact Present `D/<N>.xml`, parses that descriptor as the same kind
and Name, and admits only that registered root's exact contained subtree.
Conversely, it rejects any owner-shaped descriptor or descendant entry in the
candidate plan without that exact registration row. The CommonModule row uses
the same rule with `CommonModule`, `CommonModules`, and
`CommonModules/<N>.xml`. Thus membership in an accepted manifest is a closed
registration-aware fact, not the result of scanning a directory.

The exact `Configuration.xml` used to produce that plan is itself one ordinary
Present manifest entry, and its captured length/content fingerprint is in the
source-v2 payload. Each required owner descriptor entry is fingerprinted the
same way. Initial and final plan comparison reparses the exact corresponding
stable bytes and requires identical registration, descriptor kind/Name and
selected descendant sets. Registration authority is therefore bound by
already encoded manifest leaves rather than by an unencoded side table.

The private manifest validator rechecks the complete registration-to-descriptor
and registered-descendant closure before source fingerprinting. The derived
index then performs an exact indexed lookup of the corresponding Present owner
descriptor for every otherwise supported ordinary module row and verifies the
closed registration binding through that manifest API. Missing, optional,
wrong-kind, wrong-name, unregistered or unequal descriptor authority rejects
the snapshot; it never returns `Some` or downgrades the row to `None`. This is
why the private projector needs the complete accepted manifest. A filesystem
decoy `D/<N>.xml` plus a module subtree that is absent from ChildObjects never
enters the manifest or scan index, while a malformed internal candidate plan
that attempts to insert it is rejected before fingerprint construction.

```text
RegisteredModuleOwnerProjectionRowV1 {
  exact_directory: &'static str,
  exact_kind: &'static str,
}

CapturedRegisteredRootDescriptorRefV1<'manifest> {
  manifest_key: &'manifest SnapshotManifestKeyV1,
  material: &'manifest MaterialFileV2,
}

impl SourceManifestV2 {
  // Task4-private; owner_row/name come only from the closed projector match,
  // never from a downstream caller.
  fn registered_root_descriptor_for_module_projection_v1(
    &self,
    resolved_source_set: &ResolvedSourceSet,
    owner_row: &'static RegisteredModuleOwnerProjectionRowV1,
    exact_name: &str,
  ) -> Result<CapturedRegisteredRootDescriptorRefV1<'_>,
              SnapshotCaptureError>;
}
```

Both supporting types and all fields are private, non-serde and non-Clone;
`owner_row` can be borrowed only from the exact static registry below. The
returned ref merely borrows the exact Present descriptor entry already encoded
in `entries` and has no accessor. There is no second registration field or
hidden semantic payload excluded from the source fingerprint. Instead, the
private manifest type's construction invariant makes that descriptor entry
admissible only through the matching registration plan, and the method
revalidates the exact source-local descriptor shape, Present state and kind/
name binding before lending the ref. The encoded Configuration/descriptor
leaves bind the bytes from which that invariant was constructed. Because no
alternate manifest constructor exists, an equal independently recaptured
manifest has the same proof, while a bare map of equal-looking decoy files is
not constructible.

#### Source-local path and identifier rules

Manifest keys remain normalized workspace-relative keys. Projection first
derives one private source-local component vector as follows:

1. validate the complete `ResolvedSourceSet`, including its normalized
   `relative_root`;
2. if `relative_root == "."`, retain every manifest-key component unchanged;
3. otherwise require the manifest key to begin with all exact UTF-8
   `relative_root` components plus a component boundary, then remove precisely
   that prefix; and
4. require at least one remaining component.

A key outside that exact source-root prefix is a snapshot invariant failure,
not an unsupported module. Prefix matching is byte-exact and case-sensitive;
there is no platform-path case lookup, separator conversion, NFC/NFD
normalization, percent decoding, locale mapping or lossy conversion. Slash is
the only separator because `SnapshotManifestKeyV1` already rejected every
other path spelling.

`<N>` and `<F>` below each mean exactly one retained UTF-8 component. A
projectable identifier component contains 1..=128 Unicode scalar values, and
every scalar satisfies the same Rust `char::is_alphanumeric` or `_` predicate
used by the domain `ArtifactRef` parser. The exact captured UTF-8 spelling is
copied into the canonical ref; it is never lowercased or normalized for output.
The enclosing `ArtifactRef` must remain at most 1,024 UTF-8 bytes and pass its
complete kind/shape validation. Task4 imports that one parser and its
domain-owned Unicode-17 build gate; it does not declare a second identifier or
Unicode registry. Thus `İ` remains exact `İ` in output even though semantic
`ArtifactRef` equality uses the shared potentially expanding Unicode-lowercase
identity.

Ordinary BSL enumeration still recognizes only a final extension equal to
`.bsl` under ASCII case-insensitive comparison, so every captured BSL file is
visible. Projection to `Some` is narrower: every literal directory and filename
in the supported shapes below is exact ASCII case-sensitive. For example,
`CommonModules/X/Ext/module.bsl` remains one visible ordinary BSL item but maps
to `None`; it is never repaired to `Module.bsl`.

#### Exhaustive directory and module registries

The CommonModule family is exactly:

```text
CommonModules/<N>/Ext/Module.bsl
  -> ArtifactRef(Module, "CommonModule." || <N>)
```

This mapping additionally requires the same accepted registration plan's exact
Present `CommonModules/<N>.xml` descriptor, parsed as the registered
`CommonModule` with exact Name `<N>`. `CommonModules` is not an owner row and no
other filename below it is accepted by v1. The owner family is the Cartesian
product of every row in this exact ordered table and every exact module-kind
row that follows it:

| exact directory | exact canonical owner kind |
| --- | --- |
| `Languages` | `Language` |
| `Subsystems` | `Subsystem` |
| `StyleItems` | `StyleItem` |
| `Styles` | `Style` |
| `CommonPictures` | `CommonPicture` |
| `SessionParameters` | `SessionParameter` |
| `Roles` | `Role` |
| `CommonTemplates` | `CommonTemplate` |
| `FilterCriteria` | `FilterCriterion` |
| `Bots` | `Bot` |
| `CommonAttributes` | `CommonAttribute` |
| `ExchangePlans` | `ExchangePlan` |
| `XDTOPackages` | `XDTOPackage` |
| `WebServices` | `WebService` |
| `HTTPServices` | `HTTPService` |
| `WSReferences` | `WSReference` |
| `EventSubscriptions` | `EventSubscription` |
| `ScheduledJobs` | `ScheduledJob` |
| `SettingsStorages` | `SettingsStorage` |
| `FunctionalOptions` | `FunctionalOption` |
| `FunctionalOptionsParameters` | `FunctionalOptionsParameter` |
| `DefinedTypes` | `DefinedType` |
| `CommonCommands` | `CommonCommand` |
| `CommandGroups` | `CommandGroup` |
| `Constants` | `Constant` |
| `CommonForms` | `CommonForm` |
| `Catalogs` | `Catalog` |
| `Documents` | `Document` |
| `DocumentNumerators` | `DocumentNumerator` |
| `Sequences` | `Sequence` |
| `DocumentJournals` | `DocumentJournal` |
| `Enums` | `Enum` |
| `Reports` | `Report` |
| `DataProcessors` | `DataProcessor` |
| `InformationRegisters` | `InformationRegister` |
| `AccumulationRegisters` | `AccumulationRegister` |
| `ChartsOfCharacteristicTypes` | `ChartOfCharacteristicTypes` |
| `ChartsOfAccounts` | `ChartOfAccounts` |
| `AccountingRegisters` | `AccountingRegister` |
| `ChartsOfCalculationTypes` | `ChartOfCalculationTypes` |
| `CalculationRegisters` | `CalculationRegister` |
| `BusinessProcesses` | `BusinessProcess` |
| `Tasks` | `Task` |
| `IntegrationServices` | `IntegrationService` |

The six exact ordered module-kind rows are:

```text
Module
ObjectModule
ManagerModule
RecordSetModule
ValueManagerModule
CommandModule
```

For each `(D, K)` directory/owner row and each `M` module-kind row, the sole
ordinary owner mapping is:

```text
D/<N>/Ext/M.bsl
  -> ArtifactRef(Module, K || "." || <N> || "." || M)
```

It additionally requires the same accepted registration plan's exact Present
`D/<N>.xml` descriptor, parsed as canonical kind `K` with exact Name `<N>`.
Path shape plus descriptor existence is still insufficient when the exact
ChildObjects registration row is absent.

The 44 owner rows are the exact current shared metadata-kind registry with its
`CommonModule -> CommonModules` row removed. A permanent registry test requires
that one-to-one correspondence, exact spelling and exact order. The special
CommonModule row must equal the removed shared row, and the six module-kind
rows must equal the shared `MODULE_KIND_TAGS` registry in exact order. This is
not an open `metadata_kind_by_directory` call: its live
ASCII-case-insensitive lookup would contradict the exact-case grammar above.
Adding or renaming either shared registry makes the gate fail until this closed
policy and its version/domain are explicitly reviewed; it never auto-enlarges
v1.

#### Registered FormModule replacement

The registered-Form family is not recognized from a path-shaped ordinary row.
It exists only when the accepted Task4 capture contains one exact
`CapturedRegisteredFormV1` and its exact `FormModule` relationship. The handle
must prove all of these source-local descriptor shapes under one owner-registry
row `(D, K)`:

```text
owner descriptor = D/<N>.xml
Form descriptor  = D/<N>/Forms/<F>.xml
FormModule Present/Missing expected key, when a key exists =
  D/<N>/Forms/<F>/Ext/Form/Module.bsl
typed projection =
  ArtifactRef(Module, K || "." || <N> || ".Form." || <F> || ".FormModule")
```

All fixed tokens and terminal `.xml` spellings above are exact ASCII
case-sensitive, and `<N>`/`<F>` obey the identifier rules above. The owner key,
Form key and relationship must share the same exact source, prefix, `D`, `<N>`
and `<F>` components and must already have passed registered owner/Form
membership capture. The accepted manifest must also return the exact Present
`D/<N>.xml` descriptor through the same closed registration binding used by an
ordinary owner module. A structural disagreement, unregistered owner/Form,
unsupported directory, invalid identifier, wrong relationship kind or
`ArtifactRef` validation failure rejects the whole snapshot. A registered
FormModule never degrades to `module=None`.

For Present, the relationship's expected key must equal the one ordinary
Present manifest key byte-for-byte and the registered candidate replaces that
ordinary candidate in the same slot. Missing derives the same typed ref from
the retained descriptor/relationship authority despite absent bytes.
NotApplicable has no expected path and derives the typed ref only from the
captured owner/Form descriptor keys; it never manufactures a managed path.
An ordinary file that merely resembles
`D/<N>/Forms/<F>/Ext/Form/Module.bsl` but has no exact accepted relationship is
not promoted. Registration-aware capture normally excludes such a decoy; if an
ordinary Present row of that spelling is otherwise in an accepted manifest, it
is an unsupported ordinary BSL item with `module=None`.

#### Unsupported and ambiguity semantics

Every ordinary Present BSL row that matches none of the CommonModule or owner
families exactly remains exactly one scan item with `module=None`, its captured
length and its opaque diagnostic location. This includes source-root
application/session/external-connection modules, direct
`CommonForms/<N>/Ext/Form/Module.bsl`, nested
`.../Commands/<C>/Ext/{Module,CommandModule}.bsl`, unknown/future directories,
extra/missing components, nonexact token case, invalid identifier components,
and an unclaimed Form-shaped path. Unsupported is observable to Task6 only as
the typed `None` classification that produces
`unsupported_bsl_module_identity`; no consumer may silently skip it or attempt
a second mapping. A non-BSL manifest row is not a scan item at all. A registered
FormModule invariant failure is an error, never `None`.

Before accepting the derived index, Task4 applies its component-wise shared
Unicode-lowercase alias key to every ordinary Present BSL manifest key and to
every FormModule relationship's owner descriptor, Form descriptor and optional
expected key, and applies
the existing semantic `ArtifactRef` equality to every projected `Some` value.
Different exact spellings that are alias-equal, or two different rows that
project to one semantic module, reject the complete snapshot in either input
order. This includes a supported exact path paired with a differently cased
unsupported spelling; the latter cannot evade collision rejection by mapping
to `None`. There is no first-wins, host-filesystem-dependent choice or
deduplication. The one exception is the byte-identical Present ordinary row
claimed by its exact registered FormModule relationship: it is not two inputs
and is replaced once as specified above.

#### Mechanical registry and exhaustive projection goldens

The canonical registry transcript is UTF-8, no BOM, one line per entry, exact
`|` separators, LF after every line including the last, and contains: version
line; CommonModule line; the 44 owner rows above in table order; the six module
kind rows above in order; and the registered-Form line. Its exact line grammar
is:

```text
version|1
common|CommonModules|CommonModule|Module.bsl
owner|<exact directory>|<exact canonical owner kind>
module-kind|<exact module kind>
registered-form|Forms|Ext/Form/Module.bsl|FormModule
```

For the exact closed rows above the transcript has 53 lines and 1,780 bytes:

```text
SHA-256 = 79630b3f2826f39ab69823a9cb80d250f0802dbc41f659f2b36b5a3a4379d583
```

The exhaustive projection golden uses source-local paths after exact root
stripping. Every line is evaluated with its matching exact ChildObjects
registration, valid exact Present owner descriptor, and admitted subtree; each
registered-form line additionally has the exact nested Form registration,
descriptor and FormModule relationship. It emits, in order: the one
CommonModule line; for every owner row in table order, all six module-kind rows
in module-kind order; then for every owner row the registered-Form line. `Σ`
and `Main_1` are literal UTF-8 identifier components. Each line has exact `|`
separators and a final LF:

```text
common|CommonModules/Σ/Ext/Module.bsl|CommonModule.Σ
owner|D/Σ/Ext/M.bsl|K.Σ.M
registered-form|D/Σ/Forms/Main_1/Ext/Form/Module.bsl|K.Σ.Form.Main_1.FormModule
```

Here `D`, `K` and `M` are substituted without angle brackets from the exact
ordered registries; the registered-form lines assume the exact captured
relationship. The resulting 309-line matrix has 24,842 UTF-8 bytes:

```text
SHA-256 = 816d909df4a5362c632d20b36ed8d65ee382322e207ec73099d87ffdd0acf21a
```

Additional direct goldens require `main/CommonModules/İ/Ext/Module.bsl` under
`relative_root="main"` and `CommonModules/İ/Ext/Module.bsl` under
`relative_root="."` to both preserve exact `CommonModule.İ`, with matching
registration and descriptor authority in each fixture. Permanent
negative fixtures cover every unsupported family listed above plus both orders
of `Catalogs/X/Ext/ObjectModule.bsl` versus
`Catalogs/x/Ext/ObjectModule.bsl`, and `CommonModules/X/Ext/Module.bsl` versus
`commonmodules/X/Ext/Module.bsl`; each pair rejects the whole snapshot rather
than selecting one projection. Separate decoy fixtures put an exact
`CommonModules/Decoy.xml` plus module subtree and an exact
`Catalogs/Decoy.xml` plus ObjectModule subtree on disk without corresponding
ChildObjects rows; neither descriptor nor module may enter the manifest or
produce a projection. Internal malformed-plan fixtures attempt the same rows
directly and must fail manifest construction. Positive controls add the exact
`CommonModule/Decoy` and `Catalog/Decoy` ChildObjects rows plus matching parsed
descriptor kind/Name; only then do the same module leaves enter the manifest and
project to `CommonModule.Decoy` and `Catalog.Decoy.ObjectModule` respectively.

## 4. Totality, matrix, and self-resolving borrowed handles

For every `captured_registered_forms` entry, the expectation map contains
exactly these two keys and no others:

```text
(same owner key, same Form key, FormXml)
(same owner key, same Form key, FormModule)
```

Every expectation value's embedded relationship equals its map key. Every row
points back to exactly one captured Form. Orphan/duplicate handles, orphan rows,
unequal owner/Form keys, a missing/duplicate/reversed kind, or a third kind
rejects the whole manifest before fingerprint construction.

| Captured FormType authority | `FormXml` | `FormModule` |
| --- | --- | --- |
| exact `Known(Managed)` | independently `Missing` or `Present` | independently `Missing` or `Present` |
| exact `Known(Ordinary)` | `NotApplicable` | `NotApplicable` |
| any `Inconclusive(problem)` | `NotApplicable` | `NotApplicable` |

`NotApplicable` has no expected key, length, fingerprint, absent-component
index, or hidden path. Invoking the path serializer, absence verifier, byte
reader, or parser for that slot is a test failure.

The public view is self-resolving and borrow-only:

```text
RegisteredFormCaptureHandleV1<'snapshot> {
  // all fields private and Task4-owned; Task5B imports this type verbatim
  resolved_source_set: &'snapshot ResolvedSourceSet,
  source_fingerprint: &'snapshot SourceFingerprintV1,
  manifest: &'snapshot SourceManifestV2,
  captured: &'snapshot CapturedRegisteredFormV1,

  fn view(&self) -> CapturedRegisteredFormViewV1<'snapshot>;
  fn material(
    &self,
    kind: RegisteredMaterialKindV1,
  ) -> RegisteredMaterialExpectationHandleV1<'snapshot>;
}

CapturedRegisteredFormViewV1<'snapshot> {
  // private borrow; no constructor/serde/raw representation
  captured: &'snapshot CapturedRegisteredFormV1,

  fn owner_descriptor_manifest_key(&self)
    -> SnapshotManifestKeyRefV1<'snapshot>;
  fn form_descriptor_manifest_key(&self)
    -> SnapshotManifestKeyRefV1<'snapshot>;
  fn form_descriptor_byte_length(&self) -> u64;
  fn form_descriptor_content_fingerprint(&self)
    -> &'snapshot SnapshotLeafFingerprintV1;
  fn form_root_span(&self)
    -> &'snapshot PlatformXmlSourceSpanV1;
  fn form_type_capture(&self)
    -> &'snapshot PlatformRegisteredFormTypeCaptureV1;
  fn form_type_authority(&self)
    -> &'snapshot PlatformRegisteredFormTypeAuthorityV1;
  fn form_properties_span(&self)
    -> &'snapshot PlatformXmlSourceSpanV1;
  fn form_type_spans(&self)
    -> &'snapshot [PlatformXmlSourceSpanV1];
}

RegisteredMaterialExpectationHandleV1<'snapshot> {
  // all fields private and Task4-owned; Task5B imports this type verbatim
  resolved_source_set: &'snapshot ResolvedSourceSet,
  source_fingerprint: &'snapshot SourceFingerprintV1,
  manifest: &'snapshot SourceManifestV2,
  captured: &'snapshot CapturedRegisteredFormV1,
  expectation: &'snapshot RegisteredMaterialExpectationV1,

  fn kind(&self) -> RegisteredMaterialKindV1;
  fn relationship(&self)
    -> RegisteredMaterialRelationshipRefV1<'snapshot>;
  fn state(&self)
    -> RegisteredMaterialExpectationStateViewV1<'snapshot>;
}

RegisteredMaterialExpectationStateViewV1<'snapshot>
  NotApplicable
  | Missing {
      expected_manifest_key: SnapshotManifestKeyRefV1<'snapshot>,
    }
  | Present {
      expected_manifest_key: SnapshotManifestKeyRefV1<'snapshot>,
      byte_length: u64,
      content_fingerprint: &'snapshot SnapshotLeafFingerprintV1,
    }

CapturedAnalysisBslMaterialKindV1
  Ordinary             tag 1
  RegisteredFormModule tag 2

CapturedAnalysisBslMaterialHandleV1<'snapshot> {
  // Task4-owned opaque borrow; import is whitelisted only to the Task5B
  // context/plan builder. Task6 cannot name, inspect or serialize it.
  resolved_source_set: &'snapshot ResolvedSourceSet,
  source_fingerprint: &'snapshot SourceFingerprintV1,
  manifest: &'snapshot SourceManifestV2,
  material: &'snapshot CapturedAnalysisBslMaterialV1,

  fn kind(&self) -> CapturedAnalysisBslMaterialKindV1;
  fn module(&self) -> Option<&'snapshot ArtifactRef>;
  fn admission_byte_length(&self) -> Option<u64>;
  fn registered_form(&self)
    -> Option<CapturedRegisteredFormViewV1<'snapshot>>;
  fn diagnostic_location(&self)
    -> CapturedBslLocationRefV1<'snapshot>;
}

impl CapturedAnalysisBslMaterialIndexV1 {
  // Called only by the private SourceSetSnapshotV2 constructor after the
  // source identity and manifest/Form/relationship authority have been
  // accepted.
  fn derive_checked(
    resolved_source_set: &ResolvedSourceSet,
    manifest: &SourceManifestV2,
  ) -> Result<Self, SnapshotCaptureError>;

  // Rebuilds the same private owned entries and compares them exactly; it does
  // not trust the stored derivative and performs no filesystem I/O.
  fn validate_against(
    &self,
    resolved_source_set: &ResolvedSourceSet,
    manifest: &SourceManifestV2,
  ) -> Result<(), SnapshotCaptureError>;
}

impl CapturedAnalysisBslMaterialV1 {
  // Validates the owned authority, kind, module projection, admission length,
  // diagnostic authority and order key against the enclosing accepted
  // manifest before either Task4 specialized reader performs I/O.
  fn validate_against(
    &self,
    resolved_source_set: &ResolvedSourceSet,
    manifest: &SourceManifestV2,
  ) -> Result<(), SourceReadError>;
}

CapturedBslLocationRefV1<'snapshot> {
  // opaque manifest/relationship-derived authority; no raw accessor
  fn to_verified_location(self) -> VerifiedBslSourceLocationV1;
}

VerifiedBslSourceLocationV1 {
  // owned receipt-grade authority. Private fields bind exact source,
  // fingerprint, leaf/absence authority, opaque manifest key and optional
  // validated byte span/1-based line+column. No raw key/path accessor or serde.
}

VerifiedBslCacheLocatorV1 {
  // opaque cache-adapter capability; no String/path/key accessor or serde
}

VerifiedCapturedBslMaterialBytesV1<'snapshot> {
  // private captured ordinary-Present handle plus owned verified bytes

  fn bytes(&self) -> &[u8];
  fn location_for_range(
    &self,
    start_byte: u32,
    end_byte_exclusive: u32,
  ) -> Result<VerifiedBslSourceLocationV1, SourceReadError>;
  fn cache_locator(&self) -> VerifiedBslCacheLocatorV1;
}

impl SourceSetSnapshotV2 {
  fn registered_forms<'snapshot>(
    &'snapshot self,
  ) -> impl ExactSizeIterator<
         Item=RegisteredFormCaptureHandleV1<'snapshot>> + 'snapshot;

  fn resolve_registered_material_projection<'snapshot>(
    &'snapshot self,
    projection: &RegisteredMaterialRelationshipProjectionV1,
  ) -> Result<RegisteredMaterialExpectationHandleV1<'snapshot>,
              SourceReadError>;

  fn captured_analysis_bsl_materials<'snapshot>(
    &'snapshot self,
  ) -> impl ExactSizeIterator<
         Item=CapturedAnalysisBslMaterialHandleV1<'snapshot>> + 'snapshot;
}
```

No resolver accepts an arbitrary `&CapturedRegisteredFormV1` plus a separate
snapshot, and no consumer constructs `(owner, Form, kind)`. The capture handle
itself, not Task 5B, binds the enclosing source identity, source fingerprint,
manifest, and captured Form. `view()` exposes only the exact read-only
`CapturedRegisteredFormViewV1`; the private `CapturedRegisteredFormV1` is never
returned. `material(kind)` resolves the exact row in that same catalog.

Task 5B must import these two handle types and the view verbatim. It may not
redeclare their private fields, create a parallel handle DTO, retain detached
source/manifest/captured references, or reconstruct a handle from public
accessors. The capture view's key accessors return opaque
`SnapshotManifestKeyRefV1`, never `&SnapshotManifestKeyV1`, a string, bytes,
components, or a path.

The expectation handle privately binds source identity, source fingerprint,
captured Form handle key, kind, and the borrowed expectation. Before any I/O,
the reader verifies that all of those semantic fields equal the supplied
snapshot and that relationship/state totality still validates. A cross-source,
cross-fingerprint, cross-Form, cross-kind, or cross-state swap returns
`SourceReadError::RegisteredMaterialHandleMismatch` before I/O. Equality is
semantic, not pointer/generation identity: a separately allocated snapshot
reconstructed from exactly equal accepted authority is not rejected merely
because its address differs.

`relationship()` returns the Task4-owned opaque relationship ref, never the
private `RegisteredMaterialRelationshipKeyV1` or an owner/Form/kind tuple. Task
5B may retain only its lossless projection token. Task 6 receives only Task 5B's
final `AnalysisBslMaterialScanPlanV1`/items and passes one admitted item to the
verified dispatcher; it cannot name or access any material-authority accessor,
relationship ref or projection directly.

`resolve_registered_material_projection` is the sole inverse authority seam.
Before I/O it validates the projection's private source identity, source
fingerprint and accepted manifest/catalog authority against `self`. It then
uses the projection's private source-local relationship key for exactly one
`registered_material_expectations.get` lookup in the accepted authoritative
`BTreeMap` and
validates the referenced captured Form, exact relationship value and complete
expectation state against the projection witness. The relationship lookup is
`O(log N)`; it never walks `registered_forms`, iterates the expectation map, or
performs an `O(N)` fallback. A recording-map test may observe the single indexed
lookup but no public API can observe or reconstruct its key.

A wrong snapshot, source, fingerprint, manifest, Form, kind, relationship or
state returns the closed nonretryable
`SourceReadError::RegisteredMaterialHandleMismatch` before any verifier or
filesystem call. A separately allocated snapshot reconstructed from exactly
equal accepted semantic authority is allowed. Success returns the Task4-owned
`RegisteredMaterialExpectationHandleV1<'snapshot>` by value; the resolver
performs zero material verification, byte reads, filesystem operations or XML
parses. It returns no tuple, path, key, projection payload or raw component.

`captured_analysis_bsl_materials()` is the sole Task4 projection of one atomic
source's captured BSL surface. Task4 recognizes an ASCII-case-insensitive final
`.bsl` extension only while it owns the validated manifest key; no downstream
consumer receives that spelling or repeats the suffix test. The projection
contains, in one total order:

1. every ordinary `ManifestEntryV2::Present` classified as BSL;
2. every `FormModule` relationship in Present, Missing or NotApplicable state;
   and
3. no filesystem entry, registration decoy or path that was outside the
   accepted capture.

A Present registered FormModule must point at its exact equal ordinary Present
entry. Projection construction replaces that ordinary candidate with one
`RegisteredFormModule` candidate at the same private manifest-order position;
it never appends a second item. Duplicate claims, a Present relationship without
the equal ordinary entry, two relationships claiming one entry, or an ordinary
candidate left independently visible while the relationship claims it reject
snapshot construction. Missing uses its private expected key as its order
anchor. Keyless NotApplicable is anchored immediately after its exact captured
Form descriptor and tie-broken by the complete source-local relationship bytes.
All other addressable items sort by exact normalized manifest-key UTF-8 bytes;
the anchor class and relationship tie-break are private. This is one stable
captured-manifest-derived order across all states without manufacturing a path
for NotApplicable.

Every `Some(ArtifactRef)` module projection is unique under the existing
validated `ArtifactRef` equality/order. Two captured files, a claimed/ordinary
pair not replaced, or case/alias-equivalent spellings that would project to the
same module reject the derived index before snapshot acceptance. No Task5A
identity-byte type is imported to enforce this Task4 invariant.

The projection is a checked derived index over the already fingerprinted
resolved-source identity, manifest, captured-Form and expectation authority,
not an independently mutable semantic catalog and not an additional fingerprint
field. Construction uses checked
addition/conversion, accepts at most
`MAX_CAPTURED_ANALYSIS_BSL_SCAN_ITEMS = 400_000`, and proves iterator length
equals the unique ordinary-BSL-plus-nonpresent-FormModule count. Input
permutation cannot change the order. Because it is derived only from authority
already bound by source/composite v2 and the frozen v2 subordinate policy,
adding it changes none of the section-10
payloads or published source/composite fingerprint goldens.

The private `SourceSetSnapshotV2` constructor first finishes and validates the
owned `ResolvedSourceSet` and `SourceManifestV2`, then calls
`CapturedAnalysisBslMaterialIndexV1::derive_checked(&resolved_source_set,
&manifest)`, and finally moves all three independent owned values into the
snapshot. The index owns every
private manifest/relationship/order/location key and every projected
`ArtifactRef`; it contains no borrow into the manifest and no pointer to a
temporary. `captured_analysis_bsl_materials()` borrows the stored
`analysis_bsl_material_index.entries` vector and each returned handle's
`material` field borrows that actual stored entry. It never rebuilds an entry on
the stack. Snapshot acceptance calls the index-level `validate_against` once:
the check derives the canonical index again only from the already-bound
resolved-source identity plus manifest authority and compares the complete
vector. A specialized pre-I/O
read instead calls only the addressed entry's `validate_against`, which performs
the exact indexed manifest/relationship lookups described below and never
rebuilds or scans the complete vector. Thus scanning N admitted items remains
`O(N log N)`, not `O(N^2)`. Any addressed-entry drift is the closed
nonretryable `RegisteredMaterialHandleMismatch`, not a fallback/reorder/
reclassification.
The index, all of its backing fields, and the rebuild comparison are excluded
from source/composite fingerprint encoding; authority remains solely the
already encoded manifest/Form/relationship data.

The entry grammar is closed. `Ordinary { manifest_key }` requires the exact
Present BSL manifest row, uses `module=Some(typed module)` or `None` only when
the section-3.1 Task4 BSL-to-module grammar classifies that row as supported or
unsupported respectively,
uses `admission_byte_length=Some(row.byte_length)`,
`PresentManifestLeaf { same manifest_key }`, and
`Addressable { same anchor_manifest_key, relationship_tie_break=None }`.
`RegisteredFormModule { relationship_key }` requires an exact FormModule
relationship and always has `module=Some(exact typed FormModule artifact)`. Its
remaining fields are exactly:

| expectation | admission length | location authority | order authority |
| --- | --- | --- | --- |
| Present(key,len,leaf) | `Some(len)` | `PresentManifestLeaf { manifest_key=key }` | `Addressable { anchor_manifest_key=key, relationship_tie_break=Some(relationship_key) }` |
| Missing(key) | `None` | `MissingRegisteredLeaf { relationship_key, expected_manifest_key=key }` | `Addressable { anchor_manifest_key=key, relationship_tie_break=Some(relationship_key) }` |
| NotApplicable | `None` | `NotApplicableRegisteredLeaf { relationship_key, form_descriptor_manifest_key=the exact captured Form descriptor key }` | `AfterCapturedFormDescriptor { anchor_manifest_key=that descriptor key, relationship_tie_break=relationship_key }` |

No other combination is constructible. For Present, the relationship's key,
length and leaf must equal the ordinary Present row that the index removes from
the Ordinary partition. The private order comparator is the tuple of exact
normalized `anchor_manifest_key` bytes, `Addressable=1` versus
`AfterCapturedFormDescriptor=2`, and the optional complete relationship
tie-break; the vector is strictly increasing under that comparator. These
private owned keys support validation only and have no accessor outside Task4.

`CapturedAnalysisBslMaterialHandleV1` and `CapturedBslLocationRefV1` are
Task4-owned borrow capabilities with private fields, no Clone/serde/raw
constructor and no key/path/component/byte accessor. A product dependency scan
permits them only in Task4 and the Task5B context/plan builder. Task6 receives
only Task5B's final plan/item API and cannot import these capture types.
`module()` is the sole typed identity projection: it returns `Some` for a
supported ordinary BSL module and for every registered FormModule obligation,
and `None` only for a captured unsupported ordinary BSL file. Task4 derives it
while it owns the private manifest key and the canonical BSL-to-`ArtifactRef`
grammar; no downstream suffix/path parsing is possible. The value is a checked
projection of already fingerprinted capture authority, not a new fingerprint
field. Task5B may use it only to look up the equal context-owned `ArtifactRef`.
`admission_byte_length()` is likewise builder-only: it returns `Some(exact
captured byte length)` for ordinary Present and registered FormModule Present,
and `None` for registered Missing/NotApplicable. It exposes no state tag,
manifest key, path, fingerprint, relationship, reader argument or mutable
counter. Task5B distinguishes the two registered non-Present states only through
its already retained exact registered-material witness; Task6 cannot call this
accessor or import the capture handle.
`CapturedBslLocationRefV1::to_verified_location()` is the only builder-whitelisted
ownership transition for the pre-read diagnostic anchor. It performs no I/O and
returns the opaque owned `VerifiedBslSourceLocationV1`; it cannot expose or
authorize use of the contained key/path. Task6 receives only that final owned
location through Task5B plan items.

`VerifiedBslSourceLocationV1` is the receipt-grade location authority for
snapshot BSL observations. It may be attached directly to an evidence record
or passed to the whitelisted receipt/location encoder; neither operation
returns its private manifest spelling to Task6. `VerifiedBslCacheLocatorV1` may
be passed only to the typed BSL cache adapter, which serializes the contained
source-relative locator at its infrastructure boundary. Task6 cannot obtain a
`String`, `Path`, manifest key or reusable read argument from either value.
`location_for_range(start_byte, end_byte_exclusive)` validates the numeric
zero-based half-open range independently against the exact bytes owned by that
wrapper, including checked `u32` conversion and UTF-8/code-point boundaries,
then computes deterministic 1-based line/column and binds the exact source
fingerprint, leaf fingerprint and manifest location. It imports no Task6 parser
or span type. Equal snapshot bytes and ranges produce equal location authority;
an empty, reversed, out-of-range or non-boundary range is a nonretryable
invariant error, never a guessed coordinate.

```text
SourceReadError adds the closed nonretryable case:
  RegisteredMaterialHandleMismatch
    public reason "registered_material_handle_mismatch"
```

Handles cannot outlive the snapshot borrow, cannot be serialized, and expose no
raw expected-key string or relationship constructor.

## 5. Neutral FormType capture and later descriptor use

The neutral bounded parser operates on the exact captured Form descriptor bytes
and exact structural namespace:

```text
MDCLASSES_NS = "http://v8.1c.ru/8.3/MDClasses"
```

Capture requires one exact expanded-name path
`MetaDataObject/Form/Properties`, the registered Form envelope, and the shared
v7 semantic-view constraints. Local-name-only acceptance, `urn:1c`, a foreign
same-local envelope, DTD/entities, and unbounded DOM allocation are forbidden.

Under the one exact `Properties`, the parser examines every direct child whose
local name is `FormType`:

- zero same-local observations adds `Missing`;
- more than one exact/foreign observation adds `Duplicate`;
- one foreign-only observation adds `WrongNamespace`;
- element-bearing or ambiguous mixed content adds `MixedContent`; and
- an exact scalar other than case-sensitive `Managed` or `Ordinary`, or an
  empty, control-bearing, over-512-byte, or over-128-scalar value, adds
  `UnsupportedOrOverLimit`.

It computes the complete defect set and chooses the lowest numeric problem tag;
XML order cannot change authority. Only an empty defect set plus one exact
scalar produces `Known`. `form_type_spans` contains every exact/foreign
same-local observation participating in that result. For true absence it is
empty and `form_properties_span` is the physical fallback. The neutral parser
also returns `form_root_span` to Task 4's captured envelope.

Primary capture computes this authority once from the same stable descriptor
bytes whose length/leaf enters the ordinary Present entry and handle. The final
race pass independently recomputes and compares the whole envelope, then
discards its temporary result.

Task 5B is not forbidden from reading a Form descriptor for wrapper UUID,
membership, or other semantic-view fields absent from the Task 4 handle. Its
single public object-safe
`PlatformCatalogPort::build_context(&SourceSnapshotV2,
&dyn SourceSnapshotPort)` visits the composite's Analysis then canonical
Destination atomic snapshots and performs exactly one call to
`reader.read_registered_form_descriptor_verified(atomic_snapshot,
&capture_handle)` plus one
shared `semantic_views` parse for **every** captured Form. Task 5B may not call
`reader.read_verified(snapshot, path)` for a Form descriptor, because it has no
raw key or path and because that would discard the capture-handle binding. The shared
guard may recompute the stored capture envelope as an integrity check, but Task
5B cannot independently select, reinterpret, repair, or replace FormType
authority: it copies the stored neutral authority/spans. Any recomputed-envelope
mismatch after a verified equal descriptor leaf is an invariant/snapshot
failure, never a new FormType conclusion. There is no second task-local
FormType parser. A pure `from_prepared` constructor, if useful in
implementation, is private and cannot replace the one public port boundary.

## 6. Sole path serializer, collisions, and containment

The serializer runs only for exact `Known(Managed)`. Its input is the already
capture-validated `form_descriptor_manifest_key`, never names, an ArtifactRef,
or display text. It requires one exact case-sensitive terminal ASCII `.xml`,
removes only those four bytes to obtain `form_stem`, and applies exactly:

```text
FormXml    = form_stem || "/Ext/Form.xml"
FormModule = form_stem || "/Ext/Form/Module.bsl"
```

The two suffix literals exist in one Task 4 relationship module only (apart
from tests/spec diagnostics). A static test rejects either literal or an
equivalent owner/Form formatter in Task 5B/6/7/8 production modules.

Every result re-enters `SnapshotManifestKeyV1` validation. The serializer
rejects absent/uppercase/nonterminal/duplicated `.xml`, a 4,097-byte result,
invalid components, and checked-arithmetic overflow. Registration construction
already proves the descriptor is the exact child of its owner; a generic
unregistered `.xml` key cannot enter this API.

Within each source, handle keys, relationship keys, and expected keys are
sorted and unique. Capture rejects:

- a duplicate handle or relationship identity;
- two applicable relationships claiming one expected key;
- case-fold/alias equality between differently spelled expected keys;
- a derived key colliding case-fold/alias-wise with a differently spelled
  admitted manifest key;
- a derived path resolving to another admitted file identity;
- escape from its retained source-root handle; and
- key/path/count/length arithmetic overflow.

An exact expected key equal to its one ordinary Present manifest key is not a
collision; it is the required `Present` binding. Relationship identity remains
source-local, so equal relative spelling in two different accepted source
authorities is not collapsed.

The deterministic preflight collision key is component-wise Rust Unicode
lowercase expansion over the normalized slash spelling. Existing no-follow
handle identity is the platform-specific alias authority. Folded ambiguity is
rejected even on a case-sensitive host.

## 7. Initial capture and contained absence proof

All sources share the existing composite budget and deadline. For each source,
capture performs:

1. resolve/validate its retained root and source-map identity;
2. registration-aware enumeration of Configuration, registered owners, nested
   descriptors, and registered `Ext` subtrees, fixing the complete initial
   present-key plan while excluding unregistered decoys;
3. one contained no-follow read/hash of each globally unique Present material;
4. neutral parse of each registered Form descriptor and construction of its
   exact handle envelope;
5. exactly two relationship rows per handle; Ordinary/Inconclusive write two
   `NotApplicable` rows without serializer or path I/O;
6. for Managed, derive two expected keys; an exact key in the present plan must
   bind its equal ordinary Present length/leaf, otherwise the contained absence
   proof below must succeed before `Missing` is recorded; and
7. validate bounds, totality, matrix, ownership, aliases/collisions, descriptor
   leaf/span binding, and Present equality before manifest construction.

A Present material is one ordinary captured file referenced by one expectation;
it is neither counted nor hashed twice. Missing and NotApplicable consume
expectation capacity but zero file/byte capacity. NotApplicable causes zero
expected-path calls.

`prove_registered_material_absent(root_handle, expected_key)` proves absence at
the first absent component, not only at the final leaf:

1. every existing non-final component opens relative to the current retained
   directory handle with exact-name, directory, no-follow/reparse validation;
2. an existing final regular file means Present, never Missing;
3. the first exact `NotFound`/`PathNotFound` at any component proves the
   remaining suffix absent; and
4. symlink, junction, reparse point, special file, `NotDirectory`, access
   denial, share violation, unstable identity, case/alias sibling, or any
   other error proves neither safe absence nor Present.

Unix uses component-relative directory handles with `openat`-style
`O_NOFOLLOW`; Windows uses the existing relative `NtCreateFile`/reparse-handle
discipline. A path-wide `exists`, `canonicalize`, `symlink_metadata`, or
check-then-open sequence is nonconforming.

The first-absent index may exist only as an internal diagnostic/test value. It
is not semantic state, is not fingerprinted, and is never exposed. Missing an
entire `Ext` parent and missing only `Module.bsl` encode the same relationship/
key/Missing authority.

## 8. Final capture pass and race closure

After initial hashing and the injected mutation hook, capture independently
repeats registration-aware enumeration and recomputes:

- source-map and registered owner/Form identities;
- descriptor key, length, leaf, and stable file identity;
- root/properties/FormType authority and all canonical spans;
- complete handle and relationship sets;
- derived expected keys and applicability/presence states; and
- every Present length, leaf, and stable identity.

It requires exact initial/final plan equality. It then rereads every Present
file under its captured length bound and repeats the contained first-absent
proof for every Missing row after the last mutation hook. It performs no
absence check for NotApplicable.

Any appearance, disappearance, registration rename, FormType transition,
handle/relationship/key/state change, content/identity drift, span change,
symlink/reparse swap, or case/alias change rejects the whole snapshot as
retryable `SnapshotCaptureReason::SourceChangedDuringCapture` with reason
`source_changed_during_capture`. No partial catalog or provisional Missing
authority survives. Capture-time drift is not
`source_fingerprint_mismatch`, which is reserved for post-capture verified
reads.

Stable malformed XML/registration, unsafe topology, stable I/O failure,
deterministic resource overflow, impossible manifest state, and deadline retain
the existing Task 4 typed classification.

## 9. Exact bound semantics

Existing limits apply once to the complete composite capture:

- globally unique Present files/bytes count once toward 200,000/4 GiB;
- every examined entry in both enumerations and each absence walk consumes the
  traversal/deadline budget;
- each XML input is bounded at 64 MiB, depth 128, and 1,000,000 nodes;
- every loop, allocation, parse, hash, final read, and absence proof checks the
  injected 120-second monotonic deadline; and
- all subtraction, multiplication, summation, and allocation uses checked
  arithmetic before side effects.

The raw global expectation accumulator/decoder independently accepts exactly
400,000 and rejects 400,001, including untrusted persisted/internal count
input. A structurally valid catalog has two rows per Form, so the end-to-end
valid-matrix boundary is 400,000 pass and 400,002 fail. The accumulator sums all
source manifests; there is no per-source reset or admission prefix.

For a one-source/disjoint fixture, a nonempty Form set requires one
Configuration, one owner descriptor, and one descriptor per Form. At the
200,000 unique-file boundary that fixture has 199,998 Forms and 399,996
expectations, named only
`SINGLE_SOURCE_EXPECTATION_FIXTURE_AT_FILE_LIMIT`. Those are mandatory
end-to-end fixture boundaries, but 399,996 is not an absolute global maximum:
nested/overlapping non-identical source roots can reuse globally deduplicated
present files while retaining distinct source-local handles. A separate overlap
fixture proves checked summation up to 400,000 and valid-matrix rejection at
400,002; the isolated raw-count helper proves 400,001 rejection.

## 10. Source/composite fingerprint v2

V2 retains the v1 source-identity and ordinary-entry framing byte-for-byte,
then appends the complete captured-Form catalog and complete expectation map.
All integers are unsigned big-endian. `string(x)` is
`u64be(UTF-8 byte length) || UTF-8 bytes`.

```text
u64be(len("unica.source-set-snapshot.v2"))
|| "unica.source-set-snapshot.v2"
|| string(source name)
|| u8(source kind tag)
|| u8(source format tag)
|| string(relative root)
|| string(mapping fingerprint exact transport)
|| u64be(ordinary entry count)
|| for ordinary entries sorted by exact key UTF-8 bytes:
     string(manifest key)
     || [Present:
           u8(1) || u64be(byte_length)
           || string(leaf fingerprint exact transport)
        | AbsentOptional:
           u8(2) || u8(the existing optional tag)]
|| u64be(captured form count)
|| for handles sorted by (owner key bytes, Form key bytes):
     string(owner_descriptor_manifest_key)
  || string(form_descriptor_manifest_key)
  || u64be(form_descriptor_byte_length)
  || fingerprint32(form_descriptor_content_fingerprint)
  || u32be(form_root_span.start_byte)
  || u32be(form_root_span.end_byte_exclusive)
  || [Known:
        u16be(1) || u16be(Managed=1 | Ordinary=2)
      | Inconclusive:
        u16be(2) || u16be(problem tag)]
  || u32be(form_properties_span.start_byte)
  || u32be(form_properties_span.end_byte_exclusive)
  || u64be(form_type_span_count)
  || for each canonical sorted-unique FormType span:
       u32be(start_byte) || u32be(end_byte_exclusive)
|| u64be(expectation count)
|| for expectations sorted by
     (owner key bytes, Form key bytes, material-kind tag):
     string(owner_descriptor_manifest_key)
  || string(form_descriptor_manifest_key)
  || u16be(FormXml=1 | FormModule=2)
  || u16be(NotApplicable=1 | Missing=2 | Present=3)
  || [NotApplicable: empty
      | Missing: string(expected_manifest_key)
      | Present: string(expected_manifest_key)
        || u64be(byte_length)
        || fingerprint32(content_fingerprint)]
```

`fingerprint32` is the raw 32-byte digest, not its 71-byte transport spelling.
First-absent index, file handle/identity, mtime, epoch, and diagnostics are not
semantic encoder fields. Descriptor length/leaf and the complete neutral
authority/witness matrix are encoder fields; equal expectation rows cannot
replay a different Form descriptor or FormType capture.

The source fingerprint is lowercase `sha256:` transport of SHA-256 over the
complete bytes. `CompositeSnapshotIdV2` is the private raw 32-byte SHA-256
result with one canonical lowercase `sha256:` transport projection. The
composite encoder changes only its domain to `unica.source-composite.v2` and
embeds `analysis_snapshot()` first and then
`destination_snapshots()` in their canonical unique order, using only v2
source fingerprints plus the existing role/full-source-identity/count framing.
The diagnostic epoch is excluded. V1 and v2 snapshots are not equal, mixable,
or dual-accepted.

`write_platform_catalog_execution_digest_v1` is the sole cross-owner raw
projection. It appends exactly those same 32 private digest bytes, in order,
with no `digest32` tag, length, transport prefix, domain, hash, allocation or
return value. It does not accept caller bytes and cannot be used to construct,
parse or compare a `CompositeSnapshotIdV2`. Its production call-site whitelist
contains exactly one fully qualified Task 5B method:

```text
PlatformCatalogExecutionBindingV1::write_identity_v1
```

Task 5B's method appends the composite bytes followed by its two catalog-set
digests; Task 4 imports neither that type nor either catalog digest. Unit tests
inside the Task 4 owner module may call the writer only for the mechanical
32-byte golden. Product/static tests reject an alias, callback, function
pointer, re-export, second production caller, raw `[u8; 32]`/slice accessor,
serde ingress or new cross-owner transport/string projection. Task4's existing
private canonical diagnostic transport renderer is not broadened. This narrow writer changes no source/composite
fingerprint input or published golden; it only makes the already accepted raw
composite identity composable without duplicating its private representation.
The whitelist is phased with the production DAG: Task4-v7 acceptance permits
zero downstream production callers and reserves only the fully qualified Task5B
method above; Task5B acceptance must activate exactly that one call site and
rerun the same product/static check. The reservation is not a Task4 -> Task5B
import, compile dependency or implementation gate.

### 10.1 Normative expectation-row goldens

The common relationship uses owner `Catalogs/Σ.xml`, Form descriptor
`Catalogs/Σ/Forms/Main.xml`, and `FormXml` tag 1. Missing/Present use
`Catalogs/Σ/Forms/Main/Ext/Form.xml`; Present uses length 7 and 32 raw `0xff`
fingerprint bytes.

```text
NotApplicable row length = 61
SHA-256(row bytes) =
  919a2297228863374bc95db5c2202b207a44800963d2f908832cd7d8974900f8

Missing row length = 104
SHA-256(row bytes) =
  e1b7e076c32b256446871f8057a4b2302399d7cc4c111d0ec131ddaa01987bfe

Present row length = 144
SHA-256(row bytes) =
  a5b89c72bf780da36a0a8f84115e3ef4314843ef578fd48cab35de1032fdb2f3
```

### 10.2 Normative complete-source goldens

The source is name/root `main`, kind Configuration tag 1, format PlatformXml
tag 1, mapping fingerprint `sha256:` plus 64 `a` characters. Ordinary base
entries are:

```text
main/Configuration.xml                       length 64, leaf b*64
main/Catalogs/Owner.xml                      length 96, leaf c*64
main/Catalogs/Owner/Forms/Main.xml           length 128, leaf d*64
```

The captured Form has descriptor length 128/raw `0xdd` leaf, root span
`0..128`, properties span `20..100`, and one FormType span `40..60`.
NotApplicable uses Known Ordinary; Missing/Present use Known Managed. Present
adds Form.xml length 7/leaf `e*64` and FormModule length 11/leaf `f*64` to both
the ordinary map and their expectation rows.

```text
zero Forms, Configuration only:
  payload length = 283
  source fingerprint =
    sha256:c43a6977111248ecd07529e915d8df5161b807c85040a4c0e52073d9e504276a

one Form, both rows NotApplicable:
  payload length = 835
  source fingerprint =
    sha256:b9cbb191b66c5d77a72b3778bf00b112fcb856672cdc37283e32739dcc715372

one Form, both rows Missing:
  payload length = 944
  source fingerprint =
    sha256:1efbe72c8f737a8457da4549f7de58f964dd4181630f8d82bfcd04c0cb66c353

one Form, both rows Present:
  payload length = 1309
  source fingerprint =
    sha256:f6dfa986d1db47f20889d61abad6a2e2748ed34da9903a85f47d4dcf803c811c

zero-mutation composite over the Missing source:
  payload length = 226
  composite fingerprint =
    sha256:1b71f1419ff84829592480f1c6c810f1b3df20a6bcc39e025eb4904785d4cd16
```

Tests reconstruct complete bytes through production encoders and independently
assert lengths/order; copying digest literals alone is insufficient. Mutating
any handle key, descriptor length/leaf, root/properties/type span, authority,
problem tag, relationship field, expected key/state/length/leaf, ordinary
entry, source identity, or domain changes the appropriate fingerprint. Handle,
span, expectation, and ordinary-entry input permutations do not.

## 11. Injected Task4 handle-only verified-reader methods

Historical readers retain their closed scope:

- `SourceSnapshotPort::read_verified(snapshot, path)` reads only an ordinary
  Present entry;
- `SourceSnapshotPort::read_optional_verified(snapshot, path)` checks only five
  fixed optional tags;
- neither can create dynamic Missing authority.

The successor extends that same injected, object-safe `SourceSnapshotPort`; it
does not introduce a free reader function, hidden global filesystem/root
capability, or reader stored inside semantic snapshot/handle/projection state:

```text
trait SourceSnapshotPort {
  fn capture(
    &self,
    analysis: &ResolvedSourceSet,
    destinations: &[ResolvedSourceSet],
    diagnostic_workspace_epoch: u64,
  ) -> Result<SourceSnapshotV2, SnapshotCaptureError>;

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

The v2 migration replaces the historical `capture -> SourceSnapshot` result;
there is no parallel v1 capture overload or implicit conversion. The existing
ordinary and five-tag optional readers likewise take the atomic
`&SourceSetSnapshotV2`. `capture` is one composite operation over analysis plus
all destinations, while every byte/material reader is deliberately atomic.

These methods have only lifetime parameters, no generic type parameter, `Self`
result, associated constructor or sized receiver; all three are callable through
`&dyn SourceSnapshotPort`. `&self` is the sole live root/I/O/counter authority.
`SourceSetSnapshotV2`, all handle types, both projections and every verified
result contain no `SourceSnapshotPort` reference, callback, trait object or
filesystem/root capability.

### 11.1 Registered Form descriptor reader

Task 4 exposes the sole downstream descriptor-read result:

```text
VerifiedRegisteredFormDescriptorBytesV1<'snapshot> {
  // all fields private/non-serde; Task4-owned semantic authority
  resolved_source_set: &'snapshot ResolvedSourceSet,
  source_fingerprint: &'snapshot SourceFingerprintV1,
  manifest: &'snapshot SourceManifestV2,
  captured: &'snapshot CapturedRegisteredFormV1,
  bytes: Box<[u8]>,

  fn bytes(&self) -> &[u8];
}
```

Before I/O, the port method compares the handle's source identity and source
fingerprint authority against `snapshot` without walking the manifest. It then
uses the private captured-Form handle key for exactly one
`captured_registered_forms.get` `BTreeMap` lookup and requires exact descriptor
key/length/leaf plus root/properties/FormType envelope equality. It also requires
the descriptor's ordinary manifest entry to be exact Present with the same
length/leaf. A source, fingerprint, Form, envelope, state, key, catalog or
ordinary-entry disagreement returns the nonretryable
`SourceReadError::RegisteredMaterialHandleMismatch` /
`registered_material_handle_mismatch` before a verifier or filesystem call.
This includes impossible/corrupt internal authority after construction.
Semantic equality is sufficient; allocation/pointer identity is not authority.

The source/fingerprint checks are `O(1)` in catalog cardinality and the exact
captured-Form lookup is `O(log N)`. Descriptor validation never compares a full
`SourceManifestV2`, walks `registered_forms`, or iterates either manifest map.
Thus `N` descriptor reads do not create an `O(N^2)` context build.

After semantic validation, the port method internally obtains the private Form
descriptor key and delegates exactly once through the same injected `&self` to
the existing ordinary Present verified reader. Only external filesystem drift
observed by that read—descriptor disappearance, content/identity change, or an
ancestor topology change—returns retryable `SourceFingerprintMismatch`.
Success wraps the bytes with the same semantic snapshot/source/Form/key/length/
leaf authority.
The wrapper exposes only `bytes()`; it has no capture/key/path/string/serde/
constructor accessor and cannot be replayed as another Form descriptor.

Task 5B's public context build receives `reader: &dyn SourceSnapshotPort` and
must call `reader.read_registered_form_descriptor_verified(...)` exactly once
per captured Form in canonical iterator order. It cannot call raw
`read_verified`, unwrap a key, or introduce a second descriptor reader. Method
calls and the delegated ordinary reads are recorded by that exact injected port
spy; they are not dynamic FormXml/FormModule verifier calls.

### 11.2 Registered Form material reader

The successor adds:

```text
RegisteredMaterialReadV1<'snapshot>
  NotApplicable(VerifiedRegisteredMaterialNotApplicableV1<'snapshot>) tag 1
  Missing(VerifiedRegisteredMaterialAbsenceV1<'snapshot>)             tag 2
  Present(VerifiedRegisteredMaterialBytesV1<'snapshot>)               tag 3

VerifiedRegisteredMaterialNotApplicableV1<'snapshot>
VerifiedRegisteredMaterialAbsenceV1<'snapshot>

VerifiedRegisteredMaterialBytesV1<'snapshot> {
  // private semantic snapshot/source/relationship/key/length/leaf authority
  fn bytes(&self) -> &[u8];
  fn location_for_range(
    &self,
    start_byte: u32,
    end_byte_exclusive: u32,
  ) -> Result<VerifiedBslSourceLocationV1, SourceReadError>;
  fn cache_locator(
    &self,
  ) -> Result<VerifiedBslCacheLocatorV1, SourceReadError>;
}
```

There is no overload accepting a path, ArtifactRef, owner/Form names, raw tuple,
expected key, or caller digest. Before I/O the injected port method validates
the private source/fingerprint/manifest/catalog/Form/kind binding, exact indexed
relationship row, complete state/key/length/leaf and, for Present, equality with
the ordinary entry. Any internal handle/projection/relationship/state/key/
ordinary-entry disagreement is either impossible after accepted construction or
returns nonretryable `RegisteredMaterialHandleMismatch` before I/O.

| Frozen state | Verified behavior | Result |
| --- | --- | --- |
| `NotApplicable` | no verifier filesystem operation | opaque NotApplicable authority |
| `Missing(key)` | repeat contained first-absent no-follow proof | opaque absence bound to source fingerprint, relationship, and key |
| `Present(key,len,leaf)` | require equal ordinary Present entry, then perform its existing verified read | opaque registered-material bytes bound to source, relationship, key, length, and leaf |

Only after that semantic validation can external filesystem drift return
retryable `SourceReadError::SourceFingerprintMismatch` with reason
`source_fingerprint_mismatch`: a captured Missing key appears, a captured
Present file disappears or changes content/identity, or an existing ancestor's
topology/identity changes. Stable I/O/identity service failure remains
`SnapshotUnavailable`. Internal semantic disagreement and a foreign/corrupt
handle are never reclassified as filesystem drift.

All three verified result fields are private and non-serde. The Present wrapper
prevents bytes verified for one Form/kind from being replayed as another and
exposes bytes plus only the opaque BSL location/cache projections above; for a
non-FormModule relationship either projection returns the nonretryable
`RegisteredMaterialHandleMismatch`. In particular, `cache_locator()` is
fallible on this common FormXml/FormModule wrapper: it contains no `unwrap`,
panic, fabricated BSL locator or unchecked kind cast. Missing/NotApplicable
expose no raw key or state payload;
Task 5B obtains key/relationship projections from the matching pre-read handle
views, never by unpacking a verified result.

### 11.3 Captured ordinary BSL reader and location authority

`read_captured_bsl_material_verified` accepts only an
`Ordinary` item produced by `captured_analysis_bsl_materials()`. Before I/O it
semantically validates the handle's source identity, source fingerprint,
manifest membership, private canonical order identity, ordinary-claim
partition, exact Present state, byte length and leaf fingerprint against the
supplied atomic snapshot. A registered FormModule item, a foreign snapshot,
cross-item swap, changed claim partition or internal manifest disagreement is
nonretryable `RegisteredMaterialHandleMismatch` before a reader/verifier/file
counter increments. It then delegates exactly once through the same injected
`&self` to the existing ordinary Present verified read and wraps the bytes;
only post-validation external drift can return retryable
`SourceFingerprintMismatch`.

The method has no path/ArtifactRef/string overload. Task5B's final analysis-BSL
plan is the sole whitelisted caller and consumes the capture handle by value.
Task6 cannot call ordinary `read_verified` for a scan item because it never
receives a manifest key; it can only pass the Task5B plan item back to the
Task5B context dispatcher with the injected reader. For one item the exact
Task4 counter ownership is:

| Captured item/read state | registered verifier calls | ordinary verified byte reads |
| --- | ---: | ---: |
| ordinary Present | 0 | 1 |
| registered FormModule Present | 1 | 1, delegated inside the registered reader |
| registered FormModule Missing | 1 | 0 |
| registered FormModule NotApplicable | 0 | 0 |

A claimed Present FormModule can take only the registered row; calling both
reader methods for that one captured leaf is a contract failure. The verified
ordinary and registered Present wrappers compute byte-span locations and cache
locators through the same Task4-owned implementation, so the identical
captured leaf/span produces byte-identical receipt/cache location authority
regardless of which private read branch delivered it.

## 12. Task 5B consumption and exact I/O boundary

The public object-safe
`PlatformCatalogPort::build_context(&SourceSnapshotV2, &dyn
SourceSnapshotPort)` borrows the one composite snapshot, visits
`analysis_snapshot()` and then every canonical `destination_snapshots()` member,
and iterates each atomic snapshot's Task4
`RegisteredFormCaptureHandleV1` values. It validates the handle/expectation
bijection and retains opaque relationship authority. It
does not redeclare or copy the handle's private source/fingerprint/manifest/
captured fields. For `N` captured Forms across the complete composite it
performs exactly `N` calls to
`reader.read_registered_form_descriptor_verified(atomic_snapshot,
&capture_handle)` and
exactly `N` shared semantic-view parses, once per Form in canonical handle
order inside canonical source order. It uses
`VerifiedRegisteredFormDescriptorBytesV1::bytes()` for wrapper
UUID, membership, and other catalog fields not carried by Task 4, while copying
the stored FormType authority through the read-only view. It does not format a
material path, probe dynamic Form.xml/FormModule, call
`reader.read_registered_material_verified`, or freeze a new material state. Any
pure prepared-context constructor is private.

Calling the catalog port twice with the same semantically equal composite and
reader authority is permitted and must produce equal context semantics and
digests. Task4 supplies no mutable consumed-generation token. The exactly-once
per execution requirement belongs to Task7 orchestration/static call-site tests
and recording spies; non-Clone context values prevent detached copies but do not
pretend to make a deterministic public build API globally one-shot.

The returned Task 5B context may retain a clone of the opaque
`CompositeSnapshotIdV2`, but cannot read its bytes. Its
`PlatformCatalogExecutionBindingV1` is the only downstream owner allowed to
compose that ID with the two catalog-set digests. The binding's exact identity
writer must delegate its first 32 bytes to
`write_platform_catalog_execution_digest_v1`; copying a private field,
rendering/reparsing `sha256:`, hashing a transport spelling, or asking Task 7 to
reconstruct the digest is forbidden. Context/query validation compares typed
semantic authorities and does not use pointer identity or a byte round trip.

The only lossless key projection allowed in Task 5B is:

```text
// Task5B-private consumer newtype; Task4 never imports it.
RegisteredFormManifestKeyV1 {
  capture_key: SnapshotManifestKeyProjectionV1,
}

// Required private field on the existing Task5B authority:
RegisteredFormMaterialAuthorityV1::capture_relationship:
  RegisteredMaterialRelationshipProjectionV1

impl RegisteredFormManifestKeyV1 {
  // private; called only with a ref returned by a capture view/state
  fn from_capture_key(
    key: SnapshotManifestKeyRefV1<'_>,
  ) -> Self {
    Self { capture_key: key.to_projection() }
  }

  fn encode_identity_v1(
    &self,
    encoder: &mut CanonicalIdentityEncoderV1,
  ) -> Result<(), CanonicalEncodingErrorV1> {
    self.capture_key.encode_identity_v1(encoder)
  }

  fn encode_registered_form_catalog_string_v1(
    &self,
    encoder: &mut CanonicalIdentityEncoderV1,
  ) -> Result<(), CanonicalEncodingErrorV1> {
    self.capture_key.encode_catalog_string_u32_v1(encoder)
  }
}

impl RegisteredFormMaterialAuthorityV1 {
  // private; relationship must come from the matching expectation handle
  fn retain_capture_relationship(
    relationship: RegisteredMaterialRelationshipRefV1<'_>,
  ) -> RegisteredMaterialRelationshipProjectionV1 {
    relationship.to_projection()
  }

  fn encode_capture_relationship_identity_v1(
    &self,
    enclosing_source: &ResolvedSourceSet,
    enclosing_source_fingerprint: &SourceFingerprintV1,
    encoder: &mut CanonicalIdentityEncoderV1,
  ) -> Result<(), CanonicalEncodingErrorV1> {
    self.capture_relationship.encode_source_local_identity_v1(
      enclosing_source,
      enclosing_source_fingerprint,
      encoder,
    )
  }
}
```

The downstream wrapper is private, has no generic/raw constructor, and stores
the Task4-owned projection token rather than a String, bytes, `PathBuf`, parsed
components, or a second normalized-key type. It may be created only from an
opaque key ref returned by `CapturedRegisteredFormViewV1` or
`RegisteredMaterialExpectationStateViewV1`. Clone/equality/order/hash delegate
to `SnapshotManifestKeyProjectionV1`; manifest/source identity delegates to
Task 4's unchanged u64 encoder, while each Task5B registered-Form sidecar
`string` field delegates to Task 4's sealed u32 catalog-string encoder through
`encode_registered_form_catalog_string_v1`. The wrapper has no raw accessor,
length accessor, local prefix writer or generic callback that could select a
third framing. No Task 5B module may inspect or independently encode key
spelling. This is a one-way consumer dependency and creates no Task4-to-Task5B
edge.

The same rule applies to the complete capture relationship. Task 5B obtains its
opaque ref only from the exact material handle, immediately turns it into the
Task4-owned projection, and stores that token inside
`RegisteredFormMaterialAuthorityV1`. It cannot retain/re-encode a private
`RegisteredMaterialRelationshipKeyV1`, owner/Form keys, source strings or a
kind tuple. Relationship equality/order/hash and source-binding validation
delegate to Task 4; after that check the nested encoder emits the unchanged
source-local owner/Form/kind row because the enclosing catalog header already
contains source identity/fingerprint. Task 6 sees only Task 5B's final
`AnalysisBslMaterialScanPlanV1`/item and
`read_analysis_bsl_material_verified` API; it has no material accessor,
resolver or relationship projection API.

Task 5B's private semantic resolver is the sole current consumer of
`SourceSetSnapshotV2::resolve_registered_material_projection`: for one
deduplicated demanded authority it passes the stored projection, receives the
Task4-owned handle by value, and only then invokes
`reader.read_registered_material_verified(snapshot, handle)` on the same
injected `&dyn SourceSnapshotPort`. Only Task 5B's private unified-material
dispatcher calls that resolver after consuming an admitted registered item;
Task 6 calls only `read_analysis_bsl_material_verified` and never receives,
clones, constructs or resolves a `RegisteredMaterialRelationshipProjectionV1`.
Projection resolution itself is index-only and zero-I/O, so it adds no verifier,
file-read or parse count to the demand table below.

Every query constructor performs **zero** reader calls, filesystem operations,
canonicalization/existence probes, material verifier calls, byte reads, and XML
parses. Dynamic registered-material freshness is demand-scoped to the semantic
provider after query construction and deduplicated by the complete private
source-fingerprint/Form/kind relationship identity.

Context-build spies for `N` captured Forms assert:

```text
registered_form_descriptor_verified_calls = N
ordinary_form_descriptor_verified_reads = N
ordinary_form_descriptor_semantic_view_parses = N
file_byte_read_calls = N                 // all ordinary Form descriptors
xml_parse_calls = N                      // all shared descriptor semantic views
registered_material_verifier_calls = 0
registered_material_file_byte_read_calls = 0
registered_material_xml_parse_calls = 0
direct_filesystem_calls = 0
```

Every counter above belongs to the injected `SourceSnapshotPort` spy or its
`FilesystemSourceSnapshots` implementation. There is no global/root reader
counter and no semantic snapshot/handle capability that can bypass the spy.

For one deduplicated demanded relationship, recording spies assert:

| State | `registered_material_verifier_calls` | `file_byte_read_calls` | `xml_parse_calls` |
| --- | ---: | ---: | ---: |
| `NotApplicable` | 0 | 0 | 0 |
| `Missing` | 1 | 0 | 0 |
| `Present` | 1 | 1 | 1 only if the consuming provider parses the returned bytes |

`Missing`'s verifier call is the handle-only contained absence check, not a byte
read or parse. Repeated demand for the same complete relationship within one
provider invocation reuses that one verification result. Demand for FormXml
does not verify FormModule, and vice versa. No provider performs direct
filesystem I/O. The Form-descriptor reads/parses have already occurred exactly
once per Form at the public context boundary and do not change these later
registered-material counters.

Task 6 observes FormModule only through the later Task 5B unified scan plan and
verified dispatcher; the private relationship projection never crosses that
boundary. Task 8 retains separate source-qualified analysis and destination
authorities. Neither can reuse an analysis expectation for a destination source
or recover the capture serializer.

## 13. Failure classification

| Observation | Capture result | Post-capture read result |
| --- | --- | --- |
| stable Managed + first absent component | accepted `Missing` | verified Missing while still absent |
| stable Ordinary/Inconclusive | accepted keyless `NotApplicable`; no path call | verified NotApplicable; no I/O |
| initial Missing, final Present | retryable whole-snapshot `SourceChangedDuringCapture` | n/a |
| initial Present, final Missing/change | retryable whole-snapshot `SourceChangedDuringCapture` | n/a |
| registration/FormType/span/key/state drift | retryable whole-snapshot `SourceChangedDuringCapture` | n/a |
| captured Missing later appears | n/a | retryable `SourceFingerprintMismatch` |
| captured Present later disappears/changes | n/a | retryable `SourceFingerprintMismatch` |
| registered Form descriptor later disappears/changes | n/a | retryable `SourceFingerprintMismatch` from descriptor reader |
| foreign source/fingerprint/Form/kind/state handle | snapshot construction/resolution rejection | nonretryable `RegisteredMaterialHandleMismatch` before I/O |
| internal handle/projection/relationship/state/key/ordinary-entry disagreement | nonretryable invariant rejection if observed | nonretryable `RegisteredMaterialHandleMismatch` before I/O |
| stable symlink/reparse/special/alias topology | existing unsafe/path classification | unavailable or mismatch according to observed drift |
| stable permission/transient I/O failure | existing typed Task 4 failure | `SnapshotUnavailable` |
| file/byte/traversal/XML/expectation overflow | nonretryable `SnapshotResourceLimit` | n/a |
| deadline | retryable `SnapshotDeadlineExceeded` | n/a |
| impossible manifest/catalog/envelope matrix | nonretryable `SnapshotInvariantViolation` | no I/O |

No capture failure returns partial catalog authority or invokes an evidence
provider. Dynamic material becomes a Task 5B gap only after a complete accepted
v2 snapshot and demand-scoped verified projection exist.

## 14. Mandatory permanent RED matrix

### 14.1 Domain, matrix, and encoders

```text
source_v2_complete_goldens_zero_not_applicable_missing_present
expectation_row_goldens_not_applicable_missing_present
composite_v2_golden_binds_source_v2
composite_platform_catalog_execution_writer_is_exact_raw_32_byte_golden
composite_platform_catalog_execution_writer_matches_private_composite_digest
composite_platform_catalog_execution_writer_adds_no_frame_prefix_or_transport
source_snapshot_v2_is_composite_and_source_set_snapshot_v2_is_atomic
composite_destinations_sort_by_complete_identity_and_reject_duplicates
diagnostic_epoch_changes_no_source_composite_or_catalog_identity
source_v1_and_v2_are_not_mixable
handle_and_expectation_permutations_are_byte_identical
each_handle_envelope_field_mutation_changes_source_fingerprint
each_expectation_field_mutation_changes_source_fingerprint
not_applicable_encodes_no_expected_key
present_requires_equal_ordinary_manifest_entry
handle_value_key_and_relationship_value_key_mismatch_are_rejected
expectation_totality_requires_exactly_form_xml_and_form_module
descriptor_length_leaf_and_span_bounds_are_validated
opaque_key_projection_is_lossless_and_preserves_task4_identity_bytes
opaque_key_catalog_string_projection_uses_exact_u32_utf8_byte_framing
opaque_key_identity_and_catalog_string_projections_keep_u64_and_u32_distinct
opaque_key_catalog_string_n_and_n_plus_one_prefixes_are_exact
opaque_key_catalog_string_4096_frames_and_4097_cannot_construct
opaque_key_projection_does_not_change_source_or_composite_goldens
opaque_relationship_projection_is_lossless_and_source_bound
relationship_projection_emits_unchanged_source_local_row_bytes
relationship_projection_rejects_wrong_enclosing_source_before_encoding
relationship_projection_resolution_preserves_all_published_goldens
relationship_index_is_authoritative_and_validated_before_fingerprint
opaque_projection_seam_preserves_all_published_nonempty_goldens
derived_bsl_scan_projection_changes_none_of_the_eight_published_fingerprint_goldens
```

### 14.2 Neutral parser and descriptor seam

```text
managed_form_gets_two_independent_applicable_states
ordinary_and_each_inconclusive_problem_get_two_keyless_not_applicable_states
form_type_full_defect_set_selects_lowest_tag_for_all_permutations
form_type_absence_uses_properties_span
root_properties_type_spans_and_authority_bind_descriptor_leaf
foreign_mdclasses_envelope_is_capture_fatal
xml_64mib_depth128_nodes1m_boundaries_pass_and_n_plus_one_fail
task5b_has_no_independent_form_type_parser_or_selector
task5b_verified_descriptor_semantic_view_mismatch_is_invariant_failure
neutral_span_and_form_type_accessors_are_read_only_and_complete
capture_view_exposes_every_required_witness_without_private_struct_access
```

### 14.3 Paths, containment, collision, and bounds

```text
serializer_runs_only_for_known_managed
serializer_accepts_only_exact_terminal_dot_xml
analysis_bsl_projection_registry_transcript_53_lines_1780_bytes_has_frozen_sha
analysis_bsl_projection_matrix_309_lines_24842_bytes_has_frozen_sha
analysis_bsl_projection_exhausts_common_44x6_owner_and_44_registered_form_rows
analysis_bsl_projection_registry_matches_shared_metadata_registry_minus_commonmodule
analysis_bsl_projection_module_kind_registry_matches_shared_exact_order
analysis_bsl_projection_registry_change_requires_source_fingerprint_domain_bump
analysis_bsl_projection_strips_dot_or_exact_multicomponent_relative_root_only
analysis_bsl_projection_preserves_exact_unicode_spelling_and_expanding_i_dot
analysis_bsl_projection_requires_exact_ascii_directory_and_filename_tokens
analysis_bsl_commonmodule_some_requires_exact_registration_descriptor_and_name
analysis_bsl_owner_module_some_requires_exact_registration_descriptor_kind_and_name
analysis_bsl_registered_form_some_reuses_exact_registered_owner_descriptor_authority
analysis_bsl_registration_plan_must_match_fingerprinted_configuration_and_descriptor_leaves
analysis_bsl_commonmodule_and_owner_filesystem_decoys_never_enter_manifest_or_index
analysis_bsl_internal_plan_rejects_owner_descendant_without_registration
analysis_bsl_registered_owner_missing_or_wrong_descriptor_is_capture_fatal_not_none
analysis_bsl_unsupported_root_commonform_nested_command_future_and_bad_identifier_map_none
analysis_bsl_registered_form_requires_exact_capture_relationship_and_never_maps_none
analysis_bsl_unclaimed_form_shaped_ordinary_row_cannot_promote_itself
analysis_bsl_path_and_artifact_case_alias_pairs_reject_in_both_orders
derived_key_4096_bytes_passes_and_4097_fails
derived_key_revalidates_contained_path
capture_and_expectation_views_return_only_opaque_key_refs
opaque_key_ref_projection_has_no_raw_string_bytes_path_or_components
relationship_expected_key_casefold_and_alias_collisions_fail
exact_expected_key_may_bind_only_its_equal_present_entry
first_missing_ext_form_and_final_components_each_prove_missing
symlink_reparse_special_not_directory_permission_and_alias_do_not_prove_missing
foreign_snapshot_form_and_kind_handles_fail_before_io
equal_reconstructed_snapshot_authority_is_not_pointer_rejected
projection_resolver_equal_reconstructed_semantic_snapshot_succeeds
isolated_expectation_cap_400000_passes_and_400001_fails
single_source_disjoint_399996_boundary_passes
overlapping_source_expectations_sum_checked_to_global_cap
captured_bsl_scan_item_count_400000_passes_and_400001_fails_checked
present_material_counts_once_in_global_unique_file_and_byte_budget
missing_and_not_applicable_count_zero_files_and_bytes
deadline_is_checked_during_catalog_and_absence_loops
```

### 14.4 Capture races and verified/provider reads

```text
initial_missing_final_present_discards_whole_snapshot
initial_present_final_missing_discards_whole_snapshot
form_type_or_span_mutation_discards_whole_snapshot
registration_rename_or_relationship_swap_discards_whole_snapshot
final_missing_check_rejects_new_ancestor_or_leaf
capture_drift_uses_source_changed_not_source_fingerprint_mismatch
post_capture_missing_appearance_returns_source_fingerprint_mismatch
post_capture_present_change_returns_source_fingerprint_mismatch
registered_material_handle_mismatch_is_nonretryable_and_pre_io
projection_resolver_cross_source_fingerprint_form_kind_and_state_mismatch_is_pre_io
projection_resolver_has_zero_verifier_filesystem_read_and_parse_calls
projection_resolver_200000_rows_uses_one_btreemap_get_and_zero_iterators
descriptor_handle_mismatch_is_nonretryable_and_pre_io
descriptor_reader_200000_forms_uses_one_captured_form_get_and_zero_manifest_or_form_iterators_per_call
descriptor_internal_key_envelope_or_ordinary_entry_disagreement_is_handle_mismatch_pre_io
descriptor_reader_delegates_exactly_once_to_ordinary_present_verified_read
descriptor_reader_change_returns_source_fingerprint_mismatch
descriptor_verified_wrapper_exposes_only_bytes
present_verified_wrapper_prevents_cross_form_or_kind_replay
material_verified_wrappers_expose_no_raw_or_projection_payload
public_context_reads_and_parses_each_form_descriptor_exactly_once
public_context_has_zero_dynamic_material_verifier_and_probe_calls
private_from_prepared_is_not_a_second_public_port
query_constructors_have_zero_all_io_spies
on_demand_not_applicable_has_0_0_0_material_spies
on_demand_missing_has_1_verifier_0_byte_0_parse_spies
on_demand_present_has_1_verifier_1_byte_and_conditional_parse_spies
repeated_relationship_demand_is_deduplicated
present_verified_read_reuses_equal_ordinary_manifest_authority
captured_ordinary_bsl_reader_delegates_once_to_ordinary_verified_read
claimed_registered_form_module_present_replaces_ordinary_scan_item_exactly_once
claimed_registered_form_module_never_calls_both_ordinary_and_registered_readers
captured_bsl_index_is_owned_in_snapshot_and_handles_borrow_stored_entries
captured_bsl_index_rebuild_matches_complete_backing_grammar_at_snapshot_acceptance
captured_bsl_index_drift_is_handle_mismatch_before_io
captured_bsl_item_reads_validate_one_entry_without_full_index_rebuild
registered_form_module_missing_and_not_applicable_keep_canonical_virtual_slots
ordinary_unsupported_bsl_remains_one_visible_scan_item
captured_bsl_module_projection_is_typed_some_for_supported_and_registered_none_only_for_unsupported
captured_bsl_module_projection_rejects_duplicate_or_case_alias_equal_artifact_refs
unregistered_form_shaped_decoy_outside_capture_is_not_a_scan_item
merged_bsl_scan_order_is_identical_under_manifest_and_relationship_permutation
verified_bsl_location_range_is_reproducible_for_equal_bytes_and_offsets
verified_bsl_location_rejects_empty_reversed_out_of_range_and_nonboundary_ranges
missing_not_applicable_and_unsupported_items_have_opaque_diagnostic_locations
verified_bsl_cache_locator_exists_only_after_present_verified_read
registered_form_xml_cache_locator_is_handle_mismatch_without_panic_or_io
raw_path_readers_cannot_authorize_dynamic_missing
internal_handle_projection_state_key_and_ordinary_entry_disagreement_is_pre_io_handle_mismatch
source_fingerprint_mismatch_is_only_external_filesystem_drift_after_semantic_validation
```

### 14.5 Static ownership

```text
managed_form_suffix_literals_exist_in_one_task4_module_only
analysis_bsl_projection_registry_and_path_tokens_exist_in_one_task4_module_only
task5b_task6_task7_task8_have_no_bsl_filename_directory_or_artifact_formatter
task5b_and_task6_import_no_analysis_bsl_projection_registry_or_version_branch
source_manifest_v2_has_no_raw_map_constructor_serde_or_field_mutation
ordinary_bsl_projector_requires_accepted_manifest_not_bare_manifest_key
no_provider_or_task8_artifact_to_form_path_formatter
no_raw_serde_or_string_manifest_relationship_constructor
no_raw_serde_string_path_or_generic_key_projection_constructor
no_raw_tuple_or_generic_relationship_projection_constructor
projection_resolver_returns_only_task4_handle_by_value
projection_resolver_has_no_linear_scan_or_raw_key_escape
specialized_readers_are_object_safe_source_snapshot_port_methods
no_free_registered_reader_function_or_hidden_global_io_capability
semantic_snapshots_handles_projections_and_results_contain_no_reader_capability
captured_bsl_index_owns_no_manifest_borrow_pointer_or_temporary_entry
captured_bsl_iterator_borrows_only_snapshot_stored_index_entries
specialized_reader_spies_and_counters_belong_to_injected_source_snapshot_port
neutral_platform_xml_has_no_task4_or_task5b_dependency
neutral_capture_and_digest_authorities_clone_losslessly_without_conversion
composite_snapshot_id_has_no_raw_slice_array_or_serde_ingress_and_no_string_or_digest_constructor
composite_raw_writer_reserves_then_has_exactly_one_task5b_production_caller
composite_raw_writer_cannot_be_aliased_reexported_or_taken_as_function_pointer
task7_never_imports_or_calls_the_composite_raw_writer
no_task_local_form_type_authority_alias_or_parser
task5b_imports_task4_capture_and_material_handles_without_field_duplicates
captured_private_struct_never_crosses_task4_boundary
task5b_descriptor_reads_use_only_handle_bound_task4_reader
task5b_descriptor_and_material_reads_use_only_injected_source_snapshot_port_methods
task5b_key_wrapper_contains_only_task4_owned_projection_token
task5b_key_identity_encoding_delegates_to_task4
task5b_catalog_key_string_encoding_delegates_to_task4_without_local_framing
task5b_registered_form_271_byte_entry_golden_is_unchanged_by_projection_seam
task4_has_no_task5b_projection_or_catalog_dependency
public_form_handle_material_api_is_self_resolving
task5b_private_resolver_is_the_only_projection_resolver_consumer
task6_never_receives_or_resolves_relationship_projection
task6_cannot_name_captured_analysis_bsl_handle_or_location_ref
task6_cannot_call_captured_analysis_bsl_module_projection
captured_bsl_admission_byte_length_is_some_for_both_present_kinds_and_none_for_missing_not_applicable
claimed_present_formmodule_admission_length_equals_its_equal_ordinary_manifest_entry
task6_cannot_call_captured_analysis_bsl_admission_byte_length
captured_bsl_location_ref_converts_only_to_opaque_verified_location
task6_receives_no_bsl_manifest_key_path_or_raw_task4_reader_method
verified_bsl_location_and_cache_capabilities_have_no_raw_accessor_or_serde
only_task5b_context_dispatches_captured_ordinary_and_registered_bsl_readers
platform_catalog_context_retains_borrowed_opaque_authority_only
owner_header_status_is_external_ledger_only_and_freeze_invariant
```

Every RED is first observed failing against the live v1 implementation and then
kept permanently. Race/deadline tests use injected hooks and clocks, never
sleeps. Copied prose constants without production-byte reconstruction do not
satisfy encoder REDs.

## 15. Co-freeze and implementation sequence

Design STOP gates:

1. reconcile the four candidate documents to this exact type grammar, handle
   envelope, 400,000 global cap, 4,096 live-bound interpretation, first-absent
   proof, demand-scoped I/O table, descriptor semantic-view seam, indexed
   projection-to-handle resolver, closed manifest-BSL projection registry, and
   error split;
2. make all four design bytes immutable and obtain fresh self-audits plus
   independent cross-reviews against those exact bytes; and
3. accept the four-identity candidate tuple atomically in one ledger operation.

No standalone Task 4 hash or review can satisfy those gates.

After co-freeze, production proceeds sequentially:

1. write domain/encoder/parser/path/bounds REDs, including mechanical goldens
   and compile-fail constructor/dependency tests;
2. introduce private manifest-key/leaf/handle/expectation types while retaining
   ordinary-entry v1 framing inside source v2;
3. extend the neutral parser and capture the descriptor-bound root/properties/
   FormType envelope;
4. add exact two-row totality and the Task 4-only Managed serializer;
5. add the source-bound capture handle, read-only capture/neutral views, opaque
   key ref/projection and Task4-owned identity encoder;
6. replace composite capture output with exact `SourceSnapshotV2` while keeping
   every byte reader atomic on `SourceSetSnapshotV2`; add canonical destination
   validation, prove diagnostic epoch exclusion, and add the sole sealed raw
   composite-digest writer with its exact Task5B binding call-site whitelist;
7. extend the existing object-safe `SourceSnapshotPort` with all three
   handle-bound reader methods, add descriptor/material/captured-BSL wrappers,
   opaque receipt/cache location capabilities, and the Unix/Windows
   first-absent primitives;
8. build the checked captured-BSL projection, replace claimed Present
   FormModules rather than duplicating them, and freeze its total virtual-anchor
   order plus the section-3.1 closed root/directory/module/Form grammar without
   adding fingerprint fields;
9. integrate initial/final plan comparison, final Present rereads, and final
   Missing proofs under existing hooks/budgets/error mapping;
10. migrate every source/composite/query/baseline consumer atomically to v2 and
   delete v1 acceptance instead of adding fallback;
11. compile-test the future Task 5B consumer with imported handles/views only,
   no duplicated private fields or raw key/path access;
12. synchronize active spec/ADR/product contracts in the implementation slice;
13. run focused/full/fmt/clippy/product/Windows compile checks; and
14. obtain independent implementation review and record exact commands/results
    and accepted production OID in a successor report.

STOP if a provider derives a suffix, Task6 scans ordinary and registered BSL in
separate orders, a claimed Present FormModule remains visible twice, Task6
receives a raw manifest/cache/location path or captured Task4 BSL handle,
Task5B/6 recognizes a BSL filename/directory or formats a module ref, a newly
registered metadata directory silently enlarges projection v1, an exact-case or
Unicode alias chooses a winner, an unclaimed Form-shaped path promotes itself,
Missing is inferred after capture, a
missing parent is rejected as non-Missing, NotApplicable carries a path,
FormType gets a second task-local authority, Task 5B duplicates handle fields or
obtains a raw key/path, context construction performs I/O other than its exact
one-per-Form injected-port handle-bound descriptor read/parse, either specialized
reader is a free/global function or a semantic snapshot/handle stores reader
capability, descriptor validation scans the manifest/Forms, query construction
performs any I/O, an atomic `SourceSetSnapshotV2` is passed where the composite
`SourceSnapshotV2` is required, v1/v2 are dual-accepted, 399,996 is treated as the global cap,
or a partial expectation prefix survives any error/bound. STOP if
`CompositeSnapshotIdV2` exposes raw bytes, cross-owner string/transport or serde
ingress, if any production caller
other than `PlatformCatalogExecutionBindingV1::write_identity_v1` reaches its
sealed raw writer, or if that caller adds/removes/reorders bytes instead of
placing the exact 32-byte digest first in the Task5B-owned 96-byte binding.

## 16. Non-goals

This design does not implement Platform XML evidence, BSL parsing, Task 7
association/admission, Task 8 mutation planning, receipts, public MCP wire
changes, EDT dynamic materials, a generic optional-path registry, or a
filesystem watcher. It supplies only the immutable Task 4 capture/read authority
needed by those later layers. The sealed raw composite writer is only an
owner-controlled identity projection for Task5B's opaque execution binding; it
does not implement that binding, expose snapshot bytes or add a Task4-to-Task5B
dependency.
