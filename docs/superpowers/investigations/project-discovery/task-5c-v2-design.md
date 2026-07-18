# Task 5C v2 — production ParentConfigurations authority and support mutation

Status: **superseded combined working history; zero prerequisite,
implementation, acceptance, or hash authority**.

This ignored combined draft does not edit or weaken the immutable Task 5C v1
design or its root pre-review notes. It must never be frozen, hashed, cited as a
prerequisite, or implemented as one artifact: doing so recreates the Evidence
before Task8 versus whole-file-after-Task8 cycle. The read-only candidate moved
to `.superpowers/sdd/task-5c-evidence-v2-design.md`; a mutation addendum is a
separate future artifact only after its writer/receipt prerequisites exist.
Everything below is historical working material and has no normative force.

## 0. Acceptance dependency ledger

Task 5C has two independently reviewable, ordered deliveries:

1. **5C-Evidence** — the pure parser, shared snapshot authority, discovery
   provider, projection, renderers, and fail-closed assessment;
2. **5C-Mutation** — `unica.support.edit` and the authoritative guard window,
   both using the accepted generic artifact writer/lease/WAL/outcome pipeline.

Neither delivery may be called final from a dirty tree or an ignored design
alone. The dependency graph is deliberately acyclic:

```text
accepted Task5A -> accepted Task5B -> accepted Task7-v6 design
  -> accepted Task5C-Evidence commit -> accepted Task6/7 implementation
  -> accepted Task8 -> accepted Task9 -> accepted Task10
  -> accepted Task5C-Mutation commit
  -> final Task5C report naming both Task5C commits
```

Task 6, Task 7, and Task 8 must name the exact
`TASK5C_EVIDENCE_ACCEPTED_GIT_OID`, not `TASK5C_ACCEPTED_SHA`, “Task5C v2
SHAs”, or a vague “GREEN Task 5C”. The former is one 40-lowercase-hex Git
object for the read-only Evidence slice; every whole-Task5C spelling would
silently make those upstream tasks wait for the downstream Mutation slice and
recreate the cycle this ledger removes.
Task5C-Evidence never claims dependency on Task8/9/10. The final Task 5C report
must record all of these exact authorities:

| Gate | Required immutable authority | Draft state on 2026-07-18 |
| --- | --- | --- |
| Task 5A | accepted 40-hex implementation commit containing typed support facts and the ownership back-propagation in section 6 | absent; STOP |
| Task 5B | accepted v6 contract SHA-256, its independent no-P0/P1 review SHA-256, and accepted 40-hex implementation commit exporting the neutral catalog seam/future-consumer contract | not frozen; STOP; Task8 import/implementation must not gate Task5B acceptance |
| Task 7 | accepted v6 design SHA-256 owning the 4096-subject plan and first lossy evidence admission; its implementation is downstream of 5C-Evidence and upstream of Task8 | not frozen; STOP |
| Task 5C-Evidence | accepted 40-hex commit based on accepted Task5A/5B and its independent no-P0/P1 review SHA-256 | not implemented; Task8 prerequisite |
| Task 8 | accepted v6 design SHA-256, its independent no-P0/P1 review SHA-256, and accepted 40-hex implementation commit exporting the generic writer seam in section 9 and naming `TASK5C_EVIDENCE_ACCEPTED_GIT_OID` | not frozen; STOP for 5C-Mutation |
| Task 9 | accepted v6 addendum SHA-256 and accepted implementation commit for schema-v3 persistence and writer-WAL correlation | artifact does not yet exist; STOP |
| Task 10 | accepted v6 addendum SHA-256 and accepted implementation commit for under-lease policy revalidation, outcome reconciliation, and receipt transitions | artifact does not yet exist; STOP |

There are intentionally no fake `<pending>` hashes in this draft. A branch
name, `HEAD`, prior v5 design, file mtime, or self-audit hash cannot satisfy a
row. 5C-Evidence may be implemented only after accepted Task 5A and Task 5B
implementation commits plus accepted Task7-v6 design exist. It does not wait
for Task7 implementation, which consumes its support provider/group contract.
5C-Mutation and the production guard integration
remain hard STOP until every downstream row, including Task 8/9/10, is
accepted. The Task5C-Mutation commit must be based on those exact commits;
Task5C-Evidence instead precedes them. Rebasing either slice across one of its
dependencies requires rerunning that slice's full cross-review and recording
new hashes.

Task 5B acceptance proves and exports the neutral v2 catalog seam plus static
future-consumer constraints. It does **not** require an actual Task 8 import,
Task 8 test, or Task 8 implementation commit: that would add the hidden reverse
edge `Task8 -> Task5B` to `Task5B -> Task7 -> Task8`. Task 6/7 implementation
may require only the accepted Task5C-Evidence Git OID, never the final
Task5C-Mutation OID or a whole-Task5C aggregate.

The transitional Task5C-Evidence commit deletes/disables the unsafe legacy
`fs::write` implementation of applied `unica.support.edit`: applied calls return
`support_edit_atomic_writer_required` with zero writes until Task5C-Mutation
lands; dry-run may show a non-authoritative preview. This temporary fail-closed
state prevents the dependency split from leaving the old path writer live.
Task5C-Evidence may expose the typed guard assessment, but the authoritative
under-lease applied window and every claim of serializable guard safety belong
to Task5C-Mutation after Task8/9/10.

This draft itself must not receive a published SHA-256 or self-audit until the
Task 8 v6 artifact is frozen and its exported type/reason names are imported
verbatim. After that event, section 17 defines the freeze procedure.

## 1. Executive decision

The production boundary is one strict, bounded byte parser and one neutral
Platform XML catalog authority. Every consumer receives an explicit typed
result; no consumer is allowed to infer safety from parser silence, file size,
a source-set label, display text, or another adapter.

The v2 decisions that correct v1 are:

1. The sole provenance-backed serialized shape is a BOM-prefixed, exactly one-
   vendor, fully consumed `6` document. BOM-less input is unsupported. Zero or
   multiple vendors are unsupported. Every duplicate object UUID is rejected;
   identical duplicates are not deduplicated.
2. Global flag `1`, vendor editing flag `1`, and object rule `1` are
   current-product compatibility contracts, not Designer-fixture proof. Either
   global or vendor editing flag `1` is a fail-safe read-only conclusion. Tests
   and documentation must label the provenance grade
   honestly; no synthetic helper may be cited as an exported fixture.
3. `ExtensionWithoutParentConfigurations` says only that the support-policy
   leaf is absent in one exact extension snapshot. It never proves that a
   target exists or belongs to that extension. The old direct projection to
   `ExtensionOwned` is removed.
4. Metadata and support providers consume the same borrowed
   `PlatformConfigurationCatalogV1` instance for each source snapshot. The
   support adapter never invokes MetadataCatalog, decodes provider evidence, or
   runs a second local-name parser.
5. `unica.support.edit` is not a special path writer. It resolves one canonical
   artifact, acquires the accepted Task 8 root-wide artifact lease, recovers its
   WAL, rereads and reparses policy under that witness, and calls the accepted
   control-staged atomic writer. There is no `canonicalize`/`exists`/`stat` then
   `fs::write` fallback.
6. An early support guard may resolve mode, target, and routing only. Its
   authoritative read occurs after artifact-WAL recovery under the same
   root-wide lease used by the mutation, and that witness stays alive through
   handler outcome, current-receipt handoff when one exists, and the
   authoritative post snapshot. Other-receipt reconciliation starts only after
   current receipt and artifact locks are released. A second artifact lease is
   forbidden.
7. Content replacement preserves the accepted non-content metadata policy but
   does not copy a stale pre-edit content timestamp as a post-edit truth. The
   final wording and types are imported from frozen Task 8 v6; Task 5C does not
   invent a divergent timestamp policy.

## 2. Closed scope

### 2.1 In scope

- a pure parser for the one-vendor v2 subset;
- exact problem/retry classes and stable reason vocabulary;
- one snapshot-built Platform XML catalog set shared by Metadata and Support;
- a snapshot-bound support provider for base and extension sources;
- application-owned ownership/support projection for direct candidates and
  the current `CfePatchMethod` intent;
- one contained live read boundary for legacy information and guard tools;
- explicit `Safe | Violation | Indeterminate` support assessment;
- typed renderer output for missing, known, unsupported, malformed, and
  unavailable states;
- typed `ResolvedSupportEditPlanV1`, exact byte-span mutation, reparse validation, and
  Task 8 writer/WAL/outcome reuse;
- Task 9/10 integration so support edits invalidate stale receipt baselines and
  support policy is revalidated under the writer witness;
- exact fixture provenance, RED/GREEN, cross-platform, package-contract, and
  documentation gates.

### 2.2 Explicitly out of scope

- guessing explicit present-file “not under support” bytes;
- zero-vendor or multi-vendor composition;
- duplicate/per-vendor aggregation rules;
- BOM-less, UTF-16, compressed, encrypted, or opaque binary containers;
- nested Form/Template destructive authorization policy without a real
  owner-plus-child exported fixture and a product decision;
- changing `ParentConfigurations.bin` by a standalone temp/rename helper;
- treating Task 8's cooperative writer as CAS against arbitrary Designer or
  third-party writers;
- adding a second configuration-flavor, UUID, membership, or MDClasses parser;
- parsing human-facing renderer text back into discovery or guard authority.

## 3. Evidence and compatibility provenance

### 3.1 Exported fixture authority

The three tracked `ParentConfigurations.bin` files are one semantic example,
not three independent format witnesses. They are byte-identical, 337 bytes,
SHA-256
`6750bbf0b567b5bf475ee8a3b2b00c5391dba487358cf05c47c77c07e01e90e3`,
strict UTF-8 with exactly one BOM and no final newline. They prove only:

- marker `6`;
- global flag `0` as the current enabled-editing layout;
- one vendor record with vendor flag `0`;
- three object records;
- object flags `0` (`Locked`) and `2` (`Removed`);
- one record without a mirrored UUID and two records with a same-value mirror.

The implementation copies these bytes once to
`tests/fixtures/project_discovery/parent_configurations/exported_one_vendor_v1.bin`
and records a provenance manifest containing all three source paths and the
digest above. A test proves all original paths equal the canonical fixture.

### 3.2 Compatibility-only authority

Current Rust product tests synthesize:

- global flag `1` by text replacement;
- object flag `1` (`Editable`);
- the mirrored-UUID form for the first object record;
- `Capability=off` output where global, vendor, and every object flag become
  `1`.

These rows are adopted as an explicit versioned product compatibility contract
because current public tools create/consume them. They are never described as
Designer-exported evidence. The parsed document records:

```text
SupportSerializationAuthorityV2 =
  ExactTrackedExportV1        // exact canonical 337-byte content digest
  | AcceptedCompatibilityV2   // every other accepted document/profile value
```

The classification is deterministic: only exact content digest
`6750bbf0b567b5bf475ee8a3b2b00c5391dba487358cf05c47c77c07e01e90e3`
receives tag 1; every other accepted document receives tag 2.
Thus a generalized grammar or synthetic helper is never mislabeled as an
exported artifact. The grade is diagnostic/provenance, not a hidden
authorization override:

- effective editing is enabled only for `(global, vendor) = (0, 0)`;
  `(0,1)`, `(1,0)`, and `(1,1)` all map to
  `ConfigurationReadOnly`, regardless of object rules, and therefore cannot
  open a mutation;
- object `1` maps to `Editable` only when both enclosing flags are `0` and the
  complete one-vendor document is otherwise accepted;
- a product/reviewer may later require real exported proof before receipt-grade
  `Editable`; that change is a new contract version, not a reinterpretation of
  v2 bytes.

### 3.3 Unsupported is not corrupt

An unsupported shape is not accused of being invalid 1C data. It means Unica
does not have authority to interpret it. Both malformed and unsupported present
files produce no support fact and fail closed, but diagnostics preserve the
difference.

## 4. Pure byte parser

Create
`crates/unica-coder/src/infrastructure/parent_configurations/{mod,lexer,parser,encoder}.rs`.
The module has no filesystem, snapshot, renderer, guard, adapter, or operation
dependency.

### 4.1 Bounds

All arithmetic is checked before allocation:

```text
MAX_PARENT_CONFIGURATIONS_BYTES          = 67_108_864   // 64 MiB
MAX_PARENT_CONFIGURATIONS_TOKENS         = 1_000_000
MAX_PARENT_CONFIGURATIONS_OBJECT_RULES   = 200_000
MAX_PARENT_CONFIGURATIONS_QUOTED_BYTES   = 4_096 per scalar
PARENT_CONFIGURATIONS_VENDOR_COUNT_V2    = 1
```

There is no wall-clock-dependent partial result. Reaching any bound returns one
non-retryable `parent_configurations_resource_limit`; no parsed prefix, edit
span, or support fact survives.

### 4.2 Exact framing and lexical rules

Accepted input begins with exactly bytes `EF BB BF`, then `{`, and ends with
the matching `}` at EOF. There is no leading/trailing whitespace or final
newline. A second BOM, missing BOM, embedded NUL, invalid UTF-8, control byte,
unbalanced brace/quote, escape sequence not represented by the proven subset,
or semantic byte after `}` is rejected.

Outside quoted scalars, only ASCII digits, lowercase canonical UUID bytes,
comma, `{`, and `}` are accepted. Numeric tokens are canonical decimal: `0` is
the only zero spelling and nonzero values have no leading zero. UUIDs are exact
36-byte, lowercase, hyphenated, non-nil `PlatformUuid` spellings. The neutral
domain UUID type remains the identity owner; the byte parser adds the lowercase
serialization restriction.

Quoted vendor version/vendor/configuration labels are strict UTF-8, nonempty,
at most 4096 bytes, and contain no quote, backslash, NUL, Unicode control, or
unpaired encoding. Commas/braces inside a quoted scalar remain scalar bytes and
cannot create a record. V2 defines no escaping syntax.

### 4.3 Accepted one-vendor grammar

The fully consumed semantic token sequence is:

```text
Document := BOM "{" 6 "," Global "," 1 "," Vendor "," Rules "}"

Global := 0 | 1

Vendor :=
  vendor_configuration_uuid ","
  vendor_editing_flag        ","       // 0 | 1
  vendor_instance_uuid       ","
  quoted_version             ","
  quoted_vendor              ","
  quoted_configuration       ","
  object_rule_count

Rules := exactly object_rule_count Record values, object_rule_count >= 1

Record :=
  rule_flag "," 0 "," object_uuid [ "," mirrored_object_uuid ]

rule_flag := 0 | 1 | 2
```

The optional mirror is recognized only by its complete UUID token shape and,
when present, must equal `object_uuid`. This accepts the two shapes observed in
the exported fixture/current product helper without treating the mirror as a
second rule. A malformed or unequal mirror is rejected.

Every `object_uuid` must be unique. Repeated equal rule is
`duplicate_parent_configuration_rule`; repeated distinct rule is
`conflicting_parent_configuration_rules`. Both reject the whole document.
Declared count must equal the exact number of records and at most 200,000.

Marker other than `6`, vendor count `0` or `>1`, and another structurally valid
field shape are unsupported. An out-of-domain flag is malformed. An otherwise
well-formed extra token is trailing material. The parser does not guess a newer
grammar.

Rejection precedence is fixed so one byte string cannot change reason by
branch/order: byte/resource ceiling; BOM/framing; strict UTF-8/NUL/control;
lexical truncation; marker/vendor-count variant; scalar/flag/UUID/record shape;
mirror; duplicate/conflict; declared count; trailing material. Within one
stage the lowest byte offset wins, then stable reason tag. Empty/no-BOM input is
`unsupported_parent_configurations_framing`; a proper BOM followed by an
incomplete document is `truncated_parent_configurations`.

`GlobalEditingV2` is derived from the **pair** of enclosing flags, not the
global byte alone:

```text
(global=0, vendor=0) -> Enabled                 tag 1
(global=0, vendor=1) -> ReadOnlyVendor          tag 2
(global=1, vendor=0) -> ReadOnlyGlobal          tag 3
(global=1, vendor=1) -> ReadOnlyGlobalAndVendor tag 4
```

Only tag 1 permits per-object projection. Tags 2..4 all emit
`ConfigurationReadOnly`. The exact two raw flag bytes and the derived tag are
encoded. No contradictory combination is normalized away during a read.
`Capability=on|off` has a separate explicit normalization contract in section
9; a read never rewrites merely to make the flags agree.

### 4.4 Typed result and immutable spans

```rust
pub(crate) enum ParentConfigurationsParseV2 {
    Parsed(ParentConfigurationsDocumentV2),
    Rejected(ParentConfigurationsProblemV2),
}

pub(crate) enum ParentConfigurationsProblemClassV2 {
    Malformed,      // tag 1
    Unsupported,    // tag 2
    ResourceLimit,  // tag 3
}

pub(crate) struct ParentConfigurationsDocumentV2 {
    framing: ParentConfigurationsFramingV2, // Utf8Bom, tag 1 only
    authority: SupportSerializationAuthorityV2,
    global_editing: GlobalEditingV2,
    vendor: ParentConfigurationVendorV2,
    rules: std::collections::BTreeMap<PlatformUuid, ParentConfigurationRuleV2>,
    serialized_order: Vec<PlatformUuid>,
    edit_spans: ParentConfigurationsEditSpansV2,
    semantic_digest: Digest32,
    skeleton_digest: Digest32,
    content_digest: Digest32,
}
```

Every field is private and smart-constructed from one immutable byte buffer.
Edit spans are sorted, disjoint, in bounds, and each slices exactly one ASCII
flag byte from that buffer. They include the global flag, vendor flag, and one
rule flag per record. UUID mirror spans are never mutable. `serialized_order`
is retained only to rewrite exact original bytes; semantic rule lookup uses the
unique canonical map.

`semantic_digest` represents the fully parsed document in serialized record
order; v2 does not guess that vendor rule order is semantically irrelevant.
The separate unique `BTreeMap` is only exact UUID lookup.
`skeleton_digest` hashes the exact source bytes after replacing every admitted
mutable flag byte with a domain-specific sentinel; it therefore binds BOM,
field framing, labels, UUIDs, mirrors, and record order without binding current
flag values. `content_digest` hashes the exact original bytes. No span or
digest is constructible from caller-provided offsets/text.

### 4.5 Root-rule semantic validation

The pure parser does not know which UUID is the source configuration root.
Provider/live assessment receives that exact root UUID from the shared
`PlatformConfigurationCatalogV1`. A parsed under-support document missing the
known root UUID is rejected at the semantic boundary as
`support_configuration_root_rule_missing`; it is never `ObjectNotListed` for
the root. A registered child UUID absent from the complete map may become
`ObjectNotListed`.

## 5. One neutral Platform XML authority

Task 5B v6 owns the exact MDClasses namespace and freezes:

```text
PLATFORM_CONFIGURATION_CATALOG = "platform-configuration-catalog/v1"
PLATFORM_CONFIGURATION_CATALOG_ENCODER =
  "platform-configuration-catalog-encoder/v1"
PLATFORM_CONFIGURATION_CATALOG_SET =
  "platform-configuration-catalog-set/v1"
PLATFORM_CONFIGURATION_OBJECT_KEY_ENCODER =
  "platform-configuration-object-key/v1"
PlatformConfigurationCatalogV1
PlatformConfigurationObjectAuthorityV1
PlatformConfigurationCatalogSetV1
PlatformConfigurationObjectKeyV1
PlatformConfigurationCatalogPort
```

Task 5C imports this exact single-build/shared-input port and set rather than
redeclaring either type or choosing independent adapter reparsing. The accepted
set contains `contract_version`, exact `composite_snapshot_id`, and catalogs
sorted Analysis first then Destinations by complete `AtomicSourceIdentityV2`;
its digest is the accepted Task 5B encoder over composite snapshot identity and
catalog digests.

The application invokes this port exactly once after the composite snapshot is
captured and before MetadataCatalog/Form/Support providers. It stores a borrow
in `EvidenceExecutionContext`. MetadataCatalog and SupportState receive the
same object addresses and catalog digests. Recording fakes fail if a consumer
receives a clone rebuilt from another parse run, even if the visible fields are
equal.

The set constructor proves:

- exact `ResolvedSourceSet` value equality with every captured source;
- exact source fingerprint equality;
- exact Task 5B capture-catalog digest;
- one analysis role and every destination exactly once in canonical
  `AtomicSourceIdentityV2` order;
- no mixed capture envelopes or source-kind repair;
- exact `MDCLASSES_NS`, semantic flavor, UUID, and Own/Adopted membership from
  Task 5B's neutral builder.

SupportState receives only a source catalog and exact requested entries from
that catalog, except for the typed planned-absence authority in section 6.1.
An untyped/missing entry is `support_subject_unresolved`, never
`ObjectNotListed`. An inconclusive flavor/membership is a scoped support gap.

Static product tests reject all of these dependencies from the Support module:

- `PlatformXmlMetadataCatalogProvider` or `MetadataCatalogPort` invocation;
- `ProviderFact`, evidence record, display, JSON, or renderer parsing;
- local-name-only XML helpers;
- another `Configuration.xml`/descriptor parser;
- caller-supplied source kind used as semantic flavor.

Task 5C does not rename or wrap that authority with a second encoder.

## 6. Snapshot-bound support provider

Create
`crates/unica-coder/src/infrastructure/discovery/support.rs` with provider
identity:

```text
provider = unica.support_state / 2
parser   = parent-configurations/v2
reasons  = parent-configurations-reasons/v2
```

### 6.1 Query contract

Task 5A's `SupportQueryPlan` is versioned to carry exact typed authority rather
than bare names:

```rust
pub(crate) struct SupportQuerySubjectV2 {
    source: AtomicSourceIdentityV2,
    subject: SourceScopedArtifact,
    authority: SupportSubjectAuthorityV2,
}

pub(crate) enum SupportSubjectAuthorityV2 {
    Existing(PlatformConfigurationObjectKeyV1),
    PlannedDestinationAbsent(PlannedDestinationAbsentV1),
}

pub(crate) struct PlannedDestinationAbsentV1 {
    pair: DestinationMembershipPair,
    analysis_authority: AnalysisMetadataAuthorityObservationV1,
    destination_absence: DestinationMetadataMembershipObservationV1,
    analysis_fact_digest: Digest32,
    destination_fact_digest: Digest32,
    analysis_source_fingerprint: Digest32,
    destination_source_fingerprint: Digest32,
    provenance_evidence_ids: Vec<EvidenceId>,
    catalog_set_digest: Digest32,
}

pub(crate) struct SupportAuthorityIndexV2 {
    by_subject: BTreeMap<SourceScopedArtifact, SupportSubjectAuthorityV2>,
    index_digest: Digest32,
}

pub(crate) struct SupportSourceGroupV2 {
    source: AtomicSourceIdentityV2,
    catalog_digest: Digest32,
    configuration_root_key: PlatformConfigurationObjectKeyV1,
    subjects: Vec<SupportQuerySubjectV2>,
}

pub(crate) struct SupportQueryPlanV2 {
    discovery: DiscoveryQueryPlan,
    catalog_set_digest: Digest32,
    groups: Vec<SupportSourceGroupV2>,
}
```

Private constructors resolve each key against the exact borrowed
`PlatformConfigurationCatalogSetV1` before provider I/O. Groups sort by
`AtomicSourceIdentityV2`; subjects sort by exact source-scoped artifact bytes.
The accepted Task 7 v6 bound is imported exactly as
`MAX_SUPPORT_QUERY_SUBJECTS = 4096` over the union of all group subjects. It is
independent of public `maxEvidence`; 4097 subjects reject the plan before
provider I/O, while 4096 are evaluated in full. A source group is evaluated once: one optional
ParentConfigurations read/parse produces every requested subject conclusion.

Task 5C imports Task 5B's exact
`PLATFORM_CONFIGURATION_OBJECT_KEY_ENCODER =
"platform-configuration-object-key/v1"` and
`PlatformConfigurationObjectKeyV1 { catalog_digest, artifact }`. Its bytes are
`digest32(catalog_digest) || ArtifactIdentityBytesV1(artifact)`; it is resolved
against exactly one catalog/one canonical artifact and callers cannot construct
it from a display name. The key does not enter the catalog payload and therefore
creates no digest cycle.

`PlannedDestinationAbsentV1` is the only non-present **query** authority. Its
constructor requires the already-validated Task 5B `DestinationMembershipPair`,
one exact present `AnalysisMetadataAuthorityObservationV1` whose
flavor/membership is BaseConfiguration+Own, and one exact
`DestinationMetadataMembershipObservationV1` whose state is Absent for the
same destination source/artifact. The destination catalog must be a known
ExtensionConfiguration, the pair must belong to the current composite catalog
set, and every source/fingerprint/pair/fact digest must match. The constructor
accepts the already freshness-validated preliminary raw Metadata records,
recomputes their semantic fact digests, compares each fingerprint to its own
catalog, and retains every sorted/unique nonempty provenance evidence ID
(bounded by the accepted evidence scope). A name, caller UUID, declared
extension kind, stale fact, or generic MetadataAbsent cannot construct it.

This preliminary authority permits the provider to ask what the future
destination policy would say about the analysis UUID; it is not the final
ownership join. After every Task 7 per-port/global admission rewrite,
`SupportOwnershipAuthorityV2` must be rebuilt from the **retained** Metadata
records and the retained support record, and both fact digests must still equal
the values carried by `PlannedDestinationAbsentV1`. If either Metadata half or
the support group was dropped, only the query/index audit trail remains and the
final projection is `Unknown`. Neither the raw preliminary records nor the
authority index can resurrect a dropped fact.

This authority exists only for an explicit planned `unica.cfe.borrow`/borrow-
required destination. It is never a direct-candidate ownership proof and never
means the artifact already belongs to the extension.

`SupportAuthorityIndexV2` is built with the query plan and lives through
collection/projection. `collect_support_for_snapshot_limited` returns the
provider batch together with this immutable index; each accepted support record
must resolve to exactly one index entry. EvidenceGraph receives both. A support
record without authority or an authority without the exact query subject is a
contract violation. Task 7 loss can drop a support record, but the remaining
index alone never creates a fact or positive projection.

The query digest is:

```text
H("unica.support-query/v2",
  u16be(schema=2) || digest32(catalog_set_digest) ||
  vec(groups: atomic_source_identity_v2 || digest32(catalog_digest) ||
      artifact_identity(configuration_root_key) ||
      vec(subject: source_scoped_artifact || authority_tag ||
          [PlatformConfigurationObjectKeyBytesV1(existing_key) |
           planned_absence_digest])))
```

Authority tags are Existing=1 and PlannedDestinationAbsent=2. The selected
payload is mandatory; no empty/default branch exists.

It contains no absolute path, provider order, display string, epoch, raw source
kind label, or arbitrary caller digest.

### 6.2 Read mapping

For each Platform XML source, the exact manifest key is the versioned optional
leaf `Ext/ParentConfigurations.bin` below that source root. The provider first
reads the immutable manifest entry without filesystem I/O. A declared Present
length above 64 MiB yields `parent_configurations_resource_limit` without
allocating/reading the body. Otherwise it calls the Task 4 bounded form
`read_optional_verified_bounded(..., MAX_PARENT_CONFIGURATIONS_BYTES)` exactly
once per source group. That reader consumes at most declared-length-plus-one
and at most max-plus-one, then proves the exact manifest identity/content; it
cannot allocate the snapshot-wide 4 GiB budget for one support file. Task 4
must add this bounded signature if it is not already accepted; an unbounded
`Vec` read is a STOP.

| Snapshot result | Typed read | Provider result | Retryable |
| --- | --- | --- | --- |
| verified `Ok(None)` tombstone | `Missing` with exact source/catalog authority | complete missing-file fact per resolvable subject | no |
| verified `Ok(Some(bytes))`, accepted v2 | `Parsed` | complete known fact per subject | no |
| present malformed/unsupported/resource-limited | `Malformed`/`Unsupported` | one exact source-scoped Bounded gap, zero support facts for that group | no |
| source content/identity changed | `IoFailure(source_fingerprint_mismatch)` | Unavailable, zero group records | yes |
| snapshot reader unavailable/deadline | exact snapshot reason | Unavailable, zero group records | inherited closed retry bit |
| `NotInManifest` for the declared optional leaf | none | `ProviderContractViolation` | no |
| catalog/source fingerprint mismatch | none | `ProviderContractViolation` | no |

No records from a failed source group survive an unavailable read. Independent
source groups remain independently classifiable only when the accepted Task 5B
atomic-group/result-limit contract admits that isolation; a shared catalog-set
invariant failure fails the whole port.

### 6.3 Raw fact construction

The seven Task 5A fact tags remain append-only and unchanged:

```text
Editable                              tag 1
Locked                                tag 2
ConfigurationReadOnly                 tag 3
Removed                               tag 4
ObjectNotListed                       tag 5
BaseWithoutParentConfigurations       tag 6
ExtensionWithoutParentConfigurations  tag 7
```

Construction is exact:

- a known `BaseConfiguration` catalog plus verified missing leaf emits
  `BaseWithoutParentConfigurations` for each exact registered requested
  subject;
- a known `ExtensionConfiguration` catalog plus verified missing leaf emits
  `ExtensionWithoutParentConfigurations` for each exact registered requested
  subject;
- missing leaf plus inconclusive/contradictory flavor emits no fact and the
  exact flavor gap;
- parsed effective editing state other than `(0,0)` emits
  `ConfigurationReadOnly` for every exact subject;
- parsed `(global=0,vendor=0)` plus exact subject UUID emits its exact
  `Locked|Editable|Removed` rule;
- effective `(global=0,vendor=0)`, exact registered child UUID, and UUID absent
  from the complete map emits `ObjectNotListed`;
- the known configuration-root UUID absent from the map is
  `support_configuration_root_rule_missing`, not `ObjectNotListed`;
- missing catalog entry, missing/invalid UUID, catalog membership
  inconclusive, or source mismatch emits no fact and an exact gap.

For `PlannedDestinationAbsentV1` only:

- verified missing extension policy emits
  `ExtensionWithoutParentConfigurations` scoped to the planned subject;
- parsed effective read-only emits `ConfigurationReadOnly`;
- parsed effective `(0,0)` and absence of `analysis_base_uuid` from the complete
  rule map emits scoped `ObjectNotListed`;
- any rule for that UUID contradicts exact metadata absence and yields
  `support_absent_destination_rule_conflict`, no raw fact;
- it never emits `Editable|Locked|Removed`, and its fact cannot project as
  direct `extension_owned`.

Exhaustive tests prove vendor flag `1` never emits
`ObjectNotListed|Editable|Removed`, regardless of global byte or object rules.

The provider never emits raw `Unknown`, `ExtensionRequired`,
`ExtensionOwned`, a guessed explicit-not-under-support state, or a fact for an
unregistered/absent target **without exact `PlannedDestinationAbsentV1`
authority**. Planned-absence facts are advice-only inputs to RequiresBorrow;
they cannot direct-project to ownership, issue a patch receipt, or construct a
patch plan. Location-distinct witnesses for the same semantic value retain
provenance only after semantic equality is proven.

### 6.4 Atomic groups and first lossy admission

All facts for one `(source, parsed/missing material, subject,
support-subject-authority-digest)` are one
**provider-local** semantic atomic group under the accepted Task 5B/Task 7 v2
group encoder. Parsing, catalog join, root invariant, duplicate checks, and all
group construction finish before loss.

Existing and PlannedDestinationAbsent use distinct authority tags/digests and
can never share a group key even if source/artifact text is equal. The group
retains the exact planned pair/fingerprint binding through application
projection; it is not reconstructed from the returned raw fact.

The Task5C-Evidence provider classifies each Support record using the accepted
Task 5B/Task 7 `StandaloneFact` tag and this exact Support semantic digest:

```text
H("unica.support-evidence-group/v2",
  source_free_provider_fact_digest || support_subject_authority_digest)
```

Task 7 implementation imports the already-landed Task5C-Evidence group value
before its first lossy admission; Task5C-Evidence does not wait for Task 7
implementation. The accepted Task 7 v6 design owns the closed group tag and
first-admission algorithm, while this v2 design owns the Support-specific
semantic payload. This does not make a cross-provider group.

SupportState has **no independent lossy record ceiling** and never emits
`platform_xml_result_limit`. The 4096 query bound is an all-or-nothing planning
gate before provider I/O. Task 7 is the first evidence admission: its per-port
and global limiters admit/drop whole support groups and emit their accepted
closed reasons/sentinels. Metadata ownership companions remain separate
provider groups; loss of either half makes the application join Unknown. Task
5C does not claim cross-provider atomicity and does not create a third limit
reason.

## 7. Ownership-aware application projection

Remove `SupportFactState::direct_projection()`. A raw support observation has no
public projection without exact catalog authority and mutation intent.

Add an application-owned total function:

```rust
fn project_support_v2(
    observation: SupportObservationV2,
    authority: SupportOwnershipAuthorityV2,
    intent: SupportProjectionIntentV2,
) -> SupportProjectionV2;
```

`SupportOwnershipAuthorityV2` can be constructed only from the same accepted
Task 5B facts/catalog values and independently fresh source halves. For final
projection those facts must be present in the post-admission retained record
set, and the support observation must resolve through the immutable authority
index to the same digests. Preliminary/dropped records are ineligible. It never
uses a canonical name match as ownership.

### 7.1 Direct candidate projection

For an exact **base** registered present subject under known
`BaseConfiguration` + `Own`:

| Raw support state | Public state | Authorization meaning |
| --- | --- | --- |
| `Editable` | `editable` | known direct policy permits editing |
| `Locked` | `locked` | direct edit blocked |
| `ConfigurationReadOnly` | `configuration_read_only` | all direct edits blocked |
| `Removed` | `removed` | compatible direct edit may proceed |
| `ObjectNotListed` | `not_under_support` | exact registered child absent from complete rule map |
| `BaseWithoutParentConfigurations` | `not_under_support` | exact verified missing policy leaf |
| extension-only fact or missing/inconclusive authority | `unknown` | blocking/ineligible |

For an exact **extension** registered present subject under known
`ExtensionConfiguration` and exact `Own` or `Adopted` membership in that same
source:

| Raw support state | Public state |
| --- | --- |
| `Locked` | `locked` |
| `ConfigurationReadOnly` | `configuration_read_only` |
| `Editable`, `Removed`, `ObjectNotListed`, `ExtensionWithoutParentConfigurations` | `extension_owned` |
| base-only fact, absent target, flavor/membership gap | `unknown` |

Thus `ExtensionWithoutParentConfigurations` participates only after target
existence/flavor/membership is proven. It is never sufficient by itself.

### 7.2 `CfePatchMethod` proposal projection

The current CFE intent writes one distinct destination extension. It requires:

1. exact known analysis `BaseConfiguration` and `Own` root/optional Form UUIDs;
2. exact known destination `ExtensionConfiguration`;
3. exact destination support policy state;
4. the Task 5B pair join for every required owner;
5. analysis and destination support facts fresh against their own snapshots.

Precedence is total:

1. any malformed/unavailable/gapped analysis or destination support authority
   -> `unknown`, `support_state_inconclusive` plus exact reasons;
2. destination `ConfigurationReadOnly` -> `configuration_read_only`, blocking;
3. destination `Locked` -> `locked`, blocking;
4. destination membership `Own`, wrong adopted UUID, or inconclusive ->
   `unknown` with the exact Task 5B ownership blocker;
5. one or more required destination owners exact Absent, with safe planned-
   absence support authority and no unsafe/inconclusive owner ->
   `extension_required`, `destination_borrow_required`; this is actionable
   advice for a separate `unica.cfe.borrow` intent, but remains receipt-
   ineligible for `unica.cfe.patch_method` and is never implicit borrow;
6. every owner adopted from the exact analysis UUID and destination support is
   one of `Editable|Removed|ObjectNotListed|ExtensionWithout...` ->
   `extension_owned`;
7. no other row exists; a new intent must add an explicit matrix.

Analysis `Locked`/`ConfigurationReadOnly` does not override an exact safe
extension destination because the mutation is not direct. Analysis support
must nevertheless be known; an unknown base policy cannot produce a receipt.
`BaseWithoutParentConfigurations` on analysis does not silently change the
explicit destination intent into a direct edit.

### 7.3 Mandatory collision cases

- identical canonical refs in base and extension remain separate by exact
  source identity and fingerprint;
- a destination source declared `extension` but parsed as Base is Unknown;
- an extension missing ParentConfigurations but missing the target remains
  `ExtensionRequired` or Unknown according to the exact membership polarity,
  never `ExtensionOwned`;
- a same-name destination Own object is Unknown, not owned-from-base;
- adopted destination with another extended UUID is Unknown;
- missing catalog flavor/entry/fingerprint is Unknown and receipt-ineligible;
- no raw fact is treated as graph conflict merely because another source has a
  different state for the same canonical ref.

## 8. Live read, renderers, and support assessment

### 8.1 One contained live reader

Create
`crates/unica-coder/src/infrastructure/parent_configurations/live_reader.rs`.
It resolves the trusted workspace/configuration source once and walks every
component through the existing retained no-follow capability layer. It never
uses `Path::canonicalize`, `exists`, `is_file`, an absolute reopen, or a prefix
string comparison as authority.

```rust
pub(crate) enum ParentConfigurationsReadV2 {
    Missing(VerifiedOptionalAbsenceV1),
    Parsed(ParentConfigurationsDocumentV2),
    Rejected(ParentConfigurationsProblemV2),
    IoFailure(ParentConfigurationsIoFailureV2),
}
```

`Missing` requires a retained exact parent and a no-follow negative leaf lookup
with unchanged ancestor identity. Directory, special file, symlink/reparse,
dangling link, permission failure, or not-found-after-observed-presence is not
Missing.

For Present, retained-handle metadata is checked before allocation. The reader
consumes at most 64 MiB + 1 and rechecks identity/length/content afterwards;
larger stable input is ResourceLimit and a change during the bounded read is a
retryable race. It never reserves the reported unchecked file length.

Retry classification is closed:

- retryable: interrupted, would-block, timed-out, and a proved identity/content
  race;
- non-retryable: permission denied, invalid/escaping path, non-regular leaf,
  symlink/reparse, unsupported backend operation, deterministic rejected bytes;
- unknown OS kinds default non-retryable and remain Indeterminate.

### 8.2 Renderer contract

`cf.info`, `form.info`, `meta.info`, `skd.info`, `mxl.info`, `role.info`, and
`subsystem.info` consume the typed result. They may keep `ok=true`, but their
support lines are explicit:

- verified missing base: `не на поддержке`;
- verified missing extension plus exact extension target authority:
  `объект принадлежит расширению; файл политики поддержки отсутствует`;
- parsed known state: current localized state;
- unsupported: `состояние поддержки неизвестно: формат не поддерживается`;
- malformed: `состояние поддержки неизвестно: файл некорректен`;
- I/O/identity failure: `состояние поддержки недоступно`.

Malformed/unsupported/I/O output must never contain `не на поддержке`, `правки
свободны`, `снята с поддержки`, or another positive authorization phrase.
Display is never parsed back.

### 8.3 Assessment algebra

```rust
pub(crate) enum SupportGuardAssessmentV2 {
    Safe(SupportSafetyProofV2),
    Violation(SupportGuardViolationV2),
    Indeterminate(SupportAssessmentProblemV2),
}
```

The assessment consumes exact target/catalog authority and the typed read.
Unresolved UUID, absent target, rejected bytes, I/O, flavor/membership gap, or
root-rule invariant is Indeterminate. `ObjectNotListed` is Safe only for a
resolved registered child UUID.

Requirement rules remain explicit:

- `ConfigurationReadOnly` violates every guarded mutation;
- `Editable` requirement is violated only by exact `Locked`;
- `Removed` requirement is violated by `Locked` and `Editable`, and accepts
  only `Removed`, exact ObjectNotListed, or exact no-support policy states;
- extension safety additionally requires the ownership join from section 7;
- no default enum arm converts a future state to Safe.

### 8.4 Mode and authoritative lease window

The configured `editingAllowedCheck` values remain exact `off|warn|deny`.
Missing, unreadable, malformed, or unknown configuration aliases to `deny` with
an operator diagnostic; it is not silently `off`.

Early application work resolves only mode, typed target, required state, and
the source root. For every applied guarded operation:

```text
validate args/workspace -> resolve guard route/mode
prepare target mutation
open workspace/control/source root and qualify backend
acquire one root-wide ArtifactMutationLeaseWitnessV1
recover ArtifactWriterWalV1
refresh target/catalog under that witness
authoritative ParentConfigurations read + assessment under same witness
discovery/receipt guard when applicable
handler -> MutationHandlerOutcome -> current-receipt handoff -> post snapshot
release current receipt, then release artifact witness
bounded canonical other-receipt reconciliation with neither lock held
```

A CFE operation reuses its one already-required root-wide witness. It must not
acquire a second support lease. A legacy support-guarded operation obtains the
same generic root-wide witness for its configuration source and holds it
through the handler. Writer/lease/WAL safety is not a configurable support
policy. Failure to qualify the exact backend, acquire a non-Busy witness, or
recover a valid WAL blocks **every** applied mutation in `off`, `warn`, and
`deny` before the handler.

Only after a valid recovered witness exists does the support-mode matrix apply:

| Mode | Known support violation | Rejected/I/O/unresolved support state |
| --- | --- | --- |
| `off` | intentional bypass; no support read or safety claim | intentional bypass |
| `warn` | handler runs with exact warning | handler runs with exact indeterminate warning |
| `deny` | handler is not invoked | handler is not invoked |

`artifact_writer_*` backend/WAL/recovery/lease failures are returned as their
typed operation-neutral failures and are never downgraded to a support warning
or bypass. Panic-handler REDs cover all three modes.

Dry-run performs no lease or write. If it displays a guarded preview it uses a
snapshot/live typed assessment labelled non-serializing; `deny` may block the
preview, but no dry-run result is mutation authority.

This contract serializes cooperating Unica writers only. External Designer or
third-party writes are detected by final identity/content recapture where the
operation has a Task 8 plan; Task 5C does not advertise portable filesystem
CAS.

## 9. `unica.support.edit` on the shared artifact writer

### 9.1 Imported Task 8 boundary

Task 5C defines no filesystem writer protocol. After the Task 8 v6 freeze it
must import its names verbatim. The currently coordinated export seam is:

```text
ArtifactMutationLeaseCandidateV1 -> ArtifactMutationLeaseWitnessV1
ArtifactWriterPlanV1
ControlStagedPublicationV1::PresentFileReplaceCrossDirectory
ArtifactWriterWalV1 / "unica.artifact-writer-wal.v1"
ArtifactMetadataPolicyV1
PresentTargetMetadataV1
MutationHandlerOutcome / "unica.mutation-outcome.v3"
```

Generic writer/recovery failures use the accepted operation-neutral
`artifact_writer_*` vocabulary. Task 5C may add support-specific pre-plan
rejections, but it must not translate a typed writer outcome or recovery state
to a support string. If final Task 8 names or semantics differ, this section is
updated before Task 5C is hashed; an alias layer is forbidden.

### 9.2 Typed resolver and action

Create:

- `crates/unica-coder/src/domain/support_edit.rs`;
- `crates/unica-coder/src/application/discovery_guard/target_resolvers/support_edit.rs`;
- adapt `crates/unica-coder/src/infrastructure/native_operations/support.rs`
  to consume only the typed plan/witness.

```rust
pub(crate) enum SupportEditActionV1 {
    CapabilityOn,             // tag 1
    CapabilityOff,            // tag 2
    SetLocked,                // tag 3
    SetEditable,              // tag 4
    SetRemoved,               // tag 5
}

pub(crate) struct ResolvedSupportEditPlanV1 {
    version: u16,
    source: ResolvedSourceSet,
    catalog_digest: Digest32,
    policy_artifact: CanonicalArtifactLocusV1,
    target: SupportEditTargetV1,
    action: SupportEditActionV1,
    authoritative_policy_digest: Digest32,
    disposition: SupportEditDispositionV1,
    normalized_arguments_digest: Digest32,
    execution_plan_digest: Digest32,
}

pub(crate) enum SupportEditDispositionV1 {
    MissingNoChange {
        verified_absence_digest: Digest32,
    },
    AlreadySatisfiedNoChange {
        before_content_digest: Digest32,
        before_semantic_digest: Digest32,
        before_skeleton_digest: Digest32,
    },
    Replace {
        before_content_digest: Digest32,
        before_semantic_digest: Digest32,
        before_skeleton_digest: Digest32,
        after_content_digest: Digest32,
        after_semantic_digest: Digest32,
        writer: ArtifactWriterPlanV1,
    },
}
```

Disposition tags are MissingNoChange=1, AlreadySatisfiedNoChange=2, Replace=3.
Only Replace can contain/call `ArtifactWriterPlanV1`; the two NoChange variants
cannot stage or name a publication primitive.

Raw aliases are normalized once. The request must contain exactly one
`Capability=on|off` or `Set=locked|editable|off-support`. Alias conflict,
unknown value, non-string value, or both/neither actions rejects before lease.

`Path` is resolved against one configured Platform XML source and the neutral
catalog, not by ancestor/file-name heuristics:

- Capability acts on that exact configuration source;
- Set accepts the configuration root or one registered top-level metadata
  object with an exact catalog UUID;
- nested Form/Template/Command policy mutation is
  `support_edit_nested_policy_unproven` until the stop in section 16 is cleared;
- an absent/unregistered/ambiguous target is
  `support_edit_target_unresolved` and never uses a guessed UUID;
- a source declared as configuration but with inconclusive/extension semantic
  flavor is rejected.

### 9.3 Exact applied lifetime

Preview parses arguments and may show intended semantic changes from a captured
snapshot, but acquires no lease and makes no authority claim for later apply.

Applied execution is exact:

1. validate arguments/workspace and derive one canonical source root and policy
   locus `Ext/ParentConfigurations.bin` without material read;
2. open the workspace/control/source root through Task 8's retained capability,
   qualify one exact atomic backend, and acquire
   `ArtifactMutationLeaseWitnessV1` for the root-wide collision universe;
3. recover `ArtifactWriterWalV1`; any non-recoverable state blocks;
4. refresh source mapping/catalog under the witness and require the same
   physical source root and canonical policy locus;
5. perform the authoritative no-follow leaf read under the witness, capture
   physical identity/content/metadata, and parse that exact byte slice once;
6. construct `authoritative_policy_digest` from witness generation, source and
   catalog identity, requested action, and either exact verified-absence
   authority or physical leaf identity plus content/semantic/skeleton digests;
7. derive new bytes only from validated spans, reparse those exact bytes, and
   prove the postcondition in section 9.4;
8. for Replace only, create `ArtifactWriterPlanV1` binding the same witness generation,
   `authoritative_policy_digest`, expected before identity/content/metadata,
   exact after content/metadata, and
   `PresentFileReplaceCrossDirectory`;
9. immediately before control staging, re-read/revalidate policy through the
   same retained witness; mismatch is pre-install NoChange with
   `support_policy_changed_under_lease`, never a rebuilt plan;
10. call the generic WAL-backed writer once; retain the witness through typed
    outcome, receipt-free Terminal-to-Idle WAL handling, and the authoritative
    post snapshot, then release it;
11. with no artifact/current-receipt lock held, reconcile other receipts in
    bounded canonical ID order using non-blocking receipt leases;
12. derive `AdapterOutcome` only after the typed outcome exists.

The operation never calls `Path::canonicalize`, `exists`, `is_file`,
`fs::write`, same-directory target staging, or another semantic parser during
this lifetime. It never opens an absolute destination after the one trusted
workspace-root open.

A verified missing `ParentConfigurations.bin` is a typed `NoChange` no-op. It
does not create a policy file because v2 has no authoritative serialization for
“not under support”. Rejected/unavailable material returns no writer plan.

### 9.4 Span mutation and semantic postconditions

Mutation starts with a clone of the exact authoritative bytes and replaces only
one-byte validated flag spans:

- `CapabilityOff` when effective editing is Enabled writes global=`1`,
  vendor=`1`, and every object rule=`1`;
- `CapabilityOff` when already ReadOnly is NoChange and preserves the exact
  bytes, including a safe `(0,1)` or `(1,0)` compatibility shape;
- `CapabilityOn` when ReadOnly writes global=`0`, vendor=`0`, and every object
  rule=`0` (all objects Locked), matching the existing documented reset;
- `CapabilityOn` when effective editing is already Enabled is NoChange and
  preserves existing per-object rules;
- `Set*` requires effective `(0,0)` and changes exactly the one requested UUID
  rule span; a UUID absent from the complete rule map is NoChange and never
  appends a record.

After replacement, the pure parser must accept the complete bytes and prove:

- exact BOM/framing and byte length unchanged;
- skeleton digest unchanged;
- vendor UUIDs/labels, record count/order, object UUIDs/mirrors unchanged;
- only the allowed flag set differs;
- effective global state and target rule equal the requested action;
- output semantic/content digests equal the plan's computed values.

Failure to construct that proof is `support_edit_postcondition_failed` before
staging. No regex, substring replacement, or “replace all UUID matches” path
exists.

### 9.5 Metadata and timestamp policy

Support edit is a **content replacement** of a Present regular file. It reuses
the final accepted Task 8 metadata policy for owner/group, mode or DACL, ACL,
xattrs or ADS, flags, link count, reparse/special storage, and every pre/post
verification. Unsupported metadata blocks before source mutation.

It must not copy stale pre-edit content timestamps as if the file had not
changed. The frozen Task 8 policy must distinguish preserved security/storage
metadata from content-change timestamps:

- Unix content mtime/ctime follow the accepted writer-defined post-write
  observation policy rather than being restored to before values;
- Windows creation time may be preserved only if the accepted Task 8 policy
  says so, while LastWriteTime/ChangeTime follow the accepted content-change
  policy and are never claimed equal to the old bytes' timestamps;
- every timestamp included in expected metadata is postverified under the same
  writer policy/digest; an unqueryable/mismatched observation remains definite
  `Committed` with metadata Unknown/Mismatch and cannot advance a receipt.

Task 5C freezes no independent timestamp enum. If Task 8 v6 does not export a
content-replacement policy satisfying these rows, 5C-Mutation remains STOP.

### 9.6 Outcome and receipt semantics

Every applied support edit returns `Some(MutationHandlerOutcome)` even when the
adapter view is `ok=false`:

- no missing-file or already-satisfied change -> `NoChange` with exact clean
  control lifecycle;
- definite atomic replace -> `Committed` with one Updated target, exact known
  expected content, metadata observation, cleanup, and durability;
- unknown rename completion -> `Uncertain`; it is never retried as NoChange;
- definite install plus failed fresh path/content/metadata observation remains
  `Committed`, non-advancing and revoking.

`unica.support.edit` is not itself authorized by a discovery receipt and does
not invoke `cfe.borrow`. Its writer WAL uses the accepted receipt-free terminal
handoff policy. Nevertheless ParentConfigurations is part of every affected
source fingerprint. Task 10 must therefore:

- capture the post snapshot after the typed outcome;
- release the artifact witness before acquiring any other-receipt lease, then
  revoke/reconcile affected receipts in bounded canonical order according to
  each exact composite baseline;
- ensure a crash before reconciliation cannot authorize a stale receipt (its
  next guard sees a fingerprint mismatch);
- never infer source effects from `AdapterOutcome.changes/artifacts`;
- keep writer terminal/recovery authority independent from observation failure.

Crash before other-receipt reconciliation is safe because each old receipt's
content fingerprint mismatches on its next guard and is lazily revoked; safety
does not come from holding one artifact lease while acquiring many receipt
locks. Acquiring another receipt lease under an artifact/current-receipt lock,
or reacquiring the artifact lease during reconciliation, is a hard order
violation.

## 10. Canonical encoders

All new encoders use SHA-256 over a domain string terminated by `\0` and an
unambiguous payload. `u16be`, `u32be`, and `u64be` are fixed-width big endian;
`bytes(x)=u64be(len)||x`; `string` is UTF-8 bytes; `digest32` decodes exactly 64
lowercase hex characters (with a separately validated optional `sha256:` wire
prefix) to 32 bytes. Debug/JSON/serde order, enum discriminants, path separator,
absolute paths, timestamps, epoch, and pointer addresses never enter a semantic
digest.

### 10.1 Parsed document

```text
H("unica.parent-configurations.semantic/v2",
  u16be(2) || framing_tag=1 || authority_tag ||
  global_raw_tag || vendor_raw_tag || effective_editing_tag ||
  uuid(vendor_configuration_uuid) || uuid(vendor_instance_uuid) ||
  string(version) || string(vendor) || string(configuration) ||
  u32be(rule_count) ||
  vec(in serialized order: uuid || rule_tag || mirror_present_tag))
```

Stable tags:

```text
authority ExactTrackedExportV1=1, AcceptedCompatibilityV2=2
raw flag 0=1, 1=2
effective Enabled=1, ReadOnlyVendor=2,
          ReadOnlyGlobal=3, ReadOnlyGlobalAndVendor=4
rule Locked=1, Editable=2, Removed=3
mirror Absent=1, PresentEqual=2
```

The skeleton digest is:

```text
H("unica.parent-configurations.skeleton/v2",
  u64be(original_len) || exact_bytes_with_each_validated_flag_replaced_by_FF)
```

The content digest is ordinary SHA-256 over exact original bytes, stored as a
typed `Digest32`. A vendor flag is never omitted from semantic or skeleton
authority.

### 10.2 Support authority and planned absence

Task 5C reuses Task 5B's exact
`platform-configuration-catalog-set/v1` digest. The planned-absence digest is:

```text
H("unica.support-planned-destination-absent/v1",
  DestinationMembershipPairIdentityBytesV2(pair) ||
  SourceScopedArtifactIdentityBytesV2(analysis_authority.scope) ||
  uuid(analysis_authority.object_uuid) || base_flavor_tag || own_tag ||
  SourceScopedArtifactIdentityBytesV2(destination_absence.scope) ||
  extension_flavor_tag || destination_membership_absent_tag ||
  digest32(analysis_fact_digest) || digest32(destination_fact_digest) ||
  digest32(analysis_source_fingerprint) ||
  digest32(destination_source_fingerprint) ||
  digest32(catalog_set_digest))
```

Provenance evidence IDs are retained in the report/verdict evidence vector but
excluded from the semantic planned-absence digest after their facts/freshness
have been validated; location-duplicate provenance cannot change authority.
`SupportAuthorityIndexV2` hashes the canonical subject -> authority semantic
digest vector under `H("unica.support-authority-index/v2", ...)`.

No generic name/UUID can be encoded without the smart-constructed Task 5B pair.

### 10.3 Support edit

```text
normalizedArguments = H("unica.support-edit.arguments/v1",
  source_identity || target_kind_and_identity || action_tag)

authoritativePolicy = H("unica.support-edit.authoritative-policy/v1",
  witness_generation_digest || source_identity || catalog_digest ||
  policy_artifact_identity || action_tag || leaf_state_tag ||
  [verified_absence_digest |
   physical_leaf_identity_digest || before_content_digest ||
   before_semantic_digest || before_skeleton_digest])

executionPlan = H("unica.support-edit.execution-plan/v1",
  normalized_arguments_digest || authoritative_policy_digest ||
  disposition_tag ||
  [verified_absence_digest |
   before_content_digest || before_semantic_digest || before_skeleton_digest |
   publication_tag || metadata_policy_version_and_digest ||
   before_content_digest || after_content_digest ||
   before_semantic_digest || after_semantic_digest ||
   skeleton_digest || writer_wal_schema || mutation_outcome_schema ||
   atomic_backend_contract_version])
```

The bracket alternatives follow disposition tags 1, 2, and 3 respectively;
there is no empty optional arm. Action tags follow declaration order 1..5.
Publication tag is imported from Task 8; Task 5C never assigns another number.

### 10.4 Problem/reason encoding

`ParentConfigurationsProblemV2` stores a closed class tag, stable reason tag,
and optional bounded byte offset (`u64`; no path/raw token). Its digest is:

```text
H("unica.parent-configurations.problem/v2",
  class_tag || u16be(reason_tag) || offset_option_tag || [u64be(offset)])
```

Changing a spelling/tag or adding a reason requires
`parent-configurations-reasons/v3`; tags are append-only.

## 11. Stable reasons v2

The exact leading code is stable; human detail is bounded and never authority.

| Tag | Class | Code |
| ---: | --- | --- |
| 1 | malformed | `invalid_parent_configurations_encoding` |
| 2 | unsupported | `unsupported_parent_configurations_framing` |
| 3 | malformed | `parent_configurations_embedded_nul` |
| 4 | resource | `parent_configurations_resource_limit` |
| 5 | malformed | `truncated_parent_configurations` |
| 6 | unsupported | `unsupported_parent_configurations_variant` |
| 7 | malformed | `invalid_parent_configurations_global_flag` |
| 8 | malformed | `invalid_parent_configurations_vendor_flag` |
| 9 | malformed | `invalid_parent_configurations_uuid` |
| 10 | malformed | `invalid_parent_configurations_quoted_scalar` |
| 11 | malformed | `invalid_parent_configuration_rule` |
| 12 | malformed | `parent_configuration_mirror_uuid_mismatch` |
| 13 | unsupported | `duplicate_parent_configuration_rule` |
| 14 | unsupported | `conflicting_parent_configuration_rules` |
| 15 | malformed | `parent_configurations_object_count_mismatch` |
| 16 | malformed | `parent_configurations_trailing_material` |

Provider/application reasons are separately closed:

- `support_subject_unresolved`;
- `support_configuration_flavor_inconclusive`;
- `support_membership_inconclusive`;
- `support_configuration_root_rule_missing`;
- `support_absent_destination_rule_conflict`;
- `support_state_inconclusive`;
- `source_fingerprint_mismatch`;
- `support_query_subject_limit`;
- `support_edit_target_unresolved`;
- `support_edit_nested_policy_unproven`;
- `support_edit_action_conflict`;
- `support_edit_atomic_writer_required`;
- `support_edit_policy_read_failed`;
- `support_policy_changed_under_lease`;
- `support_edit_postcondition_failed`.

Generic lease/backend/metadata/WAL/outcome failures remain the exact accepted
`artifact_writer_*` codes and typed states. They are not aliased to a support
reason. `ProviderContractViolation` remains an application error class, not a
fake evidence gap.

## 12. Mandatory RED -> GREEN matrix

Every named RED is written first, run in isolation, and its actual failing
assertion/test count is recorded in `.superpowers/sdd/task-5c-report.md`.
“Command succeeded with zero matching tests” is a failed gate.

### 12.1 Fixture/provenance REDs

1. `canonical_export_fixture_matches_all_three_donors` checks exact bytes,
   length 337, one BOM, no final newline, and the frozen SHA-256.
2. `provenance_manifest_calls_three_files_one_example` rejects wording/counts
   that present them as three independent variants.
3. `synthetic_compatibility_fixture_is_not_export_provenance` proves the
   global/vendor/object flag-1 helper is labelled
   `AcceptedCompatibilityV2`, never `ExactTrackedExportV1` evidence.

Run:

```text
cargo test --locked -p unica-coder parent_configurations::tests::provenance -- --nocapture
```

Expected RED: three named tests execute and fail because the canonical fixture,
manifest, and authority grade do not yet exist.

### 12.2 Pure parser REDs

Positive rows:

- exact canonical exported fixture -> one vendor, three rules, expected
  no-mirror/mirror shapes, Enabled;
- current BOM-prefixed helper -> exact Locked/Editable/Removed compatibility;
- exact flag pair matrix `(0,0)|(0,1)|(1,0)|(1,1)` crossed with object rules
  `0|1|2`: only `(0,0)` yields per-object state; all other 9 rows yield
  ConfigurationReadOnly;
- first/last record with or without equal mirror parses without changing record
  count;
- non-ASCII vendor label remains one scalar token and cannot create a rule.

Negative rows:

- missing/double BOM, BOM after whitespace, leading/trailing whitespace and
  final newline;
- invalid UTF-8, UTF-16, NUL, control byte, unsupported escape;
- empty and every byte length `1..=32`;
- marker other than `6`; vendor count `0`, `2`, and integer overflow;
- global/vendor flag outside `0|1` and rule outside `0|1|2`;
- noncanonical numeric token and uppercase/braced/compact/nil/padded UUID;
- empty/oversized/quoted-control vendor scalar;
- truncated quote/UUID/record/brace; extra token or bytes after `}`;
- lexical rule-shaped bytes inside quoted strings;
- mirror unequal/invalid; declared rule count lower/higher than actual;
- identical duplicate UUID and conflicting duplicate UUID, independent of
  order;
- exact bounds at 64 MiB/1,000,000 tokens/200,000 rules pass where a complete
  valid document can be constructed; N+1 fails before oversized allocation;
- rejection returns zero edit spans/rules/semantic digest.

Encoder rows:

- goldens for four global/vendor effective tags and all three rules;
- changing record order changes semantic, skeleton, and content digests; v2
  makes no unproven order-insensitivity claim;
- flipping global, vendor, or object flag changes semantic/content but not
  skeleton digest;
- changing BOM, label, UUID, mirror presence, count, or order changes skeleton;
- no two stable tags collide and unknown tags fail decode.

Run:

```text
cargo test --locked -p unica-coder parent_configurations::tests::parser_v2 -- --nocapture
cargo test --locked -p unica-coder parent_configurations::tests::encoder_v2 -- --nocapture
```

### 12.3 Shared catalog authority REDs

- `catalog_port_runs_once_per_composite_snapshot` records exactly one call;
- Metadata and Support receive the same borrowed catalog-set address, exact
  per-source addresses, set/catalog digests, and source fingerprints;
- a reconstructed equal-looking catalog object fails the identity recording
  seam;
- same canonical artifact in base and extension remains source-distinct;
- forged source kind cannot change parsed flavor;
- missing/inconclusive flavor/membership/UUID and mixed capture envelopes fail;
- any Support dependency on Metadata adapter/evidence/display/local-name parser
  fails a static product scan;
- arbitrary prefix with exact MDClasses URI passes; `urn:1c` and same-local
  foreign namespace fail through the Task 5B authority.

Run:

```text
cargo test --locked -p unica-coder platform_configuration_catalog_shared -- --nocapture
```

### 12.4 Query/provider REDs

Planning:

- 4096 exact semantic subjects construct and invoke once;
- 4097 reject before provider I/O with `support_query_subject_limit`;
- changing only `maxEvidence` does not change query subjects/query digest;
- source/subject order and duplicates produce one canonical plan;
- provider has no local lossy-limit branch or `platform_xml_result_limit`.

Facts:

- verified missing Base -> BaseWithout for every exact present requested entry;
- verified missing Extension -> ExtensionWithout, but no direct ownership;
- global/vendor four-state matrix -> ConfigurationReadOnly for tags 2..4;
- Enabled + exact UUID -> Locked/Editable/Removed;
- Enabled + exact registered child absent from rules -> ObjectNotListed;
- any vendor=1 never emits Editable/Removed/ObjectNotListed;
- missing root rule -> `support_configuration_root_rule_missing`, zero facts;
- rejected present bytes -> one exact Bounded group gap, zero group facts;
- manifest length 64 MiB is bounded-read eligible; 64 MiB + 1 returns resource
  gap with a panic source-reader proving zero body reads/allocations;
- fingerprint mutation/read race -> retryable Unavailable, zero group facts;
- optional key absent from manifest and catalog mismatch -> contract violation;
- base/extension identical bytes still produce source-distinct fact IDs;
- provider/source/order reversal produces byte-identical canonical output.

Planned absence:

- exact BaseOwned UUID + exact destination membership Absent + safe missing
  extension policy -> scoped ExtensionWithout;
- same pair + accepted Enabled document with UUID absent -> scoped
  ObjectNotListed;
- same pair + effective read-only -> ConfigurationReadOnly;
- any rule for the absent UUID ->
  `support_absent_destination_rule_conflict`, zero fact;
- forged source/name/UUID/pair, generic MetadataAbsent, missing flavor, or stale
  fingerprint cannot construct `PlannedDestinationAbsentV1`;
- a planned-absence fact cannot enter a direct-candidate projection;
- query/authority-index/provider order changes preserve byte-identical planned
  digest and application join; location-duplicate provenance adds evidence IDs
  without changing semantic authority;
- a returned support record paired with Existing when query says PlannedAbsent
  (or the reverse) is a contract violation;
- authority index without a retained support record after Task 7 admission
  creates no support fact/projection.

Run:

```text
cargo test --locked -p unica-coder discovery::support::tests -- --nocapture
```

### 12.5 Projection REDs

- exhaustive base direct matrix from section 7.1;
- exhaustive extension direct matrix with present Own and present Adopted;
- extension missing-policy fact without exact present membership -> Unknown;
- identical refs in base/extension never merge facts;
- destination Locked/ConfigurationReadOnly precedence over safe analysis;
- exact Absent + safe planned-absence support -> ExtensionRequired with
  `destination_borrow_required`, but:
  `receiptEligibility=false`, resolver calls=0, issuer calls=0, handler calls=0,
  and `/cfe-borrow` appears only as guidance;
- absent without exact planned authority -> Unknown, not ExtensionRequired;
- exact equal Adopted + safe policy -> ExtensionOwned;
- Own/wrong Adopted/inconclusive -> their exact Task 5B Unknown blockers;
- unknown analysis support -> Unknown even with a safe destination;
- no test or code path calls borrow implicitly.

Run:

```text
cargo test --locked -p unica-coder support_projection_v2 -- --nocapture
```

### 12.6 Live reader/renderer REDs

Parameterize configuration and object renderers over:

- verified missing base/extension;
- every parsed effective/per-object state;
- malformed, unsupported, resource limit;
- retryable and non-retryable I/O;
- unresolved target/flavor/membership/root invariant.

Assert no positive authorization phrase occurs for an unknown state. Native
Unix/Windows fakes race every ancestor/leaf identity, replace with symlink or
reparse, directory/special file, disappear-after-presence, permission failure,
and final content mutation. Only stable exact negative lookup is Missing.

Run:

```text
cargo test --locked -p unica-coder parent_configurations::live_reader -- --nocapture
cargo test --locked -p unica-coder support_renderer_v2 -- --nocapture
```

### 12.7 Guard REDs

Cross `off|warn|deny` with:

- Locked, ConfigurationReadOnly, Removed, ObjectNotListed, exact missing base;
- rejected/unsupported/resource-limited bytes;
- I/O/identity race, unresolved UUID, missing/inconclusive ownership;
- Editable versus Removed requirement;
- invalid/missing guard configuration.

For each row assert exact assessment, warning/block reason, handler call count,
and source write count. Additionally:

- all three modes with backend qualification failure, Busy/unavailable lease,
  corrupt/nonrecoverable WAL, or witness-generation mismatch invoke a panic
  handler zero times;
- early mode/route stage performs no authoritative policy read;
- authoritative read occurs after WAL recovery under the same witness passed to
  handler and post-mutation seam;
- CFE acquires exactly one root-wide artifact lease, never a support lease plus
  target lease;
- support change before under-lease read is observed as current policy;
- cooperative support edit cannot change policy while the witness is held;
- final policy mismatch before staging returns typed NoChange/zero source
  effects, never a rebuilt plan;
- `off` bypasses only support assessment, not lease/WAL/writer safety.

Run:

```text
cargo test --locked -p unica-coder support_guard_v2 -- --nocapture
```

### 12.8 Pure support-edit plan REDs

- action alias matrix: exact one action accepted; both/neither/conflict/unknown
  rejected;
- CapabilityOff from Enabled changes global/vendor/all rule spans to `1`;
- CapabilityOff from each ReadOnly tag is exact NoChange;
- CapabilityOn from each ReadOnly tag changes global/vendor/all rules to `0`;
- CapabilityOn from Enabled preserves existing object rules and is NoChange;
- Set Locked/Editable/Removed changes exactly one UUID span under `(0,0)`;
- Set under any ReadOnly tag blocks; absent rule is NoChange/no append;
- top-level registered target resolves exact UUID; nested Form/Template/Command
  stops; same-name/unregistered/absent target cannot resolve;
- output reparses, skeleton/length/nonmutable bytes stay equal, and exact semantic
  postcondition holds;
- hook mutation after parse/before plan or before staging refuses stale input;
- missing/rejected/I/O input creates no writer plan;
- no regex/string replacement/path-authority function remains reachable.

Run:

```text
cargo test --locked -p unica-coder support_edit_v1::plan -- --nocapture
```

### 12.9 Artifact writer/WAL/outcome integration REDs

Run the accepted Task 8 generic suite unchanged, then support-specific cases:

- Present control-staged replace only; direct target-parent staging forbidden;
- exact before identity/content/security metadata and final policy recapture;
- target swap/ancestor move/content drift before staging -> no source mutation;
- non-content metadata drift -> preinstall rejection;
- every WAL transition hard-crash/restart recovery boundary;
- torn/equal-generation-conflict/unknown-tag/orphan staging -> recovery required;
- definite rename plus post-read/path/metadata failure remains Committed;
- unknown rename completion is Uncertain and is never retried blindly;
- control staging cleanup/durability failure remains typed;
- updated bytes and accepted content-replacement metadata postverify exactly;
- stale LastWriteTime is not restored; accepted new timestamp semantics are
  observed/encoded, including injected query mismatch/unknown;
- AdapterOutcome failure cannot erase mutation outcome;
- support edit changes the source fingerprint; matching receipts revoke/stale;
- crash after commit/before other-receipt reconciliation cannot authorize a
  subsequent handler with the old baseline;
- recording lock fakes prove receipt-free Terminal-to-Idle and post snapshot
  occur under artifact witness, then artifact drops before the first sorted
  other-receipt lease; panic fakes reject other-receipt-under-artifact,
  artifact reacquisition during reconciliation, or two receipt leases at once.

Run:

```text
cargo test --locked -p unica-coder artifact_writer -- --nocapture
cargo test --locked -p unica-coder support_edit_v1::writer -- --nocapture
cargo test --locked -p unica-coder support_edit_v1::receipt_reconciliation -- --nocapture
```

## 13. Cross-platform semantics and release gate

Pure parsing/projection is platform-independent. Live authority and mutation
are platform-specific only through the already reviewed contained filesystem
and Task 8 writer contracts.

An applied support edit, and an applied guarded handler that needs an
authoritative lease window, is enabled only on exact Task 8 qualified tuples:

- local Linux ext4/XFS rows with their exact kernel/architecture/native-suite
  digest;
- local macOS APFS case-sensitive and case-insensitive rows with independent
  OS/architecture/native-suite digests;
- local Windows NTFS rows with exact build/architecture/native-suite digest.

NFS/CIFS/SMB/FUSE/overlay/tmpfs, ReFS/FAT/exFAT, remote/virtual/read-only,
nested/unidentified mount/volume, or a tuple without the exact reviewed WAL,
metadata, cross-directory replace, lock, and durability transcript blocks all
applied modes before source mutation. Syscall presence/filesystem name alone is
not qualification.

Platform suites must prove:

- component-relative no-follow containment and retained physical identity;
- one common root-wide lock inode/FileId and Busy across path/case/Unicode/workspace
  aliases admitted by the backend;
- control and source are on the same qualified mount/volume;
- cross-directory Present replace, two-parent durability, WAL recovery, and
  metadata/timestamp policy;
- Windows uses handle-relative NT primitives and never path-based
  `MoveFileExW`/destination reopen;
- Unix uses accepted `openat`/rename/fsync/full-sync primitives and never a
  path reopen;
- forced-crash/restart evidence on each exact claimed tuple.

Read-only info may report typed unavailable on an unsupported/racy live
filesystem. `warn`/`off` cannot upgrade unsupported mutation infrastructure to
safe apply.

## 14. Implementation sequence and commit boundaries

Use TDD and fresh review at each independently rejectable boundary. No step may
silently edit the immutable v1 design/notes.

### 14.1 Task5C-Evidence commit (precedes Task 8)

1. **Gate and fixture.** Record accepted Task5A/Task5B implementation Git OIDs
   and accepted Task7-v6 design SHA-256; add the canonical
   fixture/provenance manifest and REDs.
2. **Pure parser.** Land bounded lexer/parser/types/spans/digests/reasons after
   every parser RED is observed.
3. **Shared authority.** Consume the exact Task 5B
   `PlatformConfigurationCatalogSetV1` one-build/shared-input port and its
   identity tests; do not create an adapter bridge.
4. **Query/provider.** Land the 4096-subject all-or-nothing query, existing and
   planned-absence authority, one-read-per-source provider, raw facts, and
   provider tests. No local lossy limit.
5. **Projection.** Remove the context-free direct projection and implement the
   total ownership/intention matrices, including zero issuer for
   ExtensionRequired patch advice.
6. **Live read/render/assessment.** Migrate all listed renderers and legacy
   guard assessment to typed results. Do not claim the later serializable lease
   window.
7. **Transitional mutation STOP.** Delete the old support-edit pattern/path
   writer and make applied `unica.support.edit` return
   `support_edit_atomic_writer_required`, zero writes. Keep only typed preview
   and pure postcondition planning tests that require no Task 8 type.
8. **Docs/product contract.** Synchronize active spec/ADR/historical plan with
   the evidence/ownership/format boundary, compatibility provenance, 4096
   bound, no implicit borrow, and the explicit transitional STOP.
9. **Verify and review.** Run the 5C-Evidence gate in section 15, obtain a fresh
   independent no-P0/P1 review, commit as:

```text
feat: сделать support evidence строгим и ownership-aware
```

Record the exact 40-hex Git OID as `TASK5C_EVIDENCE_ACCEPTED_GIT_OID` and its
review's 64-hex SHA-256. Task 8 then imports that exact commit.

### 14.2 Task 8/9/10 bridge

Before Task5C-Mutation code:

1. Task 6/7 implementations consume the accepted Task5C-Evidence provider and
   support-group contract, pass their independent gates, and land.
2. Task 8 v6 freezes/lands the generic lease/writer/WAL/metadata/outcome seam,
   names the accepted Task5C-Evidence Git OID, and passes its native matrix.
3. Task 9 v6 addendum/implementation persists schema-v3 and exact WAL
   correlation without reimplementing the writer.
4. Task 10 v6 addendum/implementation freezes application order: early support
   routing only, one artifact witness, WAL recovery, authoritative policy
   revalidation, optional current-receipt lease, handler outcome/current-receipt
   handoff, post snapshot, release current receipt then artifact witness, and
   only then bounded other-receipt reconciliation.

Any older receipt-first order, CFE-specific-only writer, schema v2 outcome, or
support guard before the authoritative lease is a bridge failure.

### 14.3 Task5C-Mutation commit

1. **Import, do not adapt.** Import exact accepted Task 8 types/tags/reasons;
   add compile-time/product tests rejecting an alias or local writer.
2. **Typed resolver.** Land `SupportEditActionV1`, target resolution, span-only
   plan/postcondition encoders, and pure REDs.
3. **Authoritative guard window.** Integrate the Task 10 order for CFE and every
   legacy support-guarded applied operation; one root-wide witness only.
4. **Writer integration.** Replace the transitional STOP with the exact
   Task 8 Present writer and receipt-free WAL handoff; no direct path write.
5. **Metadata/outcome.** Import final content-replacement timestamp policy,
   schema-v3 outcome, cleanup/durability/recovery, and all fault/crash tests.
6. **Receipt reconciliation.** Prove changed policy fingerprints stale/revoke
   affected receipts and a crash cannot authorize an old baseline.
7. **Docs/product contract.** Remove only the transitional STOP text; document
   exact supported backend tuples, compatibility limitations, WAL recovery,
   operator reasons, and remaining format/policy stops.
8. **Verify and review.** Run section 15 in full, obtain fresh independent
   no-P0/P1 code/spec reviews, commit as:

```text
fix: перевести support edit на durable artifact writer
```

Record the exact 40-hex Git OID as `TASK5C_MUTATION_ACCEPTED_GIT_OID`.

The final `.superpowers/sdd/task-5c-report.md` names both Task5C Git OIDs, every
dependency design/review SHA-256, actual RED/GREEN outputs, supported native
tuple evidence, and every remaining STOP. It makes no single circular “Task5C
commit required by Task8 and based on Task8” claim.

## 15. Verification gates

### 15.1 Task5C-Evidence gate

```text
cargo fmt --all -- --check
cargo test --locked -p unica-coder parent_configurations -- --nocapture
cargo test --locked -p unica-coder platform_configuration_catalog_shared -- --nocapture
cargo test --locked -p unica-coder discovery::support -- --nocapture
cargo test --locked -p unica-coder support_projection_v2 -- --nocapture
cargo test --locked -p unica-coder support_renderer_v2 -- --nocapture
cargo test --locked -p unica-coder support_guard_v2 -- --nocapture
cargo test --locked -p unica-coder application::discovery -- --nocapture
cargo test --locked -p unica-coder
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
python3.12 tests/ci/test_product_contracts.py
git diff --check
```

The focused output must list the named tests and nonzero test counts. A
zero-test filter is failure. Product contracts must prove applied support edit
is temporarily fail-closed and no old path writer remains.

### 15.2 Task5C-Mutation/final gate

Run the complete Evidence gate plus:

```text
cargo test --locked -p unica-coder artifact_writer -- --nocapture
cargo test --locked -p unica-coder support_edit_v1 -- --nocapture
cargo test --locked -p unica-coder discovery_guard -- --nocapture
cargo test --locked -p unica-coder discovery_receipts -- --nocapture
```

Then run every Task 8 native qualified-tuple race/failure/crash suite and its
package support-matrix check on the exact claimed OS/filesystem rows. A local
unit fake cannot qualify a release tuple.

Static scans must prove:

- no semantic `ParentConfigurations` parser remains outside the neutral module;
- no support edit uses `fs::write`, path temp/rename, `canonicalize`, `exists`,
  regex, raw substring replacement, or a second read/parse;
- Support imports no Metadata adapter/display parser;
- no `ExtensionWithout... -> ExtensionOwned` context-free branch remains;
- no vendor=1 branch emits a writable per-object fact;
- no local Support evidence ceiling or cross-provider atomic-group claim
  remains;
- no CFE patch receipt/plan/issuer is produced for ExtensionRequired;
- all applied support modes retain unconditional writer/lease/WAL safety;
- Git OIDs and SHA-256 evidence fields cannot be swapped.

## 16. Hard STOP conditions

Stop instead of guessing if any implementation/review needs:

1. a present explicit-not-under-support byte encoding;
2. zero/multiple vendor composition or duplicate/conflict aggregation;
3. BOM-less/UTF-16/other container acceptance;
4. a flag/value/record shape outside the exported plus explicitly labelled
   compatibility subset;
5. nested Form/Template destructive authorization or support editing without a
   real exported owner-plus-child fixture and product matrix;
6. ownership from `ExtensionWithoutParentConfigurations`, source kind, name,
   path, or generic absence without the Task 5B typed join;
7. a patch receipt/resolved plan/implicit borrow for ExtensionRequired;
8. a Metadata-to-Support adapter call, repeated MDClasses parse, or distinct
   catalog instance;
9. a Support provider-local lossy limit, partial prefix, or cross-provider
   atomicity claim;
10. an applied mutation after backend/lease/WAL failure in any support mode;
11. Task 8 writer reuse through an alias/wrapper that changes tags, errors,
    outcome, WAL, metadata, recovery, or durability semantics;
12. stale LastWriteTime/content timestamp restoration or an invented Task 5C
    timestamp policy;
13. target-parent staging, path reopen/write/rename, unqualified backend, or a
    portable arbitrary-writer CAS claim;
14. receipt transition before writer Terminal, early receipt-mode WAL clear, a
    second artifact lease, or reversed lock/release order;
15. implementation before its exact dependency Git OIDs/design/review hashes
    exist and validate;
16. an intermediate Task5C-Evidence commit that leaves the unsafe legacy
    support-edit path writer active;
17. a release with only Task5C-Evidence and without the later Mutation commit.

Clearing a format STOP requires the smallest redistributable real
Designer-exported fixture, provenance, and before/after platform behavior.
Clearing the nested-policy or implicit-borrow STOP additionally requires an ADR
and explicit product approval. Clearing a backend STOP requires the complete
native race/failure/hard-crash evidence and a reviewed support-matrix digest.

## 17. Freeze and self-audit procedure

This draft becomes hashable only after Task 8 v6 publishes its immutable
SHA-256 and final generic export seam. Then the design owner must:

1. replace every provisional Task 8 name in section 9 with exact frozen names;
2. import the exact timestamp policy, generic reason codes, tags, and schema
   strings without aliases;
3. re-read accepted Task 5B/7 and Task 8 documents fully and run a stale-text
   search over this v2 file;
4. validate the dependency ledger with disjoint types:

```text
*_ACCEPTED_GIT_OID  = exactly 40 lowercase hexadecimal characters
*_DESIGN_SHA256     = exactly 64 lowercase hexadecimal characters
*_REVIEW_SHA256     = exactly 64 lowercase hexadecimal characters
```

   A field named `*_commit_sha256`, a 64-char Git slot, a 40-char document
   digest, uppercase hex, branch name, or dirty-tree pseudo-SHA is invalid.
5. run placeholder/ambiguity scans, encoder/tag uniqueness checks, and verify
   every RED has one implementation owner and one expected outcome;
6. write `.superpowers/sdd/task-5c-v2-self-audit.md` with exact source design
   hash, dependency values, every correction above, and a no-open-P0/P1
   verdict or an explicit STOP;
7. compute SHA-256 for the final design and self-audit only after both files are
   closed and unchanged;
8. obtain a fresh independent cross-review. Any finding changes a new version
   or the still-unfrozen v2 draft, invalidates old hashes, and reruns this
   procedure.

The self-audit must explicitly search for and reject these stale phrases:

```text
optional BOM
duplicate rules dedupe
ExtensionWithoutParentConfigurations -> ExtensionOwned
MAX_SUPPORT_QUERY_SUBJECTS = 656
support provider-local result limit
support guard before artifact lease
warn/off bypass backend failure
implicit borrow
one final Task5C commit required by and based on Task8
preserve LastWriteTime
fs::write ParentConfigurations
```

## 18. Design result

Task5C-Evidence can land before Task 8 without circularity and supplies the
strict parser/provider/ownership authority Task 8 needs. Task5C-Mutation lands
only after Task8/9/10 and reuses their exact durable primitive. The resulting
system distinguishes real evidence, explicit compatibility, unknown material,
ownership, support policy, and filesystem commit authority instead of
collapsing them into path heuristics or `Option` silence.
