# Task 8 Source-Bound Shared CFE Mutation Resolver Implementation Design v5

> **For agentic workers:** REQUIRED SUB-SKILL: use
> `superpowers:subagent-driven-development` or
> `superpowers:executing-plans`. Implement every numbered slice RED -> GREEN;
> do not skip prerequisite or STOP gates.

**Goal:** Реализовать один строгий source-bound resolver для
`unica.cfe.patch_method`. Он должен нормализовать direct arguments и discovery
`mutationIntent`, доказать exact base-Configuration analysis method и exact
already-borrowed destination, захватить present/absent destination module, обе
Form binding surfaces и typed parent chain, построить один canonical immutable
`ResolvedMutationPlan`, а затем передать тот же объект под workspace-invariant
artifact mutation lease до guard seam, typed native handler и
post-snapshot/future Task 10 receipt-transition seam без повторного разбора
output-affecting raw arguments.

**Architecture:** Один resolver имеет две обязательные стадии одного
контракта: `prepare` разрешает topology, selectors и bounded `SnapshotWatch` до
capture; `resolve` после атомарного capture читает exact analysis method,
проверяет adopted UUID chain, source-derived signature/context/kind,
destination material и duplicates, затем рендерит exact patch. Before/After
копируют проверенную сигнатуру исходной процедуры.
ModificationAndControl/ChangeAndValidate клонирует проверенное определение
исходного метода и добавляет no-op diff scaffold. Applied flow после initial
topology preparation открывает physical destination root из retained workspace
handle, доказывает единый whole-workspace mount/volume и qualified native
backend, выводит semantic key exact canonical artifact locus и берёт отдельный
cross-process lease по collision-safe root-wide filesystem key,
повторно разрешает актуальный mapping под этим lease и держит его через запись
и все authoritative post-mutation шаги. На Windows все destination-root,
parent/temp/target operations после единственного workspace-root open остаются
handle-relative; path-based
`MoveFileExW` не является допустимым CFE backend. Task 8 создаёт plumbing seam
и typed committed/no-change/uncertain effect contract, включая name-based
staging lifecycle, durable rollback, independent durability state и detached
filesystem identities, но ещё не хранит receipts и не применяет guard policy.

**Tech Stack:** Rust; existing Task 4 source snapshots; Task 5 shared Platform
XML catalog, registration/adoption/form-binding parsers; Task 6 bounded BSL
lexer/parser; Task 7 discovery use case; `serde_json`, `sha2`, already-landed
native OS APIs; existing native adapter and parity harness. Новых parser/runtime
dependencies нет.

## Global constraints

- Source of truth: code/tests/package metadata > active spec > historical plan.
- Task 8 начинается только после GREEN Task 4, Task 5A/5B/5C, Task 6A and
  Task 7 including the back-propagations in section 2. Task 5A must have an
  accepted committed SHA; Task 5B must be freshly accepted on that SHA with no
  open P0/P1, including strict MDClasses flavor gating, Own authority, its one
  neutral Form registry and the closed EventSubscription compatibility matrix.
- Resolver не читает display text, SQLite/RLM/workspace index, ignored corpora
  или unconstrained live tree.
- `unica.cfe.patch_method` не заимствует object/form implicitly. Exact
  destination root and optional form must already be `Adopted` and UUID-bound
  to the selected analysis chain. Только отсутствующий destination descriptor
  is `destination_borrow_required`; present Own, wrong UUID and malformed or
  inconclusive material preserve their distinct Task 5A Unknown blockers.
- `ExtensionRequired` is useful discovery advice, but is never receipt-eligible
  for this tool. Only exact `ExtensionOwned` adopted binding can resolve.
- Source method existence, exact spelling, kind, context, signature and body
  come from the immutable analysis snapshot. Raw `Context` and `IsFunction`
  are optional assertions, never output truth and never synthetic defaults.
- Omitted Context/IsFunction and explicit matching assertions produce the same
  normalized arguments digest and grant. Mismatch fails before rendering.
- Direct `SourceSet|sourceSet` is optional only when filtering configured
  candidates to Configuration + PlatformXml leaves exactly one analysis source.
  Discovery/receipt analysis identity is authoritative; a raw selector, if
  present, is only an exact assertion against it.
- CFE patch analysis is exactly declared `SourceSetKind::Configuration` +
  Platform XML **and** captured `ConfigurationFlavor::BaseConfiguration`.
  An Extension analysis source remains valid for general Explore evidence, but
  its CFE mutation proposal is Unknown, receipt-ineligible and issuer-silent
  with `cfe_analysis_configuration_required`. Never compare an extension
  wrapper descriptor `@uuid` as the base metadata identity and do not introduce
  an alternative `BaseMetadataIdentity` model in v1. Every analysis root/Form
  descriptor used in the UUID join is additionally proven `Own`; a topology
  label cannot override captured XML flavor or descriptor membership.
- Destination `ScriptVariant` reuses domain `KnownScriptVariant` from the shared
  `PlatformConfigurationCatalogV1`; no `CfeScriptVariant` and no second
  Configuration.xml parser are allowed.
- Exact source method material and destination material are snapshot-bound.
  An absent destination module is authoritative only through a watched
  tombstone captured in the same initial/final scan.
- `prepare` performs zero Platform XML/BSL/snapshot/filesystem material reads.
  It may normalize arguments and configured topology and derive lexical watch
  paths only. Capture owns bounded no-follow I/O; resolve owns verified material
  interpretation.
- Every applied CFE mutation, including `off/observe/warn` without a receipt,
  opens the workspace once, resolves the destination root component-by-component
  from that handle, and acquires one non-blocking process + OS artifact mutation
  lease before its authoritative capture. The persistent control root is the
  fixed descriptor-relative `.build/unica/project-discovery/control-v1`; it has
  no path-derived workspace-key subdirectory. The semantic artifact key uses
  stable physical destination-root identity plus canonical artifact locus, but
  the actual v1 filesystem-collision/lock key deliberately over-locks the whole
  physical destination root. Thus filesystem case/Unicode aliases, including an
  absent target whose FileId does not yet exist, cannot select another lock.
  Neither key uses source-set name, map-wide mapping digest, lexical workspace
  alias, `UNICA_CACHE_DIR` or another cache override. The lease remains held through
  handler, typed effects, post-snapshot and any future receipt transition.
  Dry-run acquires no lease.
- A first patch may create only the exact missing directory suffix recorded in
  the watched parent-chain plan. No generic `create_dir_all`, unplanned parent,
  link/reparse traversal or silent adoption of a concurrently appeared parent.
- FormModule ordinary methods may use module annotations. A method named by any
  binding in Task 5B's accepted neutral, versioned complete Form projection is
  the wrong mechanism and must fail with
  `cfe_form_handler_wrong_mechanism`; incomplete or unsupported event-bearing
  Form.xml can never prove Ordinary. The destination Form.xml must independently
  prove that the generated method name is not already bound by that same
  projection.
  Task 8 does not rewrite Form.xml callType.
- Task 8 contains no Form definition/item/event/callType/Action registry,
  cardinality table or lexical policy. It imports only Task 5B's accepted
  neutral `PlatformFormRegistryVersion`, complete typed projection and lookup
  result, while Task 6 supplies the same canonical method-identifier identity.
  Task 5B must be accepted with no open P0/P1 and its registry's exhaustive
  fixtures/N/N+1 tests GREEN before Task 8 code starts.
- One plan authorizes one source method, one interceptor, one destination, one
  generated method and one destination BSL artifact. No cross-product match.
- Dry-run performs the same selection, capture, adoption, source-method,
  duplicate and render work as apply, but writes nothing and touches no receipt.
- Task 8 does not register `unica.project.discover`, persist/lease a receipt,
  enforce observe/warn/deny, or mutate public tool/package registration. The
  prerequisite backend support matrix/product CI is a required package-contract
  back-propagation. Its artifact mutation
  lease is a distinct correctness primitive and is mandatory in every applied
  rollout mode.
- `AdapterOutcome` is presentation only. Every applied CFE handler returns a
  typed mutation outcome even when `ok=false`. `NoChange` is constructible only
  with verified-clean staging and zero persistent **source-artifact mutation**
  effects; expected creation of the fixed control-plane directories/lock inode
  is a separate internal lease observation and never enters grant effects.
  If any source staging/parent was created, NoChange additionally requires a
  fully durable reverse-removal transcript; otherwise the result is Uncertain.
  `Committed` means the target commit is definite and carries exact effects,
  independent `VerifiedClean|Residue|Unknown` staging cleanup and independent
  `VerifiedDurable|Unknown` durability. `Uncertain` means target commit itself
  is not proven. Detached/relocated objects retain stable physical identity and
  never masquerade as the intended path. No post-commit, durability or cleanup
  error may collapse to a string.
- Present replacement is only cooperatively serializable. Absent installation
  is atomic no-replace. On Windows the absolute trusted path is used at most once
  to open the workspace root; destination-root traversal, both install modes,
  temp creation and parent creation are rooted one component at a time in
  retained directory handles. Path-based destination reopen/rename/create is a
  hard STOP.
- The artifact lock universe is one physical whole workspace. The retained
  workspace root, fixed control root and destination source root must remain on
  one proven per-call mount/volume instance; an internal Linux bind/mount,
  nested macOS volume, Windows reparse/mounted-volume boundary or unavailable
  proof fails before authoritative capture. Two aliases/bind views of the same
  **whole** physical workspace are allowed only because they reach the same
  physical control-root and persistent lock inode; equal key bytes alone are
  never serialization proof.
- Native mutation backends are closed allowlists. V1 has candidate local
  Linux ext4/XFS, local macOS APFS and local Windows NTFS tuples only after each
  exact kernel/OS/filesystem tuple passes its process-lock, atomic install,
  staging-lifecycle, metadata-durability, failure and crash/restart gates.
  NFS/CIFS/SMB, FUSE, ReFS, FAT/exFAT, network, nested/unidentified and any
  unqualified tuple fail `cfe_patch_atomic_backend_unsupported` before the first
  source-tree mutation; syscall availability or filesystem name alone is not
  qualification.

---

## 1. Mandatory corrections to the historical Task 8

### 1.1 A method name is not enough to generate a valid interceptor

The live writer emits `GeneratedName()` for every target. Real hooks may have
parameters, `Val`, defaults, async modifiers and procedure/function semantics.
Official extension examples preserve the intercepted signature. A
ChangeAndValidate method additionally needs the original definition and body,
not an empty TODO method.

The live writer also derives the same `NamePrefix + MethodName` declaration
name for Before and After. Those two valid hooks then collide, while the
historical acceptance text incorrectly says they can coexist. Task 8 fixes the
generator contract: Before/After always receive the destination-variant
semantic suffix defined in §9.5; ChangeAndValidate keeps the conventional
prefixed source name. The old donor name is not a compatibility contract.

Therefore the plan must bind:

- the exact analysis source set and source fingerprint;
- exact registered source module artifact and module digest;
- exactly one unconditional parsed definition with exact source spelling;
- method kind, BSL execution context, async/export facts and parameter bytes;
- definition/declaration/name/parameter/body/terminator spans, declaration line
  ending and definition/signature/body digests;
- for ModificationAndControl, exact bounded definition bytes used for cloning.

Missing, duplicate, conditional, malformed, unsupported or stale source method
material is a pre-write blocker. Task 8 must never fall back to the raw
`IsFunction`, raw `Context` or an empty parameter list.

### 1.2 Registration is not proof of borrowing

The skill's live precondition says the object must already be borrowed. A same-
name object owned by the extension can be registered and physically contain a
module, but it is not authority to intercept the analysis method.

For each root owner, and each Form owner when applicable, require:

```text
destination ObjectBelonging == Adopted
destination ExtendedConfigurationObject UUID == analysis descriptor UUID
destination registered canonical identity == requested canonical identity
```

All singleton XML fields and UUID spellings are exact and parsed by the shared
Task 5 descriptor parser. The state lattice is not collapsible: only descriptor
absence is `destination_borrow_required`; a present Own descriptor is
`destination_object_not_adopted`; a present Adopted descriptor bound to another
base UUID is `destination_extended_object_mismatch`; malformed/failed/gapped
material is `destination_membership_inconclusive` plus its exact provider/gap
reason. The latter three stay Unknown even if another root/Form pair is absent.
Nothing is repaired in Task 8. An implicit `cfe.borrow` would be a broader
multi-artifact mutation with different scope and would be unsafe for Own or
wrong-UUID material.

### 1.3 Form event/action methods use a different mechanism

Task 5B's accepted neutral, versioned complete Form projection is authoritative.
If its canonical lookup says the source target is bound, `cfe.patch_method`
must not emit
`&Before/&After/&ChangeAndValidate`. Return
`cfe_form_handler_wrong_mechanism` and keep the proposal/report visible. Only a
FormModule method whose accepted complete lookup is `Unbound` is ordinary and
eligible for the annotation writer. Any registry/parser incompleteness yields a
scoped incomplete result, never negative proof. Task 8 does not know which Form
row caused `Bound` or incomplete and therefore cannot diverge from Task 5B.

### 1.4 Prepare must precede watched capture

A final resolver request cannot already require a watch that the same resolver
is supposed to derive. The shared implementation is explicitly two-stage:

```text
raw/typed arguments + authoritative topology
    -> PreparedMutationResolution + SnapshotWatch
    -> capture_with_watches(...)
    -> ResolvedMutationPlan
```

This is one resolver implementation and one normalized seed, not two parsers.
Direct preview invokes one resolution service call that owns both stages.
Direct apply performs prepare, acquires the derived artifact lease, and only
then performs its authoritative capture+resolve. Discovery batches prepared
seeds/watches before its one shared read-only capture.

Phase ownership is strict:

| Phase | May consume | Must not consume | Output |
| --- | --- | --- | --- |
| validate/support | raw args, workspace context, configured source topology | target Platform XML/BSL bytes | accepted typed request |
| prepare | typed request, exact configured source identities, lexical registries | `SourceSnapshotPort`, `CfeResolutionMaterialPort`, direct `fs` reads, descriptor/module bytes | initial seed, watch, physical-root path candidate + canonical artifact locus |
| preview capture | bounded snapshot port | writes, leases | watched immutable snapshot |
| applied lease | retained workspace handle, component-wise control/destination opener, mount/volume + backend allowlist, opaque lock store | source material, receipt lock, process cache identity | RAII witness owning physical control/destination/lock identities + qualified backend capability |
| topology refresh | already-typed request, current configured topology, lease witness | raw args, source material | authoritative seed/watch with same physical root + canonical locus |
| authoritative capture | snapshot port over refreshed seed while artifact lease is held | writes | watched immutable snapshot |
| resolve | seed + verified captured material only | raw args, live filesystem | immutable plan |
| precommit/write | plan + artifact lease witness + optional already-acquired current receipt lease + one final same-watch recapture + descriptor-relative no-follow handles | reparsing/rerendering | typed target/effects + staging lifecycle + commit/rollback durability outcome |
| post-mutation | same artifact lease, typed handler outcome, fresh snapshot, optional future Task 10 current receipt lease | event/cache/reconciliation side effects | Task 8 records exact inputs; Task 10 later owns advance/revoke; `AdapterOutcome` is never authority |

A recording `PanicOnMaterialRead` fake must prove `prepare` performs exactly
zero snapshot/material/filesystem reads. Moving descriptor validation into
prepare is a contract failure even if the same bytes are read later.

### 1.5 Artifact serialization and first-patch parent creation are mandatory

The existing `precondition -> unconditional rename/MoveFileEx(REPLACE)` shape
does not serialize two Unica processes and cannot support a legitimately
borrowed descriptor whose `<Name>/Ext` directory has not been created yet.
Task 8 therefore adds two independent, typed preconditions:

1. an artifact mutation lease derived from the stable physical identity of the
   opened destination source root and one parser-canonical artifact locus,
   excluding lexical workspace/source aliases and the map-wide mapping digest;
   and
2. a watched parent chain from selected source root to the module's exact parent,
   with every component classified as PresentDirectory or AbsentDirectory.

The lease serializes all supported cooperating Unica writers inside one proven
physical whole-workspace universe regardless of receipt mode. A destination
root crossing an internal mount/bind boundary is outside that universe and is
rejected rather than pretending its workspace-local lock is shared.
The parent chain permits bounded ancestor-first creation of only its exact
absent suffix and reverse-order rollback of directories created by this call if
the file was not committed. These are root-cause requirements, not atomic-file
implementation details.

### 1.6 The historical file list is incomplete

Changing only `cfe_method_patch.rs`, target resolver and `cfe.rs` cannot carry a
typed plan through the current `ToolSpec + raw args + context` adapter path.
Task 8 must also change operation descriptors, application ports/composition,
native adapter/registry, source snapshots, Task 5/6 projections, discovery
issuance and dedicated CFE tool schema. Omitting those changes is a hard STOP.

### 1.7 V5 supersedes the reviewed v4 cleanup, durability, Form and backend boundary

This v5 supersedes source design SHA-256
`5b2436cbb38af24a011410769a83e9dd00fdd60abd596cef82c06b6833639a01`
and resolves every P0/P1/P2 in fresh-review SHA-256
`180a097ed71839bf198cc5f676279a6c7ce480fa243398007f4b4a22916f7431`.
The superseded v4 text is not an alternative implementation path.

The following are one correction set and must not be implemented independently:

1. artifact and receipt locks use one fixed descriptor-relative control root
   directly under the opened physical workspace, with no path-hashed workspace
   subnamespace. Semantic artifact identity remains physical destination-root
   identity plus canonical module locus, but the actual v1 filesystem-collision
   key over-locks the whole physical destination root so case/NFC/NFD and absent
   aliases cannot split it. Serialization additionally requires the same
   physical control root and persistent lock object. An internal mount/bind
   boundary is rejected; two aliases of the same whole physical workspace must
   prove one lock inode/FileId and Busy, never merely equal key bytes;
2. Windows containment opens the trusted workspace root once and is then
   handle-relative for every destination component, parent creation, temp
   creation, target precondition and atomic install;
   `MoveFileExW` and checked-then-path-based `CreateDirectoryW` are forbidden;
3. handler output separates definite target commit from cleanup state. A
   definite commit may carry VerifiedClean, Residue or Unknown cleanup and an
   independent VerifiedDurable or Unknown durability state. VerifiedClean
   accounts every created staging identity exactly once as Removed or
   ConsumedByTarget; the installed object may survive at target while its
   staging name is absent. Only exact expected durable+clean commit can advance;
4. renamed-away parents/objects retain a typed physical identity instead of
   being reported at a false intended path, while staging cleanup remains
   independent from target location;
5. a Form plan proves both that the source `MethodName` is an ordinary module
   method and that destination `generated_method_name` is unbound by consuming
   only Task 5B's accepted neutral registry/version and complete lookup. Task 8
   contains no second item/event/callType/Action table;
6. Linux/macOS/Windows backends are explicit versioned tuple allowlists with
   exact lock/install/durability primitives plus native race/failure/crash gates;
   network/FUSE/unknown tuples never fall through;
7. `NoChange` means zero persistent source-artifact mutation effects; expected
   fixed control-plane initialization is separate internal lease telemetry;
8. `Configuration`/`Extension` source labels are assertions checked against
   captured root XML flavor, and analysis object/Form descriptors must be Own
   before their UUIDs can become base authority; and
9. optional raw assertions exist only in the prepared request/diagnostics. The
   final plan stores one derived canonical core, so omission and an explicit
   matching assertion are value-equal as well as digest-equal.

Any implementation that retains only the v2 prose guarantee but lacks the
corresponding types, failure-injection tests and back-propagations is a hard
STOP.

---

## 2. Mandatory back-propagations before Task 8 code

These are accepted corrections to earlier task designs, not optional cleanup.
Update their design/spec/tests before writing the Task 8 resolver.

### 2.1 Task 5A: support projection and shared domain facts

- Add the one canonical `PlatformUuid` to
  `domain/discovery_registry.rs`, which already owns `KnownScriptVariant`,
  and `ScriptVariant`. Replace any application-only
  `MetadataUuid`; Task 5B catalogs, Task 8 plans and receipts reuse this exact
  domain type.
  - accept only hyphenated 8-4-4-4-12 ASCII hex;
  - normalize hex case to lowercase for equality/stable encoding;
  - reject nil as an adoption binding;
  - expose no unchecked string constructor.
- Move/reuse `BslExecutionContext` in the same domain module so Task 5, Task 6,
  CFE plan and receipts use one six-value type and stable tags.
- Keep `KnownScriptVariant` as the only known Russian/English type.
- Change `CfePatchMethod` support projection:
  - exact adopted UUID-bound root/form destination -> `extension_owned`;
  - absent descriptor, with every other pair AlreadyBorrowed ->
    `extension_required` plus blocker `destination_borrow_required`;
  - present Own -> `unknown` plus `destination_object_not_adopted`;
  - present Adopted with another base UUID -> `unknown` plus
    `destination_extended_object_mismatch`;
  - malformed/failed/gapped analysis or destination material -> `unknown` plus
    `analysis_metadata_identity_inconclusive` or
    `destination_membership_inconclusive` and the exact provider/gap reason;
  - Unknown takes precedence over RequiresBorrow when root/Form rows mix;
  - `extension_required` is reportable/actionable but receipt-ineligible;
  - no support fact may turn an unborrowed destination into an eligible grant.
- Add tests proving same-name extension-owned objects and cross-UUID borrowed
  objects are not `extension_owned` for CFE patch.

### 2.2 Task 5B: one Platform XML catalog and adoption/form bindings

Extend, do not duplicate, the shared pure parsers:

```rust
pub(crate) struct PlatformConfigurationCatalogV1 {
    pub(crate) flavor: ConfigurationFlavor,
    pub(crate) script_variant: ScriptVariant,
    pub(crate) name_prefix: SingletonText,
    pub(crate) registrations: Vec<RootRegistration>,
}

pub(crate) enum ConfigurationFlavor {
    BaseConfiguration,
    ExtensionConfiguration,
}

pub(crate) struct RegisteredDescriptorIdentity {
    pub(crate) artifact: ArtifactRef,
    pub(crate) descriptor_artifact: String,
    pub(crate) descriptor_digest: String,
    pub(crate) object_uuid: PlatformUuid,
    pub(crate) membership: CfeMembershipKind,
}

pub(crate) enum CfeMembershipKind {
    Own,
    Adopted {
        extended_configuration_object_uuid: PlatformUuid,
    },
}

// These three Task 5B-owned neutral types are imported, never redefined by
// Task 8. Their fields/registry rows stay private to Task 5B.
pub(crate) struct PlatformFormRegistryVersion(/* private accepted version */);

pub(crate) struct CompleteFormMethodBindings {
    registry_version: PlatformFormRegistryVersion,
    form: ArtifactRef,
    semantic_digest: String,
    _private_complete_material: (),
}

pub(crate) enum CompleteFormMethodLookup {
    Unbound,
    Bound { binding_identity_digest: String },
}

// Task 5B-owned read-only interface; Task 8 imports it with the projection.
pub(crate) trait CompleteFormMethodBindingsView {
    pub(crate) fn registry_version(&self) -> &PlatformFormRegistryVersion;
    pub(crate) fn semantic_digest(&self) -> &str;
    pub(crate) fn lookup_method(
        &self,
        method: &BslMethodIdentifierIdentity,
    ) -> CompleteFormMethodLookup;
}

pub(crate) enum SingletonText {
    Missing,
    Empty,
    Value(String),
}
```

- `name_prefix` is direct exact Configuration/Properties material, preserving
  Missing/Empty/Value rather than defaulting; a duplicate is a typed catalog
  parse error and never chooses one node.
- `flavor` is captured material, not a copy of `SourceSetKind`. The shared
  parser classifies only exact direct `Configuration/Properties` fields in the
  exact `http://v8.1c.ru/8.3/MDClasses` namespace; same-local-name unqualified,
  foreign-namespace, attribute or descendant decoys never substitute:
  `BaseConfiguration` is exactly absent `ObjectBelonging` plus absent
  `ConfigurationExtensionPurpose`; `ExtensionConfiguration` is exact
  `ObjectBelonging=Adopted` plus exactly one supported purpose (`Patch`,
  `Customization`, or `AddOn`). Purpose without belonging, Adopted without a
  valid purpose, another/duplicate belonging, duplicate purpose or mixed-content
  singleton is an inconclusive flavor view. Direct
  `ConfigurationExtensionCompatibilityMode` and
  `KeepMappingToExtendedConfigurationObjectsByIDs` are optional `0..1` on both
  flavors and are never discriminators. When present, compatibility is one
  nonempty control-free scalar of at most 256 UTF-8 bytes/128 scalars and
  KeepMapping is exact lowercase `true|false`; invalid/duplicate optional
  material makes only the flavor semantic view inconclusive. This exact table
  must accept tracked base fixtures that contain compatibility mode and the
  tracked adopted extension fixture that omits both optional fields. A source
  topology kind and captured flavor mismatch is a stable resolver/provider
  failure, never normalization. `NamePrefix` remains a separate semantic view.
- A valid flavor is an actual CFE membership/emission gate, not detached
  diagnostics. The shared provider may preserve unrelated capture-valid
  registrations for general Explore, but it cannot emit a CFE-eligible
  Configuration/Extension membership row or construct Task 8's verified catalog
  projection when flavor is absent/inconclusive or declared kind disagrees.
  Task 8 must receive the gated typed projection; it cannot parse the raw fields
  again or join UUIDs first and check flavor later.
- `PlatformUuid` accepts exactly the hyphenated 8-4-4-4-12 ASCII-hex shape,
  normalizes hex case to lowercase for equality/digest, and rejects a nil UUID
  as an adoption binding.
- Root/form descriptor parser exposes exact UUID, ObjectBelonging and
  ExtendedConfigurationObject; descendants/attributes/wrong namespace do not
  substitute for direct fields. A successfully parsed present descriptor is
  exactly `Own` or `Adopted { uuid }`; descriptor absence is the separate
  MetadataAbsent polarity. Unknown/duplicate/malformed belonging or extended
  UUID material fails the provider with its exact stable reason and never
  becomes an enum payload.
- A CFE analysis UUID may be consumed only from a descriptor parsed as
  `CfeMembershipKind::Own` under a `BaseConfiguration` catalog. An Adopted
  analysis root/Form is `analysis_descriptor_not_base_owned`; it cannot be
  reduced to plain `MetadataIdentity`. The destination catalog must be
  `ExtensionConfiguration`, and its exact object/Form descriptors remain the
  Adopted side of the join. This closes a misdeclared extension source without
  adding a second `BaseMetadataIdentity` model.
- Task 5B first extracts one neutral registry from the audited Form
  implementation and gives it a closed `PlatformFormRegistryVersion`. That
  single registry owns every definition/item/event compatibility row,
  callType/BaseForm rule, command Action cardinality, recursive completeness
  edge, identifier/opaque-ID lexical rule and limit. Form edit/validate and the
  complete binding catalog consume the same value. Task 8 is forbidden to name,
  copy, filter or extend those rows.
- `CompleteFormMethodBindings` is constructible only by the accepted Task 5B
  parser after its whole-document audit succeeds. Its only Task 8 operation is
  a canonical method-identity lookup returning `Unbound` or `Bound` with a
  bounded opaque binding digest. Unknown token/kind, illegal cardinality,
  unsupported callType/BaseForm pairing, namespace error, parser limit or any
  unconsumed binding-shaped material prevents construction; Task 8 receives
  `cfe_form_binding_inconclusive`, never an empty/partial view.
- `PlatformFormRegistryVersion` and the projection's semantic digest enter the
  Form semantic proof, grant tuple and rolling baseline. A registry upgrade
  changes the version/digest and requires fresh discovery; Task 8 never treats
  two versions as compatible merely because their current lookups match.
- The neutral registry imports Task 6's one canonical Unicode method-identifier
  identity. Task 8 converts only its requested source/generated names through
  that same service; it does not restate item/event/command/ID syntax or bounds.
  Task 5B's exhaustive matrix, lexical N/N+1, duplicate, BaseForm/callType and
  zero/duplicate-Action REDs must be accepted and GREEN first.
- Task 4 catalog selection and Task 5 providers continue using these same
  parsers. Task 8 receives typed projections through an application port.
- Task 5B preserves typed Platform XML Extension facts for general Explore and
  does not emit a source-readiness failure for them. The application CFE
  mutation preflight accepts analysis facts only from exact
  `SourceSetKind::Configuration` + PlatformXml; for an Extension it emits the
  closed Task 7/§5.4 result, leaves the proposal Unknown/ineligible and never
  sends that proposal to issuer. The Configuration descriptor
  `MetadataIdentity` remains the one base UUID authority; no
  `BaseMetadataIdentity` fact/parser is added and an extension wrapper's local
  `MetaDataObject/@uuid` is never substituted.
- Task 5B exposes complete Form binding material for both selected analysis and
  exact destination Form artifacts. The analysis projection proves the source
  method is Ordinary; the destination projection proves the generated method
  identity is unbound. Both use `CompleteFormMethodBindings`; a second Form
  parser or a BSL-only destination proof is forbidden.
- Task 5B's closed EventSubscription event/source compatibility matrix remains a
  mandatory shared-parser/provider prerequisite because the same catalog must
  not emit semantically impossible relationships. Task 8 consumes no
  EventSubscription row and adds no EventSubscription field to a CFE plan,
  digest or receipt; it neither bypasses nor reimplements that Task 5B gate.

### 2.3 Task 6: source extraction and duplicate facts

Keep one bounded lexer/parser. Extend `BslSyntaxDefinition` with validated spans
needed for exact source extraction:

```rust
// Task 6-owned opaque Unicode spelling/comparison identity reused by Task 5B/8.
pub(crate) struct BslMethodIdentifierIdentity(/* private */);

pub(crate) struct BslSyntaxDefinition {
    pub(crate) name: String,
    pub(crate) name_identity: BslMethodIdentifierIdentity,
    pub(crate) definition_span: BslSpan,
    pub(crate) declaration_span: BslSpan,
    pub(crate) name_span: BslSpan,
    pub(crate) parameter_list_span: BslSpan,
    pub(crate) body_span: BslSpan,
    pub(crate) terminator_span: BslSpan,
    pub(crate) declaration_line_ending: BslLineEnding,
    pub(crate) shape: DefinitionShape,
    pub(crate) local_shadow_names: Vec<String>,
}

pub(crate) struct BslSpan {
    pub(crate) start_byte: u32,
    pub(crate) end_byte_exclusive: u32,
}

pub(crate) enum BslLineEnding { Lf, Crlf }

pub(crate) enum ObservedCfeInterceptorKind {
    Before,
    After,
    Around,
    ModificationAndControl,
}
```

- `definition_span` contains declaration/body/terminator;
  `declaration_span` contains name and parameter-list; name and parameter-list
  do not overlap; body ends at the terminator start. Every range is ordered,
  in-bounds and validated against the exact file bytes/content digest.
- `definition_span` starts at optional Async/Procedure/Function and excludes
  preceding context/annotations; it ends after the matching terminator token.
  `declaration_span` ends after optional Export but before its line ending;
  `parameter_list_span` includes both parentheses; `body_span` starts after the
  declaration line ending and ends at the terminator start.
- Parameter-list bytes and definition bytes are sliced only from
  `read_verified()` material; they are not accepted from an untrusted cache.
- The CFE projection copies validated offsets into domain `BslByteRange` with
  checked u32 arithmetic; it never recomputes offsets by searching text.
- Retain well-formed interceptor annotation facts with exact bounded string
  argument, attached definition and conditional/deleted status for destination
  duplicate preflight.
- Export the one parser-owned canonical Unicode identifier identity for every
  definition and annotation target. Task 8 and the shared Form catalog use that
  same identity when comparing a generated handler; neither may implement a
  second lowercase/case-fold routine.
- The observed annotation catalog is closed and bilingual:
  `&Перед|&Before -> Before`, `&После|&After -> After`,
  `&Вместо|&Around -> Around`, and
  `&ИзменениеИКонтроль|&ChangeAndValidate -> ModificationAndControl`.
  `ObservedCfeInterceptorKind` is separate from the three-value requested
  `CfeInterceptorType`; its explicit stable tags are Before=1, After=2,
  Around=3, ModificationAndControl=4. It must not reuse requested enum order,
  whose ModificationAndControl tag remains 3.
- A conditional/unsupported/malformed source target cannot supply extraction.
- Task 6 cache keys remain parser-contract + content digest. If DTO shape is
  persisted, bump its schema explicitly; never accept old spans as v2.

### 2.4 Task 7: optional assertions and resolver-ready issuance

- Change `CfePatchMethodArguments.Context` and `IsFunction` from defaulted
  values to `Option`. Absence means derive from the exact source definition.
- Keep those Options only in `PreparedCfeMethodPatchRequest`. Once resolve has
  checked them, construct one `ResolvedCfeMethodPatchCore` with derived
  context/kind/async and no assertion-presence bits. Omitted and explicit
  matching assertions must produce value-equal final plans, not merely equal
  hashes. Assertion status belongs to bounded preview/report diagnostics and is
  non-authoritative.
- Proposal pre-validation compares tool/module/method/interceptor and defers
  context/kind authority to the Task 8 resolved plan.
- Carry the request's exact resolved analysis source into every CFE preparation;
  mutation intent still forbids a nested SourceSet key.
- Pre-validation requires that authoritative analysis source to be exact
  Configuration + PlatformXml. Extension analysis keeps its report/proposal but
  receives `cfe_analysis_configuration_required`, Unknown support,
  `receiptEligibility=false`, and zero issuer calls.
- This is an application mutation preflight result, not source readiness.
  Materialize exactly one closed check per canonical affected chunk:
  `code=mutation_preflight`, `provider=DiscoveryPreflight`, `state=skipped`,
  `outcome=inconclusive`, `coverage=unknown`, `severity=blocking`, sorted unique
  nonempty `affects=["proposal:<id>", ...]`,
  `reasonCode=cfe_analysis_configuration_required`, `retryable=false`,
  `details=[]`, `evidenceIds=[]`. Do not attach candidate affects, provider
  evidence or retry guidance and do not relabel it `source_readiness`.
- Prepare all CFE seeds/watches using a dependency surface that has no snapshot,
  material or filesystem-read port. Discovery performs one bounded capture only
  after every prepare succeeds; direct apply acquires its physical artifact
  lease after initial prepare, refreshes current topology under that same lease,
  and only then performs authoritative capture.
- After Task 8 lands, issuer assessment receives sorted
  `ResolvedProposalMutationPlan { proposal_id, plan }`; no plan means no grant.
- `extension_required` plus `destination_borrow_required` remains a useful
  report result but makes receipt eligibility false.
- Existing Task 7 report/evidence output remains available when resolution is
  blocked; no fake partial grant vector is issued.

### 2.5 Active spec, ADR, skill and historical roadmap

Before code, update:

- `spec/architecture/extension-point-discovery.md`;
- `spec/decisions/0008-project-discovery-and-discovery-receipts.md`;
- `plugins/unica/skills/cfe-patch-method/SKILL.md`;
- `tests/ci/test_product_contracts.py`;
- package/CI support matrix for qualified atomic backend tuple digests;
- outcome schema/observability/replay documentation for
  `unica.mutation-outcome.v2`;
- historical Task 5A/5B/6/7/8/9/10 text where it would direct a later worker to
  reintroduce defaults, implicit borrowing or a duplicate parser.

Required spec/skill clauses:

- SourceSet optional selector/assertion behavior;
- exact Configuration-only CFE analysis preflight, while Extension analysis is
  still reportable in general Explore but never mutation/receipt eligible;
- captured Configuration/Extension flavor proof, declared-kind mismatch
  rejection, Own-only analysis root/Form UUID authority, and the fixture-proven
  flavor table: compatibility/KeepMapping are optional validated values on both
  flavors and never discriminators; strict MDClasses namespace and valid flavor
  gate CFE membership/eligibility emission before UUID joins;
- closed `DiscoveryPreflight/mutation_preflight` skipped+inconclusive+unknown+
  blocking tuple with proposal-only affects and no evidence/details/retry;
- Context/IsFunction derive-on-absence semantics and canonical final-plan value
  equality for omitted versus explicit-matching assertions;
- exact signature/body cloning;
- source wrong-mechanism rejection plus destination generated-handler
  collision/absence proof from the same accepted Task 5B neutral registry
  version and complete shared projection; Task 8 contains no registry table;
- already-borrowed UUID chain and no implicit borrow;
- `ExtensionRequired` is CFE-patch receipt-ineligible;
- SnapshotWatch, digest fields, limits and stable reasons;
- accepted Task 5B neutral Form registry/version with exhaustive compatibility,
  callType/BaseForm, Action-cardinality, lexical and completeness tests; the
  spec points to that authority and does not copy its rows into Task 8;
- exact Own/Adopted/missing/mismatch/inconclusive lattice and guidance that
  `/cfe-borrow` is appropriate only for the absent-descriptor branch;
- observed Before/After/Around/ModificationAndControl conflict matrix;
- typed parent-chain creation/rollback and allowed directory effects;
- separate semantic artifact identity (physical destination-root + canonical
  locus) and backend-qualified filesystem-collision lock key. V1 intentionally
  maps every locus below one physical destination root to one over-lock inode,
  so filesystem NFC/NFD/case aliases and absent targets cannot split it; fixed
  lock order, topology refresh under lease, the cooperative boundary of Present
  replacement versus no-replace Absent, and a future narrowing only through an
  ADR with mixed-version dual-lock/migration proof;
- one fixed `.build/unica/project-discovery/control-v1` root opened below the
  retained physical workspace, with no `<workspaceKey>` layer and no input from
  lexical workspace alias, source-set name, map-wide mapping digest,
  `UNICA_CACHE_DIR`, cwd or PID;
- one conservative whole-workspace lock universe: Linux `STATX_MNT_ID`, macOS
  `st_dev+(f_fsid,f_mntonname)` and Windows VolumeSerial/reparse checks reject every internal
  destination/control mount boundary before capture; whole-workspace aliases
  must prove the same control-root and lock inode/FileId plus actual Busy, never
  only equal key bytes;
- `unica.unix-contained-atomic.v1` and
  `unica.windows-contained-atomic.v1` closed tuple allowlists with exact lock,
  no-replace/replace, staging-name and durability primitives; network/FUSE/
  unknown tuples fail before source mutation and filesystem names alone confer
  no support. macOS APFS case-sensitive and case-insensitive rows are distinct
  exact `fpathconf(fd, _PC_CASE_SENSITIVE)` tuples, while both reuse the same
  root-wide collision protocol and NFC/NFD Present/Absent two-process proof;
- Windows handle-relative destination-root/parent/temp/target operations after
  one trusted workspace open, explicit backend version/support allowlist, and
  rejection of path-based destination reopen, `MoveFileExW` and
  checked-then-create containment claims;
- typed NoChange/Committed/Uncertain handler outcomes; definite target commit
  separated from name-based `VerifiedClean|Residue|Unknown` cleanup and
  independent `VerifiedDurable|Unknown` durability; Removed versus
  ConsumedByTarget staging lifecycle; exact expected/unexpected/possible and
  detached physical effects; NoChange scoped only to source artifacts; and
  receipt revocation even when presentation outcome reports failure;
- RU/EN renderer and one-plan plumbing.

The historical Task 9 plan must delete `${cache_root}`, path-derived workspace
keys and `MoveFileExW`: Task 8 owns the first safe BSL atomic writer, fixed
workspace control-root opener, semantic artifact key and collision-safe
root-wide physical artifact-lock key. Task 9 reuses
those primitives, adds only receipt record/lease namespaces under the same fixed
root, and owns persistent receipt revision tests. The historical Task 10 plan,
active spec and ADR must replace receipt-first guard order with artifact lease ->
refreshed current plan -> optional current receipt lease, and replace its weak
display-derived `MutationEffects` with the exact Task 8 outcome/cleanup/detached
types, staging lifecycle and durability field. The skill must remove synthetic Context/IsFunction defaults and describe
busy/retry, exact Form/flavor and failure boundaries. None of these documents may
remain contradictory when Task 8 production code starts.

The back-propagation gate is exact:

| Source | Stale contract that must disappear | Required replacement |
| --- | --- | --- |
| active architecture spec | receipt lease is first; no physical/mount/backend/collision/durability scope | artifact open/mount/backend/root-wide-collision lease/refresh first, then optional receipt lease; fixed control root; staging lifecycle + durability |
| ADR 0008 | receipt-only serialization, canonical-locus lock and receipt-first post flow | artifact->receipt order, reverse release, physical control/root-wide-collision lock universe, mixed-version migration gate, durable+clean-only advance |
| CFE skill | Context defaults; duplicated/incomplete Form prose; no backend boundary | derive assertions; consume accepted Task 5B registry version; busy/mount/backend/durability diagnostics |
| Task 5B design/implementation | multiple Form registries or open P0/P1; unclosed EventSubscription compatibility | one accepted neutral versioned registry shared by edit/validate/catalog; strict flavor/Own/EventSubscription gates GREEN |
| historical Task 9 | `${cache_root}/.../<workspace-key>`, semantic/canonical-locus artifact lock, `MoveFileExW`, old effect schema, store/lease tests attributed to Task 8 | reuse exact Task 8 control/mount/backend/root-wide collision lease; persist generic outcome schema v2 only; Task 9 alone owns store/revision/receipt lease |
| historical Task 10 | display-shaped effects, receipt-first handler order, clean-only-without-durability advance | exact target/staging/durability authority, artifact-held current plan, durable+clean-only production advance/revoke |
| observability/replay | infer success from display or matching post-manifest; control files mixed with grant effects | preserve schema v2 target/cleanup/durability dimensions; control initialization is separate bounded telemetry |
| package/CI support | generic Unix/Windows filesystem support, syscall-presence fallback or lock filename derived from semantic path bytes | exact qualified tuple digests, root-wide collision protocol and native path/case/NFC/NFD lock/race/failure/crash gates; absent row fails before source write |

`tests/ci/test_product_contracts.py` must fail on each stale cell and pass on each
replacement before any Task 8 production slice. Updating only this design does
not satisfy the gate.

### 2.6 Task 9/10: immutable grants versus rolling baseline

- Task 9 `DiscoveryGrantV1` stores the complete atomic scope tuple and
  `grantScopeDigest`, including the exact allowed file plus bounded stable
  parent-directory creation scope, not the current absent suffix, destination
  fingerprint or module precondition.
- That grant tuple also stores captured Base/Extension flavor tags, exact Own
  analysis + Adopted destination UUID chains, analysis Ordinary Form semantic
  proof and destination generated-name-unbound Form semantic proof, including
  the exact accepted Task 5B `PlatformFormRegistryVersion`. Task 9
  baseline stores both exact Form material identities; Task 10 re-proves the
  semantic tuple and compares the refreshed materials before any advance.
- Task 9/10 consume only `ResolvedCfeMethodPatchCore`; neither grant storage nor
  guard comparison has a field for optional assertion presence or alias
  spelling. Equal omitted/explicit-matching calls therefore compare as the same
  plan value, not merely the same digest.
- Receipt `baseline` separately stores sorted source snapshots and composite
  fingerprint. Issuance validates every plan against that one initial baseline.
- Task 9 reuses one `WorkspaceDiscoveryControlRootV1` already opened by Task 8
  from the retained physical workspace handle and fixed relative suffix
  `.build/unica/project-discovery/control-v1`. There is no workspace-key
  subdirectory. Artifact locks, receipt locks and receipt records use that root;
  `WorkspaceContext.cache_root`, `UNICA_CACHE_DIR`, cwd, lexical workspace alias
  and caller configuration are not identity inputs. Every control component is
  opened/created no-follow below the retained workspace handle. Task 9 reuses
  Task 8's same-mount/volume capability proof, semantic-key/collision-key split
  and exact root-wide lock inode; it never creates a second artifact lock
  universe or hashes a semantic locus into an artifact lock filename. Receipt
  records remain workspace-scoped; a destination mounted
  from another workspace is rejected before capture rather than receiving a
  receipt/control-root fallback.
- Task 10 follows one lock order for every applied enforceable call:
  `artifact mutation lease -> current receipt lease`; release is
  `current receipt lease -> artifact mutation lease`. It never acquires an
  artifact lease while holding any receipt lease and never acquires a second
  receipt while either is held.
- Under the already-held artifact lease Task 10 captures/resolves one current
  plan, then acquires the current receipt lease, rereads revision + baseline
  under it, and requires that baseline to equal the plan capture before
  comparing one exact grant scope. The current `executionPlanDigest` is only for
  same-call precondition/audit. `off/observe/warn` without a receipt still uses
  the artifact lease.
- After successful in-scope A, Task 10 captures post snapshot and advances the
  baseline/revision while both leases remain held and leaves all grant
  tuples/scope digests unchanged.
- A later B resolves again over the advanced baseline. It may have a different
  execution digest/module boundary but must match the same immutable B grant.
- If A changed an immutable B scope field (source definition, adoption UUID,
  ScriptVariant, NamePrefix, generated identity or allowed effects), B's scope
  no longer matches and is denied/revoked; no grant is rewritten to hide drift.
- Add a two-grant same-destination-module persistent revision RED test to Task
  9/10/spec before the overall feature is considered integration-ready; Task 8
  itself covers only the corresponding pure algebra.
- Task 10 typed effects must distinguish `CreatedDirectory`, `CreatedFile`,
  `UpdatedFile`, `OwnedStagingFile` and
  `DetachedOrRelocatedFilesystemObject`. Every path-resolved created directory
  must equal the execution plan's current `directories_to_create`, ancestor-
  first, and fall inside the grant's stable creation scope; any detached identity
  is unexpected and non-advancing even if commit is definite. Post-manifest diff
  and typed effects must agree before receipt advance. Reconciliation/event/cache
  work begins only after both current leases are released.
- Task 10 consumes typed mutation outcome on both adapter success and failure.
  `NoChange` with verified clean staging permits no receipt transition;
  `Committed` advances only when durability is `VerifiedDurable`, cleanup is
  `VerifiedClean`, every staging identity is exactly Removed or
  ConsumedByTarget, expected effects exactly equal the plan,
  unexpected/possible sets are empty and the post-manifest agrees. `Committed`
  with durability Unknown, Residue/Unknown, any unexpected/possible/detached
  effect and every
  `Uncertain` outcome revoke the current receipt before lease release. A
  lower-level atomic-writer `Result::Err` is reserved for failures proven to
  occur before the first source mutation and must carry `VerifiedClean` plus
  `NoChangeSourceTreeProof::Unmodified`;
  the native handler maps it to `HandlerOutcome` with typed `NoChange` rather
  than dropping mutation authority.
- Task 10 never discovers staging residue from display text or a catalog that
  may ignore random temp names. It consumes the exact internal typed path and
  identity returned by the writer and verifies cleanup while the artifact lease
  is still held.
- Task 9 persists only the generic `unica.mutation-outcome.v2` schema boundary:
  sorted expected/unexpected/possible typed effects with opaque object identity,
  every staging `OwnedStagingIdentity`, Removed/ConsumedByTarget tag,
  `target_effect_digest` and cleanup tag, committed backend-qualified durability
  state/proof whose transcript binds every required committed staging-namespace
  sync, and NoChange
  Unmodified/RolledBackDurably backend/proof fields. It stores the exact closed
  tags, digest encodings and `MAX_MUTATION_EFFECTS`/`MAX_OWNED_STAGING` bounds,
  not display text, paths derived from detached labels or an advance boolean. It
  does not choose advance policy. Task 10 production replay and
  observability preserve all three independent dimensions and never infer
  durability from a matching live post-manifest or `AdapterOutcome.ok`.
- Fixed control-plane initialization is recorded in a separate bounded internal
  lease-observation channel. Task 9/10 never compare its directories/lock inode
  with grant-authorized source effects, and `NoChange` may coexist with expected
  first-call control initialization while source effect vectors stay empty.

Task ownership is executable, not editorial: Task 8 final verification runs
only resolver/discovery determinism, pure grant/effect algebra, artifact lease,
native writer and recording-fake suites. Task 9 owns persistent receipt store,
revision and receipt-lease tests. Task 10 owns the production guard order,
advance/revoke/reconciliation pipeline and `discovery_receipts` integration
suite. A Task 8 fake must not be cited as production receipt proof.

---

## 3. Exact file map

### Domain

- Create: `crates/unica-coder/src/domain/cfe_method_patch.rs`
  - closed interceptor/method/module values, identifiers, seed/material/write
    plans, stable encoders and immutable resolved plan.
- Modify: `crates/unica-coder/src/domain/mod.rs`.
- Modify: `crates/unica-coder/src/domain/discovery_registry.rs`
  - reuse exact metadata/module registries and `KnownScriptVariant`;
  - domain-own `BslExecutionContext`, canonical `PlatformUuid` and descriptor
    belonging types; add shared `ConfigurationFlavor`; remove any
    application-only UUID/flavor representation.
- Create: `crates/unica-coder/src/domain/mutation_effects.rs`
  - generic NoChange/Committed/Uncertain state, exact typed effects, effect
    confidence, name-based Removed/ConsumedByTarget staging lifecycle,
    `VerifiedClean|Residue|Unknown` cleanup, independent
    `VerifiedDurable|Unknown` durability, detached/relocated object identity,
    v2 schema tags and uninhabitable-state constructors.
- Modify: `crates/unica-coder/src/domain/source_snapshot.rs`
  - SnapshotWatch, typed parent-component outcomes, watched tombstone and stable
    fingerprint tags.

### Application resolver and invocation plumbing

- Create: `crates/unica-coder/src/application/discovery_guard/mod.rs`.
- Create:
  `crates/unica-coder/src/application/discovery_guard/target_resolvers/mod.rs`
  - resolver kinds, prepare/resolve port, expected binding, plan enum.
- Create:
  `crates/unica-coder/src/application/discovery_guard/target_resolvers/cfe_patch_method.rs`
  - alias extraction, seed preparation, exact assertions, plan construction.
- Modify: `crates/unica-coder/src/application/ports.rs`
  - `ArtifactMutationLeasePort`, physical destination-root opener, separate
    semantic/collision keys,
    unforgeable lease witness,
    `ResolvedMutationCall`, `HandlerInvocation`, direct resolution service and
    material/precondition ports; extend `HandlerOutcome` with mandatory typed
    mutation outcome for applied CFE calls; recording fakes.
- Modify: `crates/unica-coder/src/application/operation_descriptors.rs`
  - only `cfe-patch-method` has `CfeMethodPatchV1` resolver.
- Modify: `crates/unica-coder/src/application/tool_contracts.rs`
  - dedicated CFE schema with seven alias pairs and no synthetic defaults.
- Modify: `crates/unica-coder/src/application/mod.rs`
  - support first; prepare; for apply open physical destination root, acquire
    artifact lease, refresh current mapping/seed under that lease, then perform
    authoritative capture/resolve; retain owned plan + lease and pass the same
    references through handler and future post-mutation seam; full CFE dry-run
    without lease.

### Discovery/back-propagated application contracts

- Modify: `crates/unica-coder/src/application/discovery/contract.rs`
  - reuse domain interceptor/context types; Context/IsFunction become Option.
- Modify: `crates/unica-coder/src/application/discovery/model.rs`
  - move shared BSL context; adoption/form binding facts; validate the closed
    `DiscoveryPreflight/mutation_preflight` check tuple accepted in §2.
- Modify: `crates/unica-coder/src/application/discovery/proposal_validator.rs`
  - exact Configuration mutation preflight, adoption/form-mechanism blockers.
- Modify: `crates/unica-coder/src/application/discovery/ports.rs`
  - capture-with-watches, CFE material projection and resolved issuance rows.
- Modify: `crates/unica-coder/src/application/discovery/use_case.rs`
  - emit proposal-only mutation-preflight checks, prepare intents before capture,
    resolve after capture, all-or-nothing plans.
- Modify: `crates/unica-coder/src/application/discovery/determinism.rs`
  - optional assertion encoding and final resolved-plan/grant digests.
- Modify: `crates/unica-coder/src/application/discovery/mod.rs`
  - focused mechanism and issuance integration tests.

### Infrastructure

- Modify: `crates/unica-coder/src/infrastructure/project_sources.rs`
  - exact analysis selector and ExtensionPath-to-destination mapping.
- Modify: `crates/unica-coder/src/infrastructure/source_snapshot.rs`
  - watched present/absent capture in initial/final scans.
- Modify Task 5 landed Platform XML parser/provider files
  - shared ConfigurationFlavor + NamePrefix, closed Own/Adopted membership and
    complete analysis/destination Form binding projections using the one neutral
    accepted `PlatformFormRegistryVersion`; Task 8 adds no Form registry row.
- Modify Task 6 landed files:
  `crates/unica-coder/src/infrastructure/discovery/bsl/{lexer,parser}.rs`
  - all retained extraction spans/line ending and bilingual observed
    Before/After/Around/ModificationAndControl annotation facts; no second lexer.
- Create:
  `crates/unica-coder/src/infrastructure/discovery/cfe_resolution_material.rs`
  - snapshot-only implementation of `CfeResolutionMaterialPort` using shared
    Task 5/6 parsers.
- Modify: `crates/unica-coder/src/infrastructure/native_operations.rs`.
- Modify:
  `crates/unica-coder/src/infrastructure/native_operations/registry.rs`.
- Modify: `crates/unica-coder/src/infrastructure/native_operations/cfe.rs`
  - typed preview/apply only; delete raw CFE path/context/renderer fallback.
- Create: `crates/unica-coder/src/infrastructure/workspace_locks.rs`
  - one retained physical-workspace handle, the fixed descriptor-relative
    control root with no workspace-key subdirectory, persistent-inode process +
    exact platform OS lease utility and separate artifact/future receipt
    namespaces; it proves per-call mount/volume containment, physical control-
    root and lock-object identities, must not consume
    `WorkspaceContext.cache_root`, and is reused by Task 9.
- Create:
  `crates/unica-coder/src/infrastructure/artifact_mutation_leases.rs`
  - open the destination root component-by-component; reject an internal
    mount/bind/reparse boundary; derive stable physical destination/control/lock
    identities; combine destination identity with canonical locus only for the
    semantic key and derive the actual root-wide collision lock key without
    locus bytes;
    prove the actual common lock inode/FileId and implement process/OS guards.
- Extract/create: `crates/unica-coder/src/infrastructure/atomic_file.rs`
  - contained same-directory BSL Present cooperative replace, Absent no-replace
    install, exact parent suffix creation/rollback, Removed/ConsumedByTarget
    staging lifecycle, typed durability and closed Linux ext4/XFS, both macOS
    APFS case-mode and Windows NTFS capability gates; Windows is handle-relative and never uses
    `MoveFileExW`; reused later by Task 9.
- Modify: `crates/unica-coder/src/infrastructure/runtime_jobs.rs`
  - reuse extracted primitive without runtime-job behavior drift.

### Tests/fixtures/docs

- Create: `tests/fixtures/cfe_method_patch/**`
  - registered Configuration analysis + adopted Extension destination
    Russian/English fixtures and Extension-analysis wrapper UUID decoys;
  - procedure/function parameters/defaults/async; owner/common/form ordinary;
  - accepted Task 5B complete Form lookup Bound/Unbound and registry-version
    mismatch; same-name own and wrong-UUID decoys;
  - present/absent destination, absent parent suffix, Around duplicates,
    source and destination Task 5B complete projection identities,
    misdeclared Configuration/Extension flavor, tracked base-with-compat and
    extension-without-compat/KeepMapping cases,
    unrelated-mapping/source/workspace-alias and APFS NFC/NFD/case-mode lease
    cases, retained-parent rename,
    and external-race/effect-failure cases.
- Modify: `tests/ci/test_unica_mcp_script_parity.py`
  - retain semantic donor comparison for registered Russian Before/After but
    assert the intentional collision-free name/signature differences; invalid
    donor behavior is native rejection, not fake byte parity.
- Modify active spec/ADR/skill/product tests listed in §2.5.

### No production implementation in Task 8

- public `unica.project.discover` registration;
- receipt persistence/revision, receipt lease and guard policy (the separate
  artifact mutation lease is explicitly in Task 8 scope);
- observation-journal storage/service implementation, public tool registration,
  provenance publication and release execution. This does **not** waive the
  mandatory Task 8 backprop of the schema-v2 observability/replay contract or
  the exact backend support matrix/product-CI gates into active spec, ADR,
  package-contract documentation and tests listed in §2.5;
- implicit `cfe.borrow` or Form.xml event/callType mutation.

---

## 4. Domain contract

### 4.1 Reused and closed semantic values

Do not create `CfeScriptVariant`. Reuse:

```rust
pub(crate) enum KnownScriptVariant { Russian, English }

pub(crate) enum BslExecutionContext {
    ModuleDefault,
    AtServer,
    AtClient,
    AtServerNoContext,
    AtClientAtServer,
    AtClientAtServerNoContext,
}
```

Task 8 adds/moves:

```rust
pub(crate) enum CfeMutationClass { MethodPatch }

pub(crate) enum CfeInterceptorType {
    Before,
    After,
    ModificationAndControl,
}

pub(crate) enum ObservedCfeInterceptorKind {
    Before,
    After,
    Around,
    ModificationAndControl,
}

pub(crate) enum CfeMethodKind { Procedure, Function }

pub(crate) enum CfeModuleKind {
    Module,
    ObjectModule,
    ManagerModule,
    RecordSetModule,
    ValueManagerModule,
    CommandModule,
}
```

`CfeInterceptorType` is the moved domain form of the existing discovery
`InterceptorType`, not a second enum. Contract deserialization, intent
determinism, plans and receipts all use its one stable tag implementation.

Stable tags are explicit, never enum order:

| Type | Tags |
| --- | --- |
| interceptor | Before=1, After=2, ModificationAndControl=3 |
| observed interceptor | Before=1, After=2, Around=3, ModificationAndControl=4 |
| method kind | Procedure=1, Function=2 |
| script variant | Russian=1, English=2 (existing known tags) |
| BSL context | ModuleDefault=1, AtServer=2, AtClient=3, AtServerNoContext=4, AtClientAtServer=5, AtClientAtServerNoContext=6 |
| module identity | Common=1, Owner=2, Form=3 |
| module kind | Module=1, ObjectModule=2, ManagerModule=3, RecordSetModule=4, ValueManagerModule=5, CommandModule=6 |
| configuration flavor | BaseConfiguration=1, ExtensionConfiguration=2 |

Before/After require a parsed source Procedure. Parsed source Function requires
ModificationAndControl. `IsFunction` never changes this fact.
`ObservedCfeInterceptorKind` is destination syntax evidence, not a public
request value: Task 8 cannot generate Around, but must recognize it to avoid
adding a conflicting interceptor.

### 4.2 Identifier and exact module identity

`CfeIdentifier` is owned/case-preserving:

- 1..=512 UTF-8 bytes and 1..=128 Unicode scalars;
- first scalar Unicode alphabetic or `_`; rest alphanumeric or `_`;
- no whitespace/control/dot/slash/backslash/colon;
- no trim, normalization or rewrite of the retained spelling;
- equality/ordering and duplicate detection use the one Task 6/`ArtifactRef`
  scalar-by-scalar Unicode-lowercase comparison key. NFC/NFD/NFKC are not
  applied, so canonically equivalent but byte-distinct spellings remain
  distinct unless that exact shared service later changes under a version bump.

```rust
pub(crate) enum CfeModuleIdentity {
    Common { name: CfeIdentifier },
    Owner {
        owner_kind: CanonicalMetadataKind,
        owner_name: CfeIdentifier,
        module_kind: CfeModuleKind,
    },
    Form {
        owner_kind: CanonicalMetadataKind,
        owner_name: CfeIdentifier,
        form_name: CfeIdentifier,
    },
}

pub(crate) struct CanonicalArtifactLocusV1 {
    module: CfeModuleIdentity,
    destination_relative_artifact: String,
}
```

Exact accepted raw shapes:

```text
<CommonModule tag-or-directory>.<Name>
<registered tag-or-directory>.<Name>.<registered module kind>
<registered tag-or-directory>.<Name>.Form.<FormName>
```

Structural tag/directory/Form/module tokens normalize ASCII-case-insensitively
through the one shared registry. User names preserve exact source spelling.
Extra/empty/traversal/separator/control segments fail before filesystem access.
The canonical method target must pass existing `ArtifactRef` grammar.
`CanonicalArtifactLocusV1` is constructed only by the shared registry after the
destination module artifact round-trips below the selected destination source
root. Its stable encoder is domain-separated and contains the closed canonical
module identity plus the source-root-relative slash artifact. It never contains
source-set name, `ResolvedSourceSet.mapping_digest`, absolute/root spelling,
workspace alias or caller `ExtensionPath`. Structural and case/path aliases that
resolve to one registered module therefore produce one value; a filesystem case
collision fails before a locus can be constructed.

```text
canonicalArtifactLocusDigest = SHA256(
  "unica.cfe-artifact-locus.v1\0" ||
  u64be(canonicalModuleIdentityBytes.len) || canonicalModuleIdentityBytes ||
  u64be(destinationRelativeArtifactBytes.len) || destinationRelativeArtifactBytes)
```

`canonicalModuleIdentityBytes` is the exact tag/length-prefixed encoding reused
by §11 normalized arguments. The artifact bytes are the parser-canonical slash
form. No filesystem/platform case folding is applied after registry resolution.
This digest is semantic plan identity only, not a filesystem collision key.
In particular, byte-distinct NFC/NFD or case spellings may still address one
entry on a qualified filesystem, and an absent entry has no FileId. §8 therefore
derives the separate root-wide `FilesystemArtifactCollisionKey`; hashing this
semantic locus into a lock filename is forbidden even when a prior case-collision
scan succeeded.

### 4.3 Prepared assertions and canonical resolved core

```rust
pub(crate) struct PreparedCfeMethodPatchRequest {
    module: CfeModuleIdentity,
    method_name: CfeIdentifier,
    interceptor: CfeInterceptorType,
    context_assertion: Option<BslExecutionContext>,
    method_kind_assertion: Option<CfeMethodKind>,
}

pub(crate) struct ResolvedCfeMethodPatchCore {
    module: CfeModuleIdentity,
    method_name: CfeIdentifier,
    interceptor: CfeInterceptorType,
    context: BslExecutionContext,
    method_kind: CfeMethodKind,
    is_async: bool,
}

pub(crate) struct CfeAssertionDiagnostics {
    source_set_was_asserted: bool,
    context_was_asserted: bool,
    method_kind_was_asserted: bool,
}
```

Only four public Context spellings can be asserted:

```text
НаСервере -> AtServer
НаКлиенте -> AtClient
НаСервереБезКонтекста -> AtServerNoContext
НаКлиентеНаСервереБезКонтекста -> AtClientAtServerNoContext
```

`ModuleDefault` and `AtClientAtServer` are still valid derived source facts and
can be used when Context is omitted. They are not guessed aliases. `IsFunction`
maps `true -> Function`, `false -> Procedure` only when present.

`PreparedCfeMethodPatchRequest` exists only through resolve. After exact source
facts validate every present assertion, the resolver constructs
`ResolvedCfeMethodPatchCore`; there is no Option or assertion-presence bit in
the final plan, grant tuple or any canonical encoder. `CfeAssertionDiagnostics`
is returned beside preview/report presentation, never inside
`ResolvedMutationPlan`, expected binding, receipt or handler authority. Thus
omitted and explicit matching assertions produce structurally value-equal final
plans as well as equal digests.

### 4.4 Prepared resolution seed

```rust
pub(crate) struct CfeMethodPatchSeed {
    version: u16, // 1
    request: PreparedCfeMethodPatchRequest,
    analysis: ResolvedSourceSet,
    destination: ResolvedSourceSet,
    analysis_configuration_artifact: String,
    destination_configuration_artifact: String,
    analysis_module_artifact: String,
    destination_module_artifact: String,
    analysis_owner_artifacts: Vec<String>,
    destination_owner_artifacts: Vec<String>,
    analysis_form_binding_artifact: Option<String>,
    destination_form_binding_artifact: Option<String>,
    artifact_locus: CanonicalArtifactLocusV1,
    watch: SnapshotWatch,
    expected: NormalizedExpectedBinding,
}

pub(crate) enum PreparedMutationResolution {
    CfeMethodPatch(CfeMethodPatchSeed),
}
```

The seed has no source method kind/context/body yet and therefore no final
normalized arguments digest. It is immutable and contains the one prepared
request with non-authoritative assertion Options; resolve never returns to raw
arguments and never copies those Options into the final plan. For Applied flow,
the initial seed is only a lease candidate. After lease acquisition the resolver
re-evaluates current configured topology from the already-typed request and
constructs one fresh authoritative seed; it may update mapping digests but must
retain the same opened physical destination-root identity and exact
`artifact_locus`, otherwise it fails before capture and never switches keys.

### 4.5 Source method, adoption and write plan

```rust
pub(crate) struct CfeSourceMethodPlan {
    analysis_source_fingerprint: String,
    analysis_configuration_artifact: String,
    analysis_configuration_length: u64,
    analysis_configuration_digest: String,
    analysis_configuration_flavor: ConfigurationFlavor,
    analysis_script_variant: KnownScriptVariant,
    module_artifact: String,
    module_length: u64,
    module_digest: String,
    parser_contract: &'static str,
    definition_span: BslByteRange,
    declaration_span: BslByteRange,
    name_span: BslByteRange,
    parameter_list_span: BslByteRange,
    body_span: BslByteRange,
    terminator_span: BslByteRange,
    declaration_line_ending: BslLineEnding,
    definition_digest: String,
    signature_digest: String,
    body_digest: String,
    exact_method_name: CfeIdentifier,
    method_kind: CfeMethodKind,
    context: BslExecutionContext,
    is_async: bool,
    is_exported: bool,
    form_safety: CfeFormBindingSafetyPlan,
    analysis_descriptor_chain: Vec<DescriptorIdentityDigest>,
}

pub(crate) struct BslByteRange {
    start_byte: u32,
    end_byte_exclusive: u32,
}

pub(crate) enum CfeFormBindingSafetyPlan {
    NotForm {
        semantic_digest: String,
    },
    OrdinaryAndDestinationNameUnbound {
        registry_version: PlatformFormRegistryVersion,
        source_method_identity: String,
        generated_method_identity: String,
        analysis_semantic_digest: String,
        destination_semantic_digest: String,
        analysis_material: VerifiedMaterialIdentity,
        destination_material: VerifiedMaterialIdentity,
    },
}

pub(crate) struct VerifiedMaterialIdentity {
    artifact: String,
    byte_length: u64,
    content_digest: String,
}

pub(crate) struct CfeAdoptedDestinationPlan {
    destination_source_fingerprint: String,
    descriptor_chain: Vec<AdoptedDescriptorBinding>,
    configuration_artifact: String,
    configuration_length: u64,
    configuration_digest: String,
    configuration_flavor: ConfigurationFlavor,
    script_variant: KnownScriptVariant,
    name_prefix: String,
    generated_method_name: CfeIdentifier,
}

pub(crate) struct DescriptorIdentityDigest {
    artifact: ArtifactRef,
    descriptor_artifact: String,
    object_uuid: PlatformUuid,
    membership: CfeMembershipKind,
    semantic_identity_digest: String,
    descriptor_content_digest: String,
}

pub(crate) struct AdoptedDescriptorBinding {
    analysis: DescriptorIdentityDigest,
    destination_artifact: String,
    destination_descriptor_digest: String,
    destination_object_uuid: PlatformUuid,
    destination_extended_object_uuid: PlatformUuid,
}

pub(crate) enum CfeModuleMaterialState {
    AbsentWatched,
    Present { byte_length: u64, content_digest: String },
}

pub(crate) struct CfeParentChainPlan {
    components: Vec<CfeParentComponentPlan>,
    creation_scope_start_index: u8,
    first_absent_index: Option<u8>,
}

pub(crate) struct CfeParentComponentPlan {
    path: String,
    state: CfeParentComponentState,
}

pub(crate) enum CfeParentComponentState {
    PresentDirectory,
    AbsentDirectory,
}

pub(crate) struct CfeAllowedEffects {
    file_artifact: String,
    parent_directory_creation_scope: Vec<String>,
}

pub(crate) enum CfeAppendBoundary {
    Absent,
    PresentEmpty,
    PresentEndsLf,
    PresentOther,
}

pub(crate) struct CfeMethodPatchWritePlan {
    parent_chain: CfeParentChainPlan,
    directories_to_create: Vec<String>,
    expected_module: CfeModuleMaterialState,
    append_boundary: CfeAppendBoundary,
    rendered_patch: Vec<u8>,
    rendered_patch_digest: String,
    expected_after_length: u64,
    expected_after_digest: String,
}
```

`AdoptedDescriptorBinding` records exact source/destination ArtifactRefs,
descriptor artifacts/digests, a successfully parsed destination
`CfeMembershipKind::Adopted`, source object UUID and matching destination
ExtendedConfigurationObject UUID. Its constructor accepts no Own payload and no
partially parsed membership. Its `analysis` descriptor is accepted only when
its membership is exactly Own and its catalog flavor is BaseConfiguration; the
destination descriptor is accepted only under ExtensionConfiguration. Root is
always present; Form adds a second binding in root-then-form order. Declared
source kinds are checked assertions, not substitutes for either material fact.

`CfeSourceMethodPlan` retains every parser-authoritative range. Constructor
validation over the one exact verified module byte slice requires:

```text
definition.start == declaration.start
declaration contains name_span and parameter_list_span
name_span.end <= parameter_list_span.start
declaration.end + encoded(declaration_line_ending).len == body.start
body.end == terminator.start
terminator.end == definition.end
all ranges are ordered, in-bounds and slice the parser's exact tokens
```

No constructor accepts detached declaration/parameter/body bytes. Exact digest
algorithms are:

```text
definitionDigest = SHA256(
  "unica.cfe-source-definition.v1\0" || u64be(definitionBytes.len) || definitionBytes)
bodyDigest = SHA256(
  "unica.cfe-source-body.v1\0" || u64be(bodyBytes.len) || bodyBytes)
signatureDigest = SHA256(
  "unica.cfe-source-signature.v1\0" || methodKindTag || contextTag ||
  asyncByte || exportByte || lineEndingTag ||
  u64be(declarationBytes.len) || declarationBytes ||
  u64be(parameterListBytes.len) || parameterListBytes)
```

`lineEndingTag` is Lf=1, Crlf=2. `definitionBytes`, `bodyBytes`,
`declarationBytes` and `parameterListBytes` are sliced from the same
digest-verified module buffer using the retained ranges. The immutable plan and
execution encoder retain all six ranges plus line ending; renderer splices only
through those ranges.

`CfeParentChainPlan` lists every directory component below the exact selected
destination source root through the target's immediate parent, ancestor first,
with at most `MAX_CFE_PARENT_COMPONENTS=8`. Present components must form a
prefix; absent components, if any, form the complete remaining suffix.
`creation_scope_start_index` is topology-derived and does not depend on current
existence: Common/Owner creation scope starts at their `<Name>` directory; Form
scope starts at its `<Form>` directory because the registered `<Form>.xml`
descriptor already proves the preceding `<Owner>/<Name>/Forms` anchor. Every
component before that index must be Present.

`CfeAllowedEffects.parent_directory_creation_scope` is the complete stable path
vector from `creation_scope_start_index` through the target parent and enters
immutable grant scope whether those directories are currently present or
absent. `write.directories_to_create` is the current exact absent suffix and
enters only execution plan/baseline. It must be a suffix of the stable creation
scope. Thus grant B remains valid after grant A creates shared parents, while B
resolves an empty current create list. A file, link/reparse entry, case alias,
missing component before the creation scope, or Present component after the
first Absent cannot construct a plan.

`DescriptorIdentityDigest.semantic_identity_digest` encodes only the stable
tuple `(ArtifactRef, descriptor artifact, PlatformUuid, Own membership tag)`
under an explicit v1 domain. It excludes descriptor bytes. Analysis and
destination configuration flavor tags are separate immutable grant fields.
`descriptor_content_digest` and
`destination_descriptor_digest` bind the current captured XML and participate
only in the execution-plan digest/current baseline. The grant encoder consumes
the semantic identity plus the Adopted/extended-UUID tuple, never the rolling
content digest.

Task 8 has no event/item/Action inspection outcome. A successful Form plan
requires two independent complete Task 5B lookups under one accepted registry
version: analysis exact `MethodName` is `Unbound`, and destination canonical
generated method identity is `Unbound`.
`OrdinaryAndDestinationNameUnbound` stores the registry version and separate semantic
digests and exact material identities for both. Each semantic digest encodes
form owner identity, queried method identity, negative result, completeness
state and registry version; unrelated binding ordering/content does not enter
grant scope, while both complete Form.xml content identities enter execution/
baseline. For a non-Form target, the fixed NotForm semantic digest is the only
row. An absent/incomplete destination Form.xml never becomes an empty binding
set.

### 4.6 Final immutable plan

```rust
pub(crate) struct CfeArtifactMutationScope {
    canonical_artifact_locus: CanonicalArtifactLocusV1,
}

pub(crate) struct CfeMethodPatchPlan {
    version: u16, // 1
    core: ResolvedCfeMethodPatchCore,
    canonical_target: ArtifactRef,
    mutation_class: CfeMutationClass, // MethodPatch
    analysis: ResolvedSourceSet,
    destination: ResolvedSourceSet,
    source_method: CfeSourceMethodPlan,
    adopted_destination: CfeAdoptedDestinationPlan,
    artifact_mutation_scope: CfeArtifactMutationScope,
    allowed_effects: CfeAllowedEffects,
    snapshot_watch: SnapshotWatch,
    renderer_version: &'static str,
    write: CfeMethodPatchWritePlan,
    normalized_arguments_digest: String,
    grant_scope_digest: String,
    execution_plan_digest: String,
}

pub(crate) enum ResolvedMutationPlan {
    CfeMethodPatch(CfeMethodPatchPlan),
}
```

All fields are private. Constructors validate ordering, bounds, source/dest
identity, UUID chain, derived assertions, paths, rendered bytes/digests and
stable encodings. No receipt stores `rendered_patch` or source text; it stores
the atomic grant fields and digests defined later. The plan stores only semantic
`artifact_mutation_scope`. Applied infrastructure combines its canonical locus
with the handle-derived physical destination-root identity to create the opaque
lease key. Physical identity and lock filename are current-call control-plane
facts and never enter plan/grant/receipt encodings, preserving direct/discovery
value equality while still making cooperating aliases share one lock.

---

## 5. Direct and discovery input contract

### 5.1 Dedicated direct-tool keys

`unica.cfe.patch_method` accepts only:

```text
cwd, dryRun, confirm
SourceSet | sourceSet
ExtensionPath | extensionPath
ModulePath | modulePath
MethodName | methodName
InterceptorType | interceptorType
Context | context
IsFunction | isFunction
```

The four required semantic groups are ExtensionPath, ModulePath, MethodName and
InterceptorType for dry-run and apply. SourceSet, Context and IsFunction are
optional. The tool no longer inherits unrelated generic XML/DSL arguments.

Alias-aware schema uses `allOf/anyOf` for the four required groups. Runtime
merge remains authoritative because JSON Schema cannot express equality of two
aliases. Unknown keys, wrong types and null fail.

### 5.2 Alias merge

Parse each alias independently, then merge semantically:

- one present -> value/assertion;
- both absent required -> `cfe_missing_argument`;
- both absent optional -> `None`, never a synthetic default;
- both present and semantically equal -> one normalized value;
- both present conflict, or one invalid -> fail; no first-key-wins.

Semantic equality:

- SourceSet: same exact resolved analysis source identity;
- ExtensionPath: same exact mapped destination root;
- ModulePath: same normalized module identity;
- MethodName: byte-exact case-preserving identifier;
- Interceptor/Context/method-kind assertion: same closed value.

All 2^7 single-alias spelling permutations with all optional fields present
produce one digest over the same snapshot. Omitted Context/IsFunction and
explicit matching values also produce that digest after source resolution.

### 5.3 Exact source selection

Selection order:

1. a discovery request or future receipt grant supplies authoritative analysis;
   if direct raw SourceSet is also present, resolve it byte-exact and compare it
   to that authority;
2. without authority, an explicit raw SourceSet resolves byte-exact and becomes
   the selected analysis assertion;
3. an authority/raw-selected source must be the exact pair Configuration +
   PlatformXml; anything else, including Extension + PlatformXml or
   Configuration + EDT, fails `cfe_analysis_configuration_required` before
   watch/material reads;
4. only when neither authority nor raw selector exists, first filter configured
   analysis candidates to exact Configuration + PlatformXml and then require
   exactly one: zero -> `cfe_analysis_source_not_found`, multiple ->
   `cfe_analysis_source_required`; adjacent Extension sources do not create an
   ambiguity and one Configuration plus any number of Extensions selects that
   Configuration;
5. analysis and destination must be distinct exact source identities.

Case-only source names do not match. Raw SourceSet omitted and explicit exact
match normalize identically. A future grant cannot be redirected by raw args.
Only the selected Configuration descriptors provide analysis `MetadataIdentity`;
the resolver never reads or compares an extension wrapper UUID as base identity.

### 5.4 Configuration mismatch is a mutation preflight, not source readiness

General discovery may analyze a supported Platform XML Extension. Therefore
`cfe_analysis_configuration_required` must not be projected as
`source_readiness` or make the entire discovery source unreadable. It affects
only CFE mutation proposals and uses this exact closed wire tuple:

```json
{
  "code": "mutation_preflight",
  "provider": "DiscoveryPreflight",
  "state": "skipped",
  "outcome": "inconclusive",
  "coverage": "unknown",
  "severity": "blocking",
  "affects": ["proposal:<canonical-id>"],
  "reasonCode": "cfe_analysis_configuration_required",
  "retryable": false,
  "details": [],
  "evidenceIds": []
}
```

`affects` is nonempty, proposal-only, sorted/deduplicated and chunked by the
existing 128-entry bound. `Check::validate` accepts `DiscoveryPreflight` only
with `code=mutation_preflight` and this exact tuple; no evidence port may emit
it. The affected proposal verdict is Unknown with the same blocker, remains in
the report, is receipt-ineligible and never reaches issuer. A direct tool call
returns the same leading reason without manufacturing a report check.

### 5.5 Discovery intent remains strict

Mutation intent keeps strict PascalCase-only arguments and no SourceSet key:

```json
{
  "tool": "unica.cfe.patch_method",
  "destinationSourceSet": "AcceptanceExtension",
  "arguments": {
    "ExtensionPath": "src-cfe",
    "ModulePath": "Documents.Order.ObjectModule",
    "MethodName": "BeforeWrite",
    "InterceptorType": "Before"
  }
}
```

Context/IsFunction may be present as assertions. The discovery request's exact
resolved `sourceSet` supplies analysis authority. LowerCamel/common transport
keys remain forbidden inside intent.

---

## 6. Prepare-stage source and destination mapping

### 6.1 Exact ExtensionPath mapping

Prepare accepts only a lexical alias already attached to one configured
Extension + PlatformXml topology entry:

1. the entry's exact configured source-root alias; or
2. its exact configured `<root>/Configuration.xml` alias.

Prepare does not `stat`, canonicalize, open or otherwise prove either alias from
the live filesystem. It rejects a sibling/parent/nested/other-file spelling,
Configuration kind, EDT, Unknown/Invalid, lexical traversal,
separators/control, case-only alias, no match or multiple match using configured
topology alone. Capture is the first phase allowed to prove that the mapped root
and Configuration.xml are regular, workspace-contained and free of
symlink/reparse/case-alias traversal. Discovery also compares the exact mapped
identity with `destinationSourceSet`.

### 6.2 Exact source/destination artifacts

One module identity derives both analysis and destination paths under their
respective roots:

| Identity | Exact suffix |
| --- | --- |
| Common | `CommonModules/<Name>/Ext/Module.bsl` |
| Owner | `<OwnerDir>/<Name>/Ext/<ModuleKind>.bsl` |
| Form | `<OwnerDir>/<Name>/Forms/<Form>/Ext/Form/Module.bsl` |

Owner descriptor suffix is `<OwnerDir>/<Name>.xml`; Form also requires its
descriptor `<OwnerDir>/<Name>/Forms/<Form>.xml`. Source mechanism classification
reads the separately registered managed-form material
`<OwnerDir>/<Name>/Forms/<Form>/Ext/Form.xml` from the analysis snapshot. A Form
target also reads that exact suffix from the destination snapshot to prove the
generated handler name is not already an XML-bound event/action procedure.
Exact catalog registrations are required in both analysis and destination. The
analysis module and both Form.xml binding materials must be ordinary Present;
only the destination BSL module may be watched absent.

Analysis artifacts are derived only under the exact selected Configuration
source root. Destination artifacts are derived only under the exact Extension
source root. Reusing the symmetric suffix table does not make the source kinds
interchangeable and does not authorize wrapper UUID comparison.

### 6.3 Expected binding at prepare time

```rust
pub(crate) struct ExpectedMutationBinding<'a> {
    pub(crate) proposal_target: Option<&'a ArtifactRef>,
    pub(crate) authoritative_analysis: Option<&'a ResolvedSourceSet>,
    pub(crate) authoritative_destination: Option<&'a ResolvedSourceSet>,
    // Task 10 adds one atomic expected receipt grant here.
}

pub(crate) struct NormalizedExpectedBinding {
    proposal_target: Option<ArtifactRef>,
    authoritative_analysis: Option<ResolvedSourceSet>,
    authoritative_destination: Option<ResolvedSourceSet>,
}
```

Tool, canonical target, analysis/destination identities, ModulePath and exact
MethodName compare as one tuple. Context/kind assertions are finalized only
after source parsing. `prepare` only normalizes the expected topology tuple,
derives descriptor/module/form candidate paths and stores the owned form. It
does not open or validate any borrowed descriptor, Configuration.xml, Form.xml
or BSL material. Those references become authoritative only when capture places
them in the manifest and `resolve` consumes their typed projection. The seed
carries no proposal ID or raw display value. No field-wise cross-product
acceptance.

---

## 7. Common SnapshotWatch extension to Task 4

### 7.1 Exact domain contract and bounds

```rust
pub(crate) const MAX_SNAPSHOT_WATCHES: usize = 32;
pub(crate) const MAX_WATCH_PREREQUISITES: usize = 2;
pub(crate) const MAX_CFE_PARENT_COMPONENTS: usize = 8;
pub(crate) const MAX_WATCH_PATH_BYTES: usize = 4096;

pub(crate) enum SnapshotWatchPurpose {
    CfeMethodPatchModuleV1,
}

pub(crate) struct SnapshotWatch {
    source_set: String,
    purpose: SnapshotWatchPurpose,
    artifact: String,
    required_registered_paths: Vec<String>,
    parent_components: Vec<String>,
}

pub(crate) enum SnapshotWatchUnresolvedReason {
    RequiredRegisteredPathMissing { path: String },
    ParentComponentUnsafe { path: String },
    ParentComponentCaseAlias { path: String },
}

pub(crate) enum SnapshotWatchOutcome {
    Resolved { watch: SnapshotWatch, state: WatchedMutationTargetState },
    Unresolved { watch: SnapshotWatch, reason: SnapshotWatchUnresolvedReason },
}

pub(crate) struct WatchedMutationTargetState {
    parent_chain: Vec<WatchedParentComponent>,
    artifact_state: WatchedMaterialState,
}

pub(crate) struct WatchedParentComponent {
    path: String,
    state: WatchedParentComponentState,
}

pub(crate) enum WatchedParentComponentState {
    PresentDirectory,
    AbsentDirectory,
}

pub(crate) struct WatchedSourceSnapshot {
    snapshot: SourceSnapshot,
    outcomes: Vec<SnapshotWatchOutcome>,
}
```

Validation:

- 0..=32 unique semantic watches after sort/dedup;
- source-set uses existing 1..=1024 stable bytes;
- every path is contained workspace-relative slash form, 1..=4096 bytes;
- artifact belongs to exact selected source root;
- prerequisites are sorted unique and contain exact root descriptor plus Form
  descriptor when applicable; 1..=2;
- parent components contain every slash-prefix directory below the selected
  source root through the target parent, 1..=8, ancestor-first and unique; the
  last component is exactly `parent(artifact)`;
- purpose-specific artifact/prerequisites round-trip through the same module
  identity derivation, and parent components are derived by that same mapping;
- same `(sourceSet, artifact)` with conflicting purpose, prerequisites or parent
  component vector fails;
- canonical order is sourceSet bytes, artifact, purpose tag, prerequisite
  vector, then parent-component vector.

Watch is a semantic key and intentionally contains no proposal/request ID.
Discovery owns a bounded `proposal_id -> watch` join map and deduplicates watches
for capture. Two proposals may share one captured destination module.

### 7.2 Backward-compatible port change

Keep existing Task 4 `capture()` signature and behavior byte-identical. Add:

```rust
fn capture_with_watches(
    &self,
    analysis: &ResolvedSourceSet,
    mutations: &[ResolvedSourceSet],
    watches: &[SnapshotWatch],
    workspace_epoch: u64,
) -> Result<WatchedSourceSnapshot, SnapshotCaptureError>;
```

The trait default accepts only an empty list, delegates to existing `capture`,
and wraps zero outcomes. Non-empty default returns
`snapshot_watches_unsupported`. `FilesystemSourceSnapshots` overrides the
method. This preserves existing fake/Task 4 implementers and exact empty-watch
fingerprints while forcing Task 8 fakes to implement real watch behavior.

### 7.3 Atomic initial/final semantics

For each initial scan:

1. build ordinary registration-derived plans for all selected sources;
2. locate watch source and exact prerequisites without opening target artifact;
3. missing prerequisite -> typed Unresolved; physical decoy is not opened;
4. walk every declared parent component with no-follow/no-reparse lookup and
   exact directory-entry spelling; record PresentDirectory until the first
   missing component and semantic AbsentDirectory for that complete suffix;
5. a file, link/reparse entry, case alias or unexpected Present component behind
   the first absence -> typed Unresolved; never inspect through it;
6. present artifact already in ordinary plan -> reuse Present entry, which is
   valid only when every parent is PresentDirectory;
7. absent artifact with safe parent chain ->
   `AbsentWatched(CfeMethodPatchModuleV1)` manifest entry, including when an
   ancestor in the planned parent suffix is absent;
8. count watched directory entries/files/bytes/deadline under existing capture
   budgets.

Final scan repeats all decisions. Outcomes, every parent state, plan membership,
file identity, length and digest must match initial scan. Parent or target
appearance/disappearance, type/link/reparse swap, registration or case alias
change is `source_changed_during_capture` and returns no snapshot prefix.

### 7.4 Manifest and fingerprint encoding

Append without changing existing encodings:

```text
Manifest entry tag 1 = Present (existing)
Manifest entry tag 2 = AbsentOptional + existing purpose tags 1..5
Manifest entry tag 3 = AbsentWatched + purpose tag 1 CfeMethodPatchModuleV1
Manifest entry tag 4 = PresentWatchedDirectory + purpose tag 1
Manifest entry tag 5 = AbsentWatchedDirectory + purpose tag 1
```

Present watched material uses ordinary Present encoding. Absent watched uses
tag 3. Every watched parent path receives tag 4 or 5 and therefore participates
in the source/composite fingerprint; directory entries carry no byte length or
content digest. The manifest path binds the exact component/artifact. Empty-watch
source and composite fixture hashes stay byte-for-byte identical; watched
fixture hashes are fixed separately.

Canonical watch encoding inside the CFE execution-plan digest is:

```text
schema byte 1
sourceSet length+bytes
purpose tag 1
artifact length+bytes
prerequisite count u64
each sorted prerequisite length+bytes
parent component count u64
each ancestor-first parent path length+bytes
```

The execution-plan outcome immediately follows that key as parent-state count,
each `(path, Present=1|Absent=2)` pair, and artifact material state. No proposal
ID, cwd, absolute path, opaque filesystem identity or workspaceEpoch enters it.

### 7.5 Common capture errors and CFE mapping

Common stable capture reasons:

```text
snapshot_watch_limit
snapshot_watch_invalid
snapshot_watch_conflict
snapshot_watch_source_not_selected
snapshot_watches_unsupported
source_changed_during_capture
```

Existing path/resource/deadline errors remain unchanged. Unresolved is an
outcome, not a fatal capture error. Missing root/form descriptor prerequisites
are joined with the same Task 5A existence polarity: proven destination
descriptor absence is `destination_borrow_required`; failed/gapped/ambiguous
membership is `destination_membership_inconclusive` plus its exact reason;
analysis identity absence/incompleteness is
`analysis_metadata_identity_inconclusive`. Task 8 does not invent
owner/form-not-registered aliases that collapse the lattice. Unsafe/type/link
parent maps to `cfe_destination_parent_unsafe`; a spelling collision maps to
`cfe_destination_parent_case_alias`. Resolver never maps generic NotInManifest
to absence.

---

## 8. Two-stage shared resolver APIs with Applied topology refresh

### 8.1 Resolver types

```rust
pub(crate) enum MutationResolverKind { CfeMethodPatchV1 }

pub(crate) enum MutationResolutionArguments<'a> {
    Public(&'a Map<String, Value>),
    Intent(&'a CfePatchMethodArguments),
}

pub(crate) struct MutationPreparationRequest<'a> {
    pub(crate) kind: MutationResolverKind,
    pub(crate) arguments: MutationResolutionArguments<'a>,
    pub(crate) context: &'a WorkspaceContext,
    pub(crate) expected: ExpectedMutationBinding<'a>,
}

pub(crate) struct MutationFinalizationRequest<'a> {
    pub(crate) prepared: &'a PreparedMutationResolution,
    pub(crate) captured: &'a WatchedSourceSnapshot,
}

pub(crate) struct MutationPreparationRefreshRequest<'a> {
    pub(crate) initial: &'a PreparedMutationResolution,
    pub(crate) context: &'a WorkspaceContext,
    pub(crate) artifact_lease: &'a ArtifactMutationLeaseWitness,
}

pub(crate) trait MutationPlanResolverPort {
    fn prepare(
        &self,
        request: MutationPreparationRequest<'_>,
    ) -> Result<PreparedMutationResolution, MutationResolutionError>;

    fn refresh_under_artifact_lease(
        &self,
        request: MutationPreparationRefreshRequest<'_>,
    ) -> Result<PreparedMutationResolution, MutationResolutionError>;

    fn resolve(
        &self,
        request: MutationFinalizationRequest<'_>,
    ) -> Result<ResolvedMutationPlan, MutationResolutionError>;
}

pub(crate) enum MutationCallPurpose { Preview, Applied }

pub(crate) struct WorkspaceDiscoveryControlRootV1 {
    _private: (),
}

pub(crate) struct ArtifactMutationLeaseCandidate {
    destination_root_workspace_relative: String,
    canonical_artifact_locus: CanonicalArtifactLocusV1,
}

pub(crate) struct PhysicalDestinationRootIdentityDigest(String);
pub(crate) struct PhysicalControlRootIdentityDigest(String);
pub(crate) struct PhysicalLockObjectIdentityDigest(String);
pub(crate) struct CanonicalArtifactLocusDigest(String);

pub(crate) struct ArtifactMutationSemanticKey {
    physical_destination_root_identity_digest: PhysicalDestinationRootIdentityDigest,
    canonical_artifact_locus_digest: CanonicalArtifactLocusDigest,
}

// Private constructor requires an already-qualified backend. V1 is deliberately
// root-wide; CanonicalArtifactLocus does not enter this key.
pub(crate) struct RootWideCollisionProtocolV1;

pub(crate) struct FilesystemArtifactCollisionKey {
    physical_destination_root_identity_digest: PhysicalDestinationRootIdentityDigest,
    collision_protocol_version: RootWideCollisionProtocolV1,
}

pub(crate) struct ArtifactMutationLeaseUniverseKey {
    physical_control_root_identity_digest: PhysicalControlRootIdentityDigest,
    filesystem_collision_key: FilesystemArtifactCollisionKey,
}

pub(crate) trait ArtifactMutationLeasePort {
    fn try_acquire(
        &self,
        context: &WorkspaceContext,
        candidate: &ArtifactMutationLeaseCandidate,
    ) -> Result<Box<dyn ArtifactMutationLeaseGuard>, ArtifactMutationLeaseAcquireFailure>;
}

pub(crate) enum ArtifactMutationLeaseError {
    Busy,
    Unavailable,
    MountBoundaryUnsupported,
    AtomicBackendUnsupported,
}

pub(crate) struct ArtifactMutationLeaseAcquireFailure {
    error: ArtifactMutationLeaseError,
    control_plane_observation: ControlPlaneLeaseObservationV1,
}

pub(crate) trait ArtifactMutationLeaseGuard {
    fn witness(&self) -> &ArtifactMutationLeaseWitness;
}

pub(crate) struct ArtifactMutationLeaseWitness {
    semantic_key: ArtifactMutationSemanticKey,
    filesystem_collision_key: FilesystemArtifactCollisionKey,
    universe_key: ArtifactMutationLeaseUniverseKey,
    physical_lock_object_identity_digest: PhysicalLockObjectIdentityDigest,
    backend_contract: AtomicMutationBackendContract,
    canonical_artifact_locus: CanonicalArtifactLocusV1,
    control_plane_observation: ControlPlaneLeaseObservationV1,
    retained_scope: RetainedArtifactMutationScopeCapability,
}

pub(crate) struct ControlPlaneLeaseObservationV1 {
    created_directory_count: u8, // private ctor: 0..=8
    lock_file_created: bool,
}

pub(crate) struct RetainedArtifactMutationScopeCapability {
    control_root: WorkspaceDiscoveryControlRootV1,
    // cfg(unix/windows) private workspace/control/destination handles
    _private: (),
}

pub(crate) struct ResolvedMutationCall {
    plan: ResolvedMutationPlan,
    artifact_lease: Option<Box<dyn ArtifactMutationLeaseGuard>>,
}
```

The same concrete CFE resolver implements all three methods. Initial `prepare`
parses arguments once, resolves configured topology and derives candidate
registration/parent paths plus `ArtifactMutationLeaseCandidate` without reading
source material or calling either snapshot/material port. Preview/discovery use
that seed directly. Applied flow first acquires the physical artifact lease,
then `refresh_under_artifact_lease` re-reads current configured topology from the
already-typed request, reconstructs source identities/watch and accepts the
refreshed seed only when destination root resolves to the witness's retained
physical root, same mount/volume universe, same qualified backend tuple and
`canonical_artifact_locus` is value-equal. An unrelated
mapping edit may change the refreshed map digest without changing the key. A
different root/locus fails `cfe_artifact_scope_changed`; a changed mount/backend
fails its exact unsupported reason, all with zero material
capture/write; the implementation drops the old lease and requires a new call,
never switches lock keys in-place. Capture validates the refreshed paths against
the shared catalog. `resolve` never accepts raw arguments and cannot select a
different source/watch/path.

`WorkspaceDiscoveryControlRootV1` and `ArtifactMutationLeaseWitness` have no
public constructors and are neither Clone, Serialize nor Debug-with-paths. The
semantic-key and filesystem-collision-key digest newtypes also have no string
constructor: infrastructure constructs them only from the exact algorithms
below and validates 32 digest bytes before lowercase-hex filename encoding. The
witness owns private retained workspace, control-root and destination-root
directory capabilities for the guard lifetime; the native writer consumes that
same borrowed capability rather than reopening an absolute path. Infrastructure
may resolve one trusted absolute `cwd` only to open the workspace root. From
that handle it opens/creates exactly one fixed relative control root:

```text
.build/unica/project-discovery/control-v1
```

Every component is opened or created one-at-a-time relative to the retained
workspace handle, with exact spelling, restrictive permissions and link/reparse
rejection. There is deliberately no path-derived `<workspaceKey>` below it: two
path/case aliases or bind views of the same **whole physical workspace** reach
the same `.build` objects. `WorkspaceContext.cache_root`, `UNICA_CACHE_DIR`, lexical cwd bytes,
server process ID and caller cache configuration are not identity inputs. A
failure to prove this one root is `cfe_artifact_lock_failed`; there is no fallback
to a process cache.

Before authoritative capture and lock lookup, infrastructure walks
`candidate.destination_root_workspace_relative` component-by-component from the
retained workspace handle with no-follow/no-reparse and exact entry checks. It
also opens every fixed control-root component. One platform mount proof must be
value-equal for the workspace root, every control component and every
destination-root component **within that call**:

- Linux: `statx(fd, "", AT_EMPTY_PATH | AT_STATX_DONT_SYNC, STATX_MNT_ID, ...)`
  must return `STATX_MNT_ID`; every returned `stx_mnt_id` equals the retained
  workspace-root anchor. Missing statx support/mask or an internal bind/mount
  transition is unsupported. The anchor may differ between two bind views of
  the same whole workspace; mount IDs are never cross-process identity or key
  bytes;
- macOS: each retained handle's `fstat` `st_dev` and `fstatfs`
  (`f_fsid`, NUL-terminated `f_mntonname`) mount-instance tuple must equal the
  workspace-root anchor, `f_fstypename` must be exact allowlisted local `apfs`,
  `MNT_LOCAL` must be set and `MNT_RDONLY` clear. `fpathconf(fd, _PC_CASE_SENSITIVE)` on every
  retained workspace/control/destination directory must return one unchanged
  exact value: `1` selects the case-sensitive APFS row and `0` selects the
  case-insensitive row. `-1`, another value, errno ambiguity or changing
  material is unsupported. This value does
  not narrow the root-wide collision key. The mountpoint bytes are per-call
  proof only and never key/digest input. A nested volume/mount,
  nullfs/FUSE/unknown stack or unavailable/truncated field is unsupported;
- Windows: every handle has the workspace anchor's `VolumeSerialNumber`; every
  traversed component is opened with reparse-point semantics and rejected if it
  is a reparse/mounted-volume boundary. `FileIdInfo` remains object identity.

Thus W1 and W2 that internally bind the same destination D are both rejected
with `cfe_destination_mount_boundary_unsupported` before capture/mutation.
Whole-workspace aliases are accepted only if later lock-object identity and
Busy tests prove they reach one inode/FileId. Infrastructure then
derives one opaque `physical_destination_root_identity_digest` from handle
facts:

```text
UnixPhysicalRootV1 =
  SHA256("unica.physical-directory.unix.v1\0" ||
         u64be(st_dev) || u64be(st_ino))

WindowsPhysicalRootV1 =
  SHA256("unica.physical-directory.windows.v1\0" ||
         u64be(volume_serial_number) || file_id_128)
```

The opened control-root and lock object use the same field encodings but distinct
domains `unica.physical-control-root.{unix|windows}.v1\0` and
`unica.physical-lock-object.{unix|windows}.v1\0` to yield
`PhysicalControlRootIdentityDigest` and `PhysicalLockObjectIdentityDigest`.
Unix fields are `u64be(st_dev)||u64be(st_ino)`; Windows fields are
`u64be(VolumeSerialNumber)||FileId128`. On Windows physical directory/file values
come from VolumeSerial+FileId. Zero/truncated/unavailable identities fail closed.
Raw identities and per-call mount IDs are never logged or placed in receipts.

Infrastructure creates the witness only after a process-local non-blocking
guard and the exact allowlisted OS lock primitive succeed on the opened
persistent lock object. The process-local registry is keyed by
`ArtifactMutationLeaseUniverseKey` (physical control root + filesystem collision
key), never by semantic-key bytes, path string or source-set identity. Drop
releases both locks but never deletes the lock file. The semantic key preserves
the §4.2 canonical-locus digest for plan/scope equality, but it is **not** the
lock filename authority: APFS can treat byte-distinct NFC/NFD names as aliases,
case behavior is volume-qualified, and an Absent target has no target FileId to
key by.

V1 therefore chooses the conservative collision protocol
`RootWideCollisionProtocolV1`: after the backend and physical destination root
are proven, every semantic locus below that one physical destination root maps
to the same filesystem collision digest:

```text
filesystemArtifactCollisionKey =
  SHA256("unica.filesystem-artifact-collision.root-wide.v1\0" ||
         u64be(physicalRootIdentityDigest.len) || physicalRootIdentityDigest)
```

`physicalRootIdentityDigest` here is the raw 32-byte digest (`len=32`), never
its 64-byte hexadecimal presentation.
The private constructor requires a qualified backend but the backend tuple and
canonical locus are intentionally absent from these bytes, so two supported
binary/backend rows cannot split one v1 root. A future narrower collision key is
a lock-protocol migration/ADR with mixed-version dual-lock proof, never an
in-place digest change. The exact lock path is
`<control-root>/locks/artifacts/<filesystemCollisionKey>.lock`, where the final
component is lowercase hex of that digest. This intentionally serializes
unrelated artifacts in one destination source root; reduced concurrency is the
v1 price of a sound Present/Absent alias boundary.

The lease-specific semantic/collision key wrappers, control-root identity,
lock-object identity and lock key are control-plane-only and excluded from
plan/grant/receipt encodings; the canonical locus itself remains independently
present in the plan/grant tuple. This does not erase schema-v2 mutation-effect
object identity digests required for detached/staging authority. Raw
workspace/artifact text never becomes a filename. Directories and the lock
inode are created with restrictive permissions, link/reparse rejection and
create-new/open-existing retry; the inode is persistent. Task 9 receipt locks
use sibling `locks/receipts/`, never the artifact namespace, and its receipt
records use `<control-root>/receipts/`. Different `UNICA_CACHE_DIR`, unrelated
source-map edits and source-set aliases inside one supported physical workspace
cannot split the lock. Path/case/Unicode/bind aliases of the same whole physical
workspace must prove the same physical control root, same root-wide collision
key, same opened lock inode/FileId and one contender Busy. Semantic keys may be
different for NFC/NFD or case-distinct spellings and therefore are neither
necessary nor sufficient serialization proof. Equal collision-key bytes also
remain insufficient without actual common lock-object identity and Busy.

`ControlPlaneLeaseObservationV1` is the only channel for first-call control
initialization. It is a bounded internal count/boolean with no source paths,
physical IDs or grant effect tags; it is emitted separately from
`MutationHandlerOutcome` and cannot be converted to `MutationEffectsV2`.
Reusing an existing control root records zero/false. Dry-run creates neither the
objects nor this observation.

### 8.2 Direct flow

The application-facing direct resolution service is one call:

```rust
fn resolve_mutation_call(
    &self,
    spec: ToolSpec,
    args: &Map<String, Value>,
    context: &WorkspaceContext,
    purpose: MutationCallPurpose,
) -> Result<Option<ResolvedMutationCall>, String>;
```

For CFE Preview, its default implementation performs exactly one prepare, one
capture-with-watch over selected analysis+destination and one resolve, returning
no lease. For CFE Applied it performs one raw parse/initial prepare, opens the
workspace once and walks the destination root from that handle, derives/acquires
one physical artifact lease,
performs one topology refresh under that lease, and then exactly one
authoritative capture+resolve from the refreshed seed. Other descriptors return
None. A failed acquire returns `cfe_artifact_busy` or
`cfe_artifact_lock_failed`; an internal mount/control boundary returns its exact
unsupported reason, an unqualified backend returns
`cfe_patch_atomic_backend_unsupported`, and a changed refreshed root/locus
returns `cfe_artifact_scope_changed`, all before material capture or write.
Recording fakes assert stage counts and stable universe keys across unrelated
mapping/source aliases; native tests prove actual control/lock-object identity
and Busy across whole-workspace aliases. Applied returns one live retained
capability. This method is opaque to
`call_tool`; raw args are not passed to the refresh or resolve stage.

### 8.3 Discovery batch flow

1. parse all strict mutation intents to prepared seeds using request analysis;
2. resolve exact destinations and derive proposal->watch join rows;
3. sort/deduplicate at most 32 semantic watches;
4. capture analysis + all destinations + watches once;
5. run Task 7 providers/report evaluation;
6. resolve each prepared seed against its exact watch/snapshot;
7. validate pairwise compatibility for plans sharing one allowed artifact:
   generated definition names must differ under parser case identity, duplicate
   interceptor rules from §10.5 must allow the pair, and adding either plan's
   one annotation/definition fact to the captured syntax must leave the other
   plan's duplicate preflight successful;
8. pass sorted proposal+plan rows to issuer only if every selected eligible
   proposal resolves and the batch is compatible; otherwise no partial grant
   vector.

Invalid/adoption/source/form/watch blockers remain proposal-scoped and retain
the report. Mapping/snapshot safety failure remains the existing typed request
failure with zero accepted evidence prefix. A pairwise collision is
`cfe_batch_plan_conflict` on both proposals (bounded IDs in diagnostics only,
never in a digest) and cannot be issued as two grants that are known to make
one another unusable.

### 8.4 Fixed applied lock/lifetime order

The exact future-compatible order is:

```text
validate raw/workspace/path -> support guard
prepare (zero material reads)
open workspace once -> same-mount/volume + qualified backend proof
physical control/destination identities -> semantic key + root-wide collision universe
actual common collision-lock inode/lease
refresh current topology under lease; require same physical root + locus
authoritative watched capture of refreshed seed -> resolve current immutable plan
discovery guard -> optional current receipt lease + revision/baseline reread
final same-watch precondition recapture -> handler -> typed effects/staging/durability
post snapshot -> receipt advance/revoke
drop current receipt lease
drop artifact mutation lease
events -> cache work -> other-receipt reconciliation -> result
```

Task 8 implements the artifact/open/refresh/capture/resolve/final-check/handler/
typed-effect/post-snapshot path and an explicit recording seam at the receipt
positions. The receipt-lease, transition, reconciliation and production guard
lines are mandatory Task 9/10 back-propagated order, not Task 8 production code
or a Task 8 integration-test claim.

Receipt validation after acquisition must compare its reread baseline with the
already captured plan; a concurrent baseline advance produces a mismatch and
zero handler calls. Every applied CFE call follows this order, including calls
allowed without a receipt. No path may acquire an artifact lease while holding
a receipt lease, a second artifact lease while holding a receipt lease, or a
second receipt lease while either current lease is held. V1 plans exactly one
artifact; a future multi-artifact ADR must deduplicate and acquire all
filesystem collision keys in canonical byte order before any receipt lease.
Semantic keys do not determine acquisition order. Dry-run and discovery
issuance acquire neither lease.

---

## 9. Snapshot-only material projection

### 9.1 Application-owned port

```rust
pub(crate) trait CfeResolutionMaterialPort {
    fn inspect(
        &self,
        seed: &CfeMethodPatchSeed,
        captured: &WatchedSourceSnapshot,
    ) -> Result<CfeResolutionMaterial, MutationResolutionError>;
}

pub(crate) struct CfeResolutionMaterial {
    analysis_catalog: VerifiedParsedMaterial<PlatformConfigurationCatalogV1>,
    analysis_module: VerifiedBslMaterial,
    analysis_descriptors: Vec<VerifiedParsedMaterial<RegisteredDescriptorIdentity>>,
    analysis_form_bindings: Option<VerifiedParsedMaterial<CompleteFormMethodBindings>>,
    destination_catalog: VerifiedParsedMaterial<PlatformConfigurationCatalogV1>,
    destination_descriptors: Vec<VerifiedParsedMaterial<RegisteredDescriptorIdentity>>,
    destination_form_bindings: Option<VerifiedParsedMaterial<CompleteFormMethodBindings>>,
    destination_target: WatchedMutationTargetState,
    destination_module: OptionalVerifiedBslMaterial,
}

pub(crate) struct VerifiedParsedMaterial<T> {
    identity: VerifiedMaterialIdentity,
    value: T,
}

pub(crate) struct VerifiedBslMaterial {
    identity: VerifiedMaterialIdentity,
    bytes: Vec<u8>,
    syntax: BslSyntaxProjection,
}

pub(crate) enum OptionalVerifiedBslMaterial {
    AbsentWatched,
    Present(VerifiedBslMaterial),
}
```

Infrastructure implementation may read only manifest members via
`read_verified/read_optional_verified`. It uses the shared Task 5/6 pure
parsers and returns typed material plus exact artifact/length/content-digest
provenance. `BslSyntaxProjection` is the Task 6 result containing definitions,
annotation facts, scoped gaps, parser contract and the same content digest; it
is not a CFE parser. It does not accept absolute paths or raw arguments. Any
artifact/digest/source-fingerprint disagreement, a Form target without exactly
one verified analysis and one verified destination Form-binding material, or a
non-Form target with either material discards the whole projection. The port
also requires the captured catalog flavor to agree with declared source kind;
it never accepts the topology label as the parsed value.

### 9.2 Bounds

```rust
const MAX_CFE_CONFIGURATION_BYTES: u64 = MAX_SNAPSHOT_XML_BYTES; // 64 MiB
const MAX_CFE_DESCRIPTOR_BYTES: u64 = MAX_SNAPSHOT_XML_BYTES;    // shared cap
const MAX_CFE_MODULE_BYTES: u64 = MAX_BSL_FILE_BYTES;            // 16 MiB
const MAX_CFE_RENDERED_PATCH_BYTES: u64 = MAX_BSL_FILE_BYTES;    // clone may be large
const MAX_CFE_FINAL_MODULE_BYTES: u64 = MAX_BSL_FILE_BYTES;      // 16 MiB
```

Task 5 XML node/depth/count limits and Task 6 token/definition/deadline limits
also apply. Check manifest length before allocation; all sums/conversions are
checked. N/N+1 tests cover every local/shared limit.

### 9.3 Source method extraction

Requirements:

- analysis module is Present and strict UTF-8 with zero/one leading BOM;
- exactly one unconditional definition has exact byte spelling MethodName;
- no target-scoped parser gap, unsupported annotation/preprocessor, duplicate
  or incomplete form-binding result;
- all extraction spans validate against exact bytes and module digest;
- all six retained slices validate against the verified bytes; the
  definition/signature/body digests recompute exactly from the ranges and line
  ending specified in §4.5;
- method kind/context/async/export facts equal the parser projection;
- Context/IsFunction, if present, equal derived facts exactly;
- Function + Before/After is rejected;
- a Task 5B complete lookup `Bound` rejects as wrong mechanism; only `Unbound`
  under the accepted registry version can classify a Form target as Ordinary;
- for the same Form target, a second Complete destination binding proof must
  find no parser-canonical handler equal to the derived generated method name;
  an existing orphan XML binding is a collision even when the destination BSL
  definition is absent;
- any Task 5B registry/parser/resource gap or incomplete catalog fails closed as
  `cfe_form_binding_inconclusive`, never Ordinary; Task 8 does not reinterpret
  the underlying Form row.

Case-fold-only method match is not silently rewritten. Annotation target and
canonical ArtifactRef use the exact source spelling; a different raw spelling
fails `cfe_source_method_spelling_mismatch`.

### 9.4 Exact adopted chain

For every root/form pair in canonical order:

1. analysis is declared Configuration + PlatformXml, its captured catalog is
   exact `BaseConfiguration`, and its `MetadataPresent` and exact descriptor
   `MetadataIdentity` equal the requested owner;
2. every analysis descriptor that contributes that UUID is exactly `Own`;
3. destination is declared Extension + PlatformXml and its captured catalog is
   exact `ExtensionConfiguration`; existence polarity remains independent from
   membership;
4. a present destination descriptor must have one successful closed
   `CfeMembershipKind` projection;
5. only `Adopted { extended_configuration_object_uuid }` can construct an
   `AdoptedDescriptorBinding`;
6. that UUID equals the Own analysis descriptor object UUID;
7. descriptor paths/digests belong to their exact snapshots;
8. no duplicate/case-alias descriptor identity exists.

The exact Task 5A lattice and leading reason are reused, not translated into a
second CFE lattice:

| Analysis side | Destination side | Pair result | Leading blocker/reason | Guidance |
| --- | --- | --- | --- | --- |
| Present + exact identity | descriptor absent | RequiresBorrow | `destination_borrow_required` | `/cfe-borrow` may be proposed |
| Present + exact identity | present `Own` | Indeterminate | `destination_object_not_adopted` | inspect/resolve ownership; do not overwrite via borrow |
| Present + exact identity | present `Adopted` with equal UUID | AlreadyBorrowed | none | eligible pair |
| Present + exact identity | present `Adopted` with different UUID | Indeterminate | `destination_extended_object_mismatch` | inspect wrong-base binding; do not borrow over it |
| missing/failed/gapped identity | any | Indeterminate | `analysis_metadata_identity_inconclusive` + exact provider/gap reason | repair evidence/material |
| valid analysis | missing/failed/gapped membership | Indeterminate | `destination_membership_inconclusive` + exact provider/gap reason | repair evidence/material |

A declared/captured flavor mismatch is `cfe_configuration_flavor_mismatch`; an
Adopted analysis root/Form is `analysis_descriptor_not_base_owned`. Both are
Unknown/ineligible and precede UUID comparison. A wrapper/local UUID can never
become a candidate base UUID, even when its bytes equal the destination's
ExtendedConfigurationObject by construction.

Aggregate root + optional Form with Indeterminate precedence. All pairs
AlreadyBorrowed is ExtensionOwned. With no Indeterminate, at least one
RequiresBorrow and every remaining pair AlreadyBorrowed is ExtensionRequired;
this includes a single-pair target whose only descriptor is absent. Any
Indeterminate is Unknown even when another pair is absent. All non-success rows
are receipt-ineligible, but only the true RequiresBorrow row carries
`destination_borrow_required`. Malformed/unknown/duplicate fields never become
Missing or Own, and Task 8 never invokes borrow for them.

### 9.5 Configuration facts

Use `PlatformConfigurationCatalogV1` already produced from exact direct
Configuration/Properties nodes:

- destination ScriptVariant must be `Known(Russian|English)`;
- destination NamePrefix must be exactly one non-empty bounded value;
- generated method name is deterministic and has no user override:

| Interceptor | Russian destination | English destination |
| --- | --- | --- |
| Before | `NamePrefix + MethodName + "Перед"` | `NamePrefix + MethodName + "Before"` |
| After | `NamePrefix + MethodName + "После"` | `NamePrefix + MethodName + "After"` |
| ModificationAndControl | `NamePrefix + MethodName` | `NamePrefix + MethodName` |

- the complete generated name must pass `CfeIdentifier`; overflow or an
  invalid concatenation is `cfe_generated_method_name_invalid`;
- Missing/Duplicate/Empty/Unknown fails; no `Расш_` fallback;
- analysis catalog ScriptVariant must also be known and is recorded as source
  material, but destination variant controls newly rendered tokens.

No Task 8 code calls roxmltree over Configuration.xml independently.

---

## 10. Source-bound renderer and destination preflight

### 10.1 Renderer identity and bilingual semantic tokens

One pure renderer has exact identity:

```text
unica.cfe-method-patch-renderer.v1
```

Newly generated structural tokens use destination `KnownScriptVariant`:

| Semantic | Russian | English |
| --- | --- | --- |
| AtServer | `&НаСервере` | `&AtServer` |
| AtClient | `&НаКлиенте` | `&AtClient` |
| AtServerNoContext | `&НаСервереБезКонтекста` | `&AtServerNoContext` |
| AtClientAtServer | `&НаКлиентеНаСервере` | `&AtClientAtServer` |
| AtClientAtServerNoContext | `&НаКлиентеНаСервереБезКонтекста` | `&AtClientAtServerNoContext` |
| Before | `&Перед` | `&Before` |
| After | `&После` | `&After` |
| ModificationAndControl | `&ИзменениеИКонтроль` | `&ChangeAndValidate` |
| Procedure/end | `Процедура` / `КонецПроцедуры` | `Procedure` / `EndProcedure` |
| Async | `Асинх` | `Async` |
| Delete/end | `#Удаление` / `#КонецУдаления` | `#Delete` / `#EndDelete` |
| Insert/end | `#Вставка` / `#КонецВставки` | `#Insert` / `#EndInsert` |

`ModuleDefault` renders no context directive. Source Function never reaches
Before/After. ModificationAndControl clones the source declaration, so its
Procedure/Function/end/export/parameter spellings remain exact source bytes;
only added annotation/context/diff tokens use destination variant. Mixed
Russian/English BSL tokens are intentionally allowed by the platform grammar
and are bound by exact golden bytes.

### 10.2 Before/After exact signature renderer

For a parsed source Procedure:

1. render derived context line unless ModuleDefault;
2. render requested Before/After annotation with exact source method spelling;
3. render destination-variant optional Async + Procedure;
4. render the exact generated name from §9.5, including the mandatory
   destination-variant Before/After suffix;
5. copy exact verified parameter-list bytes, including parentheses, Val, names
   and defaults;
6. do not add Export to the generated hook;
7. render one variant-specific TODO comment and EndProcedure;
8. use CRLF for new lines and one trailing CRLF.

Russian comments preserve current valid donor output. English comments have
independent fixed golden text. A signature slice that does not round-trip to
the parsed `DefinitionShape` is `cfe_source_signature_inconclusive`.

### 10.3 ModificationAndControl exact clone

For Procedure or Function:

1. render derived context line unless ModuleDefault;
2. render destination-variant ChangeAndValidate annotation;
3. clone exact verified source `definition_span` bytes;
4. replace only parsed declaration `name_span` with generated name;
5. immediately after the declaration line insert a no-runtime-effect diff
   scaffold using that declaration's exact line ending:

```text
#Delete
#EndDelete
#Insert
    // TODO: extension change
#EndInsert
```

Russian uses Russian markers/comment, English uses English markers/comment.
The Delete region contains zero source bytes and the Insert region contains
only a comment; the original body remains byte-exact and ordered. The patch
ends with one CRLF after the cloned terminator.

This scaffold is not accepted on assumption alone. Fixtures for both variants
and Procedure/Function must be loaded/validated by the repository's real
available 1C compile path. If the platform rejects an empty Delete region,
STOP and replace it with the smallest platform-proven no-op marker shape;
never fall back to an empty method body.

The clone does not rewrite recursive calls, literals, source comments, source
line endings or source language keywords. All splice offsets come from
validated Task 6 spans; substring search is forbidden.

### 10.4 Destination module encoding and append boundary

Destination module accepts strict UTF-8 with zero or one leading BOM. Interior
or duplicate BOM and invalid UTF-8 fail. Final bytes always have exactly one
leading UTF-8 BOM. Existing payload bytes after BOM are preserved exactly.

| State | Separator before rendered patch |
| --- | --- |
| AbsentWatched | empty |
| PresentEmpty | empty |
| PresentEndsLf | `CRLF` |
| PresentOther | `CRLF CRLF` |

Resolver stores boundary tag, patch bytes/digest and exact final
length/digest. Handler reconstructs final bytes only from a digest-verified
current payload + planned boundary + planned patch, then verifies the computed
after digest before atomic replace. It never rerenders.

### 10.5 Complete duplicate preflight

Task 6 bilingual annotation/definition facts drive destination scan. The
requested enum remains three-value, but the observed enum includes official
`Around|Вместо`. For every well-formed annotation targeting the exact source
method, use this closed conflict matrix (`conflict` blocks the request):

| Existing observed kind | request Before | request After | request ModificationAndControl |
| --- | --- | --- | --- |
| Before | conflict | allowed | conflict |
| After | allowed | conflict | conflict |
| Around | conflict | conflict | conflict |
| ModificationAndControl | conflict | conflict | conflict |

Around is never generated by this tool, but it is an exclusive replacement and
therefore cannot be ignored during negative proof. A destination already
containing Around together with any other interceptor for the same target is
also material inconsistency and fails closed. Beyond the matrix, reject:

- any procedure/function with the generated handler name, regardless of kind;
- for a Form target, any complete destination Form.xml event or Action whose
  handler canonical identity equals the generated handler name, including an
  orphan binding with no current BSL definition;
- duplicate instances of an otherwise allowed observed kind;
- a possible target/name duplicate inside an unknown conditional branch;
- malformed/limited annotation or definition parse that prevents complete
  negative proof.

Before and After may coexist for a Procedure because their deterministic
suffixes make the generated handler names distinct; either still fails when
that exact name already exists. Batch simulation applies the same matrix after
inserting each proposed fact, in both orders. Comparisons use parser canonical
Unicode case identity. Comments, strings, dates and deleted blocks do not create
duplicates. Unknown unrelated annotations remain gaps only when they affect the
target/definition; no regex or substring scan.

The destination Form binding lookup uses the same complete Task 5 catalog and
parser-owned canonical identifier identity as source role classification. A
missing/incomplete/unsupported destination Form.xml is
`cfe_form_binding_inconclusive`; a proven generated-name binding is
`cfe_destination_form_handler_collision`. BSL silence alone is never negative
proof for a Form module.

### 10.6 Final material construction

Resolver computes with checked arithmetic:

- expected destination module state/length/digest;
- complete typed parent chain, stable topology-derived directory creation scope,
  first-absent index and current exact `directories_to_create` suffix;
- append boundary and rendered patch bytes/digest;
- final bytes length/digest, capped at 16 MiB;
- generated method identity and exact allowed file/directory effects;
- source/adoption/configuration material digests;
- normalized arguments, grant scope and execution plan digests.

Rendered clone may be large, so the old 8 KiB patch cap is invalid. It is
bounded by the 16 MiB BSL cap and stricter final-module cap. No truncation or
partial prefix is accepted.

---

## 11. Canonical digests and atomic grant separation

### 11.1 Encoding and digest roles

Use SHA-256 over explicit domain-separated binary encodings. Strings are
`u64 big-endian byte length || UTF-8 bytes`; byte blobs are length + SHA-256;
counts are u64; byte ranges are `u32be(start) || u32be(endExclusive)`; booleans
are 0/1; enums and line endings use stable tags. No JSON/debug/serde order,
platform separator, pointer address or enum discriminant participates.

Three digests have different jobs and MUST NOT be conflated:

```text
unica.cfe-method-patch.arguments.v1
unica.cfe-method-patch.grant-scope.v1
unica.cfe-method-patch.execution-plan.v1
```

### 11.2 Final normalized arguments digest

Computed only after source extraction, so omitted assertions and matching
explicit assertions normalize identically. The encoder accepts only
`ResolvedCfeMethodPatchCore`, never `PreparedCfeMethodPatchRequest`. Fields in
order:

1. schema byte 1 and tool tag;
2. resolved analysis identity: name/kind/format/relative root/mapping digest;
3. destination identity: name/kind/format/relative root/mapping digest;
4. canonical module identity and exact source MethodName;
5. interceptor tag;
6. derived BSL context tag;
7. derived method-kind tag;
8. derived async boolean.

Do not encode whether SourceSet/Context/IsFunction was omitted, alias spelling,
raw ExtensionPath or structural aliases. Conflict produces no digest.

### 11.3 Grant-scope digest

Receipt grant authority must survive a legitimate rolling baseline advance for
another grant. It binds immutable semantic scope, not current module bytes or
composite fingerprint. Fields in order:

1. schema byte 1 + normalized arguments digest;
2. canonical tool/target/mutation class/interceptor;
3. derived context/kind/async;
4. analysis and destination exact source identities/mapping digest;
5. analysis module, destination allowed file and ancestor-first stable
   parent-directory creation scope;
6. source definition/signature/body digests and parser contract;
7. exact eligible form-role, complete shared form-binding contract, analysis
   Ordinary semantic digest and destination generated-name-unbound semantic
   digest when applicable;
8. BaseConfiguration/ExtensionConfiguration flavor tags, exact Own analysis
   root/form identities and exact adopted destination UUID tuples;
9. analysis/destination KnownScriptVariant, destination NamePrefix and
   generated method identity;
10. resolver and renderer versions.

It excludes current source/composite fingerprints, destination module
present/absent/content, rendered bytes, expected-after digest, receipt/proposal
IDs and revision. Task 9 stores the whole atomic grant tuple plus this digest;
Task 10 compares that tuple atomically and compares current composite baseline
separately.

With two grants, applying A changes the destination fingerprint. If mutable
fingerprint were inside B's authority digest, rolling advance would make B
unusable without rewriting every remaining grant. Scope digest + separately
advanced baseline keeps two-step in-scope execution coherent.

If a prior mutation changes NamePrefix, ScriptVariant, adopted descriptor,
source definition or another immutable scope field, scope digest changes and
the old grant correctly stops matching even after baseline advance.

### 11.4 Execution-plan digest

Current-call digest includes every grant-scope field plus:

1. analysis source fingerprint and analysis catalog digest;
2. source module length/content digest; definition, declaration, name,
   parameter-list, body and terminator ranges in that exact order; declaration
   line-ending tag; definition/signature/body digests;
3. destination source fingerprint;
4. destination Configuration length/content digest;
5. analysis/destination descriptor artifact content digests;
6. analysis and destination Form-binding artifact/length/content digests when
   applicable, in that order;
7. canonical SnapshotWatch encoding, every parent state and artifact outcome;
8. creation-scope start index, first-absent index and current exact
   `directories_to_create` vector;
9. destination module material tag/length/digest;
10. append-boundary tag;
11. rendered patch length/digest;
12. expected final module length/digest.

It proves exact current preflight/rendered execution. It is used for same-call
identity, audit/diagnostics and precondition verification, not as the immutable
key of a remaining multi-grant receipt after another grant advances baseline.

### 11.5 Exclusions and digest tests

Excluded from all three: absolute workspace/cwd, dryRun, confirm, alias
spelling, receipt/proposal/analysis IDs, task/source text, timestamps,
workspaceEpoch and pointer identity.

Required tests:

- all seven alias-pair permutations and equal dual aliases;
- omitted versus explicit matching SourceSet/Context/IsFunction produce
  value-equal `ResolvedCfeMethodPatchCore` and full plans, not only equal
  digests; assertion diagnostics may differ but are outside the plan;
- every conflicting assertion fails before hashing;
- context/kind/async/signature/body changes expected digests;
- changing any retained range or declaration line ending changes the execution
  digest; a forged range/digest/slice combination cannot construct a plan;
- RU/EN, NamePrefix, configuration flavor, Own/adopted UUID or allowed effects
  changes scope+plan;
- destination Form generated-name binding changes scope+plan and blocks resolve;
  unrelated destination Form content changes execution/baseline only;
- only current destination module content changes execution plan, not grant
  scope, while immutable fields stay equal;
- provider/file/source ordering and cwd do not change digests;
- stable parent creation scope changes scope+execution; current parent states or
  `directories_to_create`, target present/absent, boundary/rendered/final change
  execution only;
- pure two-grant fixture supplies post-A baseline S1 without a store/revision,
  then B still matches immutable scope and receives a new execution plan over
  S1; persistent advance belongs to Tasks 9/10.

---

## 12. Atomic expected binding and one-object plumbing

### 12.1 Atomic compare

At issuance and future guard, compare one tuple:

```text
tool
canonical target
mutation class
interceptor/change kind
derived BSL context
derived method kind
derived async flag
analysis source identity
destination source identity
source module/definition/signature/body identity
BaseConfiguration + Own analysis root/form UUID chain
ExtensionConfiguration + adopted destination root/form UUID chain
analysis Ordinary + destination generated-name-unbound Form proof
exact destination allowed file + stable parent-directory creation scope
generated method identity
resolver/parser/renderer versions
normalized arguments digest
grant-scope digest
```

No per-field search across grants. One exact grant row must match the whole
tuple. Execution plan is checked against the separately validated current
receipt baseline and same-call material preconditions.

### 12.2 HandlerInvocation

```rust
pub(crate) struct HandlerInvocation<'a> {
    pub(crate) spec: ToolSpec,
    pub(crate) args: &'a Map<String, Value>,
    pub(crate) context: &'a WorkspaceContext,
    pub(crate) dry_run: bool,
    pub(crate) resolved_mutation: Option<&'a ResolvedMutationPlan>,
    pub(crate) artifact_lease: Option<&'a ArtifactMutationLeaseWitness>,
}

pub(crate) struct HandlerOutcome {
    pub(crate) adapter: AdapterOutcome, // presentation only
    pub(crate) job: Option<Value>,
    pub(crate) mutation: Option<MutationHandlerOutcome>,
}

pub(crate) const MUTATION_OUTCOME_SCHEMA_V2: &str = "unica.mutation-outcome.v2";
pub(crate) const MAX_MUTATION_EFFECTS: usize = 32;
pub(crate) const MAX_OWNED_STAGING: usize = 32;

pub(crate) enum MutationHandlerOutcome {
    NoChange {
        cleanup: VerifiedClean,
        source_tree: NoChangeSourceTreeProof,
    },
    Committed {
        effects: MutationEffectsV2,
        cleanup: MutationCleanupState,
        durability: MutationDurabilityState,
    },
    Uncertain {
        effects: MutationEffectsV2,
        cleanup: MutationCleanupState,
    },
}

pub(crate) struct MutationEffectsV2 {
    expected: Vec<TypedMutationEffect>,
    unexpected: Vec<TypedMutationEffect>,
    possible: Vec<TypedMutationEffect>,
}

pub(crate) struct PhysicalFilesystemObjectIdentity {
    volume_identity_digest: String,
    object_identity_digest: String,
}

pub(crate) enum TypedMutationEffect {
    CreatedDirectory {
        artifact: String,
        object: PhysicalFilesystemObjectIdentity,
    },
    CreatedFile {
        artifact: String,
        object: PhysicalFilesystemObjectIdentity,
        content_digest: String,
    },
    UpdatedFile {
        artifact: String,
        object: PhysicalFilesystemObjectIdentity,
        before_digest: String,
        after_digest: String,
    },
    OwnedStagingFile {
        artifact: String,
        object: PhysicalFilesystemObjectIdentity,
        retained_parent_identity_digest: String,
    },
    DetachedOrRelocatedFilesystemObject {
        intended_artifact: String,
        kind: DetachedFilesystemObjectKind,
        volume_identity_digest: String,
        object_identity_digest: String,
        retained_parent_identity_digest: String,
        content: DetachedContentState,
    },
}

pub(crate) enum DetachedFilesystemObjectKind {
    CreatedDirectory,
    CreatedFile,
    UpdatedFile,
    OwnedStagingFile,
}

pub(crate) enum DetachedContentState {
    NotApplicable,
    Known { content_digest: String },
    Unknown,
}

pub(crate) struct VerifiedClean {
    staging_lifecycle: Vec<VerifiedStagingLifecycle>,
}

pub(crate) struct OwnedStagingIdentity {
    staging_artifact: String,
    retained_parent_identity_digest: String,
    object: PhysicalFilesystemObjectIdentity,
}

pub(crate) enum VerifiedStagingLifecycle {
    Removed {
        staging: OwnedStagingIdentity,
    },
    ConsumedByTarget {
        staging: OwnedStagingIdentity,
        target_effect_digest: String,
    },
}

pub(crate) enum MutationCleanupState {
    VerifiedClean(VerifiedClean),
    Residue {
        otherwise_clean: Vec<VerifiedStagingLifecycle>,
        owned_staging: Vec<TypedMutationEffect>, // private ctor: nonempty
    },
    Unknown {
        otherwise_clean: Vec<VerifiedStagingLifecycle>,
        possible_staging: Vec<TypedMutationEffect>, // private ctor: nonempty
    },
}

pub(crate) enum MutationDurabilityState {
    VerifiedDurable {
        backend_contract: AtomicMutationBackendContract,
        proof_digest: String,
    },
    Unknown {
        stage: DurabilityUnknownStage,
    },
}

pub(crate) enum NoChangeSourceTreeProof {
    Unmodified,
    RolledBackDurably {
        backend_contract: AtomicMutationBackendContract,
        proof_digest: String,
    },
}

pub(crate) enum AtomicMutationBackendContract {
    LinuxContainedAtomicV1 { qualified_tuple_digest: String },
    MacOsContainedAtomicV1 { qualified_tuple_digest: String },
    WindowsContainedAtomicV1 { qualified_tuple_digest: String },
}

pub(crate) enum DurabilityUnknownStage {
    TargetDataFlush,
    CreatedParentMetadataSync,
    TargetInstallMetadataSync,
    StagingNameRemovalMetadataSync,
    InstalledTargetFlush,
    BackendObservation,
}
```

`call_tool()` owns `Option<ResolvedMutationCall>` for the full call. Future
guard and handler receive `plan` by the same reference; applied CFE additionally
receives the witness borrowed from the still-owned lease guard. Neither is
cloned, serialized or rebuilt. Constructors reject Applied CFE without witness,
Preview CFE with witness and non-CFE with either plan/witness. Fake tests record
plan address, witness address, grant-scope and execution digest at guard,
handler and post-mutation seams; an equal reconstructed object fails identity
testing.

An applied CFE invocation must return `Some(mutation)` regardless of
`AdapterOutcome.ok`. `None` is valid only for non-mutating preview or legacy
non-CFE handlers not yet migrated. A lower-level `Result::Err` after the first
filesystem mutation is forbidden: it would discard effect authority; a proven
pre-source-mutation error is converted to typed `NoChange` at the native
boundary. Fixed control-plane initialization may already have occurred and is
reported only by the internal lease observation channel, never by
`MutationEffectsV2`.
Constructors make state semantics uninhabitable-by-mistake:

- `NoChange::new` accepts no effect vectors and only `VerifiedClean`; target and
  every created source parent must be proven absent/rolled back. It means zero
  persistent source-artifact mutation effects, not zero control-plane setup;
  every lifecycle row must be `Removed` (a `ConsumedByTarget` row would prove a
  target commit and is rejected). `NoChangeSourceTreeProof::Unmodified` is legal
  only when no source-tree name/object was ever created. If temp/parent creation
  began, `RolledBackDurably` requires the exact backend transcript proving every
  staging unlink and created-directory removal plus each retained-parent
  namespace durability primitive; absence in a live walk alone is insufficient;
- `Committed::new` requires exactly one definitely occurred target-commit effect:
  either the expected path-resolved CreatedFile/UpdatedFile or one unexpected
  `DetachedOrRelocatedFilesystemObject` of target kind. Cleanup may be
  VerifiedClean, Residue or Unknown; durability independently must be
  VerifiedDurable or Unknown. Neither dimension changes definite commit into
  NoChange/Uncertain;
- `Uncertain::new` requires target completion not proven and retains every
  definitely occurred non-target effect plus every possible target/staging/
  detached effect. A definitely committed target cannot be downgraded to it
  merely because cleanup failed.

Cleanup is a name lifecycle, not global object extinction. `VerifiedClean`
contains every `OwnedStagingIdentity` created by the call exactly once and in
exactly one closed state:

- `Removed`: the retained-parent staging name is observed absent through the
  still-retained parent capability; this staging identity is not the definite
  target effect;
- `ConsumedByTarget`: the retained-parent staging name is proven absent and the
  staging object's physical identity is value-equal to the one definite target
  effect named by `target_effect_digest`. The target may be path-resolved or
  detached/relocated; its continued existence is required, not a cleanup gap.

Thus normal Present rename and Absent no-replace install can be clean while the
installed target still has staging identity `T`. `VerifiedClean` is forbidden
if any owned staging name remains, any extra owned staging object/name is known
or possible, the retained-parent name state cannot be queried, one created
staging identity is missing/duplicated, a
`ConsumedByTarget` row does not resolve to the unique definite target effect, or
two lifecycle rows use the same name/object. Its vector may be empty only when
no staging object was created and is bounded by `MAX_OWNED_STAGING=32`. Residue
and Unknown constructors apply the same bound to the union of `otherwise_clean`
and unresolved staging rows.

`target_effect_digest` is not display text. It is SHA-256 over
`"unica.mutation-effect.v2\0"` plus the stable effect tag
(CreatedDirectory=1, CreatedFile=2, UpdatedFile=3, OwnedStagingFile=4,
DetachedOrRelocated=5) and length-prefixed typed fields in declaration order.
Other v2 stable tags are: outcome NoChange=1/Committed=2/Uncertain=3; cleanup
VerifiedClean=1/Residue=2/Unknown=3; lifecycle Removed=1/ConsumedByTarget=2;
durability VerifiedDurable=1/Unknown=2; detached kind
CreatedDirectory=1/CreatedFile=2/UpdatedFile=3/OwnedStagingFile=4; detached
content NotApplicable=1/Known=2/Unknown=3; NoChange source-tree proof
Unmodified=1/RolledBackDurably=2; backend contract Linux=1/macOS=2/Windows=3.
Durability-unknown stage tags are TargetDataFlush=1,
CreatedParentMetadataSync=2, TargetInstallMetadataSync=3,
StagingNameRemovalMetadataSync=4, InstalledTargetFlush=5 and
BackendObservation=6.
Constructors recompute the digest and require it to select the unique definite
CreatedFile/UpdatedFile/detached-target row, and require exact
`PhysicalFilesystemObjectIdentity` equality with the staging row.
Every schema-v2 `*_digest` string is accepted only through a private constructor
that decodes exactly 64 lowercase-hex ASCII characters to 32 bytes (or receives
the internal 32-byte value before encoding); arbitrary text, wrong domain/length/case and display
digests cannot enter persisted authority.

`Residue` and `Unknown` do not forge a clean lifecycle for unresolved rows.
Their `otherwise_clean` rows account every other staging identity; their
nonempty `owned_staging`/`possible_staging` rows are respectively value-equal to
the complete owned-staging set in `effects.unexpected`/`effects.possible`.
Known residue after an Absent install may reference the same object as the
definite target but still records the extra staging **name**. An owned-staging
row is never expected. Live retained-parent absence is sufficient for the
structural cleanup dimension even when the later namespace sync fails:
cleanup may be `VerifiedClean` while committed durability is `Unknown`. A
remaining name is `Residue`; an unqueryable name state is cleanup `Unknown`.
Detached target/parent location does not itself choose cleanup.

`MutationDurabilityState` is independent **definite target/allowed-parent**
authority. `VerifiedDurable` is constructible only by an allowlisted backend
after all created-parent metadata, temp/installed-target data, target-install
parent and every committed staging-name removal/consumption namespace
durability primitive required by that exact tuple have succeeded and the proof
transcript matches `backend_contract`. For same-directory rename, one qualified
parent sync binds both the target install and consumed staging-name
disappearance. A future link-plus-unlink row requires a qualified final parent
sync after unlink. Cleanup is still derived from live retained-parent name and
identity observation; durability is never inferred from that live observation.
A failure or unavailable target-durability proof
after a definite target commit returns `Committed + Unknown { stage }`; it is
never cleanly advanceable and never downgraded to `Uncertain`. A failure before
target commit follows NoChange/Uncertain effect rules.

NoChange has no `Unknown` durability variant: its `Unmodified` or
`RolledBackDurably` proof is mandatory. Failure/unavailability of any unlink,
rmdir or parent namespace sync after source-tree creation makes strict NoChange
unconstructible and returns `Uncertain` with every known/possible directory and
staging effect. A successful in-memory rollback followed by failed sync cannot
claim zero persistent source-artifact effects because crash recovery may
resurrect a name.

`MutationEffectsV2.expected` and `.unexpected` contain only definitely occurred
effects; `.possible` contains only non-proven effects. Non-detached variants
require workspace-relative contained paths. A detached/relocated variant is
never `expected`: `intended_artifact` is a label, while the opaque volume,
object and retained-parent identity digests are authority. Constructors keep
vectors sorted/unique, bound every vector to `MAX_MUTATION_EFFECTS=32`, and never
serialize staging paths or physical identity into public diagnostics;
observations use domain-separated digests.

At discovery issuance one plan object reaches issuer assessment for that call.
A later apply necessarily builds a fresh current plan from a fresh snapshot;
pointer identity is not expected across calls. Cross-call authority is the
atomic grant tuple/scope digest plus current rolling baseline.

### 12.3 Native adapter/registry

- CFE dry-run requires `ResolvedMutationPlan::CfeMethodPatch` and typed preview;
- CFE apply requires the same variant plus a live exact artifact lease witness;
- CFE without plan -> `cfe_resolved_plan_required`, no legacy fallback;
- non-CFE with a plan -> `cfe_unexpected_resolved_plan`;
- generic native dry-run placeholder is unreachable for CFE;
- other operation signatures remain raw until their own resolvers land.

Replace raw `patch_extension_method(args, context)` with:

```rust
fn preview_cfe_method_patch(plan: &CfeMethodPatchPlan) -> HandlerOutcome;

fn apply_cfe_method_patch(
    plan: &CfeMethodPatchPlan,
    lease: &ArtifactMutationLeaseWitness,
    context: &WorkspaceContext,
) -> HandlerOutcome;
```

`cfe.rs` must not call raw argument helpers, map ModulePath, parse
Configuration.xml, select Context/kind, inspect warning paths or rerender.

---

## 13. Dry-run, apply and atomic write behavior

### 13.1 Dry-run

Dry-run executes selection, prepare, watched capture, shared material
projection, exact source/adoption/form checks, duplicate scan, render and all
digests. It reports:

- canonical analysis target and destination artifact;
- derived context/kind/async and assertion status;
- create/update preview plus exact parent directories that apply would create;
- grant-scope and execution-plan digests in typed diagnostics;
- no source/control-plane files, lock inodes, directories, leases, events, cache
  invalidation or receipt state changes.

Ambiguous source, unborrowed destination, wrong form mechanism, source method
gap, duplicate or bound fails exactly as apply preflight. Generic placeholder
output is forbidden.

### 13.2 Apply precondition

Applied resolution has already captured and resolved the plan while its exact
artifact lease is held. Task 8 invokes the final check directly at its recording
seam; future Task 10 invokes that same check after its optional receipt lease/
baseline reread. Immediately before any directory/file mutation, without
reparsing raw arguments:

1. run one final `capture_with_watches` with the plan's exact selected sources
   and watch; do not resolve or rerender a replacement plan;
2. require its source/composite fingerprints, watch outcomes and all execution
   material identities to equal the immutable plan;
3. revalidate exact source topology/mapping digest;
4. prove the supplied witness canonical locus equals the plan artifact mutation
   scope and current selected destination root still has the witness's retained
   physical identity, same-mount/volume proof and qualified backend tuple;
5. from the already-retained destination-root capability, walk the complete
   parent plan descriptor-relatively with no-follow/no-reparse semantics; an
   absolute or workspace-path destination reopen is forbidden;
6. require every PresentDirectory and AbsentDirectory state to remain exact;
7. Present target: compare exact bytes length/digest/boundary and object identity;
8. Absent target: prove the tombstoned target remains absent;
9. reconstruct final bytes and compare expected-after length/digest;
10. mismatch -> `cfe_patch_material_changed` or
    `cfe_artifact_scope_changed` with zero planned mutation.

This final recapture verifies the existing plan; it is not a second resolver and cannot change
target, source method, renderer or allowed effects. Source/adoption changes after
this recapture are subject to the same explicitly bounded non-cooperating-writer
window as Present replacement; the writer never rereads and reinterprets them
independently.

### 13.3 Contained atomic write

Do not use `create_dir_all`. Adopted registration proves descriptor authority,
not that `<Name>/Ext` or `<Form>/Ext/Form` already exists. The contained writer
performs this exact sequence while the artifact lease remains held:

1. before the first source-tree write, require the retained mount/volume proof
   and one exact row in the closed backend allowlist; bind its
   `qualified_tuple_digest` into `AtomicMutationBackendContract`;
2. validate the whole Present prefix and semantic Absent suffix;
3. create each planned absent directory ancestor-first using one-component
   create-new/no-follow semantics, record an opaque identity token for every
   directory actually created, run that backend row's exact directory/parent
   durability primitives, and fail/rollback rather than adopt a concurrently
   appeared component;
4. retain the complete verified directory-handle chain, open the exact resulting
   parent descriptor-relatively and create an unpredictable create-new/no-follow
   temp in it; record one `OwnedStagingIdentity` (name, retained parent and
   physical object) before writing;
5. write all planned bytes, flush the temp with the tuple's qualified primitive
   and preserve safe existing target permissions for Present replacement;
6. immediately repeat target type/identity/length/digest precondition;
7. install according to the exact tuple primitive below, then retain/open the
   installed object and verify final length/digest/physical identity;
8. prove the owned staging **name** absent through the retained parent. Classify
   the staging identity as `ConsumedByTarget` only when it equals the unique
   definite target object; classify it `Removed` only when it is not that target.
   A remaining name is Residue; an unqueryable name state is cleanup Unknown;
9. independently perform every target/created-parent and committed staging-name
   namespace durability primitive required by the tuple. A physical parent sync
   call may bind both target-install and consumed staging-name facts when its
   qualified transcript says so. Any required durability failure after definite install preserves
   `Committed` but sets `MutationDurabilityState::Unknown { stage }`;
10. resolve the expected path again from the retained destination-root handle and
    classify path-resolved versus detached target identity;
11. return exact target/effects, staging lifecycle and durability authority even
    when adapter presentation is failure.

For `AbsentWatched`, installation is no-replace: the Linux row uses
handle-relative `renameat2(..., RENAME_NOREPLACE)`, macOS uses
`renameatx_np(..., RENAME_EXCL)`, and Windows uses the exact no-replace
`FileRenameInfoEx` flags below. Each consumes the temp name into target and
therefore yields `ConsumedByTarget` after name-absence/object-equality proof.
Appearance of target loses the race and returns
`cfe_patch_absent_target_appeared` without overwriting it. If `linkat` commits
the target in a future separately qualified Unix row, its target keeps the same
object identity: successful temp-name unlink is `ConsumedByTarget`, while unlink
failure is `Committed` with expected CreatedFile plus unexpected
`OwnedStagingFile`/Residue. It is never NoChange. A platform without a tested
atomic no-replace primitive is a hard STOP, not permission to check then rename.

Unix mutation uses the closed `unica.unix-contained-atomic.v1` contract. Its
allowlist is data, not an optimistic `cfg(unix)` branch. A row is enabled only
when its exact OS/kernel build family, architecture, filesystem discriminator,
mount flags and native-suite identity produce a recorded
`qualified_tuple_digest`; no prefix/family fallback is allowed:

```text
qualifiedTupleDigest = SHA256(
  "unica.atomic-backend-qualified.v1\0" ||
  lp(contractId) || lp(osBuildPredicateId) || lp(architectureId) ||
  lp(filesystemDiscriminatorId) || lp(mountPolicyId) ||
  lp(lockPrimitiveTranscriptId) || lp(installPrimitiveTranscriptId) ||
  lp(durabilityPrimitiveTranscriptId) || lp(nativeSuiteEvidenceSha256))
```

`lp(x) = u64be(x.len) || UTF-8 x`. Every predicate/transcript ID is a closed
constant in `atomic_file.rs`; the evidence field is the reviewed native-suite
artifact SHA. Runtime facts must satisfy the exact row before its digest enters
the witness. Adding/changing any primitive, OS predicate or evidence creates a
different row/digest and requires spec/product-CI review.

Every enabled v1 row constructs the same root-wide
`FilesystemArtifactCollisionKey` before opening the persistent artifact lock.
No backend may substitute raw/case-folded/NFC/NFD locus bytes, target FileId or
a successful lookup. Linux/macOS/Windows may differ in path semantics, but v1's
over-lock deliberately makes those semantics irrelevant to serialization; the
qualified capability remains necessary for containment/install/durability.

| Candidate tuple | Runtime proof before source mutation | Process/OS lease | Parent/temp and install primitives | Durability transcript required for `VerifiedDurable` |
| --- | --- | --- | --- | --- |
| Linux kernel >= 5.8, local ext4 (`EXT4_SUPER_MAGIC`) | retained `statx` returns `STATX_MNT_ID`; workspace/control/destination components share the per-call mount ID; `fstatfs` magic exact and `fstatvfs` has `ST_RDONLY` clear on every handle; control-root capability probe proves the exact syscalls | process registry by universe key + `flock(LOCK_EX\|LOCK_NB)` on the persistent inode | `mkdirat`, `openat(O_CREAT\|O_EXCL\|O_NOFOLLOW)`, Absent `renameat2(RENAME_NOREPLACE)`, Present `renameat2(flags=0)`, rollback `unlinkat(temp,0)` then reverse `unlinkat(dir,AT_REMOVEDIR)` | `fsync` every created directory and its retained parent after mkdir; `fsync` temp before install; after install/readback `fsync` installed target and retained target parent, with the rename-parent transcript binding both target entry and consumed temp-name absence; every call succeeds |
| Linux kernel >= 5.8, local XFS (`XFS_SUPER_MAGIC`) | same proof, with exact XFS magic and its own qualified tuple digest | same explicit `flock` contract | same descriptor-relative primitives | same named calls, independently qualified native/crash suite; ext4 evidence cannot qualify XFS |
| macOS >= 13, local case-sensitive APFS (`f_fstypename=apfs`, `MNT_LOCAL`, not `MNT_RDONLY`) | retained `st_dev` + (`f_fsid`,`f_mntonname`) tuple equal across workspace/control/destination; exact `fpathconf(fd, _PC_CASE_SENSITIVE)=1` on every retained directory; exclusive rename probe; root-wide collision protocol | process registry by collision-universe key + `flock(LOCK_EX\|LOCK_NB)` | `mkdirat`, `openat(O_CREAT\|O_EXCL\|O_NOFOLLOW)`, Absent `renameatx_np(RENAME_EXCL)`, Present `renameatx_np(flags=0)`, rollback `unlinkat(temp,0)` then reverse `unlinkat(dir,AT_REMOVEDIR)` | `fcntl(F_FULLFSYNC)` on temp before install and installed target after readback; `fsync` each created directory/retained parent and target parent, with rename-parent transcript binding target entry + consumed temp-name absence; every required call succeeds |
| macOS >= 13, local case-insensitive APFS (`f_fstypename=apfs`, `MNT_LOCAL`) | same retained mount proof; exact `fpathconf(fd, _PC_CASE_SENSITIVE)=0` on every retained directory; its own qualified digest/native evidence; root-wide collision protocol is unchanged | same collision-universe process registry + `flock` | same descriptor-relative primitives | same named calls, independently qualified failure/crash suite; case-sensitive evidence cannot qualify this row |

Each displayed row is a shape that expands into separate exact architecture and
OS-build predicate rows (for example x86_64 versus aarch64/Apple Silicon); no
native evidence SHA or qualified digest is inherited across architectures,
kernel build predicates, APFS case modes or filesystems.

These are candidate tuple shapes, not filesystem-name assumptions. The Task 8
implementation/release gate enables a row only after native same/two-process
Busy, no-replace race, Present replacement, symlink/mount race, every injected
failure, staging identity and forced-crash/restart persistence suites pass on
that exact tuple. The durability gate is a disposable native VM/block-device
harness that terminates the guest/storage at every transcript seam, boots/
remounts, and verifies target/parent/staging state; an ordinary process kill or
one successful `fsync` is not durability proof. A happy syscall probe only
proves availability. Until a row's
qualified digest is present in the reviewed support matrix, it returns
`cfe_patch_atomic_backend_unsupported` before mkdir/temp/target mutation. NFS,
CIFS/SMB, FUSE, overlay, tmpfs, network, read-only, nested-mount and unknown
magic/flags are always unsupported in v1. Failure of a named durability call
after definite install returns `Committed + MutationDurabilityState::Unknown`;
it cannot mint `VerifiedDurable` from read-back or a live manifest.

The Windows backend is the versioned
`unica.windows-contained-atomic.v1` contract. Its initial candidate tuple is
local NTFS on Windows 10 build 17763+ or Windows Server 2019+ with exact
architecture/build/filesystem capability digest and the complete native suite;
it is enabled only after that digest is recorded in the reviewed support matrix.
Windows x64 and arm64/build-family rows have independent evidence and digests;
one never qualifies another.
network/SMB, ReFS, FAT/exFAT, older or unproven build/filesystem tuples fail with
`cfe_patch_atomic_backend_unsupported` before the first source-tree mutation.
Expanding that tuple requires the complete native race/failure suite and a
contract-version or package-matrix update; nominal API presence alone is not
support proof. The exact sequence is:

1. resolve trusted cwd once and open only the workspace root with `CreateFileW`
   as a directory capability using `FILE_FLAG_OPEN_REPARSE_POINT`; reject a
   reparse tag and retain its volume/FileId identity. On that handle and every
   retained destination-root handle, require exact successful
   `GetVolumeInformationByHandleW`: filesystem name `NTFS`, one unchanged volume
   serial, `FILE_READ_ONLY_VOLUME` clear. Also require exact successful ntdll
   `NtQueryVolumeInformationFile(FileFsDeviceInformation)` with
   `FILE_REMOTE_DEVICE|FILE_READ_ONLY_DEVICE|FILE_WRITE_ONCE_MEDIA|
   FILE_VIRTUAL_VOLUME` all clear and `FILE_DEVICE_IS_MOUNTED` set. Missing
   binding/field, SMB/remote/virtual/read-only result or disagreement is
   unsupported before control-lock/source mutation;
2. open **every** destination-source-root component from that workspace handle,
   then every planned parent, temp and target, using `NtCreateFile`/`NtOpenFile`
   (reusing the reviewed `contained_fs` primitive) with
   `OBJECT_ATTRIBUTES.RootDirectory=<retained parent handle>`,
   `FILE_OPEN_REPARSE_POINT`, the exact directory/non-directory option and
   retained `FileIdInfo`; parent creation uses create-new directory disposition.
   Every retained directory handle that can receive a namespace change is
   opened synchronously with the exact write/append access required by the
   qualified flush transcript plus
   `FILE_SHARE_READ|FILE_SHARE_WRITE|FILE_SHARE_DELETE`; an access-denied/open downgrade is unsupported before
   source mutation, never a reason to omit the later namespace flush;
   temp, installed-target and every directory created by this call additionally
   request `DELETE` access up front; failure cannot be repaired by path reopen;
3. acquire the persistent lock object with process-universe registry plus
   `LockFileEx(LOCKFILE_EXCLUSIVE_LOCK | LOCKFILE_FAIL_IMMEDIATELY)`; a
   control-root capability probe must already have qualified lock coherence,
   relative rename flags and the exact retained-directory
   `NtFlushBuffersFileEx(parent, Flags=0, Parameters=NULL,
   ParametersSize=0, &io_status)` transcript for this tuple;
4. flush the temp handle with `FlushFileBuffers` before install;
5. install the already-open temp with a reviewed
   `SetFileInformationByHandle(FileRenameInfoEx)` request whose
   `RootDirectory` is the retained target-parent handle and whose relative
   `FileName` is the target: `Flags=0` for Absent and exactly
   `FILE_RENAME_FLAG_REPLACE_IF_EXISTS` for Present;
6. classify destination-exists, unsupported information class/filesystem,
   sharing violation and unknown completion separately; unknown completion is
   `Uncertain`, never retry-as-NoChange;
7. reopen/read the target and prove the staging name absent through the same
   retained parent handle, then re-walk from the retained destination-root handle
   and compare VolumeSerial/FileId to detect a moved original parent/object;
8. call `FlushFileBuffers` on the installed target, then call exactly
   `NtFlushBuffersFileEx(retained_parent, Flags=0, Parameters=NULL,
   ParametersSize=0, &io_status)` and require `STATUS_SUCCESS`. The ordinary
   flags-zero operation is the only v1 directory-namespace candidate; data-only,
   no-sync, volume-handle and path-reopened substitutes are forbidden. The call
   uses the existing `windows-sys` `Wdk_Storage_FileSystem` ntdll binding; an
   unavailable exact binding/export excludes the tuple. The call
   succeeding by itself is not authority: its exact OS/architecture/NTFS tuple,
   access/share/open options and forced-crash evidence must match the reviewed
   `durabilityPrimitiveTranscriptId`. The transcript binds both target namespace
   durability and consumed staging-name absence. After each Windows directory
   create, staging delete-mark/close or rollback directory delete, the same flags-zero call runs on
   the retained receiving parent; a newly created directory is also flushed
   before its parent. The retained-parent live query in step 7 creates the
   structural `Removed`/`ConsumedByTarget` lifecycle; only a fully successful
   matched durability transcript creates `VerifiedDurable` or
   `RolledBackDurably`. Failure after rename can therefore be definite
   `Committed + VerifiedClean + durability Unknown`; inability to qualify the
   parent namespace flush capability keeps the tuple out of the allowlist before
   any source write.
9. on a precommit rollback, compare the retained temp/directory FileId first and
   enumerate each created directory as empty, then call exactly
   `SetFileInformationByHandle(handle, FileDispositionInfo,
   FILE_DISPOSITION_INFO { DeleteFile=TRUE })`, close that retained handle,
   requery its one name through the retained parent and require absence. Temp is
   removed first and created directories are removed reverse-order. After every
   proved name removal, run the flags-zero retained-parent
   `NtFlushBuffersFileEx` transcript from step 8. Delete-mark/close/query/flush
   failure or an external handle that leaves the name present is Uncertain with
   Residue/Unknown, never path-based retry or NoChange.

Path-based `MoveFileExW`, path-based `CreateDirectoryW` after a checked walk,
path-based `DeleteFileW`/`RemoveDirectoryW`, path-based `CreateFileW`
destination reopen, or any absolute-path reopen after
the one workspace open cannot satisfy this contract. Build/filesystem support,
required information classes and access/share flags are validated before the
first source-tree mutation. Native same/two-process, race, failure and forced-
crash/restart durability tests must run on every claimed tuple; official API
shape or one successful call is insufficient qualification.
Retained handles prevent a parent swap from redirecting the operation to a
replacement directory; they do not create a hash-CAS or prevent a
non-cooperating actor from renaming the already-open directory object itself.
Post-path/identity verification therefore uses the detached-object contract
below rather than inventing an in-scope path effect.

After any platform reports a definite commit, the writer compares the retained
target/parent identities with a fresh handle-relative walk from the retained
destination root. If the intended path is absent or resolves to another object,
the target effect is
`DetachedOrRelocatedFilesystemObject { intended_artifact, kind,
volume_identity_digest, object_identity_digest,
retained_parent_identity_digest, content }`. A definite install completion
returns non-advancing `Committed`; cleanup is independently `VerifiedClean`
when every staging name is absent and its identity is exactly Removed or
ConsumedByTarget, `Residue` when an extra owned staging name is known present,
and `Unknown` when staging-name state cannot be proven. A relocated installed
target may still be the exact `ConsumedByTarget` object and therefore clean;
the detached target effect independently prevents advance. It never
emits an expected CreatedFile/UpdatedFile at the intended path. Unknown install
completion returns `Uncertain` with the detached effect in `possible` and every
known created parent/staging effect retained. Unix uses `fstat`/`fstatfs` handle
identity; Windows uses VolumeSerial/FileId. The identity digests are
domain-separated opaque values and are preserved through Task 10 revocation.

For Present, the writer performs an atomic same-directory replacement only
after the final exact precondition. The artifact lease makes that replacement
serializable against every Unica writer, including receipt-free modes. It is
not advertised as a filesystem compare-and-swap against arbitrary external
processes: an external writer that commits before the final check is detected,
but a non-cooperating writer can still race in the check-to-replace window. Rust
and the target filesystems expose no portable content-hash conditional replace.
If the product requires zero lost updates against arbitrary external writers,
STOP and add a separately proven platform-specific transaction/CAS contract;
do not call the cooperative Present operation “conditional replace”.

On any failure before file commit, remove the temp and remove only directories
created by this call, reverse order, requiring the recorded identity and an
empty directory. After each temp unlink/rmdir, run the exact qualified retained-
parent namespace durability primitive before claiming restoration; the rollback
transcript covers every name removal in reverse order. Only a fully successful
and durable transcript creates `NoChangeSourceTreeProof::RolledBackDurably` and
restores the pre-call tree. A rollback identity/nonempty/removal **or namespace-
sync** failure returns `cfe_parent_rollback_failed`, exposes typed Uncertain
known/possible effects and forces future Task 10 receipt revocation; live name
absence after failed sync is not NoChange. After the
file commits, created parents are legitimate allowed effects and are never
rolled back. Post-write digest mismatch is `cfe_patch_after_digest_mismatch` and
also forces revocation. Temp removal failure before target commit is `Uncertain`
with `MutationCleanupState::Residue` and an `OwnedStagingFile`; a cleanup/open
error that cannot prove whether the temp exists uses
`MutationCleanupState::Unknown` plus
`possible`. The artifact lease remains held through typed
classification and direct cleanup verification. A target commit followed by any
required parent/namespace durability failure remains definite `Committed`,
retains its independently derived cleanup state, carries
`MutationDurabilityState::Unknown`, and is non-advancing/revoking.

### 13.4 Typed result authority

Plan/handler authority uses workspace-relative slash paths only when a fresh
handle-relative walk proves the current path resolves to the retained object;
detached effects use physical identity and only label their intended path.
Display changes, stdout and artifacts are derived after preview/apply and never
parsed back.
Task 8 returns the generic `MutationHandlerOutcome` from §12.2. A successful CFE
commit has expected effects equal to ancestor-first `CreatedDirectory` rows for
exact `write.directories_to_create` plus exactly one `CreatedFile` or
`UpdatedFile` for the allowed artifact. `unexpected` and `possible` are empty,
cleanup proves every owned staging identity exactly Removed or ConsumedByTarget,
and durability is `VerifiedDurable` for the exact qualified backend transcript.

`NoChange` is legal only when the target was not committed, all created
directories were identity-checked and removed, every staging path is proven
Removed, and all three **source-artifact** effect vectors are empty. Expected
fixed control directories and persistent lock inode may have been initialized
before this result; they are not grant effects. It additionally carries
`Unmodified` when no source mutation began or `RolledBackDurably` after every
removal and retained-parent namespace sync succeeds. A known committed target with a
staging residue remains `Committed`, carries `MutationCleanupState::Residue` and has an
unexpected `OwnedStagingFile`; a known commit with unobservable cleanup carries
`MutationCleanupState::Unknown`. `Uncertain` covers every state in which the target is
not definitely committed but the strict NoChange invariant cannot be built:
known precommit residue/partial parent effects as well as unknown install or
rollback target completion. A renamed-away retained parent/object uses
the detached effect above and never a fabricated expected path. Task 10 compares
expected effects with the plan and post-manifest, requires
`MutationCleanupState::VerifiedClean` plus
`MutationDurabilityState::VerifiedDurable`, and revokes on durability Unknown,
any unexpected/possible/detached, Residue/Unknown/Uncertain row. It never parses adapter display and does not
assume that a random staging name appears in the ordinary source manifest.

---

## 14. Stable reason codes

Every rejection has a stable leading snake_case code and bounded detail.

| Group | Codes |
| --- | --- |
| raw aliases/types | `cfe_missing_argument`, `cfe_unexpected_argument`, `cfe_argument_type`, `cfe_alias_conflict` |
| source selector | `cfe_analysis_source_not_found`, `cfe_analysis_source_required`, `cfe_analysis_source_mismatch`, `cfe_analysis_source_unsupported`, `cfe_analysis_configuration_required` |
| semantic path | `cfe_invalid_identifier`, `cfe_generated_method_name_invalid`, `cfe_invalid_module_path`, `cfe_unknown_metadata_kind`, `cfe_unknown_module_kind`, `cfe_invalid_interceptor`, `cfe_invalid_context` |
| destination mapping | `cfe_destination_not_found`, `cfe_destination_case_alias`, `cfe_destination_kind_unsupported`, `cfe_destination_format_unsupported`, `cfe_extension_path_mismatch`, `cfe_destination_path_unsafe` |
| watch/registration | `snapshot_watch_limit`, `snapshot_watch_invalid`, `snapshot_watch_conflict`, `snapshot_watch_source_not_selected`, `snapshot_watches_unsupported`, `source_changed_during_capture`, `cfe_destination_parent_unsafe`, `cfe_destination_parent_case_alias` |
| source method | `cfe_source_module_missing`, `cfe_source_method_missing`, `cfe_source_method_spelling_mismatch`, `cfe_source_method_ambiguous`, `cfe_source_method_inconclusive`, `cfe_source_signature_inconclusive`, `cfe_context_assertion_mismatch`, `cfe_method_kind_assertion_mismatch`, `cfe_function_interceptor_incompatible` |
| form/adoption | `cfe_form_handler_wrong_mechanism`, `cfe_destination_form_handler_collision`, `cfe_form_binding_inconclusive`, `destination_borrow_required`, `destination_object_not_adopted`, `destination_extended_object_mismatch`, `analysis_metadata_identity_inconclusive`, `analysis_descriptor_not_base_owned`, `destination_membership_inconclusive` plus exact shared provider/gap reason |
| configuration | `cfe_configuration_missing`, `cfe_configuration_malformed`, `cfe_configuration_flavor_mismatch`, `cfe_name_prefix_missing`, `cfe_name_prefix_duplicate`, `cfe_name_prefix_invalid`, `cfe_script_variant_missing`, `cfe_script_variant_duplicate`, `cfe_script_variant_unknown` |
| module/render | `cfe_module_too_large`, `cfe_module_encoding_unsupported`, `cfe_module_preflight_inconclusive`, `cfe_duplicate_interceptor`, `cfe_duplicate_generated_method`, `cfe_rendered_patch_limit`, `cfe_final_module_limit` |
| batch | `cfe_batch_plan_conflict` |
| expected/grant | `cfe_proposal_target_mismatch`, `cfe_destination_source_mismatch`, `cfe_allowed_effects_mismatch`, `cfe_grant_scope_mismatch`, `cfe_execution_plan_mismatch` |
| lease/apply | `cfe_artifact_busy`, `cfe_artifact_lock_failed`, `cfe_artifact_identity_unavailable`, `cfe_artifact_scope_changed`, `cfe_control_mount_boundary_unsupported`, `cfe_destination_mount_boundary_unsupported`, `cfe_resolved_plan_required`, `cfe_unexpected_resolved_plan`, `cfe_patch_material_changed`, `cfe_patch_target_unsafe`, `cfe_patch_absent_target_appeared`, `cfe_patch_atomic_backend_unsupported`, `cfe_patch_write_failed`, `cfe_patch_staging_cleanup_failed`, `cfe_patch_commit_uncertain`, `cfe_patch_durability_unknown`, `cfe_patch_object_relocated`, `cfe_parent_rollback_failed`, `cfe_patch_after_digest_mismatch` |

The direct tool and discovery reuse the exact Task 5A membership leading codes;
there is no `cfe_destination_borrow_required` alias that could collapse Own or
wrong-UUID states. Snapshot/common capture reasons remain the exact common codes
in §7.5. Errors never dump raw arguments, source body, absolute lock path or
unrelated paths.

`cfe_analysis_configuration_required` is the direct tool's stable leading
reason, but discovery carries it only as the reasonCode of the closed
application-owned `DiscoveryPreflight/mutation_preflight` tuple in §5.4. It is
never a provider-evidence or `source_readiness` reason.

---

## 15. RED -> GREEN implementation sequence

### 15.0 Mandatory RED and back-propagation matrix

Every row must first fail for the named missing behavior, then turn GREEN. A
documentation edit without its RED test, or a test weakened to accept the old
behavior, does not satisfy the row.

| Contract gap | Required RED proof | Mandatory back-propagation before Task 8 code |
| --- | --- | --- |
| prepare/material phase ownership | `prepare_has_zero_material_reads`: panic snapshot/material/fs fakes record zero calls; capture is the first reader | Task 7 design/order; active spec two-stage resolver; CFE skill troubleshooting must not tell the handler to probe live files |
| Configuration-only analysis authority | Configuration + PlatformXml resolves; Extension analysis remains visible in Explore but its CFE proposal is Unknown/ineligible, issuer-silent and has exactly one proposal-only `mutation_preflight` check per canonical affected chunk from `DiscoveryPreflight` with skipped/inconclusive/unknown/blocking, `cfe_analysis_configuration_required`, retryable false and empty details/evidence; no `source_readiness` check; wrapper UUID decoy never joins | Task 5B typed fact/source-kind preservation and Task 7 application preflight validation; active spec/ADR/skill; explicitly forbid a v1 `BaseMetadataIdentity` alternative |
| captured flavor + Own base authority | strict MDClasses tracked base-with-compat/no belonging+purpose is Base; tracked Adopted+Customization extension without compatibility/KeepMapping is Extension; optional fields valid on either flavor never discriminate; wrong namespace/partial/other/duplicate flavor emits no CFE membership/eligible row; Adopted analysis root/Form fails before UUID comparison; wrapper/local UUID decoy cannot join | Task 5A owns closed flavor/membership facts and blocker projection; Task 5B shared catalog exact §6.4 flavor/emission gate + descriptor Own; Task 7 issuer requires proof; Task 9 grant stores it and Task 10 current guard re-proves it; active spec/ADR/skill/product contract |
| exact membership lattice | table test for Absent -> RequiresBorrow, Own -> Unknown/not-adopted, equal Adopted -> ExtensionOwned, wrong Adopted -> Unknown/mismatch, malformed/gap -> Unknown/inconclusive; mixed Form row preserves Unknown precedence | Task 5A + Task 5B provider contract; active spec; CFE skill recommends borrow only for descriptor absence |
| single accepted Form authority | static/product scan proves Task 8 defines no Form item/event/callType/Action registry; accepted Task 5B registry tests include current audited `Button/Click` rejection, every accepted event-bearing kind such as `RadioButtonField/OnChange`, unknown event fail-closed, exact regular-vs-extension/BaseForm callType rule, zero/duplicate Action cardinality, lexical/duplicate N/N+1; Task 8 consumes only registry version + complete lookup | Task 5B must land one neutral registry shared by edit/validate/catalog and receive fresh no-P0/P1 acceptance on accepted Task 5A SHA; Task 6 identity service; active spec/product tests point to that authority without copying rows |
| destination Form generated-name ownership | accepted complete destination lookup bound to generated name with no BSL definition still blocks; case variant, orphan and any incomplete destination projection fail closed; unrelated accepted binding preserves scope digest but changes execution material | Task 5B captures/projects both Forms under the same registry version; Task 7 issuance carries both proofs; Task 9 stores version/semantic proof/baseline and Task 10 re-proves it; active spec/ADR/skill |
| complete observed interceptor set | RU/EN Before, After, Around/Вместо and ModificationAndControl facts plus exhaustive 4x3 conflict matrix and conditional/malformed fail-closed cases | Task 6 parser design/cache schema/tests; active spec duplicate rules; CFE skill duplicate diagnostics |
| immutable exact source slices | forged definition/declaration/name/parameter/body/terminator range and line-ending cases each fail; changing any exact slice changes the specified digest fixture | Task 6 DTO/parser/cache version/tests; active spec digest table; CFE skill source-bound explanation |
| canonical assertion erasure | omitted and explicit matching SourceSet/Context/IsFunction yield value-equal resolved cores/full plans/digests; diagnostic assertion flags may differ but never enter plan/grant/handler | Task 7 prepared intent owns Options and resolved issuance owns canonical core; Task 9 grant schema stores only resolved values; Task 10 guard compares that canonical core and cannot observe assertion presence; active spec/ADR/skill/product contract |
| first-patch parent chain | borrowed descriptor with missing `<Name>/Ext` succeeds; file/symlink/case alias fails; N/N+1 chain; failure after k mkdirs removes only those k in reverse; grant A creates parents and grant B keeps the same stable creation scope but resolves zero current mkdirs; rollback uncertainty is typed | Task 4 watched manifest/outcome; Task 9 stable grant allowed-effects model; Task 10 current typed effects/post-diff; active spec/ADR/skill |
| collision-safe artifact serialization universe | every v1 backend maps all semantic loci under one physical destination root to one root-wide collision key/inode. Two processes over path/case/NFC/NFD/bind aliases of the same whole physical workspace prove same control-root, collision key, opened lock inode/FileId and one Busy for Present and Absent; semantic locus bytes may differ. APFS case-sensitive and case-insensitive rows each run NFC/NFD + case-pair REDs; even genuinely distinct case-sensitive files intentionally over-lock. W1/W2 with one shared destination bind are rejected before capture. Equal semantic/collision-key bytes alone never satisfy the actual-inode+Busy RED | Task 9 reuses Task 8 collision protocol/universe/mount/backend; Task 10 deduplicates collision keys before receipt locks; active spec/ADR define root-wide v1 over-lock, mixed-version migration gate and delete broad destination-bind/canonical-locus-lock claims |
| one workspace control universe | control path is exactly `.build/unica/project-discovery/control-v1` below retained workspace with no `<workspaceKey>`; cache/cwd/PID aliases cannot split it; artifact/receipt namespaces are siblings; unsafe `.build/unica` link/reparse fails closed | Historical Task 9 deletes `${cache_root}` and workspace-key hashing; active spec/ADR/deployment/package tests name the exact fixed root; Task 10 accepts no process-cache lease |
| honest install/replace semantics | Absent external creator wins and is never overwritten; Present precheck catches completed edit; test/docs explicitly do not assert CAS against a non-cooperating writer in the check/replace window | Task 9 shared atomic primitive; Task 10 uncertainty/revocation; active spec/ADR/skill guarantee boundary |
| Unix backend boundary | exact qualified Linux-ext4, Linux-XFS and both macOS-APFS case-mode tuple rows run OS-lock Busy, no-replace/replace, staging, directory durability, every failure and forced-crash/restart suites; NFS/CIFS/FUSE/overlay/tmpfs/network/unknown/missing tuple fails before source mkdir/temp | Task 9 reuses `unica.unix-contained-atomic.v1`; Task 10 consumes durability; package/CI support matrix, active spec/ADR and support docs name exact qualified digests/primitives |
| Windows handle-relative containment/durability | after one trusted workspace `CreateFileW`, handle queries prove exact NTFS, same serial, writable mounted non-remote/non-virtual device; every destination-root/parent/temp/target/install/rollback-delete is retained-handle-relative; exact tuple qualifies LockFileEx, FileRenameInfoEx, DELETE-access `FileDispositionInfo` temp/directory rollback, file `FlushFileBuffers` and flags-zero retained-directory `NtFlushBuffersFileEx` after install/mkdir/delete with native race/failure/crash suite; missing binding/export or unsupported tuple fails before source mutation; path/no-sync/data-only/volume substitutes are statically forbidden | Task 9 reuses backend/schema; Task 10 consumes detached/durability Unknown; active spec/ADR/skill and Windows package matrix |
| staging-name lifecycle | Absent hard-link identity T -> target T + removed temp name constructs ConsumedByTarget clean; Present rename does same; extra temp name for T is Residue; detached target with absent temp remains clean/nonadvancing; every created staging appears exactly once as Removed/Consumed only in VerifiedClean | Task 9 schema v2 stores closed tags/bounds; Task 10 requires clean; active spec/ADR/observability/replay distinguish staging name from target object extinction |
| durability authority | parent/namespace sync failure after definite Present and Absent commit/readback/clean staging returns Committed+durability Unknown; preinstall data-flush failure attempts reverse removal and is NoChange only after the complete RolledBackDurably transcript, otherwise Uncertain; exact expected Committed advances only with VerifiedDurable+VerifiedClean | Task 9 persists generic schema/version only; Task 10 owns durable+clean advance/revoke; active spec/ADR/platform matrix/observability/replay preserve field |
| NoChange effect scope and durable rollback | first applied call initializes fixed control/lock then fails pre-source-mutation: NoChange+Unmodified has empty source effects and separate control telemetry. After each temp/parent creation, inject unlink/rmdir and retained-parent namespace-sync failures plus forced crash: only complete transcript yields Removed-only `RolledBackDurably`; any failure is Uncertain with known/possible effects | Task 9 schema v2 preserves NoChange proof; Task 10 ignores control rows and never advances NoChange; spec/observability/replay say zero durably restored source-artifact effects, never zero all filesystem effects |
| typed target/cleanup/effect authority | type tests reject NoChange+effects/ConsumedByTarget/nonclean, allow definite Committed with every cleanup+durability combination, reject Committed without one target effect; failure injection preserves target/cleanup/durability independently; moved retained parent is detached identity | Task 9 generic schema/bounds only; Task 10 advances exact expected VerifiedDurable+VerifiedClean Committed and revokes durability Unknown/Residue/Unknown/detached/possible/Uncertain; active spec/ADR/observability/replay |
| artifact/receipt lock order ownership | Task 8 recording fakes/pure algebra expose artifact-held handler/post seams but do not implement receipt persistence/lease; Task 9/10 production RED proves artifact -> current receipt acquisition, receipt -> artifact release, no second receipt under either | Task 9 owns receipt lease/store tests; Task 10 owns production pipeline and `discovery_receipts` integration; active spec Guard Order + ADR lease section must be changed before Task 8 code |

`tests/ci/test_product_contracts.py` must assert the normative clauses in the
active spec and `plugins/unica/skills/cfe-patch-method/SKILL.md`; it must also
reject the stale phrases “all non-Adopted means borrow”, “prepare validates
descriptors”, “no parent creation”, “three observed decorators”, and
“atomic replace prevents every external lost update”; it must also reject
`source_readiness` carrying `cfe_analysis_configuration_required`, a lock path
rooted in `<cache_root>`/`UNICA_CACHE_DIR`, Windows CFE `MoveFileExW`, a
BSL-only Form duplicate proof, and an applied CFE API returning only
`AdapterOutcome`. It additionally rejects `<workspaceKey>` beneath the fixed
workspace control root, source-set/mapping digest in the artifact lease key,
path-based destination `CreateFileW`, mandatory extension-only compatibility/
KeepMapping, numeric-normalized Form IDs, `Committed` requiring a clean-only
proof, and an intended-path effect for a detached object. It additionally
rejects any Task 8 Form registry/table, global staging-object extinction as
cleanliness, an advance predicate without durability, key-byte equality as lock
proof, broad destination bind aliases, generic `cfg(unix)`/filesystem fallback,
canonical-locus/case-fold/NFC bytes as the v1 lock filename, and unqualified
“NoChange has zero persistent effects”. Historical Task
5B/6/9/10, outcome-schema, observability/replay and support-matrix texts must be
corrected in the same prerequisite slice so a later worker cannot reintroduce
the reviewed defects.

### Task 8.0: Back-propagation and prerequisite gate

- [ ] Update Task 5A/5B/6/7/9/10 designs, active spec, ADR, CFE skill,
  outcome-schema/observability/replay text and platform support matrix with
  sections 1-2 before Task 8 production code. Record accepted committed Task 5A
  SHA and fresh accepted Task 5B review SHA; absence/open P0/P1 is RED.
- [ ] Add product-contract assertions for:
  - source-bound signature/body;
  - derive-on-absence Context/IsFunction;
  - optional SourceSet exact selection;
  - Configuration-only analysis for CFE mutation/receipt, with Extension
    analysis remaining reportable but exact Unknown/ineligible and issuer-silent
    for that proposal;
  - fixture-proven flavor table, optional non-discriminating compatibility/
    KeepMapping, declared/captured agreement and Own-only analysis UUID authority;
  - exact application `DiscoveryPreflight/mutation_preflight` tuple from §5.4,
    proposal-only sorted/chunked affects, and the prohibition on expressing this
    mutation blocker as `source_readiness`;
  - exact Absent/Own/Adopted-mismatch/inconclusive lattice and no implicit
    borrow outside the actual absence row;
  - one accepted neutral Task 5B Form registry/version shared by edit/validate/
    catalog, no Task 8 table, complete analysis unbound and destination
    generated-name-unbound lookup proofs;
  - observed Around conflict matrix;
  - all source spans/line ending and exact digest encoding;
  - typed parent creation/rollback and allowed effects;
  - fixed descriptor-relative control root with no workspace key, separate
    physical-destination-root + canonical-locus semantic key, and one
    backend-qualified root-wide filesystem collision key independent of locus,
    mapping, source/workspace aliases and process cache environment;
  - conservative whole-workspace mount universe, physical control/lock identity
    and actual inode/FileId+Busy proof; APFS NFC/NFD and both case-mode aliases
    share the root-wide inode for Present/Absent; internal shared-destination bind
    rejected;
  - exact qualified Linux ext4/XFS, both macOS APFS case-mode and Windows NTFS backend tuples,
    named lock/install/durability primitives and fail-before-source-write rule;
  - one Windows workspace open followed by handle-relative destination-root/
    parent/temp/install contract, versioned support tuple, exact unsupported
    boundary and honest Present/Absent write guarantees;
  - typed NoChange/Committed/Uncertain outcomes on both adapter success/failure,
    target certainty separated from Removed/Consumed staging cleanup and
    VerifiedDurable/Unknown durability, detached physical identities,
    source-effect-scoped NoChange and Task 10 durable+clean revocation rules;
  - canonical final-plan equality after optional assertions are validated;
  - grant-scope versus execution-plan digest and rolling baseline.
- [ ] Run `python3 tests/ci/test_product_contracts.py`; expected GREEN for
  accepted text. A zero-assertion accidental pass is failure.
- [ ] Confirm Task 4 empty-watch fixture hashes + parent-watch tags, Task 5
  shared catalog/adoption/form projections, Task 6 extraction spans + observed
  annotations and Task 7 zero-read prepare/report path are GREEN.
- [ ] STOP if any prerequisite still defaults Context/IsFunction, treats
  ExtensionRequired as eligible, collapses Own/mismatch into borrow, lacks a
  complete accepted Task 5B neutral Form registry/dual-source projection/Around
  fact, or Task 5A lacks accepted committed SHA/Task 5B has open P0/P1; accepts a declared-kind-only
  Configuration or wrapper UUID as CFE base authority, roots locks in a process
  cache or `<workspaceKey>`, keys artifacts by mapping/source name, uses a
  path-based Windows destination mutation primitive, drops typed failure/
  detached effects, forces Committed to be clean, lacks durability/staging-name
  authority, accepts internal mount/network/unknown backend, equates key bytes
  with the lock inode, retains assertion presence in the final plan, or lacks
  exact source ranges/line ending.

### Task 8.1: Domain parser, aliases and source selector RED/GREEN

- [ ] Add RED tests:
  - exact common/owner/form ModulePath shapes and canonical ArtifactRef target;
  - six module kinds; unknown/extra/empty/traversal/separator rejection;
  - all seven Pascal/lowerCamel alias-pair permutations;
  - every dual-alias conflict and unexpected key;
  - SourceSet explicit exact, omitted-single, omitted-zero/multiple, case alias;
  - omitted SourceSet with one Configuration + any number of Extensions selects
    that Configuration and is not ambiguous;
  - Configuration analysis accepted; Extension analysis fails exact
    `cfe_analysis_configuration_required` before material reads; direct failure
    does not manufacture a discovery check;
  - Context/IsFunction absent stay None; present map to exact assertions;
  - omitted and explicit-matching assertions resolve to value-equal
    `ResolvedCfeMethodPatchCore` and full plan fixtures; only detached diagnostic
    flags differ;
  - structural aliases normalize, user identifier spelling does not;
  - `PanicOnMaterialRead` snapshot/material/filesystem fakes prove prepare makes
    zero material calls while deriving exact parent/watch plus physical-root
    candidate and canonical locus; it does not invent a path-derived key.
- [ ] Run
  `cargo test --locked -p unica-coder cfe_method_patch::tests -- --nocapture`;
  expected RED because domain/prepare types do not exist; zero tests is failure.
- [ ] Implement closed types, identifier/module parser, prepared request,
  canonical resolved core, seed, lease candidate/locus and initial topology
  preparation only.
- [ ] Re-run expected GREEN. Review domain has no filesystem/JSON/raw Map
  dependency beyond application extraction.

### Task 8.2: SnapshotWatch RED/GREEN

- [ ] Add RED snapshot tests:
  - existing `capture()` and empty `capture_with_watches` fixture hashes equal;
  - 32 watches accepted, 33 rejected before scan;
  - 4096-byte path accepted, 4097 rejected;
  - duplicate semantic watch dedupes; conflicting purpose, prerequisites or
    parent-component vectors reject;
  - registered Present reuses ordinary entry;
  - registered absent gets exact watched tombstone/tag;
  - Common/owner/form parent chains record exact Present prefix + Absent suffix;
  - first missing component makes every descendant semantically Absent without
    traversal; file/link/reparse/case alias returns exact Unresolved;
  - parent appearance/disappearance/type change between scans returns no
    snapshot; watched parent tags change only nonempty-watch fixtures;
  - 8 parent components accepted and 9 rejected before scan;
  - missing root/form returns typed Unresolved and never opens physical decoy;
  - appear/disappear/content/symlink/case/registration change between scans
    returns no snapshot;
  - optional verified read observes unchanged tombstone only;
  - default fake rejects nonempty watch with exact unsupported code.
- [ ] Run focused Task 4 source-snapshot tests; expected RED.
- [ ] Implement common domain/API/fingerprint/capture behavior.
- [ ] Re-run all Task 4 + new tests and compare saved empty-watch hashes.

### Task 8.3: Shared source method extraction RED/GREEN

- [ ] Extend Task 6 RED tests for definition/declaration/name/parameter/body/
  terminator spans under RU, EN, mixed tokens, CRLF/LF, BOM and Unicode names.
- [ ] Add extraction tests for:
  - zero/multiple/conditional/malformed target definitions;
  - Procedure/Function, Async, Export, Val/default and 0/N parameters;
  - exact MethodName versus case-fold-only spelling;
  - all six derived BSL contexts including ModuleDefault;
  - Context/IsFunction omitted derive exact facts;
  - explicit matching assertions equal omitted digest;
  - mismatching assertions and Function+Before/After reject;
  - each forged range relation, line-ending mismatch, slice/digest mismatch and
    stale parser cache reject;
  - fixed definition/signature/body vectors use the exact §4.5 byte order.
- [ ] Run Task 6 parser + CFE material tests; expected RED.
- [ ] Implement spans and `CfeResolutionMaterialPort` source projection using
  one parser and verified snapshot bytes.
- [ ] Re-run all Task 6 evidence/cache tests to prove no semantic regression.

### Task 8.4: Adopted UUID chain and form mechanism RED/GREEN

- [ ] Add Russian/English Platform XML fixtures:
  - exact Adopted root; exact Adopted root+form;
  - missing/Own/wrong-case/duplicate ObjectBelonging;
  - missing/duplicate/malformed/cross-object ExtendedConfigurationObject;
  - same-name extension-owned object decoy;
  - Extension-as-analysis wrapper UUID equal/different decoys never substitute
    for Configuration `MetadataIdentity` and produce no plan;
  - source declared Configuration but captured ExtensionConfiguration,
    destination declared Extension but captured BaseConfiguration, partial/mixed
    flavor fields, and Adopted analysis root/Form; a deliberately equal wrapper
    UUID still fails before UUID join;
  - tracked base configuration with compatibility mode and no belonging/purpose
    is Base; tracked Adopted+Customization extension with no compatibility or
    KeepMapping is Extension; valid optional fields on either flavor do not
    change it, while wrong-MDClasses-namespace/duplicate/invalid material is
    inconclusive and emits no CFE membership/eligible row;
  - root correct but form UUID wrong;
  - accepted Task 5B analysis projection lookup Unbound versus Bound;
  - accepted destination projection generated-name Unbound versus Bound,
    including canonical case identity and orphan BSL case;
  - incomplete projection and mismatched/unaccepted
    `PlatformFormRegistryVersion` fail closed without inspecting internal rows.
- [ ] In the prerequisite Task 5B slice, run its authoritative exhaustive Form
  registry suite: audited valid/invalid kind-event pairs, unknown token,
  regular-vs-extension/BaseForm callType, zero/duplicate Action, lexical limits,
  recursion and completeness. Product/static tests prove Task 8 contains no
  item/event/callType/Action enum/table or parser branch.
- [ ] Assert exact reason partition and aggregation: descriptor Absent ->
  RequiresBorrow/`destination_borrow_required`; present Own -> Unknown/
  `destination_object_not_adopted`; Adopted wrong UUID -> Unknown/
  `destination_extended_object_mismatch`; provider/gap -> Unknown/inconclusive;
  an Unknown root/Form wins over an absent sibling; every row is ineligible.
- [ ] Add RED tests proving only exact root/form UUID chain is ExtensionOwned
  and receipt-resolvable; ExtensionRequired has blocker and no plan.
- [ ] Run Task 5 providers/support + accepted registry/material tests; expected
  RED until Task 5B is accepted and Task 8 lookup consumption exists.
- [ ] Extend only shared Task 5 catalog/descriptor/flavor projection as required
  by the accepted prerequisite, then implement Task 8 consumption of its opaque
  complete Form lookup/version. Do not define or parse Form rows in CFE resolver.
- [ ] Re-run Task 5/7 tests expected GREEN.

### Task 8.5: RU/EN source-bound renderer RED/GREEN

- [ ] Add exact golden cases for both destination variants:
  - Before/After with 0/N parameters, Val/default and Async;
  - mandatory `Перед`/`После` and `Before`/`After` generated-name
    suffixes, coexistence, and no order-dependent fallback name;
  - ModuleDefault (no directive) plus all five explicit context directives;
  - ModificationAndControl Procedure/Function with RU/EN/mixed source body;
  - exact name splice only; body/terminator bytes preserved;
  - destination variant changes only generated semantic tokens;
  - NamePrefix/generated-name boundary;
  - Absent/PresentEmpty/PresentEndsLf/PresentOther + BOM rules;
  - patch/final 16 MiB N/N+1 checked arithmetic.
- [ ] Run focused renderer tests; expected RED.
- [ ] Implement pure renderer, boundary and final digest calculation.
- [ ] Run repository real 1C compile/load validation for the four
  ModificationAndControl RU/EN Procedure/Function fixtures. A skipped test is
  not proof; if environment unavailable, record Task 8 as blocked rather than
  claiming the scaffold is platform-valid.
- [ ] Keep only platform-proven no-op diff marker shape, then fix golden bytes.

### Task 8.6: Destination duplicate and material preflight RED/GREEN

- [ ] Add RED cases:
  - exact same decorator duplicate for all four observed kinds and RU/EN tokens;
  - Before+After allowed with distinct names;
  - exhaustive observed Before/After/Around/ModificationAndControl x requested
    Before/After/ModificationAndControl matrix from §10.5;
  - generated Procedure/Function duplicate case-insensitively;
  - destination Form.xml generated-handler binding with no BSL definition;
    complete absence succeeds, duplicate/case variant collides, and incomplete
    destination binding material fails closed;
  - comment/string/date/delete decoys ignored;
  - conditional possible duplicate and malformed/limit parse fail closed;
  - existing/absent module watched material and stale read;
  - missing/duplicate/empty NamePrefix and missing/unknown ScriptVariant;
  - source/destination catalog parser reused exactly once per digest.
- [ ] Run CFE material/preflight tests; expected RED.
- [ ] Implement complete preflight using Task 5/6 projections; no regex.
- [ ] Re-run Task 5/6 and CFE tests expected GREEN.

### Task 8.7: Digests, discovery issuance and pure downstream algebra seams

- [ ] Add RED digest tests from §11, including fixed byte-vector fixtures for
  arguments, grant scope and execution plan.
- [ ] Add recording discovery fakes:
  - request analysis identity reaches each prepared seed;
  - watches dedupe while proposal rows remain distinct;
  - source method/adoption/form blockers keep report and prevent partial issue;
  - issuer sees exact sorted atomic plan/grant rows;
  - direct and discovery over identical capture have value-equal canonical
    plans and equal digests; omitted versus explicit matching assertions do too;
  - ExtensionRequired never reaches issuer as eligible;
  - Extension analysis proposal remains in report as Unknown with exact blocker;
    an Extension-only request calls issuer zero times and a mixed request never
    sends a row for that proposal;
  - its report contains exactly the §5.4 `DiscoveryPreflight` tuple, canonical
    nonempty sorted/deduplicated/chunked proposal-only affects, no evidence,
    details or retry guidance, and no `source_readiness` relabeling; malformed
    provider/code/state/outcome/coverage/severity/affects/retry/details/evidence
    combinations fail `Check::validate`.
- [ ] Assert grant scope contains exact allowed file + stable topology-derived
  parent creation scope, while execution digest additionally contains current
  `directories_to_create`, every captured parent state and all six source ranges
  + line ending.
- [ ] Assert grant scope contains Base/Extension flavor tags, Own/adopted UUID
  chain, analysis Ordinary and destination generated-name-unbound semantic
  proofs; both exact Form.xml material identities enter execution/baseline.
- [ ] Add same-artifact batch cases: Before+After same source succeeds;
  generated-name collision, duplicate annotation and ChangeAndValidate/hook
  conflict mark both proposals and issue no partial grant vector.
- [ ] Add pure grant/effect algebra RED tests, with no store or receipt lease:
  - two immutable grants A/B over baseline value S0; applying the supplied pure
    post-state transform for A yields S1 without rewriting either grant; a fresh
    B execution plan over S1 still matches B scope and has zero already-created
    parent effects;
  - `receipt_advance_eligible(outcome, plan, manifest_diff)` is true only for
    exact expected `Committed + VerifiedDurable + VerifiedClean` with every
    staging identity Removed/ConsumedByTarget, no unexpected/possible/detached
    rows and matching manifest; false for durability Unknown, NoChange,
    Residue, cleanup Unknown and Uncertain;
  - recording fakes prove Task 8 exposes the artifact-held plan/outcome/post-
    snapshot seam but has no receipt-store/lease implementation or production
    revision transition.
- [ ] In the back-propagated Task 9 plan, place persistent store/revision and
  current-receipt lease REDs; in Task 10 place production artifact->receipt
  order, advance/revoke, reconciliation and `discovery_receipts` integration
  REDs. Those suites are not a Task 8 final gate.
- [ ] Run discovery/determinism/grant-and-effect algebra tests; expected RED.
- [ ] Implement three encoders, resolved issuance rows, pure eligibility algebra
  and recording seams plus Task 9/10 contract back-propagation. Do not implement
  receipt persistence, receipt lease or guard policy in Task 8.
- [ ] Re-run expected GREEN.

### Task 8.8: HandlerInvocation, dry-run and atomic apply RED/GREEN

- [ ] Add application fake RED tests:
  - Preview prepare/capture/resolve once and no lease; Applied parses/prepares
    once, opens workspace/destination root, acquires artifact lease once,
    refreshes topology from the typed request once, then authoritative capture/
    resolve once and one final precondition recapture;
  - changed refreshed physical destination/locus returns
    `cfe_artifact_scope_changed` before material capture; unrelated mapping-
    digest change with the same physical root/locus updates the seed under the
    same held key;
  - handler and Task 8 post-mutation recording seam receive same plan/witness
    addresses and both digests; the future receipt insertion seam is abstract and
    no receipt store/lease/revision service is instantiated;
  - receipt-free off/observe/warn Applied calls still require artifact lease;
  - same/different process, Present/Absent, unrelated source-map edit and two
    source aliases inside one workspace preserve universe; path/case/bind views
    of the same whole physical workspace prove the same physical control root,
    same root-wide collision key, same opened artifact inode/FileId and one Busy.
    Different `UNICA_CACHE_DIR` cannot bypass Busy; process registry uses
    physical control identity + collision key, never semantic locus/key;
  - macOS native two-process suite runs both qualified APFS case-sensitive and
    case-insensitive rows. For Present and Absent, byte-distinct NFC/NFD semantic
    loci must open the same root-wide lock inode and one contender is Busy. A
    case-only pair is also Busy in both rows: in the case-sensitive row this is
    intentional conservative over-lock. The fixture records that hashing the two
    semantic loci would produce different bytes and proves those bytes are not
    used as lock filenames. Unknown/contradictory `fpathconf` case capability
    material fails before capture;
  - Linux W1/W2 internally bind the same destination root/locus: both fail exact
    mount-boundary reason before capture; equal semantic or collision-key bytes
    are asserted insufficient and never cited as actual-inode proof;
  - busy/failed/mount-boundary/unqualified-backend lease causes zero capture/
    handler/source write; any control directories/lock created before the failure
    appear only in bounded `ControlPlaneLeaseObservationV1`, never mutation effects;
  - raw args mutated after resolve cannot alter target/output;
  - CFE missing plan and non-CFE unexpected plan reject;
  - CFE dry-run never reaches generic placeholder and writes/emits nothing.
- [ ] Add native/atomic RED tests:
  - type-level constructors reject NoChange with effects, ConsumedByTarget,
    nonclean staging or absent Unmodified/RolledBackDurably proof; reject
    Committed without one target/durability state; definite Committed accepts
    every cleanup x durability combination without changing target certainty;
  - Absent hard-link lifecycle seam: temp identity T, target link T, removed temp
    name -> ConsumedByTarget clean; an extra temp name for T -> Residue. Present
    rename T -> same clean state. Relocated target T with absent temp remains
    clean but detached/non-advancing;
  - exact present/absent precondition success;
  - analysis/destination/config/descriptor/module/topology change -> zero write;
  - exact missing parent suffix creation; no unplanned parent and no
    symlink/reparse/case traversal;
  - failure after each created component rolls back only own empty directories
    in reverse; after every temp unlink/rmdir inject retained-parent namespace-
    sync failure and forced crash. Only complete transcript yields
    RolledBackDurably NoChange; identity/nonempty/remove/sync failure is
    Uncertain with known/possible effects;
  - Absent no-replace loses safely to external creator; unsupported platform
    primitive fails closed;
  - qualified Linux ext4/XFS and both macOS APFS case-mode rows run exact flock, descriptor-
    relative no-replace/replace, staging, durability, two-process and crash
    suites; NFS/CIFS/FUSE/overlay/tmpfs/network/unknown rows fail before source
    mkdir/temp even when one syscall probe succeeds;
  - native Windows backend opens workspace by trusted absolute path once, then
    uses retained root-relative handles for every destination component, parent,
    temp and target; junction/rename swaps at every boundary cannot redirect the
    write; handle volume/device queries reject remote/SMB/virtual/read-only and
    non-NTFS/disagreeing serials before source mutation; qualified NTFS row runs LockFileEx/FileRenameInfoEx,
    DELETE-access retained-handle `FileDispositionInfo` rollback,
    `FlushFileBuffers` on temp/installed target and flags-zero
    `NtFlushBuffersFileEx` on each retained receiving directory after install,
    mkdir, staging unlink and rollback rmdir. Inject failure and forced crash at
    every one of those Windows flush seams; retained-parent live absence plus
    identity equality mints the structural clean lifecycle, while only the exact
    matched transcript mints target durability or RolledBackDurably. A
    unallowlisted build/filesystem, missing ntdll binding/export or unavailable
    namespace durability proof fails before source mutation;
    path-based destination `CreateFileW`, `MoveFileExW`, `CreateDirectoryW`,
    `DeleteFileW` and `RemoveDirectoryW`
    use is statically rejected;
  - move the original retained parent itself after final walk: definite commit
    returns non-advancing Committed with opaque detached volume/object/parent
    identity and cleanup determined only by the staging proof
    (VerifiedClean/Residue/Unknown); unknown completion returns Uncertain;
    neither reports Created/UpdatedFile at the intended path;
  - Present completed external edit before final check rejects; cooperative
    same-artifact writers serialize; no test claims arbitrary-writer CAS;
  - pre-commit failure preserves the original target; it returns NoChange only
    after durable rollback, otherwise Uncertain with source residue/possibility;
    after digest is verified;
  - inject failure after every mkdir/write/data-flush/link/unlink/rename/target-
    flush/parent-sync/rollback-unlink/rollback-rmdir/rollback-parent-sync/
    read-back: handler always returns exact NoChange/Committed/Uncertain state,
    expected/unexpected/possible effects, staging lifecycle, NoChange rollback
    proof and durability even when adapter `ok=false`; committed target + live
    absent staging + failed post-install parent sync is
    Committed+VerifiedClean+durability Unknown; committed link +
    failed temp unlink is definite target+Residue and both identities;
  - first call may create control directories/lock then fail final material
    precondition as NoChange+Unmodified with empty source effects and separate
    bounded control initialization telemetry;
  - handler writes exactly allowed effects and returns typed file/directories;
  - `cfe.rs` contains no output-affecting raw helper calls.
- [ ] Extract fixed-workspace-root persistent-inode/universe-key lease + closed platform atomic
  primitives, implement HandlerInvocation/witness lifetime, typed native
  preview/apply and delete legacy raw CFE resolver path. Runtime-job
  `MoveFileExW` may remain for runtime jobs but cannot be the CFE backend.
- [ ] Re-run application/native/runtime-job tests expected GREEN.

### Task 8.9: Parity, safety and documentation closure

- [ ] Replace orphan parity setup with mapped analysis + exact adopted
  destination fixtures.
- [ ] Run valid registered Russian Before/After reference/native parity.
- [ ] Keep source-bound params/English/Modification native golden + real compile
  proof. Record the intentional generated-name divergence from the old donor;
  donor script is not modified to pretend unsupported cases are valid.
- [ ] Add native rejection cases for arbitrary Context/module kind, implicit
  borrow, form event annotation, duplicate and synthetic zero-arg signature.
- [ ] Run twice under source/provider/file/watch permutation and different cwd;
  all digests/output remain stable.
- [ ] Synchronize final spec/ADR/skill/product assertions in the implementation
  commit. Skill says `/cfe-borrow` only for actual descriptor absence, directs
  Own/wrong UUID/inconclusive users to inspect/repair, explains complete Form
  lookups on both analysis and destination sides under one accepted Task 5B
  registry version, without copying its internal rows. It also explains:
  - declared Configuration/Extension labels are insufficient without captured
    BaseConfiguration + Own / ExtensionConfiguration + Adopted proof;
  - omitted matching assertions disappear from the canonical resolved plan;
  - artifact busy behavior, fixed cache-independent descriptor-relative control
    root with no workspace key, whole-workspace mount universe, physical
    control/lock-object proof and rejection of internal shared-destination bind;
  - parent/detached effects, Removed/Consumed staging, durable rollback,
    VerifiedClean/Residue/Unknown and VerifiedDurable/Unknown consequences even
    when adapter presentation reports failure; NoChange excludes control setup;
  - exact qualified Unix/Windows backend matrix and fail-before-source-write
    behavior for network/FUSE/unknown/unqualified tuples;
  - Windows CFE apply uses one workspace open and handle-relative destination-
    root/parent/temp/target operations; path-based destination reopen/
    `MoveFileExW` is forbidden, and an unsupported v1 tuple fails before source
    mutation;
  - Present replacement is cooperative serialization, not portable CAS against
    arbitrary external writers.

### Task 8 final verification

- [ ] Run:

```text
cargo test --locked -p unica-coder cfe_method_patch -- --nocapture
cargo test --locked -p unica-coder source_snapshot -- --nocapture
cargo test --locked -p unica-coder infrastructure::discovery::bsl -- --nocapture
cargo test --locked -p unica-coder application::discovery -- --nocapture
cargo test --locked -p unica-coder mutation_effects -- --nocapture
python3 -m unittest tests.ci.test_unica_mcp_script_parity -v
python3 tests/ci/test_product_contracts.py
cargo test --locked -p unica-coder
cargo fmt --all -- --check
cargo clippy --locked -p unica-coder --all-targets -- -D warnings
git diff --check
```

- [ ] Run/record the real 1C validation command and exact fixture identities for
  RU/EN ChangeAndValidate Procedure/Function. A missing runtime is a blocker.
- [ ] `git grep` proves no CFE raw parser/path/context/default and no second
  Configuration/BSL parser.
- [ ] `git grep` proves no `BaseMetadataIdentity`, no CFE Extension-analysis
  acceptance, no stale `cfe_destination_borrow_required`, and no three-kind-only
  destination annotation scan.
- [ ] `git grep` proves no public discovery registration, receipt persistence or
  guard mode was added in Task 8.
- [ ] `git grep` and product tests prove Task 8 has no Form definition/item/
  event/callType/Action registry/table/parser; it imports one accepted Task 5B
  registry version and complete lookup only.
- [ ] `git grep` proves no CFE artifact/future-receipt control root consumes
  `WorkspaceContext.cache_root`, `UNICA_CACHE_DIR`, cwd, PID or caller-selected
  cache configuration; no `<workspaceKey>` layer exists; source-set name and
  mapping digest do not enter `ArtifactMutationSemanticKey` or
  `FilesystemArtifactCollisionKey`; canonical locus, case-folded locus and
  NFC/NFD locus bytes do not enter the root-wide collision digest/lock filename;
  all namespaces derive from the fixed `WorkspaceDiscoveryControlRootV1`.
- [ ] `git grep` proves the Windows CFE writer contains no path-based
  destination `CreateFileW`, `MoveFileExW`, checked-then-`CreateDirectoryW`,
  `DeleteFileW` or `RemoveDirectoryW` backend after the one workspace open. A separately
  scoped runtime-job use does not satisfy or invalidate this check.
- [ ] Run native same/two-process Present+Absent tests with different cache env,
  mapping edits and source aliases. Whole-workspace path/case/Unicode/bind aliases
  record same physical control root, root-wide collision key and opened lock
  inode/FileId plus one Busy; W1/W2 with shared internally bound destination both
  reject before capture. On both APFS case-mode rows run NFC/NFD and case-only
  semantic loci for Present and Absent; differing semantic hashes still share the
  inode. Do not cite equal semantic/collision key bytes as inode proof.
- [ ] Run all misdeclared Configuration-flavor, non-Own analysis root/Form and
  accepted destination Form Bound/Unbound/incomplete/version fixtures, including
  tracked base-with-compat and extension-without-compat/KeepMapping. Run the
  authoritative Task 5B registry matrix/lexical suite separately; Task 8 owns no
  duplicate fixtures/table.
- [ ] Run exhaustive filesystem failure injection and record the exact typed
  NoChange/Committed/Uncertain outcome, source effects, staging lifecycle,
  NoChange restoration proof, cleanup and durability at every mkdir/write/data-
  flush/install/staging-remove/target-flush/parent-sync/rollback-remove/
  rollback-sync/read-back seam, including adapter failure and moved parent.
- [ ] Run the focused process lock, no-replace/replace, staging, parent rollback
  and forced-crash/restart durability suite on every exact enabled Linux ext4/
  XFS, both macOS APFS case-mode and Windows NTFS tuples. A skipped/unrecorded tuple remains
  disabled and blocks that platform claim; network/FUSE/unknown rejection runs.
- [ ] Record fixed Task 4 empty-watch hashes and all three CFE digest fixtures.
- [ ] Confirm no persistent receipt store/revision, receipt lease, guard policy
  or production `discovery_receipts` integration test was implemented in Task 8;
  their exact RED/GREEN ownership is recorded in updated Tasks 9/10.
- [ ] Suggested commit slices:
  - `spec: уточнить source-bound cfe receipt scope`;
  - `feat: добавить watched mutation targets`;
  - `refactor: унифицировать source-bound cfe resolver`;
  - `refactor: передавать resolved mutation plan в native handler`.

---

## 16. Required acceptance matrix

1. Common/owner/form ModulePath maps to one canonical analysis target and one
   destination artifact through the shared registry.
2. Prepare derives only configured identities, lexical artifacts, parent paths,
   watch, destination-root candidate and canonical artifact locus; panic fakes
   prove zero snapshot/material/filesystem reads. Physical identity/key is
   derived later from retained handles, never invented in prepare.
3. CFE analysis is exact Configuration + PlatformXml. Extension analysis stays
   visible for general Explore but its mutation proposal is Unknown/ineligible,
   is never sent to issuer and has exactly one closed §5.4 application
   `DiscoveryPreflight/mutation_preflight` check per proposal-only canonical
   affected chunk with reason `cfe_analysis_configuration_required`; no
   `source_readiness`, evidence, detail or retry projection exists, and its
   wrapper UUID never substitutes for base Configuration `MetadataIdentity`.
4. Unknown module kind, extra/empty/traversal/separator/control or case-only
   user spelling ambiguity fails before filesystem write.
5. SourceSet omitted first filters to Configuration + PlatformXml and selects
   only one such analysis source, ignoring adjacent Extensions for ambiguity;
   an explicit raw selector is an assertion against discovery/receipt authority.
6. Context/IsFunction/SourceSet absence derives exact source facts; after a
   matching explicit assertion is validated, its presence is erased. Omitted
   and explicit-matching inputs construct value-equal
   `ResolvedCfeMethodPatchCore` and full immutable plans, not merely equal
   digests; assertion-status diagnostics stay outside grant/plan/handler state.
7. Exactly one unconditional exact-spelling analysis method is required;
   definition/declaration/name/parameter/body/terminator ranges plus declaration
   line ending are retained in the immutable plan and validated against one
   verified module byte slice.
8. Definition/signature/body digests use the exact domain/field order in §4.5;
   forged range, line-ending, slice or parser-cache identity cannot construct a
   plan.
9. Before/After copy exact parameter list and source-derived async/context;
   Function cannot use Before/After.
10. ModificationAndControl clones exact bounded definition/body, replaces only
   declaration name and uses a real-platform-proven no-op diff scaffold.
11. Russian/English destination variants render exact independent golden bytes;
   Missing/Unknown ScriptVariant never falls back to Russian.
12. Shared `KnownScriptVariant`, PlatformConfigurationCatalogV1, descriptor,
   accepted Task 5B neutral `PlatformFormRegistryVersion`/complete lookup and
   BSL parsers are reused; Task 8 defines no Form registry/table/parser.
13. Declared source kind is never flavor authority: the captured analysis
    catalog is exactly BaseConfiguration, every contributing analysis root/form
    descriptor is Own, the captured destination catalog is exactly
    ExtensionConfiguration, and destination root/optional form are exactly
    Adopted with extended UUIDs equal to the corresponding Own analysis UUIDs.
    Flavor is absent belonging+purpose for Base or exact Adopted+supported
    purpose for Extension; compatibility/KeepMapping are optional validated
    non-discriminators on both. Partial/mixed/misdeclared flavors and wrapper/
    local UUID decoys fail before UUID comparison.
14. The membership lattice remains exact: only absent descriptor is
    RequiresBorrow/`destination_borrow_required`; present Own is Unknown/
    `destination_object_not_adopted`; wrong adopted UUID is Unknown/
    `destination_extended_object_mismatch`; malformed/gapped is Unknown with
    exact inconclusive/provider reason. Unknown wins over an absent sibling.
15. `ExtensionRequired` remains visible advice but cannot produce CFE receipt
    eligibility or a resolved issuance plan.
16. A Form plan requires two independent `Unbound` lookups from accepted
    complete Task 5B projections under one exact registry version: source method
    and generated destination method. `Bound` produces wrong-mechanism/collision
    even without BSL definition; incomplete/version mismatch is inconclusive.
    All item/event/callType/Action compatibility, cardinality, lexical and limit
    authority remains solely in Task 5B's freshly accepted exhaustive suite.
17. Present/absent destination module and every parent component are captured
    atomically; absence has typed tombstones and an unregistered physical decoy
    or unsafe parent is never opened.
18. Empty-watch Task 4 capture and fingerprints are byte-identical; nonempty
    parent state tags have fixed independent fixtures.
19. A first patch creates only the planned absent parent suffix ancestor-first;
    precommit failure removes only this call's empty identity-matching
    directories/temp names in reverse and runs the exact retained-parent
    namespace durability primitive after every removal. Only Unmodified or a
    complete RolledBackDurably transcript may return NoChange; removal/sync
    uncertainty is typed Uncertain.
20. Before/After names use deterministic destination-variant suffixes and may
    coexist. Destination parsing observes RU/EN Before, After, Around/Вместо and
    ModificationAndControl and applies the exhaustive 4x3 matrix; comments,
    strings, dates and deleted blocks do not block, conditional uncertainty does.
21. Every resource limit has N/N+1 proof with no unreported partial effect.
22. Normalized arguments, grant scope and execution plan use three exact
    domain-separated encodings with fixed fixtures. Only the resolved canonical
    core enters arguments/grant identity; optional-assertion presence can affect
    detached diagnostics but cannot affect full plan equality.
23. Grant scope contains immutable exact allowed file + topology-derived parent
    creation scope; current parent states, `directories_to_create`, destination
    fingerprint/module/output appear only in execution plan/baseline.
24. Pure grant/baseline algebra proves that supplied post-state S1 after A leaves
    immutable A/B grants unchanged and a fresh non-conflicting B plan can match
    scope with new execution material; no receipt store/revision is implemented
    in Task 8. A pair known at issuance to collide receives no grant.
25. Tool/target/interceptor/derived context+kind+async/source/destination/UUID
    chain/allowed effects/version compare as one grant tuple; no cross-product
    match.
26. Direct/discovery resolution over identical snapshot yields equal values and
    digests; only within one call is pointer identity required.
27. Dry-run performs full source/adoption/form/duplicate/render validation and
    causes no source/control-plane file, directory, lease, event, cache or
    receipt mutation.
28. Every applied CFE call in off/observe/warn/deny opens the workspace once,
    proves one supported same-mount/volume whole-workspace universe and qualified
    backend, resolves destination handle-relative, derives its separate semantic
    physical-destination/canonical-locus key, then acquires the physical-control
    universe + root-wide filesystem-collision process/OS artifact lease,
    and refreshes topology before capture. It keeps the actual inode/capability through
    handler, typed effects and Task 8 post-snapshot seam. The control path is
    exactly `.build/unica/project-discovery/control-v1` with no workspace key;
    cache root, mapping digest, source alias, cwd bytes and PID cannot split it.
    Whole-workspace aliases prove one control root/collision lock inode+Busy; an
    internal destination bind is rejected. APFS case-sensitive/insensitive
    NFC/NFD and case-only Present/Absent aliases share that inode even when
    semantic hashes differ. Equal semantic or collision-key bytes alone are not
    actual-lock proof.
29. Task 8 proves only pure/recording downstream seams. Updated Tasks 9/10 own
    persistent receipt store/revision, current receipt lease, production
    artifact->receipt acquisition/reverse release, reconciliation and the
    `discovery_receipts` integration suite; none is faked as production-green in
    Task 8.
30. The Task 8 applied path performs one final same-watch recapture, exact plan/
    physical-scope comparison, parent/target precondition and after-digest
    verification without resolving/rerendering another plan. The recording seam
    permits Task 10 to insert its current receipt lease before that recapture
    without changing the plan or artifact capability.
31. Absent installation is atomic no-replace and never overwrites an external
    winner; platforms without a qualified lock/install/durability tuple fail
    before source write. Linux ext4/XFS, both macOS APFS case-mode and Windows NTFS rows use the
    exact §13.3 primitives and require native race/failure/crash gates; NFS/CIFS/
    FUSE/overlay/tmpfs/network/unknown never fall through. On Windows only
    the workspace root may be path-opened once; every destination-root component,
    parent/temp/target operation and install is relative to retained,
    reparse-checked handles. Unallowlisted v1 tuples fail before source mutation;
    destination `CreateFileW`, `MoveFileExW` and checked-path `CreateDirectoryW`
    plus path `DeleteFileW`/`RemoveDirectoryW` are forbidden. Windows rollback
    uses DELETE-access retained-handle `FileDispositionInfo`. The NTFS
    durability row names `FlushFileBuffers` for file
    data and flags-zero `NtFlushBuffersFileEx` on the retained receiving
    directory for install/mkdir/unlink/rmdir namespace changes; no substitute or
    success-only probe can mint authority.
32. Present replacement is atomic and serializable for cooperating Unica
    writers. Documentation/tests explicitly do not claim portable content-CAS
    against a non-cooperating writer in the final check/replace window.
33. Every applied handler returns a typed NoChange/Committed/Uncertain outcome
    even when `AdapterOutcome.ok=false`. It retains expected/unexpected/possible
    CreatedDirectory, CreatedFile/UpdatedFile, OwnedStagingFile and detached/
    relocated physical-object effects plus exact name-based staging cleanup and
    independent durability. VerifiedClean accounts every staging identity once
    as live retained-parent-name-absent Removed or ConsumedByTarget; installed identity may survive at target.
    NoChange has zero source effects and mandatory Unmodified/RolledBackDurably
    proof while control initialization is separate. Committed always means a definite target effect and can carry any cleanup/durability;
    Uncertain means the target is not definitely committed and NoChange's clean,
    durably restored invariant is unavailable. Only exact expected
    VerifiedDurable+VerifiedClean Committed effects equal to post-manifest
    satisfy pure advance; durability Unknown, unexpected, possible, detached,
    Residue/cleanup Unknown or Uncertain revoke. Display is never authority.
34. Handler cannot reparse raw args, select source/path/context/kind, parse XML
    or rerender; its only directory creation path consumes the exact typed
    parent-chain plan under the lease.
35. Valid adopted Russian Before/After retain donor semantic structure but use
    the corrected collision-free generated names; English and source-bound
    clone scenarios have independent golden/compile proof. Historical donor
    byte parity is explicitly not asserted for generated names/signatures.
36. Task 8 adds no public discovery tool, receipt store/receipt lease, guard
    policy, observation service, tool/package registration, release or Form.xml
    mutation; prerequisite schema/observability text and atomic-backend support
    matrix/product CI are updated as contract back-propagation. The
    separate mandatory artifact mutation lease is in scope. Its final gate does
    not run or claim production `discovery_receipts`; Tasks 9/10 own that proof.
37. Production Task 8 code remains STOP until Task 5A has an accepted committed
    SHA and Task 5B is freshly accepted on it with no P0/P1, strict MDClasses
    flavor/Own gate, one neutral Form registry and closed EventSubscription
    compatibility matrix GREEN.

---

## 17. Hard STOP conditions

Stop implementation and show the owner if any condition is true:

- Task 4/5/6/7 back-propagations or the active spec/ADR/skill/historical Task
  9/10 corrections in §§2/15.0 are not GREEN;
- Task 5A has no accepted committed SHA, Task 5B is not freshly accepted on it
  with zero P0/P1, or strict MDClasses flavor/Own, neutral Form registry and
  closed EventSubscription compatibility gates are not GREEN;
- raw Context/IsFunction absence still becomes synthetic AtServer/Procedure;
- raw SourceSet can override discovery/receipt analysis authority;
- an Extension analysis source is accepted for CFE mutation/receipt, its wrapper
  UUID is compared as base identity, or a second `BaseMetadataIdentity` v1 model
  is introduced instead of requiring Configuration analysis;
- a declared Configuration/Extension topology label is trusted without exact
  captured BaseConfiguration/ExtensionConfiguration flavor proof, any analysis
  root/Form UUID contributor is not Own, or a partial/mixed flavor catalog is
  allowed to reach UUID comparison;
- ConfigurationExtensionCompatibilityMode or KeepMapping is required as an
  extension-only discriminator, a tracked base-with-compat fixture is rejected,
  or an Adopted+supported-purpose extension without those optionals is rejected;
- flavor fields are accepted outside exact MDClasses namespace, invalid flavor
  still emits a CFE membership/eligible projection, or UUID joining precedes the
  flavor gate;
- `cfe_analysis_configuration_required` is emitted as `source_readiness`, by an
  evidence provider, with candidate/non-proposal affects, or with any tuple
  value differing from the closed §5.4 application mutation-preflight contract;
- source target kind/context/signature/body comes from arguments, display text,
  provider prose or an unverified cache rather than exact snapshot bytes;
- source method is missing/duplicate/conditional/malformed yet rendering
  proceeds, or case-fold-only spelling is silently substituted;
- any definition/declaration/name/parameter/body/terminator range or declaration
  line ending is dropped from the immutable plan/digest, recomputed by search,
  or accepted without exact slice validation;
- Before/After emits empty parameters for a parameterized source method;
- ModificationAndControl emits an empty TODO method, fails to preserve source
  body, uses substring splice or lacks real RU/EN platform validation;
- Task 8 defines or copies any Form definition/item/event/callType/Action row,
  parser or lexical/cardinality rule instead of consuming the accepted Task 5B
  versioned complete lookup;
- only the analysis Form lookup is checked, destination BSL silence is treated
  as generated-name absence, a Task 5B Bound row is ignored, or incomplete/
  mismatched registry versions become Unbound;
- registration/name alone is treated as borrowing, destination is not Adopted,
  UUID chain mismatches, or Task 8 implicitly calls/implements cfe.borrow;
- descriptor absence, present Own, adopted UUID mismatch and malformed/gapped
  membership are collapsed into one borrow-required result, or Unknown fails to
  take precedence over an absent sibling;
- `extension_required` becomes CFE receipt-eligible;
- CFE defines another ScriptVariant, metadata/module registry, Configuration
  parser, descriptor parser, form parser or BSL lexer;
- absent module is inferred from NotInManifest without same-capture tombstone;
- missing parent components are not captured as a typed chain, a first patch is
  rejected merely because exact `<Name>/Ext` is absent, or parent creation lacks
  reverse identity-checked rollback and typed partial effects;
- unregistered physical decoy is opened/promoted by SnapshotWatch;
- nonempty watch default silently ignores watches or empty-watch hash changes;
- watch/proposal ID, absolute path, raw alias, receipt ID, timestamp or
  workspaceEpoch enters a canonical CFE digest;
- prepared optional-assertion presence, alias spelling or assertion-status flags
  survive in `ResolvedCfeMethodPatchCore`, the immutable plan, a grant or the
  handler; omitted and explicit-matching inputs are not value-equal plans;
- grant-scope digest includes mutable source/composite fingerprint,
  current absent parent suffix, destination module before/after or rendered
  output;
- rolling A requires rewriting B's grant to let B run, or B skips fresh current
  resolution/execution precondition after baseline advance;
- fields from different grants can satisfy one call;
- mutually conflicting same-artifact plans can be issued together;
- Around/Вместо is ignored by destination duplicate preflight or the exhaustive
  observed 4x3 conflict matrix is not GREEN;
- future guard, handler and post-mutation seam receive different current plan or
  artifact-witness objects/digests;
- handler parses raw arguments/Configuration, rerenders or chooses another path;
- CFE dry-run returns generic placeholder or skips source/adoption/duplicate
  checks;
- any applied CFE path reaches authoritative capture/handler without first
  opening workspace/control/destination handles, proving same-mount/volume and
  qualified backend, acquiring the artifact lease and
  refreshing current topology under it; refresh can switch physical root/locus
  without reacquiring; future Task 10 acquires receipt before artifact, releases
  artifact before receipt transition, or reconciles another receipt while
  either is held;
- an artifact/receipt lock or receipt record is rooted in
  `WorkspaceContext.cache_root`, `UNICA_CACHE_DIR`, cwd, PID or another
  process-selectable location, allowing two processes on one canonical
  workspace to create disjoint control planes;
- the fixed control root contains a path-derived `<workspaceKey>` subdirectory,
  semantic artifact key includes source-set name/map digest/lexical alias
  instead of physical destination root + canonical locus, the actual v1 lock
  filename includes canonical/case-folded/NFC/NFD locus bytes instead of the
  root-wide physical-destination collision digest, or process registry omits the
  physical-control-root/collision-key universe;
- semantic NFC/NFD/case aliases can select distinct v1 lock files, equal semantic
  or collision-key bytes are cited as proof of a shared lock inode, APFS
  case-sensitive/insensitive Present+Absent alias suites do not prove one
  physical control/lock object and Busy, a future narrower key ships without a
  mixed-version migration/dual-lock ADR, or an internal destination/control
  bind/mount boundary is accepted;
- applied path skips the final same-watch precondition recapture after optional
  receipt acquisition;
- Absent target uses an overwriting/check-then-rename install, or Present replace
  is described/tested as portable CAS against arbitrary external writers;
- Windows CFE containment path-opens/reopens the destination root, uses path-
  based `MoveFileExW`/`CreateDirectoryW`/`DeleteFileW`/`RemoveDirectoryW`, or
  cannot bind every destination component/parent/temp/target/install/rollback
  delete after the one workspace open to retained reparse-checked handles; an
  unallowlisted/unsupported backend mutates source before failing closed;
- a claimed Windows row omits exact handle-based NTFS/volume-serial and
  `FileFsDeviceInformation` mounted+writable non-remote/non-virtual proof, or
  remote/SMB/ReFS/FAT/read-only/virtual material reaches the source writer;
- a claimed Windows tuple omits exact DELETE-access retained-handle
  `FileDispositionInfo` rollback, file `FlushFileBuffers` plus flags-zero
  retained-directory `NtFlushBuffersFileEx` install/mkdir/delete transcripts,
  accepts an unavailable ntdll binding/export, substitutes a
  data-only/no-sync/volume/path-reopened flush, mints durability or
  RolledBackDurably from syscall success without the matching forced-crash
  evidence, or mints structural clean without retained-parent name and identity
  observation;
- Linux/macOS use generic `cfg(unix)`, syscall-presence or filesystem-name
  fallback; NFS/CIFS/SMB/FUSE/overlay/tmpfs/network/unknown/unqualified tuple
  reaches source mutation; or any claimed ext4/XFS/APFS/NTFS tuple lacks exact
  lock/install/durability primitives and native race/failure/crash evidence;
- apply uses create_dir_all, creates an unplanned parent, follows a link/reparse
  target, writes in place with partial-file risk or writes outside exact allowed
  effects;
- an applied CFE handler returns only `AdapterOutcome`, returns `Err` after the
  first mutation, loses known/possible effects or owned staging residue, maps an
  unknown install/cleanup result to NoChange, cannot construct definite
  Committed with Residue/Unknown or durability Unknown, constructs Committed
  without a definite target/durability state,
  reports a moved retained object only at the intended path, loses its physical
  identity, requires a consumed staging object to cease existing, marks clean
  while an owned staging name remains/unaccounted or its retained-parent name
  state is unqueryable, mints `VerifiedDurable` without every required committed
  staging-namespace sync, or Task 10 advances anything
  other than exact expected VerifiedDurable+VerifiedClean Committed effects;
- NoChange is constructed after a source name/object was created without a
  complete Removed-only `RolledBackDurably` transcript, rollback unlink/rmdir/
  namespace-sync failure maps to NoChange, or fixed control-plane setup is mixed
  into grant/source effects;
- Task 8 adds receipt persistence/lease/revision/policy/public discovery/tool
  package registration, or its final gate runs/claims production
  `discovery_receipts` before Tasks 9/10. Contract-only schema/observability and
  backend support-matrix back-propagation remains mandatory, not forbidden.

---

## 18. Source notes and design result

The source-bound correction follows official 1C extension behavior:

- official module-extension examples show hook declarations carrying the
  intercepted method parameters, and functions restricted to the replacement
  form in the documented hook model;
- official module-extension material also exposes Around/Вместо; even though the
  public v1 writer does not generate it, negative duplicate proof must observe
  its exclusive conflict;
- ChangeAndValidate uses Insert/Delete markers and therefore requires verified
  source method material rather than a synthetic empty body;
- managed-form event extension uses form metadata/editor hook type rather than
  ordinary module annotations;
- the language accepts Russian/English paired tokens, while Task 8 still
  preserves exact user/source identifiers, spans and paths for byte authority.

Primary references:

- <https://1c-dn.com/blog/module-extensions/>
- <https://kb.1ci.com/1C_Enterprise_Platform/Guides/Developer_Guides/1C_Enterprise_8.3.23_Developer_Guide/Chapter_4._1C_Enterprise_language/4.8._Differences_between_various_system_startup_options/4.8.1._Procedures_and_functions_execution/>
- <https://kb.1ci.com/1C_Enterprise_Platform/Guides/Developer_Guides/1C_Enterprise_8.3.23_Developer_Guide/Chapter_36._Configuration_extension/36.4._Extension_objects/36.4.3._Forms/>
- <https://learn.microsoft.com/en-us/windows/win32/api/winternl/nf-winternl-ntcreatefile>
- <https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-setfileinformationbyhandle>
- <https://learn.microsoft.com/en-us/windows/win32/api/winbase/ns-winbase-file_rename_info>
- <https://learn.microsoft.com/en-us/windows/win32/api/winbase/ns-winbase-file_disposition_info>
- <https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-flushfilebuffers>
- <https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-getvolumeinformationbyhandlew>
- <https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/ntifs/nf-ntifs-ntflushbuffersfileex>
- <https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/ntifs/nf-ntifs-ntqueryvolumeinformationfile>
- <https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-fscc/616b66d5-b335-4e1c-8f87-b4a55e8d3e4a>
- <https://developer.apple.com/documentation/foundation/urlresourcevalues/volumesupportsexclusiverenaming>
- <https://developer.apple.com/library/archive/documentation/System/Conceptual/ManPages_iPhoneOS/man2/pathconf.2.html>
- <https://developer.apple.com/library/archive/documentation/FileManagement/Conceptual/APFS_Guide/FAQ/FAQ.html>
- <https://man7.org/linux/man-pages/man2/renameat.2.html>

This v5 candidate is ready for a fresh independent design review, not yet for
production implementation. Implementation remains a hard STOP until that review
finds no P0/P1 and every prerequisite back-propagation/RED gate in §15.0 is
committed and GREEN, including accepted committed Task 5A and freshly accepted
Task 5B with one neutral Form registry. Its resolved contract is one Configuration-source-bound,
two-stage semantic resolver plus one Applied topology refresh under a physical-
artifact lease, with one canonical current-call plan. It no longer
confuses a declared source label with captured Base/Extension flavor, a wrapper
UUID with Own base authority, a method name with a compilable interceptor,
registration with borrowing, descriptor absence with present Own/mismatch, an
analysis-only Form scan, a Task 8-owned Form table or destination BSL silence
with accepted complete negative proof,
three generated interceptors with the complete observed set, optional assertion
presence with canonical plan state, or a mutable execution plan with immutable
grant authority.

The control plane is exactly one fixed descriptor-relative directory below one
physical whole-workspace mount universe, without a path-hashed layer. The
semantic key is physical destination root plus canonical locus, while the actual
v1 lock uses a separate backend-qualified root-wide destination collision key.
It intentionally over-locks unrelated loci so NFC/NFD, case and absent aliases
cannot split serialization before capture. Actual authority additionally proves
the same physical control root and opened lock inode/FileId plus Busy; internal
shared-destination mounts are rejected. Linux,
macOS and Windows each use closed qualified backend tuples and named native
lock/install/durability primitives, never a network/FUSE/unknown fallback.

Every applied handler separates definite target commit, name-based staging
cleanup and durability. VerifiedClean accounts every staging identity exactly
as Removed or ConsumedByTarget, so an installed object can survive at target;
detached location remains independent. NoChange is limited to zero durably
restored source effects and requires Unmodified or RolledBackDurably, while
control initialization is separate. Parent/namespace sync failure after commit
is definite Committed with durability Unknown. Task 10 can advance only exact
expected VerifiedDurable+VerifiedClean effects. Fixture-proven flavor and the
accepted Task 5B registry prevent false negative authority. Task 8 exposes only pure grant/effect
algebra and recording seams; Task 9/10 own persistent receipts, leases, revision
and production integration. Typed parent-chain creation still makes the first
valid patch possible without broad directory mutation, and Present replacement
remains deliberately cooperative rather than an unavailable portable external-
writer CAS.
