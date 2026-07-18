# Task 5B v5 — fresh adversarial contract self-audit

Date: 2026-07-18

Status: **PASS-CONDITIONAL for the normative design; STOP for implementation**.

This audit does not accept the current dirty implementation and does not replace
the mandatory implementation review. It verifies that the frozen v5 contract
and its Task 7 back-propagation now close the known design P0/P1/P2 findings.
The implementation gate remains closed because no accepted immutable Task 5A
commit exists.

## 1. Immutable audit basis

```text
Task 5B v5 contract
  file: .superpowers/sdd/task-5b-contract.md
  SHA-256: 13ca8e3599ce3e4843ae82773a8911194f2786ce741b9040c14563b60dbedbab

Task 7 v5-back-propagated design
  file: .superpowers/sdd/task-7-design.md
  SHA-256: 6792d70c58a57a35871a91f5dd9059371ee13599a96e0c00e97e27a974f6ca2a

Observed repository HEAD for the implementation-gap comparison
  20f6afa7a09430614babebc0cdeebeb94c8a0189

TASK5A_ACCEPTED_SHA
  unavailable
```

Rejected/historical anchors remain immutable:

```text
rejected v4 contract before rejection
  5c25c74d18b87799e0eea383e9a684d8674b4eefe98cfc2382f5f74fdb2df8bb

rejected v4 self-audit before rejection
  de13c4509d6333eceb63fde7f70fcd3818ad970fb878fbc6b9e278f5987fdf6d

pre-v5 Task 7 snapshot
  dfe521ab491b4696b89728b5ed0089da57eec3320c2af7685c0dced7aef02736
```

The SHA-256 of this audit is published externally after the file is frozen. It
cannot be embedded into itself without changing the value.

## 2. Audit method

The re-audit re-read the v5 contract and Task 7 design as independent documents,
then compared them with the live code/spec surface at the observed HEAD plus its
dirty working diff. It also rechecked the official 1C primary examples linked in
contract section 2 for Form-command and HTTP handler shapes.

Static checks performed before freezing:

- Markdown fences are balanced: contract 76, Task 7 design 42;
- no trailing whitespace exists in either frozen file;
- superseded per-source Metadata invocation wording appears only in the explicit
  rejection banner; the normative orchestration requires one composite call;
- old provider type spellings were removed from both the prerequisite list and
  native-composition example;
- no stale “seven supported flows” statement remains: the scope has eight,
  including DocumentLifecycle;
- all v3/v4 implementation documents retain explicit superseded/rejected
  banners and immutable historical hashes where available.

No cargo/Python/product test result is claimed by this design audit. Those are
implementation acceptance gates and cannot be meaningful against the current
unaccepted dirty candidate.

## 3. Closed design findings

### P0 closure

| Former defect | v5 closure |
| --- | --- |
| incomplete EventSubscription family table | exact 13 family/root rows, 21 compatible event/family cells, three signature classes, partial lookup and no permitted catch-all |
| Source XML namespace conflation | exact MDClasses Source element, direct data-core Type element, independently QName-resolved current-config scalar |
| handler owner segment lost | EventSubscription Handler and ScheduledJob MethodName require literal `CommonModule.<registered owner>.<method>` |
| selected-source fragments treated as authorities | descriptor selected set is sole authority; companion set and ExchangePlan-filtered uses subset require exact equality |
| record limit split cross-fact conclusions | closed `SemanticAtomicGroupIdV1`, exact total order, whole-group canonical prefix-stop, complete dropped material scopes |
| Form event owners disappeared | closed binding-owner grammar plus independent bounded binding-shaped descendant audit, including ExtendedTooltip companions |
| literal Direct accepted | one Option-based parser: absence means Direct; present Direct/empty/unknown is invalid in native, Task 5B and Task 8 |
| Form main type parsed by `cfg:` text | exact data-core Type element plus namespace-resolved current-config QName |
| callback/HTTP behavior depended on external assumptions | four-row callback registry and closed HTTP path/cardinality/verb rules are self-contained |
| Form/HTTP Definition compatibility guessed | primary-backed versioned pending requirements and tri-state closed policies; no runtime edge before a policy Yes |
| async declaration bit discarded | Task 5A must retain `DefinitionShape.is_async`; every handler family states its sync/async policy and Task 7 binds the versions into analysis identity |
| ScheduledJob disabled conflated with incomplete binding | separate metadata-only `ScheduledJobActivationV1::Disabled` gives exact No from registered job + exact Use=false only |
| declared scope counted seven while Task 7 used eight | v5 explicitly lists the eight families/flows, including DocumentLifecycle |

### ScheduledJob atomicity proof

`ScheduledJobActivationV1::Disabled` is sufficient without contradicting whole-
descriptor atomicity:

1. exact Use=false selects `ScheduledJobCluster(DisabledActivation)`;
2. no positive `ScheduledJobBindingV1` is constructed in that branch, so there
   is no hidden descriptor half to retain or drop;
3. Predefined, MethodName, module profile and Definition are nonmaterial to that
   negative and cannot downgrade it;
4. exact Use=true selects `ScheduledJobCluster(EnabledDescriptor)`, where Use and
   all positive descriptor facts remain one atomic group;
5. each physical record belongs to exactly one mutually exclusive branch.

The metadata-first Definition matrix is closed: only exact Use=true,
Predefined=true, Global=false, Server=true plus valid registered MethodName/
owner schedules the job's Definition endpoint. Disabled is No; nonpredefined,
profile-invalid, malformed, or gapped metadata is Unknown and schedules no
Definition solely for that job.

### P1/P2 closure

- arbitrary XML prefix spelling is nonsemantic while exact element/QName URI is
  material;
- object UUID location, spelling, nil rejection, identifier/QName/token limits,
  form ID domains and `-1` sentinel scope are explicit;
- Form commands require at least one Action and exact plain/borrowed extension
  call-type cardinalities;
- persistent Form main-context families are closed and DynamicList is explicitly
  nonpersistent;
- HTTP path preservation/rejection behavior and 2048/4096 boundaries are exact;
- callbacks, event subscriptions, jobs, Form commands and HTTP routes all define
  No versus Unknown precedence rather than equating unsupported with absent;
- Task 7 consumes accepted whole facts/policy outcomes, never re-parses platform
  XML or reconstructs compatibility from edge fragments;
- Task 7 applies the same atomic-group prefix-stop at per-port and global limits,
  rebuilds mechanisms after drops and binds policy/group versions into analysis
  identity;
- the 48-case Task 7 corpus now freezes the two positive fixture semantics for
  every family, including synchronous/asynchronous Form rows and Procedure/
  Function ScheduledJob rows.

No open normative P0, P1, or P2 is known in the two frozen design artifacts.
That statement is about design completeness only.

## 4. Remaining implementation blockers at the observed snapshot

These are **not** design findings and do not invalidate the PASS-CONDITIONAL
above. They keep implementation at STOP:

### Open P0

1. `TASK5A_ACCEPTED_SHA` does not exist. HEAD is not an accepted Task 5A commit,
   and the worktree contains concurrent uncommitted code/spec changes.
2. The live semantic grouping implementation still uses generic
   `Anchored(Vec<SemanticAtomicAnchorV1>)` unioning, not the closed v5 classifier
   or the mutually exclusive ScheduledJob Disabled/Enabled cluster.
3. `event_handler_signature_class_v1` still contains a wildcard
   `Some(SourceAndCancel)` arm. A preceding compatibility guard does not satisfy
   the v5 no-catch-all invariant.
4. FormCommand and HTTP still return `PolicyUnavailable`; the two accepted
   versioned compatibility policies and their REDs are not implemented.
5. ScheduledJob still uses one `ScheduledJobDescriptorV1` flag bundle rather
   than a separate activation fact. Event/Scheduled explicit contexts are still
   folded into hard signature mismatch instead of the v5 unsupported-variant
   Unknown row.
6. Concrete Platform XML/Form/BSL/support providers, Task 7 mechanism modules and
   the frozen 48-case corpus were not present in tracked files at audit time.
7. No immutable implementation diff has passed the required focused/full tests,
   formatting, clippy, product contracts, Windows compile and fresh review.

### Open P1

1. Active spec/ADR/historical plan/product-contract assertions do not yet state
   the complete v5 handler-policy versions, async semantics, ScheduledJob
   activation split and exact metadata-first Definition matrix.
2. The Task 5B delivery report cannot record fixture/source provenance,
   implementation SHA or command results until the implementation exists and is
   immutable.

There is no legitimate implementation shortcut around these blockers. In
particular, accepting a dirty diff hash, a branch name, current HEAD, or a
temporary `PolicyUnavailable` outcome as `TASK5A_ACCEPTED_SHA` would violate the
gate this audit is verifying.

## 5. Verdict

The v5 contract and Task 7 back-propagation are sufficiently closed to build an
implementation plan. Implementation must remain stopped until Task 5A lands as
one accepted immutable commit and every open P0/P1 above is resolved against a
fresh code/spec review. The rejected v4 self-audit must never be reused as
evidence of readiness.
