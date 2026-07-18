# Task 8 Source-Bound Shared CFE Mutation Resolver Implementation Design v6

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
control/WAL/staging/target operations после единственного workspace-root open остаются
handle-relative; path-based
`MoveFileExW` не является допустимым CFE backend. Task 8 создаёт plumbing seam,
bounded durable artifact-writer WAL и typed committed/no-change/uncertain
effect contract. Строящийся объект живёт только под control root; source tree
меняется одним атомарным install. Task 8 не хранит receipt records и не
применяет guard policy, но задаёт обязательный durable WAL-to-receipt handoff
seam для Task 10.

**Tech Stack:** Rust; existing Task 4 source snapshots; Task 5 shared Platform
XML catalog, registration/adoption/form-binding parsers; Task 6 bounded BSL
lexer/parser; Task 7 discovery use case; `serde_json`, `sha2`, already-landed
native OS APIs; existing native adapter and parity harness. Новых parser/runtime
dependencies нет.

## Global constraints

- Source of truth: code/tests/package metadata > active spec > historical plan.
- Task 8 production code starts only after GREEN Task 4 and the accepted Task 5A
  implementation, plus
  the accepted separate immutable **Task5C-Evidence** family rooted at
  `.superpowers/sdd/task-5c-evidence-v2-design.md`
  (parser/provider/projection/assessment only; no `support.edit` mutation),
  plus the explicit prerequisites in
  `.superpowers/sdd/task-5b-v7-contract.md`,
  `.superpowers/sdd/task-6-v2-v7-addendum.md` and
  `.superpowers/sdd/task-7-v6-v7-addendum.md`. Their matching self-audits and
  independent reviews, plus Task 5B/6 production implementations, the exact
  independently implementable Task 7 prerequisite slice defined in §2.4, and the
  exact Task5C-Evidence implementation OID, must
  each be independently accepted with ledger values and no open P0/P1/P2. A
  rejected Task 5B v6 or historical Task 6-v2/Task 7-v6 base hash cannot
  satisfy this gate. The Task 9/10 v6
  **addendum designs and their independent reviews** are also contract
  prerequisites before Task 8 production code; their store/handoff
  **implementations are not** and remain downstream of the accepted Task 8
  writer. The combined unfrozen
  `.superpowers/sdd/task-5c-v2-design.md`, whole Task 5C and its mutation slice
  have zero prerequisite authority for Task 8. Task5C-Mutation is specified by
  the separate `.superpowers/sdd/task-5c-mutation-v2-addendum.md` family and
  must wait for the accepted Task 8 writer and Task 9/10 implementations.
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
  Platform XML **and** captured `ConfigurationFlavorV1::BaseConfiguration`.
  An Extension analysis source remains valid for general Explore evidence, but
  its CFE mutation proposal is Unknown, receipt-ineligible and issuer-silent
  with `cfe_analysis_configuration_required`. Never compare an extension
  wrapper descriptor `@uuid` as the base object identity and do not introduce
  an alternative `BaseMetadataIdentity` model in v1. Every analysis root/Form
  object UUID used in the adoption join comes only from the accepted
  `BaseOwnedMetadataIdentityV1` companion and is additionally proven `Own`; a
  topology label or the catalog's separate Configuration-root UUID authority
  cannot override captured XML flavor or descriptor membership.
- Destination `ScriptVariant` reuses domain `KnownScriptVariant` from the shared
  Task 5B v7 `PlatformConfigurationCatalogV1.script_variant_authority`; no `CfeScriptVariant` and no second
  Configuration.xml parser are allowed.
- Exact source method material and destination material are snapshot-bound.
  An absent destination module is authoritative only through a watched
  tombstone captured in the same initial/final scan.
- `prepare` performs zero Platform XML/BSL/snapshot/filesystem material reads.
  It may normalize arguments/configured topology, request capture of typed
  selected source roots, and derive only destination target/parent watch
  topology available without material. It never obtains or formats Form
  sidecar manifest keys. Capture owns bounded no-follow I/O and sidecar
  discovery; resolve owns verified material interpretation.
- Every applied CFE mutation under the public closed guard modes
  `off|observe|warn|deny`, whether its writer handoff is NotRequired or Required,
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
  alias, `UNICA_CACHE_DIR` or another cache override. Before authoritative
  capture, every acquired root-wide lease first recovers
  the bounded artifact-writer WAL for that collision universe. The lease
  remains held through handler, typed effects, post-snapshot, the Task 10
  receipt handoff when Task 10 supplies a validated currently leased receipt,
  and only then terminal-WAL GC.
  Dry-run acquires no lease.
- A first patch never creates a source-tree staging name or one directory at a
  time. It builds the exact missing suffix plus `Module.bsl` below the trusted
  same-mount control staging root and publishes that whole subtree with one
  atomic no-replace directory rename into the first-present source parent.
  Present replacement and Absent-with-present-parent likewise publish one
  control-staged file by a qualified cross-directory atomic rename. Direct
  target-parent staging, `create_dir_all`, link/reparse traversal and
  checked-then-create fallback are forbidden.
- FormModule methods not registered as Form bindings may use module annotations.
  A method named by any
  binding in Task 5B's accepted neutral, versioned complete Form auxiliary catalog is
  the wrong mechanism and must fail with
  `cfe_form_handler_wrong_mechanism`; incomplete or unsupported event-bearing
  Form.xml can never prove Ordinary. The destination Form.xml must independently
  prove that the generated method name is not already bound by that same
  projection.
  Task 8 does not rewrite Form.xml callType.
- Task 8 contains no Form definition/item/event/callType/Action registry,
  cardinality table or lexical policy. It imports only Task 5B's accepted
  neutral `PlatformFormBindingRegistryVersionV2`, complete typed V2 auxiliary catalog and lookup
  result. Task 5B, Task 6 and Task 8 are sibling consumers of Task 5A/domain's
  one `ArtifactRef` + `ArtifactIdentityBytesV1` canonical method-identity seam;
  none imports that authority from another sibling.
  Task 5B v7 must be accepted with no open P0/P1/P2 and its registry's exhaustive
  fixtures plus lexical and outer XML capture N/N+1 tests GREEN before Task 8
  code starts; no separate binding-count N+1 exists.
- One plan authorizes one source method, one interceptor, one destination, one
  generated method and one destination BSL artifact. No cross-product match.
- Dry-run performs the same selection, capture, adoption, source-method,
  duplicate and render work as apply, but writes nothing and touches no receipt.
- Task 8 does not register `unica.project.discover`, persist/lease a receipt,
  apply rollout guard policy, or mutate public tool/package registration. The
  prerequisite backend support matrix/product CI is a required package-contract
  back-propagation. Its artifact mutation
  lease is a distinct correctness primitive and is mandatory in every applied
  rollout mode.
- `AdapterOutcome` is presentation only. Every applied CFE handler returns a
  typed mutation outcome even when `ok=false`. `NoChange` means zero persistent
  source-tree effects and exact `Unmodified` proof; control-only staging is
  accounted independently as VerifiedClean/Residue/Unknown. Only clean
  NoChange may tombstone its WAL; Residue/Unknown remains a blocking terminal
  and revokes rather than pretending install uncertainty. `Committed` contains exactly one
  `DefiniteTargetCommitV1`, even when a fresh location/content/metadata query is
  Unknown, plus independent cleanup and durability. `Uncertain` is reserved for
  unknown **install completion**, never for a definite install with an unknown
  post-observation. Detached/relocated objects retain stable identity and never
  masquerade as the intended path. No post-commit, durability, metadata,
  cleanup or recovery error may collapse to a string.
- Present replacement is only cooperatively serializable. Absent installation
  is atomic no-replace. Staging is below the fixed control root, on the same
  qualified mount/volume, and installation is a cross-directory rename into a
  retained source parent. On Windows the absolute trusted path is used at most
  once to open the workspace root; destination/control traversal, both install
  modes and every staging/install operation remain handle-relative. Path-based
  destination reopen/rename/create is a hard STOP.
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
  exact kernel/OS/filesystem tuple passes its process-lock, cross-directory file
  and directory atomic install, closed Present/Created metadata,
  staging-lifecycle, WAL recovery, metadata-durability, failure and
  crash/restart gates.
  NFS/CIFS/SMB, FUSE, ReFS, FAT/exFAT, network, nested/unidentified and any
  unqualified tuple fail `artifact_writer_backend_unsupported` before control
  staging or source mutation; syscall availability or filesystem name alone is not
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
destination ExtendedConfigurationObject UUID == analysis BaseOwnedMetadataIdentityV1.object_uuid
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

Task 5B's accepted neutral, versioned complete Form auxiliary catalog is authoritative.
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
| precommit/write | plan + artifact lease witness + descriptor-relative handles; when required, an already-held current receipt lease/baseline; then one final same-watch recapture, durable Prepared and only afterwards durable InFlight | reparsing/rerendering | WAL-backed typed install certainty/current observations + cleanup + durability/recovery outcome |
| post-mutation | same artifact lease, typed handler outcome, fresh snapshot and, when Required, the same current receipt lease retained through terminal reconciliation | event/cache/other-receipt reconciliation side effects | Task 8 records exact inputs; Task 10 later owns the current-receipt baseline-revalidated clear/advance/revoke; `AdapterOutcome` is never authority |

A recording `PanicOnMaterialRead` fake must prove `prepare` performs exactly
zero snapshot/material/filesystem reads. Moving descriptor validation into
prepare is a contract failure even if the same bytes are read later.

### 1.5 Artifact serialization and first-patch atomic publication are mandatory

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
physical whole-workspace universe regardless of handoff variant. A destination
root crossing an internal mount/bind boundary is outside that universe and is
rejected rather than pretending its workspace-local lock is shared.
The parent chain instead selects one atomic publication shape. If the target
parent exists, the writer stages one file under the control root. If a suffix is
absent, it stages the complete top-absent directory subtree, including every
nested directory and final `Module.bsl`, and publishes that subtree with one
no-replace directory rename into the retained parent of the first absent
component. Nothing is created in the source tree before this single install.
The exact metadata of every staged file/directory is synthesized from the
retained destination-parent policy and verified after publication; restrictive
control-root metadata must never leak into the source tree. These are root-cause
requirements, not atomic-file implementation details.

### 1.6 The historical file list is incomplete

Changing only `cfe_method_patch.rs`, target resolver and `cfe.rs` cannot carry a
typed plan through the current `ToolSpec + raw args + context` adapter path.
Task 8 must also change operation descriptors, application ports/composition,
native adapter/registry, source snapshots, Task 5/6 projections, discovery
issuance and dedicated CFE tool schema. Omitting those changes is a hard STOP.

### 1.7 V6 supersedes immutable v5 and closes the independent review

This v6 is a new artifact mechanically derived from immutable v5 design
SHA-256 `ed31f6d9714a6be8890202e4e8181560e195bdcdeee85277daabb2052537f3e3`.
It closes independent-review SHA-256
`dac3c14ba4e7ca64bcf19b0485e71de48286ed88861919547a97cfc304067297`.
Neither v5 file is edited, superseded in place, or accepted as an alternative
implementation path. V6 makes these three corrections one indivisible set:

1. a bounded fsynced artifact-writer WAL is durable before the first source
   namespace mutation and is recovered under every root-wide artifact lease,
   including NotRequired handoff;
2. definite install certainty is represented independently from current
   location/content/metadata observation, so a definite install plus an
   unqueryable target is always `Committed`, non-advancing and revoking; and
3. a closed metadata policy covers Present replacement and every file/directory
   created in an atomically published absent subtree. Metadata inherited from
   the restrictive control root is never allowed to escape into source.

### 1.8 Bounded artifact-writer WAL and recovery authority

The fixed control root contains two private descriptor-relative namespaces:

```text
mutation-wal/artifacts/<collision-key-digest>.a
mutation-wal/artifacts/<collision-key-digest>.b
staging/artifacts/<collision-key-digest>/<transaction-id>
```

`collision-key-digest` is the exact 64-lowercase-hex root-wide digest. A
`transaction-id` is exactly 16 OS-CSPRNG bytes, not all zero, encoded as 32 lowercase
hex characters, selected once under the empty locked bucket and persisted in
Prepared before its name can be created; PID/time/path-derived names and
unbounded collision retry are forbidden. Secrecy of the ID is not a safety
boundary.

There is exactly one two-slot WAL and one staging bucket per root-wide collision
key. Each slot is at
most `MAX_ARTIFACT_WAL_RECORD_BYTES = 65_536`; each owned staging tree is at
most the already-bounded plan and there is at most one live transaction per key.
The control directories and records are owner-only on Unix and protected by the
reviewed service SID/owner DACL on Windows. A WAL record contains only bounded
workspace-relative component vectors, opaque physical-identity digests,
backend/policy digests, transaction/correlation IDs, the chosen control-staging
basename and typed outcome state. Exact relative component bytes are private
recovery authority and never leave the protected record. It never contains
absolute paths, unrelated raw tool arguments, source method/body bytes, rendered
BSL bytes or receipt secrets. Public diagnostics expose only stable reason codes
plus domain-separated digests.

Every slot uses exact schema `unica.artifact-writer-wal.v1` and this non-JSON
byte frame:

```text
literal "unica.artifact-writer-wal.v1\0" ||
u64be(generation) || u32be(payloadLength) || canonicalPayload ||
u32be(CRC32C(u64be(generation) || u32be(payloadLength) || canonicalPayload)) ||
SHA256("unica.artifact-writer-wal.v1\0" || u64be(generation) ||
       u32be(payloadLength) || canonicalPayload)
```

`canonicalPayload` begins with the stable one-byte state tag and uses §11
length/count rules plus the exact field order declared here; debug/JSON/host-
endian encodings are forbidden. Generation is monotonically increasing `u64`;
payload and whole-frame lengths are checked before allocation, CRC32C detects a
torn frame and SHA-256 binds its authority.
`CRC32C` is reflected CRC-32C/Castagnoli: polynomial `0x82F63B78`, initial
register `0xFFFFFFFF`, final xor `0xFFFFFFFF`, no appended zero augmentation;
ASCII `123456789` yields `0xE3069283`. Its numeric result is encoded `u32be` in
the frame. Hardware/library paths must match this fixed vector and the complete
frame fixtures; host-native CRC variants are forbidden.
Before writing `Prepared`, the plan constructor encodes or checked-sizes the
maximum reachable frame for every state, including complete Terminal and
self-contained ReceiptHandoffAcked. If any exact frame can exceed
`MAX_ARTIFACT_WAL_RECORD_BYTES`, it returns
`artifact_writer_wal_capacity_exceeded` with zero request WAL transition,
staging or source mutation (already completed fixed Idle/control initialization
remains separate lease telemetry);
truncation, lossy field omission and a larger runtime-only record are forbidden.
Transition writes the complete next state to the inactive slot, flushes the
file, and flushes `mutation-wal/artifacts` when the slot name is first created;
only then is the generation authoritative. Recovery selects the highest valid
generation. A brand-new bucket with both slots absent and zero staging entries
is initialized by durably writing generation 0 `Idle`; both slots absent with a
staging entry is an orphan blocker. One valid slot plus one torn/invalid slot
replays the valid earlier state only when the other frame is provably torn by
length/CRC/digest and probes all later syscall ambiguity; it never assumes the
invalid write was or was not semantically completed. Exact reason mapping is:

- `artifact_writer_wal_corrupt`: no usable valid current record, two invalid
  nonempty slots, equal-generation unequal payloads, generation overflow, or a
  well-framed unknown schema/state tag; retain slots and objects;
- `artifact_writer_orphan_detected`: both slots absent/Idle with a staging
  entry, a second/overflow/unreadable transaction entry, or a control object not
  named by the selected valid WAL; retain it without deletion;
- `artifact_writer_recovery_required`: a selected valid state conflicts with
  its Intent-authorized bounded shape/identity checkpoint or source observation,
  or the exact crash table cannot classify install/cleanup; retain all evidence.

All three reveal no raw material and fail closed before capture. The two slots
are the bounded journal GC policy. The selected collision bucket is enumerated
with `MAX_STAGING_BUCKET_ENTRIES = 1`; zero or the one WAL-named transaction is
valid, while a second/overflow/unreadable entry is the exact orphan reason above.
Other collision buckets are never scanned under this lease. An implementation
may never append a third record or scan/delete an unbounded directory.

A failed transition write never authorizes its following syscall. If source
install certainty is already known, the live handler still returns the exact
typed in-memory NoChange/Committed/Uncertain plus a generic WAL/handoff reason,
but Task 10 performs no receipt transition until complete Terminal is durable;
the last valid earlier state remains blocking and recovery reconstructs/probes
the same outcome. If certainty is not known, no later mutation syscall runs.
Receipt InFlight therefore remains safely pending across Terminal-write failure
and is reconciled only by recovery; presentation success can never substitute
for the durable state.

The only stable WAL state tags and legal forward transitions are:

```rust
pub(crate) enum ArtifactWriterWalStateV1 {
    Idle(IdleWalV1),              // tag 1
    Prepared(PreparedWalV1),      // tag 2
    StagingIntent(StagingIntentV1), // tag 3
    StagingReady(StagingReadyV1), // tag 4
    InstallIntent(InstallIntentV1), // tag 5
    InstallObserved(InstallObservedV1), // tag 6
    DurabilityIntent(DurabilityIntentV1), // tag 7
    CleanupIntent(CleanupIntentV1), // tag 8
    Terminal(TerminalWalV1),      // tag 9
    ReceiptHandoffAcked(ReceiptHandoffAckedV1), // tag 10
}

pub(crate) struct ArtifactWriterWalV1 {
    pub(crate) generation: u64,
    pub(crate) state: ArtifactWriterWalStateV1,
}
```

`Idle` is not a unit/success flag. Its canonical payload is closed:

```rust
pub(crate) enum IdleWalV1 {
    BrandNew, // inner tag 1; legal only at generation 0
    Tombstone { // inner tag 2
        terminal_generation: u64,
        terminal_outcome_digest_v3: Digest32,
    },
}

pub(crate) enum WalReceiptHandoffPolicyV1 {
    NotRequired, // tag 1
    Required(ReceiptHandoffCorrelationV1), // tag 2
}
```

Every non-Idle record is independently recoverable when the other slot is torn.
It therefore begins, after its one-byte outer state tag, with the same complete
`recoveryCoreCanonicalV1`; no state may contain only a digest pointing into the
older slot. The exact core field order is:

```text
transactionId[16] || filesystemCollisionKeyDigest[32] ||
receiptHandoffPolicyCanonicalV1 || backendContractTag ||
qualifiedTupleDigest[32] || metadataPolicyDigest[32] ||
metadataQualificationDigest[32] || planDigest[32] ||
witnessGenerationDigest[32] ||
physicalWorkspaceRootIdentityDigest[32] ||
physicalControlRootIdentityDigest[32] ||
physicalDestinationRootIdentityDigest[32] ||
canonicalArtifactLocusCanonicalV1 ||
destinationRootRelativeComponentsCanonicalV1 ||
targetRelativeComponentsCanonicalV1 ||
sourceReceivingParentRelativeComponentsCanonicalV1 ||
controlCollisionBucketRelativeComponentsCanonicalV1 ||
retainedDestinationParentIdentityDigest[32] ||
retainedControlCollisionBucketIdentityDigest[32] ||
targetPrestateCanonicalV1 || expectedContentDigest[32] ||
expectedMetadataDigest[32] || firstAbsentIndexCanonicalV1 ||
publicationShapeCanonicalV1 || ownedStagingAuthorizationCanonicalV1 ||
presentTimestampRecoveryWitnessCanonicalV1
```

The four relative-component values are bounded vectors of exact private
workspace-relative components; they are not slash strings. Their exact encoding
is `u64be(count)` followed by one platform component per row: Unix=`tag 1 ||
lp(raw OsStr bytes)`; Windows=`tag 2 || u64be(UTF-16 code-unit count) || each
u16be(code-unit)`. Empty, `.`/`..`, separator/NUL, absolute/prefix and
platform-overlimit components are rejected before the plan. No Unicode, case,
WTF-8 or display-string normalization is permitted in recovery authority;
Windows unpaired code units round-trip as exact u16 values. The backend tag and
every component tag must agree. `firstAbsentIndexCanonicalV1` is None=tag 1 or
Some=tag 2 followed by one `u8`. `presentTimestampRecoveryWitnessCanonicalV1`
is None=tag 1 or Some=tag 2 followed by the §1.10 Unix=1/Windows=2 fields in
declaration order. `targetPrestateCanonicalV1` is Absent=tag 1 or Present=tag 2
followed by captured physical identity, content digest and stable metadata
digest; the separate Present witness carries the exact volatile timestamps.
Publication tags are the §4.5 three
tags 1..3. The lowercase-hex transaction ID is itself the one staging
publication-root name in the collision bucket: for file publication that name
is a file, and for subtree publication it is the root directory. There is no
extra transaction-container directory or second staging basename to orphan.
Rename may give that object its intended source basename atomically.
`OwnedStagingAuthorizationV1` is root-kind File=1/Directory=2 followed by a sorted unique bounded vector of
`relative-components || expected-object-kind-tag`; it includes the root row and
every permitted descendant, so subset validation never relies on a marker.

After that common core, the exact state-specific suffixes are:

| State | Canonical suffix after `recoveryCoreCanonicalV1` |
| --- | --- |
| `Prepared` | empty |
| `StagingIntent` | `receiptGateCanonicalV1` |
| `StagingReady` | `receiptGateCanonicalV1 || stagedObjectCheckpointVectorCanonicalV1 || stagedTreeDurabilityTranscriptDigest[32]` |
| `InstallIntent` | the StagingReady suffix, then `installAuthorityCanonicalV1` |
| `InstallObserved` | the InstallIntent suffix, then `definiteInstallObservationCanonicalV1` |
| `DurabilityIntent` | the InstallObserved suffix, then `pendingDurabilityOperationVectorCanonicalV1` |
| `CleanupIntent` | `receiptGateCanonicalV1 || installClassificationCanonicalV1 || cleanupDurabilityCanonicalV1 || cleanupAuthorizationCanonicalV1` |
| `Terminal` | `mutationOutcomeCanonicalV3 || mutationOutcomeDigestV3[32] || terminalHandoffDirectiveTag` |
| `ReceiptHandoffAcked` | the complete Terminal suffix, then `receiptTerminalDurableProofCanonicalV1` |

`receiptGateCanonicalV1` is NotRequired=tag 1 or Required=tag 2 followed by the
complete `ReceiptInFlightDurableProofV1`; it is never a bare boolean.
`stagedObjectCheckpointVectorCanonicalV1` is sorted by relative-component bytes
and each row contains relative components, File=1/Directory=2, physical identity,
optional content digest, expected/observed metadata digests and the exact
bottom-up durability proof digest. `installAuthorityCanonicalV1` contains, in
order, primitive tag, staged publication-root identity, staged target identity,
planned root/descendant relation, retained control source-parent identity,
retained source receiving-parent identity, target prestate, exact native rename
flags/transcript digest and expected after-state. `definiteInstallObservation`
contains the unique definite installed root/target identities plus every
available current target and Updated-old-object observation; missing queries use
the closed Unknown tags, never omitted fields. Pending durability rows are
sorted unique `(operation-tag, object/parent identity, transcript-id digest)`.
`installClassificationCanonicalV1` is NoInstall=1, Definite=2 with the complete
definite observation, or Possible=3 with complete `PossibleTargetInstallV1`.
`cleanupDurabilityCanonicalV1` is NotApplicableBeforeInstall=1,
VerifiedDurable=2 with the complete durability proof, or Unknown=3 with the
closed failure stage; it cannot invent target durability for NoInstall.
`cleanupAuthorizationCanonicalV1` repeats every bounded WAL-owned name/identity,
expected Removed/ConsumedByPublication relation and the exact allowed delete
set. Recovery disposition occurs exactly once inside
`mutationOutcomeCanonicalV3`; a duplicate free-standing value is forbidden.
`terminalHandoffDirectiveTag` is NotRequired=tag 0 when the common policy is
NotRequired. For Required it is recomputed from the full outcome: clean
NoChange=`RevalidateUnchangedBaselineThenClearOrRevoke` tag 1; a Committed shape
that is intrinsically eligible before post-snapshot comparison=
`ValidateEligibleCommitThenAdvanceOrRevoke` tag 2; dirty cleanup, durability
Unknown, observation mismatch/unknown, Uncertain or any other intrinsically
ineligible outcome=`Revoke` tag 3. Tags 1/2 are permission to revalidate, never
authority to clear/advance.

All nested digests above are raw 32 bytes. Every vector is preflight-bounded,
sorted and unique under §11 bytes. The state table, core order and nested tag
orders are part of `unica.artifact-writer-wal.v1`; changing or omitting a field
requires a new WAL schema and recovery migration. This closes the otherwise
undefined promise that `canonicalPayload` has an "exact field order".

The complete legal edge set is closed and versioned:

```text
Idle -> Prepared
Prepared -> StagingIntent | Terminal(NoChange)
StagingIntent -> StagingReady | CleanupIntent | Terminal(NoChange)
StagingReady -> InstallIntent | CleanupIntent
InstallIntent -> InstallObserved(Definite) | CleanupIntent | Terminal(Uncertain)
InstallObserved -> DurabilityIntent
DurabilityIntent -> CleanupIntent
CleanupIntent -> Terminal
Terminal -> ReceiptHandoffAcked                 (Required)
Terminal -> Idle                                (NotRequired and tombstone-eligible)
ReceiptHandoffAcked -> Idle                     (tombstone-eligible)
```

The first alternative on each normal row gives `Idle -> Prepared ->
StagingIntent -> StagingReady -> InstallIntent -> InstallObserved ->
DurabilityIntent -> CleanupIntent -> Terminal`. `Prepared` may reach Terminal
directly only while creation was never authorized and the chosen name is
absent. `StagingIntent|StagingReady` must durably enter `CleanupIntent` before
any explicit removal. `InstallIntent` never maps an error return directly to
NoChange: recovery probes install completion first. No other transition is
constructible.

Tombstone eligibility is recomputed, never trusted from a boolean: it is either
NoChange whose cleanup is durably `VerifiedClean`, or Committed whose cleanup is
`VerifiedClean` **and** whose mutation durability is `VerifiedDurable`; it is
never Uncertain/PossibleTargetInstall. The required receipt handoff must also be
durably acknowledged. NoChange with Residue/Unknown, Committed with
Residue/Unknown or durability Unknown, and every Uncertain remain non-Idle
blocking states even after receipt revocation/acknowledgement. Clearing a
Committed durability-Unknown WAL could turn a rename rolled back by hard crash
into an unowned staging orphan, so receipt revocation is not GC authority.
`ReceiptHandoffAckedV1` therefore repeats
the complete bounded terminal outcome, recovery authority and exact receipt
transition digest; it is self-contained if the older slot is torn. Such a
blocking Terminal/Ack has no automatic deletion/GC edge and requires a
separately reviewed repair or stronger future schema. Receipt-free blocking
Terminal likewise remains durable. `Idle` is a durable tombstone containing the
prior terminal digest and generation, not file deletion; it overwrites the
inactive slot and bounds old data. Terminal clearing is complete only after the
Idle slot and its parent directory are durable.

“Clearing” is logical state-machine GC, not a false secure-erasure claim. The
other lower-generation slot may retain at most one private predecessor payload
until the next legal transition overwrites that slot; both slots remain under
the restrictive control ACL and are never scanned as history. SSD/filesystem
forensic erasure is outside this contract and must not be promised in privacy
documentation. The next Prepared write replaces that predecessor, so retained
logical history stays bounded to one slot/one prior transaction.

`PreparedWalV1` is durable before receipt InFlight and control staging and
contains: transaction ID and the complete `ReceiptHandoffCorrelationV1` or an
explicit NotRequired tag; exact backend/qualified-tuple, metadata-policy and plan digests;
physical workspace/control/destination-root identities; retained destination
parent and control-staging-parent identities; canonical artifact locus; target
prestate; expected target content/metadata; the first-absent index; the complete
planned source receiving point; the already-chosen transaction ID/publication-
root name; and an
`OwnedStagingAuthorizationV1` containing the exact root type plus complete
allowed relative component/type set. `StagingIntent` is durable before creating
that root. Under the private locked control parent it authorizes deletion only
of that exact no-follow name when every observed entry is a subset of the
planned component/type set and no hardlink/reparse/extra entry exists. Thus a
crash after root/object creation but before an identity/marker write is
recoverable; no separate marker is assumed atomic with mkdir. Once an identity
is persisted, exact identity is mandatory.
For required handoff, `StagingIntentV1` additionally binds the exact
`ReceiptInFlightDurableProofV1` digest and rejects a proof whose correlation or
Prepared generation differs; NotRequired work stores the closed NotRequired
tag. `StagingReadyV1` additionally binds every staged file/directory physical
identity, expected content/metadata digest and bottom-up durability transcript.
`InstallIntentV1` is durably written immediately before the one source rename
and selects exactly one closed primitive:

```text
PresentFileReplaceCrossDirectory        tag 1
AbsentFileNoReplaceCrossDirectory       tag 2
AbsentSubtreeNoReplaceCrossDirectory    tag 3
```

It binds the staged root identity, definite target-file identity, retained
control source parent, retained source receiving parent, prestate, rename flags
and expected after-state. `InstallObservedV1` binds install certainty and every
available current observation. `DurabilityIntentV1` enumerates every still
required target-data, target-parent, control-source-parent and moved-subtree
directory flush. `CleanupIntentV1` binds the exact owned identities, expected
Removed/ConsumedByPublication disposition and whether install is proven absent/definite; it
is durable before any explicit control deletion. `TerminalWalV1` retains that
bounded authorization/recovery authority and contains the complete bounded
canonical schema-v3 mutation outcome **and** its digest, recovery disposition,
receipt-handoff policy/correlation tuple and the recomputed terminal handoff
directive from the state table above.
Task 10 can replay the exact transition without a live-manifest guess. Its
receipt InFlight record is
supplemental; it never replaces any of these fields.

The syscall-versus-journal crash rule is exact: a durable intent means the
syscall may or may not have happened, and a durable after-state means it did.
Recovery therefore probes rather than infers:

| Durable state at restart | Required observation and action |
| --- | --- |
| `Prepared` | chosen name absent -> staging was never authorized, so durably construct terminal clean NoChange. For Required handoff, reconcile that Terminal through the recovery port: exact current-baseline revalidation permits matching InFlight clear, already-applied exact clear or the narrowly legal absent-row `InFlightAbsentNoOp`; drift/inconclusive revalidation revokes, while unavailable, mismatched or differently transitioned receipt authority blocks. NotRequired work needs no receipt action. Any chosen-name object fails closed because creation was not yet authorized |
| `StagingIntent` | chosen name absent -> terminal clean `RecoveredNoInstall`; exact no-follow bounded subset -> persist `CleanupIntent`, attempt authorized removal and parent flush, then terminal clean or blocking NoChange Residue/Unknown; recovery never resumes stale work; invalid WAL/authorization remains unchanged and fails closed |
| `StagingReady` | exact ready tree below control -> persist `CleanupIntent`, attempt durable removal and terminal clean or blocking NoChange Residue/Unknown; chosen name absent, an unauthorized source effect or invalid identity is impossible at this highest durable state and remains unchanged/fail closed |
| `InstallIntent`, the exact staged source name/identity still exists with expected nlink, regardless of an external target change | consuming rename did not happen; persist `CleanupIntent`, attempt durable removal and terminal clean or blocking NoChange Residue/Unknown; never retry rename |
| `InstallIntent`, destination proves the staged file root or the staged subtree root plus its exact plan-bound target descendant identity | rename happened; persist `InstallObserved(Definite)` even if a later fresh location/content/metadata query is Unknown; never return NoChange |
| `InstallIntent`, either root name is present but its identity is unqueryable, staging root is absent while destination root is absent/different/unqueryable, or both names/partial subtree/identity invariants conflict | install completion is `Unknown`; persist terminal `Uncertain` with both staged root/target identities and planned relation, keep evidence/WAL and require revoke; never retry rename blindly |
| `InstallObserved` | retain definite install, repeat bounded current target/directory/Updated-old-object observations with Unknown on query failure, then durably write `DurabilityIntent`; never reclassify as Uncertain |
| `DurabilityIntent` | re-run every listed idempotent flush; a crash after a flush but before its journal update does not infer durability. Success may prove durable; failure yields definite `Committed + durability Unknown` |
| `CleanupIntent` | exact owned name present -> repeat authorized delete then control-parent flush; name absent -> repeat control-parent flush; persist clean Terminal after Removed/ConsumedByPublication proof. A returned delete/query/sync failure is persisted, when the valid WAL can still advance, as blocking NoChange/Committed Residue-or-Unknown with the authorization retained; invalid identity/extra/hardlink/reparse ambiguity never authorizes deletion and remains fail closed |
| `Terminal(Required)` | acquire the exact correlated receipt lease only while retaining artifact lease, recompute the outcome/directive and reconcile idempotently: clean NoChange clears unchanged only after exact baseline revalidation else revokes; intrinsically eligible Committed advances only after exact post-state validation else revokes; every tag-3 outcome revokes. Persist self-contained acknowledgement only for the exact durable proof |
| `Terminal(NotRequired)` | tombstone only if recomputed eligible; otherwise retain the blocking Terminal and return recovery-required before any new capture |
| `ReceiptHandoffAcked` | verify the receipt transition digest and complete terminal payload; write durable Idle only if recomputed eligible; dirty cleanup, durability Unknown or possible install stays Acked and blocks, while mismatch/cannot-read fails closed |

Recovery runs immediately after acquiring the process+OS root-wide artifact
lease and proving the same control mount/backend, before topology refresh,
authoritative capture or a new receipt lease. A second process gets Busy and
cannot race recovery. If recovery mutates source state or discovers a definite/
possible prior install, the current request stops with
`artifact_writer_recovered_retry` after recovery/handoff; it never silently treats
the recovered state as the new request's clean baseline. Corruption or
ambiguity leaves the WAL non-Idle and blocks every later mutation until a
separately reviewed repair procedure supplies authority. An orphan control
staging directory not named by the selected valid WAL is never garbage-collected
automatically; it is corruption, because deleting an unowned object is unsafe.

For `ArtifactWriterReceiptHandoffV1::Required`, Task 8 first durably writes `Prepared` with the
correlation ID and plan/grant/baseline digests. Only then does the Task 10 port
durably write the matching receipt InFlight row with that exact Prepared WAL
generation and intended transition. `StagingIntent` is forbidden until its
durable InFlight proof digest is available. This closes the kill window in
which a receipt could be InFlight while artifact WAL still looked Idle. Freeze
order is exact under locks `artifact -> receipt`:

```text
final source/metadata precondition
-> artifact Prepared(correlation) durable
-> receipt InFlight(Prepared generation) durable
-> artifact StagingIntent/.../InstallIntent/rename/outcome Terminal durable
-> receipt baseline-revalidated NoChange-clear / post-state-validated advance /
   revoke durable according to Terminal directive
-> artifact ReceiptHandoffAcked durable
-> if tombstone-eligible: artifact Idle tombstone + WAL-parent sync;
   otherwise retain blocking ReceiptHandoffAcked
```

The receipt lease is released before the artifact lease. Crash after Prepared
but before/during InFlight insertion constructs durable terminal clean NoChange
because staging was never authorized, then reconciles that exact correlation
under the receipt lease: baseline equality permits matching-row clear or an
absent-row idempotent no-op, drift revokes, and unavailable/mismatch/different
transition blocks. Crash after receipt
InFlight but before install similarly recovers from Prepared/Staging intent and
never invents a commit.
Crash after Terminal but before receipt transition replays the terminal outcome.
Crash after receipt transition but before WAL acknowledgement compares the same
correlation/outcome/transition digests and acknowledges idempotently. Crash
after acknowledgement but before Idle completes only the tombstone. A terminal
WAL in Required handoff may never be cleared before this handoff. With
`ArtifactWriterReceiptHandoffV1::NotRequired`, writer recovery may clear only a recomputed tombstone-eligible
Terminal directly; dirty cleanup, durability Unknown or possible install remains
a blocking WAL.

### 1.9 Total outcome algebra: install certainty is not observation certainty

Task 8/9 use exact schema `unica.mutation-outcome.v3`. A definite source rename
constructs exactly one `DefiniteTargetCommitV1` and therefore exactly one
`Committed`, even if every fresh query fails afterwards:

```rust
pub(crate) enum DefiniteTargetMutationKind {
    Created,                                  // tag 1
    Updated { before_content_digest: Digest32 }, // tag 2
}

pub(crate) struct DefiniteTargetCommitV1 {
    pub(crate) kind: DefiniteTargetMutationKind,
    pub(crate) intended_artifact: String,
    pub(crate) object: PhysicalFilesystemObjectIdentity,
    pub(crate) retained_destination_parent_identity_digest: Digest32,
    pub(crate) expected_content_digest: Digest32,
    pub(crate) expected_metadata_digest: Digest32,
    pub(crate) replaced_prestate: ReplacedPrestateObservationV1,
    pub(crate) location: CurrentTargetLocationV1,
    pub(crate) identity_observation: CurrentTargetIdentityObservationV1,
    pub(crate) content: CurrentTargetContentV1,
    pub(crate) metadata: CurrentTargetMetadataV1,
}

pub(crate) enum ReplacedPrestateObservationV1 {
    NotApplicableForCreated, // tag 1; legal only for kind Created
    Updated {                // tag 2; legal only for kind Updated
        object: PhysicalFilesystemObjectIdentity,
        content: ReplacedPrestateContentObservationV1,
        metadata: ReplacedPrestateMetadataObservationV1,
    },
}
pub(crate) enum ReplacedPrestateContentObservationV1 {
    MatchesCaptured,                         // tag 1
    Different { observed_digest: Digest32 }, // tag 2
    Unknown,                                 // tag 3
}
pub(crate) enum ReplacedPrestateMetadataObservationV1 {
    MatchesCaptured,                         // tag 1
    Different {                              // tag 2
        changed_field_mask: u64,
        observed_stable_digest: Digest32,
    },
    Unknown,                                 // tag 3
}

pub(crate) enum CurrentTargetLocationV1 {
    AtIntendedPath,       // tag 1
    DetachedOrRelocated,  // tag 2
    Unknown,              // tag 3
}
pub(crate) enum CurrentTargetContentV1 {
    Known { digest: Digest32 }, // tag 1
    Unknown,                    // tag 2
}
pub(crate) enum CurrentTargetIdentityObservationV1 {
    MatchesDefiniteObject,                 // tag 1
    Different { observed_object_digest: Digest32 }, // tag 2
    Absent,                                // tag 3
    Unknown,                               // tag 4
}
pub(crate) enum CurrentTargetMetadataV1 {
    VerifiedExact { policy_digest: Digest32, observed_digest: Digest32 }, // tag 1
    Mismatch {                                                           // tag 2
        policy_digest: Digest32,
        observed_digest: Digest32,
        changed_field_mask: u64,
    },
    Unknown { policy_digest: Digest32 },                                  // tag 3
}
```

The object identity is known from the flushed control-staged target before
install and remains the definite installed object identity. A failed fresh
FileId/stat query changes `location` to Unknown; it does not erase the install
identity. `AtIntendedPath` requires a fresh retained-root handle-relative walk
with `MatchesDefiniteObject`. `DetachedOrRelocated` requires `Different` or
`Absent`. Query failure produces both location Unknown and identity-observation
Unknown; it does not erase the definite staged identity. Content is Known only
after a bounded read of that exact object. Metadata is VerifiedExact only after
§1.10 postverification. Other location/identity combinations are uninhabitable.

For Present/Updated, the original target handle and physical identity remain
retained across replacement. Immediately after definite install the writer
re-reads that exact detached old object. The Updated constructor requires its
identity to equal the captured before-object and records content and stable
metadata independently as Matches/Different/Unknown. Created requires
NotApplicable. A late in-place non-cooperating content/chmod/ACL change can no
longer be erased by replacement and followed by receipt advance: Different or
Unknown remains definite Committed but revokes. This still is not namespace
CAS: an arbitrary actor may replace the path object in the irreducible final
check/rename window, so that limitation remains explicitly documented.

`Committed` owns the unique target commit; `MutationEffectsV3` contains only
definite non-target created directories plus unexpected control/source residue.
`Uncertain` contains a `PossibleTargetInstallV1` only when the source rename
completion itself cannot be determined. Target commit must never be duplicated
as both a target field and a Created/Updated effect. The stable digest is:

```text
SHA256("unica.definite-target-commit.v1\0" ||
  kind-tag || [before-content-digest-if-Updated] ||
  lp(intended-artifact) || physical-object-identity ||
  retained-parent-digest || expected-content-digest || expected-metadata-digest ||
  replaced-prestate-tag || [old-physical-object-identity ||
    old-content-observation-tag || [observed-old-content-digest] ||
    old-metadata-observation-tag ||
    [u64be(old-metadata-changed-field-mask) || observed-old-stable-digest]] ||
  location-tag || identity-observation-tag || [different-object-digest] ||
  content-tag || [observed-content-digest] ||
  metadata-tag || policy-digest || [observed-metadata-digest] ||
  [u64be(metadata-changed-field-mask)-if-Mismatch])
```

Outcome tags remain NoChange=1, Committed=2, Uncertain=3 under the v3 schema;
target kind Created=1/Updated=2; replaced-prestate NotApplicable=1/Updated=2 and
its independent content/metadata Matches=1/Different=2/Unknown=3;
location/content/metadata tags are exactly those above; identity observation
Matches=1/Different=2/Absent=3/Unknown=4. All lengths/counts use the canonical
§11 encoding. `ConsumedByPublication` references this definite-target-commit
digest plus the exact publication-root/target relation, not a display effect
digest or an invalid assumption that a subtree root is the target file.

Any Updated replaced-prestate content/metadata observation other than
MatchesCaptured, `location != AtIntendedPath`, identity observation != Matches, content Unknown or unequal to expected,
metadata Mismatch/Unknown, durability Unknown, cleanup Residue/Unknown,
recovery ambiguity or unexpected/possible effect is non-advancing and requires
receipt revocation. The sole advancing shape is one exact intended target,
Created NotApplicable or Updated with both replaced-prestate observations
MatchesCaptured,
Known expected content, VerifiedExact expected metadata, exact expected created
directories, VerifiedClean, VerifiedDurable, no possible/unexpected rows and a
matching post-manifest. Reopen failure, read failure, fresh-walk failure,
FileId/stat failure and metadata-query failure after definite rename each have a
RED proving `Committed` remains constructible and no receipt advances.

### 1.10 Closed metadata contracts for Present and Created artifacts

Every plan binds `ArtifactMetadataPolicyVersion::V1`, one platform-neutral
closed
`metadata_policy_digest`, and one expected metadata digest for the target and
for every planned new directory. The immutable grant binds policy version,
backend contract IDs and stable created-object policy scope, but never a current
Present/Absent tag, before-metadata digest or synthesized per-object metadata
digest. Those are rolling-baseline facts: putting them in the grant would make
an initially Absent second grant unusable after the first grant creates the
module. The execution plan/baseline bind the latest captured before and exact
expected-after digests, and the final precondition rejects
chmod/chown/ACL/xattr/security drift before writing. `DefiniteTargetCommitV1`, every `CreatedDirectory`
effect and the WAL bind policy + expected/observed digests. Omitting metadata
from any plan/grant/effect/WAL encoder is a compile-time constructor failure.

Stable metadata encodings use §11 `lp`/big-endian rules and raw 32-byte digests:

```text
metadataPolicyDigest = SHA256(
  "unica.artifact-metadata-policy.v1\0" || schemaByte || lp(ruleSetId) ||
  timestampPolicySetTag)

metadataQualificationDigest = SHA256(
  "unica.artifact-metadata-qualification.v1\0" || metadataPolicyDigest ||
  qualifiedTupleDigest || lp(metadataPrimitiveTranscriptId) ||
  timestampQuantumProofDigest)

presentMetadataDigest = SHA256(
  "unica.present-target-metadata.v1\0" || backendTag || stableTupleFields)

createdMetadataDigest = SHA256(
  "unica.created-artifact-metadata.v1\0" || objectKindTag || backendTag ||
  stableTupleFields || objectTimestampPolicyTag)
```

`schemaByte` is exactly `1`. The closed platform-neutral `ruleSetId` values are
UTF-8 `present-ordinary-preserve-v1` and
`created-source-parent-derived-v1`; no OS name, runtime tuple or free-form
policy label is accepted there. For Present,
`timestampQuantumProofDigest` is the qualified tuple's native resolution proof;
for Created it is the fixed raw digest
`SHA256("unica.timestamp-quantum-not-applicable.v1\0")`, never a fabricated
quantum. The backend-specific facts enter only
qualification and present/created expected/observed digests as shown.

Backend tags Unix=1/Windows=2; created kind File=1/Directory=2. Plan-level
timestamp-policy-set tags are PresentTargetStrictlyNewer=1 and
CreatedObjectsOsAssigned=2. Per-object timestamp tags are
ContentChangedStrictlyNewer=1 and OsAssignedAtCreate=2; every Created target
file/directory uses tag 2 because no old target timestamp exists to compare.
Unix stable fields are
`u32 mode || u64 uid || u64 gid || u64 nlink` for both Present and Created
(Created directory nlink is derived from the exact planned child topology under
the qualified tuple), then ACL Trivial=1, xattr Empty=1 and flags Zero=1.
Windows fields are `ownerSidDigest || groupSidDigest || canonicalDaclDigest || u64
numberOfLinks || u32 basicAttrs || streams OnlyUnnamed=1 || sacl Absent=1 || reparse None=1 ||
compressed False=1 || encrypted False=1 || sparse False=1`. Constructors accept
only these typed **stable metadata-digest** fields, never debug/JSON or volatile
time values. Timestamp recovery uses a separate private non-semantic witness:

```rust
pub(crate) enum ContentChangeTimestampPolicyV1 {
    StrictlyNewerThanCapturedPresent, // tag 1
}

pub(crate) enum PresentTimestampRecoveryWitnessV1 {
    Unix { // tag 1
        before_seconds: i64,
        before_nanos: u32,
        assigned_seconds: i64,
        assigned_nanos: u32,
        quantum_nanos: u64,
    },
    Windows { // tag 2; unsigned 100ns FILETIME ticks
        before_ticks: u64,
        assigned_ticks: u64,
        quantum_ticks: u64,
    },
}
```

Nanos must be `< 1_000_000_000`; every addition/comparison is checked. The
noncanonical live plan retains the captured-before value for the final
precondition and computes `assigned` with one operation-clock read before
Prepared. `Prepared` and every later non-Idle recovery state store the same
complete private witness and prove `assigned >= before + quantum`; no later
clock read may select a different value. Recovery can therefore
recheck the staged/installed timestamp after the old path object is gone.
Because this bounded private payload is recovery authority, the WAL frame
CRC/SHA necessarily protects its exact bytes. Raw timestamps never enter
plan/grant/outcome/receipt/metadata-policy or metadata-observation semantic
digests and never enter public diagnostics; terminal outcome exposes only
Verified/Mismatch/Unknown plus a field mask. Stable mask bit `0x1` means the
closed stable tuple differed and `0x2` means the Present last-write relation or
captured-old last-write prestate differed; zero and unknown bits are rejected.

Policy/
expected/observed digests in plan/grant/WAL/effects are the same values;
re-encoding disagreement is a constructor error. The platform-neutral policy
digest enters preview/discovery plans and grants without opening a backend. The
apply-only qualification digest enters the witness, WAL, effects and durability
proof; it never makes dry-run touch the control plane or causes otherwise equal
direct/discovery semantic plans to differ.

`PresentTargetMetadataV1` is deliberately conservative and closed:

| Surface | Unix ext4/XFS/APFS v1 | Windows NTFS v1 |
| --- | --- | --- |
| type/links | no-follow regular file; `st_nlink == 1` | non-directory ordinary data file; `NumberOfLinks == 1` |
| owner/group | exact numeric uid/gid; current credentials prove `fchown` can reproduce both | exact owner SID + primary group SID, bounded self-relative descriptor |
| mode/DACL | exact permission bits; owner read+write required; no execute/setuid/setgid/sticky; ACL must be canonical trivial ACL equivalent to mode | exact canonical owner/group/DACL copied with `SetSecurityInfo`; SACL must be absent and queryable; descriptor <= 64 KiB |
| xattr/ADS | no extended attributes; Linux `listxattr`/macOS `flistxattr` must return empty; APFS resource fork is therefore rejected | `FileStreamInformation` contains only unnamed `::$DATA`; any ADS is rejected |
| flags/special storage | Linux `FS_IOC_GETFLAGS == 0`; macOS `st_flags == 0` | no reparse point and no `COMPRESSED`, `ENCRYPTED`, `SPARSE_FILE`, offline/system/temporary flags |
| basic attrs/times | mode/uid/gid plus trivial ACL/empty xattr/zero flags are the complete stable tuple; old timestamps are never restored | allowed attributes are only normalized `NORMAL`, `ARCHIVE`, `READONLY`; old creation/access/write/change times are never copied as metadata authority |

Present precondition captures this tuple from the retained target handle,
rejects any row outside the table with `artifact_writer_metadata_unsupported`, and
re-reads it immediately before install. The writer applies it to the staged file
in closed order (Unix `fchown -> fchmod -> trivial ACL -> empty xattr -> zero
flags`; Windows owner/group/DACL -> times -> allowed attributes, with readonly
last), reads it back before `StagingReady`, then reads it after install. “Times”
here means applying the common `ContentChangeTimestampPolicyV1`, never copying
the old last-write time. The qualified tuple exposes its timestamp quantum; the
writer sets staged target mtime/LastWriteTime to
`max(before + quantum, operation_clock)` with checked range and proves the
post-install value is strictly newer than the before value. Creation/access/
change time remain OS-managed observations. The final precondition rechecks the
old target timestamp against the private captured value; after replacement the
retained old-object handle is checked again, while the installed object is
checked against `assigned`. A known mismatch sets field-mask bit `0x2`; a query
failure is Unknown. Actual volatile values follow the private WAL-witness rule
above; semantic authority encodes only policy/qualification/relation tags and
digests. If the clock/range/resolution or write-attributes operation
cannot provide this guarantee, the backend is unsupported before install. A
pre-install mismatch is NoChange; a post-install mismatch is definite
`Committed + metadata Mismatch`, non-advancing and revoking. A query failure is
Unknown, never presumed preserved.

`CreatedArtifactMetadataV1` covers an Absent target **and every directory in the
atomically moved subtree**. It is synthesized before staging from the retained
source receiving parent, never inherited from control root:

- Unix v1 requires the receiving parent and every synthesized parent policy to
  have queryable uid/gid/mode, trivial access ACL, no default ACL, empty xattrs
  and zero flags. Owner is effective uid. Group is inherited from a setgid
  parent, otherwise effective gid; the credential set must prove it can be
  applied. Directory mode is exact `0755` plus inherited setgid when present;
  file mode is exact `0644`; ambient umask is not authority because `fchmod`
  sets the exact value. ACL is rebuilt as the trivial ACL matching that mode;
  xattrs/flags remain empty/zero. The created file must have `st_nlink == 1`;
  every directory's link count must equal the qualified filesystem's exact
  value for its planned immediate child-directory topology. Each synthesized directory becomes the parent
  input for the next component. Any default ACL, nonzero flags/xattrs or
  unrepresentable ownership makes the tuple unsupported before staging/install.
- Windows v1 reads the retained receiving parent's bounded owner/group/DACL and
  uses exact reviewed `CreatePrivateObjectSecurityEx` generic mapping plus the
  captured process-token identity to synthesize the child owner/group/inherited
  canonical DACL; the resulting descriptor is applied to the staged object and
  binary-canonicalized for comparison. Every synthesized directory descriptor
  is the next parent input. Parent SACL, noncanonical/unqueryable security,
  reparse, compression, encryption, sparse inheritance, ADS policy or any
  inheritance-synthesis disagreement makes the tuple unsupported. New
  directories have only the implicit DIRECTORY attribute; the file has exact
  ARCHIVE/NORMAL policy and no ADS/reparse/compress/encrypt/sparse state; every
  created file/directory has `NumberOfLinks == 1` on the qualified NTFS row.
  Created-object timestamps are explicitly `OsAssignedAtCreate`; there is no
  nonexistent old target timestamp against which to claim "strictly newer".
  Stable policy tags remain in semantic authority; Created needs no before/after
  recovery witness and stores no volatile timestamp value.

Before the one source rename, every staged object is walked bottom-up by retained
control handles and must exactly match its `CreatedArtifactMetadataV1` digest.
After publication, a fresh retained-destination walk must verify the target and
all created directories against the same digests. This explicitly prevents a
mode/DACL inherited from `.build/.../control-v1` from entering source. If exact
inheritance synthesis, cross-directory rename metadata preservation or
postverification cannot be qualified on an OS/filesystem tuple, that tuple is
`artifact_writer_backend_unsupported` before control staging; target-parent
staging is not a fallback.

Native REDs change every metadata field before the final precondition and
in-place on the retained old Present object in the final replace window, inject
every query/set/postverify failure, and prove: completed precondition drift does
not mutate source; late old-object drift is retained as Updated
replaced-prestate Different/Unknown on definite Committed and revokes; a
definite install plus target unknown/mismatch stays Committed; no
target/created-directory metadata row can be absent from effects;
hardlink/ACL/xattr/flag/ADS/reparse/compression/encryption/sparse cases reject
without lossy replacement. A dedicated RED proves a Present content change has
a strictly newer mtime/LastWriteTime and is visible to external IDE/build/file-
watcher probes; preserving the old timestamp is a contract failure.

### 1.11 Versioned back-propagation, never frozen-file mutation

Frozen accepted/reviewed artifacts are evidence, not editable requirements.
Every correction from this v6 flows only through explicit new versioned
artifacts or addenda and receives a fresh independent audit. The pending
versioned paths are:

- upstream `.superpowers/sdd/task-5c-evidence-v2-design.md`,
  `.superpowers/sdd/task-5c-evidence-v2-self-audit.md` and
  `.superpowers/sdd/task-5c-evidence-v2-independent-review.md`; only this
  immutable Evidence family plus its implementation commit can satisfy Task 8;
- downstream `.superpowers/sdd/task-5c-mutation-v2-addendum.md`,
  `.superpowers/sdd/task-5c-mutation-v2-self-audit.md` and
  `.superpowers/sdd/task-5c-mutation-v2-independent-review.md`;
- `.superpowers/sdd/task-5b-v7-contract.md`,
  `.superpowers/sdd/task-5b-v7-self-audit.md` and
  `.superpowers/sdd/task-5b-v7-independent-review.md`;
- `.superpowers/sdd/task-6-v2-v7-addendum.md`,
  `.superpowers/sdd/task-6-v2-v7-self-audit.md` and
  `.superpowers/sdd/task-6-v2-v7-independent-review.md`; immutable
  `.superpowers/sdd/task-6-v2-design.md` is lineage only;
- `.superpowers/sdd/task-7-v6-v7-addendum.md`,
  `.superpowers/sdd/task-7-v6-v7-self-audit.md` and
  `.superpowers/sdd/task-7-v6-v7-independent-review.md`; immutable
  `.superpowers/sdd/task-7-v6-design.md` is lineage only;
- `.superpowers/sdd/task-9-v6-addendum.md` for schema-v3 persistence, WAL
  correlation fields and bounded replay;
- `.superpowers/sdd/task-10-v6-addendum.md` for receipt InFlight/handoff,
  baseline-revalidated clear, advance/revoke and crash reconciliation.

The combined working/history file `.superpowers/sdd/task-5c-v2-design.md` is not
an accepted artifact and contributes no hash, review or prerequisite edge.

No rejected v6 Task 5B hash or historical Task 6-v2/Task 7-v6 base hash
satisfies those successor gates. Their final
accepted hashes are recorded in the implementation prerequisite checklist only
after the artifacts exist and have been independently reviewed. V5 Task 8,
its independent review, and any frozen Task 5B/7/9/10 design remain byte-exact.
Active `spec/`, ADR, package contracts and production code are updated normally
only after these versioned contracts agree. Any implementation that edits a
frozen artifact, skips a required addendum/re-audit, retains target-parent
staging, clears receipt-mode Terminal WAL early, or persists schema v2 is a hard
STOP.

---

## 2. Mandatory back-propagations before Task 8 code

These are required versioned corrections, not optional cleanup. Never edit a
frozen accepted design in place: land the explicit v6 prerequisite/addendum in
§1.11, independently re-audit it, then update active spec/code/tests before
writing the Task 8 resolver.

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
- Keep one domain-owned `ArtifactRef` grammar and one opaque versioned
  `ArtifactIdentityBytesV1` constructor/comparison encoding for canonical
  method identity. Task 5B Form auxiliary catalog, Task 6 BSL projection and Task 8
  resolver are sibling consumers. Task 5B acceptance must not depend on Task 6
  implementation, and neither sibling may define its own lowercase/case-fold
  identity type.
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

### 2.2 Task 5B v7 prerequisite: one Platform XML catalog and adoption/form bindings

The contractual source for this subsection is the newly accepted
`.superpowers/sdd/task-5b-v7-contract.md`, not rejected v6 or an older Task 5B hash.

Extend, do not duplicate, the shared pure parsers:

```rust
// Exact accepted Task 5B v7/domain imports; Task 8 never redeclares their
// fields, tags, parser or smart constructors.
use crate::domain::{
    ArtifactRef, ArtifactIdentityBytesV1, BaseOwnedMetadataIdentityV1,
    ConfigurationFlavorV1, ExtensionMetadataMembershipV1,
};
use crate::infrastructure::platform_xml::{
    PlatformConfigurationCatalogSetV1, PlatformConfigurationCatalogV1,
    RegisteredManagedFormCatalogSetV1, RegisteredManagedFormCatalogV1,
    RegisteredManagedFormAuthorityV1, RegisteredFormManifestKeyV1,
    CatalogScriptVariantAuthorityV1, CatalogNamePrefixAuthorityV1,
    BoundedNamePrefixV1,
    RegisteredPlatformFormV1, RegisteredPlatformFormAuthorityDigestV1,
    TypedFormCatalogFailureV2, parse_platform_form_binding_catalog_v2,
    PlatformFormDocumentFlavorV2,
    CompleteFormMethodBindingsV2, PlatformFormBindingRegistryVersionV2,
    CompleteFormMethodLookupV2, InvalidFormMethodLookupV2,
    CompleteFormMethodBindingsV2View,
};
```

The neutral Task 5B v7 module, not Task 8, owns
`CompleteFormMethodBindingsV2View::lookup_method(&RegisteredPlatformFormV1, &ArtifactRef) ->
Result<CompleteFormMethodLookupV2, InvalidFormMethodLookupV2>`. `Bound` carries a nonempty sorted matching-binding
set digest plus checked `NonZeroU32` count because several legal bindings may name the
same method; no consumer may select a first row. `Unbound` is constructible only
from a whole-document-complete V2 catalog. The view also exposes the opaque
`PlatformFormBindingRegistryVersionV2` and complete catalog semantic digest.
These exact typed exports are a Task 5B v7 freeze prerequisite; an unchecked
String version, consumer-side scan, or old unversioned lookup is a hard STOP.
The count is checked `NonZeroU32` and is proven `<=` the accepted whole-capture
XML node bound because every retained binding consumes a distinct audited
binding-shaped node. There is no separate binding/matching cap, failure branch
or unreachable local 1,000,001-binding test: the capture node bound owns its
outer N/N+1 proof. A `u16` truncation, first-row selection or downstream tighter
binding cap is forbidden.
The same view must expose the parser-owned closed
`PlatformFormDocumentFlavorV2` (`Plain=1`, `Borrowed=2`) through
`form_document_flavor(&self) -> PlatformFormDocumentFlavorV2`; Task 8 may not
infer flavor from source kind, membership or BaseForm text. This typed accessor
is an accepted Task 5B v7 freeze prerequisite.
Task 8 maps an invalid/foreign-Form/wrong-module lookup request to
`cfe_form_binding_inconclusive`; it never treats the error as Unbound or retries
with a display/case-normalized method. Bounded detail retains only the exact
closed `InvalidFormMethodLookupV2` tag (`RegisteredAuthorityMismatch`,
`InvalidArtifactRef`, `WrongKind` or `ForeignFormModule`), never raw
ArtifactRef/display text.
The input is the exact module-qualified Method `ArtifactRef`, because kind and
FormModule ownership cannot be recovered safely from opaque comparison bytes.
Only after the view accepts that `ArtifactRef` does Task 8 construct and retain
the same Task 5A/domain `ArtifactIdentityBytesV1` for plan/digest comparison.
`CompleteFormMethodBindingsV2` is an auxiliary snapshot-bound lookup catalog,
not a `ProviderFact`, `EvidenceRecord` or evidence-admission group. Task 8
consumes its `VerifiedRegisteredFormBindingsV2` and semantic/material/authority
digests directly;
it never expects Form/Element event evidence rows, a retained-group proof or
`maxEvidence` admission to authorize `Bound`/`Unbound`. The separate Task 5B
tag-4 FormCommand fact narrowing has no Task 8 lookup payload.
More precisely, Task 8 independently constructs the opaque
`RegisteredPlatformFormV1` only from the one borrowed
`RegisteredManagedFormAuthorityV1` sidecar entry, its exact owner in the bound
Configuration catalog and the matching captured manifest material, then invokes the same neutral
`parse_platform_form_binding_catalog_v2(&registered_form, verified_bytes)` over
its own `read_verified` bytes. Every lookup passes that exact current handle;
the accepted view internally derives its private binding and compares it before
it inspects the Method. Task 8 never caller-compares a detached binding/digest.
After successful lookup it retains the view's
`RegisteredPlatformFormAuthorityDigestV1` for execution/baseline. Source/
catalog/form/fingerprint/content mismatch is `cfe_form_binding_inconclusive` with bounded
`platform_xml_snapshot_catalog_mismatch` detail, never Unbound. A provider
record transport, serialized catalog, raw tuple/path reconstruction or replay
of one complete catalog under another source/Form/snapshot is forbidden.
`TypedFormCatalogFailureV2` likewise maps to
`cfe_form_binding_inconclusive` plus only its exact closed variant tag and
optional already-validated bounded Form span; Task 8 never invents a provider
fact/gap or exposes parser text.

Task 8 borrows both exact Configuration catalogs from the one accepted
`PlatformConfigurationCatalogSetV1` and the matching analysis/destination Form
sidecars from the same-snapshot `RegisteredManagedFormCatalogSetV1`. It consumes
the Configuration catalogs' existing `script_variant_authority` and
`name_prefix_authority` fields directly, constructs no properties wrapper and
does not reparse Configuration.xml. A Form sidecar is never synthesized from a
canonical ref or path spelling.

- `CatalogNamePrefixAuthorityV1` is direct exact
  Configuration/Properties material, preserving Missing/Empty/Value/
  Inconclusive rather than defaulting; only destination `Value(BoundedNamePrefixV1)`
  is usable. Duplicate/wrong-namespace/mixed/invalid material is the exact
  Inconclusive problem and never chooses one node.
- `CatalogScriptVariantAuthorityV1` preserves Missing/Known/Unknown/
  Inconclusive. Task 8 accepts only Known Russian/English and maps the other
  closed states to their exact §14 reasons without a Russian fallback.
- `CatalogConfigurationFlavorAuthorityV1::Known(ConfigurationFlavorV1)` is
  captured material, not a copy of `SourceSetKind`. The shared
  parser classifies only exact direct `Configuration/Properties` fields in the
  exact `http://v8.1c.ru/8.3/MDClasses` namespace; same-local-name unqualified,
  foreign-namespace, attribute or descendant decoys never substitute:
  configuration-level `BaseConfiguration` is exactly absent root
  `ObjectBelonging` plus absent `ConfigurationExtensionPurpose`;
  `ExtensionConfiguration` is exact root `ObjectBelonging=Adopted` plus exactly one supported purpose (`Patch`,
  `Customization`, or `AddOn`). Purpose without belonging, Adopted without a
  valid purpose, another/duplicate belonging, duplicate purpose or mixed-content
  singleton is an inconclusive flavor view. Direct
  `ConfigurationExtensionCompatibilityMode` and
  `KeepMappingToExtendedConfigurationObjectsByIDs` are optional `0..1` on both
  flavors and are never discriminators. When present, compatibility is one
  nonempty control-free scalar of at most 256 UTF-8 bytes/128 scalars and
  KeepMapping is exact lowercase `true|false`; invalid/duplicate optional
  material makes only the flavor semantic view inconclusive. This root flavor
  grammar is separate from each registered object's membership: object-level
  absent or exact `ObjectBelonging=Own` with no extended UUID is Known Own and
  must remain accepted on either flavor. This exact table
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
- `PlatformConfigurationCatalogV1.configuration_root_uuid` remains the
  accepted semantic `ConfigurationRootUuidAuthorityV1`; its `Inconclusive`
  variant is a catalog-semantic problem, not a snapshot/capture failure. Task 8
  does not require `Known` as a global gate for unrelated registered root/Form
  adoption rows. If an exact queried upstream companion specifically depends on
  that Configuration object authority, its normal typed gap/inconclusive result
  is consumed; Task 8 never relabels the source unreadable, never promotes the
  root authority to a raw `MetadataIdentity`, and never uses it in place of a
  root/Form `BaseOwnedMetadataIdentityV1.object_uuid` adoption join.
- Root/form descriptor parser exposes exact UUID, ObjectBelonging and
  ExtendedConfigurationObject through the accepted v7 catalog authorities and
  typed `BaseOwnedMetadataIdentityV1` / `ExtensionMetadataMembershipV1`
  companions; descendants/attributes/wrong namespace do not substitute for
  direct fields. A successfully parsed present descriptor is exactly Own or
  Adopted with its typed UUID; descriptor absence is the separate
  MetadataAbsent polarity. Unknown/duplicate/malformed belonging or extended
  UUID material fails the provider with its exact stable reason and never
  becomes a Task 8 enum payload.
- For every Form pair, the accepted Task 5B provider joins descriptor
  membership with the same parser-derived Form document flavor before it emits
  a CFE companion: analysis BaseConfiguration+Own requires `Plain`; destination
  ExtensionConfiguration+Own requires `Plain`; destination Adopted requires
  `Borrowed`. Any other pairing is an exact scoped typed provider gap and emits
  no CFE companion. Task 8 independently asserts analysis `Plain` and adopted
  destination `Borrowed` from its own handle-bound auxiliary catalogs and binds
  both stable flavor tags into Form safety/execution proof; `Unbound` alone is
  never sufficient. A direct mismatch maps to
  `cfe_form_binding_inconclusive` with exact bounded
  `form_flavor_membership_mismatch` detail, matching the upstream scoped gap.
- A CFE analysis UUID may be consumed only from
  `BaseOwnedMetadataIdentityV1` under a known BaseConfiguration catalog. An Adopted
  analysis root/Form is `analysis_not_base_owned`; it cannot be
  reduced to plain `MetadataIdentity`. The destination catalog must be
  `ExtensionConfiguration`, and its exact object/Form descriptors remain the
  Adopted side of the join. This closes a misdeclared extension source without
  adding a second `BaseMetadataIdentity` model.
- Task 5B first extracts one neutral registry from the audited Form
  implementation and gives it the opaque
  `PlatformFormBindingRegistryVersionV2` whose sole accepted value is
  `platform-form-bindings/v2`. That
  single registry owns every definition/item/event compatibility row,
  callType/BaseForm rule, command Action cardinality, recursive completeness
  edge, identifier/opaque-ID lexical rule and limit. Form edit/validate and the
  complete binding catalog consume the same value. Task 8 is forbidden to name,
  copy, filter or extend those rows.
- `CompleteFormMethodBindingsV2` is constructible only by the accepted Task 5B v7
  parser after its whole-document audit succeeds. Its only Task 8 operation is
  an exact module-qualified Method `ArtifactRef` lookup returning `Unbound` or `Bound` with a
  bounded opaque binding digest. Unknown token/kind, illegal cardinality,
  unsupported callType/BaseForm pairing, namespace error, parser limit or any
  unconsumed binding-shaped material prevents construction; Task 8 receives
  `cfe_form_binding_inconclusive`, never an empty/partial view.
- `PlatformFormBindingRegistryVersionV2` and the query-specific method result
  enter the immutable Form semantic proof/grant. The complete catalog semantic
  digest, exact Form material identity and
  `RegisteredPlatformFormAuthorityDigestV1` are rolling execution/baseline
  facts, not grant fields: otherwise an unrelated Form binding edit would
  invalidate a still-unbound grant. A registry upgrade changes the semantic proof and
  requires fresh discovery; Task 8 never treats two versions as compatible
  merely because their current lookups match.
- The neutral registry imports Task 5A/domain's one `ArtifactRef` +
  `ArtifactIdentityBytesV1` canonical Unicode method-identifier seam. Task 6
  projections and Task 8 requested source/generated names use that same seam;
  Task 5B imports no Task 6 implementation. Task 8 does not restate
  item/event/command/ID syntax or bounds.
  Task 5B's exhaustive matrix, lexical N/N+1, duplicate, BaseForm/callType and
  zero/duplicate-Action REDs must be accepted and GREEN first.
- Task 4 catalog selection and Task 5 providers continue using these same
  parsers. Task 8 receives typed projections through an application port.
- Task 5B preserves typed Platform XML Extension facts for general Explore and
  does not emit a source-readiness failure for them. The application CFE
  mutation preflight accepts analysis facts only from exact
  `SourceSetKind::Configuration` + PlatformXml; for an Extension it emits the
  closed Task 7/§5.4 result, leaves the proposal Unknown/ineligible and never
  sends that proposal to issuer. The exact object UUID authority for an
  adoption join is the source-bound
  `BaseOwnedMetadataIdentityV1.object_uuid`; no `BaseMetadataIdentity`
  fact/parser is added and neither an extension wrapper's local
  `MetaDataObject/@uuid` nor the separate Configuration-root UUID catalog
  authority is substituted.
- Task 5B exposes the neutral complete Form parser/binding material seam for
  both selected analysis and exact destination Form artifacts. Task 8's
  independently snapshot-bound analysis catalog proves the source method is
  Ordinary; its independently bound destination catalog proves the generated method
  identity is unbound. Both use `CompleteFormMethodBindingsV2`; a second Form
  parser or a BSL-only destination proof is forbidden.
- Task 5B's closed EventSubscription event/source compatibility matrix remains a
  mandatory shared-parser/provider prerequisite because the same catalog must
  not emit semantically impossible relationships. Task 8 consumes no
  EventSubscription row and adds no EventSubscription field to a CFE plan,
  digest or receipt; it neither bypasses nor reimplements that Task 5B gate.

### 2.3 Task 6 v2+v7-addendum prerequisite: source extraction and duplicate facts

The contractual source is accepted
`.superpowers/sdd/task-6-v2-v7-addendum.md`, read with immutable
`.superpowers/sdd/task-6-v2-design.md` as lineage only; the base hash alone is
not an acceptance gate. Task 5B's successor contract/independent review is
accepted before Task 6 implementation starts, but Task 6 imports no Task 5B
identifier code: both are sibling consumers of the already-landed Task 5A/domain
identity seam. Keep one bounded lexer/parser. Extend `BslSyntaxDefinition` with validated spans
needed for exact source extraction while retaining every accepted v2 field;
the successor addendum must extend this shape, not replace it:

```rust
// Owned by the Task 6 lexer module; private representation.
pub(crate) struct BslIdentifierV1(/* private */);

pub(crate) fn parse_complete_bsl_identifier_v1(
    input: &str,
) -> Result<BslIdentifierV1, BslIdentifierErrorV1>;

impl BslIdentifierV1 {
    pub(crate) fn exact_spelling(&self) -> &str;
}

pub(crate) struct BslSyntaxDefinition {
    pub(crate) name: String,
    // Constructed only by Task 5A/domain ArtifactIdentityBytesV1 authority.
    pub(crate) name_identity: ArtifactIdentityBytesV1,
    pub(crate) definition_span: BslSpan,
    pub(crate) declaration_span: BslSpan,
    pub(crate) name_span: BslSpan,
    pub(crate) parameter_list_span: BslSpan,
    pub(crate) body_span: BslSpan,
    pub(crate) terminator_span: BslSpan,
    pub(crate) declaration_line_ending: BslLineEnding,
    pub(crate) shape: DefinitionShape,
    pub(crate) local_shadow_names: Vec<String>,
    pub(crate) maybe_local_shadow_names: Vec<String>,
}

pub(crate) struct BslSpan {
    pub(crate) start_byte: u32,
    pub(crate) end_byte_exclusive: u32,
    pub(crate) line: u32,
    pub(crate) column: u32,
}

pub(crate) enum BslLineEnding { Lf, Crlf, Cr }

pub(crate) enum ObservedCfeInterceptorKind {
    Before,
    After,
    Around,
    ModificationAndControl,
}
```

`parse_complete_bsl_identifier_v1` runs the exact same Task 6 token classifier
and accepted 512-byte/128-scalar bounds, requires exactly one non-keyword
Identifier token followed by EOF, and preserves its bytes. Empty input,
trailing tokens, whitespace padding, a keyword, lexical failure or either N+1
limit is an error. It is the only standalone identifier constructor exported to
Task 8; it is not a second parser or a Task 8 approximation.

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
- Project every definition and annotation target through the one upstream
  Task 5A/domain `ArtifactIdentityBytesV1` constructor. Task 5B Form catalog,
  Task 6 parser DTO and Task 8 generated-handler comparison consume that same
  opaque identity; Task 6 owns spans/syntax, not identifier comparison, and no
  sibling may implement a second lowercase/case-fold routine.
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

### 2.4 Task 7 v6+v7-addendum prerequisite: optional assertions and resolver-ready issuance

The contractual source for this subsection is the newly accepted
`.superpowers/sdd/task-7-v6-v7-addendum.md`, read with immutable
`.superpowers/sdd/task-7-v6-design.md` as lineage only; the base hash alone is
not an acceptance gate.

The addendum must partition its implementation boundary explicitly. Its
`Task7PrerequisiteSliceV1` is independently implementable before Task 8 and
contains the optional raw assertions, exact Configuration-only application
preflight, zero-material-read prepare orchestration and injected resolver/issuer
ports with recording fakes. It contains no concrete `ResolvedMutationPlan` and
cannot claim that Task 8 resolution is already implemented. The concrete
`Task7Task8IntegrationV1` that carries `ResolvedProposalMutationPlan` into the
issuer is implemented and evidenced inside the later Task 8 implementation
commit. One undifferentiated "Task 7 implementation complete before Task 8"
gate would be a dependency cycle and is forbidden.

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
- outcome/WAL schema, privacy, observability and replay documentation for
  `unica.mutation-outcome.v3` and `unica.artifact-writer-wal.v1`;
- the explicit v6 artifacts/addenda in §1.11. Frozen historical files are not
  edited; active docs must point to their accepted successors.

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
- the acyclic Task 7 split: `Task7PrerequisiteSliceV1` has zero Task 8 import
  and is the only upstream implementation gate, while
  `Task7Task8IntegrationV1` plus its separate evidence hash is delivered by the
  Task 8 commit; active spec/ADR/ledger must never call a full Task 7
  implementation an upstream Task 8 prerequisite;
- exact signature/body cloning;
- source wrong-mechanism rejection plus destination generated-handler
  collision/absence proof from the accepted Task 5B v7 neutral registry
  version and complete shared projection; Task 8 contains no registry table;
- already-borrowed UUID chain and no implicit borrow;
- `ExtensionRequired` is CFE-patch receipt-ineligible;
- SnapshotWatch, digest fields, limits and stable reasons;
- accepted Task 5B v7 neutral Form registry/version with exhaustive compatibility,
  callType/BaseForm, Action-cardinality, lexical and completeness tests; the
  spec points to that authority and does not copy its rows into Task 8;
- exact Own/Adopted/missing/mismatch/inconclusive lattice and guidance that
  `/cfe-borrow` is appropriate only for the absent-descriptor branch;
- observed Before/After/Around/ModificationAndControl conflict matrix;
- one control-staged atomic subtree publication for an absent parent suffix,
  exact `CreatedArtifactMetadataV1` for every moved file/directory and allowed
  directory effects; no source-tree parent-by-parent creation/rollback;
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
- `unica.unix-contained-atomic.v2` and
  `unica.windows-contained-atomic.v2` closed tuple allowlists with exact lock,
  cross-directory file/subtree no-replace/replace, WAL, closed metadata and
  both source/control-parent durability primitives; network/FUSE/
  unknown tuples fail before source mutation and filesystem names alone confer
  no support. macOS APFS case-sensitive and case-insensitive rows are distinct
  exact `fpathconf(fd, _PC_CASE_SENSITIVE)` tuples, while both reuse the same
  root-wide collision protocol and NFC/NFD Present/Absent two-process proof;
- Windows handle-relative destination-root/control-staging/target operations after
  one trusted workspace open, explicit backend version/support allowlist, and
  rejection of path-based destination reopen, `MoveFileExW` and
  checked-then-create containment claims;
- typed schema-v3 NoChange/Committed/Uncertain handler outcomes; one
  `DefiniteTargetCommitV1` separated from current location/content/metadata,
  name-based `VerifiedClean|Residue|Unknown` cleanup and
  independent `VerifiedDurable|Unknown` durability; Removed versus
  ConsumedByPublication staging-root lifecycle with exact root/target relation;
  exact expected/unexpected/possible and
  detached physical effects; NoChange scoped only to source artifacts; and
  receipt revocation even when presentation outcome reports failure;
- bounded dual-slot WAL recovery on every lease, exact crash-window matrix,
  fail-closed corruption/orphan handling and eligible-only terminal GC;
- Required receipt InFlight before install and terminal WAL -> durable receipt
  transition -> acknowledged handoff -> eligible Idle tombstone or blocking Ack
  ordering; NotRequired has no fabricated receipt row;
- Present and Created metadata policy/version in grant scope, with only the
  stable Created synthesis scope there; current Present/Absent state and exact
  expected/observed metadata digests live in plan/baseline, WAL and effects,
  including absent-subtree directory metadata;
- acyclic split dependency: separate Task5C-Evidence-v2 immutable
  design/self-audit/review -> Evidence implementation, then with Task5B/6/7 ->
  Task8 design -> accepted Task9/10 addendum designs -> Task8 writer
  implementation -> Task9 store implementation -> Task10 handoff implementation
  -> separate Task5C-Mutation addendum/review -> Mutation implementation;
  immutable hash ledger and CI rejection of the combined file/whole-Task5C as a
  Task8 prerequisite;
- RU/EN renderer and one-plan plumbing.

The Task 9 v6 addendum must delete `${cache_root}`, path-derived workspace
keys and `MoveFileExW`: Task 8 owns the first safe BSL atomic writer, fixed
workspace control-root opener, semantic artifact key and collision-safe
root-wide physical artifact-lock key. Task 9 reuses
those primitives, adds only receipt record/lease namespaces under the same fixed
root, persists complete schema v3 (unique target, possible publication
root/target relation, Updated replaced-prestate, dirty NoChange cleanup,
metadata relation masks) plus WAL correlation fields, and owns persistent
receipt revision tests. The Task 10 v6 addendum, active spec and ADR must replace
receipt-first guard order with artifact lease -> WAL recovery ->
refreshed current plan -> optional current receipt lease, and replace its weak
display-derived `MutationEffects` with the exact Task 8 outcome/cleanup/detached
types, current target and Updated replaced-prestate observations, metadata,
staging-root/target relation lifecycle, durability and terminal handoff. The skill must remove synthetic Context/IsFunction defaults and describe
busy/retry, exact Form/flavor and failure boundaries. None of these documents may
remain contradictory when Task 8 production code starts.

The back-propagation gate is exact:

| Source | Stale contract that must disappear | Required replacement |
| --- | --- | --- |
| active architecture spec | receipt lease is first; no WAL/metadata/current-observation scope | artifact open/mount/backend/root-wide lease, WAL recovery, refresh, optional receipt lease; control staging; schema-v3 authority |
| ADR 0008 | receipt-only serialization and receipt-first post flow | artifact->receipt order, reverse release, durable InFlight/Terminal/receipt/ack/tombstone handoff and mixed-version migration gate |
| CFE skill | Context defaults; duplicated Form prose; no recovery/metadata boundary | derive assertions; consume Task 5B v7; busy/recovery/metadata/backend diagnostics |
| Task 5B prerequisite | rejected v6 or an open v7 review finding | accepted `.superpowers/sdd/task-5b-v7-contract.md` family, shared registry and strict flavor/Own/EventSubscription gates GREEN |
| Task 6 prerequisite | historical v2 base treated as current acceptance | accepted `.superpowers/sdd/task-6-v2-v7-addendum.md` family plus implemented v2+addendum behavior; base file is lineage only |
| Task 7 prerequisite | historical v6 base or implicit current-plan assumptions | accepted `.superpowers/sdd/task-7-v6-v7-addendum.md` family plus implemented v6+addendum behavior; base file is lineage only |
| Task 9 | v2 effects, no WAL correlation, mutable historical edit | new `.superpowers/sdd/task-9-v6-addendum.md`; persist generic schema v3 and WAL/receipt correlation only; Task 9 owns store/revision/receipt lease |
| Task 10 | display-shaped effects, receipt-first order, early journal clear | new `.superpowers/sdd/task-10-v6-addendum.md`; exact target observations/metadata, InFlight and idempotent terminal handoff, baseline-revalidated clear plus strict advance/revoke |
| observability/replay | infer success from display/live manifest; expose staging paths | preserve schema-v3 install/observation/cleanup/durability/recovery dimensions; bounded redacted WAL diagnostics |
| package/CI support | same-directory writer or syscall-presence fallback | exact v2 tuple digests, cross-directory file+subtree rename, metadata synthesis, WAL recovery and native race/failure/hard-crash gates |

`tests/ci/test_product_contracts.py` must fail on each stale cell and pass on each
replacement before any Task 8 production slice. Updating only this design does
not satisfy the gate.

### 2.6 Task 9/10: immutable grants versus rolling baseline

- Task 9 `DiscoveryGrantV1` stores the complete atomic scope tuple and
  `grantScopeDigest`, including the exact allowed file plus bounded stable
  parent-directory creation scope, not the current absent suffix, destination
  fingerprint or module precondition. It additionally stores metadata policy
  version/digest, stable Created metadata synthesis scope and the required
  artifact-WAL/receipt-handoff versions. Current Present/Absent state,
  before-metadata and synthesized per-object expected digests are baseline/
  execution material, never immutable grant authority.
- That grant tuple also stores captured Base/Extension flavor tags, exact Own
  analysis + Adopted destination UUID chains, analysis source-method-unbound
  Form semantic proof and destination generated-name-unbound Form semantic
  proof, including
  the exact accepted Task 5B v7 `PlatformFormBindingRegistryVersionV2`. Task 9
  baseline stores the exact six-field `CfeCatalogSnapshotBindingV1` and both
  exact Form material identities; Task 10 re-proves the semantic tuple and
  compares the refreshed catalog binding/materials before any advance.
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
  `artifact mutation lease -> WAL recovery -> current receipt lease`; release is
  `current receipt lease -> artifact mutation lease`. It never acquires an
  artifact lease while holding any receipt lease and never acquires a second
  receipt while either is held.
- Under the already-held artifact lease Task 10 captures/resolves one current
  plan. Guard policy then decides whether a validated receipt is actually
  presented and leased. If so, Task 10 rereads revision + baseline under that
  lease, requires the baseline to equal the plan capture, compares one exact
  grant scope and supplies `ArtifactWriterReceiptHandoffV1::Required`; after the
  final precondition and durable Prepared, but before StagingIntent/install, its
  port durably persists correlated InFlight. Otherwise an allowed call supplies
  `NotRequired` and creates no receipt row. This choice is not derived inside the
  writer from a mode string: public modes are exactly `off|observe|warn|deny`;
  deny with a valid leased receipt is normally Required, while warn/observe may
  be Required only when a real validated receipt lease was acquired; off never
  acquires one and is NotRequired, while deny without acceptable receipt stops
  before the writer. `enforce`
  is not a public/closed guard value. Every allowed branch still uses the
  artifact lease and WAL. The current `executionPlanDigest` is only for same-call
  precondition/audit.
- After successful in-scope A, Task 10 captures post snapshot and advances the
  baseline/revision while both leases remain held and leaves all grant
  tuples/scope digests unchanged. The writer's Terminal WAL stays durable until
  that exact receipt transition is durable and acknowledged; WAL tombstoning is
  the final artifact-side step.
- A later B resolves again over the advanced baseline. It may have a different
  execution digest/module boundary but must match the same immutable B grant.
- If A changed an immutable B scope field (source definition, adoption UUID,
  ScriptVariant, NamePrefix, generated identity or allowed effects), B's scope
  no longer matches and is denied/revoked; no grant is rewritten to hide drift.
- Add a two-grant same-destination-module persistent revision RED test to Task
  9/10/spec before the overall feature is considered integration-ready; Task 8
  itself covers only the corresponding pure algebra.
- Task 10 consumes the unique `DefiniteTargetCommitV1` separately from
  `CreatedDirectoryV3`, `OwnedStagingObject` and possible-install rows. Every
  path-resolved created directory
  must equal the execution plan's current `directories_to_create`, ancestor-
  first, and fall inside the grant's stable creation scope; any detached identity
  is unexpected and non-advancing even if commit is definite. Post-manifest diff
  and typed effects/metadata observations must agree before receipt advance. Reconciliation/event/cache
  work begins only after both current leases are released.
- Task 10 consumes typed mutation outcome on both adapter success and failure.
  `NoChange` with verified clean staging invokes the tag-1 directive: only an
  exact current composite-baseline revalidation durably clears its correlated
  InFlight row without baseline/revision advance; drift or inconclusive
  revalidation revokes instead. `NoChange` with
  Residue/Unknown revokes and leaves the artifact WAL blocking;
  `Committed` advances only when durability is `VerifiedDurable`, cleanup is
  `VerifiedClean`, target location is `AtIntendedPath`, identity observation is
  Matches, content is Known exact, and Created has NotApplicableForCreated or
  Updated has both replaced-prestate observations MatchesCaptured,
  metadata is VerifiedExact, every staging-root identity is exactly Removed or
  ConsumedByPublication with the expected relation, expected non-target
  effects/metadata exactly equal the plan,
  unexpected/possible sets are empty and the post-manifest agrees. `Committed`
  with an Unknown/different location/content/metadata, durability Unknown,
  Residue/Unknown, any unexpected/possible effect and every
  `Uncertain` outcome revoke the current receipt before lease release. The
  atomic writer never has an untyped `Result::Err` escape: before Prepared its
  typed apply result carries the exact operation-neutral reason plus
  NoChange+Unmodified+empty VerifiedClean and zero request WAL/staging/source
  effect; once `Prepared` is durable, an ordinary returned failure is represented
  by the exact WAL-backed typed outcome and presentation status (control
  initialization remains separate telemetry), so the applied handler always
  returns typed authority;
  pre-install failure maps to `NoChangeSourceTreeProof::Unmodified` with its
  actual clean/residue/unknown cleanup state rather than dropping mutation
  authority or inventing install uncertainty.
- Task 10 never discovers staging residue from display text or a manifest that
  may ignore control names. It consumes the exact WAL-backed identities and
  verifies cleanup while the artifact lease is still held.
- Task 9 persists only the generic `unica.mutation-outcome.v3` schema boundary:
  the unique definite target commit or possible install, current location/
  content/metadata observations, Updated retained-old-object prestate
  observations, sorted non-target effects, exact staging
  lifecycle, cleanup, backend-qualified durability/recovery disposition and the
  WAL/receipt correlation tuple. It stores closed tags/digest encodings and all
  bounds, not display text, raw paths or an advance boolean. Task 10 production
  replay and observability preserve every independent dimension and never infer
  install, metadata or durability from a matching live post-manifest or
  `AdapterOutcome.ok`.
- Fixed control-plane initialization is recorded in a separate bounded internal
  lease-observation channel. Task 9/10 never compare its directories/lock inode
  with grant-authorized source effects, and `NoChange` may coexist with expected
  first-call control initialization while source effect vectors stay empty.
  Control staging and WAL state are nevertheless exact recovery authority and
  cannot be omitted or garbage-collected merely because they are not grant
  effects.

Task ownership is executable, not editorial: Task 8 final verification runs
only resolver/discovery determinism, pure grant/effect algebra, artifact lease,
native writer and recording-fake suites. Task 9 owns persistent receipt store,
revision and receipt-lease tests. Task 10 owns the production guard order,
advance/revoke/reconciliation pipeline and `discovery_receipts` integration
suite. A Task 8 fake must not be cited as production receipt proof.

### 2.7 Acyclic dependency and immutable hash ledger

The feature order is deliberately split to avoid the Task8<->Task5C cycle and
to distinguish an accepted contract from its later implementation:

```text
Task5C-Evidence-v2 immutable design/self-audit/review
    + Task5B-v7 contract/self-audit/review
    + Task6-v2-v7 addendum/self-audit/review
    + Task7-v6-v7 addendum/self-audit/review
    -> accepted Task5A implementation
    -> Task5B-v7 implementation
    -> Task6-v2+v7-addendum implementation + Task5C-Evidence implementation
    -> Task7-v6+v7-addendum Task7PrerequisiteSliceV1 implementation
    -> Task8-v6 design
    -> Task9-v6 + Task10-v6 addendum designs/reviews
    -> Task8 generic writer/WAL/outcome + Task7Task8IntegrationV1 implementation
    -> Task9 persistence implementation
    -> Task10 production handoff implementation
    -> Task5C-Mutation-v2 addendum/self-audit/review
    -> Task5C-Mutation implementation (`support.edit` imports Task8 writer)
```

Before each arrow is crossed, `docs/project-discovery-v6-hash-ledger.md` records
64-lowercase-hex SHA-256 for each immutable design/addendum, self-audit,
**separate independent review** and native evidence bundle. Implementation
commits use this repository's verified Git object format, currently SHA-1, and
are exact 40-lowercase-hex Git OIDs named `*_implementation_commit` (never
mislabelled SHA-256):

| Ledger key | Required before |
| --- | --- |
| `TASK5A_ACCEPTED_SHA` (exact 40-lowercase-hex implementation Git OID from the accepted Task 5B v7 ledger) | Task 5B/6 and Task 7 prerequisite-slice production, and therefore Task 8 production code |
| `task5c_evidence_v2_design_sha256`, `task5c_evidence_v2_self_audit_sha256`, `task5c_evidence_v2_independent_review_sha256`, `TASK5C_EVIDENCE_ACCEPTED_GIT_OID` | Task 8 production code |
| `task5b_v7_contract_sha256`, `task5b_v7_self_audit_sha256`, `task5b_v7_independent_review_sha256`, `task5b_v7_implementation_commit` | Task 8 production code |
| `task6_v2_v7_addendum_sha256`, `task6_v2_v7_self_audit_sha256`, `task6_v2_v7_independent_review_sha256`, `task6_v2_v7_implementation_commit` | Task 8 production code; immutable `task-6-v2-design.md` hash is lineage, not a substitute |
| `task7_v6_v7_addendum_sha256`, `task7_v6_v7_self_audit_sha256`, `task7_v6_v7_independent_review_sha256`, `task7_v6_v7_prerequisite_implementation_commit` | Task 8 production code; the commit proves only exact `Task7PrerequisiteSliceV1`, while immutable `task-7-v6-design.md` is lineage, not a substitute |
| `task8_v6_design_sha256`, `task8_v6_self_audit_sha256`, `task8_v6_independent_review_sha256` | Task 8 production code |
| `task9_v6_addendum_sha256`, `task9_v6_independent_review_sha256` | Task 8 production code |
| `task10_v6_addendum_sha256`, `task10_v6_independent_review_sha256` | Task 8 production code |
| `task8_writer_implementation_commit`, `task7_task8_integration_evidence_sha256`, per-tuple `task8_native_evidence_sha256` | Task 9 persistence implementation; the Task 8 commit includes exact `Task7Task8IntegrationV1` |
| `task9_store_implementation_commit` | Task 10 handoff implementation |
| `task10_handoff_implementation_commit`, `task5c_mutation_v2_addendum_sha256`, `task5c_mutation_v2_self_audit_sha256`, `task5c_mutation_v2_independent_review_sha256` | Task5C-Mutation production implementation |
| `task5c_mutation_implementation_commit` | integrated feature acceptance/release |

No placeholder, branch name or mutable tag satisfies a ledger cell. A self-
audit and independent review must name distinct immutable files and digests; one
cannot satisfy the other. Product CI validates 64 versus 40 hex lengths, exact
Git object existence/type, rejects swapped/mislabelled values, and
must assert that Task 8 depends only on the separate immutable+implemented
`Task5C-Evidence` family, never on the combined working file, whole Task 5C or
`support.edit`; it must also assert that the Task5C-Mutation addendum/review and
implementation import the accepted Task 8 generic interfaces and are downstream
of Task 9/10 implementation. A phrase or build edge
making whole Task 5C GREEN a Task 8 prerequisite is a cycle and a hard STOP.

---

## 3. Exact file map

### Domain

- Create: `crates/unica-coder/src/domain/cfe_method_patch.rs`
  - closed interceptor/method/module values, identifiers, seed/material/write
    plans, stable encoders and immutable resolved plan.
- Modify: `crates/unica-coder/src/domain/mod.rs`.
- Verify/reuse the accepted Task 5A
  `crates/unica-coder/src/domain/discovery_registry.rs` implementation:
  exact metadata/module registries, `KnownScriptVariant`, domain-owned
  `BslExecutionContext`, canonical `PlatformUuid`, source-bound CFE companion
  facts and shared `ConfigurationFlavorV1`. Task 8 may add imports but must not
  smuggle a prerequisite semantic change into its commit.
- Create: `crates/unica-coder/src/domain/mutation_effects.rs`
  - schema-v3 NoChange/Committed/Uncertain state, unique
    `DefiniteTargetCommitV1`, separate current location/content/metadata and
    Updated replaced-prestate, possible publication-root/target relation,
    exact non-target effects, name-based Removed/ConsumedByPublication lifecycle,
    `VerifiedClean|Residue|Unknown` cleanup, independent
    `VerifiedDurable|Unknown` durability, recovery disposition, v3 tags and
    uninhabitable-state constructors.
- Create: `crates/unica-coder/src/domain/artifact_metadata.rs`
  - closed `PresentTargetMetadataV1`/`CreatedArtifactMetadataV1`, policy/version/
    expected/observed digests, one platform-neutral policy constructor and
    closed Unix/Windows tuple constructors plus private timestamp witness.
- Create: `crates/unica-coder/src/domain/artifact_writer_wal.rs`
  - exact bounded state tags/transitions/correlation records from §1.8; no I/O.
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
  - early routing support only; prepare; for apply open physical destination
    root, acquire artifact lease, recover WAL, refresh authoritative support/
    mapping/seed under that same lease, then perform
    authoritative capture/resolve; retain owned plan + lease and pass the same
    references through handler and future post-mutation seam; full CFE dry-run
    without lease.

### Discovery/back-propagated application contracts

- Verify/reuse accepted Task 7 v6+v7-addendum
  `crates/unica-coder/src/application/discovery/contract.rs`:
  domain interceptor/context types and already-optional Context/IsFunction.
- Modify: `crates/unica-coder/src/application/discovery/model.rs`
  - add only Task 8 resolved-plan/diagnostic integration; reuse accepted shared
    BSL context, source-bound adoption/Form facts and closed preflight tuple.
- Verify/reuse accepted Task 7
  `crates/unica-coder/src/application/discovery/proposal_validator.rs` exact
  Configuration mutation preflight. Task 8 may wire resolver blockers but may
  not redefine the tuple or make a prerequisite semantic edit.
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
- Verify/reuse the accepted Task 5B v7 Platform XML/parser/provider commit:
  shared catalog set with exact ScriptVariant/NamePrefix authorities, closed source-bound
  BaseOwn/Extension membership companions and complete V2 Form bindings under
  `PlatformFormBindingRegistryVersionV2`. Task 8 adds no parser/registry row and
  does not edit that upstream implementation.
- Verify/reuse the accepted Task 6 v2+v7-addendum implementation in
  `crates/unica-coder/src/infrastructure/discovery/bsl/{lexer,parser}.rs`:
  all retained extraction spans/line ending and bilingual observed
  Before/After/Around/ModificationAndControl facts. Task 8 adds no second lexer
  and does not fold prerequisite parser edits into its commit.
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
    prove the actual common lock inode/FileId, implement process/OS guards and
    run WAL recovery before returning an authoritative-capture-capable witness.
- Create: `crates/unica-coder/src/infrastructure/artifact_writer_wal.rs`
  - private dual-slot durable store, exact fsync/flush protocol, bounded
    intent-authorized staging recovery, corruption/orphan hard blocker and receipt-handoff
    seam; never public diagnostics/raw material.
- Create: `crates/unica-coder/src/infrastructure/contained_atomic_writer.rs`
  - same-mount control-staged cross-directory Present file replace, Absent file
    no-replace and complete absent-subtree no-replace install; closed Present/
    Created metadata synthesis and verification; WAL-driven lifecycle and
    target/control-parent durability; Linux ext4/XFS, both macOS APFS case-mode
    and Windows NTFS v2 gates. Windows is handle-relative and never uses
    `MoveFileExW`; its contained handle/durability primitives are reused later
    by Task 9.
- Do not route `crates/unica-coder/src/infrastructure/runtime_jobs.rs` through
  this contract merely to share a filename. Its existing scoped path-based
  atomic-file utility may remain behaviorally unchanged and is neither imported
  by CFE nor evidence for the contained writer. Any later unification is a
  separate migration with both suites GREEN.

### Tests/fixtures/docs

- Create: `tests/fixtures/cfe_method_patch/**`
  - registered Configuration analysis + adopted Extension destination
    Russian/English fixtures and Extension-analysis wrapper UUID decoys;
  - procedure/function parameters/defaults/async; owner/common/form ordinary;
  - accepted Task 5B v7 complete Form lookup Bound/Unbound and registry-version
    mismatch; same-name own and wrong-UUID decoys;
  - present/absent destination, absent parent suffix, Around duplicates,
    source and destination Task 5B complete auxiliary-catalog authority identities,
    misdeclared Configuration/Extension flavor, tracked base-with-compat and
    extension-without-compat/KeepMapping cases,
    unrelated-mapping/source/workspace-alias and APFS NFC/NFD/case-mode lease
    cases, retained-parent rename, Present ACL/xattr/ADS/hardlink/special-flag
    rejection, Created metadata inheritance, control-root metadata non-leakage,
    WAL corruption/orphans and external-race/effect-failure cases.
- Modify: `tests/ci/test_unica_mcp_script_parity.py`
  - retain semantic donor comparison for registered Russian Before/After but
    assert the intentional collision-free name/signature differences; invalid
    donor behavior is native rejection, not fake byte parity.
- Modify active spec/ADR/skill/product tests listed in §2.5.

### Explicitly out of Task 8 production scope

- public `unica.project.discover` registration;
- receipt persistence/revision, receipt lease and guard policy (the separate
  artifact mutation lease is explicitly in Task 8 scope);
- observation-journal storage/service implementation, public tool registration,
  provenance publication and release execution. This does **not** waive the
  mandatory Task 8 backprop of the schema-v3/WAL observability/replay contract or
  the exact backend support matrix/product-CI gates into active spec, ADR,
  package-contract documentation and tests listed in §2.5;
- implicit `cfe.borrow` or Form.xml event/callType mutation.

---

## 4. Domain contract

### 4.1 Reused and closed semantic values

Do not create `CfeScriptVariant`. Reuse:

```rust
// Exact upstream values; illustrative import paths are resolved by the
// accepted Task 5A/domain and Task 6 implementation. Task 8 redeclares neither.
use crate::domain::KnownScriptVariant;
use crate::infrastructure::bsl::BslExecutionContext;
```

Task 8 adds/moves:

```rust
pub(crate) enum CfeMutationClass { MethodPatch }

pub(crate) enum CfeInterceptorType {
    Before,
    After,
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
`ObservedCfeInterceptorKind` is the one shared type introduced with Task 6's
`BslSyntaxDefinition` in §2.3; Task 8 consumes it and does not redeclare or map
through a second enum.

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

`CfeIdentifier` is only a Task 8 type alias for the accepted Task 6
`BslIdentifierV1`; it adds no constructor or representation and remains
case-preserving:

```rust
pub(crate) type CfeIdentifier = BslIdentifierV1;
```

- constructible only through the exact accepted Task 6 v2+v7-addendum BSL
  identifier grammar and limits; Task 8 defines no `is_alphabetic`, XID or
  Russian/English approximation and cannot accept a spelling the shared lexer
  rejects;
- its local storage proves those shared byte/scalar ceilings fit the plan's
  bounded encoding, with the same N/N+1 fixtures rather than a second policy;
- no trim, normalization or rewrite of the retained spelling;
- equality/ordering and duplicate detection use Task 5A/domain's one comparison
  authority. For a method, the resolver first constructs the exact canonical
  module-qualified method `ArtifactRef` and only then obtains
  `ArtifactIdentityBytesV1`; a bare identifier is never passed as an artifact.
  That identity uses the shared
  `ArtifactRef` grammar with its
  scalar-by-scalar Unicode-lowercase comparison key. NFC/NFD/NFKC are not
  applied beyond that exact shared authority, so canonically equivalent but
  byte-distinct spellings remain distinct unless the service later changes
  under a version bump.

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
Source and generated method lookup targets are exact descendants of the
canonical module target and carry that module in their ArtifactRef; this is what
lets the V2 Form lookup reject a foreign FormModule instead of returning false
Unbound.
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
    analysis_configuration_artifact: ArtifactRef,
    destination_configuration_artifact: ArtifactRef,
    analysis_module_artifact: ArtifactRef,
    destination_module_artifact: ArtifactRef,
    analysis_owner_artifacts: Vec<ArtifactRef>,
    destination_owner_artifacts: Vec<ArtifactRef>,
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
Prepare watches only typed selected source roots/topology and never obtains,
constructs or formats a Form descriptor/Form.xml sidecar key. Full source
snapshot capture discovers the opaque sidecar entries and manifest keys;
resolve selects them by exact Form `ArtifactRef` and only then retains those
opaque keys in current plan/baseline. Missing, duplicated, foreign-source or
cross-side entries fail resolve, never widen prepare into material I/O.

### 4.5 Source method, adoption and write plan

```rust
pub(crate) struct CfeSourceMethodPlan {
    analysis_source_fingerprint: String,
    analysis_configuration_artifact: ArtifactRef,
    analysis_configuration_length: u64,
    analysis_configuration_digest: String,
    analysis_configuration_flavor: ConfigurationFlavorV1,
    analysis_script_variant: KnownScriptVariant,
    module_artifact: ArtifactRef,
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
}

pub(crate) struct BslByteRange {
    start_byte: u32,
    end_byte_exclusive: u32,
}

pub(crate) enum CfeFormBindingSafetyPlan {
    NotForm {
        semantic_digest: Digest32,
    },
    ManagedFormMethodsUnbound {
        registry_version: PlatformFormBindingRegistryVersionV2,
        analysis_document_flavor: PlatformFormDocumentFlavorV2, // exact Plain
        destination_document_flavor: PlatformFormDocumentFlavorV2, // exact Borrowed
        source_method_identity: ArtifactIdentityBytesV1,
        generated_method_identity: ArtifactIdentityBytesV1,
        analysis_unbound_proof_digest: Digest32,
        destination_unbound_proof_digest: Digest32,
        analysis_registered_form_authority_digest: RegisteredPlatformFormAuthorityDigestV1,
        destination_registered_form_authority_digest: RegisteredPlatformFormAuthorityDigestV1,
        analysis_descriptor_manifest_key: RegisteredFormManifestKeyV1,
        analysis_form_xml_manifest_key: RegisteredFormManifestKeyV1,
        destination_descriptor_manifest_key: RegisteredFormManifestKeyV1,
        destination_form_xml_manifest_key: RegisteredFormManifestKeyV1,
        analysis_material: VerifiedMaterialIdentity,
        destination_material: VerifiedMaterialIdentity,
    },
}

pub(crate) struct VerifiedMaterialIdentity {
    artifact: ArtifactRef,
    byte_length: u64,
    content_digest: String,
}

pub(crate) struct CfeAdoptedDestinationPlan {
    destination_source_fingerprint: String,
    descriptor_chain: Vec<AdoptedDescriptorAuthorityBindingV1>,
    configuration_artifact: ArtifactRef,
    configuration_length: u64,
    configuration_digest: String,
    configuration_flavor: ConfigurationFlavorV1,
    script_variant: KnownScriptVariant,
    name_prefix: BoundedNamePrefixV1,
    generated_method_name: CfeIdentifier,
}

pub(crate) struct AdoptedDescriptorAuthorityBindingV1 {
    analysis: BaseOwnedMetadataIdentityV1,
    destination: ExtensionMetadataMembershipV1,
    analysis_descriptor_material: VerifiedMaterialIdentity,
    destination_descriptor_material: VerifiedMaterialIdentity,
    semantic_join_digest: Digest32,
}

pub(crate) enum CfeModuleMaterialState {
    AbsentWatched {
        created_target_metadata: CreatedArtifactMetadataV1,
    },
    Present {
        byte_length: u64,
        content_digest: String,
        before_metadata: PresentTargetMetadataV1,
    },
}

pub(crate) struct CfeParentChainPlan {
    components: Vec<CfeParentComponentPlan>,
    creation_scope_start_index: u8,
    first_absent_index: Option<u8>,
}

pub(crate) struct CfeParentComponentPlan {
    path: String,
    state: CfeParentComponentState,
    created_metadata: Option<CreatedArtifactMetadataV1>,
}

pub(crate) enum CfeParentComponentState {
    PresentDirectory,
    AbsentDirectory,
}

pub(crate) struct CfeAllowedEffects {
    file_artifact: String,
    parent_directory_creation_scope: Vec<String>,
    metadata_policy_version: ArtifactMetadataPolicyVersion,
    metadata_policy_digest: Digest32,
    created_metadata_scope_digest: Digest32,
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
    publication: AtomicPublicationShapeV1,
    expected_target_metadata_digest: Digest32,
    expected_created_directory_metadata: Vec<(String, Digest32)>,
    append_boundary: CfeAppendBoundary,
    rendered_patch: Vec<u8>,
    rendered_patch_digest: String,
    expected_after_length: u64,
    expected_after_digest: String,
}

pub(crate) enum AtomicPublicationShapeV1 {
    PresentFileReplaceCrossDirectory,
    AbsentFileNoReplaceCrossDirectory,
    AbsentSubtreeNoReplaceCrossDirectory { first_absent_index: u8 },
}
```

`AdoptedDescriptorAuthorityBindingV1` records the exact accepted source-bound
Task 5A/v7 companions and their two verified descriptor materials. Its private
constructor accepts only `BaseOwnedMetadataIdentityV1` plus the Adopted variant
of `ExtensionMetadataMembershipV1`, requires equal pair/artifact roles and
analysis object UUID == destination extended-object UUID, recomputes one
domain-separated semantic join digest, and rejects Own/partial/plain UUID
payloads. Known Base/Extension flavor is already inside the companion authority;
the explicit plan flavor fields must agree. Root is always present; Form adds a
second binding in root-then-form order. Declared source kinds are checked
assertions, not substitutes for either material fact.

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

The vector authorizes effects; it does not authorize incremental mkdir. When
nonempty, `publication` is exactly `AbsentSubtreeNoReplaceCrossDirectory` and
the complete suffix plus target file is built below control staging, assigned
the per-component `CreatedArtifactMetadataV1`, verified and published in one
rename. When empty, publication is one control-staged file rename. The policy
version/digest and each expected metadata digest are constructor-checked against
§1.10; a missing or reordered directory metadata row cannot construct the plan.

`AdoptedDescriptorAuthorityBindingV1.semantic_join_digest` encodes the complete
source-free semantic payloads of the two accepted typed companions and their
exact pair/role join; it excludes descriptor bytes, source fingerprints and
evidence IDs. Analysis and destination configuration flavor tags remain
explicit immutable grant fields and must equal the companion facts.
`analysis_descriptor_material` and `destination_descriptor_material` bind the
current captured XML and participate only in execution-plan/current baseline.
The grant encoder consumes the semantic join, never rolling content/provenance.

Task 8 has no event/item/Action inspection outcome. A successful Form plan
requires two independent complete Task 5B v7 lookups under one accepted registry
version: analysis exact `MethodName` is `Unbound`, and destination canonical
generated Method `ArtifactRef` is `Unbound`.
`ManagedFormMethodsUnbound` stores the registry version and separate
query-specific Unbound proof digests plus exact material identities for both.
After each successful handle-bound lookup, its private constructor additionally
requires the analysis view's exact flavor
to be `Plain` and the adopted destination view's exact flavor to be `Borrowed`,
then stores both typed tags; any mismatch fails before lookup proof construction.
The two digests are returned only by one private smart constructor that consumes
the exact accepted `CompleteFormMethodBindingsV2View`, its current opaque
`RegisteredPlatformFormV1` handle, plus the exact
module-qualified Method `ArtifactRef`, calls the view, requires `Unbound`, and
rejects `Bound` or `InvalidFormMethodLookupV2`; callers cannot supply a detached
digest or opaque identity as lookup authority. It passes the handle directly to
`lookup_method`; the view internally derives/compares its private registered
binding before Method validation, so no caller-side digest equality can stand
in for authority. `RegisteredAuthorityMismatch` is inconclusive. Only after
successful lookup does it read the view's authority digest and construct the
same `ArtifactIdentityBytesV1` retained in the plan. Their
encoding reuses the accepted Task 5B v7 primitives and
`ArtifactIdentityBytesV1` bytes exactly:

```text
CFE_FORM_SAFETY_PROOF = "unica.cfe-form-safety-proof/v1"
CFE_FORM_UNBOUND_PROOF = "unica.cfe-form-unbound-proof/v1"

NotFormSemanticDigestV1 =
  H(CFE_FORM_SAFETY_PROOF,
    u16be(1))

FormUnboundProofDigestV1(role, form_owner, queried_method, registry) =
  H(CFE_FORM_UNBOUND_PROOF,
    u16be(role)                                      // analysis=1, destination=2
    || string(ARTIFACT_IDENTITY_ENCODER)             // exact artifact-identity/v1
    || ArtifactIdentityBytesV1(form_owner)
    || ArtifactIdentityBytesV1(queried_method)
    || u16be(1)                                      // Complete
    || u16be(1)                                      // Unbound
    || string("platform-form-bindings/v2"))           // accepted opaque value
```

`form_owner` is the canonical registered Form identity and `queried_method` is
its exact module-qualified Method identity; neither is a bare/display method
name. Checked length framing and `H` are exactly the accepted Task 5B v7 shared
encoder primitives. The private constructor compares the view's registry
version and relies on the successful handle-bound lookup for exact Form
ownership before hashing. Fixed byte-vector
fixtures cover both role tags, NotForm, a foreign module, Bound, a registry
version mismatch and cross-source/catalog/snapshot handle replay. Unrelated
binding ordering/content does not enter grant scope, while both complete
Form.xml content identities, registered-authority digests and whole catalog
semantic digests enter execution/baseline. For a non-Form target, the exact
`NotFormSemanticDigestV1` above is the only row. An absent/incomplete
destination Form.xml never becomes an empty binding set.

### 4.6 Final immutable plan

```rust
pub(crate) struct CfeArtifactMutationScope {
    canonical_artifact_locus: CanonicalArtifactLocusV1,
}

pub(crate) struct CfeCatalogSnapshotBindingV1 {
    configuration_catalog_set_digest: Digest32,
    registered_managed_form_catalog_set_digest: Digest32,
    analysis_configuration_catalog_digest: Digest32,
    destination_configuration_catalog_digest: Digest32,
    analysis_registered_managed_form_catalog_digest: Digest32,
    destination_registered_managed_form_catalog_digest: Digest32,
}

pub(crate) struct CfeMethodPatchPlan {
    version: u16, // 2: WAL + control publication + metadata authority
    core: ResolvedCfeMethodPatchCore,
    canonical_target: ArtifactRef,
    mutation_class: CfeMutationClass, // MethodPatch
    analysis: ResolvedSourceSet,
    destination: ResolvedSourceSet,
    source_method: CfeSourceMethodPlan,
    adopted_destination: CfeAdoptedDestinationPlan,
    catalog_snapshot_binding: CfeCatalogSnapshotBindingV1,
    artifact_mutation_scope: CfeArtifactMutationScope,
    allowed_effects: CfeAllowedEffects,
    snapshot_watch: SnapshotWatch,
    renderer_version: &'static str,
    writer_wal_schema: &'static str,
    mutation_outcome_schema: &'static str,
    atomic_backend_contract_version: &'static str,
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
stable encodings. `CfeCatalogSnapshotBindingV1` is constructible only from the
two accepted catalog sets after proving their exact UTF-8
`composite_snapshot_id` values equal, source coverage/order equal and every
selected Form-sidecar catalog's `configuration_catalog_digest` equals its
matching selected Configuration-catalog digest. Its six fields are recomputed
from the sets and the exact analysis/destination source-bound selections; no
caller supplies a detached digest. The two set digests already commit to the
same exact composite-snapshot ID, so the plan does not retain a second raw ID
string. This binding is rolling execution/baseline material and is excluded
from immutable grant scope.
No receipt stores `rendered_patch` or source text; it stores
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
Only accepted source-bound `BaseOwnedMetadataIdentityV1` companions from the
selected Configuration catalog provide object UUID authority for the adoption
join. The resolver never reads or compares an extension wrapper UUID or the
separate Configuration-root catalog UUID as a substitute object identity.

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
    pub(crate) artifact_lease: &'a ArtifactMutationLeaseWitnessV1,
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

pub(crate) struct ArtifactMutationLeaseCandidateV1 {
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
        candidate: &ArtifactMutationLeaseCandidateV1,
        receipt_recovery: &mut dyn ArtifactWriterReceiptHandoffPortV1,
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
    fn witness(&self) -> &ArtifactMutationLeaseWitnessV1;
}

pub(crate) struct ArtifactMutationLeaseWitnessV1 {
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
    created_directory_count: u8, // private ctor: 0..=MAX_CONTROL_INIT_DIRS_V1
    lock_file_created: bool,
}

pub(crate) const MAX_CONTROL_INIT_DIRS_V1: u8 = 16;

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
registration/parent paths plus `ArtifactMutationLeaseCandidateV1` without reading
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

Every acquisition receives an operation-neutral receipt-recovery port even for
    a NotRequired current call, because a prior Required Prepared/Terminal may be
the selected WAL. The port is invoked only after the artifact lock is held and
acquires/reuses the exact correlated receipt lease in artifact->receipt order.
If no prior Required state exists, an unavailable/no-op recording port is never
called. If one does exist and Task 10 production authority is unavailable, the
WAL remains blocking before capture; a current NotRequired handoff is not permission to
skip an older handoff. Task 8 provides only recording/failure fakes; Task 10
later provides the persistent production port.

`WorkspaceDiscoveryControlRootV1` and `ArtifactMutationLeaseWitnessV1` have no
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
failure to prove this one root is `artifact_writer_lock_failed`; there is no fallback
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
present in the plan/grant tuple. This does not erase schema-v3 outcome,
metadata, target and control-staging identity digests required for authority. Raw
workspace/artifact text never becomes a filename. Directories and the lock
inode are created with restrictive permissions, link/reparse rejection and
create-new/open-existing retry; the inode is persistent. Task 9 receipt locks
use exact sibling `locks/receipts/<receiptIdentityDigest>.lock`, never the
artifact namespace, and records use
`<control-root>/receipts/<receiptIdentityDigest>.receipt`; final components are
64 lowercase hex and the locked record revalidates its full primary key/digest.
Different `UNICA_CACHE_DIR`, unrelated
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
`MutationHandlerOutcome` and cannot be converted to `MutationEffectsV3`.
The exact v1 fixed namespace, including the selected collision bucket and Task
9 sibling directories, fits `MAX_CONTROL_INIT_DIRS_V1=16`; code may not
saturate/wrap the counter or silently create a seventeenth layout component—a
layout expansion is a version/review change.
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
one physical artifact lease, recovers its WAL before capture, performs one
authoritative support/topology refresh under that same lease, and then exactly one
authoritative capture+resolve from the refreshed seed. Other descriptors return
None. A failed acquire returns `artifact_writer_busy` or
`artifact_writer_lock_failed`; recovery/corruption returns its exact generic
writer reason; an internal mount/control boundary returns its exact
unsupported reason, an unqualified backend returns
`artifact_writer_backend_unsupported`, and a changed refreshed root/locus
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
validate raw/workspace/path -> early routing support check only
prepare (zero material reads)
open workspace once -> same-mount/volume + qualified backend proof
physical control/destination identities -> semantic key + root-wide collision universe
actual common collision-lock inode/lease
recover artifact WAL under lease; stop/retry or fail closed before capture
refresh topology + authoritative operation support/policy under the same lease
authoritative watched capture of refreshed seed -> resolve current immutable plan
discovery guard -> optional current receipt lease + revision/baseline reread
final same-watch/content/metadata precondition recapture
Prepared WAL with correlation
Required handoff: durable correlated receipt InFlight proof for Prepared generation
Staging/Install WAL -> one control-to-source rename -> typed outcome -> Terminal WAL
post snapshot -> durable receipt baseline-revalidated clear / advance / revoke
Terminal handoff ack -> eligible durable Idle tombstone, else blocking Ack
drop current receipt lease
drop artifact mutation lease
events -> cache work -> other-receipt reconciliation -> result
```

Task 8 implements artifact open/lease/WAL recovery, under-witness authoritative
support refresh, capture/resolve/final-check, generic writer/Terminal/ack seam,
typed outcome and post-snapshot path plus recording seams at receipt positions.
Receipt persistence/lease/transition/reconciliation and production guard are
owned by Task 9/10 v6 addenda, not Task 8 production code.

Receipt validation after acquisition must compare its reread baseline with the
already captured plan; a concurrent baseline advance produces a mismatch and
zero handler calls. Every applied CFE call follows this order, including calls
allowed without a receipt. No path may acquire an artifact lease while holding
a receipt lease, a second artifact lease while holding a receipt lease, or a
second receipt lease while either current lease is held. Authoritative support/
policy cannot be read only before the lease; CFE and `support.edit` reuse this
same witness through writer/outcome/handoff and never acquire a second one. V1 plans exactly one
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
    configuration_catalog_set: PlatformConfigurationCatalogSetV1,
    registered_form_catalog_set: RegisteredManagedFormCatalogSetV1,
    analysis_configuration_catalog_digest: Digest32,
    destination_configuration_catalog_digest: Digest32,
    analysis_registered_managed_form_catalog_digest: Digest32,
    destination_registered_managed_form_catalog_digest: Digest32,
    analysis_configuration_material: VerifiedMaterialIdentity,
    destination_configuration_material: VerifiedMaterialIdentity,
    analysis_module: VerifiedBslMaterial,
    analysis_memberships: Vec<BaseOwnedMetadataIdentityV1>,
    analysis_form_bindings: Option<VerifiedRegisteredFormBindingsV2>,
    destination_memberships: Vec<ExtensionMetadataMembershipV1>,
    destination_form_bindings: Option<VerifiedRegisteredFormBindingsV2>,
    destination_target: WatchedMutationTargetState,
    destination_module: OptionalVerifiedBslMaterial,
}

pub(crate) struct VerifiedRegisteredFormBindingsV2 {
    identity: VerifiedMaterialIdentity,
    descriptor_manifest_key: RegisteredFormManifestKeyV1,
    form_xml_manifest_key: RegisteredFormManifestKeyV1,
    registered_form: RegisteredPlatformFormV1,
    registered_authority_digest: RegisteredPlatformFormAuthorityDigestV1,
    document_flavor: PlatformFormDocumentFlavorV2,
    value: CompleteFormMethodBindingsV2,
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
constructs each `RegisteredPlatformFormV1` only from the borrowed catalog-set
entry plus that same verified material, parses through the neutral V2 parser,
then retains the exact handle, result and view-provided opaque authority digest
in `VerifiedRegisteredFormBindingsV2`. Exact handle/binding equality is enforced
inside every later `lookup_method(handle, method)` call, never by a detached
caller digest comparison. The port
also requires the captured catalog flavor to agree with declared source kind;
it never accepts the topology label as the parsed value.
`CfeResolutionMaterial` has a private constructor that selects and borrows the
exact analysis/destination Configuration and registered-Form catalogs by
source-bound key from the two accepted same-snapshot catalog sets, requires
identical source coverage/order and exact sidecar-to-Configuration catalog
digest binding, recomputes all four selected catalog digests plus both set
digests, constructs the sole `CfeCatalogSnapshotBindingV1`, and verifies every
membership fact, Form sidecar entry and
Configuration/Form material identity against those entries and the same
composite snapshot. ScriptVariant/NamePrefix are read from the borrowed Configuration
catalog authorities. Callers cannot supply detached catalog copies, properties
wrappers, inferred Form paths or plain UUID/membership strings.

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
- after exact current registered-handle/view-authority equality, a Task 5B
  complete V2 lookup `Bound` rejects as wrong mechanism; only `Unbound` under
  that same accepted Task 5B v7 registry version and snapshot binding can
  classify the exact method as an unregistered FormModule method;
- for a Form target the same catalogs must report analysis document flavor
  `Plain` and destination document flavor `Borrowed`, agreeing with the
  Base+Own / Extension+Adopted companion chain before either Unbound proof is
  accepted;
- for the same Form target, a second Complete destination binding proof must
  find no parser-canonical handler equal to the derived generated method name;
  an existing orphan XML binding is a collision even when the destination BSL
  definition is absent;
- any Task 5B registry/parser/resource gap, registered-authority mismatch or incomplete catalog fails closed as
  `cfe_form_binding_inconclusive`, never Ordinary; Task 8 does not reinterpret
  the underlying Form row.

Case-fold-only method match is not silently rewritten. Annotation target and
canonical ArtifactRef use the exact source spelling; a different raw spelling
fails `cfe_source_method_spelling_mismatch`.

### 9.4 Exact adopted chain

For every root/form pair in canonical order:

1. analysis is declared Configuration + PlatformXml, its captured catalog is
   exact `BaseConfiguration`, and its complete pair half contains
   `MetadataPresent` plus exact `BaseOwnedMetadataIdentityV1` for the requested owner;
2. every analysis descriptor authority is therefore exact Base+Own, never a
   promoted plain `MetadataIdentity`;
3. destination is declared Extension + PlatformXml and its captured catalog is
   exact `ExtensionConfiguration`; existence polarity remains independent from
   membership;
4. a present destination descriptor must have one successful closed
   `ExtensionMetadataMembershipV1` companion in the same complete pair half;
5. only its Adopted variant can construct an
   `AdoptedDescriptorAuthorityBindingV1`;
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
Adopted analysis root/Form is `analysis_not_base_owned`, while a destination
catalog without exact Extension flavor is `destination_not_extension_flavor`. These are
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

Use the selected `PlatformConfigurationCatalogV1` authorities already projected
by the accepted Task 5B v7 exact-namespace parser from direct
Configuration/Properties nodes and bound to the catalog/source/material:

- destination `script_variant_authority` must be
  `CatalogScriptVariantAuthorityV1::Known(Russian|English)`;
- destination `name_prefix_authority` must be
  `CatalogNamePrefixAuthorityV1::Value(BoundedNamePrefixV1)` and nonempty by
  that smart constructor;
- generated method name is deterministic and has no user override:

| Interceptor | Russian destination | English destination |
| --- | --- | --- |
| Before | `NamePrefix + MethodName + "Перед"` | `NamePrefix + MethodName + "Before"` |
| After | `NamePrefix + MethodName + "После"` | `NamePrefix + MethodName + "After"` |
| ModificationAndControl | `NamePrefix + MethodName` | `NamePrefix + MethodName` |

- the complete generated name must pass `CfeIdentifier`; overflow or an
  invalid concatenation is `cfe_generated_method_name_invalid`;
- every non-usable closed authority maps without collapsing its semantic
  state:

| Authority state | Leading Task 8 reason | Bounded detail |
| --- | --- | --- |
| `CatalogNamePrefixAuthorityV1::Missing` | `cfe_name_prefix_missing` | none |
| `CatalogNamePrefixAuthorityV1::Empty` | `cfe_name_prefix_empty` | none |
| `Inconclusive(DuplicateExactField)` for NamePrefix | `cfe_name_prefix_duplicate` | exact problem tag |
| `Inconclusive(WrongNamespace\|MixedContent\|InvalidOrOverLimit)` for NamePrefix | `cfe_name_prefix_invalid` | exact problem tag |
| `CatalogScriptVariantAuthorityV1::Missing` | `cfe_script_variant_missing` | none |
| `CatalogScriptVariantAuthorityV1::Unknown(token)` | `cfe_script_variant_unknown` | bounded token digest, never raw token |
| `Inconclusive(DuplicateExactField)` for ScriptVariant | `cfe_script_variant_duplicate` | exact problem tag |
| `Inconclusive(WrongNamespace\|MixedContent\|InvalidOrOverLimit)` for ScriptVariant | `cfe_script_variant_invalid` | exact problem tag |

  No row falls back to `Расш_` or Russian;
- analysis catalog ScriptVariant authority must also be Known and is recorded as source
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
inserting each proposed fact, in both orders. Comparisons use the shared Task
5A/domain `ArtifactIdentityBytesV1` Unicode case identity. Comments, strings,
dates and deleted blocks do not create
duplicates. Unknown unrelated annotations remain gaps only when they affect the
target/definition; no regex or substring scan.

The destination Form binding lookup uses the same complete Task 5 catalog and
domain-owned canonical identifier identity as source role classification. A
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
are 0/1; enums use their declared stable tags and `BslLineEnding` is
Lf=1/Crlf=2/Cr=3. No JSON/debug/serde order,
platform separator, pointer address or enum discriminant participates.

Three resolver digests have different jobs and MUST NOT be conflated; §1.8/1.9
add independent WAL-record/outcome/definite-commit digests:

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
   Plain and destination Borrowed document-flavor tags, analysis
   source-method-unbound semantic digest and destination
   generated-name-unbound semantic digest when applicable;
8. BaseConfiguration/ExtensionConfiguration flavor tags, exact Own analysis
   root/form identities and exact adopted destination UUID tuples;
9. analysis/destination KnownScriptVariant, destination NamePrefix and
   generated method identity;
10. `ArtifactMetadataPolicyVersion::V1`, platform-neutral metadata-policy digest
    and stable Created metadata synthesis scope; no current Present/Absent tag,
    before-metadata or synthesized per-object expected digest;
11. writer WAL schema, mutation outcome schema, v2 atomic backend contract IDs
    (not an apply-only qualified tuple/evidence digest), resolver and renderer
    versions.

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

1. exact `CfeCatalogSnapshotBindingV1` fields in declaration order, then
   analysis source fingerprint and analysis Configuration
   artifact/length/content digest;
2. source module length/content digest; definition, declaration, name,
   parameter-list, body and terminator ranges in that exact order; declaration
   line-ending tag; definition/signature/body digests;
3. destination source fingerprint;
4. destination Configuration length/content digest;
5. analysis/destination descriptor artifact content digests;
6. when the target is Form, analysis then destination each encode the two exact opaque
   `RegisteredFormManifestKeyV1` canonical values (descriptor then Form.xml),
   Form-binding artifact/length/content digest,
   `RegisteredPlatformFormAuthorityDigestV1`, parser-derived document-flavor
   tag and complete binding-catalog semantic digest in that exact nested order;
7. canonical SnapshotWatch encoding, every parent state and artifact outcome;
8. creation-scope start index, first-absent index and current exact
   `directories_to_create` vector;
9. publication-shape tag and per-component expected Created metadata digests;
10. destination module material tag/length/digest plus Present before-metadata
    or Absent target expected-metadata digest;
11. append-boundary tag;
12. rendered patch length/digest;
13. expected final module length/digest.

It proves exact current preflight/rendered execution. It is used for same-call
identity, audit/diagnostics and precondition verification, not as the immutable
key of a remaining multi-grant receipt after another grant advances baseline.
Opaque Form manifest-key bytes are hashed only inside this private execution
digest/WAL authority and retained for exact recapture lookup; they never enter
grant scope, public diagnostics, receipt display or path reconstruction.

### 11.5 Exclusions and digest tests

Excluded from all three: absolute workspace/cwd, dryRun, confirm, alias
spelling, receipt/proposal/analysis IDs, task/source text, wall-clock timestamps,
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
- replaying an analysis/destination Form catalog under another registered-form
  handle, source fingerprint, configuration catalog digest or verified content
  digest cannot construct a plan; each exact current authority digest changes
  execution/baseline but not a still-identical Unbound grant proof;
- changing either catalog-set digest or any one of the four exact selected
  catalog digests changes execution/baseline but not grant scope; mismatched
  composite-snapshot IDs, source coverage/order or sidecar-to-Configuration
  digest binding cannot construct `CfeCatalogSnapshotBindingV1`;
- only current destination module content changes execution plan, not grant
  scope, while immutable fields stay equal;
- provider/file/source ordering and cwd do not change digests;
- stable parent creation scope changes scope+execution; current parent states or
  `directories_to_create`, target present/absent, boundary/rendered/final change
  execution only;
- metadata policy/version or stable Created synthesis scope changes grant+
  execution; current Present before-metadata, parent state/publication shape and
  exact per-object expected Created metadata change execution/baseline only;
  every effect/WAL digest round-trips the same policy/expected digest and an
  omitted directory row is rejected;
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
analysis source-method-unbound + destination generated-name-unbound Form proof
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
    pub(crate) artifact_lease: Option<&'a ArtifactMutationLeaseWitnessV1>,
    pub(crate) artifact_writer_handoff: Option<ArtifactWriterReceiptHandoffV1<'a>>,
}

pub(crate) struct HandlerOutcome {
    pub(crate) adapter: AdapterOutcome, // presentation only
    pub(crate) job: Option<Value>,
    pub(crate) mutation: Option<MutationHandlerOutcome>,
}

pub(crate) const MUTATION_OUTCOME_SCHEMA_V3: &str = "unica.mutation-outcome.v3";
pub(crate) const MAX_MUTATION_EFFECTS: usize = 32;
pub(crate) const MAX_OWNED_STAGING: usize = 1;

pub(crate) enum MutationHandlerOutcome {
    NoChange {
        cleanup: MutationCleanupState,
        source_tree: NoChangeSourceTreeProof,
        recovery: MutationRecoveryDispositionV1,
    },
    Committed {
        target: DefiniteTargetCommitV1,
        effects: MutationEffectsV3,
        cleanup: MutationCleanupState,
        durability: MutationDurabilityState,
        recovery: MutationRecoveryDispositionV1,
    },
    Uncertain {
        possible_target: PossibleTargetInstallV1,
        effects: MutationEffectsV3,
        cleanup: MutationCleanupState,
        recovery: MutationRecoveryDispositionV1,
    },
}

pub(crate) struct MutationEffectsV3 {
    expected_non_target: Vec<TypedNonTargetMutationEffectV3>,
    unexpected: Vec<TypedNonTargetMutationEffectV3>,
    possible: Vec<TypedNonTargetMutationEffectV3>,
}

pub(crate) struct PhysicalFilesystemObjectIdentity {
    volume_identity_digest: String,
    object_identity_digest: String,
}

pub(crate) enum TypedNonTargetMutationEffectV3 {
    CreatedDirectory {
        intended_artifact: String,
        object: PhysicalFilesystemObjectIdentity,
        location: CurrentTargetLocationV1,
        metadata: CurrentTargetMetadataV1,
    },
}

pub(crate) struct PossibleTargetInstallV1 {
    install_kind: DefiniteTargetMutationKind,
    intended_artifact: String,
    staged_publication_root: PhysicalFilesystemObjectIdentity,
    staged_target: PhysicalFilesystemObjectIdentity,
    target_relation: PlannedPublicationTargetRelationV1,
    retained_destination_parent_identity_digest: Digest32,
    expected_content_digest: Digest32,
    expected_metadata_digest: Digest32,
    observation: PossibleInstallObservationV1,
}

pub(crate) enum PlannedPublicationTargetRelationV1 {
    TargetIsPublicationRoot, // tag 1
    TargetIsDescendant {     // tag 2
        target_relative_components_digest: Digest32,
    },
}

pub(crate) enum PossibleInstallObservationV1 {
    StagingRootOnlyButIdentityUnqueryable,      // tag 1
    DestinationRootOnlyButIdentityUnqueryable, // tag 2
    BothRootNamesOrConflictingIdentity,         // tag 3
    NeitherRootNameOrQueryFailed,               // tag 4
}

pub(crate) struct VerifiedClean {
    staging_lifecycle: Vec<VerifiedStagingLifecycle>,
}

pub(crate) struct OwnedStagingIdentity {
    private_control_name_digest: Digest32,
    retained_parent_identity_digest: String,
    object: PhysicalFilesystemObjectIdentity,
}

pub(crate) enum VerifiedStagingLifecycle {
    Removed {
        staging: OwnedStagingIdentity,
    },
    ConsumedByPublication {
        staging: OwnedStagingIdentity,
        relation: PublicationTargetRelationV1,
        definite_target_commit_digest: Digest32,
    },
}

pub(crate) enum PublicationTargetRelationV1 {
    TargetIsPublicationRoot { // tag 1; Present/Absent file publication
        installed_root: PhysicalFilesystemObjectIdentity,
    },
    TargetIsDescendant { // tag 2; Absent subtree publication
        installed_root: PhysicalFilesystemObjectIdentity,
        target_relative_components_digest: Digest32,
        target: PhysicalFilesystemObjectIdentity,
    },
}

pub(crate) enum MutationCleanupState {
    VerifiedClean(VerifiedClean),
    Residue {
        otherwise_clean: Vec<VerifiedStagingLifecycle>,
        owned_staging: Vec<OwnedStagingIdentity>, // private ctor: nonempty
    },
    Unknown {
        otherwise_clean: Vec<VerifiedStagingLifecycle>,
        possible_staging: Vec<OwnedStagingIdentity>, // private ctor: nonempty
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
}

pub(crate) enum AtomicMutationBackendContract {
    LinuxContainedAtomicV2 { qualified_tuple_digest: String },
    MacOsContainedAtomicV2 { qualified_tuple_digest: String },
    WindowsContainedAtomicV2 { qualified_tuple_digest: String },
}

pub(crate) enum DurabilityUnknownStage {
    ControlStagedDataFlush,
    ControlStagedTreeMetadataSync,
    SourceReceivingParentMetadataSync,
    ControlSourceParentMetadataSync,
    InstalledTargetFlush,
    PublishedSubtreeMetadataSync,
    BackendObservation,
}

pub(crate) enum MutationRecoveryDispositionV1 {
    Normal,
    RecoveredNoInstall { prior_wal_generation: u64 },
    RecoveredDefiniteInstall { prior_wal_generation: u64 },
    RecoveredPossibleInstall { prior_wal_generation: u64 },
}
```

`call_tool()` owns `Option<ResolvedMutationCall>` for the full call. Future
guard and handler receive `plan` by the same reference; applied CFE additionally
receives the witness borrowed from the still-owned lease guard and consumes the
single current-call handoff value constructed by Task 10 only after its guard
decision. Neither plan nor witness is cloned, serialized or rebuilt; the
handoff cannot be cloned or retained past the call. Constructors require
Applied CFE to have plan+witness+exactly one `Some(Required|NotRequired)`, require
Preview CFE to have the plan but no witness/handoff, and require non-CFE to have
none of plan/witness/handoff. Fake tests record plan address, witness address,
grant-scope and execution digest at guard, handler and post-mutation seams; an
equal reconstructed object fails identity testing and a missing/duplicated/
mode-derived handoff never reaches the writer.

An applied CFE invocation must return `Some(mutation)` regardless of
`AdapterOutcome.ok`. `None` is valid only for non-mutating preview or legacy
non-CFE handlers not yet migrated. The writer's typed apply result has no
untyped `Result::Err`; dropping its mutation field after durable `Prepared`
would discard WAL/outcome authority. A proven pre-install error becomes typed `NoChange` with
source `Unmodified` and its exact cleanup state; only a clean one can tombstone.
Fixed control-plane initialization is internal lease telemetry, but control
staging/WAL remain exact recovery state and may not be omitted from cleanup.
Constructors make state semantics uninhabitable-by-mistake:

- `NoChange::new` accepts no source-effect vectors and only `Unmodified`.
  Its recovery disposition is exactly `Normal` for the current transaction or
  `RecoveredNoInstall` for prior-WAL recovery; the other recovered tags are
  rejected.
  Its `VerifiedClean` may contain only `Removed` rows and additionally requires
  durable control removal plus parent flush; an empty row set proves the
  retained staging bucket was enumerated empty and no request staging name was
  ever created. A failed/query-unknown cleanup or required parent sync instead constructs
  complete bounded `Residue`/`Unknown`, stays non-Idle and forces receipt
  revocation; it never becomes `Uncertain`, because install is proven absent.
  Every NoChange cleanup constructor rejects `ConsumedByPublication` in both clean
  and `otherwise_clean` rows and binds the complete WAL-owned staging set.
  Because source has no pre-install staging names or incremental mkdir, source
  rollback is not a Task 8 state;
- `Committed::new` requires exactly one `DefiniteTargetCommitV1`. Cleanup and
  durability remain independent. Location/content/metadata Unknown or Mismatch
  are legal constructor inputs and force nonadvance/revoke; they never change
  definite install into `Uncertain`. Its recovery disposition is exactly
  `Normal` or `RecoveredDefiniteInstall`;
- `Uncertain::new` requires exactly one bounded `PossibleTargetInstallV1` and
  install completion not proven. It cannot accept a definite target commit.
  Its recovery disposition is exactly `Normal` or
  `RecoveredPossibleInstall`.
  File publication requires equal staged root/target with relation tag 1;
  subtree publication requires distinct directory root/file target plus the
  exact plan-bound descendant digest with relation tag 2. An untyped single
  `staged_object` cannot represent both and is rejected.
  Every definite/possible created-directory and control-staging observation is
  retained. Every recovered variant requires a nonzero generation lower than
  the Terminal generation that records it; `Normal` carries none. Cross-outcome
  recovery tags, generation zero/future/equal and a recovered tag on a fresh
  no-prior-WAL path are unconstructible.

Cleanup is a top-level control-name lifecycle, not global object extinction.
`VerifiedClean` structurally contains every WAL-owned staging-root name created
by the call exactly once and in exactly one closed state. For NoChange its
private constructor also requires the explicit-removal control-parent sync;
for Committed the independent `MutationDurabilityState` proves or fails the
publication/control-parent syncs. Nested staged objects are bound by
`StagingReady`, target/non-target effects and the publication relation; they are
not incorrectly modelled as disappearing names under their moved parent:

- `Removed`: the retained control-parent staging name is observed absent through
  the retained capability and that removal is WAL-accounted;
- `ConsumedByPublication`: the retained control-parent staging-root name is
  proven absent. For file publication, `TargetIsPublicationRoot` requires the
  staging root, installed publication root and unique target commit identities
  all equal. For absent-subtree publication, `TargetIsDescendant` requires the
  installed root to equal the staged directory root and a retained-handle walk
  at the exact plan-bound relative-component digest to resolve the unique
  target commit identity. The target's current path may later be
  AtIntendedPath, DetachedOrRelocated or Unknown; observation uncertainty is
  not itself a cleanup gap.

Thus normal Present rename and Absent no-replace install can be clean while the
installed target still has staging identity `T`. `VerifiedClean` is forbidden
if any owned staging name remains, any extra owned staging object/name is known
or possible, the retained-parent name state cannot be queried, one created
staging identity is missing/duplicated, a
`ConsumedByPublication` row does not resolve through its exact root/descendant
relation to the unique definite target commit, or
two lifecycle rows use the same name/object. Its vector may be empty only when
no staging object was created; otherwise the complete union of clean and
unresolved lifecycle rows has exactly one entry and is bounded by
`MAX_OWNED_STAGING=1`. A second collision-bucket sibling is an orphan, not a
second owned row. An unauthorized descendant makes the selected root
non-deletable/recovery-required and cannot be relabelled as owned authority.

`definite_target_commit_digest` is the exact §1.9 domain-separated encoding.
Other v3 stable tags are: outcome NoChange=1/Committed=2/Uncertain=3; cleanup
VerifiedClean=1/Residue=2/Unknown=3; lifecycle Removed=1/ConsumedByPublication=2;
durability VerifiedDurable=1/Unknown=2; non-target CreatedDirectory=1;
NoChange source-tree proof Unmodified=1; backend contract Linux=1/macOS=2/
Windows=3; recovery Normal=1/RecoveredNoInstall=2/
RecoveredDefiniteInstall=3/RecoveredPossibleInstall=4. Durability-unknown stage
tags follow declaration order 1..7. Constructors recompute all digests, require
the publication root to equal the staging root, require relation tag 1 for file
publication and tag 2 plus exact descendant proof for subtree publication,
require the relation target to equal the unique target commit, and reject target
duplication in effects.
Every schema-v3 `*_digest` string is accepted only through a private constructor
that decodes exactly 64 lowercase-hex ASCII characters to 32 bytes (or receives
the internal 32-byte value before encoding); arbitrary text, wrong domain/length/case and display
digests cannot enter persisted authority.

The complete persisted/correlated outcome digest is not the target digest and
cannot erase a dirty NoChange cleanup:

```text
mutationOutcomeDigestV3 = SHA256(
  "unica.mutation-outcome.v3\0" || outcomeTag || outcomePayload)

NoChange payload =
  cleanupCanonicalV3 || sourceTreeProofTag || recoveryCanonicalV1
Committed payload =
  lpBytes(definiteTargetCommitCanonicalV1) || effectsCanonicalV3 ||
  cleanupCanonicalV3 || durabilityCanonicalV1 || recoveryCanonicalV1
Uncertain payload =
  lpBytes(possibleTargetInstallCanonicalV1) || effectsCanonicalV3 ||
  cleanupCanonicalV3 || recoveryCanonicalV1
```

`lpBytes` is `u64be(length) || bytes`. Each struct canonical encoding follows
its declaration order; every String is `lp(UTF-8)`, each `Digest32` is raw 32
bytes, each physical identity is `lp(volume-digest) || lp(object-digest)`, and
each vector is `u64be(count)` followed by its already sorted/unique rows.
`effectsCanonicalV3` is expected, unexpected, possible in that order; a
directory row is tag 1 then intended artifact, physical identity, location and
metadata. `cleanupCanonicalV3` begins with cleanup tag 1/2/3 and then, in field
order, all clean lifecycle rows followed by the nonempty unresolved rows;
lifecycle rows use Removed=1/ConsumedByPublication=2 then private-name digest,
retained-parent digest, staging-root physical identity, relation tag and its
root/optional-descendant fields, then the raw 32-byte definite target digest.
`durabilityCanonicalV1` uses its state/backend/stage tags plus
qualified-tuple/proof digests in declaration order. `recoveryCanonicalV1` uses
its tag plus `u64be(prior_wal_generation)` when present.
`possibleTargetInstallCanonicalV1` follows its declaration order and uses
relation TargetIsPublicationRoot=1/TargetIsDescendant=2 and observation
StagingRootOnlyButIdentityUnqueryable=1,
DestinationRootOnlyButIdentityUnqueryable=2,
BothRootNamesOrConflictingIdentity=3, NeitherRootNameOrQueryFailed=4. The canonical
definite-target bytes are exactly the §1.9 field order used by its digest,
including Updated before-content digest. Unknown variants emit no invented
observation bytes. Terminal WAL, Task 9 persistence, receipt correlation,
replay and observability must compare this exact complete digest and closed
payload, never only `definite_target_commit_digest` or display JSON.

`Residue` and `Unknown` do not forge a clean lifecycle for unresolved rows.
Their `otherwise_clean` rows account every other staging identity; their
nonempty `owned_staging`/`possible_staging` rows are the complete WAL-owned
control-staging set; these private control identities are not grant-authorized
source effects and are redacted from public diagnostics.
Known residue after an Absent install may reference the same object as the
definite target but still records the extra staging **name**. An owned-staging
row is never expected. Live retained-parent absence is sufficient for the
structural cleanup dimension even when the later namespace sync fails:
Committed cleanup may be `VerifiedClean` while mutation durability is `Unknown`,
but that pair remains a blocking non-Idle WAL. A
remaining name is `Residue`; an unqueryable name state is cleanup `Unknown`.
Target location/content/metadata observation does not itself choose cleanup.

`MutationDurabilityState` is independent **definite target/allowed-parent**
authority. `VerifiedDurable` is constructible only by an allowlisted backend
after control-staged data/tree metadata, installed target, source receiving
parent, control staging source parent and every moved-subtree directory
durability primitive required by that exact tuple have succeeded and the proof
transcript matches `backend_contract`. Cross-directory rename modifies two
parent namespaces; neither parent sync can stand in for the other. Cleanup is
derived from retained control-parent name/identity observation; durability is
never inferred from that live observation.
A failure or unavailable target-durability proof
after a definite target commit returns `Committed + Unknown { stage }`; it is
never cleanly advanceable and never downgraded to `Uncertain`. A failure before
target commit follows NoChange/Uncertain effect rules.

NoChange has no target-durability variant: source is `Unmodified`; its cleanup
independently says VerifiedClean, Residue or Unknown. Only its stricter
VerifiedClean constructor proves control staging removal plus control-parent
sync and permits tombstoning. Committed additionally requires VerifiedDurable
before tombstoning. If
source install completion is ambiguous, the only legal result is `Uncertain`;
if source install is definite, it is `Committed`. A failed control cleanup is
typed NoChange+Residue/Unknown, revoking and WAL-blocking, never a clean retry.

`MutationEffectsV3.expected_non_target` and `.unexpected` contain only definite
created-directory effects; `.possible` contains only non-proven non-target
effects. Each directory carries physical identity, current location and exact
metadata observation; only AtIntendedPath+VerifiedExact may be expected. The
unique target is never in these vectors. Constructors keep vectors sorted/
unique, reject a duplicate identity/path across categories, and require the sum
of all three vector lengths to be `<= MAX_MUTATION_EFFECTS=32`; control names and raw physical
identities never enter public diagnostics, only domain-separated digests.

At discovery issuance one plan object reaches issuer assessment for that call.
A later apply necessarily builds a fresh current plan from a fresh snapshot;
pointer identity is not expected across calls. Cross-call authority is the
atomic grant tuple/scope digest plus current rolling baseline.

### 12.3 Generic reusable artifact-writer boundary

Task 8 exports one operation-neutral interface; CFE is its first caller and Task
5C/later writers import it rather than defining another path protocol:

```rust
// Reuse the exact §8.1 ArtifactMutationLeaseCandidateV1 and
// ArtifactMutationLeaseWitnessV1; a second definition is forbidden.

pub(crate) enum ControlStagedPublicationV1 {
    PresentFileReplaceCrossDirectory,
    AbsentFileNoReplaceCrossDirectory,
    AbsentSubtreeNoReplaceCrossDirectory { first_absent_index: u8 },
}

pub(crate) enum ArtifactMetadataPolicyV1 {
    Present {
        target: PresentTargetMetadataV1,
        content_timestamp: ContentChangeTimestampPolicyV1,
    },
    Created {
        target: CreatedArtifactMetadataV1,
        directories: Vec<CreatedArtifactMetadataV1>,
    },
}

pub(crate) struct ArtifactWriterPlanV1 {
    publication: ControlStagedPublicationV1,
    target: ArtifactWriterTargetV1,
    created_directories: Vec<CreatedArtifactPlanV1>,
    content: VerifiedPlannedContentV1,
    metadata: ArtifactMetadataPolicyV1,
    authoritative_policy_digest: Digest32,
    witness_generation_digest: Digest32,
    plan_digest: Digest32,
}

pub(crate) struct ReceiptHandoffCorrelationV1 {
    receipt_identity_digest: Digest32,
    expected_revision: u64,
    handoff_nonce: [u8; 16],
    plan_digest: Digest32,
    grant_digest: Digest32,
    baseline_digest: Digest32,
}

pub(crate) struct ReceiptInFlightRequestV1 {
    correlation: ReceiptHandoffCorrelationV1,
    prepared_wal_generation: u64,
}

pub(crate) struct ReceiptInFlightDurableProofV1 {
    correlation_digest: Digest32,
    prepared_wal_generation: u64,
    receipt_state_digest_after: Digest32,
    in_flight_transition_digest: Digest32,
}

pub(crate) struct ReceiptTerminalTransitionRequestV1 {
    correlation: ReceiptHandoffCorrelationV1,
    terminal_wal_generation: u64,
    mutation_outcome_digest_v3: Digest32,
    directive: ReceiptTerminalHandoffDirectiveV1,
}

pub(crate) enum ReceiptTerminalHandoffDirectiveV1 {
    RevalidateUnchangedBaselineThenClearOrRevoke, // tag 1
    ValidateEligibleCommitThenAdvanceOrRevoke,    // tag 2
    Revoke,                                       // tag 3
}

pub(crate) enum ReceiptTerminalTransitionKindV1 {
    ClearNoChangeWithoutBaselineAdvance, // tag 1
    AdvanceExactEligibleCommit,          // tag 2
    Revoke,                              // tag 3
}

pub(crate) enum ReceiptTerminalTransitionObservationV1 {
    AppliedDurably,       // tag 1
    AlreadyAppliedExact, // tag 2
    InFlightAbsentNoOp,  // tag 3; legal only for clean pre-staging NoChange + exact baseline revalidation
}

pub(crate) struct ReceiptTerminalDurableProofV1 {
    correlation_digest: Digest32,
    terminal_wal_generation: u64,
    mutation_outcome_digest_v3: Digest32,
    directive: ReceiptTerminalHandoffDirectiveV1,
    transition: ReceiptTerminalTransitionKindV1,
    observation: ReceiptTerminalTransitionObservationV1,
    transition_authority_digest: Digest32,
    receipt_state_digest_after: Digest32,
    receipt_transition_digest: Digest32,
}

pub(crate) enum ArtifactWriterReceiptHandoffFailureV1 {
    Unavailable,
    CorrelationMismatch,
    GenerationMismatch,
    TransitionMismatch,
    DurabilityUnknown,
}

pub(crate) trait ArtifactWriterReceiptHandoffPortV1 {
    fn ensure_in_flight(
        &mut self,
        request: &ReceiptInFlightRequestV1,
    ) -> Result<ReceiptInFlightDurableProofV1, ArtifactWriterReceiptHandoffFailureV1>;

    fn reconcile_terminal(
        &mut self,
        request: &ReceiptTerminalTransitionRequestV1,
        terminal_outcome: &MutationHandlerOutcome,
    ) -> Result<ReceiptTerminalDurableProofV1, ArtifactWriterReceiptHandoffFailureV1>;
}

pub(crate) enum ArtifactWriterReceiptHandoffV1<'a> {
    NotRequired,
    Required {
        correlation: ReceiptHandoffCorrelationV1,
        port: &'a mut dyn ArtifactWriterReceiptHandoffPortV1,
    },
}

pub(crate) enum ArtifactWriterPresentationStatusV1 {
    Completed,
    Failed { reason: ArtifactWriterReasonV1 },
}

// One-to-one with the operation-neutral artifact_writer_* codes in §14;
// stable tags follow declaration order 1..18. There is no String/unknown ctor.
pub(crate) enum ArtifactWriterReasonV1 {
    Busy,
    LockFailed,
    IdentityUnavailable,
    ScopeChanged,
    BackendUnsupported,
    WalCapacityExceeded,
    WalCorrupt,
    RecoveryRequired,
    RecoveredRetry,
    OrphanDetected,
    StagingCleanupFailed,
    InstallUncertain,
    DurabilityUnknown,
    MetadataUnsupported,
    MetadataChanged,
    MetadataPostverifyFailed,
    ReplacedPrestateChanged,
    ReceiptHandoffFailed,
}

pub(crate) struct ArtifactWriterApplyResultV1 {
    pub(crate) mutation: MutationHandlerOutcome,
    pub(crate) presentation: ArtifactWriterPresentationStatusV1,
}

pub(crate) fn apply_artifact_writer_v1(
    plan: &ArtifactWriterPlanV1,
    witness: &ArtifactMutationLeaseWitnessV1,
    handoff: ArtifactWriterReceiptHandoffV1<'_>,
) -> ArtifactWriterApplyResultV1;
```

`ReceiptHandoffCorrelationV1` is complete rather than a caller-chosen opaque
token. `receipt_identity_digest` is the Task 9 v6 domain-separated digest of
the exact currently leased receipt primary key:
`SHA256("unica.discovery-receipt-primary-key.v1\0" ||
u64be(canonicalPrimaryKeyBytes.len) || canonicalPrimaryKeyBytes)`, where Task 9
accepts only its bounded canonical primary-key grammar. `expected_revision` is the
revision reread under that lease; `handoff_nonce` is a distinct nonzero 16-byte
OS-CSPRNG value selected by Task 10 while the artifact/current-receipt locks are
held; and the three digests equal the immutable current plan/grant/baseline.
The nonce is deliberately not the writer's later staging transaction ID: making
Task 10 supply an ID that only the writer can choose would create a data-order
cycle. Prepared binds both values separately. The correlation's exact digest is:

```text
SHA256("unica.artifact-receipt-correlation.v1\0" ||
       receipt_identity_digest || u64be(expected_revision) ||
       handoff_nonce[16] || plan_digest || grant_digest ||
       baseline_digest)
```

Task 9's store-backed recovery port must use `receipt_identity_digest` as its
direct bounded record/lock lookup key and revalidate the primary key inside the
locked record. Reverse-hash lookup, scanning/enumerating receipt records, or
trusting a filename without the in-record digest comparison is forbidden. This
makes a WAL-only restart able to reacquire one exact receipt without persisting
a receipt secret or an unbounded index search.

The two returned proof digests are also exact, not opaque success tokens:

```text
inFlightTransitionDigestV1 = SHA256(
  "unica.receipt-inflight-transition.v1\0" || correlationDigest[32] ||
  u64be(preparedWalGeneration) || receiptStateDigestAfter[32])

receiptTerminalTransitionDigestV1 = SHA256(
  "unica.receipt-terminal-transition.v1\0" || correlationDigest[32] ||
  u64be(terminalWalGeneration) || mutationOutcomeDigestV3[32] ||
  directiveTag || transitionTag || observationTag ||
  transitionAuthorityDigest[32] || receiptStateDigestAfter[32])
```

`receiptStateDigestAfter` is Task 9's canonical digest of the complete durable
locked receipt record after the transition. The writer recomputes both formulas,
requires nonzero Prepared/Terminal generations equal its selected WAL records,
and persists the complete proof, not only the final digest.

The full tuple and digest are bound in Prepared; a receipt identity digest not
recomputed from the currently leased primary key, an all-zero transaction ID,
an all-zero handoff nonce, another revision/plan/grant/baseline, and a
NotRequired call carrying a
correlation are rejected before Prepared. The
InFlight durable proof constructor accepts only the same correlation digest and
Prepared generation; `StagingIntentV1` binds that proof's complete canonical
digest. Terminal transition and acknowledgement constructors repeat and compare
the same correlation, outcome and generations. Thus a port cannot accidentally
acknowledge another receipt operation merely because a display ID or transaction
label matches.

The Terminal directive prevents another false implication: clean NoChange does
not itself prove the receipt baseline is still current. A second precondition
may have detected external drift, and Prepared recovery happens before the new
call's ordinary capture. For directive tag 1, the Task 10 port revalidates the
exact current composite baseline under the correlated receipt lease: equality
permits `ClearNoChangeWithoutBaselineAdvance`, while mismatch/inconclusive
requires `Revoke`. `InFlightAbsentNoOp` is legal only for that exact equal-
baseline clear (including a crash before InFlight became durable); absence never
suppresses a required revoke. Tag 2 similarly advances only after exact eligible
outcome plus post-snapshot/baseline validation; otherwise it revokes. Tag 3 may
only revoke. The port recomputes the supplied full terminal outcome digest and
returns a `transition_authority_digest` binding its exact baseline/post-state
decision. Ack stores that proof. A transition outside the directive, a clear or
advance without matching authority, or an unavailable revalidation is a
handoff failure and leaves Terminal blocking.

The candidate cannot write. Acquiring the witness opens the one trusted root,
proves same-mount/backend, takes process+OS lock and recovers
`ArtifactWriterWalV1` before it exposes capture capability. `ArtifactWriterPlanV1`
accepts only content/policy/relative components already validated by an
operation resolver **under that same witness**. Its private constructor binds
the witness generation and authoritative operation-policy digest. An early
support/mode check may route or reject, but is never write authority; CFE
revalidates adoption/material and `support.edit` revalidates current support
policy only after WAL recovery under the root-wide witness, then retains that
one witness through handler/outcome/handoff. Acquiring a second artifact lease
or applying a plan built before the authoritative under-witness read is a hard
STOP. `ArtifactMetadataPolicyV1` is the closed union of
`PresentTargetMetadataV1` and per-object `CreatedArtifactMetadataV1`. Output is
always `unica.mutation-outcome.v3`; WAL is always
`unica.artifact-writer-wal.v1`. The generic writer emits only §14
`artifact_writer_*` reasons. These type/schema/reason names are package-internal
compatibility authority and may change only by a new version plus migration and
hard-crash requalification.

`ArtifactWriterReceiptHandoffPortV1` is an inverted persistence boundary, not a
Task 8 receipt store. Required apply calls invoke `ensure_in_flight` only after
Prepared is durable and validate every returned digest/generation before
StagingIntent. After Terminal they invoke `reconcile_terminal`, persist the
returned proof in self-contained ReceiptHandoffAcked and never trust the port's
display/error text. Task 8 owns recording/mismatch/crash fakes; Task 9/10 later
own the store-backed implementation and receipt lease. The same trait is passed
to lease recovery so a current NotRequired call cannot skip a prior Required handoff.

`ArtifactWriterApplyResultV1` always contains schema-v3 mutation authority,
including when presentation is Failed. Before durable Prepared, a rejected
final precondition/capacity/metadata check returns
`NoChange+Unmodified+empty VerifiedClean` plus its typed reason and proves zero
request WAL/staging/source effect. If Required had been selected, no InFlight
exists yet: after the moved handoff borrow ends, Task 10 revalidates the still-
locked receipt baseline and revokes on drift/inconclusive material, otherwise
leaves the unchanged receipt row intact. After Prepared, every ordinary failure
is first reduced to the exact durable WAL-backed outcome/Terminal/handoff state;
the presentation reason never substitutes for that authority. There is no
untyped `Result::Err` escape on either side of Prepared.

### 12.4 Native adapter/registry

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
    lease: &ArtifactMutationLeaseWitnessV1,
    handoff: ArtifactWriterReceiptHandoffV1<'_>,
    context: &WorkspaceContext,
) -> HandlerOutcome;
```

`cfe.rs` must not call raw argument helpers, map ModulePath, parse
Configuration.xml, select Context/kind, inspect warning paths or rerender.
It moves the invocation handoff exactly once into `apply_artifact_writer_v1`;
no context/global/service lookup may reconstruct Required/NotRequired inside
the native handler.

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
artifact lease is held and after that lease recovered any non-Idle WAL. Task 8
invokes the final check directly at its recording seam; Required Task 10 flow
invokes that same check after its receipt lease/baseline reread. Only after this
check may the writer persist Prepared; for Required it then invokes the Task 10 port
to durably write correlated receipt InFlight before StagingIntent. Immediately
before request WAL/control staging, without
reparsing raw arguments:

1. run one final `capture_with_watches` with the plan's exact selected sources
   and watch; do not resolve or rerender a replacement plan;
2. require its source/composite fingerprints, watch outcomes and all execution
   material identities to equal the immutable plan, including the exact six-field
   `CfeCatalogSnapshotBindingV1`, both current registered-Form handle/binding
   authority digests and complete binding-catalog semantic digests when
   applicable;
3. revalidate exact source topology/mapping digest;
4. prove the supplied witness canonical locus equals the plan artifact mutation
   scope and current selected destination root still has the witness's retained
   physical identity, same-mount/volume proof and qualified backend tuple;
5. from the already-retained destination-root capability, walk the complete
   parent plan descriptor-relatively with no-follow/no-reparse semantics; an
   absolute or workspace-path destination reopen is forbidden;
6. require every PresentDirectory and AbsentDirectory state to remain exact and
   re-read the retained receiving-parent metadata used by
   `CreatedArtifactMetadataV1`;
7. Present target: compare exact bytes length/digest/boundary, object identity,
   every `PresentTargetMetadataV1` stable field and the private captured
   last-write timestamp prestate;
8. Absent target: prove the tombstoned target remains absent and re-synthesize
   every expected file/directory metadata digest from the unchanged parent;
9. reconstruct final bytes and compare expected-after length/digest;
10. prove the qualified tuple supports the plan's exact cross-directory file or
    directory publication, both parent durability operations, WAL transitions
    and metadata apply/postverify contract;
11. mismatch -> `cfe_patch_material_changed`, `artifact_writer_metadata_changed` or
    `cfe_artifact_scope_changed` with zero planned mutation.

This final pre-Prepared recapture verifies the existing plan; it is not a second resolver and cannot change
target, source method, renderer or allowed effects. Source/adoption changes after
it are checked once more after StagingReady by §13.3 step 4. Changes after that
last comparison are subject to the same explicitly bounded
non-cooperating-writer window as Present replacement; the writer never rereads
and reinterprets them independently or claims portable CAS.

### 13.3 Contained atomic write

Do not use `create_dir_all` and do not stage below the source target parent.
Adopted registration proves descriptor authority, not that `<Name>/Ext` or
`<Form>/Ext/Form` exists. The writer performs this exact WAL-backed sequence
while the artifact lease remains held:

1. require one exact v2 backend row proving same mount/volume, root-wide lock,
   dual-slot WAL, cross-directory file **and directory** rename semantics,
   metadata synthesis/application and both-parent durability. Unsupported is a
   STOP before control staging or source mutation;
2. durably write `Prepared`, including chosen control name, plan/backend/
   metadata digests, source prestate, retained source/control parent identities
   and receipt correlation. For Required handoff, invoke the Task 10 port and
   require a durable InFlight proof bound to that exact Prepared generation;
   persist that proof digest in `StagingIntent`. NotRequired has an explicit
   no-receipt tag. Only then durably write `StagingIntent`;
3. create exact
   `control-v1/staging/artifacts/<collision-key-digest>/<transaction-id>` as
   either the one publication file or the root directory of the complete
   top-absent subtree. There is no enclosing transaction directory. Create every
   descendant component no-follow, write
   exact final bytes, apply §1.10 metadata to the target and every directory,
   record all physical identities, flush file and staged directories bottom-up,
   verify content+metadata, then durably write `StagingReady`;
4. immediately before install, repeat the plan's exact same-watch capture and
   compare every analysis/destination material identity, source method/adoption/
   both Form surfaces including registered handle/binding authority,
   mapping/topology, target/parent/content/metadata and
   private Present timestamp precondition. This is comparison only: no new
   resolve/render/plan is constructible. A failure first durably writes
   `CleanupIntent`, then attempts to remove only control staging and flush its
   parent. It records terminal NoChange with source `Unmodified` and exact
   VerifiedClean/Residue/Unknown cleanup; Required terminal reconciliation
   revalidates and revokes any drifted receipt baseline. Only the clean outcome
   is tombstone-eligible;
5. durably write `InstallIntent` immediately before exactly one rename:
   Present file replace, Absent file no-replace, or complete absent-subtree
   no-replace into the retained parent of the first absent component;
6. classify rename completion by §1.8 even when the syscall reports an error.
   Exact staged file at target or exact staged subtree root plus plan-bound
   descendant target relation constructs `InstallObserved(Definite)`;
   irreducible completion ambiguity constructs `Uncertain`; no retry-as-clean;
7. for definite install, persist the unique staged target identity, then fresh-
   walk target and created directories and re-read the retained old Present
   object to produce current and replaced-prestate content/metadata/timestamp
   observations. Query failure yields Unknown fields on `Committed`;
8. durably write `DurabilityIntent`; flush installed target, every published
   directory required by the tuple, the source receiving parent and the control
   source parent. A failure keeps definite `Committed` with durability Unknown;
9. durably write `CleanupIntent` with every owned identity and the expected
   Consumed/Removed disposition before any explicit deletion. Classify the
   control staging name as `ConsumedByPublication` only when its staged root is
   proven installed and the exact file-root/descendant relation resolves the
   unique target commit; otherwise attempt the
   authorized removal and classify it as Removed. Remaining/unqueryable control
   names are Residue/Unknown and make the terminal state blocking;
10. durably write Terminal with the complete schema-v3 outcome. Eligible
    NotRequired handoff may then tombstone Idle; Required follows the mandatory
    receipt transition/ack/tombstone handoff in §1.8;
11. return exact target/non-target effects, metadata observations, cleanup,
    durability and recovery disposition even when adapter presentation fails.

For `AbsentWatched`, installation is no-replace: the Linux row uses
handle-relative cross-directory `renameat2(..., RENAME_NOREPLACE)`, macOS uses
cross-directory `renameatx_np(..., RENAME_EXCL)`, and Windows uses the exact
rooted no-replace `FileRenameInfoEx` flags below. The primitive must qualify both
file and nonempty directory-subtree source objects. Each consumes the control
staging-root name into source and therefore yields `ConsumedByPublication`
after name-absence plus exact file-root/subtree-descendant relation proof.
Appearance of target loses the race and returns
`cfe_patch_absent_target_appeared` without overwriting it. If `linkat` commits
the target in a future separately qualified Unix contract, that is a new
version; v2 permits rename only. A platform without tested atomic no-replace
file+directory primitives is a hard STOP, not permission to check then rename,
mkdir incrementally or stage under the target parent.

Unix mutation uses the closed `unica.unix-contained-atomic.v2` contract. Its
allowlist is data, not an optimistic `cfg(unix)` branch. A row is enabled only
when its exact OS/kernel build family, architecture, filesystem discriminator,
mount flags and native-suite identity produce a recorded
`qualified_tuple_digest`; no prefix/family fallback is allowed:

```text
qualifiedTupleDigest = SHA256(
  "unica.atomic-backend-qualified.v2\0" ||
  lp(contractId) || lp(osBuildPredicateId) || lp(architectureId) ||
  lp(filesystemDiscriminatorId) || lp(mountPolicyId) ||
  lp(lockPrimitiveTranscriptId) || lp(installPrimitiveTranscriptId) ||
  lp(metadataPrimitiveTranscriptId) ||
  lp(replacedPrestateObservationTranscriptId) || lp(walRecoveryTranscriptId) ||
  lp(durabilityPrimitiveTranscriptId) || lp(nativeSuiteEvidenceSha256))
```

`lp(x) = u64be(x.len) || UTF-8 x`. Every predicate/transcript ID is a closed
constant in `contained_atomic_writer.rs`; the evidence field is the reviewed native-suite
artifact SHA. Runtime facts must satisfy the exact row before its digest enters
the witness. Adding/changing any primitive, OS predicate or evidence creates a
different row/digest and requires spec/product-CI review.

Every enabled v2 row constructs the same root-wide
`FilesystemArtifactCollisionKey` before opening the persistent artifact lock.
No backend may substitute raw/case-folded/NFC/NFD locus bytes, target FileId or
a successful lookup. Linux/macOS/Windows may differ in path semantics, but v2's
over-lock deliberately makes those semantics irrelevant to serialization; the
qualified capability remains necessary for containment/install/durability.

| Candidate tuple | Runtime/WAL/metadata proof before source install | Process/OS lease | Control staging + one install primitive | Durability transcript required for `VerifiedDurable` |
| --- | --- | --- | --- | --- |
| Linux kernel >= 5.8, local ext4 (`EXT4_SUPER_MAGIC`) | retained `STATX_MNT_ID`/fs magic/writable proof is equal for workspace, control and destination; native suite proves two-slot WAL torn writes/recovery, empty-xattr/trivial-ACL/zero-flags metadata, and cross-directory file+nonempty-directory rename on this mount | process registry + `flock(LOCK_EX\|LOCK_NB)` on persistent inode; recovery before capture | create/write/metadata/flush below control via `mkdirat/openat`; Present `renameat2(flags=0)`, both Absent shapes `renameat2(RENAME_NOREPLACE)` with distinct native file/directory tests; no source mkdir/temp | `fsync` WAL records/dir; staged file and every staged directory bottom-up; after rename installed target/published directories, retained source receiving parent **and retained control source parent**; all calls/evidence exact |
| Linux kernel >= 5.8, local XFS (`XFS_SUPER_MAGIC`) | same categories with exact XFS magic and independent WAL, metadata, file+directory rename/crash evidence | same explicit `flock` contract | same named descriptor-relative primitives, independently qualified | same complete two-parent transcript; ext4 evidence cannot qualify XFS |
| macOS >= 13, local case-sensitive APFS (`f_fstypename=apfs`, `MNT_LOCAL`, not `MNT_RDONLY`) | retained `st_dev+(f_fsid,f_mntonname)` equality and `_PC_CASE_SENSITIVE=1`; native suite proves dual-slot WAL, trivial ACL/empty xattr/zero `st_flags`, cross-directory file+nonempty-directory semantics and metadata preservation | process registry + `flock(LOCK_EX\|LOCK_NB)` | control `mkdirat/openat`; Present `renameatx_np(flags=0)`, both Absent shapes `renameatx_np(RENAME_EXCL)`; no source mkdir/temp | `F_FULLFSYNC` staged/installed files; `fsync` staged/published directories, WAL dir, source receiving parent and control source parent; hard-crash evidence exact |
| macOS >= 13, local case-insensitive APFS (`f_fstypename=apfs`, `MNT_LOCAL`) | same categories with `_PC_CASE_SENSITIVE=0`, plus NFC/NFD/case collision/recovery matrix and its own evidence digest | same collision-universe process registry + `flock` | same named control/cross-directory primitives, independently qualified | same complete transcript; case-sensitive evidence cannot qualify this row |

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
`artifact_writer_backend_unsupported` before control staging/source install. NFS,
CIFS/SMB, FUSE, overlay, tmpfs, network, read-only, nested-mount and unknown
magic/flags are always unsupported in v2. Failure of a named durability call
after definite install returns `Committed + MutationDurabilityState::Unknown`;
it cannot mint `VerifiedDurable` from read-back or a live manifest.

The Windows backend is the versioned
`unica.windows-contained-atomic.v2` contract. Its initial candidate tuple is
local NTFS on Windows 10 build 17763+ or Windows Server 2019+ with exact
architecture/build/filesystem capability digest and the complete native suite;
it is enabled only after that digest is recorded in the reviewed support matrix.
Windows x64 and arm64/build-family rows have independent evidence and digests;
one never qualifies another.
network/SMB, ReFS, FAT/exFAT, older or unproven build/filesystem tuples fail with
`artifact_writer_backend_unsupported` before control staging.
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
   unsupported before control staging/source mutation;
2. open every destination and control component with `NtCreateFile`/`NtOpenFile`
   rooted in the retained parent handle, `FILE_OPEN_REPARSE_POINT`, exact
   directory/non-directory options and retained `FileIdInfo`. Open the control
   staging parent and source receiving parent with the exact namespace-write,
   share and flush access from the qualified tuple. Create files/directories
   only below control staging; never create a temp or parent below source;
3. acquire the persistent lock object with process-universe registry plus
   `LockFileEx(LOCKFILE_EXCLUSIVE_LOCK | LOCKFILE_FAIL_IMMEDIATELY)`; a
   control-root capability probe must already have qualified lock coherence,
   dual-slot WAL, cross-directory file/nonempty-directory relative rename flags,
   closed Windows metadata synthesis and the exact retained-directory
   `NtFlushBuffersFileEx(parent, Flags=0, Parameters=NULL,
   ParametersSize=0, &io_status)` transcript for this tuple;
4. synthesize/apply/verify owner, group, inherited canonical DACL and allowed
   basic attributes for every control-staged file/directory; reject SACL, ADS,
   reparse, compression, encryption and sparse state; flush target data and the
   staged directory tree bottom-up before `StagingReady`;
5. after durable `InstallIntent`, install the already-open staged file or
   subtree root with reviewed `SetFileInformationByHandle(FileRenameInfoEx)`
   whose `RootDirectory` is the retained source receiving parent and `FileName`
   is the target/first-absent component. Absent uses no replace flag; Present
   uses exactly `FILE_RENAME_FLAG_REPLACE_IF_EXISTS`. File and nonempty-directory
   cases have independent native evidence;
6. classify destination-exists, unsupported information class/filesystem,
   sharing violation and unknown completion separately; unknown completion is
   `Uncertain`, never retry-as-NoChange;
7. re-walk from retained destination root, compare VolumeSerial/FileId for the
   unique target and every created directory, read target content/metadata, and
   prove the control staging name absent through its retained control parent;
8. call `FlushFileBuffers` on the installed target; call the exact flags-zero
   `NtFlushBuffersFileEx` operation on every published directory bottom-up, then
   call
   `NtFlushBuffersFileEx(retained_source_receiving_parent, Flags=0,
   Parameters=NULL, ParametersSize=0, &io_status)` and independently the same
   operation on `retained_control_source_parent`; require `STATUS_SUCCESS` for
   both. Flags-zero is the only v2 directory-namespace candidate; data-only,
   no-sync, volume-handle and path-reopened substitutes are forbidden. The exact
   OS/architecture/NTFS tuple, access/share options and hard-crash evidence must
   match `durabilityPrimitiveTranscriptId`. Only the full target + published
   tree + source-parent + control-parent transcript creates `VerifiedDurable`.
   Failure after rename is definite `Committed + durability Unknown`; inability
   to qualify either parent flush excludes the tuple before control staging;
9. before an explicit pre-install cleanup, durably write `CleanupIntent`, then
   abandon only the exact WAL-owned control staging root using retained-handle
   `FileDispositionInfo`, verify its control name absent and flush the retained
   control parent. Failure returns typed NoChange+Residue/Unknown with a non-
   Idle WAL and never clean retry. After `InstallIntent`, never infer
   rollback: probe install completion via §1.8 and return Committed or Uncertain.

Path-based `MoveFileExW`, path-based `CreateDirectoryW` after a checked walk,
path-based `DeleteFileW`/`RemoveDirectoryW`, path-based `CreateFileW`
destination reopen, or any absolute-path reopen after
the one workspace open cannot satisfy this contract. Build/filesystem support,
required information classes, security synthesis and access/share flags are
validated before control staging. Native same/two-process, file/subtree rename,
metadata race/failure and hard-crash/restart WAL+durability tests must run on
every claimed tuple; official API shape or one successful call is insufficient
qualification.
Retained handles prevent a parent swap from redirecting the operation to a
replacement directory; they do not create a hash-CAS or prevent a
non-cooperating actor from renaming the already-open directory object itself.
Post-path/identity verification therefore uses the detached-object contract
below rather than inventing an in-scope path effect.

After any platform reports a definite commit, the writer compares the retained
target/parent identities with a fresh handle-relative walk from the retained
destination root. That definite staged identity always constructs exactly one
`DefiniteTargetCommitV1`. Exact match is `AtIntendedPath`; conclusive absence or
different object is `DetachedOrRelocated`; failed/unavailable query is
`location=Unknown`. A content/metadata read failure likewise becomes Unknown
on the same `Committed`. Cleanup is independently VerifiedClean/Residue/Unknown
from the control staging name. Only genuinely unknown rename completion creates
`Uncertain(PossibleTargetInstallV1)`. Unix uses `fstat`/`fstatfs`; Windows uses
VolumeSerial/FileId. All identities are domain-separated opaque values retained
through Task 10 revocation.

For Present, the writer performs an atomic cross-directory control-to-source
replacement only
after the final exact precondition. The artifact lease makes that replacement
serializable against every Unica writer, including NotRequired handoff. It is
not advertised as a filesystem compare-and-swap against arbitrary external
processes: an external writer that commits before the final check is detected,
but a non-cooperating writer can still race in the check-to-replace window. Rust
and the target filesystems expose no portable content-hash conditional replace.
If the product requires zero lost updates against arbitrary external writers,
STOP and add a separately proven platform-specific transaction/CAS contract;
do not call the cooperative Present operation “conditional replace”.

On failure before `InstallIntent`, first persist `CleanupIntent`, then remove
only the exact WAL-owned control staging root, verify its name absent and flush
its retained control parent. Write Terminal(NoChange) with source `Unmodified`
and the exact cleanup state; tombstone according to handoff variant only when it is
VerifiedClean. There is no source parent rollback. Cleanup failure returns typed
NoChange+Residue/Unknown and leaves a non-Idle WAL, so the call is not cleanly
retryable. After
`InstallIntent`, the writer probes completion and returns either definite
`Committed` or `Uncertain`; it never attempts semantic rollback. Post-write
content or metadata mismatch forces revocation. A target commit followed by any
required target/source-parent/control-parent durability failure remains definite
`Committed`, retains cleanup, carries durability Unknown and is non-advancing.

### 13.4 Typed result authority

Plan/handler authority uses workspace-relative slash paths as intended labels;
current location authority is the separate fresh handle-relative observation.
Display changes, stdout and artifacts are derived after preview/apply and never
parsed back.
Task 8 returns `ArtifactWriterApplyResultV1`: the generic
`MutationHandlerOutcome` from §12.2 plus operation-neutral typed presentation
status. Receipt/WAL authority consumes only the mutation and durable handoff
proof, never presentation. A successful CFE commit has exactly one target commit with AtIntendedPath+Matches, Known expected content
and VerifiedExact metadata, plus Created NotApplicableForCreated or Updated
replaced-prestate content+metadata MatchesCaptured. Its expected non-target effects are ancestor-first
CreatedDirectory rows for exact `write.directories_to_create`, each with exact
identity/location/metadata; unexpected/possible are empty, cleanup accounts the
control staging identity, and durability proves the qualified two-parent
transcript.

`NoChange` is legal only before install or after recovery proves install did not
happen, with source `Unmodified`, empty source-effect vectors and one complete
cleanup state. VerifiedClean requires durably removed control staging; a
Residue/Unknown NoChange revokes and keeps the WAL blocking. Fixed control
directories/WAL slots/lock inode are internal, not grant effects. A known commit with staging residue remains `Committed`; a
known commit with unobservable cleanup remains `Committed + cleanup Unknown`.
`Uncertain` is only unknown rename completion and carries PossibleTargetInstall.
Task 10 compares target and non-target effects/metadata with plan/post-manifest,
requires exact Created/Updated replaced-prestate state, AtIntendedPath + Matches
+ Known expected content + VerifiedExact metadata,
`MutationCleanupState::VerifiedClean` plus
`MutationDurabilityState::VerifiedDurable`, and revokes on durability Unknown,
any unknown/mismatch/relocated observation, unexpected/possible effect,
Residue/Unknown/Uncertain or recovery ambiguity. It never parses display or
assumes a control staging name appears in the ordinary source manifest.

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
| form/adoption | `cfe_form_handler_wrong_mechanism`, `cfe_destination_form_handler_collision`, `cfe_form_binding_inconclusive`, `destination_borrow_required`, `destination_object_not_adopted`, `destination_extended_object_mismatch`, `analysis_metadata_identity_inconclusive`, `analysis_not_base_owned`, `destination_not_extension_flavor`, `destination_membership_inconclusive` plus exact shared provider/gap reason |
| configuration | `cfe_configuration_missing`, `cfe_configuration_malformed`, `cfe_configuration_flavor_mismatch`, `cfe_name_prefix_missing`, `cfe_name_prefix_empty`, `cfe_name_prefix_duplicate`, `cfe_name_prefix_invalid`, `cfe_script_variant_missing`, `cfe_script_variant_duplicate`, `cfe_script_variant_unknown`, `cfe_script_variant_invalid` |
| module/render | `cfe_module_too_large`, `cfe_module_encoding_unsupported`, `cfe_module_preflight_inconclusive`, `cfe_duplicate_interceptor`, `cfe_duplicate_generated_method`, `cfe_rendered_patch_limit`, `cfe_final_module_limit` |
| batch | `cfe_batch_plan_conflict` |
| expected/grant | `cfe_proposal_target_mismatch`, `cfe_destination_source_mismatch`, `cfe_allowed_effects_mismatch`, `cfe_grant_scope_mismatch`, `cfe_execution_plan_mismatch` |
| generic artifact writer/recovery | `artifact_writer_busy`, `artifact_writer_lock_failed`, `artifact_writer_identity_unavailable`, `artifact_writer_scope_changed`, `artifact_writer_backend_unsupported`, `artifact_writer_wal_capacity_exceeded`, `artifact_writer_wal_corrupt`, `artifact_writer_recovery_required`, `artifact_writer_recovered_retry`, `artifact_writer_orphan_detected`, `artifact_writer_staging_cleanup_failed`, `artifact_writer_install_uncertain`, `artifact_writer_durability_unknown`, `artifact_writer_metadata_unsupported`, `artifact_writer_metadata_changed`, `artifact_writer_metadata_postverify_failed`, `artifact_writer_replaced_prestate_changed`, `artifact_writer_receipt_handoff_failed` |
| CFE resolve/apply | `cfe_control_mount_boundary_unsupported`, `cfe_destination_mount_boundary_unsupported`, `cfe_artifact_scope_changed`, `cfe_resolved_plan_required`, `cfe_unexpected_resolved_plan`, `cfe_patch_material_changed`, `cfe_patch_target_unsafe`, `cfe_patch_absent_target_appeared`, `cfe_patch_after_digest_mismatch` |

The direct tool and discovery reuse the exact Task 5A membership leading codes;
there is no `cfe_destination_borrow_required` alias that could collapse Own or
wrong-UUID states. Snapshot/common capture reasons remain the exact common codes
in §7.5. Errors never dump raw arguments, source body, absolute lock path or
unrelated paths.

The reusable writer emits only operation-neutral `artifact_writer_*` reasons
and typed schema-v3 outcomes. CFE maps pre-writer resolver/material failures to
`cfe_*`; it does not rename writer recovery/metadata/install reasons. Task 5C
and later operations import the same reason namespace instead of cloning a
path-specific protocol.

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
| acyclic phased ledger | product test fails when Task 8 depends on the combined Task5C file/whole Task5C/`support.edit`, when Evidence lacks separate design+self-audit+review+implementation, when Task5C-Mutation addendum precedes Task8/9/10 implementations, when a full Task 7 implementation (including Task8 integration) is an upstream Task8 gate, when `Task7PrerequisiteSliceV1` imports Task8, when Task8 delivery lacks distinct `task7_task8_integration_evidence_sha256`, when a self-audit substitutes for a distinct independent review, or when 40-hex Git OID and 64-hex SHA-256 keys are swapped/mislabelled; exact Git objects and immutable files must exist | separate Task5C-Evidence-v2 and downstream Task5C-Mutation-v2 families, Task5B/6, split Task7 prerequisite/integration, Task9/10 addenda and `docs/project-discovery-v6-hash-ledger.md` use §2.7 names/order |
| prepare/material phase ownership | `prepare_has_zero_material_reads`: panic snapshot/material/fs fakes record zero calls; capture is the first reader | Task 7 design/order; active spec two-stage resolver; CFE skill troubleshooting must not tell the handler to probe live files |
| Configuration-only analysis authority | Configuration + PlatformXml resolves; Extension analysis remains visible in Explore but its CFE proposal is Unknown/ineligible, issuer-silent and has exactly one proposal-only `mutation_preflight` check per canonical affected chunk from `DiscoveryPreflight` with skipped/inconclusive/unknown/blocking, `cfe_analysis_configuration_required`, retryable false and empty details/evidence; no `source_readiness` check; wrapper UUID decoy never joins | Task 5B typed fact/source-kind preservation and Task 7 application preflight validation; active spec/ADR/skill; explicitly forbid a v1 `BaseMetadataIdentity` alternative |
| captured flavor + Own base authority | strict MDClasses tracked base-with-compat/no belonging+purpose is Base; tracked Adopted+Customization extension without compatibility/KeepMapping is Extension; optional fields valid on either flavor never discriminate; wrong namespace/partial/other/duplicate flavor emits no CFE membership/eligible row; Adopted analysis root/Form fails before UUID comparison; wrapper/local UUID decoy cannot join | Task 5A owns closed flavor/membership facts and blocker projection; Task 5B shared catalog exact §6.4 flavor/emission gate + descriptor Own; Task 7 issuer requires proof; Task 9 grant stores it and Task 10 current guard re-proves it; active spec/ADR/skill/product contract |
| exact membership lattice | table test for Absent -> RequiresBorrow, Own -> Unknown/not-adopted, equal Adopted -> ExtensionOwned, wrong Adopted -> Unknown/mismatch, malformed/gap -> Unknown/inconclusive; mixed Form row preserves Unknown precedence | Task 5A + Task 5B provider contract; active spec; CFE skill recommends borrow only for descriptor absence |
| single accepted Form authority | static/product scan proves Task 8 defines no Form item/event/callType/Action registry; Task 5B v7 registry tests include current audited `Button/Click` rejection, every accepted event-bearing kind such as `RadioButtonField/OnChange`, unknown event fail-closed, exact regular-vs-extension/BaseForm callType rule, zero/duplicate Action cardinality, lexical limits and outer XML capture N/N+1; no separate binding-count cap/N+1 exists; Task 8 consumes only registry version + complete lookup | `.superpowers/sdd/task-5b-v7-contract.md` and its separate self-audit/independent review must receive fresh no-P0/P1/P2 acceptance; Task 5A/domain `ArtifactRef` + `ArtifactIdentityBytesV1` identity seam is upstream of sibling Task5B/6/8; active spec/product tests point to that authority without copying rows |
| destination Form generated-name ownership | accepted complete destination lookup bound to generated name with no BSL definition still blocks; case variant, orphan and any incomplete destination auxiliary catalog fail closed; unrelated accepted binding preserves scope digest but changes execution material | Task 4 captures both Forms; Task 8 independently builds both auxiliary catalogs with Task 5B's one parser/current registered handles and carries both proofs to Task 7 issuance; Task 9 stores version/semantic proof/baseline and Task 10 re-proves it; active spec/ADR/skill |
| complete observed interceptor set | RU/EN Before, After, Around/Вместо and ModificationAndControl facts plus exhaustive 4x3 conflict matrix and conditional/malformed fail-closed cases | Task 6 parser design/cache schema/tests; active spec duplicate rules; CFE skill duplicate diagnostics |
| immutable exact source slices | forged definition/declaration/name/parameter/body/terminator range and line-ending cases each fail; changing any exact slice changes the specified digest fixture | Task 6 DTO/parser/cache version/tests; active spec digest table; CFE skill source-bound explanation |
| canonical assertion erasure | omitted and explicit matching SourceSet/Context/IsFunction yield value-equal resolved cores/full plans/digests; diagnostic assertion flags may differ but never enter plan/grant/handler | Task 7 prepared intent owns Options and resolved issuance owns canonical core; Task 9 grant schema stores only resolved values; Task 10 guard compares that canonical core and cannot observe assertion presence; active spec/ADR/skill/product contract |
| first-patch atomic subtree | missing `<Name>/Ext` builds the complete suffix+Module.bsl only under control and publishes one no-replace directory rename; file/symlink/case alias fails; N/N+1 chain; injected preinstall failure leaves source byte-identical; grant A creates parents while B retains stable scope and later resolves file-only publication | Task 4 watch; Task 9 v6 stable grant+metadata scope; Task 10 v6 typed post-diff; active spec/ADR/skill forbid incremental mkdir/target-parent staging |
| collision-safe artifact serialization universe | every v2 atomic backend uses root-wide collision protocol v1 and maps all semantic loci under one physical destination root to one collision key/inode. Two processes over path/case/NFC/NFD/bind aliases of the same whole physical workspace prove same control-root, collision key, opened lock inode/FileId and one Busy for Present and Absent; semantic locus bytes may differ. APFS case-sensitive and case-insensitive rows each run NFC/NFD + case-pair REDs; even genuinely distinct case-sensitive files intentionally over-lock. W1/W2 with one shared destination bind are rejected before capture. Equal semantic/collision-key bytes alone never satisfy the actual-inode+Busy RED | Task 9 reuses Task 8 collision protocol/universe/mount/backend; Task 10 deduplicates collision keys before receipt locks; active spec/ADR define root-wide v1 over-lock, mixed-version migration gate and delete broad destination-bind/canonical-locus-lock claims |
| one workspace control universe | control path is exactly `.build/unica/project-discovery/control-v1` below retained workspace with no `<workspaceKey>`; cache/cwd/PID aliases cannot split it; lock/WAL/staging/receipt namespaces are fixed siblings; unsafe `.build/unica` link/reparse fails closed | Task 9 v6 addendum deletes `${cache_root}` and workspace-key hashing; active spec/ADR/deployment/package tests name the exact fixed root; Task 10 accepts no process-cache lease |
| honest install/replace semantics | Absent external creator wins and is never overwritten; Present precheck catches completed edit; test/docs explicitly do not assert CAS against a non-cooperating writer in the check/replace window | Task 9 shared atomic primitive; Task 10 uncertainty/revocation; active spec/ADR/skill guarantee boundary |
| Unix v2 backend boundary | each ext4/XFS/APFS case-mode row independently proves dual-slot WAL, cross-directory Present file replace, Absent file and nonempty-subtree no-replace, Present/Created metadata, source+control-parent sync and hard-kill/remount matrix; any missing primitive/evidence and NFS/CIFS/FUSE/overlay/tmpfs/network/unknown fails before control staging | Task 9 reuses `unica.unix-contained-atomic.v2`; Task 10 consumes v3 authority; package matrix/spec/ADR name exact digests |
| Windows v2 handle-relative boundary | after one workspace open, all destination/control/WAL/stage/install/cleanup operations are rooted handles; exact NTFS tuple proves file+nonempty-directory FileRenameInfoEx, owner/group/DACL inheritance, attrs/ADS/reparse/compress/encrypt/sparse gates, WAL and two-parent NtFlush hard-crash matrix; no source temp/mkdir/rollback; missing capability fails before control staging | Task 9 v6 reuses backend/schema; Task 10 v6 consumes observations/durability; active spec/ADR/skill and Windows matrix |
| WAL recovery and bounded GC | hard-kill at every transition and before/after every syscall covers syscall-not-run/syscall-run windows; second process Busy; restart recovers before capture for both NotRequired and Required handoff; torn/equal-divergent/unknown-version slots, orphan/mismatched staged identity and ambiguous source fail closed; two slots never grow; dirty/durability-unknown/possible Terminal or Ack never tombstones; eligible Terminal/Idle parent sync proven | Task 9 v6 persists correlation schema; Task 10 v6 handoff; active spec/ADR/observability/privacy/repair docs |
| control staging lifecycle | the zero-or-one WAL-owned publication-root name appears exactly once as Removed or ConsumedByPublication; file-root and subtree-descendant relations are distinct and exact; a remaining owned root is Residue, query failure Unknown, a second sibling is orphan and an unauthorized descendant is recovery-required; no public raw name | Task 9 schema v3 closed tags/bounds; Task 10 requires clean; observability/replay distinguish control root from unique target identity |
| metadata authority | Present mode/uid/gid/nlink/ACL/xattr/flags and Windows owner/group/DACL/basic attrs/ADS/reparse/compress/encrypt/sparse rows exhaustively accept/reject; Absent target plus every moved directory receives parent-derived Created metadata, never control-root mode/DACL; precheck and retained-old-object race/query/set/postverify failures have exact typed outcomes | Task 9 v6 grant/effect schema binds policy/digests; Task 10 v6 re-proves Updated replaced-prestate and revokes; package tuples qualify inheritance synthesis |
| total definite-install algebra | injected target reopen/read/stat/FileId/metadata and Updated old-object content/metadata failures after exact rename all construct Committed with unique physical identity and independent Unknown/Different observations; no constructor can emit Uncertain or NoChange for that definite install; only unknown rename completion yields PossibleTargetInstall | Task 9 persists schema v3/tags; Task 10 revokes every unknown/mismatch; observability/replay never infer from display/live manifest |
| durability authority | after definite install, failure of installed-data, published-dir, source-parent or control-parent flush returns Committed+Unknown and leaves Terminal/Ack non-Idle; preinstall NoChange is tombstone-eligible only after removal+control-parent flush; Committed is tombstone-eligible only with VerifiedDurable+VerifiedClean, and advances only with those plus exact observations | Task 9 v6 persists schema only; Task 10 v6 owns advance/revoke; platform matrix/replay preserve transcript |
| NoChange scope | preinstall failure after any control-staging step leaves source Unmodified and empty source effects; exact removal+sync yields VerifiedClean, while cleanup failure yields typed Residue/Unknown with non-Idle WAL, never source rollback, install uncertainty or clean retry | Task 9 schema v3; Task 10 clears clean InFlight unchanged only after exact current-baseline revalidation, otherwise revokes; dirty NoChange always revokes; spec says no source names exist before install |
| artifact/receipt handoff | hard-kill REDs after final-check/before Prepared, Prepared->InFlight, InFlight->StagingIntent, Terminal->receipt transition, transition->WAL ack, eligible ack->Idle and dirty/durability-unknown/possible ack->blocking prove artifact->receipt order, idempotent correlation, terminal retention and reverse release. Prepared recovery permits exact clear/already-clear/absent-no-op only after baseline equality, revokes drift, and blocks unavailable/mismatched/differently transitioned authority. NotRequired calls still receive the recovery port so an earlier Required WAL cannot be bypassed; NotRequired eligible-only Terminal GC is separately recovered | Task 9 owns store/lease/correlation; Task 10 v6 production pipeline; spec/ADR Guard Order |

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
"NoChange has zero persistent effects". It additionally rejects source-parent
temp/mkdir/rollback, same-directory publication, schema v2,
`target_effect_digest`, a definite install represented as Uncertain, WAL clear
before receipt handoff, metadata copied from control inheritance, or a backend
row lacking cross-directory file+directory/two-parent durability evidence. New
Task 5B/7 v6 and Task 9/10 v6 addenda, outcome/WAL schema, observability/replay
and support-matrix texts must agree; frozen historical artifacts stay untouched.
Task 7 text must expose the §2.4 split and may not put concrete Task 8 resolver,
plan or writer imports into its prerequisite implementation commit.
It also rejects `enforce` as a public guard-mode value, any public mode set other
than exact `off|observe|warn|deny`, mode-name-to-handoff inference inside the
writer, unconditional clean-NoChange receipt clear without baseline
revalidation, Committed durability-Unknown tombstoning, and an extra staging
transaction container outside the one WAL-owned publication-root name.

### Task 8.0: Back-propagation and prerequisite gate

- [ ] Accept the Task 5B v7 contract/self-audit/independent-review family and
  the Task 6-v2-v7 and Task 7-v6-v7 addendum/self-audit/independent-review
  families; treat their Task 6-v2/Task 7-v6 bases as lineage only. Accept and
  implement the separate
  Task5C-Evidence-v2 design/self-audit/independent-review family,
  create/accept Task 9/10 v6 addenda,
  then update active spec, ADR, skill, schema/observability/replay and platform
  matrix. Record immutable SHA-256 values and no-open-P0/P1/P2 reviews. Do not
  edit frozen v5/historical artifacts.
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
  - one accepted neutral Task 5B v7 Form registry/version shared by edit/validate/
    catalog, no Task 8 table, complete analysis unbound and destination
    generated-name-unbound lookup proofs;
  - observed Around conflict matrix;
  - all source spans/line ending and exact digest encoding;
  - control-staged file/subtree publication, exact allowed directory effects
    and no source parent-by-parent creation/rollback;
  - fixed descriptor-relative control root with no workspace key, separate
    physical-destination-root + canonical-locus semantic key, and one
    backend-qualified root-wide filesystem collision key independent of locus,
    mapping, source/workspace aliases and process cache environment;
  - conservative whole-workspace mount universe, physical control/lock identity
    and actual inode/FileId+Busy proof; APFS NFC/NFD and both case-mode aliases
    share the root-wide inode for Present/Absent; internal shared-destination bind
    rejected;
  - exact qualified Linux ext4/XFS, both macOS APFS case-mode and Windows NTFS
    v2 tuples, WAL, cross-directory file+directory install, metadata and
    source/control-parent durability primitives with fail-before-staging rule;
  - one Windows workspace open followed by handle-relative destination/control/
    staging/install contract, versioned support tuple, exact unsupported
    boundary and honest Present/Absent write guarantees;
  - schema-v3 outcomes on success/failure, unique definite target separated from
    current location/content/metadata, staging cleanup, durability and recovery;
  - bounded dual-slot WAL recovery on every lease and Required receipt
    Prepared->InFlight->StagingIntent->Terminal->transition->ack handoff,
    eligible-only ack->Idle and dirty/durability-unknown/possible blocking Ack;
  - Present plus per-created-file/directory metadata policy/digests in plan,
    grant, WAL and effects, never control-root inheritance;
  - acyclic contract/implementation ledger: separate Task5C-Evidence-v2
    design/self-audit/review -> Evidence implementation, then with Task 5B/6 and
    `Task7PrerequisiteSliceV1` -> Task 8 design -> accepted Task 9/10 addendum
    designs -> Task 8 writer + `Task7Task8IntegrationV1` implementation and
    separate integration evidence -> Task 9 store implementation -> Task 10 handoff
    implementation -> separate Task5C-Mutation-v2 addendum/review -> Mutation
    implementation, with no combined/whole-Task5C prerequisite edge;
  - canonical final-plan equality after optional assertions are validated;
  - grant-scope versus execution-plan digest and rolling baseline.
- [ ] Run `python3 tests/ci/test_product_contracts.py`; expected GREEN for
  accepted text. A zero-assertion accidental pass is failure.
- [ ] Confirm Task 4 empty-watch fixture hashes + parent-watch tags, Task 5
  shared catalog/adoption/form projections, Task 6 extraction spans + observed
  annotations and Task 7 zero-read prepare/report path are GREEN.
- [ ] STOP if any prerequisite still defaults Context/IsFunction, treats
  ExtensionRequired as eligible, collapses Own/mismatch into borrow, lacks a
  complete accepted Task 5B v7 neutral Form registry/dual-source auxiliary catalog/
  Around fact, or the Task 5B-v7/Task 6-v2-v7/Task 7-v6-v7 successor families
  have open P0/P1/P2, or Task 7 presents its concrete Task 8 integration/full
  implementation as an upstream gate instead of the exact §2.4 split; accepts a declared-kind-only
  Configuration or wrapper UUID as CFE base authority, roots locks in a process
  cache or `<workspaceKey>`, keys artifacts by mapping/source name, uses a
  path-based Windows destination mutation primitive, drops typed failure/current
  observations, forces Committed to be clean, lacks WAL/metadata/durability/staging-name
  authority, accepts internal mount/network/unknown backend, equates key bytes
  with the lock inode, retains assertion presence in the final plan, lacks exact
  source ranges/line ending, stages under source, incrementally mkdirs, clears
  Terminal before receipt handoff or uses schema v2.

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
  - every ModulePath/name component is accepted only through
    `parse_complete_bsl_identifier_v1`; keyword, padding, trailing token,
    512-byte/128-scalar N/N+1 and generated concatenation cases map to the
    correct Task 8 reason without a local lexical branch;
  - omitted and explicit-matching assertions resolve to value-equal
    `ResolvedCfeMethodPatchCore` and full plan fixtures; only detached diagnostic
    flags differ;
  - structural aliases normalize, user identifier spelling does not;
  - `PanicOnMaterialRead` snapshot/material/filesystem fakes prove prepare makes
    zero material calls while deriving exact parent/watch plus physical-root
    candidate and canonical locus; a Form case requests typed source-root
    capture without touching/formatting sidecar manifest keys, and it does not
    invent a path-derived key.
- [ ] Run
  `cargo test --locked -p unica-coder cfe_method_patch::tests -- --nocapture`;
  expected RED because domain/prepare types do not exist; zero tests is failure.
- [ ] Implement closed types, the module-path parser consuming the accepted
  Task 6 standalone identifier authority, prepared request,
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

### Task 8.3: Consume accepted shared source extraction RED/GREEN

- [ ] Verify the accepted Task 6 v2+v7-addendum tests are already GREEN for
  definition/declaration/name/parameter/body/terminator spans under RU, EN,
  mixed tokens, CRLF/LF/bare-CR, BOM and Unicode names, plus the shared standalone
  identifier constructor's exact lexer parity, keyword/trailing-input and
  512-byte/128-scalar N/N+1 cases; Task 8 does not edit them.
- [ ] Add Task 8 material-consumer RED tests for:
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
- [ ] Run accepted Task 6 parser tests (must remain GREEN) plus new CFE material
  tests (expected RED only in the Task 8 consumer).
- [ ] Implement `CfeResolutionMaterialPort` source projection using the accepted
  spans/parser and verified snapshot bytes; do not change Task 6 semantics.
- [ ] Re-run all Task 6 evidence/cache tests to prove no semantic regression.

### Task 8.4: Adopted UUID chain and form mechanism RED/GREEN

- [ ] Add Russian/English Platform XML fixtures:
  - exact Adopted root; exact Adopted root+form;
  - missing/Own/wrong-case/duplicate ObjectBelonging;
  - missing/duplicate/malformed/cross-object ExtendedConfigurationObject;
  - same-name extension-owned object decoy;
  - Extension-as-analysis wrapper UUID equal/different decoys never substitute
    for `BaseOwnedMetadataIdentityV1.object_uuid` and produce no plan;
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
  - independently parsed Task 5B auxiliary analysis catalog lookup Unbound versus Bound;
  - independently parsed destination auxiliary catalog generated-name Unbound versus Bound,
    including canonical case identity and orphan BSL case;
  - opaque registered Form handle and view binding agree for each side;
    nonregistered Form, wrong source/catalog/fingerprint/content digest and
    cross-Form/snapshot complete-catalog replay fail inconclusive before lookup;
  - Configuration and registered-Form catalog sets have identical composite
    snapshot/source coverage/order; the selected sidecar binds its exact owner,
    Form, wrapper membership, Configuration catalog digest and opaque
    descriptor/Form.xml keys. Missing/duplicate/cross-side keys fail without
    canonical-ref path inference;
  - each closed `TypedFormCatalogFailureV2` maps to inconclusive with only its
    exact tag/validated optional span and no fabricated provider record;
  - registered-authority mismatch, invalid ArtifactRef, wrong kind and
    foreign-FormModule lookups return their exact closed
    `InvalidFormMethodLookupV2` tags and map to inconclusive, never Unbound or a
    raw/display detail;
    multiple legal matches return the exact upstream full-set digest and checked
    `NonZeroU32` count; the count is bounded by the already-proven outer capture
    node limit, with no separate binding/matching failure or local N+1 and no
    u16/first-row truncation in Task 8;
  - incomplete auxiliary catalog and mismatched/unaccepted
    `PlatformFormBindingRegistryVersionV2` fail closed without inspecting internal rows.
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
- [ ] Run Task 5 providers/support + accepted registry/material tests; they must
  remain GREEN. Only the new Task 8 consumption tests are expected RED before
  this slice is implemented.
- [ ] Implement only Task 8 consumption of the already accepted opaque V2
  catalog/mutation-properties/source-bound membership/Form lookup seams. If any
  typed projection is missing, STOP and return it to the versioned Task 5B v7
  owner; do not amend Task 5 in the Task 8 commit or parse Form rows locally.
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
  - every closed non-usable NamePrefix and ScriptVariant authority row from
    §9.5, including each Inconclusive problem tag and exact leading reason;
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
  + line ending, publication shape and all metadata policy/expected digests.
- [ ] Assert grant scope contains Base/Extension flavor tags, Own/adopted UUID
  chain, analysis source-method-unbound and destination generated-name-unbound
  semantic proofs; both exact Form.xml material identities enter
  execution/baseline.
- [ ] Add same-artifact batch cases: Before+After same source succeeds;
  generated-name collision, duplicate annotation and ChangeAndValidate/hook
  conflict mark both proposals and issue no partial grant vector.
- [ ] Add pure grant/effect algebra RED tests, with no store or receipt lease:
  - two immutable grants A/B over baseline value S0; applying the supplied pure
    post-state transform for A yields S1 without rewriting either grant; a fresh
    B execution plan over S1 still matches B scope and has zero already-created
    parent effects;
  - `receipt_advance_eligible(outcome, plan, manifest_diff)` is true only for
    one Created-NotApplicable or Updated-both-MatchesCaptured target with
    AtIntendedPath+Matches, Known expected content, VerifiedExact metadata,
    exact created-directory metadata, VerifiedDurable+VerifiedClean,
    no unexpected/possible rows and matching manifest; false for every Unknown/
    Mismatch/DetachedOrRelocated, durability Unknown, NoChange, Residue/Unknown
    cleanup, recovered ambiguity and Uncertain;
  - constructor matrix proves definite install + failed path/read/stat/FileId/
    metadata observation is Committed, never Uncertain; only unknown rename
    completion admits PossibleTargetInstall;
  - recording fakes prove Task 8 exposes the artifact-held plan/outcome/post-
    snapshot seam but has no receipt-store/lease implementation or production
    revision transition.
- [ ] In Task 9/10 v6 addenda place persistent schema-v3/WAL-correlation and
  production artifact->receipt InFlight/Terminal/transition/ack/tombstone REDs,
  advance/revoke, reconciliation and integration. Those production suites are
  not a Task 8 final gate.
- [ ] Run discovery/determinism/grant-and-effect algebra tests; expected RED.
- [ ] Implement three encoders, resolved issuance rows, pure eligibility algebra
  and recording seams plus Task 9/10 v6 addenda. Do not implement
  receipt persistence, receipt lease or guard policy in Task 8.
- [ ] Re-run expected GREEN.

### Task 8.8: HandlerInvocation, dry-run and atomic apply RED/GREEN

- [ ] Add application fake RED tests:
  - Preview prepare/capture/resolve once and no lease; Applied parses/prepares
    once, opens workspace/destination root, acquires artifact lease once,
    recovers WAL before capture, refreshes topology from the typed request once,
    then authoritative capture/
    resolve once and one final precondition recapture;
  - changed refreshed physical destination/locus returns
    `cfe_artifact_scope_changed` before material capture; unrelated mapping-
    digest change with the same physical root/locus updates the seed under the
    same held key;
  - handler and Task 8 post-mutation recording seam receive same plan/witness
    addresses and both digests; Applied invocation moves exactly one explicit
    Required/NotRequired handoff into the writer, while Preview/non-CFE carries
    none; missing, duplicated or context/mode-reconstructed handoff fails; the
    future receipt insertion seam is abstract and no receipt store/lease/revision
    service is instantiated;
  - NotRequired Applied calls still require artifact lease+WAL;
    Required calls expose exact receipt-handoff correlation and cannot clear
    Terminal before Task 10 acknowledgment;
  - every lease acquisition receives the operation-neutral recovery handoff
    port even when the new call is NotRequired, so a prior Required Prepared/
    Terminal/Ack is reconciled or blocks rather than being skipped;
  - writer inputs contain no rollout-mode string. Task 10 fixtures cover exact
    off/observe/warn/deny independently from NotRequired/Required: off is
    NotRequired, deny+valid leased receipt reaches Required, deny without one
    never calls the writer, warn/observe reach Required only with a real
    validated lease and otherwise NotRequired, and `enforce` is rejected as an
    unknown public mode;
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
  - busy/failed/mount-boundary/unqualified-backend or non-Idle unrecoverable WAL
    causes zero capture/handler/source write; any control directories/lock created before the failure
    appear only in bounded `ControlPlaneLeaseObservationV1`, never mutation effects;
  - raw args mutated after resolve cannot alter target/output;
  - WAL component fixtures round-trip Unix non-UTF-8 bytes and Windows exact
    UTF-16 code units (including an unpaired surrogate) without display/Unicode/
    case normalization; platform-tag mismatch and separator/dot/empty/overflow
    components fail before Prepared;
  - CFE missing plan and non-CFE unexpected plan reject;
  - CFE dry-run never reaches generic placeholder and writes/emits nothing.
- [ ] Add native/atomic RED tests:
  - type constructors reject NoChange with source effects, ConsumedByPublication or
    an incomplete/unbounded cleanup set; accept complete Residue/Unknown as a
    blocking non-install outcome; reject Committed without exactly one target;
    reject Uncertain with a definite target. Definite Committed accepts every
    location/content/metadata x cleanup x durability combination without losing
    target certainty;
  - recovery-disposition matrix accepts only Normal/RecoveredNoInstall for
    NoChange, Normal/RecoveredDefiniteInstall for Committed and
    Normal/RecoveredPossibleInstall for Uncertain; recovered generation is
    nonzero and below Terminal, while cross-kind/zero/equal/future tags fail;
  - control-staged file root T consumed by one source rename ->
    ConsumedByPublication(TargetIsPublicationRoot) clean; staged subtree root R
    with target file T -> ConsumedByPublication(TargetIsDescendant) clean only
    with exact plan-bound relation; an extra control name -> Residue. Relocated or
    unqueryable target T with absent control name remains clean but nonadvancing;
  - exact present/absent precondition success;
  - analysis/destination/config/descriptor/module/topology change -> zero write;
  - mutate each watched analysis/destination module/descriptor/Form/mapping row
    after durable InFlight+StagingReady but before InstallIntent: the second
    same-watch comparison writes CleanupIntent, installs nothing, returns exact
    NoChange cleanup and clears only on unchanged baseline else revokes;
  - exact missing suffix is built only as one control subtree with per-object
    Created metadata; no source name appears before the one install and no
    unplanned/symlink/reparse/case traversal;
  - failure after each control-staging step removes only the WAL-owned tree and
    flushes its control parent; complete cleanup yields clean
    NoChange+Unmodified, while identity/remove/query/sync failure yields typed
    NoChange+Residue/Unknown+non-Idle WAL;
  - Absent no-replace loses safely to external creator; unsupported platform
    primitive fails closed;
  - qualified Linux ext4/XFS and both macOS APFS case-mode rows run exact flock,
    dual-slot WAL, cross-directory Present-file/Absent-file/Absent-subtree
    rename, Unix metadata, two-parent durability, two-process and hard-crash
    suites; unqualified rows fail before control staging even if one syscall
    probe succeeds;
  - exhaustive metadata REDs cover Unix mode/uid/gid/nlink/ACL/xattr/flags and
    Windows owner/group/DACL/basic attrs/ADS/reparse/compressed/encrypted/sparse;
    Present copies only the closed policy, while Absent derives target+every
    directory policy from retained source parent. Control-root restrictive
    mode/DACL is deliberately different and must not appear after install.
    Present timestamp REDs change the captured old value before final check,
    fail setting/querying it, hard-kill after StagingReady/install, and prove
    private WAL recovery witness plus public relation/mismatch mask without raw
    time in semantic/receipt digests;
  - two-slot WAL REDs cover torn/oversize/unknown-tag/equal-generation-divergent
    records, generation overflow, staged identity-checkpoint mismatch, orphan
    control tree, every intent/syscall window including `CleanupIntent`, bounded
    eligible-only tombstone GC and two-process recovery; a plan whose largest
    reachable Terminal/Ack frame exceeds 65,536 bytes fails capacity preflight
    before `Prepared` with no truncation or request WAL/staging/source mutation;
    fixed prior lease/Idle initialization remains separately observed;
  - native Windows backend opens workspace by trusted absolute path once, then
    uses retained root-relative handles for destination, control WAL/staging and
    source receiving parent; junction/rename swaps cannot redirect the write.
    Qualified NTFS proves dual-slot flush, file+nonempty-subtree
    FileRenameInfoEx, owner/group/DACL/attrs/ADS/special-state metadata gates,
    staged/installed FlushFileBuffers and flags-zero NtFlush on source **and**
    control parents. Inject failure/hard crash at every seam. Only the exact
    matched transcript mints durability; missing binding/export/inheritance/
    namespace proof fails before control staging;
    path-based destination `CreateFileW`, `MoveFileExW`, `CreateDirectoryW`,
    `DeleteFileW` and `RemoveDirectoryW`
    use is statically rejected;
  - move the original retained parent itself after final walk: definite commit
    returns non-advancing Committed with the same target identity and
    location=DetachedOrRelocated (or Unknown if query fails), while cleanup is
    independent; only unknown rename completion returns Uncertain;
  - mutate content and every supported metadata field through the retained old
    Present object after final check but before replace: the target commit stays
    definite, Updated replaced-prestate is Different/Unknown and receipt
    eligibility is false. A completed change before final check remains
    NoChange; a path-object swap in the irreducible window is documented as the
    non-cooperating namespace-CAS exclusion, not silently claimed prevented;
  - Present completed external edit before final check rejects; cooperative
    same-artifact writers serialize; no test claims arbitrary-writer CAS;
  - pre-InstallIntent failure preserves source and returns NoChange with exact
    cleanup: VerifiedClean only after durable cleanup, otherwise
    Residue/Unknown with a blocking WAL. Post-InstallIntent error is classified
    by install observation, never rollback;
  - inject failure/hard-kill after final precondition before Prepared, after
    durable Prepared before/during InFlight, after durable InFlight before
    StagingIntent, and before/after every later WAL slot write, staging mkdir/
    write/metadata/data+directory flush, rename, target/location/content/
    metadata observation, target/source-parent/control-parent flush,
    CleanupIntent, Terminal, receipt transition, ack, eligible Idle and blocking
    Ack. Assert exact v3 outcome/recovery even when
    adapter `ok=false`; definite rename plus any observation failure remains
    Committed; ambiguous rename is Uncertain; no source name is manifest-ignored;
  - hard-kill/remount after a Committed target or either renamed parent whose
    durability proof is Unknown retains Terminal/Ack and its owned authorization;
    it must not reach Idle or turn a reappearing staging name into an orphan;
  - clean NoChange with exact unchanged baseline clears or accepts absent
    InFlight idempotently; the same clean outcome with drift, query failure or
    recovery-time inconclusive baseline revokes. A recording port that returns a
    clear/advance outside the Terminal directive or mismatched authority digest
    leaves Terminal blocking;
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
  lookups on both analysis and destination sides under one accepted Task 5B v7
  registry version/current opaque registered handles, analysis Plain versus
  adopted-destination Borrowed flavor, and no Form path inference or provider-
  evidence transport, without copying its internal rows. It also explains:
  - declared Configuration/Extension labels are insufficient without captured
    BaseConfiguration + Own / ExtensionConfiguration + Adopted proof;
  - omitted matching assertions disappear from the canonical resolved plan;
  - artifact busy behavior, fixed cache-independent descriptor-relative control
    root with no workspace key, whole-workspace mount universe, physical
    control/lock-object proof and rejection of internal shared-destination bind;
  - atomic subtree directory effects, definite target current observations,
    Removed/ConsumedByPublication control staging,
    VerifiedClean/Residue/Unknown and
    VerifiedDurable/Unknown consequences even
    when adapter presentation reports failure; NoChange excludes control setup;
  - exact qualified Unix/Windows v2 backend matrix and fail-before-control-staging
    behavior for network/FUSE/unknown/unqualified tuples;
  - Windows CFE apply uses one workspace open and handle-relative destination/
    control/WAL/staging/source-parent operations; path-based reopen/`MoveFileExW`
    is forbidden, and an unsupported v2 tuple fails before control staging;
  - WAL recovery precedes capture; Required terminal handoff ordering;
    Present/Created metadata including every moved directory and no control-root
    metadata leakage;
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
cargo test --locked -p unica-coder artifact_writer_wal -- --nocapture
cargo test --locked -p unica-coder artifact_metadata -- --nocapture
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
  event/callType/Action registry/table/parser and no canonical-ref-to-Form-path
  formatter; it imports the accepted same-snapshot Form sidecar, opaque handle/
  keys, one Task 5B v7 neutral parser/registry version and complete lookup only.
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
  registered-handle/sidecar replay and analysis Plain/destination Borrowed
  membership-flavor matrix, plus tracked base-with-compat and extension-without-
  compat/KeepMapping. Run the
  authoritative Task 5B registry matrix/lexical suite separately; Task 8 owns no
  duplicate fixtures/table.
- [ ] Run exhaustive failure/hard-kill injection and record schema-v3 outcome,
  target observations, directory metadata, cleanup, durability/recovery at every
  WAL write, control mkdir/write/metadata/flush, rename, observation, target/
  source-parent/control-parent flush, Terminal, receipt handoff and Idle seam.
- [ ] Run focused lock, cross-directory Present-file/Absent-file/Absent-subtree,
  metadata, dual-slot WAL and hard-crash/restart suite on every enabled Linux ext4/
  XFS, both macOS APFS case-mode and Windows NTFS tuples. A skipped/unrecorded tuple remains
  disabled and blocks that platform claim; network/FUSE/unknown rejection runs.
- [ ] Record fixed Task 4 empty-watch hashes and all three CFE digest fixtures.
- [ ] Confirm no persistent receipt store/revision, receipt lease, guard policy
  or production `discovery_receipts` integration test was implemented in Task 8;
  their exact RED/GREEN ownership is recorded in Task 9/10 v6 addenda. Task 8
  still implements the generic WAL and receipt-handoff seam.
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
   wrapper UUID and the separate Configuration-root UUID authority never
   substitute for a `BaseOwnedMetadataIdentityV1.object_uuid` adoption join.
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
12. Shared `KnownScriptVariant`, same-snapshot
   `PlatformConfigurationCatalogSetV1` + `RegisteredManagedFormCatalogSetV1`,
   exact ScriptVariant/NamePrefix/sidecar authorities, source-bound membership
   companions, opaque Form handles/manifest keys, accepted Task 5B v7 neutral
   `PlatformFormBindingRegistryVersionV2`/complete V2 lookup and BSL parsers are
   reused; Task 8 defines no Form registry/table/parser or Form path inference.
13. Declared source kind is never flavor authority: the captured analysis
    catalog is exactly BaseConfiguration, every contributing analysis root/form
    descriptor is Own, the captured destination catalog is exactly
   ExtensionConfiguration, and destination root/optional form are exactly
   Adopted with extended UUIDs equal to the corresponding Own analysis UUIDs.
   A Form pair additionally requires parser-derived analysis Plain and adopted
   destination Borrowed flavors, matching the upstream companion join.
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
    authority remains solely in Task 5B v7's freshly accepted exhaustive suite.
17. Present/absent destination module and every parent component are captured
    atomically; absence has typed tombstones and an unregistered physical decoy
    or unsafe parent is never opened.
18. Empty-watch Task 4 capture and fingerprints are byte-identical; nonempty
    parent state tags have fixed independent fixtures.
19. A first patch builds the complete missing suffix plus target only below
    control staging and publishes it with one no-replace directory rename. No
    source name exists before install. Preinstall failure removes/syncs only the
    WAL-owned control tree and returns NoChange+Unmodified; cleanup failure
    returns Residue/Unknown on that NoChange plus non-Idle WAL, never source
    rollback or install uncertainty.
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
    fingerprint/module/output appear only in execution plan/baseline. Metadata
    policy/version/scope is immutable grant authority; current Present before
    metadata, publication shape and every expected created-object digest are
    execution authority.
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
28. Every CFE call that Task 10 permits to apply under the closed public modes
    off/observe/warn/deny opens the workspace once,
    proves one supported same-mount/volume whole-workspace universe and qualified
    backend, resolves destination handle-relative, derives its separate semantic
    physical-destination/canonical-locus key, then acquires the physical-control
    universe + root-wide filesystem-collision process/OS artifact lease,
    recovers the WAL before capture, then refreshes authoritative support/
    topology under the same witness. It keeps the actual inode/capability through
    handler, typed outcome, Terminal handoff and post-snapshot. The control path is
    exactly `.build/unica/project-discovery/control-v1` with no workspace key;
    cache root, mapping digest, source alias, cwd bytes and PID cannot split it.
    Whole-workspace aliases prove one control root/collision lock inode+Busy; an
    internal destination bind is rejected. APFS case-sensitive/insensitive
    NFC/NFD and case-only Present/Absent aliases share that inode even when
    semantic hashes differ. Equal semantic or collision-key bytes alone are not
    actual-lock proof.
29. Task 8 proves the generic WAL/writer and pure/recording downstream seams.
    Task 9/10 v6 addenda own
    persistent receipt store/revision, current receipt lease, production
    artifact->receipt acquisition/reverse release, reconciliation and the
    `discovery_receipts` integration suite; none is faked as production-green in
    Task 8.
30. The Task 8 applied path performs one final same-watch recapture, exact plan/
    physical-scope comparison, parent/target content+metadata precondition and after-digest
    verification without resolving/rerendering another plan. Required is chosen
    only from a real validated leased receipt, never from the public mode name;
    NotRequired creates no receipt row. The recording seam writes
    Prepared(correlation) first, then permits Task 10 to durably insert
    receipt InFlight bound to that generation before StagingIntent/source
    install, without changing the plan or artifact capability. Terminal cannot
    tombstone until Task 10's correlated transition is durable and acknowledged,
    and dirty cleanup, durability Unknown or possible install cannot tombstone
    even then.
31. Present file replacement and Absent file/nonempty-subtree publication are
    cross-directory control-to-source atomic operations; Absent never overwrites
    an external winner. Linux ext4/XFS, both APFS case modes and NTFS each need
    independent v2 evidence for dual-slot WAL, file+directory rename semantics,
    metadata and source/control-parent durability. Missing evidence and network/
    FUSE/unknown fail before control staging. Windows path-opens only workspace;
    all destination/control/WAL/staging/install operations are rooted retained
    handles. Path `MoveFileExW`/destination `CreateFileW`/checked mkdir/delete is
    forbidden. `FlushFileBuffers` plus flags-zero `NtFlushBuffersFileEx` on both
    source receiving and control source parents is mandatory; no success-only
    probe or source-parent staging fallback can qualify.
32. Present replacement is atomic and serializable for cooperating Unica
    writers. Documentation/tests explicitly do not claim portable content-CAS
    against a non-cooperating writer in the final check/replace window.
33. Every applied handler returns a typed NoChange/Committed/Uncertain outcome
    even when `AdapterOutcome.ok=false`. Committed owns exactly one
    `DefiniteTargetCommitV1`; location/content/metadata are independent current
    observations. Non-target directory effects include identity/location/
    metadata; control staging has exact Removed/ConsumedByPublication
    root/target-relation lifecycle; durability
    and recovery are independent. NoChange always proves source Unmodified and
    carries exact cleanup; NoChange requires its durably VerifiedClean cleanup
    to tombstone, while Committed additionally requires VerifiedDurable.
    Residue/Unknown or durability Unknown blocks even after receipt revocation.
    Created requires NotApplicableForCreated;
    Updated requires both retained replaced-prestate observations
    MatchesCaptured to advance. Only AtIntendedPath+Matches+Known expected content+VerifiedExact
    metadata+exact directories+VerifiedDurable+VerifiedClean with no possible/
    unexpected rows advances. Any Unknown/Mismatch/Relocated, recovery ambiguity,
    Residue/Unknown or Uncertain revokes. Display is never authority.
34. Handler cannot reparse raw args, select source/path/context/kind, parse XML
    or rerender; it consumes one under-witness-authorized generic writer plan.
    Every directory is created only in the control subtree and source changes by
    one install.
35. Valid adopted Russian Before/After retain donor semantic structure but use
    the corrected collision-free generated names; English and source-bound
    clone scenarios have independent golden/compile proof. Historical donor
    byte parity is explicitly not asserted for generated names/signatures.
36. Task 8 adds no public discovery tool, receipt store/receipt lease, guard
    policy, observation service, tool/package registration, release or Form.xml
    mutation; prerequisite schema/WAL/observability text and v2 backend support
    matrix/product CI are updated through versioned addenda/active docs. The
    mandatory artifact lease, generic WAL/writer and handoff seam are in scope. Its final gate does
    not run or claim production `discovery_receipts`; Tasks 9/10 own that proof.
37. Production Task 8 code remains STOP until the separate Task5C-Evidence-v2
    family has distinct design/self-audit/independent-review SHA-256 values plus
    its exact `TASK5C_EVIDENCE_ACCEPTED_GIT_OID`, and the Task 5B-v7,
    Task 6-v2-v7-addendum and Task 7-v6-v7-addendum families, Task 5B/6
    implementations and exact `Task7PrerequisiteSliceV1` implementation have their exact
    required ledger values with no P0/P1/P2, and the Task 9/10 v6 **addendum designs**
    have distinct accepted design/independent-review ledger values. Task 9/10
    **implementation commits are downstream of Task 8** and are not Task 8 code
    prerequisites. Concrete `Task7Task8IntegrationV1` is likewise part of Task
    8 delivery, not an upstream full-Task7 gate, and must have its separate
    integration evidence before Task 8 is accepted. Older hashes, self-audit
    substituted for review, or frozen-file edits do not satisfy the gate.
38. A dual-slot WAL is durable before source install, recovered under every
    root-wide lease before capture, bounded/tombstoned exactly as §1.8, private
    and fail-closed on corruption/ambiguity/orphans. Hard-kill tests cover every
    syscall-versus-journal window and multi-process recovery.
39. Present and Created metadata use one closed policy/version. Created metadata
    covers target plus every moved directory and is derived from source parent,
    never control root. Old mtime/LastWriteTime is not preserved: the stable
    content-change timestamp policy proves a strictly newer visible value.
    Volatile bytes stay outside semantic plan/grant/outcome/receipt/metadata
    digests and public diagnostics; the private WAL frame alone protects the
    bounded exact Present recovery witness needed after a crash.
40. For Required handoff, final precondition precedes durable Prepared, and receipt
    InFlight for that exact Prepared generation is durable before StagingIntent
    or any control/source mutation; Terminal
    remains until correlated receipt baseline-revalidated clear, post-state-
    validated advance, or revoke and WAL acknowledgment;
    only a recomputed clean NoChange or clean+VerifiedDurable Committed then
    reaches Idle parent sync, while dirty/durability-unknown/possible state
    remains blocking Ack. Clean NoChange does not authorize clear by itself:
    baseline drift/inconclusive revalidation revokes. Every boundary crash is
    idempotently reconciled under artifact->receipt locks. NotRequired has its
    own eligible-only recovery/GC proof.
41. Early support is routing only. Authoritative operation policy/material is
    re-read under the recovered root-wide witness; the same witness is retained
    through handler/outcome/handoff. CFE, `support.edit` and future callers never
    acquire a second artifact lease or clone a path protocol.
42. Task 7's upstream gate is exactly independently implementable
    `Task7PrerequisiteSliceV1` with zero concrete Task 8 import. The Task 8
    delivery implements `Task7Task8IntegrationV1` and records its distinct
    integration evidence hash together with the concrete resolver/writer; no
    ledger or active-spec clause creates a full-Task7-before-Task8 cycle.

---

## 17. Hard STOP conditions

Stop implementation and show the owner if any condition is true:

- the separate Task5C-Evidence-v2 design/self-audit/review/implementation,
  Task 5B-v7/Task 6-v2-v7-addendum/Task 7-v6-v7-addendum prerequisites,
  Task 9/10 v6 addenda or active
  spec/ADR/skill/schema/platform gates in §§2/15.0 are absent/not GREEN, or a
  frozen predecessor was edited in place;
- any prerequisite lacks any distinct artifact/evidence/implementation ledger
  value required by its exact §2.7 row, with correct 64/40-hex type and zero open
  P0/P1/P2, or its strict flavor/Own/Form/EventSubscription gates are not GREEN;
- Task 7 uses an undifferentiated full implementation as an upstream Task 8
  gate, `Task7PrerequisiteSliceV1` imports a concrete Task 8 type, or Task 8
  delivery lacks accepted `Task7Task8IntegrationV1` evidence;
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
- Task 8 open-codes an `is_alphabetic`/XID/Russian-English identifier grammar,
  accepts a name rejected by the accepted Task 6 lexer, or passes a bare method
  name or opaque `ArtifactIdentityBytesV1` rather than its exact
  module-qualified Method `ArtifactRef` into Form lookup, or treats identity
  bytes as ownership/kind authority before that lookup succeeds, or
  caller-compares a detached registered-Form binding/digest instead of passing
  the exact current `RegisteredPlatformFormV1` handle into the view;
- any definition/declaration/name/parameter/body/terminator range or declaration
  line ending is dropped from the immutable plan/digest, recomputed by search,
  or accepted without exact slice validation;
- Before/After emits empty parameters for a parameterized source method;
- ModificationAndControl emits an empty TODO method, fails to preserve source
  body, uses substring splice or lacks real RU/EN platform validation;
- Task 8 defines or copies any Form definition/item/event/callType/Action row,
  parser or lexical/cardinality rule instead of consuming the accepted Task 5B v7
  versioned complete lookup;
- Task 8 ignores the same-snapshot registered-Form sidecar, formats a Form
  descriptor/Form.xml path from canonical ref text, obtains an opaque sidecar
  key during zero-read prepare, swaps a key/catalog across sources, or accepts
  analysis Form flavor other than Plain / adopted destination flavor other than
  Borrowed;
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
  rejected because `<Name>/Ext` is absent, or it incrementally creates/rolls
  back source parents instead of one control-staged subtree rename;
- unregistered physical decoy is opened/promoted by SnapshotWatch;
- nonempty watch default silently ignores watches or empty-watch hash changes;
- watch/proposal ID, absolute path, raw alias, receipt ID, timestamp or
  workspaceEpoch enters a canonical CFE digest;
- prepared optional-assertion presence, alias spelling or assertion-status flags
  survive in `ResolvedCfeMethodPatchCore`, the immutable plan, a grant or the
  handler; omitted and explicit-matching inputs are not value-equal plans;
- grant-scope digest includes mutable source/composite fingerprint,
  current absent parent suffix, destination module before/after or rendered
  output, current Present/Absent metadata, or synthesized per-object expected
  metadata instead of only stable policy/synthesis scope;
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
  qualified backend, acquiring the artifact lease, recovering WAL and
  refreshing authoritative support/topology under that same witness; an early
  support check is treated as authority, a second lease is acquired, refresh can switch physical root/locus
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
- applied path skips the final same-watch precondition recapture after the
  current receipt lease/baseline reread when Required, performs Prepared or InFlight before
  that recapture, or proceeds from StagingReady to InstallIntent without the
  second exact same-watch/material+parent/target comparison immediately before
  install;
- Absent target uses an overwriting/check-then-rename install, or Present replace
  is described/tested as portable CAS against arbitrary external writers;
- Windows CFE containment path-opens/reopens the destination root, uses path-
  based `MoveFileExW`/`CreateDirectoryW`/`DeleteFileW`/`RemoveDirectoryW`, or
  cannot bind every destination/control/WAL/staged file-or-subtree/source-parent
  install/cleanup operation after the one workspace open to retained reparse-
  checked handles; it creates source temp/parents; an unsupported backend stages
  or mutates source before failing closed;
- a claimed Windows row omits exact handle-based NTFS/volume-serial and
  `FileFsDeviceInformation` mounted+writable non-remote/non-virtual proof, or
  remote/SMB/ReFS/FAT/read-only/virtual material reaches the source writer;
- a claimed Windows tuple omits exact owner/group/DACL/basic-attrs/ADS/reparse/
  compression/encryption/sparse policy, file+nonempty-directory
  `FileRenameInfoEx`, control cleanup, `FlushFileBuffers` plus flags-zero
  `NtFlushBuffersFileEx` on both source receiving and control source parents,
  accepts an unavailable ntdll binding/export, substitutes a
  data-only/no-sync/volume/path-reopened flush, mints durability from syscall
  success without matching hard-crash evidence, or mints structural clean without retained-parent name and identity
  observation;
- Linux/macOS use generic `cfg(unix)`, syscall-presence or filesystem-name
  fallback; NFS/CIFS/SMB/FUSE/overlay/tmpfs/network/unknown/unqualified tuple
  reaches control staging/source mutation; or any claimed tuple lacks exact WAL,
  cross-directory file+directory install, metadata, both-parent durability and
  native race/failure/hard-crash evidence;
- apply uses create_dir_all, creates a source temp/unplanned parent, stages under
  target parent, inserts a separate transaction-container directory around the
  one `<transaction-id>` publication root, follows a link/reparse, writes in
  place or writes outside exact allowed effects;
- an applied CFE handler returns only `AdapterOutcome`, returns `Err` after the
  durable Prepared state for an ordinary non-crash failure, loses target/non-target/metadata/cleanup/recovery
  authority, maps definite install plus failed observation to Uncertain/NoChange,
  cannot construct Committed with Unknown/Mismatch/Residue/durability Unknown,
  duplicates/omits the unique target, loses physical identity, marks clean with
  an unaccounted control name, mints durability without target/published-tree/
  source-parent/control-parent proof, or Task 10 advances anything other than
  the exact fully observed VerifiedDurable+VerifiedClean schema-v3 shape; a
  recovery disposition contradicts its NoChange/Committed/Uncertain kind or
  carries a zero/equal/future prior generation;
- NoChange is constructed without source Unmodified or without one complete
  bounded exact control-cleanup state, source rollback exists as a Task 8 path,
  dirty NoChange is tombstoned/treated as clean or fails to revoke, or control
  setup is mixed into grant/source effects;
- the writer derives Required/NotRequired from `off|observe|warn|deny` rather
  than the validated leased-receipt handoff value, accepts public mode `enforce`,
  or a clean NoChange clears/uses `InFlightAbsentNoOp` without exact current-
  baseline revalidation; drift/inconclusive baseline fails to revoke;
- Committed with durability Unknown is tombstoned/advanced, its Terminal/Ack
  loses recovery authority, or a hard-crash rollback can turn a previously
  WAL-owned staging name into an Idle-state orphan;
- WAL is absent in any NotRequired call, written after source install, unbounded,
  lacks exact slot/parent flush, is cleared on corruption/ambiguity/orphan, or
  recovery does not run under every root-wide lease before capture;
- a durable intent is treated as proof a syscall did/did not run, a process
  retries rename without identity observation, or hard-kill/restart coverage
  omits any syscall-versus-journal window;
- Required handoff writes InFlight before Prepared, permits StagingIntent/control or
  source mutation without an exact durable InFlight proof bound to that Prepared
  generation, clears a recovered Required Prepared without the recovery port,
  or source install precedes durable receipt InFlight; Terminal WAL is
  cleared before durable receipt clear/advance/revoke+ack, the returned
  transition violates its Terminal directive, correlation/authority digests are not
  idempotently compared, or lock order differs from artifact->receipt;
- Present replacement silently loses mode/uid/gid/nlink/ACL/xattr/flags or
  Windows owner/group/DACL/basic attrs/ADS/reparse/compressed/encrypted/sparse
  state; Absent target/any moved directory inherits control-root metadata; old
  mtime/LastWriteTime is preserved; volatile timestamp bytes enter semantic
  plan/grant/outcome/receipt/metadata digests or public diagnostics; the private
  WAL omits/fails to integrity-bind the exact bounded Present recovery witness;
  or unsupported metadata reaches source install;
- a definite installed staged identity is represented as Uncertain/NoChange
  because current path/content/FileId/stat/metadata observation failed, target
  is duplicated in effects, or an unknown rename completion is represented as
  a definite commit;
- Task 8 adds receipt persistence/lease/revision/policy/public discovery/tool
  package registration, or its final gate runs/claims production
  `discovery_receipts` before Tasks 9/10. Generic WAL and receipt handoff seams
  are mandatory Task 8 scope; schema/observability/support-matrix changes flow
  through new addenda/active docs, never frozen-file edits.

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

This v6 artifact is ready only after its self-audit and a separate independent
review each exist with distinct immutable SHA-256 values. Production Task 8
implementation remains STOP until
the separate immutable+implemented Task5C-Evidence-v2 family, the new Task 5B
v7/Task 6-v2-v7-addendum/Task 7-v6-v7-addendum prerequisites, and the
Task 9/10 v6 **addendum designs plus independent reviews** are accepted with no
open P0/P1/P2 and all §15 RED gates are GREEN. The Task 9 store and Task 10
handoff **implementations remain downstream of the accepted Task 8 writer**;
they are not prerequisites for writing it. Frozen v5 designs/reviews remain
byte-exact. The semantic resolver remains one Configuration-source-bound,
two-stage resolver with one authoritative under-witness refresh and one
canonical current-call plan; it keeps all v5 source/adoption/Form/signature
corrections while adding recovery and metadata authority.

The control plane is one fixed descriptor-relative directory in one qualified
whole-workspace mount universe. A root-wide lock intentionally over-locks
aliases. Under that lock, every call first recovers a bounded private dual-slot
`unica.artifact-writer-wal.v1` before capture. Linux ext4/XFS, both APFS case
modes and NTFS qualify independently for control-to-source cross-directory
Present file replace, Absent file no-replace and Absent nonempty-subtree
no-replace, closed metadata synthesis, both-parent durability and every hard-
crash window. Missing capability is unsupported before control staging; there
is no target-parent or incremental-source fallback.

Every apply builds the file or complete missing subtree under control, assigns
the exact source-parent-derived metadata to the target and every directory,
flushes/verifies it, then changes source with one rename. Present security/
ownership/attributes are preserved only through the closed policy; stale mtime/
LastWriteTime is never restored. Volatile values stay out of semantic/public
digests, while the private integrity-bound WAL retains only the bounded Present
timestamp recovery witness. The generic exported writer/witness/WAL/outcome seam is reusable by
`support.edit` and later operations, which must perform their authoritative
policy read under the same recovered witness.

Schema `unica.mutation-outcome.v3` separates install certainty from current
observation. A definite installed identity always yields exactly one
`DefiniteTargetCommitV1`, even when location/content/metadata is Unknown;
Uncertain is only unknown rename completion. Cleanup, durability and recovery
remain independent. Task 10 may advance only the exact AtIntendedPath+Matches, Known
expected content, VerifiedExact metadata, Created NotApplicableForCreated or
Updated both-MatchesCaptured replaced-prestate, exact directory effects,
VerifiedClean+VerifiedDurable shape. Required handoff retains Terminal WAL until
receipt InFlight is reconciled to a durable baseline-revalidated clear, advance
or revoke, acknowledged and
only when recomputed eligible tombstoned; otherwise Ack remains blocking.
Committed durability Unknown remains a blocking Terminal/Ack rather than
discarding recovery authority. NotRequired recovery is independently
eligible-only. Task 8 owns this
generic safety machinery and pure/recording seams; Task 9/10 own receipt
persistence, leases, revisions, policy and production integration. Present
replacement remains deliberately cooperative rather than a nonexistent
portable external-writer content CAS.
