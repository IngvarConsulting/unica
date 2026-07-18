# Task 7 v7 successor addendum — owner self-audit

Audit date: 2026-07-18.

Status: **owner self-audit; owner-local P0 = 0, P1 = 0; downstream P2 =
3**. This finding neither declares nor denies candidate or accepted design
state. The sole design-status authority is the external four-document package
ledger `.superpowers/sdd/task-4-7-v7-design-package-acceptance.md`.
Production authorization is separate and depends on the implementation OIDs
specified by the owner contracts; this audit publishes no candidate/accepted
owner hash, review hash or implementation OID value.

This is not an independent review and cannot substitute for one.

## 1. Scope and method

The owner audit covers the exact current bytes of:

```text
.superpowers/sdd/task-7-v6-design.md
.superpowers/sdd/task-7-v7-addendum.md
```

Cross-owner compatibility was checked against the exact current bytes of:

```text
.superpowers/sdd/task-4-v7-dynamic-material-addendum.md
.superpowers/sdd/task-5b-v7-contract.md
.superpowers/sdd/task-6-v2-v7-addendum.md
.superpowers/sdd/task-6-v3-golden-generator.py
```

The historical Task 7 v6 file mechanically matches the immutable lineage
identity recorded in the addendum. No moving/current owner or peer digest is
copied into this audit. The external coordinator binds this audit, the four
owner documents, generator evidence and independent reviews to one exact tuple.
Any later owner-byte edit makes that package evidence stale and requires a
fresh cross-owner audit/review/generator cycle.

Write scope is limited to the Task 7 addendum and this self-audit. Peer owner
documents, Task5C-Evidence, Task 8, active spec, production code, tests and
package metadata were read only. Existing unrelated dirty worktree changes were
not touched or normalized.

## 2. Reconciliation results

### 2.1 Freeze-invariant owner status

The addendum now defines semantics without declaring local draft, blocked,
candidate or accepted state. It embeds no current self/peer/audit/review hash
and no implementation OID value. Section 14 assigns all design status to one
external four-owner ledger transition and separates later production OIDs.

Result: PASS.

### 2.2 Opaque Task 4 material and whole Task 5B context

Task 4 owns the atomic manifests, registered-material authority and composite
`SourceSnapshotV2`; Task 5B owns the context-bound FormXml/Analysis-BSL scan and
read boundaries. Task 7 does not project, reconstruct, serialize, format or
read any of their opaque items.

Task 7 imports only the whole non-forgeable `PlatformCatalogContextV1` built by
the exact object-safe `PlatformCatalogPort::build_context(&SourceSnapshotV2,
&dyn SourceSnapshotPort)` boundary. Its `EvidenceExecutionContext` stores exact
borrows of that context, the composite snapshot and the injected reader. Task 6
receives `snapshot.analysis_snapshot()`; Metadata/Form receive their exact
Task 5B capabilities. Task 7 accepts no detached catalog set, half-context or
locally invented Analysis catalog view and creates no second/hidden reader.
The context is exhaustive rather than a two-witness shorthand: it carries the
configuration and registered-Form catalog sets plus all three exact
configuration, registered-Form and Analysis-BSL witness sets.
The Task 6-facing `AnalysisPlatformCatalogViewV1` is a restricted header view
with no registered-Form iterator/lookup or Form/material capability; Task 8's
separate any-source `PlatformCatalogViewV1` retains the typed Form lookup.
Repeated direct borrowed builds are permitted and deterministic; Task 7's
recording orchestration spy, rather than an impossible linear API claim,
enforces exactly one build per execution.

The final Task 5B any-source verified-platform-Form future-consumer surface is
downstream Task 8 authority. Task 7 neither imports nor restates it.

Task 7 never calls a specialized registered-material reader and never sees a
material item/verification/state/key/path or reconstructs the resolver chain.
MetadataCatalog/FormInspection may call only the context-owned Task 5B FormXml
verification boundary inside their provider calls; only the private Task 5B
context resolves a Task4 handle and invokes the raw registered reader. On the
BSL FormModule path, only Task 6 consumes
`AnalysisBslMaterialScanPlanV1` and
`read_analysis_bsl_material_verified` with the exact Analysis atom and same
injected port; it receives no `RegisteredFormAuthorityViewV1`, material ref or
resolver and cannot call that private chain directly. The Platform catalog
context and all material views/items contain no reader;
the explicit `EvidenceExecutionContext.source_reader` field is the sole reader
capability, and all verifier/read counters belong to that injected port.

The imported result/error matrix is unchanged: Ordinary Present performs
0/1/1 registered verifier/read/parse calls, Registered Present performs 1/1/1,
Registered Missing performs 1/0/0, and Registered NotApplicable, unsupported
Ordinary or per-item `FileBytesLimit` perform 0/0/0 while preserving their
exact typed gap. `FileBytesLimit` is nonterminal and leaves later selected
items eligible; terminal `FileCount`/`TotalBytes` omit the selected suffix and
permit no later material I/O. Semantic authority mismatch remains nonretryable
`registered_material_handle_mismatch` before I/O; only later external
filesystem drift is retryable
`source_fingerprint_mismatch`. Both discard the whole invocation prefix, and
Task 7 association/admission cannot reclassify either.

Result: PASS.

### 2.3 Task 5B query/group authority

The current cross-owner contract is exact:

```text
PlatformCatalogContextV1
PlatformCatalogPort
MetadataCompositeQueryV2
FormSourceSetQueryV2
FormMaterialAssociationBuilderV1
SupportStateQueryV2
ProviderGroupMaterialIdentityV2
SemanticAtomicGroupIdV2
```

Task 7 imports smart query digest accessors byte-for-byte. Pair, presence,
Form-scope and Support subject construction remains typed; Task 7 accepts no
caller-selected inner digest and declares no Metadata/Form/Support encoder.
`FormMaterialAssociationBuilderV1` removes duplicate proposal contributions
from provider work, while `MaterialAssociationMapV2` independently retains why
the material matters.

Result: PASS.

### 2.4 Task 6 query v3 publication

Task 6 now owns and publishes six query-v3 positive goldens for empty/one-member
CodeSearch, Definition and CallGraph plus the forbidden extra-frame negative.
Its framing appends the 148-byte `encode(AtomicSourceIdentityV2)` directly
after the port tag, without another `bytes(...)` frame.

The current Task 6 provider boundary was also checked against Task 4/Task 5B:
its query smart constructors receive only the whole Platform context; provider
execution additionally receives the exact Analysis atomic snapshot and
injected reader. Task 6 alone consumes the owner-defined Analysis BSL scan/read
boundary. Task 7 imports none of those item/verification types. This does not
forbid the separate Task 5B Metadata/Form providers from reading FormXml inside
their own calls.

Task 7 section 2.3 imports only the six final `query_digest()` byte strings.
It names the Task 6 extra-frame value solely as a mandatory rejection sentinel
and neither copies nor reconstructs payload hex, payload length, payload
SHA-256, source framing or `H`. Running the standalone
stdlib-only generator independently completed both encoding paths and all
registry, canonical-order, duplicate, mutation, redundant-frame and historical
v2 assertions. Its run-binding tuple must be refreshed externally after owner
edits; that is an external package gate, not an owner-local semantic P1.

Result: PASS.

### 2.5 Provider/application separation

`MaterialAssociationMapV2` is the only boundary associating provider material
or testimony with application conclusions. Application traversal state and
`TraversalGap` legitimately retain `ConclusionScope`; provider query,
raw/effective outcome, semantic group, physical record, provider gap, cache
identity and retention order contain no Request/Proposal/Mechanism field.
The v6 provider-adjacent `conclusion_scopes` fields and
`EvidenceMaterialSubjectV2::Conclusion` are normatively deleted, not hidden in
a wrapper. Application stage/depth/round lives in
`ApplicationInvocationTraceV2` and does not alter the imported query digest.

Result: PASS.

### 2.6 Invocation, admission and v4 identity closure

The owner now defines every previously implicit successor type:

- typed pre-I/O `RegisteredProviderInvocationPlanV2` containing only the
  imported port/query authority and execution token, never a response-owned
  provider identity;
- consuming `record_terminal(plan, raw_outcome)` validation which records the
  exact returned provider, enforces one provider identity per port, and returns
  a non-forgeable `RecordedProviderInvocationHandleV2` for every post-response
  association; later associations can recover only that already-recorded
  handle through the checked registry lookup;
- exactly one terminal raw outcome and one Invocation root per consumed plan,
  with staged pre-I/O failure rollback and a finish-time
  plan/raw/Invocation-root bijection;
- a Task 5A authoritative Definition work plan with `methods=[]` still produces
  the Task 6-owned typed `DefinitionQuery` and remains a real invocation; only
  absence of a scheduled typed query omits a plan, while an empty association
  scope is separately invalid and cannot schedule I/O;
- raw-only `RawProviderInvocationSnapshotV2` and
  `ScopedProviderRollupSnapshotV2` with exact bytes/order/duplicate rules,
  including raw `retryable` testimony and the complete Task-7-owned raw
  provider-gap/source-location encoder;
- `EffectiveGapOwnerV3` Provider/Admission tags and private
  `StableReasonCodeV3` grammar, reserved codes and constructors;
- the complete v3 effective-outcome digest formula;
- distinct raw `ProviderGapScope` and application-admission scope tag orders;
  PerPort and Global admission are each projected as at most one same-port row
  per reason/outcome, while only EvidenceGapLimit is portless;
- exact application stage depth/round bytes and raw/effective/trace key
  bijections;
- complete `EffectiveGapIdentityBytesV3`, `TraversalGapIdentityBytesV3` and
  `AnalysisExecutionSnapshotIdentityBytesV4` encodings;
- an explicitly additive, exhaustive and ordered v4 registry.

The second-proposal metamorphic rule is provider-invariance rather than the
impossible “map/analysis only” claim: request, traversal trace and proposal
conclusions may also change. Pair projection covers both real halves and their
AtomicGroups, including a shared half, and cross-owner structural goldens cover
all root/scope variants.

Result: PASS.

### 2.7 Acyclic downstream DAG

Task5C-Evidence imports only its Task 4/5A/5B authorities and never imports,
revalidates or waits for Task 7. Task 6 has no Task5C dependency. Only Task 7
production later requires the exact accepted Task5C-Evidence implementation
OID for `SupportStatePort`.

`Task7PrerequisiteSliceV1` is independently implementable with generic
prepare/preflight/report and recording resolver/issuer seams but zero concrete
Task 8 type. Task 8 later owns `Task7Task8IntegrationV1`, concrete resolved-plan
delivery and distinct integration evidence. Task 8 cannot gate or reopen the
four-owner design package.

Result: PASS.

## 3. MaterialAssociationMapV2 mechanical audit

### 3.1 Type and authority separation

- `ProviderInvocationKeyV2` is the exact port plus imported typed query digest.
- `SourceGroup` uses complete `AtomicSourceIdentityV2`.
- `Material` uses the closed `ProviderGroupMaterialIdentityV2`.
- `AtomicGroup` uses the complete `SemanticAtomicGroupIdV2` key under one exact
  invocation.
- every immutable entry has a nonempty sorted-unique scope vector;
- duplicate raw contributions union only in the application builder;
- duplicate final roots/scopes and foreign authority reject;
- the map is never a provider ordering/admission input.

Result: PASS.

### 3.2 Independent encoder reproduction

The audit independently rebuilt the section-3.4 fixtures from fixed-width
big-endian integers, explicit `bytes`/`string`/`vec` framing, MetadataCatalog
port tag 1, query digest byte `0x11` repeated 32 times and:

```text
H(domain, payload) =
  SHA-256(u32be(domain length) || domain
         || u32be(payload length) || payload)
```

The three positive results reproduced exactly:

```text
empty map
  payload length = 6
  SHA-256(payload) =
    3686cb37b7fe4758ac2024a76e30a4c6ee2fdc25c66aa21dd55a9569b91ea504
  H =
    5e15eccbf7fa19a4f376efa6fcbb71ea64f771d72f220ae868754c709f515be4

[Request, Proposal("p")]
  payload length = 59
  SHA-256(payload) =
    048ea3bd5d3218322592de805f5a0b10dc8c9e3829edb1eee68c40d37b4bd273
  H =
    82c0d668ce886e5c27bf241899d0fb72a227ad92bf6726e4657d28e0086e9d96

[Request, Proposal("p"), Proposal("q")]
  payload length = 66
  SHA-256(payload) =
    09791f3637ceb7fc093eaf977931c5c8dc284a06bd796945eda07b21b466fc80
  H =
    f6ae6d13401e23ba46266033598c4edba3598909f4849acbedf7c85b014d9572
```

Omitting both mandatory frames reproduced the three rejected values:

```text
7d2f555ab2846e8a1eb90489578d376296f4b5a8c3d423916efcdb12657a90f8
8a21a924d61360b7112728a7eb0b77287bd75ec0920ea1e310036e7be83f38c8
a922b904a2283e013d615a9f8e6c2f98c5ef8cc7bb5b9f040331789bb936f6c6
```

For the empty payload, retaining only the domain frame or only the payload
frame reproduced the two rejected values:

```text
a7862a9248e2ba78a0939a09573001ebf27e99a04d578ff031431d71a9bddde5
8a421273ff37ae58b0e2414ccc035df9534286b01d519b8615b12819662b8b89
```

No negative equals a valid `material_association_digest`.

The audit also independently rebuilt the four new cross-owner fixtures without
calling a production root encoder. It used the Task 6 148-byte Analysis source,
Task 5B Material/group identities and Task 7 Mechanism identity and reproduced:

```text
SourceGroup + Request          lengths 188/198/204
  payload 05756f78177871ca24b0eecbe4d59b1b04b64b63372035d893c736393895c8af
  H       7ff578af64e7949633080495e2cada6acc0b58307480690a1f3c76dbacc77fa2
Material + Request             lengths 221/231/237
  payload cf0610fc5ecf676c5788e01433fda2857b3429f548dec2caf1361a08c483558f
  H       9e7fe772a06a3747f8d050ea15de703cb99f355c3e2d2d63b15618ea8dacf50e
AtomicGroup + Request          group 388, root/entry/payload 428/438/444
  group   b3c237eaccb7eb8732611804c1e972ec329c2f6a3d303ffd017863d4c16eed01
  payload c69abb12f28c3d6cc8db6ffb32bb949f6473e5c3d8474e150a86f083199250d1
  H       c2f9bfbfc8b12e687e153762d902b544808e93a0234e799d06e0973d1e9ee553
Invocation + Mechanism        root/scope/entry/payload 36/61/105/111
  payload 1da4310b9f021f65b020bcb733a0e056b2e02d6c43928c425713601b9bbbb33b
  H       366f40dad89cc048ccbb8eec15e5284da60af21745e8d4401d7b04d221b744b6
```

Single-field tag/frame/source/material/group/family/entry/handler mutations
changed the bytes or failed validation. The shared-pair test remains a required
production RED because it needs the finished typed association builder.

Result: PASS.

## 4. Requirement coverage

| Requirement | Owner evidence | Result |
| --- | --- | --- |
| freeze-invariant header; ledger sole status authority | header, sections 1 and 14 | PASS |
| no owner self/peer hashes or implementation OID values | header and section 14 | PASS |
| exact four-owner tuple/audits/reviews/generator/atomic ledger protocol | sections 1.1 and 14 | PASS |
| edit restarts tuple-derived evidence | sections 1.1 and 14 | PASS |
| exact opaque Task 4 material remains owner-only | section 2.1 and Stage 0 | PASS |
| whole Task 5B context, no detached half or local view | sections 2.1-2.2 | PASS |
| restricted Analysis header view exposes no Form lookup | sections 2.1-2.2 | PASS |
| composite/Analysis/context/injected-reader pass-through | sections 2.1 and 6 | PASS |
| owner-attributed FormXml/BSL reads; no hidden reader | sections 2.1, 6, 10 and 13 | PASS |
| raw Task4 handle/registered reader remains private to Task5B context | sections 2.1, 6 and 13 | PASS |
| exact Task 6 counter/error/zero-prefix matrix retained | sections 2.1, 11 and 13 | PASS |
| Metadata/Form/Support imported, not re-encoded | sections 2.2 and 2.4 | PASS |
| six Task 6 v3 digests imported only as bytes | section 2.3 | PASS |
| direct AtomicSource framing and extra-frame rejection | section 2.3 | PASS |
| typed invocation lifecycle and strict finish-time bijection | sections 3.1-3.2 and 6 | PASS |
| Task5A `methods=[]` plan produces one Task6 typed invocation | sections 3.1-3.2 and 6 | PASS |
| empty association scope is distinct and cannot schedule I/O | sections 3.1-3.2 and 13 | PASS |
| response-owned provider appears only at terminal recording | sections 3.1-3.2 | PASS |
| recorded handle gates every post-response association | sections 3.1-3.2 | PASS |
| exact `MaterialAssociationMapV2` roots/scopes and pair halves | section 3 | PASS |
| numeric and cross-owner structural map goldens | section 3.4 | PASS |
| no provider-side conclusion scope fields | sections 4-5 | PASS |
| closed raw/effective/trace/owner/reason encoders | sections 4-5 | PASS |
| raw retryable and exact raw gap/location identity bytes | section 4.1 | PASS |
| Global/PerPort admission is same-port; only limit is portless | section 5.2 | PASS |
| exhaustive additive v4 registry and snapshot bytes | section 4.2 | PASS |
| association cannot influence admission order | sections 5.3 and 10.4 | PASS |
| Task5C-Evidence one-way DAG | sections 1.3, 2.4 and 13 | PASS |
| Task 7 prerequisite / Task 8 integration split | section 9 | PASS |
| public MCP/package/skill surface unchanged | constraints, file map and STOP gates | PASS |

## 5. Findings by severity

### P0

None.

### P1

None. The owner-local zero count is based on exact compatibility with the
current peer bytes inspected in section 1. If any owner byte changes, the
external tuple evidence and this cross-owner result are stale until rerun; that
does not authorize an in-place status/hash edit to the owner file.

### P2 — downstream, non-gating for the four-owner design package

1. Task5C-Evidence implementation must consume the final Task 5B Support query
   and produce its independently accepted production OID before Task 7
   production can execute `SupportStatePort`.
2. Task 8 delivery must consume `Task7PrerequisiteSliceV1`, implement
   `Task7Task8IntegrationV1` and record distinct integration evidence without
   redefining Task 7 query/association/admission semantics.
3. The implementation slice must synchronize active spec/ADR/plan/product
   guards and run the complete Task 5B/6/5C/7 production verification matrix;
   green design checks are not production acceptance.

These P2 items gate their downstream delivery only. They create no reverse
edge into the Task4/Task5B/Task6/Task7 design package.

## 6. Static, whitespace and stale-text audit

The final owner checks require and produced:

```text
immutable Task 7 v6 lineage check                         PASS
independent map positive/negative reproduction           PASS
Task 6 standalone two-path generator assertions          PASS
Task 6 six digest bytes + extra-frame negative match      PASS
Markdown fences balanced                                  PASS
no-index whitespace diagnostics                          PASS
no trailing whitespace or tab indentation                PASS
no unresolved drafting markers                           PASS
no mutable owner-state or stale query-v3 blocker prose   PASS
no current self/peer/audit/review hash in owner addendum  PASS
no implementation OID value in owner addendum/audit      PASS
no provider struct field named conclusion_scopes          PASS
no live EvidenceMaterialSubjectV2::Conclusion variant     PASS
no local Metadata/Form/BSL/Support query encoder          PASS
exact EvidenceExecutionContext capability triple          PASS
no live Task 7 registered-material resolver/reader path   PASS
no hidden reader outside EvidenceExecutionContext field  PASS
restricted Analysis header view has no Form lookup        PASS
providers call context FormXml/BSL verification only      PASS
raw Task4 handle/registered reader stays Task5B-private   PASS
FormXml calls attributed to Task 5B owner providers       PASS
Task 6 BSL reader/error matrix preserved without reclassify PASS
typed plan/raw/Invocation-root lifecycle closed           PASS
scheduled empty-member Definition remains an invocation   PASS
pre-I/O plan contains no response-owned provider identity PASS
all post-response associations require recorded handle    PASS
raw retryable and exact gap/location bytes are projected  PASS
Global and PerPort admissions remain same-port rows       PASS
raw/effective/trace vector encodings and bijections closed PASS
effective owner/reason/digest v3 closed                   PASS
complete additive v4 registry/snapshot encoding closed   PASS
all map root/scope structural goldens required            PASS
no Task 8 concrete import in Task7PrerequisiteSliceV1     PASS
no Task 7 prerequisite edge in Task5C-Evidence            PASS
```

References to superseded v2 bytes, deleted v6 fields and forbidden Task 8
imports remain only as explicit negative requirements. The external generator
evidence must be rebound to the final owner tuple after any owner edit; this
audit does not publish that tuple or claim the ledger transition.

## 7. Owner verdict

The Task 7 owner semantics are closed and cross-owner compatible at P0 = 0 and
P1 = 0: provider testimony remains proposal-independent, Task 4 opaque material
stays behind the whole Task 5B context, the exact composite/Analysis snapshots
and injected reader pass through Task 7 without an opaque material/read path,
Task 6 v3 digest bytes are imported without reconstruction, the typed
plan/recorded-handle/raw-root lifecycle and every v3/v4 identity are closed,
raw retryable/gap/location testimony remains lossless, application Global and
PerPort admission remains port-qualified, provider-material
association/admission stay separate from application traversal scopes,
Task5C-Evidence has a one-way production edge, and Task 8 owns only the later
integration.

This owner audit is not package acceptance. The external ledger may determine
design status only after it binds the exact four-owner tuple, refreshed
generator evidence, all owner audits and separate independent reviews in one
atomic transition. Production remains separately gated by exact implementation
OIDs and implementation verification.
